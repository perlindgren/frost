//! The tile map batch: scissor arithmetic and the packed byte layouts.
//! The collection side lives with the canvas tests.

use super::black;
use crate::backend::frame::{Draw, tilemap_instance, tilemap_uniform_data};
use crate::objects::{Color, SpriteFilter};
use std::sync::Arc;

fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color { r, g, b, a }
}

/// A tile-map draw with the given pixel bounds and optional clip, over
/// one blank tile.
fn map_draw(bounds: ([f32; 2], [f32; 2]), clip: Option<([f32; 2], [f32; 2])>) -> Draw {
    Draw::TileMap {
        data: vec![0u8; 48],
        count: 1,
        color: black(),
        atlas_data: Arc::from(vec![0u8; 4].into_boxed_slice()),
        atlas_size: [1, 1],
        atlas_generation: 1,
        filter: SpriteFilter::Nearest,
        bounds,
        clip,
        z: 1.0,
        diagnostic: false,
    }
}

#[test]
fn tilemap_scissor_is_the_batch_box() {
    // Unclipped, the batch's own pixel box, as usual floored to the
    // containing pixel and spanned to the ceiling.
    let draw = map_draw(([10.0, 20.0], [50.0, 40.0]), None);
    assert_eq!(draw.scissor_rect([100, 100]), Some([10, 20, 40, 20]));
}

#[test]
fn tilemap_scissor_intersects_the_view_clip() {
    // The clip cuts the batch box; the map's tiles outside it rasterize
    // nowhere, which is exactly the point of the view rectangle.
    let draw = map_draw(
        ([10.0, 20.0], [50.0, 40.0]),
        Some(([30.0, 0.0], [70.0, 60.0])),
    );
    assert_eq!(draw.scissor_rect([100, 100]), Some([30, 20, 20, 20]));
}

#[test]
fn tilemap_clip_off_surface_skips_the_draw() {
    // A map whose view window is wholly off-screen draws nothing — the
    // scissor is None and the render loop moves on without a draw call.
    let draw = map_draw(
        ([10.0, 20.0], [50.0, 40.0]),
        Some(([60.0, 60.0], [80.0, 80.0])),
    );
    assert_eq!(draw.scissor_rect([100, 100]), None);
}

#[test]
fn tilemap_instances_pack_four_vec4s() {
    // Center and x-half-edge, y-half-edge (padded), UV bounds, tint —
    // sixteen little-endian floats, 64 bytes, in the order
    // tilemap.wgsl reads them. The edges are plain vectors: a rotated
    // node's negative or swiveled directions pass through untouched —
    // they carry the rotation.
    let bytes = tilemap_instance(
        65.0,
        25.0,
        [16.0, -16.0],
        [16.0, 16.0],
        [0.25, 0.5, 0.75, 1.0],
        &rgba(1.0, 0.5, 0.25, 0.8),
        [3.0, 1.0],
    );
    let (words, tail) = bytes.as_chunks::<4>();
    assert!(tail.is_empty());
    let floats: Vec<f32> = words.iter().map(|b| f32::from_le_bytes(*b)).collect();
    assert_eq!(
        floats,
        vec![
            65.0, 25.0, 16.0, -16.0, 16.0, 16.0, 3.0, 1.0, 0.25, 0.5, 0.75, 1.0, 1.0, 0.5, 0.25,
            0.8
        ]
    );
    // The edge vec4's two floats of slack are no longer slack: they
    // carry the cell's own orientation — flip bits (both here) and the
    // clockwise quarter-turn count — and nothing else rides there.
    assert_eq!(f32::from_le_bytes(bytes[24..28].try_into().unwrap()), 3.0);
    assert_eq!(f32::from_le_bytes(bytes[28..32].try_into().unwrap()), 1.0);
}
#[test]
fn tilemap_uniforms_layout_is_size_then_color() {
    // vec2 @ 0, vec4 @ 16 — 32 bytes, the WGSL-mirrored layout pinned by
    // the shaders test.
    let data = tilemap_uniform_data([1440.0, 810.0], rgba(1.0, 0.5, 0.0, 0.25));
    assert_eq!(data.len(), 32);
    let f = |offset: usize| f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
    assert_eq!(f(0), 1440.0);
    assert_eq!(f(4), 810.0);
    assert_eq!(f(16), 1.0);
    assert_eq!(f(20), 0.5);
    assert_eq!(f(24), 0.0);
    assert_eq!(f(28), 0.25);
}
