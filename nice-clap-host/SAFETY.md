# Native plugin loading boundary

`Entry::load` is intentionally unsafe. Clack documents that loading a dynamic
library may run arbitrary code and a nonconforming CLAP entry can invalidate the
assumptions of its safe APIs. We cannot prove those contracts from a pathname,
file extension, metadata, or a successful descriptor query.

The caller must trust the library and all instantiated plugins to implement the
CLAP ABI, lifetimes and thread contracts correctly. The sole unsafe operation in
the library delegates that same obligation to `PluginEntry::load`. No raw pointers
are exposed by our adapter. All other library code denies unsafe code.

`clap-probe` is an in-process diagnostic with an explicit `--trust-plugin` flag.
The flag records the operator's intent, not a safety check. The diagnostic's one
unsafe call is the same boundary; neither this executable nor the safe APIs it
uses protect against a crashing, hanging or malicious plugin. Never use it as an
automatic scanner of unknown plugins. Future untrusted scanning requires an
isolated helper and bounded supervision.

Instantiation, port queries, state I/O, activation and deactivation happen on the
calling main thread. The activated processing handle moves to a scoped worker;
the worker starts, processes and stops, then returns the stopped handle. Only the
main thread deactivates it. Callback requests are serviced on that thread during
the worker operation. Restart requests are surfaced after shutdown; this single
block probe does not claim live restart support.

This prototype allocates offline buffers and state storage and has no native UI
event loop. It does not establish production realtime safety, plugin editor
compatibility, state semantic equivalence, crash isolation or general plugin
correctness. Tests exercise the repository's own plugin before third-party code.

For offline graph rendering, each activated processor has a bounded, one-slot
return channel associated with its main-thread instance. Normal completion stops
processing on the worker and retires the plan on the main thread. Early setup or
preparation errors drop the processor slots, returning their handles; owners then
deactivate on the main thread. Rust panic unwinding is not a native plugin crash
or hang recovery mechanism. There is no automatic restart in this prototype.

LiveSession/LiveAudio use a bounded SPSC handoff of the entire stopped graph.
Native stop_processing runs on the processing thread; deactivation and instance
destruction run on the owner's main thread after receiving the graph. The
per-instance return channels are used only during retirement there, not from
normal live process calls. Complete cooperative stop and service before dropping
either active endpoint. The rack now places this endpoint on a dedicated DSP worker, separate from
device callbacks. That worker keeps polling with no input, so a dead device
cannot prevent the stop handoff. A hung native process call is still not safely
interruptible. LiveAudio Drop also stops/returns its plan during ordinary Rust
unwinding; instance deactivation remains with the main-thread owner.

## Native editors

`LiveSession::open_editor` is a safe public wrapper around CLAP's unsafe
`set_parent`. Its generic parent must satisfy `HasWindowHandle`. The function
borrows that live handle, creates the GUI on the main thread, and holds an Arc
for the entire GUI lifetime. On setup errors it destroys the GUI before the
local Arc drops. On success the owning instance retains the Arc; explicit close
and Owner::drop call destroy before releasing it. The standalone window manager
also closes editors before graph replacement. No raw handle escapes this wrapper.
The plugin must still honor its native-code/CLAP contract.

GUI requests from arbitrary threads use atomic request slots and are fulfilled
on the main thread. Parameter changes cross to process through a bounded SPSC
queue, drained into a preallocated 64-event Vec. Inactive parameters use CLAP's
main-thread flush API instead. No editor, parameter enumeration or state I/O
runs in the audio callback. Editor dimensions are checked before accepting
plugin resize requests. Live CLAP state save uses the specified main-thread
state API; graph replacement still uses the cooperative stop protocol.

Desktop editor creation/placement is not covered by headless tests. The current
standalone window path embeds Cocoa, Win32 or X11 interfaces. Native Wayland
embedding/floating-only editors and plugins requiring additional host extensions
(such as timer or POSIX FD support) need further integration.
