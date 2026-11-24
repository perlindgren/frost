//! An XDND receiver that listens with X RECORD, so drops land under KDE.
//!
//! Under KDE's Wayland session the XWayland drag-and-drop bridge speaks
//! XDND to X11 windows even when the drag source (Dolphin) is Wayland
//! native — but the `XdndPosition` messages are addressed to the window
//! Xwayland's pointer tracking reports under the cursor, the window
//! manager's frame, never baseview's plugin child. Baseview listens only on
//! its own child, so nobody answers the handshake and the drag cursor
//! turns into a 🚫. Selecting ClientMessage on the child ourselves changes
//! nothing: the messages simply never arrive there.
//!
//! So instead of guessing which window the messages will hit, this module
//! listens to the protocol itself: an X RECORD context captures every
//! ClientMessage delivered anywhere on the server, and when one is
//! addressed to our child window or its frame, we run the XDND responder
//! ourselves on our own connections — answering `XdndStatus` and
//! `XdndFinished` to the source, and converting `XdndSelection` with
//! `CurrentTime`, which no timestamp game can invalidate.
//!
//! This coexists with baseview's own half-answer: whatever answers first
//! carries the drag, and `PluginWindow::deliver_files` absorbs a duplicate.
//!
//! # Safety
//!
//! The whole module is Xlib/RECORD FFI, hence the crate-wide `unsafe_code`
//! deny is relaxed here. One rule keeps it honest: each Xlib connection is
//! touched by exactly one thread — the one this module spawns — and no
//! connection ever crosses back to the window thread. Only the channel of
//! finished paths does, and channels are thread-safe by design.
#![allow(unsafe_code)]

use std::ffi::{CString, c_char, c_int, c_long, c_uchar, c_ulong, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::ptr;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver as MpScReceiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use x11_dl::xlib::{self, Atom, Display, XErrorEvent, XEvent, Xlib};
use x11_dl::xrecord::{XRecordInterceptData, Xf86vmode as Xrecord};

/// libXtst's RECORD entry points, loaded once. The struct is named
/// `Xf86vmode` in x11-dl for historical reasons; it is the RECORD linker.
static XTST: OnceLock<Option<Xrecord>> = OnceLock::new();
static X11: OnceLock<Option<Xlib>> = OnceLock::new();

fn xlib() -> Option<&'static Xlib> {
    X11.get_or_init(|| Xlib::open().ok()).as_ref()
}

fn record() -> Option<&'static Xrecord> {
    XTST.get_or_init(|| Xrecord::open().ok()).as_ref()
}

/// X.h `SelectionNotifyMask`; x11-dl omits the selection masks.
const SELECTION_NOTIFY_MASK: c_long = 1 << 19;

/// X.h `XRecordAllClients` — the client-spec meaning "every client".
const ALL_CLIENTS: c_ulong = 3;

/// Handles this receiver needs. Every name is XDND's own registry spelling
/// or the property's literal type name, exactly as baseview and every other
/// implementation interns them.
struct Atoms {
    enter: Atom,
    position: Atom,
    drop: Atom,
    leave: Atom,
    finished: Atom,
    status: Atom,
    action_copy: Atom,
    selection: Atom,
    type_list: Atom,
    text_uri_list: Atom,
    x_atom: Atom,
}

impl Atoms {
    fn intern(x: &Xlib, display: *mut Display) -> Self {
        // SAFETY: `display` is the connection this module opened and owns;
        // interning never blocks, and each string is a NUL-terminated
        // literal whose backing bytes outlive the call.
        unsafe {
            let atom = |name: &str| {
                let cname = CString::new(name).expect("atom names contain no NUL");
                (x.XInternAtom)(display, cname.as_ptr(), xlib::False)
            };
            Self {
                enter: atom("XdndEnter"),
                position: atom("XdndPosition"),
                drop: atom("XdndDrop"),
                leave: atom("XdndLeave"),
                finished: atom("XdndFinished"),
                status: atom("XdndStatus"),
                action_copy: atom("XdndActionCopy"),
                selection: atom("XdndSelection"),
                type_list: atom("XdndTypeList"),
                text_uri_list: atom("text/uri-list"),
                x_atom: atom("ATOM"),
            }
        }
    }
}

/// The one drag in flight. XDND is strictly sequential: an Enter opens a
/// drag and a Leave, Finished, or new Enter closes it, so one record is all
/// the state there is.
#[derive(Default)]
struct Drag {
    /// The source's window, 0 outside a drag. Every reply goes there.
    source: c_ulong,
    /// The type we will accept for this drag, 0 meaning refuse it.
    accepted: Atom,
}

/// Everything the RECORD callback needs, living on the record thread. The
/// callback receives it as the closure pointer; it is only ever
/// dereferenced from the thread that boxed it, and freed after the last
/// possible callback.
struct State {
    /// Reply connection: property reads, selection conversion, Status and
    /// Finished. Never touched by the window thread, never by the RECORD
    /// connection's own machinery.
    pdpy: *mut Display,
    /// Our unmapped requestor window on `pdpy`.
    our_window: c_ulong,
    /// The baseview child's X11 id...
    child: c_ulong,
    /// ...and its two ancestors: the parent Xwayland window and the
    /// window-manager frame that wraps it. Xwayland's drag bridge addresses
    /// pointer-targeted XDND messages to whichever of these its pointer
    /// tracking reports, so a message to any of the three is addressed to
    /// us. Each is `child` if that hop does not exist.
    frame: [c_ulong; 2],
    atoms: Atoms,
    drag: Drag,
    tx: Sender<Vec<PathBuf>>,
}

/// Owns the receiver thread; stopping is dropping.
struct Receiver {
    stop: Arc<AtomicBool>,
    joiner: Option<JoinHandle<()>>,
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(joiner) = self.joiner.take() {
            // The loop re-checks `stop` every few milliseconds, so this
            // returns promptly. A selection conversion in flight adds up to
            // its five-second wait; a wedged X server would hang anything.
            let _ = joiner.join();
        }
    }
}

/// The window side of a live receiver: the paths thread it answers with,
/// and the handle that keeps the thread alive.
pub(crate) struct Session {
    pub(crate) paths: MpScReceiver<Vec<PathBuf>>,
    /// Read only by its own `Drop`, which is the whole point: dropping the
    /// session stops the thread.
    _receiver: Receiver,
}

/// Start intercepting XDND traffic for `child_window` (the baseview
/// child's X11 id). Errors are unrecoverable-by-retry — no X server, no
/// RECORD extension, no thread — so the caller treats one as "keep
/// baseview's answer as the only one".
pub(crate) fn spawn(child_window: u64) -> Result<Session, String> {
    let (tx, paths) = std::sync::mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let joiner = thread::Builder::new()
        .name("nice-xdnd".to_string())
        .spawn(move || run(child_window as c_ulong, thread_stop, tx))
        .map_err(|error| error.to_string())?;
    Ok(Session {
        paths,
        _receiver: Receiver {
            stop,
            joiner: Some(joiner),
        },
    })
}

fn run(child: c_ulong, stop: Arc<AtomicBool>, tx: Sender<Vec<PathBuf>>) {
    let (Some(x), Some(xt)) = (xlib(), record()) else {
        return;
    };
    // SAFETY: documented Xlib initialization. The return value is ignored
    // on purpose: baseview opened its display first and did not call this,
    // so 0 ("already initialized") is the normal answer here. What matters
    // is that each of *our* connections is touched by one thread only —
    // this one — which is how Xlib is safely usable regardless.
    unsafe { (x.XInitThreads)() };
    // SAFETY: installing a process-global handler is Xlib's documented way
    // to survive a window dying mid-drag; ours swallows, because every
    // Xlib error this thread can cause is a drag racing its own window
    // teardown. The default handler would exit the process.
    unsafe { (x.XSetErrorHandler)(Some(swallow_x_errors)) };

    // Two connections, both owned by this thread. The RECORD connection is
    // reserved for pumping context data; the reply connection answers the
    // protocol. Keeping them apart means a five-second wait for selection
    // data inside a callback never stalls the capture of what comes next.
    // SAFETY: a NULL display name is Xlib's "use $DISPLAY".
    let pdpy = unsafe { (x.XOpenDisplay)(ptr::null()) };
    if pdpy.is_null() {
        return;
    }
    // SAFETY: as above.
    let rdpy = unsafe { (x.XOpenDisplay)(ptr::null()) };
    if rdpy.is_null() {
        // SAFETY: `pdpy` is this thread's live connection.
        unsafe { (x.XCloseDisplay)(pdpy) };
        return;
    }

    let close_both = |x: &Xlib| {
        // SAFETY: both connections are live and owned by this thread.
        unsafe {
            (x.XCloseDisplay)(rdpy);
            (x.XCloseDisplay)(pdpy);
        }
    };

    let Some(reply_type) = record_reply_type(x, rdpy) else {
        close_both(x);
        return;
    };
    let _ = reply_type; // pumping goes through XRecordProcessReplies instead
    let our_window = requestor_window(x, pdpy);
    let atoms = Atoms::intern(x, pdpy);
    let frame = resolve_frame(x, pdpy, child);
    let Some(context) = create_context(x, xt, rdpy) else {
        close_both(x);
        return;
    };

    let state = Box::into_raw(Box::new(State {
        pdpy,
        our_window,
        child,
        frame,
        atoms,
        drag: Drag::default(),
        tx,
    }));
    // SAFETY: live connections; `state` is a live Box whose ownership is
    // conceptually lent to the callback and reclaimed below, after the
    // context is disabled, so no callback can outlive it.
    unsafe {
        (xt.XRecordEnableContextAsync)(rdpy, context, Some(record_cb), state.cast::<c_char>());
        (x.XSync)(rdpy, xlib::False);
    }
    // Pump replies until asked to stop. XRecordProcessReplies is libX11's
    // own drain for async context data: it delivers every queued batch to
    // the callback and frees the event data. 4 ms is imperceptible to a
    // human drag.
    while !stop.load(Ordering::Relaxed) {
        // SAFETY: live connection; this only drains what is already queued.
        unsafe { (xt.XRecordProcessReplies)(rdpy) };
        std::thread::sleep(Duration::from_millis(4));
    }
    // SAFETY: the loop has ended, so this thread will not pump any more
    // data; disabling first guarantees no callback fires after the state
    // box dies. Closing each connection is allowed because each was used
    // by this thread only.
    unsafe {
        (xt.XRecordDisableContext)(rdpy, context);
        (xt.XRecordFreeContext)(rdpy, context);
        (x.XSync)(rdpy, xlib::False);
        (x.XCloseDisplay)(rdpy);
        (x.XCloseDisplay)(pdpy);
        drop(Box::from_raw(state));
    }
}

/// Xlib error sink — see [`run`]'s installation comment.
#[allow(unsafe_code)]
unsafe extern "C" fn swallow_x_errors(_display: *mut Display, _error: *mut XErrorEvent) -> c_int {
    0
}

/// Ask where the RECORD extension puts its events, so we know the context
/// exists at all. The reply is the extension's first event type,
/// `event_base + XRecord_Reply(0)`.
fn record_reply_type(x: &Xlib, display: *mut Display) -> Option<c_int> {
    let name = CString::new("RECORD").expect("no NUL in extension name");
    let (mut major, mut event_base, mut error_base) = (0, 0, 0);
    // SAFETY: live connection and four live out-params; the name is a
    // stack CString whose bytes outlive the call.
    let ok = unsafe {
        (x.XQueryExtension)(
            display,
            name.as_ptr(),
            &mut major,
            &mut event_base,
            &mut error_base,
        )
    };
    (ok != 0).then_some(event_base)
}

/// Create our unmapped requestor window, the one `XConvertSelection` names
/// and the property lands on. Named so an `xwininfo` tour finds it.
fn requestor_window(x: &Xlib, display: *mut Display) -> c_ulong {
    // SAFETY: `display` is this thread's live connection; the root is its
    // own, and creating an unmapped 1x1 window cannot fail for any reason
    // this thread handles differently.
    unsafe {
        let root = (x.XDefaultRootWindow)(display);
        let window = (x.XCreateSimpleWindow)(display, root, 0, 0, 1, 1, 0, 0, 0);
        let name = CString::new("frost-xdnd-target").expect("name has no NUL");
        (x.XStoreName)(display, window, name.as_ptr());
        (x.XSelectInput)(display, window, SELECTION_NOTIFY_MASK);
        (x.XSync)(display, xlib::False);
        window
    }
}

/// Find `child`'s ancestors — the parent Xwayland window and the
/// window-manager frame above it — retrying briefly because window
/// managers reparent when the window is first mapped and that may still be
/// in flight when the editor opens. Missing hops collapse to `child`.
fn resolve_frame(x: &Xlib, display: *mut Display, child: c_ulong) -> [c_ulong; 2] {
    let mut frame = [child, child];
    for _ in 0..60 {
        let parent = query_parent(x, display, child);
        let grand = parent.and_then(|p| query_parent(x, display, p));
        frame = [
            parent.unwrap_or(child),
            grand.unwrap_or(parent.unwrap_or(child)),
        ];
        if frame[0] != child && frame[1] != frame[0] {
            return frame;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    frame
}

/// The direct parent of `window`, or `None` while the tree is not yet
/// established (unmapped, or a WM reparent still in flight).
fn query_parent(x: &Xlib, display: *mut Display, window: c_ulong) -> Option<c_ulong> {
    let (mut root, mut parent) = (0, 0);
    let (mut children, mut n) = (ptr::null_mut(), 0);
    // SAFETY: live connection and four live out-params; the error
    // handler covers a window that stopped existing between attempts.
    let ok = unsafe {
        (x.XQueryTree)(
            display,
            window,
            &mut root,
            &mut parent,
            &mut children,
            &mut n,
        )
    };
    if !children.is_null() {
        // SAFETY: XQueryTree allocated this child list on success;
        // freeing it is its documented contract.
        unsafe { (x.XFree)(children.cast::<c_void>()) };
    }
    (ok != 0 && parent != 0 && parent != root && parent != window).then_some(parent)
}

/// Create the RECORD context that captures delivered ClientMessages. The
/// datum is packed 32-bit units (datum size 0), i.e. the wire events.
fn create_context(x: &Xlib, xt: &Xrecord, display: *mut Display) -> Option<c_ulong> {
    // SAFETY: live connection. XRecordAllocRange returns a zeroed range —
    // nothing enabled — that we own and must free.
    unsafe {
        let range = (xt.XRecordAllocRange)();
        if range.is_null() {
            return None;
        }
        // SAFETY: the range is a live allocation per the call above; this
        // is the one field worth enabling: delivered events of type
        // ClientMessage (33), the only way XDND messages travel.
        (*range).delivered_events.first = xlib::ClientMessage as c_uchar;
        (*range).delivered_events.last = xlib::ClientMessage as c_uchar;
        let mut clients = ALL_CLIENTS;
        let mut ranges = [range];
        // SAFETY: live connection; `clients` and `ranges` are live locals
        // that outlive the call, which only copies them into a request.
        let context =
            (xt.XRecordCreateContext)(display, 0, &mut clients, 1, ranges.as_mut_ptr(), 1);
        (x.XSync)(display, xlib::False);
        // SAFETY: XRecordAllocRange allocated this range and the server
        // has consumed it into the request above.
        (x.XFree)(range.cast::<c_void>());
        (context != 0).then_some(context)
    }
}

/// The RECORD callback, fired while libX11 drains the context on the
/// record thread. `closure` is our [`State`] box; `intercept` is a batch
/// of packed 32-bit wire events owned by us once it is delivered.
unsafe extern "C" fn record_cb(closure: *mut c_char, intercept: *mut XRecordInterceptData) {
    let Some(xt) = record() else { return };
    if intercept.is_null() {
        return;
    }
    // SAFETY: libX11 hands a live batch here on the record thread; the
    // closure pointer is the Box [`run`] created, or null.
    unsafe {
        // Only data from the server itself (category FromServer = 0);
        // client-capture categories are off, but stay defensive.
        if (*intercept).category != 0 || (*intercept).data.is_null() {
            (xt.XRecordFreeData)(intercept);
            return;
        }
        let Some(state) = (closure as *mut State).as_mut() else {
            (xt.XRecordFreeData)(intercept);
            return;
        };
        let Some(x) = xlib() else {
            (xt.XRecordFreeData)(intercept);
            return;
        };
        // datum_len counts 4-byte units; each wire event is exactly 32.
        let len = ((*intercept).data_len as usize) * 4;
        let bytes = std::slice::from_raw_parts((*intercept).data, len);
        for event in bytes.as_chunks::<32>().0 {
            intercept_event(x, state, event);
        }
        (xt.XRecordFreeData)(intercept);
    }
}

/// One intercepted wire event. A delivered ClientMessage on the wire packs
/// as `[type|flags, window, message_type, data0..data4]` — the offsets
/// this module verified against live kwin traffic.
fn intercept_event(x: &Xlib, state: &mut State, event: &[u8]) {
    // The send_event flag rides in bit 7; XDND messages are synthetic.
    if event[0] & 0x7f != xlib::ClientMessage as u8 {
        return;
    }
    let word = |index: usize| {
        u32::from_ne_bytes(event[index * 4..index * 4 + 4].try_into().unwrap()) as c_ulong
    };
    let (window, message_type) = (word(1), word(2));
    // Our child, the parent Xwayland window, or the WM frame above it.
    // Everything else on the session — including our own Status and
    // Finished leaving for the source — is not ours to answer.
    if window != state.child && !state.frame.contains(&window) {
        return;
    }
    let atoms = &state.atoms;
    if message_type == atoms.enter {
        let (source, flags) = (word(3), word(4));
        on_enter(x, state, source, flags, word(5), word(6), word(7));
    } else if message_type == atoms.position {
        let source = word(3);
        if state.drag.source == 0 {
            // Enter may have gone to a window we do not match; a Position
            // addressed to a window we do match still deserves an answer.
            state.drag.source = source;
            state.drag.accepted = atoms.text_uri_list;
        }
        send_status(x, state.pdpy, source, atoms, state.drag.accepted != 0);
    } else if message_type == atoms.drop {
        on_drop(x, state);
    } else if message_type == atoms.leave {
        state.drag = Drag::default();
    }
}

/// Open a drag: decide whether text/uri-list is on offer. A failed type
/// list read accepts optimistically — refusing a real file drag is the
/// worse failure by far.
fn on_enter(
    x: &Xlib,
    state: &mut State,
    source: c_ulong,
    flags: c_ulong,
    type2: c_ulong,
    type3: c_ulong,
    type4: c_ulong,
) {
    let uri = state.atoms.text_uri_list;
    // Per XDND, l[1] packs the version in bits 24..31 and the more-types
    // flag in bit 0; the type list lives in XdndTypeList on the source
    // window only when that low bit is set.
    let accepted = if flags & 1 != 0 {
        // SAFETY: live connection; the property read tolerates a vanished
        // window through the error handler.
        match unsafe {
            read_type_list(
                x,
                state.pdpy,
                source,
                state.atoms.type_list,
                state.atoms.x_atom,
            )
        } {
            Some(types) => types.contains(&uri),
            None => true,
        }
    } else {
        type2 == uri || type3 == uri || type4 == uri
    };
    state.drag = Drag {
        source,
        accepted: if accepted { uri } else { 0 },
    };
}

/// Answer a drop: convert `XdndSelection` on our reply connection and wait
/// for the owner's data. Blocks inside the RECORD callback for at most the
/// five-second poll below; the RECORD connection being a separate one
/// means the capture itself never stalls beyond that.
fn on_drop(x: &Xlib, state: &mut State) {
    let atoms = &state.atoms;
    if state.drag.accepted == 0 {
        finish(x, state, false);
        return;
    }
    // SAFETY: live connection and our own window. `CurrentTime` (0) is the
    // whole trick: the server substitutes a time the owner can only
    // accept, so no timestamp packed wrongly by anyone can stall us. The
    // delete clears any stale property from an earlier drag.
    let ok = unsafe {
        (x.XDeleteProperty)(state.pdpy, state.our_window, atoms.selection);
        let ok = (x.XConvertSelection)(
            state.pdpy,
            atoms.selection,
            atoms.text_uri_list,
            atoms.selection,
            state.our_window,
            0,
        );
        (x.XFlush)(state.pdpy);
        ok
    };
    if ok == 0 {
        finish(x, state, false);
        return;
    }
    for _ in 0..500 {
        // SAFETY: zeroed union; XCheckTypedWindowEvent fills the arm it
        // reports. Live connection and our own window.
        let mut event: XEvent = unsafe { std::mem::zeroed() };
        let got = unsafe {
            (x.XCheckTypedWindowEvent)(
                state.pdpy,
                state.our_window,
                xlib::SelectionNotify,
                &mut event,
            )
        };
        if got != 0 {
            // SAFETY: the type check above says the selection arm is what
            // Xlib wrote.
            let notify = unsafe { event.selection };
            if notify.selection != atoms.selection {
                continue;
            }
            if notify.property == 0 {
                finish(x, state, false);
                return;
            }
            // SAFETY: live connection and our own window; the property the
            // notify names is one this connection requested.
            let bytes = unsafe { read_property(x, state.pdpy, state.our_window, atoms.selection) };
            let paths = parse_uri_list(bytes.as_deref().unwrap_or_default());
            if paths.is_empty() {
                finish(x, state, false);
            } else {
                // The window thread may already be gone; losing a drop to
                // a closed editor beats panicking across the boundary.
                let _ = state.tx.send(paths);
                finish(x, state, true);
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    finish(x, state, false);
}

/// Close the drag with XdndFinished (the source's protocol ends here) and
/// forget it.
fn finish(x: &Xlib, state: &mut State, accepted: bool) {
    let source = state.drag.source;
    if source != 0 {
        send_message(
            x,
            state.pdpy,
            source,
            state.atoms.finished,
            [
                source as c_long,
                c_long::from(accepted),
                (if accepted { state.atoms.action_copy } else { 0 }) as c_long,
                0,
                0,
            ],
        );
    }
    state.drag = Drag::default();
}

fn send_status(x: &Xlib, display: *mut Display, source: c_ulong, atoms: &Atoms, accepted: bool) {
    send_message(
        x,
        display,
        source,
        atoms.status,
        [
            source as c_long,
            c_long::from(accepted),
            // An empty target rectangle: we accept everywhere in the
            // plugin window, and XDND spells that as no rectangle at all.
            0,
            0,
            (if accepted { atoms.action_copy } else { 0 }) as c_long,
        ],
    );
}

/// Send one XDND ClientMessage. Every XDND implementation — the KDE bridge
/// included — sends these with event mask 0 and `send_event` set, and every
/// implementation accepts them the same way, so this mirrors that exactly.
fn send_message(
    x: &Xlib,
    display: *mut Display,
    destination: c_ulong,
    message_type: Atom,
    data: [c_long; 5],
) {
    // SAFETY: a synthetic event is Xlib's documented use of the `XEvent`
    // union: the `client_message` arm is fully written before the event is
    // passed, `XSendEvent` only reads it, and mask 0 with `send_event` set
    // is how XDND itself addresses every one of these messages.
    unsafe {
        let mut event: XEvent = std::mem::zeroed();
        let mut client_message = xlib::XClientMessageEvent {
            type_: xlib::ClientMessage,
            serial: 0,
            send_event: xlib::True,
            display,
            window: destination,
            message_type,
            format: 32,
            data: std::mem::zeroed(),
        };
        for (index, value) in data.iter().enumerate() {
            client_message.data.set_long(index, *value);
        }
        event.client_message = client_message;
        (x.XSendEvent)(display, destination, xlib::False, 0, &mut event);
        (x.XFlush)(display);
    }
}

/// Read a whole window property as bytes, deleting it as the last page is
/// taken (XDND targets are expected to clean up their drop property).
/// Returns `None` if any page failed.
///
/// # Safety
///
/// `display` is a live connection, `window` valid on it, and `property` a
/// property name; a stale id or missing property yields `None` through the
/// error handler, never a crash.
unsafe fn read_property(
    x: &Xlib,
    display: *mut Display,
    window: c_ulong,
    property: Atom,
) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut offset: c_long = 0;
    loop {
        let (mut actual_type, mut actual_format) = (0, 0);
        let (mut nitems, mut bytes_after) = (0, 0);
        let mut data: *mut c_uchar = ptr::null_mut();
        // SAFETY: per the function contract; the five out-params are live
        // locals, and `delete: True` removes the property once its last
        // page has been read into `out` below.
        let ok = unsafe {
            (x.XGetWindowProperty)(
                display,
                window,
                property,
                offset,
                4096,
                xlib::True,
                xlib::AnyPropertyType as c_ulong,
                &mut actual_type,
                &mut actual_format,
                &mut nitems,
                &mut bytes_after,
                &mut data,
            )
        };
        if ok != 0 {
            // `Success` is 0; any other status is a vanished window or a
            // property that stopped existing mid-drag.
            return None;
        }
        if !data.is_null() {
            // Format is bits per item; property text is format 8, the ATOM
            // list is 32 (stored four bytes per atom on the wire anyway).
            let width = match actual_format {
                32 => 4,
                16 => 2,
                _ => 1,
            };
            // SAFETY: XGetWindowProperty allocated exactly
            // `nitems * width` bytes for this format; copying that range
            // and handing the pointer to XFree is its documented contract.
            unsafe {
                let bytes = std::slice::from_raw_parts(data, (nitems as usize) * width);
                out.extend_from_slice(bytes);
                (x.XFree)(data.cast::<c_void>());
            }
        }
        offset += nitems as c_long;
        if bytes_after == 0 {
            return Some(out);
        }
    }
}

/// Read the source's `XdndTypeList` property (an ATOM array) as atom
/// values, `None` if the property could not be read at all. The property
/// belongs to the source's window, so nothing is deleted.
///
/// # Safety
///
/// `display` is a live connection and `window` a window valid on it; a
/// stale id or missing property yields `None` through the error handler,
/// never a crash.
unsafe fn read_type_list(
    x: &Xlib,
    display: *mut Display,
    window: c_ulong,
    property: Atom,
    expected_type: Atom,
) -> Option<Vec<Atom>> {
    let (mut actual_type, mut actual_format) = (0, 0);
    let (mut nitems, mut bytes_after) = (0, 0);
    let mut data: *mut c_uchar = ptr::null_mut();
    // SAFETY: per the function contract; the five out-params are live
    // locals. 64 longs covers any real XdndTypeList (drag sources advertise
    // a handful of types); a longer list still answers this question
    // correctly for the prefix, and `delete: False` leaves the source's
    // property alone.
    let ok = unsafe {
        (x.XGetWindowProperty)(
            display,
            window,
            property,
            0,
            64,
            xlib::False,
            expected_type as c_ulong,
            &mut actual_type,
            &mut actual_format,
            &mut nitems,
            &mut bytes_after,
            &mut data,
        )
    };
    if ok != 0 {
        // `Success` is 0; on failure there is no buffer to free.
        return None;
    }
    // SAFETY: format 32 means `nitems` 4-byte items, one per atom, in the
    // buffer XGetWindowProperty just allocated; copying them out and
    // handing the pointer to XFree is that function's documented contract.
    let atoms = unsafe {
        let atoms = if actual_format == 32 {
            std::slice::from_raw_parts(data.cast::<u32>(), nitems as usize)
                .iter()
                .map(|value| *value as Atom)
                .collect()
        } else {
            Vec::new()
        };
        if !data.is_null() {
            (x.XFree)(data.cast::<c_void>());
        }
        atoms
    };
    let _ = bytes_after;
    Some(atoms)
}

/// Parse an XDND `text/uri-list` body: `\r\n`-separated lines, `#`-comments,
/// percent-escapes, only `file:` URIs load. Mirrors what baseview's parser
/// hands the plugins, so the two paths feed `files_dropped` identically.
fn parse_uri_list(bytes: &[u8]) -> Vec<PathBuf> {
    let text = String::from_utf8_lossy(bytes);
    let mut out = Vec::new();
    for line in text.split(['\r', '\n']) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.strip_prefix("file://") else {
            continue;
        };
        // `file://localhost/…` is the same local path per RFC 8089.
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        let path = percent_decode(rest.as_bytes());
        if path.is_empty() {
            continue;
        }
        let candidate = PathBuf::from(std::ffi::OsStr::from_bytes(&path));
        out.push(candidate.canonicalize().unwrap_or(candidate));
    }
    out
}

fn percent_decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        // Only consume the two bytes after '%' once both are hex digits;
        // a literal '%' keeps its following bytes intact.
        if input[i] == b'%'
            && i + 2 < input.len()
            && let (Some(h), Some(l)) = (hex_value(input[i + 1]), hex_value(input[i + 2]))
        {
            out.push(h << 4 | l);
            i += 3;
            continue;
        }
        out.push(input[i]);
        i += 1;
    }
    out
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_escaped_uris() {
        let body = b"file:///home/u/tone%20drive.nam\r\nfile:///home/u/IR%20L.wav\r\n#comment\r\n";
        let paths = parse_uri_list(body);
        assert_eq!(paths.len(), 2);
        assert_eq!(
            paths[0].file_name().unwrap().to_str().unwrap(),
            "tone drive.nam"
        );
        assert_eq!(paths[1].file_name().unwrap().to_str().unwrap(), "IR L.wav");
    }

    #[test]
    fn keeps_only_file_uris_and_localhost_variant() {
        let body = b"http://example.test/x.wav\r\nfile://localhost/home/u/a.wav\r\n";
        let paths = parse_uri_list(body);
        assert_eq!(paths.len(), 1);
        assert!(paths[0].to_string_lossy().ends_with("/home/u/a.wav"));
    }

    #[test]
    fn undecodable_escapes_pass_through_literally() {
        assert_eq!(percent_decode(b"a%zzb%41"), b"a%zzbA");
    }
}
