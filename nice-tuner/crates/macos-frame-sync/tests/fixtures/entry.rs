//! The same wrapper statically linked into two independently loaded libraries.
#![deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]
use macos_frame_sync::DisplaySync;
use objc2::{MainThreadMarker, rc::Retained};
use objc2_app_kit::NSView;
use raw_window_handle::{
    AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle,
};
use std::{
    ffi::c_void,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

static DROPPED: AtomicUsize = AtomicUsize::new(0);
struct Witness;
impl Drop for Witness {
    fn drop(&mut self) {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}
struct View(Retained<NSView>);
impl HasWindowHandle for View {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::AppKit(AppKitWindowHandle::new(NonNull::from(&*self.0).cast()));
        // SAFETY: the borrow cannot outlive the retained, main-thread NSView.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}
struct Instance {
    _sync: DisplaySync,
    _view: View,
}
#[unsafe(no_mangle)]
pub extern "C" fn review_create() -> *mut c_void {
    let result = std::panic::catch_unwind(|| {
        let main = MainThreadMarker::new().ok_or("not the main thread")?;
        let view = View(NSView::new(main));
        let witness = Witness;
        let sync = DisplaySync::new(&view, move || {
            std::hint::black_box(&witness);
            true
        })?;
        Ok::<_, String>(Box::new(Instance {
            _sync: sync,
            _view: view,
        }))
    });
    match result {
        Ok(Ok(instance)) => Box::into_raw(instance).cast(),
        Ok(Err(error)) => {
            eprintln!("display callback fixture: {error}");
            std::ptr::null_mut()
        }
        Err(_) => {
            eprintln!("display callback fixture panicked");
            std::ptr::null_mut()
        }
    }
}
#[unsafe(no_mangle)]
/// # Safety
/// Return only a live allocation from review_create, exactly once, on the main
/// thread while this library remains loaded.
pub unsafe extern "C" fn review_destroy(instance: *mut c_void) {
    // SAFETY: the test runner returns each live allocation exactly once, on the
    // same main thread, before unloading this library.
    drop(unsafe { Box::from_raw(instance.cast::<Instance>()) });
}
#[unsafe(no_mangle)]
pub extern "C" fn review_dropped() -> usize {
    DROPPED.load(Ordering::Relaxed)
}
