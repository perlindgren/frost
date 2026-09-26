//! Text shapes: TTF glyphs drawn through the scene tree. "immortal tomato
//! 0123456789" is set in `assets/fonts/Leofont-Regular.ttf` at 48 pixels on
//! top of a circle at the window center — the same swaying scene as the
//! `text` example, with a different font. The text node is a child of the
//! circle node and carries the identity transform, so the text sits at the
//! screen center. The circle, with the text riding on it, sways gently up
//! and down to show the text moving like any other shape.
//! Run with:
//!
//! ```text
//! cargo run --example text_leo
//! ```

struct Demo {
    /// Elapsed time in seconds.
    t: f32,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        self.t += dt;

        // The whole line drifts up and down around the screen center, like
        // any other node in the tree.
        let text = &mut ctx.scene().root.children[0];
        text.transform = frost::Transform::translate([0.0, (self.t * 0.5).sin() * 20.0]);
        log::trace!("process: dt {:?}", dt);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset paths to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let hello = match frost::Shape::text(
        format!("{root}/assets/fonts/Leofont-Regular.ttf"),
        "immortal tomato 0123456789",
        48.0,
    ) {
        Ok(shape) => shape,
        Err(err) => {
            log::error!("failed to load the font: {err}");
            std::process::exit(1);
        }
    };

    let scene = frost::Scene::new(frost::SceneNode {
        // Deep indigo background; the node's transform is ignored.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.09,
                g: 0.06,
                b: 0.16,
                a: 1.0,
            },
        }),
        children: vec![Box::new(frost::SceneNode {
            // The identity transform keeps the text centered on the screen;
            // process() nudges it vertically each frame.
            shape: Some(frost::Shape::Circle {
                center: [0.0, 0.0],
                radius: 90.0,
                color: frost::Color {
                    r: 0.25,
                    g: 0.35,
                    b: 0.6,
                    a: 1.0,
                },
            }),

            children: vec![Box::new(frost::SceneNode {
                shape: Some(hello),
                ..Default::default()
            })],
            ..Default::default()
        })],
        ..Default::default()
    });

    if let Err(err) = frost::run(scene, Demo { t: 0.0 }) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
