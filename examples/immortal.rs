//! A watering can as a mouse cursor over a full-screen grass field.
//! The window opens at `WINDOW` pixels (1920x1080), set through
//! `run_configured` and `Config::window_size`; `assets/sprites/grass.png`
//! is exactly that size, so it fills the window without being stretched,
//! and is re-stretched every frame to keep it covered if the window is
//! resized. `assets/sprites/water_can.png` follows the pointer: the
//! sprite is cropped to the can, so the demo scales it to `CAN_SIZE`
//! pixels wide and its node — the texture's center — lands on the cursor.
//! The cursor position comes from [`frost::Context::mouse_position`].
//! Holding the left mouse button down turns the can a quarter turn
//! counter-clockwise around the pointer over `ROTATE_TIME` seconds;
//! at the full tilt, water pours out of the spout as blue drops, and
//! releasing turns the can back the same way (both through
//! [`frost::Context::mouse_button_down`]). Run with:
//!
//! ```text
//! cargo run --example immortal
//! ```

/// The window's inner size in logical pixels, via `Config::window_size`.
/// The grass photo is exactly this size, so it fills the window 1:1.
const WINDOW: [u32; 2] = [1920, 1080];

/// `grass.png`'s texture size in pixels: a full-bleed 1920x1080 photo.
const GRASS_SIZE: [f32; 2] = [1920.0, 1080.0];

/// `water_can.png`'s texture size in pixels: the can's content, cropped
/// to the image.
const CAN_IMAGE: [f32; 2] = [331.0, 247.0];

/// The can's visible content in the image's own pixel space: `(0, 0)` is
/// the upper-left corner, `x` grows to the right, `y` grows down. The
/// crop fills the image, so the content box is the whole texture.
const CAN_BOX: [[f32; 2]; 2] = [
    [0.0, 0.0], // content upper-left
    [331.0, 247.0], // content lower-right
];

/// The rendered can's width in pixels; its height follows the content's
/// 334:251 aspect ratio.
const CAN_SIZE: f32 = 100.0;

/// Scales the whole texture so the can's content is `CAN_SIZE` wide.
const CAN_SCALE: f32 = CAN_SIZE / (CAN_BOX[1][0] - CAN_BOX[0][0]);

/// The can's rotated pose: a quarter turn counter-clockwise, applied while
/// the left mouse button is held down.
const CAN_ANGLE: f32 = std::f32::consts::FRAC_PI_2;

/// Seconds for the can to travel between its two poses, on press and on
/// release alike.
const ROTATE_TIME: f32 = 0.5;

/// The can's content center in node-local space: the sprite is centered on
/// its node's origin and the scene's y axis points up, so the content
/// center's image pixels `(cx, cy)` convert to `(cx - w/2, h/2 - cy)` —
/// the y flip included.
const CAN_LOCAL: [f32; 2] = {
    let cx = (CAN_BOX[0][0] + CAN_BOX[1][0]) / 2.0;
    let cy = (CAN_BOX[0][1] + CAN_BOX[1][1]) / 2.0;
    [cx - CAN_IMAGE[0] / 2.0, CAN_IMAGE[1] / 2.0 - cy]
};

/// The spout tip in `water_can.png`'s pixel space: `(0, 0)` is the
/// upper-left corner, `x` grows right, `y` grows down.
const SPOUT: [f32; 2] = [0.0, 25.0];

/// The spout tip in node-local space, with the same y flip as
/// `CAN_LOCAL`; the water is emitted from here, rotated with the can.
const SPOUT_LOCAL: [f32; 2] =
    [SPOUT[0] - CAN_IMAGE[0] / 2.0, CAN_IMAGE[1] / 2.0 - SPOUT[1]];

/// The node transform that puts the can's content center exactly on
/// `(mx, my)` and rotates the can by `angle` radians around that center.
///
/// The content center sits `CAN_LOCAL` (scaled by `CAN_SCALE`) from the
/// node's origin in node space, so the transform shifts the center to the
/// origin, rotates, then shifts back to the pointer. At `angle = 0` the
/// rotation is the identity, so this is exactly the unrotated
/// pointer-follow position.
fn can_transform(mx: f32, my: f32, angle: f32) -> frost::Transform {
    frost::Transform::translate(-CAN_LOCAL[0] * CAN_SCALE, -CAN_LOCAL[1] * CAN_SCALE)
        .compose(&frost::Transform::rotate(angle))
        .compose(&frost::Transform::translate(mx, my))
}

/// The drops' emission rate, in drops per second, while the can is
/// fully tilted.
const RATE: f32 = 120.0;

/// The drops' launch speed range, in pixels per second, along the spout
/// direction.
const SPEED: (f32, f32) = (140.0, 240.0);

/// The half-width of the launch cone, in radians around the spout
/// direction.
const SPREAD: f32 = 0.12;

/// The drops' lifetime range, in seconds.
const LIFE: (f32, f32) = (0.5, 1.2);

/// The drops' radius range, in pixels.
const SIZE: (f32, f32) = (1.5, 3.5);

/// The gravity, in pixels per second, per second, pulling the drops
/// down (y points up, so it is negative).
const GRAVITY: f32 = 420.0;

/// The z the drops are drawn at, above the can.
const Z: f32 = 2.0;

/// The drop color; the alpha is set per drop from its remaining life.
const DROP: frost::Color = frost::Color {
    r: 0.35,
    g: 0.62,
    b: 1.0,
    a: 1.0,
};

/// The can is treated as fully tilted — and pouring — once it is within
/// this many radians of `CAN_ANGLE`.
const TILT_EPS: f32 = 0.05;

/// The spout tip's world (user) position and the direction the water
/// leaves it, for the can at `(mx, my)` rotated `angle` radians.
///
/// The tip sits `SPOUT_LOCAL` (scaled by `CAN_SCALE`) from the content
/// center in node space; rotating that offset by `angle` — the same
/// rotation the can itself undergoes, about the pointer — gives its
/// world offset. The water leaves along the rotated offset's direction,
/// i.e. straight out of the spout: at the full tilt that is nearly
/// straight down.
fn spout(mx: f32, my: f32, angle: f32) -> ([f32; 2], [f32; 2]) {
    let (ex, ey) = (SPOUT_LOCAL[0] * CAN_SCALE, SPOUT_LOCAL[1] * CAN_SCALE);
    let (c, s) = (angle.cos(), angle.sin());
    let (ox, oy) = (c * ex - s * ey, s * ex + c * ey);
    let l = (ox * ox + oy * oy).sqrt();
    ([mx + ox, my + oy], [ox / l, oy / l])
}

/// A tiny deterministic random source (splitmix64), so the example needs
/// no external random crate: seeded from the current time, it gives a
/// different stream on each run.
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

    /// The next uniform value in [lo, hi].
    fn in_range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.next_f32() * (hi - lo)
    }
}

struct Demo {
    /// The cursor's last reported position; the can sticks here while the
    /// cursor is outside the window.
    mouse: [f32; 2],
    /// Whether the left mouse button is currently pressed.
    pressed: bool,
    /// The can's current rotation, in radians counter-clockwise.
    angle: f32,
    /// The rotation tween, restarted from `angle` on every press or
    /// release so a quick tap never jumps the can.
    rotation: frost::Tween<f32>,
    /// The drops pouring out of the spout: the simulation state, stepped
    /// once per frame.
    water: frost::ParticleSystem,
    /// The random source, for the per-drop jitter.
    rng: Rng,
    /// The emission accumulator: `RATE * dt` is added each frame and one
    /// drop is spawned per whole unit, so the rate holds at any dt.
    acc: f32,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // Stretch the grass to exactly fill the window, whatever its aspect
        // ratio, so it stays filled across resizes.
        let (w, h) = ctx.size();
        let grass = &mut ctx.scene().root.children[0];
        grass.scale = [w / GRASS_SIZE[0], h / GRASS_SIZE[1]];

        // Follow the pointer, keeping the last known position while the
        // cursor is outside the window.
        if let Some(pos) = ctx.mouse_position() {
            self.mouse = pos;
        }
        let [mx, my] = self.mouse;

        // Hold the left mouse button down to turn the can a quarter turn
        // counter-clockwise around the pointer; release to turn it back.
        // Each leg is a `ROTATE_TIME`-second tween restarted from wherever
        // the can currently is, so a mid-rotation press or release picks
        // up from the can's live angle.
        let pressed = ctx.mouse_button_down(frost::MouseButton::Left);
        if pressed != self.pressed {
            self.pressed = pressed;
            let target = if pressed { CAN_ANGLE } else { 0.0 };
            self.rotation =
                frost::Tween::new(self.angle, target, ROTATE_TIME).repeat(frost::Repeat::Once);
        }
        self.angle = self.rotation.tick(dt);

        let can = &mut ctx.scene().root.children[1];
        can.transform = can_transform(mx, my, self.angle);

        // Water pours out of the spout once the can is fully tilted: one
        // drop per whole unit of the accumulator, launched from the tip
        // (rotated with the can) along the spout direction.
        if self.angle >= CAN_ANGLE - TILT_EPS {
            let ([sx, sy], [dx, dy]) = spout(mx, my, self.angle);
            let base = dy.atan2(dx);
            self.acc += RATE * dt;
            while self.acc >= 1.0 {
                self.acc -= 1.0;
                let a = base + self.rng.in_range(-SPREAD, SPREAD);
                let life = self.rng.in_range(LIFE.0, LIFE.1);
                self.water.spawn(frost::Particle {
                    pos: [sx, sy],
                    vel: [
                        a.cos() * self.rng.in_range(SPEED.0, SPEED.1),
                        a.sin() * self.rng.in_range(SPEED.0, SPEED.1),
                    ],
                    life,
                    max_life: life,
                    size: self.rng.in_range(SIZE.0, SIZE.1),
                });
            }
        }

        // Advance the drops even while not pouring, so a released can's
        // stream keeps falling until it dies out.
        self.water.update(dt, [0.0, -GRAVITY]);

        // Draw each drop as a circle whose alpha is its remaining life
        // fraction, so the stream fades as it falls.
        for p in &self.water.particles {
            let fade = (p.life / p.max_life).clamp(0.0, 1.0);
            ctx.circle(
                p.pos[0],
                p.pos[1],
                p.size,
                frost::Color {
                    r: DROP.r,
                    g: DROP.g,
                    b: DROP.b,
                    a: fade,
                },
                Z,
            );
        }
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset paths to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let grass = frost::Shape::sprite(format!("{root}/assets/sprites/grass.png"))
        .expect("failed to load assets/sprites/grass.png");
    let can = frost::Shape::sprite(format!("{root}/assets/sprites/water_can.png"))
        .expect("failed to load assets/sprites/water_can.png");

    let scene = frost::Scene::new(frost::SceneNode {
        // Dark ground under the grass; only the sparse transparent gaps in
        // the grass texture show it.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.04,
                g: 0.1,
                b: 0.04,
                a: 1.0,
            },
        }),
        children: vec![
            Box::new(frost::SceneNode {
                // Stretched to fill the window by the process, every frame.
                shape: Some(grass),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // Starts at the window's center; the process moves it to
                // the pointer from the first frame on.
                scale: [CAN_SCALE, CAN_SCALE],
                shape: Some(can),
                ..Default::default()
            }),
        ],
        ..Default::default()
    });

    // The rotation tween starts as a 0→0 tween that never moves; the first
    // button event replaces it.
    if let Err(err) = frost::run_configured(
        scene,
        Demo {
            mouse: [0.0, 0.0],
            pressed: false,
            angle: 0.0,
            rotation: frost::Tween::new(0.0, 0.0, 1.0).repeat(frost::Repeat::Once),
            water: frost::ParticleSystem::new(),
            rng: Rng::new(),
            acc: 0.0,
        },
        frost::Config {
            window_size: Some(WINDOW),
            ..Default::default()
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
