//! frost — a minimal winit + wgpu immediate-mode drawing library.
//!
//! Provide a [`Scene`] and a [`Process`]: a function called once per frame
//! with a [`Context`] and the delta time in seconds since the previous
//! frame. The scene is drawn every frame, *after* the process runs and the
//! scene's tree has been updated by a visit (every node's
//! [`Node::process`], children before their parent), so both can mutate it
//! in place to animate it. Draw with window-centered
//! pixel coordinates: the origin is the window center, y points up, so the
//! top-left corner is `(-width/2, height/2)`. The unit is the logical
//! pixel — the OS's window units: one panel pixel each at 100% scaling,
//! more per unit on a high-density display (see [`Canvas::scale_factor`]) —
//! so a layout sized in these units keeps its physical size on every
//! screen, while text and shapes rasterize at the panel's own resolution.
//!
//! ```no_run
//! let scene = frost::Scene::new(frost::SceneNode {
//!     shape: Some(frost::Shape::Background {
//!         color: frost::Color { r: 0.05, g: 0.06, b: 0.12, a: 1.0 },
//!     }),
//!     ..Default::default()
//! });
//! frost::run(
//!     scene,
//!     |ctx: &mut frost::Context, _dt: f32| {
//!         let (w, h) = ctx.size();
//!         ctx.line(
//!             -w / 2.0, h / 2.0, w / 2.0, -h / 2.0,
//!             frost::Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
//!             2.0,
//!             0.0,
//!         );
//!         ctx.circle(
//!             0.0, 0.0, h / 4.0,
//!             frost::Color { r: 0.9, g: 0.4, b: 0.2, a: 1.0 },
//!             1.0,
//!         );
//!     },
//! );
//! ```
//!
//! Draw order is set by each object's `z`: lower `z` is drawn first (further
//! back). Objects with the same `z` are drawn in call order, so the last one
//! drawn is on top.
//!
//! A [`Scene`] can additionally define rendering [`Layer`]s: hard draw
//! partitions, each with its own order, its own parallax speed, and its own
//! root node. Groups are painted by ascending layer order — higher order
//! closer to the camera, drawn later, on top — and the scene's root subtree
//! is the group at the implicit order `0.0`, declared before the explicit
//! layers. Within a group the local `z` ordering applies. A layer's
//! [`Layer::repeat`] tiles it in one or both axes with a period of at least
//! the window size, so it can be scrolled through infinitely — an object
//! crossing a tile boundary is split into wrapping slices, and any setup
//! that would draw the same object twice aborts with an error.
//!
//! A [`Scene`] can also designate a camera node ([`Scene::camera`]): the
//! scene is then drawn in that node's coordinate space, so the node stays at
//! the window origin while the rest of the scene moves around it — hang the
//! camera under a moving node and it follows. Each group renders from the
//! camera scaled by its speed: the base group at the full speed `1.0`, each
//! [`Layer`] at its [`Layer::speed`].
//!
//! Each object is clipped to its tight bounding box (the scissor test), so a
//! frame's cost scales with the objects' on-screen areas, not the window size.
//!
//! Beyond the immediate draws, a [`Scene`] is a tree of [`SceneNode`]s where
//! each node holds a [`Transform`], a `scale`, a `modulate`, and an `order`,
//! all relative to its parent, plus its own optional [`Shape`]; the scale
//! and transform apply to the node's shape and compose onto its children,
//! the modulate multiplies into the node's shape color and composes onto
//! its children's, and the order does the same for the subtree's draw
//! order. Every [`SceneNode`] is a [`Node`]: its [`Node::visit`] walks its
//! children before itself (post-order), so the whole tree updates with a
//! single [`Scene::visit`]. A [`Shape::Background`]
//! node fills the whole window with its color, ignoring its transform, and
//! is drawn at the very back.
//! The scene passed to [`run`] is drawn every frame; use
//! [`Canvas::draw_scene`] to draw additional scenes.
//!
//! A [`ParticleSystem`] is a plain collection of [`Particle`]s for
//! effects like smoke or sparks. It is pure simulation, independent of the
//! renderer: spawn particles in [`Process::process`] and advance them each
//! frame with [`ParticleSystem::update`] under a constant gravity. Draw
//! them by putting the system in the node's [`Shape::Particles`] shape —
//! the batch is then drawn in the node's local space, transformed,
//! scaled, and tinted by the node — or, for batches that live directly in
//! the window's user space, with the deprecated immediate
//! [`Canvas::particles`] draw; either way each particle fades by its
//! remaining lifetime.
//!
//! A [`Shape::Polyline`] connects a list of points with straight segments
//! of a fixed width and draws the whole chain in one draw call, whatever
//! its length — the batched alternative to a run of thin rectangles or a
//! chain of [`Canvas::line`] draws.
//!
//! Randomness comes from [`Rng`], a small seeded splitmix64 generator:
//! seed it from the clock for a different stream on each run, or fix the
//! seed for a reproducible one. It is pure integer arithmetic, so it runs
//! identically on every target, native or `wasm32`, and keeps the crate
//! free of a `rand` dependency.
//!
//! Key presses are logged, the currently held keys are reported by
//! [`Context::key_down`], the mouse cursor's position by
//! [`Context::mouse_position`], the held mouse buttons by
//! [`Context::mouse_button_down`], the mouse wheel's movement since the
//! previous frame by [`Context::mouse_wheel`], and the connected gamepads
//! by [`Context::gamepads`]; Escape closes the window.
//!
//! On native targets, [`Sound`]s decoded at load time play through an
//! [`Audio`]: one-shots mix in parallel, and one sound loops at a time.
//!
//! A [`Diagnostics`] overlay reports the window size and, for each
//! statistic enabled in its [`DiagnosticsFlags`], a left-aligned line in
//! the window's top-left corner: the smoothed frame rate, the current
//! frame time, the last frame's total processing time (the engine's own
//! probe of its per-frame CPU work), that time excluding the overlay's own
//! update cost (the overlay times itself and subtracts), and the last
//! frame's draw-call count (the engine's own probe of how many GPU draw
//! calls it issued) — with a scrolling ten-second strip chart beneath each
//! line: the frame rate and the frame time in one series each, and the
//! processing time and the draw-call count folded into one chart each —
//! the total, the app's share, and the overlay's own share. Hold one in
//! the demo state and call its [`Process::process`] each frame; its parts
//! can be toggled at runtime — Alt-0 the overlay as a whole, Alt-1..Alt-4
//! the charts, Alt-T the readout lines — with the layout reflowing around
//! whatever is hidden, and Alt+'+' / Alt+'-' grow and shrink the whole
//! overlay, font included.
//!
//! Presentation is vsync'd by default: frames are presented once per
//! vertical blank, at the display's refresh rate — the rate
//! [`Context::expected_fps`] reports. [`run_configured`] takes a [`Config`]
//! to turn vsync off for uncapped frame rates.

use std::collections::{HashMap, HashSet};
use std::error::Error;

use wgpu::Instance;
// Only the native `run_configured` requests the adapter and device inline;
// on the web that happens inside `WebFrost`, which imports its own.
#[cfg(not(target_arch = "wasm32"))]
use wgpu::{DeviceDescriptor, RequestAdapterOptions};
use winit::event_loop::EventLoop;

mod backend;
use backend::*;

mod objects;
pub use objects::*;

mod canvas;
pub use canvas::Canvas;

/// The physical keyboard keys used by [`Context::key_down`], such as
/// `KeyCode::KeyW`. Physical keys identify the key's position on the
/// keyboard (its scancode), independent of the active layout, which is what
/// game controls like WASD want.
pub use winit::keyboard::KeyCode;

/// The mouse buttons used by [`Context::mouse_button_down`], such as
/// `MouseButton::Left`.
pub use winit::event::MouseButton;

/// The window the frames are drawn into, reachable through
/// [`Context::window`]: the engine's `winit` window, to parent native
/// dialogs to (such as `rfd`'s file picker, through its `set_parent`) so
/// they open on top of the window, and for any window state the platform
/// exposes.
pub use winit::window::Window;

/// The gamepad buttons and axes used to query the gamepads reported by
/// [`Context::gamepads`], such as `Axis::LeftStickY` and `Button::South`,
/// as gilrs names them.
pub use gilrs::{Axis, Button, Gamepad};

mod particles;
pub use particles::*;

mod shaders;

mod text;

mod tween;
pub use tween::*;

mod collision;
pub use collision::*;

mod diagnostics;
pub use diagnostics::{Diagnostics, DiagnosticsFlags};

mod ui;
pub use ui::{
    Align, Col, ColSize, Menu, MenuItem, TreeEvent, TreeLine, TreeOut, TreeSpec, TreeState,
    TreeStyle, Ui, UiStyle,
};

mod rng;
pub use rng::*;

#[cfg(not(target_arch = "wasm32"))]
mod audio;
#[cfg(not(target_arch = "wasm32"))]
pub use audio::{Audio, AudioError, Sound};

/// The per-frame context handed to [`Process::process`].
///
/// Derefs to the frame's [`Canvas`] (immediate draws) and
/// [`scene`](Context::scene) gives mutable access to the [`Scene`] passed to
/// [`run`]: mutate it here to animate it, and it is drawn after the process
/// returns.
pub struct Context<'c> {
    canvas: &'c mut Canvas,
    scene: &'c mut Scene,
    keys: &'c HashSet<KeyCode>,
    /// The character each held key typed as it went down, under the user's
    /// keyboard layout (see [`Context::char_down`]).
    typed: &'c HashMap<KeyCode, char>,
    /// The engine's live stretch filter, mutable so a running app can flip
    /// it (see [`Context::set_blit_filter`]).
    blit_filter: &'c mut SpriteFilter,
    /// The window this frame is drawn into, or `None` before the window
    /// exists.
    window: Option<&'c Window>,
    expected_fps: Option<f32>,
    /// The cursor's position in user coordinates, or `None` when the cursor
    /// is outside the window (or has never moved into it).
    mouse: Option<[f32; 2]>,
    /// The mouse buttons currently held down.
    mouse_buttons: &'c HashSet<MouseButton>,
    /// The mouse wheel's vertical movement since the previous frame, in
    /// lines (see [`Context::mouse_wheel`]).
    mouse_wheel: f32,
    /// The gamepad controller, or `None` when gilrs could not open the
    /// platform's input devices at startup (then no gamepad state exists).
    gilrs: Option<&'c gilrs::Gilrs>,
    /// The CPU time in milliseconds the engine spent processing the last
    /// completed frame (see [`Context::frame_processing_ms`]).
    frame_processing_ms: f64,
    /// The GPU draw calls the engine issued for the last completed frame
    /// (see [`Context::frame_draw_calls`]).
    frame_draw_calls: u32,
    /// The GPU draw calls the last completed frame spent on the
    /// [`Diagnostics`] overlay's own nodes (see
    /// [`Context::frame_diagnostic_draw_calls`]).
    frame_diagnostic_draw_calls: u32,
}

impl Context<'_> {
    /// Mutable access to the scene owned by [`run`].
    pub fn scene(&mut self) -> &mut Scene {
        &mut *self.scene
    }

    /// Whether the physical `key` is currently held down.
    ///
    /// The state is updated as keyboard events arrive, so it reflects every
    /// press and release since the previous frame.
    pub fn key_down(&self, key: KeyCode) -> bool {
        self.keys.contains(&key)
    }

    /// Whether a key held down right now typed the character `ch` as it
    /// went down, under the user's current keyboard layout.
    ///
    /// This is the layout-following sibling of [`Context::key_down`]:
    /// physical codes name US-ANSI key positions, so the key that types
    /// `'+'` lives wherever the user's layout puts it — left of `= ¨ ´`
    /// on a Swedish keyboard, say, where the US layout has its `-` key —
    /// and `char_down('+')` finds that user's `+` key wherever it is. The
    /// character is the one the press produced, so a modifier involved in
    /// the press (a macOS Option, which rewrites characters) shows up in
    /// it. Multi-character presses match on their first character.
    pub fn char_down(&self, ch: char) -> bool {
        self.typed.values().any(|&c| c == ch)
    }

    /// The window this frame is drawn into, or `None` before the window
    /// exists (it is created before the first frame, so it is `Some` for
    /// every frame that is actually drawn).
    ///
    /// It is the engine's [`Window`]: parent native dialogs to it (for
    /// example with `rfd::FileDialog::set_parent`) so they open on top of
    /// the window, and use it for any window state the platform exposes.
    pub fn window(&self) -> Option<&Window> {
        self.window
    }

    /// The filter that stretches the fixed [`Config::render_size`] buffer
    /// over the window — the upscaling filter (see
    /// [`Config::blit_filter`]).
    pub fn blit_filter(&self) -> SpriteFilter {
        *self.blit_filter
    }

    /// Sets the stretch filter at runtime: the mid-session sibling of
    /// setting [`Config::blit_filter`] at startup, effective from this
    /// frame's stretch pass on. The live change rebuilds the blit sampler
    /// and its bind group once, on the first frame that stretches with the
    /// new setting. Without [`Config::render_size`] there is no stretch to
    /// filter — the setting records faithfully and shows its effect the
    /// moment a fixed buffer ever runs.
    pub fn set_blit_filter(&mut self, filter: SpriteFilter) {
        *self.blit_filter = filter;
    }

    /// The frame rate this app is expected to run at, in frames per second.
    ///
    /// `Some(fps)` when [`Config::vsync`] is on and winit reports the
    /// refresh rate of the monitor the window sits on: with vsync, frames
    /// are presented once per vertical blank, at exactly that rate.
    /// `None` when vsync is off (presentation is uncapped, so there is no
    /// expected rate) or the refresh rate is unknown — the latter happens
    /// on some displays and in the browser.
    pub fn expected_fps(&self) -> Option<f32> {
        self.expected_fps
    }

    /// The CPU time, in milliseconds, the engine spent processing the last
    /// completed frame.
    ///
    /// The backend probes its own frame pipeline: the measurement starts
    /// just after the frame's surface acquire — so it includes this frame's
    /// [`Process::process`] call, the scene update, the draw list, and the
    /// command encoding — and stops when the frame's command buffer is
    /// submitted to the queue. The surface acquire, the present handoff to
    /// the display, and the wait for the next frame are excluded, so the
    /// value measures the CPU work of a frame rather than its on-screen
    /// duration — that is the `dt` [`Process::process`] receives. Before
    /// the first frame completes it is `0.0`.
    ///
    /// Compare it with `1000.0 / fps`: as the two approach each other the
    /// app becomes CPU-bound, and the gap between them is the headroom the
    /// [`Diagnostics`] overlay's processing-time chart shows.
    pub fn frame_processing_ms(&self) -> f64 {
        self.frame_processing_ms
    }

    /// The GPU draw calls the engine issued for the last completed frame.
    ///
    /// The backend counts one draw call per `pass.draw` / `pass.draw_indexed`
    /// it actually submits into the frame's render pass — a shape, a line, a
    /// sprite, a glyph quad, a chart polyline, a particle batch, and so on.
    /// Draws the engine skips are not counted: a shape fully outside the
    /// window, a background (which became the clear color), a light (which
    /// became part of the light field), or a particle batch with no live
    /// particles. Before the first frame completes it is `0`.
    ///
    /// This is the per-frame draw-call load of the app. In this engine each
    /// draw call also costs CPU work (a fresh uniform buffer, bind group,
    /// and scissor/pipeline state per draw), so a rising count shows up in
    /// the [`Diagnostics`] overlay's processing-time chart as well.
    pub fn frame_draw_calls(&self) -> u32 {
        self.frame_draw_calls
    }

    /// The GPU draw calls the last completed frame spent on the
    /// [`Diagnostics`] overlay's own nodes: the subset of
    /// [`frame_draw_calls`](Self::frame_draw_calls) whose draws the overlay
    /// produced — the text lines, the graph panels, the reference lines,
    /// and the chart polylines, one per glyph quad in the text.
    ///
    /// The overlay tags the scene nodes it creates with the
    /// [`SceneNode::diagnostic`] engine-managed marker, the marker rides
    /// onto every draw the nodes produce (the glyphs included), and the
    /// backend counts a tagged draw wherever it
    /// would count the total: the same skips (off-screen, background,
    /// light, empty) apply, so the value is a true subset of the total.
    /// Before the first frame completes it is `0`, like the total.
    pub fn frame_diagnostic_draw_calls(&self) -> u32 {
        self.frame_diagnostic_draw_calls
    }

    /// The mouse cursor's position in user coordinates (origin at the
    /// window's center, y up), or `None` when the cursor is outside the
    /// window.
    ///
    /// The position is updated as cursor events arrive, so it reflects the
    /// cursor's latest position rather than its position at frame start.
    /// While the cursor is outside the window — after it has left, or
    /// before it has first moved in — the last known position is no longer
    /// reported, so keep one yourself if you want the pointer to stick.
    pub fn mouse_position(&self) -> Option<[f32; 2]> {
        self.mouse
    }

    /// Whether the `button` mouse button is currently held down.
    ///
    /// The state is updated as mouse input events arrive, so it reflects
    /// every press and release since the previous frame. When the cursor
    /// leaves the window or the window loses focus, the held state is
    /// dropped, so a button can never appear stuck down.
    pub fn mouse_button_down(&self, button: MouseButton) -> bool {
        self.mouse_buttons.contains(&button)
    }

    /// The mouse wheel's vertical movement since the previous frame, in
    /// lines: positive when the wheel moves up (away from the user),
    /// negative when it moves down.
    ///
    /// The delta sums every wheel event that arrived since the previous
    /// frame — several notches may accumulate in one frame — and is consumed
    /// per frame: a frame that never reads it discards the delta. `0.0`
    /// when the wheel did not move.
    pub fn mouse_wheel(&self) -> f32 {
        self.mouse_wheel
    }

    /// The gamepads currently connected, in the order they connected.
    ///
    /// Each gamepad reports the buttons it has pressed and the values its
    /// axes rest at, updated as gamepad input events arrive, so the state
    /// reflects every event since the previous frame. Empty when no
    /// gamepad is connected, or when the platform's input devices could
    /// not be opened at startup.
    pub fn gamepads(&self) -> impl Iterator<Item = gilrs::Gamepad<'_>> {
        // `Option::iter` yields the controller once when it is present
        // and not at all when it is not, so one concrete iterator type
        // covers both cases: the connected gamepads, or nothing. The
        // iterator's ids are dropped here; the pads themselves are all
        // the user needs.
        self.gilrs.iter().flat_map(|g| g.gamepads().map(|(_, g)| g))
    }
}

impl std::ops::Deref for Context<'_> {
    type Target = Canvas;

    fn deref(&self) -> &Canvas {
        self.canvas
    }
}

impl std::ops::DerefMut for Context<'_> {
    fn deref_mut(&mut self) -> &mut Canvas {
        self.canvas
    }
}

/// Called once per frame; mutate the scene and draw into the canvas.
///
/// `ctx` gives mutable access to the scene owned by [`run`] (via
/// [`Context::scene`]) and derefs to the frame's [`Canvas`] for the
/// immediate draw methods. `dt` is the time in seconds
/// since the previous frame (`0.0` on the first frame, clamped to at most
/// `1.0`s to absorb stalls). Use it to advance animation state such as a
/// [`Tween`].
pub trait Process {
    fn process(&mut self, ctx: &mut Context, dt: f32);
}

/// Any closure `FnMut(&mut Context, f32)` is a [`Process`].
impl<F> Process for F
where
    F: FnMut(&mut Context, f32),
{
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        self(ctx, dt);
    }
}

/// Options for [`run_configured`].
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Present frames in sync with the display's vertical blank ("vsync").
    ///
    /// When `true`, frames are presented once per vertical blank and the
    /// frame rate is capped at the display's refresh rate — the rate
    /// [`Context::expected_fps`] reports. When `false`, frames are
    /// presented as soon as they are rendered, uncapped.
    pub vsync: bool,
    /// The window's initial inner size in logical pixels, `[width, height]`.
    ///
    /// Logical pixels are OS pixels: they equal physical pixels at 100%
    /// display scaling, but a scale factor of 2 doubles the panel pixels
    /// behind them — see [`Config::window_size_px`] for the physical-pixel
    /// request. `None` keeps the platform default (winit's default window
    /// size, or the web canvas's 900x600 default). The user can resize the
    /// window afterwards; this only sets the size it opens at.
    ///
    /// An opening request is an ambition, not a promise: a size larger
    /// than the primary monitor is clamped down to fit it, aspect
    /// preserved, so the window opens whole and centered rather than
    /// hanging off the screen.
    pub window_size: Option<[u32; 2]>,
    /// The window's initial inner size in *physical* pixels,
    /// `[width, height]`.
    ///
    /// winit converts the request through the monitor's scale factor, so
    /// `Some([1920, 1080])` opens a 1920x1080 framebuffer on every
    /// display: a 1920x1080-point window at 1.0 scale, and a
    /// 960x540-point one on a Retina Mac — the same physical window
    /// everywhere. Ignored on the web, where the canvas lives in CSS pixels
    /// and [`Config::window_size`] already says what it means. Wins over
    /// `window_size` when both are set; if both are `None` the platform
    /// default stands.
    ///
    /// The request sizes the *window*, not the user space: coordinates stay
    /// logical pixels ([`Canvas::size`]), so a fixed physical window is not
    /// a fixed render target — an app that lays itself out from
    /// `ctx.size()` adapts either way. For a fixed render target — art
    /// authored at one exact pixel size, rendered identically on every
    /// display — see [`Config::render_size`].
    ///
    /// Like [`Config::window_size`], a request larger than the primary
    /// monitor is clamped down to fit it, aspect preserved.
    pub window_size_px: Option<[u32; 2]>,
    /// Whether the user can resize the window. Default `true`.
    ///
    /// `false` removes the resize affordance (and pins the size with hard
    /// min/max bounds for window managers that ignore the style hint).
    /// Ignored on the web, where the page owns the canvas's size.
    pub resizable: bool,
    /// A fixed render size in *physical* pixels, `[width, height]`.
    ///
    /// The frame is rendered into an offscreen buffer of exactly this size
    /// and stretched over the whole window by one sampling pass — the
    /// classic "fixed virtual resolution" of game engines, plain blit and
    /// nothing smart. The app's user space is then the buffer's pixels:
    /// [`Canvas::size`] reports `render_size` and [`Canvas::scale_factor`]
    /// reports `1.0`, so an app authored at these pixels renders at exactly
    /// the same physical size on every display, and window resizes (and
    /// display density changes) only stretch the picture — never reflow it.
    /// When the window's surface happens to be exactly this size the buffer
    /// and the stretch pass are both skipped and the frame renders straight
    /// to the screen, so a window pinned with [`Config::window_size_px`] to
    /// the same numbers costs nothing over rendering directly.
    ///
    /// Stretched pixels are resampled, not re-rendered: text baked into a
    /// smaller buffer comes up soft, which is why apps that draw readable UI
    /// either leave this `None` (the default: the logical user space rides
    /// the display's own resolution, sharp at any density) or set it to the
    /// window's exact physical size.
    ///
    /// A window whose shape differs from the render size is never squashed:
    /// the buffer is presented letterboxed — the largest centered rectangle
    /// of the render size's aspect, black bars around it, and the mouse
    /// mapped through that rectangle (cursor coordinates past its edges
    /// report outside the play area, like an off-window position). A
    /// resizable window additionally gets an engine-side aspect lock: a
    /// drag to a wrong shape is answered with a request for the corrected
    /// one, so the corner slides along the diagonal. Maximization, tiling
    /// and window managers that decline the request simply letterbox.
    pub render_size: Option<[u32; 2]>,
    /// The filter that resamples the fixed `render_size` buffer when the
    /// stretch pass scales it to the window — the upscaling filter.
    /// Default [`SpriteFilter::Linear`].
    ///
    /// This only matters with [`Config::render_size`] set and a window
    /// whose surface differs from the buffer: without a fixed buffer there
    /// is no stretch to filter, and when the surface matches the buffer
    /// exactly the pass is skipped and every pixel lands one-for-one,
    /// filter irrelevant.
    ///
    /// [`SpriteFilter::Linear`] (bilinear) blends each output pixel from
    /// its neighbouring texels, smoothing scaled edges — the right default
    /// for authored art, and the only sane choice at the fractional
    /// ratios letterboxing and odd window sizes produce.
    /// [`SpriteFilter::Nearest`] copies one texel per output pixel whole,
    /// the pixel-art look: at an integer scale factor (a 1920x1080 buffer
    /// on a 3840x2160 surface, say) every texel becomes an exact block of
    /// screen pixels and hard edges stay hard. At fractional ratios the
    /// same filter shows its other face — uneven texel sizes, shimmering
    /// crawl while resizing — which is precisely the trade pixel purists
    /// make on purpose.
    pub blit_filter: SpriteFilter,
}

impl Default for Config {
    /// Vsync on, a resizable window, bilinear upscaling, and the
    /// platform's default size.
    fn default() -> Self {
        Self {
            vsync: true,
            window_size: None,
            window_size_px: None,
            resizable: true,
            render_size: None,
            blit_filter: SpriteFilter::Linear,
        }
    }
}

/// Opens the window and runs the event loop, calling `process` once per
/// frame and drawing `scene` after each call.
///
/// The scene is drawn every frame, *after* `process` returns, so `process`
/// can mutate it (via [`Context::scene`]) to animate it. Pass
/// [`Scene::default`] for apps that only use the immediate draw methods.
///
/// The window closes on Escape or when the user requests it.
#[cfg(not(target_arch = "wasm32"))]
pub fn run<P: Process>(scene: Scene, process: P) -> Result<(), Box<dyn Error>> {
    run_configured(scene, process, Config::default())
}

/// Same as [`run`], but with the given [`Config`].
///
/// Use it to turn vsync off (for uncapped frame rates), to open the window
/// at a specific [`Config::window_size`], or to inspect the expected frame
/// rate via [`Context::expected_fps`].
#[cfg(not(target_arch = "wasm32"))]
pub fn run_configured<P: Process>(
    scene: Scene,
    process: P,
    config: Config,
) -> Result<(), Box<dyn Error>> {
    log::info!("frost starting up");

    let instance = Instance::default();
    let adapter = block_on(instance.request_adapter(&RequestAdapterOptions::default()))
        .expect("no suitable GPU adapter found");
    log::info!("using adapter: {:?}", adapter.get_info().name);

    let (device, queue) = block_on(adapter.request_device(&DeviceDescriptor::default()))
        .expect("failed to create GPU device");

    let event_loop = EventLoop::new()?;
    let mut app = Frost::new(
        instance,
        adapter,
        device,
        queue,
        config.vsync,
        config.window_size,
        config.window_size_px,
        config.resizable,
        config.render_size,
        config.blit_filter,
        scene,
        process,
    );
    event_loop.run_app(&mut app)?;

    log::info!("event loop finished");
    Ok(())
}

/// Same as [`run`], but for `wasm32`.
///
/// In the browser, `request_adapter` and `request_device` are JS promises:
/// the first poll of each returns `Pending`, and the browser only resolves
/// the promise once the main thread is free — so the main thread cannot
/// block on it (blocking is exactly what would starve the resolution).
/// Instead, the window is shown immediately and the event loop (which never
/// blocks on the web) polls the setup future on every iteration; the first
/// frame is drawn as soon as the adapter and device resolve.
#[cfg(target_arch = "wasm32")]
pub fn run<P: Process>(scene: Scene, process: P) -> Result<(), Box<dyn Error>> {
    run_configured(scene, process, Config::default())
}

/// Same as [`run`](self::run), but with the given [`Config`].
#[cfg(target_arch = "wasm32")]
pub fn run_configured<P: Process>(
    scene: Scene,
    process: P,
    config: Config,
) -> Result<(), Box<dyn Error>> {
    log::info!("frost starting up");

    let instance = Instance::default();
    let event_loop = EventLoop::new()?;
    let mut app = WebFrost::new(
        instance,
        scene,
        process,
        config.vsync,
        config.window_size,
        config.window_size_px,
        config.resizable,
        config.render_size,
        config.blit_filter,
    );
    event_loop.run_app(&mut app)?;

    log::info!("event loop finished");
    Ok(())
}
