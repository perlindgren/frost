# macOS display-synchronized callback boundary

This project-owned wrapper uses public AppKit/objc2 APIs, with no third-party
source changes or dependency version changes. Updated: 2026-11-17.

## Native contracts

`DisplaySync::new` first obtains a main-thread marker and checks macOS 14 API
availability. It accepts a `HasWindowHandle` provider and rejects non-AppKit or
unavailable handles. The provider's raw-window-handle contract guarantees that
the NSView pointer is valid for this borrow. We form a reference only during
construction, on the main thread; no raw view pointer is stored.

The wrapper registers a private NSObject subclass once per linked library. Its
name contains the address of that library's registration cache and a collision
suffix. Existing names are never adopted: different plugins, independently
loaded builds of one plugin, and a later library mapped at an old address must
not accidentally use another image's method implementations. Registration uses
public objc2 runtime APIs. A bounded registration failure returns an error so
the caller can use the existing timer fallback.

Each target has one nullable raw-pointer ivar, declared and accessed as
`*const c_void`. It borrows a Callback allocation owned by a Rust Rc in Target;
it does not own a Rust object and has no Rust native destructor or custom
initialization. Only this private class is instantiated, and no subclasses or
target handles are exposed to application callers. All ivar access is on the
main thread. The main-thread marker check precedes callback-state access.

The `frame:` method has Objective-C's receiver and selector arguments, a void
result, and one CADisplayLink object argument. Native arguments are valid for
the invocation under AppKit's callback contract. The target and link are
explicitly retained during dispatch. A non-null ivar implies its Rust Target
still owns the Rc allocation: main-thread dispatch cannot race Target's drop.
The method increments the strong count and consumes that increment with
Rc::from_raw before invoking user code. An in-flight call therefore keeps its
state alive even if the callback drops its owning driver.

AppKit's view-bound display link follows the view's display and suspends when
the view is hidden or off-screen. It is registered on the main run loop in
common modes, including event tracking. The immutable Foundation mode constant
is the only foreign static read. AppKit owns callback scheduling; no background
thread accesses Rust GUI state.

The driver owns both native objects. On drop it marks the callback stopped,
clears the target's pointer, and invalidates the link before releasing them.
Target's own drop also clears the pointer before releasing the Rc. An extra
native retain or delayed autorelease cannot keep the Rust closure alive or
cause native deallocation to call Rust after a library unload. Both Target and
DisplaySync are !Send and !Sync through their Rc ownership; a compile-fail test
checks that the driver cannot be sent to another thread. Returning false stops
the callback. A nested invocation is
skipped using `try_borrow_mut`, and callback panics are caught before returning
to AppKit, invalidate the link, and remain observable through `has_panicked()`.
Cancellation during a callback cannot be overwritten by its return value.

An outer unwind boundary also covers destruction of captured values after a
driver drops itself. If that destruction panics, the link is invalidated and
the panic payload is intentionally forgotten to prevent a second panicking
payload destructor crossing the Objective-C ABI. This exceptional failure path
can leak one panic payload per stopped driver.

Registered classes remain in the Objective-C runtime. They must never be used
to instantiate new targets after their implementing library unloads. Their
names are private and no safe API exposes that operation. The host must not
unload a library while one of its functions is still on the call stack. This
change handles this wrapper's display callbacks; it does not fix unrelated
plugin background threads or other unload contracts.

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
dispatch, cancellation during dispatch, cached registration, and refusal to
reuse occupied names. The native `lifecycle` example creates and drops 32
view-bound display links on the main thread without a window or audio device.

`cargo test -p macos-frame-sync --test multi_library --locked --offline` runs on
the real OS main thread. It exercises the actual private target implementation:
self-destruction during dispatch, state retention until that dispatch returns,
late dispatch to a retained stopped target, and panic containment. It also
builds two temporary cdylibs statically linking the wrapper, opens multiple
instances from each, checks distinct cached class registrations, reopens an
instance, and verifies prompt callback-state destruction. Both libraries are
closed before the enclosing autorelease pool drains. No dependency source is
modified and no GPU, visible window, audio device or network is used. Platforms
other than macOS print that this native check is macOS-only.

Actual cadence, hidden/reopened editors, moving between monitors, multiple
instances and both plugin formats need testing in a real DAW and visible
standalone window. Source review and tests are not a formal proof or independent
safety audit. No claim of exact 60 FPS follows from compilation or lifecycle tests.
