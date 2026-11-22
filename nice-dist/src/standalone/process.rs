//! Plugin-specific regression tests for the shared standalone adapter.
use crate::DistPlugin;
use nice_plug::prelude::*;
const BLOCK: usize = crate::BLOCK;
type Context = nice_plugin::standalone::Context<DistPlugin>;
fn activate(plugin: &mut DistPlugin, rate: f32) -> Result<(), String> {
    nice_plugin::lifecycle::activate(
        plugin,
        rate,
        BLOCK,
        &mut Context::new(rate),
        "activation failed",
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mono_audio_matches_the_dsp_across_host_block_boundaries() {
        let mut plugin = DistPlugin::default();
        activate(&mut plugin, 48_000.0).unwrap();
        let mut reference = crate::dsp::Dist::new(
            48_000.0,
            &crate::dsp::Controls::default(),
            crate::controls::Variant::Ts808,
            nice_common::oversample::Oversampling::X8,
        );
        let mut context = Context::new(48_000.0);
        let mut planar = plugin_buffer::PlanarBuffer::<{ 6 * BLOCK }>::new(1, 3 * BLOCK);
        for frames in [0, 1, BLOCK - 1, BLOCK, BLOCK + 1, 2 * BLOCK + 7] {
            planar.with_buffer(frames, |buffer| {
                for samples in buffer.as_slice() {
                    for (i, sample) in samples[..frames].iter_mut().enumerate() {
                        *sample = ((i * 13) % 101) as f32 / 100.0 - 0.5;
                    }
                }
                plugin.process(
                    buffer,
                    &mut AuxiliaryBuffers {
                        inputs: &mut [],
                        outputs: &mut [],
                    },
                    &mut context,
                );
                for i in 0..frames {
                    let sample = ((i * 13) % 101) as f32 / 100.0 - 0.5;
                    assert_eq!(buffer.as_slice_immutable()[0][i], reference.process(sample));
                }
            });
        }
    }
}
