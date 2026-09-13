//! The smallest possible scene: a background node and a single static
//! rectangle centered in the window.
//!
//! The scene is built once and passed to `frost::run`, which draws it every
//! frame; the process does nothing. The user-space origin is the window
//! center (y up), so a rectangle at `center: [0.0, 0.0]` sits in the middle
//! of the screen. The root node holds the background — any node may, but
//! the root is the natural home. Run with:
//!
//! ```text
//! cargo run --example one_rect
//! ```

fn main() {
    env_logger::init();
    log::info!("frost started");

    let scene = frost::Scene::new(frost::SceneNode {
        transform: frost::Transform::identity(),
        // Deep indigo background; the node's transform is ignored.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.09,
                g: 0.06,
                b: 0.16,
            },
        }),
        children: vec![Box::new(frost::SceneNode {
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
        })],
    });

    if let Err(err) = frost::run(scene, |_ctx: &mut frost::Context, _dt: f32| {}) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
