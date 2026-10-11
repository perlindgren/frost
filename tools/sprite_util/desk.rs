//! The workbench's state: the Demo struct, the active view, the
//! frame sync, and the history steps.
// The subjects this one reads.
use crate::art::*;
use crate::band::*;
use crate::clip::*;
use crate::journal::*;
use crate::map::*;
use crate::ron_panels::*;
use crate::sidecar::*;
use crate::strip::*;
use crate::theme::*;
use crate::world::*;
use std::sync::Arc;

/// The workbench's view: which set of panels answers to the View menu.
/// `Markers` is the full editing desk — every panel; `TileSet` keeps
/// only the tile-set panel, the cut's rows and columns alone on the
/// screen, and its bench holds tile sets: each one a sheet, a cut,
/// and the roles its document speaks. (Tile Map completes the trio:
/// the painting desk.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum View {
    Markers,
    TileSet,
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
    pub(crate) fn name(self) -> &'static str {
        match self {
            View::Markers => "Markers",
            View::TileSet => "Tile Set",
            View::TileMap => "Tile Map",
        }
    }
}

/// The demo's state.
pub(crate) struct Demo {
    /// The widget layer: the View and Operations panels.
    pub(crate) ui: frost::Ui,
    /// The tileset band's base shape: a `TileMap` shape sharing the
    /// active tileset's very pixel buffer (the same texture the sprite
    /// desk draws), kept so each cell is a one-tile clone of it.
    pub(crate) band_base: Option<frost::Shape>,
    /// What `band_base` was built from — slot, texture identity, grid —
    /// so the rebuild happens only when one of them moves.
    pub(crate) band_key: Option<(usize, usize, u64, usize, usize)>,
    /// The band's per-cell shapes: one tile each, sized to fit the
    /// cell box, rebuilt with the base.
    pub(crate) cell_shapes: Vec<frost::Shape>,
    /// The atlas cell the next click will paint: the brush. A tool
    /// mode like the active slot, outside the undo road.
    pub(crate) picked_cell: Option<usize>,
    /// The tile-set bench's hover — slot and spoken line — and the
    /// label it wears. A still cursor over an unchanged bench shapes
    /// no text: the key decides when to reshape.
    pub(crate) hover_slot_key: Option<(usize, String)>,
    pub(crate) hover_label: Option<frost::Shape>,
    /// The tile-map bench: the layer the desk views and draws to,
    /// and which layers the canvas shows. Painting answers to this
    /// position, never to a name — two layers may wear one tileset;
    /// the visibility is the eye's, outside the undo road.
    pub(crate) active_layer: usize,
    pub(crate) layer_visible: Vec<bool>,
    /// Each layer remembers the tile it last wore and that tile's
    /// turn: `Some` once the layer has been left once, `None` while
    /// its first choosing is still owed (the top-left tile, neutral).
    pub(crate) layer_brush: Vec<Option<(usize, u8)>>,
    /// The number row's held state, one per digit, so a press is
    /// one choice and not a repeat.
    pub(crate) was_digits: [bool; 9],
    /// Each view keeps its own desk: what it loaded, what it holds,
    /// how far its road walked. The three desks are disjunct
    /// worlds — switching archives one and recalls the other, so
    /// nothing one desk carries ever shows on another's row.
    pub(crate) benches: [Bench; 3],
    /// The tile desk's painted layers — the picture, and the undo
    /// road's property (captured into every `World`).
    pub(crate) maps: Vec<MapLayer>,
    /// The tileset documents the desk carries: opened with a map,
    /// rewritten when one is saved. The document — not a sprite's
    /// momentary atlas panel — is the cut's memory.
    pub(crate) shelf: Vec<ShelfTileset>,
    /// A paint or erase stroke in flight: opened by a left-press (or
    /// by Delete over the desk) and lived out while its trigger holds.
    pub(crate) paint: Option<Paint>,
    /// The very first frame re-syncs the work pool after the world
    /// exists: the boot construction baked the sprite into a node by
    /// hand, and everything the world can carry (maps included)
    /// reaches the canvas through `sync_work` from then on.
    pub(crate) first_frame: bool,
    /// The loaded sprites — a sprite's slot is its index.
    pub(crate) sprites: Vec<Sprite>,
    /// The slot whose sprite fills the work area and takes the
    /// operations. Meaningless while `sprites` is empty.
    pub(crate) active: usize,
    /// The animation built from the sprites: frames of layers, on a
    /// clock, shown in the work area while any frame exists.
    pub(crate) anim: Anim,
    /// The current zoom: the sprite's scale factor (1.0 is texture size).
    pub(crate) zoom: f32,
    /// The sprite's pan offset from the work area's center, in window
    /// pixels; set by middle-dragging and by the wheel's anchor.
    pub(crate) offset: [f32; 2],
    /// The crop selection in texture pixels, `[x0, y0, x1, y1]` (y down):
    /// dragged with the left button, consumed by Crop.
    pub(crate) selection: Option<[f32; 4]>,
    /// Where the current work-area press began, in window pixels — only
    /// set when the press was the work area's, not the UI's or a slot's.
    pub(crate) drag_from: Option<[f32; 2]>,
    /// The slot being dragged between slots, and where its press began.
    pub(crate) slot_drag: Option<(usize, [f32; 2])>,
    /// The slot strip's scroll, in window pixels — `f32::MAX` parks
    /// it at the far end, clamped back each frame.
    pub(crate) slot_scroll: f32,
    /// The scrollbar knob's grab offset while it is dragged.
    pub(crate) slot_scrub: Option<f32>,
    /// Where the tileset panel's plate stands — `None` keeps it at
    /// its corner above the strip's left end.
    pub(crate) band_origin: Option<[f32; 2]>,
    /// The panel's own zoom: the wheel over it scales the rack's
    /// cells, never the canvas.
    pub(crate) band_zoom: f32,
    /// A held panel drag: the grab offset in the plate, the press
    /// point, and the origin the drag started from.
    pub(crate) band_drag: Option<([f32; 2], [f32; 2], [f32; 2])>,
    /// Whether Ctrl-Z was held last frame: its rising edge is the undo.
    pub(crate) was_undo: bool,
    /// Whether Ctrl-O was held last frame: its rising edge is the open.
    pub(crate) was_open: bool,
    /// Whether space was held last frame: its rising edge toggles play.
    pub(crate) was_space: bool,
    /// Whether Escape was held last frame: its rising edge releases a
    /// picked position.
    pub(crate) was_esc: bool,
    /// The checker's light grey level, 0.0-1.0; the Light slider sets it.
    pub(crate) light: f32,
    /// The checker's dark grey level, 0.0-1.0; the Dark slider sets it.
    pub(crate) dark: f32,
    /// The checker texture's key: its cell count and the two grey levels,
    /// 0-255 quantized — the texture is rebuilt only when one of them
    /// changes.
    pub(crate) checker_key: (u32, u32, u8, u8),
    /// Whether the left mouse button was held on the previous frame: the
    /// edges of the two are the press and the release.
    pub(crate) was_down: bool,
    /// The cursor's position on the previous frame, for the middle-drag's
    /// frame-to-frame pan delta.
    pub(crate) last_mouse: Option<[f32; 2]>,
    /// The last reported click: its position in the active texture's
    /// pixel space, and whether the spot was inside the texture.
    pub(crate) last_click: Option<([f32; 2], bool)>,
    /// The seat numbers' font bytes: one Arc's worth of FiraCode, the
    /// numbers beside the markers cloning the pointer, never the file.
    pub(crate) ron_font: Arc<[u8]>,
    /// Where the right button went down, for its still-click test.
    pub(crate) r_press: Option<[f32; 2]>,
    /// The middle button's press point: a release without travel
    /// moves the held block; a press that travels is the pan.
    pub(crate) m_press: Option<[f32; 2]>,
    /// The middle button's level last poll — the press edge.
    pub(crate) was_mdown: bool,
    /// Whether the exit-confirmation box stands over the editor.
    pub(crate) quit_prompt: bool,
    /// Last frame's Escape, for the exit box's own press edge.
    pub(crate) was_esc_prompt: bool,
    /// Whether the right button was held on the previous frame.
    pub(crate) was_rdown: bool,
    /// Last frame's Delete/Backspace, for the eraser's press edge.
    pub(crate) was_del: bool,
    /// The copied rectangle held on the desk, dressed and pasted as
    /// one picture; `None` while the single-tile brush is held.
    pub(crate) clip: Option<Clip>,
    /// The selection's cell bounds, inclusive `[c0, r0, c1, r1]` —
    /// the frame around a held block, and the Delete key's target.
    pub(crate) sel: Option<[i32; 4]>,
    /// The cell where Shift + left went down, while the drag lives.
    pub(crate) sel_drag: Option<[i32; 2]>,
    /// Last frame's Escape, for the block-drop edge.
    pub(crate) was_block_esc: bool,
    /// An in-progress list reordering: which slot's sequence, at which
    /// path, and the entry's current seat.
    pub(crate) ron_drag: Option<(usize, Vec<u16>, usize)>,
    /// The shared interaction state of the sidecar trees: the widget's
    /// claim on the close box and the grip, the thumbs and the corner
    /// drags in flight. One set of trees per frame, so one state across
    /// them all.
    pub(crate) ron_state: frost::TreeState,
    /// Every open file's panel as the UI laid it out — the frame's own
    /// record of the views, rebuilt between the panel calls and the
    /// tree painting.
    pub(crate) ron_views: Vec<RonView>,
    /// The view the View menu last chose: decides which panels this
    /// frame declares, and names itself at the menu bar's right end.
    pub(crate) view: View,
    /// The road back: one whole-world snapshot per command, newest
    /// last.
    pub(crate) undo_stack: Vec<World>,
    /// The road forward again: the worlds undo stepped off, newest
    /// last; a fresh command empties it — the future a new past erases.
    pub(crate) redo_stack: Vec<World>,
    /// Whether Ctrl(-Shift)-Z rested down last frame: the keys speak on
    /// their rising edges only.
    pub(crate) was_redo: bool,
    /// The zoom pair's edges, same rising-edge rule.
    pub(crate) was_zoom_in: bool,
    pub(crate) was_zoom_out: bool,
    /// The paintbrush's standing orientation: bit 0 flip-x, bit 1
    /// flip-y, bits 2..3 clockwise quarter-turns. Tool state, like the
    /// picked cell — outside the undo road; it changes what the NEXT
    /// stroke paints, and every map keeps what it was painted with.
    pub(crate) brush: u8,
    /// The orientation keys' edges: X mirrors sideways, Y mirrors
    /// top-to-bottom, R turns a quarter clockwise, Shift-R back.
    pub(crate) was_flipx: bool,
    pub(crate) was_flipy: bool,
    pub(crate) was_turn: bool,
    /// The folder the next dialog opens in: where the last file was
    /// read from or written to, or — before any dialog has run — the
    /// folder the command line named, the working directory when no
    /// argument did.
    pub(crate) dir: Option<std::path::PathBuf>,
    /// The document the desk works on: the map it last opened or
    /// saved. Save map answers with its name and folder — the desk
    /// remembers the document, not only the bench it paints with.
    pub(crate) map_path: Option<std::path::PathBuf>,
    /// The bench's ear for a running application, when it is
    /// listening to one: reread on every telling, unmade when the
    /// card dies. One attachment at a time — a conversation, not a
    /// watchlist.
    pub(crate) attached: Option<crate::actions::Attachment>,
    /// The status band's message: the outcome of the last operation,
    /// spoken at the window's bottom edge — and, as it changes, the
    /// log's newest voice.
    pub(crate) status: String,
    /// The log bar's memory: every status the bench has spoken, oldest
    /// first, as many as `LOG_KEEP` of them.
    pub(crate) log: Vec<String>,
    /// The last status captured: a message is an event when it
    /// changes, and a change is measured against this.
    pub(crate) log_seen: String,
    /// Whether the log panel stands open above its band.
    pub(crate) log_open: bool,
    /// The panel's highlighted row and first visible row, as display
    /// indices into the newest-first view: 0 is the newest message.
    pub(crate) log_sel: usize,
    pub(crate) log_top: usize,
    /// Long lines' shared x offset, in pixels.
    pub(crate) log_sx: f32,
    /// A held thumb: whether it is the vertical one, and the grab's
    /// offset inside it.
    pub(crate) log_grab: Option<(bool, f32)>,
    /// The arrows' held state: a press steps once; holding does not run.
    pub(crate) log_arrows: [bool; 4],
    /// Whether the sidecar's authored `shapes:` draw over the sprite:
    /// the View menu's Shapes line. Validation ignores it — a refusal
    /// speaks on the notes whether or not the overlay is checked.
    pub(crate) show_shapes: bool,
}

/// Which view's archive a view opens: the slot in `benches`.
pub(crate) fn view_idx(view: View) -> usize {
    match view {
        View::Markers => 0,
        View::TileSet => 1,
        View::TileMap => 2,
    }
}

/// One desk's whole memory, kept while its view stands unoccupied:
/// the bench and what it wears, the work and the copy held over it,
/// the eye's zoom and scroll, and the undo road walked on it. A
/// desk is not shared between views — what you load for one stays
/// on it; each view recalls its own state, nothing more and nothing
/// less.
pub(crate) struct Bench {
    /// The sprite slots and which sits at the bench's right hand.
    pub(crate) sprites: Vec<Sprite>,
    pub(crate) active: usize,
    /// The tile map's layers, the tileset cargo, and the bench of
    /// layers' own words: highlight, eyes, and tile memories.
    pub(crate) maps: Vec<MapLayer>,
    pub(crate) shelf: Vec<ShelfTileset>,
    pub(crate) active_layer: usize,
    pub(crate) layer_visible: Vec<bool>,
    pub(crate) layer_brush: Vec<Option<(usize, u8)>>,
    /// The map file this desk's picture came from and returns to.
    pub(crate) map_path: Option<std::path::PathBuf>,
    /// The brush: the picked atlas cell and its turn.
    pub(crate) picked_cell: Option<usize>,
    pub(crate) brush: u8,
    /// The held copy: block and its frame.
    pub(crate) clip: Option<Clip>,
    pub(crate) sel: Option<[i32; 4]>,
    /// The crop selection dragged on the work area.
    pub(crate) selection: Option<[f32; 4]>,
    /// The animation built on this desk.
    pub(crate) anim: Anim,
    /// The eye: the work area's zoom and pan, the bench's scroll.
    pub(crate) zoom: f32,
    pub(crate) offset: [f32; 2],
    pub(crate) slot_scroll: f32,
    /// The undo road walked on this desk alone.
    pub(crate) undo_stack: Vec<World>,
    pub(crate) redo_stack: Vec<World>,
}

impl Default for Bench {
    fn default() -> Self {
        Self {
            anim: Anim {
                looping: true,
                dir: 1,
                ..Anim::default()
            },
            zoom: 1.0,
            ..Self::bare()
        }
    }
}

impl Bench {
    /// An untouched desk: every collection empty, every eye at rest.
    fn bare() -> Self {
        Self {
            sprites: Vec::new(),
            active: 0,
            maps: Vec::new(),
            shelf: Vec::new(),
            active_layer: 0,
            layer_visible: Vec::new(),
            layer_brush: Vec::new(),
            map_path: None,
            picked_cell: None,
            brush: 0,
            clip: None,
            sel: None,
            selection: None,
            anim: Anim::default(),
            zoom: 1.0,
            offset: [0.0, 0.0],
            slot_scroll: 0.0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }
}

impl Demo {
    /// The active sprite, or `None` while all slots are empty.
    pub(crate) fn active(&self) -> Option<&Sprite> {
        self.sprites.get(self.active)
    }

    /// The active sprite's sidecar panel state, if it has one.
    pub(crate) fn ron(&self) -> Option<&RonDoc> {
        self.active()?.ron.as_ref()
    }

    /// The same, writable: the wheel's scroll and the folds' toggles.
    pub(crate) fn ron_mut(&mut self) -> Option<&mut RonDoc> {
        self.sprites.get_mut(self.active)?.ron.as_mut()
    }

    /// One sidecar-panel text shape, on the shared font bytes.
    pub(crate) fn ron_text(
        &self,
        text: String,
        size: f32,
        weight: f32,
        color: frost::Color,
    ) -> frost::Shape {
        frost::Shape::Text {
            text,
            font: Arc::clone(&self.ron_font),
            size,
            weight,
            color,
            alpha: 1.0,
        }
    }

    /// The line a hover speaks: the name the slot answers to — the
    /// bound document's spoken name when one is worn, the sheet's own
    /// stem otherwise — together with the cut it wears.
    pub(crate) fn tileset_answer(&self, i: usize) -> String {
        let name = match self
            .sprites
            .get(i)
            .and_then(|sp| sp.doc.as_ref())
            .and_then(|d| self.shelf.iter().find(|e| &e.path == d))
        {
            Some(e) => e.tileset.name.clone(),
            None => self
                .sprites
                .get(i)
                .map_or_else(String::new, |sp| sp.name.clone()),
        };
        match self.sprites.get(i).and_then(|sp| sp.atlas) {
            Some((rows, cols)) => format!("{name} · {rows} x {cols}"),
            None => format!("{name} · no grid"),
        }
    }

    /// Exchange the working fields with the archive of the view now
    /// held: laid down when leaving, taken up when entering — one
    /// road, walked twice through a door. Nothing copies: the desks
    /// travel whole, and an occupied view's archive stands bare.
    /// In-flight gestures are no part of a desk's memory; whoever
    /// calls this road releases them on the way out.
    fn swap_desk(&mut self) {
        let b = &mut self.benches[view_idx(self.view)];
        std::mem::swap(&mut b.sprites, &mut self.sprites);
        std::mem::swap(&mut b.active, &mut self.active);
        std::mem::swap(&mut b.maps, &mut self.maps);
        std::mem::swap(&mut b.shelf, &mut self.shelf);
        std::mem::swap(&mut b.active_layer, &mut self.active_layer);
        std::mem::swap(&mut b.layer_visible, &mut self.layer_visible);
        std::mem::swap(&mut b.layer_brush, &mut self.layer_brush);
        std::mem::swap(&mut b.map_path, &mut self.map_path);
        std::mem::swap(&mut b.picked_cell, &mut self.picked_cell);
        std::mem::swap(&mut b.brush, &mut self.brush);
        std::mem::swap(&mut b.clip, &mut self.clip);
        std::mem::swap(&mut b.sel, &mut self.sel);
        std::mem::swap(&mut b.selection, &mut self.selection);
        std::mem::swap(&mut b.anim, &mut self.anim);
        std::mem::swap(&mut b.zoom, &mut self.zoom);
        std::mem::swap(&mut b.offset, &mut self.offset);
        std::mem::swap(&mut b.slot_scroll, &mut self.slot_scroll);
        std::mem::swap(&mut b.undo_stack, &mut self.undo_stack);
        std::mem::swap(&mut b.redo_stack, &mut self.redo_stack);
    }

    /// The door between desks. The views are disjunct worlds: the
    /// desk you leave is laid down whole, the desk you enter is
    /// recalled whole — a tileset loaded for one never stands on
    /// another's row, and neither desk's undo road crosses into
    /// the other. Derived things retake themselves: the rack's
    /// shapes, the layer bench's alignment, the canvas's work.
    pub(crate) fn switch_view(&mut self, ctx: &mut frost::Context, to: View) {
        self.swap_desk();
        self.view = to;
        self.swap_desk();
        // In-flight gestures belong to the desk left behind.
        self.sel_drag = None;
        self.slot_drag = None;
        self.drag_from = None;
        self.slot_scrub = None;
        self.paint = None;
        // The rack retells itself from the entered desk's slot.
        self.band_key = None;
        self.align_layers();
        self.sync_work(ctx);
        self.refresh_slots(ctx);
    }

    /// Keep the tile-map bench honest after any change of the maps:
    /// the highlight never stands beyond the stack, and every layer
    /// has exactly one visibility word.
    pub(crate) fn align_layers(&mut self) {
        self.active_layer = self.active_layer.min(self.maps.len().saturating_sub(1));
        self.layer_visible.resize(self.maps.len(), true);
        self.layer_brush.resize(self.maps.len(), None);
    }

    /// The standing layer's remembered tile onto the rack: the last
    /// one painted there, or — the first time — the atlas's top-left
    /// tile worn neutral.
    pub(crate) fn recall_layer(&mut self) {
        let (cell, tfm) = self
            .layer_brush
            .get(self.active_layer)
            .copied()
            .flatten()
            .unwrap_or((0, 0));
        self.picked_cell = Some(cell);
        self.brush = tfm;
    }

    /// The slot wearing a layer's sheet takes the bench's seat, so
    /// the rack shows the very tiles this layer paints with.
    fn wake_layer_sheet(&mut self, i: usize) {
        let sheet = self
            .maps
            .get(i)
            .and_then(|m| dressed(&m.tileset, &self.shelf, &self.sprites))
            .map(|(sp, _)| sp.path.clone());
        if let Some(sheet) = sheet
            && let Some(s) = self.sprites.iter().position(|sp| sp.path == sheet)
        {
            self.active = s;
        }
    }

    /// The bottom row by keyboard: the plate at place `i` answers
    /// as if clicked. On the map desk a layer takes the highlight —
    /// its tile memory recalled, its sheet woken; elsewhere a slot
    /// wakes to the bench.
    pub(crate) fn row_pick(&mut self, ctx: &mut frost::Context, i: usize) {
        if self.view == View::TileMap {
            if i < self.maps.len() && i != self.active_layer {
                let who = self.maps[i].tileset.clone();
                self.select_layer(i);
                self.selection = None;
                self.last_click = None;
                self.status = format!("map: painting on '{who}'");
                self.sync_work(ctx);
                self.refresh_slots(ctx);
            }
        } else if i < self.sprites.len() {
            self.active = i;
            self.selection = None;
            self.last_click = None;
            self.status = format!("slot: '{}'", asset_name(&self.sprites[i].path));
            self.sync_work(ctx);
        }
    }

    /// One dress verb, aimed where the cursor will drop: a held
    /// block takes it as one picture — the desk's rule, keys and
    /// mouse alike; with empty hands the single-tile brush answers.
    /// The words say which was dressed.
    pub(crate) fn dress_verb(&mut self, d: Dress) -> String {
        let word = match d {
            Dress::FlipX => "flipped x",
            Dress::FlipY => "flipped y",
            Dress::Turn(cw) => {
                if cw {
                    "turned cw"
                } else {
                    "turned ccw"
                }
            }
        };
        if self.clip.is_some() {
            if let Some(clip) = &mut self.clip {
                dress_block(clip, &mut self.sel, d);
            }
            format!("block {word}")
        } else {
            self.brush = dress_brush(self.brush, d);
            format!("brush {word} — {}", turn_note(self.brush))
        }
    }

    /// Leave one layer, enter another: the layer departed keeps the
    /// tile and the turn it wore; the layer entered speaks its own
    /// memory. A click on the standing highlight is no departure —
    /// the rack keeps its tile, and only the sheet wakes.
    pub(crate) fn select_layer(&mut self, i: usize) {
        if i >= self.maps.len() {
            return;
        }
        if i != self.active_layer {
            if let Some(mem) = self.layer_brush.get_mut(self.active_layer) {
                *mem = Some((self.picked_cell.unwrap_or(0), self.brush));
            }
            self.active_layer = i;
            self.recall_layer();
        }
        self.wake_layer_sheet(i);
    }

    /// The layer's face on the bench: its tileset's sheet thumbnail,
    /// dressed the way the canvas dresses the layer — the cell wears
    /// the picture the map is painted with.
    pub(crate) fn layer_thumb(&self, i: usize) -> Option<frost::Shape> {
        let name = self.maps.get(i)?.tileset.as_str();
        let (sp, _) = dressed(name, &self.shelf, &self.sprites)?;
        Some(sp.thumb.clone())
    }

    /// The active sprite's texture size, or zero while there is none.
    pub(crate) fn work_size(&self) -> [f32; 2] {
        match self.active() {
            Some(sp) => [sp.current.width() as f32, sp.current.height() as f32],
            None => [0.0, 0.0],
        }
    }

    /// A press inside the open panel: the thumbs claim first — their
    /// drags live in the held state — and otherwise the row under the
    /// cursor takes the highlight.
    pub(crate) fn log_press(&mut self, p: [f32; 2], w: f32, h: f32) {
        let panel = log_panel_rect(w, h);
        let body = log_body(panel);
        let len = self.log.len();
        let (off, kh) = log_v_thumb(len, body[3] - body[1], self.log_top);
        if len > LOG_ROWS
            && p[0] >= body[2]
            && p[0] <= body[2] + LOG_TRACK_W
            && p[1] <= body[3] - off
            && p[1] >= body[3] - off - kh
        {
            self.log_grab = Some((true, p[1] - (body[3] - off)));
            return;
        }
        let view_w = body[2] - body[0];
        let content = log_wide(&self.log) as f32 * RON_ADV;
        let (hoff, hw) = log_h_thumb(view_w, content, view_w, self.log_sx);
        if content > view_w
            && p[1] <= panel[1] + LOG_PAD
            && p[0] >= body[0] + hoff
            && p[0] <= body[0] + hoff + hw
        {
            self.log_grab = Some((false, p[0] - (body[0] + hoff)));
            return;
        }
        if let Some(row) = log_row_at(len, self.log_top, body, p[1]) {
            self.log_sel = row;
        }
    }

    /// Rebuild the tileset band when the tileset, its texture or its
    /// grid moved. The base is the active sprite's own pixels wearing a
    /// `TileMap` face — one texture for the sprite and every cell of
    /// the band, keyed by the same buffer identity the engine's
    /// texture cache uses — and each cell is that base with a single
    /// tile: the cell's atlas rectangle, stretched over the cell box.
    pub(crate) fn sync_band(&mut self) {
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
    pub(crate) fn sync_work(&self, ctx: &mut frost::Context) {
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
                    .filter(|_| self.layer_visible.get(i).copied().unwrap_or(true))
                    .and_then(|m| layer_shape(m, &self.shelf, &self.sprites)),
                Some(f) => f.layers.get(i).map(|l| l.shape.clone()),
                None if i == 0 => self.active().map(|sp| sp.shape.clone()),
                None => None,
            };
        }
    }

    /// Re-point every slot node at its slot's thumbnail (or hide empty
    /// slots). Only needed when slots change: opens, closes, swaps.
    pub(crate) fn refresh_slots(&self, ctx: &mut frost::Context) {
        let bench_layers = self.view == View::TileMap;
        for i in 0..SLOTS {
            let node = &mut ctx.scene().root.children[THUMBS + i];
            node.shape = if bench_layers {
                self.layer_thumb(i)
            } else {
                self.sprites.get(i).map(|sp| sp.thumb.clone())
            };
            // The z-order number: shaped here, placed by the draw.
            let num = &mut ctx.scene().root.children[LAYZ0 + i];
            num.shape = if bench_layers && i < self.maps.len() {
                Some(self.ron_text(format!("{i}"), 14.0, 0.55, HOVER_INK))
            } else {
                None
            };
        }
    }

    /// Replace the active sprite's texture: re-encode it to PNG bytes,
    /// rebuild its shape, and put it on screen — the selection and the
    /// marker referred to the old texture, so both clear.
    pub(crate) fn apply(
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

    /// The bench as pure data: every sprite's pixels, sidecar and grid,
    /// the animation's frames, the active slot, the view, the selection.
    pub(crate) fn capture(&self) -> World {
        World {
            sprites: self
                .sprites
                .iter()
                .map(|sp| WorldSprite {
                    path: sp.path.clone(),
                    doc: sp.doc.clone(),
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
    pub(crate) fn stamp(&mut self) {
        let now = self.capture();
        push_step(&mut self.undo_stack, &mut self.redo_stack, now);
    }

    /// Undo: one step back over every kind of command — crops, marks,
    /// frame edits, slot moves — restoring the whole bench, textures
    /// rebuilt from the snapshot's own pixels.
    pub(crate) fn undo(&mut self, ctx: &mut frost::Context) {
        let now = self.capture();
        let Some(past) = step_back(&mut self.undo_stack, &mut self.redo_stack, now) else {
            self.status = String::from("undo: nothing to undo");
            return;
        };
        self.restore(ctx, past, "undo");
    }

    /// Redo: one step forward again, the same worlds walked backwards
    /// down the road undo left behind.
    pub(crate) fn redo(&mut self, ctx: &mut frost::Context) {
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
    pub(crate) fn restore(&mut self, ctx: &mut frost::Context, world: World, word: &str) {
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
                doc: ws.doc,
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
        self.align_layers();
        self.sync_work(ctx);
        self.refresh_slots(ctx);
    }
}

/// One frame's shared geometry: the canvas, the pan and zoom, the
/// pointer, the strip's scroll view, and the tileset panel's
/// frame-state. The handlers compute it once; every painter in
/// `draw.rs` reads the same record instead of threading it argument
/// by argument.
pub(crate) struct FrameState {
    pub(crate) w: f32,
    pub(crate) h: f32,
    pub(crate) view: [f32; 2],
    pub(crate) size: [f32; 2],
    pub(crate) tile_view: bool,
    pub(crate) pos: Option<[f32; 2]>,
    pub(crate) sv: StripView,
    pub(crate) slot_count: usize,
    pub(crate) brows: usize,
    pub(crate) bcols: usize,
    pub(crate) bcell: f32,
    pub(crate) brects: Vec<[f32; 4]>,
    pub(crate) bcells: usize,
    pub(crate) bplate: Option<[f32; 4]>,
}
