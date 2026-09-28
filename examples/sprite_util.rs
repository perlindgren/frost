//! A little sprite utility: pick any PNG with a native file dialog (rfd),
//! display it in the window, zoom it with a slider or the mouse wheel,
//! pan it by grabbing the canvas with a right-drag, and log the clicked
//! position in the sprite's own pixel space through the `log` facade.
//!
//! The dialog is a child of the window, so it opens on top of it: the
//! window is created first and the dialog is parented to it on the first
//! frame (a dialog without a parent opens at the screen's default spot,
//! far from the window). It is modal, so the frame loop blocks until a
//! file is picked; canceling quits.
//!
//! The engine has no widget toolkit, so the slider is built from scene
//! primitives — a thin rectangle track with a circle handle, repositioned
//! every frame from the window size. Pointer state comes from
//! [`frost::Context::mouse_position`], [`frost::Context::mouse_button_down`]
//! and [`frost::Context::mouse_wheel`]: pressing the track or handle and
//! dragging scrubs the zoom, the wheel zooms about the cursor (the point
//! under it stays put), a right-drag grabs the canvas and moves the sprite
//! with the cursor, and a left click anywhere else is a sprite click — its
//! window position is converted to the texture's pixel space (`(0, 0)`
//! upper-left, `x` right, `y` down, shifted by the pan and divided by the
//! current zoom) and printed with `log::info!`, with a marker dot left at
//! the spot.
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

/// The vertical tolerance for grabbing the slider, in pixels.
const SLIDER_GRAB: f32 = 16.0;

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
    /// Whether the slider handle is being dragged.
    dragging: bool,
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

        let (_w, h) = ctx.size();
        let sy = -h / 2.0 + SLIDER_MARGIN;

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
            // The slider's hit region: a generous box around the track,
            // wide enough to grab the handle from either side.
            let over = mx.abs() <= TRACK_HALF + HANDLE_RADIUS && (my - sy).abs() <= SLIDER_GRAB;

            if down {
                // A press on the slider — or a drag that wandered off it —
                // scrubs the zoom: the pointer's x maps linearly onto the
                // track.
                if over || self.dragging {
                    self.dragging = true;
                    let t = ((mx + TRACK_HALF) / (2.0 * TRACK_HALF)).clamp(0.0, 1.0);
                    self.zoom = ZOOM_MIN + t * (ZOOM_MAX - ZOOM_MIN);
                }
            } else {
                self.dragging = false;
            }

            // A click that is not on the slider: report the position
            // relative to the sprite. The sprite sits at the window's
            // center plus `offset`, at scale `zoom`, so window coordinates
            // map to texture pixels with `px = (x - ox) / zoom + w/2` and
            // `py = h/2 - (y - oy) / zoom` — the y flip included, since the
            // texture's y grows down.
            if clicked && !over && !self.dragging {
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

        // Lay out the slider and its labels from the current window size,
        // so resizing the window keeps everything pinned in place.
        let track = &mut ctx.scene().root.children[1];
        track.transform = frost::Transform::translate([0.0, sy]);
        let t = (self.zoom - ZOOM_MIN) / (ZOOM_MAX - ZOOM_MIN);
        let handle = &mut ctx.scene().root.children[2];
        handle.transform = frost::Transform::translate([-TRACK_HALF + t * 2.0 * TRACK_HALF, sy]);
        let zoom_label = &mut ctx.scene().root.children[3];
        zoom_label.transform = frost::Transform::translate([0.0, sy - 26.0]);
        set_text(&mut zoom_label.shape, format!("zoom: {:.2}x", self.zoom));
        let readout = &mut ctx.scene().root.children[4];
        readout.transform = frost::Transform::translate([0.0, sy - 54.0]);
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
                    "slider or wheel to zoom   right-drag to pan   click to log the pixel",
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
            dragging: false,
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
