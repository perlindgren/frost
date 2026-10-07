//! A side-scrolling tile-map parallax: the tilemap riding the engine's
//! full camera story — camera node, per-layer parallax speeds, and
//! infinite layer repeat — with one white-on-transparent tileset
//! modulated into five colored depths.
//!
//! The tileset is the repository's own `assets/sprites/basic_tiles.png`
//! — four 32 px cells (a half-filled cell and three diagonals), drawn as
//! white glyphs on a transparent ground: white because modulation
//! multiplies and only a white texel carries a tint whole, transparent
//! because everything a glyph does not cover lets the layers behind
//! show. Its grid comes from the sprite_util sidecar `basic_tiles.ron`
//! (`atlas: [rows, cols]`), read with the same RON-subset tree parser
//! `sprite_util` uses.
//!
//! The camera drifts right on its own; hold Right (or D) to speed the
//! scroll up, Left (or A) to push it back. It is the camera that moves —
//! a shapeless pivot hung under the player sprite, designated as
//! `Scene::camera` — so the player sits fixed at the window origin while
//! the world sweeps past, exactly like the `parallax` example.
//!
//! The world is five [`frost::Layer`]s, all at negative order so the
//! scene's base group (background + the `Diagnostics` overlay) stays on
//! top. The overlay rides a layer of the engine's own making, at
//! parallax speed 0.0 — a heads-up display belongs to the window, not
//! to the scrolling world. Back to front: a dusty-amber far band (speed 0.35), a teal band
//! turned 45 degrees (speed 0.7 — rotation rides the repeat machinery
//! like any other transform), the dense amber ground the camera follows
//! at full speed (1.0), the white player alone on its non-repeating
//! layer (1.0), and huge sparse near blocks rushing past (1.6). Each
//! repeating layer's period is set on the first frame from the live
//! window width (never below 1024 px): the minimum legal period is the
//! window itself, and every map's *painted extent* stays inside one
//! period — a repeating object spanning more than its period would draw
//! onto its own copy, and the engine stops the program with a named
//! error the moment that happens. A repeat re-draws its layer once per
//! copy that overlaps the window (at most two here), so a tilemap on a
//! repeating layer simply becomes one instanced draw per copy.
//!
//! The strips were not authored to wrap, so there is a visible seam
//! where each map's right edge meets its own left — this demo shows the
//! parallax machinery, not seamless art; and the `DRAW` readout stays
//! tiny (a handful of instanced calls) no matter how far the camera
//! travels. Alt-0 hides the overlay, Alt-1..Alt-4 the charts, Alt-T the
//! readout lines, Alt+'+'/Alt+'-' rescale it.
//!
//! Run with:
//!
//! ```text
//! cargo run --example tilemap
//! ```

use frost::{
    Color, Diagnostics, DiagnosticsFlags, Layer, NodePath, Scene, SceneNode, Shape, Tile, Transform,
};

/// The camera's resting drift, in pixels per second to the right.
const DRIFT: f32 = 90.0;

/// How much holding an arrow key adds to (or subtracts from) the drift.
const NUDGE: f32 = 240.0;

/// The smallest repeat period the demo uses, in pixels — it keeps the
/// period legal (never below the window) on any window size the demo
/// opens at, so the map sizes below can stay fixed.
const MIN_PERIOD: f32 = 1024.0;

/// The layer indices, back to front.
const FAR: usize = 0;
const MID: usize = 1;
const GROUND: usize = 2;
const PLAYER: usize = 3;
const NEAR: usize = 4;

/// The RON parser and tree model, shared with `ron_view` and
/// `sprite_util`: this example compiles the same `ron_view/tree.rs`
/// through a path include and reads one fact from it — the sidecar's
/// grid. `ron_view` uses the whole module; the sidecar panel leaves its
/// viewer-only helpers unused, hence the blanket allow.
#[allow(dead_code)]
#[path = "ron_view/tree.rs"]
mod ron_tree;

/// The tile grid the sidecar's root names, if it names one: its
/// `atlas` field's two entries, `[rows, cols]`. A field of the wrong
/// shape — not a two-entry array, or a missing or zero count — reads
/// as no grid, the way a missing field does.
fn atlas_of(root: &ron_tree::Val) -> Option<(usize, usize)> {
    let ron_tree::Val::Struct { fields, .. } = root else {
        return None;
    };
    let (_, item) = fields.iter().find(|(key, _)| *key == "atlas")?;
    let ron_tree::Val::Seq { items, .. } = &item.val else {
        return None;
    };
    if items.len() != 2 {
        return None;
    }
    let counts: Vec<usize> = items
        .iter()
        .filter_map(|it| match &it.val {
            ron_tree::Val::Atom(text, ron_tree::Kind::Num) => text.parse().ok(),
            _ => None,
        })
        .collect();
    match counts.as_slice() {
        [rows, cols] if *rows > 0 && *cols > 0 => Some((*rows, *cols)),
        _ => None,
    }
}

/// The UV bounds of atlas cell `idx` (row-major over a `rows`-by-`cols`
/// grid): the cell's own rectangle, top-left origin like the PNG's rows.
fn cell_uv(idx: usize, rows: usize, cols: usize) -> [f32; 4] {
    let (row, col) = ((idx / cols) as f32, (idx % cols) as f32);
    let (rows, cols) = (rows as f32, cols as f32);
    [
        col / cols,
        row / rows,
        (col + 1.0) / cols,
        (row + 1.0) / rows,
    ]
}

/// A deterministic stand-in for terrain: a xorshift over the cell
/// coordinates, whose low bits pick the atlas cell and high bits decide
/// whether the cell is left as a hole. Real maps read a file.
fn terrain(x: i32, y: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x27d4eb2d) ^ (y as u32).wrapping_mul(0x165667b1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545f491);
    h ^= h >> 13;
    h
}

/// A tileset read from the sidecar: the atlas grid, and the cell's own
/// texture size derived from it — the two facts maps are laid with.
struct Tileset {
    rows: usize,
    cols: usize,
    /// The cell's pixel size: the atlas divided by its grid.
    tile: [f32; 2],
}

/// Lays tiles over the rectangle `[x0, y0]..[x1, y1]` (local units),
/// one cell per grid step, cycling the atlas by the terrain hash — and
/// skipping one cell in `holes` of every hundred, leaving gaps the
/// layers below can show through.
fn fill(map: &mut Shape, set: &Tileset, x0: f32, y0: f32, x1: f32, y1: f32, holes: u32) {
    let (rows, cols, tile) = (set.rows, set.cols, set.tile);
    let cells = rows * cols;
    let mut out = Vec::new();
    let mut gy = y0;
    while gy < y1 {
        let mut gx = x0;
        while gx < x1 {
            let h = terrain((gx / tile[0]) as i32, (gy / tile[1]) as i32);
            if h % 100 >= holes {
                let uv = cell_uv((h >> 8) as usize % cells, rows, cols);
                out.push(Tile::new(
                    [gx + tile[0] / 2.0, gy + tile[1] / 2.0],
                    tile,
                    uv,
                ));
            }
            gx += tile[0];
        }
        gy += tile[1];
    }
    map.set_tiles(out);
}

/// A flat color for the layer modulates.
const fn rgba(r: f32, g: f32, b: f32) -> Color {
    Color { r, g, b, a: 1.0 }
}

/// A layer holding one map node, at the given parallax order and speed.
fn layer(order: f32, speed: f32, map: SceneNode) -> Layer {
    Layer {
        order,
        speed,
        repeat: [0.0, 0.0],
        root: SceneNode {
            children: vec![Box::new(map)],
            ..Default::default()
        },
    }
}

/// The key pair along one axis, as `-1`, `0`, or `1` — the `parallax`
/// example's idiom.
fn axis(ctx: &frost::Context, positive: frost::KeyCode, negative: frost::KeyCode) -> f32 {
    (ctx.key_down(positive) as i32 - ctx.key_down(negative) as i32) as f32
}

struct Demo {
    /// The diagnostics overlay — including the draw-call readout, which
    /// stays at a handful of instanced calls however far the camera has
    /// scrolled.
    diag: Diagnostics,
    /// The camera's (and player's) position in world pixels; only x
    /// moves — this is a side-scroller.
    pos: [f32; 2],
    /// Whether the repeating layers' periods have been set yet (they
    /// need the live window size, known from the first frame).
    tiled: bool,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // The overlay first: it keeps its own layer and handles its own
        // Alt toggles.
        self.diag.process(ctx, dt);

        // The camera drifts right on its own; the arrows (or A/D) ride
        // on top of the drift — hold Right to race, Left to push the
        // world back the other way.
        let dx = axis(ctx, frost::KeyCode::ArrowRight, frost::KeyCode::ArrowLeft)
            + axis(ctx, frost::KeyCode::KeyD, frost::KeyCode::KeyA);
        self.pos[0] += (DRIFT + NUDGE * dx) * dt;

        // The player node carries the camera as its child, so moving the
        // player moves the camera, and every layer feels it at its own
        // speed. (The overlay rides a layer of its own, at parallax
        // speed 0.0, so it stays pinned to the window by itself.)
        {
            let player = &mut ctx.scene().layers[PLAYER].root.children[0];
            player.transform = Transform::translate(self.pos);
        }

        // The periods, once: the window width (never below the demo's
        // floor) in x. The window size is only known from the first
        // frame, which is why the layers start non-repeating. The
        // player's layer never tiles — the player is unique, not part of
        // the pattern.
        if !self.tiled {
            let (w, _) = ctx.size();
            let period = w.max(MIN_PERIOD);
            for i in [FAR, MID, GROUND, NEAR] {
                ctx.scene().layers[i].repeat = [period, 0.0];
            }
            self.tiled = true;
        }

        log::trace!("process: pos {:?}", self.pos);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset paths to the crate root, so
    // the example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let png_path = format!("{root}/assets/sprites/basic_tiles.png");
    let ron_path = format!("{root}/assets/sprites/basic_tiles.ron");
    let diag = match Diagnostics::new(
        format!("{root}/assets/fonts/FiraCode-VariableFont_wght.ttf"),
        DiagnosticsFlags::all(),
    ) {
        Ok(diagnostics) => diagnostics,
        Err(err) => {
            log::error!("failed to load the font: {err}");
            std::process::exit(1);
        }
    };

    // The atlas, decoded once. The shape clones share its pixel buffer
    // — and therefore its GPU texture — across every map and the player
    // sprite made from it.
    let base = Shape::tilemap(&png_path).expect("basic_tiles.png ships with the crate");
    let sidecar = std::fs::read_to_string(&ron_path).expect("basic_tiles.ron ships with the crate");
    let (rows, cols) = atlas_of(&ron_tree::parse(&sidecar).expect("the shipped sidecar parses"))
        .expect("the shipped sidecar names a grid");
    let [w, h] = base.sprite_size().expect("a tile map reports its atlas");
    let set = Tileset {
        rows,
        cols,
        tile: [w / cols as f32, h / rows as f32],
    };

    // The far band: sparse, high, dimmed amber — the haze layer at a
    // third of the camera's speed. 768 px wide, well inside one period.
    let mut far_map = base.clone();
    fill(&mut far_map, &set, -384.0, 40.0, 384.0, 184.0, 45);
    let far = SceneNode {
        modulate: rgba(0.8, 0.52, 0.2),
        shape: Some(far_map),
        ..Default::default()
    };

    // The mid band: turned 45 degrees and scaled 2.5x, teal. Its
    // painted box — the rotated square, about 849 px — still fits the
    // 1024 px floor of the period, which is the rule repeating maps
    // live by: painted extent within one period, or the engine stops
    // the program.
    let mut mid_map = base.clone();
    fill(&mut mid_map, &set, -120.0, -110.0, 120.0, 110.0, 25);
    let mid = SceneNode {
        // Rotate in place, then lift the band above the sightline.
        transform: Transform::rotate(std::f32::consts::FRAC_PI_4)
            .compose(&Transform::translate([0.0, 60.0])),
        scale: [2.5, 2.5],
        modulate: rgba(0.35, 0.95, 0.75),
        shape: Some(mid_map),
        ..Default::default()
    };

    // The ground: two dense rows at the feet of the window, full speed.
    let mut ground_map = base.clone();
    fill(&mut ground_map, &set, -384.0, -288.0, 384.0, -224.0, 12);
    let ground = SceneNode {
        modulate: rgba(1.0, 0.62, 0.25),
        shape: Some(ground_map),
        ..Default::default()
    };

    // The near foreground: a six-cell map scaled 4x — 768 painted px,
    // again inside the period — sparse huge blocks rushing past at
    // almost twice the camera's speed.
    let mut near_map = base.clone();
    fill(&mut near_map, &set, -96.0, -32.0, 96.0, 32.0, 60);
    let near = SceneNode {
        transform: Transform::translate([0.0, -40.0]),
        scale: [4.0, 4.0],
        modulate: rgba(0.18, 0.5, 0.45),
        shape: Some(near_map),
        ..Default::default()
    };

    // The player: the untinted atlas strip itself, marking the world
    // origin the camera orbits around — with the shapeless camera pivot
    // as its child. Unique, so its layer never repeats.
    //
    // The player carries NO scale: the camera's transform is the whole
    // chain from the scene down to the pivot, and every factor on it —
    // scale included — becomes the view every group is drawn through.
    // A scaled player would be a zoom camera by accident. (Deliberately
    // scaled camera chains are a legitimate zoom; inherited zoom by
    // surprise is the trap.)
    let mut player = SceneNode {
        shape: Some(Shape::sprite(&png_path).expect("basic_tiles.png is a valid PNG")),
        ..Default::default()
    };
    player.children.push(Box::new(SceneNode::default()));

    let layers = vec![
        layer(-2.0, 0.35, far),
        layer(-1.5, 0.7, mid),
        layer(-1.0, 1.0, ground),
        layer(-0.5, 1.0, player),
        layer(-0.25, 1.6, near),
    ];

    let scene = Scene {
        // The base group: the background, plus (from the first frame)
        // the overlay's nodes. It sits at the implicit order 0.0 — above
        // every layer, since all the world's layers chose negative
        // orders.
        root: SceneNode {
            shape: Some(Shape::Background {
                color: Color {
                    r: 0.07,
                    g: 0.07,
                    b: 0.10,
                    a: 1.0,
                },
            }),
            ..Default::default()
        },
        layers,
        // The camera: the player's shapeless child. It stays at the
        // window origin, and every layer sweeps past at its own speed.
        camera: Some(NodePath {
            group: Some(PLAYER),
            children: vec![0, 0],
        }),
        ambient: frost::AMBIENT,
    };

    if let Err(err) = frost::run(
        scene,
        Demo {
            diag,
            pos: [0.0, 0.0],
            tiled: false,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
