//! The sidecar's positions: the two-number tuples the tree names
//! and the markers they wear on the sprite.
// The subjects this one reads.
use frost::ron as ron_tree;

pub(crate) const SPOTS_MAX: usize = 24;

pub(crate) const SPOT_ORDER: f32 = 2.9;

/// The seat numbers beside list-entry markers: deliberately larger
/// than the row text — they're read at a glance over the art, where
/// the palette dot alone is not enough.
pub(crate) const SEAT_SIZE: f32 = 20.0;

/// How close a right-click must land to a marker (window pixels) for
/// it to count as a click ON the position: a touch larger than even
/// the swollen marker, so picking is forgiving.
pub(crate) const SPOT_HIT_R: f32 = 16.0;

/// A position the sidecar names: its fold-path in the tree (the very
/// paths `walk` speaks, so marker and rows share an address), its pixel
/// coordinates, and the label the status line shows. The scan index is
/// the palette slot.
pub(crate) struct Spot {
    pub(crate) path: Vec<u16>,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) label: String,
    /// The entry number when the spot lives directly in a sequence —
    /// the sprite view prints it beside the marker.
    pub(crate) idx: Option<usize>,
}

/// Every position in the tree: a struct whose exactly two fields are
/// both numeric atoms — the plant files' `(x, y)` anchors and flowers,
/// in any nesting. Sequence children label as `parent[n]`, fields by
/// name; positions never nest deeper (their tuples are leaves).
pub(crate) fn scan_spots(v: &ron_tree::Val, path: &mut Vec<u16>, label: &str, out: &mut Vec<Spot>) {
    if out.len() >= SPOTS_MAX {
        return;
    }
    let in_seq = matches!(v, ron_tree::Val::Seq { .. });
    for (n, (key, kid)) in v.kids().iter().enumerate() {
        path.push(n as u16);
        let here = if key.is_empty() {
            format!("{label}[{n}]")
        } else {
            key.trim_end_matches([':', ' ']).to_string()
        };
        if let Some((x, y)) = spot_coords(kid) {
            out.push(Spot {
                path: path.clone(),
                x,
                y,
                label: here,
                idx: in_seq.then_some(n),
            });
        } else {
            scan_spots(kid, path, &here, out);
        }
        path.pop();
        if out.len() >= SPOTS_MAX {
            return;
        }
    }
}

/// The coordinates when a value is a position: two fields, both numeric
/// atoms that read back as floats.
pub(crate) fn spot_coords(v: &ron_tree::Val) -> Option<(f32, f32)> {
    let ron_tree::Val::Struct { fields, .. } = v else {
        return None;
    };
    if fields.len() != 2 {
        return None;
    }
    let nums: Vec<f32> = fields
        .iter()
        .filter_map(|(_, it)| match &it.val {
            ron_tree::Val::Atom(t, ron_tree::Kind::Num) => t.parse().ok(),
            _ => None,
        })
        .collect();
    (nums.len() == 2).then(|| (nums[0], nums[1]))
}

/// The spot a tree row describes: the row IS the position (its head or
/// close row), or it is one of the position's two coordinate rows.
pub(crate) fn spot_of_row(spots: &[Spot], row: &[u16]) -> Option<usize> {
    spots.iter().position(|s| {
        row == s.path.as_slice()
            || (row.len() == s.path.len() + 1
                && row[..s.path.len()] == s.path[..]
                && row[s.path.len()] < 2)
    })
}

/// A position's two numbers written back as one-decimal atoms, the way a
/// click on the sprite writes them.
pub(crate) fn write_spot(v: &mut ron_tree::Val, x: f32, y: f32) {
    if let ron_tree::Val::Struct { fields, .. } = v {
        for (n, f) in fields.iter_mut().enumerate().take(2) {
            f.1.val = ron_tree::Val::Atom(format!("{:.1}", [x, y][n]), ron_tree::Kind::Num);
        }
    }
}

/// A crop's job for a sidecar: every position moves by the cut's origin
/// `(dx, dy)`, clamped into the cropped texture's bounds — so a point
/// keeps its place relative to the pixels that survive, and a point the
/// cut eats lands on the new edge. The number of positions touched.
pub(crate) fn shift_spots(root: &mut ron_tree::Val, dx: f32, dy: f32, bounds: (u32, u32)) -> usize {
    let mut spots = Vec::new();
    scan_spots(root, &mut Vec::new(), "", &mut spots);
    let mut n = 0;
    for spot in &spots {
        let (cx, cy) = (
            (spot.x - dx).clamp(0.0, bounds.0 as f32),
            (spot.y - dy).clamp(0.0, bounds.1 as f32),
        );
        if let Some(v) = ron_tree::walk(root, &spot.path) {
            write_spot(v, cx, cy);
            n += 1;
        }
    }
    n
}

/// The pan offset that puts texture point `(x, y)` at the work area's
/// centre — the click map solved backwards for the offset (the strip's
/// vertical shift cancels: the point lands at the work area's centre,
/// wherever that sits, so the offset needs no term for it; the texture
/// measures y down while the window measures it up, hence the second
/// line's opposite sign).
pub(crate) fn center_offset(spot: (f32, f32), size: [f32; 2], zoom: f32) -> [f32; 2] {
    [
        (size[0] / 2.0 - spot.0) * zoom,
        (spot.1 - size[1] / 2.0) * zoom,
    ]
}

#[cfg(test)]
pub(crate) fn spots_of(root: &ron_tree::Val) -> Vec<(String, f32, f32)> {
    let mut fresh = Vec::new();
    scan_spots(root, &mut Vec::new(), "", &mut fresh);
    fresh.iter().map(|s| (s.label.clone(), s.x, s.y)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{WORK_Y, tex_point};

    #[test]
    fn spots_are_the_two_number_tuples_of_the_tree() {
        let root = ron_tree::parse(
            "(segment: 1, image: \"p.png\", lower: (317.0, 671.0), ups: [(1.0, 2.0), (3.5, 4.5)])",
        )
        .expect("a small sidecar");
        let mut spots = Vec::new();
        scan_spots(&root, &mut Vec::new(), "", &mut spots);
        let labels: Vec<&str> = spots.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["lower", "ups[0]", "ups[1]"]);
        assert_eq!((spots[0].x, spots[0].y), (317.0, 671.0));
        // List entries carry their seat for the sprite-view label; a
        // plain field carries none.
        assert_eq!(
            spots.iter().map(|s| s.idx).collect::<Vec<_>>(),
            [None, Some(0), Some(1)]
        );
        // The head row and either coordinate row belong to the spot;
        // an unrelated path belongs to none.
        assert_eq!(spot_of_row(&spots, &spots[0].path), Some(0));
        let mut coord = spots[1].path.clone();
        coord.push(1);
        assert_eq!(spot_of_row(&spots, &coord), Some(1));
        assert_eq!(spot_of_row(&spots, &[9]), None);
    }

    #[test]
    fn a_crop_shifts_every_position_by_the_cut_origin() {
        let mut root = ron_tree::parse(
            "(segment: 1, lower_anchor: (317.0, 600.0), \
            upper_anchor: (317.0, 578.0), flower_anchors: [(308.0, 615.0)])",
        )
        .expect("a small sidecar");
        // The cut's origin (100, 50) leaves a 630 x 620 texture: every
        // point keeps its place relative to the pixels that survive.
        let n = shift_spots(&mut root, 100.0, 50.0, (630, 620));
        assert_eq!(n, 3);
        let mut spots = Vec::new();
        scan_spots(&root, &mut Vec::new(), "", &mut spots);
        assert_eq!((spots[0].x, spots[0].y), (217.0, 550.0));
        assert_eq!((spots[1].x, spots[1].y), (217.0, 528.0));
        assert_eq!((spots[2].x, spots[2].y), (208.0, 565.0));
    }

    #[test]
    fn the_cut_clamps_the_positions_it_eats_onto_the_new_edge() {
        let mut root = ron_tree::parse("(a: (10.0, 700.0), b: (300.0, 660.0))").expect("parses");
        // (10 - 100, 700 - 50) = (-90, 650) is outside the new 300 x 600
        // bounds altogether; (300 - 100, 660 - 50) = (200, 610) is inside
        // the width but past the new bottom.
        let n = shift_spots(&mut root, 100.0, 50.0, (300, 600));
        assert_eq!(n, 2);
        let mut spots = Vec::new();
        scan_spots(&root, &mut Vec::new(), "", &mut spots);
        assert_eq!((spots[0].x, spots[0].y), (0.0, 600.0));
        assert_eq!((spots[1].x, spots[1].y), (200.0, 600.0));
    }

    #[test]
    fn centering_puts_the_point_under_the_eye() {
        let [x, y] = center_offset((30.0, 20.0), [100.0, 100.0], 2.0);
        // The click map, run forward from the work centre, must land
        // back on the very texture point that was centred.
        let view = [x, y + WORK_Y];
        let [px, py] = tex_point([0.0, WORK_Y], [100.0, 100.0], view, 2.0);
        assert_eq!((px, py), (30.0, 20.0));
    }
}
