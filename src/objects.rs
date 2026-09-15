//! The scene objects: the [`Transform`], [`Color`], [`Shape`], [`SceneNode`]
//! and [`Scene`] types that make up the scene tree drawn by frost.

use std::path::Path;
use std::sync::Arc;

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
    pub(crate) fn channels(&self) -> [f32; 3] {
        [self.r, self.g, self.b]
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

/// A filled geometric shape that a [`SceneNode`] can hold.
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
    /// with every sampled pixel; white (`[1.0, 1.0, 1.0]`) leaves the
    /// texture unchanged. The texture's own alpha channel is scaled by
    /// `alpha`, so `alpha` is the sprite's overall opacity.
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
    /// color (white gives white text), and `alpha` is the text's overall
    /// opacity.
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
        let image = image::load_from_memory(&bytes).map_err(SpriteError::Decode)?;
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
            },
            alpha: 1.0,
        })
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

/// A node in a [`Scene`] tree.
///
/// A node is a [`Transform`], a `scale`, and an `order`, plus its own
/// optional [`Shape`] and its children. The transform and the scale are
/// relative to the node's parent and apply to the node's own shape as well as
/// composing onto all descendants; the order does the same for the subtree's
/// draw order. A node without a shape (`shape: None`) is a pure group or
/// pivot node, and a node without children is a leaf.
#[derive(Clone, Debug)]
pub struct SceneNode {
    /// The transform from the parent's coordinate space to this node's,
    /// applied to the node's own shape and composed onto its children.
    pub transform: Transform,
    /// The node's non-uniform scale, applied in its own coordinate space
    /// before its transform, so it scales the node's shape and its whole
    /// subtree. `[1.0, 1.0]` (the default) is no scaling.
    pub scale: [f32; 2],
    /// The node's draw-order offset, added to the order inherited from its
    /// ancestors: the node's own shape draws at that total, and the same
    /// total is passed on to the children, so the order propagates down the
    /// whole subtree, just like the scale and the transform. `0.0` (the
    /// default) keeps the subtree at its inherited draw order.
    pub order: f32,
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
                order: 0.0,
                shape: None,
                children: Vec::new(),
            },
        }
    }
}
