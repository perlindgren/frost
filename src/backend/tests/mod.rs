//! The backend's unit tests, split by topic into the sibling modules:
//! scissor, transform, scene, nodes, context, particles, uniforms,
//! fields, layers, camera, repeat, sprites, text, tilemap, draw cost,
//! light cost, emission, the shared machine, bloom, the present harness, tone map, frame
//! buffer, the letterbox probe, the sRGB envelope probe and config.
//!
//! Each module imports the backend's public items and this module's
//! shared [`black`] helper; every test keeps the name it had when the
//! suite was a single file.

mod blend;
pub(crate) mod bloom;
mod camera;
mod config;
mod context;
mod draw_cost;
mod emission;
mod fields;
mod frame_buffer;
mod identity;
mod layers;
mod letterbox;
mod light_cost;
mod machine;
mod nodes;
mod particle_cost;
mod particles;
mod penumbra;
mod polygon;
mod present;
mod repeat;
mod scene;
mod scissor;
mod shadows;
mod sprites;
mod srgb_envelope;
mod text;
mod tilemap;
mod tonemap;
mod transform;
mod uniforms;

/// The half-float a `Rgba16Float` target stores, widened. A readback of a
/// float target arrives as 16-bit channels, and the alternative — asking for
/// `Rgba32Float`, which no adapter here will blend without a feature — would
/// test a format the engine never allocates.
pub(crate) fn half_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = (bits >> 10) & 0x1f;
    let fraction = u32::from(bits & 0x3ff);
    let bits = match exponent {
        0 => sign,
        0x1f => sign | 0x7f80_0000 | (fraction << 13),
        e => sign | ((u32::from(e) + 127 - 15) << 23) | (fraction << 13),
    };
    f32::from_bits(bits)
}

use crate::objects::Color;

fn black() -> Color {
    Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    }
}
