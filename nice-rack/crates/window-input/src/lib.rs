//! Native cursor fallback for OS file drags which do not generate cursor events.
#![deny(unsafe_op_in_unsafe_fn)]
use raw_window_handle::HasWindowHandle;

/// Cursor in logical window points, centered with y up. Only valid on the
/// main thread while the supplied window lives. Other platforms use the normal
/// event position in the application and return None from this fallback.
pub fn cursor(window: &impl HasWindowHandle) -> Option<[f32; 2]> {
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
        centered_in_client(point.x, point.y, client.right, client.bottom, dpi)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = window;
        None
    }
}

/// Point in client-area pixels to logical points centered on the window, y up.
///
/// `GetClientRect` always reports its top-left as (0, 0), so the pixel point
/// from `ScreenToClient` and the size share one origin. A point outside the
/// client area yields None, which leaves a drop over the title bar, or over
/// another monitor, to the caller's own fallback.
#[cfg(windows)]
fn centered_in_client(
    x_px: i32,
    y_px: i32,
    client_width: i32,
    client_height: i32,
    dpi: u32,
) -> Option<[f32; 2]> {
    if client_width <= 0 || client_height <= 0 {
        return None;
    }
    if x_px < 0 || y_px < 0 || x_px >= client_width || y_px >= client_height {
        return None;
    }
    // GetDpiForWindow returns the same scale winit reports, and 0 only once the
    // window is gone; 96 keeps the division honest rather than reporting a point.
    let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
    let x = x_px as f32 / scale;
    let y = y_px as f32 / scale;
    Some([
        x - client_width as f32 / scale / 2.0,
        client_height as f32 / scale / 2.0 - y,
    ])
}

#[cfg(all(test, windows))]
mod tests {
    use super::centered_in_client;

    #[test]
    fn client_center_is_the_origin_at_one_hundred_percent() {
        assert_eq!(centered_in_client(400, 300, 800, 600, 96), Some([0.0, 0.0]));
    }

    #[test]
    fn top_left_stays_top_left_with_y_up() {
        assert_eq!(
            centered_in_client(0, 0, 800, 600, 96),
            Some([-400.0, 300.0])
        );
    }

    #[test]
    fn client_center_is_the_origin_at_one_hundred_fifty_percent() {
        assert_eq!(
            centered_in_client(600, 450, 1200, 900, 144),
            Some([0.0, 0.0])
        );
    }

    #[test]
    fn one_column_step_is_one_column_step_at_one_hundred_fifty_percent() {
        // 224 logical points, the rack's column step, is 336 pixels at 150%.
        assert_eq!(
            centered_in_client(600 + 336, 450, 1200, 900, 144),
            Some([224.0, 0.0])
        );
    }

    #[test]
    fn points_outside_the_client_area_are_none() {
        assert_eq!(centered_in_client(-1, 300, 800, 600, 96), None);
        assert_eq!(centered_in_client(400, -1, 800, 600, 96), None);
        assert_eq!(centered_in_client(800, 300, 800, 600, 96), None);
        assert_eq!(centered_in_client(400, 600, 800, 600, 96), None);
    }

    #[test]
    fn a_window_without_dpi_reports_ninety_six() {
        assert_eq!(centered_in_client(400, 300, 800, 600, 0), Some([0.0, 0.0]));
    }

    #[test]
    fn an_empty_client_area_is_none() {
        assert_eq!(centered_in_client(10, 10, 0, 600, 96), None);
        assert_eq!(centered_in_client(10, 10, 800, 0, 96), None);
    }
}
