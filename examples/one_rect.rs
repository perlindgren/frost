//! The smallest possible scene: a single static rectangle centered in the
//! window.
//!
//! The scene is built once and passed to `frost::run`, which draws it every
//! frame; the process only sets the background. The user-space origin is the
//! window center (y up), so a rectangle at `center: [0.0, 0.0]` sits in the
//! middle of the screen. Run with:
//!
//! ```text
//! cargo run --example one_rect
//! ```

fn main() {
    env_logger::init();
    log::info!("frost started");

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

    if let Err(err) = frost::run(scene, |ctx: &mut frost::Context, _dt: f32| {
        // Deep indigo background, set per frame.
        ctx.set_background(frost::Color {
            r: 0.09,
            g: 0.06,
            b: 0.16,
        });
    }) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
