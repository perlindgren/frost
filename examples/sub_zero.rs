//! Text shapes: TTF glyphs drawn through the scene tree. "hello world" is
//! set in `assets/fonts/JameGem08_2026-Regular.ttf` at 48 pixels and
//! centered on the screen (a node at the user-space origin is the screen
//! center, so the text node carries the identity transform). The node
//! sways gently up and down to show the text moving like any other shape.
//! Run with:
//!
//! ```text
//! cargo run --example text
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
        text.transform =
            frost::Transform::translate(0.0, (self.t % std::f32::consts::TAU).sin() * 40.0);
        log::trace!("process: dt {:?}", dt);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset paths to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let ttf_path = format!("{root}/assets/fonts/JameGem08_2026-Regular.ttf");

    let s = match frost::Shape::text(&ttf_path, "S", 48.0) {
        Ok(shape) => shape,
        Err(err) => {
            log::error!("failed to load the font: {err}");
            std::process::exit(1);
        }
    };

    let u = match frost::Shape::text(&ttf_path, "U", 48.0) {
        Ok(shape) => shape,
        Err(err) => {
            log::error!("failed to load the font: {err}");
            std::process::exit(1);
        }
    };

    let b = match frost::Shape::text(&ttf_path, "B", 48.0) {
        Ok(shape) => shape,
        Err(err) => {
            log::error!("failed to load the font: {err}");
            std::process::exit(1);
        }
    };

    let zero = match frost::Shape::text(&ttf_path, "0", 48.0 * 6.0) {
        Ok(shape) => shape,
        Err(err) => {
            log::error!("failed to load the font: {err}");
            std::process::exit(1);
        }
    };

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
        children: vec![Box::new(frost::SceneNode {
            // The identity transform keeps the text centered on the screen;
            // process() nudges it vertically each frame.
            transform: frost::Transform::identity(),
            scale: [1.0, 1.0],
            order: 0.0,
            shape: Some(frost::Shape::Circle {
                center: [0.0, 0.0],
                radius: 90.0,
                color: frost::Color {
                    r: 0.25,
                    g: 0.35,
                    b: 0.6,
                },
            }),

            children: vec![
                Box::new(frost::SceneNode {
                    transform: frost::Transform::translate(0.0, 50.0),
                    scale: [1.0, 1.0],
                    order: 0.0,
                    shape: Some(s),
                    children: vec![],
                }),
                Box::new(frost::SceneNode {
                    transform: frost::Transform::translate(0.0, 0.0),
                    scale: [1.0, 1.0],
                    order: 0.0,
                    shape: Some(u),
                    children: vec![],
                }),
                Box::new(frost::SceneNode {
                    transform: frost::Transform::translate(0.0, -50.0),
                    scale: [1.0, 1.0],
                    order: 0.0,
                    shape: Some(b),
                    children: vec![],
                }),
                Box::new(frost::SceneNode {
                    transform: frost::Transform::translate(-10.0, -90.0),
                    scale: [1.0, 1.0],
                    order: 0.0,
                    shape: Some(zero),
                    children: vec![],
                }),
            ],
        })],
    });

    if let Err(err) = frost::run(scene, Demo { t: 0.0 }) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
