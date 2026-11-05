//! Standalone audio-settings persistence.
//!
//! The standalone application owns its CPAL streams or JACK client and applies validated
//! settings in place. Plugin hosts own their audio devices and ignore these
//! standalone device settings.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Read from and written to the process's working directory.
pub const FILE_NAME: &str = "nice-tuner-settings.json";

/// Additional monitoring buffering in seconds. Explicit settings override it;
/// device buffer sizes are queried separately.
pub const OUR_INPUT_LATENCY_DEFAULT: f32 = 0.005;

/// Audio-system settings — the tuner's whole say over its own audio.
/// The application resolves saved choices against queried device defaults before
/// opening streams. Audio command-line flags are ignored. `None` means follow
/// the reported default; an unset backend means silence.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// A nice-plug `BackendType` value (clap spellings): `auto`, `jack`,
    /// `alsa`, `core-audio`, `wasapi`, `asio`, `dummy`.
    pub backend: Option<String>,
    /// Exact CPAL device name (ASIO driver name), or a full named JACK
    /// audio source port. For JACK, output_device is the left/mono destination.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    /// Optional second named JACK playback destination. Both destinations
    /// receive the full mono signal; absent means a single destination.
    pub jack_output_right: Option<String>,
    /// CPAL physical output route; JACK uses named destinations instead.
    pub output_channels: Option<OutputChannels>,
    pub sample_rate: Option<f32>,
    pub output_sample_rate: Option<f32>,
    pub period_size: Option<u32>,
    /// Output stream buffer size. `period_size` remains the input setting
    /// so existing settings files retain their chosen input size.
    pub output_period_size: Option<u32>,
    /// The alignment slack, in seconds, nice-plug inserts between its
    /// independent input and output streams.
    pub input_latency: Option<f32>,
    /// Plugin params, standalone-only. Inside a host these belong to the
    /// host (state save/load); standalone has no host, so the settings page
    /// plays that role. The editor reads these values directly.
    pub a4_ref: Option<f32>,
    pub gate_db: Option<f32>,
    /// Input channel, 1-based; absent = no channel selected (or follow a
    /// backend-reported default, when available). Legacy Auto (0) loads as
    /// unset. The audio path uses its safe fallback while unset;
    /// that fallback is not a device-reported default in the settings UI.
    #[serde(deserialize_with = "deserialize_input_channel")]
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

fn deserialize_input_channel<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u32>, D::Error> {
    Ok(Option::<u32>::deserialize(deserializer)?.filter(|&channel| channel > 0))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "channels", rename_all = "snake_case")]
pub enum OutputChannels {
    Mono(u32),
    Stereo([u32; 2]),
}

impl OutputChannels {
    pub fn channels(self) -> Vec<u32> {
        match self {
            Self::Mono(channel) => vec![channel],
            Self::Stereo(channels) => channels.to_vec(),
        }
    }
    pub fn raw(self) -> String {
        match self {
            Self::Mono(channel) => channel.to_string(),
            Self::Stereo([left, right]) => format!("{left},{right}"),
        }
    }
    pub fn label(self) -> String {
        match self {
            Self::Mono(channel) => format!("{channel} (mono)"),
            Self::Stereo([left, right]) => format!("{left}–{right} (stereo)"),
        }
    }
    pub fn parse(raw: &str) -> Option<Self> {
        let channels = raw
            .split(',')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        match channels.as_slice() {
            [channel] if *channel > 0 => Some(Self::Mono(*channel)),
            [left, right] if *left > 0 && *right > 0 && left != right => {
                Some(Self::Stereo([*left, *right]))
            }
            _ => None,
        }
    }
    pub fn fits(self, count: u32) -> bool {
        let channels = self.channels();
        channels
            .iter()
            .all(|&channel| channel > 0 && channel <= count)
            && (channels.len() == 1 || channels[0] != channels[1])
    }
    pub fn required_channels(self) -> u32 {
        self.channels().into_iter().max().unwrap_or(0)
    }
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
        if self.backend.as_deref() == Some("jack") {
            // Replace the old CPAL virtual-device labels with graph defaults.
            if self.input_device.as_deref() == Some("JACK Input") {
                self.input_device = None;
                self.input_channel = None;
            }
            if self.output_device.as_deref() == Some("JACK Output") {
                self.output_device = None;
            }
            self.output_channels = None;
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
            && self.jack_output_right.is_none()
            && self.output_channels.is_none()
            && self.sample_rate.is_none()
            && self.output_sample_rate.is_none()
            && self.period_size.is_none()
            && self.output_period_size.is_none()
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

    /// Update only window geometry, preserving saved preferences and unknown
    /// fields. Resizing must never commit the settings panel's draft.
    pub fn save_window(window: WindowGeom) -> Result<(), String> {
        Self::save_window_to(&Self::path(), window)
    }

    fn save_window_to(path: &std::path::Path, window: WindowGeom) -> Result<(), String> {
        if !window.width.is_finite()
            || !window.height.is_finite()
            || window.width <= 0.0
            || window.height <= 0.0
        {
            return Err("Cannot save an invalid window size".into());
        }
        let mut value: serde_json::Value = match std::fs::read_to_string(path) {
            Ok(text) if text.trim().is_empty() => serde_json::json!({}),
            Ok(text) => serde_json::from_str(&text)
                .map_err(|error| format!("{}: {error}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        let original = value.clone();
        let object = value
            .as_object_mut()
            .ok_or("Settings file must contain a JSON object")?;
        let geometry = object.entry("window").or_insert(serde_json::json!({}));
        if geometry.is_null() {
            *geometry = serde_json::json!({});
        }
        let geometry = geometry
            .as_object_mut()
            .ok_or("Window settings must contain a JSON object")?;
        geometry.insert("width".into(), window.width.into());
        geometry.insert("height".into(), window.height.into());
        if let Some(x) = window.x {
            geometry.insert("x".into(), x.into());
        }
        if let Some(y) = window.y {
            geometry.insert("y".into(), y.into());
        }
        if value != original {
            let text = serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
            std::fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))?;
        }
        Ok(())
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

/// CoreAudio configures buffer frame size per device; ASIO shares it
/// across the input/output sides of one driver. JACK's server controls the
/// buffer for all clients, including separate input/output virtual devices.
/// Other backends are not assumed to require a shared size merely because
/// names match.
pub fn buffers_must_match(backend: &str, input: Option<&str>, output: Option<&str>) -> bool {
    (cfg!(target_os = "linux") && backend == "jack")
        || matches!(backend, "core-audio" | "coreaudio" | "asio")
            && input.is_some_and(|name| !name.is_empty() && Some(name) == output)
}

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
    if let Some(channels) = settings.output_channels {
        push("--output-channels", channels.raw());
    }
    if let Some(r) = settings.sample_rate {
        push("--sample-rate", format_rate(r));
    }
    if let Some(p) = settings.output_period_size {
        push("--period-size", p.to_string());
    }
    if settings.period_size.is_some() || settings.output_period_size.is_some() {
        // The wrapper's legacy omission means "same as output". An unset
        // input setting instead follows its own 512-frame default.
        push(
            "--input-period-size",
            settings.period_size.unwrap_or(512).to_string(),
        );
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

    fn geometry_test_path() -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "nice-tuner-geometry-{}-{unique}.json",
            std::process::id()
        ))
    }

    #[test]
    fn resizing_updates_only_geometry_and_preserves_saved_and_unknown_preferences() {
        let path = geometry_test_path();
        let initial = serde_json::json!({
            "backend": "jack", "a4_ref": 432.0, "future_option": {"enabled": true},
            "window": {"x": 10, "y": 20, "width": 640.0, "height": 400.0, "future_window_option": 7}
        });
        std::fs::write(&path, initial.to_string()).unwrap();
        let size = WindowGeom {
            x: None,
            y: None,
            width: 900.0,
            height: 600.0,
        };
        Settings::save_window_to(&path, size).unwrap();
        let updated: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let mut expected = initial;
        expected["window"]["width"] = 900.0.into();
        expected["window"]["height"] = 600.0.into();
        assert_eq!(updated, expected);
        // An unchanged size must leave even the file's formatting untouched.
        let compact = updated.to_string();
        std::fs::write(&path, &compact).unwrap();
        Settings::save_window_to(&path, size).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), compact);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn resizing_can_create_a_geometry_only_config_and_never_overwrites_corrupt_json() {
        let path = geometry_test_path();
        let size = WindowGeom {
            x: None,
            y: None,
            width: 800.0,
            height: 500.0,
        };
        Settings::save_window_to(&path, size).unwrap();
        let restored: Settings =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(restored.window, Some(size));
        assert!(restored.is_blank());
        std::fs::write(&path, "broken config").unwrap();
        assert!(Settings::save_window_to(&path, size).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken config");
        assert!(Settings::save_window_to(&path, WindowGeom { width: 0.0, ..size }).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn output_sample_rate_round_trips_and_is_independent_of_input() {
        let settings = Settings {
            sample_rate: Some(48000.0),
            output_sample_rate: Some(44100.0),
            ..Default::default()
        };
        assert_eq!(
            serde_json::from_str::<Settings>(&serde_json::to_string(&settings).unwrap()).unwrap(),
            settings
        );
        assert_eq!(
            serde_json::from_str::<Settings>(r#"{"sample_rate":48000}"#)
                .unwrap()
                .output_sample_rate,
            None
        );
        assert!(
            !Settings {
                output_sample_rate: Some(44100.0),
                ..Default::default()
            }
            .is_blank()
        );
    }

    #[test]
    fn output_channel_configuration_round_trips_and_legacy_files_follow_default() {
        for route in [OutputChannels::Mono(2), OutputChannels::Stereo([3, 4])] {
            assert_eq!(OutputChannels::parse(&route.raw()), Some(route));
            let settings = Settings {
                output_channels: Some(route),
                ..Default::default()
            };
            let restored: Settings =
                serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
            assert_eq!(restored.output_channels, Some(route));
            assert!(!restored.is_blank());
            let args = effective_args(&restored);
            let at = args
                .iter()
                .position(|arg| arg == "--output-channels")
                .unwrap();
            assert_eq!(args[at + 1], route.raw());
        }
        assert_eq!(
            serde_json::from_str::<Settings>("{}")
                .unwrap()
                .output_channels,
            None
        );
        assert_eq!(OutputChannels::parse("0"), None);
        assert_eq!(OutputChannels::parse("2,2"), None);
    }

    #[test]
    fn buffer_sizes_are_saved_and_passed_independently() {
        let settings = Settings {
            period_size: Some(64),
            output_period_size: Some(256),
            ..Default::default()
        };
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
        let args = effective_args(&restored);
        let input = args
            .iter()
            .position(|arg| arg == "--input-period-size")
            .unwrap();
        let output = args.iter().position(|arg| arg == "--period-size").unwrap();
        assert_eq!(args[input + 1], "64");
        assert_eq!(args[output + 1], "256");
        let output_only = effective_args(&Settings {
            output_period_size: Some(256),
            ..Default::default()
        });
        let input = output_only
            .iter()
            .position(|arg| arg == "--input-period-size")
            .unwrap();
        assert_eq!(output_only[input + 1], "512");
        let legacy: Settings = serde_json::from_str(r#"{"period_size":128}"#).unwrap();
        assert_eq!(legacy.period_size, Some(128));
        assert_eq!(legacy.output_period_size, None);
        assert!(
            !Settings {
                output_period_size: Some(64),
                ..Default::default()
            }
            .is_blank()
        );
    }

    #[test]
    fn buffer_linking_requires_a_shared_device_and_backend_rule() {
        for backend in ["core-audio", "coreaudio", "asio"] {
            assert!(buffers_must_match(
                backend,
                Some("Interface"),
                Some("Interface")
            ));
            assert!(!buffers_must_match(
                backend,
                Some("Microphone"),
                Some("Speakers")
            ));
            assert!(!buffers_must_match(backend, None, None));
        }
        assert!(!buffers_must_match(
            "wasapi",
            Some("Interface"),
            Some("Interface")
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn jack_buffers_follow_one_server_across_different_virtual_devices() {
        assert!(buffers_must_match(
            "jack",
            Some("JACK Input"),
            Some("JACK Output")
        ));
    }

    #[test]
    fn input_channel_loads_legacy_auto_as_unset() {
        for json in ["{}", r#"{"input_channel":null}"#, r#"{"input_channel":0}"#] {
            let settings: Settings = serde_json::from_str(json).unwrap();
            assert_eq!(settings.input_channel, None);
        }
        let settings: Settings = serde_json::from_str(r#"{"input_channel":3}"#).unwrap();
        assert_eq!(settings.input_channel, Some(3));
        let unset = serde_json::to_value(Settings::default()).unwrap();
        assert!(unset["input_channel"].is_null());
    }

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
        assert_eq!(value_of("--input-period-size").as_deref(), Some("64"));
        assert_eq!(value_of("--period-size"), None);
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

    #[test]
    fn named_jack_destinations_round_trip_and_legacy_virtual_devices_migrate() {
        let mut settings = Settings {
            backend: Some("jack".into()),
            input_device: Some("JACK Input".into()),
            output_device: Some("JACK Output".into()),
            input_channel: Some(2),
            output_channels: Some(OutputChannels::Stereo([1, 2])),
            jack_output_right: Some("interface:playback_R".into()),
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(settings.input_device, None);
        assert_eq!(settings.output_device, None);
        assert_eq!(settings.input_channel, None);
        assert_eq!(settings.output_channels, None);
        let serialized = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            serde_json::from_str::<Settings>(&serialized).unwrap(),
            settings
        );
        assert_eq!(
            settings.jack_output_right.as_deref(),
            Some("interface:playback_R")
        );
    }
}
