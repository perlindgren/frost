//! The player example with collision, from the `collision` module: the
//! button is an oriented box the size of the sprite, and it is pushed out of
//! four random obstacle squares and of the window edges, its velocity
//! reflected off each surface it meets (restitution `BOUNCE`). A thin
//! outline shows the button's actual collision box.
//!
//! The button is steered like a small car: `W`/`A`/`S`/`D` accelerate it
//! relative to the way it is facing, `Q`/`E` turn it, and with no key held
//! it coasts and slows, so a bounce carries it until you steer again.
//!
//! The collision module owns no state. This example owns the button's
//! position and velocity and applies the module's push-out and reflection
//! to them every frame.
//!
//! Run with:
//!
//! ```text
//! cargo run --example collision
//! ```

/// The button's top speed, in pixels per second: `ACCEL / DAMP`.
const SPEED: f32 = 100.0;

/// The button's acceleration while a movement key is held, in pixels per
/// square second.
const ACCEL: f32 = 500.0;

/// The drag on the button, in inverse seconds: each second it keeps
/// `1 - DAMP * dt` of its velocity. `ACCEL / DAMP` is the top speed.
const DAMP: f32 = 5.0;

/// The button's angular speed, in radians per second — 360 degrees per second.
const ROT_SPEED: f32 = std::f32::consts::TAU;

/// The restitution of a bounce: `0` kills the velocity into the surface,
/// `1` keeps it.
const BOUNCE: f32 = 0.6;

/// The full width and height of each obstacle square, in pixels.
const SIZE: f32 = 100.0;

/// How many obstacles the frame has.
const OBSTACLES: usize = 4;

/// The button sprite's half size in pixels: Button.png is 164 x 195.
const BUTTON_HALF: [f32; 2] = [82.0, 97.5];

/// The thickness of the invisible wall around the window, in pixels.
const WALL: f32 = 50.0;

/// The collision box outline, drawn on top of the button (z = 1.0).
const OUTLINE: frost::Color = frost::Color {
    r: 0.7,
    g: 0.9,
    b: 1.0,
    a: 0.9,
};

/// A fixed obstacle: a square centered at `pos` in `color`.
struct Obstacle {
    /// The square's center in window-centered pixels.
    pos: [f32; 2],
    color: frost::Color,
}

/// A tiny deterministic random source (splitmix64), so the example needs no
/// external random crate: seeded from the current time, it gives a different
/// obstacle set on each run.
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self(nanos)
    }

    /// The next uniform value in [0, 1).
    fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u32 << 24) as f32
    }

    /// The next uniform value in [lo, hi]. When `hi <= lo` — the window is
    /// smaller than an obstacle on that axis — the midpoint instead, so the
    /// value stays as central as it can be.
    fn in_range(&mut self, lo: f32, hi: f32) -> f32 {
        if hi <= lo {
            (lo + hi) / 2.0
        } else {
            lo + self.next_f32() * (hi - lo)
        }
    }
}

/// The direction along one axis from two held keys: `+1` if only `positive`
/// is down, `-1` if only `negative` is, `0` if both or neither is.
fn axis(ctx: &frost::Context, positive: frost::KeyCode, negative: frost::KeyCode) -> f32 {
    (ctx.key_down(positive) as i32 - ctx.key_down(negative) as i32) as f32
}

/// The demo state: the button's motion and the obstacles it collides with.
struct Demo {
    /// The button's position in window-centered pixels.
    pos: [f32; 2],
    /// The button's facing angle in radians, counter-clockwise from +x.
    rot: f32,
    /// The button's current velocity, in pixels per second.
    vel: [f32; 2],
    /// The fixed obstacles, generated once from the first frame's window
    /// size and kept unchanged for the whole run.
    obstacles: Option<Vec<Obstacle>>,
}

impl Demo {
    /// Pushes the button out of every collider it overlaps, reflecting its
    /// velocity off each surface it meets.
    fn collide(&mut self, ctx: &frost::Context) {
        let (w, h) = ctx.size();

        // The obstacles, plus one wall just outside each window edge,
        // reaching `WALL` past every corner so nothing slips between two.
        // Four separate walls, never one enclosing box: for a box that has
        // crossed an edge the SAT least-penetration answer is the single
        // face it crossed least, which is the wrong push away from the
        // other three.
        let mut colliders: Vec<frost::Collider> = Vec::with_capacity(OBSTACLES + 4);
        if let Some(obstacles) = &self.obstacles {
            for o in obstacles {
                colliders.push(frost::Collider::Box(frost::OrientedBox::new(
                    o.pos,
                    [SIZE / 2.0, SIZE / 2.0],
                )));
            }
        }
        colliders.push(frost::Collider::Box(frost::OrientedBox::new(
            [w / 2.0 + WALL / 2.0, 0.0],
            [WALL / 2.0, h / 2.0 + WALL],
        )));
        colliders.push(frost::Collider::Box(frost::OrientedBox::new(
            [-w / 2.0 - WALL / 2.0, 0.0],
            [WALL / 2.0, h / 2.0 + WALL],
        )));
        colliders.push(frost::Collider::Box(frost::OrientedBox::new(
            [0.0, h / 2.0 + WALL / 2.0],
            [w / 2.0 + WALL, WALL / 2.0],
        )));
        colliders.push(frost::Collider::Box(frost::OrientedBox::new(
            [0.0, -h / 2.0 - WALL / 2.0],
            [w / 2.0 + WALL, WALL / 2.0],
        )));

        // One collider at a time, rebuilt from the current position after
        // each push-out, so each test sees the correction from the last.
        for c in &colliders {
            let player = frost::Collider::Box(frost::OrientedBox::rotated(
                self.pos,
                BUTTON_HALF,
                self.rot,
            ));
            if let Some(po) = player.push_out(c) {
                self.pos[0] += po.dir[0] * po.depth;
                self.pos[1] += po.dir[1] * po.depth;
                self.vel = frost::reflect(self.vel, po.dir, BOUNCE);
            }
        }
    }
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // The engine already clamps `dt` to a second, but a full-second
        // step after a stall would move the button 100 pixels in one frame
        // and straight through an obstacle.
        let dt = dt.min(1.0 / 30.0);

        // `Q` turns counter-clockwise (a positive angle), `E` clockwise.
        let drot = axis(ctx, frost::KeyCode::KeyQ, frost::KeyCode::KeyE);
        self.rot += drot * ROT_SPEED * dt;

        // The WASD direction in the button's own frame, rotated into the
        // world by the facing angle, applied to the velocity as an
        // acceleration.
        let (sin, cos) = self.rot.sin_cos();
        let dx = axis(ctx, frost::KeyCode::KeyD, frost::KeyCode::KeyA);
        let dy = axis(ctx, frost::KeyCode::KeyW, frost::KeyCode::KeyS);
        self.vel[0] += (dx * cos - dy * sin) * ACCEL * dt;
        self.vel[1] += (dx * sin + dy * cos) * ACCEL * dt;

        // Drag: with no key held the button coasts and slows.
        let drag = (1.0 - DAMP * dt).max(0.0);
        self.vel[0] *= drag;
        self.vel[1] *= drag;

        // The top speed, `ACCEL / DAMP`, as a safety cap.
        let speed = (self.vel[0] * self.vel[0] + self.vel[1] * self.vel[1]).sqrt();
        if speed > SPEED {
            let k = SPEED / speed;
            self.vel[0] *= k;
            self.vel[1] *= k;
        }

        // Move, then resolve the collisions the move caused.
        self.pos[0] += self.vel[0] * dt;
        self.pos[1] += self.vel[1] * dt;
        self.collide(ctx);

        let button = &mut ctx.scene().root.children[0];
        // Rotate about the button's center, then place it at `pos`.
        button.transform = frost::Transform::rotate(self.rot)
            .compose(&frost::Transform::translate(self.pos[0], self.pos[1]));

        // The obstacles: generated once, at random positions with the whole
        // square inside the visible area, in random colors.
        let (w, h) = ctx.size();
        let obstacles = self.obstacles.get_or_insert_with(|| {
            let mut rng = Rng::new();
            (0..OBSTACLES)
                .map(|_| {
                    let half = SIZE / 2.0;
                    Obstacle {
                        pos: [
                            rng.in_range(-w / 2.0 + half, w / 2.0 - half),
                            rng.in_range(-h / 2.0 + half, h / 2.0 - half),
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

        // All at the default draw order (z = 0.0). The scene's button is
        // drawn after the process, so at equal z it ends up on top of them.
        for o in obstacles {
            ctx.rectangle(o.pos[0], o.pos[1], SIZE / 2.0, SIZE / 2.0, o.color, 0.0);
        }

        // The button's actual collision box, on top of the button itself
        // (the scene draws at z = 0.0, the outline at z = 1.0).
        let corners = frost::OrientedBox::rotated(self.pos, BUTTON_HALF, self.rot).corners();
        for i in 0..4 {
            let a = corners[i];
            let b = corners[(i + 1) % 4];
            ctx.line(a[0], a[1], b[0], b[1], OUTLINE, 2.0, 1.0);
        }

        log::trace!(
            "process: dt {:?} pos {:?} rot {:?} vel {:?}",
            dt,
            self.pos,
            self.rot,
            self.vel
        );
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

    if let Err(err) = frost::run(
        frost::Scene::new(frost::SceneNode {
            // Deep indigo background; the node's transform is ignored.
            shape: Some(frost::Shape::Background {
                color: frost::Color {
                    r: 0.09,
                    g: 0.06,
                    b: 0.16,
                    a: 1.0,
                },
            }),
            children: vec![Box::new(frost::SceneNode {
                // The button starts at the window center, facing +x.
                shape: Some(button),
                ..Default::default()
            })],
            ..Default::default()
        }),
        Demo {
            pos: [0.0, 0.0],
            rot: 0.0,
            vel: [0.0, 0.0],
            obstacles: None,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
