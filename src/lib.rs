//! frost — a minimal winit + wgpu immediate-mode drawing library.
//!
//! Provide a [`Scene`] and a [`Process`]: a function called once per frame
//! with a [`Context`] and the delta time in seconds since the previous
//! frame. The scene is drawn every frame, *after* the process runs, so the
//! process can mutate it in place to animate it. Draw with window-centered
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
//! order. A [`Shape::Background`]
//! node fills the whole window with its color, ignoring its transform, and
//! is drawn at the very back.
//! The scene passed to [`run`] is drawn every frame; use
//! [`Canvas::draw_scene`] to draw additional scenes.
//!
//! Key presses are logged, the currently held keys are reported by
//! [`Context::key_down`], and Escape closes the window.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::future::Future;
#[cfg(target_arch = "wasm32")]
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll, Waker};

use wgpu::{
    Adapter, AddressMode, BindGroup, BindGroupDescriptor, BindGroupEntry, BindingResource,
    Buffer, BufferBinding, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    CommandEncoderDescriptor, CurrentSurfaceTexture, Device, DeviceDescriptor, Extent3d,
    FilterMode, FragmentState, Instance, MipmapFilterMode, MultisampleState,
    PipelineCompilationOptions, PrimitiveState, Queue, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, RequestAdapterOptions,
    Sampler, SamplerDescriptor, ShaderModule, ShaderModuleDescriptor, ShaderSource, StoreOp,
    Surface, SurfaceTexture, TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor, VertexState,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
#[cfg(target_arch = "wasm32")]
use winit::event::StartCause;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

mod objects;
pub use objects::*;

/// The physical keyboard keys used by [`Context::key_down`], such as
/// `KeyCode::KeyW`. Physical keys identify the key's position on the
/// keyboard (its scancode), independent of the active layout, which is what
/// game controls like WASD want.
pub use winit::keyboard::KeyCode;

mod shaders;
use shaders::*;

mod text;

mod tween;
pub use tween::*;

/// The frame's clear color when no [`Shape::Background`] is drawn.
const DEFAULT_BACKGROUND: Color = Color {
    r: 0.07,
    g: 0.09,
    b: 0.14,
    a: 1.0,
};

/// The anti-alias band in pixels, mirrored by the `aa` constant in the SDF
/// shader sources; keep the two in sync so bounding boxes cover every pixel
/// the shaders can write.
const AA_BAND: f32 = 0.75;
// ============================ public API ============================

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

/// A drawing recorded for the current frame, in pixel space.
///
/// `z` is the draw order: lower `z` is drawn first (further back). Drawings
/// with the same `z` are drawn in call order, so the last one drawn is on top.
#[derive(Clone, Debug)]
enum Draw {
    Line {
        a: [f32; 2],
        b: [f32; 2],
        width: f32,
        color: Color,
        z: f32,
    },
    Circle {
        center: [f32; 2],
        radius: f32,
        color: Color,
        z: f32,
    },
    Rectangle {
        center: [f32; 2],
        extent: [f32; 2],
        color: Color,
        z: f32,
    },
    Shape {
        /// The transform from the shape's local space to pixel space.
        world: Transform,
        /// The shape center in its local space.
        center: [f32; 2],
        /// Circle: `[radius, 0.0]`; rectangle: `[half_width, half_height]`.
        params: [f32; 2],
        /// `0.0` for a circle, `1.0` for a rectangle.
        kind: f32,
        /// The anti-alias band in local units (the screen `AA_BAND` divided
        /// by the transform's scale).
        aa: f32,
        color: Color,
        z: f32,
    },
    /// A sprite: a texture sampled in the sprite's local space, which is
    /// centered on the origin and extends by `size / 2` along each axis.
    Sprite {
        /// The transform from the sprite's local space to pixel space.
        world: Transform,
        /// The RGBA8 pixel data, row by row, top row first. Two sprites
        /// created from the same file share this buffer; glyph quads
        /// expanded by [`Canvas::expand_text`] share the same atlas buffer.
        data: Arc<[u8]>,
        /// The local extent of the quad in local units; the sampled region
        /// spans `uv_rect` of the texture.
        size: [f32; 2],
        /// The sampled texture's size in pixels. Normally equal to
        /// `size` (one unit per texture pixel); for glyph quads the texture
        /// is a shared atlas that is larger than the quad.
        texture_size: [u32; 2],
        /// The anti-alias band in local units (the screen `AA_BAND` divided
        /// by the transform's scale).
        aa: f32,
        /// The per-pixel tint, multiplied with every sampled pixel.
        tint: Color,
        /// The sprite's opacity, multiplied with the texture's own alpha.
        alpha: f32,
        /// The sub-rectangle of the texture the quad covers, as
        /// `[min_x, min_y, max_x, max_y]` in [0, 1] texture coordinates
        /// (top-left origin). `[0.0, 0.0, 1.0, 1.0]` covers the whole
        /// texture.
        uv_rect: [f32; 4],
        z: f32,
    },
    /// A block of text: expanded into one [`Draw::Sprite`] per glyph by
    /// [`Canvas::expand_text`] before the frame is rendered, so this
    /// variant never reaches the render loop itself.
    Text {
        /// The node's world transform in user space (not yet composed with
        /// the user-to-pixel transform).
        world: Transform,
        /// The font file's bytes.
        font: Arc<[u8]>,
        /// The string to lay out.
        text: String,
        /// The font size in pixels per em.
        size: f32,
        /// The glyph color.
        color: Color,
        /// The text's opacity.
        alpha: f32,
        z: f32,
    },
    /// A [`Shape::Background`]: never drawn; it is promoted to the frame's
    /// clear color at render time.
    Background {
        color: Color,
    },
}

impl Draw {
    fn z(&self) -> f32 {
        match self {
            Draw::Line { z, .. } => *z,
            Draw::Circle { z, .. } => *z,
            Draw::Rectangle { z, .. } => *z,
            Draw::Shape { z, .. } => *z,
            Draw::Sprite { z, .. } => *z,
            Draw::Text { z, .. } => *z,
            // The background is always at the very back.
            Draw::Background { .. } => f32::MIN,
        }
    }

    /// The tight pixel rectangle the draw can write to, including the
    /// anti-alias band the shaders render beyond each geometric edge.
    ///
    /// `area` is the render area in pixels; the result is clamped to it, or
    /// `None` if the draw is fully outside the surface or is a background
    /// (which is never drawn — it becomes the frame's clear color).
    fn scissor_rect(&self, area: [u32; 2]) -> Option<[u32; 4]> {
        if let Draw::Background { .. } = self {
            return None;
        }
        let (min, max) = match self {
            Draw::Line { a, b, width, .. } => {
                let pad = width * 0.5 + AA_BAND;
                (
                    [a[0].min(b[0]) - pad, a[1].min(b[1]) - pad],
                    [a[0].max(b[0]) + pad, a[1].max(b[1]) + pad],
                )
            }
            Draw::Circle {
                center, radius, ..
            } => {
                let pad = *radius + AA_BAND;
                (
                    [center[0] - pad, center[1] - pad],
                    [center[0] + pad, center[1] + pad],
                )
            }
            Draw::Rectangle {
                center, extent, ..
            } => (
                [
                    center[0] - extent[0] - AA_BAND,
                    center[1] - extent[1] - AA_BAND,
                ],
                [
                    center[0] + extent[0] + AA_BAND,
                    center[1] + extent[1] + AA_BAND,
                ],
            ),
            Draw::Shape {
                world,
                center,
                params,
                kind,
                aa,
                ..
            } => {
                // The shape's bounding box in its local space, including the
                // local anti-alias band. A circle's box is a square around its
                // radius; a rectangle's box is its extents.
                let (hx, hy) = if *kind < 0.5 {
                    let r = params[0].max(0.0) + *aa;
                    (r, r)
                } else {
                    (
                        params[0].max(0.0) + *aa,
                        params[1].max(0.0) + *aa,
                    )
                };
                aabb_of_box(world, *center, hx, hy)
            }
            Draw::Sprite {
                world, size, aa, ..
            } => {
                // The sprite's box is the texture, centered on the origin,
                // plus the local anti-alias band.
                aabb_of_box(world, [0.0, 0.0], size[0] * 0.5 + *aa, size[1] * 0.5 + *aa)
            }
            // A background never draws; the early return above covers it.
            Draw::Background { .. } => unreachable!(),
            // Text is expanded into glyph sprites by `Canvas::expand_text`
            // before the render loop, so it never reaches the scissor.
            Draw::Text { .. } => unreachable!(),
        };
        clamp_box_to_area(min, max, area)
    }
}

/// The axis-aligned bounding box of the four transformed corners of a box in
/// local space centered at `center` with half extents `(hx, hy)`. The box may
/// be rotated or skewed by `world`, so the corners are transformed first and
/// the axis-aligned box of the result is taken.
fn aabb_of_box(world: &Transform, center: [f32; 2], hx: f32, hy: f32) -> ([f32; 2], [f32; 2]) {
    let corners = [
        world.apply([center[0] - hx, center[1] - hy]),
        world.apply([center[0] + hx, center[1] - hy]),
        world.apply([center[0] - hx, center[1] + hy]),
        world.apply([center[0] + hx, center[1] + hy]),
    ];
    let mut min = corners[0];
    let mut max = corners[0];
    for corner in corners.iter().skip(1) {
        min = [min[0].min(corner[0]), min[1].min(corner[1])];
        max = [max[0].max(corner[0]), max[1].max(corner[1])];
    }
    (min, max)
}

/// The tight pixel rectangle a bounding box touches, clamped to the render
/// `area`, or `None` if the box is empty or fully outside the surface.
///
/// `floor`/`ceil` pick the pixel columns and rows the box touches; the
/// clamps keep the box inside the surface (float-to-int casts saturate, so
/// out-of-range coordinates stay consistent).
fn clamp_box_to_area(min: [f32; 2], max: [f32; 2], area: [u32; 2]) -> Option<[u32; 4]> {
    let x0 = min[0].max(0.0).floor() as u32;
    let y0 = min[1].max(0.0).floor() as u32;
    let x1 = max[0].min(area[0] as f32).ceil() as u32;
    let y1 = max[1].min(area[1] as f32).ceil() as u32;
    if x1 > x0 && y1 > y0 {
        Some([x0, y0, x1 - x0, y1 - y0])
    } else {
        None
    }
}

/// The frame's clear color: the color of the last [`Draw::Background`] in
/// call order, or [`DEFAULT_BACKGROUND`] when the frame has none.
fn clear_color(draws: &[Draw]) -> Color {
    draws
        .iter()
        .rev()
        .find_map(|draw| match draw {
            Draw::Background { color } => Some(*color),
            _ => None,
        })
        .unwrap_or(DEFAULT_BACKGROUND)
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
    let mut app = Frost {
        instance,
        adapter,
        device,
        queue,
        window_id: None,
        window: None,
        logical_size: (0, 0),
        scale: 1.0,
        surface: None,
        line_pipeline: None,
        circle_pipeline: None,
        rect_pipeline: None,
        shape_pipeline: None,
        sprite_pipeline: None,
        sprite_resources: HashMap::new(),
        text_atlases: HashMap::new(),
        format: None,
        last_millis: None,
        scene,
        keys: HashSet::new(),
        process,
    };
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
    let mut app = WebFrost {
        instance: Some(instance),
        core: Some(Core { scene, process }),
        window: None,
        gpu: GpuInit::Idle,
        frost: None,
    };
    event_loop.run_app(&mut app)?;

    log::info!("event loop finished");
    Ok(())
}

// ============================ internal machinery ============================

struct Frost<P: Process> {
    instance: Instance,
    adapter: Adapter,
    device: Device,
    queue: Queue,
    #[allow(dead_code)]
    window_id: Option<WindowId>,
    /// The winit window (shared), kept so we can call `request_redraw` for
    /// continuous per-frame rendering.
    window: Option<Arc<Window>>,
    logical_size: (u32, u32),
    scale: f32,
    surface: Option<Surface<'static>>,
    line_pipeline: Option<RenderPipeline>,
    circle_pipeline: Option<RenderPipeline>,
    rect_pipeline: Option<RenderPipeline>,
    shape_pipeline: Option<RenderPipeline>,
    sprite_pipeline: Option<RenderPipeline>,
    /// The GPU resources for each distinct sprite image, keyed by the
    /// pointer of its pixel-data `Arc`. Sprites sharing one file share one
    /// texture, so the map stays bounded by the number of distinct images.
    /// A `TextureView` keeps its texture alive, so only the view and the
    /// sampler are stored.
    sprite_resources: HashMap<*const (), (TextureView, Sampler)>,
    /// The rasterized glyph atlas for each distinct `(font, size)` pair,
    /// keyed by the font buffer's pointer and the size's bits. Kept between
    /// frames so unchanged text never re-rasterizes and its pixel buffer —
    /// and therefore the GPU texture in `sprite_resources` — keeps a stable
    /// identity.
    text_atlases: HashMap<(u64, u32), text::Atlas>,
    /// The surface format the current pipelines were built for; they are only
    /// rebuilt when this changes.
    format: Option<TextureFormat>,
    /// Millisecond timestamp of the previous rendered frame, used to
    /// compute `dt` (see `now_millis`).
    last_millis: Option<f64>,
    /// The scene drawn every frame, after the process runs.
    scene: Scene,
    /// The physical keys currently held down, updated as keyboard events arrive.
    keys: HashSet<KeyCode>,
    process: P,
}

/// Creates the frost window and requests its first frame.
///
/// On the web, winit's canvas is neither appended to the page nor sized by
/// default: without the append it is invisible, and without a size it stays
/// the browser's 300x150 default.
fn create_window(event_loop: &ActiveEventLoop) -> Arc<Window> {
    let attributes = Window::default_attributes().with_title("frost");
    #[cfg(target_arch = "wasm32")]
    let attributes = {
        use winit::dpi::LogicalSize;
        use winit::platform::web::WindowAttributesExtWebSys;

        attributes
            .with_append(true)
            .with_inner_size(LogicalSize::new(900, 600))
    };
    let window = Arc::new(
        event_loop
            .create_window(attributes)
            .expect("failed to create window"),
    );
    // winit (Wayland) only delivers RedrawRequested after a compositor
    // frame callback, so explicitly request the first frame; otherwise
    // the window is never mapped and nothing is ever drawn.
    window.request_redraw();
    window
}

impl<P: Process> ApplicationHandler for Frost<P> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }
        self.attach_window(create_window(event_loop));
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                log::info!("key event: {event:?}");
                // Track the held physical keys so `Context::key_down` can
                // report them to the process on the next frame.
                if let PhysicalKey::Code(code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed => {
                            self.keys.insert(code);
                        }
                        ElementState::Released => {
                            self.keys.remove(&code);
                        }
                    }
                }
                if event.state == ElementState::Pressed && event.logical_key == NamedKey::Escape {
                    log::info!("escape pressed, exiting");
                    event_loop.exit();
                }
            }
            WindowEvent::Focused(false) => {
                // The window lost focus: key releases may never arrive, so
                // drop the held-key state rather than stick the keys.
                self.keys.clear();
            }
            WindowEvent::CloseRequested => {
                log::info!("window close requested, exiting");
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                // `size` is the physical client size; store the logical size
                // so `pixel_size()` reproduces the true physical pixels.
                if size.width > 0 && size.height > 0 {
                    let scale = (self.scale as f64).max(0.01);
                    self.logical_size = (
                        (size.width as f64 / scale).max(1.0).round() as u32,
                        (size.height as f64 / scale).max(1.0).round() as u32,
                    );
                    self.resize();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor as f32;
                self.resize();
            }
            WindowEvent::RedrawRequested => {
                self.render();
                // winit only delivers RedrawRequested after we ask for it, so
                // request the next frame to keep animation running continuously.
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

// ==================== web startup (wasm32 only) ====================
//
// In the browser, `request_adapter` and `request_device` are JS promises:
// the first poll of each returns `Pending`, and the browser only runs the
// resolution microtask once the main thread is free — so the main thread
// cannot block on them. `run` on wasm therefore defers the GPU setup: the
// canvas is created immediately, and the event loop (which never blocks on
// the web) polls the setup future on every iteration. When it completes,
// the real `Frost` app takes over and drives the frame loop.

/// The scene and process, held until the GPU setup completes on the web.
#[cfg(target_arch = "wasm32")]
struct Core<P: Process> {
    scene: Scene,
    process: P,
}

/// The in-flight async GPU setup on the web.
#[cfg(target_arch = "wasm32")]
enum GpuInit {
    /// Not started yet (before the first `resumed`).
    Idle,
    /// The combined `request_adapter` + `request_device` future.
    Init(Pin<Box<dyn Future<Output = Result<(Adapter, Device, Queue), String>>>>),
    /// The setup failed and the page shows the error.
    Failed,
}

/// The web event-loop handler: creates the window immediately, drives the
/// async GPU setup, and hands everything to `Frost` once it completes.
#[cfg(target_arch = "wasm32")]
struct WebFrost<P: Process> {
    instance: Option<Instance>,
    /// The scene and process, held until the GPU setup completes.
    core: Option<Core<P>>,
    /// The canvas, created before the GPU is ready.
    window: Option<Arc<Window>>,
    /// The in-flight async GPU setup.
    gpu: GpuInit,
    /// The real app, once the adapter and device are ready.
    frost: Option<Frost<P>>,
}

#[cfg(target_arch = "wasm32")]
impl<P: Process> ApplicationHandler for WebFrost<P> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.frost.is_some() || matches!(self.gpu, GpuInit::Failed) {
            return;
        }
        // Create the canvas immediately so the page shows *something* while
        // the GPU setup is in flight.
        if self.window.is_none() {
            self.window = Some(create_window(event_loop));
        }
        if matches!(self.gpu, GpuInit::Idle) {
            // The async block owns its own `Instance` clone (a cheap
            // refcounted bump), so the future borrows nothing from `self`.
            let instance = self
                .instance
                .clone()
                .expect("the instance is created in `run`");
            self.gpu = GpuInit::Init(Box::pin(async move {
                let adapter = instance
                    .request_adapter(&RequestAdapterOptions::default())
                    .await
                    .map_err(|error| {
                        format!(
                            "no suitable GPU adapter found: {error} \
                             (is WebGPU enabled in this browser?)"
                        )
                    })?;
                log::info!("using adapter: {:?}", adapter.get_info().name);
                let (device, queue) = adapter
                    .request_device(&DeviceDescriptor::default())
                    .await
                    .map_err(|error| format!("failed to create GPU device: {error}"))?;
                Ok((adapter, device, queue))
            }));
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: StartCause) {
        // On the web the loop runs continuously while the GPU is coming up
        // (PollStrategy::Scheduler), so poll the setup future every
        // iteration until it is ready.
        if self.frost.is_none() {
            self.poll_gpu();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if let Some(frost) = self.frost.as_mut() {
            frost.window_event(event_loop, window_id, event);
            return;
        }
        // The GPU is still coming up: keep the window alive, let Escape and
        // a close request still exit, and keep asking for frames.
        if let WindowEvent::KeyboardInput { event, .. } = &event {
            if event.state == ElementState::Pressed && event.logical_key == NamedKey::Escape {
                log::info!("escape pressed, exiting");
                event_loop.exit();
                return;
            }
        }
        match event {
            WindowEvent::CloseRequested => {
                log::info!("window close requested, exiting");
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
        self.poll_gpu();
    }
}

#[cfg(target_arch = "wasm32")]
impl<P: Process> WebFrost<P> {
    /// Polls the in-flight GPU setup; when it is ready, builds the `Frost`
    /// app and hands it the canvas.
    fn poll_gpu(&mut self) {
        let GpuInit::Init(future) = &mut self.gpu else {
            return; // Idle (before `resumed`) or already failed
        };
        // The event loop itself keeps running between polls (the web
        // ControlFlow::Poll strategy reschedules every frame), so a waker
        // that never wakes is fine: the next iteration polls again.
        let mut cx = TaskContext::from_waker(Waker::noop());
        let outcome = match future.as_mut().poll(&mut cx) {
            Poll::Pending => return, // the loop polls again on the next iteration
            Poll::Ready(outcome) => outcome,
        };

        let (adapter, device, queue) = match outcome {
            Ok(gpu) => gpu,
            Err(message) => {
                // No WebGPU (or the browser refused the device): surface the
                // error on the page instead of a blank canvas.
                self.gpu = GpuInit::Failed;
                log::error!("{message}");
                show_fallback(&message);
                return;
            }
        };
        self.gpu = GpuInit::Idle;

        // Invariant: `resumed` always ran first (winit bootstraps the loop
        // with a `Resumed` event before any poll iteration), so the window
        // and the core were created by then.
        let window = self
            .window
            .take()
            .expect("the window is created in `resumed` before the GPU setup completes");
        let instance = self
            .instance
            .take()
            .expect("the instance is created in `run`");
        let core = self
            .core
            .take()
            .expect("`core` is only taken once, when the GPU setup completes");

        let mut frost = Frost {
            instance,
            adapter,
            device,
            queue,
            window_id: None,
            window: None,
            logical_size: (0, 0),
            scale: 1.0,
            surface: None,
            line_pipeline: None,
            circle_pipeline: None,
            rect_pipeline: None,
            shape_pipeline: None,
            sprite_pipeline: None,
            sprite_resources: HashMap::new(),
            text_atlases: HashMap::new(),
            format: None,
            last_millis: None,
            scene: core.scene,
            keys: HashSet::new(),
            process: core.process,
        };
        frost.attach_window(window);
        self.frost = Some(frost);
    }
}

/// Removes the page's "Loading frost…" placeholder; the first frame has
/// landed.
#[cfg(target_arch = "wasm32")]
fn hide_fallback() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    if let Some(fallback) = document.get_element_by_id("fallback") {
        let _ = fallback.remove();
    }
}

/// Replaces the page's "Loading frost…" placeholder with `message` (in red)
/// so a failed GPU setup is visible instead of a blank canvas.
#[cfg(target_arch = "wasm32")]
fn show_fallback(message: &str) {
    use web_sys::wasm_bindgen::prelude::{JsCast, Upcast};

    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(fallback) = document.get_element_by_id("fallback") else {
        return;
    };
    // `set_text_content` lives on `Node`, so upcast from `Element`.
    let node: &web_sys::Node = fallback.upcast();
    node.set_text_content(Some(message));
    if let Some(fallback) = fallback.dyn_ref::<web_sys::HtmlElement>() {
        let _ = fallback.style().set_property("color", "#e5695e");
    }
}

/// Monotonic milliseconds since an arbitrary process-start epoch:
/// `std::time::Instant` on native, and the browser's `performance.now()`
/// on wasm — `Instant::now()` panics on wasm32-unknown-unknown, where the
/// clock has to come from the page. Only differences between two calls
/// matter (see `Frost::last_millis`), so the epochs may differ.
fn now_millis() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // `Instant` has no absolute epoch, so pin one at the first call;
        // `elapsed` stays monotonic.
        static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let epoch = *EPOCH.get_or_init(std::time::Instant::now);
        epoch.elapsed().as_secs_f64() * 1000.0
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|window| window.performance())
            .map(|performance| performance.now())
            .unwrap_or(0.0)
    }
}

impl<P: Process> Frost<P> {
    /// Sizes the surface from `window`, builds the pipelines and renders
    /// the first frame.
    ///
    /// Called from [`Self::resumed`] once a window exists; on the web the
    /// window is created before the async GPU setup completes, and this runs
    /// when it does.
    fn attach_window(&mut self, window: Arc<Window>) {
        self.window_id = Some(window.id());
        // winit reports the client area in *physical* pixels, so convert it
        // to logical here; `pixel_size()` multiplies by the scale factor.
        let inner = window.inner_size();
        let scale = window.scale_factor();
        self.scale = scale as f32;
        self.logical_size = (
            (inner.width as f64 / scale).max(1.0).round() as u32,
            (inner.height as f64 / scale).max(1.0).round() as u32,
        );
        log::info!(
            "window created ({}x{} @ {:.2}x)",
            self.logical_size.0,
            self.logical_size.1,
            self.scale
        );

        // An `Arc<Window>` is passed by value to `create_surface`, so the
        // resulting `Surface` is 'static without borrowing `self.window`. We
        // keep a clone of the `Arc` for per-frame `request_redraw`.
        let surface = self
            .instance
            .create_surface(window.clone())
            .expect("failed to create wgpu surface");
        // Keep the window for per-frame `request_redraw` (continuous frames).
        self.window = Some(window);
        let pixel_size = self.pixel_size();
        let config = surface
            .get_default_config(&self.adapter, pixel_size.0, pixel_size.1)
            .expect("no compatible surface format");
        surface.configure(&self.device, &config);
        log::info!(
            "surface configured at {}x{}, format {:?}",
            config.width,
            config.height,
            config.format
        );

        self.set_up_pipelines(config.format);
        self.format = Some(config.format);
        self.surface = Some(surface);
        // Commit the first frame immediately so the compositor maps the window.
        self.render();
        // On the web, the page shows a "Loading frost…" placeholder until the
        // first frame lands; remove it now that it has.
        #[cfg(target_arch = "wasm32")]
        hide_fallback();
    }

    fn pixel_size(&self) -> (u32, u32) {
        (
            (self.logical_size.0 as f64 * self.scale as f64).max(1.0) as u32,
            (self.logical_size.1 as f64 * self.scale as f64).max(1.0) as u32,
        )
    }

    fn resize(&mut self) {
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let pixel_size = self.pixel_size();
        let config = surface
            .get_default_config(&self.adapter, pixel_size.0, pixel_size.1)
            .expect("no compatible surface format");
        surface.configure(&self.device, &config);
        log::info!(
            "window resized to {}x{} ({}x{} px)",
            self.logical_size.0,
            self.logical_size.1,
            config.width,
            config.height
        );
        // Pipelines depend only on the surface format, not the size, so they
        // are rebuilt only if the format actually changed.
        if self.format != Some(config.format) {
            self.set_up_pipelines(config.format);
            self.format = Some(config.format);
        }
    }

    /// Creates the shared line and circle pipelines. Uniform buffers and bind
    /// groups are created per draw call, see `primitive_uniform`.
    fn set_up_pipelines(&mut self, format: TextureFormat) {
        let device = &self.device;

        let line_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("line shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(LINE_SHADER)),
        });
        self.line_pipeline = Some(Self::create_pipeline(
            device,
            &line_module,
            format,
            "line pipeline",
        ));

        let circle_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("circle shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(CIRCLE_SHADER)),
        });
        self.circle_pipeline = Some(Self::create_pipeline(
            device,
            &circle_module,
            format,
            "circle pipeline",
        ));

        let rect_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("rectangle shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(RECT_SHADER)),
        });
        self.rect_pipeline = Some(Self::create_pipeline(
            device,
            &rect_module,
            format,
            "rectangle pipeline",
        ));

        let shape_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("shape shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(SHAPE_SHADER)),
        });
        self.shape_pipeline = Some(Self::create_pipeline(
            device,
            &shape_module,
            format,
            "shape pipeline",
        ));

        let sprite_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("sprite shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(SPRITE_SHADER)),
        });
        self.sprite_pipeline = Some(Self::create_pipeline(
            device,
            &sprite_module,
            format,
            "sprite pipeline",
        ));
    }

    /// Builds a render pipeline for a full-screen-triangle shader whose single
    /// uniform is bound at binding 0.
    fn create_pipeline(
        device: &Device,
        module: &ShaderModule,
        format: TextureFormat,
        label: &str,
    ) -> RenderPipeline {
        device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some(label),
            layout: None,
            vertex: VertexState {
                module,
                entry_point: Some("vs_main"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(FragmentState {
                module,
                entry_point: Some("fs_main"),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    /// Creates the uniform buffer for one draw call plus its bind group.
    ///
    /// One buffer per draw call is required: `write_buffer` copies are flushed
    /// as a batch *before any draw executes*, so a buffer shared between draws
    /// would make every draw read the last-written parameters. The buffer and
    /// bind group may be dropped as soon as the command buffer is submitted;
    /// wgpu keeps them alive until the GPU is finished with them.
    fn primitive_uniform(
        &self,
        pipeline: &RenderPipeline,
        label: &str,
        data: &[u8],
    ) -> (Buffer, BindGroup) {
        let device = &self.device;
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: data.len() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, data);
        let layout = pipeline.get_bind_group_layout(0);
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some(label),
            layout: &layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: None,
                }),
            }],
        });
        (buffer, bind_group)
    }

    /// Creates the sprite's texture, view and sampler for one image.
    ///
    /// The pixels arrive as tightly packed RGBA8 bytes (one per texel), so
    /// the upload is a single `write_texture` of `width * height * 4` bytes
    /// with `bytes_per_row = width * 4`. The texture is created in
    /// `Rgba8UnormSrgb` so the sample lands in linear space and the
    /// sRGB blending state produces the same colors the file was authored
    /// in. The texture itself is kept alive by the view: `TextureView`
    /// holds a reference to its texture, so storing the view in
    /// `sprite_resources` is enough.
    fn sprite_texture(&self, data: &[u8], size: [f32; 2]) -> (TextureView, Sampler) {
        let width = (size[0] as u32).max(1);
        let height = (size[1] as u32).max(1);
        let texture = self.device.create_texture(&TextureDescriptor {
            label: Some("sprite texture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        self.queue.write_texture(
            TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            data,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: None,
            },
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let sampler = self.device.create_sampler(&SamplerDescriptor {
            label: Some("sprite sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: f32::MAX,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        });
        (view, sampler)
    }

    /// Creates the uniform buffer and three-entry bind group (uniform,
    /// texture view, sampler) for one sprite draw call. Same per-draw
    /// buffer rationale as [`Frost::primitive_uniform`].
    fn sprite_uniform(
        &self,
        pipeline: &RenderPipeline,
        label: &str,
        view: &TextureView,
        sampler: &Sampler,
        data: &[u8],
    ) -> (Buffer, BindGroup) {
        let device = &self.device;
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: data.len() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, data);
        let layout = pipeline.get_bind_group_layout(0);
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some(label),
            layout: &layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(sampler),
                },
            ],
        });
        (buffer, bind_group)
    }

    fn render(&mut self) {
        let (output, reconfigure) = self.acquire_frame();
        if reconfigure {
            self.resize();
        }
        let Some(output) = output else {
            return;
        };
        log::trace!("render: acquired surface texture, submitting frame");

        // Let the user update the scene and draw this frame, in their
        // coordinate system.
        let mut canvas = Canvas::new(self.pixel_size());
        let now = now_millis();
        let dt = self
            .last_millis
            .map(|last| ((now - last).max(0.0) / 1000.0).min(1.0) as f32)
            .unwrap_or(0.0);
        self.last_millis = Some(now);
        let process = &mut self.process;
        let scene = &mut self.scene;
        let keys = &self.keys;
        {
            let mut ctx = Context {
                canvas: &mut canvas,
                scene,
                keys,
            };
            process.process(&mut ctx, dt);
        }
        // The scene was just updated; draw it into the frame's draw list.
        canvas.draw_scene(&self.scene);
        // Expand the text into per-glyph sprite quads before the sort, so
        // each glyph keeps its node's position in the paint order.
        canvas.expand_text(&mut self.text_atlases);

        // Paint order: ascending z, lower z behind. `sort_by` is stable, so
        // draws with equal z keep call order and the last drawn is on top.
        canvas.draws.sort_by(|a, b| {
            a.z()
                .partial_cmp(&b.z())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let (
            Some(line_pipeline),
            Some(circle_pipeline),
            Some(rect_pipeline),
            Some(shape_pipeline),
            Some(sprite_pipeline),
        ) = (
            self.line_pipeline.as_ref(),
            self.circle_pipeline.as_ref(),
            self.rect_pipeline.as_ref(),
            self.shape_pipeline.as_ref(),
            self.sprite_pipeline.as_ref(),
        ) else {
            return;
        };

        // The frame's clear color: the last background node in call order,
        // or the default when the frame has none.
        let clear = clear_color(&canvas.draws);

        let view = output
            .texture
            .create_view(&TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear.r as f64,
                            g: clear.g as f64,
                            b: clear.b as f64,
                            a: clear.a as f64,
                        }),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // One pass for the whole frame. The background is cleared once
            // for the full surface, then each draw sets the scissor to its
            // tight bounding box, so fragments outside it are discarded and
            // the fragment shader only runs over the pixels the object can
            // write. The viewport is left at the full surface, so the
            // shaders' pixel coordinates stay absolute.
            let render_area = [output.texture.width(), output.texture.height()];
            for draw in canvas.draws {
                let Some([x, y, w, h]) = draw.scissor_rect(render_area) else {
                    // Fully outside the surface, or a background (which
                    // became the clear color); nothing to draw.
                    continue;
                };
                pass.set_scissor_rect(x, y, w, h);
                // A fresh uniform buffer (and bind group) per draw call, since
                // all write_buffer copies complete before any draw executes.
                match draw {
                    Draw::Line {
                        a, b, width, color, ..
                    } => {
                        let (_buffer, bind_group) = self.primitive_uniform(
                            line_pipeline,
                            "line uniforms",
                            &line_uniform_data(a, b, color, width),
                        );
                        pass.set_pipeline(line_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Circle {
                        center,
                        radius,
                        color,
                        ..
                    } => {
                        let (_buffer, bind_group) = self.primitive_uniform(
                            circle_pipeline,
                            "circle uniforms",
                            &circle_uniform_data(center, color, radius),
                        );
                        pass.set_pipeline(circle_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Rectangle {
                        center,
                        extent,
                        color,
                        ..
                    } => {
                        let (_buffer, bind_group) = self.primitive_uniform(
                            rect_pipeline,
                            "rectangle uniforms",
                            &rect_uniform_data(center, extent, color),
                        );
                        pass.set_pipeline(rect_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Shape {
                        world,
                        center,
                        params,
                        kind,
                        aa,
                        color,
                        ..
                    } => {
                        let Some(inv) = world.invert() else {
                            // Degenerate transform; the shape collapses to a
                            // line or a point and its inverse does not exist.
                            continue;
                        };
                        let (_buffer, bind_group) = self.primitive_uniform(
                            shape_pipeline,
                            "shape uniforms",
                            &shape_uniform_data(inv, center, params, kind, aa, color),
                        );
                        pass.set_pipeline(shape_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Sprite {
                        world,
                        data,
                        size,
                        texture_size,
                        tint,
                        alpha,
                        uv_rect,
                        ..
                    } => {
                        let Some(inv) = world.invert() else {
                            // Degenerate transform; the sprite collapses to a
                            // line or a point and its inverse does not exist.
                            continue;
                        };
                        // Look up the GPU resources for this image, creating
                        // them on first use. Two sprites from the same file
                        // share one texture, so the image is uploaded once
                        // per file; glyph quads from the same atlas share
                        // one atlas texture the same way.
                        let key = Arc::as_ptr(&data) as *const ();
                        let (view, sampler) = match self.sprite_resources.get(&key) {
                            Some((view, sampler)) => (view.clone(), sampler.clone()),
                            None => {
                                let (view, sampler) =
                                    self.sprite_texture(&data, [texture_size[0] as f32, texture_size[1] as f32]);
                                self.sprite_resources
                                    .insert(key, (view.clone(), sampler.clone()));
                                (view, sampler)
                            }
                        };
                        let (_buffer, bind_group) = self.sprite_uniform(
                            sprite_pipeline,
                            "sprite uniforms",
                            &view,
                            &sampler,
                            &sprite_uniform_data(inv, size, tint, alpha, uv_rect),
                        );
                        pass.set_pipeline(sprite_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    // A background's scissor rect is `None`, so it continued
                    // above; this arm keeps the match exhaustive.
                    Draw::Background { .. } => {}
                    // Text is expanded into glyph sprites before the render
                    // loop, so it never reaches the match; this arm keeps it
                    // exhaustive.
                    Draw::Text { .. } => {}
                }
            }
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(output);
        log::trace!("render: frame presented");
    }

    /// Acquires the current surface texture.
    ///
    /// Returns the texture and whether the surface should be reconfigured
    /// because it is only suboptimal.
    fn acquire_frame(&self) -> (Option<SurfaceTexture>, bool) {
        let Some(surface) = self.surface.as_ref() else {
            return (None, false);
        };
        match surface.get_current_texture() {
            CurrentSurfaceTexture::Success(tex) => (Some(tex), false),
            CurrentSurfaceTexture::Suboptimal(tex) => {
                log::info!("suboptimal surface texture, reconfiguring");
                (Some(tex), true)
            }
            CurrentSurfaceTexture::Timeout => {
                log::info!("surface timeout, skipping frame");
                (None, false)
            }
            _ => {
                log::info!("no surface texture available, skipping frame");
                (None, false)
            }
        }
    }
}

/// Writes `value` as little-endian f32 bytes into `data` at byte `offset`.
fn write_f32_at(data: &mut [u8], offset: usize, value: f32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Line uniform data, 32 bytes, matching the WGSL uniform-space layout of
/// the `Uniforms` Wgsl struct: `a` @ 0, `b` @ 8, `color` @ 16 (a vec3<f32>
/// is 16-byte aligned in uniform space), `width` @ 28 (the next member is
/// aligned to its own alignment, so the f32 follows the vec3 without a gap);
/// the struct size rounds up to 32.
fn line_uniform_data(a: [f32; 2], b: [f32; 2], color: Color, width: f32) -> Vec<u8> {
    let mut data = vec![0u8; 48];
    write_f32_at(&mut data, 0, a[0]);
    write_f32_at(&mut data, 4, a[1]);
    write_f32_at(&mut data, 8, b[0]);
    write_f32_at(&mut data, 12, b[1]);
    write_f32_at(&mut data, 16, color.r);
    write_f32_at(&mut data, 20, color.g);
    write_f32_at(&mut data, 24, color.b);
    write_f32_at(&mut data, 28, color.a);
    write_f32_at(&mut data, 32, width);
    data
}

/// Circle uniform data, 48 bytes, matching the WGSL uniform-space layout of
/// the `CircleUniforms` Wgsl struct: `center` @ 0, `color` @ 16 (a
/// vec4<f32> is 16-byte aligned in uniform space, leaving an 8-byte gap),
/// `radius` @ 32 (the next member is aligned to its own alignment, so the
/// f32 follows the vec4 without a gap); the struct size rounds up to 48.
fn circle_uniform_data(center: [f32; 2], color: Color, radius: f32) -> Vec<u8> {
    let mut data = vec![0u8; 48];
    write_f32_at(&mut data, 0, center[0]);
    write_f32_at(&mut data, 4, center[1]);
    write_f32_at(&mut data, 16, color.r);
    write_f32_at(&mut data, 20, color.g);
    write_f32_at(&mut data, 24, color.b);
    write_f32_at(&mut data, 28, color.a);
    write_f32_at(&mut data, 32, radius);
    data
}

/// Rectangle uniform data, matching the `RectUniforms` Wgsl struct: center,
/// extent, color. The 32 bytes of data exactly fill the struct's 32-byte
/// size, so no padding is needed.
fn rect_uniform_data(center: [f32; 2], extent: [f32; 2], color: Color) -> Vec<u8> {
    let mut data: Vec<u8> = center
        .iter()
        .chain(extent.iter())
        .chain(color.channels().iter())
        .flat_map(|f| f.to_le_bytes())
        .collect();
    data.resize(32, 0);
    data
}

/// Shape uniform data, 80 bytes, matching the WGSL uniform-space layout of
/// the `ShapeUniforms` Wgsl struct. Per the WGSL memory layout rules (and
/// naga's `Layouter`), in the uniform address space a mat2x2<f32> is 16
/// bytes total with 8-byte alignment, so its two vec2 columns are packed at
/// @ 0 and @ 8; a vec3<f32> is 12 bytes with 16-byte alignment; a vec2<f32>
/// is 8 bytes with 8-byte alignment. With that rule, the struct lays out as:
/// `to_local` @ 0 (column 0 = (m[0][0], m[1][0]) at @ 0, column 1 =
/// (m[0][1], m[1][1]) at @ 8; stored column-major, so the WGSL matrix holds
/// the Rust matrix element-for-element and `m * p + t` in the shader
/// reproduces `Transform::apply`), `translation` @ 16, `center` @ 24,
/// `params` @ 32, `color` @ 48 (a vec4<f32>, spanning 48..64), `misc` @ 64
/// (aa @ 64, kind @ 68); the struct size rounds up to 80.
fn shape_uniform_data(
    to_local: Transform,
    center: [f32; 2],
    params: [f32; 2],
    kind: f32,
    aa: f32,
    color: Color,
) -> Vec<u8> {
    let mut data = vec![0u8; 80];
    let m = to_local.m;
    // mat2x2: 16 bytes total, vec2 columns with an 8-byte stride.
    write_f32_at(&mut data, 0, m[0][0]);
    write_f32_at(&mut data, 4, m[1][0]);
    write_f32_at(&mut data, 8, m[0][1]);
    write_f32_at(&mut data, 12, m[1][1]);
    write_f32_at(&mut data, 16, to_local.t[0]);
    write_f32_at(&mut data, 20, to_local.t[1]);
    write_f32_at(&mut data, 24, center[0]);
    write_f32_at(&mut data, 28, center[1]);
    write_f32_at(&mut data, 32, params[0]);
    write_f32_at(&mut data, 36, params[1]);
    // vec4: 16 bytes with 16-byte alignment, so it starts at 48 and spans
    // 48..64.
    write_f32_at(&mut data, 48, color.r);
    write_f32_at(&mut data, 52, color.g);
    write_f32_at(&mut data, 56, color.b);
    write_f32_at(&mut data, 60, color.a);
    write_f32_at(&mut data, 64, aa);
    write_f32_at(&mut data, 68, kind);
    data
}

/// Sprite uniform data, 80 bytes, matching the WGSL uniform-space layout of
/// the `SpriteUniforms` Wgsl struct. The mat2x2 column packing is the same
/// as in [`shape_uniform_data`]; `translation` sits at @ 16, `size` (a
/// vec2) at @ 24, the tint `vec4` at @ 32 (16 bytes, 16-byte aligned,
/// spanning 32..48), the scalar `alpha` (4-byte aligned) at @ 48, and the
/// `uv_rect` `vec4` (16-byte aligned) at @ 64; the struct size is 80.
fn sprite_uniform_data(
    to_local: Transform,
    size: [f32; 2],
    tint: Color,
    alpha: f32,
    uv_rect: [f32; 4],
) -> Vec<u8> {
    let mut data = vec![0u8; 80];
    let m = to_local.m;
    // mat2x2: 16 bytes total, vec2 columns with an 8-byte stride.
    write_f32_at(&mut data, 0, m[0][0]);
    write_f32_at(&mut data, 4, m[1][0]);
    write_f32_at(&mut data, 8, m[0][1]);
    write_f32_at(&mut data, 12, m[1][1]);
    write_f32_at(&mut data, 16, to_local.t[0]);
    write_f32_at(&mut data, 20, to_local.t[1]);
    write_f32_at(&mut data, 24, size[0]);
    write_f32_at(&mut data, 28, size[1]);
    // vec4: 16 bytes with 16-byte alignment, so it starts at 32 and spans
    // 32..48. The scalar alpha is 4-byte aligned, so it follows the tint at
    // 48, and the 16-byte-aligned uv_rect starts at 64.
    write_f32_at(&mut data, 32, tint.r);
    write_f32_at(&mut data, 36, tint.g);
    write_f32_at(&mut data, 40, tint.b);
    write_f32_at(&mut data, 44, tint.a);
    write_f32_at(&mut data, 48, alpha);
    write_f32_at(&mut data, 64, uv_rect[0]);
    write_f32_at(&mut data, 68, uv_rect[1]);
    write_f32_at(&mut data, 72, uv_rect[2]);
    write_f32_at(&mut data, 76, uv_rect[3]);
    data
}

/// Drives `future` on this thread until it completes.
///
/// Native only: in the browser the main thread cannot block on a future,
/// because the browser only resolves it once the thread is free. See
/// [`run`].
#[cfg(not(target_arch = "wasm32"))]
fn block_on<F: Future>(future: F) -> F::Output {
    let mut cx = TaskContext::from_waker(Waker::noop());
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn black() -> Color {
        Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }
    }

    #[test]
    fn line_scissor_includes_width_and_aa_band() {
        let draw = Draw::Line {
            a: [10.0, 20.0],
            b: [30.0, 40.0],
            width: 2.0,
            color: black(),
            z: 0.0,
        };
        // pad = width/2 + AA_BAND = 1.75
        assert_eq!(draw.scissor_rect([100, 100]), Some([8, 18, 24, 24]));
    }

    #[test]
    fn circle_scissor_includes_aa_band() {
        let draw = Draw::Circle {
            center: [50.0, 50.0],
            radius: 10.0,
            color: black(),
            z: 0.0,
        };
        assert_eq!(draw.scissor_rect([100, 100]), Some([39, 39, 22, 22]));
    }

    #[test]
    fn rectangle_scissor_includes_aa_band() {
        let draw = Draw::Rectangle {
            center: [50.0, 50.0],
            extent: [10.0, 20.0],
            color: black(),
            z: 0.0,
        };
        assert_eq!(draw.scissor_rect([100, 100]), Some([39, 29, 22, 42]));
    }

    #[test]
    fn off_screen_draw_has_no_scissor() {
        let draw = Draw::Circle {
            center: [-200.0, -200.0],
            radius: 10.0,
            color: black(),
            z: 0.0,
        };
        assert_eq!(draw.scissor_rect([100, 100]), None);
    }

    #[test]
    fn scissor_is_clamped_to_the_render_area() {
        let draw = Draw::Line {
            a: [-50.0, 0.0],
            b: [50.0, 0.0],
            width: 0.0,
            color: black(),
            z: 0.0,
        };
        assert_eq!(draw.scissor_rect([100, 100]), Some([0, 0, 51, 1]));
    }

    #[test]
    fn transform_compose_applies_self_then_other() {
        let rot = Transform::rotate(std::f32::consts::FRAC_PI_2);
        let tr = Transform::translate(10.0, 0.0);
        let world = rot.compose(&tr);
        // (1, 0) rotates to (0, 1), then translates to (10, 1).
        let p = world.apply([1.0, 0.0]);
        assert!((p[0] - 10.0).abs() < 1e-4);
        assert!((p[1] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn transform_compose_respects_order() {
        // Rotation and translation do not commute, so this pins down which
        // order the composition applies them in.
        let rot = Transform::rotate(std::f32::consts::FRAC_PI_2);
        let tr = Transform::translate(10.0, 0.0);
        // Translate first, then rotate: (1, 0) -> (11, 0) -> (0, 11).
        let world = tr.compose(&rot);
        let p = world.apply([1.0, 0.0]);
        assert!(p[0].abs() < 1e-4);
        assert!((p[1] - 11.0).abs() < 1e-4);
        // The reverse composition: (1, 0) -> (0, 1) -> (10, 1).
        let world = rot.compose(&tr);
        let p = world.apply([1.0, 0.0]);
        assert!((p[0] - 10.0).abs() < 1e-4);
        assert!((p[1] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn transform_inverse_round_trips() {
        let world = Transform::translate(3.0, -4.0)
            .compose(&Transform::rotate(0.7))
            .compose(&Transform::scale(2.0, 0.5));
        let Some(inv) = world.invert() else {
            panic!("expected an inverse");
        };
        for p in [[1.2, -3.4], [-7.0, 2.0], [0.0, 0.0]] {
            let back = world.apply(inv.apply(p));
            assert!((back[0] - p[0]).abs() < 1e-3);
            assert!((back[1] - p[1]).abs() < 1e-3);
        }
    }

    #[test]
    fn degenerate_transform_has_no_inverse() {
        assert!(Transform::scale(0.0, 1.0).invert().is_none());
        assert!(Transform::identity().invert().is_some());
    }

    #[test]
    fn transform_scales_are_the_column_norms() {
        // Scale first, then rotate: the stretch along each axis is exactly
        // the scale factors, no matter the rotation.
        let world = Transform::scale(2.0, 3.0).compose(&Transform::rotate(0.5));
        let [sx, sy] = world.scales();
        assert!((sx - 2.0).abs() < 1e-4);
        assert!((sy - 3.0).abs() < 1e-4);
    }

    #[test]
    fn draw_scene_composes_transforms_down_the_tree() {
        let mut canvas = Canvas::new((100, 100));
        let scene = Scene::new(SceneNode {
            transform: Transform::translate(10.0, 0.0),
            scale: [1.0, 1.0],
            modulate: WHITE,
            order: 0.0,
            shape: Some(Shape::Circle {
                center: [0.0, 0.0],
                radius: 5.0,
                color: black(),
            }),
            children: vec![Box::new(SceneNode {
                transform: Transform::scale_uniform(2.0),
                scale: [1.0, 1.0],
                modulate: WHITE,
                order: 0.0,
                shape: None,
                children: vec![Box::new(SceneNode {
                    transform: Transform::identity(),
                    scale: [1.0, 1.0],
                    modulate: WHITE,
                    order: 0.0,
                    shape: Some(Shape::Circle {
                        center: [1.0, 1.0],
                        radius: 1.0,
                        color: black(),
                    }),
                    children: vec![],
                })],
            })],
        });
        canvas.draw_scene(&scene);
        let [Draw::Shape {
            world: w0,
            center: c0,
            aa: aa0,
            ..
        }, Draw::Shape {
            world: w1,
            center: c1,
            aa: aa1,
            ..
        }] = &canvas.draws[..]
        else {
            panic!("expected two shape draws");
        };
        // The root circle translates to (10, 0) in user space, which is
        // (60, 50) in pixel space on a 100x100 window (x + w/2, h/2 - y).
        assert_eq!(w0.apply(*c0), [60.0, 50.0]);
        assert!((*aa0 - AA_BAND).abs() < 1e-6);
        // The grandchild's own scale applies first, then the root's
        // translate: (1, 1) -> (2, 2) -> (12, 2) in user space, (62, 48) in
        // pixel space, and the local AA band halves to stay 0.75 screen
        // pixels.
        assert_eq!(w1.apply(*c1), [62.0, 48.0]);
        assert!((*aa1 - AA_BAND / 2.0).abs() < 1e-6);
    }

    #[test]
    fn draw_scene_rotates_a_translated_child() {
        // A translated child of a rotated parent (an orbiting shape): the
        // parent's rotation must rotate the child's translation, not the
        // other way around.
        let mut canvas = Canvas::new((100, 100));
        let scene = Scene::new(SceneNode {
            transform: Transform::rotate(std::f32::consts::FRAC_PI_2),
            scale: [1.0, 1.0],
            modulate: WHITE,
            order: 0.0,
            shape: None,
            children: vec![Box::new(SceneNode {
                transform: Transform::translate(10.0, 0.0),
                scale: [1.0, 1.0],
                modulate: WHITE,
                order: 0.0,
                shape: Some(Shape::Circle {
                    center: [0.0, 0.0],
                    radius: 1.0,
                    color: black(),
                }),
                children: vec![],
            })],
        });
        canvas.draw_scene(&scene);
        let [Draw::Shape { world: w, center: c, .. }] = &canvas.draws[..] else {
            panic!("expected one shape draw");
        };
        // (0, 0) translates to (10, 0) and rotates to (0, 10) in user space,
        // which is (50, 40) in pixel space on a 100x100 window.
        assert_eq!(w.apply(*c), [50.0, 40.0]);
    }

    #[test]
    fn scene_shape_at_user_origin_lands_at_window_center() {
        // Regression: a scene shape at the user-space origin must land at
        // the window center, not the top-left pixel corner.
        let mut canvas = Canvas::new((100, 100));
        let scene = Scene::new(SceneNode {
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: WHITE,
            order: 0.0,
            shape: Some(Shape::Circle {
                center: [0.0, 0.0],
                radius: 10.0,
                color: black(),
            }),
            children: vec![],
        });
        canvas.draw_scene(&scene);
        let [Draw::Shape { world: w, center: c, .. }] = &canvas.draws[..] else {
            panic!("expected one shape draw");
        };
        assert_eq!(w.apply(*c), [50.0, 50.0]);
    }

    #[test]
    fn context_reports_held_keys() {
        let mut canvas = Canvas::new((100, 100));
        let mut scene = Scene::default();
        let mut keys = HashSet::new();
        {
            let ctx = Context {
                canvas: &mut canvas,
                scene: &mut scene,
                keys: &keys,
            };
            assert!(!ctx.key_down(KeyCode::KeyW));
        }
        keys.insert(KeyCode::KeyW);
        let ctx = Context {
            canvas: &mut canvas,
            scene: &mut scene,
            keys: &keys,
        };
        assert!(ctx.key_down(KeyCode::KeyW));
        assert!(!ctx.key_down(KeyCode::KeyA));
    }

    #[test]
    fn shape_scissor_under_non_uniform_scale() {
        let draw = Draw::Shape {
            world: Transform::scale(2.0, 1.0),
            center: [25.0, 50.0],
            params: [10.0, 0.0],
            kind: 0.0,
            aa: 0.75 / 2.0,
            color: black(),
            z: 0.0,
        };
        // The local box (25 ± 10.375, 50 ± 10.375) stretches to
        // x: [29.25, 70.75] and y: [39.625, 60.375].
        assert_eq!(draw.scissor_rect([100, 100]), Some([29, 39, 42, 22]));
    }

    #[test]
    fn shape_scissor_under_rotation() {
        // The center is chosen so that the 45° rotation (about the origin)
        // maps it onto (50, 50).
        let center = [50.0 * std::f32::consts::SQRT_2, 0.0];
        let draw = Draw::Shape {
            world: Transform::rotate(std::f32::consts::FRAC_PI_4),
            center,
            params: [10.0, 0.0],
            kind: 0.0,
            aa: 0.75,
            color: black(),
            z: 0.0,
        };
        // The ±10.75 box rotated 45° has half-extent 10.75 * sqrt(2), so the
        // axis-aligned box is [34.797, 65.203] on both axes.
        assert_eq!(draw.scissor_rect([100, 100]), Some([34, 34, 32, 32]));
    }

    #[test]
    fn off_screen_shape_has_no_scissor() {
        let draw = Draw::Shape {
            world: Transform::identity(),
            center: [-200.0, -200.0],
            params: [10.0, 0.0],
            kind: 0.0,
            aa: 0.75,
            color: black(),
            z: 0.0,
        };
        assert_eq!(draw.scissor_rect([100, 100]), None);
    }

    #[test]
    fn shape_uniform_bytes_follow_the_wgsl_layout() {
        // Lock the byte layout of `shape_uniform_data` to the WGSL
        // uniform-space layout of `ShapeUniforms`, so a reorder of the Wgsl
        // struct is caught here. Distinct values make any offset swap
        // visible. Layout per the WGSL memory layout rules (naga's
        // `Layouter`): mat2x2<f32> is 16 bytes total with 8-byte alignment
        // (vec2 columns, stride 8), so `to_local` spans 0..16 with column 0
        // at 0 and column 1 at 8; `translation` @ 16; `center` @ 24;
        // `params` @ 32; vec4<f32> (16 bytes, 16-byte aligned) `color` @ 48
        // (spanning 48..64); `misc` vec2 @ 64; struct span rounds up to 80.
        let inv = Transform::translate(1.5, -2.5).invert().unwrap();
        let data = shape_uniform_data(inv, [7.0, 8.0], [9.0, 10.0], 1.0, 0.25, Color {
            r: 0.1,
            g: 0.2,
            b: 0.3,
            a: 0.4,
        });
        assert_eq!(data.len(), 80);

        let f32_at = |off: usize| {
            f32::from_le_bytes(data[off..off + 4].try_into().unwrap())
        };
        // to_local is the identity matrix (inverting a pure translation keeps
        // the matrix identity), stored column-major: column 0 = (1, 0) at @ 0,
        // column 1 = (0, 1) at @ 8.
        assert_eq!(f32_at(0), 1.0);
        assert_eq!(f32_at(4), 0.0);
        assert_eq!(f32_at(8), 0.0);
        assert_eq!(f32_at(12), 1.0);
        // The inverse of translate(1.5, -2.5) is translate(-1.5, 2.5).
        assert_eq!(f32_at(16), -1.5);
        assert_eq!(f32_at(20), 2.5);
        assert_eq!(f32_at(24), 7.0);
        assert_eq!(f32_at(28), 8.0);
        assert_eq!(f32_at(32), 9.0);
        assert_eq!(f32_at(36), 10.0);
        // `params` ends at 40; the 16-byte-aligned vec4 color starts at 48,
        // so bytes 40..48 are padding (zeroed by the `vec![0u8; 80]` init).
        assert_eq!(f32_at(40), 0.0);
        assert_eq!(f32_at(48), 0.1);
        assert_eq!(f32_at(52), 0.2);
        assert_eq!(f32_at(56), 0.3);
        // The vec4's alpha channel follows its RGB channels at byte 60, and
        // the `misc` vec2 begins at byte 64: `aa` at 64, `kind` at 68;
        // 72..80 is the struct's alignment padding.
        assert_eq!(f32_at(60), 0.4);
        assert_eq!(f32_at(64), 0.25);
        assert_eq!(f32_at(68), 1.0);
        assert_eq!(f32_at(72), 0.0);
        assert_eq!(data.len(), 80);
    }

    #[test]
    fn clear_color_falls_back_to_the_default_without_a_background() {
        let draws = [Draw::Circle {
            center: [0.0, 0.0],
            radius: 5.0,
            color: black(),
            z: 0.0,
        }];
        assert_eq!(clear_color(&draws), DEFAULT_BACKGROUND);
    }

    #[test]
    fn clear_color_is_the_background_when_the_frame_has_one() {
        let indigo = Color {
            r: 0.09,
            g: 0.06,
            b: 0.16,
            a: 1.0,
        };
        let draws = [
            Draw::Background { color: indigo },
            Draw::Circle {
                center: [0.0, 0.0],
                radius: 5.0,
                color: black(),
                z: 0.0,
            },
        ];
        assert_eq!(clear_color(&draws), indigo);
    }

    #[test]
    fn clear_color_is_the_last_background_in_call_order() {
        let first = Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        };
        let second = Color {
            r: 0.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
        };
        let draws = [
            Draw::Background { color: first },
            Draw::Background { color: second },
        ];
        assert_eq!(clear_color(&draws), second);
    }

    #[test]
    fn background_node_sorts_to_the_back_and_keeps_its_color() {
        let mut canvas = Canvas::new((100, 100));
        canvas.draw_scene(&Scene::new(SceneNode {
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: WHITE,
            order: 0.0,
            shape: Some(Shape::Rectangle {
                center: [0.0, 0.0],
                extent: [10.0, 10.0],
                color: black(),
            }),
            children: vec![Box::new(SceneNode {
                // Transformed on purpose: the background must ignore it.
                transform: Transform::translate(50.0, 50.0),
                scale: [1.0, 1.0],
                modulate: WHITE,
                order: 0.0,
                shape: Some(Shape::Background {
                    color: Color {
                        r: 0.0,
                        g: 0.0,
                        b: 1.0,
                        a: 1.0,
                    },
                }),
                children: vec![],
            })],
        }));
        canvas
            .draws
            .sort_by(|a, b| a.z().partial_cmp(&b.z()).unwrap_or(std::cmp::Ordering::Equal));
        let [Draw::Background { color }, Draw::Shape { .. }] = &canvas.draws[..] else {
            panic!("expected one background and one shape, got {:?}", canvas.draws);
        };
        assert_eq!(
            *color,
            Color {
                r: 0.0,
                g: 0.0,
                b: 1.0,
                a: 1.0
            }
        );
    }

    #[test]
    fn node_scale_applies_to_the_shape_and_its_subtree() {
        let mut canvas = Canvas::new((100, 100));
        let scene = Scene::new(SceneNode {
            // Scale 2x in x only; the transform still positions the node.
            transform: Transform::translate(10.0, 0.0),
            scale: [2.0, 1.0],
            modulate: WHITE,
            order: 0.0,
            shape: Some(Shape::Circle {
                center: [1.0, 1.0],
                radius: 1.0,
                color: black(),
            }),
            children: vec![Box::new(SceneNode {
                transform: Transform::translate(1.0, 0.0),
                scale: [1.0, 1.0],
                modulate: WHITE,
                order: 0.0,
                shape: Some(Shape::Circle {
                    center: [0.0, 0.0],
                    radius: 1.0,
                    color: black(),
                }),
                children: vec![],
            })],
        });
        canvas.draw_scene(&scene);
        let [
            Draw::Shape {
                world: w0,
                center: c0,
                aa: aa0,
                ..
            },
            Draw::Shape {
                world: w1,
                center: c1,
                ..
            },
        ] = &canvas.draws[..]
        else {
            panic!("expected two shape draws");
        };
        // The node's scale applies before its transform: (1, 1) -> (2, 1)
        // -> (12, 1) in user space, (62, 49) in pixel space.
        assert_eq!(w0.apply(*c0), [62.0, 49.0]);
        // The child's (1, 0) translation is scaled by the parent's 2x:
        // (0, 0) -> (1, 0) -> (2, 0) -> (12, 0) in user space, (62, 50) in
        // pixel space.
        assert_eq!(w1.apply(*c1), [62.0, 50.0]);
        // The AA band compensates for the 2x world scale.
        assert!((aa0 - AA_BAND / 2.0).abs() < 1e-6);
    }

    #[test]
    fn node_order_accumulates_down_the_tree_like_the_transform() {
        // A chain of nodes with orders 1, 2, 4: each node's own shape draws
        // at the order inherited from its ancestors plus its own, and the
        // children inherit that total, so the z's are 1, 3, 7.
        let mut canvas = Canvas::new((100, 100));
        let scene = Scene::new(SceneNode {
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: WHITE,
            order: 1.0,
            shape: Some(Shape::Circle {
                center: [0.0, 0.0],
                radius: 1.0,
                color: black(),
            }),
            children: vec![Box::new(SceneNode {
                transform: Transform::identity(),
                scale: [1.0, 1.0],
                modulate: WHITE,
                order: 2.0,
                shape: Some(Shape::Circle {
                    center: [0.0, 0.0],
                    radius: 1.0,
                    color: black(),
                }),
                children: vec![Box::new(SceneNode {
                    transform: Transform::identity(),
                    scale: [1.0, 1.0],
                    modulate: WHITE,
                    order: 4.0,
                    shape: Some(Shape::Circle {
                        center: [0.0, 0.0],
                        radius: 1.0,
                        color: black(),
                    }),
                    children: vec![],
                })],
            })],
        });
        canvas.draw_scene(&scene);
        let zs = canvas
            .draws
            .iter()
            .map(|draw| draw.z())
            .collect::<Vec<_>>();
        assert_eq!(zs, vec![1.0, 3.0, 7.0]);
    }

    #[test]
    fn node_modulate_multiplies_into_the_shape_and_its_subtree() {
        // A chain of nodes: the root modulates red to half and blue out,
        // the child modulates green to half and opacity to half. Each node's
        // own shape color is the channel-wise product of the modulates on
        // the path from the root, accumulated like the order.
        let white = Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };
        let mut canvas = Canvas::new((100, 100));
        let scene = Scene::new(SceneNode {
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: Color {
                r: 0.5,
                g: 1.0,
                b: 0.0,
                a: 1.0,
            },
            order: 0.0,
            shape: Some(Shape::Circle {
                center: [0.0, 0.0],
                radius: 1.0,
                color: white,
            }),
            children: vec![Box::new(SceneNode {
                transform: Transform::identity(),
                scale: [1.0, 1.0],
                modulate: Color {
                    r: 1.0,
                    g: 0.5,
                    b: 1.0,
                    a: 0.5,
                },
                order: 0.0,
                shape: Some(Shape::Circle {
                    center: [0.0, 0.0],
                    radius: 1.0,
                    color: white,
                }),
                children: vec![],
            })],
        });
        canvas.draw_scene(&scene);
        let [
            Draw::Shape { color: c0, .. },
            Draw::Shape { color: c1, .. },
        ] = &canvas.draws[..]
        else {
            panic!("expected two shape draws");
        };
        // The root's own shape is white times its own modulate.
        assert_eq!(*c0, Color { r: 0.5, g: 1.0, b: 0.0, a: 1.0 });
        // The child inherits the root's modulate multiplied by its own:
        // blue stays zeroed by the root, and the child's half opacity
        // multiplies into the alpha channel.
        assert_eq!(*c1, Color { r: 0.5, g: 0.5, b: 0.0, a: 0.5 });
    }

    #[test]
    fn node_modulate_multiplies_sprite_tint_and_text_color_not_their_alpha() {
        // The modulate multiplies the sprite's tint and the text's color,
        // channel by channel, but leaves the separate `alpha` opacity field
        // untouched: the opacity is not a color.
        let mut canvas = Canvas::new((100, 100));
        canvas.draw_scene(&Scene::new(SceneNode {
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: Color {
                r: 0.5,
                g: 1.0,
                b: 1.0,
                a: 0.5,
            },
            order: 0.0,
            shape: Some(Shape::Sprite {
                data: Arc::new([0u8; 16]),
                width: 4,
                height: 4,
                color: Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                alpha: 0.25,
            }),
            children: vec![],
        }));
        canvas.draw_scene(&Scene::new(SceneNode {
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: Color {
                r: 0.25,
                g: 0.5,
                b: 1.0,
                a: 1.0,
            },
            order: 0.0,
            shape: Some(Shape::Text {
                text: "hi".to_string(),
                font: Arc::new([0u8]),
                size: 12.0,
                color: Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                alpha: 0.75,
            }),
            children: vec![],
        }));
        let [
            Draw::Sprite { tint, alpha, .. },
            Draw::Text { color, alpha: t_alpha, .. },
        ] = &canvas.draws[..]
        else {
            panic!("expected a sprite and a text draw");
        };
        assert_eq!(*tint, Color { r: 0.5, g: 1.0, b: 1.0, a: 0.5 });
        assert_eq!(*alpha, 0.25);
        assert_eq!(*color, Color { r: 0.25, g: 0.5, b: 1.0, a: 1.0 });
        assert_eq!(*t_alpha, 0.75);
    }

    #[test]
    fn scene_sprite_at_user_origin_lands_at_window_center() {
        // A sprite node at the user-space origin must be centered on the
        // window center (not offset to a corner), and its texture size, tint
        // and z must travel onto the draw untouched.
        let mut canvas = Canvas::new((100, 100));
        let scene = Scene::new(SceneNode {
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: WHITE,
            order: 0.0,
            shape: Some(Shape::Sprite {
                data: Arc::new([0u8; 16]),
                width: 4,
                height: 4,
                color: black(),
                alpha: 1.0,
            }),
            children: vec![],
        });
        canvas.draw_scene(&scene);
        let [Draw::Sprite {
            world,
            data,
            size,
            texture_size,
            aa,
            tint,
            alpha,
            uv_rect,
            z,
        }] = &canvas.draws[..]
        else {
            panic!("expected one sprite draw");
        };
        // The sprite's local space is centered on the origin.
        assert_eq!(world.apply([0.0, 0.0]), [50.0, 50.0]);
        assert_eq!(*size, [4.0, 4.0]);
        assert_eq!(*texture_size, [4, 4]);
        assert_eq!(data.len(), 16);
        assert!((*aa - AA_BAND).abs() < 1e-6);
        assert_eq!(*tint, black());
        assert_eq!(*alpha, 1.0);
        assert_eq!(*uv_rect, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(*z, 0.0);
    }

    #[test]
    fn sprite_scissor_is_the_texture_box_plus_aa_band() {
        let draw = Draw::Sprite {
            world: Transform::translate(50.0, 50.0),
            data: Arc::new([0u8; 16]),
            size: [40.0, 20.0],
            texture_size: [40, 20],
            aa: 0.75,
            tint: black(),
            alpha: 1.0,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            z: 0.0,
        };
        // The box is (50 ± 20.75, 50 ± 10.75), i.e. [29.25, 70.75] on x and
        // [39.25, 60.75] on y.
        assert_eq!(draw.scissor_rect([100, 100]), Some([29, 39, 42, 22]));
    }

    #[test]
    fn sprite_scissor_under_rotation() {
        // A 45° rotation about the sprite's own center: the local box
        // (±20.75, ±10.75) rotates to an axis-aligned box with half-extent
        // (20.75 + 10.75) / sqrt(2) = 22.274 on both axes, centered on
        // (50, 50).
        let draw = Draw::Sprite {
            world: Transform::rotate(std::f32::consts::FRAC_PI_4)
                .compose(&Transform::translate(50.0, 50.0)),
            data: Arc::new([0u8; 16]),
            size: [40.0, 20.0],
            texture_size: [40, 20],
            aa: 0.75,
            tint: black(),
            alpha: 1.0,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            z: 0.0,
        };
        assert_eq!(draw.scissor_rect([100, 100]), Some([27, 27, 46, 46]));
    }

    #[test]
    fn sprite_uniform_bytes_follow_the_wgsl_layout() {
        // Lock the byte layout of `sprite_uniform_data` to the WGSL
        // uniform-space layout of `SpriteUniforms`, the same way the shape
        // test does: the mat2x2 `<f32>` spans 0..16 (column 0 @ 0, column 1
        // @ 8), `translation` @ 16, `size` @ 24, the tint vec4 (16 bytes,
        // 16-byte aligned) @ 32 spanning 32..48, the scalar alpha @ 48, and
        // the `uv_rect` vec4 (16-byte aligned) @ 64 spanning 64..80; the
        // struct size is 80.
        let inv = Transform::translate(1.5, -2.5).invert().unwrap();
        let data = sprite_uniform_data(
            inv,
            [12.0, 34.0],
            Color {
                r: 0.5,
                g: 0.25,
                b: 0.125,
                a: 0.3,
            },
            0.75,
            [0.25, 0.5, 0.75, 1.0],
        );
        assert_eq!(data.len(), 80);

        let f32_at = |off: usize| {
            f32::from_le_bytes(data[off..off + 4].try_into().unwrap())
        };
        // `to_local` is the identity matrix (inverting a pure translation
        // leaves the matrix identity), stored column-major.
        assert_eq!(f32_at(0), 1.0);
        assert_eq!(f32_at(4), 0.0);
        assert_eq!(f32_at(8), 0.0);
        assert_eq!(f32_at(12), 1.0);
        // The inverse of translate(1.5, -2.5) is translate(-1.5, 2.5).
        assert_eq!(f32_at(16), -1.5);
        assert_eq!(f32_at(20), 2.5);
        assert_eq!(f32_at(24), 12.0);
        assert_eq!(f32_at(28), 34.0);
        assert_eq!(f32_at(32), 0.5);
        assert_eq!(f32_at(36), 0.25);
        assert_eq!(f32_at(40), 0.125);
        // The tint's alpha channel follows its RGB channels at byte 44.
        assert_eq!(f32_at(44), 0.3);
        // The scalar alpha is 4-byte aligned, so it occupies the 48..52 slot
        // right after the tint.
        assert_eq!(f32_at(48), 0.75);
        // Bytes 52..64 are the struct's alignment padding (zeroed by the
        // `vec![0u8; 80]` init), so the 16-byte-aligned uv_rect starts at 64;
        // a whole-texture sprite passes the identity rect.
        assert_eq!(f32_at(52), 0.0);
        assert_eq!(f32_at(56), 0.0);
        assert_eq!(f32_at(64), 0.25);
        assert_eq!(f32_at(68), 0.5);
        assert_eq!(f32_at(72), 0.75);
        assert_eq!(f32_at(76), 1.0);
    }

    /// The font file used by the expand_text tests, loaded from the crate's
    /// asset directory.
    fn test_font() -> Arc<[u8]> {
        Arc::from(
            std::fs::read(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/assets/fonts/JameGem08_2026-Regular.ttf"
            ))
            .expect("test font should be readable"),
        )
    }

    #[test]
    fn expand_text_splices_glyph_sprites_in_place() {
        // A text draw between two circle draws must be replaced by its
        // glyph sprites without disturbing the neighbours' positions: the
        // circles keep their slots and the sprites inherit the text's tint,
        // alpha and z.
        let font = test_font();
        let size = 48.0;
        let mut canvas = Canvas::new((800, 600));
        let tint = Color { r: 1.0, g: 0.5, b: 0.25, a: 1.0 };
        canvas.draws = vec![
            Draw::Circle {
                center: [-100.0, 0.0],
                radius: 10.0,
                color: black(),
                z: 0.0,
            },
            Draw::Text {
                world: Transform::identity(),
                font: font.clone(),
                text: "hi".to_string(),
                size,
                color: tint,
                alpha: 0.9,
                z: 1.0,
            },
            Draw::Circle {
                center: [100.0, 0.0],
                radius: 10.0,
                color: black(),
                z: 2.0,
            },
        ];
        let mut atlases: HashMap<(u64, u32), text::Atlas> = HashMap::new();
        canvas.expand_text(&mut atlases);

        // The layout's glyph count is the source of truth for how many
        // sprites the text should produce (every glyph of "hi" has ink).
        let layout =
            text::layout(&font, "hi", size).expect("the test font should shape 'hi'");
        assert_eq!(canvas.draws.len(), 2 + layout.glyphs.len());
        let [Draw::Circle { z: z0, .. }, .., Draw::Circle { z: z1, .. }] =
            &canvas.draws[..]
        else {
            panic!("the circles must keep their slots");
        };
        assert_eq!(*z0, 0.0);
        assert_eq!(*z1, 2.0);
        for draw in canvas.draws.iter().skip(1).take(layout.glyphs.len()) {
            let Draw::Sprite { size, texture_size, uv_rect, tint, alpha, z, .. } = draw
            else {
                panic!("every spliced draw must be a sprite");
            };
            assert_eq!(*tint, Color { r: 1.0, g: 0.5, b: 0.25, a: 1.0 });
            assert_eq!(*alpha, 0.9);
            assert_eq!(*z, 1.0);
            // Glyph quads are a sub-rectangle of the 512x512 atlas.
            assert_eq!(*texture_size, [512, 512]);
            assert!(size[0] > 0.0 && size[1] > 0.0);
            assert!(uv_rect[0] < uv_rect[2]);
            assert!(uv_rect[1] < uv_rect[3]);
        }
    }

    #[test]
    fn expand_text_reuses_the_atlas_across_frames() {
        // A second frame with the same font, text and size must not
        // re-rasterize: the atlas data Arc stays the same pointer, so the
        // GPU texture is reused and no bytes are re-uploaded.
        let font = test_font();
        let size: f32 = 48.0;
        let mut atlases: HashMap<(u64, u32), text::Atlas> = HashMap::new();
        let key = (Arc::as_ptr(&font) as *const () as u64, size.to_bits());

        let mut frame = || {
            let mut canvas = Canvas::new((800, 600));
            canvas.draws = vec![Draw::Text {
                world: Transform::identity(),
                font: font.clone(),
                text: "hello world".to_string(),
                size,
                color: Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
                alpha: 1.0,
                z: 0.0,
            }];
            canvas.expand_text(&mut atlases);
            let Draw::Sprite { data, .. } = &canvas.draws[0] else {
                panic!("the first glyph must be a sprite");
            };
            Arc::as_ptr(data)
        };
        let first = frame();
        let second = frame();
        assert_eq!(first, second, "frame two must reuse the atlas buffer");
        // The atlas was actually registered under its font key.
        assert!(atlases.contains_key(&key));
    }

    #[test]
    fn sprite_loader_round_trips_a_png_file() {
        let path = std::env::temp_dir().join(format!(
            "frost-sprite-test-{}.png",
            std::process::id()
        ));
        let buf: Vec<u8> = [
            [255u8, 0, 0, 255],
            [0, 255, 0, 128],
            [0, 0, 255, 0],
            [10, 20, 30, 40],
        ]
        .iter()
        .flat_map(|pixel| pixel.iter().copied())
        .collect();
        image::save_buffer(&path, &buf, 2, 2, image::ColorType::Rgba8)
            .expect("writing the test png");
        let shape = match Shape::sprite(&path) {
            Ok(shape) => shape,
            Err(err) => panic!("failed to load the test png: {err}"),
        };
        let Shape::Sprite {
            data,
            width,
            height,
            color,
            alpha,
        } = shape else {
            panic!("expected a sprite shape");
        };
        assert_eq!((width, height), (2, 2));
        // The decoded RGBA8 buffer matches the file's pixels byte for byte.
        assert_eq!(&data[..], &buf[..]);
        // The default tint is white and the default opacity is 1.0.
        assert_eq!(
            color,
            Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0
            }
        );
        assert_eq!(alpha, 1.0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn cloned_sprites_share_their_pixel_buffer() {
        // The pixels live behind an `Arc`: cloning a sprite shape must not
        // copy the buffer.
        let path = std::env::temp_dir().join(format!(
            "frost-sprite-share-{}.png",
            std::process::id()
        ));
        let buf = [255u8, 0, 0, 255, 0, 255, 0, 255];
        image::save_buffer(&path, &buf, 2, 1, image::ColorType::Rgba8)
            .expect("writing the test png");
        let shape = Shape::sprite(&path).unwrap();
        let Shape::Sprite { data: a, .. } = &shape else {
            panic!("expected a sprite shape");
        };
        let Shape::Sprite { data: b, .. } = &shape.clone() else {
            panic!("expected a sprite shape");
        };
        assert!(Arc::ptr_eq(a, b));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sprite_loader_reports_a_missing_file_as_io() {
        let err = Shape::sprite("frost-missing-sprite-file.png").unwrap_err();
        assert!(matches!(
            err,
            SpriteError::Io(err) if err.kind() == std::io::ErrorKind::NotFound
        ));
    }
}
