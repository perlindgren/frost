//! The CPU-side uniform bytes must follow the WGSL struct layouts.

use super::super::*;
use crate::objects::*;

#[test]
fn particles_uniform_bytes_follow_the_wgsl_layout() {
    // Lock the byte layout of `particles_uniform_data` to the WGSL
    // uniform-space layout of `ParticlesUniforms`, so a reorder of the
    // WGSL struct is caught here. `size` (a vec2) spans 0..8; the
    // 16-byte-aligned `color` vec4 starts at 16, so bytes 8..16 are
    // padding; `misc` (a vec2) starts at 32; `lit` sits at 40; the struct
    // spans 48 bytes.
    let data = particles_uniform_data(
        [1280.0, 720.0],
        Color {
            r: 0.1,
            g: 0.2,
            b: 0.3,
            a: 0.4,
        },
        2.0,
        1.5,
        1.0,
    );
    assert_eq!(data.len(), 48);

    let f32_at = |off: usize| f32::from_le_bytes(data[off..off + 4].try_into().unwrap());
    assert_eq!(f32_at(0), 1280.0);
    assert_eq!(f32_at(4), 720.0);
    // The padding between `size` and the 16-byte-aligned `color` is
    // zeroed by the `vec![0u8; 48]` init.
    assert_eq!(f32_at(8), 0.0);
    assert_eq!(f32_at(12), 0.0);
    assert_eq!(f32_at(16), 0.1);
    assert_eq!(f32_at(20), 0.2);
    assert_eq!(f32_at(24), 0.3);
    assert_eq!(f32_at(28), 0.4);
    // `misc` carries the shape's kind and aspect at 32 and 36, and the
    // lit flag takes the struct's final slot at 40.
    assert_eq!(f32_at(32), 2.0);
    assert_eq!(f32_at(36), 1.5);
    assert_eq!(f32_at(40), 1.0);
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
    // (spanning 48..64); `misc` vec2 @ 64; `emission` vec4 @ 80; `glow` vec4
    // @ 96; struct size 112.
    let inv = Transform::translate([1.5, -2.5]).invert().unwrap();
    let data = shape_uniform_data(
        inv,
        [7.0, 8.0],
        [9.0, 10.0],
        1.0,
        0.25,
        Color {
            r: 0.1,
            g: 0.2,
            b: 0.3,
            a: 0.4,
        },
        Color {
            r: 0.5,
            g: 0.6,
            b: 0.7,
            a: 0.8,
        },
        // Distinct from the emission on purpose: the point of this test is the
        // offsets, and a glow that happened to equal the emission would let a
        // writer that ignores the field pass.
        Some(Color {
            r: 0.55,
            g: 0.65,
            b: 0.75,
            a: 0.9,
        }),
        1.0,
    );

    let f32_at = |off: usize| f32::from_le_bytes(data[off..off + 4].try_into().unwrap());
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
    // the `misc` vec2 begins at byte 64: `aa` at 64, `kind` at 68; the lit
    // flag takes the slot at 72; the 16-byte-aligned emission vec4 follows at
    // 80 (spanning 80..96) and the glow vec4 at 96 (96..112, so the struct is
    // 112 bytes).
    assert_eq!(f32_at(60), 0.4);
    assert_eq!(f32_at(64), 0.25);
    assert_eq!(f32_at(68), 1.0);
    assert_eq!(f32_at(72), 1.0);
    assert_eq!(f32_at(80), 0.5);
    assert_eq!(f32_at(84), 0.6);
    assert_eq!(f32_at(88), 0.7);
    assert_eq!(f32_at(92), 0.8);
    assert_eq!(f32_at(96), 0.55);
    assert_eq!(f32_at(100), 0.65);
    assert_eq!(f32_at(104), 0.75);
    assert_eq!(f32_at(108), 0.9);
    assert_eq!(data.len(), 112);
}

#[test]
fn sprite_uniform_bytes_follow_the_wgsl_layout() {
    // Lock the byte layout of `sprite_uniform_data` to the WGSL
    // uniform-space layout of `SpriteUniforms`, the same way the shape
    // test does: the mat2x2 `<f32>` spans 0..16 (column 0 @ 0, column 1
    // @ 8), `translation` @ 16, `size` @ 24, the tint vec4 (16 bytes,
    // 16-byte aligned) @ 32 spanning 32..48, the scalar alpha @ 48, the
    // scalar lit @ 52, the `uv_rect` vec4 (16-byte aligned) @ 64 spanning
    // 64..80, the emission vec4 (16-byte aligned) @ 80 spanning 80..96, and
    // the glow vec4 @ 96 spanning 96..112; the struct size is 112.
    let inv = Transform::translate([1.5, -2.5]).invert().unwrap();
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
        Color {
            r: 0.6,
            g: 0.7,
            b: 0.8,
            a: 0.9,
        },
        Some(Color {
            r: 0.65,
            g: 0.75,
            b: 0.85,
            a: 0.95,
        }),
        1.0,
        [0.25, 0.5, 0.75, 1.0],
    );
    assert_eq!(data.len(), 112);

    let f32_at = |off: usize| f32::from_le_bytes(data[off..off + 4].try_into().unwrap());
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
    // right after the tint, and the lit flag follows at 52.
    assert_eq!(f32_at(48), 0.75);
    assert_eq!(f32_at(52), 1.0);
    // Bytes 56..64 are the struct's alignment padding (zeroed by the
    // `vec![0u8; 96]` init), so the 16-byte-aligned uv_rect starts at 64;
    // a whole-texture sprite passes the identity rect.
    assert_eq!(f32_at(56), 0.0);
    assert_eq!(f32_at(64), 0.25);
    assert_eq!(f32_at(68), 0.5);
    assert_eq!(f32_at(72), 0.75);
    assert_eq!(f32_at(76), 1.0);
    // The emission vec4 follows the uv_rect, 16-byte aligned at 80 (80..96),
    // and the glow vec4 follows the emission at 96 (96..112).
    assert_eq!(f32_at(80), 0.6);
    assert_eq!(f32_at(84), 0.7);
    assert_eq!(f32_at(88), 0.8);
    assert_eq!(f32_at(92), 0.9);
    assert_eq!(f32_at(96), 0.65);
    assert_eq!(f32_at(100), 0.75);
    assert_eq!(f32_at(104), 0.85);
    assert_eq!(f32_at(108), 0.95);
}

/// A draw that asks for no separate glow gets its emission written into the
/// glow slot as well: the `None` case is resolved here rather than in the
/// shader, so a scene that wants nothing new ends up with the bytes it always
/// had, and the emissive entry point reads one vec4 and knows nothing about
/// the split.
#[test]
fn no_glow_means_the_emission_is_written_twice() {
    let inv = Transform::translate([1.5, -2.5]).invert().unwrap();
    let emission = Color {
        r: 0.5,
        g: 0.6,
        b: 0.7,
        a: 0.8,
    };
    let one = |glow: Option<Color>| {
        shape_uniform_data(
            inv,
            [7.0, 8.0],
            [9.0, 10.0],
            1.0,
            0.25,
            Color {
                r: 0.1,
                g: 0.2,
                b: 0.3,
                a: 0.4,
            },
            emission,
            glow,
            1.0,
        )
    };
    let unset = one(None);
    assert_eq!(
        &unset[80..96],
        &unset[96..112],
        "the glow slot should hold the emission when no glow was asked for"
    );
    assert_eq!(unset, one(Some(emission)));
}
