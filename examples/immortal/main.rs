//! A watering can and a spray can as mouse cursors over a full-screen
//! grass field. The window opens at `WINDOW` pixels (1920x1080), set
//! through `run_configured` and `Config::window_size`;
//! `assets/sprites/grass.png` is exactly that size, so it fills the window
//! without being stretched, and is re-stretched every frame to keep it
//! covered if the window is resized.
//!
//! Pressing and releasing the right mouse button toggles between the two
//! tools (both through [`frost::Context::mouse_button_down`]):
//!
//! - The watering can, `assets/sprites/water_can_outline.png`, follows
//!   the pointer: the sprite is cropped to the can, so the demo scales it
//!   `CAN_SIZE` pixels wide and its node — the texture's center — lands on
//!   the cursor. Holding the left mouse button down turns the can a
//!   quarter turn counter-clockwise around the pointer over
//!   `ROTATE_TIME` seconds; at the full tilt, water pours out of the spout
//!   as blue drops, and releasing turns the can back the same way.
//!
//! - The spray can is drawn at its natural size, with the pivot
//!   (`SPRAY_PIVOT`, in image pixels) on the cursor and the at-rest frame
//!   `assets/sprites/Spray1.png`. A fresh press of the left mouse button —
//!   one after a release; holding or re-pressing mid-burst does nothing —
//!   triggers a burst: over `BURST_TIME` seconds the can tweens a 45
//!   degree clockwise turn and back while showing the pressed frame
//!   (`assets/sprites/Spray2.png`), and during the first half — the
//!   clockwise tilting — it emits a green spray from the nozzle
//!   (`SPRAY_NOZZLE`, in image pixels).
//!
//! `assets/sprites/items.png` is the inventory panel itself. It is scaled
//! uniformly to fit the window's height, with a 20 pixel clearance to the
//! top and bottom borders, and sits 20 pixels clear of the left border,
//! centered vertically — mid left. Its space is split into four slots, 0 to
//! 3 from the top, evenly along the y axis inside a 60 pixel margin at the
//! top and bottom. The watering can rests in slot 3 and the spray can in
//! slot 2, drawn on top of the panel; a basket in slot 0 and a second
//! watering can in slot 1 sit ghosted at 20 percent alpha, via their nodes'
//! modulate.
//!
//! On the grass, six tomato plants grow slice by slice: the same slice-
//! chain construction and travelling wind as the `grow` example, reused
//! through the [`plant`] module, but with five slices — the base grows over
//! 3 seconds, the low middle over 6, the middle over 9, the high middle
//! over 12, the top over 15. They grow one at a time, in `PLANT_POS` order —
//! the first starts at launch, and each next one starts when the previous
//! is fully grown — concurrently with the tool system. Each root joint
//! stays glued to its own `grass.png` pixel across resizes.
//!
//! The cursor position comes from [`frost::Context::mouse_position`]. Run
//! with:
//!
//! ```text
//! cargo run --example immortal
//! ```

mod plant;

/// The window's inner size in logical pixels, via `Config::window_size`.
/// The grass photo is exactly this size, so it fills the window 1:1.
const WINDOW: [u32; 2] = [1920, 1080];

/// `grass.png`'s texture size in pixels: a full-bleed 1920x1080 photo.
const GRASS_SIZE: [f32; 2] = [1920.0, 1080.0];

/// The plants' root joints in `grass.png`'s pixel space — `(0, 0)` at the
/// upper-left, `x` right, `y` down — in growth order: the first grows from
/// launch, and each next one starts when the previous is fully grown.
const PLANT_POS: [[f32; 2]; 6] = [
    [923.0, 514.0],
    [1248.0, 546.0],
    [1633.0, 603.0],
    [739.0, 571.0],
    [1081.0, 640.0],
    [1463.0, 719.0],
];

/// The plant's fit scale: the full plant spans about 1797 px around the
/// root joint — its top edge 1614 px above it, its bottom edge 183 px
/// below — so at this scale the tops can reach past the window's top edge
/// on the plants anchored high on the grass.
const PLANT_SCALE: f32 = 0.45;

/// `water_can_outline.png`'s texture size in pixels: the can's content,
/// cropped to the image.
const CAN_IMAGE: [f32; 2] = [333.0, 251.0];

/// `basket.png`'s texture size in pixels.
const BASKET_IMAGE: [f32; 2] = [271.0, 251.0];

/// The can's visible content in the image's own pixel space: `(0, 0)` is
/// the upper-left corner, `x` grows to the right, `y` grows down. The
/// crop fills the image, so the content box is the whole texture.
const CAN_BOX: [[f32; 2]; 2] = [
    [0.0, 0.0],     // content upper-left
    [333.0, 251.0], // content lower-right
];

/// The rendered can's width in pixels; its height follows the content's
/// 333:251 aspect ratio.
const CAN_SIZE: f32 = 200.0;

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

/// The spout tip in `water_can_outline.png`'s pixel space: `(0, 0)` is
/// the upper-left corner, `x` grows right, `y` grows down.
const SPOUT: [f32; 2] = [0.0, 25.0];

/// The spout tip in node-local space, with the same y flip as
/// `CAN_LOCAL`; the water is emitted from here, rotated with the can.
const SPOUT_LOCAL: [f32; 2] = [SPOUT[0] - CAN_IMAGE[0] / 2.0, CAN_IMAGE[1] / 2.0 - SPOUT[1]];

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

/// `items.png`'s texture size in pixels: the cropped panel itself.
const ITEMS_SIZE: [f32; 2] = [389.0, 991.0];

/// The clearance the panel keeps from the window's top, bottom, and left
/// borders, in pixels.
const MARGIN: f32 = 20.0;

/// The panel's logical slots, 0 (top) to 3 (bottom), evenly distributed
/// over the slot strip's height.
const SLOTS: usize = 4;

/// The margin the slot strip keeps from the panel's top and bottom edges,
/// in pixels.
const SLOT_MARGIN: f32 = 60.0;

/// The slot the resting watering can sits in.
const CAN_SLOT: usize = 3;

/// The slot the resting spray can sits in.
const SPRAY_SLOT: usize = 2;

/// The slot the ghosted basket sits in.
const BASKET_SLOT: usize = 0;

/// The slot the ghosted second watering can sits in.
const CAN2_SLOT: usize = 1;

/// The alpha the ghosted slot 0 and slot 1 items carry, applied as their
/// nodes' modulate.
const GHOST_ALPHA: f32 = 0.2;

/// The padding each resting can keeps from its slot's edges, in pixels.
const SLOT_INSET: f32 = 12.0;

/// The center of slot `i` (0 = top) in the items node's local space: the
/// slot's image-pixel center inside the margin-inset strip, y-flipped
/// about the panel's center, so a child translated here lands on the
/// slot's center.
fn slot_local(i: usize) -> [f32; 2] {
    let h = ITEMS_SIZE[1] - 2.0 * SLOT_MARGIN;
    let py = SLOT_MARGIN + (i as f32 + 0.5) * (h / SLOTS as f32);
    [0.0, ITEMS_SIZE[1] / 2.0 - py]
}

/// The uniform scale that fits a `size`-pixel sprite into one slot,
/// keeping `SLOT_INSET` clear of the slot's edges.
fn slot_scale(size: [f32; 2]) -> f32 {
    let w = ITEMS_SIZE[0] - 2.0 * SLOT_INSET;
    let h = (ITEMS_SIZE[1] - 2.0 * SLOT_MARGIN) / SLOTS as f32 - 2.0 * SLOT_INSET;
    (w / size[0]).min(h / size[1])
}

/// `Spray1.png` and `Spray2.png`'s texture size in pixels: both frames
/// are the same size and are drawn at their natural size, unscaled.
const SPRAY_IMAGE: [f32; 2] = [136.0, 276.0];

/// The spray can's pivot in the `Spray{1,2}.png` pixel space: the point
/// that lands on the cursor, and the point about which the can rotates
/// during a burst. `(0, 0)` is the upper-left corner, `x` grows right,
/// `y` grows down.
const SPRAY_PIVOT: [f32; 2] = [98.0, 60.0];

/// The pivot in node-local space, with the same y flip as
/// `SPOUT_LOCAL`; the spray can's node transform keeps it on the cursor.
const SPRAY_PIVOT_LOCAL: [f32; 2] = [
    SPRAY_PIVOT[0] - SPRAY_IMAGE[0] / 2.0,
    SPRAY_IMAGE[1] / 2.0 - SPRAY_PIVOT[1],
];

/// The nozzle tip in the `Spray{1,2}.png` pixel space: the point where
/// the green spray is emitted, rotated with the can.
const SPRAY_NOZZLE: [f32; 2] = [21.0, 13.0];

/// The nozzle tip in node-local space, with the same y flip as
/// `SPRAY_PIVOT_LOCAL`.
const SPRAY_NOZZLE_LOCAL: [f32; 2] = [
    SPRAY_NOZZLE[0] - SPRAY_IMAGE[0] / 2.0,
    SPRAY_IMAGE[1] / 2.0 - SPRAY_NOZZLE[1],
];

/// The burst's tilt, in radians: 45 degrees clockwise, negative because
/// the scene's y axis points up and counter-clockwise is positive.
const BURST_ANGLE: f32 = -std::f32::consts::FRAC_PI_4;

/// Seconds for a whole burst: `BURST_HALF` of clockwise tilting and
/// `BURST_HALF` of the return.
const BURST_TIME: f32 = 0.25;
const BURST_HALF: f32 = BURST_TIME / 2.0;

/// The spray's emission rate, in particles per second, while the burst is
/// tilting.
const SPRAY_RATE: f32 = 300.0;

/// The spray's launch speed range, in pixels per second, along the nozzle
/// direction.
const SPRAY_SPEED: (f32, f32) = (160.0, 380.0);

/// The half-width of the launch cone, in radians around the nozzle
/// direction.
const SPRAY_SPREAD: f32 = 0.4;

/// The spray particles' lifetime range, in seconds.
const SPRAY_LIFE: (f32, f32) = (0.35, 0.8);

/// The spray particles' radius range, in pixels.
const SPRAY_SIZE: (f32, f32) = (1.0, 2.5);

/// The gravity, in pixels per second, per second, pulling the spray down:
/// lighter than `GRAVITY`, because mist falls slower than drops.
const SPRAY_GRAVITY: f32 = 120.0;

/// The spray's color; the alpha is set per particle from its remaining
/// life.
const SPRAY: frost::Color = frost::Color {
    r: 0.45,
    g: 0.9,
    b: 0.4,
    a: 1.0,
};

/// The node transform that puts the spray can's pivot exactly on `(mx, my)`
/// and rotates the can by `angle` radians around that pivot.
///
/// The pivot sits `SPRAY_PIVOT_LOCAL` from the node's origin in node
/// space, so the transform shifts the pivot to the origin, rotates, then
/// shifts back to the pointer. At `angle = 0` the rotation is the
/// identity, so this is exactly the unrotated pointer-follow position.
fn spray_transform(mx: f32, my: f32, angle: f32) -> frost::Transform {
    frost::Transform::translate(-SPRAY_PIVOT_LOCAL[0], -SPRAY_PIVOT_LOCAL[1])
        .compose(&frost::Transform::rotate(angle))
        .compose(&frost::Transform::translate(mx, my))
}

/// The nozzle tip's world (user) position and the direction the spray
/// leaves it, for the can at `(mx, my)` rotated `angle` radians.
///
/// The tip sits `SPRAY_NOZZLE_LOCAL - SPRAY_PIVOT_LOCAL` from the pivot in
/// node space; rotating that offset by `angle` — the same rotation the can
/// itself undergoes, about the pivot, which sits on the cursor — gives its
/// world offset. The spray leaves along the rotated offset's direction,
/// i.e. straight out of the nozzle: at the full tilt that is nearly
/// straight up.
fn nozzle(mx: f32, my: f32, angle: f32) -> ([f32; 2], [f32; 2]) {
    let (ex, ey) = (
        SPRAY_NOZZLE_LOCAL[0] - SPRAY_PIVOT_LOCAL[0],
        SPRAY_NOZZLE_LOCAL[1] - SPRAY_PIVOT_LOCAL[1],
    );
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

/// The two cursor tools, toggled by pressing and releasing the right
/// mouse button.
#[derive(Clone, Copy, PartialEq)]
enum Tool {
    /// The watering can: hold the left button to tilt it and pour water.
    WaterCan,
    /// The spray can: a fresh left-button press triggers a green burst.
    SprayCan,
}

struct Demo {
    /// The cursor's last reported position; the tool sticks here while the
    /// cursor is outside the window.
    mouse: [f32; 2],
    /// The active tool.
    tool: Tool,
    /// Whether the left mouse button was down on the previous frame; the
    /// press and release edges are derived from it.
    pressed: bool,
    /// Whether the right mouse button was down on the previous frame; the
    /// tool toggles on its release.
    right_pressed: bool,
    /// The active can's current rotation, in radians counter-clockwise:
    /// the watering can's tilt or the spray can's burst angle.
    angle: f32,
    /// The rotation tween, restarted from `angle` on every press, release,
    /// or burst so a quick tap never jumps the can.
    rotation: frost::Tween<f32>,
    /// Elapsed seconds of the current spray burst; `None` while the spray
    /// can stands upright.
    burst: Option<f32>,
    /// Whether the tool node currently shows `spray2`, the pressed frame;
    /// it flips at the start and end of each burst.
    showing_spray2: bool,
    /// The watering can's shape, restored to the tool node when the
    /// toggle switches back to it.
    can: frost::Shape,
    /// The two spray frames, loaded once: `spray1` is the at-rest frame
    /// and `spray2` the pressed frame; swapping the node's `shape` between
    /// them is a cheap `Arc` clone of the pixel buffer.
    spray1: frost::Shape,
    spray2: frost::Shape,
    /// The drops pouring out of the spout: the simulation state, stepped
    /// once per frame.
    water: frost::ParticleSystem,
    /// The green spray emitted by the spray can: the simulation state,
    /// stepped once per frame.
    spray: frost::ParticleSystem,
    /// The random source, for the per-particle jitter.
    rng: Rng,
    /// The emission accumulator: `RATE * dt` (or `SPRAY_RATE * dt`) is
    /// added each frame and one particle is spawned per whole unit, so the
    /// rate holds at any dt.
    acc: f32,
    /// The tomato plants on the grass, in growth order: each has its own
    /// growth clock, and the process steps it only once the previous one
    /// is fully grown — the first from launch on — then lays it out on the
    /// matching child of the plants node (root children[2]), in parallel
    /// with the tool system.
    plants: [plant::Plant; PLANT_POS.len()],
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // Stretch the grass to exactly fill the window, whatever its aspect
        // ratio, so it stays filled across resizes.
        let (w, h) = ctx.size();
        let grass = &mut ctx.scene().root.children[0];
        grass.scale = [w / GRASS_SIZE[0], h / GRASS_SIZE[1]];
        // Fit the panel into the window's height with `MARGIN` clear of the
        // top and bottom borders; the x scale follows, keeping the aspect
        // ratio. The cans, riding on its slots as the node's children, stay
        // in place.
        let items = &mut ctx.scene().root.children[1];
        let s = (h - 2.0 * MARGIN) / ITEMS_SIZE[1];
        items.scale = [s, s];
        // Mid left: `MARGIN` clear of the left border, centered vertically.
        items.transform =
            frost::Transform::translate(-(w / 2.0) + MARGIN + ITEMS_SIZE[0] * s / 2.0, 0.0);

        // Grow the plants one at a time, in parallel with the tool
        // system: the first starts at launch, each next one starts when
        // the previous is fully grown. Each root joint stays glued to its
        // `PLANT_POS` pixel on the grass, which the grass stretch maps to
        // the matching user-space anchor.
        let plants_node = &mut ctx.scene().root.children[2];
        for i in 0..PLANT_POS.len() {
            if i == 0 || self.plants[i - 1].fully_grown() {
                self.plants[i].step(dt);
            }
            let anchor = [
                PLANT_POS[i][0] * w / GRASS_SIZE[0] - w / 2.0,
                h / 2.0 - PLANT_POS[i][1] * h / GRASS_SIZE[1],
            ];
            self.plants[i].layout(&mut plants_node.children[i], anchor);
        }

        // Follow the pointer, keeping the last known position while the
        // cursor is outside the window.
        if let Some(pos) = ctx.mouse_position() {
            self.mouse = pos;
        }
        let [mx, my] = self.mouse;

        let left = ctx.mouse_button_down(frost::MouseButton::Left);
        let right = ctx.mouse_button_down(frost::MouseButton::Right);

        // A press followed by a release of the right mouse button toggles
        // between the tools. Switching resets the can to its upright,
        // at-rest pose and puts the new tool's shape and scale on the node,
        // so neither tool inherits the other's tilt, burst, or sprite.
        if self.right_pressed && !right {
            let to_spray = self.tool == Tool::WaterCan;
            self.tool = if to_spray {
                Tool::SprayCan
            } else {
                Tool::WaterCan
            };
            self.angle = 0.0;
            self.burst = None;
            self.rotation = frost::Tween::new(0.0, 0.0, 1.0).repeat(frost::Repeat::Once);
            let tool_node = &mut ctx.scene().root.children[3];
            if to_spray {
                self.showing_spray2 = false;
                tool_node.shape = Some(self.spray1.clone());
                tool_node.scale = [1.0, 1.0];
            } else {
                tool_node.shape = Some(self.can.clone());
                tool_node.scale = [CAN_SCALE, CAN_SCALE];
            }
        }
        self.right_pressed = right;

        match self.tool {
            // Hold the left mouse button down to turn the can a quarter
            // turn counter-clockwise around the pointer; release to turn it
            // back. Each leg is a `ROTATE_TIME`-second tween restarted from
            // wherever the can currently is, so a mid-rotation press or
            // release picks up from the can's live angle.
            Tool::WaterCan => {
                if left != self.pressed {
                    self.pressed = left;
                    let target = if left { CAN_ANGLE } else { 0.0 };
                    self.rotation = frost::Tween::new(self.angle, target, ROTATE_TIME)
                        .repeat(frost::Repeat::Once);
                }
                self.angle = self.rotation.tick(dt);

                // Water pours out of the spout once the can is fully
                // tilted: one drop per whole unit of the accumulator,
                // launched from the tip (rotated with the can) along the
                // spout direction.
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
            }
            // A fresh press of the left button — one after a release, not a
            // hold or a re-press mid-burst — triggers a burst while the can
            // stands upright.
            Tool::SprayCan => {
                if left && !self.pressed && self.burst.is_none() {
                    self.pressed = left;
                    self.burst = Some(0.0);
                    // PingPong is the tween's default: `0 -> BURST_ANGLE`
                    // in `BURST_HALF` seconds and back in the same time, so
                    // the tilting and the return take `BURST_TIME` in all.
                    // The ticking below stops there, before the cycle could
                    // wrap.
                    self.rotation = frost::Tween::new(0.0, BURST_ANGLE, BURST_HALF);
                    self.set_spray_frame(ctx, true);
                } else {
                    self.pressed = left;
                }

                if let Some(t) = self.burst {
                    let nt = t + dt;
                    if nt >= BURST_TIME {
                        // The burst is over: the can stands upright again,
                        // showing the at-rest frame.
                        self.burst = None;
                        self.angle = 0.0;
                        self.set_spray_frame(ctx, false);
                    } else {
                        self.burst = Some(nt);
                        self.angle = self.rotation.tick(dt);
                        // Emission is limited to the first half — the
                        // clockwise tilting: one particle per whole unit of
                        // the accumulator, launched from the nozzle
                        // (rotated with the can) along the nozzle
                        // direction.
                        if nt < BURST_HALF {
                            let ([nx, ny], [dx, dy]) = nozzle(mx, my, self.angle);
                            let base = dy.atan2(dx);
                            self.acc += SPRAY_RATE * dt;
                            while self.acc >= 1.0 {
                                self.acc -= 1.0;
                                let a = base + self.rng.in_range(-SPRAY_SPREAD, SPRAY_SPREAD);
                                let life = self.rng.in_range(SPRAY_LIFE.0, SPRAY_LIFE.1);
                                self.spray.spawn(frost::Particle {
                                    pos: [nx, ny],
                                    vel: [
                                        a.cos() * self.rng.in_range(SPRAY_SPEED.0, SPRAY_SPEED.1),
                                        a.sin() * self.rng.in_range(SPRAY_SPEED.0, SPRAY_SPEED.1),
                                    ],
                                    life,
                                    max_life: life,
                                    size: self.rng.in_range(SPRAY_SIZE.0, SPRAY_SIZE.1),
                                });
                            }
                        }
                    }
                }
            }
        }

        let tool_node = &mut ctx.scene().root.children[3];
        tool_node.transform = match self.tool {
            Tool::WaterCan => can_transform(mx, my, self.angle),
            Tool::SprayCan => spray_transform(mx, my, self.angle),
        };

        // Advance the particles even while not emitting, so an ongoing
        // stream keeps falling until it dies out.
        self.water.update(dt, [0.0, -GRAVITY]);
        self.spray.update(dt, [0.0, -SPRAY_GRAVITY]);

        // Draw each particle as a circle whose alpha is its remaining life
        // fraction, so the streams fade as they fall.
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
        for p in &self.spray.particles {
            let fade = (p.life / p.max_life).clamp(0.0, 1.0);
            ctx.circle(
                p.pos[0],
                p.pos[1],
                p.size,
                frost::Color {
                    r: SPRAY.r,
                    g: SPRAY.g,
                    b: SPRAY.b,
                    a: fade,
                },
                Z,
            );
        }
    }
}

impl Demo {
    /// Shows the pressed spray frame (`spray2`) if `on` and the at-rest
    /// frame (`spray1`) otherwise, but only when the tool node currently
    /// shows the other one, so the swap — a cheap `Arc` clone — happens at
    /// most once per change.
    fn set_spray_frame(&mut self, ctx: &mut frost::Context, on: bool) {
        if self.showing_spray2 == on {
            return;
        }
        self.showing_spray2 = on;
        ctx.scene().root.children[3].shape = Some(if on {
            self.spray2.clone()
        } else {
            self.spray1.clone()
        });
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
    let can = frost::Shape::sprite(format!("{root}/assets/sprites/water_can_outline.png"))
        .expect("failed to load assets/sprites/water_can_outline.png");
    let spray1 = frost::Shape::sprite(format!("{root}/assets/sprites/Spray1.png"))
        .expect("failed to load assets/sprites/Spray1.png");
    let spray2 = frost::Shape::sprite(format!("{root}/assets/sprites/Spray2.png"))
        .expect("failed to load assets/sprites/Spray2.png");
    let items = frost::Shape::sprite(format!("{root}/assets/sprites/items.png"))
        .expect("failed to load assets/sprites/items.png");
    let basket = frost::Shape::sprite(format!("{root}/assets/sprites/basket.png"))
        .expect("failed to load assets/sprites/basket.png");
    let plant1 = frost::Shape::sprite(format!("{root}/assets/sprites/plant1.png"))
        .expect("failed to load assets/sprites/plant1.png");
    let plant2 = frost::Shape::sprite(format!("{root}/assets/sprites/plant2.png"))
        .expect("failed to load assets/sprites/plant2.png");
    let plant3 = frost::Shape::sprite(format!("{root}/assets/sprites/plant3.png"))
        .expect("failed to load assets/sprites/plant3.png");
    let plant4 = frost::Shape::sprite(format!("{root}/assets/sprites/plant4.png"))
        .expect("failed to load assets/sprites/plant4.png");
    let plant5 = frost::Shape::sprite(format!("{root}/assets/sprites/plant5.png"))
        .expect("failed to load assets/sprites/plant5.png");
    let plant = plant::Plant::new([&plant1, &plant2, &plant3, &plant4, &plant5]);

    // One plant node, cloned for each plant: its origin is the root joint
    // (plant1's lower joint), positioned by the process every frame. The
    // fit scale is constant — each slice grows individually, riding the
    // node. The clones are cheap `Arc` clones of the slice pixel buffers.
    let plant_node = frost::SceneNode {
        scale: [PLANT_SCALE, PLANT_SCALE],
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
            Box::new(frost::SceneNode {
                shape: Some(plant4),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                shape: Some(plant5),
                ..Default::default()
            }),
        ],
        ..Default::default()
    };

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
                // The inventory panel, positioned and scaled by the process
                // every frame: mid left, `MARGIN` clear of the top, bottom,
                // and left borders. Its four resting items are its children
                // — a node paints its shape before its children, so they
                // render on top of the panel — and they ride its fit on
                // resize. The slot 0 and slot 1 items carry `GHOST_ALPHA`
                // via their nodes' modulate.
                shape: Some(items),
                children: vec![
                    Box::new(frost::SceneNode {
                        // The basket at rest, centered in slot 0, ghosted.
                        transform: frost::Transform::translate(
                            slot_local(BASKET_SLOT)[0],
                            slot_local(BASKET_SLOT)[1],
                        ),
                        scale: [slot_scale(BASKET_IMAGE), slot_scale(BASKET_IMAGE)],
                        modulate: frost::Color {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: GHOST_ALPHA,
                        },
                        shape: Some(basket),
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        // The second watering can at rest, centered in slot
                        // 1, ghosted.
                        transform: frost::Transform::translate(
                            slot_local(CAN2_SLOT)[0],
                            slot_local(CAN2_SLOT)[1],
                        ),
                        scale: [slot_scale(CAN_IMAGE), slot_scale(CAN_IMAGE)],
                        modulate: frost::Color {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: GHOST_ALPHA,
                        },
                        shape: Some(can.clone()),
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        // The spray can at rest, centered in slot 2.
                        transform: frost::Transform::translate(
                            slot_local(SPRAY_SLOT)[0],
                            slot_local(SPRAY_SLOT)[1],
                        ),
                        scale: [slot_scale(SPRAY_IMAGE), slot_scale(SPRAY_IMAGE)],
                        shape: Some(spray1.clone()),
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        // The watering can at rest, centered in slot 3.
                        transform: frost::Transform::translate(
                            slot_local(CAN_SLOT)[0],
                            slot_local(CAN_SLOT)[1],
                        ),
                        scale: [slot_scale(CAN_IMAGE), slot_scale(CAN_IMAGE)],
                        shape: Some(can.clone()),
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The plants on the grass: one plant node per `PLANT_POS`
                // entry, in growth order. The group carries no shape or
                // scale of its own; each plant child holds the fit scale
                // and is positioned by the process every frame. The slices
                // paint on top of the grass; the group sits under the tool
                // node, so the cursor paints above the plants.
                children: (0..PLANT_POS.len())
                    .map(|_| Box::new(plant_node.clone()))
                    .collect(),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The active tool, starting as the watering can: scaled to
                // `CAN_SIZE` wide. The toggle switches the shape — and the
                // scale, since the spray frames are drawn at their natural
                // size — on this same node. It stays the last child, so the
                // cursor paints above the panel and the plant.
                // Starts at the window's center; the process moves it to
                // the pointer from the first frame on.
                scale: [CAN_SCALE, CAN_SCALE],
                shape: Some(can.clone()),
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
            tool: Tool::WaterCan,
            pressed: false,
            right_pressed: false,
            angle: 0.0,
            rotation: frost::Tween::new(0.0, 0.0, 1.0).repeat(frost::Repeat::Once),
            burst: None,
            showing_spray2: false,
            can,
            spray1,
            spray2,
            water: frost::ParticleSystem::new(),
            spray: frost::ParticleSystem::new(),
            rng: Rng::new(),
            acc: 0.0,
            // Six identical growth clocks, one per plant: each starts at
            // zero and the process steps it when the previous is fully
            // grown.
            plants: std::array::from_fn(|_| plant.clone()),
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
