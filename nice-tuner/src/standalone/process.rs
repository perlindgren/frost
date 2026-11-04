//! Adapter for the same Plugin activation and process calls used by VST3/CLAP.
use crate::TunerPlugin;
use cpal::Sample;
use nice_plug::context::process::SendEventError;
use nice_plug::prelude::*;
use std::sync::{Arc, Mutex};

pub(super) const BLOCK: usize = 1024;

pub(super) struct Context {
    transport: Transport,
}
impl Context {
    pub fn new(rate: f32) -> Self {
        Self {
            transport: Transport::new(rate),
        }
    }
}
// TunerPlugin has no MIDI, background tasks, latency or polyphonic voices.
impl ActivateContext<TunerPlugin> for Context {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }
    fn execute(&self, _: ()) {}
    fn set_latency_samples(&self, _: u32) {}
    fn set_current_voice_capacity(&self, _: u32) {}
}
impl ProcessContext<TunerPlugin> for Context {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }
    fn execute_background(&self, _: ()) {}
    fn execute_gui(&self, _: ()) {}
    fn transport(&self) -> &Transport {
        &self.transport
    }
    fn next_event(&mut self) -> Option<PluginNoteEvent<TunerPlugin>> {
        None
    }
    fn try_send_event(
        &mut self,
        event: PluginNoteEvent<TunerPlugin>,
    ) -> Result<(), (PluginNoteEvent<TunerPlugin>, SendEventError)> {
        Err((event, SendEventError::NoOutputBuffer))
    }
    fn set_latency_samples(&self, _: u32) {}
    fn request_restart(&self) {}
    fn set_current_voice_capacity(&self, _: u32) {}
}

pub(super) fn activate(plugin: &mut TunerPlugin, channels: usize, rate: f32) -> Result<(), String> {
    if let Some(&hz) = super::TEST_FREQUENCY.get()
        && hz >= f64::from(rate) / 2.0
    {
        return Err(format!(
            "Test frequency {hz} Hz must be below half the input sample rate ({rate} Hz)"
        ));
    }
    let layout = TunerPlugin::AUDIO_IO_LAYOUTS
        .iter()
        .find(|layout| layout.main_input_channels.map(|n| n.get() as usize) == Some(channels))
        .ok_or("The plugin does not support this input layout")?;
    plugin.deactivate();
    if !plugin.activate(
        layout,
        &BufferConfig {
            sample_rate: rate,
            min_buffer_size: None,
            max_buffer_size: BLOCK as u32,
            process_mode: ProcessMode::Realtime,
        },
        &mut Context::new(rate),
    ) {
        return Err("The tuner could not activate this audio configuration".into());
    }
    plugin.reset();
    Ok(())
}

pub(super) struct Processor {
    buffer: plugin_buffer::PlanarBuffer<{ 2 * BLOCK }>,
    channels: usize,
    context: Context,
    tone: Option<TestTone>,
    plugin: Arc<Mutex<TunerPlugin>>,
}
impl Processor {
    pub fn new(plugin: Arc<Mutex<TunerPlugin>>, channels: usize, rate: f32) -> Self {
        Self {
            buffer: plugin_buffer::PlanarBuffer::new(channels, BLOCK),
            channels,
            context: Context::new(rate),
            tone: super::TEST_FREQUENCY
                .get()
                .map(|&hz| TestTone::new(hz, rate)),
            plugin,
        }
    }
    pub fn run<T: cpal::Sample>(&mut self, interleaved: &[T], mut output: impl FnMut(f32))
    where
        f32: cpal::FromSample<T>,
    {
        // The manager locks only after both old streams have been dropped.
        // A missed lock produces silence instead of ever blocking a callback.
        let Ok(mut plugin) = self.plugin.try_lock() else {
            return;
        };
        for block in interleaved.chunks(BLOCK * self.channels) {
            let frames = block.len() / self.channels;
            self.buffer.with_buffer(frames, |buffer| {
                for (frame, input) in block.chunks_exact(self.channels).enumerate() {
                    let tone = self.tone.as_mut().map(TestTone::next);
                    for (channel, planar) in buffer.as_slice().iter_mut().enumerate() {
                        planar[frame] = tone.unwrap_or_else(|| f32::from_sample(input[channel]));
                    }
                }
                let mut auxiliary = AuxiliaryBuffers {
                    inputs: &mut [],
                    outputs: &mut [],
                };
                plugin.process(buffer, &mut auxiliary, &mut self.context);
                let selected = plugin.shared.input_channel.load().saturating_sub(1) as usize;
                if let Some(mono) = buffer.as_slice_immutable().get(selected) {
                    for &sample in mono.iter() {
                        output(sample);
                    }
                } else {
                    for _ in 0..frames {
                        output(0.0);
                    }
                }
            });
        }
        plugin
            .shared
            .buffer_frames
            .store((interleaved.len() / self.channels) as u32);
    }
}

struct TestTone {
    phase: f64,
    step: f64,
}
impl TestTone {
    fn new(hz: f64, rate: f32) -> Self {
        Self {
            phase: 0.0,
            step: hz / f64::from(rate),
        }
    }
    fn next(&mut self) -> f32 {
        let sample = (std::f64::consts::TAU * self.phase).sin() as f32;
        self.phase = (self.phase + self.step).fract();
        sample
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_tone_overrides_all_channels_and_keeps_phase_between_callbacks() {
        let mut plugin = TunerPlugin::default();
        activate(&mut plugin, 2, 48000.0).unwrap();
        plugin.shared.input_channel.store(2);
        let mut adapter = Processor::new(Arc::new(Mutex::new(plugin)), 2, 48000.0);
        adapter.tone = Some(TestTone::new(1000.0, 48000.0));
        let mut output = Vec::new();
        for frames in [13, 1050, 41] {
            adapter.run(&vec![0.37f32; frames * 2], |sample| output.push(sample));
        }
        for (i, sample) in output.iter().enumerate() {
            let expected = (std::f64::consts::TAU * i as f64 / 48.0).sin() as f32;
            assert!((sample - expected).abs() < 1e-5);
        }
        assert!((output[12] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn standalone_runs_the_plugin_and_preserves_the_selected_channel_at_input_rate() {
        let mut plugin = TunerPlugin::default();
        activate(&mut plugin, 2, 44100.0).unwrap();
        let shared = Arc::clone(&plugin.shared);
        shared
            .open
            .store(true, std::sync::atomic::Ordering::Release);
        shared.input_channel.store(2);
        let plugin = Arc::new(Mutex::new(plugin));
        let mut adapter = Processor::new(plugin, 2, 44100.0);
        let mut output = Vec::new();
        adapter.run(&[0.1f32, 0.25, 0.2, -0.5, 0.3, 0.75], |sample| {
            output.push(sample)
        });
        assert_eq!(output, [0.25, -0.5, 0.75]);
        assert_eq!(shared.sample_rate_hz.load(), 44100.0);
        assert_eq!(shared.buffer_frames.load(), 3);
        assert_eq!(shared.input_peak.load(), 0.75);
        // Analysis comes from that same Plugin::process call.
        let mut analysis = shared.take_consumer().unwrap();
        assert_eq!(analysis.pop().unwrap(), 0.25);
        assert_eq!(analysis.pop().unwrap(), -0.5);
        assert_eq!(analysis.pop().unwrap(), 0.75);
        shared.input_channel.store(1);
        output.clear();
        adapter.run(&[0.4f32, 0.9], |sample| output.push(sample));
        assert_eq!(output, [0.4]);
    }
}
