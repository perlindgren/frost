//! Node inheritance: scale, order and modulate accumulate down the subtree, and visit is post-order.

use super::super::*;
use super::*;
use crate::Canvas;
use crate::objects::*;
use std::sync::Arc;

#[test]
fn node_scale_applies_to_the_shape_and_its_subtree() {
    let mut canvas = Canvas::new((100, 100));
    let scene = Scene::new(SceneNode {
        diagnostic: false,
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
            diagnostic: false,
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
        diagnostic: false,
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
            diagnostic: false,
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
                diagnostic: false,
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
    let zs = canvas.draws.iter().map(|draw| draw.z()).collect::<Vec<_>>();
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
        diagnostic: false,
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
            diagnostic: false,
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
    let [Draw::Shape { color: c0, .. }, Draw::Shape { color: c1, .. }] = &canvas.draws[..] else {
        panic!("expected two shape draws");
    };
    // The root's own shape is white times its own modulate.
    assert_eq!(
        *c0,
        Color {
            r: 0.5,
            g: 1.0,
            b: 0.0,
            a: 1.0
        }
    );
    // The child inherits the root's modulate multiplied by its own:
    // blue stays zeroed by the root, and the child's half opacity
    // multiplies into the alpha channel.
    assert_eq!(
        *c1,
        Color {
            r: 0.5,
            g: 0.5,
            b: 0.0,
            a: 0.5
        }
    );
}

#[test]
fn node_modulate_multiplies_sprite_tint_and_text_color_not_their_alpha() {
    // The modulate multiplies the sprite's tint and the text's color,
    // channel by channel, but leaves the separate `alpha` opacity field
    // untouched: the opacity is not a color.
    let mut canvas = Canvas::new((100, 100));
    canvas.draw_scene(&Scene::new(SceneNode {
        diagnostic: false,
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
            filter: SpriteFilter::Linear,
            generation: 0,
        }),
        children: vec![],
    }));
    canvas.draw_scene(&Scene::new(SceneNode {
        diagnostic: false,
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
            weight: 400.0,
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
        Draw::Text {
            color,
            alpha: t_alpha,
            ..
        },
    ] = &canvas.draws[..]
    else {
        panic!("expected a sprite and a text draw");
    };
    assert_eq!(
        *tint,
        Color {
            r: 0.5,
            g: 1.0,
            b: 1.0,
            a: 0.5
        }
    );
    assert_eq!(*alpha, 0.25);
    assert_eq!(
        *color,
        Color {
            r: 0.25,
            g: 0.5,
            b: 1.0,
            a: 1.0
        }
    );
    assert_eq!(*t_alpha, 0.75);
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
