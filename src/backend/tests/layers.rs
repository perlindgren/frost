//! Layers as hard draw partitions, z-ordering within and across them, backgrounds and the clear color.

use super::super::*;
use super::*;
use crate::Canvas;
use crate::objects::*;

#[test]
fn clear_color_falls_back_to_the_default_without_a_background() {
    let draws = [Draw::Circle {
        diagnostic: false,
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
            diagnostic: false,
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

#[test]
fn background_node_sorts_to_the_back_and_keeps_its_color() {
    let mut canvas = Canvas::new((100, 100));
    canvas.draw_scene(&Scene::new(SceneNode {
        diagnostic: false,
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
            diagnostic: false,
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
    canvas.draws.sort_by(|a, b| {
        a.z()
            .partial_cmp(&b.z())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let [Draw::Background { color }, Draw::Shape { .. }] = &canvas.draws[..] else {
        panic!(
            "expected one background and one shape, got {:?}",
            canvas.draws
        );
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
                    diagnostic: false,
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
                order: 1.0,
                speed: 1.0,
                repeat: [0.0, 0.0],
                root: SceneNode {
                    diagnostic: false,
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
                    diagnostic: false,
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
                            diagnostic: false,
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
                            transform: Transform::identity(),
                            scale: [1.0, 1.0],
                            modulate: WHITE,
                            lit: false,
                            occludes: false,
                            order: 200.0,
                            shape: Some(Shape::Circle {
                                center: [0.0, 0.0],
                                radius: 1.0,
                                color: Color {
                                    r: 0.0,
                                    g: 1.0,
                                    b: 0.0,
                                    a: 1.0,
                                },
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
    canvas.circle(
        0.0,
        0.0,
        1.0,
        Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        },
        0.0,
    );
    let scene = Scene {
        root: SceneNode::default(),
        layers: vec![Layer {
            order: 0.0,
            speed: 1.0,
            repeat: [0.0, 0.0],
            root: SceneNode {
                diagnostic: false,
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
                    color: Color {
                        r: 0.0,
                        g: 0.0,
                        b: 1.0,
                        a: 1.0,
                    },
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
            diagnostic: false,
            glow: black(),
            transform: Transform::identity(),
            scale: [1.0, 1.0],
            modulate: WHITE,
            lit: false,
            occludes: false,
            order: 0.0,
            shape: Some(Shape::Background {
                color: Color {
                    r: 0.1,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                },
            }),
            children: vec![],
        },
        layers: vec![Layer {
            order: 1.0,
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
                shape: Some(Shape::Background {
                    color: Color {
                        r: 0.0,
                        g: 0.1,
                        b: 0.0,
                        a: 1.0,
                    },
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
        Color {
            r: 0.0,
            g: 0.1,
            b: 0.0,
            a: 1.0
        }
    );
}

#[test]
fn without_layers_the_paint_order_is_the_global_z_sort() {
    let mut canvas = Canvas::new((100, 100));
    canvas.circle(
        0.0,
        0.0,
        1.0,
        Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        },
        1.0,
    );
    canvas.circle(
        0.0,
        0.0,
        1.0,
        Color {
            r: 0.0,
            g: 1.0,
            b: 0.0,
            a: 1.0,
        },
        -1.0,
    );
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
