//! The Pi-Fase signal path — **not modelled yet**.
//!
//! This file is a scaffold: [`PiFace`] passes audio through unchanged so the
//! plugin, editor, standalone host and bundling can be built and exercised
//! before a single filter exists. Nothing here is a model of anything, and no
//! number in it should be quoted as one. The repository rule is *measure,
//! never speculate*, and a pass-through at least has the honesty of claiming
//! nothing.
//!
//! What the real DSP has to implement, settled from the owner's manual (the
//! complete six-page scan: control descriptions, three wiring figures with the
//! panel drawn, and the specification table) and the service documents the user
//! supplied (drawings 77204 sheet D and 77206 sheet B, plus the hand-drawn "A
//! PHASE SHIFT"/"B PHASE SHIFT" page). Recorded here so neither has to be
//! re-derived from scans:
//!
//! * **Six first-order all-pass sections per lane, not four.** Lane A is
//!   op-amps 1A,1B,2A,2B,3A,3B, lane B is 4A..6B. Each section: 0.0068 µF
//!   coupling (10%), 47K series, 220K feedback resistor in parallel with an
//!   incandescent lamp (PH3A..PH3F on lane A, PH4A..PH4F on lane B), and a
//!   .47 coupling cap out (C12/C26). The manual corroborates the count: with
//!   the phasors in series "this gives you a 12-stage phase shifter".
//! * **The variable element is a lamp, not a JFET.** The sweep reaches the
//!   lamps through a voltage-to-current converter (Q1/Q2, C1852, R98 220K,
//!   CR16/CR17 1N4148, PM3/PM4). Filament temperature lags the drive current,
//!   which is what makes the sweep smooth and gives the pedal its compression.
//!   Represent that as a first-order lag on the *control* signal with the
//!   corner well below the LFO's top rate — and name it as the documented
//!   approximation it is, rather than tuning it until it sounds right.
//! * **Two sweep generators, one per lane, with a sync switch.** Lane A from
//!   21A/21B (C33 .47, R70/R71/R72 shape network), lane B from 2HA/2HB (C34
//!   .47, R90/R91/R94). S5/S6 are SWEEP, S7 is SWEEP SYNC NORM/REV, S3/S4 are
//!   SHAPE, S2 selects RATE CONTROL MAN/FXD with the pedal at J6-2. The two
//!   generators may drift against each other: that beating is where the wide
//!   sweeps come from, so they must stay two oscillators. The manual's own name
//!   for it is "two-speed phasing", and it is one switch away from the machine's
//!   starting configuration.
//! * **Three basic modes of operation, selected by the Phasor B Input switch
//!   (S1) — not by a Mode control, which the machine does not have.** The
//!   manual says so in as many words: "this switch effectively selects one of
//!   the three basic modes of operation by selecting which of three signals is
//!   applied to the input of Phasor B". In `In A` the one input at Input A feeds
//!   both phasors, "the normal setting for stereo effects"; in `Out A` phasor
//!   A's output feeds phasor B, putting them in series, "the normal setting for
//!   using the Bi-Phase with one instrument and one amplifier"; in `In B` phasor
//!   B gets its own jack, for a second instrument or a stereo one. The routing
//!   is therefore `b_input` and nothing else — see [`Controls::b_input`].
//! * **Two independent input/output paths.** Input A and Input B, output A at
//!   J3 and output B at J4, each through its own 29B/211A stage with PH1/PH2
//!   lamps. Stereo or dual-mono falls out of the circuit rather than being
//!   bolted on.
//! * **Rate range .1 Hz to 18 Hz** — "10 seconds per sweep cycle" at the
//!   bottom. Input impedance 180K ohms in parallel (two-instrument) mode and
//!   390K in single mode, minimum; output 600 Ω unbalanced; insertion loss
//!   under 1 dB; bypass response 20 Hz–20 kHz ± 1 dB; signal-to-noise typically
//!   86 dB below a 2 V input. Headroom: one phasor no feedback 4 V RMS, both in
//!   series 50% feedback 2 V, 100% feedback 1 V.
//! * **Behaviours with words attached, which the model has to reproduce.**
//!   Square-wave sweep "jumps the phasing from one extreme of the sweep to the
//!   other", and at moderate rates with feedback up "resembles an echo repeat";
//!   at the top of the rate range, "an interesting pseudo-ring modulation effect
//!   results, particularly if the Feedback controls are turned up".
//!   Rate Control on Ped with no pedal plugged in leaves the generator at "a
//!   very low rate". Phasor A's Sweep on Ped with a pedal that is not moving
//!   "changes the timbres (or tone) of the sound but does not sweep back and
//!   forth" — a held pedal is a static filter, not a slow one. Sweep Sync does
//!   nothing unless both phasors are swept by the same source, and in Rev there
//!   is no double-depth phasing because the lanes move against each other.
//!
//! Not to be confused with the "BI-PHASER" sheet in the same pile: 4558
//! op-amps, BF245A JFETs, 47nF/10k, four stages, dual 9–18 V. That is a JFET
//! clone of the idea, useful as a simpler reference and wrong as a source of
//! truth. The README says so again, where a reader will meet it first.
//!
//! Everything below is fixed-size and allocation-free, as the real filter will
//! have to be under nice-plug's `assert_process_allocs`.

/// The span the coefficients will be recomputed over. A host block is chopped
/// into these, so the control-refresh resolution never depends on how large a
/// block the host happens to hand us.
pub const BLOCK: usize = 64;

/// The bottom of the sweep generators' documented range, hertz. The manual
/// gives both ends in one sentence: "Each control smoothly and continuously
/// varies the sweep rate from .1Hz (10 seconds per sweep cycle) to 18Hz."
pub const RATE_MIN_HZ: f32 = 0.1;
/// The top of it. The lamps' thermal lag is to stay well below this, which is
/// why the two numbers belong next to each other.
pub const RATE_MAX_HZ: f32 = 18.0;

/// The three positions of the Phasor B Input switch (S1), which *are* the
/// machine's three basic modes of operation.
///
/// Nothing branches on these yet because there is no filter to branch in — the
/// same reason the whole module is a pass-through — so the lint is told about
/// it here rather than by deleting the names the tests and the eventual routing
/// code both speak about.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Routing {
    /// Input A feeds both phasors, as if by a Y-cord: the manual's normal
    /// setting for stereo effects, and the setting its stereo figure draws.
    InA,
    /// Phasor A's output feeds phasor B: twelve stages in series, the manual's
    /// normal setting for one instrument and one amplifier.
    OutA,
    /// Phasor B has its own input, for a second instrument or a stereo one.
    InB,
}

#[allow(dead_code)]
impl Routing {
    /// The routing `b_input` selects. The panel prints the positions in this
    /// order — `IN A`, `OUT A`, `IN B` — so the integers match the hardware's
    /// own layout, and anything out of range reads as `OutA`: the series path
    /// the manual calls normal for the common case, rather than a routing no
    /// one asked for.
    pub fn from_param(value: i32) -> Routing {
        match value {
            0 => Routing::InA,
            2 => Routing::InB,
            _ => Routing::OutA,
        }
    }

    /// The parameter value for this routing.
    pub fn to_param(self) -> i32 {
        match self {
            Routing::InA => 0,
            Routing::OutA => 1,
            Routing::InB => 2,
        }
    }

    /// What the panel prints.
    pub fn label(self) -> &'static str {
        match self {
            Routing::InA => "In A",
            Routing::OutA => "Out A",
            Routing::InB => "In B",
        }
    }
}

/// One block's worth of controls, taken at the top of the block so the sample
/// loop touches no atomics. Fields carry the parameter ids they come from;
/// they are read by the DSP that has not landed yet, which is why the derives
/// below are load-bearing for the linter as well as for the tests.
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
    /// Phasor A's depth, 0..=1.
    pub a_depth: f32,
    /// Phasor A's feedback, 0..=1. The headroom notes above say what the
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
    /// What drives phasor B, and so which of the machine's three basic modes
    /// of operation it is in: 0 `In A` (one input feeding both phasors — the
    /// stereo setting), 1 `Out A` (A into B, twelve stages in series — the one
    /// instrument, one amplifier setting), 2 `In B` (B has its own input — two
    /// instruments, or a stereo one). [`Routing`] names these. The order is the
    /// panel's own, printed clockwise round the rotary switch as `IN A`, `OUT
    /// A`, `IN B`.
    ///
    /// One thing the hardware never had to answer, and the plugin must: what
    /// `In A` means for a *stereo* plugin input — each lane taking its own
    /// side, or the summed input feeding both lanes. The machine's "single
    /// input" is one jack; a plugin's is two. Pick one in the DSP phase and
    /// write the choice down here.
    pub b_input: i32,
    /// The expression pedal, 0..=1 — the plugin's port of the pedal jack.
    pub pedal: f32,
}

/// The placeholder signal path: two lanes of nothing, at the rate the host
/// gave us. The name is kept honest by the module documentation — this is the
/// chassis the filter goes into, not the filter.
pub struct PiFace {
    sample_rate: f32,
}

impl PiFace {
    pub fn new(sample_rate: f32, _controls: &Controls) -> Self {
        Self { sample_rate }
    }

    /// Re-tune for a new device or host rate. The real filter will rebuild its
    /// coefficients here; this has none, and keeps the rate only so that the
    /// retune is a set_rate away from being real.
    pub fn set_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    pub fn reset(&mut self) {}

    /// Pass-through. `controls` is accepted because the real filter reads it
    /// every block; the placeholder reads none of it, and says so.
    pub fn process_mono(&mut self, controls: &Controls, input: &[f32], output: &mut [f32]) {
        self.passthrough(controls, input, output);
    }

    /// Pass-through, per channel. The real version runs lane A on the first
    /// channel and lane B on the second, with phasor B's input taken from
    /// `b_input`.
    pub fn process_stereo(
        &mut self,
        controls: &Controls,
        input: [&[f32]; 2],
        output: [&mut [f32]; 2],
    ) {
        self.passthrough(controls, input[0], output[0]);
        self.passthrough(controls, input[1], output[1]);
    }

    fn passthrough(&mut self, controls: &Controls, input: &[f32], output: &mut [f32]) {
        let _ = (controls.b_input, self.sample_rate);
        let n = input.len().min(output.len());
        output[..n].copy_from_slice(&input[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_modes_are_the_three_positions_of_one_switch() {
        // The manual's words: the Phasor B Input switch "effectively selects
        // one of the three basic modes of operation". So the integers here are
        // the panel's own layout — IN A, OUT A, IN B round the rotary — and not
        // a second numbering with a number of its own.
        assert_eq!(Routing::from_param(0), Routing::InA);
        assert_eq!(Routing::from_param(1), Routing::OutA);
        assert_eq!(Routing::from_param(2), Routing::InB);
        assert_eq!(Routing::InA.label(), "In A");
        assert_eq!(Routing::OutA.label(), "Out A");
        assert_eq!(Routing::InB.label(), "In B");
        for routing in [Routing::InA, Routing::OutA, Routing::InB] {
            assert_eq!(Routing::from_param(routing.to_param()), routing);
        }
        // A hand-edited file or a stale preset cannot land the plugin on a
        // routing that does not exist; it lands on the series path, which is the
        // manual's normal setting for one instrument and one amplifier.
        assert_eq!(Routing::from_param(-1), Routing::OutA);
        assert_eq!(Routing::from_param(9), Routing::OutA);
    }

    #[test]
    fn the_placeholder_leaves_the_signal_alone() {
        let controls = Controls::default();
        let mut pi_face = PiFace::new(48_000.0, &controls);
        let input: Vec<f32> = (0..BLOCK).map(|i| (i as f32 / 64.0) - 0.5).collect();
        let mut mono = [0.0f32; BLOCK];
        pi_face.process_mono(&controls, &input, &mut mono);
        assert_eq!(&mono[..], &input[..]);

        let mut left = [0.0f32; BLOCK];
        let mut right = [0.0f32; BLOCK];
        pi_face.process_stereo(
            &controls,
            [&input[..], &input[..]],
            [&mut left[..], &mut right[..]],
        );
        assert_eq!(&left[..], &input[..]);
        assert_eq!(&right[..], &input[..]);
    }

    /// Retuning is the one thing the placeholder must get right, because the
    /// standalone re-activates on every device change and a filter that forgot
    /// the rate would silently keep old coefficients.
    #[test]
    fn retuning_leaves_the_signal_alone_too() {
        let controls = Controls::default();
        let mut pi_face = PiFace::new(44_100.0, &controls);
        pi_face.set_rate(96_000.0);
        pi_face.reset();
        let input = [0.125f32; BLOCK];
        let mut output = [0.0f32; BLOCK];
        pi_face.process_mono(&controls, &input, &mut output);
        assert_eq!(&output[..], &input[..]);
    }

    #[test]
    fn the_sweep_generators_range_is_the_documented_one() {
        // .1 Hz to 18 Hz is the sentence on page 1 of the owner's manual, not a
        // listening session; if a later measurement contradicts it, this test is
        // where the argument has to be made.
        assert_eq!(RATE_MIN_HZ, 0.1);
        assert_eq!(RATE_MAX_HZ, 18.0);
    }
}
