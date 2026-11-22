//! Plugin-specific regression tests for the shared standalone adapter.
use crate::{PiFacePlugin, PiFaceSharedState};
use nice_common::audio_io::Endpoints;
use std::sync::{Arc, Mutex, atomic::Ordering};
const BLOCK: usize = crate::dsp::BLOCK;
type Context = nice_plugin::standalone::Context<PiFacePlugin>;
fn activate(plugin: &mut PiFacePlugin, rate: f32) -> Result<(), String> {
    nice_plugin::lifecycle::activate(
        plugin,
        rate,
        BLOCK,
        &mut Context::new(rate),
        "activation failed",
    )
}
fn pump_once(
    buffer: &mut plugin_buffer::PlanarBuffer<{ 2 * BLOCK }>,
    context: &mut Context,
    plugin: &Arc<Mutex<PiFacePlugin>>,
    shared: &PiFaceSharedState,
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
    fn activation_and_one_pump_move_audio_through_the_filter() {
        // The standalone's whole promise in one test: the same plugin the CLAP
        // entry point builds runs on this transport, frames in one ring and out
        // the other, meters published, nothing allocated. The frames come back
        // filtered rather than as they went in — the filter is `dsp.rs`'s subject
        // and its closed forms are argued there — so what the plumbing is held to
        // here is its own share: the count, and audio arriving at the far end.
        let plugin = Arc::new(Mutex::new(PiFacePlugin::default()));
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
        // The same ramp on both channels. In the machine's single-input modes the
        // two jacks are summed, so an anti-phase pair would cancel to silence
        // before it reached a phasor — `dsp.rs` tests that as a routing behaviour,
        // and here it would only hide the audio this test is looking for.
        for index in 0..4 {
            capture
                .push([0.25 * index as f32, 0.25 * index as f32])
                .unwrap();
        }
        let mut buffer = plugin_buffer::PlanarBuffer::<{ 2 * BLOCK }>::new(2, BLOCK);
        let mut context = Context::new(48_000.0);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            4
        );
        assert_eq!(drain.slots(), 4);
        let mut loudest = 0.0f32;
        while let Ok(frame) = drain.pop() {
            for sample in frame.0 {
                assert!(sample.is_finite(), "the filter handed back {sample}");
                loudest = loudest.max(sample.abs());
            }
        }
        // A ramp up to three quarters of full scale, and the phasors' insertion
        // loss is under 1 dB, so anything this quiet means the trip through the
        // two rings lost the signal rather than coloured it.
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
        let plugin = Arc::new(Mutex::new(PiFacePlugin::default()));
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
