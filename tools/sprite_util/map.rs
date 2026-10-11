//! The tile-map desk: the brush and its dressing, erasing and
//! the ghost. The facts this desk paints — tileset identities,
//! layers, cells, and the file that carries them — live in
//! `frost::map`, the format's home since the demo rung asked the
//! world to read maps; what stays here is motor skill: gestures no
//! file needs and no game should have to learn.
// The subjects this one reads.
use crate::art::asset_name;
use crate::clip::*;
use crate::layout::*;
use crate::world::*;
pub(crate) use frost::map::{EMPTY_CELL, MapLayer, cell_in, cell_uv, tileset_names};

/// Where a sheet lives, resolved from the tileset document that
/// names it — document-relative, the one dialect every reference in
/// these files speaks: a sheet beside its document says just
/// `"basic_tiles.png"`, and a document in `assets/sprites/` puts it
/// in `assets/sprites/`. Same pair of operations the map's own tileset
/// references ride in `open_map`: resolve, then join the directory.
pub(crate) fn sheet_beside(doc: &std::path::Path, sheet_ref: &str) -> std::path::PathBuf {
    let resolved = frost::map::resolve(&doc.to_string_lossy(), sheet_ref);
    doc.parent()
        .map(|dir| dir.join(&resolved))
        .unwrap_or_else(|| std::path::PathBuf::from(&resolved))
}

/// A tileset document on the desk's shelf: where it lives, and
/// what it says. A map opens its documents onto the shelf and a map
/// save writes them back — the document, not a sprite's momentary
/// atlas panel, is the cut's memory and the cells' roles' home.
#[derive(Clone, Debug)]
pub(crate) struct ShelfTileset {
    pub(crate) path: std::path::PathBuf,
    pub(crate) tileset: frost::tileset::Tileset,
}

/// The sprite a tileset cuts its pictures from, and the cut to
/// dress it with: the shelf document's word when one speaks for
/// the name (and the sheet it names must stand on the bench — a
/// cut without its picture paints nothing), the sprite's own atlas
/// declaration while the desk is still authoring the tileset and
/// no document has been written yet.
pub(crate) fn dressed<'a>(
    name: &str,
    shelf: &[ShelfTileset],
    sprites: &'a [Sprite],
) -> Option<(&'a Sprite, (usize, usize))> {
    if let Some(entry) = shelf.iter().find(|e| e.tileset.name == name) {
        let sheet = std::path::Path::new(&entry.tileset.sheet)
            .file_stem()?
            .to_string_lossy()
            .into_owned();
        let sp = sprites.iter().find(|sp| asset_name(&sp.path) == sheet)?;
        return Some((sp, (entry.tileset.rows, entry.tileset.cols)));
    }
    let sp = sprites.iter().find(|sp| asset_name(&sp.path) == name)?;
    Some((sp, sp.atlas.unwrap_or((1, 1))))
}

/// A layer dressed for the canvas: its shape from the tileset it
/// names — the shelf or the bench dressing it — or `None` while
/// the tileset waits off both.
pub(crate) fn layer_shape(
    layer: &MapLayer,
    shelf: &[ShelfTileset],
    sprites: &[Sprite],
) -> Option<frost::Shape> {
    let (sp, (rows, cols)) = dressed(&layer.tileset, shelf, sprites)?;
    let ts = frost::tileset::Tileset::new(layer.tileset.clone(), "", rows, cols);
    layer.shape(&ts, Some(&sp.shape))
}

/// The map a first click creates: 40 by 24 tiles at the origin — a
/// generous field one canvas wide at 1:1 zoom. A map-size control is
/// the ergonomics step's; the layer stores its own size, so the
/// default can change without disturbing any painted map.
pub(crate) const MAP_COLS: usize = 40;

pub(crate) const MAP_ROWS: usize = 24;

/// The cursor ghost's order: above the cells, below dragged things.
pub(crate) const GHOST_Z: f32 = 2.6;

/// The cursor ghost's opacity: a promise, not a commitment.
pub(crate) const GHOST_A: f32 = 0.55;

/// The map-space point's grid cell: columns to the right, rows
/// downward — the grid's rows run with the texture's, not the window's.
pub(crate) fn grid_cell(mx: f32, my: f32, tw: f32, th: f32) -> (i32, i32) {
    (mx.div_euclid(tw) as i32, (-my).div_euclid(th) as i32)
}

/// The bounds the brush reaches: the highlighted layer's own field,
/// or the map a first click would create.
pub(crate) fn paint_bounds(maps: &[MapLayer], at: usize) -> (usize, usize) {
    maps.get(at)
        .map_or((MAP_COLS, MAP_ROWS), |m| (m.cols, m.rows))
}

/// The ghost's list: the drop's cells, cut to the map — the very
/// intersection the drop applies, lifted out of the draw so the test
/// can stand in for the eye.
pub(crate) fn ghost_cells(
    clip: &Clip,
    blk: &Block,
    col: i32,
    row: i32,
    cols: usize,
    rows: usize,
) -> Vec<(i32, i32, u32, u8)> {
    block_cells(clip, blk, col, row)
        .into_iter()
        .filter(|(c, r, _, _)| cell_in(*c, *r, cols, rows))
        .collect()
}

/// Lift every cell of the selection, on every layer. One sweep, and
/// the caller's one undo step.
pub(crate) fn erase_rect(maps: &mut [MapLayer], sel: [i32; 4]) -> bool {
    let mut changed = false;
    for row in sel[1]..=sel[3] {
        for col in sel[0]..=sel[2] {
            changed |= erase_at(maps, col, row);
        }
    }
    changed
}

/// The cell under a window point on the desk: the map's own floors,
/// through the desk's scale and pan. Needs the active tileset, whose
/// atlas cuts the cell the grid draws with.
pub(crate) fn desk_cell(
    sp: Option<&Sprite>,
    p: [f32; 2],
    view: [f32; 2],
    zoom: f32,
) -> Option<(i32, i32)> {
    let sp = sp?;
    let (arows, acols) = sp.atlas.unwrap_or((1, 1));
    let tw = sp.current.width() as f32 / acols.max(1) as f32;
    let th = sp.current.height() as f32 / arows.max(1) as f32;
    Some(grid_cell(
        (p[0] - view[0]) / zoom,
        (p[1] - view[1]) / zoom,
        tw,
        th,
    ))
}

/// The paint's landing place is the highlighted layer — the one the
/// strip chose, by position — never "the first layer whose name
/// matches": two layers may wear one tileset, and the courtyard's do.
/// Only an empty map summons a layer into being, a fresh field
/// wearing the brush's tileset.
pub(crate) fn paint_at(
    maps: &mut Vec<MapLayer>,
    at: usize,
    tileset: &str,
    cell: usize,
    tfm: u8,
    col: i32,
    row: i32,
) -> bool {
    if maps.is_empty() {
        maps.push(MapLayer::blank(tileset, MAP_COLS, MAP_ROWS));
    }
    maps.get_mut(at)
        .is_some_and(|m| m.set_cell(col, row, cell as u32, tfm))
}

/// The tile-map base a sprite's shape lends to a ghost: the very
/// texture, emptied of placements, ready to wear tiles. A shape
/// that is neither sprite nor tile map lends nothing — and the
/// call site drops that sheet's ghost rather than hang an
/// undressed sprite on the cursor. (`set_tiles` answers only to a
/// tile map: handing it a bare sprite loses every placement in
/// silence — the ghost that started this file's care.)
pub(crate) fn ghost_base(shape: &frost::Shape) -> Option<frost::Shape> {
    let base = match shape {
        frost::Shape::Sprite {
            data,
            width,
            height,
            filter,
            generation,
            ..
        } => frost::Shape::TileMap {
            data: data.clone(),
            width: *width,
            height: *height,
            generation: *generation,
            filter: *filter,
            tiles: Vec::new(),
            clip: None,
            color: frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
        },
        frost::Shape::TileMap { .. } => shape.clone(),
        _ => return None,
    };
    Some(base)
}

/// The bench's right-click: raise layer `i` one place in the draw
/// stack — the top wraps under, so the stack cycles and nothing ever
/// leaves it. Answers with where the moved layer came to rest, and
/// where the highlight now stands: the highlight rides with its
/// layer over its own move, and steps aside over everyone else's.
pub(crate) fn raise_layer(
    maps: &mut Vec<MapLayer>,
    i: usize,
    active: usize,
) -> Option<(usize, usize)> {
    let n = maps.len();
    if n < 2 || i >= n {
        return None;
    }
    if i + 1 == n {
        let m = maps.remove(i);
        maps.insert(0, m);
        Some((0, if active == i { 0 } else { active + 1 }))
    } else {
        maps.swap(i, i + 1);
        let a = if active == i {
            i + 1
        } else if active == i + 1 {
            i
        } else {
            active
        };
        Some((i + 1, a))
    }
}

/// The gesture table: what the pointer's modifiers mean as a dress
/// verb. Every arm consults it — the rack's pick, the desk's brush,
/// the held block, the keyboard's R — because the direction saga
/// ended with one arm wired by hand, differently from its siblings.
/// Plain turns counterclockwise (the drawn expectation), Shift turns
/// back clockwise, Alt mirrors along the horizontal, Control along
/// the vertical; with modifiers stacked, Alt leads, then Control,
/// then Shift.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub(crate) enum Dress {
    FlipX,
    FlipY,
    /// A quarter turn: true is clockwise.
    Turn(bool),
}

pub(crate) fn dress_gesture(shift: bool, ctrl: bool, alt: bool) -> Dress {
    if alt {
        Dress::FlipY
    } else if ctrl {
        Dress::FlipX
    } else {
        Dress::Turn(shift)
    }
}

/// Apply a gesture's verb to a brush tile. Every brush arm — rack,
/// desk, keyboard — lands here, and so does the test: there is no
/// second copy of the routing anywhere to drift.
pub(crate) fn dress_brush(brush: u8, d: Dress) -> u8 {
    match d {
        Dress::FlipY => tfm_flip_y(brush),
        Dress::FlipX => tfm_flip_x(brush),
        Dress::Turn(cw) => tfm_turn(brush, cw),
    }
}

/// Apply a gesture's verb to a held block: the whole picture —
/// arrangement and every tile — as one; a turn moves the selection
/// frame with it.
pub(crate) fn dress_block(clip: &mut Clip, sel: &mut Option<[i32; 4]>, d: Dress) {
    match d {
        Dress::FlipY => clip_flip(clip, false),
        Dress::FlipX => clip_flip(clip, true),
        Dress::Turn(cw) => {
            clip_turn(clip, cw);
            *sel = sel.map(sel_turn);
        }
    }
}

/// Turn a packed orientation one quarter — clockwise, or back. The
/// flips ride along untouched: they name the screen's axes, so a
/// turned, mirrored cell is still mirrored sideways, and turning has
/// no reason to rearrange them.
pub(crate) fn tfm_turn(code: u8, cw: bool) -> u8 {
    let rot = ((code >> 2) & 3) + if cw { 1 } else { 3 };
    (code & 3) | ((rot & 3) << 2)
}

/// Flip the tile's picture sideways — on SCREEN axes, whatever the
/// tile's turn: the shader walks the turns first and lays the flip
/// bits down afterwards, so a tile turned an odd quarter mirrors
/// along the OTHER bit. A golden test pins this byte-for-byte
/// against the real rotated pixels.
pub(crate) fn tfm_flip_x(code: u8) -> u8 {
    code ^ if (code >> 2) & 1 == 1 { 2 } else { 1 }
}

/// Flip the tile's picture upright-mirrored: same story as
/// `tfm_flip_x`, one axis over.
pub(crate) fn tfm_flip_y(code: u8) -> u8 {
    code ^ if (code >> 2) & 1 == 1 { 1 } else { 2 }
}

/// The brush's turn in words, for the status line and the rack.
pub(crate) fn turn_note(code: u8) -> &'static str {
    match (code >> 2) & 3 {
        0 => "upright",
        1 => "90 deg cw",
        2 => "upside down",
        _ => "90 deg ccw",
    }
}

/// Clear one cell on every layer: the eraser lifts what stands there,
/// whichever tileset painted it. True when anything came off.
pub(crate) fn erase_at(maps: &mut [MapLayer], col: i32, row: i32) -> bool {
    // `fold`, not `any`: the eraser must visit every layer even after
    // one gave up its cell — short-circuiting here would leave the
    // other tilesets' paint standing under the cursor.
    maps.iter_mut().fold(false, |changed, m| {
        changed | m.set_cell(col, row, EMPTY_CELL, 0)
    })
}

/// The ghost node's transform: the tile scaled into the cell's world
/// center — scale FIRST, pan after, which is the desk's own order.
/// Composing them the other way round scales the pan too, and the
/// ghost drifts away from the cursor as the zoom moves off 1.
pub(crate) fn ghost_transform(zoom: f32, center: [f32; 2]) -> frost::Transform {
    frost::Transform::scale([zoom, zoom]).compose(&frost::Transform::translate(center))
}

/// The grid's drawn step, in map units: the cell's own pitch, doubled
/// up the ladder while its lines would stand closer than
/// `GRID_MIN_PX` window pixels apart. Always a whole power-of-two
/// count of CELLS — that is the point: a step merely clamped to the
/// pixel floor would slide the lines off the cell boundaries, and a
/// grid the ghost no longer stands on is a lie about the map.
pub(crate) fn grid_step(cell: f32, zoom: f32) -> f32 {
    let mut s = cell;
    while s * zoom < GRID_MIN_PX {
        s *= 2.0;
    }
    s
}

/// The block's dress, byte-for-byte honest: composite the block's
/// tiles into a real picture the same way the shader samples them,
/// transform THAT with the image crate (the honest rotation and
/// mirror), and rebuild the block with the demo's rules. The two
/// composites must match to the pixel — the test that pinned the
/// flip-conjugation rule after two algebraic guesses missed it.
#[cfg(test)]
mod the_block_dresses_like_the_picture {
    use super::*;

    /// Render one tile the way the shader does: the walk first, the
    /// flip bits after, sampling the atlas cell (both planes y-down,
    /// as `quad_uv` is).
    fn render_tile(
        atlas: &image::RgbaImage,
        ar: usize,
        ac: usize,
        cell: u32,
        code: u8,
        tw: u32,
        th: u32,
    ) -> image::RgbaImage {
        let (aw, ah) = atlas.dimensions();
        let (row, col) = (cell as usize / ac, cell as usize % ac);
        let mut out = image::RgbaImage::from_pixel(tw, th, image::Rgba([0, 0, 0, 0]));
        for y in 0..th {
            for x in 0..tw {
                let mut q = ((x as f32 + 0.5) / tw as f32, (y as f32 + 0.5) / th as f32);
                match (code >> 2) & 3 {
                    1 => q = (q.1, 1.0 - q.0),
                    2 => q = (1.0 - q.0, 1.0 - q.1),
                    3 => q = (1.0 - q.1, q.0),
                    _ => {}
                }
                if code & 1 != 0 {
                    q.0 = 1.0 - q.0;
                }
                if code & 2 != 0 {
                    q.1 = 1.0 - q.1;
                }
                let sx = ((col as f32 + q.0) * (aw / ac as u32) as f32) as u32;
                let sy = ((row as f32 + q.1) * (ah / ar as u32) as f32) as u32;
                out.put_pixel(x, y, *atlas.get_pixel(sx.min(aw - 1), sy.min(ah - 1)));
            }
        }
        out
    }

    /// A synthetic atlas with zero symmetry in every cell, so two
    /// different pictures can never alias into a passing match.
    fn wild_atlas() -> image::RgbaImage {
        let mut img = image::RgbaImage::new(128, 32);
        for y in 0..32u32 {
            for x in 0..128u32 {
                let cell = x / 32;
                let lx = x % 32;
                img.put_pixel(
                    x,
                    y,
                    image::Rgba([
                        (lx * 8 + cell * 3) as u8,
                        (y * 8) as u8,
                        ((lx * lx + y) % 251) as u8,
                        255,
                    ]),
                );
            }
        }
        img
    }

    fn composite(clip: &Clip, atlas: &image::RgbaImage) -> image::RgbaImage {
        let (ar, ac, tw, th) = (1usize, 4usize, 32u32, 32u32);
        let mut img = image::RgbaImage::new((clip.cols as u32) * tw, (clip.rows as u32) * th);
        for p in img.pixels_mut() {
            *p = image::Rgba([0, 0, 0, 0]);
        }
        let blk = &clip.blocks[0];
        for (i, c) in blk.cells.iter().enumerate() {
            if *c == EMPTY_CELL {
                continue;
            }
            let t = render_tile(atlas, ar, ac, *c, blk.tfms[i], tw, th);
            for y in 0..th {
                for x in 0..tw {
                    img.put_pixel(
                        (i % clip.cols) as u32 * tw + x,
                        (i / clip.cols) as u32 * th + y,
                        *t.get_pixel(x, y),
                    );
                }
            }
        }
        img
    }

    fn equal(a: &image::RgbaImage, b: &image::RgbaImage) -> bool {
        a.dimensions() == b.dimensions() && a.pixels().zip(b.pixels()).all(|(x, y)| x == y)
    }

    /// A deliberately uneven block: turned, mirrored, and plain
    /// tiles mixed, with a gap and three different tiles.
    fn demo_clip() -> Clip {
        Clip {
            cols: 3,
            rows: 2,
            blocks: vec![Block {
                layer: 0,
                cells: vec![0, 1, 2, 3, 1, EMPTY_CELL],
                tfms: vec![0, 1, 2, 4, 5, 7],
            }],
        }
    }

    #[test]
    fn every_dress_matches_the_real_pixels() {
        let atlas = wild_atlas();
        let clip = demo_clip();
        let base = composite(&clip, &atlas);
        let mut c = clip.clone();
        clip_turn(&mut c, true);
        assert!(
            equal(&composite(&c, &atlas), &image::imageops::rotate90(&base)),
            "a cw block turn must equal a cw picture turn"
        );
        let mut c = clip.clone();
        clip_turn(&mut c, false);
        assert!(
            equal(&composite(&c, &atlas), &image::imageops::rotate270(&base)),
            "a ccw block turn must equal a ccw picture turn"
        );
        let mut c = clip.clone();
        clip_flip(&mut c, true);
        assert!(
            equal(
                &composite(&c, &atlas),
                &image::imageops::flip_horizontal(&base)
            ),
            "a block flip-x must mirror the picture sideways"
        );
        let mut c = clip;
        clip_flip(&mut c, false);
        assert!(
            equal(
                &composite(&c, &atlas),
                &image::imageops::flip_vertical(&base)
            ),
            "a block flip-y must mirror the picture upright"
        );
    }
}

/// The names a map asks for that neither the shelf nor the bench
/// can dress — what `open_map` names in its note: the guard's
/// other face, the one the tool can answer at the opening. A
/// document whose sheet closed waits as much as a name nobody
/// wears: half a dressing dresses nothing.
pub(crate) fn waiting_names<'a>(
    layers: &'a [MapLayer],
    shelf: &[ShelfTileset],
    sprites: &'a [Sprite],
) -> Vec<&'a str> {
    let mut waiting: Vec<&'a str> = Vec::new();
    for layer in layers {
        if dressed(&layer.tileset, shelf, sprites).is_none()
            && !waiting.contains(&layer.tileset.as_str())
        {
            waiting.push(&layer.tileset);
        }
    }
    waiting
}

#[cfg(test)]
mod tests {
    /// The highlight decides where the brush lands, not the name:
    /// two layers may wear one tileset, and the cell reaches only
    /// the one standing under the highlight.
    #[test]
    fn the_highlighted_layer_takes_the_paint() {
        let mut maps = vec![
            MapLayer::blank("stones", 2, 2),
            MapLayer::blank("stones", 2, 2),
        ];
        assert!(paint_at(&mut maps, 1, "stones", 3, 0, 0, 0));
        // Read back by grid coordinate: the cells store from the
        // map's top-left, while coordinates radiate from the center.
        let here = maps[1].at(0, 0).unwrap();
        assert_eq!(
            maps[0].cells[here], EMPTY_CELL,
            "the twin below keeps its field"
        );
        assert_eq!(maps[1].cells[here], 3);
    }

    /// One place up, the top wrapping under; the highlight rides
    /// with its layer, and a stack of one has nowhere to go.
    #[test]
    fn raising_a_layer_cycles_the_stack() {
        fn names(ms: &[MapLayer]) -> Vec<&str> {
            ms.iter().map(|m| m.tileset.as_str()).collect()
        }
        let mut maps = vec![
            MapLayer::blank("a", 2, 2),
            MapLayer::blank("b", 2, 2),
            MapLayer::blank("c", 2, 2),
        ];
        assert_eq!(raise_layer(&mut maps, 0, 0), Some((1, 1)));
        assert_eq!(names(&maps), ["b", "a", "c"]);
        assert_eq!(raise_layer(&mut maps, 2, 1), Some((0, 2)));
        assert_eq!(names(&maps), ["c", "b", "a"]);
        assert_eq!(raise_layer(&mut maps, 0, 2), Some((1, 2)));
        assert_eq!(names(&maps), ["b", "c", "a"]);
        let mut one = vec![MapLayer::blank("solo", 2, 2)];
        assert_eq!(raise_layer(&mut one, 0, 0), None);
    }
    /// The seam `open_map` walks for every tileset it shelves: the
    /// sheet reference is the document's word, resolved from the
    /// document's own place — beside it when named bare, and up
    /// through folders when the author shelves them apart.
    #[test]
    fn a_sheet_resolves_from_the_document_that_names_it() {
        let beside = std::path::Path::new("/work/assets/sprites/basic_tiles.tileset.ron");
        assert_eq!(
            sheet_beside(beside, "basic_tiles.png"),
            std::path::PathBuf::from("/work/assets/sprites/basic_tiles.png")
        );
        let apart = std::path::Path::new("/work/assets/tilesets/deep/yard.tileset.ron");
        assert_eq!(
            sheet_beside(apart, "../sheets/yard.png"),
            std::path::PathBuf::from("/work/assets/tilesets/sheets/yard.png")
        );
    }
    use super::*;

    /// One sheet, two documents: that is what two tilesets are now.
    /// The wide document says 1 x 4 of the very same sheet the
    /// sprite wears as 2 x 2 — and each name is dressed by its own
    /// document's word, while a name neither shelf nor bench speaks
    /// waits, and a document whose sheet closed waits likewise: a
    /// cut without its picture paints nothing.
    #[test]
    fn one_sheet_two_documents_stay_two_tilesets() {
        let sheet = tileset("same.png"); // the fixture wears 2 x 2
        let wide = ShelfTileset {
            path: "/shelf/wide.tileset.ron".into(),
            tileset: frost::tileset::Tileset::new("wide", "same.png", 1, 4),
        };
        let shelf = vec![wide];
        let sprites = vec![sheet];
        let (_, cut) = dressed("wide", &shelf, &sprites).expect("the sheet stands on the bench");
        assert_eq!(cut, (1, 4), "the document says how its numbers cut");
        let (_, cut) = dressed("same", &[], &sprites).expect("the name is on the bench");
        assert_eq!(
            cut,
            (2, 2),
            "authoring without a document wears the sprite's own claim"
        );
        assert!(
            dressed("tall", &shelf, &sprites).is_none(),
            "nobody speaks tall"
        );
        assert!(
            dressed("wide", &shelf, &[]).is_none(),
            "the sheet closed; the cut alone paints nothing"
        );
    }

    #[test]
    fn the_ghost_transform_scales_then_pans() {
        // The ghost must ride the desk's own order: the tile's local
        // center scales into the cell's world center. The reversed
        // composition scales the pan as well — the very drift the
        // desk was built to avoid.
        let (zoom, view) = (0.4, [30.0, -12.0]);
        let (tw, row, col) = (32.0, -1, 2);
        let center = [
            (col as f32 + 0.5) * tw * zoom + view[0],
            -((row as f32 + 0.5) * tw * zoom) + view[1],
        ];
        let t = ghost_transform(zoom, center);
        let p = t.apply([0.0, 0.0]);
        assert!((p[0] - center[0]).abs() < 1e-3 && (p[1] - center[1]).abs() < 1e-3);
        // A tile's edge: half a zoomed tile right of the center.
        let e = t.apply([tw / 2.0, 0.0]);
        assert!((e[0] - (center[0] + tw / 2.0 * zoom)).abs() < 1e-3);
    }

    #[test]
    fn paint_rides_the_button_and_the_eraser_rides_the_key() {
        let at = |erase: bool, key_opened: bool| Paint {
            erase,
            touched: false,
            n: 0,
            key_opened,
        };
        // Paint: the left button alone opens and keeps the stroke —
        // with or without Delete, which never gates painting.
        assert!(stroke_held(&at(false, false), true, false));
        assert!(stroke_held(&at(false, false), true, true));
        assert!(!stroke_held(&at(false, false), false, true));
        // The eraser a button opened: key and button together.
        assert!(stroke_held(&at(true, false), true, true));
        assert!(!stroke_held(&at(true, false), true, false));
        assert!(!stroke_held(&at(true, false), false, true));
        // The eraser the key opened: the key alone sweeps, and
        // letting go ends the stroke wherever the mouse is.
        assert!(stroke_held(&at(true, true), true, true));
        assert!(stroke_held(&at(true, true), false, true));
        assert!(!stroke_held(&at(true, true), true, false));
        assert!(!stroke_held(&at(true, true), false, false));
    }

    #[test]
    fn the_grid_ladder_stands_on_whole_cells() {
        // Whatever the tile and whatever the zoom, the drawn grid
        // keeps its lines on cell boundaries — a power-of-two count
        // of whole cells — and keeps them a readable distance apart.
        for &cell in &[32.0f32, 24.0, 16.0, 7.0] {
            for &zoom in &[4.0f32, 1.0, 0.5, 0.3, 0.25, 0.0625] {
                let step = grid_step(cell, zoom);
                assert!(
                    step * zoom >= GRID_MIN_PX - 1e-4,
                    "{cell} px at {zoom} zoom drew {step} apart"
                );
                let k = (step / cell).log2().round();
                assert!(
                    k >= 0.0 && (step / cell - 2f32.powf(k)).abs() < 1e-3,
                    "{cell} px at {zoom} zoom drew {step}: not a ladder of cells"
                );
            }
        }
    }

    #[test]
    fn four_clockwise_quarters_come_home_and_the_flips_ride() {
        let mut c = 0u8;
        for _ in 0..4 {
            c = tfm_turn(c, true);
        }
        assert_eq!(c, 0, "a full turn is the identity");
        assert_eq!(tfm_turn(tfm_turn(0, true), false), 0, "back and forth");
        let turned = tfm_turn(1 | (2 << 2), true); // flip x, one turn on
        assert_eq!(turned & 3, 1, "the flip survived the turn");
        assert_eq!((turned >> 2) & 3, 3, "and the turn counted");
    }

    #[test]
    fn grid_cell_hands_the_stroke_the_same_cell_as_the_ghost() {
        // The i32 face of the snapping, used by the actual strokes:
        // floors, including through the negative side.
        assert_eq!(grid_cell(1.0, 1.0, 32.0, 16.0), (0, -1));
        assert_eq!(grid_cell(-33.0, 17.0, 32.0, 16.0), (-2, -2));
        assert_eq!(grid_cell(31.9, -0.1, 32.0, 16.0), (0, 0));
    }

    #[test]
    fn the_plain_turn_is_the_drawn_one() {
        // The expectation, drawn by hand: a row of the atlas's cells
        // 2 and 3, turned plain (right-click, no modifier), becomes a
        // column with cell 3 on top and cell 2 below, each dressed a
        // quarter counterclockwise — rot + 3, no flips. Plain rides
        // clip_turn's counterclockwise arm; the drawn picture is the
        // oracle, so this test is the contract.
        let mut clip = Clip {
            cols: 2,
            rows: 1,
            blocks: vec![Block {
                layer: 0,
                cells: vec![2, 3],
                tfms: vec![0, 0],
            }],
        };
        clip_turn(&mut clip, false);
        assert_eq!((clip.cols, clip.rows), (1, 2), "the pair stands up");
        assert_eq!(clip.blocks[0].cells, vec![3, 2], "cell 3 rides to the top");
        assert_eq!(
            clip.blocks[0].tfms,
            vec![12, 12],
            "every tile turns counterclockwise"
        );
    }

    #[test]
    fn the_gesture_table_is_the_drawn_contract() {
        // Every arm reads this one table; the table itself is the
        // contract the drawn expectations fixed: plain turns
        // counterclockwise, Shift turns back, the mirrors keep their
        // keys, and stacked modifiers have a stated order.
        assert_eq!(dress_gesture(false, false, false), Dress::Turn(false));
        assert_eq!(dress_gesture(true, false, false), Dress::Turn(true));
        assert_eq!(dress_gesture(false, false, true), Dress::FlipY);
        assert_eq!(dress_gesture(false, true, false), Dress::FlipX);
        assert_eq!(dress_gesture(true, true, true), Dress::FlipY);
        assert_eq!(dress_gesture(true, true, false), Dress::FlipX);
    }

    #[test]
    fn block_and_brush_never_disagree() {
        // One gesture, two arms: the brush's byte rule and the
        // block's whole-picture rule must dress a one-cell block
        // exactly as they dress the brush — same table, same
        // outcome, every code, every gesture row.
        let rows = [
            (false, false, false),
            (true, false, false),
            (false, false, true),
            (false, true, false),
            (true, true, true),
        ];
        for code in 0..16u8 {
            for (shift, ctrl, alt) in rows {
                let d = dress_gesture(shift, ctrl, alt);
                let brush = dress_brush(code, d);
                let mut clip = Clip {
                    cols: 1,
                    rows: 1,
                    blocks: vec![Block {
                        layer: 0,
                        cells: vec![1],
                        tfms: vec![code],
                    }],
                };
                let mut sel = None;
                dress_block(&mut clip, &mut sel, d);
                assert_eq!(
                    clip.blocks[0].tfms,
                    vec![brush],
                    "code {code}, gesture {d:?}"
                );
                assert_eq!(clip.blocks[0].cells, vec![1], "the lone cell stays put");
            }
        }
    }

    #[test]
    fn the_ghost_and_the_drop_are_one_list() {
        // Ghost and paste consume the same clip_cells: the promise
        // can never show a cell the drop will skip — least of all
        // where the block hangs past a map edge.
        let clip = Clip {
            cols: 3,
            rows: 2,
            blocks: vec![Block {
                layer: 0,
                cells: vec![0, 1, 2, 3, 1, EMPTY_CELL],
                tfms: vec![0, 1, 2, 4, 5, 7],
            }],
        };
        let cells = clip_cells(&clip, &[true], 5, 7);
        assert_eq!(cells.len(), 5, "the gap does not travel");
        assert!(
            cells.iter().any(|(_, c, r, ..)| (*c, *r) == (5, 7)),
            "the anchor lands on the cursor"
        );
        // The rim: cursor at the map's right edge, cells hanging
        // past it. The drop — and with it the preview — keeps
        // exactly the part that fits, measured by the engine's own
        // bounds predicate: grid coordinates radiate from the map's
        // center (cell_in, not a hand-drawn range — the first
        // version of this test drew one, at 0..8, and failed honest:
        // the engine was right and the test was wrong).
        let rim = clip_cells(&clip, &[true], 3, 0);
        let fits = rim
            .iter()
            .filter(|(_, c, r, _, _)| cell_in(*c, *r, 8, 8))
            .count();
        assert!(
            fits > 0 && fits < rim.len(),
            "the rim really clips some cells"
        );
        let mut maps = vec![MapLayer::blank("wild", 8, 8)];
        let n = clip_paste(&mut maps, &[true], &clip, 3, 0) as usize;
        assert_eq!(n, fits, "the promise and the drop agree at the edge");
        let written = (-4..4)
            .flat_map(|row| (-4..4).map(move |col| (col, row)))
            .filter(|(c, r)| {
                maps[0]
                    .at(*c, *r)
                    .is_some_and(|i| maps[0].cells[i] != EMPTY_CELL)
            })
            .count();
        assert_eq!(n, written, "every counted cell is a written cell");
        // And the preview itself: the ghost's list is the drop's
        // list, cut to the map — same cells, same order, same
        // dressings.
        assert_eq!(
            ghost_cells(&clip, &clip.blocks[0], 3, 0, 8, 8),
            block_cells(&clip, &clip.blocks[0], 3, 0)
                .into_iter()
                .filter(|(c, r, _, _)| cell_in(*c, *r, 8, 8))
                .collect::<Vec<_>>(),
            "the promise IS the cut-down drop"
        );
    }

    #[test]
    fn the_selection_erase_sweeps_every_layer() {
        let mut maps = vec![MapLayer::blank("a", 4, 4), MapLayer::blank("b", 4, 4)];
        maps[0].set_cell(0, 0, 7, 1);
        maps[1].set_cell(1, 1, 9, 0);
        assert!(erase_rect(&mut maps, [0, 0, 1, 1]));
        for m in &maps {
            for i in 0..16 {
                assert_eq!(m.cells[i], EMPTY_CELL);
            }
        }
        // And a sweep over an empty rectangle changes nothing, so it
        // takes no place on the undo road.
        assert!(!erase_rect(&mut maps, [0, 0, 1, 1]));
    }

    /// Waiting is the absence of a dressing: a document whose
    /// sheet closed waits as much as a name nobody wears — and each
    /// waits once, named, for the status band to speak.
    #[test]
    fn a_waiting_tileset_is_named_once() {
        let moss_doc = ShelfTileset {
            path: "/shelf/moss.tileset.ron".into(),
            tileset: frost::tileset::Tileset::new("moss", "moss.png", 1, 4),
        };
        let layers = vec![
            MapLayer::blank("grass", 4, 2),
            MapLayer::blank("moss", 4, 2),
            MapLayer::blank("bug", 4, 2),
            MapLayer::blank("moss", 4, 2),
        ];
        // The moss document speaks for the name, but moss.png is
        // off the bench: half a dressing is a waiter.
        assert_eq!(
            waiting_names(
                &layers,
                std::slice::from_ref(&moss_doc),
                &[tileset("grass.png")]
            ),
            ["moss", "bug"],
            "a closed sheet waits as much as a missing name, and waits once"
        );
        assert!(
            waiting_names(
                &layers,
                &[moss_doc],
                &[
                    tileset("grass.png"),
                    tileset("moss.png"),
                    tileset("bug.png")
                ]
            )
            .is_empty(),
            "a desk wearing every sheet has no waiters"
        );
    }

    /// A sprite's shape lends a ghost its texture — as a tile map.
    /// The old bug slipped `set_tiles` onto the bare sprite, where it
    /// did nothing: no placements, no dressing, no alpha. The base a
    /// ghost gets must WEAR the tiles.
    #[test]
    fn the_ghost_base_wears_what_it_is_handed() {
        use std::sync::Arc;
        let sprite = frost::Shape::Sprite {
            data: Arc::from(vec![0u8; 4 * 2 * 2]),
            width: 2,
            height: 2,
            color: frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            alpha: 1.0,
            filter: frost::SpriteFilter::Nearest,
            generation: 7,
        };
        let Some(mut base) = ghost_base(&sprite) else {
            panic!("a sprite lends a base");
        };
        base.set_tiles(vec![frost::Tile::new(
            [0.0, 0.0],
            [1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
        )]);
        let frost::Shape::TileMap { tiles, .. } = &base else {
            panic!("the base is a tile map");
        };
        assert_eq!(tiles.len(), 1, "the placement stuck");
        // A tile map lends itself, whole.
        assert!(ghost_base(&base).is_some());
        // Anything else lends nothing.
        assert!(
            ghost_base(&frost::Shape::Circle {
                center: [0.0, 0.0],
                radius: 1.0,
                color: frost::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0
                },
            })
            .is_none()
        );
    }
}
