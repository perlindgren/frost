use super::*;
use std::{
    io::{Read, Write},
    thread,
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};
const PLUGIN: nice_rack_engine::NodeId = nice_rack_engine::NodeId(1);

/// Run only in a dedicated helper executable mode, on its initial/main thread.
/// # Safety
/// The spawning operator must trust the selected library to obey the CLAP ABI.
/// This contains process crashes; it is not a security boundary against native code.
#[allow(unsafe_code)]
pub unsafe fn run_helper(address: &str, token: &str) -> Result<(), String> {
    let address: std::net::SocketAddr = address
        .parse()
        .map_err(|error| format!("Invalid helper address: {error}"))?;
    if !address.ip().is_loopback() {
        return Err("Helper endpoint must be loopback".into());
    }
    let mut control = TcpStream::connect(address).map_err(|e| e.to_string())?;
    if !control
        .peer_addr()
        .map_err(|e| e.to_string())?
        .ip()
        .is_loopback()
    {
        return Err("Helper endpoint must be loopback".into());
    }
    control.set_nodelay(true).map_err(|e| e.to_string())?;
    control
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(|e| e.to_string())?;
    let Command::Hello {
        version,
        token: received,
        config,
    } = wire::receive(&mut control)?
    else {
        return Err("Missing helper handshake".into());
    };
    if version != wire::PROTOCOL || received != token {
        return Err("Invalid helper protocol/token".into());
    }
    // SAFETY: the executable's explicit --clap-helper trust boundary accepts this
    // operator-selected entry. Only this helper process executes its native code.
    let entry = unsafe { Entry::load(&config.path) }?;
    let mut classes = Vec::new();
    for descriptor in entry.descriptors()?.into_iter().take(16384) {
        if let Ok((inputs, outputs)) = entry.main_channels(&descriptor.id) {
            classes.push(Class {
                descriptor,
                inputs: inputs.count() as u8,
                outputs: outputs.count() as u8,
            });
        }
    }
    let Some(class) = &config.class else {
        wire::send(
            &mut control,
            &Reply::Ready {
                classes,
                latency: 0,
                parameters: Vec::new(),
                state: None,
                editor: false,
            },
        )?;
        return Ok(());
    };
    use nice_rack_engine::{Channels, Connection, NodeId, NodeKind, RackGraph};
    let inputs = Channels::new(config.inputs as u8)
        .filter(|c| c.count() == config.inputs)
        .ok_or("Invalid helper input layout")?;
    let outputs = Channels::new(config.outputs as u8)
        .filter(|c| c.count() == config.outputs)
        .ok_or("Invalid helper output layout")?;
    let graph = RackGraph {
        nodes: [
            (NodeId(0), NodeKind::Input(inputs)),
            (PLUGIN, NodeKind::Plugin { inputs, outputs }),
            (NodeId(2), NodeKind::Output(outputs)),
        ]
        .into(),
        connections: [
            Connection {
                from: NodeId(0),
                to: PLUGIN,
            },
            Connection {
                from: PLUGIN,
                to: NodeId(2),
            },
        ]
        .into(),
        ..RackGraph::default()
    };
    let states = config
        .state
        .as_ref()
        .map(|s| (PLUGIN, s.clone()))
        .into_iter()
        .collect();
    let (mut session, mut audio) = LiveSession::prepare(
        &graph,
        &[(
            PLUGIN,
            Binding {
                entry,
                class: class.clone(),
            },
        )]
        .into(),
        config.sample_rate,
        config.max_frames,
        &states,
    )?;
    let state = session.save_states()?.remove(&PLUGIN);
    let parameters = session.parameters(PLUGIN);
    let editor = session.editor_supported(PLUGIN);
    let layout =
        Layout::new(config.inputs, config.outputs, config.max_frames).map_err(|e| e.to_string())?;
    let memory = Arc::new(
        // SAFETY: the parent owns the private file until this process exits;
        // both protocol endpoints use only atomic access and never truncate it.
        unsafe {
            AudioMemory::open(
                config
                    .memory
                    .as_ref()
                    .ok_or("Missing helper audio mapping")?,
                layout,
            )
        }
        .map_err(|e| e.to_string())?,
    );
    let mut socket = TcpStream::connect(address).map_err(|e| e.to_string())?;
    socket.set_nodelay(true).map_err(|e| e.to_string())?;
    socket
        .set_read_timeout(Some(Duration::from_millis(4)))
        .map_err(|e| e.to_string())?;
    let title = classes
        .iter()
        .find(|c| c.descriptor.id == *class)
        .map(|c| c.descriptor.name.clone())
        .unwrap_or_else(|| class.clone());
    let map = memory.clone();
    let (started, ready) = mpsc::sync_channel(1);
    let worker = thread::Builder::new()
        .name("rack-isolated-dsp".into())
        .spawn(move || {
            let mut input = vec![vec![0.0; layout.max_frames]; layout.inputs];
            let mut output = vec![vec![0.0; layout.max_frames]; layout.outputs];
            let _ = started.send(());
            // Slices and buffers are allocated once, before any requested block.
            // Keep the CLAP graph on this persistent audio thread for its whole lifetime.
            loop {
                if map.stopping() || map.failed() {
                    break;
                }
                let flush = map.flush_requested();
                // Also service plugin-originated request_flush while idle or
                // paused, without waiting for another captured audio block.
                audio.flush_parameters();
                if flush != map.flush_completed() {
                    map.finish_flush(flush);
                }
                let mut command = [0];
                match socket.read_exact(&mut command) {
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                        ) =>
                    {
                        continue;
                    }
                    Err(_) => break,
                    Ok(()) => {}
                }
                if command[0] == 0 || map.stopping() {
                    break;
                }
                let sequence = map.requested();
                let frames = map.frames();
                if map.take_reset() {
                    audio.reset();
                }
                if sequence == 0 || frames == 0 {
                    map.fail();
                    break;
                }
                for (channel, samples) in input.iter_mut().enumerate() {
                    map.read_input(channel, &mut samples[..frames]);
                }
                // Input references are reborrowed below; no allocation in the block loop.
                let mut input_slices: [&[f32]; 64] = [&[]; 64];
                for (slot, samples) in input_slices.iter_mut().zip(&input) {
                    *slot = &samples[..frames];
                }
                let mut output_slices: [&mut [f32]; 64] = std::array::from_fn(|_| &mut [][..]);
                for (slot, samples) in output_slices.iter_mut().zip(&mut output) {
                    *slot = &mut samples[..frames];
                }
                if !matches!(
                    audio.process(
                        &input_slices[..layout.inputs],
                        &mut [&mut output_slices[..layout.outputs]],
                        frames
                    ),
                    Ok(LiveStatus::Running)
                ) {
                    map.fail();
                    break;
                }
                for (channel, samples) in output.iter().enumerate() {
                    map.write_output(channel, &samples[..frames]);
                }
                map.finish(sequence);
                if socket.write_all(&[1]).is_err() {
                    map.fail();
                    break;
                }
            }
            // Dropping LiveAudio cooperatively stops and returns the prepared graph.
            drop(audio);
        })
        .map_err(|e| e.to_string())?;
    ready
        .recv_timeout(Duration::from_secs(15))
        .map_err(|_| "Helper audio worker did not become ready")?;
    wire::send(
        &mut control,
        &Reply::Ready {
            classes,
            latency: session.latency_samples(),
            parameters,
            state,
            editor,
        },
    )?;
    control.set_read_timeout(None).map_err(|e| e.to_string())?;
    let (incoming, commands) = mpsc::sync_channel(32);
    let (replies, outgoing) = mpsc::sync_channel(32);
    let mut reader = control.try_clone().map_err(|e| e.to_string())?;
    let orphan_memory = memory.clone();
    thread::Builder::new()
        .name("rack-helper-control-in".into())
        .spawn(move || {
            while let Ok(cmd) = wire::receive(&mut reader) {
                if incoming.try_send(cmd).is_err() {
                    break;
                }
            }
            // Parent loss must also end a helper whose GUI/main thread is
            // stuck inside foreign code. This watchdog is independent of both
            // the GUI and DSP threads. Normal shutdown exits before the grace.
            orphan_memory.stop();
            drop(incoming);
            thread::sleep(Duration::from_secs(2));
            std::process::exit(1);
        })
        .map_err(|e| e.to_string())?;
    thread::Builder::new()
        .name("rack-helper-control-out".into())
        .spawn(move || {
            while let Ok(reply) = outgoing.recv() {
                if wire::send(&mut control, &reply).is_err() {
                    break;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    let mut app = App {
        session,
        memory,
        title,
        worker: Some(worker),
        commands,
        replies,
        window: None,
        closing: false,
        last_state: Instant::now(),
        state_pending: false,
        save_request: None,
        pending_open: None,
        finished: false,
        save_fence: None,
        pending_geometry: None,
    };
    // CLAP's main thread exists without a window system. Defer connecting to
    // AppKit/X11/Win32 until an editor is actually requested, while continuing
    // to service state, parameters and host callbacks on this initial thread.
    while app.pending_open.is_none() && !app.finished {
        app.tick(None);
        if app.pending_open.is_none() && !app.finished {
            thread::sleep(Duration::from_millis(4));
        }
    }
    if app.finished {
        return Ok(());
    }
    let mut builder = EventLoop::builder();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder.with_activation_policy(ActivationPolicy::Accessory);
    }
    #[cfg(target_os = "linux")]
    {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_x11();
    }
    let event_loop = builder.build().map_err(|e| e.to_string())?;
    event_loop.run_app(&mut app).map_err(|e| e.to_string())
}
struct App {
    session: LiveSession,
    title: String,
    memory: Arc<AudioMemory>,
    worker: Option<thread::JoinHandle<()>>,
    commands: Receiver<Command>,
    replies: SyncSender<Reply>,
    window: Option<Arc<Window>>,
    closing: bool,
    last_state: Instant,
    state_pending: bool,
    save_request: Option<u64>,
    pending_open: Option<Geometry>,
    finished: bool,
    save_fence: Option<u32>,
    pending_geometry: Option<Geometry>,
}
impl App {
    fn reply(&self, reply: Reply) {
        if self.replies.try_send(reply).is_err() {
            self.memory.fail();
        }
    }
    fn close(&mut self) {
        self.session.close_editors();
        self.window = None;
        self.reply(Reply::Closed);
    }
    fn geometry(&mut self) {
        if let Some(w) = &self.window {
            let size = w.inner_size().to_logical::<u32>(w.scale_factor());
            self.pending_geometry = Some(Geometry {
                position: w.outer_position().ok().map(|p| [p.x, p.y]),
                size: [size.width, size.height],
            });
        }
    }
    fn open(&mut self, event_loop: &ActiveEventLoop, geometry: Geometry) -> Result<(), String> {
        if let Some(window) = &self.window {
            window.set_visible(true);
            window.focus_window();
            return Ok(());
        }
        let attrs = Window::default_attributes()
            .with_title(format!("{} — Nice Rack", self.title))
            .with_visible(false)
            .with_inner_size(winit::dpi::LogicalSize::new(
                geometry.size[0].max(1),
                geometry.size[1].max(1),
            ));
        let window = Arc::new(event_loop.create_window(attrs).map_err(|e| e.to_string())?);
        if let Some(p) = geometry.position {
            window.set_outer_position(winit::dpi::PhysicalPosition::new(p[0], p[1]));
        }
        let saved = Some(if cfg!(target_os = "macos") {
            geometry.size
        } else {
            let size = winit::dpi::LogicalSize::new(geometry.size[0], geometry.size[1])
                .to_physical::<u32>(window.scale_factor());
            [size.width, size.height]
        });
        let (size, resizable) =
            self.session
                .open_editor(PLUGIN, window.clone(), saved, window.scale_factor())?;
        window.set_resizable(resizable);
        if cfg!(target_os = "macos") {
            let _ = window.request_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]));
        } else {
            let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(size[0], size[1]));
        }
        window.set_visible(true);
        self.window = Some(window);
        self.reply(Reply::Opened);
        self.geometry();
        Ok(())
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(geometry) = self.pending_open.take()
            && let Err(error) = self.open(event_loop, geometry)
        {
            self.reply(Reply::EditorError(error));
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.close(),
            WindowEvent::Moved(_) => self.geometry(),
            WindowEvent::Resized(size) => {
                if let Some(w) = &self.window {
                    let size = if cfg!(target_os = "macos") {
                        let s = size.to_logical::<u32>(w.scale_factor());
                        [s.width, s.height]
                    } else {
                        [size.width, size.height]
                    };
                    if size.iter().all(|n| *n > 0)
                        && let Ok(adjusted) = self.session.resize_editor(PLUGIN, size)
                        && adjusted != size
                    {
                        if cfg!(target_os = "macos") {
                            let _ = w.request_inner_size(winit::dpi::LogicalSize::new(
                                adjusted[0],
                                adjusted[1],
                            ));
                        } else {
                            let _ = w.request_inner_size(winit::dpi::PhysicalSize::new(
                                adjusted[0],
                                adjusted[1],
                            ));
                        }
                    }
                }
                self.geometry();
            }
            _ => {}
        }
        if self.closing {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(4),
            ));
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.tick(Some(event_loop));
        if self.finished {
            event_loop.exit();
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(4),
            ));
        }
    }
}
impl App {
    fn tick(&mut self, event_loop: Option<&ActiveEventLoop>) {
        for _ in 0..32 {
            let command = match self.commands.try_recv() {
                Ok(v) => v,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(_) => {
                    self.closing = true;
                    break;
                }
            };
            let result = match command {
                Command::Parameter { id, value } => {
                    let result = self.session.set_parameter(PLUGIN, id, value);
                    if result.is_ok() {
                        self.memory.request_flush();
                    }
                    result
                }
                Command::Save(request) => {
                    self.state_pending = true;
                    self.save_request = Some(request);
                    self.save_fence = Some(self.memory.request_flush());
                    Ok(())
                }
                Command::Open { position, size } => {
                    if let Some(event_loop) = event_loop {
                        if let Err(error) = self.open(event_loop, Geometry { position, size }) {
                            self.reply(Reply::EditorError(error));
                        }
                    } else {
                        self.pending_open = Some(Geometry { position, size });
                    }
                    Ok(())
                }
                Command::Close => {
                    self.close();
                    Ok(())
                }
                Command::Shutdown => {
                    self.closing = true;
                    Ok(())
                }
                Command::Hello { .. } => Err("Repeated helper handshake".into()),
            };
            if let Err(e) = result {
                self.reply(Reply::Error(e));
            }
        }
        if let Err(e) = self.session.service() {
            self.reply(Reply::Error(e));
            self.memory.fail();
        }
        for (_, event) in self.session.editor_events() {
            match event {
                EditorEvent::Closed => self.close(),
                EditorEvent::Hide => {
                    if let Some(w) = &self.window {
                        w.set_visible(false)
                    }
                }
                EditorEvent::Show => {
                    if let Some(w) = &self.window {
                        w.set_visible(true)
                    }
                }
                EditorEvent::Resize(width, height) => {
                    if let Some(w) = &self.window {
                        if cfg!(target_os = "macos") {
                            let _ =
                                w.request_inner_size(winit::dpi::LogicalSize::new(width, height));
                        } else {
                            let _ =
                                w.request_inner_size(winit::dpi::PhysicalSize::new(width, height));
                        }
                    }
                }
            }
        }
        if !self.session.dirty_instances().is_empty() {
            self.state_pending = true;
        }
        if self.state_pending
            && self
                .save_fence
                .is_none_or(|fence| self.memory.flush_reached(fence))
            && (self.save_request.is_some()
                || self.last_state.elapsed() >= Duration::from_millis(100))
        {
            match self.session.save_states() {
                Ok(mut states) => {
                    let state = states.remove(&PLUGIN);
                    if let Some(request) = self.save_request.take() {
                        self.reply(Reply::Saved { request, state });
                    } else {
                        self.reply(Reply::State(state));
                    }
                    let parameters = self.session.parameters(PLUGIN);
                    self.reply(Reply::Parameters(parameters));
                    self.state_pending = false;
                    self.save_fence = None;
                    self.last_state = Instant::now();
                }
                Err(e) => self.reply(Reply::Error(e)),
            }
        }
        if let Some(geometry) = self.pending_geometry.take()
            && let Err(mpsc::TrySendError::Full(_)) = self.replies.try_send(Reply::Geometry {
                position: geometry.position,
                size: geometry.size,
            })
        {
            self.pending_geometry = Some(geometry);
        }
        if self.closing {
            self.session.request_stop();
            self.memory.stop();
            if self
                .worker
                .as_ref()
                .is_some_and(thread::JoinHandle::is_finished)
            {
                let _ = self.worker.take().unwrap().join();
                let _ = self.session.service();
                self.close();
                self.finished = true;
            }
        }
    }
}
