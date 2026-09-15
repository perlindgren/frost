//! Text shapes: TTF glyphs drawn through the scene tree. "SUB0" is set in
//! `assets/fonts/JameGem08_2026-Regular.ttf`: the letters S, U, and B at
//! 48 pixels stacked around the screen center, and a large 0 at the lower
//! left, all riding on a circle that sways gently up and down like any
//! other node in the tree. The three letters are treated as channels:
//! each one's alpha is modulated 120 degrees out of phase with the
//! others, sweeping from 0 to 1 and back over one second, infinitely
//! repeated. Run with:
//!
//! ```text
//! cargo run --example sub_zero
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
        let line = &mut ctx.scene().root.children[0];
        line.transform =
            frost::Transform::translate(0.0, (self.t % std::f32::consts::TAU).sin() * 40.0);

        // The S, U, and B letters — the first three children; the zero
        // keeps its default white modulate — are three channels, 120
        // degrees out of phase with each other. Each channel's alpha
        // sweeps 0 -> 1 -> 0 over one second and repeats forever: a
        // raised cosine, so the sweep is smooth. The sweep rides on each
        // node's `modulate`, white except for the animated alpha, which
        // multiplies into the white glyph color channel by channel.
        let t = self.t;
        for (child, k) in line.children.iter_mut().zip([0, 1, 2]) {
            let phase = k as f32 * std::f32::consts::TAU / 3.0;
            let alpha = 0.5 * (1.0 - (std::f32::consts::TAU * t + phase).cos());
            child.modulate = frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: alpha,
            };
        }
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

            children: vec![
                Box::new(frost::SceneNode {
                    transform: frost::Transform::translate(0.0, 50.0),
                    shape: Some(s),
                    ..Default::default()
                }),
                Box::new(frost::SceneNode {
                    shape: Some(u),
                    ..Default::default()
                }),
                Box::new(frost::SceneNode {
                    transform: frost::Transform::translate(0.0, -50.0),
                    shape: Some(b),
                    ..Default::default()
                }),
                Box::new(frost::SceneNode {
                    transform: frost::Transform::translate(-10.0, -90.0),
                    shape: Some(zero),
                    ..Default::default()
                }),
            ],
            ..Default::default()
        })],
        ..Default::default()
    });

    if let Err(err) = frost::run(scene, Demo { t: 0.0 }) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
