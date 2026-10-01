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
//! The first dialog is a child of the window, so it opens on top of it:
//! the window is created first and the dialog is parented to it on the
//! first frame (a dialog without a parent opens at the screen's default
//! spot, far from the window). It is modal, so the frame loop blocks
//! until a file is picked; canceling quits.
//!
//! The controls are two draggable [`frost::Ui`] panels (drag one by its
//! title bar, click the bar to fold it away). The "View" panel is laid out
//! as a [`frost::Ui::table`]: three rows of `[label | track | readout]`
//! for the zoom and the checker's two grey levels, above the last-click
//! line. The table gives each column a policy — the label column hugs its
//! text and aligns left, the track column stretches to fill the row, and
//! the value readout hugs and centers — so the labels and values line up
//! down the panel while the tracks soak up the slack.
//!
//! The "Operations" panel edits the active sprite: **Open** (**Ctrl-O**)
//! adds another sprite to the next free slot — while the slots fill up,
//! the newest sprite becomes the active one; **Crop** cuts the texture to
//! the selection rectangle if one is dragged (a left-drag over the work
//! area draws it; a click that barely moves instead logs the pixel and
//! its slot) — and to the opaque content when there is no selection,
//! trimming the fully transparent borders. Every crop is remembered per
//! sprite, so **Ctrl-Z** undoes it, walking back step by step to that
//! sprite's original. **Save** writes the texture over the file it came
//! from, and **Save As** asks a native dialog for a new name, defaulted
//! to the sprite's current file name; both ask for confirmation before
//! overwriting an existing file. **Close** takes the active sprite out of
//! its slot, and every later slot shifts left to fill the gap.
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
//! The wheel zooms about the cursor (the point under it stays put), a
//! right-drag grabs the work area and moves the sprite with the cursor,
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
//! dialog, and a folder is where the file dialog opens — a relative path
//! is expanded against the working directory and resolved to an absolute
//! one, because the native dialog (notably on Windows) ignores anything
//! else and opens in its default spot. Without the argument, the dialog
//! opens in the folder the example was launched from:
//!
//! ```text
//! cargo run --example sprite_util -- -i assets/sprites/bird1.png
//! cargo run --example sprite_util -- -i assets/sprites
//! ```

//! A sprite may bring a sidecar: when `<name>.png` has a `<name>.ron`
//! beside it, the file — the RON subset `ron_view` speaks, parsed by the
//! very same `tree.rs`, shared by path-include — opens as a foldable
//! tree in a `Ui` panel titled by the file name, so it drags and folds
//! like every other panel. Click a `[+]/[-]` row to fold or open its
//! node, and the wheel scrolls the rows while the cursor is over the
//! body (everywhere else it keeps zooming). No sidecar, an unparseable
//! one, or a closed slot: the sprite simply loads, and closing a sprite
//! takes its panel with it.

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

/// The command line arguments.
#[derive(Parser, Debug)]
#[command(name = "sprite_util")]
struct Args {
    /// A PNG file to open immediately, skipping the file dialog — or a
    /// folder, in which case the file dialog opens there (a relative
    /// path is expanded against the working directory). Without the
    /// argument, the dialog opens in the working directory.
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

/// The checker's default light grey level, 0.0-1.0.
const GREY_LIGHT: f32 = 0.95;

/// The checker's default dark grey level, 0.0-1.0.
const GREY_DARK: f32 = 0.65;

/// How far the left button may travel between press and release and still
/// count as a click (select a slot, or log a pixel) rather than a drag
/// (a selection rectangle in the work area, or a slot swap).
const CLICK_TOL: f32 = 4.0;

/// The number of sprite slots along the bottom.
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

/// The bounding box's order: above the HUD, below the click marker.
const BBOX_Z: f32 = 1.5;

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
const RON_LINE: f32 = 20.0;
const RON_W: f32 = 300.0;
const RON_VIEW_H: f32 = 240.0;
const RON_PAD: f32 = 6.0;
const RON_INSET: f32 = 12.0;
const RON_ROWS: usize = 16;

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
/// The sidecar tree's pool: two text nodes per row. They draw at 15 000
/// — the UI counts up from its 10 000 base, and the rows must sit on
/// top of the panel's plate.
const RON: usize = THUMBS + SLOTS;
const RON_NODES: usize = RON_ROWS * 2;
const RON_ORDER: f32 = 15_000.0;

/// One sprite: its files, its textures and its slot's look.
struct Sprite {
    /// The file the sprite was loaded from — Save writes back to it.
    path: std::path::PathBuf,
    /// The file's name, for the HUD, the dialogs and the logs.
    name: String,
    /// The working texture: crops replace it, saves write it.
    current: image::RgbaImage,
    /// The texture snapshots each crop replaced: Ctrl-Z pops them back,
    /// and the history bottom is the sprite's original.
    history: Vec<image::RgbaImage>,
    /// The working texture as a shape — the work area's sprite node.
    shape: frost::Shape,
    /// The original, minimized to a slot's width — the slot's picture.
    /// It never changes: a slot shows the sprite as it was loaded.
    thumb: frost::Shape,
    /// The parsed sidecar `<name>.ron`, `None` when the sprite has none
    /// or the file does not parse. It lives and dies with the sprite:
    /// closing the slot drops the panel with it.
    ron: Option<RonDoc>,
}

/// A sprite's parsed sidecar: the tree with its fold flags, the
/// flattened visible rows, and the panel's scroll and fold state.
struct RonDoc {
    /// The sidecar's file name, for the panel's title.
    name: String,
    /// The parsed tree — the containers' fold flags live here.
    root: ron_tree::Val,
    /// The visible rows, rebuilt whenever a fold flips.
    rows: Vec<ron_tree::Row>,
    /// The scroll offset in pixels, 0 at the tree's top.
    scroll: f32,
}

impl RonDoc {
    /// Re-pin the scroll into the panel's content height.
    fn clamp_scroll(&mut self, view_h: f32) {
        self.scroll = self.scroll.clamp(
            0.0,
            (self.rows.len() as f32 * RON_LINE + RON_PAD - view_h).max(0.0),
        );
    }
}

/// One picture within a frame: the active sprite's shape, snapshotted
/// when the layer was added — later crops and closed slots leave the
/// frame's layers untouched.
struct Layer {
    shape: frost::Shape,
}

/// One animation frame: a set of layers, stacked as added, and the time
/// it holds before the next frame.
struct Frame {
    layers: Vec<Layer>,
    next_time: f32,
}

/// The animation: an ordered set of frames kept by a clock. Looping
/// playback wraps at the ends; ping-pong walks back through the frames
/// instead, each end held once per sweep.
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
    /// pixels; set by right-dragging and by the wheel's anchor.
    offset: [f32; 2],
    /// The crop selection in texture pixels, `[x0, y0, x1, y1]` (y down):
    /// dragged with the left button, consumed by Crop.
    selection: Option<[f32; 4]>,
    /// Where the current work-area press began, in window pixels — only
    /// set when the press was the work area's, not the UI's or a slot's.
    drag_from: Option<[f32; 2]>,
    /// The slot being dragged between slots, and where its press began.
    slot_drag: Option<(usize, [f32; 2])>,
    /// Whether Ctrl-Z was held last frame: its rising edge is the undo.
    was_undo: bool,
    /// Whether Ctrl-O was held last frame: its rising edge is the open.
    was_open: bool,
    /// Whether space was held last frame: its rising edge toggles play.
    was_space: bool,
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
    /// The cursor's position on the previous frame, for the right-drag's
    /// frame-to-frame pan delta.
    last_mouse: Option<[f32; 2]>,
    /// The last reported click: its position in the active texture's
    /// pixel space, and whether the spot was inside the texture.
    last_click: Option<([f32; 2], bool)>,
    /// The sidecar rows' font bytes: one Arc's worth of FiraCode, each
    /// row's shapes cloning the pointer and never the file.
    ron_font: Arc<[u8]>,
    /// The sidecar panel's in-progress press, for the click test.
    ron_press: Option<[f32; 2]>,
    /// The open panel body's viewport rectangle — the rows' clip box,
    /// the wheel's and row clicks' hit area — as the UI laid the panel
    /// out; `None` while the panel is folded or there is no sidecar.
    ron_body: Option<[f32; 4]>,
    /// The folder the first file dialog opens in, still to pick: `Some`
    /// until the first frame has shown the dialog. A picked file loads
    /// into the first slot; canceling quits the program.
    pending: Option<std::path::PathBuf>,
    /// The folder the next dialog opens in: where the last file was
    /// read from or written to.
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
    fn ron_text(&self, text: String, weight: f32, color: frost::Color) -> frost::Shape {
        frost::Shape::Text {
            text,
            font: Arc::clone(&self.ron_font),
            size: RON_SIZE,
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

    /// Repaint the work area's layer pool: the animation's current frame
    /// when any frame exists (so the panel previews what it edits), else
    /// the active sprite alone in the first node.
    fn sync_work(&self, ctx: &mut frost::Context) {
        let frame = if self.anim.frames.is_empty() {
            None
        } else {
            Some(&self.anim.frames[self.anim.frame.min(self.anim.frames.len() - 1)])
        };
        for i in 0..LAYER_NODES {
            let node = &mut ctx.scene().root.children[i];
            node.shape = match &frame {
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
        let shape = frost::Shape::sprite_bytes(&png).ok()?;
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
        if self.sprites.len() >= SLOTS {
            self.status = format!("open: all {SLOTS} slots are full");
            return;
        }
        match read_sprite(&path) {
            Err(err) => {
                log::warn!("open failed: {err}");
                self.status = format!("open: {err}");
            }
            Ok(sp) => {
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
        if self.sprites.len() >= SLOTS {
            self.status = format!("open: all {SLOTS} slots are full");
            return;
        }
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
    /// content — trimming the fully transparent borders. The replaced
    /// texture goes on the sprite's history, so undo can walk it back.
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
        let cut = match rect {
            Some((x, y, cw, ch)) => image::imageops::crop_imm(&sp.current, x, y, cw, ch).to_image(),
            None => {
                if let Some(err) = err {
                    self.status = err;
                }
                return;
            }
        };
        let (nx, ny) = (cut.width(), cut.height());
        let sp = &mut self.sprites[self.active];
        sp.history.push(sp.current.clone());
        if self
            .apply(ctx, cut, format!("cropped to {nx} x {ny} px"))
            .is_none()
        {
            self.sprites[self.active].history.pop();
            self.status = String::from("crop: could not rebuild the sprite");
        }
    }

    /// One step back through the active sprite's crops; the history ends
    /// at its original.
    fn undo(&mut self, ctx: &mut frost::Context) {
        let prev = self
            .sprites
            .get_mut(self.active)
            .and_then(|sp| sp.history.pop());
        match prev {
            Some(prev) => {
                let (w, h) = (prev.width(), prev.height());
                if self
                    .apply(ctx, prev, format!("undo: back to {w} x {h} px"))
                    .is_none()
                {
                    self.status = String::from("undo: could not rebuild the sprite");
                }
            }
            None => self.status = String::from("undo: already at the original"),
        }
    }

    /// Close: take the active sprite out of its slot; every later sprite
    /// shifts one slot left, and the slot the active one leaves (or the
    /// new last one, if it was last) becomes active.
    fn close_active(&mut self, ctx: &mut frost::Context) {
        if self.sprites.is_empty() {
            return;
        }
        let name = self.sprites.remove(self.active).name;
        self.active = self.active.min(self.sprites.len().saturating_sub(1));
        self.selection = None;
        self.last_click = None;
        self.status = format!("closed '{name}'");
        self.sync_work(ctx);
        self.refresh_slots(ctx);
    }

    /// Save: write the active texture over the file it was loaded from,
    /// asking first — that file exists, by definition of overwriting it.
    fn save(&mut self, ctx: &mut frost::Context) {
        let Some(sp) = self.active() else { return };
        if sp.current.width() == 0 {
            return;
        }
        if sp.path.as_os_str().is_empty() {
            self.status = String::from("save: no file to overwrite — use save as");
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
    /// file — the name and the Save target both follow. The slot keeps
    /// showing the original: a slot is a sprite's identity, not its edit.
    fn write_png(&mut self, path: &std::path::Path, name: String) {
        let current = match self.active() {
            Some(sp) => sp.current.clone(),
            None => return,
        };
        let done = png_bytes(&current)
            .and_then(|png| std::fs::write(path, png).map_err(|e| e.to_string()));
        match done {
            Ok(()) => {
                log::info!("saved '{name}'");
                if let Some(sp) = self.sprites.get_mut(self.active) {
                    sp.path = path.to_path_buf();
                    sp.name = name.clone();
                }
                self.dir = path
                    .parent()
                    .filter(|d| !d.as_os_str().is_empty())
                    .map(std::path::PathBuf::from);
                self.status = format!("saved '{name}'");
            }
            Err(err) => self.status = format!("save failed: {err}"),
        }
    }
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // When no file was given on the command line, pick one in a
        // native file dialog — on the first frame, now that the window
        // exists. The dialog is a child of the window, so it opens on top
        // of it (a parentless dialog would open at the screen's default
        // spot), and it is modal: the loop blocks here until the user
        // picks a file or cancels.
        if let Some(dir) = &self.pending {
            let mut dialog = rfd::FileDialog::new()
                .set_title("sprite_util: choose a sprite")
                .set_directory(dir)
                .add_filter("PNG images", &["png"]);
            if let Some(window) = ctx.window() {
                dialog = dialog.set_parent(window);
            }
            let picked = dialog.pick_file();
            self.pending = None;
            let Some(path) = picked else {
                log::warn!("no file selected — quitting");
                std::process::exit(0);
            };
            // Load the pick into the first slot; the first interactive
            // frame is the next one.
            self.load_sprite(ctx, path);
            return;
        }

        // The animation clock runs on wall time; when it rolls to a new
        // frame, the work area repaints its layer pool to that frame.
        if self.anim.tick(dt) {
            self.sync_work(ctx);
        }

        let (w, h) = ctx.size();
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let rdown = ctx.mouse_button_down(frost::MouseButton::Right);
        let pressed = down && !self.was_down;
        let released = !down && self.was_down;
        self.was_down = down;
        let pos = ctx.mouse_position();
        let size = self.work_size();
        // The active sprite's center in window coordinates: the work
        // area's center plus the pan offset.
        let view = [self.offset[0], self.offset[1] + WORK_Y];

        // The sidecar tree re-pins its scroll every frame, so folding
        // the panel, shrinking the window or folding away a node can
        // never strand the view past the rows. The body rectangle the
        // wheel and the pan guard read is the panel's from LAST frame;
        // row clicks read the fresh one, laid out below.
        if let Some(doc) = self.ron_mut() {
            doc.clamp_scroll(RON_VIEW_H);
        }
        let over_ron = pos.is_some_and(|p| self.ron_body.is_some_and(|b| in_rect(b, p)));

        // The wheel zooms: multiplicative per line (up zooms in), clamped
        // to the slider's range, anchored so the texture point under the
        // cursor stays put — the offset absorbs the scale change the cursor
        // itself would have drifted. With the cursor outside the window the
        // anchor is the work area's center, so only the zoom changes.
        let wheel = ctx.mouse_wheel();
        // The wheel scrolls the sidecar tree while the cursor rests on
        // its body; anywhere else it keeps zooming.
        let mut scrolled = false;
        if wheel != 0.0
            && over_ron
            && let Some(doc) = self.ron_mut()
        {
            doc.scroll -= wheel * RON_LINE * 2.0;
            doc.clamp_scroll(RON_VIEW_H);
            scrolled = true;
        }
        if wheel != 0.0 && !scrolled {
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

        // A right-drag grabs the work area: the sprite follows the
        // cursor's frame-to-frame movement, so it always lands under the
        // pointer.
        if rdown
            && !over_ron
            && let (Some([lx, ly]), Some([mx, my])) = (self.last_mouse, pos)
        {
            self.offset[0] += mx - lx;
            self.offset[1] += my - ly;
        }

        // The UI frame: the View panel's sliders read and write the demo's
        // values, and both panels' bodies claim the mouse over them — a
        // press the UI holds is never a selection drag, a slot touch or a
        // sprite click.
        self.ui.begin(ctx);
        let click_label = match &self.last_click {
            Some(([px, py], inside)) => format!(
                "click: ({px:.1}, {py:.1}) {}",
                if *inside { "inside" } else { "outside" }
            ),
            None => String::from("click: none"),
        };
        self.ui.panel(
            ctx,
            "View",
            [-w / 2.0 + PANEL_W / 2.0 + 20.0, h / 2.0 - 120.0],
            PANEL_W,
            |ui, ctx| {
                // One table, three rows of [label | track | readout]: the
                // label column hugs its text and sits left, the track
                // column stretches to fill, the readout hugs and centers.
                ui.table(
                    ctx,
                    "view",
                    &[
                        frost::Col::auto(frost::Align::Left),
                        frost::Col::stretch(1.0, frost::Align::Left),
                        frost::Col::auto(frost::Align::Center),
                    ],
                    |ui, ctx| {
                        ui.label(ctx, "zoom");
                        ui.slider_track(ctx, "zoom", &mut self.zoom, ZOOM_MIN, ZOOM_MAX);
                        ui.readout(ctx, &format!("{:.2}", self.zoom));
                        ui.label(ctx, "light");
                        ui.slider_track(ctx, "light", &mut self.light, 0.0, 1.0);
                        ui.readout(ctx, &format!("{:.2}", self.light));
                        ui.label(ctx, "dark");
                        ui.slider_track(ctx, "dark", &mut self.dark, 0.0, 1.0);
                        ui.readout(ctx, &format!("{:.2}", self.dark));
                    },
                );
                ui.space(6.0);
                ui.label(ctx, &click_label);
            },
        );
        // The Operations panel: which buttons were pressed is collected
        // here and acted on after the panels, so the modal dialogs and the
        // texture edits never run inside a panel's layout closure.
        let mut want_open = false;
        let mut want_crop = false;
        let mut want_save = false;
        let mut want_save_as = false;
        let mut want_close = false;
        let title = match self.active() {
            Some(sp) if !sp.history.is_empty() => {
                format!("{} *  ·  slot {}/{}", sp.name, self.active + 1, SLOTS)
            }
            Some(sp) => format!("{}  ·  slot {}/{}", sp.name, self.active + 1, SLOTS),
            None => format!("no sprite  ·  0/{SLOTS}"),
        };
        let ops_line = if self.status.is_empty() {
            match self.selection {
                Some([x0, y0, x1, y1]) => {
                    format!("selection {:.0} x {:.0} px", x1 - x0, y1 - y0)
                }
                None => String::from("drag to select, or crop trims"),
            }
        } else {
            self.status.clone()
        };
        self.ui.panel(
            ctx,
            "Operations",
            [-w / 2.0 + PANEL_W / 2.0 + 20.0, h / 2.0 - 330.0],
            PANEL_W,
            |ui, ctx| {
                ui.label(ctx, &title);
                if ui.button(ctx, "open") {
                    want_open = true;
                }
                if ui.button(ctx, "crop") {
                    want_crop = true;
                }
                if ui.button(ctx, "save") {
                    want_save = true;
                }
                if ui.button(ctx, "save as") {
                    want_save_as = true;
                }
                if ui.button(ctx, "close") {
                    want_close = true;
                }
                ui.space(6.0);
                ui.label(ctx, &ops_line);
                ui.label(ctx, "ctrl-o open  ·  ctrl-z undo");
            },
        );

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

        // Ctrl-O's and Ctrl-Z's rising edges: the open dialog, and one
        // step back through the active sprite's crops.
        let ctrl =
            ctx.key_down(frost::KeyCode::ControlLeft) || ctx.key_down(frost::KeyCode::ControlRight);
        let open_key = ctrl && ctx.key_down(frost::KeyCode::KeyO);
        let undo_key = ctrl && ctx.key_down(frost::KeyCode::KeyZ);
        if open_key && !self.was_open {
            self.open_dialog(ctx);
        }
        if undo_key && !self.was_undo {
            self.undo(ctx);
        }
        self.was_open = open_key;
        self.was_undo = undo_key;

        // Space's rising edge starts and stops the animation clock.
        let space = ctx.key_down(frost::KeyCode::Space);
        if space && !self.was_space {
            self.anim.playing = !self.anim.playing;
            self.anim.dir = 1;
        }
        self.was_space = space;

        // The sidecar panel: a `Ui` panel titled by the sidecar's file
        // name, so its bar drags the panel around and folds it whole —
        // like the View and Animation panels. The body is one empty
        // viewport; the tree's rows paint into it below.
        if let Some(title) = self.ron().map(|doc| doc.name.clone()) {
            let (row_h, pad) = {
                let st = self.ui.style_mut();
                (st.row_h, st.pad)
            };
            let mut open = false;
            self.ui.panel(
                ctx,
                &title,
                ron_default(w, h, row_h, pad),
                RON_W,
                |ui, _ctx| {
                    ui.space(RON_VIEW_H);
                    open = true;
                },
            );
            // The body viewport's rectangle, from the panel's position
            // as laid out this frame; a folded panel has none.
            self.ron_body = if open {
                self.ui.panel_position(&title).map(|pc| {
                    let top = pc[1] + (row_h + 2.0 * pad + RON_VIEW_H) / 2.0 - row_h - pad;
                    [
                        pc[0] - RON_W / 2.0,
                        top - RON_VIEW_H,
                        pc[0] + RON_W / 2.0,
                        top,
                    ]
                })
            } else {
                None
            };
        } else {
            self.ron_body = None;
        }

        // The collected button presses, now outside every panel closure.
        if want_open {
            self.open_dialog(ctx);
        }
        if want_crop {
            self.crop(ctx);
        }
        if want_close {
            self.close_active(ctx);
        }
        if want_save {
            self.save(ctx);
        }
        if want_save_as {
            self.save_as(ctx);
        }

        // The animation edits. The duration slider and loop checkbox
        // wrote their locals; commit them, then act on the buttons.
        if let Some(f) = self.anim.frames.get_mut(self.anim.frame) {
            f.next_time = next_time;
        }
        self.anim.looping = looping;
        if want_add_frame {
            // A new frame, inserted after the one being edited, opens
            // with the active sprite as its first layer.
            let at = (self.anim.frame + 1).min(self.anim.frames.len());
            let shape = self.active().map(|sp| sp.shape.clone());
            self.anim.frames.insert(
                at,
                Frame {
                    layers: shape.map(|s| vec![Layer { shape: s }]).unwrap_or_default(),
                    next_time: TIME_DEFAULT,
                },
            );
            self.anim.set_frame(at);
            self.sync_work(ctx);
        }
        if want_add_layer {
            let shape = self.active().map(|sp| sp.shape.clone());
            if let Some(shape) = shape
                && let Some(f) = self.anim.frames.get_mut(self.anim.frame)
                && f.layers.len() < LAYER_NODES
            {
                f.layers.push(Layer { shape });
            }
            self.sync_work(ctx);
        }
        if want_del_layer {
            if let Some(f) = self.anim.frames.get_mut(self.anim.frame) {
                f.layers.pop();
            }
            self.sync_work(ctx);
        }
        if want_del_frame {
            if self.anim.frame < self.anim.frames.len() {
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

        // The left button's work: press lands on a slot, on the work area
        // or nowhere the demo owns; release decides — click or drag.
        if pressed
            && !self.ui.hovering()
            && let Some(p) = pos
        {
            match slot_at(p, w, h) {
                Some(i) => self.slot_drag = Some((i, p)),
                None if in_work_area(p, w, h) => self.drag_from = Some(p),
                None => {}
            }
        }
        // The tree rows live over the panel body, which the UI claims —
        // so their press rides the raw mouse, beside the widget layer.
        if pressed
            && let Some(p) = pos
            && self.ron_body.is_some_and(|b| in_rect(b, p))
        {
            self.ron_press = Some(p);
        }
        // A work-area drag draws the crop selection in the texture's
        // pixel space.
        if down && let (Some(from), Some(p)) = (self.drag_from, pos) {
            self.selection = sel_rect_from(from, p, size, view, self.zoom);
        }
        if released {
            // The sidecar tree's still-click: a row carrying a `[+]/[-]`
            // marker folds its node, and the visible rows are
            // re-flattened from the tree. Folding the panel whole is
            // the UI's own title-bar click.
            if let Some(from) = self.ron_press.take()
                && let Some(p) = pos
                && ((p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2)).sqrt() < CLICK_TOL
                && let Some(body) = self.ron_body
                && let Some(doc) = self.ron_mut()
                && let Some(i) = ron_row_at(p, body, doc.scroll, doc.rows.len())
                && doc.rows[i].fold.is_some()
            {
                let path = doc.rows[i].path.clone();
                if let Some(v) = ron_tree::walk(&mut doc.root, &path) {
                    v.toggle();
                }
                doc.rows = ron_tree::layout(&doc.root);
                doc.clamp_scroll(RON_VIEW_H);
            }
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
                    self.active = from_slot;
                    self.selection = None;
                    self.last_click = None;
                    self.sync_work(ctx);
                } else if let Some(to) = slot_at(p, w, h)
                    && to != from_slot
                {
                    self.sprites.swap(from_slot, to);
                    self.active = swapped_active(self.active, from_slot, to);
                    self.refresh_slots(ctx);
                }
            }
        }

        // The work area's layer pool: every node carries the pan and the
        // zoom — a uniform scale about its center, then the pan offset
        // from the work area's center. The shapes come from the sync.
        for node in &mut ctx.scene().root.children[..LAYER_NODES] {
            node.transform = frost::Transform::translate(view);
            node.scale = [self.zoom, self.zoom];
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
        // The box's intersection with the work area.
        let (rx0, ry0, rx1, ry1) = (
            bx0.max(-w / 2.0),
            by0.max(-h / 2.0 + STRIP_H),
            bx1.min(w / 2.0),
            by1.min(h / 2.0),
        );
        let checker = &mut ctx.scene().root.children[CHECKER];
        if rx1 > rx0 && ry1 > ry0 && tw > 0.0 {
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

        // The bounding box: the sprite's full on-screen rectangle; the
        // parts outside the window are clipped by the GPU.
        if tw > 0.0 && th > 0.0 {
            ctx.line(bx0, by0, bx1, by0, BBOX, BBOX_WIDTH, BBOX_Z);
            ctx.line(bx1, by0, bx1, by1, BBOX, BBOX_WIDTH, BBOX_Z);
            ctx.line(bx1, by1, bx0, by1, BBOX, BBOX_WIDTH, BBOX_Z);
            ctx.line(bx0, by1, bx0, by0, BBOX, BBOX_WIDTH, BBOX_Z);
        }

        // The crop selection: the texture-space rectangle mapped back to
        // the window — the same transform the marker dot uses.
        if let Some([x0, y0, x1, y1]) = self.selection {
            let wx = |px: f32| (px - tw / 2.0) * self.zoom + view[0];
            let wy = |py: f32| (th / 2.0 - py) * self.zoom + view[1];
            let (sx0, sx1) = (wx(x0), wx(x1));
            let (sy0, sy1) = (wy(y0), wy(y1));
            ctx.line(sx0, sy0, sx1, sy0, SELECT, BBOX_WIDTH, SELECT_Z);
            ctx.line(sx1, sy0, sx1, sy1, SELECT, BBOX_WIDTH, SELECT_Z);
            ctx.line(sx1, sy1, sx0, sy1, SELECT, BBOX_WIDTH, SELECT_Z);
            ctx.line(sx0, sy1, sx0, sy0, SELECT, BBOX_WIDTH, SELECT_Z);
        }

        // The slots strip: its floor, each slot's plate, and the active
        // slot's frame. The thumbnails are scene nodes, positioned below.
        ctx.rectangle(
            0.0,
            -h / 2.0 + STRIP_H / 2.0,
            w / 2.0,
            STRIP_H / 2.0,
            STRIP,
            STRIP_Z,
        );
        for i in 0..SLOTS {
            let [cx, cy] = slot_center(i, w, h);
            let plate = if i < self.sprites.len() {
                PLATE
            } else {
                PLATE_EMPTY
            };
            ctx.rectangle(cx, cy, SLOT / 2.0, SLOT / 2.0, plate, PLATE_Z);
            if i == self.active && i < self.sprites.len() {
                let r = SLOT / 2.0;
                ctx.line(cx - r, cy - r, cx + r, cy - r, SELECT, BBOX_WIDTH, FRAME_Z);
                ctx.line(cx + r, cy - r, cx + r, cy + r, SELECT, BBOX_WIDTH, FRAME_Z);
                ctx.line(cx + r, cy + r, cx - r, cy + r, SELECT, BBOX_WIDTH, FRAME_Z);
                ctx.line(cx - r, cy + r, cx - r, cy - r, SELECT, BBOX_WIDTH, FRAME_Z);
            }
        }

        // The slot thumbnails: minimized originals, centered on their
        // plates. A dragged thumbnail rides the cursor, above everything
        // until it lands.
        for i in 0..SLOTS {
            let node = &mut ctx.scene().root.children[THUMBS + i];
            let dragging = self.slot_drag.is_some_and(|(slot, _)| slot == i);
            node.order = if dragging { DRAG_ORDER } else { THUMB_ORDER };
            node.transform = match (dragging, pos) {
                (true, Some(p)) => frost::Transform::translate(p),
                _ => frost::Transform::translate(slot_center(i, w, h)),
            };
        }

        // --- The sidecar tree ------------------------------------------
        // The visible rows of the open panel body: keys at weight 700,
        // values at 500, the ron_view palette — drawn at 15 000, right
        // over the UI's plate. A folded or absent panel draws nothing
        // and the pool rests.
        let mut drawn = 0usize;
        let adv = RON_SIZE * ADVANCE_EM;
        if let (Some(doc), Some(body)) = (self.ron(), self.ron_body) {
            let budget = (RON_W - 2.0 * RON_INSET) / adv;
            let first = (doc.scroll / RON_LINE).floor().max(0.0) as usize;
            for (k, i) in (first..doc.rows.len()).take(RON_ROWS).enumerate() {
                let row = &doc.rows[i];
                let y = body[3] - RON_PAD - (i as f32 + 0.5) * RON_LINE + doc.scroll;
                let nodes = &mut ctx.scene().root.children;
                if y < body[1] || y > body[3] {
                    // Out of the viewport — half clipped at the bottom,
                    // scrolled past the top: not this frame's picture.
                    nodes[RON + k * 2].shape = None;
                    nodes[RON + k * 2 + 1].shape = None;
                    drawn = k + 1;
                    continue;
                }
                let key = fit(&row.key, (budget * 0.55).max(8.0) as usize);
                let klen = key.chars().count();
                let val = fit(&row.val, (budget - klen as f32).max(0.0) as usize);
                let vlen = val.chars().count();
                let kx = body[0] + RON_INSET + klen as f32 * adv / 2.0;
                let vx = kx + (klen + vlen) as f32 * adv / 2.0;
                nodes[RON + k * 2].shape = if klen > 0 {
                    Some(self.ron_text(
                        key,
                        700.0,
                        frost::Color {
                            r: 0.87,
                            g: 0.87,
                            b: 0.90,
                            a: 1.0,
                        },
                    ))
                } else {
                    None
                };
                nodes[RON + k * 2].transform = frost::Transform::translate([kx, y]);
                nodes[RON + k * 2 + 1].shape = if vlen > 0 {
                    let head = matches!(row.vkind, ron_tree::VKind::Head);
                    Some(self.ron_text(val, if head { 700.0 } else { 500.0 }, ron_color(row.vkind)))
                } else {
                    None
                };
                nodes[RON + k * 2 + 1].transform = frost::Transform::translate([vx, y]);
                drawn = k + 1;
            }
        }
        // Retire the rows the panel did not show this frame.
        let nodes = &mut ctx.scene().root.children;
        for node in &mut nodes[RON + drawn * 2..RON + RON_NODES] {
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

        // The cursor's position for next frame's right-drag delta; `None`
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
    let shape = frost::Shape::sprite(path).map_err(|e| format!("failed to load '{name}': {e}"))?;
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
    let thumb = frost::Shape::sprite_bytes(&png_bytes(&small)?)
        .map_err(|e| format!("failed to build the thumbnail: {e}"))?;
    Ok(Sprite {
        path: path.to_path_buf(),
        name,
        current: tex,
        history: Vec::new(),
        shape,
        thumb,
        ron: load_ron(path),
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
    let mut root = match ron_tree::parse(&text) {
        Ok(v) => v,
        Err(err) => {
            log::warn!("sidecar '{}': {err}", file_name_of(&side));
            return None;
        }
    };
    // The tree opens its first two levels; everything deeper waits.
    root.open_to(0, 1);
    let name = file_name_of(&side);
    log::info!("sidecar '{name}' loaded for '{}'", file_name_of(png));
    let rows = ron_tree::layout(&root);
    Some(RonDoc {
        name,
        root,
        rows,
        scroll: 0.0,
    })
}

/// Where a sidecar panel first opens: at the work area's lower right,
/// clear of the slot strip — where the panel was docked before it learned
/// to move. `row_h` and `pad` are the UI style's, and the panel's height
/// is its title bar, twice the padding and the tree viewport.
fn ron_default(w: f32, h: f32, row_h: f32, pad: f32) -> [f32; 2] {
    let panel_h = row_h + 2.0 * pad + RON_VIEW_H;
    [
        w / 2.0 - 20.0 - RON_W / 2.0,
        -h / 2.0 + STRIP_H + 12.0 + panel_h / 2.0,
    ]
}

/// Whether a window point is inside a rectangle.
fn in_rect(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3]
}

/// The tree row under a window point inside the body viewport `body`,
/// given the scroll and the row count. Row `i` draws centered at
/// `body_top - PAD - (i + 0.5) * LINE + scroll`, so its band spans
/// `body_top - PAD - (i + 1) * LINE + scroll` up to the same less one
/// line — exactly what a plain `floor` of the inverted center formula
/// selects: subtracting the row's half-line before flooring would shift
/// every click in a row's upper half onto the row above.
fn ron_row_at(p: [f32; 2], body: [f32; 4], scroll: f32, n: usize) -> Option<usize> {
    if !in_rect(body, p) {
        return None;
    }
    let i = ((body[3] - RON_PAD - p[1] + scroll) / RON_LINE).floor();
    (i >= 0.0 && (i as usize) < n).then_some(i as usize)
}

/// Clip a line to `max` characters, ellipsis included.
fn fit(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The panel's value-side color: container heads amber, closing
/// brackets muted, atoms by kind — `ron_view`'s palette on the dark
/// plate.
fn ron_color(k: ron_tree::VKind) -> frost::Color {
    let c = |r, g, b| frost::Color { r, g, b, a: 1.0 };
    match k {
        ron_tree::VKind::Head => c(0.93, 0.78, 0.44),
        ron_tree::VKind::Close => c(0.55, 0.55, 0.60),
        ron_tree::VKind::Atom(ron_tree::Kind::Str) => c(0.72, 0.86, 0.66),
        ron_tree::VKind::Atom(ron_tree::Kind::Char) => c(0.66, 0.80, 0.78),
        ron_tree::VKind::Atom(ron_tree::Kind::Num) => c(0.60, 0.78, 0.96),
        ron_tree::VKind::Atom(ron_tree::Kind::Bool) => c(0.83, 0.68, 0.95),
        ron_tree::VKind::Atom(ron_tree::Kind::Path) => c(0.64, 0.86, 0.84),
    }
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

/// The `i`th slot's center, in window coordinates.
fn slot_center(i: usize, w: f32, h: f32) -> [f32; 2] {
    [
        -w / 2.0 + SLOT_MARGIN + SLOT / 2.0 + i as f32 * (SLOT + SLOT_GAP),
        -h / 2.0 + STRIP_H / 2.0,
    ]
}

/// The slot under a window point, if any.
fn slot_at(p: [f32; 2], w: f32, h: f32) -> Option<usize> {
    if p[1] > -h / 2.0 + STRIP_H {
        return None;
    }
    (0..SLOTS).find(|i| {
        let [cx, cy] = slot_center(*i, w, h);
        (p[0] - cx).abs() <= SLOT / 2.0 && (p[1] - cy).abs() <= SLOT / 2.0
    })
}

/// Whether a window point is in the work area — above the slots strip.
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
    // exists, and takes the leftmost slot; without the argument all slots
    // start empty and the demo's first frame opens the file dialog — as a
    // child of the window, so it lands on top of it — and loads the pick
    // into the first slot (see `load_sprite`).
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
    // overlay uses — shared by the panel's labels and the usage line.
    let font = format!("{root}/assets/fonts/FiraCode-VariableFont_wght.ttf");
    let ui = frost::Ui::from_font(&font).expect("failed to load the UI font");
    // The sidecar rows' font: one Arc's worth of the same FiraCode,
    // every row's shapes cloning the pointer, never re-reading the file.
    let ron_font: Arc<[u8]> =
        Arc::from(std::fs::read(&font).expect("failed to read the sidecar font"));
    let help = Some(
        frost::Shape::text(
            &font,
            "space play   ctrl-o open   ctrl-z undo   wheel zoom",
            20.0,
        )
        .expect("failed to load assets/fonts/FiraCode-VariableFont_wght.ttf")
        .with_weight(520.0),
    );

    // The work area's layer pool comes first: the process points the
    // nodes at the animation's current frame (or the active sprite
    // alone, in the first node). Then the fixed nodes: help, marker and
    // checker.
    let work_shape = sprites.first().map(|sp| sp.shape.clone());
    let mut children: Vec<Box<frost::SceneNode>> = (0..LAYER_NODES)
        .map(|i| {
            Box::new(frost::SceneNode {
                // A command-line sprite is already in the first node.
                shape: if i == 0 { work_shape.clone() } else { None },
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
    // The sidecar panel's pool: two text nodes per row and the title,
    // hidden until a sprite with a sidecar is active.
    children.extend((0..RON_NODES).map(|_| {
        Box::new(frost::SceneNode {
            order: RON_ORDER,
            ..Default::default()
        })
    }));

    let scene = frost::Scene::new(frost::SceneNode {
        // Dark background; the node's transform is ignored.
        shape: Some(frost::Shape::Background { color: BG }),
        children,
        ..Default::default()
    });

    if let Err(err) = frost::run(
        scene,
        Demo {
            ui,
            sprites,
            active: 0,
            anim: Anim {
                frames: Vec::new(),
                frame: 0,
                looping: true,
                playing: false,
                dir: 1,
                elapsed: 0.0,
            },
            zoom: 1.0,
            offset: [0.0, 0.0],
            selection: None,
            drag_from: None,
            slot_drag: None,
            was_undo: false,
            was_open: false,
            was_space: false,
            light: GREY_LIGHT,
            dark: GREY_DARK,
            checker_key: (0, 0, 0, 0),
            was_down: false,
            last_mouse: None,
            last_click: None,
            pending: dialog_dir,
            dir,
            status: String::new(),
            ron_font,
            ron_press: None,
            ron_body: None,
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
        let [cx, cy] = slot_center(0, w, h);
        assert_eq!(cx, -w / 2.0 + SLOT_MARGIN + SLOT / 2.0);
        assert_eq!(cy, -h / 2.0 + STRIP_H / 2.0);
        // Neighbors sit one slot and one gap apart.
        let [cx1, _] = slot_center(1, w, h);
        assert_eq!(cx1 - cx, SLOT + SLOT_GAP);
        // Each slot's center is its own; the gaps and the area above the
        // strip belong to no slot.
        for i in 0..SLOTS {
            let [x, y] = slot_center(i, w, h);
            assert_eq!(slot_at([x, y], w, h), Some(i));
        }
        let [cx0, cy0] = slot_center(0, w, h);
        assert_eq!(
            slot_at([cx0 + SLOT / 2.0 + SLOT_GAP / 2.0, cy0], w, h),
            None
        );
        assert_eq!(slot_at([cx0, cy0 + STRIP_H / 2.0 + 1.0], w, h), None);
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
            layers: Vec::new(),
            next_time,
        }
    }

    fn anim(times: &[f32], looping: bool) -> Anim {
        Anim {
            frames: times.iter().map(|&t| frame(t)).collect(),
            frame: 0,
            looping,
            playing: true,
            dir: 1,
            elapsed: 0.0,
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
        let c = ron_default(w, h, row_h, pad);
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
    fn a_panel_click_maps_to_the_row_under_it() {
        // A synthetic body viewport: 300 wide, 240 tall, top edge at 100.
        let body = [-150.0, -140.0, 150.0, 100.0];
        // Row 0's center sits half a line under the viewport's padding.
        let y0 = 100.0 - RON_PAD - 0.5 * RON_LINE;
        assert_eq!(ron_row_at([0.0, y0], body, 0.0, 10), Some(0));
        // The whole band belongs to the row: a half-line above or below
        // its center — anywhere but the very borders — is still row 0.
        assert_eq!(
            ron_row_at([0.0, y0 + 0.45 * RON_LINE], body, 0.0, 10),
            Some(0)
        );
        assert_eq!(
            ron_row_at([0.0, y0 - 0.45 * RON_LINE], body, 0.0, 10),
            Some(0)
        );
        // One line of scroll brings row 1 to the very same point.
        assert_eq!(ron_row_at([0.0, y0], body, RON_LINE, 10), Some(1));
        // Above the viewport is no row; past the last row is no row.
        assert_eq!(ron_row_at([0.0, 105.0], body, 0.0, 10), None);
        assert_eq!(ron_row_at([0.0, -139.0], body, 0.0, 2), None);
    }
    #[test]
    fn sidecar_lines_fit_the_panel_with_an_ellipsis() {
        assert_eq!(fit("water", 10), "water");
        assert_eq!(fit("0123456789", 6), "01234\u{2026}");
        assert_eq!(fit("", 6), "");
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
}
