//! Nominal TS808/TS9/OD-1/SD-1/RAT signal-path model. See README.md for sources and limits.
#![allow(clippy::needless_range_loop)]
#[path = "dsp/rat.rs"]
mod rat;
use crate::controls::Variant;
use nice_common::oversample::{Oversampling, Resampler, TAPS, fir};
const CONTACT: f64 = 0.1;
const IS: f64 = 2.52e-9; // nominal silicon pair, not a measured pedal fit
const NVT: f64 = 0.045;
const LATENCY: u32 = 32;

#[derive(Clone, Copy)]
pub struct Controls {
    pub drive: f32,
    pub tone: f32,
    pub level: f32,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            drive: 0.5,
            tone: 0.5,
            level: 0.5,
        }
    }
}
fn position(x: f32) -> f64 {
    if x.is_finite() {
        f64::from(x).clamp(0.0, 1.0)
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
    // An amplifier equation replaces this KCL row; source 1 supplies its RHS.
    constraint_row: Option<usize>,
}
impl<const SIZE: usize, const CAPS: usize> Network<SIZE, CAPS> {
    fn new(caps: [Capacitor; CAPS]) -> Self {
        Self {
            matrix: [[0.0; SIZE]; SIZE],
            inverse: [[0.0; SIZE]; SIZE],
            sources: [[0.0; 2]; SIZE],
            caps,
            constraint_row: None,
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
        if let Some(row) = self.constraint_row {
            rhs[row] = sources[1];
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

#[derive(Clone, Copy, Default)]
struct Filter {
    b0: f64,
    b1: f64,
    a1: f64,
    x: f64,
    y: f64,
}
impl Filter {
    fn highpass(rate: f64, cap: f64, series: f64, load: f64) -> Self {
        let k = 2.0 * rate * cap;
        let denominator = 1.0 + k * (series + load);
        let b0 = k * load / denominator;
        Self {
            b0,
            b1: -b0,
            a1: (1.0 - k * (series + load)) / denominator,
            ..Self::default()
        }
    }
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x - self.a1 * self.y;
        self.x = x;
        self.y = y;
        y
    }
}

/// Fast symmetric feedback solution shared by both Tube Screamer variants.
fn symmetric_feedback_voltage(current: f64, g: f64) -> f64 {
    let target = current.abs();
    if target == 0.0 {
        return 0.0;
    }
    let mut lo = 0.0;
    let mut hi = (target / g).min(NVT * (target / (2.0 * IS)).asinh());
    let mut v = hi;
    for _ in 0..20 {
        let a = v / NVT;
        let error = g * v + 2.0 * IS * a.sinh() - target;
        if error.abs() < 1e-22 + target * 1e-12 {
            break;
        }
        if error > 0.0 {
            hi = v;
        } else {
            lo = v;
        }
        let next = v - error / (g + 2.0 * IS / NVT * a.cosh());
        v = if next > lo && next < hi {
            next
        } else {
            (lo + hi) * 0.5
        };
    }
    v.copysign(current)
}

/// Solve the parallel resistor/capacitor and antiparallel diode strings.
/// SD-1 has two diodes for positive output-minus-input voltage and one
/// for negative voltage; Tube Screamers have one in each direction.
fn feedback_voltage(current: f64, g: f64, variant: Variant) -> f64 {
    if !matches!(variant, Variant::Sd1 | Variant::Od1) {
        return symmetric_feedback_voltage(current, g);
    }
    let target = current.abs();
    if target == 0.0 {
        return 0.0;
    }
    let (forward, reverse) = if current >= 0.0 {
        (2.0, 1.0)
    } else {
        (1.0, 2.0)
    };
    let mut lo = 0.0;
    let mut hi = (target / g).min(forward * NVT * (target / IS).ln_1p());
    let mut v = hi;
    for _ in 0..24 {
        let a = v / (forward * NVT);
        let b = -v / (reverse * NVT);
        let diode = IS * (a.exp_m1() - b.exp_m1());
        let error = g * v + diode - target;
        if error.abs() < 1e-22 + target * 1e-12 {
            break;
        }
        if error > 0.0 {
            hi = v;
        } else {
            lo = v;
        }
        let next = v - error / (g + IS * (a.exp() / (forward * NVT) + b.exp() / (reverse * NVT)));
        v = if next > lo && next < hi {
            next
        } else {
            (lo + hi) * 0.5
        };
    }
    v.copysign(current)
}

struct Circuit {
    input: Filter,
    coupling: Filter,
    ground_cap: f64,
    ground_history: f64,
    feedback_cap: f64,
    feedback_history: f64,
    resistance: f64,
    variant: Variant,
    tone: Network<5, 3>,
    level: Network<5, 2>,
    output: Filter,
    od_tone: Filter,
    rat: Option<rat::Rat>,
    clone: Option<nice_common::clone_drive::Circuit>,
}
impl Circuit {
    fn new(rate: f64, c: [f64; 3], variant: Variant) -> Self {
        let od1 = variant == Variant::Od1;
        let sd1 = matches!(variant, Variant::Sd1 | Variant::Od1);
        let (series, shunt, output_cap) = match variant {
            Variant::Ts808 => (100.0, 10_000.0, 10e-6),
            Variant::Ts9 => (470.0, 100_000.0, 10e-6),
            Variant::Sd1 | Variant::Od1 | Variant::Rat | Variant::Clone => {
                (1_000.0, 100_000.0, 1e-6)
            }
        };
        let mut circuit = Self {
            variant,
            clone: (variant == Variant::Clone)
                .then(|| nice_common::clone_drive::Circuit::new(rate, c)),
            rat: (variant == Variant::Rat).then(|| rat::Rat::new(rate, c)),
            // OD-1 R7=R8=10k, C4=18nF: inverting unity-gain low pass.
            od_tone: {
                let k = 2.0 * rate * 10_000.0 * 18e-9;
                Filter {
                    b0: -1.0 / (1.0 + k),
                    b1: -1.0 / (1.0 + k),
                    a1: (1.0 - k) / (1.0 + k),
                    ..Filter::default()
                }
            },
            input: Filter::highpass(
                rate,
                if sd1 { 47e-9 } else { 22e-9 },
                if od1 {
                    1_000.0
                } else if sd1 {
                    10_000.0
                } else {
                    1_000.0
                },
                if sd1 { 470_000.0 } else { 510_000.0 },
            ),
            coupling: Filter::highpass(
                rate,
                if od1 {
                    4.7e-9
                } else if sd1 {
                    18e-9
                } else {
                    1e-6
                },
                0.0,
                if sd1 { 100_000.0 } else { 10_000.0 },
            ),
            ground_cap: 2.0 * rate * 47e-9,
            ground_history: 0.0,
            feedback_cap: if sd1 { 0.0 } else { 2.0 * rate * 51e-12 },
            feedback_history: 0.0,
            resistance: 0.0,
            tone: Network::new([
                Capacitor::new(N(0), G, if sd1 { 18e-9 } else { 220e-9 }, rate),
                Capacitor::new(N(1), N(2), if sd1 { 27e-9 } else { 220e-9 }, rate),
                Capacitor::new(N(3), N(4), if sd1 { 10e-9 } else { 0.0 }, rate),
            ]),
            level: Network::new([
                Capacitor::new(S(0), N(0), 1e-6, rate),
                Capacitor::new(N(3), N(4), if sd1 { 47e-9 } else { 100e-9 }, rate),
            ]),
            output: Filter::highpass(
                rate,
                output_cap,
                series,
                1.0 / (1.0 / shunt + 1.0 / 1_000_000.0),
            ),
        };
        circuit.tune(c);
        circuit
    }
    fn tune(&mut self, [drive, tone, level]: [f64; 3]) {
        if let Some(clone) = &mut self.clone {
            clone.tune([drive, tone, level]);
            return;
        }
        if let Some(rat) = &mut self.rat {
            rat.tune([drive, tone, level]);
            return;
        }
        let od1 = self.variant == Variant::Od1;
        let sd1 = matches!(self.variant, Variant::Sd1 | Variant::Od1);
        self.resistance = if sd1 {
            33_000.0 + 1_000_000.0 * drive
        } else {
            51_000.0 + 500_000.0 * drive * drive
        };
        self.tone.begin();
        self.tone
            .resistor(S(0), N(0), if sd1 { 10_000.0 } else { 1_000.0 });
        if !sd1 {
            self.tone.resistor(N(0), G, 10_000.0);
        }
        self.tone.resistor(N(0), N(1), 20_000.0 * tone);
        self.tone.resistor(N(1), N(3), 20_000.0 * (1.0 - tone));
        self.tone.resistor(N(2), G, if sd1 { 470.0 } else { 220.0 });
        self.tone
            .resistor(N(3), N(4), if sd1 { 10_000.0 } else { 1_000.0 });
        // Ideal second op-amp: inverting input (3) equals noninverting (0).
        // Capacitor history on the output node must not enter the ideal
        // op-amp voltage constraint (row 4 is not an output KCL equation).
        self.tone.constraint_row = Some(4);
        self.tone.matrix[4] = [-1.0, 0.0, 0.0, 1.0, 0.0];
        self.tone.finish();
        let level = if sd1 { level } else { level * level };
        let pot = if sd1 { 10_000.0 } else { 100_000.0 };
        self.level.begin();
        self.level
            .resistor(N(0), N(1), if sd1 { 4_700.0 } else { 1_000.0 });
        self.level.resistor(N(1), N(2), pot * (1.0 - level));
        self.level.resistor(N(2), G, pot * level);
        self.level
            .resistor(N(2), N(3), if sd1 && !od1 { 22_000.0 } else { CONTACT });
        if sd1 && !od1 {
            self.level.resistor(N(3), G, 22_000.0);
        }
        self.level
            .resistor(N(4), G, if sd1 { 1_000_000.0 } else { 510_000.0 });
        self.level.finish();
    }
    fn tick(&mut self, x: f64) -> f64 {
        if let Some(clone) = &mut self.clone {
            return clone.tick(x);
        }
        if let Some(rat) = &mut self.rat {
            return rat.tick(x);
        }
        let x = self.input.tick(x).clamp(-3.0, 3.0);
        let x = self.coupling.tick(x);
        let grounded =
            (self.ground_cap * x + self.ground_history) / (self.ground_cap + 1.0 / 4_700.0);
        let current = grounded / 4_700.0;
        self.ground_history = -2.0 * self.ground_cap * (x - grounded) - self.ground_history;
        let delta = feedback_voltage(
            current + self.feedback_history,
            self.resistance.recip() + self.feedback_cap,
            self.variant,
        );
        let clipped = (x + delta).clamp(-3.0, 3.0);
        self.feedback_history = 2.0 * self.feedback_cap * (clipped - x) - self.feedback_history;
        let toned = if self.variant == Variant::Od1 {
            self.od_tone.tick(clipped).clamp(-3.0, 3.0)
        } else {
            let nodes = self.tone.solve([clipped, 0.0]);
            self.tone.commit(&nodes, [clipped, 0.0]);
            nodes[4].clamp(-3.0, 3.0)
        };
        let nodes = self.level.solve([toned, 0.0]);
        self.level.commit(&nodes, [toned, 0.0]);
        self.output.tick(nodes[4].clamp(-3.0, 3.0))
    }
}

pub struct Dist {
    rate: f64,
    controls: [f64; 3],
    targets: [f64; 3],
    until_update: usize,
    mode: Oversampling,
    requested: Oversampling,
    variant: Variant,
    requested_variant: Variant,
    circuits: [Circuit; 2],
    resamplers: [Resampler; 2],
    taps: [f64; TAPS],
    fade: f64,
    off_delay: [[f64; 32]; 2],
    cursor: usize,
}
impl Dist {
    pub fn new(rate: f32, c: &Controls, variant: Variant, mode: Oversampling) -> Self {
        let rate = f64::from(rate);
        assert!(rate.is_finite() && rate >= 8_000.0);
        let values = [position(c.drive), position(c.tone), position(c.level)];
        Self {
            rate,
            controls: values,
            targets: values,
            until_update: 0,
            mode,
            requested: mode,
            variant,
            requested_variant: variant,
            circuits: std::array::from_fn(|_| {
                Circuit::new(rate * mode.factor() as f64, values, variant)
            }),
            resamplers: std::array::from_fn(|_| Resampler::new()),
            taps: fir(mode),
            fade: 1.0,
            off_delay: [[0.0; 32]; 2],
            cursor: 0,
        }
    }
    pub fn latency(&self) -> u32 {
        LATENCY
    }
    pub fn set_controls(&mut self, c: &Controls, variant: Variant, mode: Oversampling) {
        self.targets = [position(c.drive), position(c.tone), position(c.level)];
        self.requested = mode;
        self.requested_variant = variant;
    }
    pub fn reset(&mut self) {
        let controls = Controls {
            drive: self.targets[0] as f32,
            tone: self.targets[1] as f32,
            level: self.targets[2] as f32,
        };
        *self = Self::new(
            self.rate as f32,
            &controls,
            self.requested_variant,
            self.requested,
        );
    }
    pub fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
        if self.requested != self.mode || self.requested_variant != self.variant {
            self.fade = (self.fade - 1.0 / 64.0).max(0.0);
            if self.fade == 0.0 {
                self.mode = self.requested;
                self.variant = self.requested_variant;
                self.circuits = std::array::from_fn(|_| {
                    Circuit::new(
                        self.rate * self.mode.factor() as f64,
                        self.controls,
                        self.variant,
                    )
                });
                self.resamplers = std::array::from_fn(|_| Resampler::new());
                self.taps = fir(self.mode);
                self.off_delay = [[0.0; 32]; 2];
                self.cursor = 0;
            }
        } else {
            self.fade = (self.fade + 1.0 / 64.0).min(1.0);
        }
        if self.until_update == 0 {
            let mix = 1.0 - (-16.0 / (0.010 * self.rate)).exp();
            let mut changed = false;
            for i in 0..3 {
                let error = self.targets[i] - self.controls[i];
                if error != 0.0 {
                    self.controls[i] = if error.abs() < 1e-8 {
                        self.targets[i]
                    } else {
                        self.controls[i] + mix * error
                    };
                    changed = true;
                }
            }
            if changed {
                for circuit in &mut self.circuits {
                    circuit.tune(self.controls);
                }
            }
            self.until_update = 16;
        }
        self.until_update -= 1;
        let mut out = [0.0; 2];
        for i in 0..2 {
            let x = if input[i].is_finite() {
                f64::from(input[i]).clamp(-64.0, 64.0)
            } else {
                0.0
            };
            let y = if self.mode == Oversampling::Off {
                let y = self.circuits[i].tick(x);
                let delayed = self.off_delay[i][self.cursor];
                self.off_delay[i][self.cursor] = y;
                delayed
            } else {
                self.resamplers[i].tick(
                    x,
                    &self.taps[..self.mode.taps()],
                    self.mode.factor(),
                    |x| self.circuits[i].tick(x),
                )
            };
            out[i] = (y * self.fade) as f32;
        }
        self.cursor = (self.cursor + 1) % 32;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clone_model_matches_existing_plugin_dsp_with_fixed_controls() {
        for mode in Oversampling::ALL {
            let c = Controls {
                drive: 0.7,
                tone: 0.3,
                level: 0.8,
            };
            let mut family = Dist::new(48000.0, &c, Variant::Clone, mode);
            let mut original = nice_common::clone_drive::CloneDrive::with_oversampling(
                48000.0,
                &nice_common::clone_drive::Controls {
                    gain: c.drive,
                    treble: c.tone,
                    output: c.level,
                },
                mode,
            );
            let mut delay = [[0.0; 2]; 32];
            for i in 0..4096 {
                let x = (i as f32 * 0.031).sin() * 0.3;
                let actual = family.process([x, -x]);
                let mut expected = original.process([x, -x]);
                if mode == Oversampling::Off {
                    std::mem::swap(&mut expected, &mut delay[i % 32]);
                }
                assert_eq!(actual, expected, "{mode:?}, frame {i}");
            }
        }
    }
    #[test]
    fn od1_tone_is_fixed_and_differs_from_sd1() {
        let mut a = Circuit::new(384000.0, [0.5, 0.0, 0.7], Variant::Od1);
        let mut b = Circuit::new(384000.0, [0.5, 1.0, 0.7], Variant::Od1);
        let mut sd = Circuit::new(384000.0, [0.5, 0.5, 0.7], Variant::Sd1);
        let mut difference = 0.0;
        for i in 0..32768 {
            let x = 0.2 * (i as f64 * std::f64::consts::TAU * 750.0 / 384000.0).sin();
            let y = a.tick(x);
            assert_eq!(y, b.tick(x));
            difference += (y - sd.tick(x)).abs();
        }
        assert!(difference > 1.0);
    }
    #[test]
    fn sd1_diode_strings_are_asymmetric_and_satisfy_kirchhoff() {
        let g = 1.0 / 1_033_000.0;
        for current in [-1.0, -0.01, -0.0001, -1e-12, 1e-12, 0.0001, 0.01, 1.0] {
            let v = feedback_voltage(current, g, Variant::Sd1);
            let solved = g * v + IS * ((v / (2.0 * NVT)).exp_m1() - (-v / NVT).exp_m1());
            assert!(
                (solved - current).abs() < 1e-11 * current.abs() + 1e-20,
                "{current}: {solved}"
            );
        }
        let positive = feedback_voltage(0.0001, g, Variant::Sd1);
        let negative = feedback_voltage(-0.0001, g, Variant::Sd1);
        assert!(positive > 1.8 * negative.abs());
    }
    #[test]
    fn silicon_feedback_solution_satisfies_kirchhoff() {
        for current in [-1.0, -0.01, -0.0001, 0.0, 0.0001, 0.01, 1.0] {
            let g = 1.0 / 551000.0 + 2.0 * 384000.0 * 51e-12;
            let v = feedback_voltage(current, g, Variant::Ts808);
            let solved = g * v + 2.0 * IS * (v / NVT).sinh();
            assert!((solved - current).abs() < 1e-10, "{current} {solved}");
        }
    }
    #[test]
    fn small_signal_matches_independent_complex_nodal_reference() {
        for line in include_str!("../tests/reference/ac.csv").lines().skip(1) {
            let p: Vec<f64> = line.split(',').map(|x| x.parse().unwrap()).collect();
            let mut c = Circuit::new(
                384000.0,
                [p[1], p[2], p[3]],
                Variant::from_index(p[4] as i32),
            );
            let (mut re, mut im) = (0.0, 0.0);
            let n = 32768;
            for i in 0..n * 16 {
                let phase = std::f64::consts::TAU * p[0] * i as f64 / 384000.0;
                let y = c.tick(1e-7 * phase.cos());
                if i >= n * 15 {
                    re += y * phase.cos();
                    im -= y * phase.sin();
                }
            }
            let h = [re * 2.0 / n as f64 / 1e-7, im * 2.0 / n as f64 / 1e-7];
            let error = ((h[0] - p[5]).powi(2) + (h[1] - p[6]).powi(2)).sqrt();
            assert!(
                error < 0.002 * p[5].hypot(p[6]),
                "{line}: {h:?}, error {error}"
            );
        }
    }
    #[test]
    fn reset_matches_fresh_and_stereo_remains_independent() {
        for mode in Oversampling::ALL {
            let c = Controls::default();
            for variant in Variant::ALL {
                let mut effect = Dist::new(48000.0, &c, variant, mode);
                for _ in 0..1000 {
                    effect.process([0.5, -0.3]);
                }
                effect.reset();
                let mut fresh = Dist::new(48000.0, &c, variant, mode);
                for _ in 0..512 {
                    let out = effect.process([0.25, 0.0]);
                    assert_eq!(out, fresh.process([0.25, 0.0]));
                    assert_eq!(out[1], 0.0);
                    assert!(out[0].is_finite());
                }
                assert_eq!(effect.latency(), 32);
            }
        }
    }
    #[test]
    fn moving_controls_and_quality_switches_stay_finite() {
        for rate in [8000.0, 44100.0, 48000.0, 96000.0, 192000.0] {
            let mut effect =
                Dist::new(rate, &Controls::default(), Variant::Ts808, Oversampling::X8);
            for i in 0..4096 {
                if i % 256 == 0 {
                    effect.set_controls(
                        &Controls {
                            drive: (i % 7) as f32 / 6.0,
                            tone: (i % 5) as f32 / 4.0,
                            level: 0.7,
                        },
                        Variant::ALL[(i / 256) % Variant::ALL.len()],
                        Oversampling::ALL[(i / 256) % 4],
                    );
                }
                let input = match i % 7 {
                    0 => f32::NAN,
                    1 => f32::INFINITY,
                    2 => -64.0,
                    _ => 0.2,
                };
                assert!(
                    effect
                        .process([input, -input])
                        .iter()
                        .all(|x| x.is_finite() && x.abs() < 16.0)
                );
            }
        }
    }
}

#[cfg(test)]
mod signal_tests {
    use super::*;
    fn spectrum(
        variant: Variant,
        mode: Oversampling,
        c: Controls,
        frequency: f64,
        amplitude: f64,
        bins: &[f64],
    ) -> Vec<f64> {
        let mut effect = Dist::new(48000.0, &c, variant, mode);
        let mut sums = vec![(0.0, 0.0); bins.len()];
        for i in 0..16384 {
            let t = i as f64 / 48000.0;
            let x = amplitude * (std::f64::consts::TAU * frequency * t).sin();
            let y = f64::from(effect.process([x as f32, 0.0])[0]);
            if i >= 8192 {
                for (sum, f) in sums.iter_mut().zip(bins) {
                    let phase = std::f64::consts::TAU * f * t;
                    sum.0 += y * phase.cos();
                    sum.1 += y * phase.sin();
                }
            }
        }
        sums.into_iter()
            .map(|(re, im)| re.hypot(im) * 2.0 / 8192.0)
            .collect()
    }
    #[test]
    fn sd1_produces_even_harmonics_unlike_the_symmetric_ts_clipper() {
        let c = Controls {
            drive: 0.7,
            tone: 0.5,
            level: 0.8,
        };
        let sd1 = spectrum(
            Variant::Sd1,
            Oversampling::X8,
            c,
            750.0,
            0.1,
            &[750.0, 1500.0],
        );
        let ts = spectrum(
            Variant::Ts808,
            Oversampling::X8,
            c,
            750.0,
            0.1,
            &[750.0, 1500.0],
        );
        assert!(sd1[1] / sd1[0] > 0.001, "SD-1: {sd1:?}");
        assert!(
            sd1[1] / sd1[0] > 100.0 * ts[1] / ts[0],
            "SD-1 {sd1:?}, TS808 {ts:?}"
        );
    }
    #[test]
    fn eightfold_oversampling_reduces_folded_clipping_harmonic() {
        let c = Controls {
            drive: 1.0,
            tone: 1.0,
            level: 0.8,
        };
        let off = spectrum(
            Variant::Ts808,
            Oversampling::Off,
            c,
            9000.0,
            0.7,
            &[9000.0, 21000.0],
        );
        let on = spectrum(
            Variant::Ts808,
            Oversampling::X8,
            c,
            9000.0,
            0.7,
            &[9000.0, 21000.0],
        );
        let reduction = 20.0 * ((off[1] / off[0]) / (on[1] / on[0])).log10();
        eprintln!("8x normalized alias reduction at 21 kHz: {reduction:.1} dB");
        assert!(reduction > 20.0, "Off {off:?}, 8x {on:?}");
    }
    #[test]
    fn drive_adds_distortion_and_level_can_mute() {
        let c = Controls {
            drive: 0.0,
            tone: 0.5,
            level: 0.8,
        };
        let low = spectrum(
            Variant::Ts808,
            Oversampling::X8,
            c,
            750.0,
            0.02,
            &[750.0, 2250.0],
        );
        let high = spectrum(
            Variant::Ts808,
            Oversampling::X8,
            Controls { drive: 1.0, ..c },
            750.0,
            0.02,
            &[750.0, 2250.0],
        );
        assert!(
            high[1] / high[0] > low[1] / low[0] * 2.0,
            "{low:?} {high:?}"
        );
        let muted = spectrum(
            Variant::Ts808,
            Oversampling::X8,
            Controls { level: 0.0, ..c },
            750.0,
            0.02,
            &[750.0],
        );
        assert!(muted[0] < low[0] * 0.0001, "{muted:?}");
    }
}
