//! Plugin-independent stereo transport. Device callbacks own no plugin handles.
//! DSP runs separately, so shutdown does not depend on another device callback.
use crate::{
    OutputChannels, Preferences, Settings,
    resample::{Monitor, StereoFrame},
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig, SupportedStreamConfigRange};
use crossbeam::channel::{Receiver, Sender, bounded};
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
    Cpal {
        _input: Stream,
        _output: Stream,
        _asio: Option<crate::devices::AsioStreamGuard>,
    },
    #[cfg(target_os = "linux")]
    Jack(Box<jack_io::Transport>),
}
pub struct DeviceStreams {
    streams: Streams,
    errors: Receiver<String>,
    pub counters: Arc<Counters>,
}
impl DeviceStreams {
    pub fn failure(&self) -> Option<String> {
        #[cfg(target_os = "linux")]
        if let Streams::Jack(streams) = &self.streams
            && let failure @ Some(_) = streams.failure()
        {
            return failure;
        }
        self.errors.try_recv().ok()
    }
}
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

fn errors(
    sender: Sender<String>,
    direction: &'static str,
    name: String,
) -> impl FnMut(cpal::Error) + Send + 'static {
    move |error| {
        let _ = sender.try_send(format!("{direction} device '{name}': {error}"));
    }
}
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
    let asio = (backend == "asio").then(crate::devices::asio_stream_guard);
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
                _asio: asio,
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
