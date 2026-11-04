use pitch_detection::detector::PitchDetector as DetectionTrait;
use pitch_detector::pitch::PitchDetector as FftPitchTrait;
use pitch_estimate::PitchDetector as EstimateTrait;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    hint::black_box,
    time::Instant,
};
thread_local! { static COUNT:Cell<bool>=const{Cell::new(false)}; static ALLOCS:Cell<usize>=const{Cell::new(0)}; static BYTES:Cell<usize>=const{Cell::new(0)}; }
struct Allocator;
fn record(bytes: usize) {
    if COUNT.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCS.try_with(|n| n.set(n.get() + 1));
        let _ = BYTES.try_with(|n| n.set(n.get() + bytes));
    }
}
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        record(l.size());
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        record(l.size());
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        record(n);
        unsafe { System.realloc(p, l, n) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
const N: usize = 2048;
const HOP: usize = 1024;
const FACTOR: usize = 4;
const BLOCK: usize = 256;
#[derive(Clone, Copy)]
enum Kind {
    Local,
    EYin,
    EMpm,
    DYin,
    DMpm,
    Fft,
    Cep,
    Bcf,
    BcfNative,
    DYinUngated,
}
const KINDS: [Kind; 10] = [
    Kind::Local,
    Kind::EYin,
    Kind::EMpm,
    Kind::DYin,
    Kind::DMpm,
    Kind::Fft,
    Kind::Cep,
    Kind::Bcf,
    Kind::BcfNative,
    Kind::DYinUngated,
];
impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Local => "local_yin",
            Self::EYin => "estimate_yin",
            Self::EMpm => "estimate_mpm",
            Self::DYin => "detection_yin",
            Self::DMpm => "detection_mpm",
            Self::Fft => "detector_fft",
            Self::Cep => "detector_cepstrum",
            Self::Bcf => "registry_bcf_decimated",
            Self::BcfNative => "registry_bcf_native",
            Self::DYinUngated => "detection_yin_ungated",
        }
    }
}
enum Engine {
    Fft(pitch_detector::pitch::HannedFftDetector),
    Cep(pitch_detector::pitch::PowerCepstrum),
    Bcf,
    BcfNative,
    Local(local_pitch::PitchDetector),
    EYin(pitch_estimate::YinDetector<f32>),
    EMpm(pitch_estimate::McLeodDetector<f32>),
    DYin(pitch_detection::detector::yin::YINDetector<f32>),
    DMpm(pitch_detection::detector::mcleod::McLeodDetector<f32>),
}
struct Stream {
    ignore_clarity: bool,
    engine: Engine,
    rate: f64,
    window: [f32; N],
    wide: [f64; N],
    native: Vec<f64>,
    native_filled: usize,
    filled: usize,
    phase: usize,
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}
impl Stream {
    fn new(kind: Kind, sr: u32) -> Self {
        let rate = sr as f64 / FACTOR as f64;
        let lag_max = (rate / 30.0).floor() as usize;
        let engine = match kind {
            Kind::Fft => Engine::Fft(Default::default()),
            Kind::Cep => Engine::Cep(Default::default()),
            Kind::Bcf => Engine::Bcf,
            Kind::BcfNative => Engine::BcfNative,
            Kind::Local => Engine::Local(local_pitch::PitchDetector::new(local_pitch::Config {
                sample_rate: sr as f64,
                min_freq: 30.0,
                max_freq: 2100.0,
                downsample_factor: FACTOR,
                min_clarity: 0.85,
            })),
            Kind::EYin => Engine::EYin(
                pitch_estimate::YinDetector::new(N, N - lag_max)
                    .unwrap()
                    .with_absolute_threshold(0.1),
            ),
            Kind::EMpm => Engine::EMpm(
                pitch_estimate::McLeodDetector::new(N, N / 2)
                    .unwrap()
                    .with_clarity_threshold(0.85),
            ),
            Kind::DYin | Kind::DYinUngated => {
                Engine::DYin(pitch_detection::detector::yin::YINDetector::new(N, N))
            }
            Kind::DMpm => {
                Engine::DMpm(pitch_detection::detector::mcleod::McLeodDetector::new(N, N))
            }
        };
        // Match the current detector's actual preprocessing, including its f64 IIR.
        let w = std::f64::consts::TAU * (rate * 0.45) / (sr as f64);
        let c = w.cos();
        let alpha = w.sin() * std::f64::consts::FRAC_1_SQRT_2;
        let a0 = 1.0 + alpha;
        Self {
            ignore_clarity: matches!(kind, Kind::DYinUngated),
            engine,
            rate,
            window: [0.0; N],
            wide: [0.0; N],
            native: if matches!(kind, Kind::BcfNative) {
                vec![0.0; N * FACTOR]
            } else {
                Vec::new()
            },
            native_filled: 0,
            filled: 0,
            phase: 0,
            b: [(1.0 - c) * 0.5 / a0, (1.0 - c) / a0, (1.0 - c) * 0.5 / a0],
            a: [-2.0 * c / a0, (1.0 - alpha) / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }
    fn push(&mut self, samples: &[f32]) -> Option<(f64, f64)> {
        if let Engine::Local(det) = &mut self.engine {
            return det.push_samples(samples).map(|d| (d.freq_hz, d.clarity));
        }
        let mut last = None;
        if matches!(self.engine, Engine::BcfNative) {
            for &sample in samples {
                self.native[self.native_filled] = sample as f64;
                self.native_filled += 1;
                if self.native_filled == self.native.len() {
                    let hz = registry_pitch::detect(&self.native).0 * self.rate * FACTOR as f64
                        / 48000.0;
                    last = if hz.is_finite() && (30.0..=2100.0).contains(&hz) {
                        Some((hz, 1.0))
                    } else {
                        None
                    };
                    self.native.copy_within(HOP * FACTOR.., 0);
                    self.native_filled = HOP * FACTOR;
                }
            }
            return last;
        }
        for &sample in samples {
            let input = sample as f64;
            let value = self.b[0] * input + self.b[1] * self.x[0] + self.b[2] * self.x[1]
                - self.a[0] * self.y[0]
                - self.a[1] * self.y[1];
            self.x[1] = self.x[0];
            self.x[0] = input;
            self.y[1] = self.y[0];
            self.y[0] = value;
            self.phase += 1;
            if self.phase < FACTOR {
                continue;
            }
            self.phase = 0;
            self.window[self.filled] = value as f32;
            self.filled += 1;
            if self.filled == N {
                if matches!(self.engine, Engine::Fft(_) | Engine::Cep(_) | Engine::Bcf) {
                    for (out, &input) in self.wide.iter_mut().zip(&self.window) {
                        *out = input as f64;
                    }
                }
                let pitch = match &mut self.engine {
                    Engine::Fft(det) => det
                        .detect_pitch_in_range(&self.wide, self.rate, 30.0..2100.0)
                        .map(|hz| (hz, 1.0)),
                    Engine::Cep(det) => det
                        .detect_pitch_in_range(&self.wide, self.rate, 30.0..2100.0)
                        .map(|hz| (hz, 1.0)),
                    Engine::Bcf => Some((
                        registry_pitch::detect(&self.wide).0 * self.rate / 48000.0,
                        1.0,
                    )),
                    Engine::EYin(det) => det
                        .detect(&self.window, self.rate as u32)
                        .map(|p| (p.frequency as f64, p.clarity as f64)),
                    Engine::EMpm(det) => det
                        .detect(&self.window, self.rate as u32)
                        .map(|p| (p.frequency as f64, p.clarity as f64)),
                    Engine::DYin(det) => det
                        .get_pitch(&self.window, self.rate as usize, 0.0, 0.85)
                        .map(|p| (p.frequency as f64, p.clarity as f64)),
                    Engine::DMpm(det) => det
                        .get_pitch(&self.window, self.rate as usize, 0.0, 0.85)
                        .map(|p| (p.frequency as f64, p.clarity as f64)),
                    _ => unreachable!(),
                };
                last = pitch.filter(|(hz, c)| {
                    hz.is_finite()
                        && *hz >= 30.0
                        && *hz <= 2100.0
                        && c.is_finite()
                        && (self.ignore_clarity || (*c >= 0.85 && *c <= 1.0 + 1e-5))
                });
                self.window.copy_within(HOP..N, 0);
                self.filled = HOP;
            }
        }
        last
    }
}
fn random(seed: &mut u64) -> f64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*seed >> 40) as f64 / (1u64 << 23) as f64) - 1.0
}
fn signal(sr: u32, f: f64, shape: &str, seconds: f64) -> Vec<f32> {
    let n = (sr as f64 * seconds) as usize;
    let mut seed = 0x3141592653589793;
    let mut data = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / sr as f64;
        let p = std::f64::consts::TAU * f * t;
        let fundamental = if shape == "sharp7" {
            (p * 2f64.powf(7.0 / 1200.0)).sin()
        } else {
            p.sin()
        };
        let value = match shape {
            "sine" | "sharp7" => 0.5 * fundamental,
            "harmonics" | "noise_low" | "noise_high" | "pluck" => {
                let s = (1..=7)
                    .map(|k| (p * k as f64).sin() / k as f64)
                    .sum::<f64>()
                    / 2.593;
                let amp = if shape == "pluck" {
                    (t / 0.006).min(1.0) * (-2.0 * t).exp()
                } else {
                    1.0
                };
                let noise = match shape {
                    "noise_low" => 0.025,
                    "noise_high" => 0.08,
                    _ => 0.0,
                };
                0.6 * s * amp + noise * random(&mut seed)
            }
            "weak_fundamental" => {
                0.01 * fundamental
                    + 0.35 * (2.0 * p).sin()
                    + 0.22 * (3.0 * p).sin()
                    + 0.12 * (4.0 * p).sin()
            }
            "missing_fundamental" => {
                0.35 * (2.0 * p).sin() + 0.22 * (3.0 * p).sin() + 0.12 * (4.0 * p).sin()
            }
            "dc_tone" => 0.6 + 0.05 * fundamental,
            "silence" => 0.0,
            "noise" => 0.2 * random(&mut seed),
            "dc" => 0.5,
            "nan" => f64::NAN,
            "infinity" => f64::INFINITY,
            "overflow" => fundamental * 1e30,
            _ => unreachable!(),
        };
        data.push(value as f32);
    }
    data
}
fn quantile(data: &mut [f64], p: f64) -> f64 {
    if data.is_empty() {
        return f64::NAN;
    }
    data.sort_by(f64::total_cmp);
    data[((data.len() - 1) as f64 * p).round() as usize]
}
fn accuracy() {
    println!(
        "section,engine,sr,case,expected_hz,opportunities,detected,within5c,octave_errors,median_abs_cents,p95_abs_cents,first_correct_ms"
    );
    let notes = [
        30.867706,
        41.203445,
        55.0,
        65.406391,
        82.406889,
        110.0,
        146.832384,
        196.0,
        261.625565,
        329.627557,
        440.0,
        659.255114,
        1046.502261,
        1567.981744,
        2090.0,
    ];
    for sr in [44100, 48000] {
        for shape in [
            "sine",
            "sharp7",
            "harmonics",
            "weak_fundamental",
            "missing_fundamental",
            "noise_low",
            "noise_high",
            "pluck",
            "dc_tone",
        ] {
            for f in notes {
                let input = signal(sr, f, shape, 0.85);
                let expected = if shape == "sharp7" {
                    f * 2f64.powf(7.0 / 1200.0)
                } else {
                    f
                };
                for kind in KINDS {
                    let mut stream = Stream::new(kind, sr);
                    let (mut count, mut correct, mut octaves) = (0, 0, 0);
                    let mut errors = Vec::new();
                    let mut first = f64::NAN;
                    for (i, chunk) in input.chunks(BLOCK).enumerate() {
                        if let Some((hz, _)) = stream.push(chunk) {
                            count += 1;
                            let error = 1200.0 * (hz / expected).log2();
                            errors.push(error.abs());
                            if error.abs() <= 5.0 {
                                correct += 1;
                                if first.is_nan() {
                                    first = ((i + 1) * BLOCK) as f64 / sr as f64 * 1000.0;
                                }
                            }
                            if (error.abs() - 1200.0).abs() < 50.0
                                || (error.abs() - 2400.0).abs() < 50.0
                            {
                                octaves += 1;
                            }
                        }
                    }
                    let opp = if input.len() / FACTOR < N {
                        0
                    } else {
                        1 + (input.len() / FACTOR - N) / HOP
                    };
                    let median = quantile(&mut errors, 0.5);
                    let p95 = quantile(&mut errors, 0.95);
                    println!(
                        "accuracy,{},{},{},{:.6},{},{},{},{},{:.6},{:.6},{:.3}",
                        kind.name(),
                        sr,
                        shape,
                        expected,
                        opp,
                        count,
                        correct,
                        octaves,
                        median,
                        p95,
                        first
                    );
                }
            }
        }
    }
    for sr in [44100, 48000] {
        for shape in ["silence", "noise", "dc", "nan", "infinity", "overflow"] {
            let input = signal(sr, 110.0, shape, 0.85);
            for kind in KINDS {
                let mut stream = Stream::new(kind, sr);
                let mut count = 0;
                for chunk in input.chunks(BLOCK) {
                    if stream.push(chunk).is_some() {
                        count += 1;
                    }
                }
                let opp = 1 + (input.len() / FACTOR - N) / HOP;
                println!(
                    "negative,{},{},{},0,{},{},0,0,0,0,0",
                    kind.name(),
                    sr,
                    shape,
                    opp,
                    count
                );
            }
        }
    }
}
fn performance() {
    println!(
        "section,engine,sr,trial,audio_seconds,cpu_ms,active_median_us,active_p95_us,allocations,allocated_bytes,analysis_calls,detections"
    );
    for sr in [44100, 48000] {
        let input = signal(sr, 110.0, "harmonics", 12.0);
        for trial in 0..5 {
            for offset in 0..9 {
                let kind = KINDS[(trial + offset) % 9];
                let mut stream = Stream::new(kind, sr);
                // Warm all lazy FFT pools, then reset counters without reconstructing.
                for chunk in input[..(sr as usize)].chunks(BLOCK) {
                    black_box(stream.push(chunk));
                }
                let mut times = Vec::with_capacity(300);
                let mut detections = 0;
                let mut sample_count = sr as usize;
                let mut analyses = 0;
                ALLOCS.with(|n| n.set(0));
                BYTES.with(|n| n.set(0));
                COUNT.with(|c| c.set(true));
                let start = Instant::now();
                for chunk in input.chunks(BLOCK) {
                    let before = sample_count / FACTOR;
                    let tick = Instant::now();
                    let result = stream.push(black_box(chunk));
                    let elapsed = tick.elapsed().as_secs_f64() * 1e6;
                    sample_count += chunk.len();
                    let after = sample_count / FACTOR;
                    let active =
                        after >= N && (before < N || (after - N) / HOP > (before - N) / HOP);
                    if active {
                        times.push(elapsed);
                        analyses += 1;
                    }
                    if black_box(result).is_some() {
                        detections += 1;
                    }
                }
                let total = start.elapsed().as_secs_f64() * 1000.0;
                COUNT.with(|c| c.set(false));
                let alloc = ALLOCS.with(Cell::get);
                let bytes = BYTES.with(Cell::get);
                let median = quantile(&mut times, 0.5);
                let p95 = quantile(&mut times, 0.95);
                println!(
                    "performance,{},{},{},12,{:.6},{:.6},{:.6},{},{},{},{}",
                    kind.name(),
                    sr,
                    trial,
                    total,
                    median,
                    p95,
                    alloc,
                    bytes,
                    analyses,
                    detections
                );
            }
        }
    }
}
fn cold_allocations() {
    println!("section,engine,sr,allocations,allocated_bytes");
    for sr in [44100, 48000] {
        for kind in KINDS {
            let mut stream = Stream::new(kind, sr);
            let input = signal(sr, 110.0, "sine", 0.2);
            ALLOCS.with(|n| n.set(0));
            BYTES.with(|n| n.set(0));
            COUNT.with(|c| c.set(true));
            for chunk in input.chunks(BLOCK) {
                black_box(stream.push(chunk));
            }
            COUNT.with(|c| c.set(false));
            println!(
                "cold_allocations,{},{},{},{}",
                kind.name(),
                sr,
                ALLOCS.with(Cell::get),
                BYTES.with(Cell::get)
            );
        }
    }
}
fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    match std::env::args().nth(1).as_deref() {
        Some("accuracy") => accuracy(),
        Some("performance") => performance(),
        Some("allocations") => cold_allocations(),
        Some("probe") => {
            for f in [30.867706, 82.406889, 110.0, 440.0, 2093.0] {
                let input: Vec<f32> = (0..N)
                    .map(|i| (std::f64::consts::TAU * f * i as f64 / 12000.0).sin() as f32)
                    .collect();
                let mut det = pitch_detection::detector::yin::YINDetector::new(N, N);
                let value = det.get_pitch(&input, 12000, 0.0, 0.85);
                println!("{f}: {:?}", value.map(|p| (p.frequency, p.clarity)));
            }
        }
        _ => panic!("mode"),
    }
}
