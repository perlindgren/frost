//! Three partially overlapping fountains and a full-screen waterfall, all
//! simulated on the CPU and drawn as one batched (instanced) draw each
//! through [`frost::Canvas::particles`].
//!
//! Each fountain beats at its own slow pace: the launch speed undulates so
//! the plume's crest stays between 64% and 100% of the way to the middle
//! of the screen, the launch direction sways on a slower period, and the
//! cone breathes open and shut — three incommensurate clocks per fountain,
//! so the plumes never surge in unison. The tallest surge just crests the
//! middle of the screen. The left fountain is green, the
//! middle blue, and the right one circulates through the whole spectrum
//! via a cosine palette. Behind them all, a lighter blue waterfall falls
//! from a line 100 pixels below the top edge, at a z-order behind the
//! fountains.
//!
//! Every batch fades per particle: a particle's alpha is its remaining
//! life fraction, so plumes dissolve as their drops die.
//!
//! Run with:
//!
//! ```text
//! cargo run --example fountains
//! ```

/// The gravity, in pixels per second, per second, pulling everything down
/// (y points up, so it is negative).
const GRAVITY: f32 = 420.0;

/// The fountains' emission rate, in particles per second, each.
const RATE: f32 = 140.0;

/// The launch speed is set per frame so the tallest surge just crests the
/// middle of the screen: the apex of a launch at speed `v` is
/// `v^2 / (2 * GRAVITY)` above the source, so the peak speed follows the
/// window height (computed in `process`).

/// The base half-width of the launch cone, in radians around straight up;
/// the breathing multiplies it.
const SPREAD: f32 = 0.30;

/// The fountains' draw size range, in pixels (a radius).
const SIZE: (f32, f32) = (2.0, 4.5);

/// The fountains' horizontal offsets, as a fraction of the window width
/// from the center: close enough that the plumes partially overlap.
const OFFSETS: [f32; 3] = [-0.24, 0.0, 0.24];

/// The fountains' height above the bottom edge, in pixels.
const RISE: f32 = 90.0;

/// The z-orders: the waterfall sits behind everything, the middle
/// fountain in front of its two neighbors.
const WATERFALL_Z: f32 = -1.0;
const ZS: [f32; 3] = [0.0, 1.0, 0.0];

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

/// The middle fountain's color: blue.
const BLUE: frost::Color = frost::Color {
    r: 0.25,
    g: 0.50,
    b: 1.0,
    a: 1.0,
};

/// The right fountain's color at time `t`: a cosine palette circulating
/// through the whole spectrum, one full cycle every ~7.7 seconds.
fn cycle(t: f32) -> frost::Color {
    let k = 2.0 * std::f32::consts::PI;
    let u = k * 0.13 * t;
    frost::Color {
        r: 0.5 + 0.5 * u.cos(),
        g: 0.5 + 0.5 * (u + k / 3.0).cos(),
        b: 0.5 + 0.5 * (u + 2.0 * k / 3.0).cos(),
        a: 1.0,
    }
}

/// One fountain: its particles, its emission accumulator, and its own
/// pulse frequency and phase — so the three beat at different paces.
struct Fountain {
    /// The particles: the simulation state, stepped once per frame.
    system: frost::ParticleSystem,
    /// The emission accumulator: `RATE * dt` is added each frame and one
    /// particle is spawned per whole unit.
    acc: f32,
    /// The pulse frequency, in cycles per second.
    freq: f32,
    /// The pulse phase, in radians: desynchronizes the three fountains.
    phase: f32,
}

struct Demo {
    /// The three fountains: green (left), blue (middle), circulating
    /// (right).
    fountains: [Fountain; 3],
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

        // The fountains: each beats at its own pace. The pulse sets the
        // launch speed, a slower clock sways the launch direction, and a
        // third breathes the cone open and shut — incommensurate
        // frequencies, so the pattern never exactly repeats.
        // The tallest surge should just crest the middle of the screen:
        // the apex of a launch at speed v is v^2 / (2 * GRAVITY) above
        // the source, so the peak speed follows the window height.
        let rise = (h / 2.0 - RISE).max(40.0);
        let peak = (2.0 * GRAVITY * rise).sqrt();

        for (i, f) in self.fountains.iter_mut().enumerate() {
            let wt = 2.0 * std::f32::consts::PI * f.freq * self.t + f.phase;
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
                let angle = std::f32::consts::PI / 2.0 + tilt + self.rng.in_range(-spread, spread);
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
                });
            }
            f.system.update(dt, [0.0, -GRAVITY]);
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
            });
        }
        self.fall.update(dt, [0.0, -GRAVITY]);

        // Draw the batches: the frame z-sorts them, so the waterfall
        // (z -1) sits behind the fountains, and the middle fountain
        // (z 1) in front of its two neighbors (z 0). Each particle's
        // alpha is its remaining life fraction, so everything fades as
        // it dies.
        let colors = [GREEN, BLUE, cycle(self.t)];
        ctx.particles(&self.fall.particles, FALL_COLOR, WATERFALL_Z);
        for i in 0..3 {
            ctx.particles(&self.fountains[i].system.particles, colors[i], ZS[i]);
        }

        // The sources, behind the plumes.
        for i in 0..3 {
            ctx.circle(OFFSETS[i] * w, -h / 2.0 + RISE, 6.0, colors[i], -0.5);
        }

        let n: usize = self.fountains.iter().map(|f| f.system.len()).sum::<usize>()
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
                    freq: 0.75,
                    phase: 2.1,
                },
                Fountain {
                    system: frost::ParticleSystem::new(),
                    acc: 0.0,
                    freq: 0.6,
                    phase: 4.2,
                },
            ],
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
