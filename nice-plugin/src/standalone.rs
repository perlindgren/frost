//! One audio owner, Apply/rollback loop and transport pump for mono/stereo standalones.
mod process;
use crossbeam::channel::{Receiver, bounded};
use nice_common::{
    Preferences, Settings,
    audio_io::{self, DeviceStreams},
    control::{Command, Control},
    devices,
};
use nice_plug::{editor::EditorHandle, prelude::Plugin};
pub use process::{Context, process_once};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
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
fn open_session<
    P: Plugin<BackgroundTask = ()>,
    Pref: Preferences,
    B: Fn(usize) + Send + Sync + 'static,
    const BLOCK: usize,
    const CAPACITY: usize,
>(
    settings: &Settings<Pref>,
    plugin: &Arc<Mutex<P>>,
    on_buffer: &Arc<B>,
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
        crate::lifecycle::activate(
            &mut *instance,
            rate,
            BLOCK,
            &mut Context::<P>::new(rate),
            &format!("{} could not activate", P::NAME),
        )?;
    }
    let (io, endpoints) = audio_io::open(&resolved)?;
    let stop = Arc::new(AtomicBool::new(false));
    let pump = {
        let stop = Arc::clone(&stop);
        let on_buffer = Arc::clone(on_buffer);
        let plugin = Arc::clone(plugin);
        std::thread::Builder::new()
            .name(format!("{} DSP", P::NAME))
            .spawn(move || {
                process::pump::<P, B, BLOCK, CAPACITY>(plugin, on_buffer, endpoints, stop, rate)
            })
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
struct AudioOwner<P: Plugin, Pref: Preferences + 'static, B> {
    plugin: Arc<Mutex<P>>,
    on_buffer: Arc<B>,
    session: Option<Session>,
    applied: Settings<Pref>,
    control: &'static Control<Settings<Pref>>,
}

impl<
    P: Plugin<BackgroundTask = ()>,
    Pref: Preferences + Send + 'static,
    B: Fn(usize) + Send + Sync + 'static,
> AudioOwner<P, Pref, B>
{
    fn open_ready<const BLOCK: usize, const CAPACITY: usize>(
        &mut self,
        settings: &Settings<Pref>,
    ) -> Result<Option<Session>, String> {
        open_session::<P, Pref, B, BLOCK, CAPACITY>(settings, &self.plugin, &self.on_buffer)
    }

    fn apply<const BLOCK: usize, const CAPACITY: usize>(
        &mut self,
        settings: Settings<Pref>,
    ) -> Result<(), String> {
        // Dropping the session here is the synchronisation point: its Drop
        // joins the pump, so by the time a new activation takes the lock the
        // old one is off the plugin for good.
        drop(self.session.take());
        match self.open_ready::<BLOCK, CAPACITY>(&settings) {
            Ok(session) => {
                self.session = session;
                self.applied = settings;
                Ok(())
            }
            Err(error) => {
                let previous = self.applied.clone();
                match self.open_ready::<BLOCK, CAPACITY>(&previous) {
                    Ok(session) => self.session = session,
                    Err(restore) => {
                        self.applied = Settings::default();
                        self.control.report_error(format!(
                            "{error}\nRestoring the previous audio configuration also failed: {restore}"
                        ));
                    }
                }
                Err(error)
            }
        }
    }
}

fn audio_worker<
    P: Plugin<BackgroundTask = ()>,
    Pref: Preferences + Send + 'static,
    B: Fn(usize) + Send + Sync + 'static,
    D: Fn(u64),
    const BLOCK: usize,
    const CAPACITY: usize,
>(
    mut owner: AudioOwner<P, Pref, B>,
    initial: Settings<Pref>,
    commands: Receiver<Command<Settings<Pref>>>,
    pending: Arc<AtomicBool>,
    ready: crossbeam::channel::Sender<()>,
    on_dropped: D,
) {
    let control = owner.control;
    if let Err(error) = owner.apply::<BLOCK, CAPACITY>(initial) {
        eprintln!("{error}");
        control.report_error(error);
    }
    let _ = ready.send(());
    loop {
        match commands.recv_timeout(Duration::from_millis(20)) {
            Ok(command) => {
                let result = owner.apply::<BLOCK, CAPACITY>(command.settings);
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
            on_dropped(dropped);
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
            control.report_error(error);
        }
    }
}

/// Shared standalone owner for the pedal family. Plugin DSP, view
/// construction and preferences remain with the caller.
pub fn run<P, Pref, U, B, D, const BLOCK: usize, const CAPACITY: usize>(
    plugin: P,
    editor: crate::Editor<U>,
    settings: Settings<Pref>,
    control: &'static Control<Settings<Pref>>,
    on_buffer: B,
    on_dropped: D,
) -> Result<(), String>
where
    P: Plugin<BackgroundTask = ()>,
    Pref: Preferences + Send + 'static,
    U: frost::Process + 'static,
    B: Fn(usize) + Send + Sync + 'static,
    D: Fn(u64) + Send + 'static,
{
    let layout = &P::AUDIO_IO_LAYOUTS[0];
    let inputs = layout.main_input_channels.map_or(0, |n| n.get());
    let outputs = layout.main_output_channels.map_or(0, |n| n.get());
    if inputs != outputs || !(1..=2).contains(&inputs) {
        return Err("This standalone adapter requires matching mono or stereo buses".into());
    }
    if BLOCK == 0 || CAPACITY < BLOCK * outputs as usize {
        return Err("Insufficient standalone buffer capacity".into());
    }
    let plugin = Arc::new(Mutex::new(plugin));
    let (sender, commands) = bounded(1);
    let pending = Arc::new(AtomicBool::new(false));
    let registration = control.register(sender, Arc::clone(&pending));
    let (ready, started) = bounded(1);
    let worker = std::thread::Builder::new()
        .name(format!("{} audio", P::NAME))
        .spawn(move || {
            let owner = AudioOwner {
                plugin,
                on_buffer: Arc::new(on_buffer),
                control,
                session: None,
                applied: Settings::default(),
            };
            audio_worker::<P, Pref, B, D, BLOCK, CAPACITY>(
                owner, settings, commands, pending, ready, on_dropped,
            )
        })
        .map_err(|e| e.to_string())?;
    let result = if started.recv().is_err() {
        Err("The audio owner stopped during startup".into())
    } else {
        editor
            .spawn_window(None, false, None, None, None)
            .map_err(|e| e.to_string())
            .and_then(|spawned| {
                let result = crate::EditorHandle::run_until_closed(spawned.window)
                    .map_err(|e| e.to_string());
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
