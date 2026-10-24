//! The window's shape and the desktop's answer to it: the 16:9 fit,
//! the fixed tier's standing claim (`TierAsk`), the policy that
//! applies a settings change to the live window, the per-frame
//! watch that re-asks when the compositor has other plans, and
//! the letterbox that frames whatever size survives.

use crate::*;

/// All the window's bookkeeping, grouped off `Demo`: the first
/// policy to apply, the probe's clock, a queued move, the tier's
/// standing claim, and the fullscreen drift. The watch, the policy
/// and the menu all reach the same record through `Demo::window`.
pub(crate) struct WindowPolicy {
    /// The window takes its first shape on the first frame the
    /// engine's handle exists: the OS will not size a window that
    /// has not been born yet.
    pub(crate) pending: bool,
    /// Diagnostic clock for `--resize-probe`: cycles the real
    /// resolution row once on a timer, so the window's answer can
    /// be read from the log without a hand on the mouse.
    pub(crate) probe: Option<f32>,
    /// A press that arrived while a move was in flight: one move
    /// on screen at a time, and the newest rung always gets its
    /// turn — queued pressed, applied resolved.
    pub(crate) move_queued: bool,
    /// A fixed tier's standing claim on the window's size — the ask,
    /// and how the asking is going. See [`TierAsk`] for why it keeps
    /// asking after it has arrived.
    pub(crate) expect: Option<TierAsk>,
    /// Frames the window's fullscreen state has disagreed with the
    /// settings row. The desktop can move that state on its own — a
    /// green button, a window rule, a shortcut the WM answers itself —
    /// and the only news of it is the state, so the state is polled,
    /// the way the size is. The count is the grace, and it is a
    /// second's worth on purpose: it has to outlive both a compositor
    /// round trip and macOS's fullscreen animation, because acting
    /// sooner would mean the game yanking the window out of an
    /// entrance the desktop was still drawing.
    pub(crate) drift: u32,
}

/// The largest 16:9 box inside a window: native mode's view. A
/// fixed tier needs no fitting — its buffer *is* 16:9 and the engine
/// letterboxes the stretch to the window's aspect — but at native
/// the window is whatever the user dragged, so the game lays itself
/// out in this box and lets black bars frame what's left over.
pub(crate) fn fit_16x9(w: f32, h: f32) -> (f32, f32) {
    let vw = w.min(h * 16.0 / 9.0);
    (vw, vw * 9.0 / 16.0)
}

/// A fixed tier's claim on the window's size, and the record of how
/// that claim is being answered.
///
/// The claim outlives its own arrival, which is not the obvious design and
/// is the whole point of the type. Measured on macOS, coming out of a
/// fullscreen exit: the request landed — the engine logged the window at
/// 1920×1080 — and then the desktop resized it away twice more, to 1600×1144
/// and then to 1600×900, a second and a half later, during a stretch where
/// the surface was gone and the game was not even running its frames. A
/// policy that stopped caring the moment the size appeared left the window
/// exactly there: pinned by nothing, holding neither the tier's pixels nor
/// the aspect those pixels were chosen for.
///
/// So arrival starts a tail of quiet frames instead of ending the watch, and
/// any deviation during it re-asks. Both bounds are there because the other
/// answer is worse: a desktop that truly owns this window's geometry — a
/// tile, a snap, a window rule — gets argued with a bounded number of times
/// and then told about, rather than forever.
#[derive(Clone, Copy)]
pub(crate) struct TierAsk {
    /// The pixels the tier asked for, which fix the aspect too: one
    /// promise, not two.
    pub(crate) w: u32,
    pub(crate) h: u32,
    /// Frames since the window was last at those pixels.
    pub(crate) off: u32,
    /// Frames it has spent there. `CALM_HOLD` of them and the tier stands
    /// on its own.
    pub(crate) calm: u32,
    /// Asks made in all, counted so the give-up can say how many times it
    /// knocked.
    pub(crate) asks: u32,
}

impl TierAsk {
    /// Frames spent at the asked size before the move counts as over: half
    /// a second of a window sitting still. Until then a press queues
    /// behind it — one move on screen at a time — and after that the tail
    /// keeps watching in the background and a new press simply replaces the
    /// ask.
    const SETTLED: u32 = 30;
    /// How long the tail runs: five seconds of the window holding still,
    /// long past anything a compositor might still be doing about a window
    /// it just left fullscreen.
    const HOLD: u32 = 300;
    /// Frames off-size before the desktop is declared the owner of this
    /// window's geometry, and total asks before the same verdict on a
    /// window that keeps being moved back.
    const PATIENCE: u32 = 150;
    const PATIENCE_ASKS: u32 = 8;
    /// Frames between asks: six requests in six frames on a compositor
    /// that answers configures asynchronously is six fights in the log
    /// instead of one answer.
    const CADENCE: u32 = 15;

    /// Whether this ask still has a move in flight — off-size, or arrived
    /// too recently for the desktop to be done with the window.
    pub(crate) fn in_flight(&self) -> bool {
        self.off > 0 || self.calm < Self::SETTLED
    }
}

impl Demo {
    /// Black bars from the view's edge to the window's, above
    /// everything the game owns. Nothing draws outside the view, so
    /// the bars only ever cover the backdrop and the backdrop's
    /// darkness — they frame the picture and touch nothing else.
    pub(crate) fn draw_letterbox(&self, ctx: &mut Context) {
        let (w, h) = ctx.size();
        let (vw, vh) = self.view(ctx);
        let black = Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        let z = UI + 20.0;
        if w - vw > 0.5 {
            let d = (w - vw) / 4.0;
            ctx.rectangle(-(vw + w) / 4.0, 0.0, d, h / 2.0, black, z);
            ctx.rectangle((vw + w) / 4.0, 0.0, d, h / 2.0, black, z);
        }
        if h - vh > 0.5 {
            let d = (h - vh) / 4.0;
            ctx.rectangle(0.0, (vh + h) / 4.0, w / 2.0, d, black, z);
            ctx.rectangle(0.0, -(vh + h) / 4.0, w / 2.0, d, black, z);
        }
    }

    /// Bring the OS window to what the tier and the fullscreen flag
    /// ask for. A fixed windowed tier gets a window of exactly the
    /// buffer's pixels and no resize handle — picture and window are
    /// one rectangle, and the 16:9 question never comes up. Native
    /// takes the resize handle back: the user drags any shape, and
    /// [`fit_16x9`] keeps the game a 16:9 picture inside it.
    /// Fullscreen leaves the window to the display and the engine's
    /// letterboxed stretch to keep 16:9 on non-16:9 monitors.
    /// Whether the window is mid-move: a tier's ask that is either off-size or
    /// arrived so recently the desktop may still be dragging the window. One
    /// move on screen at a time, so a press during one queues behind it —
    /// behind the *move*, not behind the tail that keeps watching it.
    pub(crate) fn in_flight(&self) -> bool {
        self.window.expect.as_ref().is_some_and(TierAsk::in_flight)
    }

    pub(crate) fn apply_window_policy(&mut self, ctx: &mut Context) {
        let Some(window) = ctx.window() else {
            return;
        };
        log::info!(
            "window policy: ask {:?} (before {}x{}, fullscreen {})",
            self.settings.quality.size(),
            window.inner_size().width,
            window.inner_size().height,
            self.settings.fullscreen,
        );
        // The browser owns the window on web, so this whole policy is a
        // desktop concern — above all the size ask, which on web writes a CSS
        // width and height onto the canvas and puts a 1920×1080 hill inside a
        // 1366×682 page, its bottom and right third past the edge of the
        // screen. The engine sizes the canvas to the viewport and keeps it
        // there; the rung still decides the frame's quality on web, only not
        // by deciding how big the page is.
        if cfg!(target_arch = "wasm32") {
            return;
        }
        // One move on screen at a time: a press during an in-flight
        // move re-aims it when that move resolves — half-moves
        // stacking up mid-flight is what flickered.
        if self.in_flight() {
            self.window.move_queued = true;
            log::info!("window policy: move in flight, re-aim queued");
            return;
        }
        // Touch fullscreen only when the ask differs from the
        // state: winit rewrites the window's whole state list on
        // every call (X11 especially), and states it does not know
        // — like KWin's tile atom — die in the rewrite. KWin
        // notices its tile was yanked and retiles: the window
        // jumps to a size nobody asked for.
        if self.settings.fullscreen {
            window.set_resizable(true);
            window.set_min_inner_size(None::<frost::PhysicalSize<u32>>);
            window.set_max_inner_size(None::<frost::PhysicalSize<u32>>);
            if window.fullscreen().is_none() {
                window.set_fullscreen(Some(frost::Fullscreen::Borderless(None)));
            }
            self.window.expect = None;
        } else {
            if window.fullscreen().is_some() {
                window.set_fullscreen(None);
            }
            match self.settings.quality.size() {
                Some([w, h]) => {
                    // Unlock (hints and all), then move: pin min/max
                    // to the rung — on Wayland that clamp is the
                    // engine, since winit's request never leaves the
                    // app — and lock (set_resizable(false)) only on
                    // arrival, because locking pins to the CURRENT
                    // size and would stomp the new one wherever the
                    // request lands asynchronously.
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
                    window.set_min_inner_size(Some(frost::PhysicalSize::new(w, h)));
                    window.set_max_inner_size(Some(frost::PhysicalSize::new(w, h)));
                    self.window.expect = Some(TierAsk {
                        w,
                        h,
                        off: 0,
                        calm: 0,
                        asks: 1,
                    });
                    log::info!(
                        "window policy: asked {w}x{h} ({got:?}, local {}x{})",
                        window.inner_size().width,
                        window.inner_size().height,
                    );
                }
                None => {
                    window.set_resizable(true);
                    window.set_min_inner_size(None::<frost::PhysicalSize<u32>>);
                    window.set_max_inner_size(None::<frost::PhysicalSize<u32>>);
                    self.window.expect = None;
                }
            }
        }
    }

    /// One frame of the watch: the fullscreen state the row is
    /// written against, the tier's standing claim, and the resize
    /// probe — lifted from `process` whole, statement for
    /// statement, so the frame loop reads as an orchestration.
    pub(crate) fn watch_window(&mut self, ctx: &mut Context, dt: f32) {
        // A window the compositor owns — tiled or snapped, by KWin's
        // tiling editor or a window rule — re-asserts its own
        // geometry no matter what the game asks for, and says so
        // only through configure events. Watch those events: if the
        // window is not at the requested size, re-ask, and say the
        // quiet part aloud if it never comes. The letterbox keeps the
        // game 16:9 and playable in whatever size survives.
        //
        // First the state the row is written against. Leaving fullscreen is
        // the case that bites: an exit the desktop makes on its own leaves the
        // row saying `on` while the window sits windowed, at the size the
        // desktop saved before it went fullscreen — and no policy ever runs
        // again, because the game believes its own row, so the tier's pixels
        // and the pin that holds the aspect both stay off. Adopt what the
        // window says and let the policy answer to *that*: the size comes
        // back, the lock comes back, and the row tells the truth instead of
        // the one the player pressed an hour ago. Not saved to the file — a
        // gesture the desktop made on its own should not outlive the run.
        if let Some(window) = ctx.window() {
            let state = window.fullscreen().is_some();
            if state == self.settings.fullscreen || self.in_flight() {
                self.window.drift = 0;
            } else {
                self.window.drift += 1;
                if self.window.drift == 60 {
                    log::info!(
                        "window policy: the desktop has it {}fullscreen, the row said otherwise",
                        if state { "" } else { "out of " },
                    );
                    self.settings.fullscreen = state;
                    self.apply_window_policy(ctx);
                }
            }
        }
        if let Some(ask) = self.window.expect
            && let Some(window) = ctx.window()
        {
            {
                let TierAsk {
                    w,
                    h,
                    off,
                    calm,
                    asks,
                } = ask;
                let size = window.inner_size();
                let at_tier = size.width.abs_diff(w) < 2 && size.height.abs_diff(h) < 2;
                if at_tier && calm == 0 {
                    // Arrived, and only now take the handle: locking pins to
                    // the *current* size, so locking before arrival is how you
                    // pin a window to the wrong one. Idempotent where the
                    // compositor obeyed the hints, curative where a stomp left
                    // them off.
                    let px = frost::PhysicalSize::new(w, h);
                    window.set_min_inner_size(Some(px));
                    window.set_max_inner_size(Some(px));
                    window.set_resizable(false);
                    self.window.expect = Some(TierAsk {
                        w,
                        h,
                        off: 0,
                        calm: 1,
                        asks,
                    });
                } else if at_tier {
                    self.window.expect = if calm + 1 >= TierAsk::HOLD {
                        log::info!(
                            "window policy: {}x{} held for {} seconds, the tier stands",
                            w,
                            h,
                            TierAsk::HOLD / 60,
                        );
                        None
                    } else {
                        Some(TierAsk {
                            w,
                            h,
                            off,
                            calm: calm + 1,
                            asks,
                        })
                    };
                } else if off > TierAsk::PATIENCE || asks > TierAsk::PATIENCE_ASKS {
                    self.window.expect = None;
                    println!(
                        "antifrost: asked the window for {w}x{h} {} times, it stayed {}x{} \u{2014} \
                         the desktop is keeping its own geometry: a tile or a snap, a window \
                         rule, or a fullscreen exit that restored the shape the window had \
                         before it went in. Take it out of the tile, or set the row back to \
                         native and let the letterbox hold the 16:9.",
                        asks, size.width, size.height,
                    );
                } else {
                    // Not there yet, so ask again rather than only watch. One
                    // ask is a bet that the first request still stands, and
                    // two ordinary things take that bet away: an exit from
                    // fullscreen restores the window's old shape *after* the
                    // request made in the same breath as `set_fullscreen(None)`,
                    // and a min/max pin left at the tier's size while the
                    // window sits somewhere else is a hint the compositor has
                    // no reason to obey. So unlock, ask, and leave it unlocked
                    // until it arrives — which is the same order the policy
                    // uses, because locking pins to the current size and the
                    // current size is exactly what is wrong.
                    if off % TierAsk::CADENCE == TierAsk::CADENCE - 1 {
                        window.set_min_inner_size(None::<frost::PhysicalSize<u32>>);
                        window.set_max_inner_size(None::<frost::PhysicalSize<u32>>);
                        let again = window.request_inner_size(frost::PhysicalSize::new(w, h));
                        if off == TierAsk::CADENCE - 1 {
                            // Once, not on every re-ask. The line is there to
                            // say the first ask was answered by something other
                            // than the window arriving; the asks after it are
                            // that same fact repeated.
                            log::info!(
                                "window policy: still {}x{}, not the {w}x{h} asked \
                                 for — re-asking ({again:?})",
                                size.width,
                                size.height,
                            );
                        }
                    }
                    self.window.expect = Some(TierAsk {
                        w,
                        h,
                        off: off + 1,
                        calm: 0,
                        asks: if off % TierAsk::CADENCE == TierAsk::CADENCE - 1 {
                            asks + 1
                        } else {
                            asks
                        },
                    });
                }
            }
        }
        if self.window.move_queued && !self.in_flight() {
            self.window.move_queued = false;
            self.apply_window_policy(ctx);
        }
        if let Some(clock) = self.window.probe {
            let t = clock + dt;
            self.window.probe = Some(t);
            // Edge-triggered, every one: a range check on a 60 Hz
            // clock is a six-frame window, and six resize requests
            // on Wayland — where configures land asynchronously —
            // is six fights to watch in the log instead of one
            // answer. Each threshold crosses exactly once.
            if clock < 2.5 && t >= 2.5 {
                log::info!("resize probe: cycling the real quality row");
                self.settings.quality = self.settings.quality.next();
                ctx.set_render_size(self.settings.quality.size());
                self.apply_window_policy(ctx);
            }
            // High's `next` is native — which asks for no resize —
            // so the probe cycles twice: unlock at High's exit, and
            // the real 1080 -> 720 request at the next row.
            if clock < 4.5 && t >= 4.5 {
                log::info!("resize probe: cycling the real quality row again");
                self.settings.quality = self.settings.quality.next();
                ctx.set_render_size(self.settings.quality.size());
                self.apply_window_policy(ctx);
            }
            // Cycling the rung is not the whole question. A windowed tier's
            // size is asked for in the same breath as the fullscreen exit, and
            // a compositor that answers the exit second wins: the window comes
            // back at whatever shape it saved before it went in, which is the
            // one thing the row is supposed to rule out. So the probe goes in,
            // comes out, and says what survived — with the tier named, so the
            // log answers whether the window agrees without anyone counting
            // pixels on a screen.
            if clock < 7.0 && t >= 7.0 {
                log::info!("resize probe: into fullscreen");
                self.settings.fullscreen = true;
                self.apply_window_policy(ctx);
            }
            if clock < 9.0 && t >= 9.0 {
                log::info!("resize probe: out of fullscreen");
                self.settings.fullscreen = false;
                self.apply_window_policy(ctx);
            }
            if clock < 12.0 && t >= 12.0 {
                if let Some(window) = ctx.window() {
                    log::info!(
                        "resize probe: settled at {}x{}, tier asks {:?}",
                        window.inner_size().width,
                        window.inner_size().height,
                        self.settings.quality.size(),
                    );
                }
                self.window.probe = None;
            }
        }
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;

    #[test]
    fn the_native_view_keeps_the_window_when_it_is_already_wide_enough() {
        // Exact 16:9 passes through untouched: no bars to draw.
        assert_eq!(fit_16x9(1920.0, 1080.0), (1920.0, 1080.0));
        // A wide window loses height to bars; a tall one loses width.
        let (w, h) = fit_16x9(1600.0, 1000.0);
        assert_eq!((w, h), (1600.0, 900.0));
        let (w, h) = fit_16x9(1000.0, 1000.0);
        assert_eq!((w, h), (1000.0, 562.5));
        // The box is always 16:9 and always inside the window.
        for (ww, hh) in [(800.0, 600.0), (2560.0, 1080.0), (400.0, 900.0)] {
            let (vw, vh) = fit_16x9(ww, hh);
            assert!((vw / vh - 16.0 / 9.0).abs() < 1e-3, "{ww}x{hh}");
            assert!(vw <= ww + 1e-3 && vh <= hh + 1e-3, "{ww}x{hh}");
        }
    }
}
