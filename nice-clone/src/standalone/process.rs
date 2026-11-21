//! The plugin adapter between nice-common's transport rings and the plugin's
//! own `process()`. This is the entire standalone DSP: the device callbacks,
//! both backends, the resampling monitor and the ring sizing all live in
//! `nice_common::audio_io` — the rack runs the same transport — and what the
//! rack calls `spawn_dsp` is what `pump` is here.
use crate::{ClonePlugin, CloneSharedState};
use nice_common::audio_io::Endpoints;
use nice_common::resample::StereoFrame;
use nice_plug::context::process::SendEventError;
use nice_plug::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The same quantum the plugin's own scratch buffers are sized for, so one
/// block of it is always one call of the plugin.
pub(super) const BLOCK: usize = crate::BLOCK;

pub(super) struct Context {
    transport: Transport,
    #[cfg(test)]
    latency: std::cell::Cell<u32>,
    #[cfg(test)]
    latency_reports: std::cell::Cell<usize>,
}

impl Context {
    pub fn new(rate: f32) -> Self {
        Self {
            transport: Transport::new(rate),
            #[cfg(test)]
            latency: std::cell::Cell::new(0),
            #[cfg(test)]
            latency_reports: std::cell::Cell::new(0),
        }
    }
}

// No MIDI, background tasks, or voices. Standalone needs no host latency compensation.
impl ActivateContext<ClonePlugin> for Context {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }
    fn execute(&self, _: ()) {}
    fn set_latency_samples(&self, samples: u32) {
        #[cfg(test)]
        {
            self.latency.set(samples);
            self.latency_reports.set(self.latency_reports.get() + 1);
        }
        #[cfg(not(test))]
        let _ = samples;
    }
    fn set_current_voice_capacity(&self, _: u32) {}
}

impl ProcessContext<ClonePlugin> for Context {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }
    fn execute_background(&self, _: ()) {}
    fn execute_gui(&self, _: ()) {}
    fn transport(&self) -> &Transport {
        &self.transport
    }
    fn next_event(&mut self) -> Option<PluginNoteEvent<ClonePlugin>> {
        None
    }
    fn try_send_event(
        &mut self,
        event: PluginNoteEvent<ClonePlugin>,
    ) -> Result<(), (PluginNoteEvent<ClonePlugin>, SendEventError)> {
        Err((event, SendEventError::NoOutputBuffer))
    }
    fn set_latency_samples(&self, samples: u32) {
        #[cfg(test)]
        {
            self.latency.set(samples);
            self.latency_reports.set(self.latency_reports.get() + 1);
        }
        #[cfg(not(test))]
        let _ = samples;
    }
    fn request_restart(&self) {}
    fn set_current_voice_capacity(&self, _: u32) {}
}

/// (Re)activate for a freshly opened pair of streams, on the plugin's own mono
/// layout. The device contract in `nice_common::audio_io` is stereo — the rings
/// carry stereo frames whatever the device layout — so the fold from a captured
/// frame down to the box's one input, and the widening of its one output back
/// to the device's two lanes, happen in `pump_once` below, at the host's edges.
/// Width is the host's work; the box has one of each.
pub(super) fn activate(plugin: &mut ClonePlugin, rate: f32) -> Result<(), String> {
    plugin.deactivate();
    if !plugin.activate(
        &ClonePlugin::AUDIO_IO_LAYOUTS[0],
        &BufferConfig {
            sample_rate: rate,
            min_buffer_size: None,
            max_buffer_size: BLOCK as u32,
            process_mode: ProcessMode::Realtime,
        },
        &mut Context::new(rate),
    ) {
        return Err("Nice Clone could not activate".into());
    }
    Ok(())
}

/// One drain of the rings, bounded by the plugin's block: pop up to a block
/// of captured frames, run the plugin over them, push the result to the
/// monitor. Zero means the capture had nothing to say this moment.
///
/// A locked plugin answers with silence rather than stale input: the lock is
/// the audio owner performing a `deactivate`/`activate` pair for new streams,
/// and passing a device's live signal through a plugin mid-retuning is worse
/// than one short gap on a reconfiguration the user asked for.
fn pump_once(
    buffer: &mut plugin_buffer::PlanarBuffer<{ BLOCK }>,
    context: &mut Context,
    plugin: &Arc<Mutex<ClonePlugin>>,
    shared: &CloneSharedState,
    io: &mut Endpoints,
) -> usize {
    let count = io.input.slots().min(BLOCK);
    if count == 0 {
        return 0;
    }
    buffer.with_buffer(count, |buffer| {
        // The layout asks for exactly one channel, and the ring hands over a
        // stereo frame: the fold happens here, at the host's edge, and the
        // box never sees two of anything.
        let channels = buffer.as_slice();
        let mono = channels
            .first_mut()
            .expect("the layout asks for one channel");
        for slot in mono[..count].iter_mut() {
            let [left, right] = io.input.pop().expect("slots() promised a frame");
            *slot = (left + right) * 0.5;
        }
        match plugin.try_lock() {
            Ok(mut instance) => {
                let _status = instance.process(
                    buffer,
                    &mut AuxiliaryBuffers {
                        inputs: &mut [],
                        outputs: &mut [],
                    },
                    context,
                );
            }
            Err(_) => {
                for channel in buffer.as_slice() {
                    channel[..count].fill(0.0);
                }
            }
        }
        for frame in 0..count {
            // Back out at the other edge: the one mono answer, duplicated so
            // both device lanes carry the clone.
            let sample = buffer.as_slice_immutable()[0][frame];
            let _ = io.output.push(StereoFrame([sample, sample]));
        }
    });
    shared.buffer_frames.store(count as u32, Ordering::Relaxed);
    count
}

/// The pump thread: the rack's `spawn_dsp` shape, parked between drains so
/// an idle transport costs nothing and a running one nothing extra. The
/// plugin's own `process()` publishes the meters, so the editor sees a
/// standalone exactly as a host would see it.
pub(super) fn pump(
    plugin: Arc<Mutex<ClonePlugin>>,
    shared: Arc<CloneSharedState>,
    mut io: Endpoints,
    stop: Arc<AtomicBool>,
    rate: f32,
) {
    let mut buffer = plugin_buffer::PlanarBuffer::<{ BLOCK }>::new(1, BLOCK);
    let mut context = Context::new(rate);
    while !stop.load(Ordering::Relaxed) {
        if pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io) == 0 {
            std::thread::park_timeout(Duration::from_micros(250));
        }
    }
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
        assert_eq!(context.latency.get(), 32);
        let mut buffer = plugin_buffer::PlanarBuffer::<64>::new(1, 32);
        for mode in [
            Oversampling::Off,
            Oversampling::X2,
            Oversampling::X4,
            Oversampling::X8,
        ] {
            let previous_latency = plugin.drive.latency();
            let reports = context.latency_reports.get();
            select(&mut plugin, mode);
            block(&mut plugin, &mut buffer, &mut context);
            assert_eq!(
                context.latency_reports.get(),
                reports,
                "switch has not finished fading down"
            );
            for _ in 0..3 {
                block(&mut plugin, &mut buffer, &mut context);
            }
            assert_eq!(plugin.drive.oversampling(), mode);
            assert_eq!(context.latency.get(), mode.latency());
            assert_eq!(
                context.latency_reports.get(),
                reports + usize::from(previous_latency != mode.latency())
            );
        }
        select(&mut plugin, Oversampling::Off);
        block(&mut plugin, &mut buffer, &mut context);
        plugin.reset();
        block(&mut plugin, &mut buffer, &mut context);
        assert_eq!(
            context.latency.get(),
            0,
            "a reset during a pending switch still needs a latency notification"
        );
    }
}
