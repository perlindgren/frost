//! frost — a minimal winit + wgpu immediate-mode drawing library.
//!
//! Provide a [`Scene`] and a [`Process`]: a function called once per frame
//! with a [`Context`] and the delta time in seconds since the previous
//! frame. The scene is drawn every frame, *after* the process runs and the
//! scene's tree has been updated by a visit (every node's
//! [`Node::process`], children before their parent), so both can mutate it
//! in place to animate it. Draw with window-centered
//! pixel coordinates: the origin is the window center, y points up, so the
//! top-left corner is `(-width/2, height/2)`.
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
//! Key presses are logged, the currently held keys are reported by
//! [`Context::key_down`], and Escape closes the window.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::sync::Arc;

use wgpu::{DeviceDescriptor, Instance, RequestAdapterOptions};
use winit::event_loop::EventLoop;

mod backend;
use backend::*;

mod objects;
pub use objects::*;

/// The physical keyboard keys used by [`Context::key_down`], such as
/// `KeyCode::KeyW`. Physical keys identify the key's position on the
/// keyboard (its scancode), independent of the active layout, which is what
/// game controls like WASD want.
pub use winit::keyboard::KeyCode;

mod shaders;

mod text;

mod tween;
pub use tween::*;

/// The drawing surface for a frame, reachable through the [`Context`] passed
/// to [`Process::process`] (which derefs to it).
///
/// Coordinates are in pixels with the origin at the window center and the y
/// axis pointing up: the top-left corner is `(-width/2, height/2)` and the
/// bottom-right corner is `(width/2, -height/2)`.
pub struct Canvas {
    size: (f32, f32),
    draws: Vec<Draw>,
}

impl Canvas {
    fn new(pixel_size: (u32, u32)) -> Self {
        Self {
            size: (pixel_size.0 as f32, pixel_size.1 as f32),
            draws: Vec::new(),
        }
    }

    /// The window size in pixels as `(width, height)`.
    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    /// Draws a line from `(x0, y0)` to `(x1, y1)` in `color` with `width` in
    /// pixels.
    ///
    /// `z` is the draw order: lower `z` is drawn first (further back). Lines
    /// with the same `z` are drawn in call order, so the last one drawn is on
    /// top.
    #[allow(clippy::too_many_arguments)] // immediate-mode draw call
    pub fn line(
        &mut self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        color: Color,
        width: f32,
        z: f32,
    ) {
        self.draws.push(Draw::Line {
            a: self.user_to_pixels(x0, y0),
            b: self.user_to_pixels(x1, y1),
            width: width.max(0.0),
            color,
            z,
        });
    }

    /// Draws a filled circle centered at `(cx, cy)` with `radius` in pixels,
    /// in `color`.
    ///
    /// `z` is the draw order: lower `z` is drawn first (further back). Circles
    /// with the same `z` are drawn in call order, so the last one drawn is on
    /// top.
    pub fn circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color, z: f32) {
        self.draws.push(Draw::Circle {
            center: self.user_to_pixels(cx, cy),
            radius: radius.max(0.0),
            color,
            z,
        });
    }

    /// Draws a filled rectangle centered at `(cx, cy)` with `dx` and `dy`
    /// extents (half-width and half-height) in pixels, in `color`.
    ///
    /// `z` is the draw order: lower `z` is drawn first (further back).
    /// Rectangles with the same `z` are drawn in call order, so the last one
    /// drawn is on top.
    pub fn rectangle(&mut self, cx: f32, cy: f32, dx: f32, dy: f32, color: Color, z: f32) {
        self.draws.push(Draw::Rectangle {
            center: self.user_to_pixels(cx, cy),
            extent: [dx.max(0.0), dy.max(0.0)],
            color,
            z,
        });
    }

    /// Draws a [`Scene`] into the canvas.
    ///
    /// The scene passed to [`run`] is drawn automatically every frame, after
    /// the [`Process`] runs; use this method to draw additional scenes.
    ///
    /// Scene coordinates are the same user space as the direct draw methods:
    /// the origin is at the window center, y points up, and positive
    /// rotations are counterclockwise on screen.
    ///
    /// The scene is walked depth-first from the root. Each node's transform
    /// and scale are relative to its parent and compose onto the parent's
    /// (scale innermost), so a descendant's world transform is the full
    /// root-to-leaf composition. Each node's modulate multiplies into its
    /// shape's color and composes onto its children's, so a descendant's
    /// color is the channel-wise product of the modulates on the path from
    /// the root.
    /// Every node with a shape draws it (in the node's own local space)
    /// before its children, so parents paint under their descendants.
    ///
    /// All scene shapes share `z = 0.0`, so they are ordered by tree position
    /// and interleave with immediate draws by the usual stable `z` ordering.
    /// A [`Shape::Background`] is the exception: it ignores its transform,
    /// always sorts to the very back, and becomes the frame's clear color.
    pub fn draw_scene(&mut self, scene: &Scene) {
        self.draw_node(&scene.root, &Transform::identity(), 0.0, WHITE);
    }

    fn draw_node(
        &mut self,
        node: &SceneNode,
        parent: &Transform,
        order: f32,
        modulate: Color,
    ) {
        // The node's effective draw order: the order inherited from its
        // ancestors plus its own, applied to the node's shape and passed on
        // to its whole subtree, just like the transform and the scale.
        let order = order + node.order;
        // The node's effective color modulation: the modulation inherited
        // from its ancestors multiplied by its own, applied to the node's
        // shape's color and passed on to its whole subtree, just like the
        // transform and the scale.
        let modulate = modulate.mul(node.modulate);
        // The node's world transform in user space: its scale first
        // (innermost, in the node's own space), then its transform (local ->
        // parent space), then the parent's world transform. Through `world`
        // the scale therefore applies to the node's shape and to its whole
        // subtree, just like the transform.
        let local = Transform::scale(node.scale[0], node.scale[1])
            .compose(&node.transform);
        let world = local.compose(parent);
        if let Some(shape) = &node.shape {
            // Scale the anti-alias band with the transform's scale so it
            // stays a constant number of screen pixels wide under scaling.
            let [sx, sy] = world.scales();
            let aa = AA_BAND / sx.max(sy).max(1e-9);
            let draw = match shape {
                // The shape shader evaluates in pixel space (top-left, y
                // down), so compose the user-to-pixel transform onto the
                // world transform, like the direct-draw methods do for
                // their arguments.
                Shape::Circle {
                    center,
                    radius,
                    color,
                } => Draw::Shape {
                    world: world.compose(&self.user_to_pixel()),
                    center: *center,
                    params: [(*radius).max(0.0), 0.0],
                    kind: 0.0,
                    aa,
                    color: color.mul(modulate),
                    z: order,
                },
                Shape::Rectangle {
                    center,
                    extent,
                    color,
                } => Draw::Shape {
                    world: world.compose(&self.user_to_pixel()),
                    center: *center,
                    params: [extent[0].max(0.0), extent[1].max(0.0)],
                    kind: 1.0,
                    aa,
                    color: color.mul(modulate),
                    z: order,
                },
                // The sprite's local space is centered on the origin, one
                // texture pixel per scene pixel, so its extent is the
                // texture size.
                Shape::Sprite {
                    data,
                    width,
                    height,
                    color,
                    alpha,
                } => Draw::Sprite {
                    world: world.compose(&self.user_to_pixel()),
                    data: data.clone(),
                    size: [(*width as f32).max(0.0), (*height as f32).max(0.0)],
                    texture_size: [*width, *height],
                    aa,
                    tint: color.mul(modulate),
                    alpha: *alpha,
                    uv_rect: [0.0, 0.0, 1.0, 1.0],
                    z: order,
                },
                // Text is recorded in user space; `Canvas::expand_text`
                // lays it out and turns each glyph into a sprite quad
                // before the frame is rendered.
                Shape::Text {
                    text,
                    font,
                    size,
                    color,
                    alpha,
                } => Draw::Text {
                    world,
                    font: font.clone(),
                    text: text.clone(),
                    size: *size,
                    color: color.mul(modulate),
                    alpha: *alpha,
                    z: order,
                },
                // The background ignores its transform: it is recorded in
                // call order and becomes the frame's clear color at render
                // time.
                Shape::Background { color } => {
                    Draw::Background { color: color.mul(modulate) }
                }
            };
            self.draws.push(draw);
        }
        for child in &node.children {
            self.draw_node(child, &world, order, modulate);
        }
    }

    /// The affine transform from user coordinates (window center origin,
    /// y up) to the pixel coordinates the shaders use (top-left origin,
    /// y down): `(x, y) -> (x + w/2, h/2 - y)`.
    fn user_to_pixel(&self) -> Transform {
        Transform {
            m: [[1.0, 0.0], [0.0, -1.0]],
            t: [self.size.0 / 2.0, self.size.1 / 2.0],
        }
    }

    /// Expands every [`Draw::Text`] in this frame into one
    /// [`Draw::Sprite`] per shaped glyph, spliced in at the text draw's
    /// position so the stable z-sort keeps the glyph quads where the text
    /// node was.
    ///
    /// The glyphs are laid out with `text::layout` (pen positions, y up,
    /// from the text's left baseline origin) and rasterized with
    /// `text::rasterize` into the per-`(font, size)` atlas in `atlases`,
    /// which the caller keeps between frames so unchanged text never
    /// re-rasterizes and its texture buffer keeps a stable identity.
    /// Each glyph's quad is centered on its ink box; the whole text block
    /// is centered on the text node's origin.
    pub(crate) fn expand_text(&mut self, atlases: &mut HashMap<(u64, u32), text::Atlas>) {
        let draws = std::mem::take(&mut self.draws);
        let mut expanded = Vec::with_capacity(draws.len());
        for draw in draws {
            match draw {
                Draw::Text {
                    world,
                    font,
                    text: string,
                    size,
                    color,
                    alpha,
                    z,
                } => {
                    // A broken font leaves the text undrawn; `Shape::text`
                    // validates the font up front, so this only guards a
                    // buffer that turned out unreadable.
                    let Some(layout) = text::layout(&font, &string, size) else {
                        continue;
                    };
                    let key = (Arc::as_ptr(&font) as *const () as u64, size.to_bits());
                    let mut atlas = atlases.get(&key).cloned().unwrap_or_default();
                    // Rasterize the glyphs the atlas does not have yet.
                    let mut missing = Vec::new();
                    for glyph in &layout.glyphs {
                        if atlas.has(glyph.id) {
                            continue;
                        }
                        if let Some(raster) = text::rasterize(&font, glyph.id, size) {
                            missing.push((glyph.id, raster));
                        }
                    }
                    if !missing.is_empty() {
                        atlas.insert_many(&missing);
                    }
                    // Keep the atlas in the map even when nothing was
                    // packed (e.g. all-space text) so later frames hit it.
                    let atlas = atlases.entry(key).or_insert(atlas);
                    let [sx, sy] = world.scales();
                    let aa = AA_BAND / sx.max(sy).max(1e-9);
                    // Center the text block (width by ascent + descent,
                    // baseline at the pen-space origin, y up) on the node's
                    // origin.
                    let origin = [
                        -layout.width / 2.0,
                        (layout.ascent - layout.descent) / 2.0,
                    ];
                    let (atlas_w, atlas_h) = (atlas.width as f32, atlas.height as f32);
                    for glyph in &layout.glyphs {
                        let Some(cell) = atlas.cell(glyph.id) else {
                            continue;
                        };
                        let (gw, gh) = (cell.width as f32, cell.height as f32);
                        // The glyph's pen position in the node's local
                        // space, plus the ink box's center offset from the
                        // pen: `left` right and `top - height / 2` above the
                        // baseline.
                        let ink = [
                            origin[0] + glyph.x + cell.left as f32 + gw / 2.0,
                            origin[1] + glyph.y + cell.top as f32 - gh / 2.0,
                        ];
                        let glyph_world = Transform::translate(ink[0], ink[1])
                            .compose(&world)
                            .compose(&self.user_to_pixel());
                        // Inset the cell by half a texel so the quad's edges
                        // sample the edge texels exactly instead of blending
                        // with the neighboring cell.
                        let uv_rect = [
                            (cell.x as f32 + 0.5) / atlas_w,
                            (cell.y as f32 + 0.5) / atlas_h,
                            (cell.x as f32 + cell.width as f32 - 0.5) / atlas_w,
                            (cell.y as f32 + cell.height as f32 - 0.5) / atlas_h,
                        ];
                        expanded.push(Draw::Sprite {
                            world: glyph_world,
                            data: atlas.data.clone(),
                            size: [gw, gh],
                            texture_size: [atlas.width, atlas.height],
                            aa,
                            tint: color,
                            alpha,
                            uv_rect,
                            z,
                        });
                    }
                }
                other => expanded.push(other),
            }
        }
        self.draws = expanded;
    }

    /// Converts user coordinates (window center origin, y up) to the pixel
    /// coordinates the shaders use (top-left origin, y down).
    fn user_to_pixels(&self, x: f32, y: f32) -> [f32; 2] {
        self.user_to_pixel().apply([x, y])
    }
}

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
    log::info!("frost starting up");

    let instance = Instance::default();
    let adapter = block_on(instance.request_adapter(&RequestAdapterOptions::default()))
        .expect("no suitable GPU adapter found");
    log::info!("using adapter: {:?}", adapter.get_info().name);

    let (device, queue) = block_on(adapter.request_device(&DeviceDescriptor::default()))
        .expect("failed to create GPU device");

    let event_loop = EventLoop::new()?;
    let mut app = Frost::new(instance, adapter, device, queue, scene, process);
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
    log::info!("frost starting up");

    let instance = Instance::default();
    let event_loop = EventLoop::new()?;
    let mut app = WebFrost::new(instance, scene, process);
    event_loop.run_app(&mut app)?;

    log::info!("event loop finished");
    Ok(())
}
