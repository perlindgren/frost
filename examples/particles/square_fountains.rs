//! A variant of the `fountains` example: the middle fountain spits out
//! rotating squares, each in its own color.
//!
//! The left and right fountains are the same as in the original — green
//! circles and a circulating rainbow — and the waterfall still falls
//! behind them all, each drawn through the (deprecated) immediate particle
//! draw. The middle fountain is the new thing: its batch is a node's
//! [`frost::Shape::Particles`] shape with a
//! [`frost::ParticleShape::Rectangle`], so every drop is a square, and each
//! square is spawned with its own random hue and its own spin rate. The
//! spin is a render property the simulation never touches, so the process
//! advances it one step per frame. The node rides the identity transform,
//! so the squares live in the window's user space just like the circles,
//! and its order of 1.0 puts the plume in front of its two neighbors.
//!
//! Run with:
//!
//! ```text
//! cargo run --example square_fountains
//! ```

/// The gravity, in pixels per second, per second, pulling everything down
/// (y points up, so it is negative).
const GRAVITY: f32 = 420.0;

/// The fountains' emission rate, in particles per second, each.
const RATE: f32 = 140.0;

/// The base half-width of the launch cone, in radians around straight up;
/// the breathing multiplies it.
const SPREAD: f32 = 0.30;

/// The circle fountains' draw size range, in pixels (a radius).
const SIZE: (f32, f32) = (2.0, 4.5);

/// The square fountain's draw size range, in pixels (a half-width, so the
/// squares are 5 to 12 pixels on a side) — a little chunkier than the
/// circles, since they are the point of the demo.
const SQUARE_SIZE: (f32, f32) = (2.5, 6.0);

/// The squares' spin-rate range, in radians per second, each spawned with
/// a random sign: slow enough to read as tumbling, fast enough to be
/// alive.
const SPIN: (f32, f32) = (1.5, 5.0);

/// The circle fountains' horizontal offsets, as a fraction of the window
/// width from the center: the left and right plumes, close enough to
/// partially overlap the middle one.
const OFFSETS: [f32; 2] = [-0.24, 0.24];

/// The fountains' height above the bottom edge, in pixels.
const RISE: f32 = 90.0;

/// The middle fountain's pulse frequency, in cycles per second, and phase,
/// in radians: its beat, desynchronized from the two circle fountains.
const MID_FREQ: f32 = 0.75;
const MID_PHASE: f32 = 2.1;

/// The z-orders: the waterfall sits behind everything, the circle
/// fountains at zero, and the middle square plume in front of them.
const WATERFALL_Z: f32 = -1.0;
const CIRCLE_Z: f32 = 0.0;
const SQUARE_Z: f32 = 1.0;

/// The waterfall's top margin: its drops spawn this far below the top
/// edge, in pixels.
const TOP_MARGIN: f32 = 100.0;

/// The waterfall's emission rate, in particles per second.
const FALL_RATE: f32 = 260.0;

/// The waterfall's lifetime range, in seconds: long enough to cross the
/// screen at common window sizes before fading out.
const FALL_LIFE: (f32, f32) = (2.0, 3.4);

/// The waterfall's draw size range, in pixels (a radius).
const FALL_SIZE: (f32, f32) = (1.5, 3.5);

/// The waterfall's color: a lighter blue than the middle fountain's.
const FALL_COLOR: frost::Color = frost::Color {
    r: 0.62,
    g: 0.80,
    b: 1.0,
    a: 0.8,
};

/// The left fountain's color: green.
const GREEN: frost::Color = frost::Color {
    r: 0.30,
    g: 0.85,
    b: 0.35,
    a: 1.0,
};

/// The middle source's color: white — a neutral emitter for a plume that
/// carries the whole spectrum.
const WHITE: frost::Color = frost::Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};

/// The right fountain's color at time `t`: a cosine palette circulating
/// through the whole spectrum, one full cycle every ~7.7 seconds.
fn cycle(t: f32) -> frost::Color {
    palette(TAU * 0.13 * t)
}

/// A color from the cosine palette at angle `u`: a full sweep of `u` walks
/// the whole spectrum. The circulating fountain steps through it over
/// time, and the square fountain picks each square's hue from it at
/// spawn.
fn palette(u: f32) -> frost::Color {
    frost::Color {
        r: 0.5 + 0.5 * u.cos(),
        g: 0.5 + 0.5 * (u + TAU / 3.0).cos(),
        b: 0.5 + 0.5 * (u + 2.0 * TAU / 3.0).cos(),
        a: 1.0,
    }
}

const TAU: f32 = 2.0 * std::f32::consts::PI;

/// One circle fountain: its particles, its emission accumulator, and its
/// own pulse frequency and phase — so the two beat at different paces.
struct Fountain {
    /// The particles: the simulation state, stepped once per frame.
    system: frost::ParticleSystem,
    /// The emission accumulator: `RATE * dt` is added each frame and one
    /// particle is spawned per whole unit.
    acc: f32,
    /// The pulse frequency, in cycles per second.
    freq: f32,
    /// The pulse phase, in radians: desynchronizes the fountains.
    phase: f32,
}

struct Demo {
    /// The two circle fountains: green (left), circulating (right).
    fountains: [Fountain; 2],
    /// The square fountain's emission accumulator: its particles live in
    /// the node's [`frost::Shape::Particles`] shape, not here.
    square_acc: f32,
    /// The squares' spin rates, in radians per second: one per live
    /// square, in the same order as the node's particles.
    spins: Vec<f32>,
    /// The waterfall's particles.
    fall: frost::ParticleSystem,
    /// The waterfall's emission accumulator.
    fall_acc: f32,
    /// The random source, for the per-particle jitter: the engine's
    /// `frost::Rng`, seeded from the clock.
    rng: frost::Rng,
    /// The demo time, in seconds: the clock the modulations run on.
    t: f32,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let (w, h) = ctx.size();
        self.t += dt;

        // The circle fountains: each beats at its own pace. The pulse sets
        // the launch speed, a slower clock sways the launch direction, and
        // a third breathes the cone open and shut — incommensurate
        // frequencies, so the pattern never exactly repeats.
        // The tallest surge should just crest the middle of the screen:
        // the apex of a launch at speed v is v^2 / (2 * GRAVITY) above
        // the source, so the peak speed follows the window height.
        let rise = (h / 2.0 - RISE).max(40.0);
        let peak = (2.0 * GRAVITY * rise).sqrt();

        for (i, f) in self.fountains.iter_mut().enumerate() {
            let wt = TAU * f.freq * self.t + f.phase;
            let pulse = 0.5 + 0.5 * wt.sin();
            let tilt = 0.12 * (0.6 * wt).sin();
            let breath = 0.5 + 0.5 * (0.9 * wt).sin();
            // The plume undulates between 80% and 100% of the cresting
            // speed; the per-particle jitter only pulls down from it.
            let speed = peak * (0.8 + 0.2 * pulse);
            let spread = SPREAD * (0.75 + 0.45 * breath);

            let x = OFFSETS[i] * w;
            let y = -h / 2.0 + RISE;
            f.acc += RATE * dt;
            while f.acc >= 1.0 {
                f.acc -= 1.0;
                let angle = TAU / 4.0 + tilt + self.rng.in_range(-spread, spread);
                let v = speed * self.rng.in_range(0.8, 1.0);
                // A life long enough to fall back toward the source: the
                // flight time is about 2v/g, jittered.
                let life = 2.0 * v / GRAVITY * self.rng.in_range(0.8, 1.25);
                f.system.spawn(frost::Particle {
                    pos: [x + self.rng.in_range(-5.0, 5.0), y],
                    vel: [angle.cos() * v, angle.sin() * v],
                    life,
                    max_life: life,
                    size: self.rng.in_range(SIZE.0, SIZE.1),
                    angle: 0.0,
                    color: WHITE,
                });
            }
            f.system.update(dt, [0.0, -GRAVITY]);
        }

        // The square fountain: the middle plume, a batch of rotating
        // squares in the node's [`frost::Shape::Particles`] shape. The
        // node rides the identity transform, so the squares spawn in the
        // window's user space, at the center, just like the circles do.
        if let Some(frost::Shape::Particles { system, .. }) =
            ctx.scene().root.children[0].shape.as_mut()
        {
            // Each square spins at its own rate: one step per frame, added
            // to the angle the simulation never touches.
            for (p, spin) in system.particles.iter_mut().zip(self.spins.iter()) {
                p.angle += *spin * dt;
            }
            // The update removes the dead particles in place, so the same
            // spin rates have to go: a square survives this frame iff its
            // life is still positive after the step — the very test the
            // update applies.
            let keep: Vec<bool> = system.particles.iter().map(|p| p.life - dt > 0.0).collect();
            system.update(dt, [0.0, -GRAVITY]);
            let mut keep = keep.into_iter();
            self.spins.retain(|_| keep.next().unwrap());

            let wt = TAU * MID_FREQ * self.t + MID_PHASE;
            let pulse = 0.5 + 0.5 * wt.sin();
            let tilt = 0.12 * (0.6 * wt).sin();
            let breath = 0.5 + 0.5 * (0.9 * wt).sin();
            let speed = peak * (0.8 + 0.2 * pulse);
            let spread = SPREAD * (0.75 + 0.45 * breath);

            let y = -h / 2.0 + RISE;
            self.square_acc += RATE * dt;
            while self.square_acc >= 1.0 {
                self.square_acc -= 1.0;
                let angle = TAU / 4.0 + tilt + self.rng.in_range(-spread, spread);
                let v = speed * self.rng.in_range(0.8, 1.0);
                let life = 2.0 * v / GRAVITY * self.rng.in_range(0.8, 1.25);
                system.spawn(frost::Particle {
                    pos: [self.rng.in_range(-5.0, 5.0), y],
                    vel: [angle.cos() * v, angle.sin() * v],
                    life,
                    max_life: life,
                    size: self.rng.in_range(SQUARE_SIZE.0, SQUARE_SIZE.1),
                    // Each square starts at its own orientation and spins
                    // at its own rate, in a random direction.
                    angle: self.rng.in_range(0.0, TAU),
                    color: palette(self.rng.in_range(0.0, TAU)),
                });
                self.spins.push(
                    self.rng.in_range(SPIN.0, SPIN.1)
                        * if self.rng.next_f32() < 0.5 {
                            1.0
                        } else {
                            -1.0
                        },
                );
            }
        }

        // The waterfall: drops fall from a line TOP_MARGIN below the top
        // edge, spread across the whole width.
        self.fall_acc += FALL_RATE * dt;
        while self.fall_acc >= 1.0 {
            self.fall_acc -= 1.0;
            let life = self.rng.in_range(FALL_LIFE.0, FALL_LIFE.1);
            self.fall.spawn(frost::Particle {
                pos: [self.rng.in_range(-w / 2.0, w / 2.0), h / 2.0 - TOP_MARGIN],
                vel: [self.rng.in_range(-15.0, 15.0), -self.rng.in_range(30.0, 120.0)],
                life,
                max_life: life,
                size: self.rng.in_range(FALL_SIZE.0, FALL_SIZE.1),
                angle: 0.0,
                color: WHITE,
            });
        }
        self.fall.update(dt, [0.0, -GRAVITY]);

        // Draw the free batches: the frame z-sorts them, so the waterfall
        // (z -1) sits behind the fountains. The middle plume is not drawn
        // here — the node's batch is drawn by the scene, at z 1, in front
        // of the two circle fountains (z 0). Each particle's alpha is its
        // remaining life fraction, so everything fades as it dies.
        // Free batches in the window's user space, so they stay on the
        // (deprecated) immediate particle draw instead of riding a node's
        // transform.
        #[allow(deprecated)]
        {
            ctx.particles(&self.fall.particles, FALL_COLOR, WATERFALL_Z);
            for i in 0..2 {
                let color = if i == 0 { GREEN } else { cycle(self.t) };
                ctx.particles(&self.fountains[i].system.particles, color, CIRCLE_Z);
            }
        }

        // The sources, behind the plumes.
        ctx.circle(OFFSETS[0] * w, -h / 2.0 + RISE, 6.0, GREEN, -0.5);
        ctx.circle(0.0, -h / 2.0 + RISE, 6.0, WHITE, -0.5);
        ctx.circle(OFFSETS[1] * w, -h / 2.0 + RISE, 6.0, cycle(self.t), -0.5);

        let n: usize = self.fountains.iter().map(|f| f.system.len()).sum::<usize>()
            + self.spins.len()
            + self.fall.len();
        log::trace!("process: dt {:?} particles {:?}", dt, n);
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
                // The middle fountain: a batch of rotating squares in the
                // window's user space (the identity transform), in front
                // of the two circle fountains. The batch's color is white,
                // so each square's own color shows through untouched.
                shape: Some(frost::Shape::Particles {
                    system: frost::ParticleSystem::new(),
                    color: WHITE,
                    shape: frost::ParticleShape::Rectangle { aspect: 1.0 },
                }),
                order: SQUARE_Z,
                ..frost::SceneNode::default()
            })],
            ..Default::default()
        }),
        Demo {
            fountains: [
                Fountain {
                    system: frost::ParticleSystem::new(),
                    acc: 0.0,
                    freq: 0.5,
                    phase: 0.0,
                },
                Fountain {
                    system: frost::ParticleSystem::new(),
                    acc: 0.0,
                    freq: 0.6,
                    phase: 4.2,
                },
            ],
            square_acc: 0.0,
            spins: Vec::new(),
            fall: frost::ParticleSystem::new(),
            fall_acc: 0.0,
            rng: frost::Rng::new(),
            t: 0.0,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
