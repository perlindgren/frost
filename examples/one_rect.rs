//! The smallest possible scene: a single static rectangle centered in the
//! window.
//!
//! The user-space origin is the window center (y up), so a rectangle at
//! `center: [0.0, 0.0]` sits in the middle of the screen. Run with:
//!
//! ```text
//! cargo run --example one_rect
//! ```

struct Demo;

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Canvas, _dt: f32) {
        // Deep indigo background, set per frame.
        ctx.set_background(frost::Color {
            r: 0.09,
            g: 0.06,
            b: 0.16,
        });

        let scene = frost::Scene::new(frost::SceneNode {
            transform: frost::Transform::identity(),
            shape: Some(frost::Shape::Rectangle {
                center: [0.0, 0.0],
                extent: [80.0, 40.0],
                color: frost::Color {
                    r: 0.9,
                    g: 0.45,
                    b: 0.2,
                },
            }),
            children: vec![],
        });
        ctx.draw_scene(&scene);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");
    if let Err(err) = frost::run(Demo) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
