//! The dark torch-lit hall of the `cone` example, with physics added: the
//! walls that cut shadows out of the beam now also push the player back.
//!
//! The player is the lit square, steered with `W`/`A`/`S`/`D` (relative to
//! its facing, so `W` always walks toward where the torch points) and turned
//! with `Q` (counter-clockwise) and `E` (clockwise). A connected gamepad
//! drives the same controls: the left stick walks (up forward, left and
//! right strafing) and the right stick's left and right turns; a stick
//! inside the [`DEADZONE`] reads as no input, so keys and stick mix freely.
//! It carries the same two lights as `cone.rs`, as child nodes of its scene
//! node: a small omni point light (radius [`GLOW_RADIUS`]) pinned to its
//! center, and a torch — a cone light (radius [`TORCH_RADIUS`], full
//! opening angle [`TORCH_SPREAD`]) mounted on the square's front face and
//! aimed along its local +x, so its rotation sweeps the beam across the
//! room.
//!
//! The new part: everything the beam stops dead at, the player also stops
//! dead at. In `cone.rs` the walls are drawn by a `wall()` call each; here
//! the same four rectangles live exactly once, as a [`Vec<Wall>`], and each
//! half of the demo gets its own view of that one list:
//!
//! * the scene graph gets [`Wall::node`] — an unlit, `occludes: true`
//!   rectangle, drawn exactly as `cone.rs` drew its walls, and
//! * the process gets [`Wall::collider`] — an [`frost::OrientedBox`] with
//!   the same center, half extents and tilt, which the player is pushed out
//!   of every frame.
//!
//! Because both views read the same numbers, a wall that shadows but does
//! not block — or blocks but does not shadow — is not expressible: the
//! silhouette of every shadow is guaranteed to be the outline of every
//! obstacle, with no second list of coordinates to drift out of sync.
//!
//! Each frame, after the same movement as `cone.rs`, the player is resolved
//! against the walls one at a time (the collision example's idiom: rebuild
//! the player's box from the corrected position after each push, so every
//! test sees the last correction), with zero restitution — the velocity's
//! component into a wall dies and the rest slides along it, so walls feel
//! like walls, not trampolines. The window-edge clamp stays last, so a
//! push-out near an edge can never leave the player outside the window.
//!
//! Run with:
//!
//! ```text
//! cargo run --example cone_collider
//! ```

/// The player square's half extents, in pixels.
const PLAYER_HALF: [f32; 2] = [16.0, 16.0];

/// The point light's falloff extent, in pixels: the player's own dim pool.
const GLOW_RADIUS: f32 = 50.0;

/// The point light's strength.
const GLOW_INTENSITY: f32 = 1.2;

/// The torch's falloff extent, in pixels.
const TORCH_RADIUS: f32 = 300.0;

/// The torch's strength: brighter than the pool, so the beam reads as the
/// scene's key light.
const TORCH_INTENSITY: f32 = 2.2;

/// The torch's full opening angle, in radians: 20 degrees, so the beam
/// spans 10 degrees to each side of the facing axis.
const TORCH_SPREAD: f32 = 20.0_f32.to_radians();

/// The top walking speed, in pixels per second.
const SPEED: f32 = 180.0;

/// The acceleration the keys apply, in pixels per second, per second.
const ACCEL: f32 = 900.0;

/// The velocity damping per second: with no key held the player coasts
/// and slows.
const DAMP: f32 = 5.0;

/// The turning rate, in radians per second: a full turn per second.
const ROT_SPEED: f32 = std::f32::consts::TAU;

/// The gamepad stick deadzone: a resting stick jitters around zero, and
/// readings this small (or smaller) count as no input, so the keys take
/// over instead of the player creeping.
const DEADZONE: f32 = 0.15;

/// The player's color: warm brass, bright enough to read inside its own
/// pool.
const PLAYER: frost::Color = frost::Color {
    r: 0.85,
    g: 0.60,
    b: 0.30,
    a: 1.0,
};

/// The pool light's color: a warm white.
const GLOW: frost::Color = frost::Color {
    r: 1.0,
    g: 0.90,
    b: 0.72,
    a: 1.0,
};

/// The torch's color: a slightly warmer, flamey white.
const TORCH: frost::Color = frost::Color {
    r: 1.0,
    g: 0.85,
    b: 0.60,
    a: 1.0,
};

/// The backdrop's color: a mid slate, near-black at the scene's low
/// ambient, and a good bright gray wherever a light reaches.
const BACKDROP: frost::Color = frost::Color {
    r: 0.30,
    g: 0.30,
    b: 0.40,
    a: 1.0,
};

/// The occluder walls' color.
const WALL: frost::Color = frost::Color {
    r: 0.38,
    g: 0.36,
    b: 0.46,
    a: 1.0,
};

/// The direction along one axis from two held keys: `+1` if only `positive`
/// is down, `-1` if only `negative` is, `0` if both or neither is.
fn axis(ctx: &frost::Context, positive: frost::KeyCode, negative: frost::KeyCode) -> f32 {
    (ctx.key_down(positive) as i32 - ctx.key_down(negative) as i32) as f32
}

/// One wall of the hall: the single source of truth for a rectangle that
/// both occludes the lights and stops the player. The tilt leans the wall
/// counter-clockwise about its own center, and the half extents are exactly
/// what both views consume: the renderer draws them, the collider is built
/// from them, so the shadow's silhouette and the obstacle's outline are
/// the same rectangle by construction.
struct Wall {
    /// The wall's center in window-centered pixels.
    center: [f32; 2],
    /// The wall's half extents, in pixels.
    half: [f32; 2],
    /// The wall's lean in radians, counter-clockwise.
    tilt: f32,
}

impl Wall {
    /// The wall's scene node: left `lit: false` so it draws in its own full
    /// color regardless of the lights (a lit wall would vanish into the
    /// dark between light hits, which reads oddly for an obstacle), while
    /// its rectangle still cuts hard shadows out of every light behind it.
    fn node(&self) -> Box<frost::SceneNode> {
        Box::new(frost::SceneNode {
            // Lean by `tilt` about the wall's own center, then place it.
            transform: frost::Transform::rotate(self.tilt)
                .compose(&frost::Transform::translate(self.center)),
            shape: Some(frost::Shape::Rectangle {
                center: [0.0, 0.0],
                extent: self.half,
                color: WALL,
            }),
            lit: false,
            occludes: true,
            ..Default::default()
        })
    }

    /// The wall's physics body: the same rectangle, as an oriented box the
    /// player is pushed out of. The lean is the same positive-CCW angle the
    /// node's transform rotates by, so the box matches the drawing to the
    /// degree.
    fn collider(&self) -> frost::Collider {
        frost::Collider::Box(frost::OrientedBox::rotated(
            self.center,
            self.half,
            self.tilt,
        ))
    }
}

/// The demo state: the player's motion and the walls it shares the room
/// with.
struct Demo {
    /// The player's position in window-centered pixels.
    pos: [f32; 2],
    /// The player's facing angle in radians, counter-clockwise from +x:
    /// also the torch's axis.
    rot: f32,
    /// The player's current velocity, in pixels per second.
    vel: [f32; 2],
    /// The walls, shared by the scene (which was built from them) and the
    /// collision below (which is resolved against them).
    walls: Vec<Wall>,
}

impl Demo {
    /// Push the player out of every wall it ended the frame inside of.
    ///
    /// One wall at a time, rebuilding the player's box from the corrected
    /// position after each push so each test sees the correction from the
    /// last: the player can be wedged in a corner between two walls, and
    /// sequential resolution squeezes it out of both. The velocity is
    /// reflected with zero restitution, so the component into the wall dies
    /// and the rest slides along it — walking into a wall stops you at it,
    /// it does not bounce you off. The player's box turns with its facing,
    /// like the drawn square does.
    fn collide(&mut self) {
        for wall in &self.walls {
            let player =
                frost::Collider::Box(frost::OrientedBox::rotated(self.pos, PLAYER_HALF, self.rot));
            if let Some(push) = player.push_out(&wall.collider()) {
                self.pos[0] += push.dir[0] * push.depth;
                self.pos[1] += push.dir[1] * push.depth;
                self.vel = frost::reflect(self.vel, push.dir, 0.0);
            }
        }
    }
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let dt = dt.min(1.0 / 30.0);

        // The gamepad, when one is connected: the first pad's sticks drive
        // the same three inputs as the keys. A stick inside the deadzone
        // (a resting stick jitters around zero) reads as no input and the
        // keys take over, so the two can be mixed freely.
        let pad = ctx.gamepads().next();
        let stick = |a: frost::Axis| -> f32 {
            let value = pad.map(|pad| pad.value(a)).unwrap_or(0.0);
            if value.abs() < DEADZONE {
                0.0
            } else {
                value
            }
        };

        // `Q` turns counter-clockwise (a positive angle), `E` clockwise;
        // the right stick does the same, pushed right (positive X) turning
        // clockwise, so its reading is negated into the same convention.
        let drot = {
            let s = -stick(frost::Axis::RightStickX);
            if s != 0.0 {
                s
            } else {
                axis(ctx, frost::KeyCode::KeyQ, frost::KeyCode::KeyE)
            }
        };
        self.rot += drot * ROT_SPEED * dt;

        // The WASD direction in the square's own frame: the facing is the
        // square's local +x (where the torch points), so `W`/`S` drive
        // along that axis and `A`/`D` strafe across it — right is the
        // facing turned clockwise, the square's local -y. The left stick
        // walks the same frame (up forward, right strafe). The body-frame
        // vector (forward, -strafe) is rotated into the world by the facing
        // angle and applied to the velocity as an acceleration.
        let (sin, cos) = self.rot.sin_cos();
        let forward = {
            let s = stick(frost::Axis::LeftStickY);
            if s != 0.0 {
                s
            } else {
                axis(ctx, frost::KeyCode::KeyW, frost::KeyCode::KeyS)
            }
        };
        let strafe = {
            let s = stick(frost::Axis::LeftStickX);
            if s != 0.0 {
                s
            } else {
                axis(ctx, frost::KeyCode::KeyD, frost::KeyCode::KeyA)
            }
        };
        self.vel[0] += (forward * cos + strafe * sin) * ACCEL * dt;
        self.vel[1] += (forward * sin - strafe * cos) * ACCEL * dt;

        // Drag, then the top speed as a safety cap.
        let drag = (1.0 - DAMP * dt).max(0.0);
        self.vel[0] *= drag;
        self.vel[1] *= drag;
        let speed = (self.vel[0] * self.vel[0] + self.vel[1] * self.vel[1]).sqrt();
        if speed > SPEED {
            let k = SPEED / speed;
            self.vel[0] *= k;
            self.vel[1] *= k;
        }

        // Move, then resolve against the walls.
        self.pos[0] += self.vel[0] * dt;
        self.pos[1] += self.vel[1] * dt;
        self.collide();

        // Then keep the whole square inside the window, killing the
        // velocity's component into whichever edge was hit. The clamp runs
        // after the wall pushes, as the last word on position, so a
        // push-out beside an edge can never leave the player outside it.
        let (w, h) = ctx.size();
        let (limit_x, limit_y) = (w / 2.0 - PLAYER_HALF[0], h / 2.0 - PLAYER_HALF[1]);
        if self.pos[0].abs() > limit_x {
            self.pos[0] = self.pos[0].clamp(-limit_x, limit_x);
            self.vel[0] = 0.0;
        }
        if self.pos[1].abs() > limit_y {
            self.pos[1] = self.pos[1].clamp(-limit_y, limit_y);
            self.vel[1] = 0.0;
        }

        // Carry the square and everything it carries — the pool light and
        // the torch — along: rotate about its center, then place
        // it at `pos`. The lights are child nodes, so their spots and the
        // beam's axis ride this one transform write; nothing is emitted
        // per frame anymore.
        let player = &mut ctx.scene().root.children[0];
        player.transform =
            frost::Transform::rotate(self.rot).compose(&frost::Transform::translate(self.pos));

        log::trace!(
            "process: dt {:?} pos {:?} rot {:?} vel {:?}",
            dt,
            self.pos,
            self.rot,
            self.vel
        );
    }
}

/// A lit panel node near the given spot, in the given color: a receiver
/// only, so the beam lights it and the walls shadow it, but it never
/// shadows anything itself.
fn panel(pos: [f32; 2], color: frost::Color) -> Box<frost::SceneNode> {
    Box::new(frost::SceneNode {
        order: -0.5,
        shape: Some(frost::Shape::Rectangle {
            center: [0.0, 0.0],
            extent: [110.0, 26.0],
            color,
        }),
        transform: frost::Transform::translate(pos),
        lit: true,
        ..Default::default()
    })
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // The four walls, declared once. Everything downstream — the scene's
    // occluder nodes and the process's colliders — is derived from this
    // single list, so shadow and obstacle can never disagree.
    let walls = vec![
        // A vertical wall left, a leaned wall center, a wide slab right,
        // and a small block below: the same four rectangles `cone.rs`
        // draws.
        Wall {
            center: [-330.0, 20.0],
            half: [24.0, 150.0],
            tilt: 0.0,
        },
        Wall {
            center: [60.0, 10.0],
            half: [24.0, 170.0],
            tilt: 0.35,
        },
        Wall {
            center: [330.0, -30.0],
            half: [160.0, 24.0],
            tilt: 0.0,
        },
        Wall {
            center: [-70.0, -200.0],
            half: [36.0, 36.0],
            tilt: 0.0,
        },
    ];

    // The player node first: the square and the two lights it carries,
    // drawn above the room's scenery by its `order`, and moved by the
    // process below.
    let player = Box::new(frost::SceneNode {
        order: 1.0,
        shape: Some(frost::Shape::Rectangle {
            center: [0.0, 0.0],
            extent: PLAYER_HALF,
            color: PLAYER,
        }),
        lit: true,
        children: vec![
            Box::new(frost::SceneNode {
                // The pool light at the square's own center: a light node,
                // so it is never drawn — it just rides the parent, keeping
                // its omni glow pinned to the player.
                shape: Some(frost::Shape::Light {
                    light: frost::Light::point(GLOW, GLOW_INTENSITY, GLOW_RADIUS),
                }),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The torch, mounted on the square's front face: local +x
                // at the surface itself (x = PLAYER_HALF[0]), where the
                // player's collider stops — an offset mount would poke
                // through a wall the square cannot enter. Its `direction`
                // is 0 in the square's local space, along that +x, and the
                // square's rotation, folded in by parenting, is what aims
                // the beam. No per-frame angle bookkeeping.
                transform: frost::Transform::translate([PLAYER_HALF[0], 0.0]),
                shape: Some(frost::Shape::Light {
                    light: frost::Light::cone(
                        TORCH,
                        TORCH_INTENSITY,
                        TORCH_RADIUS,
                        0.0,
                        TORCH_SPREAD,
                    ),
                }),
                ..Default::default()
            }),
        ],
        ..Default::default()
    });

    let mut scene = frost::Scene::new(frost::SceneNode {
        // Near-black clear color behind the lit backdrop, visible only
        // outside the huge rectangle's reach.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.02,
                g: 0.02,
                b: 0.035,
                a: 1.0,
            },
        }),
        children: vec![
            // The player square, first in the tree's children — the
            // process below drives it as `children[0]` — and on top of
            // everything by its `order`.
            player,
            Box::new(frost::SceneNode {
                // The backdrop: a huge lit rectangle far beyond the
                // window's edges, so the torch pool and every wall's
                // shadow stretch out over it however the player stands.
                // It is a receiver only: flagging it would shadow the
                // whole scene behind its own silhouette.
                order: -1.0,
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, 0.0],
                    extent: [2000.0, 2000.0],
                    color: BACKDROP,
                }),
                lit: true,
                ..Default::default()
            }),
            // Four lit panels near the room's edges: cream and rose in
            // open floor, blue and green tucked behind walls, so walking
            // into a wall visibly drops the panel behind it.
            panel(
                [-330.0, -195.0],
                frost::Color {
                    r: 0.85,
                    g: 0.68,
                    b: 0.45,
                    a: 1.0,
                },
            ),
            panel(
                [60.0, 210.0],
                frost::Color {
                    r: 0.55,
                    g: 0.68,
                    b: 0.95,
                    a: 1.0,
                },
            ),
            panel(
                [340.0, -180.0],
                frost::Color {
                    r: 0.50,
                    g: 0.85,
                    b: 0.62,
                    a: 1.0,
                },
            ),
            panel(
                [-120.0, 170.0],
                frost::Color {
                    r: 0.90,
                    g: 0.50,
                    b: 0.60,
                    a: 1.0,
                },
            ),
        ],
        ..Default::default()
    });

    // Every wall enters the scene through the same list the process below
    // collides against: one `Wall` per occluder, no second set of
    // coordinates.
    for wall in &walls {
        scene.root.children.push(wall.node());
    }

    // A low ambient floor: without it (at the default gray) the room
    // would read fully lit and the torch would reveal nothing.
    scene.ambient = frost::Color {
        r: 0.06,
        g: 0.06,
        b: 0.09,
        a: 1.0,
    };

    if let Err(err) = frost::run(
        scene,
        Demo {
            pos: [0.0, 0.0],
            rot: 0.0,
            vel: [0.0, 0.0],
            walls,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
