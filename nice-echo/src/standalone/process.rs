//! Plugin-specific regression tests for the shared standalone adapter.
use crate::{EchoPlugin, EchoSharedState};
use nice_common::audio_io::Endpoints;
use std::sync::{Arc, Mutex, atomic::Ordering};
const BLOCK: usize = crate::dsp::BLOCK;
type Context = nice_plugin::standalone::Context<EchoPlugin>;
fn activate(plugin: &mut EchoPlugin, rate: f32) -> Result<(), String> {
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
    plugin: &Arc<Mutex<EchoPlugin>>,
    shared: &EchoSharedState,
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
    fn activation_and_one_pump_move_audio_through_the_echo() {
        // The standalone's whole promise in one test: the same plugin the CLAP
        // entry point builds runs on this transport, frames in one ring and
        // out the other, meters published, nothing allocated. The frames come
        // back coloured rather than as they went in — the tape path is
        // `dsp.rs`'s subject and its behaviour is argued there — so what the
        // plumbing is held to here is its own share: the count, and audio
        // arriving at the far end.
        let plugin = Arc::new(Mutex::new(EchoPlugin::default()));
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
        // The same ramp on both device lanes: the host folds the captured
        // frame down to the box's one input, so an anti-phase pair here
        // would cancel at that fold and only hide the audio this test is
        // looking for.
        for index in 0..4 {
            capture
                .push([0.25 * index as f32, 0.25 * index as f32])
                .unwrap();
        }
        let mut buffer = plugin_buffer::PlanarBuffer::<{ BLOCK }>::new(1, BLOCK);
        let mut context = Context::new(48_000.0);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            4
        );
        assert_eq!(drain.slots(), 4);
        let mut loudest = 0.0f32;
        while let Ok(frame) = drain.pop() {
            let [left, right] = frame.0;
            for sample in frame.0 {
                assert!(sample.is_finite(), "the echo handed back {sample}");
                loudest = loudest.max(sample.abs());
            }
            assert_eq!(
                left, right,
                "the box answers once and the host duplicates it"
            );
        }
        // A ramp up to three quarters of full scale, the dry path is a
        // passthrough at this setting, so anything this quiet means the trip
        // through the two rings lost the signal rather than coloured it.
        assert!(loudest > 0.1, "the standalone muted the audio: {loudest}");
        assert_eq!(
            shared.buffer_frames.load(Ordering::Relaxed),
            4,
            "the panel should be able to read the running block size"
        );
        // The meters are the audio thread's, even with no host to run it.
        assert_eq!(shared.input_peak.load(), 0.75);
    }

    #[test]
    fn an_empty_transport_pumps_nothing() {
        let plugin = Arc::new(Mutex::new(EchoPlugin::default()));
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
