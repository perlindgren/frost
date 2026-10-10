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
use frost::ron as ron_tree;

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

/// The Operations menu: the verbs that edit the active sprite's pixels.
/// The panel that once carried them is gone — the menu is their home.
pub(crate) const OPERATIONS_MENU: frost::Menu<'static> = frost::Menu {
    title: "Operations",
    items: &[frost::MenuItem::new("Crop", "")],
};

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
        let stem = self.active().map_or_else(
            || "world".to_string(),
            |sp| crate::art::asset_name(&sp.path),
        );
        let mut dialog = rfd::FileDialog::new()
            .set_title("sprite_util: save map")
            .set_file_name(format!("{stem}.map.ron"))
            .add_filter("RON documents", &["ron"]);
        let dir = self
            .active()
            .and_then(|sp| sp.path.parent())
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
        let name = file_name_of(&target);
        if target.exists() && !confirm_overwrite(ctx, &name) {
            self.status = String::from("map: canceled");
            return;
        }
        let text = ron_tree::to_text_doc(MAP_FILE_HEADER, &map_value(&self.maps), "");
        match std::fs::write(&target, text) {
            Ok(()) => self.status = format!("map: wrote {} layers to {name}", self.maps.len()),
            Err(err) => {
                log::warn!("map save failed: {err}");
                self.status = format!("map: cannot write {name}: {err}");
            }
        }
    }

    /// Open a map file: the canvas becomes the file's layers, and the
    /// undo road remembers the canvas that was — opening a map is a
    /// step like any other. Tilesets off the bench keep their layers
    /// waiting, dressed again when their source opens.
    pub(crate) fn open_map(&mut self, ctx: &mut frost::Context) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("sprite_util: open map")
            .add_filter("RON documents", &["ron"]);
        if let Some(dir) = &self.dir {
            dialog = dialog.set_directory(dir);
        }
        if let Some(window) = ctx.window() {
            dialog = dialog.set_parent(window);
        }
        let Some(target) = dialog.pick_file() else {
            self.status = String::from("map: canceled");
            return;
        };
        let name = file_name_of(&target);
        let Ok(src) = std::fs::read_to_string(&target) else {
            self.status = format!("map: cannot read {name}");
            return;
        };
        let Ok(doc) = ron_tree::parse_doc(&src) else {
            self.status = format!("map: {name} is not RON");
            return;
        };
        let (layers, refused) = map_of(&doc.root);
        if layers.is_empty() {
            let why = refused.first().map(String::as_str).unwrap_or("no layers");
            self.status = format!("map: {name} holds no map ({why})");
            return;
        }
        let before = self.capture();
        self.maps = layers;
        if self.maps != before.maps {
            push_step(&mut self.undo_stack, &mut self.redo_stack, before);
        }
        self.sync_work(ctx);
        let names = tileset_names(&self.maps);
        log::info!("map '{name}': it asks for {}", names.join(", "));
        let waiting = waiting_names(&self.maps, &self.sprites);
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
}
