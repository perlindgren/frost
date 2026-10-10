//! The look: colors, text metrics, z-orders, and the fixed
//! scene-node pool ids.
// The subjects this one reads.
use crate::band::*;
use crate::spots::*;
use crate::strip::*;

pub(crate) const BG: frost::Color = frost::Color {
    r: 0.09,
    g: 0.09,
    b: 0.11,
    a: 1.0,
};

pub(crate) const MARKER: frost::Color = frost::Color {
    r: 1.0,
    g: 0.45,
    b: 0.4,
    a: 1.0,
};

/// The bounding box around the sprite.
pub(crate) const BBOX: frost::Color = frost::Color {
    r: 0.9,
    g: 0.9,
    b: 0.9,
    a: 1.0,
};

/// The crop selection's rectangle, and the active slot's frame.
pub(crate) const SELECT: frost::Color = frost::Color {
    r: 0.35,
    g: 0.72,
    b: 1.0,
    a: 1.0,
};

/// The authored `shapes:` of a sidecar, drawn over the sprite: a warm
/// amber that is none of the desk's other furniture — the bounding box
/// is white, the selection blue, the atlas grid green, and the position
/// markers cycle the palette.
pub(crate) const SHAPE: frost::Color = frost::Color {
    r: 1.0,
    g: 0.78,
    b: 0.22,
    a: 0.95,
};

/// The authored shapes' outline stroke: the bounding box's weight, so
/// the two read as one family of desk furniture.
pub(crate) const SHAPE_WIDTH: f32 = 2.0;

/// The tile-map desk's grid lines: one order above the checker and
/// below every shape, so painted tiles will sit on the grid, not in it.
pub(crate) const GRID_Z: f32 = -0.5;

/// The map's own axes: a touch louder than the grid, still beneath
/// everything paintable.
pub(crate) const AXIS: frost::Color = frost::Color {
    r: 0.62,
    g: 0.62,
    b: 0.68,
    a: 1.0,
};

pub(crate) const AXIS_Z: f32 = -0.4;

/// The atlas' tile grid: the sprite split into its rows and columns.
pub(crate) const TILE: frost::Color = frost::Color {
    r: 0.55,
    g: 0.95,
    b: 0.65,
    a: 1.0,
};

/// The slots strip's floor, darker than the window.
pub(crate) const STRIP: frost::Color = frost::Color {
    r: 0.055,
    g: 0.055,
    b: 0.07,
    a: 1.0,
};

/// An occupied slot's plate.
pub(crate) const PLATE: frost::Color = frost::Color {
    r: 0.13,
    g: 0.13,
    b: 0.16,
    a: 1.0,
};

/// A free slot's plate.
pub(crate) const PLATE_EMPTY: frost::Color = frost::Color {
    r: 0.075,
    g: 0.075,
    b: 0.095,
    a: 1.0,
};

/// The status band's fill: the window's bottommost row, and the log
/// panel's shelf when it stands.
pub(crate) const LOG_BAND: frost::Color = frost::Color {
    r: 0.085,
    g: 0.085,
    b: 0.105,
    a: 1.0,
};

/// The log plate: a shade above the band, a shade under the slot
/// plates — the panel is furniture, not a third voice.
pub(crate) const LOG_PLATE: frost::Color = frost::Color {
    r: 0.11,
    g: 0.11,
    b: 0.135,
    a: 1.0,
};

/// The messages' ink, and the thumbs' grey.
pub(crate) const LOG_TEXT: frost::Color = frost::Color {
    r: 0.78,
    g: 0.8,
    b: 0.86,
    a: 1.0,
};

pub(crate) const LOG_THUMB: frost::Color = frost::Color {
    r: 0.3,
    g: 0.3,
    b: 0.37,
    a: 1.0,
};

/// The bounding box's line width, in pixels.
pub(crate) const BBOX_WIDTH: f32 = 2.0;

/// The tile grid's line width, in pixels: thinner than the box, so the
/// grid reads as divisions, not frames.
pub(crate) const TILE_WIDTH: f32 = 1.0;

/// The bounding box's order: above the HUD, below the click marker.
pub(crate) const BBOX_Z: f32 = 1.5;

/// The tile grid's order: above the bounding box, below the crop
/// selection, so a selection still reads over the grid.
pub(crate) const TILE_Z: f32 = 1.6;

/// The selection's order: above the bounding box, below the click marker.
pub(crate) const SELECT_Z: f32 = 1.8;

/// The authored shapes' order: above the crop selection, below the
/// position markers — an overlay of the sprite's own furniture, never
/// above the things the author is editing.
pub(crate) const SHAPE_Z: f32 = 2.0;

/// The checker backdrop's order: behind the sprite and the HUD.
pub(crate) const CHECK_ORDER: f32 = -1.0;

/// The slots strip's fill order: above the checker, below the HUD.
pub(crate) const STRIP_Z: f32 = 0.2;

/// A slot plate's order: above its strip, below the HUD.
pub(crate) const PLATE_Z: f32 = 0.3;

/// The active slot's frame order: above the thumbnails.
pub(crate) const FRAME_Z: f32 = 0.9;

/// The slot thumbnails' order: above the strip and its plates.
pub(crate) const THUMB_ORDER: f32 = 0.6;

/// The order of a thumbnail being dragged between slots: above the HUD.
pub(crate) const DRAG_ORDER: f32 = 3.0;

pub(crate) const LAYER_NODES: usize = 8;

/// The scene's fixed child indices: the layer pool starts at 0, then
/// the help line, the marker, the checker and the slot thumbnails.
pub(crate) const HELP: usize = LAYER_NODES;

pub(crate) const MARKER_NODE: usize = LAYER_NODES + 1;

pub(crate) const CHECKER: usize = LAYER_NODES + 2;

pub(crate) const THUMBS: usize = LAYER_NODES + 3;

/// The sidecar trees' order: they paint in the shared UI font, at 15 000
/// over the panel plates — the UI counts up from its 10 000 base, and
/// the trees must sit on top of their own plates.
pub(crate) const RON_ORDER: f32 = 15_000.0;

/// The log bar's order: over every panel the UI paints and over the
/// sidecar trees — the docked log wins the pixels it stands on.
pub(crate) const LOG_Z: f32 = 15_400.0;

/// The position markers' pool: two circles per position — the colour
/// dot and its white centre — ordered above the sprite, its box and the
/// click marker, and under the UI's panel plate.
/// The band's cell nodes: after every node the sprite desks own, and
/// before the single ghost node that completes the pool.
pub(crate) const BAND0: usize = SPOTT + SPOTS_MAX;

pub(crate) const GHOST: usize = BAND0 + BAND_CELLS;

/// The plate's corner chip: the brush as it stands — the picked
/// cell's art wearing the brush's transform.
pub(crate) const HELD: usize = BAND0 + BAND_CELLS + 1;

pub(crate) const SPOTS: usize = THUMBS + SLOTS;

pub(crate) const SPOTT: usize = SPOTS + 2 * SPOTS_MAX;

/// Maps draw above the desk's grid, below the band and the ghost.
pub(crate) const MAP_Z: f32 = 1.0;

/// The position palette: a marker and the tree rows describing it share
/// a colour, cycling through these hues.
pub(crate) const PALETTE: [(f32, f32, f32); 8] = [
    (0.95, 0.45, 0.42),
    (0.98, 0.72, 0.35),
    (0.93, 0.87, 0.45),
    (0.55, 0.85, 0.50),
    (0.45, 0.85, 0.82),
    (0.52, 0.66, 0.97),
    (0.90, 0.58, 0.92),
    (0.75, 0.90, 0.55),
];
