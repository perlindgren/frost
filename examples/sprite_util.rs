//! A little sprite utility: pick any PNG out of `assets/sprites` with a
//! native file dialog (rfd), display it centered in the window, zoom it
//! with a slider, and log the clicked position in the sprite's own pixel
//! space through the `log` facade.
//!
//! The engine has no widget toolkit, so the slider is built from scene
//! primitives — a thin rectangle track with a circle handle, repositioned
//! every frame from the window size. Pointer state comes from
//! [`frost::Context::mouse_position`] and
//! [`frost::Context::mouse_button_down`]: pressing the track or handle and
//! dragging scrubs the zoom, and a click anywhere else is a sprite click —
//! its window position is converted to the texture's pixel space
//! (`(0, 0)` upper-left, `x` right, `y` down, divided by the current zoom)
//! and printed with `log::info!`, with a marker dot left at the spot.
//!
//! Run with:
//!
//! ```text
//! cargo run --example sprite_util
//! ```
//!
//! (Set `RUST_LOG=info` to see the click lines.)

/// The lowest zoom: the sprite at a quarter of its texture size.
const ZOOM_MIN: f32 = 0.25;

/// The highest zoom: the sprite four times its texture size.
const ZOOM_MAX: f32 = 4.0;

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
    /// The picked sprite's texture size, in pixels.
    size: [f32; 2],
    /// The picked file's name, for the HUD and the log lines.
    name: String,
    /// Whether the slider handle is being dragged.
    dragging: bool,
    /// Whether the left mouse button was held on the previous frame: the
    /// rising edge of the two is the click.
    was_down: bool,
    /// The last reported click: its position in the texture's pixel space,
    /// and whether the spot was inside the texture.
    last_click: Option<([f32; 2], bool)>,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, _dt: f32) {
        let (_w, h) = ctx.size();
        let sy = -h / 2.0 + SLIDER_MARGIN;

        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let clicked = down && !self.was_down;
        self.was_down = down;

        if let Some([mx, my]) = ctx.mouse_position() {
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
            // relative to the sprite. The sprite is centered on the
            // window's center at scale `zoom`, so window coordinates map
            // to texture pixels with `px = x / zoom + w/2` and
            // `py = h/2 - y / zoom` — the y flip included, since the
            // texture's y grows down.
            if clicked && !over && !self.dragging {
                let [tw, th] = self.size;
                let px = mx / self.zoom + tw / 2.0;
                let py = th / 2.0 - my / self.zoom;
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

        // The sprite's node carries the zoom: a uniform scale about its
        // center, which the sprite (centered on the node's origin) picks up
        // directly.
        let sprite = &mut ctx.scene().root.children[0];
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
                (px - tw / 2.0) * self.zoom,
                (th / 2.0 - py) * self.zoom,
            ]);
            if let Some(frost::Shape::Circle { radius, .. }) = &mut marker.shape {
                *radius = 4.0;
            }
        } else if let Some(frost::Shape::Circle { radius, .. }) = &mut marker.shape {
            *radius = 0.0;
        }
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
    let sprites = format!("{root}/assets/sprites");

    // Pick a PNG out of assets/sprites in a native file dialog. The dialog
    // opens in the sprites folder, filters to PNG files, and blocks until
    // the user picks a file or cancels — so it runs before the window
    // exists.
    let picked = rfd::FileDialog::new()
        .set_title("sprite_util: choose a sprite (assets/sprites)")
        .set_directory(&sprites)
        .add_filter("PNG images", &["png"])
        .pick_file();

    let Some(path) = picked else {
        log::warn!("no file selected — quitting");
        return;
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
                // process applies the zoom to it each frame.
                shape: Some(sprite),
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
                    "drag the slider to zoom   click the sprite to log its pixel",
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
            size,
            name,
            dragging: false,
            was_down: false,
            last_click: None,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
