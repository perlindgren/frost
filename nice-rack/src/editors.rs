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
    /// A roster edit destroys every plugin instance, and a CLAP editor cannot
    /// outlive its instance, so an open editor goes down with the chain — but
    /// nobody asked for that, and the new chain is built out of the state the
    /// old one saved (`Runtime::states` reaches `LiveSession::prepare`). So an
    /// id stays in here for as long as its owner wants the window, through
    /// every rebuild and through a rollback that rebuilds the old chain, and
    /// leaves when the user closes the editor, when the plugin closes it, when
    /// its card is gone, or when the rack itself quits. Everything on screen or
    /// on its way is in here; the reverse is not true while a rebuild has the
    /// chain down.
    wanted: Vec<NodeId>,
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
    }
    pub fn service(&mut self, event_loop: &ActiveEventLoop, runtime: &mut Runtime) {
        if runtime.busy() {
            self.close(runtime);
            return;
        }
        // Anything the user still wants that the rebuild took away goes back
        // on the queue: dropping a plugin into the rack is not a request to
        // close the editors of the plugins that were already there.
        self.requeue_wanted();
        if let Some(session) = &mut runtime.session {
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
                if let Some(session) = &mut runtime.session {
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

    /// Dropping a second plugin into the rack rebuilds the chain, which
    /// destroys every plugin instance, and a CLAP editor cannot outlive its
    /// instance — so the editors of the plugins that stayed went down with it.
    /// Nobody asked for that, so what the user wants open is now kept apart
    /// from what happens to be on screen: the windows go, the wanting does not,
    /// and the first idle frame after the new chain queues them again. The
    /// window server stays out of it because every step here only moves ids.
    #[test]
    fn a_rebuilt_chain_buys_its_editors_back() {
        let mut windows = Windows::default();
        let mut runtime = Runtime::new(crate::Settings::default());
        windows.open(NodeId(3));
        windows.open(NodeId(7));

        windows.close(&mut runtime);
        assert!(
            windows.windows.is_empty() && windows.pending.is_empty(),
            "an edit should close the windows"
        );
        assert_eq!(
            windows.wanted,
            vec![NodeId(3), NodeId(7)],
            "an edit should not cancel the editors"
        );

        // `service` asks every frame from here on, and a second request for an
        // editor already on its way would re-focus it each time.
        for _ in 0..3 {
            windows.requeue_wanted();
        }
        assert_eq!(windows.pending, vec![NodeId(3), NodeId(7)]);
    }

    /// Closing an editor is a decision the next rebuild does not get to
    /// overrule, which is the other half of remembering what the user wanted:
    /// the wanting ends when the editor ends.
    #[test]
    fn a_closed_editor_stays_closed_through_a_rebuild() {
        let mut windows = Windows::default();
        let mut runtime = Runtime::new(crate::Settings::default());
        windows.open(NodeId(3));
        windows.open(NodeId(7));
        windows.forget(NodeId(3));

        windows.close(&mut runtime);
        windows.requeue_wanted();
        assert_eq!(
            windows.pending,
            vec![NodeId(7)],
            "the editor that was closed should stay out of the queue"
        );
    }

    #[test]
    fn quitting_the_rack_promises_nothing_back() {
        let mut windows = Windows::default();
        let mut runtime = Runtime::new(crate::Settings::default());
        runtime.closing = true;
        windows.open(NodeId(3));

        windows.close(&mut runtime);
        assert!(
            windows.wanted.is_empty(),
            "a rack that is closing left an editor queued for a session that will not exist"
        );
    }
}
