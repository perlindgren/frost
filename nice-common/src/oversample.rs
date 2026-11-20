//! Shared allocation-free FIR up/downsampling for nonlinear effects.
//! Linear-phase round-trip latency is 32 host samples at 2x/4x/8x.
pub const TAPS: usize = 257; // Storage for the largest (8x) filter.

/// Stable host parameter order. Existing sessions default to the original 8x.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Oversampling {
    Off,
    X2,
    X4,
    #[default]
    X8,
}
impl Oversampling {
    pub const ALL: [Self; 4] = [Self::Off, Self::X2, Self::X4, Self::X8];
    pub fn from_index(index: i32) -> Self {
        match index {
            0 => Self::Off,
            1 => Self::X2,
            2 => Self::X4,
            _ => Self::X8,
        }
    }
    pub fn from_factor(factor: u32) -> Self {
        match factor {
            1 => Self::Off,
            2 => Self::X2,
            4 => Self::X4,
            _ => Self::X8,
        }
    }
    pub fn index(self) -> i32 {
        self as i32
    }
    pub fn factor(self) -> usize {
        1 << self.index()
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::X2 => "2×",
            Self::X4 => "4×",
            Self::X8 => "8×",
        }
    }
    pub fn latency(self) -> u32 {
        if self == Self::Off { 0 } else { 32 }
    }
    pub fn taps(self) -> usize {
        32 * self.factor() + 1
    }
}
#[derive(Clone)]
pub struct Resampler {
    up: [f64; 33],
    down: [f64; TAPS],
    up_at: usize,
    down_at: usize,
}
impl Resampler {
    pub fn new() -> Self {
        Self {
            up: [0.0; 33],
            down: [0.0; TAPS],
            up_at: 0,
            down_at: 0,
        }
    }
    /// Process one host sample using a preset FIR and its matching factor.
    /// Use `fir(mode)[..mode.taps()]` with `mode.factor()` for 2x/4x/8x;
    /// Off bypasses this resampler. The callback runs at the elevated rate.
    pub fn tick(
        &mut self,
        sample: f64,
        taps: &[f64],
        factor: usize,
        mut process: impl FnMut(f64) -> f64,
    ) -> f64 {
        self.up[self.up_at] = sample;
        let mut output = 0.0;
        for phase in 0..factor {
            let mut upsampled = 0.0;
            let mut j = self.up_at;
            for k in (phase..taps.len()).step_by(factor) {
                upsampled += factor as f64 * taps[k] * self.up[j];
                j = if j == 0 { 32 } else { j - 1 };
            }
            self.down[self.down_at] = process(upsampled);
            if phase == 0 {
                let mut j = self.down_at;
                for &tap in taps {
                    output += tap * self.down[j];
                    j = if j == 0 { taps.len() - 1 } else { j - 1 };
                }
            }
            self.down_at = (self.down_at + 1) % taps.len();
        }
        self.up_at = (self.up_at + 1) % 33;
        output
    }
}
pub fn fir(mode: Oversampling) -> [f64; TAPS] {
    let factor = mode.factor();
    let count = mode.taps();
    let mut taps = [0.0; TAPS];
    for (i, tap) in taps[..count].iter_mut().enumerate() {
        let n = i as f64 - (count - 1) as f64 / 2.0;
        let sinc = if n == 0.0 {
            1.0 / factor as f64
        } else {
            (std::f64::consts::PI * n / factor as f64).sin() / (std::f64::consts::PI * n)
        };
        let theta = 2.0 * std::f64::consts::PI * i as f64 / (count - 1) as f64;
        *tap = sinc * (0.42 - 0.5 * theta.cos() + 0.08 * (2.0 * theta).cos());
    }
    let sum: f64 = taps.iter().sum();
    for tap in &mut taps {
        *tap /= sum;
    }
    taps
}

impl Default for Resampler {
    fn default() -> Self {
        Self::new()
    }
}
