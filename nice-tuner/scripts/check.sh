#!/usr/bin/env bash
# The plugin's own gate, run from the workspace root (`scripts/` sits one level
# below it now that nice-tuner is a member beside the engine).
#
# Every cargo step is scoped with `-p`. That is not tidiness: `--workspace`
# here would drag in the engine's 460-test GPU tree and every game, which is the
# engine's own gate's work (`.githooks/pre-push`, `.github/workflows/gate.yml`)
# and costs ten times as much. What belongs to this script is what only this
# script can see: the plugin's feature combinations, the standalone binary's
# link, and a bundle.
#
# RUSTFLAGS is cleared because the workspace root forces `-C
# target-feature=+crt-static` for MSVC — the engine's choice, for a game that
# ships as one file with no VC redist to install. A VST3 loaded into somebody
# else's process is the wrong shape for a static CRT (see the note in
# `.cargo/config.toml`), and this script builds the plugin, not the engine.
set -euo pipefail
cd "$(dirname "$0")/../.."
export RUSTFLAGS=""

echo "== cargo fmt =="
cargo fmt --all -- --check

echo "== clippy (the plugin's own configs, deny warnings) =="
cargo clippy -p nice-tuner -p pitch -p xtask --all-targets -- -D warnings
cargo clippy -p nice-tuner --features standalone --all-targets -- -D warnings
# No second feature set follows: ASIO arrives with `standalone` on Windows and
# nowhere else, through the optional `asio-host` carrier (see the feature note
# in nice-tuner/Cargo.toml), so there is nothing left to add to the line above.
# The `--features standalone-asio` step that stood here asked for a feature the
# manifest dropped in 3130e2e, so this script stopped at it on every platform:
# `cargo clippy -p nice-tuner --features standalone-asio --all-targets` answers
# "error: the package 'nice-tuner' does not contain this feature:
# standalone-asio" with exit code 101, before compiling anything.

echo "== plugin lib (cdylib) check =="
cargo check -p nice-tuner

echo "== standalone check =="
cargo check -p nice-tuner --features standalone

echo "== windows target typecheck (cfg(windows) code a macOS run never compiles) =="
# What this step is for: the Win32 window-placement code and the ASIO transport,
# neither of which a macOS pass ever compiles. The C++ build script that used to
# make this impossible for the ASIO path is gone — `azo` declares the interfaces
# by hand and builds nothing — so nothing about ASIO holds this step back any
# more. Whether it now reaches that code from a macOS machine has not been
# measured here; the workspace's Windows coverage runs on a Windows runner, in
# the `windows` job of `.github/workflows/gate.yml`.
if rustup target list --installed 2>/dev/null | grep -q x86_64-pc-windows-msvc; then
  cargo clippy -p nice-tuner --features standalone --target x86_64-pc-windows-msvc -- -D warnings
else
  echo "skipped: rustup target add x86_64-pc-windows-msvc"
fi

echo "== tests =="
cargo test -p nice-tuner -p pitch

echo "== no event loop of ours, anywhere in the plugin =="
# This grep used to run over a vendored copy of the engine whose entire point
# was that winit had been cut out of it. The engine proper keeps winit — `run()`
# is its front door — so the guard moved to the code that is actually ours: a
# plugin that builds its own event loop inside a host's process is the one
# mistake this design cannot survive, and it would be a quiet one.
#
# The path is qualified because this script runs at the workspace root, where a
# bare `src` is the engine's — and the engine's answers that grep with nineteen
# lines across four files, so the guard failed on someone else's code and never
# reached ours. `check.ps1` already read `nice-tuner\src`.
if grep -rn "EventLoop\|ActiveEventLoop\|ApplicationHandler\|WindowAttributes" nice-tuner/src --include='*.rs' \
   | grep -v '//' | grep -v '///'; then
  echo "FAIL: nice-tuner drives a winit event loop; it is a guest, see frost::embedded"
  exit 1
fi

echo "== standalone bin build (linker gate) =="
cargo build -p nice-tuner --features standalone --bin nice-tuner-standalone

echo "== plugin bundling =="
cargo xtask bundle nice-tuner --release

echo "== the font licence travels with the bundle =="
# The editor compiles Fira Code in (`src/gui.rs`), and OFL 1.1 permits embedding on
# the condition that the licence text travels with the font. A font that is
# compiled in is a file nobody ever reaches for, and `nice-plug-xtask` has no
# resource step at all, so this assertion is the mechanism that keeps the duty in
# the build rather than in a person's memory. `xtask` writes the text; this proves
# it arrived, arrived in both shapes a bundle has, and arrived as the tree's own
# copy — the file a distributor ships, not a summary of it.
for shipped in \
  "target/bundled/Nice Tuner.vst3/Contents/Resources/OFL.txt" \
  "target/bundled/OFL.txt"; do
  test -f "$shipped" || { echo "FAIL: $shipped is missing — the CLAP is a single file, so its copy is the archive's to carry"; exit 1; }
  cmp -s assets/fonts/OFL.txt "$shipped" || { echo "FAIL: $shipped is not the tree's assets/fonts/OFL.txt"; exit 1; }
done

for shipped in \
  "target/bundled/Nice Tuner.vst3/Contents/Resources/LICENSE-FluentSystemIcons.txt" \
  "target/bundled/LICENSE-FluentSystemIcons.txt"; do
  cmp -s nice-tuner/assets/fonts/LICENSE-FluentSystemIcons.txt "$shipped" || { echo "FAIL: $shipped is missing or differs from the icon font's MIT notice"; exit 1; }
done

echo "ALL GATES PASSED"
