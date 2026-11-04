//! Native smoke check: create/drop display callbacks on an unattached view.
//! No window or audio stream is opened. This does not measure frame pacing.
#[cfg(target_os = "macos")]
fn main() {
    use objc2::rc::Retained;
    use objc2_app_kit::NSView;
    use objc2_foundation::MainThreadMarker;
    use raw_window_handle::{
        AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle,
    };
    use std::ptr::NonNull;
    struct View(Retained<NSView>);
    impl HasWindowHandle for View {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let raw =
                RawWindowHandle::AppKit(AppKitWindowHandle::new(NonNull::from(&*self.0).cast()));
            // SAFETY: This borrow cannot outlive View, which retains this live
            // NSView. View and the caller stay on the main thread.
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
    }
    let mtm = MainThreadMarker::new().expect("main thread");
    let view = View(NSView::new(mtm));
    for _ in 0..32 {
        let sync =
            macos_frame_sync::DisplaySync::new(&view, || true).expect("macOS 14+ display callback");
        assert!(!sync.has_panicked());
        drop(sync);
    }
    println!(
        "32 native display-callback create/drop cycles passed; no visible-window cadence test performed"
    );
}
#[cfg(not(target_os = "macos"))]
fn main() {
    println!("macOS-only lifecycle check");
}
