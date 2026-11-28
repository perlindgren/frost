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
fn pump_once(
    buffer: &mut plugin_buffer::PlanarBuffer<BLOCK>,
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
/// it, at the levels the tests describe. A driver hands its capture over a pair
/// at a time whatever the plugin declared, so a pair is what this builds; what
/// a mono box hears of that pair is the adapter's doing, and one of the tests
/// below is the pin on it.
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

/// An instance with its rate established, which is the state a host leaves a
/// plugin in before the first block.
fn running() -> (Arc<Mutex<EqPlugin>>, Arc<EqSharedState>) {
    let plugin = Arc::new(Mutex::new(EqPlugin::default()));
    {
        let mut instance = plugin.lock().unwrap();
        activate(&mut instance, 48_000.0).unwrap();
    }
    let shared = Arc::clone(&plugin.lock().unwrap().shared);
    (plugin, shared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_equaliser_hands_back_what_it_was_given() {
        // The standalone's whole promise in one test: the same plugin the CLAP
        // entry point builds runs on this transport, frames in one ring and out
        // the other, meters published, nothing allocated. And because a flat
        // cascade is exactly transparent — the claim `dsp`'s own test pins to
        // one part in ten thousand through all seven sections — what comes back
        // is what went in, on the one lane the box has. The playback side gets
        // that lane twice only because a driver has two places to put it.
        let (plugin, shared) = running();
        let (mut io, mut drain) = transport(0.5, 0.5, 4);
        let mut buffer = plugin_buffer::PlanarBuffer::<{ BLOCK }>::new(1, BLOCK);
        let mut context = Context::new(48_000.0);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            4
        );
        assert_eq!(drain.slots(), 4);
        for _ in 0..4 {
            let [left, right] = drain.pop().unwrap().0;
            assert!(left.is_finite() && right.is_finite());
            assert!((left - 0.5).abs() < 1e-6, "the lane read {left}");
            assert!((right - 0.5).abs() < 1e-6, "the duplicate read {right}");
        }
        assert_eq!(
            shared.buffer_frames.load(Ordering::Relaxed),
            4,
            "the panel should be able to read the running block size"
        );
        assert!(shared.meter_updated.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn a_stereo_capture_is_mixed_by_the_shell_before_the_box_hears_one() {
        // Declaring one input does not make the interface one channel, so the
        // adapter does for this card what it does for every other mono pedal in
        // the set: mean the capture pair, hand the box that single signal, and
        // answer on both playback lanes. The equaliser never mixes anything —
        // this is the shell deciding what a mono box on a stereo interface is
        // fed — and the number is pinned here so that decision cannot change
        // quietly underneath a plugin whose tests assumed it.
        let (plugin, shared) = running();
        let (mut io, mut drain) = transport(0.5, -0.25, 2);
        let mut buffer = plugin_buffer::PlanarBuffer::<{ BLOCK }>::new(1, BLOCK);
        let mut context = Context::new(48_000.0);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            2
        );
        for _ in 0..2 {
            let [left, right] = drain.pop().unwrap().0;
            assert!(
                (left - 0.125).abs() < 1e-6,
                "the mean of 0.5 and -0.25 read {left}"
            );
            assert_eq!(left, right, "one lane out, given twice");
        }
    }

    #[test]
    fn the_feed_fills_only_while_an_editor_is_listening() {
        // The analyser's feed is the audio thread's only extra work, and two
        // things switch it off: no window, and a window that said `off`. This
        // covers the first, which is the one that runs on every machine that
        // never opens the editor at all.
        let (plugin, shared) = running();
        let (mut io, _drain) = transport(0.25, 0.25, 4);
        let mut buffer = plugin_buffer::PlanarBuffer::<{ BLOCK }>::new(1, BLOCK);
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
        // default, so both rings get the four samples, one per block at each
        // end of the box.
        shared.open.store(true, Ordering::Release);
        let (mut io, _drain) = transport(0.25, 0.25, 4);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            4
        );
        assert_eq!(feed_in.slots(), 4);
        assert_eq!(feed_out.slots(), 4);
        assert_eq!(shared.feed_samples.load(Ordering::Relaxed), 8);
        assert_eq!(shared.dropped_samples.load(Ordering::Relaxed), 0);
        while let Ok(sample) = feed_in.pop() {
            assert_eq!(sample, 0.25, "the input tap should carry the signal");
        }
        shared.put_feed((feed_in, feed_out));
    }

    #[test]
    fn an_empty_transport_pumps_nothing() {
        let (plugin, shared) = running();
        let (mut io, _drain) = transport(0.0, 0.0, 0);
        let mut buffer = plugin_buffer::PlanarBuffer::<{ BLOCK }>::new(1, BLOCK);
        let mut context = Context::new(44_100.0);
        assert_eq!(
            pump_once(&mut buffer, &mut context, &plugin, &shared, &mut io),
            0
        );
    }
}
