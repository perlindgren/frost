//! A torch with two kinds of particles, side by side: embers that ride the
//! torch, and sparks that keep flying once they leave it.
//!
//! The embers live in the torch's [`frost::Shape::Particles`] shape, in the
//! torch's local space: a few spawn at the coal head every frame and drift
//! upward in the torch's own coordinates, and the batch is drawn through the
//! torch's world transform — so when the torch moves, tilts, or pulses, the
//! whole ember cloud is carried, tilted, and scaled along with it. The
//! embers are stuck to the torch.
//!
//! The sparks are the opposite: a free spray in the window's user space,
//! drawn through the (deprecated) [`frost::Canvas::particles`]. When the
//! torch moves fast enough, sparks are kicked out of the tip with a share of
//! the torch's velocity in reverse, plus a scatter — and then they are on
//! their own: gravity pulls them down, they lag behind the moving torch,
//! and the torch's scale pulse does not touch them.
//!
//! The torch chases the cursor while it is inside the window, and wanders on
//! its own until then (and after it leaves), so the demo is alive from the
//! first frame. Move the mouse quickly to kick up a spray of sparks, and
//! watch the embers follow every move of the torch.
//!
//! Run with:
//!
//! ```text
//! cargo run --example torch
//! ```

use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// The ember emission rate, in particles per second (the batch is small: the
/// torch's own cloud, not a fountain).
const EMBER_RATE: f32 = 70.0;

/// The embers' local buoyancy, in pixels per second, per second: a gentle
/// upward pull in the torch's own coordinates, so the cloud drifts off the
/// coal instead of falling.
const EMBER_BUOYANCY: [f32; 2] = [0.0, 26.0];

/// The ember lifetime range, in seconds; each ember gets one in between.
const EMBER_LIFE: (f32, f32) = (0.5, 1.3);

/// The ember size range, in pixels (a radius); each ember gets one in
/// between.
const EMBER_SIZE: (f32, f32) = (1.5, 3.5);

/// The speed, in pixels per second, above which the torch kicks up sparks.
const SPARK_THRESHOLD: f32 = 90.0;

/// The spark emission rate at exactly the threshold, in particles per
/// second; it scales up with the torch's speed, up to four times.
const SPARK_RATE: f32 = 90.0;

/// The spark gravity, in pixels per second, per second, pulling the free
/// spray down in the window's user space (y points up, so it is negative).
const SPARK_GRAVITY: [f32; 2] = [0.0, -500.0];

/// The spark lifetime range, in seconds; each spark gets one in between.
const SPARK_LIFE: (f32, f32) = (0.4, 1.0);

/// The spark size range, in pixels (a radius); each spark gets one in
/// between.
const SPARK_SIZE: (f32, f32) = (1.0, 2.5);

/// The ember color: the batch's base tint in the torch's
/// [`frost::Shape::Particles`] shape.
const EMBER: frost::Color = frost::Color {
    r: 1.0,
    g: 0.5,
    b: 0.18,
    a: 1.0,
};

/// The spark color, for the free spray drawn in the window's user space.
const SPARK: frost::Color = frost::Color {
    r: 1.0,
    g: 0.82,
    b: 0.4,
    a: 1.0,
};

struct Demo {
    /// The torch's position, in the window's user space (window-centered, y
    /// up): the tip of the torch, where the coal and the embers sit.
    pos: [f32; 2],
    /// The torch's tilt, in radians, 0.0 being upright (flame straight up);
    /// eased toward the direction of motion so the flame leans forward.
    angle: f32,
    /// The elapsed time, in seconds, for the wander path and the scale pulse.
    t: f32,
    /// The free spark spray: the simulation state, in the window's user
    /// space, stepped once per frame.
    spray: frost::ParticleSystem,
    /// The ember emission accumulator: `EMBER_RATE * dt` is added each frame
    /// and one ember is spawned per whole unit, so the rate holds at any dt.
    ember_acc: f32,
    /// The spark emission accumulator, for the speed-scaled spawn rate.
    spray_acc: f32,
    /// The random source, for the per-particle jitter: the engine's
    /// `frost::Rng`, seeded from the clock.
    rng: frost::Rng,
}

/// Wraps an angle difference into `[-pi, pi]`, so easing a rotation takes
/// the short way around instead of spinning the long way.
fn wrap_angle(a: f32) -> f32 {
    (a + PI).rem_euclid(TAU) - PI
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        self.t += dt;

        // The target: the cursor while it is inside the window, otherwise a
        // slow wander through the middle of the window, so the torch keeps
        // moving before the mouse has ever moved in and after it leaves.
        let (w, h) = ctx.size();
        let target = ctx.mouse_position().unwrap_or([
            w * 0.28 * (self.t * 0.8).sin(),
            h * 0.30 * (self.t * 0.53 + 1.3).sin(),
        ]);

        // Ease toward the target: exponential smoothing, independent of the
        // frame rate.
        let k = 1.0 - (-10.0 * dt).exp();
        let prev = self.pos;
        self.pos[0] += (target[0] - self.pos[0]) * k;
        self.pos[1] += (target[1] - self.pos[1]) * k;
        // The torch's velocity, in pixels per second, for the tilt and the
        // spark kickback; 0.0 on the first frame, when dt is 0.0.
        let vel = if dt > 0.0 {
            [(self.pos[0] - prev[0]) / dt, (self.pos[1] - prev[1]) / dt]
        } else {
            [0.0, 0.0]
        };
        let speed = (vel[0] * vel[0] + vel[1] * vel[1]).sqrt();

        // Lean the flame toward the direction of motion: a rotation that
        // sends the torch's local +y axis (the flame's way) onto the motion
        // direction is the motion's angle minus pi/2; below the threshold
        // the torch stands upright.
        let target_angle = if speed > SPARK_THRESHOLD {
            vel[1].atan2(vel[0]) - FRAC_PI_2
        } else {
            0.0
        };
        self.angle += wrap_angle(target_angle - self.angle) * (1.0 - (-8.0 * dt).exp());

        // The torch node: its transform carries the whole subtree — the
        // handle, the coal, and, crucially, the ember batch.
        let torch = &mut ctx.scene().root.children[0];
        torch.transform = frost::Transform::rotate(self.angle)
            .compose(&frost::Transform::translate(self.pos[0], self.pos[1]));
        // The pulse: the torch's scale oscillates about 1, and the ember
        // radii — and the whole torch — scale with the node.
        let s = 1.0 + 0.12 * (2.0 * self.t).sin();
        torch.scale = [s, s];

        // The embers: a batch in the torch's local space, held in the ember
        // node's Shape::Particles shape. They spawn at the coal and drift
        // upward in the torch's own coordinates, so the torch carries them
        // wherever it goes.
        let mut embers = 0;
        if let Some(frost::Shape::Particles { system, .. }) = torch.children[2].shape.as_mut() {
            self.ember_acc += EMBER_RATE * dt;
            while self.ember_acc >= 1.0 {
                self.ember_acc -= 1.0;
                let life = self.rng.in_range(EMBER_LIFE.0, EMBER_LIFE.1);
                system.spawn(frost::Particle {
                    pos: [
                        self.rng.in_range(-4.0, 4.0),
                        self.rng.in_range(2.0, 8.0),
                    ],
                    vel: [
                        self.rng.in_range(-14.0, 14.0),
                        self.rng.in_range(20.0, 44.0),
                    ],
                    life,
                    max_life: life,
                    size: self.rng.in_range(EMBER_SIZE.0, EMBER_SIZE.1),
                });
            }
            system.update(dt, EMBER_BUOYANCY);
            embers = system.len();
        }

        // The sparks: the free spray in the window's user space. The faster
        // the torch moves, the more it kicks up, up to four times the base
        // rate; each spark leaves with half the torch's velocity in reverse,
        // plus a scatter, and is then on its own under gravity.
        if speed > SPARK_THRESHOLD {
            self.spray_acc +=
                SPARK_RATE * (speed / SPARK_THRESHOLD).min(4.0) * dt;
        }
        while self.spray_acc >= 1.0 {
            self.spray_acc -= 1.0;
            let life = self.rng.in_range(SPARK_LIFE.0, SPARK_LIFE.1);
            self.spray.spawn(frost::Particle {
                pos: [
                    self.pos[0] + self.rng.in_range(-6.0, 6.0),
                    self.pos[1] + self.rng.in_range(-6.0, 6.0),
                ],
                vel: [
                    -vel[0] * 0.5 + self.rng.in_range(-70.0, 70.0),
                    -vel[1] * 0.5 + self.rng.in_range(-70.0, 70.0),
                ],
                life,
                max_life: life,
                size: self.rng.in_range(SPARK_SIZE.0, SPARK_SIZE.1),
            });
        }
        self.spray.update(dt, SPARK_GRAVITY);

        // The spray is a free batch in the window's user space — it lives on
        // the (deprecated) immediate particle draw, the embers ride the
        // torch's node instead.
        #[allow(deprecated)]
        {
            ctx.particles(&self.spray.particles, SPARK, 2.0);
        }

        log::trace!("process: dt {:?} embers {} sparks {:?}", dt, embers, self.spray.len());
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    if let Err(err) = frost::run(
        frost::Scene::new(frost::SceneNode {
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
                // The torch: a group node, moved, tilted, and scaled each
                // frame; the subtree rides its transform.
                children: vec![
                    Box::new(frost::SceneNode {
                        // The stick: a 6-by-60 bar hanging below the coal.
                        shape: Some(frost::Shape::Rectangle {
                            center: [0.0, -30.0],
                            extent: [3.0, 30.0],
                            color: frost::Color {
                                r: 0.35,
                                g: 0.22,
                                b: 0.12,
                                a: 1.0,
                            },
                        }),
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        // The coal head at the tip, where the embers spawn.
                        shape: Some(frost::Shape::Circle {
                            center: [0.0, 2.0],
                            radius: 7.0,
                            color: frost::Color {
                                r: 0.45,
                                g: 0.15,
                                b: 0.08,
                                a: 1.0,
                            },
                        }),
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        // The embers: a particle batch in the torch's local
                        // space, drawn above the coal.
                        shape: Some(frost::Shape::Particles {
                            system: frost::ParticleSystem::new(),
                            color: EMBER,
                        }),
                        order: 1.0,
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            })],
            ..Default::default()
        }),
        Demo {
            pos: [0.0, 0.0],
            angle: 0.0,
            t: 0.0,
            spray: frost::ParticleSystem::new(),
            ember_acc: 0.0,
            spray_acc: 0.0,
            rng: frost::Rng::new(),
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
