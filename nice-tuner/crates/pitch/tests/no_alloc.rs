//! Hard gate: the *detection* path of the pitch detector performs zero heap
//! allocations once the detector is warm.
//!
//! The buffers (`window`, `d`, `cmnd`) are allocated in `PitchDetector::new`
//! (run by `TunerUi`, on the UI thread); the per-window analysis contract
//! then requires silence from the allocator on every push. This test enforces
//! that contract through a safe allocation-measurement API. The
//! complementary RT-side contract (audio thread → ring handoff) is gated by
//! nice-tuner/tests/rt_handoff_no_alloc.rs, and nice-plug's
//! `assert_process_allocs` guards the full `process()` callback at runtime.
//!
//! Counts are local to the measured thread and stay active in release builds.

#![forbid(unsafe_code)]

/// Feed `sample_rate`-Hz audio to the detector and assert the measured
/// stretch allocated nothing.
#[test]
fn detection_path_never_allocates_when_warm() {
    use pitch::{Config, PitchDetector};

    const SR: f32 = 44100.0;
    let cfg = Config {
        sample_rate: SR as f64,
        min_freq: 30.0,
        max_freq: 2100.0,
        ..Config::default()
    };
    let mut detector = PitchDetector::new(cfg);

    // 12 s of a 110 Hz sine with a touch of harmonic content and a slow
    // amplitude ramp — plenty of completed windows and varying RMS.
    let n = (SR * 12.0) as usize;
    let signal: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            let env = 0.4 + 0.3 * (std::f32::consts::TAU * 0.5 * t).sin();
            env * ((std::f32::consts::TAU * 110.0 * t).sin()
                + 0.4 * (std::f32::consts::TAU * 220.0 * t).sin())
        })
        .collect();

    // Warm-up stretch: first passes may touch lazily initialized state.
    // (Detector buffers themselves were allocated in `new`.)
    let warm = (SR * 2.0) as usize;
    for chunk in signal[..warm].chunks(512) {
        let _ = detector.push_samples(chunk);
    }

    // Measured stretch: ten full seconds of detections must stay dark.
    let probe = allocation_counter::measure(|| {
        std::hint::black_box(vec![0u8; 1024]);
    });
    assert!(
        probe.count_total > 0,
        "allocation measurement must be active"
    );
    let mut detections = 0usize;
    let allocations = allocation_counter::measure(|| {
        for chunk in signal[warm..].chunks(512) {
            if detector.push_samples(chunk).is_some() {
                detections += 1;
            }
        }
    });
    let allocs = allocations.count_total;
    // Sanity: the signal really did drive detections, otherwise the zero
    // below would be vacuous.
    assert!(
        detections > 50,
        "expected detections to exercise the hot path, got {detections}"
    );
    assert_eq!(
        allocs, 0,
        "detection path performed {allocs} heap allocations"
    );
}
