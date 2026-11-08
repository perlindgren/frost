//! STAGING (M4/M5): the tuner UI as a frost `Process`.
//! Becomes `src/tuner_ui.rs` at integration. Design: docs/SCENE.md.
//!
//! Layout (logical px, origin center, y up), designed for 640×400 and scaled
//! by `fit = min(w/640, h/400)`:
//!
//! ```text
//!        "A2"            note,  size 96, y +95
//!   ├───────┼────┤        gauge ticks on the needle band, asinh scale
//!        ▲              needle at x = cents_to_units(smoothed cents)
//!       +4.1¢            cents, size 28, y −5
//!   440.0 Hz · A4=440    freq + ref, size 22 dim, y −45
//!   ▂▃▅ level ▅▃▂        dBFS meter, y −150, halfwidth 240 (0 to −48 dB)
//! ```

use crate::TunerSharedState;
use crate::host_controls::{HostControls, HostParameter};
use crate::settings::Settings;
use frost::{Color, Context, Process, Scene, SceneNode, Shape};
use nice_common::dropdown::DropdownLayout;
use pitch::freq_to_note;
use std::sync::{Arc, atomic::Ordering};

// Clarity gating lives in the detector (Config::min_clarity, audio thread);
// the UI only freshness-gates + level-gates (see process()).
/// Edge scale factor: ±50 ¢ spans ±240 fit units (the ±320 half-width
/// keeps margins). NOTE: cents→position is NOT linear — see
/// [`cents_to_units`]; CENTS_PX only fixes the outer scale.
const CENTS_PX: f32 = 4.8;
/// Half-width of the gauge, in fit units (50 ¢ × CENTS_PX).
const GAUGE_HALF: f32 = 50.0 * CENTS_PX;
/// Default curvature knee of the gauge scale, in cents (the panel can
/// change it live: `Settings::gauge_knee_cents`).
const GAUGE_KNEE_CENTS: f32 = 5.0;
/// Gauge mapping: sign-preserving asinh with a knee (default ±5 ¢, panel
/// param) — near-linear and readable inside a few cents, progressively
/// compressing beyond it. At the default knee the ±3 ¢ verdict spans
/// ~91 of 480 units, 3.2× the linear size; edges still reach ±50 ¢.
/// asinh over a power curve: finite slope at 0, so needle jitter near
/// perfect pitch stays calm even at steep settings.
fn cents_to_units(cents: f32, knee: f32) -> f32 {
    let k = knee.max(1.0);
    let c = cents.clamp(-50.0, 50.0) / k;
    GAUGE_HALF * c.asinh() / (50.0 / k).asinh()
}
/// Default half-width of the in-tune band and verdict, in cents (the
/// panel can change it live: `Settings::in_tune_cents`). Drives both the
/// green zone behind the needle and the color lock; the band's *pixel*
/// width comes from [`cents_to_units`], so scale changes never alter the
/// verdict's meaning in cents.
const IN_TUNE_CENTS: f64 = 3.0;
/// ms of continuous same-note stability before latching
const LATCH_MS: f32 = 400.0;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}
const BG: Color = rgb(0x17, 0x12, 0x26); // deep indigo, frost-style
const AMBER: Color = rgb(0xe8, 0xa3, 0x3d);
const GREEN: Color = rgb(0x4a, 0xde, 0x80);
const DIM: Color = rgb(0x6b, 0x72, 0x80);
const TRACK: Color = rgb(0x2a, 0x24, 0x3d);
/// Dropdown well: distinctly darker than the page so the open list reads as
/// a cut-in surface; a near-page tint was invisible against the background.
const MENU_BG: Color = rgb(0x0d, 0x0a, 0x16);
const HOVER: Color = rgb(0x3a, 0x33, 0x4f);
const FG: Color = rgb(0xe6, 0xe6, 0xea);

/// Static chrome: the background only. Gauge ticks are NOT here — they
/// follow the panel-adjustable curve and are drawn per frame in
/// [`TunerUi::process`] together with the needle they share a mapping
/// with, so chrome and reading cannot disagree about the scale.
pub fn static_scene() -> Scene {
    Scene::new(SceneNode {
        shape: Some(Shape::Background { color: BG }),
        ..Default::default()
    })
}

/// The UI state machine + immediate-mode drawing, run inside frost's frame.
pub struct TunerUi {
    shared: Arc<TunerSharedState>,
    font: Arc<[u8]>,
    icon_font: Arc<[u8]>,
    host_controls: Option<HostControls>,
    received_samples: u64,
    #[cfg(feature = "diag")]
    diag: frost::Diagnostics,
    // smoothed display state
    needle_cents: f32, // EMA'd cents shown on the gauge
    latch_midi: i16,   // -1 = nothing latched
    /// Input meter ballistics (bar + sticky peak marker), fully unit-tested
    /// in [`crate::meter`].
    meter: crate::meter::PeakMeter,
    latch_name: &'static str,
    latch_oct: i8,
    cents_shown: f64,
    freq_shown: f64,
    stable_ms: f32,
    green_t: f32, // 0 = amber, 1 = green (lerped) // decayed meter fill, 0..1
    // ---- settings page (host audio fields are read-only) ----
    page: Page,
    prev_mouse_down: bool,
    settings: Settings,
    #[cfg(feature = "standalone")]
    audio_panel: Option<nice_common::ui::AudioPanel<crate::settings::TunerPreferences>>,
    /// Rising-edge memory for Escape on the settings page.
    esc_held: bool,
    /// The audio settings currently running, updated after a successful
    /// live reconfiguration. Params outside this class apply immediately.
    applied: Settings,
    /// The settings exactly as last SAVED — boot file, APPLY, or RESET.
    /// APPLY is clickable only while the draft differs from this:
    /// when there is nothing to write, there is nothing to press.
    /// when there is nothing to write, there is nothing to press.
    saved: Settings,
    /// True when NO settings file existed at startup: a cold start is
    /// shown as a clean slate — every unset audio row a dash, nothing
    /// narrated. Applies to the whole session (choosing values lights
    /// those rows; APPLY ends coldness for good, next boot is warm).
    cold: bool,
    settings_loaded: bool,
    devices_loaded: bool,
    open_dropdown: Option<Dropdown>,
    list_scroll: f32,
    dropdown_scroll_drag: bool,
    backend_opts: Vec<(String, String)>,
    input_opts: Vec<String>,
    output_opts: Vec<String>,
    output_channel_opts: Vec<crate::settings::OutputChannels>,
    default_output_channels: Option<crate::settings::OutputChannels>,
    rate_opts: Vec<String>,
    output_rate_opts: Vec<String>,
    default_output_rate: Option<f32>,
    /// System-default device names for the current backend: the concrete
    /// value "(default)" hides, shown instead in the device rows.
    default_in: Option<String>,
    default_out: Option<String>,
    /// Capabilities of the input device selected on the settings page.
    input_channels: Option<u32>,
    default_input_channel: Option<u32>,
    /// Rate the driver itself is set to, if known — what "(default)"
    /// resolves to at startup (see main.rs / devices::driver_default_rate).
    driver_rate: Option<f32>,
    /// The backend's last read — devices, defaults, rates, buffers —
    /// taken in-process, or through a streamless child when this one
    /// is ASIO-blind while streaming (devices::snapshot_via_child).
    #[cfg(feature = "standalone")]
    snap: Option<crate::devices::Snapshot>,
    buffer_opts: Vec<String>,
    output_buffer_opts: Vec<String>,
    buffers_linked: bool,
    /// One ASIO driver clocks both directions: the output rate mirrors the
    /// sample rate and cannot be picked apart from it.
    rates_linked: bool,
    save_note: Option<String>,
    #[cfg(feature = "standalone")]
    pending_apply: Option<PendingAudioApply>,
    // ---- analysis. This is the UI thread (nice-plug's main thread, pumped
    // by frost frames): detection runs HERE, never in the audio callback.
    // The consumer half is borrowed from the shared state on open and
    // returned on close (Drop); TunerPlugin owns the producer half. ----
    rb: Option<rtrb::Consumer<f32>>,
    detector: Option<pitch::PitchDetector>,
    det_rate: f64,
    note_frequency: Option<f64>,
    waveform: Waveform,
}

#[cfg(feature = "standalone")]
struct PendingAudioApply {
    result: crossbeam::channel::Receiver<Result<(), String>>,
    settings: Settings,
}

/// Return the analysis feed so the next editor session can take it.
/// Main thread: frost tears the UI down inside the window handler.
impl Drop for TunerUi {
    fn drop(&mut self) {
        if let Some(rb) = self.rb.take() {
            self.shared.put_consumer(rb);
        }
    }
}

impl TunerUi {
    pub fn new(shared: Arc<TunerSharedState>, font: Arc<[u8]>) -> Self {
        let mut rb = shared.take_consumer();
        // A reopened editor analyzes fresh audio, not last session's backlog.
        if let Some(rb) = rb.as_mut() {
            while rb.pop().is_ok() {}
        }
        #[cfg(feature = "diag")]
        let diag =
            frost::Diagnostics::from_bytes(&crate::gui::font(), frost::DiagnosticsFlags::all())
                .expect("The tuner font must support diagnostics")
                .expose_vsync(false);
        Self {
            shared,
            font,
            icon_font: crate::gui::icon_font(),
            host_controls: None,
            received_samples: 0,
            #[cfg(feature = "diag")]
            diag,
            needle_cents: 0.0,
            latch_midi: -1,
            meter: crate::meter::PeakMeter::new(crate::now_epoch_ms()),
            latch_name: "–",
            latch_oct: 0,
            cents_shown: 0.0,
            freq_shown: 0.0,
            stable_ms: 0.0,
            green_t: 0.0,
            page: Page::Main,
            prev_mouse_down: false,
            settings: Settings::default(),
            #[cfg(feature = "standalone")]
            audio_panel: None,
            esc_held: false,
            applied: Settings::default(),
            saved: Settings::default(),
            cold: false,
            settings_loaded: false,
            devices_loaded: false,
            open_dropdown: None,
            list_scroll: 0.0,
            dropdown_scroll_drag: false,
            backend_opts: Vec::new(),
            input_opts: Vec::new(),
            output_opts: Vec::new(),
            output_channel_opts: Vec::new(),
            default_output_channels: None,
            rate_opts: Vec::new(),
            output_rate_opts: Vec::new(),
            default_output_rate: None,
            default_in: None,
            default_out: None,
            input_channels: None,
            default_input_channel: None,
            driver_rate: None,
            #[cfg(feature = "standalone")]
            snap: None,
            buffer_opts: Vec::new(),
            output_buffer_opts: Vec::new(),
            buffers_linked: false,
            rates_linked: false,
            save_note: None,
            #[cfg(feature = "standalone")]
            pending_apply: None,
            rb,
            detector: None,
            det_rate: 0.0,
            #[cfg(feature = "standalone")]
            note_frequency: crate::standalone::NOTE_FREQUENCY.get().copied(),
            #[cfg(not(feature = "standalone"))]
            note_frequency: None,
            waveform: Waveform::default(),
        }
    }

    pub(crate) fn set_host_controls(&mut self, controls: HostControls) {
        self.host_controls = Some(controls);
        self.settings = Settings::default();
        self.settings_loaded = true; // A hosted editor never reads standalone JSON.
        self.cold = false;
        self.sync_host_settings();
        self.applied = self.settings.clone();
        self.saved = self.settings.clone();
    }

    fn is_hosted(&self) -> bool {
        self.host_controls.is_some()
    }

    fn sync_host_settings(&mut self) {
        let Some(host) = &self.host_controls else {
            return;
        };
        let a4 = host.params.a4_ref.value();
        let gate = host.params.gate_db.value();
        self.settings.a4_ref = Some(a4);
        self.settings.gate_db = Some(gate);
        self.settings.in_tune_cents = Some(host.params.in_tune_cents.value());
        self.settings.gauge_knee_cents = Some(host.params.gauge_knee_cents.value());
        self.shared
            .a4_ref_bits
            .store((a4 as f64).to_bits(), Ordering::Relaxed);
        self.shared.gate_db.store(gate);
    }

    fn is_audio_field(dd: Dropdown) -> bool {
        matches!(
            dd,
            Dropdown::Backend
                | Dropdown::Input
                | Dropdown::InputCh
                | Dropdown::Output
                | Dropdown::OutputCh
                | Dropdown::InputBuffer
                | Dropdown::OutputBuffer
                | Dropdown::Rate
                | Dropdown::OutputRate
        )
    }

    fn host_audio_value(&self, dd: Dropdown) -> Option<String> {
        let host = self.host_controls.as_ref()?;
        let inputs = self.shared.input_channels.load();
        let outputs = self.shared.output_channels.load();
        let channels = |count| {
            if count > 0 {
                format!(
                    "{count} {}",
                    if count == 1 { "channel" } else { "channels" }
                )
            } else {
                "Unavailable".into()
            }
        };
        let rate = self.shared.sample_rate_hz.load();
        Some(match dd {
            Dropdown::Backend => format!("{} host", host.api),
            Dropdown::Input => channels(inputs),
            Dropdown::Output => channels(outputs),
            Dropdown::InputCh => {
                if inputs > 0 {
                    format!("{} (analysis)", self.shared.input_channel.load())
                } else {
                    "Unavailable".into()
                }
            }
            Dropdown::OutputCh => match outputs {
                0 => "Unavailable".into(),
                1 => "Mono".into(),
                2 => "Stereo".into(),
                _ => channels(outputs),
            },
            Dropdown::Rate | Dropdown::OutputRate => {
                if rate.is_finite() && rate > 0.0 {
                    fmt_rate(rate as f32)
                } else {
                    "Unavailable".into()
                }
            }
            Dropdown::InputBuffer | Dropdown::OutputBuffer => {
                let frames = self.shared.buffer_frames.load();
                let maximum = self.shared.max_buffer_frames.load();
                if frames > 0 {
                    format!("{frames} frames")
                } else if maximum > 0 {
                    format!("Up to {maximum} frames")
                } else {
                    "Unavailable".into()
                }
            }
            _ => return None,
        })
    }

    fn host_audio_summary(&self) -> String {
        let count = |value: u32| {
            if value > 0 {
                value.to_string()
            } else {
                "?".into()
            }
        };
        let rate = self.shared.sample_rate_hz.load();
        let rate = if rate.is_finite() && rate > 0.0 {
            fmt_rate(rate as f32)
        } else {
            "rate unavailable".into()
        };
        format!(
            "{} in · {} out · {rate}",
            count(self.shared.input_channels.load()),
            count(self.shared.output_channels.load())
        )
    }

    /// Drain the RT feed and run pitch detection — on this (UI) thread, so
    /// the bursty window search can at worst delay the needle, never glitch
    /// audio. Called once per frost frame, including while the settings page
    /// is up (state keeps warming behind it).
    fn analyze(&mut self) {
        let Some(rb) = self.rb.as_mut() else { return };
        let rate = self.shared.sample_rate_hz.load();
        if rate > 0.0
            && (self.det_rate != rate || (self.note_frequency.is_none() && self.detector.is_none()))
        {
            self.detector = self.note_frequency.is_none().then(|| {
                pitch::PitchDetector::new(pitch::Config {
                    sample_rate: rate,
                    min_freq: 30.0,   // 5-string bass B0
                    max_freq: 2100.0, // harmonics up past the guitar's E7 territory
                    ..Default::default()
                })
            });
            self.det_rate = rate;
            self.waveform.reset(rate);
        }
        if self.det_rate <= 0.0 {
            return;
        }
        // Catch-up cap: after a long pump stall, analyze the most recent
        // audio rather than grinding through a stale backlog. Older samples
        // are dropped from the head (Consumer side: legal in SPSC).
        const MAX_DRAIN: usize = 16384; // ~0.34 s
        if rb.slots() > MAX_DRAIN {
            self.waveform.samples.clear();
            let skip = rb.slots() - MAX_DRAIN;
            self.waveform.pending_frames += skip;
            for _ in 0..skip {
                let _ = rb.pop();
            }
        }
        const CHUNK: usize = 4096;
        let mut buf = [0f32; CHUNK];
        // Meter levels come from the audio thread (pre-mix, hottest
        // channel RMS + sample peak): averaging the mono downmix would
        // understate one live channel beside a silent sibling by 6 dB.
        let mut drained = false;
        while rb.slots() > 0 {
            let want = rb.slots().min(CHUNK);
            if rb.pop_entire_slice(&mut buf[..want]).is_err() {
                break;
            }
            self.waveform.push(&buf[..want]);
            self.received_samples = self.received_samples.wrapping_add(want as u64);
            drained = true;
            if let Some(hz) = self.note_frequency {
                self.shared.freq_hz.store(hz);
                self.shared.clarity.store(1.0);
                self.shared.rms.store(self.shared.input_rms.load());
                self.shared
                    .last_detection_ms
                    .store(crate::now_epoch_ms(), Ordering::Relaxed);
            } else if let Some(d) = self
                .detector
                .as_mut()
                .and_then(|detector| detector.push_samples(&buf[..want]))
            {
                self.shared.freq_hz.store(d.freq_hz);
                self.shared.clarity.store(d.clarity);
                self.shared.rms.store(d.rms);
                self.shared
                    .last_detection_ms
                    .store(crate::now_epoch_ms(), Ordering::Relaxed);
            }
        }
        // Two-layer metering, both updated every frame:
        //   bar   — current level (RMS), EMA with fast attack / moderate
        //           release, always chasing the ongoing signal;
        //   peak  — the max of frame peaks, STICKS for 1 s (wall-clock, so
        //           the hold is fps-independent); any frame that beats the
        //           held peak jumps to it and RESETS the 1 s timer; once
        //           expired it eases down toward the current bar level.
        let db_of = |a: f64| -> f32 {
            if a > 1e-7 {
                (20.0 * a.log10()) as f32
            } else {
                -120.0
            }
        };
        let (rms_db, frame_peak_db) = if drained {
            (
                db_of(self.shared.input_rms.load() as f64),
                db_of(self.shared.input_peak.load() as f64),
            )
        } else {
            (-120.0, -120.0)
        };
        self.meter
            .tick(rms_db, frame_peak_db, crate::now_epoch_ms());
    }
}

impl Process for TunerUi {
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        #[cfg(feature = "diag")]
        self.diag.process(ctx, dt);
        let dt = dt.clamp(0.0, 0.25); // tolerate host pauses
        // Standalone reads its settings file on the first frame. Hosted editors
        // already seeded their settings from parameters in set_host_controls().
        if !self.settings_loaded {
            let (settings, _) = Settings::load();
            // Cold: no file, or a file that holds no decision (a bare
            // geometry written by window moves does not count).
            self.cold = !Settings::exists() || settings.is_blank();
            self.applied = settings.clone();
            self.saved = settings.clone();
            self.settings = settings;
            self.settings_loaded = true;
            #[cfg(feature = "standalone")]
            if let Some((applied, error)) = crate::STANDALONE_BOOT.get() {
                self.applied = applied.clone();
                if let Some(error) = error {
                    self.open_settings_page();
                    self.save_note = Some(error.clone());
                }
            }
        }
        self.sync_host_settings();
        #[cfg(feature = "standalone")]
        if !self.is_hosted() {
            self.poll_audio_apply();
        }
        self.analyze();
        self.waveform.advance(dt);

        // ---- read shared state (updated by analyze(), above) --------------
        // The detector already applied its clarity gate
        // (push_samples only yields confident windows), so "signal present"
        // here = a *fresh* detection + level above the user gate.
        let freq = self.shared.freq_hz.load();
        let rms = self.shared.rms.load() as f64;
        let a4 = self.shared.a4_ref();
        let gate_db = self.shared.gate_db.load() as f64;
        let rms_db = if rms > 1e-7 {
            20.0 * rms.log10()
        } else {
            -120.0
        };
        let last_ms = self.shared.last_detection_ms.load(Ordering::Relaxed);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let fresh = last_ms != 0 && now_ms.saturating_sub(last_ms) < 500;
        let signal = fresh && rms_db >= gate_db;

        // ---- note latch / stability ---------------------------------------
        if let Some(note) = signal.then(|| freq_to_note(freq, a4)) {
            if note.midi == self.latch_midi {
                self.stable_ms += dt * 1000.0;
            } else {
                self.latch_midi = note.midi;
                self.latch_name = note.name;
                self.latch_oct = note.octave;
                self.stable_ms = 0.0;
            }
            self.cents_shown = note.cents;
            self.freq_shown = freq;
        } else {
            self.stable_ms = (self.stable_ms - dt * 500.0).max(0.0);
        }
        let stable = self.stable_ms >= LATCH_MS;
        let in_tune = signal
            && stable
            && self.cents_shown.abs()
                <= self.settings.in_tune_cents.unwrap_or(IN_TUNE_CENTS as f32) as f64;

        // ---- color lerp amber→green (≈200 ms) ------------------------------
        let target = if in_tune { 1.0 } else { 0.0 };
        let k = 1.0 - (-dt / 0.08).exp(); // EMA factor
        self.green_t += (target - self.green_t) * k;
        let main = Color {
            r: AMBER.r + (GREEN.r - AMBER.r) * self.green_t,
            g: AMBER.g + (GREEN.g - AMBER.g) * self.green_t,
            b: AMBER.b + (GREEN.b - AMBER.b) * self.green_t,
            a: 1.0,
        };
        let main = if signal { main } else { DIM };

        // ---- needle: EMA toward clamped cents, spring to 0 when silent ----
        let wanted = if signal {
            (self.cents_shown.clamp(-50.0, 50.0) as f32) * 1.0
        } else {
            0.0
        };
        let nk = if signal {
            1.0 - (-dt / 0.08).exp()
        } else {
            1.0 - (-dt / 0.15).exp()
        };
        self.needle_cents += (wanted - self.needle_cents) * nk;

        // ---- draw (Context derefs to Canvas) -------------------------------
        let (w, h) = ctx.size();
        let fit = (w / 640.0).min(h / 400.0).max(0.35);

        // Mouse edge + page routing. The settings page exists only in the
        // standalone app: inside a host, the host owns the audio system.
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let clicked = down && !self.prev_mouse_down;
        self.prev_mouse_down = down;
        if self.page == Page::Settings {
            self.process_settings_page(ctx, clicked);
            return;
        }
        {
            let gx = w / 2.0 - 26.0 * fit;
            let gy = h / 2.0 - 22.0 * fit;
            let hov = point_in(ctx.mouse_position(), gx, gy, 16.0 * fit, 16.0 * fit);
            ctx.rectangle(
                gx,
                gy,
                16.0 * fit,
                16.0 * fit,
                if hov { HOVER } else { TRACK },
                0.4,
            );
            ctx.text(
                gx,
                gy,
                &self.icon_font,
                "\u{f6aa}", // ic_fluent_settings_24_regular (see local glyph map)
                24.0 * fit,
                400.0,
                if hov { AMBER } else { DIM },
                0.41,
            );
            if hov && clicked {
                self.open_settings_page();
                return;
            }
        }
        if self.is_hosted() {
            let summary = self.host_audio_summary();
            let size = 16.2 * fit;
            let half_width = ctx.text_width(&self.font, &summary, size, 550.0) / 2.0;
            ctx.text(
                -w / 2.0 + 16.0 * fit + half_width,
                h / 2.0 - 18.0 * fit,
                &self.font,
                summary,
                size,
                550.0,
                DIM,
                0.4,
            );
        }
        let font = &self.font;
        let knee = self.settings.gauge_knee_cents.unwrap_or(GAUGE_KNEE_CENTS);
        let in_tune_cents = self.settings.in_tune_cents.unwrap_or(IN_TUNE_CENTS as f32);
        let x = cents_to_units(self.needle_cents, knee) * fit;
        // The drawn extents, named once. The click regions at the end of this
        // function use these same values, so a resized window or a steeper curve
        // moves what answers to a click along with what is on screen.
        let gauge_y = -2.0 * fit;
        let band_hh = 9.0 * fit;
        let gauge_hh = 16.0 * fit;
        let band_half = cents_to_units(in_tune_cents, knee) * fit;
        let gauge_half = cents_to_units(50.0, knee) * fit;
        let meter_half = 240.0 * fit;
        let meter_y = -150.0 * fit;
        let tick_drop = 26.0 * fit;
        let freq_y = -104.0 * fit;

        // Three periods of the ideal note frequency (including A tuning), in the free area
        // above the gauge. All sample retention and drawing stay on the UI thread.
        if signal {
            let sweep_hz = freq_to_note(self.freq_shown, a4).nearest_freq;
            let traces = self.waveform.traces(sweep_hz, 481);
            for (i, points) in traces.iter().enumerate() {
                let ink = if i == 0 {
                    Color {
                        a: main.a * 0.3,
                        ..main
                    }
                } else {
                    main
                };
                for pair in points.windows(2) {
                    ctx.line(
                        pair[0][0] * 240.0 * fit,
                        (95.0 + pair[0][1] * 38.0) * fit,
                        pair[1][0] * 240.0 * fit,
                        (95.0 + pair[1][1] * 38.0) * fit,
                        ink,
                        1.5 * fit,
                        0.2,
                    );
                }
            }
        }

        // Gauge ticks: dynamic (they follow the panel-adjustable curve),
        // sharing the needle's mapping so they can never lie about it.
        for cents in (-50..=50).step_by(10) {
            let tx = cents_to_units(cents as f32, knee) * fit;
            let long = cents % 50 == 0;
            ctx.rectangle(
                tx,
                gauge_y,
                0.75 * fit,
                if long { 9.0 } else { 4.5 } * fit,
                if long { DIM } else { TRACK },
                0.04,
            );
        }

        // in-tune zone behind the needle — panel-adjustable width, same
        // nonlinear mapping, so its edges sit exactly where the needle
        // reads ±the configured window
        ctx.rectangle(
            0.0,
            gauge_y,
            band_half,
            band_hh,
            Color { a: 0.15, ..GREEN },
            0.05,
        );
        // needle
        ctx.line(
            x,
            gauge_y - gauge_hh,
            x,
            gauge_y + gauge_hh,
            main,
            2.5 * fit,
            0.2,
        );
        // note label
        // Nothing latched yet: a plain dash — not the octave-0 placeholder
        // that used to leak out as "–0".
        let label = if self.latch_midi < 0 {
            "–".to_string()
        } else {
            format!("{}{}", self.latch_name, self.latch_oct)
        };
        // Fixed columns on the original cents row; neither depends on the
        // current note width, cents sign or number of visible digits.
        ctx.text(
            -140.0 * fit,
            -52.0 * fit,
            font,
            label,
            39.6 * fit,
            700.0,
            main,
            0.2,
        );
        ctx.text(0.0, -52.0 * fit, font, "·", 39.6 * fit, 600.0, main, 0.2);
        // cents readout: TABULAR on a fixed pitch — every glyph lands in
        // its own digit-width slot (see draw_tabular). Constant glyph
        // count alone was not enough: `ctx.text` centers the INK box,
        // and `+` versus `−` carry visibly different ink widths, so the
        // whole line shifted a hair whenever the sign flipped. Slotted
        // layout cannot: nothing depends on what the glyphs look like.
        // Anchor the decimal to the right column, where its idle dash sits.
        // No signal, no reading: a bare dash, not a confident "+00.0¢".
        let cents_s = if signal {
            let cents = self.cents_shown;
            trim_zeros(&format!(
                "{}{:04.1}¢",
                if cents >= 0.0 { "+" } else { "−" },
                cents.abs()
            ))
        } else {
            "—".to_string()
        };
        self.draw_tabular(
            ctx,
            140.0 * fit,
            -52.0 * fit,
            &cents_s,
            '.',
            39.6 * fit,
            600.0,
            main,
            0.2,
        );
        // Right-align the frequency before fixed Hz and reference columns.
        // The midpoint marker keeps both columns fixed; the decimal
        // replaces the idle frequency dash.
        let freq_s = frequency_readout(signal.then_some(self.freq_shown), a4);
        self.draw_tabular(ctx, 0.0, freq_y, &freq_s, '·', 39.6 * fit, 550.0, DIM, 0.2);
        // ---- input meter with a real dBFS scale ----------------------------
        // 0 dBFS right, -48 left, ticks every 6 dB, as wide as the needle
        // sweep. Driven by meter_db (continuous), so it shows input level
        // always — silent room reads silence, no detection required.
        {
            let half = meter_half;
            let y = meter_y;
            ctx.rectangle(0.0, y, half, 2.5 * fit, Color { a: 0.5, ..TRACK }, 0.05);
            // dBFS scale: 0 at the right end, -48 at the left, every 6 dB.
            // Short ticks (±4) with numerals well below — digit tops must
            // not crowd the ticks.
            for i in 0..=8u32 {
                let xv = half - i as f32 * (half / 4.0);
                ctx.rectangle(xv, y, 1.0 * fit, 4.0 * fit, Color { a: 0.6, ..DIM }, 0.06);
                ctx.text(
                    xv,
                    y - tick_drop,
                    font,
                    format!("{}", -(6 * i as i32)),
                    16.0 * fit,
                    550.0,
                    DIM,
                    0.06,
                );
            }
            // Bar: current level.
            let m = self.meter.level_db().clamp(-48.0, 0.0);
            let xv = half * (1.0 + m / 24.0);
            if xv > -half + 0.5 {
                ctx.rectangle(
                    (-half + xv) / 2.0,
                    y,
                    (xv + half) / 2.0,
                    2.5 * fit,
                    // Same amber→green state as note and needle (main lerps).
                    main,
                    0.07,
                );
            }
            // Peak marker: a thin bright line at the held peak, above the
            // bar in z; sits at the bar while rising, sticks after it falls.
            let p = self.meter.peak_db().clamp(-48.0, 0.0);
            let px = half * (1.0 + p / 24.0);
            if p > -47.5 {
                ctx.rectangle(px, y, 1.5 * fit, 6.0 * fit, Color { a: 0.9, ..FG }, 0.08);
            }
        }

        // Escape closes an open dropdown: the same rising edge the settings page
        // uses, and the same rule - it un-changes, it never writes.
        let esc = ctx.key_down(frost::KeyCode::Escape);
        if esc && !self.esc_held && self.open_dropdown.take().is_some() {
            self.list_scroll = 0.0;
        }
        self.esc_held = esc;
        // Clicking a control where it is shown opens the setting it displays: the
        // green band is the in-tune window, the gauge behind it its curve, the dB
        // scale the gate, and the frequency column the A4 reference it is computed
        // from. The inner region is tested first on purpose — the band sits inside
        // the gauge, and the one you aimed at must be the one that answers.
        // Selection goes through the same `pick` the settings page uses, so dirty
        // state, Apply and persistence cannot diverge between the two.
        let mouse = ctx.mouse_position();
        // The band is the drawn band, the gauge reaches its outermost tick, the meter
        // covers the bar and the numerals hung below it, and the frequency column
        // takes the reference — all of it scaled by the same it the drawing uses.
        let wanted = if point_in(mouse, 0.0, gauge_y, band_half, band_hh) {
            Some(Dropdown::InTune)
        } else if point_in(mouse, 0.0, gauge_y, gauge_half, gauge_hh) {
            Some(Dropdown::GaugeCurve)
        } else if point_in(
            mouse,
            0.0,
            meter_y - tick_drop / 2.0,
            meter_half,
            tick_drop / 2.0 + 4.0 * fit,
        ) {
            Some(Dropdown::Gate)
        } else if point_in(mouse, 0.0, freq_y, meter_half, 20.0 * fit) {
            Some(Dropdown::A4)
        } else {
            None
        };
        // The popup is drawn over these regions, so it takes the click first and the
        // regions only see what it refused.
        let mut consumed = false;
        if self.open_dropdown.is_some() {
            consumed = self.draw_dropdown(
                ctx,
                [w, h],
                fit,
                Placement::Centred([110.0 * fit, 11.0 * fit]),
                0.5,
                clicked,
            );
        }
        if clicked && !consumed {
            match wanted {
                Some(dd) if self.open_dropdown == Some(dd) => self.open_dropdown = None,
                Some(dd) => {
                    self.open_dropdown = Some(dd);
                    self.list_scroll = 0.0;
                    self.dropdown_scroll_drag = false;
                }
                None => self.open_dropdown = None,
            }
        }
        // Hovering one of those regions says what it is and what it currently holds,
        // in the same words the dropdown will use, so the click is not a guess about
        // an unlabelled strip of pixels. Suppressed while a dropdown is open: the
        // popup covers this part of the view and a tooltip under it would be noise.
        if self.open_dropdown.is_none()
            && let (Some(dd), Some([mx, my])) = (wanted, mouse)
        {
            let mut rows = self.dropdown_rows(dd).0;
            let at = self.selected_index(dd).min(rows.len() - 1);
            let text = format!("{}  {}", setting_name(dd), rows.swap_remove(at).0);
            let size = 15.0 * fit;
            let half_w = ctx.text_width(&self.font, &text, size, 550.0) / 2.0 + 9.0 * fit;
            let half_h = 12.0 * fit;
            let margin = 6.0 * fit;
            // Up and to the right of the pointer, then pushed back inside the
            // window: the gauge and the meter run to both edges, so a tooltip that
            // only knew its preferred offset would leave the screen.
            let cx = (mx + half_w + 16.0 * fit)
                .min(w / 2.0 - margin - half_w)
                .max(-w / 2.0 + margin + half_w);
            let cy = (my + half_h + 22.0 * fit)
                .min(h / 2.0 - margin - half_h)
                .max(-h / 2.0 + margin + half_h);
            ctx.rectangle(cx, cy, half_w, half_h, MENU_BG, 0.6);
            ctx.text(cx, cy, &self.font, text, size, 550.0, FG, 0.61);
        }
    }
}

// ---- input mapping (baseview 0.3.3 / keyboard_types 0.8 → frost) ----------

pub fn map_key(key: &keyboard_types::Key) -> Option<frost::KeyCode> {
    use keyboard_types::{Key, NamedKey};
    match key {
        Key::Character(s) => match s.as_str() {
            "a" | "A" => Some(frost::KeyCode::KeyA),
            "b" | "B" => Some(frost::KeyCode::KeyB),
            "c" | "C" => Some(frost::KeyCode::KeyC),
            "d" | "D" => Some(frost::KeyCode::KeyD),
            "e" | "E" => Some(frost::KeyCode::KeyE),
            "f" | "F" => Some(frost::KeyCode::KeyF),
            "g" | "G" => Some(frost::KeyCode::KeyG),
            "h" | "H" => Some(frost::KeyCode::KeyH),
            "i" | "I" => Some(frost::KeyCode::KeyI),
            "j" | "J" => Some(frost::KeyCode::KeyJ),
            "k" | "K" => Some(frost::KeyCode::KeyK),
            "l" | "L" => Some(frost::KeyCode::KeyL),
            "m" | "M" => Some(frost::KeyCode::KeyM),
            "n" | "N" => Some(frost::KeyCode::KeyN),
            "o" | "O" => Some(frost::KeyCode::KeyO),
            "p" | "P" => Some(frost::KeyCode::KeyP),
            "q" | "Q" => Some(frost::KeyCode::KeyQ),
            "r" | "R" => Some(frost::KeyCode::KeyR),
            "s" | "S" => Some(frost::KeyCode::KeyS),
            "t" | "T" => Some(frost::KeyCode::KeyT),
            "u" | "U" => Some(frost::KeyCode::KeyU),
            "v" | "V" => Some(frost::KeyCode::KeyV),
            "w" | "W" => Some(frost::KeyCode::KeyW),
            "x" | "X" => Some(frost::KeyCode::KeyX),
            "y" | "Y" => Some(frost::KeyCode::KeyY),
            "z" | "Z" => Some(frost::KeyCode::KeyZ),
            "0" => Some(frost::KeyCode::Digit0),
            "1" => Some(frost::KeyCode::Digit1),
            "2" => Some(frost::KeyCode::Digit2),
            "3" => Some(frost::KeyCode::Digit3),
            "4" => Some(frost::KeyCode::Digit4),
            "5" => Some(frost::KeyCode::Digit5),
            "6" => Some(frost::KeyCode::Digit6),
            "7" => Some(frost::KeyCode::Digit7),
            "8" => Some(frost::KeyCode::Digit8),
            "9" => Some(frost::KeyCode::Digit9),
            " " => Some(frost::KeyCode::Space),
            _ => None,
        },
        Key::Named(named) => Some(match named {
            NamedKey::Escape => frost::KeyCode::Escape,
            NamedKey::Enter => frost::KeyCode::Enter,
            NamedKey::Tab => frost::KeyCode::Tab,
            NamedKey::Backspace => frost::KeyCode::Backspace,
            NamedKey::Delete => frost::KeyCode::Delete,
            NamedKey::ArrowLeft => frost::KeyCode::ArrowLeft,
            NamedKey::ArrowRight => frost::KeyCode::ArrowRight,
            NamedKey::ArrowUp => frost::KeyCode::ArrowUp,
            NamedKey::ArrowDown => frost::KeyCode::ArrowDown,
            _ => return None,
        }),
    }
}

pub fn map_mouse(button: baseview::MouseButton) -> Option<frost::MouseButton> {
    Some(match button {
        baseview::MouseButton::Left => frost::MouseButton::Left,
        baseview::MouseButton::Right => frost::MouseButton::Right,
        baseview::MouseButton::Middle => frost::MouseButton::Middle,
        _ => return None,
    })
}

// ============================================================================
// Settings page — backend/device/buffer configuration for the standalone app.
// Hand-rolled widgets on frost's immediate canvas (rectangles + text + mouse
// edges); the static scene underneath is page-agnostic.
// ============================================================================

/// Where an open dropdown is put: hanging under the control that opened it, as a
/// settings row does, or in the middle of the window for a control painted on the
/// main view, which has no row beneath it.
enum Placement {
    Row([f32; 2], [f32; 2]),
    Centred([f32; 2]),
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Main,
    Settings,
}

#[derive(Clone, Copy, PartialEq)]
enum Dropdown {
    Backend,
    Rate,
    InputBuffer,
    OutputBuffer,
    Input,
    Output,
    OutputCh,
    OutputRate,
    /// Plugin-param pickers. In a host the host renders these controls;
    /// standalone has none, so the panel plays that role — same atomics the
    /// `process()` mirror feeds in host mode (see lib.rs).
    InputCh,
    A4,
    Gate,
    /// Editor look/judgement params (verdict window, gauge steepness).
    InTune,
    GaugeCurve,
}

/// What a centred popup calls the setting it is offering: it floats in the middle
/// of the window with no row beside it to say where it came from.
fn setting_name(dd: Dropdown) -> &'static str {
    match dd {
        Dropdown::Backend => "AUDIO BACKEND",
        Dropdown::Rate => "SAMPLE RATE",
        Dropdown::InputBuffer => "INPUT BUFFER SIZE",
        Dropdown::OutputBuffer => "OUTPUT BUFFER SIZE",
        Dropdown::Input => "INPUT DEVICE",
        Dropdown::Output => "OUTPUT DEVICE",
        Dropdown::OutputCh => "OUTPUT CHANNELS",
        Dropdown::OutputRate => "OUTPUT SAMPLE RATE",
        Dropdown::InputCh => "INPUT CHANNEL",
        Dropdown::A4 => "A4 REFERENCE",
        Dropdown::Gate => "INPUT GATE",
        Dropdown::InTune => "IN-TUNE WINDOW",
        Dropdown::GaugeCurve => "GAUGE CURVE",
    }
}

/// Sensible tuning-standards list; 440 sits mid-list.
const A4_OPTIONS: [f32; 9] = [
    432.0, 438.0, 439.0, 440.0, 441.0, 442.0, 443.0, 444.0, 446.0,
];
const GATE_OPTIONS: [f32; 7] = [-80.0, -70.0, -60.0, -50.0, -40.0, -30.0, -20.0];
/// Verdict / green-band half-width choices, cents (3.0 = the original).
const IN_TUNE_OPTIONS: [f32; 7] = [1.0, 2.0, 3.0, 4.0, 5.0, 7.0, 10.0];
/// Gauge curvature knee choices, cents (5.0 = the original steep middle).
const GAUGE_KNEE_OPTIONS: [f32; 6] = [3.0, 4.0, 5.0, 7.0, 10.0, 20.0];

fn point_in(p: Option<[f32; 2]>, cx: f32, cy: f32, hw: f32, hh: f32) -> bool {
    p.is_some_and(|[x, y]| (x - cx).abs() <= hw && (y - cy).abs() <= hh)
}

fn clip_port(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        return s.to_owned();
    }
    // Keep the channel/port suffix visible, where JACK names usually differ.
    let head = n / 3;
    let tail = n.saturating_sub(head + 1);
    format!(
        "{}…{}",
        chars[..head].iter().collect::<String>(),
        chars[chars.len() - tail..].iter().collect::<String>()
    )
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

/// UI-owned rolling history; 0.22 seconds covers two three-period sweeps near the
/// detector's 30 Hz lower bound. No changes to the audio callback are needed.
#[derive(Default)]
struct Waveform {
    samples: std::collections::VecDeque<f32>,
    rate: f64,
    limit: usize,
    sweep_time: f64,
    pending_frames: usize,
}

impl Waveform {
    fn reset(&mut self, rate: f64) {
        self.rate = rate;
        self.sweep_time = 0.0;
        self.pending_frames = 0;
        self.limit = (rate * 0.22).ceil() as usize + 2;
        self.samples = std::collections::VecDeque::with_capacity(self.limit);
    }

    // Predict elapsed sweep time using the frame dt, then correct to the
    // actual audio clock. Callback batches and UI stalls must not introduce
    // phase jitter; with no new audio, keep the last capture stationary.
    fn advance(&mut self, dt: f32) {
        if self.rate <= 0.0 {
            return;
        }
        let dt = f64::from(dt.max(0.0));
        let predicted = self.sweep_time + dt;
        self.sweep_time = predicted + (self.pending_frames as f64 / self.rate - dt);
        self.pending_frames = 0;
    }

    fn push(&mut self, samples: &[f32]) {
        self.pending_frames += samples.len();
        for &sample in samples {
            if self.samples.len() == self.limit {
                self.samples.pop_front();
            }
            self.samples
                .push_back(if sample.is_finite() { sample } else { 0.0 });
        }
    }

    // Keep the last complete sweep and overlay the current partial sweep.
    // Never splice samples of different ages inside a single connected trace.
    fn traces(&self, frequency: f64, count: usize) -> [Vec<[f32; 2]>; 2] {
        let empty = || [Vec::new(), Vec::new()];
        if !frequency.is_finite() || frequency <= 0.0 || count < 2 || self.samples.is_empty() {
            return empty();
        }
        let span = 3.0 * self.rate / frequency;
        if !span.is_finite() || span <= 0.0 {
            return empty();
        }
        let end = self.sweep_time * self.rate - 1.0;
        let boundary = (end / span).floor() * span;
        let oldest = end - (self.samples.len() - 1) as f64;
        let peak = self
            .samples
            .iter()
            .fold(0.0f32, |p, v| p.max(v.abs()))
            .max(1e-6);
        let sample = |at: f64| {
            let index = (at - oldest).clamp(0.0, (self.samples.len() - 1) as f64);
            let lo = index.floor() as usize;
            let hi = (lo + 1).min(self.samples.len() - 1);
            (self.samples[lo] * (1.0 - index.fract() as f32)
                + self.samples[hi] * index.fract() as f32)
                / peak
        };
        let trace = |start: f64, stop: f64| {
            if start < oldest || start < 0.0 || stop <= start {
                return Vec::new();
            }
            let fraction = ((stop - start) / span).min(1.0);
            let steps = ((count - 1) as f64 * fraction).ceil() as usize;
            (0..=steps)
                .map(|i| {
                    let t = (i as f64 / (count - 1) as f64).min(fraction);
                    [(t * 2.0 - 1.0) as f32, sample(start + t * span)]
                })
                .collect()
        };
        [trace(boundary - span, boundary), trace(boundary, end)]
    }
}

/// Blank padding zeros without moving the remaining slots. Keep the zero
/// immediately before a decimal point: "+00.7¢" → "+ 0.7¢", "0.0" → "0.0".
fn trim_zeros(s: &str) -> String {
    let mut c: Vec<char> = s.chars().collect();
    // Skip a leading sign so the zero run is found where it lives.
    let start = usize::from(matches!(
        c.first(),
        Some('+') | Some('-') | Some('\u{2212}') | Some(' ')
    ));
    let mut end = start;
    while end < c.len() && c[end] == '0' {
        end += 1;
    }
    if end > start && c.get(end) == Some(&'.') {
        end -= 1;
    }
    for x in &mut c[start..end] {
        *x = ' ';
    }
    c.into_iter().collect()
}

/// Place the requested anchor glyph at the origin, or center the slots if it
/// is absent (the idle cents dash). Blanks occupy slots without being drawn.
fn tabular_glyphs(text: &str, pitch: f32, anchor: char) -> impl Iterator<Item = (char, f32)> + '_ {
    let anchor = text
        .chars()
        .position(|ch| ch == anchor)
        .map_or_else(|| (text.chars().count() as f32 - 1.0) / 2.0, |i| i as f32);
    text.chars()
        .enumerate()
        .filter_map(move |(i, ch)| (ch != ' ').then_some((ch, (i as f32 - anchor) * pitch)))
}

fn frequency_readout(frequency: Option<f64>, a4: f64) -> String {
    let frequency = frequency.map_or_else(
        || "    — ".to_owned(),
        |hz| trim_zeros(&format!("{hz:06.1}")),
    );
    format!("{frequency} Hz  ·  A4={a4:.0}")
}

fn fmt_rate(r: f32) -> String {
    if r.fract() == 0.0 {
        format!("{} Hz", r as u32)
    } else {
        format!("{r:.1} Hz")
    }
}

impl TunerUi {
    fn open_settings_page(&mut self) {
        #[cfg(feature = "standalone")]
        {
            self.audio_panel = None;
        }

        if self.is_hosted() {
            self.sync_host_settings();
            self.open_dropdown = None;
            self.save_note = None;
            self.page = Page::Settings;
            return;
        }
        #[cfg(feature = "standalone")]
        if crate::standalone::control::available() {
            self.page = Page::Settings;
            return;
        }
        // File contents arrive on the first frame (see process); opening
        // the page additionally pays for device enumeration, which can
        // load ASIO drivers and must stay lazy.
        if !self.devices_loaded {
            let (_, warning) = Settings::load();
            self.save_note = warning;
            #[cfg(feature = "standalone")]
            {
                self.backend_opts = crate::devices::available_backends()
                    .into_iter()
                    .map(|b| (b.id, b.label))
                    .collect();
            }
            self.refresh_devices();
            self.devices_loaded = true;
        } else if self.settings.backend.as_deref() == Some("jack") {
            self.refresh_devices();
        }
        self.open_dropdown = None;
        self.page = Page::Settings;
    }

    /// nice-plug's backend key for the devices module (standalone only;
    /// the plugin build has no devices to enumerate). Unset means what
    /// effective_args will boot: `dummy` — the panel enumerates nothing,
    /// exactly as the app connects to nothing.
    #[cfg(feature = "standalone")]
    fn backend_key(&self) -> &str {
        self.settings
            .backend
            .as_deref()
            .unwrap_or(crate::settings::BACKEND_WHEN_UNSET)
    }

    /// (Re-)enumerate devices for the selected backend; selections that the
    /// new backend cannot offer fall back to "(system default)".
    /// Read the current backend ONCE: devices, defaults, rates, buffers.
    ///
    /// ASIO is asked through a child process, because asking in process is not
    /// free while audio runs: the SDK keeps ONE loaded driver per process and
    /// the temporary handle a capability query opens shuts that driver down on
    /// the way out (`asio-sys` 0.4.0 `bindings/mod.rs:1035`), so a query here
    /// would silence the live stream with nothing reported. The child opens no
    /// stream, so it loads the SELECTED driver freely and answers its real
    /// rates, channels and buffer ranges. The parent's own view is unioned in
    /// as well — while a stream is running its ASIO capability answers are
    /// unknowns by design (see `devices::asio_stream_live`), so the union adds
    /// driver names, which cost nothing to read, and nothing else.
    #[cfg(feature = "standalone")]
    fn take_snapshot(&mut self) {
        let backend = self.backend_key().to_owned();
        let input = self.settings.input_device.clone();
        let output = self.settings.output_device.clone();
        let in_proc = || crate::devices::snapshot(&backend, input.as_deref(), output.as_deref());
        self.snap = if backend == "asio" {
            match crate::devices::snapshot_via_child(&backend, input.as_deref(), output.as_deref())
            {
                Some(mut s) => {
                    let p = in_proc();
                    s.inputs.extend(p.inputs);
                    s.inputs.sort();
                    s.inputs.dedup();
                    s.outputs.extend(p.outputs);
                    s.outputs.sort();
                    s.outputs.dedup();
                    s.rates.extend(p.rates);
                    s.rates.sort_by(f32::total_cmp);
                    s.rates.dedup();
                    s.buffers.extend(p.buffers);
                    s.buffers.sort_unstable();
                    s.buffers.dedup();
                    s.output_buffers.extend(p.output_buffers);
                    s.output_buffers.sort_unstable();
                    s.output_buffers.dedup();
                    if s.output_rates.is_empty() {
                        s.output_rates = p.output_rates;
                    }
                    if s.default_output_rate.is_none() {
                        s.default_output_rate = p.default_output_rate;
                    }
                    if s.output_channel_options.is_empty() {
                        s.output_channel_options = p.output_channel_options;
                    }
                    if s.default_output_channels.is_none() {
                        s.default_output_channels = p.default_output_channels;
                    }
                    if s.output_buffer_pref.is_none() {
                        s.output_buffer_pref = p.output_buffer_pref;
                    }
                    if s.def_in.is_none() {
                        s.def_in = p.def_in;
                    }
                    if s.def_out.is_none() {
                        s.def_out = p.def_out;
                    }
                    if s.driver_rate.is_none() {
                        s.driver_rate = p.driver_rate;
                    }
                    if s.buffer_pref.is_none() {
                        s.buffer_pref = p.buffer_pref;
                    }
                    if s.input_channels.is_none() {
                        s.input_channels = p.input_channels;
                    }
                    if s.default_input_channel.is_none() {
                        s.default_input_channel = p.default_input_channel;
                    }
                    Some(s)
                }
                None => Some(in_proc()),
            }
        } else {
            Some(in_proc())
        };
        if let Some(frames) = self.live_jack_buffer()
            && let Some(snapshot) = self.snap.as_mut()
        {
            snapshot.buffer_pref = Some(frames);
            snapshot.output_buffer_pref = Some(frames);
        }
    }

    /// The snapshot's stream side onto the option lists, with the
    /// stored-value keep rule (a value the settings file holds is
    /// never dropped just because this moment's enumeration missed it).
    #[cfg(feature = "standalone")]
    fn apply_stream_snap(&mut self) {
        let Some(s) = self.snap.clone() else {
            return;
        };
        self.rate_opts = s.rates.iter().map(|r| fmt_rate(*r)).collect();
        self.output_rate_opts = s.output_rates.iter().map(|r| fmt_rate(*r)).collect();
        self.default_output_rate = s.default_output_rate;
        if self
            .settings
            .output_sample_rate
            .is_some_and(|rate| !s.output_rates.contains(&rate))
        {
            self.settings.output_sample_rate = None;
        }
        self.buffer_opts = s.buffers.iter().map(|b| b.to_string()).collect();
        self.output_buffer_opts = s.output_buffers.iter().map(|b| b.to_string()).collect();
        self.set_buffers_linked(s.buffers_linked);
        self.set_rates_linked(s.rates_linked);
        self.apply_input_channels(s.input_channels, s.default_input_channel);
        self.output_channel_opts = s.output_channel_options.clone();
        self.default_output_channels = s.default_output_channels;
        if self
            .settings
            .output_channels
            .is_some_and(|route| !self.output_channel_opts.contains(&route))
        {
            self.settings.output_channels = None;
        }
        // QUERIED, REMEMBERED: a driver rate once learned is never
        // cleared by a later blind refresh (driver busy, enumeration
        // hiccup) — the page would forget the truth in front of the
        // user's eyes. New answers overwrite; silence keeps the last.
        self.driver_rate = s.driver_rate;
        if let Some(p) = self.settings.period_size
            && !self.buffer_opts.contains(&p.to_string())
        {
            self.buffer_opts.insert(0, p.to_string());
        }
        if let Some(p) = self.settings.output_period_size
            && !self.output_buffer_opts.contains(&p.to_string())
        {
            self.output_buffer_opts.insert(0, p.to_string());
        }
        if let Some(r) = self.settings.sample_rate
            && !self.rate_opts.contains(&fmt_rate(r))
        {
            self.rate_opts.insert(0, fmt_rate(r));
        }
    }

    /// Selecting a backend adopts its default devices, then queries their
    /// preferred channel, stream rate and input/output buffers.
    #[cfg(feature = "standalone")]
    fn adopt_backend_defaults(&mut self) {
        // Selecting a backend is a fresh device selection, even in a warm
        // session. Query and adopt that backend's real defaults.
        self.settings.input_channel = None;
        self.settings.sample_rate = None;
        self.settings.output_sample_rate = None;
        self.settings.period_size = None;
        self.settings.output_period_size = None;
        self.settings.output_channels = None;
        // Devices fall to the probed defaults — for ASIO the first
        // driver with at least one input channel (devices.rs filters
        // the nomination by capability, registry order).
        self.settings.input_device.clone_from(&self.default_in);
        self.settings.output_device = if self.backend_key() == "asio" {
            // One ASIO driver per process is not policy but asio-sys
            // law: input and output must be the SAME interface, and
            // the input-capable one owns that choice.
            self.settings.input_device.clone()
        } else {
            self.default_out.clone()
        };
        // The adopted devices are new stream truth: re-read with them
        // in hand, then adopt the device's stream defaults too.
        self.take_snapshot();
        self.apply_stream_snap();
        self.adopt_stream_defaults();
    }

    /// The devices changed (adopted with a backend, or
    /// overridden by hand): buffer and sample rate follow the NOW
    /// selected device's queried defaults, whenever one is known.
    #[cfg(feature = "standalone")]
    fn adopt_stream_defaults(&mut self) {
        if self.settings.sample_rate.is_none() {
            self.settings.sample_rate = self.driver_rate;
        }
        if self.settings.period_size.is_none() {
            self.settings.period_size = self.snap.as_ref().and_then(|s| s.buffer_pref);
        }
        if self.settings.output_period_size.is_none() {
            self.settings.output_period_size =
                self.snap.as_ref().and_then(|s| s.output_buffer_pref);
        }
        self.sync_linked_buffers();
        self.sync_linked_rates();
    }

    fn refresh_devices(&mut self) {
        #[cfg(feature = "standalone")]
        {
            self.take_snapshot();
            if let Some(s) = self.snap.clone() {
                self.input_opts = s.inputs;
                self.output_opts = s.outputs;
                self.default_in = s.def_in;
                self.default_out = s.def_out;
            }
            // The same read carries the stream side — applying it here
            // means ONE enumeration pass per backend switch; a second
            // child spin-up would just pay the same cost again.
            self.apply_stream_snap();
        }
        // JACK choices are graph ports: never add a vanished selection back
        // into the list of available routes. Its saved name remains visible
        // in the field, and Apply explains why it must be replaced.
        if self.settings.backend.as_deref() == Some("jack") {
            return;
        }
        // A saved device the current enumeration does not list is KEPT,
        // not dropped: ASIO enumeration is driver-flaky (cold starts,
        // driver-busy moments), and silently nulling the user's choice
        // here persisted "back to default" files on the next Apply.
        // Instead the saved name joins the option list, so it stays
        // visible and re-selectable even while enumeration is blind.
        if let Some(d) = &self.settings.input_device
            && !self.input_opts.contains(d)
        {
            self.input_opts.insert(0, d.clone());
        }
        if let Some(d) = &self.settings.output_device
            && !self.output_opts.contains(d)
        {
            self.output_opts.insert(0, d.clone());
        }
    }

    fn refresh_stream_options(&mut self) {
        #[cfg(feature = "standalone")]
        {
            self.take_snapshot();
            self.apply_stream_snap();
        }
    }

    #[cfg(any(feature = "standalone", test))]
    fn set_buffers_linked(&mut self, linked: bool) {
        self.buffers_linked = linked;
        self.sync_linked_buffers();
        if linked && self.open_dropdown == Some(Dropdown::OutputBuffer) {
            self.open_dropdown = None;
        }
    }

    fn sync_linked_buffers(&mut self) {
        if self.buffers_linked {
            self.settings.output_period_size = self.settings.period_size;
        }
    }

    /// The rate pair mirrors the buffer pair, including who may call it:
    /// the standalone snapshot applies it, and the tests apply it by hand.
    #[cfg(any(feature = "standalone", test))]
    fn set_rates_linked(&mut self, linked: bool) {
        self.rates_linked = linked;
        self.sync_linked_rates();
        if linked && self.open_dropdown == Some(Dropdown::OutputRate) {
            self.open_dropdown = None;
        }
    }

    fn sync_linked_rates(&mut self) {
        if self.rates_linked {
            self.settings.output_sample_rate = self.settings.sample_rate;
        }
    }

    fn dropdown_disabled(&self, dd: Dropdown) -> bool {
        (self.is_hosted() && Self::is_audio_field(dd))
            || self.is_applying()
            || (dd == Dropdown::InputCh && self.settings.backend.as_deref() == Some("jack"))
            // One ASIO driver per process is asio-sys law, not this app's
            // policy. The output row already mirrors the input driver, so
            // offering to repoint it only offers a pair that cannot open —
            // and a second driver cannot even be loaded to be asked.
            || (dd == Dropdown::Output && self.settings.backend.as_deref() == Some("asio"))
            || (dd == Dropdown::OutputBuffer && self.buffers_linked)
            || (dd == Dropdown::OutputRate && self.rates_linked)
            || (matches!(dd, Dropdown::OutputCh | Dropdown::OutputRate)
                && (self
                    .settings
                    .output_device
                    .as_ref()
                    .or(self.default_out.as_ref())
                    .is_none()
                    || if dd == Dropdown::OutputCh {
                        self.output_channel_opts.is_empty()
                    } else {
                        self.output_rate_opts.is_empty()
                    }))
    }

    fn is_applying(&self) -> bool {
        #[cfg(feature = "standalone")]
        {
            self.pending_apply.is_some()
        }
        #[cfg(not(feature = "standalone"))]
        {
            false
        }
    }

    fn apply_blocker(&self) -> Option<String> {
        if self.is_hosted() {
            return None;
        }
        if self.is_applying() {
            return Some("Applying audio settings…".into());
        }
        if !self.audio_dirty() {
            return None;
        }
        #[cfg(feature = "standalone")]
        {
            let snapshot = match &self.snap {
                Some(snapshot) => snapshot,
                None => return Some("Device information is not available".into()),
            };
            if let Err(error) = snapshot.resolve_settings(&self.settings) {
                return Some(error);
            }
            if !crate::standalone::control::available() {
                return Some("Live audio configuration is unavailable".into());
            }
            None
        }
        #[cfg(not(feature = "standalone"))]
        {
            Some("Audio settings are controlled by the host".into())
        }
    }

    #[cfg(feature = "standalone")]
    fn poll_audio_apply(&mut self) {
        if let Some(error) = crate::standalone::control::take_error() {
            self.applied = Settings::default();
            self.open_settings_page();
            self.save_note = Some(format!("Audio stopped: {error}"));
        }
        let Some(pending) = self.pending_apply.as_ref() else {
            return;
        };
        let result = match pending.result.try_recv() {
            Ok(result) => result,
            Err(crossbeam::channel::TryRecvError::Empty) => return,
            Err(crossbeam::channel::TryRecvError::Disconnected) => {
                Err("The audio worker stopped while applying settings".into())
            }
        };
        let pending = self.pending_apply.take().unwrap();
        match result {
            Ok(()) => {
                self.applied = pending.settings.clone();
                self.applied.input_channel =
                    self.applied.input_channel.or(self.default_input_channel);
                self.save_applied_settings(pending.settings);
            }
            Err(error) => {
                self.shared
                    .input_channel
                    .store(self.applied.input_channel.unwrap_or(1));
                eprintln!("Could not apply audio settings:\n{error}");
                self.save_note = Some(format!("Could not apply audio settings: {error}"));
            }
        }
    }

    /// Window geometry is independent of the draft and may have changed while
    /// an audio Apply was pending. Always capture its latest value before saving.
    fn with_current_window(&self, mut settings: Settings) -> Settings {
        let size = self.shared.logical_size();
        let position = *self.shared.window_pos.lock().unwrap();
        let previous = Settings::load().0.window.or(settings.window);
        let (width, height) = if size.width.is_finite()
            && size.height.is_finite()
            && size.width > 0.0
            && size.height > 0.0
        {
            (size.width, size.height)
        } else {
            // An audio Apply can finish while the window is minimized.
            previous
                .filter(|window| {
                    window.width > 0.0
                        && window.height > 0.0
                        && window.width.is_finite()
                        && window.height.is_finite()
                })
                .map(|window| (window.width, window.height))
                .unwrap_or((crate::DEFAULT_SIZE.width, crate::DEFAULT_SIZE.height))
        };
        settings.window = Some(crate::settings::WindowGeom {
            x: position
                .map(|point| point.0)
                .or(previous.and_then(|window| window.x)),
            y: position
                .map(|point| point.1)
                .or(previous.and_then(|window| window.y)),
            width,
            height,
        });
        settings
    }

    fn save_applied_settings(&mut self, settings: Settings) {
        let settings = self.with_current_window(settings);
        let result = settings.save();
        self.record_apply_save(settings, result);
    }

    fn record_apply_save(&mut self, settings: Settings, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.settings = settings.clone();
                self.saved = settings.clone();
                self.applied = settings;
                self.applied.input_channel =
                    self.applied.input_channel.or(self.default_input_channel);
                self.cold = false;
                self.page = Page::Main;
                self.open_dropdown = None;
                self.dropdown_scroll_drag = false;
                self.devices_loaded = false;
                self.save_note = None;
            }
            Err(error) => {
                self.page = Page::Settings;
                self.save_note = Some(format!("Could not save settings: {error}"));
            }
        }
    }

    fn reset_settings_draft(&mut self) {
        if let Some(host) = &self.host_controls {
            for (parameter, value) in [
                (HostParameter::A4, 440.0),
                (HostParameter::Gate, -60.0),
                (HostParameter::InTune, 3.0),
                (HostParameter::GaugeCurve, 5.0),
            ] {
                host.set(parameter, value);
            }
            self.save_note = None;
            return;
        }
        let window = self.settings.window;
        self.settings = Settings {
            window,
            ..Default::default()
        };
        self.shared
            .a4_ref_bits
            .store(440f64.to_bits(), Ordering::Relaxed);
        self.shared.gate_db.store(-60.0);
        self.refresh_devices();
        self.save_note = Some("Defaults selected — Apply to save".into());
    }

    fn leave_settings(&mut self) {
        // An in-flight Apply must finish before another navigation action can
        // race its save. BACK and Escape are temporarily disabled while waiting.
        if self.is_applying() {
            return;
        }
        if self.is_hosted() {
            self.page = Page::Main;
            self.open_dropdown = None;
            self.dropdown_scroll_drag = false;
            self.save_note = None;
            return;
        }
        self.settings = self.applied.clone();
        self.shared.a4_ref_bits.store(
            (self.settings.a4_ref.unwrap_or(440.0) as f64).to_bits(),
            Ordering::Relaxed,
        );
        self.shared
            .gate_db
            .store(self.settings.gate_db.unwrap_or(-60.0));
        self.shared
            .input_channel
            .store(self.settings.input_channel.unwrap_or(1));
        self.page = Page::Main;
        self.open_dropdown = None;
        self.dropdown_scroll_drag = false;
        self.list_scroll = 0.0;
        self.devices_loaded = false;
        self.save_note = None;
    }

    /// Reconcile a channel selection with the selected device's real
    /// capabilities. Zero was the removed Auto option, not a channel.
    #[cfg(any(feature = "standalone", test))]
    fn apply_input_channels(&mut self, count: Option<u32>, default: Option<u32>) {
        self.input_channels = count.filter(|&n| n > 0);
        self.default_input_channel =
            default.filter(|&ch| ch > 0 && self.input_channels.is_some_and(|count| ch <= count));
        if self
            .settings
            .input_channel
            .is_some_and(|ch| ch == 0 || self.input_channels.is_some_and(|count| ch > count))
        {
            self.settings.input_channel = None;
        }
        if self.settings.input_channel.is_none() {
            self.settings.input_channel = self.default_input_channel;
        }
        // Draft only. The running stream's channel count is whatever was
        // applied, and `standalone::audio` publishes that to the plugin when it
        // opens the stream; a draft must not reach the audio state.
    }

    /// PipeWire can apply its quantum only after a JACK client activates.
    /// CPAL's earlier device query is then stale; the audio callback publishes
    /// the actual block size. Use it only for the currently applied backend.
    fn live_jack_buffer(&self) -> Option<u32> {
        if cfg!(all(target_os = "linux", feature = "standalone"))
            && self.settings.backend.as_deref() == Some("jack")
            && self.applied.backend.as_deref() == Some("jack")
            && !self.is_applying()
        {
            let frames = self.shared.buffer_frames.load();
            return (frames > 0).then_some(frames);
        }
        None
    }

    fn displayed_jack_buffer(&self) -> Option<u32> {
        // A pending selection must stay visible until Apply succeeds.
        (self.settings.period_size == self.applied.period_size
            && self.settings.output_period_size == self.applied.output_period_size)
            .then(|| self.live_jack_buffer())
            .flatten()
    }

    /// A live JACK block size or queried backend/device default.
    fn buffer_default(&self, dd: Dropdown) -> Option<u32> {
        if let Some(frames) = self.live_jack_buffer() {
            return Some(frames);
        }
        #[cfg(feature = "standalone")]
        {
            self.snap.as_ref().and_then(|snapshot| {
                if dd == Dropdown::InputBuffer || self.buffers_linked {
                    snapshot.buffer_pref
                } else {
                    snapshot.output_buffer_pref
                }
            })
        }
        #[cfg(not(feature = "standalone"))]
        {
            let _ = dd;
            None
        }
    }

    /// Best-known driver rate. A fresh query wins; otherwise the value
    /// remembered at boot — the file's rate EQUALS what this process
    /// started with, so it was driver-resolved, not a draft the user
    /// is choosing right now. Queried once, remembered: a blind
    /// moment never erases the truth into a dash.
    fn driver_rate_known(&self) -> Option<f32> {
        self.driver_rate.or_else(|| {
            self.settings
                .sample_rate
                .filter(|r| self.applied.sample_rate == Some(*r))
        })
    }

    /// The concrete value the default hides — shown wherever the word
    /// "(default)" used to stand. None = genuinely unknowable (no system
    /// default device, silent driver) → rendered as an em dash.
    fn default_label(&self, dd: Dropdown) -> Option<String> {
        Some(match dd {
            // The backend has NO default: a backend is only ever what
            // the user chose. Row 0 renders as "\u{2014}" — the honest
            // "nothing chosen" head — and RESET falls back to it.
            Dropdown::Backend => return None,
            Dropdown::Input => self.default_in.clone()?,
            Dropdown::Output => self.default_out.clone()?,
            Dropdown::OutputCh => self.default_output_channels?.label(),
            Dropdown::Rate => fmt_rate(self.driver_rate_known()?),
            Dropdown::OutputRate => fmt_rate(self.default_output_rate?),
            Dropdown::InputBuffer | Dropdown::OutputBuffer => self.buffer_default(dd)?.to_string(),
            Dropdown::InputCh => self.default_input_channel?.to_string(),
            Dropdown::A4 => "440".to_owned(),
            Dropdown::Gate => "-60".to_owned(),
            Dropdown::InTune => format!("{IN_TUNE_CENTS:.0}"),
            Dropdown::GaugeCurve => format!("{GAUGE_KNEE_CENTS:.0}"),
        })
    }

    /// The ONE list a dropdown shows, is selected from, and is picked
    /// from: row 0 is the default — its concrete value, colored green — and
    /// the default never reappears among the plain options. Picking row
    /// 0 means "follow the default", which keeps tracking hardware
    /// (system device, driver rate) where a pinned option would freeze
    /// a snapshot. Pairs are (display label, raw value); second item of
    /// the tuple is the default row's raw value ("follow" sentinel).
    fn dropdown_rows(&self, dd: Dropdown) -> (Vec<(String, String)>, String) {
        if self.settings.backend.as_deref() == Some("jack") && dd == Dropdown::OutputCh {
            let left = self
                .settings
                .output_device
                .as_ref()
                .or(self.default_out.as_ref());
            let mut rows = vec![("Mono (no right output)".into(), String::new())];
            rows.extend(
                self.output_opts
                    .iter()
                    .filter(|name| Some(*name) != left)
                    .map(|name| (name.clone(), name.clone())),
            );
            return (rows, String::new());
        }
        // Plain options: (display label, raw value it selects).
        let opts: Vec<(String, String)> = match dd {
            Dropdown::Backend => self
                .backend_opts
                .iter()
                .map(|(id, l)| (l.clone(), id.clone()))
                .collect(),
            Dropdown::Input | Dropdown::Output => {
                let list = match dd {
                    Dropdown::Input => &self.input_opts,
                    _ => &self.output_opts,
                };
                list.iter().map(|n| (n.clone(), n.clone())).collect()
            }
            Dropdown::Rate | Dropdown::OutputRate => (if dd == Dropdown::Rate {
                &self.rate_opts
            } else {
                &self.output_rate_opts
            })
            .iter()
            .map(|l| (l.clone(), l.split(' ').next().unwrap_or("").to_owned()))
            .collect(),
            Dropdown::InputBuffer | Dropdown::OutputBuffer => {
                let options = if dd == Dropdown::InputBuffer {
                    &self.buffer_opts
                } else {
                    &self.output_buffer_opts
                };
                options
                    .iter()
                    .map(|value| (value.clone(), value.clone()))
                    .collect()
            }
            Dropdown::OutputCh => self
                .output_channel_opts
                .iter()
                .map(|route| (route.label(), route.raw()))
                .collect(),
            Dropdown::InputCh => (1..=self.input_channels.unwrap_or(0))
                .map(|ch| (ch.to_string(), ch.to_string()))
                .collect(),
            Dropdown::A4 => A4_OPTIONS
                .iter()
                .map(|v| (format!("{v:.0} Hz"), format!("{v:.0}")))
                .collect(),
            Dropdown::Gate => GATE_OPTIONS
                .iter()
                .map(|v| (format!("{v:.0} dB"), format!("{v:.0}")))
                .collect(),
            Dropdown::InTune => IN_TUNE_OPTIONS
                .iter()
                .map(|v| (format!("{v:.0} \u{a2}"), format!("{v:.0}")))
                .collect(),
            Dropdown::GaugeCurve => GAUGE_KNEE_OPTIONS
                .iter()
                .map(|v| (format!("{v:.0} \u{a2} knee"), format!("{v:.0}")))
                .collect(),
        };
        // Raw value of the default row ("" when unknowable: the row
        // still exists — "\u{2014}" — and still means follow).
        let def_raw = self.default_raw(dd);
        // The four tuner params are tables whose entries are the values themselves,
        // the default included. A default row would lift `5` out of its place between
        // 4 and 7 and put it on top, where it reads as a value the list was arranged
        // around; they keep the table's own order, and the default is marked in place.
        // Everything else — devices, rates, channels — has a default that is not one
        // of its own options ("Default", "System default") and needs its own row.
        if matches!(
            dd,
            Dropdown::A4 | Dropdown::Gate | Dropdown::InTune | Dropdown::GaugeCurve
        ) {
            return (opts, def_raw);
        }
        let head = self.default_label(dd).unwrap_or_else(|| "\u{2014}".into());
        let mut rows = vec![(head, def_raw.clone())];
        rows.extend(
            opts.into_iter()
                .filter(|(_, raw)| raw.is_empty() || raw != &def_raw),
        );
        (rows, def_raw)
    }

    /// The raw value the default row selects ("" = unknowable default).
    fn default_raw(&self, dd: Dropdown) -> String {
        if self.settings.backend.as_deref() == Some("jack") && dd == Dropdown::OutputCh {
            return String::new();
        }
        match dd {
            // "" = no default to follow (see default_label).
            Dropdown::Backend => String::new(),
            Dropdown::Input => self.default_in.clone().unwrap_or_default(),
            Dropdown::Output => self.default_out.clone().unwrap_or_default(),
            Dropdown::OutputCh => self
                .default_output_channels
                .map(|route| route.raw())
                .unwrap_or_default(),
            Dropdown::OutputRate => self
                .default_output_rate
                .map(|r| r.to_string())
                .unwrap_or_default(),
            Dropdown::Rate => self
                .driver_rate_known()
                .map(|r| r.to_string())
                .unwrap_or_default(),
            Dropdown::InputBuffer | Dropdown::OutputBuffer => self
                .buffer_default(dd)
                .map(|value| value.to_string())
                .unwrap_or_default(),
            Dropdown::InputCh => self
                .default_input_channel
                .map(|ch| ch.to_string())
                .unwrap_or_default(),
            Dropdown::A4 => "440".to_owned(),
            Dropdown::Gate => "-60".to_owned(),
            Dropdown::InTune => format!("{IN_TUNE_CENTS:.0}"),
            Dropdown::GaugeCurve => format!("{GAUGE_KNEE_CENTS:.0}"),
        }
    }

    /// The raw value the row shows right now (unset settings resolve to
    /// their default — that IS the current truth).
    fn current_raw(&self, dd: Dropdown) -> String {
        if self.settings.backend.as_deref() == Some("jack") && dd == Dropdown::OutputCh {
            return self.settings.jack_output_right.clone().unwrap_or_default();
        }
        match dd {
            Dropdown::Backend => self.settings.backend.clone().unwrap_or_default(),
            Dropdown::Input => self
                .settings
                .input_device
                .clone()
                .or_else(|| self.default_in.clone())
                .unwrap_or_default(),
            Dropdown::Output => self
                .settings
                .output_device
                .clone()
                .or_else(|| self.default_out.clone())
                .unwrap_or_default(),
            Dropdown::OutputRate => self
                .settings
                .output_sample_rate
                .or(self.default_output_rate)
                .map(|r| r.to_string())
                .unwrap_or_default(),
            Dropdown::Rate => self
                .settings
                .sample_rate
                .map(|r| r.to_string())
                .or_else(|| self.driver_rate_known().map(|r| r.to_string()))
                .unwrap_or_default(),
            Dropdown::InputBuffer | Dropdown::OutputBuffer => {
                let period = if dd == Dropdown::InputBuffer {
                    self.settings.period_size
                } else {
                    self.settings.output_period_size
                };
                self.displayed_jack_buffer()
                    .or(period)
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| {
                        self.buffer_default(dd)
                            .map(|value| value.to_string())
                            .unwrap_or_default()
                    })
            }
            Dropdown::OutputCh => self
                .settings
                .output_channels
                .or(self.default_output_channels)
                .map(|route| route.raw())
                .unwrap_or_default(),
            Dropdown::InputCh => self
                .settings
                .input_channel
                .or(self.default_input_channel)
                .map(|ch| ch.to_string())
                .unwrap_or_default(),
            Dropdown::A4 => (self.shared.a4_ref() as f32).to_string(),
            Dropdown::Gate => format!("{:.0}", self.shared.gate_db.load()),
            Dropdown::InTune => format!(
                "{:.0}",
                self.settings.in_tune_cents.unwrap_or(IN_TUNE_CENTS as f32)
            ),
            Dropdown::GaugeCurve => format!(
                "{:.0}",
                self.settings.gauge_knee_cents.unwrap_or(GAUGE_KNEE_CENTS)
            ),
        }
    }

    /// The row carrying the current truth: row 0 while the setting is
    /// unset (or set to exactly the default), else the matching option,
    /// else row 0 again (unknown value = shown as following default).
    fn selected_index(&self, dd: Dropdown) -> usize {
        let current = self.current_raw(dd);
        self.dropdown_rows(dd)
            .0
            .iter()
            .position(|(_, raw)| *raw == current)
            .unwrap_or(0)
    }

    /// True when the current value follows or equals the default.
    fn is_default(&self, dd: Dropdown) -> bool {
        let current = self.current_raw(dd);
        current.is_empty() || current == self.default_raw(dd)
    }

    /// The open dropdown, drawn last so it floats above whichever page asked for
    /// it. `Some(anchor)` hangs the list off that control, as a settings row does;
    /// `None` puts it in the middle of the window, which is what a control painted
    /// on the main view needs - there is no row under it to hang from. Returns true
    /// when the popup consumed the click, so the caller stops routing that click.
    fn draw_dropdown(
        &mut self,
        ctx: &mut Context,
        window: [f32; 2],
        fit: f32,
        place: Placement,
        z: f32,
        clicked: bool,
    ) -> bool {
        let mouse = ctx.mouse_position();
        // Cloned, not borrowed: the body calls `self.pick` while laying out rows, and
        // an immutable borrow of the font would still be held there.
        let font = self.font.clone();

        // Open dropdown: drawn last (on top), items hit first.
        if let Some(dd) = self.open_dropdown {
            let items = self.dropdown_rows(dd).0;
            // A named popup gets a row for its name, so the box is measured one row
            // taller and every item shifts down by it.
            let header = matches!(place, Placement::Centred(_));
            let extra = header as usize;
            let layout = match place {
                Placement::Row(at, half) => {
                    DropdownLayout::new(window, at, half, fit, items.len() + extra)
                }
                Placement::Centred(half) => {
                    DropdownLayout::centred(window, half, fit, items.len() + extra)
                }
            };
            let visible_items = layout.visible.saturating_sub(extra).max(1);
            let max_scroll = items.len().saturating_sub(visible_items) as f32;
            self.list_scroll = (self.list_scroll + ctx.mouse_wheel() * 0.5).clamp(0.0, max_scroll);
            // The list keeps its own order: opening does not lift the current value to
            // the top, which read as a re-sort. The scroll is only nudged when the
            // selection would otherwise be off the visible rows, and then only as far
            // as bringing it in - never to the front.
            let sel_idx = self.selected_index(dd);
            let shown_from = self.list_scroll.floor() as usize;
            if sel_idx < shown_from {
                self.list_scroll = (sel_idx as f32).min(max_scroll);
            } else if sel_idx >= shown_from + visible_items {
                self.list_scroll = ((sel_idx + 1 - visible_items) as f32).clamp(0.0, max_scroll);
            }
            if !ctx.mouse_button_down(frost::MouseButton::Left) {
                self.dropdown_scroll_drag = false;
            }
            let scrollable = max_scroll > 0.0;
            let thumb_half = if scrollable {
                (layout.track_half[1] * visible_items as f32 / items.len() as f32)
                    .max(6.0 * fit)
                    .min(layout.track_half[1])
            } else {
                0.0
            };
            let travel = (2.0 * (layout.track_half[1] - thumb_half)).max(0.0);
            let over_track = scrollable
                && point_in(
                    mouse,
                    layout.track_x,
                    layout.center[1],
                    layout.track_half[0],
                    layout.track_half[1],
                );
            if clicked && over_track {
                self.dropdown_scroll_drag = true;
            }
            if self.dropdown_scroll_drag
                && scrollable
                && travel > 0.0
                && let Some([_, mouse_y]) = mouse
            {
                let fraction = ((layout.center[1] + layout.track_half[1] - thumb_half - mouse_y)
                    / travel)
                    .clamp(0.0, 1.0);
                self.list_scroll = (fraction * max_scroll).round();
            }
            let skip = self.list_scroll.floor() as usize;
            let [px, py] = layout.center;
            let [pw, ph] = layout.half;
            ctx.rectangle(px, py, pw, ph, HOVER, 0.44);
            ctx.rectangle(
                px,
                py,
                (pw - layout.border).max(0.0),
                (ph - layout.border).max(0.0),
                MENU_BG,
                0.445,
            );
            if scrollable {
                ctx.rectangle(
                    layout.track_x,
                    py,
                    layout.track_half[0],
                    layout.track_half[1],
                    TRACK,
                    0.45,
                );
                let thumb_y =
                    py + layout.track_half[1] - thumb_half - skip as f32 / max_scroll * travel;
                ctx.rectangle(
                    layout.track_x,
                    thumb_y,
                    layout.track_half[0],
                    thumb_half,
                    if over_track || self.dropdown_scroll_drag {
                        AMBER
                    } else {
                        DIM
                    },
                    0.46,
                );
            }
            let menu_size = (18.7 * fit).min(layout.item_half[1] * (18.7 / 13.0));
            let char_width = ctx.text_width(&font, "0", menu_size, 700.0).max(fit);
            let max_chars = ((2.0 * layout.item_half[0] - 4.0 * fit) / char_width)
                .floor()
                .max(1.0) as usize;
            let selected = self.selected_index(dd);
            let def_raw = self.default_raw(dd);
            if header {
                let hy = layout.row_y(0);
                ctx.text(
                    layout.item_x,
                    hy,
                    &font,
                    setting_name(dd),
                    menu_size,
                    700.0,
                    DIM,
                    z + 0.06,
                );
                // Half a step below the name is exactly midway to the first row.
                ctx.rectangle(
                    layout.center[0],
                    hy - (hy - layout.row_y(1)) / 2.0,
                    layout.item_half[0],
                    (1.0 * fit).max(0.5),
                    Color { a: 0.5, ..TRACK },
                    z + 0.06,
                );
            }
            for (slot, (idx, (label, raw))) in items
                .iter()
                .enumerate()
                .skip(skip)
                .take(visible_items)
                .enumerate()
            {
                let iy = layout.row_y(slot + extra);
                let ix = layout.item_x;
                let [iw, ih] = layout.item_half;
                let sel = selected == idx;
                let hov = point_in(mouse, ix, iy, iw, ih);
                ctx.rectangle(
                    ix,
                    iy,
                    iw,
                    ih,
                    if hov {
                        HOVER
                    } else if sel {
                        TRACK
                    } else {
                        Color {
                            a: 0.0,
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                        }
                    },
                    z + 0.05,
                );
                ctx.text(
                    ix,
                    iy,
                    &font,
                    if self.settings.backend.as_deref() == Some("jack")
                        && matches!(dd, Dropdown::Input | Dropdown::Output | Dropdown::OutputCh)
                    {
                        clip_port(label, max_chars)
                    } else {
                        clip(label, max_chars)
                    },
                    menu_size,
                    if sel { 700.0 } else { 550.0 },
                    if !def_raw.is_empty() && *raw == def_raw {
                        GREEN
                    } else if sel || hov {
                        AMBER
                    } else {
                        FG
                    },
                    z + 0.06,
                );
                if clicked && hov {
                    self.pick(dd, idx);
                    return true;
                }
            }
            if clicked && point_in(mouse, px, py, pw, ph) {
                return true; // The popup surface consumes clicks between its rows too.
            }
        }
        false
    }
    fn pick(&mut self, dd: Dropdown, idx: usize) {
        if self.dropdown_disabled(dd) {
            return;
        }
        self.save_note = None;
        // The raw value lives on the same row the user clicked — no
        // index arithmetic anywhere. Row 0 carries the default's raw
        // value; the setting is stored as None (follow) exactly then.
        let (rows, def_raw) = self.dropdown_rows(dd);
        let Some((_, raw)) = rows.get(idx).cloned() else {
            return;
        };
        let at_default = raw.is_empty() || raw == def_raw;
        match dd {
            Dropdown::Backend => {
                // Row 0 (the platform host) means follow → None. A plain
                // "auto" row is a CHOICE — kept as Some("auto"), the one
                // road that may go looking for jack.
                self.settings.backend = if at_default { None } else { Some(raw) };
                self.settings.input_channel = None;
                self.settings.jack_output_right = None;
                self.refresh_devices();
                #[cfg(feature = "standalone")]
                self.adopt_backend_defaults();
            }
            Dropdown::Input => {
                self.settings.input_device = if at_default { None } else { Some(raw) };
                self.settings.input_channel = None;
                self.settings.period_size = None;
                self.settings.sample_rate = None;
                // ASIO shares one driver. CoreAudio and other backends
                // retain the independently selected output device.
                if self.settings.backend.as_deref() == Some("asio") {
                    self.settings.output_device = self.settings.input_device.clone();
                }
                self.refresh_stream_options();
                #[cfg(feature = "standalone")]
                self.adopt_stream_defaults();
            }
            Dropdown::Output => {
                self.settings.output_device = if at_default { None } else { Some(raw) };
                if self.settings.jack_output_right.as_ref()
                    == self
                        .settings
                        .output_device
                        .as_ref()
                        .or(self.default_out.as_ref())
                {
                    self.settings.jack_output_right = None;
                }
                self.settings.output_channels = None;
                self.settings.output_sample_rate = None;
                self.settings.output_period_size = None;
                // Duplex constraint: the output device narrows the buffer
                // range too — recompute.
                self.refresh_stream_options();
                #[cfg(feature = "standalone")]
                self.adopt_stream_defaults();
            }
            Dropdown::OutputCh if self.settings.backend.as_deref() == Some("jack") => {
                self.settings.jack_output_right = if raw.is_empty() { None } else { Some(raw) };
            }
            Dropdown::OutputCh => {
                self.settings.output_channels = if at_default {
                    None
                } else {
                    crate::settings::OutputChannels::parse(&raw)
                };
            }
            Dropdown::OutputRate => {
                self.settings.output_sample_rate = if at_default { None } else { raw.parse().ok() };
            }
            Dropdown::Rate => {
                self.settings.sample_rate = if at_default { None } else { raw.parse().ok() };
                self.sync_linked_rates();
            }
            Dropdown::InputBuffer => {
                self.settings.period_size = if at_default { None } else { raw.parse().ok() };
                self.sync_linked_buffers();
            }
            Dropdown::OutputBuffer => {
                self.settings.output_period_size = if at_default { None } else { raw.parse().ok() };
            }
            // The input channel belongs to the stream: it is opened with as
            // many planes as the applied setting names. A draft must therefore
            // not reach the audio state — picking a channel the running stream
            // does not have selects a plane that is not there, which is silence
            // for the meter and the monitor until APPLY rebuilds the stream.
            Dropdown::InputCh => {
                self.settings.input_channel = if at_default { None } else { raw.parse().ok() };
            }
            // Params are live the moment they are picked (no restart needed);
            // APPLY only decides whether they persist to the next launch.
            Dropdown::A4 => {
                let v: f32 = raw.parse().unwrap_or(440.0);
                self.shared
                    .a4_ref_bits
                    .store((v as f64).to_bits(), Ordering::Relaxed);
                self.settings.a4_ref = if at_default { None } else { Some(v) };
            }
            Dropdown::Gate => {
                let v: f32 = raw.parse().unwrap_or(-60.0);
                self.shared.gate_db.store(v);
                self.settings.gate_db = if at_default { None } else { Some(v) };
            }
            // Editor look/judgement: applied the instant they are picked
            // (draw + verdict re-read settings every frame); APPLY persists.
            Dropdown::InTune => {
                let v: f32 = raw.parse().unwrap_or(IN_TUNE_CENTS as f32);
                self.settings.in_tune_cents = if at_default { None } else { Some(v) };
            }
            Dropdown::GaugeCurve => {
                let v: f32 = raw.parse().unwrap_or(GAUGE_KNEE_CENTS);
                self.settings.gauge_knee_cents = if at_default { None } else { Some(v) };
            }
        }
        if let Some(host) = &self.host_controls {
            let change = match dd {
                Dropdown::A4 => Some((HostParameter::A4, self.settings.a4_ref.unwrap_or(440.0))),
                Dropdown::Gate => {
                    Some((HostParameter::Gate, self.settings.gate_db.unwrap_or(-60.0)))
                }
                Dropdown::InTune => Some((
                    HostParameter::InTune,
                    self.settings.in_tune_cents.unwrap_or(IN_TUNE_CENTS as f32),
                )),
                Dropdown::GaugeCurve => Some((
                    HostParameter::GaugeCurve,
                    self.settings.gauge_knee_cents.unwrap_or(GAUGE_KNEE_CENTS),
                )),
                _ => None,
            };
            if let Some((parameter, value)) = change {
                host.set(parameter, value);
            }
        }
        self.open_dropdown = None;
        self.list_scroll = 0.0;
    }

    /// Draw `text` glyph-by-glyph on a fixed advance pitch (FiraCode is
    /// monospaced, so one digit's width is THE pitch). Each glyph is
    /// ink-centered inside its own slot. The selected anchor stays fixed when
    /// signs, hidden zeros or the number of digits change. Whole-string `ctx.text`
    /// cannot promise that: its centering follows the ink box, and glyph ink
    /// varies (+ vs −, and future fonts).
    #[allow(clippy::too_many_arguments)] // immediate-mode draw call, as in frost
    fn draw_tabular(
        &self,
        ctx: &mut Context,
        cx: f32,
        cy: f32,
        text: &str,
        anchor: char,
        size: f32,
        weight: f32,
        color: Color,
        z: f32,
    ) {
        let font = std::sync::Arc::clone(&self.font);
        let pitch = ctx.text_width(&font, "0", size, weight);
        if pitch <= 0.0 {
            // Font cannot be shaped: fall back to a plain draw rather
            // than silently vanishing.
            ctx.text(cx, cy, &font, text, size, weight, color, z);
            return;
        }
        for (ch, offset) in tabular_glyphs(text, pitch, anchor) {
            let mut buf = [0u8; 4];
            ctx.text(
                cx + offset,
                cy,
                &font,
                ch.encode_utf8(&mut buf),
                size,
                weight,
                color,
                z,
            );
        }
    }

    /// Defaults stay green while hovered or open. An unset value is a
    /// placeholder, not a concrete default, and keeps the regular colors.
    fn value_color(&self, dd: Dropdown, highlighted: bool) -> Color {
        if self.is_default(dd) && self.value_text(dd) != "\u{2014}" {
            GREEN
        } else if highlighted {
            AMBER
        } else {
            FG
        }
    }

    fn value_text(&self, dd: Dropdown) -> String {
        if let Some(value) = self.host_audio_value(dd) {
            return value;
        }
        if self.settings.backend.as_deref() == Some("jack") {
            match dd {
                Dropdown::OutputCh => {
                    return self
                        .settings
                        .jack_output_right
                        .as_deref()
                        .map(|name| clip_port(name, 26))
                        .unwrap_or_else(|| "Mono".into());
                }
                Dropdown::Input => {
                    return self
                        .settings
                        .input_device
                        .as_ref()
                        .or(self.default_in.as_ref())
                        .map(|name| clip_port(name, 26))
                        .unwrap_or_else(|| "—".into());
                }
                Dropdown::Output => {
                    return self
                        .settings
                        .output_device
                        .as_ref()
                        .or(self.default_out.as_ref())
                        .map(|name| clip_port(name, 26))
                        .unwrap_or_else(|| "—".into());
                }
                _ => {}
            }
        }
        if matches!(dd, Dropdown::InputBuffer | Dropdown::OutputBuffer)
            && let Some(frames) = self.displayed_jack_buffer()
        {
            return frames.to_string();
        }
        match dd {
            Dropdown::Backend => match self.settings.backend.as_deref() {
                // Nothing chosen yet: a bare dash. The tuner boots SILENT
                // (dummy) until a backend is chosen — no default is ever
                // contacted.
                None => "\u{2014}".to_string(),
                Some(id) => self
                    .backend_opts
                    .iter()
                    .find(|(i, _)| i == id)
                    .map(|(_, l)| l.clone())
                    .unwrap_or_else(|| id.to_owned()),
            },
            Dropdown::OutputRate => self
                .settings
                .output_sample_rate
                .or(self.default_output_rate)
                .map(fmt_rate)
                .unwrap_or_else(|| "—".into()),
            Dropdown::Rate => match self.settings.sample_rate {
                Some(r) => fmt_rate(r),
                // Cold start: nothing chosen, nothing narrated. Warm
                // and unset: "follow the driver" IS the state — show
                // the number (queried or remembered), colored green.
                None if self.cold => "\u{2014}".into(),
                None => self
                    .driver_rate_known()
                    .map(fmt_rate)
                    .unwrap_or_else(|| "\u{2014}".into()),
            },
            Dropdown::InputBuffer | Dropdown::OutputBuffer => {
                match if dd == Dropdown::InputBuffer {
                    self.settings.period_size
                } else {
                    self.settings.output_period_size
                } {
                    // Same rule as Backend: unset is unset — the dropdown's
                    // green default row names the value that follows (the live
                    // driver-granted block size); the button stays a dash.
                    None => self
                        .buffer_default(dd)
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "\u{2014}".into()),
                    Some(p) => p.to_string(),
                }
            }
            // Devices follow the same cold rule: on a fresh install the
            // panel does not announce the system default before the
            // user has shown any interest in devices at all. Warm and
            // unset, "follow the system default" is the state — name it.
            Dropdown::Input => self
                .settings
                .input_device
                .clone()
                .or_else(|| {
                    if self.cold {
                        None
                    } else {
                        self.default_in.clone()
                    }
                })
                .map(|d| clip(&d, 26))
                .unwrap_or_else(|| "\u{2014}".into()),
            Dropdown::Output => self
                .settings
                .output_device
                .clone()
                .or_else(|| {
                    if self.cold {
                        None
                    } else {
                        self.default_out.clone()
                    }
                })
                .map(|d| clip(&d, 26))
                .unwrap_or_else(|| "\u{2014}".into()),
            Dropdown::OutputCh => self
                .settings
                .output_channels
                .or(self.default_output_channels)
                .map(|route| route.label())
                .unwrap_or_else(|| "—".into()),
            Dropdown::InputCh => self
                .settings
                .input_channel
                .or(self.default_input_channel)
                .map(|ch| format!("channel {ch}"))
                .unwrap_or_else(|| "\u{2014}".into()),
            Dropdown::A4 => format!("{:.1} Hz", self.shared.a4_ref()),
            Dropdown::Gate => format!("{:.0} dB", self.shared.gate_db.load()),
            Dropdown::InTune => format!(
                "{:.0} \u{a2} window",
                self.settings.in_tune_cents.unwrap_or(IN_TUNE_CENTS as f32)
            ),
            Dropdown::GaugeCurve => format!(
                "{:.0} \u{a2} knee",
                self.settings.gauge_knee_cents.unwrap_or(GAUGE_KNEE_CENTS)
            ),
        }
    }

    /// Whether Apply needs to replace audio streams. Other parameters are
    /// already live and need only persistence.
    fn audio_dirty(&self) -> bool {
        let (a, s) = (&self.applied, &self.settings);
        a.backend != s.backend
            || a.input_device != s.input_device
            || a.output_device != s.output_device
            || a.jack_output_right != s.jack_output_right
            || a.output_channels != s.output_channels
            || a.sample_rate != s.sample_rate
            || a.output_sample_rate != s.output_sample_rate
            // The stream is opened with as many input planes as this names, so
            // a wider selection is a different stream — not a live parameter.
            || a.input_channel != s.input_channel
            || a.period_size != s.period_size
            || a.output_period_size != s.output_period_size
            || a.input_latency != s.input_latency
    }

    /// Anything to write at all: draft differs from the last save.
    /// Window GEOMETRY is excluded — it persists directly on resize
    /// independently of draft preferences, so counting it
    /// would keep APPLY perpetually "dirty" for merely using the app.
    fn apply_dirty(&self) -> bool {
        let (a, s) = (&self.saved, &self.settings);
        a.backend != s.backend
            || a.input_device != s.input_device
            || a.output_device != s.output_device
            || a.jack_output_right != s.jack_output_right
            || a.output_channels != s.output_channels
            || a.sample_rate != s.sample_rate
            || a.output_sample_rate != s.output_sample_rate
            || a.period_size != s.period_size
            || a.output_period_size != s.output_period_size
            || a.input_latency != s.input_latency
            || a.a4_ref != s.a4_ref
            || a.gate_db != s.gate_db
            || a.input_channel != s.input_channel
            || a.in_tune_cents != s.in_tune_cents
            || a.gauge_knee_cents != s.gauge_knee_cents
    }

    #[cfg(feature = "standalone")]
    fn process_shared_audio_panel(&mut self, ctx: &mut Context, clicked: bool) {
        use nice_common::ui::{AudioPanel, Event, ExtraRow};
        let mut panel = self.audio_panel.take().unwrap_or_else(|| {
            let mut panel = AudioPanel::new(
                self.settings.clone(),
                self.applied.clone(),
                self.saved.clone(),
                self.cold,
            );
            panel.refresh();
            panel.note = self.save_note.clone();
            panel
        });
        panel.live_frames = Some(self.shared.buffer_frames.load());
        let extras: Vec<ExtraRow> = [
            Dropdown::A4,
            Dropdown::Gate,
            Dropdown::InTune,
            Dropdown::GaugeCurve,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, dd)| {
            let label = [
                "A4 reference",
                "Input gate",
                "In-tune window",
                "Gauge curve",
            ][i]
                .to_string();
            let default = self.default_raw(dd);
            let current = match dd {
                Dropdown::A4 => panel.draft.a4_ref,
                Dropdown::Gate => panel.draft.gate_db,
                Dropdown::InTune => panel.draft.in_tune_cents,
                _ => panel.draft.gauge_knee_cents,
            }
            .map(|v| format!("{v:.0}"));
            ExtraRow {
                label,
                default,
                current,
                options: self.dropdown_rows(dd).0,
            }
        })
        .collect();
        let size = self.shared.logical_size();
        let window = nice_common::ui::window_geometry(
            [size.width, size.height],
            *self.shared.window_pos.lock().unwrap(),
            panel.draft.window,
        );
        let event = panel.process(
            ctx,
            &self.font,
            clicked,
            extras,
            window,
            crate::standalone::control::available(),
            crate::standalone::control::request,
        );
        match event {
            Some(Event::ExtraPicked(index, raw)) => {
                let value = raw.and_then(|v| v.parse::<f32>().ok());
                match index {
                    0 => panel.draft.a4_ref = value,
                    1 => panel.draft.gate_db = value,
                    2 => panel.draft.in_tune_cents = value,
                    _ => panel.draft.gauge_knee_cents = value,
                }
            }
            Some(Event::Back) => {
                self.page = Page::Main;
                self.devices_loaded = false;
            }
            Some(Event::Applied) => {
                self.applied = panel.applied.clone();
                self.saved = panel.saved.clone();
                self.cold = false;
                self.page = Page::Main;
                self.devices_loaded = false;
                self.shared.input_channel.store(
                    panel
                        .snapshot
                        .resolve_settings(&panel.applied)
                        .ok()
                        .and_then(|s| s.input_channel)
                        .unwrap_or(1),
                );
            }
            Some(Event::Reset) | None => {}
        }
        self.applied = panel.applied.clone();
        self.saved = panel.saved.clone();
        self.cold = panel.cold;
        self.settings = panel.draft.clone();
        self.shared.a4_ref_bits.store(
            (self.settings.a4_ref.unwrap_or(440.0) as f64).to_bits(),
            Ordering::Relaxed,
        );
        self.shared
            .gate_db
            .store(self.settings.gate_db.unwrap_or(-60.0));
        self.audio_panel = Some(panel);
    }

    fn process_settings_page(&mut self, ctx: &mut Context, clicked: bool) {
        #[cfg(feature = "standalone")]
        if !self.is_hosted() {
            self.process_shared_audio_panel(ctx, clicked);
            return;
        }

        let (w, h) = ctx.size();
        let fit = (w / 720.0).min(h / 440.0).max(0.45);
        let mouse = ctx.mouse_position();
        let font = std::sync::Arc::clone(&self.font);
        let z = 0.4f32;

        // Escape changes nothing — it UN-changes: close an open dropdown,
        // or leave the page with drafts unsaved. Never a write, never a
        // restart.
        let esc = ctx.key_down(frost::KeyCode::Escape);
        if esc && !self.esc_held {
            if self.open_dropdown.is_some() {
                self.open_dropdown = None;
                self.list_scroll = 0.0;
            } else {
                self.leave_settings();
                return;
            }
        }
        self.esc_held = esc;

        // Audio block first, in signal order (what feeds me, which channel
        // of it, what I answer through, then the stream's shape), then a
        // gap, then the live param block.
        const ROWS: [(&str, Dropdown); 13] = [
            ("Backend", Dropdown::Backend),
            ("Input device", Dropdown::Input),
            ("Input channel", Dropdown::InputCh),
            ("Output device", Dropdown::Output),
            ("Output channels", Dropdown::OutputCh),
            ("Input buffer size", Dropdown::InputBuffer),
            ("Output buffer size", Dropdown::OutputBuffer),
            ("Input sample rate", Dropdown::Rate),
            ("Output sample rate", Dropdown::OutputRate),
            ("A4 reference", Dropdown::A4),
            ("Input gate", Dropdown::Gate),
            ("In-tune window", Dropdown::InTune),
            ("Gauge curve", Dropdown::GaugeCurve),
        ];
        // Thirteen rows, with a gap before the tuning parameters, fit
        // above the bottom action buttons at the original window size.
        let row_y = |i: usize| {
            h / 2.0 - 32.0 * fit - i as f32 * 24.0 * fit - if i >= 9 { 10.0 } else { 0.0 } * fit
        };
        // Left column: genuinely LEFT-aligned. ctx.text centers INK, so
        // each label is centered at margin + half its measured width —
        // long names can no longer spill past the left screen edge.
        let lx = -w / 2.0 + 18.0 * fit;
        // Right column: value buttons, three characters wider, anchored by
        // their RIGHT edge at the screen margin — they cannot spill right
        // no matter how wide the dropdown text grows.
        let charw = ctx
            .text_width(&font, "0", 23.4 * fit, 550.0)
            .max(14.0 * fit);
        let bhw = 165.0 * fit + 3.0 * charw;
        let bx = w / 2.0 - 18.0 * fit - bhw;
        let bhh = 11.0 * fit;

        for (i, (label, dd)) in ROWS.iter().enumerate() {
            let label = if self.is_hosted() {
                match dd {
                    Dropdown::Input => "Host inputs",
                    Dropdown::Output => "Host outputs",
                    _ => *label,
                }
            } else if self.settings.backend.as_deref() == Some("jack") {
                match dd {
                    Dropdown::Input => "Input source",
                    Dropdown::Output => "Output left / mono",
                    Dropdown::OutputCh => "Output right",
                    _ => *label,
                }
            } else {
                *label
            };
            let y = row_y(i);
            let disabled = self.dropdown_disabled(*dd);
            let open = !disabled && self.open_dropdown == Some(*dd);
            let hov = !disabled && point_in(mouse, bx, y, bhw, bhh);
            let lw = ctx.text_width(&font, label, 21.6 * fit, 550.0);
            ctx.text(lx + lw / 2.0, y, &font, label, 21.6 * fit, 550.0, DIM, z);
            ctx.rectangle(bx, y, bhw, bhh, if open || hov { HOVER } else { TRACK }, z);
            ctx.text(
                bx,
                y,
                &font,
                self.value_text(*dd),
                23.4 * fit,
                550.0,
                if disabled {
                    DIM
                } else {
                    self.value_color(*dd, hov || open)
                },
                z + 0.01,
            );
        }

        // Apply is enabled only when a complete, supported configuration
        // can be applied without restarting the application.
        let by = -h / 2.0 + 30.0 * fit;
        let apply_label = if self.is_hosted() { "DONE" } else { "APPLY" };
        let blocker = self.apply_blocker();
        let aw = (ctx.text_width(&font, apply_label, 23.4 * fit, 700.0) + 26.0 * fit) / 2.0;
        let ax = w / 2.0 - 32.0 * fit - aw; // right-aligned with a margin
        // Nothing differs from the last save: the button stays, greyed,
        // and swallows nothing — there is no click when there is no do.
        let dirty =
            (self.is_hosted() || self.apply_dirty() || self.audio_dirty()) && blocker.is_none();
        let back = !self.is_applying()
            && point_in(mouse, -w / 2.0 + 90.0 * fit, by, 65.0 * fit, 15.0 * fit);
        let reset = !self.is_applying() && point_in(mouse, 0.0, by, 65.0 * fit, 15.0 * fit);
        let apply = dirty && point_in(mouse, ax, by, aw, 15.0 * fit);
        ctx.rectangle(
            -w / 2.0 + 90.0 * fit,
            by,
            65.0 * fit,
            15.0 * fit,
            if back { HOVER } else { TRACK },
            z,
        );
        ctx.text(
            -w / 2.0 + 90.0 * fit,
            by,
            &font,
            "BACK",
            23.4 * fit,
            650.0,
            if self.is_applying() {
                DIM
            } else if back {
                AMBER
            } else {
                FG
            },
            z + 0.01,
        );
        ctx.rectangle(
            0.0,
            by,
            65.0 * fit,
            15.0 * fit,
            if reset { HOVER } else { TRACK },
            z,
        );
        ctx.text(
            0.0,
            by,
            &font,
            "RESET",
            23.4 * fit,
            650.0,
            if reset { AMBER } else { DIM },
            z + 0.01,
        );
        ctx.rectangle(
            ax,
            by,
            aw,
            15.0 * fit,
            if apply {
                HOVER
            } else if dirty {
                TRACK
            } else {
                BG
            },
            z,
        );
        ctx.text(
            ax,
            by,
            &font,
            apply_label,
            23.4 * fit,
            700.0,
            if apply {
                AMBER
            } else if dirty {
                GREEN
            } else {
                DIM
            },
            z + 0.01,
        );

        if self.is_hosted() {
            let shared = &self.shared;
            let level = |peak: f32| {
                if peak > 0.0 {
                    format!("{:.0} dB", 20.0 * peak.log10())
                } else {
                    "silent".into()
                }
            };
            for (offset, text) in [
                (
                    57.0,
                    format!(
                        "Calls {} · Feed {} · Read {} · Lost {}",
                        shared.process_calls.load(Ordering::Relaxed),
                        shared.feed_samples.load(Ordering::Relaxed),
                        self.received_samples,
                        shared.dropped_samples.load(Ordering::Relaxed)
                    ),
                ),
                (
                    39.0,
                    format!(
                        "{} ch · L {} · R {} · editor {}",
                        shared.process_channels.load(),
                        level(shared.channel_peaks[0].load()),
                        level(shared.channel_peaks[1].load()),
                        if shared.is_open() { "on" } else { "off" }
                    ),
                ),
            ] {
                ctx.text(
                    0.0,
                    by + offset * fit,
                    &font,
                    text,
                    15.0 * fit,
                    550.0,
                    DIM,
                    z,
                );
            }
        }

        // Transient status only, on demand — no standing banner text.
        if let Some(note) = self.save_note.as_ref().or(blocker.as_ref()) {
            ctx.text(
                0.0,
                by + 34.0 * fit,
                &font,
                clip(note, 90),
                21.6 * fit,
                550.0,
                AMBER,
                z,
            );
        }

        // Open dropdown: drawn last (on top), items hit first.
        let row = ROWS
            .iter()
            .position(|(_, d)| Some(*d) == self.open_dropdown)
            .unwrap_or(0);
        if self.draw_dropdown(
            ctx,
            [w, h],
            fit,
            Placement::Row([bx, row_y(row)], [bhw, bhh]),
            z,
            clicked,
        ) {
            return;
        }

        if clicked {
            if self.open_dropdown.is_some() {
                self.open_dropdown = None;
                self.list_scroll = 0.0;
            } else {
                for (i, (_, dd)) in ROWS.iter().enumerate() {
                    if !self.dropdown_disabled(*dd) && point_in(mouse, bx, row_y(i), bhw, bhh) {
                        if self.settings.backend.as_deref() == Some("jack")
                            && matches!(dd, Dropdown::Input | Dropdown::Output | Dropdown::OutputCh)
                        {
                            self.refresh_devices();
                        }
                        self.open_dropdown = Some(*dd);
                        self.list_scroll = 0.0;
                        self.dropdown_scroll_drag = false;
                        break;
                    }
                }
                if back {
                    self.leave_settings();
                    return;
                }
                if reset {
                    self.reset_settings_draft();
                }
                if apply {
                    if self.is_hosted() {
                        self.leave_settings();
                        return;
                    }
                    #[cfg(feature = "standalone")]
                    if self.audio_dirty() {
                        let request = self
                            .snap
                            .as_ref()
                            .ok_or_else(|| "Device information is not available".to_string())
                            .and_then(|snapshot| snapshot.resolve_settings(&self.settings))
                            .and_then(crate::standalone::control::request);
                        match request {
                            Ok(result) => {
                                self.pending_apply = Some(PendingAudioApply {
                                    result,
                                    settings: self.settings.clone(),
                                });
                                self.save_note = Some("Applying audio settings…".into());
                            }
                            Err(error) => self.save_note = Some(error),
                        }
                        return;
                    }
                    self.save_applied_settings(self.settings.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Dropdown, TunerUi};
    use super::{
        GAUGE_KNEE_OPTIONS, cents_to_units, fmt_rate, frequency_readout, tabular_glyphs, trim_zeros,
    };
    use crate::TunerSharedState;
    use std::sync::Arc;

    fn channel_ui() -> TunerUi {
        TunerUi::new(Arc::new(TunerSharedState::new()), Arc::from([]))
    }

    #[test]
    fn successful_apply_returns_to_main_but_save_failure_stays_in_settings() {
        let mut ui = channel_ui();
        ui.page = super::Page::Settings;
        let next = crate::settings::Settings {
            preferences: crate::settings::TunerPreferences {
                a4_ref: Some(432.0),
                ..Default::default()
            },
            backend: Some("dummy".into()),
            ..Default::default()
        };
        ui.record_apply_save(next.clone(), Err("disk is read-only".into()));
        assert!(ui.page == super::Page::Settings);
        assert_eq!(ui.saved, crate::settings::Settings::default());
        assert!(ui.save_note.as_ref().unwrap().contains("disk is read-only"));
        ui.record_apply_save(next.clone(), Ok(()));
        assert!(ui.page == super::Page::Main);
        assert_eq!(ui.saved, next);
        assert!(!ui.audio_dirty());
        assert!(!ui.apply_dirty());
        assert!(ui.save_note.is_none());
    }

    #[test]
    fn back_and_reset_do_not_commit_drafts_and_back_restores_live_preferences() {
        let mut ui = channel_ui();
        let saved = crate::settings::Settings {
            preferences: crate::settings::TunerPreferences {
                a4_ref: Some(432.0),
                gate_db: Some(-48.0),
                ..Default::default()
            },
            backend: Some("dummy".into()),
            ..Default::default()
        };
        ui.record_apply_save(saved.clone(), Ok(()));
        ui.page = super::Page::Settings;
        ui.reset_settings_draft();
        assert_eq!(ui.saved, saved);
        assert_eq!(ui.shared.a4_ref(), 440.0);
        ui.leave_settings();
        assert!(ui.page == super::Page::Main);
        assert_eq!(ui.settings, saved);
        assert_eq!(ui.saved, saved);
        assert_eq!(ui.shared.a4_ref(), 432.0);
        assert_eq!(ui.shared.gate_db.load(), -48.0);
    }

    #[test]
    fn apply_captures_size_changes_that_happened_while_audio_was_pending() {
        let ui = channel_ui();
        let pending = crate::settings::Settings {
            window: Some(crate::settings::WindowGeom {
                x: None,
                y: None,
                width: 640.0,
                height: 400.0,
            }),
            ..Default::default()
        };
        ui.shared
            .set_logical_size(nice_plug::editor::dpi::LogicalSize::new(900.0, 600.0));
        *ui.shared.window_pos.lock().unwrap() = Some((30, 40));
        let current = ui.with_current_window(pending).window.unwrap();
        assert_eq!((current.width, current.height), (900.0, 600.0));
        assert_eq!((current.x, current.y), (Some(30), Some(40)));
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn back_waits_for_an_in_flight_apply_to_finish() {
        let mut ui = channel_ui();
        ui.page = super::Page::Settings;
        let (_reply, result) = crossbeam::channel::bounded(1);
        ui.pending_apply = Some(super::PendingAudioApply {
            result,
            settings: ui.settings.clone(),
        });
        ui.leave_settings();
        assert!(ui.page == super::Page::Settings);
        assert!(ui.pending_apply.is_some());
    }

    fn hosted_ui(
        events: std::rc::Rc<std::cell::RefCell<Vec<(crate::host_controls::HostParameter, f32)>>>,
    ) -> TunerUi {
        let mut ui = channel_ui();
        ui.set_host_controls(crate::host_controls::HostControls::for_test(
            Arc::new(crate::TunerParams::default()),
            move |parameter, value| events.borrow_mut().push((parameter, value)),
        ));
        ui
    }

    #[test]
    fn hosted_audio_fields_are_read_only_and_report_actual_operating_conditions() {
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut ui = hosted_ui(events.clone());
        assert!(ui.settings_loaded);
        assert!(ui.settings.backend.is_none());
        assert_eq!(ui.value_text(Dropdown::Rate), "Unavailable");
        assert_eq!(ui.value_text(Dropdown::InputBuffer), "Unavailable");
        ui.shared.input_channels.store(2);
        ui.shared.output_channels.store(1);
        ui.shared.sample_rate_hz.store(44100.0);
        ui.shared.max_buffer_frames.store(512);
        assert_eq!(ui.value_text(Dropdown::InputBuffer), "Up to 512 frames");
        ui.shared.buffer_frames.store(128);
        assert_eq!(ui.host_audio_summary(), "2 in · 1 out · 44100 Hz");
        assert_eq!(ui.value_text(Dropdown::Input), "2 channels");
        assert_eq!(ui.value_text(Dropdown::Output), "1 channel");
        assert_eq!(ui.value_text(Dropdown::OutputCh), "Mono");
        assert_eq!(ui.value_text(Dropdown::InputBuffer), "128 frames");
        assert_eq!(ui.value_text(Dropdown::OutputRate), "44100 Hz");
        for dd in [
            Dropdown::Backend,
            Dropdown::Input,
            Dropdown::InputCh,
            Dropdown::Output,
            Dropdown::OutputCh,
            Dropdown::Rate,
            Dropdown::OutputRate,
            Dropdown::InputBuffer,
            Dropdown::OutputBuffer,
        ] {
            assert!(ui.dropdown_disabled(dd));
            let before = ui.settings.clone();
            ui.pick(dd, 0);
            assert_eq!(ui.settings, before);
        }
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn hosted_preferences_notify_the_host_and_navigation_never_saves_standalone_settings() {
        use crate::host_controls::HostParameter;
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut ui = hosted_ui(events.clone());
        ui.open_settings_page();
        assert!(ui.page == super::Page::Settings);
        for (dd, raw) in [
            (Dropdown::A4, "442"),
            (Dropdown::Gate, "-40"),
            (Dropdown::InTune, "7"),
            (Dropdown::GaugeCurve, "10"),
        ] {
            assert!(!ui.dropdown_disabled(dd));
            let index = ui
                .dropdown_rows(dd)
                .0
                .iter()
                .position(|(_, value)| value == raw)
                .unwrap();
            ui.pick(dd, index);
        }
        assert_eq!(
            *events.borrow(),
            vec![
                (HostParameter::A4, 442.0),
                (HostParameter::Gate, -40.0),
                (HostParameter::InTune, 7.0),
                (HostParameter::GaugeCurve, 10.0)
            ]
        );
        let saved = ui.saved.clone();
        ui.leave_settings();
        assert!(ui.page == super::Page::Main);
        assert_eq!(ui.saved, saved); // Host mode has no JSON save/commit operation.
        assert_eq!(events.borrow().len(), 4); // Closing must not undo live host edits.
        events.borrow_mut().clear();
        ui.reset_settings_draft();
        assert_eq!(
            *events.borrow(),
            vec![
                (HostParameter::A4, 440.0),
                (HostParameter::Gate, -60.0),
                (HostParameter::InTune, 3.0),
                (HostParameter::GaugeCurve, 5.0)
            ]
        );
    }

    #[test]
    fn output_rate_picker_follows_only_the_output_device_default() {
        let mut ui = channel_ui();
        assert!(ui.dropdown_disabled(Dropdown::OutputRate));
        ui.settings.output_device = Some("Speakers".into());
        ui.output_rate_opts = vec![fmt_rate(44100.0), fmt_rate(48000.0)];
        ui.driver_rate = Some(48000.0);
        assert_eq!(ui.value_text(Dropdown::OutputRate), "—");
        ui.default_output_rate = Some(44100.0);
        assert!(!ui.dropdown_disabled(Dropdown::OutputRate));
        assert_eq!(ui.value_text(Dropdown::OutputRate), fmt_rate(44100.0));
        assert!(ui.is_default(Dropdown::OutputRate));
        let rows = ui.dropdown_rows(Dropdown::OutputRate).0;
        assert_eq!(rows.iter().filter(|(_, raw)| raw == "44100").count(), 1);
        ui.pick(Dropdown::OutputRate, 1);
        assert_eq!(ui.settings.output_sample_rate, Some(48000.0));
        assert!(ui.audio_dirty());
        assert!(!ui.is_default(Dropdown::OutputRate));
        ui.pick(Dropdown::OutputRate, 0);
        assert_eq!(ui.settings.output_sample_rate, None);
        assert!(ui.is_default(Dropdown::OutputRate));
    }

    #[test]
    fn output_channel_picker_requires_a_device_and_shows_its_default() {
        use crate::settings::OutputChannels::{Mono, Stereo};
        let mut ui = channel_ui();
        assert!(ui.dropdown_disabled(Dropdown::OutputCh));
        ui.settings.output_device = Some("Speakers".into());
        ui.output_channel_opts = vec![Mono(1), Mono(2), Stereo([1, 2])];
        ui.default_output_channels = Some(Stereo([1, 2]));
        assert!(!ui.dropdown_disabled(Dropdown::OutputCh));
        assert_eq!(ui.value_text(Dropdown::OutputCh), "1–2 (stereo)");
        assert!(ui.is_default(Dropdown::OutputCh));
        let rows = ui.dropdown_rows(Dropdown::OutputCh).0;
        assert_eq!(rows[0].1, "1,2");
        assert_eq!(rows.iter().filter(|(_, raw)| raw == "1,2").count(), 1);
        ui.pick(Dropdown::OutputCh, 2);
        assert_eq!(ui.settings.output_channels, Some(Mono(2)));
        assert!(ui.audio_dirty());
        assert!(ui.apply_dirty());
        ui.pick(Dropdown::OutputCh, 0);
        assert_eq!(ui.settings.output_channels, None);
        assert_eq!(ui.value_text(Dropdown::OutputCh), "1–2 (stereo)");
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn incomplete_audio_settings_disable_apply_and_unknown_defaults_stay_unset() {
        let mut ui = channel_ui();
        ui.settings.backend = Some("core-audio".into());
        ui.snap = Some(crate::devices::Snapshot::default());
        assert!(ui.apply_blocker().is_some());
        assert_eq!(ui.buffer_default(Dropdown::InputBuffer), None);
        assert_eq!(ui.buffer_default(Dropdown::OutputBuffer), None);
        assert_eq!(ui.value_text(Dropdown::InputBuffer), "—");
        assert_eq!(ui.value_text(Dropdown::OutputBuffer), "—");
        ui.settings.backend = Some("unsupported-backend".into());
        assert!(
            ui.apply_blocker()
                .unwrap()
                .contains("not supported by the standalone application")
        );
    }

    #[cfg(all(target_os = "linux", feature = "standalone"))]
    #[test]
    fn jack_picker_preserves_named_destinations_and_supports_optional_right_output() {
        let mut ui = channel_ui();
        ui.settings.backend = Some("jack".into());
        ui.default_in = Some("instrument:capture_1".into());
        ui.default_out = Some("interface:playback_L".into());
        ui.output_opts = vec!["interface:playback_L".into(), "interface:playback_R".into()];
        ui.output_channel_opts = vec![crate::settings::OutputChannels::Mono(1)];
        ui.applied = ui.settings.clone();
        ui.saved = ui.settings.clone();
        let (rows, _) = ui.dropdown_rows(Dropdown::OutputCh);
        assert_eq!(
            rows,
            vec![
                ("Mono (no right output)".into(), String::new()),
                ("interface:playback_R".into(), "interface:playback_R".into()),
            ]
        );
        assert!(!ui.dropdown_disabled(Dropdown::OutputCh));
        assert!(ui.dropdown_disabled(Dropdown::InputCh));
        ui.pick(Dropdown::OutputCh, 1);
        assert_eq!(
            ui.settings.jack_output_right.as_deref(),
            Some("interface:playback_R")
        );
        assert_eq!(ui.current_raw(Dropdown::OutputCh), "interface:playback_R");
        assert!(ui.audio_dirty());
        assert!(ui.apply_dirty());
        ui.pick(Dropdown::OutputCh, 0);
        assert_eq!(ui.settings.jack_output_right, None);
        assert_eq!(ui.value_text(Dropdown::OutputCh), "Mono");
        assert!(!ui.audio_dirty());
        let long = "Very long audio interface name:playback_FR";
        let label = super::clip_port(long, 26);
        assert_eq!(label.chars().count(), 26);
        assert!(label.ends_with("playback_FR"));
    }

    #[cfg(all(target_os = "linux", feature = "standalone"))]
    #[test]
    fn jack_buffer_display_follows_callbacks_instead_of_saved_or_probed_size() {
        let mut ui = channel_ui();
        ui.settings.backend = Some("jack".into());
        ui.settings.period_size = Some(1024);
        ui.settings.output_period_size = Some(1024);
        ui.applied = ui.settings.clone();
        ui.saved = ui.settings.clone();
        ui.set_buffers_linked(true);
        ui.buffer_opts = crate::devices::JACK_BUFFER_CHOICES
            .iter()
            .map(u32::to_string)
            .collect();
        ui.snap = Some(crate::devices::Snapshot {
            buffer_pref: Some(1024),
            output_buffer_pref: Some(1024),
            ..Default::default()
        });
        assert_eq!(ui.value_text(Dropdown::InputBuffer), "1024");
        for frames in [64, 48, 32] {
            ui.shared.buffer_frames.store(frames);
            for dd in [Dropdown::InputBuffer, Dropdown::OutputBuffer] {
                assert_eq!(ui.value_text(dd), frames.to_string());
                assert_eq!(ui.current_raw(dd), frames.to_string());
                assert_eq!(ui.buffer_default(dd), Some(frames));
                assert_eq!(ui.dropdown_disabled(dd), dd == Dropdown::OutputBuffer);
                assert!(ui.is_default(dd));
            }
            assert!(!ui.audio_dirty());
            assert!(!ui.apply_dirty());
        }
        let rows = ui.dropdown_rows(Dropdown::InputBuffer).0;
        let index = rows.iter().position(|(_, raw)| raw == "128").unwrap();
        ui.pick(Dropdown::InputBuffer, index);
        assert_eq!(ui.value_text(Dropdown::InputBuffer), "128");
        assert_eq!(ui.value_text(Dropdown::OutputBuffer), "128");
        assert!(ui.audio_dirty());
        ui.settings = ui.applied.clone();
        // Displaying the live answer must not rewrite the startup request.
        assert_eq!(ui.settings.period_size, Some(1024));
        ui.settings.backend = Some("alsa".into());
        assert_eq!(ui.live_jack_buffer(), None);
        assert_eq!(ui.value_text(Dropdown::InputBuffer), "1024");
        ui.settings.backend = Some("jack".into());
        ui.applied.backend = None;
        assert_eq!(ui.live_jack_buffer(), None);
    }

    #[test]
    fn linked_output_buffer_follows_input_and_cannot_be_picked() {
        let mut ui = channel_ui();
        ui.buffer_opts = vec!["64".into(), "128".into()];
        ui.output_buffer_opts = vec!["256".into()];
        ui.settings.period_size = Some(64);
        ui.settings.output_period_size = Some(256);
        ui.open_dropdown = Some(Dropdown::OutputBuffer);
        ui.set_buffers_linked(true);
        assert!(ui.dropdown_disabled(Dropdown::OutputBuffer));
        assert!(!ui.dropdown_disabled(Dropdown::InputBuffer));
        assert!(ui.open_dropdown.is_none());
        assert_eq!(ui.value_text(Dropdown::OutputBuffer), "64");
        ui.pick(Dropdown::OutputBuffer, 1);
        assert_eq!(ui.settings.output_period_size, Some(64));
        ui.pick(Dropdown::InputBuffer, 2);
        assert_eq!(ui.settings.period_size, Some(128));
        assert_eq!(ui.settings.output_period_size, Some(128));
        ui.pick(Dropdown::InputBuffer, 0);
        assert_eq!(ui.settings.period_size, None);
        assert_eq!(ui.settings.output_period_size, None);
    }

    /// One ASIO driver per process is asio-sys law: the output row carries
    /// the input driver's name, cannot be repointed, and its buffer and rate
    /// are the driver's single values.
    #[test]
    fn asio_locks_output_device_buffer_and_rate_to_the_input_driver() {
        let mut ui = channel_ui();
        ui.settings.backend = Some("asio".into());
        ui.settings.input_device = Some("FlexASIO".into());
        ui.settings.output_device = Some("FlexASIO".into());
        assert!(ui.dropdown_disabled(Dropdown::Output));
        assert!(!ui.dropdown_disabled(Dropdown::Input));
        assert_eq!(ui.value_text(Dropdown::Output), "FlexASIO");
        ui.set_buffers_linked(true);
        assert!(ui.dropdown_disabled(Dropdown::OutputBuffer));
        let before = ui.settings.output_device.clone();
        ui.pick(Dropdown::Output, 1);
        assert_eq!(
            ui.settings.output_device, before,
            "a disabled row must not change"
        );
        ui.settings.sample_rate = Some(44100.0);
        ui.settings.output_sample_rate = Some(48000.0);
        ui.set_rates_linked(true);
        assert!(ui.dropdown_disabled(Dropdown::OutputRate));
        assert_eq!(ui.settings.output_sample_rate, Some(44100.0), "one clock");
        ui.pick(Dropdown::OutputRate, 0);
        assert_eq!(ui.settings.output_sample_rate, Some(44100.0));
        ui.settings.sample_rate = Some(48000.0);
        ui.sync_linked_rates();
        assert_eq!(ui.settings.output_sample_rate, Some(48000.0));
    }

    #[test]
    fn separate_output_buffer_can_be_changed_without_changing_input() {
        let mut ui = channel_ui();
        ui.output_buffer_opts = vec!["256".into()];
        ui.settings.period_size = Some(64);
        ui.set_buffers_linked(false);
        assert!(!ui.dropdown_disabled(Dropdown::OutputBuffer));
        ui.pick(Dropdown::OutputBuffer, 1);
        assert_eq!(ui.settings.period_size, Some(64));
        assert_eq!(ui.settings.output_period_size, Some(256));
        assert!(ui.audio_dirty());
        assert!(ui.apply_dirty());
    }

    #[test]
    fn channel_dropdown_uses_device_count_without_auto_or_invented_default() {
        let mut ui = channel_ui();
        for count in [1, 2, 12] {
            ui.settings.input_channel = None;
            ui.apply_input_channels(Some(count), None);
            let (rows, default) = ui.dropdown_rows(Dropdown::InputCh);
            assert!(default.is_empty());
            assert_eq!(rows.len(), count as usize + 1);
            assert!(rows[0].1.is_empty());
            for channel in 1..=count {
                assert_eq!(rows[channel as usize].1, channel.to_string());
            }
            assert_eq!(ui.value_text(Dropdown::InputCh), "—");
            assert_eq!(ui.settings.input_channel, None);
        }
    }

    #[test]
    fn channel_choice_and_unset_round_trip_without_promoting_runtime_fallback() {
        let mut ui = channel_ui();
        ui.apply_input_channels(Some(2), None);
        ui.pick(Dropdown::InputCh, 2);
        assert_eq!(ui.settings.input_channel, Some(2));
        assert_eq!(ui.shared.input_channel.load(), 1);
        assert_eq!(ui.value_text(Dropdown::InputCh), "channel 2");
        ui.pick(Dropdown::InputCh, 0);
        assert_eq!(ui.settings.input_channel, None);
        assert_eq!(ui.value_text(Dropdown::InputCh), "—");
        assert!(ui.current_raw(Dropdown::InputCh).is_empty());
    }

    #[test]
    fn reported_channel_default_is_selected_and_stale_choices_are_cleared() {
        let mut ui = channel_ui();
        ui.apply_input_channels(Some(4), Some(3));
        assert_eq!(ui.settings.input_channel, Some(3));
        assert_eq!(ui.shared.input_channel.load(), 1);
        assert_eq!(ui.value_text(Dropdown::InputCh), "channel 3");
        let rows = ui.dropdown_rows(Dropdown::InputCh).0;
        assert_eq!(rows[0], ("3".into(), "3".into()));
        assert_eq!(rows.iter().filter(|(_, raw)| raw == "3").count(), 1);

        // Switching to a smaller device cannot leave an invalid channel
        // selected; a default outside its capabilities is invalid too.
        ui.apply_input_channels(Some(1), Some(3));
        assert_eq!(ui.settings.input_channel, None);
        assert_eq!(ui.value_text(Dropdown::InputCh), "—");
        ui.settings.input_channel = Some(0); // legacy Auto
        ui.apply_input_channels(Some(1), None);
        assert_eq!(ui.settings.input_channel, None);
    }

    /// The channel belongs to the stream, which is opened with as many input
    /// planes as the applied setting names. Picking a wider channel must
    /// therefore be APPLY's business — and APPLY must rebuild the stream —
    /// rather than silently aim the meter and the monitor at a plane that is
    /// not there, which is what a live pick did: instant silence, no error, and
    /// an APPLY that changed nothing because the draft was already stored.
    #[test]
    fn a_wider_channel_is_an_audio_change_not_a_live_parameter() {
        let mut ui = channel_ui();
        ui.apply_input_channels(Some(4), Some(1));
        ui.applied.input_channel = Some(1);
        ui.settings.input_channel = Some(1);
        ui.shared.input_channel.store(1);
        assert!(!ui.audio_dirty());
        let third = ui
            .dropdown_rows(Dropdown::InputCh)
            .0
            .iter()
            .position(|(_, raw)| *raw == "3")
            .expect("a four-port driver offers channel 3");
        ui.pick(Dropdown::InputCh, third);
        assert_eq!(ui.settings.input_channel, Some(3));
        assert_eq!(
            ui.shared.input_channel.load(),
            1,
            "the running stream still has one plane"
        );
        assert!(ui.audio_dirty(), "a wider channel needs a wider stream");
    }

    #[test]
    fn unknown_channel_count_does_not_offer_guessed_channels() {
        let mut ui = channel_ui();
        for count in [None, Some(0)] {
            ui.apply_input_channels(count, None);
            let rows = ui.dropdown_rows(Dropdown::InputCh).0;
            assert_eq!(rows.len(), 1);
            assert!(rows[0].1.is_empty());
            assert_eq!(ui.value_text(Dropdown::InputCh), "—");
        }
    }

    #[test]
    fn waveform_shows_three_periods_ending_at_the_latest_sample() {
        let mut waveform = super::Waveform::default();
        waveform.reset(48000.0);
        let samples: Vec<_> = (0..6000)
            .map(|i| (std::f64::consts::TAU * i as f64 / 100.0).sin() as f32)
            .collect();
        waveform.push(&samples);
        waveform.advance(0.016);
        assert_eq!(waveform.samples.len(), samples.len().min(waveform.limit));
        let [points, current] = waveform.traces(480.0, 301);
        assert_eq!(points.len(), 301);
        assert_eq!(points[0][0], -1.0);
        assert_eq!(points[300][0], 1.0);
        assert!((current.last().unwrap()[1] - samples[5999]).abs() < 1e-5);
        for (i, point) in points.iter().enumerate() {
            assert!((point[1] - samples[5400 + i]).abs() < 1e-5);
        }
        waveform.reset(44100.0);
        assert!(waveform.traces(440.0, 301).iter().all(Vec::is_empty));
        assert!(waveform.traces(f64::NAN, 301).iter().all(Vec::is_empty));
    }

    #[test]
    fn scope_is_phase_stable_with_irregular_frame_times_and_audio_batches() {
        let rate = 48000.0;
        let hz = pitch::freq_to_note(82.41, 440.0).nearest_freq;
        let mut scope = super::Waveform::default();
        scope.reset(rate);
        let mut index = 0usize;
        for (frames, dt) in [(6000, 0.016), (743, 0.021), (1297, 0.009), (0, 0.1)] {
            let samples: Vec<_> = (index..index + frames)
                .map(|i| (std::f64::consts::TAU * hz * i as f64 / rate).sin() as f32)
                .collect();
            index += frames;
            scope.push(&samples);
            scope.advance(dt);
            for [x, y] in scope.traces(hz, 481).into_iter().flatten() {
                let expected = (3.0 * std::f64::consts::TAU * (f64::from(x) + 1.0) / 2.0).sin();
                assert!((f64::from(y) - expected).abs() < 0.001);
            }
        }
    }

    #[test]
    fn scope_retraces_only_at_edges_and_current_trace_reaches_newest_sample() {
        let mut scope = super::Waveform::default();
        scope.reset(48000.0);
        scope.push(&(0..6000).map(|i| i as f32).collect::<Vec<_>>());
        scope.advance(0.016);
        let traces = scope.traces(480.0, 301);
        for trace in &traces {
            assert_eq!(trace.first().unwrap()[0], -1.0);
            for pair in trace.windows(2) {
                assert!(pair[1][0] > pair[0][0]);
                assert!(pair[1][1] >= pair[0][1]);
            }
        }
        assert_eq!(traces[0].last().unwrap()[0], 1.0);
        assert!(traces[1].last().unwrap()[0] < 1.0);
        assert_eq!(traces[1].last().unwrap()[1], 1.0);
    }

    #[test]
    fn forced_note_drains_waveform_without_creating_a_pitch_detector() {
        let mut ui = channel_ui();
        let (mut producer, consumer) = rtrb::RingBuffer::new(1024);
        ui.rb = Some(consumer);
        ui.note_frequency = Some(440.0);
        ui.shared.sample_rate_hz.store(48000.0);
        ui.shared.input_rms.store(0.5);
        producer.push_entire_slice(&[0.25; 512]).unwrap();
        ui.analyze();
        assert!(ui.detector.is_none());
        assert_eq!(ui.shared.freq_hz.load(), 440.0);
        assert_eq!(ui.shared.rms.load(), 0.5);
        assert_eq!(ui.waveform.samples.len(), 512);
        assert!(
            ui.shared
                .last_detection_ms
                .load(std::sync::atomic::Ordering::Relaxed)
                > 0
        );
    }

    #[test]
    fn trim_zeros_hides_padding_but_keeps_the_zero_before_the_decimal() {
        assert_eq!(trim_zeros("0055.0"), "  55.0");
        assert_eq!(trim_zeros("0005.0"), "   5.0");
        assert_eq!(trim_zeros("+00.7¢"), "+ 0.7¢");
        assert_eq!(trim_zeros("−00.0¢"), "− 0.0¢");
        assert_eq!(trim_zeros("0440.0"), " 440.0");
        assert_eq!(trim_zeros("+05.2¢"), "+ 5.2¢");
        assert_eq!(trim_zeros("0.7"), "0.7");
        assert_eq!(trim_zeros("0.0"), "0.0");
        assert_eq!(trim_zeros("440.0"), "440.0");
        for s in ["0055.0", "0440.0", "440.0", "+00.7¢", "0.7"] {
            assert_eq!(trim_zeros(s).chars().count(), s.chars().count());
        }
    }

    #[test]
    fn cents_decimal_replaces_idle_dash_at_the_same_position() {
        let idle: Vec<_> = tabular_glyphs("—", 12.0, '.').collect();
        assert_eq!(idle, [('—', 0.0)]);
        for (reading, visible) in [
            ("+00.0¢", "+0.0¢"),
            ("−00.7¢", "−0.7¢"),
            ("+05.2¢", "+5.2¢"),
            ("−50.0¢", "−50.0¢"),
            ("+100.0¢", "+100.0¢"),
        ] {
            let text = trim_zeros(reading);
            let glyphs: Vec<_> = tabular_glyphs(&text, 12.0, '.').collect();
            assert_eq!(
                glyphs.iter().find(|(ch, _)| *ch == '.').unwrap().1,
                idle[0].1
            );
            assert_eq!(glyphs.iter().find(|(ch, _)| *ch == '¢').unwrap().1, 24.0);
            assert_eq!(glyphs.iter().map(|(ch, _)| ch).collect::<String>(), visible);
        }
    }

    #[test]
    fn frequency_is_right_aligned_with_fixed_units_and_reference() {
        let idle = frequency_readout(None, 440.0);
        let idle_glyphs: Vec<_> = tabular_glyphs(&idle, 12.0, '·').collect();
        let position =
            |glyphs: &[(char, f32)], ch| glyphs.iter().find(|(glyph, _)| *glyph == ch).unwrap().1;
        for (hz, visible) in [
            (0.7, "0.7"),
            (5.2, "5.2"),
            (55.0, "55.0"),
            (440.0, "440.0"),
            (1000.0, "1000.0"),
            (9999.96, "10000.0"),
        ] {
            let text = frequency_readout(Some(hz), 442.0);
            assert_eq!(text.split(" Hz").next().unwrap().trim(), visible);
            assert!(text.ends_with("A4=442"));
            let glyphs: Vec<_> = tabular_glyphs(&text, 12.0, '·').collect();
            assert_eq!(position(&glyphs, '·'), 0.0);
            assert_eq!(position(&glyphs, '.'), position(&idle_glyphs, '—'));
            for ch in ['H', 'z', 'A', '='] {
                assert_eq!(position(&glyphs, ch), position(&idle_glyphs, ch));
            }
        }
    }

    #[test]
    fn gauge_scale_endpoints_symmetry_and_midroom() {
        // Endpoints land exactly on the gauge edges, at EVERY selectable
        // knee (the panel can pick any of them).
        for &knee in &GAUGE_KNEE_OPTIONS {
            assert!((cents_to_units(50.0, knee) - 240.0).abs() < 1e-2);
            assert!((cents_to_units(-50.0, knee) + 240.0).abs() < 1e-2);
            // Odd and monotone — the gauge never folds.
            let mut prev = f32::MIN;
            let mut c = -50.0f32;
            while c <= 50.0 {
                let x = cents_to_units(c, knee);
                assert!(x > prev, "monotone at {c} knee {knee}");
                assert!(
                    (cents_to_units(-c, knee) + x).abs() < 1e-2,
                    "odd at {c} knee {knee}"
                );
                prev = x;
                c += 0.5;
            }
            // The point of it all: the ±3 ¢ verdict zone is ≥ 3× the
            // linear span (14.4 units) at the DEFAULT knee…
            if (knee - 5.0).abs() < 0.01 {
                assert!(
                    cents_to_units(3.0, knee) >= 44.0,
                    "mid gains too little room"
                );
            }
            // …and smaller knees never shrink it.
            if knee <= 5.0 {
                assert!(cents_to_units(3.0, knee) >= cents_to_units(3.0, 5.0));
            }
            // Beyond-range input (EMA overshoot) cannot escape the gauge.
            assert!(cents_to_units(80.0, knee) <= 240.0 + 1e-2);
        }
        // The 10 ¢ mark sits before the gauge's midpoint at the default.
        assert!(
            cents_to_units(10.0, 5.0) < 120.0,
            "edges compress too weakly"
        );
    }
}
