//! Main-thread controls. Native parenting is where we hand a plugin a window of
//! ours. A Win32 compatibility helper checks that an editor's child is shown;
//! redraw scheduling belongs to the plugin's toolkit.
use super::*;
use clack_host::events::{Pckn, event_types::ParamValueEvent};
use raw_window_handle::HasWindowHandle;
// The raw handle is used only by the Win32 compatibility helper below,
// so its import has the same platform gate. Named alongside
// `HasWindowHandle` instead, it would be an unused import on every platform but
// this one, which is a warning, and the gate turns warnings into errors on the
// platforms whose `cfg` arm this file also has to compile.
#[cfg(target_os = "windows")]
use raw_window_handle::RawWindowHandle;

pub(in crate::graph) struct Editor {
    // Holds the native parent alive until after PluginGui::destroy.
    _parent: Arc<dyn HasWindowHandle>,
    reveal_until: Option<std::time::Instant>,
}
impl Editor {
    pub(in crate::graph) fn service(&mut self) {
        if let Some(deadline) = self.reveal_until
            && (std::time::Instant::now() >= deadline
                || ensure_editor_child_shown(self._parent.as_ref()))
        {
            self.reveal_until = None;
        }
    }
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Parameter {
    pub id: u32,
    pub name: String,
    pub module: String,
    pub min: f64,
    pub max: f64,
    pub value: f64,
    pub text: String,
    pub stepped: bool,
    pub readonly: bool,
}
pub(in crate::graph) struct ParameterMetadata {
    id: ClapId,
    name: String,
    module: String,
    min: f64,
    max: f64,
    flags: ParamInfoFlags,
    formatted: Option<(f64, String)>,
}
impl Owner {
    fn refresh_parameters(&mut self) {
        if self.signals.0.params_changed.swap(false, Ordering::AcqRel) {
            self.parameters = None;
        }
        if self.parameters.is_some() {
            return;
        }
        let handle = self.instance.plugin_handle();
        let mut scratch = ParamInfoBuffer::new();
        self.parameters = Some(handle.get_extension::<PluginParams>().map_or_else(
            Vec::new,
            |params| {
                (0..params.count(&handle).min(16384))
                    .filter_map(|index| {
                        let info = params.get_info(&handle, index, &mut scratch)?;
                        Some(ParameterMetadata {
                            id: info.id,
                            name: String::from_utf8_lossy(info.name).into_owned(),
                            module: String::from_utf8_lossy(info.module).into_owned(),
                            min: info.min_value,
                            max: info.max_value,
                            flags: info.flags,
                            formatted: None,
                        })
                    })
                    .collect()
            },
        ));
    }
}
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum EditorEvent {
    Resize(u32, u32),
    Show,
    Hide,
    Closed,
}
impl HostGuiImpl for Callbacks {
    fn resize_hints_changed(&self) {}
    fn request_resize(&self, size: GuiSize) -> Result<(), HostError> {
        if size.width == 0 || size.height == 0 || size.width > 16384 || size.height > 16384 {
            return Err(HostError::Message("Invalid editor size"));
        }
        self.0
            .gui_resize
            .store(size.pack_to_u64(), Ordering::Release);
        Ok(())
    }
    fn request_show(&self) -> Result<(), HostError> {
        self.0.gui_visibility.store(1, Ordering::Release);
        Ok(())
    }
    fn request_hide(&self) -> Result<(), HostError> {
        self.0.gui_visibility.store(2, Ordering::Release);
        Ok(())
    }
    fn closed(&self, _: bool) {
        self.0.gui_visibility.store(3, Ordering::Release);
    }
}
impl HostParamsImplShared for Callbacks {
    fn request_flush(&self) {
        self.0.process.store(true, Ordering::Release);
    }
}
impl HostParamsImplMainThread for Callbacks {
    fn rescan(&self, flags: ParamRescanFlags) {
        if flags.intersects(ParamRescanFlags::VALUES | ParamRescanFlags::ALL) {
            self.0.changed.store(true, Ordering::Release);
        }
        self.0.params_changed.store(true, Ordering::Release);
    }
    fn clear(&self, _: ClapId, _: ParamClearFlags) {
        self.0.params_changed.store(true, Ordering::Release);
    }
}
impl LiveSession {
    pub fn editor_supported(&mut self, id: NodeId) -> bool {
        if let Some(owner) = self.remote_owners.get(&id) {
            return owner.editor_supported;
        }
        let Some(owner) = self.owners.get_mut(&id) else {
            return false;
        };
        let handle = owner.instance.plugin_handle();
        handle.get_extension::<PluginGui>().is_some_and(|gui| {
            gui.is_api_supported(
                &handle,
                GuiConfiguration {
                    api_type: platform_api(),
                    is_floating: false,
                },
            )
        })
    }
    /// Open a native embedded editor in a host-owned top-level window. The
    /// parent is retained internally until the editor has been destroyed.
    #[allow(unsafe_code)]
    pub fn open_editor<W: HasWindowHandle + 'static>(
        &mut self,
        id: NodeId,
        parent: Arc<W>,
        saved_size: Option<[u32; 2]>,
        scale_factor: f64,
    ) -> Result<([u32; 2], bool), String> {
        self.close_editor(id);
        let owner = self
            .owners
            .get_mut(&id)
            .ok_or("Plugin instance not found")?;
        let handle = owner.instance.plugin_handle();
        let gui = handle
            .get_extension::<PluginGui>()
            .ok_or("Plugin has no editor")?;
        let window = clack_extensions::gui::Window::from_window(parent.as_ref())
            .ok_or("Unsupported native window type")?;
        let config = GuiConfiguration {
            api_type: window.api_type(),
            is_floating: false,
        };
        if !gui.is_api_supported(&handle, config) {
            return Err("Plugin cannot embed its editor on this platform".into());
        }
        gui.create(&handle, config).map_err(|e| e.to_string())?;
        let result = (|| {
            if !cfg!(target_os = "macos") && scale_factor.is_finite() && scale_factor > 0.0 {
                let _ = gui.set_scale(&handle, scale_factor);
            }
            let resizable = gui.can_resize(&handle);
            if resizable
                && let Some([width, height]) =
                    saved_size.filter(|s| s.iter().all(|n| (1..=16384).contains(n)))
            {
                let size = gui
                    .adjust_size(&handle, GuiSize { width, height })
                    .ok_or("Plugin rejected saved size")?;
                gui.set_size(&handle, size).map_err(|e| e.to_string())?;
            }
            let size = gui
                .get_size(&handle)
                .ok_or("Plugin did not report editor size")?;
            if size.width == 0 || size.height == 0 || size.width > 16384 || size.height > 16384 {
                return Err("Invalid plugin editor size".into());
            }
            // SAFETY: parent stays alive locally through all error cleanup and
            // is retained by Owner on success. close/drop destroys the editor
            // before releasing it. All operations use the main-thread handle.
            unsafe { gui.set_parent(&handle, window) }.map_err(|e| e.to_string())?;
            gui.show(&handle).map_err(|e| e.to_string())?;
            Ok(([size.width, size.height], resizable))
        })();
        if result.is_err() {
            gui.destroy(&handle);
        } else {
            owner.editor = Some(Editor {
                _parent: parent,
                reveal_until: cfg!(target_os = "windows")
                    .then(|| std::time::Instant::now() + std::time::Duration::from_millis(500)),
            });
        }
        result
    }
    /// Whether the instance behind this node is showing an editor of ours.
    ///
    /// A rack window is built for one instance, and an instance is the only thing
    /// a CLAP editor can be attached to, so a window standing over a node that
    /// answers no here is a frame around an instance that has since been built
    /// again — the number it was opened for outlived the plugin it was opened on.
    /// It is the rack's to take down.
    pub fn has_editor(&self, id: NodeId) -> bool {
        if let Some(owner) = self.remote_owners.get(&id) {
            return owner.editor_open;
        }
        self.owners
            .get(&id)
            .is_some_and(|owner| owner.editor.is_some())
    }
    pub fn close_editor(&mut self, id: NodeId) {
        if let Some(owner) = self.remote_owners.get_mut(&id) {
            owner.close();
            return;
        }
        if let Some(owner) = self.owners.get_mut(&id)
            && let Some(parent) = owner.editor.take()
        {
            let handle = owner.instance.plugin_handle();
            if let Some(gui) = handle.get_extension::<PluginGui>() {
                let _ = gui.hide(&handle);
                gui.destroy(&handle);
            }
            drop(parent);
        }
    }
    pub fn close_editors(&mut self) {
        let ids: Vec<_> = self
            .owners
            .keys()
            .chain(self.remote_owners.keys())
            .copied()
            .collect();
        for id in ids {
            self.close_editor(id);
        }
    }
    pub fn resize_editor(&mut self, id: NodeId, size: [u32; 2]) -> Result<[u32; 2], String> {
        let owner = self
            .owners
            .get_mut(&id)
            .ok_or("Plugin instance not found")?;
        let handle = owner.instance.plugin_handle();
        let gui = handle
            .get_extension::<PluginGui>()
            .ok_or("Plugin has no editor")?;
        if gui
            .get_size(&handle)
            .is_some_and(|current| [current.width, current.height] == size)
        {
            return Ok(size);
        }
        if !gui.can_resize(&handle) {
            return gui
                .get_size(&handle)
                .map(|size| [size.width, size.height])
                .ok_or("Plugin did not report editor size".into());
        }
        let size = gui
            .adjust_size(
                &handle,
                GuiSize {
                    width: size[0],
                    height: size[1],
                },
            )
            .ok_or("Plugin rejected editor size")?;
        gui.set_size(&handle, size).map_err(|e| e.to_string())?;
        Ok([size.width, size.height])
    }
    pub fn editor_events(&self) -> Vec<(NodeId, EditorEvent)> {
        let mut events = Vec::new();
        for (&id, owner) in &self.owners {
            let packed = owner.signals.0.gui_resize.swap(0, Ordering::AcqRel);
            if packed != 0 {
                let size = GuiSize::unpack_from_u64(packed);
                events.push((id, EditorEvent::Resize(size.width, size.height)));
            }
            let visibility = owner.signals.0.gui_visibility.swap(0, Ordering::AcqRel);
            if let Some(event) = match visibility {
                1 => Some(EditorEvent::Show),
                2 => Some(EditorEvent::Hide),
                3 => Some(EditorEvent::Closed),
                _ => None,
            } {
                events.push((id, event));
            }
        }
        events
    }
    pub fn parameters(&mut self, id: NodeId) -> Vec<Parameter> {
        if let Some(owner) = self.remote_owners.get(&id) {
            return owner.parameters.clone();
        }
        let Some(owner) = self.owners.get_mut(&id) else {
            return Vec::new();
        };
        owner.refresh_parameters();
        let handle = owner.instance.plugin_handle();
        let Some(params) = handle.get_extension::<PluginParams>() else {
            return Vec::new();
        };
        let mut text = [0u8; 256];
        owner
            .parameters
            .as_mut()
            .unwrap()
            .iter_mut()
            .filter_map(|info| {
                if info.flags.contains(ParamInfoFlags::IS_HIDDEN) {
                    return None;
                }
                let value = params.get_value(&handle, info.id)?;
                if info.formatted.as_ref().is_none_or(|(old, _)| *old != value) {
                    let label = params
                        .value_to_text(&handle, info.id, value, &mut text)
                        .map(|s| String::from_utf8_lossy(s).into_owned())
                        .unwrap_or_else(|_| format!("{value:.3}"));
                    info.formatted = Some((value, label));
                }
                Some(Parameter {
                    id: info.id.get(),
                    name: info.name.clone(),
                    module: info.module.clone(),
                    min: info.min,
                    max: info.max,
                    value,
                    text: info.formatted.as_ref().unwrap().1.clone(),
                    stepped: info.flags.contains(ParamInfoFlags::IS_STEPPED),
                    readonly: info.flags.contains(ParamInfoFlags::IS_READONLY),
                })
            })
            .collect()
    }
    pub fn set_parameter(&mut self, id: NodeId, param: u32, value: f64) -> Result<(), String> {
        if let Some(owner) = self.remote_owners.get_mut(&id) {
            return owner.parameter(param, value);
        }
        if !value.is_finite() {
            return Err("Invalid parameter value".into());
        }
        let owner = self
            .owners
            .get_mut(&id)
            .ok_or("Plugin instance not found")?;
        owner.refresh_parameters();
        let info = owner
            .parameters
            .as_ref()
            .unwrap()
            .iter()
            .find(|info| info.id.get() == param)
            .map(|info| (info.id, info.flags, info.min, info.max))
            .ok_or("Parameter not found")?;
        let handle = owner.instance.plugin_handle();
        let params = handle
            .get_extension::<PluginParams>()
            .ok_or("Plugin has no parameters")?;
        if info.1.contains(ParamInfoFlags::IS_READONLY) {
            return Err("Parameter is read only".into());
        }
        let mut value = value.clamp(info.2, info.3);
        if info.1.contains(ParamInfoFlags::IS_STEPPED) {
            value = value.round();
        }
        let event = ParamValueEvent::new(0, info.0, Pckn::match_all(), value);
        if owner.instance.is_active() {
            owner
                .changes
                .push(event)
                .map_err(|_| "Parameter queue is full".to_owned())?;
        } else {
            let mut handle = owner
                .instance
                .inactive_plugin_handle()
                .ok_or("Plugin is active")?;
            params.flush(
                &mut handle,
                &InputEvents::from_buffer(&event),
                &mut OutputEvents::from(&mut OutputChanges::new(&owner.signals.0.changed)),
            );
        }
        owner.signals.0.changed.store(true, Ordering::Release);
        Ok(())
    }
}
fn platform_api() -> GuiApiType<'static> {
    #[cfg(target_os = "macos")]
    {
        GuiApiType::COCOA
    }
    #[cfg(target_os = "windows")]
    {
        GuiApiType::WIN32
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        GuiApiType::X11
    }
}
/// Show the plugin's window if its own `show()` did not.
///
/// CLAP's `show` is a request, and a toolkit can answer it without doing
/// anything. older baseview 0.3.3 editors created a
/// parented child with the style `WS_CHILD` and nothing else
/// (`wrappers/win32/style.rs`), and reaches `ShowWindow` from one arm of
/// `WindowHandle::show()` only (`platform/win/window.rs`). That arm, and the one
/// in `set_parent()`, both start by taking an unsynchronised
/// `Cell<Option<WindowInitializer>>` which the plugin's own GUI thread takes too
/// — its `run_until_closed()` calls `show()` as its first act — and whichever
/// caller finds the cell already taken returns `Ok(())` having shown nothing. So
/// our `gui.show()` can succeed truthfully while the child stays hidden, and a
/// hidden child is not a window that merely has not painted yet: it receives no
/// paint, no input, and no amount of invalidating the parent will reveal it. What
/// is left on screen is our own editor window with nothing in it, and no error
/// anywhere to explain it.
///
/// Asking Windows is only what the CLAP request already authorised, and it fixes
/// any plugin whose toolkit answers the request as literally. `ShowWindowAsync`
/// and not `ShowWindow`: the child belongs to the plugin's GUI thread, and a post
/// cannot freeze the rack if that thread stops listening. The child may not exist
/// yet either — it is created on that other thread. Editor::service retries
/// once per UI tick for up to half a second; opening never sleeps waiting for
/// another GUI thread to create or show its child.
///
/// The cost of doing it by window rather than by plugin: a toolkit that kept a
/// helper window of its own hidden *directly inside* our editor window would get
/// it shown too. Nothing in the CLAP contract gives a plugin a hidden child in a
/// host window, and the alternative — trusting the one call that is documented to
/// have done it already — is what left the editor empty, so the visible-by-default
/// guarantee wins here.
#[allow(unsafe_code)]
fn ensure_editor_child_shown(parent: &dyn HasWindowHandle) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::ptr::null;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            FindWindowExW, GWL_STYLE, GetWindowLongPtrW, SW_SHOW, ShowWindowAsync, WS_VISIBLE,
        };
        let Ok(handle) = parent.window_handle() else {
            return true;
        };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return true;
        };
        let mut child = unsafe { FindWindowExW(win32.hwnd.get(), 0, null(), null()) };
        let mut seen = false;
        let mut all_visible = true;
        while child != 0 {
            seen = true;
            if (unsafe { GetWindowLongPtrW(child, GWL_STYLE) } as u32) & WS_VISIBLE == 0 {
                all_visible = false;
                unsafe { ShowWindowAsync(child, SW_SHOW) };
            }
            child = unsafe { FindWindowExW(win32.hwnd.get(), child, null(), null()) };
        }
        seen && all_visible
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = parent;
        true
    }
}
#[cfg(test)]
mod parameter_change_tests {
    use super::*;
    #[test]
    fn a_value_rescan_marks_state_dirty_and_invalidates_metadata() {
        let signals = Callbacks::default();
        signals.rescan(ParamRescanFlags::VALUES);
        assert!(signals.0.changed.load(Ordering::Acquire));
        assert!(signals.0.params_changed.load(Ordering::Acquire));
    }
    #[test]
    fn text_rescan_invalidates_formatting_without_claiming_a_state_change() {
        let signals = Callbacks::default();
        signals.rescan(ParamRescanFlags::TEXT);
        assert!(!signals.0.changed.load(Ordering::Acquire));
        assert!(signals.0.params_changed.load(Ordering::Acquire));
    }
}
