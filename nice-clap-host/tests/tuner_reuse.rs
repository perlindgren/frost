//! A roster edit keeps the plugin instances it did not touch.
//! Run explicitly after building this repository's own nice-tuner CLAP export.
#![deny(unsafe_op_in_unsafe_fn)]

use nice_clap_host::{Binding, Entry, LiveAudio, LiveSession, LiveStatus};
use nice_rack_engine::{Channels, Connection, NodeId, NodeKind, RackGraph};
use std::collections::BTreeMap;

const TUNER: &str = "com.nice-tuner.nice-tuner";
const BLOCK: usize = 32;

/// One plugin node, the way the rack would wire it.
fn chain() -> RackGraph {
    RackGraph {
        nodes: [
            (NodeId(0), NodeKind::Input(Channels::Mono)),
            (
                NodeId(1),
                NodeKind::Plugin {
                    inputs: Channels::Mono,
                    outputs: Channels::Mono,
                },
            ),
            (NodeId(2), NodeKind::Output(Channels::Mono)),
        ]
        .into(),
        connections: [
            Connection {
                from: NodeId(0),
                to: NodeId(1),
            },
            Connection {
                from: NodeId(1),
                to: NodeId(2),
            },
        ]
        .into(),
        ..RackGraph::default()
    }
}

/// Take a chain off the transport the way the rack does: ask, keep the audio side
/// calling until it hands the graph back, then service the main thread until every
/// instance has returned its handle. What is left is a session whose plugins are
/// alive, inactive, and waiting to be asked for again.
fn stop(session: &mut LiveSession, audio: LiveAudio) {
    session.request_stop();
    std::thread::spawn(move || {
        let mut audio = audio;
        audio.process(&[], &mut [], 0).unwrap()
    })
    .join()
    .unwrap();
    while session.service().unwrap() != LiveStatus::Stopped {
        std::thread::yield_now();
    }
}

/// One block through a chain's audio side, on a thread of its own as the rack's
/// DSP worker runs it, and the audio side back again.
fn block(mut audio: LiveAudio) -> (LiveAudio, [f32; BLOCK]) {
    std::thread::spawn(move || {
        let mut mono = [0.0; BLOCK];
        audio
            .process(&[&[0.25; BLOCK]], &mut [&mut [&mut mono[..]][..]], BLOCK)
            .unwrap();
        (audio, mono)
    })
    .join()
    .unwrap()
}

fn value(session: &mut LiveSession, param: u32) -> f64 {
    session
        .parameters(NodeId(1))
        .into_iter()
        .find(|p| p.id == param)
        .unwrap()
        .value
}

/// What a plugin holds that no state save carries is the reason a roster edit
/// should not rebuild what it did not change, and the reason `prepare` cannot
/// answer for it: state is a favour a plugin does its host, and the tuner's own
/// state is not what this feeds the next chain. So the value set here is one the
/// second chain can only be told about by being the same instance — and the block
/// of audio is what proves the new chain is really processing it, rather than
/// merely keeping a plugin around that nothing is asking anything of.
#[test]
#[ignore = "requires NICE_RACK_TEST_CLAP pointing to a freshly built repository nice-tuner CLAP export"]
fn a_rebuilt_chain_keeps_the_instance_the_edit_did_not_touch() {
    let path = std::env::var_os("NICE_RACK_TEST_CLAP").expect("Set NICE_RACK_TEST_CLAP");
    // SAFETY: explicit opt-in to this repository's own freshly built plugin.
    let entry = unsafe { Entry::load(std::path::Path::new(&path)).unwrap() };
    let graph = chain();
    let bindings = BTreeMap::from([(
        NodeId(1),
        Binding {
            entry,
            class: TUNER.to_owned(),
        },
    )]);
    let states = BTreeMap::new();
    let (mut session, audio) = LiveSession::prepare(&graph, &bindings, 48000.0, BLOCK, &states)
        .expect("the tuner builds a live chain");
    let mute = session
        .parameters(NodeId(1))
        .into_iter()
        .find(|p| p.name == "Mute")
        .unwrap();
    assert_eq!(value(&mut session, mute.id), 0.0);

    stop(&mut session, audio);
    session
        .set_parameter(NodeId(1), mute.id, 1.0)
        .expect("an inactive instance takes a parameter");
    assert_eq!(value(&mut session, mute.id), 1.0);

    let (mut carried, audio) =
        match LiveSession::reusing(Some(session), &graph, &bindings, 48000.0, BLOCK, &states) {
            Ok(built) => built,
            Err(error) => panic!("the chain that was stopped should be carried: {error}"),
        };
    assert_eq!(
        value(&mut carried, mute.id),
        1.0,
        "the plugin this edit did not touch should still be the plugin it was"
    );
    let (audio, mono) = block(audio);
    assert_eq!(
        mono, [0.0; BLOCK],
        "the new chain should be processing the instance it inherited, which is the one that is muted"
    );
    stop(&mut carried, audio);

    // The same edit with nothing to carry, which is every rebuild before this one:
    // the plugin is built again out of the state the chain before it saved, and
    // what that state did not hold went with the instance that had it.
    let (mut replaced, audio) =
        LiveSession::prepare(&graph, &bindings, 48000.0, BLOCK, &states).unwrap();
    assert_eq!(
        value(&mut replaced, mute.id),
        0.0,
        "a plugin built from nothing but the saved state should be a fresh one"
    );
    let (audio, mono) = block(audio);
    assert_eq!(
        mono, [0.25; BLOCK],
        "and the tuner should be passing the block through, mute and all"
    );
    stop(&mut replaced, audio);
}
