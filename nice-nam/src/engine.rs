//! Off-thread preparation and retirement of DSP instances.
use crate::{dsp::Chain, settings::NamPreferences};
use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub struct EngineState {
    pub paths: Arc<Mutex<NamPreferences>>,
    pub rate: AtomicU32,
    pub status: Mutex<String>,
    pub accepted: Mutex<Option<NamPreferences>>,
    pub revision: AtomicU64,
}
pub struct Started<C = Box<Chain>> {
    pub state: Arc<EngineState>,
    pub incoming: rtrb::Consumer<C>,
    pub retiring: rtrb::Producer<C>,
    pub worker: Worker,
}

/// Owned by NamPlugin, not the shared GUI state. Destruction must wait for this
/// thread before returning to a host that may unload the plugin's executable.
pub struct Worker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn shutdown(&mut self) {
        if let Some(worker) = self.thread.take() {
            self.stop.store(true, Ordering::Release);
            worker.thread().unpark();
            // Do not detach on a timeout: an in-flight dependency load cannot
            // always be cancelled. Joining also completes panic unwinding; a
            // worker panic must not propagate across plugin destruction's ABI.
            let _ = worker.join();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn start(paths: Arc<Mutex<NamPreferences>>) -> Started {
    start_with(paths, |paths, rate| {
        Chain::load(paths, rate).map(|(chain, description)| (Box::new(chain), description))
    })
}

fn start_with<C: Send + 'static>(
    paths: Arc<Mutex<NamPreferences>>,
    load: impl FnMut(&NamPreferences, u32) -> Result<(C, String), String> + Send + 'static,
) -> Started<C> {
    let state = Arc::new(EngineState {
        paths,
        rate: AtomicU32::new(48000),
        status: Mutex::new("No model loaded".into()),
        accepted: Mutex::new(None),
        revision: AtomicU64::new(0),
    });
    let weak = Arc::downgrade(&state);
    let (ready, incoming) = rtrb::RingBuffer::new(2);
    let (retiring, retired) = rtrb::RingBuffer::new(8);
    let stop = Arc::new(AtomicBool::new(false));
    let requested = stop.clone();
    let thread = thread::spawn(move || run(weak, ready, retired, &requested, load));
    Started {
        state,
        incoming,
        retiring,
        worker: Worker {
            stop,
            thread: Some(thread),
        },
    }
}

fn run<C>(
    state: Weak<EngineState>,
    mut ready: rtrb::Producer<C>,
    mut retired: rtrb::Consumer<C>,
    stop: &AtomicBool,
    mut load: impl FnMut(&NamPreferences, u32) -> Result<(C, String), String>,
) {
    let mut previous = None;
    let mut pending = None;
    loop {
        while let Ok(chain) = retired.pop() {
            drop(chain);
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        let Some(state) = state.upgrade() else {
            break;
        };
        let mut paths = state.paths.lock().unwrap().clone();
        paths.gate_db = None;
        let rate = state.rate.load(Ordering::Acquire);
        let revision = state.revision.load(Ordering::Acquire);
        if previous.as_ref() != Some(&(paths.clone(), rate, revision)) {
            previous = Some((paths.clone(), rate, revision));
            *state.status.lock().unwrap() = "Preparing model and cabinet…".into();
            if stop.load(Ordering::Acquire) {
                break;
            }
            let result = load(&paths, rate);
            if stop.load(Ordering::Acquire) {
                // Dispose of a just-finished load here instead of publishing it
                // after cancellation. This also finishes its native cleanup
                // before the join can return to plugin destruction.
                drop(result);
                break;
            }
            match result {
                Ok((chain, description)) => {
                    pending = Some(chain);
                    *state.accepted.lock().unwrap() = Some(paths.clone());
                    *state.status.lock().unwrap() = description;
                }
                Err(error) => {
                    *state.status.lock().unwrap() =
                        format!("{error}\nThe previous working chain is retained.");
                }
            }
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        if ready.slots() > 0
            && let Some(chain) = pending.take()
        {
            let _ = ready.push(chain);
        }
        drop(state);
        // unpark retains a wake token if shutdown happens before this call.
        thread::park_timeout(Duration::from_millis(20));
    }
    // The plugin has stopped producing audio before it is destroyed. Finish
    // retirement on this worker; ready items still in incoming remain owned by
    // the plugin and are released when its fields are dropped after shutdown.
    while let Ok(chain) = retired.pop() {
        drop(chain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Instant};

    struct Exited(Arc<AtomicBool>);
    impl Drop for Exited {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    fn paths() -> Arc<Mutex<NamPreferences>> {
        Arc::new(Mutex::new(NamPreferences::default()))
    }
    fn wait_for(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(
                Instant::now() < deadline,
                "worker did not reach the expected state"
            );
            thread::yield_now();
        }
    }
    fn change<C>(started: &Started<C>) {
        started.state.revision.fetch_add(1, Ordering::Release);
        started.worker.thread.as_ref().unwrap().thread().unpark();
    }

    #[test]
    fn dropping_an_idle_worker_joins_even_while_shared_state_is_retained() {
        let exited = Arc::new(AtomicBool::new(false));
        let witness = Exited(exited.clone());
        let started = start_with(paths(), move |_, _| {
            std::hint::black_box(&witness);
            Ok(((), "ready".into()))
        });
        wait_for(|| started.incoming.slots() == 1);
        let retained = started.state.clone();
        drop(started.worker);
        assert!(exited.load(Ordering::Acquire));
        assert_eq!(Arc::weak_count(&retained), 0);
        assert_eq!(*retained.status.lock().unwrap(), "ready");
    }

    #[test]
    fn shutdown_waits_for_an_in_flight_load_and_does_not_publish_its_result() {
        let (entered, loading) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let exited = Arc::new(AtomicBool::new(false));
        let witness = Exited(exited.clone());
        let Started {
            state,
            incoming,
            retiring: _retiring,
            mut worker,
        } = start_with(paths(), move |_, _| {
            std::hint::black_box(&witness);
            entered.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(((), "must not publish".into()))
        });
        loading.recv_timeout(Duration::from_secs(5)).unwrap();
        let stop = worker.stop.clone();
        let (finished, returned) = mpsc::channel();
        let closer = thread::spawn(move || {
            worker.shutdown();
            finished.send(()).unwrap();
        });
        wait_for(|| stop.load(Ordering::Acquire));
        assert_eq!(returned.try_recv(), Err(mpsc::TryRecvError::Empty));
        release.send(()).unwrap();
        returned.recv_timeout(Duration::from_secs(5)).unwrap();
        closer.join().unwrap();
        assert!(exited.load(Ordering::Acquire));
        assert_eq!(incoming.slots(), 0);
        assert!(state.accepted.lock().unwrap().is_none());
    }

    #[test]
    fn shutdown_finishes_pending_and_retired_cleanup_before_returning() {
        #[derive(Debug)]
        struct Asset {
            id: usize,
            drops: Arc<Mutex<Vec<(usize, thread::ThreadId)>>>,
        }
        impl Drop for Asset {
            fn drop(&mut self) {
                self.drops
                    .lock()
                    .unwrap()
                    .push((self.id, thread::current().id()));
            }
        }
        let drops = Arc::new(Mutex::new(Vec::new()));
        let events = drops.clone();
        let (entered, loading) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let exited = Arc::new(AtomicBool::new(false));
        let witness = Exited(exited.clone());
        let mut calls = 0;
        let mut started = start_with(paths(), move |_, _| {
            std::hint::black_box(&witness);
            calls += 1;
            entered.send(calls).unwrap();
            if calls == 4 {
                resume.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            Ok((
                Asset {
                    id: calls,
                    drops: events.clone(),
                },
                "ready".into(),
            ))
        });
        assert_eq!(loading.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
        wait_for(|| started.incoming.slots() == 1);
        change(&started);
        assert_eq!(loading.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
        wait_for(|| started.incoming.slots() == 2); // ready queue full
        change(&started);
        assert_eq!(loading.recv_timeout(Duration::from_secs(5)).unwrap(), 3); // pending
        change(&started);
        assert_eq!(loading.recv_timeout(Duration::from_secs(5)).unwrap(), 4); // in flight
        for id in 10..18 {
            started
                .retiring
                .push(Asset {
                    id,
                    drops: drops.clone(),
                })
                .unwrap();
        }
        let stop = started.worker.stop.clone();
        let worker_id = started.worker.thread.as_ref().unwrap().thread().id();
        let release_load = thread::spawn(move || {
            wait_for(|| stop.load(Ordering::Acquire));
            release.send(()).unwrap();
        });
        started.worker.shutdown();
        started.worker.shutdown(); // idempotent
        release_load.join().unwrap();
        assert!(exited.load(Ordering::Acquire));
        let events = drops.lock().unwrap().clone();
        let mut ids: Vec<_> = events.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        assert_eq!(ids, [3, 4, 10, 11, 12, 13, 14, 15, 16, 17]);
        assert!(events.iter().all(|(_, id)| *id == worker_id));
        assert_eq!(started.incoming.slots(), 2);
        drop(started.incoming); // unconsumed ready chains are owned by the plugin
        let events = drops.lock().unwrap();
        assert_eq!(events.len(), 12);
        assert!(
            events
                .iter()
                .filter(|(id, _)| *id <= 2)
                .all(|(_, id)| *id == thread::current().id())
        );
    }

    #[test]
    fn a_worker_panic_is_joined_without_panicking_in_shutdown() {
        let exited = Arc::new(AtomicBool::new(false));
        let witness = Exited(exited.clone());
        let (entered, loading) = mpsc::channel();
        let mut started = start_with(paths(), move |_, _| -> Result<((), String), String> {
            std::hint::black_box(&witness);
            entered.send(()).unwrap();
            panic!("preparation worker test");
        });
        loading.recv_timeout(Duration::from_secs(5)).unwrap();
        started.worker.shutdown();
        assert!(exited.load(Ordering::Acquire));
        assert!(started.worker.thread.is_none());
    }

    #[test]
    fn plugin_destruction_joins_while_an_editor_keeps_shared_state_alive() {
        let plugin = crate::NamPlugin::default();
        let shared = plugin.shared.clone();
        let stop = plugin.worker.stop.clone();
        drop(plugin);
        assert!(stop.load(Ordering::Acquire));
        assert_eq!(
            Arc::weak_count(&shared.engine),
            0,
            "worker must release its weak reference before destruction returns"
        );
    }
}
