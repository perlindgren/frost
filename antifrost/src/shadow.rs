//! The hill's dug geometry: the dig watch that times each
//! dissolve, and the occluder merge that packs the solids into the
//! fewest shadow boxes the cover can prove. Pure functions on cell
//! lists — no window, no scene — with the tests that hold the cover
//! exact beside them.

use crate::*;

/// One frame of the dig watch, kept whole and pure so it can be
/// tested without a running hill. `seen` is the set of dirt cells as
/// of the last frame, `now` as of this one, `digs` the live list of
/// `(col, row, seconds left, seed)` carried over. Dirt that vanished
/// between the two — eaten by the player's move, since no ant digs —
/// starts a dissolve at the full hop length, so the eat finishes as
/// the step lands; dirt that came back (an undo restores the tile)
/// cancels a live dig at once. `left` counts down by `dt` and a dig
/// whose tile has fully dissolved is dropped. The caller then stores
/// `now` back as the watch's `seen`. Seeds come from the cell so no
/// two holes eat with the same pattern.
pub(crate) fn watch_digs(
    digs: &mut Vec<(i32, i32, f32, f32)>,
    seen: &[(i32, i32)],
    now: &[(i32, i32)],
    dt: f32,
) {
    let gone = |c: &(i32, i32)| !now.contains(c);
    let back = |c: &(i32, i32)| !seen.contains(c) && now.contains(c);
    // Age the live digs, then drop the ones that finished and the ones
    // whose dirt came back.
    digs.retain_mut(|(_, _, left, _)| {
        *left -= dt;
        *left > 0.0
    });
    digs.retain(|(col, row, _, _)| !back(&(*col, *row)));
    // Every cell that vanished this frame begins a dig — unless one is
    // already running on it (a cell dug, undone, and dug again inside
    // one dissolve inherits the running countdown rather than
    // restarting behind it).
    for cell in seen.iter().copied().filter(gone) {
        if !digs.iter().any(|(c, r, _, _)| (*c, *r) == cell) {
            let (col, row) = cell;
            let seed = (col as f32 * 12.9 + row as f32 * 78.2).sin().abs() % 1.0;
            digs.push((col, row, DIG_HOP, seed));
        }
    }
}

/// The dirt cells of a hill, in the same row-major order the tile map
/// walks them — the digest the dig watch diffs frame to frame. Rock
/// stays out (it never dissolves); this is loose earth only.
pub(crate) fn dirt_cells(g: &game::Game) -> Vec<(i32, i32)> {
    (0..g.level.rows as i32)
        .flat_map(|row| (0..g.level.cols as i32).map(move |col| (col, row)))
        .filter(|&(col, row)| matches!(g.tile(game::Cell::new(col, row)), level::Tile::Dirt))
        .collect()
}

/// A merged occluder: columns `c0..=c1`, rows `r0..=r1`, inclusive grid
/// cells — the shape the cache holds and each frame packs into a world
/// rectangle.
pub(crate) type OccBox = (i32, i32, i32, i32);

/// The seed cover the fixpoint starts from, on its own so the bench can
/// weigh the two against each other: maximal runs per row — `tiles`
/// arrives row-major, so a run is a walk while the row holds and the
/// column stays adjacent — then a stack of equal spans merged upward.
/// Sorted by span, then row, so a stack of equal spans — the frame's
/// sides, a pile's flank — sits adjacent in line for the upward merge.
/// A row-major scan could never see it: a row with two runs puts the
/// other column's run between the stack's own links, and the stack
/// stayed one box per row, leaking a hairline of light through every
/// seam it drew.
pub(crate) fn seed_cover(tiles: &[(i32, i32, bool)]) -> Vec<OccBox> {
    let mut runs: Vec<(i32, i32, i32)> = Vec::new();
    let mut i = 0;
    while i < tiles.len() {
        let (row, col_s, _) = tiles[i];
        let mut col_e = col_s;
        let mut j = i + 1;
        while j < tiles.len() && tiles[j].0 == row && tiles[j].1 == col_e + 1 {
            col_e = tiles[j].1;
            j += 1;
        }
        runs.push((col_s, col_e, row));
        i = j;
    }
    runs.sort_unstable();
    let mut boxes: Vec<OccBox> = Vec::new();
    let mut k = 0;
    while k < runs.len() {
        let (col_s, col_e, row0) = runs[k];
        let mut row1 = row0;
        let mut m = k + 1;
        while m < runs.len() && runs[m].0 == col_s && runs[m].1 == col_e && runs[m].2 == row1 + 1 {
            row1 = runs[m].2;
            m += 1;
        }
        boxes.push((col_s, col_e, row0, row1));
        k = m;
    }
    boxes
}

/// The occluder-merge cache, as a pure function so the tests can walk it.
///
/// A box is `(col_s, col_e, row0, row1)`; the cover must hold every solid
/// cell in some box and no box may hold an empty one — the hill's shadow
/// is exactly the hill, and a box over a dug cell would wall light off
/// from a tunnel. The seed cover is the two-stage scan the cache has
/// always run: maximal runs per row, then a stack of equal spans merged
/// upward. The fixpoint pass comes after it, and it sees what the seed
/// cannot: two boxes of equal span split by a row whose run is wider —
/// the bar of a cross, the headroom of a shaft — leaves that span's
/// column stranded in two boxes, and whenever the stranded cells lie
/// solid the pair is one box. So the pass loops, merging any pair whose
/// bounding rectangle lies wholly inside the solids, until a round
/// merges nothing.
///
/// The rule keeps the cover exact without consulting any other box: the
/// merged rectangle holds only solid cells, and the pair's union already
/// covered those cells' rows and columns, so every cell that was covered
/// stays covered and nothing new joins the shadow — a merge may overlap
/// its neighbours freely. Every merge deletes a box, so the rounds
/// terminate, and the cover only ever shrinks toward the fewest boxes
/// this rule can prove.
pub(crate) fn merge_boxes(tiles: &[(i32, i32, bool)]) -> Vec<OccBox> {
    use std::collections::HashSet;
    let solid: HashSet<(i32, i32)> = tiles.iter().map(|&(row, col, _)| (row, col)).collect();
    let mut boxes = seed_cover(tiles);

    // The fixpoint. A pair may merge when its bounding rectangle is all
    // solid cells; one merge per round, rescanned, so a merge that
    // stretches a box is given its chance to enable the next. The count
    // falls by one each time, so the rounds end.
    let solid_rect = |c0: i32, c1: i32, r0: i32, r1: i32| {
        (r0..=r1).all(|row| (c0..=c1).all(|col| solid.contains(&(row, col))))
    };
    loop {
        let mut found: Option<(usize, usize, OccBox)> = None;
        'pairs: for a in 0..boxes.len() {
            for b in a + 1..boxes.len() {
                let (c0, c1) = (boxes[a].0.min(boxes[b].0), boxes[a].1.max(boxes[b].1));
                let (r0, r1) = (boxes[a].2.min(boxes[b].2), boxes[a].3.max(boxes[b].3));
                // Two boxes are one box exactly when the rectangle that
                // bounds the pair holds only solid cells — the all-solid
                // test is also the whole exactness argument: no empty is
                // ever covered. A rectangle no larger than `a` itself
                // says `b` already lies inside `a`; nothing is to gain.
                let rect = (c0, c1, r0, r1);
                if rect != boxes[a] && solid_rect(c0, c1, r0, r1) {
                    found = Some((a, b, rect));
                    break 'pairs;
                }
            }
        }
        let Some((a, b, rect)) = found else { break };
        boxes.remove(b);
        boxes.remove(a);
        boxes.push(rect);
    }
    boxes
}

#[cfg(test)]
mod dig_tests {
    use super::*;

    /// The watch's arithmetic, frame by frame: a dig is born the
    /// frame its dirt vanishes, at exactly the hop's length; it ages
    /// with the clock; and the frame its countdown runs out is the
    /// frame it is gone — never a node outliving its tile.
    #[test]
    fn a_dig_is_born_at_full_hop_and_eaten_by_its_end() {
        let mut digs: Vec<(i32, i32, f32, f32)> = Vec::new();
        let mut seen: Vec<(i32, i32)> = Vec::new();
        let full = [(3, 4)];
        // Dirt arriving is not a dig: a hill loaded is a hill with
        // dirt, not a field of holes opening.
        watch_digs(&mut digs, &seen, &full, 1.0 / 60.0);
        seen = full.to_vec();
        assert!(digs.is_empty());
        // The move: the cell vanishes this frame, and the digest is
        // born whole — its first tick is next frame's.
        watch_digs(&mut digs, &seen, &[], 1.0 / 60.0);
        seen = Vec::new();
        assert_eq!(digs.len(), 1);
        assert_eq!(digs[0].0, 3);
        assert_eq!(digs[0].1, 4);
        assert!(
            (digs[0].2 - DIG_HOP).abs() < 1e-6,
            "born at the full dig hop"
        );
        // The clock carries it; the seed rode from the first frame,
        // so the pattern never shifts mid-bite. One tick spent is one
        // tick spent — exact to the f32, plus the epsilon a
        // subtract-and-compare deserves.
        let seed = digs[0].3;
        watch_digs(&mut digs, &seen, &[], 1.0 / 60.0);
        assert!(digs[0].2 <= DIG_HOP - 1.0 / 60.0 + 1e-6);
        assert_eq!(digs[0].3, seed);
        // And it ends: past the dig hop's length the list empties,
        // because the tile dissolved with the step that landed. Ninety
        // frames at 60 fps is a second and a half, comfortably past
        // the dig hop's four ordinary hops.
        for _ in 0..90 {
            watch_digs(&mut digs, &seen, &[], 1.0 / 60.0);
        }
        assert!(digs.is_empty(), "a dissolved tile outlives no node");
    }

    /// An undo puts the dirt back mid-bite: the tile stands again,
    /// so the digest that was eating it stops — and one tile keeps
    /// one node, never a stack of eaters on the same square.
    #[test]
    fn restored_dirt_cancels_and_undedirt_does_not_stack() {
        let mut digs: Vec<(i32, i32, f32, f32)> = Vec::new();
        let mut seen: Vec<(i32, i32)> = Vec::new();
        let full = [(1, 1)];
        // Dig.
        watch_digs(&mut digs, &seen, &full, 0.01);
        seen = full.to_vec();
        watch_digs(&mut digs, &seen, &[], 0.01);
        seen = Vec::new();
        assert_eq!(digs.len(), 1);
        // The undo: the hill has the cell again, the digest stops.
        watch_digs(&mut digs, &seen, &full, 0.01);
        seen = full.to_vec();
        assert!(digs.is_empty(), "the tile stands, the eat stops");
        // Dug again after the cancel: one fresh digest, and the same
        // gone picture re-offered changes nothing — one tile is one
        // node, and a cell only ever vanishes once per digest.
        watch_digs(&mut digs, &seen, &[], 0.01);
        seen = Vec::new();
        assert_eq!(digs.len(), 1);
        // Same picture again — `seen` never re-offers the gone cell,
        // so the single dig just ages.
        watch_digs(&mut digs, &seen, &[], 0.01);
        assert_eq!(digs.len(), 1);
    }
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    /// A solid-cell list in the shape's own coordinates, row-major like
    /// the level's, in the tuple the cache keys on.
    fn cells(list: &[(i32, i32)]) -> Vec<(i32, i32, bool)> {
        let mut v: Vec<(i32, i32, bool)> =
            list.iter().map(|&(row, col)| (row, col, true)).collect();
        v.sort();
        v
    }

    /// The cover contract, asserted rather than assumed: the boxes hold
    /// every solid cell and no box holds an empty one — the shadow is
    /// exactly the hill, in whichever fewest boxes the merge can prove.
    fn assert_exact_cover(list: &[(i32, i32)], boxes: &[OccBox]) {
        let solid: std::collections::HashSet<(i32, i32)> = list.iter().copied().collect();
        let union: std::collections::HashSet<(i32, i32)> = boxes
            .iter()
            .flat_map(|&(c0, c1, r0, r1)| {
                (r0..=r1).flat_map(move |r| (c0..=c1).map(move |c| (r, c)))
            })
            .collect();
        assert_eq!(union, solid, "the boxes must cover the cells exactly");
    }

    fn merged(list: &[(i32, i32)]) -> Vec<OccBox> {
        let boxes = merge_boxes(&cells(list));
        assert_exact_cover(list, &boxes);
        boxes
    }

    #[test]
    fn rectangles_still_merge_to_one_box() {
        let rect: Vec<(i32, i32)> = (0..4).flat_map(|r| (0..7).map(move |c| (r, c))).collect();
        assert_eq!(merged(&rect), vec![(0, 6, 0, 3)], "a pile is one box");
    }

    #[test]
    fn a_frame_stays_four_bars() {
        // Rock's ring: no merge may ever cross its hollow middle.
        let mut ring: Vec<(i32, i32)> = Vec::new();
        for r in 0..5 {
            for c in 0..5 {
                if r == 0 || r == 4 || c == 0 || c == 4 {
                    ring.push((r, c));
                }
            }
        }
        let boxes = merged(&ring);
        assert_eq!(boxes.len(), 4, "top, bottom, and the two sides");
    }

    #[test]
    fn the_fixpoint_sees_across_the_wider_row() {
        // The plus: the seed's stack cannot link the column across the
        // bar, because the bar's run is wider than the column's span.
        // The fixpoint asks only whether the stranded cell is solid —
        // and it is — so the post becomes one box beside the bar.
        let plus = [(0, 1), (1, 0), (1, 1), (1, 2), (2, 1)];
        let mut boxes = merged(&plus);
        boxes.sort_unstable();
        assert_eq!(
            boxes,
            vec![(0, 2, 1, 1), (1, 1, 0, 2)],
            "a bar and a post, two boxes, exact"
        );
    }

    #[test]
    fn an_h_merges_its_stranded_posts() {
        // Two columns joined by a middle row: the seed cuts both posts
        // in four; the fixpoint welds each post whole.
        let h: Vec<(i32, i32)> = (0..3)
            .flat_map(|r| [(r, 0), (r, 2)])
            .chain([(1, 1)])
            .collect();
        let mut boxes = merged(&h);
        boxes.sort_unstable();
        // Three boxes is the floor, and both shapes of cover reach it:
        // the bar may shrink to its middle cell or keep its full width
        // and overlap the posts — a cover is a union, and the merge
        // never saw a reason to cut the bar's ends off. What the seed
        // could not do and the fixpoint can: weld each stranded post
        // whole across the bar's wider row.
        assert_eq!(boxes.len(), 3, "two whole posts and the bar");
        assert!(
            boxes.contains(&(0, 0, 0, 2)) && boxes.contains(&(2, 2, 0, 2)),
            "each post stands as one box: {boxes:?}"
        );
    }

    #[test]
    fn a_dug_gap_still_divides() {
        // The merge must never wall a tunnel: two posts split by a dug
        // cell stay two boxes whatever the fixpoint tries.
        let split = [(0, 0), (1, 0), (3, 0), (4, 0)];
        let mut boxes = merged(&split);
        boxes.sort_unstable();
        assert_eq!(
            boxes,
            vec![(0, 0, 0, 1), (0, 0, 3, 4)],
            "the empty row is a wall the merge cannot cross"
        );
    }

    #[test]
    fn a_checkerboard_yields_its_cells() {
        // No two cells share a bounding rectangle that lies solid: the
        // fixpoint must leave every cell its own box.
        let board: Vec<(i32, i32)> = (0..4)
            .flat_map(|r| {
                (0..4)
                    .filter(move |c| (r + c) % 2 == 0)
                    .map(move |c| (r, c))
            })
            .collect();
        assert_eq!(merged(&board).len(), board.len(), "no merge is legal here");
    }

    #[test]
    fn empty_hill_no_boxes() {
        assert_eq!(merge_boxes(&[]), vec![]);
    }

    /// The house rule from `docs/benchmarking.md` §5, applied where the
    /// fixpoint lives: A/B the mechanism, on the real hills and on a
    /// worst case, and report the numbers whatever they say. The seed
    /// cover is the old merge; `merge_boxes` is the seed plus the
    /// fixpoint; the two differ by exactly the pass under test.
    #[test]
    fn the_fixpoint_cut_is_measured_not_claimed() {
        // Every shipped hill, as the cache will see it.
        let levels = level::load_levels(include_str!("../levels.ron"));
        let mut shipped_gain = 0usize;
        for lvl in &levels {
            let tiles: Vec<(i32, i32, bool)> = (0..lvl.rows as i32)
                .flat_map(|row| (0..lvl.cols as i32).map(move |col| (row, col)))
                .filter_map(|(row, col)| {
                    let t = lvl.tiles[row as usize * lvl.cols + col as usize];
                    matches!(t, level::Tile::Rock | level::Tile::Dirt).then_some((
                        row,
                        col,
                        matches!(t, level::Tile::Rock),
                    ))
                })
                .collect();
            let before = seed_cover(&tiles).len();
            let after = merge_boxes(&tiles);
            let list: Vec<(i32, i32)> = tiles.iter().map(|&(r, c, _)| (r, c)).collect();
            assert_exact_cover(&list, &after);
            shipped_gain += before - after.len();
            eprintln!(
                "merge bench: {}: {before} -> {} boxes",
                lvl.name,
                after.len()
            );
        }

        // The worst case the fixpoint exists for: a dug hill whose
        // tunnels cross, stranding every pillar the seed cuts loose.
        let mut dug: Vec<(i32, i32)> = Vec::new();
        for r in 0..24 {
            for c in 0..32 {
                if r % 4 != 2 || c % 5 != 3 {
                    dug.push((r, c));
                }
            }
        }
        let before = seed_cover(&cells(&dug)).len();
        let after = merged(&dug);
        eprintln!(
            "merge bench: crossing tunnels 24x32: {before} -> {} boxes",
            after.len()
        );
        assert!(
            after.len() * 2 < before,
            "the fixpoint must at least halve a field of stranded pillars: \
             {before} -> {}",
            after.len()
        );
        // And the shipped hills are reported honestly whatever they say.
        let _ = shipped_gain;
    }
}
