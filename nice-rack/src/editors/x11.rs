//! Editor windows for the one platform setup frost's own window cannot serve.
//!
//! A CLAP editor is handed its native parent as one of three things: an X11
//! window id, a Win32 handle, or a Cocoa view pointer. The ABI reserves the
//! name "wayland" for a future parent type — `clap-sys` ships the
//! `CLAP_WINDOW_API_WAYLAND` string — but the `clap_window_handle` union
//! still carries slots for `cocoa`, `x11` and `win32` only, no slot for a
//! surface, and a plugin asked to parent into a Wayland surface says no: our
//! own wrapper among them, today. `clack-extensions` says it outright, with
//! its `supports_window_embedding` answering false for Wayland and true for
//! the three kinds the union carries. Frost runs Wayland-native, so its
//! window has no handle the CLAP GUI API can name, and before this loop
//! existed an editor opened under Wayland simply failed with "Unsupported
//! native window type".
//!
//! So the editors get their own frames over a second, X11-only connection
//! running under XWayland. Those frames carry an X11 parent a plugin will
//! accept, and everything else about an editor — the wanting, the saved
//! geometry, the plugin handshake — stays the `Windows` layer's business. It
//! talks to this loop through commands (the frame bookkeeping belongs to the
//! pump thread, so there is no other way in) and through answers: window
//! events the session must hear about on the rack's own beat.
//!
//! The loop speaks Xlib directly — through `x11-dl`, not winit — because
//! winit 0.30 allows exactly one live event loop per process, flagged the
//! moment the first one is built no matter which backend it took. Frost's
//! Wayland loop already holds that flag, so a second winit `EventLoop` for
//! these frames failed with `RecreationAttempt` even though it would never
//! touch a Wayland window, and the flag is not behind any public knob. Xlib
//! has no such rule: `XInitThreads` serializes connection use across
//! threads, so one pump thread drives every frame on a single blocking
//! `XNextEvent`, woken for queued commands by a `ClientMessage` to the
//! hidden window the [`Connection`] keeps for exactly that.
//!
//! When the CLAP ABI grows a real Wayland parent slot, clap-wrapper and the
//! GUI toolkits learn to embed a `wl_surface`, and plugins say yes to it,
//! this file deletes itself: `Windows` hands the plugin the rack's own
//! window again and never starts this loop. Until then, two honest limits
//! travel with it: scale factors are always 1.0 here, because Xlib reports
//! no per-window scale and XWayland applies HiDPI translation for its own
//! windows anyway; and Xlib's I/O error handler is left as shipped — if the
//! X connection dies mid-session, Xlib ends the process, as it does for
//! every other X11 client.

use crate::routing::NodeId;
use crossbeam::channel::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::collections::{BTreeMap, HashMap};
use std::ffi::{CString, c_int, c_ulong};
use std::ptr::{self, NonNull};
use std::sync::Arc;
use std::time::Duration;
use winit::raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, WindowHandle, XlibWindowHandle,
};
use x11_dl::xlib::{
    self, Display, Visual, XClassHint, XClientMessageEvent, XErrorEvent, XEvent,
    XSetWindowAttributes, Xlib,
};

/// How long the loop gets to answer before we call it absent. Opening an
/// Xlib connection is a millisecond matter when XWayland is there, and when
/// it is not there no amount of waiting produces one.
const WAIT: Duration = Duration::from_secs(10);

/// The size a frame is born at, in pixels. Plugins are asked for their
/// preferred size right after attaching and the frame is resized then,
/// before it is ever shown.
const INITIAL_SIZE: [u32; 2] = [640, 480];

/// One act on the loop. Every frame change is a message because the frame
/// bookkeeping belongs to the pump thread; a second thread poking the same
/// map would need locks the message queue already makes unnecessary. (Xlib
/// itself is safe from any thread — `Service::send` proves it by ringing the
/// wake window — but the windows are the pump's.)
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
    commands: Sender<Command>,
    connection: Arc<Connection>,
    events: Receiver<WorkerEvent>,
}

impl Service {
    /// Start the loop, waiting for it to say it is up. An editor is opened on
    /// the strength of that answer, so a machine without XWayland should hear
    /// why now rather than watch a click do nothing.
    pub(crate) fn start() -> Result<Self, String> {
        let (commands_tx, commands_rx) = crossbeam::channel::unbounded();
        let (ready_tx, ready_rx) = crossbeam::channel::bounded(1);
        let (events_tx, events_rx) = crossbeam::channel::unbounded();
        std::thread::spawn(move || run(commands_rx, events_tx, ready_tx));
        match ready_rx.recv_timeout(WAIT) {
            Ok(Ok(connection)) => Ok(Self {
                commands: commands_tx,
                connection,
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
        let _ = self.commands.send(command);
        // The pump sleeps inside `XNextEvent` until X has something for it,
        // and a queued command is not something X knows about. Ring the
        // loop's wake window and the pump is on its way before this returns.
        // The ring deliberately comes *after* the queue: a pump woken early
        // by an earlier ring still drains the channel, while a ring sent
        // before the queue could wake a pump that finds nothing, parks
        // again, and sleeps straight past the command that lands after it.
        self.connection.notify();
    }
}

/// The loop's thread. It lives as long as the rack wants X11 frames and dies
/// on `Quit`, which the rack sends only after every editor is closed.
fn run(
    commands: Receiver<Command>,
    events: Sender<WorkerEvent>,
    ready: Sender<Result<Arc<Connection>, String>>,
) {
    let connection = match Connection::open() {
        Ok(connection) => Arc::new(connection),
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    if ready.send(Ok(Arc::clone(&connection))).is_err() {
        // The rack stopped waiting for this loop — it timed out while the
        // connection was being opened. Nobody will ever send a command here;
        // let the connection drop and go.
        return;
    }
    let mut pump = Loop {
        connection,
        windows: BTreeMap::new(),
        ids: HashMap::new(),
        events,
    };
    pump.pump(&commands);
}

/// Xlib answers every protocol error — a window destroyed under our hands,
/// say — by calling this instead of running its own handler, which exits the
/// process. There is nothing to retry and nothing to report: the loop and
/// the rack's `send` address resources whose loss they already absorb, and
/// real losses still surface as the missing answers the rack reads anyway.
// The declaration itself is unsafe code — an `extern "C"` body may only be
// entered through C — so it carries the same item-level allowance as every
// other hand-written C interaction in this file.
#[allow(unsafe_code)]
unsafe extern "C" fn swallow_x_errors(_display: *mut Display, _error: *mut XErrorEvent) -> c_int {
    0
}

/// The loop's own Xlib connection, its screen facts, the atoms it needs, and
/// the hidden window the rack rings when it queues a command.
///
/// The connection is deliberately never closed. A `Service::send` racing the
/// pump's shutdown must find a live connection to ring — `XInitThreads` made
/// concurrent use of it legal — and a wake window destroyed between the ring
/// and its delivery costs nothing but a swallowed `BadWindow` in the error
/// handler. At process exit the X server reclaims the rest anyway.
struct Connection {
    x11: Xlib,
    display: NonNull<Display>,
    screen: c_int,
    root: c_ulong,
    visual: *mut Visual,
    visual_id: c_ulong,
    /// The rack's bell: a 1×1 window that is never mapped and never gets an
    /// event mask of any kind. `XSendEvent`ing it a `ClientMessage` is how
    /// the rack wakes the parked pump, and no mask is needed for that: the
    /// mask libX11 derives from `ClientMessage` is 0, so the event is
    /// delivered regardless of what the window selects — the same mechanism
    /// by which window managers reach any window with `WM_DELETE_WINDOW`.
    wake: c_ulong,
    wake_atom: c_ulong,
    wm_protocols: c_ulong,
    delete_atom: c_ulong,
    net_wm_name: c_ulong,
    net_wm_pid: c_ulong,
    utf8_string: c_ulong,
    x_atom: c_ulong,
}

// SAFETY: every method is a call through `Xlib`, whose connection use this
// process serialized with `XInitThreads` before the display was opened
// (checked in `open`); the loaded library behind `Xlib` is `Send` and `Sync`
// in libloading itself, and the remaining fields are numbers and a pointer
// into the display's own read-only visual list.
#[allow(unsafe_code)]
unsafe impl Send for Connection {}
// SAFETY: as above — shared access to the connection is Xlib's to
// serialize, not ours to coordinate.
#[allow(unsafe_code)]
unsafe impl Sync for Connection {}

// The rack forbids `unsafe` crate-wide, and this file is the one place its
// editors cannot be built without it: Xlib is a C API of raw pointers, and
// `x11-dl` hands it out as declared-`unsafe` functions over raw pointers.
// Every other editor path in this workspace crosses the same line the same
// way — see `nice-clap-host`'s window plumbing and `nice-common`'s ASIO
// transport.
#[allow(unsafe_code)]
impl Connection {
    /// Open the loop's own connection. Xlib is asked to serialize first —
    /// frost's loop never touches this connection, but every plugin
    /// reparented into a frame here opens its own conversation through the
    /// same library, and two threads writing one socket unpickled corrupt it.
    fn open() -> Result<Self, String> {
        let x11 = Xlib::open().map_err(|error| format!("Xlib could not be loaded: {error}"))?;
        // `x11-dl` exposes Xlib as raw function-pointer fields, so every
        // call below is a C call made by hand.
        // SAFETY: each call is an Xlib entry point given arguments its own
        // documentation declares valid: `XOpenDisplay` with a null name
        // opens `$DISPLAY` or returns null (checked here); the screen,
        // root, depth and visual are read back from that same live display;
        // the wake window is a 1×1 child of the root in the display's own
        // default visual and depth with a zero valuemask, so Xlib reads no
        // attribute struct at all; every atom name is a NUL-terminated
        // literal. None of these calls unwind Rust: Xlib reports protocol
        // errors through the handler installed here, never by unwinding.
        unsafe {
            if (x11.XInitThreads)() == 0 {
                return Err("Xlib would not become thread-safe (XInitThreads failed)".to_string());
            }
            (x11.XSetErrorHandler)(Some(swallow_x_errors));
            let Some(display) = NonNull::new((x11.XOpenDisplay)(ptr::null())) else {
                return Err(format!(
                    "X11 opened no connection for frost's editor frames (DISPLAY is {})",
                    std::env::var("DISPLAY").unwrap_or_else(|_| "unset".to_string())
                ));
            };
            let d = display.as_ptr();
            let screen = (x11.XDefaultScreen)(d);
            let visual = (x11.XDefaultVisual)(d, screen);
            if visual.is_null() {
                return Err("the X screen carries no default visual".to_string());
            }
            // Never mapped, never selected on: the bell described on `wake`.
            // An empty valuemask keeps the choice explicit — nothing about
            // this window needs to be set for a ClientMessage to reach it.
            let wake = (x11.XCreateWindow)(
                d,
                (x11.XRootWindow)(d, screen),
                0,
                0,
                1,
                1,
                0,
                (x11.XDefaultDepth)(d, screen),
                xlib::InputOutput as _,
                visual,
                0,
                ptr::null_mut(),
            );
            if wake == 0 {
                return Err("Xlib refused the loop's wake window".to_string());
            }
            // Read the remaining facts before the struct literal: writing
            // `x11` into it moves the table, and the calls below need it.
            let root = (x11.XRootWindow)(d, screen);
            let wake_atom = (x11.XInternAtom)(d, c"_NICE_RACK_EDITOR_WAKE".as_ptr(), xlib::False);
            let wm_protocols = (x11.XInternAtom)(d, c"WM_PROTOCOLS".as_ptr(), xlib::False);
            let delete_atom = (x11.XInternAtom)(d, c"WM_DELETE_WINDOW".as_ptr(), xlib::False);
            let net_wm_name = (x11.XInternAtom)(d, c"_NET_WM_NAME".as_ptr(), xlib::False);
            let net_wm_pid = (x11.XInternAtom)(d, c"_NET_WM_PID".as_ptr(), xlib::False);
            let utf8_string = (x11.XInternAtom)(d, c"UTF8_STRING".as_ptr(), xlib::False);
            // x11-dl ships no xatom constants; `ATOM` is the type every
            // property holding atoms is typed with.
            let x_atom = (x11.XInternAtom)(d, c"ATOM".as_ptr(), xlib::False);
            Ok(Self {
                x11,
                display,
                screen,
                root,
                visual,
                visual_id: (*visual).visualid,
                wake,
                wake_atom,
                wm_protocols,
                delete_atom,
                net_wm_name,
                net_wm_pid,
                utf8_string,
                x_atom,
            })
        }
    }

    fn display(&self) -> *mut Display {
        self.display.as_ptr()
    }

    /// Ring the pump: queue a `ClientMessage` at the wake window and push it
    /// out now. The shape window managers use for `WM_DELETE_WINDOW` —
    /// `propagate: false`, no event mask — and the same reason that works
    /// everywhere: a ClientMessage is delivered regardless of what the
    /// window's event mask selects.
    fn notify(&self) {
        // SAFETY: a synthetic event is Xlib's documented use of the `XEvent`
        // union: the `client_message` arm is fully written before the event
        // is passed, its `display` and `window` name this connection and its
        // own wake window, and `XSendEvent` only reads the event. `XFlush`
        // pushes this thread's own output buffer.
        unsafe {
            let mut event: XEvent = std::mem::zeroed();
            event.client_message = XClientMessageEvent {
                type_: xlib::ClientMessage,
                serial: 0,
                send_event: xlib::True,
                display: self.display(),
                window: self.wake,
                message_type: self.wake_atom,
                format: 8,
                data: std::mem::zeroed(),
            };
            (self.x11.XSendEvent)(self.display(), self.wake, xlib::False, 0, &mut event);
            // The pump may already be parked in `XNextEvent`; a request
            // still sitting in this thread's output buffer wakes nobody.
            (self.x11.XFlush)(self.display());
        }
    }

    /// Republish a frame's WM size hints: where it was placed, when the
    /// rack's saved position was honored — `USPosition` tells the window
    /// manager that placement is chosen, not a guess — and when the plugin
    /// wants a fixed size, min set to max so the WM cannot drag the frame
    /// away from the size the plugin asked for.
    fn size_hints(&self, state: &FrameState) {
        // SAFETY: `XAllocSizeHints` returns an allocation of the library's
        // own current `XSizeHints` layout — or null, which is checked — and
        // only plain integer fields are written into it before
        // `XSetWMNormalHints` copies it out; `XFree` then frees exactly the
        // allocation Xlib made. `state.window` names one of this loop's
        // live frames on this live connection.
        unsafe {
            let hints = (self.x11.XAllocSizeHints)();
            if hints.is_null() {
                return; // no hints this time; the window manager is free to place it
            }
            ptr::write(hints, std::mem::zeroed());
            let mut flags = 0;
            if let Some([left, top]) = state.placed {
                flags |= xlib::USPosition;
                (*hints).x = left;
                (*hints).y = top;
            }
            if !state.resizable {
                flags |= xlib::PMinSize | xlib::PMaxSize;
                (*hints).min_width = state.inner[0] as c_int;
                (*hints).min_height = state.inner[1] as c_int;
                (*hints).max_width = state.inner[0] as c_int;
                (*hints).max_height = state.inner[1] as c_int;
            }
            (*hints).flags = flags;
            (self.x11.XSetWMNormalHints)(self.display(), state.window, hints);
            (self.x11.XFree)(hints.cast());
        }
    }
}

/// Everything the loop remembers about one frame it owns.
struct FrameState {
    window: c_ulong,
    inner: [u32; 2],
    outer: Option<[i32; 2]>,
    mapped: bool,
    resizable: bool,
    placed: Option<[i32; 2]>,
}

/// The loop's frames, by both names they are known under: the rack's node
/// and the X window.
struct Loop {
    connection: Arc<Connection>,
    windows: BTreeMap<NodeId, FrameState>,
    ids: HashMap<c_ulong, NodeId>,
    events: Sender<WorkerEvent>,
}

// SAFETY: as for `Connection` — this type only calls through the
// thread-serialized Xlib connection and moves plain numbers and the events
// channel, and it runs on the pump thread alone.
#[allow(unsafe_code)]
impl Loop {
    /// Drive the loop until told to quit. The rhythm is: take everything X
    /// has already delivered, take everything the rack queued, then sleep in
    /// `XNextEvent` until one or the other has something again — the rack's
    /// wake ring guarantees the second half happens.
    fn pump(&mut self, commands: &Receiver<Command>) {
        let connection = Arc::clone(&self.connection);
        let x = &connection.x11;
        let display = connection.display();
        'pump: loop {
            // SAFETY (the blocks below through the blocking `XNextEvent`):
            // `display` is the connection this loop opened and never
            // closes, and `event` is a live local `XEvent` — the pairing
            // Xlib documents for `XPending` and `XNextEvent`.
            while unsafe { (x.XPending)(display) > 0 } {
                let mut event: XEvent = unsafe { std::mem::zeroed() };
                unsafe { (x.XNextEvent)(display, &mut event) };
                self.handle_event(&event);
            }
            loop {
                match commands.try_recv() {
                    Ok(Command::Quit) => break 'pump,
                    Ok(command) => self.handle_command(command),
                    Err(TryRecvError::Empty) => break,
                    // The rack let go of the loop without saying `Quit` —
                    // there is nobody left to make frames for.
                    Err(TryRecvError::Disconnected) => break 'pump,
                }
            }
            let mut event: XEvent = unsafe { std::mem::zeroed() };
            unsafe { (x.XNextEvent)(display, &mut event) };
            self.handle_event(&event);
        }
        self.take_down();
    }

    fn handle_event(&mut self, event: &XEvent) {
        match event.get_type() {
            xlib::ConfigureNotify => {
                // SAFETY: the union's discriminant was just read as
                // `ConfigureNotify`, so `configure` is its active arm.
                let configure = unsafe { event.configure };
                let Some(&id) = self.ids.get(&configure.window) else {
                    return;
                };
                let Some(state) = self.windows.get_mut(&id) else {
                    return;
                };
                let connection = self.connection.as_ref();
                let size = [
                    configure.width.max(0) as u32,
                    configure.height.max(0) as u32,
                ];
                if size[0] > 0 && size[1] > 0 && size != state.inner {
                    state.inner = size;
                    let _ = self.events.send(WorkerEvent::Resized {
                        id,
                        size,
                        scale: 1.0,
                    });
                }
                // The window manager may have reparented the frame, so the
                // frame's own x/y lie about where it sits. Ask the root.
                let (mut left, mut top) = (0, 0);
                let mut child: c_ulong = 0;
                // SAFETY: the source window is one this loop made and has
                // not destroyed, the destination is the connection's own
                // root, and the three out-params are live locals.
                let translated = unsafe {
                    (connection.x11.XTranslateCoordinates)(
                        connection.display(),
                        state.window,
                        connection.root,
                        0,
                        0,
                        &mut left,
                        &mut top,
                        &mut child,
                    )
                };
                if translated != 0 {
                    let position = [left, top];
                    if Some(position) != state.outer {
                        state.outer = Some(position);
                        let _ = self.events.send(WorkerEvent::Moved {
                            id,
                            position,
                            scale: 1.0,
                        });
                    }
                }
            }
            xlib::ClientMessage => {
                // SAFETY: as above — the discriminant reads `ClientMessage`,
                // so `client_message` is the union's active arm.
                let message = unsafe { event.client_message };
                if message.window == self.connection.wake {
                    return; // the rack's ring; the command rode a channel
                }
                // The window manager does not name `WM_DELETE_WINDOW` as
                // the message type; it sends a `WM_PROTOCOLS` message and
                // names the protocol inside the data, as the ICCCM asks.
                // Comparing the message type against the delete atom — as
                // an early cut of this pump did — matches nothing, and
                // every close button quietly stops working.
                if message.message_type == self.connection.wm_protocols
                    && message.format == 32
                    && message.data.get_long(0) as c_ulong == self.connection.delete_atom
                    && let Some(&id) = self.ids.get(&message.window)
                {
                    let _ = self.events.send(WorkerEvent::CloseRequested { id });
                }
            }
            _ => {}
        }
    }

    fn handle_command(&mut self, command: Command) {
        match command {
            Command::Open {
                id,
                reply,
                title,
                position,
            } => {
                // Exactly one answer per request: the rack waits on this
                // channel and a missing reply is an editor that never appears.
                let _ = reply.send(self.open_frame(id, &title, position));
            }
            Command::Visible { id, visible } => {
                if let Some(state) = self.windows.get_mut(&id) {
                    let connection = self.connection.as_ref();
                    // SAFETY (this arm and the ones below): the window id
                    // comes out of this loop's own live frame map, naming a
                    // window it created on its own connection and has not
                    // destroyed; map, unmap, raise, set-focus, resize and
                    // destroy on one's own window over a thread-serialized
                    // connection is Xlib's documented use.
                    if visible {
                        unsafe { (connection.x11.XMapWindow)(connection.display(), state.window) };
                    } else {
                        unsafe {
                            (connection.x11.XUnmapWindow)(connection.display(), state.window)
                        };
                    }
                    state.mapped = visible;
                }
            }
            Command::Focus { id } => {
                if let Some(state) = self.windows.get(&id) {
                    let connection = self.connection.as_ref();
                    // SAFETY: own live window on the loop's own connection,
                    // as documented at the `Visible` arm.
                    unsafe {
                        (connection.x11.XRaiseWindow)(connection.display(), state.window);
                        // `RevertToParent`: if the frame dies while the
                        // focus transfer is in flight, focus returns to the
                        // rack's window rather than to the root's nowhere.
                        (connection.x11.XSetInputFocus)(
                            connection.display(),
                            state.window,
                            xlib::RevertToParent,
                            xlib::CurrentTime,
                        );
                    }
                }
            }
            Command::Resize { id, size } => {
                if let Some(state) = self.windows.get_mut(&id) {
                    let connection = self.connection.as_ref();
                    // SAFETY: own live window on the loop's own connection,
                    // as documented at the `Visible` arm.
                    unsafe {
                        (connection.x11.XResizeWindow)(
                            connection.display(),
                            state.window,
                            size[0],
                            size[1],
                        )
                    };
                    // An unmapped window hears nothing from X — the first
                    // `ConfigureNotify` comes with its first map — but the
                    // session needs the plugin's size while the frame is
                    // still hidden, so report the resize the loop itself made.
                    if !state.mapped && size[0] > 0 && size[1] > 0 && size != state.inner {
                        state.inner = size;
                        let _ = self.events.send(WorkerEvent::Resized {
                            id,
                            size,
                            scale: 1.0,
                        });
                    }
                }
            }
            Command::Resizable { id, resizable } => {
                if let Some(state) = self.windows.get_mut(&id) {
                    state.resizable = resizable;
                    self.connection.size_hints(state);
                }
            }
            Command::Close { id } => {
                // Out of both maps before the window falls: a destroyed
                // window must not stay addressable by either of its names.
                if let Some(state) = self.windows.remove(&id) {
                    self.ids.remove(&state.window);
                    let connection = self.connection.as_ref();
                    // SAFETY: own window on the loop's own connection, as
                    // documented at the `Visible` arm; it leaves both maps
                    // here, so no later call names it again.
                    unsafe { (connection.x11.XDestroyWindow)(connection.display(), state.window) };
                }
            }
            // The pump intercepts `Quit` before a command ever gets here.
            Command::Quit => {}
        }
    }

    fn open_frame(
        &mut self,
        id: NodeId,
        title: &str,
        position: Option<[i32; 2]>,
    ) -> Result<Opened, String> {
        if self.windows.contains_key(&id) {
            return Err("An editor window is already open for this plugin".to_string());
        }
        let connection = self.connection.as_ref();
        let x = &connection.x11;
        let display = connection.display();
        // Honor the saved position only where this screen can actually carry
        // it — a frame the window manager cannot place is worse than one it
        // must re-place.
        // SAFETY (through the class-hint block below): each call is Xlib's
        // documented way to create and label one top-level window. The
        // parent is this display's own root; width, height, depth and
        // visual are values from this display's own screen; the attribute
        // struct is a fully written local whose valuemask names exactly the
        // fields set on it; every string and property array outlives the
        // call that reads it (Xlib copies what it keeps); lengths are passed
        // exactly. A refusal to create shows up as the zero window checked
        // below; protocol errors go to `swallow_x_errors`, never here.
        let placed = position.filter(|[left, top]| {
            *left >= 0
                && *top >= 0
                && *left < unsafe { (x.XDisplayWidth)(display, connection.screen) } - 40
                && *top < unsafe { (x.XDisplayHeight)(display, connection.screen) } - 40
        });
        let mut attributes: XSetWindowAttributes = unsafe { std::mem::zeroed() };
        // The only events a frame needs: where and how big it ended up.
        // (Close requests ride the `ClientMessage` arm, and X sends those
        // regardless of any mask — see `Connection::notify`.)
        attributes.event_mask = xlib::StructureNotifyMask;
        attributes.background_pixel = unsafe { (x.XBlackPixel)(display, connection.screen) };
        // Built hidden, like the frost-loop path: a frame appears once the
        // plugin is in it, never as an empty box the plugin might never join.
        let window = unsafe {
            (x.XCreateWindow)(
                display,
                connection.root,
                placed.map_or(0, |[left, _]| left),
                placed.map_or(0, |[_, top]| top),
                INITIAL_SIZE[0],
                INITIAL_SIZE[1],
                0,
                (x.XDefaultDepth)(display, connection.screen),
                xlib::InputOutput as _,
                connection.visual,
                xlib::CWEventMask | xlib::CWBackPixel,
                &mut attributes,
            )
        };
        if window == 0 {
            return Err("Xlib refused to create the frame".to_string());
        }
        unsafe {
            // Name it: `WM_NAME` for classic tools, `_NET_WM_NAME` in UTF-8
            // for the window managers that read that instead.
            if let Ok(name) = CString::new(title) {
                (x.XStoreName)(display, window, name.as_ptr());
            }
            let bytes = title.as_bytes();
            (x.XChangeProperty)(
                display,
                window,
                connection.net_wm_name,
                connection.utf8_string,
                8,
                xlib::PropModeReplace,
                bytes.as_ptr(),
                bytes.len() as c_int,
            );
            // Ask the window manager to route its close button here.
            let protocols = [connection.delete_atom];
            (x.XChangeProperty)(
                display,
                window,
                connection.wm_protocols,
                connection.x_atom,
                32,
                xlib::PropModeReplace,
                protocols.as_ptr().cast(),
                protocols.len() as c_int,
            );
            // And whose frame it is.
            let pid = [std::process::id() as c_ulong];
            (x.XChangeProperty)(
                display,
                window,
                connection.net_wm_pid,
                connection.x_atom,
                32,
                xlib::PropModeReplace,
                pid.as_ptr().cast(),
                pid.len() as c_int,
            );
            if let (Ok(res_name), Ok(res_class)) =
                (CString::new("nice-rack"), CString::new("Nice Rack"))
            {
                let mut class = XClassHint {
                    res_name: res_name.as_ptr().cast_mut(),
                    res_class: res_class.as_ptr().cast_mut(),
                };
                // Xlib copies both strings; the CStrings stay owned right here.
                (x.XSetClassHint)(display, window, &mut class);
            }
        }
        let state = FrameState {
            window,
            inner: INITIAL_SIZE,
            outer: None,
            mapped: false,
            resizable: true,
            placed,
        };
        connection.size_hints(&state);
        // `XlibWindowHandle` is `#[non_exhaustive]`, so its crate's own
        // constructor builds it and only the documented visual field goes
        // on afterwards.
        let mut handle = XlibWindowHandle::new(window);
        handle.visual_id = connection.visual_id;
        let opened = Opened {
            parent: Arc::new(X11Parent(RawWindowHandle::Xlib(handle))),
            // Xlib reports no per-window scale factor, and XWayland applies
            // the HiDPI translation for its own windows on its own, so the
            // rack takes these frames' pixels at face value.
            scale_factor: 1.0,
            inner: state.inner,
            outer: None,
        };
        self.ids.insert(window, id);
        self.windows.insert(id, state);
        Ok(opened)
    }

    /// Take every frame down, and the wake window last — everything that
    /// could be rung is gone by then. The connection itself is never closed:
    /// see [`Connection`].
    fn take_down(&mut self) {
        let connection = self.connection.as_ref();
        let frames = std::mem::take(&mut self.windows);
        self.ids.clear();
        // SAFETY: every id below was created by this loop, on this same
        // connection this thread still holds open, and none has been
        // destroyed yet — frames leave the map here or on `Close`, never
        // both. The wake window goes last so no send can reach it after.
        unsafe {
            for state in frames.into_values() {
                (connection.x11.XDestroyWindow)(connection.display(), state.window);
            }
            (connection.x11.XDestroyWindow)(connection.display(), connection.wake);
        }
    }
}

/// A plugin parent that names an X11 window another thread's loop created.
///
/// The window itself stays owned by the pump, and never dies while a parent
/// is reachable: the loop destroys a frame only on `Close`, which the rack
/// sends only after the plugin has released its editor. What crosses threads
/// here is just two numbers — a window id and a visual id — and a plugin
/// reparenting through them opens its own legal Xlib conversation; the loop
/// called `XInitThreads` before opening its display, so every connection
/// through that library is serialized.
// The rack forbids `unsafe` crate-wide, and this file is the one place its
// editors cannot be built without it: `raw_window_handle` marks its handles
// `!Send` out of caution about lifetimes, and `WindowHandle::borrow_raw` is
// the only constructor there is. Every other editor path in this workspace
// crosses the same line the same way — see `nice-clap-host`'s window
// plumbing and `nice-common`'s ASIO transport.
pub(crate) struct X11Parent(RawWindowHandle);

// SAFETY: the handle carries X resource ids, not exclusive process state,
// and the X window they name outlives every borrow taken while its frame is
// in the rack's hands: the loop keeps the frame until a `Close`, which the
// rack sends only after the plugin has released its editor.
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
        // Nothing is asserted after `Quit`: the loop may or may not drain what
        // was queued around the exit, and an answer of "gone" arriving one
        // way or another is the same promise the live paths already make.
    }

    /// The message a window manager actually sends for ✕ — a `WM_PROTOCOLS`
    /// ClientMessage naming `WM_DELETE_WINDOW` inside the data — replayed
    /// over a second connection, plus a decoy naming a different protocol.
    /// An early cut of the pump matched the message *type* against the delete
    /// atom, so the real message fell through and every close button did
    /// nothing; the decoy keeps a fix that only checks the type from passing.
    #[test]
    #[ignore = "requires a live X server (XWayland counts); set NICE_RACK_TEST_X11=1"]
    #[allow(unsafe_code)] // the test plays the window manager's part on Xlib
    fn a_window_manager_close_request_becomes_a_close_requested() {
        assert!(
            std::env::var_os("NICE_RACK_TEST_X11").is_some(),
            "Set NICE_RACK_TEST_X11=1 to run against this machine's X server"
        );
        let service = Service::start().expect("the editor loop starts under a live X server");
        let id = NodeId(1);
        let opened = match service.open(id, "frost close request test", None) {
            Ok(opened) => opened,
            Err(error) => panic!("the loop refused to build a frame: {error}"),
        };
        let window = match opened.parent.window_handle().unwrap().as_raw() {
            RawWindowHandle::Xlib(handle) => handle.window as c_ulong,
            raw => panic!("the frame carries {raw:?}, not an X11 window"),
        };
        // A window manager reaches the frame over its own connection, so the
        // test does too.
        let x11 = Xlib::open().expect("libX11 loads under a live X server");
        // SAFETY: `XOpenDisplay(null)` is the documented way to open the
        // environment's display, and null is checked; each synthetic event
        // writes the `client_message` arm of the union before `XSendEvent`
        // reads it, naming the frame window on this second connection;
        // `XCloseDisplay` closes exactly the connection opened here, after
        // the requests it carries have been flushed.
        unsafe {
            let display = (x11.XOpenDisplay)(ptr::null());
            assert!(!display.is_null(), "no X server to speak as");
            let wm_protocols = (x11.XInternAtom)(display, c"WM_PROTOCOLS".as_ptr(), xlib::False);
            let delete = (x11.XInternAtom)(display, c"WM_DELETE_WINDOW".as_ptr(), xlib::False);
            let ping = (x11.XInternAtom)(display, c"_NET_WM_PING".as_ptr(), xlib::False);
            let send = |protocol: c_ulong| {
                let mut message: XEvent = std::mem::zeroed();
                message.client_message = XClientMessageEvent {
                    type_: xlib::ClientMessage,
                    serial: 0,
                    send_event: xlib::True,
                    display,
                    window,
                    message_type: wm_protocols,
                    format: 32,
                    data: {
                        let mut data = xlib::ClientMessageData::new();
                        data.set_long(0, protocol as std::ffi::c_long);
                        data
                    },
                };
                (x11.XSendEvent)(display, window, xlib::False, 0, &mut message);
                (x11.XFlush)(display);
            };
            // The ICCCM does not let a client read its own close request as a
            // license to stop answering other protocols, and neither does
            // this pump: a protocol it was never told to honour must not
            // look like one it was.
            send(ping);
            std::thread::sleep(Duration::from_millis(200));
            assert!(
                !service
                    .drain_events()
                    .into_iter()
                    .any(|event| matches!(event, WorkerEvent::CloseRequested { .. })),
                "a protocol nobody asked to close for became a close request"
            );
            send(delete);
            (x11.XCloseDisplay)(display);
        }
        let deadline = Instant::now() + WAIT;
        let mut saw_close = false;
        while Instant::now() < deadline && !saw_close {
            for event in service.drain_events() {
                if let WorkerEvent::CloseRequested { id: event_id } = event {
                    assert_eq!(event_id, id);
                    saw_close = true;
                }
            }
            if !saw_close {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        assert!(
            saw_close,
            "a WM_DELETE_WINDOW message never became a CloseRequested"
        );
        service.close(id);
        service.quit();
    }
}
