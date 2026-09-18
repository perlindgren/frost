//! A tomato plant assembled from three sprite slices in `assets/sprites`
//! (`plant1.png` the base, `plant2.png` the middle, `plant3.png` the top),
//! chained through hand-picked joint positions: each slice's lower joint is
//! placed exactly on the previous slice's upper joint, so the three pieces
//! read as one plant.
//!
//! The joints are given in each image's own pixel space: `(0, 0)` is the
//! image's upper-left corner, `x` grows to the right and `y` grows down. A
//! sprite is centered on its node's origin, and the scene's y axis points
//! up, so a joint at image pixels `(jx, jy)` of an `w`x`h` image is
//! converted to the node-local offset `(jx - w/2, h/2 - jy)` — the y flip
//! included.
//!
//! The assembled plant is 969 px tall, so it is scaled down to fit the
//! 600 px window, and it sways in a gentle travelling wind: the base rock
//! turns the whole plant around its lower joint, and the two joints above
//! it bend a little more each, so the tip moves the most. Run with:
//!
//! ```text
//! cargo run --example tomato
//! ```

/// The hand-picked joints of the three slices, in each image's pixel space:
/// `(0, 0)` at the upper-left, `x` right, `y` down. `[0]` is where the
/// slice attaches to the previous one, `[1]` where the next slice attaches.
const JOINTS: [[(f32, f32); 2]; 3] = [
    [(321.0, 671.0), (321.0, 578.0)], // plant1, the base
    [(319.0, 581.0), (318.0, 351.0)], // plant2, the middle
    [(318.0, 463.0), (309.0, 89.0)],  // plant3, the top
];

/// Shrinks the whole plant so its 969 px span fits the 600 px window.
const SCALE: f32 = 0.55;

/// The world position of the plant's root joint (plant1's lower joint).
/// The chain's midpoint sits 301.5 px above that joint in the current art
/// (the plant's bottom edge is 183 px below it, its top edge 786 px above),
/// so anchoring the root at `-301.5 * SCALE` puts the scaled plant in the
/// window's center.
const ANCHOR: [f32; 2] = [0.0, -301.5 * SCALE];

/// One slice's two joints, already converted to node-local coordinates.
struct Link {
    /// The local position of the joint that attaches to the previous slice.
    from: [f32; 2],
    /// The local position of the joint that the next slice attaches to.
    to: [f32; 2],
}

/// Converts a joint from the image's pixel space — `(0, 0)` at the upper-
/// left, `y` down — to node-local coordinates: the sprite is centered on
/// the node's origin and the scene's y axis points up.
fn local_joint(jx: f32, jy: f32, size: [f32; 2]) -> [f32; 2] {
    [jx - size[0] / 2.0, size[1] / 2.0 - jy]
}

/// The loaded sprite's texture size in pixels.
fn sprite_size(shape: &frost::Shape) -> [f32; 2] {
    match shape {
        frost::Shape::Sprite { width, height, .. } => [*width as f32, *height as f32],
        _ => unreachable!("the slice is a sprite"),
    }
}

/// A slice's joints in node-local space, from the hand-picked pixel
/// coordinates and the texture's real size.
fn link(shape: &frost::Shape, joints: [(f32, f32); 2]) -> Link {
    let size = sprite_size(shape);
    Link {
        from: local_joint(joints[0].0, joints[0].1, size),
        to: local_joint(joints[1].0, joints[1].1, size),
    }
}

struct Demo {
    /// Elapsed time in seconds.
    t: f32,
    /// The three slices in chain order, with their joints in node-local
    /// space. The shapes themselves live in the scene; the layout only
    /// needs the joints.
    links: [Link; 3],
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        self.t += dt;

        // A gentle travelling wind: the base rock and the two joint bends
        // lag each other, and each bend is a little stronger than the last,
        // so the tip of the plant moves the most.
        let base = (self.t * 1.2).sin() * 0.025;
        let bend1 = (self.t * 1.2 - 0.8).sin() * 0.04;
        let bend2 = (self.t * 1.2 - 1.6).sin() * 0.07;

        // The plant node anchors the chain: its origin is plant1's lower
        // joint, its scale shrinks the whole plant, and the base rock turns
        // everything around that joint.
        let plant = &mut ctx.scene().root.children[0];
        plant.transform = frost::Transform::rotate(base)
            .compose(&frost::Transform::translate(ANCHOR[0], ANCHOR[1]));

        // Lay the chain out in the plant's own (unscaled) space: the first
        // slice's lower joint sits at the plant's origin, and each next
        // slice's lower joint sits on the previous slice's upper joint,
        // rotated by that slice's bend.
        let mut anchor = [0.0f32, 0.0];
        for (i, (link, node)) in self.links.iter().zip(&mut plant.children).enumerate() {
            let bend = match i {
                0 => 0.0,
                1 => bend1,
                _ => bend1 + bend2,
            };
            let rot = frost::Transform::rotate(bend);
            node.transform = frost::Transform::translate(-link.from[0], -link.from[1])
                .compose(&rot)
                .compose(&frost::Transform::translate(anchor[0], anchor[1]));
            // The next anchor: this slice's upper joint, measured from its
            // own lower joint and rotated by the slice's bend.
            let d = [link.to[0] - link.from[0], link.to[1] - link.from[1]];
            let step = rot.apply(d);
            anchor = [anchor[0] + step[0], anchor[1] + step[1]];
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
    let plant1 = frost::Shape::sprite(format!("{root}/assets/sprites/plant1.png"))
        .expect("failed to load assets/sprites/plant1.png");
    let plant2 = frost::Shape::sprite(format!("{root}/assets/sprites/plant2.png"))
        .expect("failed to load assets/sprites/plant2.png");
    let plant3 = frost::Shape::sprite(format!("{root}/assets/sprites/plant3.png"))
        .expect("failed to load assets/sprites/plant3.png");

    // The slices' joints in node-local space, from the hand-picked pixel
    // coordinates above and each texture's real size.
    let links = [
        link(&plant1, JOINTS[0]),
        link(&plant2, JOINTS[1]),
        link(&plant3, JOINTS[2]),
    ];

    let scene = frost::Scene::new(frost::SceneNode {
        // Night garden: a deep green-black sky behind the plant.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.03,
                g: 0.08,
                b: 0.04,
                a: 1.0,
            },
        }),
        children: vec![Box::new(frost::SceneNode {
            scale: [SCALE, SCALE],
            children: vec![
                Box::new(frost::SceneNode {
                    shape: Some(plant1),
                    ..Default::default()
                }),
                Box::new(frost::SceneNode {
                    shape: Some(plant2),
                    ..Default::default()
                }),
                Box::new(frost::SceneNode {
                    shape: Some(plant3),
                    ..Default::default()
                }),
            ],
            ..Default::default()
        })],
        ..Default::default()
    });

    if let Err(err) = frost::run(
        scene,
        Demo {
            t: 0.0,
            links,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
