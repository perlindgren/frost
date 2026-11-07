//! Streaming, band-limited mono conversion owned by the standalone application.
//! The plugin itself always processes at the input device's rate.
use rtrb::Consumer;

const TAPS: usize = 128;
const PHASES: usize = 1024;

pub struct Monitor {
    input: Consumer<f32>,
    history: [f32; TAPS],
    head: usize,
    phase: f64,
    needed: usize,
    nominal_ratio: f64,
    ratio: f64,
    kernels: Box<[[f32; TAPS]]>,
    target: usize,
    tolerance: usize,
    started: bool,
    same_rate: bool,
}

impl Monitor {
    pub fn new(
        input: Consumer<f32>,
        input_rate: f64,
        output_rate: f64,
        target: usize,
        tolerance: usize,
    ) -> Self {
        assert!(input_rate.is_finite() && input_rate > 0.0);
        assert!(output_rate.is_finite() && output_rate > 0.0);
        let ratio = input_rate / output_rate;
        // Leave a transition band below the lower Nyquist limit. A Blackman
        // window suppresses aliases when downsampling; each phase has unity DC.
        let cutoff = (1.0 / ratio).min(1.0) * 0.94;
        let mut kernels = vec![[0.0; TAPS]; PHASES].into_boxed_slice();
        for (phase, kernel) in kernels.iter_mut().enumerate() {
            let fraction = phase as f64 / PHASES as f64;
            let mut sum = 0.0;
            for (tap, coefficient) in kernel.iter_mut().enumerate() {
                let distance = (TAPS / 2) as f64 - fraction - tap as f64;
                let x = distance * cutoff;
                let sinc = if x.abs() < 1e-12 {
                    1.0
                } else {
                    (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
                };
                let window_phase = std::f64::consts::PI * distance / (TAPS / 2) as f64;
                let window = 0.42 + 0.5 * window_phase.cos() + 0.08 * (2.0 * window_phase).cos();
                *coefficient = (cutoff * sinc * window) as f32;
                sum += *coefficient as f64;
            }
            for coefficient in kernel {
                *coefficient /= sum as f32;
            }
        }
        Self {
            input,
            history: [0.0; TAPS],
            head: 0,
            phase: 0.0,
            needed: 1,
            nominal_ratio: ratio,
            ratio,
            kernels,
            target,
            tolerance,
            started: false,
            same_rate: ratio == 1.0,
        }
    }

    /// Once per output callback: compensate slowly for independent hardware
    /// clocks. Leave normal callback-size jitter inside a deadband untouched.
    pub fn begin_block(&mut self) {
        let available = self.input.slots();
        if !self.started {
            if available < self.target {
                return;
            }
            self.started = true;
        }
        let error = available as f64 - self.target as f64;
        let correction = if error.abs() <= self.tolerance as f64 {
            0.0
        } else {
            (error.signum() * (error.abs() - self.tolerance as f64) / self.target.max(1) as f64
                * 0.001)
                .clamp(-0.002, 0.002)
        };
        self.ratio = self.nominal_ratio * (1.0 + correction);
    }

    pub fn next_sample(&mut self) -> f32 {
        if !self.started {
            return 0.0;
        }
        while self.needed > 0 {
            let Ok(sample) = self.input.pop() else {
                self.started = false;
                return 0.0;
            };
            self.head = (self.head + 1) % TAPS;
            self.history[self.head] = sample;
            self.needed -= 1;
        }
        let value = if self.same_rate {
            // One frame of lookahead allows smooth clock correction without
            // switching filter paths or changing latency mid-stream.
            let previous = self.history[(self.head + TAPS - 1) % TAPS];
            previous + (self.history[self.head] - previous) * self.phase as f32
        } else {
            let kernel = &self.kernels[((self.phase * PHASES as f64) as usize).min(PHASES - 1)];
            let mut value = 0.0;
            for (tap, coefficient) in kernel.iter().enumerate() {
                value += self.history[(self.head + TAPS - tap) % TAPS] * coefficient;
            }
            value
        };
        let advance = self.phase + self.ratio;
        self.needed = advance.floor() as usize;
        self.phase = advance.fract();
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn convert(input_rate: u32, output_rate: u32, frequency: f64) -> Vec<f32> {
        let (mut producer, consumer) = rtrb::RingBuffer::new(input_rate as usize + 1024);
        for i in 0..input_rate {
            producer
                .push(
                    (std::f64::consts::TAU * frequency * i as f64 / input_rate as f64).sin() as f32,
                )
                .unwrap();
        }
        let mut monitor = Monitor::new(
            consumer,
            input_rate as f64,
            output_rate as f64,
            1,
            usize::MAX,
        );
        monitor.begin_block();
        (0..output_rate - 128)
            .map(|_| monitor.next_sample())
            .collect()
    }
    #[test]
    fn conversion_preserves_pitch_and_level_in_both_directions() {
        for (input, output) in [(44100, 48000), (48000, 44100), (96000, 32000)] {
            let samples = convert(input, output, 440.0);
            let steady = &samples[512..];
            let crossings = steady
                .windows(2)
                .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
                .count();
            let frequency = crossings as f64 * output as f64 / steady.len() as f64;
            assert!(
                (frequency - 440.0).abs() < 2.0,
                "{input}->{output}: {frequency}"
            );
            let rms = (steady.iter().map(|x| (*x as f64).powi(2)).sum::<f64>()
                / steady.len() as f64)
                .sqrt();
            assert!((rms - 0.5f64.sqrt()).abs() < 0.01);
        }
    }
    #[test]
    fn downsampling_rejects_above_nyquist_instead_of_aliasing() {
        let samples = convert(96000, 32000, 24000.0);
        let rms = (samples[512..].iter().map(|x| x * x).sum::<f32>()
            / (samples.len() - 512) as f32)
            .sqrt();
        assert!(rms < 0.001, "aliased RMS: {rms}");
    }
    #[test]
    fn matching_rates_preserve_samples_and_underflow_is_silence() {
        let (mut producer, consumer) = rtrb::RingBuffer::new(8);
        for value in [0.25, -0.5, 0.75, 0.0] {
            producer.push(value).unwrap();
        }
        let mut monitor = Monitor::new(consumer, 48000.0, 48000.0, 1, 8);
        monitor.begin_block();
        assert_eq!(
            [
                monitor.next_sample(),
                monitor.next_sample(),
                monitor.next_sample(),
                monitor.next_sample(),
                monitor.next_sample()
            ],
            [0.0, 0.25, -0.5, 0.75, 0.0]
        );
    }
}
