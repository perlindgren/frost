//! Nice Tuner — a stringed-instrument tuner plugin.
//!
//! Host plumbing follows nice-plug's `byo_gui_wgpu` example (tag v0.4.2):
//! a `baseview` child window embedded in the host, rendered with wgpu.
//! The triangle placeholder renderer is replaced by the frost-derived
//! embedded engine in M3/M4 (see docs/VIABILITY.md).

#![deny(unsafe_code)]

use crossbeam::atomic::AtomicCell;
use nice_common::true_peak::TruePeak;
use nice_plug::editor::dpi::LogicalSize;
use nice_plug::prelude::*;
use std::sync::atomic::AtomicU64;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const MIN_SIZE: LogicalSize<f32> = LogicalSize::new(480.0, 320.0);
const DEFAULT_SIZE: LogicalSize<f32> = LogicalSize::new(640.0, 400.0);
const RESIZE_HINT: ResizeHint = ResizeHint::resizable().with_min_logical_size(MIN_SIZE);

// ---------------------------------------------------------------------------------------------------
// GUI <-> audio-thread shared state

/// State shared between the audio thread (`process`) and the GUI.
#[derive(Debug)]
pub struct TunerSharedState {
    /// Logical window size, kept up to date by the editor.
    logical_size: AtomicCell<LogicalSize<f32>>,
    /// Whether the editor's window is currently open. The audio thread uses
    /// this to gate expensive detection work.
    open: AtomicBool,
    scale_factor: AtomicCell<f64>,
    /// Latest pitch estimate in Hz (0.0 = no detection yet).
    pub freq_hz: AtomicCell<f64>,
    /// Confidence of the latest estimate, 0.0..=1.0.
    pub clarity: AtomicCell<f64>,
    /// RMS level of the analysed window (for the input meter).
    pub rms: AtomicCell<f32>,
    /// Input meter truth, computed on the audio thread over the one channel
    /// the box was handed: its RMS and the block's true peak (dBTP, see
    /// [`nice_common::true_peak`]). There is no downmix left to meter behind
    /// — the box has a single input lane and that lane is what gets metered.
    pub input_rms: AtomicCell<f32>,
    /// The true peak the line above describes. A reading above 1.0 is a
    /// finding, not an overflow: the inter-sample crest of a signal whose
    /// samples all fit inside full scale really can exceed it.
    pub input_peak: AtomicCell<f32>,
    pub output_rms: AtomicCell<f32>,
    /// As the input pair, of what the tuner sent out (zero while muted).
    pub output_peak: AtomicCell<f32>,
    pub muted: AtomicBool,
    /// Standalone only: which of the audio device's inputs becomes the box's
    /// one input, 1-based. Defaults to 1 (the tuner's normal single-instrument
    /// cable). This is host-edge routing, decided where the device's frames
    /// are de-interleaved — the plugin itself has one input and never picks
    /// between channels. In a host the cable the patchbay already chose IS
    /// the input, so this field does nothing there.
    pub input_channel: AtomicCell<u32>,
    /// Block size (frames per channel) the live stream ACTUALLY runs
    /// with — the driver's answer, not the requested one. The settings
    /// page shows it as the buffer default; 0 until the first callback.
    pub buffer_frames: AtomicCell<u32>,
    /// Live handoff diagnostics. Counters never reset during an instance's life.
    pub process_calls: AtomicU64,
    pub feed_samples: AtomicU64,
    pub dropped_samples: AtomicU64,
    /// Standalone top-level window position, physical px (top-left),
    /// polled by the gui on Windows; read when geometry is persisted.
    pub window_pos: std::sync::Mutex<Option<(i32, i32)>>,
    /// Position to apply once at window creation: restored from the
    /// settings file, or computed screen-center on first launch.
    pub pending_geom: std::sync::Mutex<Option<(i32, i32)>>,
    /// Wall-clock ms (UNIX epoch) of the last confident detection; the UI
    /// treats the signal as absent when this goes stale (the detector only
    /// reports confident windows).
    pub last_detection_ms: AtomicU64,
    /// A4 reference frequency in Hz, mirrored from the param so the GUI can
    /// label notes with the user's tuning standard. Stored as bits of f64.
    pub a4_ref_bits: AtomicU64,
    /// Gate threshold in dB, mirrored from the param for the GUI's
    /// signal-present decision.
    pub gate_db: AtomicCell<f32>,
    /// Sample rate reported by `activate()`; the UI-thread detector is
    /// (re)built against this value whenever it changes.
    pub sample_rate_hz: AtomicCell<f64>,
    /// Active main-bus channel counts and the host's configured maximum block.
    pub input_channels: AtomicCell<u32>,
    pub output_channels: AtomicCell<u32>,
    pub max_buffer_frames: AtomicCell<u32>,
    /// Analysis-feed consumer, parked here while no editor is open.
    /// `TunerUi` borrows it on open and returns it on close (Drop) — main
    /// thread only; the audio thread only ever holds the `Producer` half.
    consumer_slot: std::sync::Mutex<Option<rtrb::Consumer<f32>>>,
}

impl Default for TunerSharedState {
    fn default() -> Self {
        Self::new()
    }
}

impl TunerSharedState {
    pub fn new() -> Self {
        Self {
            logical_size: AtomicCell::new(DEFAULT_SIZE),
            open: AtomicBool::new(false),
            scale_factor: AtomicCell::new(1.0),
            freq_hz: AtomicCell::new(0.0),
            clarity: AtomicCell::new(0.0),
            rms: AtomicCell::new(0.0),
            input_rms: AtomicCell::new(0.0),
            input_peak: AtomicCell::new(0.0),
            output_rms: AtomicCell::new(0.0),
            output_peak: AtomicCell::new(0.0),
            muted: AtomicBool::new(false),
            input_channel: AtomicCell::new(1),
            buffer_frames: AtomicCell::new(0),
            process_calls: AtomicU64::new(0),
            feed_samples: AtomicU64::new(0),
            dropped_samples: AtomicU64::new(0),
            window_pos: std::sync::Mutex::new(None),
            pending_geom: std::sync::Mutex::new(None),
            last_detection_ms: AtomicU64::new(0),
            a4_ref_bits: AtomicU64::new(440.0f64.to_bits()),
            gate_db: AtomicCell::new(-60.0),
            sample_rate_hz: AtomicCell::new(0.0),
            input_channels: AtomicCell::new(0),
            output_channels: AtomicCell::new(0),
            max_buffer_frames: AtomicCell::new(0),
            consumer_slot: std::sync::Mutex::new(None),
        }
    }

    /// Main thread (editor opening): take the analysis feed, if parked.
    pub fn take_consumer(&self) -> Option<rtrb::Consumer<f32>> {
        self.consumer_slot
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    /// Main thread (editor closing): park the feed for the next session.
    pub fn put_consumer(&self, consumer: rtrb::Consumer<f32>) {
        *self.consumer_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(consumer);
    }

    pub fn logical_size(&self) -> LogicalSize<f32> {
        self.logical_size.load()
    }

    pub fn set_logical_size(&self, size: LogicalSize<f32>) {
        self.logical_size.store(size);
    }

    pub fn scale_factor(&self) -> f64 {
        self.scale_factor.load()
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    pub fn a4_ref(&self) -> f64 {
        f64::from_bits(self.a4_ref_bits.load(Ordering::Relaxed))
    }
}

// ---------------------------------------------------------------------------------------------------
// The embedded window is supplied by the shared nice-plugin adapter,
// tuner scene + UI logic in src/tuner_ui.rs.
#[cfg(feature = "standalone")]
pub mod devices;
mod host_controls;
pub use nice_common::meter;
pub mod settings;
#[cfg(feature = "standalone")]
pub mod standalone;
mod tuner_ui;

/// True when running as the standalone app (set by its `main` before the
/// plugin is instantiated). The settings panel is standalone-only: a host
/// owns the audio system inside a DAW, so device choices would lie there.
pub static STANDALONE_MODE: AtomicBool = AtomicBool::new(false);

/// Actual startup configuration, which may be silent when saved settings
/// are incomplete. Keep the saved draft available for correction in the UI.
#[cfg(feature = "standalone")]
pub static STANDALONE_BOOT: std::sync::OnceLock<(settings::Settings, Option<String>)> =
    std::sync::OnceLock::new();

/// Standalone param ownership: with no host to drive `FloatParam`s, the
/// saved file (and later the settings page) owns A4/gate. Pure function of
/// (state, settings) so it is unit-testable without process-global mode.
pub fn seed_standalone_params(shared: &TunerSharedState, s: &settings::Settings) {
    shared
        .muted
        .store(s.muted.unwrap_or(false), Ordering::Relaxed);
    if let Some(a4) = s.a4_ref {
        shared
            .a4_ref_bits
            .store((a4 as f64).to_bits(), Ordering::Relaxed);
    }
    if let Some(gate) = s.gate_db {
        shared.gate_db.store(gate);
    }
    if let Some(ch) = s.input_channel {
        shared.input_channel.store(ch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_mute_silences_the_channel_and_metering() {
        let mut signal = [0.25f32; 32];
        let mut meter = TruePeak::new();
        // First pass warms the histories. A level arriving at an empty
        // estimator is an onset and rings above itself for a few samples
        // (see `nice_common::true_peak`); a settled one reads exactly.
        meter_output(&mut signal, false, &mut meter);
        assert_eq!(meter_output(&mut signal, false, &mut meter), (0.25, 0.25));
        assert_eq!(meter_output(&mut signal, true, &mut meter), (0.0, 0.0));
        assert_eq!(signal, [0.0; 32]);
        // Mute is a hard break, not a fade: the estimator histories went with
        // it, so the signal returning after unmute is an onset again rather
        // than a continuation it could read off the held history.
        signal.fill(0.25);
        let (_, ringing) = meter_output(&mut signal, false, &mut meter);
        assert!(
            ringing > 0.25,
            "expected onset ringing after unmute, read {ringing}"
        );
    }

    /// A quarter-rate sine whose every sample sits at 1/√2: the crest lives
    /// entirely between samples. A raw fold reports 0.7071, the tuner's
    /// output meter reports the true peak (~0.98) — and, like any tone below
    /// full scale, still stays under it.
    #[test]
    fn output_metering_finds_the_crest_between_samples() {
        let mut signal = [0.0f32; 64];
        for (i, sample) in signal.iter_mut().enumerate() {
            *sample = (std::f32::consts::FRAC_PI_2 * i as f32 + std::f32::consts::FRAC_PI_4).sin();
        }
        let raw = signal.iter().fold(0.0f32, |acc, &v| acc.max(v.abs()));
        let mut meter = TruePeak::new();
        let (_rms, peak) = meter_output(&mut signal, false, &mut meter);
        assert!(
            (raw - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5,
            "samples should all sit at 1/sqrt(2), read {raw}"
        );
        assert!(
            peak > 0.95 && peak < 1.0,
            "true peak {peak} should surface the inter-sample crest without \
             over-reading a tone"
        );
    }

    #[test]
    fn host_parameters_include_tuning_and_display_preferences_with_stable_ids() {
        let params = TunerParams::default();
        let ids: Vec<_> = params
            .param_map()
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(
            ids,
            [
                "a4_ref",
                "gate",
                "in_tune_cents",
                "gauge_knee_cents",
                "mute"
            ]
        );
        assert_eq!(params.in_tune_cents.value(), 3.0);
        assert_eq!(params.gauge_knee_cents.value(), 5.0);
    }

    #[test]
    fn standalone_seeding_applies_params_and_skips_absences() {
        let shared = TunerSharedState::new();
        assert_eq!(shared.a4_ref(), 440.0); // default untouched

        seed_standalone_params(
            &shared,
            &settings::Settings {
                preferences: crate::settings::TunerPreferences {
                    a4_ref: Some(432.0),
                    gate_db: Some(-45.0),
                    ..Default::default()
                },
                input_channel: Some(1),
                ..Default::default()
            },
        );
        assert_eq!(shared.a4_ref(), 432.0);
        assert_eq!(shared.gate_db.load(), -45.0);
        assert_eq!(shared.input_channel.load(), 1);

        // Absent fields must not clobber: values survive a seed pass.
        seed_standalone_params(&shared, &settings::Settings::default());
        assert_eq!(shared.a4_ref(), 432.0);
        assert_eq!(shared.gate_db.load(), -45.0);
        assert_eq!(shared.input_channel.load(), 1);
    }
}

// ---------------------------------------------------------------------------------------------------
// The same project-owned editor adapter used by every other nice-* plugin.
mod editor;
pub use editor::TunerEditor;
#[cfg(feature = "standalone")]
use editor::TunerEditorHandle;

// ---------------------------------------------------------------------------------------------------
// Plugin

/// The tuner plugin. Audio is passed through; the value it adds is analysis —
/// which runs on the UI thread: `process()` only feeds a lock-free ring, and
/// the editor's frost frames (nice-plug's main thread, in host and standalone
/// alike) drain and detect. When the editor is closed there is no analysis
/// cost at all: the feed is not even written.
pub struct TunerPlugin {
    params: Arc<TunerParams>,
    shared: Arc<TunerSharedState>,
    /// Producer half of the analysis feed (lock-free, preallocated; see
    /// `tests/rt_handoff_no_alloc.rs`).
    audio_ring: rtrb::Producer<f32>,
    /// True-peak estimator for the metered input, carried between callbacks
    /// so a crest straddling a block boundary is still found. One instance
    /// for the one channel the box has; fixed-size and stack-copied in
    /// `process`: the audio thread allocates nothing.
    tp_in: TruePeak,
    /// The same for the output, which is metered on every callback.
    tp_out: TruePeak,
    /// Whether the previous callback metered the input. That pass is gated
    /// on an open editor, and an estimator whose history froze while the
    /// window was shut must not ring against the first block after a reopen
    /// — so the histories are cleared on that edge.
    input_metering_warm: bool,
}

#[derive(Params)]
pub struct TunerParams {
    /// A4 concert pitch reference, in Hz.
    #[id = "a4_ref"]
    pub a4_ref: FloatParam,
    /// Detection gate in dBFS (audio passthrough is unaffected).
    #[id = "gate"]
    pub gate_db: FloatParam,
    #[id = "in_tune_cents"]
    pub in_tune_cents: FloatParam,
    #[id = "gauge_knee_cents"]
    pub gauge_knee_cents: FloatParam,
    /// Output silence, independently of analysis and host bypass.
    #[id = "mute"]
    pub mute: BoolParam,
}

impl Default for TunerPlugin {
    fn default() -> Self {
        // ~0.7 s of mono at common rates: a stalled UI pump drops audio into
        // the ring rather than corrupting it, and a reopened editor clears
        // the backlog before analyzing.
        let (producer, consumer) = rtrb::RingBuffer::new(1 << 15);
        let shared = Arc::new(TunerSharedState::new());
        shared.put_consumer(consumer);
        if STANDALONE_MODE.load(Ordering::Relaxed) {
            // Standalone: no host exists to move params, so the settings
            // page owns A4/gate. Seed from the saved file; `process()`
            // leaves the mirrors alone in this mode (see below).
            let (s, _warning) = settings::Settings::load();
            seed_standalone_params(&shared, &s);
            #[cfg(feature = "standalone")]
            if let Some((applied, _)) = STANDALONE_BOOT.get() {
                shared
                    .input_channel
                    .store(applied.input_channel.unwrap_or(1));
            }
        }
        Self {
            params: Arc::new(TunerParams::default()),
            shared,
            audio_ring: producer,
            tp_in: TruePeak::new(),
            tp_out: TruePeak::new(),
            input_metering_warm: false,
        }
    }
}

impl Default for TunerParams {
    fn default() -> Self {
        Self {
            mute: BoolParam::new("Mute", false),
            a4_ref: FloatParam::new(
                "A4 Reference",
                440.0,
                FloatRange::Linear {
                    min: 415.0,
                    max: 466.0,
                },
            )
            .with_unit(" Hz")
            .with_step_size(0.1),
            gate_db: FloatParam::new(
                "Gate",
                -60.0,
                FloatRange::Linear {
                    min: -80.0,
                    max: -20.0,
                },
            )
            .with_unit(" dB")
            .with_step_size(1.0),
            in_tune_cents: FloatParam::new(
                "In-tune Window",
                3.0,
                FloatRange::Linear {
                    min: 1.0,
                    max: 10.0,
                },
            )
            .with_unit(" cents")
            .with_step_size(1.0),
            gauge_knee_cents: FloatParam::new(
                "Gauge Curve",
                5.0,
                FloatRange::Linear {
                    min: 3.0,
                    max: 20.0,
                },
            )
            .with_unit(" cents")
            .with_step_size(1.0),
        }
    }
}

impl Plugin for TunerPlugin {
    const NAME: &'static str = "Nice Tuner";
    const VENDOR: &'static str = "nice-tuner developers";
    const URL: &'static str = "https://example.invalid/nice-tuner";
    const EMAIL: &'static str = "";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    /// Mono in, mono out, and nothing else: the box is mono — one signal
    /// through one detector — and so is this model. The plugin neither folds
    /// a stereo signal down nor widens a mono one; that width work belongs to
    /// the host and the rack, which is exactly why this is the only layout
    /// here.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(1),
        main_output_channels: NonZeroU32::new(1),
        ..AudioIOLayout::const_default()
    }];

    type Editor = TunerEditor;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
        Some(crate::editor::create(
            Arc::clone(&self.params),
            Arc::clone(&self.shared),
        ))
    }

    fn activate(
        &mut self,
        audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl ActivateContext<Self>,
    ) -> bool {
        // The detector itself lives UI-side (TunerUi); it builds itself from
        // this rate on the first frame, and rebuilds if the host restarts us
        // at another one.
        self.shared
            .sample_rate_hz
            .store(buffer_config.sample_rate as f64);
        self.shared.input_channels.store(
            audio_io_layout
                .main_input_channels
                .map_or(0, |count| count.get()),
        );
        self.shared.output_channels.store(
            audio_io_layout
                .main_output_channels
                .map_or(0, |count| count.get()),
        );
        self.shared
            .max_buffer_frames
            .store(buffer_config.max_buffer_size);
        self.shared.buffer_frames.store(0);
        // A fresh stream: whatever the last activation left in the meter
        // histories belongs to a different signal.
        self.tp_in.reset();
        self.tp_out.reset();
        self.input_metering_warm = false;
        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Param mirrors for the GUI thread (cheap, once per block). Host-mode
        // only: the host owns params there. In standalone the panel writes
        // these atomics directly, and params never move (nothing drives them),
        // so mirroring would overwrite the user's edits every block.
        if !STANDALONE_MODE.load(Ordering::Relaxed) {
            self.shared.a4_ref_bits.store(
                (self.params.a4_ref.value() as f64).to_bits(),
                Ordering::Relaxed,
            );
            self.shared.gate_db.store(self.params.gate_db.value());
            self.shared
                .muted
                .store(self.params.mute.value(), Ordering::Relaxed);
        }

        // One relaxed store per block: the size the driver gave us.
        self.shared.buffer_frames.store(buffer.samples() as u32);
        self.shared.process_calls.fetch_add(1, Ordering::Relaxed);

        if self.shared.is_open() {
            // Buffer is in-place (the framework copied host input into it);
            // analysis reads the input before output muting below.
            // A copy of the one input lane into the preallocated ring: the
            // audio thread's entire analysis contribution is this copy.
            // (assert_process_allocs guards the callback in debug builds;
            // tests/rt_handoff_no_alloc.rs gates the ring itself.)
            const CHUNK: usize = 4096;
            let mut mono = [0f32; CHUNK];
            // Input metering truth, gathered here on the RT thread: the one
            // lane's sum of squares and its TRUE peak, estimated by
            // [`nice_common::true_peak`] and carried between callbacks. A
            // copied fixed-size estimator and stack arithmetic — no
            // allocation, one atomic store pair after the loop.
            let mut tp = self.tp_in;
            if !self.input_metering_warm {
                // Reopening the editor after a gap: the held history belongs
                // to a stream that is no longer continuous with this one.
                tp.reset();
                self.input_metering_warm = true;
            }
            let mut frames = 0usize;
            let mut sum = 0.0f64;
            for (_start, blk) in buffer.iter_blocks(CHUNK) {
                let n = blk.samples().min(CHUNK);
                // The layout declares one channel, so the first lane is the
                // signal and there is nothing here to choose between.
                let Some(lane) = blk.get(0) else {
                    continue;
                };
                let lane = &lane[..n];
                frames += n;
                tp.process(lane);
                for &v in lane {
                    sum += (v as f64) * (v as f64);
                }
                mono[..n].copy_from_slice(lane);
                // Ring full (UI stalled longer than ~0.7 s): drop the block
                // whole. A torn stream would poison detections; stale
                // atomics read as silence (freshness gate) — the honest
                // failure is a paused needle, never a glitch or a wrong note.
                let counter = if self.audio_ring.push_entire_slice(&mono[..n]).is_ok() {
                    &self.shared.feed_samples
                } else {
                    &self.shared.dropped_samples
                };
                counter.fetch_add(n as u64, Ordering::Relaxed);
            }
            // The estimator keeps its history across callbacks; its running
            // peak is what this call measured, and then it restarts.
            let peak = tp.take();
            self.tp_in = tp;
            if frames > 0 {
                self.shared
                    .input_rms
                    .store((sum / frames as f64).sqrt() as f32);
                self.shared.input_peak.store(peak);
            }
        } else {
            // Metering paused while the editor is shut, so the held histories
            // are no longer continuous with the stream: flag it cold and the
            // first callback after a reopen starts from silence.
            self.input_metering_warm = false;
        }

        // Muting happens after the analysis feed; an open editor's detector
        // keeps seeing the original input while output is muted.
        let muted = self.shared.muted.load(Ordering::Relaxed);
        let channels = buffer.as_slice();
        let (rms, peak) = meter_output(
            channels
                .first_mut()
                .expect("the mono layout declares one output channel"),
            muted,
            &mut self.tp_out,
        );
        self.shared.output_rms.store(rms as f32);
        self.shared.output_peak.store(peak);
        ProcessStatus::Normal
    }
}

/// Meter the one output channel after applying mute to it.
///
/// The peak is that channel's true peak (dBTP), read by the persistent
/// `meter`, so a crest straddling a callback boundary is still found. Muting
/// IS digital silence here, so a muted channel's estimator is reset rather
/// than fed: the meter reads nothing, and the next unmuted block starts from
/// an empty history.
fn meter_output(channel: &mut [f32], muted: bool, meter: &mut TruePeak) -> (f64, f32) {
    if muted {
        channel.fill(0.0);
        meter.reset();
        return (0.0, 0.0);
    }
    meter.process(channel);
    let peak = meter.take();
    let mut sum = 0.0f64;
    for &sample in channel.iter() {
        sum += f64::from(sample).powi(2);
    }
    let rms = if channel.is_empty() {
        0.0
    } else {
        (sum / channel.len() as f64).sqrt()
    };
    (rms, peak)
}

pub(crate) fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl ClapPlugin for TunerPlugin {
    const CLAP_ID: &'static str = "com.nice-tuner.nice-tuner";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A stringed-instrument chromatic tuner");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::NoteDetector,
        ClapFeature::Mono,
        ClapFeature::Utility,
    ];
}

impl Vst3Plugin for TunerPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"NiceTunerGtrTnr1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Fx, Vst3SubCategory::Tools];
}

nice_export_clap!(TunerPlugin);
nice_export_vst3!(TunerPlugin);
