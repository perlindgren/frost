use std::sync::Arc;

use crate::objects::*;
use crate::particles::Particle;

/// The frame's clear color when no [`Shape::Background`] is drawn.
pub(crate) const DEFAULT_BACKGROUND: Color = Color {
    r: 0.07,
    g: 0.09,
    b: 0.14,
    a: 1.0,
};

/// The anti-alias band in pixels, mirrored by the `aa` constant in the SDF
/// shader sources; keep the two in sync so bounding boxes cover every pixel
/// the shaders can write.
pub(crate) const AA_BAND: f32 = 0.75;

/// A drawing recorded for the current frame, in pixel space.
///
/// `z` is the draw order: lower `z` is drawn first (further back). Drawings
/// with the same `z` are drawn in call order, so the last one drawn is on top.
#[derive(Clone, Debug)]
pub(crate) enum Draw {
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
        /// Whether the shape is lit by the frame's light field: `1.0` when
        /// the node's `lit` flag is set, `0.0` when unlit.
        lit: f32,
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
        /// Whether the sprite is lit by the frame's light field: `1.0` when
        /// the node's `lit` flag is set, `0.0` when unlit. Glyph quads
        /// expanded from a lit text block are lit too.
        lit: f32,
        z: f32,
    },
    /// A batch of particles drawn in one instanced draw call.
    ///
    /// `data` holds two `vec4<f32>`s per particle — `(px, py, size, life
    /// fraction)` and `(angle, tint.r, tint.g, tint.b)`, all in pixel space
    /// (top-left origin, y down; the angle in pixel-space radians) — packed
    /// as 32 bytes per particle, so `count == data.len() / 32`. The whole
    /// batch shares `color`, `kind`, and `aspect`; each particle's alpha is
    /// additionally scaled by its life fraction and its tint, so dying
    /// particles fade out.
    Particles {
        /// The packed instance data, eight floats (32 bytes) per particle.
        data: Vec<u8>,
        /// The number of particles in `data`.
        count: u32,
        /// The batch's base tint.
        color: Color,
        /// The shape kind each particle is drawn as: `0.0` circle,
        /// `1.0` rectangle, `2.0` sprite.
        kind: f32,
        /// The shape's height-to-width factor (`1.0` for a circle).
        aspect: f32,
        /// The sprite's RGBA8 pixel data, present only when `kind` is
        /// `2.0`.
        sprite_data: Option<Arc<[u8]>>,
        /// The sprite's `(width, height)` in pixels, valid when `kind` is
        /// `2.0`.
        sprite_size: [u32; 2],
        /// Whether the batch is lit by the frame's light field: `1.0` when
        /// the node's `lit` flag is set, `0.0` when unlit.
        lit: f32,
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
        /// Whether the text is lit by the frame's light field: `1.0` when
        /// the node's `lit` flag is set, `0.0` when unlit; the flag is
        /// passed on to every glyph quad the block expands into.
        lit: f32,
        z: f32,
    },
    /// A [`Shape::Background`]: never drawn; it is promoted to the frame's
    /// clear color at render time.
    Background {
        color: Color,
    },
    /// A [`Shape::Light`]: never drawn; it is promoted to the frame's light
    /// field at render time, where every lit receiver evaluates it per
    /// pixel.
    Light {
        /// The light's position, in pixel space (already through the node's
        /// composed transform).
        pos: [f32; 2],
        /// The falloff extent, in pixels: zero contribution beyond this
        /// distance from `pos`.
        radius: f32,
        /// The light's strength, multiplied with its color per pixel.
        intensity: f32,
        /// The light's color, already multiplied with the node's composed
        /// modulate.
        color: Color,
        z: f32,
    },
}

impl Draw {
    pub(crate) fn z(&self) -> f32 {
        match self {
            Draw::Line { z, .. } => *z,
            Draw::Circle { z, .. } => *z,
            Draw::Rectangle { z, .. } => *z,
            Draw::Shape { z, .. } => *z,
            Draw::Sprite { z, .. } => *z,
            Draw::Particles { z, .. } => *z,
            Draw::Text { z, .. } => *z,
            // The background is always at the very back.
            Draw::Background { .. } => f32::MIN,
            Draw::Light { z, .. } => *z,
        }
    }

    /// The tight pixel rectangle the draw can write to, including the
    /// anti-alias band the shaders render beyond each geometric edge.
    ///
    /// `area` is the render area in pixels; the result is clamped to it, or
    /// `None` if the draw is fully outside the surface or is a background or
    /// a light (which are never drawn — they become the frame's clear color
    /// and light field).
    pub(crate) fn scissor_rect(&self, area: [u32; 2]) -> Option<[u32; 4]> {
        if matches!(self, Draw::Background { .. } | Draw::Light { .. }) {
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
            // The batch's particles can be anywhere, so the scissor is the
            // whole surface: the per-particle quads are already tight, and
            // the rasterizer discards everything else for free.
            Draw::Particles { .. } => {
                ([0.0, 0.0], [area[0] as f32, area[1] as f32])
            }
            // A background never draws; the early return above covers it.
            Draw::Background { .. } => unreachable!(),
            // A light never draws; the early return above covers it.
            Draw::Light { .. } => unreachable!(),
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
pub(crate) fn aabb_of_box(world: &Transform, center: [f32; 2], hx: f32, hy: f32) -> ([f32; 2], [f32; 2]) {
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
pub(crate) fn clear_color(draws: &[Draw]) -> Color {
    draws
        .iter()
        .rev()
        .find_map(|draw| match draw {
            Draw::Background { color } => Some(*color),
            _ => None,
        })
        .unwrap_or(DEFAULT_BACKGROUND)
}

/// Writes `value` as little-endian f32 bytes into `data` at byte `offset`.
fn write_f32_at(data: &mut [u8], offset: usize, value: f32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Line uniform data, 48 bytes, matching the WGSL uniform-space layout of
/// the `Uniforms` Wgsl struct: `a` @ 0, `b` @ 8, `color` @ 16 (a vec4<f32>
/// is 16-byte aligned in uniform space, leaving an 8-byte gap), `width` @
/// 32 (the next member is aligned to its own alignment, so the f32 follows
/// the vec4 without a gap); the struct size rounds up to 48.
pub(crate) fn line_uniform_data(a: [f32; 2], b: [f32; 2], color: Color, width: f32) -> Vec<u8> {
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
pub(crate) fn circle_uniform_data(center: [f32; 2], color: Color, radius: f32) -> Vec<u8> {
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
pub(crate) fn rect_uniform_data(center: [f32; 2], extent: [f32; 2], color: Color) -> Vec<u8> {
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
pub(crate) fn shape_uniform_data(
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
pub(crate) fn sprite_uniform_data(
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

/// Particle batch uniform data, 48 bytes, matching the WGSL uniform-space
/// layout of the `ParticlesUniforms` Wgsl struct: `size` (a vec2) @ 0,
/// `color` (a vec4) @ 16 (16 bytes, 16-byte aligned, leaving an 8-byte
/// gap), and `misc` (a vec2: the shape's kind and aspect) @ 32; the struct
/// size rounds up to 48.
pub(crate) fn particles_uniform_data(
    size: [f32; 2],
    color: Color,
    kind: f32,
    aspect: f32,
) -> Vec<u8> {
    let mut data = vec![0u8; 48];
    write_f32_at(&mut data, 0, size[0]);
    write_f32_at(&mut data, 4, size[1]);
    write_f32_at(&mut data, 16, color.r);
    write_f32_at(&mut data, 20, color.g);
    write_f32_at(&mut data, 24, color.b);
    write_f32_at(&mut data, 28, color.a);
    write_f32_at(&mut data, 32, kind);
    write_f32_at(&mut data, 36, aspect);
    data
}

/// Packs one particle of a [`Draw::Particles`] batch: eight little-endian
/// floats, 32 bytes — first the pixel position `(px, py)`, the pixel size,
/// and the remaining life fraction, then the pixel-space angle and the
/// particle's tint `(r, g, b)` — in the layout `particles.wgsl` reads as
/// `array<vec4<f32>>`.
///
/// The life fraction is clamped to `0.0..=1.0` (and `0.0` for a
/// `max_life <= 0`), and a negative size is clamped to `0.0`.
pub(crate) fn particle_instance(
    p: &Particle,
    px: f32,
    py: f32,
    size: f32,
    angle: f32,
) -> [u8; 32] {
    let life = if p.max_life > 0.0 {
        (p.life / p.max_life).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut out = [0u8; 32];
    out[0..4].copy_from_slice(&px.to_le_bytes());
    out[4..8].copy_from_slice(&py.to_le_bytes());
    out[8..12].copy_from_slice(&size.max(0.0).to_le_bytes());
    out[12..16].copy_from_slice(&life.to_le_bytes());
    out[16..20].copy_from_slice(&angle.to_le_bytes());
    out[20..24].copy_from_slice(&p.color.r.to_le_bytes());
    out[24..28].copy_from_slice(&p.color.g.to_le_bytes());
    out[28..32].copy_from_slice(&p.color.b.to_le_bytes());
    out
}

/// The byte size of the light field's header: the light `count` (a u32),
/// 12 padding bytes, and the scene's ambient color (a `vec4`).
pub(crate) const LIGHT_FIELD_HEADER: usize = 32;

/// The byte size of one light record: two `vec4<f32>`s —
/// `(x, y, radius, intensity)` and `(r, g, b, 1.0)`.
pub(crate) const LIGHT_RECORD: usize = 32;

/// The frame's light field, packed little-endian for the GPU's storage
/// buffer.
///
/// The layout is a [`LIGHT_FIELD_HEADER`]-byte header — the light `count`
/// at bytes `0..4`, 12 padding bytes, the scene's ambient color channels at
/// bytes `16..32` — followed by one [`LIGHT_RECORD`]-byte record per light,
/// in call order: a `(x, y, radius, intensity)` `vec4`, then an
/// `(r, g, b, 1.0)` `vec4`. The records line up with the unsized WGSL
/// `array<vec4<f32>>` tail of the field's storage struct, so `data` can be
/// bound as-is.
pub(crate) struct LightField {
    /// The number of lights packed — the header's `count`.
    pub count: u32,
    /// The packed bytes: a 32-byte header plus `count * 32` bytes.
    pub data: Vec<u8>,
}

/// Packs the frame's light field from its draws and the scene's ambient
/// color.
///
/// Only [`Draw::Light`]s contribute, in call order; every other draw is
/// ignored. The ambient color comes from the scene, not the draws.
pub(crate) fn pack_light_field(draws: &[Draw], ambient: Color) -> LightField {
    let count = draws
        .iter()
        .filter(|draw| matches!(draw, Draw::Light { .. }))
        .count() as u32;
    let mut data = vec![0u8; LIGHT_FIELD_HEADER + count as usize * LIGHT_RECORD];
    data[0..4].copy_from_slice(&count.to_le_bytes());
    write_f32_at(&mut data, 16, ambient.r);
    write_f32_at(&mut data, 20, ambient.g);
    write_f32_at(&mut data, 24, ambient.b);
    write_f32_at(&mut data, 28, ambient.a);
    let mut record = 0usize;
    for draw in draws {
        if let Draw::Light {
            pos,
            radius,
            intensity,
            color,
            ..
        } = draw
        {
            let off = LIGHT_FIELD_HEADER + record * LIGHT_RECORD;
            write_f32_at(&mut data, off, pos[0]);
            write_f32_at(&mut data, off + 4, pos[1]);
            write_f32_at(&mut data, off + 8, *radius);
            write_f32_at(&mut data, off + 12, *intensity);
            write_f32_at(&mut data, off + 16, color.r);
            write_f32_at(&mut data, off + 20, color.g);
            write_f32_at(&mut data, off + 24, color.b);
            // The color vec4's w component is unused by the shader; keep it
            // at 1.0.
            write_f32_at(&mut data, off + 28, 1.0);
            record += 1;
        }
    }
    LightField { count, data }
}
