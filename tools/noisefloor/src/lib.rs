//! How busy was the machine *while* this number was being taken?
//!
//! A benchmark that reports a millisecond figure without reporting the
//! weather around it is reporting half a fact. This crate measures the other
//! half with a **canary** thread that reports what two small things cost:
//!
//! * every millisecond it asks to be woken on a deadline and records how late
//!   the wake was — the responsiveness a frame needs from the scheduler;
//! * every tenth of those, it runs a fixed quantum of floating-point work and
//!   records how long that took — the throughput a render thread needs from a
//!   core.
//!
//! The second signal is here because the first one was not enough, measured
//! rather than guessed: on the 14-core machine this project develops on,
//! eight spinning threads moved the wake lateness not at all (p50 0.20 ms
//! against 0.21 ms idle), because a *sleeping* thread that finally wakes finds
//! a free core. A render thread does not sleep. It runs, and a core is what
//! gets rationed when the machine is busy with something else. Lateness alone
//! would have called a contended machine quiet.
//!
//! The number exists to be read *with* a measurement, and to let the
//! measurement refuse. A bench that prints `REGRESSED` because a laptop
//! decided to back up is worse than one that says it cannot judge: both
//! mislead, but the second is honest about it and costs a rerun rather than an
//! afternoon.
//!
//! Deliberately small: one thread, one sleep loop, two sorts, and no
//! dependencies at all — including no `frost`, because an instrument that
//! borrows its subject's dependencies measures those instead.
//!
//! ```
//! let canary = noisefloor::Canary::start();
//! # std::thread::sleep(std::time::Duration::from_millis(30));
//! let floor = canary.report();
//! println!("noise floor {floor}");
//! ```

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How often the canary probes the scheduler: a thousand wakes a second, so
/// a stall the length of a frame is tens of samples.
///
/// The interval is a millisecond because that is what the measurement was
/// worth: at 10 ms the same code read 2.0 ms of lateness on an idle machine
/// and the same 2.0 ms under load — macOS coalesces longer sleeps onto a
/// timer grid, and a number that cannot tell a quiet machine from a busy one
/// is not an instrument. At 1 ms the coalescing does not bite (0.21 ms p50,
/// 0.29 ms p99 idle).
pub const INTERVAL: Duration = Duration::from_millis(1);

/// How many wakes pass between quanta of arithmetic. Ten keeps the burn at a
/// few percent of one core — it must not become the load it is watching for —
/// while still taking a sample every ten milliseconds of a campaign.
pub const BURN_EVERY: u64 = 10;

/// How far above its own median the 99th-percentile wake may run before the
/// scheduler is called loud, in milliseconds.
///
/// The gate reads an *excess* rather than a lateness because the absolute
/// number is a property of the operating system, not of the moment: this
/// machine alone sleeps a fifth of a millisecond late at this cadence and a
/// Windows box would differ again, and a threshold that quietly means
/// "whichever OS is nicer to sleep on" judges nothing. Measured excess, idle
/// and at four load levels here, has sat between 0.08 and 0.62 ms; two is a
/// machine that is late on purpose. Note what this gate is *not*: it is a
/// hitch detector, and an interference rare enough to fall outside a p99 has to
/// be read off [`NoiseFloor::worst_ms`] by eye instead.
pub const WAKE_EXCESS_LIMIT_MS: f64 = 2.0;

/// How much worse a burn quantum's 99th percentile may be than its own median
/// before the cores are called rationed. Preemption is probabilistic, so
/// contention shows in the spread first: one interrupted quantum costs a whole
/// scheduler slice, several times its own length.
///
/// The measurements behind the number, six seconds of quanta per row on the
/// 14-core M4 Max this develops on: nothing running 1.8× to 2.5×, eight
/// spinners 1.8×, four gzip streams 2.0× to 2.2×, and twenty spinners
/// oversubscribing the cores 2.9× and 3.2× on two runs. So this is a detector
/// of *severe* rationing, set above the loudest quiet reading rather than
/// below the loudest loud one: the failure directions are not equal, because a
/// declined judgment costs a rerun while a false `REGRESSED` costs an afternoon
/// of believing a slowdown that was weather. A machine at 2.6× passes, and that
/// is the instrument's honest limitation, not a bug to tune away — with cores to
/// spare, one thread really is not being rationed, whatever the load average
/// says.
pub const BURN_SPREAD: f64 = 3.0;

/// The fixed quantum: a dependency chain of fused multiply-adds, which no
/// optimiser can vectorise (each result feeds the next) and which a core
/// retires at a rate that depends on nothing but that core being there. Tuned
/// on this machine to a couple of hundred microseconds; the level is not the
/// signal, the spread is, so the tuning only sets how cheap a sample is.
const BURN_OPS: u32 = 60_000;

/// What the canary saw. Every number is a lateness or a duration, and none of
/// them means anything without the count they came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoiseFloor {
    /// Wake samples taken. Zero means the canary never ran, which [`Display`]
    /// shows rather than hides.
    pub wake_samples: usize,
    /// Quantum samples taken — [`wake_samples`] divided by [`BURN_EVERY`],
    /// give or take one, because the burn is decimated.
    pub burn_samples: usize,
    /// Median wake lateness: this machine's ordinary responsiveness, timer
    /// slack included. Read it as a property of the OS, not of the moment.
    pub p50_ms: f64,
    /// 99th percentile wake lateness, raw. The gate reads [`Self::wake_excess`]
    /// instead; this is here so the excess can be checked by eye.
    pub p99_ms: f64,
    /// The single worst stall, kept because one 200 ms hitch is what a player
    /// feels and a percentile hides it by design — and because a stall rare
    /// enough to miss a p99 is not rare enough to miss a frame budget. Read
    /// this number when in doubt; the gate is a floor for the eye, not a
    /// substitute for it.
    pub worst_ms: f64,
    /// Median time the fixed quantum took, µs. Its *level* is a property of
    /// the machine and of which core the thread landed on — measured here at
    /// 53 µs on a performance core and 137 µs on an efficiency one, so a level
    /// is never a verdict on its own; [`Self::burn_spread`] is.
    pub burn_p50_us: f64,
    /// 99th percentile quantum time, µs. Work on other cores pushes this up
    /// long before it moves the median.
    pub burn_p99_us: f64,
    /// The worst quantum of them all.
    pub burn_worst_us: f64,
}

impl NoiseFloor {
    /// How far the tail of the wake series sits above its own middle, in
    /// milliseconds. Subtracting the median is what makes the number portable:
    /// a fifth of a millisecond of lateness is macOS being macOS, while two
    /// milliseconds above that machine's own norm is something else living on
    /// it.
    #[must_use]
    pub fn wake_excess(&self) -> f64 {
        (self.p99_ms - self.p50_ms).max(0.0)
    }

    /// How much worse the worst-in-class quanta are than the ordinary ones.
    /// A ratio, so it says nothing about how fast the machine is and only
    /// something about how it was treated while the sample was taken.
    #[must_use]
    pub fn burn_spread(&self) -> f64 {
        if self.burn_p50_us <= 0.0 {
            0.0
        } else {
            self.burn_p99_us / self.burn_p50_us
        }
    }

    /// Whether a measurement taken inside this floor may carry a verdict. Two
    /// independent readings, either one disqualifying: wakes running more than
    /// [`WAKE_EXCESS_LIMIT_MS`] above this machine's own norm, or quanta
    /// spread by more than [`BURN_SPREAD`]× their median.
    #[must_use]
    pub fn too_loud(&self) -> bool {
        self.wake_loud() || self.burn_loud()
    }

    /// The scheduler reading on its own.
    #[must_use]
    pub fn wake_loud(&self) -> bool {
        self.wake_samples > 0 && self.wake_excess() > WAKE_EXCESS_LIMIT_MS
    }

    /// The core-throughput reading on its own. A run with no quanta has no
    /// spread at all, so silence is not noise.
    #[must_use]
    pub fn burn_loud(&self) -> bool {
        self.burn_samples > 0 && self.burn_spread() > BURN_SPREAD
    }
}

impl fmt::Display for NoiseFloor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "wake p50 {:.2} / p99 {:.2} / worst {:.2} ms (excess {:.2}), burn p50 {:.0} / p99 {:.0} / worst {:.0} µs ({:.1}×), {} wakes and {} quanta",
            self.p50_ms,
            self.p99_ms,
            self.worst_ms,
            self.wake_excess(),
            self.burn_p50_us,
            self.burn_p99_us,
            self.burn_worst_us,
            self.burn_spread(),
            self.wake_samples,
            self.burn_samples
        )
    }
}

/// A canary in flight. Dropped without [`Canary::report`], it stops at its
/// next sample and its findings are discarded; the thread holds no lock and
/// leaks nothing but the vectors it was filling.
#[derive(Debug)]
pub struct Canary {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<(Vec<f64>, Vec<f64>)>>,
}

impl Canary {
    /// Starts watching. Call it before the thing being measured, not after: a
    /// witness that arrives late testifies about the weather afterwards.
    #[must_use]
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let watching = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("noisefloor-canary".to_string())
            // A thread that panics has failed to witness, and a silence that
            // means "the instrument died" must not read as "the machine was
            // quiet". Let it take the bench down with it.
            .spawn(move || {
                let mut late = Vec::new();
                let mut burn = Vec::new();
                let start = Instant::now();
                let mut due = start + INTERVAL;
                let mut tick = 0u64;
                loop {
                    let before = Instant::now();
                    if before < due {
                        std::thread::sleep(due - before);
                    }
                    let woke = Instant::now();
                    late.push(woke.saturating_duration_since(due).as_secs_f64() * 1e3);
                    // The quantum is timed after the wake, never before it:
                    // lateness is how the wake *found* the machine, and a burn
                    // folded into that measurement would report this thread's
                    // own arithmetic as somebody else's stall.
                    if tick.is_multiple_of(BURN_EVERY) {
                        burn.push(burn_quantum().as_secs_f64() * 1e6);
                    }
                    tick += 1;
                    // Scheduled from the deadline, not from the wake, so one
                    // long stall costs one sample instead of rescheduling
                    // every later one against a clock that has slipped.
                    due += INTERVAL;
                    if watching.load(Ordering::Relaxed) {
                        return (late, burn);
                    }
                }
            })
            .expect("the noise canary is the only thread this crate spawns");
        Self {
            stop,
            thread: Some(thread),
        }
    }

    /// Stops the canary, joins it, and reports what it saw. Blocking is the
    /// point: the join waits at most one interval, and the samples are the
    /// answer.
    #[must_use]
    pub fn report(mut self) -> NoiseFloor {
        self.stop.store(true, Ordering::Relaxed);
        let (late, burn) = self
            .thread
            .take()
            .expect("taken once, by this method")
            .join()
            .expect("the canary does not panic");
        summarize(&late, &burn)
    }
}

impl Drop for Canary {
    fn drop(&mut self) {
        // Nothing to wait for; the thread notices within one interval.
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// One fixed quantum of arithmetic, timed. `mul_add` keeps the chain
/// dependent on itself on purpose, and `black_box` keeps the loop from being
/// recognised as the nothing-it-produces-that-it-is.
fn burn_quantum() -> Duration {
    let start = Instant::now();
    let mut acc = 1.0f64;
    for i in 0..BURN_OPS {
        acc = acc.mul_add(1.0000001, i as f64 * 1e-9);
    }
    std::hint::black_box(acc);
    start.elapsed()
}

/// Medians, 99th percentiles and maxima of both series, by nearest rank. A day
/// of watching is 86 million wakes and 8.6 million quanta, which is a lot of
/// `f64` and a sort nobody has complained about yet — an hour-long campaign
/// would want a histogram instead, and would notice it immediately.
#[must_use]
pub fn summarize(late_ms: &[f64], burn_us: &[f64]) -> NoiseFloor {
    let late = sorted(late_ms);
    let burn = sorted(burn_us);
    NoiseFloor {
        wake_samples: late.len(),
        burn_samples: burn.len(),
        p50_ms: at(&late, 0.50),
        p99_ms: at(&late, 0.99),
        worst_ms: at(&late, 1.0),
        burn_p50_us: at(&burn, 0.50),
        burn_p99_us: at(&burn, 0.99),
        burn_worst_us: at(&burn, 1.0),
    }
}

fn sorted(values: &[f64]) -> Vec<f64> {
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    v
}

/// The nearest-rank percentile of a sorted slice: the value at `q × (n − 1)`,
/// rounded to the nearest index. Empty in, zero out.
fn at(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[(q * (sorted.len() - 1) as f64).round() as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_read_the_sorted_list_not_the_arrival_order() {
        // Shuffled on purpose: a percentile that happens to work on sorted
        // input is a percentile that will quietly lie about a live machine.
        let samples = [9.0, 1.0, 5.0, 3.0, 7.0, 2.0, 4.0, 8.0, 6.0, 0.0];
        let floor = summarize(&samples, &samples);
        assert_eq!((floor.wake_samples, floor.burn_samples), (10, 10));
        // rank = round(0.5 × 9) = 5, rank = round(0.99 × 9) = 9.
        assert_eq!(
            (floor.p50_ms, floor.p99_ms, floor.worst_ms),
            (5.0, 9.0, 9.0)
        );
        assert_eq!(
            (floor.burn_p50_us, floor.burn_p99_us),
            (5.0, 9.0),
            "both series go through the same rank maths"
        );
    }

    #[test]
    fn a_canary_that_never_ran_reports_silence_without_inventing_it() {
        let floor = summarize(&[], &[]);
        assert_eq!((floor.wake_samples, floor.burn_samples), (0, 0));
        assert_eq!(
            (
                floor.p50_ms,
                floor.p99_ms,
                floor.worst_ms,
                floor.burn_p50_us,
                floor.burn_p99_us,
                floor.burn_worst_us
            ),
            (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        );
        assert!(!floor.too_loud(), "no samples is not evidence of noise");
        assert!(
            floor.to_string().contains("0 wakes and 0 quanta"),
            "a silent instrument must say so out loud: {floor}"
        );
    }

    #[test]
    fn the_gate_reads_the_tail_of_each_series_separately() {
        // A scheduler that misses one wake in forty, however steady its
        // median, is a scheduler that will drop a frame — and the cores are
        // untouched by it.
        let late = {
            let mut late = vec![0.1; 400];
            // Ten of four hundred is 2.5%, which is inside a p99; four would
            // be outside it, and the test would pass for the wrong reason.
            late[390..400].fill(40.0);
            late
        };
        let stalling = summarize(&late, &vec![200.0; 400]);
        assert_eq!(stalling.p50_ms, 0.1);
        assert!(stalling.wake_loud(), "{stalling}");
        assert!(!stalling.burn_loud(), "the quantum series is unchanged");
        assert!(stalling.too_loud());

        // And its mirror: wakes on time every time, cores rationed.
        let burn = {
            let mut burn = vec![200.0; 400];
            burn[390..400].fill(1_400.0);
            burn
        };
        let crowded = summarize(&vec![0.1; 400], &burn);
        assert!(
            !crowded.wake_loud(),
            "the wake series is unchanged: {crowded}"
        );
        assert!(
            crowded.burn_loud(),
            "the quantum tail outran its median: {crowded}"
        );
        assert!(crowded.too_loud());
    }

    #[test]
    fn ordinary_jitter_stays_inside_the_burn_gate() {
        // The gate must not fire on the arithmetic's own noise: a few percent
        // of wobble is a cache miss, not a neighbour.
        let late: Vec<f64> = (0..400).map(|i| 0.1 + (i % 3) as f64 * 0.01).collect();
        let burn: Vec<f64> = (0..400).map(|i| 200.0 + (i % 7) as f64 * 2.0).collect();
        let floor = summarize(&late, &burn);
        assert!(!floor.burn_loud(), "ordinary jitter: {floor}");
        assert!(!floor.too_loud());
    }

    #[test]
    fn a_sluggish_os_is_not_mistaken_for_a_busy_one() {
        // The whole reason the gate reads an excess. Every wake on this
        // machine is two milliseconds late — an operating system with a coarse
        // timer, a slow box, a virtualised guest, take your pick — and the
        // machine is still perfectly capable of judging a benchmark, because
        // nothing is *interrupting* it.
        let slow_but_steady = summarize(&vec![2.0; 400], &vec![90.0; 40]);
        assert_eq!(slow_but_steady.p50_ms, 2.0);
        assert!(
            !slow_but_steady.wake_loud(),
            "steady slack is not noise: {slow_but_steady}"
        );
        assert!(!slow_but_steady.too_loud());

        // The same machine with something living on it: same slack, tail
        // three milliseconds worse.
        let interrupted = {
            let mut late = vec![2.0; 400];
            // Ten of them, not four: a p99 over 400 samples reads the fifth
            // value from the top, so a four-sample tail is invisible to it by
            // construction — which is itself worth knowing before trusting
            // the gate with one-off stalls, and why `worst_ms` is printed.
            late[390..400].fill(5.0);
            summarize(&late, &vec![90.0; 40])
        };
        assert!(
            interrupted.wake_loud(),
            "3 ms above this machine's own norm is: {interrupted}"
        );
    }

    #[test]
    fn a_live_canary_wakes_burns_and_reports() {
        // Nothing here bounds how late or how slow the machine may be — that
        // is the thing the crate reports, and a test asserting it would fail
        // on precisely the loaded host it exists to catch. What this pins is
        // the plumbing: both series fill, the join returns them, and a
        // quantum of arithmetic arrives as a quantum rather than as zero.
        let canary = Canary::start();
        std::thread::sleep(Duration::from_millis(120));
        let floor = canary.report();
        assert!(
            floor.wake_samples >= 20,
            "120 ms of a 1 ms canary saw {} wakes",
            floor.wake_samples
        );
        assert!(
            floor.burn_samples >= 2,
            "the decimated quantum series saw {} quanta",
            floor.burn_samples
        );
        assert!(
            floor.burn_samples * (BURN_EVERY as usize) <= floor.wake_samples + BURN_EVERY as usize,
            "the burn is not being taken at {:?} as intended",
            INTERVAL * BURN_EVERY as u32
        );
        assert!(
            floor.worst_ms < 5_000.0,
            "a five second stall is a hang, not noise: {floor}"
        );
        assert!(
            floor.burn_p50_us > 0.0,
            "the quantum measured nothing at all: {floor}"
        );
    }

    #[test]
    fn dropping_a_canary_leaves_the_machine_measurable() {
        // The point of the Drop is that the abandoned thread stops being a
        // second witness, so what to check is not that it died — nobody is
        // listening for that — but that a canary started after it still sees
        // the machine rather than a wedged predecessor.
        drop(Canary::start());
        let second = Canary::start();
        std::thread::sleep(Duration::from_millis(30));
        assert!(second.report().wake_samples > 0);
    }
}
