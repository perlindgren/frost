//! Bridge between main-thread CLAP instances and the rack processor.
mod live;
use super::*;
use clack_host::{
    plugin::PluginInstanceError,
    process::{PluginAudioProcessor, StoppedPluginAudioProcessor},
};
pub use live::{Binding, EditorEvent, LiveAudio, LiveSession, LiveStatus, Parameter};
use nice_rack_engine::{
    NodeId, NodeKind, RackGraph,
    audio::{AudioBuffer, PreparedRack, ProcessError, Processor, ProcessorSlot},
};
use std::{
    collections::BTreeMap,
    sync::mpsc::{Receiver, SyncSender, sync_channel},
};

type ReturnHandle = (
    StoppedPluginAudioProcessor<Host>,
    Option<PluginInstanceError>,
);
struct Owner {
    instance: PluginInstance<Host>,
    signals: Callbacks,
    returned: Receiver<ReturnHandle>,
    failure: Option<String>,
    editor: Option<live::Editor>,
    changes: rtrb::Producer<clack_host::events::event_types::ParamValueEvent>,
}
impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(editor) = self.editor.take() {
            let handle = self.instance.plugin_handle();
            if let Some(gui) = handle.get_extension::<PluginGui>() {
                gui.destroy(&handle);
            }
            drop(editor);
        }
    }
}
impl Owner {
    fn service(&mut self) {
        pump(&mut self.instance, &self.signals);
        if !self.instance.is_active()
            && self.signals.0.process.swap(false, Ordering::AcqRel)
            && let Some(mut handle) = self.instance.inactive_plugin_handle()
            && let Some(params) = handle.get_extension::<PluginParams>()
        {
            params.flush(
                &mut handle,
                &InputEvents::empty(),
                &mut OutputEvents::void(),
            );
        }
        if let Ok((stopped, failure)) = self.returned.try_recv() {
            self.instance.deactivate(stopped);
            self.failure = failure.map(|error| error.to_string());
        }
    }
}

struct ClapProcessor {
    processor: Option<PluginAudioProcessor<Host>>,
    returned: SyncSender<ReturnHandle>,
    failure: Option<PluginInstanceError>,
    input: Vec<Vec<f32>>,
    input_ports: AudioPorts,
    output_ports: AudioPorts,
    latency: usize,
    steady_time: u64,
    changes: rtrb::Consumer<clack_host::events::event_types::ParamValueEvent>,
    events: Vec<clack_host::events::event_types::ParamValueEvent>,
    restart: Callbacks,
}
impl Processor for ClapProcessor {
    fn latency_samples(&self) -> usize {
        self.latency
    }
    fn process(
        &mut self,
        input: &AudioBuffer,
        output: &mut AudioBuffer,
        frames: usize,
    ) -> Result<(), ProcessError> {
        if self.restart.0.restart.load(Ordering::Acquire)
            || self.restart.0.latency_changed.load(Ordering::Acquire)
        {
            return Err(ProcessError::ProcessorFailed);
        }
        let processor = self.processor.as_mut().unwrap();
        if !processor.is_started()
            && let Err(error) = processor.start_processing()
        {
            self.failure = Some(error);
            return Err(ProcessError::ProcessorFailed);
        }
        for (channel, data) in self.input.iter_mut().enumerate() {
            data[..frames].copy_from_slice(input.channel(channel, frames));
        }
        let inputs = self.input_ports.with_input_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_input_only(
                self.input
                    .iter_mut()
                    .map(|data| InputChannel::variable(&mut data[..frames])),
            ),
        }]);
        let mut outputs = self.output_ports.with_output_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_output_only(output.channel_slices_mut(frames)),
        }]);
        self.events.clear();
        while self.events.len() < 64 {
            let Ok(event) = self.changes.pop() else {
                break;
            };
            self.events.push(event);
        }
        if let Err(error) = processor.as_started_mut().unwrap().process(
            &inputs,
            &mut outputs,
            &InputEvents::from_buffer(&self.events),
            &mut OutputEvents::void(),
            Some(self.steady_time),
            None,
        ) {
            self.failure = Some(error);
            return Err(ProcessError::ProcessorFailed);
        }
        self.steady_time = self.steady_time.wrapping_add(frames as u64);
        Ok(())
    }
    fn reset(&mut self) {
        self.processor.as_mut().unwrap().reset();
        self.steady_time = 0;
    }
    fn stop(&mut self) {
        let processor = self.processor.as_mut().unwrap();
        if processor.is_started() {
            processor.stop_processing().unwrap();
        }
    }
}
impl Drop for ClapProcessor {
    fn drop(&mut self) {
        // Ordinary processing never sends or locks. This one-shot handback is
        // retirement/control work. Its capacity is one and it has one sender.
        self.stop();
        let PluginAudioProcessor::Stopped(stopped) = self.processor.take().unwrap() else {
            unreachable!()
        };
        let _ = self.returned.send((stopped, self.failure.take()));
    }
}

#[derive(Debug)]
pub struct GraphReport {
    pub audio: Vec<Vec<f32>>,
    pub latency_samples: usize,
    pub saved_state_bytes: BTreeMap<NodeId, Option<usize>>,
}
impl Entry {
    /// Render an offline graph using classes from this trusted entry. Creates
    /// distinct instances for every plugin node, even if their IDs are equal.
    /// There is no live graph mutation, editor, device I/O, MIDI or automation.
    pub fn render_graph(
        &self,
        graph: &RackGraph,
        plugins: &BTreeMap<NodeId, String>,
        input: &[Vec<f32>],
        sample_rate: f64,
        max_frames: usize,
    ) -> Result<GraphReport, String> {
        if !sample_rate.is_finite() || sample_rate <= 0.0 || !(1..=65536).contains(&max_frames) {
            return Err("Invalid graph audio configuration".into());
        }
        graph.compile().map_err(|error| error.to_string())?;
        let frames = input.first().map_or(0, Vec::len);
        let input_channels = graph
            .nodes
            .values()
            .find_map(|kind| match kind {
                NodeKind::Input(channels) => Some(channel_count(*channels)),
                _ => None,
            })
            .ok_or("Graph has no input")?;
        if frames == 0
            || input.len() != input_channels
            || input.iter().any(|channel| channel.len() != frames)
        {
            return Err(
                "Input must contain equal-length nonempty channels matching the graph".into(),
            );
        }
        let output_channels = graph
            .nodes
            .values()
            .find_map(|kind| match kind {
                NodeKind::Output(channels) => Some(channel_count(*channels)),
                _ => None,
            })
            .ok_or("Graph has no output")?;
        let plugin_count = graph
            .nodes
            .values()
            .filter(|kind| matches!(kind, NodeKind::Plugin { .. }))
            .count();
        if plugins.len() != plugin_count
            || plugins
                .keys()
                .any(|id| !matches!(graph.nodes.get(id), Some(NodeKind::Plugin { .. })))
        {
            return Err("Every plugin node needs exactly one class ID".into());
        }
        let mut owners = BTreeMap::new();
        let mut processors = BTreeMap::new();
        // On partial setup failure, drop the processor slots before servicing
        // their owners, so activated handles always return for deactivation.
        let setup = (|| {
            for (&id, class) in plugins {
                let NodeKind::Plugin { inputs, outputs } = graph.nodes[&id] else {
                    unreachable!()
                };
                let (owner, processor) = self.graph_instance(
                    class,
                    sample_rate,
                    max_frames,
                    channel_count(inputs),
                    channel_count(outputs),
                    None,
                )?;
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
        // Allocate final render storage and pointer arrays before processing.
        // The offline result itself can exceed a block, unlike plan buffers.
        let mut rendered = vec![vec![0.0; frames]; output_channels];
        let outcome = std::thread::scope(|scope| {
            let worker = scope.spawn(move || {
                let mut rack = PreparedRack::prepare(graph, processors, max_frames)
                    .map_err(|error| error.to_string())?;
                let latency = rack.latency_samples();
                let mut result = Ok(());
                for offset in (0..frames).step_by(max_frames) {
                    let count = max_frames.min(frames - offset);
                    let inputs: [&[f32]; 2] = std::array::from_fn(|channel| {
                        input
                            .get(channel)
                            .map_or(&[][..], |data| &data[offset..offset + count])
                    });
                    let mut outputs = rendered
                        .iter_mut()
                        .map(|data| &mut data[offset..offset + count]);
                    let mut first = outputs.next().unwrap();
                    let mut second = outputs.next().unwrap_or(&mut []);
                    let mut buffers: [&mut [f32]; 2] = [&mut first, &mut second];
                    if let Err(error) = rack.process(
                        &inputs[..input_channels],
                        &mut buffers[..output_channels],
                        count,
                    ) {
                        result = Err(format!("Graph processing failed: {error:?}"));
                        break;
                    }
                }
                rack.stop();
                // Return the stopped plan for main-thread retirement, even on
                // a normal process error. The plugin instances remain there.
                Ok::<_, String>((rack, rendered, latency, result))
            });
            while !worker.is_finished() {
                for owner in owners.values_mut() {
                    owner.service();
                }
                std::thread::park_timeout(std::time::Duration::from_millis(1));
            }
            worker
                .join()
                .map_err(|_| "Graph worker panicked".to_owned())
        });
        let mut audio_result = None;
        let mut render_error = None;
        match outcome {
            Ok(Ok((rack, audio, latency, result))) => {
                drop(rack);
                audio_result = Some((audio, latency));
                if let Err(error) = result {
                    render_error = Some(error);
                }
            }
            Ok(Err(error)) | Err(error) => render_error = Some(error),
        }
        for (&id, owner) in &mut owners {
            owner.service();
            if owner.instance.is_active() {
                return Err(format!(
                    "Plugin node {id:?} did not return its processing handle"
                ));
            }
            if let Some(error) = &owner.failure {
                return Err(format!("Plugin node {id:?}: {error}"));
            }
            if owner.signals.0.latency_changed.load(Ordering::Acquire) {
                return Err(format!(
                    "Plugin node {id:?} changed latency; graph requires preparation again"
                ));
            }
            if owner.signals.0.restart.load(Ordering::Acquire) {
                return Err(format!(
                    "Plugin node {id:?} requested restart; graph was stopped"
                ));
            }
        }
        if let Some(error) = render_error {
            return Err(error);
        }
        let mut saved_state_bytes = BTreeMap::new();
        for (&id, owner) in &mut owners {
            saved_state_bytes.insert(
                id,
                save_state(&mut owner.instance)?.map(|state| state.len()),
            );
        }
        let (audio, latency_samples) = audio_result.unwrap();
        Ok(GraphReport {
            audio,
            latency_samples,
            saved_state_bytes,
        })
    }

    fn graph_instance(
        &self,
        class: &str,
        rate: f64,
        frames: usize,
        inputs: usize,
        outputs: usize,
        state: Option<&[u8]>,
    ) -> Result<(Owner, ClapProcessor), String> {
        let class = CString::new(class).map_err(|error| error.to_string())?;
        let signals = Callbacks::default();
        let shared = signals.clone();
        let main = signals.clone();
        let info = HostInfo::new(
            "Nice Rack",
            "frost",
            "https://github.com/perlindgren/frost",
            "0.1.0",
        )
        .map_err(|error| error.to_string())?;
        let mut instance =
            PluginInstance::<Host>::new(|_| shared, |_| main, &self.0, &class, &info)
                .map_err(|error| error.to_string())?;
        pump(&mut instance, &signals);
        if let Some(mut bytes) = state {
            let handle = instance.plugin_handle();
            handle
                .get_extension::<PluginState>()
                .ok_or("Plugin cannot restore state")?
                .load(&handle, &mut bytes)
                .map_err(|error| error.to_string())?;
        }
        {
            let handle = instance.plugin_handle();
            let ports = handle
                .get_extension::<PluginAudioPorts>()
                .ok_or("Plugin has no audio port extension")?;
            let mut scratch = AudioPortInfoBuffer::new();
            for (is_input, expected) in [(true, inputs), (false, outputs)] {
                if ports.count(&handle, is_input) != 1 {
                    return Err("Graph adapter supports one main bus per direction".into());
                }
                let port = ports
                    .get(&handle, 0, is_input, &mut scratch)
                    .ok_or("Could not query plugin audio port")?;
                if port.channel_count as usize != expected
                    || !port.flags.contains(AudioPortFlags::IS_MAIN)
                {
                    return Err("Plugin main bus does not match graph channel configuration".into());
                }
            }
        }
        // State operations stay outside the worker. Live reconfiguration may
        // restore a snapshot before port validation and activation.
        let _ = save_state(&mut instance)?;
        let (returned, receiver) = sync_channel(1);
        let (changes, incoming_changes) = rtrb::RingBuffer::new(256);
        let input = vec![vec![0.0; frames]; inputs];
        let input_ports = AudioPorts::with_capacity(inputs, 1);
        let output_ports = AudioPorts::with_capacity(outputs, 1);
        signals.0.restart.store(false, Ordering::Release);
        let stopped = instance
            .activate(
                |_, _| (),
                PluginAudioConfiguration {
                    sample_rate: rate,
                    min_frames_count: 1,
                    max_frames_count: frames as u32,
                },
            )
            .map_err(|error| error.to_string())?;
        let latency = {
            let handle = instance.plugin_handle();
            handle
                .get_extension::<PluginLatency>()
                .map(|extension| extension.get(&handle) as usize)
                .unwrap_or(0)
        };
        signals.0.latency_changed.store(false, Ordering::Release);
        Ok((
            Owner {
                instance,
                signals: signals.clone(),
                returned: receiver,
                failure: None,
                editor: None,
                changes,
            },
            ClapProcessor {
                processor: Some(stopped.into()),
                returned,
                failure: None,
                input,
                input_ports,
                output_ports,
                latency,
                steady_time: 0,
                changes: incoming_changes,
                events: Vec::with_capacity(64),
                restart: signals,
            },
        ))
    }
}
fn channel_count(channels: nice_rack_engine::Channels) -> usize {
    match channels {
        nice_rack_engine::Channels::Mono => 1,
        nice_rack_engine::Channels::Stereo => 2,
    }
}
