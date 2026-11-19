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
        ..RackGraph::default()
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
    rack.process(
        &[&[1.0, -1.0, 0.25, 0.0]],
        &mut [&mut [&mut output[..]][..]],
        4,
    )
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
                rack.process(&[&input], &mut [&mut [&mut output[..]][..]], frames)
                    .unwrap();
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
        ..RackGraph::default()
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
    rack.process(
        &[&[1.0, 0.0], &[0.0, 1.0]],
        &mut [&mut [&mut left[..], &mut right[..]][..]],
        2,
    )
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
        rack.process(&[&[1.0; 4]], &mut [&mut [&mut output[..]][..]], 4),
        Err(ProcessError::NodeFailed(NodeId(2)))
    );
    assert_eq!(output, [0.0; 4]);
    output.fill(99.0);
    assert_eq!(
        rack.process(&[], &mut [&mut [&mut output[..]][..]], 4),
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
            rack.process(&[&[1.0; 4]], &mut [&mut [&mut output[..]][..]], frames)
                .unwrap();
        }
        rack.reset();
        assert_eq!(
            rack.process(&[], &mut [&mut [&mut output[..]][..]], 4),
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
    rack.process(&[&block], &mut [&mut [&mut output[..]][..]], 4)
        .unwrap();
    assert_eq!(
        levels.sample(NodeId(1).0 << 1),
        Some((input_rms, 1.018_378_7))
    );
    // One block later the same pattern is a steady signal, so what the wire
    // publishes is its inter-sample crest. Both branches see the input block,
    // rms sqrt(2.25/4) = 0.75 with a crest of 1.0, and each output reflects
    // its own gain: true peak scales with the signal, so 2.0 and 0.5.
    rack.process(&[&block], &mut [&mut [&mut output[..]][..]], 4)
        .unwrap();
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
fn card_keys_stay_clear_of_the_master_key() {
    assert_eq!(card_wire_key(2, false), Some(4));
    assert_eq!(card_wire_key(2, true), Some(5));
    // The last id a card may own: its output side is two below the master.
    assert_eq!(card_wire_key((u64::MAX >> 1) - 1, true), Some(u64::MAX - 2));
    // `u64::MAX >> 1` folds onto `MASTER_WIRE_KEY` on the output side, and
    // larger ids shift outright; neither can be a card.
    assert_eq!(card_wire_key(u64::MAX >> 1, true), None);
    assert_eq!(card_wire_key(u64::MAX, true), None);
}
#[test]
fn master_output_trim_scales_and_meters_the_summed_signal() {
    // The rack's own output wire: the primary `Output` node is addressed
    // `MASTER_WIRE_KEY`, so its gain stage trims what the mixer summed from
    // every branch, and its meter reads the post-trim signal — what leaves
    // for the DAC.
    let graph = RackGraph {
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
            (NodeId(MASTER_WIRE_KEY), NodeKind::Output(Channels::Mono)),
        ]
        .into(),
        connections: [(0, 1), (0, 2), (1, 3), (2, 3), (3, MASTER_WIRE_KEY)]
            .map(|(a, b)| Connection {
                from: NodeId(a),
                to: NodeId(b),
            })
            .into(),
        ..RackGraph::default()
    };
    let processors = || {
        [
            (NodeId(1), slot(1.0, false, false)),
            (NodeId(2), slot(1.0, false, false)),
        ]
        .into()
    };
    let mut rack = PreparedRack::prepare(&graph, processors(), 4).unwrap();
    let levels = Arc::new(WireLevels::new(8));
    rack.attach_wire_levels(Arc::clone(&levels));
    // Both branches are unity and the mixer sums at unity, so the jack would
    // carry the block doubled; a master trim of 0.5 hands it over whole.
    levels.set_gain(MASTER_WIRE_KEY, 0.5);
    let mut output = [0.0; 4];
    let block = [1.0, -1.0, 0.5, 0.0];
    let rms = (2.25f32 / 4.0).sqrt();
    rack.process(&[&block], &mut [&mut [&mut output[..]][..]], 4)
        .unwrap();
    assert_eq!(output, block);
    // The meter reads the wire after its trim, and the estimator starts
    // empty, so the first block reports the reconstructed crest (the same
    // 1.018… a plugin input wire shows for this block).
    assert_eq!(levels.sample(MASTER_WIRE_KEY), Some((rms, 1.018_378_7)));
    rack.process(&[&block], &mut [&mut [&mut output[..]][..]], 4)
        .unwrap();
    assert_eq!(levels.sample(MASTER_WIRE_KEY), Some((rms, 1.0)));
    // A rebuild re-registers the master key, so the trim the operator set
    // survives the plan swap instead of silently reverting to unity.
    let mut rebuilt = PreparedRack::prepare(&graph, processors(), 4).unwrap();
    rebuilt.attach_wire_levels(Arc::clone(&levels));
    rebuilt
        .process(&[&block], &mut [&mut [&mut output[..]][..]], 4)
        .unwrap();
    assert_eq!(output, block);
    assert_eq!(levels.gain_of(MASTER_WIRE_KEY), Some(0.5));
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
    rack.process(
        &[&[1.0, -1.0, 0.5, 0.0]],
        &mut [&mut [&mut output[..]][..]],
        4,
    )
    .unwrap();
    rack.process(
        &[&[1.0, -1.0, 0.5, 0.0]],
        &mut [&mut [&mut output[..]][..]],
        4,
    )
    .unwrap();
    assert_eq!(levels.sample(NodeId(1).0 << 1), Some((0.375, 0.5)));
    assert_eq!(levels.sample(NodeId(1).0 << 1 | 1), Some((0.75, 1.0)));
    assert_eq!(levels.sample(NodeId(2).0 << 1), Some((0.75, 1.0)));
    assert_eq!(levels.sample(NodeId(2).0 << 1 | 1), Some((1.5, 2.0)));
    // Audible proof, not just metering: the summed output reflects both trims.
    assert_eq!(output, [3.0, -3.0, 1.5, 0.0]);
}
#[test]
fn the_master_switch_silences_the_jack_without_blinding_its_meter() {
    // The jack's wire again, from `master_output_trim_scales_and_meters_the_summed_signal`:
    // the primary `Output` node is addressed `MASTER_WIRE_KEY`, and it now
    // carries a break as well as a trim and a meter.
    let graph = RackGraph {
        nodes: [
            (NodeId(0), NodeKind::Input(Channels::Mono)),
            (
                NodeId(1),
                NodeKind::Plugin {
                    inputs: Channels::Mono,
                    outputs: Channels::Mono,
                },
            ),
            (NodeId(3), NodeKind::Mixer(Channels::Mono)),
            (NodeId(MASTER_WIRE_KEY), NodeKind::Output(Channels::Mono)),
        ]
        .into(),
        connections: [(0, 1), (1, 3), (3, MASTER_WIRE_KEY)]
            .map(|(a, b)| Connection {
                from: NodeId(a),
                to: NodeId(b),
            })
            .into(),
        ..RackGraph::default()
    };
    let mut rack =
        PreparedRack::prepare(&graph, [(NodeId(1), slot(1.0, false, false))].into(), 4).unwrap();
    let levels = Arc::new(WireLevels::new(8));
    rack.attach_wire_levels(Arc::clone(&levels));
    let mut output = [0.0; 4];
    rack.process(&[&[1.0; 4]], &mut [&mut [&mut output[..]][..]], 4)
        .unwrap();
    assert_eq!(output, [1.0; 4]);
    // Break the wire into the output, as a right-click on the run does.
    levels.set_open(MASTER_WIRE_KEY, true);
    rack.process(&[&[1.0; 4]], &mut [&mut [&mut output[..]][..]], 4)
        .unwrap();
    assert_eq!(output, [0.0; 4]);
    // The meter measures before the break: it reads what arrives at the
    // stub, so an open jack still says the rack is alive — the whole
    // difference between an unplugged wire and a switched-off amplifier.
    let (rms, peak) = levels.sample(MASTER_WIRE_KEY).unwrap();
    assert!(
        rms > 0.0 && peak > 0.0,
        "open jack must still meter: {rms} {peak}"
    );
    // Closing the circuit lets the sound through again.
    levels.set_open(MASTER_WIRE_KEY, false);
    rack.process(&[&[1.0; 4]], &mut [&mut [&mut output[..]][..]], 4)
        .unwrap();
    assert_eq!(output, [1.0; 4]);
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
    rack.process(&[&[1.0; 4]], &mut [&mut [&mut output[..]][..]], 4)
        .unwrap();
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
        ..RackGraph::default()
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
        rack.process(&[&[0.25; 128]], &mut [&mut [&mut output[..]][..]], frames)
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
        ..RackGraph::default()
    };
    let mut rebuilt = PreparedRack::prepare(&empty, BTreeMap::new(), 128).unwrap();
    rebuilt.attach_processing_times(times.clone(), 48_000.0);
    assert_eq!(times.sample(NodeId(1)), None);
    assert_eq!(times.sample(NodeId(2)), None);
}

#[test]
fn live_bypass_is_unity_keeps_processing_and_preserves_untrimmed_latency_history() {
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Counted {
        effect: Effect,
        calls: Arc<AtomicU64>,
    }
    impl Processor for Counted {
        fn latency_samples(&self) -> usize {
            3
        }
        fn process(
            &mut self,
            input: &AudioBuffer,
            output: &mut AudioBuffer,
            frames: usize,
        ) -> Result<(), ProcessError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.effect.process(input, output, frames)
        }
    }
    let calls = Arc::new(AtomicU64::new(0));
    let graph = RackGraph {
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
        ..RackGraph::default()
    };
    let mut rack = PreparedRack::prepare(
        &graph,
        [(
            NodeId(1),
            ProcessorSlot {
                processor: Box::new(Counted {
                    calls: calls.clone(),
                    effect: Effect {
                        gain: 2.0,
                        delayed: true,
                        history: [0.0; 3],
                        cursor: 0,
                        fail: false,
                    },
                }),
                bypassed: false,
            },
        )]
        .into(),
        4,
    )
    .unwrap();
    let wires = Arc::new(WireLevels::new(2));
    rack.attach_wire_levels(wires.clone());
    wires.set_gain(2, 4.0);
    wires.set_gain(3, 2.0);
    let mut output = [0.0; 4];
    rack.process(&[&[0.25; 4]], &mut [&mut [&mut output[..]][..]], 4)
        .unwrap();
    assert_eq!(output, [0.0, 0.0, 0.0, 4.0]);
    wires.set_bypassed(NodeId(1), true);
    wires.set_open(2, true);
    wires.set_open(3, true);
    rack.process(
        &[&[0.5, 0.6, 0.7, 0.8]],
        &mut [&mut [&mut output[..]][..]],
        4,
    )
    .unwrap();
    assert_eq!(output, [0.25, 0.25, 0.25, 0.5]);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(rack.latency_samples(), 3);
    assert_eq!(wires.gain_of(2), Some(4.0));
    assert_eq!(wires.gain_of(3), Some(2.0));
    wires.set_bypassed(NodeId(1), false);
    rack.process(&[&[0.4; 4]], &mut [&mut [&mut output[..]][..]], 4)
        .unwrap();
    assert_eq!(
        output, [0.0; 4],
        "the saved cable breaks return when re-enabled"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 3);
    wires.set_open(2, false);
    wires.set_open(3, false);
    let allocations = allocation_counter::measure(|| {
        for bypassed in [true, false, true, false] {
            wires.set_bypassed(NodeId(1), bypassed);
            rack.process(&[&[0.25; 4]], &mut [&mut [&mut output[..]][..]], 4)
                .unwrap();
        }
    });
    assert_eq!(allocations.count_total, 0);
}

#[test]
fn live_bypass_supports_mono_stereo_conversion_and_keeps_parallel_branches() {
    struct WetOutput;
    impl Processor for WetOutput {
        fn process(
            &mut self,
            _: &AudioBuffer,
            output: &mut AudioBuffer,
            frames: usize,
        ) -> Result<(), ProcessError> {
            for channel in 0..output.channels() {
                output.channel_mut(channel, frames).fill(123.0);
            }
            Ok(())
        }
    }
    for (input, output) in [
        (Channels::Mono, Channels::Stereo),
        (Channels::Stereo, Channels::Mono),
    ] {
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
            ..RackGraph::default()
        };
        let mut rack = PreparedRack::prepare(
            &graph,
            [(
                NodeId(1),
                ProcessorSlot {
                    processor: Box::new(WetOutput),
                    bypassed: false,
                },
            )]
            .into(),
            4,
        )
        .unwrap();
        let wires = Arc::new(WireLevels::new(2));
        rack.attach_wire_levels(wires.clone());
        wires.set_bypassed(NodeId(1), true);
        let mut left = [0.0; 4];
        let mut right = [0.0; 4];
        if input == Channels::Mono {
            rack.process(
                &[&[0.75; 4]],
                &mut [&mut [&mut left[..], &mut right[..]][..]],
                4,
            )
            .unwrap();
            assert_eq!(left, [0.75; 4]);
            assert_eq!(right, left);
        } else {
            rack.process(
                &[&[0.75; 4], &[0.25; 4]],
                &mut [&mut [&mut left[..]][..]],
                4,
            )
            .unwrap();
            assert_eq!(left, [0.5; 4]);
        }
    }
    let mut rack = PreparedRack::prepare(
        &graph(),
        [
            (NodeId(1), slot(2.0, false, false)),
            (NodeId(2), slot(3.0, false, false)),
        ]
        .into(),
        4,
    )
    .unwrap();
    let wires = Arc::new(WireLevels::new(4));
    rack.attach_wire_levels(wires.clone());
    wires.set_bypassed(NodeId(1), true);
    let mut output = [0.0; 4];
    rack.process(&[&[0.1; 4]], &mut [&mut [&mut output[..]][..]], 4)
        .unwrap();
    for sample in output {
        assert!((sample - 0.4).abs() < 1e-6);
    }
}

#[test]
fn pause_mutes_audio_and_output_meter_but_can_be_bypassed_independently() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Tail(Arc<AtomicUsize>);
    impl Processor for Tail {
        fn process(
            &mut self,
            _: &AudioBuffer,
            output: &mut AudioBuffer,
            frames: usize,
        ) -> Result<(), ProcessError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            output.channel_mut(0, frames).fill(0.75);
            Ok(())
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let graph = RackGraph {
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
        ..RackGraph::default()
    };
    let mut rack = PreparedRack::prepare(
        &graph,
        [(
            NodeId(1),
            ProcessorSlot {
                processor: Box::new(Tail(calls.clone())),
                bypassed: false,
            },
        )]
        .into(),
        64,
    )
    .unwrap();
    let wires = Arc::new(WireLevels::new(2));
    rack.attach_wire_levels(wires.clone());
    wires.set_gain(2, 2.0);
    wires.set_gain(3, 2.0);
    let mut output = [0.0; 64];
    rack.process(&[&[0.25; 64]], &mut [&mut [&mut output[..]][..]], 64)
        .unwrap();
    assert_eq!(output, [1.5; 64]);
    assert!(wires.sample(3).unwrap().1 > 0.0);
    wires.set_paused(NodeId(1), true);
    rack.process(&[&[0.25; 64]], &mut [&mut [&mut output[..]][..]], 64)
        .unwrap();
    assert_eq!(
        output, [0.0; 64],
        "a paused processor's tail cannot reach its output"
    );
    assert_eq!(
        wires.sample(3),
        Some((0.0, 0.0)),
        "no old true-peak history may leak into the paused meter"
    );
    assert!(
        wires.sample(2).unwrap().0 > 0.0,
        "the input still reads its arriving signal"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    wires.set_bypassed(NodeId(1), true);
    rack.process(&[&[0.25; 64]], &mut [&mut [&mut output[..]][..]], 64)
        .unwrap();
    assert_eq!(
        output, [0.25; 64],
        "dry bypass works even while the plugin remains paused, without either gain trim"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    wires.set_bypassed(NodeId(1), false);
    rack.process(&[&[0.25; 64]], &mut [&mut [&mut output[..]][..]], 64)
        .unwrap();
    assert_eq!(
        output, [0.0; 64],
        "removing bypass returns to the paused branch"
    );
    wires.set_paused(NodeId(1), false);
    rack.process(&[&[0.25; 64]], &mut [&mut [&mut output[..]][..]], 64)
        .unwrap();
    assert_eq!(output, [1.5; 64]);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    let allocations = allocation_counter::measure(|| {
        for paused in [true, false, true, false] {
            wires.set_paused(NodeId(1), paused);
            rack.process(&[&[0.25; 64]], &mut [&mut [&mut output[..]][..]], 64)
                .unwrap();
        }
    });
    assert_eq!(allocations.count_total, 0);
}

#[test]
fn bypass_does_not_replay_old_history_and_tracks_input_while_bypassed() {
    let graph = RackGraph {
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
        connections: [(0, 1), (1, 2)]
            .map(|(from, to)| Connection {
                from: NodeId(from),
                to: NodeId(to),
            })
            .into(),
        ..RackGraph::default()
    };
    // Exercise both live and prepare-time bypass, and partial blocks.
    for prepared_bypass in [false, true] {
        let mut rack = PreparedRack::prepare(
            &graph,
            [(NodeId(1), slot(2.0, true, prepared_bypass))].into(),
            4,
        )
        .unwrap();
        let wires = Arc::new(WireLevels::new(2));
        rack.attach_wire_levels(wires.clone());
        let mut output = [0.0; 4];
        rack.process(&[&[1.0]], &mut [&mut [&mut output[..]][..]], 1)
            .unwrap();
        assert_eq!(output[0], 0.0);
        wires.set_bypassed(NodeId(1), true);
        for _ in 0..8 {
            rack.process(&[&[0.0; 4]], &mut [&mut [&mut output[..]][..]], 4)
                .unwrap();
        }
        // An impulse from before bypass must have left the processor already.
        wires.set_bypassed(NodeId(1), false);
        rack.process(&[&[0.0; 4]], &mut [&mut [&mut output[..]][..]], 4)
            .unwrap();
        assert_eq!(output, [0.0; 4]);
        // New input during bypass must reach the wet history, not just silence.
        wires.set_bypassed(NodeId(1), true);
        rack.process(&[&[0.25]], &mut [&mut [&mut output[..]][..]], 1)
            .unwrap();
        wires.set_bypassed(NodeId(1), false);
        rack.process(&[&[0.0; 4]], &mut [&mut [&mut output[..]][..]], 4)
            .unwrap();
        let expected = if prepared_bypass { 0.25 } else { 0.5 };
        assert_eq!(output, [0.0, 0.0, expected, 0.0]);
    }
}

/// One fork, two buses: card 1 feeds output 10 directly and, through card 2,
/// output 11. Card 2 is only ever audible on the second bus.
fn forked_graph() -> RackGraph {
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
            (NodeId(10), NodeKind::Output(Channels::Mono)),
            (NodeId(11), NodeKind::Output(Channels::Mono)),
        ]
        .into(),
        connections: [(0, 1), (1, 10), (1, 2), (2, 11)]
            .map(|(a, b)| Connection {
                from: NodeId(a),
                to: NodeId(b),
            })
            .into(),
        ..RackGraph::default()
    }
}

/// A gain card that counts how many times the plan evaluates it.
struct Counted {
    gain: f32,
    runs: Arc<std::sync::atomic::AtomicUsize>,
}
impl Processor for Counted {
    fn latency_samples(&self) -> usize {
        0
    }
    fn process(
        &mut self,
        input: &AudioBuffer,
        output: &mut AudioBuffer,
        frames: usize,
    ) -> Result<(), ProcessError> {
        self.runs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        for frame in 0..frames {
            output.channel_mut(0, frames)[frame] = input.channel(0, frames)[frame] * self.gain;
        }
        Ok(())
    }
    fn reset(&mut self) {}
}
fn counted_slot(gain: f32, runs: Arc<std::sync::atomic::AtomicUsize>) -> ProcessorSlot {
    ProcessorSlot {
        processor: Box::new(Counted { gain, runs }),
        bypassed: false,
    }
}

#[test]
fn each_output_bus_hears_exactly_the_cards_upstream_of_it() {
    let runs = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut rack = PreparedRack::prepare(
        &forked_graph(),
        [
            (NodeId(1), counted_slot(2.0, Arc::clone(&runs))),
            (NodeId(2), slot(3.0, false, false)),
        ]
        .into(),
        8,
    )
    .expect("a card that feeds only the second bus is connected");
    assert_eq!(
        rack.output_buses().collect::<Vec<_>>(),
        [(NodeId(10), 1), (NodeId(11), 1)],
        "bus groups must be ordered by ascending Output node id"
    );
    let mut lane = [0.0; 4];
    let mut trunk = [0.0; 4];
    let mut group_lane = [&mut lane[..]];
    let mut group_trunk = [&mut trunk[..]];
    rack.process(
        &[&[1.0, -1.0, 0.5, 0.0]],
        &mut [&mut group_lane[..], &mut group_trunk[..]],
        4,
    )
    .unwrap();
    assert_eq!(lane, [2.0, -2.0, 1.0, 0.0]);
    assert_eq!(trunk, [6.0, -6.0, 3.0, 0.0]);
    // A fork evaluates shared upstream cards exactly once, not once per bus.
    assert_eq!(runs.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[test]
fn processing_requires_exactly_one_group_and_enough_channels_per_bus() {
    let mut rack = PreparedRack::prepare(
        &forked_graph(),
        [
            (NodeId(1), slot(1.0, false, false)),
            (NodeId(2), slot(1.0, false, false)),
        ]
        .into(),
        4,
    )
    .unwrap();
    let input = [1.0; 4];
    let mut a = [0.0; 4];
    let mut b = [0.0; 4];
    let mut spare = [0.0; 4];
    let mut short_channel = [0.0; 3];
    // Two buses need two groups: one lane group alone is refused.
    assert_eq!(
        rack.process(&[&input[..]], &mut [&mut [&mut a[..]][..]], 4),
        Err(ProcessError::InvalidBuffers)
    );
    // Each group must carry exactly the channels of its bus.
    assert_eq!(
        rack.process(
            &[&input[..]],
            &mut [&mut [&mut a[..], &mut spare[..]][..], &mut [&mut b[..]][..]],
            4
        ),
        Err(ProcessError::InvalidBuffers),
        "a mono bus accepts exactly one channel"
    );
    // Every channel must cover the whole block.
    assert_eq!(
        rack.process(
            &[&input[..]],
            &mut [&mut [&mut short_channel[..]][..], &mut [&mut b[..]][..]],
            4
        ),
        Err(ProcessError::InvalidBuffers)
    );
    // The correct arrangement still works after the rejections.
    rack.process(
        &[&input[..]],
        &mut [&mut [&mut a[..]][..], &mut [&mut b[..]][..]],
        4,
    )
    .unwrap();
    assert_eq!(a, [1.0; 4]);
    assert_eq!(b, [1.0; 4]);
}

#[test]
fn latency_reports_the_maximum_over_every_output_bus() {
    // The delayed card sits only on the path to bus 11; hosts still delay
    // compensate against the single largest number.
    let rack = PreparedRack::prepare(
        &forked_graph(),
        [
            (NodeId(1), slot(1.0, false, false)),
            (NodeId(2), slot(1.0, true, false)),
        ]
        .into(),
        4,
    )
    .unwrap();
    assert_eq!(rack.latency_samples(), 3);
}

#[test]
fn preparation_still_demands_one_input_bus_and_at_least_one_output() {
    let processors = || {
        BTreeMap::from([
            (NodeId(1), slot(1.0, false, false)),
            (NodeId(2), slot(1.0, false, false)),
        ])
    };
    let mut two_inputs = forked_graph();
    two_inputs
        .nodes
        .insert(NodeId(5), NodeKind::Input(Channels::Mono));
    assert!(matches!(
        PreparedRack::prepare(&two_inputs, processors(), 4),
        Err(PrepareError::SingleInputRequired)
    ));
    let no_outputs = RackGraph {
        nodes: [
            (NodeId(0), NodeKind::Input(Channels::Mono)),
            (
                NodeId(1),
                NodeKind::Plugin {
                    inputs: Channels::Mono,
                    outputs: Channels::Mono,
                },
            ),
        ]
        .into(),
        connections: [(0, 1)]
            .map(|(a, b)| Connection {
                from: NodeId(a),
                to: NodeId(b),
            })
            .into(),
        ..RackGraph::default()
    };
    assert!(matches!(
        PreparedRack::prepare(
            &no_outputs,
            BTreeMap::from([(NodeId(1), slot(1.0, false, false))]),
            4
        ),
        Err(PrepareError::NoOutputs)
    ));
}
