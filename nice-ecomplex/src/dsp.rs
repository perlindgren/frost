//! Ecomplex's tape path: a tube-style front end feeding one drum-driven tape
//! loop with one playback head, in the shape of the Binson Echoplex — the
//! machine by Binson & Cerrone (France) that this crate models, the Echo
//! Drive of 1964 developed into the T.E.C. around 1970. Nothing here is a
//! reproduction of that machine: no schematic was consulted, nothing was
//! measured, and the numbers below are a player's model of how the machine
//! behaves, not its electrical behaviour.
//!
//! What the talk around the machine agrees on, and this model follows:
//!
//! * One motor turns one drum; the **Delay Time** knob changes the tape
//!   *speed*, so a longer delay is literally slower tape — and slower tape
//!   wobbles more. Delay and flutter are the same control, not two. (The
//!   Roland RE-201 later did the same trick; both differ from fixed-head
//!   designs that select delay by picking a head.)
//! * One playback head on the drum (longer loops and the rarer two-head
//!   variants are exactly that: variants). There is no mode switch, no bank
//!   of patterns, and no spring tank anywhere in the machine.
//! * A **Recount** control feeds the echo back toward the record head, up to
//!   self-oscillation. A **Timbre** control shapes the tone. An **Echo
//!   Volume** (often just *Volume* on the panel) sets the level of the
//!   repeats against a dry signal that is **always present** — the machine
//!   has no wet/dry crossfade and no dry-kill.
//! * A tube front end (the familiar 12AX7 stage) with its own microphone
//!   input: a hi-fi input by the standards of 1964, but one that softens —
//!   gently compresses, then soft-clips — when driven hard. The machine to
//!   compare it with on this point is its own solid-state successor, the
//!   RE-201, which does not do this.
//!
//! This model's own numbers are named assumptions, kept here where the code
//! can cite them and carried into `README.md` where the reader can find them:
//!
//! * **A-1** — Delay range: 40 ms to 3 s on an exponential taper. The real
//!   range came from the installed loop length and the speed range; only a
//!   service sheet could say more.
//! * **A-2** — The drum reference speed is 5 Hz at the fastest setting, and
//!   the wow rate tracks the drum, so it *slows down* as the delay lengthens
//!   (same motor, slower drum).
//! * **A-3** — Flutter depth: 0.8 ms at the fastest setting, growing toward
//!   about 3.17 ms at the slowest — a constant physical wobble is a bigger
//!   share of a longer delay window as the tape slows.
//! * **A-4** — The loop reaches unity gain at about three quarters of the
//!   Recount travel and the model stops at 1.15×; beyond unity the loop runs
//!   away on a real machine too, which clips instead of stopping. The
//!   feedback-path soft limiter here stands in for that clipping.
//! * **A-5** — The repeat band is one low-pass (300 Hz–12 kHz, exponential,
//!   under Timbre) and one fixed 90 Hz high-pass. The machine's tone stage is
//!   a real circuit; one-pole filters are this model's shorthand for it.
//! * **A-6** — The front-end curve is `x·(1+g)/(1+g·|x|)`: small signals see
//!   a clean gain of `1+g`, `x = 1` passes unchanged, and the output ceiling
//!   is `(1+g)/g`. `g` comes from COLOR squared (so the first half of the
//!   knob stays nearly clean) times 6. The mic trim scales up to ×2 and the
//!   guitar trim up to ×1 — nominal numbers, since each machine's trim is
//!   whatever the unit does.
//! * **A-7** — A plugin has one input bus where the machine has two jacks
//!   (mic and guitar) that mix inside it; the model feeds both trims from the
//!   same arriving signal, exactly as running one source into both jacks
//!   would. The trims therefore set drive into the front end, which is what
//!   they do on the panel.
//! * **A-8** — The wet taper is `Echo Volume² × 1.2`: a perceptually even
//!   knob with a little more level at the top. The dry path beside it is
//!   honest machine behaviour, not an assumption — it never fades out.
//!
//! Honest debts this file states but does not repay: when the machine moves
//! its TIME knob the tape speed changes, so the repeats audibly change pitch
//! while the delay settles; this model keeps repeat pitch stable and leaves
//! the transient smear to TODO.md. Repeated tape saturation between repeats
//! is likewise not modelled — the darkening here comes from the tone stage
//! alone.

/// The block the transport runs, and the number of samples every buffer in
/// this crate and its tests is sized by. The same 64 the whole family uses.
pub const BLOCK: usize = 64;

/// The extremes of the Delay Time knob, in seconds (**A-1**).
pub const DELAY_MIN_S: f32 = 0.040;
pub const DELAY_MAX_S: f32 = 3.0;

/// The Recount position at which this model's loop passes unity (**A-4**).
pub const RECOUNT_UNITY: f32 = 0.75;
/// How far past unity the model lets Recount push the loop before the travel
/// runs out (**A-4**).
pub const RECOUNT_MAX_GAIN: f32 = 1.15;

/// The Delay Time knob's exponential map, in seconds. Exposed because the
/// editor reads the knob in milliseconds — the unit the talk about the
/// machine is in — and a readout must come from the same map the transport
/// runs on, never from a second copy.
pub fn delay_seconds(knob: f32) -> f32 {
    let ratio = DELAY_MAX_S / DELAY_MIN_S;
    DELAY_MIN_S * ratio.powf(knob.clamp(0.0, 1.0))
}

/// The wow's rate and depth for a Delay Time knob position (**A-2**,
/// **A-3**): same motor, so slower drum = slower wow, and a longer delay
/// window so the same physical wobble covers more of it. Exposed for the
/// same reason as [`delay_seconds`] — and so a test can pin the monotonicity
/// the machine is known for: slower tape wobbles deeper and more slowly.
pub fn wow_profile(delay_knob: f32) -> (f32, f32) {
    let delay = delay_seconds(delay_knob);
    let speed_ratio = DELAY_MIN_S / delay; // 1.0 at the fastest setting
    let hz = 5.0 * speed_ratio;
    let depth_s = 0.0008 * (1.0 + 3.0 * (1.0 - speed_ratio));
    (hz, depth_s)
}

/// The Recount knob's loop gain (**A-4**).
pub fn feedback_gain(recount: f32) -> f32 {
    (recount.clamp(0.0, 1.0) / RECOUNT_UNITY).min(RECOUNT_MAX_GAIN)
}

/// The parameter snapshot the transport runs a block on. The host sees one
/// parameter each; this is that set read once per block, which is what lets
/// the sample loop touch no parameter and no atomics at all.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    /// Microphone trim: drive into the front end (**A-6**, **A-7**).
    pub mic_level: f32,
    /// Guitar trim: the same, for the instrument jack (**A-6**, **A-7**).
    pub gtr_level: f32,
    /// COLOR: the tube-style front end's drive. 0 is clean passthrough.
    pub color: f32,
    /// Delay Time: tape speed, coupled to wow exactly as on the machine.
    pub delay_time: f32,
    /// Recount: repeats back toward the record head, past unity at three
    /// quarters (**A-4**).
    pub recount: f32,
    /// Timbre: the tone of the repeats (**A-5**).
    pub timbre: f32,
    /// Echo Volume: the level of the repeats against the ever-present dry
    /// (**A-8**). There is no dry-kill to model because the machine has none.
    pub echo_volume: f32,
}

impl Default for Controls {
    /// The middle of the machine: unity through the input mixer at full
    /// scale (0.25 × 2 + 0.5 × 1 = 1, **A-6**), a front end just cracked
    /// open, a mid-length delay (about 350 ms on the A-1 taper), repeats
    /// that decay well short of howling, and Echo Volume half up. Every
    /// number here is this model's taste; none of them is a factory setting,
    /// because the machine has no recalls.
    fn default() -> Self {
        Self {
            mic_level: 0.25,
            gtr_level: 0.5,
            color: 0.2,
            delay_time: 0.5,
            recount: 0.4,
            timbre: 0.5,
            echo_volume: 0.5,
        }
    }
}

/// The front end of **A-6**: clean gain `1+g` for small signals, `x = 1`
/// passes unchanged, ceiling `(1+g)/g`. The shape — softening early,
/// compressing harder as the signal grows, clipping nowhere abruptly — is
/// the tube stage's reputation, not a measured curve.
fn preamp(x: f32, g: f32) -> f32 {
    x * (1.0 + g) / (1.0 + g * x.abs())
}

/// The feedback-path soft limiter standing in for the machine clipping
/// (**A-4**): under 1.0 nothing happens; above it, the excess is compressed
/// by `1 + excess` so the loop runs away loudly but stays finite.
fn loop_guard(x: f32) -> f32 {
    let a = x.abs();
    if a <= 1.0 {
        x
    } else {
        let over = a - 1.0;
        x.signum() * (1.0 + over / (1.0 + over))
    }
}

/// The `a` coefficient of a one-pole **low-pass** for corner `fc` at
/// `rate` — the `y += a·(x − y)` form, so `a` is the share of the new
/// sample admitted. Clamped so no setting can run past the sample rate.
fn low_pole(fc: f32, rate: f32) -> f32 {
    (1.0 - (-std::f32::consts::TAU * fc / rate.max(1.0)).exp()).clamp(0.0, 1.0)
}

/// The `a` coefficient of a one-pole **high-pass** for corner `fc` at
/// `rate` — the `y = a·(y + x − x′)` form, whose pole is `e^{−k}`, the
/// low-pass pole's complement. The two coefficients are *not* interchangeable
/// here: feeding the low-pass `a` to the high-pass form gives a filter whose
/// passband gain is `a` — a mute switch, not a 90 Hz tilt. (Found by test;
/// kept as a comment because it is the easiest mistake to re-make.)
fn high_pole(fc: f32, rate: f32) -> f32 {
    (-std::f32::consts::TAU * fc / rate.max(1.0))
        .exp()
        .clamp(0.0, 1.0)
}

/// What one block runs on: everything derived from `Controls` once, so the
/// sample loop is arithmetic only.
struct Block {
    input_gain: f32,
    pre_g: f32,
    delay_frames: f32,
    wow_step: f32,
    wow_depth_frames: f32,
    gain: f32,
    lp_a: f32,
    hp_a: f32,
    wet: f32,
}

/// The machine: one tape loop, one playback head, one tube-style front end.
pub struct Ecomplex {
    /// The tape, as a ring the head writes into while the playback head
    /// reads an interpolated position behind it.
    line: Vec<f32>,
    write: usize,
    rate: f32,
    /// Wow phase; its rate is re-derived from the block's Delay Time
    /// snapshot, so a knob move changes the wobble at the next block, not
    /// mid-sample.
    wow_phase: f32,
    /// The tone stage's memories (**A-5**): the low-pass output, and the
    /// high-pass' previous input and output.
    lp_state: f32,
    hp_prev: f32,
    hp_state: f32,
}

impl Ecomplex {
    /// A machine running at `rate`, armed from `controls`. The tape is sized
    /// for the A-1 ceiling at this rate, so no later knob move allocates.
    pub fn new(rate: f32, controls: &Controls) -> Self {
        let mut machine = Self {
            line: Vec::new(),
            write: 0,
            rate: 0.0,
            wow_phase: 0.0,
            lp_state: 0.0,
            hp_prev: 0.0,
            hp_state: 0.0,
        };
        machine.set_rate(rate);
        machine.configure(controls);
        machine
    }

    pub fn rate(&self) -> f32 {
        self.rate
    }

    /// Re-arm the tape for a new sample rate. Called at `activate` and on a
    /// rate change — lifecycle moments, not block-time — so this is where
    /// the one allocation of the machine's life happens.
    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate;
        let frames = (DELAY_MAX_S * rate).ceil() as usize + 8;
        self.line = vec![0.0; frames];
        self.write = 0;
        self.lp_state = 0.0;
        self.hp_prev = 0.0;
        self.hp_state = 0.0;
    }

    /// Per-block setup: everything derived from `Controls` that would be
    /// wasted work per sample. The wow's rate follows the drum (**A-2**), so
    /// it is recomputed here together with the delay it belongs to.
    fn configure(&self, controls: &Controls) -> Block {
        let (wow_hz, wow_depth_s) = wow_profile(controls.delay_time);
        let color = controls.color.clamp(0.0, 1.0);
        let echo_volume = controls.echo_volume.clamp(0.0, 1.0);
        Block {
            input_gain: controls.mic_level.clamp(0.0, 1.0) * 2.0
                + controls.gtr_level.clamp(0.0, 1.0), // A-6
            pre_g: color * color * 6.0, // A-6
            delay_frames: delay_seconds(controls.delay_time) * self.rate,
            wow_step: std::f32::consts::TAU * wow_hz / self.rate.max(1.0),
            wow_depth_frames: wow_depth_s * self.rate, // A-3
            gain: feedback_gain(controls.recount),     // A-4
            // The tone stage's poles (A-5): the low-pass follows Timbre
            // exponentially, 300 Hz to 12 kHz; the high-pass sits at a fixed
            // 90 Hz for the head's thinness.
            lp_a: low_pole(
                300.0f32 * (12_000.0f32 / 300.0).powf(controls.timbre.clamp(0.0, 1.0)),
                self.rate,
            ),
            hp_a: high_pole(90.0, self.rate),
            wet: echo_volume * echo_volume * 1.2, // A-8
        }
    }

    /// One block on one tape path. The dry signal is added after the machine
    /// and never enters it — the panel has no way to fade it out, so there is
    /// no code path that does.
    pub fn process_mono(&mut self, controls: &Controls, input: &[f32], output: &mut [f32]) {
        let block = self.configure(controls);
        let n = input.len().min(output.len());
        let tape_floor = self.line.len().saturating_sub(2).max(1) as f32;
        for i in 0..n {
            let dry = input[i];

            // Into the front end: both trims from one bus (A-7), then the
            // tube curve (A-6).
            let driven = preamp(dry * block.input_gain, block.pre_g);

            // The drum's wow: fundamental plus a little second harmonic so
            // the wobble is a machine, not a signal generator (A-2, A-3).
            self.wow_phase += block.wow_step;
            if self.wow_phase > std::f32::consts::TAU {
                self.wow_phase -= std::f32::consts::TAU;
            }
            let wow = self.wow_phase.sin() + 0.35 * (2.0 * self.wow_phase + 1.1).sin();

            // The playback head: an interpolated read, its distance from the
            // record head wobbling with the drum. Guarded so a knob move at
            // either end of the travel can never index off the tape.
            let read = (block.delay_frames + block.wow_depth_frames * wow).clamp(1.0, tape_floor);
            let played = self.read_interpolated(read);

            // The tone stage on the loop path (A-5): the repeat that will be
            // heard and the repeat that will be re-recorded are the same
            // toned signal, which is why dark Timbre darkens each generation
            // again — the machine's most familiar trick, arriving here for
            // free.
            let hp = block.hp_a * (self.hp_state + played - self.hp_prev);
            self.hp_prev = played;
            self.hp_state = hp;
            self.lp_state += block.lp_a * (hp - self.lp_state);
            let toned = self.lp_state;

            // Recount, guarded by the standing-in limiter (A-4), back to the
            // record head beside the freshly driven input; onto the tape.
            let fed = loop_guard(toned * block.gain);
            self.line[self.write] = driven + fed;
            self.write = (self.write + 1) % self.line.len();

            // And out: the wet repeats over the dry that is always there.
            output[i] = dry + toned * block.wet;
        }
    }

    /// The stereo shape is the mono machine in front of a stereo rig — the
    /// way players actually used it: the input sums to the one tape path,
    /// the dry keeps its own two lanes, and the echo arrives identically in
    /// both. A true two-tape spread is TODO.md debt, stated where a reader
    /// will look.
    pub fn process_stereo(
        &mut self,
        controls: &Controls,
        input: [&[f32]; 2],
        output: [&mut [f32]; 2],
    ) {
        let n = input[0]
            .len()
            .min(input[1].len())
            .min(output[0].len())
            .min(output[1].len());
        let mut summed = [0.0f32; BLOCK];
        let mut wetted = [0.0f32; BLOCK];
        let mut done = 0;
        while done < n {
            let m = (n - done).min(BLOCK);
            for i in 0..m {
                summed[i] = (input[0][done + i] + input[1][done + i]) * 0.5;
            }
            self.process_mono(controls, &summed[..m], &mut wetted[..m]);
            for i in 0..m {
                // The machine heard the sum and answered with `wetted`; the
                // wet share is that answer minus what went in, and it lands
                // in both lanes beside each lane's own dry.
                let wet = wetted[i] - summed[i];
                output[0][done + i] = input[0][done + i] + wet;
                output[1][done + i] = input[1][done + i] + wet;
            }
            done += m;
        }
    }

    /// Linear interpolation on the tape: cheap, and its high-frequency
    /// rolloff at fractional positions reads as tape smear rather than as a
    /// bug. `read` is frames behind the write head, already guarded.
    fn read_interpolated(&self, read: f32) -> f32 {
        let len = self.line.len() as i64;
        let base = (self.write as i64 - read as i64).rem_euclid(len);
        let next = (base - 1).rem_euclid(len) as usize;
        let frac = read - read.floor();
        let a = self.line[base as usize];
        let b = self.line[next];
        a + (b - a) * frac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed a whole input through the machine block by block, collecting the
    /// output — the shape every test below reads.
    fn render(machine: &mut Ecomplex, controls: &Controls, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(input.len());
        for chunk in input.chunks(BLOCK) {
            let n = chunk.len();
            let mut o = [0.0; BLOCK];
            machine.process_mono(controls, chunk, &mut o[..n]);
            out.extend_from_slice(&o[..n]);
        }
        out
    }

    #[test]
    fn delay_knob_spans_a_named_range_on_an_exponential_taper() {
        // A-1 endpoints, to the millisecond the assumption names.
        assert!((delay_seconds(0.0) * 1000.0 - 40.0).abs() < 0.01);
        assert!((delay_seconds(1.0) * 1000.0 - 3000.0).abs() < 0.1);
        // Exponential: the knob's middle is the geometric mean, not the
        // arithmetic one — sqrt(0.04 × 3) ≈ 0.346 s.
        let mid = delay_seconds(0.5);
        assert!((mid - (0.04f32 * 3.0).sqrt()).abs() < 1e-4, "mid = {mid}");
        // Monotone by construction of the powf, and worth pinning because
        // every other number in the machine hangs off this map.
        let mut last = 0.0;
        for i in 0..=64 {
            let v = delay_seconds(i as f32 / 64.0);
            assert!(v > last, "delay must grow with the knob");
            last = v;
        }
    }

    #[test]
    fn slower_tape_wobbles_deeper_and_more_slowly() {
        // A-2 and A-3: one motor, one drum. Fastest setting: 5 Hz, 0.8 ms.
        let (hz_fast, depth_fast) = wow_profile(0.0);
        assert!((hz_fast - 5.0).abs() < 1e-6);
        assert!((depth_fast - 0.0008).abs() < 1e-9);
        let mut last_hz = f32::INFINITY;
        let mut last_depth = 0.0;
        for i in 0..=32 {
            let (hz, depth) = wow_profile(i as f32 / 32.0);
            assert!(
                hz <= last_hz,
                "the drum never speeds up with a longer delay"
            );
            assert!(depth >= last_depth, "and never wobbles less");
            last_hz = hz;
            last_depth = depth;
        }
        // At the far end, the named numbers: a drum well below 0.1 Hz, and
        // the 3.17 ms depth A-3 claims (0.0008 × (1 + 3·(1 − 0.04/3))).
        let (hz_slow, depth_slow) = wow_profile(1.0);
        assert!(hz_slow < 0.1, "drum at the slowest setting: {hz_slow} Hz");
        assert!(
            (depth_slow * 1000.0 - 3.168).abs() < 0.01,
            "depth at the slowest setting: {depth_slow}"
        );
    }

    #[test]
    fn echo_volume_zero_leaves_the_dry_untouched() {
        // The machine's defining promise, as a test: the dry path never
        // fades out, so with the echo down the plugin is a straight wire —
        // however hard COLOR and the input trims drive the tape path, and
        // however close to self-oscillation the loop runs.
        let mut machine = Ecomplex::new(48_000.0, &Controls::default());
        let mut controls = Controls {
            echo_volume: 0.0,
            color: 0.9,
            mic_level: 1.0,
            recount: 0.9,
            ..Controls::default()
        };
        let input: Vec<f32> = (0..BLOCK * 8)
            .map(|i| (i as f32 * 0.37).sin() * 0.8)
            .collect();
        let out = render(&mut machine, &controls, &input);
        for (a, b) in out.iter().zip(&input) {
            assert_eq!(a, b, "a dry through the machine must be identity");
        }
        controls.timbre = 0.0;
        let out = render(&mut machine, &controls, &input);
        for (a, b) in out.iter().zip(&input) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn a_single_repeat_arrives_after_the_selected_delay() {
        // Recount to zero: one repeat, no regeneration. An impulse goes in;
        // energy must come out around delay_seconds(0.5) ≈ 346 ms at 48 kHz
        // ≈ frame 16611, inside the tolerance of one flutter half-width.
        let controls = Controls {
            recount: 0.0,
            color: 0.0,
            mic_level: 0.0,
            gtr_level: 1.0,
            echo_volume: 1.0,
            ..Controls::default()
        };
        let mut machine = Ecomplex::new(48_000.0, &controls);
        let mut input = vec![0.0; 48_000];
        input[0] = 1.0;
        let out = render(&mut machine, &controls, &input);
        let expected = delay_seconds(0.5) * 48_000.0;
        let (_, depth) = wow_profile(0.5);
        let window = (depth * 48_000.0) as usize + BLOCK;
        // Skip the dry impulse's own neighbourhood, then look for the peak.
        let search = (expected as usize - window).max(2 * BLOCK)
            ..(expected as usize + window).min(out.len());
        let peak = search.map(|i| out[i].abs()).fold(0.0f32, f32::max);
        // The one-pole tone and the fractional read shave the impulse's
        // single sample, so the bar is "an unmistakable repeat", not the
        // impulse's own height.
        assert!(
            peak > 0.15,
            "repeat should arrive near frame {expected}, saw {peak}"
        );
        // And beyond the repeat, silence: nothing fed back, nothing ringing —
        // once the tone stack itself has rung out. The fixed 90 Hz high-pass
        // is a pole at 0.988 per sample, so its impulse response needs tens of
        // milliseconds to fall below 1e-6; the tail window starts 50 ms after
        // the repeat to ask the loop, not the filters, whether it is dead.
        let tail_start = (expected as usize + window + 48_000 / 20).min(out.len() - 1);
        let tail = out[tail_start..].iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(tail < 1e-6, "a dead loop must go silent, tail = {tail}");
    }

    #[test]
    fn past_the_crossover_the_loop_runs_away_but_stays_finite() {
        // A-4: Recount at full sends the loop past unity; the limiter stands
        // in for the machine's clipping, so loud must still mean finite. A
        // 200 Hz tone is the fair witness — it survives the fixed 90 Hz
        // high-pass and the default Timbre nearly untouched, so what it
        // does over generations is the loop gain and nothing else.
        let controls = Controls {
            recount: 1.0,
            color: 0.0,
            echo_volume: 0.8,
            ..Controls::default()
        };
        assert!(feedback_gain(1.0) > 1.0);
        let mut machine = Ecomplex::new(48_000.0, &controls);
        let input: Vec<f32> = (0..48_000 * 4)
            .map(|i| (std::f32::consts::TAU * 200.0 * i as f32 / 48_000.0).sin() * 0.5)
            .collect();
        let out = render(&mut machine, &controls, &input);
        let peak_in = |r: std::ops::Range<usize>| out[r].iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let delay = (delay_seconds(0.5) * 48_000.0) as usize;
        let first = peak_in(2 * BLOCK..delay);
        let late = peak_in(out.len() - delay..out.len());
        assert!(
            late > first,
            "past unity the repeats grow: {first} then {late}"
        );
        assert!(
            out.iter().all(|x| x.is_finite()),
            "the loop must stay finite"
        );
        // The limiter compresses the excess by 1+excess, so the loop's own
        // ceiling is 2.0 before the wet taper; with headroom for the dry.
        assert!(late < 8.0, "…and bounded: {late}");
    }

    #[test]
    fn color_softens_a_hot_input_and_passes_small_signals_clean() {
        // A-6, both halves. Small signals: the stage is a clean `1+g`. The
        // probe has to be small against 1/g for the bend term to be invisible
        // at this tolerance — 1e-4 still bends 4e-3, which is the curve being
        // honest, not the test's slack being too tight.
        let g = 6.0; // color = 1.0
        let tiny = preamp(1e-6, g);
        assert!((tiny / 1e-6 - (1.0 + g)).abs() < 1e-3);
        // A hot signal: it flattens toward the named ceiling, never past it.
        let hot = preamp(10.0, g);
        assert!(hot < (1.0 + g) / g + 1e-6 && hot > (1.0 + g) / g * 0.95);
        // x = 1 passes unchanged — the curve pivots on unity.
        assert!((preamp(1.0, g) - 1.0).abs() < 1e-6);
        // And through the machine: a hot sine comes out bounded and alive.
        let controls = Controls {
            color: 1.0,
            echo_volume: 1.0,
            gtr_level: 1.0,
            mic_level: 0.0,
            ..Controls::default()
        };
        let mut machine = Ecomplex::new(48_000.0, &controls);
        let input: Vec<f32> = (0..48_000)
            .map(|i| (std::f32::consts::TAU * 200.0 * i as f32 / 48_000.0).sin() * 2.0)
            .collect();
        let out = render(&mut machine, &controls, &input);
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 0.5 && peak < 8.0, "driven peak {peak}");
    }

    #[test]
    fn darker_timbre_thins_the_repeats() {
        // A-5, as behaviour: feed noise into a short delay with no feedback
        // and compare one-generation repeats. The high-frequency proxy is
        // the mean square of the first difference — crude, but the ordering
        // it gives a low-pass is not up for argument.
        let hf_energy = |timbre: f32| -> f64 {
            let controls = Controls {
                timbre,
                recount: 0.0,
                color: 0.0,
                gtr_level: 1.0,
                mic_level: 0.0,
                echo_volume: 1.0,
                delay_time: 0.0, // 40 ms: well inside the test window
            };
            let mut machine = Ecomplex::new(48_000.0, &controls);
            // Deterministic pseudo-noise (an LCG), burst then silence.
            let mut state = 0x2545_f491_u32;
            let mut input = vec![0.0f32; 48_000 / 4];
            for x in input.iter_mut().take(1000) {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *x = ((state >> 8) as f32 / 8_388_608.0) - 1.0;
            }
            let out = render(&mut machine, &controls, &input);
            // Listen only in the repeat window, after the burst's own dry
            // tail: the burst ends at 1000, the repeat spans roughly
            // 1900…2900, so from 2000 the window is mostly echo.
            let from = 2000;
            out[from..]
                .windows(2)
                .map(|w| (w[1] - w[0]) as f64 * (w[1] - w[0]) as f64)
                .sum::<f64>()
                / (out.len() - from) as f64
        };
        let dark = hf_energy(0.0);
        let centre = hf_energy(0.5);
        let bright = hf_energy(1.0);
        assert!(dark < centre, "darker must thin: {dark} vs {centre}");
        assert!(centre < bright, "brighter must open: {centre} vs {bright}");
    }

    #[test]
    fn every_knob_at_every_position_stays_finite() {
        // The cheap sweep the family runs: no combination of settings may
        // produce NaN or infinity, including at the self-oscillation edge
        // and the deep-flutter edge at once.
        let mut machine = Ecomplex::new(44_100.0, &Controls::default());
        for i in 0i32..=10 {
            let t = i as f32 / 10.0;
            let controls = Controls {
                mic_level: t,
                gtr_level: 1.0 - t,
                color: t,
                delay_time: t,
                recount: t,
                timbre: t,
                echo_volume: t,
            };
            let input: Vec<f32> = (0..BLOCK * 4)
                .map(|s| ((s + i as usize) as f32 * 0.01).sin() * 0.5)
                .collect();
            let out = render(&mut machine, &controls, &input);
            assert!(out.iter().all(|x| x.is_finite()), "finite at t={t}");
        }
    }

    #[test]
    fn stereo_keeps_its_dry_lanes_and_shares_one_tape() {
        // The declared stereo shape, in both directions. With the echo down,
        // each lane passes through untouched — even when the two lanes
        // disagree, because the summed tape drives nothing.
        let n = BLOCK * 80; // long enough for a 40 ms delay to arrive
        let l: Vec<f32> = (0..n).map(|i| (i as f32 * 0.11).sin()).collect();
        let r: Vec<f32> = (0..n).map(|i| (i as f32 * 0.97).cos() * -0.5).collect();
        let mut ol = vec![0.0f32; n];
        let mut or_ = vec![0.0f32; n];
        let quiet = Controls {
            echo_volume: 0.0,
            color: 1.0,
            gtr_level: 1.0,
            ..Controls::default()
        };
        let mut machine = Ecomplex::new(48_000.0, &quiet);
        machine.process_stereo(&quiet, [&l, &r], [&mut ol, &mut or_]);
        assert_eq!(ol, l);
        assert_eq!(or_, r);
        // With the echo up, both lanes move — by exactly the same amount,
        // because there is one tape, not two.
        let loud = Controls {
            echo_volume: 0.9,
            delay_time: 0.0,
            ..quiet
        };
        let mut machine = Ecomplex::new(48_000.0, &loud);
        let mut ol = vec![0.0f32; n];
        let mut or_ = vec![0.0f32; n];
        machine.process_stereo(&loud, [&l, &r], [&mut ol, &mut or_]);
        let delta_l: Vec<f32> = ol.iter().zip(&l).map(|(o, x)| o - x).collect();
        let delta_r: Vec<f32> = or_.iter().zip(&r).map(|(o, x)| o - x).collect();
        let shared = delta_l
            .iter()
            .zip(&delta_r)
            .all(|(a, b)| (a - b).abs() <= 1e-6f32.max(a.abs() * 1e-6));
        assert!(shared, "one tape answers both lanes with the same wet");
        let echo_present = delta_l[n - 1].abs() > 1e-4;
        assert!(
            echo_present,
            "and the echo has arrived by the end: {}",
            delta_l[n - 1]
        );
    }

    #[test]
    fn the_high_pass_is_a_tilt_not_a_mute() {
        // The companion to the comment on `high_pole`: the 90 Hz high-pass,
        // fed a constant signal after a settled start (a constant is the
        // worst case for a high-pass), must keep passing mid-band content at
        // a sane fraction of unity — not at the low-pass coefficient's ~0.01.
        let a = high_pole(90.0, 48_000.0);
        assert!(a > 0.98 && a < 1.0, "90 Hz pole at 48 kHz: {a}");
        // A 1 kHz sine through the exact loop used in process_mono: measure
        // output peak over the settled half.
        let mut y = 0.0f32;
        let mut x_prev = 0.0f32;
        let mut peak = 0.0f32;
        for i in 0..48_000 {
            let x = (std::f32::consts::TAU * 1000.0 * i as f32 / 48_000.0).sin();
            y = a * (y + x - x_prev);
            x_prev = x;
            if i > 4_800 {
                peak = peak.max(y.abs());
            }
        }
        assert!(peak > 0.9, "1 kHz through a 90 Hz high-pass kept {peak}");
    }
}
