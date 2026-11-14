//! Nice Echo: a Roland RE-201 Space Echo tape-delay model as a CLAP/VST3
//! plugin, with a frost editor. *Nice Echo* is this crate's own name; the
//! Roland RE-201 Space Echo is named only as the machine it models, and
//! `dsp`'s module documentation records which facts about that machine came
//! from the sources and which — A-1 through A-10 — are this model's named
//! assumptions.
//!
//! This file is the interface to the host: the parameters a host can automate,
//! a shared view of the metering for the editor, the two plugin-API identities,
//! and the block loop that hands samples to [`dsp`]. The transport snapshots
//! these controls once a block and runs every sample in between against that
//! snapshot, so what a host can say and what the tape is doing never meet in
//! the middle of a block.
//!
//! The control set is the machine's own, as the retained sources describe it:
//! Repeat Rate, Intensity, Echo Volume, Reverb Volume, Treble, Bass, the
//! twelve-position Mode dial (three tape heads and the spring tank in the
//! combinations A-4), the Echo/Normal switch, and the output-level selector.
//! The mode dial carries the machine's panel numbering — positions "1" to
//! "12" — while the integers below it, the parameter ids, and
//! [`dsp::mode_mask`] all run from zero; panel position is internal integer
//! plus one, and a test pins that relation.
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
use editor::EchoEditor;
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
/// nice-wah, nice-nam, nice-tuner and nice-pi-face: one codebase, and only
/// the question "who owns the numbers" changes.
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
pub struct EchoSharedState {
    pub input_rms: AtomicCell<f32>,
    pub input_peak: AtomicCell<f32>,
    pub output_rms: AtomicCell<f32>,
    pub output_peak: AtomicCell<f32>,
    pub meter_updated: AtomicU64,
    /// The parameters as the two threads last saw them. In a host `process()`
    /// mirrors these from the parameters each block, so they say what the
    /// transport ran on. In standalone the mirror is skipped and these cells
    /// are the parameters' truth: nice-plug gives a parameter no public
    /// setter, so a hostless app has no other door to write through, and the
    /// view writes these cells directly.
    pub mode: AtomicCell<i32>,
    pub repeat_rate: AtomicCell<f32>,
    pub intensity: AtomicCell<f32>,
    pub echo_volume: AtomicCell<f32>,
    pub reverb_volume: AtomicCell<f32>,
    pub treble: AtomicCell<f32>,
    pub bass: AtomicCell<f32>,
    pub echo_switch: AtomicCell<i32>,
    pub output_level: AtomicCell<i32>,
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

impl EchoSharedState {
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
            mode: self.mode.load(),
            repeat_rate: self.repeat_rate.load(),
            intensity: self.intensity.load(),
            echo_volume: self.echo_volume.load(),
            reverb_volume: self.reverb_volume.load(),
            treble: self.treble.load(),
            bass: self.bass.load(),
            echo_switch: self.echo_switch.load(),
            output_level: self.output_level.load(),
        }
    }

    /// The other direction: publish what the host's parameters say, so the
    /// cells agree with what the transport is about to run on.
    fn publish(&self, controls: &dsp::Controls) {
        self.mode.store(controls.mode);
        self.repeat_rate.store(controls.repeat_rate);
        self.intensity.store(controls.intensity);
        self.echo_volume.store(controls.echo_volume);
        self.reverb_volume.store(controls.reverb_volume);
        self.treble.store(controls.treble);
        self.bass.store(controls.bass);
        self.echo_switch.store(controls.echo_switch);
        self.output_level.store(controls.output_level);
    }
}

/// A switch's position names, in the order its integers run.
type Labels = &'static [&'static str];
/// The two conversions that let a host show a switch as its positions instead
/// of as an integer: a value to its label, and a typed label back to a value.
type ValueToLabel = Arc<dyn Fn(i32) -> String + Send + Sync>;
type LabelToValue = Arc<dyn Fn(&str) -> Option<i32> + Send + Sync>;

/// A switch the host sees as a stepped integer with names on its positions.
///
/// The original's switching is physical — MODE is a twelve-way rotary, the
/// Echo switch a two-way toggle — and a host's generic UI should show that,
/// not a bare number. One helper for every such control means the labels, the
/// parsing of a typed label, and the range all come from the same list, so a
/// position cannot exist in one place and not the others.
/// A typed position name read back as the integer underneath it, against a
/// label list. `stepped()` registers this very function with the host, so a
/// test that asks "what does typing 7 mean" gets the host's answer, not a
/// second copy of the rule.
fn parse_position(labels: Labels, low: i32, text: &str) -> Option<i32> {
    let text = text.trim();
    labels
        .iter()
        .position(|label| label.eq_ignore_ascii_case(text))
        .map(|index| low + index as i32)
}

fn stepped(name: &str, low: i32, default: i32, labels: Labels) -> IntParam {
    let high = low + labels.len() as i32 - 1;
    let to_text: ValueToLabel = Arc::new(move |value| {
        labels
            .get((value - low) as usize)
            .copied()
            .unwrap_or("?")
            .to_string()
    });
    let to_value: LabelToValue = Arc::new(move |text: &str| parse_position(labels, low, text));
    IntParam::new(
        name,
        default,
        IntRange::Linear {
            min: low,
            max: high,
        },
    )
    .with_value_to_string(to_text)
    .with_string_to_value(to_value)
}

/// The Mode dial's names: the machine's own panel numbers, printed 1 to 12,
/// while the integers underneath run 0 to 11. The retained sources say which
/// heads and the spring each panel position means (the RBMA walk-through and
/// this crate's assumption A-4 record the split); the numbers here are what a
/// player reads off the machine.
const MODE_LABELS: [&str; dsp::MODES] = [
    "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12",
];
/// The Echo/Normal toggle. NORMAL runs dry and echo together; ECHO is the
/// 100 %-echo position the RBMA walk-through describes. **A-7** records which
/// integer means which.
const ECHO_SWITCH_LABELS: [&str; 2] = ["Normal", "Echo"];
/// The output-level selector, named for the machine's three output jacks.
/// The service manual states the level each jack is trimmed for;
/// [`dsp::OUTPUT_TRIM_DB`] carries the resulting steps.
const OUTPUT_LEVEL_LABELS: [&str; 3] = ["H", "M", "L"];

/// The parameters, each exactly as the host sees it. Floats for the
/// continuous controls, stepped integers for what the original builds as
/// rotary and toggle switches.
#[derive(Params)]
pub struct EchoParams {
    /// The Mode dial: which heads and the spring [`dsp::mode_mask`] selects.
    /// Integer 0 is panel position "1".
    #[id = "mode"]
    pub mode: IntParam,
    /// Repeat Rate: tape speed across the A-3 range, 0 = slowest.
    #[id = "repeat_rate"]
    pub repeat_rate: FloatParam,
    /// Intensity: repeat count, crossing unity loop gain near three-quarters
    /// of its travel (A-10).
    #[id = "intensity"]
    pub intensity: FloatParam,
    /// Echo Volume: how much echo in the output.
    #[id = "echo_volume"]
    pub echo_volume: FloatParam,
    /// Reverb Volume: how much spring, working on the mode positions that
    /// have the spring in.
    #[id = "reverb_volume"]
    pub reverb_volume: FloatParam,
    /// Treble, one of the two tone knobs; centred is passthrough.
    #[id = "treble"]
    pub treble: FloatParam,
    /// Bass, the other tone knob; centred is passthrough.
    #[id = "bass"]
    pub bass: FloatParam,
    /// The Echo/Normal toggle: 0 = NORMAL, 1 = ECHO (dry gone).
    #[id = "echo_switch"]
    pub echo_switch: IntParam,
    /// The output-level selector: H, M, L, the service manual's jacks.
    #[id = "output_level"]
    pub output_level: IntParam,
}

impl Default for EchoParams {
    /// Where the controls sit on a fresh instance: the widest mode the dial
    /// has, mode "11" — all three heads and the spring, a position the RBMA
    /// walk-through lists among those where the Reverb Volume works, so the
    /// factory sound has both effects in it and both knobs audible.
    ///
    /// The Intensity default sits under A-10's unity crossover on purpose:
    /// the walk-through says the higher the Intensity, the more repeats,
    /// until the machine self-oscillates — a first listen should hear the
    /// repeats decay, and the top quarter of the knob is the howling quarter
    /// for whoever wants it. Both tone knobs centre, which this model reads
    /// as passthrough (**A-7**'s companion in `dsp`), and the Echo switch
    /// starts on NORMAL — dry and echo together — because the machine's
    /// normal playing position is not 100 % wet.
    ///
    /// The Repeat Rate default is the weakest number here and says so: the
    /// sources give the machine's tape speeds and this knob's direction of
    /// travel, never a taper (that is A-3), so 0.45 of a linear span from
    /// fastest to slowest is this model's reading of "medium", rounded to
    /// what the evidence supports.
    fn default() -> Self {
        let knob =
            |name, value| FloatParam::new(name, value, FloatRange::Linear { min: 0.0, max: 1.0 });
        Self {
            mode: stepped("Mode", 0, 10, &MODE_LABELS),
            repeat_rate: knob("Repeat Rate", 0.45),
            intensity: knob("Intensity", 0.4),
            echo_volume: knob("Echo Volume", 0.5),
            reverb_volume: knob("Reverb Volume", 0.3),
            treble: knob("Treble", 0.5),
            bass: knob("Bass", 0.5),
            echo_switch: stepped("Echo Switch", 0, 0, &ECHO_SWITCH_LABELS),
            output_level: stepped("Output Level", 0, 0, &OUTPUT_LEVEL_LABELS),
        }
    }
}

impl EchoParams {
    /// The DSP's reading of the parameter set. Called at the top of each
    /// block: parameter reads are atomic loads, and taking the whole set at
    /// once is what lets the transport's sample loop touch no atomics at all.
    fn controls(&self) -> dsp::Controls {
        dsp::Controls {
            mode: self.mode.value(),
            repeat_rate: self.repeat_rate.value(),
            intensity: self.intensity.value(),
            echo_volume: self.echo_volume.value(),
            reverb_volume: self.reverb_volume.value(),
            treble: self.treble.value(),
            bass: self.bass.value(),
            echo_switch: self.echo_switch.value(),
            output_level: self.output_level.value(),
        }
    }
}

/// The metering's two questions to one block of scratch: total energy, and
/// the loudest sample. Over the channel dimension first and the sample
/// dimension second — nice-wah's comment carries the reason this is a
/// function with a test rather than four expressions inlined twice: a mixup
/// of those two crashed the metering inside a real host while every in-crate
/// test stayed green.
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

pub struct EchoPlugin {
    params: Arc<EchoParams>,
    shared: Arc<EchoSharedState>,
    echo: dsp::Echo,
}

/// Seed the shared cells from the saved standalone preferences. A `None`
/// preference keeps the parameter's own default, so the settings file stays a
/// record of moves rather than a snapshot of everything; a value outside the
/// parameter's range is clamped exactly as a host's write would be, because a
/// hand-edited RON is a user error, not a license to feed the transport
/// nonsense.
#[cfg(feature = "standalone")]
pub(crate) fn seed_standalone_params(
    shared: &EchoSharedState,
    params: &EchoParams,
    prefs: &settings::EchoPreferences,
) {
    let seed = |cell: &AtomicCell<f32>, param: &FloatParam, saved: Option<f32>| {
        if let Some(value) = saved {
            // The normalize round-trip is the clamp the host would have
            // applied, on whatever curve the parameter uses.
            let t = param.range().normalize(value).clamp(0.0, 1.0);
            cell.store(param.range().unnormalize(t));
        }
    };
    let seed_stepped = |cell: &AtomicCell<i32>, param: &IntParam, saved: Option<i32>| {
        if let Some(value) = saved {
            let IntRange::Linear { min, max } = param.range() else {
                return;
            };
            cell.store(value.clamp(min, max));
        }
    };
    seed_stepped(&shared.mode, &params.mode, prefs.mode);
    seed(&shared.repeat_rate, &params.repeat_rate, prefs.repeat_rate);
    seed(&shared.intensity, &params.intensity, prefs.intensity);
    seed(&shared.echo_volume, &params.echo_volume, prefs.echo_volume);
    seed(
        &shared.reverb_volume,
        &params.reverb_volume,
        prefs.reverb_volume,
    );
    seed(&shared.treble, &params.treble, prefs.treble);
    seed(&shared.bass, &params.bass, prefs.bass);
    seed_stepped(&shared.echo_switch, &params.echo_switch, prefs.echo_switch);
    seed_stepped(
        &shared.output_level,
        &params.output_level,
        prefs.output_level,
    );
}

impl Default for EchoPlugin {
    fn default() -> Self {
        let params = Arc::new(EchoParams::default());
        let shared = Arc::new(EchoSharedState {
            input_rms: AtomicCell::new(0.0),
            input_peak: AtomicCell::new(0.0),
            output_rms: AtomicCell::new(0.0),
            output_peak: AtomicCell::new(0.0),
            meter_updated: AtomicU64::new(0),
            mode: AtomicCell::new(params.mode.value()),
            repeat_rate: AtomicCell::new(params.repeat_rate.value()),
            intensity: AtomicCell::new(params.intensity.value()),
            echo_volume: AtomicCell::new(params.echo_volume.value()),
            reverb_volume: AtomicCell::new(params.reverb_volume.value()),
            treble: AtomicCell::new(params.treble.value()),
            bass: AtomicCell::new(params.bass.value()),
            echo_switch: AtomicCell::new(params.echo_switch.value()),
            output_level: AtomicCell::new(params.output_level.value()),
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
                eprintln!("nice-echo: {warning}");
            }
            seed_standalone_params(&shared, &params, &settings.preferences);
        }
        // The rate arrives with `activate`; until then the transport idles at
        // a rate any record could have been made at, and it is re-armed
        // there.
        let echo = dsp::Echo::new(48_000.0, &shared.controls());
        Self {
            params,
            shared,
            echo,
        }
    }
}

impl Plugin for EchoPlugin {
    const NAME: &'static str = "Nice Echo";
    const VENDOR: &'static str = "frost developers";
    const URL: &'static str = "https://github.com/perlindgren/frost";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// Stereo first, because the machine was played through two outputs and
    /// players expect a stereo lane pair even when the echo itself is one
    /// tape (v1 duplicates its lanes; the spread is a TODO.md debt, honestly
    /// declared); mono for the hosts that only offer an insert that way.
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

    type Editor = EchoEditor;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _: AsyncExecutor<Self>) -> Option<Self::Editor> {
        Some(EchoEditor {
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
        self.echo.set_rate(config.sample_rate);
        true
    }

    fn reset(&mut self) {
        // Re-arming at the current rate is this transport's whole state
        // reset: the loop, the heads, the tank and the filters all come back
        // empty, which is what a tape transport that never ran looks like.
        let rate = self.echo.rate();
        self.echo.set_rate(rate);
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
                self.echo
                    .process_mono(&controls, &input[0][..n], &mut output[0][..n]);
            } else {
                let (left, right) = output.split_at_mut(1);
                self.echo.process_stereo(
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

impl ClapPlugin for EchoPlugin {
    const CLAP_ID: &'static str = "com.frost.nice-echo";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Tape echo with a spring tank, modelled on the Roland RE-201 Space Echo");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Delay,
        ClapFeature::Reverb,
    ];
}

impl Vst3Plugin for EchoPlugin {
    /// Sixteen bytes, exactly, and different from every other member's: two
    /// plugins claiming one class id is a host that loads the wrong one.
    const VST3_CLASS_ID: [u8; 16] = *b"FrostNiceEcho001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Delay,
        Vst3SubCategory::Reverb,
    ];
}

// The two plugin entry points, unconditional: the same `cdylib` answers for
// both formats, and `standalone` is a separate concern that brings the device
// transport with it — ASIO through azo on Windows, cpal's hosts and JACK
// elsewhere. A plain comment rather than a doc comment:
// rustdoc has nothing to attach here, and these macros do not carry docs
// through.
nice_export_clap!(EchoPlugin);
nice_export_vst3!(EchoPlugin);

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
        let plugin = EchoPlugin::default();
        let map = plugin.params().param_map();
        let mut ids: Vec<&str> = map.iter().map(|(id, _, _)| id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            [
                "bass",
                "echo_switch",
                "echo_volume",
                "intensity",
                "mode",
                "output_level",
                "repeat_rate",
                "reverb_volume",
                "treble"
            ]
        );
    }

    #[test]
    fn every_switch_offers_the_positions_it_claims_to() {
        // The label lists are what the editor draws and what a host parses a
        // typed value against. A range one narrower than the labels would let
        // a position exist on screen and nowhere in the parameter.
        let params = EchoParams::default();
        let positions = |param: &IntParam| {
            let IntRange::Linear { min, max } = param.range() else {
                panic!("switches are linear integer ranges");
            };
            (max - min + 1) as usize
        };
        assert_eq!(positions(&params.mode), dsp::MODES);
        assert_eq!(positions(&params.echo_switch), ECHO_SWITCH_LABELS.len());
        assert_eq!(positions(&params.output_level), OUTPUT_LEVEL_LABELS.len());
        // And each default is a position the labels actually have.
        for (param, labels) in [
            (&params.mode, &MODE_LABELS[..]),
            (&params.echo_switch, &ECHO_SWITCH_LABELS[..]),
            (&params.output_level, &OUTPUT_LEVEL_LABELS[..]),
        ] {
            let IntRange::Linear { min, max } = param.range() else {
                unreachable!()
            };
            let value = param.value();
            assert!(
                (min..=max).contains(&value),
                "{} defaults outside its range",
                param.name()
            );
            assert!(!labels[(value - min) as usize].is_empty());
        }
    }

    #[test]
    fn the_mode_dial_shows_the_panels_numbers() {
        // The dial prints 1 to 12; the integers run 0 to 11. A host showing
        // "0" or a user typing "13" would be the model arguing with the
        // machine, so the labels carry the panel's numbering and this test
        // pins the off-by-one that connects them.
        let params = EchoParams::default();
        let IntRange::Linear { min, max } = params.mode.range() else {
            unreachable!()
        };
        assert_eq!((min, max), (0, dsp::MODES as i32 - 1));
        for value in min..=max {
            assert_eq!(
                MODE_LABELS[value as usize].parse::<i32>().unwrap(),
                value + 1,
                "panel number must be integer plus one"
            );
        }
        // A typed panel number comes back as the integer underneath it,
        // through the same parse the registered conversion uses.
        let to_value = parse_position(&MODE_LABELS, 0, "7").expect("a panel number parses");
        assert_eq!(to_value, 6);
        assert_eq!(parse_position(&MODE_LABELS, 0, "13"), None);
    }

    #[test]
    fn a_switch_shows_the_name_of_the_position_it_is_on() {
        // Hosts print a parameter's Display and let a user type a value back
        // in, so the names on the switches are part of what a host sees.
        let params = EchoParams::default();
        assert_eq!(format!("{}", params.mode), "11");
        assert_eq!(format!("{}", params.echo_switch), "Normal");
        assert_eq!(format!("{}", params.output_level), "H");
        // Names are unique within a control, or a typed value would be a
        // coin toss between two positions.
        for labels in [
            &MODE_LABELS[..],
            &ECHO_SWITCH_LABELS[..],
            &OUTPUT_LEVEL_LABELS[..],
        ] {
            for (i, a) in labels.iter().enumerate() {
                for b in labels[i + 1..].iter() {
                    assert_ne!(a.to_lowercase(), b.to_lowercase(), "{a} vs {b}");
                }
            }
        }
    }

    #[test]
    fn the_defaults_are_a_configuration_the_sources_describe() {
        // Mode "11": all three heads and the spring — the table's widest
        // combination, and one of the positions the RBMA walk-through says
        // the Reverb Volume works on, so both effects and both their knobs
        // are live on a fresh instance.
        let params = EchoParams::default();
        let controls = params.controls();
        assert_eq!(controls.mode, 10);
        assert_eq!(format!("{}", params.mode), "11");
        assert_eq!(dsp::mode_mask(controls.mode), 0b1111);
        // Intensity sits under A-10's unity crossover: the factory sound
        // decays, and the howling quarter stays for whoever reaches for it.
        assert!(
            dsp::feedback_gain(controls.intensity) < 1.0,
            "factory Intensity must not self-oscillate"
        );
        // Both tone knobs centre — passthrough — and the Echo switch starts
        // on NORMAL, dry and echo together (A-7).
        assert_eq!(params.treble.value(), params.bass.value());
        assert_eq!(controls.treble, 0.5);
        assert_eq!(controls.bass, 0.5);
        assert_eq!(controls.echo_switch, 0);
        // The output selector starts on H, the service manual's hottest jack.
        assert_eq!(controls.output_level, 0);
        // The whole set is what `dsp` itself calls the resting place; if
        // these two ever drift, the plugin and the model disagree about the
        // factory sound and neither says so.
        assert_eq!(controls.mode, dsp::Controls::default().mode);
        assert_eq!(controls.repeat_rate, dsp::Controls::default().repeat_rate);
        assert_eq!(controls.intensity, dsp::Controls::default().intensity);
    }

    #[test]
    fn publishing_leaves_the_cells_saying_what_the_transport_runs_on() {
        // The host half of the deal: `process()` publishes the parameters
        // before reading the cells, so the DSP and the editor answer to one
        // set of numbers even while a host automates them mid-block.
        let plugin = EchoPlugin::default();
        let mut controls = plugin.params.controls();
        controls.mode = 3;
        controls.intensity = 0.9;
        controls.output_level = 2;
        plugin.shared.publish(&controls);
        let read = plugin.shared.controls();
        assert_eq!(read.mode, 3);
        assert_eq!(read.intensity, 0.9);
        assert_eq!(read.output_level, 2);
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn saved_moves_seed_the_cells_and_silence_keeps_the_defaults() {
        let plugin = EchoPlugin::default();
        let defaults = plugin.shared.controls();
        seed_standalone_params(
            &plugin.shared,
            &plugin.params,
            &settings::EchoPreferences {
                mode: Some(4),
                repeat_rate: Some(0.8),
                intensity: Some(0.9),
                echo_switch: Some(1),
                ..Default::default()
            },
        );
        let seeded = plugin.shared.controls();
        // Tolerance, not identity: seeding walks the parameter's own
        // normalize/unnormalize round-trip, and a value need not come back
        // from it bit for bit.
        let close = |a: f32, b: f32| (a - b).abs() <= 0.01f32.max(a.abs() * 1e-3);
        assert!(close(seeded.repeat_rate, 0.8));
        assert!(close(seeded.intensity, 0.9));
        assert_eq!(seeded.mode, 4);
        assert_eq!(seeded.echo_switch, 1);
        // A control never moved keeps the parameter's own default: the file
        // records moves, it does not restate the plugin.
        assert_eq!(seeded.bass, defaults.bass);
        assert_eq!(seeded.output_level, defaults.output_level);
        // A hand-edited file cannot push a parameter off its range; the clamp
        // is the one a host's write would have applied.
        seed_standalone_params(
            &plugin.shared,
            &plugin.params,
            &settings::EchoPreferences {
                repeat_rate: Some(4.0),
                mode: Some(19),
                output_level: Some(7),
                ..Default::default()
            },
        );
        assert_eq!(plugin.shared.repeat_rate.load(), 1.0);
        // Position 20 of a twelve-position dial does not exist; the clamp
        // keeps it on the control's range, and the mask it lands on is one
        // the mode table actually has.
        assert_eq!(plugin.shared.mode.load(), dsp::MODES as i32 - 1);
        assert_eq!(plugin.shared.output_level.load(), 2);
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
        let (energy, peak) = block_stats(&scratch, 2, dsp::BLOCK);
        assert!((energy - (dsp::BLOCK as f64 * (0.25 + 0.0625))).abs() < 1e-9);
        assert_eq!(peak, 0.5);
        // A partial tail block reads only its first `n` samples; whatever the
        // previous block left behind in the scratch is not this block's
        // business.
        scratch[0][0] = 1.0;
        let (energy, peak) = block_stats(&scratch, 1, 0);
        assert_eq!((energy, peak), (0.0, 0.0));
    }
}
