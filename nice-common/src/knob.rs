//! The family's one knob, drawn the way nice-wah drew the first: a face,
//! a bore, the end ticks, and a pointer. There is no arc in frost's
//! immediate canvas, and a sweep of short chords from the start tick to
//! the pointer says "this far along" well enough to be worth its lines.
//!
//! The drawing takes a normalized `t` because what a knob means — which
//! parameter, what range, what units — belongs to the plugin holding it;
//! what belongs here is what a knob *looks like*, and the plugins of this
//! family have never disagreed about that.
use frost::{Color, Context};
use std::sync::Arc;

/// The knob drag that carries a parameter from one end of its range to
/// the other, in logical pixels; holding Shift stretches it for fine
/// moves.
pub const TRAVEL: f32 = 200.0;
pub const TRAVEL_FINE: f32 = 800.0;

/// The heaviest cut the embedded font has: `FiraCode-VariableFont_wght.ttf`
/// carries a single `wght` axis spanning 300..700, so 700 is as black as
/// it gets.
const TEXT_WEIGHT: f32 = 700.0;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

/// The face of every knob, button and rail in the family.
pub const PANEL: Color = rgb(65, 57, 82);
/// The bore of a knob, a shade under its face so the pointer has
/// something to sit on.
pub const BORE: Color = rgb(46, 40, 60);
/// The house green: a value at rest.
pub const GREEN: Color = rgb(103, 237, 175);
/// The house amber: a grabbed control, a held peak, an active choice.
pub const AMBER: Color = rgb(250, 190, 115);
pub const TEXT: Color = rgb(235, 230, 241);
pub const MUTED: Color = rgb(150, 142, 166);

/// One knob, one call. `t` is the value normalized to `0.0..=1.0` — the
/// caller has the parameter and its range, and formats `readout` in its
/// own units — and `touched` is hover or drag, which is the only reason
/// a knob ever speaks amber. The caption rides above the pointer, the
/// readout above the caption, both centred on the knob, both scaled by
/// the window's `fit` like the rest of the chrome.
#[allow(clippy::too_many_arguments)] // immediate-mode draw call, as in frost's own canvas
pub fn draw(
    ctx: &mut Context,
    font: &Arc<[u8]>,
    at: [f32; 2],
    radius: f32,
    t: f32,
    touched: bool,
    label: &str,
    readout: String,
    fit: f32,
) {
    let (kx, ky) = (at[0], at[1]);
    let t = t.clamp(0.0, 1.0);
    ctx.circle(kx, ky, radius, PANEL, 0.5);
    ctx.circle(kx, ky, radius * 0.66, BORE, 0.52);
    let angle = |t: f32| (t * 270.0 - 135.0).to_radians();
    // The value sweep, in ten-degree steps so it reads as a ring.
    let steps = (t * 27.0) as i32;
    for s in 0..=steps {
        let a = angle(s as f32 / 27.0);
        let (sx, sy) = (a.sin(), a.cos());
        ctx.line(
            kx + sx * radius * 1.06,
            ky + sy * radius * 1.06,
            kx + sx * radius * 1.22,
            ky + sy * radius * 1.22,
            if touched { AMBER } else { GREEN },
            2.0 * fit,
            0.45,
        );
    }
    // The far tick, muted: the ring only reaches it when the knob
    // bottoms out, and a player should see how much rail is left.
    if t < 1.0 {
        let a = angle(1.0);
        let (sx, sy) = (a.sin(), a.cos());
        ctx.line(
            kx + sx * radius * 1.06,
            ky + sy * radius * 1.22,
            kx + sx * radius * 1.3,
            ky + sy * radius * 1.3,
            MUTED,
            2.0 * fit,
            0.44,
        );
    }
    let a = angle(t);
    ctx.line(
        kx,
        ky,
        kx + a.sin() * radius * 0.9,
        ky + a.cos() * radius * 0.9,
        if touched { AMBER } else { TEXT },
        3.0 * fit,
        0.55,
    );
    ctx.text(
        kx,
        ky - radius - 14.0 * fit,
        font,
        label,
        13.0 * fit,
        TEXT_WEIGHT,
        MUTED,
        0.5,
    );
    ctx.text(
        kx,
        ky - radius - 30.0 * fit,
        font,
        readout,
        14.0 * fit,
        TEXT_WEIGHT,
        TEXT,
        0.5,
    );
}

/// Whether a pointer sits on a knob drawn at `at` with `radius` — the
/// four-pixel margin the family's hit tests have always given a round
/// target, so a grab need not be exact.
pub fn contains(at: [f32; 2], radius: f32, mouse: [f32; 2]) -> bool {
    (mouse[0] - at[0]).abs() <= radius + 4.0 && (mouse[1] - at[1]).abs() <= radius + 4.0
}

#[cfg(test)]
mod tests {
    use super::{TRAVEL, TRAVEL_FINE, contains};

    #[test]
    fn a_grab_is_forgiving_and_a_fine_drag_is_fine() {
        const { assert!(TRAVEL_FINE > TRAVEL) };
        assert!(contains([0.0, 0.0], 24.0, [27.0, 0.0]));
        assert!(!contains([0.0, 0.0], 24.0, [29.0, 0.0]));
        assert!(!contains([0.0, 0.0], 24.0, [0.0, 40.0]));
    }
}
