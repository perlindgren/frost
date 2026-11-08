//! Pointer-relative tooltip shared by main-view controls.
use frost::{Color, Context};

/// Draw above and to the right of the pointer, clamped inside the window.
pub fn draw(ctx: &mut Context, font: &std::sync::Arc<[u8]>, text: &str, fit: f32) {
    let Some([mx, my]) = ctx.mouse_position() else {
        return;
    };
    let (w, h) = ctx.size();
    let size = 15.0 * fit;
    let half_w = ctx.text_width(font, text, size, 550.0) / 2.0 + 9.0 * fit;
    let half_h = 12.0 * fit;
    let margin = 6.0 * fit;
    // Up and to the right of the pointer, then pushed back inside the
    // window: the gauge and the meter run to both edges, so a tooltip that
    // only knew its preferred offset would leave the screen.
    let cx = (mx + half_w + 16.0 * fit)
        .min(w / 2.0 - margin - half_w)
        .max(-w / 2.0 + margin + half_w);
    let cy = (my + half_h + 22.0 * fit)
        .min(h / 2.0 - margin - half_h)
        .max(-h / 2.0 + margin + half_h);
    ctx.rectangle(cx, cy, half_w, half_h, MENU_BG, 0.6);
    ctx.text(cx, cy, font, text, size, 550.0, FG, 0.61);
}
const MENU_BG: Color = Color {
    r: 13.0 / 255.0,
    g: 10.0 / 255.0,
    b: 22.0 / 255.0,
    a: 1.0,
};
const FG: Color = Color {
    r: 230.0 / 255.0,
    g: 230.0 / 255.0,
    b: 234.0 / 255.0,
    a: 1.0,
};
