//! The Pi-Fase signal path: two lamp-swept all-pass lanes, and the two
//! generators that walk them.
//!
//! The model is the circuit the owner's manual describes and the service
//! drawings (77204 sheet D, 77206 sheet B, and the hand-drawn "A PHASE SHIFT"/
//! "B PHASE SHIFT" page) draw, with the things that cannot be read off a
//! schematic named where they are assumed rather than buried in a constant.
//! What follows is the whole contract, so that no number in this file has to be
//! re-derived from scans and nobody mistakes a taste for a measurement.
//!
//! # What the documents fix
//!
//! * **Six first-order sections a lane, twelve in the machine.** Lane A is
//!   op-amps 1A,1B,2A,2B,3A,3B; lane B is 4A..6B. The manual corroborates the
//!   count with its own arithmetic: with the phasors in series "this gives you a
//!   12-stage phase shifter". Every section: 0.0068 µF coupling, 47K series,
//!   220K across the op-amp in parallel with an incandescent lamp (PH3A..PH3F,
//!   PH4A..PH4F), .47 out.
//! * **The variable element is a lamp, not a JFET**, driven through a
//!   voltage-to-current converter (Q1/Q2, C1852, R98, CR16/CR17, PM3/PM4). One
//!   converter a phasor, so a lane's six lamps track one current and — the
//!   resistors being equal section to section — sit at one corner. That is why a
//!   lane is [`SECTIONS`] copies of one [`Allpass`] on one coefficient, and it is
//!   the same ganged-capacitor argument the four-stage phaser runs on.
//! * **Two sweep generators, free-running.** Lane A's from 21A/21B, lane B's from
//!   2HA/2HB, with S7 reversing B against A. Two oscillators and not one with a
//!   ratio, because their drift is the effect: the manual's name for it is
//!   "two-speed phasing", and that is this plugin's factory state.
//! * **Three modes, and they are one switch.** The Phasor B Input switch (S1)
//!   "effectively selects one of the three basic modes of operation by selecting
//!   which of three signals is applied to the input of Phasor B" — so the routing
//!   is [`Controls::b_input`] and nothing else, and no Mode parameter exists to
//!   contradict the panel.
//! * **.1 Hz to 18 Hz**, "10 seconds per sweep cycle" at the bottom; 390K input
//!   impedance in single-instrument mode and 180K in parallel mode; insertion
//!   loss under 1 dB; and headroom that *shrinks* as feedback grows — 4 V RMS
//!   one phasor no feedback, 2 V both in series at 50% feedback, 1 V at 100%.
//!
//! # What the model adds, and says so
//!
//! * **[`SWEEP_LO_HZ`] is arithmetic from the drawing.** The highest resistance a
//!   section can present is the 220K across it, over the 0.0068 µF it drives:
//!   106 Hz, a *bound* rather than a setting — nothing in this machine can sit at
//!   a lower corner.
//! * **[`SWEEP_HI_HZ`] is a taste number**, and the largest one here. The honest
//!   version wants a PH3A lamp's hot resistance, which no document in this pile
//!   gives. See the constant for what brackets it.
//! * **[`LAMP_LAG_S`] is a thermal time constant guessed in milliseconds.** What
//!   pins it from above is the documented rate range: the lag's corner is 40 Hz,
//!   twice the top sweep rate, so 18 Hz still arrives (9% down) and the
//!   "interesting pseudo-ring modulation" the manual hears at the top of the dial
//!   survives. The lag is what turns a square wave's step into the manual's
//!   "jumps the phasing from one extreme of the sweep to the other" instead of a
//!   coefficient discontinuity, and what glides a pedal. An earlier note in this
//!   file asked for a corner *below* the top rate; that would have strangled the
//!   top third of a dial the manual calls the loudest third.
//! * **[`headroom`] stands in for headroom figures quoted in volts.** Nothing
//!   documents a volt-to-full-scale mapping, so the knee sits at half scale and
//!   the ceiling at one full scale — and everything below half scale is bit-exact
//!   linear, which is where the "under 1 dB" spec lives. It caps both ends of a
//!   lane, which buys the one guarantee the model can actually state: nothing the
//!   switches can reach leaves full scale.
//!
//! # What the model does not do
//!
//! Every section here is an *ideal* all-pass, flat whatever its corner. The
//! drawing's sections have 47K in and a 220K-lamp across, so a real section's
//! gain tilts as its lamp heats; the sheet at hand does not settle that topology
//! well enough to write the transfer function down, and inventing one would dress
//! a guess in a drawing's authority. Two consequences are worth stating plainly.
//! A single lane on one channel with its feedback down is unity magnitude at
//! every frequency — which is precisely what "insertion loss under 1 dB" says the
//! machine does, and why the manual's dramatic settings all have Feedback up and
//! its stereo figure has two amplifiers. And the manual's observation that a
//! *held* pedal "changes the timbres (or tone) of the sound but does not sweep
//! back and forth" reads here as the two-lane and feedback cases, not as proof of
//! a lossy section. A scope across a real lane settles it in an afternoon, and
//! `TODO.md` keeps that note first.
//!
//! Not modelled either: the output-stage lamps PH1/PH2 at J3 and J4, the
//! filament's own compression, and the noise floor — 86 dB below a 2 V input, for
//! anyone who wants to argue about it.
//!
//! Not to be confused with the "BI-PHASER" sheet in the same pile: 4558 op-amps,
//! BF245A JFETs, 47nF/10k, four stages, dual 9–18 V. That is a JFET clone of the
//! idea — useful as a simpler reference, wrong as a source of truth. The README
//! says so again, where a reader meets it first.
//!
//! Everything below is fixed-size and allocation-free, as it has to be under
//! nice-plug's `assert_process_allocs`. The allocation witness includes this file
//! by name rather than the crate, because nice-plug's own debug allocator and the
//! counter cannot both be linked into one binary.

use std::f32::consts::{PI, TAU};

/// The span the control state is refreshed over. A host block is chopped into
/// these, so the resolution of automation — and of everything the switches
/// select — never depends on how large a block the host happens to hand us.
/// Everything inside a block moves every sample.
pub const BLOCK: usize = 64;

/// The bottom of the sweep generators' documented range, hertz. The manual gives
/// both ends in one sentence: "Each control smoothly and continuously varies the
/// sweep rate from .1Hz (10 seconds per sweep cycle) to 18Hz."
pub const RATE_MIN_HZ: f32 = 0.1;
/// The top of it. The lamps' thermal lag is named next to this number because it
/// has to stay out of the way up here.
pub const RATE_MAX_HZ: f32 = 18.0;

/// The sections in one phasor: six, from the drawing's 1A..3B and 4A..6B and from
/// the manual's "12-stage phase shifter" for the pair of them.
pub const SECTIONS: usize = 6;

/// The coupling capacitor every section of every lane is built around, farads:
/// 0.0068 µF, marked at each section's input on the phase-shift sheets.
const COUPLING_F: f32 = 6.8e-9;

/// The fixed resistor across every section's op-amp, ohms. The lamp sits in
/// parallel with it, which is the whole reason the corner moves.
const SHUNT_OHMS: f32 = 220_000.0;

/// The corner of a first-order RC section from the two parts a drawing names. A
/// function rather than a literal so that [`SWEEP_LO_HZ`] stays visibly the
/// arithmetic it is.
const fn corner(ohms: f32, farads: f32) -> f32 {
    1.0 / (2.0 * PI * ohms * farads)
}

/// The bottom of a lane's travel, hertz, derived rather than chosen: 220K with
/// nothing across it over 0.0068 µF is the highest resistance a section can
/// present, so no corner in this machine sits lower. 106 Hz.
pub const SWEEP_LO_HZ: f32 = corner(SHUNT_OHMS, COUPLING_F);

/// The top of the travel, hertz. **A taste number.** The honest version of it is
/// the lamp's hot resistance over the same capacitor, and no document here states
/// a PH3A's resistance at any current. What brackets it is behaviour the manual
/// does commit to: "the Bi-Phase will pick out the individual notes and harmonics
/// of a chord as it sweeps up and down", and six equal sections put their dips at
/// 0.27, 1.0 and 3.7 times the corner, so 4 kHz walks that dip family from 28 Hz
/// to 15 kHz through an instrument's whole harmonic band without parking most of
/// the travel above hearing. It is 38 times [`SWEEP_LO_HZ`], far more resistance
/// travel than a filament's cold-to-hot ratio, so what this constant really
/// asserts is that the voltage-to-current converter's swing, and not the
/// filament's own, is what sets the width of the sweep. Measured, it replaces this
/// paragraph; `TODO.md` carries the debt meanwhile.
pub const SWEEP_HI_HZ: f32 = 4_000.0;

/// The lamps' thermal lag, seconds. Filament temperature follows drive current
/// with a delay, and the manual credits that lag with the sweep's smoothness. The
/// number is a guess in milliseconds with one hard constraint — the corner
/// 1/(2π·τ) ≈ 40 Hz sits well above the documented 18 Hz top rate, so the fastest
/// sweep arrives 9% down, while a square wave's step takes three time constants
/// (12 ms) to arrive.
const LAMP_LAG_S: f32 = 0.004;

/// How fast depth and feedback follow a move, seconds: the siblings' gain slew,
/// for the siblings' reason. A level that lags by tens of milliseconds is its own
/// artifact, and host automation arrives in steps.
const GAIN_SLEW_S: f32 = 0.010;

/// The highest feedback ratio the model closes. The control reaches 1.0 because
/// the machine does — the manual gives a headroom figure for that position and
/// describes what it does — and at exactly 1.0 a loop around a flat magnitude has
/// its pole on the unit circle, which is not a tone but an integrator. The last
/// hundredth is where the hardware breaks into the sustained oscillation owners
/// buy it for; the model stops one hair short rather than grow without bound, and
/// keeps the control's whole travel in exchange.
const FEEDBACK_MAX: f32 = 0.99;

/// Below this level a lane is bit-exact linear; above it [`headroom`] bends.
const LINEAR_LEVEL: f32 = 0.5;

/// The corner of a lane's input coupling network, hertz, from two documented
/// parts: the .47 µF the drawing hangs on each section's output and the 390K input
/// impedance the specification gives for single-instrument mode. 0.87 Hz.
///
/// The machine has no path that carries DC from one section to the next, and this
/// lane has none either — which is not merely tidiness. A chain of ideal
/// all-passes has unity gain at DC, so a feedback loop closed around one
/// amplifies a host's DC offset by 1/(1−feedback): a hundredfold at the top of the
/// control, enough to lodge a lane against [`headroom`] and leave it there. The
/// coupling capacitor is what stops that, here as there.
const DC_CORNER_HZ: f32 = corner(390_000.0, 0.47e-6);

/// The three positions of the Phasor B Input switch (S1), which *are* the
/// machine's three basic modes of operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Routing {
    /// Input A feeds both phasors, as if by a Y-cord: the manual's normal setting
    /// for stereo effects, and the setting its stereo figure draws.
    InA,
    /// Phasor A's output feeds phasor B: twelve stages in series, the manual's
    /// normal setting for one instrument and one amplifier.
    OutA,
    /// Phasor B has its own input, for a second instrument or a stereo one.
    InB,
}

impl Routing {
    /// The routing `b_input` selects. The panel prints the positions in this
    /// order — `IN A`, `OUT A`, `IN B` — so the integers match the hardware's own
    /// layout, and anything out of range reads as `OutA`: the series path the
    /// manual calls normal for the common case, rather than a routing no one asked
    /// for.
    pub fn from_param(value: i32) -> Routing {
        match value {
            0 => Routing::InA,
            2 => Routing::InB,
            _ => Routing::OutA,
        }
    }

    /// The parameter value for this routing. The plugin itself only ever reads the
    /// other direction — the host's integer arrives and [`Routing::from_param`]
    /// interprets it — so this is the face's way back: a segmented control holds a
    /// routing and needs the panel position to write. `TODO.md` has that phase.
    #[allow(dead_code)]
    pub fn to_param(self) -> i32 {
        match self {
            Routing::InA => 0,
            Routing::OutA => 1,
            Routing::InB => 2,
        }
    }

    /// What the panel prints, in the order the positions go round the rotary. The
    /// parameter's position names are built from these, so the words on the face
    /// and the routing the filter runs on cannot drift apart.
    pub const fn label(self) -> &'static str {
        match self {
            Routing::InA => "In A",
            Routing::OutA => "Out A",
            Routing::InB => "In B",
        }
    }

    /// What phasor A hears. `OutA` sums like `InA` because its one instrument is
    /// plugged into Input A; the two positions differ at the *other* lane.
    fn lane_input(self, left: f32, right: f32) -> f32 {
        match self {
            Routing::InB => left,
            Routing::InA | Routing::OutA => summed(left, right),
        }
    }

    /// What phasor B hears, or `None` when the answer is "whatever phasor A just
    /// made" — the cable from output A to input B that makes `OutA` twelve stages
    /// instead of six, which only the sample loop can answer.
    fn lane_b_input(self, left: f32, right: f32) -> Option<f32> {
        match self {
            Routing::InA => Some(summed(left, right)),
            Routing::InB => Some(right),
            Routing::OutA => None,
        }
    }
}

/// The single source a plugin's two input channels make when the hardware had one
/// jack: summed, not split.
///
/// This is the one thing the machine never had to answer and the plugin must. Its
/// "single input" is a mono jack; a plugin's is two channels. Summed, both lanes
/// see the same source, which is what the manual's sentence says they do — "the
/// single input source at Input A is the input to both phasors" — and it leaves
/// the stereo image where the machine put it: in the two phasors drifting against
/// each other, which is why that position is its "normal setting for stereo
/// effects". The dual-mono reading, one lane per side, already exists and already
/// has a name printed on the panel: `In B`, whose second instrument is very often
/// one instrument's other output.
///
/// What it costs is what a Y-cord costs. Two sides that are exact negatives of each
/// other cancel in this sum here as they do in the cable, so a source whose life is
/// in its sides — a wide stereo recording, a mid/sides pair fed through a decoder —
/// wants `In B`, where each lane keeps a jack. The tests carry that as a behaviour
/// rather than a surprise.
fn summed(left: f32, right: f32) -> f32 {
    (left + right) * 0.5
}

/// Which of the machine's sweep sources a phasor is told to follow.
///
/// The positions arrive from the host as integers and the mapping is the panel's:
/// Phasor A has `GEN 1 | PEDAL` (S5), Phasor B has `GEN 1 | GEN 2 | PEDAL` (S6).
/// Anything outside a position reads as the generator a lane starts on, because a
/// lane with no sweep at all is a stranger sound than a lane on the wrong
/// oscillator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Gen1,
    Gen2,
    Pedal,
}

impl Source {
    fn of_a(controls: &Controls) -> Source {
        if controls.a_sweep == 1 {
            Source::Pedal
        } else {
            Source::Gen1
        }
    }

    fn of_b(controls: &Controls) -> Source {
        match controls.b_sweep {
            0 => Source::Gen1,
            2 => Source::Pedal,
            _ => Source::Gen2,
        }
    }

    /// Where this source sits on the travel right now: 0 at the bottom, 1 at the
    /// top. Unipolar on purpose — the bottom is the lamps at rest, which is what
    /// makes depth zero a filter *sitting at the bottom of the band* rather than
    /// parked mid-band, and what makes a held pedal the held filter the manual
    /// describes rather than a slow one.
    fn position(
        self,
        shapes: (i32, i32),
        gen1_sine: f32,
        gen2_sine: f32,
        pedal: f32,
        reversed: bool,
    ) -> f32 {
        let position = match self {
            Source::Pedal => pedal.clamp(0.0, 1.0),
            Source::Gen1 => shaped(shapes.0, gen1_sine),
            Source::Gen2 => shaped(shapes.1, gen2_sine),
        };
        if reversed { 1.0 - position } else { position }
    }
}

/// One generator's position from its shape and its sine: the sine rides its own
/// midpoint, the square "jumps the phasing from one extreme of the sweep to the
/// other". The only judgement in the square is which half is high, and it follows
/// the oscillator's own start: a generator begins at its zero crossing, so a
/// square high from there begins the lane at the top of its travel.
fn shaped(shape: i32, sine: f32) -> f32 {
    if shape == 1 {
        if sine >= 0.0 { 1.0 } else { 0.0 }
    } else {
        (1.0 + sine) * 0.5
    }
}

/// One block's worth of controls, taken at the top of the block so the sample loop
/// touches no atomics. Fields carry the parameter ids they come from.
#[derive(Debug, Clone, Copy, Default)]
pub struct Controls {
    /// Sweep generator 1's rate, hertz.
    pub gen1_rate: f32,
    /// Sweep generator 1's shape: 0 sine, 1 square (the original's SHAPE, S3).
    pub gen1_shape: i32,
    /// Sweep generator 1's rate source: 0 manual, 1 from the pedal (S2).
    pub gen1_rate_source: i32,
    /// Sweep generator 2's rate, hertz.
    pub gen2_rate: f32,
    /// Sweep generator 2's shape (S4).
    pub gen2_shape: i32,
    /// Phasor A's depth, 0..=1: how far the sweep travels up from the bottom of
    /// the band, which is what the panel's own description of the control says it
    /// is. Zero is a filter sitting still at [`SWEEP_LO_HZ`].
    pub a_depth: f32,
    /// Phasor A's feedback, 0..=1. The headroom figures in the header say what the
    /// lamp path expects when this and B's are both up.
    pub a_feedback: f32,
    /// Phasor A's sweep source: 0 generator 1, 1 pedal.
    pub a_sweep: i32,
    /// Phasor B's depth, 0..=1.
    pub b_depth: f32,
    /// Phasor B's feedback, 0..=1.
    pub b_feedback: f32,
    /// Phasor B's sweep sync: 0 normal, 1 reversed (S7).
    pub b_sync: i32,
    /// Phasor B's sweep source: 0 generator 1, 1 generator 2, 2 pedal.
    pub b_sweep: i32,
    /// What drives phasor B, and so which of the machine's three basic modes of
    /// operation it is in: 0 `In A` (one input feeding both phasors — the stereo
    /// setting), 1 `Out A` (A into B, twelve stages in series — the one
    /// instrument, one amplifier setting), 2 `In B` (B has its own input — two
    /// instruments, or a stereo one). [`Routing`] names these, and says what its
    /// positions mean for a plugin's two input channels. The order is the panel's
    /// own, printed clockwise round the rotary as `IN A`, `OUT A`, `IN B`.
    pub b_input: i32,
    /// The expression pedal, 0..=1 — the plugin's port of the pedal jack. Its two
    /// uses are the machine's two switches: a phasor's sweep (S5, S6) or
    /// generator 1's rate (S2). A plugin has no jack, so it has no "nothing
    /// plugged in" state either; a host that never touches this control leaves it
    /// at 0, which is exactly what the hardware's rate generator does with an
    /// empty socket — "a very low rate".
    pub pedal: f32,
}

/// The all-pass coefficient for a section with its −90° crossing at `fc`. With
/// `g = tan(π f / fs)` as the warped frequency coordinate, the section
/// `H = (a + z⁻¹) / (1 + a z⁻¹)` has phase −2·atan(g / t), `t = tan(π fc / fs)`:
/// 0° at DC, exactly −90° at `f = fc`, −180° at Nyquist — the digital mirror of
/// the analog all-pass the hardware built. `t > 0` keeps `a` strictly inside the
/// unit circle for every finite `t`, so every section is stable for any corner the
/// parameters can name, and a lane's six of them are stable together.
fn allpass_a(fc: f32, rate: f32) -> f32 {
    let t = (PI * fc / rate).tan();
    (t - 1.0) / (t + 1.0)
}

/// Where a lane's corner sits for a position on the travel, on the logarithmic
/// scale: equal moves of the sweep are equal *ratios*, which is why it sounds like
/// a filter being walked rather than a knob being turned. Below 0 and above 1 the
/// generator simply stops, so automation outside the parameter's range moves
/// nothing.
pub fn sweep_hz(position: f32) -> f32 {
    SWEEP_LO_HZ * (SWEEP_HI_HZ / SWEEP_LO_HZ).powf(position.clamp(0.0, 1.0))
}

/// The frequency where a lane of `sections` equal all-passes hands its output back
/// *in* phase with its input, `index` whole turns in: the chain's phase is
/// −2·n·atan(g/t) and a trip round the feedback wire wants an even multiple of π,
/// so `g = t·tan(kπ/n)`. Those are the loop's **peaks**. `index` 0 is DC, where the
/// chain has turned not at all, and `k = n/2` is Nyquist, so the usable indices run
/// `1..sections/2`.
///
/// This and its two partners below are gated on `cfg(test)` rather than carried in
/// the binary: the filter walks a corner from the sweep and never asks where its own
/// peaks fell, while the tone probes below are precisely that question. A closed
/// form that only runs inside a test is still the thing under test, which is why it
/// sits out here with the rest of the model rather than inside `mod tests`.
#[cfg(test)]
pub fn peak_hz(fc: f32, index: usize, sections: usize, rate: f32) -> f32 {
    let psi = index as f32 * PI / sections as f32;
    unwarped(fc, psi, rate)
}

/// The loop's **dips**, from the same angle sum at the odd multiples of π:
/// `g = t·tan((2k+1)π/2n)`. For six sections that is 0.27, 1.0 and 3.7 times the
/// corner — the same three ratios the four-stage phaser quotes for its notches,
/// with its 22.5° and 67.5° replaced by this lane's 15°, 45° and 75°.
#[cfg(test)]
pub fn dip_hz(fc: f32, index: usize, sections: usize, rate: f32) -> f32 {
    let psi = (2 * index + 1) as f32 * PI / (2 * sections) as f32;
    unwarped(fc, psi, rate)
}

/// `g = t·tan ψ` warped back to hertz, which is what keeps the two ratios above
/// true at 8 kHz sample rates as well as 192.
#[cfg(test)]
fn unwarped(fc: f32, psi: f32, rate: f32) -> f32 {
    let t = (PI * fc / rate).tan();
    (t * psi.tan()).atan() * rate / PI
}

/// The lane's amplitude ceiling, standing in for the headroom the manual quotes in
/// volts: exactly linear below [`LINEAR_LEVEL`], bending towards one full scale
/// above it, with a continuous slope at the knee.
///
/// The hardware's limit is inside the loop and at its far end, and [`Lane::process`]
/// applies this at both. Inside, because what the 4 V/2 V/1 V figures measure is
/// where a resonating phasor stops growing — a ceiling only at the output would
/// leave the loop itself free to run away. At the output, because the headroom
/// figures are quoted as a voltage into the load, every section is an op-amp with
/// rails, and the jacks have their own lamps (PH1/PH2, not modelled) sitting in
/// front of them. The consequence is a promise worth having: no position of any
/// control, and no combination of the two lanes, leaves the plugin's outputs
/// outside full scale. The filament's own, subtler, level-dependent compression is
/// not here, and `TODO.md` says so.
fn headroom(x: f32) -> f32 {
    let magnitude = x.abs();
    if magnitude <= LINEAR_LEVEL {
        return x;
    }
    let over = (magnitude - LINEAR_LEVEL) / (1.0 - LINEAR_LEVEL);
    let bent = LINEAR_LEVEL + (1.0 - LINEAR_LEVEL) * over.tanh();
    x.signum() * bent
}

/// One section's memory: the input and the output a sample back. That is the whole
/// of a first-order all-pass, and the reason the machine could afford six.
#[derive(Clone, Copy, Default)]
struct Allpass {
    x1: f32,
    y1: f32,
}

impl Allpass {
    /// `y[n] = a·x[n] + x[n−1] − a·y[n−1]`: two multiply-accumulates of actual
    /// work. The coefficient arrives every sample because the lamps are moving
    /// every sample, not because the section wants advice mid-block.
    fn process(&mut self, a: f32, x: f32) -> f32 {
        let y = a * x + self.x1 - a * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// One phasor: six sections on one corner, the input coupling network, the feedback
/// wire, and the lamp's remembered position.
#[derive(Clone, Copy)]
struct Lane {
    sections: [Allpass; SECTIONS],
    /// The coupling network's two memories, in the direct form
    /// `y = a·(y₁ + x − x₁)`: a first-order highpass at [`DC_CORNER_HZ`] once `a`
    /// is the pole [`block_at`] gives, which is the whole of why that is a
    /// function and not a division.
    y1: f32,
    x1: f32,
    /// Where the lamps sit on the travel, 0..=1, mid-lag. The one piece of state
    /// the ear is actually watching: a face that shows the sweep reads this, and
    /// the tests read it far more often than a host does.
    lamp: f32,
    /// Depth and feedback mid-slew, so a switch or an automation step moves the
    /// filter over 10 ms rather than in one sample.
    depth: f32,
    feedback: f32,
    /// The lane's output as of the previous sample: the one-sample register the
    /// feedback loop closes through.
    fb_bus: f32,
}

impl Lane {
    /// A lane at rest at the position its controls describe, so that activation —
    /// and a sample-rate change — begins where the settings say the lamps already
    /// are instead of slewing up from wherever the defaults sat.
    fn resting(depth: f32, feedback: f32, position: f32) -> Self {
        Self {
            sections: [Allpass::default(); SECTIONS],
            y1: 0.0,
            x1: 0.0,
            lamp: depth * position,
            depth,
            feedback,
            fb_bus: 0.0,
        }
    }

    /// One sample: the coupling cap, the summing node, the six sections, and the
    /// wire back.
    ///
    /// [`headroom`] is applied twice — at the summing node, where it is what stops
    /// a resonating loop growing, and on the way out, because every section is an
    /// op-amp with rails and the manual quotes its headroom as a voltage *at the
    /// output*. The second one is not decoration: a chain of all-passes is flat
    /// forever but not on a transient, and a lane fed a limited signal can hand
    /// back more than it was given.
    fn process(&mut self, a: f32, feedback: f32, coupling: f32, input: f32) -> f32 {
        let blocked = coupling * (self.y1 + input - self.x1);
        self.y1 = blocked;
        self.x1 = input;
        let mut sample = headroom(blocked + feedback * self.fb_bus);
        for section in self.sections.iter_mut() {
            sample = section.process(a, sample);
        }
        let tap = headroom(sample);
        self.fb_bus = tap;
        tap
    }
}

/// The two generators' and the lanes' constants for one block, derived once from
/// the controls so the sample loop does arithmetic rather than policy.
#[derive(Clone, Copy)]
struct Step {
    /// Radians per sample, one per generator.
    gen1: f32,
    gen2: f32,
    /// Both shapes, as `(generator 1, generator 2)`.
    shapes: (i32, i32),
    a_source: Source,
    b_source: Source,
    /// The pedal, clamped once here instead of twice per sample.
    pedal: f32,
    /// S7: does phasor B mirror the source it is on?
    reversed: bool,
    /// The two depths and the two feedback ratios, clamped to the control ranges
    /// the host may exceed by hand.
    a_depth: f32,
    a_feedback: f32,
    b_depth: f32,
    b_feedback: f32,
    /// The one-pole steps for the lamp's lag and for the gain slews.
    lamp: f32,
    gain: f32,
    /// The input coupling network's pole, from [`block_at`].
    coupling: f32,
    /// The most any section's corner may be told at this sample rate.
    nyquist: f32,
}

impl Step {
    fn new(rate: f32, controls: &Controls) -> Self {
        let rate = rate.max(1.0);
        // Generator 1's rate comes either from its knob or from the pedal, and the
        // pedal end has to make a documented sentence true: Rate Control on Ped
        // with nothing in the jack "leaves the generator at a very low rate". A
        // plugin's pedal at rest *is* an empty socket, so the taper starts at
        // `RATE_MIN_HZ` and the sentence holds by construction. Between the two
        // documented ends the taper is exponential, which is the assumption the
        // rate knob's own default already says it is.
        let pedalled =
            RATE_MIN_HZ * (RATE_MAX_HZ / RATE_MIN_HZ).powf(controls.pedal.clamp(0.0, 1.0));
        let gen1 = if controls.gen1_rate_source == 1 {
            pedalled
        } else {
            controls.gen1_rate.max(0.0)
        };
        Self {
            gen1: TAU * gen1 / rate,
            gen2: TAU * controls.gen2_rate.max(0.0) / rate,
            shapes: (controls.gen1_shape, controls.gen2_shape),
            a_source: Source::of_a(controls),
            b_source: Source::of_b(controls),
            pedal: controls.pedal.clamp(0.0, 1.0),
            reversed: controls.b_sync == 1,
            a_depth: controls.a_depth.clamp(0.0, 1.0),
            a_feedback: controls.a_feedback.clamp(0.0, 1.0),
            b_depth: controls.b_depth.clamp(0.0, 1.0),
            b_feedback: controls.b_feedback.clamp(0.0, 1.0),
            lamp: time_constant(rate, LAMP_LAG_S),
            gain: time_constant(rate, GAIN_SLEW_S),
            coupling: block_at(rate, DC_CORNER_HZ),
            nyquist: rate * 0.45,
        }
    }
}

/// The one-pole step for a time constant of `seconds` at `rate`. The lag and the
/// slews advance by wall-clock rather than by block count, so the feel is the same
/// at a 64-sample block and a 2048-sample one — and, since both move per sample
/// and not per block, the same at any block *length* too.
fn time_constant(rate: f32, seconds: f32) -> f32 {
    1.0 - (-1.0 / (rate.max(1.0) * seconds)).exp()
}

/// The pole of a first-order coupling network whose corner is `fc` hertz at
/// `rate`: the number [`Lane::process`] iterates on, which is the *decay*
/// `exp(−2π f_c/fs)` — one minus the same [`time_constant`] the lags use, and
/// just under unity.
///
/// Worth a function of its own because the near miss is invisible in arithmetic
/// and total in sound: write the corner itself, `2π f_c/fs`, into that
/// difference equation and the lane turns into a differentiator some 70 dB down
/// at every frequency the instrument lives at, rather than a capacitor that
/// passes everything above 0.87 Hz and stops only the DC.
fn block_at(rate: f32, fc: f32) -> f32 {
    1.0 - time_constant(rate, 1.0 / (TAU * fc))
}

/// The Bi-Phase: two sweep generators, two phasors of six sections each, and the
/// switch that says what feeds the second one.
pub struct PiFace {
    rate: f32,
    /// Both generators' phase in radians, wrapped to `0..2π`. Phase and not time,
    /// so a rate move continues the sweep where it stands instead of snapping the
    /// oscillator to a new clock; two accumulators and not one with a ratio,
    /// because their drift is the effect the manual calls two-speed phasing.
    gen1_phase: f32,
    gen2_phase: f32,
    lanes: [Lane; 2],
}

impl PiFace {
    /// A filter ready for `rate`, parked where the controls it is handed say it
    /// already is.
    pub fn new(rate: f32, controls: &Controls) -> Self {
        let rate = rate.max(1.0);
        let step = Step::new(rate, controls);
        // Both generators start on their zero crossing, so both lanes start at the
        // middle of whatever travel their source gives them.
        let a_position = step
            .a_source
            .position(step.shapes, 0.0, 0.0, step.pedal, false);
        let b_position = step
            .b_source
            .position(step.shapes, 0.0, 0.0, step.pedal, step.reversed);
        Self {
            rate,
            gen1_phase: 0.0,
            gen2_phase: 0.0,
            lanes: [
                Lane::resting(step.a_depth, step.a_feedback, a_position),
                Lane::resting(step.b_depth, step.b_feedback, b_position),
            ],
        }
    }

    /// Re-tune for a new device or host rate. Every corner is re-derived from the
    /// position the lamps are sitting on, so a rate change moves the *digital*
    /// filter while leaving the sweep where the player left it, which is the one
    /// thing the standalone's device switch needs to be able to say.
    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate.max(1.0);
    }

    /// Silence's memory: the sections, the coupling caps and the feedback wires
    /// empty, while the generators keep their phase and the lamps keep their heat.
    /// A reset should not restart a sweep under the player's feet, and a filament
    /// does not cool between phrases.
    pub fn reset(&mut self) {
        for lane in self.lanes.iter_mut() {
            lane.sections = [Allpass::default(); SECTIONS];
            lane.y1 = 0.0;
            lane.x1 = 0.0;
            lane.fb_bus = 0.0;
        }
    }

    /// Where lane `lane`'s corner sits, hertz — the readout for a face that shows
    /// the sweep, and the thread that knows the sample rate. The editor that will
    /// read it is the GUI phase in `TODO.md`, so until then the tests are its only
    /// caller.
    #[allow(dead_code)]
    pub fn corner_hz(&self, lane: usize) -> f32 {
        sweep_hz(self.lanes[lane.min(1)].lamp)
    }

    /// Where lane `lane`'s lamps sit on the travel, 0..=1, mid-lag. The number a
    /// face animates, and the one the movement tests read.
    #[allow(dead_code)]
    pub fn sweep_position(&self, lane: usize) -> f32 {
        self.lanes[lane.min(1)].lamp
    }

    /// Run one block of one channel — up to [`BLOCK`] samples, in and out separate
    /// buffers; the caller copies through if the host asked for in-place. With a
    /// second channel, prefer [`PiFace::process_stereo`].
    ///
    /// One input channel is one source available at *both* jacks, which is what
    /// makes `In B` mean the same thing here that it does in a host with a stereo
    /// insert: with one mono signal, a second instrument and a Y-cord are the same
    /// patch. The output is phasor A except in `Out A`, where a mono insert *is*
    /// the machine's one-instrument-one-amplifier case and the answer is the far
    /// end of the series chain, twelve stages.
    pub fn process_mono(&mut self, controls: &Controls, input: &[f32], output: &mut [f32]) {
        let n = input.len().min(output.len()).min(BLOCK);
        let step = Step::new(self.rate, controls);
        let routing = Routing::from_param(controls.b_input);
        for (i, out) in output[..n].iter_mut().enumerate() {
            let (a_out, b_out) = self.run_sample(&step, routing, input[i], input[i]);
            *out = match routing {
                Routing::OutA => b_out,
                _ => a_out,
            };
        }
    }

    /// Run one block of two channels: phasor A to the left, phasor B to the right,
    /// which is output A at J3 and output B at J4. In `Out A` the left is the
    /// six-stage tap and the right the twelve-stage one — the same two jacks, and a
    /// player who only wants the series sound takes one of them.
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
        let step = Step::new(self.rate, controls);
        let routing = Routing::from_param(controls.b_input);
        for i in 0..n {
            let (a_out, b_out) = self.run_sample(&step, routing, inputs[0][i], inputs[1][i]);
            outputs[0][i] = a_out;
            outputs[1][i] = b_out;
        }
    }

    /// One sample of both lanes, whatever the routing says joins them. Returns both
    /// taps; a mono caller drops one.
    fn run_sample(&mut self, step: &Step, routing: Routing, left: f32, right: f32) -> (f32, f32) {
        // The generators walk one sample at a time: a square wave's edge belongs to
        // the sample it lands on, not to the block that happens to contain it.
        self.gen1_phase = (self.gen1_phase + step.gen1) % TAU;
        self.gen2_phase = (self.gen2_phase + step.gen2) % TAU;
        let s1 = self.gen1_phase.sin();
        let s2 = self.gen2_phase.sin();

        let a_position = step
            .a_source
            .position(step.shapes, s1, s2, step.pedal, false);
        let b_position = step
            .b_source
            .position(step.shapes, s1, s2, step.pedal, step.reversed);

        // Both lanes at once, so the borrow checker can see them as the two
        // separate state machines they are.
        let [a, b] = &mut self.lanes;
        a.depth += (step.a_depth - a.depth) * step.gain;
        a.feedback += (step.a_feedback - a.feedback) * step.gain;
        a.lamp += (a.depth * a_position - a.lamp) * step.lamp;
        let a_corner = sweep_hz(a.lamp).min(step.nyquist);
        let a_out = a.process(
            allpass_a(a_corner, self.rate),
            a.feedback.min(FEEDBACK_MAX),
            step.coupling,
            routing.lane_input(left, right),
        );

        b.depth += (step.b_depth - b.depth) * step.gain;
        b.feedback += (step.b_feedback - b.feedback) * step.gain;
        b.lamp += (b.depth * b_position - b.lamp) * step.lamp;
        let b_corner = sweep_hz(b.lamp).min(step.nyquist);
        // `Out A` is a cable from output A to input B, and a cable in an analog box
        // carries no sample of latency: lane B sees the sample lane A just made, in
        // the same instant.
        let b_in = routing.lane_b_input(left, right).unwrap_or(a_out);
        let b_out = b.process(
            allpass_a(b_corner, self.rate),
            b.feedback.min(FEEDBACK_MAX),
            step.coupling,
            b_in,
        );
        (a_out, b_out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    /// Both generators frozen mid-travel, which is what every tone probe below
    /// needs: a rate of 0 leaves the phase where it started, both sines are 0, and
    /// every source therefore sits at 0.5. The parameter's own floor is 0.1 Hz; the
    /// filter has no business refusing what a test asks for, any more than the
    /// phaser's does.
    fn frozen() -> Controls {
        Controls {
            a_depth: 1.0,
            b_depth: 1.0,
            ..Default::default()
        }
    }

    /// Run the filter over `seconds` of two channels, returning both taps.
    fn run_stereo(
        controls: &Controls,
        seconds: f32,
        mut left: impl FnMut(usize) -> f32,
        mut right: impl FnMut(usize) -> f32,
    ) -> (Vec<f32>, Vec<f32>) {
        let mut pi_face = PiFace::new(RATE, controls);
        let total = (RATE * seconds) as usize;
        let mut out_l = vec![0.0; total];
        let mut out_r = vec![0.0; total];
        let mut in_l = [0.0; BLOCK];
        let mut in_r = [0.0; BLOCK];
        let mut buf_l = [0.0; BLOCK];
        let mut buf_r = [0.0; BLOCK];
        let mut start = 0;
        while start < total {
            let n = BLOCK.min(total - start);
            for (i, sample) in in_l[..n].iter_mut().enumerate() {
                *sample = left(start + i);
            }
            for (i, sample) in in_r[..n].iter_mut().enumerate() {
                *sample = right(start + i);
            }
            pi_face.process_stereo(
                controls,
                [&in_l[..n], &in_r[..n]],
                [&mut buf_l[..n], &mut buf_r[..n]],
            );
            out_l[start..start + n].copy_from_slice(&buf_l[..n]);
            out_r[start..start + n].copy_from_slice(&buf_r[..n]);
            start += n;
        }
        (out_l, out_r)
    }

    /// The same on one channel, taken from the left tap. The probe goes to *both*
    /// inputs: in `In A` the two are summed, so a tone on one side alone would be
    /// measuring the summing rather than the lane.
    fn run(controls: &Controls, seconds: f32, signal: impl Fn(usize) -> f32 + Copy) -> Vec<f32> {
        let (left, _) = run_stereo(controls, seconds, signal, signal);
        left
    }

    /// Drive the filter for `seconds` and watch lane 0's sweep, block by block: the
    /// questions about *movement* are all this.
    fn positions(controls: &Controls, seconds: f32) -> Vec<f32> {
        let mut pi_face = PiFace::new(RATE, controls);
        let block = [0.0f32; BLOCK];
        let mut out = [0.0f32; BLOCK];
        let mut seen = Vec::new();
        for _ in 0..((RATE * seconds) as usize / BLOCK) {
            pi_face.process_mono(controls, &block, &mut out);
            seen.push(pi_face.sweep_position(0));
        }
        seen
    }

    /// The amplitude of `probe_hz` in a window of samples, one bin of a DFT. Every
    /// magnitude question below reduces to this: how loud does a pure tone come
    /// out, measured once the filters have settled.
    fn amplitude(x: &[f32], probe_hz: f32) -> f32 {
        // A DFT over nothing measures nothing, and the ratio at the end answers NaN
        // rather than complaining — which is how a probe whose window was sliced
        // away entirely reads as "no signal" instead of as a broken test. Every
        // window here is at least a tenth of a second.
        assert!(
            x.len() > RATE as usize / 10,
            "measured {} samples of {} Hz",
            x.len(),
            probe_hz
        );
        let w = 2.0 * PI * probe_hz / RATE;
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

    /// A probe tone, at a level well inside the linear half of the lane so that
    /// [`headroom`] cannot colour a closed-form prediction.
    fn tone(probe_hz: f32) -> impl Fn(usize) -> f32 + Copy {
        move |i| 0.2 * (2.0 * PI * probe_hz * i as f32 / RATE).sin()
    }

    /// The left tap's gain at a frequency with the sweep frozen. Half a second is a
    /// dozen lifetimes of both slews, and the first quarter of it is discarded so
    /// the coupling network is through by the time anything is measured.
    fn gain_at(controls: &Controls, probe_hz: f32) -> f32 {
        let signal = run(controls, 0.5, tone(probe_hz));
        amplitude(&signal[(RATE as usize / 4)..], probe_hz) / 0.2
    }

    /// The same question on the right-hand tap, which is how the routing tests ask
    /// which lane a signal reached. Both channels carry the tone, because `In A`
    /// sums them and a test that fed one side only would be measuring the sum.
    fn gain_at_right(controls: &Controls, probe_hz: f32) -> f32 {
        let (_, right) = run_stereo(controls, 0.5, tone(probe_hz), tone(probe_hz));
        amplitude(&right[(RATE as usize / 4)..], probe_hz) / 0.2
    }

    #[test]
    fn six_sections_a_lane_and_arithmetic_behind_the_floor() {
        // The count is the drawing's — 1A..3B and 4A..6B — and the manual's own
        // "12-stage phase shifter". The floor is arithmetic: the 220K across a
        // section over its 0.0068 µF is the most resistance the section can
        // present, so nothing in this machine sweeps below it.
        assert_eq!(SECTIONS, 6);
        assert!((SWEEP_LO_HZ - 106.4).abs() < 0.5, "floor {SWEEP_LO_HZ}");
        assert!((SWEEP_LO_HZ - corner(220_000.0, 6.8e-9)).abs() < 1e-3);
        // And the travel is 4 kHz of taste laid over that 106 Hz of arithmetic.
        // Compile time, because the claim is about two constants and watching a
        // runtime add together two numbers that cannot change is theatre.
        const _: () = assert!(SWEEP_HI_HZ > 30.0 * SWEEP_LO_HZ);
    }

    #[test]
    fn the_travel_is_exponential_between_its_ends() {
        assert!((sweep_hz(0.0) - SWEEP_LO_HZ).abs() < 0.01);
        assert!((sweep_hz(1.0) - SWEEP_HI_HZ).abs() < 0.01);
        // The geometric mean, because the scale is the ratio and not the step.
        let middle = sweep_hz(0.5);
        assert!((middle - (SWEEP_LO_HZ * SWEEP_HI_HZ).sqrt()).abs() < 0.5);
        // Past the ends the generator simply stops: automation outside the
        // parameter's range moves nothing.
        assert!((sweep_hz(-3.0) - SWEEP_LO_HZ).abs() < 0.01);
        assert!((sweep_hz(9.0) - SWEEP_HI_HZ).abs() < 0.01);
    }

    #[test]
    fn the_coefficient_always_lives_in_the_unit_circle() {
        // The stability argument for one lane, and therefore for twelve sections in
        // series: at every rate a host may activate at and every position depth and
        // the generators can put a lane at, the coefficient stays strictly inside
        // the unit circle — and so does any blend of two of them.
        for &rate in &[8_000.0, 22_057.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            for i in 0..=64 {
                let position = i as f32 / 64.0;
                let fc = sweep_hz(position).min(rate * 0.45);
                let a = allpass_a(fc, rate);
                assert!(a.abs() < 1.0, "position {position} rate {rate}: a {a}");
            }
        }
    }

    #[test]
    fn a_lane_without_feedback_is_flat_and_unity() {
        // The specification's "insertion loss under 1 dB", stated as the
        // consequence of a chain of all-passes rather than as an aspiration: a
        // frozen lane with its feedback down passes everything at unity, at every
        // frequency. This is the model's most conspicuous consequence — a lone lane
        // on one channel cannot change a magnitude — and the header argues for it
        // rather than hiding it.
        let controls = frozen();
        for probe in [80.0, 200.0, 650.0, 1_500.0, 4_000.0, 9_000.0] {
            let g = gain_at(&controls, probe);
            assert!((g - 1.0).abs() < 0.02, "{probe} Hz: gain {g}");
        }
    }

    #[test]
    fn the_peaks_and_dips_sit_where_six_sections_put_them() {
        // The feedback wire is the only thing in a lane that moves magnitude, so it
        // is the thing worth a closed form. Six equal sections: the loop peaks where
        // the round trip is an even multiple of π — tan 30° and tan 60° times the
        // warped corner — and dips at tan 15°, tan 45° and tan 75°. A loop with a
        // flat magnitude and one sample of delay answers 1/(1−f) at a peak and
        // 1/(1+f) at a dip.
        let fc = sweep_hz(0.5);
        let feedback = 0.5;
        let controls = Controls {
            a_feedback: feedback,
            ..frozen()
        };
        for index in 1..SECTIONS / 2 {
            let peak = peak_hz(fc, index, SECTIONS, RATE);
            let want = 1.0 / (1.0 - feedback);
            let measured = gain_at(&controls, peak);
            assert!(
                (measured - want).abs() < 0.08,
                "peak {index} at {peak:.0} Hz: {measured} against {want}"
            );
        }
        for index in 0..SECTIONS / 2 {
            let dip = dip_hz(fc, index, SECTIONS, RATE);
            if dip >= RATE * 0.2 {
                continue;
            }
            let want = 1.0 / (1.0 + feedback);
            let measured = gain_at(&controls, dip);
            assert!(
                (measured - want).abs() < 0.05,
                "dip {index} at {dip:.0} Hz: {measured} against {want}"
            );
        }
    }

    #[test]
    fn a_lane_at_full_feedback_stops_growing_instead_of_never_stopping() {
        // The control reaches 1.0 because the machine does, and the manual's
        // headroom figures — 4 V, 2 V, 1 V as the feedback goes up — are the
        // hardware saying where a resonating lane stops. Here that job is
        // [`headroom`]'s, and the promise is bounded rather than exact.
        let fc = sweep_hz(0.5);
        let peak = peak_hz(fc, 1, SECTIONS, RATE);
        let controls = Controls {
            a_feedback: 1.0,
            ..frozen()
        };
        let out = run(&controls, 2.0, |i| {
            0.9 * (2.0 * PI * peak * i as f32 / RATE).sin()
        });
        let worst = out.iter().fold(0.0f32, |m, &y| m.max(y.abs()));
        assert!(out.iter().all(|&y| y.is_finite()), "NaN out of a lane");
        // The lane's tap passes through [`headroom`] on its way out, whose
        // asymptote is one full scale, so this bound is the model's promise rather
        // than an observation: a lane at the top of a control that reaches its top
        // rings as loud as it is allowed to and no louder.
        assert!(worst > 0.5, "the loop never rang: {worst}");
        assert!(worst <= 1.0, "100% feedback reached {worst}");
    }

    #[test]
    fn depth_is_how_far_the_sweep_travels_and_zero_leaves_it_still() {
        // "How far the sweep travels" is the panel's own account of the control, so
        // depth 0 is a filter sitting at the bottom of the band and depth 1 walks
        // the whole 106 Hz-to-4 kHz travel.
        let still = Controls {
            gen1_rate: 4.0,
            a_depth: 0.0,
            ..Default::default()
        };
        let seen = positions(&still, 2.0);
        let lowest = seen.iter().fold(f32::MAX, |m, &p| m.min(p));
        let highest = seen.iter().fold(f32::MIN, |m, &p| m.max(p));
        assert!(
            (highest - lowest) < 1e-6,
            "depth 0 moved the sweep: {lowest}..{highest}"
        );
        assert_eq!(lowest, 0.0, "depth 0 is not sitting at the bottom");
        assert!((sweep_hz(lowest) - SWEEP_LO_HZ).abs() < 0.01);

        let full = Controls {
            gen1_rate: 4.0,
            a_depth: 1.0,
            ..Default::default()
        };
        let seen = positions(&full, 4.0);
        let lowest = seen.iter().fold(f32::MAX, |m, &p| m.min(p));
        let highest = seen.iter().fold(f32::MIN, |m, &p| m.max(p));
        assert!(lowest < 0.02, "bottom of the travel: {lowest}");
        assert!(highest > 0.98, "top of the travel: {highest}");
    }

    #[test]
    fn two_generators_beat_against_each_other() {
        // Why there are two oscillators rather than one with a ratio: on different
        // rates they drift, and the manual's two-speed phasing *is* the drift. Lane
        // A on generator 1 at 1 Hz, lane B on generator 2 at 1.3 Hz — the gap
        // between them has to change sign, or the two are one oscillator with an
        // offset painted on.
        let mut controls = Controls {
            gen1_rate: 1.0,
            gen2_rate: 1.3,
            a_depth: 1.0,
            b_depth: 1.0,
            b_sweep: 1,
            ..Default::default()
        };
        let mut pi_face = PiFace::new(RATE, &controls);
        let block = [0.0f32; BLOCK];
        let mut out = [0.0f32; BLOCK];
        let mut crossings = 0;
        let mut previous = 0.0f32;
        for _ in 0..((12.0 * RATE) as usize / BLOCK) {
            pi_face.process_mono(&controls, &block, &mut out);
            let gap = pi_face.sweep_position(0) - pi_face.sweep_position(1);
            if previous != 0.0 && gap.signum() != previous.signum() {
                crossings += 1;
            }
            if gap.abs() > 1e-6 {
                previous = gap;
            }
        }
        assert!(crossings >= 2, "the lanes drifted but never met");
        controls.gen2_rate = 1.0;
        let mut pi_face = PiFace::new(RATE, &controls);
        let mut apart = 0.0f32;
        for _ in 0..((12.0 * RATE) as usize / BLOCK) {
            pi_face.process_mono(&controls, &block, &mut out);
            apart = apart.max((pi_face.sweep_position(0) - pi_face.sweep_position(1)).abs());
        }
        assert!(
            apart < 1e-6,
            "two generators on one rate should agree, {apart}"
        );
    }

    #[test]
    fn sync_normal_walks_together_and_reversed_walks_against() {
        // One source, two lanes, and the switch between them. Norm: the two
        // positions are the same number, which is what makes the series routing
        // "double depth" rather than something else. Rev: they sum to the depth,
        // because one is the other mirrored — and that is the manual's reason for
        // mentioning the position at all, "there is no double-depth phasing".
        let base = Controls {
            gen1_rate: 0.6,
            a_depth: 1.0,
            b_depth: 1.0,
            a_sweep: 0,
            b_sweep: 0,
            ..Default::default()
        };
        for (sync, want_together) in [(0, true), (1, false)] {
            let controls = Controls {
                b_sync: sync,
                ..base
            };
            let mut pi_face = PiFace::new(RATE, &controls);
            let block = [0.0f32; BLOCK];
            let mut out = [0.0f32; BLOCK];
            let mut worst = 0.0f32;
            let warmup = (0.5 * RATE) as usize / BLOCK;
            for i in 0..((3.0 * RATE) as usize / BLOCK) {
                pi_face.process_mono(&controls, &block, &mut out);
                if i > warmup {
                    let a = pi_face.sweep_position(0);
                    let b = pi_face.sweep_position(1);
                    let error = if want_together {
                        (a - b).abs()
                    } else {
                        (a + b - 1.0).abs()
                    };
                    worst = worst.max(error);
                }
            }
            assert!(worst < 5e-3, "sync {sync}: worst error {worst}");
        }
    }

    #[test]
    fn a_held_pedal_is_a_static_filter_and_a_moving_one_sweeps() {
        // Two sentences from the manual, both about S5 on Pedal: a pedal that is not
        // moving "changes the timbres (or tone) of the sound but does not sweep back
        // and forth", and the generator's own rate is then beside the point — 12 Hz
        // of generator has to leave lane A exactly where the pedal put it.
        let mut controls = Controls {
            gen1_rate: 12.0,
            a_depth: 1.0,
            a_sweep: 1,
            pedal: 0.4,
            ..Default::default()
        };
        let mut pi_face = PiFace::new(RATE, &controls);
        let block = [0.0f32; BLOCK];
        let mut out = [0.0f32; BLOCK];
        for _ in 0..((3.0 * RATE) as usize / BLOCK) {
            pi_face.process_mono(&controls, &block, &mut out);
        }
        let want = sweep_hz(0.4);
        assert!(
            (pi_face.corner_hz(0) - want).abs() < 1.0,
            "a held pedal is a filter at {want} Hz, not at {}",
            pi_face.corner_hz(0)
        );
        let held = pi_face.corner_hz(0);
        for _ in 0..((2.0 * RATE) as usize / BLOCK) {
            pi_face.process_mono(&controls, &block, &mut out);
        }
        assert!(
            (pi_face.corner_hz(0) - held).abs() < 0.01,
            "a held pedal must not sweep"
        );

        // The same pedal, moved: the corner follows it, which is all the switch is
        // for.
        controls.pedal = 0.9;
        for _ in 0..((2.0 * RATE) as usize / BLOCK) {
            pi_face.process_mono(&controls, &block, &mut out);
        }
        assert!(
            pi_face.corner_hz(0) > want + 100.0,
            "the pedal did nothing: {}",
            pi_face.corner_hz(0)
        );
    }

    #[test]
    fn rate_control_on_pedal_spans_the_documented_rate_range() {
        // S2 on Man is the knob; on Ped it is the expression pedal, and the manual
        // adds that with nothing plugged in the generator "stays at a very low
        // rate". A plugin's pedal at rest *is* an empty socket, so the taper has to
        // start at `RATE_MIN_HZ` for that sentence to be true rather than aspirational
        // — and its top is the other documented end, 18 Hz. The shape between them is
        // exponential, which is an assumption and says so where the rates are chosen.
        let step_at = |pedal: f32| {
            Step::new(
                RATE,
                &Controls {
                    gen1_rate: 7.0,
                    gen1_rate_source: 1,
                    pedal,
                    ..Default::default()
                },
            )
            .gen1
        };
        let hz = |step: f32| step * RATE / TAU;
        assert!((hz(step_at(0.0)) - RATE_MIN_HZ).abs() < 1e-4);
        assert!((hz(step_at(1.0)) - RATE_MAX_HZ).abs() < 1e-3);
        assert!((hz(step_at(0.5)) - (RATE_MIN_HZ * RATE_MAX_HZ).sqrt()).abs() < 1e-2);
        // On Man the same generator obeys the knob instead, and the knob's range is
        // the documented one.
        let manual = Step::new(
            RATE,
            &Controls {
                gen1_rate: 12.0,
                gen1_rate_source: 0,
                pedal: 1.0,
                ..Default::default()
            },
        );
        assert!((hz(manual.gen1) - 12.0).abs() < 1e-4);
        // A rate of 0 freezes a generator, which is how the tone probes work; a
        // negative one from a hand-edited preset runs backwards rather than
        // wrapping into a thump.
        let stopped = Step::new(RATE, &Controls::default());
        assert_eq!(hz(stopped.gen1), 0.0);
    }

    #[test]
    fn a_square_sweep_holds_one_extreme_and_then_jumps_to_the_other() {
        // "Jumps the phasing from one extreme of the sweep to the other." The lamps'
        // lag rounds the jump — a filament, not a relay — so the honest check is that
        // a cycle is two plateaus and a little transition: at 1 Hz with a 4 ms lag,
        // over 95% of samples sit within 5% of one end or the other.
        let controls = Controls {
            gen1_rate: 1.0,
            gen1_shape: 1,
            a_depth: 1.0,
            ..Default::default()
        };
        let seen = positions(&controls, 3.0);
        let settled = &seen[seen.len() / 3..];
        let at_an_end = settled.iter().filter(|&&p| p < 0.05 || p > 0.95).count();
        let share = at_an_end as f32 / settled.len() as f32;
        assert!(share > 0.95, "only {share} of the square sat at an extreme");
        // And it does reach both: a shape that only ever sat high would pass the
        // test above.
        assert!(settled.iter().any(|&p| p > 0.95));
        assert!(settled.iter().any(|&p| p < 0.05));
    }

    #[test]
    fn in_a_puts_one_summed_source_into_both_phasors() {
        // The decision the hardware never had to make, taken here and nailed down.
        // `In A` with a stereo input feeds both lanes the *sum*, so a source on the
        // left alone and one on the right alone drive phasor A with the same
        // 0.5·signal each and it cannot tell the two cases apart. The lanes differ
        // from each other — that is where the stereo comes from — but the input is
        // not split between them, because that is what `In B` is.
        let controls = Controls {
            b_input: Routing::InA.to_param(),
            ..frozen()
        };
        let mut left_only = PiFace::new(RATE, &controls);
        let mut right_only = PiFace::new(RATE, &controls);
        let mut in_a = [0.0f32; BLOCK];
        let mut in_b = [0.0f32; BLOCK];
        let mut a_left = [0.0f32; BLOCK];
        let mut a_right = [0.0f32; BLOCK];
        let mut b_left = [0.0f32; BLOCK];
        let mut b_right = [0.0f32; BLOCK];
        let mut worst = 0.0f32;
        let warmup = (RATE / 8.0) as usize / BLOCK;
        for start in 0..((RATE / 2.0) as usize / BLOCK) {
            for i in 0..BLOCK {
                let sample = tone(640.0)(start * BLOCK + i);
                in_a[i] = sample;
                in_b[i] = 0.0;
            }
            // Both instances run the same number of samples, one with the tone on
            // each side: lane A must not be able to tell which.
            left_only.process_stereo(&controls, [&in_a, &in_b], [&mut a_left, &mut a_right]);
            right_only.process_stereo(&controls, [&in_b, &in_a], [&mut b_left, &mut b_right]);
            if start > warmup {
                for i in 0..BLOCK {
                    worst = worst.max((a_left[i] - b_left[i]).abs());
                }
            }
        }
        assert!(worst < 1e-6, "the two cases differ by {worst}");
    }

    #[test]
    fn one_summed_source_that_is_its_own_negative_reaches_no_lane() {
        // The sharp edge of summing, named before anyone meets it by surprise. In
        // `In A` and `Out A` the two input channels are one source, exactly as a
        // Y-cord makes one source of two jacks — and a source whose right side is
        // the exact negative of its left sums to nothing. Both taps are then
        // *silent* rather than coloured, because nothing reached a phasor to be
        // swept. That is what the cable does, so the model does it too; the panel's
        // answer is its third position, which is the second half of this test.
        let probe = tone(700.0);
        for routing in [Routing::InA, Routing::OutA] {
            let controls = Controls {
                b_input: routing.to_param(),
                ..frozen()
            };
            let (left, right) = run_stereo(&controls, 0.5, probe, move |i| -probe(i));
            assert!(
                left.iter().chain(right.iter()).all(|&y| y == 0.0),
                "{routing:?} made something out of a cancelled source"
            );
        }
        // `In B`: each lane has a jack of its own, and the same anti-phase input is
        // sound in both — the two instruments the manual is describing.
        let controls = Controls {
            b_input: Routing::InB.to_param(),
            ..frozen()
        };
        let (left, right) = run_stereo(&controls, 0.5, probe, move |i| -probe(i));
        assert!(
            amplitude(&left[(RATE as usize / 4)..], 700.0) > 0.1,
            "In B lost the left instrument"
        );
        assert!(
            amplitude(&right[(RATE as usize / 4)..], 700.0) > 0.1,
            "In B lost the right instrument"
        );
    }

    #[test]
    fn out_a_hands_phasor_a_to_phasor_b_and_in_a_does_not() {
        // The routing switch, heard. Lane A has a resonant loop — feedback 0.5, a
        // peak of 1/(1−f) = 2 at the predicted frequency — and lane B is flat. So in
        // `Out A` that peak turns up on the right-hand tap, because the right lane is
        // being fed by the left one; in `In A` the right lane hears the input instead
        // and stays at unity.
        let fc = sweep_hz(0.5);
        let peak = peak_hz(fc, 1, SECTIONS, RATE);
        let base = Controls {
            a_feedback: 0.5,
            b_feedback: 0.0,
            ..frozen()
        };
        let series = Controls {
            b_input: Routing::OutA.to_param(),
            ..base
        };
        let parallel = Controls {
            b_input: Routing::InA.to_param(),
            ..base
        };
        let in_series = gain_at_right(&series, peak);
        let in_parallel = gain_at_right(&parallel, peak);
        assert!(
            (in_series - 2.0).abs() < 0.1,
            "Out A must carry A's resonance: {in_series}"
        );
        assert!(
            (in_parallel - 1.0).abs() < 0.02,
            "In A must leave B alone: {in_parallel}"
        );
        // And in a mono insert, `Out A` answers with the far end of the chain: twelve
        // sections means the peak is there too, where the six-stage tap has it as
        // well but the other two positions hand back a flat lane.
        let mono_out = {
            let mut pi_face = PiFace::new(RATE, &series);
            let mut worst = 0.0f32;
            let mut block_in = [0.0f32; BLOCK];
            let mut block_out = [0.0f32; BLOCK];
            for start in 0..((RATE / 2.0) as usize / BLOCK) {
                for (i, sample) in block_in.iter_mut().enumerate() {
                    *sample = tone(peak)(start * BLOCK + i);
                }
                pi_face.process_mono(&series, &block_in, &mut block_out);
                if start > ((RATE / 8.0) as usize / BLOCK) {
                    worst = worst.max(block_out.iter().fold(0.0, |m, &y| m.max(y.abs())));
                }
            }
            worst
        };
        // 0.2 in, twice round a loop of 0.5, and the mono tap is the series one.
        assert!(
            (mono_out - 0.4).abs() < 0.05,
            "mono Out A should hand back the twelve-stage tap: {mono_out}"
        );
    }

    #[test]
    fn in_b_gives_phasor_b_a_input_of_its_own() {
        // Two instruments. With only the left carrying sound, phasor B's jack is
        // empty and its tap is exactly nothing — not coloured, not leaked, empty.
        // And the reverse: sound on the right never reaches phasor A.
        let controls = Controls {
            b_input: Routing::InB.to_param(),
            ..frozen()
        };
        let (left, right) = run_stereo(&controls, 0.5, tone(700.0), |_| 0.0);
        assert!(
            right.iter().all(|&y| y == 0.0),
            "In B fed phasor B from phasor A's input"
        );
        assert!(amplitude(&left[(RATE as usize / 4)..], 700.0) > 0.1);
        let (left, right) = run_stereo(&controls, 0.5, |_| 0.0, tone(700.0));
        assert!(left.iter().all(|&y| y == 0.0), "In B crossed into phasor A");
        assert!(amplitude(&right[(RATE as usize / 4)..], 700.0) > 0.1);
    }

    #[test]
    fn the_three_modes_are_the_three_positions_of_one_switch() {
        // The manual's words: the Phasor B Input switch "effectively selects one of
        // the three basic modes of operation". So the integers here are the panel's
        // own layout — IN A, OUT A, IN B round the rotary — and not a second
        // numbering with a number of its own.
        assert_eq!(Routing::from_param(0), Routing::InA);
        assert_eq!(Routing::from_param(1), Routing::OutA);
        assert_eq!(Routing::from_param(2), Routing::InB);
        assert_eq!(Routing::InA.label(), "In A");
        assert_eq!(Routing::OutA.label(), "Out A");
        assert_eq!(Routing::InB.label(), "In B");
        for routing in [Routing::InA, Routing::OutA, Routing::InB] {
            assert_eq!(Routing::from_param(routing.to_param()), routing);
        }
        // A hand-edited file or a stale preset cannot land the plugin on a routing
        // that does not exist; it lands on the series path, which is the manual's
        // normal setting for one instrument and one amplifier.
        assert_eq!(Routing::from_param(-1), Routing::OutA);
        assert_eq!(Routing::from_param(9), Routing::OutA);
    }

    #[test]
    fn the_sweep_sources_are_the_two_switches_the_panel_prints() {
        assert_eq!(Source::of_a(&Controls::default()), Source::Gen1);
        assert_eq!(
            Source::of_a(&Controls {
                a_sweep: 1,
                ..Default::default()
            }),
            Source::Pedal
        );
        // The starting config the panel describes — B following the second
        // generator. `Controls::default()` is the zero snapshot rather than the
        // plugin's defaults, and its zero is `GEN 1`, so the panel's setting is
        // spelled out here. `PiFaceParams` is what a host activates with, and
        // `lib.rs` checks that its B sweep reads `Gen 2`.
        assert_eq!(
            Source::of_b(&Controls {
                b_sweep: 1,
                ..Default::default()
            }),
            Source::Gen2
        );
        assert_eq!(Source::of_b(&Controls::default()), Source::Gen1);
        assert_eq!(
            Source::of_b(&Controls {
                b_sweep: 0,
                ..Default::default()
            }),
            Source::Gen1
        );
        assert_eq!(
            Source::of_b(&Controls {
                b_sweep: 2,
                ..Default::default()
            }),
            Source::Pedal
        );
        // An out-of-range position leaves a lane on an oscillator rather than off
        // the map.
        assert_eq!(
            Source::of_b(&Controls {
                b_sweep: 7,
                ..Default::default()
            }),
            Source::Gen2
        );
        // The shapes, at the angles that decide them.
        assert_eq!(shaped(0, 0.0), 0.5);
        assert_eq!(shaped(0, 1.0), 1.0);
        assert_eq!(shaped(0, -1.0), 0.0);
        assert_eq!(shaped(1, 0.0), 1.0);
        assert_eq!(shaped(1, -0.001), 0.0);
    }

    #[test]
    fn retuning_leaves_the_sweep_where_the_player_left_it() {
        // The standalone re-activates on every device change. A filter that forgot
        // where its lamps were would thump; one that forgot the rate would keep
        // digital coefficients tuned for a rate it no longer runs at.
        let controls = Controls {
            gen1_rate: 3.0,
            a_depth: 1.0,
            ..Default::default()
        };
        let mut pi_face = PiFace::new(44_100.0, &controls);
        let block = [0.0f32; BLOCK];
        let mut out = [0.0f32; BLOCK];
        for _ in 0..((RATE as usize) / BLOCK) {
            pi_face.process_mono(&controls, &block, &mut out);
        }
        let before = pi_face.sweep_position(0);
        pi_face.set_rate(96_000.0);
        assert!(
            (pi_face.sweep_position(0) - before).abs() < 1e-6,
            "a rate change moved the sweep"
        );
        pi_face.process_mono(&controls, &block, &mut out);
        assert!(out.iter().all(|&y| y.is_finite()));
    }

    #[test]
    fn silence_leaves_no_trace_in_any_routing() {
        for routing in [Routing::InA, Routing::OutA, Routing::InB] {
            let controls = Controls {
                b_input: routing.to_param(),
                gen1_rate: 5.0,
                gen2_rate: 7.0,
                a_feedback: 0.8,
                b_feedback: 0.8,
                a_depth: 0.9,
                b_depth: 0.9,
                ..Default::default()
            };
            let (left, right) = run_stereo(&controls, 1.0, |_| 0.0, |_| 0.0);
            assert!(
                left.iter().chain(right.iter()).all(|&y| y == 0.0),
                "{routing:?} makes something out of nothing"
            );
        }
    }

    #[test]
    fn no_control_combination_grows_or_goes_nan() {
        // The stability argument here is "all-passes are flat, so a loop under unity
        // cannot grow", and the switch matrix is wide enough that walking it beats a
        // proof on paper: every routing, both shapes, both sync positions, both rate
        // sources, all three of Phasor B's sweep sources, feedback at the top of a
        // control that reaches its top, and a pedal on the move.
        let mut worst = 0.0f32;
        for routing in [Routing::InA, Routing::OutA, Routing::InB] {
            for shape in [0, 1] {
                for sync in [0, 1] {
                    for rate_source in [0, 1] {
                        for b_sweep in 0..3 {
                            let mut controls = Controls {
                                gen1_rate: 11.0,
                                gen1_shape: shape,
                                gen1_rate_source: rate_source,
                                gen2_rate: 17.0,
                                gen2_shape: shape,
                                a_depth: 1.0,
                                b_depth: 1.0,
                                a_feedback: 1.0,
                                b_feedback: 1.0,
                                b_sweep,
                                b_sync: sync,
                                b_input: routing.to_param(),
                                ..Default::default()
                            };
                            let mut pi_face = PiFace::new(44_100.0, &controls);
                            for block in 0..64 {
                                controls.pedal = block as f32 / 63.0;
                                let mut input = [0.0f32; BLOCK];
                                for (i, sample) in input.iter_mut().enumerate() {
                                    let t = (block * BLOCK + i) as f32 / 44_100.0;
                                    *sample = 0.9
                                        * ((2.0 * PI * 110.0 * t).sin()
                                            + 0.5 * (2.0 * PI * 830.0 * t).sin());
                                }
                                let mut left = [0.0f32; BLOCK];
                                let mut right = [0.0f32; BLOCK];
                                pi_face.process_stereo(
                                    &controls,
                                    [&input, &input],
                                    [&mut left, &mut right],
                                );
                                for &y in left.iter().chain(right.iter()) {
                                    assert!(y.is_finite(), "{routing:?} produced {y}");
                                    worst = worst.max(y.abs());
                                }
                            }
                        }
                    }
                }
            }
        }
        // Full scale, exactly, and for a reason rather than by measurement: both
        // ends of a lane are capped, so every position of every switch is capped
        // with it. An input already at 1.35 times full scale — the sum of its two
        // tones — arrives as the manual's overdriven case and leaves inside the
        // ceiling the hardware's rails impose.
        assert!(worst <= 1.0, "the switch matrix reached {worst}");
    }
}
