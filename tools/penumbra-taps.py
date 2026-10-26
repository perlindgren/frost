#!/usr/bin/env python3
"""Draw the penumbra tap ladder, straight out of the shader.

`docs/penumbra-taps.png` and `docs/penumbra-taps-rotated.png` are in the
repository because §28 of docs/benchmarking.md argues from them, and a diagram
that cannot be regenerated is a diagram that will be wrong within a season. So
this script does not know the tap table: it reads `SHADOW_TAPS` out of
shaders/lighting.wgsl, takes the prefixes the shader actually walks, and draws
what it finds. Run it after touching the ladder and the pictures cannot lie.

Each of the nine panels is one rung — the first N entries of the table — and
they read left to right, top to bottom, rung 1 to rung 9. A filled point is a
ray this rung fires, a hollow one a point of the lattice this rung leaves alone,
the ring is the light's own disk at unit radius, and the arrow is the mean of
the rung's sample positions: how far off the light's centre that rung is
averaging over. The frame is green where that mean is zero and amber where it
is not, because a rung whose samples are off-centre is averaging the disk in
the wrong place.

The second file draws the same nine sets at five rotations each, since a
per-pixel hash spins every pixel's set to a different angle. It is the picture
that settles what an off-centre rung actually costs: five arrows pointing five
ways is a bias that cannot add up across a shadow's edge, which is what §28
measured and what the first account of this defect had assumed.

    python3 tools/penumbra-taps.py

Pure standard library — this box has no PIL, no numpy, no matplotlib, and a
diagram that needs one of them is a diagram nobody re-runs.
"""

import re
import struct
import zlib
from math import cos, hypot, sin, tau

SHADER = "shaders/lighting.wgsl"
UPRIGHT = "docs/penumbra-taps.png"
ROTATED = "docs/penumbra-taps-rotated.png"

PANEL = 168
GUTTER = 12
MARGIN = 16
SCALE = 52.0  # pixels per tap radius
RADIUS = 6  # the marker's radius in pixels

BG = (24, 26, 30)
RING = (92, 98, 116)
LIT = (226, 74, 58)
DRY = (104, 108, 120)
MEAN = (72, 208, 226)
BALANCED = (74, 206, 116)
BIASED = (232, 162, 52)
SPIN = [(226, 74, 58), (226, 118, 58), (214, 158, 92), (166, 118, 214), (108, 130, 226)]


def ladder():
    """The tap points, in the order the shader walks them."""
    text = open(SHADER, encoding="utf-8").read()
    block = re.search(r"const SHADOW_TAPS = array<vec2<f32>, 9>\((.*?)\);", text, re.S)
    if block is None:
        raise SystemExit(f"{SHADER} no longer declares SHADOW_TAPS as a 9-entry array")
    pairs = re.findall(r"vec2<f32>\(\s*(-?[\d.]+)(?:f32)?\s*,\s*(-?[\d.]+)(?:f32)?\s*\)", block.group(1))
    taps = [(float(x), float(y)) for x, y in pairs]
    if len(taps) != 9:
        raise SystemExit(f"read {len(taps)} taps from {SHADER}, expected 9")
    return taps


class Canvas:
    """An RGB buffer and the handful of marks the panels need."""

    def __init__(self, w, h):
        self.w, self.h = w, h
        self.px = [BG] * (w * h)

    def dot(self, x, y, colour, r=3):
        x, y = round(x), round(y)
        for dy in range(-r, r + 1):
            for dx in range(-r, r + 1):
                if dx * dx + dy * dy <= r * r:
                    self.poke(x + dx, y + dy, colour)

    def ring(self, cx, cy, rad, colour):
        steps = max(180, int(tau * rad * 2))
        for i in range(steps):
            a = tau * i / steps
            self.poke(round(cx + rad * cos(a)), round(cy + rad * sin(a)), colour)

    def arrow(self, x0, y0, x1, y1, colour):
        steps = max(2, int(hypot(x1 - x0, y1 - y0) * 2))
        for i in range(steps + 1):
            t = i / steps
            self.poke(round(x0 + (x1 - x0) * t), round(y0 + (y1 - y0) * t), colour)
        for a in (0.44, -0.44):
            ux, uy = x1 - x0, y1 - y0
            n = max(hypot(ux, uy), 1e-6)
            ux, uy = ux / n, uy / n
            hx, hy = ux * cos(a) - uy * sin(a), ux * sin(a) + uy * cos(a)
            for k in range(1, 7):
                self.poke(round(x1 - hx * k), round(y1 - hy * k), colour)

    def frame(self, x, y, w, h, colour):
        for i in range(w):
            for dy in range(2):
                self.poke(x + i, y + dy, colour)
                self.poke(x + i, y + h - 1 - dy, colour)
        for j in range(h):
            for dx in range(2):
                self.poke(x + dx, y + j, colour)
                self.poke(x + w - 1 - dx, y + j, colour)

    def poke(self, x, y, colour):
        if 0 <= x < self.w and 0 <= y < self.h:
            self.px[y * self.w + x] = colour

    def write(self, path):
        raw = b"".join(
            b"\x00" + b"".join(struct.pack("BBB", *self.px[y * self.w + x]) for x in range(self.w))
            for y in range(self.h)
        )

        def chunk(kind, body):
            return (
                struct.pack(">I", len(body))
                + kind
                + body
                + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
            )

        png = b"\x89PNG\r\n\x1a\n"
        png += chunk(b"IHDR", struct.pack(">IIBBBBB", self.w, self.h, 8, 2, 0, 0, 0))
        png += chunk(b"IDAT", zlib.compress(raw, 9))
        png += chunk(b"IEND", b"")
        open(path, "wb").write(png)
        print(f"wrote {path} ({self.w}x{self.h})")


def panel(canvas, ox, oy, taps, rung, spin=None):
    """One rung's sample set, centred in the panel at `ox`, `oy`."""
    taken = taps[:rung]
    seen = set(taken)
    cx, cy = ox + PANEL // 2, oy + PANEL // 2
    canvas.ring(cx, cy, SCALE, RING)
    for x, y in taps:
        colour = LIT if (x, y) in seen else DRY
        if spin is None:
            canvas.dot(cx + SCALE * x, cy + SCALE * y, colour, RADIUS if (x, y) in seen else 4)
            if (x, y) not in seen:
                # Hollow: clear the middle so an unused point reads as absent.
                canvas.dot(cx + SCALE * x, cy + SCALE * y, BG, 2)
    if spin is None:
        mx = sum(x for x, _ in taken) / rung
        my = sum(y for _, y in taken) / rung
        off = hypot(mx, my)
        if off < 1e-6:
            # Nothing to point at: this rung averages on the light's centre, so
            # the centre marker keeps its face rather than wearing an arrow.
            canvas.dot(cx, cy, BALANCED, 3)
        else:
            canvas.arrow(cx, cy, cx + SCALE * mx, cy + SCALE * my, MEAN)
            canvas.dot(cx + SCALE * mx, cy + SCALE * my, BG, 2)
        canvas.frame(ox, oy, PANEL, PANEL, BALANCED if off < 1e-6 else BIASED)
        return off
    for k, angle in enumerate(spin):
        ca, sa = cos(angle), sin(angle)
        colour = SPIN[k % len(SPIN)]
        sx = sy = 0.0
        for x, y in taken:
            # The shader's own rotation, applied about the disk's centre.
            qx, qy = x * ca - y * sa, x * sa + y * ca
            canvas.dot(cx + SCALE * qx, cy + SCALE * qy, colour, 3)
            sx, sy = sx + qx, sy + qy
        # The one thing the spin can do to a rung: carry its mean around with
        # it. A balanced rung's five arrows are one dot at the centre; an
        # off-centre rung's are a fan, and a fan is a bias that cannot add up.
        canvas.arrow(cx, cy, cx + SCALE * sx / rung, cy + SCALE * sy / rung, colour)
    canvas.dot(cx, cy, MEAN, 2)
    canvas.frame(ox, oy, PANEL, PANEL, RING)
    return None


def sheet(taps, spin=None):
    side = MARGIN * 2 + 3 * PANEL + 2 * GUTTER
    canvas = Canvas(side, side)
    for rung in range(1, 10):
        i, j = (rung - 1) % 3, (rung - 1) // 3
        panel(
            canvas,
            MARGIN + j * (PANEL + GUTTER),
            MARGIN + i * (PANEL + GUTTER),
            taps,
            rung,
            spin,
        )
    return canvas


def main():
    taps = ladder()
    sheet(taps).write(UPRIGHT)
    # The angle the engine's hash advances per row down a column: the outer
    # multiplier of `6.2831853 * fract(52.9829 * fract(0.06711 * x + 0.005837 * y))`.
    step = tau * ((52.9829 * 0.005837) % 1.0)
    sheet(taps, spin=[step * k for k in range(5)]).write(ROTATED)
    print("\nrung  set                              mean offset (radii)  reach")
    for rung in range(1, 10):
        taken = taps[:rung]
        mean = (
            hypot(sum(x for x, _ in taken) / rung, sum(y for _, y in taken) / rung),
        )[0]
        reach = max(hypot(x, y) for x, y in taken)
        where = "balanced" if mean < 1e-6 else "off centre"
        print(f"{rung:>4}  {str(taken):<32}  {mean:>6.3f}  {where}   {reach:.3f}")


if __name__ == "__main__":
    main()
