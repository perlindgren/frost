//! Nice Wah: an envelope-sensing wah pedal as a CLAP/VST3 plugin, with a
//! frost editor. The DSP lives in [`dsp`]; this file is the interface to the
//! host — parameters the host can automate, a shared view of the metering
//! for the editor, and the two plugin-API identities.
#![deny(unsafe_code)]

mod dsp;
mod editor;
mod gui;
#[cfg(feature = "standalone")]
pub mod settings;
#[cfg(feature = "standalone")]
pub mod standalone;
mod ui;

use crossbeam::atomic::AtomicCell;
use editor::WahEditor;
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
/// nice-tuner and nice-nam, the plugin code itself is one codebase; only the
/// question "who owns the numbers" changes.
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
pub struct WahSharedState {
    pub input_rms: AtomicCell<f32>,
    pub input_peak: AtomicCell<f32>,
    pub output_rms: AtomicCell<f32>,
    pub output_peak: AtomicCell<f32>,
    /// The follower, normalised 0..1, and the effective sweep position —
    /// the pedal plus the envelope — that the filter actually ran at.
    pub env: AtomicCell<f32>,
    pub effective: AtomicCell<f32>,
    /// The frequency the corner actually sits at, hertz: the number the
    /// editor prints, computed by the thread that knows the sample rate.
    pub corner_hz: AtomicCell<f32>,
    pub meter_updated: AtomicU64,
    /// The seven parameters as the two threads last saw them. In a host
    /// `process()` mirrors these from the parameters each block, so they say
    /// what the filter actually ran on. In standalone the mirror is skipped
    /// and these cells are the parameters' truth: nice-plug gives a
    /// parameter no public setter, so a hostless app has no other door to
    /// write through, and the view writes these cells directly.
    pub position: AtomicCell<f32>,
    pub env_amount: AtomicCell<f32>,
    pub heel_hz: AtomicCell<f32>,
    pub toe_hz: AtomicCell<f32>,
    pub resonance: AtomicCell<f32>,
    pub taper: AtomicCell<f32>,
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
impl WahSharedState {
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
            position: self.position.load(),
            env_amount: self.env_amount.load(),
            heel_hz: self.heel_hz.load(),
            toe_hz: self.toe_hz.load(),
            q: self.resonance.load(),
            taper: self.taper.load(),
            gain_db: self.out_db.load(),
        }
    }

    /// The other direction: publish what the host's parameters say, so the
    /// cells agree with what the filter is about to run on.
    fn publish(&self, controls: &dsp::Controls) {
        self.position.store(controls.position);
        self.env_amount.store(controls.env_amount);
        self.heel_hz.store(controls.heel_hz);
        self.toe_hz.store(controls.toe_hz);
        self.resonance.store(controls.q);
        self.taper.store(controls.taper);
        self.out_db.store(controls.gain_db);
    }
}

/// The seven parameters, each exactly as the host sees it. Every one is a
/// plain float the host can automate; the voices are not a mode but a set
/// of moves on five of them, which is why a preset is a convenience and
/// automation stays honest.
#[derive(Params)]
pub struct WahParams {
    /// The treadle, 0 heel to 1 toe.
    #[id = "position"]
    pub position: FloatParam,
    /// How far the envelope may push past the treadle.
    #[id = "env_amount"]
    pub env_amount: FloatParam,
    /// The voice's sweep ends.
    #[id = "heel_hz"]
    pub heel_hz: FloatParam,
    #[id = "toe_hz"]
    pub toe_hz: FloatParam,
    /// The height of the resonance.
    #[id = "resonance"]
    pub resonance: FloatParam,
    /// Where the treadle's resolution sits: −1 toe, +1 heel.
    #[id = "taper"]
    pub taper: FloatParam,
    /// The output trim.
    #[id = "out_db"]
    pub out_db: FloatParam,
}

impl Default for WahParams {
    fn default() -> Self {
        // The archetype on the pedalboard: the wide modern sweep, until a
        // voice or a preset says otherwise.
        let voice = dsp::VOICES[1];
        Self {
            position: FloatParam::new("Position", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 }),
            env_amount: FloatParam::new(
                "Env Amount",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            heel_hz: FloatParam::new(
                "Heel",
                voice.heel_hz,
                FloatRange::Linear {
                    min: 40.0,
                    max: 1_000.0,
                },
            )
            .with_unit(" Hz"),
            toe_hz: FloatParam::new(
                "Toe",
                voice.toe_hz,
                FloatRange::Linear {
                    min: 1_500.0,
                    max: 12_000.0,
                },
            )
            .with_unit(" Hz"),
            resonance: FloatParam::new(
                "Resonance",
                voice.q,
                FloatRange::Linear {
                    min: 0.7,
                    max: 12.0,
                },
            ),
            taper: FloatParam::new(
                "Taper",
                voice.taper,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            ),
            out_db: FloatParam::new(
                "Output",
                voice.trim_db,
                FloatRange::Linear {
                    min: -12.0,
                    max: 12.0,
                },
            )
            .with_unit(" dB"),
        }
    }
}

impl WahParams {
    /// The DSP's reading of the parameter set. Called at the top of each
    /// block: parameter reads are atomic loads, and taking the whole set at
    /// once is what lets the filter's sample loop touch no atomics at all.
    fn controls(&self) -> dsp::Controls {
        dsp::Controls {
            position: self.position.value(),
            env_amount: self.env_amount.value(),
            heel_hz: self.heel_hz.value(),
            toe_hz: self.toe_hz.value(),
            q: self.resonance.value(),
            taper: self.taper.value(),
            gain_db: self.out_db.value(),
        }
    }
}

/// The metering's two questions to one block of scratch: total energy, and
/// the loudest sample. Over the channel dimension first and the sample
/// dimension second — a mixup of those two once crashed the metering inside
/// a real host while every in-crate test stayed green, which is why this
/// is a function with a test rather than four expressions inlined twice.
fn block_stats(scratch: &[[f32; dsp::BLOCK]; 2], channels: usize, n: usize) -> (f64, f32) {
    scratch[..channels]
        .iter()
        .fold((0.0f64, 0.0f32), |(energy, peak), channel| {
            channel[..n]
                .iter()
                .fold((energy, peak), |(energy, peak), &x| {
                    (energy + f64::from(x).powi(2), peak.max(x.abs()))
                })
        })
}

pub struct WahPlugin {
    params: Arc<WahParams>,
    shared: Arc<WahSharedState>,
    filter: dsp::Wah,
}

/// Seed the shared cells from the saved standalone preferences. A `None`
/// preference keeps the parameter's own default, so the settings file can
/// stay a record of moves rather than a snapshot of everything; a value
/// outside the parameter's range is clamped exactly as a host's write would
/// be, because a hand-edited RON is a user error, not a license to feed the
/// filter nonsense.
#[cfg(feature = "standalone")]
pub(crate) fn seed_standalone_params(
    shared: &WahSharedState,
    params: &WahParams,
    prefs: &settings::WahPreferences,
) {
    let seed = |cell: &AtomicCell<f32>, param: &FloatParam, saved| {
        if let Some(value) = saved {
            // The normalize round-trip is the clamp the host would have
            // applied, on whatever curve the parameter uses.
            let t = param.range().normalize(value).clamp(0.0, 1.0);
            cell.store(param.range().unnormalize(t));
        }
    };
    seed(&shared.position, &params.position, prefs.position);
    seed(&shared.env_amount, &params.env_amount, prefs.env_amount);
    seed(&shared.heel_hz, &params.heel_hz, prefs.heel_hz);
    seed(&shared.toe_hz, &params.toe_hz, prefs.toe_hz);
    seed(&shared.resonance, &params.resonance, prefs.resonance);
    seed(&shared.taper, &params.taper, prefs.taper);
    seed(&shared.out_db, &params.out_db, prefs.out_db);
}

impl Default for WahPlugin {
    fn default() -> Self {
        let params = Arc::new(WahParams::default());
        let shared = Arc::new(WahSharedState {
            input_rms: AtomicCell::new(0.0),
            input_peak: AtomicCell::new(0.0),
            output_rms: AtomicCell::new(0.0),
            output_peak: AtomicCell::new(0.0),
            env: AtomicCell::new(0.0),
            effective: AtomicCell::new(0.0),
            corner_hz: AtomicCell::new(0.0),
            meter_updated: AtomicU64::new(0),
            position: AtomicCell::new(params.position.value()),
            env_amount: AtomicCell::new(params.env_amount.value()),
            heel_hz: AtomicCell::new(params.heel_hz.value()),
            toe_hz: AtomicCell::new(params.toe_hz.value()),
            resonance: AtomicCell::new(params.resonance.value()),
            taper: AtomicCell::new(params.taper.value()),
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
                eprintln!("nice-wah: {warning}");
            }
            seed_standalone_params(&shared, &params, &settings.preferences);
        }
        // The rate arrives with `activate`; until then the filter idles at a
        // rate any record could have been made at, and it is re-tuned there.
        let filter = dsp::Wah::new(44_100.0, &shared.controls());
        Self {
            params,
            shared,
            filter,
        }
    }
}

impl Plugin for WahPlugin {
    const NAME: &'static str = "Nice Wah";
    const VENDOR: &'static str = "frost developers";
    const URL: &'static str = "https://github.com/perlindgren/frost";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(1),
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];

    type Editor = WahEditor;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _: AsyncExecutor<Self>) -> Option<Self::Editor> {
        Some(WahEditor {
            params: self.params.clone(),
            shared: self.shared.clone(),
        })
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
            let (energy, peak) = block_stats(&input, channels, n);
            input_energy += energy;
            input_peak = input_peak.max(peak);
            if channels == 1 {
                self.filter
                    .process_mono(&controls, &input[0][..n], &mut output[0][..n]);
            } else {
                let (left, right) = output.split_at_mut(1);
                self.filter.process_stereo(
                    &controls,
                    [&input[0][..n], &input[1][..n]],
                    [&mut left[0][..n], &mut right[0][..n]],
                );
            }
            let (energy, peak) = block_stats(&output, channels, n);
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
            self.shared.env.store(self.filter.env_level());
            let effective = self.filter.effective_position();
            self.shared.effective.store(effective);
            let controls = self.shared.controls();
            self.shared.corner_hz.store(dsp::sweep_hz(
                effective,
                controls.heel_hz,
                controls.toe_hz,
                controls.taper,
            ));
            self.shared.meter_updated.store(now_ms(), Ordering::Relaxed);
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for WahPlugin {
    const CLAP_ID: &'static str = "com.frost.nice-wah";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("An envelope-sensing wah with the three classic voicings");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Filter];
}

impl Vst3Plugin for WahPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"FrostNiceWahFx01";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Filter];
}

nice_export_clap!(WahPlugin);
nice_export_vst3!(WahPlugin);

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
        let plugin = WahPlugin::default();
        let map = plugin.params().param_map();
        let mut ids: Vec<&str> = map.iter().map(|(id, _, _)| id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            [
                "env_amount",
                "heel_hz",
                "out_db",
                "position",
                "resonance",
                "taper",
                "toe_hz"
            ]
        );
    }

    #[test]
    fn the_defaults_are_the_archetype_voice() {
        let voice = dsp::VOICES[1];
        let params = WahParams::default();
        assert_eq!(params.heel_hz.value(), voice.heel_hz);
        assert_eq!(params.toe_hz.value(), voice.toe_hz);
        assert_eq!(params.resonance.value(), voice.q);
        assert_eq!(params.taper.value(), voice.taper);
        assert_eq!(params.out_db.value(), voice.trim_db);
        // Pedal down the middle, envelope half wired: a plain wah out of
        // the box, one move from the auto one.
        assert_eq!(params.position.value(), 0.5);
        assert_eq!(params.env_amount.value(), 0.5);
    }

    #[test]
    fn publishing_leaves_the_cells_saying_what_the_filter_runs_on() {
        // The host half of the deal: `process()` publishes the parameters
        // before reading the cells, so the DSP and the editor answer to one
        // set of numbers even while a host automates them mid-block.
        let plugin = WahPlugin::default();
        let mut controls = plugin.params.controls();
        controls.heel_hz = 123.0;
        controls.gain_db = -2.5;
        plugin.shared.publish(&controls);
        let read = plugin.shared.controls();
        assert_eq!(read.heel_hz, 123.0);
        assert_eq!(read.gain_db, -2.5);
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn saved_moves_seed_the_cells_and_silence_keeps_the_defaults() {
        let plugin = WahPlugin::default();
        let defaults = plugin.shared.controls();
        seed_standalone_params(
            &plugin.shared,
            &plugin.params,
            &settings::WahPreferences {
                position: Some(0.25),
                heel_hz: Some(85.0),
                toe_hz: Some(2_200.0),
                resonance: Some(4.0),
                taper: Some(-0.2),
                out_db: Some(3.5),
                env_amount: None,
            },
        );
        let seeded = plugin.shared.controls();
        // Tolerance, not identity: seeding walks the parameter's own
        // normalize/unnormalize round-trip, and a knob like taper — a
        // symmetric range stored as a fraction of itself — need not come
        // back from it bit for bit.
        let close = |a: f32, b: f32| (a - b).abs() <= 0.01f32.max(a.abs() * 1e-3);
        assert!(close(seeded.position, 0.25));
        assert!(close(seeded.heel_hz, 85.0));
        assert!(close(seeded.toe_hz, 2_200.0));
        assert!(close(seeded.q, 4.0));
        assert!(close(seeded.taper, -0.2));
        assert!(close(seeded.gain_db, 3.5));
        // A knob never moved keeps the parameter's own default: the file
        // records moves, it does not restate the plugin.
        assert_eq!(seeded.env_amount, defaults.env_amount);
        // A hand-edited file cannot push a parameter off its range; the
        // round-trip clamps the way a host's write would have.
        seed_standalone_params(
            &plugin.shared,
            &plugin.params,
            &settings::WahPreferences {
                out_db: Some(900.0),
                ..Default::default()
            },
        );
        assert_eq!(plugin.shared.out_db.load(), 12.0);
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
        let (energy, peak) = block_stats(&scratch, 2, dsp::BLOCK);
        assert!((energy - (dsp::BLOCK as f64 * (0.25 + 0.0625))).abs() < 1e-9);
        assert_eq!(peak, 0.5);
        // A partial tail block reads only its first `n` samples; whatever
        // the previous block left behind in the scratch is not this block's
        // business.
        scratch[0][0] = 1.0;
        let (energy, peak) = block_stats(&scratch, 1, 0);
        assert_eq!((energy, peak), (0.0, 0.0));
    }

    #[test]
    fn every_voice_fits_the_parameters_that_carry_it() {
        // A preset button writes the parameters, so a voice whose numbers
        // the parameter ranges refuse would write something else instead.
        for voice in dsp::VOICES {
            let mut controls = dsp::Controls::from(voice);
            controls.position = 0.5;
            let params = WahParams {
                heel_hz: FloatParam::new(
                    "",
                    voice.heel_hz,
                    FloatRange::Linear {
                        min: 40.0,
                        max: 1_000.0,
                    },
                ),
                toe_hz: FloatParam::new(
                    "",
                    voice.toe_hz,
                    FloatRange::Linear {
                        min: 1_500.0,
                        max: 12_000.0,
                    },
                ),
                resonance: FloatParam::new(
                    "",
                    voice.q,
                    FloatRange::Linear {
                        min: 0.7,
                        max: 12.0,
                    },
                ),
                taper: FloatParam::new(
                    "",
                    voice.taper,
                    FloatRange::Linear {
                        min: -1.0,
                        max: 1.0,
                    },
                ),
                out_db: FloatParam::new(
                    "",
                    voice.trim_db,
                    FloatRange::Linear {
                        min: -12.0,
                        max: 12.0,
                    },
                ),
                ..WahParams::default()
            };
            let read = params.controls();
            assert_eq!(read.heel_hz, controls.heel_hz, "{} heel", voice.name);
            assert_eq!(read.toe_hz, controls.toe_hz, "{} toe", voice.name);
            assert_eq!(read.q, voice.q, "{} resonance", voice.name);
            assert_eq!(read.taper, voice.taper, "{} taper", voice.name);
            assert_eq!(read.gain_db, voice.trim_db, "{} trim", voice.name);
        }
    }
}
