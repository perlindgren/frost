use super::*;
use nice_rack_engine::audio::{AudioBuffer, ProcessError, Processor};
use std::{
    io::{Read, Write},
    process::{Child, Stdio},
    thread,
};

#[derive(Clone)]
pub struct RemoteEntry {
    path: PathBuf,
    executable: PathBuf,
    classes: Arc<Vec<Class>>,
}
impl RemoteEntry {
    pub fn inspect(path: &Path, executable: &Path) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let config = Config {
            path: path.clone(),
            class: None,
            sample_rate: 48000.0,
            max_frames: 64,
            inputs: 0,
            outputs: 1,
            state: None,
            memory: None,
        };
        let (_client, ready) = Client::start(executable, config, None)?;
        let Reply::Ready { classes, .. } = ready else {
            return Err("Invalid helper metadata reply".into());
        };
        Ok(Self {
            path,
            executable: executable.to_owned(),
            classes: Arc::new(classes),
        })
    }
    pub fn descriptors(&self) -> Vec<Descriptor> {
        self.classes.iter().map(|c| c.descriptor.clone()).collect()
    }
    pub fn main_channels(
        &self,
        id: &str,
    ) -> Result<(nice_rack_engine::Channels, nice_rack_engine::Channels), String> {
        let c = self
            .classes
            .iter()
            .find(|c| c.descriptor.id == id)
            .ok_or("Plugin class not found")?;
        Ok((
            nice_rack_engine::Channels::new(c.inputs).ok_or("Invalid helper input channels")?,
            nice_rack_engine::Channels::new(c.outputs).ok_or("Invalid helper output channels")?,
        ))
    }
    #[allow(unsafe_code)]
    pub fn prepare(
        &self,
        class: &str,
        rate: f64,
        max: usize,
        inputs: usize,
        outputs: usize,
        state: Option<&[u8]>,
    ) -> Result<(RemoteOwner, RemoteProcessor), String> {
        let layout = Layout::new(inputs, outputs, max).map_err(|e| e.to_string())?;
        let scratch = Scratch::new()?;
        let path = scratch.0.join("audio.map");
        let memory = Arc::new(
            // SAFETY: private creator-owned directory, trusted protocol peer, no
            // endpoint truncates or performs non-atomic writes while mapped.
            unsafe { AudioMemory::create(&path, layout) }.map_err(|e| e.to_string())?,
        );
        let config = Config {
            path: self.path.clone(),
            class: Some(class.into()),
            sample_rate: rate,
            max_frames: max,
            inputs,
            outputs,
            state: state.map(<[u8]>::to_vec),
            memory: Some(path),
        };
        let (mut client, ready) = Client::start(&self.executable, config, Some(memory.clone()))?;
        client.scratch = Some(scratch);
        let Reply::Ready {
            latency,
            parameters,
            state,
            editor,
            ..
        } = ready
        else {
            return Err("Invalid helper activation reply".into());
        };
        let processor = RemoteProcessor {
            memory: memory.clone(),
            wake: client
                .audio_wake
                .clone()
                .ok_or("Missing helper audio transport")?,
            sequence: 0,
            pending: None,
            rate,
            latency,
        };
        Ok((
            RemoteOwner {
                client,
                entry: self.path.clone(),
                class: class.into(),
                rate,
                max,
                layout,
                latency,
                parameters,
                state,
                dirty: true,
                editor_supported: editor,
                editor_open: false,
                geometry: None,
                failure: None,
                editor_error: None,
                closed: false,
                save: None,
                snapshot_ready: false,
                next_save: 0,
            },
            processor,
        ))
    }
}
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, String> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        for _ in 0..100 {
            let dir = std::env::temp_dir().join(format!(
                "nice-rack-ipc-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            let mut builder = builder;
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("Could not create helper IPC directory".into())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
// Kill on every partial-startup error, including failure to start a transport
// thread. Normal teardown transfers this guard to the asynchronous reaper.
struct ChildGuard(Option<Child>);
impl std::ops::Deref for ChildGuard {
    type Target = Child;
    fn deref(&self) -> &Child {
        self.0.as_ref().unwrap()
    }
}
impl std::ops::DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut Child {
        self.0.as_mut().unwrap()
    }
}
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
        }
    }
}
static REAPERS: std::sync::Mutex<Vec<thread::JoinHandle<()>>> = std::sync::Mutex::new(Vec::new());
fn collect_reapers() {
    let mut reapers = REAPERS.lock().unwrap_or_else(|e| e.into_inner());
    let mut i = 0;
    while i < reapers.len() {
        if reapers[i].is_finished() {
            let _ = reapers.swap_remove(i).join();
        } else {
            i += 1;
        }
    }
}
/// Call after the rack event loop returns, before the executable exits. Reaping
/// remains asynchronous during UI operation; exit gives all reapers a shared
/// three-second grace period, rather than abandoning them with live children.
pub fn finish_shutdown() {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        collect_reapers();
        if REAPERS.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
            || Instant::now() >= deadline
        {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
}
struct Client {
    commands: SyncSender<Command>,
    replies: Receiver<Reply>,
    child: Option<ChildGuard>,
    memory: Option<Arc<AudioMemory>>,
    audio_wake: Option<thread::Thread>,
    scratch: Option<Scratch>,
}
impl Client {
    fn start(
        executable: &Path,
        config: Config,
        memory: Option<Arc<AudioMemory>>,
    ) -> Result<(Self, Reply), String> {
        collect_reapers();
        use std::hash::{BuildHasher, Hasher};
        let listener =
            TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let mut hash = std::collections::hash_map::RandomState::new().build_hasher();
        hash.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        );
        let token = format!("{:016x}", hash.finish());
        let mut command = std::process::Command::new(executable);
        command
            .arg("--clap-helper")
            .arg(
                listener
                    .local_addr()
                    .map_err(|e| e.to_string())?
                    .to_string(),
            )
            .arg(&token)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: helpers own GUI windows, not separate consoles.
            command.creation_flags(0x08000000);
        }
        let mut child = ChildGuard(Some(command.spawn().map_err(|e| e.to_string())?));
        let result = (|| {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut stream = accept(&listener, &mut child, deadline)?;
            stream.set_nodelay(true).map_err(|e| e.to_string())?;
            stream
                .set_read_timeout(Some(Duration::from_secs(15)))
                .map_err(|e| e.to_string())?;
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .map_err(|e| e.to_string())?;
            wire::send(
                &mut stream,
                &Command::Hello {
                    version: wire::PROTOCOL,
                    token: token.clone(),
                    config,
                },
            )?;
            let ready: Reply = wire::receive(&mut stream)?;
            if let Reply::Error(error) = ready {
                return Err(error);
            }
            let audio = if memory.is_some() {
                Some(accept(&listener, &mut child, deadline)?)
            } else {
                None
            };
            Ok((stream, audio, ready))
        })();
        let (stream, audio, ready) = match result {
            Ok(v) => v,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        stream.set_read_timeout(None).map_err(|e| e.to_string())?;
        let (commands, incoming) = mpsc::sync_channel(32);
        let (responses, replies) = mpsc::sync_channel(32);
        let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
        let mut reader = stream;
        thread::Builder::new()
            .name("rack-helper-control-send".into())
            .spawn(move || {
                while let Ok(cmd) = incoming.recv() {
                    if wire::send(&mut writer, &cmd).is_err() {
                        break;
                    }
                }
                let _ = writer.shutdown(std::net::Shutdown::Both);
            })
            .map_err(|e| e.to_string())?;
        thread::Builder::new()
            .name("rack-helper-control-recv".into())
            .spawn(move || {
                while let Ok(reply) = wire::receive(&mut reader) {
                    if responses.send(reply).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let audio_wake = if let Some(mut socket) = audio {
            socket.set_nodelay(true).map_err(|e| e.to_string())?;
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .map_err(|e| e.to_string())?;
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .map_err(|e| e.to_string())?;
            let map = memory.as_ref().unwrap().clone();
            let (started, ready) = mpsc::sync_channel(1);
            let worker = thread::Builder::new()
                .name("rack-helper-audio-bridge".into())
                .spawn(move || {
                    let mut last = 0;
                    let _ = started.send(());
                    loop {
                        if map.stopping() || map.failed() {
                            let _ = socket.write_all(&[0]);
                            break;
                        }
                        let request = map.requested();
                        if request == last {
                            thread::park_timeout(Duration::from_millis(50));
                            continue;
                        }
                        if socket
                            .write_all(&[1])
                            .and_then(|()| socket.read_exact(&mut [0]))
                            .is_err()
                        {
                            map.fail();
                            break;
                        }
                        last = request;
                    }
                })
                .map_err(|e| e.to_string())?;
            if ready.recv_timeout(Duration::from_secs(2)).is_err() {
                memory.as_ref().unwrap().fail();
                worker.thread().unpark();
                return Err("Helper audio bridge did not become ready".into());
            }
            Some(worker.thread().clone())
        } else {
            None
        };
        Ok((
            Self {
                commands,
                replies,
                child: Some(child),
                memory,
                audio_wake,
                scratch: None,
            },
            ready,
        ))
    }
    fn send(&self, command: Command) -> Result<(), String> {
        self.commands
            .try_send(command)
            .map_err(|_| "Helper control queue is busy or disconnected".into())
    }
}
fn accept(
    listener: &TcpListener,
    child: &mut Child,
    deadline: Instant,
) -> Result<TcpStream, String> {
    loop {
        match listener.accept() {
            Ok((stream, addr)) if addr.ip().is_loopback() => {
                // Darwin inherits O_NONBLOCK from the listening socket.
                // Timeouts below require a blocking transport socket.
                stream.set_nonblocking(false).map_err(|e| e.to_string())?;
                return Ok(stream);
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.to_string()),
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!("Plugin helper exited during setup: {status}"));
        }
        if Instant::now() >= deadline {
            return Err("Plugin helper startup timed out".into());
        }
        thread::sleep(Duration::from_millis(2));
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.send(Command::Shutdown);
        if let Some(map) = &self.memory {
            map.stop()
        }
        if let Some(wake) = &self.audio_wake {
            wake.unpark()
        }
        if let Some(mut child) = self.child.take() {
            let scratch = self.scratch.take();
            let reaper = thread::Builder::new()
                .name("rack-helper-reaper".into())
                .spawn(move || {
                    let deadline = Instant::now() + Duration::from_secs(2);
                    loop {
                        match child.try_wait() {
                            Ok(Some(_)) => break,
                            Err(_) => {
                                let _ = child.kill();
                                let _ = child.wait();
                                break;
                            }
                            _ => {}
                        }
                        if Instant::now() >= deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    drop(scratch);
                });
            if let Ok(reaper) = reaper {
                REAPERS
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(reaper);
            }
        }
    }
}
pub(crate) struct RemoteOwner {
    client: Client,
    entry: PathBuf,
    class: String,
    rate: f64,
    max: usize,
    layout: Layout,
    pub latency: usize,
    pub parameters: Vec<Parameter>,
    pub state: Option<Vec<u8>>,
    pub dirty: bool,
    pub editor_supported: bool,
    pub editor_open: bool,
    pub geometry: Option<Geometry>,
    pub failure: Option<String>,
    pub editor_error: Option<String>,
    pub closed: bool,
    save: Option<(u64, Instant)>,
    snapshot_ready: bool,
    next_save: u64,
}
impl RemoteOwner {
    pub fn reusable(
        &self,
        entry: &RemoteEntry,
        class: &str,
        rate: f64,
        max: usize,
        inputs: usize,
        outputs: usize,
    ) -> bool {
        self.failure.is_none()
            && !self
                .client
                .memory
                .as_ref()
                .is_some_and(|map| map.failed() || map.stopping())
            && self.entry == entry.path
            && self.class == class
            && self.rate == rate
            && self.max == max
            && self.layout.inputs == inputs
            && self.layout.outputs == outputs
    }
    pub fn processor(&mut self) -> RemoteProcessor {
        self.snapshot_ready = false;
        self.client.memory.as_ref().unwrap().request_reset();
        RemoteProcessor {
            memory: self.client.memory.as_ref().unwrap().clone(),
            wake: self.client.audio_wake.as_ref().unwrap().clone(),
            sequence: self.client.memory.as_ref().unwrap().completed(),
            pending: None,
            rate: self.rate,
            latency: self.latency,
        }
    }
    pub fn poll(&mut self) {
        collect_reapers();
        for _ in 0..64 {
            match self.client.replies.try_recv() {
                Ok(Reply::Saved { request, state }) => {
                    if self.save.is_some_and(|(id, _)| id == request) {
                        self.state = state;
                        self.dirty = true;
                        self.save = None;
                        self.snapshot_ready = true;
                    }
                }
                Ok(Reply::State(state)) => {
                    self.state = state;
                    self.dirty = true
                }
                Ok(Reply::Parameters(parameters)) => self.parameters = parameters,
                Ok(Reply::Geometry { position, size }) => {
                    self.geometry = Some(Geometry { position, size })
                }
                Ok(Reply::Opened) => {
                    self.editor_open = true;
                    self.closed = false
                }
                Ok(Reply::Closed) => {
                    self.editor_open = false;
                    self.closed = true
                }
                Ok(Reply::Error(e)) => self.failure = Some(e),
                Ok(Reply::EditorError(error)) => {
                    self.editor_error = Some(error);
                    self.editor_open = false;
                    self.closed = true;
                }
                Ok(_) => {}
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.failure
                        .get_or_insert("Plugin helper disconnected".into());
                    break;
                }
            }
        }
        if self.client.memory.as_ref().is_some_and(|m| m.failed()) {
            self.failure
                .get_or_insert("Plugin helper missed its audio deadline or failed".into());
        }
    }
    pub fn parameter(&mut self, id: u32, value: f64) -> Result<(), String> {
        if !value.is_finite() {
            return Err("Invalid parameter value".into());
        }
        let parameter = self
            .parameters
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("Parameter not found")?;
        if parameter.readonly {
            return Err("Parameter is read only".into());
        }
        if !parameter.min.is_finite() || !parameter.max.is_finite() || parameter.min > parameter.max
        {
            return Err("Plugin reported invalid parameter bounds".into());
        }
        let mut value = value.clamp(parameter.min, parameter.max);
        if parameter.stepped {
            value = value.round();
        }
        self.client.send(Command::Parameter { id, value })?;
        parameter.value = value;
        parameter.text = value.to_string();
        self.snapshot_ready = false;
        Ok(())
    }
    pub fn open(&mut self, geometry: Geometry) -> Result<(), String> {
        self.client.send(Command::Open {
            position: geometry.position,
            size: geometry.size,
        })?;
        self.editor_open = true;
        Ok(())
    }
    pub fn close(&mut self) {
        let _ = self.client.send(Command::Close);
        self.editor_open = false;
    }
    pub fn snapshot_ready(&mut self) -> Result<bool, String> {
        self.poll();
        if self.snapshot_ready {
            return Ok(true);
        }
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if let Some((_, since)) = self.save {
            if since.elapsed() >= Duration::from_secs(2) {
                return Err(
                    "Plugin helper state capture timed out; retaining last confirmed state".into(),
                );
            }
        } else {
            self.next_save = self
                .next_save
                .checked_add(1)
                .ok_or("State request exhausted")?;
            self.client.send(Command::Save(self.next_save))?;
            self.save = Some((self.next_save, Instant::now()));
        }
        Ok(false)
    }
}
pub(crate) struct RemoteProcessor {
    memory: Arc<AudioMemory>,
    wake: thread::Thread,
    sequence: u32,
    pending: Option<(u32, Instant)>,
    rate: f64,
    latency: usize,
}
impl RemoteProcessor {
    fn submit(
        &mut self,
        input: &AudioBuffer,
        frames: usize,
        deadline: Instant,
    ) -> Result<(), ProcessError> {
        if self.pending.is_some() || self.memory.failed() || self.memory.stopping() {
            return Err(ProcessError::ProcessorFailed);
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(ProcessError::ProcessorFailed)?;
        for channel in 0..input.channels() {
            self.memory
                .write_input(channel, input.channel(channel, frames))
        }
        self.memory.publish(self.sequence, frames);
        self.pending = Some((self.sequence, deadline));
        self.wake.unpark();
        Ok(())
    }
    fn finish(&mut self, output: &mut AudioBuffer, frames: usize) -> Result<(), ProcessError> {
        let (sequence, deadline) = self.pending.take().ok_or(ProcessError::ProcessorFailed)?;
        let mut spins = 0;
        while self.memory.completed() != sequence {
            if self.memory.failed() || Instant::now() >= deadline {
                self.memory.fail();
                return Err(ProcessError::ProcessorFailed);
            }
            if spins < 32 {
                std::hint::spin_loop();
                spins += 1
            } else {
                thread::yield_now()
            }
        }
        if self.memory.failed() || Instant::now() >= deadline {
            self.memory.fail();
            return Err(ProcessError::ProcessorFailed);
        }
        for channel in 0..output.channels() {
            self.memory
                .read_output(channel, output.channel_mut(channel, frames))
        }
        Ok(())
    }
}
impl Processor for RemoteProcessor {
    fn is_deferred(&self) -> bool {
        true
    }
    fn reset(&mut self) {
        self.memory.request_reset();
    }
    fn stop(&mut self) {
        if self.pending.take().is_some() {
            self.memory.fail();
        }
    }
    fn begin_process(
        &mut self,
        input: &AudioBuffer,
        _output: &mut AudioBuffer,
        frames: usize,
        deadline: Instant,
    ) -> Result<(), ProcessError> {
        self.submit(input, frames, deadline)
    }
    fn finish_process(
        &mut self,
        output: &mut AudioBuffer,
        frames: usize,
    ) -> Result<(), ProcessError> {
        self.finish(output, frames)
    }
    fn latency_samples(&self) -> usize {
        self.latency
    }
    fn process(
        &mut self,
        input: &AudioBuffer,
        output: &mut AudioBuffer,
        frames: usize,
    ) -> Result<(), ProcessError> {
        self.submit(
            input,
            frames,
            Instant::now() + Duration::from_secs_f64(frames as f64 / self.rate),
        )?;
        self.finish(output, frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> RemoteEntry {
        let helper = PathBuf::from(
            std::env::var_os("NICE_RACK_TEST_HELPER").expect("NICE_RACK_TEST_HELPER"),
        );
        let plugin =
            PathBuf::from(std::env::var_os("NICE_RACK_TEST_CLAP").expect("NICE_RACK_TEST_CLAP"));
        RemoteEntry::inspect(&plugin, &helper).unwrap()
    }
    #[test]
    #[ignore = "requires NICE_RACK_TEST_HELPER and NICE_RACK_TEST_CLAP"]
    fn exited_helper_is_not_reused() {
        let entry = fixture();
        let class = &entry.classes[0];
        let (mut owner, _processor) = entry
            .prepare(
                &class.descriptor.id,
                48000.,
                64,
                class.inputs as usize,
                class.outputs as usize,
                None,
            )
            .unwrap();
        let child = owner.client.child.as_mut().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while owner.failure.is_none() {
            owner.poll();
            assert!(Instant::now() < deadline, "dead helper was not detected");
            thread::sleep(Duration::from_millis(2));
        }
        assert!(!owner.reusable(
            &entry,
            &class.descriptor.id,
            48000.,
            64,
            class.inputs as usize,
            class.outputs as usize
        ));
    }
    #[cfg(unix)]
    #[test]
    #[ignore = "requires NICE_RACK_TEST_HELPER and NICE_RACK_TEST_CLAP; suspends its own helper"]
    fn stalled_helper_times_out_and_late_completion_cannot_revive_it() {
        use nice_rack_engine::{
            Channels, Connection, NodeId, NodeKind, RackGraph,
            audio::{PreparedRack, ProcessorSlot},
        };
        let entry = fixture();
        let class = &entry.classes[0];
        assert_eq!(
            (class.inputs, class.outputs),
            (1, 1),
            "use the mono nice-echo fixture"
        );
        let (mut owner, processor) = entry
            .prepare(&class.descriptor.id, 48000., 64, 1, 1, None)
            .unwrap();
        struct PausedChild(ChildGuard);
        impl Drop for PausedChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let paused_child = PausedChild(owner.client.child.take().unwrap());
        let pid = paused_child.0.id();
        assert!(
            std::process::Command::new("/bin/kill")
                .args(["-STOP", &pid.to_string()])
                .status()
                .unwrap()
                .success()
        );
        let memory = owner.client.memory.as_ref().unwrap().clone();
        let c = Channels::Mono;
        let graph = RackGraph {
            nodes: [
                (NodeId(0), NodeKind::Input(c)),
                (
                    NodeId(1),
                    NodeKind::Plugin {
                        inputs: c,
                        outputs: c,
                    },
                ),
                (NodeId(2), NodeKind::Output(c)),
            ]
            .into(),
            connections: [(0, 1), (1, 2)]
                .map(|(a, b)| Connection {
                    from: NodeId(a),
                    to: NodeId(b),
                })
                .into(),
            ..RackGraph::default()
        };
        let mut rack = PreparedRack::prepare(
            &graph,
            [(
                NodeId(1),
                ProcessorSlot {
                    processor: Box::new(processor),
                    bypassed: false,
                },
            )]
            .into(),
            64,
        )
        .unwrap();
        let mut out = [99.; 64];
        let started = Instant::now();
        let allocations = allocation_counter::measure(|| {
            assert_eq!(
                rack.process(&[&[0.25; 64]], &mut [&mut [&mut out]], 64),
                Err(ProcessError::NodeFailed(NodeId(1)))
            );
        });
        assert_eq!(allocations.count_total, 0);
        assert_eq!(out, [0.; 64]);
        assert!(memory.failed());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "audio waited for a stopped process"
        );
        memory.write_output(0, &[1.; 64]);
        memory.finish(memory.requested());
        assert_eq!(
            rack.process(&[&[0.25; 64]], &mut [&mut [&mut out]], 64),
            Err(ProcessError::NodeFailed(NodeId(1)))
        );
        assert_eq!(
            out, [0.; 64],
            "late output must never leak into another block"
        );
        rack.stop();
        drop(rack);
        drop(paused_child); // reap even a SIGSTOP'd process before test exit
        drop(owner);
        finish_shutdown();
    }
}
