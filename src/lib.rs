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
//! renderer: spawn particles in [`Process::process`], advance them each
//! frame with [`ParticleSystem::update`] under a constant gravity, and
//! draw them with the immediate draws, fading each one by its remaining
//! lifetime.
//!
//! Key presses are logged, the currently held keys are reported by
//! [`Context::key_down`], the mouse cursor's position by
//! [`Context::mouse_position`], and Escape closes the window.
//!
//! Presentation is vsync'd by default: frames are presented once per
//! vertical blank, at the display's refresh rate — the rate
//! [`Context::expected_fps`] reports. [`run_configured`] takes a [`Config`]
//! to turn vsync off for uncapped frame rates.

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

mod particles;
pub use particles::*;

mod shaders;

mod text;

mod tween;
pub use tween::*;

mod collision;
pub use collision::*;

/// The drawing surface for a frame, reachable through the [`Context`] passed
/// to [`Process::process`] (which derefs to it).
///
/// Coordinates are in pixels with the origin at the window center and the y
/// axis pointing up: the top-left corner is `(-width/2, height/2)` and the
/// bottom-right corner is `(width/2, -height/2)`.
pub struct Canvas {
    size: (f32, f32),
    /// The base draw group: the immediate draw methods and every scene's
    /// root subtree, mixed by `z` and call order.
    draws: Vec<Draw>,
    /// One draw list per explicit scene layer, in declaration order (the
    /// same index as in `layer_orders`).
    layer_draws: Vec<Vec<Draw>>,
    /// The order of each explicit scene layer, in declaration order.
    layer_orders: Vec<f32>,
}

impl Canvas {
    fn new(pixel_size: (u32, u32)) -> Self {
        Self {
            size: (pixel_size.0 as f32, pixel_size.1 as f32),
            draws: Vec::new(),
            layer_draws: Vec::new(),
            layer_orders: Vec::new(),
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
    /// The scene's root subtree joins the base group — the same group as
    /// the immediate draw methods — so its shapes interleave with the
    /// immediate draws by the usual stable `z` ordering, each shape carrying
    /// the `z` accumulated from its ancestors' `order` values.
    ///
    /// The scene's layers are separate groups: each layer is painted as a
    /// whole at its [`Layer::order`], after every group with a lower order
    /// and before every group with a higher one (higher order on top), and
    /// within the layer the local ordering applies.
    ///
    /// If the scene has a [`Scene::camera`], every group is drawn in the
    /// camera node's coordinate space instead of the fixed window-centered
    /// user space: the scene is transformed by the inverse of the camera's
    /// world transform (with the translation scaled per group), so the
    /// camera stays at the window origin and the scene moves and rotates
    /// around it. Each group uses the camera's translation scaled by its
    /// speed — the base group at `1.0`, each layer at its [`Layer::speed`] —
    /// so a layer with a higher speed moves faster than the camera (it reads
    /// as closer) and one with a lower speed slower (farther away).
    ///
    /// A [`Shape::Background`] is the exception in every group: it ignores
    /// its transform, never draws, and becomes the frame's clear color.
    ///
    /// A layer with a non-zero [`Layer::repeat`] component is drawn once
    /// per copy of its content that can overlap the window — the same
    /// content, re-used with just a displacement — so it can be scrolled
    /// through infinitely. With a pure-translation camera the window's box
    /// is at most one period wide, so only the few copies it and the
    /// content's span around it reach are drawn per axis; a rotated camera
    /// widens the box, and every copy it reaches is drawn. An object
    /// crossing a tile boundary is split into wrapping slices by the
    /// per-object scissor test. A period below the window size, or an
    /// object whose extent in a repeating axis exceeds its period, aborts
    /// the program with an error.
    pub fn draw_scene(&mut self, scene: &Scene) {
        let pixel = self.user_to_pixel();
        // The scene's camera, when it has one: the groups are drawn in the
        // camera node's coordinate space, so it stays at the window origin
        // and the scene moves around it. Without a camera (or with a
        // degenerate camera transform) the groups draw in the fixed
        // window-centered user space.
        let camera = scene.camera_world();
        // The base group renders from the camera at the full speed 1.0.
        let view = |speed: f32| match &camera {
            Some(c) => c
                .scaled_translation(speed)
                .invert()
                .unwrap_or(Transform::identity()),
            None => Transform::identity(),
        };
        draw_node(pixel, &scene.root, &view(1.0), 0.0, WHITE, &mut self.draws);
        for layer in &scene.layers {
            let index = self.layer_orders.len();
            self.layer_orders.push(layer.order);
            self.layer_draws.push(Vec::new());
            // A repeating layer is drawn once per tile the window overlaps,
            // each copy displaced by the tile's offset in the layer's own
            // coordinate space (before the camera view is applied).
            let view = view(layer.speed);
            for (ox, oy) in layer_repeat_offsets(layer, &view, self.size) {
                draw_node(
                    pixel,
                    &layer.root,
                    &Transform::translate(ox, oy).compose(&view),
                    0.0,
                    WHITE,
                    &mut self.layer_draws[index],
                );
            }
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

    /// Expands every [`Draw::Text`] in every draw group of this frame — the
    /// base group and each layer's group — into one [`Draw::Sprite`] per
    /// shaped glyph, spliced in at the text draw's position so the stable
    /// z-sort keeps the glyph quads where the text node was. The glyphs stay
    /// in the group their text node belongs to, so a layer's text is sorted
    /// within that layer.
    ///
    /// The glyphs are laid out with `text::layout` (pen positions, y up,
    /// from the text's left baseline origin) and rasterized with
    /// `text::rasterize` into the per-`(font, size)` atlas in `atlases`,
    /// which the caller keeps between frames so unchanged text never
    /// re-rasterizes and its texture buffer keeps a stable identity.
    /// Each glyph's quad is centered on its ink box; the whole text block
    /// is centered on the text node's origin.
    pub(crate) fn expand_text(&mut self, atlases: &mut HashMap<(u64, u32), text::Atlas>) {
        let pixel = self.user_to_pixel();
        let draws = std::mem::take(&mut self.draws);
        self.draws = expand_text_list(pixel, draws, atlases);
        for list in &mut self.layer_draws {
            let draws = std::mem::take(list);
            *list = expand_text_list(pixel, draws, atlases);
        }
    }

    /// Converts user coordinates (window center origin, y up) to the pixel
    /// coordinates the shaders use (top-left origin, y down).
    fn user_to_pixels(&self, x: f32, y: f32) -> [f32; 2] {
        self.user_to_pixel().apply([x, y])
    }

    /// The frame's draws in paint order.
    ///
    /// The base group and every explicit scene layer are separate draw
    /// groups. The groups are painted by ascending layer order — higher
    /// order on top — with the base group at order `0.0`, declared before
    /// every layer, so at an equal order it is painted first. Within each
    /// group, the draws are sorted by ascending `z`; the sort is stable, so
    /// draws with equal `z` keep call order and the last drawn is on top.
    pub(crate) fn paint_order(&mut self) -> Vec<Draw> {
        // The groups as (layer order, declaration index): the base group is
        // order 0.0, declared before every layer, so at an equal order it
        // is painted first.
        enum Group {
            Base,
            Layer(usize),
        }
        let mut groups: Vec<Group> = Vec::with_capacity(1 + self.layer_draws.len());
        groups.push(Group::Base);
        groups.extend((0..self.layer_draws.len()).map(Group::Layer));
        let order_of = |group: &Group| match group {
            Group::Base => (0.0f32, 0usize),
            Group::Layer(index) => (self.layer_orders[*index], *index + 1),
        };
        groups.sort_by(|a, b| {
            let (order_a, declared_a) = order_of(a);
            let (order_b, declared_b) = order_of(b);
            order_a
                .partial_cmp(&order_b)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(declared_a.cmp(&declared_b))
        });
        let mut painted = Vec::new();
        for group in groups {
            let list = match group {
                Group::Base => &mut self.draws,
                Group::Layer(index) => &mut self.layer_draws[index],
            };
            let mut draws = std::mem::take(list);
            draws.sort_by(|a, b| {
                a.z()
                    .partial_cmp(&b.z())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            painted.append(&mut draws);
        }
        painted
    }
}

/// Draws one [`SceneNode`] and its subtree into `draws`, depth-first: the
/// node's shape (if any) before its children, so parents paint under their
/// descendants.
///
/// `parent` is the node's ancestors' world transform in user space, `order`
/// their accumulated draw order, and `modulate` their accumulated color
/// modulation; each is combined with the node's own value and passed on to
/// the children, so a descendant's values are the full root-to-leaf
/// composition.
fn draw_node(
    user_to_pixel: Transform,
    node: &SceneNode,
    parent: &Transform,
    order: f32,
    modulate: Color,
    draws: &mut Vec<Draw>,
) {
    // The node's effective draw order: the order inherited from its
    // ancestors plus its own, applied to the node's shape and passed on to
    // its whole subtree, just like the transform and the scale.
    let order = order + node.order;
    // The node's effective color modulation: the modulation inherited from
    // its ancestors multiplied by its own, applied to the node's shape's
    // color and passed on to its whole subtree, just like the transform and
    // the scale.
    let modulate = modulate.mul(node.modulate);
    // The node's world transform in user space: its scale first (innermost,
    // in the node's own space), then its transform (local -> parent space),
    // then the parent's world transform. Through `world` the scale
    // therefore applies to the node's shape and to its whole subtree, just
    // like the transform.
    let local = Transform::scale(node.scale[0], node.scale[1]).compose(&node.transform);
    let world = local.compose(parent);
    if let Some(shape) = &node.shape {
        // Scale the anti-alias band with the transform's scale so it stays a
        // constant number of screen pixels wide under scaling.
        let [sx, sy] = world.scales();
        let aa = AA_BAND / sx.max(sy).max(1e-9);
        let draw = match shape {
            // The shape shader evaluates in pixel space (top-left, y down),
            // so compose the user-to-pixel transform onto the world
            // transform, like the direct-draw methods do for their
            // arguments.
            Shape::Circle {
                center,
                radius,
                color,
            } => Draw::Shape {
                world: world.compose(&user_to_pixel),
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
                world: world.compose(&user_to_pixel),
                center: *center,
                params: [extent[0].max(0.0), extent[1].max(0.0)],
                kind: 1.0,
                aa,
                color: color.mul(modulate),
                z: order,
            },
            // The sprite's local space is centered on the origin, one
            // texture pixel per scene pixel, so its extent is the texture
            // size.
            Shape::Sprite {
                data,
                width,
                height,
                color,
                alpha,
            } => Draw::Sprite {
                world: world.compose(&user_to_pixel),
                data: data.clone(),
                size: [(*width as f32).max(0.0), (*height as f32).max(0.0)],
                texture_size: [*width, *height],
                aa,
                tint: color.mul(modulate),
                alpha: *alpha,
                uv_rect: [0.0, 0.0, 1.0, 1.0],
                z: order,
            },
            // Text is recorded in user space; `Canvas::expand_text` lays it
            // out and turns each glyph into a sprite quad before the frame
            // is rendered.
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
            // The background ignores its transform: it is recorded in call
            // order and becomes the frame's clear color at render time.
            Shape::Background { color } => Draw::Background {
                color: color.mul(modulate),
            },
        };
        draws.push(draw);
    }
    for child in &node.children {
        draw_node(user_to_pixel, child, &world, order, modulate, draws);
    }
}

/// The copy offsets a repeating [`Layer`] is drawn at for this frame, in
/// the layer's own coordinate space (before the camera view is applied):
/// one offset per copy of the content that can overlap the window. A
/// non-repeating axis (`repeat` component `0.0`) has exactly one copy, at
/// offset `0.0`; a repeating axis has one copy per `abs(repeat)` period
/// that can reach the window's box — with a pure-translation camera the
/// box is at most one period wide, so only the few copies it and the
/// content's span around it reach. The offsets are ordered so `(0.0, 0.0)`
/// comes first when it is among them, so the stable z-sort keeps the base
/// content drawn before its copies at equal `z`.
///
/// Aborts the program with an error when a non-zero repeat period is
/// smaller than the window's width/height (the minimum repeat offset), or
/// when an object's extent in a repeating axis exceeds that axis's period
/// (the same object would be drawn twice).
fn layer_repeat_offsets(layer: &Layer, view: &Transform, size: (f32, f32)) -> Vec<(f32, f32)> {
    let (w, h) = size;
    // The window in the layer's coordinate space: the user-space window
    // rectangle (origin center, y up) mapped by the inverse view, as an
    // axis-aligned box.
    let to_layer = view.invert().unwrap_or(Transform::identity());
    let corners = [
        to_layer.apply([-w / 2.0, -h / 2.0]),
        to_layer.apply([w / 2.0, -h / 2.0]),
        to_layer.apply([-w / 2.0, h / 2.0]),
        to_layer.apply([w / 2.0, h / 2.0]),
    ];
    let mut min = corners[0];
    let mut max = corners[0];
    for corner in corners.iter().skip(1) {
        min = [min[0].min(corner[0]), min[1].min(corner[1])];
        max = [max[0].max(corner[0]), max[1].max(corner[1])];
    }
    // The layer's content extent in the layer's coordinate space (the union
    // of all its objects' boxes), or None when the layer has no drawable
    // objects: a copy of the content displaced by k * period spans
    // [content_min + k * period, content_max + k * period], and the content
    // need not sit inside the tile [0, period).
    let content = check_layer_repeat_extent(&layer.root, &Transform::identity(), layer.repeat);
    // The offsets of the axis: every copy of the content that can reach
    // the window's box [min_edge, max_edge], or the single copy at 0.0 for
    // a non-repeating axis. A copy displaced by k * period spans
    // [content_min + k * period, content_max + k * period], so the copies
    // that reach the box run one content span further out than the tiles
    // the box alone overlaps. With a pure-translation camera the box is
    // the window itself, at most one period wide, so this is only a few
    // copies; a rotated camera widens the box, and the extra copies cover
    // it.
    let offsets = |component: f32, min_edge: f32, max_edge: f32, window: f32, axis: u8| -> Vec<f32> {
        if component == 0.0 {
            return vec![0.0];
        }
        let period = component.abs();
        if period < window {
            let (name, what) = match axis {
                0 => ("repeat_x", "the window width"),
                _ => ("repeat_y", "the window height"),
            };
            panic!(
                "frost: the layer's {name} {component} is smaller than {what} \
                 ({window}px); the minimum repeat offset is the window size"
            );
        }
        let (content_min, content_max) = match (content, axis) {
            (Some((cmin, cmax)), 0) => (cmin[0], cmax[0]),
            (Some((cmin, cmax)), _) => (cmin[1], cmax[1]),
            (None, _) => (0.0, 0.0),
        };
        // The copies [k * period] whose displaced content overlaps the box:
        // content_min + k * period < max_edge and content_max + k * period
        // > min_edge, a copy just touching the box (zero visible width) is
        // not drawn, matching the window-edge convention elsewhere.
        let k0 = ((min_edge - content_max) / period).floor() as i64 + 1;
        let k1 = ((max_edge - content_min) / period).ceil() as i64 - 1;
        (k0..=k1).map(|k| k as f32 * period).collect()
    };
    let xs = offsets(layer.repeat[0], min[0], max[0], w, 0);
    let ys = offsets(layer.repeat[1], min[1], max[1], h, 1);
    let mut tile_offsets = xs
        .iter()
        .flat_map(|x| ys.iter().map(move |y| (*x, *y)))
        .collect::<Vec<_>>();
    if let Some(index) = tile_offsets.iter().position(|&(x, y)| x == 0.0 && y == 0.0) {
        tile_offsets.swap(0, index);
    }
    tile_offsets
}

/// Aborts the program with an error when an object of a repeating layer has
/// an extent in a repeating axis that exceeds that axis's period: beyond
/// that, the object wraps more than once and the same object would be drawn
/// twice.
///
/// `parent` is the node's ancestors' transform in the layer's coordinate
/// space (the same accumulation as in [`draw_node`], without the camera
/// view), so each object's extent is measured in layer-space pixels — where
/// the repeat period lives. The extent is the object's axis-aligned box,
/// which is exact for the axis-parallel tile displacements.
///
/// Returns the union of all the layer's objects' boxes — the content's
/// extent in the layer's coordinate space — or `None` when the layer has
/// no drawable objects. [`layer_repeat_offsets`] uses it to reach the
/// copies of the content that can overlap the window's box: the content
/// need not sit inside the tile `[0, period)`, so a copy displaced by
/// `k * period` can reach the box even when that tile does not.
fn check_layer_repeat_extent(node: &SceneNode, parent: &Transform, repeat: [f32; 2]) -> Option<([f32; 2], [f32; 2])> {
    let local = Transform::scale(node.scale[0], node.scale[1]).compose(&node.transform);
    let world = local.compose(parent);
    let mut extent = None;
    if let Some((kind, center, half)) = node.shape.as_ref().and_then(shape_local_box) {
        let (box_min, box_max) = aabb_of_box(&world, center, half[0], half[1]);
        let width = box_max[0] - box_min[0];
        let height = box_max[1] - box_min[1];
        if repeat[0] != 0.0 && width > repeat[0].abs() {
            panic!(
                "frost: a {kind} in the repeating layer is wider ({width:.1}px) \
                 than its repeat_x offset ({}px); the same object would be \
                 drawn twice",
                repeat[0].abs()
            );
        }
        if repeat[1] != 0.0 && height > repeat[1].abs() {
            panic!(
                "frost: a {kind} in the repeating layer is taller ({height:.1}px) \
                 than its repeat_y offset ({}px); the same object would be \
                 drawn twice",
                repeat[1].abs()
            );
        }
        extent = Some((box_min, box_max));
    }
    for child in &node.children {
        if let Some((child_min, child_max)) = check_layer_repeat_extent(child, &world, repeat) {
            extent = Some(match extent {
                Some((min, max)) => (
                    [min[0].min(child_min[0]), min[1].min(child_min[1])],
                    [max[0].max(child_max[0]), max[1].max(child_max[1])],
                ),
                None => (child_min, child_max),
            });
        }
    }
    extent
}

/// The shape's box in its node's local space — its kind, its center, and
/// its half extents — or `None` for shapes that never draw
/// (`Shape::Background`) or cannot be measured (a broken font).
///
/// A text block is its laid-out width by its `ascent + descent`, centered
/// on the node's origin like its glyphs (see `expand_text_list`).
fn shape_local_box(shape: &Shape) -> Option<(&'static str, [f32; 2], [f32; 2])> {
    match shape {
        Shape::Circle { center, radius, .. } => {
            Some(("circle", *center, [*radius, *radius]))
        }
        Shape::Rectangle { center, extent, .. } => {
            Some(("rectangle", *center, [extent[0], extent[1]]))
        }
        // A sprite's local space is centered on the origin, one texture
        // pixel per scene pixel.
        Shape::Sprite { width, height, .. } => {
            Some(("sprite", [0.0, 0.0], [*width as f32 / 2.0, *height as f32 / 2.0]))
        }
        Shape::Text { font, text, size, .. } => {
            let layout = text::layout(font, text, *size)?;
            Some((
                "text",
                [0.0, 0.0],
                [layout.width / 2.0, (layout.ascent + layout.descent) / 2.0],
            ))
        }
        // A background never draws; nothing to measure.
        Shape::Background { .. } => None,
    }
}

/// Expands every [`Draw::Text`] in the list into one [`Draw::Sprite`] per
/// shaped glyph, spliced in at the text draw's position so the stable
/// z-sort keeps the glyph quads where the text node was.
///
/// The glyphs are laid out with `text::layout` (pen positions, y up, from
/// the text's left baseline origin) and rasterized with `text::rasterize`
/// into the per-`(font, size)` atlas in `atlases`, which the caller keeps
/// between frames so unchanged text never re-rasterizes and its texture
/// buffer keeps a stable identity. Each glyph's quad is centered on its ink
/// box; the whole text block is centered on the text node's origin.
fn expand_text_list(
    user_to_pixel: Transform,
    draws: Vec<Draw>,
    atlases: &mut HashMap<(u64, u32), text::Atlas>,
) -> Vec<Draw> {
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
                // validates the font up front, so this only guards a buffer
                // that turned out unreadable.
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
                // Keep the atlas in the map even when nothing was packed
                // (e.g. all-space text) so later frames hit it.
                let atlas = atlases.entry(key).or_insert(atlas);
                let [sx, sy] = world.scales();
                let aa = AA_BAND / sx.max(sy).max(1e-9);
                // Center the text block (width by ascent + descent, baseline
                // at the pen-space origin, y up) on the node's origin.
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
                    // The glyph's pen position in the node's local space,
                    // plus the ink box's center offset from the pen: `left`
                    // right and `top - height / 2` above the baseline.
                    let ink = [
                        origin[0] + glyph.x + cell.left as f32 + gw / 2.0,
                        origin[1] + glyph.y + cell.top as f32 - gh / 2.0,
                    ];
                    let glyph_world = Transform::translate(ink[0], ink[1])
                        .compose(&world)
                        .compose(&user_to_pixel);
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
    expanded
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
    expected_fps: Option<f32>,
    /// The cursor's position in user coordinates, or `None` when the cursor
    /// is outside the window (or has never moved into it).
    mouse: Option<[f32; 2]>,
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
}

impl Default for Config {
    /// Vsync on: the frame rate is capped at the display's refresh rate.
    fn default() -> Self {
        Self { vsync: true }
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
/// Use it to turn vsync off (for uncapped frame rates) or to inspect the
/// expected frame rate via [`Context::expected_fps`].
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
    let mut app = WebFrost::new(instance, scene, process, config.vsync);
    event_loop.run_app(&mut app)?;

    log::info!("event loop finished");
    Ok(())
}
