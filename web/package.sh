#!/usr/bin/env bash
# Package the web build for itch.io (or any static host). From the repo
# root or web/:
#   ./web/package.sh
# produces antifrost-web.zip containing:
#   index.html              the page (self-contained; no network peers)
#   pkg/antifrost.js        wasm-bindgen glue
#   pkg/antifrost_bg.wasm   the game
# On itch: new project -> Kind: HTML -> upload the zip -> tick "This
# file will be played in the browser". Requires a WebGPU browser
# (Chrome/Edge 113+, Firefox 141+, Safari 26+): desktop and Android,
# not iPhone yet.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

[ -f web/pkg/antifrost_bg.wasm ] || {
    echo "no build found — run ./web/build.sh first" >&2
    exit 1
}
rm -f antifrost-web.zip
# itch plays the zip's root index.html; the page imports ./pkg/*, so
# the tree goes in as-is. The .d.ts files are type declarations for
# npm users: dead weight here.
staging=$(mktemp -d)
mkdir "$staging/pkg"
cp web/index.html "$staging/index.html"
cp web/pkg/antifrost.js web/pkg/antifrost_bg.wasm "$staging/pkg/"
(cd "$staging" && zip -qr - .) > antifrost-web.zip
rm -rf "$staging"
echo "packed antifrost-web.zip ($(du -h antifrost-web.zip | cut -f1)) — upload it, tick 'playable in browser'"
