//! The lit and the unlit: two rectangles in the same color — one with
//! `lit: true` — and a light stuck to the mouse.
//!
//! The "background" is a huge lit rectangle behind everything (the true
//! background is the window's clear color, which is never shaded), so
//! moving the mouse sweeps a circular pool of light across it and across
//! the lit rectangle, while the unlit one stays exactly as it was born:
//! the flag is the only difference between the two.
//!
//! While the cursor is outside the window, the light stays where it was
//! last seen; it starts at the window's center, so there is a pool of
//! light from the first frame.
//!
//! Run with:
//!
//! ```text
//! cargo run --example lit_unlit
//! ```

/// The color of the huge background rectangle.
const FLOOR: frost::Color = frost::Color {
    r: 0.15,
    g: 0.15,
    b: 0.25,
    a: 1.0,
};

/// The shared rectangle color: identical on both, so the `lit` flag is the
/// only difference between them.
const RECT: frost::Color = frost::Color {
    r: 0.55,
    g: 0.52,
    b: 0.48,
    a: 1.0,
};

/// The light's color: a warm white.
const LIGHT: frost::Color = frost::Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

/// The light's strength.
const LIGHT_INTENSITY: f32 = 2.5;

/// The light's falloff extent, in pixels: the radius of the circular pool
/// around the cursor.
const LIGHT_RADIUS: f32 = 160.0;

struct Demo {
    /// The light's position, in the window's user space: the cursor's
    /// position while it is inside the window, otherwise the last known
    /// one.
    pos: [f32; 2],
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, _dt: f32) {
        // The light is attached to the mouse: one immediate light call per
        // frame, at the cursor while it is inside the window.
        if let Some(pos) = ctx.mouse_position() {
            self.pos = pos;
        }
        ctx.light(
            self.pos[0],
            self.pos[1],
            LIGHT,
            LIGHT_INTENSITY,
            LIGHT_RADIUS,
        );
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    if let Err(err) = frost::run(
        frost::Scene::new(frost::SceneNode {
            children: vec![
                Box::new(frost::SceneNode {
                    // The "background": a huge lit rectangle far beyond the
                    // window's edges, behind everything (the order puts it
                    // under the other draws). It reads as a solid color
                    // until the light's pool reaches it.
                    order: -1.0,
                    shape: Some(frost::Shape::Rectangle {
                        center: [0.0, 0.0],
                        extent: [2000.0, 2000.0],
                        color: FLOOR,
                    }),
                    lit: true,
                    ..Default::default()
                }),
                Box::new(frost::SceneNode {
                    // The lit rectangle, on the left: the light's pool
                    // sweeps across it as the mouse moves.
                    shape: Some(frost::Shape::Rectangle {
                        center: [-160.0, 0.0],
                        extent: [110.0, 70.0],
                        color: RECT,
                    }),
                    lit: true,
                    ..Default::default()
                }),
                Box::new(frost::SceneNode {
                    // The unlit rectangle, on the right: the same color and
                    // size as its twin — only the missing `lit` flag
                    // differs, and the light never touches it.
                    shape: Some(frost::Shape::Rectangle {
                        center: [160.0, 0.0],
                        extent: [110.0, 70.0],
                        color: RECT,
                    }),
                    ..Default::default()
                }),
            ],
            ..Default::default()
        }),
        Demo { pos: [0.0, 0.0] },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
