# nice-plugin

Project-owned, safe adapters between nice-plug, baseview and Frost. This is shared
application code, not a fork or vendored copy of a dependency. Third-party source
is never modified. The approved migration pins Baseview to 0.4.2.

All ten nice-* effect/instrument editors supply a small `editor::create` factory
and implement `WindowState` for their window preferences. This crate owns the
nice-plug editor/handle adaptation, keyboard and mouse forwarding, renderer setup,
resize and surface/device recovery. Baseview owns frame scheduling. A renderer
rebuild calls the view factory again; DSP and hosted-plugin lifetimes must never
be owned only by that view. Hosted windows never restore or persist standalone
placement, even if a caller has enabled standalone support in the same build.

Editors use Baseview 0.4's `draw` and `request_redraw` API. Each successful
frame requests one following animation frame, so meters and waveforms update
without parameter notifications. Input and host state/parameter notifications
also request a redraw through the window's public waker. Baseview coalesces
requests. Nice Rack may supplement paint delivery on Windows using public
Win32 paint operations; it does not send private Baseview timer messages.
Windows editors still render on the host UI thread; this migration does not
introduce renderer workers or remove possible GPU-acquisition waits.

Every editor configures its own GPU surface; the rack's `frame_latency: 1` does
not propagate into plugin windows. Editor surface buffering is currently
unchanged. The proposed latency-1 versus latency-2 comparison is recorded in
[the migration notes](../docs/rack-baseview-migration.md#presentation-latency-current-policy-and-proposed-experiment).

On macOS, Baseview now owns the view-bound display link. The former parallel
`macos-frame-sync` callback is no longer installed by this adapter; that helper
crate remains in the workspace for its existing examples/tests. Baseview 0.4
requires macOS 14 for this path. The adapter checks availability before creating
a view, and release downloads declare that minimum. Metal-layer preparation
remains in our safe `macos-metal-view` wrapper.

The optional `standalone` feature adds one Apply/rollback owner and transport pump
for Clone, Dist, Echo, Ecomplex, Eq, Phase, Pi Face and Wah. Mono layouts fold transport
input and duplicate the mono output; Pi Face's second, stereo layout preserves both
channels. Buffer bounds and plugin-declared layouts remain explicit. Tuner's test
tone/analysis input and NAM's model/device path retain their specialized adapters;
all ten use the same no-MIDI processing context and successful activation always
calls `reset()` before processing starts, matching the hosted lifecycle.

The `unsafe` feature only enables the existing project-owned Windows placement
wrapper. Each editor also opts into placement at runtime, and parented windows
are excluded. This crate itself denies unsafe code. The existing macOS wrappers
remain responsible for their native API boundaries.

Future instrumentation can measure shared editor frames at `window::FrameDriver`
and standalone blocks at `standalone::process_once`. CLAP-host call boundaries
still belong to `nice-clap-host`; share measurement/reporting types rather than
making the plugin editor own the rack's audio graph or devices.

The startup wait, driver calls, and worker joins retain their previous blocking
behavior. This refactoring does not claim to fix arbitrary driver/plugin hangs.

Tests exercise input forwarding without GPU resources, activation/reset ordering,
and existing per-plugin audio, metering, latency, and allocation regressions
through the shared production adapters. Native editor appearance still needs a
windowing session with a GPU.
