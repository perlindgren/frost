# macOS display-synchronized callback boundary

This project-owned wrapper uses public AppKit/objc2 APIs, with no third-party
source changes or dependency version changes. Review date: 2026-11-05.

## Native contracts

`DisplaySync::new` first obtains a main-thread marker and checks macOS 14 API
availability. It accepts a `HasWindowHandle` provider and rejects non-AppKit or
unavailable handles. The provider's raw-window-handle contract guarantees that
the NSView pointer is valid for this borrow. We form a reference only during
construction, on the main thread; no raw view pointer is stored.

The wrapper creates an NSObject subclass with Rust-owned callback state and no
custom native destructor. Its `frame:` method has the signature required by
CADisplayLink (void result, one display-link object argument). The target and
display link are explicitly retained during each invocation, so a callback may
drop its owning driver without freeing either native receiver mid-call.

AppKit's view-bound display link follows the view's display and suspends when
the view is hidden or off-screen. It is registered on the main run loop in
common modes, including event tracking. The immutable Foundation mode constant
is the only foreign static read. AppKit owns callback scheduling; no background
thread accesses Rust GUI state.

The driver owns both native objects. On drop it marks the callback stopped and
invalidates the link before releasing them. Both Target and DisplaySync are
main-thread-only; a compile-fail test checks that the driver cannot be sent to
another thread. Returning false stops the callback. A nested invocation is
skipped using `try_borrow_mut`, and callback panics are caught before returning
to AppKit, invalidate the link, and remain observable through `has_panicked()`.
Cancellation during a callback cannot be overwritten by its return value.

## Application lifetime and fallback

The app callback holds only a weak renderer reference and a shared error slot.
It cannot retain the editor in a cycle. Both WillClose and editor destruction
drop the driver. Rendering errors are delivered to baseview's normal error path
by its timer; it otherwise performs no rendering while display synchronization
is active. Older macOS systems or creation failures retain the original timer
path. Other platforms retain their existing behavior.

The same renderer performs frame submission, resizing, surface repair and device
recovery for both schedulers. Its frame method skips reentrant access while an
event is borrowing the renderer. Audio processing and its locks are unchanged.
Presentation remains AutoNoVsync: display synchronization schedules submissions
and does not introduce an intentional refresh wait in the host's UI thread.
This does not guarantee that graphics-driver calls can never stall.

## Verification

Unit tests cover stop/continue behavior, callback panic containment, nested
dispatch and cancellation during dispatch. The native `lifecycle` example creates
and drops 32 view-bound display links on the main thread without opening a
window or audio device. It passed here, but does not test callback cadence.

Actual cadence, hidden/reopened editors, moving between monitors, multiple
instances and both plugin formats need testing in a real DAW and visible
standalone window. Source review and tests are not a formal proof or independent
safety audit. No claim of exact 60 FPS follows from compilation or lifecycle tests.
