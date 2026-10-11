//! The tool's verbs: the menu bar's items and one method per
//! named action. A planned action is born here: a menu line
//! in the lists below and its method beside it.
// The subjects this one reads.
use crate::art::*;
use crate::desk::*;
use crate::layout::*;
use crate::map::*;
use crate::ron_panels::*;
use crate::ron_view;
use crate::sidecar::*;
use crate::spots::*;
use crate::world::push_step;

/// The File menu: the file verbs, lined up where every desktop app puts
/// them, with the two history steps among them. Open, Close, Save, Save
/// as, Undo and Redo do their named work — the pair walk the whole
/// bench's history, every crop, mark, frame and slot move; Quit raises
/// the same exit question Escape asks at the keys.
/// An item with an empty label draws the rule across the list: a
/// separator, not a command.
pub(crate) const FILE_MENU: frost::Menu<'static> = frost::Menu {
    title: "File",
    items: &[
        frost::MenuItem::new("Open", "Ctrl-O"),
        frost::MenuItem::new("Close", ""),
        frost::MenuItem::new("Save", ""),
        frost::MenuItem::new("Save as", ""),
        frost::MenuItem::new("Open map", ""),
        frost::MenuItem::new("Save map", ""),
        frost::MenuItem::new("Undo", "Ctrl-Z"),
        frost::MenuItem::new("Redo", "Ctrl-Shift-Z"),
        frost::MenuItem::SEPARATOR,
        frost::MenuItem::new("Quit", ""),
    ],
};

/// The same menu on Tile Set duty: the two verbs whose object is the
/// document. The bench holds tile sets here — sheets wearing a cut
/// and a document's roles — and Open and Save say what they open and
/// save. The rest of the bench's verbs keep their names: Close closes
/// a slot, Undo walks one history whatever the slot holds.
pub(crate) const FILE_MENU_TILESET: frost::Menu<'static> = frost::Menu {
    title: "File",
    items: &[
        frost::MenuItem::new("Open Tile Set", "Ctrl-O"),
        frost::MenuItem::new("Close", ""),
        frost::MenuItem::new("Save Tile Set", ""),
        frost::MenuItem::new("Save as Tile Set", ""),
        frost::MenuItem::new("Undo", "Ctrl-Z"),
        frost::MenuItem::new("Redo", "Ctrl-Shift-Z"),
        frost::MenuItem::SEPARATOR,
        frost::MenuItem::new("Quit", ""),
    ],
};

/// The same menu on Tile Map duty: the map is the work here, so the
/// three verbs that name their object name the map. The desk decides
/// the noun — the view says what mode the desk is in.
pub(crate) const FILE_MENU_TILEMAP: frost::Menu<'static> = frost::Menu {
    title: "File",
    items: &[
        frost::MenuItem::new("Open Tile Map", "Ctrl-O"),
        frost::MenuItem::new("Close", ""),
        frost::MenuItem::new("Save Tile Map", ""),
        frost::MenuItem::new("Save as Tile Map", ""),
        frost::MenuItem::new("Undo", "Ctrl-Z"),
        frost::MenuItem::new("Redo", "Ctrl-Shift-Z"),
        frost::MenuItem::SEPARATOR,
        frost::MenuItem::new("Quit", ""),
    ],
};

/// The File menu the bar shows: the verbs wear the view's object —
/// sprites at the editing desk, tile sets at theirs, tile maps at
/// the painting desk. No menu ever offers a thing the desk it stands
/// on is not working.
pub(crate) fn file_menu(view: crate::desk::View) -> frost::Menu<'static> {
    match view {
        crate::desk::View::Markers => FILE_MENU,
        crate::desk::View::TileSet => FILE_MENU_TILESET,
        crate::desk::View::TileMap => FILE_MENU_TILEMAP,
    }
}

/// Where an opened tile-set document goes on the bench: wake the
/// slot already wearing it, adopt a bare slot over the same sheet,
/// or make room — a fresh slot. The last case is what lets two tile
/// sets wear one sheet: the same PNG twice on the bench, each with
/// its own document, its own cut, its own roles.
#[derive(Debug, PartialEq, Eq)]
enum SlotChoice {
    Wake(usize),
    Adopt(usize),
    Fresh,
}

/// The rule, pure: no slot ever wears two documents, and no document
/// ever wakes two slots. A sheet already spoken for by another
/// document never silently gives up its cut — the newcomer takes a
/// slot of its own.
fn slot_for_doc(
    slots: &[(&std::path::Path, Option<&std::path::Path>)],
    sheet: &std::path::Path,
    doc: &std::path::Path,
) -> SlotChoice {
    if let Some(i) = slots.iter().position(|(_, d)| *d == Some(doc)) {
        return SlotChoice::Wake(i);
    }
    if let Some(i) = slots.iter().position(|(p, d)| *p == sheet && d.is_none()) {
        return SlotChoice::Adopt(i);
    }
    SlotChoice::Fresh
}

/// Which of the tile-map desk's two roads a chosen file takes.
/// The name carries the office, as everywhere on the desk:
/// `*.map.ron` is a map, any other RON a tileset document, a PNG
/// a raw sheet awaiting its cut.
#[derive(Debug, PartialEq, Eq)]
enum Road {
    Map,
    TilesetDoc,
    Sheet,
}

fn opened_road(path: &std::path::Path) -> Road {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        Road::Sheet
    } else if path
        .to_string_lossy()
        .replace('\\', "/")
        .ends_with(".map.ron")
    {
        Road::Map
    } else {
        Road::TilesetDoc
    }
}

/// The folder every application tells into — the well-known place
/// the debugger lists and the bench listens at.
pub(crate) const ATTACH_DIR: &str = "target/frost/live";

/// The App menu: the bench's ear for running applications. Attach
/// cycles the announced folder — the debugger's list of living
/// tellings; while attached the bench rereads each telling and
/// speaks what it says: rev, names, planted beds. Detach ends the
/// conversation. The bench writes nothing into the folder: a tool
/// reads beings and writes only documents it owns.
pub(crate) const APP_MENU: frost::Menu<'static> = frost::Menu {
    title: "App",
    items: &[
        frost::MenuItem::new("Attach", ""),
        frost::MenuItem::new("Detach", ""),
    ],
};

/// One attachment to one running application: what its card
/// announced, the sense on its live file (the same sense the
/// reload ladder runs on — roles swap, machinery doesn't), and the
/// last rev read, so only real tellings are spoken.
pub(crate) struct Attachment {
    pub(crate) card: frost::Announcement,
    pub(crate) watch: frost::Watch,
    pub(crate) rev: u64,
}

/// The Operations menu: the verbs that edit the active sprite's pixels.
/// The panel that once carried them is gone — the menu is their home.
pub(crate) const OPERATIONS_MENU: frost::Menu<'static> = frost::Menu {
    title: "Operations",
    items: &[frost::MenuItem::new("Crop", "")],
};

/// The shape of a chosen list once the desk's law has read it:
/// the one map allowed, the maps it refused, and the tilesets
/// waiting to join — each already knowing which road it takes
/// (document first, sheet second).
pub(crate) struct Selection<'a> {
    pub(crate) map: Option<&'a std::path::PathBuf>,
    pub(crate) refused: usize,
    pub(crate) tilesets: Vec<(
        Option<&'a std::path::PathBuf>,
        Option<&'a std::path::PathBuf>,
    )>,
}

impl Demo {
    /// Open a sprite from `path`: load its texture, minimize the original
    /// to a slot thumbnail, and add it to the next free slot — which
    /// becomes the active one. A full house or an unloadable file is
    /// reported on the status line, not fatal.
    pub(crate) fn load_sprite(&mut self, ctx: &mut frost::Context, path: std::path::PathBuf) {
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
    pub(crate) fn open_dialog(&mut self, ctx: &mut frost::Context) {
        // Each desk opens its own thing: the map desk goes straight
        // to a map; the tile-set desk answers two roads — a PNG is a
        // new tile set awaiting its cut, a RON one already remembered
        // — and the sprite desk keeps its single door.
        if self.view == View::TileMap {
            self.open_map_or_tileset(ctx);
            return;
        }
        let tileset = self.view == View::TileSet;
        let mut dialog = rfd::FileDialog::new()
            .set_title(if tileset {
                "sprite_util: open tile sets"
            } else {
                "sprite_util: open sprites"
            })
            .add_filter("PNG images", &["png"]);
        if tileset {
            dialog = dialog.add_filter("RON documents", &["ron"]);
        }
        if let Some(dir) = &self.dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(window) = ctx.window() {
            dialog = dialog.set_parent(window);
        }
        // Many at once: the dialog's list walks the same two roads,
        // in the order they were chosen — sprites join the bench in
        // a string of loads, each keeping its own verdict.
        match dialog.pick_files() {
            Some(paths) if !paths.is_empty() => {
                for path in paths {
                    if tileset
                        && path
                            .extension()
                            .is_some_and(|e| e.eq_ignore_ascii_case("ron"))
                    {
                        self.open_tileset_doc(ctx, path);
                    } else {
                        self.load_sprite(ctx, path);
                    }
                }
            }
            _ => self.status = String::from("open: canceled"),
        }
    }

    /// The map desk's sorting of a chosen list: the FIRST map (if
    /// any) opens as the bench's one and only picture, every map
    /// after it waits on disk — one map per bench is the desk's
    /// law — and every tileset, document or sheet, keeps its chosen
    /// order to join that picture as layers.
    fn sort_selection(paths: &[std::path::PathBuf]) -> Selection<'_> {
        let mut map = None;
        let mut refused = 0;
        let mut tilesets = Vec::new();
        for path in paths {
            match opened_road(path) {
                Road::Map if map.is_none() => map = Some(path),
                Road::Map => refused += 1,
                Road::TilesetDoc => tilesets.push((Some(path), None)),
                Road::Sheet => tilesets.push((None, Some(path))),
            }
        }
        Selection {
            map,
            refused,
            tilesets,
        }
    }

    /// The tile-map desk's open, both roads at once: a map (`*.map.ron`)
    /// opens as the desk's one and only map, its layers replacing the
    /// bench; a tileset — remembered document or raw PNG sheet —
    /// joins the current map as a NEW EMPTY LAYER wearing it. Every
    /// layer names a tileset, every tileset names a sheet: the chain
    /// is what the bench shows.
    pub(crate) fn open_map_or_tileset(&mut self, ctx: &mut frost::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("sprite_util: open a tile map (one) and tile sets")
            .add_filter("RON documents", &["ron"])
            .add_filter("PNG images", &["png"]);
        let dir = self
            .map_path
            .as_ref()
            .and_then(|p| p.parent())
            .filter(|d| !d.as_os_str().is_empty())
            .or(self.dir.as_deref())
            .map(std::path::Path::to_path_buf);
        if let Some(dir) = &dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(window) = ctx.window() {
            dialog = dialog.set_parent(window);
        }
        let Some(paths) = dialog.pick_files() else {
            self.status = String::from("open: canceled");
            return;
        };
        if paths.is_empty() {
            self.status = String::from("open: canceled");
            return;
        }
        let sel = Self::sort_selection(&paths);
        // The picture first — its layers replace the bench — then
        // every tileset the choice carried, each growing an empty
        // layer on the map that just opened (or the one already
        // standing, if the choice carried no map).
        if let Some(map) = sel.map {
            self.open_map_at(ctx, map.clone());
        }
        for (doc, sheet) in sel.tilesets {
            self.add_tileset_layer(ctx, doc.cloned(), sheet.cloned());
        }
        if sel.refused > 0 {
            // The law speaks plainly when a choice breaks it.
            self.status = format!(
                "map: one at a time — {refused} left on disk",
                refused = sel.refused
            );
        }
    }

    /// The tileset's road onto the map desk, whether remembered (a
    /// `.ron` document speaking name, sheet, cut and roles) or raw (a
    /// PNG sheet named after itself, wearing its sidecar's cut — or
    /// none yet). The sheet walks to the bench, the document — when
    /// there is one — to the shelf, and the map grows an empty layer
    /// wearing the tileset's spoken name: as large as the layers it
    /// joins, a field of its own when it is the first. The highlight
    /// moves to the newcomer — what you loaded is what you draw with.
    pub(crate) fn add_tileset_layer(
        &mut self,
        ctx: &mut frost::Context,
        doc: Option<std::path::PathBuf>,
        sheet_choice: Option<std::path::PathBuf>,
    ) {
        let (spoken, sheet) = match (&doc, sheet_choice) {
            (Some(path), _) => {
                let name = file_name_of(path);
                let Ok(src) = std::fs::read_to_string(path) else {
                    self.status = format!("tile set: cannot read {name}");
                    return;
                };
                let (ts, refused) = frost::tileset::from_text(&src);
                let Some(ts) = ts.filter(|_| refused.is_empty()) else {
                    let why = refused.first().map(String::as_str).unwrap_or("no tile set");
                    self.status = format!("tile set: {name} holds no tile set ({why})");
                    return;
                };
                let sheet = sheet_beside(path, &ts.sheet);
                (Some(ts), sheet)
            }
            (None, Some(sheet)) => (None, sheet),
            (None, None) => return,
        };
        // The sheet to the bench: wake its slot, or carry it in.
        match self.sprites.iter().position(|sp| sp.path == sheet) {
            Some(i) => {
                self.active = i;
                self.selection = None;
                self.last_click = None;
            }
            None => {
                self.load_sprite(ctx, sheet.clone());
                if self.active().is_none_or(|sp| sp.path != sheet) {
                    return; // load_sprite spoke the refusal
                }
            }
        }
        let ts = match spoken {
            Some(ts) => {
                // The document's cut replaces the bench's, sidecar
                // panel and all — the tile set mode's law, kept here.
                self.stamp();
                if let Some(sp) = self.sprites.get_mut(self.active)
                    && sp.atlas != Some((ts.rows, ts.cols))
                {
                    sp.atlas = Some((ts.rows, ts.cols));
                    if let Some(sdoc) = sp.ron.as_mut()
                        && set_atlas(&mut sdoc.root, Some((ts.rows, ts.cols)))
                    {
                        sdoc.rows = ron_view::layout(&sdoc.root);
                        sdoc.clamp_scroll();
                    }
                }
                if let Some(path) = &doc {
                    if let Some(e) = self.shelf.iter_mut().find(|e| e.tileset.name == ts.name) {
                        e.path = path.clone();
                        e.tileset = ts.clone();
                    } else {
                        self.shelf.push(ShelfTileset {
                            path: path.clone(),
                            tileset: ts.clone(),
                        });
                    }
                }
                ts
            }
            None => {
                // A raw sheet speaks for itself: named after itself,
                // one tile until the panel says otherwise.
                let (rows, cols) = self.active().and_then(|sp| sp.atlas).unwrap_or((1, 1));
                frost::tileset::Tileset::new(asset_name(&sheet), file_name_of(&sheet), rows, cols)
            }
        };
        let (cols, rows) = self
            .maps
            .first()
            .map_or((MAP_COLS, MAP_ROWS), |m| (m.cols, m.rows));
        self.stamp();
        self.maps.push(MapLayer::blank(ts.name.clone(), cols, rows));
        self.align_layers();
        let last = self.maps.len() - 1;
        self.select_layer(last);
        self.sync_work(ctx);
        self.refresh_slots(ctx);
        self.status = format!("map: layer '{}' joins the bench", ts.name);
    }

    /// Open Tile Set, the document's road: the `.ron` speaks name,
    /// sheet, cut and roles; the sheet it names is adopted onto the
    /// bench — or its open slot is woken — and the cut the document
    /// wears replaces the bench's, the way the panel sets it: the
    /// document is the cut's memory, and on this road memory wins.
    pub(crate) fn open_tileset_doc(&mut self, ctx: &mut frost::Context, path: std::path::PathBuf) {
        let name = file_name_of(&path);
        let Ok(src) = std::fs::read_to_string(&path) else {
            self.status = format!("tile set: cannot read {name}");
            return;
        };
        let (ts, refused) = frost::tileset::from_text(&src);
        let Some(ts) = ts.filter(|_| refused.is_empty()) else {
            let why = refused.first().map(String::as_str).unwrap_or("no tile set");
            self.status = format!("tile set: {name} holds no tile set ({why})");
            return;
        };
        let sheet = sheet_beside(&path, &ts.sheet);
        let bench = self
            .sprites
            .iter()
            .map(|sp| (sp.path.as_path(), sp.doc.as_deref()))
            .collect::<Vec<_>>();
        let wake = |demo: &mut Self, i: usize| {
            demo.active = i;
            demo.selection = None;
            demo.last_click = None;
        };
        match slot_for_doc(&bench, &sheet, &path) {
            SlotChoice::Wake(i) => wake(self, i),
            SlotChoice::Adopt(i) => {
                wake(self, i);
                self.sprites[i].doc = Some(path.clone());
            }
            SlotChoice::Fresh => {
                self.load_sprite(ctx, sheet.clone());
                if self.active().is_none_or(|sp| sp.path != sheet) {
                    // load_sprite spoke the refusal; no more to say.
                    return;
                }
                self.sprites[self.active].doc = Some(path.clone());
            }
        }
        self.stamp();
        if let Some(sp) = self.sprites.get_mut(self.active)
            && sp.atlas != Some((ts.rows, ts.cols))
        {
            sp.atlas = Some((ts.rows, ts.cols));
            if let Some(doc) = sp.ron.as_mut()
                && set_atlas(&mut doc.root, Some((ts.rows, ts.cols)))
            {
                doc.rows = ron_view::layout(&doc.root);
                doc.clamp_scroll();
            }
        }
        if let Some(e) = self.shelf.iter_mut().find(|e| e.tileset.name == ts.name) {
            e.path = path;
            e.tileset = ts.clone();
        } else {
            self.shelf.push(ShelfTileset {
                path,
                tileset: ts.clone(),
            });
        }
        self.sync_work(ctx);
        self.refresh_slots(ctx);
        self.status = format!("tile set: opened '{}' from {name}", ts.name);
    }

    /// The document the active tile set would save, as pure data:
    /// where it lives — the shelf's place when one remembers it, the
    /// desk's auto-shelf beside the sheet otherwise — and what it
    /// says: the bench's cut worn now, the shelf's roles kept. The
    /// file itself is Save Tile Set's to write.
    fn tileset_to_save(
        sheet: &std::path::Path,
        cut: (usize, usize),
        known: Option<&ShelfTileset>,
    ) -> (std::path::PathBuf, frost::tileset::Tileset) {
        match known {
            Some(e) => {
                let mut ts = e.tileset.clone();
                ts.rows = cut.0;
                ts.cols = cut.1;
                (e.path.clone(), ts)
            }
            None => {
                let mut path = sheet.to_path_buf();
                path.set_extension("tileset.ron");
                let ts = frost::tileset::Tileset::new(
                    asset_name(sheet),
                    file_name_of(sheet),
                    cut.0,
                    cut.1,
                );
                (path, ts)
            }
        }
    }

    /// Save Tile Set: the document is the work. The bench's cut and
    /// the shelf's roles go to the file — the sheet is never touched.
    /// Pixels are the sprite's affair, edited where sprites are
    /// edited; this verb owns only what the tile set is.
    /// A tile set re-shelved under a new name, as pure data: the
    /// spoken name follows the chosen file — the desk keeps name and
    /// shelf in step — the sheet reference is re-voiced from the new
    /// folder, so the chain walks from wherever the document now
    /// stands, and the bench's cut and the shelf's roles ride along.
    fn tileset_as(
        sheet: &std::path::Path,
        cut: (usize, usize),
        known: Option<&ShelfTileset>,
        new_path: &std::path::Path,
    ) -> frost::tileset::Tileset {
        let file = new_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let spoken = file
            .strip_suffix(".tileset.ron")
            .or_else(|| file.strip_suffix(".ron"))
            .unwrap_or(&file)
            .to_string();
        let new_dir = new_path
            .parent()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let sheet_ref = frost::map::relative(&new_dir, &sheet.to_string_lossy());
        let mut ts = known.map(|e| e.tileset.clone()).unwrap_or_else(|| {
            frost::tileset::Tileset::new(spoken.clone(), sheet_ref.clone(), cut.0, cut.1)
        });
        ts.name = spoken;
        ts.sheet = sheet_ref;
        ts.rows = cut.0;
        ts.cols = cut.1;
        ts
    }

    /// Save as Tile Set: the document goes out under a new name. The
    /// sheet is never touched, and the old document — if one stands
    /// where the shelf remembers it — stands untouched too.
    pub(crate) fn save_as_tileset(&mut self, ctx: &mut frost::Context) {
        let Some(sp) = self.active() else {
            self.status = String::from("tile set: no tile set on the bench");
            return;
        };
        let Some(cut) = sp.atlas else {
            self.status = format!(
                "tile set: '{}' waits for its cut — set the grid first",
                sp.name
            );
            return;
        };
        if sp.path.as_os_str().is_empty() {
            self.status = String::from("tile set: the sheet has no file to stand beside");
            return;
        }
        let sheet = sp.path.clone();
        let doc = sp.doc.clone();
        let known = doc
            .as_ref()
            .and_then(|d| self.shelf.iter().find(|e| &e.path == d).cloned())
            .or_else(|| {
                self.shelf
                    .iter()
                    .find(|e| sheet_beside(&e.path, &e.tileset.sheet) == sheet)
                    .cloned()
            });
        let (suggested, dir) = match &known {
            Some(e) => (
                e.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "tileset.ron".to_string()),
                e.path
                    .parent()
                    .filter(|d| !d.as_os_str().is_empty())
                    .map(std::path::Path::to_path_buf),
            ),
            None => (
                format!("{}.tileset.ron", asset_name(&sheet)),
                sheet
                    .parent()
                    .filter(|d| !d.as_os_str().is_empty())
                    .map(std::path::Path::to_path_buf),
            ),
        };
        let mut dialog = rfd::FileDialog::new()
            .set_title("sprite_util: save tile set as")
            .set_file_name(suggested)
            .add_filter("RON documents", &["ron"]);
        if let Some(dir) = &dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(window) = ctx.window() {
            dialog = dialog.set_parent(window);
        }
        let Some(mut target) = dialog.save_file() else {
            self.status = String::from("tile set: canceled");
            return;
        };
        match target.extension() {
            None => {
                target.set_extension("ron");
            }
            Some(ext) if ext.eq_ignore_ascii_case("ron") => {}
            Some(_) => {
                self.status = String::from("tile set: only RON documents");
                return;
            }
        }
        let ts = Self::tileset_as(&sheet, cut, known.as_ref(), &target);
        let who = ts.name.clone();
        let name = file_name_of(&target);
        if target.exists() && !confirm_overwrite(ctx, &name) {
            self.status = String::from("tile set: canceled");
            return;
        }
        match std::fs::write(&target, frost::tileset::to_text(&ts)) {
            Ok(()) => {
                if let Some(sp) = self.sprites.get_mut(self.active) {
                    sp.doc = Some(target.clone());
                }
                // A name is an identity: the shelf entry joins by it.
                // The entry the rename walked away from keeps its
                // file where it stood — the shelf remembers both.
                if let Some(e) = self.shelf.iter_mut().find(|e| e.tileset.name == who) {
                    e.path = target.clone();
                    e.tileset = ts;
                } else {
                    self.shelf.push(ShelfTileset {
                        path: target,
                        tileset: ts,
                    });
                }
                self.status = format!("tile set: wrote '{who}' to {name}");
            }
            Err(err) => self.status = format!("tile set: cannot write {name}: {err}"),
        }
    }

    pub(crate) fn save_tileset(&mut self, ctx: &mut frost::Context) {
        let Some(sp) = self.active() else {
            self.status = String::from("tile set: no tile set on the bench");
            return;
        };
        let Some(cut) = sp.atlas else {
            self.status = format!(
                "tile set: '{}' waits for its cut — set the grid first",
                sp.name
            );
            return;
        };
        if sp.path.as_os_str().is_empty() {
            self.status = String::from("tile set: the sheet has no file to stand beside");
            return;
        }
        let sheet = sp.path.clone();
        let doc = sp.doc.clone();
        // The slot's own document answers first; only a slot wearing
        // none asks the shelf to guess from the sheet.
        let known = doc
            .as_ref()
            .and_then(|d| self.shelf.iter().find(|e| &e.path == d).cloned())
            .or_else(|| {
                self.shelf
                    .iter()
                    .find(|e| sheet_beside(&e.path, &e.tileset.sheet) == sheet)
                    .cloned()
            });
        let (path, ts) = Self::tileset_to_save(&sheet, cut, known.as_ref());
        let who = ts.name.clone();
        let name = file_name_of(&path);
        if path.exists() && !confirm_overwrite(ctx, &name) {
            self.status = String::from("tile set: canceled");
            return;
        }
        match std::fs::write(&path, frost::tileset::to_text(&ts)) {
            Ok(()) => {
                if let Some(sp) = self.sprites.get_mut(self.active) {
                    sp.doc = Some(path.clone());
                }
                if let Some(e) = self.shelf.iter_mut().find(|e| e.tileset.name == who) {
                    e.path = path;
                    e.tileset = ts;
                } else {
                    self.shelf.push(ShelfTileset { path, tileset: ts });
                }
                self.status = format!("tile set: wrote '{who}' to {name}");
            }
            Err(err) => self.status = format!("tile set: cannot write {name}: {err}"),
        }
    }

    /// Crop: to the selection when one is dragged, else to the opaque
    /// content — trimming the fully transparent borders. The sidecar's
    /// positions shift by the cut's origin, clamped to the new bounds, so
    /// they stay put relative to the pixels that survive. The whole world
    /// before the cut goes onto the history, so undo brings back the
    /// pixels, the positions and everything else the stroke touched.
    pub(crate) fn crop(&mut self, ctx: &mut frost::Context) {
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
            doc.rows = ron_view::layout(&doc.root);
            if n > 0 {
                self.status = format!("cropped to {nx} x {ny} px — {n} positions shifted");
            }
        }
    }

    /// Close: take the active sprite out of its slot; every later sprite
    /// shifts one slot left, and the slot the active one leaves (or the
    /// new last one, if it was last) becomes active.
    pub(crate) fn close_active(&mut self, ctx: &mut frost::Context) {
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
    pub(crate) fn close_slot(&mut self, ctx: &mut frost::Context, i: usize) {
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
    pub(crate) fn save(&mut self, ctx: &mut frost::Context) {
        // On the tile-set desk, Save is Save Tile Set — the label
        // the bar wears and the work the button does are one thing.
        if self.view == View::TileSet {
            self.save_tileset(ctx);
            return;
        }
        // On the map desk, Save Tile Map goes quietly back to the
        // map it came from; one never yet saved asks where, like any
        // other Save without a file.
        if self.view == View::TileMap {
            match self.map_path.clone() {
                Some(target) => self.save_map_to(ctx, target),
                None => self.save_map(ctx),
            }
            return;
        }
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
    pub(crate) fn save_as(&mut self, ctx: &mut frost::Context) {
        // On the tile-set desk the verb is Save as Tile Set; on the
        // map desk, Save as Tile Map — the dialog either way.
        if self.view == View::TileSet {
            self.save_as_tileset(ctx);
            return;
        }
        if self.view == View::TileMap {
            self.save_map(ctx);
            return;
        }
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

    /// Write the canvas's layers to a file of their own: one document
    /// for the map, every layer and the tileset that dresses it. The
    /// PNGs stay untouched — this file says what stands where, and
    /// dressed by whom.
    pub(crate) fn save_map(&mut self, ctx: &mut frost::Context) {
        if self.maps.is_empty() {
            self.status = String::from("map: nothing painted to save");
            return;
        }
        // The document names its own save: an opened or saved map
        // speaks its name and opens its folder. Only paint born on
        // the bench, never saved, falls back to naming itself after
        // the sheet it was painted with.
        let bench_dir = self
            .active()
            .and_then(|sp| sp.path.parent())
            .filter(|d| !d.as_os_str().is_empty())
            .or(self.dir.as_deref())
            .map(std::path::Path::to_path_buf);
        let (file, dir) = match &self.map_path {
            Some(p) => (
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "world.map.ron".to_string()),
                p.parent()
                    .filter(|d| !d.as_os_str().is_empty())
                    .map(std::path::Path::to_path_buf)
                    .or(bench_dir),
            ),
            None => (
                format!(
                    "{}.map.ron",
                    self.active().map_or_else(
                        || "world".to_string(),
                        |sp| crate::art::asset_name(&sp.path)
                    )
                ),
                bench_dir,
            ),
        };
        let mut dialog = rfd::FileDialog::new()
            .set_title("sprite_util: save map")
            .set_file_name(file)
            .add_filter("RON documents", &["ron"]);
        if let Some(dir) = &dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(window) = ctx.window() {
            dialog = dialog.set_parent(window);
        }
        let Some(mut target) = dialog.save_file() else {
            self.status = String::from("map: canceled");
            return;
        };
        match target.extension() {
            None => {
                target.set_extension("ron");
            }
            Some(ext) if ext.eq_ignore_ascii_case("ron") => {}
            Some(_) => {
                self.status = String::from("map: only RON documents");
                return;
            }
        }
        self.save_map_to(ctx, target);
    }

    /// The writer half of saving a map: the place is already known —
    /// the dialog's choice, or the path the desk remembers from the
    /// last open or save. The overwrite asks its question here too:
    /// a Save over a file somebody else made is still a replace.
    fn save_map_to(&mut self, ctx: &mut frost::Context, target: std::path::PathBuf) {
        let name = file_name_of(&target);
        if target.exists() && !confirm_overwrite(ctx, &name) {
            self.status = String::from("map: canceled");
            return;
        }
        // The cargo first: every tileset a layer names gets its
        // document written — to the shelf's place beside the sheet —
        // before the map that points at them. A layer whose tileset
        // neither shelf nor bench can dress is refused by name and
        // stays out of the file: a map must not promise a dressing it
        // cannot ship. The layer waits on the bench in memory; the
        // file stays honest about what it carries.
        let map_dir = target
            .parent()
            .map(|d| d.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let mut map_out = frost::map::Map::default();
        let mut carried: Vec<&str> = Vec::new();
        let mut refused: Vec<String> = Vec::new();
        for who in tileset_names(&self.maps) {
            let Some((sp, (rows, cols))) = dressed(who, &self.shelf, &self.sprites) else {
                refused.push(format!("layer '{who}': its tileset waits off the bench"));
                continue;
            };
            let (path, mut ts) = match self.shelf.iter().find(|e| e.tileset.name == who) {
                Some(entry) => (entry.path.clone(), entry.tileset.clone()),
                None => {
                    let mut path = sp.path.clone();
                    path.set_extension("tileset.ron");
                    let sheet = sp
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_else(|| format!("{who}.png"));
                    (path, frost::tileset::Tileset::new(who, sheet, rows, cols))
                }
            };
            ts.rows = rows;
            ts.cols = cols;
            if let Err(err) = std::fs::write(&path, frost::tileset::to_text(&ts)) {
                refused.push(format!("'{who}': its document cannot be written: {err}"));
                continue;
            }
            let reference =
                frost::map::relative(&map_dir, &path.to_string_lossy().replace('\\', "/"));
            if !map_out.tilesets.contains(&reference) {
                map_out.tilesets.push(reference);
            }
            carried.push(who);
        }
        map_out.layers = self
            .maps
            .iter()
            .filter(|m| carried.contains(&m.tileset.as_str()))
            .cloned()
            .collect();
        match std::fs::write(&target, frost::map::to_text(&map_out)) {
            Ok(()) => {
                self.map_path = Some(target.clone());
                self.status = format!("map: wrote {} layers to {name}", map_out.layers.len());
                for why in &refused {
                    log::warn!("map save: {why}");
                }
                if let Some(first) = refused.first() {
                    let more = refused.len() - 1;
                    self.status.push_str(&format!(
                        " — {first}{}",
                        if more > 0 {
                            format!(", and {more} more")
                        } else {
                            String::new()
                        }
                    ));
                }
            }
            Err(err) => {
                log::warn!("map save failed: {err}");
                self.status = format!("map: cannot write {name}: {err}");
            }
        }
    }

    /// The map road with the place already chosen: the loader half,
    /// one stroke — cargo to the shelf, sheets to the bench, layers
    /// to the bench of layers.
    pub(crate) fn open_map_at(&mut self, ctx: &mut frost::Context, target: std::path::PathBuf) {
        let name = file_name_of(&target);
        let Ok(src) = std::fs::read_to_string(&target) else {
            self.status = format!("map: cannot read {name}");
            return;
        };
        let (map, mut refused) = frost::map::from_text(&src);
        if map.layers.is_empty() {
            let why = refused.first().map(String::as_str).unwrap_or("no layers");
            self.status = format!("map: {name} holds no map ({why})");
            return;
        }
        // The cargo comes onto the shelf with the map: every tileset
        // document the file points at, resolved against the map's own
        // path — the one operation the guard and the desk share —
        // read and parsed. A document that answers nothing is named,
        // and its layers wait for it exactly as a closed sprite's do.
        let mut shelf: Vec<ShelfTileset> = Vec::new();
        for reference in &map.tilesets {
            let resolved = frost::map::resolve(&target.to_string_lossy(), reference);
            let path = target
                .parent()
                .map(|dir| dir.join(&resolved))
                .unwrap_or_else(|| std::path::PathBuf::from(&resolved));
            match std::fs::read_to_string(&path) {
                Ok(doc_src) => {
                    let (ts, errs) = frost::tileset::from_text(&doc_src);
                    for why in errs {
                        refused.push(format!("'{reference}': {why}"));
                    }
                    if let Some(ts) = ts
                        && !shelf.iter().any(|e| e.tileset.name == ts.name)
                    {
                        shelf.push(ShelfTileset { path, tileset: ts });
                    }
                }
                Err(err) => refused.push(format!(
                    "tileset '{reference}' answers nothing ({}: {err})",
                    path.display()
                )),
            }
        }
        // The cargo's sheets walk onto the bench themselves. A map
        // is dressed by sprites and the desk paints with what stands
        // under hand — so opening a map opens what its tilesets cut,
        // each sheet resolved from the document that names it, the
        // same operation the guard walks. A sheet already open keeps
        // its slot; one that answers nothing is left to the waiting
        // names below, exactly as a missing identity always was.
        let mut admitted: Vec<String> = Vec::new();
        for entry in &shelf {
            let sheet = sheet_beside(&entry.path, &entry.tileset.sheet);
            if self.sprites.iter().any(|sp| sp.path == sheet) {
                continue;
            }
            self.load_sprite(ctx, sheet.clone());
            admitted.push(file_name_of(&sheet));
        }
        // And the desk turns to face what it opened: the Tile Map
        // view is where the map's layers live. The document is now
        // the desk's own — Save map will answer with its name.
        self.view = View::TileMap;
        self.map_path = Some(target.clone());
        let before = self.capture();
        self.maps = map.layers;
        self.shelf = shelf;
        // A new map is a new stack of memories: every layer's first
        // choosing is owed again, top-left and neutral.
        self.layer_brush.clear();
        self.active_layer = 0;
        self.align_layers();
        self.recall_layer();
        if self.maps != before.maps {
            push_step(&mut self.undo_stack, &mut self.redo_stack, before);
        }
        self.sync_work(ctx);
        self.refresh_slots(ctx);
        let names = tileset_names(&self.maps);
        log::info!("map '{name}': it asks for {}", names.join(", "));
        let waiting = waiting_names(&self.maps, &self.shelf, &self.sprites);
        let mut status = format!("map: opened {} layers from {name}", self.maps.len());
        if !waiting.is_empty() {
            // Named, not counted — the guard's face the desk can
            // show at the opening: who waits, and why (the desk
            // does not wear that identity).
            let spoken = if waiting.len() > 3 {
                format!(
                    "{}, and {} more",
                    waiting[..3].join(", "),
                    waiting.len() - 3
                )
            } else {
                waiting.join(", ")
            };
            status.push_str(&format!(", waiting for their tilesets: {spoken}"));
            for who in &waiting {
                log::warn!("map '{name}': '{who}' waits undressed — the desk does not wear it");
            }
        }
        for why in &refused {
            log::warn!("map '{name}': {why}");
        }
        if let Some(first) = refused.first() {
            let more = refused.len() - 1;
            status.push_str(&format!(
                " — {first}{}",
                if more > 0 {
                    format!(", and {more} more refused")
                } else {
                    String::new()
                }
            ));
        }
        if !admitted.is_empty() {
            status.push_str(&format!(" — sheet to the bench: {}", admitted.join(", ")));
        }
        self.status = status;
    }

    /// Write the active texture to `path` as PNG and make it the sprite's
    /// file — the name and the Save target both follow. Each save target
    /// is compared with the bytes on disk and left alone when equal: a
    /// save that only brings a sidecar into the world no longer re-encodes
    /// pixels it never touched, and an untouched PNG keeps the container
    /// that made it. Save As to a new name is always a change — the new
    /// file knows nothing yet. The slot keeps showing the original: a slot
    /// is a sprite's identity, not its edit.
    pub(crate) fn write_png(&mut self, path: &std::path::Path, name: String) {
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

    /// Attach to the next living telling in the folder: the
    /// debugger's list, cycled, so several running worlds can each
    /// be reached from one bench.
    pub(crate) fn attach(&mut self) {
        let mut cards = frost::Teller::announce(ATTACH_DIR);
        if cards.is_empty() {
            self.status = String::from("app: no running application to attach to");
            return;
        }
        let next = match &self.attached {
            Some(held) => cards
                .iter()
                .position(|c| c.pid == held.card.pid)
                .map(|i| (i + 1) % cards.len())
                .unwrap_or(0),
            None => 0,
        };
        let card = cards.swap_remove(next);
        log::info!("app: attached to {} (pid {})", card.name, card.pid);
        self.attached = Some(Attachment {
            watch: frost::Watch::new(&card.live_file),
            card,
            rev: 0,
        });
        self.read_attached();
    }

    /// The frame's listening: one stat on the card, one on the live
    /// file. The card's absence is the app's death — its documents
    /// may remain, its announcement does not. A changed live file
    /// is a telling: read it and speak what it says.
    pub(crate) fn read_attached(&mut self) {
        if let Some(held) = self.attached.as_mut() {
            if !held.card.card.exists() {
                log::info!("app: {} is gone — its card died with it", held.card.name);
                self.status = format!("app: {} is gone", held.card.name);
                self.attached = None;
                return;
            }
            if !held.watch.changed() {
                return;
            }
            let Some(tide) = frost::Teller::tiding(&held.card) else {
                self.status = format!("app: '{}' told unreadable news", held.card.name);
                return;
            };
            if tide.rev != held.rev {
                let beds = planted_beds(&tide.being);
                let names = spoken_names(&tide.being, "ships");
                log::info!(
                    "app: '{}' tells rev {} — {names} names, {beds} beds planted",
                    held.card.name,
                    tide.rev
                );
                self.status = format!(
                    "app: attached {} (pid {}) — rev {}, {names} names, {beds} beds",
                    held.card.name, held.card.pid, tide.rev
                );
                held.rev = tide.rev;
            }
        }
    }

    /// End the conversation: the app, its sense, and its news are
    /// forgotten. The folder never felt the bench write.
    pub(crate) fn detach(&mut self) {
        match self.attached.take() {
            Some(held) => {
                log::info!("app: detached from {}", held.card.name);
                self.status = format!("app: detached from {}", held.card.name);
            }
            None => self.status = String::from("app: nothing to detach"),
        }
    }
}

/// How many beds a told being says are planted: the entries of the
/// beds record wearing a `true`. Read by name, not by place — the
/// dialect is the contract.
pub(crate) fn planted_beds(being: &frost::ron::Val) -> usize {
    match frost::ron::field(being, "beds") {
        Some(frost::ron::Val::Struct { fields, .. }) => fields
            .iter()
            .filter(|(_, item)| item.val.inline().contains("true"))
            .count(),
        _ => 0,
    }
}

/// How many spoken names a named record of the being holds.
pub(crate) fn spoken_names(being: &frost::ron::Val, what: &str) -> usize {
    match frost::ron::field(being, what) {
        Some(frost::ron::Val::Struct { fields, .. }) => fields.len(),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    /// Save Tile Set's file, pure: a never-shelved sheet is shelved
    /// beside itself, named after itself, wearing the bench's cut;
    /// a shelved document keeps its place and its spoken roles and
    /// takes only the new cut. No dialog, no disk — the shape the
    /// verb writes.
    /// The name tells which road a file takes on the map desk:
    /// `*.map.ron` is a map, any other RON a tileset document,
    /// a PNG a raw sheet.
    /// The chosen list obeys the desk's law: the first map rules,
    /// later maps wait, tilesets join in order — documents as
    /// documents, sheets as sheets.
    #[test]
    fn one_map_rules_the_choice_and_tilesets_join_in_order() {
        use std::path::PathBuf;
        let paths: Vec<PathBuf> = [
            "/m/b.map.ron",
            "/t/forest.tileset.ron",
            "/s/other.map.ron",
            "/s/new_sheet.png",
            "/t/yard.tileset.ron",
        ]
        .map(PathBuf::from)
        .to_vec();
        let sel = Demo::sort_selection(&paths);
        assert_eq!(
            sel.map.map(|p| p.to_path_buf()),
            Some(PathBuf::from("/m/b.map.ron"))
        );
        assert_eq!(sel.refused, 1);
        assert_eq!(
            sel.tilesets,
            vec![
                (Some(&PathBuf::from("/t/forest.tileset.ron")), None),
                (None, Some(&PathBuf::from("/s/new_sheet.png"))),
                (Some(&PathBuf::from("/t/yard.tileset.ron")), None),
            ]
        );
        // No map chosen: the current picture keeps its place, layers
        // still join; only tilesets, none refused.
        let only_tiles: Vec<PathBuf> = ["/t/a.tileset.ron", "/s/b.png"].map(PathBuf::from).to_vec();
        let sel = Demo::sort_selection(&only_tiles);
        assert!(sel.map.is_none());
        assert_eq!(sel.refused, 0);
        assert_eq!(sel.tilesets.len(), 2);
    }

    #[test]
    fn the_name_tells_which_road_a_file_takes() {
        use std::path::Path;
        assert_eq!(
            opened_road(Path::new("/m/courtyard.map.ron")),
            Road::Map,
            "the map wears its office"
        );
        assert_eq!(
            opened_road(Path::new("/s/basic_tiles.tileset.ron")),
            Road::TilesetDoc
        );
        assert_eq!(
            opened_road(Path::new("/s/handwritten.ron")),
            Road::TilesetDoc,
            "any other RON is a document"
        );
        assert_eq!(opened_road(Path::new("/s/basic_tiles.png")), Road::Sheet);
        assert_eq!(opened_road(Path::new("/s/odd.PNG")), Road::Sheet);
    }

    /// One sheet, two documents: the document already worn wakes its
    /// slot; a bare sheet adopts the opened document; a sheet already
    /// worn by another document makes room for a new slot — two tile
    /// sets may wear one sheet, and never fight over one cut.
    #[test]
    fn two_tile_sets_may_wear_one_sheet() {
        use std::path::Path;
        let sheet = Path::new("/a/sheet.png");
        let doc1 = Path::new("/a/one.tileset.ron");
        let doc2 = Path::new("/a/two.tileset.ron");
        let empty: Vec<(&Path, Option<&Path>)> = Vec::new();
        assert_eq!(slot_for_doc(&empty, sheet, doc1), SlotChoice::Fresh);
        let bare = vec![(sheet, None)];
        assert_eq!(slot_for_doc(&bare, sheet, doc1), SlotChoice::Adopt(0));
        let worn = vec![(sheet, Some(doc1))];
        assert_eq!(slot_for_doc(&worn, sheet, doc1), SlotChoice::Wake(0));
        assert_eq!(slot_for_doc(&worn, sheet, doc2), SlotChoice::Fresh);
    }

    /// Save as Tile Set, pure: the spoken name follows the chosen
    /// file — `green.tileset.ron` speaks "green" — the sheet is
    /// re-voiced from the new folder, and a shelved document's roles
    /// ride along to the new name.
    #[test]
    fn a_tile_set_renamed_revoices_its_chain() {
        let sheet = std::path::Path::new("/work/assets/sprites/moss.png");
        let fresh = Demo::tileset_as(
            sheet,
            (1, 4),
            None,
            std::path::Path::new("/work/assets/sprites/green.tileset.ron"),
        );
        assert_eq!(fresh.name, "green", "the name follows the shelf");
        assert_eq!(
            fresh.sheet, "moss.png",
            "beside it, the sheet is named bare"
        );
        assert_eq!((fresh.rows, fresh.cols), (1, 4));
        let mut shelved = frost::tileset::Tileset::new("moss", "moss.png", 1, 1);
        shelved.set_solid(2, true);
        let known = ShelfTileset {
            path: std::path::PathBuf::from("/work/assets/sprites/moss.tileset.ron"),
            tileset: shelved,
        };
        let moved = Demo::tileset_as(
            sheet,
            (2, 2),
            Some(&known),
            std::path::Path::new("/work/sets/green.ron"),
        );
        assert_eq!(moved.name, "green");
        assert_eq!(
            moved.sheet, "../assets/sprites/moss.png",
            "the reference walks from the new folder"
        );
        assert!(moved.is_solid(2), "roles ride along to the new name");
        assert_eq!((moved.rows, moved.cols), (2, 2));
    }

    #[test]
    fn the_tile_set_saves_its_own_document() {
        let sheet = std::path::Path::new("/work/assets/sprites/moss.png");
        let (path, ts) = Demo::tileset_to_save(sheet, (2, 4), None);
        assert_eq!(
            path,
            std::path::PathBuf::from("/work/assets/sprites/moss.tileset.ron"),
            "the auto-shelf stands beside the sheet"
        );
        assert_eq!(
            (ts.name.as_str(), ts.sheet.as_str(), ts.rows, ts.cols),
            ("moss", "moss.png", 2, 4)
        );
        let mut shelved = frost::tileset::Tileset::new("moss", "../sprites/moss.png", 1, 1);
        shelved.set_solid(3, true);
        let known = ShelfTileset {
            path: std::path::PathBuf::from("/work/sets/moss.ron"),
            tileset: shelved,
        };
        let (path, ts) = Demo::tileset_to_save(sheet, (2, 4), Some(&known));
        assert_eq!(path, std::path::PathBuf::from("/work/sets/moss.ron"));
        assert_eq!(
            (ts.rows, ts.cols),
            (2, 4),
            "the bench's cut is the current word"
        );
        assert!(ts.is_solid(3), "roles the document spoke survive the save");
    }
    use super::*;

    /// The bench reads a being by name, not by place: beds counted
    /// by their `true`, names counted from the record it was told
    /// to count, and silence read as none. The dialect is the
    /// contract — order is not.
    #[test]
    fn a_being_is_read_by_name_not_by_place() {
        let being = frost::ron::record_named(&[
            ("told", frost::ron::text("immortal")),
            (
                "ships",
                frost::ron::record(vec![frost::ron::text("grass"), frost::ron::text("bug")]),
            ),
            (
                "beds",
                frost::ron::record(vec![
                    frost::ron::record(vec![frost::ron::number(0.0), frost::ron::flag(true)]),
                    frost::ron::record(vec![frost::ron::number(1.0), frost::ron::flag(false)]),
                ]),
            ),
        ]);
        assert_eq!(spoken_names(&being, "ships"), 2);
        assert_eq!(planted_beds(&being), 1);
        assert_eq!(
            spoken_names(&being, "absent"),
            0,
            "a silent field reads as none"
        );
    }
}
