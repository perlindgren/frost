struct Demo {
    /// Orbit and tumble angle in radians.
    spin: f32,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Canvas, dt: f32) {
        self.spin += dt;
        // Deep indigo background, set per frame.
        ctx.set_background(frost::Color {
            r: 0.09,
            g: 0.06,
            b: 0.16,
        });

        // A scene tree: a central circle, a rectangle orbiting it while it
        // tumbles, and a small circle riding on the rectangle that
        // counter-rotates.
        //
        // Each node holds a transform relative to its parent plus its own
        // shape; the transform applies to the node's shape and composes onto
        // its children.
        let scene = frost::Scene::new(frost::SceneNode {
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
            children: vec![
                Box::new(frost::SceneNode {
                    // Rotate around the origin, then push out to the orbit
                    // radius: the rectangle orbits while the rotation tumbles
                    // its own shape.
                    transform: frost::Transform::rotate(self.spin)
                        .compose(&frost::Transform::translate(140.0, 0.0)),
                    shape: Some(frost::Shape::Rectangle {
                        center: [0.0, 0.0],
                        extent: [40.0, 20.0],
                        color: frost::Color {
                            r: 0.9,
                            g: 0.45,
                            b: 0.2,
                        },
                    }),
                    children: vec![Box::new(frost::SceneNode {
                        // Counter-rotates in the rectangle's space, so the
                        // small circle wobbles as it rides the orbit.
                        transform: frost::Transform::rotate(-2.0 * self.spin),
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
                }),
            ],
        });
        ctx.draw_scene(&scene);
        log::trace!("process: dt {:?}", dt);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");
    if let Err(err) = frost::run(Demo { spin: 0.0 }) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
