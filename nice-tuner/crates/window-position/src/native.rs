use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::io;
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromPoint};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    SystemParametersInfoW,
};

fn with_hwnd<R>(
    window: &impl HasWindowHandle,
    call: impl FnOnce(HWND) -> io::Result<R>,
) -> io::Result<R> {
    let handle = window
        .window_handle()
        .map_err(|error| io::Error::other(error.to_string()))?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => call(handle.hwnd.get()),
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Expected a Win32 window",
        )),
    }
}

fn rect_of(hwnd: HWND) -> io::Result<RECT> {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: hwnd is borrowed through HasWindowHandle for the enclosing call.
    // rect is initialized, aligned and writable for the synchronous Win32 call;
    // no pointer is retained. A failed or destroyed window returns an error.
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(rect)
}

/// Read the top-left position in Win32 screen coordinates.
pub fn top_left(window: &impl HasWindowHandle) -> io::Result<(i32, i32)> {
    with_hwnd(window, |hwnd| {
        let rect = rect_of(hwnd)?;
        Ok((rect.left, rect.top))
    })
}

/// Restore a saved position if its title bar still falls on a monitor;
/// otherwise center in the primary work area. Does not resize or activate.
///
/// Call on the window's UI thread. The borrowed handle is not stored.
pub fn restore(window: &impl HasWindowHandle, saved: Option<(i32, i32)>) -> io::Result<()> {
    with_hwnd(window, |hwnd| {
        let mut work = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: SPI_GETWORKAREA only writes a RECT into this live initialized
        // and aligned slot. uiParam/fWinIni are zero as specified for this query.
        if unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work as *mut RECT).cast(), 0) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        let rect = rect_of(hwnd)?;
        let window = [rect.left, rect.top, rect.right, rect.bottom];
        let center = super::centered([work.left, work.top, work.right, work.bottom], window)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Invalid window rectangle")
            })?;
        let position = saved
            .filter(|&saved| {
                super::title_probe(saved, window).is_some_and(|(x, y)| {
                    // SAFETY: POINT is passed by value, and MONITOR_DEFAULTTONULL
                    // asks for a null result if no monitor covers the point. No
                    // native pointer is supplied or subsequently dereferenced.
                    unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL) != 0 }
                })
            })
            .unwrap_or(center);
        // SAFETY: hwnd remains borrowed from the caller for this call. Scalar
        // coordinates are checked to fit i32. The flags preserve size, Z-order
        // and activation, so insert-after and dimensions are ignored. This
        // call stores no Rust pointer and runs on the caller's UI thread.
        if unsafe {
            SetWindowPos(
                hwnd,
                0,
                position.0,
                position.1,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })
}
