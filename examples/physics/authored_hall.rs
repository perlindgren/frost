//! A dark hall of bricks whose shadows and collisions are written in a
//! picture editor, not in this file: the room is built from `brick.png`
//! instances, and every wall's footprint — where it stops the light,
//! where it stops the player, how bouncy a hit off it feels — comes from
//! the sidecar beside it, `assets/sprites/brick.ron`, through the crate's
//! load-time seam, [`frost::bake_sidecar`].
//!
//! The seam runs exactly once, before the window opens: read the PNG's
//! size, read the sidecar beside it, parse, read the vocabulary, bake.
//! What comes back is one [`frost::Baked`] — the brick's authored set in
//! the brick's own space — and the whole demo is what a game does with
//! it, per instance and per frame:
//!
//! * [`frost::Baked::declare_occluders`] under each brick's transform, so
//!   every brick cuts its shadow out of the player's torch-glow — the
//!   sprites themselves are *not* marked `occludes`, the silhouette is
//!   the authored rectangles', not the pictures';
//! * and the baked solids, translated to each brick, are what the player
//!   is pushed out of, `reflect`ing off them with the restitution the
//!   sidecar's `(bounce: 0.35)` named.
//!
//! One sidecar, one bake, thirty-odd bricks: the same records ride every
//! brick's transform, which is the bake's whole point — the geometry was
//! authored once, in the texture's pixels, and nothing downstream
//! re-reads the file.
//!
//! If the sidecar is mis-authored the example *refuses to start* and
//! prints the reader's whole report — every entry's every defect, at
//! once. That is the policy the seam exists to make possible: a game
//! silently missing its collision is the bug nobody can see by playing.
//!
//! The player is the lit square, steered with `W`/`A`/`S`/`D`, and it
//! carries two of the `cone` example's lights: a dim round pool at its
//! feet, and a torch whose axis always points from the player toward the
//! mouse cursor. The beam is the scene's key light, and the bricks'
//! baked rectangles are what it casts shadows of — one authored
//! footprint feeding both the round light and the cone. (The immediate
//! mode cone is hard-edged by design: no feather knob, no penumbra.)
//! Run with:
//!
//! ```text
//! cargo run --example authored_hall
//! ```

/// The player square's half extents, in pixels.
const PLAYER_HALF: [f32; 2] = [12.0, 12.0];

/// The player's top speed, in pixels per second: `ACCEL / DAMP`.
const SPEED: f32 = 160.0;

/// The player's acceleration while a movement key is held.
const ACCEL: f32 = 800.0;

/// The drag on the player, in inverse seconds.
const DAMP: f32 = 5.0;

/// The restitution off the window's invisible edge walls: the bricks
/// bring their own, from the sidecar; edges are not authored things.
const EDGE_BOUNCE: f32 = 0.0;

/// The player's light pool: small, dim and warm — a nightlight under
/// the torch, so the hall outside the beam is dark and the bricks'
/// shadows read against it.
const GLOW_RADIUS: f32 = 240.0;
const GLOW_INTENSITY: f32 = 1.0;

/// The torch's falloff extent, in pixels.
const TORCH_RADIUS: f32 = 420.0;

/// The torch's strength: brighter than the pool, so the beam reads as
/// the room's key light.
const TORCH_INTENSITY: f32 = 2.2;

/// The torch's full opening angle, in radians: 24 degrees, so the beam
/// spans 12 to each side of the aim.
const TORCH_SPREAD: f32 = 24.0_f32.to_radians();

/// The floor the light falls on, and the window's dark beyond it.
const LIT: frost::Color = frost::Color {
    r: 0.62,
    g: 0.60,
    b: 0.70,
    a: 1.0,
};

/// The brick picture's size in pixels — and so the pitch of the brick
/// walls, laid edge to edge on the picture's own grid.
const BRICK: f32 = 64.0;

/// The direction along one axis from two held keys: `+1` if only
/// `positive` is down, `-1` if only `negative` is, `0` if both or neither.
fn axis(ctx: &frost::Context, positive: frost::KeyCode, negative: frost::KeyCode) -> f32 {
    (ctx.key_down(positive) as i32 - ctx.key_down(negative) as i32) as f32
}

/// One baked collider moved to stand where a brick instance stands.
/// This is all "the set rides the instance" needs when the instances are
/// unturned; a game that turns its sprites would add the same tilt the
/// node carries — declare_occluders takes the full transform, the CPU
/// colliders are the ones with no transform helper yet.
fn placed(c: &frost::Collider, at: [f32; 2]) -> frost::Collider {
    match *c {
        frost::Collider::Box(mut b) => {
            b.center = [b.center[0] + at[0], b.center[1] + at[1]];
            frost::Collider::Box(b)
        }
        frost::Collider::Circle(mut d) => {
            d.center = [d.center[0] + at[0], d.center[1] + at[1]];
            frost::Collider::Circle(d)
        }
    }
}

/// The hall, sized to the window it is built in: two side columns
/// standing flush against the clip edges, a top wall with a doorway in
/// the middle, and three free-standing pillars. The layout happens
/// exactly once, from the first frame's `ctx.size()` — the collision
/// example's one-time pattern — so a Retina window, a maximised one and
/// a half-screen one all get a whole room instead of a cropped one.
fn hall(w: f32, h: f32) -> Vec<[f32; 2]> {
    let mut bricks = Vec::new();
    let edge = BRICK / 2.0;
    let mut y = -h / 2.0 + edge;
    while y <= h / 2.0 - edge {
        bricks.push([-w / 2.0 + edge, y]);
        bricks.push([w / 2.0 - edge, y]);
        y += BRICK;
    }
    let mut x = -w / 2.0 + edge;
    while x <= w / 2.0 - edge {
        if x.abs() > BRICK {
            bricks.push([x, h / 2.0 - edge]);
        }
        x += BRICK;
    }
    bricks.push([-w / 6.0, -h / 10.0]);
    bricks.push([w / 6.0, h / 8.0]);
    bricks.push([w / 8.0, -h / 3.0]);
    bricks
}

/// The demo state: the player's motion, and the two halves of the one
/// bake every brick shares.
struct Demo {
    /// The player's position in window-centered pixels.
    pos: [f32; 2],
    /// The player's velocity, in pixels per second.
    vel: [f32; 2],
    /// The brick centers — filled by the layout on the first frame, and
    /// the same numbers the scene's brick nodes were pushed with: one
    /// source, the scene graph and the process each get their own view
    /// of it.
    bricks: Vec<[f32; 2]>,
    /// The brick picture, cloned onto every node the layout pushes.
    brick: frost::Shape,
    /// The one bake, from the one sidecar.
    baked: frost::Baked,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let dt = dt.min(1.0 / 30.0);

        // The first frame builds the room, once, at the window's real
        // size: brick nodes go onto the scene (the player keeps seat 0)
        // and the player starts in the open courtyard below the pillars.
        if self.bricks.is_empty() {
            let (w, h) = ctx.size();
            self.pos = [0.0, -h / 2.0 + BRICK * 2.0];
            self.bricks = hall(w, h);
            for at in &self.bricks {
                ctx.scene().root.children.push(Box::new(frost::SceneNode {
                    shape: Some(self.brick.clone()),
                    transform: frost::Transform::translate(*at),
                    ..Default::default()
                }));
            }
        }

        // Straight WASD steering — this hall is about what the walls
        // are made of, not about driving.
        let dx = axis(ctx, frost::KeyCode::KeyD, frost::KeyCode::KeyA);
        let dy = axis(ctx, frost::KeyCode::KeyW, frost::KeyCode::KeyS);
        self.vel[0] += dx * ACCEL * dt;
        self.vel[1] += dy * ACCEL * dt;
        let drag = (1.0 - DAMP * dt).max(0.0);
        self.vel[0] *= drag;
        self.vel[1] *= drag;
        let speed = (self.vel[0] * self.vel[0] + self.vel[1] * self.vel[1]).sqrt();
        if speed > SPEED {
            let k = SPEED / speed;
            self.vel[0] *= k;
            self.vel[1] *= k;
        }
        self.pos[0] += self.vel[0] * dt;
        self.pos[1] += self.vel[1] * dt;

        // The collisions: every brick's share of the shared bake, moved
        // to the brick, tested one at a time — the collision example's
        // idiom of rebuilding the player's box after every push, so
        // each test sees the last correction. The restitution is the
        // sidecar's: `(bounce: 0.35)`, arriving here through a PNG
        // editor and a RON file.
        for brick in &self.bricks {
            for solid in &self.baked.solids {
                let wall = placed(&solid.shape, *brick);
                let player = frost::Collider::Box(frost::OrientedBox::new(self.pos, PLAYER_HALF));
                if let Some(po) = player.push_out(&wall) {
                    self.pos[0] += po.dir[0] * po.depth;
                    self.pos[1] += po.dir[1] * po.depth;
                    self.vel = frost::reflect(self.vel, po.dir, solid.bounce);
                }
            }
        }

        // The window's edge walls — four, never one enclosing box (the
        // collision example's reason) — un-bouncy: the bricks are the
        // authored things here, the frame of the window is not.
        let (w, h) = ctx.size();
        // Each wall's inner face sits exactly on the window's edge —
        // push the 50-thick body fully outside — so the barrier reads
        // as the window's border itself, not a wall floating inside it.
        let edges = [
            frost::Collider::Box(frost::OrientedBox::new(
                [w / 2.0 + 50.0, 0.0],
                [50.0, h / 2.0 + 50.0],
            )),
            frost::Collider::Box(frost::OrientedBox::new(
                [-w / 2.0 - 50.0, 0.0],
                [50.0, h / 2.0 + 50.0],
            )),
            frost::Collider::Box(frost::OrientedBox::new(
                [0.0, h / 2.0 + 50.0],
                [w / 2.0 + 50.0, 50.0],
            )),
            frost::Collider::Box(frost::OrientedBox::new(
                [0.0, -h / 2.0 - 50.0],
                [w / 2.0 + 50.0, 50.0],
            )),
        ];
        for edge in &edges {
            let player = frost::Collider::Box(frost::OrientedBox::new(self.pos, PLAYER_HALF));
            if let Some(po) = player.push_out(edge) {
                self.pos[0] += po.dir[0] * po.depth;
                self.pos[1] += po.dir[1] * po.depth;
                self.vel = frost::reflect(self.vel, po.dir, EDGE_BOUNCE);
            }
        }

        // The shadows: every brick declares the shared bake under its
        // own transform. No brick sprite carries `occludes` — the
        // silhouette the light field gets is the one the sidecar drew.
        for brick in &self.bricks {
            self.baked
                .declare_occluders(ctx, frost::Transform::translate(*brick));
        }

        // The player's pool, its torch, and the player. The torch aims
        // wherever the mouse is: sweep the cursor along a wall and every
        // brick's authored rectangle stretches a hard shadow the other
        // way, the beam's second reading of the same one bake.
        let warm = frost::Color {
            r: 1.0,
            g: 0.85,
            b: 0.6,
            a: 1.0,
        };
        ctx.light(self.pos[0], self.pos[1], warm, GLOW_INTENSITY, GLOW_RADIUS);
        if let Some(aim) = ctx.mouse_position() {
            let dx = aim[0] - self.pos[0];
            let dy = aim[1] - self.pos[1];
            if dx * dx + dy * dy > 1.0 {
                ctx.light_cone(
                    self.pos[0],
                    self.pos[1],
                    warm,
                    TORCH_INTENSITY,
                    TORCH_RADIUS,
                    dy.atan2(dx),
                    TORCH_SPREAD,
                );
            }
        }
        let player = &mut ctx.scene().root.children[0];
        player.transform = frost::Transform::translate(self.pos);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset path to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let png = format!("{root}/assets/sprites/brick.png");
    let brick = frost::Shape::sprite(&png).expect("failed to load assets/sprites/brick.png");

    // The seam, once, loudly: a sidecar that does not read is a hall
    // with silent holes in it, and that hall does not get to open.
    let baked = frost::bake_sidecar(&png).unwrap_or_else(|err| {
        eprintln!("brick.ron refused: {err}");
        std::process::exit(1);
    });
    log::info!(
        "baked {} occluders and {} solids from one sidecar",
        baked.occluders.len() + baked.disc_occluders.len() + baked.poly_occluders.len(),
        baked.solids.len(),
    );

    // The player keeps seat 0 of the scene; the bricks are laid by the
    // process on its first frame, when the window's real size is known
    // — every brick then declares and collides against this one bake.
    let mut scene = frost::Scene::new(frost::SceneNode {
        // The beyond: darker than the darkest floor the glow can
        // reach, so a shadow is a hole in the room, not a dimmer
        // room.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.02,
                g: 0.02,
                b: 0.03,
                a: 1.0,
            },
        }),
        children: vec![
            Box::new(frost::SceneNode {
                // The lit square the process drives as `children[0]`,
                // on top of the room's scenery by its `order` — and a
                // *receiver*: it is the light's glow on this square,
                // and on the backdrop, that makes the room a lit one.
                order: 1.0,
                lit: true,
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, 0.0],
                    extent: PLAYER_HALF,
                    color: frost::Color {
                        r: 0.95,
                        g: 0.92,
                        b: 0.85,
                        a: 1.0,
                    },
                }),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The floor: a huge lit rectangle far beyond the
                // window's edges, so the pool and every brick's shadow
                // stretch over it however the player stands. A receiver
                // only — flagging it would shadow the whole room behind
                // its own silhouette. (`Canvas::rectangle` has no lit
                // flag; only scene nodes receive light.)
                order: -1.0,
                lit: true,
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, 0.0],
                    extent: [2000.0, 2000.0],
                    color: LIT,
                }),
                ..Default::default()
            }),
        ],
        ..Default::default()
    });

    // A low ambient floor: at the default gray the room would read
    // fully lit and the glow — and the shadows the bricks cut out of
    // it — would reveal nothing.
    scene.ambient = frost::Color {
        r: 0.05,
        g: 0.05,
        b: 0.08,
        a: 1.0,
    };

    if let Err(err) = frost::run(
        scene,
        Demo {
            pos: [0.0, 0.0],
            vel: [0.0, 0.0],
            bricks: Vec::new(),
            brick,
            baked,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
