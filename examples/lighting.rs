//! A lighting demo: a lit floor, a lit ball with an ember plume, and three
//! lights sweeping across them.
//!
//! The receivers are the floor (a lit rectangle), the ball (a lit circle),
//! and the plume (a lit particle fountain): every pixel of a lit receiver
//! is multiplied, per frame, by the scene's ambient color plus the
//! contribution of every light in the frame's light field, falling off
//! quadratically with the distance to each light.
//!
//! The emitters show the two ways to make a light:
//!
//! - The orbiting light is a scene node holding a [`frost::Shape::Light`]:
//!   it is never drawn, and its transform is updated each frame to carry
//!   the light around the ball. A node light rides its node's transform —
//!   and inherits its modulate tint — exactly as a shape does.
//! - The cursor-chasing light and the static light are immediate
//!   [`frost::Canvas::light`] calls in the window's user space. The frame's
//!   light field is rebuilt from that frame's calls, so moving a light
//!   means calling it again with the new position.
//!
//! Move the mouse to steer the chasing light; while it is out of the window
//! the light wanders on its own, so the demo is alive from the first frame.
//!
//! Run with:
//!
//! ```text
//! cargo run --example lighting
//! ```

/// The plume's emission rate, in particles per second.
const PLUME_RATE: f32 = 120.0;

/// The ember lifetime range, in seconds; each ember gets one in between.
const PLUME_LIFE: (f32, f32) = (0.8, 1.6);

/// The ember size range, in pixels (a radius); each ember gets one in
/// between.
const PLUME_SIZE: (f32, f32) = (2.0, 4.5);

/// The plume's gravity, in pixels per second, per second: a gentle pull so
/// the embers arc over instead of drifting straight up.
const PLUME_GRAVITY: [f32; 2] = [0.0, -90.0];

/// The orbit's center and semi-axes, in the window's user space: the light
/// sweeps an ellipse around the ball, over the floor.
const ORBIT_CENTER: [f32; 2] = [0.0, -50.0];
const ORBIT_AXES: [f32; 2] = [190.0, 120.0];

/// The orbit's angular speed, in radians per second.
const ORBIT_SPEED: f32 = 0.8;

/// The static light's position, in the window's user space: a warm corner
/// lamp on the right.
const STATIC_POS: [f32; 2] = [310.0, -70.0];

/// The orbiting light's color: warm yellow.
const ORBIT_LIGHT: frost::Color = frost::Color {
    r: 1.0,
    g: 0.85,
    b: 0.45,
    a: 1.0,
};

/// The cursor-chasing light's color: cool blue.
const CHASE_LIGHT: frost::Color = frost::Color {
    r: 0.45,
    g: 0.75,
    b: 1.0,
    a: 1.0,
};

/// The static light's color: amber.
const STATIC_LIGHT: frost::Color = frost::Color {
    r: 1.0,
    g: 0.5,
    b: 0.25,
    a: 1.0,
};

struct Demo {
    /// The elapsed time, in seconds: the orbit angle and the wander path.
    t: f32,
    /// The chasing light's eased position, in the window's user space.
    chase: [f32; 2],
    /// The plume's spawn accumulator: `PLUME_RATE * dt` is added each frame
    /// and one ember is spawned per whole unit, so the rate holds at any dt.
    acc: f32,
    /// The random source, for the per-particle jitter.
    rng: frost::Rng,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        self.t += dt;

        // The cursor while it is inside the window, otherwise a slow wander
        // across the left of the window, so the chasing light keeps moving
        // before the mouse has ever moved in and after it leaves.
        let (w, h) = ctx.size();
        let target = ctx.mouse_position().unwrap_or([
            -w * 0.25 * (self.t * 0.6).sin(),
            -h * 0.1 + h * 0.18 * (self.t * 0.37 + 1.2).sin(),
        ]);

        // The orbiting light: a scene node holding a Shape::Light, carried
        // around the ball by its transform.
        let orbit = &mut ctx.scene().root.children[3];
        let angle = self.t * ORBIT_SPEED;
        orbit.transform = frost::Transform::translate([ORBIT_CENTER[0] + ORBIT_AXES[0] * angle.cos(), ORBIT_CENTER[1] + ORBIT_AXES[1] * angle.sin()]);

        // The chasing light: an immediate draw in the window's user space,
        // eased toward its target with frame-rate-independent smoothing.
        let k = 1.0 - (-7.0 * dt).exp();
        self.chase[0] += (target[0] - self.chase[0]) * k;
        self.chase[1] += (target[1] - self.chase[1]) * k;
        ctx.light(self.chase[0], self.chase[1], CHASE_LIGHT, 1.1, 170.0);
        // The static light: an immediate draw at a fixed spot; the field is
        // rebuilt every frame, so it is simply called again.
        ctx.light(STATIC_POS[0], STATIC_POS[1], STATIC_LIGHT, 0.9, 150.0);

        // The plume: a fountain of embers rising off the ball, a lit
        // receiver like the floor and the ball.
        let plume = &mut ctx.scene().root.children[2];
        if let Some(frost::Shape::Particles { system, .. }) = plume.shape.as_mut() {
            self.acc += PLUME_RATE * dt;
            while self.acc >= 1.0 {
                self.acc -= 1.0;
                let life = self.rng.in_range(PLUME_LIFE.0, PLUME_LIFE.1);
                system.spawn(frost::Particle {
                    pos: [self.rng.in_range(-12.0, 12.0), -8.0],
                    vel: [
                        self.rng.in_range(-18.0, 18.0),
                        self.rng.in_range(50.0, 110.0),
                    ],
                    life,
                    max_life: life,
                    size: self.rng.in_range(PLUME_SIZE.0, PLUME_SIZE.1),
                    angle: 0.0,
                    color: frost::Color {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                });
            }
            system.update(dt, PLUME_GRAVITY);
        }
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    let mut scene = frost::Scene::new(frost::SceneNode {
        // Near-black background; the lit receivers do the talking.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.04,
                g: 0.04,
                b: 0.07,
                a: 1.0,
            },
        }),
        children: vec![
            Box::new(frost::SceneNode {
                // The floor: a wide lit slab the light pools sweep across.
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, -130.0],
                    extent: [420.0, 40.0],
                    color: frost::Color {
                        r: 0.55,
                        g: 0.55,
                        b: 0.62,
                        a: 1.0,
                    },
                }),
                lit: true,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The ball: a lit circle resting on the floor.
                shape: Some(frost::Shape::Circle {
                    center: [0.0, -50.0],
                    radius: 40.0,
                    color: frost::Color {
                        r: 0.75,
                        g: 0.6,
                        b: 0.5,
                        a: 1.0,
                    },
                }),
                lit: true,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The plume: a lit particle fountain rising off the ball,
                // drawn just above it.
                shape: Some(frost::Shape::Particles {
                    system: frost::ParticleSystem::new(),
                    color: frost::Color {
                        r: 1.0,
                        g: 0.62,
                        b: 0.25,
                        a: 1.0,
                    },
                    shape: frost::ParticleShape::Circle,
                }),
                order: 1.0,
                lit: true,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The orbiting light: a node that is never drawn; its
                // transform is updated each frame to carry the light around.
                transform: frost::Transform::translate([ORBIT_CENTER[0] + ORBIT_AXES[0], ORBIT_CENTER[1]]),
                shape: Some(frost::Shape::Light {
                    light: frost::Light::point(ORBIT_LIGHT, 1.3, 190.0),
                }),
                ..Default::default()
            }),
        ],
        ..Default::default()
    });
    // The ambient floor of the light field: a dark blue-gray, so unlit
    // areas stay dim but never fully black.
    scene.ambient = frost::Color {
        r: 0.12,
        g: 0.12,
        b: 0.16,
        a: 1.0,
    };

    if let Err(err) = frost::run(
        scene,
        Demo {
            t: 0.0,
            chase: [0.0, 0.0],
            acc: 0.0,
            rng: frost::Rng::new(),
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
