//! Courtyard: the first app that boots a map file off its own cargo.
//!
//! Twelve by eight: a floor of tile 0 inside a ring of tile 3, read
//! from `assets/maps/courtyard.map.ron` — a file exactly as the desk
//! writes it, and dressed by `assets/sprites/basic_tiles.tileset.ron`
//! exactly as the desk shelves it. The chain is the whole demo: the
//! map names the tileset *by path relative to itself*, the tileset
//! names its sheet *by file name relative to itself*, and one
//! `frost::map::resolve` walks the links identically over a folder of
//! loose files and a table of embedded bytes — the guard's road is
//! the boot's road, and the tests below walk it over the shipped
//! table before the window ever opens.
//!
//! Rung 1 landed: the floor is up. Rung 2 is alight: both layers
//! take light, the lamp follows the cursor and sticks where the
//! cursor left it, and the rampart declares itself cell by cell —
//! so the walls' shadows sweep the floor as the lamp moves. The
//! marble that reads the tileset's solidity is rung 3, still owed.
//! Rung 4 has landed: the courtyard watches its own cargo — every
//! path in its table, loose on disk — and re-dresses the scene when
//! any of them is saved, refusing to lose the last honest courtyard
//! to a half-written file; and it tells, publishing its affair to
//! the well-known folder, where `attune` counts the revs. Paint in
//! the desk, save, and the courtyard follows without a restart;
//! `attune` hears the courtyard think.
//!
//! What the sheet actually is was worth learning the hard way
//! (a grey+alpha PNG probed as if RGBA is a sheet of lies): tile 0
//! is a solid slab, and tiles 1, 2, 3 are diagonal wedges of
//! increasing steepness — the tilemap example paints them as
//! mountains. The courtyard takes the slab for its floor and the
//! half-wedge for a rampart ring; a sawtooth is what tile 3 looks
//! like, as one screenshot proved. Where the wedges let the
//! background through, rung 2's shadows will too — and what is
//! painted is grey 255: a pure receiver where it stands at all.
//! Layers wear tints the way every other frost scene tints — by
//! `modulate`.

use frost::map::{self, MapLayer};
use frost::tileset::{self, Tileset};
use frost::{
    Assets, Color, Context, Layer, Process, Scene, SceneNode, Shape, Table, Transform, Watch,
};

/// A flat color for the layer modulates.
const fn rgba(r: f32, g: f32, b: f32) -> Color {
    Color { r, g, b, a: 1.0 }
}

/// The app's cargo: root-relative paths, byte-identical between the
/// table key and the `include_bytes!` argument — the same strings
/// the documents speak. A shipped world needs nothing else; the dev
/// main admits the loose root on top, and the next save in the desk
/// outranks the table from the next load.
const CARGO: Table = &[
    (
        "assets/maps/courtyard.map.ron",
        include_bytes!("../assets/maps/courtyard.map.ron"),
    ),
    (
        "assets/sprites/basic_tiles.tileset.ron",
        include_bytes!("../assets/sprites/basic_tiles.tileset.ron"),
    ),
    (
        "assets/sprites/basic_tiles.png",
        include_bytes!("../assets/sprites/basic_tiles.png"),
    ),
];

/// The map, by the path both worlds speak.
const MAP_PATH: &str = "assets/maps/courtyard.map.ron";

/// The lamp: warm, generous, and immediate — it is the demo's hand,
/// declared fresh to every frame like every other immediate light.
const LAMP: Color = Color {
    r: 1.0,
    g: 0.82,
    b: 0.55,
    a: 1.0,
};
const LAMP_INTENSITY: f32 = 1.8;
const LAMP_RADIUS: f32 = 220.0;

/// The rampart's tile, and the cell's edge in world pixels (the
/// sheet is four tiles across 128 px). The collider of rung 3 will
/// reuse both — same cells, same boxes, a different question.
const RAMPART: u32 = 1;
const CELL_PX: f32 = 32.0;

/// The well-known folder every app tells into and every tool reads
/// — the same path `attune` listens to and `sprite_util` speaks to.
const TELL_DIR: &str = "target/frost/live";

/// A tileset document and its decoded sheet: the desk's shelf, in
/// app clothing.
struct Shelf {
    entry: Tileset,
    sheet: Shape,
}

/// The one chain read: map text, tileset documents, sheets — every
/// reference resolved against the document that speaks it. Refusals
/// are named and carried out of the function the way the desk
/// carries them: the world boots as much of the chain as is honest,
/// and the log says what waits.
fn load_cargo(store: &Assets) -> (Vec<MapLayer>, Vec<Shelf>, Vec<String>) {
    let Some(src) = store.bytes(MAP_PATH) else {
        return (
            Vec::new(),
            Vec::new(),
            vec!["the cargo carries no map".into()],
        );
    };
    let (map, mut refused) = map::from_text(&String::from_utf8_lossy(&src));
    let mut shelf: Vec<Shelf> = Vec::new();
    for reference in &map.tilesets {
        let path = map::resolve(MAP_PATH, reference);
        let Some(doc) = store.bytes(&path) else {
            refused.push(format!(
                "tileset reference '{reference}' resolves to '{path}', which the cargo does not carry"
            ));
            continue;
        };
        let (entry, errs) = tileset::from_text(&String::from_utf8_lossy(&doc));
        refused.extend(errs);
        let Some(entry) = entry else { continue };
        let sheet_path = map::resolve(&path, &entry.sheet);
        let Some(bytes) = store.bytes(&sheet_path) else {
            refused.push(format!(
                "'{path}' cuts sheet '{sheet_path}', which the cargo does not carry"
            ));
            continue;
        };
        match Shape::sprite_bytes_nearest(bytes.as_ref()) {
            Ok(sheet) => shelf.push(Shelf { entry, sheet }),
            Err(err) => refused.push(format!("'{sheet_path}' is no sprite: {err}")),
        }
    }
    (map.layers, shelf, refused)
}

/// Dress the layers: one node per layer, tinted by its office — the
/// floor green, the ring stone-gray — and lit. Both boot and reload
/// dress through this one function, so what a save brings about is
/// exactly what the app would have built from nothing.
fn build_nodes(layers: &[MapLayer], shelf: &[Shelf]) -> (Vec<SceneNode>, Vec<String>) {
    // Plain values out; the engine's tree wants boxes, and the two
    // callers box at the boundary where they hand the children to
    // `SceneNode` — `clippy::vec_box` is right that a helper has no
    // business shipping an allocation twice over.
    let mut children: Vec<SceneNode> = Vec::new();
    let mut warns = Vec::new();
    for (i, layer) in layers.iter().enumerate() {
        let Some(dressed) = shelf.iter().find(|s| s.entry.name == layer.tileset) else {
            warns.push(format!("'{}' waits dressed as nothing", layer.tileset));
            continue;
        };
        let Some(shape) = layer.shape(&dressed.entry, Some(&dressed.sheet)) else {
            continue; // an empty layer asks nothing of the renderer
        };
        let tint = if i == 0 {
            rgba(0.35, 0.7, 0.35) // the floor: grass over white
        } else {
            rgba(0.55, 0.55, 0.6) // the ring: stone over white
        };
        children.push(SceneNode {
            shape: Some(shape),
            modulate: tint,
            lit: true,
            ..Default::default()
        });
    }
    (children, warns)
}

/// The declared rampart: one 32 px box per painted wall cell, in
/// world space — the silhouette claim the light asks about. A map
/// paints; it is the app that says which painted tiles stand in the
/// light's way.
fn rampart_boxes(layers: &[MapLayer]) -> Vec<([f32; 2], [f32; 2])> {
    let mut boxes = Vec::new();
    for layer in layers {
        for (i, &code) in &mut layer.cells.iter().enumerate() {
            if code != RAMPART {
                continue;
            }
            // Cells are stored row-major from the layer's top-left;
            // the grid's center sits at the world origin.
            let col = i as f32 % layer.cols as f32;
            let row = i as f32 / layer.cols as f32;
            let half = CELL_PX / 2.0;
            boxes.push((
                [
                    col * CELL_PX + half - (layer.cols as f32 * CELL_PX) / 2.0,
                    (layer.rows as f32 * CELL_PX) / 2.0 - row * CELL_PX - half,
                ],
                [half, half],
            ));
        }
    }
    boxes
}

/// The courtyard holds its lamp: the cursor's last word, so the
/// light sticks where the pointer left the window rather than
/// snapping back to the middle of the dark.
struct Courtyard {
    lamp: Option<[f32; 2]>,
    boxes: Vec<([f32; 2], [f32; 2])>,
    /// The loose root the watchers' paths hang from — the same one
    /// `with_dir` admitted at boot, so the ears and the store agree
    /// about which courtyard is the live one.
    root: std::path::PathBuf,
    watchers: Vec<Watch>,
    teller: frost::Teller,
    /// What stands, as last re-dressed: the telling's subject.
    floors: usize,
    reloads: u64,
}

impl Courtyard {
    /// The app's affair in its own words — the ARCHITECTURE telling:
    /// what a tool may ask without touching a window. `publish`
    /// deduplicates by the written body, so this is spoken every
    /// frame and written only when something changed.
    fn being(&self) -> frost::ron::Val {
        frost::ron::record_named(&[
            ("map", frost::ron::text(MAP_PATH)),
            ("layers", frost::ron::number(self.floors as f64)),
            ("walls", frost::ron::number(self.boxes.len() as f64)),
            ("reloads", frost::ron::number(self.reloads as f64)),
        ])
    }
}

impl Process for Courtyard {
    fn process(&mut self, ctx: &mut Context, _dt: f32) {
        // The ears: every path this app's cargo carries, watched on
        // the loose root. Save any of them in the desk — map,
        // tileset, even the sheet — and the courtyard re-reads the
        // whole chain, resolves every reference again, and re-dresses
        // the scene from bytes it did not have a frame ago.
        let mut stirred = false;
        for watcher in &mut self.watchers {
            if watcher.changed() {
                stirred = true;
            }
        }
        if stirred {
            let store = Assets::embedded(CARGO).with_dir(&self.root);
            let (layers, shelf, refused) = load_cargo(&store);
            for why in &refused {
                log::warn!("courtyard: {why}");
            }
            let (children, warns) = build_nodes(&layers, &shelf);
            for why in warns {
                log::warn!("courtyard: {why}");
            }
            if children.is_empty() {
                // A half-written save — or a fully broken one — must
                // not unmake a standing courtyard: the last honest
                // world holds until an honest one arrives.
                log::warn!("courtyard: the reload dressed nothing — the old courtyard stands");
            } else {
                self.floors = layers.len();
                self.reloads += 1;
                self.boxes = rampart_boxes(&layers);
                log::info!("courtyard: re-read the cargo, reload #{}", self.reloads);
                ctx.scene().layers[0].root.children = children.into_iter().map(Box::new).collect();
            }
        }
        // The tongue: coming into existence is a transition, and so
        // is every re-dressing — the first publish lands on frame
        // one, and `publish` keeps the silence otherwise.
        let being = self.being();
        self.teller.publish(&being);
        if let Some(where_) = ctx.mouse_position() {
            self.lamp = Some(where_);
        }
        let [x, y] = self.lamp.unwrap_or([0.0, 0.0]);
        ctx.light(x, y, LAMP, LAMP_INTENSITY, LAMP_RADIUS);
        // The rampart declares itself, cell by cell, every frame.
        // Nothing painted casts until it says so — a shadow is a
        // claim, and rung 3's collider will be these same 36 boxes
        // asked a different question.
        for &(center, half) in &self.boxes {
            ctx.occluder(Transform::identity(), center, half);
        }
        // Its markers: immediate draws are unlit, so the lamp stays
        // bright inside its own night — a halo in its own color and
        // a white-hot core.
        ctx.circle(x, y, 26.0, Color { a: 0.35, ..LAMP }, 2.0);
        ctx.circle(
            x,
            y,
            5.0,
            Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            2.0,
        );
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");
    let store = Assets::embedded(CARGO).with_dir(std::env!("CARGO_MANIFEST_DIR"));
    let (layers, shelf, refused) = load_cargo(&store);
    for why in &refused {
        log::warn!("courtyard: {why}");
    }
    // Both layers take the light — rung 2 flipped that one switch
    // and changed nothing else, which is the seam the splice was cut
    // to leave: the tiles still draw their same instance batches,
    // and read the light field like any other shape.
    let root = std::path::PathBuf::from(std::env!("CARGO_MANIFEST_DIR"));
    let (children, warns) = build_nodes(&layers, &shelf);
    for why in warns {
        log::warn!("courtyard: {why}");
    }
    if children.is_empty() {
        log::error!("courtyard: the map dressed nothing — nothing to show");
        std::process::exit(1);
    }
    log::info!("courtyard: {} layers stand", children.len());
    let scene = Scene {
        root: SceneNode {
            shape: Some(Shape::Background {
                color: rgba(0.07, 0.07, 0.1),
            }),
            ..Default::default()
        },
        layers: vec![Layer {
            order: -1.0,
            speed: 0.0,
            repeat: [0.0, 0.0],
            root: SceneNode {
                children: children.into_iter().map(Box::new).collect(),
                ..Default::default()
            },
        }],
        // No camera yet: the courtyard sits at the world origin, which
        // is the window's center. The ambient is read now — dimmed to
        // where the professor's line becomes visible: a lamp is the
        // dimming's, and this is how far from it the floor falls to
        // its night color.
        camera: None,
        ambient: rgba(0.08, 0.08, 0.12),
    };
    // The ears are the table: every path the cargo carries, watched
    // where the loose world keeps it. Add a file to the table and
    // the courtyard hears it too — the watchers follow the cargo,
    // not a hand-kept list.
    let watchers = CARGO
        .iter()
        .map(|(key, _)| Watch::new(root.join(key)))
        .collect();
    let courtyard = Courtyard {
        lamp: None,
        boxes: rampart_boxes(&layers),
        root,
        watchers,
        teller: frost::Teller::open(TELL_DIR, "courtyard"),
        floors: layers.len(),
        reloads: 0,
    };
    if let Err(err) = frost::run(scene, courtyard) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(store: &Assets) -> impl Fn(&str) -> Option<String> + '_ {
        move |p| {
            store
                .bytes(p)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
        }
    }

    /// The telling, end to end and without a window: the courtyard's
    /// affair, published, is findable by a tool through the card —
    /// announced, read back, and speaking the map it stands on.
    /// (The live folder itself is exercised by the running apps;
    /// this pin runs in its own temp folder and leaves nothing.)
    #[test]
    fn the_courtyard_tells_what_stands() {
        // Claimed, not assumed: `temp_dir().join(pid)` — the naming this
        // test used until it ran for the first time — is the pattern
        // `frost::scratch` exists to retire, and under a shell that hands
        // cargo a POSIX `TMP` it resolves to a folder that cannot be
        // created at all. `Teller::open` swallows that (an app must not
        // die over a dev folder), so what the test was really asserting
        // was that the folder existed, and calling it silence.
        let dir = frost::scratch::claim("frost_courtyard_tell");
        let mut teller = frost::Teller::open(&dir, "courtyard-test");
        let stands = Courtyard {
            lamp: None,
            boxes: vec![([0.0, 0.0], [16.0, 16.0]); 36],
            root: std::path::PathBuf::from("/unused"),
            watchers: Vec::new(),
            teller: frost::Teller::open(&dir, "courtyard-test-reader"),
            floors: 2,
            reloads: 1,
        };
        assert!(
            teller.publish(&stands.being()),
            "a first telling is never silence"
        );
        let cards = frost::Teller::announce(&dir);
        let mine = cards
            .iter()
            .find(|c| c.name == "courtyard-test")
            .expect("the card is announced");
        let tide = frost::Teller::tiding(mine).expect("the live file answers");
        assert_eq!(tide.rev, 1, "one telling has been published");
        let body = frost::ron::to_text_doc("", &tide.being, "");
        assert!(
            body.contains("courtyard.map.ron"),
            "it speaks its map: {body}"
        );
        assert!(body.contains("walls: 36"), "it counts its rampart: {body}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The scoreboard's open box closes here: the first app that
    /// ships a map asserts its whole chain — map, every tileset
    /// document it points at, every sheet every tileset cuts — over
    /// its own table, without a filesystem in sight. A forgotten
    /// `include_bytes!` dies at `cargo test`, not in the field.
    #[test]
    fn the_courtyard_ships_its_whole_chain() {
        let store = Assets::embedded(CARGO);
        let src = read(&store)(MAP_PATH).expect("the map ships");
        let refused = map::guard_documents(MAP_PATH, &src, &read(&store));
        assert!(refused.is_empty(), "the cargo has a hole: {refused:?}");
    }

    /// The courtyard's meaning, chosen to survive its painter: the
    /// map resolves clean, both layers dress, and the floor and the
    /// rampart never stand on one cell. Exact counts are *not*
    /// pinned — the desk paints this file, and a living map's data
    /// owns its truth, as the pace file's does for `immortal`. The
    /// coordinate convention has its own pin, on a fixture no hand
    /// ever paints over, in `the_rampart_declares_itself_in_full`.
    #[test]
    fn the_floor_and_the_ring_are_where_they_are_painted() {
        let store = Assets::embedded(CARGO);
        let (layers, shelf, refused) = load_cargo(&store);
        assert!(refused.is_empty(), "refused: {refused:?}");
        assert!(!shelf.is_empty(), "the cargo's tilesets dress");
        assert_eq!(layers.len(), 2, "floor and rampart");
        let (floor, ring) = (&layers[0], &layers[1]);
        assert!(
            floor.cells.iter().any(|&c| c != map::EMPTY_CELL),
            "some floor stands"
        );
        assert!(
            ring.cells.iter().any(|&c| c != map::EMPTY_CELL),
            "some rampart stands"
        );
        assert!(
            floor
                .cells
                .iter()
                .zip(&ring.cells)
                .all(|(f, r)| !(*f != map::EMPTY_CELL && *r != map::EMPTY_CELL)),
            "no cell wears both offices"
        );
    }

    /// The solidity is the tileset's word, not the map's geometry:
    /// cell 3 stands in the world's way because the *document* says
    /// so — the claim rung 3's marble will hold the physics to.
    #[test]
    fn the_ring_is_solid_because_the_tileset_says_so() {
        let store = Assets::embedded(CARGO);
        let src = read(&store)("assets/sprites/basic_tiles.tileset.ron").expect("it ships");
        let (ts, refused) = tileset::from_text(&src);
        assert!(refused.is_empty(), "refused: {refused:?}");
        let ts = ts.expect("a tileset");
        assert!(ts.is_solid(1), "the rampart wedge stands in the way");
        assert!(!ts.is_solid(0), "the floor asks nothing of a collider");
        assert_eq!((ts.rows, ts.cols), (1, 4));
    }

    /// Two promises, each where it belongs. On the living map:
    /// boxes follow paint — one per rampart cell, 32 px on a side,
    /// however much or little the desk has painted today. On a
    /// fixture no hand paints over: the coordinate convention
    /// itself — a 2 × 1 map's north-west cell, the grid's first,
    /// stands at `[-16, 16]`, center-out, y up.
    #[test]
    fn the_rampart_declares_itself_in_full() {
        let store = Assets::embedded(CARGO);
        let (layers, _shelf, refused) = load_cargo(&store);
        assert!(refused.is_empty(), "refused: {refused:?}");
        let painted = layers
            .iter()
            .flat_map(|l| l.cells.iter())
            .filter(|&&c| c == RAMPART)
            .count();
        let boxes = rampart_boxes(&layers);
        assert_eq!(boxes.len(), painted, "every rampart cell declares one box");
        assert!(
            boxes.iter().all(|&(_, h)| h == [CELL_PX / 2.0; 2]),
            "every declaration is one cell on a side"
        );
        // A 2 × 2 fixture, because a one-row map has no north to
        // pin: the grid's first cell must be the north-west corner
        // of a grid that knows which way is up.
        let mut fixture = MapLayer::blank("fixture", 2, 2);
        fixture.cells[0] = RAMPART;
        let standing = rampart_boxes(std::slice::from_ref(&fixture));
        assert_eq!(standing.len(), 1, "one painted cell, one box");
        assert_eq!(
            standing[0],
            ([-CELL_PX / 2.0, CELL_PX / 2.0], [CELL_PX / 2.0; 2]),
            "the grid's first cell is the north-west corner: center-out, y up"
        );
    }
}
