"""Cut pine-needle glyphs out of desk photos -> transparent RGBA PNGs.

v5, verification-driven, scipy-accelerated. The filename states which
letter/digit the photo contains — ground truth. Each candidate
segmentation is scored against the same character rendered from the
repo font (bbox-normalised, rotation-tolerant symmetric coverage): a
solid mesh cut lands ~0.5+, sparse grit or desk contamination ~0.3.
Best set wins; below PASS the image is flagged, not shipped. Output is
also shown on a contact sheet — the score can miss visible defects
(straight cuts, blobs), so eyeball the sheet before installing.

Segmentation per candidate set (s_off, v_off, bridge, pullback):
  1. Calibrate paperV/paperS from this image; paper dominates every frame.
  2. Needle = warmer than paper by a margin, or clearly darker. Shadows
     only darken; they never warm up — that is the seam.
  3. Sheet = big paper components; closed wide enough that the paper
     threading a stroke band swallows it, but pulled back past its own
     boundary so the desk border and the tanned paper edge stay out.
  4. Bridge the mesh, keep big components (body + i/j dots), peel
     pixels that really look like paper, feather, crop, zero RGB at
     alpha 0.
  5. desk_extend: strokes that ran off the paper onto the desk are
     recovered where they keep needle texture (edge-dense, warm,
     out-of-background contrast) AND continue an existing stroke.

Usage: python cutout.py SRC_DIR OUT_DIR NAME [NAME...]   (SRC has NAME.jpg)
Deps: pillow numpy scipy. Known limits: desk wood that matches the
needles in colour and texture defeats recovery; loose single needles
lying on the desk are not separable from wood grain at all — if the
glyph overshoots the paper, reshoot instead of pushing the tool.
"""
import sys, os, time
import numpy as np
from scipy import ndimage
from PIL import Image, ImageDraw, ImageFilter, ImageFont

MARGIN = 20
_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
FONT = os.environ.get('CUTOUT_FONT',
                      os.path.join(_ROOT, 'assets/fonts/FiraCode-VariableFont_wght.ttf'))
PASS = 0.45
K3 = np.ones((3, 3), bool)

# (saturation margin, darkness margin, sheet bridge r, extra pullback r)
PARAM_SETS = [
    (30, 60, 70, 12),
    (25, 50, 60, 12),
    (35, 70, 90, 12),
    (45, 45, 50, 12),
    (18, 40, 60, 12),
]


def dilate(mask, r):
    return ndimage.binary_dilation(mask, structure=K3, iterations=r)


def erode(mask, r):
    return ndimage.binary_erosion(mask, structure=K3, iterations=r)


def down(mask, scale):
    return np.asarray(Image.fromarray((mask * 255).astype(np.uint8)).resize(
        (max(1, mask.shape[1] // scale), max(1, mask.shape[0] // scale)),
        Image.BOX)) > 40


def up(small, shape):
    return np.asarray(Image.fromarray((small * 255).astype(np.uint8)).resize(
        (shape[1], shape[0]), Image.NEAREST)) > 127


def big_ccs(mask, scale, keep_frac):
    """Downscaled labeling; mask-space booleans of components >= keep_frac
    of the largest, plus the small-space labels for further use."""
    sm = down(mask, scale)
    lab, n = ndimage.label(sm, structure=np.ones((3, 3), int))
    if n == 0:
        return np.zeros(mask.shape, bool), lab
    counts = np.bincount(lab.ravel())
    if counts[1:].max() == 0:
        return np.zeros(mask.shape, bool), lab
    ids = 1 + np.where(counts[1:] >= keep_frac * counts[1:].max())[0]
    return up(np.isin(lab, ids), mask.shape), lab


def paper_stats(V, S):
    rough = (S < 50) & (V > 150)
    if rough.mean() < 0.05:
        rough = V > np.percentile(V, 60)
    pv = float(np.median(V[rough]))
    paper = (V > pv - 25) & (S < 90)
    return pv, float(np.median(S[paper]))


def normalize128(mask, rotate=0):
    img = Image.fromarray((mask * 255).astype(np.uint8))
    bb = img.getbbox()
    if bb is None:
        return np.zeros((128, 128), bool)
    img = img.crop(bb).resize((128, 128), Image.BILINEAR)
    if rotate:
        img = img.rotate(rotate, fillcolor=0)
        bb = img.getbbox()
        if bb is None:
            return np.zeros((128, 128), bool)
        img = img.crop(bb).resize((128, 128), Image.BILINEAR)
    return np.asarray(img) > 127


_refs = {}
def reference(ch):
    if ch not in _refs:
        font = ImageFont.truetype(FONT, 200)
        img = Image.new('L', (512, 512), 0)
        ImageDraw.Draw(img).text((100, 100), ch, fill=255, font=font)
        bb = img.getbbox()
        _refs[ch] = np.asarray(img.crop(bb).resize((128, 128), Image.BILINEAR)) > 127
    return _refs[ch]


def shape_score(mask, ch):
    """Coverage-based: how much of the mask sits on (a dilated) reference
    stroke, and vice versa. Mesh gaps and thick needle strokes make plain
    IoU punish good cuts; symmetric dilation coverage does not."""
    ref = reference(ch)
    rref = ndimage.binary_dilation(ref, structure=K3, iterations=10)
    best = 0.0
    for rot in (-15, -10, -5, 0, 5, 10, 15):
        m = normalize128(mask, rot)
        if m.sum() == 0:
            continue
        cov_mask = (m & rref).sum() / m.sum()          # mask sits on strokes
        cov_ref = (ref & ndimage.binary_dilation(m, structure=K3,
                    iterations=12)).sum() / ref.sum()  # strokes covered
        best = max(best, min(cov_mask, cov_ref))
    return best


def segment(im, s_off, v_off, bridge, pullback):
    hsv = np.asarray(im.convert('HSV')).astype(np.int32)
    S, V = hsv[..., 1], hsv[..., 2]
    paperV, paperS = paper_stats(V, S)
    s_thr = min(max(paperS + s_off, 35.0), 90.0)
    # Shadows only darken; they never warm up. Gate the darkness rule by
    # a small saturation floor so shaded paper stays paper.
    needle = (S > s_thr) | ((V < paperV - v_off) & (S > paperS + 10))

    sheet, _ = big_ccs(~needle, 6, 0.15)
    if not sheet.any():
        raise RuntimeError('no paper found')
    # Closing swallows the stroke band (paper threads it); the extra
    # pullback then erases any desk rim or tanned edge it bridged over.
    closed = erode(ndimage.binary_fill_holes(dilate(sheet, bridge)), bridge + pullback)
    glyph = needle & closed
    if glyph.mean() < 0.001:
        raise RuntimeError('no glyph pixels')

    solid, _ = big_ccs(dilate(glyph, 20), 3, 0.01)
    mask = dilate(solid, 20) & glyph
    solid2, _ = big_ccs(dilate(mask, 10), 2, 0.01)
    mask &= solid2

    paperish = (S < paperS + 20) & (V > paperV - 30)
    for _ in range(4):
        mask &= ~(paperish & dilate(~mask, 6))

    # Wood grain and paper folds have edges too, but only as one or two
    # parallel lines per window (~0.1 density); a needle mesh is crossing
    # needles in every direction (~0.4). Edge density separates them.
    ed = (np.asarray(im.convert('L').filter(ImageFilter.FIND_EDGES)) > 25).astype(float)
    dens = ndimage.uniform_filter(ed, size=15)
    mask &= dens > 0.16

    # Stray fragments far from the body die; compact near things survive
    # (those are the i/j dots).
    lab, n = ndimage.label(mask, structure=np.ones((3, 3), int))
    if n > 1:
        counts = np.bincount(lab.ravel())
        main = int(np.argmax(counts[1:])) + 1
        objs = ndimage.find_objects(lab)
        areas = counts[1:]
        mw = objs[main - 1][1].stop - objs[main - 1][1].start
        mh = objs[main - 1][0].stop - objs[main - 1][0].start
        dist = ndimage.distance_transform_edt(lab != main)
        keep = [main]
        for i in range(1, n + 1):
            if i == main:
                continue
            w_i = objs[i - 1][1].stop - objs[i - 1][1].start
            h_i = objs[i - 1][0].stop - objs[i - 1][0].start
            gap = dist[objs[i - 1]].min()
            # i/j dots: small, chunky. Desk chips and wood slivers are
            # either much wider than a dot or long thin splines.
            compact = (areas[i - 1] >= 0.003 * areas[main - 1]
                       and w_i < 0.15 * mw and h_i < 0.15 * mh)
            spline = (max(w_i, h_i) >= 0.35 * max(mw, mh)
                      and min(w_i, h_i) <= 0.05 * min(mw, mh))
            if not spline and (gap <= 20 or compact):
                keep.append(i)   # part of the letter, or an i/j dot
        # Desk strips hug the letter but are solid bright wood blocks
        # (fill > 0.38, V near paper level); real stroke fragments are
        # open meshes of dark needles.
        hsv = np.asarray(im.convert('HSV')).astype(np.int32)
        Vv = hsv[..., 2]
        _, pv_ref = paper_stats(Vv, hsv[..., 1])
        solid_keep = []
        for c in keep:
            if c == main:
                solid_keep.append(c)
                continue
            o = objs[c - 1]
            fill = areas[c - 1] / max(1, (o[1].stop - o[1].start) * (o[0].stop - o[0].start))
            if fill <= 0.38 or np.median(Vv[lab == c]) < pv_ref - 75:
                solid_keep.append(c)   # open mesh stroke fragment or dot
        mask = np.isin(lab, solid_keep)
    return mask


def desk_extend(im, mask, s_off, v_off, bridge, pullback):
    """Recover stroke parts that ran off the paper onto the desk (some
    photos were framed tight). On the desk needles share the wood's
    colour, but they stand OUT of the background (either direction — the
    desk can be brighter wood or darker cloth), are warm and edge-dense,
    and a recovery only counts where it continues a stroke off the paper.
    Paper-edge shadow lines are caught later by the spline guard."""
    hsv = np.asarray(im.convert('HSV')).astype(np.int32)
    S, V = hsv[..., 1], hsv[..., 2]
    paperV, paperS = paper_stats(V, S)
    s_thr = min(max(paperS + s_off, 35.0), 90.0)
    needle = (S > s_thr) | ((V < paperV - v_off) & (S > paperS + 10))
    sheet, _ = big_ccs(~needle, 6, 0.15)
    if not sheet.any():
        return mask
    closed = erode(ndimage.binary_fill_holes(dilate(sheet, bridge)), bridge + pullback)
    desk_far = ~dilate(closed, 100)
    if desk_far.mean() < 0.02:
        return mask
    bg = ndimage.maximum_filter(V, size=31)          # bright background near px
    bgd = ndimage.minimum_filter(V, size=31)         # dark background near px
    ed = (np.asarray(im.convert('L').filter(ImageFilter.FIND_EDGES)) > 25).astype(float)
    dens = ndimage.uniform_filter(ed, size=15)
    contrast = (V < bg - 22) | (V > bgd + 30)
    # Right at the paper boundary the tanned shadow line must not be
    # recovered as a stroke, so demand high density there; deeper on the
    # desk ordinary needle density suffices.
    band = dilate(closed, 16) & ~closed
    cand = (~closed) & (band & (dens > 0.30) | ~band & (dens > 0.22))
    cand &= (S > paperS + 25) & contrast
    cand &= dilate(mask, 30)
    lab, n = ndimage.label(cand, structure=np.ones((3, 3), int))
    if n == 0:
        return mask
    counts = np.bincount(lab.ravel())
    keep = 1 + np.where(counts[1:] >= 250)[0]
    return mask | np.isin(lab, keep)


def finalize(im, mask):
    h, w = mask.shape
    ys, xs = np.where(mask)
    if len(ys) == 0:
        raise RuntimeError('mask empty')
    x0, x1 = max(0, xs.min() - MARGIN), min(w - 1, xs.max() + MARGIN)
    y0, y1 = max(0, ys.min() - MARGIN), min(h - 1, ys.max() + MARGIN)
    alpha = np.asarray(Image.fromarray((mask * 255).astype(np.uint8))
                       .filter(ImageFilter.GaussianBlur(0.8)))
    out = np.asarray(im.convert('RGBA')).copy()[y0:y1 + 1, x0:x1 + 1]
    out[..., 3] = alpha[y0:y1 + 1, x0:x1 + 1]
    out[out[..., 3] == 0, :3] = 0
    return Image.fromarray(out), 100.0 * (alpha[y0:y1 + 1, x0:x1 + 1] > 200).mean()


def main():
    src_dir, out_dir = sys.argv[1], sys.argv[2]
    os.makedirs(out_dir, exist_ok=True)
    names = sys.argv[3:]
    tiles = []
    for name in names:
        t0 = time.time()
        im = Image.open(os.path.join(src_dir, name + '.jpg')).convert('RGB')
        im.thumbnail((1200, 1600))
        best = None
        scores = []
        cands = []
        for ps in PARAM_SETS:
            try:
                mask = segment(im, *ps)
            except Exception as err:
                scores.append(f'{ps}:fail')
                continue
            cands.append((ps, mask))
        # Tiny slivers can resemble a letter when stretched to fit;
        # size is the ground-truth anchor the shape score lacks.
        biggest = max((m.sum() for _, m in cands), default=0)
        for ps, mask in cands:
            if mask.sum() < 0.3 * biggest or mask.sum() < 2000:
                scores.append(f'{ps}:sliver')
                continue
            score = shape_score(mask, name)
            scores.append(f'{ps}:{score:.2f}')
            if best is None or score > best[0]:
                best = (score, ps, mask)
        if best is not None:
            # The scorer is a weak judge of on-desk content; extend the
            # winner and accept unless the letter visibly degrades.
            ext = desk_extend(im, best[2], *best[1])
            if ext.sum() > best[2].sum() and shape_score(ext, name) >= best[0] - 0.01:
                scores.append(f'EXT+{int(ext.sum() - best[2].sum())}')
                best = (max(shape_score(ext, name), best[0]), best[1], ext)
        if best is None:
            print(f'{name}: FAIL all sets', flush=True)
            continue
        score, ps, mask = best
        try:
            png, opaque = finalize(im, mask)
        except Exception as err:
            print(f'{name}: score {score:.2f} finalize fail {err}', flush=True)
            continue
        flag = '' if score >= PASS else '  <-- LOW'
        print(f'{name}: best {score:.2f} set {ps} opaque {opaque:.0f}% '
              f'{png.size} {time.time()-t0:.0f}s{flag} [{" ".join(scores)}]',
              flush=True)
        if score >= PASS:
            png.save(os.path.join(out_dir, name + '.png'), optimize=True)
        tile = Image.new('RGBA', (250, 270), (0, 140, 255, 255))
        th = png.copy()
        th.thumbnail((240, 238))
        tile.alpha_composite(th, ((250 - th.width) // 2, (238 - th.height) // 2))
        d = ImageDraw.Draw(tile)
        d.rectangle([0, 240, 250, 270], fill=(255, 255, 255, 255))
        d.text((8, 248), f'{name} {score:.2f}{"" if score >= PASS else " LOW"}',
               fill=(200, 0, 0) if score < PASS else (0, 0, 0))
        tiles.append(tile)
    if tiles:
        cols = 6
        rows = (len(tiles) + cols - 1) // cols
        sheet = Image.new('RGB', (cols * 250, rows * 270), (30, 30, 30))
        for i, t in enumerate(tiles):
            sheet.paste(t.convert('RGB'), ((i % cols) * 250, (i // cols) * 270))
        path = os.path.join(out_dir, 'contact_sheet.png')
        sheet.save(path)
        print(f'sheet: {path}', flush=True)


if __name__ == '__main__':
    main()
