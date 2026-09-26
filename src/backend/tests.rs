use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use wgpu::PresentMode;

use super::*;
use crate::objects::*;
use crate::text;
use crate::{Canvas, Context, KeyCode, MouseButton, Particle};

fn black() -> Color {
    Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    }
}

#[test]
fn line_scissor_includes_width_and_aa_band() {
    let draw = Draw::Line {
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
        points: vec![[10.0, 20.0]],
        width: 2.0,
        color: black(),
        z: 0.0,
    };
    assert_eq!(one.scissor_rect([100, 100]), None);
    let none = Draw::Polyline {
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
        a: [-50.0, 0.0],
        b: [50.0, 0.0],
        width: 0.0,
        color: black(),
        z: 0.0,
    };
    assert_eq!(draw.scissor_rect([100, 100]), Some([0, 0, 51, 1]));
}

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

#[test]
fn draw_scene_composes_transforms_down_the_tree() {
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene::new(SceneNode {
        glow: black(),
        transform: Transform::translate([10.0, 0.0]),
        scale: [1.0, 1.0],
        modulate: WHITE,
        lit: false,
        occludes: false,
        order: 0.0,
        shape: Some(Shape::Circle {
            center: [0.0, 0.0],
            radius: 5.0,
            color: black(),
        }),
        children: vec![Box::new(SceneNode {
            glow: black(),
            transform: Transform::scale_uniform(2.0),
            scale: [1.0, 1.0],
            modulate: WHITE,
            lit: false,
            occludes: false,
            order: 0.0,
            shape: None,
            children: vec![Box::new(SceneNode {
                glow: black(),
                transform: Transform::identity(),
                scale: [1.0, 1.0],
                modulate: WHITE,
                lit: false,
                occludes: false,
                order: 0.0,
                shape: Some(Shape::Circle {
                    center: [1.0, 1.0],
                    radius: 1.0,
                    color: black(),
                }),
                children: vec![],
            })],
        })],
    });
    canvas.draw_scene(&scene);
    let [Draw::Shape {
        world: w0,
        center: c0,
        aa: aa0,
        ..
    }, Draw::Shape {
        world: w1,
        center: c1,
        aa: aa1,
        ..
    }] = &canvas.draws[..]
    else {
        panic!("expected two shape draws");
    };
    // The root circle translates to (10, 0) in user space, which is
    // (60, 50) in pixel space on a 100x100 window (x + w/2, h/2 - y).
    assert_eq!(w0.apply(*c0), [60.0, 50.0]);
    assert!((*aa0 - AA_BAND).abs() < 1e-6);
    // The grandchild's own scale applies first, then the root's
    // translate: (1, 1) -> (2, 2) -> (12, 2) in user space, (62, 48) in
    // pixel space, and the local AA band halves to stay 0.75 screen
    // pixels.
    assert_eq!(w1.apply(*c1), [62.0, 48.0]);
    assert!((*aa1 - AA_BAND / 2.0).abs() < 1e-6);
}

#[test]
fn draw_scene_rotates_a_translated_child() {
    // A translated child of a rotated parent (an orbiting shape): the
    // parent's rotation must rotate the child's translation, not the
    // other way around.
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene::new(SceneNode {
        glow: black(),
        transform: Transform::rotate(std::f32::consts::FRAC_PI_2),
        scale: [1.0, 1.0],
        modulate: WHITE,
        lit: false,
        occludes: false,
        order: 0.0,
        shape: None,
        children: vec![Box::new(SceneNode {
            glow: black(),
            transform: Transform::translate([10.0, 0.0]),
            scale: [1.0, 1.0],
            modulate: WHITE,
            lit: false,
            occludes: false,
            order: 0.0,
            shape: Some(Shape::Circle {
                center: [0.0, 0.0],
                radius: 1.0,
                color: black(),
            }),
            children: vec![],
        })],
    });
    canvas.draw_scene(&scene);
    let [Draw::Shape { world: w, center: c, .. }] = &canvas.draws[..] else {
        panic!("expected one shape draw");
    };
    // (0, 0) translates to (10, 0) and rotates to (0, 10) in user space,
    // which is (50, 40) in pixel space on a 100x100 window.
    assert_eq!(w.apply(*c), [50.0, 40.0]);
}

#[test]
fn scene_shape_at_user_origin_lands_at_window_center() {
    // Regression: a scene shape at the user-space origin must land at
    // the window center, not the top-left pixel corner.
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene::new(SceneNode {
        glow: black(),
        transform: Transform::identity(),
        scale: [1.0, 1.0],
        modulate: WHITE,
        lit: false,
        occludes: false,
        order: 0.0,
        shape: Some(Shape::Circle {
            center: [0.0, 0.0],
            radius: 10.0,
            color: black(),
        }),
        children: vec![],
    });
    canvas.draw_scene(&scene);
    let [Draw::Shape { world: w, center: c, .. }] = &canvas.draws[..] else {
        panic!("expected one shape draw");
    };
    assert_eq!(w.apply(*c), [50.0, 50.0]);
}

#[test]
fn context_reports_held_keys() {
    let mut canvas = Canvas::new((100, 100));
    let mut scene = Scene::default();
    let mut keys = HashSet::new();
    let mouse_buttons = HashSet::new();
    {
        let ctx = Context {
            canvas: &mut canvas,
            scene: &mut scene,
            keys: &keys,
            expected_fps: None,
            mouse: None,
            mouse_buttons: &mouse_buttons,
            gilrs: None,
            frame_processing_ms: 0.0,
            frame_draw_calls: 0,
        };
        assert!(!ctx.key_down(KeyCode::KeyW));
    }
    keys.insert(KeyCode::KeyW);
    let ctx = Context {
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert!(ctx.key_down(KeyCode::KeyW));
    assert!(!ctx.key_down(KeyCode::KeyA));
}

#[test]
fn context_reports_mouse_position() {
    let mut canvas = Canvas::new((100, 100));
    let mut scene = Scene::default();
    let keys = HashSet::new();
    let mouse_buttons = HashSet::new();
    let ctx = Context {
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert_eq!(ctx.mouse_position(), None);
    let ctx = Context {
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        expected_fps: None,
        mouse: Some([12.0, -34.0]),
        mouse_buttons: &mouse_buttons,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert_eq!(ctx.mouse_position(), Some([12.0, -34.0]));
}

#[test]
fn context_reports_held_mouse_button() {
    let mut canvas = Canvas::new((100, 100));
    let mut scene = Scene::default();
    let keys = HashSet::new();
    let mut mouse_buttons = HashSet::new();
    {
        let ctx = Context {
            canvas: &mut canvas,
            scene: &mut scene,
            keys: &keys,
            expected_fps: None,
            mouse: None,
            mouse_buttons: &mouse_buttons,
            gilrs: None,
            frame_processing_ms: 0.0,
            frame_draw_calls: 0,
        };
        assert!(!ctx.mouse_button_down(MouseButton::Left));
    }
    mouse_buttons.insert(MouseButton::Left);
    let ctx = Context {
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert!(ctx.mouse_button_down(MouseButton::Left));
    assert!(!ctx.mouse_button_down(MouseButton::Right));
}

#[test]
fn context_reports_no_gamepads_without_gilrs() {
    let mut canvas = Canvas::new((100, 100));
    let mut scene = Scene::default();
    let keys = HashSet::new();
    let mouse_buttons = HashSet::new();
    let ctx = Context {
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    // Without a gamepad controller (gilrs could not open the platform's
    // input devices, or none is connected) the list is empty, not an
    // error.
    assert_eq!(ctx.gamepads().count(), 0);
}

#[test]
fn shape_scissor_under_non_uniform_scale() {
    let draw = Draw::Shape {
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
    // The local box (25 ± 10.375, 50 ± 10.375) stretches to
    // x: [29.25, 70.75] and y: [39.625, 60.375].
    assert_eq!(draw.scissor_rect([100, 100]), Some([29, 39, 42, 22]));
}

#[test]
fn shape_scissor_under_rotation() {
    // The center is chosen so that the 45° rotation (about the origin)
    // maps it onto (50, 50).
    let center = [50.0 * std::f32::consts::SQRT_2, 0.0];
    let draw = Draw::Shape {
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
    // The ±10.75 box rotated 45° has half-extent 10.75 * sqrt(2), so the
    // axis-aligned box is [34.797, 65.203] on both axes.
    assert_eq!(draw.scissor_rect([100, 100]), Some([34, 34, 32, 32]));
}

#[test]
fn off_screen_shape_has_no_scissor() {
    let draw = Draw::Shape {
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
fn particles_scissor_covers_the_whole_surface() {
    let draw = Draw::Particles {
        data: vec![],
        count: 0,
        color: black(),
        kind: 0.0,
        aspect: 1.0,
        sprite_data: None,
        sprite_size: [0, 0],
        lit: 0.0,
        z: 0.0,
    };
    // The batch's particles can be anywhere, so the scissor is the whole
    // render area; the per-particle quads are already tight.
    assert_eq!(draw.scissor_rect([100, 100]), Some([0, 0, 100, 100]));
}

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

    let f32_at = |off: usize| {
        f32::from_le_bytes(data[off..off + 4].try_into().unwrap())
    };
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
    let [Draw::Particles {
        data,
        count,
        color,
        z,
        ..
    }] = &canvas.draws[..]
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
    // (spanning 48..64); `misc` vec2 @ 64; `glow` vec4 @ 80; struct size 96.
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
    // flag takes the slot at 72; the 16-byte-aligned glow vec4 follows at
    // 80 (spanning 80..96, so the struct is 96 bytes).
    assert_eq!(f32_at(60), 0.4);
    assert_eq!(f32_at(64), 0.25);
    assert_eq!(f32_at(68), 1.0);
    assert_eq!(f32_at(72), 1.0);
    assert_eq!(f32_at(80), 0.5);
    assert_eq!(f32_at(84), 0.6);
    assert_eq!(f32_at(88), 0.7);
    assert_eq!(f32_at(92), 0.8);
    assert_eq!(data.len(), 96);
}

#[test]
fn clear_color_falls_back_to_the_default_without_a_background() {
    let draws = [Draw::Circle {
        center: [0.0, 0.0],
        radius: 5.0,
        color: black(),
        z: 0.0,
    }];
    assert_eq!(clear_color(&draws), DEFAULT_BACKGROUND);
}

#[test]
fn clear_color_is_the_background_when_the_frame_has_one() {
    let indigo = Color {
        r: 0.09,
        g: 0.06,
        b: 0.16,
        a: 1.0,
    };
    let draws = [
        Draw::Background { color: indigo },
        Draw::Circle {
            center: [0.0, 0.0],
            radius: 5.0,
            color: black(),
            z: 0.0,
        },
    ];
    assert_eq!(clear_color(&draws), indigo);
}

#[test]
fn clear_color_is_the_last_background_in_call_order() {
    let first = Color {
        r: 1.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    let second = Color {
        r: 0.0,
        g: 0.0,
        b: 1.0,
        a: 1.0,
    };
    let draws = [
        Draw::Background { color: first },
        Draw::Background { color: second },
    ];
    assert_eq!(clear_color(&draws), second);
}

/// Reads the little-endian f32 at byte `offset` in the packed light field.
fn field_f32(data: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

#[test]
fn light_field_packs_an_empty_header_with_the_ambient() {
    // No lights: just the 32-byte header — count 0, 12 padding bytes, the
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
    // 48-byte record per light, in call order — a `(x, y, radius,
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
    // Record 0 @ 32: (60, 30, 16, 2), then (1, 0.5, 0, 8) — the color
    // vec4's w now carries the penumbra radius — then the cone
    // (0.6, 0.8, 0, 0.25) — the feather rides the cone vec4's w.
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
    // Record 1 @ 80: (-12.5, 8.25, 0, 0.5), then (0, 1, 1, 0) — the
    // penumbra is zero here — then the omni cone (0, -1, -1, 0).
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
    // Record 2 @ 128: the directional light — the sentinel radius (-1)
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
    // No occluders: just the 16-byte header — count 0, 12 padding bytes —
    // and no records.
    let field = pack_occluder_field(&[Draw::Light {
        pos: [10.0, 20.0],
        radius: 5.0,
        intensity: 1.0,
        color: black(),
        penumbra: 0.0,
        dir: [1.0, 0.0],
        cos_half: -1.0,
        feather: 0.0,
        z: 0.0,
    }]);
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
    let field =
        pack_occluder_field(&[translated, unflagged, rotated, circle, degenerate]);
    assert_eq!(field.count, 2);
    assert_eq!(field.data.len(), OCCLUDER_FIELD_HEADER + 2 * OCCLUDER_RECORD);
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
    // Record 1 @ 64: the 90° rotation's inverse is the -90° rotation,
    // column-major (m00, m10, m01, m11) = (0, -1, 1, 0); the translation is
    // zero. The diagonals are cos(90°) in f32, so they are ~1e-8 rather
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
fn field_buffer_sizes_floor_at_the_wgsl_minimum_binding_size() {
    // A lightless or occluder-less frame still binds both field buffers,
    // and the WGSL layout's minimum binding size is the header plus one
    // vec4 — the buffer must never be sized below it, or wgpu rejects the
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
fn background_node_sorts_to_the_back_and_keeps_its_color() {
    let mut canvas = Canvas::new((100, 100));
    canvas.draw_scene(&Scene::new(SceneNode {
        glow: black(),
        transform: Transform::identity(),
        scale: [1.0, 1.0],
        modulate: WHITE,
        lit: false,
        occludes: false,
        order: 0.0,
        shape: Some(Shape::Rectangle {
            center: [0.0, 0.0],
            extent: [10.0, 10.0],
            color: black(),
        }),
        children: vec![Box::new(SceneNode {
            glow: black(),
            // Transformed on purpose: the background must ignore it.
            transform: Transform::translate([50.0, 50.0]),
            scale: [1.0, 1.0],
            modulate: WHITE,
            lit: false,
            occludes: false,
            order: 0.0,
            shape: Some(Shape::Background {
                color: Color {
                    r: 0.0,
                    g: 0.0,
                    b: 1.0,
                    a: 1.0,
                },
            }),
            children: vec![],
        })],
    }));
    canvas
        .draws
        .sort_by(|a, b| a.z().partial_cmp(&b.z()).unwrap_or(std::cmp::Ordering::Equal));
    let [Draw::Background { color }, Draw::Shape { .. }] = &canvas.draws[..] else {
        panic!("expected one background and one shape, got {:?}", canvas.draws);
    };
    assert_eq!(
        *color,
        Color {
            r: 0.0,
            g: 0.0,
            b: 1.0,
            a: 1.0
        }
    );
}

#[test]
fn layers_are_hard_draw_partitions() {
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene {
        root: SceneNode::default(),
        layers: vec![
            Layer {
                order: -1.0,
                speed: 1.0,
                repeat: [0.0, 0.0],
                root: SceneNode {
                    glow: black(),
                    // High local z, but the layer's low order still puts
                    // it behind every group with a higher order.
                    transform: Transform::identity(),
                    scale: [1.0, 1.0],
                    modulate: WHITE,
                    lit: false,
                    occludes: false,
                    order: 100.0,
                    shape: Some(Shape::Circle {
                        center: [0.0, 0.0],
                        radius: 1.0,
                        color: Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
                    }),
                    children: vec![],
                },
            },
            Layer {
                order: 1.0,
                speed: 1.0,
                repeat: [0.0, 0.0],
                root: SceneNode {
                    glow: black(),
                    // Low local z, but the layer's high order still puts
                    // it on top.
                    transform: Transform::identity(),
                    scale: [1.0, 1.0],
                    modulate: WHITE,
                    lit: false,
                    occludes: false,
                    order: -100.0,
                    shape: Some(Shape::Circle {
                        center: [0.0, 0.0],
                        radius: 1.0,
                        color: Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 },
                    }),
                    children: vec![],
                },
            },
        ],
        ambient: AMBIENT,
        camera: None,
    };
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 2);
    // The red (low layer) paints before the blue (high layer), no matter
    // how far apart their local z values are.
    assert!(matches!(
        &painted[0],
        Draw::Shape { color, .. } if color.r == 1.0 && color.b == 0.0
    ));
    assert!(matches!(
        &painted[1],
        Draw::Shape { color, .. } if color.b == 1.0 && color.r == 0.0
    ));
}

#[test]
fn within_a_layer_the_local_z_ordering_applies_and_z_does_not_leak_across_layers() {
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene {
        root: SceneNode::default(),
        layers: vec![
            Layer {
                order: 1.0,
                speed: 1.0,
                repeat: [0.0, 0.0],
                root: SceneNode {
                    glow: black(),
                    transform: Transform::identity(),
                    scale: [1.0, 1.0],
                    modulate: WHITE,
                    lit: false,
                    occludes: false,
                    order: 0.0,
                    shape: None,
                    // Local ascending z: red (100) before green (200).
                    children: vec![
                        Box::new(SceneNode {
                            glow: black(),
                            transform: Transform::identity(),
                            scale: [1.0, 1.0],
                            modulate: WHITE,
                            lit: false,
                            occludes: false,
                            order: 100.0,
                            shape: Some(Shape::Circle {
                                center: [0.0, 0.0],
                                radius: 1.0,
                                color: Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
                            }),
                            children: vec![],
                        }),
                        Box::new(SceneNode {
                            glow: black(),
                            transform: Transform::identity(),
                            scale: [1.0, 1.0],
                            modulate: WHITE,
                            lit: false,
                            occludes: false,
                            order: 200.0,
                            shape: Some(Shape::Circle {
                                center: [0.0, 0.0],
                                radius: 1.0,
                                color: Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 },
                            }),
                            children: vec![],
                        }),
                    ],
                },
            },
            Layer {
                order: 2.0,
                speed: 1.0,
                repeat: [0.0, 0.0],
                root: SceneNode {
                    glow: black(),
                    transform: Transform::identity(),
                    scale: [1.0, 1.0],
                    modulate: WHITE,
                    lit: false,
                    occludes: false,
                    order: 0.0,
                    shape: Some(Shape::Circle {
                        center: [0.0, 0.0],
                        radius: 1.0,
                        color: Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 },
                    }),
                    children: vec![],
                },
            },
        ],
        ambient: AMBIENT,
        camera: None,
    };
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 3);
    // The high layer's blue paints last even though its local z (0) is
    // far below the red's (100) and the green's (200): the layer's
    // order wins, and z never leaks across layers.
    assert!(matches!(
        &painted[0],
        Draw::Shape { color, .. } if color.r == 1.0 && color.g == 0.0
    ));
    assert!(matches!(
        &painted[1],
        Draw::Shape { color, .. } if color.g == 1.0 && color.r == 0.0
    ));
    assert!(matches!(
        &painted[2],
        Draw::Shape { color, .. } if color.b == 1.0 && color.r == 0.0
    ));
}

#[test]
fn the_base_group_paints_first_at_equal_order() {
    let mut canvas = Canvas::new((100, 100));
    canvas.circle(0.0, 0.0, 1.0, Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }, 0.0);
    let scene = Scene {
        root: SceneNode::default(),
        layers: vec![Layer {
            order: 0.0,
            speed: 1.0,
            repeat: [0.0, 0.0],
            root: SceneNode {
                glow: black(),
                // Negative local z, but the base group is declared
                // before the layer, so at the equal order 0.0 it still
                // paints first.
                transform: Transform::identity(),
                scale: [1.0, 1.0],
                modulate: WHITE,
                lit: false,
                occludes: false,
                order: -1.0,
                shape: Some(Shape::Circle {
                    center: [0.0, 0.0],
                    radius: 1.0,
                    color: Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 },
                }),
                children: vec![],
            },
        }],
    ambient: AMBIENT,
    camera: None,
    };
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 2);
    // The base group's circle (an immediate draw) paints before the
    // layer's shape, even though the layer draw's local z is lower.
    assert!(matches!(
        &painted[0],
        Draw::Circle { color, .. } if color.r == 1.0 && color.b == 0.0
    ));
    assert!(matches!(
        &painted[1],
        Draw::Shape { color, .. } if color.b == 1.0 && color.r == 0.0
    ));
}

#[test]
fn a_background_in_a_higher_layer_sets_the_clear_color() {
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene {
        root: SceneNode {
            glow: black(),
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: WHITE,
            lit: false,
            occludes: false,
            order: 0.0,
            shape: Some(Shape::Background {
                color: Color { r: 0.1, g: 0.0, b: 0.0, a: 1.0 },
            }),
            children: vec![],
        },
        layers: vec![Layer {
            order: 1.0,
            speed: 1.0,
            repeat: [0.0, 0.0],
            root: SceneNode {
                glow: black(),
                transform: Transform::identity(),
                scale: [1.0, 1.0],
                modulate: WHITE,
                lit: false,
                occludes: false,
                order: 0.0,
                shape: Some(Shape::Background {
                    color: Color { r: 0.0, g: 0.1, b: 0.0, a: 1.0 },
                }),
                children: vec![],
            },
        }],
    ambient: AMBIENT,
    camera: None,
    };
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    // The higher layer's background is last in paint order, so it is
    // the frame's clear color; the base group's background is not.
    assert_eq!(
        clear_color(&painted),
        Color { r: 0.0, g: 0.1, b: 0.0, a: 1.0 }
    );
}

#[test]
fn without_layers_the_paint_order_is_the_global_z_sort() {
    let mut canvas = Canvas::new((100, 100));
    canvas.circle(0.0, 0.0, 1.0, Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }, 1.0);
    canvas.circle(0.0, 0.0, 1.0, Color { r: 0.0, g: 1.0, b: 0.0, a: 1.0 }, -1.0);
    let scene = Scene::new(SceneNode::default());
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 2);
    // Same result as the pre-layer global z sort: green (-1) behind
    // red (1).
    assert!(matches!(
        &painted[0],
        Draw::Circle { color, .. } if color.g == 1.0 && color.r == 0.0
    ));
    assert!(matches!(
        &painted[1],
        Draw::Circle { color, .. } if color.r == 1.0 && color.g == 0.0
    ));
}

#[test]
fn a_camera_anchors_the_view_to_its_node() {
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene {
        root: SceneNode::default(),
        layers: vec![Layer {
            order: 0.0,
            speed: 1.0,
            repeat: [0.0, 0.0],
            root: SceneNode {
                glow: black(),
                transform: Transform::identity(),
                scale: [1.0, 1.0],
                modulate: WHITE,
                lit: false,
                occludes: false,
                order: 0.0,
                shape: None,
                children: vec![
                    Box::new(SceneNode {
                        glow: black(),
                        // A circle at the camera's position.
                        transform: Transform::translate([10.0, -4.0]),
                        scale: [1.0, 1.0],
                        modulate: WHITE,
                        lit: false,
                        occludes: false,
                        order: 0.0,
                        shape: Some(Shape::Circle {
                            center: [0.0, 0.0],
                            radius: 1.0,
                            color: Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
                        }),
                        children: vec![],
                    }),
                    Box::new(SceneNode {
                        glow: black(),
                        // The camera: a shapeless pivot at the circle's
                        // position.
                        transform: Transform::translate([10.0, -4.0]),
                        scale: [1.0, 1.0],
                        modulate: WHITE,
                        lit: false,
                        occludes: false,
                        order: 0.0,
                        shape: None,
                        children: vec![],
                    }),
                ],
            },
        }],
        ambient: AMBIENT,
        camera: Some(NodePath {
            group: Some(0),
            children: vec![1],
        }),
    };
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 1);
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected one shape draw");
    };
    // The camera's offset is subtracted from the circle, so the circle
    // lands on the window origin: user (0, 0) -> pixel (50, 50).
    assert_eq!(world.apply([0.0, 0.0]), [50.0, 50.0]);
}

#[test]
fn layer_speed_scales_the_camera_motion() {
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene {
        root: SceneNode {
            glow: black(),
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: WHITE,
            lit: false,
            occludes: false,
            order: 0.0,
            shape: None,
            // The camera is a shapeless pivot under the scene's root.
            children: vec![Box::new(SceneNode {
                glow: black(),
                transform: Transform::translate([10.0, 0.0]),
                scale: [1.0, 1.0],
                modulate: WHITE,
                lit: false,
                occludes: false,
                order: 0.0,
                shape: None,
                children: vec![],
            })],
        },
        layers: vec![
            Layer {
                order: 1.0,
                // Double the camera's speed: the layer's point at the
                // camera's position shifts twice the camera's offset.
                speed: 2.0,
                repeat: [0.0, 0.0],
                root: SceneNode {
                    glow: black(),
                    transform: Transform::translate([10.0, 0.0]),
                    scale: [1.0, 1.0],
                    modulate: WHITE,
                    lit: false,
                    occludes: false,
                    order: 0.0,
                    shape: Some(Shape::Circle {
                        center: [0.0, 0.0],
                        radius: 1.0,
                        color: Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
                    }),
                    children: vec![],
                },
            },
            Layer {
                order: 2.0,
                // Half the camera's speed: the same point shifts half
                // the camera's offset.
                speed: 0.5,
                repeat: [0.0, 0.0],
                root: SceneNode {
                    glow: black(),
                    transform: Transform::translate([10.0, 0.0]),
                    scale: [1.0, 1.0],
                    modulate: WHITE,
                    lit: false,
                    occludes: false,
                    order: 0.0,
                    shape: Some(Shape::Circle {
                        center: [0.0, 0.0],
                        radius: 1.0,
                        color: Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 },
                    }),
                    children: vec![],
                },
            },
        ],
        ambient: AMBIENT,
        camera: Some(NodePath {
            group: None,
            children: vec![0],
        }),
    };
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 2);
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected one shape draw");
    };
    // Red, at double speed: user (10 - 2*10, 0) = (-10, 0) ->
    // pixel (40, 50).
    assert_eq!(world.apply([0.0, 0.0]), [40.0, 50.0]);
    let Draw::Shape { world, .. } = &painted[1] else {
        panic!("expected one shape draw");
    };
    // Blue, at half speed: user (10 - 0.5*10, 0) = (5, 0) ->
    // pixel (55, 50).
    assert_eq!(world.apply([0.0, 0.0]), [55.0, 50.0]);
}

/// A layer, a circle straddling the window's bottom-left corner: the
/// scene for the repeat tests, built at the given repeat offsets.
/// Straddling an edge makes the layer's tiling visible: the base copy
/// shows at the bottom-left edge and its period copies at the
/// opposite edges.
fn repeat_layer_scene(repeat: [f32; 2]) -> Scene {
    Scene {
        root: SceneNode::default(),
        layers: vec![Layer {
            order: 0.0,
            speed: 1.0,
            repeat,
            root: SceneNode {
                glow: black(),
                transform: Transform::identity(),
                scale: [1.0, 1.0],
                modulate: WHITE,
                lit: false,
                occludes: false,
                order: 0.0,
                shape: Some(Shape::Circle {
                    center: [-50.0, -50.0],
                    radius: 1.0,
                    color: Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
                }),
                children: vec![],
            },
        }],
        ambient: AMBIENT,
        camera: None,
    }
}

#[test]
fn a_zero_repeat_layer_draws_its_content_once() {
    let mut canvas = Canvas::new((100, 100));
    let scene = repeat_layer_scene([0.0, 0.0]);
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 1);
    // The single copy is undispaced: user (-50, -50) -> pixel
    // (0, 100).
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected one shape draw");
    };
    assert_eq!(world.apply([-50.0, -50.0]), [0.0, 100.0]);
}

#[test]
fn a_repeating_layer_draws_only_the_copies_reaching_the_window() {
    let mut canvas = Canvas::new((100, 100));
    // The window spans user x in [-50, 50]: with a period of 100, the
    // circle straddling the bottom edge is visible in its base copy at
    // the left edge and in its copy displaced by one period at the
    // right edge — two copies, and no copy further out, because the
    // one displaced by -100 lies fully off the window.
    let scene = repeat_layer_scene([100.0, 0.0]);
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 2);
    // The base content (offset 0.0) is drawn first: user (-50, -50)
    // -> pixel (0, 100).
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected one shape draw");
    };
    assert_eq!(world.apply([-50.0, -50.0]), [0.0, 100.0]);
    // ...then its copy displaced by one period, at the right edge:
    // user (50, -50) -> pixel (100, 100).
    let Draw::Shape { world, .. } = &painted[1] else {
        panic!("expected one shape draw");
    };
    assert_eq!(world.apply([-50.0, -50.0]), [100.0, 100.0]);
}

#[test]
fn repeat_offsets_follow_the_camera() {
    let mut canvas = Canvas::new((100, 100));
    // A camera at user x = 60 shifts the window to layer x in
    // [10, 110]: the circle's base copy has scrolled fully off the
    // window's left edge, and only its copy displaced by one period
    // reaches it — at the window's bottom edge.
    let scene = Scene {
        root: SceneNode {
            glow: black(),
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: WHITE,
            lit: false,
            occludes: false,
            order: 0.0,
            shape: None,
            children: vec![Box::new(SceneNode {
                transform: Transform::translate([60.0, 0.0]),
                ..Default::default()
            })],
        },
        ambient: AMBIENT,
        camera: Some(NodePath {
            group: None,
            children: vec![0],
        }),
        ..repeat_layer_scene([100.0, 0.0])
    };
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 1);
    // The copy has shifted with the camera, to the window's bottom
    // edge: user (50 - 60, -50) = (-10, -50) -> pixel (40, 100).
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected one shape draw");
    };
    assert_eq!(world.apply([-50.0, -50.0]), [40.0, 100.0]);
}

#[test]
fn a_copy_from_a_tile_the_window_does_not_overlap_reaches_the_window() {
    // The content need not sit inside the tile [0, period): a circle
    // at layer x = -60 lies in the tile [-100, 0), and the window's
    // box [-50, 50] overlaps only the tiles [-100, 0) and [0, 100).
    // The circle's copy displaced by one period is at 40, inside the
    // window, though its tile [100, 200) does not overlap it — it must
    // be drawn, or it would pop into the middle of the window the
    // moment a moving camera's box crossed the tile boundary at 100.
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene {
        root: SceneNode::default(),
        layers: vec![Layer {
            order: 0.0,
            speed: 1.0,
            repeat: [100.0, 0.0],
            root: SceneNode {
                shape: Some(Shape::Circle {
                    center: [-60.0, 0.0],
                    radius: 1.0,
                    color: Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
                }),
                ..Default::default()
            },
        }],
        ambient: AMBIENT,
        camera: None,
    };
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    // Exactly one copy's displaced content reaches the window's box:
    // the period copy at 40, in the window's mid-right: pixel (90, 50).
    // The base copy at -60 lies fully off the window's left edge.
    assert_eq!(painted.len(), 1);
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected a shape draw");
    };
    assert_eq!(world.apply([-60.0, 0.0]), [90.0, 50.0]);
}

#[test]
fn an_object_crossing_a_tile_boundary_is_split_without_aborting() {
    let mut canvas = Canvas::new((100, 100));
    // A rectangle twenty wide, centered at layer x = 95: it straddles
    // the tile boundary at 100. With a period of 100 (greater than its
    // 20 width), it is legal: the window must see its two copies, one
    // at each edge.
    let scene = Scene {
        root: SceneNode {
            glow: black(),
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: WHITE,
            lit: false,
            occludes: false,
            order: 0.0,
            shape: None,
            // A camera at 145 puts the window at layer x in [95, 195],
            // straddling the boundary.
            children: vec![Box::new(SceneNode {
                transform: Transform::translate([145.0, 0.0]),
                ..Default::default()
            })],
        },
        ambient: AMBIENT,
        camera: Some(NodePath {
            group: None,
            children: vec![0],
        }),
        layers: vec![Layer {
            order: 0.0,
            speed: 1.0,
            repeat: [100.0, 0.0],
            root: SceneNode {
                glow: black(),
                transform: Transform::identity(),
                scale: [1.0, 1.0],
                modulate: WHITE,
                lit: false,
                occludes: false,
                order: 0.0,
                shape: Some(Shape::Rectangle {
                    center: [95.0, 0.0],
                    extent: [10.0, 5.0],
                    color: Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
                }),
                children: vec![],
            },
        }],
    };
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    // No abort, and both copies are recorded: the object's part past
    // the boundary appears on the opposite side of the window.
    assert_eq!(painted.len(), 2);
    // The base copy is clipped at the window's left edge: user
    // (95 - 145, 0) = (-50, 0) -> pixel (0, 50).
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected one shape draw");
    };
    assert_eq!(world.apply([95.0, 0.0]), [0.0, 50.0]);
    // Its wrapped copy is clipped at the right edge: user
    // (95 + 100 - 145, 0) = (50, 0) -> pixel (100, 50).
    let Draw::Shape { world, .. } = &painted[1] else {
        panic!("expected one shape draw");
    };
    assert_eq!(world.apply([95.0, 0.0]), [100.0, 50.0]);
}

#[test]
#[should_panic(expected = "minimum repeat offset")]
fn repeat_smaller_than_the_window_aborts() {
    let mut canvas = Canvas::new((100, 100));
    let scene = repeat_layer_scene([50.0, 0.0]);
    canvas.draw_scene(&scene);
}

#[test]
#[should_panic(expected = "drawn twice")]
fn an_object_wider_than_its_repeat_aborts() {
    let mut canvas = Canvas::new((100, 100));
    // A rectangle one hundred and twenty wide exceeds its period of
    // 100: its copies would overlap, so the same object would be drawn
    // twice.
    let scene = Scene {
        layers: vec![Layer {
            order: 0.0,
            speed: 1.0,
            repeat: [100.0, 0.0],
            root: SceneNode {
                glow: black(),
                transform: Transform::identity(),
                scale: [1.0, 1.0],
                modulate: WHITE,
                lit: false,
                occludes: false,
                order: 0.0,
                shape: Some(Shape::Rectangle {
                    center: [0.0, 0.0],
                    extent: [60.0, 5.0],
                    color: Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
                }),
                children: vec![],
            },
        }],
        ..repeat_layer_scene([0.0, 0.0])
    };
    canvas.draw_scene(&scene);
}

#[test]
fn repeat_is_per_axis() {
    // Both axes repeat: the circle straddling the bottom-left corner
    // is visible at all four corners of the window — four copies.
    let mut canvas = Canvas::new((100, 100));
    let scene = repeat_layer_scene([100.0, 100.0]);
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 4);
    // One axis repeats: two copies, not four.
    let mut canvas = Canvas::new((100, 100));
    let scene = repeat_layer_scene([100.0, 0.0]);
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 2);
}

#[test]
fn a_camera_followed_object_in_a_repeating_layer_stays_at_the_window_center() {
    // The camera-followed-object case (the parallax player): an object
    // that moves through the layer's space while the camera follows it.
    // The object's base copy is drawn whenever the object is inside
    // the window's box — which, under the following camera, is always —
    // so the object stays at the window's center no matter how far it
    // wanders. Its wrapped copies, one full period away, sit off the
    // window's edges, because the period is at least the window size.
    let scene = |repeat: [f32; 2], pos: f32| Scene {
        root: SceneNode::default(),
        layers: vec![Layer {
            order: 0.0,
            speed: 1.0,
            repeat,
            root: SceneNode {
                shape: None,
                children: vec![
                    // The moving object, at layer x = pos.
                    Box::new(SceneNode {
                        transform: Transform::translate([pos, 0.0]),
                        shape: Some(Shape::Circle {
                            center: [0.0, 0.0],
                            radius: 1.0,
                            color: Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
                        }),
                        ..Default::default()
                    }),
                    // The shapeless camera pivot, following the object.
                    Box::new(SceneNode {
                        transform: Transform::translate([pos, 0.0]),
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            },
        }],
        ambient: AMBIENT,
        camera: Some(NodePath {
            group: Some(0),
            children: vec![1],
        }),
    };
    // pos = 60: the window spans layer x in [10, 110]. The object's
    // base copy is at the window center: user (60 - 60, 0) = (0, 0)
    // -> pixel (50, 50).
    let mut canvas = Canvas::new((100, 100));
    canvas.draw_scene(&scene([100.0, 0.0], 60.0));
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 1);
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected a shape draw");
    };
    assert_eq!(world.apply([0.0, 0.0]), [50.0, 50.0]);
    // pos = 150: the object has wandered a window and a half from the
    // origin; it is still at the window center.
    let mut canvas = Canvas::new((100, 100));
    canvas.draw_scene(&scene([100.0, 0.0], 150.0));
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 1);
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected a shape draw");
    };
    assert_eq!(world.apply([0.0, 0.0]), [50.0, 50.0]);
    // A non-repeating layer has no such limit: the same object is
    // drawn at the window center, wherever it is.
    let mut canvas = Canvas::new((100, 100));
    canvas.draw_scene(&scene([0.0, 0.0], 150.0));
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 1);
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected a shape draw");
    };
    assert_eq!(world.apply([0.0, 0.0]), [50.0, 50.0]);
}

#[test]
fn the_camera_rotation_turns_the_view() {
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene {
        root: SceneNode {
            glow: black(),
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: WHITE,
            lit: false,
            occludes: false,
            order: 0.0,
            shape: None,
            children: vec![
                Box::new(SceneNode {
                    glow: black(),
                    // A circle ten units to the camera's right.
                    transform: Transform::translate([10.0, 0.0]),
                    scale: [1.0, 1.0],
                    modulate: WHITE,
                    lit: false,
                    occludes: false,
                    order: 0.0,
                    shape: Some(Shape::Circle {
                        center: [0.0, 0.0],
                        radius: 1.0,
                        color: Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
                    }),
                    children: vec![],
                }),
                Box::new(SceneNode {
                    glow: black(),
                    // The camera, rotated a quarter turn: it turns the
                    // view rather than moving it.
                    transform: Transform::rotate(std::f32::consts::FRAC_PI_2),
                    scale: [1.0, 1.0],
                    modulate: WHITE,
                    lit: false,
                    occludes: false,
                    order: 0.0,
                    shape: None,
                    children: vec![],
                }),
            ],
        },
        layers: vec![],
        ambient: AMBIENT,
        camera: Some(NodePath {
            group: None,
            children: vec![1],
        }),
    };
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 1);
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected one shape draw");
    };
    // The circle, which sat to the camera's right, now appears below
    // the camera, at user (0, -10) -> pixel (50, 60). The rotation's
    // cosine is not exactly zero in f32, so compare with a tolerance.
    let p = world.apply([0.0, 0.0]);
    assert!((p[0] - 50.0).abs() < 1e-3 && (p[1] - 60.0).abs() < 1e-3);
}

#[test]
fn a_missing_camera_path_renders_without_a_camera() {
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene {
        root: SceneNode {
            glow: black(),
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: WHITE,
            lit: false,
            occludes: false,
            order: 0.0,
            shape: None,
            children: vec![Box::new(SceneNode {
                glow: black(),
                transform: Transform::translate([10.0, 0.0]),
                scale: [1.0, 1.0],
                modulate: WHITE,
                lit: false,
                occludes: false,
                order: 0.0,
                shape: Some(Shape::Circle {
                    center: [0.0, 0.0],
                    radius: 1.0,
                    color: Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
                }),
                children: vec![],
            })],
        },
        layers: vec![],
        // No such layer: the camera path names no node.
        ambient: AMBIENT,
        camera: Some(NodePath {
            group: Some(5),
            children: vec![],
        }),
    };
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 1);
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected one shape draw");
    };
    // The scene falls back to the fixed window-centered user space:
    // user (10, 0) -> pixel (60, 50).
    assert_eq!(world.apply([0.0, 0.0]), [60.0, 50.0]);
}

#[test]
fn node_scale_applies_to_the_shape_and_its_subtree() {
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene::new(SceneNode {
        glow: black(),
        // Scale 2x in x only; the transform still positions the node.
        transform: Transform::translate([10.0, 0.0]),
        scale: [2.0, 1.0],
        modulate: WHITE,
        lit: false,
        occludes: false,
        order: 0.0,
        shape: Some(Shape::Circle {
            center: [1.0, 1.0],
            radius: 1.0,
            color: black(),
        }),
        children: vec![Box::new(SceneNode {
            glow: black(),
            transform: Transform::translate([1.0, 0.0]),
            scale: [1.0, 1.0],
            modulate: WHITE,
            lit: false,
            occludes: false,
            order: 0.0,
            shape: Some(Shape::Circle {
                center: [0.0, 0.0],
                radius: 1.0,
                color: black(),
            }),
            children: vec![],
        })],
    });
    canvas.draw_scene(&scene);
    let [
        Draw::Shape {
            world: w0,
            center: c0,
            aa: aa0,
            ..
        },
        Draw::Shape {
            world: w1,
            center: c1,
            ..
        },
    ] = &canvas.draws[..]
    else {
        panic!("expected two shape draws");
    };
    // The node's scale applies before its transform: (1, 1) -> (2, 1)
    // -> (12, 1) in user space, (62, 49) in pixel space.
    assert_eq!(w0.apply(*c0), [62.0, 49.0]);
    // The child's (1, 0) translation is scaled by the parent's 2x:
    // (0, 0) -> (1, 0) -> (2, 0) -> (12, 0) in user space, (62, 50) in
    // pixel space.
    assert_eq!(w1.apply(*c1), [62.0, 50.0]);
    // The AA band compensates for the 2x world scale.
    assert!((aa0 - AA_BAND / 2.0).abs() < 1e-6);
}

#[test]
fn node_order_accumulates_down_the_tree_like_the_transform() {
    // A chain of nodes with orders 1, 2, 4: each node's own shape draws
    // at the order inherited from its ancestors plus its own, and the
    // children inherit that total, so the z's are 1, 3, 7.
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene::new(SceneNode {
        glow: black(),
        transform: Transform::identity(),
        scale: [1.0, 1.0],
        modulate: WHITE,
        lit: false,
        occludes: false,
        order: 1.0,
        shape: Some(Shape::Circle {
            center: [0.0, 0.0],
            radius: 1.0,
            color: black(),
        }),
        children: vec![Box::new(SceneNode {
            glow: black(),
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: WHITE,
            lit: false,
            occludes: false,
            order: 2.0,
            shape: Some(Shape::Circle {
                center: [0.0, 0.0],
                radius: 1.0,
                color: black(),
            }),
            children: vec![Box::new(SceneNode {
                glow: black(),
                transform: Transform::identity(),
                scale: [1.0, 1.0],
                modulate: WHITE,
                lit: false,
                occludes: false,
                order: 4.0,
                shape: Some(Shape::Circle {
                    center: [0.0, 0.0],
                    radius: 1.0,
                    color: black(),
                }),
                children: vec![],
            })],
        })],
    });
    canvas.draw_scene(&scene);
    let zs = canvas
        .draws
        .iter()
        .map(|draw| draw.z())
        .collect::<Vec<_>>();
    assert_eq!(zs, vec![1.0, 3.0, 7.0]);
}

#[test]
fn node_modulate_multiplies_into_the_shape_and_its_subtree() {
    // A chain of nodes: the root modulates red to half and blue out,
    // the child modulates green to half and opacity to half. Each node's
    // own shape color is the channel-wise product of the modulates on
    // the path from the root, accumulated like the order.
    let white = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene::new(SceneNode {
        glow: black(),
        transform: Transform::identity(),
        scale: [1.0, 1.0],
        modulate: Color {
            r: 0.5,
            g: 1.0,
            b: 0.0,
            a: 1.0,
        },
        lit: false,
        occludes: false,
        order: 0.0,
        shape: Some(Shape::Circle {
            center: [0.0, 0.0],
            radius: 1.0,
            color: white,
        }),
        children: vec![Box::new(SceneNode {
            glow: black(),
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: Color {
                r: 1.0,
                g: 0.5,
                b: 1.0,
                a: 0.5,
            },
            lit: false,
            occludes: false,
            order: 0.0,
            shape: Some(Shape::Circle {
                center: [0.0, 0.0],
                radius: 1.0,
                color: white,
            }),
            children: vec![],
        })],
    });
    canvas.draw_scene(&scene);
    let [
        Draw::Shape { color: c0, .. },
        Draw::Shape { color: c1, .. },
    ] = &canvas.draws[..]
    else {
        panic!("expected two shape draws");
    };
    // The root's own shape is white times its own modulate.
    assert_eq!(*c0, Color { r: 0.5, g: 1.0, b: 0.0, a: 1.0 });
    // The child inherits the root's modulate multiplied by its own:
    // blue stays zeroed by the root, and the child's half opacity
    // multiplies into the alpha channel.
    assert_eq!(*c1, Color { r: 0.5, g: 0.5, b: 0.0, a: 0.5 });
}

#[test]
fn node_modulate_multiplies_sprite_tint_and_text_color_not_their_alpha() {
    // The modulate multiplies the sprite's tint and the text's color,
    // channel by channel, but leaves the separate `alpha` opacity field
    // untouched: the opacity is not a color.
    let mut canvas = Canvas::new((100, 100));
    canvas.draw_scene(&Scene::new(SceneNode {
        glow: black(),
        transform: Transform::identity(),
        scale: [1.0, 1.0],
        modulate: Color {
            r: 0.5,
            g: 1.0,
            b: 1.0,
            a: 0.5,
        },
        lit: false,
        occludes: false,
        order: 0.0,
        shape: Some(Shape::Sprite {
            data: Arc::new([0u8; 16]),
            width: 4,
            height: 4,
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            alpha: 0.25,
        }),
        children: vec![],
    }));
    canvas.draw_scene(&Scene::new(SceneNode {
        glow: black(),
        transform: Transform::identity(),
        scale: [1.0, 1.0],
        modulate: Color {
            r: 0.25,
            g: 0.5,
            b: 1.0,
            a: 1.0,
        },
        lit: false,
        occludes: false,
        order: 0.0,
        shape: Some(Shape::Text {
            text: "hi".to_string(),
            font: Arc::new([0u8]),
            size: 12.0,
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            alpha: 0.75,
        }),
        children: vec![],
    }));
    let [
        Draw::Sprite { tint, alpha, .. },
        Draw::Text { color, alpha: t_alpha, .. },
    ] = &canvas.draws[..]
    else {
        panic!("expected a sprite and a text draw");
    };
    assert_eq!(*tint, Color { r: 0.5, g: 1.0, b: 1.0, a: 0.5 });
    assert_eq!(*alpha, 0.25);
    assert_eq!(*color, Color { r: 0.25, g: 0.5, b: 1.0, a: 1.0 });
    assert_eq!(*t_alpha, 0.75);
}

#[test]
fn scene_sprite_at_user_origin_lands_at_window_center() {
    // A sprite node at the user-space origin must be centered on the
    // window center (not offset to a corner), and its texture size, tint
    // and z must travel onto the draw untouched.
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene::new(SceneNode {
        glow: black(),
        transform: Transform::identity(),
        scale: [1.0, 1.0],
        modulate: WHITE,
        lit: false,
        occludes: false,
        order: 0.0,
        shape: Some(Shape::Sprite {
            data: Arc::new([0u8; 16]),
            width: 4,
            height: 4,
            color: black(),
            alpha: 1.0,
        }),
        children: vec![],
    });
    canvas.draw_scene(&scene);
    let [Draw::Sprite {
        world,
        data,
        size,
        texture_size,
        aa,
        tint,
        alpha,
        glow,
        uv_rect,
        lit,
        z,
        ..
    }] = &canvas.draws[..]
    else {
        panic!("expected one sprite draw");
    };
    // The sprite's local space is centered on the origin.
    assert_eq!(world.apply([0.0, 0.0]), [50.0, 50.0]);
    assert_eq!(*size, [4.0, 4.0]);
    assert_eq!(*texture_size, [4, 4]);
    assert_eq!(data.len(), 16);
    assert!((*aa - AA_BAND).abs() < 1e-6);
    assert_eq!(*tint, black());
    assert_eq!(*alpha, 1.0);
    // The node carries no glow: it packs as black.
    assert_eq!(*glow, black());
    assert_eq!(*uv_rect, [0.0, 0.0, 1.0, 1.0]);
    // The node is unlit, so the flag packs as 0.0.
    assert_eq!(*lit, 0.0);
    assert_eq!(*z, 0.0);
}

#[test]
fn sprite_scissor_is_the_texture_box_plus_aa_band() {
    let draw = Draw::Sprite {
        glow: black(),
        world: Transform::translate([50.0, 50.0]),
        data: Arc::new([0u8; 16]),
        size: [40.0, 20.0],
        texture_size: [40, 20],
        aa: 0.75,
        tint: black(),
        alpha: 1.0,
        uv_rect: [0.0, 0.0, 1.0, 1.0],
        lit: 0.0,
        generation: 0,
        z: 0.0,
    };
    // The box is (50 ± 20.75, 50 ± 10.75), i.e. [29.25, 70.75] on x and
    // [39.25, 60.75] on y.
    assert_eq!(draw.scissor_rect([100, 100]), Some([29, 39, 42, 22]));
}

#[test]
fn sprite_scissor_under_rotation() {
    // A 45° rotation about the sprite's own center: the local box
    // (±20.75, ±10.75) rotates to an axis-aligned box with half-extent
    // (20.75 + 10.75) / sqrt(2) = 22.274 on both axes, centered on
    // (50, 50).
    let draw = Draw::Sprite {
        glow: black(),
        world: Transform::rotate(std::f32::consts::FRAC_PI_4)
            .compose(&Transform::translate([50.0, 50.0])),
        data: Arc::new([0u8; 16]),
        size: [40.0, 20.0],
        texture_size: [40, 20],
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

#[test]
fn sprite_uniform_bytes_follow_the_wgsl_layout() {
    // Lock the byte layout of `sprite_uniform_data` to the WGSL
    // uniform-space layout of `SpriteUniforms`, the same way the shape
    // test does: the mat2x2 `<f32>` spans 0..16 (column 0 @ 0, column 1
    // @ 8), `translation` @ 16, `size` @ 24, the tint vec4 (16 bytes,
    // 16-byte aligned) @ 32 spanning 32..48, the scalar alpha @ 48, the
    // scalar lit @ 52, the `uv_rect` vec4 (16-byte aligned) @ 64 spanning
    // 64..80, and the glow vec4 (16-byte aligned) @ 80 spanning 80..96;
    // the struct size is 96.
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
        1.0,
        [0.25, 0.5, 0.75, 1.0],
    );
    assert_eq!(data.len(), 96);

    let f32_at = |off: usize| {
        f32::from_le_bytes(data[off..off + 4].try_into().unwrap())
    };
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
    // The glow vec4 follows the uv_rect, 16-byte aligned at 80 (80..96).
    assert_eq!(f32_at(80), 0.6);
    assert_eq!(f32_at(84), 0.7);
    assert_eq!(f32_at(88), 0.8);
    assert_eq!(f32_at(92), 0.9);
}

/// The font file used by the expand_text tests, loaded from the crate's
/// asset directory.
fn test_font() -> Arc<[u8]> {
    Arc::from(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/fonts/JameGem08_2026-Regular.ttf"
        ))
        .expect("test font should be readable"),
    )
}

#[test]
fn expand_text_splices_glyph_sprites_in_place() {
    // A text draw between two circle draws must be replaced by its
    // glyph sprites without disturbing the neighbours' positions: the
    // circles keep their slots and the sprites inherit the text's tint,
    // alpha and z.
    let font = test_font();
    let size = 48.0;
    let mut canvas = Canvas::new((800, 600));
    let tint = Color { r: 1.0, g: 0.5, b: 0.25, a: 1.0 };
    canvas.draws = vec![
        Draw::Circle {
            center: [-100.0, 0.0],
            radius: 10.0,
            color: black(),
            z: 0.0,
        },
        Draw::Text {
            glow: black(),
            world: Transform::identity(),
            font: font.clone(),
            text: "hi".to_string(),
            size,
            color: tint,
            alpha: 0.9,
            lit: 1.0,
            z: 1.0,
        },
        Draw::Circle {
            center: [100.0, 0.0],
            radius: 10.0,
            color: black(),
            z: 2.0,
        },
    ];
    let mut atlases: HashMap<(u64, u32), text::Atlas> = HashMap::new();
    canvas.expand_text(&mut atlases);

    // The layout's glyph count is the source of truth for how many
    // sprites the text should produce (every glyph of "hi" has ink).
    let layout =
        text::layout(&font, "hi", size).expect("the test font should shape 'hi'");
    assert_eq!(canvas.draws.len(), 2 + layout.glyphs.len());
    let [Draw::Circle { z: z0, .. }, .., Draw::Circle { z: z1, .. }] =
        &canvas.draws[..]
    else {
        panic!("the circles must keep their slots");
    };
    assert_eq!(*z0, 0.0);
    assert_eq!(*z1, 2.0);
    for draw in canvas.draws.iter().skip(1).take(layout.glyphs.len()) {
        let Draw::Sprite { size, texture_size, uv_rect, tint, alpha, lit, z, .. } = draw
        else {
            panic!("every spliced draw must be a sprite");
        };
        assert_eq!(*tint, Color { r: 1.0, g: 0.5, b: 0.25, a: 1.0 });
        assert_eq!(*alpha, 0.9);
        // The block's `lit` flag passes on to every glyph quad.
        assert_eq!(*lit, 1.0);
        assert_eq!(*z, 1.0);
        // Glyph quads are a sub-rectangle of the 512x512 atlas.
        assert_eq!(*texture_size, [512, 512]);
        assert!(size[0] > 0.0 && size[1] > 0.0);
        assert!(uv_rect[0] < uv_rect[2]);
        assert!(uv_rect[1] < uv_rect[3]);
    }
}

#[test]
fn expand_text_reuses_the_atlas_across_frames() {
    // A second frame with the same font, text and size must not
    // re-rasterize: the atlas data Arc stays the same pointer, so the
    // GPU texture is reused and no bytes are re-uploaded.
    let font = test_font();
    let size: f32 = 48.0;
    let mut atlases: HashMap<(u64, u32), text::Atlas> = HashMap::new();
    let key = (Arc::as_ptr(&font) as *const () as u64, size.to_bits());

    let mut frame = || {
        let mut canvas = Canvas::new((800, 600));
        canvas.draws = vec![Draw::Text {
            glow: black(),
            world: Transform::identity(),
            font: font.clone(),
            text: "hello world".to_string(),
            size,
            color: Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
            alpha: 1.0,
            lit: 0.0,
            z: 0.0,
        }];
        canvas.expand_text(&mut atlases);
        let Draw::Sprite { data, .. } = &canvas.draws[0] else {
            panic!("the first glyph must be a sprite");
        };
        Arc::as_ptr(data)
    };
    let first = frame();
    let second = frame();
    assert_eq!(first, second, "frame two must reuse the atlas buffer");
    // The atlas was actually registered under its font key.
    assert!(atlases.contains_key(&key));
}

#[test]
fn expand_text_packs_glyphs_first_seen_on_a_later_frame() {
    // The atlas key (font, size) is registered on the first frame; a later
    // frame whose text introduces new glyphs must rasterize them into the
    // same persistent atlas and draw them. The diagnostics readout relies
    // on this: its "FPS" line's glyphs never appear in its "size" line's
    // first-frame text.
    let font = test_font();
    let size = 48.0;
    let mut atlases: HashMap<(u64, u32), text::Atlas> = HashMap::new();

    let mut frame = |text: &str| -> usize {
        let mut canvas = Canvas::new((800, 600));
        canvas.draws = vec![Draw::Text {
            glow: black(),
            world: Transform::identity(),
            font: font.clone(),
            text: text.to_string(),
            size,
            color: Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
            alpha: 1.0,
            lit: 0.0,
            z: 0.0,
        }];
        canvas.expand_text(&mut atlases);
        canvas.draws.len()
    };

    // The first frame seeds the atlas with its seven inked glyphs.
    assert_eq!(frame("800x600"), 7);
    // The second frame's "F", "P", "S" and "5" have never been packed
    // before: all five inked glyphs must be drawn ("8" was packed by the
    // first text; the space has no ink and gets no quad).
    assert_eq!(
        frame("FPS 58"),
        5,
        "glyphs first seen on a later frame must be packed and drawn"
    );
}

#[test]
fn sprite_loader_round_trips_a_png_file() {
    let path = std::env::temp_dir().join(format!(
        "frost-sprite-test-{}.png",
        std::process::id()
    ));
    let buf: Vec<u8> = [
        [255u8, 0, 0, 255],
        [0, 255, 0, 128],
        [0, 0, 255, 0],
        [10, 20, 30, 40],
    ]
    .iter()
    .flat_map(|pixel| pixel.iter().copied())
    .collect();
    image::save_buffer(&path, &buf, 2, 2, image::ColorType::Rgba8)
        .expect("writing the test png");
    let shape = match Shape::sprite(&path) {
        Ok(shape) => shape,
        Err(err) => panic!("failed to load the test png: {err}"),
    };
    let Shape::Sprite {
        data,
        width,
        height,
        color,
        alpha,
    } = shape else {
        panic!("expected a sprite shape");
    };
    assert_eq!((width, height), (2, 2));
    // The decoded RGBA8 buffer matches the file's pixels byte for byte.
    assert_eq!(&data[..], &buf[..]);
    // The default tint is white and the default opacity is 1.0.
    assert_eq!(
        color,
        Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0
        }
    );
    assert_eq!(alpha, 1.0);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn cloned_sprites_share_their_pixel_buffer() {
    // The pixels live behind an `Arc`: cloning a sprite shape must not
    // copy the buffer.
    let path = std::env::temp_dir().join(format!(
        "frost-sprite-share-{}.png",
        std::process::id()
    ));
    let buf = [255u8, 0, 0, 255, 0, 255, 0, 255];
    image::save_buffer(&path, &buf, 2, 1, image::ColorType::Rgba8)
        .expect("writing the test png");
    let shape = Shape::sprite(&path).unwrap();
    let Shape::Sprite { data: a, .. } = &shape else {
        panic!("expected a sprite shape");
    };
    let Shape::Sprite { data: b, .. } = &shape.clone() else {
        panic!("expected a sprite shape");
    };
    assert!(Arc::ptr_eq(a, b));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn sprite_loader_reports_a_missing_file_as_io() {
    let err = Shape::sprite("frost-missing-sprite-file.png").unwrap_err();
    assert!(matches!(
        err,
        SpriteError::Io(err) if err.kind() == std::io::ErrorKind::NotFound
    ));
}

/// A custom test node that records that its `process` has run.
#[derive(Debug)]
struct Visited {
    /// Whether `process` has run.
    done: bool,
}

impl Node for Visited {
    fn process(&mut self) {
        self.done = true;
    }
    fn visit(&mut self) {
        self.process();
    }
}

/// A custom test parent, in the shape of the trait's docs: it owns its
/// child and visits the child before processing itself.
#[derive(Debug)]
struct VisitParent {
    child: Box<Visited>,
    /// The child's state as observed when this node was processed.
    saw_done: bool,
}

impl Node for VisitParent {
    fn process(&mut self) {
        self.saw_done = self.child.done;
    }
    fn visit(&mut self) {
        self.child.visit();
        self.process();
    }
}

#[test]
fn node_visit_runs_children_before_their_parent() {
    // Post-order: when the parent's `process` runs, its child's
    // `process` must have already run.
    let mut parent = VisitParent {
        child: Box::new(Visited { done: false }),
        saw_done: false,
    };
    parent.visit();
    assert!(parent.child.done);
    assert!(parent.saw_done);
}

#[test]
fn present_mode_follows_the_vsync_flag() {
    assert_eq!(present_mode_for(true), PresentMode::AutoVsync);
    assert_eq!(present_mode_for(false), PresentMode::AutoNoVsync);
}

#[test]
fn config_defaults_to_vsync_on() {
    assert!(crate::Config::default().vsync);
}
