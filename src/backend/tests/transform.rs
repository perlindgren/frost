//! Affine transform composition, inversion and scale extraction.

use crate::objects::*;

#[test]
fn transform_compose_applies_self_then_other() {
    let rot = Transform::rotate(std::f32::consts::FRAC_PI_2);
    let tr = Transform::translate([10.0, 0.0]);
    let world = rot.compose(&tr);
    // (1, 0) rotates to (0, 1), then translates to (10, 1).
    let p = world.apply([1.0, 0.0]);
    assert!((p[0] - 10.0).abs() < 1e-4);
    assert!((p[1] - 1.0).abs() < 1e-4);
}

#[test]
fn transform_compose_respects_order() {
    // Rotation and translation do not commute, so this pins down which
    // order the composition applies them in.
    let rot = Transform::rotate(std::f32::consts::FRAC_PI_2);
    let tr = Transform::translate([10.0, 0.0]);
    // Translate first, then rotate: (1, 0) -> (11, 0) -> (0, 11).
    let world = tr.compose(&rot);
    let p = world.apply([1.0, 0.0]);
    assert!(p[0].abs() < 1e-4);
    assert!((p[1] - 11.0).abs() < 1e-4);
    // The reverse composition: (1, 0) -> (0, 1) -> (10, 1).
    let world = rot.compose(&tr);
    let p = world.apply([1.0, 0.0]);
    assert!((p[0] - 10.0).abs() < 1e-4);
    assert!((p[1] - 1.0).abs() < 1e-4);
}

#[test]
fn transform_inverse_round_trips() {
    let world = Transform::translate([3.0, -4.0])
        .compose(&Transform::rotate(0.7))
        .compose(&Transform::scale([2.0, 0.5]));
    let Some(inv) = world.invert() else {
        panic!("expected an inverse");
    };
    for p in [[1.2, -3.4], [-7.0, 2.0], [0.0, 0.0]] {
        let back = world.apply(inv.apply(p));
        assert!((back[0] - p[0]).abs() < 1e-3);
        assert!((back[1] - p[1]).abs() < 1e-3);
    }
}

#[test]
fn degenerate_transform_has_no_inverse() {
    assert!(Transform::scale([0.0, 1.0]).invert().is_none());
    assert!(Transform::identity().invert().is_some());
}

#[test]
fn transform_scales_are_the_column_norms() {
    // Scale first, then rotate: the stretch along each axis is exactly
    // the scale factors, no matter the rotation.
    let world = Transform::scale([2.0, 3.0]).compose(&Transform::rotate(0.5));
    let [sx, sy] = world.scales();
    assert!((sx - 2.0).abs() < 1e-4);
    assert!((sy - 3.0).abs() < 1e-4);
}
