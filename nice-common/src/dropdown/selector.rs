//! Reusable immediate-mode dropdown selector, independent of audio settings.
use super::DropdownLayout;
use frost::{Color, Context};

pub enum Placement {
    Row([f32; 2], [f32; 2]),
    Centred([f32; 2]),
}

#[derive(Default)]
pub struct DropdownResponse {
    pub picked: Option<usize>,
    pub consumed: bool,
}

/// Draw an open selector. The caller owns its rows, selected value and open/close
/// state; scroll and scrollbar dragging persist between frames. A returned index
/// refers directly to `items`, so applications retain control of their values.
#[allow(clippy::too_many_arguments)]
pub fn draw_dropdown(
    ctx: &mut Context,
    font: &std::sync::Arc<[u8]>,
    window: [f32; 2],
    fit: f32,
    place: Placement,
    z: f32,
    clicked: bool,
    items: &[(String, String)],
    selected: usize,
    def_raw: &str,
    title: &str,
    preserve_suffix: bool,
    scroll: &mut f32,
    dragging: &mut bool,
) -> DropdownResponse {
    if items.is_empty() {
        return DropdownResponse::default();
    }
    let mouse = ctx.mouse_position();
    // A named popup gets a row for its name, so the box is measured one row
    // taller and every item shifts down by it.
    let header = matches!(place, Placement::Centred(_));
    let extra = header as usize;
    let layout = match place {
        Placement::Row(at, half) => DropdownLayout::new(window, at, half, fit, items.len() + extra),
        Placement::Centred(half) => DropdownLayout::centred(window, half, fit, items.len() + extra),
    };
    let visible_items = layout.visible.saturating_sub(extra).max(1);
    let max_scroll = items.len().saturating_sub(visible_items) as f32;
    *scroll = (*scroll + ctx.mouse_wheel() * 0.5).clamp(0.0, max_scroll);
    // The list keeps its own order: opening does not lift the current value to
    // the top, which read as a re-sort. The scroll is only nudged when the
    // selection would otherwise be off the visible rows, and then only as far
    // as bringing it in - never to the front.
    let sel_idx = selected;
    let shown_from = scroll.floor() as usize;
    if sel_idx < shown_from {
        *scroll = (sel_idx as f32).min(max_scroll);
    } else if sel_idx >= shown_from + visible_items {
        *scroll = ((sel_idx + 1 - visible_items) as f32).clamp(0.0, max_scroll);
    }
    if !ctx.mouse_button_down(frost::MouseButton::Left) {
        *dragging = false;
    }
    let scrollable = max_scroll > 0.0;
    let thumb_half = if scrollable {
        (layout.track_half[1] * visible_items as f32 / items.len() as f32)
            .max(6.0 * fit)
            .min(layout.track_half[1])
    } else {
        0.0
    };
    let travel = (2.0 * (layout.track_half[1] - thumb_half)).max(0.0);
    let over_track = scrollable
        && point_in(
            mouse,
            layout.track_x,
            layout.center[1],
            layout.track_half[0],
            layout.track_half[1],
        );
    if clicked && over_track {
        *dragging = true;
    }
    if *dragging
        && scrollable
        && travel > 0.0
        && let Some([_, mouse_y]) = mouse
    {
        let fraction = ((layout.center[1] + layout.track_half[1] - thumb_half - mouse_y) / travel)
            .clamp(0.0, 1.0);
        *scroll = (fraction * max_scroll).round();
    }
    let skip = scroll.floor() as usize;
    let [px, py] = layout.center;
    let [pw, ph] = layout.half;
    ctx.rectangle(px, py, pw, ph, HOVER, z - 0.06);
    ctx.rectangle(
        px,
        py,
        (pw - layout.border).max(0.0),
        (ph - layout.border).max(0.0),
        MENU_BG,
        z - 0.055,
    );
    if scrollable {
        ctx.rectangle(
            layout.track_x,
            py,
            layout.track_half[0],
            layout.track_half[1],
            TRACK,
            z - 0.05,
        );
        let thumb_y = py + layout.track_half[1] - thumb_half - skip as f32 / max_scroll * travel;
        ctx.rectangle(
            layout.track_x,
            thumb_y,
            layout.track_half[0],
            thumb_half,
            if over_track || *dragging { AMBER } else { DIM },
            z - 0.04,
        );
    }
    let menu_size = (18.7 * fit).min(layout.item_half[1] * (18.7 / 13.0));
    let char_width = ctx.text_width(font, "0", menu_size, 700.0).max(fit);
    let max_chars = ((2.0 * layout.item_half[0] - 4.0 * fit) / char_width)
        .floor()
        .max(1.0) as usize;
    if header {
        let hy = layout.row_y(0);
        ctx.text(
            layout.item_x,
            hy,
            font,
            title,
            menu_size,
            700.0,
            DIM,
            z + 0.06,
        );
        // Half a step below the name is exactly midway to the first row.
        ctx.rectangle(
            layout.center[0],
            hy - (hy - layout.row_y(1)) / 2.0,
            layout.item_half[0],
            (1.0 * fit).max(0.5),
            Color { a: 0.5, ..TRACK },
            z + 0.06,
        );
    }
    for (slot, (idx, (label, raw))) in items
        .iter()
        .enumerate()
        .skip(skip)
        .take(visible_items)
        .enumerate()
    {
        let iy = layout.row_y(slot + extra);
        let ix = layout.item_x;
        let [iw, ih] = layout.item_half;
        let sel = selected == idx;
        let hov = point_in(mouse, ix, iy, iw, ih);
        ctx.rectangle(
            ix,
            iy,
            iw,
            ih,
            if hov {
                HOVER
            } else if sel {
                TRACK
            } else {
                Color {
                    a: 0.0,
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                }
            },
            z + 0.05,
        );
        ctx.text(
            ix,
            iy,
            font,
            if preserve_suffix {
                clip_port(label, max_chars)
            } else {
                clip(label, max_chars)
            },
            menu_size,
            if sel { 700.0 } else { 550.0 },
            if !def_raw.is_empty() && *raw == def_raw {
                GREEN
            } else if sel || hov {
                AMBER
            } else {
                FG
            },
            z + 0.06,
        );
        if clicked && hov {
            return DropdownResponse {
                picked: Some(idx),
                consumed: true,
            };
        }
    }
    if clicked && point_in(mouse, px, py, pw, ph) {
        return DropdownResponse {
            picked: None,
            consumed: true,
        };
    }
    DropdownResponse::default()
}

fn point_in(p: Option<[f32; 2]>, cx: f32, cy: f32, hw: f32, hh: f32) -> bool {
    p.is_some_and(|[x, y]| (x - cx).abs() <= hw && (y - cy).abs() <= hh)
}

fn clip_port(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        return s.to_owned();
    }
    // Keep the channel/port suffix visible, where JACK names usually differ.
    let head = n / 3;
    let tail = n.saturating_sub(head + 1);
    format!(
        "{}…{}",
        chars[..head].iter().collect::<String>(),
        chars[chars.len() - tail..].iter().collect::<String>()
    )
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

const AMBER: Color = rgb(0xe8, 0xa3, 0x3d);
const GREEN: Color = rgb(0x4a, 0xde, 0x80);
const DIM: Color = rgb(0x6b, 0x72, 0x80);
const TRACK: Color = rgb(0x2a, 0x24, 0x3d);
/// Dropdown well: distinctly darker than the page so the open list reads as
/// a cut-in surface; a near-page tint was invisible against the background.
const MENU_BG: Color = rgb(0x0d, 0x0a, 0x16);
const HOVER: Color = rgb(0x3a, 0x33, 0x4f);
const FG: Color = rgb(0xe6, 0xe6, 0xea);

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}
