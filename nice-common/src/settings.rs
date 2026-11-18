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
    /// ASIO only: the first channel (1-based, as this file spells them) of a
    /// second output group that carries a copy of the first one. The group is as
    /// wide as the route needs, so `Some(3)` with a route reaching channel 2
    /// copies 1–2 onto 3–4. Absent means one output group.
    ///
    /// Whose channels those are is `asio_second_device`: absent, the group is on
    /// the output driver, one clock and a bit-for-bit copy; named, the copy rides
    /// a second driver at that driver's own period, with the two clocks of two
    /// interfaces slowly walking apart.
    pub asio_second_output: Option<u32>,
    /// ASIO only: the driver carrying the copy named by `asio_second_output`.
    /// Absent — or saved as the same name as `output_device`, which means the same
    /// thing and is read back as absent — the copy stays on the output driver's own
    /// channels. Any other name is a second driver, opened beside the first and
    /// fed a copy of the same signal.
    pub asio_second_device: Option<String>,
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
            && self.asio_second_output.is_none()
            && self.asio_second_device.is_none()
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
        atomic_write(path, text.as_bytes())
    }

    /// Update only window geometry, preserving saved preferences and unknown
    /// fields. Resizing must never commit the settings panel's draft.
    pub fn save_window(window: WindowGeom) -> Result<(), String> {
        Self::save_window_to(&Self::path(), window)
    }

    /// Update geometry at an explicit path without overwriting preferences.
    pub fn save_window_to(path: &std::path::Path, window: WindowGeom) -> Result<(), String> {
        Self::save_window_to_with(path, window, atomic_write)
    }

    fn save_window_to_with(
        path: &std::path::Path,
        window: WindowGeom,
        write: impl FnOnce(&std::path::Path, &[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
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
            let text = ron::ser::to_string_pretty(&value, ron::ser::PrettyConfig::default())
                .map_err(|e| e.to_string())?;
            write(path, text.as_bytes())?;
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
/// Stage in the destination directory so rename stays on one filesystem.
/// A successful replacement exposes the complete new file; no delete-first
/// fallback is permitted. This is not a power-loss durability guarantee for
/// the directory entry, nor a lock against concurrent read/modify/write saves.
fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    atomic_write_with(
        path,
        |file| {
            file.write_all(bytes)?;
            file.sync_all()
        },
        |from, to| std::fs::rename(from, to),
    )
    .map_err(|error| format!("{}: {error}", path.display()))
}

fn atomic_write_with(
    path: &std::path::Path,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
    replace: impl FnOnce(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Staged(PathBuf);
    impl Drop for Staged {
        fn drop(&mut self) {
            // Also clean up on write/sync/rename failure or unwinding. The
            // handle is closed first, including on Windows.
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    if path.file_name().is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Missing settings filename",
        ));
    }
    // create_new never truncates someone else's temporary file. The bounded
    // retries also tolerate files left by an abruptly terminated process.
    for _ in 0..128 {
        let temporary = parent.join(format!(
            ".nice-settings-{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = match options.open(&temporary) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let staged = Staged(temporary);
        let mut file = file;
        // Declare the handle after the guard so unwinding closes it first.
        // Normal error handling closes explicitly before the guard runs.
        let result = write(&mut file);
        drop(file);
        result?;
        return replace(&staged.0, path);
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "Could not reserve a settings temporary file",
    ))
}

#[cfg(all(test, feature = "widgets"))]
mod persistence_tests {
    use super::*;

    fn only_destination_remains(path: &std::path::Path) {
        let entries: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(entries, [path]);
    }

    #[test]
    fn atomic_save_replaces_an_existing_file() {
        let path = path();
        let original = Settings::<NoPreferences>::default();
        original.save_to(&path).unwrap();
        let updated = Settings::<NoPreferences> {
            backend: Some("dummy".into()),
            sample_rate: Some(96_000.0),
            ..original
        };
        updated.save_to(&path).unwrap();
        assert_eq!(Settings::<NoPreferences>::load_from(&path), (updated, None));
        only_destination_remains(&path);
    }

    #[test]
    fn a_partial_write_failure_preserves_the_previous_file() {
        use std::io::Write;
        let path = path();
        Settings::<NoPreferences>::default().save_to(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        let error = atomic_write_with(
            &path,
            |file| {
                file.write_all(b"incomplete")?;
                Err(std::io::Error::other("injected write failure"))
            },
            |_, _| panic!("must not replace after a failed write"),
        );
        assert!(error.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        only_destination_remains(&path);
    }

    #[test]
    fn a_failed_geometry_replacement_preserves_preferences_and_geometry() {
        use std::io::Write;
        let path = path();
        Settings::<NoPreferences>::default().save_to(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        let error = Settings::<NoPreferences>::save_window_to_with(
            &path,
            WindowGeom {
                width: 900.0,
                height: 620.0,
                x: Some(10),
                y: Some(20),
            },
            |path, bytes| {
                atomic_write_with(
                    path,
                    |file| {
                        file.write_all(bytes)?;
                        file.sync_all()
                    },
                    |staged, destination| {
                        // Complete new RON exists, but the destination has not changed.
                        assert!(read_value(&std::fs::read_to_string(staged)?).is_ok());
                        assert_eq!(std::fs::read(destination)?, before);
                        Err(std::io::Error::other("injected replacement failure"))
                    },
                )
                .map_err(|e| e.to_string())
            },
        );
        assert!(error.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        only_destination_remains(&path);
    }

    #[test]
    fn missing_parent_and_real_replacement_failure_leave_no_temporary_file() {
        let root = path();
        let missing = root.join("missing").join("settings.ron");
        assert!(
            Settings::<NoPreferences>::default()
                .save_to(&missing)
                .is_err()
        );
        assert!(!root.exists());
        // Renaming a file over a directory fails on every supported platform.
        std::fs::create_dir(&root).unwrap();
        assert!(atomic_write(&root, b"new settings").is_err());
        assert!(root.is_dir());
        only_destination_remains(&root);
    }

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

    /// A second output group is a decision, so it survives a save, and a file
    /// written before the row existed still loads as "one output group".
    #[test]
    fn a_second_output_group_round_trips_and_absent_means_one_group() {
        let path = frost::scratch::claim("nice-common-second-output").join("settings.ron");
        let settings = Settings::<NoPreferences> {
            backend: Some("asio".into()),
            output_channels: Some(OutputChannels::Stereo([1, 2])),
            asio_second_output: Some(3),
            ..Default::default()
        };
        settings.save_to(&path).unwrap();
        let (restored, warning) = Settings::<NoPreferences>::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(restored, settings);
        assert!(!restored.is_blank(), "a chosen group read as a cold start");

        let before = Settings::<NoPreferences>::from_ron(
            r#"{ "backend": "asio", "output_channels": {"kind":"stereo", "channels":[1,2]} }"#,
        )
        .unwrap();
        assert_eq!(before.asio_second_output, None);
        assert_eq!(
            Settings::<NoPreferences>::from_ron(r#"{ "asio_second_output": 5 }"#)
                .unwrap()
                .asio_second_output,
            Some(5)
        );
    }

    /// The driver carrying a copy is a decision too, so it survives a save; a file
    /// written before the row existed, and one where the row was never touched,
    /// both load as "the copy stays on the output driver".
    #[test]
    fn a_second_asio_device_round_trips_and_absent_means_the_output_driver() {
        let path = frost::scratch::claim("nice-common-second-device").join("settings.ron");
        let settings = Settings::<NoPreferences> {
            backend: Some("asio".into()),
            output_device: Some("Komplete Audio 6".into()),
            output_channels: Some(OutputChannels::Stereo([1, 2])),
            asio_second_output: Some(3),
            asio_second_device: Some("ReaRoute 3".into()),
            ..Default::default()
        };
        settings.save_to(&path).unwrap();
        let (restored, warning) = Settings::<NoPreferences>::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(restored, settings);
        assert!(
            !restored.is_blank(),
            "a chosen second driver read as a cold start"
        );

        let before = Settings::<NoPreferences>::from_ron(
            r#"{ "backend": "asio", "output_device": "Komplete Audio 6" }"#,
        )
        .unwrap();
        assert_eq!(before.asio_second_device, None);
        // A driver named on its own is not a blank file either: it is half a
        // decision the panel has to show, not one it gets to lose.
        let alone =
            Settings::<NoPreferences>::from_ron(r#"{ "asio_second_device": "ReaRoute 3" }"#)
                .unwrap();
        assert_eq!(alone.asio_second_device.as_deref(), Some("ReaRoute 3"));
        assert!(!alone.is_blank());
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

    /// A cold start is defaults and silence. `load_from` answers a path that
    /// does not exist with no warning, in its `Err(_) if !path.exists()` arm,
    /// and nothing else here checks that: the other tests in this module all
    /// write their file first, and the other half of the rule — a file that
    /// exists but will not parse, which DOES warn — is the last thing
    /// `ron_geometry_save_preserves_unknown_fields` does. Get this branch wrong
    /// and every first launch of every plugin shows "ignoring unreadable …" at
    /// someone who has never had a settings file.
    ///
    /// Own scratch claim, not `path()`: these tests share a process and run on
    /// several threads, so a test that asks whether a file is absent cannot
    /// stand in the same folder as three that write one.
    #[test]
    fn a_cold_start_is_defaults_and_silence() {
        let path = frost::scratch::claim("nice-common-cold-start").join("never-written.ron");
        let (settings, warning) = Settings::<NoPreferences>::load_from(&path);
        assert_eq!(settings, Settings::default());
        assert_eq!(warning, None);
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
