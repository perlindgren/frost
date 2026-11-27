//! Nice Eq's signal path: seven fixed-frequency sections in series and an
//! output trim, all of it derived once per block and crossfaded per sample.
//!
//! The shape of the machine is worth stating before the code, because it is
//! the same shape every graphic equaliser has and the reason for each piece
//! is a listening consequence:
//!
//! - **Fixed centres, moving gain.** A fader changes one section's depth,
//!   never its frequency. That is what makes a graphic equaliser graphic: the
//!   response you see is the slider bank seen from the side, so the sliders
//!   and the response have to share one axis.
//! - **A cascade, not a sum.** Sections in series multiply, which is why two
//!   neighbours both pushed read as one wide lift rather than as two hills.
//!   The cost of the cascade is that the *shape* of one band's skirt decides
//!   how its neighbours look, which is what [`Band`]'s `q` is for.
//! - **Six bells and one shelf.** Everything above the top band is program
//!   material with no slider of its own, so the seventh section shelves
//!   instead of ringing back down to unity. Everything below 100 Hz is
//!   rumble a player usually wants left alone, so the first section is a bell
//!   with a skirt on both sides. See `DECISIONS.md`.
//!
//! Nothing here knows about hosts, parameters or pixels. This file is the one
//! the isolated allocation witness includes by path, so it imports nothing
//! from the crate either.

use std::f32::consts::PI;

/// Samples per call. A host's block is chunked to this; the crossfade below
/// runs over one chunk, which is why a stepped automation lane cannot click.
pub const BLOCK: usize = 64;

/// Every mode in this crate has this many bands, and the fader bank, the
/// parameter list and the cascade all take their length from here.
pub const BANDS: usize = 7;

/// The travel of every band fader and of the trim, in decibels, both ways.
/// One constant because the sliders are one bank: a mode whose faders moved
/// different amounts would not read as a set.
pub const MAX_DB: f32 = 15.0;

/// The width of a bell, chosen in `DECISIONS.md` so that octave-spaced
/// neighbours overlap instead of leaving teeth between them.
pub const BELL_Q: f32 = 1.4;

/// The shelving section's Q. `1/√2` is the maximally-flat member of the
/// shelf family; a wider one overshoots just below the corner and prints a
/// hump nobody asked for.
pub const SHELF_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// What a band section is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A peaking bell: unity far from the centre, the full depth at it.
    Bell,
    /// A high shelf: unity well below the corner, the full depth above it.
    HighShelf,
}

/// One band as a mode names it: where, how wide, what shape.
#[derive(Clone, Copy, Debug)]
pub struct Band {
    pub hz: f32,
    pub q: f32,
    pub shape: Shape,
}

const fn bell(hz: f32) -> Band {
    Band {
        hz,
        q: BELL_Q,
        shape: Shape::Bell,
    }
}

/// The octave ladder, lowest first — the row the eye reads left to right.
///
/// The centres are the 1987 GE-7's: 100 Hz to 6.4 kHz by octaves, the seven
/// linear-taper potentiometers of the redrawn service schematic named for
/// exactly these frequencies. ±15 dB of travel per band and ±15 dB of output
/// trim are the unit's published figures. The top band is the shelf, and the
/// reason is in the module header and in `DECISIONS.md`; it is a shape choice
/// of ours, not something the drawing states.
pub const G7_BANDS: [Band; BANDS] = [
    bell(100.0),
    bell(200.0),
    bell(400.0),
    bell(800.0),
    bell(1_600.0),
    bell(3_200.0),
    Band {
        hz: 6_400.0,
        q: SHELF_Q,
        shape: Shape::HighShelf,
    },
];

/// What the faders mean. A mode is a band table and nothing else, which is
/// what lets one parameter set serve every mode in the crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// [`G7_BANDS`]: the seven-band octave graphic, the GE-7's grid.
    G7,
}

impl Mode {
    /// The mode a host's integer names. There is one position today, so every
    /// integer means it — the arm is written this way rather than as a match
    /// so that adding a mode is the only edit the function needs, and so a
    /// session holding an integer from a future version still reads as
    /// something rather than nothing.
    pub fn from_param(_value: i32) -> Mode {
        Mode::G7
    }

    pub const fn to_param(self) -> i32 {
        match self {
            Mode::G7 => 0,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Mode::G7 => "G7",
        }
    }

    pub const fn bands(self) -> &'static [Band; BANDS] {
        match self {
            Mode::G7 => &G7_BANDS,
        }
    }
}

/// What the analyser under the sliders is asked to do. It lives here because
/// the audio thread reads it: `Off` means the sample feed that the transform
/// downstream would consume is not written at all, which is a statement about
/// this thread's work, not about drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// No feed, no transform, no picture.
    Off,
    /// The selected transform length only.
    Top,
    /// Every length, overlaid: short windows for the fast picture, long ones
    /// for the resolved one.
    All,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::Off, Scope::Top, Scope::All];

    /// Out of range reads as `Top`: the position the panel opens on, and the
    /// one that shows something. `Off` is reached by asking for it.
    pub fn from_param(value: i32) -> Scope {
        match value {
            0 => Scope::Off,
            2 => Scope::All,
            _ => Scope::Top,
        }
    }

    pub const fn to_param(self) -> i32 {
        match self {
            Scope::Off => 0,
            Scope::Top => 1,
            Scope::All => 2,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Scope::Off => "OFF",
            Scope::Top => "TOP",
            Scope::All => "ALL",
        }
    }
}

/// Which end of the box the analyser is pointed at. Like [`Scope`] it lives
/// here because the audio thread acts on it: the feed is written at the tap
/// that was asked for and not at the other one, so `In` costs one ring copy
/// per block and `Both` costs two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tap {
    /// Before the cascade: what came in.
    In,
    /// After the cascade and the trim: what went out.
    Out,
    /// Both, in two colours, so the picture is the difference.
    Both,
}

impl Tap {
    pub const ALL: [Tap; 3] = [Tap::In, Tap::Out, Tap::Both];

    /// Out of range reads as `Both`. Showing both halves is the one answer
    /// that cannot mislead about which signal it is: the labels say so.
    pub fn from_param(value: i32) -> Tap {
        match value {
            0 => Tap::In,
            1 => Tap::Out,
            _ => Tap::Both,
        }
    }

    pub const fn to_param(self) -> i32 {
        match self {
            Tap::In => 0,
            Tap::Out => 1,
            Tap::Both => 2,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Tap::In => "IN",
            Tap::Out => "OUT",
            Tap::Both => "BOTH",
        }
    }

    /// Whether the audio thread copies the dry lane into the analyser feed.
    pub const fn wants_input(self) -> bool {
        !matches!(self, Tap::Out)
    }

    /// Whether it copies the processed lane.
    pub const fn wants_output(self) -> bool {
        !matches!(self, Tap::In)
    }
}

/// One block's control state, taken whole. The sample loop touches no
/// parameter and no atomic because it has this instead.
#[derive(Clone, Copy, Debug)]
pub struct Controls {
    /// The mode selector's integer, mapped by [`Mode::from_param`].
    pub mode: i32,
    /// One entry per band, decibels, −[`MAX_DB`]…+[`MAX_DB`].
    pub bands: [f32; BANDS],
    /// The trim, after the cascade.
    pub gain_db: f32,
}

impl Default for Controls {
    /// Flat: every band at unity, the trim at unity, the first mode. This is
    /// the state a fresh instance opens in, and the state in which the
    /// cascade is required to change nothing.
    fn default() -> Self {
        Self {
            mode: 0,
            bands: [0.0; BANDS],
            gain_db: 0.0,
        }
    }
}

/// A biquad's numerator and denominator, denominator normalised so `a0` is
/// one and the per-sample work is four multiply-accumulates.
#[derive(Clone, Copy, Debug, Default)]
struct Coefs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

/// The blend of two coefficient sets: the per-sample crossfade that keeps a
/// block-boundary recalculation from stepping the poles and printing a click
/// through the speaker. Written as `a + (b − a)·t` rather than the weighted
/// pair so that blending a set with itself returns it bit for bit — the flat
/// cascade is then the identity by arithmetic, not by tolerance, and one
/// subtraction costs less than two multiplies.
fn blend(a: Coefs, b: Coefs, t: f32) -> Coefs {
    let d = |x: f32, y: f32| x + (y - x) * t;
    Coefs {
        b0: d(a.b0, b.b0),
        b1: d(a.b1, b.b1),
        b2: d(a.b2, b.b2),
        a1: d(a.a1, b.a1),
        a2: d(a.a2, b.a2),
    }
}

/// The trim's law, including its ends: past ±[`MAX_DB`] the fader is at the
/// end of its travel, wherever a host's automation number thinks it is.
fn trim_gain(db: f32) -> f32 {
    db_to_gain(db.clamp(-MAX_DB, MAX_DB))
}

/// Decibels to a linear multiplier, floored where a fader's own floor is
/// below audibility: the slew passing through exact zero would invert phase
/// for a sample on its way down, which is a click with extra steps.
fn db_to_gain(db: f32) -> f32 {
    10.0f32.powf(db.max(-144.0) / 20.0)
}

/// Decibels to the square root of a gain ratio, which is what the cookbook's
/// peaking and shelving formulae want: their `A` squares to the passband
/// ratio, so +15 dB arrives as `10^(15/40)` and behaves as 5.62 at the top.
fn db_to_amplitude_sqrt(db: f32) -> f32 {
    10.0f32.powf(db.clamp(-MAX_DB, MAX_DB).max(-144.0) / 40.0)
}

/// The RBJ peaking section of nominal centre `hz`, written out in normalised
/// form. At 0 dB the numerator and the denominator come out equal term by
/// term, which is why a flat band is transparent by identity rather than by
/// tolerance — the test that says so is in this file.
fn bell_coefs(hz: f32, q: f32, boost_db: f32, rate: f32) -> Coefs {
    let w = corner(hz, rate);
    let (sin_w, cos_w) = w.sin_cos();
    let alpha = sin_w / (2.0 * q.max(0.1));
    let a = db_to_amplitude_sqrt(boost_db);
    let a0 = 1.0 + alpha / a;
    Coefs {
        b0: (1.0 + alpha * a) / a0,
        b1: -2.0 * cos_w / a0,
        b2: (1.0 - alpha * a) / a0,
        a1: -2.0 * cos_w / a0,
        a2: (1.0 - alpha / a) / a0,
    }
}

/// The RBJ high shelf with corner `hz`: unity below, the full depth above,
/// and no return to unity at the top of the band. The shelf's Q is its width
/// parameter in this form, and [`SHELF_Q`] is the flat member of the family.
fn shelf_coefs(hz: f32, q: f32, boost_db: f32, rate: f32) -> Coefs {
    let w = corner(hz, rate);
    let (sin_w, cos_w) = w.sin_cos();
    let alpha = sin_w / (2.0 * q.max(0.1));
    let a = db_to_amplitude_sqrt(boost_db);
    let sqrt_a = a.sqrt();
    let two_sqrt_a_alpha = 2.0 * sqrt_a * alpha;
    let a0 = (a + 1.0) - (a - 1.0) * cos_w + two_sqrt_a_alpha;
    Coefs {
        b0: a * ((a + 1.0) + (a - 1.0) * cos_w + two_sqrt_a_alpha) / a0,
        b1: -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w) / a0,
        b2: a * ((a + 1.0) + (a - 1.0) * cos_w - two_sqrt_a_alpha) / a0,
        a1: 2.0 * ((a - 1.0) - (a + 1.0) * cos_w) / a0,
        a2: ((a + 1.0) - (a - 1.0) * cos_w - two_sqrt_a_alpha) / a0,
    }
}

/// The corner in radians, with the design frequency held to 45 % of the
/// sample rate. At an 8 kHz capture rate the 6.4 kHz shelf has nothing above
/// it to shelf, and designing a section at Nyquist puts its poles on the unit
/// circle at −1: a ring that never dies. Folding the corner down is the
/// honest reading of what the fader means at that rate — the top of the band
/// is 3.6 kHz there, not 6.4 — and it keeps every section conditioned, with
/// a sine of 0.31 in its α instead of 1e-4.
fn corner(hz: f32, rate: f32) -> f32 {
    let rate = rate.max(1.0);
    let hz = hz.max(0.0).min(0.45 * rate);
    2.0 * PI * hz / rate
}

/// Design every section for one control state.
fn target_coefs(controls: &Controls, rate: f32) -> [Coefs; BANDS] {
    let bands = Mode::from_param(controls.mode).bands();
    let mut out = [Coefs::default(); BANDS];
    for (i, out) in out.iter_mut().enumerate() {
        let band = bands[i];
        let db = controls.bands[i].clamp(-MAX_DB, MAX_DB);
        *out = match band.shape {
            Shape::Bell => bell_coefs(band.hz, band.q, db, rate),
            Shape::HighShelf => shelf_coefs(band.hz, band.q, db, rate),
        };
    }
    out
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

/// The equaliser: seven sections per lane, one trim, one rate, and the two
/// states a crossfade needs.
pub struct Eq {
    rate: f32,
    /// The sections as the last block ran them — the near end of the blend.
    previous: [Coefs; BANDS],
    previous_gain: f32,
    /// Per lane, because two lanes are the same equaliser with separate
    /// memory: summing them would be the plugin deciding a host's mix.
    state: [[Biquad; BANDS]; 2],
    /// The last controls seen, so a rate change retunes from where the
    /// faders actually stand rather than from a stale set.
    held: Controls,
}

impl Eq {
    pub fn new(rate: f32, controls: &Controls) -> Self {
        let rate = rate.max(1.0);
        Self {
            rate,
            previous: target_coefs(controls, rate),
            previous_gain: trim_gain(controls.gain_db),
            state: [[Biquad::default(); BANDS]; 2],
            held: *controls,
        }
    }

    /// Re-derive every sample-rate-dependent coefficient. Recomputed from the
    /// controls rather than blended from the old rate's poles, because a rate
    /// change is a lifecycle moment and the old poles describe a filter
    /// nobody asked for any more.
    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate.max(1.0);
        let held = self.held;
        self.previous = target_coefs(&held, self.rate);
        self.previous_gain = trim_gain(held.gain_db);
    }

    /// Silence the filters' memory. Deliberately not the blend state: a reset
    /// should settle to where the faders are, not sweep from wherever the
    /// last block happened to be on its way.
    pub fn reset(&mut self) {
        self.state = [[Biquad::default(); BANDS]; 2];
    }

    /// Run one block of up to [`BLOCK`] samples on `lanes` lanes. The caller
    /// copies through if the host asked for in-place buffers.
    pub fn process(
        &mut self,
        controls: &Controls,
        lanes: usize,
        input: &[[f32; BLOCK]; 2],
        output: &mut [[f32; BLOCK]; 2],
    ) {
        let n = input[0].len().min(output[0].len()).min(BLOCK);
        let lanes = lanes.clamp(1, 2);
        let target = target_coefs(controls, self.rate);
        let gain_from = self.previous_gain;
        let gain_target = trim_gain(controls.gain_db);
        for i in 0..n {
            let u = (i as f32 + 1.0) / n as f32;
            let c = blend_set(self.previous, target, u);
            let gain = gain_from + (gain_target - gain_from) * u;
            for lane in 0..lanes {
                let mut sample = input[lane][i];
                for (band, state) in self.state[lane].iter_mut().enumerate() {
                    sample = state.process(c[band], sample);
                }
                output[lane][i] = sample * gain;
            }
        }
        self.previous = target;
        self.previous_gain = gain_target;
        self.held = *controls;
    }
}

/// The crossfade, all seven sections at once: one `u` for the whole cascade,
/// so the bands move together rather than each on its own schedule.
fn blend_set(a: [Coefs; BANDS], b: [Coefs; BANDS], t: f32) -> [Coefs; BANDS] {
    let mut out = [Coefs::default(); BANDS];
    for i in 0..BANDS {
        out[i] = blend(a[i], b[i], t);
    }
    out
}

/// Whether a section's poles are inside the unit circle: the Jury conditions
/// on the normalised denominator, which is the cheap and complete answer to
/// "can this filter ring forever".
#[cfg(test)]
fn stable(c: Coefs) -> bool {
    c.a2.abs() < 1.0 && c.a1.abs() < 1.0 + c.a2
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    /// One band pushed to `db`, everything else flat.
    fn one_band(band: usize, db: f32) -> Controls {
        let mut controls = Controls::default();
        controls.bands[band] = db;
        controls
    }

    /// Run one already-built equaliser in real blocks, on both lanes, and
    /// hand back the left channel. The index handed to `signal` is the honest
    /// one — the position in the stream — so a test's sine stays a sine.
    fn run_through(
        eq: &mut Eq,
        rate: f32,
        seconds: f32,
        controls: &Controls,
        mut signal: impl FnMut(usize) -> f32,
    ) -> Vec<f32> {
        let total = (seconds * rate) as usize;
        let mut input = [[0.0f32; BLOCK]; 2];
        let mut output = [[0.0f32; BLOCK]; 2];
        let mut out: Vec<f32> = Vec::with_capacity(total);
        while out.len() < total {
            let n = BLOCK.min(total - out.len());
            // The whole buffer is fed even on a short tail: `process` takes
            // the array it is handed, and a test's tone is better continued
            // than interrupted by zeros at the end of the run. One generator
            // drives both lanes, which is the point: the lanes must be able
            // to carry different signals elsewhere without being summed.
            let (left, right) = input.split_at_mut(1);
            for (i, sample) in left[0].iter_mut().enumerate() {
                let v = signal(out.len() + i);
                *sample = v;
                right[0][i] = v;
            }
            eq.process(controls, 2, &input, &mut output);
            out.extend_from_slice(&output[0][..n]);
        }
        out
    }

    /// Run the real `process` in real blocks at the house rate, on both
    /// lanes, and hand back the left channel.
    fn run(controls: &Controls, seconds: f32, signal: impl FnMut(usize) -> f32) -> Vec<f32> {
        let mut eq = Eq::new(RATE, controls);
        run_through(&mut eq, RATE, seconds, controls, signal)
    }

    /// The amplitude of `probe_hz` in a window of samples, one bin of a DFT.
    /// The whole suite asks one question of the equaliser — how loud does a
    /// pure tone come out — and this is that question made exact.
    fn amplitude_at(x: &[f32], probe_hz: f32, rate: f32) -> f32 {
        let w = 2.0 * PI * probe_hz / rate;
        let cw = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for &sample in x {
            let s0 = sample + cw * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let n = x.len() as f32;
        let real = s1 - s2 * w.cos();
        let imag = s2 * w.sin();
        2.0 * real.hypot(imag) / n
    }

    /// The steady gain at a frequency: a tone at full amplitude once every
    /// slew has settled, measured over the settled half second.
    fn gain_on(controls: &Controls, rate: f32, probe_hz: f32) -> f32 {
        let mut eq = Eq::new(rate, controls);
        let signal = run_through(&mut eq, rate, 1.5, controls, |i| {
            (2.0 * PI * probe_hz * i as f32 / rate).sin()
        });
        let tail = &signal[(rate as usize / 2)..];
        amplitude_at(tail, probe_hz, rate)
    }

    fn gain_at(controls: &Controls, probe_hz: f32) -> f32 {
        gain_on(controls, RATE, probe_hz)
    }

    fn db(gain: f32) -> f32 {
        20.0 * gain.max(1e-9).log10()
    }

    #[test]
    fn the_ladder_is_the_octave_grid_it_claims_to_be() {
        let bands = Mode::G7.bands();
        let hz: Vec<f32> = bands.iter().map(|b| b.hz).collect();
        assert_eq!(hz, [100.0, 200.0, 400.0, 800.0, 1_600.0, 3_200.0, 6_400.0]);
        // Six bells and a shelf, the bell at the bottom of the ladder and the
        // shelf at the top of it: the two ends are different because what
        // lives beyond them is different.
        assert_eq!(bands[0].shape, Shape::Bell);
        assert_eq!(bands[BANDS - 2].shape, Shape::Bell);
        assert_eq!(bands[BANDS - 1].shape, Shape::HighShelf);
    }

    #[test]
    fn a_band_table_outside_the_modes_clamps_rather_than_invents_one() {
        // There is one mode today, so any integer means it; a session holding
        // a position from a newer build must still open as something.
        assert_eq!(Mode::from_param(-7), Mode::G7);
        assert_eq!(Mode::from_param(9), Mode::G7);
        assert_eq!(Mode::G7.to_param(), 0);
        assert_eq!(Mode::from_param(Mode::G7.to_param()).bands().len(), BANDS);
        assert_eq!(Mode::G7.label(), "G7");
        assert_eq!(Scope::from_param(-1), Scope::Top);
        assert_eq!(Scope::from_param(99), Scope::Top);
        for (index, scope) in Scope::ALL.iter().enumerate() {
            assert_eq!(Scope::from_param(index as i32), *scope);
        }
    }

    #[test]
    #[ignore = "a printed report, not an assertion: run it with -- --ignored --nocapture"]
    fn the_probe_prints_the_curves_it_was_asked_about() {
        // Not an assertion: the numbers the thresholds in this file are set
        // from, kept so the next person can see what the ladder actually does
        // instead of trusting a comment. Run with -- --nocapture.
        println!("one band pushed to +15 dB, read at each label:");
        for index in 0..BANDS {
            let controls = one_band(index, MAX_DB);
            let row: Vec<String> = Mode::G7
                .bands()
                .iter()
                .map(|probe| format!("{:.2}", db(gain_at(&controls, probe.hz))))
                .collect();
            println!(
                "  {:>6} Hz {:?}: {}  off-label: {}",
                Mode::G7.bands()[index].hz,
                Mode::G7.bands()[index].shape,
                row.join(" "),
                [50.0, 400.0, 16_000.0, 20_000.0]
                    .iter()
                    .map(|hz| format!("{hz:.0}={:.2}", db(gain_at(&controls, *hz))))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
        println!("one band cut to -15 dB, read at each label:");
        for index in 0..BANDS {
            let controls = one_band(index, -MAX_DB);
            let row: Vec<String> = Mode::G7
                .bands()
                .iter()
                .map(|probe| format!("{:.2}", db(gain_at(&controls, probe.hz))))
                .collect();
            println!("  {:>6} Hz: {}", Mode::G7.bands()[index].hz, row.join(" "));
        }
        let all = Controls {
            bands: [MAX_DB; BANDS],
            ..Default::default()
        };
        let sweep: Vec<String> = (0..=84)
            .map(|t| {
                let hz = 100.0 * 2.0f32.powf(t as f32 / 12.0);
                format!("{:.1}Hz={:.2}", hz, db(gain_at(&all, hz)))
            })
            .collect();
        println!(
            "full boost everywhere, twelfth by twelfth:\n  {}",
            sweep.join(" ")
        );
        let mut eq = Eq::new(RATE, &Controls::default());
        println!(
            "shelf corners at the rates the settings offer: {}",
            [8_000.0, 44_100.0, 192_000.0]
                .iter()
                .map(|r| format!(
                    "{r:.0}->{:.1} Hz",
                    corner(Mode::G7.bands()[BANDS - 1].hz, *r) / (2.0 * PI) * r
                ))
                .collect::<Vec<_>>()
                .join(" ")
        );
        eq.process(
            &Controls::default(),
            1,
            &[[0.0; BLOCK]; 2],
            &mut [[0.0; BLOCK]; 2],
        );
    }

    #[test]
    fn flat_is_transparent_because_the_two_polynomials_are_the_same_one() {
        // The strong claim, on the coefficients: at 0 dB a peaking section's
        // numerator equals its denominator term by term, and so does a
        // shelf's. That is arithmetic, so it is checked exactly.
        for rate in [8_000.0, 44_100.0, 96_000.0, 192_000.0] {
            for c in target_coefs(&Controls::default(), rate) {
                assert_eq!(c.b0, 1.0, "flat section at {rate} Hz has gain");
                assert_eq!(c.b1, c.a1, "flat section at {rate} Hz has b1");
                assert_eq!(c.b2, c.a2, "flat section at {rate} Hz has b2");
            }
        }
        // The consequence, in samples: an identity cascade still accumulates
        // rounding in its own state, because seven direct-form I sections run
        // for sixty thousand samples. What is being asserted is that the
        // departure is rounding and not response — eighty decibels down, on a
        // 100 Hz section whose poles sit within a hundredth of the unit
        // circle, which is where this drift is worst.
        let controls = Controls::default();
        let out = run(&controls, 1.0, |i| {
            0.4 * (2.0 * PI * 997.0 * i as f32 / RATE).sin()
                + 0.2 * (2.0 * PI * 51.0 * i as f32 / RATE).sin()
        });
        let mut worst = 0.0f32;
        for (i, sample) in out.iter().enumerate().skip(RATE as usize / 4) {
            let wanted = 0.4 * (2.0 * PI * 997.0 * i as f32 / RATE).sin()
                + 0.2 * (2.0 * PI * 51.0 * i as f32 / RATE).sin();
            worst = worst.max((sample - wanted).abs());
        }
        assert!(worst < 1e-4, "flat cascade departs by {worst}");
    }

    #[test]
    fn a_bell_holds_its_centre_and_lets_go_two_octaves_away() {
        let boost = 10f32.powf(15.0 / 20.0);
        // Band 0, 100 Hz: the full +15 dB at the label, and down to under a
        // decibel by 400 Hz. "Under a decibel" is the measured skirt of a
        // Q of 1.4 — the ladder's neighbours are meant to overlap, and the
        // residue here is that overlap seen two octaves out, not a filter
        // that has not finished falling.
        let at = gain_at(&one_band(0, 15.0), 100.0);
        let away = gain_at(&one_band(0, 15.0), 400.0);
        assert!(
            (db(at) - 15.0).abs() < 0.3,
            "100 Hz band at its own centre reads {} dB",
            db(at)
        );
        assert!(db(away).abs() < 1.0, "two octaves up reads {} dB", db(away));
        // And a cut is the mirror of the boost, not a different filter.
        let cut = gain_at(&one_band(0, -15.0), 100.0);
        assert!((db(cut) + 15.0).abs() < 0.3, "the cut reads {} dB", db(cut));
        assert!((boost - at).abs() < at * 0.02);
    }

    #[test]
    fn the_lowest_band_is_a_hill_and_not_a_shelf() {
        // A 100 Hz shelf would keep its boost all the way down into the
        // rumble. This is a bell: an octave below the centre it is already
        // on its way back to unity, and two octaves below it is there.
        let boost = gain_at(&one_band(0, 15.0), 100.0);
        let one_octave_down = gain_at(&one_band(0, 15.0), 50.0);
        let two_down = gain_at(&one_band(0, 15.0), 25.0);
        assert!(
            db(one_octave_down) < db(boost) - 3.0,
            "half the ladder is still up an octave down: {} dB against {} dB",
            db(one_octave_down),
            db(boost)
        );
        assert!(
            db(two_down).abs() < 1.0,
            "two octaves down reads {} dB",
            db(two_down)
        );
    }

    #[test]
    fn the_top_band_is_a_shelf_and_stays_up() {
        // A shelf's corner is the middle of its transition, not its end: at
        // 6.4 kHz this one has risen half of the requested lift (+7.5 dB of
        // the +15), and it goes on to the full lift above. A bell labelled
        // 6.4 kHz would read +15 at the label and be falling away past it —
        // which is the whole reason the top of this ladder is not a bell.
        let corner = gain_at(&one_band(BANDS - 1, MAX_DB), 6_400.0);
        assert!(
            (db(corner) - MAX_DB / 2.0).abs() < 0.2,
            "the shelf's corner reads {} dB, not half its lift",
            db(corner)
        );
        // Above the corner it does not come back down: it arrives.
        let plateau = gain_at(&one_band(BANDS - 1, MAX_DB), 20_000.0);
        assert!(
            (db(plateau) - MAX_DB).abs() < 0.2,
            "20 kHz reads {} dB against the requested {MAX_DB}",
            db(plateau)
        );
        let between = gain_at(&one_band(BANDS - 1, MAX_DB), 16_000.0);
        assert!(
            db(between) > db(corner) + 6.0,
            "16 kHz reads {} dB against the corner's {} dB",
            db(between),
            db(corner)
        );
        // And well below the corner it leaves the rest of the ladder alone.
        let below = gain_at(&one_band(BANDS - 1, MAX_DB), 800.0);
        assert!(db(below).abs() < 0.2, "800 Hz reads {} dB", db(below));
    }

    #[test]
    fn every_band_reaches_its_own_label_and_leaves_the_ladder_almost_alone() {
        // Push one band and read all seven centres. The numbers here are the
        // measured ones, kept loose enough to be a contract and tight enough
        // to catch a redesign: a bell at its own label reads the lift; the
        // shelf at its label is half way up; a neighbour's centre takes about
        // a fifth of the lift, which is the overlap an octave ladder has by
        // construction; and anything two steps off is under a decibel.
        for index in 0..BANDS {
            let controls = one_band(index, MAX_DB);
            let bands = Mode::G7.bands();
            for (other, probe) in bands.iter().enumerate() {
                let gain = db(gain_at(&controls, probe.hz));
                let distance = other.abs_diff(index);
                if distance == 0 {
                    let wants = match bands[index].shape {
                        Shape::Bell => MAX_DB,
                        Shape::HighShelf => MAX_DB / 2.0,
                    };
                    assert!(
                        (gain - wants).abs() < 0.4,
                        "band {index} at its own {} Hz reads {gain} dB, wants {wants}",
                        probe.hz
                    );
                } else if distance == 1 {
                    assert!(
                        gain.abs() < 3.6,
                        "band {index} at {} moves band {other}'s centre to {gain} dB",
                        probe.hz
                    );
                } else {
                    assert!(
                        gain.abs() < 1.0,
                        "band {index} at {} moves band {other}'s centre to {gain} dB",
                        probe.hz
                    );
                }
            }
        }
    }

    #[test]
    fn full_boost_everywhere_lifts_everything_and_never_cancels() {
        // The criterion A-2 was chosen against: octave-spaced bells at the
        // full +15 dB must not leave a hole the ear reads as a comb. Measured,
        // the all-up curve runs from +23.5 dB at the labels down to +11.2 dB
        // in the valley between the 3.2 kHz bell and the shelf, which is
        // where the two shapes hand over. Two things are worth pinning: the
        // curve never falls to unity anywhere (the ladder covers the whole
        // range it claims), and it never moves by more than a few decibels
        // between neighbouring third-octave points, which is what a toothed
        // response would do.
        let controls = Controls {
            bands: [MAX_DB; BANDS],
            ..Default::default()
        };
        let mut previous: Option<f32> = None;
        let mut worst_step = 0.0f32;
        let mut lowest = 0.0f32;
        let mut probes = 0;
        for twelfth in 0..=84 {
            // 100 Hz up to 6.4 kHz in twelfths, then on to 12.8 kHz.
            let hz = 100.0 * 2.0f32.powf(twelfth as f32 / 12.0);
            let gain = db(gain_at(&controls, hz));
            if let Some(previous) = previous {
                worst_step = worst_step.max((gain - previous).abs());
            } else {
                lowest = gain;
            }
            lowest = lowest.min(gain);
            previous = Some(gain);
            probes += 1;
        }
        assert_eq!(probes, 85);
        assert!(
            worst_step < 4.0,
            "the loudest twelfth-to-twelfth step is {worst_step} dB"
        );
        assert!(
            lowest > MAX_DB - 5.0,
            "the all-up curve falls to {lowest} dB, which is a hole and not a lift"
        );
    }

    #[test]
    fn the_trim_is_a_level_and_the_last_word() {
        let mut controls = Controls {
            gain_db: 15.0,
            ..Default::default()
        };
        let gain = gain_at(&controls, 999.0);
        assert!((db(gain) - 15.0).abs() < 0.05, "trim reads {}", db(gain));
        controls.gain_db = -15.0;
        let gain = gain_at(&controls, 999.0);
        assert!((db(gain) + 15.0).abs() < 0.05, "trim reads {}", db(gain));
        // Past the ends of its range the parameter is clamped at the host,
        // but the DSP does not depend on that.
        controls.gain_db = 90.0;
        let gain = gain_at(&controls, 999.0);
        assert!(
            (db(gain) - 15.0).abs() < 0.05,
            "an over-range trim reads {}",
            db(gain)
        );
    }

    #[test]
    fn every_section_is_stable_at_every_rate_the_settings_offer() {
        let rates = [
            8_000.0, 11_025.0, 16_000.0, 22_050.0, 32_000.0, 44_100.0, 48_000.0, 88_200.0,
            96_000.0, 192_000.0,
        ];
        let levels = [-MAX_DB, -9.0, -0.0, 4.5, MAX_DB];
        for rate in rates {
            for level in levels {
                for band in Mode::G7.bands() {
                    let coefs = match band.shape {
                        Shape::Bell => bell_coefs(band.hz, band.q, level, rate),
                        Shape::HighShelf => shelf_coefs(band.hz, band.q, level, rate),
                    };
                    let all_finite = [coefs.b0, coefs.b1, coefs.b2, coefs.a1, coefs.a2]
                        .iter()
                        .all(|v| v.is_finite());
                    assert!(
                        all_finite && stable(coefs),
                        "{} Hz {:?} at {level} dB and {rate} Hz is unstable: {coefs:?}",
                        band.hz,
                        band.shape,
                    );
                }
            }
        }
    }

    #[test]
    fn a_stepped_move_spreads_over_the_block_instead_of_clicking() {
        // A host's automation arrives as a step at a block boundary. Dropped
        // straight in, a trim move from unity to +15 dB on a 0.9 signal moves
        // one sample to the next by 0.9 × (5.62 − 1) = 4.2, which is not a
        // sound. Spread over a 64-sample block the same move is worth
        // 4.62/64 = 0.072 of gain per sample, 0.065 through a 0.9 signal: the
        // ramp is the filter's own rate of change, and the assertion is that
        // the block contains the ramp and not the step.
        let mut eq = Eq::new(RATE, &Controls::default());
        let mut input = [[0.0f32; BLOCK]; 2];
        let mut output = [[0.0f32; BLOCK]; 2];
        for i in 0..BLOCK {
            input[0][i] = 0.9;
            input[1][i] = 0.9;
        }
        let mut quiet = Controls::default();
        eq.process(&quiet, 2, &input, &mut output);
        quiet.gain_db = MAX_DB;
        eq.process(&quiet, 2, &input, &mut output);
        let mut worst = 0.0f32;
        let mut previous = output[0][0];
        for &sample in output[0].iter() {
            worst = worst.max((sample - previous).abs());
            previous = sample;
        }
        assert!(
            worst < 0.1,
            "one sample moved the output by {worst}, which is a step, not a ramp"
        );
        // And the block does end at the requested level: the ramp is across
        // the block, not an approach that never arrives.
        let end = db(output[0][BLOCK - 1] / 0.9);
        assert!(
            (end - MAX_DB).abs() < 0.1,
            "the block ended at {end} dB instead of {MAX_DB}"
        );
    }

    #[test]
    fn a_rate_change_leaves_the_response_where_the_faders_left_it() {
        // The same question the other tests ask, asked at the new rate after
        // going through `set_rate` rather than being born there: a 44.1 kHz
        // instance moved to 96 kHz must hand back exactly what a 96 kHz
        // instance hands back, because a rate change re-derives the sections
        // from the held faders and retunes nothing the faders did not ask for.
        let controls = one_band(3, 12.0);
        let moved = {
            let mut eq = Eq::new(44_100.0, &controls);
            eq.set_rate(96_000.0);
            let out = run_through(&mut eq, 96_000.0, 1.5, &controls, |i| {
                (2.0 * PI * 800.0 * i as f32 / 96_000.0).sin()
            });
            db(amplitude_at(&out[24_000..], 800.0, 96_000.0))
        };
        let native = db(gain_on(&controls, 96_000.0, 800.0));
        assert!(
            (moved - native).abs() < 0.05,
            "moved to 96 kHz the 800 Hz boost reads {moved:.3} dB, born at 96 kHz it reads {native:.3} dB"
        );
    }

    #[test]
    fn the_tap_selector_decides_which_ends_of_the_box_get_copied() {
        // The feed policy is here rather than in the drawing code because the
        // audio thread pays for it: `In` copies one lane per block, `Both`
        // copies two, and `Scope::Off` — checked in the analyser's own tests —
        // copies neither.
        assert!(Tap::In.wants_input() && !Tap::In.wants_output());
        assert!(!Tap::Out.wants_input() && Tap::Out.wants_output());
        assert!(Tap::Both.wants_input() && Tap::Both.wants_output());
        for (value, want) in [(0, Tap::In), (1, Tap::Out), (2, Tap::Both), (9, Tap::Both)] {
            assert_eq!(Tap::from_param(value), want, "index {value}");
            assert_eq!(Tap::ALL[value.min(2) as usize].label(), want.label());
        }
        for tap in Tap::ALL {
            assert_eq!(Tap::from_param(tap.to_param()), tap);
        }
    }

    #[test]
    fn silence_leaves_no_trace() {
        let controls = Controls {
            bands: [MAX_DB; BANDS],
            ..Default::default()
        };
        let out = run(&controls, 0.5, |_| 0.0);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn two_lanes_get_the_same_equaliser_and_never_each_other() {
        // A stereo host must not have its channels summed: the right lane
        // carries its own copy of the same filter, and one lane's input can
        // never appear in the other's output.
        let mut eq = Eq::new(RATE, &one_band(2, 12.0));
        let mut input = [[0.0f32; BLOCK]; 2];
        let mut output = [[0.0f32; BLOCK]; 2];
        let mut worst_left = 0.0f32;
        let mut worst_right = 0.0f32;
        for block in 0..200 {
            for i in 0..BLOCK {
                let n = block * BLOCK + i;
                input[0][i] = (2.0 * PI * 400.0 * n as f32 / RATE).sin();
                input[1][i] = (2.0 * PI * 400.0 * n as f32 / RATE).cos() * 0.5;
            }
            eq.process(&one_band(2, 12.0), 2, &input, &mut output);
            for i in 0..BLOCK {
                // Same filter, same input shape, different phase: the lanes
                // stay a fixed ratio apart, which a sum would destroy.
                worst_left = worst_left.max(output[0][i].abs());
                worst_right = worst_right.max(output[1][i].abs());
            }
        }
        assert!(worst_left > 1.0 && worst_right > 0.3 && worst_right < worst_left);
    }
}
