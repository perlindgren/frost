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
//!   spot, its body peristaltic-beating between the two frames; a worm
//!   moving clearly left is flipped about its center — a negative x
//!   scale — so the sprite faces its travel.
//! - **Burrowing** — once the target is reached, sinking back into the
//!   soil over [SINK_TIME], then starting the next underground delay.
//!
//! At most [MAX_ABOVE] worms are above ground at once: a worm whose delay
//! has expired may emerge only while fewer than [MAX_ABOVE] worms are out
//! of the soil, and keeps waiting — rechecked every frame — until a slot
//! frees up as its worms burrow.
//!
//! The two peristaltic frames (`Worm1_crop.png` and `Worm2_crop.png`) are
//! different pixel sizes; each is scaled to the same rendered width,
//! [WORM_SIZE], so the body length stays constant while the frames'
//! different heights pulse the body — the peristaltic wave.
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
/// width, so the two peristaltic poses — different pixel sizes — keep the
/// same body length while their heights pulse.
const WORM_SIZE: f32 = 60.0;

/// How long a worm takes to emerge out of the soil, in seconds.
const EMERGE_TIME: f32 = 0.6;

/// How long a worm takes to burrow back into the soil, in seconds.
const SINK_TIME: f32 = 0.6;

/// The crawling speed range, in pixels per second.
const SPEED_MIN: f32 = 24.0;
const SPEED_MAX: f32 = 46.0;

/// The peristaltic beat range, in frame flips per second: the body
/// alternates its two frames at this rate while the worm is out of the
/// soil.
const BEAT_MIN: f32 = 3.0;
const BEAT_MAX: f32 = 6.0;

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

/// The facing dead band, in pixels per second: below this horizontal
/// speed a worm keeps its current facing, so the sprite does not flip
/// back and forth as the speed crosses zero on a diagonal crawl.
const FACING_EPS: f32 = 4.0;

/// The target-retry budget: on emergence the worm draws a target at least
/// [MIN_TRAVEL] pixels from its spawn point, redrawing up to this many
/// times before accepting the last draw.
const TARGET_RETRIES: usize = 8;

/// A worm's phase in its life cycle.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Waiting underground, counting down its delay.
    Underground,
    /// Raising out of the soil at its spawn point.
    Emerging,
    /// Crawling in a straight line toward its target.
    Crawling,
    /// Sinking back into the soil at its target.
    Burrowing,
}

/// One worm of the swarm.
#[derive(Clone, Copy)]
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
    /// The last movement, in user-space pixels per second: the sign of
    /// its `x` component sets the facing.
    vel: [f32; 2],
    /// The facing: 1.0 crawling right, -1.0 crawling left; the sprite is
    /// flipped about its center — a negative x scale — while left.
    facing: f32,
    /// The crawling speed, in pixels per second.
    speed: f32,
    /// The peristaltic beat clock, in frame flips: it advances by
    /// `beat_speed * dt` while the worm is out of the soil, and the frame
    /// is its floor modulo 2.
    beat: f32,
    /// The frame the worm currently shows: 0 or 1.
    frame: u8,
    /// The frame last laid out on the worm's node, so the shape swap — a
    /// cheap `Arc` clone — happens only on a flip: [u8::MAX] while the
    /// cache is stale — a fresh worm, a re-emergence, a restore.
    shown: u8,
    /// The frame flip rate, in flips per second.
    beat_speed: f32,
}

impl Worm {
    /// A fresh worm: underground at the pool's dummy position, counting
    /// down its delay — [Worms::new] draws the first one.
    fn fresh() -> Self {
        Worm {
            phase: Phase::Underground,
            phase_t: 0.0,
            delay: 0.0,
            pos: [0.0, 0.0],
            target: [0.0, 0.0],
            vel: [0.0, 0.0],
            facing: 1.0,
            speed: 0.0,
            beat: 0.0,
            frame: 0,
            shown: u8::MAX,
            beat_speed: 0.0,
        }
    }

    /// Crawls the worm toward its target for `dt` seconds: a straight
    /// line at its speed, the facing from the horizontal component of
    /// the motion, and the arrival — the burrow — when the target is
    /// reached.
    fn crawl(&mut self, dt: f32) {
        let to = [self.target[0] - self.pos[0], self.target[1] - self.pos[1]];
        let d = to[0] * to[0] + to[1] * to[1];
        let dist = d.sqrt();
        if dist <= ARRIVE.max(self.speed * dt) {
            // The target is reached — or this step lands past it: the
            // worm arrives exactly on the target and burrows.
            self.pos = self.target;
            self.phase = Phase::Burrowing;
            self.phase_t = 0.0;
            self.vel = [0.0, 0.0];
            return;
        }
        let k = self.speed * dt / dist;
        self.pos = [self.pos[0] + to[0] * k, self.pos[1] + to[1] * k];
        self.vel = [to[0] * self.speed / dist, to[1] * self.speed / dist];
        // The facing, with a dead band, so the sprite does not flip back
        // and forth as the horizontal speed crosses zero on a diagonal
        // crawl.
        if self.vel[0] > FACING_EPS {
            self.facing = 1.0;
        } else if self.vel[0] < -FACING_EPS {
            self.facing = -1.0;
        }
    }
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
    /// width, so the frames keep the same rendered body length.
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
    /// The last movement, in user-space pixels per second.
    pub vel: [f32; 2],
    /// The facing: 1.0 crawling right, -1.0 crawling left.
    pub facing: f32,
    /// The crawling speed, in pixels per second.
    pub speed: f32,
    /// The peristaltic beat clock, in frame flips.
    pub beat: f32,
    /// The frame the worm currently shows: 0 or 1.
    pub frame: u8,
    /// The frame flip rate, in flips per second.
    pub beat_speed: f32,
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
            vel: worm.vel,
            facing: worm.facing,
            speed: worm.speed,
            beat: worm.beat,
            frame: worm.frame,
            beat_speed: worm.beat_speed,
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
            vel: s.vel,
            facing: s.facing,
            speed: s.speed,
            beat: s.beat,
            frame: s.frame,
            shown: u8::MAX,
            beat_speed: s.beat_speed,
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
        Self::build(Rng::new(), frames)
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
        // Each frame is scaled to [WORM_SIZE] pixels wide, so the two
        // peristaltic poses — different pixel sizes — keep the same
        // rendered body length while their heights pulse.
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
                        w.vel = [0.0, 0.0];
                        above -= 1;
                    }
                }
            }
            // The peristaltic body beats while the worm is out of the
            // soil — emerging, crawling, or burrowing — and rests
            // underground.
            if w.phase != Phase::Underground {
                w.beat += w.beat_speed * dt;
                w.frame = ((w.beat as u32) % 2) as u8;
            }
        }
    }

    /// Lays the swarm out on the worms node.
    ///
    /// Each out-of-the-soil slot's transform is the worm's position and
    /// its scale the current frame's uniform scale, with the x component
    /// carrying the facing flip and the y component the emergence and
    /// burrowing progress: 0 to 1 while the worm rises out of the soil,
    /// 1 to 0 while it sinks back in. A worm above ground shows its
    /// current peristaltic frame; a worm underground clears its slot.
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
            let frame_scale = self.scale[w.frame as usize] * s;
            child.transform = frost::Transform::translate(w.pos);
            child.scale = [w.facing * frame_scale, frame_scale];
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

/// Emerges `worm` out of the soil: a random spawn point inside `hull`,
/// a random target at least [MIN_TRAVEL] pixels away, and fresh speed
/// and beat draws from `rng`.
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
    worm.vel = [0.0, 0.0];
    worm.speed = rng.in_range(SPEED_MIN, SPEED_MAX);
    worm.beat_speed = rng.in_range(BEAT_MIN, BEAT_MAX);
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

    /// A worm's facing follows its horizontal travel with a dead band:
    /// crawling left flips the sprite about its center — the laid-out x
    /// scale goes negative — and crawling right brings it back.
    #[test]
    fn the_facing_follows_the_horizontal_travel() {
        let f = frames();
        let mut worms = Worms::with_seed(7, [&f[0], &f[1]]);
        let dt = 0.05;
        // Run the swarm until some worm is crawling leftward, then check
        // its laid-out scale on a fresh node.
        let mut node = node();
        let mut saw_left = false;
        let mut saw_right = false;
        for _ in 0..2000 {
            worms.step(dt, &ANCHORS);
            worms.layout(&mut node, [&f[0], &f[1]]);
            for (w, c) in worms.worms.iter().zip(&node.children) {
                if w.phase == Phase::Crawling {
                    if w.facing < 0.0 {
                        let s = c.scale[0];
                        assert!(s < 0.0, "a leftward worm flips: x scale {s}");
                        saw_left = true;
                    } else {
                        let s = c.scale[0];
                        assert!(s > 0.0, "a rightward worm does not flip: x scale {s}");
                        saw_right = true;
                    }
                }
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
}
