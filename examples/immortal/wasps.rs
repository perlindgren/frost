//! A swarm of thirty wasps — one per plant layer — buzzing around the
//! flower bench, the row of plants on the grass, concurrently with the
//! tools and the plants themselves. Each wasp orbits the segment its
//! layer spawned it — the midpoint of that segment along the static,
//! fully grown chain, the sway left out — at its own radius, speed, and
//! height, breathing its radius and bobbing vertically, so the swarm
//! reads as a loose cluster circling the bench rather than a rigid
//! formation.
//!
//! The swarm is not all in the air at once: each fully grown plant layer
//! spawns one wasp, so the swarm grows layer by layer as the bench
//! grows. Wasp `i` is the `(i % LAYERS)`-th layer's wasp of plant
//! `i / LAYERS`, and it exists only once its own plant's layer has fully
//! grown — the plants grow concurrently, each on its own clock, so each
//! plant's wasps come up independently, each circling its own segment,
//! rather than as one global prefix in swarm order.
//!
//! The sprites are `Waspeye1.png` and `Waspeye2.png`, two frames of a
//! wasp flying to the right. The swarm is driven by a [`Wasps`] value:
//! `new` builds the thirty wasps from a deterministic schedule of the wasp
//! index, so each run looks the same and no random crate is needed,
//! `step` advances the swarm by `dt` seconds around the plants' root
//! joints given the grown-layer count per plant, and `layout` lays the
//! wasps out in a node whose children — in swarm order — are the thirty
//! wasp nodes.
//!
//! A wasp's facing comes from the sign of its velocity's x: the sprite is
//! drawn unflipped while the wasp flies to the right and is flipped about
//! its center — a negative x scale — while it flies to the left, with a
//! small dead band so the flip does not chatter at the orbit's side
//! points, where the horizontal speed crosses zero. The two frames
//! alternate at each wasp's own wingbeat rate, so the swarm flaps out of
//! phase with itself.

/// The swarm size: thirty wasps, one per plant layer — five per plant
/// across the six-plant bench, all in the air once the bench is fully
/// grown.
pub const N: usize = 30;

/// The layers each plant has, in chain order: the swarm has
/// [LAYERS] × six = [N] wasps, and wasp `i` belongs to plant
/// `i / LAYERS`, the plant whose layer spawned it.
pub const LAYERS: usize = 5;

/// The rendered width of a wasp, in pixels; the height follows the
/// sprite's own aspect ratio.
const BEE_SIZE: f32 = 25.0;

/// The golden ratio's fractional part, the deterministic "random"
/// generator: `frac(i * GOLDEN)` spreads the wasps' parameters over their
/// ranges without a random crate.
const GOLDEN: f32 = 0.618_034;

/// The golden angle, in radians: the orbit phase step between
/// neighbours, so no two wasps start at the same point of their orbits.
const GOLDEN_ANGLE: f32 = 2.399_963_1;

/// The orbit's tilt: the orbit is an ellipse, its vertical extent
/// squashed to this fraction of its horizontal extent, like a circle seen
/// from the side.
const TILT: f32 = 0.45;

/// The velocity dead band, in pixels per second: below this, a wasp keeps
/// its current facing, so the sprite does not flip back and forth as the
/// horizontal speed crosses zero at the orbit's side points.
const FACING_EPS: f32 = 4.0;

/// The fractional part of `x`.
fn frac(x: f32) -> f32 {
    x - x.floor()
}

/// One wasp of the swarm.
#[derive(Default)]
struct Wasp {
    /// The wasp's current position in user space, y up.
    pos: [f32; 2],
    /// The wasp's velocity in user-space pixels per second, from the last
    /// step: the sign of its x is the facing.
    vel: [f32; 2],
    /// The facing: `1.0` while the wasp flies to the right, `-1.0` while
    /// it flies to the left; the sprite is flipped about its center for
    /// the left.
    facing: f32,
    /// Whether the wasp has been spawned — its plant layer has fully
    /// grown — and placed at least once: until then its `pos` is a dummy
    /// and its velocity is synthetic, and its slot draws nothing.
    placed: bool,
    /// The wingbeat clock, in beats: it advances by `flap_speed * dt` and
    /// the frame is its integer part modulo 2.
    flap: f32,
    /// The frame the wasp currently shows: 0 or 1.
    frame: u8,
    /// The frame last laid out on the wasp's node, so the shape swap — a
    /// cheap `Arc` clone — happens at most once per wingbeat change.
    shown: u8,
    /// The plant the wasp orbits, in bench order: the plant whose layer
    /// spawned the wasp — `i / LAYERS` — fixed on the wasp's first step.
    home: usize,
    /// The orbit phase, in radians.
    phase: f32,
    /// The orbit's angular speed, in radians per second.
    omega: f32,
    /// The orbit's radius, in pixels, before the wobble.
    radius: f32,
    /// The height, in pixels, of the orbit's center above the midpoint
    /// of the segment that spawned the wasp.
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

/// A swarm of thirty wasps — one per plant layer — buzzing around the
/// flower bench.
#[derive(Default)]
pub struct Wasps {
    /// The thirty wasps, in swarm order: wasp `i` rides child `i` of the
    /// wasps node and is the `(i % LAYERS)`-th layer's wasp of plant
    /// `i / LAYERS`.
    wasps: [Wasp; N],
    /// The swarm's clock, in seconds.
    t: f32,
    /// The uniform scale that fits the sprite's width to `BEE_SIZE`
    /// pixels.
    scale: f32,
}

/// One wasp's snapshot state: every field of the [Wasp] except the shape-
/// swap cache — the frame its node last showed, which [Wasps::layout]
/// re-derives from the wasp's own frame.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WaspState {
    /// The wasp's position, in user space, y up.
    pub pos: [f32; 2],
    /// The wasp's velocity, in user-space pixels per second.
    pub vel: [f32; 2],
    /// The facing: 1.0 flying right, -1.0 flying left.
    pub facing: f32,
    /// Whether the wasp has been placed at least once.
    pub placed: bool,
    /// The wingbeat clock, in beats.
    pub flap: f32,
    /// The frame the wasp currently shows: 0 or 1.
    pub frame: u8,
    /// The plant the wasp orbits, in bench order.
    pub home: usize,
    /// The orbit phase, in radians.
    pub phase: f32,
    /// The orbit's angular speed, in radians per second.
    pub omega: f32,
    /// The orbit's radius, in pixels, before the wobble.
    pub radius: f32,
    /// The orbit center's height above its segment's midpoint.
    pub hover: f32,
    /// The wobble's frequency, in radians per second.
    pub wob: f32,
    /// The bob's amplitude, in pixels.
    pub bob_amp: f32,
    /// The bob's frequency, in radians per second.
    pub bob_freq: f32,
    /// The wingbeat rate, in beats per second.
    pub flap_speed: f32,
}

/// The swarm's snapshot state: its wasps and its clock. The sprite's fit
/// scale is left out: it is derived from the sprite.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct WaspsState {
    /// The thirty wasps, in swarm order.
    pub wasps: Vec<WaspState>,
    /// The swarm's clock, in seconds.
    pub t: f32,
}

impl From<&Wasps> for WaspsState {
    fn from(swarm: &Wasps) -> Self {
        WaspsState {
            wasps: swarm.wasps.iter().map(WaspState::from).collect(),
            t: swarm.t,
        }
    }
}

impl From<&Wasp> for WaspState {
    fn from(wasp: &Wasp) -> Self {
        WaspState {
            pos: wasp.pos,
            vel: wasp.vel,
            facing: wasp.facing,
            placed: wasp.placed,
            flap: wasp.flap,
            frame: wasp.frame,
            home: wasp.home,
            phase: wasp.phase,
            omega: wasp.omega,
            radius: wasp.radius,
            hover: wasp.hover,
            wob: wasp.wob,
            bob_amp: wasp.bob_amp,
            bob_freq: wasp.bob_freq,
            flap_speed: wasp.flap_speed,
        }
    }
}

impl Wasp {
    /// Rebuilds the wasp from its snapshot state: the shape-swap cache
    /// starts stale, so [Wasps::layout] re-syncs the wasp's node on the
    /// next frame.
    fn from_state(s: &WaspState) -> Wasp {
        Wasp {
            pos: s.pos,
            vel: s.vel,
            facing: s.facing,
            placed: s.placed,
            flap: s.flap,
            frame: s.frame,
            shown: u8::MAX,
            home: s.home,
            phase: s.phase,
            omega: s.omega,
            radius: s.radius,
            hover: s.hover,
            wob: s.wob,
            bob_amp: s.bob_amp,
            bob_freq: s.bob_freq,
            flap_speed: s.flap_speed,
        }
    }
}

impl Wasps {
    /// The swarm's snapshot state (see [WaspsState]).
    pub fn state(&self) -> WaspsState {
        WaspsState::from(self)
    }

    /// Restores the swarm from a snapshot: the wasps and the clock — the
    /// sprite's fit scale is kept.
    pub fn restore(&mut self, state: &WaspsState) {
        for (wasp, s) in self.wasps.iter_mut().zip(state.wasps.iter()) {
            *wasp = Wasp::from_state(s);
        }
        self.t = state.t;
    }
}

impl Wasps {
    /// Builds the swarm from the sprite's texture size, with the swarm
    /// clock at zero: the wasps' parameters are a deterministic schedule
    /// of the wasp index through `GOLDEN`, so every run looks the same and
    /// no random crate is needed.
    pub fn new(size: [f32; 2]) -> Self {
        Wasps {
            wasps: std::array::from_fn(|i| {
                let f = frac(i as f32 * GOLDEN);
                let f2 = frac(f * 7.0 + 0.31);
                let f3 = frac(f * 13.0 + 0.71);
                Wasp {
                    facing: 1.0,
                    // Stagger the wingbeats across the swarm.
                    flap: f * 2.0,
                    shown: u8::MAX,
                    phase: i as f32 * GOLDEN_ANGLE,
                    omega: 0.9 + 1.1 * f,
                    radius: 70.0 + 70.0 * f2,
                    hover: 90.0 + 80.0 * f3,
                    wob: 0.7 + 0.8 * f,
                    bob_amp: 6.0 + 8.0 * f2,
                    bob_freq: 1.6 + 1.4 * f3,
                    flap_speed: 5.0 + 4.0 * f,
                    ..Wasp::default()
                }
            }),
            scale: BEE_SIZE / size[0],
            ..Wasps::default()
        }
    }

    /// Advances the swarm by `dt` seconds around `centers`, the orbit
    /// centers in user space: for each plant, in bench order, the
    /// [LAYERS] midpoints of its layer segments along the static, fully
    /// grown chain — the sway left out.
    ///
    /// `grown` is the count of fully grown layers per plant, in bench
    /// order. The swarm is not all in the air at once — each fully grown
    /// layer spawns one wasp, and wasp `i` orbits the midpoint of the
    /// segment that spawned it, `centers[i / LAYERS][i % LAYERS]`. A wasp
    /// exists only when its own plant's layer has fully grown —
    /// `grown[i / LAYERS] > i % LAYERS` — so the plants' wasps come up
    /// independently, each circling its own segment, rather than as one
    /// global prefix in swarm order.
    ///
    /// Each wasp's orbit is an ellipse around the midpoint of the segment
    /// that spawned it, lifted by its `hover` height: the orbit angle
    /// advances at its own `omega`, the radius breathes by ±15 % at its
    /// own wobble rate, and a slow vertical bob rides on top, so no two
    /// wasps trace the same path. The velocity is the difference to the
    /// last position, and the facing follows its x with a dead band, so
    /// the sprite flips only when the wasp is clearly flying one way or
    /// the other.
    pub fn step(&mut self, dt: f32, centers: &[[[f32; 2]; LAYERS]], grown: &[usize]) {
        self.t += dt;
        let t = self.t;
        for (i, wasp) in self.wasps.iter_mut().enumerate() {
            let home = (i / LAYERS) % centers.len();
            let layer = i % LAYERS;
            // The wasp exists only once its own plant's layer has fully
            // grown — not on the swarm's global prefix.
            if grown.get(home).copied().unwrap_or(0) <= layer {
                continue;
            }
            if !wasp.placed {
                wasp.home = home;
            }
            let a = wasp.phase + t * wasp.omega;
            let r = wasp.radius * (0.85 + 0.15 * (t * wasp.wob + 2.0 * wasp.phase).sin());
            let [cx, cy] = centers[home][layer];
            let pos = [
                cx + a.cos() * r,
                cy + wasp.hover
                    + a.sin() * r * TILT
                    + wasp.bob_amp * (t * wasp.bob_freq + 3.0 * wasp.phase).sin(),
            ];
            // The velocity from the last step, or a synthetic rightward
            // one on the very first step, when there is no last step.
            let vel = if wasp.placed && dt > 0.0 {
                [(pos[0] - wasp.pos[0]) / dt, (pos[1] - wasp.pos[1]) / dt]
            } else {
                [wasp.omega * wasp.radius, 0.0]
            };
            if vel[0] < -FACING_EPS {
                wasp.facing = -1.0;
            } else if vel[0] > FACING_EPS {
                wasp.facing = 1.0;
            }
            wasp.flap += dt * wasp.flap_speed;
            wasp.frame = ((wasp.flap as u32) % 2) as u8;
            wasp.pos = pos;
            wasp.vel = vel;
            wasp.placed = true;
        }
    }

    /// Lays the swarm out in `node`, whose children — in swarm order —
    /// are the thirty wasp nodes: each spawned wasp's transform carries it
    /// to its position, its scale fits the sprite to `BEE_SIZE` and flips
    /// it about its center while it flies to the left, and its shape swaps
    /// between the two frames — cheap `Arc` clones — at most once per
    /// wingbeat change. The unspawned slots are cleared, so a wasp that
    /// no layer has spawned yet never draws.
    pub fn layout(&mut self, node: &mut frost::SceneNode, frames: [&frost::Shape; 2]) {
        for (wasp, child) in self.wasps.iter_mut().zip(&mut node.children) {
            if !wasp.placed {
                child.shape = None;
                continue;
            }
            child.transform = frost::Transform::translate(wasp.pos);
            child.scale = [wasp.facing * self.scale, self.scale];
            if wasp.shown != wasp.frame {
                child.shape = Some(frames[wasp.frame as usize].clone());
                wasp.shown = wasp.frame;
            }
        }
    }

    /// Each wasp's facing — +1.0 flying right (the near side of its orbit,
    /// the lower half, where it moves with the screen) and −1.0 flying
    /// left (the far side, the upper half) — for the scene's depth scheme:
    /// a placed wasp's order is its segment's order plus this facing, so the
    /// near half of an orbit rides one unit above its segment and the far
    /// half one below it. The unspawned slots read 0.0; they draw nothing,
    /// so their order is moot.
    pub fn facings(&self) -> [f32; N] {
        std::array::from_fn(|i| {
            if self.wasps[i].placed {
                self.wasps[i].facing
            } else {
                0.0
            }
        })
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
                [
                    ANCHORS[p][0] + k as f32 * 20.0,
                    ANCHORS[p][1] + k as f32 * 20.0,
                ]
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
            filter: frost::SpriteFilter::Linear,
            generation: 0,
        }
    }

    /// The swarm is not all in the air at once: a wasp exists only once
    /// its own plant's layer has fully grown, so stepping with `grown`
    /// places exactly the wasps whose plant's layer count passes their
    /// layer index, and wasp `i` homes to the plant whose layer spawned it
    /// — `i / LAYERS`.
    #[test]
    fn a_grown_layer_spawns_its_own_wasp() {
        let c = centers();
        let mut v = Wasps::new([100.0, 90.0]);

        // No layer is grown yet: nothing steps, nothing is placed.
        v.step(0.05, &c, &[0, 0, 0, 0, 0, 0]);
        assert!(v.wasps.iter().all(|b| !b.placed));

        // Plant 0's first layer finishes: wasp 0 spawns on plant 0.
        v.step(0.05, &c, &[1, 0, 0, 0, 0, 0]);
        assert!(v.wasps[0].placed);
        assert_eq!(v.wasps[0].home, 0);
        assert!(!v.wasps[1].placed);

        // All of plant 0's layers, then plant 1's first: the homes
        // follow the layer order, five per plant.
        v.step(0.05, &c, &[5, 1, 0, 0, 0, 0]);
        for i in 0..6 {
            assert!(v.wasps[i].placed, "wasp {i} was not spawned");
            assert_eq!(v.wasps[i].home, i / LAYERS, "wasp {i} homes wrong");
        }
        assert!(!v.wasps[6].placed, "wasp 6 spawned early");

        // The bench is fully grown: all thirty are in the air, and an
        // over-supplied count cannot place a ghost.
        v.step(0.05, &c, &[5, 5, 5, 5, 5, 5]);
        for (i, wasp) in v.wasps.iter().enumerate() {
            assert!(wasp.placed, "wasp {i} never spawned");
            assert_eq!(wasp.home, i / LAYERS, "wasp {i} homes wrong");
        }
    }

    /// A wasp exists on its own plant's schedule, not the swarm's global
    /// prefix: a later plant's grown layers spawn its own wasps even when
    /// an earlier plant has none, and each wasp circles its own segment.
    #[test]
    fn wasps_spawn_on_their_own_plants() {
        let c = centers();
        let mut v = Wasps::new([100.0, 90.0]);

        // Plant 2 has two layers grown, but plant 0 and 1 have none:
        // only plant 2's wasps exist — the global prefix (wasps 0-1)
        // must NOT spawn.
        v.step(0.05, &c, &[0, 0, 2, 0, 0, 0]);
        assert!(
            !v.wasps[0].placed,
            "wasp 0 (plant 0) spawned on a global prefix"
        );
        assert!(
            !v.wasps[1].placed,
            "wasp 1 (plant 0) spawned on a global prefix"
        );
        assert!(
            !v.wasps[5].placed,
            "wasp 5 (plant 1) spawned on a global prefix"
        );
        assert!(v.wasps[10].placed, "wasp 10 (plant 2 layer 0) missing");
        assert_eq!(v.wasps[10].home, 2, "wasp 10 homes to plant 2");
        assert!(v.wasps[11].placed, "wasp 11 (plant 2 layer 1) missing");
        assert_eq!(v.wasps[11].home, 2, "wasp 11 homes to plant 2");
        assert!(
            !v.wasps[12].placed,
            "wasp 12 (plant 2 layer 2) spawned early"
        );
    }

    /// A spawned wasp circles the segment that spawned it: its position
    /// stays within its breathing radius of that segment's midpoint, and
    /// at the midpoint's height plus the hover — not at the home plant's
    /// root joint.
    #[test]
    fn a_wasp_orbits_its_own_segment() {
        let c = centers();
        let mut v = Wasps::new([100.0, 90.0]);
        v.step(0.05, &c, &[5, 5, 5, 5, 5, 5]);
        for (i, wasp) in v.wasps.iter().enumerate() {
            let [cx, cy] = c[i / LAYERS][i % LAYERS];
            // The radius breathes by ±15 %, so the horizontal distance
            // from the orbit center never exceeds the nominal radius.
            let dx = (wasp.pos[0] - cx).abs();
            assert!(
                dx <= wasp.radius + 1e-3,
                "wasp {i} drifted {dx} px sideways from its segment's midpoint"
            );
            // And the vertical distance from the midpoint is the hover
            // plus at most the tilted radius and the bob.
            let dy = (wasp.pos[1] - (cy + wasp.hover)).abs();
            assert!(
                dy <= wasp.radius * TILT + wasp.bob_amp + 1e-3,
                "wasp {i} drifted {dy} px vertically from its segment's midpoint"
            );
        }
    }

    /// `layout` draws only the spawned prefix: unspawned slots stay
    /// shapeless, so a wasp that no layer has spawned yet never shows.
    #[test]
    fn layout_keeps_unspawned_slots_empty() {
        let f = frame();
        let mut v = Wasps::new([100.0, 90.0]);
        let mut node = frost::SceneNode {
            children: (0..N)
                .map(|_| Box::new(frost::SceneNode::default()))
                .collect(),
            ..Default::default()
        };

        v.step(0.05, &centers(), &[3, 0, 0, 0, 0, 0]);
        v.layout(&mut node, [&f, &f]);
        for i in 0..3 {
            assert!(node.children[i].shape.is_some(), "wasp {i} is invisible");
        }
        for (i, child) in node.children.iter().enumerate().skip(3) {
            assert!(child.shape.is_none(), "unspawned wasp {i} drew");
        }
    }
}
