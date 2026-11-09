//! Main-thread native smoke check; no GPU or application window is needed.
#![deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]
#[cfg(target_os = "macos")]
fn main() {
    use objc2::{MainThreadMarker, rc::Retained, runtime::AnyClass};
    use objc2_app_kit::NSView;
    use objc2_quartz_core::CAMetalLayer;
    use raw_window_handle::{
        AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle,
    };
    struct View(Retained<NSView>);
    impl HasWindowHandle for View {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let raw = RawWindowHandle::AppKit(AppKitWindowHandle::new(
                std::ptr::NonNull::from(&*self.0).cast(),
            ));
            // SAFETY: the returned borrow cannot outlive the retained NSView.
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
    }
    let main = MainThreadMarker::new().expect("Run this check on the main thread");
    for scale in [1.0, 2.0] {
        let view = View(NSView::new(main));
        macos_metal_view::prepare(&view, scale).unwrap();
        let first = view.0.layer().unwrap().downcast::<CAMetalLayer>().unwrap();
        macos_metal_view::prepare(&view, scale + 0.5).unwrap();
        let second = view.0.layer().unwrap().downcast::<CAMetalLayer>().unwrap();
        assert!(
            std::ptr::eq(&*first, &*second),
            "Surface recovery must reuse the layer"
        );
        assert_eq!(second.contentsScale(), scale + 0.5);
    }
    assert!(
        AnyClass::get(c"RawWindowMetalLayer").is_none(),
        "Must not register the conflicting observer class"
    );
    println!("Metal view preparation and reuse passed without registering an observer class");
}
#[cfg(not(target_os = "macos"))]
fn main() {
    println!("macOS-only check");
}
