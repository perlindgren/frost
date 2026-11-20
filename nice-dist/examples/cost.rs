//! Informational standalone DSP benchmark; no audio device or window required.
use nice_common::oversample::Oversampling;
use nice_dist::{
    controls::Variant,
    dsp::{Controls, Dist},
};
fn main() {
    for variant in Variant::ALL {
        for mode in Oversampling::ALL {
            let mut effect = Dist::new(48000.0, &Controls::default(), variant, mode);
            let samples: Vec<_> = (0..48000)
                .map(|i| 0.3 * (std::f32::consts::TAU * 750.0 * i as f32 / 48000.0).sin())
                .collect();
            for x in &samples[..1024] {
                effect.process([*x, -*x]);
            }
            let mut best = f64::INFINITY;
            for _ in 0..3 {
                let now = std::time::Instant::now();
                for x in &samples {
                    std::hint::black_box(effect.process(std::hint::black_box([*x, -*x])));
                }
                best = best.min(now.elapsed().as_secs_f64());
            }
            println!(
                "{} {}: {:.2} ms per stereo audio second ({:.2}% of one core)",
                variant.label(),
                mode.label(),
                best * 1000.0,
                best * 100.0
            );
        }
    }
}
