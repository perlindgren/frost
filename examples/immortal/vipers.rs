//! A swarm of thirty vipers — one per plant layer — buzzing around the
//! flower bench, the row of plants on the grass, concurrently with the
//! tools and the plants themselves. Each viper orbits the segment its
//! layer spawned it — the midpoint of that segment along the static,
//! fully grown chain, the sway left out — at its own radius, speed, and
//! height, breathing its radius and bobbing vertically, so the swarm
//! reads as a loose cluster circling the bench rather than a rigid
//! formation.
//!
//! The swarm is not all in the air at once: each fully grown plant layer
//! spawns one viper, so the swarm grows layer by layer as the bench
//! grows. Viper `i` is the `(i % LAYERS)`-th layer's viper of plant
//! `i / LAYERS` — the plants grow one at a time, each layer in turn, so
//! the layers' completion order is the swarm order — and only the
//! spawned prefix of the swarm steps and draws.
//!
//! The sprites are `Getingeye1.png` and `Getingeye2.png`, two frames of a
//! viper flying to the right. The swarm is driven by a [`Vipers`] value:
//! `new` builds the thirty bees from a deterministic schedule of the bee
//! index, so each run looks the same and no random crate is needed,
//! `step` advances the swarm by `dt` seconds around the plants' root
//! joints given how many layers have fully grown, and `layout` lays the
//! bees out in a node whose children — in swarm order — are the thirty
//! bee nodes.
//!
//! A bee's facing comes from the sign of its velocity's x: the sprite is
//! drawn unflipped while the bee flies to the right and is flipped about
//! its center — a negative x scale — while it flies to the left, with a
//! small dead band so the flip does not chatter at the orbit's side
//! points, where the horizontal speed crosses zero. The two frames
//! alternate at each bee's own wingbeat rate, so the swarm flaps out of
//! phase with itself.

/// The swarm size: thirty bees, one per plant layer — five per plant
/// across the six-plant bench, all in the air once the bench is fully
/// grown.
pub const N: usize = 30;

/// The layers each plant has, in chain order: the swarm has
/// [LAYERS] × six = [N] vipers, and viper `i` belongs to plant
/// `i / LAYERS`, the plant whose layer spawned it.
pub const LAYERS: usize = 5;

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
    /// Whether the bee has been spawned — its plant layer has fully
    /// grown — and placed at least once: until then its `pos` is a dummy
    /// and its velocity is synthetic, and its slot draws nothing.
    placed: bool,
    /// The wingbeat clock, in beats: it advances by `flap_speed * dt` and
    /// the frame is its integer part modulo 2.
    flap: f32,
    /// The frame the bee currently shows: 0 or 1.
    frame: u8,
    /// The frame last laid out on the bee's node, so the shape swap — a
    /// cheap `Arc` clone — happens at most once per wingbeat change.
    shown: u8,
    /// The plant the bee orbits, in bench order: the plant whose layer
    /// spawned the bee — `i / LAYERS` — fixed on the bee's first step.
    home: usize,
    /// The orbit phase, in radians.
    phase: f32,
    /// The orbit's angular speed, in radians per second.
    omega: f32,
    /// The orbit's radius, in pixels, before the wobble.
    radius: f32,
    /// The height, in pixels, of the orbit's center above the midpoint
    /// of the segment that spawned the bee.
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

/// A swarm of thirty vipers — one per plant layer — buzzing around the
/// flower bench.
pub struct Vipers {
    /// The thirty bees, in swarm order: bee `i` rides child `i` of the
    /// vipers node and is the `(i % LAYERS)`-th layer's viper of plant
    /// `i / LAYERS`.
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

    /// Advances the swarm by `dt` seconds around `centers`, the orbit
    /// centers in user space: for each plant, in bench order, the
    /// [LAYERS] midpoints of its layer segments along the static, fully
    /// grown chain — the sway left out.
    ///
    /// `spawned` is how many of the swarm's vipers exist: the number of
    /// fully grown plant layers on the bench. The swarm is not all in the
    /// air at once — each fully grown layer spawns one viper, and viper
    /// `i` orbits the midpoint of the segment that spawned it,
    /// `centers[i / LAYERS][i % LAYERS]` — so only the spawned prefix
    /// steps; the rest keeps waiting in the grass.
    ///
    /// Each bee's orbit is an ellipse around the midpoint of the segment
    /// that spawned it, lifted by its `hover` height: the orbit angle
    /// advances at its own `omega`, the radius breathes by ±15 % at its
    /// own wobble rate, and a slow vertical bob rides on top, so no two
    /// bees trace the same path. The velocity is the difference to the
    /// last position, and the facing follows its x with a dead band, so
    /// the sprite flips only when the bee is clearly flying one way or
    /// the other.
    pub fn step(&mut self, dt: f32, centers: &[[[f32; 2]; LAYERS]], spawned: usize) {
        self.t += dt;
        let t = self.t;
        for (i, bee) in self.bees.iter_mut().enumerate().take(spawned) {
            if !bee.placed {
                bee.home = (i / LAYERS) % centers.len();
            }
            let a = bee.phase + t * bee.omega;
            let r = bee.radius * (0.85 + 0.15 * (t * bee.wob + 2.0 * bee.phase).sin());
            let [cx, cy] = centers[bee.home][i % LAYERS];
            let pos = [
                cx + a.cos() * r,
                cy + bee.hover
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
    /// are the thirty bee nodes: each spawned bee's transform carries it
    /// to its position, its scale fits the sprite to `BEE_SIZE` and flips
    /// it about its center while it flies to the left, and its shape swaps
    /// between the two frames — cheap `Arc` clones — at most once per
    /// wingbeat change. The unspawned slots are cleared, so a viper that
    /// no layer has spawned yet never draws.
    pub fn layout(&mut self, node: &mut frost::SceneNode, frames: [&frost::Shape; 2]) {
        for (bee, child) in self.bees.iter_mut().zip(&mut node.children) {
            if !bee.placed {
                child.shape = None;
                continue;
            }
            child.transform = frost::Transform::translate(bee.pos[0], bee.pos[1]);
            child.scale = [bee.facing * self.scale, self.scale];
            if bee.shown != bee.frame {
                child.shape = Some(frames[bee.frame as usize].clone());
                bee.shown = bee.frame;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The six plant roots in `grass.png`'s pixel space (`y` down), in
    /// growth order — the same points `main.rs` anchors the plants to.
    const ANCHORS: [[f32; 2]; 6] = [
        [923.0, 514.0],
        [1248.0, 546.0],
        [1633.0, 603.0],
        [739.0, 571.0],
        [1081.0, 640.0],
        [1463.0, 719.0],
    ];

    /// The orbit centers for the test anchors: each plant's [LAYERS]
    /// layer-segment midpoints, the k-th offset by `(20k, 20k)` from its
    /// root joint — the values are arbitrary, the tests only check
    /// spawning, homing, and where the orbits sit.
    fn centers() -> [[[f32; 2]; LAYERS]; 6] {
        std::array::from_fn(|p| {
            std::array::from_fn(|k| {
                [ANCHORS[p][0] + k as f32 * 20.0, ANCHORS[p][1] + k as f32 * 20.0]
            })
        })
    }

    /// A one-pixel sprite, so the layout tests have a shape to swap.
    fn frame() -> frost::Shape {
        frost::Shape::Sprite {
            data: std::sync::Arc::new([0u8; 1]),
            width: 1,
            height: 1,
            color: frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            alpha: 1.0,
        }
    }

    /// The swarm is not all in the air at once: a viper exists only once
    /// its plant layer has fully grown, so stepping with `spawned` places
    /// exactly the first `spawned` bees, and bee `i` homes to the plant
    /// whose layer spawned it — `i / LAYERS`.
    #[test]
    fn a_grown_layer_spawns_its_own_viper() {
        let c = centers();
        let mut v = Vipers::new([100.0, 90.0]);

        // No layer is grown yet: nothing steps, nothing is placed.
        v.step(0.05, &c, 0);
        assert!(v.bees.iter().all(|b| !b.placed));

        // Plant 0's first layer finishes: viper 0 spawns on plant 0.
        v.step(0.05, &c, 1);
        assert!(v.bees[0].placed);
        assert_eq!(v.bees[0].home, 0);
        assert!(!v.bees[1].placed);

        // All of plant 0's layers, then plant 1's first: the homes
        // follow the layer order, five per plant.
        v.step(0.05, &c, 6);
        for i in 0..6 {
            assert!(v.bees[i].placed, "bee {i} was not spawned");
            assert_eq!(v.bees[i].home, i / LAYERS, "bee {i} homes wrong");
        }
        assert!(!v.bees[6].placed, "bee 6 spawned early");

        // The bench is fully grown: all thirty are in the air, and an
        // over-supplied count cannot place a ghost.
        v.step(0.05, &c, N + 1);
        for (i, bee) in v.bees.iter().enumerate() {
            assert!(bee.placed, "bee {i} never spawned");
            assert_eq!(bee.home, i / LAYERS, "bee {i} homes wrong");
        }
    }

    /// A spawned viper circles the segment that spawned it: its position
    /// stays within its breathing radius of that segment's midpoint, and
    /// at the midpoint's height plus the hover — not at the home plant's
    /// root joint.
    #[test]
    fn a_viper_orbits_its_own_segment() {
        let c = centers();
        let mut v = Vipers::new([100.0, 90.0]);
        v.step(0.05, &c, N);
        for (i, bee) in v.bees.iter().enumerate() {
            let [cx, cy] = c[i / LAYERS][i % LAYERS];
            // The radius breathes by ±15 %, so the horizontal distance
            // from the orbit center never exceeds the nominal radius.
            let dx = (bee.pos[0] - cx).abs();
            assert!(
                dx <= bee.radius + 1e-3,
                "bee {i} drifted {dx} px sideways from its segment's midpoint"
            );
            // And the vertical distance from the midpoint is the hover
            // plus at most the tilted radius and the bob.
            let dy = (bee.pos[1] - (cy + bee.hover)).abs();
            assert!(
                dy <= bee.radius * TILT + bee.bob_amp + 1e-3,
                "bee {i} drifted {dy} px vertically from its segment's midpoint"
            );
        }
    }

    /// `layout` draws only the spawned prefix: unspawned slots stay
    /// shapeless, so a viper that no layer has spawned yet never shows.
    #[test]
    fn layout_keeps_unspawned_slots_empty() {
        let f = frame();
        let mut v = Vipers::new([100.0, 90.0]);
        let mut node = frost::SceneNode {
            children: (0..N)
                .map(|_| Box::new(frost::SceneNode::default()))
                .collect(),
            ..Default::default()
        };

        v.step(0.05, &centers(), 3);
        v.layout(&mut node, [&f, &f]);
        for i in 0..3 {
            assert!(node.children[i].shape.is_some(), "bee {i} is invisible");
        }
        for (i, child) in node.children.iter().enumerate().skip(3) {
            assert!(child.shape.is_none(), "unspawned bee {i} drew");
        }
    }
}
