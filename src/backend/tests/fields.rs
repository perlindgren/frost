//! Light-field and occluder-field packing into their GPU buffers.

use super::super::*;
use super::*;
use crate::canvas::Occluder;
use crate::collision::{Convex, DISC_SIDES};
use crate::objects::*;

fn field_f32(data: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

/// The window the packer culls against in these fixtures: a familiar
/// 128², the size the GPU harnesses paint at.
const VIEW: [f32; 2] = [128.0, 128.0];

#[test]
fn light_field_packs_an_empty_header_with_the_ambient() {
    // No lights: just the 32-byte header Ã¢â‚¬â€ count 0, 12 padding bytes, the
    // ambient channels at 16..32.
    let ambient = Color {
        r: 0.3,
        g: 0.3,
        b: 0.3,
        a: 1.0,
    };
    let field = pack_light_field(&[], ambient);
    assert_eq!(field.count, 0);
    assert_eq!(field.data.len(), LIGHT_FIELD_HEADER);
    assert_eq!(u32::from_le_bytes(field.data[0..4].try_into().unwrap()), 0);
    assert_eq!(&field.data[4..16], &[0u8; 12]);
    assert_eq!(field_f32(&field.data, 16), 0.3);
    assert_eq!(field_f32(&field.data, 20), 0.3);
    assert_eq!(field_f32(&field.data, 24), 0.3);
    assert_eq!(field_f32(&field.data, 28), 1.0);
}

#[test]
fn light_field_packs_the_header_ambient_and_each_light_record() {
    // Two lights among non-light draws: count 2, the header, then one
    // 64-byte record per light, in call order — a `(x, y, radius,
    // intensity)` vec4, a `(r, g, b, penumbra)` vec4, the cone vec4
    // `(dir_x, dir_y, cos_half, feather)`, and the layer vec4
    // `(layers, 0, 0, 0)`.
    let ambient = Color {
        r: 0.1,
        g: 0.2,
        b: 0.4,
        a: 1.0,
    };
    let amber = Color {
        r: 1.0,
        g: 0.5,
        b: 0.0,
        a: 1.0,
    };
    let cyan = Color {
        r: 0.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    let draws = [
        Draw::Light {
            pos: [60.0, 30.0],
            radius: 16.0,
            intensity: 2.0,
            color: amber,
            penumbra: 8.0,
            dir: [0.6, 0.8],
            cos_half: 0.0,
            feather: 0.25,
            z: 0.0,

            layers: LightLayers::default(),
        },
        Draw::Circle {
            diagnostic: false,
            center: [5.0, 5.0],
            radius: 5.0,
            color: black(),
            z: 0.0,
        },
        Draw::Light {
            pos: [-12.5, 8.25],
            radius: 0.0,
            intensity: 0.5,
            color: cyan,
            penumbra: 0.0,
            dir: [0.0, -1.0],
            cos_half: -1.0,
            feather: 0.0,
            z: 1.0,

            layers: LightLayers::default(),
        },
        // A directional light: its negative radius is the sentinel the
        // shader reads, so the packer must write it through untouched.
        Draw::Light {
            pos: [0.0, 0.0],
            radius: -1.0,
            intensity: 1.25,
            color: amber,
            penumbra: 20.0,
            dir: [-0.8, 0.6],
            cos_half: -1.0,
            feather: 0.0,
            z: 2.0,

            layers: LightLayers::default(),
        },
    ];
    let field = pack_light_field(&draws, ambient);
    assert_eq!(field.count, 3);
    assert_eq!(field.data.len(), LIGHT_FIELD_HEADER + 3 * LIGHT_RECORD);
    // The header: count at 0, padding at 4..16, ambient at 16..32.
    assert_eq!(u32::from_le_bytes(field.data[0..4].try_into().unwrap()), 3);
    assert_eq!(&field.data[4..16], &[0u8; 12]);
    assert_eq!(field_f32(&field.data, 16), 0.1);
    assert_eq!(field_f32(&field.data, 20), 0.2);
    assert_eq!(field_f32(&field.data, 24), 0.4);
    assert_eq!(field_f32(&field.data, 28), 1.0);
    // Record 0 @ 32: (60, 30, 16, 2), then (1, 0.5, 0, 8) Ã¢â‚¬â€ the color
    // vec4's w now carries the penumbra radius Ã¢â‚¬â€ then the cone
    // (0.6, 0.8, 0, 0.25) Ã¢â‚¬â€ the feather rides the cone vec4's w.
    let r0 = LIGHT_FIELD_HEADER;
    assert_eq!(field_f32(&field.data, r0), 60.0);
    assert_eq!(field_f32(&field.data, r0 + 4), 30.0);
    assert_eq!(field_f32(&field.data, r0 + 8), 16.0);
    assert_eq!(field_f32(&field.data, r0 + 12), 2.0);
    assert_eq!(field_f32(&field.data, r0 + 16), 1.0);
    assert_eq!(field_f32(&field.data, r0 + 20), 0.5);
    assert_eq!(field_f32(&field.data, r0 + 24), 0.0);
    assert_eq!(field_f32(&field.data, r0 + 28), 8.0);
    assert_eq!(field_f32(&field.data, r0 + 32), 0.6);
    assert_eq!(field_f32(&field.data, r0 + 36), 0.8);
    assert_eq!(field_f32(&field.data, r0 + 40), 0.0);
    assert_eq!(field_f32(&field.data, r0 + 44), 0.25);
    // Record 1 @ 80: (-12.5, 8.25, 0, 0.5), then (0, 1, 1, 0) Ã¢â‚¬â€ the
    // penumbra is zero here Ã¢â‚¬â€ then the omni cone (0, -1, -1, 0).
    let r1 = LIGHT_FIELD_HEADER + LIGHT_RECORD;
    assert_eq!(field_f32(&field.data, r1), -12.5);
    assert_eq!(field_f32(&field.data, r1 + 4), 8.25);
    assert_eq!(field_f32(&field.data, r1 + 8), 0.0);
    assert_eq!(field_f32(&field.data, r1 + 12), 0.5);
    assert_eq!(field_f32(&field.data, r1 + 16), 0.0);
    assert_eq!(field_f32(&field.data, r1 + 20), 1.0);
    assert_eq!(field_f32(&field.data, r1 + 24), 1.0);
    assert_eq!(field_f32(&field.data, r1 + 28), 0.0);
    assert_eq!(field_f32(&field.data, r1 + 32), 0.0);
    assert_eq!(field_f32(&field.data, r1 + 36), -1.0);
    assert_eq!(field_f32(&field.data, r1 + 40), -1.0);
    assert_eq!(field_f32(&field.data, r1 + 44), 0.0);
    // Record 2 @ 128: the directional light Ã¢â‚¬â€ the sentinel radius (-1)
    // rides through the packer exactly, the position stays whatever the
    // node carried (the shader ignores it for this kind), and the penumbra
    // disk radius reaches the shader in the color vec4's w as usual.
    let r2 = LIGHT_FIELD_HEADER + 2 * LIGHT_RECORD;
    assert_eq!(field_f32(&field.data, r2), 0.0);
    assert_eq!(field_f32(&field.data, r2 + 4), 0.0);
    assert_eq!(field_f32(&field.data, r2 + 8), -1.0);
    assert_eq!(field_f32(&field.data, r2 + 12), 1.25);
    assert_eq!(field_f32(&field.data, r2 + 16), 1.0);
    assert_eq!(field_f32(&field.data, r2 + 20), 0.5);
    assert_eq!(field_f32(&field.data, r2 + 24), 0.0);
    assert_eq!(field_f32(&field.data, r2 + 28), 20.0);
    assert_eq!(field_f32(&field.data, r2 + 32), -0.8);
    assert_eq!(field_f32(&field.data, r2 + 36), 0.6);
    assert_eq!(field_f32(&field.data, r2 + 40), -1.0);
    assert_eq!(field_f32(&field.data, r2 + 44), 0.0);
}

#[test]
fn a_light_record_carries_its_layers_in_the_fourth_vec4() {
    // The 64-byte record's fourth vec4: x is the layer mask's raw bits —
    // the shader `bitcast<u32>`s it back, so every mask round-trips exact,
    // `ALL` included — and its other components stay zero padding. The
    // three vec4s the field always had ride through untouched: a layered
    // light and a pre-layer light share their bytes position for position.
    let amber = Color {
        r: 1.0,
        g: 0.5,
        b: 0.0,
        a: 1.0,
    };
    let layers = LightLayers::layer(0) | LightLayers::layer(3);
    let draws = [Draw::Light {
        pos: [60.0, 30.0],
        radius: 16.0,
        intensity: 2.0,
        color: amber,
        penumbra: 8.0,
        dir: [0.6, 0.8],
        cos_half: 0.0,
        feather: 0.25,
        z: 0.0,
        layers,
    }];
    let field = pack_light_field(&draws, black());
    assert_eq!(field.data.len(), LIGHT_FIELD_HEADER + LIGHT_RECORD);
    let r0 = LIGHT_FIELD_HEADER;
    assert_eq!(field_f32(&field.data, r0), 60.0);
    assert_eq!(field_f32(&field.data, r0 + 4), 30.0);
    assert_eq!(field_f32(&field.data, r0 + 8), 16.0);
    assert_eq!(field_f32(&field.data, r0 + 12), 2.0);
    assert_eq!(field_f32(&field.data, r0 + 16), 1.0);
    assert_eq!(field_f32(&field.data, r0 + 20), 0.5);
    assert_eq!(field_f32(&field.data, r0 + 28), 8.0);
    assert_eq!(field_f32(&field.data, r0 + 32), 0.6);
    assert_eq!(field_f32(&field.data, r0 + 36), 0.8);
    assert_eq!(field_f32(&field.data, r0 + 44), 0.25);
    // The fourth vec4, read as bits: layers 0 and 3 land as bits 0 and 3.
    assert_eq!(
        u32::from_le_bytes(field.data[r0 + 48..r0 + 52].try_into().unwrap()),
        layers.bits()
    );
    assert_eq!(&field.data[r0 + 52..r0 + 64], &[0u8; 12]);
}

#[test]
fn occluder_field_packs_an_empty_header() {
    // No occluders: just the 16-byte header Ã¢â‚¬â€ count 0, 12 padding bytes Ã¢â‚¬â€
    // and no records.
    let field = pack_occluder_field(
        &[Draw::Light {
            pos: [10.0, 20.0],
            radius: 5.0,
            intensity: 1.0,
            color: black(),
            penumbra: 0.0,
            dir: [1.0, 0.0],
            cos_half: -1.0,
            feather: 0.0,
            z: 0.0,

            layers: LightLayers::default(),
        }],
        &[],
        [128.0, 128.0],
    );
    // One light, no occluders: header (record count 0, light count 1, 8
    // padding bytes), then the range table's single entry — a block that
    // starts where it ends, at the tail's vec4 1, right after the table.
    // No records anywhere; the shader's walk of this light is one table
    // load and no records.
    assert_eq!(field.count, 0);
    assert_eq!(field.lights, 1);
    assert_eq!(field.data.len(), OCCLUDER_FIELD_HEADER + 16);
    assert_eq!(u32::from_le_bytes(field.data[0..4].try_into().unwrap()), 0);
    assert_eq!(u32::from_le_bytes(field.data[4..8].try_into().unwrap()), 1);
    assert_eq!(&field.data[8..16], &[0u8; 8]);
    assert_eq!(field_f32(&field.data, 16), 1.0);
    assert_eq!(field_f32(&field.data, 20), 1.0);
}

#[test]
fn occluder_field_packs_flagged_rectangles_with_the_inverse_transform() {
    // Two flagged rectangles among unflagged, non-rectangle, and degenerate
    // draws: count 2, the header, then one 48-byte record per occluder, in
    // call order.
    let translated = Draw::Shape {
        diagnostic: false,
        emission: black(),
        glow: None,
        world: Transform::translate([10.0, 20.0]),
        center: [3.0, -4.0],
        params: [5.0, 2.0],
        kind: 1.0,
        aa: 0.0,
        color: black(),
        lit: 0.0,
        occludes: 1.0,
        z: 0.0,

        lit_layers: LightLayers::default(),
        occludes_layers: LightLayers::default(),
    };
    let rotated = Draw::Shape {
        diagnostic: false,
        emission: black(),
        glow: None,
        world: Transform::rotate(std::f32::consts::FRAC_PI_2),
        center: [1.0, 2.0],
        params: [4.0, 6.0],
        kind: 1.0,
        aa: 0.0,
        color: black(),
        lit: 1.0,
        occludes: 1.0,
        z: 0.0,

        lit_layers: LightLayers::default(),
        occludes_layers: LightLayers::default(),
    };
    let unflagged = Draw::Shape {
        diagnostic: false,
        emission: black(),
        glow: None,
        world: Transform::identity(),
        center: [0.0, 0.0],
        params: [1.0, 1.0],
        kind: 1.0,
        aa: 0.0,
        color: black(),
        lit: 0.0,
        occludes: 0.0,
        z: 0.0,

        lit_layers: LightLayers::default(),
        occludes_layers: LightLayers::default(),
    };
    // Flagged, but a sprite: its silhouette is the texture's, and the field
    // has nothing to pack that into — the declare channel is for this.
    let sprite = Draw::Shape {
        diagnostic: false,
        emission: black(),
        glow: None,
        world: Transform::identity(),
        center: [0.0, 0.0],
        params: [1.0, 0.0],
        kind: 2.0,
        aa: 0.0,
        color: black(),
        lit: 0.0,
        occludes: 1.0,
        z: 0.0,

        lit_layers: LightLayers::default(),
        occludes_layers: LightLayers::default(),
    };
    // Flagged, but collapsed to a line: the inverse does not exist, so the
    // shape occludes nothing.
    let degenerate = Draw::Shape {
        diagnostic: false,
        emission: black(),
        glow: None,
        world: Transform::scale([0.0, 1.0]),
        center: [0.0, 0.0],
        params: [1.0, 1.0],
        kind: 1.0,
        aa: 0.0,
        color: black(),
        lit: 0.0,
        occludes: 1.0,
        z: 0.0,

        lit_layers: LightLayers::default(),
        occludes_layers: LightLayers::default(),
    };
    let field = pack_occluder_field(
        &[translated, unflagged, rotated, sprite, degenerate],
        &[],
        VIEW,
    );
    assert_eq!(field.count, 2);
    assert_eq!(
        field.data.len(),
        OCCLUDER_FIELD_HEADER + 2 * OCCLUDER_RECORD
    );
    // The header: count at 0, padding at 4..16.
    assert_eq!(u32::from_le_bytes(field.data[0..4].try_into().unwrap()), 2);
    assert_eq!(&field.data[4..16], &[0u8; 12]);
    // Record 0 @ 16: the translate's inverse keeps the identity matrix,
    // column-major (1, 0, 0, 1), the negated translation, the local center,
    // the local half-extents, and two zero tail bytes.
    let o = OCCLUDER_FIELD_HEADER;
    assert_eq!(field_f32(&field.data, o), 1.0);
    assert_eq!(field_f32(&field.data, o + 4), 0.0);
    assert_eq!(field_f32(&field.data, o + 8), 0.0);
    assert_eq!(field_f32(&field.data, o + 12), 1.0);
    assert_eq!(field_f32(&field.data, o + 16), -10.0);
    assert_eq!(field_f32(&field.data, o + 20), -20.0);
    assert_eq!(field_f32(&field.data, o + 24), 3.0);
    assert_eq!(field_f32(&field.data, o + 28), -4.0);
    assert_eq!(field_f32(&field.data, o + 32), 5.0);
    assert_eq!(field_f32(&field.data, o + 36), 2.0);
    assert_eq!(field_f32(&field.data, o + 40), 0.0);
    // The padding slot of vec4 #2 carries the layer mask, not a zero: an
    // occluder that names no layers shadows layer 0 — bit 0, one.
    assert_eq!(
        u32::from_le_bytes(field.data[o + 44..o + 48].try_into().unwrap()),
        LightLayers::default().bits()
    );
    // Record 1 @ 64: the 90Ã‚Â° rotation's inverse is the -90Ã‚Â° rotation,
    // column-major (m00, m10, m01, m11) = (0, -1, 1, 0); the translation is
    // zero. The diagonals are cos(90Ã‚Â°) in f32, so they are ~1e-8 rather
    // than exactly 0.
    let o = OCCLUDER_FIELD_HEADER + OCCLUDER_RECORD;
    assert!(field_f32(&field.data, o).abs() < 1e-6);
    assert_eq!(field_f32(&field.data, o + 4), -1.0);
    assert_eq!(field_f32(&field.data, o + 8), 1.0);
    assert!(field_f32(&field.data, o + 12).abs() < 1e-6);
    assert_eq!(field_f32(&field.data, o + 16), 0.0);
    assert_eq!(field_f32(&field.data, o + 20), 0.0);
    assert_eq!(field_f32(&field.data, o + 24), 1.0);
    assert_eq!(field_f32(&field.data, o + 28), 2.0);
    assert_eq!(field_f32(&field.data, o + 32), 4.0);
    assert_eq!(field_f32(&field.data, o + 36), 6.0);
    assert_eq!(field_f32(&field.data, o + 40), 0.0);
    assert_eq!(
        u32::from_le_bytes(field.data[o + 44..o + 48].try_into().unwrap()),
        LightLayers::default().bits()
    );
}

#[test]
fn occluder_field_appends_declared_occluders_after_the_drawn_ones() {
    // One flagged rectangle plus two declared occluders: three records, the
    // drawn one first, the declared ones in declaration order.
    let drawn = Draw::Shape {
        diagnostic: false,
        emission: black(),
        glow: None,
        world: Transform::identity(),
        center: [0.0, 0.0],
        params: [10.0, 10.0],
        kind: 1.0,
        aa: 0.0,
        color: black(),
        lit: 0.0,
        occludes: 1.0,
        z: 0.0,

        lit_layers: LightLayers::default(),
        occludes_layers: LightLayers::default(),
    };
    let declared = [
        Occluder {
            world: Transform::translate([50.0, -20.0]),
            center: [1.0, 2.0],
            half: [10.0, 20.0],
            polygon: None,

            layers: LightLayers::default(),
        },
        Occluder {
            world: Transform::rotate(std::f32::consts::FRAC_PI_2),
            center: [0.0, 0.0],
            half: [4.0, 6.0],
            polygon: None,

            layers: LightLayers::default(),
        },
    ];
    let field = pack_occluder_field(&[drawn], &declared, VIEW);
    assert_eq!(field.count, 3);
    assert_eq!(
        field.data.len(),
        OCCLUDER_FIELD_HEADER + 3 * OCCLUDER_RECORD
    );
    assert_eq!(u32::from_le_bytes(field.data[0..4].try_into().unwrap()), 3);

    // Record 0: the drawn rectangle, identity inverse, nothing translated.
    let r0 = OCCLUDER_FIELD_HEADER;
    assert_eq!(field_f32(&field.data, r0 + 16), 0.0);
    assert_eq!(field_f32(&field.data, r0 + 32), 10.0);

    // Record 1: the declared occluder's own center and half-extents ride
    // through untouched, and the inverse carries the negated translation.
    let r1 = r0 + OCCLUDER_RECORD;
    assert_eq!(field_f32(&field.data, r1 + 16), -50.0);
    assert_eq!(field_f32(&field.data, r1 + 20), 20.0);
    assert_eq!(field_f32(&field.data, r1 + 24), 1.0);
    assert_eq!(field_f32(&field.data, r1 + 28), 2.0);
    assert_eq!(field_f32(&field.data, r1 + 32), 10.0);
    assert_eq!(field_f32(&field.data, r1 + 36), 20.0);

    // Record 2: a quarter turn, whose inverse is the turn back - the linear
    // part column-major is (0, -1, 1, 0), within the f32 of a quarter turn.
    let r2 = r1 + OCCLUDER_RECORD;
    let near = |offset: usize, want: f32| (field_f32(&field.data, offset) - want).abs() < 1e-6;
    assert!(near(r2, 0.0));
    assert!(near(r2 + 4, -1.0));
    assert!(near(r2 + 8, 1.0));
    assert!(near(r2 + 12, 0.0));
    assert_eq!(field_f32(&field.data, r2 + 32), 4.0);
    assert_eq!(field_f32(&field.data, r2 + 36), 6.0);
}

#[test]
fn a_declared_occluder_carries_its_layers_in_the_records_padding() {
    // The occluder record's vec4 #2 ends with the layer mask where the
    // zero padding used to be: a shadow declared on layer 1 lands only on
    // receivers listening on layer 1, and the mask must arrive bit-exact.
    // The record around it — extents and edge count — is unchanged, and
    // the record after it keeps its own mask: the walker strides by the
    // edge count, not by the mask.
    let declared = [
        Occluder {
            world: Transform::translate([10.0, 0.0]),
            center: [0.0, 0.0],
            half: [4.0, 6.0],
            polygon: None,
            layers: LightLayers::layer(1),
        },
        Occluder {
            world: Transform::identity(),
            center: [0.0, 0.0],
            half: [1.0, 1.0],
            polygon: None,
            layers: LightLayers::default(),
        },
    ];
    let field = pack_occluder_field(&[], &declared, VIEW);
    assert_eq!(field.count, 2);
    let r0 = OCCLUDER_FIELD_HEADER;
    assert_eq!(field_f32(&field.data, r0 + 32), 4.0);
    assert_eq!(field_f32(&field.data, r0 + 36), 6.0);
    assert_eq!(
        field_f32(&field.data, r0 + 40),
        0.0,
        "a box still carries no edges"
    );
    assert_eq!(
        u32::from_le_bytes(field.data[r0 + 44..r0 + 48].try_into().unwrap()),
        LightLayers::layer(1).bits()
    );
    let r1 = r0 + OCCLUDER_RECORD;
    assert_eq!(
        u32::from_le_bytes(field.data[r1 + 44..r1 + 48].try_into().unwrap()),
        LightLayers::default().bits(),
        "an occluder that names no layers shadows layer 0, bit 0 — not nothing"
    );
}

#[test]
fn occluder_field_skips_a_declared_occluder_it_cannot_invert() {
    // A declared occluder under a collapsed transform occludes nothing, and
    // the field keeps no hole where its record would have gone: the one that
    // does stand is packed first.
    let declared = [
        Occluder {
            world: Transform::scale([0.0, 1.0]),
            center: [0.0, 0.0],
            half: [10.0, 10.0],
            polygon: None,

            layers: LightLayers::default(),
        },
        Occluder {
            world: Transform::translate([7.0, 0.0]),
            center: [0.0, 0.0],
            half: [3.0, 4.0],
            polygon: None,

            layers: LightLayers::default(),
        },
    ];
    let field = pack_occluder_field(&[], &declared, VIEW);
    assert_eq!(field.count, 1);
    assert_eq!(field.data.len(), OCCLUDER_FIELD_HEADER + OCCLUDER_RECORD);
    assert_eq!(field_f32(&field.data, OCCLUDER_FIELD_HEADER + 16), -7.0);
    assert_eq!(field_f32(&field.data, OCCLUDER_FIELD_HEADER + 32), 3.0);
    assert_eq!(field_f32(&field.data, OCCLUDER_FIELD_HEADER + 36), 4.0);
}

#[test]
fn field_buffer_sizes_floor_at_the_wgsl_minimum_binding_size() {
    // A lightless or occluder-less frame still binds both field buffers,
    // and the WGSL layout's minimum binding size is the header plus one
    // vec4 Ã¢â‚¬â€ the buffer must never be sized below it, or wgpu rejects the
    // bind group.
    assert_eq!(light_field_buffer_size(0), LIGHT_FIELD_BUFFER_MIN as u64);
    assert_eq!(
        light_field_buffer_size(1),
        (LIGHT_FIELD_HEADER + LIGHT_RECORD) as u64
    );
    assert_eq!(
        light_field_buffer_size(3),
        (LIGHT_FIELD_HEADER + 3 * LIGHT_RECORD) as u64
    );
    // The occluder field has no count-based size any more — per-light
    // blocks and duplicated records make the packed bytes the ground
    // truth — but the floor still rules: app.rs sizes the buffer at the
    // data length rounded up to the same minimum, and the emptiest pack
    // is a bare header, well below it.
    let empty = pack_occluder_field(&[], &[], VIEW);
    assert_eq!(empty.data.len(), OCCLUDER_FIELD_HEADER);
    let buffer_size = (empty.data.len().max(OCCLUDER_FIELD_BUFFER_MIN)) as u64;
    assert_eq!(buffer_size, OCCLUDER_FIELD_BUFFER_MIN as u64);
}

#[test]
fn a_polygon_record_carries_its_edges_and_the_next_record_follows_them() {
    // A box keeps the 48 bytes it always had and a polygon pays for its own
    // edges, so the field is walked with a cursor rather than indexed by a
    // stride: a box's edge count sits in the component that used to be
    // padding - zero of them, which is what a box means - and the record after
    // a polygon starts past its last edge.
    // A hexagon rather than the widest shape the type allows, because the
    // claim under test is that a record carries *its* edges and not the most a
    // shape could have: the stride follows the count, not the cap.
    let corners: Vec<[f32; 2]> = (0..6)
        .map(|k| {
            let a = std::f32::consts::TAU * k as f32 / 6.0;
            [10.0 * a.cos(), 10.0 * a.sin()]
        })
        .collect();
    let rock = Convex::new(&corners).expect("a regular hexagon is convex");
    let planes = rock.planes();
    let edges = planes.len();
    assert_eq!(edges, 6, "a hexagon ships six edges, not the cap of 16");
    let bounds = rock.bounds_as_box();
    let declared = [
        Occluder {
            world: Transform::translate([10.0, 0.0]),
            center: [0.0, 0.0],
            half: [3.0, 4.0],
            polygon: None,

            layers: LightLayers::default(),
        },
        Occluder {
            world: Transform::identity(),
            center: bounds.center,
            half: bounds.half,
            polygon: Some(rock),

            layers: LightLayers::default(),
        },
        Occluder {
            world: Transform::translate([0.0, 5.0]),
            center: [1.0, 1.0],
            half: [2.0, 2.0],
            polygon: None,

            layers: LightLayers::default(),
        },
    ];
    let field = pack_occluder_field(&[], &declared, VIEW);
    assert_eq!(field.count, 3);
    assert_eq!(
        field.data.len(),
        OCCLUDER_FIELD_HEADER + 3 * OCCLUDER_RECORD + edges * OCCLUDER_PLANE,
        "the field should hold three records and sixteen edges, no more"
    );

    let first = OCCLUDER_FIELD_HEADER;
    assert_eq!(
        field_f32(&field.data, first + 40),
        0.0,
        "a box carries no edges"
    );
    let polygon = first + OCCLUDER_RECORD;
    assert_eq!(field_f32(&field.data, polygon + 40), edges as f32);
    assert_eq!(
        field_f32(&field.data, polygon + 32),
        bounds.half[0],
        "the record's half-extents are the box around the polygon, the cheap \
         reject the shader tests before it clips a single edge"
    );
    let after = polygon + OCCLUDER_RECORD + edges * OCCLUDER_PLANE;
    assert_eq!(field_f32(&field.data, after + 40), 0.0);
    assert_eq!(
        field_f32(&field.data, after + 32),
        2.0,
        "and the box after a polygon is a box"
    );
    assert_eq!(
        field_f32(&field.data, after + 20),
        -5.0,
        "carrying its own inverse"
    );

    // The edges themselves, verbatim from the CPU's half-planes: an outward
    // unit normal and its offset, in the occluder's own space.
    for (k, plane) in planes.iter().enumerate() {
        let at = polygon + OCCLUDER_RECORD + k * OCCLUDER_PLANE;
        assert_eq!(field_f32(&field.data, at), plane[0]);
        assert_eq!(field_f32(&field.data, at + 4), plane[1]);
        assert_eq!(field_f32(&field.data, at + 8), plane[2]);
        assert_eq!(field_f32(&field.data, at + 12), 0.0);
    }
}

#[test]
fn a_flagged_circle_is_packed_as_the_polygon_cut_from_it() {
    // Painting a circle and flagging it used to be a silent no-op: the field
    // only had room for rectangles. It now has edges, so the round silhouette
    // the author drew reaches the light as the sixteen-gon inscribed in it,
    // with the record's box left holding the bounds the shader rejects a ray
    // against before it clips a single edge.
    let radius = 12.0;
    let painted = Draw::Shape {
        diagnostic: false,
        emission: black(),
        glow: None,
        world: Transform::translate([10.0, 20.0]),
        center: [3.0, -4.0],
        params: [radius, radius],
        kind: 0.0,
        aa: 0.0,
        color: black(),
        lit: 0.0,
        occludes: 1.0,
        z: 0.0,

        lit_layers: LightLayers::default(),
        occludes_layers: LightLayers::default(),
    };
    let want = Convex::disc([3.0, -4.0], radius);
    let field = pack_occluder_field(&[painted], &[], VIEW);
    assert_eq!(field.count, 1);
    assert_eq!(
        field.data.len(),
        OCCLUDER_FIELD_HEADER + OCCLUDER_RECORD + DISC_SIDES * OCCLUDER_PLANE,
        "a painted circle should cost one record and its own sixteen edges"
    );
    let record = OCCLUDER_FIELD_HEADER;
    assert_eq!(field_f32(&field.data, record + 40), DISC_SIDES as f32);
    assert_eq!(
        field_f32(&field.data, record + 24),
        3.0,
        "the polygon's centre"
    );
    assert_eq!(field_f32(&field.data, record + 32), radius);
    assert_eq!(field_f32(&field.data, record + 36), radius);
    for (k, plane) in want.planes().iter().enumerate() {
        let at = record + OCCLUDER_RECORD + k * OCCLUDER_PLANE;
        assert_eq!(field_f32(&field.data, at), plane[0]);
        assert_eq!(field_f32(&field.data, at + 4), plane[1]);
        assert_eq!(field_f32(&field.data, at + 8), plane[2]);
    }
}

#[test]
fn a_painted_circle_without_a_usable_radius_casts_nothing_instead_of_panicking() {
    // A radius tweens through zero, and a circle is drawn at infinite size by
    // someone who meant the opposite of what they wrote. Neither is a shape the
    // field can hold - and unlike a rectangle, whose degenerate record simply
    // hits nothing, cutting a polygon out of nothing is an error the polygon
    // builder rightly refuses. The frame path takes the refusal as a skip, the
    // same answer a collapsed transform gets, rather than as a panic that stops
    // the run over a pixel nobody can see.
    let bare = |radius: f32| Draw::Shape {
        diagnostic: false,
        emission: black(),
        glow: None,
        world: Transform::identity(),
        center: [0.0, 0.0],
        params: [radius, radius],
        kind: 0.0,
        aa: 0.0,
        color: black(),
        lit: 0.0,
        occludes: 1.0,
        z: 0.0,

        lit_layers: LightLayers::default(),
        occludes_layers: LightLayers::default(),
    };
    for radius in [0.0, -4.0, f32::NAN, f32::INFINITY] {
        let field = pack_occluder_field(&[bare(radius)], &[], VIEW);
        assert_eq!(
            field.count, 0,
            "a circle of radius {radius} should be skipped, not packed"
        );
        assert_eq!(
            field.data.len(),
            OCCLUDER_FIELD_HEADER,
            "and leave no record behind"
        );
    }
    // While the real one beside them still stands.
    let field = pack_occluder_field(&[bare(0.0), bare(6.0)], &[], VIEW);
    assert_eq!(field.count, 1, "only the usable circle reaches the field");
}

/// A lamp drawn the way the packer reads one.
fn cull_light(pos: [f32; 2], radius: f32, penumbra: f32, dir: [f32; 2]) -> Draw {
    Draw::Light {
        pos,
        radius,
        intensity: 1.0,
        color: black(),
        penumbra,
        dir,
        cos_half: -1.0,
        feather: 0.0,
        z: 0.0,

        layers: LightLayers::default(),
    }
}

/// A declared box at `center`, standing still.
fn cull_box(center: [f32; 2], half: f32) -> Occluder {
    Occluder {
        world: Transform::identity(),
        center,
        half: [half, half],
        polygon: None,

        layers: LightLayers::default(),
    }
}

/// The nine points of the shader's tap grid at unit spacing — every
/// point a penumbra ray starts from, before its scaling by the light's
/// penumbra. Mirrors `SHADOW_TAPS` in `shaders/lighting.wgsl`.
const TAP_GRID: [[f32; 2]; 9] = [
    [-1.0, -1.0],
    [0.0, -1.0],
    [1.0, -1.0],
    [-1.0, 0.0],
    [0.0, 0.0],
    [1.0, 0.0],
    [-1.0, 1.0],
    [0.0, 1.0],
    [1.0, 1.0],
];

/// The shared cast: four lights — a lamp inside the window, a lamp off
/// its right edge, a penumbra lamp mid-window, and a sun shining up
/// through the window from far below — and seven occluders spread across
/// the plane: two inside, one at the top-right corner, one far below in
/// the sun's beam, a disc among the winners, one due east of the
/// penumbra lamp past its falloff but inside its penumbra's reach, and
/// one far off to the right where no ray of this frame ever travels.
fn cull_cast() -> (Vec<Draw>, Vec<Occluder>) {
    let draws = vec![
        cull_light([-40.0, -30.0], 60.0, 0.0, [1.0, 0.0]),
        cull_light([300.0, 0.0], 40.0, 0.0, [1.0, 0.0]),
        cull_light([64.0, 64.0], 20.0, 18.0, [1.0, 0.0]),
        cull_light([0.0, 0.0], -1.0, 0.0, [0.0, 1.0]),
    ];
    let disc = Occluder {
        world: Transform::identity(),
        center: [20.0, -10.0],
        half: [5.0, 5.0],
        polygon: Some(Convex::disc([20.0, -10.0], 5.0)),

        layers: LightLayers::default(),
    };
    let declared = vec![
        cull_box([0.0, 0.0], 8.0),
        cull_box([-30.0, -20.0], 8.0),
        cull_box([55.0, 55.0], 6.0),
        cull_box([0.0, -800.0], 6.0),
        disc,
        cull_box([300.0, 0.0], 8.0),
        cull_box([92.0, 64.0], 4.0),
    ];
    (draws, declared)
}

/// The occluders' world bounds, restated from the cast: with identity
/// transforms these are the boxes themselves (and the disc's bounds).
fn cull_bounds() -> Vec<[f32; 4]> {
    [
        ([0.0, 0.0], 8.0),
        ([-30.0, -20.0], 8.0),
        ([55.0, 55.0], 6.0),
        ([0.0, -800.0], 6.0),
        ([20.0, -10.0], 5.0),
        ([300.0, 0.0], 8.0),
        ([92.0, 64.0], 4.0),
    ]
    .into_iter()
    .map(|(c, h)| [c[0] - h, c[1] - h, c[0] + h, c[1] + h])
    .collect()
}

/// Byte ranges of the records packed between the field's tail-relative
/// `vec4` range `start..end`.
fn block_records(field: &OccluderField, start: u32, end: u32) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut base = start as usize;
    while base < end as usize {
        let off = OCCLUDER_FIELD_HEADER + base * 16;
        // `rec.z` holds the plane count as a float, exactly as the shader
        // reads it back with `u32(rec.z)`.
        let planes = field_f32(&field.data, off + 40) as u32;
        let span = OCCLUDER_RECORD + planes as usize * OCCLUDER_PLANE;
        out.push((off, span));
        base += 3 + planes as usize;
    }
    out
}

/// The records of a lightless pack — the master list, table-free: with
/// no table to skip it starts at the tail's first vec4 and runs to the
/// end of the field.
fn master_records(field: &OccluderField) -> Vec<(usize, usize)> {
    let end = ((field.data.len() - OCCLUDER_FIELD_HEADER) / 16) as u32;
    block_records(field, 0, end)
}

/// Whether the open segment `(l, p)` passes through the box `b` the way
/// the shader counts a block: a positive interval that reaches into
/// (0, 1).
fn seg_blocks(l: [f32; 2], p: [f32; 2], b: &[f32; 4]) -> bool {
    let d = [p[0] - l[0], p[1] - l[1]];
    let (mut tmin, mut tmax) = (0.0f32, 1.0f32);
    for axis in 0..2 {
        let (lo, hi) = (b[axis], b[axis + 2]);
        if d[axis].abs() > 1e-9 {
            let t1 = (lo - l[axis]) / d[axis];
            let t2 = (hi - l[axis]) / d[axis];
            tmin = tmin.max(t1.min(t2));
            tmax = tmax.min(t1.max(t2));
        } else if l[axis] < lo || l[axis] > hi {
            return false;
        }
    }
    tmax > tmin && tmax > 0.0 && tmin < 1.0
}

/// The safety property of the per-light cull: an occluder a light's block
/// dropped must be one that light could never have needed — for every
/// pixel the light will ever light, every shadow ray it could fire from
/// its penumbra disk to that pixel misses the occluder's world bounds
/// entirely. Pixels past a ranged light's falloff radius ask it nothing
/// (the reach reject in `light_mix`), so a ray to one blocks nothing the
/// frame will show, and they are out of the for-every. The GPU walk only
/// ever sees the block, so this is the whole argument that culling cannot
/// lose a shadow, and it is the reason `pack_occluder_field` is allowed to
/// exist.
#[test]
fn the_cull_keeps_every_occluder_that_could_block() {
    let (draws, declared) = cull_cast();
    let field = pack_occluder_field(&draws, &declared, VIEW);
    let master = pack_occluder_field(&[], &declared, VIEW);
    let masters = master_records(&master);
    assert_eq!(masters.len(), declared.len(), "the master list is the cast");
    let bounds = cull_bounds();
    let mut saw_an_exclusion = false;
    for (li, light) in draws.iter().enumerate() {
        let Draw::Light {
            pos,
            radius,
            penumbra,
            dir,
            ..
        } = light
        else {
            unreachable!("the cast is all lights")
        };
        // The ray origins a shader may fire from this light: the light
        // itself, plus every point of the tap grid scaled by the penumbra
        // — all nine of them, whatever rung the frame rides, since a
        // superset asks no promise the shader would not keep. (A
        // directional light ignores these; its source lies back along
        // the travel axis, computed per pixel below.)
        let mut origins = vec![*pos];
        if *penumbra > 0.0 {
            origins.extend(
                TAP_GRID
                    .iter()
                    .map(|t| [pos[0] + t[0] * penumbra, pos[1] + t[1] * penumbra]),
            );
        }
        let (start, end) = (
            field_f32(&field.data, OCCLUDER_FIELD_HEADER + li * 16) as u32,
            field_f32(&field.data, OCCLUDER_FIELD_HEADER + li * 16 + 4) as u32,
        );
        let kept: Vec<(usize, usize)> = block_records(&field, start, end);
        for (k, m) in masters.iter().enumerate() {
            let present = kept
                .iter()
                .any(|(off, span)| field.data[*off..*off + *span] == master.data[m.0..m.0 + m.1]);
            if present {
                continue;
            }
            saw_an_exclusion = true;
            // Culled: then no pixel the light reaches ever needed it.
            // Pixels on a dense grid across the window, plus its exact
            // corners.
            let mut xs: Vec<f32> = (0..=16).map(|i| i as f32 * 8.0).collect();
            xs.extend([0.0, 128.0]);
            let mut ys: Vec<f32> = (0..=16).map(|i| i as f32 * 8.0).collect();
            ys.extend([0.0, 128.0]);
            for x in &xs {
                for y in &ys {
                    let p = [*x, *y];
                    if *radius < 0.0 {
                        let l = [p[0] - dir[0] * SHADOW_RANGE, p[1] - dir[1] * SHADOW_RANGE];
                        assert!(
                            !seg_blocks(l, p, &bounds[k]),
                            "light {li} culled occluder {k}, yet its ray to {p:?} blocks there",
                        );
                        continue;
                    }
                    // Past the falloff radius the shader asks this light
                    // nothing, so a blocker out there blocks nothing the
                    // frame will show: the for-every is over lit pixels.
                    let rr = radius.max(1e-4);
                    let dx = p[0] - pos[0];
                    let dy = p[1] - pos[1];
                    if dx * dx + dy * dy >= rr * rr {
                        continue;
                    }
                    for q in &origins {
                        assert!(
                            !seg_blocks(*q, p, &bounds[k]),
                            "light {li} culled occluder {k}, yet its ray from {q:?} to \
                             {p:?} blocks there",
                        );
                    }
                }
            }
        }
    }
    assert!(
        saw_an_exclusion,
        "the cast must give the cull something to do"
    );
}

/// The cull's exact shape on the shared cast — which occluders each
/// light's block holds. The safety property says the packer may drop
/// nothing it needed; this says it also drops what it does not need, and
/// names the three rules that decide membership: a ranged light admits
/// the records within its contributing-ray disk, which the penumbra
/// widens past the falloff radius; a directional light admits the
/// records inside its swept beam; and no light admits a record that no
/// ray of the frame ever travels to.
#[test]
fn the_culls_admit_by_reach_penumbra_and_beam() {
    let (draws, declared) = cull_cast();
    let field = pack_occluder_field(&draws, &declared, VIEW);
    let master = pack_occluder_field(&[], &declared, VIEW);
    let masters = master_records(&master);
    // Block membership, read back as master indices by byte identity.
    let block_of = |li: usize| -> Vec<usize> {
        let start = field_f32(&field.data, OCCLUDER_FIELD_HEADER + li * 16) as u32;
        let end = field_f32(&field.data, OCCLUDER_FIELD_HEADER + li * 16 + 4) as u32;
        block_records(&field, start, end)
            .into_iter()
            .map(|(off, span)| {
                masters
                    .iter()
                    .position(|(moff, mspan)| {
                        *mspan == span
                            && field.data[off..off + span] == master.data[*moff..*moff + span]
                    })
                    .expect("every block record came from the master list")
            })
            .collect()
    };
    // The inner lamp: the two boxes and the disc within its 60-pixel
    // reach; the corner box, the deep box and its neighbour's lamp all
    // sit past that disk and walk no ray of this lamp's.
    assert_eq!(block_of(0), [0, 1, 4], "the lamp keeps what it reaches");
    // The lamp off the right edge: only the box it stands inside. The
    // window's boxes are visible to it but unreachable — the hull keeps
    // them in no block the falloff has already emptied.
    assert_eq!(block_of(1), [5], "the far lamp keeps only its own box");
    // The penumbra lamp mid-window: the corner box lies within its 20
    // pixels of falloff; the east box stands at 24, past the falloff but
    // inside `18 * √2` of penumbra — the disk a corner tap's ray can
    // still leave and so the one disk the block must cover.
    assert_eq!(
        block_of(2),
        [2, 6],
        "the penumbra keeps past the falloff, but only to the corner taps"
    );
    // The sun: everything its parallel beam sweeps — the window's boxes,
    // the disc, the east box standing in its column, and the deep box in
    // the light below the frame. The box west of the window and the lamp
    // far east stand outside the beam's column and walk no parallel ray.
    assert_eq!(
        block_of(3),
        [0, 2, 3, 4, 6],
        "the beam keeps its beam and nothing wider"
    );
}

/// The field walks like its header says it should: the table starts at
/// the tail's first vec4, blocks run back to back in light order with no
/// gaps or overlaps, every block's records advance by exactly their plane
/// counts, the record total matches the header, and each block is an
/// in-order subsequence of the master list.
#[test]
fn the_range_table_and_blocks_walk_their_documented_layout() {
    let (draws, declared) = cull_cast();
    let field = pack_occluder_field(&draws, &declared, VIEW);
    let master = pack_occluder_field(&[], &declared, VIEW);
    let masters = master_records(&master);
    assert_eq!(field.lights as usize, draws.len());
    // The table is the tail's first `lights` vec4s; records start right
    // after it.
    let mut expected_start = field.lights as usize;
    let mut total = 0usize;
    let mut prev_end = None;
    for li in 0..field.lights as usize {
        let start = field_f32(&field.data, OCCLUDER_FIELD_HEADER + li * 16) as usize;
        let end = field_f32(&field.data, OCCLUDER_FIELD_HEADER + li * 16 + 4) as usize;
        assert_eq!(start, expected_start, "block {li} starts where it must");
        assert!(end >= start, "block {li} is a range");
        if let Some(prev) = prev_end {
            assert_eq!(start, prev, "blocks are contiguous");
        }
        let recs = block_records(&field, start as u32, end as u32);
        total += recs.len();
        // An in-order subsequence of the master list, byte for byte.
        let mut next = 0usize;
        let mut last = -1isize;
        for (off, span) in &recs {
            let found = masters[next..].iter().position(|m| {
                m.1 == *span && master.data[m.0..m.0 + m.1] == field.data[*off..*off + *span]
            });
            let Some(rel) = found else {
                panic!("block {li} holds a record the master list does not");
            };
            assert!((next + rel) as isize > last, "order within a block holds");
            last = (next + rel) as isize;
            next += rel + 1;
        }
        prev_end = Some(end);
        expected_start = end;
    }
    let end_of_tail = (field.data.len() - OCCLUDER_FIELD_HEADER) / 16;
    assert_eq!(prev_end, Some(end_of_tail), "the last block ends the tail");
    assert_eq!(total, field.count as usize, "the header counts its records");
}
