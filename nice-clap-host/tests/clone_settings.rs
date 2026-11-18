//! Exercise the real repository CLAP export, rather than only its Rust params.
#![deny(unsafe_op_in_unsafe_fn)]
use nice_clap_host::{Binding, Entry, LiveSession, LiveStatus};
use nice_rack_engine::{Channels, Connection, NodeId, NodeKind, RackGraph};
use std::collections::BTreeMap;

#[test]
#[ignore = "requires NICE_CLONE_TEST_CLAP pointing to a freshly built nice-clone CLAP"]
fn clone_oversampling_is_visible_editable_and_saved_through_clap() {
    let path = std::env::var_os("NICE_CLONE_TEST_CLAP").expect("set NICE_CLONE_TEST_CLAP");
    // SAFETY: this opt-in test loads only the explicitly supplied local build.
    let entry = unsafe { Entry::load(std::path::Path::new(&path)).unwrap() };
    let graph = RackGraph {
        nodes: [
            (NodeId(0), NodeKind::Input(Channels::Stereo)),
            (
                NodeId(1),
                NodeKind::Plugin {
                    inputs: Channels::Stereo,
                    outputs: Channels::Stereo,
                },
            ),
            (NodeId(2), NodeKind::Output(Channels::Stereo)),
        ]
        .into(),
        connections: vec![
            Connection {
                from: NodeId(0),
                to: NodeId(1),
            },
            Connection {
                from: NodeId(1),
                to: NodeId(2),
            },
        ],
    };
    let bindings = [(
        NodeId(1),
        Binding {
            entry,
            class: "com.frost.nice-clone".into(),
        },
    )]
    .into();
    let (mut session, mut audio) =
        LiveSession::prepare(&graph, &bindings, 48_000.0, 128, &BTreeMap::new()).unwrap();
    let parameter = session
        .parameters(NodeId(1))
        .into_iter()
        .find(|p| p.name == "Oversampling")
        .expect("CLAP must expose oversampling");
    assert!(!parameter.readonly);
    assert!(parameter.stepped);
    assert_eq!(
        (parameter.min, parameter.max, parameter.value),
        (0.0, 3.0, 3.0)
    );
    // Edit inactive parameters through CLAP flush, the same public settings
    // path available before playback; this test does not open a native window.
    session.request_stop();
    std::thread::spawn(move || {
        audio.process(&[], &mut [], 0).unwrap();
    })
    .join()
    .unwrap();
    while session.service().unwrap() != LiveStatus::Stopped {
        std::thread::yield_now();
    }
    session.set_parameter(NodeId(1), parameter.id, 1.0).unwrap();
    let parameter = session
        .parameters(NodeId(1))
        .into_iter()
        .find(|p| p.id == parameter.id)
        .unwrap();
    assert!((parameter.value - 1.0).abs() < 1e-6);
    assert!(parameter.text.starts_with('2'));
    let saved = session.save_states().unwrap();
    assert!(!saved[&NodeId(1)].is_empty());
    let (mut restored, mut audio) =
        LiveSession::prepare(&graph, &bindings, 48_000.0, 128, &saved).unwrap();
    let value = restored
        .parameters(NodeId(1))
        .into_iter()
        .find(|p| p.id == parameter.id)
        .unwrap();
    assert!((value.value - 1.0).abs() < 1e-6);
    restored.request_stop();
    std::thread::spawn(move || {
        audio.process(&[], &mut [], 0).unwrap();
    })
    .join()
    .unwrap();
    while restored.service().unwrap() != LiveStatus::Stopped {
        std::thread::yield_now();
    }
}
