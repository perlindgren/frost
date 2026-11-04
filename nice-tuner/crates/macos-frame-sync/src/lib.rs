//! Display-synchronized main-thread callbacks for an existing macOS view.
//! Native operations are confined here; callers use only safe Rust.
#![cfg(target_os = "macos")]
#![deny(
    unsafe_op_in_unsafe_fn,
    clippy::undocumented_unsafe_blocks,
    missing_docs
)]

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, Message, available, define_class, msg_send, sel};
use objc2_app_kit::NSView;
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSRunLoop, NSRunLoopCommonModes,
};
use objc2_quartz_core::CADisplayLink;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};

struct Callback {
    call: RefCell<Box<dyn FnMut() -> bool>>,
    stopped: Cell<bool>,
    panicked: Cell<bool>,
}
impl Callback {
    fn new(call: impl FnMut() -> bool + 'static) -> Self {
        Self {
            call: RefCell::new(Box::new(call)),
            stopped: Cell::new(false),
            panicked: Cell::new(false),
        }
    }
    fn invoke(&self) -> bool {
        if self.stopped.get() {
            return false;
        }
        // A nested run-loop dispatch must not recursively render the same UI.
        let Ok(mut call) = self.call.try_borrow_mut() else {
            return true;
        };
        let keep_running = match catch_unwind(AssertUnwindSafe(&mut *call)) {
            Ok(keep_running) => keep_running,
            Err(_) => {
                self.panicked.set(true);
                false
            }
        };
        // The callback may have dropped its driver while it was running.
        let keep_running = keep_running && !self.stopped.get();
        self.stopped.set(!keep_running);
        keep_running
    }
}

define_class!(
    // SAFETY: NSObject has no subclass-specific invariants. This class has no
    // custom Drop and is main-thread-only, including its non-Send callback.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "FrostNiceTunerDisplaySyncTarget"]
    #[ivars = Callback]
    struct Target;

    // SAFETY: NSObjectProtocol adds no requirements beyond NSObject.
    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        // SAFETY: CADisplayLink invokes a void method with itself as its one
        // object argument. The signature matches exactly and panics in the
        // supplied Rust callback are contained by Callback::invoke.
        #[unsafe(method(frame:))]
        fn frame(&self, link: &CADisplayLink) {
            // The callback is allowed to drop its driver. Retain both native
            // objects locally so invalidation cannot release them mid-call.
            let _target = self.retain();
            let _link = link.retain();
            if !self.ivars().invoke() { link.invalidate(); }
        }
    }
);

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
    target: Retained<Target>,
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
        let allocated = Target::alloc(mtm).set_ivars(Callback::new(callback));
        // SAFETY: NSObject's init signature is correct; our ivars were set
        // before initialization and the superclass requires no arguments.
        let target: Retained<Target> = unsafe { msg_send![super(allocated), init] };
        // SAFETY: HasWindowHandle's AppKit contract guarantees a live NSView
        // for this borrow; we are on the main thread. It is used only to create
        // the display link, never stored or messaged after this borrowed call.
        let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        // SAFETY: Target implements frame: with the exact display-link
        // callback signature. The retained link owns its target until invalidated.
        let link = unsafe { view.displayLinkWithTarget_selector(&target, sel!(frame:)) };
        // SAFETY: This immutable Foundation constant is a valid run-loop mode.
        let mode = unsafe { NSRunLoopCommonModes };
        // SAFETY: Both creation and registration occur on the main thread,
        // and callbacks are dispatched on this main run loop (also during tracking).
        unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), mode) };
        Ok(Self { link, target })
    }

    /// Whether the callback panicked. The link is stopped when this happens.
    pub fn has_panicked(&self) -> bool {
        self.target.ivars().panicked.get()
    }
}

impl Drop for DisplaySync {
    fn drop(&mut self) {
        self.target.ivars().stopped.set(true);
        self.link.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    #[test]
    fn stopping_and_panics_prevent_future_callbacks() {
        let calls = Rc::new(Cell::new(0));
        let counter = calls.clone();
        let callback = Callback::new(move || {
            counter.set(counter.get() + 1);
            false
        });
        assert!(!callback.invoke());
        assert!(!callback.invoke());
        assert_eq!(calls.get(), 1);
        let panicking = Callback::new(|| panic!("test callback"));
        assert!(!panicking.invoke());
        assert!(panicking.panicked.get());
        assert!(!panicking.invoke());
    }
    #[test]
    fn reentrant_dispatch_is_skipped_without_stopping_the_driver() {
        let callback = Callback::new(|| true);
        let borrow = callback.call.borrow_mut();
        assert!(callback.invoke());
        assert!(!callback.stopped.get());
        drop(borrow);
        assert!(callback.invoke());
    }

    #[test]
    fn cancellation_from_inside_a_callback_cannot_restart_it() {
        let callback: Rc<Callback> = Rc::new_cyclic(|weak: &std::rc::Weak<Callback>| {
            let weak = weak.clone();
            Callback::new(move || {
                weak.upgrade().unwrap().stopped.set(true);
                true
            })
        });
        assert!(!callback.invoke());
        assert!(!callback.invoke());
    }
}
