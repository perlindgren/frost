//! The backend's unit tests, split by topic into the sibling modules:
//! scissor, transform, scene, nodes, context, particles, uniforms,
//! fields, layers, camera, repeat, sprites, text and config.
//!
//! Each module imports the backend's public items and this module's
//! shared [`black`] helper; every test keeps the name it had when the
//! suite was a single file.

mod camera;
mod config;
mod context;
mod fields;
mod layers;
mod nodes;
mod particles;
mod repeat;
mod scene;
mod scissor;
mod sprites;
mod text;
mod transform;
mod uniforms;

use crate::objects::Color;

fn black() -> Color {
    Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    }
}
