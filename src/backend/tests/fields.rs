//! Light-field and occluder-field packing into their GPU buffers.

use super::super::*;
use super::*;
use crate::canvas::Occluder;
use crate::collision::Convex;
use crate::objects::*;

fn field_f32(data: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

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
    // 48-byte record per light, in call order Ã¢â‚¬â€ a `(x, y, radius,
    // intensity)` vec4, a `(r, g, b, penumbra)` vec4, and the cone vec4
    // `(dir_x, dir_y, cos_half, feather)`.
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
        }],
        &[],
    );
    assert_eq!(field.count, 0);
    assert_eq!(field.data.len(), OCCLUDER_FIELD_HEADER);
    assert_eq!(u32::from_le_bytes(field.data[0..4].try_into().unwrap()), 0);
    assert_eq!(&field.data[4..16], &[0u8; 12]);
}

#[test]
fn occluder_field_packs_flagged_rectangles_with_the_inverse_transform() {
    // Two flagged rectangles among unflagged, non-rectangle, and degenerate
    // draws: count 2, the header, then one 48-byte record per occluder, in
    // call order.
    let translated = Draw::Shape {
        diagnostic: false,
        glow: black(),
        world: Transform::translate([10.0, 20.0]),
        center: [3.0, -4.0],
        params: [5.0, 2.0],
        kind: 1.0,
        aa: 0.0,
        color: black(),
        lit: 0.0,
        occludes: 1.0,
        z: 0.0,
    };
    let rotated = Draw::Shape {
        diagnostic: false,
        glow: black(),
        world: Transform::rotate(std::f32::consts::FRAC_PI_2),
        center: [1.0, 2.0],
        params: [4.0, 6.0],
        kind: 1.0,
        aa: 0.0,
        color: black(),
        lit: 1.0,
        occludes: 1.0,
        z: 0.0,
    };
    let unflagged = Draw::Shape {
        diagnostic: false,
        glow: black(),
        world: Transform::identity(),
        center: [0.0, 0.0],
        params: [1.0, 1.0],
        kind: 1.0,
        aa: 0.0,
        color: black(),
        lit: 0.0,
        occludes: 0.0,
        z: 0.0,
    };
    // Flagged, but a circle: only rectangles occlude.
    let circle = Draw::Shape {
        diagnostic: false,
        glow: black(),
        world: Transform::identity(),
        center: [0.0, 0.0],
        params: [1.0, 0.0],
        kind: 0.0,
        aa: 0.0,
        color: black(),
        lit: 0.0,
        occludes: 1.0,
        z: 0.0,
    };
    // Flagged, but collapsed to a line: the inverse does not exist, so the
    // shape occludes nothing.
    let degenerate = Draw::Shape {
        diagnostic: false,
        glow: black(),
        world: Transform::scale([0.0, 1.0]),
        center: [0.0, 0.0],
        params: [1.0, 1.0],
        kind: 1.0,
        aa: 0.0,
        color: black(),
        lit: 0.0,
        occludes: 1.0,
        z: 0.0,
    };
    let field = pack_occluder_field(&[translated, unflagged, rotated, circle, degenerate], &[]);
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
    assert_eq!(field_f32(&field.data, o + 44), 0.0);
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
    assert_eq!(field_f32(&field.data, o + 44), 0.0);
}

#[test]
fn occluder_field_appends_declared_occluders_after_the_drawn_ones() {
    // One flagged rectangle plus two declared occluders: three records, the
    // drawn one first, the declared ones in declaration order.
    let drawn = Draw::Shape {
        diagnostic: false,
        glow: black(),
        world: Transform::identity(),
        center: [0.0, 0.0],
        params: [10.0, 10.0],
        kind: 1.0,
        aa: 0.0,
        color: black(),
        lit: 0.0,
        occludes: 1.0,
        z: 0.0,
    };
    let declared = [
        Occluder {
            world: Transform::translate([50.0, -20.0]),
            center: [1.0, 2.0],
            half: [10.0, 20.0],
            polygon: None,
        },
        Occluder {
            world: Transform::rotate(std::f32::consts::FRAC_PI_2),
            center: [0.0, 0.0],
            half: [4.0, 6.0],
            polygon: None,
        },
    ];
    let field = pack_occluder_field(&[drawn], &declared);
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
        },
        Occluder {
            world: Transform::translate([7.0, 0.0]),
            center: [0.0, 0.0],
            half: [3.0, 4.0],
            polygon: None,
        },
    ];
    let field = pack_occluder_field(&[], &declared);
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
    assert_eq!(
        occluder_field_buffer_size(0),
        OCCLUDER_FIELD_BUFFER_MIN as u64
    );
    assert_eq!(
        occluder_field_buffer_size(1),
        (OCCLUDER_FIELD_HEADER + OCCLUDER_RECORD) as u64
    );
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
        },
        Occluder {
            world: Transform::identity(),
            center: bounds.center,
            half: bounds.half,
            polygon: Some(rock),
        },
        Occluder {
            world: Transform::translate([0.0, 5.0]),
            center: [1.0, 1.0],
            half: [2.0, 2.0],
            polygon: None,
        },
    ];
    let field = pack_occluder_field(&[], &declared);
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
