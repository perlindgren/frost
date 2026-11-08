//! Popup geometry in window coordinates (center origin, y up).
//!
//! Two placements share one measurement: [`DropdownLayout::new`] hangs the list
//! below or above the control that opened it, which is what a settings row needs,
//! and [`DropdownLayout::centred`] puts the same list in the middle of the window,
//! which is what a control drawn on the main view needs — there is no row beneath
//! such a control to hang anything from, and a popup that appeared at the window's
//! edge would point at nothing.

pub struct DropdownLayout {
    pub center: [f32; 2],
    pub half: [f32; 2],
    pub border: f32,
    pub item_x: f32,
    pub item_half: [f32; 2],
    pub visible: usize,
    pub track_x: f32,
    pub track_half: [f32; 2],
    first_y: f32,
    step: f32,
}

/// The window's own allowances, in the same terms whichever placement follows.
struct Frame {
    margin: f32,
    border: f32,
    inset: f32,
    gap: f32,
    row_gap: f32,
    normal_height: f32,
    top: f32,
    bottom: f32,
}

impl Frame {
    fn new(window: [f32; 2], fit: f32) -> Self {
        let shortest = window[0].min(window[1]);
        let margin = (6.0 * fit).min(shortest / 4.0);
        let border = (1.5 * fit).min(shortest / 16.0);
        let pad = (9.0 * fit).min(shortest / 16.0);
        let top = window[1] / 2.0 - margin;
        Self {
            margin,
            border,
            inset: border + pad,
            gap: 4.0 * fit,
            row_gap: 5.0 * fit,
            normal_height: 26.0 * fit,
            top,
            bottom: -top,
        }
    }
}

impl DropdownLayout {
    /// Below `anchor` when the list fits there, otherwise on the roomier side of
    /// it — the behaviour a settings row has always had.
    pub fn new(
        window: [f32; 2],
        anchor: [f32; 2],
        button_half: [f32; 2],
        fit: f32,
        count: usize,
    ) -> Self {
        let frame = Frame::new(window, fit);
        let below = (anchor[1] - button_half[1] - frame.gap - frame.bottom).max(0.0);
        let above = (frame.top - anchor[1] - button_half[1] - frame.gap).max(0.0);
        let desired = count as f32 * (frame.normal_height + frame.row_gap) - frame.row_gap
            + 2.0 * frame.inset;
        // Prefer below when everything fits; otherwise use the roomier side.
        let upwards = below < desired && above > below;
        let space = if upwards { above } else { below };
        let mut layout = Self::arrange(window, &frame, space, button_half[0], fit, count);
        let x = anchor[0].clamp(
            -window[0] / 2.0 + frame.margin + layout.half[0],
            window[0] / 2.0 - frame.margin - layout.half[0],
        );
        let y = if upwards {
            anchor[1] + button_half[1] + frame.gap + layout.half[1]
        } else {
            anchor[1] - button_half[1] - frame.gap - layout.half[1]
        }
        .clamp(frame.bottom + layout.half[1], frame.top - layout.half[1]);
        layout.move_to([x, y]);
        layout
    }

    /// The middle of the window, for a control that has no row to hang from. The
    /// list still clamps to the window, so a long one is scrollable rather than
    /// clipped, and a short one stays put.
    pub fn centred(window: [f32; 2], button_half: [f32; 2], fit: f32, count: usize) -> Self {
        let frame = Frame::new(window, fit);
        let mut layout = Self::arrange(
            window,
            &frame,
            frame.top - frame.bottom,
            button_half[0],
            fit,
            count,
        );
        layout.move_to([0.0, 0.0]);
        layout
    }

    /// Everything that does not depend on where the popup goes: how many rows fit
    /// in `available`, how tall they are, and how wide the box may be.
    fn arrange(
        window: [f32; 2],
        frame: &Frame,
        available_space: f32,
        button_half_width: f32,
        fit: f32,
        count: usize,
    ) -> Self {
        let available = available_space
            .max(frame.normal_height + 2.0 * frame.inset)
            .min(frame.top - frame.bottom);
        let content = (available - 2.0 * frame.inset).max(0.0);
        let row_height = frame.normal_height.min(content);
        let step = row_height + frame.row_gap;
        let visible = (((content + frame.row_gap) / step).floor() as usize)
            .max(1)
            .min(count);
        let half_height =
            ((visible as f32 * step - frame.row_gap).max(0.0) + 2.0 * frame.inset) / 2.0;
        let half_width = (button_half_width + frame.inset).min(window[0] / 2.0 - frame.margin);
        let content_half_width = (half_width - frame.inset).max(0.0);
        let scrollbar = if count > visible {
            (10.0 * fit).min(content_half_width)
        } else {
            0.0
        };
        Self {
            center: [0.0, 0.0],
            half: [half_width, half_height],
            border: frame.border,
            item_x: -scrollbar / 2.0,
            item_half: [content_half_width - scrollbar / 2.0, row_height / 2.0],
            visible,
            track_x: content_half_width - scrollbar / 2.0,
            track_half: [scrollbar / 2.0, (half_height - frame.inset).max(0.0)],
            first_y: half_height - frame.inset - row_height / 2.0,
            step,
        }
    }

    /// Re-anchor an arranged layout. Everything except `step` is an offset from
    /// the centre, so moving it is a translation and nothing is re-measured.
    fn move_to(&mut self, center: [f32; 2]) {
        let [x, y] = center;
        self.center = center;
        self.item_x += x;
        self.track_x += x;
        self.first_y += y;
    }

    pub fn row_y(&self, slot: usize) -> f32 {
        self.first_y - slot as f32 * self.step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_and_all_visible_rows_stay_inside_resized_windows() {
        for [w, h] in [
            [480.0f32, 320.0],
            [640.0, 400.0],
            [1280.0, 720.0],
            [240.0, 120.0],
            [300.0, 900.0],
        ] {
            let fit = (w / 720.0).min(h / 440.0).max(0.45);
            for row in 0..13 {
                let anchor = [
                    w / 2.0 - 225.0 * fit,
                    h / 2.0 - 32.0 * fit - row as f32 * 24.0 * fit,
                ];
                for count in [1, 8, 64, 200] {
                    let layout =
                        DropdownLayout::new([w, h], anchor, [207.0 * fit, 11.0 * fit], fit, count);
                    assert!(layout.visible > 0 && layout.visible <= count);
                    for axis in 0..2 {
                        assert!(
                            layout.center[axis].abs() + layout.half[axis]
                                <= [w, h][axis] / 2.0 + 0.001
                        );
                    }
                    for slot in 0..layout.visible {
                        assert!(layout.row_y(slot).abs() + layout.item_half[1] <= h / 2.0);
                    }
                }
            }
        }
    }

    #[test]
    fn capacity_grows_with_window_and_lower_controls_open_upwards() {
        let small = DropdownLayout::new([480.0, 320.0], [100.0, 120.0], [120.0, 10.0], 1.0, 100);
        let large = DropdownLayout::new([900.0, 800.0], [100.0, 120.0], [120.0, 10.0], 1.0, 100);
        assert!(large.visible > small.visible);
        let low = DropdownLayout::new([640.0, 400.0], [100.0, -150.0], [120.0, 10.0], 1.0, 100);
        assert!(low.center[1] - low.half[1] > -150.0);
    }

    /// A centred popup is where it says it is and stays inside the window, and a
    /// list short enough to fit anywhere measures exactly like the anchored one —
    /// the two placements share their measurement, so a row cannot grow in one
    /// view and not the other. A long list may show more rows centred, which is the
    /// reason for asking for the middle.
    #[test]
    fn a_centred_popup_sits_in_the_middle_and_agrees_with_the_anchored_one() {
        for [w, h] in [
            [480.0f32, 320.0],
            [640.0, 400.0],
            [1280.0, 720.0],
            [300.0, 900.0],
        ] {
            let fit = (w / 720.0).min(h / 440.0).max(0.45);
            for count in [1, 9, 64] {
                let centred =
                    DropdownLayout::centred([w, h], [150.0 * fit, 11.0 * fit], fit, count);
                assert_eq!(
                    centred.center,
                    [0.0, 0.0],
                    "not centred at {w}x{h} count {count}"
                );
                assert!(
                    centred.half[0] <= w / 2.0 && centred.half[1] <= h / 2.0,
                    "overflow at {w}x{h} count {count}"
                );
                for slot in 0..centred.visible {
                    assert!(centred.row_y(slot).abs() + centred.item_half[1] <= h / 2.0);
                }
                let anchored =
                    DropdownLayout::new([w, h], [0.0, 0.0], [150.0 * fit, 11.0 * fit], fit, count);
                // A list that fits in either place must measure identically; a long
                // list may show more rows in the middle, which is the point of the
                // extra room, never fewer.
                if count == 1 {
                    assert_eq!(
                        (anchored.visible, anchored.item_half, anchored.step),
                        (centred.visible, centred.item_half, centred.step),
                        "the two placements disagree where both have room at {w}x{h}"
                    );
                } else {
                    assert!(
                        centred.visible >= anchored.visible,
                        "the middle offered fewer rows than the same spot at {w}x{h} count {count}"
                    );
                }
            }
        }
        let [w, h] = [640.0f32, 400.0];
        let centred = DropdownLayout::centred([w, h], [150.0, 11.0], 1.0, 64);
        let anchored = DropdownLayout::new([w, h], [0.0, 0.0], [150.0, 11.0], 1.0, 64);
        assert!(
            centred.visible > anchored.visible,
            "centring bought nothing: {} rows against {}",
            centred.visible,
            anchored.visible
        );
    }
}

#[cfg(feature = "widgets")]
mod selector;
#[cfg(feature = "widgets")]
pub use selector::{DropdownResponse, Placement, draw_dropdown};
