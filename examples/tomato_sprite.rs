//! The `tomato.png` sprite with its `tomato_fg.png` foreground overlay,
//! both as siblings under a shapeless dummy parent node in the scene tree.
//!
//! The two sprites share the same draw order (both `0.0`), so within their
//! group the tie resolves in tree order: the foreground sprite is the later
//! child and draws on top of the base. Both carry the same transform — the
//! one that puts the anchor point `(310, 1124)` of the 638×1140 image (in
//! its own pixel space, `(0, 0)` upper-left, `y` down) onto the parent's
//! origin — and the two images are the same size, so the overlay lines up
//! pixel for pixel. The parent sits on the scene's origin, the window's
//! center. Run with:
//!
//! ```text
//! cargo run --example tomato_sprite
//! ```

/// The anchor point in each image's pixel space: `(0, 0)` at the
/// upper-left, `x` right, `y` down. It is the point of the image that
/// sits on the parent node's origin.
const ANCHOR: (f32, f32) = (310.0, 1124.0);

/// A sprite's texture size in pixels.
fn sprite_size(shape: &frost::Shape) -> [f32; 2] {
    match shape {
        frost::Shape::Sprite { width, height, .. } => [*width as f32, *height as f32],
        _ => unreachable!("the child is a sprite"),
    }
}

/// The anchor's node-local coordinates: a sprite is centered on its node's
/// origin and the scene's y axis points up, so an anchor at image pixels
/// `(ax, ay)` of a `w`x`h` image is the offset `(ax - w/2, h/2 - ay)` —
/// the y flip included.
fn local_anchor(size: [f32; 2]) -> [f32; 2] {
    [ANCHOR.0 - size[0] / 2.0, size[1] / 2.0 - ANCHOR.1]
}

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

    // Each sprite anchors the same image point onto its node's origin.
    // Both images are the same size, so the two transforms are identical
    // and the overlay lines up pixel for pixel.
    let base = local_anchor(sprite_size(&tomato));
    let foreground = local_anchor(sprite_size(&tomato_fg));

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
            // two sprite siblings on the scene's origin, the window's
            // center.
            children: vec![
                Box::new(frost::SceneNode {
                    transform: frost::Transform::translate(base[0], base[1]),
                    // The same draw order as the foreground sibling: with
                    // equal order the tie resolves in tree order, so the
                    // base draws first...
                    order: 0.0,
                    shape: Some(tomato),
                    ..Default::default()
                }),
                Box::new(frost::SceneNode {
                    transform: frost::Transform::translate(foreground[0], foreground[1]),
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
