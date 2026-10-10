//! Four garden tools — a watering can, a spray can, a spade, and a pair of
//! tweezers — as mouse cursors over a grass field
//! that fills the window. The window opens at `WINDOW` logical pixels
//! (1920x1080), set through `run_configured` and `Config::window_size` —
//! so its physical footprint scales with the display's density and is
//! clamped to the monitor if it would not fit — and `Config::render_size`
//! pins the frame to exactly 1920x1080;
//! `assets/sprites/grass.png` is exactly that size, so it fills the frame
//! without being stretched, and the frame is re-stretched every frame to
//! cover the window at its new size, its shape kept.
//!
//! The mouse can hold two tools at once: the active one, drawn as the
//! cursor's sprite, and a stored one, mirrored in the held-items panel. It
//! starts with neither. Pressing and releasing the right mouse button (both through
//! [`frost::Context::mouse_button_down`]) switches the two: the stored
//! tool becomes the active one and the active one is stored — while the
//! dock's two cells trade their tools along mirrored arcs, one bending
//! above and the other below the cells' line by half the distance
//! between them. A left click
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
//! - The spade, `assets/sprites/spade.png`, is scaled `SPADE_SIZE` pixels
//!   wide and rides the cursor by its grip — the middle of the wooden
//!   handle, `SPADE_GRIP` in image pixels — the point a hand would hold it
//!   by. A fresh press of the left mouse button — one after a release;
//!   holding or re-pressing mid-stroke does nothing — digs one stroke: three
//!   beats, one path, [`dig_offset`], with no rotation anywhere. Over
//!   `DIG_PUSH` the tool is shoved forward `DIG_PUSH_DIST` pixels along its
//!   own line, point first; over `DIG_HOLD` it stays there, buried; then
//!   over `DIG_ARC` it arches up and to the left of the cursor — through
//!   `DIG_ARC_CROWN`, out over the ground it just broke — and settles back
//!   onto its anchor, where the stroke began. Forward is the direction the
//!   art already draws, not one invented for the dig: the point sits
//!   `SPADE_TIP` 188 px left and 131 px below the grip, so the spade as
//!   drawn aims 34.9° below the horizontal and `spade_blade()` reads that
//!   lean off the sprite to get the way to push.
//!
//! - The tweezers, `assets/sprites/Tweezers.png`, are scaled
//!   `TWEEZERS_SIZE` pixels wide and ride the cursor by their jaws' points,
//!   `TWEEZERS_TIP` in image pixels — the working end, held like a pen.
//!   They do nothing yet: picking a louse out of a plant needs the pinch
//!   frames this single sprite has no pair for. Until they exist the
//!   tweezers are carried, switched, and parked like every other tool, and
//!   a left click over the grass costs them nothing.
//!
//! `assets/sprites/items.png` is the inventory panel itself. It is scaled
//! uniformly to fit the window's height, with a 20 pixel clearance to the
//! top and bottom borders, and sits 20 pixels clear of the left border,
//! centered vertically — mid left. The panel paints a shelf of four bays,
//! and the shelf is what the slots are: `BAYS` carries each bay's opening
//! centre and the front of its board's lit top — the line a tool's feet go
//! on — both read off the sprite's own pixels. The boards are evenly spaced,
//! 214, 213.5, 214 and 207 pixels from board to ceiling, and a bay's ceiling
//! is the cut of the cell above it, see [`bay_ceiling`], so whatever a tool
//! is drawn over is exactly what answers to its bay when clicked. Every tool
//! is drawn at one size, [`tool_scale`]: the largest scale at which it fits
//! every bay, so a tool neither changes size as it moves along the shelf nor
//! stands larger than the space it is in. One tool per bay: the tweezers on
//! top, then the spade, the spray can, and the watering can at the bottom,
//! all drawn on top of the panel. A bay's click cell runs the panel's full
//! width and is cut at the middle of each board, so the four cells tile the
//! panel edge to edge and a press anywhere on the shelf trades with a bay
//! instead of leaking through to the grass behind it.
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
//! and in the last 5 seconds before that fall the fruit shivers with
//! building intensity, trembling harder and harder until it lets
//! go — the shiver fills the tail of the 8-second darkening and adds no
//! time, and the drop releases from the fruit's resting point, not its
//! tremble — and a
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
//! A parallel swarm of lice runs the same life in smaller bodies: the
//! [`bugs`] module is generic over a [`bugs::Species`], and the louse
//! swarm — the two frames `assets/sprites/Lice1.png` and
//! `assets/sprites/Lice2.png`, art that walks to the LEFT, so its facing
//! flip mirrors the bug's — batches three per plant beside the bugs,
//! starts at two hits of health, and heals two hits every ten seconds
//! once it has reached its plant.
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
//! Over" in large letters above the center, and a Play button below it —
//! the `assets/sprites/held_items.png` panel for a background, panel and
//! label swelling while the cursor rests on them,
//! the label `assets/fonts/Leofont-Regular.ttf` set in. Pressing the
//! button — a left click on it — restarts the game: the bench goes bare
//! again, the basket back to its starting tomato, the swarms empty, and
//! the tools back in their slots, and the overlay goes down.
//!
//! A `frost::Diagnostics` overlay reports the window size, the stretch
//! filter, the frame rate, the frame time, the last frame's total
//! processing time, and the last frame's GPU draw-call count, with four
//! scrolling ten-second strip
//! charts — the integration is the overlay's field in the demo state and
//! one `process` call per frame; its first call creates a dedicated
//! topmost layer in the scene and appends its own nodes to that layer's
//! root, so the readout draws above every node the demo places, and the
//! demo's scene needs no other change. The overlay's parts can be toggled
//! while it runs — Alt-0 the whole overlay, Alt-1..Alt-4 the charts (top
//! chart first), Alt-T the text, Alt-F the stretch filter (bilinear ↔
//! nearest, live — with `render_size` set it is the pixel-art switch) —
//! and the layout reflows around whatever is hidden; Alt+'+' and Alt+'-'
//! grow and shrink the whole overlay, font included.
//!
//! The game state can be saved and reloaded: F5 writes a snapshot — the
//! demo's state, every plant's clocks, the swarms, the carried and the
//! fallen fruit, and both random streams' states, in the [save] module's
//! versioned RON format — to the home directory's
//! `.frost/immortal/snapshot.ron` (falling back to a `snapshot.ron` in the
//! working directory if the home directory cannot be written), and F9
//! reloads the last snapshot: the state is restored in place, the scene's
//! stateful pivots re-homed to match it, and the random streams resumed
//! exactly where they left off. The CLI can load the last snapshot at
//! startup and seed the run — both random streams start from one seed, so
//! a run is fully reproducible:
//!
//! ```text
//! cargo run --example immortal -- --load
//! cargo run --example immortal -- --seed 42
//! ```
//!
//! A text field mid top of the window shows the frame's time scale —
//! `1x` at the start — and the whole simulation runs at it: a click on
//! the field steps the scale one forward, 1x to 2x to 4x to 8x, wrapping
//! back to 1x, and a click-drag steers by distance — pulling right adds
//! a step per `SPEED_STEP` pixels, pulling left takes one away, clamped
//! to the table's ends — with the label previewing the drag's verdict
//! live. Growth, aging, swarms, falls, particles, and the tool's
//! animations all run at the chosen pace; only the diagnostics, the
//! demo's real clock, and the audio keep real time.
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
mod fall;
mod fruit;
pub(crate) use fruit::*;
mod plant;
mod save;
mod scene;
mod tomato;
mod tools;
pub(crate) use tools::*;
mod vipers;
mod worms;
mod zorder;

use assets_load::{Assets, Sounds};
use fall::{Fall, FallPhase};

/// The window's initial size in logical pixels, via
/// `Config::window_size` — its physical footprint grows with the display's
/// scale factor, clamped to the monitor — and the fixed render size, via
/// `Config::render_size`: the frame renders at exactly 1920x1080 and is
/// stretched to fill the window. The grass photo is exactly this size, so
/// it fills the frame without stretching.
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

/// The louse swarm's pool: the same batch cadence as the bugs, three
/// lice per plant.
const LICE_N: usize = bugs::BUGS_PER_PLANT * PLANT_POS.len();

/// The scene root's children, in draw order — the order in which `main`
/// builds them in the scene: the grass underlay, the items panel, the
/// plants group, the fallen-fruit container, the held-items panel, the
/// vipers group, the bugs group, the worms group, the active tool, the
/// basket, the immortality badge, the carried fruit, and the game-over
/// overlay. The root node itself is the dark ground background.
const CHILD_GRASS: usize = 0;
const CHILD_ITEMS: usize = 1;
const CHILD_PLANTS: usize = 2;
const CHILD_FALLEN_FRUIT: usize = 3;
const CHILD_HELD: usize = 4;
const CHILD_VIPERS: usize = 5;
const CHILD_BUGS: usize = 6;
const CHILD_LICE: usize = 7;
const CHILD_WORMS: usize = 8;
const CHILD_TOOL: usize = 9;
const CHILD_BASKET: usize = 10;
const CHILD_BADGE: usize = 11;
const CHILD_HELD_FRUIT: usize = 12;
const CHILD_OVERLAY: usize = 13;
const CHILD_SPEED: usize = 14;

/// The time-scale field node's children, in draw order: the background
/// rectangle and the current scale's label.
const SPEED_FIELD_BG: usize = 0;
const SPEED_FIELD_LABEL: usize = 1;

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

/// The Play button's half extents, in window pixels: the 390 by 200
/// button box that frames the `held_items.png` panel background — the
/// panel's own 389 x 200 texture, rounded to even halves.
const PLAY_HALF: [f32; 2] = [195.0, 100.0];

/// The Play button's center's height, in window pixels below the window's
/// center: low enough that the taller panel-button clears the "Game
/// Over" title above it.
const PLAY_Y: f32 = -90.0;

/// How much larger the button panel and its label grow while the cursor
/// rests on the button: a 12% swell, eased in and out.
const PLAY_SWELL: f32 = 0.12;

/// The Play button's "Play" label's size, in pixels.
const PLAY_LABEL_SIZE: f32 = 54.0;

/// Whether the user-space point `p` is inside the Play button's
/// rectangle: the button's half extents, centered on
/// `[0.0, PLAY_Y]`.
fn on_play_button(p: [f32; 2]) -> bool {
    p[0].abs() <= PLAY_HALF[0] && (p[1] - PLAY_Y).abs() <= PLAY_HALF[1]
}

/// The frame's time scales, as multiples of real time: the game's whole
/// simulation — growth, aging, swarms, falls, particles, and the tool's
/// animations — steps on `dt * TIME_SCALES[speed]` instead of `dt`, while
/// the real clock, the diagnostics, and the audio ride real time. The
/// field mid top of the window shows the current one; it starts at 1x.
const TIME_SCALES: [f32; 4] = [1.0, 2.0, 4.0, 8.0];

/// The time-scale field's label size, in pixels.
const SPEED_LABEL_SIZE: f32 = 48.0;

/// The time-scale field's half extents, in window pixels: the box the
/// label sits in, mid top of the window.
const SPEED_HALF: [f32; 2] = [70.0, 34.0];

/// The time-scale field's clearance to the window's top border, in
/// window pixels.
const SPEED_TOP: f32 = 14.0;

/// The time-scale drag's slop, in window pixels: a press released within
/// this many pixels of where it began is a click — the scale steps one
/// forward, wrapping from 8x back to 1x; only a longer pull steers by
/// distance.
const SPEED_SLOP: f32 = 8.0;

/// The time-scale drag's step, in window pixels: every one of these
/// pulled to the right adds a step to the scale, every one to the left
/// takes one away, rounded to the nearest step and clamped to the
/// table's ends.
const SPEED_STEP: f32 = 60.0;

/// The time-scale field's background: a dark, semi-transparent wash, the
/// game-over veil's lighter sibling.
const SPEED_BG: frost::Color = frost::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.45,
};

/// The time-scale field's background while the drag holds it down: the
/// wash lit a step brighter.
const SPEED_BG_HELD: frost::Color = frost::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.65,
};

/// The time-scale field's center in user space, for a window of height
/// `h`: mid-width, `SPEED_TOP` clear of the top border.
fn speed_field_center(h: f32) -> [f32; 2] {
    [0.0, h / 2.0 - SPEED_HALF[1] - SPEED_TOP]
}

/// Whether the user-space point `p` is inside the time-scale field's
/// box, for a window of height `h`.
fn on_speed_field(p: [f32; 2], h: f32) -> bool {
    let c = speed_field_center(h);
    (p[0] - c[0]).abs() <= SPEED_HALF[0] && (p[1] - c[1]).abs() <= SPEED_HALF[1]
}

/// The verdict of a time-scale click: the step after `start`, wrapping
/// from the table's last scale back to its first.
fn speed_click(start: usize) -> usize {
    (start + 1) % TIME_SCALES.len()
}

/// The verdict of a time-scale drag: the scale index reached by pulling
/// `dx` window pixels from a press that started at index `start` — one
/// [TIME_SCALES] step per `SPEED_STEP` pixels, rounded to the nearest
/// step, right faster and left slower, clamped to the table's ends
/// (unlike the click, the drag does not wrap).
fn speed_drag(start: usize, dx: f32) -> usize {
    (start as f32 + (dx / SPEED_STEP).round()).clamp(0.0, TIME_SCALES.len() as f32 - 1.0) as usize
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
fn nearest_free_slot(
    p: [f32; 2],
    w: f32,
    h: f32,
    planted: &[bool; PLANT_POS.len()],
) -> Option<usize> {
    let anchors = plant_anchors(w, h);
    (0..PLANT_POS.len())
        .filter(|&i| !planted[i])
        .min_by(|a, b| dist2(p, anchors[*a]).total_cmp(&dist2(p, anchors[*b])))
}

/// `Getingeye1.png` and `Getingeye2.png`'s texture size in pixels: both
/// frames are the same size, and the swarm fits that width to the
/// rendered bee width inside the [`vipers`] module.
const VIPER_IMAGE: [f32; 2] = [198.0, 179.0];

/// A tomato plant and its water reserve, kept together: the growth clock
/// is paced by the reserve — the growth runs at its slowed pace while the
/// reserve holds, and withers backward at half that pace toward the point
/// of full ripening while it is dry — the reserve refills from the drops
/// that fall into the plant's root hitbox, and the dryness yellows the
/// plant's node while the reserve is dry.
#[derive(Clone)]
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
    /// the watering can's tilt, the spray can's burst angle, or the spade's
    /// place in its dig stroke.
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
    /// Elapsed seconds of the current dig stroke; `None` while the spade
    /// rides the cursor in the pose it is drawn in. A fresh press starts
    /// one, and `tick_tool` runs it to `DIG_TIME`.
    dig: Option<f32>,
    /// The watering can's shape, restored to the tool node when the
    /// toggle switches back to it.
    can: frost::Shape,
    /// The two spray frames, loaded once: `spray1` is the at-rest frame
    /// and `spray2` the pressed frame; swapping the node's `shape` between
    /// them is a cheap `Arc` clone of the pixel buffer.
    spray1: frost::Shape,
    spray2: frost::Shape,
    /// The spade's shape, the one frame the dig stroke rotates and shifts.
    spade: frost::Shape,
    /// The tweezers' shape, carried but still inert: no second frame to
    /// pinch with yet.
    tweezers: frost::Shape,
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
    /// The two louse walk frames — art that walks to the left; the
    /// louse swarm's nodes swap between them as cheap `Arc` clones.
    lice1: frost::Shape,
    lice2: frost::Shape,
    /// The two worm peristaltic frames, loaded once; the swarm's worm
    /// nodes swap between them as cheap `Arc` clones.
    worm1: frost::Shape,
    worm2: frost::Shape,
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
    /// The swarm of lice running the bugs' life in smaller, untinted
    /// bodies with left-facing art: two hits of health, two recovered
    /// every ten seconds at the plant; the swarm steps and lays them
    /// out on the lice node (root's [`CHILD_LICE`] child).
    lice: bugs::Bugs,
    /// The swarm of peristaltic worms crossing the soil: the worms emerge
    /// from the convex hull of the plants' root anchors, crawl to a random
    /// point of the same hull, and burrow back in — at most ten above
    /// ground at once — and the swarm steps and lays them out on the
    /// matching child of the worms node (root's [`CHILD_WORMS`] child).
    worms: worms::Worms,
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
    /// The tool swap's flights, while they are in the air — one per
    /// dock cell, riding [CellFlight] arcs in opposite bends.
    cell_fly: Option<[CellFlight; 2]>,
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
    /// The Play button's background: the `held_items.png` panel, built
    /// once; laid onto the overlay's button child while the overlay is up.
    play_button: frost::Shape,
    /// The uniform-per-axis scale that fits the `held_items` texture to
    /// the button box, before the hover swell.
    play_fit: [f32; 2],
    /// The button's hover swell, eased 0 (at rest) to 1 (hovered) every
    /// frame; the panel and its label scale with it.
    play_hover: f32,
    /// The overlay's "Play" label, built once from the embedded font; laid
    /// onto the overlay's label child while the overlay is up.
    play_label: frost::Shape,
    /// The frame's time scale: an index into [`TIME_SCALES`], starting at
    /// 1x; the game's whole simulation steps on `dt * TIME_SCALES[speed]`.
    /// The field mid top of the window shows it: a click there steps one
    /// forward, wrapping, and a drag steers by distance — right faster,
    /// left slower, clamped (see [`speed_click`] and [`speed_drag`]).
    speed: usize,
    /// The time-scale field's four labels — "1x", "2x", "4x", "8x" —
    /// built once from the embedded font, one laid onto the field's label
    /// child every frame.
    speed_labels: [frost::Shape; TIME_SCALES.len()],
    /// The live time-scale drag, if one is held down on the field: the
    /// press's mouse x and the scale it started from. The verdict lands
    /// on the release — a click steps, a pull drags — and until then the
    /// field's label previews a dragging pointer's verdict live.
    speed_press: Option<(f32, usize)>,
    /// The diagnostics overlay: the window size, the frame rate, the
    /// frame time, the last frame's processing time, and the last
    /// frame's GPU draw-call count, with four scrolling ten-second strip
    /// charts. Its first `process` call appends its own nodes to the
    /// scene's root, after the demo's own children, and every later call
    /// updates those same nodes in place.
    diag: frost::Diagnostics,
    /// Whether F5 was down on the previous frame; the save fires on its
    /// press edge (see [`Demo::save_snapshot`]).
    save_key: bool,
    /// Whether F9 was down on the previous frame; the reload fires on its
    /// press edge (see [`Demo::reload_snapshot`]).
    reload_key: bool,
    /// The snapshot loaded at start up (`--load`), waiting for the first
    /// frame to apply: the scene's stateful pivots need the basket's fit,
    /// which the chrome layout sets on that frame.
    loaded: Option<save::Snapshot>,
}

impl frost::Process for Demo {
    /// Steps the whole frame, in the order the game's invariants require:
    /// the diagnostics overlay updates its nodes — first, so its first
    /// call appends them to the scene's root before anything else runs;
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
        // The diagnostics overlay steps first, before the demo's own clock:
        // its first call appends its own nodes to the scene's root, and
        // every later call updates those same nodes in place.
        self.diag.process(ctx, dt);

        let (w, h) = ctx.size();
        self.time += dt;
        let anchors = self.layout_chrome(ctx, w, h);

        // A snapshot loaded at start up (`--load`) applies on the first
        // frame, once the chrome has laid the basket out — the re-homed
        // fruit needs the basket's fit — and before the basket seed below,
        // so the snapshot's own basket-seed flag decides the seeding.
        if let Some(snapshot) = self.loaded.take() {
            self.apply(&snapshot, ctx);
        }

        // The first frame seeds the basket with its starting tomato: the
        // seed's scale depends on the basket's fit, which the chrome
        // layout above just set.
        if self.basket_seed {
            self.basket_seed = false;
            self.seed_basket_tomato(ctx);
        }

        // The mid-top speed field picks the scale the frame runs at: from
        // here the whole simulation — the growth and aging clocks, the
        // swarms, the fruit's flight and falls, the tool's animations, the
        // particles, and the Play hover's easing — steps on this scaled
        // frame; only the diagnostics and the demo's real clock above keep
        // real time, and the audio is what it is.
        let dt = dt * TIME_SCALES[self.speed];

        // The bugs step on the planted table itself, not the count, so a
        // withered plant's bugs retarget and a replanted slot's plant gets
        // its own batch; the count is only kept for the tests.
        let (_active_plants, started) = self.grow_plants(dt);

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

        self.step_vipers(ctx, dt, &anchors);
        self.step_worms(ctx, dt, &anchors);

        let (events, lice_events) = self.step_bugs(ctx, dt, &anchors, &started);
        self.play_bug_events(&events);
        self.play_bug_events(&lice_events);
        self.handle_input(ctx);
        self.step_cell_flight(ctx, dt);

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

        // The Play button's hover swell: ease toward the cursor's verdict
        // — any exponential approach is frame-rate friendly — so the
        // panel and label grow on approach and settle on leaving.
        let swell_to = if self.over && on_play_button(self.mouse) {
            1.0
        } else {
            0.0
        };
        self.play_hover += (swell_to - self.play_hover) * (dt * 10.0).min(1.0);

        // Lay the game-over overlay out last: its flag may have risen on
        // this frame, above, or fallen in the input's Play press, and the
        // veil must track the window's current size.
        self.layout_overlay(ctx, w, h);

        // And the time-scale field, above the veil in tree order but under
        // it by band: the box mid top, the label the live verdict.
        self.layout_speed_field(ctx, h);
    }
}

impl Demo {
    /// Builds the demo's initial state out of the loaded `assets`: the
    /// mouse holds no tool at all, the tools rest in the items panel's
    /// bays — mirroring the panel's seeded sprites, [SEEDED_SLOTS]: the
    /// tweezers, the spade, the spray can, and the watering can, one per
    /// bay top to bottom — the bench starts
    /// bare: no plant planted, all six growth clocks at zero, all six
    /// water reserves full, and the basket still holding its starting
    /// tomato, seeded on the first frame once the basket's fit is known.
    fn new(assets: Assets) -> Demo {
        // The tools at rest in the items panel's slots, mirroring the
        // panel's seeded sprites: [SEEDED_SLOTS], the one table the scene's
        // slot children are built from too.
        let slots = SEEDED_SLOTS;

        // The plant's five slice shapes, owned by the demo: the launch
        // plants and every withered-away plant's reset rebuild from them.
        let slices = [
            assets.plant1.clone(),
            assets.plant2.clone(),
            assets.plant3.clone(),
            assets.plant4.clone(),
            assets.plant5.clone(),
        ];
        let plant = plant::Plant::new([&slices[0], &slices[1], &slices[2], &slices[3], &slices[4]]);

        // The game-over overlay's shapes, built once: the title and the
        // Play label from the embedded font — each an `Arc`-shared copy of
        // the font's bytes — and the button's rectangle.
        let game_over = frost::Shape::text_bytes(&assets.font, "Game Over", OVERLAY_TITLE_SIZE)
            .expect("the embedded overlay font decodes");
        let play_label = frost::Shape::text_bytes(&assets.font, "Play", PLAY_LABEL_SIZE)
            .expect("the embedded overlay font decodes");
        // The time-scale field's four labels, built once: "1x", "2x",
        // "4x", and "8x" — the TIME_SCALES table spelled in the overlay
        // font.
        let speed_labels = std::array::from_fn(|i| {
            frost::Shape::text_bytes(
                &assets.font,
                format!("{}x", TIME_SCALES[i] as usize),
                SPEED_LABEL_SIZE,
            )
            .expect("the embedded overlay font decodes")
        });
        // The button's background is the held-items panel; the fit scale
        // maps its texture exactly onto the button box.
        let play_button = assets.held_items.clone();
        let [pw, ph] = play_button.sprite_size().expect("held_items is a sprite");
        let play_fit = [2.0 * PLAY_HALF[0] / pw, 2.0 * PLAY_HALF[1] / ph];

        // The diagnostics overlay, set in the embedded Fira Code font,
        // with every statistic enabled: the window size, the frame rate,
        // the frame time, the processing time, the draw-call count, and
        // the four strip charts.
        let diag =
            frost::Diagnostics::from_bytes(&assets.diag_font, frost::DiagnosticsFlags::all())
                .expect("the embedded overlay font decodes");

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
            dig: None,
            can: assets.can,
            spray1: assets.spray1,
            spray2: assets.spray2,
            spade: assets.spade,
            tweezers: assets.tweezers,
            viper1: assets.viper1,
            viper2: assets.viper2,
            // Three bugs per plant, each plant's batch exactly once: the
            // population climbs 3, 6, …, 18 over the first 75 seconds; a
            // killed bug pops back up at its spawn spot after a random 5
            // to 10 second delay.
            bugs: bugs::Bugs::new(
                &[&assets.bug1, &assets.bug2, &assets.bug3],
                bugs::Species::bug(),
            ),
            bug1: assets.bug1,
            bug2: assets.bug2,
            bug3: assets.bug3,
            // The louse pool: the same batch cadence, the louse skin —
            // smaller, untinted, left-facing art, two hits and a slower
            // two-at-a-time heal.
            lice: bugs::Bugs::new(&[&assets.lice1, &assets.lice2], bugs::Species::louse()),
            lice1: assets.lice1,
            lice2: assets.lice2,
            // The worm pool: fourteen slots, at most ten above ground,
            // each worm's first underground delay staggered across the
            // pool so the first wave trickles out instead of popping.
            worms: worms::Worms::new([&assets.worm1, &assets.worm2]),
            worm1: assets.worm1,
            worm2: assets.worm2,
            sounds: assets.sounds,
            pouring: false,
            falls: Vec::new(),
            flower: assets.flower,
            tomato: assets.tomato,
            tomato_fg: assets.tomato_fg,
            water: frost::ParticleSystem::default(),
            spray: frost::ParticleSystem::default(),
            rng: frost::Rng::default(),
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
            cell_fly: None,
            // The game-over overlay opens with the demo: the bench is
            // bare, so the player sees it over the whole window until the
            // first Play press. The bench never had a plant planted, so
            // its bareness is the start state, not a game over.
            over: true,
            ever_planted: false,
            game_over,
            play_button,
            play_fit,
            play_hover: 0.0,
            play_label,
            speed: 0,
            speed_labels,
            speed_press: None,
            diag,
            save_key: false,
            reload_key: false,
            loaded: None,
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
            frost::Transform::translate([-(w / 2.0) + MARGIN + ITEMS_SIZE[0] * s / 2.0, 0.0]);

        // Bottom right: `MARGIN` clear of the right and bottom borders, at
        // the panel's natural size.
        let held_panel = &mut ctx.scene().root.children[CHILD_HELD];
        held_panel.transform = frost::Transform::translate([
            w / 2.0 - MARGIN - HELD_SIZE[0] * held_panel.scale[0] / 2.0,
            -h / 2.0 + MARGIN + HELD_SIZE[1] * held_panel.scale[1] / 2.0,
        ]);

        // Bottom left: just right of the items panel — `BASKET_GAP` clear
        // of its right edge — and `MARGIN` clear of the bottom border,
        // scaled with the panel's fit so the basket keeps its proportions.
        let basket = &mut ctx.scene().root.children[CHILD_BASKET];
        let bs = basket::basket_scale(h);
        let bc = basket::basket_center(w, h);
        basket.scale = [bs, bs];
        basket.transform = frost::Transform::translate(bc);

        // Top right: `MARGIN` clear of the top and right borders, at
        // `IMMORTALITY_SCALE` of the badge's natural size.
        let badge = &mut ctx.scene().root.children[CHILD_BADGE];
        badge.scale = [IMMORTALITY_SCALE, IMMORTALITY_SCALE];
        badge.transform = frost::Transform::translate([
            w / 2.0 - MARGIN - IMMORTALITY_SIZE[0] * IMMORTALITY_SCALE / 2.0,
            h / 2.0 - MARGIN - IMMORTALITY_SIZE[1] * IMMORTALITY_SCALE / 2.0,
        ]);

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
    /// "Game Over" title above the center, the Play button — the
    /// `held_items` panel with its label — below it; panel and label
    /// swell while the cursor rests on the button; while the overlay is
    /// down, the node and its children carry no shapes at all.
    fn layout_overlay(&mut self, ctx: &mut frost::Context, w: f32, h: f32) {
        let overlay = &mut ctx.scene().root.children[CHILD_OVERLAY];
        // The children's poses hold while the overlay is up and down
        // alike: the title above the window's center, the button and its
        // label on the button's center below it.
        overlay.children[OVERLAY_TITLE].transform =
            frost::Transform::translate([0.0, OVERLAY_TITLE_Y]);
        let button = frost::Transform::translate([0.0, PLAY_Y]);
        overlay.children[OVERLAY_BUTTON].transform = button;
        overlay.children[OVERLAY_LABEL].transform = button;
        // The button's size: the fit that maps the panel texture onto the
        // button box, times the eased hover swell; the label swells with
        // it, uniformly.
        let swell = 1.0 + PLAY_SWELL * self.play_hover;
        let [fx, fy] = self.play_fit;
        overlay.children[OVERLAY_BUTTON].scale = [fx * swell, fy * swell];
        overlay.children[OVERLAY_LABEL].scale = [swell, swell];
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

    /// Lays the time-scale field out, mid top of the window: the box at
    /// its center, its background the dark wash — the brighter one while
    /// a drag holds the field — and its label the current scale, or, while
    /// a drag is past the slop, the verdict that pull would land on, so
    /// the field previews the drag live.
    fn layout_speed_field(&mut self, ctx: &mut frost::Context, h: f32) {
        let center = speed_field_center(h);
        let held = self.speed_press;
        let shown = match held {
            Some((start_x, start)) if (self.mouse[0] - start_x).abs() >= SPEED_SLOP => {
                speed_drag(start, self.mouse[0] - start_x)
            }
            _ => self.speed,
        };
        let field = &mut ctx.scene().root.children[CHILD_SPEED];
        field.transform = frost::Transform::translate(center);
        field.children[SPEED_FIELD_BG].shape = Some(frost::Shape::Rectangle {
            center: [0.0, 0.0],
            extent: SPEED_HALF,
            color: if held.is_some() {
                SPEED_BG_HELD
            } else {
                SPEED_BG
            },
        });
        field.children[SPEED_FIELD_LABEL].shape = Some(self.speed_labels[shown].clone());
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
    /// Returns the `active_plants` count of plants that are growing and
    /// the `started` table the watering, the water bars and the bugs read
    /// — the planted table, which is the bugs' liveness table: a plant
    /// that withered away this frame is not in it.
    fn grow_plants(&mut self, dt: f32) -> (usize, [bool; PLANT_POS.len()]) {
        let mut active_plants = 0usize;
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
        }
        (
            active_plants,
            std::array::from_fn(|i| self.plants[i].planted),
        )
    }

    /// Buzzes the vipers around the flower bench, in parallel with
    /// everything else: one viper per fully grown layer, so the swarm
    /// grows as the bench does — each viper exists only once its own
    /// plant's layer has fully grown — and each viper circles the segment
    /// its layer spawned it, at that segment's midpoint along the static,
    /// fully grown chain, the sway left out, lifted to its plant's anchor
    /// at the fit scale.
    fn step_vipers(&mut self, ctx: &mut frost::Context, dt: f32, anchors: &[[f32; 2]]) {
        let vipers_node = &mut ctx.scene().root.children[CHILD_VIPERS];
        let centers: [[[f32; 2]; vipers::LAYERS]; PLANT_POS.len()] = std::array::from_fn(|i| {
            self.plants[i].plant.layer_midpoints().map(|m| {
                [
                    anchors[i][0] + m[0] * PLANT_SCALE,
                    anchors[i][1] + m[1] * PLANT_SCALE,
                ]
            })
        });
        // The grown-layer count per plant, in bench order: each viper
        // exists on its own plant's schedule, not a global swarm prefix.
        let grown: [usize; PLANT_POS.len()] =
            std::array::from_fn(|i| self.plants[i].plant.grown_layers());
        self.vipers.step(dt, &centers, &grown);
        self.vipers
            .layout(vipers_node, [&self.viper1, &self.viper2]);
        // The depth, per frame: each viper rides one unit above its
        // segment's order on the near side of its orbit — flying right,
        // the lower half, where it moves with the screen — and one below
        // it on the far side, flying left. The segment's own order is the
        // plant's root, in the ground band, for the ground layer, and the
        // plant's upper band for the stalk layers — keyed to the bench
        // slot, so the front plants' vipers ride the front band and never
        // dip into the ground band.
        let facings = self.vipers.facings();
        for (i, child) in vipers_node.children.iter_mut().enumerate() {
            let home = i / vipers::LAYERS;
            let layer = i % vipers::LAYERS;
            let seg_z = if layer == 0 {
                zorder::ground(anchors[home][1])
            } else {
                zorder::upper(home, anchors[home][1])
            };
            child.order = seg_z + facings[i];
        }
    }

    /// Waddles both swarms — the bugs and the lice, same life cycle in
    /// different skins — to the plants, in parallel with everything else:
    /// `alive` is the bench's planted table — which plants are growing —
    /// and the swarms retarget away from a plant that withers and batch
    /// in for each plant that starts growing. Returns both swarms' step
    /// events, which [Demo::play_bug_events] plays.
    fn step_bugs(
        &mut self,
        ctx: &mut frost::Context,
        dt: f32,
        anchors: &[[f32; 2]; PLANT_POS.len()],
        alive: &[bool],
    ) -> (bugs::StepEvents, bugs::StepEvents) {
        let bugs_node = &mut ctx.scene().root.children[CHILD_BUGS];
        let events = self.bugs.step(dt, anchors, alive);
        self.bugs
            .layout(bugs_node, &[&self.bug1, &self.bug2, &self.bug3]);
        let lice_node = &mut ctx.scene().root.children[CHILD_LICE];
        let lice_events = self.lice.step(dt, anchors, alive);
        self.lice.layout(lice_node, &[&self.lice1, &self.lice2]);
        (events, lice_events)
    }

    /// Steps the worms' swarm, in parallel with everything else: the
    /// worms live in the convex hull of the plants' root anchors, so the
    /// same `anchors` the bugs and vipers step from feeds them, and the
    /// swarm lays itself out on the matching child of the worms node
    /// (root's [`CHILD_WORMS`] child).
    fn step_worms(&mut self, ctx: &mut frost::Context, dt: f32, anchors: &[[f32; 2]]) {
        let worms_node = &mut ctx.scene().root.children[CHILD_WORMS];
        self.worms.step(dt, anchors);
        self.worms.layout(worms_node, [&self.worm1, &self.worm2]);
    }

    /// Plays the bugs' last step's arrival sounds: a bug that just
    /// reached its destination while another bug was on the grass nearby
    /// plays one of the tjatter clips, the swarm's random pick, and a bug
    /// that just came up out of the grass — a batch spawn or a respawn —
    /// plays one of the plopp clips.
    fn play_bug_events(&mut self, events: &bugs::StepEvents) {
        for clip in &events.tjatters {
            self.sounds
                .device
                .play_once(&self.sounds.tjatters[*clip], None);
        }
        for clip in &events.plops {
            self.sounds
                .device
                .play_once(&self.sounds.plops[*clip], None);
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

        // Save and reload: F5 writes the last snapshot, F9 reads the last
        // one back — both on their press edges, and both answered even
        // with the game-over overlay up, before the game's own input is.
        let save_key = ctx.key_down(frost::KeyCode::F5);
        if save_key && !self.save_key {
            self.save_snapshot(ctx);
        }
        self.save_key = save_key;
        let reload_key = ctx.key_down(frost::KeyCode::F9);
        if reload_key && !self.reload_key {
            self.reload_snapshot(ctx);
        }
        self.reload_key = reload_key;

        // The game-over overlay is up: the input answers only the Play
        // button — a press on it restarts the game and takes the overlay
        // down — and the game's own input goes untouched this frame.
        if self.over {
            if left && !self.pressed && on_play_button(self.mouse) {
                self.over = false;
                self.restart(ctx);
            }
            // A time-scale drag caught mid-pull when the overlay rose is
            // abandoned: the release belongs to the Play button's world,
            // not the field.
            self.speed_press = None;
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
        // one and the active one is stored, while the dock's cells arc
        // their tools into each other's spots.
        if self.right_pressed && !right {
            // The dock's two tools trade places at once, in mirrored
            // arcs: the left cell's tool dips below the cells' line,
            // the right cell's tool rises above it, each bending off
            // the chord by half the distance between the cells at the
            // middle. From the cells' current spots, so a swap pressed
            // mid-flight bends on from where the tools actually are.
            let (p0, p1) = {
                let panel = &ctx.scene().root.children[CHILD_HELD];
                (
                    panel.children[0].transform.apply([0.0, 0.0]),
                    panel.children[1].transform.apply([0.0, 0.0]),
                )
            };
            let half = (held_local(1)[0] - held_local(0)[0]).abs() / 2.0;
            // Where each cell's tool lands: the other cell's spot as that
            // cell would hold this tool, so a drawing wide in its canvas
            // arrives centred rather than hanging over the frame. The
            // pairing is the one `sync_held` draws with, mid-flight
            // included — the tool leaving a cell is the one riding.
            let (left, right) = if self.cell_fly.is_some() {
                (self.held, self.active)
            } else {
                (self.active, self.held)
            };
            let land = |i: usize, tool: Option<Tool>| match tool {
                Some(tool) => cell_rest(i, tool),
                None => held_local(i),
            };
            self.cell_fly = Some([
                CellFlight {
                    from: p0,
                    to: land(1, left),
                    bump: -half,
                    t: 0.0,
                    dur: SWAP_FLIGHT_TIME,
                },
                CellFlight {
                    from: p1,
                    to: land(0, right),
                    bump: half,
                    t: 0.0,
                    dur: SWAP_FLIGHT_TIME,
                },
            ]);
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
            if on_speed_field(self.mouse, ctx.size().1) {
                // The press belongs to the mid-top time-scale field: the
                // drag starts here, and no tool, tomato, or slot sees
                // this press at all.
                self.speed_press = Some((self.mouse[0], self.speed));
                self.press_slot = None;
            } else {
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
                } else if on_slot.is_none() && self.active == Some(Tool::Spade) {
                    // A fresh press — one after a release, not a re-press
                    // mid-stroke — digs one stroke: the spade is shoved forward
                    // along its own line, held in the soil, and arched back to
                    // its anchor, all over `DIG_TIME`. Holding the button, or
                    // pressing again before the stroke lands, does nothing: the
                    // spade digs once per click.
                    if self.dig.is_none() {
                        self.dig = Some(0.0);
                    }
                }
            }
        } else if !left && self.pressed {
            if let Some((start_x, start)) = self.speed_press.take() {
                // The release settles the time-scale field's gesture: a
                // release inside the slop is a click — the scale steps one
                // forward, wrapping from 8x back to 1x — and a longer pull
                // drags by distance: right adds steps, left takes them
                // away, clamped to the table's ends.
                let dx = self.mouse[0] - start_x;
                self.speed = if dx.abs() < SPEED_SLOP {
                    speed_click(start)
                } else {
                    speed_drag(start, dx)
                };
            } else if let Some(pick) = self.picking.take() {
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

    /// Lays the plants out at their anchors: a tomato picked or snapped
    /// back this frame is reposed by its plant, and a picked slot's
    /// pivot, out of the tree, is simply skipped. The node's modulate
    /// carries the plant's dryness — white when watered, yellowing as
    /// it withers — so the whole tree tints with it.
    fn layout_plants(&mut self, ctx: &mut frost::Context, anchors: &[[f32; 2]; PLANT_POS.len()]) {
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
            // The depth, per frame: the plant node itself stays at order
            // zero so every child's order is absolute. The root slice — the
            // part that meets the soil — takes the ground band, interleaved
            // with the bugs and the worms by the anchor's y; the stalk
            // slices above it and the flower and fruit slots take the upper
            // band, keyed to the bench slot so slot 0's plant sits behind
            // slot 5's, the whole band above the ground band. A picked or
            // fallen tomato rides out of the slot (and takes the upper band
            // with it until it rehomes), so only the slot and the slices
            // need the order here.
            let ay = anchor[1];
            plant_node.children[0].order = zorder::ground(ay);
            for slice in &mut plant_node.children[1..plant::SLICE_N] {
                slice.order = zorder::upper(i, ay);
            }
            for slot in 0..plant::FLOWER_N {
                plant_node.children[plant::slot_index(slot)].order = zorder::upper(i, ay);
            }
            // The dryness tint: the modulate lerps from white to yellow
            // over the withering, and back over the watering, so the
            // slices, the flowers, and the fruit yellow with the plant.
            plant_node.modulate = PLANT_WHITE.lerp(DRY_YELLOW, self.plants[i].dryness);
        }
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
            for swarm in [&mut self.bugs, &mut self.lice] {
                let hit = swarm.hit_at(p.pos);
                for clip in hit.ajs {
                    self.sounds
                        .device
                        .play_once(&self.sounds.ajs[clip], Some(0.35));
                }
                for _ in 0..hit.deaths {
                    self.sounds.device.play_once(&self.sounds.death, None);
                }
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
        for swarm in [&self.bugs, &self.lice] {
            for ([cx, cy], hits) in swarm.health_pips() {
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
        // The swarms start empty again: all three layouts leave their
        // stale children frozen, so the swarms are rebuilt, not just
        // re-stepped.
        self.vipers = vipers::Vipers::new(VIPER_IMAGE);
        self.bugs = bugs::Bugs::new(&[&self.bug1, &self.bug2, &self.bug3], bugs::Species::bug());
        self.lice = bugs::Bugs::new(&[&self.lice1, &self.lice2], bugs::Species::louse());
        self.worms = worms::Worms::new([&self.worm1, &self.worm2]);
        // The fruit goes back to the basket: the carried and the fallen
        // fruit come off the scene, the basket empties, and the starting
        // tomato re-seeds on the next frame, once the basket's fit is
        // known.
        let root = &mut ctx.scene().root;
        root.children[CHILD_HELD_FRUIT].children.clear();
        root.children[CHILD_FALLEN_FRUIT].children.clear();
        root.children[CHILD_BASKET].children[basket::BASKET_FRUIT]
            .children
            .clear();
        self.falls.clear();
        self.basket_seed = true;
        // The mouse holds no tool again: the tools rest in their slots,
        // the held panel's cells go empty, and the in-flight states clear.
        self.slots = SEEDED_SLOTS;
        self.held = None;
        self.set_active(ctx, None);
        let items = &mut ctx.scene().root.children[CHILD_ITEMS];
        for (slot, node) in items.children.iter_mut().enumerate() {
            self.slot_set(slot, node, self.slots[slot]);
        }
        self.picking = None;
        self.flying = None;
        self.cell_fly = None;
        self.press_slot = None;
        self.right_pressed = false;
        self.acc = 0.0;
        self.time = 0.0;
        self.rng = frost::Rng::default();
    }

    /// Seeds all three random streams from `seed` — the demo's
    /// particle-jitter source, the bug swarm's spawn randomizer, and the
    /// worms' spawn randomizer — so the whole run is reproducible
    /// (`--seed`).
    fn set_seed(&mut self, seed: u64) {
        self.rng.set_state(seed);
        self.bugs.set_seed(seed);
        // The louse stream runs on a scramble of the seed: the same run
        // reproducibility, an independent draw order from the bugs'.
        self.lice.set_seed(seed.rotate_left(23));
        self.worms.set_seed(seed);
    }

    /// Loads the last snapshot at start up (`--load`), pending for the
    /// first frame's [Demo::apply] — the scene's stateful pivots need the
    /// basket's fit, which the chrome layout sets on that frame — and
    /// starts the game fresh when no snapshot exists or it cannot be read.
    fn load_last_snapshot(&mut self) {
        match save::load() {
            Ok(loaded) => {
                log::info!("snapshot loaded from {}", loaded.path.display());
                self.loaded = Some(loaded.snapshot);
            }
            Err(err) => log::warn!("{err}"),
        }
    }

    /// F5: captures the whole game state and writes it to the snapshot
    /// file ([save::save]) — the primary path, or its working-directory
    /// fallback; a failed write logs and the game keeps running.
    fn save_snapshot(&mut self, ctx: &mut frost::Context) {
        let snapshot = self.capture(ctx);
        match save::save(&snapshot) {
            Ok(path) => log::info!("snapshot saved to {}", path.display()),
            Err(err) => log::warn!("cannot save the snapshot: {err}"),
        }
    }

    /// F9: reads the last snapshot ([save::load]) and restores the whole
    /// game state from it; a missing or unreadable snapshot logs and the
    /// game keeps running as it is.
    fn reload_snapshot(&mut self, ctx: &mut frost::Context) {
        match save::load() {
            Ok(loaded) => {
                self.apply(&loaded.snapshot, ctx);
                log::info!("snapshot reloaded from {}", loaded.path.display());
            }
            Err(err) => log::warn!("{err}"),
        }
    }

    /// Captures the game's state into a snapshot: the demo's scalars, both
    /// random streams' states, the plants, the falls, the swarms, and the
    /// two parts only the scene can give — the basket's fruit, each
    /// fruit's body center read off its pivot in the basket's local space
    /// so a window resize moves none of it, and the carried fruit's
    /// frozen tint, read off its body leaf.
    fn capture(&self, ctx: &mut frost::Context) -> save::Snapshot {
        let mut snapshot = self.capture_state();
        // The basket's fruit: each pivot's body center, in the basket's
        // local space — the pivot's origin plus its scale times the
        // tomato's leaf offset.
        let [ox, oy] = tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
        snapshot.basket_fruit = ctx.scene().root.children[CHILD_BASKET].children
            [basket::BASKET_FRUIT]
            .children
            .iter()
            .map(|pivot| {
                let [px, py] = pivot.transform.apply([0.0, 0.0]);
                [px + pivot.scale[0] * ox, py + pivot.scale[0] * oy]
            })
            .collect();
        // The carried fruit's body tint, frozen since the pick: the held
        // fruit's body leaf holds it.
        snapshot.held_tint = (self.picking.is_some() || self.flying.is_some()).then(|| {
            let tint = ctx.scene().root.children[CHILD_HELD_FRUIT].children[0].children
                [tomato::TOMATO_BG]
                .modulate;
            [tint.r, tint.g, tint.b, tint.a]
        });
        snapshot
    }

    /// The snapshot's state that the scene holds none of: everything
    /// [Demo::capture] writes except the basket's fruit and the carried
    /// fruit's tint, which it leaves for the scene's read.
    fn capture_state(&self) -> save::Snapshot {
        save::Snapshot {
            version: save::VERSION,
            seed: self.rng.state(),
            bug_seed: self.bugs.rng_state(),
            lice_seed: self.lice.rng_state(),
            time: self.time,
            acc: self.acc,
            over: self.over,
            ever_planted: self.ever_planted,
            slots: self.slots,
            active: self.active,
            held: self.held,
            picking: self.picking.map(save::PickState::from),
            flying: self.flying.as_ref().map(save::FlyState::from),
            held_tint: None,
            angle: self.angle,
            turn: save::TurnState::from(&self.rotation),
            burst: self.burst,
            dig: self.dig,
            showing_spray2: self.showing_spray2,
            pouring: self.pouring,
            basket_seed: self.basket_seed,
            basket_fruit: Vec::new(),
            plants: std::array::from_fn(|i| save::WateredPlantState::from(&self.plants[i])),
            falls: self.falls.iter().map(save::FallState::from).collect(),
            bugs: self.bugs.state(),
            lice: self.lice.state(),
            vipers: self.vipers.state(),
            worms: self.worms.state(),
        }
    }

    /// Restores the demo's state from a snapshot — the scalars, the tools,
    /// the plants, the falls, the swarms, and both random streams —
    /// without touching the scene: the scene's stateful pivots are re-homed
    /// by [Demo::apply], which needs the basket's fit.
    fn restore(&mut self, snapshot: &save::Snapshot) {
        // The pour loop stops, if it plays: tick_tool re-arms it the frame
        // the restored can is fully tilted, if it is.
        if self.pouring {
            self.sounds.device.stop_loop();
            self.pouring = false;
        }
        self.time = snapshot.time;
        self.acc = snapshot.acc;
        self.over = snapshot.over;
        // A swap flight is too short a moment to snapshot: it is simply
        // dropped, and the cells' per-frame step settles them back onto
        // their spots.
        self.cell_fly = None;
        self.ever_planted = snapshot.ever_planted;
        self.slots = snapshot.slots;
        self.active = snapshot.active;
        self.held = snapshot.held;
        self.picking = snapshot.picking.map(Pick::from);
        self.flying = snapshot.flying.map(Fly::from);
        self.angle = snapshot.angle;
        self.rotation = snapshot.turn.into_tween();
        self.burst = snapshot.burst;
        self.dig = snapshot.dig;
        self.showing_spray2 = snapshot.showing_spray2;
        self.basket_seed = snapshot.basket_seed;
        // The plants: each plant's state overwrites its live one — the
        // growth and aging clocks, and every bloom's schedule and tomato.
        for (wp, s) in self.plants.iter_mut().zip(snapshot.plants) {
            wp.planted = s.planted;
            wp.water = s.water;
            wp.dryness = s.dryness;
            wp.plant.restore(&s.plant);
        }
        self.falls = snapshot.falls.iter().copied().map(Fall::from).collect();
        self.bugs.restore(&snapshot.bugs);
        self.lice.restore(&snapshot.lice);
        self.vipers.restore(&snapshot.vipers);
        self.worms.restore(&snapshot.worms);
        // All three random streams resume from the snapshot's states.
        self.rng.set_state(snapshot.seed);
        self.bugs.set_seed(snapshot.bug_seed);
        self.lice.set_seed(snapshot.lice_seed);
        // The particles are short-lived: their streams start over.
        self.water = frost::ParticleSystem::default();
        self.spray = frost::ParticleSystem::default();
    }

    /// Restores the whole game state from a snapshot: the demo's state via
    /// [Demo::restore], and the scene's stateful pivots re-homed to match
    /// it — the plant slots' picked pivots, the carried and the fallen
    /// fruit, and the basket's fruit — with the active tool's pose and
    /// frame restored on top, and the items panel's slots re-synced.
    fn apply(&mut self, snapshot: &save::Snapshot, ctx: &mut frost::Context) {
        self.restore(snapshot);
        self.rehome_scene(ctx, snapshot);
        // The active tool's pose: set_active resets it to the upright,
        // at-rest pose and re-syncs the held panel — the snapshot's tilt,
        // burst, dig, and frame come back on top.
        self.set_active(ctx, self.active);
        self.angle = snapshot.angle;
        self.rotation = snapshot.turn.into_tween();
        self.burst = snapshot.burst;
        self.dig = snapshot.dig;
        self.showing_spray2 = snapshot.showing_spray2;
        if self.active == Some(Tool::SprayCan) {
            self.set_spray_frame(ctx, self.showing_spray2);
        }
        // The items panel's slots mirror the restored tools.
        let items = &mut ctx.scene().root.children[CHILD_ITEMS];
        for (slot, node) in items.children.iter_mut().enumerate() {
            self.slot_set(slot, node, self.slots[slot]);
        }
    }

    /// Re-homes the scene's stateful pivots to the restored state: the
    /// plant slots' tomato pivots — a harvested slot keeps only its flower
    /// leaf, its picked pivot having ridden out of the tree, and an
    /// unharvested slot gets a fresh, shapeless default pivot its bloom's
    /// layout fills in — the carried and the fallen fruit's containers
    /// emptied and rebuilt, the basket's fruit emptied and rebuilt at the
    /// snapshot's body centers, in the basket's local space, at the
    /// basket's current fit, and the carried fruit's body tinted from the
    /// snapshot.
    fn rehome_scene(&mut self, ctx: &mut frost::Context, snapshot: &save::Snapshot) {
        let root = &mut ctx.scene().root;
        // The plant slots: the pivot's presence mirrors the harvested
        // mark — a live pivot, if any, carries the old game's state.
        let plants = &mut root.children[CHILD_PLANTS];
        for (pi, wp) in self.plants.iter().enumerate() {
            for si in 0..plant::FLOWER_N {
                let slot = &mut plants.children[pi].children[plant::slot_index(si)];
                match (wp.plant.is_harvested(si), slot.children.len()) {
                    (true, 2) => {
                        slot.children.remove(plant::SLOT_TOMATO);
                    }
                    (false, 1) => {
                        slot.children.push(Box::new(frost::SceneNode {
                            children: vec![
                                Box::new(frost::SceneNode::default()),
                                Box::new(frost::SceneNode::default()),
                            ],
                            ..Default::default()
                        }));
                    }
                    _ => {}
                }
            }
        }
        // The fruit: the carried and the fallen containers, and the
        // basket, all empty — the restored state re-fills them.
        root.children[CHILD_HELD_FRUIT].children.clear();
        root.children[CHILD_FALLEN_FRUIT].children.clear();
        root.children[CHILD_BASKET].children[basket::BASKET_FRUIT]
            .children
            .clear();
        // The fallen fruit: one fresh, shaped pivot per restored fall, in
        // order — the overgrown fruit, dark red — step_falls lays each
        // out from its entry every frame.
        for _ in &self.falls {
            root.children[CHILD_FALLEN_FRUIT]
                .children
                .push(self.fruit_pivot(tomato::TOMATO_STALE));
        }
        // The basket's fruit: fresh pivots at the restored body centers,
        // at the basket's current fit.
        let s = root.children[CHILD_BASKET].scale[0];
        for &body in &snapshot.basket_fruit {
            root.children[CHILD_BASKET].children[basket::BASKET_FRUIT]
                .children
                .push(Box::new(self.basket_fruit_pivot(body, s)));
        }
        // The carried fruit: a picked or a flying tomato rides the
        // held-fruit container, posed by the ride every frame, tinted
        // from the snapshot — the tint frozen since the pick.
        if let Some([r, g, b, a]) = snapshot.held_tint {
            let tint = frost::Color { r, g, b, a };
            let pivot = self.fruit_pivot(tint);
            // A basket fruit rides at the pick scale under the pointer; a
            // plant fruit the same — the rides pose the transform, the
            // scale stands.
            root.children[CHILD_HELD_FRUIT].children.push(pivot);
        }
    }
}

/// The command line's arguments.
#[derive(clap::Parser)]
struct Cli {
    /// Load the last snapshot at start up — the one F5 wrote, at
    /// `~/.frost/immortal/snapshot.ron` or its working-directory
    /// fallback.
    #[arg(long)]
    load: bool,
    /// Seed both random streams — the demo's particle-jitter source and
    /// the bug swarm's spawn randomizer — from this value, so the whole
    /// run is reproducible.
    #[arg(long)]
    seed: Option<u64>,
}

fn main() {
    env_logger::init();
    log::info!("frost started");
    let cli: Cli = clap::Parser::parse();

    let assets = Assets::load();

    // The scene tree itself lives in `scene`, with the `CHILD_*`
    // index contract beside it.
    let scene = scene::build(&assets);

    let mut demo = Demo::new(assets);
    // The command line's arguments: `--seed` makes the whole run
    // reproducible, and `--load` restores the last snapshot on the first
    // frame — the scene's stateful pivots need the basket's fit, which
    // the chrome layout sets on that frame.
    if let Some(seed) = cli.seed {
        demo.set_seed(seed);
    }
    if cli.load {
        demo.load_last_snapshot();
    }
    if let Err(err) = frost::run_configured(
        scene,
        demo,
        frost::Config {
            window_size: Some(WINDOW),
            // The art is authored at 1920x1080, so render at exactly that
            // pixel size on every display: `ctx.size()` says 1920x1080,
            // the grass fills the buffer 1:1, and anything the window does
            // beyond that is a stretch of the finished frame — shape kept,
            // letterboxed if a window manager ever denies the shape.
            render_size: Some(WINDOW),
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

    /// Every tool there is, for the tests that check what the shelf and the
    /// dock promise whoever they are handed — the player can park any of them
    /// anywhere, so a bay's guarantee cannot be proved on the four the panel
    /// happens to start with.
    const ALL_TOOLS: [Tool; 4] = [Tool::Tweezers, Tool::Spade, Tool::SprayCan, Tool::WaterCan];

    /// [Demo::new] starts the demo mirroring the scene's seeded sprites
    /// and the plants' full reserves: no tool is active or held, the four
    /// tools rest on the shelf as [SEEDED_SLOTS] lays them — the tweezers,
    /// the spade, the spray can, and the watering can, one per painted bay
    /// top to bottom — the bench is bare — no
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
        assert_eq!(demo.slots[SPADE_SLOT], Some(Tool::Spade));
        assert_eq!(demo.slots[TWEEZERS_SLOT], Some(Tool::Tweezers));
        assert!(
            demo.slots.iter().all(|s| s.is_some()),
            "one tool starts in every painted bay"
        );
        assert_eq!(demo.plants.len(), PLANT_POS.len());
        assert!(demo.plants.iter().all(|p| !p.planted));
        assert!(demo.plants.iter().all(|p| p.water == 1.0));
        assert!(demo.falls.is_empty());
        assert!(!demo.pouring);
        assert!(demo.over);
    }

    /// The demo opens with the time-scale field at 1x, its drag unheld,
    /// and the four labels — "1x", "2x", "4x", "8x" — built from the
    /// embedded font.
    #[test]
    fn the_speed_field_starts_at_one() {
        let demo = Demo::new(Assets::load());
        assert_eq!(demo.speed, 0, "1x at launch");
        assert!(demo.speed_press.is_none());
        for label in &demo.speed_labels {
            assert!(
                matches!(label, frost::Shape::Text { .. }),
                "the scale labels are text"
            );
        }
    }

    /// [speed_click] walks the table one step at a time — 1x, 2x, 4x, 8x
    /// — and wraps from the last scale back to the first.
    #[test]
    fn the_speed_click_walks_the_table_and_wraps() {
        assert_eq!(speed_click(0), 1);
        assert_eq!(speed_click(1), 2);
        assert_eq!(speed_click(2), 3);
        assert_eq!(speed_click(3), 0, "8x wraps back to 1x");
        let mut i = 0;
        for _ in 0..TIME_SCALES.len() {
            i = speed_click(i);
        }
        assert_eq!(i, 0, "a full round of clicks returns home");
    }

    /// [speed_drag] steers by distance: one [TIME_SCALES] step per
    /// `SPEED_STEP` pixels, rounded to the nearest step, right faster and
    /// left slower, clamped to the table's ends — the drag never wraps.
    #[test]
    fn the_speed_drag_pulls_right_faster_and_left_slower() {
        assert_eq!(speed_drag(0, 0.0), 0);
        assert_eq!(
            speed_drag(0, SPEED_STEP * 0.4),
            0,
            "rounds to the nearest step"
        );
        assert_eq!(speed_drag(0, SPEED_STEP * 0.6), 1);
        assert_eq!(
            speed_drag(0, SPEED_STEP * 2.0),
            2,
            "a long pull spans steps"
        );
        assert_eq!(speed_drag(1, -SPEED_STEP), 1 - 1);
        assert_eq!(speed_drag(1, -SPEED_STEP * 9.0), 0, "clamps at 1x");
        assert_eq!(speed_drag(3, SPEED_STEP * 9.0), 3, "clamps at 8x");
        assert_eq!(speed_drag(3, -SPEED_STEP * 2.0), 1);
    }

    /// The field sits mid top of the window: centered across the width,
    /// `SPEED_TOP` clear of the top border, riding the border as the
    /// window grows; a point inside the box — to its very edge — hits,
    /// a pixel beyond misses, and the window's center misses.
    #[test]
    fn the_speed_field_sits_mid_top() {
        let h = 1080.0;
        let c = speed_field_center(h);
        assert_eq!(c[0], 0.0, "mid-width");
        assert!(
            (c[1] - (h / 2.0 - SPEED_HALF[1] - SPEED_TOP)).abs() < 1e-6,
            "SPEED_TOP clear of the top border"
        );
        assert!(on_speed_field(c, h), "the center hits");
        assert!(
            on_speed_field([c[0] + SPEED_HALF[0], c[1]], h),
            "the edge counts"
        );
        assert!(!on_speed_field([c[0] + SPEED_HALF[0] + 1.0, c[1]], h));
        assert!(!on_speed_field([c[0], c[1] - SPEED_HALF[1] - 1.0], h));
        assert!(!on_speed_field([0.0, 0.0], h), "the window's center misses");
        // A taller window lifts its top border, and the field rides it.
        assert!(speed_field_center(2.0 * h)[1] > c[1]);
    }

    /// The shelf is the sprite's shelf. [SEEDED_SLOTS] puts one tool in each
    /// of the four bays `items.png` paints, in the order the panel reads them
    /// from the top — tweezers, spade, spray can, watering can; every bay is
    /// a real opening between two boards and they march down the panel; every
    /// resting tool is fitted to its own bay, centred on its opening, standing
    /// on its board, and kept under the ceiling its click cell gives it; and
    /// the four click cells tile the panel edge to edge, so no press can fall
    /// between them.
    #[test]
    fn the_shelf_holds_one_tool_per_painted_bay() {
        assert_eq!(
            SEEDED_SLOTS,
            [
                Some(Tool::Tweezers),
                Some(Tool::Spade),
                Some(Tool::SprayCan),
                Some(Tool::WaterCan)
            ]
        );
        for i in 0..SLOTS {
            let [_, rest] = BAYS[i];
            assert!(
                rest - bay_ceiling(i) > BAY_INSET,
                "bay {i} has no room to stand a tool in"
            );
            // A bay's ceiling is the cut above it, and every cut runs through
            // the board between two bays: below the line the bay above stands
            // on, above the line this bay stands on.
            if i > 0 {
                let [_, above] = BAYS[i - 1];
                assert!(
                    above < BAY_CUTS[i] && BAY_CUTS[i] < rest,
                    "cut {i} must run through the board between bays"
                );
            }
        }
        // The check runs on the drawing, not the node: a sprite's content box
        // sits off-centre in its texture, which is exactly what `slot_rest`
        // exists to correct. And it runs on every tool in every bay, not just
        // the seeded layout, because the player can park any of them
        // anywhere: what a bay guarantees is the same whoever stands in it,
        // even though the size a tool is drawn at — and so where its node
        // goes — is computed for that tool in that bay.
        for (i, tool) in (0..SLOTS).flat_map(|i| ALL_TOOLS.map(|tool| (i, tool))) {
            let [dw, dh] = tool.drawn_size();
            let (t, s) = slot_rest(i, tool);
            assert!(s > 0.0, "the fit must be a scale, not a mirror");
            let box_ = tool.drawn();
            let img = tool.image_size();
            let lx = (box_[0][0] + box_[1][0]) / 2.0 - img[0] / 2.0;
            let ly = img[1] / 2.0 - (box_[0][1] + box_[1][1]) / 2.0;
            // Back into the panel's image pixels, y down.
            let cx = ITEMS_SIZE[0] / 2.0 + t[0] + s * lx;
            let cy = ITEMS_SIZE[1] / 2.0 - (t[1] + s * ly);
            let [opening, rest] = BAYS[i];
            assert!(
                (cx - opening).abs() < 1e-2,
                "{tool:?} in bay {i} must sit on the opening centre, not {cx}"
            );
            assert!(
                (cy + s * dh / 2.0 - rest).abs() < 1e-2,
                "{tool:?} must stand on bay {i}'s board"
            );
            assert!(
                cy - s * dh / 2.0 >= bay_ceiling(i) + BAY_INSET - 1e-2,
                "{tool:?} in bay {i} must keep its inset of air overhead"
            );
            assert!(
                dw * s <= BAY_WIDTH - 2.0 * BAY_INSET + 1e-2,
                "the tool must fit between the bay's jambs"
            );
            // Every pixel of the drawing answers to its own bay's click cell
            // — what `bay_ceiling` promises is kept in the picture.
            assert!(
                cy - s * dh / 2.0 >= BAY_CUTS[i] - 1e-2
                    && cy + s * dh / 2.0 <= BAY_CUTS[i + 1] + 1e-2,
                "{tool:?} is drawn outside its own click cell"
            );
            assert!(
                (s - tool_scale(tool)).abs() < 1e-6,
                "`slot_rest` must draw at the scale `tool_scale` gives"
            );
        }
        // The click cells tile the panel: edge to edge, in order, each one
        // covering its own bay's opening.
        assert_eq!(BAY_CUTS[0], 0.0);
        assert_eq!(BAY_CUTS[SLOTS], ITEMS_SIZE[1]);
        for i in 0..SLOTS {
            assert!(BAY_CUTS[i] < BAY_CUTS[i + 1], "cell {i} is empty");
            let mid = (BAY_CUTS[i] + BAY_CUTS[i + 1]) / 2.0;
            assert!((ITEMS_SIZE[1] / 2.0 - slot_local(i)[1] - mid).abs() < 1e-3);
            let [_, rest] = BAYS[i];
            assert!(
                BAY_CUTS[i] <= bay_ceiling(i) + 1e-2 && BAY_CUTS[i + 1] >= rest,
                "cell {i} must cover its whole bay, board front and all"
            );
            // Every interior cut is the middle of the board it runs through:
            // midway between the rest line of the bay above — the front of
            // its lit top — and that same board's bright lower edge, which
            // the sprite puts at `y` 273, 490, and 708 at the opening's
            // centre.
            if i > 0 {
                const BOARD_EDGES: [f32; SLOTS - 1] = [273.0, 490.0, 708.0];
                let [_, above] = BAYS[i - 1];
                let middle = (above + BOARD_EDGES[i - 1]) / 2.0;
                assert!(
                    (BAY_CUTS[i] - middle).abs() < 1.0,
                    "cut {i} must run through the middle of its board, not {middle}"
                );
            }
            let [hw, hh] = slot_half(i);
            assert_eq!(hw, ITEMS_SIZE[0] / 2.0);
            // The cell spans exactly its own cut, edge for edge.
            let y = slot_local(i)[1];
            let top = ITEMS_SIZE[1] / 2.0 - (y + hh);
            let bottom = ITEMS_SIZE[1] / 2.0 - (y - hh);
            assert!(
                (top - BAY_CUTS[i]).abs() < 1e-3 && (bottom - BAY_CUTS[i + 1]).abs() < 1e-3,
                "cell {i} must span exactly its own cut"
            );
        }
    }

    /// A tool is one size, wherever it stands. The scale is the tightest fit
    /// on the shelf — every tool fits every bay, and moving a tool along the
    /// shelf never resizes it — so the picture stays a shelf of things at one
    /// consistent scale rather than four sprites each fitted to its own
    /// cubbyhole. That the tightest fit comes out nearly the same everywhere
    /// is the shelf's own geometry, not a fitted guess: the boards of
    /// `items.png` are evenly spaced, so a bay reading much shallower than
    /// its neighbours means a rest line was taken off the wrong board.
    #[test]
    fn a_tool_is_the_same_size_in_every_bay() {
        let wide = |tool: Tool| tool_scale(tool) * tool.drawn_size()[0];
        let high = |tool: Tool| tool_scale(tool) * tool.drawn_size()[1];
        // The picture itself, to the pixel, top bay to bottom. Nothing else
        // in this test is a magic number, so this is where a change to the
        // bays shows up — rather than on the panel, between two boards that
        // were measured once and never looked at again.
        let seeded: Vec<[u32; 2]> = (0..SLOTS)
            .map(|i| SEEDED_SLOTS[i].unwrap())
            .map(|tool| [(wide(tool) + 0.5) as u32, (high(tool) + 0.5) as u32])
            .collect();
        assert_eq!(seeded, vec![[196, 133], [196, 172], [96, 195], [196, 148]]);
        for tool in ALL_TOOLS {
            for i in 0..SLOTS {
                // One scale wherever it stands...
                assert!(
                    (slot_rest(i, tool).1 - tool_scale(tool)).abs() < 1e-9,
                    "{tool:?} changed size between bays"
                );
                // ...and it fits every bay, so nothing is drawn larger than
                // the space it stands in and no bay's click cell is outgrown.
                assert!(
                    tool_scale(tool) <= bay_scale(i, tool.drawn_size()) + 1e-9,
                    "{tool:?} at scale {} does not fit bay {i}",
                    tool_scale(tool)
                );
            }
            // The scale is the largest with that property, dictated by some
            // real bay rather than picked: a bigger one would not fit.
            assert!(
                (0..SLOTS)
                    .any(|i| (tool_scale(tool) - bay_scale(i, tool.drawn_size())).abs() < 1e-9),
                "{tool:?}'s size is dictated by no bay, so it is leaving room on the shelf"
            );
        }
        // The three wide, low shapes take the opening from jamb to jamb; the
        // spray can is the one tool the shelf's height, not its width, stops.
        for tool in [Tool::Tweezers, Tool::Spade, Tool::WaterCan] {
            assert!(
                (wide(tool) - (BAY_WIDTH - 2.0 * BAY_INSET)).abs() < 1e-3,
                "{tool:?} should span the opening from jamb to jamb"
            );
        }
        assert!(
            (high(Tool::SprayCan) - (BAYS[SLOTS - 1][1] - bay_ceiling(SLOTS - 1) - BAY_INSET))
                .abs()
                < 1e-3,
            "the spray can should stand to the tightest headroom on the shelf"
        );
        // The boards are evenly spaced in the art, so the bays they carve are
        // near enough the same height. This is the check that each rest line
        // was read off the board its bay stands on: a line taken too high or
        // too low shows up here as one shallow bay, long before it shows up
        // on the panel as a tool floating above its board.
        let first = BAYS[0][1] - bay_ceiling(0) - BAY_INSET;
        for i in 1..SLOTS {
            let h = BAYS[i][1] - bay_ceiling(i) - BAY_INSET;
            assert!(
                (h - first).abs() < 10.0,
                "bay {i} stands {h} tall against bay 0's {first}, and the boards are evenly spaced"
            );
        }
    }

    /// The dig is three beats and one path: the spade is shoved forward
    /// along its own line to the full `DIG_PUSH_DIST`, held buried for
    /// exactly `DIG_HOLD`, then arches home — and the stroke starts and ends
    /// on the anchor, the tool exactly as drawn.
    #[test]
    fn a_dig_stroke_pushes_holds_and_arches_home_to_the_anchor() {
        assert_eq!(dig_offset(0.0), [0.0, 0.0]);
        assert_eq!(dig_offset(1.0), [0.0, 0.0]);
        let at = |s: f32| dig_offset(s / DIG_TIME);
        // Forward is the way the blade points: left and down.
        let push = [
            DIG_PUSH_DIST * spade_blade().cos(),
            DIG_PUSH_DIST * spade_blade().sin(),
        ];
        assert!(push[0] < 0.0 && push[1] < 0.0, "forward is left and down");
        // Pushed to the full reach by the end of the push...
        let p = at(DIG_PUSH);
        assert!((p[0] - push[0]).abs() < 1e-4 && (p[1] - push[1]).abs() < 1e-4);
        // ...held there, to the pixel, for the whole wait...
        let mut s = DIG_PUSH;
        while s <= DIG_PUSH + DIG_HOLD {
            let o = at(s);
            assert!(
                (o[0] - push[0]).abs() < 1e-6 && (o[1] - push[1]).abs() < 1e-6,
                "the spade must hold its bite at {s} s"
            );
            s += 0.01;
        }
        // ...and home along the arch, crowning up and to the left of the
        // cursor at the arch's middle, and landing only at its end.
        let crown = at(DIG_PUSH + DIG_HOLD + DIG_ARC / 2.0);
        assert!(
            (crown[0] - DIG_ARC_CROWN[0]).abs() < 1e-3
                && (crown[1] - DIG_ARC_CROWN[1]).abs() < 1e-3,
            "the arch must pass through its crown, not {crown:?}"
        );
        assert!(
            crown[0] < 0.0 && crown[1] > 0.0,
            "the way home goes left and up"
        );
        assert_ne!(at(DIG_TIME - 0.01), [0.0, 0.0], "the arch lands at its end");
        // The whole stroke is a walk, not a jump: no millisecond of it moves
        // the tool more than the arch's own length, walked at the ease's top
        // speed, allows.
        let reach = (DIG_PUSH_DIST * DIG_PUSH_DIST
            + (DIG_ARC_CROWN[0].powi(2) + DIG_ARC_CROWN[1].powi(2))
                .sqrt()
                .powi(2))
        .sqrt();
        let stride = 1.5 * 2.0 * (2.0 * reach) / DIG_ARC * 0.001;
        let mut prev = at(0.0);
        let mut max_step = 0.0f32;
        let mut s = 0.0;
        while s <= DIG_TIME {
            let o = at(s);
            max_step = max_step.max(((o[0] - prev[0]).powi(2) + (o[1] - prev[1]).powi(2)).sqrt());
            prev = o;
            s += 0.001;
        }
        assert!(
            max_step < stride,
            "the stroke jumps {max_step} px in a millisecond, over its {stride} px pace"
        );
    }

    /// The push direction is the sprite's own, not a guess: the point lies
    /// 188 px left and 131 px below the grip, so the dig drives the tool
    /// down and to the left along the line the art already draws — 34.9°
    /// below the horizontal — instead of inventing an angle of its own.
    #[test]
    fn the_dig_pushes_along_the_line_the_art_draws() {
        let tilt = spade_blade();
        assert!(tilt < -std::f32::consts::FRAC_PI_2);
        assert!(tilt > -std::f32::consts::PI);
        assert!((tilt.to_degrees() + 180.0 - 34.87).abs() < 0.1);
        // The tilt and the grip→point chord are the same line: the push can
        // only run along the blade, never across it.
        let (ex, ey) = (
            (SPADE_TIP_LOCAL[0] - SPADE_GRIP_LOCAL[0]) * SPADE_SCALE,
            (SPADE_TIP_LOCAL[1] - SPADE_GRIP_LOCAL[1]) * SPADE_SCALE,
        );
        assert!(
            (tilt.cos() * ey - tilt.sin() * ex).abs() < 1e-3 * ex.abs(),
            "the push must run along the blade, not across it"
        );
    }

    /// The stroke's shape: the deepest the tool ever goes is its bite, held
    /// through the wait with nothing but the push behind it, and the arch
    /// swings further from the cursor than the bite does — a loop out over
    /// the ground it broke, not a rewind along the push.
    #[test]
    fn the_dig_bites_deepest_and_arches_widest() {
        let mut lowest = f32::MAX;
        let mut widest = 0.0f32;
        let mut s = 0.0;
        while s <= DIG_TIME {
            let o = dig_offset(s / DIG_TIME);
            lowest = lowest.min(o[1]);
            widest = widest.max((o[0] * o[0] + o[1] * o[1]).sqrt());
            s += 0.001;
        }
        assert!(
            (lowest - DIG_PUSH_DIST * spade_blade().sin()).abs() < 1e-3,
            "the bottom of the stroke must be the push itself"
        );
        assert!(
            widest > DIG_PUSH_DIST,
            "the arch must clear the bite's reach, and does {widest}"
        );
    }

    /// A docked tool's drawing sits inside its cell. The dock's scale fits a
    /// tool's *content*, and `cell_rest` centres that content on the cell —
    /// so a sprite exported onto a canvas wider than its drawing, like the
    /// tweezers with their jaws 37 px off the canvas's middle, no longer
    /// hangs out of the dock it was parked in.
    #[test]
    fn a_docked_tool_sits_inside_its_cell() {
        for tool in ALL_TOOLS {
            let s = held_scale(tool.drawn_size());
            let [dw, dh] = tool.drawn_size();
            let box_ = tool.drawn();
            let img = tool.image_size();
            // Where the canvas puts the drawing's centre, in node space.
            let ox = (box_[0][0] + box_[1][0]) / 2.0 - img[0] / 2.0;
            let oy = img[1] / 2.0 - (box_[0][1] + box_[1][1]) / 2.0;
            for cell in 0..2 {
                let p = cell_rest(cell, tool);
                let [cx, cy] = held_local(cell);
                // The renderer anchors the canvas; the drawing lands half an
                // `ox` further along, and that is what must be inside.
                let dx = p[0] + s * ox;
                let dy = p[1] + s * oy;
                assert!(
                    (dx - cx).abs() < 1e-3,
                    "{tool:?} in cell {cell}: its drawing sits {} px off the centre",
                    dx - cx
                );
                assert!(
                    (dy - cy).abs() < 1e-3,
                    "{tool:?} in cell {cell}: its drawing sits {} px off the line",
                    dy - cy
                );
                assert!(
                    s * dw / 2.0 <= HELD_SIZE[0] / 4.0 - HELD_INSET + 1e-3,
                    "{tool:?} is wider than its cell"
                );
                assert!(
                    s * dh / 2.0 <= HELD_SIZE[1] / 2.0 - HELD_INSET + 1e-3,
                    "{tool:?} is taller than its cell"
                );
            }
        }
        // The tweezers are the case that broke: their drawing is far enough
        // off-centre in its canvas that anchoring the canvas would throw a
        // good part of the tool outside the dock.
        let off = drawn_centring(Tool::Tweezers, held_scale(Tool::Tweezers.drawn_size()));
        assert!(
            off[0].abs() > 10.0,
            "the tweezers' canvas is badly off-centre"
        );
        assert!(
            off[0].abs()
                + held_scale(Tool::Tweezers.drawn_size()) * Tool::Tweezers.drawn_size()[0] / 2.0
                > HELD_SIZE[0] / 4.0,
            "uncorrected, the tweezers must be the tool that overhangs"
        );
    }

    /// The swap's mirrored arcs: each tool leaves its cell, bends off
    /// the cells' chord by half the slot distance — the left one below,
    /// the right one above — and lands on the other cell's spot.
    #[test]
    fn the_swap_arcs_rise_and_dip_half_a_slot_apart() {
        let d = held_local(1)[0] - held_local(0)[0];
        let down = CellFlight {
            from: held_local(0),
            to: held_local(1),
            bump: -d / 2.0,
            t: 0.0,
            dur: 1.0,
        };
        let up = CellFlight {
            from: held_local(1),
            to: held_local(0),
            bump: d / 2.0,
            t: 0.0,
            dur: 1.0,
        };
        // Both legs start on their own cell and land on the other's.
        for (fly, a, b) in [
            (down, held_local(0), held_local(1)),
            (up, held_local(1), held_local(0)),
        ] {
            let (start, end) = (cell_flight_pos(&fly, 0.0), cell_flight_pos(&fly, 1.0));
            assert!(
                (start[0] - a[0]).abs() < 1e-4
                    && (start[1] - a[1]).abs() < 1e-4
                    && (end[0] - b[0]).abs() < 1e-4
                    && (end[1] - b[1]).abs() < 1e-4,
                "the leg misses its cells: {start:?} -> {end:?}"
            );
        }
        let low = cell_flight_pos(&down, 0.5);
        let high = cell_flight_pos(&up, 0.5);
        // The midpoints stand over the cells' chord — dead center, a
        // half slot below and above it respectively.
        assert!(low[0].abs() < 1e-4 && high[0].abs() < 1e-4, "centered");
        assert!(
            (low[1] + d / 2.0).abs() < 1e-4 && (high[1] - d / 2.0).abs() < 1e-4,
            "the bends are off: {low:?} vs {high:?}"
        );
        // The dip is the dip and the rise is the rise: flanking points
        // stay between the bend and the line.
        let a = cell_flight_pos(&down, 0.35);
        let b = cell_flight_pos(&down, 0.65);
        assert!(a[1] > low[1] && b[1] > low[1] && a[1] < 0.0 && b[1] < 0.0);
        assert!((a[1] - b[1]).abs() < 1e-3, "the arch is symmetric");
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
                .compose(&frost::Transform::translate([100.0, 200.0])),
            ..Default::default()
        };
        assert_eq!(
            root_anchor(&node),
            [100.0, 200.0],
            "the anchor is the root joint, in x and y"
        );
    }

    /// The bench starts bare: no plant grows, and no reserve drains,
    /// until a seed lands in a slot — then only the planted slot grows,
    /// the other slots' seeds sitting untouched, full-watered.
    #[test]
    fn the_bench_stays_bare_until_a_seed_lands() {
        // A long frame with the bench bare: nothing would grow or drain
        // even if the clocks ran.
        let mut demo = Demo::new(Assets::load());
        let (active, started) = demo.grow_plants(9.0);
        assert_eq!(active, 0);
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
        let (active, started) = demo.grow_plants(dt);
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
        let (active, started) = demo.grow_plants(dt);
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

    /// The snapshot's round trip: capture the demo's state, perturb every
    /// part the snapshot carries — the scalars, the tools, a plant's water
    /// and clocks, a carried fruit, a seed's flight, a fall, the worms,
    /// and all three random streams — and restore: the perturbations are
    /// fully reverted, the random streams included, so a reloaded game
    /// continues the exact same streams.
    #[test]
    fn a_snapshot_round_trips_the_demo_state() {
        let mut demo = Demo::new(Assets::load());
        demo.picking = Some(Pick::Plant(0, 2));
        demo.flying = Some(Fly {
            tween: frost::Tween::new([10.0, 20.0], [30.0, 40.0], FLY_TIME)
                .repeat(frost::Repeat::Once),
            time: 0.5,
            landing: Landing::Slot(1),
        });
        demo.falls.push(Fall::new([5.0, 6.0], [7.0, 8.0]));
        let snapshot = demo.capture_state();

        // Perturb everything the snapshot carries.
        demo.time += 10.0;
        demo.acc += 0.5;
        demo.over = false;
        demo.ever_planted = true;
        demo.slots[0] = Some(Tool::SprayCan);
        demo.active = Some(Tool::WaterCan);
        demo.held = Some(Tool::SprayCan);
        demo.angle = 0.7;
        demo.rotation = frost::Tween::new(0.7, 0.0, 1.0).repeat(frost::Repeat::Once);
        demo.burst = Some(0.3);
        demo.dig = Some(0.3);
        demo.showing_spray2 = true;
        demo.basket_seed = false;
        demo.picking = None;
        demo.flying = None;
        demo.falls.clear();
        demo.plants[0].planted = true;
        demo.plants[0].water = 0.25;
        demo.plants[0].dryness = 0.5;
        demo.plants[0].plant.step(5.0);
        demo.plants[0].plant.age(2.0);
        demo.vipers
            .step(1.0, &[[[0.0, 0.0]; vipers::LAYERS]; 1], &[]);
        demo.worms
            .step(1.0, &[[100.0, 100.0], [200.0, 100.0], [150.0, 200.0]]);
        let seed_before = demo.rng.state();
        demo.rng.in_range(0.0, 1.0);
        assert_ne!(demo.rng.state(), seed_before);
        demo.bugs.set_seed(snapshot.bug_seed.wrapping_add(1));

        // The restore reverts every perturbation.
        demo.restore(&snapshot);
        assert_eq!(demo.time, snapshot.time);
        assert_eq!(demo.acc, snapshot.acc);
        assert!(demo.over);
        assert!(!demo.ever_planted);
        assert_eq!(demo.slots, snapshot.slots);
        assert_eq!(demo.active, snapshot.active);
        assert_eq!(demo.held, snapshot.held);
        assert_eq!(demo.angle, snapshot.angle);
        assert_eq!(save::TurnState::from(&demo.rotation), snapshot.turn);
        assert_eq!(demo.burst, snapshot.burst);
        assert_eq!(demo.dig, snapshot.dig);
        assert_eq!(demo.showing_spray2, snapshot.showing_spray2);
        assert!(demo.basket_seed);
        assert!(!demo.pouring);
        // The carried fruit and the seed's flight come back, whole.
        match demo.picking {
            Some(Pick::Plant(pi, si)) => assert_eq!((pi, si), (0, 2)),
            _ => panic!("the picked plant fruit should come back"),
        }
        match demo.flying {
            Some(fly) => {
                assert_eq!(fly.time, 0.5);
                assert_eq!(fly.tween.from(), [10.0, 20.0]);
                assert_eq!(fly.tween.to(), [30.0, 40.0]);
                match fly.landing {
                    Landing::Slot(i) => assert_eq!(i, 1),
                    _ => panic!("the flight's landing should come back"),
                }
            }
            None => panic!("the seed's flight should come back"),
        }
        assert_eq!(
            demo.falls
                .iter()
                .map(save::FallState::from)
                .collect::<Vec<_>>(),
            snapshot.falls
        );
        // The plant's water and clocks come back.
        assert!(!demo.plants[0].planted);
        assert_eq!(demo.plants[0].water, 1.0);
        assert_eq!(demo.plants[0].dryness, 0.0);
        assert_eq!(
            plant::PlantState::from(&demo.plants[0].plant),
            snapshot.plants[0].plant
        );
        // The swarms and all three random streams resume exactly where
        // they were.
        assert_eq!(demo.vipers.state(), snapshot.vipers);
        assert_eq!(demo.bugs.state(), snapshot.bugs);
        assert_eq!(demo.worms.state(), snapshot.worms);
        assert_eq!(demo.rng.state(), snapshot.seed);
    }

    /// The version gate: a snapshot written by another format version is
    /// rejected, and one written by this version parses.
    #[test]
    fn a_snapshot_from_another_version_is_rejected() {
        let demo = Demo::new(Assets::load());
        let mut snapshot = demo.capture_state();
        let text = ron::to_string(&snapshot).unwrap();
        assert_eq!(save::parse(&text).unwrap().version, save::VERSION);
        snapshot.version = save::VERSION + 1;
        let text = ron::to_string(&snapshot).unwrap();
        assert!(save::parse(&text).is_err());
    }

    /// The working-directory fallback: a candidate whose parent is a file
    /// cannot be written, so the write lands in the next candidate — and
    /// the read skips both the missing first candidate and a stale one
    /// whose version differs, reporting the stale file's version mismatch
    /// when nothing else is usable.
    #[test]
    fn an_unusable_snapshot_candidate_falls_back_to_the_next_one() {
        let dir = std::env::temp_dir().join(format!("frost-save-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let demo = Demo::new(Assets::load());
        let snapshot = demo.capture_state();

        // The first candidate's parent is a plain file: the folder cannot
        // be created, so the write must land in the second candidate.
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, "a file, not a folder").unwrap();
        let first = blocked.join("snapshot.ron");
        let second = dir.join("snapshot.ron");
        let written = save::save_to(&[first.clone(), second.clone()], &snapshot).unwrap();
        assert_eq!(written, second);
        assert!(!first.exists());

        // The read skips the missing first candidate and finds the
        // snapshot in the second one.
        let loaded = save::load_from(&[first.clone(), second.clone()]).unwrap();
        assert_eq!(loaded.path, second);
        assert_eq!(loaded.snapshot, snapshot);

        // A first candidate that reads but whose version differs is
        // skipped too: a stale file never shadows a readable snapshot.
        let stale_dir = dir.join("stale");
        std::fs::create_dir_all(&stale_dir).unwrap();
        let stale_path = stale_dir.join("snapshot.ron");
        let mut stale = snapshot.clone();
        stale.version = save::VERSION + 1;
        std::fs::write(&stale_path, ron::to_string(&stale).unwrap()).unwrap();
        let loaded = save::load_from(&[stale_path.clone(), second.clone()]).unwrap();
        assert_eq!(loaded.path, second);
        assert_eq!(loaded.snapshot, snapshot);

        // When no candidate is usable, the stale file's error wins: it
        // names the version mismatch.
        let err = save::load_from(&[stale_path.clone(), dir.join("missing.ron")]).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("version"),
            "the error should name the mismatch: {message}"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The turn state's round trip: the captured tween's ends, elapsed
    /// time, leg duration, and repeat mode rebuild a tween that ticks to
    /// the same value at the same moment.
    #[test]
    fn the_turn_state_round_trips_the_tween() {
        let mut original = frost::Tween::new(0.0, 1.0, 2.0).repeat(frost::Repeat::PingPong);
        original.set_time(0.5);
        let state = save::TurnState::from(&original);
        let mut restored = state.into_tween();
        assert_eq!(restored.from(), 0.0);
        assert_eq!(restored.to(), 1.0);
        assert_eq!(restored.duration(), 2.0);
        assert_eq!(restored.time(), 0.5);
        assert_eq!(restored.repeat_mode(), frost::Repeat::PingPong);
        assert_eq!(restored.tick(0.0), original.tick(0.0));
        // A mid-travel capture resumes mid-travel: after the same tick,
        // both tweens agree.
        assert_eq!(restored.tick(0.25), original.tick(0.25));
    }
}
