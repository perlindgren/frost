# Native cursor queries

## AppKit cursor query

The one unsafe operation borrows `NSView` from a `BorrowedWindowHandle` supplied
by the caller's `HasWindowHandle` implementation. The raw-window-handle contract
requires AppKit's ns_view to be a valid NSView while borrowed. The function first
checks MainThreadMarker, holds the window and borrowed handle throughout, and
never stores/exports the raw pointer or creates a mutable reference. Safe AppKit
methods convert the global cursor position through the native window into view
coordinates, respecting flipped views and logical units. Detached/outside-window
views return None. No dependency source is modified.

The window must satisfy the HasWindowHandle contract (winit's implementation
supplies it). This review establishes our local borrow/call invariants, not a
proof of AppKit or winit internals. Live macOS drag/drop still needs a desktop
check; it cannot be exercised by GPU-less unit tests.

## Win32 cursor query

`hwnd` is read out of the same kind of borrowed handle and used only for four
synchronous user32 calls on the calling thread: `GetCursorPos`, `ScreenToClient`,
`GetClientRect` and `GetDpiForWindow`. Every out parameter is an aligned,
initialized local that the callee fills before returning; the handle and the
pointers are never stored, sent to another thread, or handed back to the caller,
and each call's failure (`FALSE`, or a zero DPI) ends the query with `None`
rather than a guessed point. Converting pixels to the window-centered, y-up
logical space is a pure function taking integers and the DPI, unit-tested at
100 % and 150 % and for the outside-the-client-area and zero-DPI cases. The
window must satisfy the same HasWindowHandle contract; `GetDpiForWindow` matching
winit's scale factor is assumed from winit using the same source of truth, not
proven here. Live Windows drag/drop still needs a desktop check. No dependency
source is modified.
