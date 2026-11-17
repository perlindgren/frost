//! Prepared, single-thread graph execution. Preparation owns all allocation;
//! processing borrows external buffers and never changes graph topology.
use crate::{Channels, GraphError, NodeId, NodeKind, RackGraph};
use nice_common::true_peak::TruePeak;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

#[cfg(test)]
mod cost_bench;
#[cfg(test)]
mod tests;

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

/// One published wire observation. The key is the metered node id folded to
/// a wire side (`id << 1 | side`); zero marks a free slot because plugin
/// card ids start at one and auxiliary ids near `u64::MAX` are never metered.
struct WireSlot {
    key: AtomicU64,
    rms: AtomicU32,
    /// True peak of the wire (dBTP) in f32 bits. A linear amplitude, so it can
    /// exceed `1.0`: the inter-sample crest of a full-scale step is above full
    /// scale by construction. See [`nice_common::true_peak`].
    peak: AtomicU32,
    /// Applied UI trim as a linear multiplier in f32 bits, `1.0` for unity.
    /// The UI thread stores it at any time and the audio thread reads it once
    /// per block: a block carries either the old trim or the new one, which
    /// is exactly what a live slider needs, and a rebuild keeps the trim
    /// because the slot keeps its key (`reset` retires only dead keys).
    gain: AtomicU32,
    /// Whether the wire is an open circuit. A closed wire — `false`, the
    /// state every wire starts in — carries its signal; an open one is a
    /// switch thrown on the cable, and nothing passes it. The UI thread
    /// stores it through [`WireLevels::set_open`] at any time and the audio
    /// thread reads it once per block, exactly like `gain`, so a block is
    /// either broken whole or whole. A rebuild keeps the state with the key
    /// for the same reason the trim does; a wire that arrives under a key the
    /// table had retired comes back closed, because the plan, not the switch,
    /// decides what a new wire is.
    open: AtomicBool,
}

/// Lock-free wire observations shared between the audio thread and the UI.
/// A control thread registers wires through [`PreparedRack::attach_wire_levels`]
/// before the plan crosses to the audio thread; from then on the audio thread
/// only stores f32 bits, the UI samples by key and pushes trims through
/// [`WireLevels::set_gain`] and switches through [`WireLevels::set_open`], so
/// metering, per-wire gain and per-wire breaks cost no locks and no
/// allocations in `process`.
pub struct WireLevels {
    slots: Vec<WireSlot>,
}

impl Default for WireLevels {
    fn default() -> Self {
        Self::new(0)
    }
}

impl WireLevels {
    /// A table with room for `capacity` wires. Allocation happens only here,
    /// keeping `attach`, `publish`, `sample` and the gain path allocation-free.
    pub fn new(capacity: usize) -> Self {
        Self {
            slots: (0..capacity)
                .map(|_| WireSlot {
                    key: AtomicU64::new(0),
                    rms: AtomicU32::new(0),
                    peak: AtomicU32::new(0),
                    gain: AtomicU32::new(1.0f32.to_bits()),
                    open: AtomicBool::new(false),
                })
                .collect(),
        }
    }

    /// Retire slots whose key the rebuilt plan no longer registers, and calm
    /// the levels of the wires that stay. Control thread only: a rebuild
    /// always joins the previous audio worker before the new plan is attached,
    /// so no reader races these stores. A key that stays keeps its slot — and
    /// with it the trim the UI applied and the switch it threw — so rebuilding
    /// the rack to add or remove one card never silently resets the gains of
    /// the others or closes a wire the operator left broken.
    fn reset(&self, keeping: &[u64]) {
        for slot in &self.slots {
            let key = slot.key.load(Ordering::Relaxed);
            if key != 0 && !keeping.contains(&key) {
                slot.key.store(0, Ordering::Relaxed);
                slot.gain.store(1.0f32.to_bits(), Ordering::Relaxed);
                slot.open.store(false, Ordering::Relaxed);
            }
            slot.rms.store(0, Ordering::Relaxed);
            slot.peak.store(0, Ordering::Relaxed);
        }
    }

    /// Reserve a slot for `key`. Control thread only (see `reset`). A key the
    /// table already holds keeps its slot, its applied trim and its switch; a
    /// new key takes a free slot at unity gain and closed. A zero key or a
    /// full table declines the wire; that meter then stays unobserved instead
    /// of displacing an already published wire.
    fn attach(&self, key: u64) -> Option<usize> {
        if key == 0 {
            return None;
        }
        let held = |slot: &WireSlot| slot.key.load(Ordering::Relaxed);
        let index = self
            .slots
            .iter()
            .position(|slot| held(slot) == key)
            .or_else(|| self.slots.iter().position(|slot| held(slot) == 0))?;
        let slot = &self.slots[index];
        if held(slot) != key {
            slot.gain.store(1.0f32.to_bits(), Ordering::Relaxed);
            slot.open.store(false, Ordering::Relaxed);
        }
        slot.rms.store(0, Ordering::Relaxed);
        slot.peak.store(0, Ordering::Relaxed);
        slot.key.store(key, Ordering::Release);
        Some(index)
    }

    fn publish(&self, index: usize, rms: f32, peak: f32) {
        let Some(slot) = self.slots.get(index) else {
            return;
        };
        slot.rms.store(rms.to_bits(), Ordering::Relaxed);
        slot.peak.store(peak.to_bits(), Ordering::Relaxed);
    }

    /// Apply a linear `gain` to the wire `key`, from the UI thread, at any
    /// time; the bound keeps a runaway slider from stacking up grossly
    /// overdriven plugins. Non-finite values and unregistered keys are
    /// declined: the plan owns registration, so a UI that set a trim while
    /// audio was stopped re-states it once the wires are attached again.
    pub fn set_gain(&self, key: u64, gain: f32) {
        if key == 0 || !gain.is_finite() {
            return;
        }
        if let Some(slot) = self
            .slots
            .iter()
            .find(|slot| slot.key.load(Ordering::Acquire) == key)
        {
            slot.gain
                .store(gain.clamp(0.0, 16.0).to_bits(), Ordering::Relaxed);
        }
    }

    /// Throw or close the switch on the wire `key`, from the UI thread, at
    /// any time. An open wire carries no signal past its stub, which is the
    /// whole of what this does to the audio; the wire is still measured up to
    /// the switch (see [`PreparedRack::process`]), so the meter on a broken
    /// wire says what is arriving at the gap rather than that the gap is
    /// silent. Unregistered keys are declined for the reason [`WireLevels::set_gain`]
    /// gives: a UI that broke a wire while audio was stopped re-states the
    /// break once the wires are attached again.
    pub fn set_open(&self, key: u64, open: bool) {
        if key == 0 {
            return;
        }
        if let Some(slot) = self
            .slots
            .iter()
            .find(|slot| slot.key.load(Ordering::Acquire) == key)
        {
            slot.open.store(open, Ordering::Relaxed);
        }
    }

    /// Whether `key` is an open circuit, or `false` while the key is not
    /// registered in this table. A wire nobody registered carries nothing,
    /// which is not the same fact as a switch thrown on a wire that exists,
    /// and the UI only needs to know about the latter: an unmeasured wire is
    /// drawn as plain cable.
    pub fn open_of(&self, key: u64) -> bool {
        if key == 0 {
            return false;
        }
        self.slots
            .iter()
            .find(|slot| slot.key.load(Ordering::Acquire) == key)
            .is_some_and(|slot| slot.open.load(Ordering::Relaxed))
    }

    /// The trim applied to a registered wire slot, for the audio thread.
    fn gain(&self, index: usize) -> f32 {
        self.slots.get(index).map_or(1.0, |slot| {
            f32::from_bits(slot.gain.load(Ordering::Relaxed))
        })
    }

    /// Whether a registered wire slot is an open circuit, for the audio
    /// thread. An out-of-range index is closed: the plan owns the indices it
    /// handed out, and a wire that has no slot is not one the operator can
    /// break.
    fn open_at(&self, index: usize) -> bool {
        self.slots
            .get(index)
            .is_some_and(|slot| slot.open.load(Ordering::Relaxed))
    }

    /// The trim applied to `key`, or `None` while the key is not registered
    /// in this table. Lets the UI adopt the live trim when it opens a meter.
    pub fn gain_of(&self, key: u64) -> Option<f32> {
        if key == 0 {
            return None;
        }
        self.slots
            .iter()
            .find(|slot| slot.key.load(Ordering::Acquire) == key)
            .map(|slot| f32::from_bits(slot.gain.load(Ordering::Relaxed)))
    }

    /// The latest `(rms, true peak)` published for `key`, both linear, or
    /// `None` while the key is not registered in this table. The peak is the
    /// reconstructed crest (dBTP), not the largest sample, and a faithful
    /// reading of a signal at full scale can sit above `1.0`.
    pub fn sample(&self, key: u64) -> Option<(f32, f32)> {
        if key == 0 {
            return None;
        }
        self.slots
            .iter()
            .find(|slot| slot.key.load(Ordering::Acquire) == key)
            .map(|slot| {
                (
                    f32::from_bits(slot.rms.load(Ordering::Relaxed)),
                    f32::from_bits(slot.peak.load(Ordering::Relaxed)),
                )
            })
    }
}

/// Loudness observation of one wire for a block: a stereo-coerced RMS that
/// keeps the louder of the two channels per frame, and the wire's **true
/// peak** (dBTP) — the crest of the waveform the DAC would rebuild, not the
/// largest sample in it. A 48 kHz sine at 12 kHz whose samples all sit at
/// 1/√2 reads 0.707 as a sample peak and 0.979 as a true peak.
///
/// `meters` is one estimator per channel and must outlive the call: the
/// history they carry across blocks is what lets a *held* level read its own
/// value instead of the +0.8 dB overshoot of an onset. Passing a fresh
/// estimator per block would make every constant read high.
///
/// `frames` is nonzero whenever `process` calls this.
fn wire_level(buffer: &AudioBuffer, frames: usize, meters: &mut [TruePeak]) -> (f32, f32) {
    let mut energy = 0f64;
    if buffer.channels() >= 2 {
        for (left, right) in buffer.0[0].iter().zip(&buffer.0[1]).take(frames) {
            energy += (left * left).max(right * right) as f64;
        }
    } else {
        for sample in buffer.0[0].iter().take(frames) {
            energy += (sample * sample) as f64;
        }
    }
    let mut peak = 0f32;
    // A wire carries mono or stereo, so the two meters the node owns always
    // cover it; the spare only keeps a hypothetically wider buffer from
    // metering silently low instead of at all.
    let mut spare = TruePeak::new();
    for (index, channel) in buffer.0.iter().enumerate() {
        let meter = meters.get_mut(index).unwrap_or(&mut spare);
        meter.process(&channel[..frames]);
        peak = peak.max(meter.take());
    }
    ((energy / frames as f64).sqrt() as f32, peak)
}

/// Scale a block in place by a linear gain. Unity — the state of every
/// untouched wire — is free, and scaling never allocates, so per-wire trims
/// keep the audio-path contract of `process` intact.
fn apply_gain(buffer: &mut AudioBuffer, frames: usize, gain: f32) {
    if gain == 1.0 {
        return;
    }
    for channel in &mut buffer.0 {
        for sample in channel[..frames].iter_mut() {
            *sample *= gain;
        }
    }
}

struct Node {
    kind: NodeKind,
    input: AudioBuffer,
    output: AudioBuffer,
    edges: Vec<Edge>,
    processor: Option<ProcessorSlot>,
    bypass: Delay,
    id: NodeId,
    /// Wire meter slots inside `PreparedRack::wires`, reserved by
    /// `attach_wire_levels`; `None` until then and for every non-plugin node.
    wire_in: Option<usize>,
    wire_out: Option<usize>,
    /// True-peak history of each metered side, one estimator per channel.
    /// Owned here rather than built inside [`wire_level`] so a wire's crest
    /// survives block boundaries and a held level reads exactly itself.
    tp_in: [TruePeak; 2],
    tp_out: [TruePeak; 2],
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
    /// Wire observations shared with the UI. The table is empty until a
    /// control thread attaches one, so an unobserved plan pays nothing.
    wires: Arc<WireLevels>,
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
                wire_in: None,
                wire_out: None,
                tp_in: [TruePeak::new(), TruePeak::new()],
                tp_out: [TruePeak::new(), TruePeak::new()],
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
            wires: Arc::new(WireLevels::default()),
        })
    }
    /// Register every wire the UI should watch, then share `levels` with the
    /// plan. Call on the control thread before the plan crosses to the audio
    /// thread: slots whose keys this plan no longer registers are retired and
    /// all levels are calmed here, which is safe because a rebuild always
    /// joins the previous audio worker first. Keys that survive keep their
    /// slot and the trim the UI had applied to it. Each plugin folds to two
    /// keys (`id << 1 | side`): the input side observes the node's summed
    /// input buffer, the output side its processed output.
    pub fn attach_wire_levels(&mut self, levels: Arc<WireLevels>) {
        // Control-thread allocation is fine here; `process` stays
        // allocation-free.
        let keys: Vec<u64> = self
            .nodes
            .iter()
            .filter(|node| {
                matches!(node.kind, NodeKind::Plugin { .. }) && node.id.0 <= u64::MAX >> 1
            })
            .flat_map(|node| [node.id.0 << 1, (node.id.0 << 1) | 1])
            .collect();
        levels.reset(&keys);
        self.reset_metering();
        for node in &mut self.nodes {
            // Card ids are validated small by the rack, so `id << 1` stays
            // injective; the guard only refuses auxiliary ids near `u64::MAX`.
            let metered =
                matches!(node.kind, NodeKind::Plugin { .. }) && node.id.0 <= u64::MAX >> 1;
            if metered {
                node.wire_in = levels.attach(node.id.0 << 1);
                node.wire_out = levels.attach((node.id.0 << 1) | 1);
            } else {
                node.wire_in = None;
                node.wire_out = None;
            }
        }
        self.wires = levels;
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
        let wires = &self.wires;
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
                    // A wire trim sits on the wire itself: the input trim
                    // scales the summed buffer before the processor sees it,
                    // so the plugin and everything downstream share what the
                    // wire actually carries. The two slots are copied out
                    // first because measuring a wire borrows two of the node's
                    // fields at once.
                    let (wire_in, wire_out) = (node.wire_in, node.wire_out);
                    if let Some(trim) = wire_in {
                        apply_gain(&mut node.input, frames, wires.gain(trim));
                    }
                    // An open circuit is measured before it is broken. The
                    // stub on the switch then keeps reading the signal that
                    // arrives at the gap, which is what tells an operator
                    // chasing a fault that the far side of the break is alive;
                    // what goes on past the gap is silence. Clearing the input
                    // here also feeds that silence to a bypassed card, whose
                    // copy would otherwise walk the broken wire straight
                    // around the plugin.
                    let arrival = wire_in
                        .is_some()
                        .then(|| wire_level(&node.input, frames, &mut node.tp_in));
                    if wire_in.is_some_and(|slot| wires.open_at(slot)) {
                        node.input.clear(frames);
                    }
                    let processor = node.processor.as_mut().unwrap();
                    if processor.bypassed {
                        node.bypass.add(&node.input, &mut node.output, frames);
                    } else {
                        processor
                            .processor
                            .process(&node.input, &mut node.output, frames)
                            .map_err(|_| ProcessError::NodeFailed(node.id))?;
                    }
                    if let Some(trim) = wire_out {
                        apply_gain(&mut node.output, frames, wires.gain(trim));
                    }
                    let departure = wire_out
                        .is_some()
                        .then(|| wire_level(&node.output, frames, &mut node.tp_out));
                    // A card's output buffer is what every edge out of it
                    // reads, so breaking there is what makes the whole rest of
                    // the chain go quiet and not just the card next door.
                    if wire_out.is_some_and(|slot| wires.open_at(slot)) {
                        node.output.clear(frames);
                    }
                    // Meter both stubs of this card from the settled buffers,
                    // after the trims, so the meter and the white gain line
                    // describe the same signal. Keys are only registered by
                    // `attach_wire_levels`, so an unobserved plan takes two
                    // `None` loads per node.
                    if let (Some(slot), Some((rms, peak))) = (wire_in, arrival) {
                        wires.publish(slot, rms, peak);
                    }
                    if let (Some(slot), Some((rms, peak))) = (wire_out, departure) {
                        wires.publish(slot, rms, peak);
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

    /// Drop every stored history on the processing thread: buffers, delays,
    /// processors and the true-peak estimators, which otherwise would still
    /// carry the level from before the reset into the block after it.
    pub fn reset(&mut self) {
        self.reset_metering();
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

    /// Clear the metering histories. A true-peak estimator is a filter with
    /// memory, so `reset` is the one moment its held level is known stale —
    /// after it, a signal arriving is an onset and reads as one.
    fn reset_metering(&mut self) {
        for node in &mut self.nodes {
            for meter in node.tp_in.iter_mut().chain(node.tp_out.iter_mut()) {
                meter.reset();
            }
        }
    }
}
