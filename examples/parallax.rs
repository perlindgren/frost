//! The layers example with a camera added: a parallax demo with infinite
//! scrolling.
//!
//! A shapeless camera pivot node hangs under the player node, and the
//! scene's camera points at it, so the camera follows the player: the
//! player stays at the window origin while the rest of the scene moves
//! around it. `WASD` scrolls the world in the opposite direction of the
//! player's heading, and `Q`/`E` turn the view rather than the player
//! sprite — the player's rotation is the camera's rotation.
//!
//! There are four layers, back to front: the far squares (order -1, half
//! the camera's speed), the middle squares (order 0, the full speed), the
//! player (order 0.5, the full speed), and the near squares (order 1,
//! double speed), so moving the player makes the square layers sweep past
//! at different rates — the classic parallax effect.
//!
//! The three square layers repeat at the minimum repeat offset — the
//! first frame's window size in pixels, in both axes — so moving the
//! player scrolls each of them through endlessly: the same squares are
//! re-used in every tile with just a displacement, so no square is ever
//! drawn twice.
//!
//! The player's layer does *not* repeat: the player is a unique object,
//! not part of a pattern that should tile. (A camera-followed object in a
//! repeating layer would stay on screen too — its wrapped copies sit one
//! full period away, off the window's edge — but it would still be drawn
//! as a tiled copy wherever its period puts one, so a unique object gets
//! its own layer.)
//!
//! The background lives in the scene's root — the base group, which
//! follows the camera at the full speed 1.0; its transform is ignored
//! anyway, so it always fills the window.
//!
//! The rectangles and the layers' repeat values are generated once, on the
//! first frame — the window size is only known once the first frame runs —
//! and the rectangles keep their generated positions and colors for the
//! whole run. Each run gets a different set, seeded from the current time.
//!
//! Run with:
//!
//! ```text
//! cargo run --example parallax
//! ```

/// The player's linear speed, in pixels per second.
const SPEED: f32 = 200.0;

/// The player's angular speed, in radians per second — 360 degrees per second.
const ROT_SPEED: f32 = std::f32::consts::TAU;

/// The full width and height of each rectangle, in pixels.
const SIZE: f32 = 100.0;

/// How many rectangles each layer has.
const RECTS: usize = 4;

/// The layer orders, back to front: -1 the far squares, 0 the middle
/// squares, 0.5 the player, 1 the near squares.
const LAYER_ORDERS: [f32; 4] = [-1.0, 0.0, 0.5, 1.0];

/// The layer parallax speeds, matching `LAYER_ORDERS` back to front: the
/// far layer moves at half the camera's speed, the middle layer and the
/// player at the full speed, and the near layer at double speed.
const LAYER_SPEEDS: [f32; 4] = [0.5, 1.0, 1.0, 2.0];

/// The index of the layer the player lives in, between the middle and the
/// near square layers.
const PLAYER_LAYER: usize = 2;

/// The indices of the square layers among the scene's layers: the far,
/// middle, and near layers — every layer but the player's.
const SQUARE_LAYERS: [usize; 3] = [0, 1, 3];

/// A fixed rectangle: a square centered at `pos` in `color`.
struct Rect {
    /// The square's center in world pixels (window-centered coordinates).
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
    /// The player's position in world pixels (window-centered coordinates);
    /// the camera keeps the player at the window origin on screen.
    pos: [f32; 2],
    /// The player's (and camera's) facing angle in radians,
    /// counter-clockwise from +x.
    rot: f32,
    /// The rectangles per square layer, generated once from the first
    /// frame's window size and kept unchanged for the whole run.
    rects: Option<[Vec<Rect>; 3]>,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // The player, as in layers.rs.
        // `Q` turns counter-clockwise (a positive angle), `E` clockwise.
        let drot = axis(ctx, frost::KeyCode::KeyQ, frost::KeyCode::KeyE);
        self.rot += drot * ROT_SPEED * dt;

        // The WASD direction in the player's own frame, rotated into the
        // world by the facing angle. At `rot == 0` this is the unrotated
        // direction, i.e. the original behavior.
        let (sin, cos) = self.rot.sin_cos();
        let dx = axis(ctx, frost::KeyCode::KeyD, frost::KeyCode::KeyA);
        let dy = axis(ctx, frost::KeyCode::KeyW, frost::KeyCode::KeyS);
        self.pos[0] += (dx * cos - dy * sin) * SPEED * dt;
        self.pos[1] += (dx * sin + dy * cos) * SPEED * dt;

        // The player's node lives in the middle layer, not in the scene's
        // root; its shapeless child is the camera.
        let player = &mut ctx.scene().layers[PLAYER_LAYER].root.children[0];
        // Rotate about the player's center, then place it at `pos`. The
        // camera hangs under the player, so it follows with no work here.
        player.transform = frost::Transform::rotate(self.rot)
            .compose(&frost::Transform::translate(self.pos));

        // The rectangles: generated once, at random positions with the whole
        // square inside the visible area, in random colors, one set per
        // square layer. They are added to the layer roots as children with
        // fixed world transforms, so the parallax comes purely from the
        // camera view moving at each layer's speed.
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
            for (index, list) in SQUARE_LAYERS.iter().copied().zip(&rects) {
                let layer = &mut ctx.scene().layers[index];
                // The minimum repeat offset: the window's width and height
                // in pixels, set once here, on the first frame, when the
                // window size becomes known. The squares are static in the
                // layer's space, so the periodic copies tile them
                // seamlessly; the player's layer is left non-repeating (see
                // the module docs).
                layer.repeat = [w, h];
                for r in list {
                    // Local position: the square's extent is centered on the
                    // node's origin, so only the node's transform places it.
                    layer.root.children.push(Box::new(frost::SceneNode {
                        transform: frost::Transform::translate(r.pos),
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

    // The four layers, back to front, each with its parallax speed. Each
    // starts empty; the rectangles are added to the square layers at the
    // first frame, along with their repeat offsets. The player's layer
    // never repeats: a repeating layer is a periodic copy of its *current*
    // content, and the player moves through that content's space (see the
    // module docs).
    let mut layers: Vec<frost::Layer> = LAYER_ORDERS
        .iter()
        .zip(LAYER_SPEEDS.iter())
        .map(|(&order, &speed)| frost::Layer {
            order,
            speed,
            repeat: [0.0, 0.0],
            root: frost::SceneNode::default(),
        })
        .collect();
    // The player: the button node, with a shapeless camera pivot as its
    // child — the scene's camera, so the player stays at the window origin
    // and the world moves around it. It lives alone in its own
    // non-repeating layer, between the middle and the near square layers.
    let mut player = frost::SceneNode {
        shape: Some(button),
        ..Default::default()
    };
    player.children.push(Box::new(frost::SceneNode::default()));
    layers[PLAYER_LAYER].root.children.push(Box::new(player));

    if let Err(err) = frost::run(
        frost::Scene {
            // The background lives in the base group (the scene's root),
            // which follows the camera at the full speed 1.0; its transform
            // is ignored anyway.
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
            // The camera is the player's shapeless child pivot: the far
            // layer moves at half the camera's speed, the middle layer and
            // the player's layer at the full speed, and the near layer at
            // double speed.
            camera: Some(frost::NodePath {
                group: Some(PLAYER_LAYER),
                children: vec![0, 0],
            }),
            ambient: frost::AMBIENT,
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
