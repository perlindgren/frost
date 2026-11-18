//! Include DSP alone: nice-plug's debug allocator and allocation-counter may
//! not coexist in one binary. This is the same arrangement as nice-wah.
#[allow(dead_code)]
#[path = "../src/dsp.rs"]
mod dsp;

#[test]
fn processing_and_control_updates_allocate_nothing_including_first_sample() {
    let mut drive = dsp::CloneDrive::new(48_000.0, &dsp::Controls::default());
    let allocations = allocation_counter::measure(|| {
        for i in 0..2048 {
            if i % 256 == 0 {
                drive.request_oversampling(dsp::Oversampling::ALL[(i / 256) % 4]);
            }
            if i % 16 == 0 {
                let x = (i % 127) as f32 / 126.0;
                drive.set_controls(&dsp::Controls {
                    gain: x,
                    treble: 1.0 - x,
                    output: x,
                });
            }
            std::hint::black_box(drive.process([0.3, -0.2]));
        }
    });
    assert_eq!(allocations.count_total, 0);
}
