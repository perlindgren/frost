//! A watering can and a spray can as mouse cursors over a full-screen
//! grass field. The window opens at `WINDOW` pixels (1920x1080), set
//! through `run_configured` and `Config::window_size`;
//! `assets/sprites/grass.png` is exactly that size, so it fills the window
//! without being stretched, and is re-stretched every frame to keep it
//! covered if the window is resized.
//!
//! The mouse can hold two tools at once: the active one, drawn as the
//! cursor's sprite, and a stored one, mirrored in the held-items panel. It
//! starts with neither. Pressing and releasing the right mouse button (both through
//! [`frost::Context::mouse_button_down`]) switches the two: the stored
//! tool becomes the active one and the active one is stored. A left click
//! on a slot — a press and a release on the same slot — swaps the active
//! tool with the slot's: the tool in a populated slot moves to the mouse,
//! and the active tool moves into an empty slot, in which case the mouse
//! shows no sprite, still holding the stored tool, if any.
//!
//! While the active tool is held, the left button uses it on the grass —
//! a click over a slot instead swaps it with the slot's tool:
//!
//! - The watering can, `assets/sprites/water_can_outline.png`, follows
//!   the pointer: the sprite is cropped to the can, so the demo scales it
//!   `CAN_SIZE` pixels wide and its node — the texture's center — lands on
//!   the cursor. Holding the left mouse button down turns the can a
//!   quarter turn counter-clockwise around the pointer over
//!   `ROTATE_TIME` seconds; at the full tilt, water pours out of the spout
//!   as blue drops, and releasing turns the can back the same way. While
//!   the water actually pours — the can active and fully tilted, the same
//!   condition the drops spawn on — the audio output loops
//!   `assets/audio/WaterFlowSoft.wav`, silenced the frame pouring stops,
//!   the can turning back upright or the tool switching away from it.
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
//! top and bottom. The spray can rests in slot 2 and the watering can in
//! slot 3, drawn on top of the panel; slots 0 and 1 start empty, and a
//! left click can park either tool in any slot.
//!
//! `assets/sprites/held_items.png` is a small panel in the bottom right
//! corner, `MARGIN` pixels clear of the window's right and bottom borders,
//! drawn at its natural size. It mirrors the mouse's two tools: the active
//! one in the left half, the stored one in the right half, each sprite
//! scaled to fit its half — so the tools are always visible, even while
//! the active one is drawn as the cursor and the stored one would
//! otherwise be drawn nowhere.
//!
//! On the grass, six tomato plants grow slice by slice: the same slice-
//! chain construction and travelling wind as the `grow` example, reused
//! through the [`plant`] module, but with five slices — the base grows over
//! 3 seconds, the low middle over 6, the middle over 9, the high middle
//! over 12, the top over 15. They grow one at a time, in `PLANT_POS` order —
//! the first starts at launch, and each next one starts when the previous
//! is fully grown — concurrently with the tool system. Each root joint
//! stays glued to its own `grass.png` pixel across resizes. Once a slice is
//! fully grown, its blooms grow: at each of the four lower slices'
//! hand-picked spawn points — the top slice bears none — a flower grows
//! from zero to full size over 10 seconds, its `assets/sprites/flower.png`
//! tinted light green to yellow; once the flower is fully grown, a tomato
//! grows out of the same point over 10 seconds — the white body of
//! `assets/sprites/tomato.png` tinted dark green to red, at one third of
//! its natural size, with the dark calyx and stem of
//! `assets/sprites/tomato_fg.png` drawn on top, the body's top pinned to the
//! flower's center so the fruit hangs below it, on top of the flower.
//! Each bloom rides its slice's transform so it sways with the plant.
//!
//! Every plant keeps a water reserve, full at launch, and the plants'
//! growth runs at `1 / GROW_SLOWDOWN` of real time's pace — the slices,
//! the flowers, and the tomatoes alike: while a plant is growing —
//! started, not yet complete, its slices or its blooms still growing —
//! its growth clock is stepped by `dt / GROW_SLOWDOWN`, and its reserve
//! drains over `DRAIN_TIME` of that slowed clock —
//! `GROW_SLOWDOWN * DRAIN_TIME` real seconds — and growth proceeds only
//! while the reserve holds; a plant whose reserve runs dry stops growing,
//! its growth clock freezing, until the player pours water on its root.
//! `DRAIN_TIME` is chosen so the first plant, fully watered at launch,
//! runs dry just before its base slice is fully grown — the base grows
//! over 3 growth-clock seconds, the shortest of the five — so the bench
//! pauses before its flowers can start developing. A drop that passes
//! through a growing plant's rough hitbox — a `ROOT_RADIUS`-pixel circle
//! around the root joint — restores the reserve by `DROP_WATER`; a full
//! second of pouring, if every drop lands, is one full reserve, whatever
//! pace the growth runs at. A blue fill on a dark background, `BAR_DX`
//! wide, floats `BAR_LIFT` pixels above the root joint of every plant
//! that has started growing and is not complete yet, showing its reserve.
//!
//! A swarm of thirty vipers buzzes around the flower bench — the row of
//! plants — concurrently with everything else, through the [`vipers`]
//! module: one viper per plant layer, and the swarm is not all in the air
//! at once — each fully grown layer spawns its own viper, so the swarm
//! grows five by five as the bench does, with every viper orbiting the
//! segment its layer spawned it, at that segment's midpoint along the
//! static, fully grown chain — the sway left out. Each viper flies at its
//! own radius, speed, and height above the midpoint, and the two frames,
//! `assets/sprites/Getingeye1.png` and `assets/sprites/Getingeye2.png`,
//! alternate at its own wingbeat rate — the sprite is a viper flying to
//! the right, flipped about its center while it flies to the left.
//!
//! A swarm of bugs waddles across the grass in step with the plants —
//! three new bugs every time a plant starts growing, so the population
//! climbs 3, 6, …, 18 over the first 75 seconds — through the [`bugs`]
//! module: each bug pops up out of the ground at a random point inside
//! the convex hull of the six plant roots, standing up over 3 seconds
//! via its node's y scale, then swarm-walks — a straight line with a
//! sinusoidal perpendicular wobble — to a park spot around the plant it
//! was born for, where it sways in place. Each bug cycles the three
//! frames `assets/sprites/Bug1a.png`, `Bug2a.png`, and `Bug3a.png` while
//! walking, flipped about its center while it moves left, tinted dark
//! red by the node's `modulate`.
//!
//! A bug starts with three hits of health, loses one per mist hit — a
//! wounded bug takes its next hit only after a 0.2 s cooldown, so the
//! mist wears it down one hit at a time — and, once it has reached its
//! park spot, recovers one hit every 5 seconds, up to six; a row of
//! white pips above the bug counts the hits it can still take. The
//! killing blow
//! starts the two-phase death: over 0.5 seconds the bug bounces up off
//! the grass and flips upside down in the air — its node's y scale
//! sweeps from upright to fully inverted about the sprite center while
//! its position, growth, and walk frame freeze — landing on its back,
//! and then, over 0.5 seconds, the inverted sprite evaporates, shrinking
//! to nothing while its center sinks through the grass.
//! The dead bug then waits out a random 5 to 10 second delay and pops
//! back up at its spawn spot, fully healed, so the population dips and
//! recovers with the spraying.
//!
//! A bug that reaches its destination while another bug is on the grass
//! within 50 px of it chatters: the swarm picks one of the three tjatter
//! clips — `assets/audio/TjatterLow.wav`, `TjatterMid.wav`, and
//! `TjatterHigh.wav` — at random, and the demo plays it through
//! [`frost::Audio`].
//!
//! Every bug that pops up out of the grass — a batch spawn or a respawn
//! — plops: the swarm picks one of the three plopp clips —
//! `assets/audio/bugs_plopp1.wav`, `bugs_plopp2.wav`, and
//! `bugs_plopp3.wav` — at random, and the demo plays it through
//! [`frost::Audio`].
//!
//! Every time the mist drops a bug's health, the bug cries out: the
//! swarm picks one of the four Aj clips — `assets/audio/Aj1.wav`,
//! `Aj2.wav`, `Aj3.wav`, and `Aj4.wav` — at random, and the demo plays
//! it through [`frost::Audio`].
//!
//! The cursor position comes from [`frost::Context::mouse_position`]. Run
//! with:
//!
//! ```text
//! cargo run --example immortal
//! ```

mod bugs;
mod plant;
mod vipers;

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

/// The bug swarm's population: three bugs per plant, so the scene carries
/// one shape-less slot per bug and the swarm fills them as the plants
/// start growing.
const BUG_N: usize = bugs::BUGS_PER_PLANT * PLANT_POS.len();

/// The plant's fit scale: the full plant spans about 1797 px around the
/// root joint — its top edge 1614 px above it, its bottom edge 183 px
/// below — so at this scale the tops can reach past the window's top edge
/// on the plants anchored high on the grass.
const PLANT_SCALE: f32 = 0.45;

/// `water_can_outline.png`'s texture size in pixels: the can's content,
/// cropped to the image.
const CAN_IMAGE: [f32; 2] = [333.0, 251.0];

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

/// The bugs' health pips: one white, 4 px across, per hit a bug can
/// still take, in a row 6 px above it.
const PIP: frost::Color = frost::Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};

/// The factor by which the plants' growth runs slow: the process steps a
/// plant's growth clock by `dt / GROW_SLOWDOWN` per frame, so a slice
/// that grows over its `plant::GROW_TIMES` span takes `GROW_SLOWDOWN`
/// times that span in real time — the slices, the flowers, and the
/// tomatoes alike — and the water reserve drains with the growth, over
/// `GROW_SLOWDOWN * DRAIN_TIME` real seconds.
const GROW_SLOWDOWN: f32 = 3.0;

/// The time a plant's water reserve takes to drain from full (1.0) to dry
/// (0.0), in growth-clock seconds, while the plant is growing: the
/// process drains a started plant that is not yet complete by
/// `dt / (DRAIN_TIME * GROW_SLOWDOWN)` per frame — the drain runs at the
/// growth's slowed pace — and the span is chosen so the first plant —
/// fully watered at launch — runs dry just before its base slice is fully
/// grown: the base grows over 3 growth-clock seconds (`plant::GROW_TIMES[0]`),
/// i.e. 9 real seconds at `GROW_SLOWDOWN`, and `DRAIN_TIME * GROW_SLOWDOWN`
/// is 8.25 real seconds, so the bench pauses 0.75 seconds before the base
/// completes, before its flowers can start developing.
const DRAIN_TIME: f32 = 2.75;

/// The radius of the rough hitbox around a plant's root joint, in user
/// space pixels: a falling drop inside this circle of the root's anchor
/// waters that plant.
const ROOT_RADIUS: f32 = 60.0;

/// The share of a plant's water reserve one drop in its root hitbox
/// restores: with the drops' `RATE`, a full second of pouring — every
/// drop landing — is one full reserve.
const DROP_WATER: f32 = 1.0 / RATE;

/// The height of the water bar's center above the plant's root joint, in
/// user space pixels: the bar floats just above the root — the spot the
/// player pours on — because at the fit scale a fully grown plant's top
/// reaches the window's top edge, leaving no room above the plant itself.
const BAR_LIFT: f32 = 70.0;

/// The water bar's half-extents, in user space pixels: the dark
/// background is `BAR_DX + BAR_BORDER` wide by `BAR_DY + BAR_BORDER` tall
/// and the blue fill is `BAR_DX` wide by `BAR_DY` tall, both centered on
/// the same point, the fill's width tracking the reserve from the left
/// edge.
const BAR_DX: f32 = 44.0;
const BAR_DY: f32 = 5.0;
const BAR_BORDER: f32 = 2.0;

/// The color of the water bar's background, the dry part of the bar.
const BAR_BG: frost::Color = frost::Color {
    r: 0.16,
    g: 0.18,
    b: 0.22,
    a: 0.9,
};

/// Whether a drop at `pos` is inside the `ROOT_RADIUS`-pixel hitbox
/// around the plant's root joint at `anchor`.
fn in_root_hitbox(pos: [f32; 2], anchor: [f32; 2]) -> bool {
    let dx = pos[0] - anchor[0];
    let dy = pos[1] - anchor[1];
    dx * dx + dy * dy <= ROOT_RADIUS * ROOT_RADIUS
}

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

/// `held_items.png`'s texture size in pixels; the held-items panel is
/// drawn at this natural size, `MARGIN` clear of the window's right and
/// bottom borders.
const HELD_SIZE: [f32; 2] = [389.0, 200.0];

/// `sustainble_immortality.png`'s texture size in pixels; the corner
/// badge is drawn at `IMMORTALITY_SCALE` of this natural size, `MARGIN`
/// clear of the window's top and right borders.
const IMMORTALITY_SIZE: [f32; 2] = [536.0, 548.0];

/// The badge's scale relative to its natural size: half of it.
const IMMORTALITY_SCALE: f32 = 0.5;

/// The badge's tint alpha: a faint watermark behind the play field.
const IMMORTALITY_ALPHA: f32 = 0.2;

/// The clearance a held cell keeps from the edges of its half of the
/// panel, in pixels: past the panel's frame, with room to spare.
const HELD_INSET: f32 = 20.0;

/// The center of held cell `i` — 0 the left cell, 1 the right cell — in
/// the held node's local space: the half-panel center, y-flipped about
/// the panel's center. Both cells sit on the panel's horizontal midline,
/// so the local y is 0.
fn held_local(i: usize) -> [f32; 2] {
    let x = HELD_SIZE[0] / 4.0;
    [if i == 0 { -x } else { x }, 0.0]
}

/// The uniform scale that fits a `size`-pixel sprite into one held cell —
/// the panel's half width by its full height, keeping `HELD_INSET` clear
/// of the cell's edges.
fn held_scale(size: [f32; 2]) -> f32 {
    let w = HELD_SIZE[0] / 2.0 - 2.0 * HELD_INSET;
    let h = HELD_SIZE[1] - 2.0 * HELD_INSET;
    (w / size[0]).min(h / size[1])
}

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

/// The padding each resting can keeps from its slot's edges, in pixels.
const SLOT_INSET: f32 = 30.0;

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

/// The texture size of a sprite shape, in pixels.
fn sprite_size(shape: &frost::Shape) -> [f32; 2] {
    match shape {
        frost::Shape::Sprite { width, height, .. } => [*width as f32, *height as f32],
        _ => [0.0, 0.0],
    }
}

/// `BasketBack.png` / `BasketFront.png`'s texture size in pixels: the two
/// aligned halves of the harvest basket.
const BASKET_IMAGE: [f32; 2] = [271.0, 251.0];

/// The clearance the basket keeps from the items panel's right edge, in
/// pixels.
const BASKET_GAP: f32 = 24.0;

/// The uniform scale a picked tomato rides at: the plant's fit scale on
/// the tomato's full growth, so a picked fruit is exactly the size it had
/// on the plant.
const TOMATO_PICK_SCALE: f32 = plant::TOMATO_MAX_SCALE * PLANT_SCALE;

/// The carried tomato's body box's half extents, in window pixels, for a
/// `size`-pixel image: half the fruit's natural size at the pick scale.
fn tomato_pick_half(size: [f32; 2]) -> [f32; 2] {
    [
        size[0] * TOMATO_PICK_SCALE / 2.0,
        size[1] * TOMATO_PICK_SCALE / 2.0,
    ]
}

/// The basket's U, in the basket node's local space (unscaled, y up,
/// origin at the sprite's center): three invisible axis-aligned boxes —
/// the left wall, the right wall, and the floor — that hold the dropped
/// fruit inside the cavity, the mouth left open at the top.
fn basket_walls() -> [frost::OrientedBox; 3] {
    [
        frost::OrientedBox::new([-124.0, 30.0], [16.0, 70.0]),
        frost::OrientedBox::new([128.0, 30.0], [16.0, 70.0]),
        frost::OrientedBox::new([2.0, -25.0], [130.0, 15.0]),
    ]
}

/// The basket's cavity mouth's x bounds in local space: the inner faces of
/// the [basket_walls] walls.
const BASKET_MOUTH_X: (f32, f32) = (-108.0, 112.0);

/// The basket's floor's top in local space: the inner face of the
/// [basket_walls] bottom box.
const BASKET_FLOOR: f32 = -10.0;

/// The uniform scale the basket rides at: the same fit the items panel
/// uses, so the basket keeps its proportions across resizes.
fn basket_scale(h: f32) -> f32 {
    (h - 2.0 * MARGIN) / ITEMS_SIZE[1]
}

/// The basket group's center in window-centered user space, for a window
/// of the given size: in the bottom left, just right of the items panel —
/// `BASKET_GAP` clear of its right edge — and `MARGIN` clear of the
/// bottom border.
fn basket_center(w: f32, h: f32) -> [f32; 2] {
    let s = basket_scale(h);
    [
        -(w / 2.0) + MARGIN + ITEMS_SIZE[0] * s + BASKET_GAP + BASKET_IMAGE[0] * s / 2.0,
        -h / 2.0 + MARGIN + BASKET_IMAGE[1] * s / 2.0,
    ]
}

/// Whether a drop at the basket-local point `(lx, ly)` is kept: the body's
/// center is inside the U — the mouth's x range and above the floor. There
/// is no upper bound: fruit may be dropped above the rim, so it can pile
/// high.
fn basket_accepts(lx: f32, ly: f32) -> bool {
    BASKET_MOUTH_X.0 < lx && lx < BASKET_MOUTH_X.1 && ly > BASKET_FLOOR
}

/// Whether the user-space point `p` is inside slot `i`, for the panel node
/// `items`: the slot's cell in the panel's local space — the full panel
/// width by the strip's per-slot height, centered on `slot_local(i)` —
/// mapped through the panel's world transform, the same scale-then-
/// transform composition the renderer draws it with.
fn slot_hovered(items: &frost::SceneNode, i: usize, p: [f32; 2]) -> bool {
    let world = frost::Transform::scale(items.scale[0], items.scale[1]).compose(&items.transform);
    let [cx, cy] = world.apply(slot_local(i));
    (p[0] - cx).abs() <= ITEMS_SIZE[0] * items.scale[0] / 2.0
        && (p[1] - cy).abs()
            <= (ITEMS_SIZE[1] - 2.0 * SLOT_MARGIN) * items.scale[1] / SLOTS as f32 / 2.0
}

/// The tool a slot node holds, read from the sprite it shows: the two
/// sprites have different texture sizes, so the size identifies the tool,
/// and a node with no shape holds nothing.
fn slot_tool(node: &frost::SceneNode) -> Option<Tool> {
    let (w, h) = match &node.shape {
        Some(frost::Shape::Sprite { width, height, .. }) => (*width as f32, *height as f32),
        _ => return None,
    };
    if (w, h) == (CAN_IMAGE[0], CAN_IMAGE[1]) {
        Some(Tool::WaterCan)
    } else if (w, h) == (SPRAY_IMAGE[0], SPRAY_IMAGE[1]) {
        Some(Tool::SprayCan)
    } else {
        None
    }
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

/// `Getingeye1.png` and `Getingeye2.png`'s texture size in pixels: both
/// frames are the same size, and the swarm fits that width to the
/// rendered bee width inside the [`vipers`] module.
const VIPER_IMAGE: [f32; 2] = [198.0, 179.0];

/// The two cursor tools. The mouse holds one active tool, drawn as the
/// cursor, and one stored tool, mirrored in the held-items panel: the
/// right-button switch swaps the two, and a left click on a slot swaps the
/// active tool with the slot's.
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
    /// The active tool, or `None` while the mouse holds none: the one
    /// drawn as the cursor and mirrored in the left cell of the
    /// held-items panel. A left click on a slot swaps it with the slot's
    /// tool, and the right-button switch swaps it with the stored one.
    active: Option<Tool>,
    /// The second tool the mouse holds, or `None`: stored without a
    /// sprite of its own, mirrored in the right cell of the held-items
    /// panel; the right-button switch swaps it with the active tool.
    held: Option<Tool>,
    /// The slot the current left press started on, if any: a press that
    /// starts on a slot is a swap click, completed only if the release
    /// lands on the same slot.
    press_slot: Option<usize>,
    /// Whether the left mouse button was down on the previous frame; the
    /// press and release edges are derived from it.
    pressed: bool,
    /// The tomato being carried, if any: the plant index and the bloom
    /// slot the fruit was picked from. While set, the fruit's pivot rides
    /// the cursor in the held-fruit node (root children[9]) on top of
    /// everything; the release either keeps it in the basket or sends it
    /// back to its plant.
    picking: Option<(usize, usize)>,
    /// Whether the right mouse button was down on the previous frame; the
    /// two held tools switch on its release.
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
    /// The two viper frames, loaded once: `viper1` the first wingbeat
    /// frame and `viper2` the second; the swarm's bee nodes swap between
    /// them as cheap `Arc` clones.
    viper1: frost::Shape,
    viper2: frost::Shape,
    /// The three bug walk frames, loaded once; the swarm's bug nodes swap
    /// between them as cheap `Arc` clones.
    bug1: frost::Shape,
    bug2: frost::Shape,
    bug3: frost::Shape,
    /// The flower shape, grown on a plant slice's spawn points from the
    /// frame its slice reaches full size; the plant nodes' flower leaves
    /// swap it in as cheap `Arc` clones.
    flower: frost::Shape,
    /// The tomato background (the white fruit body), grown out of each
    /// flower once it is fully grown; the plant nodes' tomato leaves swap
    /// it in as cheap `Arc` clones.
    tomato: frost::Shape,
    /// The tomato foreground (the dark calyx and stem), drawn on top of
    /// the background at the same point; the plant nodes' tomato leaves
    /// swap it in as cheap `Arc` clones, unmodulated.
    tomato_fg: frost::Shape,
    /// The drops pouring out of the spout: the simulation state, stepped
    /// once per frame: each drop is also matched against the growing
    /// plants' root hitboxes to restore their water reserves (`waters`).
    water: frost::ParticleSystem,
    /// The green spray emitted by the spray can: the simulation state,
    /// stepped once per frame.
    spray: frost::ParticleSystem,
    /// The random source, for the per-particle jitter: the engine's
    /// `frost::Rng`, seeded from the clock, so the jitter differs between
    /// runs.
    rng: frost::Rng,
    /// The emission accumulator: `RATE * dt` (or `SPRAY_RATE * dt`) is
    /// added each frame and one particle is spawned per whole unit, so the
    /// rate holds at any dt.
    acc: f32,
    /// The tomato plants on the grass, in growth order: each has its own
    /// growth clock, and the process steps it only once the previous one
    /// is fully grown — the first from launch on — and only while its
    /// water reserve (`waters`) holds, then lays it out on the matching
    /// child of the plants node (root children[2]), in parallel with the
    /// tool system.
    plants: [plant::Plant; PLANT_POS.len()],
    /// The plants' water reserves, in growth order, each from 1.0 (well
    /// watered) to 0.0 (dry): the process drains a started plant that is
    /// not yet complete by `dt / (DRAIN_TIME * GROW_SLOWDOWN)` per frame —
    /// at the growth's
    /// slowed pace — and restores it by `DROP_WATER` per drop that falls
    /// into its root hitbox, and a plant at 0.0 stops growing, awaiting
    /// water.
    waters: [f32; PLANT_POS.len()],
    /// The swarm of vipers buzzing around the flower bench — the row of
    /// plants — in parallel with the tool system and the plants: one
    /// viper per fully grown plant layer, so the swarm grows as the bench
    /// does; stepped every frame with the number of fully grown layers
    /// and the layer-segment midpoints, and laid out on the matching
    /// child of the vipers node (root children[4]).
    vipers: vipers::Vipers,
    /// The swarm of bugs waddling to the plants: three spawn each time a
    /// plant starts growing, and the swarm steps and lays them out on the
    /// matching child of the bugs node (root children[5]).
    bugs: bugs::Bugs,
    /// The audio output, opened once at startup; the bug tjatter and
    /// plopp clips play through it.
    audio: frost::Audio,
    /// The three bug tjatter clips (low, mid, high), decoded once at
    /// startup; a bug that reaches its destination while another bug is
    /// within 50 px of it chatters on one of them, the swarm's random
    /// pick.
    tjatters: [frost::Sound; 3],
    /// The three bug plopp clips (1, 2, 3), decoded once at startup; a
    /// bug that pops up out of the grass — a batch spawn or a respawn —
    /// plops on one of them, the swarm's random pick.
    plops: [frost::Sound; 3],
    /// The four bug Aj clips (1 to 4), decoded once at startup; every
    /// time the mist drops a bug's health the bug cries out on one of
    /// them, the swarm's random pick.
    ajs: [frost::Sound; 4],
    /// The watering sound, `assets/audio/WaterFlowSoft.wav`, decoded once
    /// at startup; the audio output's one loop, playing while water
    /// actually pours out of the spout and silenced the frame pouring
    /// stops.
    pour: frost::Sound,
    /// Whether the audio output's loop is currently playing the pour
    /// sound: the process flips it on the pour's edges — the watering can
    /// active and fully tilted (rising), pouring ending, the can turning
    /// back upright or the tool switching away (falling) — starting the
    /// loop on the rising edge and stopping it on the falling one.
    pouring: bool,
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

        // Bottom right: `MARGIN` clear of the right and bottom borders, at
        // the panel's natural size.
        let held_panel = &mut ctx.scene().root.children[3];
        held_panel.transform = frost::Transform::translate(
            w / 2.0 - MARGIN - HELD_SIZE[0] * held_panel.scale[0] / 2.0,
            -h / 2.0 + MARGIN + HELD_SIZE[1] * held_panel.scale[1] / 2.0,
        );

        // Bottom left: just right of the items panel — `BASKET_GAP` clear
        // of its right edge — and `MARGIN` clear of the bottom border,
        // scaled with the panel's fit so the basket keeps its proportions.
        let basket = &mut ctx.scene().root.children[7];
        let bs = basket_scale(h);
        let bc = basket_center(w, h);
        basket.scale = [bs, bs];
        basket.transform = frost::Transform::translate(bc[0], bc[1]);

        // Top right: `MARGIN` clear of the top and right borders, at
        // `IMMORTALITY_SCALE` of the badge's natural size.
        let badge = &mut ctx.scene().root.children[8];
        badge.scale = [IMMORTALITY_SCALE, IMMORTALITY_SCALE];
        badge.transform = frost::Transform::translate(
            w / 2.0 - MARGIN - IMMORTALITY_SIZE[0] * IMMORTALITY_SCALE / 2.0,
            h / 2.0 - MARGIN - IMMORTALITY_SIZE[1] * IMMORTALITY_SCALE / 2.0,
        );

        // The plants' root joints in user space, where the grass stretch
        // maps each `PLANT_POS` pixel: the plants stay glued to them, and
        // the vipers' orbit centers lift off them.
        let anchors = PLANT_POS.map(|pos| {
            [
                pos[0] * w / GRASS_SIZE[0] - w / 2.0,
                h / 2.0 - pos[1] * h / GRASS_SIZE[1],
            ]
        });

        // Grow the plants one at a time, in parallel with the tool
        // system: the first starts at launch, each next one starts when
        // the previous is fully grown; `active_plants` counts the spawned
        // ones — the ones whose growth clock has started — for the bug
        // swarm, and `grown_layers` counts the fully grown ones, layer by
        // layer, for the viper swarm.
        //
        // Growth proceeds only while the plant is well watered, and at
        // `1 / GROW_SLOWDOWN` of real time's pace: a started plant that
        // is not yet complete — its slices or its blooms still growing —
        // drains its water reserve by
        // `dt / (DRAIN_TIME * GROW_SLOWDOWN)` every frame — the drain
        // runs with the growth — and steps its clock by `dt /
        // GROW_SLOWDOWN`, so fully grown slices keep opening blooms while
        // the water holds; a plant whose reserve runs dry stops growing —
        // its growth clock freezes — awaiting the player to pour water on
        // its root; the falling drops are matched against the roots'
        // hitboxes below, once the particles have been stepped.
        let mut active_plants = 0usize;
        let mut grown_layers = 0usize;
        let started: [bool; PLANT_POS.len()] =
            std::array::from_fn(|i| i == 0 || self.plants[i - 1].fully_grown());
        for (i, _anchor) in anchors.iter().enumerate() {
            if started[i] {
                active_plants += 1;
                if !self.plants[i].complete() {
                    self.waters[i] = (self.waters[i] - dt / (DRAIN_TIME * GROW_SLOWDOWN)).max(0.0);
                    // The growth clock runs slow, and holds while the
                    // reserve is dry.
                    if self.waters[i] > 0.0 {
                        self.plants[i].step(dt / GROW_SLOWDOWN);
                    }
                }
            }
            grown_layers += self.plants[i].grown_layers();
        }

        // Buzz the vipers around the flower bench, in parallel with
        // everything else: one viper per fully grown layer, so the swarm
        // grows as the bench does — `grown_layers` is how many exist —
        // and each viper circles the segment its layer spawned it, at
        // that segment's midpoint along the static, fully grown chain,
        // the sway left out, lifted to its plant's anchor at the fit
        // scale.
        let vipers_node = &mut ctx.scene().root.children[4];
        let centers: [[[f32; 2]; vipers::LAYERS]; PLANT_POS.len()] = std::array::from_fn(|i| {
            self.plants[i].layer_midpoints().map(|m| {
                [
                    anchors[i][0] + m[0] * PLANT_SCALE,
                    anchors[i][1] + m[1] * PLANT_SCALE,
                ]
            })
        });
        self.vipers.step(dt, &centers, grown_layers);
        self.vipers
            .layout(vipers_node, [&self.viper1, &self.viper2]);

        // Waddle the bugs to the plants, in parallel with everything
        // else: three spawn each time a plant starts growing.
        let bugs_node = &mut ctx.scene().root.children[5];
        let events = self.bugs.step(dt, &anchors, active_plants);
        self.bugs
            .layout(bugs_node, [&self.bug1, &self.bug2, &self.bug3]);
        // An arrival chatters: a bug that just reached its destination
        // while another bug was on the grass nearby plays one of the
        // tjatter clips, the swarm's random pick.
        for clip in events.tjatters {
            self.audio.play_once(&self.tjatters[clip], None);
        }
        // A pop-up plops: a bug that just came up out of the grass — a
        // batch spawn or a respawn — plays one of the plopp clips, the
        // swarm's random pick.
        for clip in events.plops {
            self.audio.play_once(&self.plops[clip], None);
        }

        // Follow the pointer, keeping the last known position while the
        // cursor is outside the window.
        if let Some(pos) = ctx.mouse_position() {
            self.mouse = pos;
        }
        let [mx, my] = self.mouse;

        let left = ctx.mouse_button_down(frost::MouseButton::Left);
        let right = ctx.mouse_button_down(frost::MouseButton::Right);

        // The slot the pointer is over, if any: a left click swaps the
        // active tool with a slot's tool, and the click is recognized by
        // the press and the release both landing on the same slot.
        let items_node = &ctx.scene().root.children[1];
        let on_slot = (0..SLOTS).find(|&i| slot_hovered(items_node, i, self.mouse));

        // A press followed by a release of the right mouse button switches
        // the two tools the mouse holds: the stored one becomes the active
        // one and the active one is stored, each in its upright, at-rest
        // pose.
        if self.right_pressed && !right {
            std::mem::swap(&mut self.active, &mut self.held);
            self.set_active(ctx, self.active);
        }
        self.right_pressed = right;

        // The left button's press and release edges. A press that starts
        // on a slot is a swap click: it uses no tool, and the release
        // completes the swap only if it lands on the same slot. A press
        // that starts elsewhere is a use of the active tool — or a tomato
        // pick, when no tool is held at all.
        if left && !self.pressed {
            self.press_slot = on_slot;
            if on_slot.is_none() && self.active.is_none() && self.picking.is_none() {
                // No tool held: pick up a fully developed tomato under the
                // pointer and carry it to the basket.
                if let Some(hit) = self.pick_tomato(ctx) {
                    self.picking = Some(hit);
                }
            } else if on_slot.is_none() && self.active == Some(Tool::WaterCan) {
                // Hold the left mouse button down to turn the can a quarter
                // turn counter-clockwise around the pointer; the release
                // turns it back. Each leg is a `ROTATE_TIME`-second tween
                // restarted from wherever the can currently is, so a
                // mid-rotation press or release picks up from the can's
                // live angle.
                self.rotation = frost::Tween::new(self.angle, CAN_ANGLE, ROTATE_TIME)
                    .repeat(frost::Repeat::Once);
            } else if on_slot.is_none() && self.active == Some(Tool::SprayCan) {
                // A fresh press — one after a release, not a re-press
                // mid-burst — triggers a burst while the can stands
                // upright.
                if self.burst.is_none() {
                    self.burst = Some(0.0);
                    // PingPong is the tween's default: `0 ->
                    // BURST_ANGLE` in `BURST_HALF` seconds and back in the
                    // same time, so the tilting and the return take
                    // `BURST_TIME` in all. The ticking below stops there,
                    // before the cycle could wrap.
                    self.rotation = frost::Tween::new(0.0, BURST_ANGLE, BURST_HALF);
                    self.set_spray_frame(ctx, true);
                }
            }
        } else if !left && self.pressed {
            if let Some((pi, si)) = self.picking.take() {
                // The press was a tomato pick: drop the fruit — into the
                // basket, or back onto the plant.
                self.drop_tomato(ctx, pi, si);
            } else if let Some(slot) = self.press_slot {
                // The press started on a slot: complete the swap only if
                // the release is still on that slot.
                if on_slot == Some(slot) {
                    self.swap_with_slot(ctx, slot);
                }
            } else if self.active == Some(Tool::WaterCan) {
                // A release that started in the world turns the can back
                // the same way, over `ROTATE_TIME`.
                self.rotation =
                    frost::Tween::new(self.angle, 0.0, ROTATE_TIME).repeat(frost::Repeat::Once);
            }
            self.press_slot = None;
        }
        self.pressed = left;

        // A carried tomato rides the cursor: the pivot under the pointer,
        // the fruit's top pinned to it, in window-centered user space on
        // top of everything. It is pushed out of the basket's U every
        // frame — the body's square matched against the walls mapped into
        // window space — so the fruit cannot be carried through a wall.
        // There is no gravity, and no collision with the fruit already in
        // the basket.
        if self.picking.is_some() {
            let basket = &ctx.scene().root.children[7];
            let s = basket.scale[0];
            let [bx, by] = basket.transform.apply([0.0, 0.0]);
            let [ox, oy] = plant::tomato_leaf_offset(sprite_size(&self.tomato));
            let [hx, hy] = tomato_pick_half(sprite_size(&self.tomato));
            let mut body = [
                self.mouse[0] + TOMATO_PICK_SCALE * ox,
                self.mouse[1] + TOMATO_PICK_SCALE * oy,
            ];
            for wall in basket_walls() {
                let wall_box = frost::Collider::Box(frost::OrientedBox::new(
                    [bx + s * wall.center[0], by + s * wall.center[1]],
                    [s * wall.half[0], s * wall.half[1]],
                ));
                let body_box = frost::Collider::Box(frost::OrientedBox::new(body, [hx, hy]));
                if let Some(push) = body_box.push_out(&wall_box) {
                    body = [
                        body[0] + push.dir[0] * push.depth,
                        body[1] + push.dir[1] * push.depth,
                    ];
                }
            }
            ctx.scene().root.children[9].children[0].transform = frost::Transform::translate(
                body[0] - TOMATO_PICK_SCALE * ox,
                body[1] - TOMATO_PICK_SCALE * oy,
            );
        }

        // The fruit that has been dropped into the basket: each one stays
        // exactly where it was released — no gravity, no settling, no
        // tomato-to-tomato collision — but none may sit inside a wall:
        // every body's square is pushed out of the U's three walls, in
        // window space, every frame.
        {
            let basket = &ctx.scene().root.children[7];
            let s = basket.scale[0];
            let [bx, by] = basket.transform.apply([0.0, 0.0]);
            let [ox, oy] = plant::tomato_leaf_offset(sprite_size(&self.tomato));
            let [hx, hy] = tomato_pick_half(sprite_size(&self.tomato));
            for fruit in &mut ctx.scene().root.children[7].children[1].children {
                let [tx, ty] = fruit.transform.apply([0.0, 0.0]);
                let mut body = [
                    bx + s * tx + TOMATO_PICK_SCALE * ox,
                    by + s * ty + TOMATO_PICK_SCALE * oy,
                ];
                for wall in basket_walls() {
                    let wall_box = frost::Collider::Box(frost::OrientedBox::new(
                        [bx + s * wall.center[0], by + s * wall.center[1]],
                        [s * wall.half[0], s * wall.half[1]],
                    ));
                    let body_box = frost::Collider::Box(frost::OrientedBox::new(body, [hx, hy]));
                    if let Some(push) = body_box.push_out(&wall_box) {
                        body = [
                            body[0] + push.dir[0] * push.depth,
                            body[1] + push.dir[1] * push.depth,
                        ];
                    }
                }
                fruit.transform = frost::Transform::translate(
                    (body[0] - bx - TOMATO_PICK_SCALE * ox) / s,
                    (body[1] - by - TOMATO_PICK_SCALE * oy) / s,
                );
            }
        }

        // Lay the plants out, after the input edges, so a tomato that was
        // picked or snapped back this frame is reposed by its plant — and
        // a picked slot's pivot, out of the tree, is simply skipped.
        let plants_node = &mut ctx.scene().root.children[2];
        for (i, anchor) in anchors.iter().enumerate() {
            self.plants[i].layout(
                &mut plants_node.children[i],
                *anchor,
                &self.flower,
                &self.tomato,
                &self.tomato_fg,
            );
        }

        // The active tool's live pose, ticked every frame so an ongoing
        // tilt, return, or burst keeps moving; no tool held is a no-op.
        match self.active {
            // The watering can pours at the full tilt.
            Some(Tool::WaterCan) => {
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
            // The spray can: the burst, once a press started it, runs to
            // `BURST_TIME`; nothing to do while no burst is in flight.
            Some(Tool::SprayCan) => {
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
            // No tool held: nothing to tilt, burst, or emit.
            None => {}
        }

        // The pour sound follows the spout: it starts, looping, the frame
        // water actually pours out of the can — the watering can active
        // and fully tilted, the same condition the drops spawn on — and
        // stops the frame pouring ends, the can turning back upright or
        // the tool switching away from it.
        let pouring = self.active == Some(Tool::WaterCan) && self.angle >= CAN_ANGLE - TILT_EPS;
        if pouring != self.pouring {
            self.pouring = pouring;
            if pouring {
                self.audio.play_loop(&self.pour);
            } else {
                self.audio.stop_loop();
            }
        }

        // A fresh transform is only needed while the node shows a tool.
        if let Some(tool) = self.active {
            let tool_node = &mut ctx.scene().root.children[6];
            tool_node.transform = match tool {
                Tool::WaterCan => can_transform(mx, my, self.angle),
                Tool::SprayCan => spray_transform(mx, my, self.angle),
            };
        }

        // Advance the particles even while not emitting, so an ongoing
        // stream keeps falling until it dies out.
        self.water.update(dt, [0.0, -GRAVITY]);
        self.spray.update(dt, [0.0, -SPRAY_GRAVITY]);

        // Water the roots: each drop that passes through a started plant's
        // rough hitbox — a `ROOT_RADIUS`-pixel circle around the root
        // joint — while the plant is not yet complete restores the plant's
        // water reserve by `DROP_WATER`, capped at full; the drops keep
        // falling on through, so one stream can top up several roots at
        // once.
        for p in &self.water.particles {
            for (i, anchor) in anchors.iter().enumerate() {
                if started[i]
                    && !self.plants[i].complete()
                    && self.waters[i] < 1.0
                    && in_root_hitbox(p.pos, *anchor)
                {
                    self.waters[i] = (self.waters[i] + DROP_WATER).min(1.0);
                }
            }
        }

        // Mist touching a bug wounds it: each live spray drop hits every
        // bug within its reach, but a wounded bug takes its next hit only
        // after its hit cooldown, and a hit that empties the counter
        // starts the bounce-then-evaporate death. A bug at its park spot
        // regains one hit of health every 5 seconds, up to six. The
        // water can's drops never touch the bugs. Every wound cries out
        // on one of the Aj clips, the swarm's random pick.
        for p in &self.spray.particles {
            let hit = self.bugs.hit_at(p.pos);
            for clip in hit.ajs {
                self.audio.play_once(&self.ajs[clip], Some(0.5));
            }
        }

        // Draw the bugs' health counters: one white pip per hit each
        // visible bug can still take, in a row above it — centered on
        // the bug's current count, three to six pips wide.
        for ([cx, cy], hits) in self.bugs.health_pips() {
            for i in 0..hits {
                ctx.circle(
                    cx - (hits as f32 - 1.0) * 3.0 + i as f32 * 6.0,
                    cy,
                    2.0,
                    PIP,
                    Z,
                );
            }
        }

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

        // Draw the water bars: a dark background with a blue fill showing
        // the reserve, `BAR_LIFT` pixels above the root joint of every
        // plant that has started growing and is not complete yet — its
        // slices or its blooms still growing — over the root, the spot the
        // player pours on, because at the fit scale a fully grown plant's
        // top reaches the window's top edge and leaves no room above the
        // plant itself. The fill grows from the bar's left edge as the
        // reserve refills.
        for (i, anchor) in anchors.iter().enumerate() {
            if started[i] && !self.plants[i].complete() {
                let level = self.waters[i];
                ctx.rectangle(
                    anchor[0],
                    anchor[1] + BAR_LIFT,
                    BAR_DX + BAR_BORDER,
                    BAR_DY + BAR_BORDER,
                    BAR_BG,
                    Z,
                );
                ctx.rectangle(
                    anchor[0] - BAR_DX * (1.0 - level),
                    anchor[1] + BAR_LIFT,
                    BAR_DX * level,
                    BAR_DY,
                    DROP,
                    Z,
                );
            }
        }
    }
}

impl Demo {
    /// Makes `tool` the active tool — `None` for no tool: it resets the
    /// can to its upright, at-rest pose (angle, burst, rotation tween, and
    /// frame) and puts the tool's shape and scale on the node, or clears
    /// it, so no tool inherits another's tilt, burst, or sprite.
    fn set_active(&mut self, ctx: &mut frost::Context, tool: Option<Tool>) {
        self.active = tool;
        self.angle = 0.0;
        self.burst = None;
        self.rotation = frost::Tween::new(0.0, 0.0, 1.0).repeat(frost::Repeat::Once);
        self.showing_spray2 = false;
        let tool_node = &mut ctx.scene().root.children[6];
        match tool {
            Some(Tool::WaterCan) => {
                tool_node.shape = Some(self.can.clone());
                tool_node.scale = [CAN_SCALE, CAN_SCALE];
            }
            Some(Tool::SprayCan) => {
                tool_node.shape = Some(self.spray1.clone());
                tool_node.scale = [1.0, 1.0];
            }
            None => {
                tool_node.shape = None;
            }
        }
        // Mirror both tools into the held-items panel: the active one in
        // the left cell, the stored one in the right. Every tool change —
        // the right-button switch and the slot swaps alike — goes through
        // this method, so this single sync keeps the panel current.
        self.sync_held(ctx);
    }

    /// Swaps the active tool with whatever rests in slot `slot`: the
    /// slot's tool becomes the active one — or nothing, if the slot is
    /// empty, in which case the mouse shows no sprite, still holding the
    /// stored tool, if any — and the active tool moves into the slot.
    fn swap_with_slot(&mut self, ctx: &mut frost::Context, slot: usize) {
        let slot_tool = slot_tool(&ctx.scene().root.children[1].children[slot]);
        let incoming = self.active;
        self.set_active(ctx, slot_tool);
        self.slot_set(&mut ctx.scene().root.children[1].children[slot], incoming);
    }

    /// Rests `tool` — or nothing — in the slot's node: the tool's shape at
    /// the slot-fit scale, or no shape at all.
    fn slot_set(&mut self, node: &mut frost::SceneNode, tool: Option<Tool>) {
        match tool {
            Some(Tool::WaterCan) => {
                node.shape = Some(self.can.clone());
                node.scale = [slot_scale(CAN_IMAGE), slot_scale(CAN_IMAGE)];
            }
            Some(Tool::SprayCan) => {
                node.shape = Some(self.spray1.clone());
                node.scale = [slot_scale(SPRAY_IMAGE), slot_scale(SPRAY_IMAGE)];
            }
            None => node.shape = None,
        }
    }

    /// Shows the pressed spray frame (`spray2`) if `on` and the at-rest
    /// frame (`spray1`) otherwise, but only when the tool node currently
    /// shows the other one, so the swap — a cheap `Arc` clone — happens at
    /// most once per change.
    fn set_spray_frame(&mut self, ctx: &mut frost::Context, on: bool) {
        if self.showing_spray2 == on {
            return;
        }
        self.showing_spray2 = on;
        ctx.scene().root.children[6].shape = Some(if on {
            self.spray2.clone()
        } else {
            self.spray1.clone()
        });
    }

    /// Mirrors the mouse's two tools into the held-items panel: the active
    /// tool in the left cell, the stored one in the right, each at its
    /// at-rest frame in the held-fit scale — or an empty cell for `None`.
    fn sync_held(&mut self, ctx: &mut frost::Context) {
        let held = &mut ctx.scene().root.children[3];
        self.cell_set(&mut held.children[0], self.active);
        self.cell_set(&mut held.children[1], self.held);
    }

    /// Puts `tool` — or nothing — in a held cell's node: the tool's shape
    /// at the held-fit scale, always the at-rest frame, or no shape at
    /// all.
    fn cell_set(&mut self, node: &mut frost::SceneNode, tool: Option<Tool>) {
        match tool {
            Some(Tool::WaterCan) => {
                node.shape = Some(self.can.clone());
                node.scale = [held_scale(CAN_IMAGE), held_scale(CAN_IMAGE)];
            }
            Some(Tool::SprayCan) => {
                node.shape = Some(self.spray1.clone());
                node.scale = [held_scale(SPRAY_IMAGE), held_scale(SPRAY_IMAGE)];
            }
            None => node.shape = None,
        }
    }

    /// Picks up the fully developed tomato the pointer rests on — the
    /// first hit in plant, then slot, order — and starts carrying it: the
    /// fruit's pivot is reparented out of its plant's slot and into the
    /// held-fruit container (root children[9]), at the pick scale, under
    /// the pointer, so it paints above everything. The plant's slot is
    /// marked harvested while it is carried; the pointer must not be on a
    /// slot or holding a tool, which the caller checks. Returns the picked
    /// plant and slot.
    fn pick_tomato(&mut self, ctx: &mut frost::Context) -> Option<(usize, usize)> {
        // The hit test: every plant's unharvested, ripe slots, their body
        // centers mapped to window space through the plant node's current
        // transform (the sway included), against the pointer's square
        // around the cursor.
        let hit = {
            let plant_nodes = &ctx.scene().root.children[2].children;
            let [hx, hy] = tomato_pick_half(sprite_size(&self.tomato));
            self.plants.iter().enumerate().find_map(|(pi, p)| {
                (0..plant::FLOWER_N)
                    .find(|&si| {
                        if p.is_harvested(si) || !p.ripe(si) {
                            return false;
                        }
                        let center =
                            p.tomato_center(si, &plant_nodes[pi].children[5 + si], &self.tomato);
                        let world = frost::Transform::scale(PLANT_SCALE, PLANT_SCALE)
                            .compose(&plant_nodes[pi].transform);
                        let [cx, cy] = world.apply(center);
                        (cx - self.mouse[0]).abs() <= hx && (cy - self.mouse[1]).abs() <= hy
                    })
                    .map(|si| (pi, si))
            })
        };
        let (pi, si) = hit?;
        self.plants[pi].harvest(si);
        // Rehome the pivot: out of the slot, into the held-fruit
        // container, at the pick scale under the pointer.
        let root = &mut ctx.scene().root;
        let pivot = root.children[2].children[pi].children[5 + si]
            .children
            .remove(1);
        let mut pivot = *pivot;
        pivot.scale = [TOMATO_PICK_SCALE, TOMATO_PICK_SCALE];
        pivot.transform = frost::Transform::translate(self.mouse[0], self.mouse[1]);
        root.children[9].children.push(Box::new(pivot));
        Some((pi, si))
    }

    /// Drops the carried tomato picked from (plant, slot) `(pi, si)`:
    /// with the body's center inside the basket's U — the mouth's x range
    /// and above the floor — the pivot is reparented into the basket's
    /// fruit container (root children[7]'s middle child), so it renders
    /// behind the front half and in front of the back, and it stays
    /// exactly where it was released: no gravity, no tomato-to-tomato
    /// collision, the fruit piles freely — and the bloom it came from
    /// starts over: the flower is removed and regrows from zero, its
    /// tomato after it, on the same water-gated, slowed schedule.
    /// Otherwise it goes back to its plant — the harvest flag clears and
    /// the pivot is reparented into the slot — where the layout, which
    /// runs later in the same frame, reposes it.
    fn drop_tomato(&mut self, ctx: &mut frost::Context, pi: usize, si: usize) {
        // The body's center in window space, and its position in the
        // basket's local space, where the U is measured.
        let (w, h) = ctx.size();
        let s = basket_scale(h);
        let center = basket_center(w, h);
        let [ox, oy] = plant::tomato_leaf_offset(sprite_size(&self.tomato));
        let body = [
            self.mouse[0] + TOMATO_PICK_SCALE * ox,
            self.mouse[1] + TOMATO_PICK_SCALE * oy,
        ];
        let lx = (body[0] - center[0]) / s;
        let ly = (body[1] - center[1]) / s;
        let kept = basket_accepts(lx, ly);

        let root = &mut ctx.scene().root;
        let mut pivot = *root.children[9].children.remove(0);
        if kept {
            // Scale the pivot into the basket's local space and pin it so
            // the body's center maps back to the release point.
            pivot.scale = [TOMATO_PICK_SCALE / s, TOMATO_PICK_SCALE / s];
            pivot.transform =
                frost::Transform::translate((body[0] - center[0]) / s, (body[1] - center[1]) / s);
            root.children[7].children[1].children.push(Box::new(pivot));
            // The bloom starts over: the plant records the reset — which
            // re-arms the growth clock, the watering hitbox and the water
            // bar — and the slot takes a fresh, shapeless tomato pivot;
            // the next layout removes the flower and regrows it from
            // zero, the tomato after it.
            self.plants[pi].regrow(si);
            root.children[2].children[pi].children[5 + si]
                .children
                .push(Box::new(frost::SceneNode {
                    children: vec![
                        Box::new(frost::SceneNode::default()),
                        Box::new(frost::SceneNode::default()),
                    ],
                    ..Default::default()
                }));
        } else {
            // Snap back to the plant; the layout reposes the pivot this
            // same frame.
            self.plants[pi].unharvest(si);
            root.children[2].children[pi].children[5 + si]
                .children
                .push(Box::new(pivot));
        }
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset paths to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    // let root = std::env::current_exe().expect("failed to get current exe");
    // let root = root.parent().expect("failed to get parent directory");
    // let root = root.to_str().expect("failed to convert root to string");
    let grass = frost::Shape::sprite(format!("{root}/assets/sprites/grass.png"))
        .expect("failed to load assets/sprites/grass.png");
    let can = frost::Shape::sprite(format!("{root}/assets/sprites/water_can_outline.png"))
        .expect("failed to load assets/sprites/water_can_outline.png");
    let spray1 = frost::Shape::sprite(format!("{root}/assets/sprites/Spray1.png"))
        .expect("failed to load assets/sprites/Spray1.png");
    let spray2 = frost::Shape::sprite(format!("{root}/assets/sprites/Spray2.png"))
        .expect("failed to load assets/sprites/Spray2.png");
    let viper1 = frost::Shape::sprite(format!("{root}/assets/sprites/Getingeye1.png"))
        .expect("failed to load assets/sprites/Getingeye1.png");
    let viper2 = frost::Shape::sprite(format!("{root}/assets/sprites/Getingeye2.png"))
        .expect("failed to load assets/sprites/Getingeye2.png");
    let bug1 = frost::Shape::sprite(format!("{root}/assets/sprites/Bug1a.png"))
        .expect("failed to load assets/sprites/Bug1a.png");
    let bug2 = frost::Shape::sprite(format!("{root}/assets/sprites/Bug2a.png"))
        .expect("failed to load assets/sprites/Bug2a.png");
    let bug3 = frost::Shape::sprite(format!("{root}/assets/sprites/Bug3a.png"))
        .expect("failed to load assets/sprites/Bug3a.png");
    let items = frost::Shape::sprite(format!("{root}/assets/sprites/items.png"))
        .expect("failed to load assets/sprites/items.png");
    let held_items = frost::Shape::sprite(format!("{root}/assets/sprites/held_items.png"))
        .expect("failed to load assets/sprites/held_items.png");
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
    let flower = frost::Shape::sprite(format!("{root}/assets/sprites/flower.png"))
        .expect("failed to load assets/sprites/flower.png");
    let tomato = frost::Shape::sprite(format!("{root}/assets/sprites/tomato.png"))
        .expect("failed to load assets/sprites/tomato.png");
    let tomato_fg = frost::Shape::sprite(format!("{root}/assets/sprites/tomato_fg.png"))
        .expect("failed to load assets/sprites/tomato_fg.png");
    let basket_back = frost::Shape::sprite(format!("{root}/assets/sprites/BasketBack.png"))
        .expect("failed to load assets/sprites/BasketBack.png");
    let basket_front = frost::Shape::sprite(format!("{root}/assets/sprites/BasketFront.png"))
        .expect("failed to load assets/sprites/BasketFront.png");
    // The immortality badge: tinted down to a faint watermark.
    let mut immortality =
        frost::Shape::sprite(format!("{root}/assets/sprites/sustainble_immortality.png"))
            .expect("failed to load assets/sprites/sustainble_immortality.png");
    if let frost::Shape::Sprite { alpha, .. } = &mut immortality {
        *alpha = IMMORTALITY_ALPHA;
    }
    let plant = plant::Plant::new([&plant1, &plant2, &plant3, &plant4, &plant5]);

    // The audio output, decoded once at startup, and the bug clips: the
    // three tjatter clips (a bug that reaches its destination while
    // another bug is on the grass within 50 px of it chatters on one of
    // them, the swarm's random pick), the three plopp clips (a bug that
    // pops up out of the grass — a batch spawn or a respawn — plops on
    // one of them, the swarm's random pick), the four Aj clips (the bug
    // cries out on one of them, the swarm's random pick, every time the
    // mist drops its health), and the watering loop (it plays, looping,
    // while water pours out of the spout, silenced the frame pouring
    // stops).
    let audio = frost::Audio::new().expect("failed to open the audio output device");
    let tjatters = [
        frost::Sound::load(format!("{root}/assets/audio/TjatterLow.wav"))
            .expect("failed to load assets/audio/TjatterLow.wav"),
        frost::Sound::load(format!("{root}/assets/audio/TjatterMid.wav"))
            .expect("failed to load assets/audio/TjatterMid.wav"),
        frost::Sound::load(format!("{root}/assets/audio/TjatterHigh.wav"))
            .expect("failed to load assets/audio/TjatterHigh.wav"),
    ];
    let plops = [
        frost::Sound::load(format!("{root}/assets/audio/bugs_plopp1.wav"))
            .expect("failed to load assets/audio/bugs_plopp1.wav"),
        frost::Sound::load(format!("{root}/assets/audio/bugs_plopp2.wav"))
            .expect("failed to load assets/audio/bugs_plopp2.wav"),
        frost::Sound::load(format!("{root}/assets/audio/bugs_plopp3.wav"))
            .expect("failed to load assets/audio/bugs_plopp3.wav"),
    ];
    let ajs = [
        frost::Sound::load(format!("{root}/assets/audio/Aj1.wav"))
            .expect("failed to load assets/audio/Aj1.wav"),
        frost::Sound::load(format!("{root}/assets/audio/Aj2.wav"))
            .expect("failed to load assets/audio/Aj2.wav"),
        frost::Sound::load(format!("{root}/assets/audio/Aj3.wav"))
            .expect("failed to load assets/audio/Aj3.wav"),
        frost::Sound::load(format!("{root}/assets/audio/Aj4.wav"))
            .expect("failed to load assets/audio/Aj4.wav"),
    ];
    let pour = frost::Sound::load(format!("{root}/assets/audio/WaterFlowSoft.wav"))
        .expect("failed to load assets/audio/WaterFlowSoft.wav");

    // One plant node, cloned for each plant: its origin is the root joint
    // (plant1's lower joint), positioned by the process every frame. The
    // fit scale is constant — each slice grows individually, riding the
    // node. The five slice children are followed by the FLOWER_N shapeless
    // flower slots, each a pivot that the process lays on its slice's
    // spawn point; a slot's children are the flower leaf and, on top of
    // it, a tomato pivot (the white fruit body leaf under the dark calyx-
    // and-stem leaf, both pinned so the body's top sits on the flower's
    // center and the fruit hangs below), so the blooms and fruit paint on
    // top of the slices and sway with the plant. The process grows and
    // tints the leaves. The clones are cheap `Arc` clones of the slice and
    // bloom pixel buffers.
    let mut plant_children: Vec<Box<frost::SceneNode>> = vec![
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
    ];
    plant_children.extend((0..plant::FLOWER_N).map(|_| {
        Box::new(frost::SceneNode {
            children: vec![
                // The flower leaf, under the tomato.
                Box::new(frost::SceneNode::default()),
                // The tomato pivot, on top of the flower: shapeless; the
                // process scales it to grow the fruit about the flower and
                // lays its two leaves so the body's top sits on the flower's
                // center, the fruit hanging below it.
                Box::new(frost::SceneNode {
                    children: vec![
                        // The fruit body (background), under the
                        // foreground.
                        Box::new(frost::SceneNode::default()),
                        // The calyx and stem (foreground), on top.
                        Box::new(frost::SceneNode::default()),
                    ],
                    ..Default::default()
                }),
            ],
            ..Default::default()
        })
    }));
    let plant_node = frost::SceneNode {
        scale: [PLANT_SCALE, PLANT_SCALE],
        children: plant_children,
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
                // and left borders. Its children are the four slots, one per
                // slot in slot order — a node paints its shape before its
                // children, so a resting tool renders on top of the panel —
                // and they ride its fit on resize. The spray can rests in
                // slot 2 and the watering can in slot 3; slots 0 and 1
                // start empty, and a left click can park either tool in any
                // slot.
                shape: Some(items),
                children: vec![
                    Box::new(frost::SceneNode {
                        // Slot 0: empty at start.
                        transform: frost::Transform::translate(slot_local(0)[0], slot_local(0)[1]),
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        // Slot 1: empty at start.
                        transform: frost::Transform::translate(slot_local(1)[0], slot_local(1)[1]),
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
                // The held-items panel in the bottom right corner, anchored
                // there by the process every frame: `MARGIN` clear of the
                // right and bottom borders, at its natural size. Its two
                // children mirror the mouse's tools — child 0 (the left
                // cell) the active one, child 1 (the right cell) the
                // stored one — each painted on top of the panel, since a
                // node paints its shape before its children; the cells are
                // synced by `set_active`.
                shape: Some(held_items),
                children: vec![
                    Box::new(frost::SceneNode {
                        // The left cell: the active tool, empty at start.
                        transform: frost::Transform::translate(held_local(0)[0], held_local(0)[1]),
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        // The right cell: the stored tool, empty at start.
                        transform: frost::Transform::translate(held_local(1)[0], held_local(1)[1]),
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The vipers' swarm around the flower bench: one child per
                // bee, in swarm order; the process lays each spawned
                // bee's pose, flip, and wingbeat frame out on its child
                // every frame, and the unspawned slots — the ones no plant
                // layer has spawned yet — never draw. The group carries no
                // shape or scale of its own. It sits under the tool node,
                // so the cursor paints above the bees.
                children: (0..vipers::N)
                    .map(|_| Box::new(frost::SceneNode::default()))
                    .collect(),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The bugs' swarm on the grass: one shape-less child per
                // slot, in spawn order; the process lays each spawned
                // bug's pose, flip, growth, and walk frame out on its
                // child every frame, and the unspawned slots never draw.
                // The group carries no shape or scale of its own; it sits
                // under the tool node, so the cursor paints above the
                // bugs.
                children: (0..BUG_N)
                    .map(|_| Box::new(frost::SceneNode::default()))
                    .collect(),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The active tool, starting with no shape: the mouse starts
                // holding no tool at all. Every switch — a slot swap or the
                // right-button switch — changes the shape — and the scale,
                // since the spray frames are drawn at their natural size —
                // on this same node; the stored tool is mirrored in the
                // held panel's right cell instead. It sits under the basket,
                // the badge, and the held-fruit node, so the cursor paints
                // above the panel, the plants, the bees, and the bugs, and
                // the carried fruit paints above the cursor.
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The harvest basket in the bottom left, just right of the
                // items panel, positioned and scaled by the process every
                // frame. Its children, in draw order, are the back half
                // (child 0), the shapeless fruit container (child 1) that
                // dropped tomatoes reparent into — piled on top of each
                // other, with no gravity and no tomato-to-tomato
                // collision — and the front half (child 2), so a dropped
                // tomato renders behind the front rim and in front of the
                // back.
                children: vec![
                    Box::new(frost::SceneNode {
                        shape: Some(basket_back),
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        shape: Some(basket_front),
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The immortality badge in the top right: half its
                // natural size, `MARGIN` clear of the top and right
                // borders, tinted to a faint watermark, positioned by the
                // process every frame. It sits under the held-fruit node,
                // so a carried tomato still paints above it.
                shape: Some(immortality),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The held-fruit container: the carried tomato's pivot
                // reparents here while the mouse button is down, so the
                // fruit paints above everything — the panel, the plants,
                // the basket, the badge, the cursor. The group carries no
                // shape or scale of its own; the child rides the cursor in
                // window-centered user space.
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
            active: None,
            held: None,
            press_slot: None,
            pressed: false,
            picking: None,
            right_pressed: false,
            angle: 0.0,
            rotation: frost::Tween::new(0.0, 0.0, 1.0).repeat(frost::Repeat::Once),
            burst: None,
            showing_spray2: false,
            can,
            spray1,
            spray2,
            viper1,
            viper2,
            // Three bugs per plant, each plant's batch exactly once: the
            // population climbs 3, 6, …, 18 over the first 75 seconds; a
            // killed bug pops back up at its spawn spot after a random 5
            // to 10 second delay.
            bugs: bugs::Bugs::new([&bug1, &bug2, &bug3]),
            bug1,
            bug2,
            bug3,
            audio,
            tjatters,
            plops,
            ajs,
            pour,
            pouring: false,
            flower,
            tomato,
            tomato_fg,
            water: frost::ParticleSystem::new(),
            spray: frost::ParticleSystem::new(),
            rng: frost::Rng::new(),
            acc: 0.0,
            // Six identical growth clocks, one per plant: each starts at
            // zero and the process steps it when the previous is fully
            // grown.
            plants: std::array::from_fn(|_| plant.clone()),
            // Six full water reserves, one per plant: each plant enters
            // well watered, the first one draining from launch on.
            waters: std::array::from_fn(|_| 1.0),
            vipers: vipers::Vipers::new(VIPER_IMAGE),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A full reserve, drained at the growth's slowed pace — a 60 fps
    /// frame step divided by `GROW_SLOWDOWN` — must run out strictly
    /// before the first plant's base slice is fully grown — over
    /// `plant::GROW_TIMES[0] * GROW_SLOWDOWN` real seconds — and only just
    /// before it: the bench is meant to pause within the last real second
    /// of the base's growth, before the flowers can start developing.
    #[test]
    fn first_plant_runs_dry_just_before_its_base_is_grown() {
        let dt = 1.0 / 60.0;
        let mut water = 1.0;
        let mut clock = 0.0; // growth-clock seconds
        let mut t = 0.0; // real seconds
        while water > 0.0 {
            water = (water - dt / (DRAIN_TIME * GROW_SLOWDOWN)).max(0.0);
            // The clock steps only while the reserve holds, as in the
            // process.
            if water > 0.0 {
                clock += dt / GROW_SLOWDOWN;
            }
            t += dt;
        }
        let base = plant::GROW_TIMES[0] * GROW_SLOWDOWN;
        assert!(
            clock < plant::GROW_TIMES[0],
            "the reserve must be dry before the base slice is fully grown (clock {clock} vs base {base})"
        );
        assert!(
            t < base,
            "the reserve must be dry before the base slice is fully grown (dry at {t}, base at {base})"
        );
        assert!(
            t >= base - 1.0,
            "the reserve must dry just before the base slice, not long before (dry at {t}, base at {base})"
        );
    }

    /// A full second of pouring — `RATE` drops, every one of them landing
    /// in the root hitbox — restores exactly one full reserve.
    #[test]
    fn a_full_second_of_pouring_is_one_full_reserve() {
        assert!((DROP_WATER * RATE - 1.0).abs() < 1e-6);
    }

    /// The root hitbox is a `ROOT_RADIUS`-pixel circle around the root
    /// joint: a drop on the boundary is inside, one just past it is not.
    #[test]
    fn root_hitbox_is_a_circle_around_the_anchor() {
        let anchor = [10.0, -20.0];
        assert!(in_root_hitbox([anchor[0] + ROOT_RADIUS, anchor[1]], anchor));
        assert!(in_root_hitbox([anchor[0], anchor[1] + ROOT_RADIUS], anchor));
        assert!(!in_root_hitbox(
            [anchor[0] + ROOT_RADIUS * 1.01, anchor[1]],
            anchor
        ));
        assert!(!in_root_hitbox(
            [anchor[0], anchor[1] - ROOT_RADIUS * 1.01],
            anchor
        ));
    }

    /// The basket rides in the bottom left corner of a 1920×1080 window:
    /// at the panel's fit scale, `BASKET_GAP` clear of the items panel's
    /// right edge and `MARGIN` clear of the bottom border.
    #[test]
    fn basket_fits_just_right_of_the_items_panel() {
        let (w, h) = (1920.0, 1080.0);
        let s = basket_scale(h);
        let c = basket_center(w, h);

        // The same fit the items panel uses.
        assert!((s - (h - 2.0 * MARGIN) / ITEMS_SIZE[1]).abs() < 1e-9);
        // The panel's right edge, from its mid-left placement.
        let panel_right = -(w / 2.0) + MARGIN + ITEMS_SIZE[0] * s;
        assert!(
            (c[0] - BASKET_IMAGE[0] * s / 2.0 - (panel_right + BASKET_GAP)).abs() < 1e-6,
            "the basket's left edge is not BASKET_GAP right of the panel"
        );
        assert!(
            (c[1] - BASKET_IMAGE[1] * s / 2.0 - (-h / 2.0 + MARGIN)).abs() < 1e-6,
            "the basket's bottom edge is not MARGIN off the bottom border"
        );
    }

    /// [basket_accepts] keeps a drop only inside the U's mouth: the x
    /// range between the walls' inner faces, above the floor — with no
    /// upper bound, so fruit dropped above the rim is kept and can pile
    /// high.
    #[test]
    fn basket_accepts_inside_the_u() {
        assert!(basket_accepts(0.0, 0.0), "the cavity's middle");
        assert!(basket_accepts(-107.0, -9.0), "just inside the left mouth");
        assert!(basket_accepts(111.0, -9.0), "just inside the right mouth");
        assert!(basket_accepts(0.0, 500.0), "above the rim, for the pile");
        assert!(!basket_accepts(-108.0, 0.0), "on the left mouth's edge");
        assert!(!basket_accepts(112.0, 0.0), "on the right mouth's edge");
        assert!(!basket_accepts(-109.0, 0.0), "outside the left mouth");
        assert!(!basket_accepts(113.0, 0.0), "outside the right mouth");
        assert!(!basket_accepts(0.0, -10.0), "on the floor");
        assert!(!basket_accepts(0.0, -11.0), "below the floor");
    }

    /// The [basket_walls] U encloses its acceptance region: a small body
    /// centered well inside the mouth — at least its radius clear of the
    /// walls' inner faces, the floor, and the walls' tops — clears every
    /// wall, while a body at the left wall's face sits in it.
    #[test]
    fn basket_walls_enclose_the_acceptance_region() {
        for lx in [-103.0, -60.0, 0.0, 60.0, 107.0] {
            for ly in [-5.0, 20.0, 60.0, 95.0] {
                assert!(basket_accepts(lx, ly));
                let body = frost::Collider::Box(frost::OrientedBox::new([lx, ly], [4.0, 4.0]));
                for wall in basket_walls() {
                    assert!(
                        body.push_out(&frost::Collider::Box(wall)).is_none(),
                        "accepted point ({lx}, {ly}) inside a wall"
                    );
                }
            }
        }
        // The left wall's face: inside it.
        let body = frost::Collider::Box(frost::OrientedBox::new([-124.0, 30.0], [4.0, 4.0]));
        let left = frost::Collider::Box(basket_walls()[0]);
        assert!(
            body.push_out(&left).is_some(),
            "the wall's face must collide"
        );
    }
}
