//! The obstacles example with rendering layers added.
//!
//! The frame is split into three layers with orders -1, 0, and 1, each
//! holding four `SIZE` x `SIZE` squares at random positions (fully inside
//! the visible area) with random colors and the default draw order. A layer
//! is a hard draw partition: a whole layer is drawn after every lower-
//! ordered layer and before every higher-ordered one (higher order closer
//! to the camera, on top), no matter what the local z values are; within a
//! layer the local draw order applies.
//!
//! The background lives in the scene's root — the base group at the
//! implicit order 0.0 — so it is still rendered first, behind all the
//! layers. The button is in the middle layer (order 0.0); it is unchanged
//! from player.rs and sits on top of that layer's squares (local order 1).
//!
//! The rectangles are generated once, on the first frame — the window size
//! is only known once the first frame runs — and keep their generated
//! positions and colors for the whole run. Each run gets a different set,
//! seeded from the current time.
//!
//! Run with:
//!
//! ```text
//! cargo run --example layers
//! ```

/// The button's linear speed, in pixels per second.
const SPEED: f32 = 100.0;

/// The button's angular speed, in radians per second — 360 degrees per second.
const ROT_SPEED: f32 = std::f32::consts::TAU;

/// The full width and height of each rectangle, in pixels.
const SIZE: f32 = 100.0;

/// How many rectangles each layer has.
const RECTS: usize = 4;

/// The layer orders, back to front: -1 behind everything, 0 the player's
/// layer, 1 on top.
const LAYER_ORDERS: [f32; 3] = [-1.0, 0.0, 1.0];

/// The index of the layer the button lives in (its order is 0.0).
const PLAYER_LAYER: usize = 1;

/// A fixed rectangle: a square centered at `pos` in `color`.
struct Rect {
    /// The square's center in window-centered pixels.
    pos: [f32; 2],
    color: frost::Color,
}

/// [`frost::Rng::in_range`], but when the window is smaller than a
/// rectangle on that axis (`hi <= lo`) the midpoint instead, so the value
/// stays as central as it can be; no draw is consumed in that case, so the
/// rest of the stream is unaffected.
fn in_range(rng: &mut frost::Rng, lo: f32, hi: f32) -> f32 {
    if hi <= lo {
        (lo + hi) / 2.0
    } else {
        rng.in_range(lo, hi)
    }
}

/// The direction along one axis from two held keys: `+1` if only `positive`
/// is down, `-1` if only `negative` is, `0` if both or neither is.
fn axis(ctx: &frost::Context, positive: frost::KeyCode, negative: frost::KeyCode) -> f32 {
    (ctx.key_down(positive) as i32 - ctx.key_down(negative) as i32) as f32
}

struct Demo {
    /// The button's position in window-centered pixels.
    pos: [f32; 2],
    /// The button's facing angle in radians, counter-clockwise from +x.
    rot: f32,
    /// The rectangles per layer, generated once from the first frame's
    /// window size and kept unchanged for the whole run.
    rects: Option<[Vec<Rect>; 3]>,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // The player, unchanged from player.rs.
        // `Q` turns counter-clockwise (a positive angle), `E` clockwise.
        let drot = axis(ctx, frost::KeyCode::KeyQ, frost::KeyCode::KeyE);
        self.rot += drot * ROT_SPEED * dt;

        // The WASD direction in the button's own frame, rotated into the
        // world by the facing angle. At `rot == 0` this is the unrotated
        // direction, i.e. the original behavior.
        let (sin, cos) = self.rot.sin_cos();
        let dx = axis(ctx, frost::KeyCode::KeyD, frost::KeyCode::KeyA);
        let dy = axis(ctx, frost::KeyCode::KeyW, frost::KeyCode::KeyS);
        self.pos[0] += (dx * cos - dy * sin) * SPEED * dt;
        self.pos[1] += (dx * sin + dy * cos) * SPEED * dt;

        // The button's node lives in the middle layer, not in the scene's
        // root.
        let button = &mut ctx.scene().layers[PLAYER_LAYER].root.children[0];
        // Rotate about the button's center, then place it at `pos`.
        button.transform = frost::Transform::rotate(self.rot)
            .compose(&frost::Transform::translate(self.pos[0], self.pos[1]));

        // The rectangles: generated once, at random positions with the whole
        // square inside the visible area, in random colors, one set per
        // layer. They are added to the layer roots as children, so each
        // layer's squares are drawn as that layer's own group.
        if self.rects.is_none() {
            let (w, h) = ctx.size();
            let mut rng = frost::Rng::new();
            let rects: [Vec<Rect>; 3] = [0, 1, 2].map(|_| {
                (0..RECTS)
                    .map(|_| {
                        let half = SIZE / 2.0;
                        Rect {
                            pos: [
                                in_range(&mut rng, -w / 2.0 + half, w / 2.0 - half),
                                in_range(&mut rng, -h / 2.0 + half, h / 2.0 - half),
                            ],
                            color: frost::Color {
                                r: rng.next_f32(),
                                g: rng.next_f32(),
                                b: rng.next_f32(),
                                a: 1.0,
                            },
                        }
                    })
                    .collect()
            });
            for (layer, list) in ctx.scene().layers.iter_mut().zip(&rects) {
                for r in list {
                    // Local position: the square's extent is centered on the
                    // node's origin, so only the node's transform places it.
                    layer.root.children.push(Box::new(frost::SceneNode {
                        transform: frost::Transform::translate(r.pos[0], r.pos[1]),
                        shape: Some(frost::Shape::Rectangle {
                            center: [0.0, 0.0],
                            extent: [SIZE / 2.0, SIZE / 2.0],
                            color: r.color,
                        }),
                        ..Default::default()
                    }));
                }
            }
            self.rects = Some(rects);
        }

        log::trace!("process: dt {:?} pos {:?} rot {:?}", dt, self.pos, self.rot);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset path to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let button = frost::Shape::sprite(format!("{root}/assets/sprites/Button.png"))
        .expect("failed to load assets/sprites/Button.png");

    // The three layers, back to front. Each starts empty; the rectangles are
    // added to them at the first frame.
    let mut layers: Vec<frost::Layer> = LAYER_ORDERS
        .iter()
        .map(|&order| frost::Layer {
            order,
            speed: 1.0,
            repeat: [0.0, 0.0],
            root: frost::SceneNode::default(),
        })
        .collect();
    layers[PLAYER_LAYER].root.children.push(Box::new(frost::SceneNode {
        // The button starts at the window center, facing +x; order 1.0 puts
        // it on top of its layer's squares, which all default to 0.0.
        order: 1.0,
        shape: Some(button),
        ..Default::default()
    }));

    if let Err(err) = frost::run(
        frost::Scene {
            // The background lives in the base group (the scene's root),
            // which paints behind every explicit layer; its transform is
            // ignored.
            root: frost::SceneNode {
                shape: Some(frost::Shape::Background {
                    color: frost::Color {
                        r: 0.09,
                        g: 0.06,
                        b: 0.16,
                        a: 1.0,
                    },
                }),
                ..Default::default()
            },
            layers,
            camera: None,
            ambient: frost::Scene::default().ambient,
        },
        Demo {
            pos: [0.0, 0.0],
            rot: 0.0,
            rects: None,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
