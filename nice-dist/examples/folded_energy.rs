//! Coherent single-tone alias probe for the nominal TS808/TS9 models.
//! Run in release; stdout is CSV. Not a broadband or hardware measurement.
use nice_common::oversample::Oversampling;
use nice_dist::{
    controls::Variant,
    dsp::{Controls, Dist},
};
const RATE: f64 = 48_000.0;
const FRAMES: usize = 32768;
const FREQUENCIES: [f64; 4] = [3000.0, 9000.0, 15000.0, 21000.0];
fn measure(variant: Variant, mode: Oversampling) -> [f64; 4] {
    let controls = Controls {
        drive: 1.0,
        tone: 1.0,
        level: 0.8,
    };
    let mut effect = Dist::new(RATE as f32, &controls, variant, mode);
    // A 9 kHz sine at 48 kHz repeats every 16 samples. Reuse exactly one
    // period to avoid accumulating oscillator error or spectral leakage.
    let period: [f32; 16] = std::array::from_fn(|i| {
        (0.7 * (std::f64::consts::TAU * 9000.0 * i as f64 / RATE).sin()) as f32
    });
    let mut sums = [(0.0, 0.0); 4];
    // Settle for 0.683 s, then measure 0.683 s (6144 complete cycles).
    for i in 0..FRAMES * 2 {
        let y = f64::from(effect.process([period[i % 16], 0.0])[0]);
        if i >= FRAMES {
            for (sum, frequency) in sums.iter_mut().zip(FREQUENCIES) {
                let angle = std::f64::consts::TAU * frequency * (i % 16) as f64 / RATE;
                sum.0 += y * angle.cos();
                sum.1 -= y * angle.sin();
            }
        }
    }
    sums.map(|(re, im)| re.hypot(im) * 2.0 / FRAMES as f64)
}
fn main() {
    println!(
        "model,oversampling,fundamental_peak,folded_power,folded_dbc,reduction_db,folded_21k_dbc"
    );
    for variant in [Variant::Ts808, Variant::Ts9] {
        let mut baseline = 0.0;
        for mode in Oversampling::ALL {
            let a = measure(variant, mode);
            // With this symmetric model and stimulus, odd harmonics above
            // Nyquist fold to 3/15/21 kHz, or onto the fundamental itself.
            // The latter cannot be separated and is excluded, as are even
            // harmonics, DC, and nonstationary/noise artifacts.
            let power = (a[0].powi(2) + a[2].powi(2) + a[3].powi(2)) * 0.5;
            let relative = 10.0 * (power / (0.5 * a[1].powi(2))).log10();
            if mode == Oversampling::Off {
                baseline = relative;
            }
            println!(
                "{},{},{:.9},{:.12e},{:.3},{:.3},{:.3}",
                variant.label(),
                mode.factor(),
                a[1],
                power,
                relative,
                baseline - relative,
                20.0 * (a[3] / a[1]).log10()
            );
        }
    }
}
