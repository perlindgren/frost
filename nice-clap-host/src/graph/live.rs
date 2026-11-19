#[path = "../controls.rs"]
mod controls;
pub(super) use controls::Editor;
pub use controls::{EditorEvent, Parameter};
// Main-thread ownership and a separate callback-side prepared graph.
use super::*;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveStatus {
    Running,
    StopRequested,
    Stopped,
}

/// Main-thread owner. Keep servicing callbacks while audio is running. Before
/// destroying streams or this owner, request stop, keep invoking the audio side
/// until it hands back the graph, and service until Stopped. This first version
/// supports a dedicated DSP worker which keeps polling without device input;
/// the rack uses that arrangement for recovery from device callback loss.
pub struct LiveSession {
    bindings: BTreeMap<NodeId, Binding>,
    graph: RackGraph,
    owners: BTreeMap<NodeId, Owner>,
    returned: rtrb::Consumer<Box<PreparedRack>>,
    stop: Arc<AtomicBool>,
    stopped: bool,
    latency: usize,
}

/// Move to the audio thread. All sample/port buffers are allocated beforehand.
/// After stop is requested, process hands the stopped graph to the main thread
/// through a bounded SPSC queue, leaving this side silent. Do not destroy an
/// active LiveAudio on the callback; complete the cooperative stop first.
pub struct LiveAudio {
    rack: Option<Box<PreparedRack>>,
    returned: rtrb::Producer<Box<PreparedRack>>,
    stop: Arc<AtomicBool>,
}
impl Drop for LiveAudio {
    fn drop(&mut self) {
        // Runs on the DSP worker, including ordinary Rust unwinding. Only a
        // stopped plan crosses to the main thread; no native instance is freed.
        if let Some(mut rack) = self.rack.take() {
            rack.stop();
            if let Err(rtrb::PushError::Full(rack)) = self.returned.push(rack) {
                drop(rack);
            }
        }
    }
}
impl LiveAudio {
    /// The empty/stopping path silences output. No waiting, graph allocation,
    /// native destruction, state I/O or lifecycle activation occurs here.
    pub fn process(
        &mut self,
        input: &[&[f32]],
        outputs: &mut [&mut [&mut [f32]]],
        frames: usize,
    ) -> Result<LiveStatus, ProcessError> {
        if self.rack.is_none() || self.stop.load(Ordering::Acquire) {
            for group in outputs.iter_mut() {
                for channel in group.iter_mut() {
                    let n = frames.min(channel.len());
                    channel[..n].fill(0.0);
                }
            }
            if let Some(mut rack) = self.rack.take() {
                rack.stop();
                if let Err(rtrb::PushError::Full(rack)) = self.returned.push(rack) {
                    self.rack = Some(rack);
                    return Ok(LiveStatus::StopRequested);
                }
            }
            return Ok(LiveStatus::Stopped);
        }
        let result = self.rack.as_mut().unwrap().process(input, outputs, frames);
        if matches!(result, Err(ProcessError::NodeFailed(_))) {
            self.stop.store(true, Ordering::Release);
        }
        result.map(|()| LiveStatus::Running)
    }

    /// Control-thread registration of per-card block-budget observations.
    pub fn attach_processing_times(
        &mut self,
        times: std::sync::Arc<nice_rack_engine::audio::ProcessingTimes>,
        sample_rate: f64,
    ) {
        if let Some(rack) = &mut self.rack {
            rack.attach_processing_times(times, sample_rate);
        }
    }

    /// Hand the shared per-wire meter table to the prepared graph. Control
    /// thread only, and only before this `LiveAudio` crosses to the DSP
    /// worker: the attach resets the table and registers every metered wire
    /// in one synchronous step, so no audio-side publish can observe a
    /// half-registered plan.
    pub fn attach_wire_levels(
        &mut self,
        levels: std::sync::Arc<nice_rack_engine::audio::WireLevels>,
    ) {
        if let Some(rack) = &mut self.rack {
            rack.attach_wire_levels(levels);
        }
    }
}
impl LiveSession {
    pub fn latency_samples(&self) -> usize {
        self.latency
    }
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    /// Service main-thread callbacks and retire a returned graph. No wait for
    /// audio is performed. Error messages are produced here, never in process.
    pub fn service(&mut self) -> Result<LiveStatus, String> {
        if let Ok(rack) = self.returned.pop() {
            drop(rack); // stopped handles return to their owning instances
            self.stopped = true;
        }
        for (&id, owner) in &mut self.owners {
            owner.service();
            if owner.signals.0.restart.load(Ordering::Acquire)
                || owner.signals.0.latency_changed.load(Ordering::Acquire)
            {
                self.stop.store(true, Ordering::Release);
            }
            if self.stopped {
                if owner.instance.is_active() {
                    return Err(format!("Plugin {id:?} did not return its native handle"));
                }
                if let Some(error) = &owner.failure {
                    return Err(format!("Plugin {id:?}: {error}"));
                }
            }
        }
        Ok(if self.stopped {
            LiveStatus::Stopped
        } else if self.stop.load(Ordering::Acquire) {
            LiveStatus::StopRequested
        } else {
            LiveStatus::Running
        })
    }

    /// Rebuild on the main thread after cooperative stop. Supported plugin state
    /// is snapshotted and restored to replacement instances before activation.
    /// Failure leaves this stopped session intact, allowing a retry/rollback.
    pub fn reconfigure(
        &mut self,
        sample_rate: f64,
        max_frames: usize,
    ) -> Result<LiveAudio, String> {
        if !self.stopped {
            return Err("Stop and retire the audio graph before reconfiguration".into());
        }
        let states = self.snapshot()?;
        let (replacement, audio) = Self::prepare(
            &self.graph,
            &self.bindings,
            sample_rate,
            max_frames,
            &states,
        )?;
        *self = replacement;
        Ok(audio)
    }
}
#[derive(Clone)]
pub struct Binding {
    pub entry: Entry,
    pub class: String,
}

impl Entry {
    pub fn prepare_live(
        &self,
        graph: &RackGraph,
        classes: &BTreeMap<NodeId, String>,
        sample_rate: f64,
        max_frames: usize,
    ) -> Result<(LiveSession, LiveAudio), String> {
        let bindings = classes
            .iter()
            .map(|(&id, class)| {
                (
                    id,
                    Binding {
                        entry: self.clone(),
                        class: class.clone(),
                    },
                )
            })
            .collect();
        LiveSession::prepare(graph, &bindings, sample_rate, max_frames, &BTreeMap::new())
    }
}
impl LiveSession {
    pub fn snapshot(&mut self) -> Result<BTreeMap<NodeId, Vec<u8>>, String> {
        if !self.stopped {
            return Err("Stop the graph before saving state".into());
        }
        let mut states = BTreeMap::new();
        for (&id, owner) in &mut self.owners {
            if let Some(bytes) = save_state(&mut owner.instance)? {
                states.insert(id, bytes);
            }
        }
        Ok(states)
    }
    pub fn prepare(
        graph: &RackGraph,
        bindings: &BTreeMap<NodeId, Binding>,
        sample_rate: f64,
        max_frames: usize,
        states: &BTreeMap<NodeId, Vec<u8>>,
    ) -> Result<(Self, LiveAudio), String> {
        Self::build(
            BTreeMap::new(),
            graph,
            bindings,
            sample_rate,
            max_frames,
            states,
        )
    }
    /// Build the chain a roster edit asks for, carrying over every instance that
    /// edit did not touch.
    ///
    /// Saving and restoring state is a favour a plugin does its host, and not
    /// every plugin does it: what a snapshot cannot carry — an editor on screen,
    /// which page of it the user was looking at, a meter's history — is lost when
    /// the instance is replaced. A card whose instance was made from the class
    /// that card still names, and whose buses are still the ones the graph wants
    /// around it, has no reason to be rebuilt at all, so it keeps its instance and
    /// everything hanging off it, and only the plugins the edit actually changed
    /// pay for the new chain. `states` is for those, and for a `previous` session
    /// whose instances could not be carried — refused, still on the transport, or
    /// asked for a different configuration — which is what a rebuild without this
    /// looked like.
    pub fn reusing(
        previous: Option<Self>,
        graph: &RackGraph,
        bindings: &BTreeMap<NodeId, Binding>,
        sample_rate: f64,
        max_frames: usize,
        states: &BTreeMap<NodeId, Vec<u8>>,
    ) -> Result<(Self, LiveAudio), String> {
        Self::build(
            previous.map_or_else(BTreeMap::new, |previous| previous.owners),
            graph,
            bindings,
            sample_rate,
            max_frames,
            states,
        )
    }
    fn build(
        mut previous: BTreeMap<NodeId, Owner>,
        graph: &RackGraph,
        bindings: &BTreeMap<NodeId, Binding>,
        sample_rate: f64,
        max_frames: usize,
        states: &BTreeMap<NodeId, Vec<u8>>,
    ) -> Result<(Self, LiveAudio), String> {
        if !sample_rate.is_finite() || sample_rate <= 0.0 || !(1..=65536).contains(&max_frames) {
            return Err("Invalid live graph audio configuration".into());
        }
        graph.compile().map_err(|error| error.to_string())?;
        let count = graph
            .nodes
            .values()
            .filter(|kind| matches!(kind, NodeKind::Plugin { .. }))
            .count();
        if bindings.len() != count
            || bindings
                .keys()
                .any(|id| !matches!(graph.nodes.get(id), Some(NodeKind::Plugin { .. })))
        {
            return Err("Each plugin node needs one class ID".into());
        }
        let mut owners = BTreeMap::new();
        let mut processors = BTreeMap::new();
        let setup = (|| {
            for (&id, binding) in bindings {
                let NodeKind::Plugin { inputs, outputs } = graph.nodes[&id] else {
                    unreachable!()
                };
                let (inputs, outputs) = (channel_count(inputs), channel_count(outputs));
                // The instance in front of this card is only worth carrying if it
                // still answers to it. Refused, it dies here with the rest of what
                // this edit did not carry, its editor with it.
                let kept = previous.remove(&id).and_then(|mut owner| {
                    owner
                        .reusable(&binding.class, inputs, outputs)
                        .then_some(owner)
                });
                let (owner, processor) = match kept {
                    Some(mut owner) => {
                        let processor =
                            owner.reactivate(sample_rate, max_frames, inputs, outputs)?;
                        (owner, processor)
                    }
                    None => binding.entry.graph_instance(
                        &binding.class,
                        sample_rate,
                        max_frames,
                        inputs,
                        outputs,
                        states.get(&id).map(Vec::as_slice),
                    )?,
                };
                owners.insert(id, owner);
                processors.insert(
                    id,
                    ProcessorSlot {
                        processor: Box::new(processor),
                        bypassed: false,
                    },
                );
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = setup {
            drop(processors);
            for owner in owners.values_mut() {
                owner.service();
            }
            return Err(error);
        }
        let rack = match PreparedRack::prepare(graph, processors, max_frames) {
            Ok(rack) => Box::new(rack),
            Err(error) => {
                for owner in owners.values_mut() {
                    owner.service();
                }
                return Err(error.to_string());
            }
        };
        let latency = rack.latency_samples();
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = rtrb::RingBuffer::new(1);
        Ok((
            LiveSession {
                bindings: bindings.clone(),
                graph: graph.clone(),
                owners,
                returned: receiver,
                stop: stop.clone(),
                stopped: false,
                latency,
            },
            LiveAudio {
                rack: Some(rack),
                returned: sender,
                stop,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nice_rack_engine::{Channels, Connection};
    use std::sync::atomic::AtomicUsize;
    struct Pass(Arc<AtomicUsize>);
    impl Processor for Pass {
        fn process(
            &mut self,
            input: &AudioBuffer,
            output: &mut AudioBuffer,
            frames: usize,
        ) -> Result<(), ProcessError> {
            output
                .channel_mut(0, frames)
                .copy_from_slice(input.channel(0, frames));
            Ok(())
        }
    }
    impl Drop for Pass {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[test]
    fn callback_handoff_does_not_allocate_or_destroy_the_graph() {
        let dropped = Arc::new(AtomicUsize::new(0));
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
            ..RackGraph::default()
        };
        let slots = [(
            NodeId(1),
            ProcessorSlot {
                processor: Box::new(Pass(dropped.clone())),
                bypassed: false,
            },
        )]
        .into();
        let rack = Box::new(PreparedRack::prepare(&graph, slots, 16).unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let (returned, mut receive) = rtrb::RingBuffer::new(1);
        let mut audio = LiveAudio {
            rack: Some(rack),
            returned,
            stop: stop.clone(),
        };
        let mut output = [0.0; 16];
        let allocations = allocation_counter::measure(|| {
            assert_eq!(
                audio.process(&[&[0.25; 16]], &mut [&mut [&mut output[..]][..]], 16),
                Ok(LiveStatus::Running)
            );
            assert_eq!(output, [0.25; 16]);
            stop.store(true, Ordering::Release);
            assert_eq!(
                audio.process(&[], &mut [&mut [&mut output[..]][..]], 16),
                Ok(LiveStatus::Stopped)
            );
            assert_eq!(output, [0.0; 16]);
            assert_eq!(
                audio.process(&[], &mut [&mut [&mut output[..]][..]], 16),
                Ok(LiveStatus::Stopped)
            );
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(allocations.count_current, 0);
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        drop(receive.pop().unwrap());
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
}
