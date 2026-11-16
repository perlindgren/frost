//! What the true-peak estimator costs on the audio thread, measured rather
//! than estimated.
//!
//! `cargo run -q -p nice-common --example true_peak_cost --release` times
//! 4096 stereo blocks of 128 frames — about 22 s of audio at 48 kHz — per run,
//! seven runs, and reports the fastest. The baseline is the `max |x|` fold the
//! workspace used before `src/true_peak.rs` existed, over the same samples in
//! the same loop shape, so the ratio is the price of the feature and not the
//! price of a second pass over memory.
//!
//! The signal is a tone with a burst in it, per channel at a different rate,
//! so the peak is data-dependent: neither loop can shortcut, and the branch
//! pattern is the one a real meter sees. Every block handed to the loop and
//! every peak published by it goes through `black_box`, so nothing between the
//! first sample and the publish is dead code.
use nice_common::true_peak::TruePeak;
use std::hint::black_box;
use std::time::Instant;

/// Frames per block, the size every producer in the workspace processes.
const FRAMES: usize = 128;
/// Blocks per run: 4096 x 128 frames = 1048576 samples per channel.
const BLOCKS: usize = 4096;
/// Runs kept; the fastest is the one reported, so that a scheduler hiccup on
/// an idle machine does not read as a cost of the filter.
const RUNS: usize = 7;
/// Sample rate the core-share figures are quoted at.
const RATE: f64 = 48_000.0;

fn tone(freq: f64, seed: usize) -> Vec<f32> {
    (0..FRAMES * BLOCKS)
        .map(|i| {
            let t = i as f64;
            let carrier = (2.0 * std::f64::consts::PI * freq * t).sin() as f32;
            let lfo = (2.0 * std::f64::consts::PI * 0.0007 * t).sin() as f32;
            let burst = if (i + seed * 977) % 60_000 < 2_000 {
                0.5
            } else {
                0.0
            };
            (carrier * (0.55 + 0.35 * lfo) + burst).clamp(-1.0, 1.0)
        })
        .collect()
}

/// The estimator, run the way a producer runs it: one persistent stream per
/// channel, `process` per block, publish and clear.
fn measure_true_peak(channels: &[&[f32]]) -> (f64, f32) {
    let start = Instant::now();
    let mut published = 0.0f32;
    let mut streams = [TruePeak::new(), TruePeak::new()];
    for block in 0..BLOCKS {
        for (channel, stream) in channels.iter().zip(streams.iter_mut()) {
            let slice = &channel[block * FRAMES..(block + 1) * FRAMES];
            stream.process(black_box(slice));
            published = published.max(black_box(stream.take()));
        }
    }
    (start.elapsed().as_nanos() as f64, published)
}

/// The pre-existing metering: one fold per block per channel, the shape the
/// producers actually wrote. Left free of per-element `black_box`, this is the
/// loop LLVM turns into a vector max — which is the honest comparison, and the
/// honest reason the ratio below is large: a reduction has no dependency
/// between samples, while the estimator's history is a recurrence the width of
/// the tap count.
fn measure_sample_peak(channels: &[&[f32]]) -> (f64, f32) {
    let start = Instant::now();
    let mut published = 0.0f32;
    for block in 0..BLOCKS {
        for channel in channels {
            let slice = &channel[block * FRAMES..(block + 1) * FRAMES];
            let peak = black_box(slice).iter().fold(0.0f32, |m, x| m.max(x.abs()));
            published = published.max(black_box(peak));
        }
    }
    (start.elapsed().as_nanos() as f64, published)
}

fn main() {
    let left = tone(0.211, 0);
    let right = tone(0.173, 1);
    let channels = [left.as_slice(), right.as_slice()];

    let mut best_true = f64::INFINITY;
    let mut best_raw = f64::INFINITY;
    let mut read_true = 0.0;
    let mut read_raw = 0.0;
    for _ in 0..RUNS {
        let (ns, peak) = measure_true_peak(&channels);
        if ns < best_true {
            best_true = ns;
            read_true = peak;
        }
        let (ns, peak) = measure_sample_peak(&channels);
        if ns < best_raw {
            best_raw = ns;
            read_raw = peak;
        }
    }

    let samples = (FRAMES * BLOCKS * 2) as f64; // both channels
    let report = |name: &str, ns: f64, peak: f32| {
        let ns_per_sample = ns / samples;
        let ns_per_block = ns / BLOCKS as f64;
        // Both channels of one second of audio, against one second of a core.
        let core = ns_per_sample * 2.0 * RATE / 1e7;
        println!(
            "{name:<30} {:>8.2} ms   {:>6.3} ns/sample   \
             {:>7.1} ns per 128-frame stereo block   {:>6.4}% of one core at \
             48 kHz   peak {peak:.4}",
            ns / 1e6,
            ns_per_sample,
            ns_per_block,
            core
        );
    };
    println!(
        "{} x {}-frame stereo blocks, {} runs, fastest kept ({} samples per \
         channel per run)",
        BLOCKS,
        FRAMES,
        RUNS,
        FRAMES * BLOCKS
    );
    report("true peak (24 MAC + abs)", best_true, read_true);
    report("sample peak (raw |x|)", best_raw, read_raw);
    println!(
        "\n{:.1}x the raw fold; {:.1} ns per stereo block is {:.4} ms of the \
         {:.2} ms one block has at 48 kHz",
        best_true / best_raw,
        best_true / BLOCKS as f64,
        best_true / BLOCKS as f64 / 1e6,
        FRAMES as f64 / RATE * 1e3,
    );
}
