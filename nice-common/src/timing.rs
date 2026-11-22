//! Every-call worst-case wall-clock observations, independent of GUI polling.
//!
//! One processing thread owns a recorder. It performs bounded stores, never
//! allocation, locking, logging or retries. Snapshots are coherent or absent;
//! readers never wait for the writer. Maxima and counts last until the recorder
//! is replaced (normally when the audio session/graph is rebuilt).
use std::sync::atomic::{AtomicU64, Ordering, fence};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub calls: u64,
    pub overruns: u64,
    pub maximum_ns: u64,
    /// Maximum duration/budget ratio, which can come from a different call
    /// than maximum_ns when actual block sizes vary.
    pub maximum_budget: f64,
}

#[derive(Default)]
pub struct Published {
    version: AtomicU64,
    calls: AtomicU64,
    overruns: AtomicU64,
    maximum_ns: AtomicU64,
    maximum_budget: AtomicU64,
}
impl Published {
    pub fn snapshot(&self) -> Option<Snapshot> {
        let version = self.version.load(Ordering::Acquire);
        if version & 1 != 0 {
            return None;
        }
        let value = Snapshot {
            calls: self.calls.load(Ordering::Relaxed),
            overruns: self.overruns.load(Ordering::Relaxed),
            maximum_ns: self.maximum_ns.load(Ordering::Relaxed),
            maximum_budget: f64::from_bits(self.maximum_budget.load(Ordering::Relaxed)),
        };
        fence(Ordering::Acquire);
        (self.version.load(Ordering::Relaxed) == version).then_some(value)
    }
}

/// Local arithmetic preserves *every* call's peak; publication never samples
/// or averages away a spike. Owning the publisher enforces a single writer.
#[derive(Default)]
pub struct Recorder {
    value: Snapshot,
    published: std::sync::Arc<Published>,
}
impl Recorder {
    pub fn reader(&self) -> std::sync::Arc<Published> {
        self.published.clone()
    }
    pub fn record(&mut self, elapsed: Duration, budget_seconds: f64) {
        if !budget_seconds.is_finite() || budget_seconds <= 0.0 {
            return;
        }
        self.value.calls = self.value.calls.saturating_add(1);
        self.value.overruns = self
            .value
            .overruns
            .saturating_add(u64::from(elapsed.as_secs_f64() > budget_seconds));
        self.value.maximum_ns = self
            .value
            .maximum_ns
            .max(elapsed.as_nanos().min(u128::from(u64::MAX)) as u64);
        self.value.maximum_budget = self
            .value
            .maximum_budget
            .max(elapsed.as_secs_f64() / budget_seconds);
        let p = &self.published;
        p.version.fetch_add(1, Ordering::AcqRel);
        p.calls.store(self.value.calls, Ordering::Relaxed);
        p.overruns.store(self.value.overruns, Ordering::Relaxed);
        p.maximum_ns.store(self.value.maximum_ns, Ordering::Relaxed);
        p.maximum_budget
            .store(self.value.maximum_budget.to_bits(), Ordering::Relaxed);
        p.version.fetch_add(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_spike_between_reads_survives_and_short_blocks_use_their_own_budget() {
        let mut recorder = Recorder::default();
        let reader = recorder.reader();
        recorder.record(Duration::from_micros(500), 0.001);
        recorder.record(Duration::from_micros(900), 0.0005);
        recorder.record(Duration::from_micros(1), 0.001);
        let value = reader.snapshot().unwrap();
        assert_eq!(value.calls, 3);
        assert_eq!(value.overruns, 1);
        assert_eq!(value.maximum_ns, 900_000);
        assert!((value.maximum_budget - 1.8).abs() < 1e-12);
        recorder.record(Duration::from_secs(1), f64::NAN);
        assert_eq!(reader.snapshot(), Some(value));
    }
    #[test]
    fn readers_never_spin_and_replacement_resets_the_session() {
        let recorder = Recorder::default();
        recorder.published.version.store(1, Ordering::Release);
        assert_eq!(recorder.reader().snapshot(), None);
        assert_eq!(
            Recorder::default().reader().snapshot(),
            Some(Snapshot::default())
        );
    }
    #[test]
    fn concurrent_reads_are_coherent() {
        let mut recorder = Recorder::default();
        let reader = recorder.reader();
        let writer = std::thread::spawn(move || {
            for _ in 0..10_000 {
                recorder.record(Duration::from_millis(2), 0.001);
            }
        });
        while !writer.is_finished() {
            if let Some(value) = reader.snapshot() {
                assert_eq!(value.calls, value.overruns);
                if value.calls > 0 {
                    assert_eq!(value.maximum_ns, 2_000_000);
                    assert_eq!(value.maximum_budget, 2.0);
                }
            }
        }
        writer.join().unwrap();
        assert_eq!(reader.snapshot().unwrap().calls, 10_000);
    }
    #[test]
    fn recording_and_reading_do_not_allocate() {
        let mut recorder = Recorder::default();
        let reader = recorder.reader();
        let allocations = allocation_counter::measure(|| {
            recorder.record(Duration::from_micros(20), 64.0 / 48_000.0);
            std::hint::black_box(reader.snapshot());
        });
        assert_eq!(allocations.count_total, 0);
    }
    #[test]
    #[ignore = "manual release timing benchmark; scheduler-dependent, not a gate"]
    fn recording_overhead() {
        use std::{hint::black_box, time::Instant};
        let mut recorder = Recorder::default();
        let mut samples = Vec::with_capacity(100_000);
        for _ in 0..100_000 {
            let start = Instant::now();
            let section = Instant::now();
            black_box(());
            recorder.record(section.elapsed(), 64.0 / 48_000.0);
            samples.push(start.elapsed().as_nanos());
        }
        samples.sort_unstable();
        eprintln!(
            "two clock boundaries + recording, ns: median={} p99={} max={}",
            samples[50_000], samples[99_000], samples[99_999]
        );
    }
}
