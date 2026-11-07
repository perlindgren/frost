//! Application-owned standalone host. nice-plug is used only through its public
//! Plugin/Editor interfaces; its standalone wrapper is neither built nor patched.
mod audio;
pub mod control;
#[cfg(target_os = "linux")]
mod jack_audio;
mod process;

use crate::{TunerEditor, TunerEditorHandle, TunerPlugin, settings::Settings};
use audio::{Failure, Session};
use crossbeam::channel::{Receiver, bounded};
use nice_plug::editor::EditorHandle;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

static TEST_FREQUENCY: std::sync::OnceLock<f64> = std::sync::OnceLock::new();

/// Override incoming standalone audio with a unit-amplitude sine tone.
/// Must be set before starting the audio host; the override survives Apply.
pub fn set_test_frequency(hz: f64) -> Result<(), String> {
    if !hz.is_finite() || hz <= 0.0 {
        return Err("test_freq must be a finite, positive frequency in Hz".into());
    }
    TEST_FREQUENCY
        .set(hz)
        .map_err(|_| "test_freq was already set".into())
}

pub(crate) static NOTE_FREQUENCY: std::sync::OnceLock<f64> = std::sync::OnceLock::new();

/// Supply a fixed detected frequency, bypassing pitch estimation in the editor.
pub fn set_note_frequency(hz: f64) -> Result<(), String> {
    if !hz.is_finite() || hz <= 0.0 {
        return Err("note_freq must be finite and greater than zero".into());
    }
    NOTE_FREQUENCY
        .set(hz)
        .map_err(|_| "note_freq was already set".into())
}

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
    plugin: Arc<Mutex<TunerPlugin>>,
    initial: Settings,
    commands: Receiver<control::Command<Settings>>,
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
    let plugin = TunerPlugin::default();
    let editor = TunerEditor {
        params: Arc::clone(&plugin.params),
        shared: Arc::clone(&plugin.shared),
    };
    let plugin = Arc::new(Mutex::new(plugin));
    let (sender, commands) = bounded(1);
    let pending = Arc::new(AtomicBool::new(false));
    let registration = control::register(sender, Arc::clone(&pending));
    let (ready, started) = bounded(1);
    let worker = std::thread::Builder::new()
        .name("nice-tuner audio".into())
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
                    TunerEditorHandle::run_until_closed(spawned.window).map_err(|e| e.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    struct FakeStream {
        live: Arc<AtomicBool>,
        start_error: bool,
    }
    impl Drop for FakeStream {
        fn drop(&mut self) {
            self.live.store(false, Ordering::Release);
        }
    }
    impl Session for FakeStream {
        fn wait_started(&mut self) -> Result<(), Failure> {
            if self.start_error {
                Err(Failure {
                    message: "system refused stream start".into(),
                    invalidated: false,
                })
            } else {
                Ok(())
            }
        }
        fn failure(&self) -> Option<Failure> {
            None
        }
    }
    #[test]
    fn apply_drops_old_streams_and_restores_previous_settings_after_failure() {
        let live = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&live);
        let opens = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&opens);
        let mut owner = AudioOwner {
            open: move |settings: &Settings| {
                assert!(
                    !observed.load(Ordering::Acquire),
                    "old device was still held open"
                );
                count.fetch_add(1, Ordering::Relaxed);
                if settings.period_size == Some(13) {
                    return Err(Failure {
                        message: "system refused buffer 13".into(),
                        invalidated: false,
                    });
                }
                observed.store(true, Ordering::Release);
                Ok(FakeStream {
                    live: Arc::clone(&observed),
                    start_error: settings.period_size == Some(17),
                })
            },
            session: None,
            applied: Settings::default(),
        };
        let good = Settings {
            backend: Some("core-audio".into()),
            period_size: Some(64),
            sample_rate: Some(44100.0),
            output_sample_rate: Some(48000.0),
            ..Default::default()
        };
        owner.apply(good.clone()).unwrap();
        for (buffer, error) in [
            (13, "system refused buffer 13"),
            (17, "system refused stream start"),
        ] {
            let mut settings = good.clone();
            settings.period_size = Some(buffer);
            assert_eq!(owner.apply(settings), Err(error.into()));
            assert_eq!(owner.applied, good);
            assert!(live.load(Ordering::Acquire));
        }
        assert_eq!(opens.load(Ordering::Relaxed), 5);
        owner.apply(Settings::default()).unwrap();
        assert!(!live.load(Ordering::Acquire));
        // Silent mode remains usable: a later Apply opens audio again.
        owner.apply(good).unwrap();
        assert!(live.load(Ordering::Acquire));
    }
    #[test]
    fn invalidation_retries_are_bounded_and_leave_the_owner_usable() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&attempts);
        let live = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&live);
        let mut owner = AudioOwner {
            open: move |settings: &Settings| {
                count.fetch_add(1, Ordering::Relaxed);
                if settings.period_size == Some(32) {
                    return Err(Failure {
                        message: "device sample rate changed".into(),
                        invalidated: true,
                    });
                }
                observed.store(true, Ordering::Release);
                Ok(FakeStream {
                    live: Arc::clone(&observed),
                    start_error: false,
                })
            },
            session: None,
            applied: Settings::default(),
        };
        let mut settings = Settings {
            backend: Some("core-audio".into()),
            period_size: Some(32),
            ..Default::default()
        };
        assert_eq!(
            owner.apply(settings.clone()),
            Err("device sample rate changed".into())
        );
        assert_eq!(attempts.load(Ordering::Relaxed), RETRIES + 1);
        assert!(owner.session.is_none());
        settings.period_size = Some(64);
        owner.apply(settings).unwrap();
        assert!(live.load(Ordering::Acquire));
    }
}
