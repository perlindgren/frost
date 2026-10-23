#!/usr/bin/env python3
"""Compare the *structure* of two frames of antifrost, ignoring registration.

Two frames of this game never line up pixel for pixel across a port: the
web canvas is whatever the page made it, the native window is whatever the
display or the Xvfb screen is, and the camera has had seconds to drift
between them. What does carry across a port is the *character* of a pass —
whether the light field arrives smooth or in 8×8 steps, whether a colour
channel is alive or flat, whether black pixels are scattered or clumped.

So this measures three things that do not need the frames to agree:

  quantisation   the 8×8 tile grid (the engine's `TILE`) against the 4×4
                 subdivision inside each tile. A light field that reached
                 the shader intact varies *within* a tile; one replaced by
                 a per-tile value does not. The ratio of within-tile spread
                 to tile-centre spread is the number that separates them:
                 near zero means the tile is a flat slab.
  spread         the same ratio at 1 pixel, for reference.
  luma histogram how much of the frame is pure black and pure white — a
                 mosaic hoards pixels at the ends, a lit hill does not.

Usage: compare-frames.py LEFT.png RIGHT.png
"""
import sys
from PIL import Image


def load(path, width=512):
    im = Image.open(path).convert("L")
    # Normalise size only; the point is texture, not registration.
    side = min(im.width, im.height)
    im = im.crop(((im.width - side) // 2, (im.height - side) // 2,
                  (im.width - side) // 2 + side, (im.height - side) // 2 + side))
    im = im.resize((width, width), Image.LANCZOS)
    return im, list(im.getdata()), width


def stats(name, path):
    im, px, w = load(path)
    if w % 8:
        w -= w % 8
    tile_spread, inner_spread = [], []
    black = white = 0
    for y in range(0, w, 8):
        for x in range(0, w, 8):
            tile = [px[(y + j) * w + x + i] for j in range(8) for i in range(8)]
            mean = sum(tile) / len(tile)
            tile_spread.append(mean)
            # Variance inside the tile versus the variance between tiles:
            # a per-tile constant leaves the first at zero.
            inner = sum((v - mean) ** 2 for v in tile) / len(tile)
            inner_spread.append(inner ** 0.5)
            black += sum(1 for v in tile if v < 8)
            white += sum(1 for v in tile if v > 247)
    total = w * w
    between = (sum((v - sum(tile_spread) / len(tile_spread)) ** 2
                   for v in tile_spread) / len(tile_spread)) ** 0.5
    inside = sum(inner_spread) / len(inner_spread)
    ratio = inside / between if between else 0.0
    print(f"{name}")
    print(f"  between-tile spread   {between:6.1f} grey levels")
    print(f"  inside-tile spread    {inside:6.1f} grey levels")
    print(f"  inside/between        {ratio:6.2f}   "
          f"{'flat slabs — the tile is one value' if ratio < 0.25 else 'varying inside tiles'}")
    print(f"  pure black            {100 * black / total:5.1f}%")
    print(f"  pure white            {100 * white / total:5.1f}%")
    print()


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    stats(sys.argv[1], sys.argv[1])
    stats(sys.argv[2], sys.argv[2])
