//! Three lights bouncing around the window like billiard balls.
//!
//! Every frame, each light moves at a constant velocity and reflects off the
//! wall it reaches, DVD-logo style, so the three colored pools of light keep
//! crossing and mixing over the scene's lit receivers: the huge backdrop
//! rectangle, the floor slab, and the circle and rectangle resting on and
//! above it. The bounce box is measured from [`frost::Context::size`] every
//! frame, so resizing the window simply moves the walls.
//!
//! Lights are never drawn, so each one is marked by two immediate circles: a
//! soft tinted halo and a small white core. Immediate draws are unlit, so
//! the markers keep their full brightness wherever the light travels.
//!
//! The scene's two rectangular receivers are flagged `occludes`, so they
//! also cut hard shadows into the light: as a light sweeps past a rectangle,
//! the lit backdrop behind it falls into that rectangle's silhouette. Only
//! rectangles occlude, so the two circles never cast shadows, and the huge
//! backdrop is a receiver only — flagging it would shadow the whole scene.
//!
//! The starting directions and speeds are randomized once at startup, so no
//! two runs bounce alike; the speeds differ enough that the pattern never
//! settles into a repeat.
//!
//! Run with:
//!
//! ```text
//! cargo run --example bouncing_lights
//! ```

/// The warm light's color: amber.
const AMBER: frost::Color = frost::Color {
    r: 1.0,
    g: 0.6,
    b: 0.25,
    a: 1.0,
};

/// The cool light's color: azure.
const AZURE: frost::Color = frost::Color {
    r: 0.45,
    g: 0.75,
    b: 1.0,
    a: 1.0,
};

/// The third light's color: mint green.
const MINT: frost::Color = frost::Color {
    r: 0.45,
    g: 1.0,
    b: 0.55,
    a: 1.0,
};

/// The shared strength of the three lights.
const INTENSITY: f32 = 1.4;

/// The shared falloff extent of the three lights, in pixels.
const RADIUS: f32 = 240.0;

/// How much the per-frame breathing pulses the light's falloff extent, as a
/// fraction of [`RADIUS`].
const PULSE: f32 = 0.06;

/// The core marker's radius, in pixels.
const CORE_RADIUS: f32 = 5.0;

/// The halo marker's radius, in pixels.
const HALO_RADIUS: f32 = 18.0;

/// The alpha of the halo marker: a soft tint around the core.
const HALO_ALPHA: f32 = 0.3;

/// How far the light's center stays from the walls, in pixels, so the core
/// marker never clips into a wall.
const MARGIN: f32 = 10.0;

/// The starting speed range, in pixels per second; each light gets one in
/// between.
const SPEED: (f32, f32) = (110.0, 190.0);

/// The breathing rate of the pulse, in radians per second.
const PULSE_RATE: f32 = 1.7;

/// One bouncing light: a position carried by a constant velocity that
/// reflects off the window's walls.
struct Bouncer {
    /// The light's position, in the window's user space.
    pos: [f32; 2],
    /// The light's velocity, in pixels per second. Only its components ever
    /// change sign, so the speed holds across bounces.
    vel: [f32; 2],
    /// The light's color, also used to tint its halo marker.
    color: frost::Color,
    /// The pulse phase offset, so the three lights breathe out of step.
    phase: f32,
}

impl Bouncer {
    /// Creates a bouncer at a random spot in the middle band of the default
    /// window, heading off at a random angle and speed from `rng`.
    fn spawn(rng: &mut frost::Rng, color: frost::Color, phase: f32) -> Self {
        let angle = rng.in_range(0.0, std::f32::consts::TAU);
        let speed = rng.in_range(SPEED.0, SPEED.1);
        Self {
            pos: [rng.in_range(-180.0, 180.0), rng.in_range(-80.0, 80.0)],
            vel: [angle.cos() * speed, angle.sin() * speed],
            color,
            phase,
        }
    }

    /// Advances one frame and reflects off the walls: the box the center may
    /// occupy is half the window, inset by [`MARGIN`]. Clamping to the wall
    /// as the velocity component is flipped to point inward makes a bounce
    /// idempotent, so a window shrink that leaves a light outside the new box
    /// snaps it to the nearest wall instead of losing it.
    fn step(&mut self, half: [f32; 2], dt: f32) {
        let hx = (half[0] - MARGIN).max(0.0);
        let hy = (half[1] - MARGIN).max(0.0);

        self.pos[0] += self.vel[0] * dt;
        self.pos[1] += self.vel[1] * dt;

        if self.pos[0] <= -hx {
            self.pos[0] = -hx;
            self.vel[0] = self.vel[0].abs();
        } else if self.pos[0] >= hx {
            self.pos[0] = hx;
            self.vel[0] = -self.vel[0].abs();
        }
        if self.pos[1] <= -hy {
            self.pos[1] = -hy;
            self.vel[1] = self.vel[1].abs();
        } else if self.pos[1] >= hy {
            self.pos[1] = hy;
            self.vel[1] = -self.vel[1].abs();
        }
    }
}

struct Demo {
    /// The elapsed time, in seconds: the clock of the pulse.
    t: f32,
    /// The three bouncing lights.
    lights: [Bouncer; 3],
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        self.t += dt;

        // The bounce box follows the window, measured fresh each frame.
        let (w, h) = ctx.size();
        let half = [w / 2.0, h / 2.0];

        for light in &mut self.lights {
            light.step(half, dt);

            // The light itself: one immediate light call per frame, gently
            // breathing so the pool's edge visibly moves even between
            // bounces.
            let pulse = 1.0 + PULSE * (self.t * PULSE_RATE + light.phase).sin();
            ctx.light(
                light.pos[0],
                light.pos[1],
                light.color,
                INTENSITY,
                RADIUS * pulse,
            );

            // Its markers: immediate draws are unlit, so they stay bright
            // wherever they travel. The halo takes the light's own color at
            // low alpha, the core stays white-hot.
            ctx.circle(
                light.pos[0],
                light.pos[1],
                HALO_RADIUS,
                frost::Color {
                    a: HALO_ALPHA,
                    ..light.color
                },
                2.0,
            );
            ctx.circle(
                light.pos[0],
                light.pos[1],
                CORE_RADIUS,
                frost::Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                2.0,
            );
        }
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    let mut rng = frost::Rng::new();
    let lights = [
        Bouncer::spawn(&mut rng, AMBER, 0.0),
        Bouncer::spawn(&mut rng, AZURE, std::f32::consts::TAU / 3.0),
        Bouncer::spawn(&mut rng, MINT, 2.0 * std::f32::consts::TAU / 3.0),
    ];

    let mut scene = frost::Scene::new(frost::SceneNode {
        // Near-black background: the clear color behind the lit backdrop,
        // visible only outside the huge rectangle's reach.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.03,
                g: 0.03,
                b: 0.05,
                a: 1.0,
            },
        }),
        children: vec![
            Box::new(frost::SceneNode {
                // The backdrop: a huge lit rectangle far beyond the window's
                // edges, so every light pools on "the walls" wherever it
                // bounces.
                order: -1.0,
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, 0.0],
                    extent: [2000.0, 2000.0],
                    color: frost::Color {
                        r: 0.3,
                        g: 0.3,
                        b: 0.4,
                        a: 1.0,
                    },
                }),
                lit: true,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The floor slab, low in the default window: a brighter
                // strip the pools sweep along. It occludes, so a light
                // passing below it throws the slab's silhouette up over the
                // backdrop.
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, -210.0],
                    extent: [430.0, 26.0],
                    color: frost::Color {
                        r: 0.6,
                        g: 0.6,
                        b: 0.68,
                        a: 1.0,
                    },
                }),
                lit: true,
                occludes: true,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // A lit circle, upper left: one of the pieces of furniture
                // the pools cross on their way between walls.
                shape: Some(frost::Shape::Circle {
                    center: [-260.0, 110.0],
                    radius: 55.0,
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
                // A lit rectangle, upper right. Like the floor slab it
                // occludes, dragging a hard shadow across the backdrop as
                // the lights swing past.
                shape: Some(frost::Shape::Rectangle {
                    center: [240.0, 80.0],
                    extent: [70.0, 45.0],
                    color: frost::Color {
                        r: 0.45,
                        g: 0.6,
                        b: 0.75,
                        a: 1.0,
                    },
                }),
                lit: true,
                occludes: true,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // A small lit circle, low center, just above the floor.
                shape: Some(frost::Shape::Circle {
                    center: [80.0, -130.0],
                    radius: 26.0,
                    color: frost::Color {
                        r: 0.7,
                        g: 0.5,
                        b: 0.65,
                        a: 1.0,
                    },
                }),
                lit: true,
                ..Default::default()
            }),
        ],
        ..Default::default()
    });
    // The ambient floor of the light field: a dark blue-gray, so the areas
    // no light has visited recently stay dim but never fully black.
    scene.ambient = frost::Color {
        r: 0.1,
        g: 0.1,
        b: 0.14,
        a: 1.0,
    };

    if let Err(err) = frost::run(
        scene,
        Demo {
            t: 0.0,
            lights,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
