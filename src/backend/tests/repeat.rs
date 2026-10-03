//! Repeating layers: copies, tile splitting, camera offsets and the abort cases.

use super::super::*;
use super::*;
use crate::Canvas;
use crate::objects::*;

fn repeat_layer_scene(repeat: [f32; 2]) -> Scene {
    Scene {
        root: SceneNode::default(),
        layers: vec![Layer {
            order: 0.0,
            speed: 1.0,
            repeat,
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
                    center: [-50.0, -50.0],
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
        }],
        ambient: AMBIENT,
        camera: None,
    }
}

#[test]
fn a_zero_repeat_layer_draws_its_content_once() {
    let mut canvas = Canvas::new((100, 100), 1.0);
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
    let mut canvas = Canvas::new((100, 100), 1.0);
    // The window spans user x in [-50, 50]: with a period of 100, the
    // circle straddling the bottom edge is visible in its base copy at
    // the left edge and in its copy displaced by one period at the
    // right edge Ã¢â‚¬â€ two copies, and no copy further out, because the
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
    let mut canvas = Canvas::new((100, 100), 1.0);
    // A camera at user x = 60 shifts the window to layer x in
    // [10, 110]: the circle's base copy has scrolled fully off the
    // window's left edge, and only its copy displaced by one period
    // reaches it Ã¢â‚¬â€ at the window's bottom edge.
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
    // window, though its tile [100, 200) does not overlap it Ã¢â‚¬â€ it must
    // be drawn, or it would pop into the middle of the window the
    // moment a moving camera's box crossed the tile boundary at 100.
    let mut canvas = Canvas::new((100, 100), 1.0);
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
                    color: Color {
                        r: 1.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    },
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
    let mut canvas = Canvas::new((100, 100), 1.0);
    // A rectangle twenty wide, centered at layer x = 95: it straddles
    // the tile boundary at 100. With a period of 100 (greater than its
    // 20 width), it is legal: the window must see its two copies, one
    // at each edge.
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
                diagnostic: false,
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
                    color: Color {
                        r: 1.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    },
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
    let mut canvas = Canvas::new((100, 100), 1.0);
    let scene = repeat_layer_scene([50.0, 0.0]);
    canvas.draw_scene(&scene);
}

#[test]
#[should_panic(expected = "drawn twice")]
fn an_object_wider_than_its_repeat_aborts() {
    let mut canvas = Canvas::new((100, 100), 1.0);
    // A rectangle one hundred and twenty wide exceeds its period of
    // 100: its copies would overlap, so the same object would be drawn
    // twice.
    let scene = Scene {
        layers: vec![Layer {
            order: 0.0,
            speed: 1.0,
            repeat: [100.0, 0.0],
            root: SceneNode {
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
                    extent: [60.0, 5.0],
                    color: Color {
                        r: 1.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    },
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
    // is visible at all four corners of the window Ã¢â‚¬â€ four copies.
    let mut canvas = Canvas::new((100, 100), 1.0);
    let scene = repeat_layer_scene([100.0, 100.0]);
    canvas.draw_scene(&scene);
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 4);
    // One axis repeats: two copies, not four.
    let mut canvas = Canvas::new((100, 100), 1.0);
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
    // the window's box Ã¢â‚¬â€ which, under the following camera, is always Ã¢â‚¬â€
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
                            color: Color {
                                r: 1.0,
                                g: 0.0,
                                b: 0.0,
                                a: 1.0,
                            },
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
    let mut canvas = Canvas::new((100, 100), 1.0);
    canvas.draw_scene(&scene([100.0, 0.0], 60.0));
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 1);
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected a shape draw");
    };
    assert_eq!(world.apply([0.0, 0.0]), [50.0, 50.0]);
    // pos = 150: the object has wandered a window and a half from the
    // origin; it is still at the window center.
    let mut canvas = Canvas::new((100, 100), 1.0);
    canvas.draw_scene(&scene([100.0, 0.0], 150.0));
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 1);
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected a shape draw");
    };
    assert_eq!(world.apply([0.0, 0.0]), [50.0, 50.0]);
    // A non-repeating layer has no such limit: the same object is
    // drawn at the window center, wherever it is.
    let mut canvas = Canvas::new((100, 100), 1.0);
    canvas.draw_scene(&scene([0.0, 0.0], 150.0));
    let painted = canvas.paint_order();
    assert_eq!(painted.len(), 1);
    let Draw::Shape { world, .. } = &painted[0] else {
        panic!("expected a shape draw");
    };
    assert_eq!(world.apply([0.0, 0.0]), [50.0, 50.0]);
}
