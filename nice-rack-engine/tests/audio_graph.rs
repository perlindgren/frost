#![deny(unsafe_code)]
use nice_rack_engine::{audio::*, *};
use std::collections::BTreeMap;

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
