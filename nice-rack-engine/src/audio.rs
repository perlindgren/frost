//! Prepared, single-thread graph execution. Preparation owns all allocation;
//! processing borrows external buffers and never changes graph topology.
use crate::{Channels, GraphError, NodeId, NodeKind, RackGraph};
use std::collections::{BTreeMap, BTreeSet};

/// Plan-owned planar sample storage. Only the requested frame prefix is valid
/// during a process call. Processors must write every output sample in that prefix.
pub struct AudioBuffer(Vec<Vec<f32>>);
impl AudioBuffer {
    fn new(channels: usize, frames: usize) -> Self {
        Self(vec![vec![0.0; frames]; channels])
    }
    pub fn channel(&self, index: usize, frames: usize) -> &[f32] {
        &self.0[index][..frames]
    }
    pub fn channel_mut(&mut self, index: usize, frames: usize) -> &mut [f32] {
        &mut self.0[index][..frames]
    }
    pub fn channel_slices_mut(&mut self, frames: usize) -> impl Iterator<Item = &mut [f32]> {
        self.0.iter_mut().map(move |channel| &mut channel[..frames])
    }

    pub fn channels(&self) -> usize {
        self.0.len()
    }
    fn clear(&mut self, frames: usize) {
        for channel in &mut self.0 {
            channel[..frames].fill(0.0);
        }
    }
}

/// An already prepared processor. Implementations must not allocate, block or
/// perform file/lifecycle work in process/reset. Layout and latency are fixed
/// until a new plan is prepared. Native plugin activation is a separate step.
pub trait Processor: Send {
    fn latency_samples(&self) -> usize {
        0
    }
    fn process(
        &mut self,
        input: &AudioBuffer,
        output: &mut AudioBuffer,
        frames: usize,
    ) -> Result<(), ProcessError>;
    fn reset(&mut self) {}
    /// Stop native processing on the audio thread before returning handles.
    fn stop(&mut self) {}
}
pub struct ProcessorSlot {
    pub processor: Box<dyn Processor>,
    /// Fixed for this plan; dry bypass preserves the processor's latency.
    pub bypassed: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessError {
    InvalidBlockSize,
    InvalidBuffers,
    ProcessorFailed,
    NodeFailed(NodeId),
}
#[derive(Debug, PartialEq, Eq)]
pub enum PrepareError {
    Graph(GraphError),
    InvalidBlockSize,
    SingleInputOutputRequired,
    DisconnectedNode(NodeId),
    MissingProcessor(NodeId),
    UnexpectedProcessor(NodeId),
    BypassRequiresMatchingChannels(NodeId),
    BufferBudgetExceeded,
}
impl std::fmt::Display for PrepareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cannot prepare rack audio: {self:?}")
    }
}
impl std::error::Error for PrepareError {}

const SAMPLE_BUDGET: usize = 16 * 1024 * 1024; // 64 MiB of f32 sample storage.
struct Budget(usize);
impl Budget {
    fn reserve(&mut self, channels: usize, frames: usize) -> Result<(), PrepareError> {
        self.0 = self
            .0
            .checked_add(
                channels
                    .checked_mul(frames)
                    .ok_or(PrepareError::BufferBudgetExceeded)?,
            )
            .filter(|&n| n <= SAMPLE_BUDGET)
            .ok_or(PrepareError::BufferBudgetExceeded)?;
        Ok(())
    }
}
struct Delay {
    history: Vec<Vec<f32>>,
    cursor: usize,
}
impl Delay {
    fn new(channels: usize, samples: usize, budget: &mut Budget) -> Result<Self, PrepareError> {
        budget.reserve(channels, samples)?;
        Ok(Self {
            history: vec![vec![0.0; samples]; channels],
            cursor: 0,
        })
    }
    fn add(&mut self, input: &AudioBuffer, output: &mut AudioBuffer, frames: usize) {
        let length = self.history[0].len();
        if length == 0 {
            for (source, destination) in input.0.iter().zip(&mut output.0) {
                for (source, destination) in source[..frames].iter().zip(&mut destination[..frames])
                {
                    *destination += source;
                }
            }
        } else {
            for frame in 0..frames {
                for channel in 0..self.history.len() {
                    output.0[channel][frame] += self.history[channel][self.cursor];
                    self.history[channel][self.cursor] = input.0[channel][frame];
                }
                self.cursor = (self.cursor + 1) % length;
            }
        }
    }
    fn reset(&mut self) {
        self.cursor = 0;
        for channel in &mut self.history {
            channel.fill(0.0);
        }
    }
}
struct Edge {
    source: usize,
    delay: Delay,
}
struct Node {
    kind: NodeKind,
    input: AudioBuffer,
    output: AudioBuffer,
    edges: Vec<Edge>,
    processor: Option<ProcessorSlot>,
    bypass: Delay,
    id: NodeId,
}
/// Owns all buffers and processor handles. Keep destruction and replacement on
/// a control/main thread; block-boundary handover is not implemented yet.
pub struct PreparedRack {
    nodes: Vec<Node>,
    input_channels: usize,
    output_channels: usize,
    output_index: usize,
    max_frames: usize,
    latency: usize,
}
fn count(channels: Channels) -> usize {
    match channels {
        Channels::Mono => 1,
        Channels::Stereo => 2,
    }
}
impl PreparedRack {
    pub fn prepare(
        graph: &RackGraph,
        mut processors: BTreeMap<NodeId, ProcessorSlot>,
        max_frames: usize,
    ) -> Result<Self, PrepareError> {
        if max_frames == 0 || max_frames > 65536 {
            return Err(PrepareError::InvalidBlockSize);
        }
        let routing = graph.compile().map_err(PrepareError::Graph)?;
        let inputs: Vec<_> = graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node, NodeKind::Input(_)))
            .collect();
        let outputs: Vec<_> = graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node, NodeKind::Output(_)))
            .collect();
        if inputs.len() != 1 || outputs.len() != 1 {
            return Err(PrepareError::SingleInputOutputRequired);
        }
        let output_id = *outputs[0].0;
        let mut reachable = BTreeSet::from([output_id]);
        for &id in routing.order.iter().rev() {
            if reachable.contains(&id) {
                for edge in routing.connections.iter().filter(|edge| edge.to == id) {
                    reachable.insert(edge.from);
                }
            }
        }
        if let Some(&id) = routing.order.iter().find(|id| !reachable.contains(id)) {
            return Err(PrepareError::DisconnectedNode(id));
        }
        let mut budget = Budget(0);
        let mut indices = BTreeMap::new();
        let mut latencies = BTreeMap::new();
        let mut nodes = Vec::new();
        for id in routing.order {
            let kind = graph.nodes[&id].clone();
            let incoming: Vec<_> = routing
                .connections
                .iter()
                .filter(|edge| edge.to == id)
                .collect();
            let arrival = incoming
                .iter()
                .map(|edge| latencies[&edge.from])
                .max()
                .unwrap_or(0usize);
            let in_count = count(kind.inputs().or(kind.outputs()).unwrap());
            let out_count = count(kind.outputs().or(kind.inputs()).unwrap());
            let processor = if matches!(kind, NodeKind::Plugin { .. }) {
                Some(
                    processors
                        .remove(&id)
                        .ok_or(PrepareError::MissingProcessor(id))?,
                )
            } else {
                None
            };
            let latency = processor
                .as_ref()
                .map(|slot| slot.processor.latency_samples())
                .unwrap_or(0);
            if processor.as_ref().is_some_and(|slot| slot.bypassed) && in_count != out_count {
                return Err(PrepareError::BypassRequiresMatchingChannels(id));
            }
            let bypass_latency = if processor.as_ref().is_some_and(|slot| slot.bypassed) {
                latency
            } else {
                0
            };
            let bypass = Delay::new(in_count, bypass_latency, &mut budget)?;
            let mut edges = Vec::new();
            for edge in incoming {
                edges.push(Edge {
                    source: indices[&edge.from],
                    delay: Delay::new(in_count, arrival - latencies[&edge.from], &mut budget)?,
                });
            }
            budget.reserve(in_count, max_frames)?;
            budget.reserve(out_count, max_frames)?;
            indices.insert(id, nodes.len());
            latencies.insert(
                id,
                arrival
                    .checked_add(latency)
                    .ok_or(PrepareError::BufferBudgetExceeded)?,
            );
            nodes.push(Node {
                kind,
                input: AudioBuffer::new(in_count, max_frames),
                output: AudioBuffer::new(out_count, max_frames),
                edges,
                processor,
                bypass,
                id,
            });
        }
        if let Some((&id, _)) = processors.first_key_value() {
            return Err(PrepareError::UnexpectedProcessor(id));
        }
        Ok(Self {
            nodes,
            input_channels: count(inputs[0].1.outputs().unwrap()),
            output_channels: count(outputs[0].1.inputs().unwrap()),
            output_index: indices[&output_id],
            max_frames,
            latency: latencies[&output_id],
        })
    }
    pub fn latency_samples(&self) -> usize {
        self.latency
    }
    /// Mixer inputs sum at unity gain. Mono->stereo duplicates; stereo->mono
    /// averages. No implicit clipping or limiting. On an error, external output
    /// is silenced; processor errors require reset/recovery by the owner.
    pub fn process(
        &mut self,
        input: &[&[f32]],
        output: &mut [&mut [f32]],
        frames: usize,
    ) -> Result<(), ProcessError> {
        for channel in output.iter_mut() {
            let n = frames.min(channel.len());
            channel[..n].fill(0.0);
        }
        if frames == 0 || frames > self.max_frames {
            return Err(ProcessError::InvalidBlockSize);
        }
        if input.len() != self.input_channels
            || output.len() != self.output_channels
            || input.iter().any(|c| c.len() < frames)
            || output.iter().any(|c| c.len() < frames)
        {
            return Err(ProcessError::InvalidBuffers);
        }
        for index in 0..self.nodes.len() {
            let (before, after) = self.nodes.split_at_mut(index);
            let node = &mut after[0];
            node.input.clear(frames);
            node.output.clear(frames);
            for edge in &mut node.edges {
                edge.delay
                    .add(&before[edge.source].output, &mut node.input, frames);
            }
            match node.kind {
                NodeKind::Input(_) => {
                    for (source, destination) in input.iter().zip(&mut node.output.0) {
                        destination[..frames].copy_from_slice(&source[..frames]);
                    }
                }
                NodeKind::Plugin { .. } => {
                    let slot = node.processor.as_mut().unwrap();
                    if slot.bypassed {
                        node.bypass.add(&node.input, &mut node.output, frames);
                    } else {
                        slot.processor
                            .process(&node.input, &mut node.output, frames)
                            .map_err(|_| ProcessError::NodeFailed(node.id))?;
                    }
                }
                NodeKind::ChannelMap { .. } if node.input.channels() != node.output.channels() => {
                    if node.output.channels() == 1 {
                        for frame in 0..frames {
                            node.output.0[0][frame] =
                                (node.input.0[0][frame] + node.input.0[1][frame]) * 0.5;
                        }
                    } else {
                        for channel in &mut node.output.0 {
                            channel[..frames].copy_from_slice(&node.input.0[0][..frames]);
                        }
                    }
                }
                _ => {
                    for (source, destination) in node.input.0.iter().zip(&mut node.output.0) {
                        destination[..frames].copy_from_slice(&source[..frames]);
                    }
                }
            }
        }
        for (source, destination) in self.nodes[self.output_index].output.0.iter().zip(output) {
            destination[..frames].copy_from_slice(&source[..frames]);
        }
        Ok(())
    }
    /// Call on the processing thread before handing the plan back for destruction.
    pub fn stop(&mut self) {
        for node in &mut self.nodes {
            if let Some(slot) = &mut node.processor {
                slot.processor.stop();
            }
        }
    }

    pub fn reset(&mut self) {
        for node in &mut self.nodes {
            node.input.clear(self.max_frames);
            node.output.clear(self.max_frames);
            for edge in &mut node.edges {
                edge.delay.reset();
            }
            node.bypass.reset();
            if let Some(slot) = &mut node.processor {
                slot.processor.reset();
            }
        }
    }
}
