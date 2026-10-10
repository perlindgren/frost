//! The slot strip: plates, thumbnails, its scroll view and knob.
// The subjects this one reads.
use crate::journal::*;

/// The strip's thumbnail pool: the most slots visible at once. A
/// deeper rack rides behind the strip's scrollbar.
pub(crate) const SLOTS: usize = 7;

/// A slot's edge, in pixels.
pub(crate) const SLOT: f32 = 100.0;

/// The gap between neighboring slots.
pub(crate) const SLOT_GAP: f32 = 8.0;

/// The slots strip's height: a slot's edge plus the margin above and
/// below it.
pub(crate) const STRIP_H: f32 = 124.0;

/// The distance from the window's left edge to the first slot's left edge.
pub(crate) const SLOT_MARGIN: f32 = 14.0;

/// The longest edge of a slot thumbnail's minimized copy, in pixels —
/// a slot's edge minus its inner padding.
pub(crate) const THUMB_MAX: f32 = 84.0;

/// The strip's visible window for a plate count: where it starts, how
/// wide the pool can show, and how far the scroll may run.
pub(crate) struct StripView {
    pub(crate) x0: f32,
    pub(crate) pitch: f32,
    pub(crate) visible_px: f32,
    pub(crate) track_w: f32,
    pub(crate) scroll_max: f32,
}

pub(crate) fn strip_view(w: f32, count: usize) -> StripView {
    let pitch = SLOT + SLOT_GAP;
    let x0 = -w / 2.0 + SLOT_MARGIN;
    let across = (((w - 2.0 * SLOT_MARGIN) + SLOT_GAP) / pitch).floor() as usize;
    let visible = across.clamp(1, SLOTS);
    let visible_px = visible as f32 * pitch - SLOT_GAP;
    let total_px = count.max(1) as f32 * pitch - SLOT_GAP;
    StripView {
        x0,
        pitch,
        visible_px,
        track_w: (w - 2.0 * SLOT_MARGIN).max(1.0),
        scroll_max: (total_px - visible_px).max(0.0),
    }
}

/// The strip's scrollbar rides the window's bottom edge.
pub(crate) const SCROLL_Y: f32 = 6.0;

pub(crate) const SCROLL_H: f32 = 8.0;

/// The knob's left edge and width at a scroll position.
pub(crate) fn scroll_knob(sv: &StripView, scroll: f32) -> (f32, f32) {
    let total = (sv.visible_px + sv.scroll_max).max(1.0);
    let kw = (sv.track_w * (sv.visible_px / total)).max(30.0);
    let x = sv.x0 + (scroll / sv.scroll_max.max(1.0)) * (sv.track_w - kw);
    (x, kw)
}

/// The usable floor: below this the window belongs to the status
/// band, and the strip, the work area and every panel stand on it.
pub(crate) fn floor_y(h: f32) -> f32 {
    -h / 2.0 + STATUS_H
}

/// The `i`th slot's center, in window coordinates, at a scroll offset.
pub(crate) fn slot_center(i: usize, w: f32, h: f32, scroll: f32) -> [f32; 2] {
    [
        -w / 2.0 + SLOT_MARGIN + SLOT / 2.0 + i as f32 * (SLOT + SLOT_GAP) - scroll,
        floor_y(h) + STRIP_H / 2.0,
    ]
}

/// The slot under a window point — among the `count` plates the strip
/// shows, and only within the pool's visible box.
pub(crate) fn slot_at(p: [f32; 2], w: f32, h: f32, scroll: f32, count: usize) -> Option<usize> {
    if p[1] > floor_y(h) + STRIP_H {
        return None;
    }
    let sv = strip_view(w, count);
    if p[0] < sv.x0 || p[0] > sv.x0 + sv.visible_px {
        return None;
    }
    (0..count).find(|i| {
        let [cx, cy] = slot_center(*i, w, h, scroll);
        (p[0] - cx).abs() <= SLOT / 2.0 && (p[1] - cy).abs() <= SLOT / 2.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{WORK_Y, in_work_area};

    #[test]
    fn slots_line_up_along_the_strip_and_are_hit_by_point() {
        let (w, h) = (800.0, 600.0);
        // The leftmost slot starts at the margin, flush with the strip.
        let [cx, cy] = slot_center(0, w, h, 0.0);
        assert_eq!(cx, -w / 2.0 + SLOT_MARGIN + SLOT / 2.0);
        assert_eq!(cy, floor_y(h) + STRIP_H / 2.0);
        // Neighbors sit one slot and one gap apart.
        let [cx1, _] = slot_center(1, w, h, 0.0);
        assert_eq!(cx1 - cx, SLOT + SLOT_GAP);
        // Each slot's center is its own; the gaps and the area above the
        // strip belong to no slot.
        for i in 0..SLOTS {
            let [x, y] = slot_center(i, w, h, 0.0);
            assert_eq!(slot_at([x, y], w, h, 0.0, i + 1), Some(i));
        }
        let [cx0, cy0] = slot_center(0, w, h, 0.0);
        assert_eq!(
            slot_at([cx0 + SLOT / 2.0 + SLOT_GAP / 2.0, cy0], w, h, 0.0, SLOTS),
            None
        );
        assert_eq!(
            slot_at([cx0, cy0 + STRIP_H / 2.0 + 1.0], w, h, 0.0, SLOTS),
            None
        );
    }

    #[test]
    fn the_strip_always_keeps_a_plate_free_to_load() {
        // One plate per sprite plus the spare: a handful of sprites
        // in a wide window all show, and nobody needs the scrollbar...
        let wide = strip_view(1440.0, 3);
        assert_eq!(wide.scroll_max, 0.0);
        // ...but the pool caps what one window can show, so the
        // plate past the pool's end — the seventh sprite's, or the
        // spare's — earns a scrollbar of exactly the overflow.
        let tight = strip_view(1440.0, SLOTS + 1);
        assert!(tight.scroll_max > 0.0);
    }

    #[test]
    fn the_scroll_moves_what_the_pick_names() {
        let (w, h) = (800.0, 600.0);
        let count = 9;
        let sv = strip_view(w, count);
        assert!(sv.scroll_max > 0.0);
        let pitch = SLOT + SLOT_GAP;
        let cy = floor_y(h) + STRIP_H / 2.0;
        // One pitch of scroll, and slot 1 stands where slot 0 stood:
        // plates, picks and thumbnails all read the same offset.
        let p = [-w / 2.0 + SLOT_MARGIN + SLOT / 2.0, cy];
        assert_eq!(slot_at(p, w, h, 0.0, count), Some(0));
        assert_eq!(slot_at(p, w, h, pitch, count), Some(1));
        // The knob runs the whole track over the scroll's span.
        let at_rest = scroll_knob(&sv, 0.0);
        let at_end = scroll_knob(&sv, sv.scroll_max);
        assert!((at_rest.0 - sv.x0).abs() < 1e-3);
        assert!((at_end.0 + at_end.1 - (sv.x0 + sv.track_w)).abs() < 1e-3);
    }

    #[test]
    fn the_strip_divides_the_work_area_from_the_slots() {
        let h = 600.0;
        let strip_top = floor_y(h) + STRIP_H;
        assert!(!in_work_area([0.0, strip_top - 1.0], 800.0, h));
        assert!(in_work_area([0.0, strip_top + 1.0], 800.0, h));
        // The work area's center is half a strip and half a band above
        // the bottom edge, so a sprite panned to it floats clear of
        // the slots and of the status bar alike.
        assert_eq!(WORK_Y, (STRIP_H + STATUS_H) / 2.0);
    }
}
