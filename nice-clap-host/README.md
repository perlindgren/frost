# nice-clap-host

Project-owned, small CLAP hosting adapter for the nice-rack prototype, using the
published Clack 0.2.0 crates. Existing dependency sources and versions are unchanged.

The `clap-probe` diagnostic enumerates an explicitly selected entry. Supplying a
plugin ID also queries mono/stereo main buses, saves and restores optional plugin
state, activates on the main thread, processes one 256-frame offline block on a
worker while servicing main-thread callbacks, then stops and deactivates. Restart
requests are reported after shutdown. It neither scans installed plugins nor
opens audio devices or plugin editors.

Loading native code is an explicit unsafe trust boundary, not a guarantee that a
plugin is safe. `--trust-plugin` is an operator precondition, not validation. Read
[SAFETY.md](SAFETY.md) before using this diagnostic. No crash/hang isolation or
realtime guarantee is provided. State is held in memory; no user files are changed.

To test with this repository's own plugin on macOS:

```sh
cargo build -p nice-tuner --lib
cargo run -p nice-clap-host --bin clap-probe -- \
  --trust-plugin target/debug/libnice_tuner.dylib com.nice-tuner.nice-tuner
NICE_RACK_TEST_CLAP="$PWD/target/debug/libnice_tuner.dylib" \
  cargo test -p nice-clap-host --test tuner_roundtrip -- --ignored
```

On Linux/Windows, use the native library or CLAP entry path produced by that
platform's build. Only the macOS path has been exercised so far.

The `nice-rack` application and routing engine remain safe Rust. The loader's
unsafe precondition is intentionally not hidden behind a safe arbitrary-path API;
an isolated loading boundary is a separate design step before untrusted scanning.

## Offline rack bridge

`Entry::render_graph` connects native CLAP processors to nice-rack-engine. Each
plugin node gets a distinct instance; this first bridge uses class IDs from one
loaded entry and each instance's default state. Serial and parallel mono/stereo
graphs are supported. It processes variable blocks up to the prepared maximum,
including a shorter final block, and returns planar audio, graph latency and
per-node saved-state byte counts. Rack-document state restoration is not yet
implemented.

Instances and lifecycle/state calls stay on the caller's main thread. The worker
receives processing handles and preallocated native port wrappers, processes the
graph, then stops the processors. The stopped plan returns to the main thread;
retirement sends the handles back to their owners for deactivation. Partial setup
and graph-preparation failures also return activated handles. Restart requests or
latency changes stop this offline operation rather than continuing with a stale
latency plan. Native errors are formatted after the worker finishes.

The bounded one-shot retirement channels are never used during normal process
calls. The rendering function is an offline diagnostic: it creates a worker,
allocates complete output storage, and polls main-thread callbacks. It is not an
audio callback API, a native GUI event loop, a live graph swap mechanism or a
crash/hang boundary. No realtime timing guarantee is claimed for hosted plugins.

The opt-in tuner integration suite now also covers two instances in series and
parallel, 513-frame input rendered in blocks of at most 128, and setup/preparation
failure followed by a successful render.

## Callback-side lifecycle

`Entry::prepare_live` creates a main-thread `LiveSession` and a sendable
`LiveAudio`. Move only LiveAudio to the audio thread, call process for each block,
and keep calling session.service on the main thread for native callbacks.
Restart/latency-change requests and processor errors initiate a stop. The audio
side stops processors at the next block boundary and returns the whole prepared
plan through a one-slot rtrb queue. It outputs silence after handoff. The main
thread then retires buffers and deactivates plugin instances.

After service reports Stopped, reconfigure snapshots plugins that support state,
prepares replacement instances with that state and a new rate/block size, and
returns a fresh LiveAudio. Failed replacement leaves the old stopped session
intact, so it can be retried or rebuilt at the previous configuration. Plugins
without a state extension currently return to defaults on replacement.

This is the lifecycle foundation for device Apply, not device I/O itself. The
caller must complete cooperative stop before dropping an active audio endpoint
or its owner. A dead device cannot drive that handshake by itself. The rack therefore uses
an independent DSP worker (described below), not direct device-callback DSP.
This adapter does not itself open device streams or isolate untrusted plugins.

Tests include stop/handoff/silence, main-thread retirement, rate/block-size
reconfiguration and retry after invalid configuration with nice-tuner. A safe
allocation-counter witness verifies that our callback processing/handoff path
allocates and frees no memory and defers graph destruction to the receiving
thread; it does not prove arbitrary native plugins allocate nothing.

The standalone rack now uses the endpoint on a dedicated DSP worker, independent
of CPAL callbacks. This provides the missing device-quiescence path: stop does
not require more capture data. The owner services the returned plan and native
handles on the main thread before reopening streams. `Binding` and
`LiveSession::prepare` support classes from multiple loaded entries, retaining
entry lifetimes across card edits; `Entry::prepare_live` remains a single-entry
convenience. See nice-rack/README.md for the current GUI and limits.
