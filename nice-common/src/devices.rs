//! cpal-backed enumeration for the standalone settings page.
//!
//! Compiled into the standalone build only: a plugin inside a host must not
//! enumerate audio systems it does not own. Everything here runs on the GUI
//! thread, on demand (page entry, backend change) — never on the audio
//! thread, and never while merely displaying the tuner.
//!
//! Caveats the UI inherits and states honestly:
//! - ASIO enumeration *loads* each registered driver to read its properties;
//!   with several drivers installed, opening the page can pause for a moment.
//! - Device lists are cpal's device names; for ASIO that is the driver name
//!   (ports are not addressable through cpal, cf. README).

use cpal::traits::{DeviceTrait, HostTrait};

#[cfg(target_os = "macos")]
mod core_audio;
#[cfg(target_os = "linux")]
pub mod jack;

/// A selectable standalone backend key and its display label.
#[derive(Debug, Clone)]
pub struct BackendOption {
    pub id: String,
    pub label: String,
}

fn host_id_value(id: cpal::HostId) -> Option<(&'static str, &'static str)> {
    // Only hosts the application's standalone audio path supports.
    // Each variant exists only where cpal builds it (platform/feature cfg);
    // if-chains keep that exhaustiveness-free.
    // cpal builds the Asio variant only on Windows, feature or not.
    #[cfg(all(windows, feature = "asio-host"))]
    if id == cpal::HostId::Asio {
        return Some(("asio", "ASIO"));
    }
    #[cfg(windows)]
    if id == cpal::HostId::Wasapi {
        return Some(("wasapi", "Windows WASAPI"));
    }
    #[cfg(target_os = "macos")]
    if id == cpal::HostId::CoreAudio {
        // Key must match nice-plug's clap ValueEnum spelling (`core-audio`,
        // kebab-case) — a cpal-style `coreaudio` here poisons the settings
        // file and the next startup dies at argv parsing, never reaching the
        // device. Reads tolerate both spellings (see settings migration).
        return Some(("core-audio", "macOS Core Audio"));
    }
    #[cfg(target_os = "linux")]
    if id == cpal::HostId::Jack {
        return Some(("jack", "Linux JACK"));
    }
    #[cfg(target_os = "linux")]
    if id == cpal::HostId::Alsa {
        return Some(("alsa", "Linux ALSA"));
    }
    let _ = id;
    None
}

fn host_id_from_value(value: &str) -> Option<cpal::HostId> {
    #[cfg(all(windows, feature = "asio-host"))]
    if value == "asio" {
        return Some(cpal::HostId::Asio);
    }
    #[cfg(windows)]
    if value == "wasapi" {
        return Some(cpal::HostId::Wasapi);
    }
    #[cfg(target_os = "macos")]
    if value == "coreaudio" || value == "core-audio" {
        return Some(cpal::HostId::CoreAudio);
    }
    #[cfg(target_os = "linux")]
    if value == "jack" {
        return Some(cpal::HostId::Jack);
    }
    #[cfg(target_os = "linux")]
    if value == "alsa" {
        return Some(cpal::HostId::Alsa);
    }
    let _ = value;
    None
}

/// Every backend the running binary can offer, system default first.
/// JACK is Linux-only and remains selectable without a running server.
/// ASIO appears in a Windows standalone build, which brings `asio-host` with it.
pub fn available_backends() -> Vec<BackendOption> {
    let mk = |id: &str, label: &str| BackendOption {
        id: id.to_owned(),
        label: label.to_owned(),
    };
    let mut v = vec![mk("auto", "System default")];
    for h in cpal::available_hosts() {
        if let Some((id, label)) = host_id_value(h) {
            v.push(mk(id, label));
        }
    }
    v.push(mk("dummy", "no audio (dummy)"));
    v
}

pub fn open_host(backend: &str) -> Option<cpal::Host> {
    if backend == "auto" || backend.is_empty() {
        return Some(cpal::default_host());
    }
    cpal::host_from_id(host_id_from_value(backend)?).ok()
}

/// ASIO driver names, read from the registry.
///
/// Loading a driver is the only way to learn what it can do, and cpal's
/// enumeration loads EVERY registered driver to build its list — a moment
/// each, per direction, per query, on a machine whose audio gear left a
/// dozen vendor drivers behind. Names alone need nothing at all, so the
/// settings dropdowns are populated from here and the capability question
/// is put to the single driver the user actually picks.
///
/// `azo` reads the names out of `HKLM\SOFTWARE\ASIO`. That is one key, in the
/// view this process sees: a driver registered per-user, or only in the
/// 32-bit view, would be missing here. Measured on this machine against the
/// SDK walk `asio-sys` used instead, both answer the same nine names in the
/// same order, and `HKCU\Software\ASIO` does not exist at all.
#[cfg(all(windows, feature = "asio-host"))]
fn asio_driver_names() -> Vec<String> {
    let mut names = azo::driver::Metadata::enumerate()
        .map(|drivers| {
            drivers
                .into_iter()
                .map(|driver| driver.description.to_string_lossy())
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();
    names.sort();
    names.dedup();
    names
}

#[cfg(not(all(windows, feature = "asio-host")))]
fn asio_driver_names() -> Vec<String> {
    Vec::new()
}

/// What one ASIO driver answers about itself, gathered in a single load.
///
/// ASIO carries no portable description of a driver: `ASIOGetChannels`,
/// `ASIOGetSampleRate`, `ASIOGetBufferSize` and `ASIOCanSampleRate` are the
/// only answers there are, and each of them needs the driver loaded. cpal
/// asks all four of EVERY registered driver to build its device list; we ask
/// the one driver the user picked. `None` is the honest answer everywhere it
/// applies — a driver whose hardware is switched off, and any question asked
/// while this process is streaming, where the answer is not worth what the
/// load would cost. See [`asio_stream_live`].
#[derive(Clone)]
#[cfg_attr(not(all(windows, feature = "asio-host")), allow(dead_code))]
struct AsioCaps {
    inputs: u32,
    outputs: u32,
    /// The rate the driver is set to right now, which is what its own
    /// control panel shows. Some drivers report 0 until a stream runs.
    rate: Option<f32>,
    /// The sizes this driver will take, from `ASIOGetBufferSize`.
    ladder: Ladder,
    /// The size to seed a fresh setting with: the driver's own preference,
    /// snapped to a size it accepts. Measured answer here, through `azo`
    /// 0.6.1 on the Komplete Audio 6: 64 frames.
    buffer_pref: Option<u32>,
    /// The common ladder this driver answers `ASIOCanSampleRate` to,
    /// plus the rate it is currently on. cpal probes the same ladder
    /// internally to build its supported-config list, so the panel's
    /// rate options are the ones it offered before.
    rates: Vec<f32>,
}

/// What one driver will take as a buffer size.
///
/// `ASIOGetBufferSize` answers a minimum, a maximum and a granularity, and the
/// granularity is not decoration. Measured on this machine with `azo` 0.6.1 on
/// the Komplete Audio 6: min 8, max 2048, granularity `-1`, and of the sizes
/// frost offers, 24, 48, 96, 192, 384 and 768 are all refused with
/// `ASE_HWMALFUNCTION` while every power of two between is accepted. Filtering
/// on `min..=max` alone — which is all this code did before — therefore offered
/// exactly the six sizes that cannot be opened: the child's answer on this
/// driver was
/// `[8, 16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024, 2048]`, and
/// with the granularity honoured it is `[8, 16, 32, 64, 128, 256, 512, 1024,
/// 2048]`. Both lists measured on the same driver by the same command, either
/// side of this change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(all(windows, feature = "asio-host")), allow(dead_code))]
struct Ladder {
    min: u32,
    max: u32,
    steps: Steps,
}

/// How a driver fills the space between its minimum and maximum buffer size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(all(windows, feature = "asio-host")), allow(dead_code))]
enum Steps {
    /// Every frame count between [`Ladder::min`] and [`Ladder::max`].
    Any,
    /// `min`, `min + step`, `min + 2 * step`, …
    Linear(u32),
    /// `min`, `2 * min`, `4 * min`, … — the ASIO granularity of `-1`, which
    /// `azo` calls `Exponential` and documents as `min * 2^n`
    /// (`azo-0.6.1/src/dto.rs:76-82`).
    ///
    /// The ASIO text for `-1` is "limited to powers of two", which on a driver
    /// whose minimum is itself a power of two — every one measured here,
    /// minimum 8 — names the same sizes either way. On a driver that answered
    /// `-1` with a minimum of, say, 6, "doublings from the minimum" and "powers
    /// of two" would differ, and this machine has no such driver to ask. We
    /// follow the doublings, as `azo` does.
    Doubling,
}

#[cfg_attr(not(all(windows, feature = "asio-host")), allow(dead_code))]
impl Ladder {
    /// The answer for a driver that would not say.
    const UNKNOWN: Ladder = Ladder {
        min: 0,
        max: 0,
        steps: Steps::Any,
    };

    /// The smallest and largest sizes we may ever ask this driver for, if it
    /// named any.
    fn range(&self) -> Option<(u32, u32)> {
        (self.max > 0).then_some((self.min, self.max))
    }

    /// Whether this driver takes exactly this frame count.
    fn accepts(&self, frames: u32) -> bool {
        if self.max == 0 || frames < self.min || frames > self.max {
            return false;
        }
        match self.steps {
            // A driver that named only a range: `granularity == 0`, or a step
            // of `-2` or less that is not the `-1` meaning doublings. min..=max
            // is then all anyone can know. See the note on `azo`'s `BufferSize`
            // about how loosely this triple is answered in the wild.
            Steps::Any => true,
            // `max(1)` so a hand-built `Linear(0)` behaves as `Any` instead of
            // dividing by zero; a driver cannot produce one, `Granularity`
            // carries a `NonZeroI32`.
            Steps::Linear(step) => (frames - self.min).is_multiple_of(step.max(1)),
            Steps::Doubling => {
                let mut size = self.min;
                while size < frames {
                    // A doubling that stops growing (a minimum of zero) can
                    // never reach anything, rather than spinning forever.
                    match size.checked_mul(2).filter(|next| *next > size) {
                        Some(next) => size = next,
                        None => return false,
                    }
                }
                size == frames
            }
        }
    }

    /// The size to seed a fresh setting with, which has to be one of the sizes
    /// [`asio_buffer_options`] offers or the panel would preselect a choice
    /// nobody can make. Falls back to the driver's minimum when it reports no
    /// preference — the only number it has given us — and to `None` when
    /// nothing in our candidate ladder fits.
    fn snap(&self, preferred: Option<u32>) -> Option<u32> {
        let wanted = preferred.filter(|frames| *frames > 0).unwrap_or(self.min);
        BUFFER_CANDIDATES
            .into_iter()
            .find(|frames| *frames >= wanted && self.accepts(*frames))
            .or_else(|| {
                BUFFER_CANDIDATES
                    .into_iter()
                    .rev()
                    .find(|frames| self.accepts(*frames))
            })
    }
}

/// How many ASIO streams this process holds open right now.
///
/// ASIO permits one loaded driver per process, and `asio-sys` — which still
/// carries the stream — guards a second load with a handle stored **on the
/// `Asio` value** that performed the first one (`asio-sys` 0.4.0,
/// `bindings/mod.rs:445-458`). A capability query opened its own `Asio::new()`,
/// so that guard was empty, the load was granted, and the temporary `Driver`'s
/// `Drop` — `stop`, `dispose_buffers`, `ASIOExit`, `remove_current_driver`
/// (`:1035`, `:1017`, `:987-997`) — ran against the driver a running stream was
/// using. Live audio was therefore one capability query away from silence, with
/// nothing reported anywhere; measured on this machine as the tuner's waveform
/// dying the moment the settings page opened and never returning until restart
/// (`BUGS.md`).
///
/// The queries now go through `azo`, which has no such guard and no teardown to
/// trigger: a second instance of the same driver was measured opening beside the
/// first and answering the same questions, and `azo`'s handles carry no `Drop`
/// at all (`azo-0.6.1/src/driver.rs:157-172`, `:210-234` — the only `Drop` in the
/// crate is `COMInit`'s `CoUninitialize`, `src/utils.rs:68`), so letting a query
/// go releases one COM reference and sends the driver no `stop`, no
/// `dispose_buffers` and no `exit`. What remains unmeasured is the driver's own
/// reaction to that release, which is driver behaviour and only becomes visible
/// on a stream that was running. Until that has been watched, the rule stands on
/// both paths: asking is cheap, and losing the audio of a live rack is not.
static ASIO_STREAMS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Whether this process is streaming through an ASIO driver right now.
pub fn asio_stream_live() -> bool {
    ASIO_STREAMS.load(std::sync::atomic::Ordering::Acquire) > 0
}

/// Marks this process as the owner of the ASIO driver until it is dropped.
///
/// The audio path takes one before it asks cpal for a device and stores it in
/// the value that holds the streams, after them, so it is released only once
/// both are gone. While one is held, [`Snapshot`] answers ASIO capability
/// questions with unknowns; the settings page reads its ASIO answers from a
/// child process, which loads drivers in a process of its own.
#[derive(Debug)]
pub struct AsioStreamGuard;

/// Take an [`AsioStreamGuard`].
pub fn asio_stream_guard() -> AsioStreamGuard {
    ASIO_STREAMS.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    AsioStreamGuard
}

impl Drop for AsioStreamGuard {
    fn drop(&mut self) {
        ASIO_STREAMS.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// The rule in front of every capability load: a process that is streaming
/// does not ask, whatever the answer would have cost it to ask.
fn asio_caps_answered(name: &str, load: impl FnOnce(&str) -> Option<AsioCaps>) -> Option<AsioCaps> {
    if asio_stream_live() { None } else { load(name) }
}

#[cfg(all(windows, feature = "asio-host"))]
fn asio_caps(name: &str) -> Option<AsioCaps> {
    asio_caps_answered(name, load_asio_caps)
}

#[cfg(all(windows, feature = "asio-host"))]
fn load_asio_caps(name: &str) -> Option<AsioCaps> {
    use azo::driver::{Driver, Metadata, SafeHandle};
    use azo::dto::Granularity;

    // A saved setting names a driver by the string `asio_driver_names` gave,
    // which is this `description`. Measured on the nine drivers here, they are
    // the same strings the previous binding enumerated, so nothing saved stops
    // matching. Should one ever disagree, the answer is `None`: the panel shows
    // a driver without capabilities, not a guessed one.
    let clsid = Metadata::enumerate()
        .ok()?
        .into_iter()
        .find(|meta| meta.description.to_string_lossy() == name)
        .map(|meta| meta.clsid)?;
    // `SafeHandle::new` is what calls `CoInitializeEx`, on this thread, and it
    // pairs the matching `CoUninitialize` when the handle is dropped
    // (`azo-0.6.1/src/driver.rs:157-164`). A thread already initialized in the
    // other model therefore fails here, and `None` — no capabilities, no
    // invented numbers — is what the panel reads.
    let driver = SafeHandle::new(&clsid).ok()?;
    // `init` is `ASIOInit`. Every question below needs a driver that has been
    // initialized, and it reports failure as `false` rather than as an error.
    if !driver.init(None) {
        return None;
    }
    let count = |n: i32| u32::try_from(n).unwrap_or(0);
    let channels = driver.channel_counts().ok()?;
    let rate = driver
        .get_sample_rate()
        .ok()
        .filter(|rate| *rate > 0.0)
        .map(|rate| rate as f32);
    // Asked once: this driver's answer is the same before and after the rate
    // probes below, measured in one run of the same instance, and there is no
    // reason to ask a driver the same question twice per load.
    let sizes = driver.buffer_size().ok();
    let ladder = match sizes.as_ref() {
        Some(size) => Ladder {
            min: u32::try_from(size.min).unwrap_or(0),
            max: u32::try_from(size.max).unwrap_or(0),
            steps: match size.granularity {
                Some(Granularity::Exponential) => Steps::Doubling,
                Some(Granularity::Linear { step }) if step.get() > 0 => {
                    Steps::Linear(step.get() as u32)
                }
                // `None`, or a step that is not `-1` and not positive: the
                // driver said nothing usable, and min..=max is all we know —
                // which is all this code used before.
                _ => Steps::Any,
            },
        },
        None => Ladder::UNKNOWN,
    };
    let preferred = sizes
        .as_ref()
        .and_then(|size| u32::try_from(size.preferred).ok());
    let buffer_pref = ladder.snap(preferred);
    let mut rates: Vec<f32> = DEFAULT_RATES
        .into_iter()
        .filter(|rate| driver.can_sample_rate(f64::from(*rate)).is_ok())
        .collect();
    if let Some(rate) = rate.filter(|rate| !rates.contains(rate)) {
        rates.push(rate);
        rates.sort_by(f32::total_cmp);
    }
    Some(AsioCaps {
        inputs: count(channels.in_),
        outputs: count(channels.out),
        rate,
        ladder,
        buffer_pref,
        rates,
    })
}

#[cfg(not(all(windows, feature = "asio-host")))]
fn asio_caps(name: &str) -> Option<AsioCaps> {
    // Nothing to load here; the rule still runs through the same path.
    asio_caps_answered(name, |_| None)
}

/// Buffer sizes this driver permits, from its own min, max and granularity.
fn asio_buffer_options(ladder: Option<Ladder>) -> Vec<u32> {
    let Some(ladder) = ladder else {
        return Vec::new();
    };
    BUFFER_CANDIDATES
        .into_iter()
        .filter(|frames| ladder.accepts(*frames))
        .collect()
}

/// Answers gathered on demand, at most once per driver per pass.
///
/// ASIO permits one loaded driver per process, so what can be held between
/// questions is the ANSWER, never the handle — and the scope is one
/// `snapshot`, because a driver's current rate is its own control panel's to
/// change. A value carried in from an earlier pass would be a claim about now,
/// made from then. Failures are held too: a driver whose hardware is off is
/// asked once per pass, not once per question.
#[derive(Default)]
struct AsioCapsCache {
    entries: Vec<(String, Option<AsioCaps>)>,
}

impl AsioCapsCache {
    fn caps(&mut self, name: Option<&str>) -> Option<AsioCaps> {
        let name = name?;
        if let Some((_, held)) = self.entries.iter().find(|(held, _)| held == name) {
            return held.clone();
        }
        let loaded = asio_caps(name);
        self.entries.push((name.to_owned(), loaded.clone()));
        loaded
    }
}

/// The rate the picked input driver is on, falling back to the output one.
fn asio_rate(caps: &mut AsioCapsCache, input: Option<&str>, output: Option<&str>) -> Option<f32> {
    let rate = caps.caps(input).and_then(|caps| caps.rate);
    rate.or_else(|| caps.caps(output).and_then(|caps| caps.rate))
}

/// The picked output driver's own rate, without falling back to input.
fn asio_output_rate(caps: &mut AsioCapsCache, output: Option<&str>) -> Option<f32> {
    caps.caps(output).and_then(|caps| caps.rate)
}

/// The picked input driver's input width; zero stays unknown.
fn asio_input_channels(caps: &mut AsioCapsCache, input: Option<&str>) -> Option<u32> {
    caps.caps(input)
        .map(|caps| caps.inputs)
        .filter(|&inputs| inputs > 0)
}

fn device_names(
    devices: impl Iterator<Item = cpal::Device>,
    backend: &str,
    direction: cpal::DeviceDirection,
) -> Vec<String> {
    let mut v: Vec<String> = devices
        .filter_map(|d| {
            let description = d.description().ok()?;
            // JACK's capability queries report formats for both directions,
            // even on its direction-specific virtual devices.
            (backend != "jack" || description.direction() == direction)
                .then(|| description.name().to_owned())
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

pub fn list_input_devices(backend: &str) -> Vec<String> {
    #[cfg(target_os = "linux")]
    if backend == "jack" {
        return jack::snapshot().inputs;
    }
    if backend == "asio" {
        return asio_driver_names();
    }
    open_host(backend)
        .and_then(|h| h.input_devices().ok())
        .map(|devices| device_names(devices, backend, cpal::DeviceDirection::Input))
        .unwrap_or_default()
}

pub fn list_output_devices(backend: &str) -> Vec<String> {
    #[cfg(target_os = "linux")]
    if backend == "jack" {
        return jack::snapshot().outputs;
    }
    if backend == "asio" {
        return asio_driver_names();
    }
    open_host(backend)
        .and_then(|h| h.output_devices().ok())
        .map(|devices| device_names(devices, backend, cpal::DeviceDirection::Output))
        .unwrap_or_default()
}

/// The system-default input/output device NAMES under this backend
/// ("auto"/empty = the platform's default host). The settings page must
/// name what "(default)" actually is — a dropdown row saying the word is
/// a row saying nothing.
pub fn default_device_names(backend: &str) -> (Option<String>, Option<String>) {
    #[cfg(target_os = "linux")]
    if backend == "jack" {
        let snapshot = jack::snapshot();
        return (snapshot.def_in, snapshot.def_out);
    }
    if backend == "asio" {
        // ASIO has no system default. cpal's convention is to nominate the
        // FIRST registered driver, and the only way to know whether that one
        // can serve a direction is to load it — and the next, and the next.
        // So nothing is nominated: the dropdown lists every registered name
        // at no cost, and everything below stays unknown until the user names
        // a driver. A cold start under ASIO shows dashes and waits for a
        // choice, which is the honest reading of "no device chosen yet".
        return (None, None);
    }
    let host = if backend == "auto" || backend.is_empty() {
        cpal::default_host()
    } else {
        match open_host(backend) {
            Some(h) => h,
            None => return (None, None),
        }
    };
    let name = |d: Option<cpal::Device>| {
        d.and_then(|d| d.description().ok())
            .map(|x| x.name().to_owned())
    };
    (
        name(host.default_input_device()),
        name(host.default_output_device()),
    )
}

/// The sample rate the DRIVER is currently set to. For ASIO this is
/// `ASIOGetSampleRate` (what the Native Instruments control panel shows),
/// for CoreAudio the device's default. The panel's "(default)" resolves
/// through here at startup because nice-plug's own default is a hard-coded
/// 48 kHz — which silently overrides a driver set to anything else.
pub fn driver_default_rate(
    backend: &str,
    input: Option<&str>,
    output: Option<&str>,
) -> Option<f32> {
    #[cfg(target_os = "linux")]
    if backend == "jack" {
        return jack::snapshot().driver_rate;
    }
    if backend == "dummy" {
        return None;
    }
    if backend == "asio" {
        // ASIO's current rate belongs to one driver. There is no host-wide
        // rate to ask, and no reason to load a driver nobody chose.
        return asio_rate(&mut AsioCapsCache::default(), input, output);
    }
    let host = if backend == "auto" || backend.is_empty() {
        cpal::default_host()
    } else {
        open_host(backend)?
    };
    // A named device that cannot be found stays unknown. Falling back to the
    // system default here would report some other device's rate under the
    // name the user picked — and since ASIO's dropdown lists every registered
    // driver, a pick whose hardware is off is now an ordinary case, not an
    // impossible one.
    let in_dev = match input {
        Some(n) => host.input_devices().ok().and_then(|d| find_named(d, n)),
        None => host.default_input_device(),
    };
    if let Some(rate) = in_dev
        .as_ref()
        .and_then(|d| d.default_input_config().ok())
        .map(|c| c.sample_rate() as f32)
    {
        return Some(rate);
    }
    let out_dev = match output {
        Some(n) => host.output_devices().ok().and_then(|d| find_named(d, n)),
        None => host.default_output_device(),
    };
    out_dev
        .as_ref()
        .and_then(|d| d.default_output_config().ok())
        .map(|c| c.sample_rate() as f32)
}

/// Query the selected output device's own default, without falling back to input.
fn output_default_rate(backend: &str, output: Option<&str>) -> Option<f32> {
    if backend == "asio" {
        return asio_output_rate(&mut AsioCapsCache::default(), output);
    }
    let host = if backend == "auto" || backend.is_empty() {
        cpal::default_host()
    } else {
        open_host(backend)?
    };
    let device = match output {
        Some(name) => find_named(host.output_devices().ok()?, name),
        None => host.default_output_device(),
    }?;
    device
        .default_output_config()
        .ok()
        .map(|config| config.sample_rate() as f32)
}

fn output_rate_options(formats: &[StreamFormat], default: Option<f32>) -> Vec<f32> {
    let mut rates = Vec::new();
    for format in formats {
        rates.extend([format.min_rate as f32, format.max_rate as f32]);
        rates.extend(
            DEFAULT_RATES
                .into_iter()
                .filter(|rate| *rate >= format.min_rate as f32 && *rate <= format.max_rate as f32),
        );
    }
    if let Some(rate) = default.filter(|rate| {
        formats
            .iter()
            .any(|f| *rate >= f.min_rate as f32 && *rate <= f.max_rate as f32)
    }) {
        rates.push(rate);
    }
    rates.sort_by(f32::total_cmp);
    rates.dedup();
    rates
}

fn find_named(mut devs: impl Iterator<Item = cpal::Device>, want: &str) -> Option<cpal::Device> {
    devs.find(|d| d.description().ok().is_some_and(|x| x.name() == want))
}

/// The selected input device's channel count. A missing device or failed
/// query remains unknown; it must not turn into a guessed eight-channel list.
fn input_channel_count(backend: &str, input: Option<&str>) -> Option<u32> {
    if backend == "asio" {
        // The one driver the user picked, asked directly. Nothing picked
        // means nothing loaded and no channel count known — the alternative
        // is loading drivers until one answers, which is the sweep this
        // whole path exists to avoid.
        return asio_input_channels(&mut AsioCapsCache::default(), input);
    }
    let host = if backend == "auto" || backend.is_empty() {
        cpal::default_host()
    } else {
        open_host(backend)?
    };
    let device = match input {
        Some(name) => find_named(host.input_devices().ok()?, name)?,
        None => host.default_input_device()?,
    };
    device
        .supported_input_configs()
        .ok()
        .into_iter()
        .flatten()
        .map(|config| u32::from(config.channels()))
        .filter(|&channels| channels > 0)
        .max()
        .or_else(|| {
            device
                .default_input_config()
                .ok()
                .map(|config| u32::from(config.channels()))
                .filter(|&channels| channels > 0)
        })
        // The tuner processes mono/stereo input; JACK advertises up to 64
        // virtual ports, which would otherwise offer unusable input choices.
        .map(|channels| {
            if backend == "jack" {
                channels.min(2)
            } else {
                channels
            }
        })
}

pub const JACK_BUFFER_CHOICES: [u32; 8] = [32, 48, 64, 96, 128, 256, 512, 1024];

const DEFAULT_RATES: [f32; 4] = [44100.0, 48000.0, 88200.0, 96000.0];
/// Every size offered when the device permits it — the ladder reaches from
/// the 8-frame floors real ASIO control panels use up to 4096.
const BUFFER_CANDIDATES: [u32; 16] = [
    8, 16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024, 2048, 4096,
];

/// Buffer candidates supported by at least one stream configuration.
fn buffer_options(configs: &[cpal::SupportedStreamConfigRange]) -> Vec<u32> {
    let ranges: Vec<_> = configs
        .iter()
        .filter_map(|c| match *c.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => Some((min, max)),
            cpal::SupportedBufferSize::Unknown => None,
        })
        .collect();
    if ranges.is_empty() {
        return Vec::new();
    }
    let mut options: Vec<_> = BUFFER_CANDIDATES
        .into_iter()
        .filter(|b| ranges.iter().any(|(min, max)| (min..=max).contains(&b)))
        .collect();
    for (min, max) in ranges {
        if !options.iter().any(|b| (min..=max).contains(b)) {
            options.push(min);
        }
    }
    options.sort_unstable();
    options.dedup();
    options
}

/// Query input and output capabilities independently. Only a shared
/// device-level buffer requires intersecting the two directions' options.
pub type StreamOptions = (
    Vec<f32>,
    Vec<u32>,
    Vec<u32>,
    Vec<StreamFormat>,
    Vec<StreamFormat>,
);

/// The stream shapes one ASIO direction can actually take: one entry per
/// channel count from one to the port count, per rate the driver agreed to.
///
/// `ASIOCreateBuffers` accepts fewer buffers than the hardware has ports, so
/// a six-input driver still opens a mono stream — which is what the tuner's
/// input always is. Listing the device only at its full width, as cpal's
/// per-rate/per-count expansion does not, makes every real configuration look
/// unsupported. Each entry pins one rate, as cpal does: a span between the
/// lowest and highest rate the driver likes would also claim the ones in
/// between, which the driver was never asked about.
fn asio_formats(rates: &[f32], buffer_range: Option<(u32, u32)>, ports: u32) -> Vec<StreamFormat> {
    rates
        .iter()
        .flat_map(|rate| {
            (1..=ports).map(move |channels| StreamFormat {
                channels,
                min_rate: *rate as u32,
                max_rate: *rate as u32,
                buffer_range,
            })
        })
        .collect()
}

/// The ASIO shape of `stream_options`: one load of the chosen driver, and
/// nothing at all while no driver is chosen. Rates come from
/// `ASIOCanSampleRate`, buffer sizes from `ASIOGetBufferSize`, widths from
/// `ASIOGetChannels` — the same three questions cpal puts to every registered
/// driver on the machine, put once to the one about to carry the audio.
fn asio_stream_options(
    caps: &mut AsioCapsCache,
    input: Option<&str>,
    output: Option<&str>,
) -> StreamOptions {
    let input_caps = caps.caps(input);
    let output_caps = caps.caps(output);
    let input_formats = input_caps
        .as_ref()
        .map(|caps| asio_formats(&caps.rates, caps.ladder.range(), caps.inputs))
        .unwrap_or_default();
    let output_formats = output_caps
        .as_ref()
        .map(|caps| asio_formats(&caps.rates, caps.ladder.range(), caps.outputs))
        .unwrap_or_default();
    (
        input_caps
            .as_ref()
            .or(output_caps.as_ref())
            .map(|caps| caps.rates.clone())
            .unwrap_or_default(),
        asio_buffer_options(input_caps.as_ref().map(|caps| caps.ladder)),
        asio_buffer_options(output_caps.as_ref().map(|caps| caps.ladder)),
        input_formats,
        output_formats,
    )
}

pub fn stream_options(backend: &str, input: Option<&str>, output: Option<&str>) -> StreamOptions {
    if backend == "asio" {
        return asio_stream_options(&mut AsioCapsCache::default(), input, output);
    }
    let host = if backend == "auto" || backend.is_empty() {
        Some(cpal::default_host())
    } else {
        open_host(backend)
    };
    let in_dev = host.as_ref().and_then(|host| match input {
        Some(name) => find_named(host.input_devices().ok()?, name),
        None => host.default_input_device(),
    });
    let out_dev = host.as_ref().and_then(|host| match output {
        Some(name) => find_named(host.output_devices().ok()?, name),
        None => host.default_output_device(),
    });
    let in_cfgs = in_dev
        .as_ref()
        .and_then(|d| d.supported_input_configs().ok())
        .map(|c| c.collect::<Vec<_>>())
        .unwrap_or_default();
    let out_cfgs = out_dev
        .as_ref()
        .and_then(|d| d.supported_output_configs().ok())
        .map(|c| c.collect::<Vec<_>>())
        .unwrap_or_default();
    let rate_src = if in_cfgs.is_empty() {
        &out_cfgs
    } else {
        &in_cfgs
    };
    let mut rates = Vec::new();
    for config in rate_src {
        let (lo, hi) = (config.min_sample_rate(), config.max_sample_rate());
        if lo == hi {
            rates.push(lo as f32);
        } else {
            rates.extend(
                DEFAULT_RATES
                    .into_iter()
                    .filter(|r| *r >= lo as f32 && *r <= hi as f32),
            );
        }
    }
    rates.sort_by(f32::total_cmp);
    rates.dedup();

    (
        rates,
        buffer_options(&in_cfgs),
        buffer_options(&out_cfgs),
        in_cfgs.iter().map(StreamFormat::from).collect(),
        out_cfgs.iter().map(StreamFormat::from).collect(),
    )
}

/// A supported combination, retained so Apply can validate whole stream
/// configurations instead of treating independent dropdown lists as proof.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StreamFormat {
    pub channels: u32,
    pub min_rate: u32,
    pub max_rate: u32,
    pub buffer_range: Option<(u32, u32)>,
}

impl From<&cpal::SupportedStreamConfigRange> for StreamFormat {
    fn from(config: &cpal::SupportedStreamConfigRange) -> Self {
        Self {
            channels: u32::from(config.channels()),
            min_rate: config.min_sample_rate(),
            max_rate: config.max_sample_rate(),
            buffer_range: match *config.buffer_size() {
                cpal::SupportedBufferSize::Range { min, max } => Some((min, max)),
                cpal::SupportedBufferSize::Unknown => None,
            },
        }
    }
}

impl StreamFormat {
    fn supports(&self, rate: f32, buffer: u32) -> bool {
        rate.is_finite()
            && rate.fract() == 0.0
            && rate >= self.min_rate as f32
            && rate <= self.max_rate as f32
            && buffer > 0
            && self
                .buffer_range
                .is_none_or(|(min, max)| (min..=max).contains(&buffer))
    }
}

/// Mono channels and disjoint stereo pairs are physical routes on the
/// selected device, not guesses about its default stream width.
fn output_routes(
    count: u32,
    default: Option<crate::settings::OutputChannels>,
) -> Vec<crate::settings::OutputChannels> {
    use crate::settings::OutputChannels;
    let mut routes: Vec<_> = (1..=count).map(OutputChannels::Mono).collect();
    routes.extend(
        (1..count)
            .step_by(2)
            .map(|left| OutputChannels::Stereo([left, left + 1])),
    );
    if let Some(default) = default.filter(|route| route.fits(count))
        && !routes.contains(&default)
    {
        routes.push(default);
    }
    routes
}

/// Everything the settings page can learn about a backend in one pass.
/// Built in-process — or, when that is provably blind (ASIO, below), in
/// a throwaway child process and shipped back as RON.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub def_in: Option<String>,
    pub def_out: Option<String>,
    pub rates: Vec<f32>,
    #[serde(default)]
    pub output_rates: Vec<f32>,
    #[serde(default)]
    pub default_output_rate: Option<f32>,
    /// Input buffer options (legacy field name in the child protocol).
    pub buffers: Vec<u32>,
    pub output_buffers: Vec<u32>,
    pub buffers_linked: bool,
    /// One driver owns both directions, so its single sample rate clocks the
    /// output stream whatever a settings file once said.
    #[serde(default)]
    pub rates_linked: bool,
    pub output_buffer_pref: Option<u32>,
    pub driver_rate: Option<f32>,
    pub input_formats: Vec<StreamFormat>,
    pub output_formats: Vec<StreamFormat>,
    pub output_channel_options: Vec<crate::settings::OutputChannels>,
    pub default_output_channels: Option<crate::settings::OutputChannels>,
    /// The size to start someone who has never chosen with: the driver's own
    /// `ASIOGetBufferSize` preference (the manufacturer panel's number), snapped
    /// to a size the driver accepts. 64 frames on the driver measured here, and
    /// the same 64 through the previous binding on the same path — this field is
    /// not where the ladder changes anything, the list above it is.
    ///
    /// One footnote kept on purpose: a standalone `asio-sys` probe, asking this
    /// same driver the same question in a different order, got `Only(2048)` for
    /// its preference. Whether that answer depends on what was asked first is
    /// unmeasured — the experiment to find out was stopped — and `azo`'s 64 is
    /// the number the manufacturer's own panel shows and the driver accepts, so
    /// that is the one used. cpal discards the preference; nothing else here
    /// carries it to the panel.
    pub buffer_pref: Option<u32>,
    /// Number of channels reported by the selected input device.
    #[serde(default)]
    pub input_channels: Option<u32>,
    /// Preferred individual input channel, if the backend reports one.
    /// A default stream config's channel COUNT is not a channel selection.
    #[serde(default)]
    pub default_input_channel: Option<u32>,
}

impl Snapshot {
    /// Encode the standalone discovery helper's stdout protocol.
    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string(self)
    }

    /// Decode the same protocol in the parent process.
    pub fn from_ron(bytes: &[u8]) -> Result<Self, ron::error::SpannedError> {
        ron::de::from_bytes(bytes)
    }

    /// Resolve real device defaults and validate a complete duplex session.
    /// Resolve every stream field before handing it to the application audio owner.
    ///
    /// A question this snapshot cannot answer is not a prohibition. An ASIO
    /// driver answers nothing while this process is streaming from it (see the
    /// `AsioStreamGuard` rule above), and nothing at all when another process
    /// owns it; a settings page and an audio owner ask the same questions from
    /// different states, and the page may hold the answers while the owner does
    /// not. What the owner cannot check, the device checks when the stream is
    /// built — `audio_io::open` answers with `Input channel is outside the
    /// device layout` and `Output route is outside the device layout` — so an
    /// unknown here steps aside and a KNOWN answer still objects.
    pub fn resolve_settings<P: crate::Preferences>(
        &self,
        settings: &crate::settings::Settings<P>,
    ) -> Result<crate::settings::Settings<P>, String> {
        let mut resolved = settings.clone();
        let backend = resolved
            .backend
            .as_deref()
            .ok_or("Select an audio backend")?;
        if backend == "dummy" {
            // Explicitly choosing no audio is complete without device
            // selections and can also be applied in the same process.
            let silent = crate::settings::Settings {
                backend: Some("dummy".into()),
                preferences: resolved.preferences,
                window: resolved.window,
                ..Default::default()
            };
            return Ok(silent);
        }
        if !matches!(
            backend,
            "auto" | "core-audio" | "coreaudio" | "wasapi" | "asio" | "alsa"
        ) && !(cfg!(target_os = "linux") && backend == "jack")
        {
            return Err("This audio backend is not supported by the standalone application".into());
        }
        resolved.input_device = resolved.input_device.or_else(|| self.def_in.clone());
        resolved.output_device = resolved.output_device.or_else(|| self.def_out.clone());
        if !resolved
            .input_device
            .as_ref()
            .is_some_and(|name| self.inputs.contains(name))
        {
            return Err("Select an available input device".into());
        }
        if !resolved
            .output_device
            .as_ref()
            .is_some_and(|name| self.outputs.contains(name))
        {
            return Err("Select an available output device".into());
        }
        let channel = resolved
            .input_channel
            .or(self.default_input_channel)
            // A driver that reports no width is not a driver with no inputs:
            // the count is unknown, and only a count that is KNOWN to be
            // smaller than the pick can object to it.
            .filter(|&ch| ch > 0 && self.input_channels.is_none_or(|count| ch <= count))
            .ok_or("Select an input channel")?;
        let output_route = resolved
            .output_channels
            .or(self.default_output_channels)
            // Same rule for the route: an empty list is no answer, so the route
            // a device did not describe is left for the device to refuse.
            .filter(|route| {
                self.output_channel_options.is_empty()
                    || self.output_channel_options.contains(route)
            })
            .ok_or("Select an output channel configuration")?;
        if cfg!(target_os = "linux")
            && backend == "jack"
            && let Some(right) = &resolved.jack_output_right
            && (!self.outputs.contains(right) || Some(right) == resolved.output_device.as_ref())
        {
            return Err(
                "Select a different available JACK destination for the right output".into(),
            );
        }
        let rate = resolved
            .sample_rate
            .or(self.driver_rate)
            .ok_or("Select a sample rate")?;
        let output_rate = if self.rates_linked {
            rate
        } else {
            resolved
                .output_sample_rate
                .or(self.default_output_rate)
                .ok_or("Select an output sample rate")?
        };
        if resolved.input_device == resolved.output_device && output_rate != rate {
            return Err(
                "The same audio device requires matching input and output sample rates".into(),
            );
        }
        let input_buffer = resolved
            .period_size
            .or(self.buffer_pref)
            .ok_or("Select an input buffer size")?;
        let output_buffer = if self.buffers_linked {
            input_buffer
        } else {
            resolved
                .output_period_size
                .or(self.output_buffer_pref)
                .ok_or("Select an output buffer size")?
        };
        let jack = cfg!(target_os = "linux") && backend == "jack";
        if jack
            && !JACK_BUFFER_CHOICES.contains(&input_buffer)
            && Some(input_buffer) != self.buffer_pref
        {
            return Err("Select a JACK buffer size from the offered choices".into());
        }
        let supports = |format: &StreamFormat, rate, buffer| {
            if jack {
                // JACK reports only its current quantum, not supported alternatives.
                let mut format = format.clone();
                format.buffer_range = None;
                format.supports(rate, buffer)
            } else {
                format.supports(rate, buffer)
            }
        };
        // A device that listed no formats listed nothing, which is not a device
        // that can do none of them; see the rule on this function. Only a list
        // this device actually wrote can contradict the request.
        if !self.input_formats.is_empty()
            && ![2, 1].into_iter().any(|channels| {
                channel <= channels
                    && self.input_formats.iter().any(|format| {
                        format.channels == channels && supports(format, rate, input_buffer)
                    })
            })
        {
            return Err(
                "The input channel, rate and buffer size are not a supported stream configuration"
                    .into(),
            );
        }
        if !self.output_formats.is_empty()
            && !self.output_formats.iter().any(|format| {
                format.channels >= output_route.required_channels()
                    && supports(format, output_rate, output_buffer)
            })
        {
            return Err(
                "The output device does not support these channels, sample rate and buffer size"
                    .into(),
            );
        }
        resolved.input_channel = Some(channel);
        resolved.output_channels = Some(output_route);
        resolved.sample_rate = Some(rate);
        resolved.output_sample_rate = Some(output_rate);
        resolved.period_size = Some(input_buffer);
        resolved.output_period_size = Some(output_buffer);
        Ok(resolved)
    }
}

/// In-process snapshot: every enumeration call, once.
pub fn snapshot(backend: &str, input: Option<&str>, output: Option<&str>) -> Snapshot {
    #[cfg(target_os = "linux")]
    if backend == "jack" {
        return jack::snapshot();
    }
    // One pass, one load per ASIO driver: every question below is answered
    // from a single gathering, so picking a driver loads it once. Other
    // backends never touch this.
    let asio = backend == "asio";
    let mut caps = AsioCapsCache::default();
    let (def_in, def_out) = default_device_names(backend);
    let (rates, mut buffers, mut output_buffers, input_formats, output_formats) = if asio {
        asio_stream_options(&mut caps, input, output)
    } else {
        stream_options(backend, input, output)
    };
    let buffers_linked = crate::settings::buffers_must_match(
        backend,
        input.or(def_in.as_deref()),
        output.or(def_out.as_deref()),
    );
    let rates_linked = crate::settings::rates_must_match(
        backend,
        input.or(def_in.as_deref()),
        output.or(def_out.as_deref()),
    );
    let driver_rate = if asio {
        asio_rate(&mut caps, input, output)
    } else {
        driver_default_rate(backend, input, output)
    };
    let default_output_rate = if asio {
        asio_output_rate(&mut caps, output)
    } else {
        output_default_rate(backend, output)
    };
    let output_rates = output_rate_options(&output_formats, default_output_rate);
    let input_channels = if asio {
        asio_input_channels(&mut caps, input)
    } else {
        input_channel_count(backend, input)
    };
    // Preferred buffer: a property of the input device (the one whose
    // clock runs the show), or of whatever the default input is.
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut buffer_pref = asio
        .then(|| input.map(str::to_owned).or_else(|| def_in.clone()))
        .flatten()
        .and_then(|name| caps.caps(Some(&name)).and_then(|caps| caps.buffer_pref));
    let mut output_buffer_pref = if buffers_linked { buffer_pref } else { None };
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut default_input_channel = None;
    let mut default_output_channels: Option<crate::settings::OutputChannels> = None;
    #[cfg(target_os = "macos")]
    if matches!(backend, "core-audio" | "coreaudio") {
        if let Some(name) = input.or(def_in.as_deref()) {
            let defaults = core_audio::defaults(name);
            buffer_pref = defaults.buffer_frames;
            default_input_channel = defaults
                .input_channel
                .filter(|ch| input_channels.is_some_and(|count| *ch > 0 && *ch <= count));
        }
        if let Some(name) = output.or(def_out.as_deref()) {
            let defaults = core_audio::defaults(name);
            output_buffer_pref = defaults.buffer_frames;
            default_output_channels = defaults.output_channels.map(|[left, right]| {
                if left == right {
                    crate::settings::OutputChannels::Mono(left)
                } else {
                    crate::settings::OutputChannels::Stereo([left, right])
                }
            });
        }
    }
    for (options, current) in [
        (&mut buffers, buffer_pref),
        (&mut output_buffers, output_buffer_pref),
    ] {
        if let Some(frames) = current
            && !options.contains(&frames)
        {
            options.push(frames);
            options.sort_unstable();
        }
    }
    if buffers_linked {
        buffers.retain(|value| output_buffers.contains(value));
        output_buffers.clone_from(&buffers);
        output_buffer_pref = buffer_pref;
    }
    let output_count = output_formats
        .iter()
        .map(|format| format.channels)
        .max()
        .unwrap_or(0);
    default_output_channels = default_output_channels.filter(|route| route.fits(output_count));
    let output_channel_options = output_routes(output_count, default_output_channels);
    Snapshot {
        inputs: list_input_devices(backend),
        outputs: list_output_devices(backend),
        def_in,
        def_out,
        rates,
        output_rates,
        default_output_rate,
        buffers,
        output_buffers,
        buffers_linked,
        rates_linked,
        output_buffer_pref,
        input_formats,
        output_formats,
        output_channel_options,
        default_output_channels,
        driver_rate,
        buffer_pref,
        input_channels,
        default_input_channel,
    }
}

/// ASIO enumeration from THIS process is blind while a stream is open:
/// asio-sys — which still carries the stream — keeps ONE driver loaded
/// globally (the running stream's) and refuses every other
/// (`DriverAlreadyExists`, measured by loading a second driver while the first
/// was loaded), and destroying drivers
/// mid-enumeration would tear at the live one. The panel cannot close
/// the device while the application owns its live stream, and the user must not have
/// to restart. So: ask a CHILD process, which opens no stream at all
/// and therefore sees every registered driver. The parent's audio
/// never stops; nothing is closed; the child enumerates and dies.
/// Which binding the child asks through cannot matter to the parent, and the
/// child now asks `azo`.
pub fn snapshot_via_child(
    backend: &str,
    input: Option<&str>,
    output: Option<&str>,
) -> Option<Snapshot> {
    let mut cmd = std::process::Command::new(std::env::current_exe().ok()?);
    cmd.arg("--list-devices")
        .arg(backend)
        .arg(input.unwrap_or(""))
        .arg(output.unwrap_or(""));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        eprintln!(
            "nice-common: device-helper exited {:?}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
        return None;
    }
    match Snapshot::from_ron(&out.stdout) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!(
                "nice-common: device-helper output unreadable: {e}: {}",
                String::from_utf8_lossy(&out.stdout)
                    .chars()
                    .take(160)
                    .collect::<String>()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_protocol_round_trips_defaults_formats_and_routes() {
        let snapshot = Snapshot {
            inputs: vec!["Mic \"one\"".into()],
            outputs: vec!["Speakers".into()],
            def_in: Some("Mic \"one\"".into()),
            rates: vec![44100.0, 48000.0],
            input_channels: Some(1),
            default_input_channel: Some(1),
            buffers: vec![64, 128, 512],
            input_formats: vec![StreamFormat {
                channels: 1,
                min_rate: 44100,
                max_rate: 96000,
                buffer_range: Some((32, 1024)),
            }],
            output_formats: vec![StreamFormat {
                channels: 2,
                min_rate: 48000,
                max_rate: 48000,
                buffer_range: None,
            }],
            output_channel_options: vec![
                crate::OutputChannels::Mono(1),
                crate::OutputChannels::Stereo([1, 2]),
            ],
            default_output_channels: Some(crate::OutputChannels::Stereo([1, 2])),
            ..Default::default()
        };
        let text = snapshot.to_ron().unwrap();
        let restored = Snapshot::from_ron(text.as_bytes()).unwrap();
        assert_eq!(restored.to_ron().unwrap(), text);
        assert_eq!(
            restored.default_output_channels,
            snapshot.default_output_channels
        );
        assert!(Snapshot::from_ron(b"invalid (").is_err());
    }

    /// The ASIO rows are the registry's driver list, identical in both
    /// directions: a driver whose hardware is switched off stays listed and
    /// is refused when picked, rather than hidden or guessed about.
    #[cfg(all(windows, feature = "asio-host"))]
    #[test]
    fn asio_lists_registered_names_in_both_directions() {
        let inputs = list_input_devices("asio");
        assert_eq!(inputs, asio_driver_names());
        assert_eq!(inputs, list_output_devices("asio"));
        let mut sorted = inputs.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(inputs, sorted);
    }

    /// A name no driver answers to is an unknown, not zero channels.
    #[cfg(all(windows, feature = "asio-host"))]
    #[test]
    fn an_unregistered_asio_name_has_no_known_channels() {
        assert!(asio_caps("no such ASIO driver").is_none());
        assert_eq!(
            input_channel_count("asio", Some("no such ASIO driver")),
            None
        );
    }

    /// Nothing chosen means nothing loaded: no rates, no buffer sizes, no
    /// channel count, and nothing invented in their place.
    #[test]
    fn asio_knows_nothing_before_a_driver_is_chosen() {
        let (rates, buffers, output_buffers, inputs, outputs) =
            asio_stream_options(&mut AsioCapsCache::default(), None, None);
        assert!(rates.is_empty());
        assert!(buffers.is_empty());
        assert!(output_buffers.is_empty());
        assert!(inputs.is_empty() && outputs.is_empty());
        assert_eq!(input_channel_count("asio", None), None);
        assert_eq!(default_device_names("asio"), (None, None));
    }

    /// A driver that will not load is asked once per pass: the refusal is
    /// held, so a dead device costs one attempt, not one per dropdown row.
    #[cfg(all(windows, feature = "asio-host"))]
    #[test]
    fn the_cache_holds_a_refusal_so_a_pass_asks_once() {
        let mut caps = AsioCapsCache::default();
        assert!(caps.caps(Some("no such ASIO driver")).is_none());
        assert!(caps.caps(Some("no such ASIO driver")).is_none());
        assert_eq!(caps.entries.len(), 1, "the second ask was a second load");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_backends_include_jack_without_connecting_to_a_server() {
        let backends = available_backends();
        assert_eq!(backends.iter().filter(|b| b.id == "jack").count(), 1);
        assert!(backends.iter().any(|b| b.id == "alsa"));
        assert!(
            !backends
                .iter()
                .any(|b| b.id == "asio" || b.id == "core-audio")
        );
        assert_eq!(host_id_from_value("jack"), Some(cpal::HostId::Jack));
        assert_eq!(
            host_id_value(cpal::HostId::Jack),
            Some(("jack", "Linux JACK"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn jack_allows_trial_buffers_but_rejects_sample_rate_overrides() {
        let mut snapshot = microphone_snapshot();
        snapshot.buffers_linked = true;
        snapshot.buffer_pref = Some(192);
        snapshot.output_buffer_pref = Some(192);
        for format in snapshot
            .input_formats
            .iter_mut()
            .chain(&mut snapshot.output_formats)
        {
            format.min_rate = 96000;
            format.max_rate = 96000;
            format.buffer_range = Some((192, 192));
        }
        snapshot.driver_rate = Some(96000.0);
        snapshot.default_output_rate = Some(96000.0);
        let settings = crate::settings::Settings::<crate::settings::NoPreferences> {
            backend: Some("jack".into()),
            ..Default::default()
        };
        let resolved = snapshot.resolve_settings(&settings).unwrap();
        assert_eq!(resolved.sample_rate, Some(96000.0));
        assert_eq!(resolved.output_sample_rate, Some(96000.0));
        assert_eq!(resolved.period_size, Some(192));
        assert_eq!(resolved.output_period_size, Some(192));
        for frames in JACK_BUFFER_CHOICES {
            let resolved = snapshot
                .resolve_settings(
                    &crate::settings::Settings::<crate::settings::NoPreferences> {
                        period_size: Some(frames),
                        ..settings.clone()
                    },
                )
                .unwrap();
            assert_eq!(resolved.period_size, Some(frames));
            assert_eq!(resolved.output_period_size, Some(frames));
        }
        assert!(
            snapshot
                .resolve_settings(
                    &crate::settings::Settings::<crate::settings::NoPreferences> {
                        sample_rate: Some(48000.0),
                        ..settings
                    }
                )
                .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a running JACK server (or pw-jack)"]
    fn jack_server_defaults_resolve_without_manual_device_settings() {
        let snapshot = snapshot("jack", None, None);
        let resolved = snapshot
            .resolve_settings(
                &crate::settings::Settings::<crate::settings::NoPreferences> {
                    backend: Some("jack".into()),
                    ..Default::default()
                },
            )
            .expect("JACK server must expose usable input/output defaults");
        assert_eq!(resolved.sample_rate, resolved.output_sample_rate);
        assert_eq!(resolved.period_size, resolved.output_period_size);
        assert!(snapshot.buffers_linked);
        assert!(snapshot.inputs.iter().all(|name| name.contains(':')));
        assert!(snapshot.outputs.iter().all(|name| name.contains(':')));
        assert_eq!(snapshot.input_channels, Some(1));
    }

    /// One ASIO driver is clocked once, for both directions: an output rate
    /// saved from an older configuration is replaced by the driver's own
    /// rate rather than refused at Apply.
    #[test]
    fn one_driver_clocks_both_directions() {
        let mut snapshot = microphone_snapshot();
        snapshot.rates_linked = true;
        let settings = crate::settings::Settings::<crate::settings::NoPreferences> {
            backend: Some("asio".into()),
            sample_rate: Some(48000.0),
            output_sample_rate: Some(44100.0),
            ..Default::default()
        };
        let resolved = snapshot.resolve_settings(&settings).unwrap();
        assert_eq!(resolved.output_sample_rate, Some(48000.0));
    }

    /// The bug this pins. A capability query used to open its own `Asio`, so
    /// `asio-sys`'s already-loaded guard — which lives on that value — was
    /// empty, the load was granted, and the temporary handle's `Drop` stopped
    /// and exited the driver a running stream was using. While a stream owns
    /// the driver, the question must not be put at all: the queries go through
    /// `azo` now, and what its teardown does to a live stream has not been
    /// watched. See [`ASIO_STREAMS`].
    #[test]
    fn a_capability_query_is_refused_while_a_stream_owns_the_driver() {
        // Both halves in one test on purpose: the counter is a process-wide
        // fact, so a test holding a guard elsewhere would make either half's
        // answer depend on the order tests happen to run in.
        static ASKED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let asked = || ASKED.load(std::sync::atomic::Ordering::Relaxed);
        let load = |_: &str| {
            ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(AsioCaps {
                inputs: 6,
                outputs: 2,
                rate: Some(48_000.0),
                ladder: Ladder {
                    min: 8,
                    max: 2_048,
                    steps: Steps::Doubling,
                },
                buffer_pref: Some(32),
                rates: vec![44_100.0, 48_000.0],
            })
        };
        let driver = "Komplete Audio ASIO Driver";
        assert!(!asio_stream_live());
        assert!(asio_caps_answered(driver, load).is_some());
        assert_eq!(asked(), 1);
        let owned = asio_stream_guard();
        assert!(asio_stream_live());
        assert!(asio_caps_answered(driver, load).is_none());
        assert_eq!(asked(), 1, "a refused question must not load a driver");
        drop(owned);
        assert!(!asio_stream_live());
        assert!(asio_caps_answered(driver, load).is_some());
    }

    /// A six-port driver opens a mono stream: ASIO takes fewer buffers than
    /// the hardware has ports, so the format list must carry every count from
    /// one to the port count, each pinned to one rate.
    #[test]
    fn an_asio_driver_offers_every_channel_count_up_to_its_ports() {
        let formats = asio_formats(&[44100.0, 48000.0], Some((64, 1024)), 6);
        assert_eq!(formats.len(), 12);
        for channels in 1..=6 {
            assert!(formats.iter().any(|format| format.channels == channels
                && format.min_rate == 44100
                && format.max_rate == 44100
                && format.buffer_range == Some((64, 1024))));
        }
        assert!(asio_formats(&[48000.0], None, 0).is_empty());
        assert!(asio_formats(&[], Some((64, 1024)), 6).is_empty());
    }

    /// The six sizes this driver refuses, measured by asking it for each one:
    /// on the Komplete Audio 6, `ASIOCreateBuffers` answered
    /// `ASE_HWMALFUNCTION` to 24, 48, 96, 192, 384 and 768, and accepted every
    /// power of two from 8 to 2048. `ASIOGetBufferSize` had answered 8, 2048
    /// and a granularity of `-1`, which was all this panel had to go on, so it
    /// offered exactly the six that cannot be opened.
    #[test]
    fn a_doubling_driver_offers_only_its_powers_of_two() {
        let ladder = Ladder {
            min: 8,
            max: 2_048,
            steps: Steps::Doubling,
        };
        assert_eq!(
            asio_buffer_options(Some(ladder)),
            vec![8, 16, 32, 64, 128, 256, 512, 1024, 2_048]
        );
        // The seeded default has to be one of them, or the panel preselects a
        // choice nobody can make. This driver prefers 64 and gets it; a
        // preference that fell between two accepted sizes rounds up to the next
        // one the driver takes. (No driver here answers that way — the case is
        // constructed, because the rule has to hold for one that someday does.)
        assert_eq!(ladder.snap(Some(64)), Some(64));
        assert_eq!(ladder.snap(Some(96)), Some(128));
        assert_eq!(ladder.snap(None), Some(8));
        assert_eq!(ladder.snap(Some(4_096)), Some(2_048));
    }

    /// A driver that steps linearly — the answer most of the ASIO world gives
    /// — gets its steps honoured, and a driver that says nothing beyond min and
    /// max still gets the whole range, which is all this filtering used to be.
    #[test]
    fn a_stepped_driver_offers_its_steps_and_a_silent_one_its_range() {
        let ladder = Ladder {
            min: 32,
            max: 512,
            steps: Steps::Linear(32),
        };
        assert_eq!(
            asio_buffer_options(Some(ladder)),
            vec![32, 64, 96, 128, 192, 256, 384, 512]
        );
        assert_eq!(ladder.snap(Some(100)), Some(128));
        assert_eq!(
            asio_buffer_options(Some(Ladder {
                min: 16,
                max: 1_024,
                steps: Steps::Any,
            })),
            vec![16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1_024]
        );
        assert!(asio_buffer_options(Some(Ladder::UNKNOWN)).is_empty());
        assert_eq!(Ladder::UNKNOWN.snap(Some(64)), None);
        assert_eq!(asio_buffer_options(None), Vec::new());
    }

    /// The panel that prompted this: ASIO, Komplete Audio ASIO Driver, mono
    /// input at 48000/128, stereo output. It has to resolve.
    #[test]
    fn a_port_limited_driver_opens_the_mono_input_the_tuner_needs() {
        let mut snapshot = microphone_snapshot();
        snapshot.rates = vec![44100.0, 48000.0];
        snapshot.input_formats = asio_formats(&snapshot.rates, Some((64, 1024)), 6);
        snapshot.output_formats = asio_formats(&snapshot.rates, Some((64, 1024)), 2);
        snapshot.input_channels = Some(6);
        snapshot.rates_linked = true;
        snapshot.buffers_linked = true;
        let settings = crate::settings::Settings::<crate::settings::NoPreferences> {
            backend: Some("asio".into()),
            sample_rate: Some(48000.0),
            period_size: Some(128),
            ..Default::default()
        };
        let resolved = snapshot.resolve_settings(&settings).unwrap();
        assert_eq!(resolved.sample_rate, Some(48000.0));
        assert_eq!(resolved.output_sample_rate, Some(48000.0));
        assert_eq!(resolved.period_size, Some(128));
        assert_eq!(resolved.output_period_size, Some(128));
    }

    fn microphone_snapshot() -> Snapshot {
        Snapshot {
            inputs: vec!["Microphone".into()],
            outputs: vec!["Speakers".into()],
            def_in: Some("Microphone".into()),
            def_out: Some("Speakers".into()),
            input_channels: Some(1),
            default_input_channel: Some(1),
            buffer_pref: Some(64),
            output_buffer_pref: Some(256),
            output_channel_options: output_routes(
                2,
                Some(crate::settings::OutputChannels::Stereo([1, 2])),
            ),
            default_output_channels: Some(crate::settings::OutputChannels::Stereo([1, 2])),
            driver_rate: Some(48000.0),
            default_output_rate: Some(48000.0),
            output_rates: vec![48000.0],
            input_formats: vec![StreamFormat {
                channels: 1,
                min_rate: 48000,
                max_rate: 48000,
                buffer_range: Some((32, 256)),
            }],
            output_formats: vec![StreamFormat {
                channels: 2,
                min_rate: 48000,
                max_rate: 48000,
                buffer_range: Some((32, 1024)),
            }],
            ..Default::default()
        }
    }

    fn core_audio_settings() -> crate::settings::Settings<crate::settings::NoPreferences> {
        crate::settings::Settings::<crate::settings::NoPreferences> {
            backend: Some("core-audio".into()),
            ..Default::default()
        }
    }

    #[test]
    fn output_routes_and_defaults_follow_device_channel_capabilities() {
        use crate::settings::OutputChannels::{Mono, Stereo};
        assert_eq!(output_routes(0, None), Vec::new());
        assert_eq!(output_routes(1, None), vec![Mono(1)]);
        let routes = output_routes(4, Some(Stereo([2, 3])));
        assert!(routes.contains(&Stereo([1, 2])));
        assert!(routes.contains(&Stereo([3, 4])));
        assert!(routes.contains(&Stereo([2, 3])));
        assert!(!routes.contains(&Mono(5)));
        let mut snapshot = microphone_snapshot();
        snapshot.output_channel_options = output_routes(4, None);
        let unsupported = crate::settings::Settings::<crate::settings::NoPreferences> {
            output_channels: Some(Stereo([3, 4])),
            ..core_audio_settings()
        };
        assert!(snapshot.resolve_settings(&unsupported).is_err()); // only a two-channel stream is available
        snapshot.default_output_channels = None;
        assert!(
            snapshot
                .resolve_settings(&core_audio_settings())
                .unwrap_err()
                .contains("output channel configuration")
        );
    }

    #[test]
    fn output_rates_use_the_output_default_and_validate_separate_devices_independently() {
        let mut snapshot = microphone_snapshot();
        snapshot.output_formats[0].min_rate = 44100;
        snapshot.default_output_rate = Some(44100.0);
        snapshot.output_rates =
            output_rate_options(&snapshot.output_formats, snapshot.default_output_rate);
        assert_eq!(snapshot.output_rates, vec![44100.0, 48000.0]);
        let mut settings = core_audio_settings();
        let resolved = snapshot.resolve_settings(&settings).unwrap();
        assert_eq!(resolved.sample_rate, Some(48000.0));
        assert_eq!(resolved.output_sample_rate, Some(44100.0));
        let mut shared_device = snapshot.clone();
        shared_device.def_out = shared_device.def_in.clone();
        shared_device.outputs = shared_device.inputs.clone();
        assert!(
            shared_device
                .resolve_settings(&settings)
                .unwrap_err()
                .contains("matching input and output sample rates")
        );
        settings.output_sample_rate = Some(48000.0);
        assert!(snapshot.resolve_settings(&settings).is_ok());
        snapshot.default_output_rate = None;
        settings.output_sample_rate = None;
        assert_eq!(
            snapshot.resolve_settings(&settings).unwrap_err(),
            "Select an output sample rate"
        );
        snapshot.default_output_rate = Some(96000.0);
        settings.sample_rate = Some(96000.0);
        assert!(snapshot.resolve_settings(&settings).is_err());
    }

    #[test]
    fn queried_defaults_resolve_a_complete_mono_microphone_session() {
        let snapshot = microphone_snapshot();
        let resolved = snapshot.resolve_settings(&core_audio_settings()).unwrap();
        assert_eq!(resolved.input_device.as_deref(), Some("Microphone"));
        assert_eq!(resolved.output_device.as_deref(), Some("Speakers"));
        assert_eq!(resolved.period_size, Some(64));
        assert_eq!(resolved.output_period_size, Some(256));
        assert_eq!(resolved.sample_rate, Some(48000.0));
        assert_eq!(resolved.output_sample_rate, Some(48000.0));
        assert_eq!(
            resolved.output_channels,
            Some(crate::settings::OutputChannels::Stereo([1, 2]))
        );
    }

    #[test]
    fn missing_defaults_block_apply_instead_of_inventing_values() {
        let mut snapshot = microphone_snapshot();
        snapshot.default_input_channel = None;
        assert_eq!(
            snapshot
                .resolve_settings(&core_audio_settings())
                .unwrap_err(),
            "Select an input channel"
        );
        let settings = crate::settings::Settings::<crate::settings::NoPreferences> {
            input_channel: Some(1),
            ..core_audio_settings()
        };
        assert!(snapshot.resolve_settings(&settings).is_ok());
        snapshot.buffer_pref = None;
        assert_eq!(
            snapshot.resolve_settings(&settings).unwrap_err(),
            "Select an input buffer size"
        );
        assert!(buffer_options(&[]).is_empty());
        let silent = crate::settings::Settings::<crate::settings::NoPreferences> {
            backend: Some("dummy".into()),
            ..Default::default()
        };
        assert!(Snapshot::default().resolve_settings(&silent).is_ok());
    }

    #[test]
    fn apply_checks_the_actual_stream_combination_and_linked_buffers() {
        let mut snapshot = microphone_snapshot();
        let unsupported = crate::settings::Settings::<crate::settings::NoPreferences> {
            period_size: Some(16),
            ..core_audio_settings()
        };
        assert!(snapshot.resolve_settings(&unsupported).is_err());
        let unsupported = crate::settings::Settings::<crate::settings::NoPreferences> {
            output_period_size: Some(2048),
            ..core_audio_settings()
        };
        assert!(snapshot.resolve_settings(&unsupported).is_err());
        let unsupported = crate::settings::Settings::<crate::settings::NoPreferences> {
            sample_rate: Some(96000.0),
            ..core_audio_settings()
        };
        assert!(snapshot.resolve_settings(&unsupported).is_err());
        snapshot.buffers_linked = true;
        let resolved = snapshot.resolve_settings(&core_audio_settings()).unwrap();
        assert_eq!(resolved.output_period_size, Some(64));
    }

    #[test]
    fn buffer_options_do_not_intersect_independent_stream_ranges() {
        let config = |min, max| {
            cpal::SupportedStreamConfigRange::new(
                2,
                48000,
                48000,
                cpal::SupportedBufferSize::Range { min, max },
                cpal::SampleFormat::F32,
            )
        };
        assert_eq!(buffer_options(&[config(32, 64)]), vec![32, 48, 64]);
        assert_eq!(buffer_options(&[config(256, 512)]), vec![256, 384, 512]);
        assert_eq!(buffer_options(&[config(70, 75)]), vec![70]);
        // Distinct valid configurations are alternatives, not constraints
        // that must all be satisfied simultaneously.
        assert_eq!(
            buffer_options(&[config(64, 64), config(256, 256)]),
            vec![64, 256]
        );
    }

    /// The panel that prompted this: ASIO, Komplete Audio ASIO Driver, channel
    /// 1, output 1–2, 64 frames at 48000, refused on APPLY with `Select an
    /// input channel` although the settings page had just resolved exactly that.
    /// The page asks in a child process and gets answers; the audio owner asks
    /// in the streaming process and gets silence, and silence is not an answer
    /// that can blame the user's own selection.
    #[test]
    fn a_snapshot_that_answered_nothing_accepts_the_configuration_it_was_given() {
        let driver = "Komplete Audio ASIO Driver";
        let silent = Snapshot {
            inputs: vec![driver.into()],
            outputs: vec![driver.into()],
            ..Default::default()
        };
        let settings = crate::settings::Settings::<crate::settings::NoPreferences> {
            backend: Some("asio".into()),
            input_device: Some(driver.into()),
            output_device: Some(driver.into()),
            input_channel: Some(1),
            output_channels: Some(crate::settings::OutputChannels::Stereo([1, 2])),
            sample_rate: Some(48000.0),
            output_sample_rate: Some(48000.0),
            period_size: Some(64),
            output_period_size: Some(64),
            ..Default::default()
        };
        let resolved = silent.resolve_settings(&settings).unwrap();
        assert_eq!(resolved.input_channel, Some(1));
        assert_eq!(resolved.period_size, Some(64));
        // A hole in the configuration is a hole whether or not the device
        // answered: nothing here is invented from silence.
        let mut unfinished = settings.clone();
        unfinished.input_channel = None;
        assert_eq!(
            silent.resolve_settings(&unfinished).unwrap_err(),
            "Select an input channel"
        );
        unfinished.input_channel = Some(1);
        unfinished.output_channels = None;
        assert_eq!(
            silent.resolve_settings(&unfinished).unwrap_err(),
            "Select an output channel configuration"
        );
        // And a driver that DID answer still overrules a pick it cannot make.
        let two_inputs = Snapshot {
            input_channels: Some(2),
            input_formats: vec![StreamFormat {
                channels: 2,
                min_rate: 48000,
                max_rate: 48000,
                buffer_range: Some((8, 1024)),
            }],
            output_channel_options: output_routes(2, None),
            ..silent.clone()
        };
        assert!(two_inputs.resolve_settings(&settings).is_ok());
        let wide = crate::settings::Settings::<crate::settings::NoPreferences> {
            input_channel: Some(5),
            ..settings.clone()
        };
        assert_eq!(
            two_inputs.resolve_settings(&wide).unwrap_err(),
            "Select an input channel"
        );
        let unknown_route = crate::settings::Settings::<crate::settings::NoPreferences> {
            output_channels: Some(crate::settings::OutputChannels::Stereo([7, 8])),
            ..settings
        };
        assert_eq!(
            two_inputs.resolve_settings(&unknown_route).unwrap_err(),
            "Select an output channel configuration"
        );
    }
}
