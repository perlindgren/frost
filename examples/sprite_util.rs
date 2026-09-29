//! A little sprite utility: pick any PNG with a native file dialog (rfd),
//! display it in the window over a transparency checkerboard with a
//! bounding box around it, zoom it with a slider or the mouse wheel, pan
//! it by grabbing the canvas with a right-drag, and log the clicked
//! position in the sprite's own pixel space through the `log` facade.
//!
//! The dialog is a child of the window, so it opens on top of it: the
//! window is created first and the dialog is parented to it on the first
//! frame (a dialog without a parent opens at the screen's default spot,
//! far from the window). It is modal, so the frame loop blocks until a
//! file is picked; canceling quits.
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
//! The "Operations" panel edits the sprite itself: **Crop** cuts the
//! texture to the selection rectangle if one is dragged (a left-drag over
//! the canvas draws it; a click that barely moves instead logs the pixel,
//! as before) — and to the opaque content when there is no selection,
//! trimming the fully transparent borders. Every crop is remembered, so
//! **Ctrl-Z** undoes it, walking back step by step to the original.
//! **Save** writes the texture over the file it came from, and **Save
//! As** asks a native dialog for a new name, defaulted to the sprite's
//! current file name; both ask for confirmation before overwriting an
//! existing file.
//!
//! The wheel zooms about the cursor (the point under it stays put), a
//! right-drag grabs the canvas and moves the sprite with the cursor, and
//! [`frost::Ui::hovering`] is what tells a click on the UI from one on
//! the scene: a press the UI claims never draws a selection or logs a
//! pixel.
//!
//! The checkerboard is the sprite's on-screen bounding box, clipped to the
//! window, filled with light/dark grey cells that never scale with the
//! sprite, so it reveals the sprite's transparency at any zoom or pan.
//! The fill is a tiny sprite — one texture pixel per cell, nearest-
//! neighbor sampled through [`frost::Shape::sprite_bytes_nearest`] — so
//! the cell edges stay hard at any window size, and the texture is
//! rebuilt only when the visible cell count or a grey level changes. The
//! bounding box itself is four [`frost::Canvas::line`] strokes around the
//! sprite's full on-screen rectangle; the parts outside the window are
//! clipped.
//!
//! Run with:
//!
//! ```text
//! cargo run --example sprite_util
//! ```
//!
//! (Set `RUST_LOG=info` to see the click lines.)
//!
//! The sprite's source can be given on the command line with
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

use clap::Parser;
use image::ImageEncoder;

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
/// count as a click (log the pixel) rather than a selection drag.
const CLICK_TOL: f32 = 4.0;

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
/// The crop selection's rectangle.
const SELECT: frost::Color = frost::Color {
    r: 0.35,
    g: 0.72,
    b: 1.0,
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

/// The demo's state.
struct Demo {
    /// The widget layer: the View and Operations panels.
    ui: frost::Ui,
    /// The current zoom: the sprite's scale factor (1.0 is texture size).
    zoom: f32,
    /// The sprite's pan offset from the window's center, in window pixels;
    /// set by right-dragging and by the wheel's anchor.
    offset: [f32; 2],
    /// The sprite's current texture size, in pixels.
    size: [f32; 2],
    /// The file the sprite was loaded from — Save writes back to it.
    path: std::path::PathBuf,
    /// The picked file's name, for the HUD, the dialogs and the logs.
    name: String,
    /// The working texture: crops replace it, saves write it.
    current: image::RgbaImage,
    /// The texture snapshots each crop replaced: Ctrl-Z pops them back,
    /// and the history bottom is the original.
    history: Vec<image::RgbaImage>,
    /// The crop selection in texture pixels, `[x0, y0, x1, y1]` (y down):
    /// dragged with the left button, consumed by Crop.
    selection: Option<[f32; 4]>,
    /// Where the current left-button press began, in window pixels — only
    /// set when the press was the canvas's, not the UI's.
    drag_from: Option<[f32; 2]>,
    /// Whether Ctrl-Z was held last frame: its rising edge is the undo.
    was_undo: bool,
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
    /// The last reported click: its position in the texture's pixel space,
    /// and whether the spot was inside the texture.
    last_click: Option<([f32; 2], bool)>,
    /// The folder the file dialog opens in, still to pick: `Some` until
    /// the first frame has shown the dialog. A picked file installs the
    /// sprite and clears it; canceling quits the program.
    pending: Option<std::path::PathBuf>,
    /// The Operations panel's status line: the selection's size, or the
    /// outcome of the last operation.
    status: String,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, _dt: f32) {
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
            let Some(path) = dialog.pick_file() else {
                log::warn!("no file selected — quitting");
                std::process::exit(0);
            };
            // Install the pick (or exit on an unloadable file); the first
            // interactive frame is the next one.
            install_file(self, ctx, path);
            return;
        }

        let (w, h) = ctx.size();
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let rdown = ctx.mouse_button_down(frost::MouseButton::Right);
        let pressed = down && !self.was_down;
        let released = !down && self.was_down;
        self.was_down = down;
        let pos = ctx.mouse_position();

        // The wheel zooms: multiplicative per line (up zooms in), clamped
        // to the slider's range, anchored so the texture point under the
        // cursor stays put — the offset absorbs the scale change the cursor
        // itself would have drifted. With the cursor outside the window the
        // anchor is the center, so only the zoom changes.
        let wheel = ctx.mouse_wheel();
        if wheel != 0.0 {
            let z1 = (self.zoom * WHEEL_ZOOM.powf(wheel)).clamp(ZOOM_MIN, ZOOM_MAX);
            if z1 != self.zoom {
                if let Some([mx, my]) = pos {
                    let k = z1 / self.zoom;
                    self.offset = [
                        mx - (mx - self.offset[0]) * k,
                        my - (my - self.offset[1]) * k,
                    ];
                }
                self.zoom = z1;
            }
        }

        // A right-drag grabs the canvas: the sprite follows the cursor's
        // frame-to-frame movement, so it always lands under the pointer.
        if rdown && let (Some([lx, ly]), Some([mx, my])) = (self.last_mouse, pos) {
            self.offset[0] += mx - lx;
            self.offset[1] += my - ly;
        }

        // The UI frame: the View panel's sliders read and write the demo's
        // values, and both panels' bodies claim the mouse over them — a
        // press the UI holds is never a selection drag or a sprite click.
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
        let mut want_crop = false;
        let mut want_save = false;
        let mut want_save_as = false;
        let edited = !self.history.is_empty();
        let title = format!("{}{}", self.name, if edited { " *" } else { "" });
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
                if ui.button(ctx, "crop") {
                    want_crop = true;
                }
                if ui.button(ctx, "save") {
                    want_save = true;
                }
                if ui.button(ctx, "save as") {
                    want_save_as = true;
                }
                ui.space(6.0);
                ui.label(ctx, &ops_line);
                ui.label(ctx, "ctrl-z undoes the last crop");
            },
        );

        // Ctrl-Z's rising edge walks one step back through the crops.
        let ctrl =
            ctx.key_down(frost::KeyCode::ControlLeft) || ctx.key_down(frost::KeyCode::ControlRight);
        let undo = ctrl && ctx.key_down(frost::KeyCode::KeyZ);
        if undo && !self.was_undo {
            self.undo(ctx);
        }
        self.was_undo = undo;

        // A left-drag the UI did not claim draws the crop selection in the
        // texture's pixel space; a release that barely moved is a click —
        // it logs the sprite pixel, as before, and clears the selection.
        if pressed
            && !self.ui.hovering()
            && let Some(p) = pos
        {
            self.drag_from = Some(p);
        }
        if down && let (Some(from), Some(p)) = (self.drag_from, pos) {
            self.selection = sel_rect_from(from, p, self.size, self.offset, self.zoom);
        }
        if released
            && let Some(from) = self.drag_from.take()
            && let Some([mx, my]) = pos
        {
            let moved = ((mx - from[0]).powi(2) + (my - from[1]).powi(2)).sqrt();
            if moved < CLICK_TOL {
                // The click's position relative to the sprite. The sprite
                // sits at the window's center plus `offset`, at scale
                // `zoom`, so window coordinates map to texture pixels with
                // `px = (x - ox) / zoom + w/2` and
                // `py = h/2 - (y - oy) / zoom` — the y flip included,
                // since the texture's y grows down.
                let [tw, th] = self.size;
                let [px, py] = tex_point([mx, my], self.size, self.offset, self.zoom);
                let inside = px >= 0.0 && px <= tw && py >= 0.0 && py <= th;
                log::info!(
                    "click at sprite pixel ({px:.1}, {py:.1}) of {tw:.0}x{th:.0} '{}' — {where}",
                    self.name,
                    where = if inside {
                        "inside the texture"
                    } else {
                        "outside the texture"
                    }
                );
                self.last_click = Some(([px, py], inside));
                self.selection = None;
            }
        }

        // The collected button presses, now outside every panel closure.
        if want_crop {
            self.crop(ctx);
        }
        if want_save {
            self.save(ctx);
        }
        if want_save_as {
            self.save_as(ctx);
        }

        // The sprite's node carries the pan and the zoom: a uniform scale
        // about its center, then the pan offset from the window's center.
        let sprite = &mut ctx.scene().root.children[0];
        sprite.transform = frost::Transform::translate(self.offset);
        sprite.scale = [self.zoom, self.zoom];

        // The checker backdrop: the sprite's on-screen bounding box,
        // clipped to the window, filled with cells that never scale with
        // the sprite — so the pattern reveals the sprite's transparency
        // no matter the zoom or the pan. The fill is a tiny sprite, one
        // texture pixel per cell, sampled with nearest-neighbor filtering
        // so the cell edges stay hard; the texture is rebuilt only when
        // the cell count or a grey level changes, so smooth pans and zooms
        // only reposition and rescale it.
        let [tw, th] = self.size;
        let (hw, hh) = ((tw * self.zoom) / 2.0, (th * self.zoom) / 2.0);
        let (bx0, by0) = (self.offset[0] - hw, self.offset[1] - hh);
        let (bx1, by1) = (self.offset[0] + hw, self.offset[1] + hh);
        // The box's intersection with the window.
        let (rx0, ry0, rx1, ry1) = (
            bx0.max(-w / 2.0),
            by0.max(-h / 2.0),
            bx1.min(w / 2.0),
            by1.min(h / 2.0),
        );
        let checker = &mut ctx.scene().root.children[3];
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
            // No sprite (or its box off screen): hide the backdrop and
            // force a rebuild when it comes back.
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
            let wx = |px: f32| (px - tw / 2.0) * self.zoom + self.offset[0];
            let wy = |py: f32| (th / 2.0 - py) * self.zoom + self.offset[1];
            let (sx0, sx1) = (wx(x0), wx(x1));
            let (sy0, sy1) = (wy(y0), wy(y1));
            ctx.line(sx0, sy0, sx1, sy0, SELECT, BBOX_WIDTH, SELECT_Z);
            ctx.line(sx1, sy0, sx1, sy1, SELECT, BBOX_WIDTH, SELECT_Z);
            ctx.line(sx1, sy1, sx0, sy1, SELECT, BBOX_WIDTH, SELECT_Z);
            ctx.line(sx0, sy1, sx0, sy0, SELECT, BBOX_WIDTH, SELECT_Z);
        }

        // The usage line, pinned near the top edge, so resizing keeps it
        // in place.
        let help = &mut ctx.scene().root.children[1];
        help.transform = frost::Transform::translate([0.0, h / 2.0 - 28.0]);

        // The marker dot rides the last click in window space — the inverse
        // of the conversion above — and shrinks away when there is none.
        let marker = &mut ctx.scene().root.children[2];
        if let Some(([px, py], _)) = self.last_click {
            let [tw, th] = self.size;
            marker.transform = frost::Transform::translate([
                (px - tw / 2.0) * self.zoom + self.offset[0],
                (th / 2.0 - py) * self.zoom + self.offset[1],
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

impl Demo {
    /// Crop: to the selection when one is dragged, else to the opaque
    /// content — trimming the fully transparent borders. The replaced
    /// texture goes on the history stack, so undo can walk it back.
    fn crop(&mut self, ctx: &mut frost::Context) {
        if self.current.width() == 0 {
            return;
        }
        let (w, h) = (self.current.width(), self.current.height());
        let rect = match self.selection {
            Some(sel) => sel_int_rect(sel, w, h),
            None => match alpha_bbox(&self.current) {
                None => {
                    self.status = String::from("crop: the sprite is fully transparent");
                    return;
                }
                Some(r) if r == (0, 0, w, h) => {
                    self.status = String::from("crop: nothing to trim — drag to select");
                    return;
                }
                Some(r) => Some(r),
            },
        };
        match rect {
            Some((x, y, cw, ch)) => {
                let cut = image::imageops::crop_imm(&self.current, x, y, cw, ch).to_image();
                let (nx, ny) = (cut.width(), cut.height());
                self.history.push(self.current.clone());
                self.install(ctx, cut, format!("cropped to {nx} x {ny} px"));
            }
            None => self.status = String::from("crop: the selection is empty"),
        }
    }

    /// One step back through the crops; the history ends at the original.
    fn undo(&mut self, ctx: &mut frost::Context) {
        match self.history.pop() {
            Some(prev) => {
                let (w, h) = (prev.width(), prev.height());
                self.install(ctx, prev, format!("undo: back to {w} x {h} px"));
            }
            None => self.status = String::from("undo: already at the original"),
        }
    }

    /// Save: write the texture over the file it was loaded from, asking
    /// first — that file exists, by definition of overwriting it.
    fn save(&mut self, ctx: &mut frost::Context) {
        if self.current.width() == 0 {
            return;
        }
        if self.path.as_os_str().is_empty() {
            self.status = String::from("save: no file to overwrite — use save as");
            return;
        }
        let name = file_name_of(&self.path);
        if self.path.exists() && !confirm_overwrite(ctx, &name) {
            self.status = String::from("save: canceled");
            return;
        }
        let target = self.path.clone();
        self.write_png(&target, name);
    }

    /// Save As: a native save dialog defaulted to the sprite's current
    /// file name; an existing target asks the same overwrite confirmation.
    fn save_as(&mut self, ctx: &mut frost::Context) {
        if self.current.width() == 0 {
            return;
        }
        let mut dialog = rfd::FileDialog::new()
            .set_title("sprite_util: save as")
            .set_file_name(&self.name)
            .add_filter("PNG images", &["png"]);
        if let Some(dir) = self.path.parent().filter(|d| !d.as_os_str().is_empty()) {
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

    /// Write the working texture to `path` as PNG and make it the sprite's
    /// file — the name and the Save target both follow.
    fn write_png(&mut self, path: &std::path::Path, name: String) {
        let done = png_bytes(&self.current)
            .and_then(|png| std::fs::write(path, png).map_err(|e| e.to_string()));
        match done {
            Ok(()) => {
                log::info!("saved '{name}'");
                self.path = path.to_path_buf();
                self.name = name.clone();
                self.status = format!("saved '{name}'");
            }
            Err(err) => self.status = format!("save failed: {err}"),
        }
    }

    /// Put a texture on screen: re-encode it to PNG bytes, rebuild the
    /// sprite's shape, and follow the new size — the selection and the
    /// marker referred to the old texture, so both clear.
    fn install(&mut self, ctx: &mut frost::Context, img: image::RgbaImage, status: String) {
        let png = match png_bytes(&img) {
            Ok(png) => png,
            Err(err) => {
                self.status = format!("could not re-encode the image: {err}");
                return;
            }
        };
        match frost::Shape::sprite_bytes(&png) {
            Ok(shape) => {
                let (iw, ih) = (img.width(), img.height());
                ctx.scene().root.children[0].shape = Some(shape);
                self.current = img;
                self.size = [iw as f32, ih as f32];
                self.selection = None;
                self.last_click = None;
                self.status = status;
            }
            Err(err) => self.status = format!("could not rebuild the sprite: {err}"),
        }
    }
}

/// The file dialog's pick (and the `-i` file's load): install the sprite,
/// its texture, its path and name into the demo. A file that is not a
/// loadable PNG ends the program, as it does everywhere else.
fn install_file(demo: &mut Demo, ctx: &mut frost::Context, path: std::path::PathBuf) {
    let name = file_name_of(&path);
    if path.extension().and_then(|e| e.to_str()) != Some("png") {
        log::error!("'{name}' is not a PNG file");
        std::process::exit(1);
    }
    let sprite = match frost::Shape::sprite(&path) {
        Ok(sprite) => sprite,
        Err(err) => {
            log::error!("failed to load '{name}': {err}");
            std::process::exit(1);
        }
    };
    let tex = match image::open(&path) {
        Ok(img) => img.to_rgba8(),
        Err(err) => {
            log::error!("failed to decode '{name}': {err}");
            std::process::exit(1);
        }
    };
    let size = sprite.sprite_size().expect("the shape is a sprite");
    let (sw, sh) = (size[0], size[1]);
    log::info!("loaded '{name}': {sw:.0}x{sh:.0} pixels");
    // Install the sprite in the scene and remember everything the panels
    // and the click math need; the first interactive frame is the next one.
    ctx.scene().root.children[0].shape = Some(sprite);
    demo.size = size;
    demo.name = name;
    demo.path = path;
    demo.current = tex;
    demo.history.clear();
    demo.selection = None;
    demo.last_click = None;
    demo.status = String::new();
    demo.pending = None;
}

/// The window point's position in the texture's pixel space: `(0, 0)`
/// upper-left, `x` right, `y` down — the sprite sits at the window's
/// center plus `offset` and is drawn at scale `zoom`.
fn tex_point(p: [f32; 2], size: [f32; 2], offset: [f32; 2], zoom: f32) -> [f32; 2] {
    let [tw, th] = size;
    [
        (p[0] - offset[0]) / zoom + tw / 2.0,
        th / 2.0 - (p[1] - offset[1]) / zoom,
    ]
}

/// The selection rectangle between two window points, in texture pixels
/// and clamped to the texture; a rectangle under a pixel wide is no
/// selection at all.
fn sel_rect_from(
    a: [f32; 2],
    b: [f32; 2],
    size: [f32; 2],
    offset: [f32; 2],
    zoom: f32,
) -> Option<[f32; 4]> {
    let [tw, th] = size;
    let ta = tex_point(a, size, offset, zoom);
    let tb = tex_point(b, size, offset, zoom);
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

    // The sprite's source: a `-i`/`--input` file is opened immediately,
    // without any file dialog; a `-i`/`--input` folder is where the file
    // dialog opens; and without the argument the dialog opens in the
    // working directory. The dialog folder is resolved to an absolute
    // path first — the native dialog (on Windows at least) ignores
    // relative paths and falls back to its default location, so the path
    // is joined onto the working directory (an absolute input replaces
    // it) and canonicalized.
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
    // exists; without the argument the scene's sprite node starts empty
    // and the demo's first frame opens the file dialog — as a child of
    // the window, so it lands on top of it — and installs the pick (see
    // `install_file`).
    let (sprite, size, name, path, texture) = match immediate {
        Some(file) => {
            let name = file
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            if file.extension().and_then(|e| e.to_str()) != Some("png") {
                log::error!("'{name}' is not a PNG file");
                std::process::exit(1);
            }
            let sprite = match frost::Shape::sprite(file) {
                Ok(sprite) => sprite,
                Err(err) => {
                    log::error!("failed to load '{name}': {err}");
                    std::process::exit(1);
                }
            };
            let texture = match image::open(file) {
                Ok(img) => img.to_rgba8(),
                Err(err) => {
                    log::error!("failed to decode '{name}': {err}");
                    std::process::exit(1);
                }
            };
            let size = sprite.sprite_size().expect("the shape is a sprite");
            let (sw, sh) = (size[0], size[1]);
            log::info!("loaded '{name}': {sw:.0}x{sh:.0} pixels");
            (Some(sprite), size, name, file.to_path_buf(), texture)
        }
        None => (
            None,
            [0.0, 0.0],
            String::new(),
            std::path::PathBuf::new(),
            image::RgbaImage::new(0, 0),
        ),
    };

    // The HUD font, the same monospaced variable font the diagnostics
    // overlay uses — shared by the panel's labels and the usage line.
    let font = format!("{root}/assets/fonts/FiraCode-VariableFont_wght.ttf");
    let ui = frost::Ui::from_font(&font).expect("failed to load the UI font");
    let help = Some(
        frost::Shape::text(
            &font,
            "wheel zoom   right pan   drag select   ctrl-z undo",
            20.0,
        )
        .expect("failed to load assets/fonts/FiraCode-VariableFont_wght.ttf")
        .with_weight(520.0),
    );

    let scene = frost::Scene::new(frost::SceneNode {
        // Dark background; the node's transform is ignored.
        shape: Some(frost::Shape::Background { color: BG }),
        children: vec![
            Box::new(frost::SceneNode {
                // The picked sprite, centered on the window's center; the
                // process applies the zoom to it each frame. Empty until
                // the first frame's picker installs a file (no
                // `-i`/`--input` argument).
                shape: sprite,
                ..Default::default()
            }),
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
                // The checker backdrop behind the sprite: the process
                // builds its tiny one-texel-per-cell sprite and positions
                // it each frame.
                shape: None,
                order: CHECK_ORDER,
                ..Default::default()
            }),
        ],
        ..Default::default()
    });

    if let Err(err) = frost::run(
        scene,
        Demo {
            ui,
            zoom: 1.0,
            offset: [0.0, 0.0],
            size,
            path,
            name,
            current: texture,
            history: Vec::new(),
            selection: None,
            drag_from: None,
            was_undo: false,
            light: GREY_LIGHT,
            dark: GREY_DARK,
            checker_key: (0, 0, 0, 0),
            was_down: false,
            last_mouse: None,
            last_click: None,
            pending: dialog_dir,
            status: String::new(),
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
        let offset = [7.0, -3.0];
        let zoom = 1.7;
        let [px, py] = tex_point([12.0, 5.0], size, offset, zoom);
        // The inverse of `tex_point`, back to window pixels.
        let wx = (px - size[0] / 2.0) * zoom + offset[0];
        let wy = (size[1] / 2.0 - py) * zoom + offset[1];
        assert!((wx - 12.0).abs() < 1e-3, "x came back at {wx}");
        assert!((wy - 5.0).abs() < 1e-3, "y came back at {wy}");
    }

    #[test]
    fn selection_is_texture_pixels_normalized_and_clamped() {
        let size = [100.0, 50.0];
        // Window center is the texture middle (50, 25); at zoom 2 a 40 px
        // drag covers 20 texture pixels.
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
}
