//! Scissor rects for every draw kind: the padded boxes, the AA band, the off-screen and clamped cases.

use super::super::*;
use super::*;
use crate::objects::*;
use std::sync::Arc;

#[test]
fn line_scissor_includes_width_and_aa_band() {
    let draw = Draw::Line {
        diagnostic: false,
        a: [10.0, 20.0],
        b: [30.0, 40.0],
        width: 2.0,
        color: black(),
        z: 0.0,
    };
    // pad = width/2 + AA_BAND = 1.75
    assert_eq!(draw.scissor_rect([100, 100]), Some([8, 18, 24, 24]));
}

#[test]
fn polyline_scissor_covers_all_points_plus_width_and_aa_band() {
    let draw = Draw::Polyline {
        diagnostic: false,
        points: vec![[10.0, 20.0], [50.0, 40.0], [20.0, 35.0]],
        width: 2.0,
        color: black(),
        z: 0.0,
    };
    // The box over the points is (10, 20)..(50, 40); pad = width/2 +
    // AA_BAND = 1.75, so (8.25, 18.25)..(51.75, 41.75).
    assert_eq!(draw.scissor_rect([100, 100]), Some([8, 18, 44, 24]));
}

#[test]
fn polyline_with_fewer_than_two_points_has_no_scissor() {
    let one = Draw::Polyline {
        diagnostic: false,
        points: vec![[10.0, 20.0]],
        width: 2.0,
        color: black(),
        z: 0.0,
    };
    assert_eq!(one.scissor_rect([100, 100]), None);
    let none = Draw::Polyline {
        diagnostic: false,
        points: vec![],
        width: 2.0,
        color: black(),
        z: 0.0,
    };
    assert_eq!(none.scissor_rect([100, 100]), None);
}

#[test]
fn circle_scissor_includes_aa_band() {
    let draw = Draw::Circle {
        diagnostic: false,
        center: [50.0, 50.0],
        radius: 10.0,
        color: black(),
        z: 0.0,
    };
    assert_eq!(draw.scissor_rect([100, 100]), Some([39, 39, 22, 22]));
}

#[test]
fn rectangle_scissor_includes_aa_band() {
    let draw = Draw::Rectangle {
        diagnostic: false,
        center: [50.0, 50.0],
        extent: [10.0, 20.0],
        color: black(),
        z: 0.0,
    };
    assert_eq!(draw.scissor_rect([100, 100]), Some([39, 29, 22, 42]));
}

#[test]
fn off_screen_draw_has_no_scissor() {
    let draw = Draw::Circle {
        diagnostic: false,
        center: [-200.0, -200.0],
        radius: 10.0,
        color: black(),
        z: 0.0,
    };
    assert_eq!(draw.scissor_rect([100, 100]), None);
}

#[test]
fn scissor_is_clamped_to_the_render_area() {
    let draw = Draw::Line {
        diagnostic: false,
        a: [-50.0, 0.0],
        b: [50.0, 0.0],
        width: 0.0,
        color: black(),
        z: 0.0,
    };
    assert_eq!(draw.scissor_rect([100, 100]), Some([0, 0, 51, 1]));
}

#[test]
fn shape_scissor_under_non_uniform_scale() {
    let draw = Draw::Shape {
        diagnostic: false,
        glow: black(),
        world: Transform::scale([2.0, 1.0]),
        center: [25.0, 50.0],
        params: [10.0, 0.0],
        kind: 0.0,
        aa: 0.75 / 2.0,
        color: black(),
        lit: 0.0,
        occludes: 0.0,
        z: 0.0,
    };
    // The local box (25 Ã‚Â± 10.375, 50 Ã‚Â± 10.375) stretches to
    // x: [29.25, 70.75] and y: [39.625, 60.375].
    assert_eq!(draw.scissor_rect([100, 100]), Some([29, 39, 42, 22]));
}

#[test]
fn shape_scissor_under_rotation() {
    // The center is chosen so that the 45Ã‚Â° rotation (about the origin)
    // maps it onto (50, 50).
    let center = [50.0 * std::f32::consts::SQRT_2, 0.0];
    let draw = Draw::Shape {
        diagnostic: false,
        glow: black(),
        world: Transform::rotate(std::f32::consts::FRAC_PI_4),
        center,
        params: [10.0, 0.0],
        kind: 0.0,
        aa: 0.75,
        color: black(),
        lit: 0.0,
        occludes: 0.0,
        z: 0.0,
    };
    // The Ã‚Â±10.75 box rotated 45Ã‚Â° has half-extent 10.75 * sqrt(2), so the
    // axis-aligned box is [34.797, 65.203] on both axes.
    assert_eq!(draw.scissor_rect([100, 100]), Some([34, 34, 32, 32]));
}

#[test]
fn off_screen_shape_has_no_scissor() {
    let draw = Draw::Shape {
        diagnostic: false,
        glow: black(),
        world: Transform::identity(),
        center: [-200.0, -200.0],
        params: [10.0, 0.0],
        kind: 0.0,
        aa: 0.75,
        color: black(),
        lit: 0.0,
        occludes: 0.0,
        z: 0.0,
    };
    assert_eq!(draw.scissor_rect([100, 100]), None);
}

#[test]
fn sprite_scissor_is_the_texture_box_plus_aa_band() {
    let draw = Draw::Sprite {
        diagnostic: false,
        glow: black(),
        world: Transform::translate([50.0, 50.0]),
        data: Arc::new([0u8; 16]),
        size: [40.0, 20.0],
        texture_size: [40, 20],
        filter: SpriteFilter::Linear,
        aa: 0.75,
        tint: black(),
        alpha: 1.0,
        uv_rect: [0.0, 0.0, 1.0, 1.0],
        lit: 0.0,
        generation: 0,
        z: 0.0,
    };
    // The box is (50 Ã‚Â± 20.75, 50 Ã‚Â± 10.75), i.e. [29.25, 70.75] on x and
    // [39.25, 60.75] on y.
    assert_eq!(draw.scissor_rect([100, 100]), Some([29, 39, 42, 22]));
}

#[test]
fn sprite_scissor_under_rotation() {
    // A 45Ã‚Â° rotation about the sprite's own center: the local box
    // (Ã‚Â±20.75, Ã‚Â±10.75) rotates to an axis-aligned box with half-extent
    // (20.75 + 10.75) / sqrt(2) = 22.274 on both axes, centered on
    // (50, 50).
    let draw = Draw::Sprite {
        diagnostic: false,
        glow: black(),
        world: Transform::rotate(std::f32::consts::FRAC_PI_4)
            .compose(&Transform::translate([50.0, 50.0])),
        data: Arc::new([0u8; 16]),
        size: [40.0, 20.0],
        texture_size: [40, 20],
        filter: SpriteFilter::Linear,
        aa: 0.75,
        tint: black(),
        alpha: 1.0,
        uv_rect: [0.0, 0.0, 1.0, 1.0],
        lit: 0.0,
        generation: 0,
        z: 0.0,
    };
    assert_eq!(draw.scissor_rect([100, 100]), Some([27, 27, 46, 46]));
}
