//! Explicit native smoke test. The fixture and helper are built separately so
//! normal unit tests neither load arbitrary code nor require a desktop session.
use nice_clap_host::{Binding, Entry, LiveSession};
use nice_rack_engine::{Channels, Connection, NodeId, NodeKind, RackGraph};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires NICE_RACK_TEST_HELPER and NICE_RACK_TEST_CLAP"]
fn helper_process_runs_audio_and_acknowledges_state_without_an_editor() {
    struct Shutdown;
    impl Drop for Shutdown {
        fn drop(&mut self) {
            nice_clap_host::isolated::finish_shutdown();
        }
    }
    let _shutdown = Shutdown;
    let executable =
        PathBuf::from(std::env::var_os("NICE_RACK_TEST_HELPER").expect("NICE_RACK_TEST_HELPER"));
    let path = PathBuf::from(std::env::var_os("NICE_RACK_TEST_CLAP").expect("NICE_RACK_TEST_CLAP"));
    let entry = Entry::isolated(&path, &executable).unwrap();
    let class = entry.descriptors().unwrap()[0].id.clone();
    let (input, output) = entry.main_channels(&class).unwrap();
    assert_eq!(
        (input, output),
        (Channels::Mono, Channels::Mono),
        "use a mono fixture"
    );
    let graph = RackGraph {
        nodes: [
            (NodeId(0), NodeKind::Input(input)),
            (
                NodeId(1),
                NodeKind::Plugin {
                    inputs: input,
                    outputs: output,
                },
            ),
            (NodeId(2), NodeKind::Output(output)),
        ]
        .into(),
        connections: [(0, 1), (1, 2)]
            .map(|(a, b)| Connection {
                from: NodeId(a),
                to: NodeId(b),
            })
            .into(),
        ..RackGraph::default()
    };
    let bindings = [(NodeId(1), Binding { entry, class })].into();
    let (mut session, mut audio) =
        LiveSession::prepare(&graph, &bindings, 48000., 4096, &BTreeMap::new()).unwrap();
    assert!(session.editor_is_isolated(NodeId(1)));
    // A large budget keeps this a functional test, not a noisy performance gate.
    let samples = [0.125; 4096];
    let mut output = [0.; 4096];
    for _ in 0..4 {
        session.service().unwrap();
        audio
            .process(&[&samples], &mut [&mut [&mut output]], 4096)
            .unwrap();
        assert!(output.iter().all(|sample| sample.is_finite()));
    }
    assert!(
        output.iter().any(|sample| sample.abs() > 1e-6),
        "nice-echo must return actual audio"
    );
    let initial_state = session.save_states().unwrap();
    let parameters = session.parameters(NodeId(1));
    let mut changed = false;
    if let Some(parameter) = parameters.iter().find(|p| !p.readonly) {
        session
            .set_parameter(
                NodeId(1),
                parameter.id,
                if parameter.value != parameter.max {
                    parameter.max
                } else {
                    parameter.min
                },
            )
            .unwrap();
        changed = true;
    }
    session.request_stop();
    drop(audio);
    session.service().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !session.isolated_snapshot_ready().unwrap() {
        assert!(
            Instant::now() < deadline,
            "state capture never acknowledged"
        );
        std::thread::sleep(Duration::from_millis(5));
        session.service().unwrap();
    }
    let state = session.snapshot().unwrap();
    assert!(state.contains_key(&NodeId(1)));
    if changed {
        assert_ne!(
            state, initial_state,
            "an edit immediately before stop must be saved"
        );
    }
    // Rebuilding carries compatible endpoints without accidentally reusing an
    // old request sequence or stopping the retained helper's audio worker.
    let (mut carried, mut audio) =
        LiveSession::reusing(Some(session), &graph, &bindings, 48000., 4096, &state).unwrap();
    audio
        .process(&[&samples], &mut [&mut [&mut output]], 4096)
        .unwrap();
    carried.service().unwrap();
    carried.request_stop();
    drop(audio);
    carried.service().unwrap();
}
