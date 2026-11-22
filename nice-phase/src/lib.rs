//! Nice Phase: the family's four-stage phaser as a CLAP/VST3 plugin,
//! with a frost editor. The DSP lives in [`dsp`]; this file is the
//! interface to the host — parameters the host can automate, a shared
//! view of the metering and the sweep for the editor, and the two
//! plugin-API identities.
#![deny(unsafe_code)]

mod dsp;
mod editor;
#[cfg(feature = "standalone")]
pub mod settings;
#[cfg(feature = "standalone")]
pub mod standalone;
mod ui;

use crossbeam::atomic::AtomicCell;
use editor::PhaseEditor;
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
/// every side of the boundary ever after. nice-plug's `FloatParam` has no
/// public setter — only a host's `GuiContext` may write one — so without a
/// host the shared cells below are the parameters' truth, and the places
/// that would otherwise defer to a host ask this flag instead. Like
/// nice-wah, nice-tuner and nice-nam, the plugin code itself is one
/// codebase; only the question "who owns the numbers" changes.
pub static STANDALONE_MODE: AtomicBool = AtomicBool::new(false);

const MIN_SIZE: LogicalSize<f32> = LogicalSize::new(560.0, 380.0);
const DEFAULT_SIZE: LogicalSize<f32> = LogicalSize::new(860.0, 440.0);
const RESIZE_HINT: ResizeHint = ResizeHint::resizable().with_min_logical_size(MIN_SIZE);

use nice_plug::prelude::*;

/// What the audio thread publishes for the editor, and nothing more: in a
/// host the editor reads and writes the parameters themselves through the
/// host's parameter API, so only the numbers born inside `process()` need a
/// crossing. Every field is an atomic cell of a single value — the audio
/// thread must never block behind the UI, and a torn meter reading is not
/// worth a lock.
pub struct PhaseSharedState {
    pub input_rms: AtomicCell<f32>,
    /// Linear *true* peak (dBTP) of the input: the crest of the waveform the
    /// DAC rebuilds between the samples, not the loudest sample. A reading
    /// above 1.0 is a finding, not an overflow.
    pub input_peak: AtomicCell<f32>,
    pub output_rms: AtomicCell<f32>,
    /// As the input peak, of what the plugin sent out.
    pub output_peak: AtomicCell<f32>,
    /// Where the sine stood, 0..1, and the stage frequency that position
    /// named — the sweep as the thread that knows the sample rate ran it.
    pub lfo: AtomicCell<f32>,
    pub center_hz: AtomicCell<f32>,
    /// Where the two notches actually stand this block — the filter's
    /// exact answer, published rather than recomputed in the view,
    /// because computing it needs the sample rate and the view does not
    /// own one. `notch_hi` reads 0.0 when the chain is a two-stage one:
    /// there is no second hole to mark.
    pub notch_lo: AtomicCell<f32>,
    pub notch_hi: AtomicCell<f32>,
    pub meter_updated: AtomicU64,
    /// The five parameters as the two threads last saw them. In a host
    /// `process()` mirrors these from the parameters each block, so they say
    /// what the filter actually ran on. In standalone the mirror is skipped
    /// and these cells are the parameters' truth: nice-plug gives a
    /// parameter no public setter, so a hostless app has no other door to
    /// write through, and the view writes these cells directly.
    pub speed: AtomicCell<f32>,
    pub mode: AtomicCell<f32>,
    pub depth: AtomicCell<f32>,
    pub mix: AtomicCell<f32>,
    pub out_db: AtomicCell<f32>,
    /// The block size the standalone pump last ran, frames; the settings
    /// panel shows it so a player can compare the screen with the dropdown.
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
    /// A saved top-left, queued by the standalone host and applied once by
    /// the window handler through the platform API — baseview has no
    /// position concept. Only placement reads it, so it must not warn
    /// without placement.
    #[cfg(feature = "standalone")]
    #[cfg_attr(not(all(windows, feature = "unsafe")), allow(dead_code))]
    pending_geom: Mutex<Option<(i32, i32)>>,
}

impl PhaseSharedState {
    fn logical_size(&self) -> LogicalSize<f32> {
        self.logical_size.load()
    }
    fn scale_factor(&self) -> f64 {
        self.scale_factor.load()
    }

    /// The DSP's reading of the cells. In standalone this is the parameter
    /// set; in a host it is last block's mirror, one audio block stale at
    /// the very worst.
    fn controls(&self) -> dsp::Controls {
        dsp::Controls {
            speed: self.speed.load(),
            mode: self.mode.load(),
            depth: self.depth.load(),
            mix: self.mix.load(),
            gain_db: self.out_db.load(),
        }
    }

    /// The other direction: publish what the host's parameters say, so the
    /// cells agree with what the filter is about to run on.
    fn publish(&self, controls: &dsp::Controls) {
        self.speed.store(controls.speed);
        self.mode.store(controls.mode);
        self.depth.store(controls.depth);
        self.mix.store(controls.mix);
        self.out_db.store(controls.gain_db);
    }
}

/// The five parameters, each exactly as the host sees it. Every one is a
/// plain float the host can automate; the mode is a float too, which is
/// how every selector in this family's history ends up under a host's
/// cursor — a stepped lane if the host offers one, a hard snap by
/// [`dsp::mode_of`] whatever it sends.
#[derive(Params)]
pub struct PhaseParams {
    /// The sine, in cycles per second.
    #[id = "speed"]
    pub speed: FloatParam,
    /// The selector, 0 .. 4, read by [`dsp::mode_of`].
    #[id = "mode"]
    pub mode: FloatParam,
    /// How deep the notches cut.
    #[id = "depth"]
    pub depth: FloatParam,
    /// The dry/wet blend.
    #[id = "mix"]
    pub mix: FloatParam,
    /// The output trim.
    #[id = "out_db"]
    pub out_db: FloatParam,
}

impl Default for PhaseParams {
    fn default() -> Self {
        // The script-logo board at its middle speed, full phase, and no
        // trim of the player's — the sound the family is known by, until
        // a selector move or a preset says otherwise.
        Self {
            speed: FloatParam::new(
                "Speed",
                0.5,
                FloatRange::Linear {
                    min: 0.05,
                    max: 20.0,
                },
            ),
            mode: FloatParam::new(
                "Mode",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: (dsp::MODES.len() - 1) as f32,
                },
            ),
            depth: FloatParam::new("Depth", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            mix: FloatParam::new("Mix", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            out_db: FloatParam::new(
                "Output",
                0.0,
                FloatRange::Linear {
                    min: -12.0,
                    max: 12.0,
                },
            ),
        }
    }
}

impl PhaseParams {
    fn controls(&self) -> dsp::Controls {
        dsp::Controls {
            speed: self.speed.value(),
            mode: self.mode.value(),
            depth: self.depth.value(),
            mix: self.mix.value(),
            gain_db: self.out_db.value(),
        }
    }
}

pub struct PhasePlugin {
    params: Arc<PhaseParams>,
    shared: Arc<PhaseSharedState>,
    filter: dsp::Phase,
    /// True-peak tracking of what arrives and what leaves, one estimator
    /// per channel, kept alive between blocks for the reason in
    /// [`block_stats`].
    tp_in: [TruePeak; 2],
    tp_out: [TruePeak; 2],
}

/// Seed the shared cells from the saved standalone preferences. A `None`
/// preference keeps the parameter's own default, so the settings file can
/// stay a record of moves rather than a snapshot of everything; a value
/// outside the parameter's range is clamped exactly as a host's write would
/// be, because a hand-edited RON is a user error, not a license to feed the
/// filter nonsense.
#[cfg(feature = "standalone")]
pub(crate) fn seed_standalone_params(
    shared: &PhaseSharedState,
    params: &PhaseParams,
    prefs: &settings::PhasePreferences,
) {
    let seed = |cell: &AtomicCell<f32>, param: &FloatParam, saved| {
        if let Some(value) = saved {
            // The normalize round-trip is the clamp the host would have
            // applied, on whatever curve the parameter uses.
            let t = param.range().normalize(value).clamp(0.0, 1.0);
            cell.store(param.range().unnormalize(t));
        }
    };
    seed(&shared.speed, &params.speed, prefs.speed);
    seed(&shared.mode, &params.mode, prefs.mode);
    seed(&shared.depth, &params.depth, prefs.depth);
    seed(&shared.mix, &params.mix, prefs.mix);
    seed(&shared.out_db, &params.out_db, prefs.out_db);
}

impl Default for PhasePlugin {
    fn default() -> Self {
        let params = Arc::new(PhaseParams::default());
        let shared = Arc::new(PhaseSharedState {
            input_rms: AtomicCell::new(0.0),
            input_peak: AtomicCell::new(0.0),
            output_rms: AtomicCell::new(0.0),
            output_peak: AtomicCell::new(0.0),
            lfo: AtomicCell::new(0.5),
            center_hz: AtomicCell::new(0.0),
            notch_lo: AtomicCell::new(0.0),
            notch_hi: AtomicCell::new(0.0),
            meter_updated: AtomicU64::new(0),
            speed: AtomicCell::new(params.speed.value()),
            mode: AtomicCell::new(params.mode.value()),
            depth: AtomicCell::new(params.depth.value()),
            mix: AtomicCell::new(params.mix.value()),
            out_db: AtomicCell::new(params.out_db.value()),
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
                eprintln!("nice-phase: {warning}");
            }
            seed_standalone_params(&shared, &params, &settings.preferences);
        }
        // The rate arrives with `activate`; until then the filter idles at a
        // rate any record could have been made at, and it is re-tuned there.
        let filter = dsp::Phase::new(44_100.0, &shared.controls());
        Self {
            params,
            shared,
            filter,
            tp_in: [TruePeak::new(); 2],
            tp_out: [TruePeak::new(); 2],
        }
    }
}

impl Plugin for PhasePlugin {
    const NAME: &'static str = "Nice Phase";
    const VENDOR: &'static str = "frost developers";
    const URL: &'static str = "https://github.com/perlindgren/frost";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// Mono in, mono out, and nothing else: the box is mono — one signal
    /// through one ganged capacitor bank — and so is this model. The
    /// plugin neither folds a stereo signal down nor widens a mono one;
    /// that width work belongs to the host and the rack, which is exactly
    /// why this is the only layout here.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(1),
        main_output_channels: NonZeroU32::new(1),
        ..AudioIOLayout::const_default()
    }];

    type Editor = PhaseEditor;
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
        self.filter.set_rate(config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.filter.reset();
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
            // One circuit board, one channel: the first channel is the
            // input. Should a host hand over a wider buffer than the mono
            // layout declares, the notches still cut channel 0 alone and
            // leave the other lanes silent — the plugin never folds
            // channels together; width is the host's work, not the
            // phaser's.
            self.filter
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
            // The metering is the same pass the filter just ran: energy
            // accumulated as f64 so a long block at low level still reports
            // a truth, published only when the block loop has actually seen
            // samples, so a zero-length call leaves the last truth standing.
            self.shared
                .input_rms
                .store((input_energy / count as f64).sqrt() as f32);
            self.shared
                .output_rms
                .store((output_energy / count as f64).sqrt() as f32);
            self.shared.input_peak.store(input_peak);
            self.shared.output_peak.store(output_peak);
            self.shared.lfo.store(self.filter.lfo_position());
            self.shared.center_hz.store(self.filter.center_hz());
            self.shared.notch_lo.store(self.filter.notch_hz(0));
            self.shared.notch_hi.store(self.filter.notch_hz(1));
            self.shared.meter_updated.store(now_ms(), Ordering::Relaxed);
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for PhasePlugin {
    const CLAP_ID: &'static str = "com.frost.nice-phase";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("The family's four-stage phaser, five settings and all");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Phaser];
}

impl Vst3Plugin for PhasePlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"FrostNicePhaseFx";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Modulation];
}

nice_export_clap!(PhasePlugin);
nice_export_vst3!(PhasePlugin);

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
        // has ever written; renaming one silently orphans a session. This
        // test exists to make that loud.
        let plugin = PhasePlugin::default();
        let map = plugin.params().param_map();
        let mut ids: Vec<&str> = map.iter().map(|(id, _, _)| id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["depth", "mix", "mode", "out_db", "speed"]);
    }

    #[test]
    fn the_defaults_are_the_script_board() {
        // Script logo at its middle speed, the summer's own depth, fully
        // wet, level in as level out: the sound out of the box.
        let params = PhaseParams::default();
        assert_eq!(params.speed.value(), 0.5);
        assert_eq!(params.mode.value(), 0.0);
        assert_eq!(params.depth.value(), 1.0);
        assert_eq!(params.mix.value(), 1.0);
        assert_eq!(params.out_db.value(), 0.0);
    }

    #[test]
    fn publishing_leaves_the_cells_saying_what_the_filter_runs_on() {
        // The host half of the deal: `process()` publishes the parameters
        // before reading the cells, so the DSP and the editor answer to one
        // set of numbers even while a host automates them mid-block.
        let plugin = PhasePlugin::default();
        let mut controls = plugin.params.controls();
        controls.speed = 4.5;
        controls.gain_db = -2.5;
        plugin.shared.publish(&controls);
        let read = plugin.shared.controls();
        assert_eq!(read.speed, 4.5);
        assert_eq!(read.gain_db, -2.5);
    }

    #[test]
    fn every_mode_fits_the_parameter_that_carries_it() {
        // The selector is one float and the table is its legend; a mode the
        // parameter cannot name would be a mode the host can never load.
        let params = PhaseParams::default();
        let last = (dsp::MODES.len() - 1) as f32;
        assert_eq!(
            dsp::mode_of(last).name,
            dsp::MODES[dsp::MODES.len() - 1].name
        );
        // Beyond the end the selector clamps rather than wraps — the same
        // total promise the parameter makes to a hand-typed settings file.
        assert_eq!(
            dsp::mode_of(99.0).name,
            dsp::MODES[dsp::MODES.len() - 1].name
        );
        assert_eq!(dsp::mode_of(-7.0).name, dsp::MODES[0].name);
        // And every trim the modes ask for is a level the trim parameter
        // can reach: the round-trip through the parameter's own
        // normalization is the reachability, proved on the host's own scale.
        for mode in dsp::MODES {
            let t = params.out_db.range().normalize(mode.trim_db);
            assert!((0.0..=1.0).contains(&t), "{}: trim out of reach", mode.name);
        }
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn saved_moves_seed_the_cells_and_silence_keeps_the_defaults() {
        let plugin = PhasePlugin::default();
        let defaults = plugin.shared.controls();
        seed_standalone_params(
            &plugin.shared,
            &plugin.params,
            &settings::PhasePreferences {
                speed: Some(2.5),
                mode: Some(3.0),
                depth: Some(0.6),
                mix: Some(0.4),
                out_db: Some(3.5),
            },
        );
        let seeded = plugin.shared.controls();
        // Tolerance, not identity: seeding walks the parameter's own
        // normalize/unnormalize round-trip, and a knob stored as a fraction
        // of its symmetric range need not come back bit for bit.
        let close = |a: f32, b: f32| (a - b).abs() <= 0.01f32.max(a.abs() * 1e-3);
        assert!(close(seeded.speed, 2.5));
        assert!(close(seeded.mode, 3.0));
        assert!(close(seeded.depth, 0.6));
        assert!(close(seeded.mix, 0.4));
        assert!(close(seeded.gain_db, 3.5));
        // A knob never moved keeps the parameter's own default: the file
        // records moves, it does not restate the plugin.
        let untouched = PhasePlugin::default();
        seed_standalone_params(
            &untouched.shared,
            &untouched.params,
            &settings::PhasePreferences::default(),
        );
        assert_eq!(untouched.shared.controls().speed, defaults.speed);
        // A hand-edited file cannot push a parameter off its range; the
        // round-trip clamps the way a host's write would have.
        seed_standalone_params(
            &plugin.shared,
            &plugin.params,
            &settings::PhasePreferences {
                out_db: Some(900.0),
                mode: Some(99.0),
                ..Default::default()
            },
        );
        assert_eq!(plugin.shared.out_db.load(), 12.0);
        assert_eq!(plugin.shared.mode.load(), 4.0);
    }

    /// The shape that crashed the metering inside a real host: a full block
    /// of stereo, where `channels` is two and `n` is the whole block. The
    /// inlined version sliced the channel dimension by the sample count and
    /// panicked on the first block a CLAP host ever sent.
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
        // A partial tail block reads only its first `n` samples; whatever
        // the previous block left behind in the scratch is not this block's
        // business.
        scratch[0][0] = 1.0;
        let (energy, peak) = block_stats(&scratch, 1, 0, &mut tp);
        assert_eq!((energy, peak), (0.0, 0.0));
    }
}
