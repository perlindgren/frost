//! The generalized [`frost::Tween`]: one scalar tween drives a line's
//! rightward ping-pong between `x = 200` and `x = 300` (1.0s per leg, i.e. a
//! constant 100px/s), and one vector tween drives a small circle back and
//! forth between `(-100, -100)` and `(100, -100)` (1.5s per leg).
//!
//! The scene holds only a background node, so the content comes from the
//! immediate draw methods. Run with:
//!
//! ```text
//! cargo run --example tween
//! ```

struct Demo {
    /// Drives the short line's rightward ping-pong.
    line_x: frost::Tween<f32>,
    /// Drives the small circle between its two positions.
    dot: frost::Tween<[f32; 2]>,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let (w, h) = ctx.size();
        // Diagonal from the top-left corner to the bottom-right corner.
        ctx.line(
            -w / 2.0,
            h / 2.0,
            w / 2.0,
            -h / 2.0,
            frost::Color {
                r: 0.6,
                g: 0.7,
                b: 0.9,
            },
            2.0,
            0.0,
        );
        // Circle at the window center, radius = height / 4.
        ctx.circle(
            0.0,
            0.0,
            h / 4.0,
            frost::Color {
                r: 0.9,
                g: 0.55,
                b: 0.25,
            },
            0.0,
        );

        // Short line, tweened between x = 200 and x = 300, on top.
        let x = self.line_x.tick(dt);
        ctx.line(
            x,
            200.0,
            x + 50.0,
            250.0,
            frost::Color {
                r: 0.4,
                g: 0.9,
                b: 0.5,
            },
            5.0,
            1.0,
        );

        // Small circle, tweened between (-100, -100) and (100, -100), on top.
        let [dx, dy] = self.dot.tick(dt);
        ctx.circle(
            dx,
            dy,
            20.0,
            frost::Color {
                r: 0.95,
                g: 0.35,
                b: 0.4,
            },
            1.0,
        );

        // Rectangle centered at (120, 120), 100 x 60 px, on top.
        ctx.rectangle(
            120.0,
            120.0,
            50.0,
            30.0,
            frost::Color {
                r: 0.95,
                g: 0.85,
                b: 0.3,
            },
            1.0,
        );
        log::trace!("process: dt {:?}", dt);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");
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
            children: vec![],
        }),
        Demo {
            line_x: frost::Tween::new(200.0, 300.0, 1.0),
            dot: frost::Tween::new([100.0, 100.0], [-100.0, -100.0], 1.5),
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
