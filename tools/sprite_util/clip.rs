//! The clipboard block: taking, dressing, trimming, and pasting
//! a rectangle of cells.
// The subjects this one reads.
use crate::map::*;

/// Paint one cell with the tileset's layer, creating the layer on the
/// first stroke. True when a cell changed.
/// A copied rectangle of the map, held on the desk: one tileset's
/// cells and orientations, row-major from the top line. It dresses
/// and pastes as one picture — a block is a brush made of tiles.
#[derive(Clone)]
pub(crate) struct Clip {
    pub(crate) tileset: std::path::PathBuf,
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    /// Row-major, top line first; `EMPTY_CELL` marks a gap.
    pub(crate) cells: Vec<u32>,
    pub(crate) tfms: Vec<u8>,
}

/// The copy of a selection rectangle: one tileset's view of the map,
/// `EMPTY_CELL` wherever its layer was empty or off the edge.
pub(crate) fn clip_take(
    maps: &[MapLayer],
    tileset: &std::path::Path,
    c0: i32,
    r0: i32,
    cols: usize,
    rows: usize,
) -> Clip {
    let layer = maps.iter().find(|m| m.tileset == tileset);
    let mut cells = Vec::with_capacity(cols * rows);
    let mut tfms = Vec::with_capacity(cols * rows);
    for r in 0..rows {
        for c in 0..cols {
            let hit = layer.and_then(|m| {
                m.at(c0 + c as i32, r0 + r as i32)
                    .map(|i| (m.cells[i], m.tfms[i]))
            });
            match hit {
                Some((cell, tfm)) => {
                    cells.push(cell);
                    tfms.push(tfm);
                }
                None => {
                    cells.push(EMPTY_CELL);
                    tfms.push(0);
                }
            }
        }
    }
    Clip {
        tileset: tileset.to_path_buf(),
        cols,
        rows,
        cells,
        tfms,
    }
}

/// The block's anchor cell: as near the center as whole cells allow
/// (even sides bias up-left). The ghost and the paste share it, so
/// what you preview is what you get.
pub(crate) fn clip_anchor(clip: &Clip) -> (i32, i32) {
    (((clip.cols - 1) / 2) as i32, ((clip.rows - 1) / 2) as i32)
}

/// Where a held block lands when the cursor drops it at `(col, row)`:
/// every non-empty cell, in absolute map coordinates, cursor-anchored
/// on the block's anchor. The ghost previews this list (intersected
/// with the map) and the paste writes it — one source, so the promise
/// and the drop cannot disagree, at the edges or anywhere else.
pub(crate) fn clip_cells(clip: &Clip, col: i32, row: i32) -> Vec<(i32, i32, u32, u8)> {
    let (ax, ay) = clip_anchor(clip);
    clip.cells
        .iter()
        .enumerate()
        .filter(|(_, c)| **c != EMPTY_CELL)
        .map(|(i, c)| {
            (
                col + (i % clip.cols) as i32 - ax,
                row + (i / clip.cols) as i32 - ay,
                *c,
                clip.tfms[i],
            )
        })
        .collect()
}

/// Turn the block a quarter: the arrangement rotates, and every tile
/// turns with it EXACTLY as it would on its own — the very rule the
/// brush obeys (`tfm_turn`): the turns count rides on, the tile's own
/// flips ride along untouched. A block is a brush made of tiles, so
/// dressing the block IS dressing each tile it holds; any other
/// composition rule would make the block disagree with the single
/// tile the picker just dressed.
pub(crate) fn clip_turn(clip: &mut Clip, cw: bool) {
    let (rows, cols) = (clip.rows, clip.cols);
    let mut cells = vec![EMPTY_CELL; rows * cols];
    let mut tfms = vec![0u8; rows * cols];
    for r in 0..rows {
        for c in 0..cols {
            let (nr, nc) = if cw {
                (c, rows - 1 - r)
            } else {
                (cols - 1 - c, r)
            };
            let at = nr * rows + nc;
            cells[at] = clip.cells[r * cols + c];
            tfms[at] = tfm_turn(clip.tfms[r * cols + c], cw);
        }
    }
    clip.cells = cells;
    clip.tfms = tfms;
    clip.cols = rows;
    clip.rows = cols;
}

/// Mirror the block: the arrangement reverses along the axis and
/// every tile mirrors with it — each through `tfm_flip_x`/
/// `tfm_flip_y`, so an odd-turned tile pays on the other bit. Byte-
/// equal to mirroring the block's rendered picture, per the golden
/// test.
pub(crate) fn clip_flip(clip: &mut Clip, x: bool) {
    let (rows, cols) = (clip.rows, clip.cols);
    let mut cells = vec![EMPTY_CELL; rows * cols];
    let mut tfms = vec![0u8; rows * cols];
    for r in 0..rows {
        for c in 0..cols {
            let (nr, nc) = if x {
                (r, cols - 1 - c)
            } else {
                (rows - 1 - r, c)
            };
            let at = nr * cols + nc;
            cells[at] = clip.cells[r * cols + c];
            tfms[at] = if x {
                tfm_flip_x(clip.tfms[r * cols + c])
            } else {
                tfm_flip_y(clip.tfms[r * cols + c])
            };
        }
    }
    clip.cells = cells;
    clip.tfms = tfms;
}

/// The selection's frame turns with the block it frames: a turned
/// clip holds transposed extents, and an unturned frame would show
/// the tiles standing in the wrong cells — right at 180 (extents
/// survive), wrong at every quarter turn. The frame pivots on its
/// anchor cell, the same cell the block itself pivots on, so frame
/// and picture always agree; cw and ccw take the same rectangle,
/// and two turns return it exactly home. A flip is a no-op: the
/// extents and the anchor cell both survive a mirror.
pub(crate) fn sel_turn(sel: [i32; 4]) -> [i32; 4] {
    let [c0, r0, c1, r1] = sel;
    let (w, h) = (c1 - c0 + 1, r1 - r0 + 1);
    let (ac, ar) = ((w - 1) / 2, (h - 1) / 2);
    let nc0 = c0 + ac - ar;
    let nr0 = r0 + ar - ac;
    [nc0, nr0, nc0 + h - 1, nr0 + w - 1]
}

/// Shrink a block to the rectangle that actually holds a picture.
/// Empty margins would swing the picture far from the pivot on a
/// turn and drop it nowhere near the cursor on a paste, so the desk
/// trims on harvest. Returns the kept rectangle in old-frame
/// coordinates (c0, r0, c1, r1), or None when the block is all gaps.
pub(crate) fn clip_trim(clip: &mut Clip) -> Option<(usize, usize, usize, usize)> {
    let (mut rmin, mut rmax, mut cmin, mut cmax) = (usize::MAX, 0usize, usize::MAX, 0usize);
    for (i, c) in clip.cells.iter().enumerate() {
        if *c != EMPTY_CELL {
            let (r, col) = (i / clip.cols, i % clip.cols);
            rmin = rmin.min(r);
            rmax = rmax.max(r);
            cmin = cmin.min(col);
            cmax = cmax.max(col);
        }
    }
    if rmin > rmax {
        return None;
    }
    let (rows, cols) = (rmax - rmin + 1, cmax - cmin + 1);
    let mut cells = Vec::with_capacity(rows * cols);
    let mut tfms = Vec::with_capacity(rows * cols);
    for r in rmin..=rmax {
        for c in cmin..=cmax {
            cells.push(clip.cells[r * clip.cols + c]);
            tfms.push(clip.tfms[r * clip.cols + c]);
        }
    }
    clip.cols = cols;
    clip.rows = rows;
    clip.cells = cells;
    clip.tfms = tfms;
    Some((cmin, rmin, cmax, rmax))
}

/// Stamp the block with its anchor cell on `(col, row)`. Gaps skip —
/// pasting lays tiles, it never erases; the eraser is its own verb.
/// Returns the cells the stamp actually changed.
/// Move the held block: cut the selection frame it was copied from,
/// drop the block cursor-anchored at (col, row), and let the frame
/// travel with the drop — the block's new home becomes its cut
/// zone. Without a frame (held since the frame was released) there
/// is nothing to cut and the move is a plain drop. Returns the cells
/// written, whether the cut changed anything, and the frame's new
/// box — the caller's one capture covers both halves.
pub(crate) fn clip_move(
    maps: &mut Vec<MapLayer>,
    clip: &Clip,
    sel: Option<[i32; 4]>,
    col: i32,
    row: i32,
) -> (u32, bool, Option<[i32; 4]>) {
    let mut erased = false;
    if let Some(f) = sel {
        erased = erase_rect(maps, f);
    }
    let n = clip_paste(maps, clip, col, row);
    let (ax, ay) = clip_anchor(clip);
    let frame = [
        col - ax,
        row - ay,
        col - ax + clip.cols as i32 - 1,
        row - ay + clip.rows as i32 - 1,
    ];
    (n, erased, Some(frame))
}

pub(crate) fn clip_paste(maps: &mut Vec<MapLayer>, clip: &Clip, col: i32, row: i32) -> u32 {
    let mut n = 0;
    for (c, r, cell, tfm) in clip_cells(clip, col, row) {
        if paint_at(maps, &clip.tileset, cell as usize, tfm, c, r) {
            n += 1;
        }
    }
    n
}

/// A block builder for the tests: one tileset, rows then cols,
/// row-major from the top line.
#[cfg(test)]
pub(crate) fn block(rows: usize, cols: usize, cells: &[u32], tfms: &[u8]) -> Clip {
    Clip {
        tileset: std::path::PathBuf::from("x.png"),
        cols,
        rows,
        cells: cells.to_vec(),
        tfms: tfms.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_turns_as_one_picture() {
        // Two tiles side by side, the right one mirrored. Turn the
        // block clockwise and it stands as a column, the left tile on
        // top, and each tile wears exactly what a single-tile turn
        // would have dressed on it: the mirror keeps its bits (the
        // turn rides on top of the tile's own dress, as on the rack).
        let mut c = block(1, 2, &[1, 2], &[0, 1]);
        clip_turn(&mut c, true);
        assert_eq!((c.rows, c.cols), (2, 1));
        assert_eq!(c.cells, vec![1, 2]);
        assert_eq!(c.tfms, vec![4, 5]);
        // Back the other way and the block is exactly what it was —
        // the two turns are true inverses, tiles included.
        clip_turn(&mut c, false);
        assert_eq!((c.rows, c.cols), (1, 2));
        assert_eq!(c.cells, vec![1, 2]);
        assert_eq!(c.tfms, vec![0, 1]);
    }

    #[test]
    fn the_block_flips_along_either_axis() {
        let c = block(2, 2, &[1, 2, 3, EMPTY_CELL], &[0, 0, 1, 0]);
        let mut x = c.clone();
        clip_flip(&mut x, true);
        assert_eq!(x.cells, vec![2, 1, EMPTY_CELL, 3]);
        assert_eq!(x.tfms, vec![1, 1, 1, 0]);
        let mut y = c.clone();
        clip_flip(&mut y, false);
        assert_eq!(y.cells, vec![3, EMPTY_CELL, 1, 2]);
        assert_eq!(y.tfms, vec![3, 2, 2, 2]);
    }

    #[test]
    fn the_paste_anchors_the_block_on_the_cursor() {
        let mut maps = vec![MapLayer::blank(std::path::PathBuf::from("x.png"), 4, 4)];
        let c = block(2, 2, &[5, 5, 5, EMPTY_CELL], &[0, 0, 0, 0]);
        // Two across, two deep, one a gap: three cells land, the
        // anchor cell riding the cursor (gaps skip, never erase).
        // The map centers on the origin, so anchor the block near
        // home: cells ride (0,0) and its right and below.
        assert_eq!(clip_paste(&mut maps, &c, 0, 0), 3);
        let m = &maps[0];
        for cell in [m.at(0, 0), m.at(1, 0), m.at(0, 1)] {
            assert_eq!(m.cells[cell.expect("inside")], 5);
        }
        // A second paste on the same spot changes nothing, and past
        // the map's edge the stamp is polite: nothing written, no
        // panic, nothing counted.
        assert_eq!(clip_paste(&mut maps, &c, 0, 0), 0);
        assert_eq!(clip_paste(&mut maps, &c, 9, 9), 0);
    }

    #[test]
    fn a_sloppy_frame_trims_to_the_picture() {
        // The repro from the wild: a 4x3 frame dragged around a
        // two-tile domino. The block must BE the domino — empty
        // margins only swing the picture away from every pivot.
        let mut c = block(
            3,
            4,
            &[
                EMPTY_CELL, EMPTY_CELL, EMPTY_CELL, EMPTY_CELL, EMPTY_CELL, 2, 3, EMPTY_CELL,
                EMPTY_CELL, EMPTY_CELL, EMPTY_CELL, EMPTY_CELL,
            ],
            &[0; 12],
        );
        assert_eq!(clip_trim(&mut c), Some((1, 1, 2, 1)));
        assert_eq!((c.cols, c.rows), (2, 1));
        assert_eq!(c.cells, vec![2, 3]);
        // An all-gap frame holds nothing to copy.
        let mut e = block(1, 2, &[EMPTY_CELL, EMPTY_CELL], &[0, 0]);
        assert_eq!(clip_trim(&mut e), None);
    }

    // The `1 * 3 + 1` below is the row-major formula spelled out: cell
    // (row 1, col 1) of a clip `c.cols` wide — `c.cols == 3` asserted
    // above, and the asserted values placed at `set_cell(1, 1, …)`:
    // `row × cols + col`, exactly the index into the flattened grid.
    // Clippy reads the multiply-by-one as an identity operation and
    // cannot know it is carrying the formula; reducing it to `4` (or even
    // `3 + 1`) leaves an index whose shape no longer names its cell. The
    // allow documents that, and marks why the expression stays.
    #[allow(clippy::identity_op)]
    #[test]
    fn the_selection_copy_reads_one_tilesets_view() {
        let mut maps = vec![
            MapLayer::blank(std::path::PathBuf::from("a.png"), 4, 4),
            MapLayer::blank(std::path::PathBuf::from("b.png"), 4, 4),
        ];
        maps[0].set_cell(1, 1, 7, 5);
        maps[1].set_cell(1, 1, 9, 0);
        let c = clip_take(&maps, std::path::Path::new("a.png"), 0, 0, 3, 2);
        assert_eq!((c.rows, c.cols), (2, 3));
        // One tileset's view of the map: its own cell, not the layer
        // painted above it; EMPTY on gaps and past the edge.
        assert_eq!(c.cells[1 * 3 + 1], 7);
        assert_eq!(c.tfms[1 * 3 + 1], 5);
        assert_eq!(c.cells[0], EMPTY_CELL);
        let d = clip_take(&maps, std::path::Path::new("missing.png"), 0, 0, 2, 2);
        assert!(d.cells.iter().all(|c| *c == EMPTY_CELL));
    }

    #[test]
    fn a_move_cuts_the_frame_and_the_frame_travels() {
        let clip = Clip {
            tileset: std::path::PathBuf::from("wild.png"),
            cols: 3,
            rows: 2,
            cells: vec![1, 2, 3, 4, 5, EMPTY_CELL],
            tfms: vec![0, 0, 0, 0, 0, 0],
        };
        let mut maps = vec![MapLayer::blank(std::path::PathBuf::from("wild.png"), 8, 8)];
        for (c, r, cell) in [(-1, -1, 1), (0, -1, 2), (1, -1, 3), (-1, 0, 4), (0, 0, 5)] {
            maps[0].set_cell(c, r, cell, 0);
        }
        let (n, erased, sel) = clip_move(&mut maps, &clip, Some([-1, -1, 1, 0]), 2, 2);
        assert_eq!((n, erased), (5, true), "the cut lands and the drop lands");
        for (c, r) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (0, 0)] {
            assert_eq!(
                maps[0].at(c, r).map(|i| maps[0].cells[i]),
                Some(EMPTY_CELL),
                "the frame is swept at ({c},{r})"
            );
        }
        assert_eq!(
            maps[0].at(2, 2).map(|i| maps[0].cells[i]),
            Some(2),
            "the anchor cell rides to the cursor"
        );
        assert_eq!(sel, Some([1, 2, 3, 3]), "the frame travels with the drop");
    }

    #[test]
    fn a_move_without_a_frame_is_a_plain_drop() {
        let clip = Clip {
            tileset: std::path::PathBuf::from("wild.png"),
            cols: 3,
            rows: 2,
            cells: vec![1, 2, 3, 4, 5, EMPTY_CELL],
            tfms: vec![0, 0, 0, 0, 0, 0],
        };
        let mut maps = vec![MapLayer::blank(std::path::PathBuf::from("wild.png"), 8, 8)];
        let (n, erased, sel) = clip_move(&mut maps, &clip, None, 0, 0);
        assert_eq!((n, erased), (5, false), "nothing to cut, all to drop");
        assert_eq!(
            sel,
            Some([-1, 0, 1, 1]),
            "the block gets a frame to call home"
        );
    }

    #[test]
    fn the_frame_turns_with_the_block() {
        // Three wide, two deep: a quarter turn swaps the extents,
        // pivoting on the anchor cell (row 0, col 1 here) so the
        // frame keeps the cell the block itself turns on...
        let t = sel_turn([0, 0, 2, 1]);
        assert_eq!(t, [1, -1, 2, 1]);
        // ...and a second turn back lands exactly home.
        assert_eq!(sel_turn(t), [0, 0, 2, 1]);
        // A square frame pivots in place; a single cell does not
        // move at all.
        assert_eq!(sel_turn([1, -2, 2, -1]), [1, -2, 2, -1]);
        assert_eq!(sel_turn([3, 3, 3, 3]), [3, 3, 3, 3]);
    }
}
