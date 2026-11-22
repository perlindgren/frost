//! Plugin-specific regression tests for the shared standalone adapter.
use crate::{WahPlugin, WahSharedState};
use nice_common::audio_io::Endpoints;
use std::sync::{Arc, Mutex, atomic::Ordering};
const BLOCK: usize = crate::dsp::BLOCK;
type Context = nice_plugin::standalone::Context<WahPlugin>;
fn activate(plugin: &mut WahPlugin, rate: f32) -> Result<(), String> {
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
    plugin: &Arc<Mutex<WahPlugin>>,
    shared: &WahSharedState,
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
    fn activation_and_one_pump_move_audio_straight_through_the_filter() {
        // The standalone's whole promise in one test: the same plugin the
        // CLAP entry point builds runs on this transport, frames in one ring
        // and out the other, meters published, nothing allocated.
        let plugin = Arc::new(Mutex::new(WahPlugin::default()));
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
        for _ in 0..4 {
            capture.push([0.5, 0.5]).unwrap();
        }
        let mut buffer = plugin_buffer::PlanarBuffer::<{ BLOCK }>::new(1, BLOCK);
        let mut context = Context::new(48_000.0);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            4
        );
        assert_eq!(drain.slots(), 4);
        while let Ok(frame) = drain.pop() {
            let [left, right] = frame.0;
            for sample in frame.0 {
                assert!(sample.is_finite(), "the filter handed back {sample}");
            }
            assert_eq!(
                left, right,
                "the pedal answers once and the host duplicates it"
            );
        }
        assert_eq!(
            shared.buffer_frames.load(Ordering::Relaxed),
            4,
            "the panel should be able to read the running block size"
        );
    }

    #[test]
    fn an_empty_transport_pumps_nothing() {
        let plugin = Arc::new(Mutex::new(WahPlugin::default()));
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
