//! Sprite shapes: PNG textures drawn through the scene tree. Two sprites
//! from `assets/sprites` (`brick.png` and `Button.png`) are loaded with
//! `Shape::sprite`; each one is centered on its node's origin, one texture
//! pixel per scene pixel, and the node's transform and scale apply to it
//! exactly as they do to the other shapes. The button carries a small brick
//! as a child node, and its tint drifts over time to show per-pixel
//! tinting; the brick fades out and back in on a 1s cycle to show the
//! sprite's alpha. Run with:
//!
//! ```text
//! cargo run --example sprites
//! ```

struct Demo {
    /// Elapsed time in seconds.
    t: f32,
    /// The brick's opacity: 0.0 -> 1.0 in half a second, then back — a 1s
    /// full cycle.
    brick_alpha: frost::Tween<f32>,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        self.t += dt;

        // The brick bobs up and down, slowly tumbles, and fades out and
        // back in over a 1s cycle.
        let brick = &mut ctx.scene().root.children[0];
        brick.transform = frost::Transform::rotate(0.6 * self.t)
            .compose(&frost::Transform::translate(
                -160.0,
                (self.t * 2.0).sin() * 20.0,
            ));
        if let Some(frost::Shape::Sprite { alpha, .. }) = &mut brick.shape {
            *alpha = self.brick_alpha.tick(dt);
        }

        // The button sways side to side while it breathes in scale around
        // its center.
        let button = &mut ctx.scene().root.children[1];
        button.transform = frost::Transform::translate(
            160.0,
            (self.t * 0.5).sin() * 30.0,
        );
        let breath = 1.0 + 0.1 * (self.t * 2.0).sin();
        button.scale = [breath, breath];
        // The per-pixel tint warms and cools, showing the tint channel
        // multiplied over every sampled pixel.
        if let Some(frost::Shape::Sprite { color, .. }) = &mut button.shape {
            *color = frost::Color {
                r: 1.0,
                g: 0.75 + 0.25 * (self.t * 0.7).sin(),
                b: 0.6 + 0.4 * (self.t * 0.7).cos(),
            };
        }

        // The little brick rides the button: it counter-rotates in the
        // button's space, so it wobbles as the button sways and breathes.
        let rider = &mut button.children[0];
        rider.transform = frost::Transform::rotate(-1.5 * self.t)
            .compose(&frost::Transform::translate(0.0, 120.0));
        log::trace!("process: dt {:?}", dt);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset paths to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let brick = frost::Shape::sprite(format!("{root}/assets/sprites/brick.png"))
        .expect("failed to load assets/sprites/brick.png");
    let button = frost::Shape::sprite(format!("{root}/assets/sprites/Button.png"))
        .expect("failed to load assets/sprites/Button.png");
    // The rider loads the same file a second time and sits on the button as
    // a child node.
    let rider = frost::Shape::sprite(format!("{root}/assets/sprites/brick.png"))
        .expect("failed to load assets/sprites/brick.png");

    let scene = frost::Scene::new(frost::SceneNode {
        transform: frost::Transform::identity(),
        scale: [1.0, 1.0],
        order: 0.0,
        // Deep indigo background; the node's transform is ignored.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.09,
                g: 0.06,
                b: 0.16,
            },
        }),
        children: vec![
            Box::new(frost::SceneNode {
                transform: frost::Transform::translate(-160.0, 0.0),
                scale: [1.0, 1.0],
                order: 0.0,
                shape: Some(brick),
                children: vec![],
            }),
            Box::new(frost::SceneNode {
                transform: frost::Transform::translate(160.0, 0.0),
                scale: [1.0, 1.0],
                order: 0.0,
                shape: Some(button),
                children: vec![Box::new(frost::SceneNode {
                    transform: frost::Transform::translate(0.0, 120.0),
                    scale: [0.5, 0.5],
                    order: 0.0,
                    shape: Some(rider),
                    children: vec![],
                })],
            }),
        ],
    });

    if let Err(err) = frost::run(
        scene,
        Demo {
            t: 0.0,
            brick_alpha: frost::Tween::new(0.0, 1.0, 0.5),
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
