//! Nice Brain Lag Tube: an original single-head tape-delay approximation, with a
//! tube-style front end. No particular hardware reproduction is claimed.
//! Sources and explicitly unverified attribution are documented in
//! `docs/plugin-inspiration.md`. DSP assumptions remain in `dsp`.
//! This file is the interface to the host: the parameters a host can automate,
//! a shared view of the metering for the editor, the two plugin-API identities,
//! and the block loop that hands samples to [`dsp`]. The transport snapshots
//! these controls once a block and runs every sample in between against that
//! snapshot, so what a host can say and what the tape is doing never meet in
//! the middle of a block.
//!
//! The control set is the machine's own, as its panel describes it: Delay
//! Time, Recount, Timbre and Echo Volume — plus Color, which the panel does
//! *not* have and this model adds as a handle on the tube front end's drive
//! (A-6), because on the real machine that colour only arrives by
//! overleveling the input, and a plugin user deserves the knob. The machine
//! had two input jacks and their trims; this model keeps neither — the one
//! input enters the tube stage at unity, and Color is the only drive. There
//! are no switches here, on either side: the machine has none — no mode
//! dial, no tank selector, no effect toggle — so this crate carries no
//! stepped parameters at all, and the parameter set is five floats.
//! Echo Volume sets the echo level against a dry that is always present
//! (A-7): like the machine, this plugin has no wet/dry crossfade and no
//! dry-kill.
#![deny(unsafe_code)]

mod dsp;
mod editor;
#[cfg(feature = "standalone")]
pub mod settings;
#[cfg(feature = "standalone")]
pub mod standalone;
mod ui;

use crossbeam::atomic::AtomicCell;
use editor::EcomplexEditor;
use nice_common::meter::block_stats;
use nice_common::true_peak::TruePeak;
use nice_plug::editor::dpi::LogicalSize;
use std::num::NonZeroU32;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};

#[cfg(feature = "standalone")]
pub use nice_common::devices;
#[cfg(feature = "standalone")]
use std::sync::Mutex;

/// Set once by the standalone binary before the plugin is built, and read on
/// every side of the boundary ever after. nice-plug's parameters have no
/// public setter — only a host's `GuiContext` may write one — so without a
/// host the shared cells below are the parameters' truth, and the places that
/// would otherwise defer to a host ask this flag instead. Same arrangement as
/// nice-echo, nice-wah, nice-nam, nice-tuner and nice-pi-face: one codebase,
/// and only the question "who owns the numbers" changes.
pub static STANDALONE_MODE: AtomicBool = AtomicBool::new(false);

const MIN_SIZE: LogicalSize<f32> = LogicalSize::new(620.0, 400.0);
const DEFAULT_SIZE: LogicalSize<f32> = LogicalSize::new(900.0, 520.0);
const RESIZE_HINT: ResizeHint = ResizeHint::resizable().with_min_logical_size(MIN_SIZE);

use nice_plug::prelude::*;

/// What the audio thread publishes for the editor, and nothing more: in a
/// host the editor reads and writes the parameters themselves through the
/// host's parameter API, so only the numbers born inside `process()` need a
/// crossing. Every field is an atomic cell of a single value — the audio
/// thread must never block behind the UI, and a torn meter reading is not
/// worth a lock.
pub struct EcomplexSharedState {
    pub input_rms: AtomicCell<f32>,
    /// Linear *true* peak (dBTP) of the input: the crest of the waveform the
    /// DAC rebuilds between the samples, not the loudest sample. A reading
    /// above 1.0 is a finding, not an overflow.
    pub input_peak: AtomicCell<f32>,
    pub output_rms: AtomicCell<f32>,
    /// As the input peak, of what the plugin sent out.
    pub output_peak: AtomicCell<f32>,
    pub meter_updated: AtomicU64,
    /// The parameters as the two threads last saw them. In a host `process()`
    /// mirrors these from the parameters each block, so they say what the
    /// transport ran on. In standalone the mirror is skipped and these cells
    /// are the parameters' truth: nice-plug gives a parameter no public
    /// setter, so a hostless app has no other door to write through, and the
    /// view writes these cells directly.
    pub color: AtomicCell<f32>,
    pub delay_time: AtomicCell<f32>,
    pub recount: AtomicCell<f32>,
    pub timbre: AtomicCell<f32>,
    pub echo_volume: AtomicCell<f32>,
    /// The block size the standalone pump last ran, frames; the settings panel
    /// shows it so a player can compare the screen with the dropdown.
    pub buffer_frames: AtomicU32,
    /// Capture frames the transport dropped because this app fell behind,
    /// copied off the transport's own counter by the audio owner.
    pub dropped: AtomicU64,
    logical_size: AtomicCell<LogicalSize<f32>>,
    scale_factor: AtomicCell<f64>,
    open: AtomicBool,
    /// The window's top-left, as the platform last reported it. Standalone
    /// only: a window inside a host belongs to the host, position included.
    #[cfg(feature = "standalone")]
    window_pos: Mutex<Option<(i32, i32)>>,
    /// A saved top-left, queued by the standalone host and applied once by the
    /// window handler through the platform API — baseview has no position
    /// concept. Only placement reads it, so it must not warn without
    /// placement.
    #[cfg(feature = "standalone")]
    #[cfg_attr(not(all(windows, feature = "unsafe")), allow(dead_code))]
    pending_geom: Mutex<Option<(i32, i32)>>,
}

impl EcomplexSharedState {
    fn logical_size(&self) -> LogicalSize<f32> {
        self.logical_size.load()
    }
    fn scale_factor(&self) -> f64 {
        self.scale_factor.load()
    }

    /// The DSP's reading of the cells. In standalone this is the parameter
    /// set; in a host it is last block's mirror, one audio block stale at the
    /// very worst.
    fn controls(&self) -> dsp::Controls {
        dsp::Controls {
            color: self.color.load(),
            delay_time: self.delay_time.load(),
            recount: self.recount.load(),
            timbre: self.timbre.load(),
            echo_volume: self.echo_volume.load(),
        }
    }

    /// The other direction: publish what the host's parameters say, so the
    /// cells agree with what the transport is about to run on.
    fn publish(&self, controls: &dsp::Controls) {
        self.color.store(controls.color);
        self.delay_time.store(controls.delay_time);
        self.recount.store(controls.recount);
        self.timbre.store(controls.timbre);
        self.echo_volume.store(controls.echo_volume);
    }
}

/// The parameters, each exactly as the host sees it: five floats, one for
/// each knob this model keeps (Color included, and it is honest about being
/// this model's addition — see the module header). The ranges are
/// all the plain 0..1 travel; every taper that is not linear — Delay Time's
/// exponential, Echo Volume's squared law — lives in `dsp`, where a test can
/// name it, because a parameter range is a wire format and a curve is a
/// model.
#[derive(Params)]
pub struct EcomplexParams {
    /// Color: how hard the tube front end is driven (A-6) — the model's only
    /// input level control, since the signal itself enters the stage at
    /// unity. Not a panel control: this model's knob for a property the
    /// machine only lets you reach by overleveling the input.
    #[id = "color"]
    pub color: FloatParam,
    /// Delay Time: tape speed, slow = long. The knob is linear; the seconds
    /// it means are not (A-1).
    #[id = "delay_time"]
    pub delay_time: FloatParam,
    /// Recount: repeat count, crossing unity loop gain at three-quarters of
    /// its travel (A-4).
    #[id = "recount"]
    pub recount: FloatParam,
    /// Timbre: the tone of the repeats, one-pole low-pass 300 Hz to 12 kHz
    /// over the fixed 90 Hz head tilt (A-5).
    #[id = "timbre"]
    pub timbre: FloatParam,
    /// Echo Volume: the level of the echo against the ever-present dry (A-7).
    /// The machine's own "Volume" — there is no dry control to fade.
    #[id = "echo_volume"]
    pub echo_volume: FloatParam,
}

impl Default for EcomplexParams {
    /// Where the controls sit on a fresh instance. The single input enters
    /// the front end at unity, so Color alone sets how hard the tube stage
    /// is worked; it sits low — the factory sound should be the tube stage
    /// at its clean edge, not a clipped one, and not timid either (A-6).
    ///
    /// Delay Time centres the exponential range at its geometric mean, about
    /// 346 ms (A-1) — "medium" on a machine whose whole charm is that its
    /// delays live where tape lives. Recount sits under A-4's unity
    /// crossover: a first listen must hear repeats decay, and the howling
    /// quarter of the knob stays for whoever reaches for it. Timbre centres
    /// the exponential tone range. Echo Volume at half is this model's
    /// "sitting behind the dry", the way the machine is usually played.
    fn default() -> Self {
        let knob =
            |name, value| FloatParam::new(name, value, FloatRange::Linear { min: 0.0, max: 1.0 });
        Self {
            color: knob("Color", 0.2),
            delay_time: knob("Delay Time", 0.5),
            recount: knob("Recount", 0.4),
            timbre: knob("Timbre", 0.5),
            echo_volume: knob("Echo Volume", 0.5),
        }
    }
}

impl EcomplexParams {
    /// The DSP's reading of the parameter set. Called at the top of each
    /// block: parameter reads are atomic loads, and taking the whole set at
    /// once is what lets the transport's sample loop touch no atomics at all.
    fn controls(&self) -> dsp::Controls {
        dsp::Controls {
            color: self.color.value(),
            delay_time: self.delay_time.value(),
            recount: self.recount.value(),
            timbre: self.timbre.value(),
            echo_volume: self.echo_volume.value(),
        }
    }
}

pub struct EcomplexPlugin {
    params: Arc<EcomplexParams>,
    shared: Arc<EcomplexSharedState>,
    ecomplex: dsp::Ecomplex,
    /// True-peak tracking of what arrives and what leaves, one estimator
    /// per channel, kept alive between blocks for the reason in
    /// [`block_stats`].
    tp_in: [TruePeak; 2],
    tp_out: [TruePeak; 2],
}

/// Seed the shared cells from the saved standalone preferences. A `None`
/// preference keeps the parameter's own default, so the settings file stays a
/// record of moves rather than a snapshot of everything; a value outside the
/// parameter's range is clamped exactly as a host's write would be, because a
/// hand-edited RON is a user error, not a license to feed the transport
/// nonsense.
#[cfg(feature = "standalone")]
pub(crate) fn seed_standalone_params(
    shared: &EcomplexSharedState,
    params: &EcomplexParams,
    prefs: &settings::EcomplexPreferences,
) {
    let seed = |cell: &AtomicCell<f32>, param: &FloatParam, saved: Option<f32>| {
        if let Some(value) = saved {
            // The normalize round-trip is the clamp the host would have
            // applied, on whatever curve the parameter uses.
            let t = param.range().normalize(value).clamp(0.0, 1.0);
            cell.store(param.range().unnormalize(t));
        }
    };
    seed(&shared.color, &params.color, prefs.color);
    seed(&shared.delay_time, &params.delay_time, prefs.delay_time);
    seed(&shared.recount, &params.recount, prefs.recount);
    seed(&shared.timbre, &params.timbre, prefs.timbre);
    seed(&shared.echo_volume, &params.echo_volume, prefs.echo_volume);
}

impl Default for EcomplexPlugin {
    fn default() -> Self {
        let params = Arc::new(EcomplexParams::default());
        let shared = Arc::new(EcomplexSharedState {
            input_rms: AtomicCell::new(0.0),
            input_peak: AtomicCell::new(0.0),
            output_rms: AtomicCell::new(0.0),
            output_peak: AtomicCell::new(0.0),
            meter_updated: AtomicU64::new(0),
            color: AtomicCell::new(params.color.value()),
            delay_time: AtomicCell::new(params.delay_time.value()),
            recount: AtomicCell::new(params.recount.value()),
            timbre: AtomicCell::new(params.timbre.value()),
            echo_volume: AtomicCell::new(params.echo_volume.value()),
            buffer_frames: AtomicU32::new(0),
            dropped: AtomicU64::new(0),
            logical_size: AtomicCell::new(DEFAULT_SIZE),
            scale_factor: AtomicCell::new(1.0),
            open: AtomicBool::new(false),
            #[cfg(feature = "standalone")]
            window_pos: Mutex::new(None),
            #[cfg(feature = "standalone")]
            pending_geom: Mutex::new(None),
        });
        // A standalone starts where the player left it; a plugin instance
        // starts at the compiled defaults and the host loads over them.
        #[cfg(feature = "standalone")]
        if STANDALONE_MODE.load(Ordering::Relaxed) {
            let (settings, warning) = settings::Settings::load();
            if let Some(warning) = warning {
                eprintln!("nice-ecomplex: {warning}");
            }
            seed_standalone_params(&shared, &params, &settings.preferences);
        }
        // The rate arrives with `activate`; until then the transport idles at
        // a rate any record could have been made at, and it is re-armed
        // there.
        let ecomplex = dsp::Ecomplex::new(48_000.0, &shared.controls());
        Self {
            params,
            shared,
            ecomplex,
            tp_in: [TruePeak::new(); 2],
            tp_out: [TruePeak::new(); 2],
        }
    }
}

impl Plugin for EcomplexPlugin {
    /// What a host lists in its plugin menu, in both wrappers. Spelled the same
    /// way as the `name` in the workspace `bundler.toml`, because a bundle
    /// called "Nice Brain Lag Tube" that opens as "Ecomplex" is a name the operator
    /// has to reconcile by hand. Neither identity string derives from this: the
    /// CLAP id and the VST3 class id below are what a saved session resolves,
    /// so renaming here does not orphan anyone's project.
    const NAME: &'static str = "Nice Brain Lag Tube";
    const VENDOR: &'static str = "frost developers";
    const URL: &'static str = "https://github.com/perlindgren/frost";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// Mono in, mono out, and nothing else: the machine is mono — one
    /// source on one tape — and so is this model, the input entering the
    /// tube stage at unity. The plugin neither folds a stereo signal down
    /// nor widens a mono one; that width work belongs to the host and the
    /// rack, which is exactly why this is the only layout here.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(1),
        main_output_channels: NonZeroU32::new(1),
        ..AudioIOLayout::const_default()
    }];

    type Editor = EcomplexEditor;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _: AsyncExecutor<Self>) -> Option<Self::Editor> {
        Some(crate::editor::create(
            self.params.clone(),
            self.shared.clone(),
        ))
    }

    fn activate(
        &mut self,
        _: &AudioIOLayout,
        config: &BufferConfig,
        _: &mut impl ActivateContext<Self>,
    ) -> bool {
        self.ecomplex.set_rate(config.sample_rate);
        true
    }

    fn reset(&mut self) {
        // Reset in place, including wow phase; hosts may reset on the audio thread.
        self.ecomplex.reset();
        // The meters come back empty with it: a crest does not outlive
        // the audio it was read from, and an estimator carrying a block of
        // sound from before the reset would ring into the first block after
        // it.
        for meter in self.tp_in.iter_mut().chain(self.tp_out.iter_mut()) {
            meter.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // A host owns the parameters, so publish them before anything reads
        // the cells. In standalone the view owns them, and mirroring here
        // would overwrite what the settings panel just wrote.
        if !STANDALONE_MODE.load(Ordering::Relaxed) {
            let controls = self.params.controls();
            self.shared.publish(&controls);
        }
        let channels = buffer.channels();
        let mut input = [[0.0f32; dsp::BLOCK]; 2];
        let mut output = [[0.0f32; dsp::BLOCK]; 2];
        let mut input_energy = 0.0f64;
        let mut output_energy = 0.0f64;
        let mut input_peak = 0.0f32;
        let mut output_peak = 0.0f32;
        let mut count = 0usize;
        for (_, block) in buffer.iter_blocks(dsp::BLOCK) {
            let n = block.samples();
            let controls = self.shared.controls();
            for (channel, scratch) in input.iter_mut().enumerate().take(channels) {
                let samples = block.get(channel).unwrap();
                scratch[..n].copy_from_slice(&samples[..n]);
            }
            let (energy, peak) = block_stats(&input, channels, n, &mut self.tp_in);
            input_energy += energy;
            input_peak = input_peak.max(peak);
            // One tape, one channel: the first channel is the input. Should
            // a host hand over a wider buffer than the mono layout declares,
            // the tape still reads channel 0 alone and leaves the other lanes
            // silent — the plugin never folds channels together; width is
            // the host's work, not the tape's.
            self.ecomplex
                .process_mono(&controls, &input[0][..n], &mut output[0][..n]);
            let (energy, peak) = block_stats(&output, channels, n, &mut self.tp_out);
            output_energy += energy;
            output_peak = output_peak.max(peak);
            count += n;
            for (channel, out) in block.into_iter().enumerate().take(channels) {
                out.copy_from_slice(&output[channel][..n]);
            }
        }
        if count > 0 {
            // The metering is the same pass the transport just ran: energy
            // accumulated as f64 so a long block at low level still reports a
            // truth, published only when the block loop has actually seen
            // samples, so a zero-length call leaves the last truth standing.
            self.shared
                .input_rms
                .store((input_energy / count as f64).sqrt() as f32);
            self.shared
                .output_rms
                .store((output_energy / count as f64).sqrt() as f32);
            self.shared.input_peak.store(input_peak);
            self.shared.output_peak.store(output_peak);
            self.shared.meter_updated.store(now_ms(), Ordering::Relaxed);
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for EcomplexPlugin {
    const CLAP_ID: &'static str = "com.frost.nice-ecomplex";
    const CLAP_DESCRIPTION: Option<&'static str> = Some(
        "Original single-head tape-delay approximation with a tube-style front end; no manufacturer affiliation",
    );
    const CLAP_MANUAL_URL: Option<&'static str> =
        Some("https://github.com/perlindgren/frost/blob/main/docs/plugin-inspiration.md");
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    // Delay only: the machine has no reverb of any kind, and this model
    // declares none — claiming Reverb here would be a feature the tape does
    // not have.
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Delay];
}

impl Vst3Plugin for EcomplexPlugin {
    /// Sixteen bytes, exactly, and different from every other member's: two
    /// plugins claiming one class id is a host that loads the wrong one.
    const VST3_CLASS_ID: [u8; 16] = *b"FrostNiceEcmplx1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Delay];
}

// The two plugin entry points, unconditional: the same `cdylib` answers for
// both formats, and `standalone` is a separate concern that brings the device
// transport with it — ASIO through azo on Windows, cpal's hosts and JACK
// elsewhere. A plain comment rather than a doc comment:
// rustdoc has nothing to attach here, and these macros do not carry docs
// through.
nice_export_clap!(EcomplexPlugin);
nice_export_vst3!(EcomplexPlugin);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_sees_exactly_the_parameters_we_promised() {
        // The parameter ids are the wire format of every automation a host
        // will ever write; renaming one silently orphans a session. This
        // crate has not shipped yet, which is why a wrong id could still be
        // changed for free — after the first release, dropping or renaming an
        // id is a migration, and this test is what makes that loud.
        let plugin = EcomplexPlugin::default();
        let map = plugin.params().param_map();
        let mut ids: Vec<&str> = map.iter().map(|(id, _, _)| id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            ["color", "delay_time", "echo_volume", "recount", "timbre"]
        );
    }

    #[test]
    fn the_defaults_are_a_configuration_the_sources_describe() {
        // There is no factory preset for a 1964 tape echo, so "default" here
        // means the playing position the sources describe as normal: dry and
        // echo together, repeats decaying, tone mid-travel, and a unity
        // input meeting a front end just cracked open — the clean edge, not
        // past it (A-6).
        let params = EcomplexParams::default();
        let controls = params.controls();
        // Recount sits under A-4's unity crossover: the factory sound decays,
        // and the howling quarter stays for whoever reaches for it.
        assert!(
            dsp::feedback_gain(controls.recount) < 1.0,
            "factory Recount must not self-oscillate"
        );
        // The dry is always present (A-7): a fresh instance must run wet but
        // not fully substituted — Echo Volume at half, squared-law, sits
        // below the 1.2 full-travel gain.
        assert!(controls.echo_volume > 0.0 && controls.echo_volume < 1.0);
        // Timbre and Delay Time centre their exponential ranges.
        assert_eq!(controls.timbre, 0.5);
        assert_eq!(controls.delay_time, 0.5);
        // The whole set is what `dsp` itself calls the resting place; if
        // these two ever drift, the plugin and the model disagree about the
        // factory sound and neither says so.
        let dsp_default = dsp::Controls::default();
        assert_eq!(controls.color, dsp_default.color);
        assert_eq!(controls.delay_time, dsp_default.delay_time);
        assert_eq!(controls.recount, dsp_default.recount);
        assert_eq!(controls.timbre, dsp_default.timbre);
        assert_eq!(controls.echo_volume, dsp_default.echo_volume);
    }

    #[test]
    fn publishing_leaves_the_cells_saying_what_the_transport_runs_on() {
        // The host half of the deal: `process()` publishes the parameters
        // before reading the cells, so the DSP and the editor answer to one
        // set of numbers even while a host automates them mid-block.
        let plugin = EcomplexPlugin::default();
        let mut controls = plugin.params.controls();
        controls.delay_time = 0.9;
        controls.recount = 0.95;
        controls.color = 0.75;
        plugin.shared.publish(&controls);
        let read = plugin.shared.controls();
        assert_eq!(read.delay_time, 0.9);
        assert_eq!(read.recount, 0.95);
        assert_eq!(read.color, 0.75);
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn saved_moves_seed_the_cells_and_silence_keeps_the_defaults() {
        let plugin = EcomplexPlugin::default();
        let defaults = plugin.shared.controls();
        seed_standalone_params(
            &plugin.shared,
            &plugin.params,
            &settings::EcomplexPreferences {
                delay_time: Some(0.8),
                recount: Some(0.9),
                color: Some(0.6),
                ..Default::default()
            },
        );
        let seeded = plugin.shared.controls();
        // Tolerance, not identity: seeding walks the parameter's own
        // normalize/unnormalize round-trip, and a value need not come back
        // from it bit for bit.
        let close = |a: f32, b: f32| (a - b).abs() <= 0.01f32.max(a.abs() * 1e-3);
        assert!(close(seeded.delay_time, 0.8));
        assert!(close(seeded.recount, 0.9));
        assert!(close(seeded.color, 0.6));
        // A control never moved keeps the parameter's own default: the file
        // records moves, it does not restate the plugin.
        assert_eq!(seeded.timbre, defaults.timbre);
        assert_eq!(seeded.echo_volume, defaults.echo_volume);
        // A hand-edited file cannot push a parameter off its range; the clamp
        // is the one a host's write would have applied.
        seed_standalone_params(
            &plugin.shared,
            &plugin.params,
            &settings::EcomplexPreferences {
                delay_time: Some(4.0),
                echo_volume: Some(-3.0),
                ..Default::default()
            },
        );
        assert_eq!(plugin.shared.delay_time.load(), 1.0);
        assert_eq!(plugin.shared.echo_volume.load(), 0.0);
    }

    /// The shape that once crashed a sibling's metering inside a real host:
    /// a full block of stereo, where `channels` is two and `n` is the whole
    /// block. An inlined version sliced the channel dimension by the sample
    /// count and panicked on the first block a CLAP host ever sent.
    #[test]
    fn metering_counts_channels_and_samples_in_the_right_order() {
        let mut scratch = [[0.0f32; dsp::BLOCK]; 2];
        for sample in scratch[0].iter_mut() {
            *sample = -0.5;
        }
        for sample in scratch[1].iter_mut() {
            *sample = 0.25;
        }
        // The first block lands on a history of silence, which is an abrupt
        // onset as far as the interpolator is concerned; the second identical
        // block is the steady state the meter settles to, and that is the
        // reading pinned here: the crest of a constant is the constant, never
        // the overshoot of its own edge.
        let mut tp = [TruePeak::new(); 2];
        block_stats(&scratch, 2, dsp::BLOCK, &mut tp);
        let (energy, peak) = block_stats(&scratch, 2, dsp::BLOCK, &mut tp);
        assert!((energy - (dsp::BLOCK as f64 * (0.25 + 0.0625))).abs() < 1e-9);
        assert_eq!(peak, 0.5);
        // A partial tail block reads only its first `n` samples; whatever the
        // previous block left behind in the scratch is not this block's
        // business.
        scratch[0][0] = 1.0;
        let (energy, peak) = block_stats(&scratch, 1, 0, &mut tp);
        assert_eq!((energy, peak), (0.0, 0.0));
    }
}
