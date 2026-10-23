#!/usr/bin/env bash
# Build antifrost for the browser. From the repo root or web/:
#   ./web/build.sh
# (On Windows, run under Git Bash, where bash, GNU sed and sha256sum
# live.)
# then serve the page:
#   python3 -m http.server 8080 --directory web
# and open http://localhost:8080 (needs a WebGPU browser: Chrome/Edge
# 113+, Firefox 141+, Safari 26+).
#
# The wasm-bindgen CLI must exactly match the wasm-bindgen crate in
# Cargo.lock; the script now reads the pin from the lock and refuses the
# build when the machine is missing the CLI or carries the wrong version,
# printing the install line either way.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

# Two things must be installed before a browser build exists, and this
# script used to learn about either of them minutes into a release
# compile: the wasm std, and a bundler whose version must match the
# crate's exactly — a wasm-bindgen CLI off by a point emits JS glue the
# wasm refuses to load, and the browser answers with a link error rather
# than a version number. So ask first, and print the remedy with the
# refusal. The pin is read from the lock rather than repeated here, so
# bumping wasm-bindgen cannot leave this check guarding the old number.
want=$(awk -F'"' '/^name = "wasm-bindgen"$/{getline; print $2; exit}' Cargo.lock)
if ! command -v wasm-bindgen >/dev/null 2>&1; then
    {
        echo "build.sh: no wasm-bindgen on PATH — the bundler is a build dependency:"
        echo "            cargo install wasm-bindgen-cli --version $want"
    } >&2
    exit 1
fi
have=$(wasm-bindgen --version | awk '{print $NF}')
if [ "$have" != "$want" ]; then
    {
        echo "build.sh: wasm-bindgen $have, but Cargo.lock pins $want."
        echo "          The glue must match the crate exactly:"
        echo "            cargo install wasm-bindgen-cli --version $want"
    } >&2
    exit 1
fi
if command -v rustup >/dev/null 2>&1 &&
   ! rustup target list --installed | grep -qx wasm32-unknown-unknown; then
    {
        echo "build.sh: the wasm32-unknown-unknown target is not installed:"
        echo "            rustup target add wasm32-unknown-unknown"
    } >&2
    exit 1
fi

cargo build -p antifrost --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir web/pkg \
    target/wasm32-unknown-unknown/release/antifrost.wasm
# Stamp index.html's module URLs with the new wasm's hash: browsers
# cache the wasm at its fixed URL and a page reload never refetches
# it, so every build must arrive under a fresh query string.
#
# The two tools below dodge their dialects rather than detect them.
# The digest: `sha256sum` is coreutils (Linux, Windows Git Bash),
# `shasum -a 256` rides macOS by default — one of the two lives on
# every machine this script runs on, so the script asks which. The
# stamp: `sed -i` is the one flag BSD and GNU spell differently —
# BSD's demands a backup-suffix argument, GNU's takes it glued —
# and each reads the other's spelling as a filename, so the rewrite
# lands through a temp file instead, the one in-place edit every
# sed has always agreed on.
if command -v sha256sum >/dev/null 2>&1; then
    v=$(sha256sum web/pkg/antifrost_bg.wasm | cut -c1-8)
else
    v=$(shasum -a 256 web/pkg/antifrost_bg.wasm | cut -c1-8)
fi
sfx=$(mktemp "${TMPDIR:-/tmp}/frost-index.XXXXXX")
sed -E 's/\?v=[0-9a-zA-Z]+/?v='"$v"'/g; s/build [0-9a-zA-Z]+/build '"$v"'/g' \
    web/index.html > "$sfx" && mv "$sfx" web/index.html
echo "built: web/pkg/antifrost_bg.wasm — serve web/ and open http://localhost:8080 (build $v)"
