//! A minimal VRR (variable refresh rate) demo.
//!
//! A slider sets the target frame rate; the frame loop blocks (sleeps)
//! between frames until the target interval has elapsed, so the app
//! presents at exactly that rate. With the diagnostics overlay running,
//! this makes it easy to observe how the display's VRR (G-SYNC /
//! FreeSync) behaves at different frame rates — especially below the
//! monitor's refresh rate, where the panel's FPS and frame-time charts
//! show the pacing, and the tearing (or lack of it) is visible on the
//! rotating body.
//!
//! The app starts with vsync on, because VRR only engages with a
//! non-tearing present. With vsync off the swap chain presents with
//! tearing (`PresentMode::Immediate`), which bypasses the refresh sync,
//! so the display never drops to match the frame rate. With vsync on
//! (`Fifo`), the present is refresh-synced, and when the slider's pacing
//! brings the frame rate below the monitor's refresh rate, the display
//! engages VRR and drops its refresh rate to match. Toggle the `VSync`
//! checkbox — or press Alt-V in the diagnostics overlay — to compare:
//! with vsync off the same paced rate tears instead of engaging VRR.
//!
//! The `Fullscreen` checkbox (or F11) swaps the window in and out of
//! borderless fullscreen, where the VRR behavior is easiest to see.
//!
//! Run with:
//!
//! ```text
//! cargo run --example vrr
//! ```

use std::time::Instant;

use frost::{Align, Col, Color, Context, KeyCode, Scene, SceneNode, Shape, Transform};

/// The width of the controls panel in pixels.
const PANEL_W: f32 = 280.0;

/// The demo's state: the widget layer, the diagnostics overlay, and the
/// values the widgets drive.
struct Demo {
    /// The immediate-mode widget layer; owns the UI font.
    ui: frost::Ui,
    /// The diagnostics overlay; owns its own copy of the font.
    diag: frost::Diagnostics,
    /// The target frame rate in frames per second, driven by the slider.
    target_fps: f32,
    /// Whether vsync is on, driven by the checkbox and mirrored from the
    /// engine (the overlay's Alt-V can flip it behind our back).
    vsync: bool,
    /// Whether the window is fullscreen, driven by the checkbox and F11
    /// and mirrored from the window (either can flip it behind the
    /// other's back).
    fullscreen: bool,
    /// Whether F11 was held down last frame, for edge detection.
    f11_down: bool,
    /// When the previous frame's pacing finished, for the next deadline.
    last_frame: Option<Instant>,
    /// The body's accumulated rotation.
    angle: f32,
}

impl Default for Demo {
    fn default() -> Self {
        // The font is loaded once, from disk here; `from_bytes` takes
        // `include_bytes!` data for a web build.
        let root = std::env!("CARGO_MANIFEST_DIR");
        let font = format!("{root}/assets/fonts/FiraCode-VariableFont_wght.ttf");
        let ui = frost::Ui::from_font(font.clone())
            .expect("failed to load assets/fonts/FiraCode-VariableFont_wght.ttf");
        let diag = frost::Diagnostics::new(font, frost::DiagnosticsFlags::all())
            .expect("failed to load the diagnostics font");
        Self {
            ui,
            diag,
            target_fps: 60.0,
            vsync: true,
            fullscreen: false,
            f11_down: false,
            last_frame: None,
            angle: 0.0,
        }
    }
}

impl Demo {
    /// Block until the target frame interval has elapsed since the
    /// previous frame's pacing finished, so the pace-to-pace interval —
    /// and thus the present interval — matches the target rate. The
    /// deadline is tracked on the wall clock, not on the engine's `dt`,
    /// because sleeping here inflates the next frame's `dt`.
    fn pace(&mut self) {
        let target = std::time::Duration::from_secs_f64(1.0 / self.target_fps.max(0.001) as f64);
        let now = Instant::now();
        if let Some(last) = self.last_frame {
            let elapsed = now.saturating_duration_since(last);
            if elapsed < target {
                std::thread::sleep(target - elapsed);
            }
        }
        self.last_frame = Some(Instant::now());
    }
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        // Block first, so the UI and the scene update at the target rate.
        self.pace();

        self.ui.begin(ctx);
        let (w, h) = ctx.size();
        let mut vsync_changed = false;
        let mut fullscreen_changed = false;
        // Top-right, clear of the diagnostics HUD pinned top-left.
        self.ui.panel(
            ctx,
            "VRR",
            [w / 2.0 - PANEL_W / 2.0 - 16.0, h / 2.0 - 150.0],
            PANEL_W,
            |ui, ctx| {
                ui.table(
                    ctx,
                    "vrr",
                    &[
                        Col::auto(Align::Left),         // the labels, hugging
                        Col::stretch(1.0, Align::Left), // the track, filling
                        Col::auto(Align::Right),        // the value, right
                    ],
                    |ui, ctx| {
                        ui.label(ctx, "Target");
                        ui.slider_track(ctx, "Target", &mut self.target_fps, 1.0, 240.0);
                        ui.readout(ctx, &format!("{:.0} fps", self.target_fps));
                        vsync_changed = ui.checkbox(ctx, "VSync", &mut self.vsync);
                        fullscreen_changed = ui.checkbox(ctx, "Fullscreen", &mut self.fullscreen);
                    },
                );
            },
        );
        if vsync_changed {
            ctx.set_vsync(self.vsync);
        }
        // The overlay's Alt-V can flip the engine's vsync behind the
        // checkbox's back; mirror it back.
        if self.vsync != ctx.vsync() {
            self.vsync = ctx.vsync();
        }
        if fullscreen_changed && let Some(window) = ctx.window() {
            window.set_fullscreen(if self.fullscreen {
                Some(frost::Fullscreen::Borderless(None))
            } else {
                None
            });
        }
        // F11 flips the window's fullscreen state — the runtime twin of
        // `Config::fullscreen`, reached through the engine's own window.
        let f11 = ctx.key_down(KeyCode::F11);
        if f11
            && !self.f11_down
            && let Some(window) = ctx.window()
        {
            window.set_fullscreen(if window.fullscreen().is_some() {
                None
            } else {
                Some(frost::Fullscreen::Borderless(None))
            });
        }
        self.f11_down = f11;
        // The window is the source of truth: the checkbox and F11 steer
        // the same state, so mirror whichever one moved.
        if let Some(window) = ctx.window() {
            self.fullscreen = window.fullscreen().is_some();
        }

        self.diag.process(ctx, dt);

        // The body spins at one radian per second of frame time, so it
        // visibly slows as the target rate drops.
        self.angle += dt;
        if let Some(body) = ctx.scene().root.children.get_mut(0) {
            body.transform = Transform::rotate(self.angle);
        }
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
            children: vec![Box::new(SceneNode {
                shape: Some(Shape::Rectangle {
                    center: [0.0, 0.0],
                    extent: [60.0, 60.0],
                    color: Color {
                        r: 0.9,
                        g: 0.55,
                        b: 0.2,
                        a: 1.0,
                    },
                }),
                ..Default::default()
            })],
            ..Default::default()
        },
        ..Default::default()
    };

    if let Err(err) = frost::run_configured(
        scene,
        Demo::default(),
        frost::Config {
            vsync: true,
            window_size: Some([1100, 700]),
            ..Default::default()
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
