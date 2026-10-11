//! The clipboard block: taking, dressing, trimming, and pasting
//! a rectangle of cells — from every layer you are looking at,
//! back onto the very layers they came from.
//!
//! A copy reads the map as the eye sees it: every visible layer
//! contributes its block, and the paste lays each block home to its
//! own layer. Hidden layers keep their silence — neither copied nor
//! pasted into. A block therefore belongs to a LAYER (its place on
//! the bench), never to a tileset's name: two layers may wear one
//! tileset, and the picture you painted on the second twin is the
//! picture you copied.

// The subjects this one reads.
use crate::map::*;

/// One layer's share of the copied frame: its cells and orientations,
/// row-major from the top line, and the place of the layer it came
/// from.
#[derive(Clone)]
pub(crate) struct Block {
    /// Where the layer stands on the bench — position, not name.
    pub(crate) layer: usize,
    /// Row-major, top line first; `EMPTY_CELL` marks a gap.
    pub(crate) cells: Vec<u32>,
    pub(crate) tfms: Vec<u8>,
}

/// A copied rectangle of the map as seen: one block per layer that
/// had a picture inside the frame. It dresses and pastes as one
/// picture across all of them — a block is a brush made of tiles,
/// and a clip is a stack made of blocks.
#[derive(Clone)]
pub(crate) struct Clip {
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) blocks: Vec<Block>,
}

/// The copy of a selection rectangle, taken as the visible layers
/// show it: one block per looked-at layer that holds a picture
/// inside the frame. A silent layer is not copied as emptiness;
/// a frame no visible layer fills is no copy at all.
pub(crate) fn clip_take(
    maps: &[MapLayer],
    visible: &[bool],
    c0: i32,
    r0: i32,
    cols: usize,
    rows: usize,
) -> Option<Clip> {
    let mut blocks = Vec::new();
    for (i, m) in maps.iter().enumerate() {
        if !visible.get(i).copied().unwrap_or(true) {
            continue;
        }
        let mut cells = Vec::with_capacity(cols * rows);
        let mut tfms = Vec::with_capacity(cols * rows);
        for r in 0..rows {
            for c in 0..cols {
                match m.at(c0 + c as i32, r0 + r as i32) {
                    Some(idx) => {
                        cells.push(m.cells[idx]);
                        tfms.push(m.tfms[idx]);
                    }
                    None => {
                        cells.push(EMPTY_CELL);
                        tfms.push(0);
                    }
                }
            }
        }
        if cells.iter().any(|c| *c != EMPTY_CELL) {
            blocks.push(Block {
                layer: i,
                cells,
                tfms,
            });
        }
    }
    (!blocks.is_empty()).then_some(Clip { cols, rows, blocks })
}

/// The block's anchor cell: as near the center as whole cells allow
/// (even sides bias up-left). The ghost and the paste share it, so
/// what you preview is what you get.
pub(crate) fn clip_anchor(clip: &Clip) -> (i32, i32) {
    (((clip.cols - 1) / 2) as i32, ((clip.rows - 1) / 2) as i32)
}

/// Where one block lands when the cursor drops it at `(col, row)`:
/// every non-empty cell, in absolute map coordinates, cursor-anchored
/// on the block's anchor. The ghost previews this list (intersected
/// with the map) and the paste writes it — one source, so the promise
/// and the drop cannot disagree, at the edges or anywhere else.
pub(crate) fn block_cells(
    clip: &Clip,
    blk: &Block,
    col: i32,
    row: i32,
) -> Vec<(i32, i32, u32, u8)> {
    let (ax, ay) = clip_anchor(clip);
    blk.cells
        .iter()
        .enumerate()
        .filter(|(_, c)| **c != EMPTY_CELL)
        .map(|(i, c)| {
            (
                col + (i % clip.cols) as i32 - ax,
                row + (i / clip.cols) as i32 - ay,
                *c,
                blk.tfms[i],
            )
        })
        .collect()
}

/// The whole drop, layer-tagged: every currently visible block, in
/// each cell the drop writes, with the layer that receives it. A
/// block hidden since the copy keeps to itself — the paste affects
/// the layers you are looking at, and only those.
pub(crate) fn clip_cells(
    clip: &Clip,
    visible: &[bool],
    col: i32,
    row: i32,
) -> Vec<(usize, i32, i32, u32, u8)> {
    clip.blocks
        .iter()
        .filter(|b| visible.get(b.layer).copied().unwrap_or(true))
        .flat_map(|b| {
            block_cells(clip, b, col, row)
                .into_iter()
                .map(move |(c, r, cell, tfm)| (b.layer, c, r, cell, tfm))
        })
        .collect()
}

/// Turn the clip a quarter: every block's arrangement rotates, and
/// every tile turns with it EXACTLY as it would on its own — the very
/// rule the brush obeys (`tfm_turn`). Dressing the clip IS dressing
/// every block it holds; any other composition rule would make the
/// stack disagree with the single tile the picker just dressed.
pub(crate) fn clip_turn(clip: &mut Clip, cw: bool) {
    let (rows, cols) = (clip.rows, clip.cols);
    for blk in &mut clip.blocks {
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
                cells[at] = blk.cells[r * cols + c];
                tfms[at] = tfm_turn(blk.tfms[r * cols + c], cw);
            }
        }
        blk.cells = cells;
        blk.tfms = tfms;
    }
    clip.cols = rows;
    clip.rows = cols;
}

/// Mirror the clip along an axis: every block's arrangement reverses
/// and every tile mirrors with it, each through `tfm_flip_x`/
/// `tfm_flip_y` — byte-equal to mirroring the rendered picture, per
/// the golden test.
pub(crate) fn clip_flip(clip: &mut Clip, x: bool) {
    let (rows, cols) = (clip.rows, clip.cols);
    for blk in &mut clip.blocks {
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
                cells[at] = blk.cells[r * cols + c];
                tfms[at] = if x {
                    tfm_flip_x(blk.tfms[r * cols + c])
                } else {
                    tfm_flip_y(blk.tfms[r * cols + c])
                };
            }
        }
        blk.cells = cells;
        blk.tfms = tfms;
    }
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

/// Shrink the clip to the rectangle that actually holds a picture —
/// the picture being every block's: the bounds are taken over the
/// whole stack, so the blocks stay congruent and the frame still
/// frames them all. Returns the kept rectangle in old-frame
/// coordinates (c0, r0, c1, r1), or None when nothing holds anything.
pub(crate) fn clip_trim(clip: &mut Clip) -> Option<(usize, usize, usize, usize)> {
    let (mut rmin, mut rmax, mut cmin, mut cmax) = (usize::MAX, 0usize, usize::MAX, 0usize);
    for blk in &clip.blocks {
        for (i, c) in blk.cells.iter().enumerate() {
            if *c != EMPTY_CELL {
                let (r, col) = (i / clip.cols, i % clip.cols);
                rmin = rmin.min(r);
                rmax = rmax.max(r);
                cmin = cmin.min(col);
                cmax = cmax.max(col);
            }
        }
    }
    if rmin > rmax {
        return None;
    }
    let (rows, cols) = (rmax - rmin + 1, cmax - cmin + 1);
    for blk in &mut clip.blocks {
        let mut cells = Vec::with_capacity(rows * cols);
        let mut tfms = Vec::with_capacity(rows * cols);
        for r in rmin..=rmax {
            for c in cmin..=cmax {
                cells.push(blk.cells[r * clip.cols + c]);
                tfms.push(blk.tfms[r * clip.cols + c]);
            }
        }
        blk.cells = cells;
        blk.tfms = tfms;
    }
    clip.cols = cols;
    clip.rows = rows;
    Some((cmin, rmin, cmax, rmax))
}

/// Move the held block: cut the selection frame where the blocks
/// came from — the layers you see, and only those; what stood hidden
/// stays where it stands — drop every block cursor-anchored at
/// `(col, row)`, and let the frame travel with the drop. Without a
/// frame (held since the frame was released) there is nothing to cut
/// and the move is a plain drop. Returns the cells written, whether
/// the cut changed anything, and the frame's new box.
pub(crate) fn clip_move(
    maps: &mut Vec<MapLayer>,
    visible: &[bool],
    clip: &Clip,
    sel: Option<[i32; 4]>,
    col: i32,
    row: i32,
) -> (u32, bool, Option<[i32; 4]>) {
    let mut erased = false;
    if let Some(f) = sel {
        for &at in &clip
            .blocks
            .iter()
            .filter(|b| visible.get(b.layer).copied().unwrap_or(true))
            .map(|b| b.layer)
            .collect::<Vec<_>>()
        {
            if let Some(m) = maps.get_mut(at) {
                for r in f[1]..=f[3] {
                    for c in f[0]..=f[2] {
                        if let Some(i) = m.at(c, r)
                            && m.cells[i] != EMPTY_CELL
                        {
                            m.cells[i] = EMPTY_CELL;
                            m.tfms[i] = 0;
                            erased = true;
                        }
                    }
                }
            }
        }
    }
    let n = clip_paste(maps, visible, clip, col, row);
    let (ax, ay) = clip_anchor(clip);
    let frame = [
        col - ax,
        row - ay,
        col - ax + clip.cols as i32 - 1,
        row - ay + clip.rows as i32 - 1,
    ];
    (n, erased, Some(frame))
}

/// Stamp every visible block with its anchor cell on `(col, row)`,
/// each home to its own layer. Gaps skip — pasting lays tiles, it
/// never erases; the eraser is its own verb. Returns the cells the
/// stamp actually changed.
pub(crate) fn clip_paste(
    maps: &mut Vec<MapLayer>,
    visible: &[bool],
    clip: &Clip,
    col: i32,
    row: i32,
) -> u32 {
    let mut n = 0;
    for (at, c, r, cell, tfm) in clip_cells(clip, visible, col, row) {
        // The block returns to its own layer, and speaks that
        // layer's tileset: the paint dresses the layer it joins.
        let Some(who) = maps.get(at).map(|m| m.tileset.clone()) else {
            continue;
        };
        if paint_at(maps, at, &who, cell as usize, tfm, c, r) {
            n += 1;
        }
    }
    n
}

/// A block builder for the tests: one layer's block, rows then cols,
/// row-major from the top line.
#[cfg(test)]
pub(crate) fn block(rows: usize, cols: usize, cells: &[u32], tfms: &[u8]) -> Clip {
    Clip {
        cols,
        rows,
        blocks: vec![Block {
            layer: 0,
            cells: cells.to_vec(),
            tfms: tfms.to_vec(),
        }],
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
        assert_eq!(c.blocks[0].cells, vec![1, 2]);
        assert_eq!(c.blocks[0].tfms, vec![4, 5]);
        // Back the other way and the block is exactly what it was —
        // the two turns are true inverses, tiles included.
        clip_turn(&mut c, false);
        assert_eq!((c.rows, c.cols), (1, 2));
        assert_eq!(c.blocks[0].cells, vec![1, 2]);
        assert_eq!(c.blocks[0].tfms, vec![0, 1]);
    }

    #[test]
    fn the_block_flips_along_either_axis() {
        let c = block(2, 2, &[1, 2, 3, EMPTY_CELL], &[0, 0, 1, 0]);
        let mut x = c.clone();
        clip_flip(&mut x, true);
        assert_eq!(x.blocks[0].cells, vec![2, 1, EMPTY_CELL, 3]);
        assert_eq!(x.blocks[0].tfms, vec![1, 1, 1, 0]);
        let mut y = c.clone();
        clip_flip(&mut y, false);
        assert_eq!(y.blocks[0].cells, vec![3, EMPTY_CELL, 1, 2]);
        assert_eq!(y.blocks[0].tfms, vec![3, 2, 2, 2]);
    }

    #[test]
    fn a_dress_goes_through_every_block_at_once() {
        // Two layers, one frame: the turn is the stack's, not the
        // block's — both take it together, each tile dressed as if
        // alone.
        let mut c = Clip {
            cols: 1,
            rows: 2,
            blocks: vec![
                Block {
                    layer: 0,
                    cells: vec![1, 2],
                    tfms: vec![0, 0],
                },
                Block {
                    layer: 2,
                    cells: vec![EMPTY_CELL, 7],
                    tfms: vec![0, 1],
                },
            ],
        };
        clip_turn(&mut c, true);
        assert_eq!((c.cols, c.rows), (2, 1));
        // A column clockwise lies down with its foot to the right,
        // every tile turned once — and the second block, a lone tile
        // with a mirror, rides by the very same rule.
        assert_eq!(c.blocks[0].cells, vec![2, 1]);
        assert_eq!(c.blocks[0].tfms, vec![4, 4]);
        assert_eq!(c.blocks[1].cells, vec![7, EMPTY_CELL]);
        // The mirror turned is a turned mirror (5), and the empty
        // slot's byte turns too — invisible, but never inconsistent.
        assert_eq!(c.blocks[1].tfms, vec![5, 4]);
    }

    #[test]
    fn the_paste_anchors_the_block_on_the_cursor() {
        let mut maps = vec![MapLayer::blank("x", 4, 4)];
        let c = block(2, 2, &[5, 5, 5, EMPTY_CELL], &[0, 0, 0, 0]);
        // Two across, two deep, one a gap: three cells land, the
        // anchor cell riding the cursor (gaps skip, never erase).
        // The map centers on the origin, so anchor the block near
        // home: cells ride (0,0) and its right and below.
        assert_eq!(clip_paste(&mut maps, &[true], &c, 0, 0), 3);
        let m = &maps[0];
        for cell in [m.at(0, 0), m.at(1, 0), m.at(0, 1)] {
            assert_eq!(m.cells[cell.expect("inside")], 5);
        }
        // A second paste on the same spot changes nothing, and past
        // the map's edge the stamp is polite: nothing written, no
        // panic, nothing counted.
        assert_eq!(clip_paste(&mut maps, &[true], &c, 0, 0), 0);
        assert_eq!(clip_paste(&mut maps, &[true], &c, 9, 9), 0);
    }

    #[test]
    fn a_sloppy_frame_trims_to_the_whole_picture() {
        // The repro from the wild: a 4x3 frame dragged around a
        // two-tile domino. The block must BE the domino — empty
        // margins only swing the picture away from every pivot. And
        // with two layers the trim keeps them congruent: the bounds
        // are the stack's, the cut the same for every block.
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
        assert_eq!(c.blocks[0].cells, vec![2, 3]);
        // An all-gap frame holds nothing to copy.
        let mut e = block(1, 2, &[EMPTY_CELL, EMPTY_CELL], &[0, 0]);
        e.blocks.clear();
        assert_eq!(clip_trim(&mut e), None);
    }

    // The `1 * 3 + 1` below is the row-major formula spelled out:
    // cell (row 1, col 1) of a 3-wide frame — `row × cols + col`,
    // exactly the index into the flattened grid. Clippy reads the
    // multiply-by-one as an identity operation and cannot know it is
    // carrying the formula; reducing it to `3` (or even `2 + 1`)
    // leaves an index whose shape no longer names its cell. The
    // allow documents that, and marks why the expression stays.
    #[allow(clippy::identity_op)]
    #[test]
    fn the_selection_copy_reads_every_layer_you_look_at() {
        let mut maps = vec![MapLayer::blank("a", 4, 4), MapLayer::blank("b", 4, 4)];
        maps[0].set_cell(1, 1, 7, 5);
        maps[1].set_cell(1, 1, 9, 0);
        let c = clip_take(&maps, &[true, true], 0, 0, 3, 2).expect("a seen picture");
        assert_eq!((c.rows, c.cols), (2, 3));
        // Every looked-at layer speaks: each keeps its own cells,
        // EMPTY wherever its layer was empty or off the edge.
        assert_eq!(c.blocks.len(), 2);
        assert_eq!(c.blocks[0].layer, 0);
        assert_eq!(c.blocks[0].cells[1 * 3 + 1], 7);
        assert_eq!(c.blocks[0].tfms[1 * 3 + 1], 5);
        assert_eq!(c.blocks[1].layer, 1);
        assert_eq!(c.blocks[1].cells[1 * 3 + 1], 9);
    }

    #[test]
    fn a_hidden_layer_neither_copies_nor_receives() {
        let mut maps = vec![MapLayer::blank("a", 4, 4), MapLayer::blank("b", 4, 4)];
        maps[0].set_cell(0, 0, 1, 0);
        maps[1].set_cell(-1, -1, 2, 0);
        let c = clip_take(&maps, &[true, false], -2, -2, 4, 4).expect("a visible picture");
        assert_eq!(
            c.blocks.len(),
            1,
            "the hidden layer keeps its silence — not copied as emptiness"
        );
        assert_eq!(c.blocks[0].layer, 0);
        let n = clip_paste(&mut maps, &[true, false], &c, 0, 0);
        assert!(n > 0, "the visible layer takes the drop");
        assert_eq!(
            maps[1].cells.iter().filter(|c| **c != EMPTY_CELL).count(),
            1,
            "the hidden layer stands exactly as it stood"
        );
    }

    #[test]
    fn blocks_belong_to_layers_not_names() {
        // Two layers, one tileset name — the courtyard's twins. The
        // copy reads the layer painted, not the first twin: position,
        // never name.
        let mut maps = vec![
            MapLayer::blank("twins", 4, 4),
            MapLayer::blank("twins", 4, 4),
        ];
        maps[1].set_cell(0, 0, 9, 0);
        let c = clip_take(&maps, &[true, true], -2, -2, 4, 4).expect("a seen picture");
        assert_eq!(c.blocks.len(), 1);
        assert_eq!(
            c.blocks[0].layer, 1,
            "the block names the layer painted, not the first twin"
        );
        assert_eq!(c.blocks[0].cells[2 * 4 + 2], 9);
    }

    #[test]
    fn a_move_cuts_the_frame_and_the_frame_travels() {
        let clip = Clip {
            cols: 3,
            rows: 2,
            blocks: vec![Block {
                layer: 0,
                cells: vec![1, 2, 3, 4, 5, EMPTY_CELL],
                tfms: vec![0, 0, 0, 0, 0, 0],
            }],
        };
        let mut maps = vec![MapLayer::blank("wild", 8, 8)];
        for (c, r, cell) in [(-1, -1, 1), (0, -1, 2), (1, -1, 3), (-1, 0, 4), (0, 0, 5)] {
            maps[0].set_cell(c, r, cell, 0);
        }
        let (n, erased, sel) = clip_move(&mut maps, &[true], &clip, Some([-1, -1, 1, 0]), 2, 2);
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
            cols: 3,
            rows: 2,
            blocks: vec![Block {
                layer: 0,
                cells: vec![1, 2, 3, 4, 5, EMPTY_CELL],
                tfms: vec![0, 0, 0, 0, 0, 0],
            }],
        };
        let mut maps = vec![MapLayer::blank("wild", 8, 8)];
        let (n, erased, sel) = clip_move(&mut maps, &[true], &clip, None, 0, 0);
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
