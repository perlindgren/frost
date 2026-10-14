//! A minimal VRR (variable refresh rate) demo.
//!
//! The keyboard sets the target frame rate; the frame loop waits on an
//! absolute deadline until the target interval has elapsed, so the app
//! presents at that rate rather than at the rate plus whatever the wait
//! cost. The panel reads out the target and the rate actually held
//! beside it, because the gap between the two is itself a measurement —
//! and because a pacing instrument that cannot see its own error is only
//! guessing. With the diagnostics overlay running, this makes it easy to
//! observe how the display's VRR (G-SYNC / FreeSync) behaves at
//! different frame rates — especially below the monitor's refresh rate,
//! where the panel's FPS and frame-time charts show the pacing, and the
//! tearing (or lack of it) is visible on the rotating body.
//!
//! `0` stands the pacer down altogether, which is the only way to tell a
//! limit this demo imposes from one the machine imposes: a free run that
//! still lands near the panel's own rate was never uncapped, whatever the
//! settings claim. It is a probe and not a way to play — it pins a core at
//! full tilt, tears above the ceiling the way any uncapped run does, and
//! leaves its own overlay a visible share of every frame. An arrow brings
//! the pace back, at whatever rate it lands the target on.
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
//! cargo run --example vrr -- --modes   # report the display's modes, then stop
//! ```

use std::time::{Duration, Instant};

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

/// How close to the pacing deadline the pacer stops sleeping and starts
/// watching the clock instead. `thread::sleep` wakes a millisecond or
/// more late — the timer is coalesced with everyone else's — and at 120
/// fps a millisecond is an eighth of the interval. That error is what
/// the panel would be following, and it is the difference between
/// measuring the display and measuring the sleep call. Spinning the last
/// millisecond costs a core's attention and buys the interval back.
const SPIN_TAIL: Duration = Duration::from_millis(1);

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
    /// `0`: stand the pacer down, or bring it back.
    free: Press,
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
    /// Whether the pacer is running. Stood down with `0`, the loop runs as
    /// fast as the machine and the presentation chain let it, which is the
    /// only way to tell a limit this demo set from a limit the chain
    /// imposes: a free run that still lands near the panel's rate was never
    /// uncapped, however the settings read.
    pacing: bool,
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
    /// Set by `--modes`: report the display's video modes on the first
    /// frame, then leave. A frame rather than before the window exists,
    /// because the mode list hangs off the window's monitor, and that is
    /// not a thing until the window is.
    report_modes: bool,
    /// The deadline the last frame paced to — a schedule, not a
    /// measurement, which is the whole point of the pacer.
    last_frame: Option<Instant>,
    /// The rate actually being held, smoothed. An instrument that sets a
    /// target owes it a reading of what it managed, so its own error sits
    /// next to the panel's readout of the same thing.
    actual_fps: f32,
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
            pacing: true,
            vsync: true,
            fullscreen: false,
            keys: Keys::default(),
            cursor_hidden: false,
            report_modes: false,
            last_frame: None,
            actual_fps: 0.0,
            angle: 0.0,
        }
    }
}

impl Demo {
    /// Block until the next deadline on an absolute schedule, so the
    /// interval between frames is the target interval rather than the
    /// target plus whatever the sleep cost. Wall clock throughout, not
    /// the engine's `dt`: sleeping inflates `dt`, and a schedule built
    /// from an inflated ruler inflates with it.
    fn pace(&mut self) {
        if !self.pacing {
            // Nothing to wait for. The anchor keeps moving with the clock
            // all the same, so bringing the pacer back picks up from this
            // frame instead of paying back every interval the free run
            // never asked for — a burst of catch-up frames the moment the
            // resume key is let go is an artifact, not a measurement.
            self.last_frame = Some(Instant::now());
            return;
        }
        let interval = Duration::from_secs_f64(1.0 / self.target_fps.max(0.001) as f64);
        let now = Instant::now();
        let Some(previous) = self.last_frame else {
            self.last_frame = Some(now);
            return;
        };
        let mut deadline = previous + interval;
        if deadline < now {
            // The frame's own work blew the schedule. Re-sync to now
            // rather than owe a catch-up frame: blowing one deadline
            // should cost one long frame, not buy a hurry-up one after
            // it — a burst is exactly the artifact this instrument is
            // here to look for, and inventing one would be dishonest.
            deadline = now;
        }
        if let Some(remaining) = deadline.checked_duration_since(now) {
            if remaining > SPIN_TAIL {
                std::thread::sleep(remaining - SPIN_TAIL);
            }
            while Instant::now() < deadline {
                std::hint::spin_loop();
            }
        }
        // The anchor is the deadline, not the moment of waking. Set from
        // the wake-up, every millisecond of sleep overshoot is written
        // into the next deadline as well, and the target is then quietly
        // never met: 8.3 ms asked for at a 120 fps request becomes 9.1 in
        // practice, and the panel faithfully tracks the error.
        self.last_frame = Some(deadline);
    }
}

/// Print every video mode of the monitor the window landed on, with its
/// refresh rate, and say which rate is live. This is the list that answers
/// "can this panel be driven at 50 Hz?" — a question worth more on macOS
/// than it looks, because the swap chain cannot ask for a variable refresh
/// there, and the way to an exact cadence at a low frame rate is instead to
/// match the display's own mode to it. Which modes exist at all is the
/// panel's business, so it is read out rather than assumed.
fn report_modes(ctx: &Context) {
    use std::cmp::Reverse;

    let Some(monitor) = ctx.window().and_then(|window| window.current_monitor()) else {
        println!("no monitor for the window yet");
        return;
    };
    let name = monitor.name().unwrap_or_else(|| "a display".to_owned());
    // Two decimals because the list distinguishes 60.00 from 59.94 and
    // 48.00 from 47.95 — the drop-frame rates, separate modes that round
    // to the same figure at one decimal and would then look like repeats.
    let live = monitor
        .refresh_rate_millihertz()
        .map(|millihertz| format!("{:.2} Hz live", millihertz as f32 / 1000.0))
        .unwrap_or_else(|| "live rate unknown".to_owned());
    println!("{name}: {live}");
    let mut modes: Vec<_> = monitor.video_modes().collect();
    let listed = modes.len();
    // Highest refresh first — on a ProMotion panel the list is long, and
    // the interesting end is the top.
    modes.sort_by_key(|mode| {
        let size = mode.size();
        (
            Reverse(mode.refresh_rate_millihertz()),
            Reverse(size.width),
            Reverse(size.height),
            Reverse(mode.bit_depth()),
        )
    });
    // A mode is its picture, not its pixel format: the same size and rate
    // arrive several times over per colour format, and the repeats are
    // noise in a list meant to be read.
    modes.dedup_by(|a, b| {
        a.size() == b.size() && a.refresh_rate_millihertz() == b.refresh_rate_millihertz()
    });
    for mode in &modes {
        let size = mode.size();
        println!(
            "  {:>5} x {:<5} {:>7.2} Hz  {}-bit",
            size.width,
            size.height,
            mode.refresh_rate_millihertz() as f32 / 1000.0,
            mode.bit_depth(),
        );
    }
    if listed != modes.len() {
        println!(
            "  ({} modes listed, {} distinct sizes and rates)",
            listed,
            modes.len()
        );
    }
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        if self.report_modes {
            self.report_modes = false;
            report_modes(ctx);
            ctx.exit();
            return;
        }
        // Block first, so the UI and the scene update at the target rate.
        self.pace();
        // And read back what that actually produced. `dt` is the interval
        // between successive frames of the engine's own loop, so it
        // carries the sleep, the frame's work, and the previous frame's
        // wait for its present — which is precisely the interval a
        // display with variable refresh is following.
        if dt > 0.0 {
            // A tenth of the way each frame: quick enough to settle in a
            // second at the rates that matter, slow enough that one
            // spiked frame does not drag the reading with it.
            self.actual_fps += (1.0 / dt - self.actual_fps) * 0.1;
        }

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
        // Alt-V is the overlay's vsync, Alt-F its scaling combo and Alt-0
        // its scale reset, and Alt often types the same plain character —
        // leave the pairs alone together or two toggles cancel inside one
        // frame.
        let alt = ctx.key_down(KeyCode::AltLeft) || ctx.key_down(KeyCode::AltRight);
        let faster = self.keys.faster.tick(ctx.key_down(KeyCode::ArrowUp), dt);
        let slower = self.keys.slower.tick(ctx.key_down(KeyCode::ArrowDown), dt);
        let higher = self.keys.higher.tick(ctx.char_down(']'), dt);
        let lower = self.keys.lower.tick(ctx.char_down('['), dt);
        // Any rate key doubles as the way back from a free run, and the
        // target it lands on is the one the pacer resumes at — so there is
        // no state a visitor can get stranded in.
        if faster || slower || higher || lower {
            self.pacing = true;
        }
        if faster {
            self.target_fps += 1.0;
        }
        if slower {
            self.target_fps -= 1.0;
        }
        self.target_fps = self.target_fps.clamp(MIN_TARGET, MAX_TARGET);
        if higher {
            self.target_fps = rung_above(self.target_fps);
        }
        if lower {
            self.target_fps = rung_below(self.target_fps);
        }
        if self.keys.free.tick(!alt && ctx.char_down('0'), dt) {
            self.pacing = !self.pacing;
        }
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
        let pacing = if self.pacing { "on" } else { "free" };
        // The rate the system believes this monitor is running at — a
        // second opinion beside the panel's own counter, and the two
        // disagree informatively: on a ProMotion display the mode moves for
        // content reasons with the app looking on, and a reading of it
        // turns that from faith into observation.
        let mode = ctx
            .window()
            .and_then(|window| window.current_monitor())
            .and_then(|monitor| monitor.refresh_rate_millihertz())
            .map(|millihertz| format!("{:.2} Hz", millihertz as f32 / 1000.0))
            .unwrap_or_else(|| "unknown".to_owned());
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
                        ui.label(ctx, "Pacing");
                        ui.readout(ctx, pacing);
                        ui.label(ctx, "Actual");
                        ui.readout(ctx, &format!("{:.1} fps", self.actual_fps));
                        ui.label(ctx, "Mode");
                        ui.readout(ctx, &mode);
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
                ui.label(ctx, "0       run free; arrows resume");
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

    // `--modes` is a question about the display rather than about the
    // demo, so it asks for as small a window as it can and leaves at once.
    let report_modes = std::env::args().any(|arg| arg == "--modes");
    let demo = Demo {
        report_modes,
        ..Default::default()
    };

    if let Err(err) = frost::run_configured(
        scene,
        demo,
        frost::Config {
            vsync: true,
            window_size: Some(if report_modes {
                [320, 200]
            } else {
                [1100, 700]
            }),
            ..Default::default()
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With the pacer stood down, `pace` must neither wait nor bank the
    /// intervals it skipped: an instrument that owes up its skipped frames
    /// fires a burst the moment the pace comes back, and the thing under
    /// test would then be the pacer's bookkeeping rather than the display.
    #[test]
    fn a_free_run_neither_waits_nor_owes() {
        let mut demo = Demo {
            target_fps: 10.0,
            pacing: false,
            ..Default::default()
        };
        let started = Instant::now();
        for _ in 0..50 {
            demo.pace();
        }
        let free = started.elapsed();
        // Fifty frames at a hundred milliseconds each would be five
        // seconds of waiting. A free run owes none of it.
        assert!(
            free < Duration::from_millis(200),
            "free run waited {free:?}"
        );

        // And the frame that brings the pace back waits one interval, not
        // the fifty it never spent.
        demo.pacing = true;
        let started = Instant::now();
        demo.pace();
        let resumed = started.elapsed();
        assert!(
            resumed < Duration::from_millis(200),
            "resuming paid back {resumed:?} of catch-up, not one interval"
        );
        assert!(
            resumed > Duration::from_millis(50),
            "the resumed frame did not pace at all: {resumed:?}"
        );
    }

    /// The ladder is the instrument's useful range, so its rungs must be
    /// climbable in both directions from anywhere in it, and must not run
    /// off either end.
    #[test]
    fn the_ladder_climbs_and_does_not_run_off() {
        assert_eq!(rung_below(rung_above(60.0)), 60.0);
        assert_eq!(rung_above(144.0), 144.0);
        assert_eq!(rung_below(24.0), 24.0);
        assert_eq!(rung_above(300.0), 300.0, "past the top, hold");
        assert_eq!(rung_below(1.0), 1.0, "below the bottom, hold");
    }
}
