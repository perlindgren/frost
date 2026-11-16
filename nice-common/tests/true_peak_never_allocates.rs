//! The true-peak estimator runs on the audio thread of every producer in the
//! workspace, between the callback's own work and the atomic publish, so the
//! claim that module makes about its footprint is measured here rather than
//! asserted in a comment. Same witness as `resample_never_allocates`: the
//! counter is proved live on a known allocation, then the real work is run and
//! checked for a checksum so the optimiser cannot quietly drop it.
#![forbid(unsafe_code)]
use nice_common::true_peak::TruePeak;

/// A signal with something to find: a tone near a third of the sample rate,
/// which is where the estimator reads furthest under, plus one hard crest in
/// the middle so there is a nonzero maximum to publish.
fn signal(frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| {
            let tone = (2.0 * std::f64::consts::PI * 0.3333 * i as f64).sin() as f32;
            let burst = if (4096..4608).contains(&i) { 0.4 } else { 0.0 };
            (0.9 * tone + burst).clamp(-1.0, 1.0)
        })
        .collect()
}

#[test]
fn measuring_a_stream_allocates_nothing_after_construction() {
    let input = signal(128 * 4096);
    let mut estimator = TruePeak::new();
    let probe = allocation_counter::measure(|| {
        std::hint::black_box(vec![0u8; 1024]);
    });
    assert!(
        probe.count_total > 0,
        "allocation measurement must be active"
    );

    let mut published = 0.0f32;
    let mut blocks = 0u64;
    let allocations = allocation_counter::measure(|| {
        for block in input.chunks(128) {
            estimator.process(block);
            published = published.max(estimator.take());
            blocks += 1;
        }
        // A long-held stream must also be free: reset is the state a producer
        // touches when the transport stops.
        for _ in 0..blocks {
            estimator.reset();
            estimator.process(&input[..128]);
            std::hint::black_box(estimator.take());
        }
    });
    assert_eq!(allocations.count_total, 0);

    // The taps are compile-time constants and the state is a fixed array, so
    // even construction is allocation-free — `new` is a `const fn` over plain
    // fields, which is what lets a producer own one beside its atomics.
    let constructed = allocation_counter::measure(|| {
        for _ in 0..1000 {
            std::hint::black_box(TruePeak::new());
        }
    });
    assert_eq!(constructed.count_total, 0);

    assert_eq!(blocks, 4096);
    let raw = input.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(
        published > raw && published < raw + 0.2,
        "peak {published} is not the reconstruction crest of a raw {raw}"
    );
}
