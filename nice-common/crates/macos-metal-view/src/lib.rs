//! Prepare a plugin-owned macOS view for Metal without runtime-defined classes.
#![deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]

/// Install a system CAMetalLayer as the root layer of a plugin-owned view.
///
/// Call on the main thread before creating a wgpu surface and when resizing or
/// changing scale. The view must be the caller's rendering view: this replaces
/// its root drawing layer. Existing CAMetalLayers are reused. AppKit retains the
/// layer, and wgpu retains it separately when creating its surface.
///
/// Providing the layer up front makes wgpu/raw-window-metal reuse it instead of
/// registering an observer subclass with a process-global Objective-C name.
#[cfg(target_os = "macos")]
pub fn prepare(window: &impl raw_window_handle::HasWindowHandle, scale: f64) -> Result<(), String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSView;
    use objc2_quartz_core::CAMetalLayer;
    use raw_window_handle::RawWindowHandle;
    let _main = MainThreadMarker::new().ok_or("Metal view preparation requires the main thread")?;
    if !scale.is_finite() || scale <= 0.0 {
        return Err("Invalid Metal view scale".into());
    }
    let borrowed = window.window_handle().map_err(|e| e.to_string())?;
    let RawWindowHandle::AppKit(handle) = borrowed.as_raw() else {
        return Err("Expected an AppKit view".into());
    };
    // SAFETY: BorrowedWindowHandle guarantees ns_view is a live NSView for this
    // borrow. Access is on the main thread, no pointer/reference escapes, and
    // neither the caller nor its window is released during the native calls.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    view.setWantsLayer(true);
    let layer = view
        .layer()
        .and_then(|l| l.downcast::<CAMetalLayer>().ok())
        .unwrap_or_else(CAMetalLayer::new);
    layer.setFrame(view.bounds());
    layer.setContentsScale(scale);
    view.setLayer(Some(&layer));
    // Explicitly keep the retained local alive through setLayer's retain.
    drop(layer);
    Ok(())
}
