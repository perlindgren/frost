# AppKit cursor query

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
