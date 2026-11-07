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
Linux, prefix Cargo commands with `RUSTFLAGS="-Ctarget-cpu=x86-64-v3"` as required
by NeuralAmpModeler-rs. Rust 1.99+ is required. Intel Macs are not supported by
that engine. Windows currently builds a dry mono-to-stereo processing stub;
model and cabinet loading buttons are disabled there.

The app opens silently until audio settings are chosen. Open the **settings cog in the upper right**;
use the same default-first dropdowns as nice-tuner, then click
**APPLY**. Defaults appear green; unset values appear as dashes. Back/Escape cancel drafts,
Reset selects defaults, and Apply stays disabled until settings are complete and changed. Linked output buffer/rate rows are disabled and
follow input values. Apply changes devices without restarting the application.
Input and output device rates may differ; monitoring uses the tuner's existing
band-limited conversion with clock drift compensation. Linux JACK uses named
ports, with an optional second output destination for duplicated stereo.

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

Standalone model name, model path, cabinet path, audio settings and window size
are stored in `nice-nam-settings.json` in the working directory. Only successful
asset preparations are saved. DAW instances persist asset paths through nice-plug
state instead; they do not read or write the standalone configuration or probe
host-owned audio devices. Asset files remain external and must stay available.

The frost editor uses baseview in standalone and plugin builds, with the same
macOS display-synchronized scheduling as nice-tuner. Shared configuration and
device validation come from `nice-settings`. Standalone stream ownership and
monitoring currently adapt the project's tuner implementation; these are
project-owned modules, not vendored third-party code.

## Build and checks

```sh
cargo build -p nice-nam --release
cargo test -p nice-nam -p audio-float --features nice-nam/standalone,nice-nam/diag
cargo clippy -p nice-nam --all-targets --features standalone,diag -- -D warnings
```

Tests include synthetic LSTM inference, variable block sizes, allocation counting,
cabinet impulse response/latency, missing files/rate mismatches, output routing,
configuration round-trips and monitoring conversion. Native listening, real
community models, multi-instance DAW use and plugin bundling require further
integration testing. Models using mirrored memory also need native POSIX shared
memory permissions; a restricted process may refuse their allocation.
