//! A scene tree animated in place: a central circle, a rectangle orbiting it
//! while it tumbles, and a small circle riding on the rectangle that
//! counter-rotates.
//!
//! The scene is built once and passed to `frost::run`, which draws it every
//! frame. The process advances the orbit angle and mutates the nodes'
//! transforms through `ctx.scene()`. Run with:
//!
//! ```text
//! cargo run --example scene
//! ```

use std::f32::consts::TAU;

struct Demo {
    /// Orbit and tumble angle in radians.
    spin: f32,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        self.spin += dt * 0.25;

        // Move the center circle according to the clamped spin angle.
        let circle = &mut ctx.scene().root.children[0];
        circle.transform = frost::Transform::translate(
            (self.spin % TAU).sin() * 140.0,
            (self.spin % TAU).cos() * 100.0,
        );

        // Rotate around the parent, then push out to the orbit radius: the
        // rectangle orbits while the rotation tumbles its own shape.
        let orbit = &mut ctx.scene().root.children[0].children[0];
        orbit.transform =
            frost::Transform::rotate(self.spin).compose(&frost::Transform::translate(
                (self.spin * 3.0 % TAU).sin() * 200.0,
                (self.spin * 3.0 % TAU).cos() * 200.0,
            ));
        // The small circle counter-rotates in the rectangle's space, so it
        // wobbles as it rides the orbit.
        orbit.children[0].transform = frost::Transform::rotate(-5.0 * self.spin);
        log::trace!("process: dt {:?}", dt);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    let scene = frost::Scene::new(frost::SceneNode {
        transform: frost::Transform::identity(),
        // Deep indigo background; the node's transform is ignored.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.09,
                g: 0.06,
                b: 0.16,
            },
        }),
        children: vec![Box::new(frost::SceneNode {
            transform: frost::Transform::identity(),
            shape: Some(frost::Shape::Circle {
                center: [0.0, 0.0],
                radius: 90.0,
                color: frost::Color {
                    r: 0.25,
                    g: 0.35,
                    b: 0.6,
                },
            }),
            children: vec![Box::new(frost::SceneNode {
                transform: frost::Transform::identity(),
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, 0.0],
                    extent: [20.0, 20.0],
                    color: frost::Color {
                        r: 0.9,
                        g: 0.45,
                        b: 0.2,
                    },
                }),
                children: vec![Box::new(frost::SceneNode {
                    transform: frost::Transform::identity(),
                    shape: Some(frost::Shape::Circle {
                        center: [55.0, 0.0],
                        radius: 12.0,
                        color: frost::Color {
                            r: 0.95,
                            g: 0.85,
                            b: 0.3,
                        },
                    }),
                    children: vec![],
                })],
            })],
        })],
    });

    if let Err(err) = frost::run(scene, Demo { spin: 0.0 }) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
