# Nice NAM

A frost GUI demonstrator for Neural Amp Modeler CLAP/VST3 plugins and a
standalone application. The same editor and `Plugin::process` implementation
run in both modes. The standalone owns CPAL streams or a Linux JACK client;
no dependency is patched, forked or vendored.

## Run

On Apple Silicon macOS:

```sh
cargo run -p nice-nam --release --features standalone
```

For editor diagnostics add `diag` (`--features standalone,diag`). On x86_64
Linux and on x86_64 Windows, prefix Cargo commands with
`RUSTFLAGS="-Ctarget-cpu=x86-64-v3"` as required by NeuralAmpModeler-rs. Rust
1.99+ is required. Intel Macs are not supported by that engine.

On every platform the model runs through NeuralAmpModeler-rs. Windows is the one
platform whose copy of it cannot come from a registry: the engine's Windows
backend arrived with 3.0.0, which has not been published, so the manifest names
the commit on its `main` branch instead of a version, under the dependency name
`nam_windows`. That is a build requirement as well as a source: the
`x86-64-v3` baseline the engine demands is now asked of Windows too, and setting
`RUSTFLAGS` that way replaces `+crt-static`, so name both if a build needs the
statically linked runtime. The git source is a stopgap rather than a preference:
once that release reaches the registry the manifest names a version again,
`nam_windows` retires, and one dependency stands for the engine everywhere.

`standalone` on Windows brings the ASIO transport with it: `azo`,
whose FFI layer declares the ASIO COM interfaces in Rust, so there is no
Steinberg SDK to obtain and `.cargo/config.toml` explains why the repository
points at nothing. Add `unsafe` to let the window reopen where you left it, or
centred the first time; the placement code lives in `window-position`, and a
window inside a host is never moved.

```sh
cargo run -p nice-nam --release --features standalone,unsafe
```

`LOAD MODEL` offers `.nam` and `.namb` everywhere, and the two cabinet rows are
live everywhere: reading both containers and convolving an impulse response are
the engine's work, and it does both here. What a model file may contain is the
loader's decision as well — byte size, the numbers around the network, the
architecture — so a corrupt file is refused on this platform by the same code
that refuses it on the others, rather than by a second set of checks this build
kept for itself. One difference remains in the order of things: a model asking
for another sample rate is refused here once it has been built, because the rate
is read from what the loader returned, so a wrong-rate file fails a little later
than it used to. The outcome, and the message, are the same.

The dialog comes from `rfd`. On Windows it is `rfd`'s async picker, which builds
the dialog on a thread `rfd` spawns and is collected on the frames that follow,
so the editor keeps painting while the dialog is open. That is not a preference:
inside a host, the interface thread already carries the COM apartment the toolkit
made when it created the window, and a common item dialog shown in that apartment
never finishes loading Downloads — the one folder the shell hands over
asynchronously, which sits at "Working on it…" — while every other folder fills
at once. Elsewhere the dialog stays on the interface thread, which on macOS is
the only thread it may use.

The app opens silently until audio settings are chosen. Open the **settings cog in the upper right**;
use the same default-first dropdowns as nice-tuner, then click
**APPLY**. Defaults appear green; unset values appear as dashes. Back/Escape cancel drafts,
Reset selects defaults, and Apply stays disabled until settings are complete and changed. Linked output buffer/rate rows are disabled and
follow input values. Apply changes devices without restarting the application.
Input and output device rates may differ; monitoring uses the tuner's existing
band-limited conversion with clock drift compensation. Linux JACK uses named
ports, with an optional second output destination for duplicated stereo.
On ASIO the two settings rows **Second output** and **Second device** copy what is
being played — onto a higher pair of the output driver's channels, or onto a second
ASIO interface running on its own clock — which is a copy of the signal rather than
a second destination: the JACK row above is still the one that opens another port.
[nice-tuner's README](../nice-tuner/README.md#second-output) documents what the two
rows offer and refuse.

## Main view and processing

- **LOAD MODEL** selects a `.nam` or `.namb` file using `rfd`.
- **LOAD CABINET** selects a WAV impulse response; **REMOVE CABINET** bypasses it.
- Model architecture, topology, native sample rate, receptive field and path
  appear after loading. Errors leave the previous working chain in place.
- Mono input → NAM mono inference → optional mono cabinet convolution → identical
  left/right output samples. Without a model the NAM stage is bypassed.
- Models must be mono and match the plugin/input processing rate. Rate mismatch
  is reported explicitly; model-rate resampling is not implemented yet. Cabinet
  IRs are resampled to the processing rate by the engine. The output device can
  independently use another rate.

The background loader prepares models and cabinet FFTs, negotiates a maximum
256-sample inference block, and transfers ownership through bounded SPSC queues.
Plugin DSP splits arbitrary host blocks, allocates no buffers and takes
no locks. The standalone adapter uses a nonblocking plugin try-lock during
reconfiguration; contention produces silence rather than waiting. Replaced chains return to the worker for destruction. Cabinet latency
is reported to the host; reset clears model and convolution state. Switching
chains happens at block boundaries; transition crossfading is not implemented.
The small local [`audio-float`](crates/audio-float/SAFETY.md) wrapper temporarily
sets the floating-point mode required by NAM and restores the caller's state.
The application itself denies unsafe code.

The plugin instance owns its preparation worker's stop signal and join handle.
Instance destruction wakes an idle worker and joins it before releasing the
instance's fields or returning to the host. This happens during destruction,
never in `process()`, `reset()` or ordinary deactivation. A model/cabinet load
already running through a non-cancellable dependency API must finish before
destruction can return. Cancellation is checked before and after preparation;
a result completed after cancellation is observed is discarded on the worker.
Pending and retired chains are cleaned up there; ready chains not consumed by
the plugin are released with the instance's fields after the join. Keeping a
copy of the GUI's shared state alive does not keep the preparation worker alive.

Standalone model name, model path, cabinet path, audio settings and window size
are stored in `nice-nam-settings.ron` in the working directory. Only successful
asset preparations are saved. DAW instances persist asset paths through nice-plug
state instead; they do not read or write the standalone configuration or probe
host-owned audio devices. Asset files remain external and must stay available.

The frost editor uses baseview in standalone and plugin builds, with the same
macOS display-synchronized scheduling as nice-tuner. Shared configuration and
device validation come from `nice-common`. Standalone stream ownership and
monitoring currently adapt the project's tuner implementation; these are
project-owned modules, not vendored third-party code.

## Build and checks

```sh
cargo build -p nice-nam --release
cargo test -p nice-nam -p audio-float --features nice-nam/standalone,nice-nam/diag
cargo clippy -p nice-nam --all-targets --features standalone,diag -- -D warnings
cargo xtask bundle nice-nam --release
```

`cargo xtask bundle` (the same `nice-plug-xtask` driver as nice-tuner, with
`Nice NAM` named in the workspace-root `bundler.toml`) detects the exported
CLAP and VST3 entry points in the built library and writes both bundles to
`target/bundled/`: the single-file `Nice NAM.clap` and the `Nice NAM.vst3/`
directory, with the embedded fonts' licence texts shipped alongside, as for
nice-tuner. On x86_64 — Linux or Windows — the `RUSTFLAGS` prefix stated above
applies to this command too, and on Windows remember that naming it drops
`+crt-static`.

Tests include synthetic LSTM inference, variable block sizes, allocation counting,
cabinet impulse response/latency, missing files/rate mismatches, output routing,
configuration round-trips and monitoring conversion. Native listening, real
community models, multi-instance DAW use and loading the bundles in a real host
require further integration testing. Models using mirrored memory also need the
system to hand it over: POSIX shared-memory permissions on Linux and macOS, a
file mapping this process may reserve and view on Windows, where that backend is
the one the engine gained most recently and still calls experimental.

Worker lifecycle tests also cover idle shutdown with retained shared state,
in-flight cancellation, a full ready queue with pending/retired chains, worker
panic cleanup, and plugin destruction before editor-held shared state expires.
An optional native check runs 32 load/create/destroy/entry-release cycles in an
isolated process, using Clack directly rather than the rack's library pinning:

```sh
cargo build -p nice-nam --lib --locked
NICE_RACK_TEST_NAM="$PWD/target/debug/libnice_nam.dylib" \
  cargo test -p nice-clap-host --test nam_unload --locked
```

The path above is for macOS; use `libnice_nam.so` on Linux (with the documented
SIMD build flags) or `nice_nam.dll` on Windows. The check uses no model, native
editor or audio device. Operating-system loader residency can delay unmapping,
so the unit tests separately verify worker completion before destruction
returns. With no `NICE_RACK_TEST_NAM` set, the native check explicitly reports
that it was not run. This does not establish every third-party engine's unload
contract or justify removing the rack's general library-retention policy.

The main view has input and output RMS/peak-hold meters along the bottom. The
input meter measures before the gate; the output meter measures after the model
and cabinet. Click the input meter to choose its gate threshold (default −60 dBFS);
hovering shows the current value. The standalone saves `gate_db` in
`nice-nam-settings.ron`. Gate gain opens over 5 ms and closes over 50 ms before
the model; its threshold is evaluated on each processing chunk's input RMS.
