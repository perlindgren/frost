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
        Arc, Mutex,
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
    /// When the thread asking for devices started. A driver that never answers
    /// would otherwise hold `busy()` — and with it every interaction the window
    /// offers, the drop queue first — for the rest of the session.
    since: Instant,
}
/// How long the rack waits for a thread that is opening devices before it says
/// so and lets the window be used again. A driver is entitled to take a moment
/// with a device it has never seen; it is not entitled to hold the application.
const OPENING_LIMIT: Duration = Duration::from_secs(20);
/// Streams nobody is waiting for any more. Devices opened after the rack
/// stopped waiting must not be dropped by the thread that opened them, because
/// releasing a driver from a thread nobody is watching is the fault the
/// deadline exists to stay out of, so they are kept mapped here instead.
static ABANDONED: Mutex<Vec<DeviceStreams>> = Mutex::new(Vec::new());
/// Whether two settings would drive the same streams. Everything counts but the
/// saved plugin list and the window, because neither of those is a device: a
/// different chain in the same window on the same interface is not a reason to
/// stop a driver and start it again.
fn same_devices(one: &Settings, other: &Settings) -> bool {
    one.backend == other.backend
        && one.input_device == other.input_device
        && one.output_device == other.output_device
        && one.jack_output_right == other.jack_output_right
        && one.output_channels == other.output_channels
        && one.sample_rate == other.sample_rate
        && one.output_sample_rate == other.output_sample_rate
        && one.period_size == other.period_size
        && one.output_period_size == other.output_period_size
        && one.input_latency == other.input_latency
        && one.input_channel == other.input_channel
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
    /// The rings of `io` while no worker owns them. `spawn_dsp` returns them
    /// when its owner stops, so an edit that only changes the chain can hand
    /// them to the next one and leave the streams — and the driver behind them
    /// — exactly as they are.
    idle: Option<Endpoints>,
    /// The settings the streams in `io` were opened from, as the resolver
    /// settled them, so a new chain is built at the rate they are running.
    live_settings: Option<Settings>,
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
            idle: None,
            live_settings: None,
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
            if let Some(worker) = self.worker.take() {
                // The rings come back with the owner that stops. Keeping them
                // is what lets the next chain have them without the rack
                // having to ask the device for anything at all.
                match worker.join() {
                    Ok(endpoints) => self.idle = Some(endpoints),
                    Err(_) => native_error = Some("Audio DSP worker panicked".into()),
                }
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
        // A thread that was asked for devices and never comes back holds
        // `busy()`, and every interaction the window offers waits behind it:
        // the dropped files first, then the settings page, then an editor.
        // Twenty seconds is far past any driver that is merely slow — this
        // machine's ASIO driver answers in half of one — and short enough that
        // a wedged one cannot keep the application. The thread is left to
        // itself: its sender has gone, and whatever it opens is parked.
        if self
            .opening
            .as_ref()
            .is_some_and(|opening| opening.since.elapsed() > OPENING_LIMIT)
        {
            let opening = self.opening.take().unwrap();
            let text = format!(
                "Audio off — opening the devices gave up after {} seconds",
                OPENING_LIMIT.as_secs()
            );
            if let Some(reply) = opening.action.reply {
                let _ = reply.send(Err(text.clone()));
            }
            if self.closing {
                self.message = text;
            } else {
                self.recover(text);
            }
        }
        if self.closing {
            if self.worker.is_none() {
                self.persist();
                self.session = None;
                self.action = None;
                if let Some(io) = self.io.take() {
                    let idle = self.idle.take();
                    self.live_settings = None;
                    std::thread::spawn(move || {
                        drop(idle);
                        drop(io);
                    });
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
            // Dropping a plugin onto a rack that is making sound changes the
            // chain, and the transport is not part of the chain: it carries
            // frames between two rings that the DSP owner on this thread feeds
            // and empties. So when the settings this action carries are the
            // ones the running streams were opened from, the streams answer the
            // action themselves. Nothing is stopped, freed or reloaded, which is
            // the point: releasing a driver while a library is being loaded into
            // the same process has this rack faulting inside the module that was
            // released, at one fixed instruction, for every plugin it can load.
            let reuse = self.io.is_some()
                && self.idle.is_some()
                && self.live_settings.is_some()
                && same_devices(&self.applied, &action.settings);
            let (sender, result) = bounded(1);
            if reuse {
                let opened = self
                    .io
                    .take()
                    .zip(self.idle.take())
                    .zip(self.live_settings.take())
                    .map(|((io, idle), resolved)| (io, idle, resolved));
                let _ = sender.send(Ok(opened));
                self.message = "Rebuilding the chain…".into();
            } else {
                let old = self.io.take();
                let idle = self.idle.take();
                self.live_settings = None;
                let settings = action.settings.clone();
                self.message = "Opening audio devices…".into();
                std::thread::spawn(move || {
                    drop(old);
                    drop(idle);
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
                    if let Err(crossbeam::channel::SendError(Ok(Some((io, ..))))) =
                        sender.send(result)
                    {
                        // The rack stopped waiting and took the receiver with it.
                        // Whatever this thread went on to open is not its to hand
                        // back, and dropping the streams here would release a driver
                        // from a thread nobody is watching, so they are kept instead.
                        ABANDONED
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .push(io);
                    }
                });
            }
            self.opening = Some(Opening {
                result,
                action,
                since: Instant::now(),
            });
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
                let Some((io, endpoints, resolved)) = opened else {
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
                    return Ok(());
                };
                let rate = resolved.sample_rate.unwrap() as f64;
                let (graph, bindings) = chain(&opening.action.cards);
                let (session, audio) =
                    match LiveSession::prepare(&graph, &bindings, rate, BLOCK, &self.states) {
                        Ok(built) => built,
                        // The transport is not what failed. Handing it back rather
                        // than letting it go here is the same release the rack
                        // avoids everywhere else, and the rollback two lines below
                        // is about to ask for those devices again.
                        Err(error) => {
                            self.io = Some(io);
                            self.idle = Some(endpoints);
                            self.live_settings = Some(resolved);
                            return Err(error);
                        }
                    };
                self.worker = Some(spawn_dsp(audio, endpoints, self.levels.clone()));
                self.session = Some(session);
                self.io = Some(io);
                self.live_settings = Some(resolved);
                self.last_audio = Instant::now();
                self.healthy_since = Instant::now();
                self.counters = (0, 0);
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
            let idle = self.idle.take();
            self.live_settings = None;
            std::thread::spawn(move || {
                drop(idle);
                drop(io);
            });
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
            since: Instant::now(),
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

    /// What a cold rack does with the first dropped file, with no settings
    /// file to give it a backend. Nothing was ever chosen, so nothing ever
    /// ran: the chain is built, stopped again before its first block, and
    /// then written out by the automatic save two seconds later, which asks
    /// instances that never saw a callback for their state. Every step of
    /// that has to end in a message rather than a dead process, because it
    /// is what a first run does the moment a plugin is dropped on it. A
    /// second drop then asks the same cold controller to retire the session
    /// it built for the first, which is the first time anything inactive is
    /// actually taken down.
    ///
    /// `NICE_RACK_TEST_CLAP_2` names the second export; unset, the first
    /// stands in. Load `Nice NAM.clap` among the two and the run can die at
    /// the very end, after every assertion here passed, with an access
    /// violation no harness can report: the Windows application log names the
    /// faulting module `Nice NAM.clap_unloaded`, code executing out of a
    /// library the loader had already unmapped. `nice-nam/src/engine.rs`
    /// spawns a detached thread per instance which sleeps twenty
    /// milliseconds a lap and is never joined, so whether that thread is
    /// still inside the module when its last card releases the library is a
    /// race, and which of two orders loses it moves with the drop order. The
    /// rack's own work is what this test witnesses; keeping a thread inside a
    /// module shorter than the thread is the plugin's promise to keep.
    #[test]
    #[ignore = "requires NICE_RACK_TEST_CLAP pointing to a freshly built repository CLAP export"]
    #[allow(unsafe_code)]
    fn a_dropped_plugin_is_saved_before_audio_has_ever_run() {
        let path = PathBuf::from(
            std::env::var_os("NICE_RACK_TEST_CLAP").expect("Set NICE_RACK_TEST_CLAP"),
        );
        // SAFETY: explicit test opt-in to this repository's own plugin export.
        let entry = unsafe { nice_clap_host::Entry::load(&path).unwrap() };
        let class = entry
            .descriptors()
            .unwrap()
            .first()
            .expect("the test export contains at least one plugin")
            .id
            .clone();
        let card = Card::new(
            NodeId(2),
            path.clone(),
            entry,
            class.clone(),
            "Test plugin".into(),
        )
        .unwrap();
        let file = frost::scratch::claim("rack-cold-drop").join("nice-rack-settings.ron");
        let _ = std::fs::remove_file(&file);

        let mut runtime = Runtime::new(Settings::default());
        settle(&mut runtime);
        assert!(!runtime.audio_running());
        assert_eq!(runtime.message, "Audio off — choose a backend in settings");

        runtime.edit(vec![card]);
        settle(&mut runtime);
        assert_eq!(runtime.cards.len(), 1);
        assert!(!runtime.audio_running());

        runtime.persistence_path = Some(file.clone());
        runtime.last_save = Instant::now() - Duration::from_secs(3);
        runtime.tick();
        let (saved, warning) = Settings::load_from(&file);
        assert_eq!(warning, None);
        assert_eq!(saved.preferences.plugins.len(), 1);
        assert_eq!(saved.preferences.plugins[0].class, class);
        assert!(!runtime.busy());

        // The second drop is its own case: the chain that never ran has to be
        // torn down and rebuilt around a second instance, so an inactive
        // session is retired for the first time rather than merely dropped,
        // and a second entry is loaded beside the first. `NICE_RACK_TEST_CLAP_2`
        // names another export; unset, the same file stands in for it.
        let second_path = std::env::var_os("NICE_RACK_TEST_CLAP_2")
            .map(PathBuf::from)
            .unwrap_or_else(|| path.clone());
        // SAFETY: explicit test opt-in to this repository's own plugin export.
        let second_entry = unsafe { nice_clap_host::Entry::load(&second_path).unwrap() };
        let second_class = second_entry
            .descriptors()
            .unwrap()
            .first()
            .expect("the second test export contains at least one plugin")
            .id
            .clone();
        let second = Card::new(
            NodeId(4),
            second_path.clone(),
            second_entry,
            second_class.clone(),
            "Second plugin".into(),
        )
        .unwrap();
        let mut cards = runtime.cards.clone();
        cards.push(second);
        runtime.edit(cards);
        settle(&mut runtime);
        assert_eq!(runtime.cards.len(), 2);
        assert!(!runtime.audio_running());
        runtime.last_save = Instant::now() - Duration::from_secs(3);
        runtime.tick();
        let (saved, warning) = Settings::load_from(&file);
        assert_eq!(warning, None);
        assert_eq!(saved.preferences.plugins.len(), 2);
        assert_eq!(saved.preferences.plugins[1].class, second_class);
        assert!(!runtime.busy());
        runtime.close();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !runtime.stopped() {
            runtime.tick();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    /// A window that stopped answering the moment a second plugin was dropped
    /// on it, with a backend chosen in Settings and the devices therefore
    /// open, is not a crashed program: every interaction the rack offers, the
    /// drop queue first of all, waits for the controller to stop being busy,
    /// so a controller that never settles is a window that answers nothing.
    /// This is the only test in the tree that hands the controller real
    /// devices instead of the dummy backend, and it opens whatever this
    /// machine has, which is why it stays opt-in. What it requires is not
    /// that this machine's audio driver answers, but that the controller
    /// always comes to rest and says why when it does not. The second file is
    /// opened with the first chain already processing, because that is the
    /// moment the report names.
    ///
    /// `NICE_RACK_TEST_LINGER` holds the process open that many milliseconds
    /// after the devices are at rest, because the answer it gives is not about
    /// this test: a run that prints the linger line and exits cleanly released
    /// this machine's ASIO driver safely, and a run that stops during the
    /// linger, after every assertion here passed, died in
    /// `nativeinstrumentsusbaudioasio_x` at an address the loader had already
    /// unmapped — the driver's own thread executing after its library was
    /// released, which nothing on this side can order.
    #[test]
    #[ignore = "opens this machine's real audio devices; needs NICE_RACK_TEST_CLAP"]
    #[allow(unsafe_code)]
    fn a_second_drop_while_the_devices_are_open_settles_again() {
        let path = PathBuf::from(
            std::env::var_os("NICE_RACK_TEST_CLAP").expect("Set NICE_RACK_TEST_CLAP"),
        );
        // SAFETY: explicit test opt-in to this repository's own plugin export.
        let entry = unsafe { nice_clap_host::Entry::load(&path).unwrap() };
        let class = entry
            .descriptors()
            .unwrap()
            .first()
            .expect("the test export contains at least one plugin")
            .id
            .clone();
        let first = Card::new(
            NodeId(2),
            path.clone(),
            entry,
            class.clone(),
            "First plugin".into(),
        )
        .unwrap();
        let second_path = std::env::var_os("NICE_RACK_TEST_CLAP_2")
            .map(PathBuf::from)
            .unwrap_or_else(|| path.clone());

        // The report is about the rack as it is actually run, on the devices
        // that were actually chosen, so the settings come from the file the
        // rack writes.
        let settings = rack_settings();
        let mut runtime = Runtime::new(settings);
        settle(&mut runtime);
        // A driver may refuse this machine for a reason of its own — no input,
        // no such driver — and a refusal is a message, which is the behaviour
        // under test as much as an opening is.
        let opened = runtime.audio_running();
        mark(&format!("devices asked for, audio = {opened}"), &runtime);

        runtime.edit(vec![first]);
        settle(&mut runtime);
        mark("first card applied", &runtime);
        assert_eq!(runtime.cards.len(), 1, "{}", runtime.message);

        // Isolate: a second drop rebuilds the chain, and this asks whether the
        // rebuild alone is enough, with the same one card in it and no second
        // library opened at all. Six of them, because the failure may be one
        // teardown in particular or simply the nth one.
        for lap in 2..=7 {
            let again = runtime.cards.clone();
            runtime.edit(again);
            settle(&mut runtime);
            mark(
                &format!("chain rebuilt with the same card, lap {lap}"),
                &runtime,
            );
        }

        // The report is a drop made while the first chain was already making
        // sound, so this is where the second file is opened: a native library
        // loaded, its plugins asked for, one of them instantiated, all of it
        // with the audio thread inside the first plugin. Opened here, on this
        // thread, because it is where the rack opens a dropped file: the event
        // loop drains the queue on the thread that draws, and a plugin loaded
        // somewhere else is not what the application does.
        // SAFETY: explicit test opt-in to this repository's own plugin export.
        let entry = unsafe { nice_clap_host::Entry::load(&second_path).unwrap() };
        let second_class = entry
            .descriptors()
            .unwrap()
            .first()
            .expect("the second test export contains at least one plugin")
            .id
            .clone();
        let second = Card::new(
            NodeId(4),
            second_path,
            entry,
            second_class,
            "Second plugin".into(),
        )
        .unwrap();
        mark("second plugin opened", &runtime);

        let mut cards = runtime.cards.clone();
        cards.push(second);
        runtime.edit(cards);
        settle(&mut runtime);
        mark("second card applied", &runtime);
        assert_eq!(runtime.cards.len(), 2, "{}", runtime.message);
        if opened {
            assert!(
                runtime.audio_running(),
                "the second drop took the audio down: {}",
                runtime.message
            );
        }

        mark("asking for the devices to close", &runtime);
        runtime.close();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !runtime.stopped() {
            runtime.tick();
            assert!(
                Instant::now() < deadline,
                "the devices never came to rest: {}",
                runtime.message
            );
            std::thread::yield_now();
        }
        mark("devices at rest", &runtime);
        // Where does the process die? The close above hands the streams to a
        // thread which releases the devices, and a driver whose own threads
        // outlive its module faults when it likes, in a library the loader has
        // already unmapped. Lingering tells the two apart from the output
        // alone: a run that stops here died with the driver it had just
        // released, a run that prints the last line died on its way out.
        let linger = std::env::var("NICE_RACK_TEST_LINGER")
            .ok()
            .and_then(|text| text.parse::<u64>().ok())
            .unwrap_or(0);
        if linger > 0 {
            std::thread::sleep(Duration::from_millis(linger));
            println!("  + linger of {linger} ms over");
        }
    }
    /// The control for the case above: the same two plugins, the same devices,
    /// the same rebuilds, with both libraries opened before the devices are
    /// asked for, so no code is loaded while a driver is streaming. If this
    /// passes where the drop-while-running dies, nothing about the plugins is
    /// at fault: it is the act of loading a library into a process whose audio
    /// driver is live that the driver cannot take.
    #[test]
    #[ignore = "opens this machine's real audio devices; needs NICE_RACK_TEST_CLAP"]
    #[allow(unsafe_code)]
    fn two_plugins_opened_before_the_devices_come_up() {
        let first_path = PathBuf::from(
            std::env::var_os("NICE_RACK_TEST_CLAP").expect("Set NICE_RACK_TEST_CLAP"),
        );
        let second_path = std::env::var_os("NICE_RACK_TEST_CLAP_2")
            .map(PathBuf::from)
            .unwrap_or_else(|| first_path.clone());
        let mut runtime = Runtime::new(rack_settings());
        let cards = [first_path, second_path]
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                // SAFETY: explicit test opt-in to this repository's own plugin export.
                let entry = unsafe { nice_clap_host::Entry::load(&path).unwrap() };
                let class = entry
                    .descriptors()
                    .unwrap()
                    .first()
                    .expect("the test export contains at least one plugin")
                    .id
                    .clone();
                Card::new(
                    NodeId(2 + index as u64),
                    path,
                    entry,
                    class,
                    format!("Plugin {}", index + 1),
                )
                .unwrap()
            })
            .collect();
        mark("both plugins opened, devices not asked for", &runtime);

        runtime.edit(cards);
        settle(&mut runtime);
        mark("two cards applied", &runtime);

        let again = runtime.cards.clone();
        runtime.edit(again);
        settle(&mut runtime);
        mark("chain rebuilt with both cards", &runtime);

        runtime.close();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !runtime.stopped() {
            runtime.tick();
            assert!(
                Instant::now() < deadline,
                "the devices never came to rest: {}",
                runtime.message
            );
            std::thread::yield_now();
        }
        mark("devices at rest", &runtime);
    }
    /// The device settings of the rack as this machine runs it.
    /// `NICE_RACK_TEST_SETTINGS` names the file and the
    /// `nice-rack-settings.ron` beside the working directory stands in, because
    /// a hand-written default asks for an input channel this machine may have
    /// no default of, and a controller answering "Select an input channel" has
    /// no chain to test at all. `NICE_RACK_TEST_BACKEND` overrides the backend
    /// and drops the saved device names with it, so that host picks its own
    /// defaults. The saved plugin list goes: the drops in these tests are the
    /// only way a card should reach the rack.
    fn rack_settings() -> Settings {
        let mut settings = match std::env::var_os("NICE_RACK_TEST_SETTINGS") {
            Some(file) => Settings::load_from(&PathBuf::from(file)).0,
            None => Settings::load_from(&Settings::path()).0,
        };
        if let Some(backend) = std::env::var_os("NICE_RACK_TEST_BACKEND") {
            settings.backend = Some(backend.to_string_lossy().into_owned());
            settings.input_device = None;
            settings.output_device = None;
        }
        if settings.backend.is_none() {
            let backend = if cfg!(windows) {
                "wasapi"
            } else if cfg!(target_os = "macos") {
                "core-audio"
            } else {
                "alsa"
            };
            settings.backend = Some(backend.into());
        }
        settings.preferences.plugins.clear();
        settings
    }
    /// Where the run got to and what the controller said there. A process that
    /// dies inside a third-party driver takes its test report down with it, so
    /// the last line left on the console is the only record of the call that
    /// did it; the elapsed time says which step was slow before it was fatal.
    fn mark(step: &str, runtime: &Runtime) {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let start = START.get_or_init(Instant::now);
        eprintln!(
            "  +{:>6} ms  {step}  |  {}",
            start.elapsed().as_millis(),
            runtime.message
        );
    }
    fn settle(runtime: &mut Runtime) {
        // Generous because a real driver is allowed to take its time: the
        // deadline only says how long the test waits before calling a
        // controller that never comes to rest stuck.
        let deadline = Instant::now() + Duration::from_secs(10);
        while runtime.busy() {
            runtime.tick();
            assert!(
                Instant::now() < deadline,
                "the controller never settled: {}",
                runtime.message
            );
            std::thread::yield_now();
        }
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
