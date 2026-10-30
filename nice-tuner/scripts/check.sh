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
# Inert on non-Windows (cpal gates its ASIO host by target); real there. cpal's
# build script finds the Steinberg SDK itself — CPAL_ASIO_DIR, then
# <temp>/asio_sdk, then a download — but the download step passes an unquoted
# path to PowerShell, so a %TEMP% under a user name with a space needs
# CPAL_ASIO_DIR set by the developer instead. See nice-tuner/README.md.
cargo clippy -p nice-tuner --features standalone-asio --all-targets -- -D warnings

echo "== plugin lib (cdylib) check =="
cargo check -p nice-tuner

echo "== standalone check =="
cargo check -p nice-tuner --features standalone

echo "== windows target typecheck (cfg(windows) code a macOS run never compiles) =="
# `standalone` WITHOUT asio here: ASIO's C++ build script cannot
# cross-compile from macOS. This still type-and-name-checks all the
# Win32 window-placement code, which no macOS pass ever sees.
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
if grep -rn "EventLoop\|ActiveEventLoop\|ApplicationHandler\|WindowAttributes" src --include='*.rs' \
   | grep -v '//' | grep -v '///'; then
  echo "FAIL: nice-tuner drives a winit event loop; it is a guest, see frost::embedded"
  exit 1
fi

echo "== standalone bin build (linker gate) =="
cargo build -p nice-tuner --features standalone --bin nice-tuner-standalone

echo "== plugin bundling =="
cargo xtask bundle nice-tuner --release

echo "ALL GATES PASSED"
