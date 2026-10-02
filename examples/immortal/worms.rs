//! Worms: a swarm of peristaltic worms crawling over the soil under the
//! flower bench, emerging from the ground, crossing the bed, and sinking
//! back in.
//!
//! The swarm holds [N] slots — a fixed pool, one per child of the worms
//! node — and each worm cycles through four phases:
//!
//! - **Underground** — invisible, counting down a random delay.
//! - **Emerging** — rising out of a random point of the convex hull the
//!   plants' root anchors span (the rombus of the bed), the node's y
//!   scale stretching from 0 to 1 over [EMERGE_TIME].
//! - **Crawling** — walking in a straight line toward another random
//!   point of the same hull, at least [MIN_TRAVEL] pixels from the spawn
//!   spot. The sprite's axis aims at 0 degrees, so the node's transform
//!   rotates the body onto the bearing drawn at emergence — the direction
//!   toward the target. The walk itself is the `worm` example's stride:
//!   the body's x scale breathes on a cosine between [STRETCH_SHORT] and
//!   [STRETCH_LONG] — gathered, reaching, gathered — and the worm gains
//!   ground only while it contracts. The rear grips the soil through the
//!   stretch half of each stride while the front reaches; through the
//!   contract half the front holds and the rear slides up to it, exactly
//!   [STRIDE_LEN] per stride, with a speed that is continuous and zero
//!   at every grip — the worm surges and settles once per stride, at the
//!   stride rate its [Worm::speed] divides out of [STRIDE_LEN].
//! - **Burrowing** — once the target is reached, sinking back into the
//!   soil over [SINK_TIME], then starting the next underground delay.
//!
//! At most [MAX_ABOVE] worms are above ground at once: a worm whose delay
//! has expired may emerge only while fewer than [MAX_ABOVE] worms are out
//! of the soil, and keeps waiting — rechecked every frame — until a slot
//! frees up as its worms burrow.
//!
//! The two peristaltic frames (`Worm1_crop.png`, the arched pose, and
//! `Worm2_crop.png`, the stretched pose) are different pixel sizes; each
//! is scaled to a rendered body length of [WORM_SIZE], and the frame
//! follows the stride's half: the stretched pose reaches through the
//! stretch, the arched pose gathers through the contract. Since both
//! frames normalize to the same neutral length, the cosine breath — not
//! the frame swap — sets the body's actual length continuously, so
//! neither pose flip pops it; the frames' different heights pulse its
//! thickness.
//!
//! Spawn and target points come from the swarm's own
//! [`frost::Rng`] stream (see [Worms::set_seed]), sampled uniformly in
//! area from the hull with [`bugs::convex_hull`] and
//! [`bugs::sample_polygon`] — the same soil the bugs live in.

use frost::Rng;

use crate::bugs;

/// The slot pool: [MAX_ABOVE] above-ground worms plus a few in the soil,
/// so the emergences run continuously instead of in waves.
pub const N: usize = 14;

/// The population cap: at most this many worms above ground at once.
pub const MAX_ABOVE: usize = 10;

/// The rendered worm width, in pixels: each frame is scaled to this
/// width, the body's neutral length, before the peristaltic beat's
/// [STRETCH_SHORT]/[STRETCH_LONG] x stretch.
const WORM_SIZE: f32 = 60.0;

/// How long a worm takes to emerge out of the soil, in seconds.
const EMERGE_TIME: f32 = 0.6;

/// How long a worm takes to burrow back into the soil, in seconds.
const SINK_TIME: f32 = 0.6;

/// The crawling speed range, in pixels per second, averaged over whole
/// strides: the gait's stride rate falls out as speed / [STRIDE_LEN].
const SPEED_MIN: f32 = 8.0;
const SPEED_MAX: f32 = 15.0;

/// The underground delay range, in seconds: the wait between one
/// burrowing and the next emergence, drawn per worm on each way in.
const DELAY_MIN: f32 = 2.0;
const DELAY_MAX: f32 = 8.0;

/// The minimum distance, in pixels, between a worm's spawn point and the
/// target it draws on emergence: a worm that comes out of the soil
/// should actually travel, not burrow on the spot.
const MIN_TRAVEL: f32 = 120.0;

/// How close, in pixels, a worm must get to its target to count as
/// arrived — and the step's overshoot bound: a step that would land past
/// the target lands exactly on it.
const ARRIVE: f32 = 2.0;

/// The breath's floor: how short the body hangs at each stride's start,
/// gathered and gripping — the arched frame's pose.
const STRETCH_SHORT: f32 = 0.85;

/// The breath's ceiling: how far the body reaches at each stride's
/// middle — the stretched frame's pose.
const STRETCH_LONG: f32 = 1.15;

/// The breath's amplitude: half the reach between [STRETCH_SHORT] and
/// [STRETCH_LONG]. The x scale rides one cosine between these extremes
/// per stride — contracted at the stride's start, stretched at its
/// middle.
const AMP: f32 = (STRETCH_LONG - STRETCH_SHORT) / 2.0;

/// Ground gained per stride: the rear slides exactly the difference
/// between the stretched and the contracted body length while the
/// contract runs — the identity the `worm` example walks on.
const STRIDE_LEN: f32 = WORM_SIZE * (STRETCH_LONG - STRETCH_SHORT);

/// The target-retry budget: on emergence the worm draws a target at least
/// [MIN_TRAVEL] pixels from its spawn point, redrawing up to this many
/// times before accepting the last draw.
const TARGET_RETRIES: usize = 8;

/// A worm's phase in its life cycle.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    /// Waiting underground, counting down its delay.
    #[default]
    Underground,
    /// Raising out of the soil at its spawn point.
    Emerging,
    /// Crawling in a straight line toward its target.
    Crawling,
    /// Sinking back into the soil at its target.
    Burrowing,
}

/// One worm of the swarm.
#[derive(Clone, Copy, Default)]
struct Worm {
    /// The current life phase.
    phase: Phase,
    /// The time in the current phase, in seconds: the delay's progress
    /// while underground, the emergence or burrowing's progress while the
    /// worm is in the soil's surface.
    phase_t: f32,
    /// The full underground delay, in seconds, drawn on the worm's way
    /// back into the soil.
    delay: f32,
    /// The current position, in user space, `y` up — the soil surface the
    /// worm crawls on.
    pos: [f32; 2],
    /// The target position, in user space, `y` up, drawn on emergence.
    target: [f32; 2],
    /// The crawling bearing, in radians, `y` up: the direction from the
    /// spawn point to the target, drawn on emergence; the sprite's axis
    /// — which aims at 0 degrees — rotates onto it, and the worm holds it
    /// until it burrows.
    heading: f32,
    /// The crawling speed, in pixels per second, averaged over whole
    /// strides: the gait's stride rate is speed / [STRIDE_LEN].
    speed: f32,
    /// The gait's phase, in strides — one per full stretch-contract
    /// cycle. [`stride_advance`] turns it into ground gained, and the
    /// body's breath and the frame both ride its half.
    beat: f32,
    /// The frame the worm currently shows: 0 or 1, derived from the
    /// stride's half.
    frame: u8,
    /// The frame last laid out on the worm's node, so the shape swap — a
    /// cheap `Arc` clone — happens only on a flip: [u8::MAX] while the
    /// cache is stale — a fresh worm, a re-emergence, a restore.
    shown: u8,
}

impl Worm {
    /// A fresh worm: underground at the pool's dummy position, counting
    /// down its delay — [Worms::new] draws the first one.
    fn fresh() -> Self {
        Worm {
            shown: u8::MAX,
            ..Worm::default()
        }
    }

    /// Crawls the worm toward its target for `dt` seconds with the
    /// peristaltic gait: the stride phase advances at the rate the
    /// worm's speed divides out of [STRIDE_LEN], and the ground gained is
    /// exactly what [`stride_advance`] says the phase has bought — the
    /// worm holds while it stretches and surges while it contracts,
    /// averaging its speed over whole strides. Landing past the target
    /// lands exactly on it and burrows.
    fn crawl(&mut self, dt: f32) {
        let before = stride_advance(self.beat);
        self.beat += (self.speed / STRIDE_LEN) * dt;
        let step = stride_advance(self.beat) - before;
        let to = [self.target[0] - self.pos[0], self.target[1] - self.pos[1]];
        let dist = (to[0] * to[0] + to[1] * to[1]).sqrt();
        if dist <= ARRIVE.max(step) {
            // The target is reached — or this surge lands past it: the
            // worm arrives exactly on the target and burrows.
            self.pos = self.target;
            self.phase = Phase::Burrowing;
            self.phase_t = 0.0;
            return;
        }
        let (sin, cos) = self.heading.sin_cos();
        self.pos = [self.pos[0] + step * cos, self.pos[1] + step * sin];
    }
}

/// The ground the gait has gained by phase `p`, in strides — the `worm`
/// example's peristaltic stride: the rear grips the soil through each
/// stretch half (nothing gained while the front reaches out) and slides
/// up to the held front end through each contract half, exactly
/// [STRIDE_LEN] per stride. The slide follows the breath's cosine, so
/// the speed is continuous and zero at every grip.
fn stride_advance(p: f32) -> f32 {
    let k = p.floor();
    let q = p - k;
    let breath = 1.0 - AMP * (std::f32::consts::TAU * q).cos();
    let slid = if q < 0.5 {
        0.0
    } else {
        (STRETCH_LONG - breath) / (STRETCH_LONG - STRETCH_SHORT)
    };
    STRIDE_LEN * (k + slid)
}

/// The swarm: its [N] worms, its clock, its per-frame scales, and its
/// spawn/target randomizer.
pub struct Worms {
    /// The [N] worms, in pool order: worm `i` rides child `i` of the
    /// worms node.
    worms: [Worm; N],
    /// The swarm clock, in seconds.
    t: f32,
    /// Each frame's uniform scale: [WORM_SIZE] over the frame's pixel
    /// width — the body's neutral length, the base the beat's stretch
    /// scales from.
    scale: [f32; 2],
    /// The spawn and target randomizer.
    rng: Rng,
}

/// One worm's snapshot state: every field of the [Worm] except the shape-
/// swap cache — the frame its node last showed, which [Worms::layout]
/// re-derives from the worm's own frame.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct WormState {
    /// The current life phase.
    pub phase: Phase,
    /// The time in the current phase, in seconds.
    pub phase_t: f32,
    /// The full underground delay, in seconds.
    pub delay: f32,
    /// The current position, in user space.
    pub pos: [f32; 2],
    /// The target position, in user space.
    pub target: [f32; 2],
    /// The crawling bearing, in radians, `y` up: the direction from the
    /// spawn point to the target, drawn on emergence.
    pub heading: f32,
    /// The crawling speed, in pixels per second.
    pub speed: f32,
    /// The gait phase at save time, in strides.
    pub beat: f32,
    /// The frame the worm currently shows: 0 or 1.
    pub frame: u8,
}

/// The swarm's snapshot state: its worms, its clock, and its randomizer's
/// state — the stream resumes exactly where it left off. The per-frame
/// scales are left out: they are derived from the frames.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct WormsState {
    /// The [N] worms, in pool order.
    pub worms: Vec<WormState>,
    /// The swarm clock, in seconds.
    pub t: f32,
    /// The spawn/target randomizer's state.
    pub seed: u64,
}

impl From<&Worm> for WormState {
    fn from(worm: &Worm) -> Self {
        WormState {
            phase: worm.phase,
            phase_t: worm.phase_t,
            delay: worm.delay,
            pos: worm.pos,
            target: worm.target,
            heading: worm.heading,
            speed: worm.speed,
            beat: worm.beat,
            frame: worm.frame,
        }
    }
}

impl From<&WormState> for Worm {
    fn from(s: &WormState) -> Self {
        Worm {
            phase: s.phase,
            phase_t: s.phase_t,
            delay: s.delay,
            pos: s.pos,
            target: s.target,
            heading: s.heading,
            speed: s.speed,
            beat: s.beat,
            frame: s.frame,
            shown: u8::MAX,
        }
    }
}

impl Worms {
    /// Builds the swarm over the two peristaltic frames.
    ///
    /// Every worm starts underground, and its first delay is drawn from
    /// the swarm's clock-seeded randomizer, staggered across the pool —
    /// no wave of ten worms at the first second.
    pub fn new(frames: [&frost::Shape; 2]) -> Self {
        Self::build(Rng::default(), frames)
    }

    /// Builds the swarm like [Worms::new], but from a seeded randomizer —
    /// the tests' reproducible stream.
    #[cfg(test)]
    fn with_seed(seed: u64, frames: [&frost::Shape; 2]) -> Self {
        Self::build(Rng::with_seed(seed), frames)
    }

    /// The constructor's shared body: the per-frame scales and the
    /// staggered first delays, from the given randomizer.
    fn build(rng: Rng, frames: [&frost::Shape; 2]) -> Self {
        // Each frame is scaled to [WORM_SIZE] pixels wide — the body's
        // neutral length, the base the peristaltic beat's stretch scales
        // from as the frames flip.
        let scale = [
            WORM_SIZE
                / frames[0]
                    .sprite_size()
                    .expect("the first frame is a sprite")[0],
            WORM_SIZE
                / frames[1]
                    .sprite_size()
                    .expect("the second frame is a sprite")[0],
        ];
        let mut rng = rng;
        let worms = std::array::from_fn(|_| {
            let mut w = Worm::fresh();
            w.delay = rng.in_range(DELAY_MIN, DELAY_MAX);
            w
        });
        Self {
            worms,
            t: 0.0,
            scale,
            rng,
        }
    }

    /// The swarm's snapshot state (see [WormsState]).
    pub fn state(&self) -> WormsState {
        WormsState {
            worms: self.worms.iter().map(WormState::from).collect(),
            t: self.t,
            seed: self.rng.state(),
        }
    }

    /// Seeds the swarm's spawn/target randomizer from `seed`, so the run
    /// is reproducible (`--seed`).
    pub fn set_seed(&mut self, seed: u64) {
        self.rng.set_state(seed);
    }

    /// Restores the swarm from a snapshot: the worms, the clock, and the
    /// randomizer's state. The per-frame scales are kept: they are
    /// derived from the frames.
    pub fn restore(&mut self, state: &WormsState) {
        self.worms = std::array::from_fn(|i| (&state.worms[i]).into());
        self.t = state.t;
        self.rng.set_state(state.seed);
    }

    /// Steps the swarm by `dt` seconds.
    ///
    /// `anchors` are the plants' root positions in user space, in plant
    /// order: the convex hull of the six anchors is the soil the worms
    /// live in, and every emergence — its spawn point and its target —
    /// samples that hull. Each worm steps its phase in pool order, and a
    /// worm whose delay has expired emerges only while fewer than
    /// [MAX_ABOVE] worms are out of the soil — the cap checked against a
    /// running count as the step's own emergences and burrows land — so
    /// the population cap holds no matter how the delays overlap.
    pub fn step(&mut self, dt: f32, anchors: &[[f32; 2]]) {
        self.t += dt;
        let hull = bugs::convex_hull(anchors);
        // The running count of the worms out of the soil: it starts at
        // the step's opening count and moves with each emergence and
        // burrow, so the cap holds within the step, not just between
        // steps.
        let mut above = self
            .worms
            .iter()
            .filter(|w| w.phase != Phase::Underground)
            .count();
        for w in &mut self.worms {
            w.phase_t += dt;
            match w.phase {
                Phase::Underground => {
                    // The delay runs out: emerge — but only while the
                    // population cap has room, so at most [MAX_ABOVE]
                    // worms are out of the soil at once. A worm that
                    // cannot emerge simply keeps waiting; the cap frees
                    // as its worms burrow.
                    if w.phase_t >= w.delay && above < MAX_ABOVE {
                        emerge(w, &mut self.rng, &hull);
                        above += 1;
                    }
                }
                Phase::Emerging => {
                    if w.phase_t >= EMERGE_TIME {
                        w.phase = Phase::Crawling;
                        w.phase_t = 0.0;
                    }
                }
                Phase::Crawling => {
                    w.crawl(dt);
                }
                Phase::Burrowing => {
                    if w.phase_t >= SINK_TIME {
                        w.phase = Phase::Underground;
                        w.phase_t = 0.0;
                        w.delay = self.rng.in_range(DELAY_MIN, DELAY_MAX);
                        above -= 1;
                    }
                }
            }
            // The gait's clock: the stride phase advances at the rate
            // the speed divides out of [STRIDE_LEN] while the worm is
            // out of the soil — emerging, crawling, or burrowing — and
            // rests underground; the crawl has already advanced it (and
            // moved the body with it). The frame rides the stride's
            // half: the stretched pose reaches through the stretch, the
            // arched pose gathers through the contract.
            if w.phase != Phase::Underground {
                if w.phase != Phase::Crawling {
                    w.beat += (w.speed / STRIDE_LEN) * dt;
                }
                w.frame = if w.beat.fract() > 0.0 && w.beat.fract() < 0.5 {
                    1
                } else {
                    0
                };
            }
        }
    }

    /// Lays the swarm out on the worms node.
    ///
    /// Each out-of-the-soil slot's transform rotates the sprite — its
    /// axis aims at 0 degrees — onto the worm's [Worm::heading] and
    /// anchors the body's trailing end, the sprite's negative x end, on
    /// the worm's position — the grip the gait slides the body forward
    /// from. The body spans its full length along the heading, its x
    /// scale breathing between [STRETCH_SHORT] and [STRETCH_LONG] across
    /// the stride, while both scales carry the emergence and burrowing
    /// progress, 0 to 1 while the worm rises out of the soil, 1 to 0
    /// while it sinks back in. A worm above ground shows its current peristaltic frame;
    /// a worm underground clears its slot.
    pub fn layout(&mut self, node: &mut frost::SceneNode, frames: [&frost::Shape; 2]) {
        for (w, child) in self.worms.iter_mut().zip(&mut node.children) {
            if w.phase == Phase::Underground {
                // In the soil: nothing to draw.
                child.shape = None;
                continue;
            }
            let s = match w.phase {
                Phase::Emerging => w.phase_t / EMERGE_TIME,
                Phase::Burrowing => 1.0 - w.phase_t / SINK_TIME,
                // Crawling, and the emerging/burrowing arms at full
                // progress: the full body.
                Phase::Crawling => 1.0,
                Phase::Underground => unreachable!(),
            };
            // The gait's breath: the body's x scale rides the stride's
            // cosine between [STRETCH_SHORT] and [STRETCH_LONG] — the
            // very curve [`stride_advance`] integrates — while the y
            // scale is the frame's own: the body's thickness pulse; both
            // shrink with the emergence and burrowing progress `s`.
            let frame_scale = self.scale[w.frame as usize];
            let breath = 1.0 - AMP * (std::f32::consts::TAU * w.beat.fract()).cos();
            child.scale = [frame_scale * s * breath, frame_scale * s];
            // The body's trailing end holds on the worm's position while
            // its leading end pumps along the heading: the sprite's axis
            // aims at 0 degrees, so the transform rotates the body onto
            // the [Worm::heading] and shifts it so the sprite's negative
            // x end — half the body's length behind its center — lands
            // exactly on the position. In the left half-plane the body
            // is additionally flipped about its axis, so the arch's back
            // stays up; the flip leaves the x axis untouched, so the
            // anchor holds in both branches.
            let (sin, cos) = w.heading.sin_cos();
            let half = WORM_SIZE * 0.5 * s * breath;
            let t = [w.pos[0] + half * cos, w.pos[1] + half * sin];
            let rot = frost::Transform::rotate(w.heading);
            let lin = if cos >= 0.0 {
                rot
            } else {
                frost::Transform::scale([1.0, -1.0]).compose(&rot)
            };
            child.transform = lin.compose(&frost::Transform::translate(t));
            // The depth: the worm rides the ground band, interleaved with
            // the bugs, the other worms, and the plants' root slices by its
            // y — its trailing end, the anchor the body pumps from.
            child.order = crate::zorder::ground(w.pos[1]);
            if w.shown != w.frame {
                child.shape = Some(frames[w.frame as usize].clone());
                w.shown = w.frame;
            }
        }
        // Any children past the pool — the scene keeps none — clear.
        for child in node.children.iter_mut().skip(self.worms.len()) {
            child.shape = None;
        }
    }
}

/// Emerges `worm` out of the soil: a random spawn point inside `hull`, a
/// random target at least [MIN_TRAVEL] pixels away, the heading from the
/// spawn to the target, and a fresh speed draw from `rng`.
///
/// A free function — not a [Worms] method — so it can draw from the
/// swarm's randomizer while the swarm's worm pool is borrowed by the
/// step's loop.
fn emerge(worm: &mut Worm, rng: &mut Rng, hull: &[[f32; 2]]) {
    let spawn = bugs::sample_polygon(hull, rng);
    // A target at least [MIN_TRAVEL] away, so the worm actually travels:
    // redraw a few times and accept the last draw.
    let mut target = bugs::sample_polygon(hull, rng);
    for _ in 0..TARGET_RETRIES {
        if dist(spawn, target) >= MIN_TRAVEL {
            break;
        }
        target = bugs::sample_polygon(hull, rng);
    }
    worm.pos = spawn;
    worm.target = target;
    // The bearing toward the target: the sprite's axis aims at 0
    // degrees, so the layout rotates the body onto this direction.
    worm.heading = (target[1] - spawn[1]).atan2(target[0] - spawn[0]);
    worm.speed = rng.in_range(SPEED_MIN, SPEED_MAX);
    worm.beat = 0.0;
    worm.frame = 0;
    // The slot was cleared while the worm was underground: the shape
    // swap cache is stale, so the layout re-sets the shape.
    worm.shown = u8::MAX;
    worm.phase = Phase::Emerging;
    worm.phase_t = 0.0;
}

/// The distance between two points, in user space.
fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The six plant roots in `grass.png`'s pixel space (`y` down), in
    /// growth order — the same points `main.rs` anchors the plants to:
    /// the tests run the swarm on the raw points, since the soil is the
    /// hull either way and the hull's shape is independent of the
    /// coordinate mapping.
    const ANCHORS: [[f32; 2]; 6] = [
        [923.0, 514.0],
        [1248.0, 546.0],
        [1633.0, 603.0],
        [739.0, 571.0],
        [1081.0, 640.0],
        [1463.0, 719.0],
    ];

    /// Whether `p` lies on the inside (or on the boundary) of the convex
    /// ring `hull`, in whatever orientation the ring happens to have —
    /// the same test the bugs' sampler uses, with the same 1e-3 px
    /// boundary tolerance for f32 noise.
    fn inside(hull: &[[f32; 2]], p: [f32; 2]) -> bool {
        let mut signed = 0.0f32;
        for w in 0..hull.len() {
            let a = hull[w];
            let b = hull[(w + 1) % hull.len()];
            signed += a[0] * b[1] - a[1] * b[0];
        }
        let ccw = signed > 0.0;
        for w in 0..hull.len() {
            let a = hull[w];
            let b = hull[(w + 1) % hull.len()];
            let dx = b[0] - a[0];
            let dy = b[1] - a[1];
            let cross = dx * (p[1] - a[1]) - dy * (p[0] - a[0]);
            let dist = cross / (dx * dx + dy * dy).sqrt();
            if dist.abs() > 1e-3 && (dist > 0.0) != ccw {
                return false;
            }
        }
        true
    }

    /// The test frames: two 10-pixel sprites, the second narrower, so the
    /// per-frame scales differ — the real frames' situation, miniature.
    fn frames() -> [frost::Shape; 2] {
        [
            frost::Shape::Sprite {
                data: std::sync::Arc::new([0u8; 1]),
                width: 10,
                height: 8,
                color: frost::Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                alpha: 1.0,
                filter: frost::SpriteFilter::Linear,
                generation: 0,
            },
            frost::Shape::Sprite {
                data: std::sync::Arc::new([1u8; 1]),
                width: 12,
                height: 7,
                color: frost::Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                alpha: 1.0,
                filter: frost::SpriteFilter::Linear,
                generation: 0,
            },
        ]
    }

    /// A node the swarm lays out on: [N] shape-less children, in pool
    /// order.
    fn node() -> frost::SceneNode {
        frost::SceneNode {
            children: (0..N)
                .map(|_| Box::new(frost::SceneNode::default()))
                .collect(),
            ..Default::default()
        }
    }

    /// A fresh swarm is all underground: every slot underground, every
    /// delay in range, and the layout leaves every slot shape-less.
    #[test]
    fn a_fresh_swarm_is_all_underground_and_shapeless() {
        let f = frames();
        let mut worms = Worms::new([&f[0], &f[1]]);
        assert!(
            worms.worms.iter().all(|w| w.phase == Phase::Underground),
            "a fresh worm must start underground"
        );
        assert!(
            worms
                .worms
                .iter()
                .all(|w| w.delay >= DELAY_MIN && w.delay < DELAY_MAX),
            "the first delays must fall in range"
        );
        let mut node = node();
        worms.layout(&mut node, [&f[0], &f[1]]);
        assert!(
            node.children.iter().all(|c| c.shape.is_none()),
            "underground worms draw nothing"
        );
    }

    /// Over a long run the population cap holds: no matter how the
    /// delays overlap, at most [MAX_ABOVE] worms are above ground after
    /// any step.
    #[test]
    fn the_population_cap_holds_over_a_long_run() {
        let f = frames();
        let mut worms = Worms::new([&f[0], &f[1]]);
        let dt = 0.05;
        for _ in 0..4000 {
            worms.step(dt, &ANCHORS);
            let above = worms
                .worms
                .iter()
                .filter(|w| w.phase != Phase::Underground)
                .count();
            assert!(above <= MAX_ABOVE, "the cap holds: {above} <= {MAX_ABOVE}");
        }
    }

    /// Every emergence samples its spawn and target from inside the soil
    /// hull: over a long run, every position a worm ever holds is inside
    /// the hull of the anchors.
    #[test]
    fn every_spawn_and_target_lands_inside_the_hull() {
        let f = frames();
        let mut worms = Worms::with_seed(0xC0FFEE, [&f[0], &f[1]]);
        let hull = bugs::convex_hull(&ANCHORS);
        let dt = 0.05;
        for _ in 0..2000 {
            worms.step(dt, &ANCHORS);
            for w in &worms.worms {
                if w.phase != Phase::Underground {
                    // The target is fixed from emergence to burrowing —
                    // while the worm is out — so both endpoints of the
                    // crawl must sit in the soil.
                    assert!(
                        inside(&hull, w.pos),
                        "the worm at {:?} is outside the soil hull",
                        w.pos
                    );
                    assert!(
                        inside(&hull, w.target),
                        "the target {:?} is outside the soil hull",
                        w.target
                    );
                }
            }
        }
    }

    /// A worm's heading rotates its body onto its travel: the laid-out
    /// transform aims the sprite's axis — which points at 0 degrees —
    /// from the position at the target, the body's trailing end holds on
    /// the position, and the x scale rides the gait's breath.
    #[test]
    fn the_heading_rotates_the_body_toward_the_target() {
        let f = frames();
        let mut worms = Worms::with_seed(7, [&f[0], &f[1]]);
        let dt = 0.05;
        // Run the swarm until it has crawled both ways, checking every
        // crawler's laid-out pose on a fresh node.
        let mut node = node();
        let mut saw_left = false;
        let mut saw_right = false;
        for _ in 0..2000 {
            worms.step(dt, &ANCHORS);
            worms.layout(&mut node, [&f[0], &f[1]]);
            for (w, c) in worms.worms.iter().zip(&node.children) {
                if w.phase != Phase::Crawling {
                    continue;
                }
                // The body's axis: the transform's image of the sprite's
                // x axis — from the origin's image to (1, 0)'s image.
                let o = c.transform.apply([0.0, 0.0]);
                let e = c.transform.apply([1.0, 0.0]);
                let axis = [e[0] - o[0], e[1] - o[1]];
                let to = [w.target[0] - w.pos[0], w.target[1] - w.pos[1]];
                let d = (to[0] * to[0] + to[1] * to[1]).sqrt();
                assert!(
                    (axis[0] - to[0] / d).abs() < 1e-3 && (axis[1] - to[1] / d).abs() < 1e-3,
                    "the body axis {axis:?} aims off the target: to {to:?}"
                );
                if to[0] < 0.0 {
                    saw_left = true;
                } else {
                    saw_right = true;
                }
                // The gait's breath: the y scale is the frame's neutral
                // scale — the full body while crawling — and the x scale
                // the stride's breath times it.
                let base = worms.scale[w.frame as usize];
                let breath = 1.0 - AMP * (std::f32::consts::TAU * w.beat.fract()).cos();
                assert!(
                    (c.scale[0] - base * breath).abs() < 1e-4,
                    "the x scale {} is not the frame scale {base} times its breath {breath}",
                    c.scale[0]
                );
                assert!(
                    (c.scale[1] - base).abs() < 1e-4,
                    "the y scale {} is not the frame scale {base}",
                    c.scale[1]
                );
                // The body's trailing end — the sprite's negative x end,
                // half the body's length behind its center — holds on
                // the position.
                let half = WORM_SIZE * 0.5 * breath;
                let tail = c.transform.apply([-half, 0.0]);
                assert!(
                    (tail[0] - w.pos[0]).abs() < 1e-3 && (tail[1] - w.pos[1]).abs() < 1e-3,
                    "the trailing end {:?} holds off the position {:?}",
                    tail,
                    w.pos
                );
            }
            if saw_left && saw_right {
                break;
            }
        }
        assert!(saw_left, "a long run must produce leftward travel");
        assert!(saw_right, "a long run must produce rightward travel");
    }

    /// The emergence's y scale rises 0 to 1 over [EMERGE_TIME] and the
    /// burrowing's falls 1 to 0 over [SINK_TIME]: the worm grows out of
    /// the soil and shrinks back into it.
    #[test]
    fn the_emergence_and_burrowing_scales_sweep() {
        let f = frames();
        let mut worms = Worms::with_seed(42, [&f[0], &f[1]]);
        let dt = 0.05;
        let mut node = node();
        let mut saw_emerge = false;
        let mut saw_burrow = false;
        for _ in 0..4000 {
            worms.step(dt, &ANCHORS);
            worms.layout(&mut node, [&f[0], &f[1]]);
            for (w, c) in worms.worms.iter().zip(&node.children) {
                match w.phase {
                    Phase::Emerging => {
                        // Mid-emergence, the y scale is a strict fraction
                        // of the frame scale — between 0 and the full
                        // body.
                        let s = w.phase_t / EMERGE_TIME;
                        if s > 0.0 && s < 1.0 {
                            let full = worms.scale[w.frame as usize];
                            let y = c.scale[1];
                            assert!(y > 0.0 && y < full, "mid-emergence y scale {y}");
                            saw_emerge = true;
                        }
                    }
                    Phase::Burrowing => {
                        if w.phase_t > 0.0 {
                            let full = worms.scale[w.frame as usize];
                            let y = c.scale[1];
                            assert!(y > 0.0 && y < full, "mid-burrow y scale {y}");
                            saw_burrow = true;
                        }
                    }
                    _ => {}
                }
            }
            if saw_emerge && saw_burrow {
                break;
            }
        }
        assert!(saw_emerge, "a long run must pass through mid-emergence");
        assert!(saw_burrow, "a long run must pass through mid-burrowing");
    }

    /// The state round trip: a capture, a perturbation, and a restore
    /// leave the swarm exactly where the capture had it — worms, clock,
    /// and the randomizer's stream.
    #[test]
    fn the_state_round_trips() {
        let f = frames();
        let mut worms = Worms::with_seed(1234, [&f[0], &f[1]]);
        let dt = 0.05;
        for _ in 0..300 {
            worms.step(dt, &ANCHORS);
        }
        let snapshot = worms.state();
        // Perturb the clock and the random stream.
        for _ in 0..50 {
            worms.step(dt, &ANCHORS);
        }
        worms.set_seed(0xBEEF);
        worms.restore(&snapshot);
        assert_eq!(
            worms.state(),
            snapshot,
            "the restore reverts the perturbation"
        );
        // The stream resumes exactly: the next draws agree with a swarm
        // that never left.
        let mut twin = Worms::with_seed(1234, [&f[0], &f[1]]);
        for _ in 0..300 {
            twin.step(dt, &ANCHORS);
        }
        for _ in 0..20 {
            twin.step(dt, &ANCHORS);
        }
        assert_eq!(worms.state().seed, twin.state().seed, "the stream resumes");
    }

    /// The `worm` example's stride, as a curve: ground is gained only
    /// while the body contracts, and the speed is continuous — zero at
    /// every grip.
    #[test]
    fn the_gait_holds_while_reaching_and_surges_while_gathering() {
        // The stretch half buys nothing: the rear grips.
        assert_eq!(stride_advance(0.0), 0.0);
        assert_eq!(stride_advance(0.25), 0.0);
        assert!(
            stride_advance(0.5).abs() < 1e-4,
            "mid-stride, at full reach, the rear has not yet moved"
        );
        // The contract half slides the rear up to the held front end.
        assert!(
            (stride_advance(0.75) - 0.5 * STRIDE_LEN).abs() < 1e-3,
            "the contract's mid-slide is half a stride"
        );
        assert!(
            (stride_advance(1.0) - STRIDE_LEN).abs() < 1e-3,
            "one stride gains one STRIDE_LEN"
        );
        // Monotone, and linear over whole strides.
        let mut prev = 0.0;
        let mut p = 0.0;
        while p <= 6.0 {
            let a = stride_advance(p);
            assert!(a >= prev - 1e-4, "the gait never steps back");
            prev = a;
            p += 0.02;
        }
        assert!((stride_advance(6.0) - 6.0 * STRIDE_LEN).abs() < 1e-2);
        // Zero speed at the grips: the slide follows the breath's sine,
        // flat at the stride's ends.
        let eps = 1e-3;
        let eases_in = (stride_advance(0.5 + eps) - stride_advance(0.5)) / eps;
        let eases_out = (stride_advance(1.0) - stride_advance(1.0 - eps)) / eps;
        assert!(
            eases_in < 0.05 * STRIDE_LEN && eases_out < 0.05 * STRIDE_LEN,
            "the surge must begin and end at zero speed: {eases_in}, {eases_out}"
        );
    }

    /// A crawling worm advances by the gait, not by the clock: nothing
    /// while it stretches, the whole stride once the contract has run.
    #[test]
    fn a_crawling_worm_walks_the_gait_not_a_slide() {
        let mut w = Worm {
            phase: Phase::Crawling,
            pos: [0.0, 0.0],
            target: [400.0, 0.0],
            heading: 0.0,
            // Exactly one stride per second.
            speed: STRIDE_LEN,
            ..Worm::default()
        };
        // The stretch half, in four steps: no ground.
        for _ in 0..4 {
            w.crawl(0.125);
        }
        assert!(
            w.pos[0].abs() < 1e-3 && w.pos[1].abs() < 1e-3,
            "the rear held while the front reached: {:?}",
            w.pos
        );
        assert_eq!(w.phase, Phase::Crawling);
        // The contract half: the same wall clock now surges.
        w.crawl(0.5);
        assert!(
            (w.pos[0] - STRIDE_LEN).abs() < 1e-2 && w.pos[1].abs() < 1e-2,
            "the contract slid the rear up to the front: {:?}",
            w.pos
        );
        // Over whole strides the gait averages to the worm's speed.
        w.crawl(3.0);
        assert!(
            (w.pos[0] - 4.0 * STRIDE_LEN).abs() < 0.1,
            "four strides at one a second: the speed averages out, got {}",
            w.pos[0]
        );
    }
}
