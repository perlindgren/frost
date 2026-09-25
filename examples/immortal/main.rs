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
//!   (`SPRAY_NOZZLE`, in image pixels). The audio output plays the spray
//!   hiss, `assets/audio/Spray.wav`, at 0.5 volume on every burst.
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
//! On the grass, six plant slots sit on the `PLANT_POS` root joints, and
//! the bench starts bare: no plant is growing, but the basket holds one
//! ripe tomato. A left click picks a tomato up — out of the basket, or
//! off any plant's ripe fruit. A release inside the basket's walls drops
//! the tomato in the basket, and a plant fruit released anywhere else
//! snaps back to the slot it grew from. A basket tomato released anywhere
//! but inside the basket's walls is planted: the seed flies over
//! `FLY_TIME` real seconds to the nearest not-yet-planted slot's root
//! anchor, is consumed on arrival, and that slot's plant starts growing
//! from a fresh, full-water seed; a release with every slot already
//! planted sends the tomato flying back to the spot it was picked up from
//! in the basket. The planted plants grow side by side — every planted
//! slot grows at once, each with the same slice-chain construction and
//! travelling wind as the `grow` example, reused through the [`plant`]
//! module, but with five slices — the base grows over 3 seconds, the low
//! middle over 6, the middle over 9, the high middle over 12, the top over
//! 15 — concurrently with the tool system. Each root joint stays glued to
//! its own `grass.png` pixel across resizes. A planted plant whose water
//! reserve runs dry withers back toward zero, and one that withers
//! completely is gone — no slice, no bloom, nothing visible — its slot
//! freed for the next seed. Once a slice is
//! fully grown, its blooms grow — one flower at a time: the first starts
//! the frame the slice finishes, and each next starts a random 2 to 5
//! seconds after the previous one, until every flower on the slice is
//! growing: at each of the four lower slices' hand-picked spawn points —
//! the top slice bears none — a flower grows from zero to full size over
//! 10 seconds from its start, its `assets/sprites/flower.png`
//! tinted light green to yellow; once the flower is fully grown, a tomato
//! grows out of the same point over 10 seconds — the white body of
//! `assets/sprites/tomato.png` tinted dark green to red, at one third of
//! its natural size, with the dark calyx and stem of
//! `assets/sprites/tomato_fg.png` drawn on top, the body's top pinned to the
//! flower's center so the fruit hangs below it, on top of the flower.
//! Each bloom rides its slice's transform so it sways with the plant. A
//! fully grown fruit stales on its plant's aging clock, which the process
//! steps every frame at the growth's slowed pace — water or not: 8 seconds
//! after full growth, its body modulates from red to a dark red over 8
//! seconds, even on a dry plant whose growth clock withers backward —
//! and a
//! picked fruit carries the color it had at pick, frozen while it rides
//! the cursor and kept when it lands in the basket.
//!
//! Every planted plant keeps a water reserve, full when its seed lands,
//! and the planted plants' growth runs at `1 / GROW_SLOWDOWN` of real
//! time's pace — the slices, the flowers, and the tomatoes alike, all
//! planted plants at once: while a plant is growing —
//! planted, not yet complete, its slices or its blooms still growing —
//! its growth clock is stepped forward by `dt / GROW_SLOWDOWN`, and its
//! reserve drains over `DRAIN_TIME` of that slowed clock —
//! `GROW_SLOWDOWN * DRAIN_TIME` real seconds — and growth proceeds only
//! while the reserve holds; a plant whose reserve runs dry withers
//! instead: its growth clock runs backward at half the growth's pace,
//! the slices, the flowers, and the fruit shrinking back together, and
//! the plant's node modulates from white to yellow over
//! `DRY_YELLOW_TIME` real seconds, all the way to a bare seed — the
//! clock runs freely past any ripe fruit, which keeps waiting and staling
//! on the aging clock and overgrows and drops as usual — until the plant
//! is gone and its slot frees up; the withering stops the moment the
//! reserve holds water again, the clock runs forward, and the node
//! rewhitens over `DRY_WHITE_TIME` — a plant dry for 3 seconds is green
//! again 1.5 seconds after it is watered.
//! `DRAIN_TIME` is chosen so a freshly planted, fully watered seed runs
//! dry just before its base slice is fully grown — the base grows over 3
//! growth-clock seconds, the shortest of the five — so every planted plant
//! pauses before its flowers can start developing. A drop that passes
//! through a growing plant's rough hitbox — a `ROOT_RADIUS`-pixel circle
//! around the root joint — restores the reserve by `DROP_WATER`; a full
//! second of pouring, if every drop lands, is one full reserve, whatever
//! pace the growth runs at. A blue fill on a dark background, `BAR_DX`
//! wide, floats `BAR_LIFT` pixels above the root joint of every plant
//! that has started growing and is not complete yet, showing its
//! reserve; a dry reserve blinks the bar's background red — on and off
//! in square halves of `DRY_BLINK` real seconds — so a withering
//! plant's meter draws the eye.
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
//! Every time the mist takes a bug's health to zero, the bug dies: the
//! demo plays the bug death clip, `assets/audio/bugsDeath.wav`, through
//! [`frost::Audio`].
//!
//! While no plant is living on the bench — at start, and, once the player
//! has planted, whenever the last survivor withers away — a game-over
//! overlay covers the window: a dim veil over the whole window, "Game
//! Over" in large letters above the center, and a Play button below it,
//! the label `assets/fonts/Leofont-Regular.ttf` set in. Pressing the
//! button — a left click on it — restarts the game: the bench goes bare
//! again, the basket back to its starting tomato, the swarms empty, and
//! the tools back in their slots, and the overlay goes down.
//!
//! The cursor position comes from [`frost::Context::mouse_position`]. Run
//! with:
//!
//! ```text
//! cargo run --example immortal
//! ```

mod assets_load;
mod basket;
mod bugs;
mod plant;
mod tomato;
mod vipers;

use assets_load::{Assets, Sounds};

/// The window's inner size in logical pixels, via `Config::window_size`.
/// The grass photo is exactly this size, so it fills the window 1:1.
const WINDOW: [u32; 2] = [1920, 1080];

/// `grass.png`'s texture size in pixels: a full-bleed 1920x1080 photo.
const GRASS_SIZE: [f32; 2] = [1920.0, 1080.0];

/// The plants' root joints in `grass.png`'s pixel space — `(0, 0)` at the
/// upper-left, `x` right, `y` down — in slot order: the bench starts bare,
/// and a seed dropped on the bench flies to the nearest not-yet-planted
/// slot and starts its plant growing there.
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

/// The scene root's children, in draw order — the order in which `main`
/// builds them in the scene: the grass underlay, the items panel, the
/// plants group, the fallen-fruit container, the held-items panel, the
/// vipers group, the bugs group, the active tool, the basket, the
/// immortality badge, the carried fruit, and the game-over overlay. The
/// root node itself is the dark ground background.
const CHILD_GRASS: usize = 0;
const CHILD_ITEMS: usize = 1;
const CHILD_PLANTS: usize = 2;
const CHILD_FALLEN_FRUIT: usize = 3;
const CHILD_HELD: usize = 4;
const CHILD_VIPERS: usize = 5;
const CHILD_BUGS: usize = 6;
const CHILD_TOOL: usize = 7;
const CHILD_BASKET: usize = 8;
const CHILD_BADGE: usize = 9;
const CHILD_HELD_FRUIT: usize = 10;
const CHILD_OVERLAY: usize = 11;

/// The game-over overlay node's children, in draw order: the "Game Over"
/// title text, the Play button's rectangle, and the button's label. The
/// dimming veil is the overlay node's own shape, not a child.
const OVERLAY_TITLE: usize = 0;
const OVERLAY_BUTTON: usize = 1;
const OVERLAY_LABEL: usize = 2;

/// The overlay's dimming veil's color: a dark, semi-transparent wash over
/// the whole window.
const OVERLAY_DIM: frost::Color = frost::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.55,
};

/// The overlay's "Game Over" title's size, in pixels.
const OVERLAY_TITLE_SIZE: f32 = 120.0;

/// The overlay's title's center's height, in window pixels above the
/// window's center.
const OVERLAY_TITLE_Y: f32 = 80.0;

/// The Play button's rectangle's half extents, in window pixels: a
/// 360 by 110 button.
const PLAY_HALF: [f32; 2] = [180.0, 55.0];

/// The Play button's rectangle's color: a tomato red under the label.
const PLAY_COLOR: frost::Color = frost::Color {
    r: 0.62,
    g: 0.13,
    b: 0.1,
    a: 1.0,
};

/// The Play button's center's height, in window pixels below the window's
/// center.
const PLAY_Y: f32 = -60.0;

/// The Play button's "Play" label's size, in pixels.
const PLAY_LABEL_SIZE: f32 = 54.0;

/// Whether the user-space point `p` is inside the Play button's
/// rectangle: the button's half extents, centered on
/// `[0.0, PLAY_Y]`.
fn on_play_button(p: [f32; 2]) -> bool {
    p[0].abs() <= PLAY_HALF[0] && (p[1] - PLAY_Y).abs() <= PLAY_HALF[1]
}

/// Whether the game-over overlay should be up over the bench: no plant is
/// planted at all — the bench bare, at start and right after the last
/// survivor has withered away.
fn bench_is_bare(plants: &[WateredPlant]) -> bool {
    !plants.iter().any(|p| p.planted)
}

/// Whether the game-over overlay is due over a bench the player has been
/// playing: the bench is bare again — the last survivor withered away —
/// after at least one plant has been planted since the last restart. A
/// bench that never had a plant planted — the launch state and the state
/// right after a Play press — is not a game over.
fn game_over_due(plants: &[WateredPlant], ever_planted: bool) -> bool {
    ever_planted && bench_is_bare(plants)
}

/// The plant's fit scale: the full plant spans about 1797 px around the
/// root joint — its top edge 1614 px above it, its bottom edge 183 px
/// below — so at this scale the tops can reach past the window's top edge
/// on the plants anchored high on the grass.
const PLANT_SCALE: f32 = 0.45;

/// The fall speed of a dropped, overgrown tomato, in user pixels per
/// second: it drops from the point it let go, at this constant speed, along
/// the straight line to its plant's root anchor — the bottom joint of its
/// root segment, where it hits the ground — closing in on it in both x and
/// y until its body center sits on the anchor.
const FALL_SPEED: f32 = 400.0;

/// The roll speed of a landed, overgrown tomato, in user pixels per
/// second: once the fall reaches the ground, the fruit picks one of the six
/// plant anchors and rolls there, along the straight line between, at this
/// constant pace.
const ROLL_SPEED: f32 = 150.0;

/// The range, in seconds, for the wait between two of a rolling tomato's
/// bumps: after each bump — and after the roll starts — the next one is
/// armed a random interval in `[BUMP_IN.0, BUMP_IN.1)` later, so the hops
/// come at an uneven, off-cadence, fairly rapid rattle.
const BUMP_IN: (f32, f32) = (0.08, 0.3);

/// The duration of a single bump, in seconds: the hop lifts the fruit off
/// its rolling line and sets it back down over this many seconds, shaped
/// as a half-sine.
const BUMP_TIME: f32 = 0.1;

/// The height of a bump's hop, in user pixels: the peak of the half-sine,
/// the farthest the fruit lifts above its rolling line.
const BUMP_HEIGHT: f32 = 3.0;

/// The sentinel for [`Fall::bump`]: no bump is running, so the next one is
/// being counted down by [`Fall::bump_in`] instead.
const BUMP_NONE: f32 = f32::NEG_INFINITY;

/// The range, in seconds, for how long a tomato that has reached its
/// target rests there, catching its breath with a squash-and-stretch,
/// before it picks its next plant anchor to roll to.
const REST: (f32, f32) = (1.5, 3.0);

/// The period of a resting tomato's breathing, in seconds: one full cycle
/// — a squash out and a stretch back in — takes this long.
const BREATH_PERIOD: f32 = 2.0;

/// The amplitude of a resting tomato's breathing: the y-scale factor
/// swings between `1.0 + BREATH_AMP` (stretched in) and `1.0 - BREATH_AMP`
/// (squashed out), the x-scale compensating to hold the area.
const BREATH_AMP: f32 = 0.12;

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

/// The drop color; the batched draw scales each drop's alpha by its
/// remaining life fraction.
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
/// process drains a planted plant that is not yet complete by
/// `dt / (DRAIN_TIME * GROW_SLOWDOWN)` per frame — the drain runs at the
/// growth's slowed pace — and the span is chosen so a freshly planted,
/// fully watered seed runs dry just before its base slice is fully grown:
/// the base grows over 3 growth-clock seconds (`plant::GROW_TIMES[0]`),
/// i.e. 9 real seconds at `GROW_SLOWDOWN`, and `DRAIN_TIME * GROW_SLOWDOWN`
/// is 8.25 real seconds, so the plant pauses 0.75 seconds before the base
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

/// The red the water bar's background blinks with while its plant's
/// reserve is dry (0.0): the bar's border flashes on and off in square
/// halves of `DRY_BLINK` real seconds, so a withering plant's meter
/// draws the eye and tells the player the plant needs water.
const DRY_RED: frost::Color = frost::Color {
    r: 0.9,
    g: 0.22,
    b: 0.2,
    a: 0.9,
};

/// One full cycle of the dry water bar's border blink, in real seconds:
/// the border is lit for the first half of each period, dark for the
/// second — two flashes per second at 0.5.
const DRY_BLINK: f32 = 0.5;

/// The real seconds a dry plant's withering takes to yellow: its node's
/// modulate modulates from white to [DRY_YELLOW] over that span.
const DRY_YELLOW_TIME: f32 = 6.0;

/// The real seconds a watered plant takes to modulate back to white:
/// twice as fast as the yellowing, so a plant dry for half a yellowing —
/// 3 seconds — is green again 1.5 seconds after it is watered.
const DRY_WHITE_TIME: f32 = 3.0;

/// The white a watered plant's node modulates with — its original,
/// healthy color; the layout lerps the plant's modulate from it to
/// [DRY_YELLOW] over the plant's dryness.
const PLANT_WHITE: frost::Color = frost::Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};

/// The yellow a dry plant's node modulates toward — the withering tint
/// that yellows the slices, the flowers, and the fruit alike, the whole
/// tree yellowing as its growth runs backward.
const DRY_YELLOW: frost::Color = frost::Color {
    r: 1.0,
    g: 0.78,
    b: 0.25,
    a: 1.0,
};

/// Whether the dry plant's water bar border is lit at `time`: the square
/// blink — on for the first half of every `DRY_BLINK` period, off for
/// the second.
fn dry_blink_on(time: f32) -> bool {
    (time % DRY_BLINK) < DRY_BLINK * 0.5
}

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

/// `sustainable_immortality.png`'s texture size in pixels; the corner
/// badge is drawn at `IMMORTALITY_SCALE` of this natural size, `MARGIN`
/// clear of the window's top and right borders.
const IMMORTALITY_SIZE: [f32; 2] = [536.0, 548.0];

/// The badge's scale relative to its natural size: half of it.
const IMMORTALITY_SCALE: f32 = 0.5;

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

/// The uniform scale a picked tomato rides at: the plant's fit scale on
/// the tomato's full growth, so a picked fruit is exactly the size it had
/// on the plant.
const TOMATO_PICK_SCALE: f32 = tomato::TOMATO_MAX_SCALE * PLANT_SCALE;

/// The carried tomato's body box's half extents, in window pixels, for a
/// `size`-pixel image: half the fruit's natural size at the pick scale.
fn tomato_pick_half(size: [f32; 2]) -> [f32; 2] {
    [
        size[0] * TOMATO_PICK_SCALE / 2.0,
        size[1] * TOMATO_PICK_SCALE / 2.0,
    ]
}

/// The seed tomato's flight time, in real seconds: the tween from its
/// release point to the nearest not-yet-planted slot's root anchor — or,
/// with every slot planted, back to its original spot in the basket.
const FLY_TIME: f32 = 0.4;

/// The starting tomato's body center in the basket's local space — the
/// basket node's origin at the sprite's center, y up: a ripe fruit
/// resting on the basket's floor — its body's bottom on the floor's top,
/// `basket::BASKET_FLOOR` plus the body's half height at the basket's fit
/// — mid-width.
const BASKET_SEED: [f32; 2] = [0.0, 23.5];

/// The plants' root joints in user space for a `w` by `h` window: each
/// `PLANT_POS` pixel mapped the way the grass stretch maps the whole
/// texture — the same mapping the chrome layout uses every frame — so a
/// dropped seed flies to the anchor the plant will grow from.
fn plant_anchors(w: f32, h: f32) -> [[f32; 2]; PLANT_POS.len()] {
    std::array::from_fn(|i| {
        let pos = PLANT_POS[i];
        [
            pos[0] * w / GRASS_SIZE[0] - w / 2.0,
            h / 2.0 - pos[1] * h / GRASS_SIZE[1],
        ]
    })
}

/// The squared distance between two user-space points.
fn dist2(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1])
}

/// The nearest not-yet-planted plant slot to the user-space point `p`:
/// the slot whose `PLANT_POS` anchor — mapped for the `w` by `h` window —
/// is closest, by squared distance; `None` when every slot is planted,
/// which sends a dropped seed back to the basket instead.
fn nearest_free_slot(p: [f32; 2], w: f32, h: f32, planted: &[bool; PLANT_POS.len()]) -> Option<usize> {
    let anchors = plant_anchors(w, h);
    (0..PLANT_POS.len())
        .filter(|&i| !planted[i])
        .min_by(|a, b| dist2(p, anchors[*a]).total_cmp(&dist2(p, anchors[*b])))
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

/// The spray's color; the batched draw scales each drop's alpha by its
/// remaining life fraction.
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
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tool {
    /// The watering can: hold the left button to tilt it and pour water.
    WaterCan,
    /// The spray can: a fresh left-button press triggers a green burst.
    SprayCan,
}

/// The phase a fallen overgrown tomato is in.
///
/// A tomato falls straight down to the ground ([`FallPhase::Falling`]),
/// then rolls to one of the six plant anchors ([`FallPhase::Rolling`]),
/// rests there catching its breath with a squash-and-stretch
/// ([`FallPhase::Resting`]), and then picks another anchor to roll to —
/// looping, until the game restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FallPhase {
    /// The tomato is falling straight down to the ground.
    Falling,
    /// The tomato has landed and is rolling to a target plant anchor.
    Rolling,
    /// The tomato has reached its target and is resting (breathing) there.
    Resting,
}

/// One overgrown tomato on the ground: its pivot — a child of the
/// fallen-fruit container, in the same order as its [Demo::falls] entry —
/// is laid out from this entry's body position and scale factors every
/// frame by [`Demo::step_falls`].
///
/// The tomato's *body center* — the fruit's visual center, which hangs
/// below its stem (the pivot's origin) — is what is tracked here and what
/// the roll, the bumps, and the rest are measured from. The pivot's
/// transform is derived from it, so the breathing squash pivots on the
/// body's center, not the stem.
struct Fall {
    /// The phase the tomato is in.
    phase: FallPhase,
    /// The body center's position, in user space, where the fall stops:
    /// the bottom anchor of the plant's root segment, the root joint, where
    /// the fruit hits the ground — targeted in both x and y.
    land_pos: [f32; 2],
    /// Whether the drop clip has played for this fruit; it plays exactly
    /// once, on the frame the fall reaches the ground.
    landed: bool,
    /// The body center's position, in user space: the base position,
    /// without the bump's hop.
    body: [f32; 2],
    /// The target plant anchor, in user space, the fruit is rolling to.
    target: [f32; 2],
    /// The body position the tomato started its current roll from.
    start: [f32; 2],
    /// The roll progress, 0.0 (at `start`) to 1.0 (at `target`).
    progress: f32,
    /// The straight-line distance from `start` to `target`, in user
    /// pixels: the roll's progress is advanced by `ROLL_SPEED * dt` over
    /// it.
    distance: f32,
    /// The running bump's progress, 0.0 (take-off) to 1.0 (back on the
    /// line), or [`BUMP_NONE`] while no bump is running.
    bump: f32,
    /// The seconds left before the next bump arms, while one is not
    /// running.
    bump_in: f32,
    /// The tomato's wheel rotation, in radians, accumulated as it rolls:
    /// the fruit turns like a wheel, the angle advancing with the distance
    /// it travels, so it reads as rolling rather than sliding. It is
    /// zeroed whenever the fruit is not rolling, so it stands upright
    /// while it falls and rests.
    spin: f32,
    /// The seconds the tomato has spent resting at its target.
    rest: f32,
    /// How long this rest lasts, in seconds, before the next roll starts.
    rest_for: f32,
}

impl Fall {
    /// A freshly dropped tomato: it starts in the falling phase, with the
    /// body at the spawn position, heading for the root anchor, no roll
    /// running, no bump armed, and standing upright (no wheel rotation).
    fn new(land_pos: [f32; 2], body: [f32; 2]) -> Self {
        Self {
            phase: FallPhase::Falling,
            land_pos,
            landed: false,
            body,
            target: [0.0, 0.0],
            start: [0.0, 0.0],
            progress: 0.0,
            distance: 0.0,
            bump: BUMP_NONE,
            bump_in: 0.0,
            spin: 0.0,
            rest: 0.0,
            rest_for: 0.0,
        }
    }

    /// Picks a random plant anchor — one that is not the tomato's current
    /// spot, so the roll always has somewhere to go — and starts rolling
    /// there: the roll restarts from the current body position, with a
    /// fresh bump countdown.
    fn start_roll(&mut self, rng: &mut frost::Rng, anchors: &[[f32; 2]; PLANT_POS.len()]) {
        let from = self.body;
        // Walk the anchor ring until one is far enough away; the ring is
        // only six, so a full pass always finds a distinct one.
        let mut i = rng.next_u64() as usize % anchors.len();
        let mut pass = 0;
        while dist2(anchors[i], from) < 1.0 && pass < anchors.len() {
            i = (i + 1) % anchors.len();
            pass += 1;
        }
        self.target = anchors[i];
        self.start = from;
        self.progress = 0.0;
        self.distance = dist2(self.start, self.target).sqrt();
        self.bump_in = rng.in_range(BUMP_IN.0, BUMP_IN.1);
        self.bump = BUMP_NONE;
    }

    /// Advances the tomato one frame by `dt` seconds, driving the phase
    /// forward: the fall closes in on the root anchor in both x and y and
    /// the first roll starts on landing, the roll runs its bumps, turns
    /// like a wheel, and reaches its target, and the rest breathes out and
    /// the next roll starts. `radius` is the fruit's rolling radius, in
    /// user pixels — the wheel's spin turns up by the distance traveled
    /// over it.
    fn step(
        &mut self,
        dt: f32,
        rng: &mut frost::Rng,
        anchors: &[[f32; 2]; PLANT_POS.len()],
        radius: f32,
    ) {
        match self.phase {
            // The body closes in on the root anchor at the fall speed,
            // along the straight line between, in both x and y; on the
            // landing frame the first roll starts.
            FallPhase::Falling => {
                let dx = self.land_pos[0] - self.body[0];
                let dy = self.land_pos[1] - self.body[1];
                let dist = (dx * dx + dy * dy).sqrt();
                let step_len = FALL_SPEED * dt;
                if dist <= step_len || dist < 1e-6 {
                    self.body = self.land_pos;
                    self.landed = true;
                    self.phase = FallPhase::Rolling;
                    self.start_roll(rng, anchors);
                } else {
                    self.body[0] += dx / dist * step_len;
                    self.body[1] += dy / dist * step_len;
                }
            }
            // The body rolls along the straight line from `start` to
            // `target` at the roll speed, hopping on its bumps and turning
            // like a wheel; on the arrival frame the rest begins and the
            // fruit stands back upright.
            FallPhase::Rolling => {
                if self.distance > 0.0 {
                    self.progress = (self.progress + ROLL_SPEED * dt / self.distance).min(1.0);
                } else {
                    self.progress = 1.0;
                }
                let from = self.body;
                self.body = [
                    self.start[0] + (self.target[0] - self.start[0]) * self.progress,
                    self.start[1] + (self.target[1] - self.start[1]) * self.progress,
                ];
                // The wheel turns up with the distance it travels: a
                // no-slip roll, the angle advancing by the travel over the
                // radius.
                if radius > 0.0 {
                    self.spin += dist2(from, self.body).sqrt() / radius;
                }
                // The bump's half-sine hop runs for `BUMP_TIME`, then the
                // next one is counted down from a fresh random interval.
                if self.bump != BUMP_NONE {
                    self.bump = (self.bump + dt / BUMP_TIME).min(1.0);
                    if self.bump >= 1.0 {
                        self.bump = BUMP_NONE;
                    }
                } else {
                    self.bump_in -= dt;
                    if self.bump_in <= 0.0 {
                        self.bump = 0.0;
                        self.bump_in = rng.in_range(BUMP_IN.0, BUMP_IN.1);
                    }
                }
                if self.progress >= 1.0 {
                    self.phase = FallPhase::Resting;
                    self.body = self.target;
                    self.spin = 0.0;
                    self.rest = 0.0;
                    self.rest_for = rng.in_range(REST.0, REST.1);
                }
            }
            // The body holds at the target, breathing upright, until the
            // rest runs out; then the next roll starts.
            FallPhase::Resting => {
                self.rest += dt;
                if self.rest >= self.rest_for {
                    self.phase = FallPhase::Rolling;
                    self.start_roll(rng, anchors);
                }
            }
        }
    }

    /// The body center's position, with the bump's hop added while a bump
    /// is running: the half-sine lifts the fruit off its rolling line and
    /// sets it back down.
    fn body_position(&self) -> [f32; 2] {
        if self.phase == FallPhase::Rolling && self.bump != BUMP_NONE {
            [
                self.body[0],
                self.body[1] + BUMP_HEIGHT * (std::f32::consts::PI * self.bump).sin(),
            ]
        } else {
            self.body
        }
    }

    /// The pivot's scale factors for this frame: neutral (no squash) while
    /// the fruit falls or rolls, and the breathing squash-and-stretch
    /// while it rests — the y-scale swings in and out, the x-scale
    /// compensating to hold the fruit's area.
    fn scale_factors(&self) -> [f32; 2] {
        if self.phase == FallPhase::Resting {
            let sy =
                1.0 + BREATH_AMP * (2.0 * std::f32::consts::PI * self.rest / BREATH_PERIOD).sin();
            [1.0 / sy, sy]
        } else {
            [1.0, 1.0]
        }
    }
}

/// The full destination of a falling overgrown tomato, in user space: the
/// bottom anchor of its plant's root segment — the plant's root joint, the
/// plant node's own origin — the point the fruit's body center drops to, in
/// both x and y. The base rock pivots around that joint, so the sway leaves
/// it fixed and the anchor is the node's origin mapped through its
/// transform.
fn root_anchor(plant: &frost::SceneNode) -> [f32; 2] {
    plant.transform.apply([0.0, 0.0])
}

/// A tomato plant and its water reserve, kept together: the growth clock
/// is paced by the reserve — the growth runs at its slowed pace while the
/// reserve holds, and withers backward at half that pace toward the point
/// of full ripening while it is dry — the reserve refills from the drops
/// that fall into the plant's root hitbox, and the dryness yellows the
/// plant's node while the reserve is dry.
struct WateredPlant {
    /// Whether a seed has landed in this slot and its plant is growing —
    /// or, once planted, has not withered completely: until a seed lands,
    /// the slot's plant stays a fresh, invisible seed at clock zero with
    /// its full reserve, and a plant withered all the way to zero is gone
    /// again, its slot freed and this flag cleared.
    planted: bool,
    /// The plant's growth state: the slices' growth, the blooms, and the
    /// aging clock.
    plant: plant::Plant,
    /// The water reserve, 1.0 (well watered) to 0.0 (dry): drained by
    /// `dt / (DRAIN_TIME * GROW_SLOWDOWN)` per frame while the plant
    /// grows, restored by `DROP_WATER` per drop that falls into its root
    /// hitbox; at 0.0 the growth withers — the clock runs backward at
    /// half the growth's pace toward the point of full ripening, where it
    /// holds — awaiting water; the ripe tomatoes' wait and stale run on
    /// the aging clock, stepped with or without water.
    water: f32,
    /// The dryness, 0.0 (white — watered) to 1.0 (full yellow — withered):
    /// a dry plant climbs it over `DRY_YELLOW_TIME` real seconds, a
    /// watered one falls it over `DRY_WHITE_TIME`, and the layout lerps
    /// the plant's node modulate from white to `DRY_YELLOW` over it.
    dryness: f32,
}

/// The tomato the mouse is carrying: ripe fruit picked off a plant's slot,
/// or a tomato picked up out of the basket.
enum Pick {
    /// Ripe fruit picked off plant `0`'s bloom slot `1`: the slot is
    /// marked harvested while the fruit rides the cursor; the release
    /// keeps it in the basket (the slot regrows) or sends it back to the
    /// slot (the harvest clears).
    Plant(usize, usize),
    /// A tomato picked up out of the basket: `0` is its transform in the
    /// basket's local space — the spot it was picked from, where a release
    /// that does not plant flies back to it.
    Basket(frost::Transform),
}

/// A seed tomato in flight: released outside the basket, it tweens from
/// its release point — the fruit's body center, in user space — to its
/// landing over `FLY_TIME` real seconds. While it flies, the fruit's
/// pivot stays in the held-fruit node, posed every frame by
/// [`Demo::step_flight`]; on arrival a slot landing consumes the seed and
/// starts the slot's plant, a basket landing re-enters the basket.
#[derive(Clone, Copy)]
struct Fly {
    /// The body-center tween in user space, `Repeat::Once`: it clamps at
    /// its destination, so the last frame poses the fruit exactly on the
    /// landing.
    tween: frost::Tween<[f32; 2]>,
    /// The flight's elapsed time, in real seconds, since the drop: the
    /// landing settles when it reaches `FLY_TIME` — the tween's own
    /// elapsed time is private, so the flight keeps its own clock.
    time: f32,
    /// Where the flight lands.
    landing: Landing,
}

/// The landing of a seed's flight.
#[derive(Clone, Copy)]
enum Landing {
    /// The seed plants into this slot on arrival: the fruit is consumed
    /// and the slot's plant starts growing, full-watered.
    Slot(usize),
    /// Every slot is planted: the fruit flies back to the basket and
    /// re-enters it at this basket-local transform — the spot it was
    /// picked up from.
    Basket(frost::Transform),
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
    /// The tool each slot of the items panel holds at rest, in slot order
    /// (0, the top slot, to `SLOTS - 1`, the bottom one), or `None` for an
    /// empty slot: the slots' sprites are the visual mirror of this table,
    /// laid out by [`slot_set`], so a slot's tool is read from here, never
    /// from its sprite.
    slots: [Option<Tool>; SLOTS],
    /// The slot the current left press started on, if any: a press that
    /// starts on a slot is a swap click, completed only if the release
    /// lands on the same slot.
    press_slot: Option<usize>,
    /// Whether the left mouse button was down on the previous frame; the
    /// press and release edges are derived from it.
    pressed: bool,
    /// The tomato being carried, if any: ripe fruit off a plant, or a
    /// tomato out of the basket. While set, the fruit's pivot rides the
    /// cursor in the held-fruit node (root's [`CHILD_HELD_FRUIT`] child)
    /// on top of everything; the release either keeps a basket fruit in
    /// the basket, sends a plant fruit back to its slot, or plants a
    /// basket fruit on the bench.
    picking: Option<Pick>,
    /// The seed tomato currently in flight, if any: dropped outside the
    /// basket, it tweens to its landing over `FLY_TIME` real seconds,
    /// posed by [`Demo::step_flight`]; no new pick starts while a seed is
    /// flying.
    flying: Option<Fly>,
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
    /// plants' root hitboxes to restore the plants' water reserves.
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
    /// The demo's running clock, in real seconds, from launch: the
    /// process adds `dt` to it every frame, and it paces the dry water
    /// bar's red border blink.
    time: f32,
    /// The tomato plants and their water reserves on the grass, in slot
    /// order: the bench starts bare — no plant planted, every clock at
    /// zero, every reserve full — and a seed landing in a slot plants it.
    /// Every planted plant grows at once: the process steps each planted
    /// plant's clock forward by `dt / GROW_SLOWDOWN` while its water
    /// reserve holds, then lays it out on the matching child of the plants
    /// node (root's [`CHILD_PLANTS`] child), in parallel with the tool
    /// system; the reserve of a planted plant that is not yet complete
    /// drains by `dt / (DRAIN_TIME * GROW_SLOWDOWN)` per frame — at the
    /// growth's slowed pace — and restores by `DROP_WATER` per drop that
    /// falls into its root hitbox, and a plant at 0.0 withers instead:
    /// its growth clock runs backward at half the growth's pace toward
    /// the point of full ripening, where it holds, its ripe tomatoes'
    /// wait and stale running on the plants' aging clocks, stepped with
    /// or without water, while its dryness yellows its node and the dry
    /// water bar's border blinks red; a planted plant that withers all
    /// the way to zero is gone and its slot is freed — the plant resets
    /// to a fresh seed, full-watered and white, un-planted.
    plants: [WateredPlant; PLANT_POS.len()],
    /// The plant's five slice shapes, owned by the demo: every fresh seed
    /// — the launch state and a withered-away plant's reset alike —
    /// rebuilds its [plant::Plant] from them, so a freed slot always
    /// regrows from the same five slices.
    slices: [frost::Shape; 5],
    /// Whether the basket still needs its starting tomato: the seed's
    /// scale depends on the basket's fit, which the chrome layout sets,
    /// so the first frame — after the chrome has laid the basket out —
    /// drops one ripe tomato on the basket's floor and clears the flag.
    basket_seed: bool,
    /// The swarm of vipers buzzing around the flower bench — the row of
    /// plants — in parallel with the tool system and the plants: one
    /// viper per fully grown plant layer, so the swarm grows as the bench
    /// does; stepped every frame with the number of fully grown layers
    /// and the layer-segment midpoints, and laid out on the matching
    /// child of the vipers node (root's [`CHILD_VIPERS`] child).
    vipers: vipers::Vipers,
    /// The swarm of bugs waddling to the plants: three spawn each time a
    /// plant starts growing, and the swarm steps and lays them out on the
    /// matching child of the bugs node (root's [`CHILD_BUGS`] child).
    bugs: bugs::Bugs,
    /// The audio output and every clip the demo plays, decoded once at
    /// startup: the swarm's tjatter, plopp, and Aj arrays, and the
    /// singles — the bug death, the watering loop, the spray hiss, and
    /// the tomato drop — all playing through its device.
    sounds: Sounds,
    /// Whether the audio output's loop is currently playing the pour
    /// sound: the process flips it on the pour's edges — the watering can
    /// active and fully tilted (rising), pouring ending, the can turning
    /// back upright or the tool switching away (falling) — starting the
    /// loop on the rising edge and stopping it on the falling one.
    pouring: bool,
    /// The overgrown tomatoes currently falling to the ground: one entry
    /// per child of the fallen-fruit container, in the same order, added
    /// as the fruits drop and never removed — landed fruits stay where
    /// they fell.
    falls: Vec<Fall>,
    /// Whether the game-over overlay is up: it opens with the demo, comes
    /// back up whenever the bench goes bare after the player has planted
    /// — the last survivor withered away — and comes down when the player
    /// presses the Play button, which also restarts the game. While up,
    /// the input answers only the button, and the process lays the overlay
    /// out over the whole window every frame.
    over: bool,
    /// Whether at least one plant has been planted since the last
    /// restart: the game can only be over after the player has actually
    /// planted, so the bare bench at launch and right after a Play press
    /// never triggers the game-over overlay on its own.
    ever_planted: bool,
    /// The overlay's "Game Over" title, built once from the embedded font;
    /// laid onto the overlay's title child while the overlay is up.
    game_over: frost::Shape,
    /// The Play button's rectangle, built once; laid onto the overlay's
    /// button child while the overlay is up.
    play_button: frost::Shape,
    /// The overlay's "Play" label, built once from the embedded font; laid
    /// onto the overlay's label child while the overlay is up.
    play_label: frost::Shape,
}

impl frost::Process for Demo {
    /// Steps the whole frame, in the order the game's invariants require:
    /// the demo's clock ticks; the chrome fits the window and the
    /// plants' anchors lift off the grass stretch; the basket takes its
    /// starting tomato on the first frame; the planted plants grow on
    /// their water; the vipers and the bugs step, and their arrival
    /// sounds play; the input's edges are handled; the seed tomato's
    /// flight steps and settles; the tomatoes are kept out of the
    /// basket's walls; the plants are laid out; the overgrown fruits drop
    /// and the fallen fruit steps; the tool ticks, emits, and takes its
    /// live pose; the particles advance; the drops water the roots —
    /// after the particles have stepped — and the mist wounds the bugs;
    /// the live overlay is drawn; and the game-over overlay lays out over
    /// the window.
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let (w, h) = ctx.size();
        self.time += dt;
        let anchors = self.layout_chrome(ctx, w, h);

        // The first frame seeds the basket with its starting tomato: the
        // seed's scale depends on the basket's fit, which the chrome
        // layout above just set.
        if self.basket_seed {
            self.basket_seed = false;
            self.seed_basket_tomato(ctx);
        }

        // The bugs step on the planted table itself, not the count, so a
        // withered plant's bugs retarget and a replanted slot's plant gets
        // its own batch; the count is only kept for the tests.
        let (_active_plants, grown_layers, started) = self.grow_plants(dt);

        // The bench went bare after the last survivor withered away —
        // the player has planted, so this is a game over: the overlay
        // rises until the player presses Play, and a pour that was running
        // when the last plant withered stops with the game over. A bench
        // that never had a plant planted — at start and right after a
        // Play press — never game-overs.
        if !self.over && game_over_due(&self.plants, self.ever_planted) {
            self.over = true;
            if self.pouring {
                self.sounds.device.stop_loop();
                self.pouring = false;
            }
        }

        self.step_vipers(ctx, dt, &anchors, grown_layers);

        let events = self.step_bugs(ctx, dt, &anchors, &started);
        self.play_bug_events(&events);
        self.handle_input(ctx);

        // A seed dropped this frame starts flying: the flight poses its
        // pivot in the held-fruit node — before the basket walls push and
        // the plants lay out — and settles it on its landing.
        self.step_flight(ctx, dt);

        self.constrain_basket_fruit(ctx);

        // Lay the plants out, after the input edges, so a tomato that was
        // picked or snapped back this frame is reposed by its plant — and
        // a picked slot's pivot, out of the tree, is simply skipped.
        self.layout_plants(ctx, &anchors);

        self.fall_overgrown_tomatoes(ctx);
        self.step_falls(ctx, dt, &anchors);

        self.tick_tool(ctx, dt);
        self.update_particles(dt);

        // The drops water the roots only after the particles have
        // stepped, so a drop that reaches a root this frame waters it
        // this frame.
        self.water_roots(&anchors, &started);
        self.spray_hits();
        self.draw_live(ctx, &anchors, &started);

        // Lay the game-over overlay out last: its flag may have risen on
        // this frame, above, or fallen in the input's Play press, and the
        // veil must track the window's current size.
        self.layout_overlay(ctx, w, h);
    }
}

impl Demo {
    /// Builds the demo's initial state out of the loaded `assets`: the
    /// mouse holds no tool at all, the tools rest in the items panel's
    /// slots — mirroring the panel's seeded sprites, the spray can in
    /// `SPRAY_SLOT`, the watering can in `CAN_SLOT` — the bench starts
    /// bare: no plant planted, all six growth clocks at zero, all six
    /// water reserves full, and the basket still holding its starting
    /// tomato, seeded on the first frame once the basket's fit is known.
    fn new(assets: Assets) -> Demo {
        // The tools at rest in the items panel's slots, mirroring the
        // panel's seeded sprites: the spray can in SPRAY_SLOT, the
        // watering can in CAN_SLOT, the other two slots empty.
        let mut slots = [None; SLOTS];
        slots[SPRAY_SLOT] = Some(Tool::SprayCan);
        slots[CAN_SLOT] = Some(Tool::WaterCan);

        // The plant's five slice shapes, owned by the demo: the launch
        // plants and every withered-away plant's reset rebuild from them.
        let slices = [
            assets.plant1.clone(),
            assets.plant2.clone(),
            assets.plant3.clone(),
            assets.plant4.clone(),
            assets.plant5.clone(),
        ];
        let plant = plant::Plant::new([
            &slices[0],
            &slices[1],
            &slices[2],
            &slices[3],
            &slices[4],
        ]);

        // The game-over overlay's shapes, built once: the title and the
        // Play label from the embedded font — each an `Arc`-shared copy of
        // the font's bytes — and the button's rectangle.
        let game_over = frost::Shape::text_bytes(
            assets.font,
            "Game Over",
            OVERLAY_TITLE_SIZE,
        )
        .expect("the embedded overlay font decodes");
        let play_label = frost::Shape::text_bytes(assets.font, "Play", PLAY_LABEL_SIZE)
            .expect("the embedded overlay font decodes");
        let play_button = frost::Shape::Rectangle {
            center: [0.0, 0.0],
            extent: PLAY_HALF,
            color: PLAY_COLOR,
        };

        Demo {
            mouse: [0.0, 0.0],
            active: None,
            held: None,
            slots,
            press_slot: None,
            pressed: false,
            picking: None,
            flying: None,
            right_pressed: false,
            angle: 0.0,
            // The rotation tween starts as a 0→0 tween that never moves;
            // the first button event replaces it.
            rotation: frost::Tween::new(0.0, 0.0, 1.0).repeat(frost::Repeat::Once),
            burst: None,
            showing_spray2: false,
            can: assets.can,
            spray1: assets.spray1,
            spray2: assets.spray2,
            viper1: assets.viper1,
            viper2: assets.viper2,
            // Three bugs per plant, each plant's batch exactly once: the
            // population climbs 3, 6, …, 18 over the first 75 seconds; a
            // killed bug pops back up at its spawn spot after a random 5
            // to 10 second delay.
            bugs: bugs::Bugs::new([&assets.bug1, &assets.bug2, &assets.bug3]),
            bug1: assets.bug1,
            bug2: assets.bug2,
            bug3: assets.bug3,
            sounds: assets.sounds,
            pouring: false,
            falls: Vec::new(),
            flower: assets.flower,
            tomato: assets.tomato,
            tomato_fg: assets.tomato_fg,
            water: frost::ParticleSystem::new(),
            spray: frost::ParticleSystem::new(),
            rng: frost::Rng::new(),
            acc: 0.0,
            time: 0.0,
            // Six bare slots, one per root joint: every plant enters as a
            // fresh, invisible seed at clock zero, un-planted, with its
            // reserve full and white — its dryness out — and a seed
            // landing in a slot is what starts its plant growing.
            plants: std::array::from_fn(|_| WateredPlant {
                planted: false,
                plant: plant.clone(),
                water: 1.0,
                dryness: 0.0,
            }),
            slices,
            basket_seed: true,
            vipers: vipers::Vipers::new(VIPER_IMAGE),
            // The game-over overlay opens with the demo: the bench is
            // bare, so the player sees it over the whole window until the
            // first Play press. The bench never had a plant planted, so
            // its bareness is the start state, not a game over.
            over: true,
            ever_planted: false,
            game_over,
            play_button,
            play_label,
        }
    }

    /// Fits the window's chrome — the grass, the items panel, the
    /// held-items panel, the basket, and the immortality badge — into
    /// whatever size the window has, and returns the plants' root joints
    /// in user space, where the grass stretch maps each `PLANT_POS`
    /// pixel: the plants stay glued to them, and the vipers' orbit
    /// centers lift off them.
    fn layout_chrome(
        &mut self,
        ctx: &mut frost::Context,
        w: f32,
        h: f32,
    ) -> [[f32; 2]; PLANT_POS.len()] {
        // Stretch the grass to exactly fill the window, whatever its aspect
        // ratio, so it stays filled across resizes.
        let grass = &mut ctx.scene().root.children[CHILD_GRASS];
        grass.scale = [w / GRASS_SIZE[0], h / GRASS_SIZE[1]];
        // Fit the panel into the window's height with `MARGIN` clear of the
        // top and bottom borders; the x scale follows, keeping the aspect
        // ratio. The cans, riding on its slots as the node's children, stay
        // in place.
        let items = &mut ctx.scene().root.children[CHILD_ITEMS];
        let s = (h - 2.0 * MARGIN) / ITEMS_SIZE[1];
        items.scale = [s, s];
        // Mid left: `MARGIN` clear of the left border, centered vertically.
        items.transform =
            frost::Transform::translate(-(w / 2.0) + MARGIN + ITEMS_SIZE[0] * s / 2.0, 0.0);

        // Bottom right: `MARGIN` clear of the right and bottom borders, at
        // the panel's natural size.
        let held_panel = &mut ctx.scene().root.children[CHILD_HELD];
        held_panel.transform = frost::Transform::translate(
            w / 2.0 - MARGIN - HELD_SIZE[0] * held_panel.scale[0] / 2.0,
            -h / 2.0 + MARGIN + HELD_SIZE[1] * held_panel.scale[1] / 2.0,
        );

        // Bottom left: just right of the items panel — `BASKET_GAP` clear
        // of its right edge — and `MARGIN` clear of the bottom border,
        // scaled with the panel's fit so the basket keeps its proportions.
        let basket = &mut ctx.scene().root.children[CHILD_BASKET];
        let bs = basket::basket_scale(h);
        let bc = basket::basket_center(w, h);
        basket.scale = [bs, bs];
        basket.transform = frost::Transform::translate(bc[0], bc[1]);

        // Top right: `MARGIN` clear of the top and right borders, at
        // `IMMORTALITY_SCALE` of the badge's natural size.
        let badge = &mut ctx.scene().root.children[CHILD_BADGE];
        badge.scale = [IMMORTALITY_SCALE, IMMORTALITY_SCALE];
        badge.transform = frost::Transform::translate(
            w / 2.0 - MARGIN - IMMORTALITY_SIZE[0] * IMMORTALITY_SCALE / 2.0,
            h / 2.0 - MARGIN - IMMORTALITY_SIZE[1] * IMMORTALITY_SCALE / 2.0,
        );

        // The plants' root joints in user space, where the grass stretch
        // maps each `PLANT_POS` pixel: the plants stay glued to them, and
        // the vipers' orbit centers lift off them.
        PLANT_POS.map(|pos| {
            [
                pos[0] * w / GRASS_SIZE[0] - w / 2.0,
                h / 2.0 - pos[1] * h / GRASS_SIZE[1],
            ]
        })
    }

    /// Lays the game-over overlay out on the overlay node (root's
    /// [`CHILD_OVERLAY`] child), for a `w` by `h` window: while the
    /// overlay is up — the bench bare — the node's own shape is a dim
    /// rectangle over the whole window, and its children carry the
    /// "Game Over" title above the center, the Play button's rectangle
    /// below it, and the button's label on the button's center; while the
    /// overlay is down, the node and its children carry no shapes at all.
    fn layout_overlay(&mut self, ctx: &mut frost::Context, w: f32, h: f32) {
        let overlay = &mut ctx.scene().root.children[CHILD_OVERLAY];
        // The children's poses hold while the overlay is up and down
        // alike: the title above the window's center, the button and its
        // label on the button's center below it.
        overlay.children[OVERLAY_TITLE].transform =
            frost::Transform::translate(0.0, OVERLAY_TITLE_Y);
        let button = frost::Transform::translate(0.0, PLAY_Y);
        overlay.children[OVERLAY_BUTTON].transform = button;
        overlay.children[OVERLAY_LABEL].transform = button;
        if self.over {
            // The dim veil: a rectangle over the whole window, a pixel
            // past each edge so no border shows, centered on the node's
            // origin.
            overlay.shape = Some(frost::Shape::Rectangle {
                center: [0.0, 0.0],
                extent: [w / 2.0 + 1.0, h / 2.0 + 1.0],
                color: OVERLAY_DIM,
            });
            overlay.children[OVERLAY_TITLE].shape = Some(self.game_over.clone());
            overlay.children[OVERLAY_BUTTON].shape = Some(self.play_button.clone());
            overlay.children[OVERLAY_LABEL].shape = Some(self.play_label.clone());
        } else {
            overlay.shape = None;
            overlay.children[OVERLAY_TITLE].shape = None;
            overlay.children[OVERLAY_BUTTON].shape = None;
            overlay.children[OVERLAY_LABEL].shape = None;
        }
    }

    /// Grows the planted plants in parallel, in parallel with the tool
    /// system: every planted slot grows at once, and a slot without a
    /// planted seed sits untouched — its fresh plant's clocks and its
    /// full reserve hold until a seed flies in. Growth proceeds only
    /// while the plant is well watered, and at `1 / GROW_SLOWDOWN` of
    /// real time's pace: a planted plant that is not yet complete — its
    /// slices or its blooms still growing — drains its water reserve by
    /// `dt / (DRAIN_TIME * GROW_SLOWDOWN)` every frame — the drain runs
    /// with the growth — and steps its clock forward by
    /// `dt / GROW_SLOWDOWN`, so fully grown slices keep opening blooms
    /// while the water holds; a plant whose reserve runs dry withers
    /// instead — its growth clock runs backward at half the growth's
    /// pace, `dt / (GROW_SLOWDOWN * 2)`, the slices, the flowers, and the
    /// green fruit shrinking back together, all the way to a bare seed —
    /// the clock runs freely past any ripe fruit, which keeps waiting and
    /// staling on the aging clock and overgrows and drops as usual —
    /// while its node yellows over
    /// `DRY_YELLOW_TIME` real seconds, until the player pours water on
    /// its root: the withering stops, the clock runs forward again, and
    /// the node rewhitens over `DRY_WHITE_TIME`; the falling drops are
    /// matched against the roots' hitboxes later in the frame, once the
    /// particles have been stepped. The aging clock, stepped by the same
    /// slowed frame, runs with or without water: a ripe fruit's wait and
    /// stale — `tomato::STALE_DELAY` then `tomato::STALE_TIME` after full
    /// growth — proceed on a dry plant. A planted plant that withers all
    /// the way to zero is gone and its slot is freed: the plant resets to
    /// a fresh, invisible seed — un-planted, full-watered and white — so
    /// a new seed can land in the slot.
    ///
    /// Returns the `grown_layers` count the vipers step on, the
    /// `active_plants` count of plants that are growing, and the `started`
    /// table the watering, the water bars and the bugs read — the planted
    /// table, which is the bugs' liveness table: a plant that withered
    /// away this frame is not in it.
    fn grow_plants(&mut self, dt: f32) -> (usize, usize, [bool; PLANT_POS.len()]) {
        let mut active_plants = 0usize;
        let mut grown_layers = 0usize;
        for i in 0..PLANT_POS.len() {
            if self.plants[i].planted {
                if !self.plants[i].plant.complete() {
                    self.plants[i].water =
                        (self.plants[i].water - dt / (DRAIN_TIME * GROW_SLOWDOWN)).max(0.0);
                }
                // The growth clock runs slow: forward while the reserve
                // is wet, and backward — the withering, at half the
                // growth's pace — while it is dry, the plant shrinking
                // back toward a bare seed, the clock running freely past
                // any ripe fruit. A dry plant yellows over DRY_YELLOW_TIME
                // real seconds, and a watered one rewhitens over
                // DRY_WHITE_TIME; the layout lerps the node's modulate
                // from white to DRY_YELLOW over the dryness.
                if self.plants[i].water > 0.0 {
                    self.plants[i].plant.step(dt / GROW_SLOWDOWN);
                    self.plants[i].dryness =
                        (self.plants[i].dryness - dt / DRY_WHITE_TIME).max(0.0);
                } else {
                    self.plants[i].plant.wither(dt / (GROW_SLOWDOWN * 2.0));
                    self.plants[i].dryness =
                        (self.plants[i].dryness + dt / DRY_YELLOW_TIME).min(1.0);
                }
                // The aging clock runs every frame, water or not: ripe
                // fruits wait and stale on it.
                self.plants[i].plant.age(dt / GROW_SLOWDOWN);
                // A planted plant that has withered completely is gone —
                // no slice, no bloom, nothing visible — and its slot is
                // free: the plant resets to a fresh, invisible seed, so
                // a new seed can land in the slot.
                if self.plants[i].plant.gone() {
                    self.plants[i].planted = false;
                    self.plants[i].plant = plant::Plant::new([
                        &self.slices[0],
                        &self.slices[1],
                        &self.slices[2],
                        &self.slices[3],
                        &self.slices[4],
                    ]);
                    self.plants[i].water = 1.0;
                    self.plants[i].dryness = 0.0;
                }
            }
            // Counted after the reset, so a plant that withered away this
            // frame is not in the count or the liveness table.
            if self.plants[i].planted {
                active_plants += 1;
            }
            grown_layers += self.plants[i].plant.grown_layers();
        }
        (
            active_plants,
            grown_layers,
            std::array::from_fn(|i| self.plants[i].planted),
        )
    }

    /// Buzzes the vipers around the flower bench, in parallel with
    /// everything else: one viper per fully grown layer, so the swarm
    /// grows as the bench does — `grown_layers` is how many exist — and
    /// each viper circles the segment its layer spawned it, at that
    /// segment's midpoint along the static, fully grown chain, the sway
    /// left out, lifted to its plant's anchor at the fit scale.
    fn step_vipers(
        &mut self,
        ctx: &mut frost::Context,
        dt: f32,
        anchors: &[[f32; 2]; PLANT_POS.len()],
        grown_layers: usize,
    ) {
        let vipers_node = &mut ctx.scene().root.children[CHILD_VIPERS];
        let centers: [[[f32; 2]; vipers::LAYERS]; PLANT_POS.len()] = std::array::from_fn(|i| {
            self.plants[i].plant.layer_midpoints().map(|m| {
                [
                    anchors[i][0] + m[0] * PLANT_SCALE,
                    anchors[i][1] + m[1] * PLANT_SCALE,
                ]
            })
        });
        self.vipers.step(dt, &centers, grown_layers);
        self.vipers
            .layout(vipers_node, [&self.viper1, &self.viper2]);
    }

    /// Waddles the bugs to the plants, in parallel with everything else:
    /// `alive` is the bench's planted table — which plants are growing —
    /// and the bugs retarget away from a plant that withers and batch in
    /// for each plant that starts growing. Returns the step's arrival
    /// events, which [Demo::play_bug_events] plays.
    fn step_bugs(
        &mut self,
        ctx: &mut frost::Context,
        dt: f32,
        anchors: &[[f32; 2]; PLANT_POS.len()],
        alive: &[bool],
    ) -> bugs::StepEvents {
        let bugs_node = &mut ctx.scene().root.children[CHILD_BUGS];
        let events = self.bugs.step(dt, anchors, alive);
        self.bugs
            .layout(bugs_node, [&self.bug1, &self.bug2, &self.bug3]);
        events
    }

    /// Plays the bugs' last step's arrival sounds: a bug that just
    /// reached its destination while another bug was on the grass nearby
    /// plays one of the tjatter clips, the swarm's random pick, and a bug
    /// that just came up out of the grass — a batch spawn or a respawn —
    /// plays one of the plopp clips.
    fn play_bug_events(&mut self, events: &bugs::StepEvents) {
        for clip in &events.tjatters {
            self.sounds.device.play_once(&self.sounds.tjatters[*clip], None);
        }
        for clip in &events.plops {
            self.sounds.device.play_once(&self.sounds.plops[*clip], None);
        }
    }

    /// Follows the pointer, keeping the last known position while the
    /// cursor is outside the window, and handles the mouse's edges: a
    /// press that starts on a slot is a swap click, completed only if the
    /// release lands on the same slot; a press that starts elsewhere is a
    /// use of the active tool — the watering can's quarter turn, the
    /// spray can's fresh burst — or a tomato pick — ripe fruit off a
    /// plant, or a tomato out of the basket — when no tool is held at all
    /// and no seed is flying; and a release of the right button switches
    /// the two tools the mouse holds. While the game-over overlay is up,
    /// the input answers only the Play button — a press on it restarts
    /// the game and takes the overlay down — and the game's own input
    /// goes untouched.
    fn handle_input(&mut self, ctx: &mut frost::Context) {
        // Follow the pointer, keeping the last known position while the
        // cursor is outside the window.
        if let Some(pos) = ctx.mouse_position() {
            self.mouse = pos;
        }

        let left = ctx.mouse_button_down(frost::MouseButton::Left);
        let right = ctx.mouse_button_down(frost::MouseButton::Right);

        // The game-over overlay is up: the input answers only the Play
        // button — a press on it restarts the game and takes the overlay
        // down — and the game's own input goes untouched this frame.
        if self.over {
            if left && !self.pressed && on_play_button(self.mouse) {
                self.over = false;
                self.restart(ctx);
            }
            self.pressed = left;
            return;
        }

        // The slot the pointer is over, if any: a left click swaps the
        // active tool with a slot's tool, and the click is recognized by
        // the press and the release both landing on the same slot.
        let items_node = &ctx.scene().root.children[CHILD_ITEMS];
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
            if on_slot.is_none()
                && self.active.is_none()
                && self.picking.is_none()
                && self.flying.is_none()
            {
                // No tool held, no seed flying: pick up a tomato under the
                // pointer — ripe fruit off a plant, or a tomato out of
                // the basket — and carry it.
                let pick = self
                    .pick_tomato(ctx)
                    .map(|(pi, si)| Pick::Plant(pi, si))
                    .or_else(|| self.pick_basket_tomato(ctx));
                if let Some(pick) = pick {
                    self.picking = Some(pick);
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
                    self.sounds.device.play_once(&self.sounds.spray, Some(0.5));
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
            if let Some(pick) = self.picking.take() {
                // The press was a tomato pick: a plant fruit drops into
                // the basket or back onto its slot, a basket fruit drops
                // at its release — kept in the basket, or planted on the
                // bench.
                match pick {
                    Pick::Plant(pi, si) => self.drop_tomato(ctx, pi, si),
                    Pick::Basket(origin) => self.drop_basket_tomato(ctx, origin),
                }
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
    }

    /// Keeps the tomatoes out of the basket's walls: the carried fruit
    /// rides the cursor, pushed out of the basket's U every frame, and
    /// each dropped fruit stays exactly where it was released, none
    /// sitting inside a wall. There is no gravity, and no
    /// tomato-to-tomato collision. The seed's flight, while one is in
    /// flight, poses the held-fruit node's pivot itself, so the ride is
    /// skipped — the flight is brief and stays over the grass.
    fn constrain_basket_fruit(&mut self, ctx: &mut frost::Context) {
        // A carried tomato rides the cursor: the pivot under the pointer,
        // the fruit's top pinned to it, in window-centered user space on
        // top of everything. It is pushed out of the basket's U every
        // frame — the body's square matched against the walls mapped into
        // window space — so the fruit cannot be carried through a wall.
        // There is no gravity, and no collision with the fruit already in
        // the basket.
        if self.picking.is_some() {
            let basket = &ctx.scene().root.children[CHILD_BASKET];
            let s = basket.scale[0];
            let [bx, by] = basket.transform.apply([0.0, 0.0]);
            let [ox, oy] = tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
            let [hx, hy] = tomato_pick_half(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
            let mut body = [
                self.mouse[0] + TOMATO_PICK_SCALE * ox,
                self.mouse[1] + TOMATO_PICK_SCALE * oy,
            ];
            for wall in basket::basket_walls() {
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
            ctx.scene().root.children[CHILD_HELD_FRUIT].children[0].transform =
                frost::Transform::translate(
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
            let basket = &ctx.scene().root.children[CHILD_BASKET];
            let s = basket.scale[0];
            let [bx, by] = basket.transform.apply([0.0, 0.0]);
            let [ox, oy] = tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
            let [hx, hy] = tomato_pick_half(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
            for fruit in
                &mut ctx.scene().root.children[CHILD_BASKET].children[basket::BASKET_FRUIT].children
            {
                let [tx, ty] = fruit.transform.apply([0.0, 0.0]);
                let mut body = [
                    bx + s * tx + TOMATO_PICK_SCALE * ox,
                    by + s * ty + TOMATO_PICK_SCALE * oy,
                ];
                for wall in basket::basket_walls() {
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
    }

    /// Lays the plants out at their anchors: a tomato picked or snapped
    /// back this frame is reposed by its plant, and a picked slot's
    /// pivot, out of the tree, is simply skipped. The node's modulate
    /// carries the plant's dryness — white when watered, yellowing as
    /// it withers — so the whole tree tints with it.
    fn layout_plants(
        &mut self,
        ctx: &mut frost::Context,
        anchors: &[[f32; 2]; PLANT_POS.len()],
    ) {
        let plants_node = &mut ctx.scene().root.children[CHILD_PLANTS];
        for (i, anchor) in anchors.iter().enumerate() {
            let plant_node = &mut plants_node.children[i];
            self.plants[i].plant.layout(
                plant_node,
                *anchor,
                &self.flower,
                &self.tomato,
                &self.tomato_fg,
            );
            // The dryness tint: the modulate lerps from white to yellow
            // over the withering, and back over the watering, so the
            // slices, the flowers, and the fruit yellow with the plant.
            plant_node.modulate = PLANT_WHITE.lerp(DRY_YELLOW, self.plants[i].dryness);
        }
    }

    /// Drops the overgrown tomatoes to the ground: when a slot's fruit
    /// reaches its final dark red — its staleness full, past the ripe
    /// red — the slot's tomato pivot leaves the plant's tree for the
    /// fallen-fruit container and falls straight down from exactly where
    /// it was laid out this frame, while the bloom restarts under the
    /// same semantics as a fruit dropped in the basket: the flower is
    /// removed and regrows from zero, its tomato after it, on the same
    /// water-gated, slowed schedule, which holds the plant's completion
    /// back as it does for a picked fruit.
    fn fall_overgrown_tomatoes(&mut self, ctx: &mut frost::Context) {
        let drops = {
            let plant_nodes = &ctx.scene().root.children[CHILD_PLANTS].children;
            self.plants
                .iter()
                .enumerate()
                .flat_map(|(pi, p)| {
                    // Shared, `Copy` borrows the `move` closure can own
                    // without owning `self`.
                    let plant_node = &plant_nodes[pi];
                    let tomato = &self.tomato;
                    (0..plant::FLOWER_N).filter_map(move |si| {
                        if p.plant.is_harvested(si) || !p.plant.overgrown(si) {
                            return None;
                        }
                        let center = p.plant.tomato_center(
                            si,
                            &plant_node.children[plant::slot_index(si)],
                            tomato,
                        );
                        let world = frost::Transform::scale(PLANT_SCALE, PLANT_SCALE)
                            .compose(&plant_nodes[pi].transform);
                        let spawn = world.apply(center);
                        // The tomato drops to the ground: the bottom anchor
                        // of its plant's root segment, the root joint — not
                        // a height measured from the spawn, which would miss
                        // the sway and the flower's offset above the joint.
                        // It is targeted in both x and y, so the body's
                        // center lands on the anchor itself.
                        let dest = root_anchor(plant_node);
                        Some((pi, si, spawn, dest))
                    })
                })
                .collect::<Vec<_>>()
        };
        if !drops.is_empty() {
            let [ox, oy] =
                tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
            let root = &mut ctx.scene().root;
            for (pi, si, spawn, dest) in drops {
                self.plants[pi].plant.regrow(si);
                // Rehome the pivot — the slot itself stays in the plant's
                // children, as the picked-fruit paths leave it — out of
                // the slot, into the fallen-fruit container, at the pick
                // scale, pinned so the body's center sits exactly where
                // the tomato was laid out this frame.
                let slot =
                    &mut root.children[CHILD_PLANTS].children[pi].children[plant::slot_index(si)];
                let mut pivot = *slot.children.remove(1);
                pivot.scale = [TOMATO_PICK_SCALE, TOMATO_PICK_SCALE];
                pivot.transform = frost::Transform::translate(
                    spawn[0] - TOMATO_PICK_SCALE * ox,
                    spawn[1] - TOMATO_PICK_SCALE * oy,
                );
                // The slot takes a fresh, shapeless tomato pivot; the next
                // layout regrows the flower and the tomato from zero.
                slot.children.push(Box::new(frost::SceneNode {
                    children: vec![
                        Box::new(frost::SceneNode::default()),
                        Box::new(frost::SceneNode::default()),
                    ],
                    ..Default::default()
                }));
                root.children[CHILD_FALLEN_FRUIT]
                    .children
                    .push(Box::new(pivot));
                // The body's center is tracked and drops to the root anchor
                // in both x and y; the pivot's transform is re-derived from
                // it every frame, so the initial transform (pinned to the
                // spawn) is only the first layout.
                self.falls.push(Fall::new(dest, spawn));
            }
        }
    }

    /// Steps the fallen fruit: each dropped tomato drops from where it let
    /// go, at the fall speed, toward its plant's root anchor — closing in
    /// in both x and y — until its body center sits on it; then it rolls to
    /// one of the six plant anchors, turning like a wheel and hopping on
    /// its bumps, rests there catching its breath with a squash-and-stretch,
    /// and picks its next anchor to roll to — looping. The drop clip plays
    /// exactly once, on the landing frame; every pivot is re-laid out from
    /// its body position, scale factors, and wheel rotation each frame, so
    /// the breathing pivots on the body's center and the roll reads as a
    /// turning wheel.
    fn step_falls(
        &mut self,
        ctx: &mut frost::Context,
        dt: f32,
        anchors: &[[f32; 2]; PLANT_POS.len()],
    ) {
        let sprite = self.tomato.sprite_size().unwrap_or([0.0, 0.0]);
        let [ox, oy] = tomato::tomato_leaf_offset(sprite);
        // The wheel's rolling radius, in user pixels: half the fruit's
        // scaled height, so the spin turns up with the distance it travels.
        let radius = TOMATO_PICK_SCALE * sprite[1] / 2.0;
        let fallen = &mut ctx.scene().root.children[CHILD_FALLEN_FRUIT];
        for (fall, node) in self.falls.iter_mut().zip(fallen.children.iter_mut()) {
            let was_falling = fall.phase == FallPhase::Falling;
            fall.step(dt, &mut self.rng, anchors, radius);
            if was_falling && fall.phase != FallPhase::Falling {
                // The fall reached the ground this frame: the drop clip
                // plays exactly once, on this flip.
                self.sounds.device.play_once(&self.sounds.tomato_drop, None);
            }
            // Lay the pivot out from the body position, the scale factors,
            // and the wheel rotation: the body's center sits at the body
            // position, and the fruit turns about it — so the breathing
            // pivots on the body's center and the roll reads as a turning
            // wheel.
            let [sx, sy] = fall.scale_factors();
            let [bx, by] = fall.body_position();
            node.transform = frost::Transform::translate(
                -TOMATO_PICK_SCALE * sx * ox,
                -TOMATO_PICK_SCALE * sy * oy,
            )
            .compose(&frost::Transform::rotate(fall.spin))
            .compose(&frost::Transform::translate(bx, by));
            node.scale = [TOMATO_PICK_SCALE * sx, TOMATO_PICK_SCALE * sy];
        }
    }

    /// Ticks the active tool's live pose every frame so an ongoing
    /// tilt, return, or burst keeps moving; no tool held is a no-op. The
    /// watering can pours at the full tilt, the spray can runs its burst
    /// to `BURST_TIME`, the pour sound starts and stops with the tilt,
    /// and the tool node takes the live transform.
    fn tick_tool(&mut self, ctx: &mut frost::Context, dt: f32) {
        let [mx, my] = self.mouse;

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
                            angle: 0.0,
                            color: frost::Color {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 1.0,
                            },
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
                                    angle: 0.0,
                                    color: frost::Color {
                                        r: 1.0,
                                        g: 1.0,
                                        b: 1.0,
                                        a: 1.0,
                                    },
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
                self.sounds.device.play_loop(&self.sounds.pour);
            } else {
                self.sounds.device.stop_loop();
            }
        }

        // A fresh transform is only needed while the node shows a tool.
        if let Some(tool) = self.active {
            let tool_node = &mut ctx.scene().root.children[CHILD_TOOL];
            tool_node.transform = match tool {
                Tool::WaterCan => can_transform(mx, my, self.angle),
                Tool::SprayCan => spray_transform(mx, my, self.angle),
            };
        }
    }

    /// Advances the particles even while not emitting, so an ongoing
    /// stream keeps falling until it dies out.
    fn update_particles(&mut self, dt: f32) {
        self.water.update(dt, [0.0, -GRAVITY]);
        self.spray.update(dt, [0.0, -SPRAY_GRAVITY]);
    }

    /// Waters the roots: each drop that passes through a started plant's
    /// rough hitbox — a `ROOT_RADIUS`-pixel circle around the root
    /// joint — while the plant is not yet complete restores the plant's
    /// water reserve by `DROP_WATER`, capped at full; the drops keep
    /// falling on through, so one stream can top up several roots at
    /// once.
    fn water_roots(
        &mut self,
        anchors: &[[f32; 2]; PLANT_POS.len()],
        started: &[bool; PLANT_POS.len()],
    ) {
        for p in &self.water.particles {
            for (i, anchor) in anchors.iter().enumerate() {
                if started[i]
                    && !self.plants[i].plant.complete()
                    && self.plants[i].water < 1.0
                    && in_root_hitbox(p.pos, *anchor)
                {
                    self.plants[i].water = (self.plants[i].water + DROP_WATER).min(1.0);
                }
            }
        }
    }

    /// Wounds the bugs the mist touches: each live spray drop hits every
    /// bug within its reach, but a wounded bug takes its next hit only
    /// after its hit cooldown, and a hit that empties the counter starts
    /// the bounce-then-evaporate death. A bug at its park spot regains
    /// one hit of health every 5 seconds, up to six. The water can's
    /// drops never touch the bugs. Every wound cries out on one of the Aj
    /// clips, the swarm's random pick, and every killing blow plays the
    /// bug death clip.
    fn spray_hits(&mut self) {
        for p in &self.spray.particles {
            let hit = self.bugs.hit_at(p.pos);
            for clip in hit.ajs {
                self.sounds.device.play_once(&self.sounds.ajs[clip], Some(0.35));
            }
            for _ in 0..hit.deaths {
                self.sounds.device.play_once(&self.sounds.death, None);
            }
        }
    }

    /// Draws the frame's live overlay: the bugs' health counters, each
    /// particle stream as one batched (instanced) draw, and the water
    /// bars over the roots of every plant that has started growing and
    /// is not complete yet.
    fn draw_live(
        &mut self,
        ctx: &mut frost::Context,
        anchors: &[[f32; 2]; PLANT_POS.len()],
        started: &[bool; PLANT_POS.len()],
    ) {
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

        // Draw each stream as one batched (instanced) draw: every particle
        // is a circle whose alpha is its remaining life fraction, so the
        // streams fade as they fall. The drops and the spray fly free from
        // the moving tool, in the window's user space, so they stay on the
        // (deprecated) immediate particle draw instead of riding a node's
        // transform.
        #[allow(deprecated)]
        {
            ctx.particles(&self.water.particles, DROP, Z);
            ctx.particles(&self.spray.particles, SPRAY, Z);
        }

        // Draw the water bars: a dark background with a blue fill showing
        // the reserve, `BAR_LIFT` pixels above the root joint of every
        // plant that has started growing and is not complete yet — its
        // slices or its blooms still growing — over the root, the spot the
        // player pours on, because at the fit scale a fully grown plant's
        // top reaches the window's top edge and leaves no room above the
        // plant itself. The fill grows from the bar's left edge as the
        // reserve refills, and a dry reserve — the plant withering —
        // blinks the background red: the border flashes on and off in
        // square halves of `DRY_BLINK` real seconds, on the demo's clock.
        for (i, anchor) in anchors.iter().enumerate() {
            if started[i] && !self.plants[i].plant.complete() {
                let level = self.plants[i].water;
                // The dry plant's border blinks red on the demo's clock.
                let bg = if level == 0.0 && dry_blink_on(self.time) {
                    DRY_RED
                } else {
                    BAR_BG
                };
                ctx.rectangle(
                    anchor[0],
                    anchor[1] + BAR_LIFT,
                    BAR_DX + BAR_BORDER,
                    BAR_DY + BAR_BORDER,
                    bg,
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
        let tool_node = &mut ctx.scene().root.children[CHILD_TOOL];
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
        let incoming = self.active;
        let outgoing = self.slots[slot];
        self.slots[slot] = incoming;
        self.set_active(ctx, outgoing);
        self.slot_set(
            &mut ctx.scene().root.children[CHILD_ITEMS].children[slot],
            incoming,
        );
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

    /// Restarts the game from scratch, called when the player presses Play
    /// on the game-over overlay: the bench goes bare again — every slot a
    /// fresh, invisible, full-watered seed — the basket back to its
    /// starting tomato, seeded on the next frame once the basket's fit is
    /// known, the carried and the fallen fruit off the scene, the swarms
    /// empty again, the tools back in their slots, the mouse holding no
    /// tool, no pour loop running, and the demo's clock back at zero.
    fn restart(&mut self, ctx: &mut frost::Context) {
        // The pour loop stops, if it plays.
        if self.pouring {
            self.sounds.device.stop_loop();
            self.pouring = false;
        }
        // The bench goes bare again: every slot a fresh, invisible seed,
        // full-watered and white, un-planted — and never having been
        // planted, so its bareness is the start state, not a game over.
        self.ever_planted = false;
        self.plants = std::array::from_fn(|_| WateredPlant {
            planted: false,
            plant: plant::Plant::new([
                &self.slices[0],
                &self.slices[1],
                &self.slices[2],
                &self.slices[3],
                &self.slices[4],
            ]),
            water: 1.0,
            dryness: 0.0,
        });
        // The swarms start empty again: both their layouts leave their
        // stale children frozen, so the swarms are rebuilt, not just
        // re-stepped.
        self.vipers = vipers::Vipers::new(VIPER_IMAGE);
        self.bugs = bugs::Bugs::new([&self.bug1, &self.bug2, &self.bug3]);
        // The fruit goes back to the basket: the carried and the fallen
        // fruit come off the scene, the basket empties, and the starting
        // tomato re-seeds on the next frame, once the basket's fit is
        // known.
        let root = &mut ctx.scene().root;
        root.children[CHILD_HELD_FRUIT].children.clear();
        root.children[CHILD_FALLEN_FRUIT].children.clear();
        root.children[CHILD_BASKET].children[basket::BASKET_FRUIT].children.clear();
        self.falls.clear();
        self.basket_seed = true;
        // The mouse holds no tool again: the tools rest in their slots,
        // the held panel's cells go empty, and the in-flight states clear.
        let mut slots = [None; SLOTS];
        slots[SPRAY_SLOT] = Some(Tool::SprayCan);
        slots[CAN_SLOT] = Some(Tool::WaterCan);
        self.slots = slots;
        self.held = None;
        self.set_active(ctx, None);
        let items = &mut ctx.scene().root.children[CHILD_ITEMS];
        for (slot, node) in items.children.iter_mut().enumerate() {
            self.slot_set(node, self.slots[slot]);
        }
        self.picking = None;
        self.flying = None;
        self.press_slot = None;
        self.right_pressed = false;
        self.acc = 0.0;
        self.time = 0.0;
        self.rng = frost::Rng::new();
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
        ctx.scene().root.children[CHILD_TOOL].shape = Some(if on {
            self.spray2.clone()
        } else {
            self.spray1.clone()
        });
    }

    /// Mirrors the mouse's two tools into the held-items panel: the active
    /// tool in the left cell, the stored one in the right, each at its
    /// at-rest frame in the held-fit scale — or an empty cell for `None`.
    fn sync_held(&mut self, ctx: &mut frost::Context) {
        let held = &mut ctx.scene().root.children[CHILD_HELD];
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

    /// Picks up the fully developed tomato a plant bears where the pointer
    /// rests — the first hit in plant, then slot, order — the first pick
    /// source tried, ahead of the basket's fruit, and starts carrying it:
    /// the fruit's pivot is reparented out of its plant's slot and into
    /// the held-fruit container (root's [`CHILD_HELD_FRUIT`] child), at
    /// the pick scale, under
    /// the pointer, so it paints above everything. The plant's slot is
    /// marked harvested while it is carried; the pointer must not be on a
    /// slot or holding a tool, and no seed may be in flight, which the
    /// caller checks. Returns the picked plant and slot.
    fn pick_tomato(&mut self, ctx: &mut frost::Context) -> Option<(usize, usize)> {
        // The hit test: every plant's unharvested, ripe slots, their body
        // centers mapped to window space through the plant node's current
        // transform (the sway included), against the pointer's square
        // around the cursor.
        let hit = {
            let plant_nodes = &ctx.scene().root.children[CHILD_PLANTS].children;
            let [hx, hy] = tomato_pick_half(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
            self.plants.iter().enumerate().find_map(|(pi, p)| {
                (0..plant::FLOWER_N)
                    .find(|&si| {
                        if p.plant.is_harvested(si) || !p.plant.ripe(si) {
                            return false;
                        }
                        let center = p.plant.tomato_center(
                            si,
                            &plant_nodes[pi].children[plant::slot_index(si)],
                            &self.tomato,
                        );
                        let world = frost::Transform::scale(PLANT_SCALE, PLANT_SCALE)
                            .compose(&plant_nodes[pi].transform);
                        let [cx, cy] = world.apply(center);
                        (cx - self.mouse[0]).abs() <= hx && (cy - self.mouse[1]).abs() <= hy
                    })
                    .map(|si| (pi, si))
            })
        };
        let (pi, si) = hit?;
        self.plants[pi].plant.harvest(si);
        // Rehome the pivot: out of the slot, into the held-fruit
        // container, at the pick scale under the pointer.
        let root = &mut ctx.scene().root;
        let pivot = root.children[CHILD_PLANTS].children[pi].children[plant::slot_index(si)]
            .children
            .remove(plant::SLOT_TOMATO);
        let mut pivot = *pivot;
        pivot.scale = [TOMATO_PICK_SCALE, TOMATO_PICK_SCALE];
        pivot.transform = frost::Transform::translate(self.mouse[0], self.mouse[1]);
        root.children[CHILD_HELD_FRUIT]
            .children
            .push(Box::new(pivot));
        Some((pi, si))
    }

    /// Drops the carried tomato picked from (plant, slot) `(pi, si)` — the
    /// plant fruit's drop path; a basket fruit's goes through
    /// [`Self::drop_basket_tomato`] instead. With the body's center inside
    /// the basket's U — the mouth's x range and above the floor — the
    /// pivot is reparented into the basket's
    /// fruit container (the [`basket::BASKET_FRUIT`] child of the [`CHILD_BASKET`]
    /// group), so it renders behind the front half and in front of the
    /// back, and it stays
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
        let s = basket::basket_scale(h);
        let center = basket::basket_center(w, h);
        let [ox, oy] = tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
        let body = [
            self.mouse[0] + TOMATO_PICK_SCALE * ox,
            self.mouse[1] + TOMATO_PICK_SCALE * oy,
        ];
        let lx = (body[0] - center[0]) / s;
        let ly = (body[1] - center[1]) / s;
        let kept = basket::basket_accepts(lx, ly);

        let root = &mut ctx.scene().root;
        let mut pivot = *root.children[CHILD_HELD_FRUIT].children.remove(0);
        if kept {
            // Scale the pivot into the basket's local space and pin it so
            // the body's center maps back to the release point.
            pivot.scale = [TOMATO_PICK_SCALE / s, TOMATO_PICK_SCALE / s];
            pivot.transform =
                frost::Transform::translate((body[0] - center[0]) / s, (body[1] - center[1]) / s);
            root.children[CHILD_BASKET].children[basket::BASKET_FRUIT]
                .children
                .push(Box::new(pivot));
            // The bloom starts over: the plant records the reset — which
            // re-arms the growth clock, the watering hitbox and the water
            // bar — and the slot takes a fresh, shapeless tomato pivot;
            // the next layout removes the flower and regrows it from
            // zero, the tomato after it.
            self.plants[pi].plant.regrow(si);
            root.children[CHILD_PLANTS].children[pi].children[plant::slot_index(si)]
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
            self.plants[pi].plant.unharvest(si);
            root.children[CHILD_PLANTS].children[pi].children[plant::slot_index(si)]
                .children
                .push(Box::new(pivot));
        }
    }

    /// Picks up the tomato the pointer rests on in the basket — the
    /// topmost hit, the last child of the fruit container, the one drawn
    /// on top of the rest — and starts carrying it: the fruit's pivot is
    /// reparented out of the basket's fruit container and into the
    /// held-fruit container (root's [`CHILD_HELD_FRUIT`] child), at the
    /// pick scale under the pointer, so it paints on top of everything.
    /// Returns the pick — the fruit's transform in the basket's local
    /// space, the spot it was picked from.
    fn pick_basket_tomato(&mut self, ctx: &mut frost::Context) -> Option<Pick> {
        let [hx, hy] = tomato_pick_half(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
        let [ox, oy] = tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
        // The hit test: every fruit's body center — its basket-local
        // pivot position, the leaf offset at the basket's fit, and the
        // basket's origin, mapped into window space — against the
        // pointer's square around the cursor, topmost fruit first.
        let hit = {
            let basket = &ctx.scene().root.children[CHILD_BASKET];
            let s = basket.scale[0];
            let [bx, by] = basket.transform.apply([0.0, 0.0]);
            let fruits = &basket.children[basket::BASKET_FRUIT].children;
            fruits.iter().enumerate().rev().find(|(_, pivot)| {
                let [px, py] = pivot.transform.apply([0.0, 0.0]);
                let [cx, cy] = [
                    bx + s * px + TOMATO_PICK_SCALE * ox,
                    by + s * py + TOMATO_PICK_SCALE * oy,
                ];
                (cx - self.mouse[0]).abs() <= hx && (cy - self.mouse[1]).abs() <= hy
            })
        };
        let (i, _) = hit?;
        let root = &mut ctx.scene().root;
        let basket_fruit = &mut root.children[CHILD_BASKET].children[basket::BASKET_FRUIT];
        let origin = basket_fruit.children[i].transform;
        let mut pivot = *basket_fruit.children.remove(i);
        pivot.scale = [TOMATO_PICK_SCALE, TOMATO_PICK_SCALE];
        pivot.transform = frost::Transform::translate(self.mouse[0], self.mouse[1]);
        root.children[CHILD_HELD_FRUIT].children.insert(0, Box::new(pivot));
        Some(Pick::Basket(origin))
    }

    /// Drops the carried tomato picked out of the basket: a release with
    /// the body's center inside the basket's U — the mouth's x range and
    /// above the floor — keeps the fruit in the basket at the release
    /// point, the same reparent into the fruit container as a plant
    /// fruit's basket drop, without a bloom to regrow. A release anywhere
    /// else is a planting attempt: the seed flies to the nearest
    /// not-yet-planted slot's root anchor, consumed on arrival, the slot's
    /// plant starting from a fresh, full-watered seed — or, with every
    /// slot planted, it flies back to the fruit's original spot in the
    /// basket.
    fn drop_basket_tomato(&mut self, ctx: &mut frost::Context, origin: frost::Transform) {
        let (w, h) = ctx.size();
        let s = basket::basket_scale(h);
        let center = basket::basket_center(w, h);
        let [ox, oy] = tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
        // The body's center in window space, and its position in the
        // basket's local space, where the U is measured.
        let body = [
            self.mouse[0] + TOMATO_PICK_SCALE * ox,
            self.mouse[1] + TOMATO_PICK_SCALE * oy,
        ];
        let lx = (body[0] - center[0]) / s;
        let ly = (body[1] - center[1]) / s;
        if basket::basket_accepts(lx, ly) {
            // Keep the fruit in the basket, at the release point.
            let root = &mut ctx.scene().root;
            let mut pivot = *root.children[CHILD_HELD_FRUIT].children.remove(0);
            pivot.scale = [TOMATO_PICK_SCALE / s, TOMATO_PICK_SCALE / s];
            pivot.transform = frost::Transform::translate(lx, ly);
            root.children[CHILD_BASKET].children[basket::BASKET_FRUIT]
                .children
                .push(Box::new(pivot));
            return;
        }
        // The release is off the basket: a planting attempt. The seed
        // flies to the nearest not-yet-planted slot's anchor — or, with
        // every slot planted, back to the fruit's original spot in the
        // basket, in window space.
        let planted = std::array::from_fn(|i| self.plants[i].planted);
        let (dest, landing) = match nearest_free_slot(body, w, h, &planted) {
            Some(i) => (plant_anchors(w, h)[i], Landing::Slot(i)),
            None => {
                let [px, py] = origin.apply([0.0, 0.0]);
                (
                    [
                        center[0] + s * px + TOMATO_PICK_SCALE * ox,
                        center[1] + s * py + TOMATO_PICK_SCALE * oy,
                    ],
                    Landing::Basket(origin),
                )
            }
        };
        self.flying = Some(Fly {
            tween: frost::Tween::new(body, dest, FLY_TIME).repeat(frost::Repeat::Once),
            time: 0.0,
            landing,
        });
        // The pivot stays in the held-fruit node for the flight;
        // step_flight poses it each frame and settles it on arrival.
    }

    /// Steps the seed tomato's flight, if one is in flight: the body's
    /// center tweens toward the landing over `FLY_TIME` real seconds, the
    /// pivot riding the held-fruit node at the pick scale, and on arrival
    /// the flight settles — a slot landing consumes the seed and starts
    /// the slot's plant, a basket landing re-enters the fruit into the
    /// basket at its original spot.
    fn step_flight(&mut self, ctx: &mut frost::Context, dt: f32) {
        let mut fly = match self.flying.take() {
            Some(fly) => fly,
            None => return,
        };
        fly.time += dt;
        let center = fly.tween.tick(dt);
        let [ox, oy] = tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
        ctx.scene().root.children[CHILD_HELD_FRUIT].children[0].transform =
            frost::Transform::translate(
                center[0] - TOMATO_PICK_SCALE * ox,
                center[1] - TOMATO_PICK_SCALE * oy,
            );
        if fly.time < FLY_TIME {
            self.flying = Some(fly);
            return;
        }
        let landing = fly.landing;
        let root = &mut ctx.scene().root;
        let pivot = *root.children[CHILD_HELD_FRUIT].children.remove(0);
        match landing {
            Landing::Slot(i) => {
                // The seed is consumed: the slot's plant starts growing
                // from a fresh seed, full-watered and white. The player
                // has planted: the bench is a living game now, and the
                // game-over overlay may come up when it goes bare again.
                self.plants[i].planted = true;
                self.plants[i].water = 1.0;
                self.plants[i].dryness = 0.0;
                self.ever_planted = true;
            }
            Landing::Basket(origin) => {
                // Back into the basket, at the fruit's original spot.
                let s = root.children[CHILD_BASKET].scale[0];
                let mut pivot = pivot;
                pivot.scale = [TOMATO_PICK_SCALE / s, TOMATO_PICK_SCALE / s];
                pivot.transform = origin;
                root.children[CHILD_BASKET].children[basket::BASKET_FRUIT]
                    .children
                    .push(Box::new(pivot));
            }
        }
    }

    /// Seeds the basket with its starting tomato — one ripe fruit
    /// resting on the basket's floor, mid-width: a fresh pivot in the
    /// basket's fruit container (the [`basket::BASKET_FRUIT`] child of
    /// the [`CHILD_BASKET`] group), at the basket's fit scale, the body's
    /// center on [`BASKET_SEED`]. The seed's scale depends on the
    /// basket's fit, so it lands on the first frame, after the chrome has
    /// laid the basket out.
    fn seed_basket_tomato(&mut self, ctx: &mut frost::Context) {
        let s = ctx.scene().root.children[CHILD_BASKET].scale[0];
        let [ox, oy] = tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
        let k = TOMATO_PICK_SCALE / s;
        let [cx, cy] = BASKET_SEED;
        ctx.scene().root.children[CHILD_BASKET].children[basket::BASKET_FRUIT]
            .children
            .push(Box::new(frost::SceneNode {
                transform: frost::Transform::translate(cx - k * ox, cy - k * oy),
                scale: [k, k],
                children: vec![
                    Box::new(frost::SceneNode {
                        shape: Some(self.tomato.clone()),
                        transform: frost::Transform::translate(ox, oy),
                        modulate: tomato::TOMATO_RED,
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        shape: Some(self.tomato_fg.clone()),
                        transform: frost::Transform::translate(ox, oy),
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            }));
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    let assets = Assets::load();

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
            shape: Some(assets.plant1.clone()),
            ..Default::default()
        }),
        Box::new(frost::SceneNode {
            shape: Some(assets.plant2.clone()),
            ..Default::default()
        }),
        Box::new(frost::SceneNode {
            shape: Some(assets.plant3.clone()),
            ..Default::default()
        }),
        Box::new(frost::SceneNode {
            shape: Some(assets.plant4.clone()),
            ..Default::default()
        }),
        Box::new(frost::SceneNode {
            shape: Some(assets.plant5.clone()),
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
        // The root's children, in draw order — the `CHILD_*` constants are
        // the indices into this list.
        children: vec![
            Box::new(frost::SceneNode {
                // Stretched to fill the window by the process, every frame.
                shape: Some(assets.grass.clone()),
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
                shape: Some(assets.items.clone()),
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
                        shape: Some(assets.spray1.clone()),
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        // The watering can at rest, centered in slot 3.
                        transform: frost::Transform::translate(
                            slot_local(CAN_SLOT)[0],
                            slot_local(CAN_SLOT)[1],
                        ),
                        scale: [slot_scale(CAN_IMAGE), slot_scale(CAN_IMAGE)],
                        shape: Some(assets.can.clone()),
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
                // The fallen fruit: one child per dropped tomato, in drop
                // order — the overgrown fruit's pivot reparents here when
                // it lets go, and the process lays each fall out on its
                // child until it lands. The group carries no shape or
                // scale of its own; it sits under the held panel, the
                // bees, and the bugs, so the fallen fruit paints on top
                // of the grass and the plants, below everything else.
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
                shape: Some(assets.held_items.clone()),
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
                // frame; its children — the back half, the fruit
                // container, and the front half — are built by
                // `basket_children`.
                children: basket::basket_children(
                    assets.basket_back.clone(),
                    assets.basket_front.clone(),
                )
                    .into_iter()
                    .map(Box::new)
                    .collect(),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The immortality badge in the top right: half its
                // natural size, `MARGIN` clear of the top and right
                // borders, tinted to a faint watermark, positioned by the
                // process every frame. It sits under the held-fruit node,
                // so a carried tomato still paints above it.
                shape: Some(assets.immortality.clone()),
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
            Box::new(frost::SceneNode {
                // The game-over overlay, above everything: the process
                // gives the node its dim veil over the whole window — and
                // clears it again — every frame, and its three children
                // carry, in draw order, the "Game Over" title above the
                // window's center, the Play button's rectangle below it,
                // and the button's label on the button's center.
                children: vec![
                    Box::new(frost::SceneNode {
                        // The "Game Over" title.
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        // The Play button's rectangle.
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        // The button's label.
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            }),
        ],
        ..Default::default()
    });

    if let Err(err) = frost::run_configured(
        scene,
        Demo::new(assets),
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

    /// [Demo::new] starts the demo mirroring the scene's seeded sprites
    /// and the plants' full reserves: no tool is active or held, the
    /// spray can rests in `SPRAY_SLOT` and the watering can in
    /// `CAN_SLOT` with the other slots empty, the bench is bare — no
    /// slot planted, every reserve full, the basket seed waiting to be
    /// seeded — nothing is falling, and no pour loop is running.
    #[test]
    fn the_initial_state_mirrors_the_seeded_scene() {
        let demo = Demo::new(Assets::load());
        assert_eq!(demo.active, None);
        assert_eq!(demo.held, None);
        assert!(demo.picking.is_none());
        assert!(demo.flying.is_none());
        assert!(demo.basket_seed);
        assert_eq!(demo.slots[SPRAY_SLOT], Some(Tool::SprayCan));
        assert_eq!(demo.slots[CAN_SLOT], Some(Tool::WaterCan));
        assert_eq!(demo.slots[0], None);
        assert_eq!(demo.slots[1], None);
        assert_eq!(demo.plants.len(), PLANT_POS.len());
        assert!(demo.plants.iter().all(|p| !p.planted));
        assert!(demo.plants.iter().all(|p| p.water == 1.0));
        assert!(demo.falls.is_empty());
        assert!(!demo.pouring);
        assert!(demo.over);
    }

    /// The Play button is the rectangle `PLAY_HALF` around its center at
    /// `[0.0, PLAY_Y]`: the center and the edges are inside, a point just
    /// past an edge is not.
    #[test]
    fn the_play_button_hits_its_rectangle() {
        let center = [0.0, PLAY_Y];
        assert!(on_play_button(center));
        assert!(on_play_button([PLAY_HALF[0], PLAY_Y]));
        assert!(on_play_button([-PLAY_HALF[0], PLAY_Y]));
        assert!(on_play_button([0.0, PLAY_Y + PLAY_HALF[1]]));
        assert!(on_play_button([0.0, PLAY_Y - PLAY_HALF[1]]));
        assert!(!on_play_button([PLAY_HALF[0] + 1.0, PLAY_Y]));
        assert!(!on_play_button([-PLAY_HALF[0] - 1.0, PLAY_Y]));
        assert!(!on_play_button([0.0, PLAY_Y + PLAY_HALF[1] + 1.0]));
        assert!(!on_play_button([0.0, PLAY_Y - PLAY_HALF[1] - 1.0]));
    }

    /// The bench is bare when no plant is planted — a fresh demo's bench —
    /// and not bare as soon as one plant is planted.
    #[test]
    fn the_bench_is_bare_only_when_nothing_is_planted() {
        let demo = Demo::new(Assets::load());
        assert!(bench_is_bare(&demo.plants));
        let mut planted = demo.plants;
        planted[0].planted = true;
        assert!(!bench_is_bare(&planted));
    }

    /// The game over is due only after the player has actually planted:
    /// the bare bench at start and right after a Play press is not a game
    /// over, and a planted bench never is one.
    #[test]
    fn the_game_over_is_due_only_after_a_planting() {
        let demo = Demo::new(Assets::load());
        assert!(!game_over_due(&demo.plants, false));
        assert!(game_over_due(&demo.plants, true));
        let mut planted = demo.plants;
        planted[0].planted = true;
        assert!(!game_over_due(&planted, true));
    }

    /// A full reserve, drained at the growth's slowed pace — a 60 fps
    /// frame step divided by `GROW_SLOWDOWN` — must run out strictly
    /// before a freshly planted seed's base slice is fully grown — over
    /// `plant::GROW_TIMES[0] * GROW_SLOWDOWN` real seconds — and only just
    /// before it: the plant is meant to pause within the last real second
    /// of the base's growth, before its flowers can start developing.
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

    /// [root_anchor] is the bottom anchor of the plant's root segment — the
    /// root joint, the plant node's own origin: the base rock pivots around
    /// that joint, so the sway leaves it fixed and the anchor is the node's
    /// origin mapped through its transform, whatever the sway angle — the
    /// point the fruit's body center drops to, in both x and y.
    #[test]
    fn the_fall_destination_is_the_root_anchor() {
        let node = frost::SceneNode {
            transform: frost::Transform::rotate(0.4)
                .compose(&frost::Transform::translate(100.0, 200.0)),
            ..Default::default()
        };
        assert_eq!(root_anchor(&node), [100.0, 200.0], "the anchor is the root joint, in x and y");
    }

    /// The bench starts bare: no plant grows, and no reserve drains,
    /// until a seed lands in a slot — then only the planted slot grows,
    /// the other slots' seeds sitting untouched, full-watered.
    #[test]
    fn the_bench_stays_bare_until_a_seed_lands() {
        // A long frame with the bench bare: nothing would grow or drain
        // even if the clocks ran.
        let mut demo = Demo::new(Assets::load());
        let (active, layers, started) = demo.grow_plants(9.0);
        assert_eq!(active, 0);
        assert_eq!(layers, 0);
        assert_eq!(started, [false; PLANT_POS.len()]);
        for i in 0..PLANT_POS.len() {
            assert_eq!(demo.plants[i].plant.grown_layers(), 0, "plant {i}");
            assert_eq!(demo.plants[i].water, 1.0, "plant {i}");
        }
        // A seed lands in slot 3 and, kept watered, grows: ten real
        // seconds — 3⅓ growth-clock seconds — is just past its base
        // slice, and the other slots' seeds hold untouched.
        let dt = 1.0 / 60.0;
        demo.plants[3].planted = true;
        for _ in 0..600 {
            demo.plants[3].water = 1.0;
            demo.grow_plants(dt);
        }
        let (active, _layers, started) = demo.grow_plants(dt);
        assert_eq!(active, 1);
        assert_eq!(started, [false, false, false, true, false, false]);
        assert_eq!(demo.plants[3].plant.grown_layers(), 1);
        for i in [0, 1, 2, 4, 5] {
            assert_eq!(demo.plants[i].plant.grown_layers(), 0, "plant {i}");
            assert_eq!(demo.plants[i].water, 1.0, "plant {i}");
        }
    }

    /// Planted plants grow side by side: two planted slots run the same
    /// slowed clock at once, so they reach the same layer together, while
    /// the unplanted slots' seeds hold untouched.
    #[test]
    fn planted_plants_grow_side_by_side() {
        let dt = 1.0 / 60.0;
        let mut demo = Demo::new(Assets::load());
        demo.plants[0].planted = true;
        demo.plants[2].planted = true;
        // Twenty real seconds — 6⅔ growth-clock seconds — every planted
        // plant kept watered: both past their base and their low-middle
        // slice, the same layer, together.
        for _ in 0..1200 {
            demo.plants[0].water = 1.0;
            demo.plants[2].water = 1.0;
            demo.grow_plants(dt);
        }
        let (active, _layers, started) = demo.grow_plants(dt);
        assert_eq!(active, 2);
        assert!(started[0] && started[2]);
        assert!(!started[1] && !started[3] && !started[4] && !started[5]);
        let g0 = demo.plants[0].plant.grown_layers();
        let g2 = demo.plants[2].plant.grown_layers();
        assert!(g0 >= 1, "plant 0 grew {g0} layers");
        assert_eq!(g0, g2, "the planted plants grow in lockstep");
        for i in [1, 3, 4, 5] {
            assert_eq!(demo.plants[i].plant.grown_layers(), 0, "plant {i}");
            assert_eq!(demo.plants[i].water, 1.0, "plant {i}");
        }
    }

    /// A planted plant left dry withers all the way back to nothing —
    /// its growth clock at zero, no slice, no bloom — and is gone: its
    /// slot is freed, the plant reset to a fresh, full-watered seed.
    #[test]
    fn a_completely_withered_plant_is_gone_and_frees_its_slot() {
        let dt = 1.0 / 60.0;
        let mut demo = Demo::new(Assets::load());
        // A fully grown plant — no ripe fruit, so nothing holds its
        // clock above zero — left dry withers back to nothing.
        demo.plants[0].planted = true;
        demo.plants[0].plant.step(15.5);
        demo.plants[0].water = 0.0;
        let mut frames = 0usize;
        while demo.plants[0].planted && frames < 20_000 {
            demo.grow_plants(dt);
            frames += 1;
        }
        assert!(
            !demo.plants[0].planted,
            "the plant never withered away in {frames} frames"
        );
        assert!(demo.plants[0].plant.gone());
        assert_eq!(demo.plants[0].plant.grown_layers(), 0);
        assert_eq!(demo.plants[0].water, 1.0);
        assert_eq!(demo.plants[0].dryness, 0.0);
    }

    /// [plant_anchors] maps each `PLANT_POS` root joint the way the grass
    /// stretch maps the whole texture: for a 1920x1080 window — the
    /// grass's natural size — the joint's pixel is centered in the window.
    #[test]
    fn plant_anchors_center_the_joints_in_a_natural_size_window() {
        let anchors = plant_anchors(GRASS_SIZE[0], GRASS_SIZE[1]);
        let want: [[f32; 2]; PLANT_POS.len()] = [
            [-37.0, 26.0],
            [288.0, -6.0],
            [673.0, -63.0],
            [-221.0, -31.0],
            [121.0, -100.0],
            [503.0, -179.0],
        ];
        for i in 0..PLANT_POS.len() {
            assert!(
                (anchors[i][0] - want[i][0]).abs() < 1e-3
                    && (anchors[i][1] - want[i][1]).abs() < 1e-3,
                "anchor {i}: got {:?}, want {:?}",
                anchors[i],
                want[i]
            );
        }
    }

    /// [nearest_free_slot] picks the closest not-yet-planted slot: a point
    /// on slot 1's anchor lands in slot 1, with slot 1 planted the same
    /// point lands in the next closest free slot — slot 4 — and with
    /// every slot planted there is no landing, so the seed goes back to
    /// the basket.
    #[test]
    fn nearest_free_slot_picks_the_closest_unplanted_slot() {
        let (w, h) = (GRASS_SIZE[0], GRASS_SIZE[1]);
        let none_planted = [false; PLANT_POS.len()];
        let p1 = plant_anchors(w, h)[1];
        assert_eq!(nearest_free_slot(p1, w, h, &none_planted), Some(1));
        let mut planted = none_planted;
        planted[1] = true;
        assert_eq!(nearest_free_slot(p1, w, h, &planted), Some(4));
        let all_planted = [true; PLANT_POS.len()];
        assert_eq!(nearest_free_slot(p1, w, h, &all_planted), None);
    }

    /// A dry plant withers at half the growth's pace: a fully grown plant
    /// whose reserve runs dry shrinks back through the last slice —
    /// 0.5 growth-clock seconds — in 0.5 × 2 × `GROW_SLOWDOWN` real
    /// seconds, twice as long as the same span would take growing.
    #[test]
    fn a_dry_plant_withers_at_half_the_growth_pace() {
        let dt = 1.0 / 60.0;
        let mut demo = Demo::new(Assets::load());
        // A planted plant, fully grown — all five slices — and dry: the
        // withering runs.
        demo.plants[0].planted = true;
        demo.plants[0].plant.step(15.5);
        demo.plants[0].water = 0.0;
        assert_eq!(demo.plants[0].plant.grown_layers(), 5);
        let mut t = 0.0;
        for _ in 0..1000 {
            if demo.plants[0].plant.grown_layers() < 5 {
                break;
            }
            demo.grow_plants(dt);
            t += dt;
        }
        assert!(
            demo.plants[0].plant.grown_layers() < 5,
            "the withering never ran back through the last slice"
        );
        let want = (15.5 - plant::FULL_GROW_TIME) * 2.0 * GROW_SLOWDOWN;
        assert!(
            (t - want).abs() < 2.0 * dt,
            "the withering took {t} real seconds, want {want}"
        );
    }

    /// A dry plant yellows over `DRY_YELLOW_TIME` real seconds, and a
    /// watered one rewhitens over `DRY_WHITE_TIME`: a plant dry for half
    /// a yellowing — 3 seconds — is green again 1.5 seconds after it is
    /// watered.
    #[test]
    fn a_dry_plant_yellows_and_a_watered_one_regreens() {
        let dt = 1.0 / 60.0;
        let mut demo = Demo::new(Assets::load());
        demo.plants[0].planted = true;
        demo.plants[0].plant.step(5.0);
        assert_eq!(demo.plants[0].dryness, 0.0);
        // Three real seconds dry: half of the yellowing span.
        demo.plants[0].water = 0.0;
        for _ in 0..180 {
            demo.grow_plants(dt);
        }
        assert!(
            (demo.plants[0].dryness - 0.5).abs() < 1e-3,
            "the dryness should be half-way yellow after 3 seconds, is {}",
            demo.plants[0].dryness
        );
        // One and a half seconds watered: back to white.
        demo.plants[0].water = 1.0;
        for _ in 0..90 {
            demo.grow_plants(dt);
        }
        assert!(
            demo.plants[0].dryness < 1e-6,
            "the watered plant should be white again, dryness is {}",
            demo.plants[0].dryness
        );
    }

    /// The dry water bar's border blinks red in square halves of
    /// `DRY_BLINK` real seconds: lit at a period's start and first half,
    /// dark in its second half, lit again at the next period.
    #[test]
    fn the_dry_bar_blinks_red_in_square_halves() {
        assert!(dry_blink_on(0.0), "lit at the period's start");
        assert!(dry_blink_on(DRY_BLINK * 0.25), "lit in the first half");
        assert!(!dry_blink_on(DRY_BLINK * 0.75), "dark in the second half");
        assert!(dry_blink_on(DRY_BLINK), "lit at the next period's start");
        assert!(
            !dry_blink_on(1.0 + DRY_BLINK * 0.75),
            "dark in a later period's second half"
        );
    }

    /// Six simple plant anchors for the fall phase-machine tests: a 3×2
    /// grid, 100 px apart horizontally and 50 px apart vertically.
    fn test_anchors() -> [[f32; 2]; 6] {
        [
            [0.0, 0.0],
            [100.0, 0.0],
            [200.0, 0.0],
            [0.0, 50.0],
            [100.0, 50.0],
            [200.0, 50.0],
        ]
    }

    /// A simple wheel radius, in user pixels, for the fall phase-machine
    /// tests: any positive value drives the spin; its exact size only sets
    /// how fast the wheel turns.
    const TEST_RADIUS: f32 = 20.0;

    /// A fresh fall drops toward the root anchor at the fall speed, in both
    /// x and y, until its body center sits on it, where it lands, plays the
    /// drop clip, and starts rolling to a random anchor.
    #[test]
    fn the_fall_lands_at_the_root_anchor_and_starts_rolling() {
        let mut rng = frost::Rng::with_seed(42);
        let anchors = test_anchors();
        let mut fall = Fall::new([0.0, 0.0], [50.0, 100.0]);
        assert_eq!(fall.phase, FallPhase::Falling);
        let dt = 1.0 / 60.0;
        for _ in 0..600 {
            fall.step(dt, &mut rng, &anchors, TEST_RADIUS);
            if fall.phase != FallPhase::Falling {
                break;
            }
        }
        assert_eq!(fall.phase, FallPhase::Rolling, "the fall should land and start rolling");
        assert_eq!(fall.body, [0.0, 0.0], "the body should sit on the root anchor");
        assert!(fall.landed, "the drop clip should have played");
        assert!(anchors.contains(&fall.target), "the target should be a plant anchor");
    }

    /// A landed tomato rolls along the straight line from its start to its
    /// target at the roll speed, and begins its rest the frame it reaches
    /// the target, sitting exactly on it.
    #[test]
    fn the_roll_reaches_its_target_and_starts_resting() {
        let mut rng = frost::Rng::with_seed(7);
        let anchors = test_anchors();
        let mut fall = Fall::new([0.0, 0.0], [0.0, 0.0]);
        // One long frame drops it to the ground and starts the first roll.
        fall.step(1.0, &mut rng, &anchors, TEST_RADIUS);
        assert_eq!(fall.phase, FallPhase::Rolling);
        let target = fall.target;
        let dt = 1.0 / 60.0;
        for _ in 0..6000 {
            fall.step(dt, &mut rng, &anchors, TEST_RADIUS);
            if fall.phase != FallPhase::Rolling {
                break;
            }
        }
        assert_eq!(fall.phase, FallPhase::Resting, "the roll should reach its target");
        assert_eq!(fall.body, target, "the body should sit on the target");
    }

    /// A resting tomato breathes — its y-scale squashes below and stretches
    /// above 1.0, its x-scale compensating — and after its random rest it
    /// picks a new, different anchor to roll to.
    #[test]
    fn the_rest_breathes_and_picks_a_new_anchor() {
        let mut rng = frost::Rng::with_seed(99);
        let anchors = test_anchors();
        let mut fall = Fall::new([0.0, 0.0], [0.0, 0.0]);
        fall.step(1.0, &mut rng, &anchors, TEST_RADIUS);
        let dt = 1.0 / 60.0;
        // Roll to the first target and into the rest.
        for _ in 0..6000 {
            fall.step(dt, &mut rng, &anchors, TEST_RADIUS);
            if fall.phase != FallPhase::Rolling {
                break;
            }
        }
        assert_eq!(fall.phase, FallPhase::Resting);
        let first_target = fall.target;
        let mut saw_squash = false;
        let mut saw_stretch = false;
        // Run well past the longest rest (3 s) and a full breath (2 s).
        for _ in 0..400 {
            fall.step(dt, &mut rng, &anchors, TEST_RADIUS);
            if fall.phase != FallPhase::Resting {
                break;
            }
            let [sx, sy] = fall.scale_factors();
            if sy < 1.0 - 1e-3 {
                saw_squash = true;
            }
            if sy > 1.0 + 1e-3 {
                saw_stretch = true;
            }
            // The area is held: the x-scale is the reciprocal of the y.
            assert!((sx * sy - 1.0).abs() < 1e-4, "the breathing holds the area");
        }
        assert!(saw_squash, "the rest should squash out (sy < 1)");
        assert!(saw_stretch, "the rest should stretch in (sy > 1)");
        assert_eq!(fall.phase, FallPhase::Rolling, "the rest should end into a new roll");
        assert_ne!(fall.target, first_target, "the new anchor should differ from the old");
    }

    /// A running bump lifts the body off its rolling line by the half-sine
    /// hop, peaking at `BUMP_HEIGHT`, without touching the x position.
    #[test]
    fn the_bump_lifts_the_body_off_its_line() {
        let mut fall = Fall::new([0.0, 0.0], [0.0, 0.0]);
        fall.phase = FallPhase::Rolling;
        fall.start = [0.0, 0.0];
        fall.target = [100.0, 0.0];
        fall.body = [50.0, 0.0];
        // Mid-hop: the half-sine is at its peak, sin(π/2) = 1.
        fall.bump = 0.5;
        let base = fall.body;
        let pos = fall.body_position();
        assert_eq!(pos[0], base[0], "the bump should not move the x");
        assert!(
            (pos[1] - (base[1] + BUMP_HEIGHT)).abs() < 1e-4,
            "the mid-hop should peak at BUMP_HEIGHT"
        );
        // Take-off and landing: the half-sine is (within float noise) zero,
        // the body is on the line.
        fall.bump = 0.0;
        let take_off = fall.body_position();
        assert!(
            (take_off[1] - base[1]).abs() < 1e-6,
            "at take-off the body is on the line"
        );
        fall.bump = 1.0;
        let landing = fall.body_position();
        assert!(
            (landing[1] - base[1]).abs() < 1e-6,
            "at landing the body is on the line"
        );
    }

    /// A rolling tomato turns like a wheel: its spin advances by the
    /// distance it travels over the radius each frame (a no-slip roll), and
    /// it stands back upright (spin zeroed) the frame it reaches its target
    /// and rests.
    #[test]
    fn the_roll_turns_the_tomato_like_a_wheel() {
        let mut rng = frost::Rng::with_seed(5);
        let anchors = test_anchors();
        let mut fall = Fall::new([0.0, 0.0], [0.0, 0.0]);
        // One long frame drops it to the ground and starts the first roll.
        fall.step(1.0, &mut rng, &anchors, TEST_RADIUS);
        assert_eq!(fall.phase, FallPhase::Rolling);
        let dt = 1.0 / 60.0;
        let mut saw_growth = false;
        let mut prev_spin = fall.spin;
        let mut prev_body = fall.body;
        // Step the roll until it reaches its target; each frame the spin
        // should advance by the distance traveled over the radius.
        for _ in 0..6000 {
            fall.step(dt, &mut rng, &anchors, TEST_RADIUS);
            if fall.phase == FallPhase::Rolling {
                let traveled = dist2(prev_body, fall.body).sqrt();
                if traveled > 1e-6 {
                    let expected = prev_spin + traveled / TEST_RADIUS;
                    assert!(
                        (fall.spin - expected).abs() < 1e-3,
                        "the spin should advance by the travel over the radius"
                    );
                    if fall.spin > prev_spin {
                        saw_growth = true;
                    }
                }
                prev_spin = fall.spin;
                prev_body = fall.body;
            } else {
                break;
            }
        }
        assert!(saw_growth, "the spin should grow while the tomato rolls");
        assert_eq!(fall.phase, FallPhase::Resting, "the roll should reach its target");
        assert_eq!(fall.spin, 0.0, "the rest should stand the tomato upright");
    }

    /// The pivot's scale factors are neutral (no squash) while the tomato
    /// falls or rolls, and only the rest breathes.
    #[test]
    fn the_scale_factors_are_neutral_outside_the_rest() {
        let mut rng = frost::Rng::with_seed(1);
        let anchors = test_anchors();
        let mut fall = Fall::new([0.0, 0.0], [50.0, 100.0]);
        assert_eq!(fall.phase, FallPhase::Falling);
        assert_eq!(fall.scale_factors(), [1.0, 1.0], "the fall is neutral");
        fall.step(1.0, &mut rng, &anchors, TEST_RADIUS);
        assert_eq!(fall.phase, FallPhase::Rolling);
        assert_eq!(fall.scale_factors(), [1.0, 1.0], "the roll is neutral");
    }

    /// A new roll always targets an anchor that is not the tomato's current
    /// spot, restarts from there, and arms a fresh bump countdown.
    #[test]
    fn the_start_roll_picks_a_distinct_anchor() {
        let mut rng = frost::Rng::with_seed(3);
        let anchors = test_anchors();
        let mut fall = Fall::new([0.0, 0.0], [100.0, 0.0]);
        fall.start_roll(&mut rng, &anchors);
        assert_ne!(fall.target, [100.0, 0.0], "the target should not be the current spot");
        assert!(anchors.contains(&fall.target), "the target should be a plant anchor");
        assert_eq!(fall.start, [100.0, 0.0], "the roll should start at the current spot");
        assert_eq!(fall.progress, 0.0, "the roll should start at zero progress");
        assert!(
            (BUMP_IN.0..BUMP_IN.1).contains(&fall.bump_in),
            "a bump countdown should be armed"
        );
    }
}
