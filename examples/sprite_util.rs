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
//! The engine has no widget toolkit, so the sliders are built from scene
//! primitives — a thin rectangle track with a circle handle, repositioned
//! every frame from the window size. Pointer state comes from
//! [`frost::Context::mouse_position`], [`frost::Context::mouse_button_down`]
//! and [`frost::Context::mouse_wheel`]: pressing a track or handle and
//! dragging scrubs that slider — the zoom, or one of the checker's grey
//! levels — the wheel zooms about the cursor (the point under it stays
//! put), a right-drag grabs the canvas and moves the sprite with the
//! cursor, and a left click anywhere else is a sprite click — its window
//! position is converted to the texture's pixel space (`(0, 0)` upper-left,
//! `x` right, `y` down, shifted by the pan and divided by the current
//! zoom) and printed with `log::info!`, with a marker dot left at the spot.
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

/// The slider track's half-width, in pixels.
const TRACK_HALF: f32 = 150.0;

/// The slider's distance from the window's bottom edge, in pixels.
const SLIDER_MARGIN: f32 = 48.0;

/// The slider handle's radius, in pixels.
const HANDLE_RADIUS: f32 = 7.0;

/// The vertical tolerance for grabbing a slider, in pixels.
const SLIDER_GRAB: f32 = 16.0;

/// The spacing between the sliders' rows, in pixels.
const SLIDER_ROW: f32 = 46.0;

/// The checker cell's edge, in window pixels: the pattern never scales
/// with the sprite — one cell is always 16 pixels on screen.
const CHECK_CELL: f32 = 16.0;

/// The checker's default light grey level, 0.0-1.0.
const GREY_LIGHT: f32 = 0.95;

/// The checker's default dark grey level, 0.0-1.0.
const GREY_DARK: f32 = 0.65;

const BG: frost::Color = frost::Color {
    r: 0.09,
    g: 0.09,
    b: 0.11,
    a: 1.0,
};
const TRACK: frost::Color = frost::Color {
    r: 0.35,
    g: 0.38,
    b: 0.45,
    a: 1.0,
};
const HANDLE: frost::Color = frost::Color {
    r: 1.0,
    g: 0.8,
    b: 0.3,
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

/// The bounding box's line width, in pixels.
const BBOX_WIDTH: f32 = 2.0;

/// The bounding box's order: above the HUD, below the click marker.
const BBOX_Z: f32 = 1.5;

/// The checker backdrop's order: behind the sprite and the HUD.
const CHECK_ORDER: f32 = -1.0;

/// The window's three sliders, in their rows' bottom-to-top order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slider {
    /// The zoom slider, the bottom row.
    Zoom,
    /// The checker's light grey level, the middle row.
    Light,
    /// The checker's dark grey level, the top row.
    Dark,
}

/// The demo's state.
struct Demo {
    /// The current zoom: the sprite's scale factor (1.0 is texture size).
    zoom: f32,
    /// The sprite's pan offset from the window's center, in window pixels;
    /// set by right-dragging and by the wheel's anchor.
    offset: [f32; 2],
    /// The picked sprite's texture size, in pixels.
    size: [f32; 2],
    /// The picked file's name, for the HUD and the log lines.
    name: String,
    /// The checker's light grey level, 0.0-1.0; the Light slider sets it.
    light: f32,
    /// The checker's dark grey level, 0.0-1.0; the Dark slider sets it.
    dark: f32,
    /// The checker texture's key: its cell count and the two grey levels,
    /// 0-255 quantized — the texture is rebuilt only when one of them
    /// changes.
    checker_key: (u32, u32, u8, u8),
    /// Which slider handle is being dragged, if one is.
    dragging: Option<Slider>,
    /// Whether the left mouse button was held on the previous frame: the
    /// rising edge of the two is the click.
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
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
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
            let size = sprite.sprite_size().expect("the shape is a sprite");
            let (sw, sh) = (size[0], size[1]);
            log::info!("loaded '{name}': {sw:.0}x{sh:.0} pixels");
            // Install the sprite in the scene and remember it for the HUD
            // and the click math; the rest of this frame's layout needs
            // the sprite's size, so the first interactive frame is the
            // next one.
            ctx.scene().root.children[0].shape = Some(sprite);
            self.size = size;
            self.name = name;
            self.pending = None;
            return;
        }

        let (w, h) = ctx.size();
        // The three slider rows, bottom to top: the zoom, then the
        // checker's two grey levels.
        let y_zoom = -h / 2.0 + SLIDER_MARGIN;
        let y_light = y_zoom + SLIDER_ROW;
        let y_dark = y_light + SLIDER_ROW;

        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let rdown = ctx.mouse_button_down(frost::MouseButton::Right);
        let clicked = down && !self.was_down;
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

        if let Some([mx, my]) = pos {
            // The slider row under the cursor, if any: each track has a
            // generous hit box, wide enough to grab the handle from either
            // side. The topmost matching row wins, so the rows never
            // fight.
            let over = [
                (Slider::Zoom, y_zoom),
                (Slider::Light, y_light),
                (Slider::Dark, y_dark),
            ]
            .iter()
            .find(|(_, y)| mx.abs() <= TRACK_HALF + HANDLE_RADIUS && (my - y).abs() <= SLIDER_GRAB)
            .map(|(s, _)| *s);

            if down {
                // A press on a slider — or a drag that wandered off it —
                // scrubs it: the pointer's x maps linearly onto the track.
                if over.is_some() || self.dragging.is_some() {
                    // `or` (not `unwrap_or`): the argument of `unwrap_or`
                    // is evaluated eagerly, so it would panic on a plain
                    // press while no drag is in progress.
                    let s = over
                        .or(self.dragging)
                        .expect("a press or drag is in progress");
                    self.dragging = Some(s);
                    let t = ((mx + TRACK_HALF) / (2.0 * TRACK_HALF)).clamp(0.0, 1.0);
                    match s {
                        Slider::Zoom => self.zoom = ZOOM_MIN + t * (ZOOM_MAX - ZOOM_MIN),
                        Slider::Light => self.light = t,
                        Slider::Dark => self.dark = t,
                    }
                }
            } else {
                self.dragging = None;
            }

            // A click that is not on a slider: report the position
            // relative to the sprite. The sprite sits at the window's
            // center plus `offset`, at scale `zoom`, so window coordinates
            // map to texture pixels with `px = (x - ox) / zoom + w/2` and
            // `py = h/2 - (y - oy) / zoom` — the y flip included, since the
            // texture's y grows down.
            if clicked && over.is_none() && self.dragging.is_none() {
                let [tw, th] = self.size;
                let px = (mx - self.offset[0]) / self.zoom + tw / 2.0;
                let py = th / 2.0 - (my - self.offset[1]) / self.zoom;
                let inside = px >= 0.0 && px <= tw && py >= 0.0 && py <= th;
                log::info!(
                    "click at sprite pixel ({px:.1}, {py:.1}) of {tw:.0}x{th:.0} '{name}' — {where}",
                    name = self.name,
                    where = if inside {
                        "inside the texture"
                    } else {
                        "outside the texture"
                    }
                );
                self.last_click = Some(([px, py], inside));
            }
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
        let checker = &mut ctx.scene().root.children[7];
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

        // Lay out the sliders and their labels from the current window
        // size, so resizing the window keeps everything pinned in place.
        let track = &mut ctx.scene().root.children[1];
        track.transform = frost::Transform::translate([0.0, y_zoom]);
        let t = (self.zoom - ZOOM_MIN) / (ZOOM_MAX - ZOOM_MIN);
        let handle = &mut ctx.scene().root.children[2];
        handle.transform =
            frost::Transform::translate([-TRACK_HALF + t * 2.0 * TRACK_HALF, y_zoom]);
        let zoom_label = &mut ctx.scene().root.children[3];
        zoom_label.transform = frost::Transform::translate([0.0, y_zoom + 22.0]);
        set_text(&mut zoom_label.shape, format!("zoom: {:.2}x", self.zoom));
        let light_track = &mut ctx.scene().root.children[8];
        light_track.transform = frost::Transform::translate([0.0, y_light]);
        let light_handle = &mut ctx.scene().root.children[9];
        light_handle.transform =
            frost::Transform::translate([-TRACK_HALF + self.light * 2.0 * TRACK_HALF, y_light]);
        let dark_track = &mut ctx.scene().root.children[10];
        dark_track.transform = frost::Transform::translate([0.0, y_dark]);
        let dark_handle = &mut ctx.scene().root.children[11];
        dark_handle.transform =
            frost::Transform::translate([-TRACK_HALF + self.dark * 2.0 * TRACK_HALF, y_dark]);
        let light_label = &mut ctx.scene().root.children[12];
        light_label.transform = frost::Transform::translate([0.0, y_light + 22.0]);
        set_text(
            &mut light_label.shape,
            format!("light grey: {:.2}", self.light),
        );
        let dark_label = &mut ctx.scene().root.children[13];
        dark_label.transform = frost::Transform::translate([0.0, y_dark + 22.0]);
        set_text(
            &mut dark_label.shape,
            format!("dark grey: {:.2}", self.dark),
        );
        let readout = &mut ctx.scene().root.children[4];
        readout.transform = frost::Transform::translate([0.0, y_zoom - 22.0]);
        set_text(
            &mut readout.shape,
            match &self.last_click {
                Some(([px, py], inside)) => format!(
                    "last click: pixel ({px:.1}, {py:.1}) ({})",
                    if *inside { "inside" } else { "outside" }
                ),
                None => String::from("last click: none"),
            },
        );
        let help = &mut ctx.scene().root.children[5];
        help.transform = frost::Transform::translate([0.0, h / 2.0 - 28.0]);

        // The marker dot rides the last click in window space — the inverse
        // of the conversion above — and shrinks away when there is none.
        let marker = &mut ctx.scene().root.children[6];
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

/// Replaces the string of a node's text shape in place, leaving the rest
/// of the node (its font, size, and transform) alone.
fn set_text(shape: &mut Option<frost::Shape>, text: String) {
    if let Some(frost::Shape::Text { text: t, .. }) = shape {
        *t = text;
    }
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
    // `Demo::process`).
    let (sprite, size, name) = match immediate {
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
            let size = sprite.sprite_size().expect("the shape is a sprite");
            let (sw, sh) = (size[0], size[1]);
            log::info!("loaded '{name}': {sw:.0}x{sh:.0} pixels");
            (Some(sprite), size, name)
        }
        None => (None, [0.0, 0.0], String::new()),
    };

    // The HUD's font, the same monospaced coding font the diagnostics
    // overlay uses.
    let font = format!("{root}/assets/fonts/FiraCode-VariableFont_wght.ttf");
    let text = |s: &str, size: f32| -> Option<frost::Shape> {
        Some(
            frost::Shape::text(&font, s, size)
                .expect("failed to load assets/fonts/FiraCode-VariableFont_wght.ttf"),
        )
    };

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
                // The slider track: a thin bar pinned near the bottom;
                // the process positions it each frame.
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, 0.0],
                    extent: [TRACK_HALF, 2.0],
                    color: TRACK,
                }),
                order: 1.0,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The slider handle.
                shape: Some(frost::Shape::Circle {
                    center: [0.0, 0.0],
                    radius: HANDLE_RADIUS,
                    color: HANDLE,
                }),
                order: 1.0,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The zoom readout, above the track.
                shape: text("zoom: 1.00x", 22.0),
                order: 1.0,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The last click's position in texture pixels.
                shape: text("last click: none", 20.0),
                order: 1.0,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The usage line, pinned near the top.
                shape: text(
                    "sliders or wheel to zoom   right-drag to pan   click to log the pixel",
                    20.0,
                ),
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
            Box::new(frost::SceneNode {
                // The light-grey slider's track.
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, 0.0],
                    extent: [TRACK_HALF, 2.0],
                    color: TRACK,
                }),
                order: 1.0,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The light-grey slider's handle.
                shape: Some(frost::Shape::Circle {
                    center: [0.0, 0.0],
                    radius: HANDLE_RADIUS,
                    color: HANDLE,
                }),
                order: 1.0,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The dark-grey slider's track.
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, 0.0],
                    extent: [TRACK_HALF, 2.0],
                    color: TRACK,
                }),
                order: 1.0,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The dark-grey slider's handle.
                shape: Some(frost::Shape::Circle {
                    center: [0.0, 0.0],
                    radius: HANDLE_RADIUS,
                    color: HANDLE,
                }),
                order: 1.0,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The light-grey readout, above its track.
                shape: text("light grey: 0.95", 20.0),
                order: 1.0,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The dark-grey readout, above its track.
                shape: text("dark grey: 0.65", 20.0),
                order: 1.0,
                ..Default::default()
            }),
        ],
        ..Default::default()
    });

    if let Err(err) = frost::run(
        scene,
        Demo {
            zoom: 1.0,
            offset: [0.0, 0.0],
            size,
            name,
            light: GREY_LIGHT,
            dark: GREY_DARK,
            checker_key: (0, 0, 0, 0),
            dragging: None,
            was_down: false,
            last_mouse: None,
            last_click: None,
            pending: dialog_dir,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
