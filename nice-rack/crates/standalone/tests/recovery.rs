//! Opt-in integration against this repository's own CLAP export, without devices/GPU.
#![deny(unsafe_op_in_unsafe_fn)]
use nice_clap_host::{Entry, LiveSession, LiveStatus};
use nice_rack::{
    routing::NodeId,
    runtime::{Card, Runtime, chain, spawn_dsp},
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires NICE_RACK_TEST_CLAP pointing to a freshly built repository nice-tuner export"]
fn native_rack_edits_and_device_independent_shutdown() {
    let path =
        PathBuf::from(std::env::var_os("NICE_RACK_TEST_CLAP").expect("Set NICE_RACK_TEST_CLAP"));
    // SAFETY: explicit test opt-in to the repository's own native plugin.
    let entry = unsafe { Entry::load(&path).unwrap() };
    let card = |id| {
        Card::new(
            NodeId(id),
            path.clone(),
            entry.clone(),
            "com.nice-tuner.nice-tuner".into(),
            "Nice Tuner".into(),
        )
        .unwrap()
    };
    let cards = vec![card(1), card(3)];
    // The same arrangement operation is used for internal drags and new files.
    let parallel = nice_rack::runtime::arrange(
        cards.clone(),
        NodeId(3),
        nice_rack::layout::Target {
            card: NodeId(1),
            side: nice_rack::layout::Side::Below,
        },
    )
    .unwrap();
    let (parallel_graph, parallel_bindings) = chain(&parallel);
    let classes = parallel_bindings
        .iter()
        .map(|(&id, binding)| (id, binding.class.clone()))
        .collect();
    let samples = vec![vec![0.125; 33], vec![-0.25; 33]];
    let rendered = entry
        .render_graph(&parallel_graph, &classes, &samples, 48000.0, 16)
        .unwrap();
    assert!(rendered.audio[0].iter().all(|&sample| sample == 0.25));
    assert!(rendered.audio[1].iter().all(|&sample| sample == -0.5));
    let serial = nice_rack::runtime::arrange(
        parallel.clone(),
        NodeId(3),
        nice_rack::layout::Target {
            card: NodeId(1),
            side: nice_rack::layout::Side::Right,
        },
    )
    .unwrap();
    let (serial_graph, _) = chain(&serial);
    let rendered = entry
        .render_graph(&serial_graph, &classes, &samples, 48000.0, 16)
        .unwrap();
    assert_eq!(rendered.audio, samples);
    let reduced = parallel
        .into_iter()
        .filter(|card| card.id != NodeId(1))
        .collect::<Vec<_>>();
    assert!(
        !chain(&reduced)
            .0
            .nodes
            .values()
            .any(|node| matches!(node, nice_rack::routing::NodeKind::Mixer(_)))
    );

    let (graph, bindings) = chain(&cards);
    let (mut session, audio) =
        LiveSession::prepare(&graph, &bindings, 48000.0, 1024, &BTreeMap::new()).unwrap();
    let (mut capture, input) = rtrb::RingBuffer::new(64);
    let (output, mut monitor) = rtrb::RingBuffer::new(64);
    for _ in 0..16 {
        capture.push([0.125, -0.25]).unwrap();
    }
    let worker = spawn_dsp(
        audio,
        nice_common::audio_io::Endpoints { input, output },
        Arc::default(),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while monitor.slots() < 16 {
        session.service().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    // No more capture data is supplied: shutdown must not depend on callbacks.
    session.request_stop();
    while !worker.is_finished() {
        session.service().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let _ = worker.join().unwrap();
    assert_eq!(session.service().unwrap(), LiveStatus::Stopped);
    assert_eq!(monitor.pop().unwrap().0, [0.125, -0.25]);
    assert_eq!(session.snapshot().unwrap().len(), 2);
    let mut runtime = Runtime::new(nice_rack::Settings::default());
    settle(&mut runtime);
    runtime.edit(cards);
    settle(&mut runtime);
    assert_eq!(runtime.cards.len(), 2);
    let mut reordered = runtime.cards.clone();
    reordered.swap(0, 1);
    runtime.edit(reordered);
    settle(&mut runtime);
    assert_eq!(
        runtime
            .cards
            .iter()
            .map(|card| card.id.0)
            .collect::<Vec<_>>(),
        [3, 1]
    );
    let mut removed = runtime.cards.clone();
    removed.remove(0);
    runtime.edit(removed);
    settle(&mut runtime);
    assert_eq!(runtime.cards.len(), 1);
    assert_eq!(runtime.cards[0].id, NodeId(1));
    runtime.close();
    runtime.tick();
    assert!(runtime.stopped());
}
fn settle(runtime: &mut Runtime) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime.busy() {
        runtime.tick();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}
