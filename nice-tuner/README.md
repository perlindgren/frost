# Nice Tuner

Nice Tuner is an example project whose primary purpose is to showcase
[frost](../README.md) as a GUI toolkit for **VST3 and CLAP plugin development**.
A working chromatic tuner for stringed instruments provides the application:
pitch detection, a tuning display, parameters and interactive settings.

Its second purpose is to demonstrate **safe integration with external crates
through thin, project-owned wrappers**, keeping third-party source unchanged
instead of maintaining a large vendored fork.

## frost in a plugin

The example shows how to embed frost's renderer in a DAW-owned plugin window,
translate window events into GUI input, and share audio analysis and parameter
state with the editor. nice-plug supplies the VST3/CLAP integration; baseview
provides the window and events; frost builds and renders the interface.

The standalone build makes debugging and inspecting logs easier outside a DAW.
It uses the same plugin processing implementation, frost UI and baseview window
integration as the plugin builds. Its application-owned audio host supplies the
streams that a DAW would normally manage. This keeps it useful for developing
the actual plugin, while host automation, editor lifecycle and other DAW-specific
behavior still need testing in a real host. See [Standalone fidelity](#standalone-fidelity-what-you-see-embedded).

The main layers are:

| layer | crate | role |
|---|---|---|
| host protocol + audio | [nice-plug 0.4.2](https://codeberg.org/RustAudio/nice-plug) | CLAP/VST3, params, DSP↔UI plumbing |
| window embedding | [baseview `=0.3.3`](https://github.com/RustAudio/baseview) | child view inside the DAW window, host-driven events & frame timer |
| rendering | `frost` (the workspace root package) | scene graph + wgpu 30 2D renderer, driven as a guest through `frost::embedded::Embedded` |
| DSP | `crates/pitch` | allocation-free YIN pitch detection + note mapping |

See [docs/VIABILITY.md](docs/VIABILITY.md) for why the GUI stack looks like this
(winit cannot embed into a host; baseview is nice-plug's sanctioned BYO-GUI path),
and [docs/SCENE.md](docs/SCENE.md) for the UI design. VIABILITY.md was written
when the answer was "fork the engine"; the engine grew the embedded path
instead, so read that document as the reasoning that got here, not the plan.

## Safe integration through thin wrappers

Both nice-tuner targets unconditionally use `#![deny(unsafe_code)]`. Where an
external API requires unsafe calls, a small local crate owns that boundary and
exposes only the safe operations this example needs:

| Local crate | Safe interface | Boundary it handles |
|---|---|---|
| [`core-audio-device`](crates/core-audio-device/src/lib.rs) | Read-only device names, preferred channel pairs and buffer frame counts | Native CoreAudio calls, output sizes and CFString ownership |
| [`window-position`](crates/window-position/src/lib.rs) | Restore, center and read a borrowed Windows window's position | Win32 placement calls; enabled by its `unsafe` feature |
| [`plugin-buffer`](crates/plugin-buffer/src/lib.rs) | Scoped access to preallocated planar audio buffers | nice-plug's unsafe buffer-slice setup, storage lifetimes and disjoint channels |

The buffer wrapper uses fixed-capacity `heapless` sample storage, boxed once
during setup to keep its address stable. The call to the plugin's `process`
method is safe and remains in the application.

Each wrapper documents its unsafe invariants and validation in its own
`SAFETY.md`: [CoreAudio](crates/core-audio-device/SAFETY.md),
[window placement](crates/window-position/SAFETY.md) and
[plugin buffers](crates/plugin-buffer/SAFETY.md). Tests cover bounds, error paths,
ownership and allocation behavior; compile-fail examples check that borrowed
audio buffers cannot escape their processing call. Safety depends on maintaining
these contracts, not merely moving an unsafe block into another crate.

The integration uses public dependency APIs. Dependencies are not modified,
patched or vendored. The wrappers keep our assumptions and platform-specific
code small enough to review, while the upstream crates retain their own
implementations. This is the pattern the example is intended to demonstrate.

## Layout

```
src/lib.rs        plugin: TunerPlugin (params, process), TunerEditor (baseview spawn)
src/gui.rs        baseview WindowHandler driving `frost::embedded::Embedded` (surface, inputs)
src/tuner_ui.rs   frost scene: gauge, needle, note readout, input meter (`Process` impl)
src/main.rs       standalone target (`--features standalone`)
crates/pitch      pure-Rust YIN pitch detector (unit-tested, no deps)
crates/core-audio-device  safe read-only CoreAudio queries
crates/window-position   safe Windows placement, behind the unsafe feature
crates/plugin-buffer     safe nice-plug buffer setup and heapless sample storage
xtask             `cargo xtask bundle`, thin wrapper over nice-plug-xtask
../src/backend/embed.rs   the engine's guest path — frost proper, not a copy
```

## Building

Run these from the **workspace root**, and name the package. That is not
ceremony: the plugin is a member but deliberately not a *default* member, so a
bare `cargo check` here builds the engine and its games, says nothing about the
plugin, and passes whether or not this crate compiles.

```sh
cargo check -p nice-tuner                    # plugin lib (cdylib) — CLAP/VST3 body
cargo test -p nice-tuner -p pitch            # the tuner's and the detector's tests
cargo run -p nice-tuner --features standalone --bin nice-tuner-standalone
```

The optional `unsafe` feature is forwarded to our `window-position` wrapper; combining
`--features standalone,unsafe` enables native Windows centering and saved-position
restoration/persistence. It never relaxes the app's unsafe-code guard. Without
that feature, placement is left to baseview and the operating system.

**The standalone binary reads audio settings from `nice-tuner-settings.json`.**
The application owns its CPAL streams or JACK client and keeps the plugin and editor alive when
Apply replaces those streams. An unset backend opens a silent application. The
one exception is `--list-devices [backend] [input] [output]`, which prints one
line of JSON and exits without a window or a stream; it exists so a process that
holds no ASIO driver can enumerate drivers the live app cannot see.

```sh
./target/debug/nice-tuner-standalone --list-devices wasapi
# {"inputs":["Input 1/2 (Komplete Audio 6 MK2)",…],"outputs":[…],"def_in":…,"rates":[…]}
```

Everything else is the panel: **SETTINGS** top-right — backend, sample rate,
buffer size, input and output devices, channel. A mono-only input (the built-in
laptop microphone) is a channel choice there, not a `-l 2` on a command line the
app never hears. The file lands in the current directory as
`nice-tuner-settings.json`; command-line flags used to outrank it, and no longer
exist.

### Linux + JACK

The normal `--features standalone` build includes **Linux JACK** in SETTINGS →
backend, alongside ALSA. JACK is offered only on Linux; Windows retains its
WASAPI/optional ASIO backends and macOS uses Core Audio.

Building on Linux requires JACK development files discoverable through
`pkg-config` (for example, `jack2` on Arch or `libjack-jackd2-dev` on Debian).
Start a JACK server before selecting JACK, or launch with `pw-jack` when using
PipeWire's JACK compatibility library:

```sh
pw-jack ./target/debug/nice-tuner-standalone
```

Select **Linux JACK**, then choose an input buffer size and **Apply**. The tuner
offers trial sizes of **32, 48, 64, 96, 128, 256, 512, and 1024 frames**. JACK
does not enumerate supported alternatives; these are requests, not guarantees.
Apply requests the selected size through JACK's public control API and verifies
that both audio callbacks actually use it. A rejection or a different negotiated
size produces an error and restores the previous audio configuration. The sample
rate continues to follow the server. Input and output buffers share one quantum,
so the output buffer follows the input selection. With no running server, no ports are available
and Apply stays disabled. An unset backend remains silent.

While JACK is streaming, the buffer fields display the actual audio callback
size unless a new size is being selected for Apply. Start with plain `pw-jack`:
an explicit `pw-jack -p`/`PIPEWIRE_QUANTUM` override may compete with the app's
selection; a resulting mismatch is reported as an error.

JACK settings list actual named audio ports from the running graph, including
hardware and other applications. Select **Input source** and **Output left / mono**.
For stereo monitoring, select a second destination under **Output right**; choose
**Mono (no right output)** to use only the first destination. Each JACK port is
mono, so the numeric input channel is fixed to 1. Both selected destinations
receive the full instrument signal at identical amplitude, with no halving or
pan-law attenuation. The tuner connects only the selected ports, without automatic
hardware routing. JACK processes input and both outputs in a single duplex
callback, so it does not need the CPAL path's independent-clock monitor buffer.
Missing or wrong-direction ports and failed connections report an error; a failed
Apply restores the previous routing. Older `JACK Input`/`JACK Output` settings
migrate to the discovered defaults. Full port names are saved even when long
labels are shortened on screen.

### Windows + ASIO

```sh
cargo build -p nice-tuner --features standalone-asio,unsafe --bin nice-tuner-standalone
./target/release/nice-tuner-standalone     # …then SETTINGS → backend: ASIO → device → APPLY
```

Without this feature ASIO is not even a value the binary knows — measured, its
`--list-devices asio` returns `"inputs":[]`. With it, on this machine:
`["FlexASIO","Komplete Audio ASIO Driver","ReaRoute ASIO (x64)"]`, with
`driver_rate` and `buffer_pref` read from the driver itself.

Nothing in this repository configures the ASIO SDK, which is proprietary and
stays out of the tree. cpal's build script finds it in three steps —
`CPAL_ASIO_DIR`, else `<temp>/asio_sdk`, else download it from
<https://www.steinberg.net/asiosdk> — and cpal's own README says the last one
should just work. It does, unless `%TEMP%` sits under a user name with a space:
asio-sys 0.4.0 interpolates `-OutFile <path>` **unquoted** into a PowerShell
command (`build.rs:303-311`), so the path splits and the build dies with
`Failed to download ASIO SDK` behind a `PositionalParameterNotFound`. Measured
twice on one machine with the same folder spelled two ways —
`C:\Users\Per Lindgren\…` fails, `C:\Users\PERLIN~1\…` downloads 8,910,208 bytes
and builds in 33 s. Until that is quoted upstream there are three ways out, and
the one worth taking is neither of the ones that touches a build: **put the SDK
at `%TEMP%\asio_sdk` once** — step two of cpal's lookup, a plain `exists()`
check in Rust with no PowerShell in it, so no quoting bug and no variable to
carry. Measured: with that directory present and `CPAL_ASIO_DIR` unset, the
build script announces `CPAL_ASIO_DIR is set at …\AppData\Local\Temp\asio_sdk`
and gets on with it.

```powershell
# once per machine; asio-sys would have put it there itself
Copy-Item "C:\path\to\asiosdk" "$env:TEMP\asio_sdk" -Recurse
```

Otherwise `setx CPAL_ASIO_DIR C:\path\to\asiosdk` — one variable, no repo change —
or build with a space-free `TMP`/`TEMP`. The engine's
gate never builds this feature, so no CI machine fetches anything either way.

ASIO uses one driver per process and one buffer size per driver. The settings
panel links input and output buffer sizes when they belong to the same driver.
The standalone application's monitoring latency defaults to 5 ms of additional
buffering unless the settings file specifies otherwise.

### Output channels

With an output device selected, **Output channels** offers its physical mono
channels and stereo pairs. CoreAudio's preferred output channels supply the
green default; unsupported or unreported defaults remain unset. Apply validates
the route against the device's supported stream configurations.

Standalone output monitors the selected mono input. A stereo pair receives the
same signal at unity gain on both channels; unselected outputs stay silent.
Input analysis reads the original channels before duplication. Plugin-host mode
retains its existing passthrough behavior. The route is saved as `output_channels`
and audio changes continue to use the live Apply path on CoreAudio.

### Output sample rate

**Input sample rate** and **Output sample rate** query their respective devices.
The output dropdown lists supported output rates and selects the device-reported
default in green. If no default is reported, the value stays unset until chosen;
without an output device, the dropdown is disabled. The output selection is saved
as `output_sample_rate`, with an absent value following the queried default.
Separate devices can use different rates. The plugin runs at the input device's
rate; the application's streaming band-limited resampler converts its output to
the output device's rate. Physical channel routing follows conversion. When both
streams use the same physical device, their selected sample rates must match.

### Independent stream buffers

The settings panel has **Input buffer size** and **Output buffer size** rows,
in frames. Each list is queried from its own device. Different devices can
use different sizes. When input and output are the same CoreAudio device or
ASIO driver, output follows input and its control is gray and disabled.
Changing to separate devices enables the output control again.

The existing JSON `period_size` field now selects the input buffer;
`output_period_size` selects the output buffer. CoreAudio defaults come from
native device queries: the preferred primary input channel and current device
buffer frame size. Failed or unsupported queries stay unset; the UI does not
invent channel 1 or a 512-frame default.

**APPLY** is enabled only when the selected CPAL devices have complete, supported
settings. It replaces the audio streams while keeping the process, plugin and
window alive. A failure leaves the draft unsaved and restores the previous audio
configuration where possible. If recovery fails, the window remains usable with
audio stopped. Ordinary tuning parameters still apply live.

### Standalone audio ownership

The standalone host lives in `src/standalone/`. It calls the same
`TunerPlugin::activate`, `reset` and `process` implementation as the VST3/CLAP
wrappers. CPAL callbacks use preallocated buffers and a single-producer,
single-consumer ring. Output conversion uses a polyphase windowed-sinc filter,
with clock-drift correction and silence during startup or underflow. Same-rate
monitoring uses linear interpolation with one frame of delay.

The GUI sends nonblocking Apply requests to a device-owning worker. The worker
drops the old streams before activating replacements, acknowledges successful
input and output callbacks, and rolls back failed changes. Transient stream
invalidations receive bounded retries.

nice-plug remains the unmodified registry dependency at version 0.4.2; its
standalone feature is not used. There is no vendored copy or Cargo dependency
patch. The application uses only public plugin and editor interfaces. DAW builds
continue to use nice-plug's original plugin wrappers and host-managed audio.

### Producing plugin binaries

```sh
cargo xtask bundle nice-tuner --release   # → target/bundled/  (Nice Tuner.clap / .vst3 on macOS)
```

`xtask` is the `nice-tuner/xtask` member — a call into `nice-plug-xtask` — and
`bundler.toml`, which names the bundle, sits at the **workspace root** because
that is where the tool opens it from. The alias that makes `cargo xtask` work
from anywhere in the tree is in `.cargo/config.toml`, also at the root: a
member's own `.cargo/config.toml` would not be read, since cargo walks *upward*
from the working directory and both the hooks and CI start at the root.

The full verification suite is `nice-tuner/scripts/check.sh` (`check.ps1` on
Windows); the bundling step clears `RUSTFLAGS`, explained in the script and in
`.cargo/config.toml`.

## Notes

- **No vsync**: the GUI surface runs `PresentMode::AutoNoVsync`; plugins must
  never block the host main thread.
- **Real-time safety**: the audio thread does no analysis at all — it
  downmixs on the stack and pushes into a preallocated lock-free
  [rtrb](https://crates.io/crates/rtrb) ring; pitch detection runs on the UI
  thread (frost's frame pump = nice-plug's main thread), gated on the editor
  being open. Gates: `tests/rt_handoff_no_alloc.rs` (zero-alloc, lossless
  handoff) + nice-plug's `assert_process_allocs` on `process()`. When the
  editor is closed even the ring write stops: `process()` is pure passthrough.
- **Debug builds stay smooth**: the detector's window search is bursty and
  runs on the UI frame pump, so the **workspace root's** `Cargo.toml` keeps
  `pitch` at `opt-level = 3` even in the dev profile — the worst burst drops
  from ~2.3 ms to ~0.26 ms and the 60 fps frame loop never visibly stutters.
  Guarded by `crates/pitch/tests/callback_budget.rs`, which measures p99 and
  fails without the pin. A `[profile.dev.package.*]` section in a *member*
  manifest is ignored by cargo, which is why the pin cannot live here.
- **Winit is absent from this plugin's runtime path**: the engine proper keeps
  winit — `run()` is its front door, and `frost::KeyCode` *is*
  `winit::KeyboardEvent`'s — but nothing in `src/` may construct an
  `EventLoop`, which `scripts/check.sh` greps for. The guard used to point at a
  vendored copy of the engine whose whole point was that winit had been deleted;
  that copy is gone, and the rule is now where it belongs: on our own code.

## Settings panel (standalone)

The standalone app's tuner screen has a `SETTINGS` button (top-right) opening a
panel for **backend, sample rate, buffer size, input/output devices and
channel — and the A4/gate params** — device dropdowns are enumerated live via cpal (ASIO appears
only in builds with `--features standalone-asio`; enumeration loads each
registered ASIO driver, so the first open can pause a moment where several are
installed).

- **Persistence**: `APPLY` writes `nice-tuner-settings.json` to the **current
  directory**; it is read again at every startup. CoreAudio stream changes
  are saved after the new stream starts; APPLY stays disabled for incomplete
  or unsupported configurations.
  `RESET` (centre button) restores every default, including window geometry.
- **Window geometry**: the last window position and size are remembered too —
  written ~2 s after the window settles and on Apply, restored at launch; with
  no settings file the window opens centred. Position is Windows-only (the
  window toolkit exposes no position API elsewhere; size still round-trips).
- **Applied directly**: CoreAudio stream changes use a background worker and
  preserve the window. Mono input devices select the mono plugin layout.
  Incomplete saved settings open the settings page with audio stopped.
- **Precedence**: explicit settings > queried device defaults. The standalone
  application ignores audio flags on its command line. Monitoring adds 5 ms of
  buffering unless the settings file specifies otherwise.
- **Inside a DAW there is deliberately no panel** (the footer reads
  `audio: host-managed`): the host owns the audio device; a per-plugin device
  picker would lie.

## Standalone fidelity (what you see embedded)

The standalone binary links the *same* lib as the CLAP/VST3 artifacts — same
`TunerPlugin`, same editor/window code, same param objects, same RT
discipline. What differs is the environment around it, and the panel closes
the one functional gap: **params**. nice-plug gives plugins no way to move
their own `FloatParam`s (the host owns them), so in standalone the settings
page plays the host: the A4/gate pickers write the very same shared atomics
that `process()` mirrors from params in host mode (`process()` skips the
mirror in standalone so your picks survive). Picks are live; `APPLY` persists
them to the settings file, which seeds them at next launch.

| Aspect | Standalone | Embedded |
|---|---|---|
| DSP, editor, GUI, RT path | identical code | identical code |
| Device / backend / sample rate | own process, cpal | host's device & rate |
| Param controls | settings panel (plays host) | host's plugin UI & automation |
| UI pump | own event loop, ~60 fps | host pumps the editor window |
| Editor lifecycle | closing the window ends the app | close/reopen freely, audio unaffected |

The last two rows are environment properties only a real host can prove.
One-time verification checklist in Reaper (load the CLAP from `target/bundled`):

- [ ] A4 Reference + Gate appear in the host's param list; automation works.
- [ ] Close the plugin window while audio runs: no glitch; reopen: meter and
      needle resume (ring park/restore works).
- [ ] Change the project sample rate (44.1 ↔ 96k): the detector rebuilds and
      notes track again within a strum.
- [ ] Resize the embedded window and check DPI scaling; nothing clips or blurs.

## Licensing

- This plugin: MIT OR Apache-2.0, inherited from the workspace in one line
  (`license.workspace = true`), which is also the statement that it is
  engine-side code rather than a game.
- The renderer is the `frost` package at the workspace root — the engine itself,
  `path = ".."`, no copy in between. Its code is MIT OR Apache-2.0, so
  `tuner_ui.rs`, which derives from the engine's UI code, sits on the same terms
  and nothing has to be reconciled by authorship any more.
- **The engine's art is not part of this crate and was never ours to ship**:
  sprites and maps under `assets/` are all rights reserved. The plugin takes one
  file out of `assets/` — the OFL-licensed Fira Code — and OFL permits that
  knowingly, on the condition that the licence text travels with the font. It
  does now: `cargo xtask bundle` writes that file into the VST3 bundle at
  `Contents/Resources/OFL.txt`, and a copy sits beside the bundles for the CLAP,
  which is a single file by design and so cannot carry one — `scripts/check.sh`
  fails the build if either copy is missing or is not the tree's own byte for
  byte. Whoever builds an archive out of a `.clap` still has to take the text
  along; nothing in a plugin binary can do that part.
- Third-party: nice-plug (ISC), vst3 (ISC), baseview (MIT/Apache-2.0). No
  Steinberg VST3 SDK is used anywhere — the VST3 surface is pure Rust. The ASIO
  SDK, for a developer building `standalone-asio`, is proprietary and stays
  outside the repository.

## Status

- [x] M1 plugin skeleton (baseview window, wgpu surface, host adapters)
- [x] M2 pitch detector (`crates/pitch`) — 17/17 tests green
- [x] M3 frost embedded engine (`frost::embedded::Embedded`) — 313 fork tests green,
      winit event loop grep-gated out
- [x] M4 baseview→frost adapter (`src/gui.rs`) + tuner scene (`src/tuner_ui.rs`)
- [x] M5 DSP→GUI wiring (analysis gated on editor open; shared atomics, zero RT allocs)
- [x] M6 `scripts/check.sh` all gates green, incl. `Nice Tuner.clap`/`.vst3` bundling
- [x] Standalone settings panel: backend/device/buffer dropdowns, JSON in CWD,
      application-owned CPAL streams and live Apply; host-managed audio inside DAWs
- [x] The fork is gone: `frost::embedded` lives in the engine, which this crate
      now depends on by path. M3's "313 fork tests" were the fork's copy of the
      engine's suite; the engine's own is larger, and the guest path has tests of
      its own (`src/backend/tests/embed.rs`, three of them, on real hardware) —
      M3's grep gate moved onto this plugin's sources, where it can still fail.

## Upstream improvements we're tracking

- **Device-preferred buffer size in standalone** — nice-plug always opens
  streams at an explicit `--period-size` (default 512) and *errors* on devices
  whose supported buffer range excludes it, instead of falling back to the
  device's preferred size (cpal's `BufferSize::Default` exists and would do
  exactly that). Upstream issue:
  [RustAudio/nice-plug#93](https://codeberg.org/RustAudio/nice-plug/issues/93)
  (open, `good first issue`). Workaround here: the buffer size is explicit on
  the command line (`-p`), which is what the tuner wants anyway; revisit this
  note if the issue lands upstream.

## Development

Full gate for this plugin alone (fmt, clippy `-D warnings` on every feature
combination, the Windows cross-check, tests, the standalone link, bundling):
`bash nice-tuner/scripts/check.sh`, from the workspace root — `check.ps1` where
msys bash cannot start.

The repository's own hooks run on every push and cover the plugin too:
`cargo fmt --all` reaches every member, and `.githooks/pre-push` plus
`.github/workflows/gate.yml` name `-p nice-tuner -p pitch` explicitly, because a
bare `cargo test` in this workspace means the engine and its games and would
pass without ever compiling this crate.
