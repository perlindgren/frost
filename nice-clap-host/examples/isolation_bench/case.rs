use super::gui::GuiChild;
use super::model::*;
use nice_clap_host::{Binding, Entry, LiveAudio, LiveSession, LiveStatus};
use nice_rack_engine::NodeId;
use std::path::PathBuf;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowId},
};

pub fn run(job: Job) -> Result<ResultRow, String> {
    if matches!(job.mode, Mode::GuiIsolated) && !job.editor_replicas {
        return Err("GUI-only mode requires the controlled editor-replica workload".into());
    }
    let setup = Instant::now();
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut bindings = BTreeMap::new();
    let mut classes = Vec::new();
    for (index, path) in job.plugins.iter().enumerate() {
        let entry = match job.mode {
            Mode::InProcess | Mode::GuiIsolated => {
                // SAFETY: this explicitly invoked benchmark executes the operator's
                // selected native plugins. They must satisfy the CLAP ABI.
                unsafe { Entry::load(path) }?
            }
            Mode::Isolated => Entry::isolated(path, &executable)?,
        };
        let class = entry
            .descriptors()?
            .first()
            .ok_or("No CLAP class")?
            .id
            .clone();
        classes.push(class.clone());
        bindings.insert(NodeId(index as u64 + 1), Binding { entry, class });
    }
    let graph = graph(&job, &bindings)?;
    let (session, mut audio) =
        LiveSession::prepare(&graph, &bindings, job.rate, job.frames, &BTreeMap::new())?;
    // Include the same optional host-side observations the rack uses, in both
    // modes. Their parent-side cost must not disappear from this comparison.
    audio.attach_wire_levels(Arc::new(nice_rack_engine::audio::WireLevels::new(
        bindings.len() * 2 + 2,
    )));
    audio.attach_processing_times(
        Arc::new(nice_rack_engine::audio::ProcessingTimes::new(
            bindings.len(),
        )),
        job.rate,
    );
    let setup_ms = setup.elapsed().as_secs_f64() * 1000.;
    let gui_setup = Instant::now();
    let mut gui_session = None;
    let mut gui_children = Vec::new();
    if job.editor_replicas {
        if matches!(job.mode, Mode::InProcess) {
            let (mut replica, audio) =
                LiveSession::prepare(&graph, &bindings, job.rate, job.frames, &BTreeMap::new())?;
            drop(audio);
            if replica.service()? != LiveStatus::Stopped {
                return Err("Editor replicas did not deactivate".into());
            }
            gui_session = Some(replica);
        } else {
            for (index, class) in classes.iter().enumerate() {
                gui_children.push(GuiChild::spawn(&job, index, class)?);
            }
        }
    }
    let mut app = Case::new(job, classes, setup_ms, session, Some(audio));
    app.gui_session = gui_session;
    app.gui_children = gui_children;
    if app.row.job.editor_replicas {
        app.opening = gui_setup;
    }
    if app.row.job.editors {
        let mut builder = EventLoop::builder();
        #[cfg(target_os = "linux")]
        {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        }
        match builder.build() {
            Ok(event_loop) => {
                if let Err(error) = event_loop.run_app(&mut app) {
                    app.fail(error.to_string());
                }
            }
            Err(error) => app.fail(error.to_string()),
        }
    } else {
        while !app.finished {
            app.tick();
            thread::sleep(Duration::from_millis(1));
        }
    }
    app.stop.store(true, Ordering::Release);
    if let Some(worker) = app.worker.take() {
        if !worker.is_finished() {
            // This code runs only in a disposable --case process. An unexpected
            // platform-loop exit must not destroy a live native owner on the
            // wrong thread. The coordinator records this as a failed case.
            eprintln!("Benchmark event loop exited while DSP was active");
            std::process::exit(2);
        }
        app.row.audio = Some(worker.join().unwrap_or_else(|_| AudioResult {
            error: Some("DSP worker panicked".into()),
            ..AudioResult::default()
        }));
    }
    // On editor-setup failure there may still be an unstarted audio handle.
    // Return it before destroying the native main-thread owner.
    drop(app.audio.take());
    let _ = app.session.service();
    // Explicitly drop session/windows before draining supervised helper reapers.
    app.view().close_editors();
    app.session.close_editors();
    app.windows.clear();
    for child in &mut app.gui_children {
        child.request_close();
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut cleanup_error = None;
    for child in &mut app.gui_children {
        if let Err(error) = child.finish(deadline) {
            cleanup_error = Some(error);
        }
    }
    if let Some(error) = cleanup_error {
        app.fail(error);
    }
    app.gui_children.clear();
    drop(app.gui_session.take());
    app.row.heartbeat_lateness = Distribution::new(std::mem::take(&mut app.late));
    app.row.heartbeat_gap = Distribution::new(std::mem::take(&mut app.gaps));
    let row = app.row;
    drop(app.session);
    nice_clap_host::isolated::finish_shutdown();
    Ok(row)
}
struct Case {
    row: ResultRow,
    session: LiveSession,
    gui_session: Option<LiveSession>,
    gui_children: Vec<GuiChild>,
    gui_ready: Option<PathBuf>,
    gui_ready_sent: bool,
    window_offset: usize,
    audio: Option<LiveAudio>,
    worker: Option<thread::JoinHandle<AudioResult>>,
    windows: Vec<Arc<Window>>,
    stop: Arc<AtomicBool>,
    started: Option<Instant>,
    opening: Instant,
    next_beat: Option<Instant>,
    last_beat: Option<Instant>,
    late: Vec<f64>,
    gaps: Vec<f64>,
    finished: bool,
    cancelled: bool,
}
impl Case {
    fn new(
        job: Job,
        classes: Vec<String>,
        setup_ms: f64,
        session: LiveSession,
        audio: Option<LiveAudio>,
    ) -> Self {
        Self {
            row: ResultRow {
                job,
                classes,
                setup_ms: Some(setup_ms),
                editor_open_ms: None,
                heartbeat_lateness: Distribution::default(),
                heartbeat_gap: Distribution::default(),
                missed_heartbeats: 0,
                audio: None,
                error: None,
            },
            session,
            gui_session: None,
            gui_children: Vec::new(),
            gui_ready: None,
            gui_ready_sent: false,
            window_offset: 0,
            audio,
            worker: None,
            windows: Vec::new(),
            stop: Arc::new(AtomicBool::new(false)),
            started: None,
            opening: Instant::now(),
            next_beat: None,
            last_beat: None,
            late: Vec::new(),
            gaps: Vec::new(),
            finished: false,
            cancelled: false,
        }
    }
    fn view(&mut self) -> &mut LiveSession {
        self.gui_session.as_mut().unwrap_or(&mut self.session)
    }
    fn fail(&mut self, error: String) {
        if self.row.error.is_none() {
            self.row.error = Some(error);
        }
    }
    fn start(&mut self) {
        if self.started.is_some() {
            return;
        }
        let Some(audio) = self.audio.take() else {
            return;
        };
        let job = self.row.job.clone();
        let stop = self.stop.clone();
        let started = Instant::now();
        let measured = started + Duration::from_secs_f64(job.warmup);
        let capacity = (job.seconds * 60.).ceil() as usize + 100;
        self.late = Vec::with_capacity(capacity);
        self.gaps = Vec::with_capacity(capacity);
        self.next_beat = Some(measured + Duration::from_secs_f64(1. / 60.));
        self.started = Some(started);
        self.worker = Some(thread::spawn(move || {
            audio_run(audio, &job, started, &stop)
        }));
    }
    fn tick(&mut self) {
        if self.finished {
            return;
        }
        if let Err(error) = self.session.service() {
            self.fail(error);
        }
        if let Some(gui) = &mut self.gui_session
            && let Err(error) = gui.service()
        {
            self.fail(error);
            self.cancelled = true;
        }
        let mut replicas_ready = true;
        for child in &mut self.gui_children {
            match child.poll() {
                Ok(ready) => replicas_ready &= ready,
                Err(error) => {
                    self.row.error.get_or_insert(error);
                    self.cancelled = true;
                }
            }
        }
        for error in self.view().isolated_editor_errors() {
            self.fail(error);
            self.cancelled = true;
        }
        if !self.view().closed_isolated_editors().is_empty() {
            self.fail("Editor closed during measurement".into());
            self.cancelled = true;
        }
        for (id, event) in self.view().editor_events() {
            match event {
                nice_clap_host::EditorEvent::Resize(w, h) => {
                    if let Some(window) = self.windows.get(id.0 as usize - 1) {
                        resize(window, [w, h]);
                    }
                }
                nice_clap_host::EditorEvent::Show | nice_clap_host::EditorEvent::Hide => {
                    if let Some(window) = self.windows.get(id.0 as usize - 1) {
                        window.set_visible(matches!(event, nice_clap_host::EditorEvent::Show));
                    }
                }
                nice_clap_host::EditorEvent::Closed => {
                    self.fail("Editor closed during measurement".into());
                    self.cancelled = true;
                }
            }
        }
        if let Some(path) = &self.gui_ready {
            if self.cancelled || self.stop.load(Ordering::Acquire) {
                self.finished = true;
                return;
            }
            if !self.gui_ready_sent {
                if let Err(error) =
                    super::gui::publish_ready(path, &Ok(self.row.classes[0].clone()))
                {
                    self.fail(error);
                    self.finished = true;
                } else {
                    self.gui_ready_sent = true;
                }
            }
            return;
        }
        if self.started.is_none() {
            if self.cancelled {
                self.finished = true;
                return;
            }
            let ready = if self.row.job.editor_replicas {
                replicas_ready
            } else if !self.row.job.editors {
                true
            } else {
                (1..=self.row.job.plugins.len()).all(|id| {
                    !self.session.editor_is_isolated(NodeId(id as u64))
                        || self.session.isolated_geometry(NodeId(id as u64)).is_some()
                })
            };
            if ready {
                if self.row.job.editors {
                    self.row.editor_open_ms = Some(self.opening.elapsed().as_secs_f64() * 1000.);
                }
                self.start();
            } else if self.opening.elapsed() > Duration::from_secs(15) {
                self.fail("Editor-open acknowledgement timed out".into());
                self.stop.store(true, Ordering::Release);
                self.finished = true;
            }
            return;
        }
        let now = Instant::now();
        let total = Duration::from_secs_f64(self.row.job.warmup + self.row.job.seconds);
        let measured_end = self.started.unwrap() + total;
        if let Some(next) = self.next_beat
            && next < measured_end
            && now >= next
        {
            let interval = Duration::from_secs_f64(1. / 60.);
            let late = now.duration_since(next);
            let skipped = ((late.as_secs_f64() / interval.as_secs_f64()).floor() as u32).min(
                (measured_end.duration_since(next).as_secs_f64() / interval.as_secs_f64()).floor()
                    as u32,
            );
            self.row.missed_heartbeats += skipped as usize;
            self.late.push(late.as_secs_f64() * 1e6);
            if let Some(last) = self.last_beat {
                self.gaps.push(now.duration_since(last).as_secs_f64() * 1e6);
            }
            self.last_beat = Some(now);
            self.next_beat = Some(next + interval * (skipped + 1));
        }
        let ending = self.cancelled || now.duration_since(self.started.unwrap()) >= total;
        if ending {
            self.stop.store(true, Ordering::Release);
        }
        // Preserve the full GUI observation duration even if audio failed early.
        if ending
            && self
                .worker
                .as_ref()
                .is_some_and(thread::JoinHandle::is_finished)
        {
            self.row.audio = Some(match self.worker.take().unwrap().join() {
                Ok(result) => result,
                Err(_) => AudioResult {
                    error: Some("DSP worker panicked".into()),
                    ..AudioResult::default()
                },
            });
            let _ = self.session.service();
            self.finished = true;
        }
    }
}
impl ApplicationHandler for Case {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.started.is_some() || !self.windows.is_empty() {
            return;
        }
        if !self.row.job.editor_replicas {
            self.opening = Instant::now();
        }
        if !self.gui_children.is_empty() {
            return;
        }
        let result = (|| {
            for index in 0..self.row.job.plugins.len() {
                let id = NodeId(index as u64 + 1);
                let offset = self.window_offset;
                if !self.view().editor_supported(id) {
                    return Err(format!("Plugin {id:?} has no supported editor"));
                }
                if self.view().editor_is_isolated(id) {
                    self.view().open_isolated_editor(
                        id,
                        nice_clap_host::isolated::Geometry {
                            position: Some([
                                40 + (index + offset) as i32 * 50,
                                40 + (index + offset) as i32 * 40,
                            ]),
                            size: [640, 480],
                        },
                    )?;
                } else {
                    let window = Arc::new(
                        event_loop
                            .create_window(
                                Window::default_attributes()
                                    .with_title(format!("Benchmark editor {}", index + 1))
                                    .with_visible(false),
                            )
                            .map_err(|e| e.to_string())?,
                    );
                    window.set_outer_position(winit::dpi::PhysicalPosition::new(
                        40 + (index + offset) as i32 * 50,
                        40 + (index + offset) as i32 * 40,
                    ));
                    let (size, resizable) =
                        self.view()
                            .open_editor(id, window.clone(), None, window.scale_factor())?;
                    window.set_resizable(resizable);
                    resize(&window, size);
                    window.set_visible(true);
                    self.windows.push(window);
                }
            }
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            self.fail(error);
            self.finished = true;
            event_loop.exit();
        }
    }
    fn window_event(&mut self, _event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::CloseRequested) {
            self.fail("Editor closed during measurement".into());
            self.stop.store(true, Ordering::Release);
            self.cancelled = true;
            return;
        }
        if let Some((index, window)) = self
            .windows
            .iter()
            .enumerate()
            .find(|(_, w)| w.id() == id)
            .map(|(i, w)| (i, w.clone()))
            && let WindowEvent::Resized(size) = event
            && size.width > 0
            && size.height > 0
        {
            let size = if cfg!(target_os = "macos") {
                let s = size.to_logical::<u32>(window.scale_factor());
                [s.width, s.height]
            } else {
                [size.width, size.height]
            };
            if let Ok(adjusted) = self.view().resize_editor(NodeId(index as u64 + 1), size)
                && adjusted != size
            {
                resize(&window, adjusted);
            }
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.tick();
        if self.finished {
            event_loop.exit();
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(4),
            ));
        }
    }
}
fn resize(window: &Window, size: [u32; 2]) {
    if cfg!(target_os = "macos") {
        let _ = window.request_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]));
    } else {
        let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(size[0], size[1]));
    }
}
fn audio_run(mut audio: LiveAudio, job: &Job, started: Instant, stop: &AtomicBool) -> AudioResult {
    let period = period(job);
    let warm_end = started + Duration::from_secs_f64(job.warmup);
    let end = warm_end + Duration::from_secs_f64(job.seconds);
    let capacity = (job.seconds * job.rate / job.frames as f64).ceil() as usize + 1;
    let mut samples = Vec::with_capacity(capacity);
    let mut late = Vec::with_capacity(capacity);
    let mut input = vec![0.; job.frames];
    let mut output = [vec![0.; job.frames], vec![0.; job.frames]];
    let mut result = AudioResult::default();
    let mut next = started;
    let mut phase = 0.;
    while next < end && !stop.load(Ordering::Acquire) {
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            thread::sleep(wait);
        }
        let measured = next >= warm_end;
        let wake = Instant::now().saturating_duration_since(next).as_secs_f64() * 1e6;
        for sample in &mut input {
            *sample = (phase as f32).sin() * 0.1;
            phase = (phase + std::f64::consts::TAU * 110. / job.rate) % std::f64::consts::TAU;
        }
        let [left, right] = &mut output;
        let before = Instant::now();
        let processed = audio.process(
            &[&input, &input],
            &mut [&mut [&mut left[..], &mut right[..]]],
            job.frames,
        );
        let took = before.elapsed();
        if measured {
            result.attempted += 1;
            late.push(wake);
        }
        if took > period {
            if measured {
                result.deadline_misses += 1;
            } else {
                result.warmup_deadline_misses += 1;
            }
        }
        match processed {
            Ok(LiveStatus::Running) => {
                if output.iter().flatten().any(|x| !x.is_finite()) {
                    result.error = Some("Non-finite output".into());
                    break;
                }
                if measured {
                    samples.push(took.as_secs_f64() * 1e6);
                    result.checksum += output.iter().flatten().map(|&s| s as f64).sum::<f64>();
                } else {
                    result.warmup_completed += 1;
                }
            }
            other => {
                // Production isolated endpoints remain poisoned after timeout.
                // Do not turn later silent/stopped blocks into fast samples.
                result.failed_during_warmup = !measured;
                result.failed_call_us = Some(took.as_secs_f64() * 1e6);
                result.error = Some(format!("{other:?}"));
                break;
            }
        }
        next += period;
    }
    if samples.is_empty() && result.error.is_none() {
        result.error = Some("No measured blocks completed".into());
    }
    result.successful = Distribution::new(samples);
    result.wake_lateness = Distribution::new(late);
    result
}

pub fn gui_only(job: Job, ready: PathBuf, index: usize) -> Result<(), String> {
    let stop = super::gui::watch_parent();
    let path = job.plugins.first().ok_or("Missing GUI fixture")?;
    // SAFETY: explicitly selected native benchmark fixture; it must obey CLAP.
    let entry = unsafe { Entry::load(path) }?;
    let class = entry
        .descriptors()?
        .first()
        .ok_or("No GUI class")?
        .id
        .clone();
    let bindings = [(
        NodeId(1),
        Binding {
            entry,
            class: class.clone(),
        },
    )]
    .into();
    let graph = graph(&job, &bindings)?;
    let (mut session, audio) =
        LiveSession::prepare(&graph, &bindings, job.rate, job.frames, &BTreeMap::new())?;
    // This returns and deactivates the processing handle WITHOUT process().
    // No audio thread or sample mapping exists for an editor-only replica.
    drop(audio);
    if session.service()? != LiveStatus::Stopped {
        return Err("GUI-only helper retained active DSP".into());
    }
    let mut app = Case::new(job, vec![class], 0., session, None);
    app.gui_ready = Some(ready);
    app.stop = stop;
    app.window_offset = index;
    let result = if app.row.job.editors {
        let mut builder = EventLoop::builder();
        #[cfg(target_os = "linux")]
        {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        }
        #[cfg(target_os = "macos")]
        {
            use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
            builder.with_activation_policy(ActivationPolicy::Accessory);
        }
        builder
            .build()
            .map_err(|e| e.to_string())
            .and_then(|event_loop| event_loop.run_app(&mut app).map_err(|e| e.to_string()))
    } else {
        while !app.finished {
            app.tick();
            thread::sleep(Duration::from_millis(4));
        }
        Ok(())
    };
    app.session.close_editors();
    app.windows.clear();
    result?;
    if let Some(error) = app.row.error {
        Err(error)
    } else {
        Ok(())
    }
}
