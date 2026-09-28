//! Camera anchoring, layer speed, view rotation and the missing-camera fallback.

use super::super::*;
use super::*;
use crate::Canvas;
use crate::objects::*;

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
                diagnostic: false,
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
                        diagnostic: false,
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
                            color: Color {
                                r: 1.0,
                                g: 0.0,
                                b: 0.0,
                                a: 1.0,
                            },
                        }),
                        children: vec![],
                    }),
                    Box::new(SceneNode {
                        diagnostic: false,
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
            diagnostic: false,
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
                diagnostic: false,
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
                        color: Color {
                            r: 1.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        },
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
                        color: Color {
                            r: 0.0,
                            g: 0.0,
                            b: 1.0,
                            a: 1.0,
                        },
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

#[test]
fn the_camera_rotation_turns_the_view() {
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene {
        root: SceneNode {
            diagnostic: false,
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
                    diagnostic: false,
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
                        color: Color {
                            r: 1.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        },
                    }),
                    children: vec![],
                }),
                Box::new(SceneNode {
                    diagnostic: false,
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
            diagnostic: false,
            glow: black(),
            transform: Transform::identity(),
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
                    color: Color {
                        r: 1.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    },
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
