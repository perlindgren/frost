//! The scene.rs example with the rectangle and its children refactored into
//! a `Satellite` node: the orbit, the tumble, and the small circle's
//! counter-rotation are computed in `Satellite`'s `frost::Node::process`
//! instead of the top-level process, so the satellite animates itself with a
//! single `visit`. The top-level process keeps only what is outside the
//! satellite — the background and the center circle — and reads the
//! satellite's spin to stay in sync.
//!
//! Because the satellite is drawn as its own scene, the rectangle orbits
//! the window center instead of riding the center circle's orbit. And
//! because `Node::process` has no `dt`, the satellite advances a fixed spin
//! step per frame (0.25 rad/s at 60 fps), so its speed is frame-rate
//! dependent.
//!
//! Run with:
//!
//! ```text
//! cargo run --example satellite
//! ```

use std::f32::consts::{PI, TAU};

use frost::Node;

/// The spin advanced per frame: `Node::process` has no `dt`, so the orbit
/// speed is a fixed step (0.25 rad/s at 60 fps, as in scene.rs).
const SPIN_PER_FRAME: f32 = 0.25 / 60.0;

/// The rectangle from scene.rs and its counter-rotating child, refactored
/// into one node that animates itself: the orbit and tumble of the
/// rectangle, and the counter-rotation of the small circle, are all
/// computed here, not in the top-level process.
struct Satellite {
    /// The rectangle and its children, drawn as an additional scene every
    /// frame.
    scene: frost::Scene,
    /// Orbit and tumble angle in radians.
    spin: f32,
}

impl Satellite {
    /// Builds the satellite's scene: the orbiting, tumbling rectangle with
    /// the small counter-rotating circle riding on it. The rectangle
    /// carries `order: 1.0` so it sorts in front of the center circle,
    /// like its position below the circle in scene.rs's tree.
    fn new() -> Self {
        let scene = frost::Scene::new(frost::SceneNode {
            shape: Some(frost::Shape::Rectangle {
                center: [0.0, 0.0],
                extent: [20.0, 20.0],
                color: frost::Color {
                    r: 0.9,
                    g: 0.45,
                    b: 0.2,
                    a: 1.0,
                },
            }),
            order: 1.0,
            children: vec![Box::new(frost::SceneNode {
                shape: Some(frost::Shape::Circle {
                    center: [55.0, 0.0],
                    radius: 12.0,
                    color: frost::Color {
                        r: 0.95,
                        g: 0.85,
                        b: 0.3,
                        a: 1.0,
                    },
                }),
                ..Default::default()
            })],
            ..Default::default()
        });
        Self { scene, spin: 0.0 }
    }
}

impl frost::Node for Satellite {
    /// Visits the subtree first (post-order: the children before the
    /// satellite's own update), then animates the satellite itself.
    fn visit(&mut self) {
        self.scene.visit();
        self.process();
    }

    /// Advances the spin and drives the whole animation from it: the
    /// rectangle orbits the origin while it tumbles, the small circle
    /// counter-rotates in the rectangle's space, and the whole subtree
    /// blooms from a point as the rectangle first reaches the bottom of
    /// its orbit (the orbit angle is 3 * spin, so the first bottom is at
    /// spin = PI / 3).
    fn process(&mut self) {
        self.spin += SPIN_PER_FRAME;

        // The eased lerp (smoothstep) starts and ends at zero velocity, so
        // the satellite eases in from rest and settles exactly at full
        // size as the rectangle bottoms out; the clamp holds it there.
        let progress = (self.spin / (PI / 3.0)).clamp(0.0, 1.0);
        let s = progress * progress * (3.0 - 2.0 * progress);
        self.scene.root.scale = [s, s];

        // Rotate around the parent, then push out to the orbit radius: the
        // rectangle orbits while the rotation tumbles its own shape.
        self.scene.root.transform =
            frost::Transform::rotate(self.spin).compose(&frost::Transform::translate(
                (self.spin * 3.0 % TAU).sin() * 200.0,
                (self.spin * 3.0 % TAU).cos() * 200.0,
            ));
        // The small circle counter-rotates in the rectangle's space, so it
        // wobbles as it rides the orbit.
        self.scene.root.children[0].transform = frost::Transform::rotate(-5.0 * self.spin);
    }
}

struct Demo {
    /// The self-animating satellite, visited once per frame.
    satellite: Satellite,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, _dt: f32) {
        // The satellite drives itself: one visit advances its spin and
        // animates the rectangle and its counter-rotating child.
        self.satellite.visit();

        // The rest of the scene shares the satellite's spin: the root
        // blooms from a point as the satellite first bottoms out, and the
        // center circle orbits at the same base rate.
        let progress = (self.satellite.spin / (PI / 3.0)).clamp(0.0, 1.0);
        let s = progress * progress * (3.0 - 2.0 * progress);
        ctx.scene().root.scale = [s, s];
        ctx.scene().root.children[0].transform = frost::Transform::translate(
            (self.satellite.spin % TAU).sin() * 140.0,
            (self.satellite.spin % TAU).cos() * 100.0,
        );

        // Draw the satellite's own scene into this frame; its rectangle
        // sorts in front of the center circle via `order: 1.0`.
        ctx.draw_scene(&self.satellite.scene);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    let scene = frost::Scene::new(frost::SceneNode {
        // Deep indigo background; the node's transform is ignored.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.09,
                g: 0.06,
                b: 0.16,
                a: 1.0,
            },
        }),
        children: vec![Box::new(frost::SceneNode {
            shape: Some(frost::Shape::Circle {
                center: [0.0, 0.0],
                radius: 90.0,
                color: frost::Color {
                    r: 0.25,
                    g: 0.35,
                    b: 0.6,
                    a: 1.0,
                },
            }),
            ..Default::default()
        })],
        ..Default::default()
    });

    if let Err(err) = frost::run(scene, Demo { satellite: Satellite::new() }) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
