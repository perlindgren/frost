//! Adapter for the same Plugin activation and process calls used by VST3/CLAP.
use crate::TunerPlugin;
use nice_common::sample::Level;
use nice_plug::prelude::*;
use std::sync::{Arc, Mutex};

pub(super) const BLOCK: usize = 1024;

type Context = nice_plugin::standalone::Context<TunerPlugin>;

pub(super) fn activate(plugin: &mut TunerPlugin, rate: f32) -> Result<(), String> {
    if let Some(&hz) = super::TEST_FREQUENCY.get()
        && hz >= f64::from(rate) / 2.0
    {
        return Err(format!(
            "Test frequency {hz} Hz must be below half the input sample rate ({rate} Hz)"
        ));
    }
    // The box declares exactly one layout: mono in, mono out. Which of the
    // device's inputs fills that single lane is routing, decided at the edge
    // in `Processor::run`, not a second layout.
    nice_plugin::lifecycle::activate(
        plugin,
        rate,
        BLOCK,
        &mut Context::new(rate),
        "The tuner could not activate this audio configuration",
    )
}

pub(super) struct Processor {
    buffer: plugin_buffer::PlanarBuffer<{ BLOCK }>,
    /// Inputs the device underneath actually delivers, used only to
    /// de-interleave its frames and pick one of them for the box.
    channels: usize,
    context: Context,
    tone: Option<TestTone>,
    plugin: Arc<Mutex<TunerPlugin>>,
}
impl Processor {
    pub fn new(plugin: Arc<Mutex<TunerPlugin>>, channels: usize, rate: f32) -> Self {
        Self {
            buffer: plugin_buffer::PlanarBuffer::new(1, BLOCK),
            channels,
            context: Context::new(rate),
            tone: super::TEST_FREQUENCY
                .get()
                .map(|&hz| TestTone::new(hz, rate)),
            plugin,
        }
    }
    /// `T` is the sample format the device underneath was built for — `f32` on
    /// the ASIO driver this crate drives through `azo`, whatever the device asked
    /// for elsewhere — and [`Level`] is the conversion this loop needs from it.
    pub fn run<T: Level>(&mut self, interleaved: &[T], mut output: impl FnMut(f32)) {
        // The manager locks only after both old streams have been dropped.
        // A missed lock produces silence instead of ever blocking a callback.
        let Ok(mut plugin) = self.plugin.try_lock() else {
            return;
        };
        for block in interleaved.chunks(BLOCK * self.channels) {
            let frames = block.len() / self.channels;
            self.buffer.with_buffer(frames, |buffer| {
                // The layout asks for exactly one channel, and the device
                // hands over `channels` of them: the choice happens here, at
                // the device's edge, and the box never sees two of anything.
                let selected = (plugin.shared.input_channel.load().saturating_sub(1) as usize)
                    .min(self.channels.saturating_sub(1));
                let lane = buffer
                    .as_slice()
                    .first_mut()
                    .expect("the layout asks for one channel");
                for (frame, input) in block.chunks_exact(self.channels).enumerate() {
                    let tone = self.tone.as_mut().map(TestTone::next);
                    lane[frame] = tone.unwrap_or_else(|| input[selected].level());
                }
                let mut auxiliary = AuxiliaryBuffers {
                    inputs: &mut [],
                    outputs: &mut [],
                };
                plugin.process(buffer, &mut auxiliary, &mut self.context);
                // Back out at the other edge: the one mono answer, which the
                // caller routes onto the device lane it asked for.
                let answered = buffer.as_slice_immutable();
                for &sample in &**answered.first().expect("the layout asks for one channel") {
                    output(sample);
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
    fn analysis_handoff_survives_48_frame_blocks_and_size_changes() {
        let mut plugin = TunerPlugin::default();
        activate(&mut plugin, 48000.0).unwrap();
        let shared = Arc::clone(&plugin.shared);
        shared
            .open
            .store(true, std::sync::atomic::Ordering::Release);
        let mut consumer = shared.take_consumer().unwrap();
        let mut adapter = Processor::new(Arc::new(Mutex::new(plugin)), 2, 48000.0);
        // Feed the level once before measuring it. A constant that arrives at
        // an empty true-peak estimator is an onset and rings above itself for
        // that first block; a settled signal then reads exactly (see
        // `nice_common::true_peak`). The warm-up block is accounted for in the
        // counters below and drained from the ring.
        adapter.run(&vec![0.375f32; 64 * 2], |_| {});
        while consumer.pop().is_ok() {}
        let base_calls = shared
            .process_calls
            .load(std::sync::atomic::Ordering::Relaxed);
        let mut samples = shared
            .feed_samples
            .load(std::sync::atomic::Ordering::Relaxed);
        for (call, frames) in [48, 32, 48, 96, 256, 48, 1024, 48].into_iter().enumerate() {
            adapter.run(&vec![0.375f32; frames * 2], |_| {});
            samples += frames as u64;
            assert_eq!(
                shared
                    .process_calls
                    .load(std::sync::atomic::Ordering::Relaxed),
                base_calls + call as u64 + 1
            );
            assert_eq!(
                shared
                    .feed_samples
                    .load(std::sync::atomic::Ordering::Relaxed),
                samples
            );
            assert_eq!(
                shared
                    .dropped_samples
                    .load(std::sync::atomic::Ordering::Relaxed),
                0
            );
            assert_eq!(shared.buffer_frames.load(), frames as u32);
            assert_eq!(shared.input_peak.load(), 0.375);
            assert_eq!(consumer.slots(), frames);
            for _ in 0..frames {
                assert_eq!(consumer.pop().unwrap(), 0.375);
            }
        }
    }

    #[test]
    fn test_tone_overrides_the_device_input_and_keeps_phase_between_callbacks() {
        let mut plugin = TunerPlugin::default();
        activate(&mut plugin, 48000.0).unwrap();
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
    fn activation_publishes_the_mono_operating_conditions() {
        let mut plugin = TunerPlugin::default();
        let rate = 48000.0;
        activate(&mut plugin, rate).unwrap();
        // One layout, one channel each side, whatever the device underneath
        // happens to be carrying.
        assert_eq!(plugin.shared.input_channels.load(), 1);
        assert_eq!(plugin.shared.output_channels.load(), 1);
        assert_eq!(plugin.shared.sample_rate_hz.load(), rate as f64);
        assert_eq!(plugin.shared.max_buffer_frames.load(), BLOCK as u32);
        assert_eq!(plugin.shared.buffer_frames.load(), 0);
    }

    #[test]
    fn standalone_takes_the_selected_device_channel_as_the_box_input() {
        let mut plugin = TunerPlugin::default();
        activate(&mut plugin, 44100.0).unwrap();
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
