//! Main-thread controls; native parenting is the sole unsafe operation here.
use super::*;
use clack_host::events::{Pckn, event_types::ParamValueEvent};
use raw_window_handle::HasWindowHandle;

pub(in crate::graph) struct Editor {
    // Holds the native parent alive until after PluginGui::destroy.
    _parent: Arc<dyn HasWindowHandle>,
}
#[derive(Clone, Debug)]
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
#[derive(Debug)]
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
    fn rescan(&self, _: ParamRescanFlags) {
        self.0.params_changed.store(true, Ordering::Release);
    }
    fn clear(&self, _: ClapId, _: ParamClearFlags) {
        self.0.params_changed.store(true, Ordering::Release);
    }
}
impl LiveSession {
    pub fn editor_supported(&mut self, id: NodeId) -> bool {
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
            owner.editor = Some(Editor { _parent: parent });
        }
        result
    }
    pub fn close_editor(&mut self, id: NodeId) {
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
        let ids: Vec<_> = self.owners.keys().copied().collect();
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
        let Some(owner) = self.owners.get_mut(&id) else {
            return Vec::new();
        };
        let handle = owner.instance.plugin_handle();
        let Some(params) = handle.get_extension::<PluginParams>() else {
            return Vec::new();
        };
        let mut scratch = ParamInfoBuffer::new();
        let mut text = [0u8; 256];
        (0..params.count(&handle).min(16384))
            .filter_map(|index| {
                let info = params.get_info(&handle, index, &mut scratch)?;
                if info.flags.contains(ParamInfoFlags::IS_HIDDEN) {
                    return None;
                }
                let value = params.get_value(&handle, info.id)?;
                Some(Parameter {
                    id: info.id.get(),
                    name: String::from_utf8_lossy(info.name).into_owned(),
                    module: String::from_utf8_lossy(info.module).into_owned(),
                    min: info.min_value,
                    max: info.max_value,
                    value,
                    text: params
                        .value_to_text(&handle, info.id, value, &mut text)
                        .map(|s| String::from_utf8_lossy(s).into_owned())
                        .unwrap_or_else(|_| format!("{value:.3}")),
                    stepped: info.flags.contains(ParamInfoFlags::IS_STEPPED),
                    readonly: info.flags.contains(ParamInfoFlags::IS_READONLY),
                })
            })
            .collect()
    }
    pub fn set_parameter(&mut self, id: NodeId, param: u32, value: f64) -> Result<(), String> {
        if !value.is_finite() {
            return Err("Invalid parameter value".into());
        }
        let owner = self
            .owners
            .get_mut(&id)
            .ok_or("Plugin instance not found")?;
        let handle = owner.instance.plugin_handle();
        let params = handle
            .get_extension::<PluginParams>()
            .ok_or("Plugin has no parameters")?;
        let mut buffer = ParamInfoBuffer::new();
        let info = (0..params.count(&handle).min(16384))
            .find_map(|i| {
                params
                    .get_info(&handle, i, &mut buffer)
                    .filter(|info| info.id.get() == param)
                    .map(|info| (info.id, info.flags, info.min_value, info.max_value))
            })
            .ok_or("Parameter not found")?;
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
                &mut OutputEvents::void(),
            );
        }
        owner.signals.0.changed.store(true, Ordering::Release);
        Ok(())
    }
    /// CLAP state save is a main-thread operation and is permitted while active.
    pub fn save_states(&mut self) -> Result<BTreeMap<NodeId, Vec<u8>>, String> {
        let mut states = BTreeMap::new();
        for (&id, owner) in &mut self.owners {
            if let Some(bytes) = save_state(&mut owner.instance)? {
                states.insert(id, bytes);
            }
        }
        Ok(states)
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
