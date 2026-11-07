//! A duplex JACK client with explicit named-port routing.
use super::{
    audio::{Failure, Session},
    process::{self, Processor},
};
use crate::{NamPlugin, settings::Settings};
use jack::{Client, ClientOptions, Control, Port, PortFlags, PortSpec, ProcessScope};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

struct Handler {
    input: Port<jack::AudioIn>,
    left: Port<jack::AudioOut>,
    right: Option<Port<jack::AudioOut>>,
    processor: Processor,
    frames: Arc<AtomicU32>,
    last_tick: Arc<AtomicU64>,
    started: Instant,
}

impl jack::ProcessHandler for Handler {
    fn process(&mut self, _: &Client, scope: &ProcessScope) -> Control {
        let input = self.input.as_slice(scope);
        let left = self.left.as_mut_slice(scope);
        left.fill(0.0);
        let mut samples = left.iter_mut();
        self.processor.run(input, |sample| {
            if let Some(destination) = samples.next() {
                *destination = sample;
            }
        });
        if let Some(right) = &mut self.right {
            // Dual mono: copy at unity gain, with no pan law or averaging.
            right.as_mut_slice(scope).copy_from_slice(left);
        }
        self.frames.store(scope.n_frames(), Ordering::Release);
        self.last_tick
            .store(self.started.elapsed().as_millis() as u64, Ordering::Release);
        Control::Continue
    }
}

pub(super) struct Streams {
    client: jack::AsyncClient<(), Handler>,
    pub(super) frames: Arc<AtomicU32>,
    last_tick: Arc<AtomicU64>,
    started: Instant,
    requested: u32,
    rate: u32,
    source: String,
    destinations: Vec<String>,
}

fn failure(message: impl Into<String>) -> Failure {
    Failure {
        message: message.into(),
        invalidated: false,
    }
}

impl Session for Streams {
    fn wait_started(&mut self) -> Result<(), Failure> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(error) = self.failure() {
                return Err(error);
            }
            let actual = self.frames.load(Ordering::Acquire);
            if actual == self.requested {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(failure(format!(
                    "JACK did not accept the requested {}-frame buffer: audio runs at {actual} frames",
                    self.requested
                )));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn failure(&self) -> Option<Failure> {
        if self.started.elapsed().as_millis() as u64 > self.last_tick.load(Ordering::Acquire) + 3000
        {
            return Some(failure("JACK stopped delivering audio"));
        }
        let client = self.client.as_client();
        if client.sample_rate() != self.rate {
            return Some(failure(
                "JACK's sample rate changed; apply audio settings again",
            ));
        }
        for name in std::iter::once(&self.source).chain(&self.destinations) {
            if client.port_by_name(name).is_none() {
                return Some(failure(format!(
                    "Selected JACK port '{name}' is no longer available"
                )));
            }
        }
        None
    }
}

fn check_port(client: &Client, name: &str, direction: PortFlags) -> Result<(), Failure> {
    let port = client.port_by_name(name).ok_or_else(|| {
        failure(format!(
            "Selected JACK port '{name}' is no longer available"
        ))
    })?;
    if !port.flags().contains(direction)
        || port.port_type().ok().as_deref() != Some(jack::AudioIn::default().jack_port_type())
    {
        return Err(failure(format!(
            "JACK port '{name}' is not an audio {}",
            if direction == PortFlags::IS_OUTPUT {
                "source"
            } else {
                "destination"
            }
        )));
    }
    Ok(())
}

fn connect(client: &Client, source: &str, destination: &str) -> Result<(), Failure> {
    // PipeWire publishes activated graph ports asynchronously. Retry only on
    // the audio owner, never on the audio callback or UI thread.
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        match client.connect_ports_by_name(source, destination) {
            Ok(()) | Err(jack::Error::PortAlreadyConnected(_, _)) => return Ok(()),
            Err(error) if Instant::now() >= deadline => {
                return Err(failure(format!(
                    "Could not connect JACK '{source}' to '{destination}': {error}"
                )));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

pub(super) fn open(settings: &Settings, plugin: Arc<Mutex<NamPlugin>>) -> Result<Streams, Failure> {
    let source = settings
        .input_device
        .clone()
        .ok_or_else(|| failure("Select a JACK input source"))?;
    let mut destinations = vec![
        settings
            .output_device
            .clone()
            .ok_or_else(|| failure("Select a JACK output destination"))?,
    ];
    if let Some(right) = &settings.jack_output_right {
        if destinations.contains(right) {
            return Err(failure(
                "Choose different JACK destinations for left and right",
            ));
        }
        destinations.push(right.clone());
    }
    static NEXT_CLIENT: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "nice-nam-audio-{}-{}",
        std::process::id(),
        NEXT_CLIENT.fetch_add(1, Ordering::Relaxed)
    );
    let (client, _) = Client::new(&name, ClientOptions::NO_START_SERVER)
        .map_err(|error| failure(format!("Could not connect to JACK: {error}")))?;
    check_port(&client, &source, PortFlags::IS_OUTPUT)?;
    for destination in &destinations {
        check_port(&client, destination, PortFlags::IS_INPUT)?;
    }
    let requested = settings
        .period_size
        .ok_or_else(|| failure("Select a JACK buffer size"))?;
    let rate = client.sample_rate();
    if settings.sample_rate != Some(rate as f32) || settings.output_sample_rate != Some(rate as f32)
    {
        return Err(failure(format!(
            "JACK is running at {rate} Hz; select the server's sample rate"
        )));
    }
    client.set_buffer_size(requested).map_err(|error| {
        failure(format!(
            "JACK rejected the requested {requested}-frame buffer: {error}"
        ))
    })?;
    let input = client
        .register_port("input", jack::AudioIn::default())
        .map_err(|error| failure(error.to_string()))?;
    let left = client
        .register_port("left", jack::AudioOut::default())
        .map_err(|error| failure(error.to_string()))?;
    let right = if destinations.len() == 2 {
        Some(
            client
                .register_port("right", jack::AudioOut::default())
                .map_err(|error| failure(error.to_string()))?,
        )
    } else {
        None
    };
    let input_name = input.name().map_err(|error| failure(error.to_string()))?;
    let mut output_names = vec![left.name().map_err(|error| failure(error.to_string()))?];
    if let Some(right) = &right {
        output_names.push(right.name().map_err(|error| failure(error.to_string()))?);
    }
    {
        let mut instance = plugin
            .lock()
            .map_err(|_| failure("The plugin audio state is unavailable"))?;
        process::activate(&mut instance, 1, rate as f32).map_err(failure)?;
        instance.shared.input_channel.store(1);
    }
    let started = Instant::now();
    let frames = Arc::new(AtomicU32::new(0));
    let last_tick = Arc::new(AtomicU64::new(0));
    let handler = Handler {
        input,
        left,
        right,
        processor: Processor::new(plugin, 1, rate as f32),
        frames: Arc::clone(&frames),
        last_tick: Arc::clone(&last_tick),
        started,
    };
    let client = client
        .activate_async((), handler)
        .map_err(|error| failure(format!("Could not activate JACK: {error}")))?;
    connect(client.as_client(), &source, &input_name)?;
    for (output, destination) in output_names.iter().zip(&destinations) {
        connect(client.as_client(), output, destination)?;
    }
    Ok(Streams {
        client,
        frames,
        last_tick,
        started,
        requested,
        rate,
        source,
        destinations,
    })
}
