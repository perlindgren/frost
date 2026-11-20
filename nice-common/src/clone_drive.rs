//! Component-based Centaur signal circuit. Component designators follow Martin
//! Chittum's corrected April 2009 trace (linked in nice-clone/README.md).
//!
//! Capacitors use trapezoidal companion models. The two clean feed-forward
//! paths and diode loading are solved as connected networks, not a dry/wet
//! crossfade. One circuit runs at the selected oversampling rate: the box is
//! mono, one signal through one pedal, and width is the host's work.
#![allow(clippy::needless_range_loop)] // Circuit matrices have fixed, small dimensions.

pub use crate::oversample::Oversampling;
use crate::oversample::{Resampler as Oversampler, TAPS, fir};
const SWITCH_FADE: f64 = 1.0 / 64.0;

/// A digital sample of 1.0 represents one volt peak at the pedal input/output.
/// This is a calibration convention, not a property inferred from an interface.
const VOLTS_PER_SAMPLE: f64 = 1.0;
const POT: f64 = 100_000.0;
const CONTACT: f64 = 0.001;
const UPDATE: usize = 16;
const SMOOTH_SECONDS: f64 = 0.010;
// Nominal germanium Shockley fit: about 0.35 V at 1 mA. No measured Behringer
// I/V curve is available. Junction capacitance, leakage asymmetry, and drift
// are not identifiable from a diode part number and are not invented here.
const IS: f64 = 2.0e-7;
const NVT: f64 = 0.041;
// TL072 nominal dominant-pole model; typical GBW from the TI datasheet.
const OPEN_GAIN: f64 = 200_000.0;
const GBW: f64 = 3_000_000.0;
const GAIN_RAIL: f64 = 3.0; // U1, 0/9 V supplies, relative to 4.5 V bias.
const OUTPUT_RAIL: f64 = 11.5; // U2, nominal charge-pump rails; symmetric approximation.

#[derive(Clone, Copy, Debug)]
pub struct Controls {
    pub gain: f32,
    pub treble: f32,
    pub output: f32,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            gain: 0.5,
            treble: 0.5,
            output: 0.5,
        }
    }
}

fn position(value: f32) -> f64 {
    if value.is_finite() {
        f64::from(value).clamp(0.0, 1.0)
    } else {
        0.5
    }
}

#[derive(Clone, Copy)]
enum Port {
    Node(usize),
    Source(usize),
    Ground,
}
use Port::{Ground as G, Node as N, Source as S};

#[derive(Clone, Copy)]
struct Capacitor {
    a: Port,
    b: Port,
    conductance: f64,
    history: f64,
}
impl Capacitor {
    fn new(a: Port, b: Port, farads: f64, rate: f64) -> Self {
        Self {
            a,
            b,
            conductance: 2.0 * rate * farads,
            history: 0.0,
        }
    }
    fn update(&mut self, nodes: &[f64], sources: &[f64; 2]) {
        let voltage = voltage(self.a, nodes, sources) - voltage(self.b, nodes, sources);
        self.history = -2.0 * self.conductance * voltage - self.history;
    }
}
fn voltage(port: Port, nodes: &[f64], sources: &[f64; 2]) -> f64 {
    match port {
        N(i) => nodes[i],
        S(i) => sources[i],
        G => 0.0,
    }
}

/// A passive network, factorized only when a knob moves. Capacitor histories
/// remain physical voltages/currents when resistances change.
#[derive(Clone)]
struct Network<const SIZE: usize, const CAPS: usize> {
    matrix: [[f64; SIZE]; SIZE],
    inverse: [[f64; SIZE]; SIZE],
    sources: [[f64; 2]; SIZE],
    caps: [Capacitor; CAPS],
}
impl<const SIZE: usize, const CAPS: usize> Network<SIZE, CAPS> {
    fn new(caps: [Capacitor; CAPS]) -> Self {
        Self {
            matrix: [[0.0; SIZE]; SIZE],
            inverse: [[0.0; SIZE]; SIZE],
            sources: [[0.0; 2]; SIZE],
            caps,
        }
    }
    fn begin(&mut self) {
        self.matrix = [[0.0; SIZE]; SIZE];
        self.sources = [[0.0; 2]; SIZE];
        for i in 0..CAPS {
            let cap = self.caps[i];
            self.stamp(cap.a, cap.b, cap.conductance);
        }
    }
    fn resistor(&mut self, a: Port, b: Port, ohms: f64) {
        self.stamp(a, b, 1.0 / ohms.max(CONTACT));
    }
    fn stamp(&mut self, a: Port, b: Port, conductance: f64) {
        for (from, to) in [(a, b), (b, a)] {
            if let N(i) = from {
                self.matrix[i][i] += conductance;
                match to {
                    N(j) => self.matrix[i][j] -= conductance,
                    S(j) => self.sources[i][j] += conductance,
                    G => {}
                }
            }
        }
    }
    fn finish(&mut self) {
        let mut a = self.matrix;
        let mut inv = [[0.0; SIZE]; SIZE];
        for i in 0..SIZE {
            inv[i][i] = 1.0;
        }
        for col in 0..SIZE {
            let mut pivot = col;
            for row in col + 1..SIZE {
                if a[row][col].abs() > a[pivot][col].abs() {
                    pivot = row;
                }
            }
            a.swap(col, pivot);
            inv.swap(col, pivot);
            let scale = a[col][col].recip();
            for j in 0..SIZE {
                a[col][j] *= scale;
                inv[col][j] *= scale;
            }
            for row in 0..SIZE {
                if row == col {
                    continue;
                }
                let weight = a[row][col];
                for j in 0..SIZE {
                    a[row][j] -= weight * a[col][j];
                    inv[row][j] -= weight * inv[col][j];
                }
            }
        }
        self.inverse = inv;
    }
    fn solve(&self, sources: [f64; 2]) -> [f64; SIZE] {
        let mut rhs = [0.0; SIZE];
        for i in 0..SIZE {
            rhs[i] = self.sources[i][0] * sources[0] + self.sources[i][1] * sources[1];
        }
        for cap in &self.caps {
            if let N(i) = cap.a {
                rhs[i] -= cap.history;
            }
            if let N(i) = cap.b {
                rhs[i] += cap.history;
            }
        }
        let mut result = [0.0; SIZE];
        for i in 0..SIZE {
            for j in 0..SIZE {
                result[i] += self.inverse[i][j] * rhs[j];
            }
        }
        result
    }
    fn commit(&mut self, nodes: &[f64; SIZE], sources: [f64; 2]) {
        for cap in &mut self.caps {
            cap.update(nodes, &sources);
        }
    }
}

#[derive(Clone, Copy)]
struct Pole {
    b0: f64,
    b1: f64,
    a1: f64,
    x: f64,
    y: f64,
}
impl Pole {
    fn new() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            a1: 0.0,
            x: 0.0,
            y: 0.0,
        }
    }
    /// H(s) = (n1*s+n0)/(d1*s+d0).
    fn tune(&mut self, n1: f64, n0: f64, d1: f64, d0: f64, rate: f64) {
        let k = 2.0 * rate;
        let a0 = d1 * k + d0;
        self.b0 = (n1 * k + n0) / a0;
        self.b1 = (n0 - n1 * k) / a0;
        self.a1 = (d0 - d1 * k) / a0;
    }
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x - self.a1 * self.y;
        self.x = x;
        self.y = y;
        y
    }
}

/// U1B with a finite-bandwidth dominant pole and actual C7/C8 feedback state. If the linear solution exceeds a
/// rail, recompute both feedback-node voltages with the output pinned there.
/// This retains saturation recovery; clipping a biquad's output would not.
#[derive(Clone, Copy)]
struct GainAmp {
    c7: Capacitor,
    c8: Capacitor,
    g0: f64,
    open_gain: f64,
    decay: f64,
    previous_output: f64,
    previous_error: f64,
}
impl GainAmp {
    fn new(rate: f64) -> Self {
        Self {
            c7: Capacitor::new(N(0), N(1), 82e-9, rate),
            c8: Capacitor::new(N(1), N(2), 390e-12, rate),
            g0: 0.0,
            open_gain: std::f64::consts::TAU * GBW
                / (2.0 * rate + std::f64::consts::TAU * GBW / OPEN_GAIN),
            decay: (2.0 * rate - std::f64::consts::TAU * GBW / OPEN_GAIN)
                / (2.0 * rate + std::f64::consts::TAU * GBW / OPEN_GAIN),
            previous_output: 0.0,
            previous_error: 0.0,
        }
    }
    fn tick(&mut self, plus: f64) -> f64 {
        let y7 = 1.0 / 15_000.0 + self.c7.conductance;
        let yf = 1.0 / 422_000.0 + self.c8.conductance;
        let a = self.g0 + y7;
        let denominator = y7 + yf - y7 * y7 / a;
        let alpha = yf / denominator;
        let beta = (self.c7.history - self.c8.history - y7 * self.c7.history / a) / denominator;
        let memory = self.open_gain * self.previous_error + self.decay * self.previous_output;
        let linear = (self.open_gain * (plus - beta) + memory) / (1.0 + self.open_gain * alpha);
        let out = linear.clamp(-GAIN_RAIL, GAIN_RAIL);
        let minus = alpha * out + beta;
        let lower = (y7 * minus - self.c7.history) / a;
        self.previous_output = out;
        self.previous_error = plus - minus;
        let nodes = [lower, minus, out];
        self.c7.update(&nodes, &[0.0; 2]);
        self.c8.update(&nodes, &[0.0; 2]);
        out
    }
}

fn diode_voltage(open: f64, resistance: f64) -> f64 {
    // Monotonic equation v + R*2*Is*sinh(v/nVt) = open. The analytical
    // upper bound avoids overflowing exp(), even after a full-scale step.
    let u = open.abs();
    if u == 0.0 {
        return 0.0;
    }
    let ri = 2.0 * resistance * IS;
    let mut lo = 0.0;
    let mut hi = u.min(NVT * (u / ri).asinh());
    let mut v = hi;
    for _ in 0..16 {
        let argument = v / NVT;
        let f = v + ri * argument.sinh() - u;
        if f.abs() < 1e-12 {
            break;
        }
        if f > 0.0 {
            hi = v;
        } else {
            lo = v;
        }
        let next = v - f / (1.0 + ri / NVT * argument.cosh());
        v = if next > lo && next < hi {
            next
        } else {
            0.5 * (lo + hi)
        };
    }
    v.copysign(open)
}

#[derive(Clone)]
/// Single-channel circuit at the processing rate; controls are Gain/Treble/Output.
/// The caller supplies smoothing and oversampling when using this raw core.
pub struct Circuit {
    input: Pole,
    front: Network<3, 3>,
    amp: GainAmp,
    // Nodes: C9 right, diodes, C10 right, C11 lower, R5 lower,
    // C6 lower, clean-pot wiper, C12 lower. Summing junction = AC ground.
    blend: Network<8, 6>,
    summer: Pole,
    tone: Pole,
    // C2 right, C15 right, output wiper. Includes the original 68k bypass
    // bridge's feed-through in effect-on mode and a 100k output load.
    output: Network<3, 2>,
    rate: f64,
}
impl Circuit {
    pub fn new(rate: f64, controls: [f64; 3]) -> Self {
        let cap = |a, b, c| Capacitor::new(a, b, c, rate);
        let mut input = Pole::new();
        input.tune(0.1, 0.0, 0.101, 1.0, rate); // C1=100n, R1=10k, R2=1M.
        let front = Network::new([
            cap(N(0), S(0), 100e-9),
            cap(N(0), N(1), 68e-9),
            cap(N(2), G, 1e-6),
        ]);
        let blend = Network::new([
            cap(S(0), N(0), 1e-6),
            cap(N(1), N(2), 1e-6),
            cap(N(2), N(3), 2.2e-9),
            cap(S(1), N(4), 68e-9),
            cap(N(4), N(5), 390e-9),
            cap(G, N(7), 27e-9),
        ]);
        let output = Network::new([cap(S(0), N(0), 4.7e-6), cap(S(1), N(1), 4.7e-6)]);
        let mut summer = Pole::new();
        summer.tune(0.0, -392_000.0, 392_000.0 * 820e-12, 1.0, rate);
        let mut circuit = Self {
            input,
            front,
            amp: GainAmp::new(rate),
            blend,
            summer,
            tone: Pole::new(),
            output,
            rate,
        };
        circuit.tune(controls);
        circuit
    }
    pub fn tune(&mut self, [gain, treble, output]: [f64; 3]) {
        self.front.begin();
        self.front.resistor(N(0), N(1), 10_000.0);
        self.front.resistor(N(1), G, gain * POT);
        self.front.resistor(N(0), N(2), 1_500.0);
        self.front.resistor(N(2), G, 15_000.0);
        self.front.finish();
        self.amp.g0 = (2_000.0 + (1.0 - gain) * POT).recip();
        self.blend.begin();
        self.blend.resistor(N(0), N(1), 1_000.0);
        self.blend.resistor(N(2), G, 47_000.0);
        self.blend.resistor(N(3), N(6), 22_000.0);
        self.blend.resistor(S(1), N(4), 5_100.0);
        self.blend.resistor(N(4), G, 1_500.0);
        self.blend.resistor(N(5), G, 1_000.0);
        self.blend.resistor(N(4), N(6), gain * POT);
        self.blend.resistor(N(6), G, (1.0 - gain) * POT);
        self.blend.resistor(N(6), G, 27_000.0);
        self.blend.resistor(N(6), N(7), 12_000.0);
        self.blend.finish();
        // U2B: exact ideal-op-amp treble transfer, R21/R23 plus P2 halves.
        let a = 1_800.0 + (1.0 - treble) * 10_000.0;
        let b = 4_700.0 + treble * 10_000.0;
        let conductance = a.recip() + b.recip();
        self.tone.tune(
            -3.9e-9 * (1.0 + 100_000.0 / a),
            -conductance,
            3.9e-9 * (1.0 + 100_000.0 / b),
            conductance,
            self.rate,
        );
        self.output.begin();
        self.output.resistor(N(0), G, 100_000.0);
        self.output.resistor(N(0), N(2), 68_560.0);
        self.output
            .resistor(N(1), N(2), 560.0 + (1.0 - output) * 10_000.0);
        self.output.resistor(N(2), G, output * 10_000.0);
        self.output.resistor(N(2), G, 100_000.0);
        self.output.finish();
    }
    pub fn tick(&mut self, sample: f64) -> f64 {
        let buffered = self.input.tick(sample).clamp(-GAIN_RAIL, GAIN_RAIL);
        let front = self.front.solve([buffered, 0.0]);
        self.front.commit(&front, [buffered, 0.0]);
        let amplified = self.amp.tick(front[1]);
        let sources = [amplified, buffered];
        let mut nodes = self.blend.solve(sources);
        let z = self.blend.inverse[1][1];
        let diode = diode_voltage(nodes[1], z);
        let current = (nodes[1] - diode) / z;
        for i in 0..8 {
            nodes[i] -= self.blend.inverse[i][1] * current;
        }
        let clean_cap = self.blend.caps[5];
        let clean_current = clean_cap.history - clean_cap.conductance * nodes[7];
        let summed_current =
            front[2] / 15_000.0 + nodes[2] / 47_000.0 + nodes[6] / 27_000.0 - clean_current;
        self.blend.commit(&nodes, sources);
        let summed = self
            .summer
            .tick(summed_current)
            .clamp(-OUTPUT_RAIL, OUTPUT_RAIL);
        let toned = self.tone.tick(summed).clamp(-OUTPUT_RAIL, OUTPUT_RAIL);
        let sources = [buffered, toned];
        let output = self.output.solve(sources);
        self.output.commit(&output, sources);
        output[2]
    }
}

pub struct CloneDrive {
    circuit: Circuit,
    resampler: Oversampler,
    taps: [f64; TAPS],
    smoothed: [f64; 3],
    targets: [f64; 3],
    until_update: usize,
    smoothing: f64,
    rate: f64,
    mode: Oversampling,
    requested_mode: Oversampling,
    switch_gain: f64,
}
impl CloneDrive {
    pub fn new(rate: f32, controls: &Controls) -> Self {
        Self::with_oversampling(rate, controls, Oversampling::default())
    }
    pub fn with_oversampling(rate: f32, controls: &Controls, mode: Oversampling) -> Self {
        let rate = f64::from(rate);
        assert!(rate.is_finite() && rate >= 8_000.0);
        let values = [
            position(controls.gain),
            position(controls.treble),
            position(controls.output),
        ];
        Self {
            circuit: Circuit::new(rate * mode.factor() as f64, values),
            resampler: Oversampler::new(),
            taps: fir(mode),
            smoothed: values,
            targets: values,
            until_update: 0,
            smoothing: (-(UPDATE as f64) / (SMOOTH_SECONDS * rate)).exp(),
            rate,
            mode,
            requested_mode: mode,
            switch_gain: 1.0,
        }
    }
    pub fn set_rate(&mut self, rate: f32) {
        let controls = Controls {
            gain: self.targets[0] as f32,
            treble: self.targets[1] as f32,
            output: self.targets[2] as f32,
        };
        *self = Self::with_oversampling(rate, &controls, self.requested_mode);
    }
    pub fn reset(&mut self) {
        self.set_rate(self.rate as f32);
    }
    pub fn oversampling(&self) -> Oversampling {
        self.mode
    }
    pub fn latency(&self) -> u32 {
        self.mode.latency()
    }
    /// Switch during playback: fade down before replacing rate-dependent state,
    /// then fade up. There is no heap allocation and no parallel old/new engine.
    pub fn request_oversampling(&mut self, mode: Oversampling) {
        self.requested_mode = mode;
    }
    fn update_oversampling(&mut self) {
        if self.requested_mode != self.mode {
            self.switch_gain = (self.switch_gain - SWITCH_FADE).max(0.0);
            if self.switch_gain == 0.0 {
                self.mode = self.requested_mode;
                self.circuit = Circuit::new(self.rate * self.mode.factor() as f64, self.smoothed);
                self.resampler = Oversampler::new();
                self.taps = fir(self.mode);
            }
        } else {
            self.switch_gain = (self.switch_gain + SWITCH_FADE).min(1.0);
        }
    }
    pub fn set_controls(&mut self, controls: &Controls) {
        self.targets = [
            position(controls.gain),
            position(controls.treble),
            position(controls.output),
        ];
    }
    /// Push one sample of one signal through the one circuit. The plugin hands
    /// this the mono input directly; the pedal never folds channels together,
    /// because width is the host's work, not the pedal's.
    pub fn process(&mut self, input: f32) -> f32 {
        self.update_oversampling();
        if self.until_update == 0 {
            let mut changed = false;
            for i in 0..3 {
                let error = self.targets[i] - self.smoothed[i];
                if error != 0.0 {
                    self.smoothed[i] = if error.abs() < 1e-8 {
                        self.targets[i]
                    } else {
                        self.targets[i] - self.smoothing * error
                    };
                    changed = true;
                }
            }
            if changed {
                self.circuit.tune(self.smoothed);
            }
            self.until_update = UPDATE;
        }
        self.until_update -= 1;
        // Nonfinite host samples cannot poison capacitor state. Extreme
        // inputs are bounded far above the modeled buffer's headroom.
        let sample = if input.is_finite() {
            f64::from(input).clamp(-64.0, 64.0)
        } else {
            0.0
        };
        let volts = sample * VOLTS_PER_SAMPLE;
        let processed = if self.mode == Oversampling::Off {
            self.circuit.tick(volts)
        } else {
            self.resampler.tick(
                volts,
                &self.taps[..self.mode.taps()],
                self.mode.factor(),
                |sample| self.circuit.tick(sample),
            )
        };
        (processed * self.switch_gain / VOLTS_PER_SAMPLE) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diode_solution_satisfies_kirchhoff_for_small_and_extreme_drives() {
        for r in [0.1, 10.0, 1_000.0, 100_000.0] {
            for input in [-100.0, -3.0, -0.35, -0.001, 0.0, 0.001, 0.35, 3.0, 100.0] {
                let v = diode_voltage(input, r);
                let residual = v + r * 2.0 * IS * (v / NVT).sinh() - input;
                assert!(
                    residual.abs() < 1e-9,
                    "R={r} input={input} v={v} residual={residual}"
                );
            }
        }
    }

    #[test]
    fn input_buffer_and_feedback_amplifier_respect_supply_headroom() {
        let mut amp = GainAmp::new(384_000.0);
        amp.g0 = 1.0 / 2_000.0;
        let mut positive = false;
        let mut negative = false;
        for i in 0..4096 {
            let plus = if i % 512 < 256 { 2.0 } else { -2.0 };
            let old7 = amp.c7;
            let old8 = amp.c8;
            let memory = amp.open_gain * amp.previous_error + amp.decay * amp.previous_output;
            let out = amp.tick(plus);
            positive |= out == GAIN_RAIL;
            negative |= out == -GAIN_RAIL;
            assert!(out.abs() <= GAIN_RAIL);
            // Recover capacitor terminal voltages from the committed history.
            let d7 = -(amp.c7.history + old7.history) / (2.0 * old7.conductance);
            let d8 = -(amp.c8.history + old8.history) / (2.0 * old8.conductance);
            let minus = d8 + out;
            let lower = d7 + minus;
            let r_lower = amp.g0 * lower
                + (1.0 / 15_000.0 + old7.conductance) * (lower - minus)
                + old7.history;
            let r_minus = (1.0 / 15_000.0 + old7.conductance) * (minus - lower) - old7.history
                + (1.0 / 422_000.0 + old8.conductance) * (minus - out)
                + old8.history;
            assert!(r_lower.abs() < 1e-12 && r_minus.abs() < 1e-12);
            if out.abs() < GAIN_RAIL {
                assert!((out - amp.open_gain * (plus - minus) - memory).abs() < 1e-9);
            }
        }
        assert!(positive && negative);
    }

    #[test]
    fn reset_removes_all_history() {
        let controls = Controls::default();
        let mut drive = CloneDrive::new(48_000.0, &controls);
        for i in 0..2048 {
            let out = drive.process((i as f32 * 0.13).sin() * 0.8);
            assert!(out.is_finite());
        }
        drive.reset();
        for _ in 0..128 {
            assert_eq!(drive.process(0.0), 0.0);
        }
    }

    #[test]
    fn oversampling_has_the_reported_delay_and_rejects_images() {
        let taps = fir(Oversampling::X8);
        let mut impulse = [0.0; 65];
        // Linear up/down round trip, independent of the circuit's own phase.
        for (host, output) in impulse.iter_mut().enumerate() {
            for a in 0..TAPS {
                let b = host * 8;
                if b >= a && b - a < TAPS {
                    *output += 8.0 * taps[a] * taps[b - a];
                }
            }
        }
        let peak = impulse
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
            .0;
        assert_eq!(peak, Oversampling::X8.latency() as usize);
        let gain: f64 = impulse.iter().sum();
        assert!((gain - 1.0).abs() < 1e-6);
        let response = |frequency: f64| {
            let (mut re, mut im) = (0.0, 0.0);
            for (i, &h) in taps.iter().enumerate() {
                let angle = 2.0 * std::f64::consts::PI * frequency * i as f64;
                re += h * angle.cos();
                im += h * angle.sin();
            }
            re.hypot(im)
        };
        assert!(response(0.4 / 8.0) > 0.999);
        assert!(response(0.6 / 8.0) < 0.001);
    }

    #[test]
    fn controls_change_distortion_brightness_and_level() {
        fn harmonics(controls: [f64; 3]) -> [f64; 3] {
            let rate = 64_000.0;
            let mut circuit = Circuit::new(rate, controls);
            let mut sums = [(0.0, 0.0); 3];
            for i in 0..16_384 {
                let phase = 2.0 * std::f64::consts::PI * i as f64 / 64.0;
                let out = circuit.tick(0.25 * phase.sin());
                if i >= 8192 {
                    for (h, sum) in sums.iter_mut().enumerate() {
                        let p = phase * (2 * h + 1) as f64;
                        sum.0 += out * p.cos();
                        sum.1 += out * p.sin();
                    }
                }
            }
            sums.map(|(re, im)| re.hypot(im) / 4096.0)
        }
        let clean = harmonics([0.0, 0.5, 0.5]);
        let dirty = harmonics([1.0, 0.5, 0.5]);
        assert!(clean[1] / clean[0] < 0.001, "{clean:?}");
        assert!(dirty[1] / dirty[0] > 0.03, "{dirty:?}");
        let dark = harmonics([0.7, 0.0, 0.5]);
        let bright = harmonics([0.7, 1.0, 0.5]);
        assert!(bright[1] / bright[0] > dark[1] / dark[0]);
        let quiet = harmonics([0.5, 0.5, 0.0]);
        assert!(quiet[0] < 1e-6, "{quiet:?}");
        let loud = harmonics([0.5, 0.5, 1.0]);
        let medium = harmonics([0.5, 0.5, 0.5]);
        assert!(loud[0] > medium[0] * 1.5);
    }

    #[test]
    fn automation_and_bad_inputs_stay_finite_at_supported_rates() {
        for rate in [8_000.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
            let mut drive = CloneDrive::new(rate, &Controls::default());
            for i in 0..512 {
                if i % 64 == 0 {
                    let value = if i % 128 == 0 { 0.0 } else { 1.0 };
                    drive.set_controls(&Controls {
                        gain: value,
                        treble: 1.0 - value,
                        output: value,
                    });
                }
                let sample = [f32::NAN, f32::INFINITY, -f32::MAX, f32::MAX, 0.0][i % 5];
                let y = drive.process(sample);
                assert!(y.is_finite() && y.abs() < 32.0, "rate={rate}, output={y}");
            }
            drive.set_controls(&Controls {
                gain: f32::NAN,
                treble: f32::INFINITY,
                output: -9.0,
            });
            assert!(drive.process(0.1).is_finite());
        }
    }
}

#[cfg(test)]
mod ac_reference_tests {
    use super::*;

    #[test]
    fn time_domain_circuit_matches_independent_full_nodal_ac_reference() {
        // Reference node map:
        // 0 buffer, 1 C3 right, 2 plus, 3 minus, 4 R10 lower, 5 gain out,
        // 6 FF1, 7 C9 right, 8 diode, 9 wet, 10 C11 lower, 11 clean top,
        // 12 C6 lower, 13 clean wiper, 14 C12 lower, 15 summer minus,
        // 16 summer out, 17 tone minus, 18 tone wiper, 19 tone out,
        // 20 C2 right, 21 C15 right, 22 output.
        // Initialize from steady-state phasors so low-frequency settling does
        // not dominate test duration. Subsequent samples run the real nonlinear
        // circuit. The fixture generator uses a separate 27x27 complex MNA.
        for line in include_str!("../tests/reference/clone-drive/ac.csv")
            .lines()
            .filter(|l| !l.starts_with('#'))
        {
            let fields: Vec<f64> = line.split(',').map(|s| s.parse().unwrap()).collect();
            let (rate, hz) = (fields[0], fields[1]);
            let controls = [fields[2], fields[3], fields[4]];
            let mut circuit = Circuit::new(rate, controls);
            let omega = 2.0 * std::f64::consts::PI * hz / rate;
            let analog_w = 2.0 * rate * (omega / 2.0).tan();
            let scale = 1e-7;
            let pair = |node: usize| (fields[5 + node * 2], fields[6 + node * 2]);
            let at = |(re, im): (f64, f64), t: f64| scale * (re * t.cos() - im * t.sin());
            let value = |node: usize, t: f64| at(pair(node), t);
            let seed_cap = |cap: &mut Capacitor, a: Option<usize>, b: Option<usize>| {
                let (ar, ai) = a.map(&pair).unwrap_or((0.0, 0.0));
                let (br, bi) = b.map(&pair).unwrap_or((0.0, 0.0));
                let (re, im) = (ar - br, ai - bi);
                let volts = at((re, im), -omega);
                // i = j*w*C*v; G = 2*fs*C.
                let current =
                    at((-im * analog_w, re * analog_w), -omega) * cap.conductance / (2.0 * rate);
                cap.history = -cap.conductance * volts - current;
            };
            circuit.input.x = at((1.0, 0.0), -omega);
            circuit.input.y = value(0, -omega);
            for (cap, (a, b)) in
                circuit
                    .front
                    .caps
                    .iter_mut()
                    .zip([(1, 0), (1, 2), (6, usize::MAX)])
            {
                seed_cap(cap, Some(a), (b != usize::MAX).then_some(b));
            }
            circuit.amp.previous_output = value(5, -omega);
            circuit.amp.previous_error = value(2, -omega) - value(3, -omega);
            seed_cap(&mut circuit.amp.c7, Some(4), Some(3));
            seed_cap(&mut circuit.amp.c8, Some(3), Some(5));
            for (cap, (a, b)) in circuit.blend.caps.iter_mut().zip([
                (5, 7),
                (8, 9),
                (9, 10),
                (0, 11),
                (11, 12),
                (15, 14),
            ]) {
                seed_cap(cap, Some(a), Some(b));
            }
            let (re, im) = pair(16);
            let current = (
                -re / 392_000.0 + im * 820e-12 * analog_w,
                -im / 392_000.0 - re * 820e-12 * analog_w,
            );
            circuit.summer.x = at(current, -omega);
            circuit.summer.y = value(16, -omega);
            circuit.tone.x = value(16, -omega);
            circuit.tone.y = value(19, -omega);
            seed_cap(&mut circuit.output.caps[0], Some(0), Some(20));
            seed_cap(&mut circuit.output.caps[1], Some(19), Some(21));
            let amplitude = scale * fields[49].hypot(fields[50]);
            for i in 0..128 {
                let phase = omega * i as f64;
                let actual = circuit.tick(scale * phase.cos());
                let expected = value(22, phase);
                assert!(
                    (actual - expected).abs() < amplitude * 2e-5 + 1e-13,
                    "rate={rate} hz={hz} controls={controls:?} sample={i}: {actual} != {expected}"
                );
            }
        }
    }
}

#[cfg(test)]
mod transient_reference_tests {
    use super::*;

    #[test]
    fn nonlinear_transients_match_full_circuit_mna_including_opamp_saturation() {
        for line in include_str!("../tests/reference/clone-drive/transient.csv")
            .lines()
            .filter(|l| !l.starts_with('#'))
        {
            let f: Vec<f64> = line.split(',').map(|s| s.parse().unwrap()).collect();
            let mut circuit = Circuit::new(f[0], [f[1], f[2], f[3]]);
            if f[1] == 1.0 {
                assert!(f[4] > 0.0, "reference must exercise opamp saturation");
            }
            for i in 0..4096 {
                let t = i as f64 / f[0];
                let raw = 0.7 * (2.0 * std::f64::consts::PI * 700.0 * t).sin()
                    + 0.1 * (2.0 * std::f64::consts::PI * 3100.0 * t).sin();
                let actual = circuit.tick(raw);
                if i % 32 == 31 {
                    let expected = f[5 + i / 32];
                    assert!(
                        (actual - expected).abs() < 2e-8,
                        "gain={} sample={i} actual={actual} expected={expected}",
                        f[1]
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod alias_tests {
    use super::*;

    #[test]
    fn oversampling_suppresses_folded_distortion_harmonics() {
        let controls = Controls {
            gain: 1.0,
            treble: 1.0,
            output: 0.5,
        };
        let mut drive = CloneDrive::new(48_000.0, &controls);
        let mut base_rate = Circuit::new(48_000.0, [1.0, 1.0, 0.5]);
        let mut oversampled = (0.0, 0.0);
        let mut plain = (0.0, 0.0);
        // A 7.5 kHz input's fifth harmonic folds from 37.5 to 10.5 kHz
        // without oversampling. These bins are coherent over the capture.
        for i in 0..16_384 {
            let t = i as f64 / 48_000.0;
            let input = 0.8 * (std::f64::consts::TAU * 7_500.0 * t).sin();
            let high = f64::from(drive.process(input as f32));
            let low = base_rate.tick(input);
            if i >= 8192 {
                let phase = std::f64::consts::TAU * 10_500.0 * t;
                oversampled.0 += high * phase.cos();
                oversampled.1 += high * phase.sin();
                plain.0 += low * phase.cos();
                plain.1 += low * phase.sin();
            }
        }
        let high = oversampled.0.hypot(oversampled.1);
        let low = plain.0.hypot(plain.1);
        assert!(
            low > 0.01,
            "control must produce measurable folded harmonics"
        );
        assert!(
            high < low * 0.1,
            "alias amplitude: oversampled={high}, base-rate={low}"
        );
    }
}

#[cfg(test)]
mod selectable_oversampling_tests {
    use super::*;

    #[test]
    fn every_factor_has_correct_internal_rate_filter_length_and_delay() {
        for mode in Oversampling::ALL {
            let drive = CloneDrive::with_oversampling(48_000.0, &Controls::default(), mode);
            assert_eq!(drive.circuit.rate, 48_000.0 * mode.factor() as f64);
            assert_eq!(Oversampling::from_index(mode.index()), mode);
            assert_eq!(Oversampling::from_factor(mode.factor() as u32), mode);
            if mode == Oversampling::Off {
                assert_eq!(drive.latency(), 0);
                continue;
            }
            let taps = &drive.taps[..mode.taps()];
            assert_eq!(taps.len(), 32 * mode.factor() + 1);
            let mut impulse = [0.0f64; 65];
            for (i, y) in impulse.iter_mut().enumerate() {
                let time = i * mode.factor();
                for a in 0..taps.len() {
                    if time >= a && time - a < taps.len() {
                        *y += mode.factor() as f64 * taps[a] * taps[time - a];
                    }
                }
            }
            let peak = impulse
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .unwrap()
                .0;
            assert_eq!(peak as u32, drive.latency());
            assert!((impulse.iter().sum::<f64>() - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn off_processes_the_host_rate_circuit_without_any_resampling() {
        let controls = Controls::default();
        let mut drive = CloneDrive::with_oversampling(48_000.0, &controls, Oversampling::Off);
        let mut circuit = Circuit::new(48_000.0, [0.5; 3]);
        for i in 0..2048 {
            let input = (i as f32 * 0.18).sin() * 0.5;
            assert_eq!(drive.process(input), circuit.tick(f64::from(input)) as f32);
        }
    }

    #[test]
    fn live_switches_fade_through_zero_and_survive_rate_change_and_reset() {
        let mut drive = CloneDrive::new(48_000.0, &Controls::default());
        for target in [
            Oversampling::Off,
            Oversampling::X2,
            Oversampling::X4,
            Oversampling::X8,
        ] {
            for _ in 0..256 {
                drive.process(0.2);
            }
            drive.request_oversampling(target);
            for i in 0..128 {
                let previous = drive.oversampling();
                let output = drive.process(0.2);
                if drive.oversampling() != previous {
                    assert_eq!(i, 63);
                    assert_eq!(output, 0.0);
                }
                assert!(output.is_finite());
            }
            assert_eq!(drive.oversampling(), target);
            assert_eq!(drive.switch_gain, 1.0);
            drive.reset();
            assert_eq!(drive.oversampling(), target);
            drive.set_rate(96_000.0);
            assert_eq!(drive.oversampling(), target);
            assert_eq!(drive.circuit.rate, 96_000.0 * target.factor() as f64);
            drive.set_rate(48_000.0);
        }
    }

    #[test]
    fn changing_rates_keeps_finite_output() {
        for mode in Oversampling::ALL {
            for rate in [8_000.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0] {
                let mut drive = CloneDrive::with_oversampling(rate, &Controls::default(), mode);
                for i in 0..512 {
                    let output = drive.process((i as f32 * 0.23).sin());
                    assert!(output.is_finite());
                }
            }
        }
    }
}
