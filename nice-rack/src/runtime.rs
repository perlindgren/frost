//! Main-thread lifecycle controller. Device transport and DSP are separate owners.
use crate::{
    Settings,
    routing::{Channels, Connection, NodeId, NodeKind, RackGraph},
};
use crossbeam::channel::{Receiver, Sender, bounded};
use nice_clap_host::{Binding, Entry, LiveAudio, LiveSession, LiveStatus};
use nice_common::audio_io::{DeviceStreams, Endpoints};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const BLOCK: usize = 1024;
#[derive(Clone)]
pub struct Card {
    pub id: NodeId,
    pub stage: u64,
    pub name: String,
    pub details: nice_clap_host::Descriptor,
    pub path: PathBuf,
    pub binding: Binding,
    pub inputs: Channels,
    pub outputs: Channels,
}
impl Card {
    pub fn new(
        id: NodeId,
        path: PathBuf,
        entry: Entry,
        class: String,
        name: String,
    ) -> Result<Self, String> {
        let details = entry
            .descriptors()?
            .into_iter()
            .find(|d| d.id == class)
            .ok_or("Plugin class not found")?;
        let (inputs, outputs) = entry.main_channels(&class)?;
        Ok(Self {
            id,
            stage: id.0,
            name,
            details,
            path,
            binding: Binding { entry, class },
            inputs,
            outputs,
        })
    }
}
pub fn stages(cards: &[Card]) -> Vec<Vec<&Card>> {
    let mut groups: Vec<Vec<&Card>> = Vec::new();
    for card in cards {
        if groups
            .last()
            .is_some_and(|group| group[0].stage == card.stage)
        {
            groups.last_mut().unwrap().push(card);
        } else {
            groups.push(vec![card]);
        }
    }
    groups
}
pub fn arrange(
    cards: Vec<Card>,
    moving: NodeId,
    target: crate::layout::Target,
) -> Result<Vec<Card>, String> {
    let layout = stages(&cards)
        .iter()
        .map(|group| group.iter().map(|card| card.id).collect())
        .collect::<Vec<_>>();
    let layout = crate::layout::place(&layout, moving, target)?;
    let mut cards = cards
        .into_iter()
        .map(|card| (card.id, card))
        .collect::<BTreeMap<_, _>>();
    let mut result = Vec::new();
    for group in layout {
        let key = group[0].0;
        for id in group {
            let mut card = cards.remove(&id).unwrap();
            card.stage = key;
            result.push(card);
        }
    }
    Ok(result)
}
pub fn chain(cards: &[Card]) -> (RackGraph, BTreeMap<NodeId, Binding>) {
    let mut graph = RackGraph::default();
    let mut bindings = BTreeMap::new();
    let input = NodeId(0);
    let output = NodeId(u64::MAX);
    let mut auxiliary = u64::MAX - 1;
    let mut next = || {
        let id = NodeId(auxiliary);
        auxiliary -= 1;
        id
    };
    graph.nodes.insert(input, NodeKind::Input(Channels::Stereo));
    graph
        .nodes
        .insert(output, NodeKind::Output(Channels::Stereo));
    let mut previous = input;
    for stage in stages(cards) {
        let mut ends = Vec::new();
        for card in stage {
            let mut branch = previous;
            if card.inputs != Channels::Stereo {
                let id = next();
                graph.nodes.insert(
                    id,
                    NodeKind::ChannelMap {
                        inputs: Channels::Stereo,
                        outputs: card.inputs,
                    },
                );
                graph.connections.push(Connection {
                    from: branch,
                    to: id,
                });
                branch = id;
            }
            graph.nodes.insert(
                card.id,
                NodeKind::Plugin {
                    inputs: card.inputs,
                    outputs: card.outputs,
                },
            );
            graph.connections.push(Connection {
                from: branch,
                to: card.id,
            });
            bindings.insert(card.id, card.binding.clone());
            branch = card.id;
            if card.outputs != Channels::Stereo {
                let id = next();
                graph.nodes.insert(
                    id,
                    NodeKind::ChannelMap {
                        inputs: card.outputs,
                        outputs: Channels::Stereo,
                    },
                );
                graph.connections.push(Connection {
                    from: branch,
                    to: id,
                });
                branch = id;
            }
            ends.push(branch);
        }
        previous = if ends.len() == 1 {
            ends[0]
        } else {
            let id = next();
            graph.nodes.insert(id, NodeKind::Mixer(Channels::Stereo));
            for end in ends {
                graph.connections.push(Connection { from: end, to: id });
            }
            id
        };
    }
    graph.connections.push(Connection {
        from: previous,
        to: output,
    });
    (graph, bindings)
}
#[derive(Default)]
pub struct Levels {
    pub input: AtomicU32,
    pub output: AtomicU32,
    pub input_peak: AtomicU32,
    pub output_peak: AtomicU32,
    pub updated: AtomicU64,
}
struct Action {
    settings: Settings,
    cards: Vec<Card>,
    reply: Option<Sender<Result<(), String>>>,
    rollback: bool,
}
type OpenResult = Result<Option<(DeviceStreams, Endpoints, Settings)>, String>;
struct Opening {
    result: Receiver<OpenResult>,
    action: Action,
}
pub struct Runtime {
    pub cards: Vec<Card>,
    pub applied: Settings,
    pub message: String,
    pub levels: Arc<Levels>,
    pub(crate) session: Option<LiveSession>,
    pub(crate) editor_geometry: BTreeMap<NodeId, crate::EditorGeometry>,
    last_save: Instant,
    pub(crate) persistence_path: Option<PathBuf>,
    saved_plugins: Vec<crate::SavedPlugin>,
    worker: Option<JoinHandle<Endpoints>>,
    io: Option<DeviceStreams>,
    action: Option<Action>,
    opening: Option<Opening>,
    states: BTreeMap<NodeId, Vec<u8>>,
    counters: (u64, u64),
    last_audio: Instant,
    healthy_since: Instant,
    retries: u8,
    retry: Option<Instant>,
    pub closing: bool,
    diagnostic: Option<String>,
    halted: bool,
}
impl Runtime {
    pub fn new(settings: Settings) -> Self {
        let mut runtime = Self {
            cards: Vec::new(),
            applied: Settings::default(),
            message: String::new(),
            levels: Arc::default(),
            session: None,
            editor_geometry: settings
                .preferences
                .plugins
                .iter()
                .filter_map(|p| p.editor.map(|g| (NodeId(p.id), g)))
                .collect(),
            saved_plugins: settings.preferences.plugins.clone(),
            last_save: Instant::now(),
            persistence_path: None,
            worker: None,
            io: None,
            action: None,
            opening: None,
            states: BTreeMap::new(),
            counters: (0, 0),
            last_audio: Instant::now(),
            healthy_since: Instant::now(),
            retries: 0,
            retry: None,
            closing: false,
            diagnostic: None,
            halted: false,
        };
        runtime.action = Some(Action {
            settings,
            cards: Vec::new(),
            reply: None,
            rollback: false,
        });
        runtime
    }
    pub fn audio_running(&self) -> bool {
        self.io.is_some() && self.worker.is_some()
    }
    pub fn busy(&self) -> bool {
        self.action.is_some() || self.opening.is_some() || self.closing
    }
    pub fn apply(&mut self, settings: Settings) -> Result<Receiver<Result<(), String>>, String> {
        if self.busy() {
            return Err("Audio configuration is already changing".into());
        }
        let (reply, receiver) = bounded(1);
        self.retries = 0;
        self.retry = None;
        self.diagnostic = None;
        self.halted = false;
        self.begin(Action {
            settings,
            cards: self.cards.clone(),
            reply: Some(reply),
            rollback: false,
        });
        Ok(receiver)
    }
    pub fn edit(&mut self, cards: Vec<Card>) {
        if !self.busy() {
            self.halted = false;
            self.retries = 0;
            self.begin(Action {
                settings: self.applied.clone(),
                cards,
                reply: None,
                rollback: false,
            });
        }
    }
    fn begin(&mut self, action: Action) {
        if let Some(session) = &mut self.session {
            session.close_editors();
        }
        self.message = "Stopping audio…".into();
        self.retry = None;
        if let Some(session) = &self.session {
            session.request_stop();
        }
        self.action = Some(action);
    }
    pub fn close(&mut self) {
        if let Some(session) = &mut self.session {
            session.close_editors();
        }
        self.closing = true;
        self.retry = None;
        if let Some(session) = &self.session {
            session.request_stop();
        }
    }
    pub fn stopped(&self) -> bool {
        self.worker.is_none() && self.opening.is_none() && self.io.is_none()
    }
    pub fn tick(&mut self) {
        let mut native_error = None;
        if let Some(session) = &mut self.session
            && let Err(error) = session.service()
        {
            native_error = Some(error);
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            if let Some(worker) = self.worker.take()
                && worker.join().is_err()
            {
                native_error = Some("Audio DSP worker panicked".into());
            }
            if let Some(session) = &mut self.session {
                if let Err(error) = session.service() {
                    native_error = Some(error);
                }
                match session.snapshot() {
                    Ok(states) => self.states.extend(states),
                    Err(error) => native_error = Some(error),
                }
            }
            self.session = None;
            if self.action.is_none() && !self.closing && !self.halted {
                self.recover(
                    native_error.unwrap_or_else(|| "Plugin requested an audio restart".into()),
                );
            }
        }
        if self.closing {
            if self.worker.is_none() {
                self.persist();
                self.session = None;
                self.action = None;
                if let Some(io) = self.io.take() {
                    std::thread::spawn(move || drop(io));
                }
            }
            if let Some(opening) = &self.opening {
                match opening.result.try_recv() {
                    Ok(result) => {
                        drop(result);
                        self.opening = None;
                    }
                    Err(crossbeam::channel::TryRecvError::Disconnected) => self.opening = None,
                    _ => {}
                }
            }
            return;
        }
        if let Some(io) = &self.io {
            let counters = (
                io.counters.input.load(Ordering::Acquire),
                io.counters.output.load(Ordering::Acquire),
            );
            if counters.0 != self.counters.0 && counters.1 != self.counters.1 {
                self.last_audio = Instant::now();
                self.counters = counters;
            }
            let error = io.failure().or_else(|| {
                (self.last_audio.elapsed() > Duration::from_secs(3))
                    .then(|| "Audio devices stopped delivering input/output callbacks".into())
            });
            if let Some(error) = error
                && self.action.is_none()
                && self.opening.is_none()
            {
                self.recover(error);
            }
            if self.healthy_since.elapsed() > Duration::from_secs(10)
                && self.last_audio.elapsed() < Duration::from_secs(1)
            {
                self.retries = 0;
            }
        }
        if let Some(deadline) = self.retry
            && Instant::now() >= deadline
            && !self.busy()
        {
            self.retry = None;
            self.begin(Action {
                settings: self.applied.clone(),
                cards: self.cards.clone(),
                reply: None,
                rollback: true,
            });
        }
        if self.worker.is_none()
            && self.opening.is_none()
            && let Some(action) = self.action.take()
        {
            if let Some(session) = &mut self.session {
                let _ = session.service();
                if let Ok(states) = session.snapshot() {
                    self.states.extend(states);
                }
            }
            self.session = None;
            let old = self.io.take();
            let settings = action.settings.clone();
            let (sender, result) = bounded(1);
            self.message = "Opening audio devices…".into();
            std::thread::spawn(move || {
                drop(old);
                let result = (|| {
                    let backend = settings.backend.as_deref().unwrap_or("dummy");
                    if backend == "dummy" {
                        return Ok(None);
                    }
                    let snapshot = nice_common::devices::snapshot(
                        backend,
                        settings.input_device.as_deref(),
                        settings.output_device.as_deref(),
                    );
                    let resolved = snapshot.resolve_settings(&settings)?;
                    let (io, endpoints) = nice_common::audio_io::open(&resolved)?;
                    Ok(Some((io, endpoints, resolved)))
                })();
                let _ = sender.send(result);
            });
            self.opening = Some(Opening { result, action });
        }
        let result = self
            .opening
            .as_ref()
            .and_then(|opening| match opening.result.try_recv() {
                Ok(result) => Some(result),
                Err(crossbeam::channel::TryRecvError::Disconnected) => {
                    Some(Err("Audio setup worker stopped".into()))
                }
                Err(crossbeam::channel::TryRecvError::Empty) => None,
            });
        if let Some(result) = result {
            let opening = self.opening.take().unwrap();
            let prepared = result.and_then(|opened| {
                if let Some((io, endpoints, resolved)) = opened {
                    let (graph, bindings) = chain(&opening.action.cards);
                    let (session, audio) = LiveSession::prepare(
                        &graph,
                        &bindings,
                        resolved.sample_rate.unwrap() as f64,
                        BLOCK,
                        &self.states,
                    )?;
                    self.worker = Some(spawn_dsp(audio, endpoints, self.levels.clone()));
                    self.session = Some(session);
                    self.io = Some(io);
                    self.last_audio = Instant::now();
                    self.healthy_since = Instant::now();
                    self.counters = (0, 0);
                } else {
                    // Retain inactive instances for controls even with audio off.
                    let (graph, bindings) = chain(&opening.action.cards);
                    let (mut session, mut audio) =
                        LiveSession::prepare(&graph, &bindings, 48000.0, BLOCK, &self.states)?;
                    session.request_stop();
                    std::thread::spawn(move || {
                        let _ = audio.process(&[], &mut [], 0);
                    })
                    .join()
                    .map_err(|_| "Plugin retirement worker panicked")?;
                    session.service()?;
                    self.session = Some(session);
                }
                Ok(())
            });
            match prepared {
                Ok(()) => {
                    self.applied = opening.action.settings;
                    self.cards = opening.action.cards;
                    self.states
                        .retain(|id, _| self.cards.iter().any(|card| card.id == *id));
                    self.message = if self.io.is_some() {
                        "Audio running".into()
                    } else {
                        "Audio off — choose a backend in settings".into()
                    };
                    if opening.action.rollback
                        && let Some(error) = &self.diagnostic
                    {
                        self.message = format!("{} — {error}", self.message);
                    }
                    if let Some(reply) = opening.action.reply {
                        let _ = reply.send(Ok(()));
                    }
                }
                Err(error) => {
                    self.message = error.clone();
                    self.diagnostic = Some(error.clone());
                    if let Some(reply) = opening.action.reply {
                        let _ = reply.send(Err(error));
                    }
                    if !opening.action.rollback {
                        self.begin(Action {
                            settings: self.applied.clone(),
                            cards: self.cards.clone(),
                            reply: None,
                            rollback: true,
                        });
                    } else if self.retries < 3 {
                        self.retries += 1;
                        self.retry = Some(Instant::now() + Duration::from_secs(1));
                    }
                }
            }
        }
        if !self.busy() && self.last_save.elapsed() > Duration::from_secs(2) {
            self.persist();
        }
    }
    /// Restore records through the same trusted entry loader used for file drops.
    pub(crate) fn restore(&mut self, cards: Vec<Card>, states: BTreeMap<NodeId, Vec<u8>>) {
        self.states = states;
        if let Some(action) = &mut self.action {
            action.cards = cards;
        }
    }
    pub(crate) fn persist(&mut self) {
        if self.action.is_some() || self.opening.is_some() {
            return;
        }
        let Some(path) = self.persistence_path.clone() else {
            return;
        };
        if let Some(session) = &mut self.session {
            match session.save_states() {
                Ok(states) => self.states.extend(states),
                Err(error) => {
                    self.message = error;
                    return;
                }
            }
        }
        let plugins: Vec<_> = self
            .cards
            .iter()
            .map(|card| crate::SavedPlugin {
                id: card.id.0,
                stage: card.stage,
                path: card.path.clone(),
                class: card.binding.class.clone(),
                name: card.name.clone(),
                state: self.states.get(&card.id).cloned(),
                editor: self.editor_geometry.get(&card.id).copied(),
            })
            .collect();
        if plugins != self.saved_plugins {
            let (mut settings, warning) = Settings::load_from(&path);
            if let Some(warning) = warning {
                self.message = warning;
                return;
            }
            settings.preferences.plugins = plugins.clone();
            match settings.save_to(&path) {
                Ok(()) => {
                    self.applied.preferences.plugins = plugins.clone();
                    self.saved_plugins = plugins;
                }
                Err(error) => self.message = error,
            }
        }
        self.last_save = Instant::now();
    }
    fn recover(&mut self, error: String) {
        self.message = error.clone();
        self.diagnostic = Some(error);
        if self.retries >= 3 {
            self.close_audio_only();
            return;
        }
        self.retries += 1;
        self.begin(Action {
            settings: self.applied.clone(),
            cards: self.cards.clone(),
            reply: None,
            rollback: true,
        });
    }
    fn close_audio_only(&mut self) {
        self.halted = true;
        if let Some(session) = &self.session {
            session.request_stop();
        }
        if let Some(io) = self.io.take() {
            std::thread::spawn(move || drop(io));
        }
        self.levels.input.store(0, Ordering::Relaxed);
        self.levels.output.store(0, Ordering::Relaxed);
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(session) = &self.session {
            session.request_stop();
        }
        // The DSP worker does not depend on device callbacks, so it will wake,
        // stop and return its graph even after both streams have died.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(session) = &mut self.session {
            let _ = session.service();
        }
    }
}
/// Start the device-independent DSP owner. The matching main-thread session
/// must be serviced and stopped before joining this worker.
pub fn spawn_dsp(
    mut audio: LiveAudio,
    mut io: Endpoints,
    levels: Arc<Levels>,
) -> JoinHandle<Endpoints> {
    std::thread::spawn(move || {
        let mut left = [0.0; BLOCK];
        let mut right = [0.0; BLOCK];
        let mut out_left = [0.0; BLOCK];
        let mut out_right = [0.0; BLOCK];
        loop {
            let count = io.input.slots().min(BLOCK);
            let mut energy = 0.0f64;
            let mut input_peak = 0.0f32;
            let mut output_peak = 0.0f32;
            for frame in 0..count {
                let samples = io.input.pop().unwrap();
                left[frame] = samples[0];
                right[frame] = samples[1];
                energy += f64::from(samples[0]).powi(2);
                input_peak = input_peak.max(samples[0].abs()).max(samples[1].abs());
            }
            let status = audio.process(
                &[&left[..count], &right[..count]],
                &mut [&mut out_left[..count], &mut out_right[..count]],
                count,
            );
            if status == Ok(LiveStatus::Stopped) {
                break;
            }
            if count == 0 {
                std::thread::park_timeout(Duration::from_micros(250));
                continue;
            }
            if status.is_err() {
                continue;
            }
            let mut output_energy = 0.0f64;
            for frame in 0..count {
                output_energy += f64::from(out_left[frame])
                    .powi(2)
                    .max(f64::from(out_right[frame]).powi(2));
                output_peak = output_peak
                    .max(out_left[frame].abs())
                    .max(out_right[frame].abs());
                let _ = io.output.push(nice_common::resample::StereoFrame([
                    out_left[frame],
                    out_right[frame],
                ]));
            }
            levels
                .input_peak
                .store(input_peak.to_bits(), Ordering::Relaxed);
            levels
                .output_peak
                .store(output_peak.to_bits(), Ordering::Relaxed);
            levels.updated.store(crate::now_ms(), Ordering::Release);
            levels.input.store(
                ((energy / count as f64).sqrt() as f32).to_bits(),
                Ordering::Relaxed,
            );
            levels.output.store(
                ((output_energy / count as f64).sqrt() as f32).to_bits(),
                Ordering::Relaxed,
            );
        }
        levels.input.store(0, Ordering::Relaxed);
        levels.output.store(0, Ordering::Relaxed);
        io
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn endpoints() -> (
        Endpoints,
        rtrb::Producer<[f32; 2]>,
        rtrb::Consumer<nice_common::resample::StereoFrame>,
    ) {
        let (capture, input) = rtrb::RingBuffer::new(64);
        let (output, consumer) = rtrb::RingBuffer::new(64);
        (Endpoints { input, output }, capture, consumer)
    }
    #[test]
    fn dsp_stops_and_returns_its_graph_when_no_device_callback_ever_arrives() {
        let (graph, bindings) = chain(&[]);
        let (mut session, audio) =
            LiveSession::prepare(&graph, &bindings, 48000.0, BLOCK, &BTreeMap::new()).unwrap();
        let (io, _capture, _output) = endpoints();
        let worker = spawn_dsp(audio, io, Arc::default());
        session.request_stop();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !worker.is_finished() {
            assert!(
                Instant::now() < deadline,
                "DSP did not stop independently of device callbacks"
            );
            std::thread::yield_now();
        }
        let _ = worker.join().unwrap();
        assert_eq!(session.service().unwrap(), LiveStatus::Stopped);
        assert!(session.snapshot().unwrap().is_empty());
    }
    #[test]
    fn dsp_preserves_stereo_and_retires_after_capture_stops() {
        let (graph, bindings) = chain(&[]);
        let (mut session, audio) =
            LiveSession::prepare(&graph, &bindings, 48000.0, BLOCK, &BTreeMap::new()).unwrap();
        let (io, mut capture, mut output) = endpoints();
        for sample in [[0.25, -0.5], [0.75, -1.0]] {
            capture.push(sample).unwrap();
        }
        let worker = spawn_dsp(audio, io, Arc::default());
        let deadline = Instant::now() + Duration::from_secs(2);
        while output.slots() < 2 {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        session.request_stop();
        let _ = worker.join().unwrap();
        assert_eq!(session.service().unwrap(), LiveStatus::Stopped);
        assert_eq!(output.pop().unwrap().0, [0.25, -0.5]);
        assert_eq!(output.pop().unwrap().0, [0.75, -1.0]);
    }
    #[test]
    fn closing_before_audio_start_cancels_pending_open() {
        let mut runtime = Runtime::new(Settings::default());
        runtime.close();
        runtime.tick();
        assert!(runtime.stopped());
        assert!(!runtime.audio_running());
    }
    #[test]
    fn failed_apply_reports_the_error_and_restores_the_previous_configuration() {
        let mut runtime = Runtime::new(Settings::default());
        runtime.action = None;
        let previous = runtime.applied.clone();
        let (reply, answer) = bounded(1);
        let (send, result) = bounded(1);
        let wanted = Settings {
            backend: Some("core-audio".into()),
            ..Default::default()
        };
        runtime.opening = Some(Opening {
            result,
            action: Action {
                settings: wanted,
                cards: Vec::new(),
                reply: Some(reply),
                rollback: false,
            },
        });
        send.send(Err("Input device 'Missing mic': device disconnected".into()))
            .unwrap();
        runtime.tick();
        assert!(
            answer
                .recv()
                .unwrap()
                .unwrap_err()
                .contains("device disconnected")
        );
        assert_eq!(runtime.applied, previous);
        assert!(runtime.action.as_ref().unwrap().rollback);
        let deadline = Instant::now() + Duration::from_secs(2);
        while runtime.busy() {
            runtime.tick();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(runtime.applied, previous);
        assert!(runtime.message.contains("device disconnected"));
    }

    #[test]
    fn recovery_retries_are_bounded_and_keep_the_failure_text() {
        let mut runtime = Runtime::new(Settings::default());
        runtime.action = None;
        runtime.retries = 3;
        runtime.recover("Device sample rate changed".into());
        assert!(runtime.action.is_none());
        assert!(runtime.retry.is_none());
        assert_eq!(runtime.message, "Device sample rate changed");
    }
}
