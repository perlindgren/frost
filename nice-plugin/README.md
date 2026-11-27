# nice-plugin

Project-owned, safe adapters between nice-plug, baseview and Frost. This is shared
application code, not a fork or vendored copy of a dependency. Third-party source
and versions are unchanged.

All ten nice-* effect/instrument editors supply a small `editor::create` factory
and implement `WindowState` for their window preferences. This crate owns the
nice-plug editor/handle adaptation, keyboard and mouse forwarding, renderer setup,
resize, surface/device recovery, and macOS display synchronization. A renderer
rebuild calls the view factory again; DSP and hosted-plugin lifetimes must never
be owned only by that view. Hosted windows never restore or persist standalone
placement, even if a caller has enabled standalone support in the same build.

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
