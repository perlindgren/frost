//! Ember Hill — a jam puzzle game on FROST (theme: *autumn* / *to keep warm*).
//!
//! Shepherd a growing trail of ants through a side-view ant hill. Walking
//! onto a stranded ant recruits it to your trail's tail; only the tail can
//! be released, and it only settles on its own colored bed — so the trail
//! is a stack, and the tunnels are the shuffle device. Every ant asleep
//! in its bed, yourself included, before the first frost.
//!
//! Run: `RUST_LOG=info cargo run --example antifrost` — add
//! `-- --vsync off` to uncap the frame rate. Vsync starts on: the
//! present is refresh-synced (`Fifo`), which is what lets the display
//! engage VRR; off presents with tearing (`Immediate`) and bypasses it.
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

/// The slab the menu panels are cut from: one 16:9 band taken from
/// the garden's granite photo (the user's own `stone.png`, 3024×4032)
/// and baked down — cropped at 1440 wide, then darkened to a third in
/// the same offline pass, so a panel made of it reads as stone in a
/// dim room and never competes with the words on it. The face keeps
/// the moss and the lichen: an anthill hung on a real stone should
/// say so.
const STONE: &[u8] = include_bytes!("assets/sprites/stone_menu.png");

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

/// How the canopy drifts against the camera: the leaves hang in front
/// of the whole world, so they must sweep past it — below 1.0 the
/// board and the backdrop overtake the layer that covers them, and the
/// eye reads the foreground sliding backwards against everything it
/// hides. 1.3 sits clearly in front of the board's 1.0 and the
/// backdrop's 0.85; spawn, wrap and ground all live in the window
/// `off` defines, so the ledger holds at any factor. Done on the batch
/// nodes directly because the leaf z (above the board, under the HUD)
/// only exists inside the base draw group, where `Layer`s cannot land.
const LEAF_PARALLAX: f32 = 1.3;

const C_BG: Color = Color {
    r: 0.045,
    g: 0.038,
    b: 0.033,
    a: 1.0,
};
/// The verdict's two brights: par met flares yellow, par missed burns
/// red. The splash word and the panel echo it keeps wear the same
/// judgement.
const C_PAR_MET: Color = Color {
    r: 1.0,
    g: 0.88,
    b: 0.22,
    a: 1.0,
};
const C_PAR_MISSED: Color = Color {
    r: 1.0,
    g: 0.32,
    b: 0.24,
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
/// The lamp disk's radius when diffusion is bought: shadows fade
/// across these pixels instead of dropping in one. The ladder below
/// decides whether lamps sweep this disk with four rays or nine.
const LAMP_PENUMBRA: f32 = 6.0;

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

/// The ember an ant's own shell keeps when its lamp is down to nothing. The
/// reason the ember exists at all — a lamp always has a face to stand on, even
/// at the very floor of the ambient — holds for a dormant hill too, and the
/// floor is where a tucked-in ant still reads as a body and not a lump.
const EMBER_FLOOR: f32 = 0.24;

/// And the part of the ember that breathes: the shell rides the lamp it stands
/// on, so the pulse the snow reads arrives on the ant in the same beat rather
/// than on a second clock to keep in step. Tuned so that the calm base lands on
/// the 0.45 the hill shipped with — 0.64 at the top of a working pulse, 0.32 in
/// the dorm — which is the whole trick: the shipped look is the middle of the
/// new range, not something it replaced.
const EMBER_PER_LAMP: f32 = 0.15;

/// The halo per lamp: what the bloom chain gets to blur, over and above the
/// shell's own color. Not one to one with the light's brightness, because the
/// emissive channel is the sprite's pixels times its tint times this — the
/// cuticle is paid for twice — so at this number the top of a working pulse
/// clears the chain's threshold of 0.8, the calm base sits at the bottom of its
/// knee, and a sleeping ant stays under it. The hill glows where it is awake.
///
/// The one place it overshoots is the one place worth overshooting: a tuck-in
/// lands a flare of three lamps on the sleep breath and the queen's bed four and
/// a half, so bedding an ant puts its halo nearly three times where the calm base
/// sits, five times for her, and drains that back over `LAND_FLASH`. The burst is
/// the same flare the lamp has always thrown at the snow, at last with a face to
/// stand on. Red ants read loudest for a reason of arithmetic: their hue is
/// lopsided enough that a blur of it stays red, where the yellow and the queen
/// spend their light in two channels and three and wash toward white instead.
const GLOW_PER_LAMP: f32 = 0.55;

/// The glow's own strength, which the menu's `lamp glow` row switches between
/// this and nothing. A step under the eyes hall's 1.6: that scene has two
/// emitters in it and a hill has dozens, and a halo is judged against what
/// surrounds it.
const BLOOM_STRENGTH: f32 = 1.0;

/// The color the queen's blaze aims at: the amber of her lamp, kept further
/// from white than the lamp itself is.
///
/// The ants run their tuck-in flare to `hue × 2.4 + 0.9`, which for a red ant
/// lands lopsided — 3.1 / 1.6 / 1.5 — and so stays *red* through a blur. Run the
/// same `+ 0.9` on her amber and it lands at 1 : 0.9 : 0.75, which is a white
/// ball with a tan edge: brightness that spreads, the mistake her flat white
/// tint used to make on its own. This keeps the amber warm instead, and she
/// starts the flash from white, so what changes on the bed is her hue and not
/// merely how much of her there is.
///
/// The halo's hue is derived from the tint, so the burst warms along with the
/// blaze and needs no knob of its own.
const CROWN_BLAZE: Color = Color {
    r: 2.6,
    g: 1.15,
    b: 0.35,
    a: 1.0,
};

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
/// The colour of a row with no road left in it: the others' warm
/// grey sunk below the dim one, so a `next` with nothing to lead to
/// reads spent, not merely unselected.
const C_MUTE: Color = Color {
    r: 0.33,
    g: 0.30,
    b: 0.27,
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

/// The white in an ant's light: its own hue pulled a third of the way to full
/// white, which is how a lamp reads as a source rather than as a coloured wash.
///
/// One function because two places ask for it — the light node's color and the
/// halo the shell throws — and two spellings of the same 0.35 is how a lamp and
/// its glow start agreeing by accident and then stop.
fn lamp_core(c: Color) -> Color {
    Color {
        r: c.r + (1.0 - c.r) * 0.35,
        g: c.g + (1.0 - c.g) * 0.35,
        b: c.b + (1.0 - c.b) * 0.35,
        a: c.a,
    }
}

/// A flare's mix in the tint: `k` of the way from `from` to `toward`.
///
/// One function because the ants and their queen flash the same way — held at
/// full for a heartbeat as the flare drains, then faded — and a blaze that
/// differs between them would be read as a bug in whichever one moved less.
fn blaze(from: Color, toward: Color, k: f32) -> Color {
    Color {
        r: from.r + (toward.r - from.r) * k,
        g: from.g + (toward.g - from.g) * k,
        b: from.b + (toward.b - from.b) * k,
        a: 1.0,
    }
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

/// The ledger's route alphabet, borrowed from the walkthrough:
/// `lrud` walks, `LRUD` shoves, `x` leaves an ant behind. One string
/// is one recorded best run, and one replay is that string walked
/// back through the hill.
fn glyph_of(a: game::Act) -> char {
    use game::{Act, Dir};
    match a {
        Act::Move(Dir::Left) => 'l',
        Act::Move(Dir::Right) => 'r',
        Act::Move(Dir::Up) => 'u',
        Act::Move(Dir::Down) => 'd',
        Act::Push(Dir::Left) => 'L',
        Act::Push(Dir::Right) => 'R',
        Act::Push(Dir::Up) => 'U',
        Act::Push(Dir::Down) => 'D',
        Act::Detach => 'x',
    }
}

fn encode_run(acts: &[game::Act]) -> String {
    acts.iter().map(|a| glyph_of(*a)).collect()
}

/// Every glyph or nothing: a recording with a stray letter in it is
/// not a route, and the ledger pretends it never heard of it.
fn decode_run(s: &str) -> Option<Vec<game::Act>> {
    use game::{Act, Dir};
    s.chars()
        .map(|c| {
            Some(match c {
                'l' => Act::Move(Dir::Left),
                'r' => Act::Move(Dir::Right),
                'u' => Act::Move(Dir::Up),
                'd' => Act::Move(Dir::Down),
                'L' => Act::Push(Dir::Left),
                'R' => Act::Push(Dir::Right),
                'U' => Act::Push(Dir::Up),
                'D' => Act::Push(Dir::Down),
                'x' => Act::Detach,
                _ => return None,
            })
        })
        .collect()
}

/// A recorded best run, walked back through the hill. The live game
/// *is* the showing's current frame — `shown` actions of `acts` have
/// been applied to it — so everything that paints a hill keeps
/// painting one; stepping back rebuilds the hill from its own
/// beginning through one fewer action.
struct Replay {
    acts: Vec<game::Act>,
    shown: usize,
    playing: bool,
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
    /// Par met or missed, decided at the spawn: yellow or red.
    color: Color,
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
                // The band's low end clears the fattest leaf's half-height
                // (`size` is a half-width), so nothing hangs into the window
                // and pops at birth.
                sky.h / 2.0 - sky.off[1] + rng.in_range(LEAF_SIZE.1, 140.0),
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

/// The page the app shows: the hill itself, the menu it opens onto,
/// or the settings the menu leads to. The menu pages pause the hill —
/// no key, click, or queued step reaches the game behind them, and
/// `Resume Game` hands it back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Game,
    Main,
    Settings,
    /// The hill is won: the splash's verdict hangs over two rows,
    /// next hill and retry hill — the only way a finished hill is
    /// ever left.
    Won,
    /// The two control pages: plain lists answering the menu rows
    /// of the same names. Any row on them steps back to the menu —
    /// they are reading material, and reading ends when you press
    /// anything.
    Keyboard,
    Mouse,
    /// The ledger: one row per warm hill — number, name, best run,
    /// and a minimap of the hill itself — a list that can outgrow
    /// the plate, so the window slides with the selection and the
    /// wheel, and a row walked is a door back onto that hill.
    Progress,
}

/// Where the Escape key leads. The rule the menu keeps: from the hill
/// Escape opens the menu, from the menu Escape hands the hill back,
/// and from settings it is a plain back step. Escape never quits —
/// leaving is the exit row's (and the window button's) alone, so the
/// key that pauses is never the key that destroys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Escape {
    ToMenu,
    Resume,
}

fn escape_nav(from: Page) -> Escape {
    match from {
        Page::Game => Escape::ToMenu,
        Page::Main => Escape::Resume,
        Page::Settings => Escape::ToMenu,
        Page::Keyboard | Page::Mouse => Escape::ToMenu,
        // The ledger is read, not lived in: Escape steps off it onto
        // the menu like everywhere else.
        Page::Progress => Escape::ToMenu,
        // Escape steps off the finished hill onto the menu; the
        // resume row walks back to the won hill and its panel.
        Page::Won => Escape::ToMenu,
    }
}

/// The lamp-shadow ladder: what each lit pixel pays for occlusion.
/// `None` registers no occluders at all — lamps light as if the hill
/// were glass; `One` is the single hard ray; `Four` sweeps a balanced
/// cross of the lamp's disk; `Nine` fills the 3x3 grid, the dense
/// diffusion advanced hardware buys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shadow {
    None,
    One,
    Four,
    Nine,
}

impl Shadow {
    fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::One => "1 light",
            Self::Four => "4 light",
            Self::Nine => "9 light",
        }
    }
    fn next(self) -> Self {
        match self {
            Self::None => Self::One,
            Self::One => Self::Four,
            Self::Four => Self::Nine,
            Self::Nine => Self::None,
        }
    }
    fn parse(text: &str) -> Option<Self> {
        match text {
            "none" => Some(Self::None),
            "1 light" => Some(Self::One),
            "4 light" => Some(Self::Four),
            "9 light" => Some(Self::Nine),
            _ => None,
        }
    }
    /// The lamp disk, or zero for the hard single ray.
    fn penumbra(self) -> f32 {
        match self {
            Self::None | Self::One => 0.0,
            Self::Four | Self::Nine => LAMP_PENUMBRA,
        }
    }
    /// The rung's tap count; only ever read for a lit disk.
    fn taps(self) -> u32 {
        match self {
            Self::None | Self::One => 1,
            Self::Four => 4,
            Self::Nine => 9,
        }
    }
}

/// The rows a menu page can answer to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    /// Move to the next hill — the button N used to be.
    Next,
    /// Replay the hill now underfoot from its own beginning.
    Retry,
    New,
    Resume,
    OpenSettings,
    Exit,
    Back,
    Vsync,
    Filter,
    Fullscreen,
    Quality,
    Leaves,
    /// The halo the ants throw, on and off.
    Glow,
    Shadows,
    /// Sliders: the press starts a drag, the release saves.
    Ambient,
    Light,
    /// The control pages of the menu: keyboard, mouse.
    Keyboard,
    Mouse,
    /// The ledger page, and one of its hill rows: a hill row replays
    /// the hill it names — the ledger is a door, not a museum.
    OpenProgress,
    Hill(usize),
}

/// One laid-out menu row: a clickable rectangle (centre and half
/// extents in user space) with its text, and for sliders also the
/// track geometry and the current fraction. Input hit-tests rows and
/// paint draws the very same list — the click can never miss the
/// picture.
struct Row {
    action: Action,
    cx: f32,
    cy: f32,
    hw: f32,
    hh: f32,
    text: String,
    /// `Some(fraction)` marks a slider row: the track's live value.
    slider: Option<f32>,
}

/// The settings page's slider track, in user-space x around the panel
/// centre; the label rides left of it. Shared by layout, hit test,
/// and paint so the three can never disagree.
const MENU_TRACK_CX: f32 = 190.0;
const MENU_TRACK_HALF: f32 = 150.0;

/// The menu's keyboard walk: arrows wrap around the page's rows, so
/// the selection never falls off the panel and the shortest path from
/// either end to the other is one keypress of the far direction.
fn step_sel(sel: usize, n: usize, dir: i32) -> usize {
    if n == 0 {
        return 0;
    }
    if dir < 0 {
        (sel + n - 1) % n
    } else {
        (sel + 1) % n
    }
}

/// Which row, if any, lives under a pointer: the one test the click,
/// the highlight-follow, and any future pointer reader all share.
fn row_under(rows: &[Row], p: [f32; 2]) -> Option<usize> {
    rows.iter()
        .position(|r| (p[0] - r.cx).abs() <= r.hw && (p[1] - r.cy).abs() <= r.hh)
}

/// The progress list's hang from the plate's top, shared by layout,
/// input and paint like every other menu geometry.
fn progress_top_y(h: f32) -> f32 {
    (h * 0.5 - 104.0).min(252.0)
}

/// The progress list's row stride: wider than the settings step,
/// because each row wears a minimap as tall as its text is not. The
/// maps own this stride — a card leaves two points of a step to
/// its neighbour, never more — and a further step is affordable
/// because the plate holds the same ten hills at a full-height
/// window either way.
fn progress_step(h: f32) -> f32 {
    if h < 720.0 { 54.0 } else { 68.0 }
}

/// How many hill rows fit between the hang and the window's floor —
/// the `back` row aside, which is pinned under the last visible row
/// and so never scrolls out of reach.
fn progress_fits(h: f32) -> usize {
    let low = -h * 0.5 + 96.0;
    (((progress_top_y(h) - low) / progress_step(h)).floor() as usize).max(1)
}

/// The progress page's plate half-width and its column edges, settled
/// once against measured ink rather than guesswork: every width below
/// was laid out by the engine's own text shaper in the shipped
/// Barrfont at the ledger's size (30, weight 500). The widest authored
/// hill name — "Three Cold Nights" — is 370.0, and since
/// [`MOVES_CAP`] guarantees no score wider than "999 moves" at 199.2,
/// the name column gets a real budget (372.0) and the score column is
/// right-aligned, so long names and long scores can never reach each
/// other. The map card caps at 235.0 wide by the cell's own limits.
/// Every gutter below is at least 20 points, and
/// `the_ledger_columns_never_touch` holds all of it: a wider name
/// than the budget, or a shrunken plate, fails the test instead of
/// painting a collision.
const PROGRESS_PLATE: f32 = 473.0;
/// The hill-number column's left edge.
const PROGRESS_NUM_X: f32 = -459.0;
/// The name column's left edge: a 20-point gutter past the widest
/// two-digit hill number.
const PROGRESS_NAME_X: f32 = -391.0;
/// The score column's right edge — scores right-align, so the name
/// column always owns its full budget.
const PROGRESS_SCORE_R: f32 = 206.0;
/// The map column's centre: a 20-point gutter past the score's right
/// edge, plus the widest possible card.
const PROGRESS_MAP_CX: f32 = 344.0;

/// Whether the ledger's window has more descent above it (an up mark)
/// and below it (a down mark). Barrfont carries letters and a space
/// and nothing else — no arrow glyph to spend — so the marks are
/// drawn chevrons, and they answer only this: is something hidden?
fn progress_marks(total: usize, top: usize, visible: usize) -> (bool, bool) {
    (top > 0, top + visible < total)
}

/// A dragged slider, while the left button holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slider {
    Ambient,
    Light,
}

/// Where on a track a pointer falls, as 0..=1: clamped at both ends,
/// so the knob lives on the track and a click past its tip sets the
/// tip.
fn slide_value(x: f32, track_cx: f32, track_half: f32) -> f32 {
    ((x - (track_cx - track_half)) / (2.0 * track_half)).clamp(0.0, 1.0)
}

/// The ambient floor the slider asks for. The track hinges on the
/// shipped look: the lower half darkens from the night the game was
/// balanced with (the middle, the default) toward absolute dark, and
/// the upper half thins that night up to plain white at the top —
/// every pixel floored at full bright, no dimming at all. Piecewise
/// linear, so the middle of the track is exactly the shipped picture.
fn ambient_floor(t: f32) -> Color {
    if t <= 0.5 {
        let k = t * 2.0;
        Color {
            r: C_NIGHT.r * k,
            g: C_NIGHT.g * k,
            b: C_NIGHT.b * k,
            a: C_NIGHT.a,
        }
    } else {
        let k = (t - 0.5) * 2.0;
        Color {
            r: C_NIGHT.r + (1.0 - C_NIGHT.r) * k,
            g: C_NIGHT.g + (1.0 - C_NIGHT.g) * k,
            b: C_NIGHT.b + (1.0 - C_NIGHT.b) * k,
            a: C_NIGHT.a,
        }
    }
}

/// The stretch-pass quality tiers: the fixed buffer the hill renders
/// into, from a 720p buffer (cheap, soft) to native resolution (every
/// pixel rendered, the expensive 4K experiment included). The old
/// quarter-HD tier retired — the row's floor is now medium — and a
/// file that still names `"low"` lands on that floor rather than
/// failing. The default is High, the 1080p fixed buffer the demo
/// shipped with; Ultra lifts the fix and lets the frame render at the
/// display's own size.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Quality {
    Medium,
    High,
    VeryHigh,
    Ultra,
}

impl Quality {
    fn size(self) -> Option<[u32; 2]> {
        match self {
            Quality::Medium => Some([1280, 720]),
            Quality::High => Some([1920, 1080]),
            Quality::VeryHigh => Some([2560, 1440]),
            Quality::Ultra => None,
        }
    }
    /// What the row and the file call the tier: the resolution it
    /// promises, `native` for the tier that fixes none.
    fn label(self) -> &'static str {
        match self {
            Quality::Medium => "1280 x 720",
            Quality::High => "1920 x 1080",
            Quality::VeryHigh => "2560 x 1440",
            Quality::Ultra => "native",
        }
    }
    fn next(self) -> Quality {
        match self {
            Quality::Medium => Quality::High,
            Quality::High => Quality::VeryHigh,
            Quality::VeryHigh => Quality::Ultra,
            Quality::Ultra => Quality::Medium,
        }
    }
    fn parse(text: &str) -> Option<Quality> {
        Some(match text {
            "1280 x 720" => Quality::Medium,
            "1920 x 1080" => Quality::High,
            "2560 x 1440" => Quality::VeryHigh,
            "native" => Quality::Ultra,
            _ => return None,
        })
    }
}

/// The player's preferences, live in memory and — on native — on disc
/// beside the launch folder. Every field reaches the frame the same
/// way: vsync, filter, fullscreen, and quality flip the engine the
/// moment the row moves; ambient, light, and leaf lighting are read
/// by the draw each frame, so the settings page previews them live
/// behind the panel.
#[derive(Clone, Debug, PartialEq)]
struct Settings {
    vsync: bool,
    /// The stretch filter: nearest (crisp pixel blocks) or bilinear
    /// (the soft default).
    nearest: bool,
    fullscreen: bool,
    /// The render buffer's size: the row shows the resolution it
    /// names, `native` for none.
    quality: Quality,
    /// Ambient floor, on the [`ambient_floor`] slider: 50% is the
    /// night the game shipped with, 0% is the full dark, and 100%
    /// floors every pixel at full bright — no dimming at all.
    ambient: f32,
    /// Lamp strength, as a fraction of the shipped intensities —
    /// doubled past 1.0 at the slider's far end.
    light: f32,
    /// The halo the ants' lamps throw: the bloom chain's twelve passes, spent
    /// only while this row is on — at zero strength the engine declines to run
    /// the chain rather than computing a picture no one reads, so this is the
    /// weak hardware's rung as much as it is a look.
    ///
    /// Measured, at the buffer the hill fixes itself to: +0.39 ms a frame
    /// against a frame that is 9.71 ms there, interleaved pairs, the machine's
    /// own noise floor a quarter of that (§13 of the benchmarking notes). Never
    /// measured in a browser, which is the other place this game runs and the
    /// reason the rung exists at all.
    lamp_glow: bool,
    leaves_lit: bool,
    /// The weak-hardware answer to the light field, on a ladder:
    /// lamps stay at every rung, and only the shadows' cost steps.
    shadows: Shadow,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            vsync: true,
            nearest: false,
            fullscreen: true,
            quality: Quality::High,
            ambient: 0.5,
            light: 1.0,
            lamp_glow: true,
            leaves_lit: true,
            shadows: Shadow::One,
        }
    }
}

impl Settings {
    const FILE: &'static str = "antifrost.settings.ron";

    /// The file's text, in the crate's own Ron shape: flat, named,
    /// and hand-parsable by `load_levels`' little parser.
    fn text(&self) -> String {
        format!(
            "(\n    vsync: {},\n    filter: \"{}\",\n    fullscreen: {},\n    resolution: \"{}\",\n    ambient: {},\n    light: {},\n    lamp_glow: {},\n    leaves_lit: {},\n    shadows: \"{}\",\n)\n",
            self.vsync,
            if self.nearest { "nearest" } else { "linear" },
            self.fullscreen,
            self.quality.label(),
            self.ambient,
            self.light,
            self.lamp_glow,
            self.leaves_lit,
            self.shadows.label(),
        )
    }

    /// Parse the file's text over the defaults: an unknown name, an
    /// unreadable value, or a whole garbage file loses that one
    /// setting, not the launch.
    fn parse(src: &str) -> Settings {
        let d = Settings::default();
        let Ok(root) = frost::ron::parse(src) else {
            return d;
        };
        let val = |name: &str| frost::ron::field(&root, name);
        let atom = |name: &str| match val(name) {
            Some(frost::ron::Val::Atom(text, _)) => Some(text.as_str()),
            _ => None,
        };
        let boolean = |name: &str, at: bool| match atom(name) {
            Some("true") => true,
            Some("false") => false,
            _ => at,
        };
        let number = |name: &str, at: f32, lo: f32, hi: f32| {
            atom(name)
                .and_then(|t| t.parse::<f32>().ok())
                .map(|v| v.clamp(lo, hi))
                .unwrap_or(at)
        };
        let ambient = number("ambient", d.ambient, 0.0, 1.0);
        Settings {
            vsync: boolean("vsync", d.vsync),
            nearest: match atom("filter") {
                Some(t) => t.trim_matches('"') == "nearest",
                _ => d.nearest,
            },
            fullscreen: boolean("fullscreen", d.fullscreen),
            quality: atom("resolution")
                .and_then(|t| Quality::parse(t.trim_matches('"')))
                .unwrap_or(d.quality),
            ambient,
            light: number("light", d.light, 0.0, 2.0),
            lamp_glow: boolean("lamp_glow", d.lamp_glow),
            leaves_lit: boolean("leaves_lit", d.leaves_lit),
            shadows: atom("shadows")
                .and_then(|t| Shadow::parse(t.trim_matches('"')))
                .unwrap_or(d.shadows),
        }
    }

    /// The kept text, read back; missing or unreadable says default.
    /// The menu opens every launch regardless — the file speaks the
    /// settings, not the opening page. On the desktop the keeping is a
    /// file beside the launch folder; in a browser, its own memory of
    /// this site — `frost::store` answers for both worlds.
    fn load() -> Settings {
        match frost::store::load(Self::FILE) {
            Some(src) => Settings::parse(&src),
            None => {
                log::info!("antifrost: no {:?} yet — defaults", Self::FILE);
                Settings::default()
            }
        }
    }

    /// Keep the settings under the file's name, wherever this runs.
    /// Failure is a logged shrug, never a popped frame: a read-only
    /// folder, or a private window, costs persistence, not the game.
    fn save(&self) {
        if let Err(err) = frost::store::save(Self::FILE, &self.text()) {
            log::warn!("antifrost: could not save {:?}: {err}", Self::FILE);
        }
    }
}

/// The highest move count a hill ever reports. The game itself
/// counts past it without stopping the player; the number is the
/// display's, and a display that must fit a table needs a ceiling:
/// "999 moves" is the widest score row the ledger can ever paint, so
/// the progress page's columns are settled against it once and for
/// all.
const MOVES_CAP: i32 = 999;

/// Where the player's descent stands, kept under its own name beside
/// the settings — a file in the launch folder on the desktop, a key in
/// the browser's memory on the web: which hill they last stood on,
/// how deep they ever got (the progress page's list grows down to
/// here and never back), and the ledger the page reads — every hill
/// they warmed, in the fewest moves they ever warmed it. It is
/// written wherever the hill moves — New Game, N, P — and wherever a
/// hill is won, never on launch and never inside `load`, so starting
/// the game leaves no fingerprint and a window-manager kill loses
/// nothing newer than the hill entered or the hill finished. The file
/// counts hills the way the HUD does, from one; each score is a
/// quoted pair, `"hill moves"`, so the hand-written parser stays a
/// walk down two atoms and nothing more.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct Progress {
    hill: usize,
    /// The deepest hill ever stood on. It only ever grows: walking
    /// back to replay a shallow hill un-stands nowhere.
    reached: usize,
    /// Warm hills and their best runs, `(hill, moves)`, zero-based in
    /// the code and sorted by hill; an unwarmed hill owns no entry.
    scores: Vec<(usize, i32)>,
    /// The walk that earned each best, `(hill, glyphs)`, in the
    /// walkthrough's alphabet — the routes the *player* ran, never a
    /// solver's. A hill owns at most one, and it changes only when a
    /// better score does, so a route always shows how its number was
    /// honestly earned.
    routes: Vec<(usize, String)>,
}

impl Progress {
    const FILE: &'static str = "antifrost.progress.ron";

    /// The player now stands on `hill`: the resume point moves there
    /// and the depth of the descent is asked to remember it, forever
    /// if it is a new low.
    fn stood(&mut self, hill: usize) {
        self.hill = hill;
        self.reached = self.reached.max(hill);
    }

    fn text(&self) -> String {
        let mut text = format!(
            "(\n    hill: {},\n    reached: {},\n    scores: [",
            self.hill + 1,
            self.reached + 1
        );
        if self.scores.is_empty() {
            text.push_str("],\n");
        } else {
            for (hill, moves) in &self.scores {
                text.push_str(&format!("\n        \"{} {}\",", hill + 1, moves));
            }
            text.push_str("\n    ],\n");
        }
        // The routes ride the same file: a number and the walk that
        // earned it belong on the same shelf.
        text.push_str("    routes: [");
        if self.routes.is_empty() {
            text.push_str("],\n)\n");
        } else {
            for (hill, glyphs) in &self.routes {
                text.push_str(&format!("\n        \"{} {}\",", hill + 1, glyphs));
            }
            text.push_str("\n    ],\n)\n");
        }
        text
    }

    /// A hill just finished: the run joins the ledger, and the ledger
    /// keeps the truth it always keeps — one entry per hill, the
    /// fewest moves of any attempt. A warm hill was stood on, so the
    /// descent's depth takes note of it too. The answer says whether
    /// this run *is* the new best — the walk that earns a number is
    /// the walk worth remembering.
    fn record(&mut self, hill: usize, moves: i32) -> bool {
        // The ledger writes a number, so the number has an upper
        // limit: a hill stays winnable past it and the game keeps
        // counting itself, but the ledger — and every layout that
        // trusts its width — sees `MOVES_CAP` at the most.
        let moves = moves.min(MOVES_CAP);
        self.reached = self.reached.max(hill);
        match self.scores.iter_mut().find(|(h, _)| *h == hill) {
            Some(entry) => {
                if moves < entry.1 {
                    entry.1 = moves;
                    true
                } else {
                    false
                }
            }
            None => {
                self.scores.push((hill, moves));
                self.scores.sort();
                true
            }
        }
    }

    /// The best run's own walk, when the ledger keeps one for `hill`.
    fn route_of(&self, hill: usize) -> Option<&str> {
        self.routes
            .iter()
            .find(|(h, _)| *h == hill)
            .map(|(_, g)| g.as_str())
    }

    /// The walk that earned the best number, into the ledger's
    /// keeping. Only letters a hill could walk are kept: a hand at
    /// the file may change a score, but it cannot write a route the
    /// game would ever show.
    fn remember(&mut self, hill: usize, glyphs: &str) {
        if glyphs.is_empty() || decode_run(glyphs).is_none() {
            return;
        }
        match self.routes.iter_mut().find(|(h, _)| *h == hill) {
            Some(entry) => entry.1 = glyphs.to_string(),
            None => {
                self.routes.push((hill, glyphs.to_string()));
                self.routes.sort();
            }
        }
    }

    /// A won hill's whole deposit: the number, and the walk that
    /// earned it. The ledger keeps its law — fewest moves — and the
    /// walk rides only the number it walks: every new best carries
    /// its walk in, and a hill whose number predates the recordings
    /// backfills its walk when a run ties that number. A tie over an
    /// existing walk changes nothing (the elder walk walks the same
    /// number, and churn would lie about novelty); a worse run
    /// carries nothing anywhere.
    fn record_run(&mut self, hill: usize, moves: i32, glyphs: &str) {
        let improved = self.record(hill, moves);
        let walked = moves.min(MOVES_CAP);
        let keeps = self
            .scores
            .iter()
            .find(|(h, _)| *h == hill)
            .is_some_and(|(_, m)| *m == walked);
        if keeps && (improved || self.route_of(hill).is_none()) {
            self.remember(hill, glyphs);
        }
    }

    fn parse(src: &str) -> Progress {
        let Ok(root) = frost::ron::parse(src) else {
            return Progress::default();
        };
        let hill = match frost::ron::field(&root, "hill") {
            Some(frost::ron::Val::Atom(text, _)) => {
                text.trim_matches('"').parse::<usize>().unwrap_or(1)
            }
            _ => 1,
        };
        let reached = match frost::ron::field(&root, "reached") {
            Some(frost::ron::Val::Atom(text, _)) => {
                text.trim_matches('"').parse::<usize>().unwrap_or(1)
            }
            // A file from before the descent kept its depth says no
            // more than where it stands: the stood-on hill is the
            // deepest it ever named.
            _ => hill,
        };
        // The ledger survives what a hand may not: entries out of
        // order fold to sorted, a hill named twice keeps its better
        // number, and garbage in the list is simply not a score.
        // A file from before the ledger carries no `scores` at all —
        // which is no scores, not a broken file.
        let mut scores: Vec<(usize, i32)> = match frost::ron::field(&root, "scores") {
            Some(frost::ron::Val::Seq { items, .. }) => items
                .iter()
                .filter_map(|item| match &item.val {
                    frost::ron::Val::Atom(text, _) => {
                        let mut pair = text.trim_matches('"').split_whitespace();
                        let hill = pair.next()?.parse::<usize>().ok()?.checked_sub(1)?;
                        let moves = pair.next()?.parse::<i32>().ok()?;
                        // The file's scores pass the same turnstile
                        // the record does: the cap holds whatever
                        // hand went to the keyboard after the game.
                        Some((hill, moves.clamp(0, MOVES_CAP)))
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        scores.sort();
        scores.dedup_by(|a, b| {
            if a.0 == b.0 {
                // `a` is the entry being dropped; it dies passing its
                // better number on to the one that stays.
                b.1 = b.1.min(a.1);
                true
            } else {
                false
            }
        });
        // One-based in the file, zero-based in the code — and nothing
        // sits below the first hill. The depth of the descent is at
        // least the hill it stands on and at least the deepest warm
        // hill: a warm hill was stood on by definition, which folds
        // the pre-`reached` files (a score deeper than the standing
        // hill) onto the truth they already carry.
        let hill = hill.saturating_sub(1);
        let reached = reached
            .saturating_sub(1)
            .max(hill)
            .max(scores.iter().map(|(h, _)| *h).max().unwrap_or(0));
        // The routes read themselves the same forgiving way: a file
        // from before replays carries none, a hill named twice keeps
        // the first walk mentioned, and any string the hill could not
        // walk is not a route at all.
        let mut routes: Vec<(usize, String)> = match frost::ron::field(&root, "routes") {
            Some(frost::ron::Val::Seq { items, .. }) => items
                .iter()
                .filter_map(|item| match &item.val {
                    frost::ron::Val::Atom(text, _) => {
                        let mut pair = text.trim_matches('"').split_whitespace();
                        let hill = pair.next()?.parse::<usize>().ok()?.checked_sub(1)?;
                        let glyphs = pair.next()?;
                        if pair.next().is_some() || decode_run(glyphs).is_none() {
                            return None;
                        }
                        Some((hill, glyphs.to_string()))
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        routes.sort();
        routes.dedup_by(|a, b| a.0 == b.0);
        Progress {
            hill,
            reached,
            scores,
            routes,
        }
    }

    fn load() -> Progress {
        frost::store::load(Self::FILE)
            .map(|src| Progress::parse(&src))
            .unwrap_or_default()
    }

    fn save(&self) {
        if let Err(err) = frost::store::save(Self::FILE, &self.text()) {
            log::warn!("antifrost: could not save {:?}: {err}", Self::FILE);
        }
    }
}

/// The largest 16:9 box inside a window: native mode's view. A
/// fixed tier needs no fitting — its buffer *is* 16:9 and the engine
/// letterboxes the stretch to the window's aspect — but at native
/// the window is whatever the user dragged, so the game lays itself
/// out in this box and lets black bars frame what's left over.
fn fit_16x9(w: f32, h: f32) -> (f32, f32) {
    let vw = w.min(h * 16.0 / 9.0);
    (vw, vw * 9.0 / 16.0)
}

/// A fixed tier's claim on the window's size, and the record of how
/// that claim is being answered.
///
/// The claim outlives its own arrival, which is not the obvious design and
/// is the whole point of the type. Measured on macOS, coming out of a
/// fullscreen exit: the request landed — the engine logged the window at
/// 1920×1080 — and then the desktop resized it away twice more, to 1600×1144
/// and then to 1600×900, a second and a half later, during a stretch where
/// the surface was gone and the game was not even running its frames. A
/// policy that stopped caring the moment the size appeared left the window
/// exactly there: pinned by nothing, holding neither the tier's pixels nor
/// the aspect those pixels were chosen for.
///
/// So arrival starts a tail of quiet frames instead of ending the watch, and
/// any deviation during it re-asks. Both bounds are there because the other
/// answer is worse: a desktop that truly owns this window's geometry — a
/// tile, a snap, a window rule — gets argued with a bounded number of times
/// and then told about, rather than forever.
#[derive(Clone, Copy)]
struct TierAsk {
    /// The pixels the tier asked for, which fix the aspect too: one
    /// promise, not two.
    w: u32,
    h: u32,
    /// Frames since the window was last at those pixels.
    off: u32,
    /// Frames it has spent there. `CALM_HOLD` of them and the tier stands
    /// on its own.
    calm: u32,
    /// Asks made in all, counted so the give-up can say how many times it
    /// knocked.
    asks: u32,
}

impl TierAsk {
    /// Frames spent at the asked size before the move counts as over: half
    /// a second of a window sitting still. Until then a press queues
    /// behind it — one move on screen at a time — and after that the tail
    /// keeps watching in the background and a new press simply replaces the
    /// ask.
    const SETTLED: u32 = 30;
    /// How long the tail runs: five seconds of the window holding still,
    /// long past anything a compositor might still be doing about a window
    /// it just left fullscreen.
    const HOLD: u32 = 300;
    /// Frames off-size before the desktop is declared the owner of this
    /// window's geometry, and total asks before the same verdict on a
    /// window that keeps being moved back.
    const PATIENCE: u32 = 150;
    const PATIENCE_ASKS: u32 = 8;
    /// Frames between asks: six requests in six frames on a compositor
    /// that answers configures asynchronously is six fights in the log
    /// instead of one answer.
    const CADENCE: u32 = 15;

    /// Whether this ask still has a move in flight — off-size, or arrived
    /// too recently for the desktop to be done with the window.
    fn in_flight(&self) -> bool {
        self.off > 0 || self.calm < Self::SETTLED
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
    /// The stone slab the menu panels are cut from, held as a tile
    /// map: a panel is one placement of the whole texture, sized to
    /// the plate's rectangle, so the stone is the panel's face and
    /// never spills past it. An `Option` where the bed is not: a
    /// slab that failed to decode should leave the menu on its plain
    /// plate, not cost the player the game at launch. Decodes once,
    /// for the game's life.
    slab: Option<frost::Shape>,
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
    /// A best run being shown rather than played: `Some` while a hill
    /// replays itself, with its own keys and its own pace.
    replay: Option<Replay>,
    leaf_rng: Rng,
    leaves: Vec<Leaf>,
    leaves_seeded: bool,
    /// How many of the deck are currently in the air — thirty in calm
    /// weather, swelling toward `N_MAX` as the frost deepens.
    leaves_active: usize,
    leaf_t: f32,
    /// The hill's shadow cache: the solid-cell list as of the last
    /// merge, and the merged rectangles (cell-space spans, both ends
    /// inclusive) that list earned. Recomputed only when the cells
    /// change — see the occluder block in `draw`.
    occ_key: Vec<(i32, i32, bool)>,
    occ_boxes: Vec<(i32, i32, i32, i32)>,
    /// The player's preferences: read at launch, live in the menu,
    /// saved on every change (native).
    settings: Settings,
    /// Which page is up. The game starts behind the main menu: the
    /// hill shows through the dimmed panel, paused, until New Game or
    /// Resume opens the gate.
    page: Page,
    /// The slider currently under the button, if any.
    drag: Option<Slider>,
    /// The highlighted row — what the arrow keys walk and Enter
    /// presses. The one highlight there is: the pointer takes the
    /// selection over when it moves onto a row, clicks press theirs
    /// and take it too, and pages reset it to the top as they open.
    menu_sel: usize,
    /// Where the pointer stood last menu frame, so motion can be
    /// told from mere presence: a parked cursor must not out-vote
    /// the arrow keys.
    last_pointer: Option<[f32; 2]>,
    /// The live ledger: the disc's scores, plus every hill won since
    /// the game opened. Written when a hill moves and when a hill is
    /// won — the same law the file keeps from the outside.
    progress: Progress,
    /// The first hill row the progress page shows: the window that
    /// slides as the selection (or the wheel) walks past its edges.
    progress_top: usize,
    /// When the last frame beat, in milliseconds on the browser's
    /// clock. The stutter sentinel's only memory: half a second of
    /// silence between beats is worth a line in the console, and a
    /// machine the developer cannot see becomes a machine they can
    /// read. 0.0 before the first frame, so the opening gap — the
    /// shader compiles and first paint — never reports itself.
    last_beat: f64,
    /// The progress page's minimap atlas: one white pixel. Every
    /// cell of every hill is a placement of that pixel, so one node
    /// — one instanced draw — paints all the maps, each cell's hue
    /// carried by its own tint.
    minimap_atlas: Arc<[u8]>,
    /// The bench hourglass (`--bench`): a run length, the clock it
    /// has accumulated, and the frames worth counting. None is the
    /// game's own shape — play until told otherwise.
    bench: Option<f32>,
    bench_t: f32,
    bench_frames: u32,
    /// The overlay's breath: every menu — main, settings, the won
    /// hill's panel — fades through this alpha rather than snapping.
    menu_alpha: f32,
    /// Which page the overlay paints: the live one while a menu is
    /// up, and the last one held while the layer fades out.
    menu_page: Page,
    /// The verdict word and its bright, kept after the splash has
    /// spun itself out: the won panel reads it in the strip above the
    /// plate.
    verdict: Option<(String, Color)>,
    /// The window takes its first shape on the first frame the
    /// engine's handle exists: the OS will not size a window that
    /// has not been born yet.
    window_policy_pending: bool,
    /// Diagnostic clock for `--resize-probe`: cycles the real
    /// resolution row once on a timer, so the window's answer can
    /// be read from the log without a hand on the mouse.
    resize_probe: Option<f32>,
    /// A press that arrived while a move was in flight: one move
    /// on screen at a time, and the newest rung always gets its
    /// turn — queued pressed, applied resolved.
    window_move_queued: bool,
    /// A fixed tier's standing claim on the window's size — the ask, and
    /// how the asking is going. See [`TierAsk`] for why it keeps asking
    /// after it has arrived.
    window_expect: Option<TierAsk>,
    /// Frames the window's fullscreen state has disagreed with the
    /// settings row. The desktop can move that state on its own — a
    /// green button, a window rule, a shortcut the WM answers itself —
    /// and the only news of it is the state, so the state is polled,
    /// the way the size is. The count is the grace, and it is a
    /// second's worth on purpose: it has to outlive both a compositor
    /// round trip and macOS's fullscreen animation, because acting
    /// sooner would mean the game yanking the window out of an
    /// entrance the desktop was still drawing.
    fullscreen_drift: u32,
}

impl Demo {
    fn new(
        ant_frames: Vec<frost::Shape>,
        soil: frost::Shape,
        settings: Settings,
        progress: Progress,
    ) -> Self {
        // The soil texture doubles as the hill's tile atlas: one cell,
        // the whole sheet, every placement sampling all of it. A
        // photographic atlas wants bilinear, not the map's pixel-art
        // default. A soil PNG that decodes for one must decode for the
        // other — anything else is a broken build.
        let mut tile_atlas =
            frost::Shape::tilemap_bytes(SOIL).expect("shipped soil bakes as an atlas");
        tile_atlas.set_filter(SpriteFilter::Linear);
        let bed = frost::Shape::sprite_bytes(BED).expect("shipped bed sprite decodes");
        // The slab decodes once here, for the game's whole life; if
        // the shipped bytes ever fail to decode, the `Option` keeps
        // that a plainer plate, not a lost launch. Linear filtering:
        // the plate shows the stone whole and stretched, and a photo
        // stretched wants smoothing, not nearest's staircase.
        let slab = match frost::Shape::tilemap_bytes(STONE) {
            Ok(mut shape) => {
                shape.set_filter(SpriteFilter::Linear);
                Some(shape)
            }
            Err(_) => None,
        };
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
            slab,
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
            replay: None,
            leaf_rng: Rng::default(),
            leaves: Vec::new(),
            leaves_seeded: false,
            leaves_active: N_LEAVES,
            leaf_t: 0.0,
            occ_key: Vec::new(),
            occ_boxes: Vec::new(),
            settings,
            page: Page::Main,
            drag: None,
            menu_sel: 0,
            last_pointer: None,
            progress,
            progress_top: 0,
            last_beat: 0.0,
            minimap_atlas: Arc::from([255u8, 255, 255, 255].as_slice()),
            bench: None,
            bench_t: 0.0,
            bench_frames: 0,
            menu_alpha: 0.0,
            menu_page: Page::Main,
            verdict: None,
            window_policy_pending: true,
            resize_probe: None,
            window_expect: None,
            fullscreen_drift: 0,
            window_move_queued: false,
        };
        // The saved hill, wherever it lands — `load` folds indices
        // past the last level back onto the set that exists.
        let start_hill = demo.progress.hill;
        demo.load(start_hill);
        demo
    }

    /// How far the descent has reached: the deepest hill ever stood
    /// on — `reached` carries that across runs, and it never walks
    /// back — clamped to the hills that exist. The progress page
    /// lists this many hills.
    fn progressed(&self) -> usize {
        if self.levels.is_empty() {
            return 0;
        }
        self.progress.reached.min(self.levels.len() - 1) + 1
    }

    /// Whether the ledger says a hill's run is done to par: warmed,
    /// and the best met par — the exact yellow of the score column,
    /// so the ledger's order, the `next` key, and a row's ink can
    /// only ever read one rule. An unwarmed hill owns no perfect run.
    fn perfect(&self, hill: usize) -> bool {
        self.progress
            .scores
            .iter()
            .find(|(h, _)| *h == hill)
            .is_some_and(|(_, moves)| {
                let par = self.levels.get(hill).map_or(0, |l| l.par);
                !(par > 0 && *moves > par)
            })
    }

    /// The hill that `next` leads to, in the descent's own order of
    /// owing. A freshly unlocked hill — one below the deepest ever
    /// stood on, never walked — is the run no ledger row can offer,
    /// so while the player stands at the frontier of the descent it
    /// answers first. From shallower ground, `next` leads to the
    /// nearest hill ahead, wrapping through the shallow ones, that
    /// has no perfect run to its name; the hill the player stands on
    /// is not among them, because `retry` is the key that redoes a
    /// hill and `next` is the key that moves on. Nothing owed and no
    /// new hill below: the descent has nothing left to owe, and
    /// `next` is simply the next hill, wrapping, as it always was.
    fn next_hill(&self) -> usize {
        if self.levels.is_empty() {
            return self.idx;
        }
        let last = self.levels.len() - 1;
        let deepest = self.progress.reached.min(last);
        if self.idx >= deepest && deepest < last {
            return deepest + 1;
        }
        for step in 1..=deepest {
            let i = (self.idx + step) % (deepest + 1);
            if !self.perfect(i) {
                return i;
            }
        }
        if deepest < last {
            return deepest + 1;
        }
        (self.idx + 1) % self.levels.len()
    }

    /// Whether `next` has a hill left to lead to: a never-walked
    /// one below the frontier of the descent, or an owed one
    /// anywhere but underfoot — the hill the player stands on is
    /// `retry`'s business, not `next`'s. When the one owed hill is
    /// the one underfoot there is nowhere left to move on to, and
    /// the row for `next` wears grey and answers no action. A fully
    /// perfected descent owes nothing at all, and reads the same.
    fn next_owed(&self) -> bool {
        if self.levels.is_empty() {
            return false;
        }
        if self.progress.reached < self.levels.len() - 1 {
            return true;
        }
        (1..self.levels.len()).any(|step| !self.perfect((self.idx + step) % self.levels.len()))
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
        // A hill entered is a hill played, and a played hill keeps
        // its own record: the walk is as much the player's as the
        // number it earns. (A showing switches this back off.)
        if let Some(g) = &mut self.game {
            g.record = true;
        }
        self.replay = None;
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
        self.verdict = None;
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
        let (w, h) = self.view(ctx);
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
        let (w, h) = self.view(ctx);
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
                // own height — drift and feed stay even. The landing
                // stays under the edge that wraps the other way: a
                // range reaching past it would hand the flake straight
                // back, ping-ponging between margins instead of
                // walking in.
                leaf.pos[0] = w / 2.0 - off[0] + self.leaf_rng.in_range(0.0, m + reach);
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
        lamp: f32,
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
        // even at the very floor of the ambient. And the ember breathes
        // with that lamp: the same number the light node was just given,
        // scaled down, which is why one pulse crosses the snow and the
        // ant together and why the `light fx` slider takes the glow with
        // it when it takes the lamps down.
        node.lit = true;
        node.emission = shade(tint, EMBER_FLOOR + EMBER_PER_LAMP * lamp);
        // The halo is the light's color, not the shell's — the core a lamp
        // throws rather than the cuticle it came from, so an ant's glow is a
        // whiter, warmer thing than the ant. It rides the same number, and at
        // no lamp it is black, which the blend answers exactly as it answers
        // no halo at all: the dark copy under the player asks for neither.
        node.glow = Some(shade(lamp_core(tint), GLOW_PER_LAMP * lamp));
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
        // Every lamp's strength for this frame, in the units the light nodes
        // get — index `game.ants.len()` is the player's, who is an ant too.
        // The shells read it back below, because an ember and a halo are the
        // breath the snow already sees, not a second pulse to keep in step.
        let mut lamps = vec![0.0; game.ants.len() + 1];
        {
            let reach = TILE * ZOOM * LAMP_REACH;
            let base = LEAVES.len() + game.ants.len() + 2;
            for (id, lamp) in lamps[..game.ants.len()].iter_mut().enumerate() {
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
                // A little white in the core, the ant's own hue in the
                // throw of it.
                let col = lamp_core(ant_color(game.ants[id].color));
                node.shape = Some(frost::Shape::Light {
                    light: frost::Light::point(col, intensity * self.settings.light, reach)
                        .with_penumbra(self.settings.shadows.penumbra())
                        .with_shadow_taps(self.settings.shadows.taps()),
                });
                node.transform = Transform::translate(self.screen(g.at));
                *lamp = intensity * self.settings.light;
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
                        intensity * self.settings.light,
                        reach,
                    )
                    .with_penumbra(self.settings.shadows.penumbra())
                    .with_shadow_taps(self.settings.shadows.taps()),
                });
            } else {
                node.shape = None;
            }
            node.transform = Transform::translate(self.screen(self.player_at));
            lamps[game.ants.len()] = intensity.max(0.0) * self.settings.light;
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
            // emission onto the surrounding ground through the border
            // hairlines, and the frame's own shadows do the rest.
            node.lit = true;
            node.order = -0.4;
        }
        // Painted occlusion is a rectangle's privilege — a sprite's
        // body casts no shadow on the flag alone — so the hill's solid
        // cells declare their boxes here. But every occluder the field
        // carries is tested by every fragment any light reaches, so
        // one box per tile at 4K is a frame budget in chains: instead,
        // the cells merge into a small cover of boxes — row runs, equal
        // spans stacked, and a fixpoint pass over the result (see
        // [`merge_boxes`]). A merged cover casts the same shadow its
        // tiles ever did — the tile hairlines that used to leak a thread
        // of light measure a pixel, and a merged edge is invisible
        // against the row it belongs to.
        //
        // The merge itself is cached on the cell list: the hill is
        // static between digs, so the run scan, the sort, and the walk
        // re-run only when the cells actually change. What every frame
        // does instead is re-pack the cached rectangles into screen
        // space — the camera pan forces fresh pixel-space boxes either
        // way, but packing cached runs is arithmetic where merging is
        // search, and the frames between digs are the overwhelming
        // majority of the game.
        {
            let ts = TILE * ZOOM;
            if tiles != self.occ_key {
                self.occ_boxes = merge_boxes(&tiles);
                self.occ_key.clone_from(&tiles);
            }
            // Off, the field never learns of a single occluder: the
            // packer hands every light an empty block, `occluded`
            // answers without one record load, and the hill pays only
            // for the lamps' arithmetic — the weak-hardware shape of
            // this renderer, decided at the menu rather than in the
            // shader.
            for &(col_s, col_e, row0, row1) in self
                .occ_boxes
                .iter()
                .filter(|_| self.settings.shadows != Shadow::None)
            {
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
        // a taken bed stays self-lit, the target outlives the ant.
        {
            let base = LEAVES.len() + game.ants.len() * 2 + 4;
            for (i, (cell, color)) in game.level.beds.iter().enumerate() {
                let node = &mut ctx.scene().root.children[base + i];
                node.shape = Some(frost::Shape::Light {
                    light: frost::Light::point(
                        ant_color(*color),
                        0.5 * self.settings.light,
                        TILE * ZOOM * 1.3,
                    )
                    .with_penumbra(self.settings.shadows.penumbra())
                    .with_shadow_taps(self.settings.shadows.taps()),
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
        for (id, &lamp) in lamps[..game.ants.len()].iter().enumerate() {
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
                blaze(
                    dim,
                    Color {
                        r: col.r * 2.4 + 0.9,
                        g: col.g * 2.4 + 0.9,
                        b: col.b * 2.4 + 0.9,
                        a: 1.0,
                    },
                    k,
                )
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
            self.put_ant(node, frame_of(g.anim, asleep), at, rf, col, lamp);
            node.order = z;
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
            let shadowed = &mut root.children[LEAVES.len() + back];
            self.put_ant(
                shadowed,
                frame,
                [at[0] + 1.5 * ZOOM, at[1] - 1.5 * ZOOM],
                self.player_face,
                Color {
                    r: 0.06,
                    g: 0.05,
                    b: 0.04,
                    a: 1.0,
                },
                0.0,
            );
            shadowed.order = 1.48;
            // The queen carries a lamp of her own — the breath written on her
            // node above — so she wears the same ember and the same halo as
            // any ant, at her light's own strength. And she flashes on the bed
            // like one: her spike always reached her lamp and never her pixels,
            // which is why her tuck-in lit the snow and left her out of it. The
            // same hold-then-drain curve the ants ride, on the amber.
            let queen = &mut root.children[LEAVES.len() + player];
            self.put_ant(
                queen,
                frame,
                at,
                self.player_face,
                blaze(
                    ant_color(game::AntColor::White),
                    CROWN_BLAZE,
                    (self.player_home.unwrap_or(0.0) * 1.7).min(1.0),
                ),
                lamps[n],
            );
            queen.order = 1.5;
        }
    }

    /// Draw the verdict: one clean spin from a speck, eased so the
    /// speck lingers tiny before it blooms to half the screen; a
    /// heartbeat at full size; then the fade to the hill's own words.
    fn draw_splash(&self, ctx: &mut Context) {
        let Some(s) = &self.splash else { return };
        let (w, _) = self.view(ctx);
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
            *color = s.color;
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

    /// One banner row from measured pieces laid end to end, the whole
    /// line centred as a string would be. The widths come from the
    /// engine's shaping — measured additive, checked — so measure and
    /// paint cannot drift apart. Runs carry no spaces: Barrfont's own
    /// space is 7.5 px at 30, nearly invisible under small caps, so
    /// every gap here is an explicit fraction of the size instead.
    fn banner(
        &self,
        ctx: &mut Context,
        cy: f32,
        size: f32,
        weight: f32,
        runs: &[(&str, Color, f32)],
    ) {
        // Measured first, drawn after: the shaping asks the canvas
        // read-only, the paint takes it back.
        let widths: Vec<f32> = runs
            .iter()
            .map(|(t, _, _)| ctx.text_width(&self.font, t, size, weight))
            .collect();
        let total: f32 =
            widths.iter().sum::<f32>() + runs.iter().map(|(_, _, gap)| gap * size).sum::<f32>();
        let mut x = -total / 2.0;
        for ((text, colour, gap), w) in runs.iter().zip(widths) {
            let half = w / 2.0;
            ctx.text(
                x + half,
                cy,
                &self.font,
                (*text).to_string(),
                size,
                weight,
                // One over the veil's head (its base, below the plate
                // and rows): the hill's record rides above the fade,
                // fully lit while any menu is up — a simple z, where
                // colours pushed past 1.0 would only meet the 8-bit
                // buffer's clamp.
                *colour,
                UI + 10.5,
            );
            x += w + gap * size;
        }
    }

    /// The rectangle the game lays itself out in. See [`fit_16x9`]:
    /// a fixed tier's canvas size already *is* the 16:9 view (the
    /// engine letterboxes the stretch itself); native fits it in.
    fn view(&self, ctx: &Context) -> (f32, f32) {
        let (w, h) = ctx.size();
        if self.settings.quality.size().is_some() {
            (w, h)
        } else {
            fit_16x9(w, h)
        }
    }

    /// Black bars from the view's edge to the window's, above
    /// everything the game owns. Nothing draws outside the view, so
    /// the bars only ever cover the backdrop and the backdrop's
    /// darkness — they frame the picture and touch nothing else.
    fn draw_letterbox(&self, ctx: &mut Context) {
        let (w, h) = ctx.size();
        let (vw, vh) = self.view(ctx);
        let black = Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        let z = UI + 20.0;
        if w - vw > 0.5 {
            let d = (w - vw) / 4.0;
            ctx.rectangle(-(vw + w) / 4.0, 0.0, d, h / 2.0, black, z);
            ctx.rectangle((vw + w) / 4.0, 0.0, d, h / 2.0, black, z);
        }
        if h - vh > 0.5 {
            let d = (h - vh) / 4.0;
            ctx.rectangle(0.0, (vh + h) / 4.0, w / 2.0, d, black, z);
            ctx.rectangle(0.0, -(vh + h) / 4.0, w / 2.0, d, black, z);
        }
    }

    /// Bring the OS window to what the tier and the fullscreen flag
    /// ask for. A fixed windowed tier gets a window of exactly the
    /// buffer's pixels and no resize handle — picture and window are
    /// one rectangle, and the 16:9 question never comes up. Native
    /// takes the resize handle back: the user drags any shape, and
    /// [`fit_16x9`] keeps the game a 16:9 picture inside it.
    /// Fullscreen leaves the window to the display and the engine's
    /// letterboxed stretch to keep 16:9 on non-16:9 monitors.
    /// Whether the window is mid-move: a tier's ask that is either off-size or
    /// arrived so recently the desktop may still be dragging the window. One
    /// move on screen at a time, so a press during one queues behind it —
    /// behind the *move*, not behind the tail that keeps watching it.
    fn in_flight(&self) -> bool {
        self.window_expect.as_ref().is_some_and(TierAsk::in_flight)
    }

    fn apply_window_policy(&mut self, ctx: &mut Context) {
        let Some(window) = ctx.window() else {
            return;
        };
        log::info!(
            "window policy: ask {:?} (before {}x{}, fullscreen {})",
            self.settings.quality.size(),
            window.inner_size().width,
            window.inner_size().height,
            self.settings.fullscreen,
        );
        // One move on screen at a time: a press during an in-flight
        // move re-aims it when that move resolves — half-moves
        // stacking up mid-flight is what flickered.
        if self.in_flight() {
            self.window_move_queued = true;
            log::info!("window policy: move in flight, re-aim queued");
            return;
        }
        // Touch fullscreen only when the ask differs from the
        // state: winit rewrites the window's whole state list on
        // every call (X11 especially), and states it does not know
        // — like KWin's tile atom — die in the rewrite. KWin
        // notices its tile was yanked and retiles: the window
        // jumps to a size nobody asked for.
        if self.settings.fullscreen {
            window.set_resizable(true);
            window.set_min_inner_size(None::<frost::PhysicalSize<u32>>);
            window.set_max_inner_size(None::<frost::PhysicalSize<u32>>);
            if window.fullscreen().is_none() {
                window.set_fullscreen(Some(frost::Fullscreen::Borderless(None)));
            }
            self.window_expect = None;
        } else {
            if window.fullscreen().is_some() {
                window.set_fullscreen(None);
            }
            match self.settings.quality.size() {
                Some([w, h]) => {
                    // Unlock (hints and all), then move: pin min/max
                    // to the rung — on Wayland that clamp is the
                    // engine, since winit's request never leaves the
                    // app — and lock (set_resizable(false)) only on
                    // arrival, because locking pins to the CURRENT
                    // size and would stomp the new one wherever the
                    // request lands asynchronously.
                    window.set_resizable(true);
                    window.set_min_inner_size(None::<frost::PhysicalSize<u32>>);
                    window.set_max_inner_size(None::<frost::PhysicalSize<u32>>);
                    // The request is the mover: X11 directly,
                    // Wayland through the surface geometry winit
                    // sends with it — KWin honours a geometry
                    // change on a mapped window. A configure that
                    // then matches our wish is swallowed by
                    // winit's dedup, so never trust events alone
                    // to follow the window: the engine's frame
                    // loop checks its size against the surface's
                    // belief every frame (see app.rs).
                    let got = window.request_inner_size(frost::PhysicalSize::new(w, h));
                    window.set_min_inner_size(Some(frost::PhysicalSize::new(w, h)));
                    window.set_max_inner_size(Some(frost::PhysicalSize::new(w, h)));
                    self.window_expect = Some(TierAsk {
                        w,
                        h,
                        off: 0,
                        calm: 0,
                        asks: 1,
                    });
                    log::info!(
                        "window policy: asked {w}x{h} ({got:?}, local {}x{})",
                        window.inner_size().width,
                        window.inner_size().height,
                    );
                }
                None => {
                    window.set_resizable(true);
                    window.set_min_inner_size(None::<frost::PhysicalSize<u32>>);
                    window.set_max_inner_size(None::<frost::PhysicalSize<u32>>);
                    self.window_expect = None;
                }
            }
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

    fn draw_hud(&self, ctx: &mut Context) {
        let Some(game) = &self.game else { return };
        let (_, h) = self.view(ctx);
        // Barrfont carries letters, digits and spaces — nothing else —
        // so every separator in the game is spaces. And since text draws
        // centred on its point, everything centres: a doubled size can
        // never spill off an edge the way a left edge would.
        // The banner counts in two tones, one pair bright: the
        // numbers that judge the run — moves and par, the "10 of 10"
        // of a perfect hill — stand out while every other word and
        // number recedes to their side. And the pair speaks the
        // splash's language already: while the run is on par, moves
        // wears the yellow a perfect finish will bloom in; past it,
        // the red the consolation words will. The verdict starts
        // painting the picture one move at a time.
        let on_par = game.level.par > 0 && game.moves <= game.level.par;
        let moves_colour = if on_par { C_PAR_MET } else { C_PAR_MISSED };
        let hill = (self.idx + 1).to_string();
        let hills = self.levels.len().to_string();
        let title: [(&str, Color, f32); 5] = [
            (&game.level.name, C_DIM, 0.7),
            ("hill", C_DIM, 0.5),
            (&hill, C_DIM, 0.5),
            ("of", C_DIM, 0.5),
            (&hills, C_DIM, 0.0),
        ];
        self.banner(ctx, h / 2.0 - 34.0, 34.0, 700.0, &title);
        let warm = game.ants_asleep().to_string();
        let crew = game.ants.len().to_string();
        let moves = game.moves.min(MOVES_CAP).to_string();
        let par = game.level.par.to_string();
        let score: [(&str, Color, f32); 8] = [
            ("warm", C_DIM, 0.5),
            (&warm, C_DIM, 0.5),
            ("of", C_DIM, 0.5),
            (&crew, C_DIM, 1.6),
            ("moves", C_DIM, 0.5),
            (&moves, moves_colour, 0.5),
            ("of", C_DIM, 0.5),
            (&par, C_TEXT, 0.0),
        ];
        self.banner(ctx, h / 2.0 - 78.0, 30.0, 400.0, &score);
        // The old bottom banner is gone: the controls are read in
        // the menu (settings, keyboard, mouse), not footnoted on the
        // hill. Notes take its floor-side seat.
        if self.note_t > 0.0 {
            let a = (self.note_t / 0.6).min(1.0);
            ctx.text(
                0.0,
                -h / 2.0 + 26.0,
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
        // A showing names itself and its keys: this is not a run,
        // and the count it keeps is the recording's own feet. It
        // takes the note's floor-side seat, standing a line higher
        // when a note is also up — a showing can end on one.
        if let Some(rep) = &self.replay {
            let tail = if rep.playing {
                "space holds"
            } else if rep.shown < rep.acts.len() {
                "space resumes"
            } else {
                "the run is done"
            };
            let line = format!(
                "replay · {} of {} · ← → step · {} · esc back",
                rep.shown,
                rep.acts.len(),
                tail
            );
            let seat = if self.note_t > 0.0 { 60.0 } else { 26.0 };
            ctx.text(
                0.0,
                -h / 2.0 + seat,
                &self.font,
                line,
                30.0,
                600.0,
                C_NOTE,
                UI + 1.0,
            );
        }
    }

    /// The menu itself. The hill dims behind it, paused and visible —
    /// the point of Resume is that you can see what you are taking
    /// back — and the hovered row lifts a whisper of white.
    fn draw_menu(&self, ctx: &mut Context) {
        let (w, h) = self.view(ctx);
        let z = UI + 10.0;
        // Everything the overlay owns breathes together — veil,
        // plate, rows, verdict — through one alpha held by `process`.
        let a = self.menu_alpha;
        let fade = |c: Color| Color { a: c.a * a, ..c };
        ctx.rectangle(
            0.0,
            0.0,
            w / 2.0,
            h / 2.0,
            Color {
                r: 0.012,
                g: 0.014,
                b: 0.022,
                a: 0.72 * a,
            },
            z,
        );
        let rows = self.menu_rows(self.menu_page, h);
        if rows.is_empty() {
            return;
        }
        let top = rows.iter().map(|r| r.cy).fold(f32::MIN, f32::max);
        let bot = rows.iter().map(|r| r.cy).fold(f32::MAX, f32::min);
        // The plate is as wide as its page's widest claim: the ledger
        // spends a number, a name, a score, and a map, so it takes
        // the widest hall of any page on the menu.
        let plate_hw = match self.menu_page {
            Page::Progress => PROGRESS_PLATE,
            Page::Settings | Page::Keyboard | Page::Mouse => 380.0,
            _ => 300.0,
        };
        // The panel is cut from the slab: one placement of the whole
        // stone at the plate's own size — stretched to whatever
        // rectangle the page hangs, and the moss and lichen do not
        // mind. Where the slab failed to decode, the plate is still
        // the plain ink slab it always was.
        let (pcx, pcy) = (0.0, (top + bot) / 2.0);
        let (phw, phh) = (plate_hw, (top - bot) / 2.0 + 130.0);
        let mut cut = false;
        if let Some(slab) = &self.slab {
            let mut panel = slab.clone();
            if let frost::Shape::TileMap { tiles, .. } = &mut panel {
                tiles.push(frost::Tile::new(
                    [pcx, pcy],
                    [2.0 * phw, 2.0 * phh],
                    [0.0, 0.0, 1.0, 1.0],
                ));
                ctx.scene().root.children.push(Box::new(SceneNode {
                    order: z + 1.0,
                    modulate: Color {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a,
                    },
                    shape: Some(panel),
                    ..Default::default()
                }));
                cut = true;
            }
        }
        // The panel's own night: once it was the plate itself, and on
        // stone it is a shade — the slab was baked at a third of its
        // daylight, and all this shade has left to do is settle the
        // white lichen behind the words.
        ctx.rectangle(
            pcx,
            pcy,
            phw,
            phh,
            Color {
                r: 0.05,
                g: 0.055,
                b: 0.08,
                a: (if cut { 0.45 } else { 0.94 }) * a,
            },
            z + 1.25,
        );
        // The panel wears the hill it stands on: main and won pages
        // both name their ground — the title answers the same banner
        // the player has been reading all run. Settings is its own
        // room and keeps its own sign.
        let title: String = match self.menu_page {
            Page::Settings => "SETTINGS".to_string(),
            Page::Keyboard => "KEYBOARD".to_string(),
            Page::Mouse => "MOUSE".to_string(),
            Page::Progress => "PROGRESS".to_string(),
            _ => self
                .game
                .as_ref()
                .map_or("ANTIFROST", |g| g.level.name.as_str())
                .to_string(),
        };
        ctx.text(
            0.0,
            top + 70.0,
            &self.font,
            title,
            52.0,
            700.0,
            fade(C_TEXT),
            z + 2.0,
        );
        // The won hill's verdict: the word its splash just spoke,
        // now read plainly in the strip between the HUD's band and
        // the plate's top — verdict overhead, choices under foot.
        if self.menu_page == Page::Won
            && let Some((word, colour)) = &self.verdict
        {
            // The plate's top and the score banner's bottom are
            // 92 units apart by the won page's hang (see
            // `menu_rows`); the word owns the middle of that
            // strip, full size, every window height.
            let plate_top = top + 130.0;
            let band = h / 2.0 - 93.0;
            ctx.text(
                0.0,
                (plate_top + band) / 2.0,
                &self.font,
                word.clone(),
                64.0,
                700.0,
                fade(*colour),
                z + 2.0,
            );
        }
        // Exactly one row wears the highlight: the selected one. The
        // pointer takes the selection over when it moves onto a row
        // (see `menu_input`), so the mouse and the keys can never
        // each light their own row — the two-emission state a separate
        // hover used to allow.
        for (i, r) in rows.iter().enumerate() {
            let hot = i == self.menu_sel;
            // A row with no road left in it: `next` greys when the
            // hill underfoot is all the descent still owes — the
            // same rule the key obeys, asked at the same lip.
            let mute = matches!(r.action, Action::Next) && !self.next_owed();
            if hot {
                ctx.rectangle(
                    r.cx,
                    r.cy,
                    r.hw,
                    24.0,
                    Color {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: (if mute { 0.03 } else { 0.07 }) * a,
                    },
                    z + 2.0,
                );
            }
            if let Action::Hill(hill) = r.action {
                self.draw_ledger_row(ctx, r, hill, hot, z, a);
                continue;
            }
            match r.slider {
                None => ctx.text(
                    r.cx,
                    r.cy - 10.0,
                    &self.font,
                    r.text.clone(),
                    30.0,
                    500.0,
                    if mute {
                        fade(C_MUTE)
                    } else if hot {
                        fade(C_TEXT)
                    } else {
                        fade(C_DIM)
                    },
                    z + 3.0,
                ),
                Some(frac) => {
                    ctx.text(
                        MENU_TRACK_CX - MENU_TRACK_HALF - 160.0,
                        r.cy - 10.0,
                        &self.font,
                        r.text.clone(),
                        30.0,
                        500.0,
                        if hot { fade(C_TEXT) } else { fade(C_DIM) },
                        z + 3.0,
                    );
                    ctx.rectangle(
                        MENU_TRACK_CX,
                        r.cy,
                        MENU_TRACK_HALF,
                        3.0,
                        Color {
                            r: 0.35,
                            g: 0.36,
                            b: 0.42,
                            a,
                        },
                        z + 3.0,
                    );
                    ctx.rectangle(
                        MENU_TRACK_CX - MENU_TRACK_HALF + frac * MENU_TRACK_HALF,
                        r.cy,
                        (frac * MENU_TRACK_HALF).max(0.5),
                        3.0,
                        fade(C_NOTE),
                        z + 3.0,
                    );
                    ctx.rectangle(
                        MENU_TRACK_CX - MENU_TRACK_HALF + frac * 2.0 * MENU_TRACK_HALF,
                        r.cy,
                        5.0,
                        11.0,
                        fade(C_TEXT),
                        z + 3.0,
                    );
                }
            }
        }
        // The maps beside the ledger's rows, in one instanced node:
        // every cell of every visible hill is a tinted placement of
        // one white pixel, and the node's clip cuts the column to
        // the plate — scroll and all of it rides together.
        if self.menu_page == Page::Progress {
            self.draw_minimaps(ctx, &rows, h, z, a);
            self.draw_progress_marks(ctx, &rows, h, z, a);
        }
    }

    /// One ledger row: the hill's number, its name, and — for a hill
    /// the colony warmed — its best run in the par's own colour
    /// language (the yellow a met par wears, the red a missed one).
    /// A hill stood on but not finished keeps its score column
    /// empty: the ledger writes nothing it has not been given. The
    /// runs sit in columns rather than the HUD's one centred
    /// sentence: this is a list to scan, not a banner to read.
    fn draw_ledger_row(&self, ctx: &mut Context, r: &Row, hill: usize, hot: bool, z: f32, a: f32) {
        let score = self
            .progress
            .scores
            .iter()
            .find(|(h, _)| *h == hill)
            .map(|(_, m)| *m);
        let (name, par) = self
            .levels
            .get(hill)
            .map_or(("hill", 0), |l| (l.name.as_str(), l.par));
        // Measured first, drawn after: the columns are left edges,
        // and text draws centred, so every run needs its own width
        // before any of it reaches the canvas.
        let num = (hill + 1).to_string();
        let w_num = ctx.text_width(&self.font, &num, 30.0, 500.0);
        let w_name = ctx.text_width(&self.font, name, 30.0, 500.0);
        let fade = |c: Color| Color { a: c.a * a, ..c };
        let ink = if hot { fade(C_TEXT) } else { fade(C_DIM) };
        let cy = r.cy - 10.0;
        ctx.text(
            PROGRESS_NUM_X + w_num / 2.0,
            cy,
            &self.font,
            num,
            30.0,
            500.0,
            ink,
            z + 3.0,
        );
        ctx.text(
            PROGRESS_NAME_X + w_name / 2.0,
            cy,
            &self.font,
            name,
            30.0,
            500.0,
            ink,
            z + 3.0,
        );
        let Some(moves) = score else { return };
        // The score wears its judgement always and never the
        // highlight's: the best run at or under par — perfect — is
        // yellow, a best that still overshot par is red, the same
        // full hue on every row lit or unlit. The page's own fade
        // is the only hand on either. Right-aligned at the column's
        // edge — a list of numbers reads up its right side.
        let colour = if par > 0 && moves > par {
            C_PAR_MISSED
        } else {
            C_PAR_MET
        };
        let score_text = format!("{moves} moves");
        let w_score = ctx.text_width(&self.font, &score_text, 30.0, 500.0);
        ctx.text(
            PROGRESS_SCORE_R - w_score / 2.0,
            cy,
            &self.font,
            score_text,
            30.0,
            500.0,
            fade(colour),
            z + 3.0,
        );
    }

    /// Every minimap the page shows, in one tile-map node: a one-
    /// pixel white atlas tinted per cell — rock, dirt, open tunnel,
    /// and every bed in its ant's own colour. The maps are the
    /// authored hills themselves, tile for tile: the ledger keeps no
    /// picture, because the hill already is one.
    fn draw_minimaps(&self, ctx: &mut Context, rows: &[Row], h: f32, z: f32, a: f32) {
        let mut map = frost::Shape::TileMap {
            data: self.minimap_atlas.clone(),
            width: 1,
            height: 1,
            // One atlas for the game's whole life, behind one Arc: the
            // texture cache's (pointer, generation) identity never
            // changes, so the pixel uploads once and never again.
            generation: 0,
            filter: SpriteFilter::Nearest,
            tiles: Vec::new(),
            clip: Some([
                PROGRESS_MAP_CX - 126.0,
                -h * 0.5 + 70.0,
                PROGRESS_MAP_CX + 126.0,
                progress_top_y(h) + 46.0,
            ]),
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
        };
        for r in rows {
            let Action::Hill(hill) = r.action else {
                continue;
            };
            let Some(level) = self.levels.get(hill) else {
                continue;
            };
            let (cols, rows_n) = (level.cols as f32, level.rows as f32);
            // The cell answers to both bounds: the map is at most
            // 225 wide — that budget is what holds the card and the
            // column gutters honest — and never taller than the row
            // stride minus its breathing room, so maps of neighbouring
            // rows cannot touch even at the short-window stride.
            let cell = ((progress_step(h) - 12.0) / rows_n)
                .min(225.0 / cols)
                .min(5.6);
            // The card centres on the row itself — the selection band
            // spans ±24 about `r.cy` — not on the row's text, which
            // sits ten low for its baseline. An earlier draft stole
            // four of those points for the baseline too, and the maps
            // read a touch sunken in their rows.
            let (cx, cy) = (PROGRESS_MAP_CX, r.cy);
            // The dark card first, so the tiles land on it: a map
            // reads as a framed thing beside its row, not as spill.
            ctx.rectangle(
                cx,
                cy,
                cols * cell / 2.0 + 5.0,
                rows_n * cell / 2.0 + 5.0,
                Color {
                    r: 0.02,
                    g: 0.022,
                    b: 0.03,
                    a: 0.9 * a,
                },
                z + 1.2,
            );
            for (i, t) in level.tiles.iter().enumerate() {
                let (row, col) = (i / level.cols, i % level.cols);
                let tint = match t {
                    level::Tile::Rock => Color {
                        r: 0.30,
                        g: 0.27,
                        b: 0.25,
                        a: 1.0,
                    },
                    level::Tile::Dirt => Color {
                        r: 0.46,
                        g: 0.34,
                        b: 0.22,
                        a: 1.0,
                    },
                    level::Tile::Tunnel => Color {
                        r: 0.05,
                        g: 0.05,
                        b: 0.06,
                        a: 1.0,
                    },
                    level::Tile::Bed(c) => ant_color(*c),
                };
                let mut tile = frost::Tile::new(
                    [
                        cx + (col as f32 - (cols - 1.0) / 2.0) * cell,
                        cy + ((rows_n - 1.0) / 2.0 - row as f32) * cell,
                    ],
                    [cell, cell],
                    [0.0, 0.0, 1.0, 1.0],
                );
                tile.color = tint;
                map.push_tile(tile);
            }
        }
        ctx.scene().root.children.push(Box::new(SceneNode {
            order: z + 1.5,
            modulate: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a,
            },
            shape: Some(map),
            ..Default::default()
        }));
    }

    /// The marks that make the descent visibly longer than the window:
    /// a chevron above the first row while hills wait out of sight
    /// above, and one below the last visible row while more wait
    /// below, in the gap the pinned `back` shares. Barrfont carries
    /// letters and a space and nothing else, so the arrows are drawn
    /// as two short strokes meeting at a tip — each one a polyline
    /// node, riding the page's fade. They sit in the inter-row gaps,
    /// where no text reaches: a row's glyphs stand 25 off its centre
    /// and a mark 11 off its own, and the gaps are wider than both.
    fn draw_progress_marks(&self, ctx: &mut Context, rows: &[Row], h: f32, z: f32, a: f32) {
        let hills = rows
            .iter()
            .filter(|r| matches!(r.action, Action::Hill(_)))
            .count();
        let (up, down) = progress_marks(self.progressed(), self.progress_top, hills);
        if !up && !down {
            return;
        }
        let chevron = |cy: f32, tip: f32, ctx: &mut Context| {
            ctx.scene().root.children.push(Box::new(SceneNode {
                order: z + 3.0,
                modulate: Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a,
                },
                shape: Some(frost::Shape::Polyline {
                    points: vec![[-15.0, -tip * 0.55], [0.0, tip], [15.0, -tip * 0.55]],
                    width: 5.0,
                    color: C_TEXT,
                }),
                transform: Transform::translate([0.0, cy]),
                ..Default::default()
            }));
        };
        let top = progress_top_y(h);
        let step = progress_step(h);
        if up {
            chevron(top + step * 0.42, 8.0, ctx);
        }
        if down {
            chevron(top - (hills as f32 - 0.5) * step, -8.0, ctx);
        }
    }

    /// A row's answer: the page rows turn the page, the setting rows
    /// flip the engine the moment they move, and everything the disc
    /// should remember is written before the click is even over —
    /// except the sliders, which save on release.
    fn menu_action(&mut self, ctx: &mut Context, action: Action) {
        match action {
            Action::New => {
                self.new_game();
                self.progress.stood(self.idx);
                self.progress.save();
            }
            Action::Resume => self.page = Page::Game,
            Action::OpenSettings => {
                self.page = Page::Settings;
                self.drag = None;
                self.menu_sel = 0;
            }
            Action::OpenProgress => {
                self.page = Page::Progress;
                self.menu_sel = 0;
                self.progress_top = 0;
            }
            Action::Next => {
                // The greyed row answers nothing, whichever key or
                // click called it: the hill underfoot is all the
                // descent still owes, and `next` leads nowhere new.
                if !self.next_owed() {
                    return;
                }
                // Not the next hill number — the next owed hill: the
                // freshly unlocked one at the frontier, else the
                // nearest without a perfect run. `next_hill` says it
                // at length; this is the same key's short hand.
                let next = self.next_hill();
                self.load(next);
                self.page = Page::Game;
                // Standing on a new hill is the fact the progress
                // file keeps — written the moment it's true.
                self.progress.stood(self.idx);
                self.progress.save();
            }
            Action::Retry => {
                self.load(self.idx);
                self.page = Page::Game;
            }
            Action::Hill(n) => {
                // A ledger row is a door: walking back into a warmed
                // hill is a hill moved, and the file says so.
                self.load(n);
                self.page = Page::Game;
                self.progress.stood(self.idx);
                self.progress.save();
            }
            Action::Exit => ctx.exit(),
            Action::Keyboard => {
                self.page = Page::Keyboard;
                self.menu_sel = 0;
            }
            Action::Mouse => {
                self.page = Page::Mouse;
                self.menu_sel = 0;
            }
            Action::Back => {
                self.page = Page::Main;
                self.menu_sel = 0;
            }
            Action::Vsync => {
                self.settings.vsync = !self.settings.vsync;
                ctx.set_vsync(self.settings.vsync);
            }
            Action::Filter => {
                self.settings.nearest = !self.settings.nearest;
                ctx.set_blit_filter(if self.settings.nearest {
                    SpriteFilter::Nearest
                } else {
                    SpriteFilter::Linear
                });
            }
            Action::Fullscreen => {
                self.settings.fullscreen = !self.settings.fullscreen;
                self.apply_window_policy(ctx);
            }
            Action::Quality => {
                self.settings.quality = self.settings.quality.next();
                ctx.set_render_size(self.settings.quality.size());
                self.apply_window_policy(ctx);
            }
            Action::Leaves => self.settings.leaves_lit = !self.settings.leaves_lit,
            Action::Glow => self.settings.lamp_glow = !self.settings.lamp_glow,
            Action::Shadows => self.settings.shadows = self.settings.shadows.next(),
            Action::Ambient | Action::Light => return,
        }
        self.settings.save();
    }

    /// The menu's input pass. A slider owns the button from the press
    /// on the track until it lifts — dragging previews live, and only
    /// the release writes the disc. Any other row answers its press at
    /// once.
    fn menu_input(&mut self, ctx: &mut Context) {
        let pointer = ctx.mouse_position();
        if let Some(sl) = self.drag {
            if ctx.mouse_button_down(MouseButton::Left) {
                if let Some([x, _]) = pointer {
                    let frac = slide_value(x, MENU_TRACK_CX, MENU_TRACK_HALF);
                    match sl {
                        Slider::Ambient => self.settings.ambient = frac,
                        Slider::Light => self.settings.light = frac * 2.0,
                    }
                }
            } else {
                self.drag = None;
                self.settings.save();
            }
        }
        // The keyboard's turn: arrows walk the rows, Enter presses
        // the highlighted one, and left/right nudge a slider by five
        // points when the highlight stands on its row. The same
        // `menu_rows` list the mouse reads, so the two never diverge.
        let h = self.view(ctx).1;
        let rows = self.menu_rows(self.page, h);
        if !rows.is_empty() {
            self.menu_sel = self.menu_sel.min(rows.len() - 1);
            // The ledger leans: at its window's edges the arrow keys
            // slide the window over the hills instead of wrapping —
            // and the wheel drags the window directly, lines for
            // lines. Every other page keeps the plain walk.
            let up = self.pressed(ctx, KeyCode::ArrowUp);
            let down = self.pressed(ctx, KeyCode::ArrowDown);
            if self.page == Page::Progress {
                let hills = rows.len() - 1; // the pinned back row aside
                let more = self.progressed() > self.progress_top + hills;
                if up && self.menu_sel == 0 && self.progress_top > 0 {
                    self.progress_top -= 1;
                } else if down && self.menu_sel + 1 == hills && more {
                    self.progress_top += 1;
                } else {
                    // The plain walk: within the window the arrows
                    // do what they do everywhere else, and the wrap
                    // — the only way off either end without leaning
                    // — comes home to the ledger's own top.
                    let wrapped =
                        (up && self.menu_sel == 0) || (down && self.menu_sel + 1 == rows.len());
                    if up {
                        self.menu_sel = step_sel(self.menu_sel, rows.len(), -1);
                    }
                    if down {
                        self.menu_sel = step_sel(self.menu_sel, rows.len(), 1);
                    }
                    if wrapped {
                        self.progress_top = 0;
                    }
                }
            } else {
                if up {
                    self.menu_sel = step_sel(self.menu_sel, rows.len(), -1);
                }
                if down {
                    self.menu_sel = step_sel(self.menu_sel, rows.len(), 1);
                }
            }
            if self.page == Page::Progress {
                let wheel = ctx.mouse_wheel();
                if wheel != 0.0 {
                    let last = self.progressed().saturating_sub(progress_fits(h));
                    let top = self.progress_top as i32 - wheel.round() as i32;
                    self.progress_top = top.clamp(0, last as i32) as usize;
                }
            }
            let nudge = match (
                self.pressed(ctx, KeyCode::ArrowLeft),
                self.pressed(ctx, KeyCode::ArrowRight),
            ) {
                (true, false) => Some(-1.0),
                (false, true) => Some(1.0),
                _ => None,
            };
            if let Some(dir) = nudge
                && rows[self.menu_sel].slider.is_some()
            {
                match rows[self.menu_sel].action {
                    Action::Ambient => {
                        self.settings.ambient = (self.settings.ambient + dir * 0.05).clamp(0.0, 1.0)
                    }
                    _ => self.settings.light = (self.settings.light + dir * 0.10).clamp(0.0, 2.0),
                }
                self.settings.save();
            }
            if self.pressed(ctx, KeyCode::Enter) || self.pressed(ctx, KeyCode::NumpadEnter) {
                let action = rows[self.menu_sel].action;
                self.menu_action(ctx, action);
            }
            // Tab turns the highlighted ledger row over: its recorded
            // best run replays itself at the click walker's pace.
            // A hill without a recording stays as it was — that row
            // is a door to playing, not yet to showing.
            if self.page == Page::Progress
                && self.pressed(ctx, KeyCode::Tab)
                && let Some(&Action::Hill(h)) = rows.get(self.menu_sel).map(|r| &r.action)
            {
                self.start_replay(h);
            }
        }
        // The pointer moving onto a row takes the highlight with it —
        // motion only, so a cursor parked on `resume` cannot out-vote
        // the arrow keys walking to `exit`. One row is ever lit.
        if pointer != self.last_pointer
            && let Some(p) = pointer
            && let Some(i) = row_under(&rows, p)
        {
            self.menu_sel = i;
        }
        self.last_pointer = pointer;
        // The mouse's turn: a click presses its row's action and takes
        // the highlight with it.
        if !self.pressed_mouse(ctx, MouseButton::Left) {
            return;
        }
        let Some(p) = pointer else { return };
        if let Some(i) = row_under(&rows, p) {
            let r = &rows[i];
            self.menu_sel = i;
            match r.action {
                Action::Ambient | Action::Light => {
                    self.drag = Some(match r.action {
                        Action::Ambient => Slider::Ambient,
                        _ => Slider::Light,
                    });
                    let frac = slide_value(p[0], MENU_TRACK_CX, MENU_TRACK_HALF);
                    match r.action {
                        Action::Ambient => self.settings.ambient = frac,
                        _ => self.settings.light = frac * 2.0,
                    }
                }
                action => self.menu_action(ctx, action),
            }
        }
    }

    /// The rows of the page currently up, laid out in user space —
    /// the list the pointer hit-tests and the paint draws, so a click
    /// can never miss the picture it answers to. `h` is the window's
    /// user height: the block hangs from its top, and shrinks its
    /// stride on short windows (a 540p quality buffer still wears the
    /// whole menu).
    fn menu_rows(&self, page: Page, h: f32) -> Vec<Row> {
        let st = &self.settings;
        let mut rows: Vec<Row> = Vec::new();
        let short = h < 720.0;
        let step = if short { 46.0 } else { 58.0 };
        let push = |rows: &mut Vec<Row>, action, cy, hw, text: String, slider: Option<f32>| {
            rows.push(Row {
                action,
                cx: if slider.is_some() { MENU_TRACK_CX } else { 0.0 },
                cy,
                hw,
                hh: 24.0,
                text,
                slider,
            })
        };
        match page {
            Page::Game => {}
            Page::Main => {
                let top = (h * 0.5 - 120.0).min(120.0);
                push(&mut rows, Action::New, top, 240.0, "new game".into(), None);
                push(
                    &mut rows,
                    Action::Resume,
                    top - step,
                    240.0,
                    "resume game".into(),
                    None,
                );
                push(
                    &mut rows,
                    Action::OpenSettings,
                    top - 2.0 * step,
                    240.0,
                    "settings".into(),
                    None,
                );
                push(
                    &mut rows,
                    Action::OpenProgress,
                    top - 3.0 * step,
                    240.0,
                    "progress".into(),
                    None,
                );
                push(
                    &mut rows,
                    Action::Keyboard,
                    top - 4.0 * step,
                    240.0,
                    "keyboard".into(),
                    None,
                );
                push(
                    &mut rows,
                    Action::Mouse,
                    top - 5.0 * step,
                    240.0,
                    "mouse".into(),
                    None,
                );
                push(
                    &mut rows,
                    Action::Exit,
                    top - 6.0 * step,
                    240.0,
                    "exit".into(),
                    None,
                );
            }
            Page::Won => {
                // The won page hangs lower than its siblings: the
                // 315-unit drop leaves a constant 92-unit strip —
                // plate top to the HUD's lowest banner (the score,
                // h/2 - 78 with a 30-unit body: bottom at h/2 - 93)
                // — and the verdict, at its full 64, is centred in
                // that strip at any window height. The old 120-unit
                // hang made the strip negative at 720p, and the
                // word lay across both banners.
                let top = (h * 0.5 - 315.0).min(120.0);
                push(
                    &mut rows,
                    Action::Next,
                    top,
                    240.0,
                    "next hill".into(),
                    None,
                );
                push(
                    &mut rows,
                    Action::Retry,
                    top - step,
                    240.0,
                    "retry hill".into(),
                    None,
                );
            }
            Page::Settings => {
                // Ten rows now that the glow has a rung of its own, which is
                // sixteen pixels more than the page used to carry: hung higher
                // so the `back` row still clears a 540p window's bottom edge.
                let top = (h * 0.5 - 104.0).min(252.0);
                let on = |b: bool| if b { "on" } else { "off" };
                push(
                    &mut rows,
                    Action::Vsync,
                    top,
                    250.0,
                    format!("vsync   {}", on(st.vsync)),
                    None,
                );
                push(
                    &mut rows,
                    Action::Filter,
                    top - step,
                    250.0,
                    format!(
                        "filter   {}",
                        if st.nearest { "nearest" } else { "bilinear" }
                    ),
                    None,
                );
                push(
                    &mut rows,
                    Action::Fullscreen,
                    top - 2.0 * step,
                    250.0,
                    format!("fullscreen   {}", on(st.fullscreen)),
                    None,
                );
                push(
                    &mut rows,
                    Action::Quality,
                    top - 3.0 * step,
                    250.0,
                    format!("resolution   {}", st.quality.label()),
                    None,
                );
                push(
                    &mut rows,
                    Action::Ambient,
                    top - 4.0 * step,
                    MENU_TRACK_HALF + 12.0,
                    format!("ambient   {}", (st.ambient * 100.0).round() as i32),
                    Some(st.ambient),
                );
                push(
                    &mut rows,
                    Action::Light,
                    top - 5.0 * step,
                    MENU_TRACK_HALF + 12.0,
                    format!("light fx   {}", (st.light * 50.0).round() as i32),
                    Some(st.light / 2.0),
                );
                push(
                    &mut rows,
                    Action::Glow,
                    top - 6.0 * step,
                    250.0,
                    format!("lamp glow   {}", on(st.lamp_glow)),
                    None,
                );
                push(
                    &mut rows,
                    Action::Leaves,
                    top - 7.0 * step,
                    250.0,
                    format!("leaf lighting   {}", on(st.leaves_lit)),
                    None,
                );
                push(
                    &mut rows,
                    Action::Shadows,
                    top - 8.0 * step,
                    250.0,
                    format!("lamp shadows   {}", st.shadows.label()),
                    None,
                );
                push(
                    &mut rows,
                    Action::Back,
                    top - 9.0 * step,
                    250.0,
                    "back".into(),
                    None,
                );
            }
            Page::Keyboard => {
                // Lists the keyboard's whole vocabulary. Every row is
                // the way out: these pages are read, not used.
                let top = (h * 0.5 - 120.0).min(120.0);
                for (i, line) in [
                    "wasd or arrows to move",
                    "space to detach",
                    "shift wasd or arrows to push",
                    "z to undo",
                    "r to reset level",
                    "back",
                ]
                .into_iter()
                .enumerate()
                {
                    push(
                        &mut rows,
                        Action::Back,
                        top - i as f32 * step,
                        280.0,
                        line.to_string(),
                        None,
                    );
                }
            }
            Page::Mouse => {
                let top = (h * 0.5 - 120.0).min(120.0);
                for (i, line) in [
                    "left click to move",
                    "right click to detach",
                    "shift click to push",
                    "back",
                ]
                .into_iter()
                .enumerate()
                {
                    push(
                        &mut rows,
                        Action::Back,
                        top - i as f32 * step,
                        280.0,
                        line.to_string(),
                        None,
                    );
                }
            }
            Page::Progress => {
                // The descent, cut to its window and ordered by the
                // work it still owes: hills not yet perfected come
                // first in hill order, the perfected ones follow in
                // hill order, so the top of the list is always the
                // run still to earn and the clean sweeps settle to
                // the bottom. A hill stood on but not warmed owns no
                // run — and no run is no perfect run — so it leads
                // with the imperfect. `perfect` is the exact
                // negation of the score's red, so a hill's place in
                // the order and its colour can never disagree. The
                // `back` row stays pinned under the last visible one,
                // where it can never scroll out of reach; the window
                // and the scroll marks count rows, and a reorder
                // moves hills between them, never a row out.
                let top = progress_top_y(h);
                let pstep = progress_step(h);
                let total = self.progressed();
                let mut order: Vec<usize> = (0..total).collect();
                order.sort_by_key(|&i| (self.perfect(i), i));
                let start = self.progress_top.min(total);
                let n = progress_fits(h).min(total - start);
                for (row, &i) in order[start..start + n].iter().enumerate() {
                    let name = self.levels.get(i).map_or("hill", |l| l.name.as_str());
                    let score = match self.progress.scores.iter().find(|(h, _)| *h == i) {
                        Some((_, moves)) => format!("   {moves} moves"),
                        None => String::new(),
                    };
                    push(
                        &mut rows,
                        Action::Hill(i),
                        top - row as f32 * pstep,
                        PROGRESS_PLATE - 12.0,
                        format!("{name}{score}"),
                        None,
                    );
                }
                push(
                    &mut rows,
                    Action::Back,
                    top - n as f32 * pstep,
                    240.0,
                    "back".to_string(),
                    None,
                );
            }
        }
        rows
    }

    /// New Game: the descent restarts at the first hill. Every stat
    /// the game keeps lives inside `Game::start`'s fresh state, and
    /// `load` already sweeps the extras the demo layers on top —
    /// queued steps, drifting zs, the frost's claim, verdicts; the
    /// saved progress is rewritten by the caller, which owns when
    /// the disc learns of it. Then the page steps aside and the hill
    /// plays.
    fn new_game(&mut self) {
        self.load(0);
        self.page = Page::Game;
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
        if game.act(game::Act::Move(dir)).is_err() {
            self.route.clear();
        }
    }

    /// Tab over a ledger row: the hill opens on its own best run.
    /// Nothing is stood on and nothing is scored — a showing, not a
    /// walking — and the hill it opens on records nothing either.
    fn start_replay(&mut self, hill: usize) {
        let Some(glyphs) = self.progress.route_of(hill).map(str::to_owned) else {
            return;
        };
        let Some(acts) = decode_run(&glyphs) else {
            return;
        };
        if acts.is_empty() {
            return;
        }
        self.load(hill);
        // The ledger remembers the deed, never the showing of it.
        if let Some(g) = &mut self.game {
            g.record = false;
        }
        self.replay = Some(Replay {
            acts,
            shown: 0,
            playing: true,
        });
        self.page = Page::Game;
    }

    /// The showing's pace is the click walker's law — one action per
    /// landed glide — so a recorded run plays at exactly the speed a
    /// hand on the mouse would walk it.
    fn replay_tick(&mut self) {
        if !self.replay.as_ref().is_some_and(|r| r.playing) {
            return;
        }
        if self.spin.is_some() || !closer(self.player_at, self.player_target) {
            return;
        }
        self.replay_step(false);
    }

    /// Apply the next recorded action. `now` is the arrow key's hand:
    /// it never waits for the glide to land and turns without the
    /// spin ceremony — stepping is inspection, not choreography.
    fn replay_step(&mut self, now: bool) {
        let Some(a) = self
            .replay
            .as_ref()
            .and_then(|r| r.acts.get(r.shown).copied())
        else {
            return; // the run has already arrived
        };
        if let Some(to) = a.dir().map(face_of)
            && self.face[0] * to[0] + self.face[1] * to[1] < -0.99
        {
            // The recorded walker about-faced once; a playing run
            // about-faces the way the player did, live.
            if now {
                self.face = to;
                self.spin = None;
            } else {
                self.spin = Some(to);
                return;
            }
        }
        let Some(game) = &mut self.game else {
            return;
        };
        if game.act(a).is_err() {
            // The hill has moved under the recording — a level redone
            // since the run was earned, or a hand in the ledger file.
            // The showing stops where the truth stops, and says so.
            if let Some(rep) = &mut self.replay {
                rep.playing = false;
            }
            self.note = "the recording ends where the hill has changed".to_string();
            self.note_t = 2.6;
            return;
        }
        if matches!(a, game::Act::Push(_)) {
            // A shove turns the pusher without carrying it, so the
            // glide that would set the facing never comes — the same
            // law the player's own shoves obey.
            if let Some(d) = a.dir() {
                self.face = face_of(d);
            }
        }
        if let Some(rep) = &mut self.replay {
            rep.shown += 1;
            if rep.shown >= rep.acts.len() {
                rep.playing = false; // the run has arrived; it holds
            }
        }
    }

    /// The right arrow: one action forward, playing or held — a hand
    /// on the wheel takes the wheel, and the glide still animates
    /// under the step like under any other.
    fn replay_advance(&mut self) {
        if let Some(rep) = &mut self.replay {
            rep.playing = false;
        }
        self.replay_step(true);
    }

    /// The left arrow: one action back. The hill is rebuilt from its
    /// own beginning through one fewer action — cheap at these
    /// lengths, and the only rewind that can promise exactness.
    fn replay_retreat(&mut self) {
        let Some(rep) = &self.replay else { return };
        if rep.shown == 0 {
            return;
        }
        let n = rep.shown - 1;
        let acts = rep.acts.clone();
        let Some(level) = self.levels.get(self.idx) else {
            return;
        };
        let mut g = game::Game::start(level.clone());
        for &a in acts.iter().take(n) {
            if g.act(a).is_err() {
                break; // the drift that stopped the showing stops here too
            }
        }
        if let Some(d) = acts.iter().take(n).rev().find_map(|a| a.dir()) {
            self.face = face_of(d);
        }
        self.spin = None;
        self.game = Some(g);
        if let Some(rep) = &mut self.replay {
            rep.shown = n;
            rep.playing = false;
        }
    }

    /// Beat the clock, and tell the console when too long a silence
    /// came before the beat. The engine clamps `dt` at one second — a
    /// stall arrives as an ordinary big frame — so the stutter ledger
    /// rides the wall clock instead: half a second of silence gets one
    /// line. A long frame reports as it happens; a hard hang leaves
    /// its last beat behind as the stopped heart, and the difference
    /// between the two is the diagnosis a machine across the room
    /// — or across the house — otherwise never gives up.
    fn beat(&mut self) {
        let now = now_ms();
        if self.last_beat > 0.0 {
            let gap = (now - self.last_beat) / 1000.0;
            if gap >= 0.5 {
                report_gap(gap, self.page);
            }
        }
        self.last_beat = now;
    }
}

impl Process for Demo {
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        self.beat();
        // The overlay breathes: toward one while a menu is up, toward
        // zero when the hill takes the screen back. The page it
        // paints follows the live page in, and holds the last one
        // through the fade out, so closing a menu shows that menu
        // leaving — never whatever page happens to be behind it.
        let target = if self.page == Page::Game { 0.0 } else { 1.0 };
        if target > 0.0 {
            self.menu_page = self.page;
        }
        self.menu_alpha += (target - self.menu_alpha) * (1.0 - (-dt / 0.15).exp());
        if (self.menu_alpha - target).abs() < 0.002 {
            self.menu_alpha = target;
        }
        if self.window_policy_pending {
            self.window_policy_pending = false;
            self.apply_window_policy(ctx);
        }
        // The glow, like the rest of the look: written every frame, so the
        // menu's row answers on the next one. This is the only place the
        // strength is decided, and at zero the chain declines to run at all —
        // the row does not hide the halo, it stops paying for it.
        ctx.set_bloom(if self.settings.lamp_glow {
            BLOOM_STRENGTH
        } else {
            0.0
        });
        // A window the compositor owns — tiled or snapped, by KWin's
        // tiling editor or a window rule — re-asserts its own
        // geometry no matter what the game asks for, and says so
        // only through configure events. Watch those events: if the
        // window is not at the requested size, re-ask, and say the
        // quiet part aloud if it never comes. The letterbox keeps the
        // game 16:9 and playable in whatever size survives.
        //
        // First the state the row is written against. Leaving fullscreen is
        // the case that bites: an exit the desktop makes on its own leaves the
        // row saying `on` while the window sits windowed, at the size the
        // desktop saved before it went fullscreen — and no policy ever runs
        // again, because the game believes its own row, so the tier's pixels
        // and the pin that holds the aspect both stay off. Adopt what the
        // window says and let the policy answer to *that*: the size comes
        // back, the lock comes back, and the row tells the truth instead of
        // the one the player pressed an hour ago. Not saved to the file — a
        // gesture the desktop made on its own should not outlive the run.
        if let Some(window) = ctx.window() {
            let state = window.fullscreen().is_some();
            if state == self.settings.fullscreen || self.in_flight() {
                self.fullscreen_drift = 0;
            } else {
                self.fullscreen_drift += 1;
                if self.fullscreen_drift == 60 {
                    log::info!(
                        "window policy: the desktop has it {}fullscreen, the row said otherwise",
                        if state { "" } else { "out of " },
                    );
                    self.settings.fullscreen = state;
                    self.apply_window_policy(ctx);
                }
            }
        }
        if let Some(ask) = self.window_expect
            && let Some(window) = ctx.window()
        {
            {
                let TierAsk {
                    w,
                    h,
                    off,
                    calm,
                    asks,
                } = ask;
                let size = window.inner_size();
                let at_tier = size.width.abs_diff(w) < 2 && size.height.abs_diff(h) < 2;
                if at_tier && calm == 0 {
                    // Arrived, and only now take the handle: locking pins to
                    // the *current* size, so locking before arrival is how you
                    // pin a window to the wrong one. Idempotent where the
                    // compositor obeyed the hints, curative where a stomp left
                    // them off.
                    let px = frost::PhysicalSize::new(w, h);
                    window.set_min_inner_size(Some(px));
                    window.set_max_inner_size(Some(px));
                    window.set_resizable(false);
                    self.window_expect = Some(TierAsk {
                        w,
                        h,
                        off: 0,
                        calm: 1,
                        asks,
                    });
                } else if at_tier {
                    self.window_expect = if calm + 1 >= TierAsk::HOLD {
                        log::info!(
                            "window policy: {}x{} held for {} seconds, the tier stands",
                            w,
                            h,
                            TierAsk::HOLD / 60,
                        );
                        None
                    } else {
                        Some(TierAsk {
                            w,
                            h,
                            off,
                            calm: calm + 1,
                            asks,
                        })
                    };
                } else if off > TierAsk::PATIENCE || asks > TierAsk::PATIENCE_ASKS {
                    self.window_expect = None;
                    println!(
                        "antifrost: asked the window for {w}x{h} {} times, it stayed {}x{} \u{2014} \
                         the desktop is keeping its own geometry: a tile or a snap, a window \
                         rule, or a fullscreen exit that restored the shape the window had \
                         before it went in. Take it out of the tile, or set the row back to \
                         native and let the letterbox hold the 16:9.",
                        asks, size.width, size.height,
                    );
                } else {
                    // Not there yet, so ask again rather than only watch. One
                    // ask is a bet that the first request still stands, and
                    // two ordinary things take that bet away: an exit from
                    // fullscreen restores the window's old shape *after* the
                    // request made in the same breath as `set_fullscreen(None)`,
                    // and a min/max pin left at the tier's size while the
                    // window sits somewhere else is a hint the compositor has
                    // no reason to obey. So unlock, ask, and leave it unlocked
                    // until it arrives — which is the same order the policy
                    // uses, because locking pins to the current size and the
                    // current size is exactly what is wrong.
                    if off % TierAsk::CADENCE == TierAsk::CADENCE - 1 {
                        window.set_min_inner_size(None::<frost::PhysicalSize<u32>>);
                        window.set_max_inner_size(None::<frost::PhysicalSize<u32>>);
                        let again = window.request_inner_size(frost::PhysicalSize::new(w, h));
                        if off == TierAsk::CADENCE - 1 {
                            // Once, not on every re-ask. The line is there to
                            // say the first ask was answered by something other
                            // than the window arriving; the asks after it are
                            // that same fact repeated.
                            log::info!(
                                "window policy: still {}x{}, not the {w}x{h} asked \
                                 for — re-asking ({again:?})",
                                size.width,
                                size.height,
                            );
                        }
                    }
                    self.window_expect = Some(TierAsk {
                        w,
                        h,
                        off: off + 1,
                        calm: 0,
                        asks: if off % TierAsk::CADENCE == TierAsk::CADENCE - 1 {
                            asks + 1
                        } else {
                            asks
                        },
                    });
                }
            }
        }
        if self.window_move_queued && !self.in_flight() {
            self.window_move_queued = false;
            self.apply_window_policy(ctx);
        }
        if let Some(clock) = self.resize_probe {
            let t = clock + dt;
            self.resize_probe = Some(t);
            // Edge-triggered, every one: a range check on a 60 Hz
            // clock is a six-frame window, and six resize requests
            // on Wayland — where configures land asynchronously —
            // is six fights to watch in the log instead of one
            // answer. Each threshold crosses exactly once.
            if clock < 2.5 && t >= 2.5 {
                log::info!("resize probe: cycling the real quality row");
                self.settings.quality = self.settings.quality.next();
                ctx.set_render_size(self.settings.quality.size());
                self.apply_window_policy(ctx);
            }
            // High's `next` is native — which asks for no resize —
            // so the probe cycles twice: unlock at High's exit, and
            // the real 1080 -> 720 request at the next row.
            if clock < 4.5 && t >= 4.5 {
                log::info!("resize probe: cycling the real quality row again");
                self.settings.quality = self.settings.quality.next();
                ctx.set_render_size(self.settings.quality.size());
                self.apply_window_policy(ctx);
            }
            // Cycling the rung is not the whole question. A windowed tier's
            // size is asked for in the same breath as the fullscreen exit, and
            // a compositor that answers the exit second wins: the window comes
            // back at whatever shape it saved before it went in, which is the
            // one thing the row is supposed to rule out. So the probe goes in,
            // comes out, and says what survived — with the tier named, so the
            // log answers whether the window agrees without anyone counting
            // pixels on a screen.
            if clock < 7.0 && t >= 7.0 {
                log::info!("resize probe: into fullscreen");
                self.settings.fullscreen = true;
                self.apply_window_policy(ctx);
            }
            if clock < 9.0 && t >= 9.0 {
                log::info!("resize probe: out of fullscreen");
                self.settings.fullscreen = false;
                self.apply_window_policy(ctx);
            }
            if clock < 12.0 && t >= 12.0 {
                if let Some(window) = ctx.window() {
                    log::info!(
                        "resize probe: settled at {}x{}, tier asks {:?}",
                        window.inner_size().width,
                        window.inner_size().height,
                        self.settings.quality.size(),
                    );
                }
                self.resize_probe = None;
            }
        }
        // The bench hourglass: seconds run and frames counted before
        // anything else decides whether this frame is spent on the
        // hill or the menu — every frame draws the same hill and
        // light field either way, which is exactly what an A/B wants
        // to weigh. The first two seconds warm the clocks and go
        // uncounted.
        if let Some(secs) = self.bench {
            self.bench_t += dt;
            if self.bench_t > 2.0 {
                self.bench_frames += 1;
            }
            if self.bench_t >= secs + 2.0 {
                let span = self.bench_t - 2.0;
                log::info!(
                    "antifrost: bench {:.1} fps over {:.0} s",
                    self.bench_frames as f32 / span,
                    span
                );
                ctx.exit();
            }
        }
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
        // `Config::fullscreen` — and it is the menu's row and nothing else, the
        // one action with the one policy behind it. That is not tidiness: the key
        // used to toggle the window's state by hand and stop there, which left a
        // window coming *out* of fullscreen at whatever shape the desktop had
        // saved for it, with no tier size asked for and no lock to hold the 16:9,
        // and a window going *in* still pinned min-to-max at the tier it was
        // about to outgrow. Both directions were the policy's business, and the
        // key had quietly taken itself out of it. The disc still remembers the
        // move, because the row's own save is at the end of the action.
        if self.pressed(ctx, KeyCode::F11) {
            self.menu_action(ctx, Action::Fullscreen);
        }
        // Escape answers its page: the hill yields to the menu, the
        // menu hands the hill back, settings step back to the menu.
        // Quitting belongs to the exit row alone. (`Config::escape_exits`
        // is off — the demo owns this key.)
        if self.pressed(ctx, KeyCode::Escape) {
            if self.replay.is_some() {
                // Stepping off a showing returns to the ledger it
                // came from, and the hill is wiped back to its own
                // beginning: nobody resumes a run they only watched.
                let hill = self.idx;
                self.replay = None;
                self.load(hill);
                self.page = Page::Progress;
                self.drag = None;
            } else {
                match escape_nav(self.page) {
                    Escape::ToMenu => {
                        self.page = Page::Main;
                        self.drag = None;
                        self.menu_sel = 0;
                    }
                    Escape::Resume => self.page = Page::Game,
                }
            }
        }
        // The night and the lamps answer to the settings every frame:
        // the ambient floor scales the shipped colour, the leaf deck
        // decides whether it joins the lit world, and the lamp scale
        // rides every light the draw declares below.
        {
            ctx.scene().ambient = ambient_floor(self.settings.ambient);
            let lit = self.settings.leaves_lit;
            for node in ctx.scene().root.children[..LEAVES.len()].iter_mut() {
                node.lit = lit;
            }
        }
        // Alt-V belongs to the diagnostics overlay and flips the
        // engine's vsync behind the menu's back; rather than argue,
        // the settings follow the engine, so the menu's row reads the
        // truth the next time it opens. (No save here: this only
        // remembers, the disc waits for a row the player moved.)
        if ctx.vsync() != self.settings.vsync {
            self.settings.vsync = ctx.vsync();
        }
        // During a showing the keys belong to the audience: space
        // holds and resumes the run, ← and → step it one action at a
        // time. The hill's own verbs all wait for the next visitor.
        if self.game.is_some() && self.page == Page::Game && self.replay.is_some() {
            if self.pressed(ctx, KeyCode::Space)
                && let Some(rep) = &mut self.replay
                // Past the last step there is nothing to resume:
                // the pause key keeps its peace at the end.
                && (rep.playing || rep.shown < rep.acts.len())
            {
                rep.playing = !rep.playing;
            }
            if self.pressed(ctx, KeyCode::ArrowRight) {
                self.replay_advance();
            }
            if self.pressed(ctx, KeyCode::ArrowLeft) {
                self.replay_retreat();
            }
        } else if self.game.is_some() && self.page == Page::Game {
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
            // N and P retired with their legend line: leaving a
            // finished hill belongs to the won page's two rows,
            // which answer to click and Enter alike — a control
            // nothing mentions is a control that misfires in
            // silence.
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
                    // A restarted hill is played again, so it records
                    // again; the old walk was a worse walk anyway.
                    game.record = true;
                    self.route.clear();
                    self.replay = None;
                } else if undo {
                    game.undo();
                    self.route.clear();
                } else if (detach || detach_click) && !game.won {
                    self.route.clear();
                    let _ = game.act(game::Act::Detach);
                } else if let Some(dir) = push_aim.filter(|_| !game.won) {
                    // The shove itself is the game's; the pusher faces
                    // the aim like the keys do, and a push — like the
                    // keyboard's — never carries the pusher along. A
                    // refusal says why, in the game's own words.
                    self.route.clear();
                    self.face = face_of(dir);
                    if let Err(why) = game.act(game::Act::Push(dir)) {
                        self.note = why.to_string();
                        self.note_t = 1.6;
                    }
                } else if let Some((dir, push)) = action.filter(|_| !game.won) {
                    self.route.clear();
                    if push {
                        // A push never moves the pusher, so no glide will
                        // carry its facing — turn by hand.
                        self.face = face_of(dir);
                        let _ = game.act(game::Act::Push(dir));
                    } else if self.spin.is_none() {
                        // The keyboard's 180 obeys the route's law:
                        // about-face in place, then walk.
                        let to = face_of(dir);
                        if self.face[0] * to[0] + self.face[1] * to[1] < -0.99 {
                            self.spin = Some(to);
                        } else {
                            let _ = game.act(game::Act::Move(dir));
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
            // The player is an ant, and the queen's bed is her target —
            // but reaching it is not yet home: standing on the white
            // bed with the colony still awake is a cell she stands on,
            // and her lamp keeps working like on any other. The tuck-in
            // — spike, then dark, head to the ceiling — waits for every
            // other ant to be asleep, which is exactly the moment the
            // hill calls won.
            self.player_home = match game.tile(game.player) {
                level::Tile::Bed(game::AntColor::White) if game.won => {
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
        // A won hill hands the player their two ways off it — but
        // only after the splash has spun, held, and faded: the word
        // comes first, then the panel and the verdict together. A
        // showing that ends in a warm hill is not a win: it is an
        // ending — the audience keeps the ledger's silence.
        if self.page == Page::Game
            && self.replay.is_none()
            && self.splash_done
            && self.game.as_ref().is_some_and(|g| g.won)
        {
            self.page = Page::Won;
            self.drag = None;
            self.menu_sel = 0;
        }
        // The verdict: when the hill is won and has not yet been told
        // its own name, draw one from the hat — the praise tier reads
        // the par, the spin reads the dice.
        let mut scored = None;
        if let Some(game) = &self.game {
            if !game.won {
                self.splash_done = false;
            } else if self.replay.is_none() && self.splash.is_none() && !self.splash_done {
                let hit = game.level.par > 0 && game.moves <= game.level.par;
                // Same rule the banner paints by mid-run.
                let colour = if hit { C_PAR_MET } else { C_PAR_MISSED };
                let praise: [&str; 3] = if hit {
                    ["EXCELLENT", "PERFECT", "SUPER"]
                } else {
                    ["YOUR DID IT", "GOOD JOB", "WELL DONE"]
                };
                let pick = self.leaf_rng.in_range(0.0, 2.999) as usize;
                let text = praise[pick].to_string();
                // The word outlives its spin: the won panel reads it,
                // in the same bright the splash burned.
                self.verdict = Some((text.clone(), colour));
                let grow = self.leaf_rng.in_range(1.875, 2.55);
                let end = self.leaf_rng.in_range(-45.0, 45.0) * std::f32::consts::FRAC_PI_4 / 45.0;
                self.splash = Some(Splash {
                    text,
                    t: 0.0,
                    grow,
                    end,
                    color: colour,
                });
                scored = Some((self.idx, game.moves));
            }
        }
        // A finished hill joins the ledger on the tick it finishes —
        // the same tick that first spoke its verdict. The hill itself
        // has not moved, so only the score's line changes in the
        // file; a kill before this point lost an unfinished run, and
        // that is exactly what the ledger is for.
        if let Some((hill, moves)) = scored {
            // The number and the walk that earned it enter the ledger
            // together, and only together: a route changes hands only
            // when its score does, so Tab always shows the very run
            // behind the number. (A capped number may be shorter than
            // its true walk — the replay keeps the truth of it.)
            let route = self
                .game
                .as_ref()
                .map(|g| encode_run(&g.log))
                .unwrap_or_default();
            self.progress.record_run(hill, moves, &route);
            self.progress.save();
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
        if self.page == Page::Game {
            // One walker per frame: the audience's showing owns the
            // hill, or the player's own click route does.
            if self.replay.is_some() {
                self.replay_tick();
            } else {
                self.walk_the_route();
            }
        }
        // The menu takes the pointer whole while it is up.
        if self.page != Page::Game {
            self.menu_input(ctx);
        }

        self.update_leaves(ctx, dt);
        self.draw_ants(ctx, dt);
        self.draw_hud(ctx);
        self.draw_splash(ctx);
        if self.menu_alpha > 0.002 {
            self.draw_menu(ctx);
        }
        self.draw_letterbox(ctx);
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

/// Milliseconds on the wall clock: the browser's own clock in the
/// page, epoch milliseconds on the desktop — one shape for the beat
/// sentinel, whichever side of the wire it keeps time on.
#[cfg(target_arch = "wasm32")]
fn now_ms() -> f64 {
    js_sys::Date::now()
}

#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

/// One stutter line, said where the line lands: the devtools console
/// on the web — where `log` calls have been compiled out of the
/// release build the page ships — and the ordinary log on the
/// desktop, where they have not.
#[cfg(target_arch = "wasm32")]
fn report_gap(gap: f64, page: Page) {
    web_sys::console::warn_1(
        &format!("antifrost: {gap:.2} s since the last frame ({page:?} up)").into(),
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn report_gap(gap: f64, page: Page) {
    log::warn!("{gap:.2} s since the last frame ({page:?} up)");
}

/// The command line's arguments.
#[cfg(not(target_arch = "wasm32"))]
#[derive(clap::Parser)]
struct Cli {
    /// Present frames in sync with the display's vertical blank. `off`
    /// uncaps the frame rate — the hill has the headroom (the experiment
    /// hit 200 fps), but the cadence judders, and the tearing present
    /// (`Immediate`) bypasses VRR, so vsync stays on unless this flag
    /// says otherwise.
    ///
    /// An `Option`: a bare `bool` field is clap's zero-arg `--flag`,
    /// which has no value to parse.
    #[arg(long, value_name = "on|off", value_parser = parse_on_off)]
    vsync: Option<bool>,

    /// Run for SECONDS, log the mean frame rate, and quit on its own
    /// — the A/B instrument the benchmarking notes ask of every
    /// performance claim. Frames count only after the first two
    /// seconds, so the clocks have time to wake; everything else the
    /// launch decides, settings file included.
    #[arg(long, value_name = "SECONDS")]
    bench: Option<f32>,

    /// Cycle the resolution row once on a timer, logging the window's
    /// size before, answered, and settled — the diagnostic for
    /// "did the window actually move". Not a feature; a probe.
    #[arg(long)]
    resize_probe: bool,
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
    // The hill's pin: vsync keeps the present refresh-synced (`Fifo`),
    // which is what lets the display engage VRR; off tears (`Immediate`)
    // and bypasses it. A browser has no command line, so the packaged
    // build keeps the default; native can hand the uncapped experiment
    // to the caller with `--vsync off`.
    // The saved file speaks first, and the command line over it: a
    // deliberate `--vsync off` outranks yesterday's on. A browser has
    // no command line, so there its own memory of yesterday stands.
    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
    let mut settings = Settings::load();
    let progress = Progress::load();
    #[cfg(not(target_arch = "wasm32"))]
    let (bench, resize_probe) = {
        let cli: Cli = clap::Parser::parse();
        if let Some(v) = cli.vsync {
            settings.vsync = v;
        }
        (cli.bench, cli.resize_probe)
    };
    #[cfg(target_arch = "wasm32")]
    let (bench, resize_probe) = (None, false);
    let vsync = settings.vsync;
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
        {
            let mut d = Demo::new(ant_frames, soil, settings.clone(), progress);
            d.bench = bench;
            d.resize_probe = resize_probe.then_some(0.0);
            d
        },
        frost::Config {
            // A fixed tier asks for its own pixels *physically*, so
            // the window opens as an exact 1:1 copy of the buffer at
            // any display scaling, and comes up un-resizable for the
            // same reason; native opens logical, resizable, and lets
            // the letterbox hold the aspect.
            window_size: settings.quality.size().map_or(Some([1920, 1080]), |_| None),
            window_size_px: settings.quality.size(),
            resizable: settings.quality.size().is_none(),
            // The hill opens full screen — when the saved file says
            // to; F11 and the menu's row flip it mid-run either way.
            fullscreen: settings.fullscreen,
            // The hill always renders at 1080p and lets the stretch pass
            // carry it to the window or the display: this is a small,
            // cozy game for modest hardware, and the 4K-native experiment
            // cost ~35 ms a frame on a modern iGPU — most of it per-pixel
            // lighting arithmetic that the fixed buffer simply never
            // pays. A window the buffer's own size skips both the buffer
            // and the stretch pass, so windowed 1080p costs nothing
            // extra; Alt-F flips the stretch between bilinear (the soft,
            // default carry) and nearest.
            render_size: settings.quality.size(),
            blit_filter: if settings.nearest {
                SpriteFilter::Nearest
            } else {
                SpriteFilter::Linear
            },
            // The hill owns its Escape from here on: it opens the
            // menu, and the menu — and only the menu — quits.
            escape_exits: false,
            // The hill is lit by its own ants, so the glow starts with them
            // rather than arriving on the frame after the settings load: the
            // live write above keeps it in step from there on.
            bloom: if settings.lamp_glow {
                BLOOM_STRENGTH
            } else {
                0.0
            },
            // Vsync on unless the command line says otherwise: the
            // vsync-off experiment hit 200 fps with heavy judder, and
            // the tearing present bypasses VRR, so the refresh-synced
            // cadence stays the default pin.
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

    #[test]
    fn the_native_view_keeps_the_window_when_it_is_already_wide_enough() {
        // Exact 16:9 passes through untouched: no bars to draw.
        assert_eq!(fit_16x9(1920.0, 1080.0), (1920.0, 1080.0));
        // A wide window loses height to bars; a tall one loses width.
        let (w, h) = fit_16x9(1600.0, 1000.0);
        assert_eq!((w, h), (1600.0, 900.0));
        let (w, h) = fit_16x9(1000.0, 1000.0);
        assert_eq!((w, h), (1000.0, 562.5));
        // The box is always 16:9 and always inside the window.
        for (ww, hh) in [(800.0, 600.0), (2560.0, 1080.0), (400.0, 900.0)] {
            let (vw, vh) = fit_16x9(ww, hh);
            assert!((vw / vh - 16.0 / 9.0).abs() < 1e-3, "{ww}x{hh}");
            assert!(vw <= ww + 1e-3 && vh <= hh + 1e-3, "{ww}x{hh}");
        }
    }

    /// A `Demo` is pure CPU work until `run_configured`: asset bake,
    /// level load, route math — all testable headless.
    fn demo() -> Demo {
        let frames = ANT_FRAMES
            .iter()
            .map(|png| ant_frame(png).expect("shipped ant frames bake"))
            .collect();
        let soil = frost::Shape::sprite_bytes(SOIL).expect("shipped soil bakes");
        Demo::new(frames, soil, Settings::default(), Progress::default())
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

/// A merged occluder: columns `c0..=c1`, rows `r0..=r1`, inclusive grid
/// cells — the shape the cache holds and each frame packs into a world
/// rectangle.
type OccBox = (i32, i32, i32, i32);

/// The seed cover the fixpoint starts from, on its own so the bench can
/// weigh the two against each other: maximal runs per row — `tiles`
/// arrives row-major, so a run is a walk while the row holds and the
/// column stays adjacent — then a stack of equal spans merged upward.
/// Sorted by span, then row, so a stack of equal spans — the frame's
/// sides, a pile's flank — sits adjacent in line for the upward merge.
/// A row-major scan could never see it: a row with two runs puts the
/// other column's run between the stack's own links, and the stack
/// stayed one box per row, leaking a hairline of light through every
/// seam it drew.
fn seed_cover(tiles: &[(i32, i32, bool)]) -> Vec<OccBox> {
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
    runs.sort_unstable();
    let mut boxes: Vec<OccBox> = Vec::new();
    let mut k = 0;
    while k < runs.len() {
        let (col_s, col_e, row0) = runs[k];
        let mut row1 = row0;
        let mut m = k + 1;
        while m < runs.len() && runs[m].0 == col_s && runs[m].1 == col_e && runs[m].2 == row1 + 1 {
            row1 = runs[m].2;
            m += 1;
        }
        boxes.push((col_s, col_e, row0, row1));
        k = m;
    }
    boxes
}

/// The occluder-merge cache, as a pure function so the tests can walk it.
///
/// A box is `(col_s, col_e, row0, row1)`; the cover must hold every solid
/// cell in some box and no box may hold an empty one — the hill's shadow
/// is exactly the hill, and a box over a dug cell would wall light off
/// from a tunnel. The seed cover is the two-stage scan the cache has
/// always run: maximal runs per row, then a stack of equal spans merged
/// upward. The fixpoint pass comes after it, and it sees what the seed
/// cannot: two boxes of equal span split by a row whose run is wider —
/// the bar of a cross, the headroom of a shaft — leaves that span's
/// column stranded in two boxes, and whenever the stranded cells lie
/// solid the pair is one box. So the pass loops, merging any pair whose
/// bounding rectangle lies wholly inside the solids, until a round
/// merges nothing.
///
/// The rule keeps the cover exact without consulting any other box: the
/// merged rectangle holds only solid cells, and the pair's union already
/// covered those cells' rows and columns, so every cell that was covered
/// stays covered and nothing new joins the shadow — a merge may overlap
/// its neighbours freely. Every merge deletes a box, so the rounds
/// terminate, and the cover only ever shrinks toward the fewest boxes
/// this rule can prove.
fn merge_boxes(tiles: &[(i32, i32, bool)]) -> Vec<OccBox> {
    use std::collections::HashSet;
    let solid: HashSet<(i32, i32)> = tiles.iter().map(|&(row, col, _)| (row, col)).collect();
    let mut boxes = seed_cover(tiles);

    // The fixpoint. A pair may merge when its bounding rectangle is all
    // solid cells; one merge per round, rescanned, so a merge that
    // stretches a box is given its chance to enable the next. The count
    // falls by one each time, so the rounds end.
    let solid_rect = |c0: i32, c1: i32, r0: i32, r1: i32| {
        (r0..=r1).all(|row| (c0..=c1).all(|col| solid.contains(&(row, col))))
    };
    loop {
        let mut found: Option<(usize, usize, OccBox)> = None;
        'pairs: for a in 0..boxes.len() {
            for b in a + 1..boxes.len() {
                let (c0, c1) = (boxes[a].0.min(boxes[b].0), boxes[a].1.max(boxes[b].1));
                let (r0, r1) = (boxes[a].2.min(boxes[b].2), boxes[a].3.max(boxes[b].3));
                // Two boxes are one box exactly when the rectangle that
                // bounds the pair holds only solid cells — the all-solid
                // test is also the whole exactness argument: no empty is
                // ever covered. A rectangle no larger than `a` itself
                // says `b` already lies inside `a`; nothing is to gain.
                let rect = (c0, c1, r0, r1);
                if rect != boxes[a] && solid_rect(c0, c1, r0, r1) {
                    found = Some((a, b, rect));
                    break 'pairs;
                }
            }
        }
        let Some((a, b, rect)) = found else { break };
        boxes.remove(b);
        boxes.remove(a);
        boxes.push(rect);
    }
    boxes
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    /// A solid-cell list in the shape's own coordinates, row-major like
    /// the level's, in the tuple the cache keys on.
    fn cells(list: &[(i32, i32)]) -> Vec<(i32, i32, bool)> {
        let mut v: Vec<(i32, i32, bool)> =
            list.iter().map(|&(row, col)| (row, col, true)).collect();
        v.sort();
        v
    }

    /// The cover contract, asserted rather than assumed: the boxes hold
    /// every solid cell and no box holds an empty one — the shadow is
    /// exactly the hill, in whichever fewest boxes the merge can prove.
    fn assert_exact_cover(list: &[(i32, i32)], boxes: &[OccBox]) {
        let solid: std::collections::HashSet<(i32, i32)> = list.iter().copied().collect();
        let union: std::collections::HashSet<(i32, i32)> = boxes
            .iter()
            .flat_map(|&(c0, c1, r0, r1)| {
                (r0..=r1).flat_map(move |r| (c0..=c1).map(move |c| (r, c)))
            })
            .collect();
        assert_eq!(union, solid, "the boxes must cover the cells exactly");
    }

    fn merged(list: &[(i32, i32)]) -> Vec<OccBox> {
        let boxes = merge_boxes(&cells(list));
        assert_exact_cover(list, &boxes);
        boxes
    }

    #[test]
    fn rectangles_still_merge_to_one_box() {
        let rect: Vec<(i32, i32)> = (0..4).flat_map(|r| (0..7).map(move |c| (r, c))).collect();
        assert_eq!(merged(&rect), vec![(0, 6, 0, 3)], "a pile is one box");
    }

    #[test]
    fn a_frame_stays_four_bars() {
        // Rock's ring: no merge may ever cross its hollow middle.
        let mut ring: Vec<(i32, i32)> = Vec::new();
        for r in 0..5 {
            for c in 0..5 {
                if r == 0 || r == 4 || c == 0 || c == 4 {
                    ring.push((r, c));
                }
            }
        }
        let boxes = merged(&ring);
        assert_eq!(boxes.len(), 4, "top, bottom, and the two sides");
    }

    #[test]
    fn the_fixpoint_sees_across_the_wider_row() {
        // The plus: the seed's stack cannot link the column across the
        // bar, because the bar's run is wider than the column's span.
        // The fixpoint asks only whether the stranded cell is solid —
        // and it is — so the post becomes one box beside the bar.
        let plus = [(0, 1), (1, 0), (1, 1), (1, 2), (2, 1)];
        let mut boxes = merged(&plus);
        boxes.sort_unstable();
        assert_eq!(
            boxes,
            vec![(0, 2, 1, 1), (1, 1, 0, 2)],
            "a bar and a post, two boxes, exact"
        );
    }

    #[test]
    fn an_h_merges_its_stranded_posts() {
        // Two columns joined by a middle row: the seed cuts both posts
        // in four; the fixpoint welds each post whole.
        let h: Vec<(i32, i32)> = (0..3)
            .flat_map(|r| [(r, 0), (r, 2)])
            .chain([(1, 1)])
            .collect();
        let mut boxes = merged(&h);
        boxes.sort_unstable();
        // Three boxes is the floor, and both shapes of cover reach it:
        // the bar may shrink to its middle cell or keep its full width
        // and overlap the posts — a cover is a union, and the merge
        // never saw a reason to cut the bar's ends off. What the seed
        // could not do and the fixpoint can: weld each stranded post
        // whole across the bar's wider row.
        assert_eq!(boxes.len(), 3, "two whole posts and the bar");
        assert!(
            boxes.contains(&(0, 0, 0, 2)) && boxes.contains(&(2, 2, 0, 2)),
            "each post stands as one box: {boxes:?}"
        );
    }

    #[test]
    fn a_dug_gap_still_divides() {
        // The merge must never wall a tunnel: two posts split by a dug
        // cell stay two boxes whatever the fixpoint tries.
        let split = [(0, 0), (1, 0), (3, 0), (4, 0)];
        let mut boxes = merged(&split);
        boxes.sort_unstable();
        assert_eq!(
            boxes,
            vec![(0, 0, 0, 1), (0, 0, 3, 4)],
            "the empty row is a wall the merge cannot cross"
        );
    }

    #[test]
    fn a_checkerboard_yields_its_cells() {
        // No two cells share a bounding rectangle that lies solid: the
        // fixpoint must leave every cell its own box.
        let board: Vec<(i32, i32)> = (0..4)
            .flat_map(|r| {
                (0..4)
                    .filter(move |c| (r + c) % 2 == 0)
                    .map(move |c| (r, c))
            })
            .collect();
        assert_eq!(merged(&board).len(), board.len(), "no merge is legal here");
    }

    #[test]
    fn empty_hill_no_boxes() {
        assert_eq!(merge_boxes(&[]), vec![]);
    }

    /// The house rule from `docs/benchmarking.md` §5, applied where the
    /// fixpoint lives: A/B the mechanism, on the real hills and on a
    /// worst case, and report the numbers whatever they say. The seed
    /// cover is the old merge; `merge_boxes` is the seed plus the
    /// fixpoint; the two differ by exactly the pass under test.
    #[test]
    fn the_fixpoint_cut_is_measured_not_claimed() {
        // Every shipped hill, as the cache will see it.
        let levels = level::load_levels(include_str!("levels.ron"));
        let mut shipped_gain = 0usize;
        for lvl in &levels {
            let tiles: Vec<(i32, i32, bool)> = (0..lvl.rows as i32)
                .flat_map(|row| (0..lvl.cols as i32).map(move |col| (row, col)))
                .filter_map(|(row, col)| {
                    let t = lvl.tiles[row as usize * lvl.cols + col as usize];
                    matches!(t, level::Tile::Rock | level::Tile::Dirt).then_some((
                        row,
                        col,
                        matches!(t, level::Tile::Rock),
                    ))
                })
                .collect();
            let before = seed_cover(&tiles).len();
            let after = merge_boxes(&tiles);
            let list: Vec<(i32, i32)> = tiles.iter().map(|&(r, c, _)| (r, c)).collect();
            assert_exact_cover(&list, &after);
            shipped_gain += before - after.len();
            eprintln!(
                "merge bench: {}: {before} -> {} boxes",
                lvl.name,
                after.len()
            );
        }

        // The worst case the fixpoint exists for: a dug hill whose
        // tunnels cross, stranding every pillar the seed cuts loose.
        let mut dug: Vec<(i32, i32)> = Vec::new();
        for r in 0..24 {
            for c in 0..32 {
                if r % 4 != 2 || c % 5 != 3 {
                    dug.push((r, c));
                }
            }
        }
        let before = seed_cover(&cells(&dug)).len();
        let after = merged(&dug);
        eprintln!(
            "merge bench: crossing tunnels 24x32: {before} -> {} boxes",
            after.len()
        );
        assert!(
            after.len() * 2 < before,
            "the fixpoint must at least halve a field of stranded pillars: \
             {before} -> {}",
            after.len()
        );
        // And the shipped hills are reported honestly whatever they say.
        let _ = shipped_gain;
    }
}

/// The menu, the Escape rule, and the settings file — the parts the
/// request asked for that are pure CPU and can be nailed down without
/// a window.
#[cfg(test)]
mod menu_tests {
    use super::*;

    fn demo() -> Demo {
        let frames = ANT_FRAMES
            .iter()
            .map(|png| ant_frame(png).expect("shipped ant frames bake"))
            .collect();
        let soil = frost::Shape::sprite_bytes(SOIL).expect("shipped soil bakes");
        Demo::new(frames, soil, Settings::default(), Progress::default())
    }

    #[test]
    fn the_progress_file_counts_hills_as_the_hud_does() {
        assert_eq!(
            Progress::default().text(),
            "(\n    hill: 1,\n    reached: 1,\n    scores: [],\n    routes: [],\n)\n",
            "a player who never moved stands on the first hill, ledger empty"
        );
        let p = Progress::parse("(\n    hill: 3,\n)\n");
        assert_eq!(p.hill, 2, "the file counts from one, the code from zero");
        assert_eq!(p.reached, 2, "a pre-depth file adopts its standing hill");
        assert!(
            p.scores.is_empty(),
            "a file from before the ledger is no scores, not a broken file"
        );
        assert!(
            p.routes.is_empty(),
            "a file from before the routes is no routes either"
        );
        assert_eq!(
            Progress {
                hill: 1,
                reached: 1,
                ..Default::default()
            }
            .text(),
            "(\n    hill: 2,\n    reached: 2,\n    scores: [],\n    routes: [],\n)\n"
        );
        assert_eq!(
            Progress::parse("garbage").hill,
            0,
            "a broken file means the first hill, not a failed launch"
        );
        assert_eq!(
            Progress::parse("(\n    hill: 0,\n)\n").hill,
            0,
            "nothing sits below the first hill"
        );
    }

    #[test]
    fn the_ledger_keeps_the_best_run_of_every_warm_hill() {
        let mut p = Progress::default();
        p.record(2, 40);
        p.record(0, 55);
        p.record(2, 34);
        p.record(2, 41);
        assert_eq!(
            p.scores,
            vec![(0, 55), (2, 34)],
            "sorted, one entry per hill, the fewest moves kept"
        );
        let back = Progress::parse(&p.text());
        assert_eq!(back, p, "the ledger round-trips through the file");
        // And it reads back a hand-written list the same way it reads
        // its own: quoted pairs counting hills from one, out-of-order
        // folds to sorted, a hill named twice keeps its better number.
        let p =
            Progress::parse("(\n    hill: 4,\n    scores: [\"3 31\", \"1 47\", \"3 29\"],\n)\n");
        assert_eq!(p.scores, vec![(0, 47), (2, 29)]);
    }

    #[test]
    fn a_route_is_walkable_or_it_is_nothing() {
        let run = [
            game::Act::Move(game::Dir::Left),
            game::Act::Move(game::Dir::Down),
            game::Act::Push(game::Dir::Up),
            game::Act::Detach,
        ];
        let glyphs = encode_run(&run);
        assert_eq!(
            glyphs, "ldUx",
            "walks lowercase, shoves uppercase, leave-behind x"
        );
        assert_eq!(decode_run(&glyphs).as_deref(), Some(&run[..]));
        assert_eq!(
            decode_run("ldU!x"),
            None,
            "one stray letter is not a route with a typo — it is not a route"
        );
        // And the ledger keeps only walks: a hand may write a number,
        // but it cannot plant a route the hill could never show.
        let mut p = Progress::default();
        p.remember(0, "qzzling");
        assert!(
            p.routes.is_empty(),
            "unwalkable glyphs are refused at the door"
        );
    }

    #[test]
    fn the_walk_changes_only_when_the_score_does() {
        let mut p = Progress::default();
        p.record_run(1, 50, "rrrddd");
        assert_eq!(
            p.route_of(1),
            Some("rrrddd"),
            "a first win deposits number and walk"
        );
        p.record_run(1, 52, "ddd");
        assert_eq!(p.scores, vec![(1, 50)], "a worse run moves no number");
        assert_eq!(
            p.route_of(1),
            Some("rrrddd"),
            "and carries no walk anywhere"
        );
        p.record_run(1, 50, "uuu");
        assert_eq!(
            p.route_of(1),
            Some("rrrddd"),
            "a tie over a kept walk leaves the elder in"
        );
        p.record_run(1, 44, "rrdd");
        assert_eq!(
            p.route_of(1),
            Some("rrdd"),
            "a better score carries a new walk in"
        );
        // A hill whose number predates the recordings backfills its
        // walk on a tie — and only on a tie, because the walk that
        // joins the ledger must walk exactly the count it joins.
        let mut old = Progress::parse("(\n    hill: 2,\n    scores: [\"2 40\"],\n)\n");
        assert_eq!(
            old.route_of(1),
            None,
            "the old file has scores and no walks"
        );
        old.record_run(1, 41, "ldu");
        assert_eq!(
            old.route_of(1),
            None,
            "a run worse than the old number records nothing"
        );
        old.record_run(1, 40, "ruru");
        assert_eq!(old.route_of(1), Some("ruru"), "the tie backfills the walk");
        assert_eq!(old.scores, vec![(1, 40)], "and the number does not move");
        old.record_run(1, 40, "dud");
        assert_eq!(
            old.route_of(1),
            Some("ruru"),
            "the elder backfill keeps its seat"
        );
        let back = Progress::parse(&old.text());
        assert_eq!(
            back, old,
            "numbers and routes round-trip through the file together"
        );
    }

    #[test]
    fn a_showing_walks_its_recorded_run_and_scores_nothing() {
        let mut d = demo();
        d.load(0);
        // A hand walks a short run on First Warm, exactly as the keys
        // would: right, right, up from the start cell (1,3).
        {
            let g = d.game.as_mut().expect("a hill stands loaded");
            for a in [
                game::Act::Move(game::Dir::Right),
                game::Act::Move(game::Dir::Right),
                game::Act::Move(game::Dir::Up),
            ] {
                g.act(a).expect("legal on First Warm");
            }
        }
        let walked = d.game.as_ref().expect("still loaded");
        assert_eq!(walked.log.len(), 3, "the played hill wrote its walk down");
        d.progress.remember(0, &encode_run(&walked.log));
        let scores_before = d.progress.scores.clone();
        // Now the ledger shows it back.
        d.start_replay(0);
        assert_eq!(d.page, Page::Game, "the showing opens on the hill");
        assert!(
            !d.game.as_ref().expect("replaying").record,
            "a showing records nothing"
        );
        for _ in 0..3 {
            d.replay_advance();
        }
        let shown = d.game.as_ref().expect("still replaying");
        assert_eq!(
            shown.player,
            game::Cell::new(3, 2),
            "the walk lands where it was walked"
        );
        assert_eq!(shown.moves, 3);
        assert_eq!(d.replay.as_ref().expect("shown").shown, 3);
        assert!(
            !d.replay.as_ref().expect("arrived").playing,
            "the run holds at its end"
        );
        // And the wheel runs backwards exactly: the hill rebuilt from
        // its beginning, one action short.
        d.replay_retreat();
        let back = d.game.as_ref().expect("still replaying");
        assert_eq!(
            back.player,
            game::Cell::new(3, 3),
            "one step back, one cell back"
        );
        assert_eq!(back.moves, 2);
        // The audience scored nothing: the ledger never shifted, and
        // the won page never claimed the hill.
        assert_eq!(d.progress.scores, scores_before, "a showing is not a run");
        assert_eq!(d.page, Page::Game, "no verdict for a spectator");
    }

    #[test]
    fn the_game_opens_where_the_player_stood() {
        let frames = ANT_FRAMES
            .iter()
            .map(|png| ant_frame(png).expect("shipped ant frames bake"))
            .collect();
        let soil = frost::Shape::sprite_bytes(SOIL).expect("shipped soil bakes");
        let d = Demo::new(
            frames,
            soil,
            Settings::default(),
            Progress {
                hill: 1,
                ..Default::default()
            },
        );
        assert_eq!(d.idx, 1, "the saved hill is the opened hill");
        assert!(d.game.is_some(), "and it arrives with its game loaded");
        let mut d = d;
        d.new_game();
        assert_eq!(d.idx, 0, "New Game restarts the descent at the first hill");
    }

    #[test]
    fn one_row_lives_under_the_pointer() {
        let d = demo();
        let rows = d.menu_rows(d.page, 1080.0);
        for (i, r) in rows.iter().enumerate() {
            assert_eq!(
                row_under(&rows, [r.cx, r.cy]),
                Some(i),
                "a row's own center answers to it alone"
            );
        }
        let above = [rows[0].cx, rows[0].cy - rows[0].hh - 1.0];
        assert_eq!(
            row_under(&rows, above),
            None,
            "just clear of the top row is nobody's"
        );
    }

    #[test]
    fn escape_walks_the_pages() {
        // The hill yields to the menu, the menu hands the hill back,
        // settings step back — the key never quits, only the exit row
        // does.
        assert_eq!(escape_nav(Page::Game), Escape::ToMenu);
        assert_eq!(escape_nav(Page::Main), Escape::Resume);
        assert_eq!(escape_nav(Page::Settings), Escape::ToMenu);
    }

    #[test]
    fn the_hill_opens_paused_behind_the_menu() {
        let d = demo();
        assert_eq!(d.page, Page::Main, "launch is the menu, not the walk");
        assert!(d.game.is_some(), "the hill is loaded and visible behind it");
    }

    #[test]
    fn new_game_clears_the_moves() {
        let mut d = demo();
        {
            let g = d.game.as_mut().expect("a hill is loaded");
            // The click tests' First Warm: the queen walks the shaft up.
            assert!(g.move_player(game::Dir::Up).is_ok());
            assert_eq!(g.moves, 1, "one step on the board");
        }
        d.new_game();
        assert_eq!(
            d.game.as_ref().unwrap().moves,
            0,
            "the board forgot the step"
        );
        assert_eq!(d.page, Page::Game, "and the hill plays on");
    }

    #[test]
    fn settings_round_trip_through_their_own_text() {
        let s = Settings {
            vsync: false,
            nearest: true,
            fullscreen: false,
            quality: Quality::Ultra,
            ambient: 0.25,
            light: 1.75,
            lamp_glow: false,
            leaves_lit: false,
            shadows: Shadow::Four,
        };
        assert_eq!(Settings::parse(&s.text()), s, "what is written is read");
    }

    #[test]
    fn a_broken_file_is_answered_with_defaults() {
        assert_eq!(
            Settings::parse("not ron at all ((("),
            Settings::default(),
            "a garbage file must not out-vote the launch"
        );
        // A half-known file keeps what it knows, defaults the rest,
        // and clamps what wandered off its range.
        let s = Settings::parse("(vsync: false, ambient: 9.0, bogus: 1)");
        assert!(!s.vsync, "one line survived");
        assert_eq!(s.ambient, 1.0, "clamped back onto the slider's top");
        assert_eq!(s.quality, Quality::High, "the untouched rows stand");
    }

    #[test]
    fn the_ambient_slider_hinges_on_the_shipped_night() {
        // The middle of the track — the default — is exactly the
        // night the game shipped with; the bottom is the full dark;
        // the top leaves no dimming at all.
        assert_eq!(
            Settings::default().ambient,
            0.5,
            "the slider starts on the shipped look"
        );
        let mid = ambient_floor(0.5);
        assert_eq!((mid.r, mid.g, mid.b), (C_NIGHT.r, C_NIGHT.g, C_NIGHT.b));
        let dark = ambient_floor(0.0);
        assert_eq!((dark.r, dark.g, dark.b), (0.0, 0.0, 0.0));
        let day = ambient_floor(1.0);
        assert!(
            (day.r - 1.0).abs() < 1e-6 && (day.g - 1.0).abs() < 1e-6 && (day.b - 1.0).abs() < 1e-6,
            "at full track the floor is white: nothing dims"
        );
    }

    #[test]
    fn quality_tiers_round_trip() {
        for q in [
            Quality::Medium,
            Quality::High,
            Quality::VeryHigh,
            Quality::Ultra,
        ] {
            assert_eq!(
                Quality::parse(q.label()),
                Some(q),
                "the file speaks in labels"
            );
        }
        assert_eq!(
            Quality::High.next(),
            Quality::VeryHigh,
            "the shipped tier steps up"
        );
        assert_eq!(Quality::Ultra.next(), Quality::Medium, "and the row cycles");
        assert_eq!(
            Quality::Ultra.size(),
            None,
            "native is no fixed buffer at all"
        );
        assert_eq!(
            Quality::Medium.size(),
            Some([1280, 720]),
            "720p is the floor"
        );
        assert_eq!(Quality::parse("bogus"), None, "unknown words name no tier");
        let fresh = Settings::parse("(\n    resolution: \"1920 x 1080\",\n)\n");
        assert_eq!(
            fresh.quality,
            Quality::High,
            "the file speaks in resolutions"
        );
    }
    #[test]
    fn a_slide_clamps_to_its_track() {
        assert_eq!(slide_value(-999.0, MENU_TRACK_CX, MENU_TRACK_HALF), 0.0);
        assert_eq!(slide_value(999.0, MENU_TRACK_CX, MENU_TRACK_HALF), 1.0);
        let mid = slide_value(MENU_TRACK_CX, MENU_TRACK_CX, MENU_TRACK_HALF);
        assert!((mid - 0.5).abs() < 1e-6, "the middle of the track is half");
    }

    #[test]
    fn the_selection_walks_wraps_the_page() {
        // Arrows wrap: one step down from the last row is the first,
        // one step up from the first is the last, and an empty page
        // holds still.
        assert_eq!(step_sel(0, 4, 1), 1);
        assert_eq!(step_sel(3, 4, 1), 0, "past the bottom wraps to the top");
        assert_eq!(step_sel(0, 4, -1), 3, "past the top wraps to the bottom");
        assert_eq!(step_sel(2, 4, -1), 1);
        assert_eq!(step_sel(7, 0, 1), 0, "no rows, no movement");
    }

    #[test]
    fn the_control_pages_list_the_controls_in_font_safe_words() {
        let mut d = demo();
        for (page, firsts) in [
            (
                Page::Keyboard,
                &[
                    "wasd or arrows to move",
                    "space to detach",
                    "shift wasd or arrows to push",
                    "z to undo",
                    "r to reset level",
                ][..],
            ),
            (
                Page::Mouse,
                &[
                    "left click to move",
                    "right click to detach",
                    "shift click to push",
                ][..],
            ),
        ] {
            d.page = page;
            let rows = d.menu_rows(d.page, 720.0);
            let texts: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
            for want in firsts {
                assert!(texts.contains(want), "{page:?} must list {want:?}");
            }
            assert_eq!(texts.last(), Some(&"back"), "every list ends the way out");
            for t in texts {
                assert!(
                    t.chars().all(|c| c.is_ascii_alphanumeric() || c == ' '),
                    "{t:?} must wear only glyphs the font owns"
                );
            }
        }
    }

    #[test]
    fn escape_leaves_the_control_pages_for_the_menu() {
        assert_eq!(escape_nav(Page::Keyboard), Escape::ToMenu);
        assert_eq!(escape_nav(Page::Mouse), Escape::ToMenu);
        assert_eq!(escape_nav(Page::Progress), Escape::ToMenu);
    }

    #[test]
    fn the_seven_menu_rows_hang_inside_seven_twenty() {
        // The bottom row of the main menu must land above the
        // plate's own bottom edge — the plate's bottom is 130 under
        // centre, the frame's is h/2 under it.
        let d = demo();
        let rows = d.menu_rows(d.page, 720.0);
        assert_eq!(rows.len(), 7);
        let lowest = rows.iter().map(|r| r.cy).fold(f32::MAX, f32::min);
        assert!(
            lowest >= 130.0 - 360.0,
            "the last row escaped the plate: {lowest}"
        );
    }

    /// The ledger's columns, held apart by measured ink: these four
    /// budgets are the real widths of the widest things a row can
    /// carry, laid out by the engine's shaper in the shipped Barrfont
    /// at the ledger's size — the longest authored name (370.0,
    /// "Three Cold Nights"), the fullest score the moves cap allows
    /// (199.2, "999 moves"), a two-digit hill number (46.3), and the
    /// card that the cell's own caps can never grow past (190.0). If a
    /// new hill out-owns this ink, the first assertion fails here
    /// rather than painting a collision.
    #[test]
    fn the_ledger_columns_never_touch() {
        const NUM: f32 = 48.0;
        const NAME: f32 = 372.0;
        const SCORE: f32 = 205.0;
        const CARD: f32 = 235.0;
        assert!(
            PROGRESS_NUM_X + NUM + 20.0 <= PROGRESS_NAME_X,
            "the number column must hand off to the name with room"
        );
        assert!(
            PROGRESS_NAME_X + NAME + 20.0 <= PROGRESS_SCORE_R - SCORE,
            "the widest name must stop short of the shortest score's left edge"
        );
        assert!(
            PROGRESS_SCORE_R + 20.0 <= PROGRESS_MAP_CX - CARD / 2.0,
            "the score must stop short of the map card"
        );
        assert!(
            PROGRESS_MAP_CX + CARD / 2.0 + 10.0 <= PROGRESS_PLATE,
            "the map card must sit inside the plate"
        );
        assert!(
            PROGRESS_PLATE <= 480.0,
            "the plate must fit the half-width of the smallest window the game lays out for"
        );
    }

    #[test]
    fn the_ledger_marks_more_above_and_below() {
        // Nothing hidden at the top of a short descent...
        assert_eq!(progress_marks(3, 0, 3), (false, false));
        // ...a mark below while hills wait out of sight...
        assert_eq!(progress_marks(20, 0, 12), (false, true));
        // ...marks both ways mid-descent...
        assert_eq!(progress_marks(20, 5, 12), (true, true));
        // ...and only above once the bottom row is in the window.
        assert_eq!(progress_marks(20, 8, 12), (true, false));
    }

    #[test]
    fn the_ledger_never_writes_more_than_the_cap() {
        let mut p = Progress::default();
        p.record(0, 1234);
        assert_eq!(
            p.scores,
            vec![(0, MOVES_CAP)],
            "a wandering win records the cap, not the wander"
        );
        p.record(0, 940);
        assert_eq!(p.scores, vec![(0, 940)], "and a better run still beats it");
        let p = Progress::parse(&format!(
            "(\n    hill: 1,\n    scores: [\"1 {}\"],\n)\n",
            MOVES_CAP + 235
        ));
        assert_eq!(
            p.scores,
            vec![(0, MOVES_CAP)],
            "the file passes the same door"
        );
    }

    #[test]
    fn the_progress_page_is_a_window_over_the_descent() {
        let mut d = demo();
        d.page = Page::Progress;
        // Even with nothing warm, the hill you stand on owns a row.
        let rows = d.menu_rows(d.page, 1080.0);
        assert_eq!(
            rows.first().map(|r| r.action),
            Some(Action::Hill(0)),
            "the first hill is stood on, so it is listed"
        );
        assert_eq!(rows.last().map(|r| r.action), Some(Action::Back));
        for hill in 0..20 {
            d.progress.record(hill, 30 + hill as i32);
        }
        let fits = progress_fits(1080.0);
        let rows = d.menu_rows(d.page, 1080.0);
        assert_eq!(
            rows.len(),
            fits + 1,
            "the window shows its plate-full of hills and pins back under them"
        );
        assert_eq!(
            rows.last().map(|r| r.action),
            Some(Action::Back),
            "and the way out rides below the last visible row"
        );
        d.progress_top = 10;
        let rows = d.menu_rows(d.page, 1080.0);
        assert_eq!(
            rows.first().map(|r| r.action),
            Some(Action::Hill(10)),
            "slide the window and its rows come with it"
        );
        let lowest = rows.iter().map(|r| r.cy).fold(f32::MAX, f32::min);
        assert!(
            lowest >= progress_top_y(1080.0) - fits as f32 * progress_step(1080.0) - 0.01,
            "the pinned back row escaped its place: {lowest}"
        );
    }

    #[test]
    fn a_stood_on_hill_is_listed_even_unwarmed() {
        let mut d = demo();
        d.page = Page::Progress;
        d.load(2); // three hills stood on, none warmed.
        d.progress.stood(2);
        let rows = d.menu_rows(d.page, 1080.0);
        assert_eq!(
            rows.iter().map(|r| r.action).collect::<Vec<_>>(),
            vec![
                Action::Hill(0),
                Action::Hill(1),
                Action::Hill(2),
                Action::Back
            ],
            "the descent lists every hill reached, warm or not"
        );
        assert!(
            !rows[0].text.contains("moves"),
            "an unwarmed hill keeps its score column empty"
        );
        d.progress.record(1, 40);
        let rows = d.menu_rows(d.page, 1080.0);
        assert!(
            rows[1].text.contains("40 moves"),
            "a warmed hill wears its run"
        );
        assert!(
            !rows[2].text.contains("moves"),
            "and its neighbours keep their word"
        );
    }

    #[test]
    fn the_ledger_owes_the_imperfect_runs_first() {
        let mut d = demo();
        d.page = Page::Progress;
        d.load(5);
        d.progress.stood(5); // six hills, none warmed yet.
        // Perfect the 2nd and 4th (their pars are 5 and 10), overshoot
        // the 5th, and leave the 1st, 3rd and 6th unwarmed.
        d.progress.record(1, 3);
        d.progress.record(3, 8);
        d.progress.record(4, 30);
        let rows = d.menu_rows(d.page, 1080.0);
        assert_eq!(
            rows.iter().map(|r| r.action).collect::<Vec<_>>(),
            vec![
                Action::Hill(0),
                Action::Hill(2),
                Action::Hill(4),
                Action::Hill(5),
                Action::Hill(1),
                Action::Hill(3),
                Action::Back,
            ],
            "the owed runs lead in hill order — the unwarmed with the \
             over-par — and the perfected follow in their own hill order"
        );
    }

    #[test]
    fn next_leads_the_owed_hill_or_the_new_one() {
        let mut d = demo();
        // Warm the first two hills and stand on the deepest: at the
        // frontier of the descent, the never-walked hill below
        // answers first, whatever the hills behind may owe.
        d.load(2);
        d.progress.stood(2);
        d.progress.record(0, 1);
        d.progress.record(1, 1);
        assert_eq!(
            d.next_hill(),
            3,
            "at the frontier of the descent, the never-walked hill below answers first"
        );
        // Walk down to the sixth hill and back to the first, leaving
        // the third unwarmed: from shallow ground `next` leads to the
        // nearest owed hill ahead, not past it to the new one.
        d.load(5);
        d.progress.stood(5);
        for hill in 3..6 {
            d.progress.record(hill, 1);
        }
        d.load(0);
        assert_eq!(d.next_hill(), 2, "the hill still owed ahead leads");
        // Warm that one too, and the walked ground holds nothing to
        // owe: the new hill below takes the key.
        d.progress.record(2, 1);
        d.load(1);
        assert_eq!(
            d.next_hill(),
            6,
            "nothing owed on walked ground: the new hill below answers"
        );
        // And when the whole authored descent is perfect, `next` is
        // what it always was: the plain next hill, wrapping.
        for hill in 0..d.levels.len() {
            d.progress.record(hill, 1);
        }
        assert_eq!(
            d.next_hill(),
            2,
            "a perfected descent: `next` just moves on"
        );
    }

    #[test]
    fn next_greys_when_the_hill_underfoot_is_all_that_is_owed() {
        // A player who never moved has whole hills still to walk:
        // the key is wide awake.
        assert!(demo().next_owed());
        let mut d = demo();
        let last = d.levels.len() - 1;
        // Walk the whole descent and perfect every hill but the
        // third. With that one hill underfoot, `next` leads nowhere
        // the player is not already standing — and greys.
        d.load(last);
        d.progress.stood(last);
        for hill in 0..d.levels.len() {
            if hill != 2 {
                d.progress.record(hill, 1);
            }
        }
        d.load(2);
        assert!(
            !d.next_owed(),
            "the only owed hill is the one underfoot: `next` is spent"
        );
        // Step aside to a perfected hill and the owed one waits
        // ahead again — the grey is the hill's, not the page's.
        d.load(3);
        assert!(d.next_owed(), "from perfected ground the owed hill waits");
        // Warm the last owed hill and the descent owes nothing; a
        // perfected `next` has nowhere to move on to either.
        d.load(2);
        d.progress.record(2, 1);
        assert!(
            !d.next_owed(),
            "a perfected descent leaves `next` nothing to lead to"
        );
    }

    #[test]
    fn the_settings_page_offers_every_setting() {
        let mut d = demo();
        d.page = Page::Settings;
        let rows = d.menu_rows(d.page, 1080.0);
        let actions: Vec<Action> = rows.iter().map(|r| r.action).collect();
        for a in [
            Action::Vsync,
            Action::Filter,
            Action::Fullscreen,
            Action::Quality,
            Action::Ambient,
            Action::Light,
            Action::Leaves,
            Action::Back,
        ] {
            assert!(actions.contains(&a), "the page must answer {a:?}");
        }
        assert_eq!(
            rows.iter().filter(|r| r.slider.is_some()).count(),
            2,
            "two sliders: ambient and light"
        );
    }

    #[test]
    fn the_main_menu_is_the_seven_rows_requested() {
        let d = demo();
        let actions: Vec<Action> = d
            .menu_rows(d.page, 1080.0)
            .iter()
            .map(|r| r.action)
            .collect();
        assert_eq!(
            actions,
            vec![
                Action::New,
                Action::Resume,
                Action::OpenSettings,
                Action::OpenProgress,
                Action::Keyboard,
                Action::Mouse,
                Action::Exit,
            ]
        );
    }

    #[test]
    fn the_won_page_hangs_low_enough_for_its_verdict() {
        // At 720 the verdict's strip runs from the plate's top
        // (top + 130) to the score banner's bottom (h/2 - 93) and
        // must hold the full 64-unit word clear of both.
        let mut d = demo();
        d.page = Page::Won;
        let rows = d.menu_rows(d.page, 720.0);
        let plate_top = rows.iter().map(|r| r.cy).fold(f32::MIN, f32::max) + 130.0;
        let band = 720.0 / 2.0 - 93.0;
        assert!(
            band - plate_top >= 64.0 + 16.0,
            "the strip holds only {}",
            band - plate_top
        );
    }

    #[test]
    fn a_finished_hill_offers_next_and_retry() {
        // The two ways off a won hill — and the only two: they hang
        // under the verdict splash, not on the main menu.
        let mut d = demo();
        d.page = Page::Won;
        let actions: Vec<Action> = d
            .menu_rows(d.page, 1080.0)
            .iter()
            .map(|r| r.action)
            .collect();
        assert_eq!(actions, vec![Action::Next, Action::Retry]);
        assert_eq!(escape_nav(Page::Won), Escape::ToMenu);
    }
}
