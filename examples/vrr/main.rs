//! A minimal VRR (variable refresh rate) demo.
//!
//! The keyboard sets the target frame rate; the frame loop blocks
//! (sleeps) between frames until the target interval has elapsed, so the
//! app presents at exactly that rate. With the diagnostics overlay
//! running, this makes it easy to
//! observe how the display's VRR (G-SYNC / FreeSync) behaves at
//! different frame rates — especially below the monitor's refresh rate,
//! where the panel's FPS and frame-time charts show the pacing, and the
//! tearing (or lack of it) is visible on the rotating body.
//!
//! Keys, not widgets, because the pointer is the one input this
//! instrument cannot trust. Pointer motion is compositor work, and the
//! window server clocks the *whole display* up to its maximum for it:
//! measured on an M4 MacBook driving an ASUS PG42UQ, dragging the old
//! slider pumped the panel to 120 Hz while the app itself went on
//! presenting at its target. A control you reach with the mouse is a
//! control that changes the thing it is set to measure. So: set a
//! target, take your hand off the mouse, and read the panel a second
//! later.
//!
//! The app starts with vsync on, because VRR only engages with a
//! non-tearing present. With vsync off the swap chain presents with
//! tearing (`PresentMode::Immediate`), which waits on nothing, so the
//! display never drops to match the frame rate. With vsync on (`Fifo`),
//! the present is refresh-synced, and when the pacing brings the frame
//! rate below the monitor's refresh rate, the display engages VRR and
//! drops its refresh rate to match. Press `v` — or Alt-V in the
//! diagnostics overlay — to compare: with vsync off the same paced rate
//! tears instead of engaging VRR.
//!
//! `f` swaps the window in and out of borderless fullscreen — F11 too,
//! but a Mac keyboard gives F11 to Mission Control unless `fn` is held,
//! so `f` is the key to reach for. Fullscreen is not a preference here:
//! measured on that same Mac and panel, the display follows the app's
//! cadence only in fullscreen, and only with vsync on. winit's
//! fullscreen is the native kind, a window in its own Space, and the
//! window server appears to float the refresh only for an app that owns
//! one. `c` hides the OS cursor, which tidies the frame and nothing
//! more: measured on the same panel, hiding the pointer does not keep the
//! compositor quiet, and a brisk move brings a large pointer straight
//! back regardless. A moving pointer is composited, and no app can
//! arrange otherwise — so the protocol is hands off the desk, not the
//! cursor hidden. Occasional spikes back to the maximum with nobody
//! touching anything are the rest of the machine finding something to
//! animate; they are the display's policy, not the app's cadence.
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

/// The bounds of the target the keys walk: down to a crawl, and up past
/// any real panel so the top of the range can be shown pinning the
/// display at its ceiling instead of tracking the app.
const MIN_TARGET: f32 = 1.0;
const MAX_TARGET: f32 = 240.0;

/// The frame rates the ladder keys step through — the paces an
/// instrument actually measures at. 24 and 25 are film and video, 30 is
/// a console pace, 48 and 50 sit at the bottom of a typical VRR window
/// (below it a driver duplicates frames instead of stretching the
/// refresh, which is worth seeing too), and 60 with its multiples is
/// where most panels live.
const LADDER: [f32; 12] = [
    24.0, 25.0, 30.0, 48.0, 50.0, 60.0, 72.0, 75.0, 90.0, 100.0, 120.0, 144.0,
];

/// How long a key must be held before it starts repeating, and how often
/// it fires once it does. The OS key repeat is tuned for text; walking a
/// frame rate wants a pace the eye can follow and a stop in the frame
/// after the finger lifts.
const REPEAT_AFTER: f32 = 0.35;
const REPEAT_EVERY: f32 = 0.05;

/// One key read as edges rather than as a level: `true` on the frame the
/// key goes down, then once every [`REPEAT_EVERY`] seconds after it has
/// been held for [`REPEAT_AFTER`] — so one tap is one step and a held
/// key walks its value at a readable pace.
#[derive(Default)]
struct Press {
    /// Whether the key was down last frame.
    held: bool,
    /// Seconds held since the press, counted only while held.
    held_for: f32,
}

impl Press {
    /// Advances the detector by the frame's `dt` and answers whether the
    /// key asks to fire this frame.
    fn tick(&mut self, down: bool, dt: f32) -> bool {
        if !down {
            *self = Self::default();
            return false;
        }
        if !self.held {
            self.held = true;
            return true;
        }
        self.held_for += dt;
        if self.held_for < REPEAT_AFTER {
            return false;
        }
        if self.held_for >= REPEAT_AFTER + REPEAT_EVERY {
            // Re-arm from this firing rather than from zero, so one slow
            // frame cannot buy the key an extra pause.
            self.held_for = REPEAT_AFTER;
            return true;
        }
        false
    }
}

/// The keys the instrument answers to, each with its own detector: a
/// held arrow and a tapped bracket are different intentions, and each
/// key needs its own memory of which it was last frame.
#[derive(Default)]
struct Keys {
    /// Up arrow: one frame per second faster.
    faster: Press,
    /// Down arrow: one frame per second slower.
    slower: Press,
    /// `]`: the next rung of [`LADDER`].
    higher: Press,
    /// `[`: the previous rung.
    lower: Press,
    /// `v`: vsync. Alt-V is the overlay's, not this one's.
    vsync: Press,
    /// `f`, and F11: fullscreen.
    fullscreen: Press,
    /// `c`: the OS cursor.
    cursor: Press,
}

/// The next rung of [`LADDER`] above `from`, or `from` at the top.
fn rung_above(from: f32) -> f32 {
    LADDER
        .iter()
        .copied()
        .find(|&rung| rung > from)
        .unwrap_or(from)
}

/// The next rung of [`LADDER`] below `from`, or `from` at the bottom.
fn rung_below(from: f32) -> f32 {
    LADDER
        .iter()
        .rev()
        .copied()
        .find(|&rung| rung < from)
        .unwrap_or(from)
}

/// The demo's state: the widget layer, the diagnostics overlay, and the
/// values the keys drive.
struct Demo {
    /// The immediate-mode widget layer; owns the UI font.
    ui: frost::Ui,
    /// The diagnostics overlay; owns its own copy of the font.
    diag: frost::Diagnostics,
    /// The target frame rate in frames per second, set by the keys.
    target_fps: f32,
    /// Whether vsync is on, toggled by `v` and mirrored from the engine
    /// (the overlay's Alt-V can flip it behind our back).
    vsync: bool,
    /// Whether the window is fullscreen, toggled by `f` and F11 and
    /// mirrored from the window (either can flip it behind the other's
    /// back, and so can the window manager).
    fullscreen: bool,
    /// The keys' edge and repeat detectors.
    keys: Keys,
    /// Whether the OS cursor is hidden — `c`, because a pointer you can
    /// see is a pointer you move, and moving it is the one thing that
    /// pulls the display off the app's cadence.
    cursor_hidden: bool,
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
            keys: Keys::default(),
            cursor_hidden: false,
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
        // Keys before the panel, so what the panel reads out this frame
        // is what the keys just did.
        //
        // Every control is a key, including the three that used to be
        // widgets: pointer motion is compositor work, and the window
        // server answers it by clocking the display to its maximum, so a
        // mouse-driven control here is a control that moves the
        // measurement. Held keys step on [`Press`]'s repeat, so a walk
        // from 120 down to 30 is one press and a breath.
        if self.keys.faster.tick(ctx.key_down(KeyCode::ArrowUp), dt) {
            self.target_fps += 1.0;
        }
        if self.keys.slower.tick(ctx.key_down(KeyCode::ArrowDown), dt) {
            self.target_fps -= 1.0;
        }
        self.target_fps = self.target_fps.clamp(MIN_TARGET, MAX_TARGET);
        if self.keys.higher.tick(ctx.char_down(']'), dt) {
            self.target_fps = rung_above(self.target_fps);
        }
        if self.keys.lower.tick(ctx.char_down('['), dt) {
            self.target_fps = rung_below(self.target_fps);
        }
        // Alt-V is the overlay's vsync and Alt-F its scaling combo, and
        // Alt often types the same plain character — leave the pairs
        // alone together or two toggles cancel inside one frame.
        let alt = ctx.key_down(KeyCode::AltLeft) || ctx.key_down(KeyCode::AltRight);
        if self.keys.vsync.tick(!alt && ctx.char_down('v'), dt) {
            self.vsync = !self.vsync;
            ctx.set_vsync(self.vsync);
        }
        // F11 is the runtime twin of `Config::fullscreen`; `f` is the
        // same switch freed from the Mac keyboard's function-row tax.
        let fullscreen_press = self.keys.fullscreen.tick(
            ctx.key_down(KeyCode::F11) || (!alt && ctx.char_down('f')),
            dt,
        );
        if fullscreen_press && let Some(window) = ctx.window() {
            window.set_fullscreen(if window.fullscreen().is_some() {
                None
            } else {
                Some(frost::Fullscreen::Borderless(None))
            });
        }
        if self.keys.cursor.tick(ctx.char_down('c'), dt)
            && let Some(window) = ctx.window()
        {
            self.cursor_hidden = !self.cursor_hidden;
            window.set_cursor_visible(!self.cursor_hidden);
        }
        // The overlay's Alt-V can flip the engine's vsync behind the
        // panel's back; mirror it back.
        if self.vsync != ctx.vsync() {
            self.vsync = ctx.vsync();
        }
        // The window is the source of truth: either key — or the window
        // manager — steers it, so mirror whichever one moved.
        if let Some(window) = ctx.window() {
            self.fullscreen = window.fullscreen().is_some();
        }

        let (w, h) = ctx.size();
        let cursor = if self.cursor_hidden {
            "hidden"
        } else {
            "shown"
        };
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
                        Col::stretch(1.0, Align::Left), // the values, after them
                    ],
                    |ui, ctx| {
                        ui.label(ctx, "Target");
                        ui.readout(ctx, &format!("{:.0} fps", self.target_fps));
                        ui.label(ctx, "VSync");
                        ui.readout(ctx, if self.vsync { "on" } else { "off" });
                        ui.label(ctx, "Fullscreen");
                        ui.readout(ctx, if self.fullscreen { "on" } else { "off" });
                        ui.label(ctx, "Cursor");
                        ui.readout(ctx, cursor);
                    },
                );
                ui.space(6.0);
                // The legend. Fira Code is monospaced, so the padded keys
                // line up on their own without a second table.
                ui.label(ctx, "↑/↓     one fps at a time");
                ui.label(ctx, "[ / ]   a standard rate");
                ui.label(ctx, "v       vsync");
                ui.label(ctx, "f       fullscreen");
                ui.label(ctx, "c       the cursor");
                ui.space(6.0);
                ui.label(ctx, "hands off the mouse to read");
            },
        );

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
