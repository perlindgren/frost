//! Application-owned standalone host. nice-plug is used only through its public
//! Plugin/Editor interfaces; its standalone wrapper is neither built nor patched.
mod audio;
pub mod control;
#[cfg(target_os = "linux")]
mod jack_audio;
mod process;
mod resample;

use crate::{NamEditor, NamEditorHandle, NamPlugin, settings::Settings};
use audio::{Failure, Session};
use crossbeam::channel::{Receiver, bounded};
use nice_plug::editor::EditorHandle;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

const RETRIES: usize = 3;

/// One owner destroys the old streams before opening replacements. It retains
/// the plugin independently of stream lifetime and rolls back a failed Apply.
struct AudioOwner<F, S> {
    open: F,
    session: Option<S>,
    applied: Settings,
}
impl<F, S> AudioOwner<F, S>
where
    F: FnMut(&Settings) -> Result<S, Failure>,
    S: Session,
{
    fn open_ready(&mut self, settings: &Settings) -> Result<Option<S>, String> {
        if settings
            .backend
            .as_deref()
            .is_none_or(|backend| backend == "dummy")
        {
            return Ok(None);
        }
        for attempt in 0..=RETRIES {
            let result = (self.open)(settings).and_then(|mut session| {
                session.wait_started()?;
                Ok(session)
            });
            match result {
                Ok(session) => return Ok(Some(session)),
                Err(error) if error.invalidated && attempt < RETRIES => {
                    // The failed session has been dropped before this delay.
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(error) => return Err(error.message),
            }
        }
        unreachable!()
    }
    fn apply(&mut self, settings: Settings) -> Result<(), String> {
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
    plugin: Arc<Mutex<NamPlugin>>,
    initial: Settings,
    commands: Receiver<control::Command>,
    pending: Arc<AtomicBool>,
    ready: crossbeam::channel::Sender<()>,
) {
    let mut owner = AudioOwner {
        open: move |settings: &Settings| audio::open(settings, Arc::clone(&plugin)),
        session: None,
        applied: Settings::default(),
    };
    if let Err(error) = owner.apply(initial) {
        eprintln!("{error}");
        control::report_error(error);
    }
    let _ = ready.send(());
    let mut stable_since = Instant::now();
    let mut recoveries = 0;
    loop {
        match commands.recv_timeout(Duration::from_millis(20)) {
            Ok(command) => {
                let result = owner.apply(command.settings);
                pending.store(false, Ordering::Release);
                let _ = command.reply.send(result);
                stable_since = Instant::now();
                recoveries = 0;
            }
            Err(crossbeam::channel::RecvTimeoutError::Disconnected) => break,
            Err(crossbeam::channel::RecvTimeoutError::Timeout) => {}
        }
        if stable_since.elapsed() >= Duration::from_secs(2) {
            recoveries = 0;
        }
        if let Some(error) = owner.session.as_ref().and_then(Session::failure) {
            drop(owner.session.take());
            if error.invalidated && recoveries < RETRIES {
                recoveries += 1;
                std::thread::sleep(Duration::from_millis(200));
                let settings = owner.applied.clone();
                match owner.open_ready(&settings) {
                    Ok(session) => {
                        owner.session = session;
                        stable_since = Instant::now();
                        continue;
                    }
                    Err(message) => control::report_error(message),
                }
            } else {
                control::report_error(error.message);
            }
            owner.applied = Settings::default();
        }
    }
}

/// Run the original editor on the main thread and device ownership on a worker.
/// Closing the window drops the command endpoint and joins the audio owner.
pub fn run(settings: Settings) -> Result<(), String> {
    let plugin = NamPlugin::default();
    if let Some(window) = settings.window {
        plugin
            .shared
            .logical_size
            .store(nice_plug::editor::dpi::LogicalSize::new(
                window.width.max(crate::MIN_SIZE.width),
                window.height.max(crate::MIN_SIZE.height),
            ));
    }
    let editor = NamEditor {
        params: Arc::clone(&plugin.params),
        shared: Arc::clone(&plugin.shared),
    };
    let plugin = Arc::new(Mutex::new(plugin));
    let (sender, commands) = bounded(1);
    let pending = Arc::new(AtomicBool::new(false));
    let registration = control::register(sender, Arc::clone(&pending));
    let (ready, started) = bounded(1);
    let worker = std::thread::Builder::new()
        .name("nice-nam audio".into())
        .spawn(move || audio_worker(plugin, settings, commands, pending, ready))
        .map_err(|e| e.to_string())?;
    let result = if started.recv().is_err() {
        Err("The audio owner stopped during startup".into())
    } else {
        editor
            .spawn_window(None, false, None, None, None)
            .map_err(|e| e.to_string())
            .and_then(|spawned| {
                let result =
                    NamEditorHandle::run_until_closed(spawned.window).map_err(|e| e.to_string());
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
