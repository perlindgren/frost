# photo_cutout — needle-glyph sprite extraction

Turns photos of pine-needle letters/digits on white paper into clean
transparent sprites. Born in the antifrost hill-puzzle jam; extracted
the 30-sprite set in `examples/antifrost/assets/sprites/` (the `leaf*`
and `ant*` sprites are unrelated).

## Usage

```sh
python -m venv .venv && .venv/bin/pip install -r tools/photo_cutout/requirements.txt
.venv/bin/python tools/photo_cutout/cutout.py SRC_DIR OUT_DIR 0 1 2 a b c ...
```

* `SRC_DIR` holds `NAME.jpg` for every glyph; **the filename is ground
  truth** — the shape scorer checks the cut against that character.
* Only glyphs scoring ≥ `PASS` (0.45) are written to `OUT_DIR`.
* Always review `OUT_DIR/contact_sheet.png` before installing sprites;
  the score cannot see every defect (see limits below).
* The verification contract for game sprites: RGBA, corner pixel
  `(0,0,0,0)`, RGB zeroed wherever alpha is 0.
* Reference font: `assets/fonts/FiraCode-VariableFont_wght.ttf`
  (override with `CUTOUT_FONT=/path/to.ttf`).

## How it works (v5)

1. **Per-photo calibration** — paper brightness/saturation medians come
   from the image itself; every threshold is relative. Absolute color
   thresholds never survive a re-shoot.
2. **Needle rule** — warmer than paper by a margin, or clearly darker
   *and* warm. Shadows darken but never warm up; that asymmetry is what
   keeps shadowed paper out of the mask.
3. **Paper sheet** — morphological *closing* of the paper components,
   then erosion **past** the bridge radius. Closing swallows the stroke
   band because the paper threads the needle mesh; the pullback then
   removes the desk rim and the tanned paper edge. (`fill_holes` fails:
   counter paper threads through mesh gaps, so stroke rings are not
   enclosed.)
4. **Guards** — edge-density gate (needle mesh ≈ 0.4 vs wood grain/paper
   folds ≈ 0.1 in a 15px window), spline/desk-strip component rules,
   size-vs-largest guard so slivers cannot score as letters.
5. **Score** — symmetric dilation coverage against the font glyph
   (plain IoU punishes good mesh cuts), best over ±15° rotation.
6. **`desk_extend`** — strokes that overshot the paper edge are
   recovered on the desk where they stay edge-dense, warm, contrast
   against the *local* background (desk can be bright wood or dark
   cloth), and touch the existing stroke.

## Known limits — do not re-derive these

The 2025-11 batch (needles on wooden desk) hit a wall: **the desk wood
matched the needles** — hue 23 vs 23, saturation 143 vs 143, overlapping
edge density and orientation coherence. Consequences, each verified
experimentally:

* Needles lying on the desk are **not separable** from wood grain. Only
  dense clusters directly continuing a stroke are recoverable; loose
  single needles off-paper are hopeless. If glyphs overshoot the paper,
  **reshoot with margin** — that is cheaper than any algorithm.
* Chaining recovery ("grow from stroke across the edge") runs away into
  grain and red-pen doodles; hue gates kill legitimate needles with
  them. Rejected after visual verification of every variant.
* Texture/edge features only work as **reject** gates on the final
  mask. Using them in the needle classification floods the desk.
* The shape score is blind to: straight cuts at the paper boundary (a
  cut letter still scores 0.7), desk contamination that sits on the
  stroke silhouette, and on-desk content in general. The contact sheet
  (blue ground) is mandatory.
* Dead ends already explored, do not retry: absolute S/V thresholds;
  `fill_holes`-only sheet; downscaled closing (hollows letters);
  IoU-vs-font scoring; harsh paper-peel; a "desk is warmer than paper"
  color guard.

## Status

Frozen with the antifrost game. The installed 30-sprite set is the
best achievable from the given photos; `2`, `3`, `5` are visibly
imperfect because their photos framed the glyphs past the paper edge.
