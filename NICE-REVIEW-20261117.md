# Nice plugins review — 2026-11-17

Reviewed revision: `ef15813` on `main`.

Scope: this repository's nice-* plugins, shared code, hosting adapters, local
wrapper crates and documentation. Third-party dependency implementations were
not reviewed as deliverables or modified. Public dependency contracts were
consulted to understand our callers' obligations.

This records findings before implementation. All items were open at the time
of the original review; see the implementation follow-up below. Severity reflects potential impact; source-level findings are
distinguished from behavior reproduced in a focused probe.

## 1. [P1] Process-global macOS display callback class name

Source:

- [macos-frame-sync/src/lib.rs](nice-tuner/crates/macos-frame-sync/src/lib.rs),
  `Target` declaration, around line 56.
- Each plugin's `src/gui.rs`, `start_display_sync()`.

The display callback target uses the fixed Objective-C class name
`FrostNiceTunerDisplaySyncTarget`. All seven plugins use this wrapper. Separate
plugin libraries carry separate Rust registration guards, but Objective-C class
names are process-global. Opening editors from different plugin libraries can
therefore attempt to register the same class twice and abort the host process.

The earlier `RawWindowMetalLayer` fix addresses a different class and does not
remove this registration conflict. Multiple instances of one loaded binary do
not exercise the same case as two independently loaded plugin binaries.

Evidence: source inspection. A mixed-plugin native editor reproduction was not
run during this review.

Suggested direction: make callback target registration safe for independently
loaded libraries. Account for callback implementation ownership and library
unloading as well as name uniqueness; blindly reusing another library's class is
not a sufficient lifetime solution.

Validation needed: open, close and reopen editors from at least two distinct
plugin bundles in the same macOS host, with multiple instances of each.

Implementation follow-up (2026-11-17): the shared wrapper now uses one uniquely
named, cached class per loaded library, with a suffix for occupied names. Native
targets borrow rather than own Rust callback state; stopping clears that borrow
before invalidation and destruction. Unit tests and a main-thread native test
cover independent cdylibs, multiple instances, reopening, self-destruction during
dispatch, late dispatch after stop, and release before library unload. See
[the updated safety contract](nice-tuner/crates/macos-frame-sync/SAFETY.md).
Visible editor cadence and mixed-plugin behavior in a real DAW remain desktop
verification tasks; this follow-up does not resolve finding 2 or other unload
obligations.

## 2. [P1] NAM preparation worker can outlive its plugin library

Source:

- [nice-nam/src/engine.rs](nice-nam/src/engine.rs), `start()`, around line 32.
- [nice-nam/src/lib.rs](nice-nam/src/lib.rs), `NamPlugin` ownership fields.
- [nice-clap-host/src/lib.rs](nice-clap-host/src/lib.rs), `LOADED` workaround.

`engine::start()` discards the preparation thread's `JoinHandle`. The worker
checks a weak reference between iterations, sleeps for 20 ms, and may spend
longer preparing a model or cabinet. Destroying the plugin instance does not
establish that this thread has exited before destruction returns to the host.

If a host subsequently unloads the plugin library, the worker may resume or
finish a load in unmapped code. This applies to Windows, macOS and Linux; the
worker is not platform-gated. Nice-rack keeps loaded libraries mapped until
process exit, which masks this issue there rather than fixing NAM's lifetime
contract for other hosts.

Evidence: source inspection, including the explicit library-retention workaround
in our host. Native DAW unload was not reproduced during this review.

Proposed fix:

1. Return an owned worker guard from `engine::start()`, containing a stop signal,
   a wake mechanism and the `JoinHandle`.
2. Replace the unconditional sleep with an interruptible wait.
3. Request stop and join during plugin destruction, before releasing resources
   the worker may access and before returning control to the host.
4. Check for cancellation before and after model/cabinet preparation. If an
   individual loading operation cannot be interrupted through its public API,
   destruction must wait for it to finish.
5. Keep normal preparation and retirement off the audio thread. Do not add a
   blocking join to `process()`.

A timeout followed by detaching the worker would preserve the unload bug.
Implement this in project-owned code without modifying dependencies. Do not
remove the rack's library-retention policy merely because this one worker has
been fixed; other callbacks and library-owned activity need separate validation.

Validation needed: destruction while idle, while preparing assets, and with
ready/retirement queues populated. Assert worker completion before destruction
returns, then exercise native load/create/destroy/unload in an isolated process.

Implementation follow-up (NAM worker shutdown): NamPlugin now owns a worker
guard with a stop signal and JoinHandle. Destruction signals stop, unparks the
worker and joins before releasing instance fields. An already-running load
must finish; cancellation is checked before and after loading, and an observed
cancelled result is not published. Idle, in-flight, queue-cleanup, panic and
editor-retained-state tests cover the ownership contract. The optional
`nice-clap-host` `nam_unload` test uses Clack directly in an isolated process for
32 native load/create/destroy/entry-release cycles; it does not use the rack's
library pinning. OS residency policies may delay actual unmapping, and the
native check does not load models or open editors. Other plugin/library unload
obligations and the rack's retention policy are unchanged.

## 3. [P1] Delay effects report no tail to the host

Source:

- [nice-echo/src/lib.rs](nice-echo/src/lib.rs), `process()`, around line 583.
- [nice-ecomplex/src/lib.rs](nice-ecomplex/src/lib.rs), `process()`, around line 499.

Both return `ProcessStatus::Normal` regardless of buffered delayed audio or
feedback. Under the public framework contract, Normal permits suspension when
input is silent. The format wrappers report no tail for this status, and the
CLAP processing result permits quiet-plugin suspension.

A host can stop calling the plugin during the quiet gap before a delayed repeat,
or truncate the end of an offline render. A rack that continuously processes
every node does not expose this scheduling error.

Evidence: source inspection against the public processing-status contract.
Tail-aware DAW scheduling and offline rendering were not exercised.

Suggested direction: report a suitable tail while buffered audio remains, and
use an appropriate keep-alive policy for sustained self-oscillation. Decide
explicitly how modeled noise affects that policy. Audit NAM's cabinet tail too;
the confirmed examples above are the two delay effects.

Validation needed: an impulse followed by silence in a host harness that honors
quiet/sleep and tail reporting, plus finite-tail rendering and self-oscillation.

## 4. [P2] Delay reset paths allocate and do not fully reset modulation

Source:

- [nice-echo/src/lib.rs](nice-echo/src/lib.rs), `reset()`, around line 505.
- [nice-echo/src/dsp.rs](nice-echo/src/dsp.rs), `set_rate()` and spring setup.
- [nice-ecomplex/src/lib.rs](nice-ecomplex/src/lib.rs), `reset()`, around line 419.
- [nice-ecomplex/src/dsp.rs](nice-ecomplex/src/dsp.rs), `set_rate()`, around line 252.

Both plugin reset methods call `set_rate()` with the existing rate. Those methods
replace delay storage with newly allocated vectors. A host may reset on the
audio thread, so allocation-free ordinary processing does not establish an
allocation-free audio lifecycle.

A temporary isolated probe included the project DSP modules and measured the
same-rate `set_rate()` paths with `allocation-counter` at 48 kHz:

| Reset path | Allocations | Allocated bytes |
| --- | ---: | ---: |
| Echo | 12 | 161,360 |
| Ecomplex | 1 | 576,032 |

The probe also warmed Ecomplex, ran its reset path, and compared a subsequent
impulse response against a fresh instance under identical controls and block
sizes. The maximum absolute sample difference was approximately `0.063357`.
`wow_phase` is initialized in the constructor but not cleared by `set_rate()`,
despite the plugin reset comment saying the wow phase returns empty. Echo also
retains modulation/transport fields outside the buffers cleared by `set_rate()`.

Suggested direction: separate allocation/rate preparation from an allocation-free
reset that clears existing storage in place. Define which modulation and
smoothing state reset intentionally preserves, and make implementation and
documentation agree. If reset is intended to match fresh activation, test that
equivalence explicitly.

Validation needed: count allocations and frees during reset as well as process;
test reset after a nontrivial history and after parameter changes. Existing DSP
allocation tests measure processing and moving controls, not these reset paths.

Implementation follow-up (delay resets): both plugin reset callbacks now clear
existing DSP storage in place instead of invoking allocating rate preparation.
Echo clears both tape lanes, heads, spring tanks, tone/record filters, drive
envelopes, noise seeds and shared modulation state, and starts transport speed
at the current Repeat Rate setting. Ecomplex clears the tape, tone filters and
wow phase. Parameters and prepared sample rate remain unchanged; meters retain
their existing reset behavior. Regression tests measure reset allocations and
frees, then compare four seconds of stereo impulse response against a fresh
instance at 8, 48 and 96 kHz, after history and control changes and repeated
resets. Native audio-thread reset execution in a DAW remains untested.

## Documentation inconsistencies

### Hosting unsafe-code inventory is incomplete

[nice-clap-host/SAFETY.md](nice-clap-host/SAFETY.md), around lines 8–11, says
library loading is the library's sole unsafe operation. The implementation now
also contains native editor parenting and Windows editor helpers.

The feature/platform distinctions are:

| Operation | Platforms | Behind the plugins' `unsafe` feature? |
| --- | --- | --- |
| Optional standalone window-position wrapper | Windows | Yes |
| CLAP editor parenting in nice-clap-host | macOS, Windows, Linux supported embedding paths | No |
| Showing native child windows and posting frame-timer messages | Windows | No; platform-gated only |

The public editor-parenting wrapper is safe for its callers. Its internal
`set_parent()` call is unsafe because the dependency cannot enforce that the
native parent remains valid until the editor is destroyed. Our wrapper retains
an `Arc` to the parent and destroys the editor before releasing it, including
error cleanup. Keep that boundary documented and audited; moving it behind a
feature would not remove the need for its safety documentation.

Update the inventory to cover each unsafe operation, ownership rule, thread rule
and platform gate. Check the older prototype wording against the current live
hosting implementation too.

### Shared-crate introduction is outdated

[nice-common/README.md](nice-common/README.md), around lines 3–6, still describes
the crate as containing no audio processing and serving tuner/NAM. It now
provides audio transport, resampling and metering used across the plugin family
and rack. Its physical-CPAL-only transport description is also outdated relative
to the ASIO and JACK paths.

Update the introduction and feature matrix to match the current callers and
backend implementations, rather than adding more exceptions to older claims.

## Improvements and refactors

### Share editor and standalone lifecycle machinery

Across wah, phase, pi-face, echo and ecomplex, the GUI shell, editor adapter,
standalone owner and standalone processing adapter total roughly 5,700 lines
with closely matching structure. A platform/lifecycle fix currently needs to be
carried through several copies.

Extract a shared editor shell and standalone pump behind small project-owned
interfaces. Keep effect DSP, parameter definitions, UI controls and settings
types local. Preserve the same plugin processing implementation in standalone
and hosted builds, and keep native unsafe primitives inside thin local wrappers.

### Write settings atomically

[nice-common/src/settings.rs](nice-common/src/settings.rs), `save_to()`, around
line 305, writes directly over the destination with `std::fs::write`. A failed or
interrupted write can leave the previous valid settings truncated; rack settings
now include native instance states as well as device choices.

Prefer writing a temporary file in the destination directory, completing the
write, and replacing the destination using a platform-appropriate operation.
Preserve the previous valid file if the write fails. Add failure-path tests,
including the geometry-only save path.

### Extend lifecycle and native integration coverage

Prioritize tests for:

- Reset-time allocation and reset determinism.
- Tail reporting and hosts that suspend quiet plugins.
- Multiple different plugin libraries with open editors in one process.
- Worker shutdown followed by library unload.
- Failed settings replacement preserving the last valid state.

These complement existing DSP, routing, state and allocation tests; passing the
existing suites does not cover the native lifecycle cases above.

## Validation performed and limits

Review host: macOS on `aarch64-apple-darwin`, rustc
`1.100.0 (0ef1bc386 2026-11-09)`. CPU brand and logical-core queries were denied
by the execution environment; no CPU-capability conclusion was drawn from that.

The following selected library suites passed, with their declared ignored tests
remaining unexecuted:

```sh
cargo test -p nice-tuner -p nice-nam -p nice-wah -p nice-phase \
  -p nice-pi-face -p nice-echo -p nice-ecomplex --lib --locked --offline

cargo test -p nice-common -p nice-clap-host -p nice-rack -p nice-rack-engine \
  --features nice-common/audio-io,nice-common/ui --lib --locked --offline
```

The review used a build-local Cargo cache. The reset measurements and Ecomplex
comparison ran in a temporary standalone harness outside the repository, using
the current DSP source files. That harness did not link nice-plug's debug
allocator alongside allocation-counter.

Not performed: full standalone feature-matrix testing, native editor opening,
real DAW unload/scheduling, hardware listening, Windows execution or Linux
execution. Source-level native risks are not reported as reproduced crashes.

No application, wrapper or dependency implementation was changed during the
review. Creating this record does not implement or resolve any finding.
