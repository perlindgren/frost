//! What a metered wire costs in front of a real card, and the numbers behind
//! the `Measured against real plugins` table of
//! [`nice_common::true_peak`] — which is the only place they are written down,
//! since a nanosecond count is a property of a machine and a code generator
//! rather than an invariant this suite could hold anybody to.
//!
//! The engine-side half of the story — the cost of each pass on its own, and of
//! a rack of cards that do nothing but pass audio — is measured in
//! `nice-rack-engine`'s own `cost_bench` module, where the private parts are
//! reachable. This is the other half: the same rack, but with the pass-through
//! processor replaced by an actual plugin loaded through this host, so that the
//! metering bill is read against the DSP it really sits behind. Nothing here
//! asserts anything about time; it prints, and the doc quotes what it printed.
//!
//! ```text
//! RUSTFLAGS="-Ctarget-cpu=x86-64-v3" cargo xtask bundle nice-echo --release
//! RUSTFLAGS="-Ctarget-cpu=x86-64-v3" \
//!   NICE_RACK_TEST_CLAP="$PWD/target/bundled/Nice Echo.clap" \
//!   NICE_RACK_TEST_CLASS=com.frost.nice-echo \
//!   cargo test -p nice-clap-host --release --test live_cost -- \
//!   --ignored --nocapture --test-threads=1
//! ```
//!
//! `NICE_RACK_TEST_CLAP` names the `.clap` to measure and has to be an absolute
//! path — a relative one reaches `dlopen` after the test binary has changed
//! directory and fails for a reason that has nothing to do with cost.
//! `NICE_RACK_TEST_CLASS` picks the plugin out of that file when it carries
//! more than one. Both belong to whoever runs this, and the honest reading of a
//! row is the machine plus the code generation target it came from. Drop the
//! `RUSTFLAGS` above and the effect is immediate and one-sided: the metering
//! half is host code, and its per-wire increment climbs from 0.29–0.35 µs to
//! 0.44–0.59 µs, while the cards themselves — already shipped built for
//! `x86-64-v3` — hardly move. A row from a run without the flag is a row about
//! a build nobody ships.
//!
//! The graph is built and driven the way `nice-rack` builds and drives one:
//! one `WireLevels` table for the rack, an attached one for the metered run and
//! a table that registers nothing for the dark run, the audio side on its own
//! thread with the main thread servicing plugin callbacks beside it.
#![deny(unsafe_op_in_unsafe_fn)]

use nice_clap_host::{Entry, LiveAudio, LiveSession};
use nice_rack_engine::{Channels, Connection, NodeId, NodeKind, RackGraph, audio::WireLevels};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The block the rack is configured with in practice, and the one the
/// engine-side harness times, so the two halves add up.
const FRAMES: usize = 128;
/// Blocks per timing sample. A card costs microseconds rather than the
/// hundreds of nanoseconds a bare pass does, so a chunk is shorter here while
/// still running to a few tens of milliseconds per sample.
const CHUNK: usize = 1_000;
/// Timing samples per row. Plugin DSP is slower and noisier than a metering
/// loop, and ten samples are enough for a median that several runs agree on.
const CHUNKS: usize = 12;
/// A 128-frame block at 48 kHz, in nanoseconds.
const BLOCK_NS: f64 = 2_666_667.0;

/// `0.55` sine, one cycle per 48 frames, plus a near-full-scale impulse every
/// 64 frames — the same material the engine harness feeds its wires, so the
/// estimator in front of a card sees what it sees in the bench next door.
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

/// `cards` copies of one class in a line between the jack and the output, with
/// the class table every plugin node needs. Cards take odd ids the way the rack
/// hands them out, so the wire keys the attach step folds out of them are the
/// real ones. The jack and the output take the card's own bus widths, which is
/// what makes a chain of copies of one card valid at all.
fn chain(
    cards: usize,
    class: &str,
    inputs: Channels,
    outputs: Channels,
) -> (RackGraph, BTreeMap<NodeId, String>) {
    let mut nodes = BTreeMap::new();
    let mut classes = BTreeMap::new();
    let mut connections = Vec::new();
    nodes.insert(NodeId(0), NodeKind::Input(inputs));
    let mut previous = NodeId(0);
    for index in 0..cards {
        let id = NodeId(1 + index as u64 * 2);
        nodes.insert(id, NodeKind::Plugin { inputs, outputs });
        classes.insert(id, class.to_string());
        connections.push(Connection {
            from: previous,
            to: id,
        });
        previous = id;
    }
    nodes.insert(NodeId(1_000), NodeKind::Output(outputs));
    connections.push(Connection {
        from: previous,
        to: NodeId(1_000),
    });
    (RackGraph { nodes, connections }, classes)
}

/// One block through the audio side, exactly as the rack's DSP worker hands it
/// over: planar input borrowed from the jack, planar output to the device.
fn block(audio: &mut LiveAudio, left: &[f32], right: &[f32]) {
    let mut out_left = [0f32; FRAMES];
    let mut out_right = [0f32; FRAMES];
    let input: [&[f32]; 2] = [left, right];
    let mut output: [&mut [f32]; 2] = [&mut out_left, &mut out_right];
    audio
        .process(&input, &mut output, FRAMES)
        .expect("the card should survive the block");
    black_box((out_left[0], out_right[0]));
}

/// Minimum and median nanoseconds per block over [`CHUNKS`] chunks, with the
/// audio side handed back.
///
/// The blocks run on a thread of their own while this one services the
/// plugins' main-thread callbacks, because that is the arrangement the rack
/// runs in and the one whose cost is worth quoting: a card whose main thread
/// never runs is a card whose latency and parameter changes never land. The
/// servicing costs a few microseconds a millisecond on a core this loop is not
/// competing for, and never on the timed one.
fn timed(
    audio: LiveAudio,
    session: &mut LiveSession,
    left: &[f32],
    right: &[f32],
) -> ((f64, f64), LiveAudio) {
    std::thread::scope(|scope| {
        let worker = scope.spawn(move || {
            let mut audio = audio;
            for _ in 0..CHUNK / 4 {
                block(&mut audio, left, right);
            }
            let mut samples = Vec::with_capacity(CHUNKS);
            for _ in 0..CHUNKS {
                let start = Instant::now();
                for _ in 0..CHUNK {
                    block(&mut audio, left, right);
                }
                let elapsed = start.elapsed().as_nanos() as f64;
                samples.push(elapsed / CHUNK as f64);
            }
            samples.sort_by(|a, b| a.total_cmp(b));
            ((samples[0], samples[samples.len() / 2]), audio)
        });
        while !worker.is_finished() {
            session.service().expect("main-thread servicing");
            std::thread::park_timeout(Duration::from_millis(1));
        }
        let (timing, audio) = worker.join().expect("the timed thread should not panic");
        (timing, audio)
    })
}

/// The card the operator asked for, by path and optionally by class.
fn card() -> (Entry, String, Channels, Channels) {
    let path = std::env::var_os("NICE_RACK_TEST_CLAP")
        .expect("Set NICE_RACK_TEST_CLAP to an absolute path to a built CLAP");
    let path = Path::new(&path);
    // SAFETY: this opt-in, `#[ignore]`d test loads the file the person running
    // it named, which is this repository's own build of a card; see the module
    // docs. It is not a path discovery mechanism and not a plugin host.
    let entry = unsafe { Entry::load(path) }.expect("the named CLAP should load");
    let descriptors = entry.descriptors().expect("the CLAP should have a factory");
    let wanted = std::env::var("NICE_RACK_TEST_CLASS").ok();
    let found = descriptors
        .iter()
        .find(|descriptor| wanted.as_deref().is_none_or(|id| descriptor.id == id))
        .expect("the named CLAP should carry the named class");
    let (inputs, outputs) = entry
        .main_channels(&found.id)
        .expect("the class should expose a main audio bus");
    let class = found.id.clone();
    println!(
        "\n{} ({class}), {:?} in / {:?} out, {FRAMES}-frame blocks, per block",
        found.name, inputs, outputs
    );
    (entry, class, inputs, outputs)
}

/// One printed row: label, minimum and median per block, and the share of a
/// 48 kHz block the median eats. Same shape as the engine harness, so the two
/// tables read as one.
fn row(label: &str, (min, median): (f64, f64)) {
    println!(
        "{label:<28} {:>9.1} µs {:>9.1} µs {:>7.3} %",
        min / 1_000.0,
        median / 1_000.0,
        median / BLOCK_NS * 100.0,
    );
}

/// The same chain twice, once with a table that registers nothing and once with
/// a table big enough for every wire, as the difference in the block.
///
/// An empty `WireLevels` is not the same thing as no meter table at all: it is
/// what the rack does when there is nothing to publish to, and the nodes carry
/// `None` for both of their slots, so the fold and the estimator are never
/// called. That is the dark run the doc quotes, and the only one worth
/// subtracting, because a rack with no metering at all is a rack that cannot
/// tell its operator where the signal went.
#[test]
#[ignore = "loads a real CLAP and prints a cost table; see the module docs"]
fn metering_cost_in_front_of_a_card() {
    let (entry, class, inputs, outputs) = card();
    let (left, right) = signal();
    println!(
        "{:<28} {:>10} {:>10} {:>9}",
        "chain", "min", "median", "of a block"
    );
    for cards in [1usize, 4] {
        let (graph, classes) = chain(cards, &class, inputs, outputs);

        let (mut session, mut audio) = entry
            .prepare_live(&graph, &classes, 48_000.0, FRAMES)
            .expect("the chain should prepare");
        audio.attach_wire_levels(Arc::new(WireLevels::new(0)));
        let (dark, audio) = timed(audio, &mut session, &left, &right);
        drop(audio);
        drop(session);

        let (mut session, mut audio) = entry
            .prepare_live(&graph, &classes, 48_000.0, FRAMES)
            .expect("the chain should prepare again");
        audio.attach_wire_levels(Arc::new(WireLevels::new(cards * 2 + 2)));
        let (lit, audio) = timed(audio, &mut session, &left, &right);
        drop(audio);
        drop(session);

        row(&format!("{cards} card(s), dark"), dark);
        row(&format!("{cards} card(s), metered"), lit);
        // Two wires per card: what one more metered card adds to the block.
        let per_wire = (lit.1 - dark.1) / (cards * 2) as f64;
        println!(
            "{:<28} {:>10.3} µs per metered wire, {:.2}× the dark block",
            "",
            per_wire / 1_000.0,
            lit.1 / dark.1,
        );
    }
}
