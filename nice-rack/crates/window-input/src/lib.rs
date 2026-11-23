//! Native cursor fallback for OS file drags which do not generate cursor events.
#![deny(unsafe_op_in_unsafe_fn)]
use raw_window_handle::HasWindowHandle;

/// Cursor in logical window points, centered with y up. Only valid on the
/// main thread while the supplied window lives. Other platforms use the normal
/// event position in the application and return None from this fallback.
///
/// `scale` is the window's own scale factor, physical pixels per logical
/// point, as the windowing layer reports it. X11 is the platform that needs
/// it: the protocol answers the pointer query in pixels and has no per-window
/// DPI, so the scale the window itself reports is the only number that lands
/// in the same coordinate space as the cursor events the application gets for
/// every other frame. macOS converts through the view and Windows divides by
/// the window's DPI, so both ignore it.
pub fn cursor(window: &impl HasWindowHandle, scale: f32) -> Option<[f32; 2]> {
    #[cfg(not(target_os = "linux"))]
    let _ = scale;

    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSEvent, NSView};
        let _main = MainThreadMarker::new()?;
        let borrowed = window.window_handle().ok()?;
        let raw_window_handle::RawWindowHandle::AppKit(handle) = borrowed.as_raw() else {
            return None;
        };
        // SAFETY: the borrowed AppKit handle guarantees ns_view denotes the
        // live NSView for this window while borrowed is alive. We are on the
        // main thread, keep the borrow and window for every call, do not retain
        // or expose the pointer, and send no mutating messages.
        let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        let native = view.window()?;
        let point = view.convertPoint_fromView(
            native.convertPointFromScreen(NSEvent::mouseLocation()),
            None,
        );
        let bounds = view.bounds();
        let x = point.x - bounds.origin.x;
        let y = point.y - bounds.origin.y;
        if x < 0.0 || y < 0.0 || x > bounds.size.width || y > bounds.size.height {
            return None;
        }
        Some([
            (x - bounds.size.width / 2.0) as f32,
            if view.isFlipped() {
                (bounds.size.height / 2.0 - y) as f32
            } else {
                (y - bounds.size.height / 2.0) as f32
            },
        ])
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{POINT, RECT};
        use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
        use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetCursorPos};

        let borrowed = window.window_handle().ok()?;
        let raw_window_handle::RawWindowHandle::Win32(handle) = borrowed.as_raw() else {
            return None;
        };
        let hwnd = handle.hwnd.get();
        let mut point = POINT { x: 0, y: 0 };
        // SAFETY: hwnd comes from the borrowed handle of a live window and is
        // only used for these synchronous user32 calls on this thread. point is
        // an aligned, initialized local which the callee fills before returning;
        // no pointer is retained, stored, or sent to another thread.
        if unsafe { GetCursorPos(&mut point) } == 0 {
            return None;
        }
        // SAFETY: the same borrowed hwnd, and point stays a writable local which
        // the call rewrites in place from screen to client coordinates.
        if unsafe { ScreenToClient(hwnd, &mut point) } == 0 {
            return None;
        }
        let mut client = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: the same borrowed hwnd, and client is an initialized local the
        // call fills synchronously.
        if unsafe { GetClientRect(hwnd, &mut client) } == 0 {
            return None;
        }
        // SAFETY: the same borrowed hwnd, passed by value to a query which only
        // reads the window's own DPI state.
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        centered_in_client(
            point.x,
            point.y,
            client.right,
            client.bottom,
            dpi_to_scale(dpi),
        )
    }
    #[cfg(target_os = "linux")]
    {
        use x11rb::protocol::xproto::ConnectionExt as _;

        let borrowed = window.window_handle().ok()?;
        // X11 names a window by a 32-bit id. An Xlib handle carries it in a
        // `c_ulong` — 64-bit on a 64-bit platform — and an Xcb handle in a
        // `NonZeroU32`; both denote the same window on the same display, which
        // is the display the cached connection below has open. Anything else
        // has no X id to ask, and a Wayland window above all — there the
        // compositor confines the pointer to the drag icon, so the coordinates
        // arrive over the data device in `frost::backend::wayland_drop`.
        let win = match borrowed.as_raw() {
            raw_window_handle::RawWindowHandle::Xlib(handle) => {
                u32::try_from(handle.window).ok()?
            }
            raw_window_handle::RawWindowHandle::Xcb(handle) => handle.window.get(),
            _ => return None,
        };
        let connection = x11_connection()?;
        // `QueryPointer` reports the pointer relative to the window it is
        // asked about, which is exactly the origin `GetClientRect` gives on
        // Windows — but only while the pointer is on that window's screen.
        // Elsewhere it answers with root coordinates of the other screen and
        // zeroes the window-relative pair, so a second monitor would otherwise
        // read as the top-left corner rather than as no point at all.
        let pointer = connection.query_pointer(win).ok()?.reply().ok()?;
        if !pointer.same_screen {
            return None;
        }
        let client = connection.get_geometry(win).ok()?.reply().ok()?;
        centered_in_client(
            pointer.win_x as i32,
            pointer.win_y as i32,
            client.width as i32,
            client.height as i32,
            scale,
        )
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = window;
        None
    }
}

/// This thread's X connection, opened on first use and kept for its lifetime.
///
/// A query costs one socket round trip; opening a connection costs a socket,
/// an auth handshake and the setup reply, so caching is what keeps this
/// cheap enough to call once per frame while a drag is in flight. A refused
/// connection is remembered rather than retried: a display that is not there
/// does not start answering because we ask again every frame, and the caller
/// then falls back to exactly what it had before this arm existed.
#[cfg(target_os = "linux")]
fn x11_connection() -> Option<std::rc::Rc<x11rb::rust_connection::RustConnection>> {
    use std::cell::RefCell;
    use std::rc::Rc;
    use x11rb::rust_connection::RustConnection;

    // `None` until the first attempt, `Some(None)` once an attempt has failed.
    thread_local! {
        static CONNECTION: RefCell<Option<Option<Rc<RustConnection>>>> = const {
            RefCell::new(None)
        };
    }

    CONNECTION.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(
                RustConnection::connect(None)
                    .ok()
                    .map(|(conn, _)| Rc::new(conn)),
            );
        }
        slot.as_ref().and_then(|cached| cached.as_ref().cloned())
    })
}

/// Point in client-area pixels to logical points centered on the window, y up.
///
/// Both callers report a client-area origin of (0, 0) — `GetClientRect` always
/// does, X11 because `QueryPointer` answers relative to the window's own
/// top-left — so the pixel point and the size share one origin. A point outside
/// the client area yields None, which leaves a drop over the title bar, or over
/// another monitor, to the caller's own fallback.
#[cfg(any(windows, target_os = "linux"))]
fn centered_in_client(
    x_px: i32,
    y_px: i32,
    client_width: i32,
    client_height: i32,
    scale: f32,
) -> Option<[f32; 2]> {
    if client_width <= 0 || client_height <= 0 {
        return None;
    }
    // A scale of zero or below, or a NaN, would move the point somewhere the
    // pointer never was; no point is better than a wrong one.
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    if x_px < 0 || y_px < 0 || x_px >= client_width || y_px >= client_height {
        return None;
    }
    let x = x_px as f32 / scale;
    let y = y_px as f32 / scale;
    Some([
        x - client_width as f32 / scale / 2.0,
        client_height as f32 / scale / 2.0 - y,
    ])
}

/// `GetDpiForWindow` returns the same scale winit reports, and 0 only once the
/// window is gone; 96 keeps the division honest rather than reporting a point.
#[cfg(windows)]
fn dpi_to_scale(dpi: u32) -> f32 {
    if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 }
}

#[cfg(all(test, any(windows, target_os = "linux")))]
mod tests {
    use super::centered_in_client;

    #[test]
    fn client_center_is_the_origin_at_one_hundred_percent() {
        assert_eq!(
            centered_in_client(400, 300, 800, 600, 1.0),
            Some([0.0, 0.0])
        );
    }

    #[test]
    fn top_left_stays_top_left_with_y_up() {
        assert_eq!(
            centered_in_client(0, 0, 800, 600, 1.0),
            Some([-400.0, 300.0])
        );
    }

    #[test]
    fn client_center_is_the_origin_at_one_hundred_fifty_percent() {
        assert_eq!(
            centered_in_client(600, 450, 1200, 900, 1.5),
            Some([0.0, 0.0])
        );
    }

    #[test]
    fn one_column_step_is_one_column_step_at_one_hundred_fifty_percent() {
        // 224 logical points, the rack's column step, is 336 pixels at 150%.
        assert_eq!(
            centered_in_client(600 + 336, 450, 1200, 900, 1.5),
            Some([224.0, 0.0])
        );
    }

    #[test]
    fn points_outside_the_client_area_are_none() {
        assert_eq!(centered_in_client(-1, 300, 800, 600, 1.0), None);
        assert_eq!(centered_in_client(400, -1, 800, 600, 1.0), None);
        assert_eq!(centered_in_client(800, 300, 800, 600, 1.0), None);
        assert_eq!(centered_in_client(400, 600, 800, 600, 1.0), None);
    }

    #[test]
    fn an_empty_client_area_is_none() {
        assert_eq!(centered_in_client(10, 10, 0, 600, 1.0), None);
        assert_eq!(centered_in_client(10, 10, 800, 0, 1.0), None);
    }

    #[test]
    fn a_scale_that_cannot_divide_is_none() {
        assert_eq!(centered_in_client(400, 300, 800, 600, 0.0), None);
        assert_eq!(centered_in_client(400, 300, 800, 600, -1.0), None);
        assert_eq!(centered_in_client(400, 300, 800, 600, f32::NAN), None);
    }

    #[cfg(windows)]
    #[test]
    fn a_window_without_dpi_reports_one_hundred_percent() {
        use super::dpi_to_scale;
        assert_eq!(dpi_to_scale(0), 1.0);
        assert_eq!(dpi_to_scale(144), 1.5);
    }
}
