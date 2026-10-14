//! The window-behaviour probe, now with a menu: one engine window,
//! the resolution ladder of the `antifrost` hill, and a spoken
//! timeline of everything the desktop does to it.
//!
//! By default the window opens free and the keys drive it:
//!
//! * `R` — the resolution ladder: `1280 x 720` → `1920 x 1080` →
//!   `2560 x 1440` → `native`, cycling. A fixed rung resizes the
//!   window to exactly its pixels (1:1) and takes the resize handle
//!   away; `native` gives the handle back and letterboxes the page
//!   to 16:9 inside whatever shape the window ends up.
//! * `F` — fullscreen on/off.
//! * `Q` — quit.
//!
//! Every request the menu makes is the request `antifrost` makes —
//! the same order (free, request, pin), the same physical pixels —
//! and the log (`RUST_LOG=info`) narrates it, so a disagreement
//! between menu and desktop has a paper trail. The page is painted
//! black-on-white on purpose: a window-size bug is then obvious even
//! while the page itself is boring.
//!
//! The `--script` flag replaces the menu with a timed series of
//! size interventions, for diagnosing a desktop that won't answer:
//!
//! * t=2.0s — **slide the bounds** to 640x480 (`min = max = new`):
//!   KWin applies bounds to *future* resizes and never clamps a live
//!   window — this move is inert, and the probe's heartbeat proves
//!   that.
//! * t=4.0s — **free** and `request_inner_size(900x700)`: plain
//!   requests on a freed window are the move that works.
//! * t=6.0s — **slide again**, this time to a lopsided 500x900, to
//!   see whether anything out there feels entitled to the aspect.
//! * t=8.0s — free once more and request 1200x500.
//! * t=10.0s — report and exit.
//!
//! `--pinned-start` opens on the 1080p rung with `resizable: false`,
//! which makes the engine pin min/max before the window is ever
//! mapped — the state antifrost's fixed tiers are born in, and the
//! one whose release once went wrong.

use std::collections::HashMap;
use std::sync::Arc;

use frost::{
    Color, Config, Context, KeyCode, Process, Scene, SceneNode, Shape, Window, run_configured,
};

/// The hill's own voice, borrowed for the probe.
const GAME_FONT: &[u8] = include_bytes!("antifrost/assets/fonts/Barrfont-Regular.ttf");

const C_WHITE: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};
const C_TEXT: Color = Color {
    r: 0.10,
    g: 0.11,
    b: 0.14,
    a: 1.0,
};
const C_DIM: Color = Color {
    r: 0.42,
    g: 0.44,
    b: 0.50,
    a: 1.0,
};
const C_BG: Color = Color {
    r: 0.12,
    g: 0.14,
    b: 0.18,
    a: 1.0,
};

/// The ladder, same rungs and same meaning as the hill's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Rung {
    Pixels720,
    Pixels1080,
    Pixels1440,
    Native,
}

impl Rung {
    fn label(self) -> &'static str {
        match self {
            Self::Pixels720 => "1280 x 720",
            Self::Pixels1080 => "1920 x 1080",
            Self::Pixels1440 => "2560 x 1440",
            Self::Native => "native",
        }
    }
    fn next(self) -> Self {
        match self {
            Self::Pixels720 => Self::Pixels1080,
            Self::Pixels1080 => Self::Pixels1440,
            Self::Pixels1440 => Self::Native,
            Self::Native => Self::Pixels720,
        }
    }
    /// The buffer the rung renders into; `None` is the window's own
    /// pixels.
    fn size(self) -> Option<[u32; 2]> {
        match self {
            Self::Pixels720 => Some([1280, 720]),
            Self::Pixels1080 => Some([1920, 1080]),
            Self::Pixels1440 => Some([2560, 1440]),
            Self::Native => None,
        }
    }
}

/// The largest 16:9 box inside a window — native's view, and the
/// hill's, with the same reasoning.
fn fit_16x9(w: f32, h: f32) -> (f32, f32) {
    let vw = w.min(h * 16.0 / 9.0);
    (vw, vw * 9.0 / 16.0)
}

struct Probe {
    font: Arc<[u8]>,
    was: HashMap<KeyCode, bool>,
    rung: Rung,
    fullscreen: bool,
    /// A press that arrived while a move was in flight: applied
    /// when the in-flight move resolves.
    move_queued: bool,
    /// A fixed rung's size request with the frames waited: the
    /// veto notice below.
    expect: Option<(u32, u32, u32)>,
    /// Skip the client-side moves (request, hints) and ask through
    /// the caption alone (`--silent`): separates winit's local wish
    /// from the compositor's configure — the suspect in a window
    /// that moves while its surface stays behind.
    silent: bool,
    /// The timed series, kept behind `--script`.
    script: bool,
    /// Fast presses on a stopwatch, kept behind `--rapid`: the
    /// ladder every 0.7 s — quicker than a configure round trip —
    /// with the window's size logged at 10 Hz, so a flicker shows
    /// as a size that belongs to no rung.
    rapid: bool,
    /// Whether the opening ask (title, hints) has gone out yet.
    asked: bool,
    /// Which rapid press we are on.
    press: u32,
    /// Which tenth-second heartbeat we last spoke on.
    beat: u32,
    t: f32,
    done: bool,
}

impl Probe {
    fn pressed(&mut self, ctx: &Context, key: KeyCode) -> bool {
        let down = ctx.key_down(key);
        let edge = down && !*self.was.entry(key).or_insert(false);
        self.was.insert(key, down);
        edge
    }

    fn size(ctx: &Context) -> (u32, u32) {
        ctx.window().map_or((0, 0), |w: &Window| {
            let s = w.inner_size();
            (s.width, s.height)
        })
    }

    /// Exactly antifrost's policy: free (hints included), request
    /// the buffer's own physical pixels, pin min/max to them, drop
    /// the handle; native frees the hints and keeps the handle.
    /// Land on a rung: the buffer follows, then the window. The
    /// `R` key speaks only through here.
    fn set_rung(&mut self, ctx: &mut Context, rung: Rung) {
        self.rung = rung;
        ctx.set_render_size(rung.size());
        self.apply_window_policy(ctx);
    }

    fn apply_window_policy(&mut self, ctx: &mut Context) {
        let Some(window) = ctx.window() else {
            return;
        };
        log::info!(
            "window policy: ask {:?} (before {}x{}, fullscreen {})",
            self.rung.size(),
            window.inner_size().width,
            window.inner_size().height,
            self.fullscreen,
        );
        if self.silent {
            // The caption asked; nothing else has standing here.
            return;
        }
        // One move on screen at a time: a press during an
        // in-flight move re-aims it when that move resolves —
        // half-moves stacking up mid-flight is what flickered.
        if self.expect.is_some() {
            self.move_queued = true;
            log::info!("window policy: move in flight, re-aim queued");
            return;
        }
        // Only touch fullscreen when the ask differs from the
        // state (see antifrost: rewriting the state list kills
        // unknown states like KWin's tile atom, and the desktop
        // retiles in answer).
        if self.fullscreen {
            window.set_resizable(true);
            window.set_min_inner_size(None::<frost::PhysicalSize<u32>>);
            window.set_max_inner_size(None::<frost::PhysicalSize<u32>>);
            if window.fullscreen().is_none() {
                window.set_fullscreen(Some(frost::Fullscreen::Borderless(None)));
            }
            self.expect = None;
        } else {
            if window.fullscreen().is_some() {
                window.set_fullscreen(None);
            }
            match self.rung.size() {
                Some([w, h]) => {
                    // Unlock (hints and all), then move as one
                    // breath: request, pin, drop the handle. The
                    // pin is the engine, not insurance — winit's
                    // request never reaches the compositor on
                    // Wayland, so clamping the hints to the new
                    // bounds IS the move. (See antifrost's policy
                    // for the full story.)
                    window.set_resizable(true);
                    window.set_min_inner_size(None::<frost::PhysicalSize<u32>>);
                    window.set_max_inner_size(None::<frost::PhysicalSize<u32>>);
                    // The request is the mover: X11 directly,
                    // Wayland through the surface geometry winit
                    // sends with it — KWin honours a geometry
                    // change on a mapped window. A configure that
                    // then matches our wish is swallowed by
                    // winit's dedup, so never trust events alone
                    // to follow the window: the engine's frame
                    // loop checks its size against the surface's
                    // belief every frame (see app.rs).
                    let got = window.request_inner_size(frost::PhysicalSize::new(w, h));
                    // The pin is the move (see antifrost); the
                    // lock waits for arrival, because
                    // set_resizable(false) pins to the CURRENT
                    // size and would stomp the new one on X11.
                    window.set_min_inner_size(Some(frost::PhysicalSize::new(w, h)));
                    window.set_max_inner_size(Some(frost::PhysicalSize::new(w, h)));
                    log::info!(
                        "window policy: asked {w}x{h} ({got:?}, local {}x{}),",
                        window.inner_size().width,
                        window.inner_size().height,
                    );
                    self.expect = Some((w, h, 0));
                }
                None => {
                    window.set_resizable(true);
                    window.set_min_inner_size(None::<frost::PhysicalSize<u32>>);
                    window.set_max_inner_size(None::<frost::PhysicalSize<u32>>);
                    self.expect = None;
                }
            }
        }
    }

    // The timed series, run when `--script` asked for it.

    fn slide(&self, ctx: &mut Context, w: u32, h: u32) {
        let Some(window) = ctx.window() else { return };
        let px = frost::PhysicalSize::new(w, h);
        window.set_resizable(true);
        window.set_min_inner_size(Some(px));
        window.set_max_inner_size(Some(px));
        log::info!("script {:+.1}: bounds slid to {w}x{h}", self.t);
    }

    fn free(&self, ctx: &mut Context, w: u32, h: u32) {
        let Some(window) = ctx.window() else { return };
        window.set_resizable(true);
        window.set_min_inner_size(None::<frost::PhysicalSize<u32>>);
        window.set_max_inner_size(None::<frost::PhysicalSize<u32>>);
        let got = window.request_inner_size(frost::PhysicalSize::new(w, h));
        log::info!(
            "script {:+.1}: freed, requested {w}x{h}, answered {got:?}",
            self.t
        );
    }

    fn run_rapid(&mut self, ctx: &mut Context, dt: f32) {
        let t = self.t;
        let press = (t / 0.7) as u32;
        if press != self.press && press < 7 {
            self.press = press;
            self.rung = self.rung.next();
            ctx.set_render_size(self.rung.size());
            log::info!("rapid {t:.1}: rung -> {}", self.rung.label());
            self.apply_window_policy(ctx);
        }
        let beat = (t * 10.0) as u32;
        if beat != self.beat {
            self.beat = beat;
            let (w, h) = Self::size(ctx);
            log::info!("rapid {t:.1}: window is {w}x{h}");
        }
        if t > 6.5 {
            let (w, h) = Self::size(ctx);
            log::info!("rapid: done, window stands at {w}x{h}");
            self.done = true;
            ctx.exit();
        }
        let _ = dt;
    }

    fn run_script(&mut self, ctx: &mut Context) {
        let was = self.t;
        let t = self.t;
        if was < 2.0 && t >= 2.0 {
            self.slide(ctx, 640, 480);
        }
        if was < 4.0 && t >= 4.0 {
            self.free(ctx, 900, 700);
        }
        if was < 6.0 && t >= 6.0 {
            self.slide(ctx, 500, 900);
        }
        if was < 8.0 && t >= 8.0 {
            self.free(ctx, 1200, 500);
        }
        if was < 10.0 && t >= 10.0 {
            let (w, h) = Self::size(ctx);
            log::info!("script: done, window stands at {w}x{h}");
            self.done = true;
            ctx.exit();
        }
    }

    /// The page: white plate, black words, the rung and the window's
    /// own size, and the letterbox bars native lives in.
    fn paint(&mut self, ctx: &mut Context) {
        let (w, h) = ctx.size();
        let (vw, vh) = if self.rung.size().is_some() {
            (w, h)
        } else {
            fit_16x9(w, h)
        };
        let step = if vh < 520.0 { 42.0 } else { 62.0 };
        let top = vh / 2.0 - step;
        let f = Arc::clone(&self.font);
        let black = Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        ctx.rectangle(0.0, 0.0, vw / 2.0, vh / 2.0, C_WHITE, 0.0);
        ctx.text(0.0, top, &f, "MINWIN", 46.0, 700.0, C_TEXT, 1.0);
        ctx.text(
            0.0,
            top - 1.3 * step,
            &f,
            format!("[R] resolution   {}", self.rung.label()),
            26.0,
            400.0,
            C_TEXT,
            1.0,
        );
        ctx.text(
            0.0,
            top - 2.3 * step,
            &f,
            format!(
                "[F] fullscreen   {}",
                if self.fullscreen { "on" } else { "off" }
            ),
            26.0,
            400.0,
            C_TEXT,
            1.0,
        );
        let (ww, hh) = Self::size(ctx);
        ctx.text(
            0.0,
            top - 3.3 * step,
            &f,
            format!("window {ww} x {hh}"),
            26.0,
            400.0,
            C_DIM,
            1.0,
        );
        // The render's own numbers: what buffer the frame goes into
        // and how it fits the window. When geometry lands but the
        // picture does not, the disagreement hides on this line.
        let bf = ctx.render_size().map_or_else(
            || "buffer window".to_string(),
            |[bw, bh]| format!("buffer {bw}x{bh}"),
        );
        ctx.text(
            0.0,
            top - 4.0 * step,
            &f,
            format!("{bf}  view {:.0}x{:.0}", vw, vh),
            22.0,
            400.0,
            C_DIM,
            1.0,
        );
        ctx.text(
            0.0,
            top - 5.0 * step,
            &f,
            "[Q] quit",
            26.0,
            400.0,
            C_DIM,
            1.0,
        );
        // The bars: above the page, black, the hill's z contract.
        if w - vw > 0.5 {
            let d = (w - vw) / 4.0;
            ctx.rectangle(-(vw + w) / 4.0, 0.0, d, h / 2.0, black, 9.0);
            ctx.rectangle((vw + w) / 4.0, 0.0, d, h / 2.0, black, 9.0);
        }
        if h - vh > 0.5 {
            let d = (h - vh) / 4.0;
            ctx.rectangle(0.0, (vh + h) / 4.0, w / 2.0, d, black, 9.0);
            ctx.rectangle(0.0, -(vh + h) / 4.0, w / 2.0, d, black, 9.0);
        }
    }
}

impl Process for Probe {
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        if self.done {
            return;
        }
        self.t += dt;

        // The veto notice, same contract as the hill: a size the
        // desktop refused is said aloud, once.
        if let Some((w, h, waited)) = self.expect
            && let Some(window) = ctx.window()
        {
            {
                let size = window.inner_size();
                if size.width.abs_diff(w) < 2 && size.height.abs_diff(h) < 2 {
                    // Arrived: re-pin (harmless when honoured) and
                    // lock — a size that exists can be locked.
                    let px = frost::PhysicalSize::new(w, h);
                    window.set_min_inner_size(Some(px));
                    window.set_max_inner_size(Some(px));
                    window.set_resizable(false);
                    self.expect = None;
                } else if waited > 60 {
                    self.expect = None;
                    println!(
                        "minwin: asked the window for {w}x{h}, it stayed {}x{} — the window \
                         is likely tiled or snapped by the desktop (or pinned by a window \
                         rule); remove it from its tile to let the ladder resize it.",
                        size.width, size.height,
                    );
                } else {
                    self.expect = Some((w, h, waited + 1));
                }
            }
        }

        if self.move_queued && self.expect.is_none() {
            self.move_queued = false;
            self.apply_window_policy(ctx);
        }
        if self.script {
            self.run_script(ctx);
            return;
        }
        if !self.asked {
            self.asked = true;
            self.apply_window_policy(ctx);
        }
        if self.rapid {
            self.run_rapid(ctx, dt);
            return;
        }

        if self.pressed(ctx, KeyCode::KeyR) {
            let next = self.rung.next();
            self.set_rung(ctx, next);
        }
        if self.pressed(ctx, KeyCode::KeyF) {
            self.fullscreen = !self.fullscreen;
            self.apply_window_policy(ctx);
        }
        if self.pressed(ctx, KeyCode::KeyQ) {
            self.done = true;
            ctx.exit();
            return;
        }

        self.paint(ctx);
    }
}

#[derive(clap::Parser)]
struct Cli {
    /// Run the timed series instead of the interactive menu: bounds
    /// slides and free requests on a stopwatch.
    #[arg(long)]
    script: bool,
    /// Press the ladder every 0.7 s on a stopwatch — faster than a
    /// configure round trip — logging the size at 10 Hz. A flicker
    /// shows up as a size that belongs to no rung.
    #[arg(long)]
    rapid: bool,
    /// Open on the 1080p rung with `resizable: false` (the engine
    /// pins min/max at creation, before the window is mapped).
    /// The caption is the only move: no request, no hints, nothing
    /// winit keeps locally. For separating configure truth from
    /// client-side wishes.
    #[arg(long)]
    silent: bool,
    #[arg(long)]
    pinned_start: bool,
}

fn main() {
    env_logger::init();
    let cli: Cli = clap::Parser::parse();
    let scene = Scene::new(SceneNode {
        shape: Some(Shape::Background { color: C_BG }),
        ..Default::default()
    });
    // A fixed rung opens 1:1 and handle-less, as on the hill; the
    // free default opens small and resizable.
    let rung = if cli.pinned_start {
        Rung::Pixels1080
    } else {
        Rung::Native
    };
    if let Err(err) = run_configured(
        scene,
        Probe {
            font: Arc::from(GAME_FONT),
            was: HashMap::new(),
            rung,

            fullscreen: false,
            expect: None,
            move_queued: false,
            silent: cli.silent,
            script: cli.script,
            rapid: cli.rapid,
            asked: false,
            press: 0,
            beat: 0,
            t: 0.0,
            done: false,
        },
        Config {
            vsync: false,
            window_size: Some(rung.size().unwrap_or([1024, 768])),
            window_size_px: rung.size(),
            render_size: rung.size(),
            resizable: rung.size().is_none(),
            ..Default::default()
        },
    ) {
        eprintln!("minwin: {err}");
        std::process::exit(1);
    }
}
