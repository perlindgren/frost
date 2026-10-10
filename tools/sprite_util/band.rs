//! The tileset band: the rack of cells below the work area.
// The subjects this one reads.
use crate::journal::*;
use crate::spots::*;
use crate::strip::*;

/// The rack's node pool: a tileset grid up to 8 x 8 shows whole; a
/// deeper grid's tail stays out of the rack.
pub(crate) const BAND_CELLS: usize = 64;

/// The band's cell boxes on screen: a row of squares this wide, above
/// the slots strip, starting at the window's left edge.
pub(crate) const BAND_CELL: f32 = 56.0;

/// The gap between two band cells.
pub(crate) const BAND_GAP: f32 = 6.0;

/// The board's margin: the quiet border between the plate's edge and
/// its cells — also the panel's grab border.
pub(crate) const BAND_PAD: f32 = 6.0;

/// The panel's own zoom ladder: how far the wheel may shrink and
/// grow the rack's cells.
pub(crate) const BAND_ZOOM_MIN: f32 = 0.4;

pub(crate) const BAND_ZOOM_MAX: f32 = 3.0;

/// The band's cell shapes' order: above the map's future tiles, below
/// the HUD, so the source art is never hidden by its own picture.
pub(crate) const BAND_Z: f32 = 1.4;

/// The rack's cell edge for a grid in a window: the standard box,
/// shrunk only when the whole matrix would not fit above the strip.
pub(crate) fn band_fit(rows: usize, cols: usize, w: f32, h: f32) -> f32 {
    let cw = (w - 40.0) / cols.max(1) as f32 - BAND_GAP;
    let ch = (h - STATUS_H - STRIP_H - 28.0) / rows.max(1) as f32 - BAND_GAP;
    BAND_CELL.min(cw).min(ch).max(12.0)
}

/// A cell's edge at the panel's own zoom: the fitted box scaled by
/// the wheel — a number the rack draws, the hit test hits, and the
/// shapes scale by, so the panel is one honest geometry.
pub(crate) fn band_cell(rows: usize, cols: usize, w: f32, h: f32, zoom: f32) -> f32 {
    (band_fit(rows, cols, w, h) * zoom).max(6.0)
}

/// The panel's corner: the plate's lower-left, at rest in the desk's
/// lower-left, above the slots strip.
pub(crate) fn band_origin_default(w: f32, h: f32) -> [f32; 2] {
    [-w / 2.0 + 14.0, floor_y(h) + STRIP_H + 8.0]
}

/// The panel's boxes: the active tileset's grid drawn as its matrix,
/// row-major like the atlas, bottom row nearest the origin. The panel
/// and its hit test draw from the same list, so what is clickable is
/// exactly what is drawn; a grid deeper than the pool keeps only its
/// first `BAND_CELLS` cells.
pub(crate) fn band_rects(rows: usize, cols: usize, cell: f32, origin: [f32; 2]) -> Vec<[f32; 4]> {
    let rows = rows.max(1);
    let cols = cols.max(1);
    let base = origin[1] + BAND_PAD + cell / 2.0;
    (0..(rows * cols).min(BAND_CELLS))
        .map(|i| {
            let (r, c) = (i / cols, i % cols);
            let cx = origin[0] + BAND_PAD + cell / 2.0 + c as f32 * (cell + BAND_GAP);
            let cy = base + (rows - 1 - r) as f32 * (cell + BAND_GAP);
            [
                cx - cell / 2.0,
                cy - cell / 2.0,
                cx + cell / 2.0,
                cy + cell / 2.0,
            ]
        })
        .collect()
}

/// The board behind the shown cells: their extent plus the pad. No
/// cells, no board.
pub(crate) fn band_plate(rects: &[[f32; 4]], cells: usize) -> Option<[f32; 4]> {
    let mut it = rects.iter().take(cells);
    let first = it.next()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first[0], first[1], first[2], first[3]);
    for r in it {
        x0 = x0.min(r[0]);
        y0 = y0.min(r[1]);
        x1 = x1.max(r[2]);
        y1 = y1.max(r[3]);
    }
    Some([x0 - BAND_PAD, y0 - BAND_PAD, x1 + BAND_PAD, y1 + BAND_PAD])
}

/// A panel origin clamped into the desk: the plate stays inside the
/// window and clear of the slots strip; a panel bigger than the desk
/// centers on the room there is.
pub(crate) fn band_clamp(
    origin: [f32; 2],
    cell: f32,
    rows: usize,
    cols: usize,
    w: f32,
    h: f32,
) -> [f32; 2] {
    let pw = 2.0 * BAND_PAD + cols.max(1) as f32 * cell + (cols.max(1) as f32 - 1.0) * BAND_GAP;
    let ph = 2.0 * BAND_PAD + rows.max(1) as f32 * cell + (rows.max(1) as f32 - 1.0) * BAND_GAP;
    let fix = |o: f32, lo: f32, hi: f32| {
        if hi < lo {
            (lo + hi) / 2.0
        } else {
            o.clamp(lo, hi)
        }
    };
    [
        fix(origin[0], -w / 2.0 + 2.0, w / 2.0 - 2.0 - pw),
        fix(origin[1], floor_y(h) + STRIP_H + 2.0, h / 2.0 - 2.0 - ph),
    ]
}

/// Whether `p` (if any) lies inside the rectangle.
pub(crate) fn p_in(p: Option<[f32; 2]>, [x0, y0, x1, y1]: [f32; 4]) -> bool {
    p.is_some_and(|[x, y]| x >= x0 && x <= x1 && y >= y0 && y <= y1)
}

/// The band cell at `p`, if `p` lands on one of the `cells` cells the
/// panel actually shows.
pub(crate) fn band_pick(
    p: [f32; 2],
    rows: usize,
    cols: usize,
    cell: f32,
    origin: [f32; 2],
    cells: usize,
) -> Option<usize> {
    band_rects(rows, cols, cell, origin)
        .into_iter()
        .take(cells)
        .position(|[x0, y0, x1, y1]| p[0] >= x0 && p[0] <= x1 && p[1] >= y0 && p[1] <= y1)
}

/// The position whose marker sits nearest `at` (window pixels), within
/// `r` of it — the sprite's way back into the tree.
pub(crate) fn spot_pick(
    spots: &[Spot],
    at: [f32; 2],
    size: [f32; 2],
    view: [f32; 2],
    zoom: f32,
    r: f32,
) -> Option<usize> {
    let [tw, th] = size;
    let mut best: Option<(f32, usize)> = None;
    for (pi, s) in spots.iter().enumerate() {
        let wx = (s.x - tw / 2.0) * zoom + view[0];
        let wy = (th / 2.0 - s.y) * zoom + view[1];
        let d = ((wx - at[0]).powi(2) + (wy - at[1]).powi(2)).sqrt();
        if d <= r && best.is_none_or(|(b, _)| d < b) {
            best = Some((d, pi));
        }
    }
    best.map(|(_, pi)| pi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use frost::ron as ron_tree;

    #[test]
    fn the_nearest_marker_wins_the_pick() {
        let mut root = ron_tree::parse("[(10.0, 20.0), (80.0, 70.0)]").expect("parses");
        root.open_to(0, 9);
        let mut spots = Vec::new();
        scan_spots(&root, &mut Vec::new(), "", &mut spots);
        let size = [100.0, 100.0];
        // Centred view, zoom 1: texture (10, 20) sits at window
        // (-40, 30) — the y flip puts y = 20 above the middle.
        let hit =
            |at: [f32; 2], view: [f32; 2], zoom: f32| spot_pick(&spots, at, size, view, zoom, 5.0);
        assert_eq!(hit([-40.0, 30.0], [0.0, 0.0], 1.0), Some(0));
        assert_eq!(hit([-41.0, 32.0], [0.0, 0.0], 1.0), Some(0));
        assert_eq!(hit([0.0, 0.0], [0.0, 0.0], 1.0), None);
        // Panning and zooming move the markers with the sprite.
        assert_eq!(hit([-30.0, 40.0], [10.0, 10.0], 1.0), Some(0));
        assert_eq!(hit([-80.0, 60.0], [0.0, 0.0], 2.0), Some(0));
    }

    #[test]
    fn band_cells_tile_the_rack_row() {
        // A 1 x 12 tileset is a flat left-aligned row above the
        // strip: exact edges hit, the gap between cells misses, and
        // a rest-placed panel starts at the desk's corner.
        let (w, h) = (1440.0, 810.0);
        let cell = band_cell(1, 12, w, h, 1.0);
        let o = band_origin_default(w, h);
        let rects = band_rects(1, 12, cell, o);
        assert_eq!(rects.len(), 12);
        let [x0, y0, x1, y1] = rects[0];
        assert!(band_pick([x0 + 1.0, (y0 + y1) / 2.0], 1, 12, cell, o, 4) == Some(0));
        let [nx0, ..] = rects[1];
        // The gap between cell 0 and cell 1 belongs to no cell.
        assert!(band_pick([x1 - 0.5, (y0 + y1) / 2.0], 1, 12, cell, o, 4) == Some(0));
        assert!(band_pick([(x1 + nx0) / 2.0, (y0 + y1) / 2.0], 1, 12, cell, o, 4).is_none());
        // A cell the panel doesn't show (only 4 of 12 painted) is
        // not clickable even where its box would sit.
        let [fx0, fy0, fx1, fy1] = rects[4];
        assert!(band_pick([(fx0 + fx1) / 2.0, (fy0 + fy1) / 2.0], 1, 12, cell, o, 4).is_none());
        assert!(band_pick([(fx0 + fx1) / 2.0, (fy0 + fy1) / 2.0], 1, 12, cell, o, 12) == Some(4));
        // Below the rack, the slots strip lives.
        assert!(y0 >= floor_y(h) + STRIP_H);
    }

    #[test]
    fn the_rack_copies_the_tileset_grid() {
        // A 2 x 2 tileset is a matrix in the atlas' own reading
        // order: cell 1 right of cell 0, cell 2 below cell 0.
        let (w, h) = (1440.0, 810.0);
        let cell = band_cell(2, 2, w, h, 1.0);
        let o = band_origin_default(w, h);
        let rects = band_rects(2, 2, cell, o);
        assert_eq!(rects.len(), 4);
        let (a, b, c) = (rects[0], rects[1], rects[2]);
        assert!(b[0] >= a[2] && (b[1] - a[1]).abs() < 1e-3);
        assert!((c[0] - a[0]).abs() < 1e-3 && c[3] <= a[1]);
        let mid = |r: [f32; 4]| [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0];
        assert_eq!(band_pick(mid(b), 2, 2, cell, o, 4), Some(1));
        assert_eq!(band_pick(mid(c), 2, 2, cell, o, 4), Some(2));
        // A deep grid stays whole and inside: an 8 x 8 rack, cells
        // fitted to the window, clamps back into the desk.
        let big_cell = band_cell(8, 8, w, h, 1.0);
        let big_o = band_clamp([5000.0, 5000.0], big_cell, 8, 8, w, h);
        let big = band_rects(8, 8, big_cell, big_o);
        assert_eq!(big.len(), 64);
        let [px0, py0, px1, py1] = band_plate(&big, 64).unwrap();
        assert!(px0 >= -w / 2.0 && px1 <= w / 2.0);
        assert!(py0 >= floor_y(h) + STRIP_H && py1 <= h / 2.0);
        // The panel's wheel is its own: the canvas never enters this
        // number, the cell simply scales.
        let doubled = band_cell(2, 2, w, h, 2.0);
        assert!((doubled - 2.0 * cell).abs() < 1e-3);
        // And the board covers the shown cells with its pad.
        let plate = band_plate(&rects, 4).unwrap();
        assert!((plate[0] - (a[0] - BAND_PAD)).abs() < 1e-3);
        assert!(band_plate(&rects, 0).is_none());
    }
}
