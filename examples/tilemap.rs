//! A tile map drawn straight from an atlas: the engine-side half of the
//! tile-map story, with no editor in the way.
//!
//! The tileset is one of the repository's own assets,
//! `assets/sprites/basic_tiles.png` — a strip of four 32 px cells (a
//! half-filled cell and three diagonals, so a mis-mapped or flipped
//! cell shows up at a glance), drawn as white glyphs on a transparent
//! ground: white because modulation multiplies, and only a white texel
//! can carry a tint whole; transparent because everything the glyph
//! does not cover lets the layers below show through. Its grid comes
//! from the sprite_util sidecar, `basic_tiles.ron`: the
//! `atlas: [rows, cols]` field is read with the same RON-subset tree
//! parser `sprite_util` uses, so the map, the sprite editor, and the
//! file on disk all speak one dialect.
//!
//! Two maps are laid from it: one fills the left of the window at native
//! scale, the other a smaller patch scaled 2.5x and translated right —
//! and their view rectangles overlap on purpose. Neither map is filled
//! solid: both skip cells by hash, and the skip patterns differ; and
//! each layer carries its own modulate — warm amber below, cool teal
//! above — which the white glyphs absorb whole. The two layers see
//! through each other twice over: through the skipped cells, and
//! through the glyphs' own transparent ground. Across the overlap the
//! scaled map still cuts the plain one at its clip seam — the scissor
//! — while everywhere else the layers trade visibility through each
//! other's gaps: layering you can watch.
//!
//! All three maps share one atlas buffer — and therefore one GPU
//! texture — with a plain sprite of the same strip shown at the top
//! right: the tileset as authored, beside what the maps do with it.
//!
//! The `frost::Diagnostics` overlay rides on top — and its draw-call
//! chart is the point of the whole demo: the maps hold hundreds of
//! cells, and the count stays at a handful, because every map is ONE
//! instanced draw call. Alt-0 hides the overlay, Alt-1..Alt-4 the
//! charts, Alt-T the readout lines, Alt+'+'/Alt+'-' rescale it.
//!
//! Run with:
//!
//! ```text
//! cargo run --example tilemap
//! ```

use frost::{Color, Diagnostics, DiagnosticsFlags, Scene, SceneNode, Shape, Tile, Transform};

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

struct Demo {
    /// The diagnostics overlay: frame rate, frame time, processing
    /// times, and — the interesting one here — the draw-call count the
    /// instanced tile maps barely move.
    diag: Diagnostics,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // The overlay appends its own nodes to the root on the first
        // call and updates them in place after that.
        self.diag.process(ctx, dt);
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
    // — and therefore its GPU texture — across every map and the sprite
    // made from it.
    let base = Shape::tilemap(&png_path).expect("basic_tiles.png ships with the crate");
    let sidecar = std::fs::read_to_string(&ron_path).expect("basic_tiles.ron ships with the crate");
    let (rows, cols) = atlas_of(&ron_tree::parse(&sidecar).expect("the shipped sidecar parses"))
        .expect("the shipped sidecar names a grid");
    // The cell's own texture size, derived rather than assumed: the
    // atlas divides evenly into the sidecar's grid.
    let [w, h] = base.sprite_size().expect("a tile map reports its atlas");
    let set = Tileset {
        rows,
        cols,
        tile: [w / cols as f32, h / rows as f32],
    };

    // The plain map: native scale, filling the left of the window,
    // clipped to its own view rectangle, with one cell in five left
    // open.
    let mut left = base.clone();
    fill(&mut left, &set, -320.0, -280.0, 60.0, 280.0, 20);
    left.set_clip(Some([-320.0, -280.0, 60.0, 280.0]));

    // The scaled map: 2.5x, translated right, its own local clip — and
    // a view rectangle that deliberately overlaps the plain map's. Its
    // holes follow a different pattern (the hash runs over its own
    // local grid), so across the overlap the two layers trade
    // visibility through each other's gaps.
    let mut right = base.clone();
    fill(&mut right, &set, -120.0, -110.0, 120.0, 110.0, 25);
    right.set_clip(Some([-120.0, -110.0, 120.0, 110.0]));

    // A map whose clip is wholly off the window: the scissor comes back
    // empty and the whole draw is skipped — no GPU work at all. Proof
    // by absence: remove this node and nothing changes on screen.
    let mut offscreen = base.clone();
    fill(&mut offscreen, &set, -100.0, -100.0, 100.0, 100.0, 20);
    offscreen.set_clip(Some([-80.0, -80.0, 80.0, 80.0]));

    // The strip itself, as a plain sprite, top right: the tileset as
    // authored, for comparison with what the maps do with it.
    let sprite = Shape::sprite(&png_path).expect("basic_tiles.png is a valid PNG");

    let scene = Scene::new(SceneNode {
        shape: Some(Shape::Background {
            color: Color {
                r: 0.07,
                g: 0.07,
                b: 0.10,
                a: 1.0,
            },
        }),
        children: vec![
            Box::new(SceneNode {
                // Warm amber: every white texel of the plain map takes
                // this tint, and the transparent ground carries nothing.
                modulate: Color {
                    r: 1.0,
                    g: 0.62,
                    b: 0.25,
                    a: 1.0,
                },
                shape: Some(left),
                ..Default::default()
            }),
            Box::new(SceneNode {
                transform: Transform::translate([150.0, -40.0]),
                scale: [2.5, 2.5],
                order: 1.0,
                // Cool teal over warm amber: the modulation is the
                // only thing that differs from the plain map's look.
                modulate: Color {
                    r: 0.35,
                    g: 0.95,
                    b: 0.75,
                    a: 1.0,
                },
                shape: Some(right),
                ..Default::default()
            }),
            Box::new(SceneNode {
                transform: Transform::translate([6000.0, 6000.0]),
                shape: Some(offscreen),
                ..Default::default()
            }),
            Box::new(SceneNode {
                transform: Transform::translate([300.0, 250.0]),
                scale: [0.5, 0.5],
                order: 2.0,
                shape: Some(sprite),
                ..Default::default()
            }),
        ],
        ..Default::default()
    });

    if let Err(err) = frost::run(scene, Demo { diag }) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
