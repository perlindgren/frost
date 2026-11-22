//! Separate binary to avoid combining nice-plug's allocator and the counter.
#[allow(dead_code)]
#[path = "../src/controls.rs"]
mod controls;
#[allow(dead_code)]
#[path = "../src/dsp.rs"]
mod dsp;
use nice_common::oversample::Oversampling;
#[test]
fn processing_switching_and_reset_do_not_allocate_or_free() {
    let mut effect = dsp::Dist::new(
        48000.0,
        &dsp::Controls::default(),
        controls::Variant::Bs808,
        Oversampling::X8,
    );
    let count = allocation_counter::measure(|| {
        for i in 0..2048 {
            if i % 256 == 0 {
                effect.set_controls(
                    &dsp::Controls {
                        drive: 0.7,
                        tone: 0.3,
                        level: 0.9,
                    },
                    controls::Variant::ALL[(i / 256) % controls::Variant::ALL.len()],
                    Oversampling::ALL[(i / 256) % 4],
                );
            }
            std::hint::black_box(effect.process(0.3));
        }
        effect.reset();
    });
    assert_eq!(count.count_total, 0);
    assert_eq!(count.count_current, 0);
}
