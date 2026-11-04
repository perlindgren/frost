//! Exercise the actual output converter used by the standalone callback.
#![cfg(feature = "standalone")]
#![forbid(unsafe_code)]
#[path = "../src/standalone/resample.rs"]
mod resample;
#[test]
fn streaming_conversion_allocates_nothing_after_setup() {
    let (mut producer, consumer) = rtrb::RingBuffer::new(4096);
    for _ in 0..882 {
        producer.push(0.25).unwrap();
    }
    let mut monitor = resample::Monitor::new(consumer, 44100.0, 48000.0, 882, 441);
    let probe = allocation_counter::measure(|| {
        std::hint::black_box(vec![0u8; 1024]);
    });
    assert!(
        probe.count_total > 0,
        "allocation measurement must be active"
    );
    let mut checksum = 0.0f64;
    let allocations = allocation_counter::measure(|| {
        for _ in 0..1000 {
            for _ in 0..441 {
                producer.push(0.25).unwrap();
            }
            monitor.begin_block();
            for _ in 0..480 {
                checksum += monitor.next() as f64;
            }
        }
    });
    assert_eq!(allocations.count_total, 0);
    assert!(
        (checksum - 120000.0).abs() < 100.0,
        "audio was lost: {checksum}"
    );
}
