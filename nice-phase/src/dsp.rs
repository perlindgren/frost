//! The allpass chain, the sine that walks it, the feedback wire, and the
//! five mode tables.
//!
//! The four-stage phaser is a standing waveform as surely as a wah is: identical
//! first-order allpass sections — each a source-follower stage with its
//! own cutoff — cascaded, and the output summed with the dry signal. An
//! allpass moves phase and leaves magnitude alone, so nothing is ever
//! cut; cancellation happens only where the phase-shifted copy rejoins
//! the original, and a notch appears exactly where the stages have
//! together turned half a cycle around. Four stages are the famous
//! circuit's answer, and their arithmetic is exact: one section's phase
//! runs from 0° at DC to −180° at Nyquist, passing −90° at its cutoff
//! frequency (see [`allpass_a`]), so the chain's runs from 0° to −720°
//! and crosses −360° and −540° once each — four stages, two notches.
//! That is not a compromise of the model; it is what the model *is*, and
//! it is what the hardware measures.
//!
//! Where the notches land follows from the same sentence. A single
//! section's phase is −2·atan(g/t) at the warped frequency g = tan(π f
//! /fs), with t = tan(π f_c /fs): the chain of `n` equal sections
//! cancels where `n`·2·atan(g/t) is an odd multiple of π, which makes
//! the notches exactly tan(π/(2n), 3π/(2n), …) times `t` — for four
//! stages, tan 22.5° ≈ 0.414 and tan 67.5° ≈ 2.414, on the warped scale
//! that keeps the statement true at every sample rate. Two stages — the
//! '45 setting, two of the four sections bypassed — have one notch,
//! exactly at the stage frequency, and that single hole sailing through
//! the mids is what "thinner and more liquid" is made of.
//!
//! The modes are data: the stage tap, the feedback ratio and the trim
//! for each of the five settings the family is known by. The research
//! that fixed this table was humbling and is recorded, URL by URL, in
//! `DECISIONS.md`: every era of the famous circuit is the same
//! four-stage JFET chain over the same sweep — no published analysis
//! gives any of them a different range, and none ever printed a sweep
//! table — so the settings differ only in what the analyses actually
//! name. Two things: how many of the four sections the output is taken
//! after, and whether a resistor feeds the summer's output back into
//! the phase network. That resistor is the whole script-versus-block
//! story — the block-era boards added it and got the resonant "whoosh";
//! the script-era boards run without it and stay mellower, with more
//! headroom. Feedback cannot move a cancellation: where the chain's
//! phase says odd multiples of −180°, the output still sums toward
//! silence — but it partly fills those holes and blooms the band around
//! them, which is exactly what a whoosh is made of. The sweep's ends
//! are the published consensus of the static analyses — a labelled
//! guess, since no box ever stated one. The switch picks a setting;
//! every dependent number slews to it, so a flip is a walk, never a
//! click. Everything here is fixed-size and allocation-free so the
//! audio thread stays clean under `nice-plug`'s `assert_process_allocs`.

/// The span the filter coefficients are recomputed over. A host block is
/// chopped into these, so the coefficient refresh rate — and with it the
/// resolution of both automation and the sweep — never depends on how
/// large a block the host happens to hand us.
pub const BLOCK: usize = 64;

/// The number of allpass sections the chain always runs: four, because
/// the flagship circuit had four. A two-stage mode reads its output
/// after two of them; the other two keep running regardless, so switching
/// the tap never meets stale memory.
pub const STAGES: usize = 4;

/// How fast a mode switch walks the sweep, the stage tap and the trim to
/// their new places, in seconds. The same 20 ms the wah gives its
/// treadle: below the speed of a hand on a toggle, far above one audio
/// block, so the toggle never prints a click through the changes it
/// causes.
const SLEW: f32 = 0.020;

/// How fast the mixer follows its target, in seconds, and how fast the
/// trim does — the same constant the wah spends on both. Quicker than
/// the sweep: a level that lags the dry signal by tens of milliseconds
/// is its own audible artifact.
const GAIN_SLEW: f32 = 0.010;

/// One mode's fingerprint: everything but the speed, depth and mix that
/// says *which* phaser this is. A plain value, published to the editor
/// so the selector's buttons — and a hand-edit of the settings file, and
/// a host preset — all mean the same handful of numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode {
    pub name: &'static str,
    /// The stage frequency at the sweep's bottom.
    pub lo_hz: f32,
    /// ...and its top.
    pub hi_hz: f32,
    /// How many of the four sections the output is summed after: four
    /// for the 90 settings, two for the 45's — "bypassing two of the
    /// phase stages" is the switch's own description, and the tap is
    /// this plugin's way of saying it.
    pub stages: usize,
    /// The feedback resistor's ratio: how much of the network's wet
    /// output, one sample old, is re-injected into its input. The
    /// script-era boards run none (`0.0`); the block-era added a
    /// resistor from the summer back into the phase network, and it —
    /// not any sweep difference, which no analysis has ever found — is
    /// what makes a block whoosh: the feedback cannot move the
    /// cancellations, only partly fill them and bloom the band around
    /// them. The nonzero number in the table is a taste of that
    /// resistor's ratio, not a measurement, and `TODO.md` keeps the
    /// debt. One sample of delay at a ratio under 1 makes the loop
    /// stable by the same argument as [`allpass_a`]'s coefficient: the
    /// allpasses hand back what they are given, so the loop can never
    /// grow.
    pub feedback: f32,
    /// The mode's own output trim. The summer halves the signal the way
    /// the hardware's does (see the mixer in [`Phase::process_mono`]), so
    /// this is what it owes back, plus whatever the mode can show a
    /// source for — which, per `TODO.md`, is none of them yet.
    pub trim_db: f32,
}

/// The modes, in selector order. The sweep ends are the published
/// consensus of the static circuit analyses, shared by every setting
/// because no analysis gives any era a different range — and they are
/// still a labelled guess, because no box ever printed one. A mode's
/// character is its tap, its feedback, and nothing else the plugin
/// pretends to know.
pub const MODES: [Mode; 5] = [
    Mode {
        name: "SCRIPT 90",
        lo_hz: 100.0,
        hi_hz: 1_600.0,
        stages: 4,
        feedback: 0.0,
        trim_db: 0.0,
    },
    Mode {
        name: "SCRIPT 45",
        lo_hz: 100.0,
        hi_hz: 1_600.0,
        stages: 2,
        feedback: 0.0,
        trim_db: 0.0,
    },
    Mode {
        name: "BLOCK 90",
        lo_hz: 100.0,
        hi_hz: 1_600.0,
        stages: 4,
        feedback: 0.25,
        trim_db: 0.0,
    },
    Mode {
        name: "BLOCK 45",
        lo_hz: 100.0,
        hi_hz: 1_600.0,
        stages: 2,
        feedback: 0.25,
        trim_db: 0.0,
    },
    // The signature model's party trick is exactly these two switches —
    // feedback in or out, four stages or two — in one box, which makes
    // it the family's superset and, as a table row, one of its corners.
    // This row takes the side the player it is named for is known for:
    // four stages, no feedback. That is a reading of the record, not a
    // specification, and `DECISIONS.md` says so out loud.
    Mode {
        name: "SIGNATURE 90",
        lo_hz: 100.0,
        hi_hz: 1_600.0,
        stages: 4,
        feedback: 0.0,
        trim_db: 0.0,
    },
];

/// How many notches a stage count puts inside one sweep of the band:
/// the chain's phase covers `stages`·180°, and an odd multiple of 180°
/// cancels. Four sections: two notches. Two sections: one.
pub const fn notch_count(stages: usize) -> usize {
    stages.div_ceil(2)
}

/// The selector value's table position: the nearest mode by index. A
/// drag through the middle of the range steps through the settings, and
/// the slews inside [`Phase`] walk every dependent number to each one —
/// which is what a selector, as opposed to a morph pedal, should sound
/// like at its detents.
pub fn mode_index(value: f32) -> usize {
    let last = (MODES.len() - 1) as f32;
    value.clamp(0.0, last).round() as usize
}

/// The mode a selector value names: rounded, clamped, and total — a
/// hand-typed 99 is the last mode, exactly as the parameter clamps it.
pub fn mode_of(value: f32) -> Mode {
    MODES[mode_index(value)]
}

/// The stage frequency at a normalized LFO position, on the logarithmic
/// scale: the sine's own output is linear, and equal swings of it move
/// equal *ratios*, which is what makes the sweep sound like the analog
/// one. The endpoints are guarded the way the wah guards its sweep,
/// because a hand-edited settings file is a user error, not a license to
/// take the logarithm of zero.
pub fn sweep_hz(t: f32, lo_hz: f32, hi_hz: f32) -> f32 {
    let lo = lo_hz.clamp(10.0, 20_000.0);
    let hi = hi_hz.clamp(lo, 22_000.0);
    let t = t.clamp(0.0, 1.0);
    (lo.ln() + (hi / lo).ln() * t).exp()
}

/// The allpass coefficient for a section with its −90° crossing at `fc`.
/// With `g = tan(π f / fs)` as the warped frequency coordinate, the
/// section `H = (a + z⁻¹) / (1 + a z⁻¹)` has phase −2·atan(g / t), where
/// `t = tan(π fc / fs)`: 0° at DC, exactly −90° at `f = fc`, −180° at
/// Nyquist — the digital mirror of the analog allpass the hardware
/// built, with the sign the summing amp wants. And `t > 0` puts `a`
/// strictly inside the unit circle for every finite `t`: `a → −1` only
/// as `fc → 0` and `a → +1` only at Nyquist, so every section here is
/// stable, always, for any frequency the parameters can name.
fn allpass_a(fc: f32, rate: f32) -> f32 {
    let t = (std::f32::consts::PI * fc / rate).tan();
    (t - 1.0) / (t + 1.0)
}

/// A cancellation frequency from the stage frequency and the stage
/// count: section phase −2·atan(g/t), chain −2·n·atan(g/t), cancellation
/// where that is an odd multiple of π — so section `notch` sits at
/// ψ = (2·notch + 1)·π / (2n) and `g = t·tan ψ`, warped back to hertz.
/// Below Nyquist the warp is what keeps the ratios honest at 8 kHz
/// sample rates as well as 192.
pub fn notch_hz(fc: f32, notch: usize, stages: usize, rate: f32) -> f32 {
    debug_assert!(notch < notch_count(stages));
    let psi = (2 * notch + 1) as f32 * std::f32::consts::PI / (2 * stages) as f32;
    let t = (std::f32::consts::PI * fc / rate).tan();
    (t * psi.tan()).atan() * rate / std::f32::consts::PI
}

/// One parameter set as the audio thread sees it: plain values, read
/// fresh from the [`nice_plug::prelude::Params`] every block. A struct,
/// so the filter asks for its whole control state at the top of each
/// block and nothing inside the sample loop touches an atomic.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    /// The sweep rate in hertz, as the speed knob is labeled.
    pub speed: f32,
    /// The mode selector as a float, 0 .. 4, read by [`mode_of`].
    pub mode: f32,
    /// How deep the notches cut, 0 dry .. 1 the summer's own half.
    pub depth: f32,
    /// The dry/wet blend after the phaser, 0 dry .. 1 wet.
    pub mix: f32,
    /// The output trim in decibels, on top of the mode's own.
    pub gain_db: f32,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            speed: 0.5,
            mode: 0.0,
            depth: 1.0,
            mix: 1.0,
            gain_db: 0.0,
        }
    }
}

/// One allpass's memory: the input and output a sample back, and per
/// section that is the whole of it — which is why four of them were
/// cheap and the hardware could afford them.
#[derive(Clone, Copy, Default)]
struct Allpass {
    x1: f32,
    y1: f32,
}

impl Allpass {
    /// `y[n] = a·x[n] + x[n−1] − a·y[n−1]`, two multiply-accumulates of
    /// actual work. The coefficient arrives every sample because it is
    /// being crossfaded, not because the section wants advice.
    fn process(&mut self, a: f32, x: f32) -> f32 {
        let y = a * x + self.x1 - a * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// The phaser itself: four allpasses per channel, one shared sine, and
/// the smoothed state that carries between blocks.
pub struct Phase {
    rate: f32,
    /// The LFO's phase in radians, wrapped to `0..2π` every block. Phase,
    /// not time: a speed change continues the sweep from where it stands
    /// instead of snapping the sine to a new clock, which is the whole
    /// difference between a tempo change and a thump.
    phase: f32,
    /// The last block's sweep position, 0..1, and the stage frequency it
    /// ran at — the editor's needle and readout.
    lfo: f32,
    center: f32,
    /// The sweep's ends, held in logarithms and mid-slew: a mode flip
    /// walks the range to its new place rather than jumping it, and on
    /// the log scale a ratio slews as musically as a frequency does.
    ln_lo: f32,
    ln_hi: f32,
    /// The stage tap, held as a continuous count — 2.0 at a two-stage
    /// setting, 4.0 at a four-stage one, and in between, mid-flip, the
    /// output is the two tap points crossfaded. Every section always
    /// runs, so the tap is the only thing a mode switch moves, and this
    /// is what keeps that walk silent.
    stages: f32,
    /// The mixer and trim states, likewise mid-slew.
    depth: f32,
    mix: f32,
    gain_db: f32,
    /// The feedback ratio mid-slew — a mode flip wires the resistor in
    /// or out over the same 20 ms the tap walks — and the one-sample
    /// memory the loop closes through: the network's wet output as of
    /// the previous sample, per channel.
    feedback: f32,
    fb_bus: [f32; 2],
    /// The allpass coefficient as the last block left it — the blend's
    /// start. One number for all four sections, because the hardware's
    /// ganged capacitor kept their cutoffs equal, and the notch
    /// arithmetic needs them that way.
    previous: f32,
    /// The linear gain the last block ended on, blended to likewise.
    previous_gain: f32,
    stages_mem: [[Allpass; STAGES]; 2],
}

impl Phase {
    /// A phaser ready for `rate`, parked at the controls it is handed so
    /// that activation — or a sample-rate change — begins where the
    /// last block left the sweep, not with a slew from wherever the
    /// defaults sat.
    pub fn new(rate: f32, controls: &Controls) -> Self {
        let mode = mode_of(controls.mode);
        let mut phase = Self {
            rate: rate.max(1.0),
            phase: 0.0,
            lfo: 0.5,
            center: 0.0,
            ln_lo: mode.lo_hz.max(10.0).ln(),
            ln_hi: mode.hi_hz.max(10.0).ln(),
            stages: mode.stages as f32,
            depth: controls.depth,
            mix: controls.mix,
            gain_db: controls.gain_db + mode.trim_db,
            feedback: mode.feedback,
            fb_bus: [0.0; 2],
            previous: 0.0,
            previous_gain: 1.0,
            stages_mem: [[Allpass::default(); STAGES]; 2],
        };
        // Which coefficient, and at what gain, is `set_rate`'s question.
        phase.set_rate(rate);
        phase
    }

    /// Re-derive every sample-rate-dependent constant. The coefficient is
    /// recomputed from where the sweep and the slews already stand, so a
    /// rate change that happens live crossfades through the blend like
    /// any other recalculation.
    pub fn set_rate(&mut self, rate: f32) {
        self.rate = rate.max(1.0);
        let fc = self.center_at(self.lfo);
        self.center = fc;
        self.previous = allpass_a(fc, self.rate);
        self.previous_gain = db_to_gain(self.gain_db);
    }

    /// Silence's memory: the stored samples, but not the sweep — a reset
    /// should still keep walking from where the sine stands rather than
    /// restart it under the player's feet. The feedback bus empties with
    /// the rest: whatever the ring was mid-flight, the reset says it
    /// never was.
    pub fn reset(&mut self) {
        self.stages_mem = [[Allpass::default(); STAGES]; 2];
        self.fb_bus = [0.0; 2];
    }

    /// Where the sine stood for the last block, 0..1 — the editor's
    /// needle, the same number the sweep ran on.
    pub fn lfo_position(&self) -> f32 {
        self.lfo
    }

    /// The stage frequency the last block ran at. The editor draws the
    /// notches from this, because this is the thread that knows the
    /// sample rate.
    pub fn center_hz(&self) -> f32 {
        self.center
    }

    /// Where the `index`th notch sits at the sweep's current standing:
    /// the exact [`notch_hz`] at whole stage counts, and mid-mode-flip —
    /// when the tap crossfade leaves a fractional count — the same
    /// arithmetic on the fraction, which tracks the hole's travel as the
    /// crossfade walks it. A notch that the chain no longer reaches
    /// answers `0.0`, and the editor skips its tick: through a flip into
    /// a two-stage setting the second notch genuinely leaves through the
    /// roof rather than blinking out.
    pub fn notch_hz(&self, index: usize) -> f32 {
        let whole = self.stages.round();
        if (self.stages - whole).abs() < 1e-3 {
            notch_hz(self.center, index, whole as usize, self.rate)
        } else {
            let psi = (2.0 * index as f32 + 1.0) * std::f32::consts::PI / (2.0 * self.stages);
            if psi >= std::f32::consts::FRAC_PI_2 {
                0.0
            } else {
                let t = (std::f32::consts::PI * self.center / self.rate).tan();
                (t * psi.tan()).atan() * self.rate / std::f32::consts::PI
            }
        }
    }

    /// Run one block of one channel — up to [`BLOCK`] samples, in and out
    /// separate buffers; the caller copies through if the host asked for
    /// in-place. With a second channel, prefer [`Phase::process_stereo`].
    pub fn process_mono(&mut self, controls: &Controls, input: &[f32], output: &mut [f32]) {
        let n = input.len().min(output.len()).min(BLOCK);
        let target = self.begin_block(controls, n);
        let previous = self.previous;
        let gain_target = db_to_gain(self.gain_db);
        let gain_from = self.previous_gain;
        let half_depth = self.depth * 0.5;
        let wet = self.mix;
        let dry = 1.0 - self.mix;
        let feedback = self.feedback;
        let mut fb_bus = self.fb_bus[0];
        for (i, out) in output[..n].iter_mut().enumerate() {
            let u = (i as f32 + 1.0) / n as f32;
            let a = previous + (target - previous) * u;
            let x = input[i];
            // The feedback wire, in the one place it can sit without
            // touching either invariant below: the network's wet output,
            // a sample old, joins the input. The block-era resistor
            // closes this loop inside the wet path; a `0.0` leaves the
            // arithmetic bit-identical to the open chain.
            let mut w = x + feedback * fb_bus;
            for stage in self.stages_mem[0].iter_mut() {
                w = stage.process(a, w);
            }
            let tap = self.tapped(0);
            fb_bus = tap;
            // The hardware's summer: dry and chain at equal weight, a
            // half each. Depth is that weighting's travel from
            // dry-only — `d = 0` is silence's share for the wet side —
            // so the notch bottoms land exactly on the circuit's own
            // cancellation at `d = 1`, and `d = 0` is the dry signal
            // bit for bit.
            let ph = x * (1.0 - half_depth) + tap * half_depth;
            *out = (x * dry + ph * wet) * (gain_from + (gain_target - gain_from) * u);
        }
        self.fb_bus[0] = fb_bus;
        self.end_block(target, gain_target);
    }

    /// Run one block of two channels. Both run the same sweep through
    /// their own allpass memories, because the hardware had one signal
    /// path and one hand on the speed knob: the effect is mono, the
    /// wiring is polite. (Same-tau stereo phasing — two sweeps a hair
    /// apart — is a real trick and a different circuit; see this crate's
    /// TODO.)
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
        let half_depth = self.depth * 0.5;
        let wet = self.mix;
        let dry = 1.0 - self.mix;
        let feedback = self.feedback;
        for i in 0..n {
            let u = (i as f32 + 1.0) / n as f32;
            let a = previous + (target - previous) * u;
            let gain = gain_from + (gain_target - gain_from) * u;
            for channel in 0..2 {
                let x = inputs[channel][i];
                // Each channel closes its own feedback loop — one hand on
                // the speed knob, two independent summers.
                let mut w = x + feedback * self.fb_bus[channel];
                for stage in self.stages_mem[channel].iter_mut() {
                    w = stage.process(a, w);
                }
                let tap = self.tapped(channel);
                self.fb_bus[channel] = tap;
                let ph = x * (1.0 - half_depth) + tap * half_depth;
                outputs[channel][i] = (x * dry + ph * wet) * gain;
            }
        }
        self.end_block(target, gain_target);
    }

    /// The wet signal at the continuous stage tap: every section has
    /// already run this sample, and a section's `y1` *is* its output for
    /// the sample just processed — so the tap is pure arithmetic over
    /// the four results. `stages` 2.0 reads after the second section,
    /// 4.0 after the fourth, and 3.0 mid-flip is the honest crossfade
    /// between neighbours, which is what keeps a selector that changes
    /// the tap silent.
    fn tapped(&self, channel: usize) -> f32 {
        let stages = self.stages;
        let below = (stages as usize).clamp(1, STAGES);
        let frac = stages - below as f32;
        // `y1` of section k is that section's output for the sample just
        // processed.
        let at = |k: usize| self.stages_mem[channel][k - 1].y1;
        if frac <= 0.0 || below >= STAGES {
            at(below)
        } else {
            at(below) * (1.0 - frac) + at(below + 1) * frac
        }
    }

    /// The block's shared advance — the slews, the held controls for the
    /// rate handler, the sine's walk, and the coefficient the blend will
    /// arrive at. The sweep reads ahead by a block rather than per
    /// sample; sixty-four samples at any rate a DAW runs is below the
    /// grain of a sweep nobody could mistake for fast anyway.
    fn begin_block(&mut self, controls: &Controls, n: usize) -> f32 {
        let mode = mode_of(controls.mode);
        let n = (n.max(1)) as f32;
        let slew = 1.0 - (-n / (self.rate.max(1.0) * SLEW)).exp();
        self.ln_lo += (mode.lo_hz.max(10.0).ln() - self.ln_lo) * slew;
        self.ln_hi += (mode.hi_hz.max(10.0).ln() - self.ln_hi) * slew;
        self.stages += (mode.stages as f32 - self.stages) * slew;
        // The feedback resistor is wired in and out on the same walk as
        // the tap: a script-to-block flip rings up over the 20 ms, which
        // is exactly the sound of a hand finding the toggle mid-song.
        self.feedback += (mode.feedback - self.feedback) * slew;
        let gain_slew = 1.0 - (-n / (self.rate.max(1.0) * GAIN_SLEW)).exp();
        self.depth += (controls.depth - self.depth) * gain_slew;
        self.mix += (controls.mix - self.mix) * gain_slew;
        self.gain_db += (controls.gain_db + mode.trim_db - self.gain_db) * gain_slew;
        // The sine walks. `speed = 0` is the frozen sweep the tests lean
        // on and the one thing the parameter's floor — 0.05 Hz, so the
        // effect is never mistaken for stopped — politely refuses.
        let advance = 2.0 * std::f32::consts::PI * controls.speed.max(0.0) * n / self.rate;
        self.phase = (self.phase + advance) % (2.0 * std::f32::consts::PI);
        let t = (1.0 + self.phase.sin()) * 0.5;
        self.lfo = t;
        self.center = self.center_at(t);
        allpass_a(self.center, self.rate)
    }

    fn end_block(&mut self, target: f32, gain: f32) {
        self.previous = target;
        self.previous_gain = gain;
    }

    /// The stage frequency for a sweep position, through the slews: the
    /// geometric walk between the mode's ends, mid-slew included.
    fn center_at(&self, t: f32) -> f32 {
        let lo = self.ln_lo.exp().clamp(10.0, 20_000.0);
        let hi = self.ln_hi.exp().clamp(lo, 22_000.0);
        sweep_hz(t, lo, hi)
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

    /// The board as it ships: the flagship setting, a frozen sweep for
    /// the tone probes, the summer's own depth and mix, no trim of the
    /// player's.
    fn stock() -> Controls {
        Controls {
            speed: 0.0,
            mode: 0.0,
            depth: 1.0,
            mix: 1.0,
            gain_db: 0.0,
        }
    }

    /// A four-stage mode and a two-stage one, by construction rather
    /// than by index, so the mechanism tests survive whatever the
    /// selector's order and names end up being.
    fn four_stage() -> Mode {
        MODES
            .into_iter()
            .find(|m| m.stages == 4)
            .expect("a 90 setting")
    }

    fn two_stage() -> Mode {
        MODES
            .into_iter()
            .find(|m| m.stages == 2)
            .expect("a 45 setting")
    }

    /// A two-stage setting that also carries the feedback wire, so the
    /// flip test moves the tap and the resistor together — which is what
    /// a real toggle between two named settings does, and the setting
    /// contract's click-free promise owes that case too.
    fn block_two() -> Mode {
        MODES
            .into_iter()
            .find(|m| m.stages == 2 && m.feedback > 0.0)
            .expect("a 45 setting with feedback")
    }

    /// The mode parameter value a mode sits at, by its table position.
    fn index_of(wanted: Mode) -> f32 {
        MODES
            .iter()
            .position(|m| *m == wanted)
            .expect("in the table") as f32
    }

    /// Run the phaser over `seconds` of a signal, returning the whole
    /// output — the tests settle by discarding a prefix, the way a
    /// listener does.
    fn run(controls: &Controls, seconds: f32, mut signal: impl FnMut(usize) -> f32) -> Vec<f32> {
        let mut phaser = Phase::new(RATE, controls);
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
            phaser.process_mono(controls, &block_in[..n], &mut block_out[..n]);
            out[start..start + n].copy_from_slice(&block_out[..n]);
            start += n;
        }
        out
    }

    /// The amplitude of `probe_hz` in a window of samples, one bin of a
    /// DFT. The whole test suite asks one question of the phaser — how
    /// loud does a pure tone come out — and this is that question made
    /// exact.
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

    /// The frozen-sweep gain at a frequency: with `speed = 0` there is
    /// nothing to settle except the slews and the coefficient blend, and
    /// a half second is a dozen lifetimes of both.
    fn gain_at(controls: &Controls, probe_hz: f32) -> f32 {
        let signal = run(controls, 0.5, |i| {
            (2.0 * std::f32::consts::PI * probe_hz * i as f32 / RATE).sin()
        });
        amplitude(&signal[(RATE as usize / 4)..], probe_hz)
    }

    #[test]
    fn the_sweep_is_exponential_between_its_ends() {
        let (lo, hi) = (100.0, 6_400.0);
        assert!((sweep_hz(0.0, lo, hi) - lo).abs() < 1e-3);
        assert!((sweep_hz(1.0, lo, hi) - hi).abs() < 1e-2);
        let middle = sweep_hz(0.5, lo, hi);
        // The geometric mean, because the scale is the ratio, not the step.
        assert!((middle - (lo * hi).sqrt()).abs() < 0.1);
        // Off the ends the sine simply stops: automation beyond the
        // parameter range moves nothing.
        assert!((sweep_hz(-2.0, lo, hi) - lo).abs() < 1e-3);
        assert!((sweep_hz(9.0, lo, hi) - hi).abs() < 1e-2);
        // And the guard rail: a bottom above the top parks at the bottom.
        assert!((sweep_hz(0.62, 2_000.0, 500.0) - 2_000.0).abs() < 1e-3);
    }

    #[test]
    fn the_coefficient_always_lives_in_the_unit_circle() {
        // The chain's entire stability argument in one line of arithmetic:
        // for every frequency any mode can name, at every sample rate a
        // host may activate at, the allpass coefficient stays strictly
        // inside the unit circle — and so does every blend of two of
        // them, which is the crossfade's whole worry.
        for &rate in &[8_000.0, 22_057.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            for mode in MODES {
                for i in 0..=64 {
                    let t = i as f32 / 64.0;
                    let fc = sweep_hz(t, mode.lo_hz, mode.hi_hz).min(rate * 0.49);
                    let a = allpass_a(fc, rate);
                    assert!(a.abs() < 1.0, "{mode:?} t {t} rate {rate}: a {a}");
                }
            }
        }
    }

    #[test]
    fn four_stages_make_exactly_two_notches() {
        let mode = four_stage();
        let controls = Controls {
            mode: index_of(mode),
            ..stock()
        };
        let fc = sweep_hz(0.5, mode.lo_hz, mode.hi_hz);
        // The two cancellation frequencies, from the exact warped-scale
        // arithmetic in `notch_hz`.
        let low = notch_hz(fc, 0, mode.stages, RATE);
        let high = notch_hz(fc, 1, mode.stages, RATE);
        assert!(gain_at(&controls, low) < 0.01, "low notch {low}");
        assert!(gain_at(&controls, high) < 0.01, "high notch {high}");
        // And the stage frequency itself passes untouched: at the −90°
        // crossing four stages total −360°, the chain arrives back in
        // phase with the dry, and the summer halves it whole.
        assert!(
            (gain_at(&controls, fc) - 1.0).abs() < 0.02,
            "gain at fc: {}",
            fc
        );
        // The response leaves its notches and stays up at both far ends
        // of the band.
        assert!(gain_at(&controls, 30.0) > 0.9, "below both");
        assert!(gain_at(&controls, 8_000.0) > 0.9, "above both");
    }

    #[test]
    fn two_stages_make_exactly_one_notch_at_the_stage_frequency() {
        // The 45's other setting, from the same arithmetic: with half
        // the sections the chain's phase covers two quarter-turns, and
        // the only odd multiple of −180° it crosses is −180° itself —
        // one notch, sitting exactly on the stage frequency. One hole
        // through the mids, and the sweep sails it.
        let mode = two_stage();
        let controls = Controls {
            mode: index_of(mode),
            ..stock()
        };
        let fc = sweep_hz(0.5, mode.lo_hz, mode.hi_hz);
        assert!(notch_count(mode.stages) == 1);
        let notch = notch_hz(fc, 0, mode.stages, RATE);
        // ψ = π/4 ⇒ tan ψ = 1 ⇒ the notch is the stage frequency itself.
        assert!((notch - fc).abs() < 1.0, "{notch} vs {fc}");
        assert!(gain_at(&controls, fc) < 0.01, "one notch, at fc");
        assert!(gain_at(&controls, fc * 0.2) > 0.9, "below it");
        assert!(gain_at(&controls, fc * 5.0) > 0.9, "above it");
    }

    #[test]
    fn the_notches_sit_where_the_angles_promise() {
        // The ratios the analog analysis promises — tan 22.5° and
        // tan 67.5° for four stages — measured on the very scale that
        // makes them exact, on a frozen sweep. A coarse scan finds each
        // hole, and the warped frequency of the hole found divided by
        // the stage frequency's must land on the promised ratio.
        let mode = four_stage();
        let controls = Controls {
            mode: index_of(mode),
            ..stock()
        };
        let fc = sweep_hz(0.5, mode.lo_hz, mode.hi_hz);
        let scan = |from: f32, to: f32| -> f32 {
            let mut best = from;
            let mut best_gain = f32::MAX;
            let mut f = from;
            while f <= to {
                let g = gain_at(&controls, f);
                if g < best_gain {
                    best_gain = g;
                    best = f;
                }
                f *= 1.01;
            }
            assert!(best_gain < 0.02, "scan {from}..{to} missed its notch");
            best
        };
        let tan_of = |f: f32| (std::f32::consts::PI * f / RATE).tan();
        let low = scan(fc * 0.2, fc * 0.8);
        let high = scan(fc * 1.3, (fc * 6.0).min(RATE * 0.15));
        assert!(
            (tan_of(low) / tan_of(fc) - std::f32::consts::FRAC_PI_8.tan()).abs() < 0.03,
            "lower ratio {}",
            tan_of(low) / tan_of(fc)
        );
        assert!(
            (tan_of(high) / tan_of(fc) - (3.0 * std::f32::consts::FRAC_PI_8).tan()).abs() < 0.05,
            "upper ratio {}",
            tan_of(high) / tan_of(fc)
        );
    }

    #[test]
    fn depth_zero_is_the_dry_signal_bit_for_bit() {
        let controls = Controls {
            speed: 0.33,
            depth: 0.0,
            mix: 1.0,
            ..stock()
        };
        let input: Vec<f32> = (0..RATE as usize)
            .map(|i| (i as f32 * 0.37).sin() * (i as f32 * 0.011).cos())
            .collect();
        let output = run(&controls, 1.0, |i| input[i]);
        for (x, y) in input.iter().zip(&output) {
            assert_eq!(y, x, "depth 0 must not touch a sample");
        }
    }

    #[test]
    fn mix_zero_is_flat_and_unity() {
        let controls = Controls {
            mix: 0.0,
            ..stock()
        };
        for probe in [120.0, 440.0, 1_200.0, 5_000.0] {
            let g = gain_at(&controls, probe);
            assert!((g - 1.0).abs() < 0.01, "mix 0 at {probe}: {g}");
        }
    }

    #[test]
    fn the_sweep_walks_the_notches_both_ways() {
        // The same probe frequency must be a hole at one end of the
        // flagship's sweep and open air at the other. The upper notch
        // rides at tan 67.5° of the stage frequency: at the sweep's
        // top (`hi_hz`) it lands near 2.414·hi_hz warped, right where
        // the probe sits; at the bottom the whole notch pair has
        // collapsed below a few hundred hertz and the probe is open
        // air by more than an order of magnitude.
        let mode = four_stage();
        let controls = Controls {
            speed: 0.25,
            mode: index_of(mode),
            ..stock()
        };
        let probe = notch_hz(mode.hi_hz, 1, mode.stages, RATE);
        // The sine starts mid-sweep and climbs, so at a quarter hertz
        // it crests at 1 s and 5 s and bottoms at 3 s and 7 s. The
        // windows straddle a crest and the bottom — ±50 ms, where the
        // stage frequency has moved a fraction of a percent.
        let out = run(&controls, 6.0, |i| {
            (2.0 * std::f32::consts::PI * probe * i as f32 / RATE).sin()
        });
        let window = |at: f32| -> f32 {
            let a = ((at - 0.05) * RATE) as usize;
            amplitude(&out[a..a + RATE as usize / 10], probe)
        };
        let at_bottom = window(3.0);
        let at_top = window(5.0);
        assert!(
            at_bottom > 0.9,
            "{probe:.0} Hz should sail through the bottom: {at_bottom}"
        );
        assert!(at_top < 0.15, "{probe:.0} Hz is the top's notch: {at_top}");
    }

    #[test]
    fn the_settings_share_one_sweep_and_keep_their_promises() {
        // The published analyses agree on the one thing the switch plates
        // never said: no setting of this family has a different sweep —
        // the difference is the tap and the feedback wire. So the table
        // must promise one range five times, and the audio must keep
        // each setting's own frozen promise at its own stage
        // frequency — a closed form per topology, below. The table's
        // numbers and the audio's agreement is the whole contract.
        let base = MODES[0];
        for mode in MODES {
            assert_eq!(mode.lo_hz, base.lo_hz, "{}'s bottom", mode.name);
            assert_eq!(mode.hi_hz, base.hi_hz, "{}'s top", mode.name);
        }
        for mode in MODES {
            let controls = Controls {
                mode: index_of(mode),
                ..stock()
            };
            let fc = sweep_hz(0.5, mode.lo_hz, mode.hi_hz);
            let g = gain_at(&controls, fc);
            let want = db_to_gain(mode.trim_db);
            // What the stage frequency itself means depends on the
            // topology, and every case has a closed form. Four stages
            // total a full turn at fc — the chain arrives in phase, so
            // the open setting passes it at exactly its trim, and the
            // feedback setting closes a *positive* loop worth
            // 0.5 + 0.5/(1−fb). Two stages total half a turn: fc *is*
            // their one notch, dead zero when open, and the feedback
            // resistor closes a *negative* loop that fills the hole to
            // exactly 0.5·fb/(1+fb). The one-sample register the wire
            // runs through costs each prediction a few thousandths of
            // its magnitude, well inside the tolerance. Feedback edits
            // the answer, never its address.
            let prediction = if mode.stages % 4 == 0 {
                want * if mode.feedback == 0.0 {
                    1.0
                } else {
                    0.5 + 0.5 / (1.0 - mode.feedback)
                }
            } else {
                want * if mode.feedback == 0.0 {
                    0.0
                } else {
                    0.5 * mode.feedback / (1.0 + mode.feedback)
                }
            };
            assert!(
                (g - prediction).abs() < 0.02,
                "{}: gain at fc {g} vs the closed form {prediction}",
                mode.name
            );
        }
    }

    #[test]
    fn feedback_fills_the_holes_it_cannot_move() {
        // The block's resistor, in sound: the same four stages at the
        // same sweep, one setting with the feedback wire and one
        // without. The cancellations keep their exact frequencies — the
        // allpasses still decide where the phase says odd multiples of
        // −180°, and a loop cannot renegotiate an angle — but the wire
        // partly fills those holes and blooms the band around the stage
        // frequency above unity. The fill and the bloom together are the
        // "whoosh" every review of a block-era board reaches for; the
        // ratio that produces them is a taste, recorded as one.
        let script = MODES
            .into_iter()
            .find(|m| m.stages == 4 && m.feedback == 0.0)
            .expect("a four-stage setting without feedback");
        let block = MODES
            .into_iter()
            .find(|m| m.stages == 4 && m.feedback > 0.0)
            .expect("its feedback sibling");
        let fc = sweep_hz(0.5, block.lo_hz, block.hi_hz);
        let notch = notch_hz(fc, 1, 4, RATE);
        let open = gain_at(
            &Controls {
                mode: index_of(script),
                ..stock()
            },
            notch,
        );
        let ring = gain_at(
            &Controls {
                mode: index_of(block),
                ..stock()
            },
            notch,
        );
        assert!(open < 0.01, "the open chain still cancels: {open}");
        assert!(
            ring > open * 3.0,
            "the wire must fill the hole it cannot move: {ring}"
        );
        assert!(ring < 0.35, "and fill it only partway: {ring}");
        // The band around the stage frequency blooms past unity — the
        // whoosh's loud half — while the open chain stands at exactly
        // its trim, one row above.
        let bloom = gain_at(
            &Controls {
                mode: index_of(block),
                ..stock()
            },
            fc,
        );
        assert!(
            bloom > 1.04,
            "feedback must ring, not merely exist: {bloom}"
        );
    }

    #[test]
    fn a_mode_flip_arrives_without_a_click() {
        // Crossfade's contract with the speaker: flip from a four-stage
        // setting without feedback to a two-stage one with it under a
        // settled tone — tap and resistor moving together, the way a
        // named toggle moves them — and the output's sample-to-sample
        // movement never exceeds what the tone itself could do by a
        // comfortable margin.
        let mut phaser = Phase::new(RATE, &stock());
        let tone = 200.0;
        let step = 2.0 * std::f32::consts::PI * tone / RATE;
        let mut block_in = [0.0; BLOCK];
        let mut block_out = [0.0; BLOCK];
        let mut previous_out = 0.0f32;
        let mut worst = 0.0f32;
        let flip = RATE as usize; // one second in
        for start in 0..(2 * RATE as usize) / BLOCK {
            let controls = Controls {
                mode: if start * BLOCK >= flip {
                    index_of(block_two())
                } else {
                    index_of(four_stage())
                },
                ..stock()
            };
            for (i, sample) in block_in[..BLOCK].iter_mut().enumerate() {
                *sample = ((start * BLOCK + i) as f32 * step).sin();
            }
            phaser.process_mono(&controls, &block_in, &mut block_out);
            for &y in &block_out {
                worst = worst.max((y - previous_out).abs());
                previous_out = y;
            }
        }
        // A 200 Hz sine at unity crosses at most sin(step) ≈ 0.026 per
        // sample; a real click would be an order of magnitude more.
        assert!(worst < 0.1, "worst sample step {worst}");
    }

    #[test]
    fn silence_leaves_no_trace() {
        let controls = stock();
        let out = run(&controls, 1.0, |_| 0.0);
        assert!(out.iter().all(|&y| y == 0.0));
    }
}
