//! A little sprite utility: pick PNGs with native file dialogs (rfd),
//! keep up to seven of them side by side, and edit them in a work area
//! above a strip of sprite slots.
//!
//! The window splits into two regions: the **work area** on top, where
//! the active sprite is displayed over a transparency checkerboard with a
//! bounding box around it, and a **slot strip** at the bottom — seven
//! 100x100 px slots, each holding a minimized copy of one sprite as it
//! was loaded. The leftmost slot takes the first sprite. Click a slot to
//! make its sprite the active one (the work area and every operation
//! follow it), and drag a slot onto another to swap the two sprites'
//! positions.
//!
//! Every dialog is a child of the window, so it opens on top of it (a
//! parentless dialog lands at the screen's default spot, far from the
//! window). Dialogs appear only when asked for — **Open**, **Ctrl-O**,
//! **Save as** — never on their own: a launch with no argument simply
//! opens an empty workbench, and canceling a dialog returns to it.
//!
//! The controls are draggable [`frost::Ui`] panels (drag one by its
//! title bar, click the bar to fold it away) — and the view's own state
//! lives in the menu bar: the View menu carries the zoom level and the
//! last click as readouts, and the checker's two grey levels as live
//! [`frost::MenuItem::slider`] lines whose tracks you can grab without
//! leaving the list. Sliders in a menu is an unusual uniform; the menu
//! widget sewed it, and it fits.
//!
//! The File and Operations menus edit the active sprite: **Open**
//! (**Ctrl-O**) adds another sprite to the next free slot — while the
//! slots fill up, the newest sprite becomes the active one; **Crop** cuts the texture to
//! the selection rectangle if one is dragged (a left-drag over the work
//! area draws it; a click that barely moves instead logs the pixel and
//! its slot) — and to the opaque content when there is no selection,
//! trimming the fully transparent borders. The sidecar's positions ride
//! the cut: each shifts by the crop's origin, clamped to the new bounds,
//! so it stays put relative to the pixels that survive. Every crop is
//! remembered on a history that spans the whole bench: **Ctrl-Z** steps
//! back over any command — crop, mark, frame edit, slot move — and
//! **Ctrl-Shift-Z** steps forward again. **Save** writes the texture over the file it came
//! from, and **Save As** asks a native dialog for a new name, defaulted
//! to the sprite's current file name; both ask for confirmation before
//! overwriting an existing file — and, when the sprite has a sidecar
//! `.ron`, writes the tree back out beside the PNG. Each file is written
//! only when its content actually differs from it: a sprite whose pixels
//! and tree match the disk has nothing to save, and **Save** shows that
//! instead of a button (the name's `*` marks the difference, and **Save
//! As** stays a write — a new file always one). A sprite without a
//! sidecar gains one the first time it names a grid: the save that sets
//! the atlas also creates the file it persists in. **Close** takes the
//! active sprite out of its slot, and every later slot shifts left to
//! fill the gap.
//!
//! The "Animation" panel turns the loaded sprites into an animation: a
//! frame is a **set of layers** — snapshots of the active slot's shape,
//! stacked in the work area — and each frame holds for its own time
//! before the next one comes on. Playback either wraps at the ends
//! (**loop**) or walks back through the frames (**ping-pong**), and the
//! play button or space starts and stops the clock. The work area shows
//! the animation's current frame while any frame exists — which is also
//! the frame being edited, so the panel previews as it builds; the solo
//! active sprite returns when the last frame is deleted.
//!
//! The wheel zooms about the cursor (the point under it stays put),
//! **Ctrl-+** and **Ctrl--** step the zoom about the work area's centre
//! (the same characters and keys as the overlay's Alt pair, so a `+`
//! means plus on every layout), a middle-drag — the wheel button held —
//! grabs the work area and moves the sprite with the cursor,
//! and [`frost::Ui::hovering`] is what tells a click on the UI from one
//! on the scene: a press the UI claims never draws a selection, touches
//! a slot or logs a pixel.
//!
//! The checkerboard is the sprite's on-screen bounding box, clipped to
//! the work area, filled with light/dark grey cells that never scale with
//! the sprite, so it reveals the sprite's transparency at any zoom or
//! pan. The fill is a tiny sprite — one texture pixel per cell, nearest-
//! neighbor sampled through [`frost::Shape::sprite_bytes_nearest`] — so
//! the cell edges stay hard at any window size, and the texture is
//! rebuilt only when the visible cell count or a grey level changes. The
//! bounding box itself is four [`frost::Canvas::line`] strokes around the
//! sprite's full on-screen rectangle; the parts outside are clipped.
//!
//! Run with:
//!
//! ```text
//! cargo run --example sprite_util
//! ```
//!
//! (Set `RUST_LOG=info` to see the click lines.)
//!
//! The first sprite's source can be given on the command line with
//! `-i`/`--input`: a PNG file is opened immediately, without any file
//! dialog, and a folder is where the first *Open* dialog will open — a
//! relative path is expanded against the working directory and resolved
//! to an absolute one, because the native dialog (notably on Windows)
//! ignores anything else and opens in its default spot. Without the
//! argument the tool starts empty and dialogs open in the folder it was
//! launched from:
//!
//! ```text
//! cargo run --example sprite_util -- -i assets/sprites/bird1.png
//! cargo run --example sprite_util -- -i assets/sprites
//! ```

//! A sprite may bring a sidecar: when `<name>.png` has a `<name>.ron`
//! beside it, the file — the RON subset `ron_view` speaks, parsed by the
//! very same `tree.rs`, shared by path-include — opens as a foldable
//! tree — one `Ui` panel per open file, each titled by its file name,
//! cascading from the work area's lower right, dragging and folding like
//! every other panel and scrolling by its own pair of handles (the
//! wheel too, sideways with Shift; rows scroll by the character, so the
//! text never escapes the panel). A corner grip resizes each view —
//! narrower views is exactly what the horizontal handle is for — and
//! every rectangle the tree reads is the plate the UI actually painted,
//! so text and background never disagree. The boxed × in a view's bar
//! closes the file: sprite and sidecar together. Every position the file names — a
//! two-number tuple — is marked on the sprite in its own colour, the
//! very colour its tree rows carry: click a row to pick that position
//! (its marker swells, the row bands, its file becomes the active one),
//! click the sprite to move the position to the clicked pixel, and a
//! second click or Escape lets go. Rows without a position fold on
//! click. No sidecar or an unparseable one: the sprite simply loads.
//!
//! Lists are editable from the tree itself: an open sequence's head row
//! carries a green **+** (append — a copy of the last entry, and of a
//! position list the fresh entry lands picked, waiting for the sprite
//! click that places it), every entry row a red **×**, and pressing an
//! entry row and dragging up or down slides it through the list, other
//! entries making way. On the sprite, an entry of a list wears its
//! seat number beside the marker, so a reorder is legible at a glance.
//! Saving rewrites the sidecar with its comments intact — leads above
//! their value, trails beside it, the header and trailer bookending the
//! tree — only the line layout is the writer's own.
//!
//! The View menu switches the workbench between two views — **Markers**
//! (the full editing desk, the panels below) and **Atlas** (only the
//! Atlas panel, the grid's rows and columns) — and the view's own name
//! is noted at the menu bar's right end. The tile strip at the bottom
//! belongs to both: a sprite is picked and worked on identically either
//! way. A launch with no argument starts empty, in Markers.
//!
//! A classic menu bar runs across the window's top edge: **File** unfolds
//! **Open / Close / Save / Save as / Quit** and **View** unfolds
//! **Markers / Atlas / Tile Map**; while a list is open the pointer
//! tracks along the bar — sliding onto another title switches the list
//! without another click — hovered lines highlight, and a click anywhere
//! else dismisses. The bar's right end carries the last command and, at
//! the very end, the view's own name. The bar itself is fully part of the app —
//! drawn over every panel, owning every click that lands on it — but its
//! items are deliberately inert: choosing one only logs the choice, it
//! runs no file operation yet.

use clap::Parser;
use image::ImageEncoder;
use std::sync::Arc;

/// The RON parser and tree model, shared with `ron_view`: both examples
/// compile the same `ron_view/tree.rs`, this one through a path include.
/// `ron_view` uses the whole module; the sidecar panel leaves its
/// viewer-only helpers unused, hence the blanket allow.
#[allow(dead_code)]
#[path = "ron_view/tree.rs"]
mod ron_tree;

/// The File menu: the file verbs, lined up where every desktop app puts
/// them, with the two history steps among them. Open, Close, Save, Save
/// as, Undo and Redo do their named work — the pair walk the whole
/// bench's history, every crop, mark, frame and slot move; Quit raises
/// the same exit question Escape asks at the keys.
/// An item with an empty label draws the rule across the list: a
/// separator, not a command.
const FILE_MENU: frost::Menu<'static> = frost::Menu {
    title: "File",
    items: &[
        frost::MenuItem::new("Open", "Ctrl-O"),
        frost::MenuItem::new("Close", ""),
        frost::MenuItem::new("Save", ""),
        frost::MenuItem::new("Save as", ""),
        frost::MenuItem::new("Undo", "Ctrl-Z"),
        frost::MenuItem::new("Redo", "Ctrl-Shift-Z"),
        frost::MenuItem::SEPARATOR,
        frost::MenuItem::new("Quit", ""),
    ],
};

/// The workbench's view: which set of panels answers to the View menu.
/// `Markers` is the full editing desk — every panel; `Atlas` keeps only
/// the Atlas panel, the grid's rows and columns alone on the screen.
/// (Tile Map will complete the trio when it exists; its menu line is
/// already there, inert.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Markers,
    Atlas,
    /// The tile-map desk: the work area becomes a map canvas — the
    /// tileset's cell pitch extended as an infinite grid over the
    /// checker, with the map's axes marked — and the sprite steps off
    /// it: the tileset is the source, the map (once it can hold tiles)
    /// is what the canvas shows. The grid is the whole view today;
    /// painting tiles into maps arrives with its own steps.
    TileMap,
}

impl View {
    /// The bar's right-hand note: the view's own name.
    fn name(self) -> &'static str {
        match self {
            View::Markers => "Markers",
            View::Atlas => "Atlas",
            View::TileMap => "Tile Map",
        }
    }
}

/// One sprite as pure data: everything its GPU shapes are derived from.
/// The shapes themselves never enter the history — a restored world
/// rebuilds them from these pixels.
#[derive(Clone)]
struct WorldSprite {
    path: std::path::PathBuf,
    name: String,
    current: image::RgbaImage,
    saved_img: image::RgbaImage,
    ron: Option<RonDoc>,
    saved_ron: Option<String>,
    atlas: Option<(usize, usize)>,
    thumb_img: image::RgbaImage,
}

/// One frame as data: the layers' PNG bytes (a layer's shape IS its
/// snapshot) and the time the frame holds.
#[derive(Clone)]
struct WorldFrame {
    layers: Vec<Vec<u8>>,
    next_time: f32,
}

/// The animation as data: the frames, the one the clock stands on, and
/// whether playback wraps. The running clock itself — playing, dir,
/// elapsed — stands outside history: undo does not pause a tune.
#[derive(Clone)]
struct WorldAnim {
    frames: Vec<WorldFrame>,
    frame: usize,
    looping: bool,
}

/// Everything the commands touch, captured whole: the unit of undo.
/// View settings (zoom, the greys), the dialogs' remembered folder and
/// the status line stand outside — history is about the bench's
/// content, not the way it is held.
#[derive(Clone)]
struct World {
    sprites: Vec<WorldSprite>,
    active: usize,
    anim: WorldAnim,
    view: View,
    selection: Option<[f32; 4]>,
    maps: Vec<MapLayer>,
}

/// One tileset's worth of painted cells, anchored on the map's origin:
/// a grid of atlas-cell indices, `EMPTY_CELL` where nothing stands.
/// A multi-tileset picture is several of these layers, each naming its
/// tileset by path — the desk paints one, the canvas shows them all.
#[derive(Clone, PartialEq)]
struct MapLayer {
    /// The PNG the cells cut their pictures from. By path, because
    /// slots shift when sprites close and a map outlives that shuffle.
    tileset: std::path::PathBuf,
    cols: usize,
    rows: usize,
    cells: Vec<u32>,
    /// Each cell's orientation, painted with the cell and erased with
    /// it: bit 0 flip-x, bit 1 flip-y, bits 2..3 clockwise quarter-
    /// turns. The transform is the map's memory, not the brush's mood
    /// — which is why it lives here, inside the layer, and rides the
    /// undo road in every snapshot of the world.
    tfms: Vec<u8>,
}

/// One mouse-held paint stroke. `touched` gates the undo step: a
/// stroke that changes nothing (a click outside the map) must not
/// claim a place on the road.
#[derive(Clone, Copy)]
struct Paint {
    erase: bool,
    touched: bool,
    n: u32,
    /// Opened by the Delete key rather than a button; such a stroke
    /// ends when the key lets go, not on a mouse release.
    key_opened: bool,
}

/// The most worlds either road keeps; older steps age off the back.
const HISTORY_MAX: usize = 64;

/// One command's worth of memory: the world as it stood goes on the
/// undo road, and the redo road — the future this step chose over —
/// dissolves.
fn push_step(undo: &mut Vec<World>, redo: &mut Vec<World>, now: World) {
    undo.push(now);
    if undo.len() > HISTORY_MAX {
        undo.remove(0);
    }
    redo.clear();
}

/// One step back: the newest past becomes the present, and the present
/// goes onto the road forward. `None` when there is no past.
fn step_back(undo: &mut Vec<World>, redo: &mut Vec<World>, now: World) -> Option<World> {
    let past = undo.pop()?;
    redo.push(now);
    Some(past)
}

/// One step forward: the mirror image.
fn step_forward(undo: &mut Vec<World>, redo: &mut Vec<World>, now: World) -> Option<World> {
    let future = redo.pop()?;
    undo.push(now);
    Some(future)
}

/// The Operations menu: the verbs that edit the active sprite's pixels.
/// The panel that once carried them is gone — the menu is their home.
const OPERATIONS_MENU: frost::Menu<'static> = frost::Menu {
    title: "Operations",
    items: &[frost::MenuItem::new("Crop", "")],
};

/// The command line arguments.
#[derive(Parser, Debug)]
#[command(name = "sprite_util")]
struct Args {
    /// A PNG file to open immediately, skipping any file dialog — or a
    /// folder, in which case later Open dialogs open there (a relative
    /// path is expanded against the working directory). Without the
    /// argument the tool starts empty, and dialogs open in the working
    /// directory.
    #[arg(short = 'i', long = "input", value_name = "PATH")]
    input: Option<std::path::PathBuf>,
}

/// The lowest zoom: the sprite at a quarter of its texture size.
const ZOOM_MIN: f32 = 0.25;

/// The highest zoom: the sprite four times its texture size.
const ZOOM_MAX: f32 = 4.0;

/// The wheel's zoom factor per line of movement: the zoom is multiplied by
/// it, so scrolling up (positive delta) zooms in and scrolling down zooms
/// out.
const WHEEL_ZOOM: f32 = 1.15;

/// The controls panels' width, in pixels.
const PANEL_W: f32 = 300.0;

/// The checker cell's edge, in window pixels: the pattern never scales
/// with the sprite — one cell is always 16 pixels on screen.
const CHECK_CELL: f32 = 16.0;

/// The checker's default light grey level, 0.0-1.0. The defaults are
/// deliberately near-black: the pattern must read as a checkerboard
/// without becoming the brightest thing in a dark room — the greys
/// stay live sliders in the View menu for whoever wants them louder.
const GREY_LIGHT: f32 = 0.10;

/// The checker's default dark grey level, 0.0-1.0.
const GREY_DARK: f32 = 0.05;

/// How far the left button may travel between press and release and still
/// count as a click (select a slot, or log a pixel) rather than a drag
/// (a selection rectangle in the work area, or a slot swap).
const CLICK_TOL: f32 = 4.0;

/// The strip's thumbnail pool: the most slots visible at once. A
/// deeper rack rides behind the strip's scrollbar.
const SLOTS: usize = 7;

/// A slot's edge, in pixels.
const SLOT: f32 = 100.0;

/// The gap between neighboring slots.
const SLOT_GAP: f32 = 8.0;

/// The slots strip's height: a slot's edge plus the margin above and
/// below it.
const STRIP_H: f32 = 124.0;

/// The distance from the window's left edge to the first slot's left edge.
const SLOT_MARGIN: f32 = 14.0;

/// The longest edge of a slot thumbnail's minimized copy, in pixels —
/// a slot's edge minus its inner padding.
const THUMB_MAX: f32 = 84.0;

/// The work area's center height above the window's bottom-edge frame:
/// the work area spans from the strip's top edge to the window's top, so
/// its center sits half a strip above the bottom — whatever the height,
/// since the strip's height is fixed.
const WORK_Y: f32 = STRIP_H / 2.0;

/// The atlas' slider ceiling: the most rows or columns one grid may
/// have. 16 x 16 tiles any sensible sprite.
const ATLAS_MAX: f32 = 16.0;

const BG: frost::Color = frost::Color {
    r: 0.09,
    g: 0.09,
    b: 0.11,
    a: 1.0,
};
const MARKER: frost::Color = frost::Color {
    r: 1.0,
    g: 0.45,
    b: 0.4,
    a: 1.0,
};
/// The bounding box around the sprite.
const BBOX: frost::Color = frost::Color {
    r: 0.9,
    g: 0.9,
    b: 0.9,
    a: 1.0,
};
/// The crop selection's rectangle, and the active slot's frame.
const SELECT: frost::Color = frost::Color {
    r: 0.35,
    g: 0.72,
    b: 1.0,
    a: 1.0,
};
/// The tile-map desk's grid lines: one order above the checker and
/// below every shape, so painted tiles will sit on the grid, not in it.
const GRID_Z: f32 = -0.5;
/// The map's own axes: a touch louder than the grid, still beneath
/// everything paintable.
const AXIS: frost::Color = frost::Color {
    r: 0.62,
    g: 0.62,
    b: 0.68,
    a: 1.0,
};
const AXIS_Z: f32 = -0.4;
/// The grid's doubling ladder: while a cell's on-screen pitch drops
/// below this many window pixels, the drawn step doubles — so the grid
/// stays a few dozen lines at any zoom, always on whole cells.
const GRID_MIN_PX: f32 = 8.0;
/// The atlas' tile grid: the sprite split into its rows and columns.
const TILE: frost::Color = frost::Color {
    r: 0.55,
    g: 0.95,
    b: 0.65,
    a: 1.0,
};
/// The slots strip's floor, darker than the window.
const STRIP: frost::Color = frost::Color {
    r: 0.055,
    g: 0.055,
    b: 0.07,
    a: 1.0,
};
/// An occupied slot's plate.
const PLATE: frost::Color = frost::Color {
    r: 0.13,
    g: 0.13,
    b: 0.16,
    a: 1.0,
};
/// A free slot's plate.
const PLATE_EMPTY: frost::Color = frost::Color {
    r: 0.075,
    g: 0.075,
    b: 0.095,
    a: 1.0,
};

/// The bounding box's line width, in pixels.
const BBOX_WIDTH: f32 = 2.0;

/// The tile grid's line width, in pixels: thinner than the box, so the
/// grid reads as divisions, not frames.
const TILE_WIDTH: f32 = 1.0;

/// The bounding box's order: above the HUD, below the click marker.
const BBOX_Z: f32 = 1.5;

/// The tile grid's order: above the bounding box, below the crop
/// selection, so a selection still reads over the grid.
const TILE_Z: f32 = 1.6;

/// The selection's order: above the bounding box, below the click marker.
const SELECT_Z: f32 = 1.8;

/// The checker backdrop's order: behind the sprite and the HUD.
const CHECK_ORDER: f32 = -1.0;

/// The slots strip's fill order: above the checker, below the HUD.
const STRIP_Z: f32 = 0.2;

/// A slot plate's order: above its strip, below the HUD.
const PLATE_Z: f32 = 0.3;

/// The active slot's frame order: above the thumbnails.
const FRAME_Z: f32 = 0.9;

/// The slot thumbnails' order: above the strip and its plates.
const THUMB_ORDER: f32 = 0.6;

/// The order of a thumbnail being dragged between slots: above the HUD.
const DRAG_ORDER: f32 = 3.0;

/// The scene's layer nodes — and the cap on layers per animation frame.
/// The pool shows the animation's current frame, or the active sprite
/// alone when there are no frames.
/// The sidecar panel: text size and row pitch, panel width, body
/// viewport height, the viewport's top padding, the rows' text inset,
/// and the row pool's capacity. The panel itself is a `Ui` panel — its
/// title bar drags it around and folds it — and its body is one tall
/// [`frost::Ui::space`] viewport these constants lay the tree out in.
const RON_SIZE: f32 = 16.0;
/// The amount every panel text origin drops below the y it should sit
/// at, as a fraction of its size. This FiraCode reports degenerate
/// vertical metrics to the shaper — under a quarter pixel of ascent
/// plus descent at row size, exactly what `frost`'s diagnostics note
/// about it — so the renderer's text centring collapses onto the
/// baseline: text placed at a row's centre prints its ink a cap's
/// worth ABOVE that centre, visibly riding its highlight band. Cap
/// height is 1374 units per 2000 em; half of it centres the ink.
const RON_LIFT: f32 = 0.344;
const RON_LINE: f32 = 20.0;
const RON_W: f32 = 300.0;
const RON_VIEW_H: f32 = 240.0;
const RON_PAD: f32 = 6.0;
const RON_INSET: f32 = 12.0;
/// FiraCode's advance in pixels: the horizontal scroll slices rows by
/// characters, so a view's width in characters is its own arithmetic.
const RON_ADV: f32 = RON_SIZE * ADVANCE_EM;

/// FiraCode's advance: exactly 600/1000 em, identical at every weight —
/// so row widths are pure character arithmetic.
const ADVANCE_EM: f32 = 0.6;

const LAYER_NODES: usize = 8;

/// The time a new frame holds before the next one, in seconds; the
/// duration slider adjusts it per frame.
const TIME_DEFAULT: f32 = 0.2;

/// The frame duration slider's range, in seconds till the next frame.
const TIME_MIN: f32 = 0.02;
const TIME_MAX: f32 = 2.0;

/// The scene's fixed child indices: the layer pool starts at 0, then
/// the help line, the marker, the checker and the slot thumbnails.
const HELP: usize = LAYER_NODES;
const MARKER_NODE: usize = LAYER_NODES + 1;
const CHECKER: usize = LAYER_NODES + 2;
const THUMBS: usize = LAYER_NODES + 3;
/// The sidecar trees' order: they paint in the shared UI font, at 15 000
/// over the panel plates — the UI counts up from its 10 000 base, and
/// the trees must sit on top of their own plates.
const RON_ORDER: f32 = 15_000.0;

/// The position markers' pool: two circles per position — the colour
/// dot and its white centre — ordered above the sprite, its box and the
/// click marker, and under the UI's panel plate.
/// The band's cell nodes: after every node the sprite desks own, and
/// before the single ghost node that completes the pool.
const BAND0: usize = SPOTT + SPOTS_MAX;
const GHOST: usize = BAND0 + BAND_CELLS;
/// The plate's corner chip: the brush as it stands — the picked
/// cell's art wearing the brush's transform.
const HELD: usize = BAND0 + BAND_CELLS + 1;
const SPOTS: usize = THUMBS + SLOTS;
const SPOTT: usize = SPOTS + 2 * SPOTS_MAX;
const SPOTS_MAX: usize = 24;
const SPOT_ORDER: f32 = 2.9;
/// How many tileset cells the band shows at most; a bigger atlas waits
/// for the band's own scrolling (a later step).
/// A cell with nothing in it. Real atlas cells count from 0.
const EMPTY_CELL: u32 = u32::MAX;
/// The map a first click creates: 40 by 24 tiles at the origin — a
/// generous field one canvas wide at 1:1 zoom. A map-size control is
/// the ergonomics step's; the layer stores its own size, so the
/// default can change without disturbing any painted map.
const MAP_COLS: usize = 40;
const MAP_ROWS: usize = 24;
/// Maps draw above the desk's grid, below the band and the ghost.
const MAP_Z: f32 = 1.0;
/// The rack's node pool: a tileset grid up to 8 x 8 shows whole; a
/// deeper grid's tail stays out of the rack.
const BAND_CELLS: usize = 64;
/// The band's cell boxes on screen: a row of squares this wide, above
/// the slots strip, starting at the window's left edge.
const BAND_CELL: f32 = 56.0;
/// The gap between two band cells.
const BAND_GAP: f32 = 6.0;
/// The board's margin: the quiet border between the plate's edge and
/// its cells — also the panel's grab border.
const BAND_PAD: f32 = 6.0;
/// The panel's own zoom ladder: how far the wheel may shrink and
/// grow the rack's cells.
const BAND_ZOOM_MIN: f32 = 0.4;
const BAND_ZOOM_MAX: f32 = 3.0;
/// The band's cell shapes' order: above the map's future tiles, below
/// the HUD, so the source art is never hidden by its own picture.
const BAND_Z: f32 = 1.4;
/// The cursor ghost's order: above the cells, below dragged things.
const GHOST_Z: f32 = 2.6;
/// The cursor ghost's opacity: a promise, not a commitment.
const GHOST_A: f32 = 0.55;
/// The seat numbers beside list-entry markers: deliberately larger
/// than the row text — they're read at a glance over the art, where
/// the palette dot alone is not enough.
const SEAT_SIZE: f32 = 20.0;
/// How close a right-click must land to a marker (window pixels) for
/// it to count as a click ON the position: a touch larger than even
/// the swollen marker, so picking is forgiving.
const SPOT_HIT_R: f32 = 16.0;

/// The position palette: a marker and the tree rows describing it share
/// a colour, cycling through these hues.
const PALETTE: [(f32, f32, f32); 8] = [
    (0.95, 0.45, 0.42),
    (0.98, 0.72, 0.35),
    (0.93, 0.87, 0.45),
    (0.55, 0.85, 0.50),
    (0.45, 0.85, 0.82),
    (0.52, 0.66, 0.97),
    (0.90, 0.58, 0.92),
    (0.75, 0.90, 0.55),
];

/// One sprite: its files, its textures and its slot's look.
struct Sprite {
    /// The file the sprite was loaded from — Save writes back to it.
    path: std::path::PathBuf,
    /// The file's name, for the HUD, the dialogs and the logs.
    name: String,
    /// The working texture: crops replace it, saves write it.
    current: image::RgbaImage,
    /// The texture as it stands on the file: the loader's decode, then
    /// each save's bytes. The pixels compare against this to answer
    /// whether Save has anything to do — undoing back to the loaded
    /// picture is not a change, and an unchanged PNG is never re-encoded
    /// (its file keeps the container that produced it).
    saved_img: image::RgbaImage,
    /// The small picture a slot shows — kept as pixels, because when a
    /// sprite returns through undo its thumbnail's shape is rebuilt
    /// from these.
    thumb_img: image::RgbaImage,
    /// The working texture as a shape — the work area's sprite node.
    shape: frost::Shape,
    /// The original, minimized to a slot's width — the slot's picture.
    /// It never changes: a slot shows the sprite as it was loaded.
    thumb: frost::Shape,
    /// The parsed sidecar `<name>.ron`, `None` when the sprite has none
    /// or the file does not parse. It lives and dies with the sprite:
    /// closing the slot drops the panel with it.
    ron: Option<RonDoc>,
    /// The sidecar as it stands on the file, in the canonical text Save
    /// writes — `None` when the sprite loaded without one. The tree's
    /// counterpart of `saved_img`: same question, same answer.
    saved_ron: Option<String>,
    /// The tile grid the sprite splits into: the rows and columns the
    /// Atlas panel sets, read back from the sidecar's `atlas` field when
    /// the sprite loads. `None` is no grid. Saving a sprite that names a
    /// grid but has no sidecar yet creates one beside the PNG, so the
    /// grid has a file to live in.
    atlas: Option<(usize, usize)>,
}

impl Sprite {
    /// Whether the texture differs from the file's bytes: Save's question.
    fn png_changed(&self) -> bool {
        self.current != self.saved_img
    }

    /// The sidecar text a save would write: the parsed tree's rendering,
    /// or — for a sprite that names a grid but has no sidecar yet — the
    /// new file's text. `None` is "a save writes no sidecar".
    fn ron_text(&self) -> Option<String> {
        match &self.ron {
            Some(doc) => Some(ron_tree::to_text_doc(&doc.header, &doc.root, &doc.trailer)),
            None => self.atlas.map(|atlas| {
                let doc = new_sidecar(file_name_of(&self.path.with_extension("ron")), atlas);
                ron_tree::to_text_doc(&doc.header, &doc.root, &doc.trailer)
            }),
        }
    }

    /// Whether the sidecar — file or file-to-be — differs from the disk:
    /// the other question Save asks.
    fn ron_changed(&self) -> bool {
        self.ron_text()
            .is_some_and(|text| Some(&text) != self.saved_ron.as_ref())
    }

    /// Whether either save target differs from its file: the gate on the
    /// Save button, the `*` marker and the overwrite prompt.
    fn changed(&self) -> bool {
        self.png_changed() || self.ron_changed()
    }
}

/// A sprite's parsed sidecar: the tree with its fold flags, the
/// flattened visible rows, and the panel's scroll and fold state.
#[derive(Clone)]
struct RonDoc {
    /// The sidecar's file name, for the panel's title.
    name: String,
    /// The comments outside the root value, kept verbatim so saving
    /// rewrites the file with them: the header above the tree and the
    /// trailer after it.
    header: String,
    trailer: String,
    /// The parsed tree — the containers' fold flags live here.
    root: ron_tree::Val,
    /// The visible rows, rebuilt whenever a fold flips.
    rows: Vec<ron_tree::Row>,
    /// The scroll offsets in pixels: `scroll` down the rows, `sx`
    /// across them, 0 at the tree's top-left.
    scroll: f32,
    sx: f32,
    /// The position picked for editing, as an index into a fresh
    /// [`scan_spots`] of this tree: the next sprite click writes to it.
    edit: Option<usize>,
    /// The view's own size, as its resize handle left it: the panel's
    /// width and the tree viewport's height.
    vw: f32,
    vh: f32,
}

impl RonDoc {
    /// Re-pin both scroll offsets into THIS view's window, so folding
    /// a node or dragging a resize handle can never strand the tree
    /// past its rows or columns.
    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.clamp(
            0.0,
            (self.rows.len() as f32 * RON_LINE + RON_PAD - self.vh).max(0.0),
        );
        let wide = ron_width(self) as f32 - ron_chars(self.vw) as f32;
        self.sx = self.sx.clamp(0.0, (wide * RON_ADV).max(0.0));
    }
}

/// One picture within a frame: the active sprite's shape, snapshotted
/// when the layer was added — later crops and closed slots leave the
/// frame's layers untouched. The PNG the shape was built from rides
/// along: the history rebuilds a layer's shape from these bytes.
struct Layer {
    shape: frost::Shape,
    png: Vec<u8>,
}

/// One animation frame: a set of layers, stacked as added, and the time
/// it holds before the next frame.
#[derive(Default)]
struct Frame {
    layers: Vec<Layer>,
    next_time: f32,
}

/// The animation: an ordered set of frames kept by a clock. Looping
/// playback wraps at the ends; ping-pong walks back through the frames
/// instead, each end held once per sweep.
#[derive(Default)]
struct Anim {
    frames: Vec<Frame>,
    /// The frame the clock is in — also the frame the panel edits.
    frame: usize,
    /// Whether playback wraps at the ends (loop) or walks back
    /// (ping-pong).
    looping: bool,
    /// Whether the clock runs.
    playing: bool,
    /// The clock's direction: `1` forward, `-1` back (ping-pong only).
    dir: i32,
    /// Seconds spent in the current frame.
    elapsed: f32,
}

impl Anim {
    /// Jump to a frame and restart its hold: the panel's prev/next.
    fn set_frame(&mut self, frame: usize) {
        self.frame = frame;
        self.elapsed = 0.0;
    }

    /// Run the clock for `dt` seconds; whether the shown frame changed.
    fn tick(&mut self, dt: f32) -> bool {
        if !self.playing || self.frames.len() < 2 {
            return false;
        }
        self.elapsed += dt;
        let mut moved = false;
        // A slow frame can spend several holds at once: consume them one
        // by one, with a guard against a zero-length run of frames.
        let mut guard = 0;
        while self.elapsed >= self.frames[self.frame].next_time && guard < 512 {
            self.elapsed -= self.frames[self.frame].next_time.max(1e-4);
            self.step();
            moved = true;
            guard += 1;
        }
        moved
    }

    /// One step along the play direction: looping always wraps forward;
    /// ping-pong walks back and forth, each end held once per sweep.
    fn step(&mut self) {
        let n = self.frames.len();
        if self.looping {
            self.dir = 1;
            self.frame = (self.frame + 1) % n;
            return;
        }
        if self.dir > 0 {
            if self.frame + 1 < n {
                self.frame += 1;
            } else {
                self.dir = -1;
                self.frame = n - 2; // `n >= 2`: bounce off the last frame
            }
        } else if self.frame > 0 {
            self.frame -= 1;
        } else {
            self.dir = 1;
            self.frame = 1.min(n - 1); // bounce off the first frame
        }
    }
}

/// The demo's state.
struct Demo {
    /// The widget layer: the View and Operations panels.
    ui: frost::Ui,
    /// The tileset band's base shape: a `TileMap` shape sharing the
    /// active tileset's very pixel buffer (the same texture the sprite
    /// desk draws), kept so each cell is a one-tile clone of it.
    band_base: Option<frost::Shape>,
    /// What `band_base` was built from — slot, texture identity, grid —
    /// so the rebuild happens only when one of them moves.
    band_key: Option<(usize, usize, u64, usize, usize)>,
    /// The band's per-cell shapes: one tile each, sized to fit the
    /// cell box, rebuilt with the base.
    cell_shapes: Vec<frost::Shape>,
    /// The atlas cell the next click will paint: the brush. A tool
    /// mode like the active slot, outside the undo road.
    picked_cell: Option<usize>,
    /// The tile desk's painted layers — the picture, and the undo
    /// road's property (captured into every `World`).
    maps: Vec<MapLayer>,
    /// A paint or erase stroke in flight: opened by a left-press (or
    /// by Delete over the desk) and lived out while its trigger holds.
    paint: Option<Paint>,
    /// The very first frame re-syncs the work pool after the world
    /// exists: the boot construction baked the sprite into a node by
    /// hand, and everything the world can carry (maps included)
    /// reaches the canvas through `sync_work` from then on.
    first_frame: bool,
    /// The loaded sprites — a sprite's slot is its index.
    sprites: Vec<Sprite>,
    /// The slot whose sprite fills the work area and takes the
    /// operations. Meaningless while `sprites` is empty.
    active: usize,
    /// The animation built from the sprites: frames of layers, on a
    /// clock, shown in the work area while any frame exists.
    anim: Anim,
    /// The current zoom: the sprite's scale factor (1.0 is texture size).
    zoom: f32,
    /// The sprite's pan offset from the work area's center, in window
    /// pixels; set by middle-dragging and by the wheel's anchor.
    offset: [f32; 2],
    /// The crop selection in texture pixels, `[x0, y0, x1, y1]` (y down):
    /// dragged with the left button, consumed by Crop.
    selection: Option<[f32; 4]>,
    /// Where the current work-area press began, in window pixels — only
    /// set when the press was the work area's, not the UI's or a slot's.
    drag_from: Option<[f32; 2]>,
    /// The slot being dragged between slots, and where its press began.
    slot_drag: Option<(usize, [f32; 2])>,
    /// The slot strip's scroll, in window pixels — `f32::MAX` parks
    /// it at the far end, clamped back each frame.
    slot_scroll: f32,
    /// The scrollbar knob's grab offset while it is dragged.
    slot_scrub: Option<f32>,
    /// Where the tileset panel's plate stands — `None` keeps it at
    /// its corner above the strip's left end.
    band_origin: Option<[f32; 2]>,
    /// The panel's own zoom: the wheel over it scales the rack's
    /// cells, never the canvas.
    band_zoom: f32,
    /// A held panel drag: the grab offset in the plate, the press
    /// point, and the origin the drag started from.
    band_drag: Option<([f32; 2], [f32; 2], [f32; 2])>,
    /// Whether Ctrl-Z was held last frame: its rising edge is the undo.
    was_undo: bool,
    /// Whether Ctrl-O was held last frame: its rising edge is the open.
    was_open: bool,
    /// Whether space was held last frame: its rising edge toggles play.
    was_space: bool,
    /// Whether Escape was held last frame: its rising edge releases a
    /// picked position.
    was_esc: bool,
    /// The checker's light grey level, 0.0-1.0; the Light slider sets it.
    light: f32,
    /// The checker's dark grey level, 0.0-1.0; the Dark slider sets it.
    dark: f32,
    /// The checker texture's key: its cell count and the two grey levels,
    /// 0-255 quantized — the texture is rebuilt only when one of them
    /// changes.
    checker_key: (u32, u32, u8, u8),
    /// Whether the left mouse button was held on the previous frame: the
    /// edges of the two are the press and the release.
    was_down: bool,
    /// The cursor's position on the previous frame, for the middle-drag's
    /// frame-to-frame pan delta.
    last_mouse: Option<[f32; 2]>,
    /// The last reported click: its position in the active texture's
    /// pixel space, and whether the spot was inside the texture.
    last_click: Option<([f32; 2], bool)>,
    /// The seat numbers' font bytes: one Arc's worth of FiraCode, the
    /// numbers beside the markers cloning the pointer, never the file.
    ron_font: Arc<[u8]>,
    /// Where the right button went down, for its still-click test.
    r_press: Option<[f32; 2]>,
    /// The middle button's press point: a release without travel
    /// moves the held block; a press that travels is the pan.
    m_press: Option<[f32; 2]>,
    /// The middle button's level last poll — the press edge.
    was_mdown: bool,
    /// Whether the exit-confirmation box stands over the editor.
    quit_prompt: bool,
    /// Last frame's Escape, for the exit box's own press edge.
    was_esc_prompt: bool,
    /// Whether the right button was held on the previous frame.
    was_rdown: bool,
    /// Last frame's Delete/Backspace, for the eraser's press edge.
    was_del: bool,
    /// The copied rectangle held on the desk, dressed and pasted as
    /// one picture; `None` while the single-tile brush is held.
    clip: Option<Clip>,
    /// The selection's cell bounds, inclusive `[c0, r0, c1, r1]` —
    /// the frame around a held block, and the Delete key's target.
    sel: Option<[i32; 4]>,
    /// The cell where Shift + left went down, while the drag lives.
    sel_drag: Option<[i32; 2]>,
    /// Last frame's Escape, for the block-drop edge.
    was_block_esc: bool,
    /// An in-progress list reordering: which slot's sequence, at which
    /// path, and the entry's current seat.
    ron_drag: Option<(usize, Vec<u16>, usize)>,
    /// The shared interaction state of the sidecar trees: the widget's
    /// claim on the close box and the grip, the thumbs and the corner
    /// drags in flight. One set of trees per frame, so one state across
    /// them all.
    ron_state: frost::TreeState,
    /// Every open file's panel as the UI laid it out — the frame's own
    /// record of the views, rebuilt between the panel calls and the
    /// tree painting.
    ron_views: Vec<RonView>,
    /// The view the View menu last chose: decides which panels this
    /// frame declares, and names itself at the menu bar's right end.
    view: View,
    /// The road back: one whole-world snapshot per command, newest
    /// last.
    undo_stack: Vec<World>,
    /// The road forward again: the worlds undo stepped off, newest
    /// last; a fresh command empties it — the future a new past erases.
    redo_stack: Vec<World>,
    /// Whether Ctrl(-Shift)-Z rested down last frame: the keys speak on
    /// their rising edges only.
    was_redo: bool,
    /// The zoom pair's edges, same rising-edge rule.
    was_zoom_in: bool,
    was_zoom_out: bool,
    /// The paintbrush's standing orientation: bit 0 flip-x, bit 1
    /// flip-y, bits 2..3 clockwise quarter-turns. Tool state, like the
    /// picked cell — outside the undo road; it changes what the NEXT
    /// stroke paints, and every map keeps what it was painted with.
    brush: u8,
    /// The orientation keys' edges: X mirrors sideways, Y mirrors
    /// top-to-bottom, R turns a quarter clockwise, Shift-R back.
    was_flipx: bool,
    was_flipy: bool,
    was_turn: bool,
    /// The folder the next dialog opens in: where the last file was
    /// read from or written to, or — before any dialog has run — the
    /// folder the command line named, the working directory when no
    /// argument did.
    dir: Option<std::path::PathBuf>,
    /// The Operations panel's status line: the selection's size, or the
    /// outcome of the last operation.
    status: String,
}

impl Demo {
    /// The active sprite, or `None` while all slots are empty.
    fn active(&self) -> Option<&Sprite> {
        self.sprites.get(self.active)
    }

    /// The active sprite's sidecar panel state, if it has one.
    fn ron(&self) -> Option<&RonDoc> {
        self.active()?.ron.as_ref()
    }

    /// The same, writable: the wheel's scroll and the folds' toggles.
    fn ron_mut(&mut self) -> Option<&mut RonDoc> {
        self.sprites.get_mut(self.active)?.ron.as_mut()
    }

    /// One sidecar-panel text shape, on the shared font bytes.
    fn ron_text(&self, text: String, size: f32, weight: f32, color: frost::Color) -> frost::Shape {
        frost::Shape::Text {
            text,
            font: Arc::clone(&self.ron_font),
            size,
            weight,
            color,
            alpha: 1.0,
        }
    }

    /// The active sprite's texture size, or zero while there is none.
    fn work_size(&self) -> [f32; 2] {
        match self.active() {
            Some(sp) => [sp.current.width() as f32, sp.current.height() as f32],
            None => [0.0, 0.0],
        }
    }

    /// Rebuild the tileset band when the tileset, its texture or its
    /// grid moved. The base is the active sprite's own pixels wearing a
    /// `TileMap` face — one texture for the sprite and every cell of
    /// the band, keyed by the same buffer identity the engine's
    /// texture cache uses — and each cell is that base with a single
    /// tile: the cell's atlas rectangle, stretched over the cell box.
    fn sync_band(&mut self) {
        let Some((key, base, tiles)) = self.active().and_then(|sp| match &sp.shape {
            frost::Shape::Sprite {
                data,
                width,
                height,
                generation,
                ..
            } => {
                let (rows, cols) = sp.atlas.unwrap_or((1, 1));
                let key = (
                    self.active,
                    std::sync::Arc::as_ptr(data) as *const u8 as usize,
                    *generation,
                    rows,
                    cols,
                );
                let base = frost::Shape::TileMap {
                    data: data.clone(),
                    width: *width,
                    height: *height,
                    generation: *generation,
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
                Some((
                    key,
                    base,
                    [
                        *width as f32 / cols.max(1) as f32,
                        *height as f32 / rows.max(1) as f32,
                    ],
                ))
            }
            _ => None,
        }) else {
            self.band_base = None;
            self.band_key = None;
            self.cell_shapes.clear();
            return;
        };
        if self.band_key == Some(key) {
            return;
        }
        self.band_key = Some(key);
        let (rows, cols) = (key.3, key.4);
        // Fit the cell's texture aspect inside the band's square box.
        let k = (BAND_CELL / tiles[0]).min(BAND_CELL / tiles[1]);
        let (dw, dh) = (tiles[0] * k, tiles[1] * k);
        let cells = (rows * cols).min(BAND_CELLS);
        self.cell_shapes = (0..cells)
            .map(|i| {
                let mut shape = base.clone();
                shape.set_tiles(vec![frost::Tile::new(
                    [0.0, 0.0],
                    [dw, dh],
                    cell_uv(i, rows, cols),
                )]);
                shape
            })
            .collect();
        self.band_base = Some(base);
        // A grid that shrank under the brush drops the brush.
        if self.picked_cell.is_some_and(|c| c >= rows * cols) {
            self.picked_cell = None;
        }
    }

    /// Repaint the work area's layer pool: the animation's current frame
    /// when any frame exists (so the panel previews what it edits), else
    /// the active sprite alone in the first node. On the tile-map desk
    /// the same pool carries the maps — one painted layer per node.
    fn sync_work(&self, ctx: &mut frost::Context) {
        let frame = if self.anim.frames.is_empty() {
            None
        } else {
            Some(&self.anim.frames[self.anim.frame.min(self.anim.frames.len() - 1)])
        };
        // The tile-map desk shows maps, not the tileset: the sprite and
        // the animation's layers stay off the canvas (the tileset is the
        // paint source, not the picture).
        let tile_view = self.view == View::TileMap;
        for i in 0..LAYER_NODES {
            let node = &mut ctx.scene().root.children[i];
            node.order = if tile_view { MAP_Z } else { i as f32 * 0.01 };
            node.shape = match &frame {
                _ if tile_view => self
                    .maps
                    .get(i)
                    .and_then(|m| m.shape(self.sprites.iter().find(|sp| sp.path == m.tileset))),
                Some(f) => f.layers.get(i).map(|l| l.shape.clone()),
                None if i == 0 => self.active().map(|sp| sp.shape.clone()),
                None => None,
            };
        }
    }

    /// Re-point every slot node at its slot's thumbnail (or hide empty
    /// slots). Only needed when slots change: opens, closes, swaps.
    fn refresh_slots(&self, ctx: &mut frost::Context) {
        for i in 0..SLOTS {
            let node = &mut ctx.scene().root.children[THUMBS + i];
            node.shape = self.sprites.get(i).map(|sp| sp.thumb.clone());
        }
    }

    /// Replace the active sprite's texture: re-encode it to PNG bytes,
    /// rebuild its shape, and put it on screen — the selection and the
    /// marker referred to the old texture, so both clear.
    fn apply(
        &mut self,
        ctx: &mut frost::Context,
        img: image::RgbaImage,
        status: String,
    ) -> Option<()> {
        let png = png_bytes(&img).ok()?;
        let shape = frost::Shape::sprite_bytes_nearest(&png).ok()?;
        let sp = self.sprites.get_mut(self.active)?;
        sp.current = img;
        sp.shape = shape;
        self.selection = None;
        self.last_click = None;
        self.status = status;
        self.sync_work(ctx);
        Some(())
    }

    /// Open a sprite from `path`: load its texture, minimize the original
    /// to a slot thumbnail, and add it to the next free slot — which
    /// becomes the active one. A full house or an unloadable file is
    /// reported on the status line, not fatal.
    fn load_sprite(&mut self, ctx: &mut frost::Context, path: std::path::PathBuf) {
        match read_sprite(&path) {
            Err(err) => {
                log::warn!("open failed: {err}");
                self.status = format!("open: {err}");
            }
            Ok(sp) => {
                // Loading is a command too: undo can give the slot back.
                self.stamp();
                self.dir = path
                    .parent()
                    .filter(|d| !d.as_os_str().is_empty())
                    .map(std::path::PathBuf::from);
                self.status = format!("opened '{}' in slot {}", sp.name, self.sprites.len() + 1);
                self.sprites.push(sp);
                self.active = self.sprites.len() - 1;
                self.selection = None;
                self.last_click = None;
                self.sync_work(ctx);
                self.refresh_slots(ctx);
            }
        }
    }

    /// Open: a native dialog for another sprite, into the next free slot.
    fn open_dialog(&mut self, ctx: &mut frost::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("sprite_util: open a sprite")
            .add_filter("PNG images", &["png"]);
        if let Some(dir) = &self.dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(window) = ctx.window() {
            dialog = dialog.set_parent(window);
        }
        match dialog.pick_file() {
            Some(path) => self.load_sprite(ctx, path),
            None => self.status = String::from("open: canceled"),
        }
    }

    /// Crop: to the selection when one is dragged, else to the opaque
    /// content — trimming the fully transparent borders. The sidecar's
    /// positions shift by the cut's origin, clamped to the new bounds, so
    /// they stay put relative to the pixels that survive. The whole world
    /// before the cut goes onto the history, so undo brings back the
    /// pixels, the positions and everything else the stroke touched.
    fn crop(&mut self, ctx: &mut frost::Context) {
        let Some(sp) = self.active() else { return };
        if sp.current.width() == 0 {
            return;
        }
        let (w, h) = (sp.current.width(), sp.current.height());
        let sel = self.selection;
        let mut err = None;
        let rect = match sel {
            Some(sel) => sel_int_rect(sel, w, h),
            None => match alpha_bbox(&sp.current) {
                None => {
                    err = Some(String::from("crop: the sprite is fully transparent"));
                    None
                }
                Some(r) if r == (0, 0, w, h) => {
                    err = Some(String::from("crop: nothing to trim — drag to select"));
                    None
                }
                Some(r) => Some(r),
            },
        };
        let Some((x, y, cw, ch)) = rect else {
            if let Some(err) = err {
                self.status = err;
            }
            return;
        };
        let cut = image::imageops::crop_imm(&sp.current, x, y, cw, ch).to_image();
        let (nx, ny) = (cut.width(), cut.height());
        self.stamp();
        if self
            .apply(ctx, cut, format!("cropped to {nx} x {ny} px"))
            .is_none()
        {
            // Nothing was rebuilt: the step never happened.
            self.undo_stack.pop();
            self.status = String::from("crop: could not rebuild the sprite");
        } else if let Some(doc) = self.sprites[self.active].ron.as_mut() {
            let n = shift_spots(&mut doc.root, x as f32, y as f32, (nx, ny));
            doc.rows = ron_tree::layout(&doc.root);
            if n > 0 {
                self.status = format!("cropped to {nx} x {ny} px — {n} positions shifted");
            }
        }
    }

    /// The bench as pure data: every sprite's pixels, sidecar and grid,
    /// the animation's frames, the active slot, the view, the selection.
    fn capture(&self) -> World {
        World {
            sprites: self
                .sprites
                .iter()
                .map(|sp| WorldSprite {
                    path: sp.path.clone(),
                    name: sp.name.clone(),
                    current: sp.current.clone(),
                    saved_img: sp.saved_img.clone(),
                    ron: sp.ron.clone(),
                    saved_ron: sp.saved_ron.clone(),
                    atlas: sp.atlas,
                    thumb_img: sp.thumb_img.clone(),
                })
                .collect(),
            active: self.active,
            anim: WorldAnim {
                frames: self
                    .anim
                    .frames
                    .iter()
                    .map(|f| WorldFrame {
                        layers: f.layers.iter().map(|l| l.png.clone()).collect(),
                        next_time: f.next_time,
                    })
                    .collect(),
                frame: self.anim.frame,
                looping: self.anim.looping,
            },
            view: self.view,
            selection: self.selection,
            maps: self.maps.clone(),
        }
    }

    /// One command's before-picture, called right before the state
    /// changes. The redo road dissolves with it: a new past erases the
    /// future it did not take.
    fn stamp(&mut self) {
        let now = self.capture();
        push_step(&mut self.undo_stack, &mut self.redo_stack, now);
    }

    /// Undo: one step back over every kind of command — crops, marks,
    /// frame edits, slot moves — restoring the whole bench, textures
    /// rebuilt from the snapshot's own pixels.
    fn undo(&mut self, ctx: &mut frost::Context) {
        let now = self.capture();
        let Some(past) = step_back(&mut self.undo_stack, &mut self.redo_stack, now) else {
            self.status = String::from("undo: nothing to undo");
            return;
        };
        self.restore(ctx, past, "undo");
    }

    /// Redo: one step forward again, the same worlds walked backwards
    /// down the road undo left behind.
    fn redo(&mut self, ctx: &mut frost::Context) {
        let now = self.capture();
        let Some(future) = step_forward(&mut self.undo_stack, &mut self.redo_stack, now) else {
            self.status = String::from("redo: nothing to redo");
            return;
        };
        self.restore(ctx, future, "redo");
    }

    /// Write a captured world back: pixels, sidecars, grids, frames, the
    /// view and the active slot, every GPU shape rebuilt from stored
    /// bytes. PNG round-trips are lossless, so a rebuilt texture is the
    /// old one to the bit.
    fn restore(&mut self, ctx: &mut frost::Context, world: World, word: &str) {
        let mut sprites = Vec::with_capacity(world.sprites.len());
        for ws in world.sprites {
            let shape = png_bytes(&ws.current)
                .ok()
                .and_then(|png| frost::Shape::sprite_bytes_nearest(&png).ok());
            let thumb_img = ws.thumb_img;
            let thumb = png_bytes(&thumb_img)
                .ok()
                .and_then(|png| frost::Shape::sprite_bytes_nearest(&png).ok());
            let (Some(shape), Some(thumb)) = (shape, thumb) else {
                log::warn!("{}: could not rebuild '{}', left out", word, ws.name);
                continue;
            };
            sprites.push(Sprite {
                path: ws.path,
                name: ws.name,
                current: ws.current,
                saved_img: ws.saved_img,
                shape,
                thumb,
                thumb_img,
                ron: ws.ron,
                saved_ron: ws.saved_ron,
                atlas: ws.atlas,
            });
        }
        self.sprites = sprites;
        self.active = world.active.min(self.sprites.len().saturating_sub(1));
        let mut anim = Anim::default();
        anim.looping = world.anim.looping;
        for wf in world.anim.frames {
            let mut layers = Vec::with_capacity(wf.layers.len());
            for png in wf.layers {
                if let Ok(shape) = frost::Shape::sprite_bytes_nearest(&png) {
                    layers.push(Layer { shape, png });
                }
            }
            anim.frames.push(Frame {
                layers,
                next_time: wf.next_time,
            });
        }
        anim.frame = world.anim.frame.min(anim.frames.len().saturating_sub(1));
        self.anim = anim;
        self.view = world.view;
        self.selection = world.selection;
        self.maps = world.maps;
        self.paint = None;
        self.last_click = None;
        let (w, h) = self
            .active()
            .map_or((0, 0), |sp| (sp.current.width(), sp.current.height()));
        self.status = if w == 0 {
            format!("{word}: back to an empty bench")
        } else {
            format!("{word}: back to {w} x {h} px")
        };
        self.sync_work(ctx);
        self.refresh_slots(ctx);
    }

    /// Close: take the active sprite out of its slot; every later sprite
    /// shifts one slot left, and the slot the active one leaves (or the
    /// new last one, if it was last) becomes active.
    fn close_active(&mut self, ctx: &mut frost::Context) {
        if self.sprites.is_empty() {
            return;
        }
        let i = self.active;
        self.close_slot(ctx, i);
    }

    /// Close one slot — sprite, sidecar and panel all at once, whoever
    /// asks; the × in a view's title bar speaks the file's name. The
    /// panel's state stays behind under the title (its place is kept
    /// for the file's return) but must not sit folded.
    fn close_slot(&mut self, ctx: &mut frost::Context, i: usize) {
        let Some(sp) = self.sprites.get(i) else {
            return;
        };
        if let Some(doc) = &sp.ron {
            let title = doc.name.clone();
            self.ui.set_folded(&title, false);
        }
        self.stamp();
        let name = self.sprites.remove(i).name;
        // The sidecar views were laid out this frame from the sprite
        // list as it stood before the close, and the rest of the frame
        // still draws from them — keep them honest for it. A press, a
        // grab or a reorder riding a view has lost the file it was on:
        // let it go.
        close_view_slot(&mut self.ron_views, i);
        self.ron_state = frost::TreeState::default();
        self.ron_drag = None;
        self.active = self.active.min(self.sprites.len().saturating_sub(1));
        self.selection = None;
        self.last_click = None;
        self.status = format!("closed '{name}'");
        self.sync_work(ctx);
        self.refresh_slots(ctx);
    }

    /// Save: write the active texture over the file it was loaded from,
    /// asking first — that file exists, by definition of overwriting it.
    /// Nothing changed since the last save means nothing to write, and
    /// nothing to ask about: the files keep their bytes untouched.
    fn save(&mut self, ctx: &mut frost::Context) {
        let Some(sp) = self.active() else { return };
        if sp.current.width() == 0 {
            return;
        }
        if sp.path.as_os_str().is_empty() {
            self.status = String::from("save: no file to overwrite — use save as");
            return;
        }
        if !sp.changed() {
            self.status = format!("'{}': nothing to save", sp.name);
            return;
        }
        let (path, name) = (sp.path.clone(), file_name_of(&sp.path));
        if path.exists() && !confirm_overwrite(ctx, &name) {
            self.status = String::from("save: canceled");
            return;
        }
        self.write_png(&path, name);
    }

    /// Save As: a native save dialog defaulted to the active sprite's
    /// current file name; an existing target asks the same overwrite
    /// confirmation.
    fn save_as(&mut self, ctx: &mut frost::Context) {
        let Some(sp) = self.active() else { return };
        if sp.current.width() == 0 {
            return;
        }
        let mut dialog = rfd::FileDialog::new()
            .set_title("sprite_util: save as")
            .set_file_name(&sp.name)
            .add_filter("PNG images", &["png"]);
        let dir = sp
            .path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .or(self.dir.as_deref())
            .map(std::path::Path::to_path_buf);
        if let Some(dir) = &dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(window) = ctx.window() {
            dialog = dialog.set_parent(window);
        }
        let Some(mut target) = dialog.save_file() else {
            self.status = String::from("save as: canceled");
            return;
        };
        match target.extension() {
            None => {
                target.set_extension("png");
            }
            Some(ext) if ext.eq_ignore_ascii_case("png") => {}
            Some(_) => {
                self.status = String::from("save as: only PNG files");
                return;
            }
        }
        let name = file_name_of(&target);
        if target.exists() && !confirm_overwrite(ctx, &name) {
            self.status = String::from("save as: canceled");
            return;
        }
        self.write_png(&target, name);
    }

    /// Write the active texture to `path` as PNG and make it the sprite's
    /// file — the name and the Save target both follow. Each save target
    /// is compared with the bytes on disk and left alone when equal: a
    /// save that only brings a sidecar into the world no longer re-encodes
    /// pixels it never touched, and an untouched PNG keeps the container
    /// that made it. Save As to a new name is always a change — the new
    /// file knows nothing yet. The slot keeps showing the original: a slot
    /// is a sprite's identity, not its edit.
    fn write_png(&mut self, path: &std::path::Path, name: String) {
        let mut status = String::new();
        let mut wrote = false;
        if let Some(sp) = self.sprites.get_mut(self.active) {
            // A different file is a change whether or not the pixels
            // moved: the target has nothing to compare against.
            let here = path == sp.path;
            let mut saved_files: Vec<String> = Vec::new();
            if !here || sp.png_changed() {
                let done = png_bytes(&sp.current)
                    .and_then(|png| std::fs::write(path, png).map_err(|e| e.to_string()));
                match done {
                    Ok(()) => {
                        log::info!("saved '{name}'");
                        sp.saved_img = sp.current.clone();
                        saved_files.push(name.clone());
                    }
                    Err(err) => status = format!("save failed: {err}"),
                }
            }
            // The sidecar travels with its sprite: the tree is written
            // back out — canonically re-laid, edits and folds faithful —
            // beside the saved PNG, but only when it differs from its
            // file. A sprite without one gains a file the first time it
            // names a grid: the save that sets the atlas also creates the
            // sidecar the grid lives in.
            if status.is_empty()
                && (!here || sp.ron_changed())
                && let Some(text) = sp.ron_text()
            {
                let side = path.with_extension("ron");
                let side_name = file_name_of(&side);
                match std::fs::write(&side, &text) {
                    Ok(()) => {
                        log::info!("saved '{side_name}'");
                        sp.saved_ron = Some(text);
                        saved_files.push(side_name.clone());
                        // A grid without a document adopts the tree
                        // its file now holds: the panel opens on the
                        // next frame, and later edits ride the same
                        // file.
                        if sp.ron.is_none()
                            && let Some(atlas) = sp.atlas
                        {
                            sp.ron = Some(new_sidecar(side_name, atlas));
                        }
                    }
                    Err(err) => log::warn!("failed to save '{side_name}': {err}"),
                }
            }
            if !saved_files.is_empty() {
                sp.path = path.to_path_buf();
                sp.name = name.clone();
                status = format!("saved '{}'", saved_files.join("' and '"));
            } else if status.is_empty() {
                // Nothing differed from its file. Save's own gate
                // refuses this, and Save As cannot reach it — belt:
                // say so plainly rather than claim a write.
                status = format!("'{name}': nothing to save");
            }
            wrote = !saved_files.is_empty();
        }
        if wrote {
            self.dir = path
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .map(std::path::PathBuf::from);
        }
        self.status = status;
    }

    /// A still click on a sidecar view's row. A row that describes a
    /// position picks it for editing — its file becomes the active one
    /// first, since the sprite click that moves the position lands on
    /// the work area; a second click lets go. Any other foldable row
    /// folds its node, and the view re-flattens. Folding a panel whole
    /// is the UI's own title-bar click, away from the rows.
    fn row_click(&mut self, ctx: &mut frost::Context, slot: usize, row: usize) {
        let Some(doc) = self.sprites.get(slot).and_then(|sp| sp.ron.as_ref()) else {
            return;
        };
        let Some(row) = doc.rows.get(row) else {
            return;
        };
        // The row's facts out of the tree first — the path it names,
        // the fold flag it carries, the position it describes and the
        // work size the glide will use — so the borrows below can
        // flip to mutable.
        let path = row.path.clone();
        let foldable = row.fold.is_some();
        let mut spots = Vec::new();
        scan_spots(&doc.root, &mut Vec::new(), "", &mut spots);
        let owner = spot_of_row(&spots, &path)
            .map(|pi| (pi, spots[pi].label.clone(), spots[pi].x, spots[pi].y));
        let wsz = [
            self.sprites[slot].current.width() as f32,
            self.sprites[slot].current.height() as f32,
        ];
        if owner.is_some() && self.active != slot {
            self.active = slot;
            self.selection = None;
            self.last_click = None;
            self.sync_work(ctx);
        }
        let mut say = None;
        if let Some(doc) = self.sprites[slot].ron.as_mut() {
            if let Some((pi, label, sx, sy)) = owner {
                doc.edit = if doc.edit == Some(pi) { None } else { Some(pi) };
                if doc.edit.is_some() {
                    // The sprite glides the picked position to its
                    // centre: the point being edited sits under the eye.
                    self.offset = center_offset((sx, sy), wsz, self.zoom);
                }
                say = Some(if doc.edit.is_some() {
                    format!("editing '{label}' — click the sprite to move it")
                } else {
                    String::from("edit: released")
                });
            } else if foldable {
                if let Some(v) = ron_tree::walk(&mut doc.root, &path) {
                    v.toggle();
                }
                doc.rows = ron_tree::layout(&doc.root);
                doc.clamp_scroll();
            }
        }
        if let Some(t) = say {
            self.status = t;
        }
    }

    /// A sidecar view's [+]: append an entry to the sequence the row
    /// opens — a copy of its last entry, or a fresh `0` in an empty
    /// one. A fresh copy of a position is picked up right away, ready
    /// for the sprite click that places it.
    fn row_add(&mut self, slot: usize, row: usize) {
        let Some(doc) = self.sprites.get(slot).and_then(|sp| sp.ron.as_ref()) else {
            return;
        };
        let Some(row) = doc.rows.get(row) else {
            return;
        };
        let cpath = row.path.clone();
        // The [+] lives only on sequence rows, so this is always a
        // real step: the world before the fresh entry goes down.
        self.stamp();
        let Some(doc) = self.sprites[slot].ron.as_mut() else {
            return;
        };
        if let Some(n) = seq_add(&mut doc.root, &cpath) {
            doc.rows = ron_tree::layout(&doc.root);
            doc.clamp_scroll();
            let mut fresh = Vec::new();
            scan_spots(&doc.root, &mut Vec::new(), "", &mut fresh);
            let mut want = cpath.clone();
            want.push(n as u16);
            doc.edit = fresh.iter().position(|x| x.path == want);
            self.status = if doc.edit.is_some() {
                format!("added entry #{n} — click the sprite to place it")
            } else {
                format!("added entry #{n} to the list")
            };
        }
    }

    /// A sidecar view's [-]: remove the entry the row names from its
    /// sequence — the picked editing goes with it.
    fn row_del(&mut self, ctx: &mut frost::Context, slot: usize, row: usize) {
        let Some(row) = self
            .sprites
            .get(slot)
            .and_then(|sp| sp.ron.as_ref())
            .and_then(|doc| doc.rows.get(row))
        else {
            return;
        };
        let path = row.path.clone();
        if path.is_empty() {
            return;
        }
        // The removal is coming: remember the entry while it lives.
        self.stamp();
        let mut spots = Vec::new();
        if let Some(doc) = self.sprites[slot].ron.as_ref() {
            scan_spots(&doc.root, &mut Vec::new(), "", &mut spots);
        }
        if spot_of_row(&spots, &path).is_some() && self.active != slot {
            self.active = slot;
            self.selection = None;
            self.last_click = None;
            self.sync_work(ctx);
        }
        let Some(doc) = self.sprites[slot].ron.as_mut() else {
            return;
        };
        let n = *path.last().expect("a non-empty path's last index") as usize;
        let cpath = &path[..path.len() - 1];
        if seq_remove(&mut doc.root, cpath, n) {
            doc.rows = ron_tree::layout(&doc.root);
            doc.clamp_scroll();
            doc.edit = None;
            self.status = format!("removed entry #{n} from the list");
        }
    }

    /// A press in a sidecar view's tree that travels past the click
    /// tolerance becomes a reorder: the entry the press started on
    /// follows the pointer, sliding one seat per crossed row.
    fn ron_drag_start(&mut self, slot: usize, row: usize) {
        // A reorder is one command however many rows it shuffles: the
        // world before the drag goes down now.
        self.stamp();
        let Some(doc) = self.sprites.get(slot).and_then(|sp| sp.ron.as_ref()) else {
            return;
        };
        let Some(row) = doc.rows.get(row) else {
            return;
        };
        let mut path = row.path.clone();
        let Some(seat) = path.pop() else {
            return;
        };
        self.ron_drag = Some((slot, path, seat as usize));
    }

    /// The pointer's row while a sidecar reorder rides: the dragged
    /// entry slides to the seat the pointer crosses, one seat at a
    /// time, and the status follows each landing.
    fn ron_drag_move(&mut self, slot: usize, row: Option<usize>) {
        let Some((_, cpath, from)) = self.ron_drag.clone() else {
            return;
        };
        let Some(j) = row else {
            return;
        };
        let Some(doc) = self.sprites.get(slot).and_then(|sp| sp.ron.as_ref()) else {
            return;
        };
        let Some(r) = doc.rows.get(j) else {
            return;
        };
        let to = (r.path.len() == cpath.len() + 1
            && r.path.starts_with(&cpath)
            && !matches!(r.vkind, ron_tree::VKind::Close))
        .then(|| r.path[cpath.len()] as usize);
        let Some(doc) = self.sprites[slot].ron.as_mut() else {
            return;
        };
        if let Some(to) = to
            && let Some(to) = seq_move(&mut doc.root, &cpath, from, to)
        {
            doc.rows = ron_tree::layout(&doc.root);
            doc.clamp_scroll();
            self.ron_drag = Some((slot, cpath, to));
            self.status = format!("reordering: entry #{from} -> #{to}");
        }
    }
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // The animation clock runs on wall time; when it rolls to a new
        // frame, the work area repaints its layer pool to that frame.
        if self.anim.tick(dt) {
            self.sync_work(ctx);
        }

        let (w, h) = ctx.size();
        // The exit guard: Escape asks, the box answers. While the box
        // stands every other key is dead and the pointer belongs to
        // the box alone — Enter or Y leaves, Escape or N stays. An
        // Escape pressed when nothing is held opens the box; while a
        // picked position or a held block stands, Escape still lets
        // go of that first, as ever.
        let esc_now = ctx.key_down_raw(frost::KeyCode::Escape);
        let esc_edge = esc_now && !self.was_esc_prompt;
        self.was_esc_prompt = esc_now;
        if self.quit_prompt {
            ctx.set_keys_frozen(true);
            if ctx.key_down_raw(frost::KeyCode::Enter) || ctx.key_down_raw(frost::KeyCode::KeyY) {
                self.quit_prompt = false;
                ctx.exit();
            } else if esc_edge || ctx.key_down_raw(frost::KeyCode::KeyN) {
                self.quit_prompt = false;
                self.status = String::from("staying");
            }
            // The box ate this Escape: the let-go handlers further
            // down must not also hear it.
            self.was_esc = esc_now;
            self.was_block_esc = esc_now;
        } else {
            ctx.set_keys_frozen(false);
            let holding = self.clip.is_some()
                || self.sel.is_some()
                || self.sel_drag.is_some()
                || self.ron().is_some_and(|d| d.edit.is_some());
            if esc_edge && !holding {
                self.quit_prompt = true;
                self.was_esc = true;
                self.was_block_esc = true;
                self.status = String::from("quit? — Enter/Y leaves, Escape/N stays");
            }
        }
        // The window manager's close button asks the same question:
        // an explicit request, like the menu's Quit — the box opens
        // whatever the bench holds, and a repeat request while the
        // box stands only asks again.
        if ctx.close_requested() && !self.quit_prompt {
            self.quit_prompt = true;
            self.status = String::from("quit? — Enter/Y leaves, Escape/N stays");
        }
        let box_up = self.quit_prompt;
        let down = ctx.mouse_button_down(frost::MouseButton::Left) && !box_up;
        let rdown = ctx.mouse_button_down(frost::MouseButton::Right) && !box_up;
        let mdown = ctx.mouse_button_down(frost::MouseButton::Middle) && !box_up;
        let rpressed = rdown && !self.was_rdown;
        let rreleased = !rdown && self.was_rdown;
        self.was_rdown = rdown;
        let mpressed = mdown && !self.was_mdown;
        let mreleased = !mdown && self.was_mdown;
        self.was_mdown = mdown;
        let pressed = down && !self.was_down;
        let released = !down && self.was_down;
        self.was_down = down;
        let pos = ctx.mouse_position();
        let size = self.work_size();
        // The active sprite's center in window coordinates: the work
        // area's center plus the pan offset.
        let view = [self.offset[0], self.offset[1] + WORK_Y];
        // The tile-map desk's canvas behavior: no sprite to pick, drag
        // or crop — the canvas is a map camera now.
        let tile_view = self.view == View::TileMap;
        if self.first_frame {
            self.first_frame = false;
            self.sync_work(ctx);
        }
        self.sync_band();

        // Every sidecar tree re-pins its scroll every frame, so folding
        // a node, resizing a panel or closing a file can never strand a
        // view past its rows. The view rectangles the wheel and the pan
        // guard read are LAST frame's; clicks read the fresh ones, laid
        // out below.
        for sp in self.sprites.iter_mut() {
            if let Some(doc) = &mut sp.ron {
                doc.clamp_scroll();
            }
        }
        let over_ron = pos.is_some_and(|p| self.ron_views.iter().any(|v| in_rect(v.panel, p)));

        // The wheel zooms: multiplicative per line (up zooms in), clamped
        // to the slider's range, anchored so the texture point under the
        // cursor stays put — the offset absorbs the scale change the cursor
        // itself would have drifted. With the cursor outside the window the
        // anchor is the work area's center, so only the zoom changes.
        let wheel = if self.quit_prompt {
            0.0
        } else {
            ctx.mouse_wheel()
        };
        // The wheel scrolls the sidecar tree under the cursor —
        // sideways with Shift — while it rests on an open view's body;
        // anywhere else it keeps zooming.
        let mut scrolled = false;
        if wheel != 0.0
            && let Some(p) = pos
        {
            let slot = self
                .ron_views
                .iter()
                .find(|v| !v.folded && in_rect(v.body, p))
                .map(|v| v.slot);
            if let Some(doc) = slot.and_then(|s| self.sprites[s].ron.as_mut()) {
                if ctx.key_down(frost::KeyCode::ShiftLeft)
                    || ctx.key_down(frost::KeyCode::ShiftRight)
                {
                    doc.sx -= wheel * 2.0 * RON_ADV;
                } else {
                    doc.scroll -= wheel * RON_LINE * 2.0;
                }
                doc.clamp_scroll();
                scrolled = true;
            }
        }
        // The slot strip's window for this frame: its plate count —
        // one per sprite plus the spare loader plate — the pool's
        // visible width, and the scroll clamped to what they allow.
        // (`f32::MAX` parked in the scroll means "ride to the end",
        // how a freshly loaded sprite earns its view.)
        let slot_count = self.sprites.len() + 1;
        let sv = strip_view(w, slot_count);
        self.slot_scroll = self.slot_scroll.clamp(0.0, sv.scroll_max);
        // The tileset panel's frame-state: its grid, its cell size at
        // the panel's own zoom, and where it stands — the live dragged
        // spot while a drag runs, the stored home otherwise, always
        // clamped into the desk. Press, wheel, ghost and paint all
        // read this one geometry; nobody recomputes it.
        let (brows, bcols) = self.active().and_then(|sp| sp.atlas).unwrap_or((1, 1));
        let bcell = band_cell(brows, bcols, w, h, self.band_zoom);
        let bhome = self
            .band_origin
            .unwrap_or_else(|| band_origin_default(w, h));
        let bor = band_clamp(
            match (self.band_drag, pos) {
                (Some((_, from, origin0)), Some(p)) => {
                    [origin0[0] + (p[0] - from[0]), origin0[1] + (p[1] - from[1])]
                }
                _ => bhome,
            },
            bcell,
            brows,
            bcols,
            w,
            h,
        );
        let brects = band_rects(brows, bcols, bcell, bor);
        let bcells = self.cell_shapes.len().min(brects.len());
        let bplate = if tile_view {
            band_plate(&brects, bcells)
        } else {
            None
        };
        if wheel != 0.0 && !scrolled {
            let over_strip = pos.is_some_and(|p| p[1] < -h / 2.0 + STRIP_H);
            let over_band = bplate.is_some_and(|r| p_in(pos, r));
            if over_band {
                // Over the tileset panel the wheel belongs to the
                // panel: the rack's cells grow and shrink, the canvas
                // does not care.
                self.band_zoom =
                    (self.band_zoom * WHEEL_ZOOM.powf(wheel)).clamp(BAND_ZOOM_MIN, BAND_ZOOM_MAX);
            } else if over_strip && sv.scroll_max > 0.0 {
                // The wheel belongs to the strip while the cursor
                // rests on it and there is something to scroll.
                self.slot_scroll = (self.slot_scroll - wheel * 48.0).clamp(0.0, sv.scroll_max);
            } else {
                let z1 = (self.zoom * WHEEL_ZOOM.powf(wheel)).clamp(ZOOM_MIN, ZOOM_MAX);
                if z1 != self.zoom {
                    let k = z1 / self.zoom;
                    let [ax, ay] = pos.unwrap_or([0.0, WORK_Y]);
                    self.offset = [
                        ax - (ax - self.offset[0]) * k,
                        (ay - WORK_Y) - ((ay - WORK_Y) - self.offset[1]) * k,
                    ];
                    self.zoom = z1;
                }
            }
        }

        // A middle-drag — the wheel button held — grabs the work area:
        // the sprite follows the cursor's frame-to-frame movement, so it
        // always lands under the pointer. The Atlas view opts out: its
        // tileset is nailed down, and the wheel is the only way the
        // picture moves.
        if mdown
            && !over_ron
            && self.view != View::Atlas
            && let (Some([lx, ly]), Some([mx, my])) = (self.last_mouse, pos)
        {
            self.offset[0] += mx - lx;
            self.offset[1] += my - ly;
        }
        // The middle click — the wheel button pressed and released
        // without travel — moves the held block, when there is one:
        // the frame it was copied from is swept, the block lands
        // cursor-anchored, and the frame travels with the drop. One
        // gesture, one undo step. A middle press that travels is the
        // pan above, never a move.
        if mpressed && tile_view && self.clip.is_some() && !self.ui.hovering() && !over_ron {
            self.m_press = pos;
        } else if mpressed {
            self.m_press = None;
        }
        if mreleased
            && tile_view
            && let (Some(from), Some(p)) = (self.m_press.take(), pos)
            && self.clip.is_some()
            && ((p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2)).sqrt() < CLICK_TOL
            && !self.ui.hovering()
            && !over_ron
            && let Some((col, row)) = desk_cell(self.active(), p, view, self.zoom)
        {
            let before = self.capture();
            let clip = self.clip.clone().expect("a block is held");
            let (n, erased, sel) = clip_move(&mut self.maps, &clip, self.sel, col, row);
            self.sel = sel;
            log::info!("block: moved to ({col},{row}) wrote {n} cut {erased}");
            if n > 0 || erased {
                push_step(&mut self.undo_stack, &mut self.redo_stack, before);
                self.status = format!(
                    "moved {n} cell{} — the block is still held",
                    if n == 1 { "" } else { "s" }
                );
                self.sync_work(ctx);
            }
        }

        // The right button picks a position back from the sprite: the
        // marker nearest the click lights up its row — the path unfolds
        // and the view scrolls that row to its centre — and the
        // position is picked, so the next LEFT click moves it. A press
        // that travels is a pan attempt, not a pick.
        if rpressed
            && !tile_view
            && let Some(p) = pos
            && !over_ron
        {
            self.r_press = Some(p);
        }
        if rreleased
            && let (Some(from), Some(p)) = (self.r_press.take(), pos)
            && ((p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2)).sqrt() < CLICK_TOL
            && !self.sprites.is_empty()
        {
            let [px, py] = tex_point(p, size, view, self.zoom);
            let inside = px >= 0.0 && px <= size[0] && py >= 0.0 && py <= size[1];
            if inside && self.sprites[self.active].ron.is_some() {
                let slot = self.active;
                let mut spots: Vec<Spot> = Vec::new();
                if let Some(d) = self.sprites[slot].ron.as_ref() {
                    scan_spots(&d.root, &mut Vec::new(), "", &mut spots);
                }
                if let Some(pi) = spot_pick(&spots, p, size, view, self.zoom, SPOT_HIT_R) {
                    let (path, label) = (spots[pi].path.clone(), spots[pi].label.clone());
                    if let Some(doc) = self.sprites[slot].ron.as_mut() {
                        doc.edit = Some(pi);
                        unfold_path(&mut doc.root, &path);
                        doc.rows = ron_tree::layout(&doc.root);
                        if let Some(i) = doc.rows.iter().position(|r| r.path == path) {
                            let track = self
                                .ron_views
                                .iter()
                                .find(|v| v.slot == slot)
                                .filter(|v| !v.folded && v.body != [0.0; 4])
                                .map(|v| v.body[3] - v.body[1]);
                            if let Some(track) = track {
                                let total = doc.rows.len() as f32 * RON_LINE + RON_PAD;
                                doc.scroll = ((i as f32 + 0.5) * RON_LINE + RON_PAD - track / 2.0)
                                    .clamp(0.0, (total - track).max(0.0));
                            }
                        }
                        doc.clamp_scroll();
                        self.status = format!("editing '{label}' — click the sprite to move it");
                    }
                }
            }
        }

        // The UI frame: the panels claim the mouse over their bodies — a
        // press the UI holds is never a selection drag, a slot touch or a
        // sprite click.
        self.ui.begin(ctx);
        // While the exit box stands, the UI owns the window whole:
        // every widget outside the box is drawn but inert.
        self.ui.set_modal(self.quit_prompt);
        // Which panels this frame declares: Markers keeps the whole
        // editing desk, Atlas keeps only its grid. The slots below and
        // the work area are the views' common ground — a sprite is
        // picked and worked on the same way in either.
        let markers_view = self.view == View::Markers;
        // The Animation panel: a frame is a set of layers holding for
        // its own time; playback wraps (loop) or bounces (ping-pong).
        // Buttons set flags acted on after the panels.
        let mut want_prev_frame = false;
        let mut want_next_frame = false;
        let mut want_add_frame = false;
        let mut want_del_frame = false;
        let mut want_add_layer = false;
        let mut want_del_layer = false;
        let mut want_play = false;
        let mut looping = self.anim.looping;
        let mut next_time = self
            .anim
            .frames
            .get(self.anim.frame)
            .map_or(TIME_DEFAULT, |f| f.next_time);
        let anim_line = if self.anim.frames.is_empty() {
            String::from("no frames yet")
        } else {
            let f = &self.anim.frames[self.anim.frame];
            format!(
                "frame {}/{} · {} layer(s) · {}",
                self.anim.frame + 1,
                self.anim.frames.len(),
                f.layers.len(),
                if self.anim.playing {
                    "playing"
                } else {
                    "paused"
                }
            )
        };
        let play_label = if self.anim.playing { "stop" } else { "play" };
        if markers_view {
            self.ui.panel(
                ctx,
                "Animation",
                [w / 2.0 - PANEL_W / 2.0 - 20.0, h / 2.0 - 120.0],
                PANEL_W,
                |ui, ctx| {
                    ui.label(ctx, &anim_line);
                    ui.table(
                        ctx,
                        "anim",
                        &[
                            frost::Col::auto(frost::Align::Center),
                            frost::Col::auto(frost::Align::Center),
                            frost::Col::auto(frost::Align::Center),
                        ],
                        |ui, ctx| {
                            if ui.button(ctx, "prev") {
                                want_prev_frame = true;
                            }
                            if ui.button(ctx, "add frame") {
                                want_add_frame = true;
                            }
                            if ui.button(ctx, "del frame") {
                                want_del_frame = true;
                            }
                            if ui.button(ctx, "next") {
                                want_next_frame = true;
                            }
                            if ui.button(ctx, "add layer") {
                                want_add_layer = true;
                            }
                            if ui.button(ctx, "del layer") {
                                want_del_layer = true;
                            }
                            ui.label(ctx, "till next");
                            ui.slider_track(ctx, "frame time", &mut next_time, TIME_MIN, TIME_MAX);
                            ui.readout(ctx, &format!("{next_time:.2}s"));
                        },
                    );
                    ui.space(6.0);
                    ui.checkbox(ctx, "loop  (off = ping-pong)", &mut looping);
                    if ui.button(ctx, play_label) {
                        want_play = true;
                    }
                },
            );
        }

        // The Atlas panel: how the active sprite splits into tiles. The
        // sliders set the grid's rows and columns, the grid draws over
        // the sprite, and the grid persists as the sidecar root's
        // `atlas` field, which Save carries with the PNG. 1 x 1 is no
        // grid.
        let (mut atlas_rows, mut atlas_cols) = self
            .active()
            .and_then(|sp| sp.atlas)
            .map_or((1.0, 1.0), |(rows, cols)| (rows as f32, cols as f32));
        let mut want_clear_atlas = false;
        let atlas_line = match self.active().and_then(|sp| sp.atlas) {
            Some((rows, cols)) => format!("{rows} x {cols} · {} tiles", rows * cols),
            None => String::from("no grid · 1 x 1 is none"),
        };
        if !markers_view {
            self.ui.panel(
                ctx,
                "Atlas",
                [w / 2.0 - PANEL_W / 2.0 - 20.0, h / 2.0 - 330.0],
                PANEL_W,
                |ui, ctx| {
                    ui.label(ctx, &atlas_line);
                    ui.table(
                        ctx,
                        "atlas",
                        &[
                            frost::Col::auto(frost::Align::Left),
                            frost::Col::stretch(1.0, frost::Align::Left),
                            frost::Col::auto(frost::Align::Center),
                        ],
                        |ui, ctx| {
                            ui.label(ctx, "rows");
                            ui.slider_track(ctx, "atlas rows", &mut atlas_rows, 1.0, ATLAS_MAX);
                            ui.readout(ctx, &format!("{atlas_rows:.0}"));
                            ui.label(ctx, "cols");
                            ui.slider_track(ctx, "atlas cols", &mut atlas_cols, 1.0, ATLAS_MAX);
                            ui.readout(ctx, &format!("{atlas_cols:.0}"));
                        },
                    );
                    ui.space(6.0);
                    if ui.button(ctx, "clear") {
                        want_clear_atlas = true;
                    }
                },
            );
        }
        // The sliders' read-back: when the grid moved, write it into the
        // active sprite — and, beside its sidecar's root, so Save
        // carries it with the PNG. The sidecar's rows rebuild from the
        // tree, the way every other sidecar edit does.
        let rows = atlas_rows.round().max(1.0) as usize;
        let cols = atlas_cols.round().max(1.0) as usize;
        let next = (rows > 1 || cols > 1).then_some((rows, cols));
        let setting = self.active().is_some_and(|sp| sp.atlas != next);
        let clearing = want_clear_atlas && self.active().is_some_and(|sp| sp.atlas.is_some());
        if setting || clearing {
            // The grid moved — by slider step or by clear: one step of
            // history per movement, the way any other command counts.
            self.stamp();
        }
        if let Some(sp) = self.sprites.get_mut(self.active) {
            if sp.atlas != next {
                sp.atlas = next;
                if let Some(doc) = sp.ron.as_mut()
                    && set_atlas(&mut doc.root, next)
                {
                    doc.rows = ron_tree::layout(&doc.root);
                    doc.clamp_scroll();
                }
            }
            if want_clear_atlas {
                sp.atlas = None;
                if let Some(doc) = sp.ron.as_mut()
                    && set_atlas(&mut doc.root, None)
                {
                    doc.rows = ron_tree::layout(&doc.root);
                    doc.clamp_scroll();
                }
            }
        }

        // Ctrl-O's and Ctrl-Z's rising edges: the open dialog, and one
        // step back through the active sprite's crops.
        let ctrl =
            ctx.key_down(frost::KeyCode::ControlLeft) || ctx.key_down(frost::KeyCode::ControlRight);
        let open_key = ctrl && ctx.key_down(frost::KeyCode::KeyO);
        let shift =
            ctx.key_down(frost::KeyCode::ShiftLeft) || ctx.key_down(frost::KeyCode::ShiftRight);
        let undo_key = ctrl && ctx.key_down(frost::KeyCode::KeyZ) && !shift;
        let redo_key = ctrl && shift && ctx.key_down(frost::KeyCode::KeyZ);
        if open_key && !self.was_open {
            self.open_dialog(ctx);
        }
        if undo_key && !self.was_undo {
            self.undo(ctx);
        }
        if redo_key && !self.was_redo {
            self.redo(ctx);
        }
        // Ctrl-+ and Ctrl-- step the zoom about the work area's centre,
        // one wheel-notch per press. The characters lead, exactly as for
        // the overlay's Alt pair: a physical code names a US-ANSI
        // position, so the Swedish `+` key reports the US `-` code and a
        // physical fallback would zoom the WRONG way on that layout —
        // only the typed character follows the user's keys. The set is
        // every character the +/− keys type with Ctrl held across
        // platforms (the plain glyphs, the `=` the US + key carries,
        // and the macOS Option weavings); the numpad codes, whose signs
        // are layout-independent, join as the safety net.
        let zoom_in = ctrl
            && (ctx.char_down('+')
                || ctx.char_down('=')
                || ctx.char_down('\u{b1}')
                || ctx.char_down('\u{2212}')
                || ctx.key_down(frost::KeyCode::NumpadAdd));
        let zoom_out = ctrl
            && (ctx.char_down('-')
                || ctx.char_down('_')
                || ctx.char_down('\u{2013}')
                || ctx.char_down('\u{2014}')
                || ctx.key_down(frost::KeyCode::NumpadSubtract));
        if zoom_in && !self.was_zoom_in {
            self.zoom = (self.zoom * WHEEL_ZOOM).clamp(ZOOM_MIN, ZOOM_MAX);
        }
        if zoom_out && !self.was_zoom_out {
            self.zoom = (self.zoom / WHEEL_ZOOM).clamp(ZOOM_MIN, ZOOM_MAX);
        }
        self.was_open = open_key;
        self.was_undo = undo_key;
        self.was_redo = redo_key;
        self.was_zoom_in = zoom_in;
        self.was_zoom_out = zoom_out;
        // The brush's orientation, on the tile desk only: X and Y
        // mirror the cell, R turns a quarter counterclockwise, Shift-R back.
        // Characters lead, as everywhere the keyboard turns — a letter
        // follows the user's layout — but the turn's DIRECTION reads
        // the physical Shift, so a Caps-Locked keyboard still turns
        // the plain way on a plain R.
        let flipx_key = tile_view && (ctx.char_down('x') || ctx.char_down('X'));
        let flipy_key = tile_view && (ctx.char_down('y') || ctx.char_down('Y'));
        let turn_key = tile_view && (ctx.char_down('r') || ctx.char_down('R'));
        if flipx_key && !self.was_flipx {
            self.brush = tfm_flip_x(self.brush);
            self.status = if self.brush & 1 != 0 {
                "flip x on".to_owned()
            } else {
                "flip x off".to_owned()
            };
        }
        if flipy_key && !self.was_flipy {
            self.brush = tfm_flip_y(self.brush);
            self.status = if self.brush & 2 != 0 {
                "flip y on".to_owned()
            } else {
                "flip y off".to_owned()
            };
        }
        if turn_key && !self.was_turn {
            // The keyboard brings no Alt or Control to R — but the
            // turn's direction still comes from the one table, so
            // key and mouse can never disagree about "plain".
            self.brush = dress_brush(self.brush, dress_gesture(shift, false, false));
            self.status = format!("turned {}", turn_note(self.brush));
        }
        self.was_flipx = flipx_key;
        self.was_flipy = flipy_key;
        self.was_turn = turn_key;

        // Space's rising edge starts and stops the animation clock.
        let space = ctx.key_down(frost::KeyCode::Space);
        if space && !self.was_space {
            self.anim.playing = !self.anim.playing;
            self.anim.dir = 1;
        }
        self.was_space = space;

        // Escape's rising edge lets go of a picked position.
        let esc = ctx.key_down(frost::KeyCode::Escape);
        if esc && !self.was_esc {
            let mut was = false;
            if let Some(doc) = self.ron_mut() {
                was = doc.edit.take().is_some();
            }
            if was {
                self.status = String::from("edit: released");
            }
        }
        self.was_esc = esc;

        let (row_h, pad) = {
            let st = self.ui.style_mut();
            (st.row_h, st.pad)
        };
        // The sidecar views: one `Ui` panel per open file, each titled
        // by its file's name — every bar drags its panel and folds it
        // whole, like the View and Animation panels, and the views
        // cascade from the work area's lower right. A panel is declared
        // at its document's own size, and every rectangle the tree,
        // handles and close marks work from is the plate the UI ACTUALLY
        // painted this frame — the plate grows from the measured height
        // of the frame before, and only that matches the pixels.
        self.ron_views.clear();
        let open_docs: Vec<(usize, String, f32, f32)> = self
            .sprites
            .iter()
            .enumerate()
            .filter(|_| markers_view)
            .filter_map(|(i, sp)| sp.ron.as_ref().map(|d| (i, d.name.clone(), d.vw, d.vh)))
            .collect();
        for (i, title, vw, vh) in open_docs {
            let mut open = false;
            self.ui.panel(
                ctx,
                &title,
                ron_default(w, h, row_h, pad, self.ron_views.len()),
                vw,
                |ui, _ctx| {
                    ui.space(vh);
                    open = true;
                },
            );
            if let Some(panel) = self.ui.panel_rect(&title) {
                // The body is the plate between the title bar and the
                // plate's bottom edge, each side padded; a plate that
                // is still just its bar (the snap-open frame) has no
                // body worth laying out.
                let body = if open {
                    [panel[0], panel[1] + pad, panel[2], panel[3] - row_h - pad]
                } else {
                    [0.0; 4]
                };
                let body = if body[3] - body[1] >= RON_LINE {
                    body
                } else {
                    [0.0; 4]
                };
                let close = [panel[2] - 17.0, panel[3] - row_h / 2.0];
                self.ron_views.push(RonView {
                    slot: i,
                    body,
                    panel,
                    close,
                    folded: !open,
                });
            }
        }

        // The menu bar, declared after every panel so it is the last
        // declared — which is what lets an open list float above the
        // plates and win their clicks. Declaring it here (after all the
        // panels, before the collected presses) puts its whole strip into
        // `ui.hovering()` too: no slot or panel drag acts through it.
        // The bar's trailing notes: the last command, then the view's
        // own name at the very end. An empty status draws nothing.
        let mut notes: Vec<&str> = Vec::with_capacity(2);
        if !self.status.is_empty() {
            notes.push(self.status.as_str());
        }
        notes.push(self.view.name());
        // The View menu is assembled each frame: the three named corners
        // all switch the workbench, and beneath them the view's own
        // state —
        // the zoom as a readout, the checker's two greys as live
        // sliders. The panel that carried all of it is gone; the menu
        // wears its clothes. The last click needs no line: the bar's
        // status note already tells it.
        let zoom_note = format!("{:.2}", self.zoom);
        let view_menu = frost::Menu {
            title: "View",
            items: &[
                frost::MenuItem::new("Markers", "").checked(self.view == View::Markers),
                frost::MenuItem::new("Atlas", "").checked(self.view == View::Atlas),
                frost::MenuItem::new("Tile Map", "").checked(self.view == View::TileMap),
                frost::MenuItem::SEPARATOR,
                frost::MenuItem::readout("Zoom", "Ctrl +/Ctrl -", &zoom_note),
                frost::MenuItem::slider("Light", 0.0, 1.0, self.light),
                frost::MenuItem::slider("Dark", 0.0, 1.0, self.dark),
            ],
        };
        // The brush's own menu, and it has no reason to exist off the
        // tile desk: transforms paint cells, and only there are cells.
        let transform_menu = frost::Menu {
            title: "Transform",
            items: &[
                frost::MenuItem::new("Flip X", "X").checked(self.brush & 1 != 0),
                frost::MenuItem::new("Flip Y", "Y").checked(self.brush & 2 != 0),
                frost::MenuItem::SEPARATOR,
                frost::MenuItem::new("Rotate 90 CW", "R"),
                frost::MenuItem::new("Rotate 90 CCW", "Shift-R"),
                frost::MenuItem::readout("Turn", "", turn_note(self.brush)),
            ],
        };
        // Operations is a sprite-desk menu: its one verb crops the
        // active texture, and the tile-map desk has no texture to cut
        // — only cells. There, the Transform menu wears the bar.
        let menus: Vec<frost::Menu> = if tile_view {
            vec![FILE_MENU, view_menu, transform_menu]
        } else {
            vec![FILE_MENU, view_menu, OPERATIONS_MENU]
        };
        match self.ui.menu_bar(ctx, &menus, &notes) {
            Some(frost::MenuEvent::Chose(menu, item)) => {
                log::info!("menu: {menu} / {item}");
                // Every acted-on line names itself first; the verbs that
                // write a richer line of their own (undo, crop, save)
                // simply write over it, and the note reads theirs.
                self.status = item.to_lowercase();
                match (menu, item) {
                    // Switching the desk re-syncs the work layer: the
                    // tile-map desk hides the sprite (it is the paint
                    // source, not the picture), and leaving it brings
                    // the sprite — or the animation frame — back.
                    ("View", "Markers") => {
                        self.view = View::Markers;
                        self.sync_work(ctx);
                    }
                    ("View", "Atlas") => {
                        self.view = View::Atlas;
                        self.sync_work(ctx);
                    }
                    ("View", "Tile Map") => {
                        self.view = View::TileMap;
                        self.sync_work(ctx);
                    }
                    // The brush turns and mirrors; the maps keep what
                    // they were painted with, so nothing re-syncs.
                    ("Transform", "Flip X") => self.brush = tfm_flip_x(self.brush),
                    ("Transform", "Flip Y") => self.brush = tfm_flip_y(self.brush),
                    ("Transform", "Rotate 90 CW") => self.brush = tfm_turn(self.brush, true),
                    ("Transform", "Rotate 90 CCW") => self.brush = tfm_turn(self.brush, false),
                    ("File", "Open") => self.open_dialog(ctx),
                    ("File", "Close") => self.close_active(ctx),
                    ("File", "Save") => self.save(ctx),
                    ("File", "Save as") => self.save_as(ctx),
                    ("File", "Undo") => self.undo(ctx),
                    ("File", "Redo") => self.redo(ctx),
                    ("Operations", "Crop") => self.crop(ctx),
                    ("File", "Quit") => {
                        // The same guard Escape raises: the box asks,
                        // the answer decides. Unlike the key — which
                        // first lets go of anything held — a named
                        // click means the question, so the box opens
                        // whatever the bench holds; the held state
                        // waits through the question untouched.
                        self.quit_prompt = true;
                        self.status = String::from("quit? — Enter/Y leaves, Escape/N stays");
                    }
                    _ => {}
                }
            }
            // A grey travelled its slider: the value lands quietly — the
            // checker re-greys live, every frame — and the status note
            // keeps the last real command's words.
            Some(frost::MenuEvent::Slid(_, line, v)) => match line {
                "Light" => self.light = v,
                "Dark" => self.dark = v,
                _ => {}
            },
            None => {}
        }

        // The collected button presses, now outside every panel closure.

        // The animation edits. The duration slider and loop checkbox
        // wrote their locals; commit them, then act on the buttons.
        let time_moved = self
            .anim
            .frames
            .get(self.anim.frame)
            .is_some_and(|f| f.next_time != next_time);
        if time_moved {
            // The duration slider steps into history as it moves; each
            // step is a command the user dragged through.
            self.stamp();
        }
        if let Some(f) = self.anim.frames.get_mut(self.anim.frame) {
            f.next_time = next_time;
        }
        if self.anim.looping != looping {
            self.stamp();
            self.anim.looping = looping;
        }
        if want_add_frame {
            // A new frame, inserted after the one being edited, opens
            // with the active sprite as its first layer.
            let at = (self.anim.frame + 1).min(self.anim.frames.len());
            self.stamp();
            let shot = self
                .active()
                .and_then(|sp| png_bytes(&sp.current).ok())
                .and_then(|png| {
                    frost::Shape::sprite_bytes_nearest(&png)
                        .ok()
                        .map(|s| (s, png))
                });
            self.anim.frames.insert(
                at,
                Frame {
                    layers: shot
                        .map(|(shape, png)| vec![Layer { shape, png }])
                        .unwrap_or_default(),
                    next_time: TIME_DEFAULT,
                },
            );
            self.anim.set_frame(at);
            self.sync_work(ctx);
        }
        if want_add_layer {
            let shot = self
                .active()
                .and_then(|sp| png_bytes(&sp.current).ok())
                .and_then(|png| {
                    frost::Shape::sprite_bytes_nearest(&png)
                        .ok()
                        .map(|s| (s, png))
                });
            let room = self
                .anim
                .frames
                .get(self.anim.frame)
                .is_some_and(|f| f.layers.len() < LAYER_NODES);
            if shot.is_some() && room {
                self.stamp();
                if let Some((shape, png)) = shot
                    && let Some(f) = self.anim.frames.get_mut(self.anim.frame)
                {
                    f.layers.push(Layer { shape, png });
                }
            }
            self.sync_work(ctx);
        }
        if want_del_layer {
            let some = self
                .anim
                .frames
                .get(self.anim.frame)
                .is_some_and(|f| !f.layers.is_empty());
            if some {
                self.stamp();
                if let Some(f) = self.anim.frames.get_mut(self.anim.frame) {
                    f.layers.pop();
                }
            }
            self.sync_work(ctx);
        }
        if want_del_frame {
            if self.anim.frame < self.anim.frames.len() {
                self.stamp();
                self.anim.frames.remove(self.anim.frame);
                self.anim.set_frame(
                    self.anim
                        .frame
                        .min(self.anim.frames.len().saturating_sub(1)),
                );
            }
            self.sync_work(ctx);
        }
        if want_prev_frame || want_next_frame {
            let n = self.anim.frames.len();
            if n > 0 {
                let delta = if want_next_frame { 1 } else { -1 };
                let nf = (self.anim.frame as isize + delta).rem_euclid(n as isize) as usize;
                self.anim.set_frame(nf);
                self.sync_work(ctx);
            }
        }
        if want_play {
            self.anim.playing = !self.anim.playing;
            self.anim.dir = 1;
        }

        // Ctrl, polled once: Ctrl + left is the mouse hand's
        // eraser, twin of the Delete key.
        let ctrl_held =
            ctx.key_down(frost::KeyCode::ControlLeft) || ctx.key_down(frost::KeyCode::ControlRight);
        // The keyboard eraser's state: held drags erase, a tap erases
        // the one cell under the cursor.
        let del = ctx.key_down(frost::KeyCode::Delete) || ctx.key_down(frost::KeyCode::Backspace);
        let del_edge = del && !self.was_del;
        self.was_del = del;
        // Shift selects; Escape drops whatever is held.
        let shift_held =
            ctx.key_down(frost::KeyCode::ShiftLeft) || ctx.key_down(frost::KeyCode::ShiftRight);
        let esc = ctx.key_down(frost::KeyCode::Escape);
        let esc_edge = esc && !self.was_block_esc;
        self.was_block_esc = esc;
        // The left button's work: press lands on a slot, on the work area
        // or nowhere the demo owns; release decides — click or drag.
        if pressed
            && !self.ui.hovering()
            && let Some(p) = pos
        {
            // The strip's scrollbar claims its bottom band first; the
            // plates sit above it and never share its pixels.
            let scrub = sv.scroll_max > 0.0
                && p[1] <= -h / 2.0 + (SCROLL_Y + SCROLL_H / 2.0)
                && p[0] >= sv.x0
                && p[0] <= sv.x0 + sv.track_w;
            if scrub {
                self.slot_scrub = Some(p[0] - scroll_knob(&sv, self.slot_scroll).0);
            } else if tile_view && bplate.is_some_and(|r| p_in(Some(p), r)) {
                // The panel claims a press anywhere on its plate: it
                // rides the cursor until release, and release
                // decides — a click picks the cell under it, a drag
                // leaves the panel where it landed.
                self.band_drag = Some(([p[0] - bor[0], p[1] - bor[1]], p, bor));
            } else {
                match slot_at(p, w, h, self.slot_scroll, slot_count) {
                    Some(i) => self.slot_drag = Some((i, p)),
                    None if in_work_area(p, w, h) && !tile_view => self.drag_from = Some(p),
                    // Shift + left drags a selection: release copies
                    // the rectangle into a block brush. The frame
                    // stays to show what is held.
                    None if tile_view && shift_held && in_work_area(p, w, h) => {
                        if let Some((c, r)) = desk_cell(self.active(), p, view, self.zoom) {
                            self.sel_drag = Some([c, r]);
                            self.sel = Some([c, r, c, r]);
                        }
                    }
                    // Ctrl + left is the mouse hand's eraser: the
                    // same stroke Delete opens — a click lifts the
                    // cell under it, a drag sweeps a path.
                    None if tile_view && (ctrl_held || del) && in_work_area(p, w, h) => {
                        self.paint = Some(Paint {
                            erase: true,
                            touched: false,
                            n: 0,
                            key_opened: false,
                        });
                    }
                    // A held block is the brush now: one click stamps
                    // the whole picture, one undo step for all of it.
                    // A click INSIDE the selection frame lays the
                    // block back over the frame — dress the block and
                    // click again to rotate in place, nothing left
                    // a click drops a copy cursor-anchored, gaps
                    // skipped; the held block never touches anything
                    // outside its own cells.
                    None if tile_view && self.clip.is_some() && in_work_area(p, w, h) => {
                        if let Some((col, row)) = desk_cell(self.active(), p, view, self.zoom) {
                            let before = self.capture();
                            let n = match &self.clip {
                                Some(clip) => clip_paste(&mut self.maps, clip, col, row),
                                None => 0,
                            };
                            log::info!("block: pasted at ({col},{row}) wrote {n}");
                            if n > 0 {
                                push_step(&mut self.undo_stack, &mut self.redo_stack, before);
                                self.status = format!(
                                    "pasted {n} cell{} — the block is still held",
                                    if n == 1 { "" } else { "s" }
                                );
                                self.sync_work(ctx);
                            }
                        }
                    }
                    None if tile_view && in_work_area(p, w, h) && self.picked_cell.is_some() => {
                        self.paint = Some(Paint {
                            erase: false,
                            touched: false,
                            n: 0,
                            key_opened: false,
                        });
                    }
                    None => {}
                }
            }
        }
        // Right now only dresses the held tile, and it is safe to
        // press anywhere on the desk: plain turns the brush a quarter
        // counterclockwise, Shift clockwise, Alt flips it over the
        // horizontal, Ctrl over the vertical. On the panel the pressed
        // cell becomes the brush as it dresses; on the desk the brush
        // dresses where it stands. Erasing rides Delete or Ctrl —
        // seen above and below. (Elsewhere right still spot-picks.)
        if rpressed
            && tile_view
            && let Some(p) = pos
            && !over_ron
            && !self.ui.hovering()
            && (band_pick(p, brows, bcols, bcell, bor, bcells).is_some() || in_work_area(p, w, h))
        {
            let shift =
                ctx.key_down(frost::KeyCode::ShiftLeft) || ctx.key_down(frost::KeyCode::ShiftRight);
            let alt =
                ctx.key_down(frost::KeyCode::AltLeft) || ctx.key_down(frost::KeyCode::AltRight);
            let ctrl = ctx.key_down(frost::KeyCode::ControlLeft)
                || ctx.key_down(frost::KeyCode::ControlRight);
            let dressed = match dress_gesture(shift, ctrl, alt) {
                Dress::FlipY => "flipped y",
                Dress::FlipX => "flipped x",
                Dress::Turn(true) => "turned cw",
                Dress::Turn(false) => "turned ccw",
            };
            self.status = match band_pick(p, brows, bcols, bcell, bor, bcells) {
                Some(i) => {
                    // The rack is single-tile country: picking there
                    // drops the block and dresses one tile instead.
                    self.picked_cell = Some(i);
                    self.clip = None;
                    self.sel = None;
                    self.brush = dress_brush(self.brush, dress_gesture(shift, ctrl, alt));
                    format!("tile {} {dressed} — {}", i + 1, turn_note(self.brush))
                }
                None => {
                    if let Some(clip) = &mut self.clip {
                        // A block is held: every verb works the whole
                        // picture — the arrangement and each tile in
                        // it turn and mirror together.
                        dress_block(clip, &mut self.sel, dress_gesture(shift, ctrl, alt));
                        log::info!(
                            "block: {dressed} -> {}x{} cells={:?} tfms={:?} sel={:?}",
                            clip.cols,
                            clip.rows,
                            clip.cells,
                            clip.tfms,
                            self.sel
                        );
                        format!("block {dressed}")
                    } else {
                        self.brush = dress_brush(self.brush, dress_gesture(shift, ctrl, alt));
                        format!("brush {dressed} — {}", turn_note(self.brush))
                    }
                }
            };
        }
        // The eraser rides the keyboard: hold Delete (or Backspace)
        // and click or drag across the desk — or tap the key over
        // the map and the cell under the cursor comes off. Ctrl +
        // click, seen above, opens the very same stroke.
        if del_edge
            && tile_view
            && !over_ron
            && !self.ui.hovering()
            && pos.is_some_and(|p| in_work_area(p, w, h))
        {
            if let Some(sel) = self.sel {
                // A selection standing on the desk: Delete lifts the
                // whole rectangle — and the held block stays, so the
                // sweep can be pasted elsewhere: a move in two beats.
                self.sel = None;
                let before = self.capture();
                if erase_rect(&mut self.maps, sel) {
                    push_step(&mut self.undo_stack, &mut self.redo_stack, before);
                    self.status = String::from("selection erased — click to paste it");
                } else {
                    self.status = String::from("the selection was already clear");
                }
                self.sync_work(ctx);
            } else {
                self.paint = Some(Paint {
                    erase: true,
                    touched: false,
                    n: 0,
                    key_opened: true,
                });
            }
        }
        if self.paint.is_some_and(|st| st.key_opened && !del) {
            self.paint = None;
        }
        // Escape drops the block and its frame; a picked tile brush,
        // if one stands, rides on untouched.
        if esc_edge && (self.clip.is_some() || self.sel.is_some() || self.sel_drag.is_some()) {
            self.clip = None;
            self.sel = None;
            self.sel_drag = None;
            self.status = String::from("selection dropped");
        }
        // The stroke, continued: every frame the painting button is
        // held, the cell under the cursor is written. The undo step is
        // taken on the first cell the stroke actually changes — a
        // click on empty space outside the map stays a no-op all the
        // way down, road unstained.
        let stroke = self
            .paint
            .map(|st| (st.erase, stroke_held(&st, down, del || ctrl_held)));
        if let Some((erase, true)) = stroke
            && let Some(p) = pos
        {
            let brush = self.active().map(|sp| {
                let (arows, acols) = sp.atlas.unwrap_or((1, 1));
                (
                    sp.path.clone(),
                    sp.current.width() as f32 / acols.max(1) as f32,
                    sp.current.height() as f32 / arows.max(1) as f32,
                )
            });
            if let Some((path, tw, th)) = brush {
                let mx = (p[0] - view[0]) / self.zoom;
                let my = (p[1] - view[1]) / self.zoom;
                let (col, row) = grid_cell(mx, my, tw, th);
                // The road keeps the world before the stroke's
                // first change — the snapshot is taken as the write
                // is about to happen, not after it happened.
                let before = self.paint.filter(|st| !st.touched).map(|_| self.capture());
                let changed = if erase {
                    erase_at(&mut self.maps, col, row)
                } else if let Some(cell) = self.picked_cell {
                    paint_at(&mut self.maps, &path, cell, self.brush, col, row)
                } else {
                    false
                };
                if changed {
                    if let Some(before) = before {
                        push_step(&mut self.undo_stack, &mut self.redo_stack, before);
                    }
                    if let Some(st) = &mut self.paint {
                        st.touched = true;
                        st.n += 1;
                    }
                    self.sync_work(ctx);
                }
            }
        }
        // Release closes the stroke, and a stroke that painted says
        // how many cells it moved.
        for (edge, erase, word) in [(released, false, "painted"), (rreleased, true, "erased")] {
            if edge
                && self.paint.is_some_and(|st| st.erase == erase)
                && let Some(st) = self.paint.take()
                && st.touched
            {
                self.status = format!("{word} {} cell{}", st.n, if st.n == 1 { "" } else { "s" });
            }
        }
        // The selection rides the cursor while Shift + left is held;
        // its release harvests — the rectangle becomes a block brush,
        // and the frame stays to show what the desk is holding.
        if let Some([c0, r0]) = self.sel_drag {
            if let (true, Some(p)) = (down, pos) {
                if let Some((c, r)) = desk_cell(self.active(), p, view, self.zoom) {
                    self.sel = Some([c0.min(c), r0.min(r), c0.max(c), r0.max(r)]);
                }
            } else {
                self.sel_drag = None;
                let path = self.active().map(|sp| sp.path.clone());
                match (self.sel, path) {
                    (Some([ca, ra, cb, rb]), Some(path)) if tile_view => {
                        let mut clip = clip_take(
                            &self.maps,
                            &path,
                            ca,
                            ra,
                            (cb - ca + 1) as usize,
                            (rb - ra + 1) as usize,
                        );
                        match clip_trim(&mut clip) {
                            Some((c0, r0, c1, r1)) => {
                                // The frame shrinks to the picture:
                                // turns pivot on it and drops hug it.
                                self.sel = Some([
                                    ca + c0 as i32,
                                    ra + r0 as i32,
                                    ca + c1 as i32,
                                    ra + r1 as i32,
                                ]);
                                let (fw, fh) = ((cb - ca + 1) as usize, (rb - ra + 1) as usize);
                                log::info!(
                                    "block: took sel [{ca},{ra},{cb},{rb}] trimmed to {:?} -> {}x{} cells={:?} tfms={:?}",
                                    self.sel,
                                    clip.cols,
                                    clip.rows,
                                    clip.cells,
                                    clip.tfms
                                );
                                self.status = if (clip.cols, clip.rows) == (fw, fh) {
                                    format!(
                                        "selected {}\u{d7}{} — click to paste, Delete clears the source",
                                        clip.cols, clip.rows
                                    )
                                } else {
                                    format!(
                                        "selected {}\u{d7}{} of {fw}\u{d7}{fh} — the frame trimmed to the picture",
                                        clip.cols, clip.rows
                                    )
                                };
                                self.clip = Some(clip);
                            }
                            None => {
                                self.sel = None;
                                self.status = "the selection held nothing".to_string();
                            }
                        }
                    }
                    _ => self.sel = None,
                }
            }
        }
        // The sidecar views' trees, one widget frame each: the widget
        // claims the close boxes, corner grips and thumbs in view order,
        // drives the scrolls, resizes and reorders, and reports what the
        // release hit.
        let mut bufs: Vec<Vec<frost::TreeLine>> =
            self.ron_views.iter().map(|_| Vec::new()).collect();
        let mut specs: Vec<frost::TreeSpec> = Vec::new();
        let mut slots: Vec<usize> = Vec::new();
        for (v, buf) in self.ron_views.iter().zip(bufs.iter_mut()) {
            if let Some(spec) = ron_spec(&self.sprites, self.active, v, buf) {
                specs.push(spec);
                slots.push(v.slot);
            }
        }
        let outs = self.ui.tree(&specs, &mut self.ron_state);
        let mut closed = None;
        for (slot, out) in slots.iter().zip(&outs) {
            if let Some(doc) = self.sprites.get_mut(*slot).and_then(|sp| sp.ron.as_mut()) {
                doc.scroll = out.scroll;
                doc.sx = out.sx;
                doc.vw = out.size[0];
                doc.vh = out.size[1];
                doc.clamp_scroll();
            }
            for ev in &out.events {
                match ev {
                    frost::TreeEvent::Close => closed = Some(*slot),
                    frost::TreeEvent::Row { row } => self.row_click(ctx, *slot, *row),
                    frost::TreeEvent::Add { row } => self.row_add(*slot, *row),
                    frost::TreeEvent::Del { row } => self.row_del(ctx, *slot, *row),
                    frost::TreeEvent::DragStart { row } => self.ron_drag_start(*slot, *row),
                    frost::TreeEvent::Drag { row } => self.ron_drag_move(*slot, *row),
                    frost::TreeEvent::DragEnd => self.ron_drag = None,
                }
            }
        }
        if let Some(slot) = closed {
            self.close_slot(ctx, slot);
        }
        // A work-area drag draws the crop selection in the texture's
        // pixel space.
        if down && let (Some(from), Some(p)) = (self.drag_from, pos) {
            self.selection = sel_rect_from(from, p, size, view, self.zoom);
        }
        if released {
            if let Some(from) = self.drag_from.take()
                && let Some([mx, my]) = pos
            {
                let moved = ((mx - from[0]).powi(2) + (my - from[1]).powi(2)).sqrt();
                if moved < CLICK_TOL {
                    // A click on the work area: log the sprite pixel and
                    // clear the selection. The active sprite sits at the
                    // work area's center plus `offset`, at scale `zoom`,
                    // so window coordinates map to texture pixels with
                    // `px = (x - vx) / zoom + w/2` and
                    // `py = h/2 - (y - vy) / zoom` — the y flip included,
                    // since the texture's y grows down.
                    if !self.sprites.is_empty() {
                        let [tw, th] = size;
                        let [px, py] = tex_point([mx, my], size, view, self.zoom);
                        let inside = px >= 0.0 && px <= tw && py >= 0.0 && py <= th;
                        let name = self.active().map_or("", |sp| sp.name.as_str());
                        log::info!(
                            "click at sprite pixel ({px:.1}, {py:.1}) of {tw:.0}x{th:.0} '{name}' — {where}",
                            where = if inside {
                                "inside the texture"
                            } else {
                                "outside the texture"
                            }
                        );
                        self.last_click = Some(([px, py], inside));

                        // A position picked in the sidecar panel: this
                        // click IS the edit — the tree's two numbers
                        // become this pixel, and the rows re-flatten to
                        // show them.
                        let mut say = None;
                        if inside
                            && self.active().is_some_and(|sp| {
                                sp.ron.as_ref().is_some_and(|doc| doc.edit.is_some())
                            })
                        {
                            // Moving or placing a mark: the stroke's
                            // before-picture goes down before the tree
                            // takes the new numbers.
                            self.stamp();
                        }
                        if inside
                            && let Some(sp) = self.sprites.get_mut(self.active)
                            && let Some(doc) = &mut sp.ron
                            && let Some(pi) = doc.edit
                        {
                            let mut spots = Vec::new();
                            scan_spots(&doc.root, &mut Vec::new(), "", &mut spots);
                            if let Some(spot) = spots.get(pi) {
                                let (path, label) = (spot.path.clone(), spot.label.clone());
                                if let Some(v) = ron_tree::walk(&mut doc.root, &path)
                                    && let ron_tree::Val::Struct { fields, .. } = v
                                {
                                    for (n, f) in fields.iter_mut().enumerate().take(2) {
                                        f.1.val = ron_tree::Val::Atom(
                                            format!("{:.1}", [px, py][n]),
                                            ron_tree::Kind::Num,
                                        );
                                    }
                                    doc.rows = ron_tree::layout(&doc.root);
                                    say = Some(format!("moved '{label}' to ({px:.1}, {py:.1})"));
                                }
                            }
                        }
                        if let Some(t) = say {
                            self.status = t;
                        }
                    }
                    self.selection = None;
                }
            }
            // A slot press: a click activates its sprite; a drag that
            // ends on another slot swaps the two sprites' places.
            if let Some((from_slot, from)) = self.slot_drag.take()
                && let Some(p) = pos
            {
                let moved = ((p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2)).sqrt();
                if moved < CLICK_TOL {
                    if from_slot < self.sprites.len() {
                        self.active = from_slot;
                        self.selection = None;
                        self.last_click = None;
                        self.sync_work(ctx);
                    } else {
                        // The spare plate: pressing it opens the
                        // dialog, and the scroll parks at the far end
                        // so the newcomer is in view when it lands.
                        self.open_dialog(ctx);
                        // Park at the far end so the newcomer lands
                        // in view. Not f32::MAX: the thumb strip
                        // divides the scroll this very frame, and a
                        // parked-infinity walked straight off the end
                        // of `usize` — a debug-build panic.
                        self.slot_scroll = sv.scroll_max;
                    }
                } else if from_slot < self.sprites.len()
                    && let Some(to) = slot_at(p, w, h, self.slot_scroll, self.sprites.len() + 1)
                    && to != from_slot
                    && to < self.sprites.len()
                {
                    self.stamp();
                    self.sprites.swap(from_slot, to);
                    self.active = swapped_active(self.active, from_slot, to);
                    self.refresh_slots(ctx);
                }
            }
            // The panel drag ends: a click picks the cell under the
            // cursor — the panel never moved — and a drag keeps the
            // panel where it landed (already clamped by the frame's
            // own geometry).
            if let Some((_, from, _)) = self.band_drag.take()
                && let Some(p) = pos
            {
                let moved = ((p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2)).sqrt();
                if moved < CLICK_TOL {
                    if let Some(i) = band_pick(p, brows, bcols, bcell, bor, bcells) {
                        self.picked_cell = Some(i);
                        self.clip = None;
                        self.sel = None;
                        let ctrl = ctx.key_down(frost::KeyCode::ControlLeft)
                            || ctx.key_down(frost::KeyCode::ControlRight);
                        self.status = if ctrl {
                            self.brush = tfm_flip_x(self.brush);
                            format!("tile {} flipped x — {}", i + 1, turn_note(self.brush))
                        } else {
                            format!("tile {} picked", i + 1)
                        };
                    }
                } else {
                    self.band_origin = Some(bor);
                }
            }
        }

        // A held scrollbar knob rides the cursor; letting go ends it.
        if let Some(grab) = self.slot_scrub
            && let Some(p) = pos
        {
            let kw = scroll_knob(&sv, self.slot_scroll).1;
            let f = ((p[0] - grab) - sv.x0) / (sv.track_w - kw).max(1.0);
            self.slot_scroll = (f * sv.scroll_max).clamp(0.0, sv.scroll_max);
        }
        if !down {
            self.slot_scrub = None;
        }

        // The work area's layer pool: every node carries the pan and the
        // zoom — a uniform scale about its center, then the pan offset
        // from the work area's center. The shapes come from the sync.
        for node in &mut ctx.scene().root.children[..LAYER_NODES] {
            node.transform = frost::Transform::translate(view);
            node.scale = [self.zoom, self.zoom];
        }
        // On the map desk the art stops at the work area: the tile
        // maps' clip is the work rectangle seen through the desk's own
        // transform (scale then pan), so no zoom or pan ever spills a
        // tile over the slots strip or the menu bar.
        if tile_view {
            let clip = [
                (-w / 2.0 - view[0]) / self.zoom,
                (-h / 2.0 + STRIP_H - view[1]) / self.zoom,
                (w / 2.0 - view[0]) / self.zoom,
                (h / 2.0 - view[1]) / self.zoom,
            ];
            for node in &mut ctx.scene().root.children[..LAYER_NODES] {
                if let Some(shape) = &mut node.shape {
                    shape.set_clip(Some(clip));
                }
            }
        }

        // The checker backdrop: the sprite's on-screen bounding box,
        // clipped to the work area, filled with cells that never scale
        // with the sprite — so the pattern reveals the sprite's
        // transparency no matter the zoom or the pan. The fill is a tiny
        // sprite, one texture pixel per cell, sampled with
        // nearest-neighbor filtering so the cell edges stay hard; the
        // texture is rebuilt only when the cell count or a grey level
        // changes, so smooth pans and zooms only reposition and rescale
        // it.
        let [tw, th] = size;
        let (hw, hh) = ((tw * self.zoom) / 2.0, (th * self.zoom) / 2.0);
        let (bx0, by0) = (view[0] - hw, view[1] - hh);
        let (bx1, by1) = (view[0] + hw, view[1] + hh);
        // The box's intersection with the work area — or, on the
        // tile-map desk, the work area entire: the map has no bounds
        // the checker should respect, so the pattern fills the canvas.
        let (rx0, ry0, rx1, ry1) = if tile_view {
            (-w / 2.0, -h / 2.0 + STRIP_H, w / 2.0, h / 2.0)
        } else {
            (
                bx0.max(-w / 2.0),
                by0.max(-h / 2.0 + STRIP_H),
                bx1.min(w / 2.0),
                by1.min(h / 2.0),
            )
        };
        let checker = &mut ctx.scene().root.children[CHECKER];
        if rx1 > rx0 && ry1 > ry0 && (tw > 0.0 || tile_view) {
            let cw = ((rx1 - rx0) / CHECK_CELL).ceil() as u32;
            let ch = ((ry1 - ry0) / CHECK_CELL).ceil() as u32;
            let key = (
                cw,
                ch,
                (self.light * 255.0).round() as u8,
                (self.dark * 255.0).round() as u8,
            );
            if key != self.checker_key {
                self.checker_key = key;
                checker.shape = checker_shape(cw, ch, self.light, self.dark);
            }
            checker.transform = frost::Transform::translate([(rx0 + rx1) / 2.0, (ry0 + ry1) / 2.0]);
            // One texture pixel per cell; the scale stretches the
            // sub-pixel last row and column so the pattern covers the
            // region exactly.
            checker.scale = [(rx1 - rx0) / cw as f32, (ry1 - ry0) / ch as f32];
        } else {
            // No sprite (or its box off the work area): hide the backdrop
            // and force a rebuild when it comes back.
            checker.shape = None;
            self.checker_key = (0, 0, 0, 0);
        }

        // The desk's own lines stop at the work area's edge: no
        // bounding box, atlas grid or crop selection escapes onto the
        // slots strip or past the top of the window.
        let (wx0, wy0, wx1, wy1) = (-w / 2.0, -h / 2.0 + STRIP_H, w / 2.0, h / 2.0);
        let clip_v = |x: f32, y0: f32, y1: f32| -> Option<(f32, f32)> {
            let (a, b) = (y0.max(wy0), y1.min(wy1));
            (x >= wx0 && x <= wx1 && b > a).then_some((a, b))
        };
        let clip_h = |y: f32, x0: f32, x1: f32| -> Option<(f32, f32)> {
            let (a, b) = (x0.max(wx0), x1.min(wx1));
            (y >= wy0 && y <= wy1 && b > a).then_some((a, b))
        };
        // The bounding box: the sprite's full on-screen rectangle,
        // cut to the desk.
        if tw > 0.0 && th > 0.0 && !tile_view {
            if let Some((a, b)) = clip_h(by0, bx0, bx1) {
                ctx.line(a, by0, b, by0, BBOX, BBOX_WIDTH, BBOX_Z);
            }
            if let Some((a, b)) = clip_v(bx1, by0, by1) {
                ctx.line(bx1, a, bx1, b, BBOX, BBOX_WIDTH, BBOX_Z);
            }
            if let Some((a, b)) = clip_h(by1, bx0, bx1) {
                ctx.line(a, by1, b, by1, BBOX, BBOX_WIDTH, BBOX_Z);
            }
            if let Some((a, b)) = clip_v(bx0, by0, by1) {
                ctx.line(bx0, a, bx0, b, BBOX, BBOX_WIDTH, BBOX_Z);
            }
        }

        // The crop selection: the texture-space rectangle mapped back to
        // the window — the same transform the marker dot uses.
        if let Some([x0, y0, x1, y1]) = self.selection.filter(|_| !tile_view) {
            let wx = |px: f32| (px - tw / 2.0) * self.zoom + view[0];
            let wy = |py: f32| (th / 2.0 - py) * self.zoom + view[1];
            let (sx0, sx1) = (wx(x0), wx(x1));
            let (sy0, sy1) = (wy(y0), wy(y1));
            if let Some((a, b)) = clip_h(sy0, sx0, sx1) {
                ctx.line(a, sy0, b, sy0, SELECT, BBOX_WIDTH, SELECT_Z);
            }
            if let Some((a, b)) = clip_v(sx1, sy0, sy1) {
                ctx.line(sx1, a, sx1, b, SELECT, BBOX_WIDTH, SELECT_Z);
            }
            if let Some((a, b)) = clip_h(sy1, sx0, sx1) {
                ctx.line(a, sy1, b, sy1, SELECT, BBOX_WIDTH, SELECT_Z);
            }
            if let Some((a, b)) = clip_v(sx0, sy0, sy1) {
                ctx.line(sx0, a, sx0, b, SELECT, BBOX_WIDTH, SELECT_Z);
            }
        }

        // The tile grid: the atlas' rows and columns mapped onto the
        // sprite, the split drawn over the texture. The inner lines
        // only — the outer ones are the bounding box.
        if let Some((rows, cols)) = self.active().and_then(|sp| sp.atlas)
            && !tile_view
            && tw > 0.0
            && th > 0.0
            && (rows > 1 || cols > 1)
        {
            let wx = |px: f32| (px - tw / 2.0) * self.zoom + view[0];
            let wy = |py: f32| (th / 2.0 - py) * self.zoom + view[1];
            for i in 1..cols {
                let x = wx(tw * i as f32 / cols as f32);
                if let Some((a, b)) = clip_v(x, by0, by1) {
                    ctx.line(x, a, x, b, TILE, TILE_WIDTH, TILE_Z);
                }
            }
            for j in 1..rows {
                let y = wy(th * j as f32 / rows as f32);
                if let Some((a, b)) = clip_h(y, bx0, bx1) {
                    ctx.line(a, y, b, y, TILE, TILE_WIDTH, TILE_Z);
                }
            }
        }

        // The tile-map grid: the active tileset's cell pitch extended
        // across the whole work area, the map axes drawn louder. The
        // drawn step doubles while a cell shrinks below GRID_MIN_PX on
        // screen, so the grid thins by powers of two — always on whole
        // cells, always a few dozen lines, and the tile coordinates
        // stay legible off any surviving line. With no tileset loaded
        // the pitch falls back to the demo tile, so the desk is never
        // a blank void.
        if tile_view {
            let (tx, ty) = self
                .active()
                .map(|sp| {
                    let (rows, cols) = sp.atlas.unwrap_or((1, 1));
                    (
                        sp.current.width() as f32 / cols.max(1) as f32,
                        sp.current.height() as f32 / rows.max(1) as f32,
                    )
                })
                .unwrap_or((32.0, 32.0));
            let (sx, sy) = (grid_step(tx, self.zoom), grid_step(ty, self.zoom));
            let (x0w, y0w, x1w, y1w) = (-w / 2.0, -h / 2.0 + STRIP_H, w / 2.0, h / 2.0);
            // The work area's corners in map space; the window's y-up
            // and the map's y-up agree, so one sign serves both axes.
            let (mx0, mx1) = ((x0w - view[0]) / self.zoom, (x1w - view[0]) / self.zoom);
            let (my0, my1) = ((y0w - view[1]) / self.zoom, (y1w - view[1]) / self.zoom);
            let wx = |x: f32| x * self.zoom + view[0];
            let wy = |y: f32| y * self.zoom + view[1];
            let cols_at = |n: i64| if n == 0 { AXIS } else { TILE };
            let wid_at = |n: i64| if n == 0 { BBOX_WIDTH } else { TILE_WIDTH };
            let z_at = |n: i64| if n == 0 { AXIS_Z } else { GRID_Z };
            let nx = |v: f32, s: f32| (v / s).floor() as i64;
            let nxe = |v: f32, s: f32| (v / s).ceil() as i64;
            let mut n = nx(mx0, sx);
            while n <= nxe(mx1, sx) {
                let x = wx(n as f32 * sx);
                ctx.line(x, y0w, x, y1w, cols_at(n), wid_at(n), z_at(n));
                n += 1;
            }
            let mut n = nx(my0, sy);
            while n <= nxe(my1, sy) {
                let y = wy(n as f32 * sy);
                ctx.line(x0w, y, x1w, y, cols_at(n), wid_at(n), z_at(n));
                n += 1;
            }
        }
        // The selection's frame: the held rectangle outlined in the
        // picker's blue, cut to the work area like every desk line.
        if tile_view && let Some([sc0, sr0, sc1, sr1]) = self.sel {
            let (sx, sy) = self
                .active()
                .map(|sp| {
                    let (arows, acols) = sp.atlas.unwrap_or((1, 1));
                    (
                        sp.current.width() as f32 / acols.max(1) as f32,
                        sp.current.height() as f32 / arows.max(1) as f32,
                    )
                })
                .unwrap_or((32.0, 32.0));
            let (fx0, fx1) = (
                sc0 as f32 * sx * self.zoom + view[0],
                (sc1 + 1) as f32 * sx * self.zoom + view[0],
            );
            let (fy0, fy1) = (
                -(sr1 + 1) as f32 * sy * self.zoom + view[1],
                -sr0 as f32 * sy * self.zoom + view[1],
            );
            if let Some((a, b)) = clip_h(fy0, fx0, fx1) {
                ctx.line(a, fy0, b, fy0, SELECT, BBOX_WIDTH, SELECT_Z);
            }
            if let Some((a, b)) = clip_h(fy1, fx0, fx1) {
                ctx.line(a, fy1, b, fy1, SELECT, BBOX_WIDTH, SELECT_Z);
            }
            if let Some((a, b)) = clip_v(fx0, fy0, fy1) {
                ctx.line(fx0, a, fx0, b, SELECT, BBOX_WIDTH, SELECT_Z);
            }
            if let Some((a, b)) = clip_v(fx1, fy0, fy1) {
                ctx.line(fx1, a, fx1, b, SELECT, BBOX_WIDTH, SELECT_Z);
            }
        }

        // The slots strip: its floor, one plate per sprite plus the
        // spare loader plate, the active slot's frame — and, when the
        // plates outrun the pool, a scrollbar along the strip's bottom
        // edge.
        ctx.rectangle(
            0.0,
            -h / 2.0 + STRIP_H / 2.0,
            w / 2.0,
            STRIP_H / 2.0,
            STRIP,
            STRIP_Z,
        );
        for i in 0..slot_count {
            let [cx, cy] = slot_center(i, w, h, self.slot_scroll);
            if cx + SLOT / 2.0 < sv.x0 || cx - SLOT / 2.0 > sv.x0 + sv.visible_px {
                continue;
            }
            let plate = if i < self.sprites.len() {
                PLATE
            } else {
                PLATE_EMPTY
            };
            ctx.rectangle(cx, cy, SLOT / 2.0, SLOT / 2.0, plate, PLATE_Z);
            if i == self.sprites.len() {
                // The spare plate wears a plus: press it for the open
                // dialog.
                let g = SLOT / 5.0;
                ctx.line(cx - g, cy, cx + g, cy, AXIS, 2.0, PLATE_Z + 0.1);
                ctx.line(cx, cy - g, cx, cy + g, AXIS, 2.0, PLATE_Z + 0.1);
            }
            if i == self.active && i < self.sprites.len() {
                let r = SLOT / 2.0;
                ctx.line(cx - r, cy - r, cx + r, cy - r, SELECT, BBOX_WIDTH, FRAME_Z);
                ctx.line(cx + r, cy - r, cx + r, cy + r, SELECT, BBOX_WIDTH, FRAME_Z);
                ctx.line(cx + r, cy + r, cx - r, cy + r, SELECT, BBOX_WIDTH, FRAME_Z);
                ctx.line(cx - r, cy + r, cx - r, cy - r, SELECT, BBOX_WIDTH, FRAME_Z);
            }
        }
        if sv.scroll_max > 0.0 {
            let ty = -h / 2.0 + SCROLL_Y;
            let (kx, kw) = scroll_knob(&sv, self.slot_scroll);
            ctx.rectangle(
                sv.x0 + sv.track_w / 2.0,
                ty,
                sv.track_w / 2.0,
                SCROLL_H / 2.0,
                PLATE_EMPTY,
                PLATE_Z + 0.05,
            );
            ctx.rectangle(
                kx + kw / 2.0,
                ty,
                kw / 2.0,
                SCROLL_H / 2.0,
                AXIS,
                PLATE_Z + 0.1,
            );
        }

        // The slot thumbnails: minimized originals, centered on their
        // plates — the pool maps the scrolled window, so scrolling
        // moves which sprite rides which node. A dragged thumbnail
        // rides the cursor, above everything until it lands.
        {
            let first = (self.slot_scroll / sv.pitch).floor().max(0.0) as usize;
            for j in 0..SLOTS {
                let node = &mut ctx.scene().root.children[THUMBS + j];
                // Saturating by stubbornness: the scroll is clamped
                // upstream, and an index that runs past the strip
                // should read as nothing, never as a panic.
                let i = first.saturating_add(j);
                if let Some(sp) = self.sprites.get(i) {
                    node.shape = Some(sp.thumb.clone());
                    let dragging = self.slot_drag.is_some_and(|(slot, _)| slot == i);
                    node.order = if dragging { DRAG_ORDER } else { THUMB_ORDER };
                    node.transform = match (dragging, pos) {
                        (true, Some(p)) => frost::Transform::translate(p),
                        _ => frost::Transform::translate(slot_center(i, w, h, self.slot_scroll)),
                    };
                } else {
                    node.shape = None;
                }
            }
        }

        // --- The tileset panel (tile desk only) -------------------------
        // The active tileset's grid, drawn as its matrix: the brush
        // rack, and a panel — drag it anywhere on the desk, and the
        // wheel over it scales its cells alone. The picked cell wears
        // the selection frame, and the cursor answers back — the cell
        // under it shows a translucent ghost of the picked tile, the
        // same one-tile `TileMap` shapes the rack draws, scaled into
        // the map and placed where a click would land.
        {
            let rects = &brects;
            let cells = bcells;
            let cell = bcell;
            if tile_view && !self.cell_shapes.is_empty() {
                // The plate: one quiet board behind the whole rack,
                // hugging the matrix as drawn.
                if let Some([x0, y0, x1, y1]) = bplate {
                    ctx.rectangle(
                        (x0 + x1) / 2.0,
                        (y0 + y1) / 2.0,
                        (x1 - x0) / 2.0,
                        (y1 - y0) / 2.0,
                        PLATE,
                        PLATE_Z,
                    );
                }
                // The pool keeps its full size; the rack's cell
                // decides what shows. Shapes are built for the
                // standard box, so a shrunk rack scales them.
                for node in &mut ctx.scene().root.children[BAND0..BAND0 + BAND_CELLS] {
                    node.shape = None;
                }
                // The shapes are built for the standard box; the
                // panel's zoom scales them with it, up or down.
                let k2 = cell / BAND_CELL;
                for (i, rect) in rects.iter().take(cells).enumerate() {
                    let node = &mut ctx.scene().root.children[BAND0 + i];
                    node.shape = self.cell_shapes.get(i).cloned();
                    node.scale = [k2, k2];
                    node.transform = frost::Transform::translate([
                        (rect[0] + rect[2]) / 2.0,
                        (rect[1] + rect[3]) / 2.0,
                    ]);
                    node.order = BAND_Z;
                }
                // The held chip: the picked cell's art wearing the
                // brush's transform — the right-click's verdict,
                // visible without leaving the panel.
                let chip_data = match (
                    self.band_base.clone(),
                    self.picked_cell.filter(|c| *c < cells),
                    bplate,
                ) {
                    (Some(base), Some(c), Some([_, _, x1, y1])) if self.clip.is_none() => {
                        let [tw, th] = self.active().map_or([1.0, 1.0], |sp| {
                            [
                                sp.current.width() as f32 / bcols.max(1) as f32,
                                sp.current.height() as f32 / brows.max(1) as f32,
                            ]
                        });
                        let k = 0.8 * k2 * ((BAND_CELL / tw).min(BAND_CELL / th));
                        let (dw, dh) = (tw * k, th * k);
                        let b = self.brush;
                        let mut shape = base;
                        shape.set_tiles(vec![
                            frost::Tile::new([0.0, 0.0], [dw, dh], cell_uv(c, brows, bcols))
                                .transformed((b >> 2) & 3, b & 1 != 0, b & 2 != 0),
                        ]);
                        Some((shape, [x1 + 8.0 + dw / 2.0, y1 - 8.0 - dh / 2.0], dw, dh))
                    }
                    _ => None,
                };
                // Beside the plate's upper-right, on its own quiet
                // board — never over a cell's art.
                if let Some((shape, at, dw, dh)) = chip_data {
                    ctx.rectangle(at[0], at[1], dw / 2.0 + 5.0, dh / 2.0 + 5.0, PLATE, PLATE_Z);
                    let chip = &mut ctx.scene().root.children[HELD];
                    chip.shape = Some(shape);
                    chip.transform = frost::Transform::translate(at);
                    chip.order = BAND_Z + 0.2;
                } else {
                    ctx.scene().root.children[HELD].shape = None;
                }
                if let Some(picked) = self.picked_cell.filter(|c| *c < cells) {
                    let [x0, y0, x1, y1] = rects[picked];
                    ctx.line(x0, y0, x1, y0, SELECT, BBOX_WIDTH, BAND_Z + 0.1);
                    ctx.line(x1, y0, x1, y1, SELECT, BBOX_WIDTH, BAND_Z + 0.1);
                    ctx.line(x1, y1, x0, y1, SELECT, BBOX_WIDTH, BAND_Z + 0.1);
                    ctx.line(x0, y1, x0, y0, SELECT, BBOX_WIDTH, BAND_Z + 0.1);
                }
                // The ghost: the picked tile where the next click
                // would place it. Off the canvas, in a panel or over
                // the rack itself, it withdraws.
                let over_band = rects
                    .iter()
                    .take(cells)
                    .any(|[x0, y0, x1, y1]| p_in(pos, [*x0, *y0, *x1, *y1]));
                // A held block ghosts as a block: the whole picture
                // gathers on the cursor's cell, dressed as copied.
                // Only the active tileset ghosts — one shape, one
                // texture; a block of another tileset still pastes.
                let block_ghost =
                    match (&self.clip, &self.band_base, pos) {
                        (Some(clip), Some(base), Some(p))
                            if tile_view
                                && !over_band
                                && !self.ui.hovering()
                                && in_work_area(p, w, h)
                                && self.active().is_some_and(|sp| sp.path == clip.tileset) =>
                        {
                            let (rows, cols) =
                                self.active().and_then(|sp| sp.atlas).unwrap_or((1, 1));
                            let (tw, th) = (
                                self.active().map_or(1.0, |sp| sp.current.width() as f32)
                                    / cols.max(1) as f32,
                                self.active().map_or(1.0, |sp| sp.current.height() as f32)
                                    / rows.max(1) as f32,
                            );
                            let (col, row) = grid_cell(
                                (p[0] - view[0]) / self.zoom,
                                (p[1] - view[1]) / self.zoom,
                                tw,
                                th,
                            );
                            let (bc, br) = paint_bounds(&self.maps, &clip.tileset);
                            {
                                // Preview the drop the desk will
                                // perform: exactly the cells the drop
                                // writes — the paste's own list,
                                // intersected with the map. Where the
                                // block hangs past an edge, the
                                // preview shows the part that lands,
                                // no more and no less.
                                let (oc, orr) = (col, row);
                                let mut shape = base.clone();
                                let tiles: Vec<frost::Tile> =
                                    ghost_cells(clip, col, row, bc, br)
                                        .into_iter()
                                        .map(|(c, r, cell, tf)| {
                                            frost::Tile::new(
                                                [(c - oc) as f32 * tw, -((r - orr) as f32) * th],
                                                [tw, th],
                                                cell_uv(cell as usize, rows, cols),
                                            )
                                            .transformed((tf >> 2) & 3, tf & 1 != 0, tf & 2 != 0)
                                        })
                                        .collect();
                                shape.set_tiles(tiles);
                                if let frost::Shape::TileMap { color, .. } = &mut shape {
                                    color.a = GHOST_A;
                                }
                                let center = [
                                    (oc as f32 + 0.5) * tw * self.zoom + view[0],
                                    -(orr as f32 + 0.5) * th * self.zoom + view[1],
                                ];
                                let node = &mut ctx.scene().root.children[GHOST];
                                node.transform = ghost_transform(self.zoom, center);
                                node.order = GHOST_Z;
                                Some(shape)
                            }
                        }
                        _ => None,
                    };
                let ghost = block_ghost.or_else(|| {
                    match (
                        &self.band_base,
                        self.picked_cell.filter(|c| *c < self.cell_shapes.len()),
                        pos,
                        tile_view
                            && in_work_area(pos.unwrap_or([0.0, -1e9]), w, h)
                            && !self.ui.hovering()
                            && !over_band,
                    ) {
                        (Some(base), Some(cell), Some(p), true) => {
                            let (rows, cols) =
                                self.active().and_then(|sp| sp.atlas).unwrap_or((1, 1));
                            let (tw, th) = (
                                self.active().map_or(1.0, |sp| sp.current.width() as f32)
                                    / cols.max(1) as f32,
                                self.active().map_or(1.0, |sp| sp.current.height() as f32)
                                    / rows.max(1) as f32,
                            );
                            // The cell the cursor stands on, in map space:
                            // columns to the right, rows downward (the
                            // grid's y runs down, the window's runs up).
                            let mx = (p[0] - view[0]) / self.zoom;
                            let my = (p[1] - view[1]) / self.zoom;
                            let (col, row) = grid_cell(mx, my, tw, th);
                            // The brush only reaches the map — the layer
                            // this tileset owns, or the map a first click
                            // would create. Past its edge, no ghost.
                            let (bc, br) = self
                                .active()
                                .map_or((0, 0), |sp| paint_bounds(&self.maps, &sp.path));
                            if cell_in(col, row, bc, br) {
                                let center = [
                                    (col as f32 + 0.5) * tw * self.zoom + view[0],
                                    -(row as f32 + 0.5) * th * self.zoom + view[1],
                                ];
                                let mut shape = base.clone();
                                let b = self.brush;
                                shape.set_tiles(vec![
                                    frost::Tile::new(
                                        [0.0, 0.0],
                                        [tw, th],
                                        cell_uv(cell, rows, cols),
                                    )
                                    .transformed(
                                        (b >> 2) & 3,
                                        b & 1 != 0,
                                        b & 2 != 0,
                                    ),
                                ]);
                                if let frost::Shape::TileMap { color, .. } = &mut shape {
                                    color.a = GHOST_A;
                                }
                                let node = &mut ctx.scene().root.children[GHOST];
                                node.transform = ghost_transform(self.zoom, center);
                                node.order = GHOST_Z;
                                Some(shape)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }
                });
                ctx.scene().root.children[GHOST].shape = ghost;
            } else {
                for node in &mut ctx.scene().root.children[BAND0..=HELD] {
                    node.shape = None;
                }
            }
        }

        // --- The sidecar views ------------------------------------------
        // Every open file's tree is one widget frame's paint: rows,
        // bands, list buttons, scroll thumbs, close boxes and corner
        // grips, in the shared UI font, at RON_ORDER over the UI's
        // plates. A folded view's tree paints nothing but its close
        // box.
        let mut spots: Vec<Spot> = Vec::new();
        if let Some(doc) = self.ron() {
            scan_spots(&doc.root, &mut Vec::new(), "", &mut spots);
        }
        let editing = self.ron().and_then(|d| d.edit);
        {
            let mut bufs: Vec<Vec<frost::TreeLine>> =
                self.ron_views.iter().map(|_| Vec::new()).collect();
            let specs: Vec<frost::TreeSpec> = self
                .ron_views
                .iter()
                .zip(bufs.iter_mut())
                .filter_map(|(v, buf)| ron_spec(&self.sprites, self.active, v, buf))
                .collect();
            for spec in &specs {
                self.ui.tree_paint(ctx, spec);
            }
        }

        // --- The position markers ---------------------------------------
        // Every spot the ACTIVE sprite's sidecar names, drawn at its
        // pixel coordinate in the row's palette colour; the picked one
        // swells. Chrome: fixed size in window pixels, so the markers
        // stay readable at any zoom.
        let mut marks = 0usize;
        let mut numbered = 0usize;
        if !tile_view
            && self
                .sprites
                .get(self.active)
                .is_some_and(|sp| sp.ron.is_some())
        {
            let [tw, th] = size;
            for (pi, s) in spots.iter().enumerate() {
                let [wx, wy] = [
                    (s.x - tw / 2.0) * self.zoom + view[0],
                    (th / 2.0 - s.y) * self.zoom + view[1],
                ];
                let (r0, r1, a) = if editing == Some(pi) {
                    (11.0, 4.2, 1.0)
                } else {
                    (6.5, 2.4, 0.85)
                };
                let (pr, pg, pb) = PALETTE[pi % PALETTE.len()];
                let nodes = &mut ctx.scene().root.children;
                nodes[SPOTS + marks].shape = Some(frost::Shape::Circle {
                    center: [0.0, 0.0],
                    radius: r0,
                    color: frost::Color {
                        r: pr,
                        g: pg,
                        b: pb,
                        a,
                    },
                });
                nodes[SPOTS + marks].transform = frost::Transform::translate([wx, wy]);
                marks += 1;
                nodes[SPOTS + marks].shape = Some(frost::Shape::Circle {
                    center: [0.0, 0.0],
                    radius: r1,
                    color: frost::Color {
                        r: 0.95,
                        g: 0.95,
                        b: 0.95,
                        a,
                    },
                });
                nodes[SPOTS + marks].transform = frost::Transform::translate([wx, wy]);
                marks += 1;
                // An entry of a list wears its seat number beside the
                // marker — the same index its `[n]:` row shows, and it
                // follows the entry through every reorder.
                if let Some(i) = s.idx {
                    nodes[SPOTT + numbered].shape = Some(self.ron_text(
                        format!("{i}"),
                        SEAT_SIZE,
                        // 700 is this FiraCode's weight axis ceiling.
                        700.0,
                        frost::Color {
                            r: 0.93,
                            g: 0.94,
                            b: 0.97,
                            a: 0.95,
                        },
                    ));
                    nodes[SPOTT + numbered].transform =
                        frost::Transform::translate([wx + 13.0, wy + 13.0 - RON_LIFT * SEAT_SIZE]);
                    numbered += 1;
                }
            }
        }
        let nodes = &mut ctx.scene().root.children;
        for node in &mut nodes[SPOTS + marks..SPOTS + 2 * SPOTS_MAX] {
            node.shape = None;
        }
        for node in &mut nodes[SPOTT + numbered..SPOTT + SPOTS_MAX] {
            node.shape = None;
        }

        // The usage line, pinned near the top edge, so resizing keeps it
        // in place.
        let help = &mut ctx.scene().root.children[HELP];
        help.transform = frost::Transform::translate([0.0, h / 2.0 - 28.0]);

        // The marker dot rides the last click in window space — the
        // inverse of the conversion above — and shrinks away when there
        // is none.
        let marker = &mut ctx.scene().root.children[MARKER_NODE];
        if let Some(([px, py], _)) = self.last_click {
            let [tw, th] = size;
            marker.transform = frost::Transform::translate([
                (px - tw / 2.0) * self.zoom + view[0],
                (th / 2.0 - py) * self.zoom + view[1],
            ]);
            if let Some(frost::Shape::Circle { radius, .. }) = &mut marker.shape {
                *radius = 4.0;
            }
        } else if let Some(frost::Shape::Circle { radius, .. }) = &mut marker.shape {
            *radius = 0.0;
        }

        // The exit box, drawn last of all: it stands over the whole
        // frame — the veil dims the world, the question waits at the
        // centre, two answers beneath.
        if self.quit_prompt {
            match self
                .ui
                .confirm(ctx, [w, h], "Leave the editor?", "Yes, quit", "No, stay")
            {
                Some(frost::Answer::Yes) => {
                    self.quit_prompt = false;
                    ctx.exit();
                }
                Some(frost::Answer::No) => {
                    self.quit_prompt = false;
                    self.status = String::from("staying");
                }
                None => {}
            }
        }

        // The cursor's position for next frame's middle-drag delta; `None`
        // (outside the window) clears it, so re-entering never jumps.
        self.last_mouse = pos;
    }
}

/// Read a PNG from disk into a sprite: its working texture, its work-area
/// shape and its slot thumbnail (the original, minimized once so the GPU
/// never minifies the full texture on its own).
fn read_sprite(path: &std::path::Path) -> Result<Sprite, String> {
    let name = file_name_of(path);
    if path.extension().and_then(|e| e.to_str()) != Some("png") {
        return Err(format!("'{name}' is not a PNG file"));
    }
    let shape =
        frost::Shape::sprite_nearest(path).map_err(|e| format!("failed to load '{name}': {e}"))?;
    let tex = image::open(path)
        .map(|img| img.to_rgba8())
        .map_err(|e| format!("failed to decode '{name}': {e}"))?;
    let (iw, ih) = (tex.width(), tex.height());
    log::info!("loaded '{name}': {iw}x{ih} pixels");
    let k = THUMB_MAX / (iw.max(ih) as f32);
    let (tw, th) = (
        ((iw as f32) * k).round().max(1.0) as u32,
        ((ih as f32) * k).round().max(1.0) as u32,
    );
    let small = image::imageops::resize(&tex, tw, th, image::imageops::FilterType::Lanczos3);
    let thumb = frost::Shape::sprite_bytes_nearest(&png_bytes(&small)?)
        .map_err(|e| format!("failed to build the thumbnail: {e}"))?;
    let ron = load_ron(path);
    // The grid the sidecar named, if it named one — the sprite reloads
    // tiled the way it was saved.
    let atlas = ron.as_ref().and_then(|doc| atlas_of(&doc.root));
    if let Some((rows, cols)) = atlas {
        log::info!("atlas for '{name}': {rows} x {cols}");
    }
    // Both baselines start as the disk: the decode is the texture's
    // saved state, and the sidecar's canonical rendering is the tree's —
    // the exact text a save would write, so a fresh sprite opens clean.
    let saved_ron = ron
        .as_ref()
        .map(|doc| ron_tree::to_text_doc(&doc.header, &doc.root, &doc.trailer));
    Ok(Sprite {
        path: path.to_path_buf(),
        name,
        current: tex.clone(),
        saved_img: tex,
        shape,
        thumb,
        thumb_img: small,
        ron,
        saved_ron,
        atlas,
    })
}

/// The sprite's sidecar: `<name>.ron` beside `<name>.png`. Absent is the
/// normal case; unparseable is logged and skipped. Either way the sprite
/// itself has already loaded and loads unchanged.
fn load_ron(png: &std::path::Path) -> Option<RonDoc> {
    let side = png.with_extension("ron");
    let Ok(text) = std::fs::read_to_string(&side) else {
        return None;
    };
    let doc = match ron_tree::parse_doc(&text) {
        Ok(d) => d,
        Err(err) => {
            log::warn!("sidecar '{}': {err}", file_name_of(&side));
            return None;
        }
    };
    let ron_tree::Doc {
        header,
        trailer,
        mut root,
    } = doc;
    // The tree opens its first two levels; everything deeper waits.
    root.open_to(0, 1);
    let name = file_name_of(&side);
    log::info!("sidecar '{name}' loaded for '{}'", file_name_of(png));
    let rows = ron_tree::layout(&root);
    Some(RonDoc {
        name,
        header,
        trailer,
        root,
        rows,
        scroll: 0.0,
        sx: 0.0,
        edit: None,
        vw: RON_W,
        vh: RON_VIEW_H,
    })
}

/// The sidecar a sprite gains on the first save that names a grid: a
/// fresh document holding just the `atlas` field, laid out and ready
/// for the panel — the shape [`load_ron`] builds for an existing file.
fn new_sidecar(name: String, atlas: (usize, usize)) -> RonDoc {
    let mut root = ron_tree::Val::Struct {
        open: true,
        head: String::new(),
        curly: false,
        fields: vec![(
            "atlas".to_string(),
            ron_tree::Item::plain(atlas_value(atlas.0, atlas.1)),
        )],
        tail: String::new(),
    };
    root.open_to(0, 1);
    let rows = ron_tree::layout(&root);
    let mut doc = RonDoc {
        name,
        header: String::new(),
        trailer: String::new(),
        root,
        rows,
        scroll: 0.0,
        sx: 0.0,
        edit: None,
        vw: RON_W,
        vh: RON_VIEW_H,
    };
    doc.clamp_scroll();
    doc
}

/// The `atlas` field's value: the counts as a two-entry array,
/// `[rows, cols]`.
fn atlas_value(rows: usize, cols: usize) -> ron_tree::Val {
    ron_tree::Val::Seq {
        open: true,
        items: vec![
            ron_tree::Item::plain(ron_tree::Val::Atom(rows.to_string(), ron_tree::Kind::Num)),
            ron_tree::Item::plain(ron_tree::Val::Atom(cols.to_string(), ron_tree::Kind::Num)),
        ],
        tail: String::new(),
    }
}

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

/// The tile grid written into the sidecar's root: its `atlas` field
/// becomes `[rows, cols]`, or is dropped when the grid is off. An
/// array, not a tuple — a two-number tuple would read as a position
/// spot, and the grid's counts are not pixels. Whether the tree
/// changed.
fn set_atlas(root: &mut ron_tree::Val, atlas: Option<(usize, usize)>) -> bool {
    let ron_tree::Val::Struct { fields, .. } = root else {
        return false;
    };
    match atlas {
        Some((rows, cols)) => {
            let seq = atlas_value(rows, cols);
            match fields.iter_mut().find(|(key, _)| *key == "atlas") {
                Some((_, item)) => item.val = seq,
                None => fields.push(("atlas".to_string(), ron_tree::Item::plain(seq))),
            }
            true
        }
        None => {
            let before = fields.len();
            fields.retain(|(key, _)| *key != "atlas");
            fields.len() != before
        }
    }
}

/// Where a sidecar panel first opens: at the work area's lower right,
/// clear of the slot strip — later views cascade up and left from
/// there. `row_h` and `pad` are the UI style's, and the panel's height
/// is its title bar, twice the padding and the tree viewport.
fn ron_default(w: f32, h: f32, row_h: f32, pad: f32, cascade: usize) -> [f32; 2] {
    let panel_h = row_h + 2.0 * pad + RON_VIEW_H;
    let step = cascade as f32 * 34.0;
    [
        w / 2.0 - 20.0 - RON_W / 2.0 - step,
        -h / 2.0 + STRIP_H + 12.0 + panel_h / 2.0 + step,
    ]
}

/// Whether a window point is inside a rectangle.
fn in_rect(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3]
}

/// The panel's value-side color: container heads amber, closing
/// brackets muted, atoms by kind — `ron_view`'s palette on the dark
/// plate.
fn ron_color(k: ron_tree::VKind) -> frost::Color {
    let c = |r, g, b| frost::Color { r, g, b, a: 1.0 };
    match k {
        // The closing bracket is its opening one's twin: same amber,
        // and bold like any head (see the line's `head` flag).
        ron_tree::VKind::Head | ron_tree::VKind::Close => c(0.93, 0.78, 0.44),
        ron_tree::VKind::Atom(ron_tree::Kind::Str) => c(0.72, 0.86, 0.66),
        ron_tree::VKind::Atom(ron_tree::Kind::Char) => c(0.66, 0.80, 0.78),
        ron_tree::VKind::Atom(ron_tree::Kind::Num) => c(0.60, 0.78, 0.96),
        ron_tree::VKind::Atom(ron_tree::Kind::Bool) => c(0.83, 0.68, 0.95),
        ron_tree::VKind::Atom(ron_tree::Kind::Path) => c(0.64, 0.86, 0.84),
    }
}

/// A position the sidecar names: its fold-path in the tree (the very
/// paths `walk` speaks, so marker and rows share an address), its pixel
/// coordinates, and the label the status line shows. The scan index is
/// the palette slot.
struct Spot {
    path: Vec<u16>,
    x: f32,
    y: f32,
    label: String,
    /// The entry number when the spot lives directly in a sequence —
    /// the sprite view prints it beside the marker.
    idx: Option<usize>,
}

/// Every position in the tree: a struct whose exactly two fields are
/// both numeric atoms — the plant files' `(x, y)` anchors and flowers,
/// in any nesting. Sequence children label as `parent[n]`, fields by
/// name; positions never nest deeper (their tuples are leaves).
fn scan_spots(v: &ron_tree::Val, path: &mut Vec<u16>, label: &str, out: &mut Vec<Spot>) {
    if out.len() >= SPOTS_MAX {
        return;
    }
    let in_seq = matches!(v, ron_tree::Val::Seq { .. });
    for (n, (key, kid)) in v.kids().iter().enumerate() {
        path.push(n as u16);
        let here = if key.is_empty() {
            format!("{label}[{n}]")
        } else {
            key.trim_end_matches([':', ' ']).to_string()
        };
        if let Some((x, y)) = spot_coords(kid) {
            out.push(Spot {
                path: path.clone(),
                x,
                y,
                label: here,
                idx: in_seq.then_some(n),
            });
        } else {
            scan_spots(kid, path, &here, out);
        }
        path.pop();
        if out.len() >= SPOTS_MAX {
            return;
        }
    }
}

/// The coordinates when a value is a position: two fields, both numeric
/// atoms that read back as floats.
fn spot_coords(v: &ron_tree::Val) -> Option<(f32, f32)> {
    let ron_tree::Val::Struct { fields, .. } = v else {
        return None;
    };
    if fields.len() != 2 {
        return None;
    }
    let nums: Vec<f32> = fields
        .iter()
        .filter_map(|(_, it)| match &it.val {
            ron_tree::Val::Atom(t, ron_tree::Kind::Num) => t.parse().ok(),
            _ => None,
        })
        .collect();
    (nums.len() == 2).then(|| (nums[0], nums[1]))
}

/// The spot a tree row describes: the row IS the position (its head or
/// close row), or it is one of the position's two coordinate rows.
fn spot_of_row(spots: &[Spot], row: &[u16]) -> Option<usize> {
    spots.iter().position(|s| {
        row == s.path.as_slice()
            || (row.len() == s.path.len() + 1
                && row[..s.path.len()] == s.path[..]
                && row[s.path.len()] < 2)
    })
}

/// A position's two numbers written back as one-decimal atoms, the way a
/// click on the sprite writes them.
fn write_spot(v: &mut ron_tree::Val, x: f32, y: f32) {
    if let ron_tree::Val::Struct { fields, .. } = v {
        for (n, f) in fields.iter_mut().enumerate().take(2) {
            f.1.val = ron_tree::Val::Atom(format!("{:.1}", [x, y][n]), ron_tree::Kind::Num);
        }
    }
}

/// A crop's job for a sidecar: every position moves by the cut's origin
/// `(dx, dy)`, clamped into the cropped texture's bounds — so a point
/// keeps its place relative to the pixels that survive, and a point the
/// cut eats lands on the new edge. The number of positions touched.
fn shift_spots(root: &mut ron_tree::Val, dx: f32, dy: f32, bounds: (u32, u32)) -> usize {
    let mut spots = Vec::new();
    scan_spots(root, &mut Vec::new(), "", &mut spots);
    let mut n = 0;
    for spot in &spots {
        let (cx, cy) = (
            (spot.x - dx).clamp(0.0, bounds.0 as f32),
            (spot.y - dy).clamp(0.0, bounds.1 as f32),
        );
        if let Some(v) = ron_tree::walk(root, &spot.path) {
            write_spot(v, cx, cy);
            n += 1;
        }
    }
    n
}

/// The pan offset that puts texture point `(x, y)` at the work area's
/// centre — the click map solved backwards for the offset (the strip's
/// vertical shift cancels: the point lands at the work area's centre,
/// wherever that sits, so the offset needs no term for it; the texture
/// measures y down while the window measures it up, hence the second
/// line's opposite sign).
fn center_offset(spot: (f32, f32), size: [f32; 2], zoom: f32) -> [f32; 2] {
    [
        (size[0] / 2.0 - spot.0) * zoom,
        (spot.1 - size[1] / 2.0) * zoom,
    ]
}

impl MapLayer {
    fn blank(tileset: std::path::PathBuf, cols: usize, rows: usize) -> Self {
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
    fn at(&self, col: i32, row: i32) -> Option<usize> {
        cell_in(col, row, self.cols, self.rows)
            .then(|| (row + self.rows as i32 / 2) * self.cols as i32 + (col + self.cols as i32 / 2))
            .map(|i| i as usize)
    }

    /// Paint over one cell — picture and orientation together; true
    /// when the cell actually changed. A repaint that only turns the
    /// tile IS a change; an eraser on an empty cell is not, whatever
    /// orientation bits it carries.
    fn set_cell(&mut self, col: i32, row: i32, cell: u32, tfm: u8) -> bool {
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
    fn shape(&self, tileset: Option<&Sprite>) -> Option<frost::Shape> {
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
fn cell_in(col: i32, row: i32, cols: usize, rows: usize) -> bool {
    col + cols as i32 / 2 >= 0
        && row + rows as i32 / 2 >= 0
        && col + cols as i32 / 2 < cols as i32
        && row + rows as i32 / 2 < rows as i32
}

/// The map-space point's grid cell: columns to the right, rows
/// downward — the grid's rows run with the texture's, not the window's.
fn grid_cell(mx: f32, my: f32, tw: f32, th: f32) -> (i32, i32) {
    (mx.div_euclid(tw) as i32, (-my).div_euclid(th) as i32)
}

/// The bounds the brush reaches with this tileset: its own layer if
/// one exists, else the map a first click would create.
fn paint_bounds(maps: &[MapLayer], tileset: &std::path::Path) -> (usize, usize) {
    maps.iter()
        .find(|m| m.tileset == tileset)
        .map_or((MAP_COLS, MAP_ROWS), |m| (m.cols, m.rows))
}

/// Paint one cell with the tileset's layer, creating the layer on the
/// first stroke. True when a cell changed.
/// A copied rectangle of the map, held on the desk: one tileset's
/// cells and orientations, row-major from the top line. It dresses
/// and pastes as one picture — a block is a brush made of tiles.
#[derive(Clone)]
struct Clip {
    tileset: std::path::PathBuf,
    cols: usize,
    rows: usize,
    /// Row-major, top line first; `EMPTY_CELL` marks a gap.
    cells: Vec<u32>,
    tfms: Vec<u8>,
}

/// The copy of a selection rectangle: one tileset's view of the map,
/// `EMPTY_CELL` wherever its layer was empty or off the edge.
fn clip_take(
    maps: &[MapLayer],
    tileset: &std::path::Path,
    c0: i32,
    r0: i32,
    cols: usize,
    rows: usize,
) -> Clip {
    let layer = maps.iter().find(|m| m.tileset == tileset);
    let mut cells = Vec::with_capacity(cols * rows);
    let mut tfms = Vec::with_capacity(cols * rows);
    for r in 0..rows {
        for c in 0..cols {
            let hit = layer.and_then(|m| {
                m.at(c0 + c as i32, r0 + r as i32)
                    .map(|i| (m.cells[i], m.tfms[i]))
            });
            match hit {
                Some((cell, tfm)) => {
                    cells.push(cell);
                    tfms.push(tfm);
                }
                None => {
                    cells.push(EMPTY_CELL);
                    tfms.push(0);
                }
            }
        }
    }
    Clip {
        tileset: tileset.to_path_buf(),
        cols,
        rows,
        cells,
        tfms,
    }
}

/// The block's anchor cell: as near the center as whole cells allow
/// (even sides bias up-left). The ghost and the paste share it, so
/// what you preview is what you get.
fn clip_anchor(clip: &Clip) -> (i32, i32) {
    (((clip.cols - 1) / 2) as i32, ((clip.rows - 1) / 2) as i32)
}

/// The ghost's list: the drop's cells, cut to the map — the very
/// intersection the drop applies, lifted out of the draw so the test
/// can stand in for the eye.
fn ghost_cells(
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

/// Where a held block lands when the cursor drops it at `(col, row)`:
/// every non-empty cell, in absolute map coordinates, cursor-anchored
/// on the block's anchor. The ghost previews this list (intersected
/// with the map) and the paste writes it — one source, so the promise
/// and the drop cannot disagree, at the edges or anywhere else.
fn clip_cells(clip: &Clip, col: i32, row: i32) -> Vec<(i32, i32, u32, u8)> {
    let (ax, ay) = clip_anchor(clip);
    clip.cells
        .iter()
        .enumerate()
        .filter(|(_, c)| **c != EMPTY_CELL)
        .map(|(i, c)| {
            (
                col + (i % clip.cols) as i32 - ax,
                row + (i / clip.cols) as i32 - ay,
                *c,
                clip.tfms[i],
            )
        })
        .collect()
}

/// Turn the block a quarter: the arrangement rotates, and every tile
/// turns with it EXACTLY as it would on its own — the very rule the
/// brush obeys (`tfm_turn`): the turns count rides on, the tile's own
/// flips ride along untouched. A block is a brush made of tiles, so
/// dressing the block IS dressing each tile it holds; any other
/// composition rule would make the block disagree with the single
/// tile the picker just dressed.
fn clip_turn(clip: &mut Clip, cw: bool) {
    let (rows, cols) = (clip.rows, clip.cols);
    let mut cells = vec![EMPTY_CELL; rows * cols];
    let mut tfms = vec![0u8; rows * cols];
    for r in 0..rows {
        for c in 0..cols {
            let (nr, nc) = if cw {
                (c, rows - 1 - r)
            } else {
                (cols - 1 - c, r)
            };
            let at = nr * rows + nc;
            cells[at] = clip.cells[r * cols + c];
            tfms[at] = tfm_turn(clip.tfms[r * cols + c], cw);
        }
    }
    clip.cells = cells;
    clip.tfms = tfms;
    clip.cols = rows;
    clip.rows = cols;
}

/// Mirror the block: the arrangement reverses along the axis and
/// every tile mirrors with it — each through `tfm_flip_x`/
/// `tfm_flip_y`, so an odd-turned tile pays on the other bit. Byte-
/// equal to mirroring the block's rendered picture, per the golden
/// test.
fn clip_flip(clip: &mut Clip, x: bool) {
    let (rows, cols) = (clip.rows, clip.cols);
    let mut cells = vec![EMPTY_CELL; rows * cols];
    let mut tfms = vec![0u8; rows * cols];
    for r in 0..rows {
        for c in 0..cols {
            let (nr, nc) = if x {
                (r, cols - 1 - c)
            } else {
                (rows - 1 - r, c)
            };
            let at = nr * cols + nc;
            cells[at] = clip.cells[r * cols + c];
            tfms[at] = if x {
                tfm_flip_x(clip.tfms[r * cols + c])
            } else {
                tfm_flip_y(clip.tfms[r * cols + c])
            };
        }
    }
    clip.cells = cells;
    clip.tfms = tfms;
}

/// The selection's frame turns with the block it frames: a turned
/// clip holds transposed extents, and an unturned frame would show
/// the tiles standing in the wrong cells — right at 180 (extents
/// survive), wrong at every quarter turn. The frame pivots on its
/// anchor cell, the same cell the block itself pivots on, so frame
/// and picture always agree; cw and ccw take the same rectangle,
/// and two turns return it exactly home. A flip is a no-op: the
/// extents and the anchor cell both survive a mirror.
fn sel_turn(sel: [i32; 4]) -> [i32; 4] {
    let [c0, r0, c1, r1] = sel;
    let (w, h) = (c1 - c0 + 1, r1 - r0 + 1);
    let (ac, ar) = ((w - 1) / 2, (h - 1) / 2);
    let nc0 = c0 + ac - ar;
    let nr0 = r0 + ar - ac;
    [nc0, nr0, nc0 + h - 1, nr0 + w - 1]
}

/// Shrink a block to the rectangle that actually holds a picture.
/// Empty margins would swing the picture far from the pivot on a
/// turn and drop it nowhere near the cursor on a paste, so the desk
/// trims on harvest. Returns the kept rectangle in old-frame
/// coordinates (c0, r0, c1, r1), or None when the block is all gaps.
fn clip_trim(clip: &mut Clip) -> Option<(usize, usize, usize, usize)> {
    let (mut rmin, mut rmax, mut cmin, mut cmax) = (usize::MAX, 0usize, usize::MAX, 0usize);
    for (i, c) in clip.cells.iter().enumerate() {
        if *c != EMPTY_CELL {
            let (r, col) = (i / clip.cols, i % clip.cols);
            rmin = rmin.min(r);
            rmax = rmax.max(r);
            cmin = cmin.min(col);
            cmax = cmax.max(col);
        }
    }
    if rmin > rmax {
        return None;
    }
    let (rows, cols) = (rmax - rmin + 1, cmax - cmin + 1);
    let mut cells = Vec::with_capacity(rows * cols);
    let mut tfms = Vec::with_capacity(rows * cols);
    for r in rmin..=rmax {
        for c in cmin..=cmax {
            cells.push(clip.cells[r * clip.cols + c]);
            tfms.push(clip.tfms[r * clip.cols + c]);
        }
    }
    clip.cols = cols;
    clip.rows = rows;
    clip.cells = cells;
    clip.tfms = tfms;
    Some((cmin, rmin, cmax, rmax))
}

/// Stamp the block with its anchor cell on `(col, row)`. Gaps skip —
/// pasting lays tiles, it never erases; the eraser is its own verb.
/// Returns the cells the stamp actually changed.
/// Move the held block: cut the selection frame it was copied from,
/// drop the block cursor-anchored at (col, row), and let the frame
/// travel with the drop — the block's new home becomes its cut
/// zone. Without a frame (held since the frame was released) there
/// is nothing to cut and the move is a plain drop. Returns the cells
/// written, whether the cut changed anything, and the frame's new
/// box — the caller's one capture covers both halves.
fn clip_move(
    maps: &mut Vec<MapLayer>,
    clip: &Clip,
    sel: Option<[i32; 4]>,
    col: i32,
    row: i32,
) -> (u32, bool, Option<[i32; 4]>) {
    let mut erased = false;
    if let Some(f) = sel {
        erased = erase_rect(maps, f);
    }
    let n = clip_paste(maps, clip, col, row);
    let (ax, ay) = clip_anchor(clip);
    let frame = [
        col - ax,
        row - ay,
        col - ax + clip.cols as i32 - 1,
        row - ay + clip.rows as i32 - 1,
    ];
    (n, erased, Some(frame))
}

fn clip_paste(maps: &mut Vec<MapLayer>, clip: &Clip, col: i32, row: i32) -> u32 {
    let mut n = 0;
    for (c, r, cell, tfm) in clip_cells(clip, col, row) {
        if paint_at(maps, &clip.tileset, cell as usize, tfm, c, r) {
            n += 1;
        }
    }
    n
}

/// Lift every cell of the selection, on every layer. One sweep, and
/// the caller's one undo step.
fn erase_rect(maps: &mut [MapLayer], sel: [i32; 4]) -> bool {
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
fn desk_cell(sp: Option<&Sprite>, p: [f32; 2], view: [f32; 2], zoom: f32) -> Option<(i32, i32)> {
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

fn paint_at(
    maps: &mut Vec<MapLayer>,
    tileset: &std::path::Path,
    cell: usize,
    tfm: u8,
    col: i32,
    row: i32,
) -> bool {
    let i = match maps.iter().position(|m| m.tileset == tileset) {
        Some(i) => i,
        None => {
            maps.push(MapLayer::blank(tileset.to_path_buf(), MAP_COLS, MAP_ROWS));
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
enum Dress {
    FlipX,
    FlipY,
    /// A quarter turn: true is clockwise.
    Turn(bool),
}

fn dress_gesture(shift: bool, ctrl: bool, alt: bool) -> Dress {
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
fn dress_brush(brush: u8, d: Dress) -> u8 {
    match d {
        Dress::FlipY => tfm_flip_y(brush),
        Dress::FlipX => tfm_flip_x(brush),
        Dress::Turn(cw) => tfm_turn(brush, cw),
    }
}

/// Apply a gesture's verb to a held block: the whole picture —
/// arrangement and every tile — as one; a turn moves the selection
/// frame with it.
fn dress_block(clip: &mut Clip, sel: &mut Option<[i32; 4]>, d: Dress) {
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
fn tfm_turn(code: u8, cw: bool) -> u8 {
    let rot = ((code >> 2) & 3) + if cw { 1 } else { 3 };
    (code & 3) | ((rot & 3) << 2)
}

/// Flip the tile's picture sideways — on SCREEN axes, whatever the
/// tile's turn: the shader walks the turns first and lays the flip
/// bits down afterwards, so a tile turned an odd quarter mirrors
/// along the OTHER bit. A golden test pins this byte-for-byte
/// against the real rotated pixels.
fn tfm_flip_x(code: u8) -> u8 {
    code ^ if (code >> 2) & 1 == 1 { 2 } else { 1 }
}

/// Flip the tile's picture upright-mirrored: same story as
/// `tfm_flip_x`, one axis over.
fn tfm_flip_y(code: u8) -> u8 {
    code ^ if (code >> 2) & 1 == 1 { 1 } else { 2 }
}

/// The brush's turn in words, for the status line and the rack.
fn turn_note(code: u8) -> &'static str {
    match (code >> 2) & 3 {
        0 => "upright",
        1 => "90 deg cw",
        2 => "upside down",
        _ => "90 deg ccw",
    }
}

/// Clear one cell on every layer: the eraser lifts what stands there,
/// whichever tileset painted it. True when anything came off.
fn erase_at(maps: &mut [MapLayer], col: i32, row: i32) -> bool {
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
fn ghost_transform(zoom: f32, center: [f32; 2]) -> frost::Transform {
    frost::Transform::scale([zoom, zoom]).compose(&frost::Transform::translate(center))
}

/// The grid's drawn step, in map units: the cell's own pitch, doubled
/// up the ladder while its lines would stand closer than
/// `GRID_MIN_PX` window pixels apart. Always a whole power-of-two
/// count of CELLS — that is the point: a step merely clamped to the
/// pixel floor would slide the lines off the cell boundaries, and a
/// grid the ghost no longer stands on is a lie about the map.
fn grid_step(cell: f32, zoom: f32) -> f32 {
    let mut s = cell;
    while s * zoom < GRID_MIN_PX {
        s *= 2.0;
    }
    s
}

/// The atlas cell `idx`'s texture rectangle in a `rows`-by-`cols` grid,
/// row-major, top-left origin like the PNG's own rows.
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

/// The rack's cell edge for a grid in a window: the standard box,
/// shrunk only when the whole matrix would not fit above the strip.
fn band_fit(rows: usize, cols: usize, w: f32, h: f32) -> f32 {
    let cw = (w - 40.0) / cols.max(1) as f32 - BAND_GAP;
    let ch = (h - STRIP_H - 28.0) / rows.max(1) as f32 - BAND_GAP;
    BAND_CELL.min(cw).min(ch).max(12.0)
}

/// A cell's edge at the panel's own zoom: the fitted box scaled by
/// the wheel — a number the rack draws, the hit test hits, and the
/// shapes scale by, so the panel is one honest geometry.
fn band_cell(rows: usize, cols: usize, w: f32, h: f32, zoom: f32) -> f32 {
    (band_fit(rows, cols, w, h) * zoom).max(6.0)
}

/// The panel's corner: the plate's lower-left, at rest in the desk's
/// lower-left, above the slots strip.
fn band_origin_default(w: f32, h: f32) -> [f32; 2] {
    [-w / 2.0 + 14.0, -h / 2.0 + STRIP_H + 8.0]
}

/// The panel's boxes: the active tileset's grid drawn as its matrix,
/// row-major like the atlas, bottom row nearest the origin. The panel
/// and its hit test draw from the same list, so what is clickable is
/// exactly what is drawn; a grid deeper than the pool keeps only its
/// first `BAND_CELLS` cells.
fn band_rects(rows: usize, cols: usize, cell: f32, origin: [f32; 2]) -> Vec<[f32; 4]> {
    let rows = rows.max(1);
    let cols = cols.max(1);
    let base = origin[1] + BAND_PAD + cell / 2.0;
    (0..(rows * cols).min(BAND_CELLS))
        .map(|i| {
            let (r, c) = (i / cols, i % cols);
            let cx = origin[0] + BAND_PAD + cell / 2.0 + c as f32 * (cell + BAND_GAP);
            let cy = base + (rows - 1 - r) as f32 * (cell + BAND_GAP);
            [
                cx - cell / 2.0,
                cy - cell / 2.0,
                cx + cell / 2.0,
                cy + cell / 2.0,
            ]
        })
        .collect()
}

/// The board behind the shown cells: their extent plus the pad. No
/// cells, no board.
fn band_plate(rects: &[[f32; 4]], cells: usize) -> Option<[f32; 4]> {
    let mut it = rects.iter().take(cells);
    let first = it.next()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first[0], first[1], first[2], first[3]);
    for r in it {
        x0 = x0.min(r[0]);
        y0 = y0.min(r[1]);
        x1 = x1.max(r[2]);
        y1 = y1.max(r[3]);
    }
    Some([x0 - BAND_PAD, y0 - BAND_PAD, x1 + BAND_PAD, y1 + BAND_PAD])
}

/// A panel origin clamped into the desk: the plate stays inside the
/// window and clear of the slots strip; a panel bigger than the desk
/// centers on the room there is.
fn band_clamp(origin: [f32; 2], cell: f32, rows: usize, cols: usize, w: f32, h: f32) -> [f32; 2] {
    let pw = 2.0 * BAND_PAD + cols.max(1) as f32 * cell + (cols.max(1) as f32 - 1.0) * BAND_GAP;
    let ph = 2.0 * BAND_PAD + rows.max(1) as f32 * cell + (rows.max(1) as f32 - 1.0) * BAND_GAP;
    let fix = |o: f32, lo: f32, hi: f32| {
        if hi < lo {
            (lo + hi) / 2.0
        } else {
            o.clamp(lo, hi)
        }
    };
    [
        fix(origin[0], -w / 2.0 + 2.0, w / 2.0 - 2.0 - pw),
        fix(origin[1], -h / 2.0 + STRIP_H + 2.0, h / 2.0 - 2.0 - ph),
    ]
}

/// Whether `p` (if any) lies inside the rectangle.
fn p_in(p: Option<[f32; 2]>, [x0, y0, x1, y1]: [f32; 4]) -> bool {
    p.is_some_and(|[x, y]| x >= x0 && x <= x1 && y >= y0 && y <= y1)
}

/// The band cell at `p`, if `p` lands on one of the `cells` cells the
/// panel actually shows.
fn band_pick(
    p: [f32; 2],
    rows: usize,
    cols: usize,
    cell: f32,
    origin: [f32; 2],
    cells: usize,
) -> Option<usize> {
    band_rects(rows, cols, cell, origin)
        .into_iter()
        .take(cells)
        .position(|[x0, y0, x1, y1]| p[0] >= x0 && p[0] <= x1 && p[1] >= y0 && p[1] <= y1)
}

/// The position whose marker sits nearest `at` (window pixels), within
/// `r` of it — the sprite's way back into the tree.
fn spot_pick(
    spots: &[Spot],
    at: [f32; 2],
    size: [f32; 2],
    view: [f32; 2],
    zoom: f32,
    r: f32,
) -> Option<usize> {
    let [tw, th] = size;
    let mut best: Option<(f32, usize)> = None;
    for (pi, s) in spots.iter().enumerate() {
        let wx = (s.x - tw / 2.0) * zoom + view[0];
        let wy = (th / 2.0 - s.y) * zoom + view[1];
        let d = ((wx - at[0]).powi(2) + (wy - at[1]).powi(2)).sqrt();
        if d <= r && best.is_none_or(|(b, _)| d < b) {
            best = Some((d, pi));
        }
    }
    best.map(|(_, pi)| pi)
}

/// Open every container along a fold-path, so a row found on the sprite
/// can actually show itself in the tree.
fn unfold_path(root: &mut ron_tree::Val, path: &[u16]) {
    for k in 0..path.len() {
        if let Some(v) = ron_tree::walk(root, &path[..k]) {
            match v {
                ron_tree::Val::Seq { open, .. }
                | ron_tree::Val::Struct { open, .. }
                | ron_tree::Val::Map { open, .. } => *open = true,
                ron_tree::Val::Atom(..) => {}
            }
        }
    }
}

/// The list-edit buttons a row carries: `(add, del)` — an add on an
/// open sequence's head row, a delete on one of a sequence's OWN item
/// rows (a head or an atom; the closing bracket row never gets one).
fn row_buttons(root: &ron_tree::Val, row: &ron_tree::Row) -> (bool, bool) {
    let add =
        matches!(row.vkind, ron_tree::VKind::Head) && row.fold == Some(true) && row.val == "[";
    let del = !row.path.is_empty()
        && !matches!(row.vkind, ron_tree::VKind::Close)
        && ron_tree::find(root, &row.path[..row.path.len() - 1])
            .is_some_and(|p| matches!(p, ron_tree::Val::Seq { .. }));
    (add, del)
}

/// Append to the sequence at `path` a copy of its last entry — the
/// shape fits, the comments do not travel — or a fresh `0` when the
/// sequence is empty; the new entry's index back.
fn seq_add(root: &mut ron_tree::Val, path: &[u16]) -> Option<usize> {
    let ron_tree::Val::Seq { items, .. } = ron_tree::walk(root, path)? else {
        return None;
    };
    let fresh = match items.last() {
        Some(it) => ron_tree::Item::plain(zero_like(&it.val)),
        // An empty list keeps no shape to copy; in this sidecar's world
        // a list is a list of positions, so the first entry starts as
        // an unplaced one at the corner.
        None => ron_tree::Item::plain(ron_tree::Val::Struct {
            open: true,
            head: String::new(),
            curly: false,
            fields: (0..2)
                .map(|_| {
                    (
                        String::new(),
                        ron_tree::Item::plain(ron_tree::Val::Atom(
                            String::from("0.0"),
                            ron_tree::Kind::Num,
                        )),
                    )
                })
                .collect(),
            tail: String::new(),
        }),
    };
    items.push(fresh);
    Some(items.len() - 1)
}

/// The emptied shape of a value: an atom to a fresh zero of its kind,
/// a container to the same skeleton with every child emptied — folds
/// open, comments left out. A sequence's new entry takes this shape
/// from its neighbour's, never a copy of one: an exact twin of the
/// last marker would land on the very pixel its twin already holds,
/// indistinguishable, unclickable, shaky.
fn zero_like(v: &ron_tree::Val) -> ron_tree::Val {
    use ron_tree::{Item, Val};
    match v {
        Val::Atom(_, kind) => Val::Atom(
            match kind {
                ron_tree::Kind::Num => String::from("0.0"),
                ron_tree::Kind::Bool => String::from("false"),
                ron_tree::Kind::Str => String::from("\"\""),
                ron_tree::Kind::Char => String::from("'\\''"),
                // A path is its variant's name: there is no emptier one.
                ron_tree::Kind::Path => v_atom_text(v),
            },
            *kind,
        ),
        Val::Seq { .. } => Val::Seq {
            open: true,
            items: Vec::new(),
            tail: String::new(),
        },
        Val::Struct {
            head,
            curly,
            fields,
            ..
        } => Val::Struct {
            open: true,
            head: head.clone(),
            curly: *curly,
            fields: fields
                .iter()
                .map(|(name, it)| (name.clone(), Item::plain(zero_like(&it.val))))
                .collect(),
            tail: String::new(),
        },
        Val::Map { entries, .. } => Val::Map {
            open: true,
            entries: entries
                .iter()
                .map(|(k, it)| (k.clone(), Item::plain(zero_like(&it.val))))
                .collect(),
            tail: String::new(),
        },
    }
}

/// The lone atom's own text — a path keeps its name when emptied.
fn v_atom_text(v: &ron_tree::Val) -> String {
    match v {
        ron_tree::Val::Atom(text, _) => text.clone(),
        _ => String::new(),
    }
}

/// Remove the nth entry of the sequence at `path`; true when it was there.
fn seq_remove(root: &mut ron_tree::Val, path: &[u16], n: usize) -> bool {
    if let Some(ron_tree::Val::Seq { items, .. }) = ron_tree::walk(root, path)
        && n < items.len()
    {
        items.remove(n);
        return true;
    }
    false
}

/// Slide one entry of the sequence to another entry's seat; the drag
/// takes single steps from the entry's current seat each frame. The
/// true seat it landed on, when anything moved.
fn seq_move(root: &mut ron_tree::Val, path: &[u16], from: usize, to: usize) -> Option<usize> {
    if let Some(ron_tree::Val::Seq { items, .. }) = ron_tree::walk(root, path)
        && from < items.len()
    {
        let to = to.min(items.len() - 1);
        if to != from {
            let it = items.remove(from);
            items.insert(to, it);
            return Some(to);
        }
    }
    None
}

/// Whether a row wears the selection band: the row describes the very
/// position picked for editing. Both-sides-`None` is NOT a match — no
/// position picked means no band, or every key and bracket row in the
/// tree would light up at once.
fn row_selected(owner: Option<usize>, editing: Option<usize>) -> bool {
    owner.is_some() && owner == editing
}

/// One sidecar view's tree spec, built from its document: the rows
/// re-labelled for the widget — a row that owns a position wears its
/// spot's palette colour, every other row its kind's — with the add
/// and delete buttons the list-edit rules give it, and the selection
/// band on the rows of the picked position, in this view alone.
fn ron_spec<'a>(
    sprites: &'a [Sprite],
    active: usize,
    v: &RonView,
    lines: &'a mut Vec<frost::TreeLine<'a>>,
) -> Option<frost::TreeSpec<'a>> {
    let doc = sprites.get(v.slot)?.ron.as_ref()?;
    let mut spots = Vec::new();
    scan_spots(&doc.root, &mut Vec::new(), "", &mut spots);
    lines.clear();
    for row in doc.rows.iter() {
        let owner = spot_of_row(&spots, &row.path);
        let val_color = owner.map(|pi| PALETTE[pi % PALETTE.len()]).map_or(
            ron_color(row.vkind),
            |(r, g, b)| frost::Color { r, g, b, a: 1.0 },
        );
        let (add, del) = row_buttons(&doc.root, row);
        lines.push(frost::TreeLine {
            key: row.key.as_str(),
            val: row.val.as_str(),
            head: matches!(row.vkind, ron_tree::VKind::Head | ron_tree::VKind::Close),
            val_color,
            add,
            del,
            band: row_selected(owner, doc.edit) && v.slot == active,
        });
    }
    Some(frost::TreeSpec {
        id: v.slot as u64,
        rows: lines.as_slice(),
        panel: v.panel,
        body: (v.body != [0.0; 4] && !v.folded).then_some(v.body),
        close: v.close,
        folded: v.folded,
        scroll: doc.scroll,
        sx: doc.sx,
        size: [doc.vw, doc.vh],
        style: frost::TreeStyle::default(),
        z: RON_ORDER,
    })
}

/// One open sidecar's panel as the UI laid it out this frame: whose
/// slot it shows, its body viewport and whole-panel rectangles, the
/// close button's centre, and whether the title bar folded it.
struct RonView {
    slot: usize,
    body: [f32; 4],
    panel: [f32; 4],
    close: [f32; 2],
    folded: bool,
}

/// Close one slot's place in a set of views laid out before the close:
/// the closed slot's view goes with the file, and every later view now
/// speaks for the slot its sprite shifted into. The frames after the
/// close rebuild the views from the sprite list; this keeps the one
/// that still draws after the close honest.
fn close_view_slot(views: &mut Vec<RonView>, slot: usize) {
    views.retain(|v| v.slot != slot);
    for v in views.iter_mut() {
        if v.slot > slot {
            v.slot -= 1;
        }
    }
}

/// How many characters a view `w` pixels wide shows side to side.
fn ron_chars(w: f32) -> usize {
    ((w - 2.0 * RON_INSET) / RON_ADV).max(8.0) as usize
}

/// The widest row of the tree, in characters — its horizontal extent.
fn ron_width(doc: &RonDoc) -> usize {
    doc.rows
        .iter()
        .map(|r| r.key.chars().count() + r.val.chars().count())
        .max()
        .unwrap_or(0)
}

/// The active slot after swapping slots `i` and `j`: the marker follows
/// the sprite it was on, which may have moved.
fn swapped_active(active: usize, i: usize, j: usize) -> usize {
    if active == i {
        j
    } else if active == j {
        i
    } else {
        active
    }
}

/// The strip's visible window for a plate count: where it starts, how
/// wide the pool can show, and how far the scroll may run.
struct StripView {
    x0: f32,
    pitch: f32,
    visible_px: f32,
    track_w: f32,
    scroll_max: f32,
}

fn strip_view(w: f32, count: usize) -> StripView {
    let pitch = SLOT + SLOT_GAP;
    let x0 = -w / 2.0 + SLOT_MARGIN;
    let across = (((w - 2.0 * SLOT_MARGIN) + SLOT_GAP) / pitch).floor() as usize;
    let visible = across.clamp(1, SLOTS);
    let visible_px = visible as f32 * pitch - SLOT_GAP;
    let total_px = count.max(1) as f32 * pitch - SLOT_GAP;
    StripView {
        x0,
        pitch,
        visible_px,
        track_w: (w - 2.0 * SLOT_MARGIN).max(1.0),
        scroll_max: (total_px - visible_px).max(0.0),
    }
}

/// The strip's scrollbar rides the window's bottom edge.
const SCROLL_Y: f32 = 6.0;
const SCROLL_H: f32 = 8.0;

/// The knob's left edge and width at a scroll position.
fn scroll_knob(sv: &StripView, scroll: f32) -> (f32, f32) {
    let total = (sv.visible_px + sv.scroll_max).max(1.0);
    let kw = (sv.track_w * (sv.visible_px / total)).max(30.0);
    let x = sv.x0 + (scroll / sv.scroll_max.max(1.0)) * (sv.track_w - kw);
    (x, kw)
}

/// The `i`th slot's center, in window coordinates, at a scroll offset.
fn slot_center(i: usize, w: f32, h: f32, scroll: f32) -> [f32; 2] {
    [
        -w / 2.0 + SLOT_MARGIN + SLOT / 2.0 + i as f32 * (SLOT + SLOT_GAP) - scroll,
        -h / 2.0 + STRIP_H / 2.0,
    ]
}

/// The slot under a window point — among the `count` plates the strip
/// shows, and only within the pool's visible box.
fn slot_at(p: [f32; 2], w: f32, h: f32, scroll: f32, count: usize) -> Option<usize> {
    if p[1] > -h / 2.0 + STRIP_H {
        return None;
    }
    let sv = strip_view(w, count);
    if p[0] < sv.x0 || p[0] > sv.x0 + sv.visible_px {
        return None;
    }
    (0..count).find(|i| {
        let [cx, cy] = slot_center(*i, w, h, scroll);
        (p[0] - cx).abs() <= SLOT / 2.0 && (p[1] - cy).abs() <= SLOT / 2.0
    })
}

/// Whether a window point is in the work area — above the slots strip.
/// What keeps an open stroke writing: paint rides the left button;
/// the eraser rides the Delete key (or its partner, Ctrl) — held as
/// a drag (key and button) or, when the key opened it alone, the key
/// by itself.
fn stroke_held(st: &Paint, down: bool, del: bool) -> bool {
    if st.erase {
        del && (down || st.key_opened)
    } else {
        down
    }
}

/// Whether a window point lies in the work area (everything above
/// the slots strip).
fn in_work_area(p: [f32; 2], _w: f32, h: f32) -> bool {
    p[1] >= -h / 2.0 + STRIP_H
}

/// The window point's position in the texture's pixel space: `(0, 0)`
/// upper-left, `x` right, `y` down — the sprite is centered at `view`
/// (its window-space center) and drawn at scale `zoom`.
fn tex_point(p: [f32; 2], size: [f32; 2], view: [f32; 2], zoom: f32) -> [f32; 2] {
    let [tw, th] = size;
    [
        (p[0] - view[0]) / zoom + tw / 2.0,
        th / 2.0 - (p[1] - view[1]) / zoom,
    ]
}

/// The selection rectangle between two window points, in texture pixels
/// and clamped to the texture; a rectangle under a pixel wide is no
/// selection at all.
fn sel_rect_from(
    a: [f32; 2],
    b: [f32; 2],
    size: [f32; 2],
    view: [f32; 2],
    zoom: f32,
) -> Option<[f32; 4]> {
    let [tw, th] = size;
    let ta = tex_point(a, size, view, zoom);
    let tb = tex_point(b, size, view, zoom);
    let x0 = ta[0].min(tb[0]).clamp(0.0, tw);
    let x1 = ta[0].max(tb[0]).clamp(0.0, tw);
    let y0 = ta[1].min(tb[1]).clamp(0.0, th);
    let y1 = ta[1].max(tb[1]).clamp(0.0, th);
    if x1 - x0 >= 1.0 && y1 - y0 >= 1.0 {
        Some([x0, y0, x1, y1])
    } else {
        None
    }
}

/// The selection's whole pixels: floor the top-left, ceil the bottom-right
/// (so a drag covers every pixel it touched), clamped to the texture;
/// `None` if nothing whole is inside.
fn sel_int_rect(sel: [f32; 4], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let x0 = sel[0].floor().max(0.0);
    let y0 = sel[1].floor().max(0.0);
    let x1 = sel[2].ceil().min(w as f32);
    let y1 = sel[3].ceil().min(h as f32);
    if x1 > x0 && y1 > y0 {
        Some((x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32))
    } else {
        None
    }
}

/// The bounding box of the pixels with a non-zero alpha, as `(x, y, w, h)`;
/// `None` when the image is fully transparent.
fn alpha_bbox(img: &image::RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] != 0 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x0 == u32::MAX {
        None
    } else {
        Some((x0, y0, x1 - x0 + 1, y1 - y0 + 1))
    }
}

/// Encode an RGBA image to PNG bytes in memory.
fn png_bytes(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    Ok(png)
}

/// The file's name for the dialogs and the logs.
fn file_name_of(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unnamed")
        .to_string()
}

/// The native "already exists — overwrite?" question, parented to the
/// window like every other dialog.
fn confirm_overwrite(ctx: &frost::Context, file: &str) -> bool {
    let mut dialog = rfd::MessageDialog::new()
        .set_title("sprite_util")
        .set_level(rfd::MessageLevel::Warning)
        .set_description(format!("'{file}' already exists. Overwrite it?"))
        .set_buttons(rfd::MessageButtons::YesNo);
    if let Some(window) = ctx.window() {
        dialog = dialog.set_parent(window);
    }
    dialog.show() == rfd::MessageDialogResult::Yes
}

/// Builds the checker backdrop's sprite: one texture pixel per cell —
/// `cw` by `ch` pixels, the light and dark greys alternating — encoded to
/// PNG in memory. The nearest-neighbor sampler then keeps the cell edges
/// hard when the sprite is scaled to cover the region.
fn checker_shape(cw: u32, ch: u32, light: f32, dark: f32) -> Option<frost::Shape> {
    let l = (light.clamp(0.0, 1.0) * 255.0).round() as u8;
    let d = (dark.clamp(0.0, 1.0) * 255.0).round() as u8;
    // Row 0 is the top row, so the upper-left cell is light.
    let mut pixels = Vec::with_capacity((cw * ch * 4) as usize);
    for y in 0..ch {
        for x in 0..cw {
            let grey = if (x + y) % 2 == 0 { l } else { d };
            pixels.extend_from_slice(&[grey, grey, grey, 255]);
        }
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&pixels, cw, ch, image::ExtendedColorType::Rgba8)
        .ok()?;
    frost::Shape::sprite_bytes_nearest(&png).ok()
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset paths to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");

    // The folder the dialog opens in when no input folder is given:
    // where the example was launched from, not where its binary lives.
    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(err) => {
            log::error!("failed to read the working directory: {err}");
            std::process::exit(1);
        }
    };

    // The first sprite's source: a `-i`/`--input` file is opened
    // immediately, without any file dialog; a `-i`/`--input` folder is
    // where the file dialog opens; and without the argument the dialog
    // opens in the working directory. The dialog folder is resolved to an
    // absolute path first — the native dialog (on Windows at least)
    // ignores relative paths and falls back to its default location, so
    // the path is joined onto the working directory (an absolute input
    // replaces it) and canonicalized.
    let Args { input } = Args::parse();
    // Where the workbench starts. The boot construction below honors
    // it (a command-line sprite must not land on a canvas that starts
    // on the tile-map desk), as does the `Demo`'s own initial state.
    let start_view = View::Markers;
    let (immediate, dialog_dir) = match &input {
        Some(path) if path.is_file() => (Some(path.as_path()), None),
        Some(path) if path.is_dir() => {
            let dir = cwd.join(path).canonicalize().unwrap_or_else(|err| {
                log::error!("failed to resolve the folder '{}': {err}", path.display());
                std::process::exit(1);
            });
            (None, Some(dir))
        }
        Some(path) => {
            log::error!("'{}' is neither a file nor a folder", path.display());
            std::process::exit(1);
        }
        None => (None, Some(cwd)),
    };

    // A file on the command line is loaded here, before the window
    // exists, and takes the leftmost slot; otherwise all slots start
    // empty: the workbench is the tool's neutral state, and the way to
    // fill it is whatever the user asks for next — File / Open, the
    // panel's open button, Ctrl-O — never something the tool does on
    // its own at launch.
    let sprites: Vec<Sprite> = match immediate {
        Some(file) => vec![read_sprite(file).unwrap_or_else(|err| {
            log::error!("{err}");
            std::process::exit(1);
        })],
        None => Vec::new(),
    };
    let dir = sprites
        .first()
        .and_then(|sp| sp.path.parent())
        .filter(|d| !d.as_os_str().is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| dialog_dir.clone());

    // The HUD font, the same monospaced variable font the diagnostics
    // overlay uses — shared by the panel's labels and the menu bar.
    let font = format!("{root}/assets/fonts/FiraCode-VariableFont_wght.ttf");
    let ui = frost::Ui::from_font(&font).expect("failed to load the UI font");
    // The seat numbers' font: one Arc's worth of the same FiraCode,
    // every marker's text cloning the pointer, never re-reading the
    // file.
    let ron_font: Arc<[u8]> =
        Arc::from(std::fs::read(&font).expect("failed to read the sidecar font"));
    // The help line that used to hang under the title bar is gone: the
    // menu bar, with its names and shortcuts, is the map now. The node
    // stays — the scene's fixed indices do not shift for an undrawn
    // shape.
    let help = None;

    // The work area's layer pool comes first: the process points the
    // nodes at the animation's current frame (or the active sprite
    // alone, in the first node). Then the fixed nodes: help, marker and
    // checker.
    let work_shape = sprites.first().map(|sp| sp.shape.clone());
    let mut children: Vec<Box<frost::SceneNode>> = (0..LAYER_NODES)
        .map(|i| {
            Box::new(frost::SceneNode {
                // A command-line sprite is already in the first node —
                // unless the tool boots straight onto the tile-map
                // desk, which shows maps, not tilesets.
                shape: if i == 0 && start_view != View::TileMap {
                    work_shape.clone()
                } else {
                    None
                },
                order: i as f32 * 0.01, // layered, under the strip's 0.2
                ..Default::default()
            })
        })
        .collect();
    children.extend(vec![
        Box::new(frost::SceneNode {
            // The usage line, pinned near the top by the process.
            shape: help,
            order: 1.0,
            ..Default::default()
        }),
        Box::new(frost::SceneNode {
            // The marker dot left at the last click; radius 0 hides it.
            shape: Some(frost::Shape::Circle {
                center: [0.0, 0.0],
                radius: 0.0,
                color: MARKER,
            }),
            order: 2.0,
            ..Default::default()
        }),
        Box::new(frost::SceneNode {
            // The checker backdrop behind the sprite: the process builds
            // its tiny one-texel-per-cell sprite and positions it each
            // frame.
            shape: None,
            order: CHECK_ORDER,
            ..Default::default()
        }),
    ]);
    // One node per slot; the process points them at the thumbnails. A
    // command-line sprite shows in its slot from the first frame.
    children.extend((0..SLOTS).map(|i| {
        Box::new(frost::SceneNode {
            shape: sprites.get(i).map(|sp| sp.thumb.clone()),
            order: THUMB_ORDER,
            ..Default::default()
        })
    }));
    // The position markers' pool: two circles per position — the colour
    // dot and its white centre — and one seat-number text each.
    children.extend((0..2 * SPOTS_MAX).map(|_| {
        Box::new(frost::SceneNode {
            order: SPOT_ORDER,
            ..Default::default()
        })
    }));
    children.extend((0..BAND_CELLS + 2).map(|_| {
        // The tileset band's cells, the cursor ghost, and the held-
        // brush chip: shapeless until the tile-map desk points them
        // at the active tileset.
        Box::new(frost::SceneNode::default())
    }));
    children.extend((0..SPOTS_MAX).map(|_| {
        Box::new(frost::SceneNode {
            order: SPOT_ORDER + 0.05,
            ..Default::default()
        })
    }));

    let scene = frost::Scene::new(frost::SceneNode {
        // Dark background; the node's transform is ignored.
        shape: Some(frost::Shape::Background { color: BG }),
        children,
        ..Default::default()
    });

    if let Err(err) = frost::run_configured(
        scene,
        Demo {
            ui,
            sprites,
            active: 0,
            anim: Anim {
                looping: true,
                dir: 1,
                ..Anim::default()
            },
            zoom: 1.0,
            offset: [0.0, 0.0],
            selection: None,
            drag_from: None,
            slot_drag: None,
            slot_scroll: 0.0,
            slot_scrub: None,
            band_origin: None,
            band_zoom: 1.0,
            band_drag: None,
            was_undo: false,
            was_open: false,
            was_space: false,
            was_esc: false,
            light: GREY_LIGHT,
            dark: GREY_DARK,
            checker_key: (0, 0, 0, 0),
            was_down: false,
            last_mouse: None,
            last_click: None,
            dir,
            view: start_view,
            band_base: None,
            band_key: None,
            cell_shapes: Vec::new(),
            picked_cell: None,
            maps: Vec::new(),
            paint: None,
            first_frame: true,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            was_redo: false,
            was_zoom_in: false,
            was_zoom_out: false,
            brush: 0,
            was_flipx: false,
            was_flipy: false,
            was_turn: false,
            status: String::new(),
            ron_font,
            r_press: None,
            m_press: None,
            was_rdown: false,
            was_mdown: false,
            quit_prompt: false,
            was_esc_prompt: false,
            was_del: false,
            clip: None,
            sel: None,
            sel_drag: None,
            was_block_esc: false,
            ron_drag: None,
            ron_state: frost::TreeState::default(),
            ron_views: Vec::new(),
        },
        // The room a sidecar tree and the art both want — in logical
        // pixels, so on a 2x display this opens as a 1440-point-wide
        // window rendering at 2880 physical: crisp panels and fonts,
        // and it still fits a laptop screen (1920 logical would not).
        frost::Config {
            window_size: Some([1440, 810]),
            // Escape belongs to the editor: the exit box asks before
            // anything is lost — and so does the window's close
            // button, which asks the very same question.
            escape_exits: false,
            close_exits: false,
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

    #[test]
    fn window_and_texture_points_round_trip() {
        let size = [40.0, 20.0];
        let view = [7.0, -3.0];
        let zoom = 1.7;
        let [px, py] = tex_point([12.0, 5.0], size, view, zoom);
        // The inverse of `tex_point`, back to window pixels.
        let wx = (px - size[0] / 2.0) * zoom + view[0];
        let wy = (size[1] / 2.0 - py) * zoom + view[1];
        assert!((wx - 12.0).abs() < 1e-3, "x came back at {wx}");
        assert!((wy - 5.0).abs() < 1e-3, "y came back at {wy}");
    }

    #[test]
    fn selection_is_texture_pixels_normalized_and_clamped() {
        let size = [100.0, 50.0];
        // The sprite's center is the texture middle (50, 25); at zoom 2 a
        // 40 px drag covers 20 texture pixels.
        let sel = sel_rect_from([0.0, 0.0], [40.0, 20.0], size, [0.0, 0.0], 2.0).expect("a rect");
        assert_eq!(sel, [50.0, 15.0, 70.0, 25.0]);
        // A drag far beyond the texture clamps to its bounds.
        let sel = sel_rect_from([-1e3, 1e3], [1e3, -1e3], size, [0.0, 0.0], 1.0).expect("a rect");
        assert_eq!(sel, [0.0, 0.0, 100.0, 50.0]);
        // Under a texture pixel wide is no selection at all.
        assert!(sel_rect_from([10.0, 0.0], [10.5, 0.5], size, [0.0, 0.0], 1.0).is_none());
    }

    #[test]
    fn int_selection_floors_out_and_clamps() {
        assert_eq!(
            sel_int_rect([10.2, 4.7, 20.8, 9.9], 100, 100),
            Some((10, 4, 11, 6))
        );
        assert_eq!(
            sel_int_rect([95.5, 96.6, 99.5, 99.9], 100, 100),
            Some((95, 96, 5, 4))
        );
        assert_eq!(sel_int_rect([50.0, 50.0, 50.0, 50.0], 100, 100), None);
    }

    #[test]
    fn alpha_bbox_bounds_the_visible_pixels() {
        let mut buf = image::RgbaImage::from_pixel(20, 10, image::Rgba([0, 0, 0, 0]));
        for (x, y, p) in buf.enumerate_pixels_mut() {
            *p = if (3..7).contains(&x) && (2..5).contains(&y) {
                image::Rgba([1, 2, 3, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            };
        }
        assert_eq!(alpha_bbox(&buf), Some((3, 2, 4, 3)));
        assert_eq!(alpha_bbox(&image::RgbaImage::new(8, 8)), None);
    }

    #[test]
    fn the_png_round_trip_survives_the_codec() {
        let mut buf = image::RgbaImage::new(4, 3);
        buf.put_pixel(2, 1, image::Rgba([255, 0, 0, 255]));
        let png = png_bytes(&buf).expect("encode");
        let back = image::load_from_memory(&png).expect("decode").to_rgba8();
        assert_eq!(back.width(), 4);
        assert_eq!(back.height(), 3);
        assert_eq!(back.get_pixel(2, 1), &image::Rgba([255, 0, 0, 255]));
        assert_eq!(back.get_pixel(0, 0), &image::Rgba([0, 0, 0, 0]));
    }

    #[test]
    fn slots_line_up_along_the_strip_and_are_hit_by_point() {
        let (w, h) = (800.0, 600.0);
        // The leftmost slot starts at the margin, flush with the strip.
        let [cx, cy] = slot_center(0, w, h, 0.0);
        assert_eq!(cx, -w / 2.0 + SLOT_MARGIN + SLOT / 2.0);
        assert_eq!(cy, -h / 2.0 + STRIP_H / 2.0);
        // Neighbors sit one slot and one gap apart.
        let [cx1, _] = slot_center(1, w, h, 0.0);
        assert_eq!(cx1 - cx, SLOT + SLOT_GAP);
        // Each slot's center is its own; the gaps and the area above the
        // strip belong to no slot.
        for i in 0..SLOTS {
            let [x, y] = slot_center(i, w, h, 0.0);
            assert_eq!(slot_at([x, y], w, h, 0.0, i + 1), Some(i));
        }
        let [cx0, cy0] = slot_center(0, w, h, 0.0);
        assert_eq!(
            slot_at([cx0 + SLOT / 2.0 + SLOT_GAP / 2.0, cy0], w, h, 0.0, SLOTS),
            None
        );
        assert_eq!(
            slot_at([cx0, cy0 + STRIP_H / 2.0 + 1.0], w, h, 0.0, SLOTS),
            None
        );
    }

    #[test]
    fn the_strip_always_keeps_a_plate_free_to_load() {
        // One plate per sprite plus the spare: a handful of sprites
        // in a wide window all show, and nobody needs the scrollbar...
        let wide = strip_view(1440.0, 3);
        assert_eq!(wide.scroll_max, 0.0);
        // ...but the pool caps what one window can show, so the
        // plate past the pool's end — the seventh sprite's, or the
        // spare's — earns a scrollbar of exactly the overflow.
        let tight = strip_view(1440.0, SLOTS + 1);
        assert!(tight.scroll_max > 0.0);
    }

    #[test]
    fn the_scroll_moves_what_the_pick_names() {
        let (w, h) = (800.0, 600.0);
        let count = 9;
        let sv = strip_view(w, count);
        assert!(sv.scroll_max > 0.0);
        let pitch = SLOT + SLOT_GAP;
        let cy = -h / 2.0 + STRIP_H / 2.0;
        // One pitch of scroll, and slot 1 stands where slot 0 stood:
        // plates, picks and thumbnails all read the same offset.
        let p = [-w / 2.0 + SLOT_MARGIN + SLOT / 2.0, cy];
        assert_eq!(slot_at(p, w, h, 0.0, count), Some(0));
        assert_eq!(slot_at(p, w, h, pitch, count), Some(1));
        // The knob runs the whole track over the scroll's span.
        let at_rest = scroll_knob(&sv, 0.0);
        let at_end = scroll_knob(&sv, sv.scroll_max);
        assert!((at_rest.0 - sv.x0).abs() < 1e-3);
        assert!((at_end.0 + at_end.1 - (sv.x0 + sv.track_w)).abs() < 1e-3);
    }

    #[test]
    fn the_strip_divides_the_work_area_from_the_slots() {
        let h = 600.0;
        let strip_top = -h / 2.0 + STRIP_H;
        assert!(!in_work_area([0.0, strip_top - 1.0], 800.0, h));
        assert!(in_work_area([0.0, strip_top + 1.0], 800.0, h));
        // The work area's center is half a strip above the bottom edge,
        // so a sprite panned to it floats clear of the slots.
        assert_eq!(WORK_Y, STRIP_H / 2.0);
    }

    #[test]
    fn swapping_slots_moves_the_active_marker_with_its_sprite() {
        // The active sprite sits in slot 2; swapping 1 and 2 moves the
        // marker, swapping 0 and 1 leaves it.
        assert_eq!(swapped_active(2, 1, 2), 1);
        assert_eq!(swapped_active(1, 1, 2), 2);
        assert_eq!(swapped_active(0, 1, 2), 0);
    }

    // A layer-less frame with a chosen hold time, for the clock tests.
    fn frame(next_time: f32) -> Frame {
        Frame {
            next_time,
            ..Frame::default()
        }
    }

    fn anim(times: &[f32], looping: bool) -> Anim {
        Anim {
            frames: times.iter().map(|&t| frame(t)).collect(),
            looping,
            playing: true,
            dir: 1,
            ..Anim::default()
        }
    }

    #[test]
    fn the_clock_holds_each_frame_for_its_time() {
        // Two frames: 0.1 s then 0.2 s. The clock steps only once the
        // current frame's hold is spent, carrying the remainder over.
        let mut a = anim(&[0.1, 0.2], true);
        assert!(!a.tick(0.05)); // mid-frame 0
        assert_eq!(a.frame, 0);
        assert!(a.tick(0.07)); // 0.12 >= 0.1 -> frame 1, 0.02 over
        assert_eq!(a.frame, 1);
        assert!(!a.tick(0.1)); // 0.12 < 0.2 -> still frame 1
        assert_eq!(a.frame, 1);
        assert!(a.tick(0.12)); // 0.24 >= 0.2 -> step, looping back to 0
        assert_eq!(a.frame, 0);
    }

    #[test]
    fn loop_wraps_while_ping_pong_bounces() {
        // Looping: 0 1 2, 0 1 2, …
        let mut a = anim(&[1.0, 1.0, 1.0], true);
        let mut seq = Vec::new();
        for _ in 0..7 {
            seq.push(a.frame);
            a.step();
        }
        assert_eq!(seq, vec![0, 1, 2, 0, 1, 2, 0]);
        // Ping-pong: 0 1 2 1, 0 1 2 1, … — each end held once per sweep.
        let mut b = anim(&[1.0, 1.0, 1.0], false);
        let mut seq = Vec::new();
        for _ in 0..9 {
            seq.push(b.frame);
            b.step();
        }
        assert_eq!(seq, vec![0, 1, 2, 1, 0, 1, 2, 1, 0]);
    }

    #[test]
    fn a_single_frame_animation_never_steps() {
        let mut a = anim(&[0.1], true);
        assert!(!a.tick(1.0));
        assert_eq!(a.frame, 0);
        // Two frames a beat apart step cleanly.
        let mut c = anim(&[0.5, 0.5], false);
        assert!(!c.tick(0.4));
        assert!(c.tick(0.2)); // 0.6 >= 0.5 -> frame 1, 0.1 over
        assert_eq!(c.frame, 1);
    }

    #[test]
    fn a_new_sidecar_panel_opens_at_the_lower_right() {
        let (w, h) = (800.0, 600.0);
        let (row_h, pad) = (28.0, 12.0); // the UI style's defaults
        let c = ron_default(w, h, row_h, pad, 0);
        let panel_h = row_h + 2.0 * pad + RON_VIEW_H;
        assert!(
            c[1] - panel_h / 2.0 >= -h / 2.0 + STRIP_H,
            "a fresh panel should clear the slot strip"
        );
        assert!(
            c[1] + panel_h / 2.0 < h / 2.0,
            "a fresh panel should stay inside"
        );
        assert!((c[0] + RON_W / 2.0 - (w / 2.0 - 20.0)).abs() < 1e-3);
    }
    #[test]
    fn spots_are_the_two_number_tuples_of_the_tree() {
        let root = ron_tree::parse(
            "(segment: 1, image: \"p.png\", lower: (317.0, 671.0), ups: [(1.0, 2.0), (3.5, 4.5)])",
        )
        .expect("a small sidecar");
        let mut spots = Vec::new();
        scan_spots(&root, &mut Vec::new(), "", &mut spots);
        let labels: Vec<&str> = spots.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["lower", "ups[0]", "ups[1]"]);
        assert_eq!((spots[0].x, spots[0].y), (317.0, 671.0));
        // List entries carry their seat for the sprite-view label; a
        // plain field carries none.
        assert_eq!(
            spots.iter().map(|s| s.idx).collect::<Vec<_>>(),
            [None, Some(0), Some(1)]
        );
        // The head row and either coordinate row belong to the spot;
        // an unrelated path belongs to none.
        assert_eq!(spot_of_row(&spots, &spots[0].path), Some(0));
        let mut coord = spots[1].path.clone();
        coord.push(1);
        assert_eq!(spot_of_row(&spots, &coord), Some(1));
        assert_eq!(spot_of_row(&spots, &[9]), None);
    }

    #[test]
    fn a_crop_shifts_every_position_by_the_cut_origin() {
        let mut root = ron_tree::parse(
            "(segment: 1, lower_anchor: (317.0, 600.0), \
            upper_anchor: (317.0, 578.0), flower_anchors: [(308.0, 615.0)])",
        )
        .expect("a small sidecar");
        // The cut's origin (100, 50) leaves a 630 x 620 texture: every
        // point keeps its place relative to the pixels that survive.
        let n = shift_spots(&mut root, 100.0, 50.0, (630, 620));
        assert_eq!(n, 3);
        let mut spots = Vec::new();
        scan_spots(&root, &mut Vec::new(), "", &mut spots);
        assert_eq!((spots[0].x, spots[0].y), (217.0, 550.0));
        assert_eq!((spots[1].x, spots[1].y), (217.0, 528.0));
        assert_eq!((spots[2].x, spots[2].y), (208.0, 565.0));
    }

    #[test]
    fn the_cut_clamps_the_positions_it_eats_onto_the_new_edge() {
        let mut root = ron_tree::parse("(a: (10.0, 700.0), b: (300.0, 660.0))").expect("parses");
        // (10 - 100, 700 - 50) = (-90, 650) is outside the new 300 x 600
        // bounds altogether; (300 - 100, 660 - 50) = (200, 610) is inside
        // the width but past the new bottom.
        let n = shift_spots(&mut root, 100.0, 50.0, (300, 600));
        assert_eq!(n, 2);
        let mut spots = Vec::new();
        scan_spots(&root, &mut Vec::new(), "", &mut spots);
        assert_eq!((spots[0].x, spots[0].y), (0.0, 600.0));
        assert_eq!((spots[1].x, spots[1].y), (200.0, 600.0));
    }

    fn bench(active: usize) -> World {
        World {
            sprites: Vec::new(),
            active,
            anim: WorldAnim {
                frames: Vec::new(),
                frame: 0,
                looping: true,
            },
            view: View::Markers,
            selection: None,
            maps: Vec::new(),
        }
    }

    #[test]
    fn the_history_walks_back_and_forth() {
        // The roads hold the before-pictures of commands. The bench sat
        // on slot 0, a command moved it to 1 (world 0 went down), a
        // second moved it to 2 (world 1 went down): the present is 2.
        let (mut undo, mut redo) = (Vec::new(), Vec::new());
        push_step(&mut undo, &mut redo, bench(0));
        push_step(&mut undo, &mut redo, bench(1));
        // One step back: world 1 emerges, the present goes to redo.
        let past = step_back(&mut undo, &mut redo, bench(2)).expect("one past");
        assert_eq!(past.active, 1);
        assert_eq!(redo.len(), 1);
        // One more: world 0, the bench's first seat.
        let older = step_back(&mut undo, &mut redo, past).expect("two pasts");
        assert_eq!(older.active, 0);
        // And forward again, the same worlds in the same order.
        let back1 = step_forward(&mut undo, &mut redo, older).expect("one future");
        assert_eq!(back1.active, 1);
        let back2 = step_forward(&mut undo, &mut redo, back1).expect("two futures");
        assert_eq!(back2.active, 2);
        // Walked dry, forward is a polite nothing.
        assert!(step_forward(&mut undo, &mut redo, back2).is_none());
    }

    #[test]
    fn a_new_command_dissolves_the_redo_road() {
        let (mut undo, mut redo) = (Vec::new(), Vec::new());
        push_step(&mut undo, &mut redo, bench(0));
        step_back(&mut undo, &mut redo, bench(1));
        assert_eq!(redo.len(), 1);
        // A fresh command: the future the undo had opened is gone.
        push_step(&mut undo, &mut redo, bench(2));
        assert!(redo.is_empty());
    }

    #[test]
    fn the_history_ages_its_oldest_steps_out() {
        let (mut undo, mut redo) = (Vec::new(), Vec::new());
        for i in 0..(HISTORY_MAX + 10) {
            push_step(&mut undo, &mut redo, bench(i % 7));
        }
        assert_eq!(undo.len(), HISTORY_MAX);
        assert_eq!(
            undo.last().expect("not empty").active,
            (HISTORY_MAX + 9) % 7
        );
    }

    fn spots_of(root: &ron_tree::Val) -> Vec<(String, f32, f32)> {
        let mut fresh = Vec::new();
        scan_spots(root, &mut Vec::new(), "", &mut fresh);
        fresh.iter().map(|s| (s.label.clone(), s.x, s.y)).collect()
    }

    #[test]
    fn a_fresh_position_never_lands_on_its_twin() {
        // The shape the sidecar writes: a list of two-number tuples.
        let mut root =
            ron_tree::parse("(flower_anchors: [(308.0, 615.0)])").expect("a small sidecar");
        let n = seq_add(&mut root, &[0]).expect("a sequence");
        assert_eq!(n, 1);
        let found = spots_of(&root);
        // Two positions, and the fresh one is an unplaced corner —
        // never an exact copy of the anchor it grew beside.
        assert_eq!(found.len(), 2);
        assert_eq!((found[1].1, found[1].2), (0.0, 0.0));
        assert_eq!((found[0].1, found[0].2), (308.0, 615.0));
    }

    #[test]
    fn the_first_entry_of_an_empty_list_is_a_placeable_position() {
        // An empty list has no neighbour to take shape from: the [+]
        // must still put down a position the click can claim, not a
        // bare `0` the tree shows but no marker or edit can reach.
        let mut root = ron_tree::parse("(flower_anchors: [])").expect("an empty list");
        let n = seq_add(&mut root, &[0]).expect("a sequence");
        assert_eq!(n, 0);
        let found = spots_of(&root);
        assert_eq!(found.len(), 1, "the new entry is a position, not an atom");
        assert_eq!((found[0].1, found[0].2), (0.0, 0.0));
    }

    #[test]
    fn a_fresh_number_joins_a_list_of_numbers() {
        // The same honesty for the shapes that are not positions: the
        // zero of a number list is a number.
        let mut root = ron_tree::parse("(seats: [1, 2])").expect("numbers");
        assert_eq!(seq_add(&mut root, &[0]), Some(2));
        let Some(ron_tree::Val::Seq { items, .. }) = ron_tree::walk(&mut root, &[0]) else {
            panic!("the seats list is not a sequence");
        };
        let ron_tree::Val::Atom(text, ron_tree::Kind::Num) = &items[2].val else {
            panic!("the fresh seat is not an atom: {:?}", items[2].val);
        };
        assert_eq!(text, "0.0");
    }

    #[test]
    fn sequence_rows_carry_their_buttons() {
        let mut root = ron_tree::parse("(a: [1, (2.0, 3.0)], b: 4)").expect("parses");
        root.open_to(0, 9);
        let rows = ron_tree::layout(&root);
        let row = |want: &str| {
            rows.iter()
                .find(|r| r.val == want && !(want == "(" && r.path.is_empty()))
                .expect(want)
        };
        // The open sequence's head row gets the add; it is a field, not
        // an entry, so it gets no delete.
        assert_eq!(row_buttons(&root, row("[")), (true, false));
        // Both kinds of entry row — a bare atom and a tuple's head —
        // get the delete, and neither is a sequence to add into.
        assert_eq!(row_buttons(&root, row("1")), (false, true));
        assert_eq!(row_buttons(&root, row("(")), (false, true));
        // A plain field row belongs to no list at all — and neither
        // does the root's own `(` head.
        assert_eq!(row_buttons(&root, row("4")), (false, false));
        let root_head = rows.iter().find(|r| r.path.is_empty()).expect("root row");
        assert_eq!(row_buttons(&root, root_head), (false, false));
    }

    #[test]
    fn lists_add_zero_delete_and_slide() {
        let mut root = ron_tree::parse("[10, 20, 30]").expect("parses");
        // Add puts down an emptied shape — a fresh 0.0 beside their
        // numbers — and comments stay behind.
        assert_eq!(seq_add(&mut root, &[]), Some(3));
        assert_eq!(seq_remove(&mut root, &[], 0), true);
        assert_eq!(seq_remove(&mut root, &[], 9), false);
        assert_eq!(seq_move(&mut root, &[], 2, 0), Some(0));
        assert_eq!(seq_move(&mut root, &[], 1, 1), None);
        assert_eq!(
            ron_tree::to_text(&root),
            "[\n    0.0,\n    20,\n    30,\n]\n"
        );
        // Nested sequences answer to their path.
        let mut nest = ron_tree::parse("(ups: [(1.0, 1.0)])").expect("parses");
        assert_eq!(seq_add(&mut nest, &[0]), Some(1));
        assert_eq!(seq_add(&mut nest, &[1]), None);
    }

    #[test]
    fn comments_survive_the_sidecar_round_trip() {
        // The user's rule: saving a sprite must not shred the notes
        // beside its data.
        let src = "// planted in spring\n(\n    lower: (1.0, 2.0), // where it sits\n)\n";
        let doc = ron_tree::parse_doc(src).expect("parses");
        let text = ron_tree::to_text_doc(&doc.header, &doc.root, &doc.trailer);
        assert!(text.contains("// planted in spring"), "{text}");
        assert!(text.contains("// where it sits"), "{text}");
        assert_eq!(ron_tree::parse_doc(&text).expect("re-parses"), doc);
    }

    #[test]
    fn the_nearest_marker_wins_the_pick() {
        let mut root = ron_tree::parse("[(10.0, 20.0), (80.0, 70.0)]").expect("parses");
        root.open_to(0, 9);
        let mut spots = Vec::new();
        scan_spots(&root, &mut Vec::new(), "", &mut spots);
        let size = [100.0, 100.0];
        // Centred view, zoom 1: texture (10, 20) sits at window
        // (-40, 30) — the y flip puts y = 20 above the middle.
        let hit =
            |at: [f32; 2], view: [f32; 2], zoom: f32| spot_pick(&spots, at, size, view, zoom, 5.0);
        assert_eq!(hit([-40.0, 30.0], [0.0, 0.0], 1.0), Some(0));
        assert_eq!(hit([-41.0, 32.0], [0.0, 0.0], 1.0), Some(0));
        assert_eq!(hit([0.0, 0.0], [0.0, 0.0], 1.0), None);
        // Panning and zooming move the markers with the sprite.
        assert_eq!(hit([-30.0, 40.0], [10.0, 10.0], 1.0), Some(0));
        assert_eq!(hit([-80.0, 60.0], [0.0, 0.0], 2.0), Some(0));
    }

    #[test]
    fn centering_puts_the_point_under_the_eye() {
        let [x, y] = center_offset((30.0, 20.0), [100.0, 100.0], 2.0);
        // The click map, run forward from the work centre, must land
        // back on the very texture point that was centred.
        let view = [x, y + WORK_Y];
        let [px, py] = tex_point([0.0, WORK_Y], [100.0, 100.0], view, 2.0);
        assert_eq!((px, py), (30.0, 20.0));
    }

    #[test]
    fn a_path_unfolds_from_root_to_row() {
        let mut root = ron_tree::parse("(a: [(1.0, 2.0), (3.0, 4.0)])").expect("parses");
        root.open_to(0, 0);
        let mut spots = Vec::new();
        scan_spots(&root, &mut Vec::new(), "", &mut spots);
        let deep = spots[1].path.clone();
        unfold_path(&mut root, &deep);
        let rows = ron_tree::layout(&root);
        // The list opened; both entries show, and the path's own row
        // (the tuple head, folded, previewing its numbers) is there to
        // be centred.
        assert!(rows.iter().any(|r| r.path == deep && r.val == "(3.0, 4.0)"));
        assert!(rows.iter().any(|r| r.path == vec![0, 0]));
    }

    #[test]
    fn a_real_plant_sidecar_round_trips_with_its_comments() {
        let src = include_str!("../assets/sprites/plant1.ron");
        let d = ron_tree::parse_doc(src).expect("plant1 parses");
        let text = ron_tree::to_text_doc(&d.header, &d.root, &d.trailer);
        assert_eq!(ron_tree::parse_doc(&text).expect("re-parses"), d);
        for line in src.lines().filter(|l| l.trim_start().starts_with("//")) {
            assert!(text.contains(line.trim_end()), "lost {line:?}");
        }
    }

    #[test]
    fn only_the_picked_positions_row_is_selected() {
        // The bug this nails: an unpicked view (`None`) must match no
        // row — and plain `owner == editing` matched every row that
        // owns no position, banding the head and bracket rows at the
        // tree's top and bottom the moment a file loaded.
        assert!(!row_selected(None, None));
        assert!(row_selected(Some(2), Some(2)));
        assert!(!row_selected(Some(1), Some(2)));
        assert!(!row_selected(None, Some(2)));
        assert!(!row_selected(Some(1), None));
    }

    #[test]
    fn a_sidecar_parses_or_the_sprite_loads_alone() {
        // The tree panel speaks the shared parser: the plant files'
        // tuples, anchors and all — and a broken one is simply None.
        let dir = std::env::temp_dir().join(format!("sprite_util_ron_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("probe.png");
        std::fs::write(&png, b"not really a png, the sidecar loader never reads it").unwrap();
        assert!(load_ron(&png).is_none(), "no sidecar, no panel");
        std::fs::write(
            dir.join("probe.ron"),
            "(segment: 1, lower_anchor: (317.0, 671.0), flower_anchors: [(4.0, 2.0)])",
        )
        .unwrap();
        let doc = load_ron(&png).expect("the sidecar parses");
        assert_eq!(doc.name, "probe.ron");
        assert!(!doc.rows.is_empty());
        std::fs::write(dir.join("probe.ron"), "(segment: ,,,)").unwrap();
        assert!(
            load_ron(&png).is_none(),
            "an unparseable sidecar is no panel"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn view(slot: usize) -> RonView {
        RonView {
            slot,
            body: [0.0; 4],
            panel: [0.0; 4],
            close: [0.0; 2],
            folded: false,
        }
    }

    #[test]
    fn closing_a_slot_shifts_the_views_behind_it() {
        // The bug this nails: a close lands AFTER the frame's views
        // were laid out, and the frame's draw still walks them — the
        // closed file's view must go with it, and every later view must
        // speak for the slot its sprite shifted into.
        let mut views = vec![view(0), view(1), view(2)];
        close_view_slot(&mut views, 1);
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].slot, 0, "the front view keeps its seat");
        assert_eq!(
            views[1].slot, 1,
            "the later view shifts into the freed seat"
        );
    }

    #[test]
    fn closing_the_last_view_leaves_no_dangling_slot() {
        // Closing the last sprite used to leave its view behind,
        // pointing one slot past the end of the shrunken list — the
        // panic in the frame's draw.
        let mut views = vec![view(0), view(1)];
        close_view_slot(&mut views, 1);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].slot, 0);
        // A slot with no view of its own still shifts the later ones.
        let mut views = vec![view(0), view(2)];
        close_view_slot(&mut views, 1);
        assert_eq!(views.len(), 2);
        assert_eq!((views[0].slot, views[1].slot), (0, 1));
    }

    #[test]
    fn the_atlas_reads_the_root_s_two_entry_array() {
        let root = ron_tree::parse("(segment: 1, atlas: [2, 3])").expect("a small sidecar");
        assert_eq!(atlas_of(&root), Some((2, 3)));
        // A field of the wrong shape reads as no grid, the way a
        // missing field does: one entry, a zero count, a float, a
        // tuple, a string.
        assert_eq!(atlas_of(&ron_tree::parse("(atlas: [2])").unwrap()), None);
        assert_eq!(atlas_of(&ron_tree::parse("(atlas: [0, 3])").unwrap()), None);
        assert_eq!(
            atlas_of(&ron_tree::parse("(atlas: [2.0, 3.0])").unwrap()),
            None
        );
        assert_eq!(atlas_of(&ron_tree::parse("(atlas: (2, 3))").unwrap()), None);
        assert_eq!(
            atlas_of(&ron_tree::parse("(atlas: \"grid\")").unwrap()),
            None
        );
        assert_eq!(atlas_of(&ron_tree::parse("(segment: 1)").unwrap()), None);
    }

    #[test]
    fn the_atlas_writes_and_rewrites_the_root_s_field() {
        let mut root = ron_tree::parse("(segment: 1, image: \"p.png\")").expect("a small sidecar");
        assert!(set_atlas(&mut root, Some((2, 3))), "appends the field");
        assert_eq!(atlas_of(&root), Some((2, 3)));
        assert!(set_atlas(&mut root, Some((4, 5))), "rewrites it in place");
        assert_eq!(atlas_of(&root), Some((4, 5)));
        // One `atlas` field, not one per write.
        let fields = match &root {
            ron_tree::Val::Struct { fields, .. } => fields,
            _ => panic!("a tuple-struct root"),
        };
        assert_eq!(fields.iter().filter(|(k, _)| *k == "atlas").count(), 1);
        assert!(set_atlas(&mut root, None), "drops the field");
        assert_eq!(atlas_of(&root), None);
        assert!(!set_atlas(&mut root, None), "nothing left to drop");
    }

    #[test]
    fn the_atlas_round_trips_through_text() {
        // What Save writes is what Load reads back.
        let mut root = ron_tree::parse("(segment: 1, image: \"p.png\")").expect("a small sidecar");
        set_atlas(&mut root, Some((3, 4)));
        let text = ron_tree::to_text_doc("", &root, "");
        let doc = ron_tree::parse_doc(&text).expect("re-parses");
        assert_eq!(atlas_of(&doc.root), Some((3, 4)));
    }

    #[test]
    fn the_atlas_array_is_not_a_position_spot() {
        // The grid's counts are not pixels: a two-entry array must not
        // read as a pickable position, the way a two-number tuple does.
        let root =
            ron_tree::parse("(atlas: [2, 3], lower: (317.0, 671.0))").expect("a small sidecar");
        let mut spots = Vec::new();
        scan_spots(&root, &mut Vec::new(), "", &mut spots);
        assert_eq!(
            spots.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(),
            ["lower"],
            "the array is invisible to the spot scan"
        );
    }

    #[test]
    fn the_atlas_restores_from_the_sidecar_on_load() {
        // The sprite reloads tiled the way it was saved: the sidecar's
        // `atlas` field reads into `Sprite.atlas` on the way in, and a
        // missing or broken field reads as no grid.
        let dir = std::env::temp_dir().join(format!("sprite_util_atlas_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("probe.png");
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
        std::fs::write(&png, png_bytes(&img).unwrap()).unwrap();
        assert_eq!(read_sprite(&png).unwrap().atlas, None, "no field, no grid");
        std::fs::write(dir.join("probe.ron"), "(segment: 1, atlas: [2, 3])").unwrap();
        assert_eq!(read_sprite(&png).unwrap().atlas, Some((2, 3)));
        std::fs::write(dir.join("probe.ron"), "(segment: 1, atlas: [0, 3])").unwrap();
        assert_eq!(
            read_sprite(&png).unwrap().atlas,
            None,
            "a broken field reads as no grid"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fresh_sidecar_holds_only_the_grid() {
        // The first save of a sprite that had no sidecar creates one:
        // it holds just the `atlas` field, and the grid reads back the
        // way a loaded file's does.
        let doc = new_sidecar("probe.ron".to_string(), (2, 3));
        assert_eq!(doc.name, "probe.ron");
        let fields = match &doc.root {
            ron_tree::Val::Struct { fields, .. } => fields,
            _ => panic!("a tuple-struct root"),
        };
        assert_eq!(
            fields.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            ["atlas"],
            "nothing but the grid"
        );
        assert!(!doc.rows.is_empty(), "the panel has rows to draw");
        let text = ron_tree::to_text_doc(&doc.header, &doc.root, &doc.trailer);
        let reparsed = ron_tree::parse(&text).expect("the fresh sidecar parses");
        assert_eq!(
            atlas_of(&reparsed),
            Some((2, 3)),
            "the grid survives the file round trip"
        );
    }

    #[test]
    fn the_canonical_sidecar_is_a_round_trip_fixed_point() {
        // The clean/dirty comparison compares canonical texts, so it is
        // only honest if canonicalizing is stable: the text Save writes
        // must re-parse to the very same text. Otherwise a freshly
        // loaded sprite would open dirty, or a save would never clear
        // its own mark.
        let fresh = new_sidecar("probe.ron".into(), (2, 3));
        let once = ron_tree::to_text_doc(&fresh.header, &fresh.root, &fresh.trailer);
        let back = ron_tree::parse_doc(&once).expect("the canonical text parses");
        let twice = ron_tree::to_text_doc(&back.header, &back.root, &back.trailer);
        assert_eq!(once, twice, "canonicalizing is idempotent");
    }

    #[test]
    fn comments_survive_the_canonical_round_trip() {
        // Header and trailer comments ride the same comparison: if the
        // rewrite dropped or reshaped them, the untouched file would
        // count as changed forever after the first save.
        let text = "// header note\n(\n    atlas: [1, 4],\n)\n// trailer note\n";
        let doc = ron_tree::parse_doc(text).expect("the commented sidecar parses");
        let once = ron_tree::to_text_doc(&doc.header, &doc.root, &doc.trailer);
        let back = ron_tree::parse_doc(&once).expect("the rewrite parses");
        let twice = ron_tree::to_text_doc(&back.header, &back.root, &back.trailer);
        assert_eq!(once, twice, "with comments, still idempotent");
        assert!(once.contains("header note") && once.contains("trailer note"));
    }

    #[test]
    fn band_cells_tile_the_rack_row() {
        // A 1 x 12 tileset is a flat left-aligned row above the
        // strip: exact edges hit, the gap between cells misses, and
        // a rest-placed panel starts at the desk's corner.
        let (w, h) = (1440.0, 810.0);
        let cell = band_cell(1, 12, w, h, 1.0);
        let o = band_origin_default(w, h);
        let rects = band_rects(1, 12, cell, o);
        assert_eq!(rects.len(), 12);
        let [x0, y0, x1, y1] = rects[0];
        assert!(band_pick([x0 + 1.0, (y0 + y1) / 2.0], 1, 12, cell, o, 4) == Some(0));
        let [nx0, ..] = rects[1];
        // The gap between cell 0 and cell 1 belongs to no cell.
        assert!(band_pick([x1 - 0.5, (y0 + y1) / 2.0], 1, 12, cell, o, 4) == Some(0));
        assert!(band_pick([(x1 + nx0) / 2.0, (y0 + y1) / 2.0], 1, 12, cell, o, 4).is_none());
        // A cell the panel doesn't show (only 4 of 12 painted) is
        // not clickable even where its box would sit.
        let [fx0, fy0, fx1, fy1] = rects[4];
        assert!(band_pick([(fx0 + fx1) / 2.0, (fy0 + fy1) / 2.0], 1, 12, cell, o, 4).is_none());
        assert!(band_pick([(fx0 + fx1) / 2.0, (fy0 + fy1) / 2.0], 1, 12, cell, o, 12) == Some(4));
        // Below the rack, the slots strip lives.
        assert!(y0 >= -h / 2.0 + STRIP_H);
    }

    #[test]
    fn the_rack_copies_the_tileset_grid() {
        // A 2 x 2 tileset is a matrix in the atlas' own reading
        // order: cell 1 right of cell 0, cell 2 below cell 0.
        let (w, h) = (1440.0, 810.0);
        let cell = band_cell(2, 2, w, h, 1.0);
        let o = band_origin_default(w, h);
        let rects = band_rects(2, 2, cell, o);
        assert_eq!(rects.len(), 4);
        let (a, b, c) = (rects[0], rects[1], rects[2]);
        assert!(b[0] >= a[2] && (b[1] - a[1]).abs() < 1e-3);
        assert!((c[0] - a[0]).abs() < 1e-3 && c[3] <= a[1]);
        let mid = |r: [f32; 4]| [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0];
        assert_eq!(band_pick(mid(b), 2, 2, cell, o, 4), Some(1));
        assert_eq!(band_pick(mid(c), 2, 2, cell, o, 4), Some(2));
        // A deep grid stays whole and inside: an 8 x 8 rack, cells
        // fitted to the window, clamps back into the desk.
        let big_cell = band_cell(8, 8, w, h, 1.0);
        let big_o = band_clamp([5000.0, 5000.0], big_cell, 8, 8, w, h);
        let big = band_rects(8, 8, big_cell, big_o);
        assert_eq!(big.len(), 64);
        let [px0, py0, px1, py1] = band_plate(&big, 64).unwrap();
        assert!(px0 >= -w / 2.0 && px1 <= w / 2.0);
        assert!(py0 >= -h / 2.0 + STRIP_H && py1 <= h / 2.0);
        // The panel's wheel is its own: the canvas never enters this
        // number, the cell simply scales.
        let doubled = band_cell(2, 2, w, h, 2.0);
        assert!((doubled - 2.0 * cell).abs() < 1e-3);
        // And the board covers the shown cells with its pad.
        let plate = band_plate(&rects, 4).unwrap();
        assert!((plate[0] - (a[0] - BAND_PAD)).abs() < 1e-3);
        assert!(band_plate(&rects, 0).is_none());
    }

    /// A stub sprite: a 4 x 2-pixel texture split 2 x 2, every cell a
    /// 2 x 1 rectangle — the sizes the layer tests quote.
    fn tileset(name: &str) -> Sprite {
        let img = image::RgbaImage::from_pixel(4, 2, image::Rgba([9, 9, 9, 255]));
        let png = png_bytes(&img).unwrap();
        Sprite {
            path: std::path::PathBuf::from(name),
            name: name.to_string(),
            current: img.clone(),
            saved_img: img.clone(),
            thumb_img: img,
            shape: frost::Shape::sprite_bytes_nearest(&png).unwrap(),
            thumb: frost::Shape::sprite_bytes_nearest(&png).unwrap(),
            ron: None,
            saved_ron: None,
            atlas: Some((2, 2)),
        }
    }

    #[test]
    fn the_transform_is_the_maps_memory() {
        // What a cell was painted with stays with the cell: the same
        // tile, unturned, is no change; the same tile turned is one
        // the road remembers; and the eraser takes the bits down
        // with the tile.
        let mut m = MapLayer::blank(std::path::PathBuf::from("t.png"), 4, 2);
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
        let mut m = MapLayer::blank(std::path::PathBuf::from("t.png"), 2, 2);
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

    /// A block builder for the tests: one tileset, rows then cols,
    /// row-major from the top line.
    fn block(rows: usize, cols: usize, cells: &[u32], tfms: &[u8]) -> Clip {
        Clip {
            tileset: std::path::PathBuf::from("x.png"),
            cols,
            rows,
            cells: cells.to_vec(),
            tfms: tfms.to_vec(),
        }
    }

    #[test]
    fn the_block_turns_as_one_picture() {
        // Two tiles side by side, the right one mirrored. Turn the
        // block clockwise and it stands as a column, the left tile on
        // top, and each tile wears exactly what a single-tile turn
        // would have dressed on it: the mirror keeps its bits (the
        // turn rides on top of the tile's own dress, as on the rack).
        let mut c = block(1, 2, &[1, 2], &[0, 1]);
        clip_turn(&mut c, true);
        assert_eq!((c.rows, c.cols), (2, 1));
        assert_eq!(c.cells, vec![1, 2]);
        assert_eq!(c.tfms, vec![4, 5]);
        // Back the other way and the block is exactly what it was —
        // the two turns are true inverses, tiles included.
        clip_turn(&mut c, false);
        assert_eq!((c.rows, c.cols), (1, 2));
        assert_eq!(c.cells, vec![1, 2]);
        assert_eq!(c.tfms, vec![0, 1]);
    }

    #[test]
    fn the_block_flips_along_either_axis() {
        let c = block(2, 2, &[1, 2, 3, EMPTY_CELL], &[0, 0, 1, 0]);
        let mut x = c.clone();
        clip_flip(&mut x, true);
        assert_eq!(x.cells, vec![2, 1, EMPTY_CELL, 3]);
        assert_eq!(x.tfms, vec![1, 1, 1, 0]);
        let mut y = c.clone();
        clip_flip(&mut y, false);
        assert_eq!(y.cells, vec![3, EMPTY_CELL, 1, 2]);
        assert_eq!(y.tfms, vec![3, 2, 2, 2]);
    }

    #[test]
    fn the_paste_anchors_the_block_on_the_cursor() {
        let mut maps = vec![MapLayer::blank(std::path::PathBuf::from("x.png"), 4, 4)];
        let c = block(2, 2, &[5, 5, 5, EMPTY_CELL], &[0, 0, 0, 0]);
        // Two across, two deep, one a gap: three cells land, the
        // anchor cell riding the cursor (gaps skip, never erase).
        // The map centers on the origin, so anchor the block near
        // home: cells ride (0,0) and its right and below.
        assert_eq!(clip_paste(&mut maps, &c, 0, 0), 3);
        let m = &maps[0];
        for cell in [m.at(0, 0), m.at(1, 0), m.at(0, 1)] {
            assert_eq!(m.cells[cell.expect("inside") as usize], 5);
        }
        // A second paste on the same spot changes nothing, and past
        // the map's edge the stamp is polite: nothing written, no
        // panic, nothing counted.
        assert_eq!(clip_paste(&mut maps, &c, 0, 0), 0);
        assert_eq!(clip_paste(&mut maps, &c, 9, 9), 0);
    }

    #[test]
    fn a_sloppy_frame_trims_to_the_picture() {
        // The repro from the wild: a 4x3 frame dragged around a
        // two-tile domino. The block must BE the domino — empty
        // margins only swing the picture away from every pivot.
        let mut c = block(
            3,
            4,
            &[
                EMPTY_CELL, EMPTY_CELL, EMPTY_CELL, EMPTY_CELL, EMPTY_CELL, 2, 3, EMPTY_CELL,
                EMPTY_CELL, EMPTY_CELL, EMPTY_CELL, EMPTY_CELL,
            ],
            &[0; 12],
        );
        assert_eq!(clip_trim(&mut c), Some((1, 1, 2, 1)));
        assert_eq!((c.cols, c.rows), (2, 1));
        assert_eq!(c.cells, vec![2, 3]);
        // An all-gap frame holds nothing to copy.
        let mut e = block(1, 2, &[EMPTY_CELL, EMPTY_CELL], &[0, 0]);
        assert_eq!(clip_trim(&mut e), None);
    }

    #[test]
    fn the_selection_copy_reads_one_tilesets_view() {
        let mut maps = vec![
            MapLayer::blank(std::path::PathBuf::from("a.png"), 4, 4),
            MapLayer::blank(std::path::PathBuf::from("b.png"), 4, 4),
        ];
        maps[0].set_cell(1, 1, 7, 5);
        maps[1].set_cell(1, 1, 9, 0);
        let c = clip_take(&maps, std::path::Path::new("a.png"), 0, 0, 3, 2);
        assert_eq!((c.rows, c.cols), (2, 3));
        // One tileset's view of the map: its own cell, not the layer
        // painted above it; EMPTY on gaps and past the edge.
        assert_eq!(c.cells[1 * 3 + 1], 7);
        assert_eq!(c.tfms[1 * 3 + 1], 5);
        assert_eq!(c.cells[0], EMPTY_CELL);
        let d = clip_take(&maps, std::path::Path::new("missing.png"), 0, 0, 2, 2);
        assert!(d.cells.iter().all(|c| *c == EMPTY_CELL));
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
            tileset: std::path::PathBuf::from("basic_tiles.png"),
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
                    tileset: std::path::PathBuf::from("t.png"),
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
            tileset: std::path::PathBuf::from("wild.png"),
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
        let mut maps = vec![MapLayer::blank(std::path::PathBuf::from("wild.png"), 8, 8)];
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
    fn a_move_cuts_the_frame_and_the_frame_travels() {
        let clip = Clip {
            tileset: std::path::PathBuf::from("wild.png"),
            cols: 3,
            rows: 2,
            cells: vec![1, 2, 3, 4, 5, EMPTY_CELL],
            tfms: vec![0, 0, 0, 0, 0, 0],
        };
        let mut maps = vec![MapLayer::blank(std::path::PathBuf::from("wild.png"), 8, 8)];
        for (c, r, cell) in [(-1, -1, 1), (0, -1, 2), (1, -1, 3), (-1, 0, 4), (0, 0, 5)] {
            maps[0].set_cell(c, r, cell, 0);
        }
        let (n, erased, sel) = clip_move(&mut maps, &clip, Some([-1, -1, 1, 0]), 2, 2);
        assert_eq!((n, erased), (5, true), "the cut lands and the drop lands");
        for (c, r) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (0, 0)] {
            assert_eq!(
                maps[0].at(c, r).map(|i| maps[0].cells[i]),
                Some(EMPTY_CELL),
                "the frame is swept at ({c},{r})"
            );
        }
        assert_eq!(
            maps[0].at(2, 2).map(|i| maps[0].cells[i]),
            Some(2),
            "the anchor cell rides to the cursor"
        );
        assert_eq!(sel, Some([1, 2, 3, 3]), "the frame travels with the drop");
    }

    #[test]
    fn a_move_without_a_frame_is_a_plain_drop() {
        let clip = Clip {
            tileset: std::path::PathBuf::from("wild.png"),
            cols: 3,
            rows: 2,
            cells: vec![1, 2, 3, 4, 5, EMPTY_CELL],
            tfms: vec![0, 0, 0, 0, 0, 0],
        };
        let mut maps = vec![MapLayer::blank(std::path::PathBuf::from("wild.png"), 8, 8)];
        let (n, erased, sel) = clip_move(&mut maps, &clip, None, 0, 0);
        assert_eq!((n, erased), (5, false), "nothing to cut, all to drop");
        assert_eq!(
            sel,
            Some([-1, 0, 1, 1]),
            "the block gets a frame to call home"
        );
    }

    #[test]
    fn the_frame_turns_with_the_block() {
        // Three wide, two deep: a quarter turn swaps the extents,
        // pivoting on the anchor cell (row 0, col 1 here) so the
        // frame keeps the cell the block itself turns on...
        let t = sel_turn([0, 0, 2, 1]);
        assert_eq!(t, [1, -1, 2, 1]);
        // ...and a second turn back lands exactly home.
        assert_eq!(sel_turn(t), [0, 0, 2, 1]);
        // A square frame pivots in place; a single cell does not
        // move at all.
        assert_eq!(sel_turn([1, -2, 2, -1]), [1, -2, 2, -1]);
        assert_eq!(sel_turn([3, 3, 3, 3]), [3, 3, 3, 3]);
    }

    #[test]
    fn the_selection_erase_sweeps_every_layer() {
        let mut maps = vec![
            MapLayer::blank(std::path::PathBuf::from("a.png"), 4, 4),
            MapLayer::blank(std::path::PathBuf::from("b.png"), 4, 4),
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

#[cfg(test)]
/// The block's dress, byte-for-byte honest: composite the block's
/// tiles into a real picture the same way the shader samples them,
/// transform THAT with the image crate (the honest rotation and
/// mirror), and rebuild the block with the demo's rules. The two
/// composites must match to the pixel — the test that pinned the
/// flip-conjugation rule after two algebraic guesses missed it.
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
            tileset: std::path::PathBuf::from("wild.png"),
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
