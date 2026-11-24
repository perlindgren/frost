//! Main-thread controls. Native parenting is where we hand a plugin a window of
//! ours; the Win32 calls at the end of this file are the two places we then ask
//! Windows about the child inside it — once to be sure it got shown, once to
//! keep its frame clock running because our own UI thread starves it.
use super::*;
use clack_host::events::{Pckn, event_types::ParamValueEvent};
use raw_window_handle::HasWindowHandle;
// The raw handle is named only by the two Win32 helpers at the end of this file,
// so it is imported under the same `cfg` they are written in. Named alongside
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
            start_editor_frame_pump(parent.as_ref());
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
        self.owners
            .get(&id)
            .is_some_and(|owner| owner.editor.is_some())
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
/// One tick is outstanding at a time, because posting is not the same as timing:
/// a posted message, unlike a timer, never coalesces, so an unconditional 15 ms
/// post demands 66 frames a second from the UI thread per editor, forever,
/// whether or not the last one was ever drawn. Measured on this machine with four
/// editors open and all four drawing: one turn of the host loop that had not
/// begun for 50 seconds. Every tick waiting in the queue is another message that
/// must be drained before that loop may wait, and draining is what makes the
/// next frame late, so the queue feeds itself. The pump therefore posts when the
/// previous tick has evidently been dispatched — one pass of the host loop since
/// the post, counted by `editor_passes` — and posts again after 100 ms of no such
/// pass, which is also how a host gone quiet is woken up.
///
/// What is posted to is deliberately narrow: a direct child of *our* editor
/// window whose class name starts with baseview's prefix, ticked only while it
/// is visible and our window is not minimized, from the moment it exists until
/// either it or our window stops existing. A plugin built on another toolkit is
/// never sent anything, because its frame clock is its own business.
///
/// The child is looked for until it appears rather than for a fixed spell, since
/// baseview creates it on whichever thread answers the CLAP request and a slow
/// attach is not a missing one: quitting early was a way to lose an editor with
/// no error anywhere. It was not the cause of the editors that stopped drawing
/// here — traced, all four children were present within 50 µs of the pump
/// starting, which is what moved the suspicion back onto the machine.
///
/// The coupling to own: 4242 is baseview's private timer id, not an API. A
/// baseview that renamed or re-armed it would stop drawing silently again — the
/// same silence that hid this bug — which is why the class check is there and
/// why this comment names the file to look in. Nor does this fix the cause:
/// frost's UI thread still burns about a third of a core with nothing on
/// screen, and fixing that is what would make this pump unnecessary.
/// Whether the editor pumps say what they found. `NICE_EDITOR_TRACE` set to
/// anything turns it on; a release build is otherwise `release_max_level_off`,
/// which is why every trace in this area is an `eprintln!` and not `log`.
#[cfg(target_os = "windows")]
fn trace_on() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("NICE_EDITOR_TRACE").is_some())
}

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
        /// How often the pump looks, which is finer than the tick it posts: the
        /// wait for an answered tick is bounded by this, not by the period.
        const POLL_PERIOD: Duration = Duration::from_millis(5);
        /// How often it looks when there is nothing to draw: no editor window
        /// yet, an editor the operator hid, or a rack sitting minimized.
        const IDLE_PERIOD: Duration = Duration::from_millis(50);
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
        // How the pump learns its tick was answered: one turn of the host's UI
        // loop is one pass, so the number going up since the post is the proof.
        let served = crate::editor_passes();
        /// How long a posted tick may sit unanswered before the pump assumes the
        /// host loop is idle rather than slow, and posts another to wake it. A
        /// request left in a queue also wakes the wait that queue is parked on,
        /// which is how an idle host starts drawing again.
        const RECOVERY: Duration = Duration::from_millis(100);
        /// How long our baseview child may be missing after having been there.
        /// baseview destroys it when the editor closes, a moment before our own
        /// window goes; past that the pump is watching a corpse.
        const GONE_TOLERANCE: Duration = Duration::from_millis(1000);

        let _ = std::thread::Builder::new()
            .name("nice-clap-editor-tick".to_owned())
            .spawn(move || {
                // The window may still be on its way: baseview creates it on
                // whichever thread answers the CLAP request first, and a pump
                // that gave up after a fixed spell could silently lose an editor
                // whose window arrived late. Measured, that turned out not to be
                // what was wrong here — every child was there within 50 µs — but
                // it is a way to lose an editor without a sound, so the pump now
                // ends only when the window itself does, or when a child it had
                // is gone. `NICE_EDITOR_TRACE` prints which of those happened,
                // and whether a child it found ever counted as worth drawing.
                let started = Instant::now();
                let complain_at = started + Duration::from_secs(2);
                let mut announced = false;
                let mut complained = false;
                let mut judged = false;
                let mut was_worth = false;
                let mut found_once = false;
                let mut missed = Duration::ZERO;
                let mut seen = served.load(Ordering::Acquire);
                let mut outstanding = false;
                let mut posted = Instant::now();
                let mut next = Instant::now();
                /// The class name of a window, for the trace only. Returns an
                /// empty string for a nameless window rather than failing.
                fn class_name(hwnd: isize) -> String {
                    let mut name = [0u16; 64];
                    let len = unsafe { GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32) }
                        .clamp(0, name.len() as i32) as usize;
                    String::from_utf16_lossy(&name[..len])
                }
                loop {
                    let now = Instant::now();
                    let mut nap = POLL_PERIOD;
                    let mut child = unsafe { FindWindowExW(top, 0, null(), null()) };
                    let mut editor = 0;
                    let mut mine = String::new();
                    while child != 0 {
                        let mut name = [0u16; 64];
                        let len =
                            unsafe { GetClassNameW(child, name.as_mut_ptr(), name.len() as i32) }
                                .clamp(0, name.len() as i32) as usize;
                        if len >= BASEVIEW.len() && name[..BASEVIEW.len()] == BASEVIEW {
                            editor = child;
                            if trace_on() {
                                mine = class_name(child);
                            }
                            break;
                        }
                        child = unsafe { FindWindowExW(top, child, null(), null()) };
                    }
                    if editor != 0 {
                        found_once = true;
                        missed = Duration::ZERO;
                        if !announced && trace_on() {
                            announced = true;
                            eprintln!(
                                "[pump] 0x{top:x}: found {mine} after {:?}",
                                started.elapsed()
                            );
                        }
                        // Ticking a hidden editor would be the plugin rendering
                        // at sixty frames a second for nobody.
                        let worth_drawing = unsafe { IsWindowVisible(editor) } != 0
                            && unsafe { IsIconic(top) } == 0;
                        if trace_on() && (!judged || worth_drawing != was_worth) {
                            judged = true;
                            was_worth = worth_drawing;
                            eprintln!(
                                "[pump] 0x{top:x}: {mine} is {} (child visible {}, ours \
                                 minimized {})",
                                if worth_drawing {
                                    "on screen"
                                } else {
                                    "off screen"
                                },
                                unsafe { IsWindowVisible(editor) } != 0,
                                unsafe { IsIconic(top) } != 0
                            );
                        }
                        // The queue this tick went into is the one the host
                        // drains once per pass, so a pass since the post is the
                        // proof the tick was dispatched and may be followed.
                        if served.load(Ordering::Acquire) != seen {
                            outstanding = false;
                        }
                        if !worth_drawing {
                            // Nothing on screen to fill, so nothing worth
                            // waiting an hour for: coming back costs one post.
                            // Looking twenty times a second instead of two
                            // hundred is what a hidden editor should cost.
                            outstanding = false;
                            nap = IDLE_PERIOD;
                        } else if now >= next && (!outstanding || now - posted >= RECOVERY) {
                            // Read before the post, so that what is being waited
                            // for is a pass that can have seen this message.
                            seen = served.load(Ordering::Acquire);
                            let _ = unsafe { PostMessageW(editor, WM_TIMER, FRAME_TICK, 0) };
                            outstanding = true;
                            posted = now;
                            next = now + TICK_PERIOD;
                        }
                    } else {
                        // No child of ours to clock right now. Twenty looks a
                        // second costs nothing, and the window is not going to
                        // arrive any sooner for being waited on harder.
                        nap = IDLE_PERIOD;
                        if unsafe { IsWindow(top) } == 0 {
                            return;
                        }
                        if found_once {
                            // It was here and now it is not: the editor is being
                            // closed, and our own window is next. Polling it for
                            // the rest of the session would be the leak this pump
                            // is supposed to avoid, so retire and say so.
                            missed += IDLE_PERIOD;
                            if missed >= GONE_TOLERANCE {
                                if trace_on() {
                                    eprintln!(
                                        "[pump] 0x{top:x}: our child is gone, retiring after \
                                         {:?}",
                                        started.elapsed()
                                    );
                                }
                                return;
                            }
                        } else if !complained && now >= complain_at {
                            complained = true;
                            if trace_on() {
                                // One look two levels deep: a toolkit that nests
                                // its drawing window one row down would otherwise
                                // look identical to one that is not there.
                                let mut list = String::new();
                                let mut kid = unsafe { FindWindowExW(top, 0, null(), null()) };
                                while kid != 0 {
                                    list.push_str(&class_name(kid));
                                    let mut inner =
                                        unsafe { FindWindowExW(kid, 0, null(), null()) };
                                    while inner != 0 {
                                        list.push_str(" > ");
                                        list.push_str(&class_name(inner));
                                        inner =
                                            unsafe { FindWindowExW(kid, inner, null(), null()) };
                                    }
                                    list.push_str(", ");
                                    kid = unsafe { FindWindowExW(top, kid, null(), null()) };
                                }
                                if list.is_empty() {
                                    list.push_str("none");
                                }
                                eprintln!(
                                    "[pump] 0x{top:x}: never had a baseview child, {:?} in; \
                                     underneath this window: {list}",
                                    started.elapsed()
                                );
                            }
                        }
                    }
                    sleep(nap);
                }
            });
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = parent;
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
