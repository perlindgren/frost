//! Work-area geometry: zoom, pan, texture-point mapping, and the
//! selection rectangle.
// The subjects this one reads.
use crate::journal::*;
use crate::strip::*;
use crate::world::*;

/// The lowest zoom: the sprite at a quarter of its texture size.
pub(crate) const ZOOM_MIN: f32 = 0.25;

/// The highest zoom: the sprite four times its texture size.
pub(crate) const ZOOM_MAX: f32 = 4.0;

/// The wheel's zoom factor per line of movement: the zoom is multiplied by
/// it, so scrolling up (positive delta) zooms in and scrolling down zooms
/// out.
pub(crate) const WHEEL_ZOOM: f32 = 1.15;

/// The controls panels' width, in pixels.
pub(crate) const PANEL_W: f32 = 300.0;

/// How far the left button may travel between press and release and still
/// count as a click (select a slot, or log a pixel) rather than a drag
/// (a selection rectangle in the work area, or a slot swap).
pub(crate) const CLICK_TOL: f32 = 4.0;

/// The work area's center as an offset: the band and the strip both
/// push the desk up, and the sprite centers between them and the top.
pub(crate) const WORK_Y: f32 = (STRIP_H + STATUS_H) / 2.0;

/// The atlas' slider ceiling: the most rows or columns one grid may
/// have. 16 x 16 tiles any sensible sprite.
pub(crate) const ATLAS_MAX: f32 = 16.0;

/// The grid's doubling ladder: while a cell's on-screen pitch drops
/// below this many window pixels, the drawn step doubles — so the grid
/// stays a few dozen lines at any zoom, always on whole cells.
pub(crate) const GRID_MIN_PX: f32 = 8.0;

/// Whether a window point is inside a rectangle.
pub(crate) fn in_rect(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3]
}

/// Whether a window point is in the work area — above the slots strip.
/// What keeps an open stroke writing: paint rides the left button;
/// the eraser rides the Delete key (or its partner, Ctrl) — held as
/// a drag (key and button) or, when the key opened it alone, the key
/// by itself.
pub(crate) fn stroke_held(st: &Paint, down: bool, del: bool) -> bool {
    if st.erase {
        del && (down || st.key_opened)
    } else {
        down
    }
}

/// Whether a window point lies in the work area (everything above
/// the slots strip).
pub(crate) fn in_work_area(p: [f32; 2], _w: f32, h: f32) -> bool {
    p[1] >= floor_y(h) + STRIP_H
}

/// The window point's position in the texture's pixel space: `(0, 0)`
/// upper-left, `x` right, `y` down — the sprite is centered at `view`
/// (its window-space center) and drawn at scale `zoom`.
pub(crate) fn tex_point(p: [f32; 2], size: [f32; 2], view: [f32; 2], zoom: f32) -> [f32; 2] {
    let [tw, th] = size;
    [
        (p[0] - view[0]) / zoom + tw / 2.0,
        th / 2.0 - (p[1] - view[1]) / zoom,
    ]
}

/// The selection rectangle between two window points, in texture pixels
/// and clamped to the texture; a rectangle under a pixel wide is no
/// selection at all.
pub(crate) fn sel_rect_from(
    a: [f32; 2],
    b: [f32; 2],
    size: [f32; 2],
    view: [f32; 2],
    zoom: f32,
) -> Option<[f32; 4]> {
    let [tw, th] = size;
    let ta = tex_point(a, size, view, zoom);
    let tb = tex_point(b, size, view, zoom);
    let x0 = ta[0].min(tb[0]).clamp(0.0, tw);
    let x1 = ta[0].max(tb[0]).clamp(0.0, tw);
    let y0 = ta[1].min(tb[1]).clamp(0.0, th);
    let y1 = ta[1].max(tb[1]).clamp(0.0, th);
    if x1 - x0 >= 1.0 && y1 - y0 >= 1.0 {
        Some([x0, y0, x1, y1])
    } else {
        None
    }
}

/// The selection's whole pixels: floor the top-left, ceil the bottom-right
/// (so a drag covers every pixel it touched), clamped to the texture;
/// `None` if nothing whole is inside.
pub(crate) fn sel_int_rect(sel: [f32; 4], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let x0 = sel[0].floor().max(0.0);
    let y0 = sel[1].floor().max(0.0);
    let x1 = sel[2].ceil().min(w as f32);
    let y1 = sel[3].ceil().min(h as f32);
    if x1 > x0 && y1 > y0 {
        Some((x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_and_texture_points_round_trip() {
        let size = [40.0, 20.0];
        let view = [7.0, -3.0];
        let zoom = 1.7;
        let [px, py] = tex_point([12.0, 5.0], size, view, zoom);
        // The inverse of `tex_point`, back to window pixels.
        let wx = (px - size[0] / 2.0) * zoom + view[0];
        let wy = (size[1] / 2.0 - py) * zoom + view[1];
        assert!((wx - 12.0).abs() < 1e-3, "x came back at {wx}");
        assert!((wy - 5.0).abs() < 1e-3, "y came back at {wy}");
    }

    #[test]
    fn selection_is_texture_pixels_normalized_and_clamped() {
        let size = [100.0, 50.0];
        // The sprite's center is the texture middle (50, 25); at zoom 2 a
        // 40 px drag covers 20 texture pixels.
        let sel = sel_rect_from([0.0, 0.0], [40.0, 20.0], size, [0.0, 0.0], 2.0).expect("a rect");
        assert_eq!(sel, [50.0, 15.0, 70.0, 25.0]);
        // A drag far beyond the texture clamps to its bounds.
        let sel = sel_rect_from([-1e3, 1e3], [1e3, -1e3], size, [0.0, 0.0], 1.0).expect("a rect");
        assert_eq!(sel, [0.0, 0.0, 100.0, 50.0]);
        // Under a texture pixel wide is no selection at all.
        assert!(sel_rect_from([10.0, 0.0], [10.5, 0.5], size, [0.0, 0.0], 1.0).is_none());
    }

    #[test]
    fn int_selection_floors_out_and_clamps() {
        assert_eq!(
            sel_int_rect([10.2, 4.7, 20.8, 9.9], 100, 100),
            Some((10, 4, 11, 6))
        );
        assert_eq!(
            sel_int_rect([95.5, 96.6, 99.5, 99.9], 100, 100),
            Some((95, 96, 5, 4))
        );
        assert_eq!(sel_int_rect([50.0, 50.0, 50.0, 50.0], 100, 100), None);
    }
}
