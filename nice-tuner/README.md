# Nice Tuner

A stringed-instrument chromatic tuner audio plugin (CLAP + VST3 + standalone),
built on:

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

## Layout

```
src/lib.rs        plugin: TunerPlugin (params, process), TunerEditor (baseview spawn)
src/gui.rs        baseview WindowHandler driving `frost::embedded::Embedded` (surface, inputs)
src/tuner_ui.rs   frost scene: gauge, needle, note readout, input meter (`Process` impl)
src/main.rs       standalone target (`--features standalone`)
crates/pitch      pure-Rust YIN pitch detector (unit-tested, no deps)
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

**The standalone binary does not read its own command line.** `main.rs` hands
nice-plug an argument list synthesized from `nice-tuner-settings.json`
(`settings::effective_args`), so a driver is only ever reached through a value
the settings page wrote — never through a flag, and never by accident of an
unset one (an unset backend is `dummy`, not nice-plug's jack-first `auto`). The
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

### Windows + ASIO

```sh
cargo build -p nice-tuner --features standalone-asio --bin nice-tuner-standalone
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

Two facts about ASIO on Windows that the panel cannot fix: **one driver per
process** (asio-sys loads exactly one, and a refused open is usually the
previous instance or Reaper still releasing it — hence the four startup retries),
and **one buffer size per driver**, so a tuner sharing a driver with a DAW must
be set to the DAW's number. nice-plug's `--input-latency` default of 150 ms is
audibly wrong for a tuner; the standalone asks for 5 ms unless the file says
otherwise.

The `Open error: is JACK server running?` lines some builds print at startup come
from nice-plug probing JACK before the chosen backend; they are stderr noise on
an unused path and say nothing about the device you asked for.

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
  directory**; it is read again at every startup. The button reads
  `APPLY & RESTART` only when a change actually needs new audio streams
  (backend, devices, rate, buffer) — everything else goes live on save.
  `RESET` (centre button) restores every default, including window geometry.
- **Window geometry**: the last window position and size are remembered too —
  written ~2 s after the window settles and on Apply, restored at launch; with
  no settings file the window opens centred. Position is Windows-only (the
  window toolkit exposes no position API elsewhere; size still round-trips).
- **Applied directly** = save, plus an automatic process respawn (~a blink)
  when audio settings changed. The respawn is how nice-plug's standalone
  wrapper reopens the audio stream — a plugin cannot reconfigure it from
  inside (same ownership story as
  [RustAudio/nice-plug#93](https://codeberg.org/RustAudio/nice-plug/issues/93)
  below).
- **Precedence**: command-line flags > settings file > defaults (including our
  5 ms `--input-latency` default).
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
  knowingly, on the condition that the licence text travels with the font. A
  bundle built here carries no licence text at all yet; see `src/gui.rs`.
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
      apply-by-respawn; host-managed honesty inside DAWs
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
