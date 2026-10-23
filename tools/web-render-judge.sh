#!/usr/bin/env bash
# git bisect's judge: build the web port at this commit and say whether the
# browser shreds it.
#
# Run from the checked-out tree by `git bisect run`; serve the tree's own
# web/ on $WEB_JUDGE_PORT (8083 by default) first, e.g.
# `python3 -m http.server 8083 --directory web`. It builds the minwin
# example for the browser — the smallest thing in the repo that draws a menu
# the desktop gets right — serves it from the tree's own web/ over :8083,
# photographs it with the headless browser, and measures the signature the
# breakage actually has: energy at a period of exactly 16 pixels along the
# column axis, against the neighbouring periods. Known-good frames sit at
# 0.41, the broken ones at 2.10, so the line is 1.0.
#
# Exits 0 good, 1 broken, 125 when this commit cannot be judged at all —
# no example, a wasm-bindgen too old for the crate, a canvas too small to
# measure — so bisect steps over the commits that cannot answer.
set -uo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
ROOT=$(git rev-parse --show-toplevel)
OUT="$ROOT/target"
PORT=${WEB_JUDGE_PORT:-8083}

[ -f examples/minwin.rs ] || { echo "  skip: no minwin example here"; exit 125; }
cargo build --example minwin --target wasm32-unknown-unknown --release >/dev/null 2>&1 \
  || { echo "  skip: wasm build failed"; exit 125; }
mkdir -p web/min
wasm-bindgen --target web --out-dir web/min/pkg \
  target/wasm32-unknown-unknown/release/examples/minwin.wasm >/dev/null 2>&1 \
  || { echo "  skip: wasm-bindgen refused"; exit 125; }

cat > web/min/index.html <<'HTML'
<!doctype html>
<html lang="en"><head><meta charset="utf-8" /><title>judge</title>
<style>html, body { margin: 0; height: 100%; background: #000; overflow: hidden; }</style>
</head><body>
<script type="module">
  import init from "./pkg/minwin.js";
  await init();
</script>
</body></html>
HTML

rm -rf "$OUT/chrome-bisect"
timeout 180 chromium --headless=new --no-sandbox --disable-dev-shm-usage \
  --ignore-gpu-blocklist --enable-features=Vulkan --use-angle=vulkan \
  --window-size=1280,800 --virtual-time-budget=25000 \
  --user-data-dir="$OUT/chrome-bisect" --screenshot="$OUT/judge.png" \
  "http://localhost:${PORT}/min/" >/dev/null 2>&1 \
  || { echo "  skip: capture failed"; exit 125; }

python3 - "$OUT/judge.png" <<'PY'
import sys
import numpy as np
from PIL import Image

a = np.asarray(Image.open(sys.argv[1]).convert("RGB"), dtype=np.float64)
lum = a.mean(2)
lit = lum > 24
if lit.sum() == 0:
    print("  verdict: skip — nothing rendered"); sys.exit(125)
ys, xs = np.nonzero(lit)
w, h = xs.max() - xs.min() + 1, ys.max() - ys.min() + 1
if w < 400 or h < 300:
    print(f"  verdict: skip — content only {w}x{h}"); sys.exit(125)
band = lum[ys.min():ys.max(), xs.min():xs.max()]
sig = band.mean(0)
sig -= sig.mean()
f = np.abs(np.fft.rfft(sig)) ** 2
def at(p): return f[int(round(len(sig) / p))]
base = np.mean([at(p) for p in (13, 14, 15, 17, 18, 19)]) or 1.0
ratio = at(16) / base
bad = ratio > 1.0
print(f"  verdict: {'BROKEN' if bad else 'clean'} at content {w}x{h}, period-16 ratio {ratio:.2f}")
sys.exit(1 if bad else 0)
PY
