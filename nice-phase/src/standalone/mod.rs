//! Application-owned standalone host. nice-plug is used only through its
//! public Plugin/Editor interfaces; its standalone wrapper is neither built
//! nor patched.
//!
//! The device transport is `nice_common::audio_io` — ASIO on Windows, cpal's
//! hosts and JACK elsewhere, whichever the settings page settled on — the
//! same opener the rack runs, so this file holds no backend code at
//! all. What it owns is everything between a settings file and that
//! transport: resolve against the devices as they are right now, hand the
//! rings to the pump in [`process`], keep one owner so an Apply never has
//! two live stream pairs, and roll back when a new one will not open.
pub mod control;
mod process;

use crate::editor::PhaseEditorHandle;
use crate::settings::Settings;
use crate::{MIN_SIZE, PhaseEditor, PhasePlugin, PhaseSharedState, devices};
use crossbeam::channel::{Receiver, bounded};
use nice_common::audio_io::{self, DeviceStreams};
use nice_plug::editor::EditorHandle;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// One live pair: the transport nice-common opened, and the pump thread that
/// carries its rings through the plugin.
struct Session {
    stop: Arc<AtomicBool>,
    pump: Option<JoinHandle<()>>,
    io: DeviceStreams,
}

impl Drop for Session {
    fn drop(&mut self) {
        // Stop, then join, and only then let the streams go: the pump holds
        // the endpoints, and a stream is not quiet the moment it is dropped —
        // cpal may still be inside its callback, and an ASIO driver keeps
        // filling buffers until `stop` returns (`asio_io.rs:650`).
        self.stop.store(true, Ordering::Release);
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
    }
}

/// Open, activate and start one pair of streams for resolved settings.
/// `None` is not a failure: with no backend chosen the app stays silent on
/// purpose, exactly as the tuner does.
fn open_session(
    settings: &Settings,
    plugin: &Arc<Mutex<PhasePlugin>>,
    shared: &Arc<PhaseSharedState>,
) -> Result<Option<Session>, String> {
    let backend = settings.backend.clone().unwrap_or_default();
    if backend.is_empty() || backend == "dummy" {
        return Ok(None);
    }
    // The saved numbers are wishes; the snapshot is the device's answer to
    // them, taken now, the same resolve-then-open the rack runs.
    let snapshot = devices::snapshot(
        &backend,
        settings.input_device.as_deref(),
        settings.output_device.as_deref(),
    );
    let resolved = snapshot.resolve_settings(settings)?;
    let rate = resolved.sample_rate.ok_or("Missing sample rate")?;
    {
        let mut instance = plugin
            .lock()
            .map_err(|_| "The plugin audio state is unavailable")?;
        process::activate(&mut instance, rate)?;
    }
    let (io, endpoints) = audio_io::open(&resolved)?;
    let stop = Arc::new(AtomicBool::new(false));
    let pump = {
        let stop = Arc::clone(&stop);
        let shared = Arc::clone(shared);
        let plugin = Arc::clone(plugin);
        std::thread::Builder::new()
            .name("nice-phase dsp".into())
            .spawn(move || process::pump(plugin, shared, endpoints, stop, rate))
            .map_err(|error| error.to_string())?
    };
    let session = Session {
        stop,
        pump: Some(pump),
        io,
    };
    wait_started(&session)?;
    Ok(Some(session))
}

/// Both device callbacks have reported at least one delivered block, or the
/// transport has said why it will not. A pair that opened but never runs is
/// a broken pair, and the rollback below should treat it as one.
fn wait_started(session: &Session) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(error) = session.io.failure() {
            return Err(error);
        }
        if session.io.counters.input.load(Ordering::Acquire) > 0
            && session.io.counters.output.load(Ordering::Acquire) > 0
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(
                "The devices opened but did not start delivering audio within 3 seconds".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// One owner destroys the old streams before opening replacements, and rolls
/// a failed Apply back to the last settings that actually made sound. It
/// keeps its own clone of the plugin so a stream's lifetime never decides
/// whether the plugin lives.
struct AudioOwner {
    plugin: Arc<Mutex<PhasePlugin>>,
    shared: Arc<PhaseSharedState>,
    session: Option<Session>,
    applied: Settings,
}

impl AudioOwner {
    fn open_ready(&mut self, settings: &Settings) -> Result<Option<Session>, String> {
        open_session(settings, &self.plugin, &self.shared)
    }

    fn apply(&mut self, settings: Settings) -> Result<(), String> {
        // Dropping the session here is the synchronisation point: its Drop
        // joins the pump, so by the time a new activation takes the lock the
        // old one is off the plugin for good.
        drop(self.session.take());
        match self.open_ready(&settings) {
            Ok(session) => {
                self.session = session;
                self.applied = settings;
                Ok(())
            }
            Err(error) => {
                let previous = self.applied.clone();
                match self.open_ready(&previous) {
                    Ok(session) => self.session = session,
                    Err(restore) => {
                        self.applied = Settings::default();
                        control::report_error(format!(
                            "{error}\nRestoring the previous audio configuration also failed: {restore}"
                        ));
                    }
                }
                Err(error)
            }
        }
    }
}

fn audio_worker(
    plugin: Arc<Mutex<PhasePlugin>>,
    shared: Arc<PhaseSharedState>,
    initial: Settings,
    commands: Receiver<control::Command<Settings>>,
    pending: Arc<AtomicBool>,
    ready: crossbeam::channel::Sender<()>,
) {
    let mut owner = AudioOwner {
        plugin,
        shared,
        session: None,
        applied: Settings::default(),
    };
    if let Err(error) = owner.apply(initial) {
        eprintln!("{error}");
        control::report_error(error);
    }
    let _ = ready.send(());
    loop {
        match commands.recv_timeout(Duration::from_millis(20)) {
            Ok(command) => {
                let result = owner.apply(command.settings);
                pending.store(false, Ordering::Release);
                let _ = command.reply.send(result);
            }
            Err(crossbeam::channel::RecvTimeoutError::Disconnected) => break,
            Err(crossbeam::channel::RecvTimeoutError::Timeout) => {}
        }
        let dropped = owner
            .session
            .as_ref()
            .map(|session| session.io.counters.dropped.load(Ordering::Relaxed));
        if let Some(dropped) = dropped {
            owner.shared.dropped.store(dropped, Ordering::Relaxed);
        }
        let died = owner
            .session
            .as_ref()
            .and_then(|session| session.io.failure());
        if let Some(error) = died {
            // The transport said it will not recover. A host that owns its
            // transport gets a `Failure` with an `invalidated` flag to retry
            // against, which is what nice-nam's carries
            // (`nice-nam/src/standalone/audio.rs:32`). The shared opener this
            // one runs through answers with a sentence and nothing more
            // (`nice_common::audio_io::DeviceStreams::failure`), so the honest
            // move is to let the dead pair go, say so, and leave the choice of
            // a new one where every other device decision lives: the panel
            // stays open for business.
            drop(owner.session.take());
            owner.applied = Settings::default();
            control::report_error(error);
        }
    }
}

/// Run the original editor on the main thread and device ownership on a
/// worker. Closing the window drops the command endpoint and joins the audio
/// owner, which stops the pump before it lets the streams go.
pub fn run(settings: Settings) -> Result<(), String> {
    let plugin = PhasePlugin::default();
    if let Some(window) = settings.window {
        plugin
            .shared
            .logical_size
            .store(nice_plug::editor::dpi::LogicalSize::new(
                window.width.max(MIN_SIZE.width),
                window.height.max(MIN_SIZE.height),
            ));
        // Size rides in through `logical_size`, which baseview understands.
        // Position does not exist in baseview at all, so it queues here and
        // the window handler applies it once, or centres the window when the
        // file never recorded one.
        *plugin.shared.pending_geom.lock().unwrap() = window.x.zip(window.y);
    }
    let editor = PhaseEditor {
        params: Arc::clone(&plugin.params),
        shared: Arc::clone(&plugin.shared),
    };
    let plugin = Arc::new(Mutex::new(plugin));
    let (sender, commands) = bounded(1);
    let pending = Arc::new(AtomicBool::new(false));
    let registration = control::register(sender, Arc::clone(&pending));
    let (ready, started) = bounded(1);
    let shared = Arc::clone(&plugin.lock().unwrap().shared);
    let worker = std::thread::Builder::new()
        .name("nice-phase audio".into())
        .spawn(move || audio_worker(plugin, shared, settings, commands, pending, ready))
        .map_err(|error| error.to_string())?;
    let result = if started.recv().is_err() {
        Err("The audio owner stopped during startup".into())
    } else {
        editor
            .spawn_window(None, false, None, None, None)
            .map_err(|error| error.to_string())
            .and_then(|spawned| {
                let result =
                    PhaseEditorHandle::run_until_closed(spawned.window).map_err(|e| e.to_string());
                drop(spawned.handle);
                result
            })
    };
    drop(registration);
    worker
        .join()
        .map_err(|_| "The audio owner stopped unexpectedly".to_string())?;
    result
}
