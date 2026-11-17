//! What a broken wire does to a running rack.
//!
//! The switch is the one part of the wire that changes what the graph *is*
//! rather than how loud it is, so the tests below are about the two halves of
//! that promise: nothing passes an open circuit, and the wire is still
//! measured up to the gap. The second half is what makes the feature usable —
//! a break that also killed its own meter would look exactly like a dead
//! source, which is the thing an operator throws a switch to tell apart.
//!
//! The cards are the copy-everything `Through` of [`cost_bench`], so anything
//! a test finds in a buffer was put there by a wire.

use super::*;
use crate::Connection;

/// The block the tests run at. Long enough for a tone to have a shape and for
/// the true-peak estimator to have something to say about it.
const FRAMES: usize = 128;

/// A card that copies its input, so the graph can only ever do to a signal
/// what its wires do to it.
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

/// `cards` stereo cards in a line between the jack and the output, on the odd
/// ids the rack hands out, so the wire keys the attach step folds out of them
/// are the real ones. Returns the plan with its own handle on the shared wire
/// table, already attached.
fn rack(cards: usize) -> (PreparedRack, Arc<WireLevels>) {
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
    let graph = RackGraph { nodes, connections };
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
    let mut prepared = PreparedRack::prepare(&graph, processors, FRAMES).expect("prepare");
    let levels = Arc::new(WireLevels::new(64));
    prepared.attach_wire_levels(Arc::clone(&levels));
    (prepared, levels)
}

/// A tone on both channels, so a meter has something to read and a silent
/// buffer is a statement rather than an absence.
fn tone() -> (Vec<f32>, Vec<f32>) {
    (
        (0..FRAMES)
            .map(|frame| 0.55 * (std::f32::consts::TAU * frame as f32 / 48.0).sin())
            .collect(),
        (0..FRAMES)
            .map(|frame| -0.4 * (std::f32::consts::TAU * frame as f32 / 24.0).sin())
            .collect(),
    )
}

/// One block through `prepared`, returning the master output.
fn run(prepared: &mut PreparedRack, left: &[f32], right: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let mut out_left = vec![0.0; FRAMES];
    let mut out_right = vec![0.0; FRAMES];
    {
        let input = [left, right];
        let mut output: Vec<&mut [f32]> = vec![&mut out_left, &mut out_right];
        prepared
            .process(&input, &mut output, FRAMES)
            .expect("process");
    }
    (out_left, out_right)
}

fn loud(buffer: &[f32]) -> bool {
    buffer.iter().any(|sample| sample.abs() > 0.1)
}

fn quiet(buffer: &[f32]) -> bool {
    buffer.iter().all(|sample| *sample == 0.0)
}

/// The same rack, wire by wire, says the same thing with the switch as the
/// cables say without it: closed passes, open does not.
#[test]
fn a_closed_wire_carries_and_an_open_one_does_not() {
    let (left, right) = tone();
    let (mut prepared, levels) = rack(2);
    let second = NodeId(3);

    let (out_left, _) = run(&mut prepared, &left, &right);
    assert!(loud(&out_left), "a closed rack has to pass something");
    assert!(
        !levels.open_of(second.0 << 1),
        "every wire starts closed, so a rack is never silently broken"
    );

    levels.set_open(second.0 << 1, true);
    let (out_left, out_right) = run(&mut prepared, &left, &right);
    assert!(quiet(&out_left), "an open input wire passes nothing");
    assert!(quiet(&out_right), "nor does it leak into the other channel");

    levels.set_open(second.0 << 1, false);
    let (out_left, _) = run(&mut prepared, &left, &right);
    assert!(
        loud(&out_left),
        "closing the switch is the whole of undoing a break"
    );
}

/// A break is measured before it is broken. The stub on the switch reads the
/// signal arriving at the gap, so the operator can see the source alive on the
/// far side of it; everything past the gap reads silence.
#[test]
fn a_broken_wire_still_reads_the_signal_arriving_at_it() {
    let (left, right) = tone();
    let (mut prepared, levels) = rack(2);
    let (first, second) = (NodeId(1), NodeId(3));
    let key = |id: NodeId, side: bool| (id.0 << 1) | u64::from(side);

    levels.set_open(key(second, false), true);
    run(&mut prepared, &left, &right);

    let (rms, peak) = levels.sample(key(second, false)).expect("attached");
    assert!(
        rms > 0.1 && peak > 0.1,
        "the wire into a break is measured up to the gap, not after it: {rms}, {peak}"
    );
    let (rms, _) = levels.sample(key(first, true)).expect("attached");
    assert!(rms > 0.1, "and the wire it hangs off is still live: {rms}");
    let (rms, _) = levels.sample(key(second, true)).expect("attached");
    assert!(
        rms == 0.0,
        "while the card past the break has nothing to send on: {rms}"
    );
}

/// A card's output buffer is what every edge out of it reads, so one thrown
/// switch on a card's output takes the rest of the chain with it — which is
/// the only way a single break can quiet a fan-out the operator did not touch.
#[test]
fn an_open_output_wire_silences_everything_downstream() {
    let (left, right) = tone();
    let (mut prepared, levels) = rack(3);
    let (first, second) = (NodeId(1), NodeId(3));

    levels.set_open((first.0 << 1) | 1, true);
    let (out_left, _) = run(&mut prepared, &left, &right);
    assert!(quiet(&out_left), "the chain past a broken output is quiet");
    let (rms, _) = levels.sample(key(second, false)).expect("attached");
    assert!(
        rms == 0.0,
        "and its meters say so too, rather than showing the level a broken wire carried: {rms}"
    );
}

fn key(id: NodeId, side: bool) -> u64 {
    (id.0 << 1) | u64::from(side)
}

/// Rebuilding the rack to add or remove a card must not quietly repaint the
/// switches the operator left. A wire still in the plan keeps its slot, and so
/// keeps its break, exactly as it keeps its trim. A wire the plan retired goes
/// back to a free closed slot: the plan, and not a switch left on a cable
/// nobody is holding any more, decides what a new wire is.
#[test]
fn a_rebuild_keeps_the_switch_on_the_wire_that_survives_it() {
    let levels = WireLevels::new(4);
    let broken = key(NodeId(1), true);
    let removed = key(NodeId(3), false);
    levels.attach(broken).expect("room");
    levels.attach(removed).expect("room");
    levels.set_gain(broken, 0.5);
    levels.set_open(broken, true);
    levels.set_open(removed, true);
    assert!(levels.open_of(broken) && levels.open_of(removed));

    // A plan that keeps one wire and drops the other.
    levels.reset(&[broken]);
    assert!(levels.open_of(broken), "a break survives a rebuild");
    assert_eq!(levels.gain_of(broken), Some(0.5));
    assert!(!levels.open_of(removed), "a retired wire stops being one");

    // And the order the interface relies on: a rebuild lands on the audio
    // thread's own schedule, so a wire that is still there has to come out of
    // it holding the switch the interface put on it, not wait to be re-told.
    let (mut prepared, shared) = rack(1);
    let only = key(NodeId(1), true);
    shared.set_open(only, true);
    prepared.attach_wire_levels(Arc::clone(&shared));
    assert!(
        shared.open_of(only),
        "re-attaching the plan a broken wire is still in keeps the break"
    );

    let (left, right) = tone();
    let (out_left, _) = run(&mut prepared, &left, &right);
    assert!(
        quiet(&out_left),
        "so the break holds the rack quiet across it"
    );
    shared.set_open(only, false);
    let (out_left, _) = run(&mut prepared, &left, &right);
    assert!(loud(&out_left), "and closing it lets the rack run again");
}
