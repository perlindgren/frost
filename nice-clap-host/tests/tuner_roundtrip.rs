//! Run explicitly after building this repository's own nice-tuner CLAP export.
#![deny(unsafe_op_in_unsafe_fn)]

#[test]
#[ignore = "requires NICE_RACK_TEST_CLAP pointing to a freshly built repository nice-tuner CLAP export"]
fn repository_tuner_processes_audio_and_round_trips_state() {
    let path = std::env::var_os("NICE_RACK_TEST_CLAP")
        .expect("Set NICE_RACK_TEST_CLAP to this repository's built nice-tuner export");
    // SAFETY: this opt-in integration test is restricted to the repository's
    // own nice-tuner export, not an arbitrary installed third-party plugin.
    let entry = unsafe { nice_clap_host::Entry::load(std::path::Path::new(&path)).unwrap() };
    let plugins = entry.descriptors().unwrap();
    assert!(
        plugins
            .iter()
            .any(|plugin| plugin.id == "com.nice-tuner.nice-tuner")
    );
    let report = entry
        .probe("com.nice-tuner.nice-tuner", 48000.0, 256)
        .unwrap();
    assert_eq!(
        (report.inputs, report.outputs, report.frames, report.latency),
        (2, 2, 256, 0)
    );
    assert!(report.state_supported);
    assert!(report.state_bytes_after.unwrap() > 0);
    assert_eq!(report.state_roundtrip_matches, Some(true));
    assert_eq!(report.audio.len(), 2);
    for channel in report.audio {
        assert_eq!(channel.len(), 256);
        assert!(channel.iter().all(|&sample| sample == 0.125));
    }
    assert!(
        entry
            .probe("com.nice-tuner.nice-tuner", f64::NAN, 256)
            .is_err()
    );
    assert!(
        entry
            .probe("com.nice-tuner.nice-tuner", 48000.0, 0)
            .is_err()
    );
}

#[test]
#[ignore = "requires NICE_RACK_TEST_CLAP pointing to a freshly built repository nice-tuner CLAP export"]
fn repository_plugins_render_serial_and_parallel_graphs_and_recover_from_setup_errors() {
    use nice_rack_engine::{Channels, Connection, NodeId, NodeKind, RackGraph};
    use std::collections::BTreeMap;
    let path = std::env::var_os("NICE_RACK_TEST_CLAP").expect("Set NICE_RACK_TEST_CLAP");
    // SAFETY: explicitly supplied build of this repository's own nice-tuner.
    let entry = unsafe { nice_clap_host::Entry::load(std::path::Path::new(&path)).unwrap() };
    let plugin = NodeKind::Plugin {
        inputs: Channels::Stereo,
        outputs: Channels::Stereo,
    };
    let classes: BTreeMap<_, _> = [
        (NodeId(1), "com.nice-tuner.nice-tuner".into()),
        (NodeId(2), "com.nice-tuner.nice-tuner".into()),
    ]
    .into();
    let input = vec![
        (0..513)
            .map(|i| if i % 2 == 0 { 0.125 } else { -0.25 })
            .collect::<Vec<f32>>(),
        vec![0.0625; 513],
    ];
    for parallel in [false, true] {
        let mut nodes: BTreeMap<_, _> = [
            (NodeId(0), NodeKind::Input(Channels::Stereo)),
            (NodeId(1), plugin.clone()),
            (NodeId(2), plugin.clone()),
            (NodeId(4), NodeKind::Output(Channels::Stereo)),
        ]
        .into();
        let routes = if parallel {
            nodes.insert(NodeId(3), NodeKind::Mixer(Channels::Stereo));
            vec![(0, 1), (0, 2), (1, 3), (2, 3), (3, 4)]
        } else {
            vec![(0, 1), (1, 2), (2, 4)]
        };
        let graph = RackGraph {
            nodes,
            connections: routes
                .into_iter()
                .map(|(from, to)| Connection {
                    from: NodeId(from),
                    to: NodeId(to),
                })
                .collect(),
        };
        let report = entry
            .render_graph(&graph, &classes, &input, 48000.0, 128)
            .unwrap();
        assert_eq!(report.latency_samples, 0);
        assert_eq!(report.saved_state_bytes.len(), 2);
        assert!(
            report
                .saved_state_bytes
                .values()
                .all(|bytes| bytes.is_some_and(|n| n > 0))
        );
        let gain = if parallel { 2.0 } else { 1.0 };
        for (output, input) in report.audio.iter().zip(&input) {
            assert_eq!(output.len(), 513);
            for (&output, &input) in output.iter().zip(input) {
                assert_eq!(output, input * gain);
            }
        }
        let mut bad_classes = classes.clone();
        bad_classes.insert(NodeId(2), "missing.plugin.id".into());
        assert!(
            entry
                .render_graph(&graph, &bad_classes, &input, 48000.0, 128)
                .is_err()
        );
        // Failure after activation of the first instance must not strand it.
        assert!(
            entry
                .render_graph(&graph, &classes, &input, 48000.0, 128)
                .is_ok()
        );
        let mut rejected = graph.clone();
        rejected
            .nodes
            .insert(NodeId(9), NodeKind::Output(Channels::Stereo));
        rejected.connections.push(Connection {
            from: NodeId(2),
            to: NodeId(9),
        });
        assert!(
            entry
                .render_graph(&rejected, &classes, &input, 48000.0, 128)
                .is_err()
        );
        assert!(
            entry
                .render_graph(&graph, &classes, &input, 48000.0, 128)
                .is_ok()
        );
    }
}

#[test]
#[ignore = "requires NICE_RACK_TEST_CLAP pointing to a freshly built repository nice-tuner CLAP export"]
fn live_graph_stops_at_a_block_boundary_then_reconfigures_and_resumes() {
    use nice_clap_host::LiveStatus;
    use nice_rack_engine::{Channels, Connection, NodeId, NodeKind, RackGraph};
    use std::{collections::BTreeMap, sync::mpsc::sync_channel};
    let path = std::env::var_os("NICE_RACK_TEST_CLAP").expect("Set NICE_RACK_TEST_CLAP");
    // SAFETY: explicit opt-in to this repository's own freshly built plugin.
    let entry = unsafe { nice_clap_host::Entry::load(std::path::Path::new(&path)).unwrap() };
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
    };
    let classes = BTreeMap::from([(NodeId(1), "com.nice-tuner.nice-tuner".into())]);
    let (mut session, audio) = entry.prepare_live(&graph, &classes, 48000.0, 128).unwrap();
    assert!(session.reconfigure(44100.0, 64).is_err());
    let mut next_audio = Some(audio);
    for cycle in 0..2 {
        let mut audio = next_audio.take().unwrap();
        std::thread::scope(|scope| {
            let (ready, receive) = sync_channel(1);
            let job = scope.spawn(move || {
                let mut left = [0.0; 128];
                let mut right = [0.0; 128];
                assert_eq!(
                    audio
                        .process(
                            &[&[0.125; 128], &[-0.25; 128]],
                            &mut [&mut left, &mut right],
                            37
                        )
                        .unwrap(),
                    LiveStatus::Running
                );
                assert!(left[..37].iter().all(|&sample| sample == 0.125));
                assert!(right[..37].iter().all(|&sample| sample == -0.25));
                ready.send(()).unwrap();
                loop {
                    let status = audio
                        .process(
                            &[&[0.125; 128], &[-0.25; 128]],
                            &mut [&mut left, &mut right],
                            37,
                        )
                        .unwrap();
                    if status == LiveStatus::Stopped {
                        assert_eq!(left[..37], [0.0; 37]);
                        assert_eq!(right[..37], [0.0; 37]);
                        break;
                    }
                    std::thread::yield_now();
                }
                audio
            });
            receive.recv().unwrap();
            assert_eq!(session.service().unwrap(), LiveStatus::Running);
            session.request_stop();
            while session.service().unwrap() != LiveStatus::Stopped {
                std::thread::yield_now();
            }
            drop(job.join().unwrap());
        });
        assert_eq!(session.latency_samples(), 0);
        assert!(session.reconfigure(f64::NAN, 64).is_err());
        assert_eq!(session.service().unwrap(), LiveStatus::Stopped);
        if cycle == 0 {
            next_audio = Some(session.reconfigure(44100.0, 64).unwrap());
        }
    }
}

#[test]
#[ignore = "requires NICE_RACK_TEST_CLAP pointing to a freshly built repository nice-tuner CLAP export"]
fn generic_mute_controls_and_native_state_are_independent_per_instance() {
    use nice_clap_host::{Binding, Entry, LiveSession};
    use nice_rack_engine::{Channels, Connection, NodeId, NodeKind, RackGraph};
    use std::collections::BTreeMap;
    let path = std::env::var_os("NICE_RACK_TEST_CLAP").unwrap();
    // SAFETY: explicit opt-in to this repository's own freshly built plugin.
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
    let bindings = BTreeMap::from([(
        NodeId(1),
        Binding {
            entry,
            class: "com.nice-tuner.nice-tuner".into(),
        },
    )]);
    let (mut session, mut audio) =
        LiveSession::prepare(&graph, &bindings, 48000.0, 32, &BTreeMap::new()).unwrap();
    assert!(session.editor_supported(NodeId(1)));
    let mute = session
        .parameters(NodeId(1))
        .into_iter()
        .find(|p| p.name == "Mute")
        .unwrap();
    assert_eq!(mute.value, 0.0);
    assert!(mute.stepped);
    assert!(session.set_parameter(NodeId(1), mute.id, f64::NAN).is_err());
    session.set_parameter(NodeId(1), mute.id, 1.0).unwrap();
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let mut left = [0.0; 32];
                let mut right = [0.0; 32];
                audio
                    .process(
                        &[&[0.25; 32], &[-0.5; 32]],
                        &mut [&mut left, &mut right],
                        32,
                    )
                    .unwrap();
                assert_eq!(left, [0.0; 32]);
                assert_eq!(right, [0.0; 32]);
            })
            .join()
            .unwrap();
    });
    let states = session.save_states().unwrap(); // active main-thread state save
    session.request_stop();
    std::thread::spawn(move || audio.process(&[], &mut [], 0).unwrap())
        .join()
        .unwrap();
    session.service().unwrap();
    // Inactive controls must flush immediately, without an audio callback.
    session.set_parameter(NodeId(1), mute.id, 0.0).unwrap();
    assert_eq!(
        session
            .parameters(NodeId(1))
            .into_iter()
            .find(|p| p.id == mute.id)
            .unwrap()
            .value,
        0.0
    );
    let (mut restored, audio) =
        LiveSession::prepare(&graph, &bindings, 44100.0, 32, &states).unwrap();
    assert_eq!(
        restored
            .parameters(NodeId(1))
            .into_iter()
            .find(|p| p.id == mute.id)
            .unwrap()
            .value,
        1.0
    );
    restored.request_stop();
    std::thread::spawn(move || {
        let mut audio = audio;
        audio.process(&[], &mut [], 0).unwrap()
    })
    .join()
    .unwrap();
    restored.service().unwrap();
}
