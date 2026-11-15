//! Standalone native editor windows, driven by frost's existing event loop.
use crate::{EditorGeometry, routing::NodeId, runtime::Runtime};
use std::{collections::BTreeMap, sync::Arc};
use winit::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::WindowEvent,
    event_loop::ActiveEventLoop,
    window::{Window, WindowId},
};
#[derive(Default)]
pub(crate) struct Windows {
    windows: BTreeMap<NodeId, Arc<Window>>,
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
}
impl Windows {
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
        self.wanted.retain(|wanted| *wanted != id);
        self.pending.retain(|pending| *pending != id);
    }
    /// Take a frame down without giving up on the editor. The plugin its editor
    /// was drawn by is gone; the card that names that plugin is still in the rack,
    /// and the instance built for it now is as willing a host as the one before
    /// it, so `wanted` keeps the id and `requeue_wanted` asks again — in the same
    /// frame this one comes down.
    fn close_window(&mut self, id: NodeId) {
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
        for (&id, window) in &self.windows {
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
    }
    pub fn close(&mut self, runtime: &mut Runtime) {
        self.capture(runtime);
        if let Some(session) = &mut runtime.session {
            session.close_editors();
        }
        if runtime.closing {
            // Quitting is the one interruption after which nothing is asked for
            // again: the rack is not coming back with a chain to build.
            self.wanted.clear();
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
                if let Some(window) = self.windows.get(&id) {
                    match event {
                        nice_clap_host::EditorEvent::Resize(w, h) => {
                            let _ = window.request_inner_size(api_size(window, [w, h]));
                        }
                        nice_clap_host::EditorEvent::Show => window.set_visible(true),
                        nice_clap_host::EditorEvent::Hide => window.set_visible(false),
                        nice_clap_host::EditorEvent::Closed => {
                            session.close_editor(id);
                            self.windows.remove(&id);
                            self.forget(id);
                        }
                    }
                } else if matches!(event, nice_clap_host::EditorEvent::Show) {
                    self.open(id);
                }
            }
        }
        for id in std::mem::take(&mut self.pending) {
            if let Some(window) = self.windows.get(&id) {
                window.set_visible(true);
                window.focus_window();
                continue;
            }
            let Some(card) = runtime.cards.iter().find(|c| c.id == id) else {
                // The card left the rack, and its instance with it, so an
                // editor for it is a window that can never be built again.
                self.forget(id);
                continue;
            };
            let saved = runtime.editor_geometry.get(&id).copied();
            let attributes = Window::default_attributes()
                .with_title(&card.name)
                .with_visible(false)
                .with_inner_size(LogicalSize::new(640.0, 480.0));
            let result = (|| {
                let window = Arc::new(
                    event_loop
                        .create_window(attributes)
                        .map_err(|e| e.to_string())?,
                );
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
                window.set_resizable(resizable);
                let _ = window.request_inner_size(api_size(&window, size));
                window.set_visible(true);
                Ok::<_, String>(window)
            })();
            match result {
                Ok(window) => {
                    self.windows.insert(id, window);
                }
                // A plugin that will not open an editor should not be asked
                // every frame, so the asking ends with the failure.
                Err(error) => {
                    self.forget(id);
                    runtime.message = error;
                }
            }
        }
    }
    pub fn event(&mut self, id: WindowId, event: &WindowEvent, runtime: &mut Runtime) -> bool {
        let Some((&node, window)) = self.windows.iter().find(|(_, w)| w.id() == id) else {
            return false;
        };
        match event {
            WindowEvent::CloseRequested => {
                self.capture(runtime);
                if runtime.busy() || runtime.session.is_none() {
                    self.dismissed.push(node);
                } else if let Some(session) = &mut runtime.session {
                    session.close_editor(node);
                }
                self.windows.remove(&node);
                self.forget(node);
                runtime.persist();
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
