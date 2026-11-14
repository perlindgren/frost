//! Plugin-independent stereo transport. Device callbacks own no plugin handles.
//! DSP runs separately, so shutdown does not depend on another device callback.
use crate::{
    OutputChannels, Preferences, Settings,
    resample::{Monitor, StereoFrame},
};
// The two roads this module can carry traffic on, and a Windows build is built
// with one of them: cpal is a `cfg(not(windows))` dependency of this crate
// (`Cargo.toml`), so everything below that speaks cpal is compiled away rather
// than answering "no host" at run time. On Windows the road is
// [`crate::asio_io`], and the code that could open a WASAPI stream is not linked
// in to be asked.
#[cfg(not(windows))]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(not(windows))]
use cpal::{Device, SampleFormat, Stream, StreamConfig, SupportedStreamConfigRange};
#[cfg(not(windows))]
use crossbeam::channel::Sender;
use crossbeam::channel::{Receiver, bounded};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[cfg(target_os = "linux")]
mod jack_io;

#[derive(Default)]
pub struct Counters {
    pub input: AtomicU64,
    pub output: AtomicU64,
    pub dropped: AtomicU64,
}
pub struct Endpoints {
    pub input: rtrb::Consumer<[f32; 2]>,
    pub output: rtrb::Producer<StereoFrame>,
}
/// Owner of whatever carries the two rings. The variants differ only in who
/// runs the callbacks; both sides of the transport and the application DSP
/// owner are identical, so recovery does not depend on the backend.
enum Streams {
    /// Two `cpal::Stream`s, one per direction. This crate declares no cpal on
    /// Windows, so there is nothing for this variant to hold: the transport it
    /// stands for is not compiled there, rather than refused at run time.
    #[cfg(not(windows))]
    Cpal { _input: Stream, _output: Stream },
    /// One driver carrying both directions, with its callbacks on the driver's
    /// own thread. The value owns the process's one ASIO slot and the guard that
    /// keeps the settings page from loading that driver behind its back, so it
    /// is an owner even where only [`DeviceStreams::failure`] reads it.
    #[cfg(all(windows, feature = "asio-host"))]
    Asio(crate::asio_io::Transport),
    #[cfg(target_os = "linux")]
    Jack(Box<jack_io::Transport>),
}
pub struct DeviceStreams {
    /// Never read as audio on purpose: owning these streams *is* what keeps the
    /// device callbacks running, and dropping this value is what stops them.
    /// Only the JACK and ASIO branches of `failure` below reach into the field,
    /// and each only where it is compiled — so on a plain WASAPI or ALSA build the
    /// linter sees an unread field where there is really an owner. The gate never
    /// noticed, because its one runner is Linux; the first Windows
    /// `--features standalone` clippy of any plugin is what surfaced it.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    streams: Streams,
    errors: Receiver<String>,
    pub counters: Arc<Counters>,
}
impl DeviceStreams {
    pub fn failure(&self) -> Option<String> {
        // A `match` rather than a chain of `if let`s, because on Windows this
        // enum has exactly one variant left: an `if let` in front of the only
        // thing a value can be is a pattern the compiler calls irrefutable, and
        // the gate's clippy runs with `-D warnings`.
        match &self.streams {
            #[cfg(target_os = "linux")]
            Streams::Jack(streams) => streams.failure().or_else(|| self.errors.try_recv().ok()),
            // The ASIO transport distinguishes two kinds of news on its one
            // channel: a stream the driver has made unusable, and one block the
            // driver dropped. Only the first is a reason to reopen, and this
            // transport's caller reopens on anything it hears
            // (`nice-rack/src/runtime.rs:467-476`), while cpal delivered both
            // through one error callback and could not tell them apart
            // (`cpal-0.18.2/src/host/asio/stream.rs:1036-1042`). So a dropped
            // block is counted here as the dropout it is, and the channel drained
            // to the first message that means the stream has to go.
            #[cfg(all(windows, feature = "asio-host"))]
            Streams::Asio(stream) => {
                while let Some(failure) = stream.failure() {
                    if failure.invalidated {
                        return Some(failure.message);
                    }
                    self.counters.dropped.fetch_add(1, Ordering::Relaxed);
                }
                self.errors.try_recv().ok()
            }
            #[cfg(not(windows))]
            Streams::Cpal { .. } => self.errors.try_recv().ok(),
        }
    }
}
// Everything between here and `rings` is one host's idea of a device: a `Host`
// to enumerate, a `Device` to ask for configurations, a `Stream` to build per
// sample format. None of it exists on Windows, where the equivalents are
// `azo`'s driver and [`crate::asio_io::open`].
#[cfg(not(windows))]
fn device(host: &cpal::Host, name: &str, input: bool) -> Result<Device, String> {
    let devices = if input {
        host.input_devices()
    } else {
        host.output_devices()
    }
    .map_err(|e| e.to_string())?;
    devices
        .into_iter()
        .find(|device| {
            device
                .description()
                .ok()
                .is_some_and(|description| description.name() == name)
        })
        .ok_or_else(|| {
            format!("Device '{name}' reports no audio channels, or is no longer available")
        })
}
#[cfg(not(windows))]
fn config(
    device: &Device,
    input: bool,
    settings: &Settings<impl Preferences>,
) -> Result<(StreamConfig, SampleFormat), String> {
    let rate = if input {
        settings.sample_rate
    } else {
        settings.output_sample_rate
    }
    .ok_or("Missing sample rate")?;
    let frames = if input {
        settings.period_size
    } else {
        settings.output_period_size
    }
    .ok_or("Missing buffer size")?;
    let count = if input {
        settings.input_channel.unwrap_or(1)
    } else {
        settings
            .output_channels
            .ok_or("Missing output channels")?
            .required_channels()
    };
    let configurations: Vec<SupportedStreamConfigRange> = if input {
        device
            .supported_input_configs()
            .map(|configs| configs.collect())
    } else {
        device
            .supported_output_configs()
            .map(|configs| configs.collect())
    }
    .map_err(|e| e.to_string())?;
    let selected = configurations.iter().filter(|candidate| {
        candidate.channels() as u32 >= count
            && rate >= candidate.min_sample_rate() as f32 && rate <= candidate.max_sample_rate() as f32
            && match candidate.buffer_size() { cpal::SupportedBufferSize::Range { min, max } => frames >= *min && frames <= *max, cpal::SupportedBufferSize::Unknown => true }
            && matches!(candidate.sample_format(), SampleFormat::F32 | SampleFormat::F64 | SampleFormat::I8 | SampleFormat::I16 | SampleFormat::I32 | SampleFormat::I64 | SampleFormat::U8 | SampleFormat::U16 | SampleFormat::U32 | SampleFormat::U64)
    }).min_by_key(|candidate| (candidate.sample_format() != SampleFormat::F32, candidate.channels())).ok_or_else(|| format!("The device does not support the selected channels at {rate} Hz with a buffer of {frames} frames"))?;
    Ok((
        StreamConfig {
            channels: selected.channels(),
            sample_rate: rate as u32,
            buffer_size: cpal::BufferSize::Fixed(frames),
        },
        selected.sample_format(),
    ))
}

/// cpal's one error channel per stream, worded the way this module's callers
/// already read `failure()`. The ASIO transport reports on a channel of its own
/// instead, with the two kinds of news kept apart
/// ([`DeviceStreams::failure`]), which is why this sits with the cpal half.
#[cfg(not(windows))]
fn errors(
    sender: Sender<String>,
    direction: &'static str,
    name: String,
) -> impl FnMut(cpal::Error) + Send + 'static {
    move |error| {
        let _ = sender.try_send(format!("{direction} device '{name}': {error}"));
    }
}
#[cfg(not(windows))]
fn capture<T: cpal::SizedSample>(
    device: &Device,
    config: &StreamConfig,
    selected: usize,
    mut queue: rtrb::Producer<[f32; 2]>,
    counter: Arc<Counters>,
    errors_sender: Sender<String>,
    name: String,
) -> Result<Stream, String>
where
    f32: cpal::FromSample<T>,
{
    let channels = config.channels as usize;
    device
        .build_input_stream(
            *config,
            move |data: &[T], _| {
                for frame in data.chunks_exact(channels) {
                    let sample = <f32 as cpal::FromSample<T>>::from_sample_(frame[selected]);
                    if queue.push([sample, sample]).is_err() {
                        counter.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
                counter.input.fetch_add(1, Ordering::Release);
            },
            errors(errors_sender, "Input", name),
            None,
        )
        .map_err(|error| error.to_string())
}
#[cfg(not(windows))]
fn playback<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &Device,
    config: &StreamConfig,
    mut monitor: Monitor<StereoFrame>,
    route: OutputChannels,
    counter: Arc<Counters>,
    errors_sender: Sender<String>,
    name: String,
) -> Result<Stream, String> {
    let channels = config.channels as usize;
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _| {
                data.fill(T::EQUILIBRIUM);
                monitor.begin_block();
                for frame in data.chunks_exact_mut(channels) {
                    let [l, r] = monitor.next_sample().0;
                    match route {
                        OutputChannels::Mono(channel) => {
                            frame[channel as usize - 1] = T::from_sample_((l + r) * 0.5)
                        }
                        OutputChannels::Stereo([a, b]) => {
                            frame[a as usize - 1] = T::from_sample_(l);
                            frame[b as usize - 1] = T::from_sample_(r);
                        }
                    }
                }
                counter.output.fetch_add(1, Ordering::Release);
            },
            errors(errors_sender, "Output", name),
            None,
        )
        .map_err(|error| error.to_string())
}
/// The two rings and the converter between them, sized from the same numbers
/// every backend must answer: the two rates, the two quanta, and the
/// application's input-latency target. Backend owners hand these to their
/// callbacks; the application owns the other ends through `Endpoints`.
pub(super) struct Rings {
    pub capture: rtrb::Producer<[f32; 2]>,
    pub input: rtrb::Consumer<[f32; 2]>,
    pub output: rtrb::Producer<StereoFrame>,
    pub monitor: Monitor<StereoFrame>,
}
pub(super) fn rings(
    input_rate: f64,
    output_rate: f64,
    input_frames: u32,
    output_frames: u32,
    input_latency: Option<f32>,
) -> Result<Rings, String> {
    let (input_frames, output_frames) = (input_frames as usize, output_frames as usize);
    if !(1..=65536).contains(&input_frames) || !(1..=65536).contains(&output_frames) {
        return Err("Buffer sizes must be between 1 and 65536 frames".into());
    }
    let tolerance =
        input_frames.max((output_frames as f64 * input_rate / output_rate).ceil() as usize);
    let target = tolerance
        + (input_latency
            .unwrap_or(crate::settings::OUR_INPUT_LATENCY_DEFAULT)
            .clamp(0.001, 0.5) as f64
            * input_rate) as usize;
    let capacity = (target * 8)
        .max(input_rate as usize / 2)
        .next_power_of_two();
    if capacity > 4 * 1024 * 1024 {
        return Err("Audio transport buffer budget exceeded".into());
    }
    let (capture, input) = rtrb::RingBuffer::new(capacity);
    let (output, drain) = rtrb::RingBuffer::new(capacity);
    let monitor = Monitor::new(drain, input_rate, output_rate, target, tolerance);
    Ok(Rings {
        capture,
        input,
        output,
        monitor,
    })
}
/// The device ends of the two rings, filled from the driver's own thread.
///
/// cpal gave each direction its own callback, its own sample type and its own
/// conversion; [`crate::asio_io`] converts the driver's bytes once, on the way
/// in and on the way out, and hands both directions as interleaved `f32`. So
/// this is the whole bridge, for both directions at once — which is also the
/// whole reason it can be one object: ASIO runs the input and the output off one
/// clock, in one callback, and the two `cpal::Stream`s it replaces were two
/// clocks pretending to be one.
#[cfg(all(windows, feature = "asio-host"))]
struct Driver {
    capture: rtrb::Producer<[f32; 2]>,
    monitor: Monitor<StereoFrame>,
    route: OutputChannels,
    counters: Arc<Counters>,
    /// The settings page's 1-based input channel, minus one: the index into each
    /// frame of the block, which carries `inputs` channels.
    selected: usize,
    inputs: usize,
    outputs: usize,
}

#[cfg(all(windows, feature = "asio-host"))]
impl crate::asio_io::Pump for Driver {
    fn input(&mut self, interleaved: &[f32]) {
        for frame in interleaved.chunks_exact(self.inputs) {
            let sample = frame[self.selected];
            if self.capture.push([sample, sample]).is_err() {
                self.counters.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.counters.input.fetch_add(1, Ordering::Release);
    }

    fn output(&mut self, interleaved: &mut [f32]) {
        self.monitor.begin_block();
        for frame in interleaved.chunks_exact_mut(self.outputs) {
            let [l, r] = self.monitor.next_sample().0;
            match self.route {
                // A channel this route does not name keeps the zero the
                // transport put there before asking: zero is silence in every
                // format it writes. cpal filled its whole buffer with
                // `T::EQUILIBRIUM` for the same reason
                // (`cpal-0.18.2/src/host/asio/stream.rs:648`).
                OutputChannels::Mono(channel) => frame[channel as usize - 1] = (l + r) * 0.5,
                OutputChannels::Stereo([a, b]) => {
                    frame[a as usize - 1] = l;
                    frame[b as usize - 1] = r;
                }
            }
        }
        self.counters.output.fetch_add(1, Ordering::Release);
    }
}

/// Open the one ASIO driver both directions name, on the settings page's rate
/// and period.
///
/// Nothing here is chosen. [`crate::asio_io::open`] takes the rate and the buffer
/// it is handed or refuses in words, so what is left is the two translations the
/// settings page's choices need — a 1-based input channel into a count of channels
/// to open, a 1-based output route into a count — and the rings, sized from the
/// same four numbers before the driver starts because the pump carries them in.
#[cfg(all(windows, feature = "asio-host"))]
fn open_asio<P: Preferences>(settings: &Settings<P>) -> Result<(DeviceStreams, Endpoints), String> {
    let input_name = settings
        .input_device
        .as_deref()
        .ok_or("Select an input device")?;
    let output_name = settings
        .output_device
        .as_deref()
        .ok_or("Select an output device")?;
    let selected = settings
        .input_channel
        .ok_or("Select an input channel")?
        .checked_sub(1)
        .ok_or("Input channels start at 1")?;
    let route = settings.output_channels.ok_or("Select output channels")?;
    let wanted = crate::asio_io::Wanted {
        input: input_name,
        output: output_name,
        input_rate: settings.sample_rate.ok_or("Missing sample rate")? as u32,
        output_rate: settings
            .output_sample_rate
            .ok_or("Missing output sample rate")? as u32,
        input_frames: settings.period_size.ok_or("Missing buffer size")?,
        output_frames: settings
            .output_period_size
            .ok_or("Select output buffer size")?,
        inputs: selected + 1,
        outputs: route.required_channels(),
    };
    // `fits` is not redundant here even though `outputs` was derived from the
    // route: a hand-edited settings file can name channel 0, or the same channel
    // twice, and `required_channels` of `[0, 0]` is `0` — which would open a
    // driver with no outputs and then index `frame[0 - 1]`.
    if !route.fits(wanted.outputs) {
        return Err("Output route is outside the device layout".into());
    }
    let Rings {
        capture: capture_queue,
        input: input_queue,
        output: output_queue,
        monitor,
    } = rings(
        wanted.input_rate as f64,
        wanted.output_rate as f64,
        wanted.input_frames,
        wanted.output_frames,
        settings.input_latency,
    )?;
    let counters = Arc::new(Counters::default());
    // The pump has to know the shape of the block it will be handed, and `wanted`
    // is taken by `open` — so the two counts it implies are read out first.
    let (inputs, outputs) = (wanted.inputs as usize, wanted.outputs as usize);
    let stream = crate::asio_io::open(
        wanted,
        Driver {
            capture: capture_queue,
            monitor,
            route,
            counters: counters.clone(),
            selected: selected as usize,
            inputs,
            outputs,
        },
    )
    .map_err(|error| format!("Attempt to open ASIO driver '{input_name}' failed: {error}"))?;
    Ok((
        DeviceStreams {
            streams: Streams::Asio(stream),
            // The ASIO transport reports on its own channel, held by
            // `crate::asio_io::Transport`. This end is left with no sender, the
            // same way the JACK transport leaves it
            // (`audio_io/jack_io.rs:218`), so `failure` answers `None` from here
            // for as long as the stream lives.
            errors: bounded(16).1,
            counters,
        },
        Endpoints {
            input: input_queue,
            output: output_queue,
        },
    ))
}

/// Call off the UI thread. Settings must already be resolved against a snapshot.
/// Capture selects a physical input channel and duplicates it into the rack's
/// stereo input. The graph's stereo output is routed without mono downmixing
/// unless an explicit mono destination is selected.
pub fn open<P: Preferences>(settings: &Settings<P>) -> Result<(DeviceStreams, Endpoints), String> {
    let backend = settings
        .backend
        .as_deref()
        .ok_or("Select an audio backend")?;
    if backend == "jack" {
        #[cfg(target_os = "linux")]
        return jack_io::open(settings);
        #[cfg(not(target_os = "linux"))]
        return Err("JACK is offered by the Linux settings page only".into());
    }
    if backend == "dummy" {
        return Err("The dummy backend opens no devices; leave audio off instead".into());
    }
    if backend == "asio" {
        // One driver, both directions, no cpal in the path: see `open_asio`.
        #[cfg(all(windows, feature = "asio-host"))]
        return open_asio(settings);
        #[cfg(not(all(windows, feature = "asio-host")))]
        return Err("ASIO is offered by the Windows standalone build only".into());
    }
    #[cfg(not(windows))]
    return open_cpal(settings);
    // And there is no third transport to fall back on. This crate declares no
    // cpal on Windows (`Cargo.toml`), so a key that is neither
    // `asio` nor `dummy` names a host this binary does not contain — a settings
    // file from another platform, or one edited by hand — rather than a device
    // this process could go and look for.
    #[cfg(windows)]
    return Err(format!(
        "Audio backend '{backend}' is not built into a Windows host: this platform speaks ASIO or nothing"
    ));
}

/// The two streams of the platform that has cpal: one device per direction, each
/// with its own clock, its own sample format and its own error callback, with
/// [`crate::resample::Monitor`] between them.
#[cfg(not(windows))]
fn open_cpal<P: Preferences>(settings: &Settings<P>) -> Result<(DeviceStreams, Endpoints), String> {
    let backend = settings
        .backend
        .as_deref()
        .ok_or("Select an audio backend")?;
    let host = crate::devices::open_host(backend).ok_or("Audio backend is unavailable")?;
    let input_name = settings
        .input_device
        .as_deref()
        .ok_or("Select an input device")?;
    let output_name = settings
        .output_device
        .as_deref()
        .ok_or("Select an output device")?;
    let input = device(&host, input_name, true)?;
    let output = device(&host, output_name, false)?;
    let (input_config, input_format) = config(&input, true, settings)
        .map_err(|error| format!("Attempt to open input device '{input_name}' failed: {error}"))?;
    let (output_config, output_format) = config(&output, false, settings).map_err(|error| {
        format!("Attempt to open output device '{output_name}' failed: {error}")
    })?;
    let selected = settings
        .input_channel
        .ok_or("Select an input channel")?
        .checked_sub(1)
        .ok_or("Input channels start at 1")? as usize;
    if selected >= input_config.channels as usize {
        return Err("Input channel is outside the device layout".into());
    }
    let route = settings.output_channels.ok_or("Select output channels")?;
    if !route.fits(output_config.channels as u32) {
        return Err("Output route is outside the device layout".into());
    }
    let input_frames = settings.period_size.ok_or("Select input buffer size")?;
    let output_frames = settings
        .output_period_size
        .ok_or("Select output buffer size")?;
    let Rings {
        capture: capture_queue,
        input: input_queue,
        output: output_queue,
        monitor,
    } = rings(
        input_config.sample_rate as f64,
        output_config.sample_rate as f64,
        input_frames,
        output_frames,
        settings.input_latency,
    )?;
    let counter = Arc::new(Counters::default());
    let (sender, receiver) = bounded(16);
    macro_rules! output_stream {
        ($t:ty) => {
            playback::<$t>(
                &output,
                &output_config,
                monitor,
                route,
                counter.clone(),
                sender.clone(),
                output_name.into(),
            )
        };
    }
    let output_stream = match output_format {
        SampleFormat::F32 => output_stream!(f32),
        SampleFormat::F64 => output_stream!(f64),
        SampleFormat::I8 => output_stream!(i8),
        SampleFormat::I16 => output_stream!(i16),
        SampleFormat::I32 => output_stream!(i32),
        SampleFormat::I64 => output_stream!(i64),
        SampleFormat::U8 => output_stream!(u8),
        SampleFormat::U16 => output_stream!(u16),
        SampleFormat::U32 => output_stream!(u32),
        SampleFormat::U64 => output_stream!(u64),
        _ => return Err("Unsupported output sample format".into()),
    }
    .map_err(|error| format!("Attempt to open output device '{output_name}' failed: {error}"))?;
    macro_rules! input_stream {
        ($t:ty) => {
            capture::<$t>(
                &input,
                &input_config,
                selected,
                capture_queue,
                counter.clone(),
                sender.clone(),
                input_name.into(),
            )
        };
    }
    let input_stream = match input_format {
        SampleFormat::F32 => input_stream!(f32),
        SampleFormat::F64 => input_stream!(f64),
        SampleFormat::I8 => input_stream!(i8),
        SampleFormat::I16 => input_stream!(i16),
        SampleFormat::I32 => input_stream!(i32),
        SampleFormat::I64 => input_stream!(i64),
        SampleFormat::U8 => input_stream!(u8),
        SampleFormat::U16 => input_stream!(u16),
        SampleFormat::U32 => input_stream!(u32),
        SampleFormat::U64 => input_stream!(u64),
        _ => return Err("Unsupported input sample format".into()),
    }
    .map_err(|error| format!("Attempt to open input device '{input_name}' failed: {error}"))?;
    output_stream.play().map_err(|error| {
        format!("Attempt to start output device '{output_name}' failed: {error}")
    })?;
    input_stream
        .play()
        .map_err(|error| format!("Attempt to start input device '{input_name}' failed: {error}"))?;
    Ok((
        DeviceStreams {
            streams: Streams::Cpal {
                _input: input_stream,
                _output: output_stream,
            },
            errors: receiver,
            counters: counter,
        },
        Endpoints {
            input: input_queue,
            output: output_queue,
        },
    ))
}
