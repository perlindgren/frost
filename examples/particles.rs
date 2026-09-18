//! A fountain of particles: a [`frost::ParticleSystem`] driven from a
//! [`frost::Process`], drawn as fading circles through the immediate
//! `circle` method.
//!
//! Every frame the demo adds `RATE * dt` to an emission accumulator and
//! spawns one particle per whole unit: each starts at the source near the
//! bottom edge, with an upward velocity in a spread cone, a random
//! lifetime, and a random size. The system then updates every particle —
//! gravity pulls it down, its life runs out, and the dead ones are removed.
//! Each particle is drawn as a circle whose alpha is its remaining life
//! fraction, so the fountain fades as it rises.
//!
//! The simulation is pure math in the library; this example owns the
//! spawning and the drawing, the split the library makes for every effect.
//! Run with:
//!
//! ```text
//! cargo run --example particles
//! ```

/// The emission rate, in particles per second.
const RATE: f32 = 120.0;

/// The launch speed, in pixels per second; each particle gets a share of
/// it, in [0.75, 1].
const SPEED: f32 = 260.0;

/// The half-width of the launch cone, in radians around straight up —
/// about 20 degrees each way.
const SPREAD: f32 = 0.35;

/// The lifetime range, in seconds; each particle gets one in between.
const LIFE: (f32, f32) = (0.8, 1.8);

/// The draw size range, in pixels (a radius); each particle gets one in
/// between.
const SIZE: (f32, f32) = (2.0, 5.0);

/// The gravity, in pixels per second, per second, pulling the particles
/// down (y points up, so it is negative).
const GRAVITY: f32 = 420.0;

/// The source's radius, in pixels.
const SOURCE: f32 = 6.0;

/// The z the particles are drawn at, above the source.
const Z: f32 = 1.0;

/// The particle color; the alpha is set per particle from its life.
const COLOR: frost::Color = frost::Color {
    r: 1.0,
    g: 0.55,
    b: 0.2,
    a: 1.0,
};

/// A tiny deterministic random source (splitmix64), so the example needs no
/// external random crate: seeded from the current time, it gives a
/// different fountain on each run.
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self(nanos)
    }

    /// The next uniform value in [0, 1).
    fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u32 << 24) as f32
    }

    /// The next uniform value in [lo, hi].
    fn in_range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.next_f32() * (hi - lo)
    }
}

struct Demo {
    /// The particles: the simulation state, stepped once per frame.
    system: frost::ParticleSystem,
    /// The random source, for the per-particle jitter.
    rng: Rng,
    /// The emission accumulator: `RATE * dt` is added each frame and one
    /// particle is spawned per whole unit, so the rate holds at any dt.
    acc: f32,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // The source sits a little above the bottom edge, at the window's
        // horizontal center; it moves with the window on resize.
        let (_, h) = ctx.size();
        let source = [0.0, -h / 2.0 + 24.0];

        // Spawn at a constant rate: the accumulator carries the fraction.
        self.acc += RATE * dt;
        while self.acc >= 1.0 {
            self.acc -= 1.0;
            // Straight up (pi/2, y points up), jittered within the cone.
            let angle = std::f32::consts::PI / 2.0 + self.rng.in_range(-SPREAD, SPREAD);
            let life = self.rng.in_range(LIFE.0, LIFE.1);
            self.system.spawn(frost::Particle {
                pos: [source[0] + self.rng.in_range(-4.0, 4.0), source[1]],
                vel: [
                    angle.cos() * self.rng.in_range(0.75 * SPEED, SPEED),
                    angle.sin() * self.rng.in_range(0.75 * SPEED, SPEED),
                ],
                life,
                max_life: life,
                size: self.rng.in_range(SIZE.0, SIZE.1),
            });
        }

        // Advance the simulation: gravity down, life out, the dead removed.
        self.system.update(dt, [0.0, -GRAVITY]);

        // Draw each particle: a circle whose alpha is its remaining life
        // fraction, so it fades out as it dies.
        for p in &self.system.particles {
            let fade = (p.life / p.max_life).clamp(0.0, 1.0);
            ctx.circle(
                p.pos[0],
                p.pos[1],
                p.size,
                frost::Color {
                    r: COLOR.r,
                    g: COLOR.g,
                    b: COLOR.b,
                    a: fade,
                },
                Z,
            );
        }
        // The source, behind the particles.
        ctx.circle(source[0], source[1], SOURCE, COLOR, 0.5);

        log::trace!("process: dt {:?} particles {:?}", dt, self.system.len());
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
            system: frost::ParticleSystem::new(),
            rng: Rng::new(),
            acc: 0.0,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
