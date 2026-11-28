//! Nice Eq: a seven-band graphic equaliser as a CLAP/VST3 plugin, with a frost
//! editor and a spectrum analyser under the faders. *Nice Eq* is this crate's
//! own name; the first mode is an independent model of the band ladder of the
//! 1987 Boss GE-7, named only as the machine that inspired it, and [`dsp`]'s
//! assumption records — A-1 through A-11 — say which facts about that machine
//! came from the retained sources and which are this model's own choices.
//!
//! This file is the interface to the host: the parameters a host can automate,
//! a shared view of the metering and the analyser feed for the editor, the two
//! plugin-API identities, and the block loop that hands samples to [`dsp`].
//!
//! Two things about that loop are worth saying here rather than leaving to the
//! code. The equaliser is mono in and mono out, one signal through seven
//! sections, because the box its layout comes from has one input jack and
//! because what a stereo bus does with a mono processor — two instances, a
//! duplicated send, a downmix made before it — is the host's decision and not
//! one to be taken from it here. And the analyser is fed, not run: `process()`
//! copies the watched end into a preallocated lock-free ring, and every
//! transform happens on the editor's thread — where a 32 768-point window may
//! take as long as it likes without making anybody's audio late. When the view
//! says `off`, nothing is copied and nothing is counted: the cheapest analyser
//! is the one that is not asked for.

#![deny(unsafe_code)]

mod dsp;
mod editor;
#[cfg(feature = "standalone")]
pub mod settings;
mod spectrum;
#[cfg(feature = "standalone")]
pub mod standalone;
mod ui;

use crossbeam::atomic::AtomicCell;
use editor::EqEditor;
use nice_common::meter::block_stats;
use nice_common::true_peak::TruePeak;
use nice_plug::editor::dpi::LogicalSize;
use rtrb::{Consumer, Producer, RingBuffer};
use std::num::NonZeroU32;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};

#[cfg(feature = "standalone")]
pub use nice_common::devices;

/// Set once by the standalone binary before the plugin is built, and read on
/// every side of the boundary ever after. nice-plug's parameters have no
/// public setter — only a host's `GuiContext` may write one — so without a
/// host the shared cells below are the parameters' truth, and the places that
/// would otherwise defer to a host ask this flag instead. Same arrangement as
/// nice-wah, nice-echo, nice-nam, nice-tuner and nice-pi-face: one codebase,
/// and only the question "who owns the numbers" changes.
pub static STANDALONE_MODE: AtomicBool = AtomicBool::new(false);

/// Faders in a column, analyser under them: the panel needs room for seven
/// travels and a decade-and-a-half of axis, so it starts wider than tall.
const MIN_SIZE: LogicalSize<f32> = LogicalSize::new(640.0, 460.0);
const DEFAULT_SIZE: LogicalSize<f32> = LogicalSize::new(900.0, 560.0);
const RESIZE_HINT: ResizeHint = ResizeHint::resizable().with_min_logical_size(MIN_SIZE);

/// Samples per analyser tap. Three quarters of a second at 44.1 kHz: as long as
/// the longest window the view offers, so the biggest transform never runs short,
/// and short enough that a stalled editor costs a paused picture rather than a
/// growing pile of memory.
const RING_CAPACITY: usize = 1 << 15;

use nice_plug::prelude::*;

/// Which of the picture's settings the view is asking for, as one value that
/// can be read and published in a single breath. The four are stored as the
/// integers the host sees, because that is the form they cross the parameter
/// API in; the enums are what the drawing and the copy policy want.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    pub scope: dsp::Scope,
    pub tap: dsp::Tap,
    pub style: spectrum::Style,
    pub size: spectrum::FftSize,
}

/// What the audio thread publishes for the editor, and nothing more: in a host
/// the editor reads and writes the parameters themselves through the host's
/// parameter API, so only the numbers born inside `process()` need a crossing.
/// Every field is an atomic cell of a single value — the audio thread must
/// never block behind the UI, and a torn meter reading is not worth a lock.
pub struct EqSharedState {
    pub input_rms: AtomicCell<f32>,
    /// Linear *true* peak (dBTP) of the input: the crest the DAC rebuilds
    /// between the samples, not the loudest sample. A reading above 1.0 is a
    /// finding, not an overflow.
    pub input_peak: AtomicCell<f32>,
    pub output_rms: AtomicCell<f32>,
    /// As the input peak, of what the plugin sent out.
    pub output_peak: AtomicCell<f32>,
    pub meter_updated: AtomicU64,
    /// The thirteen parameters as the two threads last saw them. In a host
    /// `process()` mirrors these from the parameters each block, so they say
    /// what the filter actually ran on. In standalone the mirror is skipped
    /// and these cells are the parameters' truth: nice-plug gives a parameter
    /// no public setter, so a hostless app has no other door to write through,
    /// and the view writes these cells directly.
    pub mode: AtomicCell<i32>,
    /// The seven faders, low band first, in decibels. An array rather than
    /// seven fields because the panel draws them as a row of the same control.
    pub bands: [AtomicCell<f32>; dsp::BANDS],
    pub gain_db: AtomicCell<f32>,
    pub scope: AtomicCell<i32>,
    pub tap: AtomicCell<i32>,
    pub style: AtomicCell<i32>,
    pub size: AtomicCell<i32>,
    /// The rate `activate()` reported. The analyser lives on the UI thread and
    /// bin numbers are only hertz relative to this, so it has to cross.
    pub sample_rate: AtomicCell<f32>,
    /// The block size the standalone pump last ran, frames; the settings panel
    /// shows it so a player can compare the screen with the dropdown.
    pub buffer_frames: AtomicU32,
    /// Capture frames the transport dropped because this app fell behind,
    /// copied off the transport's own counter by the audio owner.
    pub dropped: AtomicU64,
    /// Feed samples the editor was too slow to take, and samples handed to it.
    /// Counters never reset during an instance's life: a paused picture should
    /// be traceable to the moment it paused.
    pub feed_samples: AtomicU64,
    pub dropped_samples: AtomicU64,
    /// The two analyser rings' consumer halves, parked here while no editor is
    /// open. `EqUi` borrows them on open and returns them on close — main
    /// thread only; the audio thread only ever holds the producers.
    feed: Mutex<Option<(Consumer<f32>, Consumer<f32>)>>,
    logical_size: AtomicCell<LogicalSize<f32>>,
    scale_factor: AtomicCell<f64>,
    open: AtomicBool,
    /// The window's top-left, as the platform last reported it. Standalone
    /// only: a window inside a host belongs to the host, position included.
    #[cfg(feature = "standalone")]
    window_pos: Mutex<Option<(i32, i32)>>,
    /// A saved top-left, queued by the standalone host and applied once by the
    /// window handler through the platform API — baseview has no position
    /// concept. Only placement reads it, so it must not warn without placement.
    #[cfg(feature = "standalone")]
    #[cfg_attr(not(all(windows, feature = "unsafe")), allow(dead_code))]
    pending_geom: Mutex<Option<(i32, i32)>>,
}

impl EqSharedState {
    fn logical_size(&self) -> LogicalSize<f32> {
        self.logical_size.load()
    }
    fn scale_factor(&self) -> f64 {
        self.scale_factor.load()
    }
    fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// One fader by position. The index comes from `0..dsp::BANDS`; the last
    /// arm exists because a match needs one, and the assert is what makes a
    /// wider ladder fail loudly here instead of silently repeating the top
    /// fader's value.
    fn band(&self, index: usize) -> &AtomicCell<f32> {
        debug_assert!(index < dsp::BANDS);
        &self.bands[index]
    }

    /// The DSP's reading of the cells. In standalone this is the parameter
    /// set; in a host it is last block's mirror, one audio block stale at the
    /// very worst.
    fn controls(&self) -> dsp::Controls {
        dsp::Controls {
            mode: self.mode.load(),
            bands: std::array::from_fn(|index| self.band(index).load()),
            gain_db: self.gain_db.load(),
        }
    }

    /// The other direction: publish what the host's parameters say, so the
    /// cells agree with what the filter is about to run on.
    fn publish(&self, controls: &dsp::Controls) {
        self.mode.store(controls.mode);
        for (index, cell) in self.bands.iter().enumerate() {
            cell.store(controls.bands[index]);
        }
        self.gain_db.store(controls.gain_db);
    }

    fn view(&self) -> View {
        View {
            scope: dsp::Scope::from_param(self.scope.load()),
            tap: dsp::Tap::from_param(self.tap.load()),
            style: spectrum::Style::from_param(self.style.load()),
            size: spectrum::FftSize::from_param(self.size.load()),
        }
    }

    fn publish_view(&self, view: &View) {
        self.scope.store(view.scope.to_param());
        self.tap.store(view.tap.to_param());
        self.style.store(view.style.to_param());
        self.size.store(view.size.to_param());
    }

    /// Main thread (editor opening): take the analyser feed, if parked.
    pub fn take_feed(&self) -> Option<(Consumer<f32>, Consumer<f32>)> {
        self.feed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    /// Main thread (editor closing): park the feed for the next session.
    pub fn put_feed(&self, feed: (Consumer<f32>, Consumer<f32>)) {
        *self
            .feed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(feed);
    }
}

/// A switch's position names, in the order its integers run.
type Labels = &'static [&'static str];
/// The two conversions that let a host show a switch as its positions instead
/// of as an integer: a value to its label, and a typed label back to a value.
type ValueToLabel = Arc<dyn Fn(i32) -> String + Send + Sync>;
type LabelToValue = Arc<dyn Fn(&str) -> Option<i32> + Send + Sync>;

/// A stepped parameter, as the machine's switches are: a handful of named
/// positions, not a number. The labels are what the editor draws and what a
/// host parses a typed value against, so the range and the names come from the
/// same list and a position cannot exist in one place and not the other.
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

/// The mode's positions. One for now: the ladder is the only thing this plugin
/// claims to model, and a second mode arrives as a second entry, a second band
/// table, and nothing else.
const MODE_LABELS: Labels = &["G7"];
/// The analyser's three settings. `OFF` is not a picture drawn in black: it is
/// the absence of work, which is the point of having the switch.
const SCOPE_LABELS: Labels = &["Off", "Top", "All"];
/// Which end of the box the analyser reads, named for where the copy is made.
const TAP_LABELS: Labels = &["In", "Out", "Both"];
/// How the same measured array is drawn.
const STYLE_LABELS: Labels = &["Bars", "Line"];
/// The transform lengths, by their size in samples: a 4 k window is a fourth
/// of a second's worth of frequency resolution, and the label says which.
const SIZE_LABELS: Labels = &["1k", "2k", "4k", "8k", "16k", "32k"];

/// The parameters, each exactly as the host sees it. Seven faders, a trim, a
/// mode, and the four settings of the analyser: floats for the continuous
/// controls, stepped integers for what the original builds as switches.
#[derive(Params)]
pub struct EqParams {
    /// Which band ladder is in charge. Integer 0 is panel mode `G7`.
    #[id = "mode"]
    pub mode: IntParam,
    /// The seven faders, low band first. The id names the frequency so an
    /// automation lane in a ten-year-old session still says what it moves.
    #[id = "band_100"]
    pub band_100: FloatParam,
    #[id = "band_200"]
    pub band_200: FloatParam,
    #[id = "band_400"]
    pub band_400: FloatParam,
    #[id = "band_800"]
    pub band_800: FloatParam,
    #[id = "band_1600"]
    pub band_1600: FloatParam,
    #[id = "band_3200"]
    pub band_3200: FloatParam,
    #[id = "band_6400"]
    pub band_6400: FloatParam,
    /// The output trim, ±15 dB like the machine's own level control.
    #[id = "gain_db"]
    pub gain_db: FloatParam,
    /// Off, top, all: how much analysing the view may ask for.
    #[id = "scope"]
    pub scope: IntParam,
    /// Input, output, or both.
    #[id = "tap"]
    pub tap: IntParam,
    /// Bars or interpolated line.
    #[id = "style"]
    pub style: IntParam,
    /// The selected transform length.
    #[id = "size"]
    pub size: IntParam,
}

/// One fader's parameter: the label the machine prints, and the range every
/// one of them shares.
fn fader(name: &str) -> FloatParam {
    FloatParam::new(
        name,
        0.0,
        FloatRange::Linear {
            min: -dsp::MAX_DB,
            max: dsp::MAX_DB,
        },
    )
    .with_unit(" dB")
}

impl Default for EqParams {
    fn default() -> Self {
        // Flat, trim at unity, and the analyser on the selected window only:
        // an equaliser that arrives with a curve on it is arguing with the
        // room before anybody has asked it a question.
        Self {
            mode: stepped("Mode", 0, dsp::Mode::G7.to_param(), MODE_LABELS),
            band_100: fader("100 Hz"),
            band_200: fader("200 Hz"),
            band_400: fader("400 Hz"),
            band_800: fader("800 Hz"),
            band_1600: fader("1.6 kHz"),
            band_3200: fader("3.2 kHz"),
            band_6400: fader("6.4 kHz"),
            gain_db: FloatParam::new(
                "Output",
                0.0,
                FloatRange::Linear {
                    min: -dsp::MAX_DB,
                    max: dsp::MAX_DB,
                },
            )
            .with_unit(" dB"),
            scope: stepped("Spectrum", 0, dsp::Scope::Top.to_param(), SCOPE_LABELS),
            tap: stepped("Source", 0, dsp::Tap::Both.to_param(), TAP_LABELS),
            style: stepped("View", 0, spectrum::Style::Bars.to_param(), STYLE_LABELS),
            size: stepped(
                "Resolution",
                0,
                spectrum::FftSize::DEFAULT.to_param(),
                SIZE_LABELS,
            ),
        }
    }
}

impl EqParams {
    /// One fader by position, for the loops that read all seven.
    fn band(&self, index: usize) -> &FloatParam {
        debug_assert!(index < dsp::BANDS);
        match index {
            0 => &self.band_100,
            1 => &self.band_200,
            2 => &self.band_400,
            3 => &self.band_800,
            4 => &self.band_1600,
            5 => &self.band_3200,
            _ => &self.band_6400,
        }
    }

    /// The DSP's reading of the parameter set. Called at the top of each block:
    /// parameter reads are atomic loads, and taking the whole set at once is
    /// what lets the filter's sample loop touch no atomics at all.
    fn controls(&self) -> dsp::Controls {
        dsp::Controls {
            mode: self.mode.value(),
            bands: std::array::from_fn(|index| self.band(index).value()),
            gain_db: self.gain_db.value(),
        }
    }

    /// The same, for the four settings the picture runs on.
    fn view(&self) -> View {
        View {
            scope: dsp::Scope::from_param(self.scope.value()),
            tap: dsp::Tap::from_param(self.tap.value()),
            style: spectrum::Style::from_param(self.style.value()),
            size: spectrum::FftSize::from_param(self.size.value()),
        }
    }
}

pub struct EqPlugin {
    params: Arc<EqParams>,
    shared: Arc<EqSharedState>,
    equaliser: dsp::Eq,
    /// True-peak tracking of what arrives and what leaves, kept alive between
    /// blocks for the reason in [`block_stats`]: an estimator that restarts at
    /// every block boundary misses the crest that rides across it.
    tp_in: [TruePeak; 2],
    tp_out: [TruePeak; 2],
    /// The analyser feed, one ring per tap. Producers stay here, on the audio
    /// thread, for the whole life of the instance; the consumers travel to the
    /// editor and back through [`EqSharedState::take_feed`].
    ring_in: Producer<f32>,
    ring_out: Producer<f32>,
}

/// Seed the shared cells from the saved standalone preferences. A `None`
/// preference keeps the parameter's own default, so the settings file can stay
/// a record of moves rather than a snapshot of everything; a value outside the
/// parameter's range is clamped exactly as a host's write would be, because a
/// hand-edited RON is a user error, not a license to feed the filter nonsense.
#[cfg(feature = "standalone")]
pub(crate) fn seed_standalone_params(
    shared: &EqSharedState,
    params: &EqParams,
    prefs: &settings::EqPreferences,
) {
    let seed = |cell: &AtomicCell<f32>, param: &FloatParam, saved| {
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
    if let Some(bands) = prefs.bands {
        for (index, level) in bands.into_iter().enumerate() {
            seed(shared.band(index), params.band(index), Some(level));
        }
    }
    seed(&shared.gain_db, &params.gain_db, prefs.gain_db);
    seed_stepped(&shared.scope, &params.scope, prefs.scope);
    seed_stepped(&shared.tap, &params.tap, prefs.tap);
    seed_stepped(&shared.style, &params.style, prefs.style);
    seed_stepped(&shared.size, &params.size, prefs.fft_size);
}

impl Default for EqPlugin {
    fn default() -> Self {
        let params = Arc::new(EqParams::default());
        let shared = Arc::new(EqSharedState {
            input_rms: AtomicCell::new(0.0),
            input_peak: AtomicCell::new(0.0),
            output_rms: AtomicCell::new(0.0),
            output_peak: AtomicCell::new(0.0),
            meter_updated: AtomicU64::new(0),
            mode: AtomicCell::new(params.mode.value()),
            bands: std::array::from_fn(|index| AtomicCell::new(params.band(index).value())),
            gain_db: AtomicCell::new(params.gain_db.value()),
            scope: AtomicCell::new(params.scope.value()),
            tap: AtomicCell::new(params.tap.value()),
            style: AtomicCell::new(params.style.value()),
            size: AtomicCell::new(params.size.value()),
            sample_rate: AtomicCell::new(44_100.0),
            buffer_frames: AtomicU32::new(0),
            dropped: AtomicU64::new(0),
            feed_samples: AtomicU64::new(0),
            dropped_samples: AtomicU64::new(0),
            feed: Mutex::new(None),
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
                eprintln!("nice-eq: {warning}");
            }
            seed_standalone_params(&shared, &params, &settings.preferences);
        }
        // Both halves of both rings are made here, once, before anything can
        // ask for audio: a real-time path that allocated on first use would be
        // a stall waiting to happen at the worst possible bar.
        let (ring_in, feed_in) = RingBuffer::new(RING_CAPACITY);
        let (ring_out, feed_out) = RingBuffer::new(RING_CAPACITY);
        *shared
            .feed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((feed_in, feed_out));
        // The rate arrives with `activate`; until then the equaliser idles at a
        // rate any preset could have been made at, and it is re-tuned there.
        let equaliser = dsp::Eq::new(44_100.0, &shared.controls());
        Self {
            params,
            shared,
            equaliser,
            tp_in: [TruePeak::new(); 2],
            tp_out: [TruePeak::new(); 2],
            ring_in,
            ring_out,
        }
    }
}

impl Plugin for EqPlugin {
    const NAME: &'static str = "Nice Eq";
    const VENDOR: &'static str = "frost developers";
    const URL: &'static str = "https://github.com/perlindgren/frost";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// Mono in, mono out, the way the box this layout comes from is wired: one
    /// input jack, seven band filters, one output. A host that wants a stereo
    /// bus equalised puts two instances on it, or decides for itself what
    /// reaches this one — either way that is the host's routing decision, and a
    /// declared stereo pair here would only be this plugin volunteering to be
    /// twice the box it is. Should a host hand over a wider buffer than the
    /// layout declares, the sections still run on channel 0 and the other lanes
    /// come back silent: silence is honest about a channel this equaliser has
    /// no input for, where a copy would pretend and a sum would mix.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(1),
        main_output_channels: NonZeroU32::new(1),
        ..AudioIOLayout::const_default()
    }];

    type Editor = EqEditor;
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
        self.shared.sample_rate.store(config.sample_rate);
        self.equaliser.set_rate(config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.equaliser.reset();
        // The meters come back empty with it: a crest does not outlive the
        // audio it was read from, and an estimator carrying a block of sound
        // from before the reset would ring into the first block after it.
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
        // A host owns the parameters, so publish them before anything reads the
        // cells. In standalone the view owns them, and mirroring here would
        // overwrite what the settings panel just wrote.
        if !STANDALONE_MODE.load(Ordering::Relaxed) {
            let controls = self.params.controls();
            self.shared.publish(&controls);
            let view = self.params.view();
            self.shared.publish_view(&view);
        }
        let view = self.shared.view();
        // Three reasons not to copy a single sample: the view is shut, the view
        // said `off`, or the view is not watching this end of the box. All
        // three are the same answer to the audio thread, and all three leave
        // the equaliser itself untouched — the analyser watches the signal, it
        // does not own it.
        let wants_in = self.shared.is_open()
            && !matches!(view.scope, dsp::Scope::Off)
            && view.tap.wants_input();
        let wants_out = self.shared.is_open()
            && !matches!(view.scope, dsp::Scope::Off)
            && view.tap.wants_output();

        let channels = buffer.channels();
        // Two rows because that is the shape `block_stats` measures and the
        // analyser's ring expects from the pedal family's adapters; the second
        // row is filled with silence and stays that way, which is exactly what a
        // mono box's second lane is.
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
                if let Some(samples) = block.get(channel) {
                    scratch[..n].copy_from_slice(&samples[..n]);
                }
            }
            if wants_in {
                push(&mut self.ring_in, &self.shared, &input[0][..n]);
            }
            let (energy, peak) = block_stats(&input, channels, n, &mut self.tp_in);
            input_energy += energy;
            input_peak = input_peak.max(peak);
            // One signal through one cascade: channel 0 in, channel 0 out. A
            // host that handed over more lanes than the layout declares gets
            // silence in the ones there is no input for, because only the first
            // row of the output scratch is ever written — never a copy of the
            // first and never a sum of both.
            self.equaliser
                .process_mono(&controls, &input[0][..n], &mut output[0][..n]);
            let (energy, peak) = block_stats(&output, channels, n, &mut self.tp_out);
            output_energy += energy;
            output_peak = output_peak.max(peak);
            count += n;
            for (channel, out) in block.into_iter().enumerate().take(channels) {
                out.copy_from_slice(&output[channel][..n]);
            }
            if wants_out {
                push(&mut self.ring_out, &self.shared, &output[0][..n]);
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

/// Hand one block of one lane to the analyser. A ring that is full means the
/// editor has not drained for the whole length of the largest window — two
/// thirds of a second at 48 kHz: the block is dropped whole rather than torn,
/// because a half-window is a wrong spectrum and a stale picture is only a
/// stale picture.
fn push(ring: &mut Producer<f32>, shared: &EqSharedState, samples: &[f32]) {
    let counter = if ring.push_entire_slice(samples).is_ok() {
        &shared.feed_samples
    } else {
        &shared.dropped_samples
    };
    counter.fetch_add(samples.len() as u64, Ordering::Relaxed);
}

impl ClapPlugin for EqPlugin {
    const CLAP_ID: &'static str = "com.frost.nice-eq";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("A seven-band graphic equaliser with a spectrum analyser");
    const CLAP_MANUAL_URL: Option<&'static str> =
        Some("https://github.com/perlindgren/frost/blob/main/docs/plugin-inspiration.md");
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Equalizer,
        ClapFeature::Filter,
    ];
}

impl Vst3Plugin for EqPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"FrostNiceEqFx001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Tools,
        Vst3SubCategory::Eq,
    ];
}

nice_export_clap!(EqPlugin);
nice_export_vst3!(EqPlugin);

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
        // not shipped yet, which is why a wrong id could still be changed for
        // free — after the first release, dropping or renaming an id is a
        // migration, and this test is what makes that loud.
        let plugin = EqPlugin::default();
        let map = plugin.params().param_map();
        let mut ids: Vec<&str> = map.iter().map(|(id, _, _)| id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            [
                "band_100",
                "band_1600",
                "band_200",
                "band_3200",
                "band_400",
                "band_6400",
                "band_800",
                "gain_db",
                "mode",
                "scope",
                "size",
                "style",
                "tap",
            ]
        );
    }

    #[test]
    fn the_port_map_is_one_in_and_one_out() {
        // A host builds its routing out of this declaration, so it is a promise
        // in the same currency as a parameter id: two jacks here and two lanes
        // of every session's audio are allocated for a box that answers on one.
        // The hardware's own answer — one input jack, one output — is also the
        // cheapest one to keep, because a mono declaration leaves widening,
        // duplicating and downmixing where they have always been decided.
        let layouts = EqPlugin::AUDIO_IO_LAYOUTS;
        assert_eq!(layouts.len(), 1, "one box, one port map");
        assert_eq!(layouts[0].main_input_channels.map(|c| c.get()), Some(1));
        assert_eq!(layouts[0].main_output_channels.map(|c| c.get()), Some(1));
    }

    #[test]
    fn every_fader_moves_the_full_fifteen_decibels_it_claims_to() {
        let params = EqParams::default();
        let bands = dsp::Mode::G7.bands();
        for (index, band) in bands.iter().enumerate() {
            let param = params.band(index);
            let FloatRange::Linear { min, max } = param.range() else {
                panic!("faders are linear ranges");
            };
            assert_eq!((min, max), (-dsp::MAX_DB, dsp::MAX_DB));
            assert_eq!(param.value(), 0.0, "fader {index} does not start flat");
            // And the name on the parameter is the frequency on the panel, so a
            // host's generic list and this editor cannot disagree.
            let hz = band.hz;
            let expect = if hz < 1_000.0 {
                format!("{} Hz", hz)
            } else {
                format!("{} kHz", hz / 1_000.0)
            };
            assert_eq!(param.name(), expect.as_str(), "fader {index}");
        }
    }

    #[test]
    fn every_switch_offers_the_positions_it_claims_to() {
        // The label lists are what the editor draws and what a host parses a
        // typed value against. A range one narrower than the labels would let a
        // position exist on screen and nowhere in the parameter.
        let params = EqParams::default();
        let positions = |param: &IntParam| {
            let IntRange::Linear { min, max } = param.range() else {
                panic!("switches are linear integer ranges");
            };
            (max - min + 1) as usize
        };
        assert_eq!(positions(&params.mode), MODE_LABELS.len());
        assert_eq!(positions(&params.scope), SCOPE_LABELS.len());
        assert_eq!(positions(&params.tap), TAP_LABELS.len());
        assert_eq!(positions(&params.style), STYLE_LABELS.len());
        assert_eq!(positions(&params.size), SIZE_LABELS.len());
        // The one label the panel shows for the mode is the one the ladder is
        // named by, so the mode cannot be called one thing here and another
        // where the bands are chosen.
        assert_eq!(MODE_LABELS, [dsp::Mode::G7.label()]);
        // And each default is a position the labels actually have.
        for (param, labels) in [
            (&params.mode, MODE_LABELS),
            (&params.scope, SCOPE_LABELS),
            (&params.tap, TAP_LABELS),
            (&params.style, STYLE_LABELS),
            (&params.size, SIZE_LABELS),
        ] {
            let value = param.value();
            let label = labels.get(value.max(0) as usize).copied().unwrap_or("");
            assert!(
                !label.is_empty(),
                "{} defaults outside its labels",
                param.name()
            );
        }
    }

    #[test]
    fn the_analyser_starts_on_the_selected_window_and_on_both_taps() {
        // The defaults are a decision, not an accident: `Top` gives a live
        // picture for one transform a frame instead of six, and `Both` is the
        // answer to the question a player actually has, which is what the
        // equaliser did to what went in.
        let plugin = EqPlugin::default();
        let view = plugin.params.view();
        assert_eq!(view.scope, dsp::Scope::Top);
        assert_eq!(view.tap, dsp::Tap::Both);
        assert_eq!(view.size, spectrum::FftSize::DEFAULT);
        assert_eq!(view.style, spectrum::Style::Bars);
        assert!(view.tap.wants_input() && view.tap.wants_output());
    }

    /// Field by field, the way the rest of the set compares control sets: no
    /// `PartialEq` on [`dsp::Controls`], because a struct of computed floats
    /// should not invite an equality test that rounding would break.
    fn assert_same_controls(left: &dsp::Controls, right: &dsp::Controls, what: &str) {
        assert_eq!(left.mode, right.mode, "{what}: mode");
        assert_eq!(left.bands, right.bands, "{what}: bands");
        assert_eq!(left.gain_db, right.gain_db, "{what}: trim");
    }

    #[test]
    fn the_cells_start_where_the_parameters_do_and_follow_them() {
        let plugin = EqPlugin::default();
        let shared = plugin.shared.clone();
        assert_same_controls(
            &shared.controls(),
            &plugin.params.controls(),
            "a fresh instance",
        );
        assert_eq!(shared.view(), plugin.params.view());
        // A host write, published the way `process()` publishes it, is what the
        // editor then reads.
        let moved = dsp::Controls {
            mode: 0,
            bands: [-15.0, -10.0, -5.0, 0.0, 5.0, 10.0, 15.0],
            gain_db: -3.5,
        };
        shared.publish(&moved);
        assert_eq!(shared.band(0).load(), -15.0);
        assert_eq!(shared.band(6).load(), 15.0);
        assert_same_controls(&shared.controls(), &moved, "after a publish");
        shared.publish_view(&View {
            scope: dsp::Scope::All,
            tap: dsp::Tap::Out,
            style: spectrum::Style::Line,
            size: spectrum::FftSize::S32768,
        });
        assert_eq!(shared.view().scope, dsp::Scope::All);
        assert!(!shared.view().tap.wants_input());
    }

    #[test]
    fn the_feed_survives_an_editor_closing_and_opening_again() {
        // The rings belong to the instance, not to the window: an editor that
        // closes must hand its consumers back, and the next one must find two
        // halves of the same two rings rather than a plugin with nowhere to
        // write.
        let plugin = EqPlugin::default();
        let first = plugin
            .shared
            .take_feed()
            .expect("a fresh instance parks both consumers");
        assert!(
            plugin.shared.take_feed().is_none(),
            "the feed was handed out twice"
        );
        plugin.shared.put_feed(first);
        assert!(plugin.shared.take_feed().is_some());
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn a_saved_setting_arrives_clamped_the_way_a_host_write_would_be() {
        let plugin = EqPlugin::default();
        let shared = plugin.shared.clone();
        seed_standalone_params(
            &shared,
            &plugin.params,
            &settings::EqPreferences {
                mode: Some(0),
                bands: Some([-99.0, -1.0, 0.0, 0.0, 0.0, 0.0, 99.0]),
                gain_db: Some(40.0),
                scope: Some(9),
                tap: Some(-4),
                style: Some(1),
                fft_size: Some(99),
            },
        );
        let controls = shared.controls();
        // Outside the range means at the end of the range, never beyond it: the
        // filter's own clamp is a backstop, not the plan.
        assert_eq!(controls.bands[0], -dsp::MAX_DB);
        assert_eq!(controls.bands[6], dsp::MAX_DB);
        assert_eq!(controls.gain_db, dsp::MAX_DB);
        let view = shared.view();
        assert_eq!(view.scope, dsp::Scope::All);
        assert_eq!(view.tap, dsp::Tap::In);
        assert_eq!(view.size, spectrum::FftSize::S32768);
    }
}
