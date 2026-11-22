//! Plugin-specific regression tests for the shared standalone adapter.
use crate::{ClonePlugin, CloneSharedState};
use nice_common::audio_io::Endpoints;
use nice_plug::prelude::*;
use std::sync::{Arc, Mutex, atomic::Ordering};
const BLOCK: usize = crate::BLOCK;
type Context = nice_plugin::standalone::Context<ClonePlugin>;
fn activate(plugin: &mut ClonePlugin, rate: f32) -> Result<(), String> {
    nice_plugin::lifecycle::activate(
        plugin,
        rate,
        BLOCK,
        &mut Context::new(rate),
        "activation failed",
    )
}
fn pump_once(
    buffer: &mut plugin_buffer::PlanarBuffer<BLOCK>,
    context: &mut Context,
    plugin: &Arc<Mutex<ClonePlugin>>,
    shared: &CloneSharedState,
    io: &mut Endpoints,
) -> usize {
    let count = nice_plugin::standalone::process_once(buffer, context, plugin, io, BLOCK);
    if count > 0 {
        shared.buffer_frames.store(count as u32, Ordering::Relaxed);
    }
    count
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_and_one_pump_run_the_circuit() {
        // The standalone's whole promise in one test: the same plugin the
        // CLAP entry point builds runs on this transport, frames in one ring
        // and out the other, meters published, nothing allocated.
        let plugin = Arc::new(Mutex::new(ClonePlugin::default()));
        {
            let mut instance = plugin.lock().unwrap();
            activate(&mut instance, 48_000.0).unwrap();
        }
        let shared = Arc::clone(&plugin.lock().unwrap().shared);
        let (mut capture, input) = rtrb::RingBuffer::new(64);
        let (playback, mut drain) = rtrb::RingBuffer::new(64);
        let mut io = Endpoints {
            input,
            output: playback,
        };
        // A real stereo pair off the device: the pump folds it down at the
        // host's edge, so the answer below is the drive run on that fold,
        // and the same answer handed back on both lanes.
        let expected = [[0.5, -0.25], [-0.75, 0.125], [0.0, 0.9], [0.2, -0.4]];
        for frame in expected {
            capture.push(frame).unwrap();
        }
        let mut buffer = plugin_buffer::PlanarBuffer::<{ BLOCK }>::new(1, BLOCK);
        let mut context = Context::new(48_000.0);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            4
        );
        assert_eq!(drain.slots(), 4);
        let mut reference = crate::dsp::CloneDrive::new(48_000.0, &crate::dsp::Controls::default());
        for frame in expected {
            let folded = (frame[0] + frame[1]) * 0.5;
            let answer = reference.process(folded);
            assert_eq!(
                drain.pop().unwrap().0,
                [answer, answer],
                "the box answers once and the host duplicates it"
            );
        }
        assert!(shared.input_peak.load() > 0.0);
        assert!(shared.output_peak.load().is_finite());
        assert_eq!(
            shared.buffer_frames.load(Ordering::Relaxed),
            4,
            "the panel should be able to read the running block size"
        );
    }

    #[test]
    fn an_empty_transport_pumps_nothing() {
        let plugin = Arc::new(Mutex::new(ClonePlugin::default()));
        let shared = Arc::clone(&plugin.lock().unwrap().shared);
        let (_, input) = rtrb::RingBuffer::new(16);
        let (playback, _drain) = rtrb::RingBuffer::new(16);
        let mut io = Endpoints {
            input,
            output: playback,
        };
        let mut buffer = plugin_buffer::PlanarBuffer::<{ BLOCK }>::new(1, BLOCK);
        let mut context = Context::new(44_100.0);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            0
        );
    }
}

#[cfg(test)]
mod processing_tests {
    use super::*;

    #[test]
    fn mono_audio_matches_the_circuit_across_host_block_boundaries() {
        let mut plugin = ClonePlugin::default();
        activate(&mut plugin, 48_000.0).unwrap();
        let mut reference = crate::dsp::CloneDrive::new(48_000.0, &crate::dsp::Controls::default());
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

#[cfg(test)]
mod oversampling_host_tests {
    use super::*;
    use crate::dsp::Oversampling;

    fn select(plugin: &mut ClonePlugin, mode: Oversampling) {
        plugin.params = Arc::new(crate::CloneParams {
            oversampling: IntParam::new(
                "Oversampling",
                mode.index(),
                IntRange::Linear { min: 0, max: 3 },
            ),
            ..Default::default()
        });
    }
    fn block(
        plugin: &mut ClonePlugin,
        buffer: &mut plugin_buffer::PlanarBuffer<64>,
        context: &mut Context,
    ) {
        buffer.with_buffer(32, |buffer| {
            plugin.process(
                buffer,
                &mut AuxiliaryBuffers {
                    inputs: &mut [],
                    outputs: &mut [],
                },
                context,
            );
        });
    }

    #[test]
    fn host_gets_latency_updates_when_the_active_rate_changes_including_reset() {
        let mut plugin = ClonePlugin::default();
        let mut context = Context::new(48_000.0);
        assert!(plugin.activate(
            &ClonePlugin::AUDIO_IO_LAYOUTS[0],
            &BufferConfig {
                sample_rate: 48_000.0,
                min_buffer_size: None,
                max_buffer_size: 32,
                process_mode: ProcessMode::Realtime,
            },
            &mut context
        ));
        assert_eq!(context.latency_samples(), 32);
        let mut buffer = plugin_buffer::PlanarBuffer::<64>::new(1, 32);
        for mode in [
            Oversampling::Off,
            Oversampling::X2,
            Oversampling::X4,
            Oversampling::X8,
        ] {
            let previous_latency = plugin.drive.latency();
            let reports = context.latency_reports();
            select(&mut plugin, mode);
            block(&mut plugin, &mut buffer, &mut context);
            assert_eq!(
                context.latency_reports(),
                reports,
                "switch has not finished fading down"
            );
            for _ in 0..3 {
                block(&mut plugin, &mut buffer, &mut context);
            }
            assert_eq!(plugin.drive.oversampling(), mode);
            assert_eq!(context.latency_samples(), mode.latency());
            assert_eq!(
                context.latency_reports(),
                reports + usize::from(previous_latency != mode.latency())
            );
        }
        select(&mut plugin, Oversampling::Off);
        block(&mut plugin, &mut buffer, &mut context);
        plugin.reset();
        block(&mut plugin, &mut buffer, &mut context);
        assert_eq!(
            context.latency_samples(),
            0,
            "a reset during a pending switch still needs a latency notification"
        );
    }
}
