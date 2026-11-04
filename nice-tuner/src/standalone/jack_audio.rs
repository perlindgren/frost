//! A duplex JACK client with explicit named-port routing.
use super::{
    audio::{Failure, Session},
    process::{self, Processor},
};
use crate::{TunerPlugin, settings::Settings};
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

pub(super) fn open(
    settings: &Settings,
    plugin: Arc<Mutex<TunerPlugin>>,
) -> Result<Streams, Failure> {
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
        "nice-tuner-audio-{}-{}",
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

#[cfg(test)]
mod tests {
    use super::super::{AudioOwner, audio};
    use super::*;

    struct Signal {
        source: Port<jack::AudioOut>,
        left: Port<jack::AudioIn>,
        right: Port<jack::AudioIn>,
        received: Arc<[AtomicU32; 2]>,
    }
    impl jack::ProcessHandler for Signal {
        fn process(&mut self, _: &Client, scope: &ProcessScope) -> Control {
            self.source.as_mut_slice(scope).fill(0.375);
            for (port, received) in [&self.left, &self.right]
                .into_iter()
                .zip(self.received.iter())
            {
                if let Some(&sample) = port.as_slice(scope).iter().find(|&&sample| sample != 0.0) {
                    received.store(sample.to_bits(), Ordering::Release);
                }
            }
            Control::Continue
        }
    }

    #[test]
    #[ignore = "requires a JACK/PipeWire server; uses isolated test ports, no hardware connections"]
    fn named_routes_preserve_full_level_on_both_outputs_and_restore_after_failure() {
        let (client, _) =
            Client::new("frost-jack-routing-test", ClientOptions::NO_START_SERVER).unwrap();
        let source = client
            .register_port("instrument", jack::AudioOut::default())
            .unwrap();
        let left = client
            .register_port("monitor_left", jack::AudioIn::default())
            .unwrap();
        let right = client
            .register_port("monitor_right", jack::AudioIn::default())
            .unwrap();
        let source_name = source.name().unwrap();
        let left_name = left.name().unwrap();
        let right_name = right.name().unwrap();
        let received = Arc::new([AtomicU32::new(0), AtomicU32::new(0)]);
        let fixture = client
            .activate_async(
                (),
                Signal {
                    source,
                    left,
                    right,
                    received: Arc::clone(&received),
                },
            )
            .unwrap();
        let snapshot = crate::devices::snapshot("jack", None, None);
        assert!(snapshot.inputs.contains(&source_name));
        assert!(!snapshot.inputs.contains(&left_name));
        assert!(snapshot.outputs.contains(&left_name));
        assert!(!snapshot.outputs.contains(&source_name));
        assert!(
            snapshot
                .inputs
                .iter()
                .chain(&snapshot.outputs)
                .all(|name| !name.starts_with(crate::devices::jack::CLIENT_PREFIX))
        );
        let settings = snapshot
            .resolve_settings(&Settings {
                backend: Some("jack".into()),
                input_device: Some(source_name.clone()),
                output_device: Some(left_name.clone()),
                jack_output_right: Some(right_name.clone()),
                ..Default::default()
            })
            .unwrap();
        let plugin = Arc::new(Mutex::new(TunerPlugin::default()));
        let shared = Arc::clone(&plugin.lock().unwrap().shared);
        let mut owner = AudioOwner {
            open: move |settings: &Settings| audio::open(settings, Arc::clone(&plugin)),
            session: None,
            applied: Settings::default(),
        };
        for frames in crate::devices::JACK_BUFFER_CHOICES {
            owner
                .apply(Settings {
                    period_size: Some(frames),
                    output_period_size: Some(frames),
                    ..settings.clone()
                })
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(1);
            while received
                .iter()
                .any(|sample| sample.load(Ordering::Acquire) == 0)
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            for sample in received.iter() {
                assert_eq!(
                    f32::from_bits(sample.load(Ordering::Acquire)),
                    0.375,
                    "stereo must preserve the full mono amplitude"
                );
                sample.store(0, Ordering::Release);
            }
            assert_eq!(shared.buffer_frames.load(), frames);
            println!("Verified named stereo routing at {frames} frames with unity gain");
        }
        let previous = owner.applied.clone();
        let error = owner
            .apply(Settings {
                input_device: Some("missing-client:missing-source".into()),
                ..settings.clone()
            })
            .unwrap_err();
        assert!(error.contains("missing-client:missing-source"), "{error}");
        assert_eq!(owner.applied, previous);
        assert!(owner.session.is_some());
        // Wrong-direction and duplicate destinations must fail before opening.
        assert!(
            snapshot
                .resolve_settings(&Settings {
                    input_device: Some(left_name.clone()),
                    ..settings.clone()
                })
                .is_err()
        );
        assert!(
            snapshot
                .resolve_settings(&Settings {
                    jack_output_right: Some(left_name),
                    ..settings.clone()
                })
                .is_err()
        );
        drop(owner);
        // PipeWire publishes graph teardown asynchronously too.
        let deadline = Instant::now() + Duration::from_secs(1);
        while fixture
            .as_client()
            .port_by_name(&right_name)
            .unwrap()
            .connected_count()
            .unwrap()
            != 0
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            fixture
                .as_client()
                .port_by_name(&right_name)
                .unwrap()
                .connected_count()
                .unwrap(),
            0
        );
    }
}
