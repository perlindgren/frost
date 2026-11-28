# macOS plugin threading and isolation findings

Recorded 2026-11-28. These findings separate the measured results from the
architectural limits and the work still required.

## Decision and scope

Retain in-process hosting as the demonstrated general CLAP path at 64 frames.
Whole-plugin isolation is a valid architecture for unmodified third-party
plugins, but this prototype's audio transport is not yet reliable at that block
size. GUI-only isolation is an experiment using independent editor replicas; it
is **not** a working, transparent third-party plugin-hosting mode.

The Terminal test supports investigating independent GUI scheduling. It does not
establish that arbitrary plugin GUIs can be detached from their real instances.
Do not promote `--gui-isolate` to a production rack option on that basis.

The tested worktree was `codex/rack-process-isolation`, based on
`codex/baseview-0.4`, with HEAD `f4d7b6e` (Baseview 0.4 redraw scheduling) and
isolation/benchmark changes that were uncommitted at measurement time. `nice-plugin` uses Baseview 0.4.2. The
separate `win-work` rendering optimizations and instrumentation were **not**
merged. This is macOS/aarch64 evidence, not a Windows or Linux runtime result.
The report did not capture an exact CPU model or compile flags; do not infer an
M-series model or instruction-set tuning from the architecture alone.

## Thread and process ownership

### Baseline

The real plugin instances live in the rack process. One DSP worker executes the
prepared graph in topological order, including its serial and parallel branches.
A graph branch being parallel does not itself make native plugin calls concurrent.
GUI/lifecycle/main-thread callbacks are serviced separately from the DSP worker.
In-process editors can therefore delay the host's GUI loop without doing much
DSP work. This does not mean rendering and DSP are the same thread.

CLAP defines a persistent main thread for main-thread callbacks, and a symbolic
audio thread whose OS-thread identity may change, provided an instance's audio
callbacks are never concurrent. On macOS and Windows, CLAP ties its main thread
to GUI/timer event delivery on the program's main thread. Merely assigning each
third-party editor a new Rust thread is not a generally conforming solution.
See the [CLAP threading contract](https://github.com/free-audio/clap/blob/main/include/clap/ext/thread-check.h).

macOS event handling also has AppKit main-thread requirements. This is not a
blanket statement that every graphics operation must be serial: a toolkit can
use worker threads and GPU queues internally. The host cannot assume an arbitrary
editor supports moving its lifecycle and native event handling to a worker.
See Apple's [Cocoa threading guidance](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/Multithreading/ThreadSafetySummary/ThreadSafetySummary.html).

Changing Baseview to another windowing library does not remove these contracts.
Rust async can help organize nonblocking host control work, but does not turn a
blocking foreign GUI callback into independent rendering or make AppKit objects
freely transferable between threads.

### Whole-plugin isolation (`--isolate`)

In the rack implementation, each helper owns the **same real instance's GUI and
DSP**: its initial thread handles lifecycle/GUI callbacks and a persistent worker
handles audio. The rack owns the prepared graph and a DSP coordination worker.
Ready independent helper work is submitted before waiting; serial dependencies
still require the upstream result first. Separate helpers allow independent GUI
main threads and allow independent DSP branches to execute concurrently.

The current audio path uses shared sample memory with bounded copies. A parent
bridge thread performs socket wake/acknowledgement I/O. The rack DSP worker
publishes a request, unparks that bridge, and checks completion with a bounded
spin/yield wait against one absolute block deadline. No extra pipeline block is
introduced, but copies, bridge scheduling, process wakeups and completion waits
remain. Workers currently use ordinary OS scheduling.

A timeout permanently poisons that endpoint. The rack silences the block and
uses its recovery path; the benchmark stops recording audio for that failed
case. Later silent/stopped blocks are not legitimate fast timing samples.

Shared memory does not make the transport zero-copy, eliminate scheduling cost,
or make an opaque native plugin instance accessible from another process.
The measurements implicate transport/scheduling overhead in this implementation,
but do not identify the dominant stage. We have not separately timed copying,
bridge wakeup, helper dispatch, plugin computation, and completion observation.

### GUI-only experiment (`--gui-isolate` in the benchmark only)

The real audio graph remains in the case process on one DSP thread, with no
audio IPC. Separate helpers own **additional inactive editor instances**. Their
prepared audio handles are stopped/deactivated before readiness; the benchmark
creates no host DSP worker or sample mapping for them and never calls their
`process()`. A plugin could still create its own internal background threads.

For fairness, the controlled three-way benchmark uses independent inactive
editor replicas in **all** modes:

| Mode | Real DSP instances | Editor replicas |
| --- | --- | --- |
| Baseline | One local DSP worker | Local inactive instances |
| Full isolation | Per-plugin audio helpers | Separate inactive GUI helpers |
| GUI-only isolation | One local DSP worker | Separate inactive GUI helpers |

Thus the full-isolation *benchmark case* has separate audio and GUI helpers; the
rack's normal `--isolate` implementation keeps each real instance's GUI and DSP
together in one helper. These are distinct workloads. The report records
`editor_replicas: true` and rejects ratios against a live-instance GUI workload.

The replicas do not control the real graph or show its live meters/waveforms.
Parameter/state synchronization and telemetry bridging are not implemented.
This is a controlled rendering/scheduling experiment, not a usable isolated
editor architecture for arbitrary plugins.

Separate GUI processes permit concurrent CPU-side GUI execution and independent
event loops. They do not guarantee concurrent GPU execution or a particular
frame rate: processes still compete for CPU, GPU, driver and compositor resources.
A VSync wait need not block another process's main thread, but shared resource
contention can still delay it.

## Successful macOS Terminal test

The user ran the release benchmark from a normal Terminal with four identical
nice-echo instances in series, 48 kHz, 64/512 frames, three trials per mode,
five seconds of warmup and five measured seconds. Editors were enabled and all
modes used inactive editor replicas. Wire meters and per-node processing-time
observations were enabled for the real graph.

The original local source report is
`target/agent-build/isolation-three-way-editors-terminal.ron`. An unchanged copy
is preserved with the experimental branch as
[the Terminal report](benchmarks/macos-isolation-2026-11-28.ron), so the evidence
survives a build-directory cleanup. Its recorded fixture paths are machine-local.
The following tables preserve the relevant summary.

### Audio timings

Ranges below are ranges of **individual trials'** medians/p99s, not pooled
percentiles. Worst means the largest measured successful call across the three
trials. Times are wall-clock durations inside the graph call, including audio
transport waits where applicable, not CPU-utilization percentages.

| Frames | Mode | Median range (µs) | p99 range (µs) | Observed worst (µs) | Complete trials |
| --- | --- | --- | --- | --- | --- |
| 64 | Baseline | 9.96–10.33 | 14.33–16.63 | 99.83 | 3/3 |
| 64 | Full isolation | Censored | Censored | Not comparable | 0/3 |
| 64 | GUI-only isolation | 13.04–14.00 | 45.54–52.71 | 149.54 | 3/3 |
| 512 | Baseline | 76.83–77.04 | 94.50–95.21 | 234.42 | 3/3 |
| 512 | Full isolation | 320.21–337.25 | 556.21–674.58 | 1846.33 | 3/3 |
| 512 | GUI-only isolation | 98.67–100.58 | 184.04–224.25 | 1078.17 | 3/3 |

At 48 kHz the processing budgets are approximately 1.333 ms for 64 frames and
10.667 ms for 512 frames. Baseline and GUI-only cases recorded **zero measured
or warmup processing-budget misses** in this Terminal run. Full isolation failed
all three 64-frame trials: two during warmup, one during measurement. Its partial
successful-call timings must not be used as a completed-trial distribution.
All full-isolation 512-frame trials completed without recorded budget misses.

GUI-only median overhead relative to baseline was small in absolute terms:
roughly 3–4 µs at 64 frames and 22–24 µs at 512 frames. Full isolation's 512-frame
median graph-call cost was roughly 4.2–4.4 times baseline, including transport.
Those figures are specific to this short, light workload and execution context.
They do not characterize a NAM-heavy graph or the previously reported mixed
four-plugin chain using about 13% of its budget.

### Host GUI responsiveness

The benchmark schedules a nominal 60 Hz host heartbeat. The following figures
are lateness relative to that schedule, not frame-render durations or editor FPS.

| Frames | Baseline p99 heartbeat lateness | GUI-only p99 heartbeat lateness |
| --- | --- | --- |
| 64 | 7.65–8.41 ms | 4.44–4.71 ms |
| 512 | 7.70–8.40 ms | 4.25–4.50 ms |

GUI-only isolation reduced this host-loop lateness by roughly 38–49% in paired
trials. This supports investigating independently scheduled GUI processes; it
does not identify which scheduling, presentation or graphics costs dominated.
It is not proof that every editor rendered at 60 FPS, that the GPU was faster,
or that all dragging/input behavior improved. The benchmark does not draw the
rack's full Frost graph view or automate window dragging.

Do not treat full-isolation 64-frame GUI measurements as a successful equivalent
workload: its real audio chain had already stopped. Failed audio cases remain
censored even if their static editor replicas continue drawing.

### Execution environment and limits

Earlier attempts inside the agent execution environment timed out during macOS
GUI initialization and logged `com.apple.hiservices-xpcservice` connection
errors. The Terminal run subsequently produced usable editor-host measurements.
This strongly supports an execution-environment restriction, but the exact
sandbox/entitlement cause was not isolated. Screen-recording permission is a
separate permission and did not establish GUI-service access for those processes.

Earlier headless runs also produced materially different absolute audio timings.
Do not attribute that difference solely to the sandbox without a controlled
comparison of power, scheduling, build and execution conditions.

No physical audio device was opened. The worker was software-paced; CPAL/device
queues and per-device sample-rate conversion were excluded. Processing-budget
miss counts do not establish hardware xrun behavior. Ordinary OS scheduling,
software wake lateness and future workload changes still matter. No claim of
hard real-time safety or production reliability follows from a finite run.
Every measured plugin in this dataset was our own nice-echo export. No broad
third-party compatibility matrix was exercised.
Native Windows/Linux runtime performance has not been established; prior
cross-compilation checks are not execution tests.

## Implications for third-party plugins

CLAP's [GUI extension](https://github.com/free-audio/clap/blob/main/include/clap/ext/gui.h)
creates/shows/resizes/destroys a GUI through the actual plugin instance, using
main-thread callbacks. It does not define a generic protocol for detaching that
GUI into another process while its private DSP object remains elsewhere.
Serializing standard parameters/state alone does not transfer private UI/DSP
objects, live analyzer/meter data, or all proprietary interactions.

| Approach | Status for compatible unmodified third-party CLAP plugins |
| --- | --- |
| Baseline | Implemented; the only tested general path here completing 64-frame trials |
| Whole-plugin isolation | Architecturally valid; prototype needs reliable low-buffer transport/scheduling |
| GUI-only replica benchmark | Independent copies only; not a functional editor for the real graph |
| Custom GUI-only bridge for our plugins | Possible with cooperation; not implemented |

Even baseline does not mean every plugin works: format features, buses, platform
support and plugin-specific behavior still limit compatibility. Conversely, the
current isolation failures are not proof that efficient whole-plugin isolation
is impossible.

Bitwig documents whole-instance/group process hosting, including grouping by
manufacturer/type and isolating individual instances. That is a precedent for
transparent whole-plugin hosting, not evidence that an arbitrary instance's GUI
and DSP can be split through a standard GUI-only API. Its public documentation
does not specify the detailed IPC, scheduling or buffering implementation; we
have not established those details. See [Bitwig's hosting modes](https://www.bitwig.com/userguide/latest/vst_plug-in_handling_and_options/).

Our own plugins could expose a custom bridge for parameters, non-parameter state
changes, and bounded meter/waveform snapshots. That would require state identity,
ordering, lifecycle/recovery and host-notification design. It must keep the DSP
independent of GUI progress. Merely mirroring presets into a second instance is
not equivalent to implementing that bridge.

## Next investigations

1. Keep baseline as the default while preserving this repeatable comparison.
2. Instrument transport stages separately, with instrumentation overhead checked,
   before choosing optimizations or assigning the failures to one mechanism.
3. Investigate suitable audio-worker scheduling and lower-overhead wake/completion
   primitives. Compare ordinary and platform-specific scheduling explicitly.
4. Measure a grouped whole-chain helper to reduce serial process boundaries,
   without detaching opaque plugin GUIs. Grouping changes crash granularity and
   keeps that group's editors sharing a GUI loop; it is not GUI-only isolation.
5. Repeat native Windows/Linux tests and representative heavy/default-loaded
   presets. Add per-editor presented-frame instrumentation where possible and
   actual rack/device, drag, resize and minimize tests.
6. Consider a cooperating GUI bridge for our own plugins separately from the
   general third-party hosting path. Do not replace the latter with replicas.

For commands, report semantics and benchmark limitations, see
[the benchmark guide](rack-isolation-benchmark.md). For the actual helper
implementation, see [the process-isolation notes](rack-process-isolation.md).
