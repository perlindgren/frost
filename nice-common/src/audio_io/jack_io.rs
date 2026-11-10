//! Named-port JACK transport for the rack. It serves the same two rings as
//! the cpal path, so the application's DSP thread remains the only plugin
//! owner and recovery sees one transport through `DeviceStreams::failure`.
//!
//! JACK ports are mono, so the rack's stereo pairing is expressed by port
//! count, not by a channel map: the selected destination receives the left
//! port, and with no right destination selected that port carries the mono
//! average, matching an explicitly mono cpal route. Selecting a second,
//! different destination registers a right port and delivers true stereo
//! (tuner/NAM dual-mono their own output because theirs is mono; the rack
//! graph is stereo, so its second port carries the right channel).
use super::{Counters, DeviceStreams, Endpoints, Rings, Streams, rings};
use crate::Preferences;
use crate::resample::StereoFrame;
use crate::settings::Settings;
use jack::{Client, ClientOptions, Control, Port, PortFlags, PortSpec, ProcessScope};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

struct Handler {
    input: Port<jack::AudioIn>,
    left: Port<jack::AudioOut>,
    right: Option<Port<jack::AudioOut>>,
    capture: rtrb::Producer<[f32; 2]>,
    monitor: crate::resample::Monitor<StereoFrame>,
    counter: Arc<Counters>,
    last_tick: Arc<AtomicU64>,
    started: Instant,
}

impl jack::ProcessHandler for Handler {
    fn process(&mut self, _: &Client, scope: &ProcessScope) -> Control {
        for &sample in self.input.as_slice(scope) {
            if self.capture.push([sample, sample]).is_err() {
                self.counter.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.counter.input.fetch_add(1, Ordering::Release);
        let left = self.left.as_mut_slice(scope);
        left.fill(0.0);
        self.monitor.begin_block();
        match &mut self.right {
            Some(right) => {
                let right = right.as_mut_slice(scope);
                right.fill(0.0);
                for (left, right) in left.iter_mut().zip(right) {
                    let [l, r] = self.monitor.next_sample().0;
                    *left = l;
                    *right = r;
                }
            }
            None => {
                for slot in left {
                    let [l, r] = self.monitor.next_sample().0;
                    *slot = (l + r) * 0.5;
                }
            }
        }
        self.counter.output.fetch_add(1, Ordering::Release);
        self.last_tick
            .store(self.started.elapsed().as_millis() as u64, Ordering::Release);
        Control::Continue
    }
}

/// Liveness answers the transport owns but the delivered-callback counters
/// cannot: a server that stopped delivering, changed rate under us, or lost
/// one of the named ports while still cycling silence.
pub(super) struct Transport {
    client: jack::AsyncClient<(), Handler>,
    last_tick: Arc<AtomicU64>,
    started: Instant,
    rate: u32,
    source: String,
    destinations: Vec<String>,
}

impl Transport {
    pub(super) fn failure(&self) -> Option<String> {
        if self.started.elapsed().as_millis() as u64 > self.last_tick.load(Ordering::Acquire) + 3000
        {
            return Some("JACK stopped delivering audio".into());
        }
        let client = self.client.as_client();
        if client.sample_rate() != self.rate {
            return Some("JACK's sample rate changed; apply audio settings again".into());
        }
        for name in std::iter::once(&self.source).chain(&self.destinations) {
            if client.port_by_name(name).is_none() {
                return Some(format!(
                    "Selected JACK port '{name}' is no longer available"
                ));
            }
        }
        None
    }
}

fn check_port(client: &Client, name: &str, direction: PortFlags) -> Result<(), String> {
    let port = client
        .port_by_name(name)
        .ok_or_else(|| format!("Selected JACK port '{name}' is no longer available"))?;
    if !port.flags().contains(direction)
        || port.port_type().ok().as_deref() != Some(jack::AudioIn::default().jack_port_type())
    {
        return Err(format!(
            "JACK port '{name}' is not an audio {}",
            if direction == PortFlags::IS_OUTPUT {
                "source"
            } else {
                "destination"
            }
        ));
    }
    Ok(())
}

fn connect(client: &Client, source: &str, destination: &str) -> Result<(), String> {
    // PipeWire publishes activated graph ports asynchronously. Retry only on
    // the audio owner, never on the audio callback or UI thread.
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        match client.connect_ports_by_name(source, destination) {
            Ok(()) | Err(jack::Error::PortAlreadyConnected(_, _)) => return Ok(()),
            Err(error) if Instant::now() >= deadline => {
                return Err(format!(
                    "Could not connect JACK '{source}' to '{destination}': {error}"
                ));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// Call off the UI thread with settings resolved against a jack snapshot:
/// one server rate, one linked quantum, mono-named ports.
pub(super) fn open<P: Preferences>(
    settings: &Settings<P>,
) -> Result<(DeviceStreams, Endpoints), String> {
    let source = settings
        .input_device
        .clone()
        .ok_or("Select a JACK input source")?;
    let left_destination = settings
        .output_device
        .clone()
        .ok_or("Select a JACK output destination")?;
    let right_destination = settings.jack_output_right.clone();
    if right_destination.as_ref() == Some(&left_destination) {
        return Err("Choose different JACK destinations for left and right".into());
    }
    let frames = settings.period_size.ok_or("Select a JACK buffer size")?;
    if settings.input_channel.is_some_and(|channel| channel != 1) {
        return Err("JACK ports carry one channel".into());
    }
    static NEXT_CLIENT: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "{}audio-{}-{}",
        crate::devices::jack::TRANSPORT_PREFIX,
        std::process::id(),
        NEXT_CLIENT.fetch_add(1, Ordering::Relaxed)
    );
    let (client, _) = Client::new(&name, ClientOptions::NO_START_SERVER)
        .map_err(|error| format!("Could not connect to JACK: {error}"))?;
    check_port(&client, &source, PortFlags::IS_OUTPUT)?;
    let mut destinations = vec![left_destination.clone()];
    if let Some(right) = &right_destination {
        check_port(&client, right, PortFlags::IS_INPUT)?;
        destinations.push(right.clone());
    }
    let rate = client.sample_rate();
    if settings.sample_rate != Some(rate as f32) || settings.output_sample_rate != Some(rate as f32)
    {
        return Err(format!(
            "JACK is running at {rate} Hz; select the server's sample rate"
        ));
    }
    client
        .set_buffer_size(frames)
        .map_err(|error| format!("JACK rejected the requested {frames}-frame buffer: {error}"))?;
    // The linked snapshot guarantees both directions share rate and quantum.
    let Rings {
        capture,
        input,
        output,
        monitor,
    } = rings(
        rate as f64,
        rate as f64,
        frames,
        settings.output_period_size.unwrap_or(frames),
        settings.input_latency,
    )?;
    let input_port = client
        .register_port("input", jack::AudioIn::default())
        .map_err(|error| error.to_string())?;
    let left = client
        .register_port("left", jack::AudioOut::default())
        .map_err(|error| error.to_string())?;
    let right = if right_destination.is_some() {
        Some(
            client
                .register_port("right", jack::AudioOut::default())
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let input_name = input_port.name().map_err(|error| error.to_string())?;
    let mut output_names = vec![left.name().map_err(|error| error.to_string())?];
    if let Some(right) = &right {
        output_names.push(right.name().map_err(|error| error.to_string())?);
    }
    let counter = Arc::new(Counters::default());
    let started = Instant::now();
    let last_tick = Arc::new(AtomicU64::new(0));
    let (_, errors) = crossbeam::channel::bounded(16);
    let client = client
        .activate_async(
            (),
            Handler {
                input: input_port,
                left,
                right,
                capture,
                monitor,
                counter: counter.clone(),
                last_tick: last_tick.clone(),
                started,
            },
        )
        .map_err(|error| format!("Could not activate JACK: {error}"))?;
    connect(client.as_client(), &source, &input_name)?;
    for (port, destination) in output_names.iter().zip(&destinations) {
        connect(client.as_client(), port, destination)?;
    }
    Ok((
        DeviceStreams {
            streams: Streams::Jack(Box::new(Transport {
                client,
                last_tick,
                started,
                rate,
                source,
                destinations,
            })),
            errors,
            counters: counter,
        },
        Endpoints { input, output },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resample::StereoFrame;
    use std::sync::atomic::AtomicBool;

    /// The transport must be reachable by name: the settings page offers
    /// "jack" on Linux, so `open` answers with the transport's own voice,
    /// not with the retired "physical CPAL" refusal.
    #[test]
    fn jack_backend_reaches_the_transport() {
        let Err(error) = super::super::open(&Settings::<crate::settings::NoPreferences> {
            backend: Some("jack".into()),
            ..Default::default()
        }) else {
            panic!("the jack backend must not open a device transport");
        };
        assert!(error.contains("JACK"), "{error}");
    }

    struct Signal {
        source: Port<jack::AudioOut>,
        left: Port<jack::AudioIn>,
        right: Port<jack::AudioIn>,
        received: Arc<[AtomicU64; 2]>,
    }
    impl jack::ProcessHandler for Signal {
        fn process(&mut self, _: &Client, scope: &ProcessScope) -> Control {
            self.source.as_mut_slice(scope).fill(0.375);
            for (port, received) in [&self.left, &self.right]
                .into_iter()
                .zip(self.received.iter())
            {
                if let Some(&sample) = port.as_slice(scope).iter().find(|&&sample| sample != 0.0) {
                    received.store(sample.to_bits() as u64, Ordering::Release);
                }
            }
            Control::Continue
        }
    }

    fn feed(
        mut frames: rtrb::Producer<StereoFrame>,
        stop: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if frames.push(StereoFrame([0.25, 0.5])).is_err() {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        })
    }

    fn wait_muted(fixture: &jack::AsyncClient<(), Signal>, names: [&str; 2]) {
        // PipeWire publishes graph teardown asynchronously too.
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let muted = names.iter().all(|name| {
                fixture
                    .as_client()
                    .port_by_name(name)
                    .is_some_and(|port| port.connected_count().unwrap_or(1) == 0)
            });
            if muted || Instant::now() >= deadline {
                assert!(muted, "JACK connections to {names:?} outlived the client");
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    #[ignore = "requires a JACK/PipeWire server; uses isolated test ports, no hardware connections"]
    fn named_ports_carry_true_stereo_and_one_destination_gets_the_average() {
        let (client, _) =
            Client::new("frost-rack-routing-test", ClientOptions::NO_START_SERVER).unwrap();
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
        let received: Arc<[AtomicU64; 2]> = Arc::new([AtomicU64::new(0), AtomicU64::new(0)]);
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
        // PipeWire publishes an activated client's ports asynchronously.
        let deadline = Instant::now() + Duration::from_secs(3);
        let snapshot = loop {
            let snapshot = crate::devices::snapshot("jack", None, None);
            if snapshot.inputs.contains(&source_name)
                && snapshot.outputs.contains(&left_name)
                && snapshot.outputs.contains(&right_name)
            {
                break snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "fixture ports never became discoverable"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let resolved = snapshot
            .resolve_settings(&Settings::<crate::settings::NoPreferences> {
                backend: Some("jack".into()),
                input_device: Some(source_name.clone()),
                output_device: Some(left_name.clone()),
                jack_output_right: Some(right_name.clone()),
                ..Default::default()
            })
            .unwrap();
        let (io, Endpoints { mut input, output }) = super::super::open(&resolved).unwrap();
        assert!(
            crate::devices::snapshot("jack", None, None)
                .inputs
                .into_iter()
                .chain(
                    crate::devices::snapshot("jack", None, None)
                        .outputs
                        .into_iter()
                )
                .all(|name| !name.starts_with(crate::devices::jack::TRANSPORT_PREFIX)),
            "the rack must not offer its own audio ports to itself"
        );
        let stop = Arc::new(AtomicBool::new(false));
        let feeder = feed(output, Arc::clone(&stop));
        // Graph startup is asynchronous; both outputs must eventually carry
        // their own channel of the fed stereo frame.
        let deadline = Instant::now() + Duration::from_secs(5);
        while (received[0].load(Ordering::Acquire) == 0 || received[1].load(Ordering::Acquire) == 0)
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            f32::from_bits(received[0].load(Ordering::Acquire) as u32),
            0.25,
            "the left port must carry the left channel"
        );
        assert_eq!(
            f32::from_bits(received[1].load(Ordering::Acquire) as u32),
            0.5,
            "the right port must carry the right channel"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let captured = loop {
            match input.pop() {
                Ok(frame @ [l, _]) if l != 0.0 => break frame,
                Ok(_) => {}
                Err(_) if Instant::now() >= deadline => {
                    panic!("JACK capture never reached the application ring")
                }
                Err(_) => std::thread::sleep(Duration::from_millis(1)),
            }
        };
        assert_eq!(captured, [0.375, 0.375], "capture duplicates into stereo");
        assert!(io.counters.input.load(Ordering::Acquire) > 0);
        assert!(io.counters.output.load(Ordering::Acquire) > 0);
        stop.store(true, Ordering::Relaxed);
        feeder.join().unwrap();
        drop(io);
        drop(input);
        wait_muted(&fixture, [&left_name, &right_name]);
        // One destination is the explicitly-mono route: the average, not one
        // channel dropped and not a dual-mono copy of the left.
        received[0].store(0, Ordering::Relaxed);
        received[1].store(0, Ordering::Relaxed);
        let single = snapshot
            .resolve_settings(&Settings {
                jack_output_right: None,
                ..resolved.clone()
            })
            .unwrap();
        let (io, Endpoints { input, output }) = super::super::open(&single).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let feeder = feed(output, Arc::clone(&stop));
        let deadline = Instant::now() + Duration::from_secs(5);
        while received[0].load(Ordering::Acquire) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            f32::from_bits(received[0].load(Ordering::Acquire) as u32),
            (0.25 + 0.5) * 0.5,
            "a single destination must receive the mono average"
        );
        assert_eq!(
            received[1].load(Ordering::Acquire),
            0,
            "no right destination means no right port"
        );
        stop.store(true, Ordering::Relaxed);
        feeder.join().unwrap();
        drop(io);
        drop(input);
        let _ = resolved;
    }
}
