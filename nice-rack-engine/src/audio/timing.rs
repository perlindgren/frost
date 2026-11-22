//! Optional per-card wall-clock observations. A fixed slot table is attached on
//! the control thread; processing only reads the monotonic clock and publishes
//! atomics. Readers never spin if they catch a publication in progress.
use crate::NodeId;
use std::sync::atomic::{AtomicU64, Ordering, fence};
use std::time::{Duration, Instant};

/// Finish observations on early processor-error returns as well as success.
pub(super) struct GraphObservation<'a> {
    pub times: &'a ProcessingTimes,
    pub started: Instant,
    pub budget: f64,
}
impl Drop for GraphObservation<'_> {
    fn drop(&mut self) {
        let end = Instant::now();
        self.times
            .publish_graph(end.duration_since(self.started), self.budget, end);
    }
}

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
#[derive(Default)]
struct Slot {
    key: AtomicU64,
    version: AtomicU64,
    incoming: AtomicU64,
    added: AtomicU64,
    updated: AtomicU64,
    calls: AtomicU64,
    overruns: AtomicU64,
    maximum_ns: AtomicU64,
    maximum_budget: AtomicU64,
}
pub struct ProcessingTimes {
    epoch: Instant,
    slots: Vec<Slot>,
    graph: Slot,
}
impl ProcessingTimes {
    pub fn new(capacity: usize) -> Self {
        Self {
            epoch: Instant::now(),
            graph: Slot::default(),
            slots: (0..capacity).map(|_| Slot::default()).collect(),
        }
    }
    /// Before the worker starts, with the previous worker joined.
    pub(super) fn register(&self, ids: &[NodeId]) {
        for (index, slot) in self
            .slots
            .iter()
            .chain(std::iter::once(&self.graph))
            .enumerate()
        {
            slot.version.fetch_add(1, Ordering::AcqRel);
            slot.updated.store(0, Ordering::Relaxed);
            slot.calls.store(0, Ordering::Relaxed);
            slot.overruns.store(0, Ordering::Relaxed);
            slot.maximum_ns.store(0, Ordering::Relaxed);
            slot.maximum_budget.store(0, Ordering::Relaxed);
            slot.key
                .store(ids.get(index).map_or(0, |id| id.0), Ordering::Release);
            slot.version.fetch_add(1, Ordering::Release);
        }
    }
    pub(super) fn publish(
        &self,
        index: usize,
        value: ProcessingTime,
        end: Instant,
        duration: Duration,
    ) {
        let Some(slot) = (if index == usize::MAX {
            Some(&self.graph)
        } else {
            self.slots.get(index)
        }) else {
            return;
        };
        slot.version.fetch_add(1, Ordering::AcqRel);
        slot.calls.store(
            slot.calls.load(Ordering::Relaxed).saturating_add(1),
            Ordering::Relaxed,
        );
        slot.overruns.store(
            slot.overruns
                .load(Ordering::Relaxed)
                .saturating_add(u64::from(value.total() > 1.0)),
            Ordering::Relaxed,
        );
        slot.maximum_ns.store(
            slot.maximum_ns
                .load(Ordering::Relaxed)
                .max(duration.as_nanos().min(u128::from(u64::MAX)) as u64),
            Ordering::Relaxed,
        );
        let peak = f64::from_bits(slot.maximum_budget.load(Ordering::Relaxed)).max(value.total());
        slot.maximum_budget.store(peak.to_bits(), Ordering::Relaxed);
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
    /// Every block contributes, even when no editor is open. Counts cover
    /// cumulative time through this card, not just its processor call.
    /// Reset by registration when the graph is replaced; retained while stopped.
    pub fn summary(&self, id: NodeId) -> Option<nice_common::timing::Snapshot> {
        let slot = self
            .slots
            .iter()
            .find(|slot| id.0 != 0 && slot.key.load(Ordering::Acquire) == id.0)?;
        Self::read_summary(slot)
    }
    pub fn graph_summary(&self) -> Option<nice_common::timing::Snapshot> {
        Self::read_summary(&self.graph)
    }
    fn read_summary(slot: &Slot) -> Option<nice_common::timing::Snapshot> {
        let version = slot.version.load(Ordering::Acquire);
        if version & 1 != 0 {
            return None;
        }
        let value = nice_common::timing::Snapshot {
            calls: slot.calls.load(Ordering::Relaxed),
            overruns: slot.overruns.load(Ordering::Relaxed),
            maximum_ns: slot.maximum_ns.load(Ordering::Relaxed),
            maximum_budget: f64::from_bits(slot.maximum_budget.load(Ordering::Relaxed)),
        };
        fence(Ordering::Acquire);
        (slot.version.load(Ordering::Relaxed) == version).then_some(value)
    }
    /// A coherent pair from one audio block. Suspended/stopped observations
    /// expire after 250 ms; a busy writer causes one missed UI frame, no wait.
    pub fn sample(&self, id: NodeId) -> Option<ProcessingTime> {
        let slot = self
            .slots
            .iter()
            .find(|slot| slot.key.load(Ordering::Acquire) == id.0 && id.0 != 0)?;
        self.read_sample(slot)
    }
    pub fn graph_sample(&self) -> Option<ProcessingTime> {
        self.read_sample(&self.graph)
    }
    pub(super) fn publish_graph(&self, duration: Duration, budget: f64, end: Instant) {
        self.publish(
            usize::MAX,
            ProcessingTime {
                incoming: 0.0,
                added: duration.as_secs_f64() / budget,
            },
            end,
            duration,
        );
    }
    fn read_sample(&self, slot: &Slot) -> Option<ProcessingTime> {
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
    fn early_returns_finish_the_graph_observation() {
        fn fail(times: &ProcessingTimes) -> Result<(), ()> {
            let _observation = GraphObservation {
                times,
                started: Instant::now(),
                budget: 0.001,
            };
            Err(())
        }
        let times = ProcessingTimes::new(0);
        assert!(fail(&times).is_err());
        assert_eq!(times.graph_summary().unwrap().calls, 1);
    }
    #[test]
    fn peaks_survive_quiet_blocks_and_graph_timing_does_not_need_plugins() {
        let times = ProcessingTimes::new(0);
        times.register(&[]);
        times.publish_graph(Duration::from_micros(900), 0.0005, Instant::now());
        times.publish_graph(Duration::from_micros(10), 0.001, Instant::now());
        let summary = times.graph_summary().unwrap();
        assert_eq!(summary.calls, 2);
        assert_eq!(summary.overruns, 1);
        assert_eq!(summary.maximum_ns, 900_000);
        assert!((summary.maximum_budget - 1.8).abs() < 1e-12);
        assert_eq!(times.graph_sample().unwrap().added, 0.01);
        times.register(&[]);
        assert_eq!(times.graph_summary().unwrap().calls, 0);
        assert!(times.graph_sample().is_none());
    }
    #[test]
    fn registrations_expiry_and_clipped_segments_keep_observations_honest() {
        let times = ProcessingTimes::new(2);
        times.register(&[NodeId(1), NodeId(3)]);
        assert_eq!(times.sample(NodeId(1)), None);
        let value = ProcessingTime {
            incoming: 0.3,
            added: 0.25,
        };
        times.publish(0, value, Instant::now(), Duration::from_micros(550));
        let peak = times.summary(NodeId(1)).unwrap();
        assert_eq!(peak.calls, 1);
        assert_eq!(peak.maximum_ns, 550_000);
        assert_eq!(peak.maximum_budget, 0.55);
        assert_eq!(times.sample(NodeId(1)), Some(value));
        assert_eq!(times.sample(NodeId(0)), None);
        times.slots[0].updated.store(1, Ordering::Relaxed);
        let stale = ProcessingTimes {
            epoch: Instant::now() - Duration::from_secs(1),
            slots: times.slots,
            graph: times.graph,
        };
        assert_eq!(stale.sample(NodeId(1)), None);
        stale.register(&[NodeId(3)]);
        assert_eq!(stale.sample(NodeId(1)), None);
        assert_eq!(stale.sample(NodeId(3)), None);
        assert_eq!(stale.summary(NodeId(3)).unwrap().calls, 0);
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
