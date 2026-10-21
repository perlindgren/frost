//! The smallest host for [`frost_shaders::DISSOLVE`]: one tile of soil
//! stretched across the window, one hole dug forever.
//!
//! The soil is an ordinary sprite; `Shape::painted_by` hangs the
//! shelf's program on it, and a handful of scene-node fields per frame
//! do the rest — progress, a seed, softness, and the **grain**: noise
//! cycles across the node, which the host scales to its own size. A
//! game digs cells and rides five cycles per tile; this window-sized
//! host rides one clump per [`GRAIN_PX`] pixels, so the bite keeps its
//! physical character at any window size and eats in one continuous,
//! irregular front. The seed changes with every dig, so no two holes
//! are eaten alike. The rim glows whether lights or none —
//! disappearing earth is its own light source — so the tile rides
//! unlit and the dark background shows through wherever the band has
//! bitten.
//!
//! ```text
//! cargo run -p frost_shaders --example dissolve
//! ```

use frost::{Color, Context, Scene, SceneNode, Shape, Transform};

/// One dig, in seconds: the band's sweep from whole tile to none.
const DIG_SECONDS: f32 = 2.0;
/// The beat of dark before the next dig: the rest between holes.
const REST_SECONDS: f32 = 1.0;
/// The eat band's edge softness — the same figure antifrost sails
/// through its hills with.
const SOFTNESS: f32 = 0.14;
/// One noise clump per this many **physical** pixels. The game
/// bites at ~19 — a thumbnail through a 93 px tile — but this
/// showcase rides the field zoomed in, so the clumps, rims, and
/// eaten regions read as geography across the window instead of
/// texture. The shader never knows the difference; the host does.
const GRAIN_PX: f32 = 100.0;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `CARGO_MANIFEST_DIR` pins the asset path to the crate root, so
    // the example runs from anywhere. The soil is the game's own —
    // antifrost's tile, byte for byte — worn here the way the shelf
    // means it to be worn: stretched whole across the window and
    // eaten by the program.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let soil = Shape::sprite(format!("{root}/assets/soil.png"))?;
    let [tw, th] = soil.sprite_size().expect("a png carries its size");

    // The whole trick of the shelf in one line: a sprite wearing a
    // program.
    let painted = Shape::painted_by(soil, frost_shaders::DISSOLVE);

    let scene = Scene::new(SceneNode {
        // The night behind the hole: where the band has bitten
        // through, this is what shows.
        shape: Some(frost::Shape::Background {
            color: Color {
                r: 0.05,
                g: 0.05,
                b: 0.07,
                a: 1.0,
            },
        }),
        children: vec![Box::new(SceneNode {
            shape: Some(painted),
            ..Default::default()
        })],
        ..Default::default()
    });

    let mut clock = 0.0f32;
    let mut dig = -1.0f32;
    let mut seed = 0.0f32;

    frost::run(scene, move |ctx: &mut Context, dt: f32| {
        clock += dt;
        let cycle = DIG_SECONDS + REST_SECONDS;
        // A new dig every cycle, reseeded by the golden ratio so the
        // holes never fall into a visible rhythm.
        if (clock / cycle).floor() > dig {
            dig = (clock / cycle).floor();
            seed = (dig * 0.618034).fract();
        }
        let progress = (clock % cycle) / DIG_SECONDS;

        let (w, h) = ctx.size();
        let scale = ctx.scale_factor();
        let node = &mut ctx.scene().root.children[0];
        // One tile, covering the window, at rest at its centre —
        // the program does all the moving.
        node.transform = Transform::identity();
        node.scale = [w / tw, h / th];
        // The floats of the shelf's contract: progress, seed,
        // softness, and the grain — clumps of the game's own physical
        // size in a window this size, in logical units the shader
        // never has to think about.
        node.emission = Color {
            r: progress.min(1.0),
            g: seed,
            b: SOFTNESS,
            a: w.max(h) * scale / GRAIN_PX,
        };
        // …and the dirt's tint, arriving whole over a white modulate.
        node.glow = Some(Color {
            r: 0.78,
            g: 0.62,
            b: 0.5,
            a: 1.0,
        });
    })
}
