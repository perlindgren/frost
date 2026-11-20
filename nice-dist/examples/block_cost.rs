//! 64-frame, mono, 48 kHz, 8x DSP timing; stdout is CSV.
//! Includes the per-block control update and output stores, but not plugin
//! metering, device callbacks, resampling between devices, or GUI rendering.
use nice_common::oversample::Oversampling;
use nice_dist::{
    controls::Variant,
    dsp::{Controls, Dist},
};
use std::hint::black_box;
const FRAMES: usize = 64;
const RATE: f32 = 48_000.0;
const BLOCKS: usize = 8192;
fn block(effect: &mut Dist, variant: Variant, controls: &Controls, input: &[f32; FRAMES]) {
    effect.set_controls(black_box(controls), variant, Oversampling::X8);
    let mut output = [0.0; FRAMES];
    for (out, sample) in output.iter_mut().zip(black_box(input)) {
        *out = effect.process(*sample);
    }
    black_box(output);
}
fn main() {
    let controls = Controls::default();
    // Exactly one cycle of 750 Hz per block, on the box's one mono lane.
    let input = std::array::from_fn(|i| {
        (0.3 * (std::f64::consts::TAU * i as f64 / FRAMES as f64).sin()) as f32
    });
    let budget_us = FRAMES as f64 / f64::from(RATE) * 1e6;
    println!(
        "model,frames,sample_rate,oversampling,blocks,min_us,median_us,p95_us,p99_us,max_us,median_budget_percent,p95_budget_percent"
    );
    for variant in Variant::ALL {
        let mut effect = Dist::new(RATE, &controls, variant, Oversampling::X8);
        for _ in 0..512 {
            block(&mut effect, variant, &controls, &input);
        }
        let mut times = Vec::with_capacity(BLOCKS);
        for _ in 0..BLOCKS {
            let start = std::time::Instant::now();
            block(&mut effect, variant, &controls, &input);
            times.push(start.elapsed().as_secs_f64() * 1e6);
        }
        times.sort_by(f64::total_cmp);
        let median = times[BLOCKS / 2];
        let p95 = times[BLOCKS * 95 / 100];
        println!(
            "{},{FRAMES},{RATE},8,{BLOCKS},{:.3},{median:.3},{p95:.3},{:.3},{:.3},{:.3},{:.3}",
            variant.label(),
            times[0],
            times[BLOCKS * 99 / 100],
            times[BLOCKS - 1],
            median / budget_us * 100.0,
            p95 / budget_us * 100.0
        );
    }
}
