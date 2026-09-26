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
//! renderer: spawn particles in [`Process::process`] and advance them each
//! frame with [`ParticleSystem::update`] under a constant gravity. Draw
//! them by putting the system in the node's [`Shape::Particles`] shape —
//! the batch is then drawn in the node's local space, transformed,
//! scaled, and tinted by the node — or, for batches that live directly in
//! the window's user space, with the deprecated immediate
//! [`Canvas::particles`] draw; either way each particle fades by its
//! remaining lifetime.
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
//! [`Context::mouse_button_down`], and the connected gamepads by
//! [`Context::gamepads`]; Escape closes the window.
//!
//! On native targets, [`Sound`]s decoded at load time play through an
//! [`Audio`]: one-shots mix in parallel, and one sound loops at a time.
//!
//! A [`Diagnostics`] overlay reports the frame rate and the window size as
//! two left-aligned lines in the window's top-left corner: hold one in the
//! demo state and call its [`Process::process`] each frame.
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

/// The mouse buttons used by [`Context::mouse_button_down`], such as
/// `MouseButton::Left`.
pub use winit::event::MouseButton;

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
pub use diagnostics::Diagnostics;

mod rng;
pub use rng::*;

#[cfg(not(target_arch = "wasm32"))]
mod audio;
#[cfg(not(target_arch = "wasm32"))]
pub use audio::{Audio, AudioError, Sound};

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

    /// Draws every particle in `particles` as circles in one batched
    /// (instanced) draw call, in `color`, at draw order `z`.
    ///
    /// Each particle is a circle at its position with its `size` as radius,
    /// and its alpha is scaled by its remaining life fraction
    /// (`life / max_life`), so the batch fades out as its particles die.
    ///
    /// The whole batch shares one base `color` and one `z`; each particle's
    /// own `color` still multiplies with it, so per-particle tinting works
    /// here too.
    ///
    /// The positions are in the canvas's user space — the window, not any
    /// node: no transform, scale, or modulate of any scene node applies to
    /// the batch. To draw particles that follow a node (transformed,
    /// scaled, and tinted by it), give the node a
    /// [`Shape::Particles`] shape instead.
    #[deprecated(
        since = "0.1.0",
        note = "give the node a `Shape::Particles` shape instead: the batch is drawn in the node's local space, with the node's world transform, scale, and composed modulate applied. Keep this method only for batches that live directly in the window's user space, with no node to attach them to"
    )]
    pub fn particles(&mut self, particles: &[Particle], color: Color, z: f32) {
        if particles.is_empty() {
            return;
        }
        let mut data = Vec::with_capacity(particles.len() * 32);
        for p in particles {
            let [px, py] = self.user_to_pixels(p.pos[0], p.pos[1]);
            // Circles are rotationally symmetric, so the pixel-space angle
            // the fragment rotates out never matters; 0.0 keeps the packing
            // trivial.
            data.extend_from_slice(&particle_instance(p, px, py, p.size, 0.0));
        }
        self.draws.push(Draw::Particles {
            data,
            count: particles.len() as u32,
            color,
            kind: 0.0,
            aspect: 1.0,
            sprite_data: None,
            sprite_size: [0, 0],
            // Immediate canvas draws are unlit: the `lit` flag is a
            // scene-node property.
            lit: 0.0,
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

    /// Registers an omni light at `(x, y)` in the window's user space.
    ///
    /// Lights are never drawn: they join the frame's light field, which
    /// every lit receiver ([`SceneNode::lit`]) evaluates per pixel. `color`
    /// is the light's color, `intensity` its strength (`1.0` is full
    /// strength, `0.0` is off), and `radius` its falloff extent in pixels:
    /// the light reaches exactly `radius` pixels from `(x, y)`, fading to
    /// zero at the edge.
    ///
    /// An omni light shines in every direction; use [`Canvas::light_cone`]
    /// for one that shines only into a cone.
    ///
    /// The light lives in the window's user space — not any scene node's:
    /// no node's transform, scale, or modulate applies to it. For a light
    /// that rides a node (transformed and tinted by it), give the node a
    /// [`Shape::Light`] shape instead.
    ///
    /// The frame's light field is rebuilt from that frame's lights, so to
    /// move a light, call this again each frame with the new position, and
    /// the field's ambient floor comes from the scene's
    /// [`ambient`](Scene::ambient).
    pub fn light(&mut self, x: f32, y: f32, color: Color, intensity: f32, radius: f32) {
        self.light_cone(x, y, color, intensity, radius, 0.0, std::f32::consts::TAU);
    }

    /// Registers a cone light whose apex is at `(x, y)` in the window's
    /// user space.
    ///
    /// Like [`Canvas::light`], but the light shines only into a cone:
    /// `direction` is the cone's axis in radians (counter-clockwise from
    /// the window's +x axis, y-up), and `spread` is the cone's full
    /// opening angle in radians (`PI` is a half-plane, `>= 2 * PI` shines
    /// in every direction like [`Canvas::light`]). Outside the cone the
    /// light contributes nothing.
    ///
    /// This light lives in the window's user space like
    /// [`Canvas::light`]'s, so `direction` is also the axis as seen on
    /// screen; a light riding a [`SceneNode`] picks up the node's
    /// rotation — see [`Shape::Light`].
    #[allow(clippy::too_many_arguments)] // immediate-mode draw call
    pub fn light_cone(
        &mut self,
        x: f32,
        y: f32,
        color: Color,
        intensity: f32,
        radius: f32,
        direction: f32,
        spread: f32,
    ) {
        // Fold the direction through the user transform as a vector, not
        // an angle: subtracting the image of the origin from the image of
        // the unit vector's tip cancels the translation and survives the
        // y flip (user space is y-up, pixel space y-down) without special
        // casing it. Normalizing guards against a future scaled user
        // space; the fallback keeps a degenerate direction usable.
        let tip = self.user_to_pixels(direction.cos(), direction.sin());
        let origin = self.user_to_pixels(0.0, 0.0);
        self.draws.push(Draw::Light {
            pos: self.user_to_pixels(x, y),
            radius: radius.max(0.0),
            intensity,
            color,
            // Immediate-mode lights are point-sized: hard shadows.
            penumbra: 0.0,
            dir: normalized_axis([tip[0] - origin[0], tip[1] - origin[1]]),
            cos_half: half_angle_cosine(spread),
            // The immediate-mode cone has no softness knob: hard edge.
            feather: 0.0,
            // The light is never drawn, so its draw order is irrelevant.
            z: 0.0,
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
    /// before its children — when the shape is a
    /// [`Shape::Particles`] batch, it is transformed, scaled, and tinted by
    /// the node, at the node's composed order — so parents paint under
    /// their descendants.
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
                    &Transform::translate([ox, oy]).compose(&view),
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

/// Normalizes a pixel-space delta into a light cone's unit axis.
///
/// A delta with no usable length (zero, or a NaN from a NaN direction)
/// has no direction to speak of, so it falls back to the +x axis; the
/// cone gate then behaves normally around that axis.
fn normalized_axis(delta: [f32; 2]) -> [f32; 2] {
    let len = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
    if len > 0.0 && len.is_finite() {
        [delta[0] / len, delta[1] / len]
    } else {
        [1.0, 0.0]
    }
}

/// The cosine of a light cone's half opening angle, from its full
/// `spread` in radians. The half angle is clamped to `0..=PI` first so
/// the cosine covers the cone's whole meaningful range: a `spread` of
/// `2 * PI` or more yields `-1.0`, which every pixel passes (an omni
/// light), while a negative `spread` yields the degenerate axis-only
/// `1.0` like `0` does. A NaN `spread` yields NaN, which fails the cone
/// gate everywhere: the light simply contributes nothing instead of
/// corrupting the frame.
fn half_angle_cosine(spread: f32) -> f32 {
    (spread * 0.5).clamp(0.0, std::f32::consts::PI).cos()
}

/// The cone's edge feather, in cosine units, from its full `spread` and
/// its `softness` in radians: the width of the band just inside each cone
/// edge across which the shader ramps the light from zero to full, as the
/// cosine difference between the band's outer and inner angles — the
/// shader gates on cosines, so the feather ships in the same units. The
/// softness is clamped to the half angle, so the feather spans at most
/// the whole cone and the axis stays fully lit. An omni light
/// (`spread` at or beyond `2 * PI`) and a `0` or negative softness both
/// yield `0.0`: the hard edge, and never a feather on an omni light,
/// whose gate must stay the all-pass it has always been.
fn feather_band(spread: f32, softness: f32) -> f32 {
    if softness <= 0.0 || spread >= std::f32::consts::TAU {
        return 0.0;
    }
    let half = (spread * 0.5).clamp(0.0, std::f32::consts::PI);
    (half - softness.clamp(0.0, half)).cos() - half.cos()
}

/// Draws one [`SceneNode`] and its subtree into `draws`, depth-first: the
/// node's shape (if any — which may itself be a [`Shape::Particles`] batch)
/// before its children, so parents paint under their descendants.
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
    let local = Transform::scale(node.scale).compose(&node.transform);
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
            } => Some(Draw::Shape {
                world: world.compose(&user_to_pixel),
                center: *center,
                params: [(*radius).max(0.0), 0.0],
                kind: 0.0,
                aa,
                color: color.mul(modulate),
                glow: node.glow.mul(modulate),
                lit: f32::from(node.lit),
                occludes: f32::from(node.occludes),
                z: order,
            }),
            Shape::Rectangle {
                center,
                extent,
                color,
            } => Some(Draw::Shape {
                world: world.compose(&user_to_pixel),
                center: *center,
                params: [extent[0].max(0.0), extent[1].max(0.0)],
                kind: 1.0,
                aa,
                color: color.mul(modulate),
                glow: node.glow.mul(modulate),
                lit: f32::from(node.lit),
                occludes: f32::from(node.occludes),
                z: order,
            }),
            // The sprite's local space is centered on the origin, one
            // texture pixel per scene pixel, so its extent is the texture
            // size.
            Shape::Sprite {
                data,
                width,
                height,
                color,
                alpha,
            } => Some(Draw::Sprite {
                world: world.compose(&user_to_pixel),
                data: data.clone(),
                // A file's pixels are never replaced, so a static image's
                // buffer identity is the constant 0.
                generation: 0,
                size: [(*width as f32).max(0.0), (*height as f32).max(0.0)],
                texture_size: [*width, *height],
                aa,
                tint: color.mul(modulate),
                alpha: *alpha,
                glow: node.glow.mul(modulate),
                uv_rect: [0.0, 0.0, 1.0, 1.0],
                lit: f32::from(node.lit),
                z: order,
            }),
            // Text is recorded in user space; `Canvas::expand_text` lays it
            // out and turns each glyph into a sprite quad before the frame
            // is rendered.
            Shape::Text {
                text,
                font,
                size,
                color,
                alpha,
            } => Some(Draw::Text {
                world,
                font: font.clone(),
                text: text.clone(),
                size: *size,
                color: color.mul(modulate),
                alpha: *alpha,
                glow: node.glow.mul(modulate),
                lit: f32::from(node.lit),
                z: order,
            }),
            // The background ignores its transform: it is recorded in call
            // order and becomes the frame's clear color at render time.
            Shape::Background { color } => Some(Draw::Background {
                color: color.mul(modulate),
            }),
            // A light is never drawn: its position and cone axis are
            // recorded in pixel space (the node's local origin, and its
            // local cone direction, both through the composed transforms)
            // and it is promoted to the frame's light field at render time.
            Shape::Light { light } => {
                // The direction is a local-space angle, so it is folded
                // through the transform as a vector, not an angle: applying
                // the transform to the origin and to the unit vector's tip
                // and subtracting cancels the translation, carries the
                // rotation (and any mirror) into the axis, and needs no
                // atan2 that the pixel space's y flip would confuse.
                let to_pixel = world.compose(&user_to_pixel);
                let origin = to_pixel.apply([0.0, 0.0]);
                let tip = to_pixel.apply([light.direction.cos(), light.direction.sin()]);
                Some(Draw::Light {
                    pos: origin,
                    // A negative radius is the directional sentinel
                    // (`Light::directional`): clamping it to zero would
                    // silently turn a sun into a degenerate point light,
                    // so only real radii get clamped.
                    radius: if light.radius < 0.0 {
                        -1.0
                    } else {
                        light.radius.max(0.0)
                    },
                    intensity: light.intensity,
                    color: light.color.mul(modulate),
                    penumbra: light.penumbra.max(0.0),
                    dir: normalized_axis([tip[0] - origin[0], tip[1] - origin[1]]),
                    cos_half: half_angle_cosine(light.spread),
                    feather: feather_band(light.spread, light.softness),
                    z: order,
                })
            }
            // The particles ride the node's world transform and scale, are
            // tinted by `color` times the composed modulate (and each
            // particle's own color), and draw at the node's composed order —
            // one instanced draw, just after the node's own shape (if any)
            // and before its children.
            Shape::Particles {
                system,
                color,
                shape,
            } => {
                if system.is_empty() {
                    // Like an empty `Canvas::particles` call: nothing to
                    // draw.
                    None
                } else {
                    // The node's local space to the shader's pixel space,
                    // like the shapes' `world.compose(&user_to_pixel)`.
                    let to_pixel = world.compose(&user_to_pixel);
                    // The world scale as a uniform size factor: the
                    // geometric mean of the two axis scales — exact under a
                    // uniform scale, area-preserving under a non-uniform
                    // one, where the shape cannot be stretched
                    // independently along the two axes and so keeps its
                    // proportions.
                    let [sx, sy] = world.scales();
                    let radius_scale = (sx * sy).sqrt();
                    // The pixel-space angle of the node's local +x axis: the
                    // direction `to_pixel` maps `(1, 0)` to. A particle's own
                    // `angle` is measured in the local (y-up) frame, so it
                    // adds to that base angle when the mapping preserves
                    // orientation and subtracts from it when it flips the
                    // y axis (the usual case: the pixel space is y-down) —
                    // the flip turns a local counterclockwise turn into a
                    // pixel-space clockwise one.
                    let m = to_pixel.m;
                    let base = m[1][0].atan2(m[0][0]);
                    let flipped = (m[0][0] * m[1][1] - m[0][1] * m[1][0]) < 0.0;
                    let (sprite_data, sprite_size) = match shape {
                        ParticleShape::Sprite {
                            data,
                            width,
                            height,
                        } => (Some(data.clone()), [*width, *height]),
                        _ => (None, [0, 0]),
                    };
                    let mut data = Vec::with_capacity(system.len() * 32);
                    for p in &system.particles {
                        let [px, py] = to_pixel.apply(p.pos);
                        let angle = if flipped {
                            base - p.angle
                        } else {
                            base + p.angle
                        };
                        data.extend_from_slice(&particle_instance(
                            p,
                            px,
                            py,
                            p.size * radius_scale,
                            angle,
                        ));
                    }
                    Some(Draw::Particles {
                        data,
                        count: system.len() as u32,
                        color: color.mul(modulate),
                        kind: shape.kind(),
                        aspect: shape.aspect(),
                        sprite_data,
                        sprite_size,
                        lit: f32::from(node.lit),
                        z: order,
                    })
                }
            }
        };
        if let Some(draw) = draw {
            draws.push(draw);
        }
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
    let local = Transform::scale(node.scale).compose(&node.transform);
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
/// (`Shape::Background`) or cannot be measured (a broken font, or a live
/// particle batch, whose extent is dynamic).
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
        // A light never draws; nothing to measure.
        Shape::Light { .. } => None,
        // A particle batch has no fixed local box: its particles move,
        // spawn, and die every frame, so its extent is dynamic and cannot
        // be measured. A batch in a repeating layer therefore skips the
        // period-overflow check.
        Shape::Particles { .. } => None,
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
                glow,
                lit,
                z,
            } => {
                // A broken font leaves the text undrawn; `Shape::text`
                // validates the font up front, so this only guards a buffer
                // that turned out unreadable.
                let Some(layout) = text::layout(&font, &string, size) else {
                    continue;
                };
                let key = (Arc::as_ptr(&font) as *const () as u64, size.to_bits());
                // Pack the missing glyphs into the map's own atlas, not a
                // clone: a clone's new cells would be dropped, because the
                // key is already registered by the time the first new glyph
                // appears (an earlier draw or frame), and the glyphs would
                // never draw. `or_default` also keeps an atlas in the map
                // even when nothing was packed (e.g. all-space text) so
                // later frames hit it.
                let atlas = atlases.entry(key).or_default();
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
                    let glyph_world = Transform::translate(ink)
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
                        generation: atlas.generation(),
                        size: [gw, gh],
                        texture_size: [atlas.width, atlas.height],
                        aa,
                        tint: color,
                        alpha,
                        uv_rect,
                        // The glyphs are the text node's own pixels: its
                        // `lit` flag and `glow` pass on to every quad.
                        glow,
                        lit,
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
    /// The mouse buttons currently held down.
    mouse_buttons: &'c HashSet<MouseButton>,
    /// The gamepad controller, or `None` when gilrs could not open the
    /// platform's input devices at startup (then no gamepad state exists).
    gilrs: Option<&'c gilrs::Gilrs>,
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

    /// Whether the `button` mouse button is currently held down.
    ///
    /// The state is updated as mouse input events arrive, so it reflects
    /// every press and release since the previous frame. When the cursor
    /// leaves the window or the window loses focus, the held state is
    /// dropped, so a button can never appear stuck down.
    pub fn mouse_button_down(&self, button: MouseButton) -> bool {
        self.mouse_buttons.contains(&button)
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
    /// `None` keeps the platform default (winit's default window size, or
    /// the web canvas's 900x600 default). The user can resize the window
    /// afterwards; this only sets the size it opens at.
    pub window_size: Option<[u32; 2]>,
}

impl Default for Config {
    /// Vsync on, and the platform's default window size.
    fn default() -> Self {
        Self {
            vsync: true,
            window_size: None,
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
    let mut app = WebFrost::new(instance, scene, process, config.vsync, config.window_size);
    event_loop.run_app(&mut app)?;

    log::info!("event loop finished");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The user-to-pixel transform of a `(100, 100)` canvas:
    /// `(x, y) -> (x + 50, 50 - y)`.
    fn pixel_map() -> Transform {
        Transform::scale([1.0, -1.0]).compose(&Transform::translate([50.0, 50.0]))
    }

    /// A particle at `pos` with a full lifetime of `life` seconds (its
    /// `max_life` is its `life`) and the given `size`; unrotated and white.
    fn particle(pos: [f32; 2], life: f32, size: f32) -> Particle {
        Particle {
            pos,
            vel: [0.0, 0.0],
            life,
            max_life: life,
            size,
            angle: 0.0,
            color: WHITE,
        }
    }

    /// The fields of a particle batch draw.
    struct Batch<'a> {
        data: &'a [u8],
        count: u32,
        color: Color,
        z: f32,
    }

    /// The node's particle draw: panics if the node drew anything but
    /// exactly one particle batch.
    fn the_batch<'a>(draws: &'a [Draw]) -> Batch<'a> {
        match draws {
            [Draw::Particles {
                data,
                count,
                color,
                z,
                ..
            }] => Batch {
                data,
                count: *count,
                color: *color,
                z: *z,
            },
            other => panic!("expected one particle draw, got {other:?}"),
        }
    }

    /// Reads the little-endian float at component `component` (0..4 in the
    /// first instance `vec4`: position, size, life fraction; 4..7 in the
    /// second: pixel-space angle and tint) of particle `particle` in the
    /// packed instance data of the batch.
    fn instance_f32(batch: &Batch, particle: usize, component: usize) -> f32 {
        let start = particle * 32 + component * 4;
        f32::from_le_bytes(batch.data[start..start + 4].try_into().unwrap())
    }

    /// A node with one particle at `pos`, drawn alone at the canvas origin.
    fn draw_one(node: &SceneNode, draws: &mut Vec<Draw>) {
        draw_node(pixel_map(), node, &Transform::identity(), 0.0, WHITE, draws);
    }

    #[test]
    fn node_particles_follow_the_node_world_transform() {
        // A node translated to (10, 20) with a particle at local (5, 5):
        // the particle lands at user (15, 25), pixel (65, 25).
        let node = SceneNode {
            transform: Transform::translate([10.0, 20.0]),
            shape: Some(Shape::Particles {
                system: ParticleSystem {
                    particles: vec![particle([5.0, 5.0], 4.0, 2.0)],
                },
                color: WHITE,
                shape: ParticleShape::Circle,
            }),
            ..Default::default()
        };
        let mut draws = Vec::new();
        draw_one(&node, &mut draws);
        let batch = the_batch(&draws);
        assert_eq!(batch.count, 1);
        assert_eq!(instance_f32(&batch, 0, 0), 65.0);
        assert_eq!(instance_f32(&batch, 0, 1), 25.0);
        // The size is unscaled, the life fraction is one.
        assert_eq!(instance_f32(&batch, 0, 2), 2.0);
        assert_eq!(instance_f32(&batch, 0, 3), 1.0);
    }

    #[test]
    fn node_particles_rotate_with_the_node() {
        // A node rotated a quarter turn (counter-clockwise, y up) sends
        // local (10, 0) to (0, 10), pixel (50, 40); a rotation keeps the
        // scale factors at one, so the radius is unscaled.
        let node = SceneNode {
            transform: Transform::rotate(std::f32::consts::FRAC_PI_2),
            shape: Some(Shape::Particles {
                system: ParticleSystem {
                    particles: vec![particle([10.0, 0.0], 4.0, 3.0)],
                },
                color: WHITE,
                shape: ParticleShape::Circle,
            }),
            ..Default::default()
        };
        let mut draws = Vec::new();
        draw_one(&node, &mut draws);
        let batch = the_batch(&draws);
        assert_eq!(instance_f32(&batch, 0, 0), 50.0);
        assert_eq!(instance_f32(&batch, 0, 1), 40.0);
        assert_eq!(instance_f32(&batch, 0, 2), 3.0);
    }

    #[test]
    fn node_particles_scale_the_radius() {
        // A uniform world scale of 2 doubles the radius.
        let node = SceneNode {
            scale: [2.0, 2.0],
            shape: Some(Shape::Particles {
                system: ParticleSystem {
                    particles: vec![particle([0.0, 0.0], 4.0, 3.0)],
                },
                color: WHITE,
                shape: ParticleShape::Circle,
            }),
            ..Default::default()
        };
        let mut draws = Vec::new();
        draw_one(&node, &mut draws);
        let batch = the_batch(&draws);
        assert_eq!(instance_f32(&batch, 0, 2), 6.0);
    }

    #[test]
    fn node_particles_nonuniform_scale_preserves_area() {
        // A world scale of (2, 8) has a geometric mean of 4, so the radius
        // is multiplied by 4 — the particle's area is preserved.
        let node = SceneNode {
            scale: [2.0, 8.0],
            shape: Some(Shape::Particles {
                system: ParticleSystem {
                    particles: vec![particle([0.0, 0.0], 4.0, 3.0)],
                },
                color: WHITE,
                shape: ParticleShape::Circle,
            }),
            ..Default::default()
        };
        let mut draws = Vec::new();
        draw_one(&node, &mut draws);
        let batch = the_batch(&draws);
        assert_eq!(instance_f32(&batch, 0, 2), 12.0);
    }

    #[test]
    fn node_particles_take_the_shape_color_times_the_composed_modulate() {
        // A parent with modulate (0.5, 0.5, 0.5, 1) and order 2, a child
        // with a particle shape of base color (1, 1, 0.5, 1), modulate
        // (1, 0.5, 0.25, 1), and order 3: the composed modulate is
        // (0.5, 0.25, 0.125, 1), so the batch is tinted
        // (0.5, 0.25, 0.0625, 1) and draws at order 5.
        let mut parent = SceneNode {
            modulate: Color {
                r: 0.5,
                g: 0.5,
                b: 0.5,
                a: 1.0,
            },
            order: 2.0,
            ..Default::default()
        };
        let child = SceneNode {
            modulate: Color {
                r: 1.0,
                g: 0.5,
                b: 0.25,
                a: 1.0,
            },
            order: 3.0,
            shape: Some(Shape::Particles {
                system: ParticleSystem {
                    particles: vec![particle([0.0, 0.0], 4.0, 1.0)],
                },
                color: Color {
                    r: 1.0,
                    g: 1.0,
                    b: 0.5,
                    a: 1.0,
                },
                shape: ParticleShape::Circle,
            }),
            ..Default::default()
        };
        parent.children.push(Box::new(child));
        let mut draws = Vec::new();
        draw_one(&parent, &mut draws);
        let batch = the_batch(&draws);
        assert_eq!(
            batch.color,
            Color {
                r: 0.5,
                g: 0.25,
                b: 0.0625,
                a: 1.0
            }
        );
        assert_eq!(batch.z, 5.0);
    }

    #[test]
    fn node_particles_empty_system_draws_nothing() {
        // A particle shape with no live particles produces no draw.
        let node = SceneNode {
            shape: Some(Shape::Particles {
                system: ParticleSystem::new(),
                color: WHITE,
                shape: ParticleShape::Circle,
            }),
            ..Default::default()
        };
        let mut draws = Vec::new();
        draw_one(&node, &mut draws);
        assert!(draws.is_empty());
    }

    #[test]
    fn node_particles_paint_between_shape_and_children() {
        // A grandparent with a circle, its child with a particle shape, and
        // that child's child with a rectangle: the draws land in tree order
        // — grandparent's shape, then the particle batch, then the child's
        // shape — all at the same z, so the stable z-sort keeps exactly this
        // order.
        let grandparent_child = SceneNode {
            shape: Some(Shape::Rectangle {
                center: [0.0, 0.0],
                extent: [2.0, 2.0],
                color: Color {
                    r: 0.0,
                    g: 0.0,
                    b: 1.0,
                    a: 1.0,
                },
            }),
            ..Default::default()
        };
        let mut parent = SceneNode {
            shape: Some(Shape::Particles {
                system: ParticleSystem {
                    particles: vec![particle([0.0, 0.0], 4.0, 1.0)],
                },
                color: WHITE,
                shape: ParticleShape::Circle,
            }),
            ..Default::default()
        };
        parent.children.push(Box::new(grandparent_child));
        let mut grandparent = SceneNode {
            shape: Some(Shape::Circle {
                center: [0.0, 0.0],
                radius: 4.0,
                color: Color {
                    r: 1.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                },
            }),
            ..Default::default()
        };
        grandparent.children.push(Box::new(parent));
        let mut draws = Vec::new();
        draw_one(&grandparent, &mut draws);
        assert_eq!(draws.len(), 3);
        assert!(matches!(&draws[0], Draw::Shape { .. }));
        assert!(matches!(&draws[1], Draw::Particles { .. }));
        assert!(matches!(&draws[2], Draw::Shape { .. }));
        // The whole subtree drew at the composed order 0.
        for draw in &draws {
            assert_eq!(draw.z(), 0.0);
        }
    }

    /// A node at the canvas origin whose shape is a particle batch with the
    /// given per-particle `angle` and `color`.
    fn particle_node(angle: f32, color: Color, shape: ParticleShape) -> SceneNode {
        SceneNode {
            shape: Some(Shape::Particles {
                system: ParticleSystem {
                    particles: vec![Particle {
                        pos: [0.0, 0.0],
                        vel: [0.0, 0.0],
                        life: 4.0,
                        max_life: 4.0,
                        size: 1.0,
                        angle,
                        color,
                    }],
                },
                color: WHITE,
                shape,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn node_particles_pack_their_angle_and_color_in_pixel_space() {
        // The pixel space is y-down, so a particle's local (y-up,
        // counterclockwise-positive) angle packs negated: a quarter turn
        // counterclockwise on screen is -FRAC_PI_4 in pixel space, and a
        // clockwise quarter turn is +FRAC_PI_4. The particle's own color
        // packs as the tint in the second instance vec4.
        let color = Color {
            r: 0.5,
            g: 1.0,
            b: 0.25,
            a: 1.0,
        };
        for phi in [std::f32::consts::FRAC_PI_4, -std::f32::consts::FRAC_PI_4] {
            let node = particle_node(phi, color, ParticleShape::Circle);
            let mut draws = Vec::new();
            draw_one(&node, &mut draws);
            let batch = the_batch(&draws);
            assert_eq!(instance_f32(&batch, 0, 4), -phi);
            assert_eq!(instance_f32(&batch, 0, 5), color.r);
            assert_eq!(instance_f32(&batch, 0, 6), color.g);
            assert_eq!(instance_f32(&batch, 0, 7), color.b);
        }
    }

    #[test]
    fn node_particles_add_the_node_rotation_to_their_own_angle() {
        // A node rotated a quarter turn counterclockwise maps its local +x
        // axis to pixel angle -FRAC_PI_2 (y down), so a particle with its
        // own angle phi packs at -FRAC_PI_2 - phi: the node's rotation and
        // the particle's rotation compose in the local frame before the
        // y-down flip is applied.
        let phi = std::f32::consts::FRAC_PI_4;
        let mut node = particle_node(phi, WHITE, ParticleShape::Circle);
        node.transform = Transform::rotate(std::f32::consts::FRAC_PI_2);
        let mut draws = Vec::new();
        draw_one(&node, &mut draws);
        let batch = the_batch(&draws);
        let packed = instance_f32(&batch, 0, 4);
        let expected = -(std::f32::consts::FRAC_PI_2 + phi);
        assert!((packed - expected).abs() < 1e-4);
    }

    #[test]
    fn node_particles_pass_the_shape_kind_aspect_and_sprite_to_the_draw() {
        // The batch's uniform carries the shape's kind and aspect, and a
        // sprite shape hands its image and size to the draw so the pipeline
        // can bind (or cache) the texture.
        let node = particle_node(
            0.0,
            WHITE,
            ParticleShape::Rectangle { aspect: 2.0 },
        );
        let mut draws = Vec::new();
        draw_one(&node, &mut draws);
        let [Draw::Particles {
            kind,
            aspect,
            sprite_data,
            sprite_size,
            ..
        }] = &draws[..]
        else {
            panic!("expected one particle draw, got {draws:?}")
        };
        assert_eq!(*kind, 1.0);
        assert_eq!(*aspect, 2.0);
        assert!(sprite_data.is_none());
        assert_eq!(*sprite_size, [0, 0]);

        let sprite = ParticleShape::Sprite {
            data: std::sync::Arc::from([255u8, 255, 255, 255]),
            width: 2,
            height: 3,
        };
        let node = particle_node(0.0, WHITE, sprite);
        let mut draws = Vec::new();
        draw_one(&node, &mut draws);
        let [Draw::Particles {
            kind,
            aspect,
            sprite_data,
            sprite_size,
            ..
        }] = &draws[..]
        else {
            panic!("expected one particle draw, got {draws:?}")
        };
        assert_eq!(*kind, 2.0);
        assert_eq!(*aspect, 1.5);
        assert_eq!(
            sprite_data.as_deref(),
            Some(&[255u8, 255, 255, 255][..])
        );
        assert_eq!(*sprite_size, [2, 3]);
    }

    #[test]
    fn canvas_light_lands_at_the_pixel_position_with_its_fields() {
        // A light registered at user (10, 20) on a 100x100 canvas lands at
        // pixel (60, 30); a negative radius clamps to zero, the intensity
        // and color pass through, and the draw order is irrelevant, so it
        // stays 0.0. The light is omni, so its cone is wide open: the
        // cosine of the half angle is -1.0 and the axis is the fallback.
        let mut canvas = Canvas::new((100, 100));
        canvas.light(
            10.0,
            20.0,
            Color {
                r: 1.0,
                g: 0.5,
                b: 0.0,
                a: 1.0,
            },
            2.0,
            -16.0,
        );
        let [Draw::Light {
            pos,
            radius,
            intensity,
            color,
            penumbra,
            dir,
            cos_half,
            feather,
            z,
        }] = &canvas.draws[..]
        else {
            panic!("expected one light draw, got {:?}", canvas.draws);
        };
        assert_eq!(*pos, [60.0, 30.0]);
        assert_eq!(*radius, 0.0);
        assert_eq!(*intensity, 2.0);
        assert_eq!(*color, Color { r: 1.0, g: 0.5, b: 0.0, a: 1.0 });
        assert_eq!(*penumbra, 0.0);
        assert_eq!(*dir, [1.0, 0.0]);
        assert_eq!(*cos_half, -1.0);
        assert_eq!(*feather, 0.0);
        assert_eq!(*z, 0.0);
    }

    #[test]
    fn immediate_and_scene_lights_pack_into_the_field_in_call_order() {
        // A light registered immediately and one on a scene node, both at z
        // 0: the immediate draw is pushed first (the process runs before
        // the scene is drawn), so the stable z-sort keeps it first in the
        // paint order, and the packed field's records follow.
        let mut canvas = Canvas::new((100, 100));
        canvas.light(
            0.0,
            0.0,
            Color {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            1.0,
            10.0,
        );
        let scene = Scene::new(SceneNode {
            transform: Transform::translate([-10.0, 0.0]),
            shape: Some(Shape::Light {
                light: Light::point(
                    Color {
                        r: 0.0,
                        g: 0.0,
                        b: 1.0,
                        a: 1.0,
                    },
                    0.5,
                    20.0,
                ),
            }),
            ..Default::default()
        });
        canvas.draw_scene(&scene);
        let field = pack_light_field(&canvas.paint_order(), AMBIENT);
        let f32_at = |data: &[u8], off: usize| {
            f32::from_le_bytes(data[off..off + 4].try_into().unwrap())
        };
        assert_eq!(field.count, 2);
        assert_eq!(field.data.len(), LIGHT_FIELD_HEADER + 2 * LIGHT_RECORD);
        // The header carries the light count and the scene's ambient.
        let count = u32::from_le_bytes(field.data[0..4].try_into().unwrap());
        assert_eq!(count, 2);
        assert_eq!(f32_at(&field.data, 16), AMBIENT.r);
        // The immediate light first: user (0, 0) -> pixel (50, 50), radius
        // 10, intensity 1, red. Both lights are omni, so the cone vec4 is
        // the fallback axis, the always-passing -1.0, and zero padding;
        // the color vec4's w is the penumbra, zero for point-sized
        // immediate lights.
        let off = LIGHT_FIELD_HEADER;
        assert_eq!(f32_at(&field.data, off), 50.0);
        assert_eq!(f32_at(&field.data, off + 4), 50.0);
        assert_eq!(f32_at(&field.data, off + 8), 10.0);
        assert_eq!(f32_at(&field.data, off + 12), 1.0);
        assert_eq!(f32_at(&field.data, off + 16), 1.0);
        assert_eq!(f32_at(&field.data, off + 28), 0.0);
        assert_eq!(f32_at(&field.data, off + 32), 1.0);
        assert_eq!(f32_at(&field.data, off + 36), 0.0);
        assert_eq!(f32_at(&field.data, off + 40), -1.0);
        assert_eq!(f32_at(&field.data, off + 44), 0.0);
        // The scene's light second: user (-10, 0) -> pixel (40, 50), radius
        // 20, intensity 0.5, blue.
        let off = LIGHT_FIELD_HEADER + LIGHT_RECORD;
        assert_eq!(f32_at(&field.data, off), 40.0);
        assert_eq!(f32_at(&field.data, off + 4), 50.0);
        assert_eq!(f32_at(&field.data, off + 8), 20.0);
        assert_eq!(f32_at(&field.data, off + 12), 0.5);
        assert_eq!(f32_at(&field.data, off + 24), 1.0);
        assert_eq!(f32_at(&field.data, off + 32), 1.0);
        assert_eq!(f32_at(&field.data, off + 36), 0.0);
        assert_eq!(f32_at(&field.data, off + 40), -1.0);
        assert_eq!(f32_at(&field.data, off + 44), 0.0);
    }

    #[test]
    fn light_node_lands_at_the_pixel_position_with_its_fields_intact() {
        // A light node translated to (10, 20): the light sits at the node's
        // local origin, which lands at user (10, 20), pixel (60, 30) on a
        // 100x100 canvas; the radius, intensity, and color pass through
        // untinted (the modulate is white), the point light's cone is wide
        // open, and the node's order becomes the draw's z.
        let node = SceneNode {
            transform: Transform::translate([10.0, 20.0]),
            order: 3.0,
            shape: Some(Shape::Light {
                light: Light::point(
                    Color {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                    2.0,
                    16.0,
                ),
            }),
            ..Default::default()
        };
        let mut draws = Vec::new();
        draw_one(&node, &mut draws);
        let [Draw::Light {
            pos,
            radius,
            intensity,
            color,
            penumbra,
            dir,
            cos_half,
            feather,
            z,
        }] = &draws[..]
        else {
            panic!("expected one light draw, got {draws:?}")
        };
        assert_eq!(*pos, [60.0, 30.0]);
        assert_eq!(*radius, 16.0);
        assert_eq!(*intensity, 2.0);
        assert_eq!(
            *color,
            Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0
            }
        );
        assert_eq!(*penumbra, 0.0);
        assert_eq!(*dir, [1.0, 0.0]);
        assert_eq!(*cos_half, -1.0);
        assert_eq!(*feather, 0.0);
        assert_eq!(*z, 3.0);
    }

    #[test]
    fn light_color_is_tinted_by_the_composed_modulate() {
        // A red light under a half-strength green parent modulate: the
        // light's color is multiplied channel by channel, and a negative
        // radius is clamped to zero.
        let node = SceneNode {
            transform: Transform::identity(),
            shape: Some(Shape::Light {
                light: Light::point(
                    Color {
                        r: 1.0,
                        g: 1.0,
                        b: 0.0,
                        a: 1.0,
                    },
                    1.0,
                    -4.0,
                ),
            }),
            ..Default::default()
        };
        let mut draws = Vec::new();
        draw_node(
            pixel_map(),
            &node,
            &Transform::identity(),
            5.0,
            Color {
                r: 1.0,
                g: 0.5,
                b: 0.0,
                a: 1.0,
            },
            &mut draws,
        );
        let [Draw::Light {
            radius,
            color,
            z,
            ..
        }] = &draws[..]
        else {
            panic!("expected one light draw, got {draws:?}")
        };
        assert_eq!(*radius, 0.0);
        assert_eq!(
            *color,
            Color {
                r: 1.0,
                g: 0.5,
                b: 0.0,
                a: 1.0
            }
        );
        assert_eq!(*z, 5.0);
    }

    #[test]
    fn a_cone_light_records_its_axis_and_half_angle() {
        // An immediate cone aimed at user 45 degrees with a 60-degree
        // opening: the axis is folded through the canvas transform, so the
        // y-up user vector (cos 45, sin 45) becomes the y-down pixel vector
        // (0.707, -0.707), and the gate carries the cosine of the half
        // opening, cos(30 degrees).
        let mut canvas = Canvas::new((100, 100));
        canvas.light_cone(
            0.0,
            0.0,
            Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            1.0,
            10.0,
            std::f32::consts::FRAC_PI_4,
            std::f32::consts::FRAC_PI_3,
        );
        let [Draw::Light { dir, cos_half, .. }] = &canvas.draws[..] else {
            panic!("expected one light draw, got {:?}", canvas.draws);
        };
        let k = std::f32::consts::FRAC_1_SQRT_2;
        assert!((dir[0] - k).abs() < 1e-6);
        assert!((dir[1] + k).abs() < 1e-6);
        assert!((cos_half - (std::f32::consts::FRAC_PI_6).cos()).abs() < 1e-6);
    }

    #[test]
    fn a_directional_light_records_the_sentinel_and_its_travel() {
        // A `Light::directional` aimed at user PI — travelling toward
        // -x: the record keeps the negative radius sentinel exactly (the
        // zero-clamp that tames stray point radii must not demote a sun
        // to a zero-size point), the travel direction folds through the
        // canvas transform into the pixel-space vector (-1, 0), and the
        // gate opens fully like an omni light's, so nothing but the
        // shader's directional branch distinguishes it downstream.
        let node = SceneNode {
            shape: Some(Shape::Light {
                light: Light::directional(
                    Color {
                        r: 1.0,
                        g: 0.5,
                        b: 0.2,
                        a: 1.0,
                    },
                    1.25,
                    std::f32::consts::PI,
                ),
            }),
            ..Default::default()
        };
        let mut draws = Vec::new();
        draw_node(
            pixel_map(),
            &node,
            &Transform::identity(),
            0.0,
            Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            &mut draws,
        );
        let [Draw::Light {
            radius,
            intensity,
            dir,
            cos_half,
            feather,
            ..
        }] = &draws[..]
        else {
            panic!("expected one light draw, got {draws:?}")
        };
        assert_eq!(*radius, -1.0);
        assert_eq!(*intensity, 1.25);
        assert!((dir[0] + 1.0).abs() < 1e-6);
        assert!(dir[1].abs() < 1e-6);
        assert!((cos_half + 1.0).abs() < 1e-6);
        assert_eq!(*feather, 0.0);
    }

    #[test]
    fn a_parent_rotation_turns_a_cone_axis_like_any_other_shape() {
        // A cone node aimed at local 0 degrees under a parent rotated 90
        // degrees: the axis is carried by the composed transform, so the
        // world-space direction is the node transform applied to (1, 0),
        // rotated into the canvas's y-down pixel space. With a quarter turn
        // this is exactly what a point's displacement would do.
        let node = SceneNode {
            transform: Transform::rotate(std::f32::consts::FRAC_PI_2),
            shape: Some(Shape::Light {
                light: Light::cone(
                    Color {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                    1.0,
                    10.0,
                    0.0,
                    std::f32::consts::FRAC_PI_2,
                    0.0,
                ),
            }),
            ..Default::default()
        };
        let mut draws = Vec::new();
        draw_one(&node, &mut draws);
        let [Draw::Light { dir, cos_half, .. }] = &draws[..] else {
            panic!("expected one light draw, got {draws:?}")
        };
        assert!(dir[0].abs() < 1e-5);
        assert!((dir[1] + 1.0).abs() < 1e-5);
        assert!((cos_half - (std::f32::consts::FRAC_PI_4).cos()).abs() < 1e-6);
    }

    #[test]
    fn a_degenerate_cone_falls_back_to_a_usable_axis() {
        // A degenerate axis - the delta the fold produces is zero-length -
        // must not reach the shader as NaN, which would poison the angular
        // gate for every pixel. The axis falls back to +x; the wide-open
        // spread passes everywhere regardless.
        let mut canvas = Canvas::new((100, 100));
        canvas.light_cone(
            0.0,
            0.0,
            Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            1.0,
            10.0,
            f32::NAN,
            std::f32::consts::TAU,
        );
        let [Draw::Light { dir, cos_half, .. }] = &canvas.draws[..] else {
            panic!("expected one light draw, got {:?}", canvas.draws);
        };
        assert_eq!(*dir, [1.0, 0.0]);
        assert_eq!(*cos_half, -1.0);
    }

    #[test]
    fn feather_band_measures_the_soft_cone_edge() {
        // The feather is the cosine difference between the band's inner
        // and outer angles: full softness (the half angle) feathers the
        // whole cone, half softness feathers half the cosine range, and
        // zero — or a clamped negative — keeps the hard edge.
        let half = std::f32::consts::FRAC_PI_4; // spread = PI/2.
        let spread = 2.0 * half;
        assert_eq!(feather_band(spread, 0.0), 0.0);
        assert_eq!(feather_band(spread, -1.0), 0.0);
        let full = feather_band(spread, half);
        assert!((full - (1.0 - half.cos())).abs() < 1e-6);
        // Softness beyond the half angle clamps to the whole cone.
        assert_eq!(feather_band(spread, half * 10.0), full);
        // Half the cone feathered: cos(PI/8) - cos(PI/4), strictly
        // between zero and the full feather.
        let soft = feather_band(spread, half * 0.5);
        assert!((soft - (std::f32::consts::PI / 8.0).cos() + half.cos()).abs() < 1e-6);
        assert!(soft > 0.0 && soft < full);
        // Omni lights never feather, whatever the softness says.
        assert_eq!(feather_band(std::f32::consts::TAU, 1.0), 0.0);
        assert_eq!(feather_band(std::f32::consts::TAU * 2.0, 1.0), 0.0);
    }

    #[test]
    fn a_nodes_glow_reaches_its_shape_draw() {
        // The node's glow rides to the draw multiplied by the composed
        // modulate, exactly like the shape's own color.
        let node = SceneNode {
            lit: true,
            glow: Color {
                r: 0.5,
                g: 0.2,
                b: 0.0,
                a: 1.0,
            },
            shape: Some(Shape::Circle {
                center: [0.0, 0.0],
                radius: 4.0,
                color: Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
            }),
            ..Default::default()
        };
        let mut draws = Vec::new();
        draw_node(
            pixel_map(),
            &node,
            &Transform::identity(),
            0.0,
            Color {
                r: 0.5,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            &mut draws,
        );
        let [Draw::Shape { glow, .. }] = &draws[..] else {
            panic!("expected one shape draw, got {draws:?}")
        };
        assert_eq!(
            *glow,
            Color {
                r: 0.25,
                g: 0.2,
                b: 0.0,
                a: 1.0,
            }
        );
    }

    #[test]
    fn a_soft_cone_carries_its_feather_and_penumbra_into_the_draw() {
        // The scene path feeds the light's softness into the draw's
        // feather band and its penumbra through untouched (negatives
        // clamp to zero); a hard cone (and the immediate-mode API, which
        // has no knobs) stay at zero.
        let half = std::f32::consts::FRAC_PI_4;
        let node = SceneNode {
            shape: Some(Shape::Light {
                light: Light::cone(
                    Color {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                    1.0,
                    10.0,
                    0.0,
                    2.0 * half,
                    half * 0.5,
                )
                .with_penumbra(4.0),
            }),
            ..Default::default()
        };
        let mut draws = Vec::new();
        draw_one(&node, &mut draws);
        let [Draw::Light {
            feather, penumbra, ..
        }] = &draws[..]
        else {
            panic!("expected one light draw, got {draws:?}")
        };
        assert_eq!(*feather, feather_band(2.0 * half, half * 0.5));
        assert!(*feather > 0.0);
        assert_eq!(*penumbra, 4.0);
        // The builder clamps a negative radius to the hard shadow.
        let light = Light::point(
            Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            1.0,
            10.0,
        )
        .with_penumbra(-3.0);
        assert_eq!(light.penumbra, 0.0);
    }
}
