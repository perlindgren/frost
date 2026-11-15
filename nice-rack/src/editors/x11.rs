//! Editor windows for the one platform setup frost's own window cannot serve.
//!
//! A CLAP editor is handed its native parent as one of three things: an X11
//! window id, a Win32 handle, or a Cocoa view pointer. The ABI reserves the
//! name "wayland" for a future parent type, but its handle union still carries
//! no slot for one, and a plugin asked to parent into a Wayland surface says
//! no — our own wrapper among them, today. Frost runs Wayland-native, so its
//! window has no handle the CLAP GUI API can name, and before this loop existed
//! an editor opened under Wayland simply failed with "Unsupported native
//! window type".
//!
//! So the editors get their own frames from a second, X11-only windowing loop
//! running under XWayland. Those frames carry an X11 parent a plugin will
//! accept, and everything else about an editor — the wanting, the saved
//! geometry, the plugin handshake — stays the `Windows` layer's business. It
//! talks to this loop through commands (a winit window belongs to the thread
//! that created it, so there is no other way in) and through answers: window
//! events the session must hear about on the rack's own beat.
//!
//! When Clack learns to hand out a real Wayland parent and plugins say yes to
//! it, this file deletes itself: `Windows` hands the plugin the rack's own
//! window again and never starts this loop.

use crate::routing::NodeId;
use crossbeam::channel::{Receiver, RecvTimeoutError, Sender};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::platform::x11::EventLoopBuilderExtX11;
use winit::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use winit::window::{Window, WindowId};

/// How long the loop gets to answer before we call it absent. Starting a
/// second windowing loop is a millisecond matter when XWayland is there, and
/// when it is not there no amount of waiting produces one.
const WAIT: Duration = Duration::from_secs(10);

/// One act on the loop. Every frame change is a message because winit windows
/// belong to the thread that built them; there is no handle to poke from the
/// rack's thread, only requests to make of the loop that owns them.
enum Command {
    Open {
        id: NodeId,
        reply: Sender<Result<Opened, String>>,
        title: String,
        position: Option<[i32; 2]>,
    },
    Visible {
        id: NodeId,
        visible: bool,
    },
    Focus {
        id: NodeId,
    },
    Resize {
        id: NodeId,
        size: [u32; 2],
    },
    Resizable {
        id: NodeId,
        resizable: bool,
    },
    Close {
        id: NodeId,
    },
    Quit,
}

/// A frame the loop has built and the rack may now attach a plugin to.
pub(crate) struct Opened {
    pub(crate) parent: Arc<X11Parent>,
    pub(crate) scale_factor: f64,
    pub(crate) inner: [u32; 2],
    pub(crate) outer: Option<[i32; 2]>,
}

/// What the loop saw happen to a frame, for the main thread to fold into the
/// session on its own beat.
pub(crate) enum WorkerEvent {
    Resized {
        id: NodeId,
        size: [u32; 2],
        scale: f64,
    },
    Moved {
        id: NodeId,
        position: [i32; 2],
        scale: f64,
    },
    CloseRequested {
        id: NodeId,
    },
}

/// The rack's address for the editor loop, once it is up.
pub(crate) struct Service {
    commands: EventLoopProxy<Command>,
    events: Receiver<WorkerEvent>,
}

impl Service {
    /// Start the loop, waiting for it to say it is up. An editor is opened on
    /// the strength of that answer, so a machine without XWayland should hear
    /// why now rather than watch a click do nothing.
    pub(crate) fn start() -> Result<Self, String> {
        let (ready_tx, ready_rx) = crossbeam::channel::bounded(1);
        let (events_tx, events_rx) = crossbeam::channel::unbounded();
        std::thread::spawn(move || run(events_tx, ready_tx));
        match ready_rx.recv_timeout(WAIT) {
            Ok(Ok(commands)) => Ok(Self {
                commands,
                events: events_rx,
            }),
            Ok(Err(error)) => Err(format!("The X11 editor loop could not start: {error}")),
            Err(RecvTimeoutError::Timeout) => Err(
                "Opening plugin editors needs XWayland, and it did not answer in time".to_string(),
            ),
            Err(RecvTimeoutError::Disconnected) => {
                Err("The X11 editor loop died before it started".to_string())
            }
        }
    }

    /// Ask the loop for a frame for `id`, waiting for it to be built: the
    /// plugin is attached to its handle right after, and the handle has to be
    /// real by then.
    pub(crate) fn open(
        &self,
        id: NodeId,
        title: &str,
        position: Option<[i32; 2]>,
    ) -> Result<Opened, String> {
        let (reply_tx, reply_rx) = crossbeam::channel::bounded(1);
        self.send(Command::Open {
            id,
            reply: reply_tx,
            title: title.to_string(),
            position,
        });
        match reply_rx.recv_timeout(WAIT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                Err("The X11 editor loop did not answer in time".to_string())
            }
            Err(RecvTimeoutError::Disconnected) => {
                Err("The X11 editor loop has stopped".to_string())
            }
        }
    }

    pub(crate) fn visible(&self, id: NodeId, visible: bool) {
        self.send(Command::Visible { id, visible });
    }

    pub(crate) fn focus(&self, id: NodeId) {
        self.send(Command::Focus { id });
    }

    /// Ask for an inner size, in physical pixels — the units a CLAP editor
    /// speaks when the rack's compositor is not translating (Linux: it is
    /// not; `api_dimensions` is the identity there).
    pub(crate) fn resize(&self, id: NodeId, size: [u32; 2]) {
        self.send(Command::Resize { id, size });
    }

    pub(crate) fn resizable(&self, id: NodeId, resizable: bool) {
        self.send(Command::Resizable { id, resizable });
    }

    /// Destroy a frame. The rack only sends this once the plugin has released
    /// its editor, so the window never goes away under the plugin's hands.
    pub(crate) fn close(&self, id: NodeId) {
        self.send(Command::Close { id });
    }

    pub(crate) fn quit(&self) {
        self.send(Command::Quit);
    }

    /// Everything the loop saw since last time, oldest first.
    pub(crate) fn drain_events(&self) -> Vec<WorkerEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            events.push(event);
        }
        events
    }

    fn send(&self, command: Command) {
        // A closed loop is a loop that is gone, and its frames went with it.
        // Nothing useful is left to say about that mid-gesture; the next
        // answer the rack asks of the loop will report the loss.
        let _ = self.commands.send_event(command);
    }
}

/// The loop's thread. It lives as long as the rack wants X11 frames and dies
/// on `Quit`, which the rack sends only after every editor is closed.
fn run(events_tx: Sender<WorkerEvent>, ready_tx: Sender<Result<EventLoopProxy<Command>, String>>) {
    // The forced-X11 build is what makes this loop's windows carry the one
    // native parent kind the CLAP GUI API accepts here; winit calls
    // `XInitThreads` before opening its display, so this connection and any
    // a plugin opens share Xlib safely.
    let mut builder = EventLoop::<Command>::with_user_event();
    builder.with_x11();
    builder.with_any_thread(true);
    let event_loop = match builder.build() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            let _ = ready_tx.send(Err(error.to_string()));
            return;
        }
    };
    let commands = event_loop.create_proxy();
    if ready_tx.send(Ok(commands)).is_err() {
        // The rack stopped waiting for this loop — it timed out while the
        // loop was coming up. Nothing owns these frames now; let the loop
        // drop and take them along.
        return;
    }
    let mut app = Loop {
        windows: BTreeMap::new(),
        ids: HashMap::new(),
        events: events_tx,
    };
    // `run_app` is the maintained drive of this winit — frost's own loop goes
    // through it too; the closure form it replaces is deprecated.
    let _ = event_loop.run_app(&mut app);
}

/// The loop's frames, by both names they are known under: the rack's node and
/// winit's window.
struct Loop {
    windows: BTreeMap<NodeId, Arc<Window>>,
    ids: HashMap<WindowId, NodeId>,
    events: Sender<WorkerEvent>,
}

impl ApplicationHandler<Command> for Loop {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn user_event(&mut self, event_loop: &ActiveEventLoop, command: Command) {
        match command {
            Command::Open {
                id,
                reply,
                title,
                position,
            } => {
                // Exactly one answer per request: the rack waits on this
                // channel and a missing reply is an editor that never appears.
                let _ = reply.send(open_window(
                    event_loop,
                    &mut self.windows,
                    &mut self.ids,
                    id,
                    &title,
                    position,
                ));
            }
            Command::Visible { id, visible } => {
                if let Some(window) = self.windows.get(&id) {
                    window.set_visible(visible);
                }
            }
            Command::Focus { id } => {
                if let Some(window) = self.windows.get(&id) {
                    window.focus_window();
                }
            }
            Command::Resize { id, size } => {
                if let Some(window) = self.windows.get(&id) {
                    let _ = window.request_inner_size(PhysicalSize::new(size[0], size[1]));
                }
            }
            Command::Resizable { id, resizable } => {
                if let Some(window) = self.windows.get(&id) {
                    window.set_resizable(resizable);
                }
            }
            Command::Close { id } => {
                // Out of both maps before the last handle falls: a destroyed
                // window must not stay addressable by either of its names.
                if let Some(window) = self.windows.remove(&id) {
                    self.ids.remove(&window.id());
                }
            }
            Command::Quit => event_loop.exit(),
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(&id) = self.ids.get(&window_id) else {
            return;
        };
        match event {
            WindowEvent::Resized(size) if size.width > 0 && size.height > 0 => {
                let scale = self
                    .windows
                    .get(&id)
                    .map_or(1.0, |window| window.scale_factor());
                let _ = self.events.send(WorkerEvent::Resized {
                    id,
                    size: [size.width, size.height],
                    scale,
                });
            }
            WindowEvent::Moved(position) => {
                let scale = self
                    .windows
                    .get(&id)
                    .map_or(1.0, |window| window.scale_factor());
                let _ = self.events.send(WorkerEvent::Moved {
                    id,
                    position: [position.x, position.y],
                    scale,
                });
            }
            WindowEvent::CloseRequested => {
                let _ = self.events.send(WorkerEvent::CloseRequested { id });
            }
            _ => {}
        }
    }
}

fn open_window(
    event_loop: &ActiveEventLoop,
    windows: &mut BTreeMap<NodeId, Arc<Window>>,
    ids: &mut HashMap<WindowId, NodeId>,
    id: NodeId,
    title: &str,
    position: Option<[i32; 2]>,
) -> Result<Opened, String> {
    if windows.contains_key(&id) {
        return Err("An editor window is already open for this plugin".to_string());
    }
    // Built hidden, like the frost-loop path: a frame appears once the
    // plugin is in it, never as an empty box the plugin might never join.
    let window = Arc::new(
        event_loop
            .create_window(
                Window::default_attributes()
                    .with_title(title)
                    .with_visible(false)
                    .with_inner_size(LogicalSize::new(640.0, 480.0)),
            )
            .map_err(|error| error.to_string())?,
    );
    if let Some(position) = position {
        // Honor the saved position only where some monitor this loop can
        // actually see can carry it — a frame the compositor cannot place is
        // worse than one it must re-place.
        if event_loop.available_monitors().any(|monitor| {
            let point = monitor.position();
            let size = monitor.size();
            position[0] >= point.x
                && position[1] >= point.y
                && position[0] < point.x + size.width as i32 - 40
                && position[1] < point.y + size.height as i32 - 40
        }) {
            window.set_outer_position(PhysicalPosition::new(position[0], position[1]));
        }
    }
    let handle = window.window_handle().map_err(|error| error.to_string())?;
    let inner = window.inner_size();
    let opened = Opened {
        parent: Arc::new(X11Parent(handle.as_raw())),
        scale_factor: window.scale_factor(),
        inner: [inner.width, inner.height],
        outer: window.outer_position().ok().map(|p| [p.x, p.y]),
    };
    ids.insert(window.id(), id);
    windows.insert(id, window);
    Ok(opened)
}

/// A plugin parent that names the X11 window another thread's loop created.
///
/// [`winit::window::Window`] is deliberately `!Send`: its innards belong to
/// the thread that made it. The handle it hands out is not innards — on X11
/// it is a window id and a pointer to the `Display` connection, and Xlib
/// serializes use of that connection across threads because `XInitThreads`
/// was called before it opened (winit does this in its own X11 backend). A
/// plugin reparenting through these numbers from the rack's thread is
/// speaking Xlib the same legal way the loop is.
// The rack forbids `unsafe` crate-wide, and this file is the one place its
// editors cannot be built without it: `raw_window_handle` marks its handles
// `!Send` out of caution about lifetimes, and `WindowHandle::borrow_raw` is
// the only constructor there is. Every other editor path in this workspace
// crosses the same line the same way — see `nice-clap-host`'s window
// plumbing and `nice-common`'s ASIO transport.
#[allow(unsafe_code)]
pub(crate) struct X11Parent(RawWindowHandle);

// SAFETY: the handle carries X resource ids and a thread-safe `Display`
// pointer, not exclusive process state, and the referenced X window outlives
// every borrow taken while its frame is in the rack's hands: the loop keeps
// the `Window` alive until a `Close`, which the rack sends only after the
// plugin has released its editor.
#[allow(unsafe_code)]
unsafe impl Send for X11Parent {}
// SAFETY: as above — all borrows are short per-call reads of the same X11
// handle, serialized by Xlib's own connection locking.
#[allow(unsafe_code)]
unsafe impl Sync for X11Parent {}

#[allow(unsafe_code)]
impl HasWindowHandle for X11Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: the window `self.0` names is alive for as long as this
        // parent is reachable — see the `Send`/`Sync` note above.
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

/// The live state of an X11 frame, kept on the rack's main thread so geometry
/// can be captured without asking the loop anything.
pub(crate) struct Frame {
    pub(crate) inner: [u32; 2],
    pub(crate) scale: f64,
    pub(crate) outer: Option<[i32; 2]>,
}

impl Frame {
    pub(crate) fn new(opened: &Opened) -> Self {
        Self {
            inner: opened.inner,
            scale: opened.scale_factor,
            outer: opened.outer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// The loop's own contract, witnessed against a live X server rather
    /// than asserted from reading: it answers, it builds a frame that
    /// carries an X11 parent a plugin could be handed, it reports the
    /// resize it was asked for, and it takes the frame down and shuts up
    /// when told to. `NICE_RACK_TEST_CLAP` names its audio-bearing
    /// siblings for the same reason — a machine with no X server has
    /// nothing to witness here, and a refusal beats a lie about one.
    #[test]
    #[ignore = "requires a live X server (XWayland counts); set NICE_RACK_TEST_X11=1"]
    fn the_loop_answers_builds_reports_and_takes_down() {
        assert!(
            std::env::var_os("NICE_RACK_TEST_X11").is_some(),
            "Set NICE_RACK_TEST_X11=1 to run against this machine's X server"
        );
        let service = Service::start().expect("the editor loop starts under a live X server");
        let id = NodeId(1);
        let opened = match service.open(id, "frost x11 loop test", None) {
            Ok(opened) => opened,
            Err(error) => panic!("the loop refused to build a frame: {error}"),
        };
        // The one thing this whole loop exists for: a handle naming an X11
        // window, ready to be handed to a plugin.
        assert!(
            matches!(
                opened.parent.window_handle().map(|h| h.as_raw()),
                Ok(RawWindowHandle::Xlib(_))
            ),
            "a frame must carry an Xlib parent"
        );
        assert!(opened.inner[0] > 0 && opened.inner[1] > 0);
        // A second request while the frame stands is refused in words, and
        // the refusal travels the same reply channel as the frame did.
        let again = service.open(id, "frost x11 loop test", None);
        assert!(again.is_err(), "a standing frame must not be opened twice");
        // Ask for a size and wait for the loop to say it happened.
        service.resizable(id, true);
        service.resize(id, [321, 123]);
        let deadline = Instant::now() + WAIT;
        let mut saw_size = false;
        while Instant::now() < deadline && !saw_size {
            for event in service.drain_events() {
                if let WorkerEvent::Resized {
                    id: event_id, size, ..
                } = event
                {
                    assert_eq!(event_id, id);
                    saw_size = size == [321, 123];
                }
            }
            if !saw_size {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        assert!(
            saw_size,
            "the loop never reported the size it was asked for"
        );
        service.close(id);
        service.quit();
        // Nothing is asserted after `Quit`: winit may or may not drain what
        // was queued around the exit, and an answer of "gone" arriving one
        // way or another is the same promise the live paths already make.
    }
}
