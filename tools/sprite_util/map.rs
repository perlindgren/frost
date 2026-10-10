//! The tile-map desk: layers of cells, the brush and its
//! dressing, erasing and the ghost.
// The subjects this one reads.
use crate::art::asset_name;
use crate::clip::*;
use crate::layout::*;
use crate::world::*;

/// A tileset's identity: the file's name and the cut made into it.
/// The same PNG cut two ways is two tilesets — the same cell code
/// carries different pictures under each — while name and cut alike is
/// one tileset, whichever slot it sits on. By name (the file stem, not
/// the path) because the identity must outlive slots, folders, and the
/// filesystem itself: a shipped app names its embedded bytes exactly
/// so, and a path would die at the binary's edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TilesetRef {
    pub(crate) name: String,
    pub(crate) rows: usize,
    pub(crate) cols: usize,
}

impl TilesetRef {
    /// The identity of a bench sprite: its name, and the atlas
    /// declaration as the cut — a sprite with no declaration is one
    /// whole cell, the cut every unpainted sprite secretly has.
    pub(crate) fn of(sp: &Sprite) -> Self {
        let (rows, cols) = sp.atlas.unwrap_or((1, 1));
        Self {
            name: asset_name(&sp.path),
            rows,
            cols,
        }
    }

    /// The bench sprite wearing this tileset — same name, same cut —
    /// or `None` while it sits off the bench. The cut is part of the
    /// match because one file cut two ways dresses two ways: name
    /// alone would let the first same-path slot win, which is the
    /// exact bug this type exists to kill.
    pub(crate) fn sprite<'a>(&self, sprites: &'a [Sprite]) -> Option<&'a Sprite> {
        sprites.iter().find(|sp| {
            asset_name(&sp.path) == self.name
                && sp.atlas.unwrap_or((1, 1)) == (self.rows, self.cols)
        })
    }
}

/// One tileset's worth of painted cells, anchored on the map's origin:
/// a grid of atlas-cell indices, `EMPTY_CELL` where nothing stands.
/// A multi-tileset picture is several of these layers, each naming its
/// tileset by name and cut — the desk paints one, the canvas shows
/// them all.
#[derive(Clone, PartialEq)]
pub(crate) struct MapLayer {
    /// The tileset the cells cut their pictures from. By identity,
    /// because slots shift when sprites close and a map outlives that
    /// shuffle — and the files it names outlive the folders too.
    pub(crate) tileset: TilesetRef,
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) cells: Vec<u32>,
    /// Each cell's orientation, painted with the cell and erased with
    /// it: bit 0 flip-x, bit 1 flip-y, bits 2..3 clockwise quarter-
    /// turns. The transform is the map's memory, not the brush's mood
    /// — which is why it lives here, inside the layer, and rides the
    /// undo road in every snapshot of the world.
    pub(crate) tfms: Vec<u8>,
}

/// How many tileset cells the band shows at most; a bigger atlas waits
/// for the band's own scrolling (a later step).
/// A cell with nothing in it. Real atlas cells count from 0.
pub(crate) const EMPTY_CELL: u32 = u32::MAX;

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

impl MapLayer {
    pub(crate) fn blank(tileset: TilesetRef, cols: usize, rows: usize) -> Self {
        Self {
            tileset,
            cols,
            rows,
            cells: vec![EMPTY_CELL; cols * rows],
            tfms: vec![0; cols * rows],
        }
    }

    /// The storage slot of grid column/row — the map's cells are
    /// numbered from its top-left, while grid coordinates radiate
    /// from the origin, which sits at the map's center.
    pub(crate) fn at(&self, col: i32, row: i32) -> Option<usize> {
        cell_in(col, row, self.cols, self.rows)
            .then(|| (row + self.rows as i32 / 2) * self.cols as i32 + (col + self.cols as i32 / 2))
            .map(|i| i as usize)
    }

    /// Paint over one cell — picture and orientation together; true
    /// when the cell actually changed. A repaint that only turns the
    /// tile IS a change; an eraser on an empty cell is not, whatever
    /// orientation bits it carries.
    pub(crate) fn set_cell(&mut self, col: i32, row: i32, cell: u32, tfm: u8) -> bool {
        match self.at(col, row) {
            Some(i) => {
                let erased = cell == EMPTY_CELL;
                let changed = self.cells[i] != cell || (!erased && self.tfms[i] != tfm);
                if changed {
                    self.cells[i] = cell;
                    self.tfms[i] = if erased { 0 } else { tfm };
                }
                changed
            }
            _ => false,
        }
    }

    /// The layer as a `TileMap` shape sharing the tileset's pixels —
    /// or `None` when the tileset is closed (the layer waits for its
    /// source to reopen) or holds nothing (an invisible layer asks
    /// nothing of the renderer). Cells past the tileset's current
    /// grid — one shrank since they were painted — are skipped.
    pub(crate) fn shape(&self, tileset: Option<&Sprite>) -> Option<frost::Shape> {
        let sp = tileset?;
        let (data, width, height, generation) = match &sp.shape {
            frost::Shape::Sprite {
                data,
                width,
                height,
                generation,
                ..
            } => (data.clone(), *width, *height, *generation),
            _ => return None,
        };
        let (arows, acols) = sp.atlas.unwrap_or((1, 1));
        let (tw, th) = (
            width as f32 / acols.max(1) as f32,
            height as f32 / arows.max(1) as f32,
        );
        let tiles: Vec<frost::Tile> = self
            .cells
            .iter()
            .enumerate()
            .filter_map(|(i, &c)| {
                (c != EMPTY_CELL && (c as usize) < arows * acols).then_some((i, c))
            })
            .map(|(i, c)| {
                let gx = (i % self.cols) as f32 - self.cols as f32 / 2.0 + 0.5;
                let gy = (i / self.cols) as f32 - self.rows as f32 / 2.0 + 0.5;
                let t = self.tfms[i];
                frost::Tile::new(
                    [gx * tw, -gy * th],
                    [tw, th],
                    cell_uv(c as usize, arows, acols),
                )
                .transformed((t >> 2) & 3, t & 1 != 0, t & 2 != 0)
            })
            .collect();
        if tiles.is_empty() {
            return None;
        }
        let mut shape = frost::Shape::TileMap {
            data,
            width,
            height,
            generation,
            filter: frost::SpriteFilter::Nearest,
            tiles: Vec::new(),
            clip: None,
            color: frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
        };
        shape.set_tiles(tiles);
        Some(shape)
    }
}

/// Whether grid column/row lies on a `cols`-by-`rows` map centered on
/// the origin. Even dimensions make the grid half-open on the right
/// and bottom: it spans x in [-w/2, w/2) like the cells' own floors.
pub(crate) fn cell_in(col: i32, row: i32, cols: usize, rows: usize) -> bool {
    col + cols as i32 / 2 >= 0
        && row + rows as i32 / 2 >= 0
        && col + cols as i32 / 2 < cols as i32
        && row + rows as i32 / 2 < rows as i32
}

/// The map-space point's grid cell: columns to the right, rows
/// downward — the grid's rows run with the texture's, not the window's.
pub(crate) fn grid_cell(mx: f32, my: f32, tw: f32, th: f32) -> (i32, i32) {
    (mx.div_euclid(tw) as i32, (-my).div_euclid(th) as i32)
}

/// The bounds the brush reaches with this tileset: its own layer if
/// one exists, else the map a first click would create.
pub(crate) fn paint_bounds(maps: &[MapLayer], tileset: &TilesetRef) -> (usize, usize) {
    maps.iter()
        .find(|m| &m.tileset == tileset)
        .map_or((MAP_COLS, MAP_ROWS), |m| (m.cols, m.rows))
}

/// The ghost's list: the drop's cells, cut to the map — the very
/// intersection the drop applies, lifted out of the draw so the test
/// can stand in for the eye.
pub(crate) fn ghost_cells(
    clip: &Clip,
    col: i32,
    row: i32,
    cols: usize,
    rows: usize,
) -> Vec<(i32, i32, u32, u8)> {
    clip_cells(clip, col, row)
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

pub(crate) fn paint_at(
    maps: &mut Vec<MapLayer>,
    tileset: &TilesetRef,
    cell: usize,
    tfm: u8,
    col: i32,
    row: i32,
) -> bool {
    let i = match maps.iter().position(|m| &m.tileset == tileset) {
        Some(i) => i,
        None => {
            maps.push(MapLayer::blank(tileset.clone(), MAP_COLS, MAP_ROWS));
            maps.len() - 1
        }
    };
    maps[i].set_cell(col, row, cell as u32, tfm)
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

/// The atlas cell `idx`'s texture rectangle in a `rows`-by-`cols` grid,
/// row-major, top-left origin like the PNG's own rows.
pub(crate) fn cell_uv(idx: usize, rows: usize, cols: usize) -> [f32; 4] {
    let (row, col) = ((idx / cols) as f32, (idx % cols) as f32);
    let (rows, cols) = (rows as f32, cols as f32);
    [
        col / cols,
        row / rows,
        (col + 1.0) / cols,
        (row + 1.0) / rows,
    ]
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
        for (i, c) in clip.cells.iter().enumerate() {
            if *c == EMPTY_CELL {
                continue;
            }
            let t = render_tile(atlas, ar, ac, *c, clip.tfms[i], tw, th);
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
            tileset: tref("wild"),
            cols: 3,
            rows: 2,
            cells: vec![0, 1, 2, 3, 1, EMPTY_CELL],
            tfms: vec![0, 1, 2, 4, 5, 7],
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
/// A layer fixture's tileset: the stub sprites' name, cut 2 x 2 — the
/// cut the `tileset` fixture declares.
#[cfg(test)]
pub(crate) fn tref(name: &str) -> TilesetRef {
    TilesetRef {
        name: name.to_string(),
        rows: 2,
        cols: 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One PNG, cut two ways, is two tilesets: the layers must find
    /// their own dresser even when the older identity — path alone —
    /// would have dressed both from whichever slot came first. The
    /// order below is that bug's trap, left in on purpose.
    #[test]
    fn one_file_cut_two_ways_stays_two_tilesets() {
        let square = tileset("same.png"); // the fixture's 2 x 2 cut
        let four = TilesetRef::of(&square);
        let mut wide = tileset("same.png");
        wide.atlas = Some((1, 4));
        let sprites = vec![wide, square];
        assert_eq!(
            four,
            TilesetRef {
                name: "same".into(),
                rows: 2,
                cols: 2
            }
        );
        assert_ne!(four, TilesetRef::of(&sprites[0]));
        assert_eq!(four.sprite(&sprites).and_then(|sp| sp.atlas), Some((2, 2)));
        let long = TilesetRef {
            name: "same".into(),
            rows: 1,
            cols: 4,
        };
        assert_eq!(long.sprite(&sprites).and_then(|sp| sp.atlas), Some((1, 4)));
        // And a cut nothing wears is nobody's sprite: it waits, as a
        // closed tileset's layer waits, for its source to open.
        assert!(
            TilesetRef {
                name: "same".into(),
                rows: 4,
                cols: 1
            }
            .sprite(&sprites)
            .is_none()
        );
    }

    #[test]
    fn the_transform_is_the_maps_memory() {
        // What a cell was painted with stays with the cell: the same
        // tile, unturned, is no change; the same tile turned is one
        // the road remembers; and the eraser takes the bits down
        // with the tile.
        let mut m = MapLayer::blank(tref("t"), 4, 2);
        // The cell at grid (1, 0) — and the storage slot its centered
        // coordinates land in.
        let slot = m.at(1, 0).expect("inside the map");
        assert!(m.set_cell(1, 0, 3, 1 | (1 << 2))); // flip x, one turn
        assert_eq!(m.tfms[slot], 1 | (1 << 2));
        assert!(!m.set_cell(1, 0, 3, 1 | (1 << 2)), "nothing moved");
        assert!(m.set_cell(1, 0, 3, 1 | (2 << 2)), "a turn is a change");
        assert_eq!(m.tfms[slot], 1 | (2 << 2));
        assert!(m.set_cell(1, 0, EMPTY_CELL, 0));
        assert_eq!(m.tfms[slot], 0, "erasing clears the bits");
        assert!(!m.set_cell(1, 0, EMPTY_CELL, 0), "twice empty is empty");
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
    fn a_layer_hands_the_renderer_its_cells_orientations() {
        // The store decodes onto the tiles: bits out, flags in — and
        // the size stays as authored, because the renderer, not the
        // store, is what turns the extents.
        let mut m = MapLayer::blank(tref("t"), 2, 2);
        m.set_cell(-1, -1, 0, 1 | (3 << 2)); // flip x, three quarters cw
        let sp = tileset("t.png");
        let shape = m.shape(Some(&sp)).expect("one painted cell draws");
        let frost::Shape::TileMap { tiles, .. } = shape else {
            panic!("a tile map");
        };
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].rot, 3);
        assert!(tiles[0].flip_x);
        assert!(!tiles[0].flip_y);
        assert_eq!(tiles[0].size, [2.0, 1.0]);
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
            tileset: tref("basic_tiles"),
            cols: 2,
            rows: 1,
            cells: vec![2, 3],
            tfms: vec![0, 0],
        };
        clip_turn(&mut clip, false);
        assert_eq!((clip.cols, clip.rows), (1, 2), "the pair stands up");
        assert_eq!(clip.cells, vec![3, 2], "cell 3 rides to the top");
        assert_eq!(clip.tfms, vec![12, 12], "every tile turns counterclockwise");
    }

    #[test]
    fn a_cell_uv_is_its_band_in_the_files_own_order() {
        // The band convention the GPU bridge measures, pinned to the
        // demo's own formula: a cell's uv counts from the file's top
        // row — index 4 of a rack 2 wide × 3 deep is row 2, col 0,
        // the bottom-left third of the atlas; index 1 is row 0,
        // col 1, the top-right third. Bands counted from the bottom
        // would render every multi-row tileset row-swapped, a bug
        // only a real pixel can catch; this says the demo hands the
        // bridge the formula the bridge proved.
        assert_eq!(cell_uv(4, 3, 2), [0.0, 2.0 / 3.0, 0.5, 1.0]);
        assert_eq!(cell_uv(1, 3, 2), [0.5, 0.0, 1.0, 1.0 / 3.0]);
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
                    tileset: tref("t"),
                    cols: 1,
                    rows: 1,
                    cells: vec![1],
                    tfms: vec![code],
                };
                let mut sel = None;
                dress_block(&mut clip, &mut sel, d);
                assert_eq!(clip.tfms, vec![brush], "code {code}, gesture {d:?}");
                assert_eq!(clip.cells, vec![1], "the lone cell stays put");
            }
        }
    }

    #[test]
    fn the_ghost_and_the_drop_are_one_list() {
        // Ghost and paste consume the same clip_cells: the promise
        // can never show a cell the drop will skip — least of all
        // where the block hangs past a map edge.
        let clip = Clip {
            tileset: tref("wild"),
            cols: 3,
            rows: 2,
            cells: vec![0, 1, 2, 3, 1, EMPTY_CELL],
            tfms: vec![0, 1, 2, 4, 5, 7],
        };
        let cells = clip_cells(&clip, 5, 7);
        assert_eq!(cells.len(), 5, "the gap does not travel");
        assert!(
            cells.iter().any(|(c, r, ..)| (*c, *r) == (5, 7)),
            "the anchor lands on the cursor"
        );
        // The rim: cursor at the map's right edge, cells hanging
        // past it. The drop — and with it the preview — keeps
        // exactly the part that fits, measured by the engine's own
        // bounds predicate: grid coordinates radiate from the map's
        // center (cell_in, not a hand-drawn range — the first
        // version of this test drew one, at 0..8, and failed honest:
        // the engine was right and the test was wrong).
        let rim = clip_cells(&clip, 3, 0);
        let fits = rim
            .iter()
            .filter(|(c, r, _, _)| cell_in(*c, *r, 8, 8))
            .count();
        assert!(
            fits > 0 && fits < rim.len(),
            "the rim really clips some cells"
        );
        let mut maps = vec![MapLayer::blank(tref("wild"), 8, 8)];
        let n = clip_paste(&mut maps, &clip, 3, 0) as usize;
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
            ghost_cells(&clip, 3, 0, 8, 8),
            clip_cells(&clip, 3, 0)
                .into_iter()
                .filter(|(c, r, _, _)| cell_in(*c, *r, 8, 8))
                .collect::<Vec<_>>(),
            "the promise IS the cut-down drop"
        );
    }

    #[test]
    fn the_selection_erase_sweeps_every_layer() {
        let mut maps = vec![
            MapLayer::blank(tref("a"), 4, 4),
            MapLayer::blank(tref("b"), 4, 4),
        ];
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
}
