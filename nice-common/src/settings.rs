//! Standalone audio-settings persistence.
//!
//! The standalone application owns its CPAL streams or JACK client and applies validated
//! settings in place. Plugin hosts own their audio devices and ignore these
//! standalone device settings.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Application-owned fields and configuration identity.
/// Configuration files live in the process working directory.
/// Preference field names must not overlap the shared audio/window fields.
pub trait Preferences: Clone + Default + Serialize + serde::de::DeserializeOwned {
    const FILE_NAME: &'static str;
    const APP_NAME: &'static str;
    fn is_blank(&self) -> bool;
}

/// Audio-only configuration for discovery clients and tests.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NoPreferences {}
impl Preferences for NoPreferences {
    const FILE_NAME: &'static str = "nice-common.json";
    const APP_NAME: &'static str = "nice-common";
    fn is_blank(&self) -> bool {
        true
    }
}

/// Additional monitoring buffering in seconds. Explicit settings override it;
/// device buffer sizes are queried separately.
pub const OUR_INPUT_LATENCY_DEFAULT: f32 = 0.005;

/// Shared standalone audio configuration with application-owned preferences.
/// The application resolves saved choices against queried device defaults before
/// opening streams. Audio command-line flags are ignored. `None` means follow
/// the reported default; an unset backend means silence.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings<P = NoPreferences> {
    #[serde(flatten)]
    pub preferences: P,
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
    /// Input channel, 1-based; absent = no channel selected (or follow a
    /// backend-reported default, when available). Legacy Auto (0) loads as
    /// unset. The audio path uses its safe fallback while unset;
    /// that fallback is not a device-reported default in the settings UI.
    #[serde(deserialize_with = "deserialize_input_channel")]
    pub input_channel: Option<u32>,
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

impl<P: Preferences> Settings<P> {
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
            .join(P::FILE_NAME)
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
            && self.input_channel.is_none()
            && self.preferences.is_blank()
    }

    /// A missing file is the default; a corrupt file is the default plus a
    /// warning to show. Startup must never depend on a file that can rot.
    pub fn load() -> (Self, Option<String>) {
        match std::fs::read_to_string(Self::path()) {
            Err(_) => (Self::default(), None),
            Ok(text) if text.trim().is_empty() => (Self::default(), None),
            Ok(text) => match serde_json::from_str::<Self>(&text) {
                Ok(mut s) => {
                    s.normalize();
                    (s, None)
                }
                Err(e) => (
                    Self::default(),
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

    /// Update geometry at an explicit path without overwriting preferences.
    pub fn save_window_to(path: &std::path::Path, window: WindowGeom) -> Result<(), String> {
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
/// probe. Applications stay silent until a backend is chosen in the panel.
/// Never leave the flag out:
/// nice-plug's clap default is `Auto`, and Auto tries JACK FIRST on
/// every platform — on a machine where jack2 put `libjack.dll` on the
/// search path that means loading it and ATTEMPTING TO START jackd
/// while the application boots (console floods, `jack_db-*` temp dirs,
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

/// One ASIO driver is clocked at one sample rate for both directions, so a
/// separate output rate is not something it can be asked for. Narrower than
/// the buffer rule above: CoreAudio keeps its independent per-device rates,
/// which the output rate picker exists to express.
pub fn rates_must_match(backend: &str, input: Option<&str>, output: Option<&str>) -> bool {
    backend == "asio" && input.is_some_and(|name| !name.is_empty() && Some(name) == output)
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
/// the application makes on the user's behalf.
pub fn effective_args<P: Preferences>(settings: &Settings<P>) -> Vec<String> {
    let mut args: Vec<String> = vec![P::APP_NAME.to_owned()];
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

impl<P> std::ops::Deref for Settings<P> {
    type Target = P;
    fn deref(&self) -> &P {
        &self.preferences
    }
}
impl<P> std::ops::DerefMut for Settings<P> {
    fn deref_mut(&mut self) -> &mut P {
        &mut self.preferences
    }
}
