//! A watering can as a mouse cursor over a full-screen grass field.
//! `assets/sprites/grass.png` is stretched every frame to fill the window,
//! and `assets/sprites/water_can.png` follows the pointer: the can is the
//! only visible content in an otherwise transparent 1920x1080 canvas, so
//! the demo scales it to `CAN_SIZE` pixels wide and offsets its node so
//! the can's center — not the texture's center — sits on the cursor. The
//! cursor position comes from [`frost::Context::mouse_position`]. Run
//! with:
//!
//! ```text
//! cargo run --example cursor
//! ```

/// `grass.png`'s texture size in pixels: a full-bleed 1920x1080 photo.
const GRASS_SIZE: [f32; 2] = [1920.0, 1080.0];

/// `water_can.png`'s texture size in pixels.
const CAN_IMAGE: [f32; 2] = [1920.0, 1080.0];

/// The can's visible content in the image's own pixel space: `(0, 0)` is
/// the upper-left corner, `x` grows to the right, `y` grows down. The can
/// sits in the lower-left of the otherwise transparent canvas, so it must
/// be centered on the pointer by offset, not just scaled.
const CAN_BOX: [[f32; 2]; 2] = [
    [75.0, 709.0], // content upper-left
    [408.0, 959.0], // content lower-right
];

/// The rendered can's width in pixels; its height follows the content's
/// 334:251 aspect ratio.
const CAN_SIZE: f32 = 100.0;

/// Scales the whole texture so the can's content is `CAN_SIZE` wide.
const CAN_SCALE: f32 = CAN_SIZE / (CAN_BOX[1][0] - CAN_BOX[0][0]);

/// The can's content center in node-local space: the sprite is centered on
/// its node's origin and the scene's y axis points up, so the content
/// center's image pixels `(cx, cy)` convert to `(cx - w/2, h/2 - cy)` —
/// the y flip included.
const CAN_LOCAL: [f32; 2] = {
    let cx = (CAN_BOX[0][0] + CAN_BOX[1][0]) / 2.0;
    let cy = (CAN_BOX[0][1] + CAN_BOX[1][1]) / 2.0;
    [cx - CAN_IMAGE[0] / 2.0, CAN_IMAGE[1] / 2.0 - cy]
};

/// The node position that puts the can's content center exactly on
/// `(mx, my)`: the center sits `CAN_LOCAL` from the node's origin in
/// node-local space, so the origin takes the negative of that offset,
/// scaled.
fn can_position(mx: f32, my: f32) -> [f32; 2] {
    [mx - CAN_LOCAL[0] * CAN_SCALE, my - CAN_LOCAL[1] * CAN_SCALE]
}

struct Demo {
    /// The cursor's last reported position; the can sticks here while the
    /// cursor is outside the window.
    mouse: [f32; 2],
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, _dt: f32) {
        // Stretch the grass to exactly fill the window, whatever its aspect
        // ratio, so it stays filled across resizes.
        let (w, h) = ctx.size();
        let grass = &mut ctx.scene().root.children[0];
        grass.scale = [w / GRASS_SIZE[0], h / GRASS_SIZE[1]];

        // Follow the pointer, keeping the last known position while the
        // cursor is outside the window.
        if let Some(pos) = ctx.mouse_position() {
            self.mouse = pos;
        }
        let [mx, my] = self.mouse;
        let [px, py] = can_position(mx, my);
        let can = &mut ctx.scene().root.children[1];
        can.transform = frost::Transform::translate(px, py);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset paths to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let grass = frost::Shape::sprite(format!("{root}/assets/sprites/grass.png"))
        .expect("failed to load assets/sprites/grass.png");
    let can = frost::Shape::sprite(format!("{root}/assets/sprites/water_can.png"))
        .expect("failed to load assets/sprites/water_can.png");

    let scene = frost::Scene::new(frost::SceneNode {
        // Dark ground under the grass; only the sparse transparent gaps in
        // the grass texture show it.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.04,
                g: 0.1,
                b: 0.04,
                a: 1.0,
            },
        }),
        children: vec![
            Box::new(frost::SceneNode {
                // Stretched to fill the window by the process, every frame.
                shape: Some(grass),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // Starts at the window's center; the process moves it to
                // the pointer from the first frame on.
                scale: [CAN_SCALE, CAN_SCALE],
                shape: Some(can),
                ..Default::default()
            }),
        ],
        ..Default::default()
    });

    if let Err(err) = frost::run(scene, Demo { mouse: [0.0, 0.0] }) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
