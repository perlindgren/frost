//! The scene objects: the [`Transform`], [`Color`], [`Light`], [`Shape`],
//! [`SceneNode`], and [`Scene`] types that make up the scene tree drawn by
//! frost.

use std::path::Path;
use std::sync::Arc;

use crate::particles::ParticleSystem;

/// An RGBA color with channels in `0.0..=1.0`, used for the background and
/// for the color of each drawn object.
///
/// The alpha channel is the color's opacity: shapes emit
/// `coverage * a`, and sprites and text emit `texture_alpha * opacity * a`
/// (their separate `alpha` field and the color's alpha multiply). `1.0`
/// (opaque) is the default for the colors created by [`Shape::sprite`] and
/// [`Shape::text`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    /// Red channel.
    pub r: f32,
    /// Green channel.
    pub g: f32,
    /// Blue channel.
    pub b: f32,
    /// Alpha channel: the color's opacity. `1.0` is opaque.
    pub a: f32,
}

impl Color {
    /// The channels as `[r, g, b, a]`.
    pub(crate) fn channels(&self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Multiplies this color by `other` channel by channel.
    pub(crate) fn mul(&self, other: Color) -> Color {
        Color {
            r: self.r * other.r,
            g: self.g * other.g,
            b: self.b * other.b,
            a: self.a * other.a,
        }
    }

    /// The linear interpolation from `self` toward `other` at fraction
    /// `t`, channel by channel: `self` at `t = 0.0` and `other` at
    /// `t = 1.0`.
    pub fn lerp(&self, other: Color, t: f32) -> Color {
        Color {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }
}

/// White at full opacity: the identity for channel-wise multiplication, the
/// [`SceneNode::modulate`] that leaves colors unchanged.
pub(crate) const WHITE: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};

/// The default ambient color: a dim neutral gray, the floor of a scene's
/// light field where no light reaches. [`Scene::new`] and
/// [`Scene::default`] start from it, and a scene's
/// [`ambient`](Scene::ambient) field can be set to it (or to any other
/// color) to change the floor of every lit receiver.
pub const AMBIENT: Color = Color {
    r: 0.3,
    g: 0.3,
    b: 0.3,
    a: 1.0,
};

/// A 2D affine transform: `p' = m * p + t`.
///
/// The rows of `m` are `m[0]` and `m[1]`, so a point `(x, y)` maps to
/// `(m[0][0]*x + m[0][1]*y + t[0], m[1][0]*x + m[1][1]*y + t[1])`. Rotation
/// angles are measured from the +x axis toward +y, i.e. counter-clockwise in
/// the window-centered y-up coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    pub(crate) m: [[f32; 2]; 2],
    pub(crate) t: [f32; 2],
}

impl Transform {
    /// The identity transform (no change).
    pub const fn identity() -> Self {
        Self {
            m: [[1.0, 0.0], [0.0, 1.0]],
            t: [0.0, 0.0],
        }
    }

    /// A translation by `offset`, `[x, y]` in y-up user units.
    pub const fn translate(offset: [f32; 2]) -> Self {
        Self {
            m: [[1.0, 0.0], [0.0, 1.0]],
            t: offset,
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

    /// A scale by `scale[0]` horizontally and `scale[1]` vertically.
    pub const fn scale(scale: [f32; 2]) -> Self {
        Self {
            m: [[scale[0], 0.0], [0.0, scale[1]]],
            t: [0.0, 0.0],
        }
    }

    /// A uniform scale by `s`.
    pub const fn scale_uniform(s: f32) -> Self {
        Self::scale([s, s])
    }

    /// The node-local coordinates of an image anchor: the offset from a
    /// sprite's center to the anchor pixel, for a sprite that is centered
    /// on its node's origin.
    ///
    /// Image pixels are numbered from `(0, 0)` at the upper-left, `y`
    /// down; node-local coordinates have the image's center on the node's
    /// origin, `y` up. So the anchor at image pixels `(ax, ay)` of an
    /// image of `size` pixels is the offset `(ax - size[0] / 2,
    /// size[1] / 2 - ay)`, the y flip included.
    pub const fn anchor(anchor: [f32; 2], size: [f32; 2]) -> [f32; 2] {
        [anchor[0] - size[0] / 2.0, size[1] / 2.0 - anchor[1]]
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

    /// This transform with its translation scaled by `f`: the same linear
    /// part, and the translation `(f * t[0], f * t[1])`.
    pub(crate) fn scaled_translation(&self, f: f32) -> Transform {
        Transform {
            m: self.m,
            t: [self.t[0] * f, self.t[1] * f],
        }
    }

    /// The inverse transform, or `None` if the transform is degenerate (its
    /// linear part collapses points onto a line or a point).
    pub(crate) fn invert(&self) -> Option<Transform> {
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
    pub(crate) fn scales(&self) -> [f32; 2] {
        [
            (self.m[0][0] * self.m[0][0] + self.m[1][0] * self.m[1][0]).sqrt(),
            (self.m[0][1] * self.m[0][1] + self.m[1][1] * self.m[1][1]).sqrt(),
        ]
    }
}

/// The geometry of each particle in a [`Shape::Particles`] batch: one shape
/// for the whole batch, shared by every particle.
///
/// The shape's extent is scaled by each particle's own
/// [`Particle::size`](crate::Particle::size) (its half-width), its
/// orientation by each particle's own
/// [`Particle::angle`](crate::Particle::angle), and its color by the
/// particle's own [`Particle::color`](crate::Particle::color) — the batch
/// sets *what* each particle looks like,
/// while the particles keep their individual position, scale, rotation, and
/// tint.
#[derive(Clone, Debug, Default)]
pub enum ParticleShape {
    /// A filled circle with the particle's `size` as its radius.
    #[default]
    Circle,
    /// A filled rectangle, `size * 2` wide and `size * aspect * 2` tall,
    /// centered on the particle.
    Rectangle {
        /// The rectangle's height-to-width ratio; `1.0` is a square.
        aspect: f32,
    },
    /// A sprite image: each particle draws the whole image, scaled so its
    /// width is `2 * size`, and centered on the particle — the texture's
    /// own center `(1/2, 1/2)` sits on the particle's position.
    ///
    /// Created by [`ParticleShape::sprite`] and [`ParticleShape::sprite_bytes`];
    /// the pixels are decoded up front and shared behind an [`Arc`], like a
    /// [`Shape::Sprite`].
    Sprite {
        /// The RGBA8 pixel data, row by row, top row first.
        data: Arc<[u8]>,
        /// The texture width in pixels.
        width: u32,
        /// The texture height in pixels.
        height: u32,
    },
}

impl ParticleShape {
    /// Creates a sprite particle shape from the PNG file at `path`.
    ///
    /// The file is read and decoded to RGBA8 immediately, so a missing file
    /// or a non-PNG file fails here, not at render time. The pixels live
    /// behind an [`Arc`], so cloning a scene containing the shape is cheap:
    /// all the clones share the same buffer.
    pub fn sprite(path: impl AsRef<Path>) -> Result<Self, SpriteError> {
        let bytes = std::fs::read(path.as_ref()).map_err(SpriteError::Io)?;
        Self::sprite_bytes(&bytes)
    }

    /// Creates a sprite particle shape from PNG data already in memory, for
    /// example bytes embedded into the binary with `include_bytes!`.
    ///
    /// Like [`ParticleShape::sprite`], the bytes are decoded to RGBA8 up
    /// front, so a non-PNG buffer fails here, not at render time, and the
    /// pixels live behind an [`Arc`], but no file is read — this is how
    /// sprite shapes are created in environments without a file system,
    /// such as a web browser.
    pub fn sprite_bytes(bytes: impl AsRef<[u8]>) -> Result<Self, SpriteError> {
        let image = image::load_from_memory(bytes.as_ref()).map_err(SpriteError::Decode)?;
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        let data = Arc::from(rgba.into_raw().into_boxed_slice());
        Ok(Self::Sprite {
            data,
            width,
            height,
        })
    }

    /// The shape's kind as the render pipeline reads it: `0.0` for a
    /// circle, `1.0` for a rectangle, `2.0` for a sprite.
    pub(crate) fn kind(&self) -> f32 {
        match self {
            Self::Circle => 0.0,
            Self::Rectangle { .. } => 1.0,
            Self::Sprite { .. } => 2.0,
        }
    }

    /// The shape's height-to-width factor: `1.0` for a circle, the given
    /// `aspect` for a rectangle, and the texture's height over its width for
    /// a sprite.
    pub(crate) fn aspect(&self) -> f32 {
        match self {
            Self::Circle => 1.0,
            Self::Rectangle { aspect } => *aspect,
            Self::Sprite { width, height, .. } => {
                (*height as f32).max(1.0) / (*width as f32).max(1.0)
            }
        }
    }
}

/// A light source: the color, strength, falloff extent, and cone of a
/// light, held by a [`Shape::Light`] node.
///
/// A light is never drawn: it contributes to the frame's light field, and
/// every lit receiver ([`SceneNode::lit`]) evaluates it per pixel at render
/// time. The node carrying the light provides the position: the light sits
/// at the node's local origin, so the node's transform and its ancestors'
/// place it in the scene, and the node's composed modulate tints its color,
/// exactly as they would tint a shape's.
///
/// By default a light is omni-directional — it lights every pixel within
/// `radius`. Setting a narrower [`spread`](Self::spread) turns it into a
/// cone: only pixels inside the cone receive its light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    /// The light's color, multiplied with its intensity and with the
    /// falloff at each lit pixel.
    pub color: Color,
    /// The light's strength, multiplied with its color: `1.0` is full
    /// strength, `0.0` is off.
    pub intensity: f32,
    /// The falloff extent, in user units (pixels): the light reaches
    /// exactly `radius` pixels from its position, fading to zero at the
    /// edge.
    pub radius: f32,
    /// The direction the light points, in radians, counterclockwise from
    /// the local +x axis, measured in the node's local (y-up) space: like
    /// every other direction on a node, the node's transform and its
    /// ancestors' rotate it along with the position, so rotating the node
    /// turns the beam. Ignored when the light is omni-directional
    /// ([`spread`](Self::spread) at or beyond [`TAU`](std::f32::consts::TAU)).
    pub direction: f32,
    /// The cone's full opening angle, in radians: the angle between the
    /// cone's two edges, measured around [`direction`](Self::direction).
    /// `TAU` (or more) is omni-directional — the light reaches every
    /// direction, as if there were no cone at all; [`PI`](std::f32::consts::PI)
    /// is a half-plane; smaller angles narrow the beam; `0.0` degenerates
    /// to the axis alone.
    pub spread: f32,
}

impl Light {
    /// An omni-directional point light: full-circle coverage, lighting
    /// every direction equally.
    pub fn point(color: Color, intensity: f32, radius: f32) -> Self {
        Self {
            color,
            intensity,
            radius,
            // The direction is irrelevant for a full circle; +x keeps the
            // recorded cone axis defined.
            direction: 0.0,
            spread: std::f32::consts::TAU,
        }
    }

    /// A cone (spot) light: a beam of total opening angle `spread`
    /// radians, pointing `direction` radians counterclockwise from the
    /// local +x axis in the node's local space. See the
    /// [`direction`](Self::direction) and [`spread`](Self::spread) fields.
    pub fn cone(color: Color, intensity: f32, radius: f32, direction: f32, spread: f32) -> Self {
        Self {
            color,
            intensity,
            radius,
            direction,
            spread,
        }
    }
}

/// A shape that a [`SceneNode`] can hold.
///
/// A shape is a filled geometric shape drawn in the node's local space — a
/// circle, a rectangle, a sprite, text, or a particle batch — or a
/// [`Shape::Light`], which is never drawn and only lights the scene.
///
/// Coordinates and sizes are in the node's local space, in pixels; the
/// composed transforms of every ancestor apply to the shape, except for
/// [`Shape::Background`], which fills the whole window regardless of
/// transform.
#[derive(Clone, Debug)]
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
    /// A sprite: the RGBA pixels of a PNG image, created by
    /// [`Shape::sprite`].
    ///
    /// The sprite is centered on the node's origin, one texture pixel per
    /// scene pixel, and the node's transform and scale apply to it exactly as
    /// they do to the other shapes. `color` is a per-pixel tint multiplied
    /// with every sampled pixel; white (`r: 1.0, g: 1.0, b: 1.0, a: 1.0`)
    /// leaves the texture unchanged. The texture's own alpha channel is
    /// scaled by `alpha` and by `color.a`, so the sprite's overall opacity
    /// is the product of the two.
    Sprite {
        /// The RGBA8 pixel data, row by row, top row first.
        data: Arc<[u8]>,
        /// The texture width in pixels.
        width: u32,
        /// The texture height in pixels.
        height: u32,
        /// The per-pixel tint, multiplied with every sampled pixel.
        color: Color,
        /// The sprite's opacity, multiplied with the texture's own alpha
        /// channel. 1.0 (the default) leaves the texture's transparency
        /// unchanged; 0.0 makes the sprite fully transparent.
        alpha: f32,
    },
    /// A text: `text` laid out with the font loaded by [`Shape::text`] at
    /// `size` pixels per em.
    ///
    /// The text block is centered on the node's origin, one font pixel per
    /// scene pixel, and the node's transform and scale apply to it exactly
    /// as they do to the other shapes. Each glyph is drawn as a tinted
    /// texture quad sampling a shared glyph atlas: `color` is the glyph
    /// color (white gives white text), and the text's overall opacity is the
    /// product of `alpha` and `color.a`.
    Text {
        /// The string to lay out.
        text: String,
        /// The font file's bytes, shared by every text shape that loaded
        /// the same file.
        font: Arc<[u8]>,
        /// The font size in pixels per em.
        size: f32,
        /// The glyph color, multiplied with every mask pixel.
        color: Color,
        /// The text's opacity, multiplied with the mask's own alpha
        /// channel.
        alpha: f32,
    },
    /// A batch of live particles from a [`ParticleSystem`], simulated in the
    /// node's local space and drawn as one instanced draw, each particle as
    /// the batch's `shape` at its local position, fading by its remaining
    /// life fraction.
    ///
    /// Every particle shares the batch's [`ParticleShape`], but keeps its
    /// own position, scale, rotation, and color: each particle is drawn as
    /// `shape`, scaled by its `size`, rotated by its `angle` (on top of the
    /// node's rotation), and tinted by its `color` — so a spinning
    /// rectangle or a tumbling sprite needs no engine support, just
    /// `particle.angle += spin * dt;` in the update.
    ///
    /// The batch rides the node's world transform and scale exactly like the
    /// other shapes: a particle at local `(x, y)` lands wherever the node's
    /// world transform maps it, and each size is multiplied by the geometric
    /// mean of the world's two axis scales — exact under a uniform scale, and
    /// area-preserving under a non-uniform one (the particle's shape cannot
    /// be stretched independently along the two axes, so it keeps its
    /// proportions). The batch is tinted by `color` multiplied with the
    /// node's composed modulate and with each particle's own `color`, and it
    /// draws at the node's composed order.
    ///
    /// The engine never advances the simulation: the node's owner spawns into
    /// the system and calls [`ParticleSystem::update`] each frame, in the same
    /// local space the particles are drawn in — for a system held on `node`,
    /// `if let Some(Shape::Particles { system, .. }) = node.shape.as_mut() {
    /// system.update(dt, gravity); }`. A system with no live particles draws
    /// nothing.
    Particles {
        /// The live system whose particles make up the batch.
        system: ParticleSystem,
        /// The batch's base tint, multiplied with every particle (and its
        /// own color) and then with the node's composed modulate; white
        /// leaves the particles uncolored by the batch itself.
        color: Color,
        /// The geometry each particle is drawn as; the whole batch shares
        /// it. [`ParticleShape::Circle`] (the default) draws circles as
        /// before.
        shape: ParticleShape,
    },
    /// A light: contributes the node's [`Light`] to the frame's light
    /// field.
    ///
    /// The light is never drawn — it only lights the [`SceneNode::lit`]
    /// receivers, evaluated per pixel at render time. It sits at the
    /// node's local origin: the node's transform (and its ancestors')
    /// position it in the scene, rotates its cone (if it has one), and
    /// tints its color through the composed modulate, exactly as they
    /// would a shape's. The radius is in user units, mapped one to one to
    /// pixels.
    Light {
        /// The light's color, strength, falloff extent, and cone.
        light: Light,
    },
}

impl Shape {
    /// Creates a sprite shape from the PNG file at `path`.
    ///
    /// The file is read and decoded to RGBA8 immediately, so a missing file
    /// or a non-PNG file fails here, not at render time. The pixels live
    /// behind an [`Arc`], so cloning a sprite shape — or any scene node or
    /// scene containing one — is cheap: all the clones share the same
    /// buffer.
    pub fn sprite(path: impl AsRef<Path>) -> Result<Self, SpriteError> {
        let bytes = std::fs::read(path.as_ref()).map_err(SpriteError::Io)?;
        Self::sprite_bytes(&bytes)
    }

    /// Creates a sprite shape from PNG data already in memory, for example
    /// bytes embedded into the binary with `include_bytes!`.
    ///
    /// Like [`Shape::sprite`], the bytes are decoded to RGBA8 up front, so a
    /// non-PNG buffer fails here, not at render time, and the pixels live
    /// behind an [`Arc`], but no file is read — this is how sprite shapes
    /// are created in environments without a file system, such as a web
    /// browser.
    pub fn sprite_bytes(bytes: impl AsRef<[u8]>) -> Result<Self, SpriteError> {
        let image = image::load_from_memory(bytes.as_ref()).map_err(SpriteError::Decode)?;
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        let data = Arc::from(rgba.into_raw().into_boxed_slice());
        Ok(Self::Sprite {
            data,
            width,
            height,
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            alpha: 1.0,
        })
    }

    /// Creates a text shape from the font file at `path`.
    ///
    /// The file is read immediately and checked to be a TrueType/OpenType
    /// font, so a missing file or a non-font file fails here, not at render
    /// time. The bytes live behind an [`Arc`], so every text shape that
    /// loaded the same file shares one buffer, and the render pipeline can
    /// cache one glyph atlas per (font, size) pair.
    pub fn text(
        path: impl AsRef<Path>,
        text: impl Into<String>,
        size: f32,
    ) -> Result<Self, TextError> {
        let bytes = std::fs::read(path.as_ref()).map_err(TextError::Io)?;
        Self::text_bytes(&bytes, text, size)
    }

    /// Creates a text shape from font data already in memory, for example
    /// bytes embedded into the binary with `include_bytes!`.
    ///
    /// Like [`Shape::text`], the bytes are checked to be a
    /// TrueType/OpenType font up front and live behind an [`Arc`], but no
    /// file is read — this is how text shapes are created in environments
    /// without a file system, such as a web browser.
    pub fn text_bytes(
        font: &[u8],
        text: impl Into<String>,
        size: f32,
    ) -> Result<Self, TextError> {
        swash::FontRef::from_index(font, 0).ok_or(TextError::InvalidFont)?;
        Ok(Self::Text {
            text: text.into(),
            font: Arc::from(font),
            size,
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            alpha: 1.0,
        })
    }

    /// The sprite's texture size in pixels, `[width, height]`, if the
    /// shape is a sprite: `None` for circles, rectangles, backgrounds and
    /// texts.
    pub fn sprite_size(&self) -> Option<[f32; 2]> {
        match self {
            Self::Sprite { width, height, .. } => Some([*width as f32, *height as f32]),
            _ => None,
        }
    }
}

/// An error while creating a [`Shape::Text`] from a font file.
#[derive(Debug)]
pub enum TextError {
    /// The file could not be read.
    Io(std::io::Error),
    /// The file was read but is not a TrueType/OpenType font.
    InvalidFont,
}

impl std::fmt::Display for TextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "failed to read the font file: {err}"),
            Self::InvalidFont => write!(f, "the font file is not a TrueType/OpenType font"),
        }
    }
}

impl std::error::Error for TextError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::InvalidFont => None,
        }
    }
}

/// An error while creating a [`Shape::Sprite`] from a PNG file.
#[derive(Debug)]
pub enum SpriteError {
    /// The file could not be read.
    Io(std::io::Error),
    /// The file was read but is not a decodable PNG image.
    Decode(image::ImageError),
}

impl std::fmt::Display for SpriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "failed to read the sprite file: {err}"),
            Self::Decode(err) => {
                write!(f, "failed to decode the sprite as a PNG image: {err}")
            }
        }
    }
}

impl std::error::Error for SpriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Decode(err) => Some(err),
        }
    }
}

/// A node in a scene tree.
///
/// Every node knows how to update itself and walk its own children, so a
/// whole tree updates with a single call to [`Node::visit`]: the visit runs
/// on every node in post-order (children before their parent), and each
/// node's [`Node::process`] runs once per visit.
pub trait Node {
    /// Per-frame update for this node, called once per [`Node::visit`],
    /// after the node's children have been visited. The default does
    /// nothing.
    fn process(&mut self) {}

    /// Visits this node and all of its descendants in post-order: each
    /// child's [`Node::visit`] runs before [`Node::process`] on this node,
    /// so a node's `process` sees its children already updated.
    fn visit(&mut self);
}

/// A node in a [`Scene`] tree.
///
/// A node is a [`Transform`], a `scale`, a `modulate`, and an `order`, plus
/// its own optional [`Shape`] and its children. The transform and the scale
/// are relative to the node's parent and apply to the node's own shape as
/// well as composing onto all descendants; the modulate multiplies into the
/// node's own shape color as well as composing onto all descendants'; the
/// order does the same for the subtree's draw order. A node without a shape
/// (`shape: None`) is a pure group or pivot node, and a node without
/// children is a leaf. A node literal can leave any fields out by writing
/// `..SceneNode::default()`: the omitted fields take the identity transform,
/// no scale, a white modulate, order `0.0`, no lighting, no occlusion,
/// no shape, and no children.
///
/// A [`SceneNode`] is a [`Node`]: its [`Node::visit`] walks its children and
/// then runs its (default, no-op) [`Node::process`], so a whole scene tree
/// updates with a single [`Scene::visit`].
#[derive(Clone, Debug)]
pub struct SceneNode {
    /// The transform from the parent's coordinate space to this node's,
    /// applied to the node's own shape and composed onto its children.
    pub transform: Transform,
    /// The node's non-uniform scale, applied in its own coordinate space
    /// before its transform, so it scales the node's shape and its whole
    /// subtree. `[1.0, 1.0]` (the default) is no scaling.
    pub scale: [f32; 2],
    /// The node's color modulation, multiplied channel by channel into the
    /// node's own shape color and composed (multiplied) onto its children's,
    /// so it tints the whole subtree, just like the scale and the transform:
    /// the node's `modulate` multiplies the modulation inherited from its
    /// ancestors, and that product is what its children inherit. White
    /// (all channels `1.0`, the default) leaves the colors unchanged.
    pub modulate: Color,
    /// The node's draw-order offset, added to the order inherited from its
    /// ancestors: the node's own shape draws at that total, and the same
    /// total is passed on to the children, so the order propagates down the
    /// whole subtree, just like the scale and the transform. `0.0` (the
    /// default) keeps the subtree at its inherited draw order.
    pub order: f32,
    /// Whether the node's own shape is lit by the frame's light field: when
    /// true, the shape's color is multiplied, per pixel, by the scene's
    /// ambient color plus the contribution of every light at that pixel.
    ///
    /// Unlike the transform, scale, modulate, and order, the flag does not
    /// propagate to the children: it applies to this node's own shape only,
    /// and each child keeps its own. `false` (the default) draws the shape
    /// unlit, exactly as before.
    pub lit: bool,
    /// Whether the node's own shape occludes the frame's lights: when true
    /// and the shape is a rectangle, the rectangle's silhouette blocks the
    /// light's path to every lit pixel behind it — a hard shadow, per pixel,
    /// with no penumbra. The shadow is evaluated in the frame's pixel space
    /// from the rectangle's full composed transform, so a rotated, scaled,
    /// or translated node casts the shadow of wherever it actually sits.
    ///
    /// Like [`SceneNode::lit`], the flag does not propagate to the
    /// children: it applies to this node's own shape only, and each child
    /// keeps its own. `false` (the default) casts no shadow. A node can
    /// carry both flags: a `lit: true, occludes: true` wall is lit on its
    /// own surface and still blocks the light from what lies behind it.
    pub occludes: bool,
    /// The shape this node draws, in its own local space, if any.
    pub shape: Option<Shape>,
    /// The child nodes, positioned in this node's coordinate space.
    pub children: Vec<Box<SceneNode>>,
}

impl Default for SceneNode {
    /// An identity node: identity transform, no scale, a white modulate,
    /// order `0.0`, no lighting, no occlusion, no shape, and no children.
    fn default() -> Self {
        Self {
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: WHITE,
            order: 0.0,
            lit: false,
            occludes: false,
            shape: None,
            children: Vec::new(),
        }
    }
}

impl Node for SceneNode {
    /// Visits every child first, then updates this node itself (post-order),
    /// so [`Node::process`] sees the children already updated.
    fn visit(&mut self) {
        for child in &mut self.children {
            child.visit();
        }
        self.process();
    }
}

/// A rendering layer of a [`Scene`]: an independent draw group with its own
/// layer order, its own parallax speed, and its own root node.
///
/// Layers are hard draw partitions: every node of a layer is drawn after
/// (on top of) every node of a layer with a lower `order`, and before every
/// node of a layer with a higher `order`, no matter the nodes' local
/// `order` values. Within a layer the local ordering applies: lower local
/// order first, ties resolved in tree order.
///
/// The scene's root subtree is itself a group, at the implicit layer order
/// `0.0`, declared before every explicit layer: it is drawn before any
/// layer with the same or a higher order, and after any layer with a lower
/// order. A [`Shape::Background`] inside a layer keeps the background
/// behavior of every group: it never draws, it only contributes to the
/// frame's clear color.
#[derive(Clone, Debug, Default)]
pub struct Layer {
    /// The layer's order: higher order is closer to the camera (drawn
    /// later, on top of layers with a lower order).
    pub order: f32,
    /// The layer's parallax speed, as a multiple of the scene's camera
    /// motion: `1.0` (the default) follows the camera exactly, a higher
    /// speed makes the layer's contents move faster than the camera (the
    /// layer reads as closer), and a lower speed slower (farther away).
    /// Ignored when the scene has no [`Scene::camera`].
    pub speed: f32,
    /// The layer's repetition: `(repeat_x, repeat_y)`. `(0.0, 0.0)` (the
    /// default) draws the layer once. A non-zero component tiles the layer
    /// in both directions along that axis with a period of `abs(component)`
    /// pixels, so a moving camera can scroll through the layer infinitely:
    /// the layer is drawn once per copy of its content that can overlap
    /// the window — with a pure-translation camera, only the few copies
    /// the window's box and the content's span around it reach, because
    /// the minimum period is the window's width/height — and an object
    /// crossing a tile boundary is split into wrapping slices, the part
    /// past the boundary appearing on the opposite side of the window
    /// (each copy is re-used, with just a displacement, and clipped by the
    /// per-object scissor test, so no object is ever drawn twice). A
    /// non-zero period below the window size on that axis, or an object
    /// whose extent in a repeating axis exceeds its period (the same object
    /// would be drawn twice), aborts the program with an error. The
    /// component's sign is ignored.
    ///
    /// The repetition is a periodic extension of the layer's *current*
    /// content: each copy carries the content as it is now, displaced by
    /// its offset, so content that moves through the layer's space is
    /// re-tiled as it moves. An object a camera follows exactly stays at
    /// the window's center wherever it wanders: its wrapped copies sit one
    /// full period away, off the window's edge, because the period is at
    /// least the window size. A unique object (such as a player) is
    /// usually given its own non-repeating layer, because it is not part
    /// of the pattern that should tile.
    pub repeat: [f32; 2],
    /// The root node of the layer's tree, walked like a scene's root.
    pub root: SceneNode,
}

impl Layer {
    /// Creates a layer at the default order `0.0` and the default speed
    /// `1.0`, from its root node.
    pub fn new(root: SceneNode) -> Self {
        Self {
            order: 0.0,
            speed: 1.0,
            repeat: [0.0, 0.0],
            root,
        }
    }
}

/// The position of a node within a [`Scene`]: the group that contains the
/// node — the base group (the scene's root subtree) or one of the scene's
/// explicit [`Layer`]s — and the child-index path from that group's root
/// down to the node.
#[derive(Clone, Debug)]
pub struct NodePath {
    /// The group the node lives in: `None` for the base group (the scene's
    /// root subtree), `Some(i)` for the `i`th layer of the scene.
    pub group: Option<usize>,
    /// Child indices from the group's root down to the node: the first is
    /// the root's child, the second that child's child, and so on. An empty
    /// path names the group's root itself.
    pub children: Vec<usize>,
}

/// A tree of [`SceneNode`]s rooted at a single node, plus zero or more
/// rendering [`Layer`]s.
///
/// Pass a scene to [`run`](crate::run): every frame the user's
/// [`Process`](crate::Process) runs, then the tree is updated with
/// [`Scene::visit`] (every node's [`Node::process`], children before their
/// parent, on the root tree and on every layer's tree), and only then is
/// the scene drawn — so both can mutate it in place (via
/// [`Context::scene`](crate::Context::scene)) to animate it. Use
/// [`Canvas::draw_scene`](crate::Canvas::draw_scene) to draw additional
/// scenes.
///
/// The root subtree and the layers are separate draw groups: the root
/// subtree is the group at the implicit layer order `0.0`, and each layer
/// is its own group at its [`Layer::order`]. Groups are painted by ascending
/// order — higher order closer to the camera, drawn later, on top — and
/// within a group the local ordering applies.
///
/// If the scene has a [`Scene::camera`], the scene is drawn in that node's
/// coordinate space instead of the fixed window-centered user space: the
/// camera node stays at the window origin and the rest of the scene moves
/// and rotates around it, so a camera hung under a moving node follows it.
/// Each group renders from the camera scaled by its speed — the base group
/// at the full speed `1.0`, and each layer at its [`Layer::speed`].
#[derive(Clone, Debug)]
pub struct Scene {
    /// The root node; the scene is walked depth-first from here.
    pub root: SceneNode,
    /// The scene's rendering layers, in declaration order.
    pub layers: Vec<Layer>,
    /// The scene's camera node, if any: the scene is drawn in that node's
    /// coordinate space, so the node stays at the window origin while the
    /// rest of the scene moves around it. `None` (the default) draws the
    /// scene in the fixed window-centered user space.
    pub camera: Option<NodePath>,
    /// The ambient color of the scene's light field: the floor every
    /// [`SceneNode::lit`] receiver's color is multiplied by even where no
    /// light reaches. A lit shape's pixel color becomes
    /// `color * (ambient + light contributions)` — with a black ambient,
    /// pixels no light reaches are fully dark; with the default dim gray,
    /// they read as a darkened version of the shape's color.
    pub ambient: Color,
}

impl Scene {
    /// Creates a scene from its root node, with no layers, no camera, and
    /// the default dim-gray ambient.
    pub fn new(root: SceneNode) -> Self {
        Self {
            root,
            layers: Vec::new(),
            camera: None,
            ambient: AMBIENT,
        }
    }

    /// Updates the whole tree in one call: visits the root, which visits
    /// every child, and so on down the tree, so each node's
    /// [`Node::process`] runs exactly once, children before their parent —
    /// on the root tree and on every layer's tree.
    pub fn visit(&mut self) {
        self.root.visit();
        for layer in &mut self.layers {
            layer.root.visit();
        }
    }

    /// The world transform of the node at `path` — the node's own scale and
    /// transform composed over its ancestors', up to its group's root — or
    /// `None` if the path names no node (a missing layer or a missing
    /// child).
    pub(crate) fn world_at(&self, path: &NodePath) -> Option<Transform> {
        let root = match path.group {
            None => &self.root,
            Some(index) => &self.layers.get(index)?.root,
        };
        let mut node = root;
        let mut parent = Transform::identity();
        for &index in &path.children {
            let local =
                Transform::scale(node.scale).compose(&node.transform);
            parent = local.compose(&parent);
            node = node.children.get(index)?;
        }
        let local = Transform::scale(node.scale).compose(&node.transform);
        Some(local.compose(&parent))
    }

    /// The world transform of the scene's camera node, if the scene has a
    /// camera and its path names a node.
    pub(crate) fn camera_world(&self) -> Option<Transform> {
        self.camera.as_ref().and_then(|path| self.world_at(path))
    }
}

impl Default for Scene {
    /// An empty scene: an identity root node with no shape and no children,
    /// no layers, no camera, and the default dim-gray ambient, for apps that
    /// only use the immediate draw methods.
    fn default() -> Self {
        Self {
            root: SceneNode::default(),
            layers: Vec::new(),
            camera: None,
            ambient: AMBIENT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_is_unlit_by_default() {
        assert!(!SceneNode::default().lit);
    }

    #[test]
    fn scene_ambient_is_a_dim_gray_by_default() {
        let ambient = Scene::default().ambient;
        assert_eq!(
            ambient,
            Color {
                r: 0.3,
                g: 0.3,
                b: 0.3,
                a: 1.0
            }
        );
        // `Scene::new` carries the same default.
        assert_eq!(Scene::new(SceneNode::default()).ambient, ambient);
    }
}
