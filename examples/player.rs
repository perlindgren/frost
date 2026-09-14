//! The player drives the Button sprite with the `W`/`A`/`S`/`D` keys: `W`
//! moves it up, `S` down, `A` left, `D` right — 100 pixels per second —
//! starting from the window center `(0, 0)`. Run with:
//!
//! ```text
//! cargo run --example player
//! ```

/// The button's speed, in pixels per second.
const SPEED: f32 = 100.0;

/// The direction along one axis from two held keys: `+1` if only `positive`
/// is down, `-1` if only `negative` is, `0` if both or neither is.
fn axis(ctx: &frost::Context, positive: frost::KeyCode, negative: frost::KeyCode) -> f32 {
    (ctx.key_down(positive) as i32 - ctx.key_down(negative) as i32) as f32
}

struct Demo {
    /// The button's position in window-centered pixels.
    pos: [f32; 2],
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let dx = axis(ctx, frost::KeyCode::KeyD, frost::KeyCode::KeyA);
        let dy = axis(ctx, frost::KeyCode::KeyW, frost::KeyCode::KeyS);
        self.pos[0] += dx * SPEED * dt;
        self.pos[1] += dy * SPEED * dt;

        let button = &mut ctx.scene().root.children[0];
        button.transform = frost::Transform::translate(self.pos[0], self.pos[1]);
        log::trace!("process: dt {:?} pos {:?}", dt, self.pos);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset path to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let button = frost::Shape::sprite(format!("{root}/assets/sprites/Button.png"))
        .expect("failed to load assets/sprites/Button.png");

    if let Err(err) = frost::run(
        frost::Scene::new(frost::SceneNode {
            transform: frost::Transform::identity(),
            scale: [1.0, 1.0],
            // Deep indigo background; the node's transform is ignored.
            shape: Some(frost::Shape::Background {
                color: frost::Color {
                    r: 0.09,
                    g: 0.06,
                    b: 0.16,
                },
            }),
            children: vec![Box::new(frost::SceneNode {
                // The button starts at the window center.
                transform: frost::Transform::translate(0.0, 0.0),
                scale: [1.0, 1.0],
                shape: Some(button),
                children: vec![],
            })],
        }),
        Demo { pos: [0.0, 0.0] },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
