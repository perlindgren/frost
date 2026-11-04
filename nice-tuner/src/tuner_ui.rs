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
use crate::settings::Settings;
use frost::{Color, Context, Process, Scene, SceneNode, Shape};
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
    // ---- settings page (standalone-only UI; inert state elsewhere) ----
    page: Page,
    prev_mouse_down: bool,
    settings: Settings,
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
        Self {
            shared,
            font,
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
            esc_held: false,
            applied: Settings::default(),
            saved: Settings::default(),
            cold: false,
            settings_loaded: false,
            devices_loaded: false,
            open_dropdown: None,
            list_scroll: 0.0,
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
            save_note: None,
            #[cfg(feature = "standalone")]
            pending_apply: None,
            rb,
            detector: None,
            det_rate: 0.0,
        }
    }

    /// Drain the RT feed and run pitch detection — on this (UI) thread, so
    /// the bursty window search can at worst delay the needle, never glitch
    /// audio. Called once per frost frame, including while the settings page
    /// is up (state keeps warming behind it).
    fn analyze(&mut self) {
        let Some(rb) = self.rb.as_mut() else { return };
        let rate = self.shared.sample_rate_hz.load();
        if rate > 0.0 && (self.detector.is_none() || self.det_rate != rate) {
            self.detector = Some(pitch::PitchDetector::new(pitch::Config {
                sample_rate: rate,
                min_freq: 30.0,   // 5-string bass B0
                max_freq: 2100.0, // harmonics up past the guitar's E7 territory
                ..Default::default()
            }));
            self.det_rate = rate;
        }
        let Some(detector) = self.detector.as_mut() else {
            return;
        };
        // Catch-up cap: after a long pump stall, analyze the most recent
        // audio rather than grinding through a stale backlog. Older samples
        // are dropped from the head (Consumer side: legal in SPSC).
        const MAX_DRAIN: usize = 16384; // ~0.34 s
        if rb.slots() > MAX_DRAIN {
            let skip = rb.slots() - MAX_DRAIN;
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
            drained = true;
            if let Some(d) = detector.push_samples(&buf[..want]) {
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
        let dt = dt.clamp(0.0, 0.25); // tolerate host pauses
        // First frame: read the settings file. Look-and-judgement values
        // (verdict window, gauge curve) apply in hosts too — nothing RT
        // consumes them; audio settings stay standalone-only by design.
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
        #[cfg(feature = "standalone")]
        self.poll_audio_apply();
        self.analyze();

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
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            let gx = w / 2.0 - 62.0 * fit;
            let gy = h / 2.0 - 18.0 * fit;
            let hov = point_in(ctx.mouse_position(), gx, gy, 50.0 * fit, 11.0 * fit);
            ctx.rectangle(
                gx,
                gy,
                50.0 * fit,
                11.0 * fit,
                if hov { HOVER } else { TRACK },
                0.4,
            );
            ctx.text(
                gx,
                gy,
                &self.font,
                "SETTINGS",
                19.8 * fit,
                650.0,
                if hov { AMBER } else { DIM },
                0.41,
            );
            if hov && clicked {
                self.open_settings_page();
                return;
            }
        }
        if !crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            // Inside a host there is no settings page on purpose: the host
            // owns the device, a per-plugin device picker would lie.
            ctx.text(
                -w / 2.0 + 78.0 * fit,
                h / 2.0 - 16.0 * fit,
                &self.font,
                "audio: host-managed",
                18.0 * fit,
                550.0,
                DIM,
                0.4,
            );
        }
        let font = &self.font;
        let knee = self.settings.gauge_knee_cents.unwrap_or(GAUGE_KNEE_CENTS);
        let in_tune_cents = self.settings.in_tune_cents.unwrap_or(IN_TUNE_CENTS as f32);
        let x = cents_to_units(self.needle_cents, knee) * fit;

        // Gauge ticks: dynamic (they follow the panel-adjustable curve),
        // sharing the needle's mapping so they can never lie about it.
        for cents in (-50..=50).step_by(10) {
            let tx = cents_to_units(cents as f32, knee) * fit;
            let long = cents % 50 == 0;
            ctx.rectangle(
                tx,
                -2.0 * fit,
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
            -2.0 * fit,
            cents_to_units(in_tune_cents, knee) * fit,
            9.0 * fit,
            Color { a: 0.15, ..GREEN },
            0.05,
        );
        // needle
        ctx.line(
            x,
            (-2.0 - 16.0) * fit,
            x,
            (-2.0 + 16.0) * fit,
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
        ctx.text(0.0, 95.0 * fit, font, label, 86.4 * fit, 700.0, main, 0.2);
        // cents readout: TABULAR on a fixed pitch — every glyph lands in
        // its own digit-width slot (see draw_tabular). Constant glyph
        // count alone was not enough: `ctx.text` centers the INK box,
        // and `+` versus `−` carry visibly different ink widths, so the
        // whole line shifted a hair whenever the sign flipped. Slotted
        // layout cannot: nothing depends on what the glyphs look like.
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
            0.0,
            -52.0 * fit,
            &cents_s,
            50.4 * fit,
            600.0,
            main,
            0.2,
        );
        // frequency + reference — same tabular rule: always exactly six
        // glyphs before " Hz" (zero-padded width incl. the dot), and the
        // idle bar spans the same six, so even the "· A4=" tail cannot
        // shift.
        let freq_s = if signal {
            format!(
                "{} Hz  ·  A4={:.0}",
                trim_zeros(&format!("{:06.1}", self.freq_shown)),
                a4
            )
        } else {
            format!("——————  ·  A4={:.0}", a4)
        };
        ctx.text(0.0, -104.0 * fit, font, freq_s, 39.6 * fit, 550.0, DIM, 0.2);
        // ---- input meter with a real dBFS scale ----------------------------
        // 0 dBFS right, -48 left, ticks every 6 dB, as wide as the needle
        // sweep. Driven by meter_db (continuous), so it shows input level
        // always — silent room reads silence, no detection required.
        {
            let half = 240.0 * fit;
            let y = -150.0 * fit;
            ctx.rectangle(0.0, y, half, 2.5 * fit, Color { a: 0.5, ..TRACK }, 0.05);
            // dBFS scale: 0 at the right end, -48 at the left, every 6 dB.
            // Short ticks (±4) with numerals well below — digit tops must
            // not crowd the ticks.
            for i in 0..=8u32 {
                let xv = half - i as f32 * (half / 4.0);
                ctx.rectangle(xv, y, 1.0 * fit, 4.0 * fit, Color { a: 0.6, ..DIM }, 0.06);
                ctx.text(
                    xv,
                    y - 26.0 * fit,
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

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

/// Pad-with-zeros output, pruned to ONE anchor zero: "0055.0" → " 055.0",
/// "+00.7¢" → "+ 0.7¢", while "0440.0" → " 440.0" and a lone "0.7" keeps
/// its integer zero. The glyph COUNT is untouched, so the slotted
/// (draw_tabular) layout stays as immovable as with full zero padding —
/// only the ink changes: leading zeros that merely fill space become
/// blanks, the last zero stays as the digit the eye anchors on.
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
    for x in &mut c[start..end.saturating_sub(1).max(start)] {
        *x = ' ';
    }
    c.into_iter().collect()
}

fn fmt_rate(r: f32) -> String {
    if r.fract() == 0.0 {
        format!("{} Hz", r as u32)
    } else {
        format!("{r:.1} Hz")
    }
}

const VISIBLE_ITEMS: usize = 8;

impl TunerUi {
    fn open_settings_page(&mut self) {
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
    /// ASIO must be asked through a child process while this one
    /// streams: asio-sys keeps exactly ONE driver loaded (the live
    /// stream's) and refuses every other load, so in-process
    /// enumeration here sees only the driver already in use — device
    /// lists come back near-empty and stream options fall through to
    /// canned guesses (a hard-wired 64 …). A child opens no stream, so
    /// it sees every registered driver and can ask the SELECTED device
    /// for its real rates and buffer ranges. The live audio is never
    /// touched: nothing is closed, nothing restarts. The parent's own
    /// (partial) view is unioned in, so neither side's blind spot wins.
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

    fn dropdown_disabled(&self, dd: Dropdown) -> bool {
        self.is_applying()
            || (dd == Dropdown::OutputBuffer && self.buffers_linked)
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
                match pending.settings.save() {
                    Ok(()) => {
                        self.saved = pending.settings;
                        self.save_note = Some("saved — applied live".into());
                    }
                    Err(error) => {
                        self.save_note = Some(format!("audio applied, but save failed: {error}"))
                    }
                }
                self.refresh_devices();
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
        // The audio implementation still needs a concrete channel while the
        // settings remain unset. Its fallback must not appear as a default.
        self.shared
            .input_channel
            .store(self.settings.input_channel.unwrap_or(1));
    }

    /// A queried backend/device default, never a guessed wrapper constant.
    fn buffer_default(&self, dd: Dropdown) -> Option<u32> {
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
        let head = self.default_label(dd).unwrap_or_else(|| "\u{2014}".into());
        let mut rows = vec![(head, def_raw.clone())];
        rows.extend(
            opts.into_iter()
                .filter(|(_, raw)| raw.is_empty() || raw != &def_raw),
        );
        (rows, def_raw)
    }

    fn dropdown_items(&self, dd: Dropdown) -> Vec<String> {
        self.dropdown_rows(dd)
            .0
            .into_iter()
            .map(|(label, _)| label)
            .collect()
    }

    /// The raw value the default row selects ("" = unknowable default).
    fn default_raw(&self, dd: Dropdown) -> String {
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
                period.map(|p| p.to_string()).unwrap_or_else(|| {
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
            Dropdown::A4 => format!("{:.0}", self.shared.a4_ref()),
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
                self.settings.output_channels = None;
                self.settings.output_sample_rate = None;
                self.settings.output_period_size = None;
                // Duplex constraint: the output device narrows the buffer
                // range too — recompute.
                self.refresh_stream_options();
                #[cfg(feature = "standalone")]
                self.adopt_stream_defaults();
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
            }
            Dropdown::InputBuffer => {
                self.settings.period_size = if at_default { None } else { raw.parse().ok() };
                self.sync_linked_buffers();
            }
            Dropdown::OutputBuffer => {
                self.settings.output_period_size = if at_default { None } else { raw.parse().ok() };
            }
            // Channel select is live for both meter and detection.
            // No restart; APPLY persists. The empty default row stays None.
            Dropdown::InputCh => {
                self.settings.input_channel = if at_default { None } else { raw.parse().ok() };
                self.shared.input_channel.store(
                    self.settings
                        .input_channel
                        .or(self.default_input_channel)
                        .unwrap_or(1),
                );
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
        self.open_dropdown = None;
        self.list_scroll = 0.0;
    }

    /// Draw `text` glyph-by-glyph on a fixed advance pitch (FiraCode is
    /// monospaced, so one digit's width is THE pitch). Each glyph is
    /// ink-centered inside its own slot, so the line's layout depends
    /// only on the character COUNT — signs, dots and the ¢ mark can no
    /// longer push neighbours around. Whole-string `ctx.text` cannot
    /// promise that: its centering follows the ink box, and glyph ink
    /// varies (+ vs −, and future fonts).
    #[allow(clippy::too_many_arguments)] // immediate-mode draw call, as in frost
    fn draw_tabular(
        &self,
        ctx: &mut Context,
        cx: f32,
        cy: f32,
        text: &str,
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
        let n = text.chars().count() as f32;
        for (i, ch) in text.chars().enumerate() {
            if ch == ' ' {
                continue; // blanks carry layout, not ink
            }
            let mut buf = [0u8; 4];
            ctx.text(
                cx + (i as f32 - (n - 1.0) / 2.0) * pitch,
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
            || a.output_channels != s.output_channels
            || a.sample_rate != s.sample_rate
            || a.output_sample_rate != s.output_sample_rate
            || a.period_size != s.period_size
            || a.output_period_size != s.output_period_size
            || a.input_latency != s.input_latency
    }

    /// Anything to write at all: draft differs from the last save.
    /// Window GEOMETRY is excluded — it persists on its own (moved or
    /// resized, debounced; plus a flush at close), so counting it
    /// would keep APPLY perpetually "dirty" for merely using the app.
    fn apply_dirty(&self) -> bool {
        let (a, s) = (&self.saved, &self.settings);
        a.backend != s.backend
            || a.input_device != s.input_device
            || a.output_device != s.output_device
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

    fn process_settings_page(&mut self, ctx: &mut Context, clicked: bool) {
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
                self.page = Page::Main;
                self.save_note = None;
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

        // Wheel scrolls the open dropdown.
        if let Some(dd) = self.open_dropdown {
            let n = self.dropdown_items(dd).len();
            let max_scroll = n.saturating_sub(VISIBLE_ITEMS) as f32;
            self.list_scroll = (self.list_scroll + ctx.mouse_wheel() * 0.5).clamp(0.0, max_scroll);
        }

        for (i, (label, dd)) in ROWS.iter().enumerate() {
            let y = row_y(i);
            let disabled = self.dropdown_disabled(*dd);
            let open = !disabled && self.open_dropdown == Some(*dd);
            let hov = !disabled && point_in(mouse, bx, y, bhw, bhh);
            let lw = ctx.text_width(&font, label, 21.6 * fit, 550.0);
            ctx.text(lx + lw / 2.0, y, &font, *label, 21.6 * fit, 550.0, DIM, z);
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
        let apply_label = "APPLY";
        let blocker = self.apply_blocker();
        let aw = (ctx.text_width(&font, apply_label, 23.4 * fit, 700.0) + 26.0 * fit) / 2.0;
        let ax = w / 2.0 - 32.0 * fit - aw; // right-aligned with a margin
        // Nothing differs from the last save: the button stays, greyed,
        // and swallows nothing — there is no click when there is no do.
        let dirty = (self.apply_dirty() || self.audio_dirty()) && blocker.is_none();
        let back = point_in(mouse, -w / 2.0 + 90.0 * fit, by, 65.0 * fit, 15.0 * fit);
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
            if back { AMBER } else { FG },
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
        if let Some(dd) = self.open_dropdown {
            let items = self.dropdown_items(dd);
            let row = ROWS.iter().position(|(_, d)| *d == dd).unwrap_or(0);
            let ih = 13.0 * fit;
            let step = ih * 2.0 + 5.0 * fit;
            let skip = self.list_scroll as usize;
            let shown = items.len().saturating_sub(skip).min(VISIBLE_ITEMS);
            let slot_y =
                |i: usize| row_y(row) - (i as f32 - self.list_scroll) * step - ih - 8.0 * fit;

            // Backing panel: opaque, padded past the item stack on every
            // side, with a one-unit lighter rim. The list must read as its
            // own surface lifted off the page — not rows floating across the
            // tuner chrome. Drawn under the items (z), over everything else.
            if shown > 0 {
                let pad = 9.0 * fit;
                let top = slot_y(skip) + ih;
                let bottom = slot_y(skip + shown - 1) - ih;
                let py = (top + bottom) * 0.5;
                let ph = (top - bottom) * 0.5 + pad;
                let pw = bhw + pad;
                ctx.rectangle(bx, py, pw + 1.5 * fit, ph + 1.5 * fit, HOVER, 0.44);
                ctx.rectangle(bx, py, pw, ph, MENU_BG, 0.445);
            }

            for (idx, label) in items
                .iter()
                .enumerate()
                .skip(skip)
                .take(VISIBLE_ITEMS)
                .collect::<Vec<_>>()
            {
                let iy = slot_y(idx);
                let sel = self.selected_index(dd) == idx;
                let hov = point_in(mouse, bx, iy, bhw, ih);
                ctx.rectangle(
                    bx,
                    iy,
                    bhw,
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
                    bx,
                    iy,
                    &font,
                    clip(label, 36),
                    18.7 * fit,
                    if sel { 700.0 } else { 550.0 },
                    if idx == 0 && self.default_label(dd).is_some() {
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
                    return;
                }
            }
        }

        if clicked {
            if self.open_dropdown.is_some() {
                self.open_dropdown = None;
                self.list_scroll = 0.0;
            } else {
                for (i, (_, dd)) in ROWS.iter().enumerate() {
                    if !self.dropdown_disabled(*dd) && point_in(mouse, bx, row_y(i), bhw, bhh) {
                        self.open_dropdown = Some(*dd);
                        self.list_scroll = 0.0;
                        break;
                    }
                }
                if back {
                    self.page = Page::Main;
                    self.save_note = None;
                }
                if reset {
                    // Reset is a command, not a draft: defaults land in
                    // memory, in the live param atomics, and on disk at
                    // once. Audio streams keep running on the old config;
                    // Apply remains subject to complete stream validation.
                    self.settings = Settings::default();
                    self.shared
                        .a4_ref_bits
                        .store(440f64.to_bits(), Ordering::Relaxed);
                    self.shared.gate_db.store(-60.0);
                    self.shared.input_channel.store(1);
                    self.refresh_devices();
                    self.save_note = match self.settings.save() {
                        Ok(()) => {
                            self.saved = self.settings.clone();
                            Some("settings reset to defaults".to_string())
                        }
                        Err(e) => Some(format!("save failed: {e}")),
                    };
                }
                if apply {
                    // Carry the window's current geometry along: "last
                    // screen position and size" is a setting like any
                    // other, saved when the user says Apply. On Windows
                    // the live rect feeds it (see gui.rs); elsewhere
                    // only the size is knowable, position is preserved.
                    #[cfg(feature = "standalone")]
                    {
                        let ls = self.shared.logical_size();
                        let pos = *self.shared.window_pos.lock().unwrap();
                        self.settings.window = Some(crate::settings::WindowGeom {
                            x: pos.map(|p| p.0).or(self.settings.window.and_then(|g| g.x)),
                            y: pos.map(|p| p.1).or(self.settings.window.and_then(|g| g.y)),
                            width: ls.width,
                            height: ls.height,
                        });
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
                    match self.settings.save() {
                        Ok(()) => {
                            self.saved = self.settings.clone();
                            self.save_note = Some("saved — applied live".into());
                        }
                        Err(error) => self.save_note = Some(format!("save failed: {error}")),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Dropdown, TunerUi};
    use super::{GAUGE_KNEE_OPTIONS, cents_to_units, fmt_rate, trim_zeros};
    use crate::TunerSharedState;
    use std::sync::Arc;

    fn channel_ui() -> TunerUi {
        TunerUi::new(Arc::new(TunerSharedState::new()), Arc::from([]))
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
        ui.settings.backend = Some("jack".into());
        assert!(
            ui.apply_blocker()
                .unwrap()
                .contains("not supported by the standalone application")
        );
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
        assert_eq!(ui.shared.input_channel.load(), 2);
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
        assert_eq!(ui.shared.input_channel.load(), 3);
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
    fn trim_zeros_prunes_fill_keeps_anchor() {
        // Extra leading zeros become blanks…
        assert_eq!(trim_zeros("0055.0"), " 055.0");
        assert_eq!(trim_zeros("0005.0"), "  05.0");
        assert_eq!(trim_zeros("+00.7¢"), "+ 0.7¢");
        // …a single leading zero is the anchor and stays…
        assert_eq!(trim_zeros("0440.0"), "0440.0");
        assert_eq!(trim_zeros("+05.2¢"), "+05.2¢");
        assert_eq!(trim_zeros("0.7"), "0.7");
        // …no leading zeros: untouched; glyph counts are preserved.
        assert_eq!(trim_zeros("440.0"), "440.0");
        for s in ["0055.0", "0440.0", "440.0", "+00.7¢", "0.7"] {
            assert_eq!(trim_zeros(s).chars().count(), s.chars().count());
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
