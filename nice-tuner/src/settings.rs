//! Standalone audio-settings persistence and argv synthesis.
//!
//! nice-plug's standalone wrapper builds its audio streams from the parsed
//! CLI config before the plugin exists, and tears them down when the editor
//! closes — it cannot be reconfigured from inside the plugin. So the settings
//! panel persists changes here and the standalone entry respawns the process:
//! from the user's seat, a blink. Hosts never consult these values — in a
//! DAW the host owns the device — so the settings page does not exist there.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Read from and written to the process's working directory.
pub const FILE_NAME: &str = "nice-tuner-settings.json";

/// nice-plug's own `--input-latency` default (150 ms) is audible slap-back
/// for a live tuner; we stand in with this when neither the command line nor
/// the settings file has an opinion.
pub const OUR_INPUT_LATENCY_DEFAULT: f32 = 0.005;

/// Audio-system settings — the tuner's whole say over its own audio.
/// The panel writes this file and the file alone speaks: startup argv is
/// synthesized from it (see `effective_args`), and the command line has
/// no voice. `None` means "not chosen here" — for every field that is
/// its honest follow-the-default state; for `backend` it means silence
/// (see `BACKEND_WHEN_UNSET`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// A nice-plug `BackendType` value (clap spellings): `auto`, `jack`,
    /// `alsa`, `core-audio`, `wasapi`, `asio`, `dummy`.
    pub backend: Option<String>,
    /// Exact cpal device name — for ASIO, the ASIO *driver* name
    /// ("Komplete Input 1/2"), not a port/endpoint name.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub sample_rate: Option<f32>,
    pub period_size: Option<u32>,
    /// The alignment slack, in seconds, nice-plug inserts between its
    /// independent input and output streams.
    pub input_latency: Option<f32>,
    /// Plugin params, standalone-only. Inside a host these belong to the
    /// host (state save/load); standalone has no host, so the settings page
    /// plays that role. Never wrapper args — the editor reads them from
    /// this file itself, never through `effective_args`.
    pub a4_ref: Option<f32>,
    pub gate_db: Option<f32>,
    /// Input channel, 1-based; absent = channel 1 (the tuner's default
    /// single-instrument cable). An explicit 0 selects auto (hottest
    /// channel). A noisy sibling makes auto wrong — the right input
    /// should be picked explicitly.
    pub input_channel: Option<u32>,
    /// Tuning-verdict half-width and gauge curvature, cents (None = 3.0 /
    /// 5.0). Look-and-judgement values read by the editor itself, also in
    /// a host — unlike the audio settings, nothing RT consults them.
    pub in_tune_cents: Option<f32>,
    pub gauge_knee_cents: Option<f32>,
    /// Standalone window geometry (top-left in physical screen px; size
    /// in logical units). None = never saved → the window centers itself
    /// on first launch. Hosts own their windows; this is standalone-only.
    pub window: Option<WindowGeom>,
}

/// Serialized window placement for the standalone binary. `x`/`y` are
/// physical screen px (top-left) and known only on Windows — elsewhere
/// baseview has no position concept at all; size is logical px. Both
/// always round-trip: unknown position simply means "center again".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowGeom {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: f32,
    pub height: f32,
}

impl Settings {
    /// Repair known bad values from older writers. Panel builds before the
    /// clap-key fix wrote the cpal spelling "coreaudio", which nice-plug's
    /// argument parser rejects — the app then died at startup argv parsing,
    /// indistinguishable from "device refuses to open".
    pub fn normalize(&mut self) {
        if self.backend.as_deref() == Some("coreaudio") {
            self.backend = Some("core-audio".to_string());
        }
    }

    pub fn path() -> PathBuf {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(FILE_NAME)
    }

    /// Whether a settings FILE exists at all — distinct from "settings
    /// have values". The panel reads this at startup: a cold start (no
    /// file) shows unset audio rows as dashes, not narrated resolutions.
    pub fn exists() -> bool {
        Self::path().exists()
    }

    /// True when the file holds nothing a user could be said to have
    /// chosen: every setting absent. Window geometry does NOT count —
    /// it is written by moving a window, not by making a decision.
    /// A blank file means a cold start, however the file got there.
    pub fn is_blank(&self) -> bool {
        self.backend.is_none()
            && self.input_device.is_none()
            && self.output_device.is_none()
            && self.sample_rate.is_none()
            && self.period_size.is_none()
            && self.input_latency.is_none()
            && self.a4_ref.is_none()
            && self.gate_db.is_none()
            && self.input_channel.is_none()
            && self.in_tune_cents.is_none()
            && self.gauge_knee_cents.is_none()
    }

    /// A missing file is the default; a corrupt file is the default plus a
    /// warning to show. Startup must never depend on a file that can rot.
    pub fn load() -> (Settings, Option<String>) {
        match std::fs::read_to_string(Self::path()) {
            Err(_) => (Settings::default(), None),
            Ok(text) if text.trim().is_empty() => (Settings::default(), None),
            Ok(text) => match serde_json::from_str::<Settings>(&text) {
                Ok(mut s) => {
                    s.normalize();
                    (s, None)
                }
                Err(e) => (
                    Settings::default(),
                    Some(format!(
                        "ignoring unreadable {}: {e}",
                        Self::path().display()
                    )),
                ),
            },
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(Self::path(), text).map_err(|e| format!("{}: {e}", Self::path().display()))
    }
}

/// What an unset `backend` boots as: `dummy` — NO audio, no device, no
/// probe. The tuner stays silent until a backend is chosen in the panel
/// (the row that then lights APPLY & RESTART). Never leave the flag out:
/// nice-plug's clap default is `Auto`, and Auto tries JACK FIRST on
/// every platform — on a machine where jack2 put `libjack.dll` on the
/// search path that means loading it and ATTEMPTING TO START jackd
/// while the tuner boots (console floods, `jack_db-*` temp dirs,
/// seconds lost). Reaching jack — or any device at all — is a choice,
/// never an accident of a missing value.
pub const BACKEND_WHEN_UNSET: &str = "dummy";

fn format_rate(r: f32) -> String {
    if r.fract() == 0.0 {
        format!("{}", r as u32)
    } else {
        r.to_string()
    }
}

/// Synthesize the argv handed to nice-plug's clap parser. The settings
/// file is the ONLY voice: the app ignores its own command line, so a
/// driver is only ever reached through a value the panel wrote (or the
/// shipped silence). The first element is the binary name clap displays
/// and nothing else. Unnamed options keep nice-plug's defaults — that
/// remainder (tempo, time signature, dpi scale) drives no audio choice
/// the tuner could be accused of making on the user's behalf.
pub fn effective_args(settings: &Settings) -> Vec<String> {
    let mut args: Vec<String> = vec!["nice-tuner".to_owned()];
    let mut push = |flag: &str, value: String| {
        args.push(flag.to_owned());
        args.push(value);
    };

    // The backend is ALWAYS named — a choice or `dummy`, never left
    // out (absence is what hands nice-plug its jack-first `Auto`).
    push(
        "--backend",
        settings
            .backend
            .clone()
            .unwrap_or_else(|| BACKEND_WHEN_UNSET.to_owned()),
    );
    if let Some(d) = settings.input_device.as_deref() {
        push("--input-device", d.to_owned());
    }
    if let Some(d) = settings.output_device.as_deref() {
        push("--output-device", d.to_owned());
    }
    if let Some(r) = settings.sample_rate {
        push("--sample-rate", format_rate(r));
    }
    if let Some(p) = settings.period_size {
        push("--period-size", p.to_string());
    }
    push(
        "--input-latency",
        settings
            .input_latency
            .unwrap_or(OUR_INPUT_LATENCY_DEFAULT)
            .to_string(),
    );
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_geometry_round_trips_and_defaults_absent() {
        let s = Settings {
            window: Some(WindowGeom {
                x: Some(-1200),
                y: Some(80),
                width: 900.0,
                height: 560.0,
            }),
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        // Old files (no window key) load as "never saved" → the window
        // centres itself on the next start.
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.window, None);
        // Size-only entries (platforms without a position API omit x/y).
        let size_only: Settings =
            serde_json::from_str(r#"{"window":{"width":800.0,"height":500.0}}"#).unwrap();
        assert_eq!(
            size_only.window,
            Some(WindowGeom {
                x: None,
                y: None,
                width: 800.0,
                height: 500.0
            })
        );
    }

    #[test]
    fn blank_means_no_decision_geometry_never_counts() {
        // A file born from merely moving the window holds no decision:
        // still a cold slate.
        let geom_only = Settings {
            window: Some(WindowGeom {
                x: Some(10),
                y: Some(20),
                width: 800.0,
                height: 500.0,
            }),
            ..Default::default()
        };
        assert!(geom_only.is_blank());
        assert!(Settings::default().is_blank());
        // One value of any class and the slate is no longer blank.
        let one_choice = Settings {
            period_size: Some(512),
            ..Default::default()
        };
        assert!(!one_choice.is_blank());
        let look_only = Settings {
            in_tune_cents: Some(3.0),
            ..Default::default()
        };
        assert!(!look_only.is_blank());
    }

    #[test]
    fn settings_file_is_the_only_voice() {
        // The panel's file names everything it decided; nothing else
        // speaks. (The app ignores its own argv entirely — see main.)
        let s = Settings {
            backend: Some("asio".into()),
            input_device: Some("Komplete Audio ASIO Driver".into()),
            output_device: Some("Komplete Audio ASIO Driver".into()),
            sample_rate: Some(48000.0),
            period_size: Some(64),
            ..Default::default()
        };
        let args = effective_args(&s);
        let value_of = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .map(|i| args[i + 1].clone())
        };
        assert_eq!(value_of("--backend").as_deref(), Some("asio"));
        assert_eq!(
            value_of("--input-device").as_deref(),
            Some("Komplete Audio ASIO Driver")
        );
        assert_eq!(
            value_of("--output-device").as_deref(),
            Some("Komplete Audio ASIO Driver")
        );
        assert_eq!(value_of("--sample-rate").as_deref(), Some("48000"));
        assert_eq!(value_of("--period-size").as_deref(), Some("64"));
    }

    #[test]
    fn settings_file_beats_our_input_latency_default() {
        let s = Settings {
            input_latency: Some(0.02),
            ..Default::default()
        };
        let args = effective_args(&s);
        let i = args.iter().position(|a| a == "--input-latency").unwrap();
        assert_eq!(args[i + 1], "0.02");
        assert!(!args.iter().any(|a| a == "0.005"));
    }

    #[test]
    fn no_opinions_yet_boots_silent_not_default() {
        let args = effective_args(&Settings::default());
        let i = args.iter().position(|a| a == "--input-latency").unwrap();
        assert_eq!(args[i + 1], "0.005");
        // Unset backend is a CHOICE PENDING, not a default: dummy audio,
        // nothing connected, nothing probed.
        let b = args.iter().position(|a| a == "--backend").unwrap();
        assert_eq!(args[b + 1], "dummy");
        // Nothing else invented.
        assert_eq!(args.len(), 5);
    }

    #[test]
    fn unset_backend_never_touches_jack() {
        // The rule: no device is touched until the user speaks. An unset
        // backend must name `dummy` — never nice-plug's `Auto` (which
        // tries jackd first), never `jack`, never a system host.
        let args = effective_args(&Settings::default());
        assert!(!args.iter().any(|a| a == "auto"));
        assert!(!args.iter().any(|a| a == "jack"));
        assert!(!args.iter().any(|a| a == "wasapi"));
        assert!(!args.iter().any(|a| a == "core-audio"));
        assert!(!args.iter().any(|a| a == "alsa"));
    }

    #[test]
    fn chosen_auto_passes_through() {
        // Explicit `auto` in the file is a CHOICE — it passes through
        // untouched (that is the one road to jack-if-present, and the
        // user opted into it with their own hand).
        let s = Settings {
            backend: Some("auto".into()),
            ..Default::default()
        };
        let args = effective_args(&s);
        let b = args.iter().position(|a| a == "--backend").unwrap();
        assert_eq!(args[b + 1], "auto");
    }

    #[test]
    fn json_round_trip_and_lenient_reading() {
        let s = Settings {
            backend: Some("asio".into()),
            input_device: Some("Komplete Input 1/2".into()),
            period_size: Some(64),
            input_channel: Some(1),
            a4_ref: Some(432.0),
            in_tune_cents: Some(2.0),
            gauge_knee_cents: Some(7.0),
            ..Default::default()
        };
        let text = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back, s);
        // Unknown fields ignored, missing fields None — forward compatible.
        let loose: Settings =
            serde_json::from_str(r#"{"backend":"wasapi","future_thing":42}"#).unwrap();
        assert_eq!(loose.backend.as_deref(), Some("wasapi"));
        assert_eq!(loose.period_size, None);
    }

    #[test]
    fn normalize_repairs_legacy_coreaudio_key() {
        let mut s = Settings {
            backend: Some("coreaudio".into()),
            ..Default::default()
        };
        s.normalize();
        assert_eq!(s.backend.as_deref(), Some("core-audio"));
        // Correct values pass through untouched.
        let mut ok = Settings {
            backend: Some("wasapi".into()),
            ..Default::default()
        };
        ok.normalize();
        assert_eq!(ok.backend.as_deref(), Some("wasapi"));
    }
}
