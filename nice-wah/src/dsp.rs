//! The filter, the follower, and the sweep between them.
//!
//! The signal path is a single resonant bandpass — the standing waveform a
//! wah is made of — whose corner frequency follows the treadle along a log
//! scale, exactly where the hardware put the ear's work: a third-order LC
//! circuit swept by a ganged capacitor geared to the pedal. This model keeps
//! the parts that made the famous voices famous and leaves the rest behind:
//! [`Voice`] records a sweep's two ends, its resonance, its taper and its
//! trim, and the three well-known ones ship as data, [`VOICES`] below —
//! presets in the honest sense, a handful of numbers the host can automate
//! as freely as it automates the treadle itself.
//!
//! An envelope follower may add to the pedal position: hit it hard and the
//! vowel moves up on its own, which is the auto-wah half of the plugin.
//! Everything here is fixed-size and allocation-free so the audio thread
//! stays clean under `nice-plug`'s `assert_process_allocs`.

/// The span the filter coefficients are recomputed over. A host block is
/// chopped into these, so the coefficient refresh rate — and with it the
/// resolution of both automation and the follower — never depends on how
/// large a block the host happens to hand us.
pub const BLOCK: usize = 64;

/// How fast the follower climbs towards a new peak, in seconds. Deliberately
/// faster than any musical attack the input can present: a wah played with
/// the fingers wants the filter to meet the note, not chase it.
const ATTACK: f32 = 0.002;

/// How fast the follower lets go, in seconds. Long enough that a sustained
/// note holds the filter open rather than flickering at its amplitude
/// variation, short enough that a gap in the playing closes the vowel.
const RELEASE: f32 = 0.30;

/// How fast the swept frequency follows its target, in seconds. The
/// follower output and the pedal both arrive smoothed through this, which
/// is what keeps a stepped automation lane or a yanked parameter from
/// zipper-noising the output; a twentieth of a second is below how fast a
/// foot moves and far above one audio block.
const SLEW: f32 = 0.020;

/// How fast the output trim follows its target, in seconds. Quicker than
/// the sweep: a gain that lags the dry signal by tens of milliseconds is
/// its own audible artifact.
const GAIN_SLEW: f32 = 0.010;

/// The quietest level the follower still reports, in decibels, as the bottom
/// of the scale. The mapping to 0..1 is in decibels so a whisper and a shout
/// get equal say, and anything at or below this floor reads zero.
const FLOOR_DB: f32 = -60.0;

/// One voice's fingerprint: everything but the treadle and the envelope that
/// says *which* wah this is. A plain value, published to the editor so a
/// preset button can write the parameters it stands for through the host's
/// own parameter API — automation records a voice change exactly like any
/// other move of the knobs it moves.
#[derive(Clone, Copy, Debug)]
pub struct Voice {
    pub name: &'static str,
    /// The corner frequency with the treadle at the heel.
    pub heel_hz: f32,
    /// ...and at the toe.
    pub toe_hz: f32,
    /// The height of the resonance, Q as an engineer writes it.
    pub q: f32,
    /// Where the treadle's resolution lives: −1 the toe crawls, +1 the
    /// heel does, 0 equal travel per octave. See [`sweep_hz`].
    pub taper: f32,
    /// The voice's own output trim: some circuits hit the amp harder than
    /// others, and the plugins that imitate them owe the level too.
    pub trim_db: f32,
}

/// The voices, in the order the editor will list them. The numbers are the
/// well-documented consensus of each circuit, not manufacturer figures —
/// none of the three ever printed a sweep table. The names are theirs, not
/// anybody else's: a voice is a handful of parameter values, and a set of
/// values earns no protection from the pedal it was measured off. The TOX
/// is the Shakespeare inductor's small, dark world, self-damped to a hill
/// of a resonance, with most of its travel below the first kilohertz. The
/// SKY BABY is the wide modern sweep, tall and even per octave, loud the
/// way the buffer makes it loud. The CLASH is the hot-wound custom under a
/// gain stage: raised off the mud, the toe's resolution parked on the
/// singing band, and a trim up for the amp in front of it.
pub const VOICES: [Voice; 3] = [
    Voice {
        name: "TOX '70",
        heel_hz: 350.0,
        toe_hz: 2_200.0,
        q: 2.2,
        taper: 0.6,
        trim_db: -4.0,
    },
    Voice {
        name: "SKY BABY",
        heel_hz: 90.0,
        toe_hz: 4_000.0,
        q: 4.0,
        taper: 0.0,
        trim_db: 0.0,
    },
    Voice {
        name: "CLASH",
        heel_hz: 350.0,
        toe_hz: 4_500.0,
        q: 3.5,
        taper: -0.4,
        trim_db: 3.0,
    },
];

/// The filter corner frequency at a treadle position. The exponential is
/// what makes the treadle feel linear — equal travel covers equal intervals,
/// not equal hertz — and the taper bends the exponent: `t^(2^taper)` walks
/// the position from linear, through equal octaves, to either end crawling.
/// A heel set above the toe is a silent guard rail: the sweep just stops
/// moving and parks at the heel.
pub fn sweep_hz(t: f32, heel_hz: f32, toe_hz: f32, taper: f32) -> f32 {
    let heel = heel_hz.clamp(10.0, 20_000.0);
    let toe = toe_hz.clamp(heel, 22_000.0);
    let t = t.clamp(0.0, 1.0);
    let warp = t.powf(2.0f32.powf(taper.clamp(-1.0, 1.0)));
    heel * (toe / heel).powf(warp)
}

/// One parameter set as the audio thread sees it: plain values, read fresh
/// from the [`nice_plug::prelude::Params`] every block. A struct, so the
/// filter asks for its whole control state at the top of each block and
/// nothing inside the sample loop touches an atomic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    /// The treadle, 0 heel .. 1 toe.
    pub position: f32,
    /// How far the envelope may push past the treadle, 0..1.
    pub env_amount: f32,
    /// The voice's sweep ends, resonance and taper — see [`Voice`].
    pub heel_hz: f32,
    pub toe_hz: f32,
    pub q: f32,
    pub taper: f32,
    /// The output trim in decibels.
    pub gain_db: f32,
}

impl Default for Controls {
    fn default() -> Self {
        let voice = VOICES[1]; // the archetype on the pedalboard
        Self {
            position: 0.5,
            env_amount: 0.5,
            heel_hz: voice.heel_hz,
            toe_hz: voice.toe_hz,
            q: voice.q,
            taper: voice.taper,
            gain_db: voice.trim_db,
        }
    }
}

impl From<Voice> for Controls {
    /// The controls a voice button writes: the voice everywhere it is
    /// allowed to go, the treadle and the envelope exactly as they stand.
    fn from(voice: Voice) -> Self {
        Self {
            heel_hz: voice.heel_hz,
            toe_hz: voice.toe_hz,
            q: voice.q,
            taper: voice.taper,
            gain_db: voice.trim_db,
            ..Self::default()
        }
    }
}

/// A biquad's numerator and denominator, with the denominator normalised so
/// `a0` is one and the per-sample work is four multiply-accumulates.
#[derive(Clone, Copy, Debug, Default)]
struct Coefs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

/// The RBJ bandpass of constant peak gain: the response stands at unity at
/// the corner whatever the resonance, so the sweep is a vowel travelling up
/// the spectrum and never a level bump. Derived from the Audio-EQ-Cookbook
/// table, written out in the normalised form.
fn bandpass(fc: f32, q: f32, rate: f32) -> Coefs {
    let w = (2.0 * std::f32::consts::PI * fc / rate).min(std::f32::consts::PI - 1e-4);
    let (sin_w, cos_w) = w.sin_cos();
    let alpha = sin_w / (2.0 * q.max(0.1));
    let a0 = 1.0 + alpha;
    Coefs {
        b0: alpha / a0,
        b1: 0.0,
        b2: -alpha / a0,
        a1: -2.0 * cos_w / a0,
        a2: (1.0 - alpha) / a0,
    }
}

/// The blend of two coefficient sets, the per-sample crossfade that keeps a
/// block-boundary recalculation from stepping the poles and printing a click
/// through the speaker.
fn blend(a: Coefs, b: Coefs, t: f32) -> Coefs {
    let at = 1.0 - t;
    Coefs {
        b0: a.b0 * at + b.b0 * t,
        b1: a.b1 * at + b.b1 * t,
        b2: a.b2 * at + b.b2 * t,
        a1: a.a1 * at + b.a1 * t,
        a2: a.a2 * at + b.a2 * t,
    }
}

/// One biquad's memory, direct form I: two inputs and two outputs back.
#[derive(Clone, Copy, Default)]
struct Biquad {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn process(&mut self, c: Coefs, x: f32) -> f32 {
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// The wah itself: one bandpass per channel, one shared follower, and the
/// smoothed state that carries between blocks.
pub struct Wah {
    rate: f32,
    /// The follower, linear amplitude, and the normalised reading of it the
    /// sweep and the editor both see.
    env: f32,
    env_norm: f32,
    /// The swept position as it stands, mid-slew toward this block's target.
    t: f32,
    /// The resonance, likewise smoothed: a jumped Q is a ring you hear.
    q: f32,
    /// The trim, in decibels, mid-slew.
    gain_db: f32,
    /// The last block's controls, kept for the one question `set_rate` has
    /// to answer without being handed the block again.
    held: Controls,
    /// The coefficients as the last block left them — the blend's start.
    previous: Coefs,
    /// The linear gain the last block ended on, blended to likewise.
    previous_gain: f32,
    filter: [Biquad; 2],
}

impl Wah {
    /// A filter ready for `rate`, parked at the controls it is handed so
    /// that activation — or a sample-rate change — begins silent and open
    /// where the pedal says, not with a slew from wherever the defaults sat.
    pub fn new(rate: f32, controls: &Controls) -> Self {
        let mut wah = Self {
            rate: rate.max(1.0),
            env: 0.0,
            env_norm: 0.0,
            t: controls.position.clamp(0.0, 1.0),
            q: controls.q,
            gain_db: controls.gain_db,
            held: *controls,
            previous: Coefs::default(),
            previous_gain: 1.0,
            filter: [Biquad::default(), Biquad::default()],
        };
        // Which coefficients, and at what gain, is `set_rate`'s question.
        wah.set_rate(rate);
        wah
    }

    /// Re-derive every sample-rate-dependent constant. The coefficients are
    /// recomputed from where the slew already stands, so a rate change that
    /// happens live crossfades through the blend like any other recalculation.
    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate.max(1.0);
        let controls = self.held;
        self.previous = bandpass(
            sweep_hz(self.t, controls.heel_hz, controls.toe_hz, controls.taper),
            self.q,
            self.rate,
        );
        self.previous_gain = db_to_gain(self.gain_db);
    }

    /// Silence's memory: the stored samples and the follower, but not the
    /// slew — a reset should still settle to the pedal rather than sweep
    /// to it from wherever the last take left the filter.
    pub fn reset(&mut self) {
        self.filter = [Biquad::default(), Biquad::default()];
        self.env = 0.0;
        self.env_norm = 0.0;
    }

    /// Where the follower stands, 0..1 — the editor's needle and the sweep's
    /// envelope half, the same number both.
    pub fn env_level(&self) -> f32 {
        self.env_norm
    }

    /// The sweep position the last block ran at, treadle plus envelope,
    /// before the taper bent it. What the editor draws on the pedal.
    pub fn effective_position(&self) -> f32 {
        self.t
    }

    /// The normalised reading of a follower level: decibels from the floor,
    /// so the treadle's travel matches what the ear hears the note doing.
    fn normalise(env: f32) -> f32 {
        if env <= 1e-9 {
            0.0
        } else {
            let db = 20.0 * env.log10();
            ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
        }
    }

    /// Run one block of one channel — up to [`BLOCK`] samples, in and out
    /// separate buffers; the caller copies through if the host asked for
    /// in-place. With a second channel, prefer [`Wah::process_stereo`], which
    /// lets both share one follower.
    pub fn process_mono(&mut self, controls: &Controls, input: &[f32], output: &mut [f32]) {
        let n = input.len().min(output.len()).min(BLOCK);
        let target = self.begin_block(controls, n);
        let previous = self.previous;
        let gain_target = db_to_gain(self.gain_db);
        let gain_from = self.previous_gain;
        for (i, out) in output[..n].iter_mut().enumerate() {
            self.env_follow(input[i].abs());
            let u = (i as f32 + 1.0) / n as f32;
            *out = self.filter[0].process(blend(previous, target, u), input[i])
                * (gain_from + (gain_target - gain_from) * u);
        }
        self.end_block(target, gain_target);
    }

    /// Run one block of two channels. The follower reads both, as one foot
    /// hears the whole guitar: a hard-strummed low string opens the vowel for
    /// the pair, not for its own channel alone.
    pub fn process_stereo(
        &mut self,
        controls: &Controls,
        inputs: [&[f32]; 2],
        outputs: [&mut [f32]; 2],
    ) {
        let n = inputs[0]
            .len()
            .min(inputs[1].len())
            .min(outputs[0].len())
            .min(outputs[1].len())
            .min(BLOCK);
        let target = self.begin_block(controls, n);
        let previous = self.previous;
        let gain_target = db_to_gain(self.gain_db);
        let gain_from = self.previous_gain;
        for i in 0..n {
            let left = inputs[0][i];
            let right = inputs[1][i];
            self.env_follow(left.abs().max(right.abs()));
            let u = (i as f32 + 1.0) / n as f32;
            let coefs = blend(previous, target, u);
            let gain = gain_from + (gain_target - gain_from) * u;
            outputs[0][i] = self.filter[0].process(coefs, left) * gain;
            outputs[1][i] = self.filter[1].process(coefs, right) * gain;
        }
        self.end_block(target, gain_target);
    }

    /// One sample of the follower: climb fast to a new peak, let go slow.
    fn env_follow(&mut self, magnitude: f32) {
        let coefficient = if magnitude > self.env {
            self.time_constant(ATTACK)
        } else {
            self.time_constant(RELEASE)
        };
        self.env += (magnitude - self.env) * coefficient;
        self.env_norm = Self::normalise(self.env);
    }

    /// The one-pole step for a time constant across `n` samples of a block:
    /// the follower and the slews advance by wall-clock, not by block count,
    /// so the feel is the same at 64-sample and 2048-sample blocks.
    fn time_constant(&self, seconds: f32) -> f32 {
        1.0 - (-1.0 / (self.rate.max(1.0) * seconds)).exp()
    }

    /// The block's shared advance — the slew targets, the held controls for
    /// the rate handler, and the coefficients the blend will arrive at. The
    /// follower reads ahead by a block rather than per sample; sixty-four
    /// samples at any rate a DAW runs is below the grain of what the effect
    /// is meant to sound like anyway.
    fn begin_block(&mut self, controls: &Controls, n: usize) -> Coefs {
        self.held = *controls;
        let n = (n.max(1)) as f32;
        let target = (controls.position + controls.env_amount * self.env_norm).clamp(0.0, 1.0);
        let slew = 1.0 - (-n / (self.rate.max(1.0) * SLEW)).exp();
        self.t += (target - self.t) * slew;
        self.q += (controls.q - self.q) * slew;
        let gain_slew = 1.0 - (-n / (self.rate.max(1.0) * GAIN_SLEW)).exp();
        self.gain_db += (controls.gain_db - self.gain_db) * gain_slew;
        bandpass(
            sweep_hz(self.t, controls.heel_hz, controls.toe_hz, controls.taper),
            self.q,
            self.rate,
        )
    }

    fn end_block(&mut self, target: Coefs, gain: f32) {
        self.previous = target;
        self.previous_gain = gain;
    }
}

/// Decibels to a linear multiplier, with the trim's own floor: the slew
/// passing through exact zero would invert phase for a sample on its way
/// down, which is a click with extra steps.
fn db_to_gain(db: f32) -> f32 {
    10.0f32.powf(db.max(-144.0) / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    /// The generic-sane control set the filter tests start from: the
    /// classic wide sweep, a tall enough resonance for the assertions to
    /// have room, and the envelope wired out unless a test says otherwise.
    fn classic() -> Controls {
        Controls {
            position: 0.5,
            env_amount: 0.0,
            heel_hz: 250.0,
            toe_hz: 3_500.0,
            q: 6.0,
            taper: 0.0,
            gain_db: 0.0,
        }
    }

    /// Run the filter over `seconds` of a signal, returning the whole output
    /// — the tests settle by discarding a prefix, the way a listener does.
    fn run(controls: &Controls, seconds: f32, mut signal: impl FnMut(usize) -> f32) -> Vec<f32> {
        let mut wah = Wah::new(RATE, controls);
        let total = (RATE * seconds) as usize;
        let mut out = vec![0.0; total];
        let mut block_in = [0.0; BLOCK];
        let mut block_out = [0.0; BLOCK];
        let mut start = 0;
        while start < total {
            let n = BLOCK.min(total - start);
            for (i, sample) in block_in[..n].iter_mut().enumerate() {
                *sample = signal(start + i);
            }
            wah.process_mono(controls, &block_in[..n], &mut block_out[..n]);
            out[start..start + n].copy_from_slice(&block_out[..n]);
            start += n;
        }
        out
    }

    /// The amplitude of `probe_hz` in a window of samples, one bin of a
    /// DFT. The whole test suite asks one question of the filter — how loud
    /// does a pure tone come out — and this is that question made exact.
    fn amplitude(x: &[f32], probe_hz: f32) -> f32 {
        let w = 2.0 * std::f32::consts::PI * probe_hz / RATE;
        let cw = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &sample in x {
            let s0 = sample + cw * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let n = x.len() as f32;
        // A window of a whole number of periods leaves the cross terms at
        // zero and the estimate exact; off-integer, the leakage is bounded
        // by the Dirichlet kernel, which at these probe-over-rate ratios is
        // far below every tolerance below.
        let real = s1 - s2 * w.cos();
        let imag = s2 * w.sin();
        2.0 * real.hypot(imag) / n
    }

    /// The steady gain at a frequency: a tone at full amplitude after the
    /// slews and the follower have settled, over a half-second window.
    fn gain_at(controls: &Controls, probe_hz: f32) -> f32 {
        let signal = run(controls, 1.5, |i| {
            (2.0 * std::f32::consts::PI * probe_hz * i as f32 / RATE).sin()
        });
        let tail = &signal[(RATE as usize / 2)..];
        amplitude(tail, probe_hz)
    }

    #[test]
    fn the_sweep_is_exponential_between_its_ends() {
        let (heel, toe) = (100.0, 6_400.0);
        assert!((sweep_hz(0.0, heel, toe, 0.0) - heel).abs() < 1e-3);
        assert!((sweep_hz(1.0, heel, toe, 0.0) - toe).abs() < 1e-2);
        let middle = sweep_hz(0.5, heel, toe, 0.0);
        // The geometric mean, because the scale is the ratio, not the step.
        assert!((middle - (heel * toe).sqrt()).abs() < 0.1);
        // Off the ends the treadle simply stops: automation beyond the
        // parameter range moves nothing.
        assert!((sweep_hz(-2.0, heel, toe, 0.0) - heel).abs() < 1e-3);
        assert!((sweep_hz(9.0, heel, toe, 0.0) - toe).abs() < 1e-2);
        // And the guard rail: a heel above its toe parks at the heel.
        assert!((sweep_hz(0.62, 2_000.0, 500.0, 0.0) - 2_000.0).abs() < 1e-3);
    }

    #[test]
    fn the_taper_moves_the_travel_from_one_end_of_the_pedal_to_the_other() {
        let (heel, toe) = (100.0, 6_400.0);
        let middle = sweep_hz(0.5, heel, toe, 0.0);
        // A heel-weighted taper is two-thirds down the octave scale at the
        // midpoint of the treadle; toe-weighted, two-thirds up it.
        let heel_weight = sweep_hz(0.5, heel, toe, 1.0);
        let toe_weight = sweep_hz(0.5, heel, toe, -1.0);
        assert!(heel_weight < middle * 0.6, "heel weight {}", heel_weight);
        assert!(toe_weight > middle * 1.6, "toe weight {}", toe_weight);
        // The ends are the ends whatever the taper: the hardware never lost
        // its stop at either side, and neither does this.
        assert!((sweep_hz(0.0, heel, toe, -1.0) - heel).abs() < 1e-3);
        assert!((sweep_hz(1.0, heel, toe, 1.0) - toe).abs() < 1e-2);
        // And the taper is a bias, not a shuffle: each taper stays strictly
        // up the scale as the treadle travels.
        for &taper in &[-1.0, -0.4, 0.0, 0.6, 1.0] {
            let mut previous = sweep_hz(0.0, heel, toe, taper);
            for i in 1..=32 {
                let t = i as f32 / 32.0;
                let next = sweep_hz(t, heel, toe, taper);
                assert!(next > previous, "t {t} taper {taper} went backwards");
                previous = next;
            }
        }
    }

    #[test]
    fn the_poles_stay_inside_the_circle_across_the_whole_sweep() {
        // The RBJ bandpass is unconditionally stable, but the coefficients
        // survive our blending and floating point too — so: for every
        // position, voice and Q the parameter offers, the denominator keeps
        // its roots in the unit circle (Jury's conditions, degree two).
        for &t in &[0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0] {
            for voice in VOICES {
                for &q in &[0.7, 1.0, 2.0, 6.0, 12.0] {
                    let fc = sweep_hz(t, voice.heel_hz, voice.toe_hz, voice.taper);
                    let c = bandpass(fc, q, RATE);
                    assert!(
                        c.a2.abs() < 1.0,
                        "voice {} t {t} q {q}: a2 {}",
                        voice.name,
                        c.a2
                    );
                    assert!(
                        c.a1.abs() < 1.0 + c.a2,
                        "voice {} t {t} q {q}: a1 {}",
                        voice.name,
                        c.a1
                    );
                }
            }
        }
    }

    #[test]
    fn the_filter_passes_its_corner_and_nothing_like_it_elsewhere() {
        let controls = classic();
        let corner = sweep_hz(0.5, controls.heel_hz, controls.toe_hz, controls.taper);
        let at_corner = gain_at(&controls, corner);
        assert!((at_corner - 1.0).abs() < 0.02, "peak gain {}", at_corner);
        // An octave down the skirt is falling; six Q is a wall, not a hill.
        let below = gain_at(&controls, corner / 2.0);
        let above = gain_at(&controls, corner * 2.0);
        assert!(below < 0.6, "octave below: {}", below);
        assert!(above < 0.6, "octave above: {}", above);
    }

    #[test]
    fn the_treadle_walks_the_passband_up_the_spectrum() {
        // The sweep's whole idea in one assertion pair: a tone the heel
        // keeps out, the toe lets through — and one the other way round.
        // Six Q, so a Q-wide miss of the passband is worth tens of
        // decibels and the margins below are never in doubt.
        let low = Controls {
            position: 0.0,
            ..classic()
        };
        let high = Controls {
            position: 1.0,
            ..classic()
        };
        let heel_3000 = gain_at(&low, 3_000.0);
        let toe_3000 = gain_at(&high, 3_000.0);
        assert!(heel_3000 < 0.03, "heel at 3 kHz: {}", heel_3000);
        assert!(toe_3000 > 0.4, "toe at 3 kHz: {}", toe_3000);
        let heel_400 = gain_at(&low, 400.0);
        let toe_400 = gain_at(&high, 400.0);
        assert!(heel_400 > 0.1, "heel at 400 Hz: {}", heel_400);
        assert!(
            heel_400 > toe_400 * 5.0,
            "heel {} toe {}",
            heel_400,
            toe_400
        );
    }

    #[test]
    fn playing_harder_opens_the_vowel_when_the_envelope_is_asked_to() {
        // The same tone, the same pedal at the heel, two decades of level
        // apart. With the follower wired out, level changes the sound not
        // at all; wired in, the loud one drags the corner frequency up to
        // the probe and the quiet one leaves it down at the heel.
        let quiet = 0.01f32; // −40 dB: a third of the scale, and far below loud.
        let loud = 0.9f32;
        let probe = 3_000.0;
        // Two Q: wide enough that the loud note's corner, one-third of an
        // octave above the probe, still reads near unity there.
        let off = Controls {
            q: 2.0,
            ..classic()
        };
        let on = Controls {
            position: 0.0,
            env_amount: 1.0,
            ..off
        };
        let measure = |controls: &Controls, level: f32| {
            let out = run(controls, 1.5, |i| {
                level * (2.0 * std::f32::consts::PI * probe * i as f32 / RATE).sin()
            });
            amplitude(&out[(RATE as usize / 2)..], probe) / level
        };
        // Without the follower the filter never hears how loud the note is:
        // both levels ride it with the same gain.
        let (quiet_off, loud_off) = (measure(&off, quiet), measure(&off, loud));
        assert!(
            (quiet_off - loud_off).abs() < 0.02,
            "linear without the follower"
        );
        // With it, the loud note has opened the filter onto the probe and
        // the quiet one — a third of the scale up at −40 dB — has not.
        let (quiet_on, loud_on) = (measure(&on, quiet), measure(&on, loud));
        assert!(loud_on > 0.7, "loud at the probe: {}", loud_on);
        assert!(
            loud_on > quiet_on + 0.3,
            "quiet {} loud {}",
            quiet_on,
            loud_on
        );
    }

    #[test]
    fn the_trim_trims() {
        let flat = Controls {
            q: 2.0,
            ..classic()
        };
        let lifted = Controls {
            gain_db: 6.0,
            ..flat
        };
        let corner = sweep_hz(0.5, flat.heel_hz, flat.toe_hz, flat.taper);
        let ratio = gain_at(&lifted, corner) / gain_at(&flat, corner);
        assert!((ratio - 2.0).abs() < 0.05, "+6 dB is a factor of {}", ratio);
    }

    #[test]
    fn silence_is_the_one_signal_it_cannot_colour() {
        let controls = Controls::default();
        let out = run(&controls, 0.5, |_| 0.0);
        assert!(out.iter().all(|sample| *sample == 0.0));
        assert_eq!(Wah::new(RATE, &controls).env_level(), 0.0);
    }

    #[test]
    fn a_quiet_input_never_moves_the_filter() {
        // The follower floor at work: −70 dB of nothing with the envelope
        // wired all the way up should still sit at the heel.
        let controls = Controls {
            position: 0.0,
            env_amount: 1.0,
            ..classic()
        };
        let level = 0.0003f32; // −70 dB: under the scale's floor entirely.
        let out = run(&controls, 1.0, |i| {
            level * (2.0 * std::f32::consts::PI * 3_000.0 * i as f32 / RATE).sin()
        });
        let tail = &out[(RATE as usize / 2)..];
        let gain = amplitude(tail, 3_000.0) / level;
        assert!(gain < 0.05, "floor lifted the heel: gain {}", gain);
    }

    #[test]
    fn every_shipped_voice_is_a_filter_and_not_a_fault() {
        // The presets are hand-tuned numbers, so each gets the two checks
        // the manual makes by hand: the corner passes at unity at both ends
        // of its own sweep, and its resonance never runs away from the range
        // the Q parameter promises.
        for voice in VOICES {
            for position in [0.0, 1.0] {
                let controls = Controls {
                    position,
                    env_amount: 0.0,
                    heel_hz: voice.heel_hz,
                    toe_hz: voice.toe_hz,
                    q: voice.q,
                    taper: voice.taper,
                    gain_db: 0.0,
                };
                let corner = sweep_hz(position, voice.heel_hz, voice.toe_hz, voice.taper);
                let gain = gain_at(&controls, corner);
                assert!(
                    (gain - 1.0).abs() < 0.02,
                    "{} at {}: gain {}",
                    voice.name,
                    position,
                    gain
                );
            }
            assert!(
                (0.7..=12.0).contains(&voice.q),
                "{} q out of range",
                voice.name
            );
        }
    }
}
