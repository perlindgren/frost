# CLAP hosting review

CLAP is the first format, as selected by the user. The Clack prototype now uses the published host and extension crates. This is an API/documentation review, not a safety audit or a measured
compatibility result.

## Recommended direction

Use published `clack-host` 0.2.0 and `clack-extensions` 0.2.0 behind a small
project-owned adapter. Keep rack routing, audio device ownership and frost UI
outside that adapter. Verify dependency resolution without updating any existing
third-party versions before integrating.

Clack is a low-level host wrapper rather than an audio engine. Its project is
MIT OR Apache-2.0 licensed. Its extension crate provides audio ports, state,
parameters, latency, GUI, thread checks, timers and logging. Enable only the
features required by each implementation step.

The lifecycle API separates the main-thread plugin instance from a sendable audio
processing handle. Main-thread activation allocates resources; start/process/stop
run on the audio thread. Return the stopped handle to the main thread before
deactivation and instance destruction. Restart, callback and process requests
must be handled, not implemented as empty stubs.

This corrects the initial architecture sketch: plugin activation cannot simply
run on an arbitrary preparation worker. Graph buffer preparation can run away
from the audio callback, but plugin lifecycle calls retain their main-thread
requirements.

## Alternatives

`plughost-formats` 0.2.0 provides a higher-level common interface for CLAP, VST3
and Audio Units; its CLAP implementation uses Clack. It is worth considering if
unified format handling becomes the priority. It also makes more lifecycle,
processing and controller policy choices than the thin adapter proposed here.
No license/dependency audit or source audit of that alternative has been done.

Calling `clap-sys` directly would leave substantially more FFI and lifecycle
invariants in our own code. Prefer the wrapper's public APIs unless concrete
limitations justify a different choice; never modify dependency source.

## Safety and platform validation

Clack's safe API does not make an arbitrary dynamically loaded plugin trustworthy.
Review the exact published entry-loading contract before exposing loading through
our adapter; retain any unavoidable unsafe boundary in that project-owned crate.
Do not claim the adapter has been verified before it exists.

Validate on macOS first, then Linux and Windows. Cross-platform API support is
not evidence that native editors, bundle loading or event-loop integration have
been tested on those systems. Keep plugin scanning in a separate process in a
later step; initial in-process DSP will not provide crash isolation.

## Prototype implementation slice

1. Enumerate descriptors from an explicitly selected CLAP entry.
2. Instantiate one mono/stereo effect, query its ports, and process test audio.
3. Verify stop/deactivate ordering, callbacks and state round-trips.
4. Use nice-tuner/nice-nam exports as integration targets before third-party plugins.
5. Add graph processing, devices and native editors in separate steps.

No audio streams, native editor windows or automatic system-wide plugin scan are
needed for the first host test.

## Primary sources

- https://github.com/prokopyl/clack — project goals, status and license.
- https://docs.rs/crate/clack-host/0.2.0/source/Cargo.toml.orig — published manifest.
- https://docs.rs/clack-host/latest/clack_host/ — lifecycle and processing example.
- https://docs.rs/clack-host/latest/clack_host/host/ — host callbacks/thread model.
- https://docs.rs/clack-extensions/latest/clack_extensions/ — extensions/features.
- https://docs.rs/plughost-formats/latest/plughost_formats/ — higher-level alternative.

## Multi-format investigation outcome

The published `plughost-formats` 0.2.0 and `plughost` 0.2.1 manifests both declare
MIT OR Apache-2.0 licensing. `plughost-formats` provides the in-process unified
format API and uses Clack for CLAP. `plughost` provides a separate helper-process
API, with serial chains, scanning, recovery and a shared-memory realtime endpoint.
These are alternatives at different layers, not interchangeable APIs.

The upstream repository currently documents Apple Silicon macOS 15+ and Windows
x64 support, explicitly excluding Linux, Intel Macs and Rosetta. This is a
substantial limitation for a rack intended to support Linux. Its realtime helper
adds a configurable quantum delay; this is in addition to native plugin latency.
It does not own device I/O or provide a resampler, so nice-common would still be
needed. Parallel branches would still require our own graph scheduling and
alignment, rather than assuming the serial Chain API implements the rack graph.

Manifest inspection found `libloading ^0.9.0` and `plughost-core =0.2.0` for the
in-process layer; the 0.2.1 isolated layer uses its matching core plus IPC, JSON
protocol/configuration and platform libraries. Neither alternative was added to
the workspace or compile-tested. No claims about its native editor compatibility
or performance have been independently verified.

Recommendation: continue with the Clack adapter for CLAP, while keeping an
application-owned format-neutral processor boundary for later VST3 support.
Reconsider plughost if its platform coverage changes or the project deliberately
chooses macOS/Windows-only support.

Source: https://github.com/emulette/plughost (platform and realtime sections);
published crate metadata/dependency lists from crates.io for the versions above.

## Measured prototype result

On the development Mac, `clap-probe` loaded a freshly built nice-tuner export,
enumerated its descriptor, queried one stereo main bus in each direction, and
processed 256 frames at 48 kHz on a worker. A constant 0.125 input emerged at 0.125;
the plugin reported zero latency and supported state save/restore. This does not
validate native editor windows, general third-party plugins or realtime timing.

Only three third-party packages were added: clack-common, clack-host and
clack-extensions, all 0.2.0. Existing locked dependency versions were unchanged.
The explicit native-code trust boundary is documented in nice-clap-host/SAFETY.md.
