//! The `tomato.png` sprite with its `tomato_fg.png` foreground overlay,
//! both as siblings under a shapeless dummy parent node in the scene tree.
//!
//! The two sprites share the same draw order (both `0.0`), so within their
//! group the tie resolves in tree order: the foreground sprite is the later
//! child and draws on top of the base. Each child carries the transform
//! that puts the anchor point `(308, 411)` of the 638×469 image (in its
//! own pixel space, `(0, 0)` upper-left, `y` down) onto the parent's
//! origin, and the parent carries the inverse transform, so the image's
//! center sits on the window's center and the whole image fits the window.
//! The two images are the same size, so the children's transforms are
//! identical and the overlay lines up pixel for pixel. Run with:
//!
//! ```text
//! cargo run --example tomato_sprite
//! ```

/// The anchor point in each image's pixel space: `(0, 0)` at the
/// upper-left, `x` right, `y` down. It is the point of the image that
/// sits on the parent node's origin.
const ANCHOR: (f32, f32) = (308.0, 411.0);



fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset paths to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let tomato = frost::Shape::sprite(format!("{root}/assets/sprites/tomato.png"))
        .expect("failed to load assets/sprites/tomato.png");
    let tomato_fg = frost::Shape::sprite(format!("{root}/assets/sprites/tomato_fg.png"))
        .expect("failed to load assets/sprites/tomato_fg.png");

    // The anchor's node-local offset. A child translates by its negation
    // to put the anchor on the parent's origin — the same convention
    // `tomato.rs` uses for its joints — and the parent translates by the
    // offset itself, so the image's center lands on the window's center
    // and the whole 638×469 image fits the window.
    let anchor = [ANCHOR.0, ANCHOR.1];
    let base = frost::Transform::anchor(
        anchor,
        tomato.sprite_size().expect("the child is a sprite"),
    );
    let foreground = frost::Transform::anchor(
        anchor,
        tomato_fg.sprite_size().expect("the child is a sprite"),
    );

    let scene = frost::Scene::new(frost::SceneNode {
        // The background hangs on the base group (the scene's root); its
        // transform is ignored anyway, so it always fills the window.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.08,
                g: 0.09,
                b: 0.12,
                a: 1.0,
            },
        }),
        children: vec![Box::new(frost::SceneNode {
            // The dummy parent: no shape of its own, it only holds the
            // two sprite siblings, shifted so the image's center sits on
            // the window's center.
            transform: frost::Transform::translate(base),
            children: vec![
                Box::new(frost::SceneNode {
                    // The negated anchor offset puts the anchor on the
                    // parent's origin.
                    transform: frost::Transform::translate([-base[0], -base[1]]),
                    // The same draw order as the foreground sibling: with
                    // equal order the tie resolves in tree order, so the
                    // base draws first...
                    order: 0.0,
                    shape: Some(tomato),
                    ..Default::default()
                }),
                Box::new(frost::SceneNode {
                    transform: frost::Transform::translate([-foreground[0], -foreground[1]]),
                    // ...and the later sibling, the overlay, draws on top.
                    order: 0.0,
                    shape: Some(tomato_fg),
                    ..Default::default()
                }),
            ],
            ..Default::default()
        })],
        ..Default::default()
    });

    if let Err(err) = frost::run(scene, |_ctx: &mut frost::Context, _dt: f32| {}) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
