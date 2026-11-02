# Viability: frost + winit inside a nice-plug (CLAP/VST3) plugin

**Date checked:** against `nice-plug 0.4.2` (crates.io / tag `v0.4.2`), `frost@main`
(commit from 2026-10), `baseview 0.3.x`, `wgpu 30.0.1`, `winit 0.30.13`, macOS host,
rustc 1.99.

## Verdict

| Question | Answer |
|---|---|
| Do frost + nice-plug coexist in one dependency tree? | **Yes, verified by compiling** — single `wgpu 30.0.1`, single `winit 0.30.13`, single `raw-window-handle 0.6.2` |
| Can frost be used **as-is** (`frost::run`) inside a plugin? | **No.** It owns the process: own window, own winit event loop, blocks, and hijacks `NSApplicationDelegate` on macOS |
| Can **winit** windows embed into a DAW host window? | **Not on macOS** (no NSView embedding; `with_parent_window` is Windows/X11-only). Even where it works it is the wrong tool |
| Can frost's **renderer** (wgpu/scene graph) run inside nice-plug's GUI host? | **Yes, via a fork + baseview adapter** — nice-plug's own `byo_gui_wgpu` example proves the wgpu-in-host path; frost uses the same wgpu major version |
| Licensing | **Resolved 2026-11-02.** frost carries `MIT OR Apache-2.0` in `[workspace.package]`, both texts at the root, and this plugin inherits it. What this row read when the study was written — *"⚠️ frost has no license — all rights reserved"* — is true of the engine as it stood then, and §5 records how it cleared |

**Bottom line:** viable as a *rendering engine via a fork*, not viable as a drop-in GUI
dependency. The winit half of frost gets discarded at the seam; baseview (nice-plug's
host-integration layer) replaces it.

---

## Evidence

### 1. Dependency coexistence — verified by compile

Probe crate (`nice-plug 0.4.2` default-off + `editor`, `baseview 0.3.x`, `wgpu 30`,
`frost` git) **compiles clean**, including the exact seam nice-plug uses:

```rust
// wgpu 30 accepts baseview's WindowContext handle as a surface target —
// exactly what nice-plug's byo_gui_wgpu example does:
fn surface_target(window: &baseview::WindowContext) -> wgpu::SurfaceTarget<'static> {
    wgpu::SurfaceTarget::Window(Box::new(window.platform_handle()))
}
```

Resolved tree (duplicates only the usual bitflags/objc2 0.5-vs-0.6 noise):

```
baseview 0.3.7 · frost 0.1.0 (git) · wgpu 30.0.1 · winit 0.30.13 · raw-window-handle 0.6.2
```

frost pulls the only winit in the tree; baseview does **not** use winit (it uses
`objc2`/win32/x11 directly).

### 2. frost's public API cannot host-embed

- `frost::run()` / `run_configured()` (src/lib.rs ~913–1009): creates wgpu
  instance/adapter/device, then `EventLoop::new()` and **blocks in
  `event_loop.run_app(&mut app)`** until the window closes. No parent-window
  parameter, no external event pump, no external surface.
- The entire windowing/render-loop backend is **private**: `src/backend/mod.rs` says
  “Everything here is internal to the crate”; `Frost` is `pub(crate)`. Public surface
  is `Canvas`, `Context`, `Process`, `run`. Embedding = fork.
- Config knobs that do help once forked: `Config::vsync` (must be `false` — nice-plug's
  byo example warns “Plugins should not use vsync as it blocks the host's main thread”),
  `Config::resizable`, `window_size_px`.

### 3. winit-inside-a-plugin analysis (winit 0.30.13)

- **macOS:** `EventLoop::new` panics unless on the main thread
  (`platform_impl/macos/event_loop.rs:221`) — and the host already owns that thread's
  run loop. Worse, it calls `NSApplication::setDelegate(...)` with winit's own delegate
  (event_loop.rs ~227–240), **replacing the DAW's app delegate**. `pump_events` does
  exist publicly on macOS in 0.30 (`platform/mod.rs` cfg includes `macos_platform`), so
  an externally-driven loop is mechanically possible — but the delegate swap and app
  setup make this hostile in a plugin.
- **Embedding:** `WindowAttributes::with_parent_window` (window.rs ~475) is documented
  for **Windows (WS_CHILD) and X11 reparent only**; the macOS impl ignores it; Wayland
  unsupported. winit creates NSWindows, never embeddable NSViews. Proper in-host
  embedding is exactly why nice-plug uses **baseview** instead.
- nice-plug `main` even carries a workaround note (byo example Cargo.toml) warning about
  baseview coexisting with **winit in the same binary** (RustAudio/baseview#321,
  standalone mode) — another reason the fork should drop winit from the tree entirely.

### 4. What baseview gives us (the target the fork must fit)

- macOS: plugin content is an `NSView` added as a **subview** of the host view; keyboard/
  mouse/tracking arrive as native responder callbacks and are delivered as
  `baseview::Event` to `WindowHandler::on_event` on the host main thread. Frames are
  paced by an internal repeating **~15 ms timer** that calls `WindowHandler::on_frame`
  (`platform/macos/view.rs:142`). Continuous 60 fps-ish polling; fine for a tuner.
- Handler trait on crates.io 0.3.x (the one nice-plug **0.4.2** targets):
  `on_frame()`, `resized(WindowSize)`, `on_event(Event)` — no `poll/draw/on_timer` and
  no `request_redraw` on `WindowContext` (those exist only in the unreleased baseview
  that nice-plug `main` examples use). **Pin `baseview = "=0.3.3"`** (what v0.4.2
  examples were built against; 0.3.7 already shifted the API surface).
- Resize: host calls `EditorHandle::set_size` → `window.resize()` → `resized()` callback
  → reconfigure the wgpu surface. Standard for every nice-plug GUI.

### 5. Licensing — the real blocker, and how it cleared

**What the study measured (October).** frost had **no LICENSE file and no `license`
field**; README: “Copyright to the Developers (Per Lindgren/Iris Hulsman) … hopefully we
can find a license allowing the sources to be open.” Default copyright ⇒ **not legally
safe to build or ship as a product dependency** without asking. Hence the advice: settle
this before investing in the fork, by asking the authors (they are responsive) for
MIT/Apache-2.0 or explicit permission.

**What happened (checked 2026-11-02).** The authors licensed the engine.
`[workspace.package] license = "MIT OR Apache-2.0"` in the root manifest, `LICENSE-APACHE`
and `LICENSE-MIT` beside it, and `docs/licensing.md` as the table of who carries what.
The fork this document recommends was never made: the engine grew `frost::embedded` and
this plugin depends on the engine at `path = ".."`, so the two sides of the seam state one
licence rather than needing to be reconciled — and `nice-tuner` says so in one line,
`license.workspace = true`.

Verified with the command `docs/licensing.md` names as the check, because it prints what
cargo resolved rather than what a manifest appears to say — `cargo metadata --no-deps`
answers `MIT OR Apache-2.0` for `frost`, `frost_shaders`, `sprite_util`, `noisefloor`,
`nice-tuner`, `pitch` and `xtask`, and nothing for `antifrost` and `immortal`, which is
that document's idiom for all rights reserved and the intended remainder.

**What the licence asks in return** is a condition inside the grant rather than a question
about it: the UI embeds the OFL-licensed Fira Code, and OFL permits that on the condition
the licence text travels with the font. `cargo xtask bundle` now writes it into the VST3
bundle at `Contents/Resources/OFL.txt` and lays a copy beside the bundles for the CLAP, and
`scripts/check.sh` fails on a bundle whose copy is missing or is not the tree's own byte for
byte. One step stays outside the build: a CLAP is one file by the format's design, so its
copy reaches a user only through whoever packs the installer. See `README.md` §Licensing.

---

## Recommended architecture if proceeding with frost

Fork frost, surgically split renderer from windowing:

1. **Keep**: `Scene`/`Node`/`Shape`/`Canvas` scene graph, `shaders.rs`, `frame.rs`
   draw-list pipeline, `text.rs` (swash), `bake/collision` if needed.
2. **Delete in fork**: `backend/app.rs`, `backend/windowing.rs`, `run()`, and the
   `rodio`/`Audio` module (a plugin must not open its own audio device — the host owns
   audio). winit dep drops out of the tree entirely; replace re-exported
   `KeyCode`/`MouseButton` with baseview equivalents.
3. **Add `FrostEmbedded` adapter** implementing `baseview::WindowHandler`:
   - create: wgpu surface via `SurfaceTarget::Window(Box::new(ctx.platform_handle()))`,
     `PresentMode::AutoNoVsync`, surface config from `ctx.size()`;
   - `on_frame`: build draw list from the `Scene`, run `Process`-style tick with the
     latest params/pitch state, render + present;
   - `on_event`: translate `baseview::Event` → frost's input state (port the input
     handling out of `windowing.rs`);
   - `resized`: reconfigure surface.
4. **nice-plug side**: copy the `byo_gui_wgpu` example (v0.4.2 tag) as the skeleton —
   `Editor::spawn` + `EditorHandle` + host-callback adapters are ~all boilerplate.
5. **Audio path for the tuner**: nice-plug's `process()` does the DSP (pitch detect,
   e.g. YIN/FFT); publish detected pitch via an `Arc<AtomicCell<...>>` like the examples'
   `WgpuEditorState`; `on_frame` reads it and redraws. Gate detection on
   `editor_state.is_open()`.

Effort: the seam is concentrated in `backend/app.rs` + `backend/windowing.rs`
(the scene/render pipeline itself stays untouched). Realistic: a few days of
implementation + ongoing fork maintenance.

### Alternatives considered

- **Run frost unmodified in a floating winit window** ("GUI sidecar window"): blocked on
  macOS (main-thread event loop + delegate hijack; spawn() must return to the host);
  fragile on Windows/X11; UX wrong (window never follows the host). Rejected.
- **Skip frost, use nice-plug's official GUI paths** (`gain_egui`, `gain_iced`,
  raw `byo_gui_wgpu`): zero fork risk, but loses frost's scene-graph aesthetic. If the
  goal is "a good-looking tuner" rather than "frost specifically", `byo_gui_wgpu` +
  frost-inspired 2D drawing, or egui/iced, is far cheaper.

## Open risks for the frost route

- Fork maintenance while frost upstream keeps evolving (active repo, LLM-driven churn).
- baseview is in flux (API moved again post-0.3.7; nice-plug main already tracks an
  unreleased version) — pin strictly now, plan a bump when nice-plug releases next.
- DPI/scale-factor handling must use `WindowSize.scale_factor` (host-provided), not the
  monitor scale frost reads today.
- ~~Legal clearance (above).~~ Cleared: see §5. What survived is narrower — the OFL text
  must travel with the embedded font — and the bundle step now writes it in, with the
  plugin's own gate refusing a bundle that lost it.

---

## Postscript (2026-11-02): the window is X11, and that is not an oversight

Checked against `baseview 0.3.3` (what we pin) and `RustAudio/baseview@master`,
`nice-plug 0.4.2`, `nice-plug-core 0.4.2`, `clap-sys 0.5.0`, on a KDE/Plasma Wayland
session.

**Observed.** `nice-tuner-standalone` opens an **X11** window; `frost`'s own windows on
the same session are **Wayland**. Measured on the running processes, not inferred: the
tuner maps `memfd:xshmfence` (libxcb — an X11 client), `target/release/examples/minwin`
maps `memfd:smithay-client-toolkit` (sctk — a Wayland client) and no X shared memory.

**Why.** baseview has exactly three backends — `src/platform/{macos,win,x11}`, in the
pinned crate and on `master` alike — and its reason for existing, embedding, is an X11
operation: `XReparentWindow` into the window id the host hands over
(`platform/x11/event_loop.rs` → `xcb_window.rs`). The protocols agree: `nice-plug-core`'s
`ParentWindowHandle` carries `XlibWindow`/`XcbWindow`, each documented "Used with X11",
and nice-plug's CLAP arm tests `CLAP_WINDOW_API_X11` and nothing else. CLAP does declare
`CLAP_WINDOW_API_WAYLAND` (`clap-sys 0.5.0`, `ext/gui.rs`); no shipping host asks for it.

**Why Wayland cannot simply be used.** It has no cross-process window ids and no
reparenting — the compositor alone holds the global view — so the operation baseview
performs does not exist there. Worse for us than the window itself: baseview's
`visibility_tree.rs` walks the ancestor chain over `MapNotify`/`UnmapNotify`/
`SubstructureNotify` to decide whether `on_frame()` should fire at all, and a Wayland
surface gets no equivalent signal. [baseview#331](https://github.com/RustAudio/baseview/issues/331)
is that gap in the wild — an XEmbed parent XWayland reports as unmapped, `on_frame()`
never called, black panel with audio fine. Our `frame()` runs only from `on_frame`, so we
inherit the whole class. Meanwhile the things "modern" would buy an embedded panel —
decorations, placement, scale — all belong to the host. [baseview#135](https://github.com/RustAudio/baseview/issues/135)
("Add Wayland support") has been open since 2022-11 on the future milestone.

**The cost we actually pay** is HiDPI through XWayland, not the protocol:
[baseview#359](https://github.com/RustAudio/baseview/issues/359) — on X11 the host's scale
is only a *fallback* (`Xft.dpi` wins), while nice-plug still answers CLAP `set_scale` with
success. We pass `with_fallback_scale_factor` in `lib.rs`, so a scaled host can lay out at
its own factor while we draw at the system's. Test on a HiDPI host before trusting
`scale_factor`.

**What would change it** is not a baseview backend but
[`xwayland-shell-v1`](https://wayland.app/protocols/xwayland-shell-v1) (staging), which
associates X11 windows with `wl_surface`s properly — that would retire the #331 class of
bug without a line changed here. Nothing on the protocol side is coming for embedding.

**The one place Wayland is available to us** is standalone, where there is no host to
embed into: `nice-plug-core::EditorHandle` declares `type Window;` **unconstrained**
("typically `baseview::Window`") and `run_until_closed` is ours to implement, so a
Wayland top-level satisfies the trait as written and `frost::embedded` would not care —
it is handed a `wgpu::Surface`, and wgpu builds a Wayland one as readily. The price is
writing the loop, resize and visibility handling that baseview's `x11/` directory is;
re-wording the gate's winit guard, which today greps all of `src/`; and accepting that
window-position restore stays Windows-only, because Wayland will not let a client place
its own window. Not done, not scheduled — the session it was measured on runs Scale 1,
where the difference is invisible.
