//! What the process can say about the machine it is standing on.

/// The GPU name is recorded once, readable from anywhere afterwards, and
/// honest about the gap before startup has asked for an adapter. A benchmark
/// that stores a number has to store which chip produced it, and it asks at
/// an arbitrary point in a run — so the API answers `None` rather than
/// panicking or guessing, and the first answer is the one that sticks: a
/// probe on a helper thread must not rename the GPU the window draws on.
#[test]
fn the_gpu_name_is_recorded_once_and_survives_a_second_probe() {
    crate::record_adapter_name("the adapter startup chose");
    let named = crate::adapter_name().expect("the first record is the name");
    crate::record_adapter_name("a later probe on another thread");
    assert_eq!(
        crate::adapter_name(),
        Some(named),
        "the name a run measured under must not change underneath it"
    );
    assert!(
        !named.is_empty(),
        "an empty adapter name would rename a baseline file to nothing"
    );
}
