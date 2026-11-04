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
pub(crate) mod jack;

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
/// ASIO appears only when compiled via `--features standalone-asio`.
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

pub(crate) fn open_host(backend: &str) -> Option<cpal::Host> {
    if backend == "auto" || backend.is_empty() {
        return Some(cpal::default_host());
    }
    cpal::host_from_id(host_id_from_value(backend)?).ok()
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
        // ASIO has no system default: cpal nominates the FIRST driver.
        // Refine that to the first driver that can actually serve the
        // direction — an output-only driver must never be nominated
        // as the tuner's input. (list_*_devices already filters by
        // channel capability, in registry order.)
        return (
            list_input_devices(backend).into_iter().next(),
            list_output_devices(backend).into_iter().next(),
        );
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
    let host = if backend == "auto" || backend.is_empty() {
        cpal::default_host()
    } else {
        open_host(backend)?
    };
    let in_dev = match input {
        Some(n) => host
            .input_devices()
            .ok()
            .and_then(|d| find_named(d, n))
            .or_else(|| host.default_input_device()),
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
        Some(n) => host
            .output_devices()
            .ok()
            .and_then(|d| find_named(d, n))
            .or_else(|| host.default_output_device()),
        None => host.default_output_device(),
    };
    out_dev
        .as_ref()
        .and_then(|d| d.default_output_config().ok())
        .map(|c| c.sample_rate() as f32)
}

/// Query the selected output device's own default, without falling back to input.
fn output_default_rate(backend: &str, output: Option<&str>) -> Option<f32> {
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

pub(crate) const JACK_BUFFER_CHOICES: [u32; 8] = [32, 48, 64, 96, 128, 256, 512, 1024];

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

pub fn stream_options(backend: &str, input: Option<&str>, output: Option<&str>) -> StreamOptions {
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
/// a throwaway child process and shipped back as JSON.
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
    pub output_buffer_pref: Option<u32>,
    pub driver_rate: Option<f32>,
    pub input_formats: Vec<StreamFormat>,
    pub output_formats: Vec<StreamFormat>,
    pub output_channel_options: Vec<crate::settings::OutputChannels>,
    pub default_output_channels: Option<crate::settings::OutputChannels>,
    /// The device's OWN preferred buffer size (ASIOGetBufferSize's
    /// `pref` — the manufacturer panel's number). cpal discards it;
    /// the streamless child asks asio-sys directly.
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
    /// Resolve real device defaults and validate a complete duplex session.
    /// Resolve every stream field before handing it to the application audio owner.
    pub fn resolve_settings(
        &self,
        settings: &crate::settings::Settings,
    ) -> Result<crate::settings::Settings, String> {
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
            .filter(|&ch| ch > 0 && self.input_channels.is_some_and(|count| ch <= count))
            .ok_or("Select an input channel")?;
        let output_route = resolved
            .output_channels
            .or(self.default_output_channels)
            .filter(|route| self.output_channel_options.contains(route))
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
        let output_rate = resolved
            .output_sample_rate
            .or(self.default_output_rate)
            .ok_or("Select an output sample rate")?;
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
        let _layout = [2, 1]
            .into_iter()
            .position(|channels| {
                channel <= channels
                    && self.input_formats.iter().any(|format| {
                        format.channels == channels && supports(format, rate, input_buffer)
                    })
            })
            .ok_or(
                "The input channel, rate and buffer size are not a supported stream configuration",
            )?;
        if !self.output_formats.iter().any(|format| {
            format.channels >= output_route.required_channels()
                && supports(format, output_rate, output_buffer)
        }) {
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

/// The driver's preferred buffer size for an ASIO device, by direct
/// asio-sys call. Runs where cpal has just finished loading and
/// releasing its driver handles (the child), so the brief second load
/// cannot collide with anything. Everywhere else — and on any failure,
/// including this process merely streaming (`DriverAlreadyExists`) —
/// honestly None.
#[cfg(all(windows, feature = "asio-host"))]
fn asio_buffer_pref(name: &str) -> Option<u32> {
    use asio_sys::bindings::BufferPreference;
    let driver = asio_sys::Asio::new().load_driver(name).ok()?;
    let pref = driver.buffersize_range().ok()?.preferred;
    let n = match pref {
        BufferPreference::Only(n) | BufferPreference::Preferred(n) => n,
        BufferPreference::Stepped { preferred, .. } => preferred,
    };
    (n > 0).then_some(n)
}

#[cfg(not(all(windows, feature = "asio-host")))]
fn asio_buffer_pref(_name: &str) -> Option<u32> {
    None
}

/// In-process snapshot: every enumeration call, once.
pub fn snapshot(backend: &str, input: Option<&str>, output: Option<&str>) -> Snapshot {
    #[cfg(target_os = "linux")]
    if backend == "jack" {
        return jack::snapshot();
    }
    let (def_in, def_out) = default_device_names(backend);
    let (rates, mut buffers, mut output_buffers, input_formats, output_formats) =
        stream_options(backend, input, output);
    let buffers_linked = crate::settings::buffers_must_match(
        backend,
        input.or(def_in.as_deref()),
        output.or(def_out.as_deref()),
    );
    let driver_rate = driver_default_rate(backend, input, output);
    let default_output_rate = output_default_rate(backend, output);
    let output_rates = output_rate_options(&output_formats, default_output_rate);
    let input_channels = input_channel_count(backend, input);
    // Preferred buffer: a property of the input device (the one whose
    // clock runs the show), or of whatever the default input is.
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut buffer_pref = (backend == "asio")
        .then(|| input.map(str::to_owned).or_else(|| def_in.clone()))
        .flatten()
        .as_deref()
        .and_then(asio_buffer_pref);
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
/// asio-sys keeps ONE driver loaded globally (the running stream's) and
/// refuses every other (`DriverAlreadyExists`), and destroying drivers
/// mid-enumeration would tear at the live one. The panel cannot close
/// the device — nice-plug owns the stream — and the user must not have
/// to restart. So: ask a CHILD process, which opens no stream at all
/// and therefore sees every registered driver. The parent's audio
/// never stops; nothing is closed; the child enumerates and dies.
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
            "nice-tuner: device-helper exited {:?}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
        return None;
    }
    match serde_json::from_slice(&out.stdout) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!(
                "nice-tuner: device-helper output unreadable: {e}: {}",
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
        let settings = crate::settings::Settings {
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
                .resolve_settings(&crate::settings::Settings {
                    period_size: Some(frames),
                    ..settings.clone()
                })
                .unwrap();
            assert_eq!(resolved.period_size, Some(frames));
            assert_eq!(resolved.output_period_size, Some(frames));
        }
        assert!(
            snapshot
                .resolve_settings(&crate::settings::Settings {
                    sample_rate: Some(48000.0),
                    ..settings
                })
                .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a running JACK server (or pw-jack)"]
    fn jack_server_defaults_resolve_without_manual_device_settings() {
        let snapshot = snapshot("jack", None, None);
        let resolved = snapshot
            .resolve_settings(&crate::settings::Settings {
                backend: Some("jack".into()),
                ..Default::default()
            })
            .expect("JACK server must expose usable input/output defaults");
        assert_eq!(resolved.sample_rate, resolved.output_sample_rate);
        assert_eq!(resolved.period_size, resolved.output_period_size);
        assert!(snapshot.buffers_linked);
        assert!(snapshot.inputs.iter().all(|name| name.contains(':')));
        assert!(snapshot.outputs.iter().all(|name| name.contains(':')));
        assert_eq!(snapshot.input_channels, Some(1));
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

    fn core_audio_settings() -> crate::settings::Settings {
        crate::settings::Settings {
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
        let unsupported = crate::settings::Settings {
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
        let settings = crate::settings::Settings {
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
        let silent = crate::settings::Settings {
            backend: Some("dummy".into()),
            ..Default::default()
        };
        assert!(Snapshot::default().resolve_settings(&silent).is_ok());
    }

    #[test]
    fn apply_checks_the_actual_stream_combination_and_linked_buffers() {
        let mut snapshot = microphone_snapshot();
        let unsupported = crate::settings::Settings {
            period_size: Some(16),
            ..core_audio_settings()
        };
        assert!(snapshot.resolve_settings(&unsupported).is_err());
        let unsupported = crate::settings::Settings {
            output_period_size: Some(2048),
            ..core_audio_settings()
        };
        assert!(snapshot.resolve_settings(&unsupported).is_err());
        let unsupported = crate::settings::Settings {
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
}
