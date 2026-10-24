//! Ember Hill — a jam puzzle game on FROST (theme: *autumn* / *to keep warm*).
//!
//! Shepherd a growing trail of ants through a side-view ant hill. Walking
//! onto a stranded ant recruits it to your trail's tail; only the tail can
//! be released, and it only settles on its own colored bed — so the trail
//! is a stack, and the tunnels are the shuffle device. Every ant asleep
//! in its bed, yourself included, before the first frost.
//!
//! Run: `RUST_LOG=info cargo run -p antifrost` — add
//! `-- --vsync off` to uncap the frame rate. Vsync starts on: the
//! present is refresh-synced (`Fifo`), which is what lets the display
//! engage VRR; off presents with tearing (`Immediate`) and bypasses it.
//! Rules: `antifrost/DESIGN.md`

#[cfg(test)]
use antifrost::settings::Quality;
use antifrost::{
    LEVELS, game,
    ledger::{MOVES_CAP, Progress},
    level,
    route::{Replay, decode_run, encode_run},
    settings::{Settings, Shadow},
};
use frost::{
    Color, Context, KeyCode, LightLayers, MouseButton, Particle, ParticleShape, ParticleSystem,
    Process, Rng, Scene, SceneNode, SpriteFilter, Tile, Transform,
};
#[cfg(feature = "diag")]
use frost::{Diagnostics, DiagnosticsFlags};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// How the hill launches: the logging shapes, the beat sentinel's
/// clock, and the native command line.
mod cli;
pub(crate) use cli::*;
/// The school hills' lessons: the scripts, the tour state machine,
/// and the marks the rail shows while a card is open.
mod lessons;
pub(crate) use lessons::*;
/// The pages the hill opens onto and the menu they are built of.
mod menu;
pub(crate) use menu::*;
/// The hill as it breathes: animation, weather, ants, and the
/// verdict.
mod world;
pub(crate) use world::*;
/// The window's shape and the desktop's answer to it.
mod window;
pub(crate) use window::*;
/// The hill's dug geometry: the dig watch and the occluder merge,
/// pure cell math with its cover tests beside it.
mod shadow;
pub(crate) use shadow::*;

/// The hill's own voice: Barrfont for every word the game draws —
/// notes, HUD, win screen, the sleepy "z"s.
const GAME_FONT: &[u8] = include_bytes!("../assets/fonts/Barrfont-Regular.ttf");
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

/// The one layer the hill's tile occluders shadow. Receivers listen on it
/// by choice: only the far background joins the set, so the merged cell
/// boxes darken the sky behind the hill while the tiles themselves — lit on
/// plain layer 0 like everything else — never throw a shadow on each other
/// or on an ant.
const BG_SHADOW_LAYER: LightLayers = LightLayers::layer(1);

/// Fits the hill's node pool to the frame: grows with plain defaults,
/// shrinks by truncation — and sweeps every listener the surviving
/// nodes carry back to the default set. The pool survives hills, and
/// so do the fields its roles set: the deep parallax sheet listens on
/// [`BG_SHADOW_LAYER`], and it sits at the pool's last slot, which the
/// next — larger — hill hands to some other role. A pooled node that
/// kept that listener would draw tile shadows onto an ant shell or a
/// bed, surfaces the hill shadows nothing; the sweep is where a
/// listener learns it no longer listens.
fn fit_pool(root: &mut SceneNode, slots: usize) {
    while root.children.len() < slots {
        root.children.push(Box::new(SceneNode::default()));
    }
    root.children.truncate(slots);
    for node in root.children.iter_mut() {
        node.lit_layers = LightLayers::default();
    }
}

/// Put one node's light-bearing fields back to rest. Emission and
/// glow are the two fields a band can wear without every band
/// writing them — the ants' embers and the dig's fx are the wearers —
/// so a block dressed over a slot that last held a worn node would
/// draw light nobody asked for. Every other field belongs to whoever
/// dressed the node last, and the wash asks nothing of it.
fn rest_light(node: &mut SceneNode) {
    node.emission = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    node.glow = None;
}

/// The hill-threshold wash: bands slide across the pool as the
/// hill's counts change, so a node that wore an ember or a dissolve
/// last hill can stand under the backdrop today, where no block
/// writes the light fields. Once per swap the pool comes back to rest
/// before any band re-dresses it (see [`Demo::pool_reset`]).
fn wash_pool(root: &mut SceneNode) {
    for node in root.children.iter_mut() {
        rest_light(node);
    }
}

/// The backdrop: one soil texture stretched cover-first across the window,
/// under every tile. The hairline gaps between tiles and the void past the
/// hill's edge read as earth instead of as a screen.
const SOIL: &[u8] = include_bytes!("../assets/sprites/soil.png");

/// The dig's dissolve, borrowed from `frost_shaders`: a game program
/// the engine compiles behind the lighting prelude exactly like its
/// own lit kinds, hung on the soil sprite wherever a dug tile still
/// has dirt to lose (see [`Demo::digs`], and `frost_shaders::DISSOLVE`
/// for the three floats of its contract). The shelf keeps the WGSL and
/// signs its own contract test there;
/// `cargo run -p frost_shaders --example dissolve` paints the same eat
/// on a plain tile of soil. The engine's live-reload watches its own
/// shader directory, not a crate's `include_str!` — a change there is
/// a rebuild, the honest trade for a game owning its effects through a
/// dependency instead of a folder.
const DISSOLVE: &str = frost_shaders::DISSOLVE;

/// The slab the menu panels are cut from: one 16:9 band taken from
/// the garden's granite photo (the user's own `stone.png`, 3024×4032)
/// and baked down — cropped at 1440 wide, then darkened to a third in
/// the same offline pass, so a panel made of it reads as stone in a
/// dim room and never competes with the words on it. The face keeps
/// the moss and the lichen: an anthill hung on a real stone should
/// say so.
const STONE: &[u8] = include_bytes!("../assets/sprites/stone_menu.png");

/// The bed, the user's own sprite: laid square on the bed's cell,
/// tinted nothing, lit like everything else that lives in the hill.
const BED: &[u8] = include_bytes!("../assets/sprites/bed.png");

const TILE: f32 = 44.0;

/// World → screen magnification. One number scales the whole hill: cells,
/// beds, ants — everything that lives in the camera's space — while line
/// widths, the HUD, the frost rim, and the dead zone stay in screen
/// pixels. `4.0` fills the window with about five cells across.
const ZOOM: f32 = 4.0;
const HOP: f32 = 0.26; // seconds, player pixel glide
/// A dig step is the heavier move: the ant pushes *through* the hill
/// instead of around it, so she lands at quarter speed — and the soil
/// she is passing through dissolves across exactly that longer step,
/// which is the whole contract of the pair.
const DIG_HOP: f32 = HOP * 4.0;
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
    include_bytes!("../assets/sprites/leaf1.png"),
    include_bytes!("../assets/sprites/leaf2.png"),
    include_bytes!("../assets/sprites/leaf3.png"),
    include_bytes!("../assets/sprites/leaf4.png"),
    include_bytes!("../assets/sprites/leaf5.png"),
    include_bytes!("../assets/sprites/leaf6.png"),
    include_bytes!("../assets/sprites/leaf7.png"),
    include_bytes!("../assets/sprites/leaf8.png"),
    include_bytes!("../assets/sprites/leaf9.png"),
    include_bytes!("../assets/sprites/leaf10.png"),
    include_bytes!("../assets/sprites/leaf11.png"),
    include_bytes!("../assets/sprites/snow_flake.png"),
];

/// The snowflake's slot in `LEAVES` — never dealt as a leaf.
const SNOW_SHEET: usize = LEAVES.len() - 1;

/// The hand-tailored ant — four right-facing frames of a walk cycle, drawn
/// top-down like the board reads it. The sheets are large (~970 px wide),
/// so they are baked down before upload, not scaled at draw time.
const ANT_FRAMES: [&[u8]; 4] = [
    include_bytes!("../assets/sprites/ant1.png"),
    include_bytes!("../assets/sprites/ant2.png"),
    include_bytes!("../assets/sprites/ant3.png"),
    include_bytes!("../assets/sprites/ant4.png"),
];

/// The same four frames as pure silhouettes: every pixel that meets
/// the eye at all is opaque black, so a painted shadow can be
/// uniform ink at its own alpha instead of a tinted copy whose
/// density wobbles with the art's own shading.
const ANT_STENCILS: [&[u8]; 4] = [
    include_bytes!("../assets/sprites/ant1_stencil.png"),
    include_bytes!("../assets/sprites/ant2_stencil.png"),
    include_bytes!("../assets/sprites/ant3_stencil.png"),
    include_bytes!("../assets/sprites/ant4_stencil.png"),
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

/// The painted shadow every ant wears: its own stencil — the walk
/// frame with every pixel that meets the eye poured full black —
/// laid pure black, a breath shy of its full weight, beside it.
/// The queen had one first, a tinted copy of the art at full
/// weight; the stencils evened the whole line, and then the line
/// stopped asking the ground for half measures.
const ANT_SHADOW: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.85,
};

/// How far the shadow sits down-right of its ant: two under a
/// walking ant, one under a sleeping one — measured in world
/// pixels, so the lean rides `ZOOM` like the ant's own geometry
/// and keeps its share of the ant at any window size. The shadow
/// lies down when the ant does.
const SHADOW_LEAN_AWAKE: f32 = 2.0;
const SHADOW_LEAN_ASLEEP: f32 = 1.0;

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
    /// The same frames as stencils — pure silhouettes — which every
    /// ant's painted shadow is drawn from.
    ant_stencils: Vec<frost::Shape>,
    /// The soil backdrop, stretched across the window below the board.
    soil: frost::Shape,
    /// The hill's solids draw as one tile map: this atlas — the soil
    /// texture itself, a single cell, the whole sheet — is shared by
    /// every wall and dirt placement, instanced in one draw call.
    tile_atlas: frost::Shape,
    /// The soil sprite wearing the dissolve program: the same pixels
    /// and the same GPU texture as the atlas's own sheet, painted by
    /// the game's shader instead of the engine's sprite one. Each
    /// live dig draws one quad of this at its cell.
    dissolve: frost::Shape,
    /// The dig watch: every dirt cell the player has eaten since the
    /// last frame's picture, riding out its hop. `(col, row, seconds
    /// left, seed)` — the countdown starts the frame the move is
    /// issued and reaches zero the frame the step lands, so the tile
    /// is gone exactly when the player is. Dirt restored (an undo)
    /// cancels its dig mid-bite; `load` clears the whole ledger, or a
    /// hill swap would read as a field of fresh digs.
    digs: Vec<(i32, i32, f32, f32)>,
    /// The dirt the watch last saw, and the count of it the hill
    /// started with — the pool's ceiling on simultaneous digs, since
    /// one dig needs one dirt cell and dirt is only ever lost.
    dirt_key: Vec<(i32, i32)>,
    dig_cap: usize,
    /// Set by `load`, spent by the next draw: a hill swap moves every
    /// pool band — the tail can slide down into the last hill's dig
    /// territory, whose nodes wear emission and glow the backdrop
    /// blocks never write. One wash of the pool per swap puts those
    /// two fields back to rest before any band re-dresses them.
    pool_reset: bool,
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
    /// Has the cold tour said its word about the cold? Armed at
    /// par, spent above it, and unarmed whenever the walk falls
    /// back to par (an undo, a restart) or the tour itself ends.
    cold_noted: bool,
    /// The win tick marked this hill as the tour's finish: the
    /// won page is withheld, and when the praise splash fades,
    /// the answer goes up (`better_t`) and the hill restarts.
    tour_pending: bool,
    /// Seconds left on the centred answer; it restarts the hill
    /// as it runs out.
    better_t: f32,
    /// The oops deck has had its turn: the forced mistake was made
    /// and the Z took it back, so the cold tour may deal. Reset
    /// every visit — a restarted hill may humble itself again.
    mistake_shown: bool,
    /// Seconds left on the big admission.
    mistake_t: f32,
    /// The standing mistake has been taken in: the admission is
    /// said once per mistake, not once per frame beneath it.
    mistake_seen: bool,
    /// The same courtesy for the double disaster standing beneath
    /// the undo's stage.
    disaster_seen: bool,
    /// The oops deck is done with this hill for good: the double
    /// disaster was restarted out of, so the cold tour takes up
    /// the fresh hill — the deck that forces mistakes never
    /// re-baits itself again.
    oops_retired: bool,
    /// The cheap dig's par win owes the hill its send-off: pending
    /// behind the splash, then hanging at the centre, then — and
    /// only then — the won page.
    send_pending: bool,
    send_t: f32,
    /// The send-off is said once: encouragement repeated is
    /// nagging.
    send_done: bool,
    note_t: f32,
    frost_t: f32,
    frost_bit: bool,
    /// The window/tab name is stamped once, on the first frame that has
    /// a window to name.
    title_set: bool,
    was: HashMap<KeyCode, bool>,
    was_mouse: HashMap<MouseButton, bool>,
    /// The direction the first gamepad was aimed last frame — the
    /// stick and the d-pad share one memory, so an aimed pad fires
    /// one step, the same law as a key's edge.
    pad_dir: Option<game::Dir>,
    /// The last frame's face of the pad's watched buttons — Start,
    /// Cross, Circle — the edge memory [`Demo::pad_pressed`] reads
    /// and renews, for the buttons no aim can speak.
    pad_was: HashMap<frost::Button, bool>,
    /// Click-walking: the directions left to walk toward the last
    /// clicked cell, one issued per completed glide.
    route: Vec<game::Dir>,
    /// A best run being shown rather than played: `Some` while a hill
    /// replays itself, with its own keys and its own pace.
    replay: Option<Replay>,
    /// The game's one source of randomness: the leaves' deal, the
    /// zs' jitter, a splash's flare, the wind's gusts — whatever
    /// wants a number without a reason. Seeded from the clock, so
    /// no two runs drift the same way.
    rng: Rng,
    /// The leaf deck, its clock, and how much of it the frost has
    /// called into the air. See [`Weather`].
    weather: Weather,
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
    /// The menu's own memory — its highlight, its scroll, its held
    /// arrows, its drag, its breathing overlay — grouped off the
    /// struct. See [`MenuNav`].
    menu: MenuNav,
    /// The live ledger: the disc's scores, plus every hill won since
    /// the game opened. Written when a hill moves and when a hill is
    /// won — the same law the file keeps from the outside.
    progress: Progress,
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
    /// The verdict word and its bright, kept after the splash has
    /// spun itself out: the won panel reads it in the strip above the
    /// plate.
    verdict: Option<(String, Color)>,
    /// The window's claim and the desktop's answer, kept together:
    /// the first-frame policy ask, the standing tier, the queued
    /// move, the fullscreen drift, and the resize probe. See
    /// [`WindowPolicy`].
    window: WindowPolicy,
}

impl Demo {
    fn new(
        ant_frames: Vec<frost::Shape>,
        ant_stencils: Vec<frost::Shape>,
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
            ant_stencils,
            dissolve: soil.clone().with_program(DISSOLVE),
            soil,
            tile_atlas,
            digs: Vec::new(),
            dirt_key: Vec::new(),
            dig_cap: 0,
            pool_reset: false,
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
            cold_noted: false,
            tour_pending: false,
            better_t: 0.0,
            mistake_shown: false,
            mistake_t: 0.0,
            mistake_seen: false,
            disaster_seen: false,
            oops_retired: false,
            send_pending: false,
            send_t: 0.0,
            send_done: false,
            note_t: 0.0,
            frost_t: 0.0,
            frost_bit: false,
            title_set: false,
            was: HashMap::new(),
            was_mouse: HashMap::new(),
            pad_dir: None,
            pad_was: HashMap::new(),
            route: Vec::new(),
            replay: None,
            rng: Rng::default(),
            weather: Weather {
                leaves: Vec::new(),
                seeded: false,
                active: N_LEAVES,
                t: 0.0,
            },
            occ_key: Vec::new(),
            occ_boxes: Vec::new(),
            settings,
            page: Page::Main,
            menu: MenuNav {
                nav_timer: HashMap::new(),
                pad_row: None,
                pad_repeat: 0.0,
                drag: None,
                sel: 0,
                last_pointer: None,
                progress_top: 0,
                alpha: 0.0,
                page: Page::Main,
            },
            progress,
            last_beat: 0.0,
            minimap_atlas: Arc::from([255u8, 255, 255, 255].as_slice()),
            bench: None,
            bench_t: 0.0,
            bench_frames: 0,
            verdict: None,
            window: WindowPolicy {
                pending: true,
                probe: None,
                expect: None,
                drift: 0,
                move_queued: false,
            },
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
        // The dig watch starts clean on a new hill: no digs carry
        // over, and the digest is re-taken, because the last hill's
        // dirt vanishing from the ledger is not a thousand fresh
        // holes opening under the player's feet. The pool's ceiling
        // for live digs is this hill's own dirt count — digs are only
        // ever made from dirt, and dirt is only ever lost.
        self.digs.clear();
        self.dirt_key = self.game.as_ref().map(dirt_cells).unwrap_or_default();
        self.dig_cap = self.dirt_key.len();
        self.pool_reset = true;
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
        // And the hill re-deals its own humility with the rest: a
        // fresh visit means a fresh mistake to make and undo.
        self.mistake_shown = false;
        self.mistake_t = 0.0;
        self.mistake_seen = false;
        self.send_pending = false;
        self.send_t = 0.0;
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
    /// One frame of the pad read as a step: the d-pad speaks first,
    /// then the left stick past its dead zone, its dominant axis the
    /// walked way (`y` positive is up, the sign `pad_action` turns
    /// the raw axis into). No aim, no step. Pure, so the mapping is
    /// nailed by tests, not thumbs.
    fn pad_step(dpad: [bool; 4], x: f32, y: f32) -> Option<game::Dir> {
        let [up, down, left, right] = dpad;
        // A d-pad held two ways is no verb: opposing holds cancel,
        // its diagonals aim nowhere the hill would walk.
        let vy = i8::from(up) - i8::from(down);
        let vx = i8::from(right) - i8::from(left);
        if vy != 0 && vx != 0 {
            return None;
        }
        if vy != 0 {
            return Some(if vy > 0 {
                game::Dir::Up
            } else {
                game::Dir::Down
            });
        }
        if vx != 0 {
            return Some(if vx > 0 {
                game::Dir::Right
            } else {
                game::Dir::Left
            });
        }
        if x.abs().max(y.abs()) < 0.5 {
            return None;
        }
        if x.abs() >= y.abs() {
            Some(if x > 0.0 {
                game::Dir::Right
            } else {
                game::Dir::Left
            })
        } else {
            Some(if y > 0.0 {
                game::Dir::Up
            } else {
                game::Dir::Down
            })
        }
    }

    /// Where the first gamepad aims this frame: the d-pad speaks
    /// first, then the left stick, in the hill's own sign. `None`
    /// is rest. The one pad read — the hill's walk and the menu's
    /// rows aim by the same hand.
    fn pad_aim(&self, ctx: &Context) -> Option<game::Dir> {
        let pad = ctx.gamepads().next()?;
        let held = |b: frost::Button| pad.is_pressed(b);
        let dpad = [
            held(frost::Button::DPadUp),
            held(frost::Button::DPadDown),
            held(frost::Button::DPadLeft),
            held(frost::Button::DPadRight),
        ];
        let axis = |a: frost::Axis| pad.axis_data(a).map(|d| d.value()).unwrap_or(0.0);
        let x = axis(frost::Axis::LeftStickX);
        let raw_y = axis(frost::Axis::LeftStickY);
        // gilrs hands over each backend's native sign: XInput calls
        // up positive, the evdev-style macOS and Linux backends call
        // down positive. The hill thinks in ups, so the evdev sign
        // turns around at the door.
        let y = if cfg!(target_os = "windows") {
            raw_y
        } else {
            -raw_y
        };
        Self::pad_step(dpad, x, y)
    }

    /// A gamepad button's press edge: down this frame, up last
    /// frame. Watched buttons should be read every frame — a frame
    /// that short-circuits past the read can return the next as a
    /// phantom press.
    fn pad_pressed(&mut self, ctx: &Context, b: frost::Button) -> bool {
        let down = ctx.gamepads().next().is_some_and(|p| p.is_pressed(b));
        down && !self.pad_was.insert(b, down).unwrap_or(false)
    }

    /// The pad's menu intents for one frame: (up, down, left, right).
    /// Vertical follows the keys' hold law — a new aim answers at
    /// once, a held one keeps stepping on the nav beat; horizontal
    /// fires once per aim, the arrow-nudge law a slider row lives by.
    fn pad_nav(&mut self, ctx: &mut Context, dt: f32) -> (bool, bool, bool, bool) {
        let d = self.pad_aim(ctx);
        let (fresh, on_beat) = match d {
            None => {
                self.menu.pad_row = None;
                (false, false)
            }
            Some(dir) => {
                if Some(dir) != self.menu.pad_row {
                    self.menu.pad_row = Some(dir);
                    self.menu.pad_repeat = NAV_HOLD;
                    (true, false)
                } else {
                    (false, key_repeat(&mut self.menu.pad_repeat, dt))
                }
            }
        };
        let step = fresh || on_beat;
        let turn = |want| d == Some(want);
        (
            step && turn(game::Dir::Up),
            step && turn(game::Dir::Down),
            fresh && turn(game::Dir::Left),
            fresh && turn(game::Dir::Right),
        )
    }

    /// The gamepad's step this frame, if a new one: an aimed pad
    /// fires once, like a key's press edge — and holding the right
    /// trigger (`rt`, and its analog deep-press `r2`) turns the step
    /// into a shove, the pad's own Shift.
    fn pad_action(&mut self, ctx: &Context) -> Option<(game::Dir, bool)> {
        let now = self.pad_aim(ctx);
        let edge = now.filter(|d| Some(*d) != self.pad_dir);
        self.pad_dir = now;
        let push = ctx.gamepads().next().is_some_and(|p| {
            p.is_pressed(frost::Button::RightTrigger) || p.is_pressed(frost::Button::RightTrigger2)
        });
        edge.map(|d| (d, push))
    }

    fn pressed(&mut self, ctx: &Context, key: KeyCode) -> bool {
        let down = ctx.key_down(key);
        let edge = down && !*self.was.entry(key).or_insert(false);
        self.was.insert(key, down);
        edge
    }

    /// The menu walk's key test, for the two arrows: the press itself
    /// answers at once, exactly as `pressed` would, and a key held
    /// down keeps answering — one step per beat after a first delay,
    /// the way a held key scrolls a list anywhere else in the world.
    /// Every other key in the game keeps its plain rising edge; the
    /// repeat belongs to the walk alone.
    fn nav_held(&mut self, ctx: &Context, dt: f32, key: KeyCode) -> bool {
        if self.pressed(ctx, key) {
            self.menu.nav_timer.insert(key, NAV_HOLD);
            return true;
        }
        if !ctx.key_down(key) {
            return false;
        }
        self.menu
            .nav_timer
            .get_mut(&key)
            .is_some_and(|left| key_repeat(left, dt))
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
        // The dig watch, read before the glide: the glide needs to
        // know whether the step it is about to ride ate dirt, because
        // a dig lands at half speed — and the digest is born on this
        // very frame, the frame that issued the move, counting down
        // over the same doubled length. Only the player's move eats
        // dirt, so only her move starts a digest; dirt restored by an
        // undo cancels a running one mid-bite, the tile standing as
        // the eat stops.
        let now = dirt_cells(game);
        let player = (game.player.col, game.player.row);
        let dug_here = self.dirt_key.contains(&player) && !now.contains(&player);
        watch_digs(&mut self.digs, &self.dirt_key, &now, dt);
        self.dirt_key = now;
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
            self.player_hop.to(
                self.player_at,
                want,
                self.face,
                exit,
                if dug_here { DIG_HOP } else { HOP },
            );
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
            self.note_t = 3.4;
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
            self.note = "it is getting cold you can do better".to_string();
            self.note_t = 3.4;
        }
        let target = match self.game.as_ref() {
            Some(g) if g.level.par > 0 => {
                ((g.moves as f32 - g.level.par as f32) / g.level.par as f32).clamp(0.0, 1.0)
            }
            _ => 0.0,
        };
        self.frost_t += (target - self.frost_t) * (1.0 - (-(dt / 1.2)).exp());
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
                34.0,
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
            let seat = if self.note_t > 0.0 { 64.0 } else { 26.0 };
            ctx.text(
                0.0,
                -h / 2.0 + seat,
                &self.font,
                line,
                34.0,
                600.0,
                C_NOTE,
                UI + 1.0,
            );
        }
        // School hills keep their lesson open while the walk follows
        // the script: a mark over the queen, a word at the foot.
        // Elsewhere, and off the line, `lesson` is silent — except
        // the sulking hills, whose words have just faded: the first
        // shows the undo, pointing back the way the wrong step
        // came; the second, the rewind — two chevrons stepped back,
        // because one undo will not be enough this time.
        if let Some(card) = self.lesson() {
            self.draw_lesson(ctx, card);
        } else if self.mistake_standing() && self.mistake_t <= 0.0 {
            self.draw_marks(ctx, face_of(game::Dir::Left), 1, UNDO_WORD);
        } else if self.disaster_standing() && self.mistake_t <= 0.0 {
            self.draw_marks(ctx, face_of(game::Dir::Left), 2, RESTART_WORD);
        }
    }

    /// New Game: the descent restarts at the first hill and the
    /// ledger returns to its empty default — no scores, no routes,
    /// no depth — so the save starts its record from here. The
    /// settings live under their own name and stay untouched. Every
    /// stat the game keeps lives inside `Game::start`'s fresh state,
    /// and `load` already sweeps the extras the demo layers on top —
    /// queued steps, drifting zs, the frost's claim, verdicts; the
    /// caller owns when the disc learns of the restarted ledger.
    /// Then the page steps aside and the hill plays.
    fn new_game(&mut self) {
        self.progress = Progress::default();
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
        // The rail walks the pointer's route too: a first step the
        // open card does not name never starts — no turn, no hop,
        // just the gate and its word. A route that follows the card
        // walks until it doesn't, and parts there.
        if !self.rail_opens(game::Act::Move(self.route[0])) {
            self.route.clear();
            self.note = RAIL_NOTE.to_string();
            self.note_t = 2.6;
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
            self.note_t = 3.6;
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
            self.menu.page = self.page;
        }
        self.menu.alpha += (target - self.menu.alpha) * (1.0 - (-dt / 0.15).exp());
        if (self.menu.alpha - target).abs() < 0.002 {
            self.menu.alpha = target;
        }
        if self.window.pending {
            self.window.pending = false;
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
        self.watch_window(ctx, dt);
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
        // Options is the pad's Escape — it opens the menu from the
        // hill, steps off every page, and walks back out of a
        // showing, exactly as the key does; Circle joins as the
        // menus' own back, silent on the hill where Escape means
        // pause. The three edges are read before the test so no
        // short-circuit can strand one mid-air.
        let esc = self.pressed(ctx, KeyCode::Escape);
        let pad_esc = self.pad_pressed(ctx, frost::Button::Start);
        let pad_back = self.pad_pressed(ctx, frost::Button::East);
        if esc || pad_esc || (self.page != Page::Game && pad_back) {
            if self.replay.is_some() {
                // Stepping off a showing returns to the ledger it
                // came from, and the hill is wiped back to its own
                // beginning: nobody resumes a run they only watched.
                let hill = self.idx;
                self.replay = None;
                self.load(hill);
                self.page = Page::Progress;
                self.menu.drag = None;
            } else {
                match escape_nav(self.page) {
                    Escape::ToMenu => {
                        self.page = Page::Main;
                        self.menu.drag = None;
                        self.menu.sel = 0;
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
            // The pad walks the keys' road: the same gate, the same
            // shove, the same refusals — its step only stands where
            // no key answered, and its shoulders are its Shift.
            let action = action.or_else(|| self.pad_action(ctx));
            let detach = self.pressed(ctx, KeyCode::Space);
            let click_left = self.pressed_mouse(ctx, MouseButton::Left);
            let click_right = self.pressed_mouse(ctx, MouseButton::Right);
            // Right-click detaches; shift+left-click pushes instead.
            let detach_click = click_right;
            let undo = self.pressed(ctx, KeyCode::KeyZ);
            let restart = self.pressed(ctx, KeyCode::KeyR);
            // The tutorial rail answers for the hill before the hill
            // is even handed out: while a school hill keeps a card
            // open, that card's act is the only input the gate lets
            // through — keys, clicks and shoves alike — and a hill
            // sulking over its own mistake lets nothing through at
            // all. Undo and restart keep their keys: a pupil may
            // always step back to the card, and the deck always
            // re-deals; the sulking exists precisely to be stepped
            // back out of.
            let rail = self.lesson().map(|c| c.act);
            let sulking = self.sulking();
            let sulking_undo = self.mistake_standing();
            let to_restart = self.disaster_standing();
            let admitted = self.better_t > 0.0;
            let rail_opens =
                move |a: game::Act| !sulking && !admitted && rail.is_none_or(|want| want == a);
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
                    // A restarted hill is played again, so it
                    // records again; the old walk was a worse walk
                    // anyway.
                    game.record = true;
                    self.route.clear();
                    self.replay = None;
                    // And an admission cut short by the restart
                    // does not haunt the fresh hill.
                    self.mistake_t = 0.0;
                    self.mistake_seen = false;
                    self.disaster_seen = false;
                    if to_restart {
                        // The deep sulk restarted out of: the deck
                        // retires for good and the long walk deals
                        // beneath the fresh hill. The answer's own
                        // words wait at the tour's end, where they
                        // are earned — a restart that finished
                        // nothing has nothing to answer.
                        self.oops_retired = true;
                    }
                } else if undo {
                    // The one voice the first sulk hears: when the
                    // step it takes back is its own forced mistake
                    // (`sulking_undo`, computed before the hill was
                    // borrowed), the deck moves on to re-bait its
                    // trap — and the admission still mid-air goes
                    // with the step, so the fresh card does not
                    // stand under last mistake's words.
                    if game.undo() && sulking_undo {
                        self.mistake_shown = true;
                        self.mistake_t = 0.0;
                        self.mistake_seen = false;
                    }
                    self.route.clear();
                } else if (detach || detach_click) && !game.won {
                    self.route.clear();
                    if rail_opens(game::Act::Detach) {
                        let _ = game.act(game::Act::Detach);
                    } else {
                        self.note = RAIL_NOTE.to_string();
                        self.note_t = 2.6;
                    }
                } else if let Some(dir) = push_aim.filter(|_| !game.won) {
                    // The shove itself is the game's; the pusher faces
                    // the aim like the keys do, and a push — like the
                    // keyboard's — never carries the pusher along. A
                    // refusal says why, in the game's own words; the
                    // rail's refusal says only where to look, and the
                    // ant does not even turn to aim a refused shove.
                    self.route.clear();
                    if rail_opens(game::Act::Push(dir)) {
                        self.face = face_of(dir);
                        if let Err(why) = game.act(game::Act::Push(dir)) {
                            self.note = why.to_string();
                            self.note_t = 2.6;
                        }
                    } else {
                        self.note = RAIL_NOTE.to_string();
                        self.note_t = 2.6;
                    }
                } else if let Some((dir, push)) = action.filter(|_| !game.won) {
                    self.route.clear();
                    let asked = if push {
                        game::Act::Push(dir)
                    } else {
                        game::Act::Move(dir)
                    };
                    if !rail_opens(asked) {
                        // Refused at the gate: no turn, no shove, no
                        // step — as if the key had never been struck.
                        self.note = RAIL_NOTE.to_string();
                        self.note_t = 2.6;
                    } else if push {
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
                        self.note_t = 2.6;
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
        // The cold tour watches its own cost. When the walk has
        // spent its par — the first step a cleverer walk would not
        // have needed — the hill says the walk is getting cold,
        // once; falling back to par (an undo, a restart) arms the
        // word again, and leaving the tour unarms it for good.
        {
            let tour = self.cold_tour();
            let watch = self.game.as_ref().map(|g| (g.moves, g.level.par, g.won));
            match watch {
                Some((moves, par, false)) if tour => {
                    if moves > par && !self.cold_noted {
                        self.cold_noted = true;
                        self.note = COLD_NOTE.to_string();
                        self.note_t = 3.4;
                    } else if moves <= par {
                        self.cold_noted = false;
                    }
                }
                _ => self.cold_noted = false,
            }
        }
        // And the hill's own mistakes get said out loud: the moment
        // a forced step lands, its admission hangs at the centre —
        // the oops for the single wrong step, the not-again for the
        // pair — and while it is up, and after it until the taught
        // key comes, the hill sulks answering nothing (see
        // `sulking`). The single step rolls back under the undo's
        // snapshot, par and record included, so the tour that
        // follows is still eighteen moves and the cold word still
        // arrives at its tenth; the pair rolls back under the
        // restart itself.
        {
            let oops = self.mistake_standing();
            let disaster = self.disaster_standing();
            if (oops && !self.mistake_seen) || (disaster && !self.disaster_seen) {
                self.mistake_t = BETTER_HOLD;
            }
            self.mistake_seen = oops;
            self.disaster_seen = disaster;
            if self.mistake_t > 0.0 {
                self.mistake_t -= dt;
            }
        }
        // And when the long walk finishes, the praise is allowed:
        // the ordinary splash spins ("YOUR DID IT", for eighteen
        // where nine will do — or fourteen where ten) and the
        // ledger takes the score and the walk as it takes any win.
        // The won page is not: the win tick marks the hill, and
        // when the praise has faded the hill answers its own
        // praise — big and centred, a held breath — and restarts
        // on the better script.
        if self.tour()
            && self.page == Page::Game
            && self.replay.is_none()
            && self.game.as_ref().is_some_and(|g| g.won)
        {
            self.tour_pending = true;
        }
        // And the cheap dig's warm win owes the hill its send-off:
        // the same held words at the centre — behind the splash,
        // before the won page — said once, because encouragement
        // repeated is nagging.
        if self.idx == DIG_TUTOR_HILL
            && self.warmed(DIG_TUTOR_HILL)
            && !self.send_done
            && self.page == Page::Game
            && self.replay.is_none()
            && self.game.as_ref().is_some_and(|g| g.won)
        {
            self.send_pending = true;
        }
        // A won hill hands the player their two ways off it — but
        // only after the splash has spun, held, and faded: the word
        // comes first, then the panel and the verdict together. A
        // showing that ends in a warm hill is not a win: it is an
        // ending — the audience keeps the ledger's silence.
        if self.page == Page::Game
            && self.replay.is_none()
            && self.splash_done
            && !self.tour_pending
            && self.better_t <= 0.0
            && !self.send_pending
            && self.send_t <= 0.0
            && self.game.as_ref().is_some_and(|g| g.won)
        {
            self.page = Page::Won;
            self.menu.drag = None;
            self.menu.sel = 0;
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
                let pick = self.rng.in_range(0.0, 2.999) as usize;
                let text = praise[pick].to_string();
                // The word outlives its spin: the won panel reads it,
                // in the same bright the splash burned.
                self.verdict = Some((text.clone(), colour));
                let grow = self.rng.in_range(1.875, 2.55);
                let end = self.rng.in_range(-45.0, 45.0) * std::f32::consts::FRAC_PI_4 / 45.0;
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
        // The tour's answer to its own praise: the moment the
        // splash has faded, the words go up in the centre for a
        // held breath, and the hill deals again beneath them —
        // warm now, so the cards it deals are the better script.
        if self.tour_pending && self.splash.is_none() && self.splash_done {
            self.tour_pending = false;
            self.better_t = BETTER_HOLD;
        }
        if self.better_t > 0.0 {
            self.better_t -= dt;
            if self.better_t <= 0.0 {
                self.better_t = 0.0;
                let hill = self.idx;
                self.load(hill);
                self.page = Page::Game;
            }
        }
        // The send-off rides the same breath as the answer's words
        // — splash first, then the held centre — but it reloads
        // nothing when it fades: the won page is what waits under
        // it, and the hill stays as the pupil left it.
        if self.send_pending && self.splash.is_none() && self.splash_done {
            self.send_pending = false;
            self.send_done = true;
            self.send_t = BETTER_HOLD;
        }
        if self.send_t > 0.0 {
            self.send_t -= dt;
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
            self.menu_input(ctx, dt);
        }

        self.update_leaves(ctx, dt);
        self.draw_ants(ctx, dt);
        self.draw_hud(ctx);
        self.draw_splash(ctx);
        self.draw_better(ctx);
        if self.menu.alpha > 0.002 {
            self.draw_menu(ctx);
        }
        self.draw_letterbox(ctx);
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
    // And the same cycle as silhouettes, on the same bake: the stencils
    // are generated from these very frames, so one that decodes while
    // its silhouette does not would be a broken checkout, not a bad day.
    let ant_stencils: Vec<frost::Shape> = ANT_STENCILS
        .iter()
        .map(|png| {
            ant_frame(png).unwrap_or_else(|err| {
                log::error!("antifrost: bad ant stencil: {err}");
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
            let mut d = Demo::new(ant_frames, ant_stencils, soil, settings.clone(), progress);
            d.bench = bench;
            d.window.probe = resize_probe.then_some(0.0);
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
mod pad_tests {
    use super::*;

    #[test]
    fn the_pad_aim_maps_to_a_step() {
        // The d-pad speaks first, and it speaks plainly.
        assert_eq!(
            Demo::pad_step([true, false, false, false], 0.0, 0.0),
            Some(game::Dir::Up)
        );
        assert_eq!(
            Demo::pad_step([false, true, false, false], 0.0, 0.0),
            Some(game::Dir::Down)
        );
        assert_eq!(
            Demo::pad_step([false, false, true, false], 0.0, 0.0),
            Some(game::Dir::Left)
        );
        assert_eq!(
            Demo::pad_step([false, false, false, true], 0.0, 0.0),
            Some(game::Dir::Right)
        );
        // A pad held two ways aims nowhere: the hill takes no step
        // from an ambiguous cross or an opposing pair.
        assert_eq!(Demo::pad_step([true, false, true, false], 0.0, 0.0), None);
        assert_eq!(Demo::pad_step([true, true, false, false], 0.0, 0.0), None);
        // The stick: past the dead zone, the dominant axis walks,
        // and `y` positive is up — the sign the hill thinks in.
        assert_eq!(Demo::pad_step([false; 4], 0.9, 0.2), Some(game::Dir::Right));
        assert_eq!(
            Demo::pad_step([false; 4], -0.9, -0.2),
            Some(game::Dir::Left)
        );
        assert_eq!(Demo::pad_step([false; 4], 0.2, 0.9), Some(game::Dir::Up));
        assert_eq!(Demo::pad_step([false; 4], 0.0, -0.8), Some(game::Dir::Down));
        // The dead zone keeps a resting stick from walking the trail
        // on its own, and it is the same floor on both axes.
        assert_eq!(Demo::pad_step([false; 4], 0.0, 0.0), None);
        assert_eq!(Demo::pad_step([false; 4], 0.49, 0.49), None);
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
        let stencils = ANT_STENCILS
            .iter()
            .map(|png| ant_frame(png).expect("shipped ant stencils bake"))
            .collect();
        let soil = frost::Shape::sprite_bytes(SOIL).expect("shipped soil bakes");
        Demo::new(
            frames,
            stencils,
            soil,
            Settings::default(),
            Progress::default(),
        )
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
