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
}

impl Context {
    pub fn new(rate: f32) -> Self {
        Self {
            transport: Transport::new(rate),
        }
    }
}

// No MIDI, background tasks, latency, or voices in this scaffold.
impl ActivateContext<ClonePlugin> for Context {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Standalone
    }
    fn execute(&self, _: ()) {}
    fn set_latency_samples(&self, _: u32) {}
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
    fn set_latency_samples(&self, _: u32) {}
    fn request_restart(&self) {}
    fn set_current_voice_capacity(&self, _: u32) {}
}

/// (Re)activate for a freshly opened pair of streams. The transport hands the
/// pump stereo in and out whatever the device layout, so the two-channel
/// layout is the only one this host ever spawns, exactly as a DAW's stereo
/// insert would run it.
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
    buffer: &mut plugin_buffer::PlanarBuffer<{ 2 * BLOCK }>,
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
        // The layout asks for exactly two channels, and the ring hands
        // over exactly a frame: split the buffer into its two halves and
        // fill both from one pop, so neither channel's index can drift.
        let channels = buffer.as_slice();
        let (left, rest) = channels
            .split_first_mut()
            .expect("the layout asks for two channels");
        let right = rest.first_mut().expect("the layout asks for two channels");
        for (left_slot, right_slot) in left[..count].iter_mut().zip(&mut right[..count]) {
            let [left, right] = io.input.pop().expect("slots() promised a frame");
            *left_slot = left;
            *right_slot = right;
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
            let [left, right] = [
                buffer.as_slice_immutable()[0][frame],
                buffer.as_slice_immutable()[1][frame],
            ];
            let _ = io.output.push(StereoFrame([left, right]));
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
    let mut buffer = plugin_buffer::PlanarBuffer::<{ 2 * BLOCK }>::new(2, BLOCK);
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
    fn activation_and_one_pump_move_audio_straight_through_unchanged() {
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
        let expected = [[0.5, -0.25], [-0.75, 0.125], [0.0, 0.9], [0.2, -0.4]];
        for frame in expected {
            capture.push(frame).unwrap();
        }
        let mut buffer = plugin_buffer::PlanarBuffer::<{ 2 * BLOCK }>::new(2, BLOCK);
        let mut context = Context::new(48_000.0);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            4
        );
        assert_eq!(drain.slots(), 4);
        for frame in expected {
            assert_eq!(drain.pop().unwrap().0, frame);
        }
        assert_eq!(shared.input_rms.load(), shared.output_rms.load());
        assert_eq!(shared.input_peak.load(), shared.output_peak.load());
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
        let mut buffer = plugin_buffer::PlanarBuffer::<{ 2 * BLOCK }>::new(2, BLOCK);
        let mut context = Context::new(44_100.0);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            0
        );
    }
}

#[cfg(test)]
mod scaffold_tests {
    use super::*;

    #[test]
    fn mono_and_stereo_pass_through_across_blocks() {
        for channels in [1, 2] {
            let mut plugin = ClonePlugin::default();
            activate(&mut plugin, 48_000.0).unwrap();
            let mut context = Context::new(48_000.0);
            let mut planar = plugin_buffer::PlanarBuffer::<{ 6 * BLOCK }>::new(channels, 3 * BLOCK);
            for frames in [0, 1, BLOCK - 1, BLOCK, BLOCK + 1, 2 * BLOCK + 7] {
                planar.with_buffer(frames, |buffer| {
                    for (ch, samples) in buffer.as_slice().iter_mut().enumerate() {
                        for (i, sample) in samples[..frames].iter_mut().enumerate() {
                            *sample = ((i * 13 + ch * 29) % 101) as f32 / 100.0 - 0.5;
                        }
                    }
                    {
                        plugin.process(
                            buffer,
                            &mut AuxiliaryBuffers {
                                inputs: &mut [],
                                outputs: &mut [],
                            },
                            &mut context,
                        );
                    }
                    for (ch, samples) in buffer.as_slice_immutable().iter().enumerate() {
                        for (i, &sample) in samples[..frames].iter().enumerate() {
                            assert_eq!(sample, ((i * 13 + ch * 29) % 101) as f32 / 100.0 - 0.5);
                        }
                    }
                });
            }
        }
    }
}
