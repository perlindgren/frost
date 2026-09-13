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
//!     transform: frost::Transform::identity(),
//!     scale: [1.0, 1.0],
//!     shape: Some(frost::Shape::Background {
//!         color: frost::Color { r: 0.05, g: 0.06, b: 0.12 },
//!     }),
//!     children: Vec::new(),
//! });
//! frost::run(
//!     scene,
//!     |ctx: &mut frost::Context, _dt: f32| {
//!         let (w, h) = ctx.size();
//!         ctx.line(
//!             -w / 2.0, h / 2.0, w / 2.0, -h / 2.0,
//!             frost::Color { r: 1.0, g: 1.0, b: 1.0 },
//!             2.0,
//!             0.0,
//!         );
//!         ctx.circle(
//!             0.0, 0.0, h / 4.0,
//!             frost::Color { r: 0.9, g: 0.4, b: 0.2 },
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
//! each node holds a [`Transform`] and a `scale`, both relative to its
//! parent, plus its own optional [`Shape`]; the scale and transform apply to
//! the node's shape and compose onto its children. A [`Shape::Background`]
//! node fills the whole window with its color, ignoring its transform, and
//! is drawn at the very back.
//! The scene passed to [`run`] is drawn every frame; use
//! [`Canvas::draw_scene`] to draw additional scenes.
//!
//! Key presses are logged and Escape closes the window.

use std::borrow::Cow;
use std::error::Error;
use std::future::Future;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll, Wake, Waker};
use std::time::Instant;

use wgpu::{
    Adapter, BindGroup, BindGroupDescriptor, BindGroupEntry, BindingResource, Buffer,
    BufferBinding, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    CommandEncoderDescriptor, CurrentSurfaceTexture, Device, DeviceDescriptor, FragmentState,
    Instance, MultisampleState, PipelineCompilationOptions, PrimitiveState, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor,
    RequestAdapterOptions, ShaderModule, ShaderModuleDescriptor, ShaderSource, StoreOp, Surface,
    SurfaceTexture, TextureFormat, TextureViewDescriptor, VertexState,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::NamedKey;
use winit::window::{Window, WindowId};

/// The frame's clear color when no [`Shape::Background`] is drawn.
const DEFAULT_BACKGROUND: Color = Color {
    r: 0.07,
    g: 0.09,
    b: 0.14,
};

/// The anti-alias band in pixels, mirrored by the `aa` constant in the SDF
/// shader sources; keep the two in sync so bounding boxes cover every pixel
/// the shaders can write.
const AA_BAND: f32 = 0.75;
// ============================ public API ============================

/// An RGB color with channels in `0.0..=1.0`, used for the background and for
/// the color of each drawn object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    /// Red channel.
    pub r: f32,
    /// Green channel.
    pub g: f32,
    /// Blue channel.
    pub b: f32,
}

impl Color {
    /// The channels as `[r, g, b]`.
    fn channels(&self) -> [f32; 3] {
        [self.r, self.g, self.b]
    }
}

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
    /// root-to-leaf composition.
    /// Every node with a shape draws it (in the node's own local space)
    /// before its children, so parents paint under their descendants.
    ///
    /// All scene shapes share `z = 0.0`, so they are ordered by tree position
    /// and interleave with immediate draws by the usual stable `z` ordering.
    /// A [`Shape::Background`] is the exception: it ignores its transform,
    /// always sorts to the very back, and becomes the frame's clear color.
    pub fn draw_scene(&mut self, scene: &Scene) {
        self.draw_node(&scene.root, &Transform::identity());
    }

    fn draw_node(&mut self, node: &SceneNode, parent: &Transform) {
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
                    color: *color,
                    z: 0.0,
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
                    color: *color,
                    z: 0.0,
                },
                // The background ignores its transform: it is recorded in
                // call order and becomes the frame's clear color at render
                // time.
                Shape::Background { color } => Draw::Background { color: *color },
            };
            self.draws.push(draw);
        }
        for child in &node.children {
            self.draw_node(child, &world);
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

    /// Converts user coordinates (window center origin, y up) to the pixel
    /// coordinates the shaders use (top-left origin, y down).
    fn user_to_pixels(&self, x: f32, y: f32) -> [f32; 2] {
        self.user_to_pixel().apply([x, y])
    }
}

/// A 2D affine transform: `p' = m * p + t`.
///
/// The rows of `m` are `m[0]` and `m[1]`, so a point `(x, y)` maps to
/// `(m[0][0]*x + m[0][1]*y + t[0], m[1][0]*x + m[1][1]*y + t[1])`. Rotation
/// angles are measured from the +x axis toward +y, i.e. counter-clockwise in
/// the window-centered y-up coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    m: [[f32; 2]; 2],
    t: [f32; 2],
}

impl Transform {
    /// The identity transform (no change).
    pub const fn identity() -> Self {
        Self {
            m: [[1.0, 0.0], [0.0, 1.0]],
            t: [0.0, 0.0],
        }
    }

    /// A translation by `(x, y)`.
    pub const fn translate(x: f32, y: f32) -> Self {
        Self {
            m: [[1.0, 0.0], [0.0, 1.0]],
            t: [x, y],
        }
    }

    /// A rotation by `angle` radians, counter-clockwise in y-up coordinates.
    pub fn rotate(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        Self {
            m: [[c, -s], [s, c]],
            t: [0.0, 0.0],
        }
    }

    /// A scale by `x` horizontally and `y` vertically.
    pub const fn scale(x: f32, y: f32) -> Self {
        Self {
            m: [[x, 0.0], [0.0, y]],
            t: [0.0, 0.0],
        }
    }

    /// A uniform scale by `s`.
    pub const fn scale_uniform(s: f32) -> Self {
        Self::scale(s, s)
    }

    /// The transform that applies `self` first, then `other`:
    /// `self.compose(other)` maps a point `p` to `other.apply(self.apply(p))`.
    ///
    /// In matrix form, with `p' = m * p + t`, the composition multiplies in
    /// the opposite order of the application order: `other.m * self.m`.
    pub fn compose(&self, other: &Transform) -> Transform {
        let s = self.m;
        let o = other.m;
        Transform {
            m: [
                [
                    o[0][0] * s[0][0] + o[0][1] * s[1][0],
                    o[0][0] * s[0][1] + o[0][1] * s[1][1],
                ],
                [
                    o[1][0] * s[0][0] + o[1][1] * s[1][0],
                    o[1][0] * s[0][1] + o[1][1] * s[1][1],
                ],
            ],
            t: [
                o[0][0] * self.t[0] + o[0][1] * self.t[1] + other.t[0],
                o[1][0] * self.t[0] + o[1][1] * self.t[1] + other.t[1],
            ],
        }
    }

    /// Applies the transform to a point.
    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        [
            self.m[0][0] * p[0] + self.m[0][1] * p[1] + self.t[0],
            self.m[1][0] * p[0] + self.m[1][1] * p[1] + self.t[1],
        ]
    }

    /// The inverse transform, or `None` if the transform is degenerate (its
    /// linear part collapses points onto a line or a point).
    fn invert(&self) -> Option<Transform> {
        let a = self.m[0][0];
        let b = self.m[0][1];
        let c = self.m[1][0];
        let d = self.m[1][1];
        let det = a * d - b * c;
        if det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        Some(Transform {
            m: [[d * inv, -b * inv], [-c * inv, a * inv]],
            t: [
                -(self.t[0] * d - self.t[1] * b) * inv,
                (self.t[0] * c - self.t[1] * a) * inv,
            ],
        })
    }

    /// The scale factors along the x and y axes (the norms of the matrix
    /// columns), so the anti-alias band can be kept a constant size in
    /// screen pixels under scaling.
    fn scales(&self) -> [f32; 2] {
        [
            (self.m[0][0] * self.m[0][0] + self.m[1][0] * self.m[1][0]).sqrt(),
            (self.m[0][1] * self.m[0][1] + self.m[1][1] * self.m[1][1]).sqrt(),
        ]
    }
}

/// A filled geometric shape that a [`SceneNode`] can hold.
///
/// Coordinates and sizes are in the node's local space, in pixels; the
/// composed transforms of every ancestor apply to the shape, except for
/// [`Shape::Background`], which fills the whole window regardless of
/// transform.
#[derive(Clone, Copy, Debug)]
pub enum Shape {
    /// A filled circle centered at `center` with `radius`.
    Circle {
        center: [f32; 2],
        radius: f32,
        color: Color,
    },
    /// A filled rectangle centered at `center` with half-widths `extent`.
    Rectangle {
        center: [f32; 2],
        extent: [f32; 2],
        color: Color,
    },
    /// Fills the whole window with `color`.
    ///
    /// The node's transform (and its ancestors') is ignored, and the
    /// background is always at the very back of the frame: at render time it
    /// becomes the window's clear color, so it costs no draw call. Hang it on
    /// the scene root or anywhere else in the tree — position does not matter.
    /// When several nodes hold backgrounds, the last one in depth-first call
    /// order is the one that shows.
    Background {
        color: Color,
    },
}

/// A node in a [`Scene`] tree.
///
/// A node is a [`Transform`] and a `scale`, plus its own optional [`Shape`]
/// and its children. Both the transform and the scale are relative to the
/// node's parent and apply to the node's own shape as well as composing onto
/// all descendants. A node without a shape (`shape: None`) is a pure group
/// or pivot node, and a node without children is a leaf.
#[derive(Clone, Debug)]
pub struct SceneNode {
    /// The transform from the parent's coordinate space to this node's,
    /// applied to the node's own shape and composed onto its children.
    pub transform: Transform,
    /// The node's non-uniform scale, applied in its own coordinate space
    /// before its transform, so it scales the node's shape and its whole
    /// subtree. `[1.0, 1.0]` (the default) is no scaling.
    pub scale: [f32; 2],
    /// The shape this node draws, in its own local space, if any.
    pub shape: Option<Shape>,
    /// The child nodes, positioned in this node's coordinate space.
    pub children: Vec<Box<SceneNode>>,
}

/// A tree of [`SceneNode`]s rooted at a single node.
///
/// Pass a scene to [`run`]: it is drawn every frame, *after* the [`Process`]
/// runs, so the process can mutate it in place (via [`Context::scene`]) to
/// animate it. Use [`Canvas::draw_scene`] to draw additional scenes.
#[derive(Clone, Debug)]
pub struct Scene {
    /// The root node; the scene is walked depth-first from here.
    pub root: SceneNode,
}

impl Scene {
    /// Creates a scene from its root node.
    pub fn new(root: SceneNode) -> Self {
        Self { root }
    }
}

impl Default for Scene {
    /// An empty scene: an identity root node with no shape and no children,
    /// for apps that only use the immediate draw methods.
    fn default() -> Self {
        Self {
            root: SceneNode {
                transform: Transform::identity(),
                scale: [1.0, 1.0],
                shape: None,
                children: Vec::new(),
            },
        }
    }
}

/// A drawing recorded for the current frame, in pixel space.
///
/// `z` is the draw order: lower `z` is drawn first (further back). Drawings
/// with the same `z` are drawn in call order, so the last one drawn is on top.
#[derive(Clone, Copy, Debug)]
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
                // The box may be rotated, so transform its four corners and
                // take the axis-aligned bounding box of the result.
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
            // A background never draws; the early return above covers it.
            Draw::Background { .. } => unreachable!(),
        };
        clamp_box_to_area(min, max, area)
    }
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
}

impl Context<'_> {
    /// Mutable access to the scene owned by [`run`].
    pub fn scene(&mut self) -> &mut Scene {
        &mut *self.scene
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

/// A value that a [`Tween`] can interpolate.
///
/// Implement it for your own types to tween them; it is implemented for
/// `f32` and `[f32; 2]`.
pub trait Tweenable: Copy {
    /// Linearly interpolates from `self` toward `other`: `t = 0.0` yields
    /// `self`, `t = 1.0` yields `other`.
    fn tween(self, other: Self, t: f32) -> Self;
}

impl Tweenable for f32 {
    fn tween(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

impl Tweenable for [f32; 2] {
    fn tween(self, other: Self, t: f32) -> Self {
        [self[0].tween(other[0], t), self[1].tween(other[1], t)]
    }
}

/// How a [`Tween`] behaves once its duration has elapsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    /// Travel to the target and stay there.
    Once,
    /// Jump back to the start and travel again.
    Loop,
    /// Travel to the target, then back to the start, forever.
    PingPong,
}

/// A linear tween of a value from `from` to `to` over `duration` seconds.
///
/// Each [`Tween::tick`] advances the tween by a time step and returns the
/// current interpolated value. With the default [`Repeat::PingPong`] the
/// value travels `from -> to -> from -> ...` at constant speed; see
/// [`Tween::repeat`] for the other modes.
#[derive(Clone, Copy, Debug)]
pub struct Tween<T: Tweenable> {
    from: T,
    to: T,
    /// Elapsed time, in seconds, since the tween was created.
    time: f32,
    /// Seconds for one leg (the `from -> to` travel).
    duration: f32,
    repeat: Repeat,
}

impl<T: Tweenable> Tween<T> {
    /// Creates a [`Repeat::PingPong`] tween whose one-way travel from `from`
    /// to `to` takes `duration` seconds.
    pub fn new(from: T, to: T, duration: f32) -> Self {
        Self {
            from,
            to,
            time: 0.0,
            duration: duration.max(1e-6),
            repeat: Repeat::PingPong,
        }
    }

    /// Sets the repeat mode.
    pub fn repeat(mut self, repeat: Repeat) -> Self {
        self.repeat = repeat;
        self
    }

    /// Advances by `dt` seconds and returns the current interpolated value.
    pub fn tick(&mut self, dt: f32) -> T {
        self.time += dt;
        match self.repeat {
            Repeat::Once => {
                let t = (self.time / self.duration).min(1.0);
                self.from.tween(self.to, t)
            }
            Repeat::Loop => {
                let t = (self.time % self.duration) / self.duration;
                self.from.tween(self.to, t)
            }
            Repeat::PingPong => {
                let cycle = self.time % (2.0 * self.duration);
                let t = if cycle < self.duration {
                    cycle / self.duration
                } else {
                    1.0 - (cycle - self.duration) / self.duration
                };
                self.from.tween(self.to, t)
            }
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
        format: None,
        last_time: None,
        scene,
        process,
    };
    event_loop.run_app(&mut app)?;

    log::info!("event loop finished");
    Ok(())
}

// ============================ internal machinery ============================

const SHADER: &str = r#"
struct Uniforms {
    a: vec2<f32>,
    b: vec2<f32>,
    color: vec3<f32>,
    width: f32,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Full-screen triangle in NDC; the scissor is set on the CPU to the
    // line's bounding box, so fragments outside it are discarded and the
    // fragment shader only runs over the pixels the line can write.
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[i], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let p = frag_coord.xy;
    // Distance in pixels from the fragment to the segment a-b.
    let ab = u.b - u.a;
    let t = clamp(dot(p - u.a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
    let dist = length(p - (u.a + ab * t));
    let half_width = u.width * 0.5;
    let aa = 0.75; // ~1px anti-alias band.
    let alpha = 1.0 - smoothstep(half_width - aa, half_width + aa, dist);
    // Emit the line's color and coverage; composited over the existing
    // attachment via alpha blending.
    return vec4<f32>(u.color, alpha);
}
"#;

const CIRCLE_SHADER: &str = r#"
struct CircleUniforms {
    center: vec2<f32>,
    color: vec3<f32>,
    radius: f32,
};

@group(0) @binding(0)
var<uniform> u: CircleUniforms;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Full-screen triangle in NDC; the scissor is set on the CPU to the
    // circle's bounding box, so fragments outside it are discarded and the
    // fragment shader only runs over the pixels the circle can write.
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[i], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let p = frag_coord.xy;
    // Distance in pixels from the fragment to the circle center.
    let d = length(p - u.center);
    let aa = 0.75; // ~1px anti-alias band.
    let alpha = 1.0 - smoothstep(u.radius - aa, u.radius + aa, d);
    // Emit the circle's color and coverage; composited over the existing
    // attachment via alpha blending.
    return vec4<f32>(u.color, alpha);
}
"#;

const RECT_SHADER: &str = r#"
struct RectUniforms {
    center: vec2<f32>,
    extent: vec2<f32>,
    color: vec3<f32>,
};

@group(0) @binding(0)
var<uniform> u: RectUniforms;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Full-screen triangle in NDC; the scissor is set on the CPU to the
    // rectangle's bounding box, so fragments outside it are discarded and
    // the fragment shader only runs over the pixels it can write.
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[i], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let p = frag_coord.xy;
    // Signed distance in pixels from the fragment to the rectangle border:
    // negative inside, positive outside.
    let d = max(
        abs(p.x - u.center.x) - u.extent.x,
        abs(p.y - u.center.y) - u.extent.y,
    );
    let aa = 0.75; // ~1px anti-alias band.
    let alpha = 1.0 - smoothstep(-aa, aa, d);
    // Emit the rectangle's color and coverage; composited over the existing
    // attachment via alpha blending.
    return vec4<f32>(u.color, alpha);
}
"#;

const SHAPE_SHADER: &str = r#"
struct ShapeUniforms {
    to_local: mat2x2<f32>,
    translation: vec2<f32>,
    center: vec2<f32>,
    params: vec2<f32>,
    color: vec3<f32>,
    misc: vec2<f32>, // x: anti-alias band (local units), y: kind (0.0 circle, 1.0 rectangle)
};

@group(0) @binding(0)
var<uniform> u: ShapeUniforms;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Full-screen triangle in NDC; the scissor is set on the CPU to the
    // shape's bounding box, so fragments outside it are discarded and the
    // fragment shader only runs over the pixels the shape can write.
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[i], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    // Convert the fragment's pixel coordinate into the shape's local space
    // (the inverse of its world transform), then evaluate the signed
    // distance there, so rotation and scaling apply for free. The matrix
    // is stored column-major, so it multiplies the coordinate as a column
    // vector, matching the Rust-side transform convention.
    let p = u.to_local * frag_coord.xy + u.translation;
    var d: f32;
    if (u.misc.y < 0.5) {
        d = length(p - u.center) - u.params.x;
    } else {
        // Signed distance in local units from the fragment to the
        // rectangle border: negative inside, positive outside.
        let q = abs(p - u.center) - u.params;
        d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
    }
    let alpha = 1.0 - smoothstep(-u.misc.x, u.misc.x, d);
    // Emit the shape's color and coverage; composited over the existing
    // attachment via alpha blending.
    return vec4<f32>(u.color, alpha);
}
"#;

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
    /// The surface format the current pipelines were built for; they are only
    /// rebuilt when this changes.
    format: Option<TextureFormat>,
    /// Timestamp of the previous rendered frame, used to compute `dt`.
    last_time: Option<Instant>,
    /// The scene drawn every frame, after the process runs.
    scene: Scene,
    process: P,
}

impl<P: Process> ApplicationHandler for Frost<P> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }

        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("frost"))
                .expect("failed to create window"),
        );
        // winit (Wayland) only delivers RedrawRequested after a compositor
        // frame callback, so explicitly request the first frame; otherwise
        // the window is never mapped and nothing is ever drawn.
        window.request_redraw();
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
                if event.state == ElementState::Pressed && event.logical_key == NamedKey::Escape {
                    log::info!("escape pressed, exiting");
                    event_loop.exit();
                }
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

impl<P: Process> Frost<P> {
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
            source: ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
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
        let now = Instant::now();
        let dt = self
            .last_time
            .map(|last| now.duration_since(last).as_secs_f32().min(1.0))
            .unwrap_or(0.0);
        self.last_time = Some(now);
        let process = &mut self.process;
        let scene = &mut self.scene;
        {
            let mut ctx = Context {
                canvas: &mut canvas,
                scene,
            };
            process.process(&mut ctx, dt);
        }
        // The scene was just updated; draw it into the frame's draw list.
        canvas.draw_scene(&self.scene);

        // Paint order: ascending z, lower z behind. `sort_by` is stable, so
        // draws with equal z keep call order and the last drawn is on top.
        canvas.draws.sort_by(|a, b| {
            a.z()
                .partial_cmp(&b.z())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let (Some(line_pipeline), Some(circle_pipeline), Some(rect_pipeline), Some(shape_pipeline)) = (
            self.line_pipeline.as_ref(),
            self.circle_pipeline.as_ref(),
            self.rect_pipeline.as_ref(),
            self.shape_pipeline.as_ref(),
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
                            a: 1.0,
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
                    // A background's scissor rect is `None`, so it continued
                    // above; this arm keeps the match exhaustive.
                    Draw::Background { .. } => {}
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
    let mut data = vec![0u8; 32];
    write_f32_at(&mut data, 0, a[0]);
    write_f32_at(&mut data, 4, a[1]);
    write_f32_at(&mut data, 8, b[0]);
    write_f32_at(&mut data, 12, b[1]);
    write_f32_at(&mut data, 16, color.r);
    write_f32_at(&mut data, 20, color.g);
    write_f32_at(&mut data, 24, color.b);
    write_f32_at(&mut data, 28, width);
    data
}

/// Circle uniform data, 32 bytes, matching the WGSL uniform-space layout of
/// the `CircleUniforms` Wgsl struct: `center` @ 0, `color` @ 16 (a
/// vec3<f32> is 16-byte aligned in uniform space, leaving an 8-byte gap),
/// `radius` @ 28 (the next member is aligned to its own alignment, so the
/// f32 follows the vec3 without a gap); the struct size rounds up to 32.
fn circle_uniform_data(center: [f32; 2], color: Color, radius: f32) -> Vec<u8> {
    let mut data = vec![0u8; 32];
    write_f32_at(&mut data, 0, center[0]);
    write_f32_at(&mut data, 4, center[1]);
    write_f32_at(&mut data, 16, color.r);
    write_f32_at(&mut data, 20, color.g);
    write_f32_at(&mut data, 24, color.b);
    write_f32_at(&mut data, 28, radius);
    data
}

/// Rectangle uniform data, matching the `RectUniforms` Wgsl struct: center,
/// extent, color. The 28 bytes of data are zero-padded to the struct's
/// 32-byte minimum binding size.
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
/// `params` @ 32, `color` @ 48 (spanning 48..60), `misc` @ 64 (aa @ 64,
/// kind @ 68); the struct size rounds up to 80.
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
    // vec3: 12 bytes with 16-byte alignment, so it starts at 48 and spans
    // 48..60.
    write_f32_at(&mut data, 48, color.r);
    write_f32_at(&mut data, 52, color.g);
    write_f32_at(&mut data, 56, color.b);
    write_f32_at(&mut data, 64, aa);
    write_f32_at(&mut data, 68, kind);
    data
}

struct NoopWaker;

impl Wake for NoopWaker {
    fn wake(self: Arc<Self>) {}
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWaker));
    let mut cx = TaskContext::from_waker(&waker);
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
            shape: Some(Shape::Circle {
                center: [0.0, 0.0],
                radius: 5.0,
                color: black(),
            }),
            children: vec![Box::new(SceneNode {
                transform: Transform::scale_uniform(2.0),
                scale: [1.0, 1.0],
                shape: None,
                children: vec![Box::new(SceneNode {
                    transform: Transform::identity(),
                    scale: [1.0, 1.0],
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
            shape: None,
            children: vec![Box::new(SceneNode {
                transform: Transform::translate(10.0, 0.0),
                scale: [1.0, 1.0],
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
        // `params` @ 32; vec3<f32> (12 bytes, 16-byte aligned) `color` @ 48;
        // `misc` vec2 @ 64; struct span rounds up to 80.
        let inv = Transform::translate(1.5, -2.5).invert().unwrap();
        let data = shape_uniform_data(inv, [7.0, 8.0], [9.0, 10.0], 1.0, 0.25, Color {
            r: 0.1,
            g: 0.2,
            b: 0.3,
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
        // `params` ends at 40; the 16-byte-aligned vec3 color starts at 48,
        // so bytes 40..48 are padding (zeroed by the `vec![0u8; 80]` init).
        assert_eq!(f32_at(40), 0.0);
        assert_eq!(f32_at(48), 0.1);
        assert_eq!(f32_at(52), 0.2);
        assert_eq!(f32_at(56), 0.3);
        // Bytes 60..64 are the vec3's trailing padding, and the `misc` vec2
        // begins at byte 64: `aa` at 64, `kind` at 68; 72..80 is the
        // struct's alignment padding.
        assert_eq!(f32_at(60), 0.0);
        assert_eq!(f32_at(64), 0.25);
        assert_eq!(f32_at(68), 1.0);
        assert_eq!(f32_at(72), 0.0);
        assert_eq!(data.len(), 80);
    }

    #[test]
    fn tween_ping_pong_travels_to_and_fro() {
        let mut tw = Tween::new(0.0, 100.0, 1.0); // 1s per leg, ping-pong
        assert_eq!(tw.tick(0.5), 50.0); // halfway to `to`
        assert_eq!(tw.tick(0.5), 100.0); // at `to`
        assert_eq!(tw.tick(0.5), 50.0); // halfway back
        assert_eq!(tw.tick(0.5), 0.0); // back at `from`, cycle restarts
    }

    #[test]
    fn tween_once_stays_at_the_end() {
        let mut tw = Tween::new(0.0, 10.0, 1.0).repeat(Repeat::Once);
        assert_eq!(tw.tick(0.5), 5.0);
        assert_eq!(tw.tick(0.5), 10.0);
        assert_eq!(tw.tick(5.0), 10.0); // stays at the target
    }

    #[test]
    fn tween_loop_wraps_at_the_end() {
        let mut tw = Tween::new(0.0, 10.0, 1.0).repeat(Repeat::Loop);
        assert_eq!(tw.tick(0.5), 5.0);
        assert_eq!(tw.tick(0.4), 9.0);
        assert_eq!(tw.tick(0.1), 0.0); // wrapped back to `from`
        assert_eq!(tw.tick(0.5), 5.0);
    }

    #[test]
    fn tween_interpolates_vectors() {
        let mut tw = Tween::new([0.0, 10.0], [20.0, 0.0], 1.0);
        assert_eq!(tw.tick(0.5), [10.0, 5.0]);
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
        };
        let second = Color {
            r: 0.0,
            g: 0.0,
            b: 1.0,
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
            shape: Some(Shape::Rectangle {
                center: [0.0, 0.0],
                extent: [10.0, 10.0],
                color: black(),
            }),
            children: vec![Box::new(SceneNode {
                // Transformed on purpose: the background must ignore it.
                transform: Transform::translate(50.0, 50.0),
                scale: [1.0, 1.0],
                shape: Some(Shape::Background {
                    color: Color {
                        r: 0.0,
                        g: 0.0,
                        b: 1.0,
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
                b: 1.0
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
            shape: Some(Shape::Circle {
                center: [1.0, 1.0],
                radius: 1.0,
                color: black(),
            }),
            children: vec![Box::new(SceneNode {
                transform: Transform::translate(1.0, 0.0),
                scale: [1.0, 1.0],
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
}
