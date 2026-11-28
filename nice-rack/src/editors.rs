//! Standalone native editor windows, driven by frost's existing event loop.
use crate::{EditorGeometry, routing::NodeId, runtime::Runtime};
use std::{collections::BTreeMap, sync::Arc};
use winit::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowId},
};
#[cfg(any(target_os = "linux", test))]
mod request;
#[cfg(target_os = "linux")]
mod x11;
/// The frame an editor hangs from.
///
/// Normally that is a window of frost's own loop, and the plugin is attached
/// to its handle directly. On Wayland it is not: the CLAP GUI API can name an
/// X11, Win32 or Cocoa parent and nothing else, so a frost window — a Wayland
/// surface — is not something a plugin will accept as an editor's parent, and
/// the frame comes from the X11 loop in `x11` instead.
enum Frame {
    Winit(Arc<Window>),
    #[cfg(target_os = "linux")]
    X11(x11::Frame),
}
#[derive(Default)]
pub(crate) struct Windows {
    windows: BTreeMap<NodeId, Frame>,
    pending: Vec<NodeId>,
    /// The editors the user is asking for, as this layer understands it.
    ///
    /// A CLAP editor cannot outlive the instance it is attached to, and a roster
    /// edit still takes the chain down — but the plugins it did not touch come
    /// back as the same plugins (`LiveSession::reusing` carries them), so their
    /// editors sit through it untouched. What is left to remember is the rest: a
    /// node whose instance had to be built again loses the frame that was opened
    /// for the instance before it, and nobody asked for that either. So an id
    /// stays in here for as long as its owner wants the window, through every
    /// rebuild and through a rollback that rebuilds the old chain, and leaves when
    /// the user closes the editor, when the plugin closes it, when its card is
    /// gone, or when the rack itself quits. Everything on screen or on its way is
    /// in here; the reverse is not true while a rebuild has the chain down.
    wanted: Vec<NodeId>,
    /// Frames the user closed while no chain was on the transport to take the
    /// editor down with them.
    ///
    /// The frame is the rack's and goes at once; the editor belongs to a plugin
    /// instance that is off the transport in a session nothing is asking anything
    /// of, and `Owner` is the one that destroys it. These ids wait for a session
    /// to hand them back to, which keeps a plugin from being asked to destroy its
    /// interface while its audio is still being pulled.
    dismissed: Vec<NodeId>,
    /// The second windowing loop, up only when frost itself runs on Wayland and
    /// an editor has been asked for. A start that fails (no XWayland) is retried
    /// at the next asking rather than remembered as broken.
    #[cfg(target_os = "linux")]
    x11: Option<x11::Service>,
    #[cfg(target_os = "linux")]
    x11_starting: Option<request::Request<x11::Service>>,
    #[cfg(target_os = "linux")]
    x11_pending: BTreeMap<NodeId, x11::OpenRequest>,
    /// Whether frost's window is a Wayland surface, read once from its own
    /// handle. `None` means not yet looked.
    #[cfg(target_os = "linux")]
    wayland_shell: Option<bool>,
}
impl Windows {
    /// Read once, from frost's own window, whether the rack runs on Wayland —
    /// the only case whose editors need the second loop.
    ///
    /// The question is asked of a real window's handle rather than of an
    /// environment variable because that handle is precisely what the plugin
    /// would be handed: no handle, or a non-Wayland one, and the rack's own
    /// windows keep parenting editors the ordinary way.
    #[cfg(target_os = "linux")]
    pub fn probe_shell(&mut self, window: Option<&Window>) {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if self.wayland_shell.is_some() {
            return;
        }
        if let Some(window) = window
            && let Ok(handle) = window.window_handle()
        {
            self.wayland_shell = Some(matches!(handle.as_raw(), RawWindowHandle::Wayland(_)));
        }
    }
    /// Everywhere but Linux the rack's one loop can parent every editor, so
    /// there is nothing to find out.
    #[cfg(not(target_os = "linux"))]
    pub fn probe_shell(&mut self, window: Option<&Window>) {
        let _ = window;
    }
    /// What the editor button asks for: open the editor, or bring a live one
    /// to the front, which is what the `pending` pass below does with an id it
    /// already has a window for.
    pub fn open(&mut self, id: NodeId) {
        // Asking is what `wanted` remembers, so a rebuild that lands between
        // this click and the frame that would have built the window cannot
        // swallow the request along with the queue it was sitting in.
        if !self.wanted.contains(&id) {
            self.wanted.push(id);
        }
        if !self.pending.contains(&id) {
            self.pending.push(id);
        }
    }
    /// Queue every wanted editor that is not already on screen, which is what
    /// buys back the ones a rebuild took away. Called every frame, so an id
    /// with a live window is left alone: a `pending` entry against a window
    /// that exists means taking the focus to it every frame.
    fn requeue_wanted(&mut self) {
        let wanted = std::mem::take(&mut self.wanted);
        for &id in &wanted {
            if !self.windows.contains_key(&id) && !self.pending.contains(&id) {
                self.pending.push(id);
            }
        }
        self.wanted = wanted;
    }
    /// Stop remembering an editor the user is finished with, so the next
    /// rebuild does not bring it back. The window belongs to the caller, and by
    /// the time this runs it is gone.
    fn forget(&mut self, id: NodeId) {
        #[cfg(target_os = "linux")]
        self.x11_pending.remove(&id);
        self.wanted.retain(|wanted| *wanted != id);
        self.pending.retain(|pending| *pending != id);
        #[cfg(target_os = "linux")]
        if self.wanted.is_empty() && self.x11.is_none() {
            self.x11_starting = None;
        }
    }
    /// Take a frame down without giving up on the editor. The plugin its editor
    /// was drawn by is gone; the card that names that plugin is still in the rack,
    /// and the instance built for it now is as willing a host as the one before
    /// it, so `wanted` keeps the id and `requeue_wanted` asks again — in the same
    /// frame this one comes down. A frame from the X11 loop is handed back to
    /// that loop to take down; it belongs to a thread this one cannot touch.
    fn close_window(&mut self, id: NodeId) {
        #[cfg(target_os = "linux")]
        if matches!(self.windows.get(&id), Some(Frame::X11(_)))
            && let Some(service) = &self.x11
        {
            service.close(id);
        }
        self.windows.remove(&id);
    }
    /// Close every frame whose plugin no longer has an editor.
    ///
    /// An untouched plugin keeps its instance across a roster edit, and its window
    /// keeps standing; a node that had to be built again does not, and the frame
    /// left over it is a window around nothing, since the instance it was opened
    /// for was retired along with the chain before it. The card that is gone
    /// entirely is caught by the same question — it has no instance to answer —
    /// and its id leaves `wanted` one step later, with the pending pass that finds
    /// no card to build a window for.
    fn close_orphaned(&mut self, runtime: &mut Runtime) {
        let orphaned: Vec<NodeId> = runtime
            .session
            .as_ref()
            .map(|session| {
                self.windows
                    .keys()
                    .copied()
                    .filter(|id| !session.has_editor(*id))
                    .collect()
            })
            .unwrap_or_default();
        if orphaned.is_empty() {
            return;
        }
        self.capture(runtime);
        for id in orphaned {
            self.close_window(id);
        }
    }
    pub fn capture(&self, runtime: &mut Runtime) {
        for (&id, frame) in &self.windows {
            match frame {
                Frame::Winit(window) => {
                    if window.inner_size().width == 0 || window.inner_size().height == 0 {
                        continue;
                    }
                    let logical = window.inner_size().to_logical::<u32>(window.scale_factor());
                    runtime.editor_geometry.insert(
                        id,
                        EditorGeometry {
                            position: window.outer_position().ok().map(|p| [p.x, p.y]),
                            size: [logical.width, logical.height],
                        },
                    );
                }
                #[cfg(target_os = "linux")]
                Frame::X11(frame) => {
                    // The frame's live size is what the loop last reported
                    // through `pump_worker`; this thread cannot ask a window it
                    // does not own.
                    if frame.inner[0] == 0 || frame.inner[1] == 0 {
                        continue;
                    }
                    let logical = PhysicalSize::new(frame.inner[0], frame.inner[1])
                        .to_logical::<u32>(frame.scale);
                    runtime.editor_geometry.insert(
                        id,
                        EditorGeometry {
                            position: frame.outer,
                            size: [logical.width, logical.height],
                        },
                    );
                }
            }
        }
    }
    pub fn close(&mut self, runtime: &mut Runtime) {
        #[cfg(target_os = "linux")]
        {
            self.x11_pending.clear();
            self.x11_starting = None;
        }
        self.capture(runtime);
        if let Some(session) = &mut runtime.session {
            session.close_editors();
        }
        if runtime.closing {
            // Quitting is the one interruption after which nothing is asked for
            // again: the rack is not coming back with a chain to build.
            self.wanted.clear();
        }
        #[cfg(target_os = "linux")]
        if let Some(service) = &self.x11 {
            // The plugins have released every editor by now, so the frames can
            // go. When the rack itself is quitting the loop goes with them: it
            // exists for no one after this.
            for (&id, frame) in &self.windows {
                if matches!(frame, Frame::X11(_)) {
                    service.close(id);
                }
            }
            if runtime.closing {
                service.quit();
            }
        }
        self.windows.clear();
        self.pending.clear();
        // Every editor this session had is gone with `close_editors` above, so
        // none of them is waiting to be told about a frame that already fell.
        self.dismissed.clear();
    }
    pub fn service(&mut self, event_loop: &ActiveEventLoop, runtime: &mut Runtime) {
        if runtime.closing {
            self.close(runtime);
            return;
        }
        // The second loop's news is taken even while the chain is down: a frame
        // the user closed during a rebuild belongs to the session the next
        // chain restores, exactly like a frost frame closed in `event`.
        self.pump_worker(runtime);
        if runtime.busy() || runtime.session.is_none() {
            // The chain is down and the next one is on its way, and there is
            // nothing here to do in the meantime. A window whose instance was
            // carried is attached to a plugin that is merely off the transport;
            // one whose instance was not is judged against the chain that
            // replaces this, which is the first thing that happens when there is
            // a session to judge it by again.
            return;
        }
        self.close_orphaned(runtime);
        // Anything the user still wants that the rebuild took away goes back
        // on the queue: dropping a plugin into the rack is not a request to
        // close the editors of the plugins that were already there.
        self.requeue_wanted();
        if let Some(session) = &mut runtime.session {
            for id in std::mem::take(&mut self.dismissed) {
                session.close_editor(id);
            }
            for (id, event) in session.editor_events() {
                if self.windows.contains_key(&id) {
                    match event {
                        nice_clap_host::EditorEvent::Resize(w, h) => match self.windows.get(&id) {
                            Some(Frame::Winit(window)) => {
                                let _ = window.request_inner_size(api_size(window, [w, h]));
                            }
                            #[cfg(target_os = "linux")]
                            Some(Frame::X11(_)) => {
                                if let Some(service) = &self.x11 {
                                    service.resize(id, [w, h]);
                                }
                            }
                            None => {}
                        },
                        nice_clap_host::EditorEvent::Show | nice_clap_host::EditorEvent::Hide => {
                            let visible = matches!(event, nice_clap_host::EditorEvent::Show);
                            match self.windows.get(&id) {
                                Some(Frame::Winit(window)) => window.set_visible(visible),
                                #[cfg(target_os = "linux")]
                                Some(Frame::X11(_)) => {
                                    if let Some(service) = &self.x11 {
                                        service.visible(id, visible);
                                    }
                                }
                                None => {}
                            }
                        }
                        nice_clap_host::EditorEvent::Closed => {
                            session.close_editor(id);
                            self.close_window(id);
                            self.forget(id);
                        }
                    }
                } else if matches!(event, nice_clap_host::EditorEvent::Show) {
                    self.open(id);
                }
            }
        }
        for id in std::mem::take(&mut self.pending) {
            match self.windows.get(&id) {
                Some(Frame::Winit(window)) => {
                    window.set_visible(true);
                    window.focus_window();
                    continue;
                }
                #[cfg(target_os = "linux")]
                Some(Frame::X11(_)) => {
                    if let Some(service) = &self.x11 {
                        service.visible(id, true);
                        service.focus(id);
                    }
                    continue;
                }
                None => {}
            }
            let Some(card) = runtime.cards.iter().find(|c| c.id == id) else {
                // The card left the rack, and its instance with it, so an
                // editor for it is a window that can never be built again.
                self.forget(id);
                continue;
            };
            let saved = runtime.editor_geometry.get(&id).copied();
            #[cfg(target_os = "linux")]
            if matches!(self.wayland_shell, Some(true)) {
                let name = card.name.clone();
                if let Err(error) = self.open_x11(id, &name, saved, runtime) {
                    self.forget(id);
                    runtime.message = error;
                }
                continue;
            }
            let attributes = Window::default_attributes()
                .with_title(&card.name)
                .with_visible(false)
                .with_inner_size(LogicalSize::new(640.0, 480.0));
            let name = card.name.clone();
            let begun = std::time::Instant::now();
            trace_open(&format!("'{name}': building a frame to hand to the plugin"));
            let result = (|| {
                let window = Arc::new(
                    event_loop
                        .create_window(attributes)
                        .map_err(|e| e.to_string())?,
                );
                trace_open(&format!(
                    "'{name}': frame ready after {} ms",
                    begun.elapsed().as_millis()
                ));
                if let Some(position) = saved.and_then(|g| g.position) {
                    // Discard positions stranded on a disconnected display.
                    if window.available_monitors().any(|m| {
                        let p = m.position();
                        let s = m.size();
                        position[0] >= p.x
                            && position[1] >= p.y
                            && position[0] < p.x + s.width as i32 - 40
                            && position[1] < p.y + s.height as i32 - 40
                    }) {
                        window.set_outer_position(PhysicalPosition::new(position[0], position[1]));
                    }
                }
                let size = saved.map(|g| {
                    api_dimensions(
                        &window,
                        LogicalSize::new(g.size[0], g.size[1]).to_physical(window.scale_factor()),
                    )
                });
                let (size, resizable) = runtime
                    .session
                    .as_mut()
                    .ok_or("Plugin instances are unavailable")?
                    .open_editor(id, window.clone(), size, window.scale_factor())?;
                trace_open(&format!(
                    "'{name}': plugin attached its editor in {} ms, {size:?} resizable {resizable}",
                    begun.elapsed().as_millis()
                ));
                window.set_resizable(resizable);
                let _ = window.request_inner_size(api_size(&window, size));
                window.set_visible(true);
                trace_open(&format!(
                    "'{name}': editor shown, {} ms all told",
                    begun.elapsed().as_millis()
                ));
                Ok::<_, String>(window)
            })();
            match result {
                Ok(window) => {
                    self.windows.insert(id, Frame::Winit(window));
                }
                // A plugin that will not open an editor should not be asked
                // every frame, so the asking ends with the failure.
                Err(error) => {
                    trace_open(&format!(
                        "'{name}': refused after {} ms: {error}",
                        begun.elapsed().as_millis()
                    ));
                    self.forget(id);
                    runtime.message = error;
                }
            }
        }
    }
    pub fn event(&mut self, id: WindowId, event: &WindowEvent, runtime: &mut Runtime) -> bool {
        // Only frost-loop windows appear here: the X11 loop's windows live on
        // another event loop, and their news reaches the rack through
        // `pump_worker` instead.
        let Some((&node, window)) = self.windows.iter().find_map(|(node, frame)| match frame {
            Frame::Winit(window) if window.id() == id => Some((node, window)),
            _ => None,
        }) else {
            return false;
        };
        match event {
            WindowEvent::CloseRequested => {
                let begun = std::time::Instant::now();
                self.capture(runtime);
                if runtime.busy() || runtime.session.is_none() {
                    self.dismissed.push(node);
                } else if let Some(session) = &mut runtime.session {
                    session.close_editor(node);
                }
                self.close_window(node);
                self.forget(node);
                runtime.persist_document();
                trace_open(&format!(
                    "editor {} closed in {} ms",
                    node.0,
                    begun.elapsed().as_millis()
                ));
            }
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                let size = api_dimensions(window, *size);
                if let Some(session) = &mut runtime.session {
                    match session.resize_editor(node, size) {
                        Ok(adjusted) if adjusted != size => {
                            let _ = window.request_inner_size(api_size(window, adjusted));
                        }
                        Ok(_) => {}
                        Err(error) => runtime.message = error,
                    }
                }
                self.capture(runtime);
            }
            WindowEvent::Moved(_) => self.capture(runtime),
            _ => {}
        }
        true
    }
    /// Open an editor through the X11 loop instead of a frost window, because
    /// on Wayland the plugin cannot be parented into one of those.
    ///
    /// The frame is built hidden by the loop that will own it (see `x11`); the
    /// plugin is attached to its handle here, and only a plugin that takes the
    /// editor gets a visible window, exactly as on the frost-loop path.
    #[cfg(target_os = "linux")]
    fn open_x11(
        &mut self,
        id: NodeId,
        title: &str,
        saved: Option<EditorGeometry>,
        runtime: &mut Runtime,
    ) -> Result<(), String> {
        if self.x11.is_none() {
            let pending = self.x11_starting.get_or_insert_with(x11::Service::begin);
            match pending.poll() {
                Ok(Some(service)) => {
                    self.x11 = Some(service);
                    self.x11_starting = None;
                }
                Ok(None) => return Ok(()),
                Err(error) => {
                    self.x11_starting = None;
                    return Err(error);
                }
            }
        }
        let service = self.x11.as_ref().ok_or("the X11 editor loop is gone")?;
        let pending = self
            .x11_pending
            .entry(id)
            .or_insert_with(|| service.begin_open(id, title, saved.and_then(|g| g.position)));
        let Some(opened) = pending.poll()? else {
            return Ok(());
        };
        self.x11_pending.remove(&id);
        // The frame's own scale, not the main window's: these are different
        // outputs as far as XWayland is concerned.
        let size = saved.map(|g| {
            let size = LogicalSize::new(g.size[0], g.size[1]).to_physical(opened.scale_factor);
            [size.width, size.height]
        });
        // Read the frame's state before its handle moves into the session.
        let frame = x11::Frame::new(&opened);
        let (size, resizable) = match &mut runtime.session {
            Some(session) => session.open_editor(id, opened.parent, size, opened.scale_factor),
            None => Err("Plugin instances are unavailable".to_string()),
        }
        .inspect_err(|_| {
            service.close(id);
        })?;
        service.resizable(id, resizable);
        service.resize(id, size);
        service.visible(id, true);
        self.windows.insert(id, Frame::X11(frame));
        Ok(())
    }
    /// Fold the X11 loop's window events into the session, mirroring what
    /// `event` does for frost's own frames.
    #[cfg(target_os = "linux")]
    fn pump_worker(&mut self, runtime: &mut Runtime) {
        let events = match &self.x11 {
            Some(service) => service.drain_events(),
            None => return,
        };
        for event in events {
            match event {
                x11::WorkerEvent::Resized { id, size, scale } => {
                    // A hidden frame can emit configuration events before the
                    // pending reply has been attached to a plugin.
                    if !matches!(self.windows.get(&id), Some(Frame::X11(_))) {
                        continue;
                    }
                    if let Some(Frame::X11(frame)) = self.windows.get_mut(&id) {
                        frame.inner = size;
                        frame.scale = scale;
                    }
                    // On X11 the rack's size translation is the identity
                    // (`api_dimensions`), so the session's answer is the whole
                    // conversation: an adjustment comes straight back as a
                    // resize request.
                    let mut adjusted = None;
                    if let Some(session) = &mut runtime.session {
                        match session.resize_editor(id, size) {
                            Ok(answer) if answer != size => adjusted = Some(answer),
                            Ok(_) => {}
                            Err(error) => runtime.message = error,
                        }
                    }
                    if let (Some(size), Some(service)) = (adjusted, &self.x11) {
                        service.resize(id, size);
                    }
                    self.capture(runtime);
                }
                x11::WorkerEvent::Moved {
                    id,
                    position,
                    scale,
                } => {
                    if let Some(Frame::X11(frame)) = self.windows.get_mut(&id) {
                        frame.outer = Some(position);
                        frame.scale = scale;
                    }
                    self.capture(runtime);
                }
                x11::WorkerEvent::CloseRequested { id } => {
                    self.capture(runtime);
                    if runtime.busy() || runtime.session.is_none() {
                        self.dismissed.push(id);
                    } else if let Some(session) = &mut runtime.session {
                        session.close_editor(id);
                    }
                    // The frame comes down whether or not a session was there
                    // to hear about it, exactly as a frost frame does in
                    // `event`; the loop is told only after the plugin let go.
                    self.close_window(id);
                    self.forget(id);
                    runtime.persist_document();
                }
            }
        }
    }
    /// There is no second loop anywhere else, so there is nothing to pump.
    #[cfg(not(target_os = "linux"))]
    fn pump_worker(&mut self, _runtime: &mut Runtime) {}
}
fn api_size(window: &Window, size: [u32; 2]) -> PhysicalSize<u32> {
    if cfg!(target_os = "macos") {
        LogicalSize::new(size[0], size[1]).to_physical(window.scale_factor())
    } else {
        PhysicalSize::new(size[0], size[1])
    }
}
fn api_dimensions(window: &Window, size: PhysicalSize<u32>) -> [u32; 2] {
    if cfg!(target_os = "macos") {
        let s = size.to_logical::<u32>(window.scale_factor());
        [s.width, s.height]
    } else {
        [size.width, size.height]
    }
}
/// Whether editor opens are narrated to stderr. See [`trace_open`].
static TRACING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Narrate the building of one editor window, to stderr.
///
/// A plugin builds its GUI inside this call, on this thread, and a plugin that
/// stops there stops the rack with it: no window paints, no button answers, and
/// nothing on disk says what was asked for last. Timing each step tells the
/// three different things an operator then sees apart — a plugin that never came
/// back, a plugin that took seconds, and a rack that was fine at the moment it
/// looked frozen. stderr is unbuffered, so the last line survives a process that
/// has to be killed. Set `NICE_EDITOR_TRACE` to get these.
fn trace_open(text: &str) {
    if *TRACING.get_or_init(|| std::env::var_os("NICE_EDITOR_TRACE").is_some()) {
        eprintln!("[editor] {text}");
    }
}

#[cfg(test)]
mod tests {
    use super::Windows;
    use crate::{routing::NodeId, runtime::Runtime};

    /// The wanting is not the window. A chain that had to build a node's instance
    /// again leaves the frame opened for the instance before it standing over
    /// nothing, and that frame comes down — but the card is still in the rack, and
    /// its new instance is asked for an editor in the same frame this one fell, so
    /// what the user opened stays open. A plugin the edit did not touch never
    /// comes here: its instance was carried into the new chain, and its window
    /// never stopped being the one attached to it.
    #[test]
    fn a_frame_taken_down_leaves_its_wanting() {
        let mut windows = Windows::default();
        windows.open(NodeId(3));
        windows.open(NodeId(7));

        windows.close_window(NodeId(3));
        assert_eq!(
            windows.wanted,
            vec![NodeId(3), NodeId(7)],
            "taking a frame down should not cancel the editor it stood for"
        );

        windows.requeue_wanted();
        assert!(
            windows.pending.contains(&NodeId(3)),
            "the editor whose frame came down should be on its way back"
        );
    }

    /// Closing an editor is a decision the next rebuild does not get to overrule,
    /// which is the other half of remembering what the user wanted: the wanting
    /// ends when the editor ends, not when a frame does.
    #[test]
    fn a_closed_editor_stays_closed_through_a_rebuild() {
        let mut windows = Windows::default();
        windows.open(NodeId(3));
        windows.open(NodeId(7));
        windows.forget(NodeId(3));
        windows.close_window(NodeId(3));
        windows.close_window(NodeId(7));

        windows.requeue_wanted();
        assert_eq!(
            windows.pending,
            vec![NodeId(7)],
            "the editor that was closed should stay out of the queue"
        );
    }

    /// The reason the X11 loop exists, witnessed end to end with a real
    /// plugin and no UI: a frame built by the loop must carry an editor the
    /// way a frost window carries one on X11 — the same `open_editor` call
    /// the pump makes, against the same session type the rack runs. The
    /// gated loop test only proves a frame can be built; this one proves a
    /// plugin takes it. `NICE_RACK_TEST_X11` and `NICE_RACK_TEST_CLAP` are
    /// both required because a machine with no X server or no plugin has
    /// nothing to witness here.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires NICE_RACK_TEST_X11=1, a live X server, and NICE_RACK_TEST_CLAP"]
    #[allow(unsafe_code)]
    fn a_shim_frame_carries_a_real_plugin_editor() {
        use crate::runtime::{Card, Route, chain};
        use std::{collections::BTreeMap, path::PathBuf};

        assert!(
            std::env::var_os("NICE_RACK_TEST_X11").is_some(),
            "Set NICE_RACK_TEST_X11=1 to run against this machine's X server"
        );
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
        let card = Card::new(NodeId(2), path, entry, class, "Test plugin".into()).unwrap();
        // A route wide enough for the plugin both ways: this test witnesses
        // the frame, not the width transitions.
        let route = Route {
            input: crate::routing::Channels::Stereo,
            input_rate: None,
            output: crate::routing::Channels::Stereo,
            output_rate: None,
        };
        let (graph, bindings) = chain(&[card], route).unwrap();
        let (mut session, _audio) = nice_clap_host::LiveSession::prepare(
            &graph,
            &bindings,
            48000.0,
            1024,
            &BTreeMap::new(),
        )
        .expect("the test chain builds");

        let service = super::x11::Service::start().expect("the editor loop starts under XWayland");
        let opened = service
            .open(NodeId(2), "frost x11 editor test", None)
            .expect("the loop builds a frame");
        // The exact call `open_x11` makes, minus the rack around it.
        let opened_editor =
            session.open_editor(NodeId(2), opened.parent, None, opened.scale_factor);
        let result = match opened_editor {
            Ok((size, resizable)) => {
                println!("the plugin took the X11 frame: {size:?} resizable {resizable}");
                session.close_editor(NodeId(2));
                Ok(())
            }
            Err(error) => {
                println!("the plugin refused the X11 frame: {error}");
                Err(error)
            }
        };
        service.close(NodeId(2));
        service.quit();
        assert!(result.is_ok(), "the plugin refused the X11 frame");
    }

    /// The same question the test above asks on X11, asked here on Windows,
    /// where the frame comes out of this loop: an operator opened an editor and
    /// the whole rack went quiet — no window painted, no button answered — and
    /// nothing on disk said what had been asked for last.
    ///
    /// It walks the road the rack walks: a hidden frame from `create_window`,
    /// the same `open_editor`, then `Windows::service` and the session's own
    /// servicing out of `about_to_wait`, which is where the rack builds the
    /// frame at all. What is left out is frost's frame loop, so a thread that
    /// stops in here has stopped in the plugin: nothing between this and the
    /// plugin is the engine's.
    ///
    /// `NICE_RACK_TEST_CLAP` names the `.clap` to load. Every pass is announced
    /// on stderr before it starts, so a run that never comes back names the pass
    /// it stopped on, and a stop inside `service` is a stop inside `open_editor`.
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires NICE_RACK_TEST_CLAP and a desktop to open a window on"]
    #[allow(unsafe_code)]
    fn a_real_plugin_editor_does_not_wedge_the_window_thread() {
        use crate::runtime::{Card, Route, chain};
        use std::{
            collections::BTreeMap,
            path::PathBuf,
            time::{Duration, Instant},
        };
        use winit::application::ApplicationHandler;
        use winit::event_loop::{ActiveEventLoop, ControlFlow};
        use winit::platform::pump_events::EventLoopExtPumpEvents;
        use winit::platform::windows::EventLoopBuilderExtWindows;

        let path = PathBuf::from(
            std::env::var_os("NICE_RACK_TEST_CLAP")
                .expect("Set NICE_RACK_TEST_CLAP to the .clap to open an editor for"),
        );
        // SAFETY: explicit test opt-in to a plugin this machine named.
        let entry = unsafe { nice_clap_host::Entry::load(&path) }.unwrap();
        let class = entry
            .descriptors()
            .unwrap()
            .first()
            .expect("the plugin named by NICE_RACK_TEST_CLAP has no entry")
            .id
            .clone();
        let card = Card::new(NodeId(2), path, entry, class, "Probe plugin".into()).unwrap();
        let route = Route {
            input: crate::routing::Channels::Stereo,
            input_rate: None,
            output: crate::routing::Channels::Stereo,
            output_rate: None,
        };
        let cards = vec![card];
        let (graph, bindings) = chain(&cards, route).unwrap();
        let (session, _audio) = nice_clap_host::LiveSession::prepare(
            &graph,
            &bindings,
            48000.0,
            1024,
            &BTreeMap::new(),
        )
        .expect("the probe chain builds");

        struct Probe {
            windows: Windows,
            runtime: Runtime,
            passes: u32,
        }
        impl ApplicationHandler for Probe {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                event_loop.set_control_flow(ControlFlow::Poll);
            }
            fn window_event(
                &mut self,
                _event_loop: &ActiveEventLoop,
                window_id: winit::window::WindowId,
                event: winit::event::WindowEvent,
            ) {
                self.windows.event(window_id, &event, &mut self.runtime);
            }
            fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
                self.passes += 1;
                if self.passes < 4 || self.passes.is_multiple_of(2000) {
                    eprintln!(
                        "[probe] pass {}: busy {} cards {} pending {:?} wanted {:?} windows {}",
                        self.passes,
                        self.runtime.busy(),
                        self.runtime.cards.len(),
                        self.windows.pending,
                        self.windows.wanted,
                        self.windows.windows.len(),
                    );
                }
                self.windows.service(event_loop, &mut self.runtime);
                if let Some(session) = &mut self.runtime.session {
                    // The editor's own servicing, which is the part of
                    // `Runtime::tick` an open editor leans on. The rest of the
                    // tick builds chains and opens devices, which is neither on
                    // trial here nor something to do to this machine from a test.
                    let _ = session.service();
                }
            }
        }

        let mut runtime = Runtime::new(crate::Settings::default());
        runtime.cards = cards;
        runtime.session = Some(session);
        // The chain this probe stands in for is already up: `Runtime::new` starts
        // with the action that would put it there, and `busy()` reads that as a
        // chain still on its way — which is the one state in which the rack will
        // not build an editor for anything.
        runtime.chain_is_up();
        let mut windows = Windows::default();
        // The editor button's own call: the queue is what the pass below works off.
        windows.open(NodeId(2));
        let mut probe = Probe {
            windows,
            runtime,
            passes: 0,
        };

        // The harness runs this on a worker thread, and winit warns — loudly, by
        // panicking — about an event loop off the main thread. The hazard it
        // warns of is Cocoa's, and Windows takes it quietly; what is being
        // witnessed here is the plugin, not the loop's apartment.
        let mut event_loop = winit::event_loop::EventLoop::builder()
            .with_any_thread(true)
            .build()
            .unwrap();
        let begun = Instant::now();
        while begun.elapsed() < Duration::from_secs(6) {
            event_loop.pump_app_events(Some(Duration::from_millis(20)), &mut probe);
        }
        eprintln!("[probe] open survived: {} passes", probe.passes);
        assert!(
            probe.passes > 10,
            "the loop ran only {} passes",
            probe.passes
        );
        assert!(
            probe.windows.windows.contains_key(&NodeId(2)),
            "six seconds of service built no editor window"
        );
        assert!(
            probe
                .runtime
                .session
                .as_ref()
                .is_some_and(|session| session.has_editor(NodeId(2))),
            "the window stands over an instance with no editor"
        );

        // Closing is the other half of the walk: the editor's windows come down
        // between the plugin's `hide` and its `destroy`, and a frame still in
        // flight at that moment is a plugin drawing into a handle that is gone.
        if let Some(session) = &mut probe.runtime.session {
            session.close_editor(NodeId(2));
        }
        eprintln!("[probe] editor closed");
        let closed = Instant::now();
        while closed.elapsed() < Duration::from_secs(1) {
            event_loop.pump_app_events(Some(Duration::from_millis(20)), &mut probe);
        }
        eprintln!("[probe] closed survived: {} passes", probe.passes);
    }

    /// One editor open is not the workload, and measuring it as if it were is
    /// how the lag survived this long.
    ///
    /// Every editor's frames are clocked by a message posted every 15 ms (see
    /// `start_editor_frame_pump`), posted messages never coalesce, and every one
    /// of them is answered on the thread that also moves the windows — so N
    /// editors is N × 66 frames a second for one thread. What one pass of that
    /// thread's loop costs is therefore exactly how long a window drag, a click,
    /// or a knob is allowed to stall: the thread cannot answer a message in the
    /// middle of a pass.
    ///
    /// Measured on this machine with four of our own plugins open, one pass cost
    /// 1.9 s. The cause was not the number of frames: it was that the host draws
    /// its window on one graphics API and the plugin editors pick another per
    /// window, so every frame of every window is a device-context switch inside
    /// one thread. Pinning the whole process to one API took the same pass from
    /// 261 ms to 0.1 ms with two editors.
    ///
    /// Opens an editor for every plugin named in `NICE_RACK_TEST_CLAPS`
    /// (`;`-separated), pumps six seconds to let them all come up, then measures
    /// three seconds of steady state. Every steady pass must fit in
    /// [`PASS_BUDGET_MS`]; the opening pass is reported, not asserted, because
    /// building a GPU device is allowed to be slow once.
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires NICE_RACK_TEST_CLAPS and a desktop to open windows on"]
    #[allow(unsafe_code)]
    fn many_plugin_editors_do_not_drown_the_window_thread() {
        const PASS_BUDGET_MS: f64 = 33.0;

        // The line the rack's own `main` writes before anything else runs, so
        // that what this measures is the policy the product actually runs under
        // rather than an accident of which window was created first.
        frost::pin_gpu_backend();

        use crate::runtime::{Card, Route, chain};
        use std::{
            collections::BTreeMap,
            path::PathBuf,
            time::{Duration, Instant},
        };
        use winit::application::ApplicationHandler;
        use winit::event_loop::{ActiveEventLoop, ControlFlow};
        use winit::platform::pump_events::EventLoopExtPumpEvents;
        use winit::platform::windows::EventLoopBuilderExtWindows;

        /// The `n`th of a thousand parts of the samples, so `percentile(.., 0.95)`
        /// is the pass ninety-five percent of passes beat.
        fn percentile(samples: &mut [f64], part: f64) -> f64 {
            if samples.is_empty() {
                return 0.0;
            }
            samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let index = ((samples.len() - 1) as f64 * part).round() as usize;
            samples[index]
        }

        let paths: Vec<PathBuf> = std::env::var_os("NICE_RACK_TEST_CLAPS")
            .expect("Set NICE_RACK_TEST_CLAPS to `;`-separated .clap paths")
            .to_string_lossy()
            .split(';')
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect();
        assert!(!paths.is_empty(), "NICE_RACK_TEST_CLAPS named no plugin");

        let mut cards = Vec::new();
        let mut ids = Vec::new();
        for (nth, path) in paths.iter().enumerate() {
            let id = NodeId(nth as u64 + 2);
            // SAFETY: explicit test opt-in to the plugins this machine named.
            let entry = unsafe { nice_clap_host::Entry::load(path) }.unwrap();
            let class = entry
                .descriptors()
                .unwrap()
                .first()
                .unwrap_or_else(|| panic!("{} has no entry", path.display()))
                .id
                .clone();
            cards.push(Card::new(id, path.clone(), entry, class, format!("Probe {nth}")).unwrap());
            ids.push(id);
        }
        let route = Route {
            input: crate::routing::Channels::Stereo,
            input_rate: None,
            output: crate::routing::Channels::Stereo,
            output_rate: None,
        };
        let (graph, bindings) = match chain(&cards, route) {
            Ok(built) => built,
            Err(error) => {
                panic!(
                    "the probe chain of {} plugins does not build: {error:?}",
                    paths.len()
                )
            }
        };
        let (session, _audio) = nice_clap_host::LiveSession::prepare(
            &graph,
            &bindings,
            48000.0,
            1024,
            &BTreeMap::new(),
        )
        .expect("the probe chain builds");

        struct Probe {
            windows: Windows,
            runtime: Runtime,
            /// Wall time between one `about_to_wait` and the next: how long the
            /// thread was busy with something else before it could look at a new
            /// message again. A plugin's frames are answered inside the pump, in
            /// baseview's window proc, woken by the tick the pump thread posts —
            /// so that cost shows up here and nowhere in the splits below, and
            /// this is the span a window drag is actually waiting on.
            gaps: Vec<f64>,
            /// The part of that span this thread spent on purpose: building and
            /// ticking editor windows, and serving the CLAP main thread. What is
            /// left over is message handling and presents.
            host: Vec<f64>,
            clap: Vec<f64>,
            /// The work of one `about_to_wait`, `host` and `clap` added.
            passes: Vec<f64>,
            /// Running maxima rather than recomputed folds: a probe that folds
            /// its own samples on every pass is quadratic in the pass count, and
            /// a thread doing fifteen thousand passes a second turns that into a
            /// run that looks hung.
            worst_gap: f64,
            worst: f64,
            last: Option<Instant>,
        }
        impl Probe {
            fn record(&mut self, total: f64, host: f64, clap: f64) {
                self.passes.push(total);
                self.host.push(host);
                self.clap.push(clap);
                self.worst = self.worst.max(total);
            }
            /// The worst pass of what has been measured since the last reset.
            fn worst(&self) -> f64 {
                self.worst
            }
            fn reset(&mut self) {
                self.gaps.clear();
                self.host.clear();
                self.clap.clear();
                self.passes.clear();
                self.worst_gap = 0.0;
                self.worst = 0.0;
                self.last = None;
            }
        }
        impl ApplicationHandler for Probe {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                event_loop.set_control_flow(ControlFlow::Poll);
            }
            fn window_event(
                &mut self,
                _event_loop: &ActiveEventLoop,
                window_id: winit::window::WindowId,
                event: winit::event::WindowEvent,
            ) {
                self.windows.event(window_id, &event, &mut self.runtime);
            }
            fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
                let now = Instant::now();
                if let Some(previous) = self.last {
                    let gap = (now - previous).as_secs_f64() * 1000.0;
                    self.gaps.push(gap);
                    self.worst_gap = self.worst_gap.max(gap);
                }
                self.last = Some(now);
                let begun = Instant::now();
                self.windows.service(event_loop, &mut self.runtime);
                let host = begun.elapsed().as_secs_f64() * 1000.0;
                let begun = Instant::now();
                if let Some(session) = &mut self.runtime.session {
                    let _ = session.service();
                }
                let clap = begun.elapsed().as_secs_f64() * 1000.0;
                self.record(host + clap, host, clap);
            }
        }

        let mut runtime = Runtime::new(crate::Settings::default());
        runtime.cards = cards;
        runtime.session = Some(session);
        runtime.chain_is_up();
        let mut windows = Windows::default();
        for &id in &ids {
            windows.open(id);
        }
        let mut probe = Probe {
            windows,
            runtime,
            gaps: Vec::new(),
            host: Vec::new(),
            clap: Vec::new(),
            passes: Vec::new(),
            worst_gap: 0.0,
            worst: 0.0,
            last: None,
        };

        let mut event_loop = winit::event_loop::EventLoop::builder()
            .with_any_thread(true)
            .build()
            .unwrap();
        let begun = Instant::now();
        while begun.elapsed() < Duration::from_secs(6) {
            event_loop.pump_app_events(Some(Duration::from_millis(5)), &mut probe);
        }
        eprintln!(
            "[probe] {} editors standing after 6 s: worst gap {:.1} ms, worst work {:.1} ms",
            probe.windows.windows.len(),
            probe.worst_gap,
            probe.worst(),
        );

        // Steady state: every editor is up, nothing is being built. This is the
        // part of the run an operator is living in while they drag a window.
        probe.reset();
        let begun = Instant::now();
        let mut second = 1;
        let mut seen = 0;
        // Stop at the gap that is the runaway signature — a queue of frame
        // requests growing faster than the loop can drain it, which is a gap of
        // seconds, not milliseconds — and never run longer than twenty seconds.
        // Bailing at the first pass over budget instead measures forty
        // milliseconds of a three second run and reports it as steady, which is
        // how a late frame and a wedged thread read the same.
        const RUNAWAY_MS: f64 = 2_000.0;
        while begun.elapsed() < Duration::from_secs(3) {
            event_loop.pump_app_events(Some(Duration::from_millis(5)), &mut probe);
            if probe.worst_gap >= RUNAWAY_MS || begun.elapsed() > Duration::from_secs(20) {
                break;
            }
            if begun.elapsed().as_secs() >= second {
                eprintln!(
                    "[probe] steady second {second}: {} loop iterations, worst gap {:.1} ms",
                    probe.gaps.len() - seen,
                    probe.worst_gap,
                );
                seen = probe.gaps.len();
                second += 1;
            }
        }
        let seconds = begun.elapsed().as_secs_f64();
        let editors = probe.windows.windows.len();
        let samples = probe.gaps.len();
        let rate = samples as f64 / seconds;
        let (mut gaps, mut host, mut clap) = (probe.gaps, probe.host, probe.clap);
        let p50 = percentile(&mut gaps, 0.50);
        let p95 = percentile(&mut gaps, 0.95);
        let max = percentile(&mut gaps, 1.0);
        eprintln!(
            "[probe] {editors} editors steady: {samples} loop iterations in {seconds:.1} s \
             ({rate:.0}/s), gap p50 {p50:.2} p95 {p95:.2} max {max:.2} ms; work at p95 \
             {:.2} ms in the editors, {:.2} ms in the CLAP main thread",
            percentile(&mut host, 0.95),
            percentile(&mut clap, 0.95),
        );

        assert_eq!(
            editors,
            ids.len(),
            "only {editors} of {} editors were standing after six seconds of service",
            ids.len()
        );
        assert!(
            p95 < PASS_BUDGET_MS,
            "{editors} editors open: ninety-five percent of loop iterations took over \
             {p95:.1} ms and the worst took {max:.1} ms, against a {PASS_BUDGET_MS} ms budget. \
             Between one iteration and the next this thread cannot answer a message, so that \
             gap is the drag, the click and the knob. {samples} iterations were measured at \
             {rate:.0} a second, so it is not one unlucky frame.",
        );
    }

    /// Quitting is the one interruption after which nothing is asked for again:
    /// the rack is not coming back with a chain to build, and the editors still
    /// waiting for one are gone along with the instances that had them.
    #[test]
    fn quitting_the_rack_promises_nothing_back() {
        let mut windows = Windows::default();
        let mut runtime = Runtime::new(crate::Settings::default());
        runtime.closing = true;
        windows.open(NodeId(3));
        windows.dismissed.push(NodeId(3));

        windows.close(&mut runtime);
        assert!(
            windows.wanted.is_empty(),
            "a rack that is closing left an editor queued for a session that will not exist"
        );
        assert!(
            windows.dismissed.is_empty(),
            "a rack that is closing left an editor waiting for a session to close it"
        );
    }
}
