//! Particle instance packing in pixel space, and the whole-surface scissor.

use super::super::*;
use super::*;
use crate::objects::*;
use crate::{Canvas, Particle};

#[test]
fn particles_scissor_covers_the_whole_surface() {
    let draw = Draw::Particles {
        diagnostic: false,
        data: vec![],
        count: 0,
        color: black(),
        kind: 0.0,
        aspect: 1.0,
        sprite_data: None,
        sprite_size: [0, 0],
        sprite_generation: 0,
        lit: 0.0,
        z: 0.0,
    };
    // The batch's particles can be anywhere, so the scissor is the whole
    // render area; the per-particle quads are already tight.
    assert_eq!(draw.scissor_rect([100, 100]), Some([0, 0, 100, 100]));
}

#[test]
// The test exercises the deprecated immediate particle draw itself.
#[allow(deprecated)]
fn canvas_particles_packs_instances_in_pixel_space() {
    let mut canvas = Canvas::new((100, 100));
    let tint = Color {
        r: 0.5,
        g: 0.25,
        b: 0.75,
        a: 0.5,
    };
    let particles = [
        // A live particle with half its lifetime left.
        Particle {
            pos: [10.0, 20.0],
            vel: [0.0, 0.0],
            life: 2.0,
            max_life: 4.0,
            size: 3.0,
            angle: 0.0,
            color: tint,
        },
        // A dead one: its life fraction clamps to zero.
        Particle {
            pos: [0.0, 0.0],
            vel: [0.0, 0.0],
            life: -1.0,
            max_life: 2.0,
            size: 1.0,
            angle: 0.0,
            color: tint,
        },
        // An over-living one: its life clamps to one, and its negative
        // size clamps to zero.
        Particle {
            pos: [-10.0, -10.0],
            vel: [0.0, 0.0],
            life: 6.0,
            max_life: 4.0,
            size: -2.0,
            angle: 0.0,
            color: tint,
        },
        // No max lifetime: the life fraction is zero, so it fades out
        // completely.
        Particle {
            pos: [5.0, -5.0],
            vel: [0.0, 0.0],
            life: 1.0,
            max_life: 0.0,
            size: 2.0,
            angle: 0.0,
            color: tint,
        },
    ];
    canvas.particles(&particles, tint, 2.5);
    let [
        Draw::Particles {
            data,
            count,
            color,
            z,
            ..
        },
    ] = &canvas.draws[..]
    else {
        panic!("expected one particle draw");
    };
    assert_eq!(data.len(), 4 * 32);
    assert_eq!(*count, 4);
    assert_eq!(*color, tint);
    assert_eq!(*z, 2.5);

    // Each particle packs two vec4s in pixel space (top-left origin, y
    // down): (px, py, size, life fraction), then (angle, tint r, tint g,
    // tint b).
    let f32_at = |particle: usize, field: usize| {
        let off = particle * 32 + field * 4;
        f32::from_le_bytes(data[off..off + 4].try_into().unwrap())
    };
    // user (10, 20) is (60, 30) in pixel space on a 100x100 window.
    assert_eq!(f32_at(0, 0), 60.0);
    assert_eq!(f32_at(0, 1), 30.0);
    assert_eq!(f32_at(0, 2), 3.0);
    assert_eq!(f32_at(0, 3), 0.5);
    assert_eq!(f32_at(1, 0), 50.0);
    assert_eq!(f32_at(1, 1), 50.0);
    assert_eq!(f32_at(1, 2), 1.0);
    assert_eq!(f32_at(1, 3), 0.0); // negative life clamps to zero
    // user (-10, -10) is (40, 60) in pixel space.
    assert_eq!(f32_at(2, 0), 40.0);
    assert_eq!(f32_at(2, 1), 60.0);
    assert_eq!(f32_at(2, 2), 0.0); // negative size clamps to zero
    assert_eq!(f32_at(2, 3), 1.0); // over-living clamps to one
    assert_eq!(f32_at(3, 0), 55.0);
    assert_eq!(f32_at(3, 1), 55.0);
    assert_eq!(f32_at(3, 2), 2.0);
    assert_eq!(f32_at(3, 3), 0.0); // no max lifetime means no life fraction
    // The angle is zero for every particle (circles), and the packed
    // tint is the particle's own color.
    for p in 0..4 {
        assert_eq!(f32_at(p, 4), 0.0);
        assert_eq!(f32_at(p, 5), tint.r);
        assert_eq!(f32_at(p, 6), tint.g);
        assert_eq!(f32_at(p, 7), tint.b);
    }
}

#[test]
// The test exercises the deprecated immediate particle draw itself.
#[allow(deprecated)]
fn canvas_particles_with_no_particles_adds_no_draw() {
    let mut canvas = Canvas::new((100, 100));
    canvas.particles(&[], black(), 0.0);
    assert!(canvas.draws.is_empty());
}
