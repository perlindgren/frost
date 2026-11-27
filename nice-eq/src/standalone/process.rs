//! Plugin-specific regression tests for the shared standalone adapter.
use crate::{EqPlugin, EqSharedState};
use nice_common::{audio_io::Endpoints, resample::StereoFrame};
use std::sync::{Arc, Mutex, atomic::Ordering};
const BLOCK: usize = crate::dsp::BLOCK;
type Context = nice_plugin::standalone::Context<EqPlugin>;
fn activate(plugin: &mut EqPlugin, rate: f32) -> Result<(), String> {
    nice_plugin::lifecycle::activate(
        plugin,
        rate,
        BLOCK,
        &mut Context::new(rate),
        "activation failed",
    )
}
fn pump_once<const CAPACITY: usize>(
    buffer: &mut plugin_buffer::PlanarBuffer<CAPACITY>,
    context: &mut Context,
    plugin: &Arc<Mutex<EqPlugin>>,
    shared: &EqSharedState,
    io: &mut Endpoints,
) -> usize {
    let count = nice_plugin::standalone::process_once(buffer, context, plugin, io, BLOCK);
    if count > 0 {
        shared.buffer_frames.store(count as u32, Ordering::Relaxed);
    }
    count
}

/// A transport with room for `frames` frames and exactly that much audio in
/// it, at the levels the tests describe. Frames arrive as a driver hands them,
/// a pair per frame, and leave wrapped for the playback side.
fn transport(left: f32, right: f32, frames: usize) -> (Endpoints, rtrb::Consumer<StereoFrame>) {
    let (mut capture, input) = rtrb::RingBuffer::<[f32; 2]>::new(64);
    let (playback, drain) = rtrb::RingBuffer::<StereoFrame>::new(64);
    for _ in 0..frames {
        capture.push([left, right]).unwrap();
    }
    (
        Endpoints {
            input,
            output: playback,
        },
        drain,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_equaliser_hands_back_what_it_was_given_lane_for_lane() {
        // The standalone's whole promise in one test: the same plugin the CLAP
        // entry point builds runs on this transport, frames in one ring and out
        // the other, meters published, nothing allocated. And because a flat
        // cascade is exactly transparent — the claim `dsp`'s own test pins to
        // one part in ten thousand through all seven sections — what comes back
        // is what went in, on both lanes, never a sum of them.
        let plugin = Arc::new(Mutex::new(EqPlugin::default()));
        {
            let mut instance = plugin.lock().unwrap();
            activate(&mut instance, 48_000.0).unwrap();
        }
        let shared = Arc::clone(&plugin.lock().unwrap().shared);
        let (mut io, mut drain) = transport(0.5, -0.25, 4);
        let mut buffer = plugin_buffer::PlanarBuffer::<{ 2 * BLOCK }>::new(2, BLOCK);
        let mut context = Context::new(48_000.0);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            4
        );
        assert_eq!(drain.slots(), 4);
        for _ in 0..4 {
            let [left, right] = drain.pop().unwrap().0;
            assert!(left.is_finite() && right.is_finite());
            assert!((left - 0.5).abs() < 1e-6, "left lane read {left}");
            assert!((right + 0.25).abs() < 1e-6, "right lane read {right}");
        }
        assert_eq!(
            shared.buffer_frames.load(Ordering::Relaxed),
            4,
            "the panel should be able to read the running block size"
        );
        assert!(shared.meter_updated.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn the_feed_fills_only_while_an_editor_is_listening() {
        // The analyser's feed is the audio thread's only extra work, and two
        // things switch it off: no window, and a window that said `off`. This
        // covers the first, which is the one that runs on every machine that
        // never opens the editor at all.
        let plugin = Arc::new(Mutex::new(EqPlugin::default()));
        {
            let mut instance = plugin.lock().unwrap();
            activate(&mut instance, 48_000.0).unwrap();
        }
        let shared = Arc::clone(&plugin.lock().unwrap().shared);
        let (mut io, _drain) = transport(0.25, 0.25, 4);
        let mut buffer = plugin_buffer::PlanarBuffer::<{ 2 * BLOCK }>::new(2, BLOCK);
        let mut context = Context::new(48_000.0);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            4
        );
        assert_eq!(shared.feed_samples.load(Ordering::Relaxed), 0);
        let (mut feed_in, feed_out) = shared.take_feed().expect("a parked feed");
        assert_eq!(feed_in.slots(), 0, "a shut window was fed anyway");
        assert_eq!(feed_out.slots(), 0);

        // The same plugin, an editor now open: both taps are watched by
        // default, so both rings get the four samples, channel 0 of each end.
        shared.open.store(true, Ordering::Release);
        let (mut io, _drain) = transport(0.25, -0.25, 4);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            4
        );
        assert_eq!(feed_in.slots(), 4);
        assert_eq!(feed_out.slots(), 4);
        assert_eq!(shared.feed_samples.load(Ordering::Relaxed), 8);
        assert_eq!(shared.dropped_samples.load(Ordering::Relaxed), 0);
        while let Ok(sample) = feed_in.pop() {
            assert_eq!(sample, 0.25, "the input tap should carry channel 0");
        }
        shared.put_feed((feed_in, feed_out));
    }

    #[test]
    fn an_empty_transport_pumps_nothing() {
        let plugin = Arc::new(Mutex::new(EqPlugin::default()));
        let shared = Arc::clone(&plugin.lock().unwrap().shared);
        let (mut io, _drain) = transport(0.0, 0.0, 0);
        let mut buffer = plugin_buffer::PlanarBuffer::<{ 2 * BLOCK }>::new(2, BLOCK);
        let mut context = Context::new(44_100.0);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            0
        );
    }
}
