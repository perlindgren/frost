//! Main-thread controls. Native parenting is where we hand a plugin a window of
//! ours; the Win32 calls at the end of this file are the two places we then ask
//! Windows about the child inside it — once to be sure it got shown, once to
//! keep its frame clock running because our own UI thread starves it.
use super::*;
use clack_host::events::{Pckn, event_types::ParamValueEvent};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

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
            ensure_editor_child_shown(parent.as_ref());
            start_editor_frame_pump(parent.as_ref());
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
/// Show the plugin's window if its own `show()` did not.
///
/// CLAP's `show` is a request, and a toolkit can answer it without doing
/// anything. baseview 0.3.3, which every `nice-*` editor embeds, creates a
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
/// yet either — it is created on that other thread — hence the short wait, which
/// returns the moment a visible child is there. That is the ordinary case, and it
/// costs one `FindWindowExW`; the half second it can spend is what an editor open
/// may wait when the other thread is the one being slow.
///
/// The cost of doing it by window rather than by plugin: a toolkit that kept a
/// helper window of its own hidden *directly inside* our editor window would get
/// it shown too. Nothing in the CLAP contract gives a plugin a hidden child in a
/// host window, and the alternative — trusting the one call that is documented to
/// have done it already — is what left the editor empty, so the visible-by-default
/// guarantee wins here.
#[allow(unsafe_code)]
fn ensure_editor_child_shown(parent: &dyn HasWindowHandle) {
    #[cfg(target_os = "windows")]
    {
        use std::ptr::null;
        use std::thread::sleep;
        use std::time::{Duration, Instant};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            FindWindowExW, GWL_STYLE, GetWindowLongPtrW, SW_SHOW, ShowWindowAsync, WS_VISIBLE,
        };

        let Ok(handle) = parent.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return;
        };
        let parent_hwnd = win32.hwnd.get();
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut requests = 0;
        loop {
            // Walk the direct children of our editor window. The window exists
            // to hold this one plugin's GUI, so any child inside it is the
            // plugin's and is meant to be seen.
            let mut child = unsafe { FindWindowExW(parent_hwnd, 0, null(), null()) };
            let mut seen = false;
            let mut hidden = None;
            while child != 0 {
                seen = true;
                let style = unsafe { GetWindowLongPtrW(child, GWL_STYLE) };
                if (style as u32) & WS_VISIBLE == 0 {
                    hidden = Some(child);
                    break;
                }
                child = unsafe { FindWindowExW(parent_hwnd, child, null(), null()) };
            }
            if let Some(child) = hidden {
                if requests < 3 {
                    unsafe { ShowWindowAsync(child, SW_SHOW) };
                    requests += 1;
                }
            } else if seen {
                return;
            }
            if Instant::now() >= deadline {
                return;
            }
            sleep(Duration::from_millis(5));
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = parent;
    }
}
/// Keep the plugin editor drawing when our own UI thread cannot clock it.
///
/// baseview paints a window from one source only: the `WM_TIMER` whose id is its
/// private `WIN_FRAME_TIMER`, armed for 15 ms inside `BaseviewWindow::create`
/// (`platform/win/window.rs`) on whichever thread called `set_parent` — for us
/// that is frost's UI thread. The window procedure ignores that message unless
/// `wparam` carries the id, and ignores everything else for painting, so nothing
/// else can produce a frame.
///
/// Windows does not queue a `WM_TIMER` when the interval elapses; it generates
/// one inside `GetMessage`/`PeekMessage`, and only when the thread's queue holds
/// no other message. A UI thread that keeps handing work to itself therefore
/// never produces timer messages at all, however diligently it pumps. Measured
/// on the live rack, the thread owning the editor child used 36.3 % of a core
/// with an editor open and 37.9 % with none — the spin is not the editor's — and
/// 93.7 % with the rack window minimized, so its queue is never empty and the
/// clock never fires. That is the whole bug: a window that exists, is visible,
/// owns a working swapchain, and is never once asked to draw a frame, which
/// leaves it the colour of its class background brush forever with no error
/// anywhere to say so. Posting the tick by hand from outside the process changed
/// that window from one colour to 247 within a second, while posting the same
/// message with a different id changed nothing, which is what isolates the clock
/// as the only broken link. REAPER's UI thread goes idle between frames, which
/// is why the same plugin draws there and not here.
///
/// A thread of our own does the posting because a second `SetTimer` would starve
/// in exactly the same way. `PostMessageW` does not wait for the receiver, so a
/// plugin that stops listening cannot freeze the rack through us.
///
/// What is posted to is deliberately narrow: a direct child of *our* editor
/// window whose class name starts with baseview's prefix, ticked only while it
/// is visible and our window is not minimized, and only until that window stops
/// existing. A plugin built on another toolkit is never sent anything, because
/// its frame clock is its own business.
///
/// The coupling to own: 4242 is baseview's private timer id, not an API. A
/// baseview that renamed or re-armed it would stop drawing silently again — the
/// same silence that hid this bug — which is why the class check is there and
/// why this comment names the file to look in. Nor does this fix the cause:
/// frost's UI thread still burns about a third of a core with nothing on
/// screen, and fixing that is what would make this pump unnecessary.
#[allow(unsafe_code)]
fn start_editor_frame_pump(parent: &dyn HasWindowHandle) {
    #[cfg(target_os = "windows")]
    {
        use std::ptr::null;
        use std::thread::sleep;
        use std::time::{Duration, Instant};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            FindWindowExW, GetClassNameW, IsIconic, IsWindow, IsWindowVisible, PostMessageW,
            WM_TIMER,
        };

        /// baseview's `WIN_FRAME_TIMER`, and the period it arms it with.
        const FRAME_TICK: usize = 4242;
        const TICK_PERIOD: Duration = Duration::from_millis(15);
        /// baseview names each window class `Baseview-` plus a GUID, so the
        /// prefix identifies the toolkit without hard-coding the GUID.
        const BASEVIEW: [u16; 9] = [
            b'B' as u16,
            b'a' as u16,
            b's' as u16,
            b'e' as u16,
            b'v' as u16,
            b'i' as u16,
            b'e' as u16,
            b'w' as u16,
            b'-' as u16,
        ];

        let Ok(handle) = parent.window_handle() else {
            return;
        };
        let RawWindowHandle::Win32(win32) = handle.as_raw() else {
            return;
        };
        let top = win32.hwnd.get();

        let _ = std::thread::Builder::new()
            .name("nice-clap-editor-tick".to_owned())
            .spawn(move || {
                // The window may still be on its way: baseview creates it on
                // whichever thread answers the CLAP request first. Give it two
                // seconds, then decide there is nothing of ours to clock.
                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    let mut child = unsafe { FindWindowExW(top, 0, null(), null()) };
                    let mut editor = 0;
                    while child != 0 {
                        let mut name = [0u16; 64];
                        let len =
                            unsafe { GetClassNameW(child, name.as_mut_ptr(), name.len() as i32) };
                        if len as usize >= BASEVIEW.len() && name[..BASEVIEW.len()] == BASEVIEW {
                            editor = child;
                            break;
                        }
                        child = unsafe { FindWindowExW(top, child, null(), null()) };
                    }
                    if editor != 0 {
                        // Ticking a hidden editor would be the plugin rendering
                        // at sixty frames a second for nobody.
                        let worth_drawing = unsafe { IsWindowVisible(editor) } != 0
                            && unsafe { IsIconic(top) } == 0;
                        if worth_drawing {
                            let _ = unsafe { PostMessageW(editor, WM_TIMER, FRAME_TICK, 0) };
                        }
                    } else if Instant::now() >= deadline || unsafe { IsWindow(top) } == 0 {
                        return;
                    }
                    sleep(TICK_PERIOD);
                }
            });
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = parent;
    }
}
