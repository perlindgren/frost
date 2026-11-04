//! Device ownership and physical output routing for the standalone host.
use super::{
    process::{self, Processor},
    resample::Monitor,
};
use crate::{
    TunerPlugin,
    settings::{OutputChannels, Settings},
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig, SupportedStreamConfigRange};
use crossbeam::channel::{Receiver, Sender, bounded};
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
    Cpal(CpalStreams),
    #[cfg(target_os = "linux")]
    Jack(super::jack_audio::Streams),
}
impl Session for Streams {
    fn wait_started(&mut self) -> Result<(), Failure> {
        match self {
            Self::Cpal(streams) => streams.wait_started(),
            #[cfg(target_os = "linux")]
            Self::Jack(streams) => streams.wait_started(),
        }
    }
    fn failure(&self) -> Option<Failure> {
        match self {
            Self::Cpal(streams) => streams.failure(),
            #[cfg(target_os = "linux")]
            Self::Jack(streams) => streams.failure(),
        }
    }
}

pub(super) struct CpalStreams {
    // Dropping the capture stream joins its callback before the plugin can
    // be reactivated. The output stream never borrows or locks the plugin.
    _input: Stream,
    _output: Stream,
    input_seen: Arc<AtomicBool>,
    output_seen: Arc<AtomicBool>,
    errors: Receiver<Failure>,
}
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
        .ok_or_else(|| format!("Device '{name}' is no longer available"))
}
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

fn error_callback(
    sender: Sender<Failure>,
    direction: &'static str,
    name: String,
) -> impl FnMut(cpal::Error) + Send + 'static {
    move |error| {
        if matches!(
            error.kind(),
            cpal::ErrorKind::Xrun | cpal::ErrorKind::DeviceChanged
        ) {
            return;
        }
        let _ = sender.try_send(Failure {
            message: format!("{direction} device '{name}': {error}"),
            invalidated: error.kind() == cpal::ErrorKind::StreamInvalidated,
        });
    }
}

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
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
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

fn route<T: cpal::Sample + cpal::FromSample<f32>>(
    data: &mut [T],
    channels: usize,
    destination: OutputChannels,
    mut next: impl FnMut() -> f32,
) {
    data.fill(T::EQUILIBRIUM);
    let selected = match destination {
        OutputChannels::Mono(channel) => [channel, channel],
        OutputChannels::Stereo(pair) => pair,
    };
    for frame in data.chunks_exact_mut(channels) {
        let sample = T::from_sample(next());
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
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels as usize;
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _| {
                monitor.begin_block();
                route(data, channels, destination, || monitor.next());
                seen.store(true, Ordering::Release);
            },
            error_callback(errors, "Output", name),
            None,
        )
        .map_err(|e| e.to_string())
}

pub(super) fn open(
    settings: &Settings,
    plugin: Arc<Mutex<TunerPlugin>>,
) -> Result<Streams, Failure> {
    #[cfg(target_os = "linux")]
    if settings.backend.as_deref() == Some("jack") {
        return super::jack_audio::open(settings, plugin).map(Streams::Jack);
    }
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
    result.map(Streams::Cpal).map_err(|message| Failure {
        message: format!("Attempt to open {direction} device '{name}' failed:\n{message}"),
        invalidated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut output = [0u16; 2];
        route(&mut output, 2, OutputChannels::Mono(2), || 0.0);
        assert_eq!(output, [32768; 2]);
    }
}
