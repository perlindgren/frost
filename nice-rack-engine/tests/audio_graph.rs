#![deny(unsafe_code)]
use nice_rack_engine::{audio::*, *};
use std::collections::BTreeMap;
use std::sync::Arc;

struct Effect {
    gain: f32,
    delayed: bool,
    history: [f32; 3],
    cursor: usize,
    fail: bool,
}
impl Processor for Effect {
    fn latency_samples(&self) -> usize {
        if self.delayed { 3 } else { 0 }
    }
    fn process(
        &mut self,
        input: &AudioBuffer,
        output: &mut AudioBuffer,
        frames: usize,
    ) -> Result<(), ProcessError> {
        if self.fail {
            return Err(ProcessError::ProcessorFailed);
        }
        for frame in 0..frames {
            let sample = input.channel(0, frames)[frame] * self.gain;
            output.channel_mut(0, frames)[frame] = if self.delayed {
                let previous = self.history[self.cursor];
                self.history[self.cursor] = sample;
                self.cursor = (self.cursor + 1) % 3;
                previous
            } else {
                sample
            };
        }
        Ok(())
    }
    fn reset(&mut self) {
        self.history.fill(0.0);
        self.cursor = 0;
    }
}
fn slot(gain: f32, delayed: bool, bypassed: bool) -> ProcessorSlot {
    ProcessorSlot {
        processor: Box::new(Effect {
            gain,
            delayed,
            history: [0.0; 3],
            cursor: 0,
            fail: false,
        }),
        bypassed,
    }
}
fn graph() -> RackGraph {
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
            (
                NodeId(2),
                NodeKind::Plugin {
                    inputs: Channels::Mono,
                    outputs: Channels::Mono,
                },
            ),
            (NodeId(3), NodeKind::Mixer(Channels::Mono)),
            (NodeId(4), NodeKind::Output(Channels::Mono)),
        ]
        .into(),
        connections: [(0, 1), (0, 2), (1, 3), (2, 3), (3, 4)]
            .map(|(a, b)| Connection {
                from: NodeId(a),
                to: NodeId(b),
            })
            .into(),
    }
}
#[test]
fn parallel_branches_sum_without_clipping_or_implicit_gain_changes() {
    let mut rack = PreparedRack::prepare(
        &graph(),
        [
            (NodeId(1), slot(2.0, false, false)),
            (NodeId(2), slot(3.0, false, false)),
        ]
        .into(),
        8,
    )
    .unwrap();
    let mut output = [0.0; 4];
    rack.process(&[&[1.0, -1.0, 0.25, 0.0]], &mut [&mut output], 4)
        .unwrap();
    assert_eq!(output, [5.0, -5.0, 1.25, 0.0]);
}
#[test]
fn latency_compensation_and_dry_bypass_survive_variable_blocks_and_reset() {
    for bypass in [false, true] {
        let mut rack = PreparedRack::prepare(
            &graph(),
            [
                (NodeId(1), slot(1.0, false, false)),
                (
                    NodeId(2),
                    slot(if bypass { 7.0 } else { 1.0 }, true, bypass),
                ),
            ]
            .into(),
            4,
        )
        .unwrap();
        assert_eq!(rack.latency_samples(), 3);
        for _ in 0..2 {
            let mut result = Vec::new();
            for (index, frames) in [2, 1, 4].into_iter().enumerate() {
                let mut input = [0.0; 4];
                if index == 0 {
                    input[0] = 1.0;
                }
                let mut output = [0.0; 4];
                rack.process(&[&input], &mut [&mut output], frames).unwrap();
                result.extend_from_slice(&output[..frames]);
            }
            assert_eq!(result, [0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0]);
            rack.reset();
        }
    }
}
#[test]
fn serial_processing_and_explicit_channel_conversions_preserve_the_signal() {
    let graph = RackGraph {
        nodes: [
            (NodeId(0), NodeKind::Input(Channels::Stereo)),
            (
                NodeId(1),
                NodeKind::ChannelMap {
                    inputs: Channels::Stereo,
                    outputs: Channels::Mono,
                },
            ),
            (
                NodeId(2),
                NodeKind::Plugin {
                    inputs: Channels::Mono,
                    outputs: Channels::Mono,
                },
            ),
            (
                NodeId(3),
                NodeKind::Plugin {
                    inputs: Channels::Mono,
                    outputs: Channels::Mono,
                },
            ),
            (
                NodeId(4),
                NodeKind::ChannelMap {
                    inputs: Channels::Mono,
                    outputs: Channels::Stereo,
                },
            ),
            (NodeId(5), NodeKind::Output(Channels::Stereo)),
        ]
        .into(),
        connections: [(0, 1), (1, 2), (2, 3), (3, 4), (4, 5)]
            .map(|(a, b)| Connection {
                from: NodeId(a),
                to: NodeId(b),
            })
            .into(),
    };
    let mut rack = PreparedRack::prepare(
        &graph,
        [
            (NodeId(2), slot(2.0, false, false)),
            (NodeId(3), slot(3.0, false, false)),
        ]
        .into(),
        4,
    )
    .unwrap();
    let mut left = [0.0; 2];
    let mut right = [0.0; 2];
    rack.process(&[&[1.0, 0.0], &[0.0, 1.0]], &mut [&mut left, &mut right], 2)
        .unwrap();
    assert_eq!(left, [3.0, 3.0]);
    assert_eq!(left, right);
}
#[test]
fn invalid_calls_and_failed_nodes_silence_the_external_output() {
    let mut effects = BTreeMap::from([
        (NodeId(1), slot(1.0, false, false)),
        (NodeId(2), slot(1.0, false, false)),
    ]);
    effects.insert(
        NodeId(2),
        ProcessorSlot {
            processor: Box::new(Effect {
                gain: 1.0,
                delayed: false,
                history: [0.0; 3],
                cursor: 0,
                fail: true,
            }),
            bypassed: false,
        },
    );
    let mut rack = PreparedRack::prepare(&graph(), effects, 4).unwrap();
    let mut output = [99.0; 4];
    assert_eq!(
        rack.process(&[&[1.0; 4]], &mut [&mut output], 4),
        Err(ProcessError::NodeFailed(NodeId(2)))
    );
    assert_eq!(output, [0.0; 4]);
    output.fill(99.0);
    assert_eq!(
        rack.process(&[], &mut [&mut output], 4),
        Err(ProcessError::InvalidBuffers)
    );
    assert_eq!(output, [0.0; 4]);
}
#[test]
fn prepared_processing_delay_and_reset_do_not_allocate() {
    let mut rack = PreparedRack::prepare(
        &graph(),
        [
            (NodeId(1), slot(1.0, false, false)),
            (NodeId(2), slot(1.0, true, false)),
        ]
        .into(),
        4,
    )
    .unwrap();
    // Metering is attached too: publishing must stay allocation-free.
    rack.attach_wire_levels(Arc::new(WireLevels::new(4)));
    rack.attach_processing_times(Arc::new(ProcessingTimes::new(2)), 48_000.0);
    let mut output = [0.0; 4];
    let allocations = allocation_counter::measure(|| {
        for frames in [4, 1, 3, 2, 4] {
            rack.process(&[&[1.0; 4]], &mut [&mut output], frames)
                .unwrap();
        }
        rack.reset();
        assert_eq!(
            rack.process(&[], &mut [&mut output], 4),
            Err(ProcessError::InvalidBuffers)
        );
    });
    assert_eq!(allocations.count_total, 0);
}
#[test]
fn preparation_rejects_missing_processors_and_excessive_delays() {
    assert!(matches!(
        PreparedRack::prepare(&graph(), BTreeMap::new(), 4),
        Err(PrepareError::MissingProcessor(_))
    ));
    struct Excessive;
    impl Processor for Excessive {
        fn latency_samples(&self) -> usize {
            usize::MAX
        }
        fn process(
            &mut self,
            _: &AudioBuffer,
            _: &mut AudioBuffer,
            _: usize,
        ) -> Result<(), ProcessError> {
            Ok(())
        }
    }
    let effects = [
        (NodeId(1), slot(1.0, false, false)),
        (
            NodeId(2),
            ProcessorSlot {
                processor: Box::new(Excessive),
                bypassed: false,
            },
        ),
    ]
    .into();
    assert!(matches!(
        PreparedRack::prepare(&graph(), effects, 4),
        Err(PrepareError::BufferBudgetExceeded)
    ));
}
#[test]
fn attached_wires_publish_each_plugins_input_and_output() {
    let mut rack = PreparedRack::prepare(
        &graph(),
        [
            (NodeId(1), slot(2.0, false, false)),
            (NodeId(2), slot(0.5, false, false)),
        ]
        .into(),
        4,
    )
    .unwrap();
    let levels = Arc::new(WireLevels::new(4));
    rack.attach_wire_levels(Arc::clone(&levels));
    let mut output = [0.0; 4];
    let block = [1.0, -1.0, 0.5, 0.0];
    let input_rms = (2.25f32 / 4.0).sqrt();
    // A wire's first block arrives on an empty estimator, which reconstructs
    // the crest of the arriving edge rather than reporting a maximum sample:
    // here 1.0183787, above the loudest sample because the interpolated
    // step really does overshoot full scale.
    rack.process(&[&block], &mut [&mut output], 4).unwrap();
    assert_eq!(
        levels.sample(NodeId(1).0 << 1),
        Some((input_rms, 1.018_378_7))
    );
    // One block later the same pattern is a steady signal, so what the wire
    // publishes is its inter-sample crest. Both branches see the input block,
    // rms sqrt(2.25/4) = 0.75 with a crest of 1.0, and each output reflects
    // its own gain: true peak scales with the signal, so 2.0 and 0.5.
    rack.process(&[&block], &mut [&mut output], 4).unwrap();
    assert_eq!(levels.sample(NodeId(1).0 << 1), Some((input_rms, 1.0)));
    assert_eq!(levels.sample(NodeId(1).0 << 1 | 1), Some((1.5, 2.0)));
    assert_eq!(levels.sample(NodeId(2).0 << 1), Some((input_rms, 1.0)));
    assert_eq!(levels.sample(NodeId(2).0 << 1 | 1), Some((0.375, 0.5)));
    // The mixer is not a plugin, so its small id gets no meter.
    assert_eq!(levels.sample(NodeId(3).0 << 1), None);
    // Without a fresh process call the keys still report the last block.
    assert_eq!(levels.sample(NodeId(1).0 << 1), Some((input_rms, 1.0)));
}
#[test]
fn wire_registration_resets_and_declines_wires_beyond_capacity() {
    let mut rack = PreparedRack::prepare(
        &graph(),
        [
            (NodeId(1), slot(1.0, false, false)),
            (NodeId(2), slot(1.0, false, false)),
        ]
        .into(),
        4,
    )
    .unwrap();
    let first = Arc::new(WireLevels::new(3));
    rack.attach_wire_levels(Arc::clone(&first));
    // Registration follows the deterministic plan order, so the third slot
    // truncates the second card: its output is declined, not overwritten.
    assert!(first.sample(NodeId(1).0 << 1).is_some());
    assert!(first.sample(NodeId(1).0 << 1 | 1).is_some());
    assert!(first.sample(NodeId(2).0 << 1).is_some());
    assert_eq!(first.sample(NodeId(2).0 << 1 | 1), None);
    // A rebuild re-attaches onto a fresh table: cleared keys never block the
    // new registration, and the old table is left alone.
    let second = Arc::new(WireLevels::new(4));
    rack.attach_wire_levels(Arc::clone(&second));
    assert!(second.sample(NodeId(2).0 << 1 | 1).is_some());
    assert_eq!(second.sample(NodeId(0).0), None);
}
#[test]
fn wire_trims_scale_the_signal_and_its_meter_together() {
    let mut rack = PreparedRack::prepare(
        &graph(),
        [
            (NodeId(1), slot(2.0, false, false)),
            (NodeId(2), slot(0.5, false, false)),
        ]
        .into(),
        4,
    )
    .unwrap();
    let levels = Arc::new(WireLevels::new(4));
    rack.attach_wire_levels(Arc::clone(&levels));
    // Half gain on card 1's input wire, quadruple on card 2's output wire.
    // The input trim lands before the processor, so plugin, meter and the
    // downstream mixer all share what the wire carries.
    levels.set_gain(NodeId(1).0 << 1, 0.5);
    levels.set_gain(NodeId(2).0 << 1 | 1, 4.0);
    let mut output = [0.0; 4];
    // The first block is an onset, so it reads above its loudest sample; the
    // repeating pattern below reads its inter-sample crest from block two on
    // (see `attached_wires_publish_each_plugins_input_and_output`).
    rack.process(&[&[1.0, -1.0, 0.5, 0.0]], &mut [&mut output], 4)
        .unwrap();
    rack.process(&[&[1.0, -1.0, 0.5, 0.0]], &mut [&mut output], 4)
        .unwrap();
    assert_eq!(levels.sample(NodeId(1).0 << 1), Some((0.375, 0.5)));
    assert_eq!(levels.sample(NodeId(1).0 << 1 | 1), Some((0.75, 1.0)));
    assert_eq!(levels.sample(NodeId(2).0 << 1), Some((0.75, 1.0)));
    assert_eq!(levels.sample(NodeId(2).0 << 1 | 1), Some((1.5, 2.0)));
    // Audible proof, not just metering: the summed output reflects both trims.
    assert_eq!(output, [3.0, -3.0, 1.5, 0.0]);
}
#[test]
fn a_rebuild_keeps_live_trims_and_retires_cards_that_left() {
    let levels = Arc::new(WireLevels::new(4));
    let mut rack = PreparedRack::prepare(
        &graph(),
        [
            (NodeId(1), slot(1.0, false, false)),
            (NodeId(2), slot(1.0, false, false)),
        ]
        .into(),
        4,
    )
    .unwrap();
    rack.attach_wire_levels(Arc::clone(&levels));
    levels.set_gain(NodeId(1).0 << 1 | 1, 0.25);
    levels.set_gain(NodeId(2).0 << 1, 2.0);
    // A rebuild re-registers the same plan on the same table: the trims the
    // user set must survive it.
    rack.attach_wire_levels(Arc::clone(&levels));
    assert_eq!(levels.gain_of(NodeId(1).0 << 1 | 1), Some(0.25));
    assert_eq!(levels.gain_of(NodeId(2).0 << 1), Some(2.0));
    let mut output = [0.0; 4];
    rack.process(&[&[1.0; 4]], &mut [&mut output], 4).unwrap();
    assert_eq!(levels.sample(NodeId(1).0 << 1 | 1), Some((0.25, 0.25)));
    assert_eq!(levels.sample(NodeId(2).0 << 1), Some((2.0, 2.0)));
    assert_eq!(output, [2.25; 4]);
    // A card removed from the rack retires: its key, its trim and its slot.
    let smaller = RackGraph {
        nodes: [
            (NodeId(0), NodeKind::Input(Channels::Mono)),
            (
                NodeId(1),
                NodeKind::Plugin {
                    inputs: Channels::Mono,
                    outputs: Channels::Mono,
                },
            ),
            (NodeId(4), NodeKind::Output(Channels::Mono)),
        ]
        .into(),
        connections: [(0, 1), (1, 4)]
            .map(|(a, b)| Connection {
                from: NodeId(a),
                to: NodeId(b),
            })
            .into(),
    };
    let mut smaller =
        PreparedRack::prepare(&smaller, [(NodeId(1), slot(1.0, false, false))].into(), 4).unwrap();
    smaller.attach_wire_levels(Arc::clone(&levels));
    assert_eq!(levels.sample(NodeId(2).0 << 1), None);
    assert_eq!(levels.gain_of(NodeId(2).0 << 1), None);
    assert_eq!(levels.gain_of(NodeId(1).0 << 1 | 1), Some(0.25));
    // Retirement freed the slots, so the full plan fits again on a rebuild.
    rack.attach_wire_levels(Arc::clone(&levels));
    assert!(levels.sample(NodeId(2).0 << 1 | 1).is_some());
}
#[test]
fn gain_requests_decline_unknown_keys_and_rude_values() {
    let mut rack = PreparedRack::prepare(
        &graph(),
        [
            (NodeId(1), slot(1.0, false, false)),
            (NodeId(2), slot(1.0, false, false)),
        ]
        .into(),
        4,
    )
    .unwrap();
    let levels = Arc::new(WireLevels::new(4));
    // Before any plan registers wires, every key is unregistered.
    levels.set_gain(NodeId(1).0 << 1, 0.5);
    assert_eq!(levels.gain_of(NodeId(1).0 << 1), None);
    assert_eq!(levels.gain_of(0), None);
    rack.attach_wire_levels(Arc::clone(&levels));
    levels.set_gain(NodeId(1).0 << 1 | 1, f32::NAN);
    assert_eq!(levels.gain_of(NodeId(1).0 << 1 | 1), Some(1.0));
    // Out-of-range demands clamp instead of stacking grossly overdriven
    // plugins behind one slider gesture.
    levels.set_gain(NodeId(1).0 << 1 | 1, 100.0);
    assert_eq!(levels.gain_of(NodeId(1).0 << 1 | 1), Some(16.0));
}

#[test]
fn each_cards_budget_accumulates_in_execution_order_and_uses_the_actual_block_deadline() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};
    struct Work(Arc<AtomicU64>);
    impl Processor for Work {
        fn process(
            &mut self,
            input: &AudioBuffer,
            output: &mut AudioBuffer,
            frames: usize,
        ) -> Result<(), ProcessError> {
            let started = Instant::now();
            while started.elapsed() < Duration::from_micros(100) {
                std::hint::spin_loop();
            }
            output
                .channel_mut(0, frames)
                .copy_from_slice(input.channel(0, frames));
            self.0
                .store(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
            Ok(())
        }
    }
    let elapsed = Arc::new(AtomicU64::new(0));
    let mut rack = PreparedRack::prepare(
        &graph(),
        [
            (
                NodeId(1),
                ProcessorSlot {
                    processor: Box::new(Work(elapsed.clone())),
                    bypassed: false,
                },
            ),
            (NodeId(2), slot(1.0, false, false)),
        ]
        .into(),
        128,
    )
    .unwrap();
    let times = Arc::new(ProcessingTimes::new(2));
    rack.attach_processing_times(times.clone(), 48_000.0);
    for frames in [1, 64, 128] {
        let mut output = [0.0; 128];
        rack.process(&[&[0.25; 128]], &mut [&mut output], frames)
            .unwrap();
        let first = times.sample(NodeId(1)).unwrap();
        let second = times.sample(NodeId(2)).unwrap();
        let measured = elapsed.load(Ordering::Relaxed) as f64 * 1e-9;
        assert!(
            first.added + 1e-10 >= measured * 48_000.0 / frames as f64,
            "the denominator must be this call's frames, not the prepared maximum"
        );
        assert!(
            second.incoming >= first.total(),
            "parallel branches share this single processing thread's budget"
        );
        assert!(first.incoming >= 0.0 && second.added >= 0.0);
        assert_eq!(output[..frames], vec![0.5; frames]);
    }
    // A new plan clears measurements for removed and retained cards alike.
    let empty = RackGraph {
        nodes: [
            (NodeId(0), NodeKind::Input(Channels::Mono)),
            (NodeId(4), NodeKind::Output(Channels::Mono)),
        ]
        .into(),
        connections: vec![Connection {
            from: NodeId(0),
            to: NodeId(4),
        }],
    };
    let mut rebuilt = PreparedRack::prepare(&empty, BTreeMap::new(), 128).unwrap();
    rebuilt.attach_processing_times(times.clone(), 48_000.0);
    assert_eq!(times.sample(NodeId(1)), None);
    assert_eq!(times.sample(NodeId(2)), None);
}
