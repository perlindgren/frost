//! Off-thread preparation and retirement of DSP instances.
use crate::{dsp::Chain, settings::NamPreferences};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, AtomicU64, Ordering},
};

pub struct EngineState {
    pub paths: Arc<Mutex<NamPreferences>>,
    pub rate: AtomicU32,
    pub status: Mutex<String>,
    pub accepted: Mutex<Option<NamPreferences>>,
    pub revision: AtomicU64,
}
pub fn start(
    paths: Arc<Mutex<NamPreferences>>,
) -> (
    Arc<EngineState>,
    rtrb::Consumer<Box<Chain>>,
    rtrb::Producer<Box<Chain>>,
) {
    let state = Arc::new(EngineState {
        paths,
        rate: AtomicU32::new(48000),
        status: Mutex::new("No model loaded".into()),
        accepted: Mutex::new(None),
        revision: AtomicU64::new(0),
    });
    let weak = Arc::downgrade(&state);
    let (mut ready, incoming) = rtrb::RingBuffer::new(2);
    let (retiring, mut retired) = rtrb::RingBuffer::<Box<Chain>>::new(8);
    std::thread::spawn(move || {
        let mut previous = None;
        let mut pending = None;
        loop {
            while let Ok(chain) = retired.pop() {
                drop(chain);
            }
            let Some(state) = weak.upgrade() else {
                break;
            };
            let paths = state.paths.lock().unwrap().clone();
            let rate = state.rate.load(Ordering::Acquire);
            let revision = state.revision.load(Ordering::Acquire);
            if previous.as_ref() != Some(&(paths.clone(), rate, revision)) {
                previous = Some((paths.clone(), rate, revision));
                *state.status.lock().unwrap() = "Preparing model and cabinet…".into();
                match Chain::load(&paths, rate) {
                    Ok((chain, description)) => {
                        pending = Some(Box::new(chain));
                        *state.accepted.lock().unwrap() = Some(paths.clone());
                        *state.status.lock().unwrap() = description;
                    }
                    Err(error) => {
                        *state.status.lock().unwrap() =
                            format!("{error}\nThe previous working chain is retained.");
                    }
                }
            }
            if ready.slots() > 0
                && let Some(chain) = pending.take()
            {
                let _ = ready.push(chain);
            }
            drop(state);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    });
    (state, incoming, retiring)
}
