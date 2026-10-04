//! Opening the window and the geometry around it: the winit window
//! itself, the monitor fit that keeps an oversized request on screen,
//! and the letterbox fit the blit pass stretches content with.

use std::sync::Arc;
use wgpu::PresentMode;
use winit::dpi::LogicalSize;
#[cfg(not(target_arch = "wasm32"))]
use winit::dpi::PhysicalPosition;
use winit::dpi::PhysicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

/// The surface presentation mode for the user's vsync request: `AutoVsync`
/// presents once per vertical blank (capped at the display's refresh rate),
/// `AutoNoVsync` presents as soon as frames are rendered. Both `Auto*`
/// modes fall back to a supported mode when their preference is unavailable,
/// so they are safe to request on every platform.
pub(crate) fn present_mode_for(vsync: bool) -> PresentMode {
    if vsync {
        PresentMode::AutoVsync
    } else {
        PresentMode::AutoNoVsync
    }
}

/// Creates the frost window and requests its first frame.
///
/// `window_size` is the `Config`'s initial inner size in logical pixels
/// and `window_size_px` its size in physical pixels; a set physical size
/// wins, and winit converts it through the monitor's scale factor, so the
/// requested panel-pixel window opens at the same physical size on every
/// display — including Retina Macs, where a logical request would double.
/// `None` for both keeps the platform default. An oversized request is
/// clamped into the primary monitor, aspect preserved (see
/// [`fit_into_monitor`]). On the web, winit's canvas
/// is neither appended to the page nor sized by default: without the
/// append it is invisible, and without a size it stays the browser's
/// 300x150 default — so an unset size falls back to 900x600 there, and
/// the physical request is ignored (the canvas lives in CSS pixels).
pub(crate) fn create_window(
    event_loop: &ActiveEventLoop,
    window_size: Option<[u32; 2]>,
    window_size_px: Option<[u32; 2]>,
) -> Arc<Window> {
    let mut attributes = Window::default_attributes().with_title("frost");
    #[cfg(not(target_arch = "wasm32"))]
    {
        let monitor = event_loop.primary_monitor();
        if let Some([width, height]) = window_size_px {
            // Physical request: compare against the monitor in physical
            // pixels, and hand back a clamped physical size.
            let size = match monitor.as_ref() {
                Some(m) => fit_into_monitor([width, height], [m.size().width, m.size().height]),
                None => [width, height],
            };
            attributes = attributes.with_inner_size(PhysicalSize::new(size[0], size[1]));
        } else if let Some([width, height]) = window_size {
            // Logical request: the monitor's own scale factor turns its
            // physical size into the logical budget to fit into.
            let size = match monitor.as_ref() {
                Some(m) => {
                    let s = m.scale_factor().max(0.01);
                    fit_into_monitor(
                        [width, height],
                        [
                            (m.size().width as f64 / s).round().max(1.0) as u32,
                            (m.size().height as f64 / s).round().max(1.0) as u32,
                        ],
                    )
                }
                None => [width, height],
            };
            attributes = attributes.with_inner_size(LogicalSize::new(size[0], size[1]));
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::WindowAttributesExtWebSys;

        let _ = window_size_px;
        let [width, height] = window_size.unwrap_or([900, 600]);
        attributes = attributes
            .with_append(true)
            .with_inner_size(LogicalSize::new(width, height));
    }
    let window = Arc::new(
        event_loop
            .create_window(attributes)
            .expect("failed to create window"),
    );
    // Center the window on its monitor so frost always opens in the middle
    // of the screen. Native only: on the web the page decides where the
    // canvas sits.
    #[cfg(not(target_arch = "wasm32"))]
    center_on_screen(event_loop, &window);
    // winit (Wayland) only delivers RedrawRequested after a compositor
    // frame callback, so explicitly request the first frame; otherwise
    // the window is never mapped and nothing is ever drawn.
    window.request_redraw();
    window
}

/// Centers `window` on the monitor it sits on, so frost always opens in
/// the middle of the screen. Both the monitor's size and the window's
/// outer size (title bar and borders included) are in physical pixels, so
/// the position is computed in physical pixels too — no scale-factor
/// conversion. When the window is larger than the monitor, it is clamped
/// to the monitor's top-left.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn center_on_screen(event_loop: &ActiveEventLoop, window: &Window) {
    let Some(monitor) = window
        .current_monitor()
        .or_else(|| event_loop.primary_monitor())
    else {
        return;
    };
    let screen = monitor.size();
    let frame = window.outer_size();
    let x = ((screen.width as i64 - frame.width as i64) / 2).max(0) as i32;
    let y = ((screen.height as i64 - frame.height as i64) / 2).max(0) as i32;
    window.set_outer_position(PhysicalPosition::new(x, y));
    log::info!(
        "window centered at ({x}, {y}) px on a {}x{} monitor",
        screen.width,
        screen.height
    );
}

/// Fits a requested opening size into the space available on the primary
/// monitor, aspect preserved: a request that fits passes through
/// untouched, an oversized one shrinks to the largest fitting size of its
/// own aspect — a window that opens whole and centered beats one that
/// hangs off the screen, and squashing the shape is not the same as
/// making it smaller. `size` and `avail` are in the same unit (physical
/// or logical, as the caller chose); with no monitor info the request
/// passes through.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn fit_into_monitor(size: [u32; 2], avail: [u32; 2]) -> [u32; 2] {
    let (req_w, req_h) = (size[0] as f64, size[1] as f64);
    let (max_w, max_h) = (avail[0].max(1) as f64, avail[1].max(1) as f64);
    if req_w <= max_w && req_h <= max_h {
        return size;
    }
    let factor = (max_w / req_w).min(max_h / req_h);
    [
        (req_w * factor).round().max(1.0) as u32,
        (req_h * factor).round().max(1.0) as u32,
    ]
}

/// Packs a letterbox fit into the blit uniform's 16 bytes: two floats of
/// extent followed by the struct's padding.
pub(crate) fn fit_bytes(fit: (f32, f32)) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[0..4].copy_from_slice(&fit.0.to_le_bytes());
    bytes[4..8].copy_from_slice(&fit.1.to_le_bytes());
    bytes
}

/// The fraction `(width, height)` of a `surface` that a fixed
/// `render_size` buffer fills when its aspect is preserved and the result
/// is centered: `(1.0, 1.0)` when there is no fixed buffer or the aspects
/// match (the picture covers the window), and a letterbox fraction
/// otherwise — `(1.0, 0.5625)` for a 16:9 buffer on a square surface,
/// bars top and bottom included. The blit pass scales its triangle by
/// exactly this, and the mouse mapping divides it back out.
pub(crate) fn content_fit(render_size: Option<[u32; 2]>, surface: (u32, u32)) -> (f32, f32) {
    let Some([bw, bh]) = render_size else {
        return (1.0, 1.0);
    };
    let (sw, sh) = (surface.0 as f32, surface.1 as f32);
    if sw <= 0.0 || sh <= 0.0 {
        return (1.0, 1.0);
    }
    if (bw as f64) * (sh as f64) > (bh as f64) * (sw as f64) {
        // The buffer is wider than the surface: width-limited, with
        // letterbox bars above and below.
        (
            1.0,
            (bh as f64 * sw as f64 / (bw as f64 * sh as f64)) as f32,
        )
    } else {
        // Taller (or exactly as wide): pillarbox bars left and right.
        (
            (bw as f64 * sh as f64 / (bh as f64 * sw as f64)) as f32,
            1.0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_monitor_fit_leaves_sizes_that_fit_untouched() {
        assert_eq!(fit_into_monitor([1280, 720], [1512, 982]), [1280, 720]);
        // Exactly the budget is not an overflow.
        assert_eq!(fit_into_monitor([1512, 982], [1512, 982]), [1512, 982]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_monitor_fit_shrinks_an_oversized_request_aspect_preserved() {
        // 1920x1080 on a 1512-wide MacBook: the width binds, and the
        // height falls by the same factor — the 16:9 shape survives.
        let [w, h] = fit_into_monitor([1920, 1080], [1512, 982]);
        assert_eq!(w, 1512);
        assert!((w as f64 / h as f64 - 16.0 / 9.0).abs() < 0.01);
        // A tall request on a wide monitor is height-bound instead.
        let [w, h] = fit_into_monitor([1080, 1920], [1512, 982]);
        assert_eq!(h, 982);
        assert!((w as f64 / h as f64 - 1080.0 / 1920.0).abs() < 0.01);
    }

    #[test]
    fn a_matching_aspect_fills_the_surface() {
        assert_eq!(content_fit(None, (100, 100)), (1.0, 1.0));
        assert_eq!(content_fit(Some([1920, 1080]), (1920, 1080)), (1.0, 1.0));
        // The same shape at a different size fills too: 800x450 is 16:9.
        let (fx, fy) = content_fit(Some([1920, 1080]), (800, 450));
        assert!((fx - 1.0).abs() < 1e-4 && (fy - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_wider_surface_than_buffer_pillarboxes_horizontally() {
        // A square window with a 16:9 buffer: the buffer is wider, so it
        // spans the full width and shrinks in height.
        let (fx, fy) = content_fit(Some([1920, 1080]), (1000, 1000));
        assert_eq!(fx, 1.0);
        assert!((fy - 1000.0 * 1080.0 / (1920.0 * 1000.0)).abs() < 1e-4);
        assert!(fy > 0.55 && fy < 0.57);
    }

    #[test]
    fn a_taller_surface_than_buffer_letterboxes_vertically() {
        // A square window with a 1:2 (portrait) buffer: the buffer is
        // taller, so it spans the full height and shrinks in width.
        let (fx, fy) = content_fit(Some([540, 1080]), (1000, 1000));
        assert_eq!(fy, 1.0);
        assert!((fx - 1000.0 * 540.0 / (1080.0 * 1000.0)).abs() < 1e-4);
        assert!((fx - 0.5).abs() < 1e-4);
    }

    #[test]
    fn a_zero_sized_surface_falls_back_to_filling() {
        // A zero surface would divide by zero; the guard returns (1, 1).
        assert_eq!(content_fit(Some([1920, 1080]), (0, 0)), (1.0, 1.0));
    }

    #[test]
    fn fit_bytes_writes_the_two_extents_and_pads() {
        let bytes = fit_bytes((0.5, 0.25));
        assert_eq!(f32::from_le_bytes(bytes[0..4].try_into().unwrap()), 0.5);
        assert_eq!(f32::from_le_bytes(bytes[4..8].try_into().unwrap()), 0.25);
        assert_eq!(&bytes[8..], &[0u8; 8]);
    }
}
