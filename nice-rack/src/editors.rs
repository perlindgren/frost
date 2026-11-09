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
}
impl Windows {
    pub fn open(&mut self, id: NodeId) {
        if !self.pending.contains(&id) {
            self.pending.push(id);
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
        self.windows.clear();
        self.pending.clear();
    }
    pub fn service(&mut self, event_loop: &ActiveEventLoop, runtime: &mut Runtime) {
        if runtime.busy() {
            self.close(runtime);
            return;
        }
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
                Err(error) => runtime.message = error,
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
