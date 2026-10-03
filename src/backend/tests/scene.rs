//! Scene-graph flattening: transforms compose down the tree, and shapes land where the user put them.

use super::super::*;
use super::*;
use crate::Canvas;
use crate::objects::*;
use std::sync::Arc;

#[test]
fn draw_scene_composes_transforms_down_the_tree() {
    let mut canvas = Canvas::new((100, 100), 1.0);
    let scene = Scene::new(SceneNode {
        diagnostic: false,
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
            diagnostic: false,
            glow: black(),
            transform: Transform::scale_uniform(2.0),
            scale: [1.0, 1.0],
            modulate: WHITE,
            lit: false,
            occludes: false,
            order: 0.0,
            shape: None,
            children: vec![Box::new(SceneNode {
                diagnostic: false,
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
            aa: aa1,
            ..
        },
    ] = &canvas.draws[..]
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
    let mut canvas = Canvas::new((100, 100), 1.0);
    let scene = Scene::new(SceneNode {
        diagnostic: false,
        glow: black(),
        transform: Transform::rotate(std::f32::consts::FRAC_PI_2),
        scale: [1.0, 1.0],
        modulate: WHITE,
        lit: false,
        occludes: false,
        order: 0.0,
        shape: None,
        children: vec![Box::new(SceneNode {
            diagnostic: false,
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
    let [
        Draw::Shape {
            world: w,
            center: c,
            ..
        },
    ] = &canvas.draws[..]
    else {
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
    let mut canvas = Canvas::new((100, 100), 1.0);
    let scene = Scene::new(SceneNode {
        diagnostic: false,
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
    let [
        Draw::Shape {
            world: w,
            center: c,
            ..
        },
    ] = &canvas.draws[..]
    else {
        panic!("expected one shape draw");
    };
    assert_eq!(w.apply(*c), [50.0, 50.0]);
}

#[test]
fn scene_sprite_at_user_origin_lands_at_window_center() {
    // A sprite node at the user-space origin must be centered on the
    // window center (not offset to a corner), and its texture size, tint
    // and z must travel onto the draw untouched.
    let mut canvas = Canvas::new((100, 100), 1.0);
    let scene = Scene::new(SceneNode {
        diagnostic: false,
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
            filter: SpriteFilter::Linear,
            generation: 0,
        }),
        children: vec![],
    });
    canvas.draw_scene(&scene);
    let [
        Draw::Sprite {
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
        },
    ] = &canvas.draws[..]
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
