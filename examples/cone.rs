//! A square explorer carrying a torch through a dark hall of walls.
//!
//! The player is the lit square, steered with `W`/`A`/`S`/`D` (relative to
//! its facing, so `W` always walks toward where the torch points) and turned
//! with `Q` (counter-clockwise) and `E` (clockwise). It carries two lights,
//! both *child nodes* of the player's scene node, so they ride its position
//! and turn with it for free:
//!
//! * a small omni point light (radius [`GLOW_RADIUS`]) at the square's
//!   center — the dim pool that always surrounds the player, so it never
//!   fully vanishes, and
//! * a torch: a cone light (radius [`TORCH_RADIUS`], full opening angle
//!   [`TORCH_SPREAD`] — 20 degrees, 10 to each side of the axis) mounted at
//!   the grip and aimed along the square's local +x — its facing — so the
//!   square's rotation, carried down by parenting, sweeps the beam across
//!   the room.
//!
//! The room is a huge lit backdrop (the "floor and walls" the beams play
//! over), four colored lit panels near its edges, and a few lit occluder
//! walls. Only rectangles occlude, and a wall flagged `occludes: true` cuts
//! a hard shadow out of every light behind it — so the torch beam stops
//! dead at a wall, and a panel or the backdrop behind a wall drops to the
//! ambient floor while the beam sweeps past. The middle wall is leaned
//! over: its shadow leans with it. The player itself never occludes, so
//! the torch never casts the player's own shadow.
//!
//! The scene's [`ambient`](frost::Scene::ambient) floor is set low, so
//! unlit regions read as near-black and the torch is what reveals the room.
//!
//! Run with:
//!
//! ```text
//! cargo run --example cone
//! ```

/// The player square's half extents, in pixels.
const PLAYER_HALF: [f32; 2] = [16.0, 16.0];

/// The point light's falloff extent, in pixels: the player's own dim pool.
const GLOW_RADIUS: f32 = 50.0;

/// The point light's strength.
const GLOW_INTENSITY: f32 = 1.2;

/// The torch's falloff extent, in pixels.
const TORCH_RADIUS: f32 = 400.0;

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

/// The player's color: warm brass, bright enough to read inside its own
/// pool.
const PLAYER: frost::Color = frost::Color {
    r: 0.85,
    g: 0.60,
    b: 0.30,
    a: 1.0,
};

/// The torch grip's color: a dark wooden stub sticking out of the square's
/// +x side, so the facing is legible even outside the beam.
const GRIP: frost::Color = frost::Color {
    r: 0.40,
    g: 0.28,
    b: 0.16,
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

/// The demo state: the player's motion.
struct Demo {
    /// The player's position in window-centered pixels.
    pos: [f32; 2],
    /// The player's facing angle in radians, counter-clockwise from +x:
    /// also the torch's axis.
    rot: f32,
    /// The player's current velocity, in pixels per second.
    vel: [f32; 2],
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let dt = dt.min(1.0 / 30.0);

        // `Q` turns counter-clockwise (a positive angle), `E` clockwise.
        let drot = axis(ctx, frost::KeyCode::KeyQ, frost::KeyCode::KeyE);
        self.rot += drot * ROT_SPEED * dt;

        // The WASD direction in the square's own frame: the facing is the
        // square's local +x (where its grip and the torch point), so `W`/`S`
        // drive along that axis and `A`/`D` strafe across it — right is the
        // facing turned clockwise, the square's local -y. The body-frame
        // vector (forward, -strafe) is rotated into the world by the facing
        // angle and applied to the velocity as an acceleration.
        let (sin, cos) = self.rot.sin_cos();
        let forward = axis(ctx, frost::KeyCode::KeyW, frost::KeyCode::KeyS);
        let strafe = axis(ctx, frost::KeyCode::KeyD, frost::KeyCode::KeyA);
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

        // Move, then keep the whole square inside the window, killing the
        // velocity's component into whichever edge was hit.
        self.pos[0] += self.vel[0] * dt;
        self.pos[1] += self.vel[1] * dt;
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

        // Carry the square and everything it carries — the grip, the pool
        // light and the torch — along: rotate about its center, then place
        // it at `pos`. The lights are child nodes, so their spots and the
        // beam's axis ride this one transform write; nothing is emitted
        // per frame anymore.
        let player = &mut ctx.scene().root.children[0];
        player.transform = frost::Transform::rotate(self.rot)
            .compose(&frost::Transform::translate(self.pos[0], self.pos[1]));

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
        transform: frost::Transform::translate(pos[0], pos[1]),
        lit: true,
        ..Default::default()
    })
}

/// An occluder wall node: lit on its own face, and the rectangle whose
/// silhouette cuts hard shadows out of every light behind it.
fn wall(center: [f32; 2], extent: [f32; 2], tilt: f32) -> Box<frost::SceneNode> {
    Box::new(frost::SceneNode {
        // Lean by `tilt` about the wall's own center, then place it.
        transform: frost::Transform::rotate(tilt)
            .compose(&frost::Transform::translate(center[0], center[1])),
        shape: Some(frost::Shape::Rectangle {
            center: [0.0, 0.0],
            extent,
            color: WALL,
        }),
        lit: true,
        occludes: true,
        ..Default::default()
    })
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // The player node first: the square, its torch grip, and the two
    // lights it carries, drawn above the room's scenery by its `order`,
    // and moved by the process below.
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
                // The grip stub, sticking out of the square's facing side
                // (+x in the square's local space), riding every turn.
                shape: Some(frost::Shape::Rectangle {
                    center: [28.0, 0.0],
                    extent: [12.0, 4.0],
                    color: GRIP,
                }),
                lit: true,
                ..Default::default()
            }),
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
                // The torch, mounted at the grip: its `direction` is 0 in
                // the square's local space — along its +x, the facing —
                // and the square's rotation, folded in by parenting, is
                // what aims the beam. No per-frame angle bookkeeping.
                transform: frost::Transform::translate(28.0, 0.0),
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
            // open floor, blue and green tucked behind walls, so sweeping
            // the torch past a wall visibly drops them to the ambient
            // floor.
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
            // The walls: a vertical wall left, a leaned wall center (its
            // shadow leans with it), a wide slab right, and a small block
            // below. All occlude: the torch beam stops dead at each.
            wall([-330.0, 20.0], [24.0, 150.0], 0.0),
            wall([60.0, 10.0], [24.0, 170.0], 0.35),
            wall([330.0, -30.0], [160.0, 24.0], 0.0),
            wall([-70.0, -200.0], [36.0, 36.0], 0.0),
        ],
        ..Default::default()
    });

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
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
