//! Nice Pi-Fase: a Mu-Tron Bi-Phase model as a CLAP/VST3 plugin, with a frost
//! editor. The name follows the user's spelling — *Pi-Fase* — so this touches
//! no trademark; what the plugin models is the documented machine, and
//! `dsp`'s module documentation records what that means and what it does not.
//!
//! This file is the interface to the host: the parameters a host can automate,
//! a shared view of the metering for the editor, and the two plugin-API
//! identities. The signal path is in [`dsp`], and until it lands, the plugin
//! passes audio through — the chassis is real, the filter is not.
//!
//! The control set is the original's, taken from its owner's manual: two sweep
//! generators (Rate, Shape, and on generator 1 the Man/Ped rate source), and
//! two phasors with Depth, Feedback, and their own switching — phasor B adding
//! Sweep Sync and its input select. There is no Mode control on a Bi-Phase: the
//! machine's three basic modes of operation *are* the three positions of the
//! Phasor B Input switch, so [`dsp::Routing`] is a reading of `b_input` and not
//! a parameter of its own.
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
use editor::PiFaceEditor;
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
/// nice-wah, nice-nam and nice-tuner: one codebase, and only the question
/// "who owns the numbers" changes.
pub static STANDALONE_MODE: AtomicBool = AtomicBool::new(false);

const MIN_SIZE: LogicalSize<f32> = LogicalSize::new(620.0, 400.0);
const DEFAULT_SIZE: LogicalSize<f32> = LogicalSize::new(900.0, 520.0);
const RESIZE_HINT: ResizeHint = ResizeHint::resizable().with_min_logical_size(MIN_SIZE);

use nice_plug::prelude::*;

/// What the audio thread publishes for the editor, and nothing more: in a host
/// the editor reads and writes the parameters themselves through the host's
/// parameter API, so only the numbers born inside `process()` need a crossing.
/// Every field is an atomic cell of a single value — the audio thread must
/// never block behind the UI, and a torn meter reading is not worth a lock.
pub struct PiFaceSharedState {
    pub input_rms: AtomicCell<f32>,
    pub input_peak: AtomicCell<f32>,
    pub output_rms: AtomicCell<f32>,
    pub output_peak: AtomicCell<f32>,
    /// The mode the filter ran on last block is not published separately: the
    /// machine's three modes are the three positions of the Phasor B Input
    /// switch, and that value travels in `b_input` below like any other control.
    pub meter_updated: AtomicU64,
    /// The parameters as the two threads last saw them. In a host `process()`
    /// mirrors these from the parameters each block, so they say what the
    /// filter actually ran on. In standalone the mirror is skipped and these
    /// cells are the parameters' truth: nice-plug gives a parameter no public
    /// setter, so a hostless app has no other door to write through, and the
    /// view writes these cells directly.
    pub gen1_rate: AtomicCell<f32>,
    pub gen1_shape: AtomicCell<i32>,
    pub gen1_rate_source: AtomicCell<i32>,
    pub gen2_rate: AtomicCell<f32>,
    pub gen2_shape: AtomicCell<i32>,
    pub a_depth: AtomicCell<f32>,
    pub a_feedback: AtomicCell<f32>,
    pub a_sweep: AtomicCell<i32>,
    pub b_depth: AtomicCell<f32>,
    pub b_feedback: AtomicCell<f32>,
    pub b_sync: AtomicCell<i32>,
    pub b_sweep: AtomicCell<i32>,
    pub b_input: AtomicCell<i32>,
    pub pedal: AtomicCell<f32>,
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

impl PiFaceSharedState {
    fn logical_size(&self) -> LogicalSize<f32> {
        self.logical_size.load()
    }
    fn scale_factor(&self) -> f64 {
        self.scale_factor.load()
    }

    /// The DSP's reading of the cells. In standalone this is the parameter set;
    /// in a host it is last block's mirror, one audio block stale at the very
    /// worst.
    fn controls(&self) -> dsp::Controls {
        dsp::Controls {
            gen1_rate: self.gen1_rate.load(),
            gen1_shape: self.gen1_shape.load(),
            gen1_rate_source: self.gen1_rate_source.load(),
            gen2_rate: self.gen2_rate.load(),
            gen2_shape: self.gen2_shape.load(),
            a_depth: self.a_depth.load(),
            a_feedback: self.a_feedback.load(),
            a_sweep: self.a_sweep.load(),
            b_depth: self.b_depth.load(),
            b_feedback: self.b_feedback.load(),
            b_sync: self.b_sync.load(),
            b_sweep: self.b_sweep.load(),
            b_input: self.b_input.load(),
            pedal: self.pedal.load(),
        }
    }

    /// The other direction: publish what the host's parameters say, so the
    /// cells agree with what the filter is about to run on.
    fn publish(&self, controls: &dsp::Controls) {
        self.gen1_rate.store(controls.gen1_rate);
        self.gen1_shape.store(controls.gen1_shape);
        self.gen1_rate_source.store(controls.gen1_rate_source);
        self.gen2_rate.store(controls.gen2_rate);
        self.gen2_shape.store(controls.gen2_shape);
        self.a_depth.store(controls.a_depth);
        self.a_feedback.store(controls.a_feedback);
        self.a_sweep.store(controls.a_sweep);
        self.b_depth.store(controls.b_depth);
        self.b_feedback.store(controls.b_feedback);
        self.b_sync.store(controls.b_sync);
        self.b_sweep.store(controls.b_sweep);
        self.b_input.store(controls.b_input);
        self.pedal.store(controls.pedal);
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
/// The original's switching is physical — SHAPE is a two-way switch, SWEEP
/// SYNC is NORM/REV — and a host's generic UI should show that, not a number.
/// One helper for every such control means the labels, the parsing of a typed
/// label, and the range all come from the same list, so a position cannot
/// exist in one place and not the others.
fn stepped(name: &str, low: i32, default: i32, labels: Labels) -> IntParam {
    let high = low + labels.len() as i32 - 1;
    let to_text: ValueToLabel = Arc::new(move |value| {
        labels
            .get((value - low) as usize)
            .copied()
            .unwrap_or("?")
            .to_string()
    });
    let to_value: LabelToValue = Arc::new(move |text: &str| {
        let text = text.trim();
        labels
            .iter()
            .position(|label| label.eq_ignore_ascii_case(text))
            .map(|index| low + index as i32)
    });
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

/// The switch's position names, one pair per control, in the order the control
/// shows them. A test asserts every control's range against these, because the
/// ids and the position order are the wire format of every automation a host
/// has written.
const SHAPES: [&str; 2] = ["Sine", "Square"];
const RATE_SOURCES: [&str; 2] = ["Manual", "Pedal"];
const A_SWEEPS: [&str; 2] = ["Gen 1", "Pedal"];
const SYNC: [&str; 2] = ["Norm", "Rev"];
const B_SWEEPS: [&str; 3] = ["Gen 1", "Gen 2", "Pedal"];
/// The Phasor B Input switch, printed in this order round the panel's rotary:
/// `IN A`, `OUT A`, `IN B`. These three positions *are* the machine's three
/// basic modes of operation — the manual says the switch "effectively selects
/// one of the three basic modes of operation" — so no separate mode parameter
/// exists to disagree with them.
const B_INPUTS: [&str; 3] = ["In A", "Out A", "In B"];

/// The parameters, each exactly as the host sees it. Floats for the continuous
/// controls, stepped integers for what the original builds as switches.
#[derive(Params)]
pub struct PiFaceParams {
    /// Sweep generator 1's rate, .1–18 Hz as the specification has it.
    #[id = "gen1_rate"]
    pub gen1_rate: FloatParam,
    /// Sweep generator 1's shape, S3.
    #[id = "gen1_shape"]
    pub gen1_shape: IntParam,
    /// Sweep generator 1's rate source, S2: knob or pedal.
    #[id = "gen1_rate_source"]
    pub gen1_rate_source: IntParam,
    /// Sweep generator 2's rate, hertz.
    #[id = "gen2_rate"]
    pub gen2_rate: FloatParam,
    /// Sweep generator 2's shape, S4.
    #[id = "gen2_shape"]
    pub gen2_shape: IntParam,
    /// Phasor A's depth.
    #[id = "a_depth"]
    pub a_depth: FloatParam,
    /// Phasor A's feedback.
    #[id = "a_feedback"]
    pub a_feedback: FloatParam,
    /// Phasor A's sweep source, S5.
    #[id = "a_sweep"]
    pub a_sweep: IntParam,
    /// Phasor B's depth.
    #[id = "b_depth"]
    pub b_depth: FloatParam,
    /// Phasor B's feedback.
    #[id = "b_feedback"]
    pub b_feedback: FloatParam,
    /// Phasor B's sweep sync, S7: with it reversed, B moves against A.
    #[id = "b_sync"]
    pub b_sync: IntParam,
    /// Phasor B's sweep source, S6.
    #[id = "b_sweep"]
    pub b_sweep: IntParam,
    /// What drives phasor B: S1's input select, which is also the machine's
    /// mode selector — In A stereo, Out A twelve stages in series, In B two
    /// separate instruments. See [`dsp::Routing`].
    #[id = "b_input"]
    pub b_input: IntParam,
    /// The expression pedal, 0..=1. The hardware reads a jack (J6-2); a plugin
    /// has no jack, so this is the parameter a host automates or maps to a
    /// controller to feed the same two switches.
    #[id = "pedal"]
    pub pedal: FloatParam,
}

impl Default for PiFaceParams {
    /// Where the controls sit on a fresh instance: the owner's manual's own
    /// starting configuration, "Start with this" for one instrument into
    /// Input A, taken with the one switch change that turns it into two-speed
    /// phasing.
    ///
    /// Every switch position below is quoted, not chosen: Rate Control on Man,
    /// both Shapes on sine, Phasor A's Sweep on Gen 1, Phasor B Input on Out A
    /// (the manual's normal setting for one instrument and one amplifier), and
    /// Sweep Sync on Norm — the position its double-depth figure draws, and the
    /// one that keeps the two lanes moving together. Phasor B's Sweep on Gen 2
    /// is the manual's stated difference for two-speed phasing ("the only
    /// different setting is…"), and it is the position that makes both
    /// generators audible at once, which is the sound a first listen of this
    /// plugin should give. Choosing two-speed over double-depth as the factory
    /// state is the one judgement in this list; the switch itself is documented.
    ///
    /// The levels come from the manual's own advice too: "for a smooth-sounding
    /// sweep, both Depth controls should be set equally", "the smoothest-sounding
    /// phasing is found at about 8" of the dial's ten, and "start with both
    /// Feedback controls turned down for a pure … phasing effect".
    ///
    /// The two rates are the weakest numbers here, and say so. The manual's
    /// starter recipe is "Sweep Generator 1 at a slow speed (1 to 3 on the
    /// dial)" and "Sweep Generator 2 moderately fast (about 6)", and the dial is
    /// numbered 1 to 10 with no published mapping to hertz. Reading those as
    /// hertz needs a taper, and the taper is an assumption — an exponential
    /// sweep of the .1–18 Hz range puts dial 3 near a third of a hertz and dial 6
    /// near two. So: 0.3 and 2.0, which is what the *words* support, rounded to
    /// the precision the evidence has.
    fn default() -> Self {
        let rate = |name, value| {
            FloatParam::new(
                name,
                value,
                FloatRange::Linear {
                    min: dsp::RATE_MIN_HZ,
                    max: dsp::RATE_MAX_HZ,
                },
            )
            .with_unit(" Hz")
        };
        let depth =
            |name, value| FloatParam::new(name, value, FloatRange::Linear { min: 0.0, max: 1.0 });
        // The hardware's own top is 100%: the manual gives a headroom figure for
        // it (1 V RMS with both phasors in series) and describes what it does —
        // "the Bi-Phase will pick out the individual notes and harmonics of a
        // chord as it sweeps up and down". So the control reaches 1.0, and
        // whether the model stays well-behaved there is the filter's problem to
        // solve, not something to hide by shortening the control.
        let feedback =
            |name, value| FloatParam::new(name, value, FloatRange::Linear { min: 0.0, max: 1.0 });
        Self {
            gen1_rate: rate("Gen 1 Rate", 0.3),
            gen1_shape: stepped("Gen 1 Shape", 0, 0, &SHAPES),
            gen1_rate_source: stepped("Gen 1 Rate Src", 0, 0, &RATE_SOURCES),
            gen2_rate: rate("Gen 2 Rate", 2.0),
            gen2_shape: stepped("Gen 2 Shape", 0, 0, &SHAPES),
            a_depth: depth("A Depth", 0.8),
            a_feedback: feedback("A Feedback", 0.0),
            a_sweep: stepped("A Sweep", 0, 0, &A_SWEEPS),
            b_depth: depth("B Depth", 0.8),
            b_feedback: feedback("B Feedback", 0.0),
            b_sync: stepped("B Sync", 0, 0, &SYNC),
            // Gen 2, the two-speed variant — see the list above.
            b_sweep: stepped("B Sweep", 0, 1, &B_SWEEPS),
            // Out A: A into B, twelve stages in series.
            b_input: stepped("B Input", 0, 1, &B_INPUTS),
            pedal: FloatParam::new("Pedal", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
        }
    }
}

impl PiFaceParams {
    /// The DSP's reading of the parameter set. Called at the top of each block:
    /// parameter reads are atomic loads, and taking the whole set at once is
    /// what lets the filter's sample loop touch no atomics at all.
    fn controls(&self) -> dsp::Controls {
        dsp::Controls {
            gen1_rate: self.gen1_rate.value(),
            gen1_shape: self.gen1_shape.value(),
            gen1_rate_source: self.gen1_rate_source.value(),
            gen2_rate: self.gen2_rate.value(),
            gen2_shape: self.gen2_shape.value(),
            a_depth: self.a_depth.value(),
            a_feedback: self.a_feedback.value(),
            a_sweep: self.a_sweep.value(),
            b_depth: self.b_depth.value(),
            b_feedback: self.b_feedback.value(),
            b_sync: self.b_sync.value(),
            b_sweep: self.b_sweep.value(),
            b_input: self.b_input.value(),
            pedal: self.pedal.value(),
        }
    }
}

/// The metering's two questions to one block of scratch: total energy, and the
/// loudest sample. Over the channel dimension first and the sample dimension
/// second — nice-wah's comment carries the reason this is a function with a
/// test rather than four expressions inlined twice: a mixup of those two
/// crashed the metering inside a real host while every in-crate test stayed
/// green.
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

pub struct PiFacePlugin {
    params: Arc<PiFaceParams>,
    shared: Arc<PiFaceSharedState>,
    filter: dsp::PiFace,
}

/// Seed the shared cells from the saved standalone preferences. A `None`
/// preference keeps the parameter's own default, so the settings file stays a
/// record of moves rather than a snapshot of everything; a value outside the
/// parameter's range is clamped exactly as a host's write would be, because a
/// hand-edited RON is a user error, not a license to feed the filter nonsense.
#[cfg(feature = "standalone")]
pub(crate) fn seed_standalone_params(
    shared: &PiFaceSharedState,
    params: &PiFaceParams,
    prefs: &settings::PiFacePreferences,
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
    seed(&shared.gen1_rate, &params.gen1_rate, prefs.gen1_rate);
    seed_stepped(&shared.gen1_shape, &params.gen1_shape, prefs.gen1_shape);
    seed_stepped(
        &shared.gen1_rate_source,
        &params.gen1_rate_source,
        prefs.gen1_rate_source,
    );
    seed(&shared.gen2_rate, &params.gen2_rate, prefs.gen2_rate);
    seed_stepped(&shared.gen2_shape, &params.gen2_shape, prefs.gen2_shape);
    seed(&shared.a_depth, &params.a_depth, prefs.a_depth);
    seed(&shared.a_feedback, &params.a_feedback, prefs.a_feedback);
    seed_stepped(&shared.a_sweep, &params.a_sweep, prefs.a_sweep);
    seed(&shared.b_depth, &params.b_depth, prefs.b_depth);
    seed(&shared.b_feedback, &params.b_feedback, prefs.b_feedback);
    seed_stepped(&shared.b_sync, &params.b_sync, prefs.b_sync);
    seed_stepped(&shared.b_sweep, &params.b_sweep, prefs.b_sweep);
    seed_stepped(&shared.b_input, &params.b_input, prefs.b_input);
    seed(&shared.pedal, &params.pedal, prefs.pedal);
}

impl Default for PiFacePlugin {
    fn default() -> Self {
        let params = Arc::new(PiFaceParams::default());
        let shared = Arc::new(PiFaceSharedState {
            input_rms: AtomicCell::new(0.0),
            input_peak: AtomicCell::new(0.0),
            output_rms: AtomicCell::new(0.0),
            output_peak: AtomicCell::new(0.0),
            meter_updated: AtomicU64::new(0),
            gen1_rate: AtomicCell::new(params.gen1_rate.value()),
            gen1_shape: AtomicCell::new(params.gen1_shape.value()),
            gen1_rate_source: AtomicCell::new(params.gen1_rate_source.value()),
            gen2_rate: AtomicCell::new(params.gen2_rate.value()),
            gen2_shape: AtomicCell::new(params.gen2_shape.value()),
            a_depth: AtomicCell::new(params.a_depth.value()),
            a_feedback: AtomicCell::new(params.a_feedback.value()),
            a_sweep: AtomicCell::new(params.a_sweep.value()),
            b_depth: AtomicCell::new(params.b_depth.value()),
            b_feedback: AtomicCell::new(params.b_feedback.value()),
            b_sync: AtomicCell::new(params.b_sync.value()),
            b_sweep: AtomicCell::new(params.b_sweep.value()),
            b_input: AtomicCell::new(params.b_input.value()),
            pedal: AtomicCell::new(params.pedal.value()),
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
                eprintln!("nice-pi-face: {warning}");
            }
            seed_standalone_params(&shared, &params, &settings.preferences);
        }
        // The rate arrives with `activate`; until then the filter idles at a
        // rate any record could have been made at, and it is re-tuned there.
        let filter = dsp::PiFace::new(44_100.0, &shared.controls());
        Self {
            params,
            shared,
            filter,
        }
    }
}

impl Plugin for PiFacePlugin {
    const NAME: &'static str = "Nice Pi-Fase";
    const VENDOR: &'static str = "frost developers";
    const URL: &'static str = "https://github.com/perlindgren/frost";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// Stereo first, because the machine has two independent paths and a
    /// dual-mono or stereo lane pair is what that wiring asks for; mono for the
    /// hosts that only offer an insert that way.
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

    type Editor = PiFaceEditor;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _: AsyncExecutor<Self>) -> Option<Self::Editor> {
        Some(PiFaceEditor {
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
        // A host owns the parameters, so publish them before anything reads the
        // cells. In standalone the view owns them, and mirroring here would
        // overwrite what the settings panel just wrote.
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

impl ClapPlugin for PiFacePlugin {
    const CLAP_ID: &'static str = "com.frost.nice-pi-face";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Two six-stage lamp-swept phasor lanes, modelled on the Mu-Tron Bi-Phase");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Phaser,
        ClapFeature::Filter,
    ];
}

impl Vst3Plugin for PiFacePlugin {
    /// Sixteen bytes, exactly, and different from every other member's: two
    /// plugins claiming one class id is a host that loads the wrong one.
    const VST3_CLASS_ID: [u8; 16] = *b"FrostPiFaseFx001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Filter];
}

// The two plugin entry points, unconditional: the same `cdylib` answers for
// both formats, and `standalone` is a separate concern that brings cpal, JACK
// and the ASIO host with it. A plain comment rather than a doc comment: rustdoc
// has nothing to attach here, and these macros do not carry docs through.
nice_export_clap!(PiFacePlugin);
nice_export_vst3!(PiFacePlugin);

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
        // The parameter ids are the wire format of every automation a host will
        // ever write; renaming one silently orphans a session. This crate has
        // not shipped yet, which is why the duplicate `mode` parameter could
        // still be deleted for free — after the first release, dropping or
        // renaming an id is a migration, and this test is what makes that loud.
        let plugin = PiFacePlugin::default();
        let map = plugin.params().param_map();
        let mut ids: Vec<&str> = map.iter().map(|(id, _, _)| id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            [
                "a_depth",
                "a_feedback",
                "a_sweep",
                "b_depth",
                "b_feedback",
                "b_input",
                "b_sweep",
                "b_sync",
                "gen1_rate",
                "gen1_rate_source",
                "gen1_shape",
                "gen2_rate",
                "gen2_shape",
                "pedal"
            ]
        );
    }

    #[test]
    fn every_switch_offers_the_positions_it_claims_to() {
        // The label lists are what the editor draws and what a host parses a
        // typed value against. A range one narrower than the labels would let
        // a position exist on screen and nowhere in the parameter.
        let params = PiFaceParams::default();
        let positions = |param: &IntParam| {
            let IntRange::Linear { min, max } = param.range() else {
                panic!("switches are linear integer ranges");
            };
            (max - min + 1) as usize
        };
        assert_eq!(positions(&params.gen1_shape), SHAPES.len());
        assert_eq!(positions(&params.gen2_shape), SHAPES.len());
        assert_eq!(positions(&params.gen1_rate_source), RATE_SOURCES.len());
        assert_eq!(positions(&params.a_sweep), A_SWEEPS.len());
        assert_eq!(positions(&params.b_sync), SYNC.len());
        assert_eq!(positions(&params.b_sweep), B_SWEEPS.len());
        assert_eq!(positions(&params.b_input), B_INPUTS.len());
        // And each default is a position the labels actually have.
        for (param, labels) in [
            (&params.gen1_shape, &SHAPES[..]),
            (&params.gen2_shape, &SHAPES[..]),
            (&params.gen1_rate_source, &RATE_SOURCES[..]),
            (&params.a_sweep, &A_SWEEPS[..]),
            (&params.b_sync, &SYNC[..]),
            (&params.b_sweep, &B_SWEEPS[..]),
            (&params.b_input, &B_INPUTS[..]),
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
    fn a_switch_shows_the_name_of_the_position_it_is_on() {
        // Hosts print a parameter's Display and let a user type a value back
        // in, so the names on the switch are part of what a host sees.
        let params = PiFaceParams::default();
        assert_eq!(format!("{}", params.gen1_shape), "Sine");
        assert_eq!(format!("{}", params.gen1_rate_source), "Manual");
        assert_eq!(format!("{}", params.a_sweep), "Gen 1");
        assert_eq!(format!("{}", params.b_sync), "Norm");
        assert_eq!(format!("{}", params.b_sweep), "Gen 2");
        assert_eq!(format!("{}", params.b_input), "Out A");
        // Names are unique within a control, or a typed value would be a
        // coin toss between two positions.
        for labels in [
            &SHAPES[..],
            &RATE_SOURCES[..],
            &A_SWEEPS[..],
            &SYNC[..],
            &B_SWEEPS[..],
            &B_INPUTS[..],
        ] {
            for (i, a) in labels.iter().enumerate() {
                for b in labels[i + 1..].iter() {
                    assert_ne!(a.to_lowercase(), b.to_lowercase(), "{a} vs {b}");
                }
            }
        }
    }

    #[test]
    fn the_defaults_are_a_configuration_the_manual_describes() {
        // "Start with this", for one instrument into Input A, with the one
        // switch change the manual names for two-speed phasing. Asserted on the
        // position *names*: the integers are this crate's, the names are the
        // panel's, and a reordering that keeps the names attached is not a
        // behaviour change.
        let params = PiFaceParams::default();
        assert_eq!(format!("{}", params.gen1_rate_source), "Manual");
        assert_eq!(format!("{}", params.gen1_shape), "Sine");
        assert_eq!(format!("{}", params.gen2_shape), "Sine");
        assert_eq!(format!("{}", params.a_sweep), "Gen 1");
        assert_eq!(format!("{}", params.b_input), "Out A");
        assert_eq!(format!("{}", params.b_sync), "Norm");
        assert_eq!(format!("{}", params.b_sweep), "Gen 2");
        // The routing as the filter will read it: phasor A into phasor B, the
        // twelve stages in series the manual calls normal for one instrument
        // and one amplifier.
        let controls = params.controls();
        assert_eq!(
            dsp::Routing::from_param(controls.b_input),
            dsp::Routing::OutA
        );
        // "For a smooth-sounding sweep, both Depth controls should be set
        // equally"; "the smoothest-sounding phasing is found at about 8" of the
        // dial's ten; "start with both Feedback controls turned down".
        assert_eq!(params.a_depth.value(), params.b_depth.value());
        assert!((params.a_depth.value() - 0.8).abs() < 1e-6);
        assert_eq!(params.a_feedback.value(), 0.0);
        assert_eq!(params.b_feedback.value(), 0.0);
        // The rates are the manual's "slow speed (1 to 3 on the dial)" and
        // "moderately fast (about 6)" read through a taper it never published,
        // so what gets asserted is the relation the recipe states — generator 2
        // several times faster than generator 1, both inside the documented
        // range — rather than hertz this crate cannot claim to know.
        assert!(params.gen1_rate.value() < params.gen2_rate.value() / 3.0);
        for rate in [params.gen1_rate.value(), params.gen2_rate.value()] {
            assert!(
                (dsp::RATE_MIN_HZ..=dsp::RATE_MAX_HZ).contains(&rate),
                "{rate} Hz is outside the documented range"
            );
        }
        // The pedal sits at rest. The hardware's note about a switch left on Ped
        // with nothing plugged in is "a very low rate"; a plugin has no jack to
        // leave open, and Rate Control starts on Man anyway.
        assert_eq!(params.pedal.value(), 0.0);
    }

    #[test]
    fn publishing_leaves_the_cells_saying_what_the_filter_runs_on() {
        // The host half of the deal: `process()` publishes the parameters
        // before reading the cells, so the DSP and the editor answer to one set
        // of numbers even while a host automates them mid-block.
        let plugin = PiFacePlugin::default();
        let mut controls = plugin.params.controls();
        controls.gen2_rate = 3.5;
        controls.b_sweep = 2;
        controls.b_input = 0;
        plugin.shared.publish(&controls);
        let read = plugin.shared.controls();
        assert_eq!(read.gen2_rate, 3.5);
        assert_eq!(read.b_sweep, 2);
        assert_eq!(read.b_input, 0);
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn saved_moves_seed_the_cells_and_silence_keeps_the_defaults() {
        let plugin = PiFacePlugin::default();
        let defaults = plugin.shared.controls();
        seed_standalone_params(
            &plugin.shared,
            &plugin.params,
            &settings::PiFacePreferences {
                gen1_rate: Some(2.5),
                gen2_shape: Some(1),
                a_depth: Some(0.4),
                b_feedback: Some(0.8),
                b_sweep: Some(2),
                a_sweep: Some(1),
                ..Default::default()
            },
        );
        let seeded = plugin.shared.controls();
        // Tolerance, not identity: seeding walks the parameter's own
        // normalize/unnormalize round-trip, and a value need not come back from
        // it bit for bit.
        let close = |a: f32, b: f32| (a - b).abs() <= 0.01f32.max(a.abs() * 1e-3);
        assert!(close(seeded.gen1_rate, 2.5));
        assert!(close(seeded.a_depth, 0.4));
        assert!(close(seeded.b_feedback, 0.8));
        assert_eq!(seeded.gen2_shape, 1);
        assert_eq!(seeded.b_sweep, 2);
        assert_eq!(seeded.a_sweep, 1);
        // A control never moved keeps the parameter's own default: the file
        // records moves, it does not restate the plugin.
        assert_eq!(seeded.pedal, defaults.pedal);
        assert_eq!(seeded.b_input, defaults.b_input);
        // A hand-edited file cannot push a parameter off its range; the clamp
        // is the one a host's write would have applied.
        seed_standalone_params(
            &plugin.shared,
            &plugin.params,
            &settings::PiFacePreferences {
                gen1_rate: Some(9_000.0),
                b_input: Some(9),
                ..Default::default()
            },
        );
        assert_eq!(plugin.shared.gen1_rate.load(), dsp::RATE_MAX_HZ);
        // Position 9 of a three-position switch does not exist; the clamp keeps
        // it on the control's range, and the routing it lands on is one of the
        // machine's three.
        assert_eq!(plugin.shared.b_input.load(), 2);
        assert_eq!(
            dsp::Routing::from_param(plugin.shared.b_input.load()),
            dsp::Routing::InB
        );
    }

    /// The shape that once crashed a sibling's metering inside a real host: a
    /// full block of stereo, where `channels` is two and `n` is the whole
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

    #[test]
    fn the_sweep_rates_the_controls_carry_stay_in_the_documented_range() {
        for rate in [dsp::RATE_MIN_HZ, 1.0, 9.0, dsp::RATE_MAX_HZ] {
            let params = PiFaceParams {
                gen1_rate: FloatParam::new(
                    "",
                    rate,
                    FloatRange::Linear {
                        min: dsp::RATE_MIN_HZ,
                        max: dsp::RATE_MAX_HZ,
                    },
                ),
                ..PiFaceParams::default()
            };
            assert_eq!(params.controls().gen1_rate, rate, "{rate} Hz");
        }
    }
}
