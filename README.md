# FROST

A game engine experiment by N65 Game Research Station (N65 GRS).

Why?

- Primarily to gain experience of local hosted AI workflows.
- Sufficiently complex problem to challenge ability to reason on realistic use-cases.
- To gain human experience on inner workings of a games engine, starting from very limited, surface level understanding.
- If successful to provide the core mechanics of a library based games engine, following the Rust paradigms (unlike e.g., Bevy, that introduces an additional ECS abstraction to offer shared mutability.)

See [SETUP.md](SETUP.md) for AI based workflow.

## Examples

Run the examples with:

```shell
RUST_LOG=info cargo run --example gizmos
```

or under Win11 with Powershell:

```powershell
$env:RUST_LOG="info"; cargo run --example gizmos
```

## Plugins and the rack

Nine cards live in this workspace — `nice-tuner`, `nice-nam`, `nice-wah`,
`nice-phase`, `nice-pi-face`, `nice-echo`, `nice-ecomplex`, `nice-clone` and `nice-dist` —
and each one is a single editor and `process` implementation in two shapes: a
plugin, built from the crate's `cdylib`, and a standalone application, built
from its `src/main.rs`.

`nice-dist` combines TS808, TS9, OD-1, SD-1, RAT and Nice Clone models.
The separate `nice-clone` plugin remains available; both share its Centaur DSP
implementation in `nice-common`.

The `RUSTFLAGS` in the commands below is an instruction-set baseline and not
decoration: NeuralAmpModeler-rs wants AVX2, so `nice-nam` has no working models
without it. It is spelled out per command rather than put into
[`.cargo/config.toml`](.cargo/config.toml) on purpose — an entry there raises the
floor for every member of the workspace, `antifrost` included, which is a release
decision and not a build convenience. An Apple Silicon Mac has nothing to set,
because the flag names an x86-64 target and the NAM engine does not support Intel
Macs at all. It is an x86-64 spelling only: export it across the workspace and
the `wasm32-unknown-unknown` half of a lint answers one line of *"not a recognized
processor"* per crate and carries on without it, which is why the gate sets it on
the three steps that build the model plugin or share its artefacts and nowhere
else. On Windows, setting the variable *replaces* the `+crt-static` that
`.cargo/config.toml` asks for on MSVC — the environment wins over `rustflags`
there, as that file says — so name both if you want both, and read its note on
why a bundle meant for somebody's DAW is better built without the static CRT.

### Bundled plugins

Under Linux, the command carries the baseline:

```shell
RUSTFLAGS="-Ctarget-cpu=x86-64-v3" cargo xtask bundle nice-echo --release
```

or under Win11 with Powershell:

```powershell
$env:RUSTFLAGS="-Ctarget-cpu=x86-64-v3"; cargo xtask bundle nice-echo --release
```

Several at once takes `-p` per package, there being no `all`:

```shell
RUSTFLAGS="-Ctarget-cpu=x86-64-v3" cargo xtask bundle \
  -p nice-tuner -p nice-nam -p nice-wah -p nice-phase \
  -p nice-pi-face -p nice-echo -p nice-ecomplex \
  -p nice-clone -p nice-dist --release
```

Bundles land in `target/bundled/` under the display name from
[`bundler.toml`](bundler.toml) — `nice-echo` becomes `Nice Echo`, `nice-pi-face`
becomes `Nice Pi-Fase` — and each package yields both `Nice Echo.clap` and
`Nice Echo.vst3`, because these crates export both entry points. A CLAP is a
single file on Linux and Windows and a `.clap/Contents/MacOS/…` bundle on macOS;
the VST3 is a directory on all three. `cargo xtask` is the alias in
[`.cargo/config.toml`](.cargo/config.toml) for the driver in `nice-tuner/xtask`,
which also puts the two font licence texts beside the bundles: a CLAP is one file
and cannot carry the OFL text for the embedded Fira Code, so it travels next to
it instead.

macOS wants none of the `RUSTFLAGS` prefix — the flag names an x86-64 target,
and on Apple Silicon it names nothing — so the bundle command is bare. The one
extra subcommand carries both Apple instruction sets in one binary, which is
what a bundle leaving your machine wants:

```shell
cargo xtask bundle nice-echo --release
rustup target add x86_64-apple-darwin aarch64-apple-darwin
cargo xtask bundle-universal nice-echo --release
```

The plain build is for the machine it runs on. The universal one wants both
targets installed — the second line, once — and joins its two legs with `lipo`
from the Xcode command line tools. Each package is built twice, serially, into
`target/x86_64-apple-darwin/release` and `target/aarch64-apple-darwin/release`,
so expect roughly twice the time, and `--target` is refused out of hand because
the subcommand supplies it itself twice; everything else passes through to both
legs. `nice-nam` is the one card to leave out of a universal build: the x86-64
leg builds nothing that works ([nice-nam/README.md](nice-nam/README.md)); bundle
it plain, on arm64. Every macOS bundle is finished the way Apple wants one — an
`Info.plist` and a `PkgInfo` in `Contents/`, then a `codesign -f -s -` over the
directory, because arm64 macOS sometimes refuses an unsigned self-built plugin
and a missing `codesign` is only a warning.

### Standalone plugins

```shell
RUSTFLAGS="-Ctarget-cpu=x86-64-v3" cargo build -p nice-echo --release --features standalone
```

or under Win11 with Powershell:

```powershell
$env:RUSTFLAGS="-Ctarget-cpu=x86-64-v3"; cargo build -p nice-echo --release --features standalone
```

The executable is `target/release/nice-echo-standalone`, `.exe` on Windows, and it
owns its own audio device — CPAL, JACK on Linux, ASIO as well on Windows — where
the plugin is driven by a host. `--list-devices` prints the device snapshot as RON
and exits. The feature is required rather than advisory: the `[[bin]]` target
declares `required-features`, so a `cargo build --release` without it compiles the
plugin and says nothing whatever about the application. The same command leaves
the raw shared object beside the binary — `libnice_echo.so`, `libnice_echo.dylib`,
`nice_echo.dll` — which is what the bundler renames. Bundled on macOS the
application is not a loose file either: build it with the `standalone` feature
and `cargo xtask bundle` wraps it in `Nice Echo.app`, because that is where a Mac
expects an application to live; plain `cargo run` needs no such thing.

### The rack

```shell
cargo run -p nice-rack-standalone --release
```

or under Win11 with Powershell:

```powershell
cargo run -p nice-rack-standalone --release
```

The package is `nice-rack-standalone`, the executable it builds is `nice-rack`,
and it wants no prefix: it compiles at the default baseline, and each card brings
its own code generation with it. It also wants no plugin path and no scan
directory, because cards arrive by drag and drop — drop a `Nice Echo.clap` out of
`target/bundled/` onto the window. [nice-rack/README.md](nice-rack/README.md) is
about the view, the routing and the editors.

## Repository hooks

The versioned hooks in `.githooks/` gate commits (fmt + clippy) and pushes (the
test suite, plus a `wasm32-unknown-unknown` check — every other gate compiles the
host target only, so a `cfg`-gated import in the backend walks through all of them
and breaks `web/build.sh` later; that has happened).

The suite runs on every push that could change its answer, `main` included, and a
clone missing the wasm target is refused rather than warned, with the remedy
printed beside the refusal:

```shell
rustup target add wasm32-unknown-unknown
```

Three pushes are skipped, each with its reason printed rather than silence:
a deletion, which ships no objects; a tree the last run already passed, keyed on
the pushed trees, `rustc -V` and `cargo -V`, a checksum of the hook itself,
`FROST_TELL_PROBE_UNAVAILABLE` and the graphics driver version — because a green
recorded with the tell probe suppressed, or before a driver update, is not the
same claim (`docs/benchmarking.md` §37 is about measurements moving while nobody
changed the code); and a diff whose every file is outside anything the suite can
read — `docs/`, `*.md`, `.github/`, the licence texts. That last list is checked
against the tree rather than trusted: every `.rs` file in the index is scanned for
an `include_bytes!` or `include_str!` naming one of those paths, and if any does,
the list is void. `FROST_GATE_FORCE=1` insists on a full run whatever the hook
concludes.

A dirty working tree earns nothing. `cargo test` tests the directory, not the
commit, so a push from a dirty tree runs the suite, passes, and records no verdict
— otherwise the commit would be credited with work it does not contain.

None of this is a hole, and the reason is worth stating next to the skipping: the
runner is not governed by the local one. Whatever the hook decides, `gate` builds,
lints, packages and compiles for the web without condition, and it runs every test
suite the pushed change can reach — everything, whenever its `plan` job cannot see
far enough to say otherwise. `main` will not move until that has passed. A local
skip decides where the first run happens, never whether one happens.

There is CI behind this repository now, and it is the reason the hooks stopped
being the last line. `.github/workflows/gate.yml` runs the same rules on a
runner — the dropper scan, `fmt`, clippy over every target, the packaging
assertions, the web build, and the test suites a change can reach, with the
`plan` job writing which ones and why into the run's summary. The `main-gate`
ruleset requires that check before `main` may move. Each change is gated once
on its way in, as a pull request, and once more as the commit `main` becomes:
`push` is scoped to `main`, so a branch with a pull request open is not gated
twice on one commit. A hook cannot be any of this: the hooks live in per-clone
config and `--no-verify` walks past them, while the server refuses whatever it
has not seen pass. Refusing is not theoretical:

```text
remote: error: GH013: Repository rule violations found for refs/heads/main.
remote: - Required status check "gate" is failing.
! [remote rejected] ci-refusal-probe -> main (push declined due to repository rule violations)
```

which is what pushing a commit whose gate had failed printed, verbatim. The
consequence for anyone working here: `main` takes a commit only after that commit
passed on another ref, so work arrives by branch.

Two absences are worth knowing before relying on it. It has no display and no
audio device, so the GPU test tree reports without judging: a runner with no
framebuffer cannot ground a pixel assertion, and a measurement that cannot be
taken must refuse rather than come back green (`docs/benchmarking.md` §37). The
same absence rules out anything that needs a real input or output, and those
tests are named and skipped rather than counted as passed. What it does
cover is both platforms this engine ships on — `ubuntu-24.04` and
`windows-2025`, both starting on every change — so the Windows transport and
the Win32 arms of the CLAP editor embedding are compiled on every change, not
only on whichever machine is developing them.

Git runs the hooks only once this clone points at them — the trigger is a
per-clone setting, never cloned itself. After every fresh clone, once:

```shell
sh .githooks/install.sh
```

(or clone with `git clone --config core.hooksPath=.githooks <url>` and skip
the step). On Windows from PowerShell or cmd, where `sh` is usually not on
the PATH, the script's whole job is the one command:
`git config core.hooksPath .githooks` — run it from any shell (from Git
Bash, `sh .githooks/install.sh` works as everywhere). They are a gate you
opt into, not a wall: `--no-verify` skips them by design. What it skips is the
local half — a commit that skips its hooks still has to pass `gate` somewhere
else before `main` will accept it.

## License

The engine is free: `frost`, `frost_shaders` and the tools under `tools/` are
MIT OR Apache-2.0, the same terms as the Rust project itself. The full texts are
in [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE), and
[assets/fonts/OFL.txt](assets/fonts/OFL.txt) carries the SIL Open Font License
under which Fira Code is redistributed.

What is built *with* the engine is not free in the same way. `antifrost/` and
`immortal/` are the games, and they are under no licence: all rights reserved,
deliberately not inheriting the workspace terms, so "the engine is open" and "a
game built on it is open" are separate decisions. `immortal/` was an example
*target* of the engine's own package until 2026-10-29, which meant the only
licence statement anywhere near it was a claim about a crate it was not; as a
package it names no `license`, and naming nothing is how a manifest says all
rights reserved. The art follows the same split — 78 files live in
`immortal/assets/`, the ten the engine's examples draw as well stayed at the
repository root, and both homes are addressed by the same keys.

[docs/licensing.md](docs/licensing.md) is the ledger: which package carries
which terms, which assets the engine shares with the games, and what is still
unresolved — provenance for three fonts and every sound file, what is left of
the asset tree's partition, and the packaging a real `cargo publish` would want.