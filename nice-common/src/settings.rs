//! Standalone audio-settings persistence.
//!
//! The standalone application owns its CPAL streams or JACK client and applies validated
//! settings in place. Plugin hosts own their audio devices and ignore these
//! standalone device settings.

use ron::Value;
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
    const FILE_NAME: &'static str = "nice-common.ron";
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
    /// Which audio system this application speaks, in the key spellings that
    /// `devices::available_backends` writes: on Windows `asio` and `dummy`,
    /// because ASIO is the Windows audio path and there is no system host to
    /// follow; elsewhere `auto`, `jack`, `alsa`, `core-audio` and `dummy`.
    /// A value outside what this build offers is refused at Apply with the
    /// words it was saved with — a choice that no longer exists is a question
    /// for the user, not something to rewrite into the one that works.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputChannels {
    Mono(u32),
    Stereo([u32; 2]),
}

impl<'de> Deserialize<'de> for OutputChannels {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Channels {
            Mono(u32),
            Stereo([u32; 2]),
        }
        #[derive(Deserialize)]
        struct Route {
            kind: String,
            channels: Channels,
        }
        let route = Route::deserialize(deserializer)?;
        match (route.kind.as_str(), route.channels) {
            ("mono", Channels::Mono(channel)) => Ok(Self::Mono(channel)),
            ("stereo", Channels::Stereo(channels)) => Ok(Self::Stereo(channels)),
            _ => Err(serde::de::Error::custom(
                "Expected mono channel or stereo channel pair",
            )),
        }
    }
}

// Keep the discriminant a string so RON's untyped tree retains it when a
// geometry update rewrites the file (Value deliberately does not retain enums).
impl Serialize for OutputChannels {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut value = serializer.serialize_struct("OutputChannels", 2)?;
        match self {
            Self::Mono(channel) => {
                value.serialize_field("kind", "mono")?;
                value.serialize_field("channels", channel)?;
            }
            Self::Stereo(channels) => {
                value.serialize_field("kind", "stereo")?;
                value.serialize_field("channels", channels)?;
            }
        }
        value.end()
    }
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
        Self::load_from(&Self::path())
    }

    /// Read the RON settings file at the supplied path.
    pub fn load_from(path: &std::path::Path) -> (Self, Option<String>) {
        let result = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|text| {
                if text.trim().is_empty() {
                    return Ok(Self::default());
                }
                Self::from_ron(&text)
            });
        match result {
            Ok(mut settings) => {
                settings.normalize();
                (settings, None)
            }
            Err(_) if !path.exists() => (Self::default(), None),
            Err(error) => (
                Self::default(),
                Some(format!("ignoring unreadable {}: {error}", path.display())),
            ),
        }
    }

    /// Decode RON settings, including the original bare-value/`()` option spelling.
    pub fn from_ron(text: &str) -> Result<Self, String> {
        Self::deserialize(SettingsValue(read_value(text)?)).map_err(|e| e.to_string())
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
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
        let mut value: Value = match std::fs::read_to_string(path) {
            Ok(text) if text.trim().is_empty() => Value::Map(ron::Map::new()),
            Ok(text) => {
                read_value(&text).map_err(|error| format!("{}: {error}", path.display()))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Value::Map(ron::Map::new())
            }
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        let original = value.clone();
        let Value::Map(object) = &mut value else {
            return Err("Settings file must contain a map".into());
        };
        let key = Value::from("window");
        if object.get(&key).is_none() {
            object.insert(key.clone(), Value::Map(ron::Map::new()));
        }
        let mut geometry = object.get_mut(&key).unwrap();
        if matches!(geometry, Value::Unit | Value::Option(None)) {
            *geometry = Value::Map(ron::Map::new());
        }
        // Typed RON writes Some(window); the original RON schema used a bare map.
        if let Value::Option(Some(inner)) = geometry {
            geometry = inner;
        }
        let Value::Map(geometry) = geometry else {
            return Err("Window settings must contain a map".into());
        };
        geometry.insert("width", window.width);
        geometry.insert("height", window.height);
        if let Some(x) = window.x {
            geometry.insert("x", Value::from(Some(x)));
        }
        if let Some(y) = window.y {
            geometry.insert("y", Value::from(Some(y)));
        }
        if value != original {
            write_value(path, &value)?;
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

// Use a self-describing map for the flattened application fields. In RON,
// keys are strings. This also lets geometry
// updates preserve unknown fields without decoding them into application types.
// Adapt RON's tree to Serde while accepting both explicit Some/None and the
// original settings spelling (bare values / unit). Recurse through maps and
// sequences so nested geometry and flattened preferences follow the same rules.
struct SettingsValue(Value);
impl<'de> serde::de::IntoDeserializer<'de, ron::Error> for SettingsValue {
    type Deserializer = Self;
    fn into_deserializer(self) -> Self {
        self
    }
}
impl<'de> serde::Deserializer<'de> for SettingsValue {
    type Error = ron::Error;
    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf
        unit unit_struct newtype_struct seq tuple tuple_struct map struct enum identifier ignored_any
    }
    fn deserialize_any<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        match self.0 {
            Value::Map(map) => visitor.visit_map(serde::de::value::MapDeserializer::new(
                map.into_iter()
                    .map(|(key, value)| (SettingsValue(key), SettingsValue(value))),
            )),
            Value::Seq(seq) => visitor.visit_seq(serde::de::value::SeqDeserializer::new(
                seq.into_iter().map(SettingsValue),
            )),
            Value::Option(Some(inner)) => visitor.visit_some(SettingsValue(*inner)),
            value => value.deserialize_any(visitor),
        }
    }
    fn deserialize_option<V: serde::de::Visitor<'de>>(
        self,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        match self.0 {
            Value::Unit | Value::Option(None) => visitor.visit_none(),
            Value::Option(Some(inner)) => visitor.visit_some(SettingsValue(*inner)),
            value => visitor.visit_some(SettingsValue(value)),
        }
    }
}

fn read_value(text: &str) -> Result<Value, String> {
    ron::from_str(text).map_err(|e| e.to_string())
}
fn write_value(path: &std::path::Path, value: &Value) -> Result<(), String> {
    let text = ron::ser::to_string_pretty(value, ron::ser::PrettyConfig::default())
        .map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(all(test, feature = "widgets"))]
mod persistence_tests {
    use super::*;

    fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
        match value {
            Value::Map(map) => map.get(&Value::from(key)),
            _ => None,
        }
    }

    fn path() -> PathBuf {
        frost::scratch::claim("nice-common-ron").join("settings.ron")
    }

    #[test]
    fn ron_round_trips_audio_routes_and_unset_values() {
        let path = path();
        let settings = Settings::<NoPreferences> {
            backend: Some("core-audio".into()),
            output_channels: Some(OutputChannels::Stereo([1, 2])),
            sample_rate: Some(48000.0),
            ..Default::default()
        };
        settings.save_to(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(ron::from_str::<Value>(&text).is_ok());
        let (restored, warning) = Settings::<NoPreferences>::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(restored, settings);
    }

    #[test]
    fn original_ron_option_spelling_preserves_routes_and_geometry() {
        let text = r#"{
            "backend": "core-audio", "input_device": (),
            "output_channels": {"kind":"stereo", "channels":[1,2]},
            "window": {"x":12,"y":(),"width":900.0,"height":620.0}
        }"#;
        let settings = Settings::<NoPreferences>::from_ron(text).unwrap();
        assert_eq!(settings.input_device, None);
        assert_eq!(
            settings.output_channels,
            Some(OutputChannels::Stereo([1, 2]))
        );
        let path = path();
        settings.save_to(&path).unwrap();
        Settings::<NoPreferences>::save_window_to(
            &path,
            WindowGeom {
                x: None,
                y: Some(24),
                width: 1000.0,
                height: 700.0,
            },
        )
        .unwrap();
        let (restored, warning) = Settings::<NoPreferences>::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(restored.output_channels, settings.output_channels);
        let window = restored.window.unwrap();
        assert_eq!(window.x, Some(12));
        assert_eq!(window.y, Some(24));
        assert_eq!(window.width, 1000.0);
    }

    #[test]
    fn old_json_file_is_ignored_when_ron_is_absent() {
        let path = path();
        let legacy = path.with_extension("json");
        let text = r#"{"backend":"core-audio","gate_db":-40}"#;
        std::fs::write(&legacy, text).unwrap();
        let (settings, warning) = Settings::<NoPreferences>::load_from(&path);
        assert_eq!(settings, Settings::default());
        assert_eq!(warning, None);
        Settings::<NoPreferences>::save_window_to(
            &path,
            WindowGeom {
                x: None,
                y: None,
                width: 900.0,
                height: 620.0,
            },
        )
        .unwrap();
        let value = read_value(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(field(&value, "backend").is_none());
        assert!(field(&value, "gate_db").is_none());
        assert_eq!(std::fs::read_to_string(legacy).unwrap(), text);
    }

    #[test]
    fn ron_geometry_save_preserves_unknown_fields() {
        let path = path();
        let text = r#"{"backend":"coreaudio","gate_db":-40,"future":{"enabled":true},"window":{"width":640,"height":400,"x":12,"extra":7}}"#;
        std::fs::write(&path, text).unwrap();
        let (settings, warning) = Settings::<NoPreferences>::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(settings.backend.as_deref(), Some("core-audio"));
        Settings::<NoPreferences>::save_window_to(
            &path,
            WindowGeom {
                x: None,
                y: None,
                width: 900.0,
                height: 620.0,
            },
        )
        .unwrap();
        let value = read_value(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            field(field(&value, "future").unwrap(), "enabled"),
            Some(&Value::Bool(true))
        );
        assert_eq!(field(&value, "gate_db"), Some(&Value::from(-40i8)));
        assert_eq!(
            field(field(&value, "window").unwrap(), "extra"),
            Some(&Value::from(7u8))
        );
        assert_eq!(
            field(field(&value, "window").unwrap(), "x"),
            Some(&Value::from(12u8))
        );
        let before = std::fs::read_to_string(&path).unwrap();
        Settings::<NoPreferences>::save_window_to(
            &path,
            WindowGeom {
                x: None,
                y: None,
                width: 900.0,
                height: 620.0,
            },
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        // A resize must not overwrite a malformed RON file.
        std::fs::write(&path, "broken (").unwrap();
        assert!(Settings::<NoPreferences>::load_from(&path).1.is_some());
        assert!(
            Settings::<NoPreferences>::save_window_to(
                &path,
                WindowGeom {
                    x: None,
                    y: None,
                    width: 900.0,
                    height: 620.0
                }
            )
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken (");
    }
}
