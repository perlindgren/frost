//! Display-synchronized main-thread callbacks for an existing macOS view.
//! Native operations are confined here; callers use only safe Rust.
#![cfg(target_os = "macos")]
#![deny(
    unsafe_op_in_unsafe_fn,
    clippy::undocumented_unsafe_blocks,
    missing_docs
)]

use objc2::rc::Retained;
use objc2::{available, sel};
use objc2_app_kit::NSView;
use objc2_foundation::{MainThreadMarker, NSRunLoop, NSRunLoopCommonModes};
use objc2_quartz_core::CADisplayLink;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

mod callback;
use callback::Callback;
mod target;
use target::Target;

/// Owns a display callback. Drop invalidates it synchronously on the main thread.
/// The callback follows the view's display and pauses when the view is hidden.
/// Return false from the callback to stop it. Rendering must not block on audio.
///
/// ```compile_fail
/// fn require_send<T: Send>() {}
/// require_send::<macos_frame_sync::DisplaySync>();
/// ```
pub struct DisplaySync {
    link: Retained<CADisplayLink>,
    target: Target,
}

impl DisplaySync {
    /// Attach to a borrowed AppKit view on macOS 14+. Returns an error when
    /// unsupported, off the main thread, or the native handle is unavailable.
    /// No reference to the borrowed window handle escapes this call.
    pub fn new(
        window: &impl HasWindowHandle,
        callback: impl FnMut() -> bool + 'static,
    ) -> Result<Self, String> {
        let mtm =
            MainThreadMarker::new().ok_or("Display synchronization requires the main thread")?;
        if !available!(macos = 14.0) {
            return Err("Display synchronization requires macOS 14 or newer".into());
        }
        let handle = window.window_handle().map_err(|e| e.to_string())?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err("Display synchronization requires an AppKit view".into());
        };
        let target = Target::new(Callback::new(callback), mtm)?;
        // SAFETY: HasWindowHandle's AppKit contract guarantees a live NSView
        // for this borrow; we are on the main thread. It is used only to create
        // the display link, never stored or messaged after this borrowed call.
        let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        // SAFETY: Target implements frame: with the exact display-link
        // callback signature. The retained link owns its target until invalidated.
        let link = unsafe { view.displayLinkWithTarget_selector(target.object(), sel!(frame:)) };
        // SAFETY: This immutable Foundation constant is a valid run-loop mode.
        let mode = unsafe { NSRunLoopCommonModes };
        // SAFETY: Both creation and registration occur on the main thread,
        // and callbacks are dispatched on this main run loop (also during tracking).
        unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), mode) };
        Ok(Self { link, target })
    }

    /// Whether the callback panicked. The link is stopped when this happens.
    pub fn has_panicked(&self) -> bool {
        self.target.has_panicked()
    }
}

impl Drop for DisplaySync {
    fn drop(&mut self) {
        self.target.stop();
        self.link.invalidate();
    }
}
