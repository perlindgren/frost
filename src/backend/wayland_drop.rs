//! Native Wayland drag-and-drop *receiving* for the standalone window.
//!
//! Why this exists: winit 0.30's Wayland backend never registers a
//! `wl_data_device` listener on native Wayland (verified against
//! winit-0.30.13: the backend source contains zero `wl_data_device` /
//! `wl_data_offer` usage), so file drops only work there through XWayland,
//! where the XdndSelection protocol runs. Under a pure Wayland session
//! (KDE/Plasma, GNOME) XWayland is not involved at all and dropping a
//! plugin file onto the window silently does nothing.
//!
//! **REVISIT WHEN WINIT 0.31 STABLE SHIPS.** winit 0.31 tracks upstream
//! Wayland DnD support; once it is released (and its `DroppedFile` /
//! `HoveredFile` events fire natively on Wayland), delete this module and
//! the `#[cfg(target_os = "linux")]` touch points in `app.rs` — the
//! `WindowEvent::DroppedFile` arm in `window_event` already handles every
//! other backend. Until then this module is the only Wayland
//! `wl_data_device` on the display connection winit owns, so no events are
//! double-delivered.
//!
//! # How it coexists with winit's connection
//!
//! [`WaylandDrop`] wraps the `wl_display` winit already opened (via
//! `raw-window-handle`) in its own `wayland_client::backend::Backend`, which
//! creates a *separate* `wl_event_queue`. All proxies this module creates
//! (registry, seat, data-device manager, data device, offers) live in that
//! queue; winit-owned proxies (the surface, its own registry, …) are only
//! ever referenced, never re-queued:
//!
//! * Requests for our objects are issued through proxy wrappers pinned to
//!   our queue (`wl_proxy_create_wrapper` + `wl_proxy_set_queue` inside
//!   wayland-backend), so winit-owned proxies keep their queue.
//! * Inbound `object`-typed event arguments (e.g. the `wl_surface` and the
//!   `wl_data_offer` reference carried by `wl_data_device.enter`) are parsed
//!   as inert references — wayland-backend's `Object` argument parsing only
//!   clones the existing object data; `wl_proxy_set_queue` runs solely in
//!   `manage_object`, i.e. for `new_id` children *we* created. So winit's
//!   surface is never stolen into our queue.
//!
//! Pumping happens from winit's `about_to_wait` callback on the same thread
//! as winit's own dispatch: both event queues are drained non-blockingly,
//! which is exactly the multi-queue multiplexing libwayland is designed
//! for (each proxy's events route to the queue its data belongs to).
//!
//! # Protocol scope
//!
//! Receiving only, at `wl_data_device` version 1 (what KWin and Mutter both
//! advertise; version 1 is the whole interface we touch):
//!
//! * `data_offer` (opcode 0) is the only `new_id`-creating event; `enter`
//!   and `selection` carry the offer as a nullable *object reference*.
//! * `enter` and `motion` carry surface-local coordinates. Those two are the
//!   pointer for the whole drag — see [`WaylandDropEvent::HoverMove`] — and
//!   `app.rs` feeds them to the same cursor setter a host cursor uses.
//! * `offer.accept` on `enter` (with the enter serial) when the offer
//!   advertises `text/uri-list`, `offer.receive` into a pipe on `drop`, and
//!   `offer.destroy` to end the transfer — `offer.finish` is a v3 request
//!   and does not exist on a v1 offer.
//! * The pipe body is read on a helper thread (a blocking read must never
//!   stall the event loop); [`WaylandDrop::pump`] polls the channel with a
//!   bounded deadline so a source that never writes cannot wedge the app.
//!
//! Whether a compositor sends `leave` before or after `drop` (or at all) is
//! its own choice, and one offer is good for exactly one drag. That sequencing
//! lives in [`DragState`], which holds no proxies so every transition is
//! unit-tested without a compositor; the dispatch arms below only translate its
//! decisions into requests. Two rules there are load-bearing rather than tidy:
//! the offer must be destroyed once its drag is over ([`Dnd::destroy_offer`]),
//! because the library only frees an object id when its client destroys the
//! proxy — a kept offer makes the *next* drag's `data_offer` reuse an id the
//! library still holds, which it refuses as a protocol error, after which
//! every later drag is discarded; and an *accepted* offer must survive a
//! `leave`, or a compositor that orders `leave` first has its `drop` refused.
//!
//! The `SAFETY` note at [`WaylandDrop::new`] covers the one unsafe call.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::io::Read;
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use wayland_client::backend::Backend;
use wayland_client::protocol::wl_data_device::Event as WlDataDeviceEvent;
use wayland_client::protocol::wl_data_device::WlDataDevice;
use wayland_client::protocol::wl_data_device_manager::WlDataDeviceManager;
use wayland_client::protocol::wl_data_offer::{Event as WlDataOfferEvent, WlDataOffer};
use wayland_client::protocol::wl_registry::{Event as WlRegistryEvent, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{
    Connection, Dispatch, DispatchError, EventQueue, Proxy, QueueHandle, event_created_child,
};
use winit::window::Window;

/// The MIME type every real Wayland source (KWin, GTK, Qt) uses for file
/// drags; we only accept this one.
const URI_LIST_MIME: &str = "text/uri-list";

/// How long a `receive`d pipe may stay silent before the drop is abandoned
/// (with a warning) instead of wedging future input. KDE sources write the
/// uri-list promptly; the timeout only bounds pathological sources.
const READ_TIMEOUT: Duration = Duration::from_secs(3);

/// One outcome of a Wayland DnD interaction, shaped for `app.rs` to forward
/// into the `Process` file-drop API (mirroring the winit `WindowEvent`
/// variants that work on every other backend).
pub(crate) enum WaylandDropEvent {
    /// A drag with file(s) entered our surface, at the surface-local point the
    /// compositor named.
    HoverStart { point: [f32; 2] },
    /// The drag moved inside our surface, at the new surface-local point.
    ///
    /// This is the pointer for the whole drag: the compositor confines a
    /// real `wl_pointer` to the drag icon it draws, so winit's cursor events
    /// stop at the moment the drag starts. Without these, an external drop
    /// has no position and lands at the end of the rack instead of under the
    /// icon.
    HoverMove { point: [f32; 2] },
    /// The drag left our surface without dropping.
    HoverEnd,
    /// The drag was released over our surface; the payload was parsed.
    Dropped(Vec<PathBuf>),
}

/// Receiver state shared with the wayland-client dispatch machinery.
struct Dnd {
    /// Raw pointer of winit's `wl_surface`; only `enter` events naming this
    /// surface are acted on (editor child windows or other apps' surfaces
    /// on the same display are ignored).
    our_surface: usize,
    seat: Option<WlSeat>,
    manager: Option<WlDataDeviceManager>,
    device: Option<WlDataDevice>,
    /// The pending `data_offer` (created via the `data_offer` event). Only
    /// the offer in this slot is ever accepted or received, and
    /// [`Dnd::destroy_offer`] is the one way it ends.
    offer: Option<WlDataOffer>,
    /// The held offer's protocol lifecycle, kept in lockstep with `offer`.
    drag: DragState,
    /// In-flight `receive` payload reader: channel plus absolute deadline.
    reader: Option<(Receiver<Vec<PathBuf>>, Instant)>,
    /// Events queued by dispatch, drained by [`WaylandDrop::pump`].
    events: VecDeque<WaylandDropEvent>,
    /// Set once the connection is dead; stops dispatch, keeps the reader
    /// deadline active.
    dead: bool,
}

impl Dnd {
    /// Destroy the held offer, ending the transfer as far as v1 allows, and
    /// hand its object id back to the library. Every path that stops caring
    /// about an offer goes through here; see the module docs for what a
    /// skipped call costs.
    fn destroy_offer(&mut self) {
        if let Some(offer) = self.offer.take() {
            offer.destroy();
        }
    }

    /// Pull the uri-list body out of the accepted offer and start the thread
    /// that waits for it.
    ///
    /// `receive` hands the compositor our pipe's write end; the source writes
    /// the uri-list there and closes it, which is the EOF the reader thread
    /// waits for. Our own write-end copy must be dropped after the request is
    /// queued (the fd is transmitted at request-send time), or EOF never
    /// arrives.
    fn receive_offer(&mut self, conn: &Connection) {
        let Some(offer) = self.offer.as_ref() else {
            return;
        };
        let (mut reader, writer) = match std::io::pipe() {
            Ok(pair) => pair,
            Err(err) => {
                log::warn!("wayland drop receiver: pipe() failed ({err}); ignoring drop");
                return;
            }
        };
        offer.receive(URI_LIST_MIME.to_owned(), writer.as_fd());
        let _ = conn.flush();
        drop(writer);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if reader.read_to_end(&mut buf).is_ok() {
                let text = String::from_utf8_lossy(&buf);
                let _ = tx.send(parse_uri_list(&text));
            }
        });
        self.reader = Some((rx, Instant::now() + READ_TIMEOUT));
    }
}

/// One drag's protocol lifecycle, free of proxies so the sequencing is
/// unit-testable without a compositor (see the module docs).
///
/// `hovering` is what the application is told — the drag icon is over our
/// surface. `accepted` is what entitles us to `receive` on a `drop`: we sent
/// `accept` for this offer with its enter serial. They part company on
/// compositors that send `leave` *before* `drop`, which is the ordering
/// `wl_data_device`'s own protocol text describes; collapsing the two into one
/// flag is what made such a drop do nothing at all, because the `leave` had
/// already thrown the offer away.
#[derive(Debug, Default, PartialEq, Eq)]
struct DragState {
    /// The offer we hold advertised `text/uri-list`.
    uri_list: bool,
    /// We called `accept` on the offer we hold.
    accepted: bool,
    /// An accepted file drag is currently over our surface.
    hovering: bool,
}

/// What an incoming `wl_data_device.leave` asks us to do.
#[derive(Debug, Default, PartialEq, Eq)]
struct OnLeave {
    /// The held offer is finished with: destroy it. Kept when we accepted it,
    /// since a `drop` may still be on its way.
    destroy: bool,
    /// Tell the application the hover ended.
    end_hover: bool,
}

/// What an incoming `wl_data_device.drop` asks us to do.
#[derive(Debug, Default, PartialEq, Eq)]
struct OnDrop {
    /// `receive` the payload on the held offer.
    receive: bool,
    /// The drag is over: destroy the held offer.
    destroy: bool,
    /// Tell the application the hover ended — always, so a refused drop cannot
    /// leave it drawing a target for a drag that no longer exists.
    end_hover: bool,
}

impl DragState {
    /// A fresh `data_offer` arrived; anything from before it belongs to an
    /// earlier drag and must not influence this one. Returns whether the
    /// application must be told the previous hover ended: a new drag proves the
    /// old one is over even when the compositor never sent `leave` for it, and
    /// the next `enter` may be for an offer we will not accept at all.
    fn offer_arrived(&mut self) -> bool {
        let end_hover = self.hovering;
        *self = DragState::default();
        end_hover
    }

    /// An `offer` event naming the held offer.
    fn mime(&mut self, mime: &str) {
        self.uri_list |= mime == URI_LIST_MIME;
    }

    /// `enter` on our surface naming the held offer. Returns whether to
    /// `accept` it with `serial`; a `true` also means the application must be
    /// told a hover started.
    fn enter(&mut self) -> bool {
        if !self.uri_list {
            return false;
        }
        self.accepted = true;
        // Every accepted `enter` announces a *new* arrival, so the hover is
        // re-asserted rather than pushed only when not already hovering: a
        // sequence the compositor ended without `leave` must not be able to
        // swallow the next drag's start and leave the app with no target.
        self.hovering = true;
        true
    }

    /// `motion`: only ours while a hover is live.
    fn motion(&self) -> bool {
        self.hovering
    }

    fn leave(&mut self, holding_offer: bool) -> OnLeave {
        let end_hover = self.hovering;
        self.hovering = false;
        // An accepted offer survives the leave: this compositor may send
        // `drop` next, and only that offer can answer it.
        let destroy = holding_offer && !self.accepted;
        if destroy {
            self.uri_list = false;
            self.accepted = false;
        }
        OnLeave { destroy, end_hover }
    }

    fn drop(&mut self, holding_offer: bool) -> OnDrop {
        let receive = holding_offer && self.accepted;
        let end_hover = self.hovering;
        // The drag is over either way — including a refusal, which must not
        // strand the application in a hover it will never be told ends.
        *self = DragState::default();
        OnDrop {
            receive,
            destroy: holding_offer,
            end_hover,
        }
    }
}

/// Native Wayland file-drop receiver attached to a winit window. See the
/// module documentation for scope and the planned retirement path.
pub(crate) struct WaylandDrop {
    conn: Connection,
    queue: EventQueue<Dnd>,
    state: Dnd,
}

impl WaylandDrop {
    /// Attach to `window`'s Wayland display. Returns `None` (silently, with
    /// the X11 fallback in `window_event` still armed) when the window is
    /// not a native Wayland surface or the globals we need are missing;
    /// missing globals are detected lazily during the first pumps.
    pub(crate) fn new(window: &Window) -> Option<Self> {
        let RawDisplayHandle::Wayland(display) = window.display_handle().ok()?.as_raw() else {
            return None;
        };
        let RawWindowHandle::Wayland(surface) = window.window_handle().ok()?.as_raw() else {
            return None;
        };

        // SAFETY: `display.display` is the live `wl_display*` that winit
        // itself opened and keeps alive for at least as long as the window.
        // `from_foreign_display` only borrows it (it does not take
        // ownership) and puts everything it creates into its own event
        // queue; it exists because winit's default `wayland` feature
        // selects wayland-backend's `client_system` build (its Cargo.toml
        // declares `wayland-backend` with `features = ["client_system"]`),
        // and our own Cargo.toml pins the same feature explicitly rather
        // than relying on a dependency's opinion. This is the one place
        // this module crosses into winit-owned objects; everything else
        // goes through non-owning queue wrappers (see module docs).
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };

        let conn = Connection::from_backend(backend);
        let queue = conn.new_event_queue::<Dnd>();
        let qh = queue.handle();
        let our_surface = surface.surface.as_ptr() as usize;
        let state = Dnd {
            our_surface,
            seat: None,
            manager: None,
            device: None,
            offer: None,
            drag: DragState::default(),
            reader: None,
            events: VecDeque::new(),
            dead: false,
        };
        // Queues the registry request; globals are bound from `Global`
        // events during the first pumps, and a display without
        // `wl_data_device_manager` simply never gets a device.
        conn.display().get_registry(&qh, ());
        let _ = conn.flush();
        Some(Self { conn, queue, state })
    }

    /// Non-blockingly drain this module's wayland event queue and the
    /// pending `receive` payload, returning everything that happened since
    /// the last call. Call from the event loop's `about_to_wait`.
    pub(crate) fn pump(&mut self) -> Vec<WaylandDropEvent> {
        let mut out = Vec::new();
        if !self.state.dead {
            let _ = self.conn.flush();
            loop {
                match self.queue.dispatch_pending(&mut self.state) {
                    Ok(0) => break,
                    Ok(_) => continue,
                    Err(DispatchError::Backend(err)) => {
                        // Connection is gone; further pumps would only
                        // repeat this. winit will exit through its own
                        // path, so no shutdown logic lives here.
                        self.state.dead = true;
                        // A dead compositor never sends the `leave` that
                        // would have ended an in-progress hover, so end it
                        // here rather than leave the application showing a
                        // target region for a drag that cannot continue.
                        if self.state.drag.leave(false).end_hover {
                            self.state.events.push_back(WaylandDropEvent::HoverEnd);
                        }
                        log::warn!(
                            "wayland drop receiver: connection error ({err}); ignoring further wayland traffic"
                        );
                        break;
                    }
                    Err(err) => {
                        log::warn!("wayland drop receiver: discarding malformed event ({err})");
                        break;
                    }
                }
            }
        }
        // In dispatch order first (hover transitions), then any payload
        // that arrived, so consumers see HoverStart before Dropped.
        out.extend(self.state.events.drain(..));
        if let Some((rx, deadline)) = self.state.reader.take() {
            match rx.try_recv() {
                Ok(paths) => out.push(WaylandDropEvent::Dropped(paths)),
                Err(TryRecvError::Empty) => {
                    if Instant::now() < deadline {
                        self.state.reader = Some((rx, deadline));
                    } else {
                        log::warn!(
                            "wayland drop receiver: source never sent the dropped paths; ignoring drop"
                        );
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    log::warn!("wayland drop receiver: reader thread died; ignoring drop");
                }
            }
        }
        out
    }
}

/// Bind the seat / data-device-manager globals (at v1, the only version
/// this module speaks) and create the data device once both are present.
fn maybe_make_device(state: &mut Dnd, qh: &QueueHandle<Dnd>) {
    if state.device.is_some() {
        return;
    }
    if let (Some(manager), Some(seat)) = (state.manager.as_ref(), state.seat.as_ref()) {
        state.device = Some(manager.get_data_device(seat, qh, ()));
    }
}

/// Compare two proxies by the `libwayland` pointer behind them. Event
/// arguments referencing an object we created hand back the same underlying
/// proxy; the pointer is stable for the object's whole life, and an
/// already-destroyed one can never hand its id out again.
fn same_object(a: &impl Proxy, b: &impl Proxy) -> bool {
    std::ptr::eq(a.id().as_ptr(), b.id().as_ptr())
}

impl Dispatch<WlRegistry, ()> for Dnd {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: WlRegistryEvent,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        // Only `Global` matters; `GlobalRemove` of these globals would mean
        // a compositor shutdown, which the dead-connection path handles.
        if let WlRegistryEvent::Global {
            name,
            interface,
            version,
        } = event
            && version >= 1
        {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind::<WlSeat, _, _>(name, 1, qh, ()));
                    maybe_make_device(state, qh);
                }
                "wl_data_device_manager" if state.manager.is_none() => {
                    state.manager =
                        Some(registry.bind::<WlDataDeviceManager, _, _>(name, 1, qh, ()));
                    maybe_make_device(state, qh);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for Dnd {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // `capabilities` is informational here: a drag reaching us proves
        // drag-and-drop support better than any announcement.
    }
}

impl Dispatch<WlDataDeviceManager, ()> for Dnd {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The manager is a factory; it sends no events itself.
    }
}

impl Dispatch<WlDataDevice, ()> for Dnd {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: WlDataDeviceEvent,
        _: &(),
        conn: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            WlDataDeviceEvent::DataOffer { id } => {
                // A new drag replaced any offer we were still holding from
                // a sequence that ended without `leave`/`drop`. Destroying
                // the stale one is not hygiene for its own sake: v1 has no
                // `finish`, `destroy` is the only thing that hands a new-id
                // back to the library, and an offer we keep alive blocks the
                // id the next `data_offer` arrives with — which the library
                // then refuses, poisoning this queue for every later drag.
                state.destroy_offer();
                state.offer = Some(id);
                if state.drag.offer_arrived() {
                    // A new drag proves the previous one is over even if the
                    // compositor never said so.
                    state.events.push_back(WaylandDropEvent::HoverEnd);
                }
            }
            WlDataDeviceEvent::Enter {
                serial,
                surface,
                id,
                x,
                y,
            } => {
                // Ignore drops aimed at foreign surfaces (e.g. editor
                // child windows created by integrations); their own
                // toolkits handle them.
                let ours = surface.id().as_ptr() as usize == state.our_surface;
                let tracked = id.as_ref().is_some_and(|offer| {
                    state
                        .offer
                        .as_ref()
                        .is_some_and(|held| same_object(offer, held))
                });
                if ours && tracked && state.drag.enter() {
                    // The accept must carry the enter serial; sending it
                    // immediately (plus the flush in `pump`) keeps the
                    // round trip short.
                    if let Some(offer) = state.offer.as_ref() {
                        offer.accept(serial, Some(URI_LIST_MIME.to_owned()));
                    }
                    // Enter and motion coordinates are both surface-local
                    // `Fixed` — the wire format the generated bindings hand
                    // us as `f64` — so they carry the same unit and origin
                    // as the `wl_pointer.motion` a host cursor arrives
                    // through, and `app.rs` can feed either into the same
                    // setter. The event goes out on every accepted enter,
                    // not only the first: a redundant hover costs the
                    // application one event, while swallowing a real enter
                    // because a stale flag claims we are still hovering
                    // hides the target region for the whole drag.
                    state.events.push_back(WaylandDropEvent::HoverStart {
                        point: [x as f32, y as f32],
                    });
                }
            }
            WlDataDeviceEvent::Leave => {
                // Compositors disagree about whether `leave` precedes
                // `drop`, so this arm has to work in both orders: an offer
                // we accepted stays alive for a `drop` that may still be
                // coming, one we never accepted is finished business.
                let left = state.drag.leave(state.offer.is_some());
                if left.destroy {
                    state.destroy_offer();
                }
                if left.end_hover {
                    state.events.push_back(WaylandDropEvent::HoverEnd);
                }
            }
            WlDataDeviceEvent::Drop => {
                // A drop we never accepted (foreign surface, non-file drag)
                // must not start a receive, but its offer is still destroyed:
                // v1 gives a transfer no other ending, and an offer we keep
                // alive steals the id the next drag arrives with.
                let dropped = state.drag.drop(state.offer.is_some());
                if dropped.receive {
                    state.receive_offer(conn);
                }
                if dropped.destroy {
                    state.destroy_offer();
                }
                if dropped.end_hover {
                    state.events.push_back(WaylandDropEvent::HoverEnd);
                }
            }
            WlDataDeviceEvent::Motion { x, y, .. } => {
                // These are the only pointer coordinates the application sees
                // for the whole drag: the compositor confines the real pointer
                // to the drag icon it draws, so winit's cursor events stop
                // when the drag starts. `hovering` is our own record of the
                // drag being inside our surface — it stays false for a drag
                // aimed at a foreign one, whose toolkit needs this motion.
                if state.drag.motion() {
                    state.events.push_back(WaylandDropEvent::HoverMove {
                        point: [x as f32, y as f32],
                    });
                }
            }
            // `Selection` is clipboard data (a different offer object we
            // leave alone).
            _ => {}
        }
    }

    event_created_child!(Self, WlDataDevice, [0 => (WlDataOffer, ())]);
}

impl Dispatch<WlDataOffer, ()> for Dnd {
    fn event(
        state: &mut Self,
        offer: &WlDataOffer,
        event: WlDataOfferEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // v1 offers only carry `offer`; `source_actions`/`action` are v3.
        // Offers arrive before `enter` and can arrive after it, so this is
        // deliberately independent of the hover state: `enter` decides with
        // whatever has been advertised by the time it runs, and an `offer`
        // that lands later only matters for a *future* enter, which no
        // compositor sends for a drag already in progress.
        if let WlDataOfferEvent::Offer { mime_type } = event
            && state
                .offer
                .as_ref()
                .is_some_and(|held| same_object(offer, held))
        {
            state.drag.mime(&mime_type);
        }
    }
}

/// Decode a `text/uri-list` body (RFC 2483) into local paths.
///
/// Lines are URIs, one per line, CRLF-separated, `#`-prefixed comments
/// allowed. Fragments/queries (e.g. `file:///a/b#1`) are stripped; percent
/// decoding is lenient (invalid escapes stay literal) and non-UTF-8 escapes
/// use the lossy conversion. `file://` URIs with an empty or `localhost`
/// host are accepted, as are bare absolute paths (some sources omit the
/// scheme); every other scheme is skipped with a warning.
fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Split before percent-decoding: encoded `%23`/`%3F` in file names
        // survive, literal `#`/`?` fragment/query separators are dropped.
        let uri = line
            .split_once(['#', '?'])
            .map_or(line, |(base, _tail)| base);
        let uri = percent_decode(uri);
        let path = if let Some(rest) = uri.strip_prefix("file://") {
            match rest.find('/') {
                // `file:///path` (empty host) or `file://localhost/path`.
                Some(idx) if idx == 0 || rest[..idx].eq_ignore_ascii_case("localhost") => {
                    Cow::Borrowed(&rest[idx..])
                }
                Some(idx) => {
                    log::warn!(
                        "wayland drop receiver: ignoring non-local file URI host `{}`",
                        &rest[..idx]
                    );
                    continue;
                }
                None => {
                    // `file://host` with no path component at all.
                    if !(rest.is_empty() || rest.eq_ignore_ascii_case("localhost")) {
                        log::warn!(
                            "wayland drop receiver: ignoring non-local file URI host `{rest}`"
                        );
                    }
                    continue;
                }
            }
        } else if uri.starts_with('/') {
            Cow::Borrowed(&uri[..])
        } else {
            log::warn!("wayland drop receiver: ignoring unsupported dropped URI `{uri}`");
            continue;
        };
        if path.is_empty() || path == "/" {
            continue;
        }
        out.push(PathBuf::from(path.as_ref()));
    }
    out
}

/// Lenient percent-decoding: valid `%XX` escapes are decoded, malformed
/// ones stay literal; the result is lossily converted to UTF-8.
fn percent_decode(s: &str) -> Cow<'_, str> {
    if !s.contains('%') {
        return Cow::Borrowed(s);
    }
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    match String::from_utf8(out) {
        Ok(decoded) => Cow::Owned(decoded),
        Err(err) => Cow::Owned(String::from_utf8_lossy(err.as_bytes()).into_owned()),
    }
}

fn hex_val(b: u8) -> Option<u8> {
    (b as char).to_digit(16).map(|v| v as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_file_uri() {
        let paths = parse_uri_list("file:///home/user/plugin.clap\n");
        assert_eq!(paths, vec![PathBuf::from("/home/user/plugin.clap")]);
    }

    #[test]
    fn percent_decoded_spaces_and_utf8() {
        let paths = parse_uri_list("file:///home/my%20user/caf%C3%A9.clap\r\n");
        assert_eq!(paths, vec![PathBuf::from("/home/my user/café.clap")]);
    }

    #[test]
    fn localhost_host_accepted_other_hosts_skipped() {
        let paths = parse_uri_list("file://localhost/a/b.clap\nfile://elsewhere/a/b.clap\n");
        assert_eq!(paths, vec![PathBuf::from("/a/b.clap")]);
    }

    #[test]
    fn bare_path_fallback() {
        let paths = parse_uri_list("/tmp/hard/stereo.clap");
        assert_eq!(paths, vec![PathBuf::from("/tmp/hard/stereo.clap")]);
    }

    #[test]
    fn fragment_and_query_stripped_before_decode() {
        let paths = parse_uri_list("file:///a/b%23tag.clap#line?x=1\nfile:///c#d\n");
        assert_eq!(
            paths,
            vec![PathBuf::from("/a/b#tag.clap"), PathBuf::from("/c")]
        );
    }

    #[test]
    fn comments_and_blank_lines_skipped() {
        let paths = parse_uri_list("# comment\r\n\r\nfile:///x.clap\r\n\r\nfile:///y.clap");
        assert_eq!(
            paths,
            vec![PathBuf::from("/x.clap"), PathBuf::from("/y.clap")]
        );
    }

    #[test]
    fn other_schemes_skipped() {
        let paths = parse_uri_list(
            "https://example.com/f.clap\nx-special/gnome-icons:/f\nfile:///keep.clap\n",
        );
        assert_eq!(paths, vec![PathBuf::from("/keep.clap")]);
    }

    #[test]
    fn malformed_escapes_stay_literal() {
        let paths = parse_uri_list("file:///a%2/b.clap\nfile:///c%ZZd.clap\nfile:///e%2F.clap");
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/a%2/b.clap"),
                PathBuf::from("/c%ZZd.clap"),
                PathBuf::from("/e/.clap"),
            ]
        );
    }

    #[test]
    fn multiple_uris_one_line_each() {
        let paths = parse_uri_list("file:///one.clap\nfile:///two.so\nfile:///three.vst3");
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/one.clap"),
                PathBuf::from("/two.so"),
                PathBuf::from("/three.vst3"),
            ]
        );
    }

    #[test]
    fn root_and_empty_paths_dropped() {
        assert!(parse_uri_list("file:///\nfile://localhost\n").is_empty());
    }
}

/// The sequencing rules a compositor may exercise, exercised without one.
///
/// A protocol test would need a compositor, and the only way to get one
/// offline is a new dependency, which the repository rules rule out. These
/// tests therefore pin the decision table the dispatcher delegates to: the
/// two `leave`/`drop` orderings both end with the offer destroyed and the
/// application told to stop hovering, and a `leave` never throws away an
/// offer its `drop` still needs.
#[cfg(test)]
mod sequence_tests {
    use super::{DragState, URI_LIST_MIME};

    /// An offer that advertises files, entered on our surface.
    fn hovering(mut drag: DragState) -> DragState {
        drag.offer_arrived();
        drag.mime(URI_LIST_MIME);
        assert!(drag.enter());
        drag
    }

    #[test]
    fn leave_before_drop_keeps_the_accepted_offer_for_the_drop() {
        let mut drag = hovering(DragState::default());
        let left = drag.leave(true);
        assert!(
            !left.destroy,
            "destroying an accepted offer on `leave` is what swallowed the drop"
        );
        assert!(left.end_hover);

        let dropped = drag.drop(true);
        assert!(dropped.receive, "the surviving offer must answer the drop");
        assert!(dropped.destroy, "and be destroyed once answered");
        assert!(
            !dropped.end_hover,
            "the hover already ended, so no duplicate event"
        );
        assert_eq!(drag, DragState::default());
    }

    #[test]
    fn drop_before_leave_ends_everything_and_the_leave_is_a_no_op() {
        let mut drag = hovering(DragState::default());
        let dropped = drag.drop(true);
        assert!(dropped.receive && dropped.destroy && dropped.end_hover);

        // Our `destroy` is the client-side half of the compositor's own
        // cleanup; the `leave` that follows it has nothing left to do.
        let left = drag.leave(false);
        assert!(!left.destroy && !left.end_hover);
        assert_eq!(drag, DragState::default());
    }

    #[test]
    fn an_unaccepted_offer_is_destroyed_by_leave_without_touching_the_hover() {
        let mut drag = DragState::default();
        drag.offer_arrived();
        drag.mime("text/plain");
        assert!(!drag.enter(), "a non-file drag is never accepted");
        let left = drag.leave(true);
        assert!(left.destroy, "an offer we will not use must free its id");
        assert!(!left.end_hover, "we never told the app it was hovering");
    }

    #[test]
    fn a_drop_needs_both_an_offer_and_an_accept() {
        // Offer present, never accepted (foreign surface, or no `enter`).
        let mut drag = DragState::default();
        drag.offer_arrived();
        drag.mime(URI_LIST_MIME);
        let dropped = drag.drop(true);
        assert!(!dropped.receive);
        assert!(dropped.destroy);

        // Accepted earlier, but the offer itself is gone.
        let mut drag = hovering(DragState::default());
        let dropped = drag.drop(false);
        assert!(!dropped.receive, "nothing to receive from");
        assert!(dropped.end_hover, "the app still gets its hover back");
    }

    #[test]
    fn a_new_offer_restarts_the_drag_from_scratch() {
        // A compositor may drop the `leave` of a drag that ended on our
        // surface; the next offer must not inherit its knowledge.
        let mut drag = hovering(DragState::default());
        assert!(drag.offer_arrived(), "the abandoned hover must end");
        assert!(!drag.enter(), "the new offer has advertised nothing yet");
        drag.mime("text/plain");
        assert!(!drag.enter(), "and nothing we would accept");
    }

    #[test]
    fn a_repeated_enter_re_asserts_the_hover() {
        // Some compositors re-enter on the same offer (pointer crossing a
        // sub-surface, for instance), and others never sent the previous
        // `leave`. Either way the application must be told again, since a
        // swallowed start leaves it with no target region for the drag.
        let mut drag = hovering(DragState::default());
        assert!(drag.enter());
        assert!(drag.enter());
    }

    #[test]
    fn motion_follows_the_hover() {
        let mut drag = hovering(DragState::default());
        assert!(drag.motion());
        drag.leave(true);
        assert!(!drag.motion(), "motion after leave is the app's cursor");
        drag.drop(true);
        assert!(!drag.motion());
    }
}
