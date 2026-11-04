# Windows placement boundary

The `unsafe` feature enables the native implementation on Windows only. Without
it, this crate forbids unsafe code and exposes no native placement operations.
nice-tuner forwards its identically named feature, while always denying unsafe
code in the app. Review date: 2026-11-04.

The public methods accept `HasWindowHandle`, obtain a borrowed Win32 handle for
one synchronous call, and never cache a raw HWND or expose integer constructors.
They do not own or destroy the window. The caller should use its window UI thread
to avoid cross-thread window-message blocking. nice-tuner obtains a fresh handle
from baseview on each placement/read operation.

The unsafe operations are limited to:

- `GetWindowRect`: writes into one initialized, aligned, live `RECT`.
- `SystemParametersInfoW`: only `SPI_GETWORKAREA`, with one writable `RECT` and
  the documented zero values for the unused arguments. No settings are changed.
- `MonitorFromPoint`: takes a `POINT` by value. The returned monitor handle is
  only compared with zero, not dereferenced or retained.
- `SetWindowPos`: uses the borrowed HWND and integer coordinates. Flags preserve
  size, activation and Z-order; ignored dimensions and insert-after are zero.
  The operation does not receive or retain a Rust pointer.

Failures become `std::io::Error`. Native invalid-handle behavior is delegated to
Win32. Pointer and struct contracts were checked against Microsoft's API docs:
[GetWindowRect](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getwindowrect),
[SystemParametersInfoW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfow),
[MonitorFromPoint](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-monitorfrompoint),
[SetWindowPos](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos).

Position arithmetic uses `i64` intermediates and checked conversion/addition to
avoid overflow from saved coordinates. Invalid rectangles are rejected. Tests
cover centering on a negative-coordinate monitor, oversized windows, extreme
coordinates and malformed rectangles. Restoring a saved position retains the
existing policy of checking one point in its title bar against live monitors.

The wrapper and complete standalone app cross-compile for x86_64 Windows/MSVC
with placement enabled and disabled. The wrapper's four geometry tests pass on
macOS, and strict Windows-target Clippy checks pass for all wrapper targets.
Visual positioning,
monitor unplugging and DPI behavior still need a real Windows session. Source
review and tests do not constitute a formal proof or independent audit.
