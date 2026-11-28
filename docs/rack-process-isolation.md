# Rack process isolation

Implementation branch: `codex/rack-process-isolation`, based on the current
Baseview migration. The separate `win-work` performance work is not implicitly
merged. In-process hosting remains available as a comparison and fallback.

**Current measured status:** baseline is the demonstrated general 64-frame path.
The full-isolation prototype failed 64-frame macOS Terminal trials. GUI-only
isolation is a benchmark using independent inactive editor replicas, not a
transparent third-party hosting mode. See the
[macOS threading findings](macos-plugin-threading-findings.md) for the successful
Terminal results, the threading contracts, and the limits of these conclusions.

## Plan and invariants

1. A persistent helper process owns each isolated CLAP instance from creation
   through destruction. Native GUI operations and CLAP main-thread callbacks run
   on that helper's main thread. Each helper owns an independent audio worker.
2. Only audio samples and a small versioned block header occupy shared memory.
   Control messages (metadata, state, parameters, editor geometry and lifecycle)
   use a separate bounded, length-prefixed RON channel over loopback sockets.
   Plugin stdout is never used as a protocol channel.
3. Main-process audio code performs no process creation, file operations or
   control RPC. Shared sample cells are atomic f32 bit patterns: a late or failed
   helper cannot introduce a Rust data race. Preallocated local slices isolate
   the foreign plugin's pointer-based audio API from the shared control header.
   This deliberately retains bounded buffer copies; it is not a zero-copy claim.
4. All helpers are dispatched for ready, independent graph work before waiting
   for completion. Dependencies determine ordering; a serial chain remains serial.
   No per-block thread/process creation occurs. The audio-block deadline bounds
   helper completion; missing it produces silence and a visible failure rather
   than an unbounded wait or reuse of stale output.
5. Process supervision, termination and reaping run outside the audio path.
   Steady-state GUI/control commands never wait for a foreign callback on the
   rack's event loop. Initial inspection/activation still uses a synchronous
   startup handshake with timeouts.
   Recovery uses the last confirmed plugin state; a helper that fails is never
   silently treated as having successfully saved new state.
6. Helper GUI windows are top-level windows owned by their helper, avoiding
   cross-process child-window coupling. Linux helpers use X11/XWayland for CLAP
   embedding. The rack retains window geometry and confirmed plugin state.
7. Shared Rust scheduling and transport code have small platform boundaries.
   Workers currently use ordinary OS thread scheduling. Native audio-priority
   policy is a follow-up informed by platform measurements; no real-time
   scheduling guarantee is claimed. Process isolation is crash containment, not a
   security sandbox, nor isolation from GPU/driver-wide failures.

## Current implementation

Use `cargo run -p nice-rack-standalone --release -- --isolate`. Without the flag,
existing in-process hosting and its original execution order remain in use.
The isolation choice is a launch option, not a settings-file migration.

`nice-clap-host::isolated` owns the RON protocol, process supervision, control
workers, and helper entry point. `nice-rack/crates/ipc` owns the mmap/atomic unsafe
boundary; its mapping constructors explicitly require the caller to uphold file
lifetime and atomic-access invariants. The application library still denies
unsafe code. No third-party source or existing dependency versions changed.

Every helper uses its initial thread for CLAP lifecycle/state/GUI work, with
window-system initialization deferred until an editor is requested. One persistent
audio worker owns its CLAP processing handle. Shared sample cells carry f32 bits
atomically; local preallocated sample/slice buffers are passed to CLAP. A parent
bridge thread performs blocking socket wake/ack I/O; the rack DSP thread publishes
memory and unparks that bridge, then uses a deadline-bounded spin/yield wait.
There is no added block of pipeline buffering. This does add scheduling and copy
cost; shared memory is not being described as zero-copy.

The prepared graph compiles dependency levels only when a deferred processor is
present. It submits all nodes in a level before completing that level, preserving
serial dependencies and existing latency compensation, bypass, pause, wire trims,
and meters. Level barriers are conservative: a ready descendant can still wait
for an unrelated slower node in the preceding level. There are no per-block
allocations or new worker threads. Every helper sees one shared absolute deadline
for the block. Per-card timings include transport and scheduling waits; their sum
is not CPU utilization when branches overlap. Native processor-only measurement
is not added here.

A missed deadline poisons the endpoint permanently; late output cannot revive it.
The external block is silent and normal rack recovery rebuilds the failed session.
This first implementation restarts the rack graph, not just the failed branch.
Confirmed state is cached asynchronously on change. A full capture before graph
retirement uses request IDs and a parameter-flush fence on the helper audio thread,
polled by the rack across frames with a two-second limit. Timeout preserves the
last confirmed bytes and reports a diagnostic. Native state callbacks never run
on the rack's GUI thread in isolated mode.

Editors retain cached geometry. Closing an editor does not terminate its audio
helper. Geometry notifications coalesce so dragging a window cannot fill the
control channel. A shutdown reaper gives each helper two seconds, then kills and
reaps it; the executable drains reapers after its event loop returns, with a shared
three-second exit limit. Parent control-channel loss starts an independent helper
watchdog, which also ends a helper stuck in a foreign GUI callback. A process
externally suspended by a debugger cannot run that watchdog until resumed.

## Validation and platform testing

Unit tests cover framed protocol limits, independent shared mappings and layout
validation, deferred sibling dispatch before waiting, shared deadlines, silence
on failure, existing metering/bypass/pause behavior, and allocation counts.
Explicit native tests use the actual helper executable and a built mono nice-echo
CLAP export. They cover audio delivery, a parameter edit immediately before stop,
acknowledged state capture, helper exit, suspended-helper timeout, and rejection
of late output after timeout. These tests do not open editor windows.

Example from the repository root (use the paths produced by your build):

```sh
cargo build -p nice-rack-standalone
export NICE_RACK_TEST_HELPER="$PWD/target/debug/nice-rack"
export NICE_RACK_TEST_CLAP="/absolute/path/to/nice_echo.clap"
cargo test -p nice-clap-host --test isolation -- --ignored
cargo test -p nice-clap-host --lib isolated::client::tests -- --ignored
```

The suspended-helper test is Unix-only. On Windows use `nice-rack.exe` and set the
environment variables using the shell's syntax. Routine unit tests need neither
native plugins nor loopback ports; the native smoke tests require local socket
permission. All traffic stays on `127.0.0.1`.

Manual testing remains required on Windows, Linux and macOS: compare default and
`--isolate` modes with several open animated editors, move/resize/minimize editor
windows, edit/save/reload state and NAM assets, edit parallel/serial routing, apply
audio settings, and close/reopen the rack. Measure 32/64/128-frame deadline misses
separately from GUI frame rate. Ordinary OS scheduling and the bridge hop may make
small blocks impractical under load; no cross-platform performance result is
claimed before those measurements. Linux editors require X11 or XWayland.

### Local verification (2026-11-26)

On macOS/aarch64: release standalone build passed; 176 routine unit tests passed
(13 unrelated/native/manual tests remained ignored); the three explicit isolation
smoke/failure tests passed using the release helper and a built nice-echo export.
Strict Clippy checks passed for the changed host, engine, IPC and standalone
packages. Cross-checks passed for the complete Windows standalone and for the
Linux CLAP host/IPC/helper code. These are compilation checks, not native Windows
or Linux runs. No multi-editor GUI frame-rate or small-buffer performance result
had been measured at that stage; the later macOS Terminal findings are linked
above.

For automated paired performance trials, including optional native editor
windows, see [the isolation benchmark](rack-isolation-benchmark.md).
