//! Where the metering numbers come from.
//!
//! Every cost figure quoted for the wire meters — in [`crate::audio`], in the
//! rack's GUI notes and in the `# Cost` section of
//! [`nice_common::true_peak`] — was produced here, on one machine, so that a
//! later reader can re-run the same measurement instead of trusting a
//! comment. It is a test that asserts nothing on purpose: a nanosecond count
//! is a property of a machine and a code generator, not an invariant, and a
//! suite that went red on a noisy laptop would teach nothing. It is therefore
//! `#[ignore]`d and prints a table instead:
//!
//! ```text
//! RUSTFLAGS="-Ctarget-cpu=x86-64-v3" \
//!   cargo test -p nice-rack-engine --release --lib -- \
//!   --ignored --nocapture --test-threads=1 cost_bench
//! ```
//!
//! One thread, so the two tables print in order rather than interleaved.
//! Two habits matter more than the code itself. Debug builds are useless
//! here: the tap loop of the estimator is a recurrence that only becomes the
//! shipped shape under optimisation. And the numbers belong to the code
//! generation target, so they are quoted with it — the same pass measures
//! about 1.6 ns/sample at the baseline `x86-64` and 0.95 ns/sample at
//! `x86-64-v3`, which is the AVX2 fused multiply-add, not a change in the
//! meter. A block is timed per chunk of [`CHUNK`] blocks and the minimum and
//! median of [`CHUNKS`] chunks are reported: the minimum is the machine doing
//! the work, the median is roughly what a scheduler lets it keep.

use super::*;
use crate::Connection;
use std::hint::black_box;
use std::time::Instant;

/// The block the rack is configured with in practice.
const FRAMES: usize = 128;
/// Blocks per timing sample. One block is far too short to time against a
/// clock, and a chunk keeps one timer read per a few hundred microseconds.
const CHUNK: usize = 4_000;
/// Timing samples per row. The minimum wants a tail to find, the median wants
/// enough samples that one scheduler hiccup cannot carry it.
const CHUNKS: usize = 30;
/// A 128-frame block at 48 kHz, in nanoseconds: the budget everything below
/// is expressed as a fraction of.
const BLOCK_NS: f64 = 2_666_667.0;

/// `0.55` sine, one cycle per 48 frames, plus an impulse every 64 frames.
/// A metering bench on silence measures a warm loop, and a bench on full
/// scale noise hides the difference between signals in cache behaviour, so
/// this carries what a wire actually holds: a tone the estimator can lock
/// onto, with the sparse near-full-scale hits its taps exist for.
fn signal() -> (Vec<f32>, Vec<f32>) {
    let mut left = Vec::with_capacity(FRAMES);
    let mut right = Vec::with_capacity(FRAMES);
    for frame in 0..FRAMES {
        let tone = 0.55 * (std::f32::consts::TAU * frame as f32 / 48.0).sin();
        left.push(if frame % 64 == 0 { 0.98 } else { tone });
        right.push(if frame % 64 == 63 { -0.97 } else { -tone * 0.5 });
    }
    (left, right)
}

/// Minimum and median nanoseconds per operation over [`CHUNKS`] chunks.
fn timed(mut work: impl FnMut()) -> (f64, f64) {
    let mut samples = Vec::with_capacity(CHUNKS);
    for _ in 0..CHUNKS {
        let start = Instant::now();
        for _ in 0..CHUNK {
            work();
        }
        let elapsed = start.elapsed().as_nanos() as f64;
        samples.push(elapsed / CHUNK as f64);
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    (samples[0], samples[samples.len() / 2])
}

fn row(label: &str, (min, median): (f64, f64)) {
    println!(
        "{label:<34} {min:8.1} ns  {median:8.1} ns  {:>6.3} % of a block",
        median / BLOCK_NS * 100.0
    );
}

/// A card that costs nothing but its buffers, so that what a rack row measures
/// is the graph and the metering and not a plugin.
struct Through;
impl Processor for Through {
    fn process(
        &mut self,
        input: &AudioBuffer,
        output: &mut AudioBuffer,
        frames: usize,
    ) -> Result<(), ProcessError> {
        for (source, destination) in input.0.iter().zip(&mut output.0) {
            destination[..frames].copy_from_slice(&source[..frames]);
        }
        Ok(())
    }
}

/// `count` stereo cards in a line between the jack and the output. Cards take
/// odd ids the way the rack hands them out, so the wire keys the attach step
/// folds out of them are the real ones.
fn rack(cards: usize) -> PreparedRack {
    let mut nodes = std::collections::BTreeMap::new();
    let mut connections = Vec::new();
    nodes.insert(NodeId(0), NodeKind::Input(Channels::Stereo));
    let mut previous = NodeId(0);
    for index in 0..cards {
        let id = NodeId(1 + index as u64 * 2);
        nodes.insert(
            id,
            NodeKind::Plugin {
                inputs: Channels::Stereo,
                outputs: Channels::Stereo,
            },
        );
        connections.push(Connection {
            from: previous,
            to: id,
        });
        previous = id;
    }
    nodes.insert(NodeId(1_000), NodeKind::Output(Channels::Stereo));
    connections.push(Connection {
        from: previous,
        to: NodeId(1_000),
    });
    let graph = RackGraph {
        nodes,
        connections,
        ..RackGraph::default()
    };
    let processors = (0..cards)
        .map(|index| {
            (
                NodeId(1 + index as u64 * 2),
                ProcessorSlot {
                    processor: Box::new(Through),
                    bypassed: false,
                },
            )
        })
        .collect();
    PreparedRack::prepare(&graph, processors, FRAMES).expect("prepare")
}

/// The parts of one block, one at a time, on the buffers a card actually
/// holds. Every row is stereo, `FRAMES` long.
#[test]
#[ignore = "prints a benchmark table; see the module docs"]
fn cost_of_each_pass() {
    let (left, right) = signal();
    let stereo = || {
        let mut buffer = AudioBuffer::new(2, FRAMES);
        buffer.0[0].copy_from_slice(&left);
        buffer.0[1].copy_from_slice(&right);
        buffer
    };
    println!("\nparts of one {FRAMES}-frame stereo block");

    // The estimator on both channels, exactly as `wire_level` drives it: the
    // `take` belongs to the pass, because holding a crest is what lets the
    // next block continue it.
    let mut meters = [TruePeak::new(), TruePeak::new()];
    row(
        "true peak, both channels",
        timed(|| {
            let channels = [black_box(&left), black_box(&right)];
            for (meter, channel) in meters.iter_mut().zip(&channels) {
                meter.process(channel);
                black_box(meter.take());
            }
        }),
    );

    // The two reductions `wire_level` folds around the estimator, over the
    // same planar buffer it reads. They are the yardstick the docs quote: a
    // true-peak pass costs several of these, and the difference is the
    // recurrence in its taps rather than any work it cannot parallelise.
    let folded = stereo();
    row(
        "rms fold (as wire_level)",
        timed(|| {
            let mut energy = [0f64; 2];
            for (frame, l) in black_box(&folded).0[0][..FRAMES].iter().enumerate() {
                energy[0] += (l * l) as f64;
                let mut fold = 0.0f32;
                for channel in folded.0[1..].iter() {
                    fold = fold.max(channel[frame].abs());
                }
                energy[1] += (fold * fold) as f64;
            }
            black_box(energy);
        }),
    );
    row(
        "fold of absolute values",
        timed(|| {
            let mut peak = 0f32;
            for channel in &black_box(&folded).0 {
                for sample in channel[..FRAMES].iter() {
                    peak = peak.max(sample.abs());
                }
            }
            black_box(peak);
        }),
    );

    // One wire, as the graph pays for it: the folds and the estimator over
    // the node's settled buffer, then the store. `wire_level` alone is the
    // cheaper line; the publish is the four atomics — two lanes of level and
    // crest — that make the numbers visible.
    let buffer = stereo();
    let mut meters = [TruePeak::new(), TruePeak::new()];
    row(
        "wire_level",
        timed(|| {
            black_box(wire_level(&buffer, FRAMES, &mut meters));
        }),
    );
    let levels = WireLevels::new(8);
    let slot_in = levels.attach(2).unwrap();
    let slot_out = levels.attach(3).unwrap();
    let mut meters = [TruePeak::new(), TruePeak::new()];
    row(
        "wire_level + publish",
        timed(|| {
            let level = wire_level(&buffer, FRAMES, &mut meters);
            levels.publish(slot_in, level);
            black_box(level);
        }),
    );
    row(
        "publish, both wires",
        timed(|| {
            let level = WireSample {
                rms: [black_box(0.3), black_box(0.25)],
                peak: [black_box(0.9), black_box(0.85)],
            };
            levels.publish(slot_in, level);
            levels.publish(slot_out, level);
        }),
    );

    // The trim path. Unity leaves the block untouched — every wire the
    // operator never touched takes that branch — and a real trim is one
    // multiply per sample, which the same vectoriser handles.
    let mut buffer = stereo();
    row(
        "apply_gain at unity",
        timed(|| apply_gain(&mut buffer, FRAMES, black_box(1.0))),
    );
    row(
        "apply_gain at 0.5",
        timed(|| apply_gain(&mut buffer, FRAMES, black_box(0.5))),
    );

    // The bookkeeping around a card: an edge hop between two node buffers,
    // the zero-delay mix, and clearing both buffers before the next block.
    let source = stereo();
    let mut destination = AudioBuffer::new(2, FRAMES);
    row(
        "copy one buffer (stereo hop)",
        timed(|| {
            for (from, to) in source.0.iter().zip(&mut destination.0) {
                to[..FRAMES].copy_from_slice(black_box(&from[..FRAMES]));
            }
        }),
    );
    let mut delay = Delay::new(2, 0, &mut Budget(0)).unwrap();
    let mut target = AudioBuffer::new(2, FRAMES);
    row(
        "delay::add, no latency",
        timed(|| delay.add(&source, &mut target, FRAMES)),
    );
    let mut buffer = stereo();
    row("clear (stereo)", timed(|| buffer.clear(black_box(FRAMES))));
}

/// The whole `PreparedRack::process`, with the meter table attached and
/// without it, at the sizes the rack is used at. An empty table is the honest
/// comparison for a rack whose UI is closed: `attach_wire_levels` with a table
/// that registers nothing is what a blind plan looks like, and the nodes then
/// carry `None` for both of their wire slots.
#[test]
#[ignore = "prints a benchmark table; see the module docs"]
fn cost_of_a_metered_rack() {
    let (left, right) = signal();
    println!("\nPreparedRack::process, {FRAMES} frames, stereo, per block");
    for cards in [1usize, 4, 8] {
        let mut blind = rack(cards);
        blind.attach_wire_levels(Arc::new(WireLevels::new(0)));
        let mut metered = rack(cards);
        metered.attach_wire_levels(Arc::new(WireLevels::new(cards * 2 + 2)));

        let (in_min, in_median) = timed(|| {
            let mut out0 = [0f32; FRAMES];
            let mut out1 = [0f32; FRAMES];
            let input: [&[f32]; 2] = [black_box(&left), black_box(&right)];
            let mut output: [&mut [f32]; 2] = [&mut out0, &mut out1];
            let mut outputs = [&mut output[..]];
            blind.process(&input, &mut outputs, FRAMES).unwrap();
            black_box((out0[0], out1[0]));
        });
        let (on_min, on_median) = timed(|| {
            let mut out0 = [0f32; FRAMES];
            let mut out1 = [0f32; FRAMES];
            let input: [&[f32]; 2] = [black_box(&left), black_box(&right)];
            let mut output: [&mut [f32]; 2] = [&mut out0, &mut out1];
            let mut outputs = [&mut output[..]];
            metered.process(&input, &mut outputs, FRAMES).unwrap();
            black_box((out0[0], out1[0]));
        });
        row(
            &format!("{cards} card(s), metering off"),
            (in_min, in_median),
        );
        row(
            &format!("{cards} card(s), metering on"),
            (on_min, on_median),
        );
        // Two wires per card: what one more metered card adds to the block.
        let per_wire = (on_median - in_median) / (cards * 2) as f64;
        println!(
            "{:<34} {:>8.1} ns  per metered wire ({:.3} % of a block)",
            "",
            per_wire,
            per_wire / BLOCK_NS * 100.0
        );
    }
}

#[test]
#[ignore = "prints the optional per-card timing overhead; not a wall-clock gate"]
fn cost_of_per_card_timing() {
    let (left, right) = signal();
    let mut out_left = [0.0; FRAMES];
    let mut out_right = [0.0; FRAMES];
    for cards in [1, 5, 16] {
        let mut baseline = rack(cards);
        let mut observed = rack(cards);
        observed.attach_processing_times(Arc::new(ProcessingTimes::new(cards)), 48_000.0);
        row(
            &format!("{cards} cards without timing"),
            timed(|| {
                let mut group = [&mut out_left[..], &mut out_right[..]];
                let mut outputs = [&mut group[..]];
                baseline
                    .process(&[&left, &right], &mut outputs, FRAMES)
                    .unwrap();
            }),
        );
        row(
            &format!("{cards} cards with timing"),
            timed(|| {
                let mut group = [&mut out_left[..], &mut out_right[..]];
                let mut outputs = [&mut group[..]];
                observed
                    .process(&[&left, &right], &mut outputs, FRAMES)
                    .unwrap();
            }),
        );
    }
}

/// Individual-block tails complement the chunk averages above. These maxima
/// are observations on this run, never a guaranteed worst-case execution time.
#[test]
#[ignore = "manual release timing distribution; scheduler-dependent"]
fn timing_distribution() {
    let (left, right) = signal();
    let mut out_left = [0.0; FRAMES];
    let mut out_right = [0.0; FRAMES];
    for frames in [32, 64, 128] {
        for cards in [1, 5, 16] {
            for enabled in [false, true] {
                let mut graph = rack(cards);
                if enabled {
                    graph.attach_processing_times(Arc::new(ProcessingTimes::new(cards)), 48_000.0);
                }
                let mut samples = Vec::with_capacity(20_000);
                for iteration in 0..21_000 {
                    let mut group = [&mut out_left[..], &mut out_right[..]];
                    let mut outputs = [&mut group[..]];
                    let start = Instant::now();
                    graph
                        .process(black_box(&[&left, &right]), &mut outputs, frames)
                        .unwrap();
                    let ns = start.elapsed().as_nanos();
                    if iteration >= 1000 {
                        samples.push(ns);
                    }
                }
                samples.sort_unstable();
                eprintln!(
                    "frames={frames} cards={cards} timing={enabled}: ns median={} p99={} max={}",
                    samples[10_000], samples[19_800], samples[19_999]
                );
            }
        }
    }
}
