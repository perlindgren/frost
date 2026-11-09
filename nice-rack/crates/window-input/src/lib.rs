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
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window;
        None
    }
}
