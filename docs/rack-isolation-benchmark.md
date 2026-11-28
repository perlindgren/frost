# Comparing isolated and in-process hosting

The `isolation-bench` example automates paired trials of the production
`LiveSession`/`LiveAudio` adapters. It opens no audio devices, changes no settings,
and needs no new dependencies. Run it in release mode with built CLAP fixtures.
It accepts macOS bundles and native library paths, Windows CLAP files, and Linux
CLAP files. The benchmark executable also supplies its own isolated helper mode;
you do not need a separately installed rack/helper executable.

For the later successful macOS Terminal run and architectural conclusions, see
[macOS plugin threading findings](macos-plugin-threading-findings.md). Those
results supersede the earlier restricted-environment runs as the desktop
reference, while retaining the distinction between real instances and replicas.

## DSP comparison

From the repository root, using a plugin you built for this machine:

```sh
cargo run -p nice-clap-host --example isolation-bench --release -- \
  --plugin "target/bundled/Nice Echo.clap" --copies 4 \
  --frames 32,64,128 --seconds 3 --warmup 1 --trials 3 \
  --out target/isolation-dsp.ron
```

Adjust the plugin path to your build output. Repeat `--plugin PATH` to specify
an ordered list, such as tuner, distortion and echo; `--copies` repeats that list.
The first selected class must be the same in both modes: class IDs are recorded
and mismatches invalidate paired ratios. Plugins start with default state. This
is **not** a loaded-model NAM benchmark unless a fixture loads that model itself.

Both serial and parallel workloads run by default. `--topology serial` or
`--topology parallel` selects just one. Parallel means one plugin per lane, with
unity summation, not a parallelized version of the serial signal path. Explicit
channel maps connect stereo test input/output to each plugin's declared widths.
Compare modes within the same topology; signal levels and data-dependent DSP
work can differ between topologies.

Each trial starts a fresh case process, including fresh native instances and
isolated helpers. Trial order alternates native/isolated and isolated/native.
It does not flush OS/driver caches. The workload is a 110 Hz sine at amplitude
0.1, identical on both external input channels, with a persistent DSP worker
paced at `frames / sample_rate` (48 kHz unless `--rate` specifies otherwise).
The main thread continues servicing CLAP callbacks while DSP runs.
Host wire meters and per-node processing-time observations are enabled in both
modes, as they are in the rack.

The RON report stores every case and paired ratios:

- successful-call count, median, p95, p99 and maximum processing time;
- attempted measured blocks, warmup blocks, warmup and measured deadline misses;
- failed-call duration and whether failure occurred during warmup;
- DSP-worker wake lateness, separately from time inside `LiveAudio::process`;
- setup and editor-open acknowledgement time;
- host heartbeat lateness/gaps and missed heartbeat intervals;
- platform, architecture, machine label (`--machine`), rustc version, captured
  compile flags, plugin paths, class IDs, workload, topology, and trial number.

Percentiles use nearest rank. No percentile is fabricated for an empty run.
The console's worst-call time includes a failed call; the report keeps that
separate from successful-call percentiles. Warmup samples are excluded from the
measured distribution, but warmup failures and deadline misses remain visible.
The measurement includes production inter-process transport and scheduling waits,
not just plugin computation, and is wall-clock time, **not CPU utilization**.
CPAL/device-driver queues and per-device sample-rate conversion are excluded;
actual audio underruns still require a real rack/device test.

Isolation permanently poisons an endpoint after a missed deadline. Such a case
stops collecting audio timings; stopped/silent blocks are never included as fast
samples. The report retains the failure rather than silently restarting it.
Native calls can overrun the deadline without a host error, so their measured
misses are also counted. Ratios are withheld if either case failed, has fewer
than 20 successful samples, or has less than 95% of its requested measured-block
coverage. There is no aggregate ratio formed by averaging percentile values.
The process exits nonzero for failed/censored cases or insufficient comparison coverage; deadline miss counts are
reported even when a native case completes. Use longer trials for tail analysis.
A short smoke run is not evidence of a reliable p99 or worst-case bound.

## Three-way GUI-only isolation experiment

`--gui-isolate` adds `GuiIsolated` to the comparison. This is a **benchmark-only**
flag, not a nice-rack hosting mode. All three cases then use separate inactive
editor replicas so that the GUI content is comparable:

| Case | Real audio chain | Editor replicas |
| --- | --- | --- |
| InProcess (baseline) | One local DSP thread | Inactive local instances |
| Isolated | Existing per-plugin audio helpers and audio IPC | One inactive GUI helper per instance |
| GuiIsolated | One local DSP thread, no audio IPC | One inactive GUI helper per instance |

The GUI-only helpers never call `process()`. The host deactivates their prepared
audio handles before publishing readiness, retains only the CLAP main-thread
owner, and creates no host DSP worker or sample mapping for those replicas.
Their only control path is startup readiness and shutdown supervision. The real
DSP thread never waits for them per block. A third-party plugin may create its
own internal background threads; the benchmark cannot prohibit that.

The full-isolation case has separate audio and editor helper processes in this
controlled experiment, rather than the production arrangement where an active
instance and its own GUI share one helper. Each case has the same count of real
DSP instances and inactive editor replicas. These results must not be mixed
with the earlier live-instance-editor workload; the report's `editor_replicas`
flag records the distinction, and mixed workloads cannot produce paired ratios.
Both alternative modes are compared separately against the baseline; trial order
rotates through all three positions.

```sh
cargo run -p nice-clap-host --example isolation-bench --release -- \
  --plugin "target/bundled/Nice Echo.clap" --copies 4 --frames 64 \
  --topology serial --seconds 5 --warmup 2 --trials 3 \
  --gui-isolate --editors --out target/isolation-three-way-editors.ron
```

Omitting `--editors` runs a headless control with the same process/instance
arrangement. It measures audio timing and helper/control overhead, **not graphics
or editor responsiveness**. The static editor replicas neither show real-chain
meters/waveforms nor control the real DSP parameters. No knob changes, preset
mirroring, or live telemetry bridge are part of this test. This experiment is
therefore not a generic implementation of cross-process CLAP editors.

`setup_ms` measures real DSP graph preparation. In visible replica tests,
`editor_open_ms` includes replica preparation/spawn and window-open acknowledgement;
it still does not prove first presentation. Startup-ready messages are complete
RON files published by rename. Parent-owned stdin pipes supervise GUI helper
lifetime, including a separate watchdog for a stuck GUI callback. The coordinator
retains its overall timeout and records unavailable results if desktop startup
cannot complete.

## Editor responsiveness comparison

Use a real desktop and add `--editors`. Start with a generous DSP buffer so audio
failure does not change the workload while measuring the GUI:

```sh
cargo run -p nice-clap-host --example isolation-bench --release -- \
  --plugin "target/bundled/Nice Echo.clap" --copies 4 \
  --frames 1024 --seconds 10 --warmup 2 --trials 3 --editors \
  --out target/isolation-editors.ron
```

This opens all editors in each case, without manual clicks, then closes them.
Do not close/minimize/cover them differently between trials. Closing an editor
invalidates that case. Linux editor cases use X11/XWayland, matching the CLAP
embedding APIs used by the host; they do not exercise the rack's separate Wayland
frame-service bridge. A GUI case needs every plugin to expose a
supported editor; otherwise it reports a failure.

A nominal 60 Hz heartbeat measures how late the **host event loop** runs, with
median/p95/p99/max lateness, actual heartbeat gaps, and skipped intervals. The
loop services native editors on its own main thread; in isolated mode each helper
services its editor independently. Setup and editor-open acknowledgements precede
the requested warmup. An open acknowledgement does not prove a first GPU frame
has presented.

These are **not editor FPS measurements**: CLAP has no standard presented-frame
counter. The benchmark hosts the real editors and audio graph, but does not draw
the rack's Frost graph view or automatically drag windows. Consequently it
isolates adapter/editor scheduling costs, rather than reproducing all of
nice-rack's GPU work. Headless heartbeat figures only reflect the service loop's
scheduling and must not be described as GUI performance. An audio failure remains
a failed/censored case even though GUI observation continues for the requested
duration; that GUI result cannot establish equivalence under a fully active DSP
workload. Actual per-editor rendering rates and drag behavior still need separate
instrumentation or platform testing.

## Running and supervising cases

The parent coordinator enforces `--timeout` (60 seconds per case by default),
kills stalled case processes, records crashes/timeouts as unavailable results,
and continues to the other cases. Isolated helpers retain their existing
parent-loss watchdogs. Partial reports are saved after each case. No untrusted
plugin is loaded in the coordinator itself. In-process cases still execute
operator-selected native code in their disposable case process; isolation is
not a security sandbox.

All control sockets use local loopback only. No internet access or audio-device
permissions are needed. For reproducibility, keep power mode, display refresh,
GPU/backend selection, background applications, plugin builds, and visibility
consistent; record platform results separately. Ordinary OS scheduling introduces
noise, so the benchmark is opt-in and has no brittle speedup assertion in CI.

Unit checks for graph construction, percentiles, invalid workloads, report
round-tripping, and rejection of censored comparisons run without plugins:

```sh
cargo test -p nice-clap-host --example isolation-bench
```

## Local validation (2026-11-26)

On macOS/aarch64, the release example built and all six unit checks and strict
Clippy passed. The example also cross-checked for Windows/MSVC and Linux/GNU.
Those are compilation checks, not native execution on those systems.

A headless run with two nice-echo instances, rack metering/timing enabled, 64 and
512 frames, both topologies, and two trials completed all native cases. All four
isolated 64-frame cases failed their production deadline guard; isolated serial
512-frame cases also failed in this run, while isolated parallel 512-frame cases
completed. One native warmup call also overran its budget. These short local
trials demonstrate failure reporting, not a stable worst-case bound or a general
platform-performance claim. Their RON output is in the local, untracked
`target/agent-build/isolation-comparison.ron`. GUI/editor trials have not been
executed here and require a real desktop.

## Three-way local run (2026-11-27)

A release headless test used four nice-echo instances in series at 48 kHz, with
metering/timing enabled, three trials per mode and buffer size, 0.25 seconds of
warmup and one measured second. The ranges below are **per-trial DSP medians**,
not pooled percentiles:

| Frames | Baseline | Full isolation | GUI-only isolation |
| --- | --- | --- | --- |
| 64 | 57.9–65.8 µs; 3/3 completed | 0/3 completed; deadline stops | 58.8–82.5 µs; 3/3 completed |
| 512 | 378.7–498.0 µs; 3/3 completed | 1/3 completed; two deadline stops | 394.2–484.9 µs; 3/3 completed |

Completion does not imply no overruns: baseline 64-frame trials recorded one
processing-budget overrun, and GUI-only 64-frame trials recorded four including
warmup. Native paths do not poison themselves on an overrun, unlike the isolated
audio transport. Ordinary scheduling outliers remain present. The short test is
not a real-time guarantee, and the four identical echoes are not the user's
earlier mixed four-plugin chain.

Local reports: `target/agent-build/isolation-three-way.ron` and
`target/agent-build/isolation-three-way-editors.ron` (untracked build artifacts).
The visible-editor attempt timed out in all modes during macOS GUI-service
initialization, with `com.apple.hiservices-xpcservice` connection errors in this
execution environment. Those cases contain no usable rendering measurements.
Run the visible command from a normal desktop terminal to assess GUI behavior.

## Successful Terminal run (2026-11-28)

The user subsequently ran the visible three-way test from a normal macOS
Terminal: four nice-echo instances in series, 48 kHz, 64/512 frames, three trials,
five seconds of warmup and five measured seconds, with `--gui-isolate --editors`.
The local report is
`target/agent-build/isolation-three-way-editors-terminal.ron` (untracked).
Baseline and GUI-only modes completed all trials without recorded processing
budget misses. Full isolation failed all three 64-frame trials and completed all
three 512-frame trials. The GUI-only host heartbeat had lower p99 lateness than
the baseline, while still not measuring actual editor FPS.

The archived summaries, main-thread requirements, interpretation limits, and
third-party compatibility implications are in
[macOS plugin threading findings](macos-plugin-threading-findings.md). The
2026-11-27 GUI-service failures above describe that earlier execution environment,
not a failure reproduced in the user's Terminal run.
