//! Device ownership and physical output routing for the standalone host.
use super::process::{self, Processor};
use crate::{
    NamPlugin,
    settings::{OutputChannels, Settings},
};
// One host per platform, and this file is built with one of them: cpal is a
// `cfg(not(windows))` dependency of this crate (`Cargo.toml`), so the device
// enumeration, the per-format stream builders and the two-stream owner below are
// compiled away on Windows instead of standing there unable to answer. The
// Windows half of this file is `AsioStreams` and its `Pump`, one driver with one
// clock carrying the guitar in and the amp out. `route` is shared, which is why
// it speaks `Level` and not `cpal::Sample`.
#[cfg(not(windows))]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(not(windows))]
use cpal::{Device, SampleFormat, Stream, StreamConfig, SupportedStreamConfigRange};
// The error channel belongs to the two-stream host: each of its two callbacks
// reports into it. The driver transport has a channel of its own inside
// `nice_common::asio_io`, which is why this import goes where cpal's does.
#[cfg(not(windows))]
use crossbeam::channel::{Receiver, Sender, bounded};
use nice_common::resample::Monitor;
use nice_common::sample::Level;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(super) struct Failure {
    pub message: String,
    pub invalidated: bool,
}
pub(super) trait Session {
    fn wait_started(&mut self) -> Result<(), Failure>;
    fn failure(&self) -> Option<Failure>;
}

pub(super) enum Streams {
    /// Two independently clocked cpal streams. This crate declares no cpal on
    /// Windows, so there are no streams of this kind to own there and the
    /// variant is not built rather than refused at run time.
    ///
    /// Boxed, because the sizes of the variants are not theirs to choose: with
    /// the azo transport added beside it, this enum's largest variant measured
    /// 304 bytes and the next 88, and a gap over 200 bytes is
    /// `clippy::large_enum_variant`, which `-D warnings` turns into a build
    /// failure. `nice_common::audio_io::Streams` already boxes the variant that
    /// outgrew the others for the same reason (`audio_io.rs:43`). The box is one
    /// allocation on a path that goes on to load a driver and allocate a ring
    /// buffer, and the value it stores moves only where a stream is opened.
    #[cfg(not(windows))]
    Cpal(Box<CpalStreams>),
    #[cfg(all(windows, feature = "asio-host"))]
    Asio(AsioStreams),
    #[cfg(target_os = "linux")]
    Jack(super::jack_audio::Streams),
}
impl Session for Streams {
    fn wait_started(&mut self) -> Result<(), Failure> {
        match self {
            #[cfg(not(windows))]
            Self::Cpal(streams) => streams.wait_started(),
            #[cfg(all(windows, feature = "asio-host"))]
            Self::Asio(streams) => streams.wait_started(),
            #[cfg(target_os = "linux")]
            Self::Jack(streams) => streams.wait_started(),
        }
    }
    fn failure(&self) -> Option<Failure> {
        match self {
            #[cfg(not(windows))]
            Self::Cpal(streams) => streams.failure(),
            #[cfg(all(windows, feature = "asio-host"))]
            Self::Asio(streams) => streams.failure(),
            #[cfg(target_os = "linux")]
            Self::Jack(streams) => streams.failure(),
        }
    }
}

#[cfg(not(windows))]
pub(super) struct CpalStreams {
    // Dropping the capture stream joins its callback before the plugin can
    // be reactivated. The output stream never borrows or locks the plugin.
    _input: Stream,
    _output: Stream,
    input_seen: Arc<AtomicBool>,
    output_seen: Arc<AtomicBool>,
    errors: Receiver<Failure>,
}
#[cfg(not(windows))]
impl Session for CpalStreams {
    fn wait_started(&mut self) -> Result<(), Failure> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(error) = self.failure() {
                return Err(error);
            }
            if self.input_seen.load(Ordering::Acquire) && self.output_seen.load(Ordering::Acquire) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Failure {
                    message:
                        "The devices opened but did not start delivering audio within 3 seconds"
                            .into(),
                    invalidated: false,
                });
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn failure(&self) -> Option<Failure> {
        self.errors.try_recv().ok()
    }
}

/// One ASIO driver, both directions, callbacks on the driver's own thread.
///
/// `nice_common::asio_io` owns the driver — and with it the claim that keeps a
/// settings query from loading the driver this stream is using, and the guard
/// that keeps the panel answering `None` while it runs
/// (`nice_common::devices::asio_stream_live`). Both are released by dropping this
/// value, which is the whole of what stopping means here.
#[cfg(all(windows, feature = "asio-host"))]
pub(super) struct AsioStreams {
    stream: nice_common::asio_io::Transport,
    input_seen: Arc<AtomicBool>,
    output_seen: Arc<AtomicBool>,
}

#[cfg(all(windows, feature = "asio-host"))]
impl Session for AsioStreams {
    fn wait_started(&mut self) -> Result<(), Failure> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(error) = self.failure() {
                return Err(error);
            }
            if self.input_seen.load(Ordering::Acquire) && self.output_seen.load(Ordering::Acquire) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Failure {
                    message:
                        "The driver opened but did not start delivering audio within 3 seconds"
                            .into(),
                    invalidated: false,
                });
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn failure(&self) -> Option<Failure> {
        // The same split as `is_gap` above, made by the driver itself rather
        // than read off a cpal error kind: a block it dropped is counted and
        // carried on, a stream it broke is reported.
        loop {
            let failure = self.stream.failure()?;
            if failure.invalidated {
                return Some(Failure {
                    message: failure.message,
                    invalidated: true,
                });
            }
            crate::DROP_OUTS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The amplifier's two halves, run from the one callback the driver makes.
///
/// cpal ran the capture callback and the playback callback on two threads with
/// two buffers, and the monitor between them existed to cover the gap. Here both
/// halves are the same call in the same order every time, so the model sees a
/// fixed round trip — which is the reason this transport exists as much as the
/// missing SDK is.
#[cfg(all(windows, feature = "asio-host"))]
struct Pump {
    processor: Processor,
    producer: rtrb::Producer<f32>,
    monitor: Monitor,
    destination: OutputChannels,
    /// Channels per frame in the block the driver fills. The capture side needs
    /// no such number because the `Processor` was built with it and de-interleaves
    /// by itself; the playback side is written here, so it is kept.
    outputs: usize,
    input_seen: Arc<AtomicBool>,
    output_seen: Arc<AtomicBool>,
}

#[cfg(all(windows, feature = "asio-host"))]
impl nice_common::asio_io::Pump for Pump {
    fn input(&mut self, interleaved: &[f32]) {
        self.processor.run(interleaved, |sample| {
            let _ = self.producer.push(sample);
        });
        self.input_seen.store(true, Ordering::Release);
    }

    fn output(&mut self, interleaved: &mut [f32]) {
        let (channels, destination) = (self.outputs, self.destination);
        let monitor = &mut self.monitor;
        monitor.begin_block();
        // `route` zeroes the block before it fills the channels the route names,
        // so the driver's unused outputs are silence rather than whatever the
        // buffer held.
        route(interleaved, channels, destination, move || {
            monitor.next_sample()
        });
        self.output_seen.store(true, Ordering::Release);
    }
}

// From here to `open_asio` is cpal's shape of a device: a `Host` to walk, a
// `Device` to ask for configurations, a stream per direction with its own clock
// and its own error callback. On Windows the same page is written by
// `nice_common::asio_io`, which is why the shared code above it names `Level`
// rather than cpal's sample traits.
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
    settings: &Settings,
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
        candidate.channels() as u32 >= count && (!input || matches!(candidate.channels(), 1 | 2))
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

/// Whether a backend error is the audio thread missing its deadline rather than
/// the stream itself going bad. Split out because the two deserve different
/// reactions — a gap is counted and carried on, a broken stream is reported — and
/// because the difference is the whole content of this callback.
#[cfg(not(windows))]
fn is_gap(kind: cpal::ErrorKind) -> bool {
    kind == cpal::ErrorKind::Xrun
}

#[cfg(not(windows))]
fn error_callback(
    sender: Sender<Failure>,
    direction: &'static str,
    name: String,
) -> impl FnMut(cpal::Error) + Send + 'static {
    move |error| {
        if is_gap(error.kind()) {
            // Not worth interrupting anyone for, and much too important to
            // discard: this is the only place that knows the audio thread missed.
            crate::DROP_OUTS.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if error.kind() == cpal::ErrorKind::DeviceChanged {
            return;
        }
        let _ = sender.try_send(Failure {
            message: format!("{direction} device '{name}': {error}"),
            invalidated: error.kind() == cpal::ErrorKind::StreamInvalidated,
        });
    }
}

#[cfg(not(windows))]
fn input_stream<T>(
    device: &Device,
    config: &StreamConfig,
    mut processor: Processor,
    mut producer: rtrb::Producer<f32>,
    seen: Arc<AtomicBool>,
    errors: Sender<Failure>,
    name: String,
) -> Result<Stream, String>
where
    T: cpal::SizedSample + Level,
{
    device
        .build_input_stream(
            *config,
            move |data: &[T], _| {
                processor.run(data, |sample| {
                    let _ = producer.push(sample);
                });
                seen.store(true, Ordering::Release);
            },
            error_callback(errors, "Input", name),
            None,
        )
        .map_err(|e| e.to_string())
}

/// Fill a block of the device's own format with the monitor's next sample, sent
/// to the channels this route names.
///
/// `T` is whatever the device asked for, which on Windows is only ever `f32` —
/// `nice_common::asio_io` converts the driver's bytes before a callback sees them
/// — and `Level` is the pair of conversions and the one silence value this needs
/// from it, so the shared code does not have to say cpal to exist there.
fn route<T: Level>(
    data: &mut [T],
    channels: usize,
    destination: OutputChannels,
    mut next: impl FnMut() -> f32,
) {
    data.fill(T::SILENCE);
    let selected = match destination {
        OutputChannels::Mono(channel) => [channel, channel],
        OutputChannels::Stereo(pair) => pair,
    };
    for frame in data.chunks_exact_mut(channels) {
        let sample = T::from_level(next());
        for channel in selected {
            if let Some(output) = channel
                .checked_sub(1)
                .and_then(|channel| frame.get_mut(channel as usize))
            {
                *output = sample;
            }
        }
    }
}
#[cfg(not(windows))]
fn output_stream<T>(
    device: &Device,
    config: &StreamConfig,
    mut monitor: Monitor,
    destination: OutputChannels,
    seen: Arc<AtomicBool>,
    errors: Sender<Failure>,
    name: String,
) -> Result<Stream, String>
where
    T: cpal::SizedSample + Level,
{
    let channels = config.channels as usize;
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _| {
                monitor.begin_block();
                route(data, channels, destination, || monitor.next_sample());
                seen.store(true, Ordering::Release);
            },
            error_callback(errors, "Output", name),
            None,
        )
        .map_err(|e| e.to_string())
}

/// Open the driver both directions name, and start the model on it.
///
/// The message comes back as a [`Failure`] with `invalidated: false` rather than
/// as a bare sentence like the cpal path's: there is no direction to name,
/// because one call opens both halves, and the sentences
/// `nice_common::asio_io::open` produces already say which of the two the driver
/// refused and what it would have taken.
#[cfg(all(windows, feature = "asio-host"))]
fn open_asio(settings: &Settings, plugin: Arc<Mutex<NamPlugin>>) -> Result<AsioStreams, Failure> {
    let result = (|| -> Result<AsioStreams, String> {
        let input_name = settings
            .input_device
            .clone()
            .ok_or("Missing input device")?;
        let output_name = settings
            .output_device
            .clone()
            .ok_or("Missing output device")?;
        let destination = settings
            .output_channels
            .ok_or("Missing output channel configuration")?;
        let rate = settings.sample_rate.ok_or("Missing sample rate")?;
        let output_rate = settings
            .output_sample_rate
            .ok_or("Missing output sample rate")?;
        let frames = settings.period_size.ok_or("Missing buffer size")?;
        let output_frames = settings.output_period_size.ok_or("Missing buffer size")?;
        let channels = settings.input_channel.unwrap_or(1);
        if channels > 2 {
            // The cpal path filtered its candidates to one or two channels, so a
            // mono amp never opened a thirty-two port interface to read port
            // one. There is nothing to filter here — the count asked for is the
            // count opened — so the same limit is stated instead, in the same
            // words.
            return Err(format!(
                "The device does not support the selected channels at {rate} Hz with a buffer of {frames} frames"
            ));
        }
        let input_frames = frames as usize;
        let output_in_input_frames =
            (output_frames as f64 * rate as f64 / output_rate as f64).ceil() as usize;
        let tolerance = input_frames.max(output_in_input_frames);
        let target = tolerance
            + (settings
                .input_latency
                .unwrap_or(crate::settings::OUR_INPUT_LATENCY_DEFAULT)
                .max(0.001)
                * rate) as usize;
        let capacity = (target * 8).max(rate as usize / 2).next_power_of_two();
        let (producer, consumer) = rtrb::RingBuffer::new(capacity);
        let monitor = Monitor::new(consumer, rate as f64, output_rate as f64, target, tolerance);
        // Before the driver starts, not after: `open` begins delivering audio
        // before it returns, and a callback that reached an unactivated plugin or
        // an unset input channel would be a click at the top of every launch.
        {
            let mut instance = plugin
                .lock()
                .map_err(|_| "The plugin audio state is unavailable")?;
            process::activate(&mut instance, channels as usize, rate)?;
            instance.shared.input_channel.store(channels);
        }
        let processor = Processor::new(plugin, channels as usize, rate);
        let input_seen = Arc::new(AtomicBool::new(false));
        let output_seen = Arc::new(AtomicBool::new(false));
        let wanted = nice_common::asio_io::Wanted {
            input: &input_name,
            output: &output_name,
            input_rate: rate as u32,
            output_rate: output_rate as u32,
            input_frames: frames,
            output_frames,
            inputs: channels,
            outputs: destination.required_channels(),
        };
        let outputs = wanted.outputs as usize;
        tracing::info!(
            "Starting ASIO driver '{input_name}': {channels} in, {} out, {rate} Hz, {frames} frames",
            wanted.outputs
        );
        let stream = nice_common::asio_io::open(
            wanted,
            Pump {
                processor,
                producer,
                monitor,
                destination,
                outputs,
                input_seen: input_seen.clone(),
                output_seen: output_seen.clone(),
            },
        )
        .map_err(|error| format!("Attempt to open ASIO driver '{input_name}' failed:\n{error}"))?;
        Ok(AsioStreams {
            stream,
            input_seen,
            output_seen,
        })
    })();
    result.map_err(|message| Failure {
        message,
        invalidated: false,
    })
}

pub(super) fn open(settings: &Settings, plugin: Arc<Mutex<NamPlugin>>) -> Result<Streams, Failure> {
    // The number on screen belongs to the streams you can see, so a new pair of
    // them starts at zero: otherwise a device change leaves the old count under
    // the new device, which is exactly the misreading a counter invites.
    crate::DROP_OUTS.store(0, Ordering::Relaxed);
    #[cfg(target_os = "linux")]
    if settings.backend.as_deref() == Some("jack") {
        return super::jack_audio::open(settings, plugin).map(Streams::Jack);
    }
    #[cfg(all(windows, feature = "asio-host"))]
    if settings.backend.as_deref() == Some("asio") {
        return open_asio(settings, plugin).map(Streams::Asio);
    }
    #[cfg(not(windows))]
    return open_cpal(settings, plugin);
    // And nothing to fall back to. `available_backends` offers `asio` and
    // `dummy` on this platform (`nice_common::devices`), and the resolver
    // refuses every other key there, so a backend that reaches this line was
    // written for a machine whose hosts this binary does not contain.
    #[cfg(windows)]
    return Err(Failure {
        message: format!(
            "Audio backend '{}' is not built into this host: Windows speaks ASIO or nothing",
            settings.backend.as_deref().unwrap_or("nothing selected")
        ),
        invalidated: false,
    });
}

/// The host this crate was built with everywhere but Windows: two devices found
/// by name on one backend's host, each with its own clock, its own sample format
/// and its own error callback, with [`Monitor`] between them.
#[cfg(not(windows))]
fn open_cpal(settings: &Settings, plugin: Arc<Mutex<NamPlugin>>) -> Result<Streams, Failure> {
    let mut direction = "input";
    let mut name = settings.input_device.clone().unwrap_or_default();
    let result = (|| -> Result<CpalStreams, String> {
        let backend = settings
            .backend
            .as_deref()
            .ok_or("Select an audio backend")?;
        let host = crate::devices::open_host(backend)
            .ok_or_else(|| format!("Audio backend '{backend}' is unavailable"))?;
        let input = device(&host, &name, true)?;
        let (input_config, input_format) = config(&input, true, settings)?;
        direction = "output";
        name = settings
            .output_device
            .clone()
            .ok_or("Missing output device")?;
        let output = device(&host, &name, false)?;
        let (output_config, output_format) = config(&output, false, settings)?;
        let input_frames = settings.period_size.unwrap() as usize;
        let output_in_input_frames = (settings.output_period_size.unwrap() as f64
            * input_config.sample_rate as f64
            / output_config.sample_rate as f64)
            .ceil() as usize;
        let tolerance = input_frames.max(output_in_input_frames);
        let target = tolerance
            + (settings
                .input_latency
                .unwrap_or(crate::settings::OUR_INPUT_LATENCY_DEFAULT)
                .max(0.001)
                * input_config.sample_rate as f32) as usize;
        let capacity = (target * 8)
            .max(input_config.sample_rate as usize / 2)
            .next_power_of_two();
        let (producer, consumer) = rtrb::RingBuffer::new(capacity);
        let monitor = Monitor::new(
            consumer,
            input_config.sample_rate as f64,
            output_config.sample_rate as f64,
            target,
            tolerance,
        );
        let (errors, failure) = bounded(16);
        let input_seen = Arc::new(AtomicBool::new(false));
        let output_seen = Arc::new(AtomicBool::new(false));
        let destination = settings
            .output_channels
            .ok_or("Missing output channel configuration")?;
        macro_rules! build_output {
            ($ty:ty) => {
                output_stream::<$ty>(
                    &output,
                    &output_config,
                    monitor,
                    destination,
                    output_seen.clone(),
                    errors.clone(),
                    name.clone(),
                )
            };
        }
        let output_stream = match output_format {
            SampleFormat::F32 => build_output!(f32),
            SampleFormat::F64 => build_output!(f64),
            SampleFormat::I8 => build_output!(i8),
            SampleFormat::I16 => build_output!(i16),
            SampleFormat::I32 => build_output!(i32),
            SampleFormat::I64 => build_output!(i64),
            SampleFormat::U8 => build_output!(u8),
            SampleFormat::U16 => build_output!(u16),
            SampleFormat::U32 => build_output!(u32),
            SampleFormat::U64 => build_output!(u64),
            _ => Err("Unsupported output sample format".into()),
        }?;
        direction = "input";
        name = settings.input_device.clone().unwrap();
        {
            let mut instance = plugin
                .lock()
                .map_err(|_| "The plugin audio state is unavailable")?;
            process::activate(
                &mut instance,
                input_config.channels as usize,
                input_config.sample_rate as f32,
            )?;
            instance
                .shared
                .input_channel
                .store(settings.input_channel.unwrap_or(1));
        }
        let processor = Processor::new(
            plugin,
            input_config.channels as usize,
            input_config.sample_rate as f32,
        );
        macro_rules! build_input {
            ($ty:ty) => {
                input_stream::<$ty>(
                    &input,
                    &input_config,
                    processor,
                    producer,
                    input_seen.clone(),
                    errors.clone(),
                    name.clone(),
                )
            };
        }
        let input_stream = match input_format {
            SampleFormat::F32 => build_input!(f32),
            SampleFormat::F64 => build_input!(f64),
            SampleFormat::I8 => build_input!(i8),
            SampleFormat::I16 => build_input!(i16),
            SampleFormat::I32 => build_input!(i32),
            SampleFormat::I64 => build_input!(i64),
            SampleFormat::U8 => build_input!(u8),
            SampleFormat::U16 => build_input!(u16),
            SampleFormat::U32 => build_input!(u32),
            SampleFormat::U64 => build_input!(u64),
            _ => Err("Unsupported input sample format".into()),
        }?;
        tracing::info!("Starting input device '{name}': {input_config:?}");
        input_stream.play().map_err(|e| e.to_string())?;
        direction = "output";
        name = settings.output_device.clone().unwrap();
        tracing::info!("Starting output device '{name}': {output_config:?}");
        output_stream.play().map_err(|e| e.to_string())?;
        Ok(CpalStreams {
            _input: input_stream,
            _output: output_stream,
            input_seen,
            output_seen,
            errors: failure,
        })
    })();
    result
        .map(|streams| Streams::Cpal(Box::new(streams)))
        .map_err(|message| Failure {
            message: format!("Attempt to open {direction} device '{name}' failed:\n{message}"),
            invalidated: false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn a_gap_is_counted_while_a_dead_stream_still_reaches_the_user() {
        // Decided apart on purpose: a gap is a missed deadline that the show
        // carries on through, an invalidated stream is a device that has gone,
        // and a counter that mixed the two would stop meaning anything.
        assert!(is_gap(cpal::ErrorKind::Xrun));
        assert!(!is_gap(cpal::ErrorKind::DeviceChanged));
        assert!(!is_gap(cpal::ErrorKind::StreamInvalidated));
    }

    /// A fixed-point format whose silence is not zero, which is the whole reason
    /// it exists: the second case below can then tell `route`'s fill apart from a
    /// fill of zeroes.
    ///
    /// It is not a stand-in for a real device format. A real caller hands `route`
    /// either the driver's `f32` block — `nice_common::asio_io` converts the bytes
    /// before a callback sees them, so `f32` is the only format a Windows build
    /// ever passes here — or one of the formats cpal builds its streams for, and
    /// those are `Level` only where cpal is in the graph. A witness written here
    /// keeps the property checked on every platform instead of losing it on the
    /// one that no longer has cpal to name a format with.
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Q15(i32);

    impl Level for Q15 {
        const SILENCE: Self = Self(32_768);
        fn level(self) -> f32 {
            (self.0 - 32_768) as f32 / 32_768.0
        }
        fn from_level(value: f32) -> Self {
            Self((value * 32_768.0) as i32 + 32_768)
        }
    }

    #[test]
    fn routing_duplicates_selected_mono_only_to_the_selected_pair() {
        let samples = [0.25, -0.5];
        let mut index = 0;
        let mut output = [9.0f32; 8];
        route(&mut output, 4, OutputChannels::Stereo([3, 4]), || {
            let value = samples[index];
            index += 1;
            value
        });
        assert_eq!(output, [0.0, 0.0, 0.25, 0.25, 0.0, 0.0, -0.5, -0.5]);
        // The channel this route does not name keeps the fill; the one it names
        // gets the sample. A format that rests at zero could not tell those two
        // apart.
        let mut block = [Q15(7); 2];
        route(&mut block, 2, OutputChannels::Mono(2), || -0.5);
        assert_eq!(block, [Q15::SILENCE, Q15::from_level(-0.5)]);
    }
}
