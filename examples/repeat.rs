//! The repeat example: a layer that tiles itself, for infinite scrolling.
//!
//! The scene has a background, a repeating layer (order 0, speed 1.0 — it
//! follows the camera exactly), and a screen-fixed HUD layer (order 10,
//! speed 0.0, no repeat). The layer's `repeat` is set on the first frame,
//! when the window size is known: by default it is the window size in both
//! axes, so the window overlaps at most two tiles per axis and the layer
//! can be scrolled through endlessly. A "hero" circle straddling a tile
//! boundary is split into wrapping slices — the part past the boundary
//! appears on the opposite side of the window, while the grid's even
//! spacing makes the tile seam itself invisible.
//!
//! The camera auto-drifts up and to the right; `A`/`D` steer left/right and
//! `W`/`S` up/down. Escape closes the window.
//!
//! The repeat offsets can be overridden on the command line:
//!
//! ```text
//! cargo run --example repeat -- --repeat_x 500 --repeat_y 0
//! ```
//!
//! `0.0` disables the repetition along an axis, and a non-zero offset below
//! the window's width/height aborts the program with an error — the minimum
//! repeat offset is the window size.

use clap::Parser;

/// The auto-drift speed, in pixels per second.
const SPEED: f32 = 60.0;

/// The full width and height of each grid square, in pixels.
const SIZE: f32 = 40.0;

/// The hero circle's radius, in pixels — small enough to stay well under
/// half a window, so it never exceeds its repeat period.
const HERO: f32 = 90.0;

/// The index of the repeating layer.
const REPEAT_LAYER: usize = 0;

/// The index of the HUD layer.
const HUD_LAYER: usize = 1;

/// The command line arguments.
#[derive(Parser, Debug)]
#[command(name = "repeat")]
struct Args {
    /// The layer's x repeat offset in pixels. The window's width by
    /// default; `0.0` disables the repetition in x.
    #[arg(long = "repeat_x")]
    repeat_x: Option<f32>,
    /// The layer's y repeat offset in pixels. The window's height by
    /// default; `0.0` disables the repetition in y.
    #[arg(long = "repeat_y")]
    repeat_y: Option<f32>,
}

/// The demo's state.
struct Demo {
    /// The parsed command line arguments.
    args: Args,
    /// The path to the bundled font, for the HUD's text.
    font: String,
    /// The camera position in scene pixels; the pivot under the scene's
    /// root follows it.
    cam: [f32; 2],
    /// Set once the first frame has built the layer content and the HUD.
    built: bool,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        if !self.built {
            self.built = true;
            let (w, h) = ctx.size();
            let (rx, ry) = (
                self.args.repeat_x.unwrap_or(w),
                self.args.repeat_y.unwrap_or(h),
            );
            ctx.scene().layers[REPEAT_LAYER].repeat = [rx, ry];
            self.build_content(ctx, w, h, rx, ry);
            self.build_hud(ctx, w, h, rx, ry);
            log::info!("repeat: ({rx:.1}, {ry:.1}) for a {w:.0}x{h:.0} window");
        }
        // The auto drift, overridden per axis by WASD.
        let mut vel = [SPEED, SPEED * 0.6];
        if ctx.key_down(frost::KeyCode::KeyA) {
            vel[0] = -SPEED;
        }
        if ctx.key_down(frost::KeyCode::KeyS) {
            vel[1] = -SPEED * 0.6;
        }
        self.cam[0] += vel[0] * dt;
        self.cam[1] += vel[1] * dt;
        // The pivot under the scene's root carries the camera.
        ctx.scene().root.children[0]
            .transform = frost::Transform::translate(self.cam[0], self.cam[1]);
    }
}

impl Demo {
    /// Builds the repeating layer's content for the first frame: a grid of
    /// evenly spaced squares inside the content region, plus a "hero" circle
    /// straddling the right boundary of a tile, so its wrapping slice is
    /// visible. The grid's even spacing makes the tile seam invisible.
    fn build_content(
        &mut self,
        ctx: &mut frost::Context,
        w: f32,
        h: f32,
        rx: f32,
        ry: f32,
    ) {
        // The content region per axis: one tile when the axis repeats, a
        // fixed area covering twice the window (centered on the origin) when
        // it does not.
        let (x0, x1) = if rx > 0.0 { (0.0, rx) } else { (-w, w) };
        let (y0, y1) = if ry > 0.0 { (0.0, ry) } else { (-h, h) };
        let layer = &mut ctx.scene().layers[REPEAT_LAYER].root;
        let step = SIZE * 2.0;
        let mut x = x0 + step / 2.0;
        let mut column = 0;
        while x < x1 - SIZE / 2.0 {
            let mut y = y0 + step / 2.0;
            let mut row = 0;
            while y < y1 - SIZE / 2.0 {
                layer.children.push(Box::new(frost::SceneNode {
                    transform: frost::Transform::translate(x, y),
                    shape: Some(frost::Shape::Rectangle {
                        center: [0.0, 0.0],
                        extent: [SIZE / 2.0, SIZE / 2.0],
                        color: Self::color(column, row),
                    }),
                    ..Default::default()
                }));
                y += step;
                row += 1;
            }
            x += step;
            column += 1;
        }
        // The hero: a circle whose right part crosses the tile's right
        // boundary at x1, so the layer's wrap shows it on both sides of the
        // window at once (only when the x axis repeats).
        if rx > 0.0 {
            let cy = (y0 + y1) / 2.0;
            layer.children.push(Box::new(frost::SceneNode {
                transform: frost::Transform::translate(x1 - HERO / 2.0, cy),
                shape: Some(frost::Shape::Circle {
                    center: [0.0, 0.0],
                    radius: HERO,
                    color: frost::Color {
                        r: 1.0,
                        g: 0.85,
                        b: 0.3,
                        a: 1.0,
                    },
                }),
                ..Default::default()
            }));
        }
    }

    /// The screen-fixed HUD's text, for the first frame.
    fn build_hud(
        &mut self,
        ctx: &mut frost::Context,
        w: f32,
        h: f32,
        rx: f32,
        ry: f32,
    ) {
        let hud = &mut ctx.scene().layers[HUD_LAYER].root;
        hud.children.push(Box::new(frost::SceneNode {
            transform: frost::Transform::translate(0.0, h / 2.0 - 40.0),
            shape: Some(
                frost::Shape::text(
                    &self.font,
                    format!("repeat: ({rx:.0}, {ry:.0})   window: {w:.0}x{h:.0}"),
                    24.0,
                )
                .expect("failed to load assets/fonts/JameGem08_2026-Regular.ttf"),
            ),
            ..Default::default()
        }));
        hud.children.push(Box::new(frost::SceneNode {
            transform: frost::Transform::translate(0.0, h / 2.0 - 70.0),
            shape: Some(
                frost::Shape::text(
                    &self.font,
                    "A/D: left/right   W/S: up/down   Esc: quit",
                    24.0,
                )
                .expect("failed to load assets/fonts/JameGem08_2026-Regular.ttf"),
            ),
            ..Default::default()
        }));
    }

    /// A deterministic grid color for the square at `(column, row)`.
    fn color(column: usize, row: usize) -> frost::Color {
        let n = (column + row) % 5;
        match n {
            0 => frost::Color { r: 0.25, g: 0.45, b: 0.85, a: 1.0 },
            1 => frost::Color { r: 0.85, g: 0.35, b: 0.45, a: 1.0 },
            2 => frost::Color { r: 0.4, g: 0.75, b: 0.5, a: 1.0 },
            3 => frost::Color { r: 0.8, g: 0.7, b: 0.3, a: 1.0 },
            _ => frost::Color { r: 0.6, g: 0.45, b: 0.85, a: 1.0 },
        }
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");
    let args = Args::parse();
    // `CARGO_MANIFEST_DIR` pins the asset path to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let font = format!("{root}/assets/fonts/JameGem08_2026-Regular.ttf");

    if let Err(err) = frost::run(
        frost::Scene {
            // The background lives in the base group (the scene's root);
            // its transform is ignored anyway, so it always fills the
            // window. The root's child is the moving pivot; its shapeless
            // child is the camera.
            root: frost::SceneNode {
                shape: Some(frost::Shape::Background {
                    color: frost::Color {
                        r: 0.09,
                        g: 0.06,
                        b: 0.16,
                        a: 1.0,
                    },
                }),
                children: vec![Box::new(frost::SceneNode {
                    children: vec![Box::new(frost::SceneNode::default())],
                    ..Default::default()
                })],
                ..Default::default()
            },
            // The repeating layer follows the camera at the full speed; the
            // HUD is screen-fixed (speed 0.0) and never repeats.
            layers: vec![
                frost::Layer {
                    order: 0.0,
                    speed: 1.0,
                    repeat: [0.0, 0.0],
                    root: frost::SceneNode::default(),
                },
                frost::Layer {
                    order: 10.0,
                    speed: 0.0,
                    repeat: [0.0, 0.0],
                    root: frost::SceneNode::default(),
                },
            ],
            // The camera is the pivot's shapeless child: the window stays at
            // the camera, and the repeating layer scrolls through.
            camera: Some(frost::NodePath {
                group: None,
                children: vec![0, 0],
            }),
            ambient: frost::Scene::default().ambient,
        },
        Demo {
            args,
            font,
            cam: [0.0, 0.0],
            built: false,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
