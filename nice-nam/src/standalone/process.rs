//! Adapter for the same Plugin activation and process calls used by VST3/CLAP.
use crate::NamPlugin;
use nice_common::sample::Level;
use nice_plug::prelude::*;
use std::sync::{Arc, Mutex};

pub(super) const BLOCK: usize = 1024;

type Context = nice_plugin::standalone::Context<NamPlugin>;

pub(super) fn activate(plugin: &mut NamPlugin, _channels: usize, rate: f32) -> Result<(), String> {
    nice_plugin::lifecycle::activate(
        plugin,
        rate,
        BLOCK,
        &mut Context::new(rate),
        "NAM could not activate",
    )
}

pub(super) struct Processor {
    buffer: plugin_buffer::PlanarBuffer<{ 2 * BLOCK }>,
    channels: usize,
    context: Context,
    plugin: Arc<Mutex<NamPlugin>>,
}
impl Processor {
    pub fn new(plugin: Arc<Mutex<NamPlugin>>, channels: usize, rate: f32) -> Self {
        Self {
            buffer: plugin_buffer::PlanarBuffer::new(2, BLOCK),
            channels,
            context: Context::new(rate),
            plugin,
        }
    }
    /// `T` is the sample format the device underneath was built for — `f32` on
    /// the ASIO driver this crate drives through `azo`, whatever the device asked
    /// for elsewhere — and [`Level`] is the conversion this loop needs from it.
    pub fn run<T: Level>(&mut self, input: &[T], mut output: impl FnMut(f32)) {
        let Ok(mut plugin) = self.plugin.try_lock() else {
            return;
        };
        let selected = plugin.shared.input_channel.load().saturating_sub(1) as usize;
        for block in input.chunks(BLOCK * self.channels) {
            let frames = block.len() / self.channels;
            self.buffer.with_buffer(frames, |buffer| {
                for (frame, input) in block.chunks_exact(self.channels).enumerate() {
                    let mono = input.get(selected).map_or(0.0, |s| s.level());
                    buffer.as_slice()[0][frame] = mono;
                }
                plugin.process(
                    buffer,
                    &mut AuxiliaryBuffers {
                        inputs: &mut [],
                        outputs: &mut [],
                    },
                    &mut self.context,
                );
                for &sample in buffer.as_slice_immutable()[0].iter() {
                    output(sample);
                }
            });
        }
        plugin.shared.buffer_frames.store(
            (input.len() / self.channels) as u32,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_selects_mono_before_the_shared_plugin_processes_it() {
        let mut plugin = NamPlugin::default();
        activate(&mut plugin, 2, 48000.0).unwrap();
        plugin.shared.input_channel.store(2);
        let mut processor = Processor::new(Arc::new(Mutex::new(plugin)), 2, 48000.0);
        let mut result = Vec::new();
        processor.run(&[0.25f32, -0.5, 0.75, 0.125], |sample| result.push(sample));
        assert_eq!(result, [-0.5, 0.125]);
    }
}
