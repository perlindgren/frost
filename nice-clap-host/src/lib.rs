//! CLAP hosting prototype with offline rendering and callback-side lifecycle.
//! Main-thread native editors and parameters, with audio-thread event handoff.
#![deny(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]

mod graph;
pub use graph::{Binding, EditorEvent, GraphReport, LiveAudio, LiveSession, LiveStatus, Parameter};

use clack_extensions::{audio_ports::*, gui::*, latency::*, params::*, state::*};

use clack_host::{
    events::io::{InputEvents, OutputEvents},
    prelude::*,
};
use std::{
    ffi::CString,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
};

#[derive(Default)]
struct Signals {
    callback: AtomicBool,
    restart: AtomicBool,
    process: AtomicBool,
    changed: AtomicBool,
    latency_changed: AtomicBool,
    gui_resize: AtomicU64,
    gui_visibility: AtomicU8,
    params_changed: AtomicBool,
}
#[derive(Clone, Default)]
struct Callbacks(Arc<Signals>);
impl<'a> SharedHandler<'a> for Callbacks {
    fn request_callback(&self) {
        self.0.callback.store(true, Ordering::Release);
    }
    fn request_restart(&self) {
        self.0.restart.store(true, Ordering::Release);
    }
    fn request_process(&self) {
        self.0.process.store(true, Ordering::Release);
    }
}
impl<'a> MainThreadHandler<'a> for Callbacks {}
impl HostLatencyImpl for Callbacks {
    fn changed(&self) {
        self.0.latency_changed.store(true, Ordering::Release);
    }
}
impl HostStateImpl for Callbacks {
    fn mark_dirty(&self) {
        self.0.changed.store(true, Ordering::Release);
    }
}
struct Host;
impl HostHandlers for Host {
    type Shared<'a> = Callbacks;
    type MainThread<'a> = Callbacks;
    type AudioProcessor<'a> = ();
    fn declare_extensions(builder: &mut HostExtensions<Self>, _: &Callbacks) {
        builder
            .register::<HostLatency>()
            .register::<HostState>()
            .register::<HostGui>()
            .register::<HostParams>();
    }
}

#[derive(Debug, Clone)]
pub struct Descriptor {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub description: String,
}

/// Every entry this process has opened, held for the life of the process.
///
/// A `.clap` file is not data with a lifetime, it is code. A plugin can leave
/// a thread of its own running on after its last instance is gone — nice-nam
/// starts a preparation thread per instance which wakes every twenty
/// milliseconds and nothing joins it — and a library released underneath a
/// thread still standing on it is an access violation reported against a
/// module the loader has already unmapped, in a fault no host can catch and
/// no stack trace explains. Hosts that have been shown this once keep plugins
/// mapped until the process ends, and so do we: the price is one handle per
/// distinct file, which the operating system takes back at exit anyway.
static LOADED: Mutex<Vec<(PathBuf, PluginEntry)>> = Mutex::new(Vec::new());

/// An entry whose lifetime is managed by Clack. Instantiation stays on the
/// caller's main thread. Only explicitly trusted entries may be loaded.
#[derive(Clone)]
pub struct Entry(PluginEntry);
impl Entry {
    /// Load native executable code, including any dynamic-library constructors.
    ///
    /// The library then stays mapped for the life of the process, whether or
    /// not this handle and everything built from it is dropped, because a
    /// plugin's own threads are not something a host can account for: see
    /// [`LOADED`].
    ///
    /// # Safety
    /// The caller must ensure this library and its plugins are trustworthy and
    /// comply with the CLAP ABI/contracts. Paths and metadata cannot prove this.
    /// See Clack's PluginEntry::load safety contract. This prototype provides no
    /// crash isolation and must be called from the application's main thread.
    #[allow(unsafe_code)]
    pub unsafe fn load(path: &Path) -> Result<Self, String> {
        // Two spellings of one file are one plugin, so a card dropped and
        // dropped again does not ask the same entry to initialise twice.
        let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let known = LOADED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|(kept, _)| *kept == key)
            .map(|(_, entry)| entry.clone());
        if let Some(entry) = known {
            return Ok(Self(entry));
        }
        // SAFETY: caller accepts the same native-library and CLAP contracts.
        // The lock above was released before this call: loading runs the
        // plugin's own initialisation, and foreign code does not get to run
        // while holding our mutex.
        let entry = unsafe { PluginEntry::load(path) }
            .map_err(|error| error.to_string())
            .map(Self)?;
        LOADED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((key, entry.0.clone()));
        Ok(entry)
    }

    /// Query a class's currently selected main buses without activating DSP.
    pub fn main_channels(
        &self,
        id: &str,
    ) -> Result<(nice_rack_engine::Channels, nice_rack_engine::Channels), String> {
        let class = CString::new(id).map_err(|error| error.to_string())?;
        let shared = Callbacks::default();
        let main = shared.clone();
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
        let handle = instance.plugin_handle();
        let extension = handle
            .get_extension::<PluginAudioPorts>()
            .ok_or("Plugin has no audio ports")?;
        let mut buffer = AudioPortInfoBuffer::new();
        let mut query = |direction| {
            if extension.count(&handle, direction) != 1 {
                return Err("Only a single main audio bus is supported".to_owned());
            }
            let port = extension
                .get(&handle, 0, direction, &mut buffer)
                .ok_or("Could not query plugin bus")?;
            if !port.flags.contains(AudioPortFlags::IS_MAIN) {
                return Err("Plugin bus is not a main bus".into());
            }
            match port.channel_count {
                1 => Ok(nice_rack_engine::Channels::Mono),
                2 => Ok(nice_rack_engine::Channels::Stereo),
                _ => Err("Only mono/stereo effects are supported".into()),
            }
        };
        Ok((query(true)?, query(false)?))
    }

    pub fn descriptors(&self) -> Result<Vec<Descriptor>, String> {
        let factory = self
            .0
            .get_plugin_factory()
            .ok_or("CLAP entry has no plugin factory")?;
        factory
            .plugin_descriptors()
            .map(|descriptor| {
                Ok(Descriptor {
                    vendor: descriptor
                        .vendor()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    version: descriptor
                        .version()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    description: descriptor
                        .description()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    id: descriptor
                        .id()
                        .ok_or("Plugin descriptor has no ID")?
                        .to_str()
                        .map_err(|e| e.to_string())?
                        .into(),
                    name: descriptor
                        .name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                })
            })
            .collect()
    }

    /// Process one offline block on a worker, keeping callbacks, activation,
    /// state I/O and deactivation on this main thread. This is a diagnostic
    /// operation, not a realtime graph processor. Only single mono/stereo main
    /// buses are accepted. A plugin requesting restart is reported to the caller.
    pub fn probe(&self, id: &str, sample_rate: f64, frames: u32) -> Result<ProbeReport, String> {
        if !sample_rate.is_finite() || sample_rate <= 0.0 || frames == 0 || frames > 65536 {
            return Err("Probe requires a positive sample rate and 1..=65536 frames".into());
        }
        let id = CString::new(id).map_err(|e| e.to_string())?;
        let signals = Callbacks::default();
        let shared = signals.clone();
        let main = signals.clone();
        let info = HostInfo::new(
            "Nice Rack Probe",
            "frost",
            "https://github.com/perlindgren/frost",
            "0.1.0",
        )
        .map_err(|e| e.to_string())?;
        let mut instance = PluginInstance::<Host>::new(|_| shared, |_| main, &self.0, &id, &info)
            .map_err(|e| e.to_string())?;
        pump(&mut instance, &signals);
        let (inputs, outputs) = {
            let handle = instance.plugin_handle();
            let ports = handle
                .get_extension::<PluginAudioPorts>()
                .ok_or("Plugin does not expose audio ports")?;
            let mut scratch = AudioPortInfoBuffer::new();
            let mut channels = |input| -> Result<usize, String> {
                if ports.count(&handle, input) != 1 {
                    return Err("Probe requires exactly one bus in each direction".into());
                }
                let port = ports
                    .get(&handle, 0, input, &mut scratch)
                    .ok_or("Could not query audio port")?;
                if !port.flags.contains(AudioPortFlags::IS_MAIN)
                    || !(1..=2).contains(&port.channel_count)
                {
                    return Err("Probe supports only mono/stereo main buses".into());
                }
                Ok(port.channel_count as usize)
            };
            (channels(true)?, channels(false)?)
        };
        let state_before = save_state(&mut instance)?;
        if let Some(state) = &state_before {
            let handle = instance.plugin_handle();
            handle
                .get_extension::<PluginState>()
                .unwrap()
                .load(&handle, &mut state.as_slice())
                .map_err(|e| e.to_string())?;
        }
        pump(&mut instance, &signals);
        let stopped = instance
            .activate(
                |_, _| (),
                PluginAudioConfiguration {
                    sample_rate,
                    min_frames_count: 1,
                    max_frames_count: frames,
                },
            )
            .map_err(|e| e.to_string())?;
        let latency = {
            let handle = instance.plugin_handle();
            handle
                .get_extension::<PluginLatency>()
                .map(|ext| ext.get(&handle))
                .unwrap_or(0)
        };
        // These allocations precede start_processing. The offline worker owns
        // them and returns its stopped processor for main-thread deactivation.
        let mut input = vec![vec![0.125f32; frames as usize]; inputs];
        let mut output = vec![vec![0.0f32; frames as usize]; outputs];
        let mut in_ports = AudioPorts::with_capacity(inputs, 1);
        let mut out_ports = AudioPorts::with_capacity(outputs, 1);
        let (stopped, processed) = std::thread::scope(|scope| {
            let job = scope.spawn(move || {
                let mut processor: clack_host::process::PluginAudioProcessor<Host> = stopped.into();
                let processed = (|| {
                    let started = processor.start_processing().map_err(|e| e.to_string())?;
                    let input_audio = in_ports.with_input_buffers([AudioPortBuffer {
                        latency: 0,
                        channels: AudioPortBufferType::f32_input_only(
                            input.iter_mut().map(InputChannel::variable),
                        ),
                    }]);
                    let mut output_audio = out_ports.with_output_buffers([AudioPortBuffer {
                        latency: 0,
                        channels: AudioPortBufferType::f32_output_only(
                            output.iter_mut().map(|b| b.as_mut_slice()),
                        ),
                    }]);
                    started
                        .process(
                            &input_audio,
                            &mut output_audio,
                            &InputEvents::empty(),
                            &mut OutputEvents::void(),
                            Some(0),
                            None,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok::<_, String>(())
                })();
                if processor.is_started() {
                    processor.stop_processing().unwrap();
                }
                let clack_host::process::PluginAudioProcessor::Stopped(stopped) = processor else {
                    unreachable!()
                };
                (stopped, processed.map(|()| output))
            });
            while !job.is_finished() {
                pump(&mut instance, &signals);
                std::thread::park_timeout(std::time::Duration::from_millis(1));
            }
            job.join()
                .map_err(|_| "Plugin processing worker panicked".to_owned())
        })?;
        instance.deactivate(stopped);
        pump(&mut instance, &signals);
        let audio = processed?;
        if signals.0.restart.load(Ordering::Acquire) {
            return Err(
                "Plugin requested restart; the offline probe stopped and deactivated it".into(),
            );
        }
        let state_after = save_state(&mut instance)?;
        Ok(ProbeReport {
            inputs,
            outputs,
            frames,
            latency,
            audio,
            state_supported: state_before.is_some(),
            state_bytes_after: state_after.as_ref().map(|s| s.len()),
            state_roundtrip_matches: state_before
                .as_ref()
                .zip(state_after.as_ref())
                .map(|(before, after)| before == after),
            process_requested: signals.0.process.load(Ordering::Acquire),
        })
    }
}

fn pump(instance: &mut PluginInstance<Host>, signals: &Callbacks) {
    // Bound reentrant requests so one plugin cannot monopolize the main thread.
    if signals.0.callback.swap(false, Ordering::AcqRel) {
        instance.call_on_main_thread_callback();
    }
}
fn save_state(instance: &mut PluginInstance<Host>) -> Result<Option<Vec<u8>>, String> {
    let handle = instance.plugin_handle();
    let Some(state) = handle.get_extension::<PluginState>() else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    state.save(&handle, &mut bytes).map_err(|e| e.to_string())?;
    Ok(Some(bytes))
}
#[derive(Debug)]
pub struct ProbeReport {
    pub inputs: usize,
    pub outputs: usize,
    pub frames: u32,
    pub latency: u32,
    pub audio: Vec<Vec<f32>>,
    pub state_supported: bool,
    pub state_bytes_after: Option<usize>,
    /// Byte equality only; plugins may legitimately change state while processing.
    pub state_roundtrip_matches: Option<bool>,
    pub process_requested: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callbacks_remain_observable_until_the_owner_handles_them() {
        let callbacks = Callbacks::default();
        callbacks.request_callback();
        callbacks.request_restart();
        callbacks.request_process();
        assert!(callbacks.0.callback.swap(false, Ordering::AcqRel));
        assert!(!callbacks.0.callback.load(Ordering::Acquire));
        assert!(callbacks.0.restart.load(Ordering::Acquire));
        assert!(callbacks.0.process.load(Ordering::Acquire));
    }
}
