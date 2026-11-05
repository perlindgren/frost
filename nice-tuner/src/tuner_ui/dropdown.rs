//! Popup geometry in window coordinates (center origin, y up).

pub(super) struct DropdownLayout {
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

impl DropdownLayout {
    pub fn new(
        window: [f32; 2],
        anchor: [f32; 2],
        button_half: [f32; 2],
        fit: f32,
        count: usize,
    ) -> Self {
        let [w, h] = window;
        let shortest = w.min(h);
        let margin = (6.0 * fit).min(shortest / 4.0);
        let border = (1.5 * fit).min(shortest / 16.0);
        let pad = (9.0 * fit).min(shortest / 16.0);
        let inset = border + pad;
        let gap = 4.0 * fit;
        let row_gap = 5.0 * fit;
        let normal_height = 26.0 * fit;
        let top = h / 2.0 - margin;
        let bottom = -top;
        let below = (anchor[1] - button_half[1] - gap - bottom).max(0.0);
        let above = (top - anchor[1] - button_half[1] - gap).max(0.0);
        let desired = count as f32 * (normal_height + row_gap) - row_gap + 2.0 * inset;
        // Prefer below when everything fits; otherwise use the roomier side.
        let upwards = below < desired && above > below;
        let space = if upwards { above } else { below };
        let available = space.max(normal_height + 2.0 * inset).min(top - bottom);
        let content = (available - 2.0 * inset).max(0.0);
        let row_height = normal_height.min(content);
        let step = row_height + row_gap;
        let visible = (((content + row_gap) / step).floor() as usize)
            .max(1)
            .min(count);
        let half_height = ((visible as f32 * step - row_gap).max(0.0) + 2.0 * inset) / 2.0;
        let half_width = (button_half[0] + inset).min(w / 2.0 - margin);
        let x = anchor[0].clamp(
            -w / 2.0 + margin + half_width,
            w / 2.0 - margin - half_width,
        );
        let y = if upwards {
            anchor[1] + button_half[1] + gap + half_height
        } else {
            anchor[1] - button_half[1] - gap - half_height
        }
        .clamp(bottom + half_height, top - half_height);
        let content_half_width = (half_width - inset).max(0.0);
        let scrollbar = if count > visible {
            (10.0 * fit).min(content_half_width)
        } else {
            0.0
        };
        Self {
            center: [x, y],
            half: [half_width, half_height],
            border,
            item_x: x - scrollbar / 2.0,
            item_half: [content_half_width - scrollbar / 2.0, row_height / 2.0],
            visible,
            track_x: x + content_half_width - scrollbar / 2.0,
            track_half: [scrollbar / 2.0, (half_height - inset).max(0.0)],
            first_y: y + half_height - inset - row_height / 2.0,
            step,
        }
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
}
