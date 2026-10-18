#!/usr/bin/env bash
# watch.sh — rebuild the web bundle whenever its inputs change.
#
# The iterative loop the itch upload can't offer: edit any Rust or
# WGSL input, save, and seconds later a refresh in any browser on the
# network fetches the new build — serve.py answers no-store, so the
# refresh is the whole protocol, and the stamp build.sh writes into
# index.html is the proof of which build the page is showing.
#
# Nothing under web/ is watched on purpose: build.sh rewrites those
# files itself, and a page edit needs no build at all — no-store means
# the browser sees it on the next refresh by itself.
#
#   ./web/watch.sh          # alongside: python3 web/serve.py 8443 --tls
set -euo pipefail
cd "$(dirname "$0")/.."

STAMP="web/.watch-stamp"
touch "$STAMP"
trap 'rm -f "$STAMP"' EXIT

while :; do
    if [[ -n "$(find src shaders antifrost Cargo.toml Cargo.lock \
                  -newer "$STAMP" -print -quit 2>/dev/null)" ]]; then
        ./web/build.sh
        touch "$STAMP"
    fi
    sleep 1
done
