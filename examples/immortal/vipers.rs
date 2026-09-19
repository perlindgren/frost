//! A swarm of ten vipers buzzing around the flower bench — the row of
//! plants on the grass — concurrently with the tools and the plants
//! themselves. Each viper orbits the root joint of its home plant at its
//! own radius, speed, and height, breathing its radius and bobbing
//! vertically, so the swarm reads as a loose cluster circling the bench
//! rather than a rigid formation.
//!
//! The sprites are `Getingeye1.png` and `Getingeye2.png`, two frames of a
//! viper flying to the right. The swarm is driven by a [`Vipers`] value:
//! `new` builds the ten bees from a deterministic schedule of the bee
//! index, so each run looks the same and no random crate is needed,
//! `step` advances the swarm by `dt` seconds around the plants' root
//! joints, and `layout` lays the bees out in a node whose children — in
//! swarm order — are the ten bee nodes.
//!
//! A bee's facing comes from the sign of its velocity's x: the sprite is
//! drawn unflipped while the bee flies to the right and is flipped about
//! its center — a negative x scale — while it flies to the left, with a
//! small dead band so the flip does not chatter at the orbit's side
//! points, where the horizontal speed crosses zero. The two frames
//! alternate at each bee's own wingbeat rate, so the swarm flaps out of
//! phase with itself.

/// The swarm size: ten bees.
pub const N: usize = 10;

/// The rendered width of a bee, in pixels; the height follows the
/// sprite's own aspect ratio.
const BEE_SIZE: f32 = 25.0;

/// The golden ratio's fractional part, the deterministic "random"
/// generator: `frac(i * GOLDEN)` spreads the bees' parameters over their
/// ranges without a random crate.
const GOLDEN: f32 = 0.618_034;

/// The golden angle, in radians: the orbit phase step between
/// neighbours, so no two bees start at the same point of their orbits.
const GOLDEN_ANGLE: f32 = 2.399_963_1;

/// The orbit's tilt: the orbit is an ellipse, its vertical extent
/// squashed to this fraction of its horizontal extent, like a circle seen
/// from the side.
const TILT: f32 = 0.45;

/// The velocity dead band, in pixels per second: below this, a bee keeps
/// its current facing, so the sprite does not flip back and forth as the
/// horizontal speed crosses zero at the orbit's side points.
const FACING_EPS: f32 = 4.0;

/// The fractional part of `x`.
fn frac(x: f32) -> f32 {
    x - x.floor()
}

/// One bee of the swarm.
struct Bee {
    /// The bee's current position in user space, y up.
    pos: [f32; 2],
    /// The bee's velocity in user-space pixels per second, from the last
    /// step: the sign of its x is the facing.
    vel: [f32; 2],
    /// The facing: `1.0` while the bee flies to the right, `-1.0` while
    /// it flies to the left; the sprite is flipped about its center for
    /// the left.
    facing: f32,
    /// Whether the bee has been placed at least once: until then its
    /// `pos` is a dummy and its velocity is synthetic.
    placed: bool,
    /// The wingbeat clock, in beats: it advances by `flap_speed * dt` and
    /// the frame is its integer part modulo 2.
    flap: f32,
    /// The frame the bee currently shows: 0 or 1.
    frame: u8,
    /// The frame last laid out on the bee's node, so the shape swap — a
    /// cheap `Arc` clone — happens at most once per wingbeat change.
    shown: u8,
    /// The plant the bee orbits, in anchor order; fixed on the first
    /// step, when the anchor count is known.
    home: usize,
    /// The orbit phase, in radians.
    phase: f32,
    /// The orbit's angular speed, in radians per second.
    omega: f32,
    /// The orbit's radius, in pixels, before the wobble.
    radius: f32,
    /// The height, in pixels, of the orbit's center above the home
    /// plant's root joint.
    hover: f32,
    /// The wobble's frequency, in radians per second: the radius breathes
    /// by ±15 % at this rate.
    wob: f32,
    /// The bob's amplitude, in pixels.
    bob_amp: f32,
    /// The bob's frequency, in radians per second.
    bob_freq: f32,
    /// The wingbeat rate, in beats per second.
    flap_speed: f32,
}

/// A swarm of ten vipers buzzing around the flower bench.
pub struct Vipers {
    /// The ten bees, in swarm order: bee `i` rides child `i` of the
    /// vipers node.
    bees: [Bee; N],
    /// The swarm's clock, in seconds.
    t: f32,
    /// The uniform scale that fits the sprite's width to `BEE_SIZE`
    /// pixels.
    scale: f32,
}

impl Vipers {
    /// Builds the swarm from the sprite's texture size, with the swarm
    /// clock at zero: the bees' parameters are a deterministic schedule
    /// of the bee index through `GOLDEN`, so every run looks the same and
    /// no random crate is needed.
    pub fn new(size: [f32; 2]) -> Self {
        Vipers {
            bees: std::array::from_fn(|i| {
                let f = frac(i as f32 * GOLDEN);
                let f2 = frac(f * 7.0 + 0.31);
                let f3 = frac(f * 13.0 + 0.71);
                Bee {
                    pos: [0.0, 0.0],
                    vel: [0.0, 0.0],
                    facing: 1.0,
                    placed: false,
                    // Stagger the wingbeats across the swarm.
                    flap: f * 2.0,
                    frame: 0,
                    shown: u8::MAX,
                    home: 0,
                    phase: i as f32 * GOLDEN_ANGLE,
                    omega: 0.9 + 1.1 * f,
                    radius: 70.0 + 70.0 * f2,
                    hover: 90.0 + 80.0 * f3,
                    wob: 0.7 + 0.8 * f,
                    bob_amp: 6.0 + 8.0 * f2,
                    bob_freq: 1.6 + 1.4 * f3,
                    flap_speed: 5.0 + 4.0 * f,
                }
            }),
            t: 0.0,
            scale: BEE_SIZE / size[0],
        }
    }

    /// Advances the swarm by `dt` seconds around `anchors`, the plants'
    /// root joints in user space, in bench order: bee `i` orbits anchor
    /// `i % anchors.len()`.
    ///
    /// Each bee's orbit is an ellipse around its home plant's root joint,
    /// lifted by its `hover` height: the orbit angle advances at its own
    /// `omega`, the radius breathes by ±15 % at its own wobble rate, and
    /// a slow vertical bob rides on top, so no two bees trace the same
    /// path. The velocity is the difference to the last position, and the
    /// facing follows its x with a dead band, so the sprite flips only
    /// when the bee is clearly flying one way or the other.
    pub fn step(&mut self, dt: f32, anchors: &[[f32; 2]]) {
        self.t += dt;
        let t = self.t;
        for (i, bee) in self.bees.iter_mut().enumerate() {
            if !bee.placed {
                bee.home = i % anchors.len();
            }
            let a = bee.phase + t * bee.omega;
            let r = bee.radius * (0.85 + 0.15 * (t * bee.wob + 2.0 * bee.phase).sin());
            let [ax, ay] = anchors[bee.home];
            let pos = [
                ax + a.cos() * r,
                ay + bee.hover
                    + a.sin() * r * TILT
                    + bee.bob_amp * (t * bee.bob_freq + 3.0 * bee.phase).sin(),
            ];
            // The velocity from the last step, or a synthetic rightward
            // one on the very first step, when there is no last step.
            let vel = if bee.placed && dt > 0.0 {
                [(pos[0] - bee.pos[0]) / dt, (pos[1] - bee.pos[1]) / dt]
            } else {
                [bee.omega * bee.radius, 0.0]
            };
            if vel[0] < -FACING_EPS {
                bee.facing = -1.0;
            } else if vel[0] > FACING_EPS {
                bee.facing = 1.0;
            }
            bee.flap += dt * bee.flap_speed;
            bee.frame = ((bee.flap as u32) % 2) as u8;
            bee.pos = pos;
            bee.vel = vel;
            bee.placed = true;
        }
    }

    /// Lays the swarm out in `node`, whose children — in swarm order —
    /// are the ten bee nodes: each bee's transform carries it to its
    /// position, its scale fits the sprite to `BEE_SIZE` and flips it
    /// about its center while it flies to the left, and its shape swaps
    /// between the two frames — cheap `Arc` clones — at most once per
    /// wingbeat change.
    pub fn layout(&mut self, node: &mut frost::SceneNode, frames: [&frost::Shape; 2]) {
        for (bee, child) in self.bees.iter_mut().zip(&mut node.children) {
            child.transform = frost::Transform::translate(bee.pos[0], bee.pos[1]);
            child.scale = [bee.facing * self.scale, self.scale];
            if bee.shown != bee.frame {
                child.shape = Some(frames[bee.frame as usize].clone());
                bee.shown = bee.frame;
            }
        }
    }
}
