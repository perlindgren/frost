//! The reusable widget layer driving a live scene: two draggable panels —
//! drag them by their title bars — hold a color mixer, a spin toggle and
//! speed, and an about box, all wired to a face that reacts every frame.
//!
//! - The `Tomato` panel's sliders drive the body's red, green and blue and
//!   its spin speed; they are declared as one `Ui::table` — a hugging
//!   label column, a stretch track column and a hugging readout column —
//!   so the tracks and values align across the rows. The `Spin` checkbox
//!   and `Reset` button stay plain rows below the table, and `Reset`
//!   restores the defaults.
//! - The `About` panel is three labels: it shows panels stacking (declare
//!   order is paint order) and holds their own dragged positions.
//!
//! Run with:
//!
//! ```text
//! cargo run --example widgets
//! ```

use frost::{Align, Col, Color, Context, Scene, SceneNode, Shape, Transform};

/// The radius of the face's body circle.
const BODY_R: f32 = 130.0;

/// The width of the controls panel in pixels.
const PANEL_W: f32 = 280.0;

/// The width of the about panel in pixels.
const ABOUT_W: f32 = 300.0;

/// The demo's state: the widget layer plus the values its widgets drive.
struct Demo {
    /// The immediate-mode widget layer; owns the UI font.
    ui: frost::Ui,
    /// The body color's channels, driven by the three sliders.
    r: f32,
    g: f32,
    b: f32,
    /// Whether the face spins.
    spin: bool,
    /// The spin's angular velocity in radians per second.
    speed: f32,
    /// The face's accumulated rotation.
    angle: f32,
}

impl Default for Demo {
    fn default() -> Self {
        // The UI font is loaded once, from disk here; `Ui::from_bytes`
        // takes `include_bytes!` data for a web build.
        let root = std::env!("CARGO_MANIFEST_DIR");
        let ui = frost::Ui::from_font(format!(
            "{root}/assets/fonts/FiraCode-VariableFont_wght.ttf"
        ))
        .expect("failed to load assets/fonts/FiraCode-VariableFont_wght.ttf");
        Self {
            ui,
            r: 0.85,
            g: 0.25,
            b: 0.25,
            spin: true,
            speed: 1.2,
            angle: 0.0,
        }
    }
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        // Start the UI frame: snapshot the mouse, resolve last frame's
        // press, drag the panel under the button.
        self.ui.begin(ctx);
        let (w, h) = ctx.size();
        // Anchors for the first frame, before the panels have been dragged:
        // top-left and top-right corners, inset by half their widths.
        self.ui.panel(
            ctx,
            "Tomato",
            [-w / 2.0 + PANEL_W / 2.0 + 16.0, h / 2.0 - 150.0],
            PANEL_W,
            |ui, ctx| {
                // The four sliders as one table: each row declares its
                // label, its track and its readout — one widget per
                // column, wrapping to the next row after the last.
                ui.table(
                    ctx,
                    "mix",
                    &[
                        Col::auto(Align::Left),         // the labels, hugging
                        Col::stretch(1.0, Align::Left), // the tracks, filling
                        Col::auto(Align::Right),        // the values, right
                    ],
                    |ui, ctx| {
                        ui.label(ctx, "Red");
                        ui.slider_track(ctx, "Red", &mut self.r, 0.0, 1.0);
                        ui.readout(ctx, &format!("{:.2}", self.r));
                        ui.label(ctx, "Green");
                        ui.slider_track(ctx, "Green", &mut self.g, 0.0, 1.0);
                        ui.readout(ctx, &format!("{:.2}", self.g));
                        ui.label(ctx, "Blue");
                        ui.slider_track(ctx, "Blue", &mut self.b, 0.0, 1.0);
                        ui.readout(ctx, &format!("{:.2}", self.b));
                        ui.label(ctx, "Speed");
                        ui.slider_track(ctx, "Speed", &mut self.speed, 0.0, 6.0);
                        ui.readout(ctx, &format!("{:.2}", self.speed));
                    },
                );
                ui.space(6.0);
                ui.checkbox(ctx, "Spin", &mut self.spin);
                ui.space(6.0);
                if ui.button(ctx, "Reset") {
                    self.r = 0.85;
                    self.g = 0.25;
                    self.b = 0.25;
                    self.spin = true;
                    self.speed = 1.2;
                }
            },
        );
        self.ui.panel(
            ctx,
            "About",
            [w / 2.0 - ABOUT_W / 2.0 - 16.0, h / 2.0 - 70.0],
            ABOUT_W,
            |ui, ctx| {
                ui.label(ctx, "frost widgets");
                ui.label(ctx, "drag the title bars,");
                ui.label(ctx, "drive the face");
            },
        );

        // The game state the widgets drive.
        if self.spin {
            self.angle += self.speed * dt;
        }

        // Push it into the scene: recolor the body circle in place, and
        // rotate its node — the eyes, being children, ride along.
        let body = ctx
            .scene()
            .root
            .children
            .get_mut(0)
            .expect("the scene's body node");
        if let Some(Shape::Circle { color, .. }) = &mut body.shape {
            color.r = self.r;
            color.g = self.g;
            color.b = self.b;
        }
        body.transform = Transform::rotate(self.angle);
    }
}

/// The face: a body circle with two eye circles as its children, so the
/// body node's rotation spins the whole face.
fn face() -> SceneNode {
    let eye = |x: f32| {
        Box::new(SceneNode {
            shape: Some(Shape::Circle {
                center: [x, 40.0],
                radius: 22.0,
                color: Color {
                    r: 0.98,
                    g: 0.98,
                    b: 1.0,
                    a: 1.0,
                },
            }),
            children: vec![Box::new(SceneNode {
                shape: Some(Shape::Circle {
                    center: [x, 40.0],
                    radius: 9.0,
                    color: Color {
                        r: 0.05,
                        g: 0.05,
                        b: 0.08,
                        a: 1.0,
                    },
                }),
                ..Default::default()
            })],
            ..Default::default()
        })
    };
    SceneNode {
        shape: Some(Shape::Circle {
            center: [0.0, 0.0],
            radius: BODY_R,
            color: Color {
                r: 0.85,
                g: 0.25,
                b: 0.25,
                a: 1.0,
            },
        }),
        children: vec![eye(-45.0), eye(45.0)],
        ..Default::default()
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    let scene = Scene {
        root: SceneNode {
            shape: Some(Shape::Background {
                color: Color {
                    r: 0.08,
                    g: 0.09,
                    b: 0.12,
                    a: 1.0,
                },
            }),
            children: vec![Box::new(face())],
            ..Default::default()
        },
        ..Default::default()
    };

    if let Err(err) = frost::run_configured(
        scene,
        Demo::default(),
        frost::Config {
            window_size: Some([1100, 700]),
            ..Default::default()
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
