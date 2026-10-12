//! Ember Hill — a jam puzzle game on FROST (theme: *autumn* / *to keep warm*).
//!
//! Shepherd a growing trail of ants through a side-view ant hill. Walking
//! onto a stranded ant recruits it to your trail's tail; only the tail can
//! be released, and it only settles on its own colored bed — so the trail
//! is a stack, and the tunnels are the shuffle device. Every ant asleep
//! in its bed, yourself included, before the first frost.
//!
//! Run: `RUST_LOG=info cargo run --example antifrost` — add
//! `-- --vsync off` to uncap the frame rate.
//! Rules: `examples/antifrost/DESIGN.md`

mod game;
mod level;
#[cfg(test)]
mod solver;

use frost::{
    Color, Context, KeyCode, MouseButton, Particle, ParticleShape, ParticleSystem, Process, Rng,
    Scene, SceneNode, SpriteFilter, Tile, Transform,
};
#[cfg(feature = "diag")]
use frost::{Diagnostics, DiagnosticsFlags};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// The hill's own voice: Barrfont for every word the game draws —
/// notes, HUD, win screen, the sleepy "z"s.
const GAME_FONT: &[u8] = include_bytes!("assets/fonts/Barrfont-Regular.ttf");
/// Diagnostics alone keeps Fira Code: the overlay is an instrument
/// panel, not a signpost, and its numbers want a coding face.
#[cfg(feature = "diag")]
const DIAG_FONT: &[u8] = include_bytes!("../../assets/fonts/FiraCode-VariableFont_wght.ttf");
/// The far background drifts at 0.85 of the camera's pan — almost one
/// with the front layer, the small lag being the whole depth cue. It is
/// only ever seen through the map's open cells: the map — its solid
/// tiles and the ground sheet framed around them — is a clipping plane
/// that scrolls with the board at 1.0, and the tunnels are the windows
/// cut into it. Eye candy; no geology is implied.
const DEEP_PARALLAX: f32 = 0.85;

const LEVELS: &str = include_str!("levels.ron");
/// The backdrop: one soil texture stretched cover-first across the window,
/// under every tile. The hairline gaps between tiles and the void past the
/// hill's edge read as earth instead of as a screen.
const SOIL: &[u8] = include_bytes!("assets/sprites/soil.png");

/// The bed, the user's own sprite: laid square on the bed's cell,
/// tinted nothing, lit like everything else that lives in the hill.
const BED: &[u8] = include_bytes!("assets/sprites/bed.png");

const TILE: f32 = 44.0;

/// World → screen magnification. One number scales the whole hill: cells,
/// beds, ants — everything that lives in the camera's space — while line
/// widths, the HUD, the frost rim, and the dead zone stay in screen
/// pixels. `4.0` fills the window with about five cells across.
const ZOOM: f32 = 4.0;
const HOP: f32 = 0.26; // seconds, player pixel glide
const TRAIL_HOP: f32 = 0.36; // ants glide slightly lazier
const UI: f32 = 1_000_000.0;

const TAU: f32 = std::f32::consts::TAU;

/// The camera eases toward its target exponentially: every `CAM_EASE_TAU`
/// seconds it closes 63 % of the remaining distance, which reads as a
/// classic ease-out follow and stays exact at any frame rate.
const CAM_EASE_TAU: f32 = 0.18;

/// Half-extent of the camera's dead zone, as a fraction of the window's
/// half-size: the world holds perfectly still while the queen walks
/// through the middle 25 % of the window in each axis.
const CAM_DEAD: f32 = 0.25;

/// The falling deck: eleven leaf sheets and the snowflake, each sheet
/// one instanced particle batch. The twelfth sheet waits — past par
/// the wind starts dealing it into the hand, and by double par the
/// sky plays nothing else.
const LEAVES: [&[u8]; 12] = [
    include_bytes!("assets/sprites/leaf1.png"),
    include_bytes!("assets/sprites/leaf2.png"),
    include_bytes!("assets/sprites/leaf3.png"),
    include_bytes!("assets/sprites/leaf4.png"),
    include_bytes!("assets/sprites/leaf5.png"),
    include_bytes!("assets/sprites/leaf6.png"),
    include_bytes!("assets/sprites/leaf7.png"),
    include_bytes!("assets/sprites/leaf8.png"),
    include_bytes!("assets/sprites/leaf9.png"),
    include_bytes!("assets/sprites/leaf10.png"),
    include_bytes!("assets/sprites/leaf11.png"),
    include_bytes!("assets/sprites/snow_flake.png"),
];

/// The snowflake's slot in `LEAVES` — never dealt as a leaf.
const SNOW_SHEET: usize = LEAVES.len() - 1;

/// The hand-tailored ant — four right-facing frames of a walk cycle, drawn
/// top-down like the board reads it. The sheets are large (~970 px wide),
/// so they are baked down before upload, not scaled at draw time.
const ANT_FRAMES: [&[u8]; 4] = [
    include_bytes!("assets/sprites/ant1.png"),
    include_bytes!("assets/sprites/ant2.png"),
    include_bytes!("assets/sprites/ant3.png"),
    include_bytes!("assets/sprites/ant4.png"),
];

/// Baked frame texture width in pixels. The originals are ~970 px wide;
/// at `ZOOM` an ant draws ~172 user pixels — ~344 on a 2× display — so a
/// 384-wide bake covers the sheet near 1:1 where it matters, while still
/// pre-filtering the ~2.5× downscale the engine's mipmap-free bilinear
/// sampler would otherwise do one texel at a time, making the charcoal
/// strokes shimmer and thin strokes vanish.
const ANT_TEX: u32 = 384;

/// Walk-cycle frames per second. The cycle turns always — moving or not,
/// every awake ant walks — so the animation reads like a living colony:
/// at 6 fps the whole four-frame cycle unrolls in two thirds of a second,
/// comfortably followable by eye.
const ANT_WALK_FPS: f32 = 6.0;

/// How long an ant that has just been tucked into its bed blazes before
/// it settles into the dim colour of sleep. A celebration, not a fire.
const LAND_FLASH: f32 = 1.1;

/// The ant's drawn width: a whisker under one cell, its height following
/// the frame's own aspect.
const ANT_W: f32 = TILE * 0.98;

/// Bakes one hand-tailored PNG into a sprite shape: a Lanczos downscale to
/// `ANT_TEX` wide, re-encoded in memory for `Shape::sprite_bytes`.
fn ant_frame(png: &[u8]) -> Result<frost::Shape, String> {
    use image::{ExtendedColorType, ImageEncoder};
    let rgba = image::load_from_memory(png)
        .map_err(|err| err.to_string())?
        .into_rgba8();
    let (w, h) = (rgba.width().max(1), rgba.height());
    let th = ((ANT_TEX as f32) * h as f32 / w as f32).round().max(1.0) as u32;
    let small = image::imageops::resize(&rgba, ANT_TEX, th, image::imageops::FilterType::Lanczos3);
    let mut baked = Vec::new();
    image::codecs::png::PngEncoder::new(&mut baked)
        .write_image(small.as_raw(), ANT_TEX, th, ExtendedColorType::Rgba8)
        .map_err(|err| err.to_string())?;
    frost::Shape::sprite_bytes(&baked).map_err(|err| err.to_string())
}

/// The surrounding ground for one hill: a single giant soil sprite —
/// seamless because it is one sprite — with the hill's rectangle
/// punched out of its middle as a transparent window. The engine offers
/// no stencil, so the clip is baked into the sprite's alpha instead.
/// It rides the board's own speed, so the hill and its ground scroll as
/// one plane, and only the far sprite drifting behind the hole gives
/// the tunnels depth. The hole is tucked 2 px inside the hill's
/// rectangle, so the border rock's outer hairline shows ground rather
/// than depth. Same sprite as the walls wear, stretched a thousand
/// times wider than a tile: the ground can only whisper the grain,
/// while the walls shout it.
fn ground_sprite(cols: usize, rows: usize) -> Result<(frost::Shape, [f32; 2]), String> {
    use image::{ExtendedColorType, ImageEncoder};
    // The reach: the hill plus a margin wide enough that no window, at
    // any camera a hill can show, ever finds the sprite's edge. The
    // texture stays modest — the ground reads soft, which suits it.
    const MARGIN: f32 = 2600.0;
    const TEX_W: u32 = 1536;
    let cw = cols as f32 * TILE * ZOOM;
    let ch = rows as f32 * TILE * ZOOM;
    let span = [cw + 2.0 * MARGIN, ch + 2.0 * MARGIN];
    let th = ((span[1] / span[0]) * TEX_W as f32).round().max(1.0) as u32;
    let soil = image::load_from_memory(SOIL)
        .map_err(|err| err.to_string())?
        .into_rgba8();
    let mut img = image::imageops::resize(&soil, TEX_W, th, image::imageops::FilterType::Lanczos3);
    let hw = (((cw - 4.0) / span[0]) * TEX_W as f32).round().max(1.0) as u32;
    let hh = (((ch - 4.0) / span[1]) * th as f32).round().max(1.0) as u32;
    let (x0, y0) = ((TEX_W - hw) / 2, (th - hh) / 2);
    for y in y0..(y0 + hh).min(th) {
        for px in x0..(x0 + hw).min(TEX_W) {
            *img.get_pixel_mut(px, y) = image::Rgba([0, 0, 0, 0]);
        }
    }
    let mut baked = Vec::new();
    image::codecs::png::PngEncoder::new(&mut baked)
        .write_image(img.as_raw(), TEX_W, th, ExtendedColorType::Rgba8)
        .map_err(|err| err.to_string())?;
    frost::Shape::sprite_bytes(&baked)
        .map(|shape| (shape, span))
        .map_err(|err| err.to_string())
}

/// Leaves drifting over the hill at any moment. Each owns a lane, so the
/// origins stay evenly spread across the window width as they recycle.
const N_LEAVES: usize = 30;

/// The sky's ceiling: the deck swells from thirty to eighty as the
/// weather comes in, newcomers called down by the storm itself.
const N_MAX: usize = 80;

/// The terminal gale, in px/s, at full frost — and it holds there:
/// the weather caps at double par. Its speed is the deck's own mean
/// fall speed, so a full storm streaks the sky at 45 degrees on
/// average, wind and gravity met in equal measure.
const WIND_MAX: f32 = 65.0;

/// Leaves draw above the ants (z ≈ 1.5) and below the frost rim and every
/// UI pixel.
const LEAF_Z: f32 = 1.8;

/// Sprite half-widths in pixels: a leaf draws 48–96 px wide, its height
/// following each PNG's own aspect — four times the ant, in keeping with
/// the zoomed world.
const LEAF_SIZE: (f32, f32) = (24.0, 48.0);

/// How the canopy drifts against the camera: 0.5 means the leaves slide
/// across the window at half the speed the world pans. No depth claim —
/// a real depth story would scale them too — just the eye-candy drift a
/// parallax layer with `speed: 0.5` would give, done on the batch nodes
/// directly because the leaf z (above the board, under the HUD) only
/// exists inside the base draw group, where `Layer`s cannot land.
const LEAF_PARALLAX: f32 = 0.5;

const C_BG: Color = Color {
    r: 0.045,
    g: 0.038,
    b: 0.033,
    a: 1.0,
};
/// The night the lights replace: the ambient every lit pixel is floored
/// at. This is how dark the dark is — the depth the ants' lamps cut.
const C_NIGHT: Color = Color {
    r: 0.14,
    g: 0.13,
    b: 0.15,
    a: 1.0,
};
/// An ant's lamp: reach in tiles. The rim stays hard: a penumbra costs
/// the shader nine shadow rays per light at every fragment that light
/// reaches, and nine times the arithmetic is exactly what the frame
/// budget cannot pay at 4K. Tile walls cast tile shadows.
const LAMP_REACH: f32 = 2.6;
const LAMP_PENUMBRA: f32 = 0.0;

/// Lamps breathe — ants are alive, not night-lights, and the breath
/// is meant to be *seen*. Asleep: a warm pool that swells and subsides,
/// a long slow haul of a breath — the dorm is never dark. Awake and
/// still: a clear deep sway. Working: the lamp roars and dips — the
/// swing grows to most of the light itself and the pace nearly
/// triples. Stopping lets the exertion sink on an exponential, so the
/// roar decays by degrees into the sway, and the queen's bed — hers
/// or any ant's — is the slowest breath of all.
const BREATH_SLEEP_BASE: f32 = 0.55;
const BREATH_SLEEP_AMP: f32 = 0.42;
/// Breaths per second: sleep is a four-and-a-half-second affair.
const BREATH_SLEEP_RATE: f32 = 0.22;
const BREATH_LAMP_BASE: f32 = 1.4;
const BREATH_CALM_AMP: f32 = 0.6;
const BREATH_WORK_AMP: f32 = 1.25;
const BREATH_CALM_RATE: f32 = 0.45;
const BREATH_WORK_RATE: f32 = 1.15;
/// Exertion time constants: up brisk, down gradual.
const BREATH_UP_TAU: f32 = 0.4;
const BREATH_DOWN_TAU: f32 = 2.6;
/// A full-speed ant's hop, in px per second, saturates exertion.
const BREATH_FULL_SPEED: f32 = 520.0;

/// The corner arc's control arm, in tile lengths: at 90° the hop bends
/// into a quarter-circle of roughly this reach; smaller cuts tighter.
const HOP_ARC_K: f32 = 0.55;
/// How fast a facing turns, in rad/s — a 180 spins in about half a
/// second, a tuck-in lifts the head a touch slower.
const TURN_RATE: f32 = 7.0;
const C_DIRT: Color = Color {
    r: 0.44,
    g: 0.31,
    b: 0.165,
    a: 1.0,
};
const C_TEXT: Color = Color {
    r: 0.93,
    g: 0.87,
    b: 0.76,
    a: 1.0,
};
const C_DIM: Color = Color {
    r: 0.62,
    g: 0.55,
    b: 0.45,
    a: 1.0,
};
const C_NOTE: Color = Color {
    r: 1.0,
    g: 0.72,
    b: 0.35,
    a: 1.0,
};

fn ant_color(c: game::AntColor) -> Color {
    match c {
        game::AntColor::White => Color {
            r: 0.97,
            g: 0.97,
            b: 0.93,
            a: 1.0,
        },
        game::AntColor::Red => Color {
            r: 0.93,
            g: 0.30,
            b: 0.24,
            a: 1.0,
        },
        game::AntColor::Yellow => Color {
            r: 0.95,
            g: 0.78,
            b: 0.22,
            a: 1.0,
        },
        game::AntColor::Green => Color {
            r: 0.45,
            g: 0.86,
            b: 0.36,
            a: 1.0,
        },
        game::AntColor::Blue => Color {
            r: 0.36,
            g: 0.62,
            b: 0.98,
            a: 1.0,
        },
    }
}

fn shade(mut c: Color, k: f32) -> Color {
    c.r *= k;
    c.g *= k;
    c.b *= k;
    c
}

fn closer(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01
}

/// The facing a `dir` turns a walker to — one mapping shared by the key
/// moves and the click route, so both kinds of step point the same way.
fn face_of(dir: game::Dir) -> [f32; 2] {
    match dir {
        game::Dir::Left => [-1.0, 0.0],
        game::Dir::Right => [1.0, 0.0],
        game::Dir::Up => [0.0, 1.0],
        game::Dir::Down => [0.0, -1.0],
    }
}

/// Per-ant pixel glide: where it is drawn, where it is heading.
/// One hop's path: a cubic between two cells whose control arms lie
/// along the directions of travel at either end. Entering and leaving
/// the same way, the arms fall on the line and the hop is straight;
/// at a corner they stand out and the hop bends into a quarter
/// circle — position and facing curve together, because both read the
/// same curve.
#[derive(Clone, Copy)]
struct Hop {
    a: [f32; 2],
    b: [f32; 2],
    c: [f32; 2],
    d: [f32; 2],
    t: f32,
    dur: f32,
}

impl Hop {
    /// A finished hop parked at `p`, going nowhere — for snaps and spawns.
    fn flat(p: [f32; 2], dur: f32) -> Self {
        Self {
            a: p,
            b: p,
            c: p,
            d: p,
            t: 1.0,
            dur,
        }
    }
    /// Re-armed from `from` to `to`, entering along `entry` and leaving
    /// along `exit` (unit axes): perpendicular, the arms bend the
    /// corner; aligned, the arms lie flat and it is a straight line.
    fn to(&mut self, from: [f32; 2], to: [f32; 2], entry: [f32; 2], exit: [f32; 2], dur: f32) {
        let dot = entry[0] * exit[0] + entry[1] * exit[1];
        let k = if dot.abs() < 0.5 {
            HOP_ARC_K * TILE
        } else {
            0.0
        };
        *self = Self {
            a: from,
            b: [from[0] + entry[0] * k, from[1] + entry[1] * k],
            c: [to[0] - exit[0] * k, to[1] - exit[1] * k],
            d: to,
            t: 0.0,
            dur,
        };
    }
    fn tick(&mut self, dt: f32) -> [f32; 2] {
        self.t = (self.t + dt / self.dur).min(1.0);
        let t = self.t;
        let u = 1.0 - t;
        let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
        [
            w[0] * self.a[0] + w[1] * self.b[0] + w[2] * self.c[0] + w[3] * self.d[0],
            w[0] * self.a[1] + w[1] * self.b[1] + w[2] * self.c[1] + w[3] * self.d[1],
        ]
    }
}

/// Turns `from` toward `to` by at most `max` radians.
fn turn_toward(from: [f32; 2], to: [f32; 2], max: f32) -> [f32; 2] {
    let a = (from[0] * to[1] - from[1] * to[0])
        .atan2(from[0] * to[0] + from[1] * to[1])
        .clamp(-max, max);
    let (s, c) = a.sin_cos();
    [from[0] * c - from[1] * s, from[0] * s + from[1] * c]
}

/// One z off a sleeper's breath: it drifts up, wanders a little on the
/// way, and its glyph grows from a small z to a full Z before it goes.
struct Zzz {
    at: [f32; 2],
    vel: [f32; 2],
    age: f32,
    ttl: f32,
    seed: f32,
}

/// The verdict splash: when the last ant lies down, a word spins up
/// from a speck until it fills half the screen, rests a heartbeat,
/// and fades to the hill's own quiet. The growth time is drawn
/// randomly, so the spin — one clean turn and the landing — comes to
/// rest on a random tilt anywhere within 45 degrees of level.
struct Splash {
    text: String,
    t: f32,
    grow: f32,
    end: f32,
}

impl Splash {
    const HOLD: f32 = 0.35;
    const FADE: f32 = 2.475;
    fn total(&self) -> f32 {
        self.grow + Self::HOLD + Self::FADE
    }
}

struct Glide {
    at: [f32; 2],
    target: [f32; 2],
    /// The direction this walker travels — the next hop's entry tangent.
    dir: [f32; 2],
    /// Walk-cycle clock, in seconds. It turns continuously — ants mill
    /// about even where they stand — and only an asleep ant's drawing
    /// ignores it.
    anim: f32,
    hop: Hop,
}

impl Glide {
    fn step(&mut self, to: [f32; 2], dt: f32) {
        if !closer(self.target, to) {
            // The exit direction reads off the old target — the cell
            // just left; a walker that has never left anywhere reads it
            // off the hop itself.
            let mut ex = [to[0] - self.target[0], to[1] - self.target[1]];
            if ex[0].abs() + ex[1].abs() < 1e-3 {
                ex = [to[0] - self.at[0], to[1] - self.at[1]];
            }
            let exit = if ex[0].abs() >= ex[1].abs() {
                [ex[0].signum(), 0.0]
            } else {
                [0.0, ex[1].signum()]
            };
            self.hop.to(self.at, to, self.dir, exit, TRAIL_HOP);
            self.dir = exit;
            self.target = to;
        }
        self.at = self.hop.tick(dt);
        self.anim += dt;
    }
}

/// One drifting leaf — the simulation state the Demo owns. Fluttering is
/// deliberately *not* `ParticleSystem::update`: that is a gravity
/// integrator, while a leaf sways, flips edge-on and rides the gust down
/// and clean out of the world — the wind keeps no litter pile. The
/// engine leaves `size`, `angle` and `color` to the caller for exactly
/// this — the systems only carry the finished particles into the sprite
/// batches.
struct Leaf {
    /// Which sprite sheet draws this leaf (index into `LEAVES`).
    sprite: usize,
    pos: [f32; 2],
    /// Descent speed in px/s, modulated by the flip.
    fall: f32,
    /// Sidesway amplitude in px/s.
    sway: f32,
    phase: f32,
    freq: f32,
    flip_phase: f32,
    flip_freq: f32,
    tilt: f32,
    /// Integrated rotation at a wandering speed: `spin_rate` relaxes
    /// toward `spin_target`, which is redrawn at random near every
    /// `spin_next` — so each leaf turns at speeds no clock repeats.
    spin: f32,
    spin_rate: f32,
    spin_target: f32,
    spin_next: f32,
    angle: f32,
    size: f32,
}

/// Everything a fresh draw needs to know about the current sky.
struct Sky {
    w: f32,
    h: f32,
    /// The canopy's parallax offset — spawn and wrap live in the
    /// leaves' own space, which keeps every re-entry at a visible edge.
    off: [f32; 2],
    /// The frost's odds of dealing a snowflake.
    snow: f32,
    /// The windward emission reach.
    reach: f32,
}

impl Leaf {
    /// A fresh leaf slightly above the visible window — which, in the
    /// parallax-shifted space the leaf lane lives in, is the window at
    /// `-off` — jittered inside its lane. `lane` of `lanes`.
    fn fresh(lane: usize, lanes: usize, sky: &Sky, rng: &mut Rng) -> Self {
        // The deal: with the frost's own odds, the wind turns a leaf
        // face down into a snowflake — the sheet changes, the
        // behaviour does not: a flake sways, flips and spins exactly
        // like any leaf.
        let sprite = if rng.next_f32() < sky.snow {
            SNOW_SHEET
        } else {
            (rng.next_f32() * SNOW_SHEET as f32) as usize % SNOW_SHEET.max(1)
        };
        // The emission width: the window, plus the windward reach a
        // gale needs upwind so its drift still paints the whole sky.
        let lane_w = (sky.w + sky.reach) / lanes.max(1) as f32;
        Self {
            sprite,
            pos: [
                -sky.w / 2.0 - sky.off[0] + lane_w * (lane as f32 + 0.5 + rng.in_range(-0.4, 0.4)),
                sky.h / 2.0 - sky.off[1] + rng.in_range(24.0, 140.0),
            ],
            fall: rng.in_range(38.0, 92.0),
            sway: rng.in_range(26.0, 85.0),
            phase: rng.in_range(0.0, TAU),
            freq: rng.in_range(1.3, 2.8),
            flip_phase: rng.in_range(0.0, TAU),
            flip_freq: rng.in_range(0.7, 1.9),
            tilt: rng.in_range(-0.25, 0.25),
            spin: rng.in_range(0.0, TAU),
            spin_rate: rng.in_range(-0.9, 0.9),
            spin_target: rng.in_range(-1.8, 1.8),
            spin_next: 0.0,
            angle: 0.0,
            size: rng.in_range(LEAF_SIZE.0, LEAF_SIZE.1),
        }
    }
}

struct Demo {
    levels: Vec<level::Level>,
    idx: usize,
    game: Option<game::Game>,
    player_at: [f32; 2],
    player_target: [f32; 2],
    player_hop: Hop,
    /// A 180° reversal taken in place: the direction turning into, or
    /// `None` when nobody is spinning.
    spin: Option<[f32; 2]>,
    /// The facing the player's sprite actually shows — it turns toward
    /// `face` instead of snapping, and lifts to the ceiling when she
    /// tucks into the queen's bed.
    player_face: [f32; 2],
    /// The same turning for every ant, keyed by id.
    render_face: HashMap<usize, [f32; 2]>,
    /// Zs adrift over the sleepers, emitted on the breath's peak.
    zs: Vec<Zzz>,
    /// The verdict, mid-splash, if the hill just won.
    splash: Option<Splash>,
    /// Whether this hill's verdict has already been read — undo can
    /// unwin a hill, and then it earns a fresh splash.
    splash_done: bool,
    /// The bloom each sleeper owes the coming crest: how many zs are
    /// left, the phase angle of the next, and the spacing between.
    zbloom: HashMap<usize, (u32, f32, f32)>,
    player_anim: f32,
    face: [f32; 2],
    glides: HashMap<usize, Glide>,
    /// The camera's centre, in world space: everything the board draws is
    /// offset by it, so it reads as the window watching the hill.
    cam: [f32; 2],
    /// The four baked walk-cycle frames, shared by every ant on the hill.
    ant_frames: Vec<frost::Shape>,
    /// The soil backdrop, stretched across the window below the board.
    soil: frost::Shape,
    /// The hill's solids draw as one tile map: this atlas — the soil
    /// texture itself, a single cell, the whole sheet — is shared by
    /// every wall and dirt placement, instanced in one draw call.
    tile_atlas: frost::Shape,
    /// The bed sprite, laid square on every bed's cell.
    bed: frost::Shape,
    /// The giant ground sprite, rebuilt per hill: its punched hole must
    /// match the hill's rectangle, and the span it is drawn across.
    ground: Option<(frost::Shape, [f32; 2])>,
    /// The engine's diagnostics overlay: window size with the vsync
    /// readout, FPS, frame time, processing times, draw calls, with
    /// Alt-0..Alt-V shortcuts.
    /// The profiler HUD, compiled in only with `--features diag`.
    #[cfg(feature = "diag")]
    diag: Diagnostics,
    font: Arc<[u8]>,
    note: String,
    /// Landing flares: ant id → flash left, 1 down to 0, for ants that
    /// were just tucked into their beds. A 0 is a warm forgotten memory;
    /// an undo-woken ant has its entry removed, so tucking it in again
    /// flares afresh.
    landing: HashMap<usize, f32>,
    /// The player's own tuck-in: `Some` from the moment she stands on
    /// the queen's bed, flashing down from 1.0; `None` while she walks
    /// and her lamp burns. Step off and return, and it flares afresh.
    player_home: Option<f32>,
    /// Per lamp — each ant, then the player keyed at `ants.len()`:
    /// the breath phase, the exertion it rides, the last position the
    /// breath tick saw, and whether the lamp was resting last frame
    /// (the transition is where sleep strikes up). The motion in
    /// between is the exertion's cue; `BREATH_*` turns exertion into
    /// pace and depth.
    breath: HashMap<usize, (f32, f32, [f32; 2], bool)>,
    note_t: f32,
    frost_t: f32,
    frost_bit: bool,
    /// The window/tab name is stamped once, on the first frame that has
    /// a window to name.
    title_set: bool,
    was: HashMap<KeyCode, bool>,
    was_mouse: HashMap<MouseButton, bool>,
    /// Click-walking: the directions left to walk toward the last
    /// clicked cell, one issued per completed glide.
    route: Vec<game::Dir>,
    leaf_rng: Rng,
    leaves: Vec<Leaf>,
    leaves_seeded: bool,
    /// How many of the deck are currently in the air — thirty in calm
    /// weather, swelling toward `N_MAX` as the frost deepens.
    leaves_active: usize,
    leaf_t: f32,
}

impl Demo {
    fn new(ant_frames: Vec<frost::Shape>, soil: frost::Shape) -> Self {
        // The soil texture doubles as the hill's tile atlas: one cell,
        // the whole sheet, every placement sampling all of it. A
        // photographic atlas wants bilinear, not the map's pixel-art
        // default. A soil PNG that decodes for one must decode for the
        // other — anything else is a broken build.
        let mut tile_atlas =
            frost::Shape::tilemap_bytes(SOIL).expect("shipped soil bakes as an atlas");
        tile_atlas.set_filter(SpriteFilter::Linear);
        let bed = frost::Shape::sprite_bytes(BED).expect("shipped bed sprite decodes");
        let levels = level::load_levels(LEVELS);
        if levels.is_empty() {
            log::error!("antifrost: no levels loaded");
        }
        let mut demo = Self {
            levels,
            idx: 0,
            game: None,
            player_at: [0.0, 0.0],
            player_target: [0.0, 0.0],
            player_hop: Hop::flat([0.0, 0.0], HOP),
            spin: None,
            player_face: [1.0, 0.0],
            render_face: HashMap::new(),
            zs: Vec::new(),
            splash: None,
            splash_done: false,
            zbloom: HashMap::new(),
            player_anim: 0.0,
            face: [1.0, 0.0],
            glides: HashMap::new(),
            cam: [0.0, 0.0],
            ant_frames,
            soil,
            tile_atlas,
            bed,
            ground: None,
            #[cfg(feature = "diag")]
            diag: Diagnostics::from_bytes(DIAG_FONT, DiagnosticsFlags::all()).unwrap_or_else(
                |err| {
                    log::error!("antifrost: bad diagnostics font: {err}");
                    std::process::exit(1);
                },
            ),
            font: Arc::from(GAME_FONT),
            note: String::new(),
            landing: HashMap::new(),
            player_home: None,
            breath: HashMap::new(),
            note_t: 0.0,
            frost_t: 0.0,
            frost_bit: false,
            title_set: false,
            was: HashMap::new(),
            was_mouse: HashMap::new(),
            route: Vec::new(),
            leaf_rng: Rng::default(),
            leaves: Vec::new(),
            leaves_seeded: false,
            leaves_active: N_LEAVES,
            leaf_t: 0.0,
        };
        demo.load(0);
        demo
    }

    fn load(&mut self, idx: usize) {
        if self.levels.is_empty() {
            return;
        }
        self.idx = idx % self.levels.len();
        let level = self.levels[self.idx].clone();
        let start = self.cell_px(level.player_start);
        // The surrounding ground is baked per hill: its punched hole
        // must match the hill's own rectangle.
        self.ground = Some(ground_sprite(level.cols, level.rows).unwrap_or_else(|err| {
            log::error!("antifrost: bad ground bake: {err}");
            std::process::exit(1);
        }));
        self.game = Some(game::Game::start(level));
        self.player_at = start;
        self.player_target = start;
        self.player_hop = Hop::flat(start, HOP);
        self.spin = None;
        self.player_face = [1.0, 0.0];
        self.render_face.clear();
        self.zs.clear();
        self.zbloom.clear();
        self.splash = None;
        self.splash_done = false;
        self.glides.clear();
        self.cam = start;
        self.route.clear();
        self.note.clear();
        self.landing.clear();
        self.breath.clear();
        self.player_home = None;
        self.note_t = 0.0;
        self.frost_t = 0.0;
        self.frost_bit = false;
    }

    fn cell_px(&self, c: game::Cell) -> [f32; 2] {
        let (cols, rows) = (
            self.levels[self.idx].cols as f32,
            self.levels[self.idx].rows as f32,
        );
        [
            (c.col as f32 - cols / 2.0 + 0.5) * TILE,
            (rows / 2.0 - c.row as f32 - 0.5) * TILE,
        ]
    }

    /// Rising-edge key test (the repo idiom: remember last frame's state).
    fn pressed(&mut self, ctx: &Context, key: KeyCode) -> bool {
        let down = ctx.key_down(key);
        let edge = down && !*self.was.entry(key).or_insert(false);
        self.was.insert(key, down);
        edge
    }

    /// The same rising-edge test for the mouse buttons.
    fn pressed_mouse(&mut self, ctx: &Context, button: MouseButton) -> bool {
        let down = ctx.mouse_button_down(button);
        let edge = down && !*self.was_mouse.entry(button).or_insert(false);
        self.was_mouse.insert(button, down);
        edge
    }

    /// Screen → cell: the exact inverse of drawing a cell's centre —
    /// undo the camera and zoom, then undo `cell_px`'s centring, rounding
    /// to whichever cell the pointer covers.
    fn cell_at_screen(&self, at: [f32; 2]) -> game::Cell {
        let (cols, rows) = (
            self.levels[self.idx].cols as f32,
            self.levels[self.idx].rows as f32,
        );
        let wx = at[0] / ZOOM + self.cam[0];
        let wy = at[1] / ZOOM + self.cam[1];
        game::Cell::new(
            (wx / TILE + cols / 2.0 - 0.5).round() as i32,
            (rows / 2.0 - 0.5 - wy / TILE).round() as i32,
        )
    }

    /// A cell a click route may cross: open tunnel or bed, no sleeper in
    /// the way, not the player's own packed line. Wild and placed ants
    /// pass — walking over picks them up, which is half the point. Dirt
    /// and rock stay out: a click never spends a dig behind the player's
    /// back.
    fn routable(&self, c: game::Cell) -> bool {
        let Some(game) = &self.game else {
            return false;
        };
        if !game.in_bounds(c) {
            return false;
        }
        if !matches!(game.tile(c), level::Tile::Tunnel | level::Tile::Bed(_)) {
            return false;
        }
        if matches!(game.occupant(c), Some(game::Occ::Asleep(_))) {
            return false;
        }
        !(0..game.trail.len()).any(|i| game.trail_target(i) == c)
    }

    /// The click route: breadth-first over routable cells from wherever
    /// the player stands to `goal`, on this frame's snapshot. Empty when
    /// the goal is the here, `None` when the hill will not yield. The
    /// steps are re-run against the live game and the route dies at the
    /// first refusal, so a snapshot gone stale mid-walk is a stutter,
    /// never a rule broken.
    fn find_route(&self, goal: game::Cell) -> Option<Vec<game::Dir>> {
        let game = self.game.as_ref()?;
        let start = game.player;
        if start == goal {
            return Some(Vec::new());
        }
        if !self.routable(goal) {
            return None;
        }
        let mut came: HashMap<game::Cell, (game::Cell, game::Dir)> = HashMap::new();
        came.insert(start, (start, game::Dir::Right));
        let mut queue = VecDeque::new();
        queue.push_back(start);
        while let Some(c) = queue.pop_front() {
            if c == goal {
                break;
            }
            for dir in [
                game::Dir::Left,
                game::Dir::Right,
                game::Dir::Up,
                game::Dir::Down,
            ] {
                let n = c.at(dir);
                if !came.contains_key(&n) && self.routable(n) {
                    came.insert(n, (c, dir));
                    queue.push_back(n);
                }
            }
        }
        let mut path = Vec::new();
        let mut c = goal;
        while c != start {
            let (p, d) = *came.get(&c)?;
            path.push(d);
            c = p;
        }
        path.reverse();
        Some(path)
    }

    /// World → screen: everything drawn in the camera's moving space —
    /// the board, the ants, the chain — passes through here: the hill is
    /// seen through the camera at `ZOOM` magnification. The frost rim,
    /// the leaves, and the HUD stay in the window's own space.
    fn screen(&self, p: [f32; 2]) -> [f32; 2] {
        [(p[0] - self.cam[0]) * ZOOM, (p[1] - self.cam[1]) * ZOOM]
    }

    /// The follow: while the queen walks inside the dead zone the world
    /// holds still; once she steps out, the target is the nearest edge
    /// of the zone — the camera shifts just enough to bring her back
    /// onto it, never straight to her — and eases toward that target
    /// exponentially, closing a constant fraction of the remaining
    /// distance per second. Frame-rate exact, since `ease` integrates
    /// the decay over `dt` rather than assuming a tick.
    fn update_camera(&mut self, ctx: &Context, dt: f32) {
        let (w, h) = ctx.size();
        let ease = 1.0 - (-(dt / CAM_EASE_TAU)).exp();
        // The dead zone is measured in screen pixels, so its world half-
        // extent shrinks as the view magnifies: at 4× a quarter-window
        // zone is a quarter of the cells it covered at 1×.
        let [dzx, dzy] = [w * 0.5 * CAM_DEAD / ZOOM, h * 0.5 * CAM_DEAD / ZOOM];
        for (axis, half) in [(0usize, dzx), (1usize, dzy)] {
            let (p, c) = (self.player_at[axis], self.cam[axis]);
            let d = p - c;
            let to = if d.abs() <= half {
                c
            } else {
                p - d.signum() * half
            };
            self.cam[axis] = c + (to - c) * ease;
        }
    }

    fn refresh(&mut self, dt: f32) {
        let Some(game) = &self.game else { return };
        // Player glide.
        let want = self.cell_px(game.player);
        if !closer(self.player_target, want) {
            // Facing rides the step that actually happened — keys, click
            // route, or undo all turn the same way, and a move that the
            // hill refused turns nothing. And corners bend: leaving the
            // corner cell along the *old* facing and arriving along the
            // new, the hop curves a quarter circle, so the turn reads
            // through the path instead of snapping at the cell.
            let step = [
                want[0] - self.player_target[0],
                want[1] - self.player_target[1],
            ];
            let exit = if step[0].abs() >= step[1].abs() {
                [step[0].signum(), 0.0]
            } else {
                [0.0, step[1].signum()]
            };
            self.player_hop
                .to(self.player_at, want, self.face, exit, HOP);
            self.face = exit;
            self.player_target = want;
        }
        self.player_at = self.player_hop.tick(dt);
        self.player_anim += dt;

        // Where every ant wants to be; trail ants read the packed history.
        let mut wanted: HashMap<usize, game::Cell> = HashMap::new();
        for i in 0..game.trail.len() {
            wanted.insert(game.trail[i], game.trail_target(i));
        }
        for (id, ant) in game.ants.iter().enumerate() {
            if let game::AntState::Wild(c) | game::AntState::Placed(c) | game::AntState::Asleep(c) =
                ant.state
            {
                wanted.insert(id, c);
            }
        }
        // Ants sharing a cell fan out a few pixels so all stay visible.
        let mut seen: HashMap<game::Cell, usize> = HashMap::new();
        for id in 0..game.ants.len() {
            let Some(cell) = wanted.get(&id).copied() else {
                continue;
            };
            let k = seen.entry(cell).or_insert(0);
            let offset = (*k as f32) * TILE * 0.14;
            *k += 1;
            let base = self.cell_px(cell);
            let to = [base[0] - offset, base[1] + offset * 0.7];
            self.glides
                .entry(id)
                .or_insert_with(|| Glide {
                    at: to,
                    target: to,
                    dir: [0.0, 0.0],
                    anim: 0.0,
                    hop: Hop::flat(to, TRAIL_HOP),
                })
                .step(to, dt);
        }
        if let Some(note) = self.game.as_mut().and_then(|g| g.take_note()) {
            self.note = note.to_string();
            self.note_t = 2.4;
        }

        // The frost: over par and winter arrives — snow mixed into
        // the leaves, a wind off the right, both climbing with the
        // round count. Past double par the weather caps and simply
        // holds. The gauge eases toward its mark, so an undo walks
        // the snow back into the leaves.
        let over = self
            .game
            .as_ref()
            .is_some_and(|g| g.level.par > 0 && g.moves > g.level.par);
        if over && !self.frost_bit {
            self.frost_bit = true;
            self.note = "the frost bites  keep walking".to_string();
            self.note_t = 2.4;
        }
        let target = match self.game.as_ref() {
            Some(g) if g.level.par > 0 => {
                ((g.moves as f32 - g.level.par as f32) / g.level.par as f32).clamp(0.0, 1.0)
            }
            _ => 0.0,
        };
        self.frost_t += (target - self.frost_t) * (1.0 - (-(dt / 1.2)).exp());
    }

    /// Autumn over the hill: thirty leaves flutter down and drift clean
    /// out past the bottom edge, re-entering above the window — the
    /// wind keeps no litter. One step per leaf, then the eleven sprite
    /// batches are rebuilt whole — with thirty particles, incremental
    /// bookkeeping would only be a place to hide bugs.
    fn update_leaves(&mut self, ctx: &mut Context, dt: f32) {
        let (w, h) = ctx.size();
        // The canopy's parallax: the batches ride a node offset, so the
        // leaves' own space shows the window shifted by `-off` — spawn,
        // wrap, and ground all live in that space, which keeps every
        // re-entry at a visible edge.
        let off = [
            -self.cam[0] * ZOOM * LEAF_PARALLAX,
            -self.cam[1] * ZOOM * LEAF_PARALLAX,
        ];
        self.leaf_t += dt;
        // The weather: past par the gauge climbs, the wind leans out
        // of the right, and the snow creeps into the deck — all three
        // capped at double par, where the storm holds its note.
        let wind_mag = self.frost_t * WIND_MAX;
        // The emission width answers the wind: upwind reach equal to
        // the drift a flake performs across its whole fall, so the
        // gale paints an even sky instead of a bare right edge.
        let reach = wind_mag * h / WIND_MAX;
        // A slow shared gust all the leaves hang from — and under it,
        // once it comes, the gale out of the right.
        let wind =
            16.0 * (0.11 * self.leaf_t).sin() + 7.0 * (0.37 * self.leaf_t + 2.1).sin() - wind_mag;
        let sky = Sky {
            w,
            h,
            off,
            snow: self.frost_t,
            reach,
        };
        if !self.leaves_seeded {
            self.leaves_seeded = true;
            // The deck deals all eighty, but the calm-weather ledger
            // draws only the first thirty — so seed on the *drawn* lane
            // grid, not the deck's size. Seeding `lane of N_MAX` put the
            // visible thirty at the sky's left 3/8; the modulo fills
            // every drawn lane three times over, so the open frame is
            // wide even before the storm calls in the rest (who are
            // re-dealt on activation anyway).
            self.leaves = (0..N_MAX)
                .map(|lane| Leaf::fresh(lane % N_LEAVES, N_LEAVES, &sky, &mut self.leaf_rng))
                .collect();
            // Scatter the first volley through the air, so the hill
            // opens already in autumn rather than waiting for a fall.
            for leaf in &mut self.leaves {
                leaf.pos[1] = self
                    .leaf_rng
                    .in_range(-h / 2.0 - off[1] + 20.0, h / 2.0 - off[1] + 60.0);
            }
        }
        let mut batches = vec![ParticleSystem::default(); LEAVES.len()];
        // The storm's ledger: thirty in calm weather, swelling toward
        // eighty as the frost deepens — newcomers are called in from
        // above, dealt like everything else.
        let active = (N_LEAVES as f32 + (N_MAX - N_LEAVES) as f32 * self.frost_t) as usize;
        if active > self.leaves_active {
            for i in self.leaves_active..active {
                self.leaves[i] = Leaf::fresh(i, active, &sky, &mut self.leaf_rng);
            }
        }
        self.leaves_active = active;
        for (i, leaf) in self.leaves.iter_mut().enumerate().take(active) {
            leaf.phase += leaf.freq * dt;
            leaf.flip_phase += leaf.flip_freq * dt;
            let flip = leaf.flip_phase.sin();
            // A wandering spin: the rate chases a target that is
            // redrawn at random moments — speeding up, easing off,
            // sometimes reversing — never a metronome.
            if self.leaf_t >= leaf.spin_next {
                leaf.spin_target = self.leaf_rng.in_range(-1.8, 1.8);
                leaf.spin_next = self.leaf_t + self.leaf_rng.in_range(0.6, 2.4);
            }
            leaf.spin_rate += (leaf.spin_target - leaf.spin_rate) * (dt * 2.2).min(1.0);
            leaf.spin += leaf.spin_rate * dt;
            // Edge-on, a leaf falls faster; the scale barely notices
            // (a 6 % whisper), and the size "zoom" the flip used to
            // pump is gone entirely.
            let vx = leaf.sway * leaf.phase.cos() + wind;
            let vy = -leaf.fall * (0.62 + 0.38 * flip.abs());
            leaf.pos[0] += vx * dt;
            leaf.pos[1] += vy * dt;
            leaf.angle = leaf.tilt + 0.45 * leaf.phase.cos() + leaf.spin;
            // The gust may carry a leaf across the whole width. The
            // margin clears the fattest leaf's half-width so nothing
            // pops at an edge.
            let m = 100.0;
            if leaf.pos[0] > w / 2.0 - off[0] + m + reach {
                leaf.pos[0] -= w + 2.0 * m + reach;
            } else if leaf.pos[0] < -w / 2.0 - off[0] - m {
                // Taken by the gale: the flake walks back onto the
                // windward edge, anywhere along the reach and at its
                // own height — drift and feed stay even.
                leaf.pos[0] = w / 2.0 - off[0] + m + self.leaf_rng.in_range(0.0, reach + m);
            }
            // Past the bottom and the leaf is simply gone — recycled to
            // its lane above the window. No pile, no fade.
            if leaf.pos[1] < -h / 2.0 - off[1] - m {
                *leaf = Leaf::fresh(i, active, &sky, &mut self.leaf_rng);
            }
            batches[leaf.sprite].spawn(Particle {
                pos: leaf.pos,
                vel: [vx, vy],
                // Nothing here dies of old age; nothing here fades.
                life: 1.0,
                max_life: 1.0,
                size: leaf.size * (0.94 + 0.06 * flip.abs()),
                angle: leaf.angle,
                color: Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
            });
        }
        for (node, batch) in ctx.scene().root.children.iter_mut().zip(batches) {
            node.transform = Transform::translate(off);
            if let Some(frost::Shape::Particles { system, .. }) = node.shape.as_mut() {
                *system = batch;
            }
        }
    }

    /// Advance one lamp's breath: the phase turns at the lamp's rate,
    /// and the exertion chases the speed the lamp is actually
    /// travelling at — brisk on the way up, gradual on the way down,
    /// so stopping slows the pace by degrees rather than by switch.
    /// Phases start scattered on the golden angle, so a dorm of
    /// sleepers never breathes in chorus. A resting lamp also blooms
    /// its zs: a burst of two to five, spread over the eighth of a
    /// breath that leads to the crest, the last z landing exactly on
    /// the peak of the breath.
    fn breathe(&mut self, id: usize, at: [f32; 2], resting: bool, dt: f32) {
        let e = self
            .breath
            .entry(id)
            .or_insert((id as f32 * 2.399_963_3, 0.0, at, resting));
        if resting && !e.3 {
            // Sleep does not strike up mid-breath. It starts three
            // quarters into one, so the first bloom is soon behind
            // the sleeper — soon, not instant, the way a first yawn
            // still has to gather.
            e.0 = 3.0 * std::f32::consts::FRAC_PI_2;
        }
        e.3 = resting;
        let dx = at[0] - e.2[0];
        let dy = at[1] - e.2[1];
        e.2 = at;
        let speed = (dx * dx + dy * dy).sqrt() / dt.max(1e-3);
        let target = (speed / BREATH_FULL_SPEED).min(1.0);
        let tau = if target > e.1 {
            BREATH_UP_TAU
        } else {
            BREATH_DOWN_TAU
        };
        e.1 += (target - e.1) * (1.0 - (-(dt / tau)).exp());
        let rate = if resting {
            BREATH_SLEEP_RATE
        } else {
            BREATH_CALM_RATE + (BREATH_WORK_RATE - BREATH_CALM_RATE) * e.1
        };
        let prev = e.0;
        e.0 = (e.0 + dt * rate * std::f32::consts::TAU) % std::f32::consts::TAU;
        if !resting {
            self.zbloom.remove(&id);
            return;
        }
        if e.0 < prev {
            // A breath has just begun: draw the next bloom's lot — how
            // many zs, and how wide a window. The window is an eighth
            // of the breath, ± a quarter of itself redrawn every
            // cycle, always ending its last z on the crest.
            let n = (self.leaf_rng.in_range(2.0, 5.0).round() as u32).clamp(2, 5);
            let w = std::f32::consts::FRAC_PI_4 * self.leaf_rng.in_range(0.75, 1.25);
            self.zbloom
                .insert(id, (n, std::f32::consts::FRAC_PI_2 - w, w / (n - 1) as f32));
        }
        // Angles run from an eighth-turn out to the quarter-turn that
        // crowns the crest — an eighth of the period, to the last z.
        let mut fire = 0u32;
        if let Some(b) = self.zbloom.get_mut(&id) {
            while b.0 > 0 && prev < b.1 && b.1 <= e.0 {
                b.0 -= 1;
                b.1 += b.2;
                fire += 1;
            }
        }
        for _ in 0..fire {
            self.spawn_z(at);
        }
    }

    /// One z off a sleeper's breath, into the air above its head:
    /// tilted a random way and never quite the same twice.
    fn spawn_z(&mut self, at: [f32; 2]) {
        let jx = self.leaf_rng.in_range(-6.0, 6.0);
        // Three fifths of the old drift and rise — a sleepy exhale —
        // and a life stretched by the same factor, so each z still
        // climbs as high as it ever did, only slower.
        let vx = self.leaf_rng.in_range(-18.0, 18.0) * 0.6;
        let vy = self.leaf_rng.in_range(30.0, 48.0) * 0.6;
        let ttl = self.leaf_rng.in_range(1.6, 2.3) / 0.6;
        let seed = self.leaf_rng.in_range(0.0, std::f32::consts::TAU);
        self.zs.push(Zzz {
            at: [at[0] + jx, at[1] + TILE * 0.34],
            vel: [vx, vy],
            age: 0.0,
            ttl,
            seed,
        });
    }

    // ------------------------------------------------------------- draw

    /// Places one baked ant frame as a scene node: centered on `at`,
    /// turned so the head points along `face`, mirrored for a left walk,
    /// tinted by `tint`, drawn at `order` — the same z ladder the immediate
    /// draws use (trail ants ≈ 1.0, the player's silhouette 1.48, the
    /// player 1.5). Cloning the shape just bumps an `Arc`; only the node's
    /// numbers change from frame to frame.
    fn put_ant(
        &self,
        node: &mut SceneNode,
        frame: usize,
        at: [f32; 2],
        face: [f32; 2],
        tint: Color,
        order: f32,
    ) {
        let shape = &self.ant_frames[frame];
        node.shape = Some(shape.clone());
        // The frames face right. Walking left mirrors the sheet; walking
        // up or down turns it a quarter, and a mirrored sheet turns the
        // other way so its head still points where the ant walks.
        let (sx, ang) = if face[0] < 0.0 {
            (-1.0, (-face[1]).atan2(-face[0]))
        } else {
            (1.0, face[1].atan2(face[0].max(0.0)))
        };
        let s = ANT_W * ZOOM / shape.sprite_size().map_or(1.0, |sz| sz[0]).max(1.0);
        node.transform = Transform::rotate(ang).compose(&Transform::translate(at));
        node.scale = [sx * s, s];
        node.modulate = tint;
        // An ant is not only lit by the night — it carries its own
        // ember, so the lamp it casts always has a face to stand on
        // even at the very floor of the ambient.
        node.lit = true;
        node.glow = shade(tint, 0.45);
        node.order = order;
    }

    fn draw_ants(&mut self, ctx: &mut Context, dt: f32) {
        let Some(game) = &self.game else { return };
        // Chain link line: player → head → … → tail.
        if !game.trail.is_empty() {
            let mut pts: Vec<[f32; 2]> = vec![self.screen(self.player_at)];
            for i in 0..game.trail.len() {
                if let Some(g) = self.glides.get(&game.trail[i]) {
                    pts.push(self.screen(g.at));
                }
            }
            for w in pts.windows(2) {
                ctx.line(
                    w[0][0],
                    w[0][1],
                    w[1][0],
                    w[1][1],
                    Color {
                        r: 0.55,
                        g: 0.44,
                        b: 0.32,
                        a: 0.65,
                    },
                    2.0,
                    0.6,
                );
            }
        }
        // Facing for trail ants: toward whoever they follow.
        let mut facing: HashMap<usize, [f32; 2]> = HashMap::new();
        for i in 0..game.trail.len() {
            let id = game.trail[i];
            let ahead = if i == 0 {
                self.player_at
            } else {
                self.glides
                    .get(&game.trail[i - 1])
                    .map(|g| g.at)
                    .unwrap_or([0.0, 0.0])
            };
            let here = self.glides.get(&id).map(|g| g.at).unwrap_or([0.0, 0.0]);
            let (dx, dy) = (ahead[0] - here[0], ahead[1] - here[1]);
            let f = if dx.abs() >= dy.abs() {
                [dx.signum(), 0.0]
            } else {
                [0.0, dy.signum()]
            };
            facing.insert(id, if f == [0.0, 0.0] { [-1.0, 0.0] } else { f });
        }
        // One scene node per ant (plus a silhouette slot under the player),
        // hung on the root after the leaf batches. Frame index is the
        // walk-cycle clock: every awake ant cycles, standing still or
        // striding; an asleep ant always draws frame one, dimmed.
        let frame_of = |anim: f32, asleep: bool| {
            if asleep {
                0
            } else {
                ((anim * ANT_WALK_FPS) as usize) % ANT_FRAMES.len()
            }
        };
        // The hill's solid cells are soil too: the same sprite the
        // ground wears, shrunk from a thousand pixels to a tile's two
        // hundred, so the grain the background can only whisper, the
        // walls shout — one material at two distances is the whole
        // depth trick. Sprites only ride scene nodes, so the hill's
        // rock and dirt are rebuilt each frame from a pool sized to
        // the count the level holds — and as nodes they join the light
        // field: they receive the ants' lamps, and they occlude them,
        // so a wall between an ant and a pixel puts that pixel in
        // shadow. Dirt is the same sprite, tinted toward the old C_DIRT
        // brown: loose earth, kin to the wall.
        let tiles: Vec<(i32, i32, bool)> = (0..game.level.rows as i32)
            .flat_map(|row| (0..game.level.cols as i32).map(move |col| (row, col)))
            .filter_map(|(row, col)| {
                let rock = matches!(game.tile(game::Cell::new(col, row)), level::Tile::Rock);
                let dirt = matches!(game.tile(game::Cell::new(col, row)), level::Tile::Dirt);
                (rock || dirt).then_some((row, col, rock))
            })
            .collect();
        // Slots: leaves, ants, the player pair, one lamp per ant plus
        // the player's, the single tile map holding every solid, a
        // beacon and a sprite per bed (two apiece), and the two
        // backdrop sprites.
        let slots = LEAVES.len() + game.ants.len() * 2 + 6 + game.level.beds.len() * 2;
        {
            let root = &mut ctx.scene().root;
            while root.children.len() < slots {
                root.children.push(Box::new(SceneNode::default()));
            }
            root.children.truncate(slots);
        }
        // Every ant is a lamp, and every lamp breathes: at rest a
        // gentle sway, working a deeper faster pulse, and stopping
        // winds the pace down by degrees. Asleep keeps only a faint
        // slow pulse on the dark — the dorm breathes, it does not go
        // out — and the tuck-in spike burns loud over it. The player
        // is an ant too, and the queen's bed is her target: same
        // breath, same flare.
        {
            let reach = TILE * ZOOM * LAMP_REACH;
            let base = LEAVES.len() + game.ants.len() + 2;
            for id in 0..game.ants.len() {
                let node = &mut ctx.scene().root.children[base + id];
                let Some(g) = self.glides.get(&id) else {
                    node.shape = None;
                    continue;
                };
                let asleep = matches!(game.ants[id].state, game::AntState::Asleep(_));
                let flare = self.landing.get(&id).copied().unwrap_or(0.0);
                let (phase, exertion, ..) = self
                    .breath
                    .get(&id)
                    .copied()
                    .unwrap_or((0.0, 0.0, [0.0; 2], false));
                let intensity = if asleep {
                    BREATH_SLEEP_BASE + BREATH_SLEEP_AMP * phase.sin() + flare * 3.0
                } else {
                    BREATH_LAMP_BASE
                        + (BREATH_CALM_AMP + (BREATH_WORK_AMP - BREATH_CALM_AMP) * exertion)
                            * phase.sin()
                };
                if intensity <= 0.0 {
                    node.shape = None;
                    continue;
                }
                let c = ant_color(game.ants[id].color);
                // A little white in the core, the ant's own hue in the
                // throw of it.
                let col = Color {
                    r: c.r + (1.0 - c.r) * 0.35,
                    g: c.g + (1.0 - c.g) * 0.35,
                    b: c.b + (1.0 - c.b) * 0.35,
                    a: 1.0,
                };
                node.shape = Some(frost::Shape::Light {
                    light: frost::Light::point(col, intensity, reach).with_penumbra(LAMP_PENUMBRA),
                });
                node.transform = Transform::translate(self.screen(g.at));
            }
            // The player breathes with the rest: a working pulse on
            // the walk, the queen's bed a loud spike, and then the
            // faint slow sway of the tucked-in.
            let node = &mut ctx.scene().root.children[base + game.ants.len()];
            let (phase, exertion, ..) = self
                .breath
                .get(&game.ants.len())
                .copied()
                .unwrap_or((0.0, 0.0, [0.0; 2], false));
            let intensity = match self.player_home {
                Some(t) => BREATH_SLEEP_BASE + BREATH_SLEEP_AMP * phase.sin() + t * 4.5,
                None => {
                    BREATH_LAMP_BASE
                        + (BREATH_CALM_AMP + (BREATH_WORK_AMP - BREATH_CALM_AMP) * exertion)
                            * phase.sin()
                }
            };
            if intensity > 0.0 {
                node.shape = Some(frost::Shape::Light {
                    light: frost::Light::point(
                        Color {
                            r: 1.0,
                            g: 0.86,
                            b: 0.66,
                            a: 1.0,
                        },
                        intensity,
                        reach,
                    )
                    .with_penumbra(LAMP_PENUMBRA),
                });
            } else {
                node.shape = None;
            }
            node.transform = Transform::translate(self.screen(self.player_at));
        }
        // The map as a window, in two sprites. Behind: the far
        // background — the map's own rectangle, plus the one slack its
        // own drift demands — drifting at `DEEP_PARALLAX` of the
        // camera. The drift is camera-absolute and the camera chases
        // the player anywhere on the hill, so the hole can sit off the
        // sprite's centre by `(1 − DEEP_PARALLAX)` of the camera's
        // reach — up to that fraction of half the map. Size for the
        // worst case and the slack lies under the ground sheet,
        // forever invisible; undersize it and the hole outruns the
        // sprite into a band of `C_BG`, which is what launched.
        // In front: the ground sheet — flat, in the window's original
        // `Background` colour — welded to the board's speed at 1.0,
        // whose punched hole hides every far edge the hole does not
        // contain.
        {
            let [tw, th] = self.soil.sprite_size().unwrap_or([1.0, 1.0]);
            let cw = game.level.cols as f32 * TILE * ZOOM;
            let ch = game.level.rows as f32 * TILE * ZOOM;
            let slack = |m: f32| m * (1.0 - DEEP_PARALLAX) * 0.5 * 1.05;
            let node = &mut ctx.scene().root.children[slots - 1];
            node.shape = Some(self.soil.clone());
            node.transform = Transform::translate([
                -self.cam[0] * ZOOM * DEEP_PARALLAX,
                -self.cam[1] * ZOOM * DEEP_PARALLAX,
            ]);
            node.scale = [(cw + 2.0 * slack(cw)) / tw, (ch + 2.0 * slack(ch)) / th];
            node.modulate = Color {
                r: 0.45,
                g: 0.4,
                b: 0.38,
                a: 1.0,
            };
            node.lit = true;
            node.order = -0.5;
        }
        if let Some((shape, span)) = &self.ground {
            let [gw, gh] = shape.sprite_size().unwrap_or([1.0, 1.0]);
            let node = &mut ctx.scene().root.children[slots - 2];
            node.shape = Some(shape.clone());
            node.transform = Transform::translate(self.screen([0.0, 0.0]));
            node.scale = [span[0] / gw, span[1] / gh];
            node.modulate = Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            };
            // Lit but not occluding: a lamp near the edge throws its
            // glow onto the surrounding ground through the border
            // hairlines, and the frame's own shadows do the rest.
            node.lit = true;
            node.order = -0.4;
        }
        // Painted occlusion is a rectangle's privilege — a sprite's
        // body casts no shadow on the flag alone — so the hill's solid
        // cells declare their boxes here. But every occluder the field
        // carries is tested by every fragment any light reaches, so
        // one box per tile at 4K is a frame budget in chains: instead,
        // touching solids merge into per-row runs. A merged run casts
        // the same shadow its tiles ever did — the tile hairlines that
        // used to leak a thread of light measure a pixel, and a merged
        // edge is invisible against the row it belongs to.
        {
            let ts = TILE * ZOOM;
            let mut runs: Vec<(i32, i32, i32)> = Vec::new();
            let mut i = 0;
            while i < tiles.len() {
                let (row, col_s, _) = tiles[i];
                let mut col_e = col_s;
                let mut j = i + 1;
                while j < tiles.len() && tiles[j].0 == row && tiles[j].1 == col_e + 1 {
                    col_e = tiles[j].1;
                    j += 1;
                }
                runs.push((col_s, col_e, row));
                i = j;
            }
            // Sorted by span, then row, so a stack of equal spans — the
            // frame's sides, a pile's flank — sits adjacent in line for
            // the upward merge. A row-major scan could never see it: a
            // row with two runs puts the other column's run between the
            // stack's own links, and the stack stayed one box per row,
            // leaking a hairline of light through every seam it drew.
            runs.sort_unstable();
            let mut k = 0;
            while k < runs.len() {
                let (col_s, col_e, row0) = runs[k];
                let mut row1 = row0;
                let mut m = k + 1;
                while m < runs.len()
                    && runs[m].0 == col_s
                    && runs[m].1 == col_e
                    && runs[m].2 == row1 + 1
                {
                    row1 = runs[m].2;
                    m += 1;
                }
                let [x0, y0] = self.screen(self.cell_px(game::Cell::new(col_s, row0)));
                let [x1, y1] = self.screen(self.cell_px(game::Cell::new(col_e, row1)));
                // Full cells, not drawn tiles: the hairline inset is a
                // detail of the grain, while the hill's body is solid
                // to the grid — so the shadow must be too.
                ctx.occluder(
                    Transform::translate([(x0 + x1) * 0.5, (y0 + y1) * 0.5]),
                    [0.0, 0.0],
                    [((x1 - x0) + ts) * 0.5, ((y0 - y1) + ts) * 0.5],
                );
                k = m;
            }
        }
        {
            // The hill's solids draw as one tile map: every wall and
            // dirt cell is a placement sharing the soil atlas, drawn
            // instanced in a single call — the engine's answer to the
            // per-cell node pool this replaces. Placements ride the
            // node's space, which is the screen's here; tint per tile
            // — rock bare soil, dirt the old C_DIRT, both keeping the
            // standing jitter — and the whole map lights as one
            // surface. The shadows are the merged boxes declared
            // above: batched art and shadow arithmetic are separate
            // problems, and only one of them is a draw call.
            let mut map = self.tile_atlas.clone();
            let inset = TILE * ZOOM - 1.5;
            let wall = Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            };
            let dirt_tint = Color {
                r: C_DIRT.r * 2.2,
                g: C_DIRT.g * 2.2,
                b: C_DIRT.b * 2.2,
                a: 1.0,
            };
            map.set_tiles(tiles.iter().map(|(row, col, rock)| {
                let [x, y] = self.screen(self.cell_px(game::Cell::new(*col, *row)));
                let noise = 1.0 + ((col * 31 + row * 17) % 3) as f32 * 0.045;
                let mut tile = Tile::new([x, y], [inset, inset], [0.0, 0.0, 1.0, 1.0]);
                tile.color = shade(if *rock { wall } else { dirt_tint }, noise);
                tile
            }));
            let node = &mut ctx.scene().root.children[LEAVES.len() + game.ants.len() * 2 + 3];
            node.shape = Some(map);
            node.transform = Transform::translate([0.0, 0.0]);
            node.scale = [1.0, 1.0];
            node.modulate = wall;
            node.lit = true;
            node.order = 0.0;
        }
        // The beds are the hill's beacons: each carries a faint light
        // of its own colour — enough to find across a dark room and to
        // aim a push by, never enough to light one. They burn always:
        // a taken bed keeps glowing, the target outlives the ant.
        {
            let base = LEAVES.len() + game.ants.len() * 2 + 4;
            for (i, (cell, color)) in game.level.beds.iter().enumerate() {
                let node = &mut ctx.scene().root.children[base + i];
                node.shape = Some(frost::Shape::Light {
                    light: frost::Light::point(ant_color(*color), 0.5, TILE * ZOOM * 1.3)
                        .with_penumbra(LAMP_PENUMBRA),
                });
                node.transform = Transform::translate(self.screen(self.cell_px(*cell)));
            }
        }
        // And the beds receive as well as give: the user's own bed
        // sprite, square on the cell, lit by the beacon it sleeps
        // under and every lamp that reaches it.
        {
            let base = LEAVES.len() + game.ants.len() * 2 + 4 + game.level.beds.len();
            let [tw, th] = self.bed.sprite_size().unwrap_or([1.0, 1.0]);
            let s = (TILE * ZOOM - 2.0) / tw.max(th);
            for (i, (cell, color)) in game.level.beds.iter().enumerate() {
                let node = &mut ctx.scene().root.children[base + i];
                node.shape = Some(self.bed.clone());
                node.transform = Transform::translate(self.screen(self.cell_px(*cell)));
                node.scale = [s, s];
                // Each bed wears its sleeper's colour — the sprite is
                // the modulate's base cloth, not the other way round.
                node.modulate = ant_color(*color);
                node.lit = true;
                node.order = 0.05;
            }
        }
        for id in 0..game.ants.len() {
            let Some(g) = self.glides.get(&id) else {
                continue;
            };
            let asleep = matches!(game.ants[id].state, game::AntState::Asleep(_));
            let face = *facing.get(&id).unwrap_or(&[1.0, 0.0]);
            let col = ant_color(game.ants[id].color);
            let col = if asleep {
                // Tucked in: blaze loud, then fade to the dim sleeping
                // variant. The flare's modulate runs past white and
                // lets the framebuffer clamp it — the clamp is what
                // makes loud look loud. The hold-then-fade curve keeps
                // the blaze a heartbeat before it dims.
                let dim = shade(col, 0.55);
                let k = (self.landing.get(&id).copied().unwrap_or(0.0) * 1.7).min(1.0);
                if k > 0.0 {
                    Color {
                        r: dim.r + (col.r * 2.4 + 0.9 - dim.r) * k,
                        g: dim.g + (col.g * 2.4 + 0.9 - dim.g) * k,
                        b: dim.b + (col.b * 2.4 + 0.9 - dim.b) * k,
                        a: 1.0,
                    }
                } else {
                    dim
                }
            } else if matches!(game.ants[id].state, game::AntState::Wild(_)) {
                shade(col, 0.88)
            } else {
                col
            };
            let z = 1.0 + g.at[1] * 1e-4;
            let node = &mut ctx.scene().root.children[LEAVES.len() + id];
            let at = self.screen(g.at);
            // Facings turn, never snap: toward travel on foot, toward
            // the ceiling when tucked in.
            let want_face = if asleep { [0.0, 1.0] } else { face };
            let rf = self.render_face.entry(id).or_insert(want_face);
            *rf = turn_toward(*rf, want_face, TURN_RATE * dt);
            let rf = *rf;
            self.put_ant(node, frame_of(g.anim, asleep), at, rf, col, z);
        }
        // The zs: one born at each breath's crest, adrift and growing
        // from a small z to a full Z, wandering one way and then the
        // other, and thinning out as it climbs. Immediate text rides
        // above the dark like every other word in the game.
        for zz in &self.zs {
            let k = (zz.age / zz.ttl).min(1.0);
            let [x, y] = self.screen([
                zz.at[0] + 8.0 * (zz.seed + zz.age * 1.56).sin() * k,
                zz.at[1],
            ]);
            ctx.text(
                x,
                y,
                &self.font,
                "z",
                // Whole points only: the glyph atlas is keyed by size,
                // and a z that grew through fractional sizes would
                // bake itself a fresh atlas every frame.
                (16.0 + 36.0 * k).round(),
                700.0,
                Color {
                    r: C_DIM.r,
                    g: C_DIM.g,
                    b: C_DIM.b,
                    a: 1.0 - k * k,
                },
                1.56,
            );
        }
        // The player: a near-black copy of the same frame a couple of
        // pixels off, so the pale queen reads against even the darkest
        // tunnel, with her own frame on top.
        let n = game.ants.len();
        let frame = frame_of(self.player_anim, false);
        {
            let root = &mut ctx.scene().root;
            let back = n;
            let player = n + 1;
            let at = self.screen(self.player_at);
            self.put_ant(
                &mut root.children[LEAVES.len() + back],
                frame,
                [at[0] + 1.5 * ZOOM, at[1] - 1.5 * ZOOM],
                self.player_face,
                Color {
                    r: 0.06,
                    g: 0.05,
                    b: 0.04,
                    a: 1.0,
                },
                1.48,
            );
            self.put_ant(
                &mut root.children[LEAVES.len() + player],
                frame,
                at,
                self.player_face,
                ant_color(game::AntColor::White),
                1.5,
            );
        }
    }

    /// Draw the verdict: one clean spin from a speck, eased so the
    /// speck lingers tiny before it blooms to half the screen; a
    /// heartbeat at full size; then the fade to the hill's own words.
    fn draw_splash(&self, ctx: &mut Context) {
        let Some(s) = &self.splash else { return };
        let (w, _) = ctx.size();
        let k = (s.t / s.grow).min(1.0);
        let e = k * k * (3.0 - 2.0 * k);
        // The word is baked once, at one fixed size, and the growth
        // rides the node's scale: the whole splash needs exactly one
        // glyph atlas, and the bloom is smooth at every step.
        const SPLASH_EM: f32 = 200.0;
        // Full growth: the word fills half the screen's width.
        let scale = (e * w / (1.3 * SPLASH_EM * s.text.len().max(4) as f32)).max(0.002);
        // Five full turns — a quarter faster spinning than four —
        // landing on the drawn tilt within 45 degrees of level; the
        // random growth time keeps the landing unpredictable, the
        // angle is simply where the turn stops.
        let angle = (5.0 * std::f32::consts::TAU + s.end) * e;
        let alpha = if s.t <= s.grow + Splash::HOLD {
            1.0
        } else {
            (1.0 - (s.t - s.grow - Splash::HOLD) / Splash::FADE).max(0.0)
        };
        // The example's own font Arc, handed over whole: the glyph
        // atlas is keyed on the buffer's pointer, and a fresh Arc per
        // frame (what `text_bytes` builds) would mint a fresh atlas
        // every frame — the flicker this line is careful about.
        let mut shape =
            match frost::Shape::text_shared(Arc::clone(&self.font), s.text.clone(), SPLASH_EM) {
                Ok(shape) => shape,
                Err(_) => return,
            };
        if let frost::Shape::Text {
            color, alpha: a, ..
        } = &mut shape
        {
            *color = C_TEXT;
            *a = alpha;
        }
        ctx.scene().root.children.push(Box::new(SceneNode {
            shape: Some(shape),
            transform: Transform::rotate(angle),
            scale: [scale, scale],
            order: UI + 1.0,
            ..Default::default()
        }));
    }

    fn draw_hud(&self, ctx: &mut Context) {
        let Some(game) = &self.game else { return };
        let (_, h) = ctx.size();
        // Barrfont carries letters, digits and spaces — nothing else —
        // so every separator in the game is spaces. And since text draws
        // centred on its point, everything centres: a doubled size can
        // never spill off an edge the way a left edge would.
        ctx.text(
            0.0,
            h / 2.0 - 34.0,
            &self.font,
            format!(
                "{}  hill {} of {}",
                game.level.name,
                self.idx + 1,
                self.levels.len()
            ),
            34.0,
            700.0,
            C_TEXT,
            UI,
        );
        ctx.text(
            0.0,
            h / 2.0 - 78.0,
            &self.font,
            format!(
                "warm {} of {}   digs {}   moves {} of {}",
                game.ants_asleep(),
                game.ants.len(),
                game.digs_left,
                game.moves,
                game.level.par
            ),
            30.0,
            400.0,
            C_TEXT,
            UI,
        );
        ctx.text(
            0.0,
            -h / 2.0 + 26.0,
            &self.font,
            "arrows or click walk   shift arrows or shift click push",
            24.0,
            400.0,
            C_DIM,
            UI,
        );
        ctx.text(
            0.0,
            -h / 2.0 + 58.0,
            &self.font,
            "space or right click detach   z undo   r restart   n p hills   f11 fullscreen",
            24.0,
            400.0,
            C_DIM,
            UI,
        );
        if self.note_t > 0.0 {
            let a = (self.note_t / 0.6).min(1.0);
            ctx.text(
                0.0,
                -h / 2.0 + 106.0,
                &self.font,
                self.note.clone(),
                30.0,
                600.0,
                Color {
                    r: C_NOTE.r,
                    g: C_NOTE.g,
                    b: C_NOTE.b,
                    a,
                },
                UI + 1.0,
            );
        }
        if game.won && self.splash.is_none() {
            ctx.text(
                0.0,
                44.0,
                &self.font,
                "every ant is warm  winter may come",
                52.0,
                700.0,
                C_TEXT,
                UI + 2.0,
            );
            ctx.text(
                0.0,
                -8.0,
                &self.font,
                format!(
                    "{} moves  par {}{}",
                    game.moves,
                    game.level.par,
                    if self.frost_bit {
                        "  the frost bit"
                    } else {
                        ""
                    }
                ),
                30.0,
                400.0,
                C_DIM,
                UI + 2.0,
            );
            ctx.text(
                0.0,
                -62.0,
                &self.font,
                "N  the next hill",
                30.0,
                400.0,
                C_NOTE,
                UI + 2.0,
            );
        }
    }
    /// Shift+left-click: the push verb for the pointing hand. A push
    /// runs along one straight line, so the click supplies no
    /// destination — it supplies an aim. The dominant axis from the
    /// player to the clicked cell is the direction the arrow would have
    /// been, had Shift turned it. Aiming at your own feet shoves nobody.
    fn aimed_push(&self, at: [f32; 2]) -> Option<game::Dir> {
        let game = self.game.as_ref()?;
        if game.won {
            return None;
        }
        let goal = self.cell_at_screen(at);
        let dx = goal.col - game.player.col;
        let dy = goal.row - game.player.row;
        if dx == 0 && dy == 0 {
            return None;
        }
        Some(if dx.abs() >= dy.abs() {
            if dx > 0 {
                game::Dir::Right
            } else {
                game::Dir::Left
            }
        } else if dy > 0 {
            game::Dir::Down
        } else {
            game::Dir::Up
        })
    }

    /// Click-walking, one step per completed glide: the queen waits for
    /// each hop to land before the next click-step starts, so a long
    /// click route reads as the same walk the keys make, not a slide.
    /// Any refusal — a line that packed into the way, a sleeper claimed,
    /// the hill sleeping — ends the route right there.
    fn walk_the_route(&mut self) {
        if self.route.is_empty()
            || self.spin.is_some()
            || !closer(self.player_at, self.player_target)
        {
            return;
        }
        // A reversal of the travelling direction is a turn, not a
        // step: spin in place and leave the direction queued — the
        // route resumes the instant the about-face lands.
        let dir = self.route[0];
        let to = face_of(dir);
        if self.face[0] * to[0] + self.face[1] * to[1] < -0.99 {
            self.spin = Some(to);
            return;
        }
        let Some(game) = &mut self.game else {
            return;
        };
        if game.won {
            self.route.clear();
            return;
        }
        self.route.remove(0);
        if game.move_player(dir).is_err() {
            self.route.clear();
        }
    }
}

impl Process for Demo {
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        #[cfg(feature = "diag")]
        self.diag.process(ctx, dt);
        // Once, the window takes the game's own name: the engine opens
        // every app under its generic default, and on the web winit
        // carries a window title straight into the tab.
        if !self.title_set
            && let Some(window) = ctx.window()
        {
            window.set_title("Antifrost");
            self.title_set = true;
        }
        // F11 flips the window's fullscreen state — the runtime twin of
        // `Config::fullscreen`, reached through the engine's own window.
        if self.pressed(ctx, KeyCode::F11)
            && let Some(window) = ctx.window()
        {
            if window.fullscreen().is_some() {
                window.set_fullscreen(None);
            } else {
                window.set_fullscreen(Some(frost::Fullscreen::Borderless(None)));
            }
        }
        if self.game.is_some() {
            let shift = ctx.key_down(KeyCode::ShiftLeft) || ctx.key_down(KeyCode::ShiftRight);
            let mut action: Option<(game::Dir, bool)> = None;
            for (key, dir) in [
                (KeyCode::ArrowLeft, game::Dir::Left),
                (KeyCode::KeyA, game::Dir::Left),
                (KeyCode::ArrowRight, game::Dir::Right),
                (KeyCode::KeyD, game::Dir::Right),
                (KeyCode::ArrowUp, game::Dir::Up),
                (KeyCode::KeyW, game::Dir::Up),
                (KeyCode::ArrowDown, game::Dir::Down),
                (KeyCode::KeyS, game::Dir::Down),
            ] {
                if self.pressed(ctx, key) {
                    action = Some((dir, shift));
                }
            }
            let detach = self.pressed(ctx, KeyCode::Space);
            let click_left = self.pressed_mouse(ctx, MouseButton::Left);
            let click_right = self.pressed_mouse(ctx, MouseButton::Right);
            // Right-click detaches; shift+left-click pushes instead.
            let detach_click = click_right;
            let undo = self.pressed(ctx, KeyCode::KeyZ);
            let restart = self.pressed(ctx, KeyCode::KeyR);
            let next = self.pressed(ctx, KeyCode::KeyN);
            let prev = self.pressed(ctx, KeyCode::KeyP);
            if next {
                self.load(self.idx + 1);
            }
            if prev {
                self.load(self.idx + self.levels.len().saturating_sub(1));
            }
            // Shift+left-click is the pointer's push verb. The aim is
            // looked up before the game is handed out mutable — this
            // only reads.
            let push_aim = if click_left && shift {
                ctx.mouse_position().and_then(|pos| self.aimed_push(pos))
            } else {
                None
            };
            if let Some(game) = &mut self.game {
                if restart {
                    let level = game.level.clone();
                    *game = game::Game::start(level);
                    self.route.clear();
                } else if undo {
                    game.undo();
                    self.route.clear();
                } else if (detach || detach_click) && !game.won {
                    self.route.clear();
                    let _ = game.detach();
                } else if let Some(dir) = push_aim.filter(|_| !game.won) {
                    // The shove itself is the game's; the pusher faces
                    // the aim like the keys do, and a push — like the
                    // keyboard's — never carries the pusher along. A
                    // refusal says why, in the game's own words.
                    self.route.clear();
                    self.face = face_of(dir);
                    if let Err(why) = game.push(dir) {
                        self.note = why.to_string();
                        self.note_t = 1.6;
                    }
                } else if let Some((dir, push)) = action.filter(|_| !game.won) {
                    self.route.clear();
                    if push {
                        // A push never moves the pusher, so no glide will
                        // carry its facing — turn by hand.
                        self.face = face_of(dir);
                        let _ = game.push(dir);
                    } else if self.spin.is_none() {
                        // The keyboard's 180 obeys the route's law:
                        // about-face in place, then walk.
                        let to = face_of(dir);
                        if self.face[0] * to[0] + self.face[1] * to[1] < -0.99 {
                            self.spin = Some(to);
                        } else {
                            let _ = game.move_player(dir);
                        }
                    }
                }
            }
            // Plain left-click: a route to the cell under the pointer.
            if click_left
                && !shift
                && let Some(pos) = ctx.mouse_position()
            {
                let goal = self.cell_at_screen(pos);
                match self.find_route(goal) {
                    Some(path) => self.route = path,
                    None => {
                        self.note = "the hill will not open that way".to_string();
                        self.note_t = 1.6;
                    }
                }
            }
        }
        self.note_t = (self.note_t - dt).max(0.0);
        // Landing flare bookkeeping: an ant just tucked in lights at
        // full; the flash decays over `LAND_FLASH`; an ant woken by
        // undo forgets it ever blazed.
        if let Some(game) = &self.game {
            for (id, ant) in game.ants.iter().enumerate() {
                match ant.state {
                    game::AntState::Asleep(_) => {
                        self.landing.entry(id).or_insert(1.0);
                    }
                    _ => {
                        self.landing.remove(&id);
                    }
                }
            }
            // The player is an ant, and the queen's bed is her target:
            // standing on it tucks her in like any ant — spike, then
            // dark. Stepping off (undo, stubbornness) relights the lamp.
            self.player_home = match game.tile(game.player) {
                level::Tile::Bed(game::AntColor::White) => {
                    Some(self.player_home.unwrap_or(1.0).max(0.0))
                }
                _ => None,
            };
        }
        for t in self.landing.values_mut() {
            *t = (*t - dt / LAND_FLASH).max(0.0);
        }
        if let Some(t) = &mut self.player_home {
            *t = (*t - dt / LAND_FLASH).max(0.0);
        }
        // Breath tick: every lamp's phase and exertion, the player's
        // last. The resting flag is sleep for an ant, home for her.
        if let Some(game) = &self.game {
            let lamps: Vec<(usize, [f32; 2], bool)> = (0..game.ants.len())
                .map(|id| {
                    (
                        id,
                        self.glides.get(&id).map(|g| g.at).unwrap_or([0.0, 0.0]),
                        matches!(game.ants[id].state, game::AntState::Asleep(_)),
                    )
                })
                .collect();
            let home = (game.ants.len(), self.player_at, self.player_home.is_some());
            for (id, at, resting) in lamps {
                self.breathe(id, at, resting, dt);
            }
            self.breathe(home.0, home.1, home.2, dt);
        }
        // The zs drift up and grow old.
        for z in &mut self.zs {
            z.age += dt;
            z.at[0] += z.vel[0] * dt;
            z.at[1] += z.vel[1] * dt;
        }
        self.zs.retain(|z| z.age < z.ttl);
        // Facings turn and never snap: the player spins through her
        // 180s, and a tucked-in queen lifts her head to the ceiling.
        let face_to = if self.player_home.is_some() {
            [0.0, 1.0]
        } else if let Some(s) = self.spin {
            s
        } else {
            self.face
        };
        self.player_face = turn_toward(self.player_face, face_to, TURN_RATE * dt);
        if let Some(s) = self.spin
            && self.player_face[0] * s[0] + self.player_face[1] * s[1] > 0.999
        {
            self.face = s;
            self.spin = None;
            self.player_face = s;
        }
        // The verdict: when the hill is won and has not yet been told
        // its own name, draw one from the hat — the praise tier reads
        // the par, the spin reads the dice.
        if let Some(game) = &self.game {
            if !game.won {
                self.splash_done = false;
            } else if self.splash.is_none() && !self.splash_done {
                let praise: [&str; 3] = if game.level.par > 0 && game.moves <= game.level.par {
                    ["EXCELLENT", "PERFECT", "SUPER"]
                } else {
                    ["YOUR DID IT", "GOOD JOB", "WELL DONE"]
                };
                let pick = self.leaf_rng.in_range(0.0, 2.999) as usize;
                let text = praise[pick].to_string();
                let grow = self.leaf_rng.in_range(1.875, 2.55);
                let end = self.leaf_rng.in_range(-45.0, 45.0) * std::f32::consts::FRAC_PI_4 / 45.0;
                self.splash = Some(Splash {
                    text,
                    t: 0.0,
                    grow,
                    end,
                });
            }
        }
        if let Some(s) = &mut self.splash {
            s.t += dt;
            if s.t >= s.total() {
                self.splash = None;
                self.splash_done = true;
            }
        }
        self.refresh(dt);
        self.update_camera(ctx, dt);
        self.walk_the_route();

        self.update_leaves(ctx, dt);
        self.draw_ants(ctx, dt);
        self.draw_hud(ctx);
        self.draw_splash(ctx);
    }
}

/// A browser has no stderr: route whatever log survives to the
/// devtools console, and turn panics into console errors. (The crate
/// compiles `log` calls out of release builds — `release_max_level_off`
/// — and wasm builds are release builds, so in the shipped page this
/// mostly catches panics; `web/index.html` carries a page-side probe
/// for the rest.)
#[cfg(target_arch = "wasm32")]
fn init_logging() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        console_error_panic_hook::set_once();
        struct Console;
        impl log::Log for Console {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                let line = format!("[{:?}] {}", record.level(), record.args());
                web_sys::console::log_1(&line.into());
            }
            fn flush(&self) {}
        }
        static LOGGER: Console = Console;
        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
        }
    });
}

/// The command line's arguments.
#[cfg(not(target_arch = "wasm32"))]
#[derive(clap::Parser)]
struct Cli {
    /// Present frames in sync with the display's vertical blank. `off`
    /// uncaps the frame rate — the hill has the headroom (the experiment
    /// hit 200 fps), but the cadence judders, so vsync stays on unless
    /// this flag says otherwise.
    ///
    /// An `Option`: a bare `bool` field is clap's zero-arg `--flag`,
    /// which has no value to parse.
    #[arg(long, value_name = "on|off", value_parser = parse_on_off)]
    vsync: Option<bool>,
}

/// clap's value parser for the vsync flag: `on`/`off`, any case.
#[cfg(not(target_arch = "wasm32"))]
fn parse_on_off(s: &str) -> Result<bool, String> {
    if s.eq_ignore_ascii_case("on") {
        Ok(true)
    } else if s.eq_ignore_ascii_case("off") {
        Ok(false)
    } else {
        Err(format!("expected `on` or `off`, got `{s}`"))
    }
}

fn main() {
    // The hill's pin: vsync keeps presentation on the display's cadence.
    // A browser has no command line, so the packaged build keeps the
    // default; native can hand the uncapped experiment to the caller
    // with `--vsync off`.
    #[cfg(target_arch = "wasm32")]
    let vsync = true;
    #[cfg(not(target_arch = "wasm32"))]
    let vsync = {
        let cli: Cli = clap::Parser::parse();
        cli.vsync.unwrap_or(true)
    };
    // Native logs to stderr; a browser has no stderr, so there the
    // panic hook surfaces failures in the devtools console instead of
    // the module dying silently.
    #[cfg(not(target_arch = "wasm32"))]
    env_logger::init();
    #[cfg(target_arch = "wasm32")]
    init_logging();
    // One particle batch per leaf sheet, riding the identity transform so
    // the batches live in the window's user space, like every immediate
    // draw. A leaf PNG that fails to decode is a broken build: say so at
    // once, not at the first frame.
    let leaf_batches: Vec<Box<SceneNode>> = LEAVES
        .iter()
        .map(|png| {
            ParticleShape::sprite_bytes(*png).unwrap_or_else(|err| {
                log::error!("antifrost: bad leaf sprite: {err}");
                std::process::exit(1);
            })
        })
        .map(|shape| {
            Box::new(SceneNode {
                shape: Some(frost::Shape::Particles {
                    system: ParticleSystem::default(),
                    color: Color {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                    shape,
                }),
                order: LEAF_Z,
                // Leaves hang in the lit world: night dims them, an
                // ant's lamp picks them out as it passes under.
                lit: true,
                ..Default::default()
            })
        })
        .collect();
    // The ant's walk cycle, baked to texture once; a frame that fails to
    // decode is as broken as a broken leaf sheet.
    let ant_frames: Vec<frost::Shape> = ANT_FRAMES
        .iter()
        .map(|png| {
            ant_frame(png).unwrap_or_else(|err| {
                log::error!("antifrost: bad ant sprite: {err}");
                std::process::exit(1);
            })
        })
        .collect();
    // The backdrop texture, baked once like everything else.
    let soil = frost::Shape::sprite_bytes(SOIL).unwrap_or_else(|err| {
        log::error!("antifrost: bad soil sprite: {err}");
        std::process::exit(1);
    });
    let mut scene = Scene::new(SceneNode {
        shape: Some(frost::Shape::Background { color: C_BG }),
        children: leaf_batches,
        ..Default::default()
    });
    // The night settles in: every lit pixel floors at this dark, and
    // only the ants' lamps push it back up.
    scene.ambient = C_NIGHT;
    if let Err(err) = frost::run_configured(
        scene,
        Demo::new(ant_frames, soil),
        frost::Config {
            window_size: Some([1920, 1080]),
            // The hill opens full screen; F11 flips to the windowed
            // size above, and again back.
            fullscreen: true,
            // The hill always renders at 1080p and lets the stretch pass
            // carry it to the window or the display: this is a small,
            // cozy game for modest hardware, and the 4K-native experiment
            // cost ~35 ms a frame on a modern iGPU — most of it per-pixel
            // lighting arithmetic that the fixed buffer simply never
            // pays. A window the buffer's own size skips both the buffer
            // and the stretch pass, so windowed 1080p costs nothing
            // extra; Alt-F flips the stretch between bilinear (the soft,
            // default carry) and nearest.
            render_size: Some([1920, 1080]),
            // Vsync on unless the command line says otherwise: the
            // vsync-off experiment hit 200 fps with heavy judder, so
            // the display's cadence stays the default pin.
            vsync,
            ..Default::default()
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod click_tests {
    use super::*;

    /// A `Demo` is pure CPU work until `run_configured`: asset bake,
    /// level load, route math — all testable headless.
    fn demo() -> Demo {
        let frames = ANT_FRAMES
            .iter()
            .map(|png| ant_frame(png).expect("shipped ant frames bake"))
            .collect();
        let soil = frost::Shape::sprite_bytes(SOIL).expect("shipped soil bakes");
        Demo::new(frames, soil)
    }

    #[test]
    fn click_routes_walk_the_tunnels() {
        let d = demo(); // First Warm: '@' at (1, 3), tunnels open, rock seals the frame.
        let route = d
            .find_route(game::Cell::new(1, 1))
            .expect("bed above is open");
        assert_eq!(route, vec![game::Dir::Up, game::Dir::Up], "two steps north");
        assert_eq!(
            d.find_route(game::Cell::new(1, 3)).unwrap().len(),
            0,
            "clicking your own cell goes nowhere"
        );
        assert!(
            d.find_route(game::Cell::new(0, 0)).is_none(),
            "rock is no destination"
        );
        assert!(
            d.find_route(game::Cell::new(3, 1)).is_some(),
            "a wild ant's cell is routable — walking over recruits"
        );
    }

    #[test]
    fn shift_click_push_aims_along_the_dominant_axis() {
        let d = demo(); // First Warm: '@' at (1, 3)
        // Screen space washes out: a cell pushed through `screen` and
        // back through `cell_at_screen` lands on itself, whatever the
        // camera thinks.
        let aim = |col, row| d.aimed_push(d.screen(d.cell_px(game::Cell::new(col, row))));
        assert_eq!(
            aim(3, 1),
            Some(game::Dir::Right),
            "a north-east aim is east-dominant: shove east"
        );
        assert_eq!(aim(1, 1), Some(game::Dir::Up), "straight up the shaft");
        assert_eq!(aim(1, 3), None, "aimed at your own feet, nobody shoves");
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod cli_tests {
    use super::*;

    fn vsync_of(args: &[&str]) -> bool {
        <Cli as clap::Parser>::parse_from(std::iter::once("antifrost").chain(args.iter().copied()))
            .vsync
            .unwrap_or(true)
    }

    #[test]
    fn vsync_defaults_to_on() {
        assert!(
            vsync_of(&[]),
            "no flag: the display's cadence stays the pin"
        );
    }

    #[test]
    fn vsync_flag_toggles_either_way() {
        assert!(!vsync_of(&["--vsync", "off"]), "off uncaps the frame rate");
        assert!(vsync_of(&["--vsync", "on"]), "on is spellable out loud");
        assert!(
            !vsync_of(&["--vsync", "OFF"]),
            "case is not part of the deal"
        );
    }

    #[test]
    fn any_other_vsync_value_is_rejected() {
        let err = <Cli as clap::Parser>::try_parse_from(["antifrost", "--vsync", "maybe"]);
        assert!(err.is_err(), "the flag speaks on or off, nothing else");
    }
}
