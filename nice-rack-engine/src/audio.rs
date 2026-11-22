//! Prepared, single-thread graph execution. Preparation owns all allocation;
//! processing borrows external buffers and never changes graph topology.
mod timing;
use std::time::Instant;
pub use timing::{ProcessingTime, ProcessingTimes};

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
    SingleInputRequired,
    NoOutputs,
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
    /// Keep the untrimmed dry history warm while the effect is active.
    fn advance(&mut self, input: &AudioBuffer, frames: usize) {
        let length = self.history[0].len();
        if length == 0 {
            return;
        }
        for (history, source) in self.history.iter_mut().zip(&input.0) {
            let mut cursor = self.cursor;
            for &sample in &source[..frames] {
                history[cursor] = sample;
                cursor = (cursor + 1) % length;
            }
        }
        self.cursor = (self.cursor + frames) % length;
    }
    /// Latency-compensated unity bypass, with the graph's own channel-count
    /// conversion when a processor has different input and output widths:
    /// equal widths copy, a join folds every input to their mean, a split
    /// hands the first channel to every output — the same contract as
    /// [`crate::NodeKind::ChannelMap`].
    fn dry(&mut self, input: &AudioBuffer, output: &mut AudioBuffer, frames: usize) {
        let length = self.history[0].len();
        let inputs = input.channels();
        let outputs = output.channels();
        for frame in 0..frames {
            let mut samples = [0.0f32; Channels::MAX as usize];
            for (channel, sample) in samples.iter_mut().enumerate().take(inputs) {
                *sample = if length == 0 {
                    input.0[channel][frame]
                } else {
                    let old = self.history[channel][self.cursor];
                    self.history[channel][self.cursor] = input.0[channel][frame];
                    old
                };
            }
            if length != 0 {
                self.cursor = (self.cursor + 1) % length;
            }
            // The join is a mean over every input; the stereo case keeps its
            // exact (L + R) * 0.5 form, which a halved sum reproduces bit for
            // bit.
            let joined: f32 = samples[..inputs].iter().sum::<f32>() / inputs as f32;
            for channel in 0..outputs {
                output.0[channel][frame] = if inputs == outputs {
                    samples[channel]
                } else if outputs < inputs {
                    joined
                } else {
                    samples[0]
                };
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
    /// Left-lane RMS in f32 bits — for a mono wire, the whole of it.
    rms: AtomicU32,
    /// Right-lane RMS in f32 bits; silence on a mono wire. Channels past the
    /// second fold into this lane rather than meter unobserved.
    rms_r: AtomicU32,
    /// Left-lane true peak of the wire (dBTP) in f32 bits. A linear amplitude,
    /// so it can exceed `1.0`: the inter-sample crest of a full-scale step is
    /// above full scale by construction. See [`nice_common::true_peak`].
    peak: AtomicU32,
    /// Right-lane true peak; silence on a mono wire, folded max past two.
    peak_r: AtomicU32,
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
    /// Card bypass is held on its input slot and read once per block.
    bypassed: AtomicBool,
    paused: AtomicBool,
}

/// The wire key of the rack's master output trim.
///
/// The rack's primary `Output` node is addressed `NodeId(u64::MAX)`, and the
/// output gain stage sits on the wire entering its jack, so the node's own
/// address is the key the UI pushes the master trim to and samples the
/// summed-signal meter from. [`card_wire_key`] refuses the card ids that
/// could otherwise shift into it.
pub const MASTER_WIRE_KEY: u64 = u64::MAX;

/// The wire key of one card side — `id << 1 | side`, the input side at the
/// even key — or `None` when the id sits too near [`MASTER_WIRE_KEY`] for
/// that mapping to stay clear of it: id `u64::MAX >> 1` on the output side
/// would land exactly on the master key, and larger ids shift outright.
pub fn card_wire_key(id: u64, side: bool) -> Option<u64> {
    if id < u64::MAX >> 1 {
        Some((id << 1) | u64::from(side))
    } else {
        None
    }
}

/// The wire key of the rack's input jack.
///
/// The plan's single `NodeKind::Input` node is the capture side of the rack,
/// and the rack addresses it `NodeId(0)`, so its output side lands on key 1 of
/// [`card_wire_key`] — a key no card reaches, since card ids start at 1. The
/// node's *input* side gets no slot: the device stream before the rack has
/// touched it is not this wire's story. The slot carries the rack's input
/// trim, and its meter reads the signal as it enters the rack, after that
/// trim, so an operator dialing capture gain trims to what the meter shows.
/// Its outgoing switch cuts the captured feed after metering, silencing every
/// downstream branch while keeping the source visible.
pub const INPUT_WIRE_KEY: u64 = 1;

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
                    rms_r: AtomicU32::new(0),
                    peak: AtomicU32::new(0),
                    peak_r: AtomicU32::new(0),
                    gain: AtomicU32::new(1.0f32.to_bits()),
                    open: AtomicBool::new(false),
                    bypassed: AtomicBool::new(false),
                    paused: AtomicBool::new(false),
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
                slot.bypassed.store(false, Ordering::Relaxed);
                slot.paused.store(false, Ordering::Relaxed);
            }
            slot.rms.store(0, Ordering::Relaxed);
            slot.rms_r.store(0, Ordering::Relaxed);
            slot.peak.store(0, Ordering::Relaxed);
            slot.peak_r.store(0, Ordering::Relaxed);
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
            slot.bypassed.store(false, Ordering::Relaxed);
            slot.paused.store(false, Ordering::Relaxed);
        }
        slot.rms.store(0, Ordering::Relaxed);
        slot.rms_r.store(0, Ordering::Relaxed);
        slot.peak.store(0, Ordering::Relaxed);
        slot.peak_r.store(0, Ordering::Relaxed);
        slot.key.store(key, Ordering::Release);
        Some(index)
    }

    fn publish(&self, index: usize, level: WireSample) {
        let Some(slot) = self.slots.get(index) else {
            return;
        };
        slot.rms.store(level.rms[0].to_bits(), Ordering::Relaxed);
        slot.rms_r.store(level.rms[1].to_bits(), Ordering::Relaxed);
        slot.peak.store(level.peak[0].to_bits(), Ordering::Relaxed);
        slot.peak_r
            .store(level.peak[1].to_bits(), Ordering::Relaxed);
    }

    /// Toggle the whole card's latency-preserving unity bypass, from the UI.
    /// Trims and cable breaks stay saved but do not affect bypassed audio.
    pub fn set_bypassed(&self, id: NodeId, bypassed: bool) {
        if id.0 == 0 || id.0 > u64::MAX >> 1 {
            return;
        }
        if let Some(slot) = self
            .slots
            .iter()
            .find(|slot| slot.key.load(Ordering::Acquire) == id.0 << 1)
        {
            slot.bypassed.store(bypassed, Ordering::Relaxed);
        }
    }
    /// Pause only the plugin branch. A separately enabled dry bypass wins.
    pub fn set_paused(&self, id: NodeId, paused: bool) {
        if id.0 == 0 || id.0 > u64::MAX >> 1 {
            return;
        }
        if let Some(slot) = self
            .slots
            .iter()
            .find(|slot| slot.key.load(Ordering::Acquire) == id.0 << 1)
        {
            slot.paused.store(paused, Ordering::Relaxed);
        }
    }
    fn paused_at(&self, index: usize) -> bool {
        self.slots
            .get(index)
            .is_some_and(|slot| slot.paused.load(Ordering::Relaxed))
    }
    fn bypassed_at(&self, index: usize) -> bool {
        self.slots
            .get(index)
            .is_some_and(|slot| slot.bypassed.load(Ordering::Relaxed))
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
    /// whole of what this does to the audio. Input meters read after the
    /// switch, so an open input reads silence; output meters read before
    /// their outgoing switch and still show the source's signal.
    /// Unregistered keys are declined for the reason [`WireLevels::set_gain`]
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

    /// The latest `(rms, true peak)` published for `key` — both linear, the
    /// louder of the two lanes each, or `None` while the key is not
    /// registered in this table. The peak is the reconstructed crest (dBTP),
    /// not the largest sample, and a faithful reading of a signal at full
    /// scale can sit above `1.0`. [`WireLevels::sample_lanes`] reads the two
    /// lanes separately for stereo metering.
    pub fn sample(&self, key: u64) -> Option<(f32, f32)> {
        self.sample_lanes(key).map(|level| {
            (
                level.rms[0].max(level.rms[1]),
                level.peak[0].max(level.peak[1]),
            )
        })
    }

    /// The latest block's level per lane for `key`, or `None` while the key
    /// is not registered in this table. Lane 0 is the left — for a mono wire,
    /// the whole of it — and lane 1 the right, silent on a mono wire.
    pub fn sample_lanes(&self, key: u64) -> Option<WireSample> {
        if key == 0 {
            return None;
        }
        self.slots
            .iter()
            .find(|slot| slot.key.load(Ordering::Acquire) == key)
            .map(|slot| WireSample {
                rms: [
                    f32::from_bits(slot.rms.load(Ordering::Relaxed)),
                    f32::from_bits(slot.rms_r.load(Ordering::Relaxed)),
                ],
                peak: [
                    f32::from_bits(slot.peak.load(Ordering::Relaxed)),
                    f32::from_bits(slot.peak_r.load(Ordering::Relaxed)),
                ],
            })
    }
}

/// One block's metered level on a wire, per lane: `[0]` is the left — for a
/// mono wire, the whole of it — and `[1]` the right, silent on a mono wire.
/// Both members are linear amplitudes; the peaks are true peaks (dBTP).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WireSample {
    pub rms: [f32; 2],
    pub peak: [f32; 2],
}

/// Loudness observation of one wire for a block, per lane: lane 0 carries
/// the first channel — the whole wire when it is mono — and lane 1 the
/// second, into which any further channels fold per frame by the louder of
/// them, so a wider bus still meters instead of falling silent. A mono wire
/// leaves lane 1 at zero. The peaks are **true peaks** (dBTP) — the crest of
/// the waveform the DAC would rebuild, not the largest sample in it. A
/// 48 kHz sine at 12 kHz whose samples all sit at 1/√2 reads 0.707 as a
/// sample peak and 0.979 as a true peak.
///
/// `meters` is one estimator per lane and must outlive the call: the
/// history they carry across blocks is what lets a *held* level read its own
/// value instead of the +0.8 dB overshoot of an onset. Passing a fresh
/// estimator per block would make every constant read high.
///
/// `frames` is nonzero whenever `process` calls this.
fn wire_level(buffer: &AudioBuffer, frames: usize, meters: &mut [TruePeak]) -> WireSample {
    // The first channel is lane 0 on its own; every channel after it folds
    // into lane 1 per frame, keeping the louder, before its energy counts.
    let mut energy = [0f64; 2];
    for (frame, sample) in buffer.0[0][..frames].iter().enumerate() {
        energy[0] += (sample * sample) as f64;
        let mut fold = 0.0f32;
        for channel in buffer.0[1..].iter() {
            fold = fold.max(channel[frame].abs());
        }
        energy[1] += (fold * fold) as f64;
    }
    // A wire carries at most the two lanes the node's two meters cover; the
    // spare only keeps a hypothetically wider buffer from metering silently
    // low instead of at all, folding into lane 1.
    let mut spare = TruePeak::new();
    let mut level = WireSample::default();
    for (index, channel) in buffer.0.iter().enumerate() {
        let lane = usize::from(index > 0);
        let meter = meters.get_mut(index).unwrap_or(&mut spare);
        meter.process(&channel[..frames]);
        level.peak[lane] = level.peak[lane].max(meter.take());
    }
    for lane in 0..2 {
        level.rms[lane] = (energy[lane] / frames as f64).sqrt() as f32;
    }
    level
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
    /// `attach_wire_levels`; `None` until then and for every node that is
    /// neither a plugin nor the primary `Output` (which carries the master
    /// trim and meter on `wire_in` alone).
    wire_in: Option<usize>,
    wire_out: Option<usize>,
    timing_slot: Option<usize>,
    /// True-peak history of each metered side, one estimator per channel.
    /// Owned here rather than built inside [`wire_level`] so a wire's crest
    /// survives block boundaries and a held level reads exactly itself.
    tp_in: [TruePeak; 2],
    tp_out: [TruePeak; 2],
}
/// One output bus of a prepared plan: which `Output` node drives it, where
/// its node lives in `nodes`, and how many channels it carries. External
/// buffer groups are ordered by bus position, which follows ascending
/// `NodeId` so callers can bind ports to buses deterministically.
struct OutputBus {
    id: NodeId,
    index: usize,
    channels: usize,
}
/// Owns all buffers and processor handles. Keep destruction and replacement on
/// a control/main thread; block-boundary handover is not implemented yet.
pub struct PreparedRack {
    nodes: Vec<Node>,
    input_channels: usize,
    output_buses: Vec<OutputBus>,
    max_frames: usize,
    /// The largest latency over every output bus: the single number hosts
    /// delay-compensate against, which stays honest when lanes differ.
    latency: usize,
    /// Wire observations shared with the UI. The table is empty until a
    /// control thread attaches one, so an unobserved plan pays nothing.
    wires: Arc<WireLevels>,
    timings: Option<(Arc<ProcessingTimes>, f64)>,
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
        if inputs.len() != 1 {
            return Err(PrepareError::SingleInputRequired);
        }
        if outputs.is_empty() {
            return Err(PrepareError::NoOutputs);
        }
        // A node counts as connected if any output bus can still hear it, so
        // a lane that ends at its own output instead of rejoining the trunk
        // stays alive.
        let mut reachable: BTreeSet<_> = outputs.iter().map(|&(&id, _)| id).collect();
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
            let in_count = kind.inputs().or(kind.outputs()).unwrap().count();
            let out_count = kind.outputs().or(kind.inputs()).unwrap().count();
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
            // Allocate once even for active plugins: a live bypass must not
            // allocate or rebuild the graph when its switch changes.
            let bypass = Delay::new(in_count, latency, &mut budget)?;
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
                timing_slot: None,
                tp_in: [TruePeak::new(), TruePeak::new()],
                tp_out: [TruePeak::new(), TruePeak::new()],
            });
        }
        if let Some((&id, _)) = processors.first_key_value() {
            return Err(PrepareError::UnexpectedProcessor(id));
        }
        // `graph.nodes` iterates by ascending `NodeId`, so `outputs` is
        // already in the bus order promised by `output_buses`.
        let output_buses: Vec<_> = outputs
            .iter()
            .map(|&(&id, kind)| OutputBus {
                id,
                index: indices[&id],
                channels: kind.inputs().unwrap().count(),
            })
            .collect();
        Ok(Self {
            nodes,
            input_channels: inputs[0].1.outputs().unwrap().count(),
            output_buses,
            max_frames,
            latency: outputs
                .iter()
                .map(|&(&id, _)| latencies[&id])
                .max()
                .unwrap_or(0),
            wires: Arc::new(WireLevels::default()),
            timings: None,
        })
    }
    /// The output buses of this plan in external buffer-group order: each
    /// bus's `Output` node id and channel count, ascending by id.
    pub fn output_buses(&self) -> impl Iterator<Item = (NodeId, usize)> + '_ {
        self.output_buses.iter().map(|bus| (bus.id, bus.channels))
    }
    /// Where the primary `Output` node — the one addressed
    /// [`MASTER_WIRE_KEY`] — lives in `nodes`, if this plan has one. Its
    /// incoming wire carries the master output trim and the summed-signal
    /// meter; auxiliary lane outputs pass through untouched.
    fn master_output_index(&self) -> Option<usize> {
        self.output_buses
            .iter()
            .find(|bus| bus.id.0 == MASTER_WIRE_KEY)
            .map(|bus| bus.index)
    }
    /// Register every wire the UI should watch, then share `levels` with the
    /// plan. Call on the control thread before the plan crosses to the audio
    /// thread: slots whose keys this plan no longer registers are retired and
    /// all levels are calmed here, which is safe because a rebuild always
    /// joins the previous audio worker first. Keys that survive keep their
    /// slot and the trim the UI had applied to it. Each plugin folds to two
    /// keys (`id << 1 | side`): the input side observes the node's summed
    /// input buffer, the output side its processed output. The primary
    /// `Output` node registers one more key, [`MASTER_WIRE_KEY`], whose
    /// `wire_in` carries the rack's master output trim, a break switch, and
    /// a meter reading the summed signal about to leave for the DAC. The
    /// single `Input` node registers one too — [`INPUT_WIRE_KEY`] on its
    /// `wire_out` — carrying the rack's input trim and a meter reading the
    /// capture as it enters the rack.
    pub fn attach_wire_levels(&mut self, levels: Arc<WireLevels>) {
        // Control-thread allocation is fine here; `process` stays
        // allocation-free.
        let mut keys: Vec<u64> = self
            .nodes
            .iter()
            .filter(|node| matches!(node.kind, NodeKind::Plugin { .. }))
            .filter_map(|node| card_wire_key(node.id.0, false))
            .flat_map(|base| [base, base | 1])
            .collect();
        if self.master_output_index().is_some() {
            keys.push(MASTER_WIRE_KEY);
        }
        if let Some(key) = self
            .nodes
            .iter()
            .find(|node| matches!(node.kind, NodeKind::Input(_)))
            .and_then(|node| card_wire_key(node.id.0, true))
        {
            keys.push(key);
        }
        levels.reset(&keys);
        self.reset_metering();
        let master = self.master_output_index();
        for (index, node) in self.nodes.iter_mut().enumerate() {
            // Card ids are validated small by the rack, so `id << 1` stays
            // injective; `card_wire_key` refuses the ids near `u64::MAX` that
            // would shift onto the master key, which belongs to the rack's
            // own output trim on the primary `Output` node.
            node.wire_in = None;
            node.wire_out = None;
            if let Some(base) = matches!(node.kind, NodeKind::Plugin { .. })
                .then(|| card_wire_key(node.id.0, false))
                .flatten()
            {
                node.wire_in = levels.attach(base);
                node.wire_out = levels.attach(base | 1);
            } else if master == Some(index) {
                node.wire_in = levels.attach(MASTER_WIRE_KEY);
            } else if matches!(node.kind, NodeKind::Input(_)) {
                // The capture jack's trim and meter sit on the node's
                // output side — the signal as it enters the rack — and the
                // input side stays unmetred: what the device sent before
                // the rack trimmed it is not this wire's story.
                node.wire_out = card_wire_key(node.id.0, true).and_then(|key| levels.attach(key));
            }
        }
        self.wires = levels;
    }
    /// Attach timing observations before transferring this plan to its worker.
    /// The deadline is recomputed for each actual block, including short tails.
    pub fn attach_processing_times(&mut self, times: Arc<ProcessingTimes>, sample_rate: f64) {
        assert!(sample_rate.is_finite() && sample_rate > 0.0);
        let ids: Vec<_> = self
            .nodes
            .iter()
            .filter(|node| node.processor.is_some())
            .map(|node| node.id)
            .collect();
        times.register(&ids);
        let mut index = 0;
        for node in &mut self.nodes {
            node.timing_slot = node.processor.as_ref().map(|_| {
                let slot = index;
                index += 1;
                slot
            });
        }
        self.timings = Some((times, sample_rate));
    }
    pub fn latency_samples(&self) -> usize {
        self.latency
    }
    /// Mixer inputs sum at unity gain. Mono->stereo duplicates; stereo->mono
    /// averages. No implicit clipping or limiting. On an error, external output
    /// is silenced; processor errors require reset/recovery by the owner.
    /// `outputs` holds one buffer group per output bus, in
    /// [`Self::output_buses`] order, each group the bus's channel buffers.
    pub fn process(
        &mut self,
        input: &[&[f32]],
        outputs: &mut [&mut [&mut [f32]]],
        frames: usize,
    ) -> Result<(), ProcessError> {
        for group in outputs.iter_mut() {
            for channel in group.iter_mut() {
                let n = frames.min(channel.len());
                channel[..n].fill(0.0);
            }
        }
        if frames == 0 || frames > self.max_frames {
            return Err(ProcessError::InvalidBlockSize);
        }
        if input.len() != self.input_channels
            || outputs.len() != self.output_buses.len()
            || input.iter().any(|c| c.len() < frames)
            || outputs.iter().zip(&self.output_buses).any(|(group, bus)| {
                group.len() != bus.channels || group.iter().any(|c| c.len() < frames)
            })
        {
            return Err(ProcessError::InvalidBuffers);
        }
        let wires = &self.wires;
        let started = self.timings.as_ref().map(|_| Instant::now());
        let deadline = self
            .timings
            .as_ref()
            .map_or(1.0, |(_, rate)| frames as f64 / rate);
        let graph_observation = self
            .timings
            .as_ref()
            .zip(started)
            .map(|((times, _), started)| timing::GraphObservation {
                times,
                started,
                budget: deadline,
            });
        for index in 0..self.nodes.len() {
            let (before, after) = self.nodes.split_at_mut(index);
            let node = &mut after[0];
            let node_started = node.timing_slot.map(|_| Instant::now());
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
                    // The capture jack's trim sits on the wire leaving the
                    // node, so every card downstream shares the level the
                    // meter shows — the same shape as the master output
                    // stage, mirrored to the front of the rack. The meter
                    // reads the trimmed signal, because that is the signal
                    // the rack feeds itself, before its outgoing break switch.
                    if let Some(slot) = node.wire_out {
                        apply_gain(&mut node.output, frames, wires.gain(slot));
                        let level = wire_level(&node.output, frames, &mut node.tp_out);
                        wires.publish(slot, level);
                        // Capture is metered at the source, then its outgoing
                        // switch cuts the feed to every downstream branch.
                        if wires.open_at(slot) {
                            node.output.clear(frames);
                        }
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
                    let bypassed = node.processor.as_ref().unwrap().bypassed
                        || wire_in.is_some_and(|slot| wires.bypassed_at(slot));
                    let paused = wire_in.is_some_and(|slot| wires.paused_at(slot));
                    if !bypassed {
                        node.bypass.advance(&node.input, frames);
                    }
                    if let Some(trim) = wire_in.filter(|_| !bypassed && !paused) {
                        apply_gain(&mut node.input, frames, wires.gain(trim));
                    }
                    // Input meters sit on the plugin side of the cable break.
                    // Bypass ignores the break; otherwise both the processor and
                    // its input meter receive silence. Reset reconstruction history
                    // so a previously live wire cannot leak a stale peak past it.
                    let input_broken = !bypassed && wire_in.is_some_and(|slot| wires.open_at(slot));
                    if input_broken {
                        node.input.clear(frames);
                        for meter in &mut node.tp_in {
                            meter.reset();
                        }
                    }
                    let arrival = wire_in
                        .is_some()
                        .then(|| wire_level(&node.input, frames, &mut node.tp_in));
                    let processor = node.processor.as_mut().unwrap();
                    // Bypass changes routing, not time: keep the effect's
                    // history current with actual input. Only Pause suspends DSP.
                    if !paused {
                        processor
                            .processor
                            .process(&node.input, &mut node.output, frames)
                            .map_err(|_| ProcessError::NodeFailed(node.id))?;
                    }
                    if bypassed {
                        // Overwrite every wet sample with latency-matched dry
                        // audio, including when Pause is independently enabled.
                        node.bypass.dry(&node.input, &mut node.output, frames);
                    }
                    if let Some(trim) = wire_out.filter(|_| !bypassed && !paused) {
                        apply_gain(&mut node.output, frames, wires.gain(trim));
                    }
                    let departure = wire_out.is_some().then(|| {
                        if paused && !bypassed {
                            for meter in &mut node.tp_out {
                                meter.reset();
                            }
                            WireSample::default()
                        } else {
                            wire_level(&node.output, frames, &mut node.tp_out)
                        }
                    });
                    // A card's output buffer is what every edge out of it
                    // reads, so breaking there is what makes the whole rest of
                    // the chain go quiet and not just the card next door.
                    if !bypassed && wire_out.is_some_and(|slot| wires.open_at(slot)) {
                        node.output.clear(frames);
                    }
                    // Meter both stubs of this card from the settled buffers,
                    // after the trims, so the meter and the white gain line
                    // describe the same signal. Keys are only registered by
                    // `attach_wire_levels`, so an unobserved plan takes two
                    // `None` loads per node.
                    if let (Some(slot), Some(level)) = (wire_in, arrival) {
                        wires.publish(slot, level);
                    }
                    if let (Some(slot), Some(level)) = (wire_out, departure) {
                        wires.publish(slot, level);
                    }
                }
                NodeKind::Output(_) => {
                    // The rack's master output trim: a gain stage on the
                    // wire entering the jack, so what the meter shows is
                    // what leaves to the DAC — the summed signal after the
                    // unity mixer, scaled. Only the primary output carries
                    // a slot; auxiliary lane outputs have none and copy
                    // through untouched. The jack carries a switch like
                    // any card's stub: it measures before it breaks, so
                    // the meter keeps reading what arrives at the stub
                    // while an open circuit holds the DAC silent. Only
                    // the primary output carries a slot; auxiliary lane
                    // outputs have none and copy through untouched.
                    let wire_in = node.wire_in;
                    if let Some(slot) = wire_in {
                        apply_gain(&mut node.input, frames, wires.gain(slot));
                    }
                    for (source, destination) in node.input.0.iter().zip(&mut node.output.0) {
                        destination[..frames].copy_from_slice(&source[..frames]);
                    }
                    if let Some(slot) = wire_in {
                        let level = wire_level(&node.output, frames, &mut node.tp_out);
                        wires.publish(slot, level);
                        // Publish first, then break: the meter describes
                        // the signal reaching the jack, the cleared
                        // buffer keeps it out of the audio device.
                        if wires.open_at(slot) {
                            node.output.clear(frames);
                        }
                    }
                }
                NodeKind::ChannelMap { .. } if node.input.channels() != node.output.channels() => {
                    // A join folds every input to their mean — the stereo
                    // pair keeps its exact (L + R) * 0.5 form, which a halved
                    // sum reproduces bit for bit — and a split hands the
                    // first channel to every output. Equal widths never reach
                    // here; the copy arm below carries those.
                    let (inputs, outputs) = (node.input.channels(), node.output.channels());
                    let inputs = &node.input.0[..inputs];
                    let outputs = &mut node.output.0[..outputs];
                    for frame in 0..frames {
                        let value = if outputs.len() < inputs.len() {
                            let sum: f32 = inputs.iter().map(|lane| lane[frame]).sum();
                            sum / inputs.len() as f32
                        } else {
                            inputs[0][frame]
                        };
                        for lane in outputs.iter_mut() {
                            lane[frame] = value;
                        }
                    }
                }
                _ => {
                    for (source, destination) in node.input.0.iter().zip(&mut node.output.0) {
                        destination[..frames].copy_from_slice(&source[..frames]);
                    }
                }
            }
            if let (Some((times, _)), Some(started), Some(node_started), Some(slot)) =
                (&self.timings, started, node_started, node.timing_slot)
            {
                let end = Instant::now();
                times.publish(
                    slot,
                    ProcessingTime {
                        incoming: node_started.duration_since(started).as_secs_f64() / deadline,
                        added: end.duration_since(node_started).as_secs_f64() / deadline,
                    },
                    end,
                    end.duration_since(started),
                );
            }
        }
        let Self {
            nodes,
            output_buses,
            ..
        } = self;
        for (bus, group) in output_buses.iter().zip(outputs.iter_mut()) {
            for (source, destination) in nodes[bus.index].output.0.iter().zip(group.iter_mut()) {
                destination[..frames].copy_from_slice(&source[..frames]);
            }
        }
        drop(graph_observation);
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
