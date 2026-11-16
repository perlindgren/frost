//! The peak of the waveform *between* the samples.
//!
//! A meter that reports the largest `|x|` it was handed does not report the
//! level of the signal. A band-limited signal moves between its samples, and at
//! audio rates it moves a lot: a full-scale sinusoid at a quarter of the sample
//! rate is sampled four times per cycle, so its samples peak at `cos(π/4)` —
//! 3.01 dB below the crest of the wave that produced them. Nothing in `max |x|`
//! can see the difference between that signal and one recorded a third of a
//! cycle later, whose samples do reach full scale. The number that decides
//! whether a converter clips, or whether a file will clip on someone else's
//! device after one more resample, is the crest of the reconstruction, and that
//! is what [`TruePeak`] measures: the industry's inter-sample peak, the
//! dBTP of ITU-R BS.1770. Measured on the case above — a full-scale tone a
//! quarter cycle out of step with the samples — the sample peak reports 0.707
//! and this reports 0.979.
//!
//! # Design
//!
//! The estimate is the same shape BS.1770-4 prescribes — 4× oversampling done
//! as a polyphase filter, so no sample is ever actually produced at 4× the
//! rate — with coefficients generated here rather than copied from the ITU
//! table. For every
//! input sample `x[n]`, four candidate levels are considered and the largest
//! becomes the running peak.
//!
//! * The signal at the sample instants themselves, `|x[n]|`.
//! * The signal a quarter, a half and three quarters of a sample *after* it,
//!   from three 8-tap filters reading the current sample and the seven stored
//!   in [`TruePeak::hist`].
//!
//! The four filters are the four phases of one 33-tap Blackman-windowed sinc
//! whose cutoff sits at exactly the input Nyquist. Two properties fall out of
//! that cutoff choice, and the whole design is built around keeping them.
//!
//! **The first phase is a sample, not a filter.** The zeros of
//! `sin(πx)/(πx)` land on exactly the nonzero integers, and the first phase
//! reads the prototype at exactly the integer offsets, so it survives as a
//! single unit tap: it *is* `|x[n]|`, bit for bit. That is why the first
//! bullet above costs nothing — the largest sample value we already compute on
//! the way to RMS *is* one of the four phases, and the three `f32` tables below
//! are the only arithmetic the estimator adds. It also means the true peak can
//! never read below the raw sample peak a plugin used to publish: this is a
//! strict improvement on the old number, never a surprise.
//!
//! **No phase has gain above unity.** Each table is normalised to 1.0 at DC and
//! its cutoff at Nyquist rejects the images either side of it, so no steady
//! tone — however loud *within* full scale, however badly placed between the
//! samples — is read above its own amplitude. Measured on the tables below,
//! over 20 001 frequencies: the largest `|G|` of the three filters is
//! 0.9999960, and together with the exact-unity first phase the maximum is
//! exactly 1.0. Run through the real kernel, a full-scale sinusoid at 200
//! frequencies and 16 phases each is never read above 1.00000. This is what
//! makes a red warning at 0 dBFS trustworthy: the meter cannot go red because
//! of where the samples happened to fall.
//!
//! The tables are then scaled so their `f32` tap sums land a few ulps under 1.0
//! (see `HEADROOM` in the tests). On a constant input each
//! branch's dot product then rounds *below* the input, and the exact first
//! phase supplies it, so a constant input — including silence — reads exactly
//! its own value rather than a hair above it. Every plugin test that pins a
//! published peak with `assert_eq!` depends on that.
//!
//! # Known limits, measured
//!
//! Precision is bought with a bias, and the bias is deliberately one-sided:
//! under-reading costs headroom, over-reading costs trust.
//!
//! * A steady tone below 0.45 of the sample rate is read at worst 0.694 dB low
//!   (0.688 dB of that is the floor of any 4× grid, `20·log₁₀cos(π/8)`; the
//!   filters add the last 0.006 dB). Over-reading a tone *never* happens, which
//!   is the Design claim above.
//! * A steady tone exactly at Nyquist is read 6.02 dB low if it arrives at the
//!   quadrature where its samples cancel — samples ≈ 0, waveform at full
//!   scale. The half-sample phase is symmetric, so its taps sum to zero at
//!   that frequency and the two surviving quarter phases read 0.5. That blind
//!   spot belongs to the 4× grid, not to these coefficients — it is in the ITU
//!   meter too, and a signal sitting exactly at Nyquist is a pathological
//!   one — but it is wider for the raw sample peak this replaced, which reads
//!   the same signal as silence.
//! # Above full scale is a reading, not an error
//!
//! What this reports against is the ±1.0 *sample grid*, and the grid is not a
//! ceiling on the waveform. A signal reconstructed from samples that never
//! leave ±1 crests above it — at every abrupt edge, in every converter — and
//! that crest is exactly what dBTP exists to report. A reading over 0 dBFS is
//! therefore the estimator naming a real overshoot of the sample range, not
//! stepping outside its own. Nothing here needs an apology: a square edge at
//! 0 dBFS turns the meter red and a full-scale sine cannot, which is the
//! correct ordering of those two signals.
//!
//! Measured on the starkest case, a full-scale step from silence: this
//! estimator reads 1.0976, +0.81 dB. The ideal band-limited reconstruction of
//! the same samples crests at 1.1411, +1.15 dB (continuous, a quarter sample
//! before the half-way instant; on the 4× grid the nearest phase is that
//! half-sample instant, `1/2 + 2/π` = 1.1366, +1.11 dB). So the estimator reads
//! the ringing 0.30 dB *low* — the same one-sided bias it carries everywhere,
//! here because a 33-tap Blackman window cannot hold up the ideal filter's full
//! Gibbs overshoot. It is the conservative side of the reference on the edges
//! where being wrong in that direction is the safe one.
//!
//! One consequence of the same physics is operational rather than numerical: a
//! constant input reaches an estimator with an empty history as an onset, so it
//! reads 1.0976 times itself for the first few samples until the history
//! fills. That is the step response above, met by a signal that began with an
//! edge. Code that wants the settled number should hand the estimator a full
//! [`TAPS`] window first — as
//! `a_constant_input_reads_exactly_itself_once_it_has_settled` does.
//!
//! # Cost
//!
//! Eight multiplies per stored phase: 24 `f32` MAC plus an absolute value and a
//! maximum per sample per channel, about 1.15 million MAC per second per
//! channel at 48 kHz, and 6 144 MAC for a 128-frame stereo block. The state is
//! 7 `f32` of history and 1 of peak, the coefficients are 24 `f32` of static
//! data, and the loop has no allocation, no division and no data-dependent
//! branch, so it is real-time safe by construction, witnessed by
//! `tests/true_peak_never_allocates.rs`.
//!
//! Measured, in release, on 4096 128-frame stereo blocks of tone plus burst
//! (`examples/true_peak_cost.rs`, fastest of seven runs, a million samples per
//! channel streamed block after block): 0.95 ns per sample per channel, 242 ns
//! per stereo block, 0.0091 % of one core at 48 kHz, on a 32-thread Ryzen 9
//! 7950X3D. The target matters here: the same example built for the plain
//! `x86-64` baseline instead of the `x86-64-v3` this workspace is compiled for
//! costs 1.64 ns per sample and 420 ns per block, because at the baseline the
//! 24-tap inner loop stays scalar while the wider target folds the three phase
//! branches into FMA lanes. Both readings are real; the cheaper one is the one
//! the rack actually ships with.
//!
//! The `max |x|` fold this replaced clocks 0.067 ns per sample on the same
//! buffers, 0.130 ns at the baseline, so the estimator is 12 to 14 times a plain
//! fold — worth naming rather than smoothing over, and worth reading with its
//! reason: a reduction has no dependency between samples and the compiler widens
//! it to SIMD even at the baseline, while this loop's history is a recurrence the
//! width of the tap count. Twelve times 242 ns is still 0.0002 ms of the
//! 2.67 ms a 128-frame block has at 48 kHz, and the meter is one pass over
//! buffers the plugin already walked.
//!
//! ## Measured inside the rack
//!
//! Those are the figures for the loop alone. What a metered wire actually costs
//! is the fold, the two gain multiplies, the peak read and the atomic publish
//! together, in `WireLevels`, on 128-frame stereo blocks whose buffers are
//! already in L1 because the graph just walked them. There the estimator lands
//! within a few percent of the example, and the fold read in place costs 62 ns
//! per block — 0.24 ns per sample, more than the standalone fold measured above,
//! because the same fold reads differently depending on what the compiler builds
//! beside it. The ratio is therefore somewhere between 4 and 14 depending on
//! context; the absolute numbers are the useful ones:
//!
//! | per 128-frame stereo block | cost | share of a 48 kHz block |
//! | --- | --- | --- |
//! | one metered wire end to end | 314 ns | 0.012 % |
//! | its true-peak pass alone | 240 ns | 0.009 % |
//! | its fold of absolute values alone | 62 ns | 0.002 % |
//! | an rms fold for comparison | 140 ns | 0.005 % |
//! | both channels through a 0.5 trim | 6 ns | |
//! | one buffer copied between two cards | 8 ns | |
//! | publishing two wires | 1 ns | |
//! | clearing a two-channel buffer | 29 ns | |
//!
//! Eight pass-through cards, metered, cost 5.6 µs of the block — 0.21 % of a
//! core — and the metering part of that is 0.32 µs per wire, linear in the number
//! of wires, which is what a per-sample filter should do. Nothing else in the
//! plumbing is above 100 ns per wire.
//!
//! ## Measured against real plugins
//!
//! Against the DSP the meter sits behind, the same bill disappears. These are
//! real CLAP cards loaded through the host, one chain of 128-frame stereo
//! blocks, once with `WireLevels` attached and once with an empty one, medians
//! of 40 repeats on the same machine:
//!
//! | card | one card, dark | one card, metered | four cards, dark | four cards, metered |
//! | --- | --- | --- | --- | --- |
//! | Nice Echo | 12.0 µs | 12.9 µs | 42.5 µs | 46.8 µs |
//! | Nice Pi-Fase | 12.3 µs | 13.1 µs | 40.4 µs | 42.2 µs |
//! | Nice Wah | 2.8 µs | 3.5 µs | 11.3 µs | 14.1 µs |
//! | Nice Ecomplex | 2.9 µs | 3.5 µs | 11.1 µs | 13.0 µs |
//! | Nice Phase | 2.5 µs | 3.2 µs | 10.0 µs | 12.9 µs |
//! | Nice Tuner | 0.4 µs | 1.0 µs | 1.4 µs | 4.1 µs |
//!
//! The extra time per metered wire comes out between 0.22 µs and 0.54 µs on
//! every one of them, clustered at 0.33 to 0.36 µs — the standalone pass plus a
//! little, and no part of it depends on what the plugin does. That is 0.024 % of
//! a core per card, so the honest summary is a percentage in two directions: it
//! adds 4 to 10 % to a card that works for a living, 17 to 29 % to a cheap one,
//! and it nearly triples the block time of a card that does almost nothing,
//! because 0.6 µs of metering in front of 0.4 µs of DSP is the one pairing where
//! the meter is the expensive part. A rack full of tuners would notice; a rack
//! with something audible in it will not. These are single-host numbers, and any
//! of them can be re-taken by driving the rack's `PreparedRack` twice, once with
//! a registered `WireLevels` and once with an unregistered one, at
//! `NICE_RACK_TEST_CLAP`.

/// Oversampling factor of the filter bank: one branch per quarter sample.
pub const OVER: usize = 4;

/// Taps per branch, including the current sample.
pub const TAPS: usize = 8;

/// The three non-trivial phases, oldest-last and newest-first: `t[0]` multiplies
/// the current sample, `t[1]` the previous one, and so on.
///
/// Generated by `examples/true_peak_taps.rs`
/// (`cargo run -p nice-common --example true_peak_taps --release`), which
/// derives them from a Blackman-windowed sinc with cutoff at the input Nyquist,
/// normalises each phase to unity at its own DC gain, and scales it so its
/// `f32` tap sum sits just under 1.0. The third phase is the first reversed and
/// the second is symmetric; the tests recompute the recipe and compare every
/// bit, so a tap cannot drift without a failure.
const BRANCHES: [[f32; TAPS]; OVER - 1] = [
    [
        -0.00021106089,
        0.00913082,
        -0.057038095,
        0.25990182,
        0.8858624,
        -0.12008928,
        0.024854159,
        -0.002414732,
    ],
    [
        -0.0013295488,
        0.021896731,
        -0.11764908,
        0.5970799,
        0.5970799,
        -0.11764908,
        0.021896731,
        -0.0013295488,
    ],
    [
        -0.002414732,
        0.024854159,
        -0.12008928,
        0.8858624,
        0.25990182,
        -0.057038095,
        0.00913082,
        -0.00021106089,
    ],
];

/// Running inter-sample peak of one channel's signal.
///
/// One value per channel: a stereo site holds a two-element array rather than
/// folding both channels into one history, which would mix a left-phase sample
/// into a right-phase prediction.
///
/// ```
/// use nice_common::true_peak::TruePeak;
///
/// // A full-scale sinusoid at a quarter of the sample rate, one quarter cycle
/// // out of step with the sample instants: its samples peak at 0.707 and a
/// // sample peak would report −3.01 dBFS, while the waveform it describes is
/// // at full scale and the reconstruction this measures crests at 0.979.
/// let mut tp = TruePeak::new();
/// let mut samples = [0.0f32; 96];
/// for (n, s) in samples.iter_mut().enumerate() {
///     *s = (std::f32::consts::FRAC_PI_2 * n as f32 + std::f32::consts::FRAC_PI_4).sin();
/// }
/// tp.process(&samples);
/// assert!(tp.peak() > 0.97);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct TruePeak {
    /// The previous samples, newest first: `hist[i]` is the input `i + 1`
    /// samples back. Zero before the first sample, so a signal arriving from
    /// silence is filtered as the onset it is.
    hist: [f32; TAPS - 1],
    /// Highest level seen since the last [`take`](Self::take), linear, where
    /// 1.0 is full scale. Unlike a sample peak this may exceed 1.0: a
    /// reconstruction is allowed to crest above the samples that define it.
    peak: f32,
}

impl Default for TruePeak {
    fn default() -> Self {
        Self::new()
    }
}

impl TruePeak {
    /// An estimator with no history and no peak, ready for a fresh stream.
    pub const fn new() -> Self {
        Self {
            hist: [0.0; TAPS - 1],
            peak: 0.0,
        }
    }

    /// Feed one block of one channel, in order.
    ///
    /// Call it per channel per block; the history carries the peak across the
    /// block boundary, so a crest that straddles two callback blocks is still
    /// found. Splitting a stream across calls of any lengths gives the same
    /// result as one call.
    pub fn process(&mut self, block: &[f32]) {
        let mut hist = self.hist;
        let mut peak = self.peak;
        for &x in block {
            // Phase 0: the sample itself, a unit tap, so this is the exact
            // peak the plugins measured before and the floor of this one.
            let mut level = x.abs();
            for t in BRANCHES {
                let mut acc = t[0] * x;
                for (i, &h) in hist.iter().enumerate() {
                    acc += t[i + 1] * h;
                }
                level = level.max(acc.abs());
            }
            peak = peak.max(level);
            for i in (1..TAPS - 1).rev() {
                hist[i] = hist[i - 1];
            }
            hist[0] = x;
        }
        self.hist = hist;
        self.peak = peak;
    }

    /// The highest level since the last [`take`](Self::take), linear, full
    /// scale at 1.0, and possibly above it.
    pub fn peak(&self) -> f32 {
        self.peak
    }

    /// Read the peak and start the next block's measurement, keeping the
    /// history. This is what an audio callback does before publishing.
    pub fn take(&mut self) -> f32 {
        let peak = self.peak;
        self.peak = 0.0;
        peak
    }

    /// Forget everything: the peak and the tail of the previous stream. Call it
    /// when the stream restarts — a configured-rate change, or a host flush —
    /// so a resumed signal is not filtered against samples that no longer
    /// exist.
    pub fn reset(&mut self) {
        self.hist = [0.0; TAPS - 1];
        self.peak = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Half-life of the deliberate headroom: the tap sums are pushed this far
    /// under unity so that `f32` accumulation cannot carry a branch past the
    /// value a constant input holds it at.
    const HEADROOM: f32 = 1.0 - 4e-6;

    fn blackman(n: usize) -> f64 {
        let w = 2.0 * std::f64::consts::PI * n as f64 / (4 * TAPS) as f64;
        0.42 - 0.5 * w.cos() + 0.08 * (2.0 * w).cos()
    }

    /// The shipped table, rebuilt from the recipe in `f64`. The example and
    /// this share the arithmetic step for step — same prototype, same division
    /// by each phase's own DC, same four refinement passes on the `f32` sum —
    /// so the comparison below is exact rather than approximate.
    fn recipe() -> [[f32; TAPS]; OVER - 1] {
        let centre = 2 * TAPS;
        let mut h = [0.0f64; 4 * TAPS + 1];
        for (i, v) in h.iter_mut().enumerate() {
            let x = (i as f64 - centre as f64) / OVER as f64;
            let s = if x == 0.0 {
                1.0
            } else {
                let p = std::f64::consts::PI * x;
                p.sin() / p
            };
            *v = s * blackman(i);
        }
        let mut out = [[0.0f32; TAPS]; OVER - 1];
        for (p, row) in out.iter_mut().enumerate() {
            let mut t: Vec<f64> = (0..TAPS).map(|k| h[p + 1 + k * OVER]).collect();
            let dc = t.iter().sum::<f64>();
            for v in t.iter_mut() {
                *v /= dc;
            }
            let mut scale = HEADROOM as f64;
            for _ in 0..4 {
                let sum: f64 = t.iter().map(|v| (*v * scale) as f32 as f64).sum();
                scale *= HEADROOM as f64 / sum;
            }
            for (k, v) in t.iter().enumerate() {
                row[k] = (*v * scale) as f32;
            }
        }
        out
    }

    #[test]
    fn tables_are_the_generated_ones_bit_for_bit() {
        let made = recipe();
        for p in 0..OVER - 1 {
            for k in 0..TAPS {
                assert_eq!(
                    BRANCHES[p][k].to_bits(),
                    made[p][k].to_bits(),
                    "branch {} tap {k} no longer matches the generator; re-run \
                     `cargo run -p nice-common --example true_peak_taps --release`",
                    p + 1
                );
            }
        }
    }

    #[test]
    fn every_branch_is_unity_dc_or_under() {
        // The headroom claim: no branch's f32 taps add up to more than 1.0, so
        // a constant input can only be pushed down by rounding, never up.
        for (p, t) in BRANCHES.iter().enumerate() {
            let sum: f32 = t.iter().sum();
            assert!(
                sum <= 1.0 && sum > 0.9999,
                "branch {} sums to {sum}, outside the headroom window",
                p + 1
            );
        }
    }

    #[test]
    fn a_constant_input_reads_exactly_itself_once_it_has_settled() {
        // The published peak is compared with `assert_eq!` by plugin tests, and
        // silence must stay silence. The exact unit tap carries the value; the
        // three filters are held below it by the headroom above.
        //
        // The measurement starts after the history has filled, because before
        // that the constant *is* a step, and a step is not band limited: the
        // ringing it reads is real (`an_abrupt_onset_rings_above_full_scale`).
        // The plugin tests that this protects feed ramps and tones, not steps,
        // and their first crest is well inside the settling window.
        for c in [1.0f32, 0.999_999_9, 0.75, 0.5, 0.123_456_78, 1e-7] {
            for sign in [1.0, -1.0] {
                let c = c * sign;
                let mut tp = TruePeak::new();
                tp.process(&[c; TAPS]);
                tp.take();
                tp.process(&[c; 64]);
                assert_eq!(tp.peak(), c.abs(), "constant {c} read {}", tp.peak());
            }
        }
        let mut tp = TruePeak::new();
        tp.process(&[0.0; 128]);
        assert_eq!(tp.peak(), 0.0);
    }

    /// Full-scale sinusoid at `freq` (in cycles per sample) starting at
    /// `phase`, measured over `frames` samples after `warmup` of settling.
    fn read_of_sine(freq: f64, phase: f64, frames: usize, warmup: usize) -> f32 {
        let mut tp = TruePeak::new();
        let mut worst = 0.0f32;
        for n in 0..frames {
            let x = (2.0 * std::f64::consts::PI * freq * n as f64 + phase).sin() as f32;
            tp.process(std::slice::from_ref(&x));
            let peak = tp.take();
            if n >= warmup {
                worst = worst.max(peak);
            }
        }
        worst
    }

    #[test]
    fn a_steady_tone_never_reads_above_its_own_amplitude() {
        // The property the red warning rests on: nothing inside full scale can
        // be pushed over it by where the samples happen to fall.
        for i in 1..=200 {
            let freq = i as f64 / 400.0;
            for j in 0..16 {
                let phase = std::f64::consts::PI * j as f64 / 8.0;
                let read = read_of_sine(freq, phase, 384, 128);
                assert!(
                    read <= 1.0,
                    "tone at {freq:.4} cycles/sample, phase {phase:.3} read {read}"
                );
            }
        }
    }

    #[test]
    fn the_under_read_below_half_nyquist_stays_inside_the_grid_floor() {
        // Deliberate, bounded bias: at worst the meter is 0.694 dB low, and
        // 0.688 dB of that is the floor every 4× estimator shares.
        let floor = 10f64.powf(-0.75 / 20.0) as f32;
        let mut worst = (1.0f32, 0.0f64);
        for i in 1..=90 {
            let freq = i as f64 / 200.0; // 0.005 … 0.45 of the sample rate
            for j in 0..8 {
                let phase = std::f64::consts::PI * j as f64 / 4.0;
                let read = read_of_sine(freq, phase, 384, 128);
                if read < worst.0 {
                    worst = (read, freq);
                }
            }
        }
        assert!(
            worst.0 >= floor,
            "tone at {} cycles/sample read {} below the 0.75 dB budget",
            worst.1,
            worst.0
        );
    }

    #[test]
    fn a_crest_between_the_samples_is_found() {
        // A full-scale sinusoid at a quarter of the rate, a quarter cycle out of
        // step: every sample lands at `cos(π/4)`, 0.707, and the wave between
        // them is at full scale. This is the case the estimator exists for —
        // and the case the raw sample peak is structurally blind to.
        let mut samples = [0.0f32; 96];
        let mut raw = 0.0f32;
        for (n, s) in samples.iter_mut().enumerate() {
            let x = (std::f32::consts::FRAC_PI_2 * n as f32 + std::f32::consts::FRAC_PI_4).sin();
            *s = x;
            raw = raw.max(x.abs());
        }
        assert!(
            (raw - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3,
            "setup: raw peak {raw}"
        );
        let mut tp = TruePeak::new();
        tp.process(&samples);
        assert!(
            (tp.peak() - 0.9789).abs() < 2e-3,
            "read {}, expected the measured 0.9789 (raw peak {raw})",
            tp.peak()
        );
    }

    #[test]
    fn an_abrupt_onset_rings_above_full_scale() {
        // Documented, not assumed: 0.81 dB over full scale for a full-scale
        // step from silence. The ideal reconstruction of those same samples
        // crests higher still — 1.1366 at the half-sample instant of a 4× grid
        // (`1/2 + 2/π`), 1.1411 continuous — so this read sits 0.30 dB *under*
        // the ringing it describes, conservative here as everywhere else. The
        // module docs spell out why a read above 0 dBFS is a reading and not a
        // defect; the bound below is what would break if the coefficients were
        // ever re-tuned hotter than the reference.
        let mut input = [0.0f32; 32];
        input[8..].fill(1.0);
        let mut tp = TruePeak::new();
        tp.process(&input);
        assert!(
            (tp.peak() - 1.0976).abs() < 0.002,
            "step read {}, expected about 1.0976",
            tp.peak()
        );
        assert!(
            tp.peak() < 1.1366,
            "step read {} over the ideal reconstruction's own crest",
            tp.peak()
        );
    }

    #[test]
    fn an_impulse_reads_itself_and_rings() {
        let mut input = [0.0f32; 24];
        input[4] = 1.0;
        let mut tp = TruePeak::new();
        tp.process(&input);
        assert!(tp.peak() >= 1.0, "impulse read {}", tp.peak());
    }

    #[test]
    fn splitting_a_stream_across_calls_changes_nothing() {
        // The peak a callback publishes is per block, so the history has to
        // carry across: a crest straddling two 128-frame blocks is a real
        // crest of the signal.
        let mut input = [0.0f32; 512];
        for (n, s) in input.iter_mut().enumerate() {
            *s = (2.0 * std::f64::consts::PI * 0.42 * n as f64).sin() as f32 * 0.9;
        }
        let mut one = TruePeak::new();
        one.process(&input);
        let mut many = TruePeak::new();
        let mut worst = 0.0f32;
        for chunk in input.chunks(37) {
            many.process(chunk);
            worst = worst.max(many.take());
        }
        assert_eq!(one.peak(), worst);
    }

    #[test]
    fn take_clears_the_peak_but_keeps_the_history() {
        let mut tp = TruePeak::new();
        tp.process(&[0.5; TAPS]);
        tp.take(); // past the onset, so what is left is the settled level
        tp.process(&[0.5; 8]);
        assert_eq!(tp.take(), 0.5);
        assert_eq!(tp.peak(), 0.0);
        tp.process(&[0.0; 8]);
        // The tail of the 0.5 is still ringing into the silence.
        assert!(tp.peak() > 0.0);
        tp.reset();
        assert_eq!(tp.peak(), 0.0);
        tp.process(&[0.0; 8]);
        assert_eq!(tp.peak(), 0.0);
    }

    #[test]
    fn a_nan_cannot_stick_the_meter() {
        // `f32::max` ignores a NaN operand, so a poisoned sample neither marks
        // the peak nor survives in it, and the history is not the worst part
        // of the state: it leaves with the next seven samples.
        let mut tp = TruePeak::new();
        tp.process(&[0.25, f32::NAN, -f32::NAN, 0.0]);
        assert_eq!(tp.peak(), 0.25);
        tp.process(&[0.0; 16]);
        assert_eq!(tp.peak(), 0.25);
    }
}
