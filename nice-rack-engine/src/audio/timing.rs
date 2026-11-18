//! Optional per-card wall-clock observations. A fixed slot table is attached on
//! the control thread; processing only reads the monotonic clock and publishes
//! atomics. Readers never spin if they catch a publication in progress.
use crate::NodeId;
use std::sync::atomic::{AtomicU64, Ordering, fence};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProcessingTime {
    /// Fraction of this block's deadline already spent before the card starts.
    pub incoming: f64,
    /// Fraction spent on this card's routing, trims, metering and processor.
    pub added: f64,
}
impl ProcessingTime {
    pub fn total(self) -> f64 {
        self.incoming + self.added
    }
    /// Visible fractions on a full-width 0..100% meter, including clipping.
    pub fn segments(self) -> (f32, f32) {
        let incoming = self.incoming.clamp(0.0, 1.0) as f32;
        let added = self.added.max(0.0).min(1.0 - f64::from(incoming)) as f32;
        (incoming, added)
    }
}
struct Slot {
    key: AtomicU64,
    version: AtomicU64,
    incoming: AtomicU64,
    added: AtomicU64,
    updated: AtomicU64,
}
pub struct ProcessingTimes {
    epoch: Instant,
    slots: Vec<Slot>,
}
impl ProcessingTimes {
    pub fn new(capacity: usize) -> Self {
        Self {
            epoch: Instant::now(),
            slots: (0..capacity)
                .map(|_| Slot {
                    key: AtomicU64::new(0),
                    version: AtomicU64::new(0),
                    incoming: AtomicU64::new(0),
                    added: AtomicU64::new(0),
                    updated: AtomicU64::new(0),
                })
                .collect(),
        }
    }
    /// Before the worker starts, with the previous worker joined.
    pub(super) fn register(&self, ids: &[NodeId]) {
        for (index, slot) in self.slots.iter().enumerate() {
            slot.updated.store(0, Ordering::Relaxed);
            slot.version.store(0, Ordering::Relaxed);
            slot.key
                .store(ids.get(index).map_or(0, |id| id.0), Ordering::Release);
        }
    }
    pub(super) fn publish(&self, index: usize, value: ProcessingTime, end: Instant) {
        let Some(slot) = self.slots.get(index) else {
            return;
        };
        slot.version.fetch_add(1, Ordering::AcqRel);
        slot.incoming
            .store(value.incoming.to_bits(), Ordering::Relaxed);
        slot.added.store(value.added.to_bits(), Ordering::Relaxed);
        slot.updated.store(
            end.duration_since(self.epoch)
                .as_nanos()
                .min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
        slot.version.fetch_add(1, Ordering::Release);
    }
    /// A coherent pair from one audio block. Suspended/stopped observations
    /// expire after 250 ms; a busy writer causes one missed UI frame, no wait.
    pub fn sample(&self, id: NodeId) -> Option<ProcessingTime> {
        let slot = self
            .slots
            .iter()
            .find(|slot| slot.key.load(Ordering::Acquire) == id.0 && id.0 != 0)?;
        let version = slot.version.load(Ordering::Acquire);
        if version & 1 != 0 {
            return None;
        }
        let updated = slot.updated.load(Ordering::Relaxed);
        let value = ProcessingTime {
            incoming: f64::from_bits(slot.incoming.load(Ordering::Relaxed)),
            added: f64::from_bits(slot.added.load(Ordering::Relaxed)),
        };
        fence(Ordering::Acquire);
        if slot.version.load(Ordering::Relaxed) != version {
            return None;
        }
        let now = self.epoch.elapsed().as_nanos();
        (updated != 0
            && now.saturating_sub(u128::from(updated)) < Duration::from_millis(250).as_nanos())
        .then_some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registrations_expiry_and_clipped_segments_keep_observations_honest() {
        let times = ProcessingTimes::new(2);
        times.register(&[NodeId(1), NodeId(3)]);
        assert_eq!(times.sample(NodeId(1)), None);
        let value = ProcessingTime {
            incoming: 0.3,
            added: 0.25,
        };
        times.publish(0, value, Instant::now());
        assert_eq!(times.sample(NodeId(1)), Some(value));
        assert_eq!(times.sample(NodeId(0)), None);
        times.slots[0].updated.store(1, Ordering::Relaxed);
        let stale = ProcessingTimes {
            epoch: Instant::now() - Duration::from_secs(1),
            slots: times.slots,
        };
        assert_eq!(stale.sample(NodeId(1)), None);
        stale.register(&[NodeId(3)]);
        assert_eq!(stale.sample(NodeId(1)), None);
        assert_eq!(stale.sample(NodeId(3)), None);
        let (incoming, added) = ProcessingTime {
            incoming: 0.8,
            added: 0.5,
        }
        .segments();
        assert!((incoming - 0.8).abs() < 1e-6 && (added - 0.2).abs() < 1e-6);
        assert_eq!(
            ProcessingTime {
                incoming: 1.2,
                added: 0.1
            }
            .segments(),
            (1.0, 0.0)
        );
    }
}
