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
| [`macos-frame-sync`](crates/macos-frame-sync/src/lib.rs) | Main-thread display-synchronized editor callbacks | AppKit callback registration, ownership, cancellation and panic containment |

The buffer wrapper uses fixed-capacity `heapless` sample storage, boxed once
during setup to keep its address stable. The call to the plugin's `process`
method is safe and remains in the application.

Each wrapper documents its unsafe invariants and validation in its own
`SAFETY.md`: [CoreAudio](crates/core-audio-device/SAFETY.md),
[window placement](crates/window-position/SAFETY.md) and
[plugin buffers](crates/plugin-buffer/SAFETY.md) and
[macOS frame scheduling](crates/macos-frame-sync/SAFETY.md). Tests cover bounds, error paths,
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
assets/fonts     Fluent System Icons Regular, upstream glyph map and MIT notice
src/main.rs       standalone target (`--features standalone`)
crates/pitch      pure-Rust YIN pitch detector (unit-tested, no deps)
crates/core-audio-device  safe read-only CoreAudio queries
crates/window-position   safe Windows placement, behind the unsafe feature
crates/plugin-buffer     safe nice-plug buffer setup and heapless sample storage
crates/macos-frame-sync  macOS display-synchronized callbacks with scoped ownership
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

Enable frost's editor diagnostics with `--features diag` for plugin builds,
or `--features standalone,diag` for the standalone app:

```sh
cargo run -p nice-tuner --features standalone,diag
cargo xtask bundle nice-tuner --release --features diag
```

The overlay shows editor FPS, frame intervals, processing times and rendering
statistics. It receives the actual frame delta before tuner animation clamping
and stays active on the settings page as well. These are GUI measurements, not
DAW audio callback timing. The VSync readout and Alt-V shortcut are disabled
because the embedded renderer controls presentation independently of that flag.
The feature is disabled by default.

### Editor frame scheduling

On macOS 14 and newer, standalone and DAW-hosted editors submit frames from
AppKit's view-bound display callbacks through our `macos-frame-sync` wrapper.
The cadence follows the display containing the editor. A fixed 60 Hz display
provides a 60 Hz scheduling source; missed deadlines and OS scheduling can still
reduce the observed FPS. No precise FPS guarantee is implied.

The display callback runs on the main UI thread and pauses when the view is
hidden or off-screen. Baseview continues to own the window and input events;
its internal 15 ms timer remains installed but performs no rendering in this
mode. The renderer keeps non-VSync presentation to avoid deliberately waiting
for a refresh in a DAW's UI callback.

On older macOS versions, or if display callback setup fails, the editor falls
back to baseview's existing frame timer. Other platforms retain their original
scheduling. Startup logs state which scheduler was selected. The `diag` feature
measures actual editor frame intervals with either scheduler.

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

For a synthetic input instead of microphone samples:

```sh
cargo run -p nice-tuner --release --features standalone -- --test_freq 440.0
```

`--test_freq <Hz>` (also `--test_freq=<Hz>`) replaces every input channel with
an amplitude-1.0 sine before normal plugin processing. Its phase is continuous
across callbacks. It uses the configured input stream's sample rate and timing,
so select/apply an audio backend as usual. The value must be finite, positive
and below half the input sample rate. It lasts for this launch, including live
Apply changes, and is not saved in the settings file.

To compare GUI pacing with pitch detection disabled, add `--note_freq <Hz>`:

```sh
cargo run -p nice-tuner --release --features standalone,diag -- --test_freq 440 --note_freq 440
```

`--note_freq` supplies the detected frequency directly, without constructing or
running the pitch detector. Audio capture, metering, waveform history, the
A-tuned scope and normal rendering continue. It can be used with real input or
with `--test_freq`; both options also accept `=<Hz>` and are independent.
The supplied frequency must be finite and positive. Input-level gating remains
active, so silence still hides the reading. This is a launch-only diagnostic
override, not a saved tuning setting.

Audio configuration is in the panel: the **settings cogwheel** top-right — backend, sample rate,
buffer size, input and output devices, channel. A mono-only input (the built-in
laptop microphone) is a channel choice there, not a `-l 2` on a command line the
app never hears. The file lands in the current directory as
`nice-tuner-settings.json`; the test-tone option does not override those device
settings.

### Linux + JACK

The normal `--features standalone` build includes **Linux JACK** under the settings cogwheel →
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
./target/release/nice-tuner-standalone     # …then settings cogwheel → backend: ASIO → device → APPLY
```

Without this feature ASIO is not even a value the binary knows — measured, its
`--list-devices asio` returns `"inputs":[]`. With it the list is every driver
registered under `HKLM\SOFTWARE\ASIO`, read **without loading one**: nine names
on this machine, `["Audio 8 DJ","DJM-250MK2 ASIO","FlexASIO","Focusrite
Thunderbolt ASIO","Focusrite USB ASIO","Komplete Audio ASIO Driver","MOTU M
Series","NUX Audio","ReaRoute ASIO (x64)"]`. Six of those nine have no hardware
connected and report no channels; they are listed anyway, and asking one yields
an unknown rather than a guess. `driver_rate` and `buffer_pref` are read from
the driver itself, for the driver named on the command line.

One row is narrower than it looks: **Input channel** lists one entry per physical
input, but cpal's ASIO host binds a stream's buffers to the first *n* ports, so only
channel 1 and channel 2 lead anywhere and the rest fail at APPLY.
[docs/asio-host-review.md](docs/asio-host-review.md) records why, and what would
make the rest real. The row is a property of the stream, not a live control like
the A4 reference: picking a channel changes nothing until APPLY, which reopens the
stream with that many input ports.

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

The standalone app's tuner screen has a settings cogwheel (top-right) opening a
panel for **backend, sample rate, buffer size, input/output devices and
channel — and the A4/gate params** — device dropdowns are enumerated live (ASIO
appears only in builds with `--features standalone-asio`). The ASIO rows are the
registry's driver names, which loads nothing at all; channels, sample rate and
buffer sizes come from `ASIOGetChannels`, `ASIOGetSampleRate`,
`ASIOGetBufferSize` and `ASIOCanSampleRate` on **the driver the user picked**,
asked through `asio-sys` rather than through cpal — whose enumeration loads
every registered driver to build its list, which costs seconds on a machine
whose audio gear left a dozen vendor drivers behind. Measured on one with nine
still registered: a settings snapshot on ASIO with no driver chosen was 1785 ms,
now **159 ms**, against 79 ms for the same binary on the dummy backend, which is
the process's own start. With a driver chosen it is **332 ms**, one load of that
driver and nothing else — `snapshot` gathers its answers once and every question
below asks the gathered copy, so a driver whose hardware is off is refused once
per pass rather than once per row (206 ms for that). Opening the stream goes
through cpal, which enumerates once at that moment.

- **One ASIO driver**: selecting ASIO pins the output device row to the input
  driver — it shows that name and is gray, because a second driver cannot be
  loaded in this process to be anything else — and that driver's single buffer
  size and single sample rate govern both directions, so the output buffer and
  output rate rows follow the input ones and are gray. Resolving the stream says
  the same: an output rate left behind by an older settings file is replaced by
  the driver's rate rather than refused at Apply. Core Audio shares only the
  buffer rule, when input and output are the same device; its independent output
  rate is what the output rate row exists to set.
- **Persistence**: `APPLY` writes `nice-tuner-settings.json` to the **current
  directory** after audio changes succeed, then returns directly to the tuner.
  Audio or file-save errors keep the settings panel open. `BACK` and Escape
  discard draft preferences without saving; `RESET` selects draft defaults
  that are saved only by Apply. Back and Reset wait while an Apply is pending.
- **Window geometry**: the current size is saved immediately on resize on every
  platform and restored at launch. Geometry updates merge only the window fields,
  preserving saved preferences and never committing drafts from the settings panel.
  A new installation can therefore have a geometry-only config while audio stays
  unconfigured. Known position is preserved too; native position tracking is
  Windows-only with the optional placement wrapper. Reset leaves geometry alone.
- **Applied directly**: CoreAudio stream changes use a background worker and
  preserve the window. Mono input devices select the mono plugin layout.
  Incomplete saved settings open the settings page with audio stopped.
- **Precedence**: explicit settings > queried device defaults. The standalone
  application ignores audio flags on its command line. Monitoring adds 5 ms of
  buffering unless the settings file specifies otherwise.
- **Inside a DAW**, the cogwheel opens settings with editable A4 reference,
  input gate, in-tune window, and gauge curve. Changes take effect immediately
  through host parameters, support automation, and are saved with the DAW project.
  `DONE` or `BACK` closes the panel; `RESET` restores these four host parameters.
  No standalone settings file is read or written. Host-controlled audio fields
  show the active input/output bus counts, sample rate, and block size, and are
  disabled. Before the first audio callback, the block field shows the host's
  configured maximum. Physical device names are not exposed by the host.
  The main screen also shows the input/output counts and sample rate.

## Standalone fidelity (what you see embedded)

The standalone binary links the *same* lib as the CLAP/VST3 artifacts — same
`TunerPlugin`, same editor/window code, same param objects, same RT
discipline. What differs is the environment around it. In a plugin, the settings
page changes parameters through nice-plug's host parameter API. In standalone,
the A4/gate pickers write the very same shared atomics
that `process()` mirrors from params in host mode (`process()` skips the
mirror in standalone so your picks survive). Picks are live; `APPLY` persists
them to the settings file, which seeds them at next launch.

| Aspect | Standalone | Embedded |
|---|---|---|
| DSP, editor, GUI, RT path | identical code | identical code |
| Device / backend / sample rate | own process, cpal | host's device & rate |
| Param controls | settings panel, saved to JSON | settings panel & host automation, saved by DAW |
| UI pump | display callbacks on macOS 14+, otherwise baseview timer | same scheduler on the host UI thread |
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
- Fluent System Icons Regular is embedded from this member's local
  [`assets/fonts`](assets/fonts/README.md), under Microsoft's MIT licence.
  `cargo xtask bundle` ships `LICENSE-FluentSystemIcons.txt` inside VST3 resources
  and beside CLAP files. Standalone packages must carry this notice and Fira
  Code's `OFL.txt` too.
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
