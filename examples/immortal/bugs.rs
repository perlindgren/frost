//! Bugs: a swarm of small dark-red critters that pop up out of the grass
//! and waddle to the plant they were born for.
//!
//! Every time a plant starts growing, three new bugs spawn at random points
//! inside the convex hull of the six plant root positions, standing
//! up over [GROW_TIME] via the y component of their node scale, then swarm
//! walk (straight line plus a sinusoidal perpendicular wobble) to a park
//! spot around that plant's root, where they sway in place. Each bug is
//! animated over three walk frames and is flipped about its center when it
//! is clearly moving left; the sprite tint comes from the node's `modulate`.
//!
//! A bug that reaches its destination while another bug is on the grass
//! within [TJATTER_RANGE] of it chatters: [Bugs::step] reports the index
//! of one of the three tjatter clips (low, mid, high) for that frame, and
//! the demo plays it through [`frost::Audio`].
//!
//! A bug takes [HITS_TO_KILL] hits from the spray can's green mist before
//! it dies — a wounded bug takes its next hit only after a
//! [HIT_COOLDOWN] second, so the mist wears it down one hit at a time —
//! and a row of pips above the bug counts the hits it can still take
//! ([Bugs::health_pips]). A bug that has reached its park spot recovers
//! one hit of health every [REGEN_TIME] seconds, up to [MAX_HEALTH], so a
//! bug left in peace gets tougher the longer it stays. The killing blow
//! starts the two-phase death,
//! driven by the per-bug death clock [Bug::dying]: over [BOUNCE_TIME] it
//! bounces up off the grass on the parabolic arc of [bounce_lift] while
//! its node's y scale sweeps from upright to fully inverted about the
//! sprite center, position, growth, facing and walk frame frozen, so it
//! lands on its back; and then, over [SINK_TIME], the inverted sprite
//! evaporates, shrinking to nothing while its center sinks through the
//! grass. When the clock reaches [BOUNCE_TIME] plus [SINK_TIME] the bug
//! waits out a random delay in [RESPAWN_MIN]…[RESPAWN_MAX] and pops back
//! up at its original spawn spot, fully healed.
//!
//! Bugs are spawned into a fixed-size pool: the scene carries one
//! shape-less child per slot, and [Bugs::layout] drives only the spawned
//! prefix, so unspawned slots never draw. Each plant receives its batch
//! exactly once, in plant order, regardless of how many bugs survive, so
//! the population climbs 3, 6, …, 18 over the first 75 seconds; a killed
//! bug is gone from the grass for a few seconds, so the population dips
//! and recovers with the spraying. Like the vipers module, no random
//! crate is needed — a small splitmix64 [Rng] seeded from the clock
//! decides spawn points, speeds, wobbles and respawn delays.

use frost::SceneNode;

/// The three walk frames, in order.
///
/// The scene carries one default child per slot and the bug state is
/// stepped and laid out against that pool by [Bugs::layout].
pub const BUGS_PER_PLANT: usize = 3;

/// Rendered bug width in user-space px (the bees are 25 px wide).
const BUG_SIZE: f32 = 40.0;
/// How long the y-scale spawn growth takes, in seconds.
const GROW_TIME: f32 = 3.0;
/// Distance from the park target at which a bug stops walking, in px.
const ARRIVE: f32 = 4.0;
/// How close another bug must be to an arriving bug's destination for the
/// arrival to trigger a tjatter, in px.
///
/// A plant's three park spots sit `BUG_SIZE * 0.9` apart (about 36 px)
/// with up to 10 px of combined jitter, so this covers every sibling
/// spot while the next-nearest sibling stays at least about 62 px away.
const TJATTER_RANGE: f32 = 50.0;
/// Walking-speed dead band for the facing flip, in px/s.
///
/// It is above the parked sway's top speed (≈5.2) and below the slowest
/// walk (30), so parked bugs hold their facing while walking ones flip.
const FACING_EPS: f32 = 6.0;
/// The sprite tint: the node `modulate` multiplied over the walk frames.
const TINT: frost::Color = frost::Color {
    r: 0.8,
    g: 0.2,
    b: 0.2,
    a: 1.0,
};
/// How long a sprayed bug's death bounce takes, in seconds: it pops up
/// off the grass, flips upside down in the air, and lands on its back.
const BOUNCE_TIME: f32 = 0.5;
/// How high the bug's center rises at the bounce's peak, in px.
const BOUNCE_HEIGHT: f32 = 50.0;
/// How long the landed, inverted bug takes to evaporate — shrink to
/// nothing while its center sinks through the grass — in seconds.
const SINK_TIME: f32 = 0.5;
/// How many hits from the spray's mist a bug takes before it dies: the
/// pips the bug's health counter starts with. A parked bug can climb it
/// back by one every [REGEN_TIME] seconds, up to [MAX_HEALTH].
pub const HITS_TO_KILL: u8 = 3;
/// The health cap: a parked bug's pips top out at this many, no matter
/// how long it has sat at its destination.
const MAX_HEALTH: u8 = 6;
/// How often a bug parked at its destination recovers one hit of health,
/// in seconds.
const REGEN_TIME: f32 = 5.0;
/// How long a wounded bug is immune to further mist hits, in seconds: a
/// bug standing in the mist loses one hit per cooldown, so the mist wears
/// it down over several cooldowns instead of a few frames.
const HIT_COOLDOWN: f32 = 0.2;
/// How far above the bug its health pips ride, in px.
const PIP_GAP: f32 = 6.0;
/// The dead bug's respawn window, in seconds: it pops back up at its
/// spawn spot after a random delay in [RESPAWN_MIN, RESPAWN_MAX].
const RESPAWN_MIN: f32 = 5.0;
/// Upper end of the respawn window (see [RESPAWN_MIN]).
const RESPAWN_MAX: f32 = 10.0;

/// One bug in the swarm.
struct Bug {
    /// Current position in user space (the sprite center).
    pos: [f32; 2],
    /// Last movement, used for the facing dead band.
    vel: [f32; 2],
    /// Facing: +1 walking right, -1 walking left (the flip axis).
    facing: f32,
    /// True once [Bug::pos] has been used as a movement baseline.
    placed: bool,
    /// The plant this bug walks to.
    home: usize,
    /// Fixed offset from the home root where the bug parks.
    idle: [f32; 2],
    /// Walk clock in steps; the frame is `(walk as u32) % 3`.
    walk: f32,
    /// The frame index [Bug::walk] currently lands on.
    frame: u8,
    /// The frame last written into the node; the shape is swapped when the
    /// two differ, or when the slot still holds another frame's sprite.
    shown: u8,
    /// Seconds since the spawn; the y-scale grows until [GROW_TIME].
    grow: f32,
    /// Walking speed in px/s.
    speed: f32,
    /// Perpendicular wobble amplitude in px.
    wob_amp: f32,
    /// Perpendicular wobble frequency in rad/s.
    wob_freq: f32,
    /// Perpendicular wobble phase offset in rad.
    wob_phase: f32,
    /// Walk-frame rate in steps/s while moving.
    step_rate: f32,
    /// The death clock in seconds, once the killing hit landed: `None`
    /// while alive, `Some(t)` while dying. Under [BOUNCE_TIME] the bug is
    /// mid-bounce — riding the [bounce_lift] arc off the grass, flipping
    /// over in the air (position frozen); from [BOUNCE_TIME] on it has
    /// landed on its back and evaporates, sinking through the grass; at
    /// [BOUNCE_TIME] + [SINK_TIME] it enters its respawn delay.
    dying: Option<f32>,
    /// Hits from the mist still standing between this bug and death;
    /// starts at [HITS_TO_KILL], each hit takes it down by one, and a
    /// bug parked at its destination climbs it back by one every
    /// [REGEN_TIME] seconds, capped at [MAX_HEALTH]. The pips above the
    /// bug's head count it.
    hits: u8,
    /// Seconds parked at the destination: while it is above [REGEN_TIME]
    /// the bug has earned another hit of health (see [Bug::hits]).
    parked: f32,
    /// Whether the bug has already reached its destination this life:
    /// set on the frame it first parks, so the arrival tjatter (see
    /// [Bugs::step]) fires exactly once per bug — also for a bug that
    /// parks from its very first grown frame.
    arrived: bool,
    /// How much of the [HIT_COOLDOWN] after its last hit is left: while
    /// it is above zero no drop can wound the bug again, no matter how
    /// many touch it at once.
    hit_cooldown: f32,
    /// Seconds left before the dead bug pops back up; `Some` while the
    /// bug is gone from the grass.
    respawn: Option<f32>,
    /// The spot the bug popped up from; where it returns after death.
    spawn: [f32; 2],
}

/// The whole swarm: the spawned prefix of the scene's bug slot pool.
pub struct Bugs {
    /// The spawned bugs, in spawn order.
    bugs: Vec<Bug>,
    /// How many plants have received their batch of bugs; the population
    /// cap follows from it (BUGS_PER_PLANT times the plant count, the
    /// scene's slot count).
    plants_done: usize,
    /// Swarm clock in seconds (wobble phase source).
    t: f32,
    /// Final uniform sprite scale (BUG_SIZE over the widest frame).
    scale: f32,
    /// Half the rendered sprite height, so parked feet rest on the grass.
    ground: f32,
    /// Spawn/wander randomizer.
    rng: Rng,
}

impl Bugs {
    /// Build an empty swarm over the three walk frames.
    ///
    /// The swarm is batch-gated by the plants, not by a population cap:
    /// each plant receives its [BUGS_PER_PLANT] bugs exactly once (so the
    /// scene's slot pool — BUGS_PER_PLANT times the plant count — is never
    /// oversubscribed). A killed bug is not replaced by its plant's
    /// batch: it pops back up at its own spawn spot after its respawn
    /// delay.
    pub fn new(frames: [&frost::Shape; 3]) -> Self {
        let (mut w, mut h) = (0.0f32, 0.0f32);
        for frame in frames {
            let [fw, fh] = sprite_size(frame);
            w = w.max(fw);
            h = h.max(fh);
        }
        let scale = BUG_SIZE / w;
        Self {
            bugs: Vec::new(),
            plants_done: 0,
            t: 0.0,
            scale,
            ground: (scale * h) / 2.0,
            rng: Rng::new(),
        }
    }

    /// Advance the swarm by `dt` seconds.
    ///
    /// `anchors` are the plant root positions in user space (in plant
    /// order) and `active` is how many of those plants have started
    /// growing; each newly active plant receives its batch of
    /// [BUGS_PER_PLANT] bugs at points inside the hull of all the roots.
    /// Batches are counted per plant rather than by population, so a
    /// spray-killed bug never triggers a replacement batch. Dying bugs
    /// tick their [Bug::dying] clock — bouncing up off the grass and
    /// flipping over in the air while they bounce, evaporating through
    /// the grass once they have landed on their back — and enter their
    /// respawn delay when the clock reaches [BOUNCE_TIME] + [SINK_TIME];
    /// bugs in the delay count it down and, when it ends, pop back up at
    /// their spawn spot as fresh, fully healed bugs. Each bug's
    /// [HIT_COOLDOWN] runs down here, so a wounded bug takes its next
    /// hit only after the cooldown has elapsed, and a bug parked at its
    /// destination recovers one hit of health every [REGEN_TIME] seconds,
    /// capped at [MAX_HEALTH] ([Bug::parked]).
    ///
    /// Returns the indices of the tjatter clips to play this frame, one
    /// per bug that reached its destination this frame while another bug
    /// was on the grass within [TJATTER_RANGE] of its destination: index
    /// 0 for the low clip, 1 for the mid, 2 for the high, picked at
    /// random by the swarm.
    pub fn step(&mut self, dt: f32, anchors: &[[f32; 2]], active: usize) -> Vec<usize> {
        if dt < 0.0 {
            return Vec::new();
        }
        self.t += dt;

        while self.plants_done < active.min(anchors.len()) {
            let home = self.plants_done;
            self.plants_done += 1;
            let hull = convex_hull(anchors);
            for j in 0..BUGS_PER_PLANT {
                let pos = sample_polygon(&hull, &mut self.rng);
                let idle = [
                    (j as f32 - 1.0) * BUG_SIZE * 0.9 + (self.rng.next_f32() - 0.5) * 10.0,
                    self.rng.next_f32() * 6.0,
                ];
                let bug = Bug {
                    pos,
                    spawn: pos,
                    vel: [0.0, 0.0],
                    facing: 1.0,
                    placed: false,
                    home,
                    idle,
                    walk: 0.0,
                    frame: 0,
                    shown: u8::MAX,
                    grow: 0.0,
                    speed: self.rng.in_range(10.0, 30.0),
                    wob_amp: self.rng.in_range(15.0, 30.0),
                    wob_freq: self.rng.in_range(1.5, 3.5),
                    wob_phase: self.rng.next_f32() * 2.0 * std::f32::consts::PI,
                    step_rate: self.rng.in_range(6.0, 10.0),
                    dying: None,
                    hits: HITS_TO_KILL,
                    parked: 0.0,
                    arrived: false,
                    hit_cooldown: 0.0,
                    respawn: None,
                };
                self.bugs.push(bug);
            }
        }

        // The bugs that reach their destination this frame, with their
        // destinations: the arrival tjatter check runs after the loop,
        // against the whole swarm's end-of-frame positions.
        let mut arrivals: Vec<(usize, [f32; 2])> = Vec::new();
        for (i, bug) in self.bugs.iter_mut().enumerate() {
            // The hit cooldown runs down every frame, so a wounded bug
            // takes its next hit only after [HIT_COOLDOWN].
            if bug.hit_cooldown > 0.0 {
                bug.hit_cooldown = (bug.hit_cooldown - dt).max(0.0);
            }

            // A dead bug sits out for its random respawn delay; when it
            // ends, the bug pops back up at its spawn spot as a fresh
            // bug — same speed, park spot and first steps as the first
            // time.
            if let Some(r) = bug.respawn {
                if r <= dt {
                    bug.respawn = None;
                    bug.pos = bug.spawn;
                    bug.grow = 0.0;
                    bug.vel = [0.0, 0.0];
                    bug.facing = 1.0;
                    bug.placed = false;
                    bug.walk = 0.0;
                    bug.frame = 0;
                    bug.shown = u8::MAX;
                    bug.hits = HITS_TO_KILL;
                    bug.parked = 0.0;
                    bug.arrived = false;
                    bug.hit_cooldown = 0.0;
                } else {
                    bug.respawn = Some(r - dt);
                }
                continue;
            }

            // Sprayed: the death clock runs. While the bounce is in
            // flight the bug holds its spot — position, growth,
            // facing and walk frame all frozen; it rides the
            // [bounce_lift] arc up off the grass and flips over in the
            // air (both in [Bugs::layout]), then, landed on its back,
            // the center sinks through the grass while [Bugs::layout]
            // shrinks the sprite to nothing; when the clock runs out,
            // the bug enters its respawn delay.
            if let Some(dead) = bug.dying {
                let t = dead + dt;
                if t >= BOUNCE_TIME + SINK_TIME {
                    bug.dying = None;
                    bug.respawn = Some(self.rng.in_range(RESPAWN_MIN, RESPAWN_MAX));
                    continue;
                }
                bug.dying = Some(t);
                if t >= BOUNCE_TIME {
                    bug.pos[1] -= (self.ground / SINK_TIME) * dt;
                }
                continue;
            }
            bug.grow = (bug.grow + dt).min(GROW_TIME);
            if bug.grow < GROW_TIME {
                // Still growing out of the grass: hold the spawn spot.
                continue;
            }
            let target = [
                anchors[bug.home][0] + bug.idle[0],
                anchors[bug.home][1] + bug.idle[1] + self.ground,
            ];
            let dx = target[0] - bug.pos[0];
            let dy = target[1] - bug.pos[1];
            let dist = (dx * dx + dy * dy).sqrt();
            let next = if dist > ARRIVE {
                // Swarm walk: advance toward the wobbled destination,
                // never more than one stride this frame.
                let wob = (self.t * bug.wob_freq + bug.wob_phase).sin() * bug.wob_amp;
                let dest = [target[0] - dy / dist * wob, target[1] + dx / dist * wob];
                let k = ((bug.speed * dt) / dist).min(1.0);
                [
                    bug.pos[0] + (dest[0] - bug.pos[0]) * k,
                    bug.pos[1] + (dest[1] - bug.pos[1]) * k,
                ]
            } else {
                // Parked: a tiny in-place sway (below FACING_EPS).
                let next = [
                    target[0] + (self.t * bug.wob_freq + bug.wob_phase).sin() * 1.5,
                    target[1],
                ];
                // A bug at its destination recovers one hit of health
                // every [REGEN_TIME] seconds, up to [MAX_HEALTH]; the
                // cycle is consumed whether or not it pays out, so the
                // counter never runs up while capped.
                bug.parked += dt;
                while bug.parked >= REGEN_TIME {
                    bug.parked -= REGEN_TIME;
                    if bug.hits < MAX_HEALTH {
                        bug.hits += 1;
                    }
                }
                // The first parked frame is the arrival: the bug has just
                // reached its destination (a bug born on its spot counts
                // too — it parks from its first grown frame).
                if !bug.arrived {
                    bug.arrived = true;
                    arrivals.push((i, target));
                }
                next
            };
            if dt > 0.0 && bug.placed {
                bug.vel = [(next[0] - bug.pos[0]) / dt, (next[1] - bug.pos[1]) / dt];
            } else {
                bug.vel = [bug.speed, 0.0];
                bug.placed = true;
            }
            bug.pos = next;

            if bug.vel[0] > FACING_EPS {
                bug.facing = 1.0;
            } else if bug.vel[0] < -FACING_EPS {
                bug.facing = -1.0;
            }

            if dist > ARRIVE {
                bug.walk += dt * bug.step_rate;
                bug.frame = (bug.walk as u32 % 3) as u8;
            }
        }

        // An arrival chatters — a random tjatter clip — only if another
        // bug is on the grass at its destination: anything that is not
        // dying and not sitting out a respawn delay, within
        // [TJATTER_RANGE] of it.
        let mut tjatters = Vec::new();
        for (i, dest) in arrivals {
            let company = self.bugs.iter().enumerate().any(|(j, b)| {
                j != i
                    && b.dying.is_none()
                    && b.respawn.is_none()
                    && {
                        let dx = b.pos[0] - dest[0];
                        let dy = b.pos[1] - dest[1];
                        dx * dx + dy * dy <= TJATTER_RANGE * TJATTER_RANGE
                    }
            });
            if company {
                tjatters.push((self.rng.next_f32() * 3.0) as usize);
            }
        }
        tjatters
    }

    /// Land one mist hit at `p`: every live bug within half the rendered
    /// bug width of `p` takes one hit — its [Bug::hits] drops by one, and
    /// the hit starts its [Bug::dying] clock when that takes it to zero.
    /// Dying and respawning bugs take no more hits, and a wounded bug
    /// takes its next hit only after its [HIT_COOLDOWN] has run out
    /// ([Bug::hit_cooldown]). Returns the number of bugs hit.
    pub fn hit_at(&mut self, p: [f32; 2]) -> usize {
        let r2 = (BUG_SIZE / 2.0) * (BUG_SIZE / 2.0);
        let mut hits = 0;
        for bug in &mut self.bugs {
            if bug.dying.is_some() || bug.respawn.is_some() || bug.hit_cooldown > 0.0 {
                continue;
            }
            let dx = p[0] - bug.pos[0];
            let dy = p[1] - bug.pos[1];
            if dx * dx + dy * dy <= r2 {
                bug.hit_cooldown = HIT_COOLDOWN;
                bug.hits -= 1;
                if bug.hits == 0 {
                    bug.dying = Some(0.0);
                }
                hits += 1;
            }
        }
        hits
    }

    /// The health counters of the bugs that are visible right now: for
    /// every bug that has popped up, is not dying, and is not waiting out
    /// its respawn delay — in pool order — the point above the bug where
    /// the counter rides, and the number of pips to draw, one per hit it
    /// can still take. The row tracks the bug's current height, so it
    /// rides the growth and the walk with it.
    pub fn health_pips(&self) -> Vec<([f32; 2], u8)> {
        self.bugs
            .iter()
            .filter(|b| b.grow > 0.0 && b.dying.is_none() && b.respawn.is_none())
            .map(|b| {
                let half = b.grow / GROW_TIME * BUG_SIZE / 2.0;
                ([b.pos[0], b.pos[1] + half + PIP_GAP], b.hits)
            })
            .collect()
    }

    /// Write the spawned prefix of `node`'s children.
    ///
    /// Each slot's transform is the bug position, plus the [bounce_lift]
    /// arc while the bug is mid-bounce, and its modulate the tint. Its
    /// scale is the uniform sprite scale with the x component carrying
    /// the facing flip and the y component the 0→1 spawn growth — or, for
    /// a dying bug, the death animation: over [BOUNCE_TIME] the y
    /// component sweeps to its negative, so the airborne bug lands on
    /// its back, and then both components shrink to zero over [SINK_TIME]
    /// as it evaporates. Its shape swaps when the walk frame changed, or
    /// when the slot still holds another frame's sprite. A bug in its
    /// respawn delay is gone from the grass: its slot is cleared and its
    /// last pose left in place. Slots past the spawned prefix are
    /// cleared, so an unspawned slot never draws.
    pub fn layout(&mut self, node: &mut SceneNode, frames: [&frost::Shape; 3]) {
        for (bug, child) in self.bugs.iter_mut().zip(&mut node.children) {
            if bug.respawn.is_some() {
                // Dead and waiting: clear the slot, keep the last pose.
                child.shape = None;
                continue;
            }
            let g = bug.grow / GROW_TIME;
            let (sx, sy, lift) = match bug.dying {
                Some(t) if t < BOUNCE_TIME => {
                    // Bounce phase: the center lifts off the grass on a
                    // parabolic arc — peak at the halfway point, back on
                    // the ground at the end — while the y scale sweeps
                    // from upright to fully inverted, so the bug lands
                    // on its back.
                    let u = t / BOUNCE_TIME;
                    (
                        bug.facing * self.scale,
                        self.scale * g * (1.0 - 2.0 * u),
                        bounce_lift(t),
                    )
                }
                Some(t) => {
                    // Evaporate phase: the inverted sprite shrinks to
                    // nothing while [Bugs::step] sinks it through the
                    // grass.
                    let s = 1.0 - (t - BOUNCE_TIME) / SINK_TIME;
                    (
                        bug.facing * self.scale * g * s,
                        -self.scale * g * s,
                        0.0,
                    )
                }
                None => (bug.facing * self.scale, self.scale * g, 0.0),
            };
            child.transform = frost::Transform::translate(bug.pos[0], bug.pos[1] + lift);
            child.scale = [sx, sy];
            child.modulate = TINT;
            let frame = frames[bug.frame as usize];
            if bug.shown != bug.frame || !same_sprite(&child.shape, frame) {
                child.shape = Some(frame.clone());
                bug.shown = bug.frame;
            }
        }
        for child in node.children.iter_mut().skip(self.bugs.len()) {
            child.shape = None;
        }
    }
}

/// The death-bounce lift: how far above the death spot the bug's center
/// rides at death-clock `t`, in px.
///
/// A parabolic arc that leaves the grass at `t == 0`, peaks at
/// [BOUNCE_HEIGHT] halfway through [BOUNCE_TIME], and lands back on the
/// grass at `t == BOUNCE_TIME`. The y scale sweeps upright→inverted over
/// the same span, so the bug comes down on its back.
fn bounce_lift(t: f32) -> f32 {
    let u = (t / BOUNCE_TIME).clamp(0.0, 1.0);
    4.0 * BOUNCE_HEIGHT * u * (1.0 - u)
}

/// Whether `shape` already holds `frame`'s sprite pixels.
///
/// The pointer test lets a slot inherit the node's shape only when it is
/// truly the bug's frame, and force a swap when it is not.
fn same_sprite(shape: &Option<frost::Shape>, frame: &frost::Shape) -> bool {
    match (shape, frame) {
        (
            Some(frost::Shape::Sprite { data: a, .. }),
            frost::Shape::Sprite { data: b, .. },
        ) => std::sync::Arc::ptr_eq(a, b),
        _ => false,
    }
}

/// The loaded frame's texture size in pixels.
fn sprite_size(shape: &frost::Shape) -> [f32; 2] {
    match shape {
        frost::Shape::Sprite { width, height, .. } => [*width as f32, *height as f32],
        _ => unreachable!("the frames are sprites"),
    }
}

/// The convex hull of `pts` as a counter-clockwise vertex ring.
///
/// Monotone chain: sort by (x, y), then build the lower and upper hulls
/// popping while the last turn is not counter-clockwise. The six plant
/// roots are distinct, so the hull has at least three vertices.
fn convex_hull(pts: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut v: Vec<[f32; 2]> = pts.to_vec();
    v.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));

    fn half_hull(v: &[[f32; 2]], flip: bool) -> Vec<[f32; 2]> {
        let n = v.len();
        let mut hull: Vec<[f32; 2]> = Vec::with_capacity(n);
        for i in 0..n {
            let p = v[if flip { n - 1 - i } else { i }];
            while hull.len() >= 2 {
                let a = hull[hull.len() - 2];
                let b = hull[hull.len() - 1];
                let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
                // Both halves pop on the same test (a non-counter-clockwise
                // turn); only the iteration direction differs, via `flip`.
                if cross > 0.0 {
                    break;
                }
                hull.pop();
            }
            hull.push(p);
        }
        hull
    }

    let mut lower = half_hull(&v, false);
    let mut upper = half_hull(&v, true);
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// A random point inside the convex polygon `poly`, uniform in area.
///
/// The polygon is fanned from `poly[0]`; a triangle is picked with
/// probability proportional to its area and then sampled uniformly by
/// barycentric coordinates.
fn sample_polygon(poly: &[[f32; 2]], rng: &mut Rng) -> [f32; 2] {
    let n = poly.len();
    let mut areas = Vec::with_capacity(n - 2);
    let mut total = 0.0f32;
    for i in 1..n - 1 {
        let a = poly[0];
        let b = poly[i];
        let c = poly[i + 1];
        let area = ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs();
        areas.push(area);
        total += area;
    }
    let mut pick = rng.next_f32() * total;
    // `areas` is 0-based with `n - 2` fan triangles; `areas[k]` is the
    // triangle (poly[0], poly[k + 1], poly[k + 2]). Walk it while the
    // remaining `pick` exceeds the current triangle's area — checking the
    // bound before the index so the last triangle is the fall-through.
    let mut k = 0usize;
    while k + 1 < areas.len() && pick > areas[k] {
        pick -= areas[k];
        k += 1;
    }
    let a = poly[0];
    let b = poly[k + 1];
    let c = poly[k + 2];
    let u = rng.next_f32().sqrt();
    let v = rng.next_f32();
    [
        a[0] + u * (b[0] - a[0]) + u * v * (c[0] - b[0]),
        a[1] + u * (b[1] - a[1]) + u * v * (c[1] - b[1]),
    ]
}

/// A tiny splitmix64 randomizer, so the module needs no random crate.
///
/// Seeded from the clock like the particle jitter in `main.rs`, so spawn
/// points differ between runs.
struct Rng(u64);

impl Rng {
    /// Seed from the clock.
    fn new() -> Self {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e37_79b9_7f4a_7c15);
        Self(t ^ 0x9e37_79b9_7f4a_7c15)
    }

    /// The next pseudo-random u64.
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A uniform f32 in [0, 1).
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }

    /// A uniform f32 in [lo, hi).
    fn in_range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.next_f32() * (hi - lo)
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

    /// Whether `p` lies on the inside (or on the boundary) of the convex
    /// ring `hull`, in whatever orientation the ring happens to have.
    ///
    /// The boundary tolerance is 1e-3 px of perpendicular distance: f32
    /// sampling noise at these coordinate magnitudes is a few ulp, so a
    /// strict zero-cross test would flag samples sitting on the edge.
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

    /// The hull of the six roots is the *other five*: `(1081, 640)` lies
    /// ~1 px inside the segment `(739, 571)–(1463, 719)`, so it is not a
    /// vertex. The exact ring order is pinned because the fan sampler
    /// depends on it.
    #[test]
    fn hull_is_the_expected_pentagon() {
        let hull = convex_hull(&ANCHORS);
        let expected: [[f32; 2]; 5] = [
            [739.0, 571.0],
            [923.0, 514.0],
            [1248.0, 546.0],
            [1633.0, 603.0],
            [1463.0, 719.0],
        ];
        assert_eq!(hull, expected.to_vec());
    }

    /// No sample may land outside the hull — the class of bug the earlier
    /// out-of-bounds `areas[i]` read and the malformed ring both caused.
    #[test]
    fn samples_land_inside_the_hull() {
        let hull = convex_hull(&ANCHORS);
        let mut rng = Rng::new();
        for _ in 0..100_000 {
            let p = sample_polygon(&hull, &mut rng);
            assert!(inside(&hull, p), "sample {p:?} escaped the hull");
        }
    }

    /// The fan sampling is uniform in area, so the sample mean lands near
    /// the polygon's centroid — a loose bound keeps the test stable across
    /// time-seeded runs.
    #[test]
    fn samples_are_uniform() {
        let hull = convex_hull(&ANCHORS);
        let mut rng = Rng::new();
        const N: u32 = 100_000;
        let (mut sx, mut sy) = (0.0f32, 0.0f32);
        for _ in 0..N {
            let p = sample_polygon(&hull, &mut rng);
            sx += p[0];
            sy += p[1];
        }
        let (mx, my) = (sx / N as f32, sy / N as f32);
        // The polygon's centroid, computed from its vertices.
        let mut area = 0.0f32;
        let mut cx = 0.0f32;
        let mut cy = 0.0f32;
        for w in 0..hull.len() {
            let a = hull[w];
            let b = hull[(w + 1) % hull.len()];
            let cross = a[0] * b[1] - a[1] * b[0];
            area += cross;
            cx += (a[0] + b[0]) * cross;
            cy += (a[1] + b[1]) * cross;
        }
        cx /= 3.0 * area;
        cy /= 3.0 * area;
        assert!(
            (mx - cx).abs() < 20.0 && (my - cy).abs() < 10.0,
            "mean ({mx}, {my}) is far from the centroid ({cx}, {cy})"
        );
    }

    /// While a bug is still growing out of the grass it holds its spawn
    /// spot — no walking, no frame advance, no movement baseline — and the
    /// movement logic takes over only once the growth clock reaches
    /// [GROW_TIME].
    #[test]
    fn bugs_hold_their_spot_while_growing() {
        let frame = frost::Shape::Sprite {
            data: std::sync::Arc::new([0u8; 1]),
            width: 10,
            height: 10,
            color: frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            alpha: 1.0,
        };
        let dt = 0.05;
        let mut bugs = Bugs::new([&frame; 3]);
        bugs.step(dt, &ANCHORS, 1);
        let spawn = bugs.bugs.iter().map(|b| b.pos).collect::<Vec<_>>();
        // Well under GROW_TIME: every bug holds its exact spawn spot.
        for _ in 0..55 {
            bugs.step(dt, &ANCHORS, 1);
            for (i, b) in bugs.bugs.iter().enumerate() {
                assert_eq!(b.pos, spawn[i], "bug {i} moved while growing");
                assert_eq!(b.walk, 0.0);
                assert!(!b.placed);
            }
        }
        // Past GROW_TIME: the movement logic must have taken over.
        for _ in 0..20 {
            bugs.step(dt, &ANCHORS, 1);
        }
        assert!(bugs.bugs.iter().all(|b| b.placed));
    }

    /// A bug the mist keeps touching takes [HITS_TO_KILL] hits before it
    /// dies — no matter how many drops touch it in a frame — and its
    /// death plays out in two phases: its position, growth, facing and
    /// walk frame freeze at the killing blow; over [BOUNCE_TIME] it
    /// bounces up off the grass — the laid-out y scale sweeping from
    /// upright to fully inverted in the air, so it lands on its back at
    /// full height — and then, over [SINK_TIME], it evaporates: the
    /// center sinks through the grass while the whole scale shrinks to
    /// zero. The bug then waits out a random delay in [RESPAWN_MIN]…
    /// [RESPAWN_MAX] — its slot cleared meanwhile — and pops back up at
    /// its spawn spot, fully healed.
    #[test]
    fn sprayed_bug_takes_hits_then_bounces_evaporates_and_respawns() {
        let frame = frost::Shape::Sprite {
            data: std::sync::Arc::new([0u8; 1]),
            width: 10,
            height: 10,
            color: frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            alpha: 1.0,
        };
        let dt = 0.05;
        let scale = BUG_SIZE / 10.0;
        let ground = scale * 10.0 / 2.0;
        let mut bugs = Bugs::new([&frame; 3]);
        let mut node = frost::SceneNode {
            children: (0..BUGS_PER_PLANT)
                .map(|_| Box::new(frost::SceneNode::default()))
                .collect(),
            ..Default::default()
        };

        // Spawn the batch and grow it fully, so the bugs are free to move
        // when the spray touches them.
        bugs.step(dt, &ANCHORS, 1);
        for _ in 0..80 {
            bugs.step(dt, &ANCHORS, 1);
        }
        bugs.layout(&mut node, [&frame; 3]);
        assert!(bugs.bugs.iter().all(|b| b.grow >= GROW_TIME - 1e-6));

        // Wear the bug farthest from its two siblings down, one hit per
        // round: each round touches it at its current position three
        // times in the same frame (the hit cooldown makes that one hit),
        // then waits the cooldown out before the next round.
        let pos: Vec<[f32; 2]> = bugs.bugs.iter().map(|b| b.pos).collect();
        let farthest = (0..pos.len()).max_by(|&a, &b| {
            let da = pos.iter().enumerate().filter(|(i, _)| *i != a)
                .map(|(_, q)| (q[0] - pos[a][0]).powi(2) + (q[1] - pos[a][1]).powi(2))
                .fold(f32::MAX, f32::min);
            let db = pos.iter().enumerate().filter(|(i, _)| *i != b)
                .map(|(_, q)| (q[0] - pos[b][0]).powi(2) + (q[1] - pos[b][1]).powi(2))
                .fold(f32::MAX, f32::min);
            da.partial_cmp(&db).unwrap()
        })
        .unwrap();
        for round in 1..HITS_TO_KILL {
            let p = bugs.bugs[farthest].pos;
            let hits = bugs.hit_at(p);
            bugs.hit_at(p);
            bugs.hit_at(p);
            assert!(hits >= 1, "the mist found no bug");
            assert_eq!(bugs.hit_at(p), 0, "the hit cooldown did not hold");
            assert_eq!(
                bugs.bugs[farthest].hits,
                HITS_TO_KILL - round,
                "a round of mist landed more than one hit"
            );
            assert!(
                bugs.bugs[farthest].dying.is_none(),
                "a sub-lethal round started the death"
            );
            // Wait the hit cooldown out before the next round.
            for _ in 0..5 {
                bugs.step(dt, &ANCHORS, 1);
            }
        }

        // The killing blow, aimed at the bug's current position.
        let hits = bugs.hit_at(bugs.bugs[farthest].pos);
        assert!(hits >= 1);
        let initial = bugs.bugs.len();
        // The farthest bug plus any sibling that wandered into the mist —
        // the pool is never trimmed, so the indices stay valid for the
        // whole death and respawn.
        let marked_idx: Vec<usize> = (0..initial)
            .filter(|&i| bugs.bugs[i].dying.is_some())
            .collect();
        assert!(!marked_idx.is_empty());
        // What froze at the killing blow, in the same order: position,
        // frame, facing, growth.
        let frozen: Vec<([f32; 2], u8, f32, f32)> = marked_idx
            .iter()
            .map(|&i| {
                let b = &bugs.bugs[i];
                (b.pos, b.frame, b.facing, b.grow)
            })
            .collect();
        let spawn: Vec<[f32; 2]> =
            marked_idx.iter().map(|&i| bugs.bugs[i].spawn).collect();

        let mut saw_upright = false;
        let mut min_y_scale = f32::MAX;
        let mut max_abs_y_scale: f32 = 0.0;
        let mut max_lift: f32 = 0.0;
        let mut first_y = f32::MIN;
        let mut last_y = f32::MAX;
        let mut last_abs_scale = f32::MAX;
        for _ in 0..40 {
            bugs.step(dt, &ANCHORS, 1);
            bugs.layout(&mut node, [&frame; 3]);
            for (k, &i) in marked_idx.iter().enumerate() {
                let b = &bugs.bugs[i];
                if b.dying.is_some() {
                    let (x0, frame0, facing0, grow0) = frozen[k];
                    assert_eq!(b.pos[0], x0[0], "bug {i} slid sideways while dying");
                    assert_eq!(b.frame, frame0, "bug {i} walked while dying");
                    assert_eq!(b.facing, facing0, "bug {i} turned while dying");
                    assert_eq!(b.grow, grow0, "bug {i} kept growing while dying");
                    if b.dying.unwrap() < BOUNCE_TIME {
                        assert_eq!(
                            b.pos[1],
                            x0[1],
                            "bug {i} sank during the bounce"
                        );
                        // Mid-bounce, the laid-out center rides the arc
                        // strictly above the death spot.
                        let laid_out_y = node.children[i]
                            .transform
                            .apply([0.0, 0.0])[1];
                        assert!(
                            laid_out_y > b.pos[1],
                            "bug {i}'s bounce never left the grass"
                        );
                        max_lift = max_lift.max(laid_out_y - b.pos[1]);
                    }
                    let s = node.children[i].scale[1];
                    if s > 0.0 {
                        saw_upright = true;
                    }
                    min_y_scale = min_y_scale.min(s);
                    max_abs_y_scale = max_abs_y_scale.max(s.abs());
                    first_y = first_y.max(b.pos[1]);
                    last_y = last_y.min(b.pos[1]);
                    last_abs_scale = last_abs_scale.min(s.abs());
                } else {
                    // The first frame the bug waits out its respawn: the
                    // delay must be the one its death drew.
                    let r =
                        b.respawn.expect("a marked bug is neither dying nor waiting");
                    assert!(
                        r > RESPAWN_MIN && r < RESPAWN_MAX,
                        "the respawn delay {r}s left the window"
                    );
                }
            }
            if marked_idx.iter().all(|&i| bugs.bugs[i].respawn.is_some()) {
                break;
            }
        }

        // The death completed: every marked bug is in its respawn delay,
        // and the pool itself never shrank.
        assert!(
            marked_idx
                .iter()
                .all(|&i| bugs.bugs[i].respawn.is_some()),
            "a marked bug never entered its respawn delay"
        );
        assert_eq!(bugs.bugs.len(), initial, "a bug left the pool");
        assert!(
            bugs.bugs.iter().all(|b| b.dying.is_none()),
            "a dying bug survived the full death"
        );
        // The bounce really left the grass and the flip really inverted
        // the sprite: it started upright, rode the arc up to its peak,
        // and came down fully inverted at full height, on its back.
        assert!(saw_upright, "the death never showed the upright phase");
        assert!(
            (max_lift - BOUNCE_HEIGHT).abs() <= 1e-3,
            "the bounce only rose to {max_lift} px of its {BOUNCE_HEIGHT} px arc"
        );
        assert!(
            min_y_scale <= -0.9 * scale,
            "the bounce never inverted the sprite (min y scale {min_y_scale})"
        );
        assert!(
            max_abs_y_scale >= 0.99 * scale,
            "the bug never came down at full height (max |y scale| {max_abs_y_scale})"
        );
        // The evaporate really carried the bugs through the grass: their
        // y dropped at least halfway to the ground line, and the sprite
        // shrank most of the way to nothing.
        assert!(
            last_y < first_y - 0.5 * ground,
            "the sink only carried the bugs to {last_y} from {first_y}"
        );
        assert!(
            last_abs_scale < 0.5 * scale,
            "the sink never shrank the sprite (last scale {last_abs_scale})"
        );
        // The waiting bugs' slots are cleared, and so are the unspawned
        // ones.
        for &i in &marked_idx {
            assert!(
                node.children[i].shape.is_none(),
                "a waiting slot kept its sprite"
            );
        }
        for child in node.children.iter().skip(initial) {
            assert!(child.shape.is_none(), "an unspawned slot kept its sprite");
        }

        // Run the delays out: well past the worst case, every marked bug
        // pops back up at its spawn spot, fully healed and regrowing from
        // the grass.
        let mut came_back = vec![false; marked_idx.len()];
        for _ in 0..((RESPAWN_MAX + 2.0) / dt) as usize {
            bugs.step(dt, &ANCHORS, 1);
            bugs.layout(&mut node, [&frame; 3]);
            for (k, &i) in marked_idx.iter().enumerate() {
                if !came_back[k] && bugs.bugs[i].respawn.is_none() {
                    came_back[k] = true;
                    assert_eq!(
                        bugs.bugs[i].pos, spawn[k],
                        "bug {i} popped up away from its spawn spot"
                    );
                    assert_eq!(
                        bugs.bugs[i].grow, 0.0,
                        "bug {i} popped up pre-grown"
                    );
                    assert_eq!(
                        bugs.bugs[i].hits, HITS_TO_KILL,
                        "bug {i} popped up wounded"
                    );
                    assert!(
                        node.children[i].shape.is_some(),
                        "bug {i} popped back invisible"
                    );
                }
            }
            if came_back.iter().all(|c| *c) {
                break;
            }
        }
        assert!(
            came_back.iter().all(|c| *c),
            "a bug never came back from its respawn delay"
        );
    }

    /// The death-bounce arc [bounce_lift] leaves the grass at the ends,
    /// peaks at [BOUNCE_HEIGHT] halfway through [BOUNCE_TIME], and is
    /// symmetric about that halfway point.
    #[test]
    fn bounce_lift_arcs_off_the_grass_and_back() {
        assert_eq!(bounce_lift(0.0), 0.0);
        assert_eq!(bounce_lift(BOUNCE_TIME), 0.0);
        assert_eq!(bounce_lift(BOUNCE_TIME / 2.0), BOUNCE_HEIGHT);
        for u in [0.1, 0.25, 0.4, 0.6, 0.9] {
            let t = BOUNCE_TIME * u;
            assert!(
                (bounce_lift(t) - bounce_lift(BOUNCE_TIME - t)).abs() <= 1e-4,
                "the arc is not symmetric about its peak at t={t}"
            );
        }
    }

    /// A bug the mist kills while it is still growing out of the grass
    /// freezes its growth at the killing blow: the death animation scales
    /// off the height the bug had when it died, and the bug never pops
    /// past it — not while it dies, not while it waits out its respawn
    /// delay.
    #[test]
    fn a_bug_hit_while_growing_freezes_its_growth() {
        let frame = frost::Shape::Sprite {
            data: std::sync::Arc::new([0u8; 1]),
            width: 10,
            height: 10,
            color: frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            alpha: 1.0,
        };
        let dt = 0.05;
        let scale = BUG_SIZE / 10.0;
        let mut bugs = Bugs::new([&frame; 3]);
        bugs.step(dt, &ANCHORS, 1);
        // One fifth of the way through the growth: the bugs hold their
        // exact spawn spots, so the positions below are stable.
        for _ in 0..6 {
            bugs.step(dt, &ANCHORS, 1);
        }
        assert!(bugs.bugs.iter().all(|b| b.grow < GROW_TIME));
        let pos: Vec<[f32; 2]> = bugs.bugs.iter().map(|b| b.pos).collect();
        // A drop on each exact spawn spot, five rounds of one hit each,
        // spaced by the hit cooldown: every bug takes one hit per round,
        // and the fifth round is the killing blow.
        for _ in 0..HITS_TO_KILL {
            for p in &pos {
                assert!(bugs.hit_at(*p) >= 1, "a drop found no bug");
            }
            for _ in 0..5 {
                bugs.step(dt, &ANCHORS, 1);
            }
        }
        assert!(bugs.bugs.iter().all(|b| b.dying.is_some()));
        let frozen_grow: Vec<f32> = bugs.bugs.iter().map(|b| b.grow).collect();

        let mut node = frost::SceneNode {
            children: (0..BUGS_PER_PLANT)
                .map(|_| Box::new(frost::SceneNode::default()))
                .collect(),
            ..Default::default()
        };
        for _ in 0..40 {
            bugs.step(dt, &ANCHORS, 1);
            bugs.layout(&mut node, [&frame; 3]);
            for (i, b) in bugs.bugs.iter().enumerate() {
                if b.dying.is_some() {
                    assert_eq!(b.grow, frozen_grow[i], "a dying bug kept growing");
                }
                // The laid-out height never exceeds the death-time height —
                // while the bug dies, and while it waits.
                let h = node.children[i].scale[1].abs();
                assert!(
                    h <= scale * frozen_grow[i] / GROW_TIME + 1e-3,
                    "bug {i} popped past its death-time height"
                );
            }
            if bugs.bugs.iter().all(|b| b.respawn.is_some()) {
                break;
            }
        }
        assert!(
            bugs.bugs
                .iter()
                .all(|b| b.dying.is_none() && b.respawn.is_some()),
            "the dead bugs never entered their respawn delay"
        );
    }

    /// A bug parked at its destination recovers one hit of health every
    /// [REGEN_TIME] seconds — not before the interval has run, and never
    /// above [MAX_HEALTH] — while a bug still on its walk recovers
    /// nothing, however long it walks.
    #[test]
    fn parked_bug_regeners_health_up_to_the_cap() {
        let frame = frost::Shape::Sprite {
            data: std::sync::Arc::new([0u8; 1]),
            width: 10,
            height: 10,
            color: frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            alpha: 1.0,
        };
        let dt = 0.05;
        let scale = BUG_SIZE / 10.0;
        let ground = scale * 10.0 / 2.0;

        // Park a fully grown bug on its exact destination and watch its
        // counter climb one hit per full interval, capping at
        // [MAX_HEALTH].
        let mut parked = Bugs::new([&frame; 3]);
        parked.step(dt, &ANCHORS, 1);
        let b = &mut parked.bugs[0];
        b.grow = GROW_TIME;
        b.pos = [
            ANCHORS[b.home][0] + b.idle[0],
            ANCHORS[b.home][1] + b.idle[1] + ground,
        ];
        // Just under one interval: the counter has not moved.
        for _ in 0..99 {
            parked.step(dt, &ANCHORS, 1);
        }
        assert_eq!(
            parked.bugs[0].hits,
            HITS_TO_KILL,
            "the counter climbed before the interval ran"
        );
        // Cross the 5 s, 10 s and 15 s marks: one hit per full interval.
        for (extra, hits) in [
            (2, HITS_TO_KILL + 1),
            (100, HITS_TO_KILL + 2),
            (100, MAX_HEALTH),
        ] {
            for _ in 0..extra {
                parked.step(dt, &ANCHORS, 1);
            }
            assert_eq!(
                parked.bugs[0].hits, hits,
                "the parked counter missed its climb to {hits}"
            );
        }
        // Well past the cap: the counter holds at [MAX_HEALTH].
        for _ in 0..100 {
            parked.step(dt, &ANCHORS, 1);
        }
        assert_eq!(
            parked.bugs[0].hits, MAX_HEALTH,
            "the parked counter ran past the cap"
        );

        // A bug still on its walk recovers nothing: 900 px out at the
        // top walking speed (30 px/s) is 30 s of walking, so it cannot
        // reach its destination within the 20 s window below.
        let mut walking = Bugs::new([&frame; 3]);
        walking.step(dt, &ANCHORS, 1);
        let b = &mut walking.bugs[0];
        b.grow = GROW_TIME;
        b.pos = [ANCHORS[b.home][0] + 900.0, ANCHORS[b.home][1] + ground];
        for _ in 0..400 {
            walking.step(dt, &ANCHORS, 1);
        }
        let b = &walking.bugs[0];
        assert!(
            (ANCHORS[b.home][0] + b.idle[0] - b.pos[0]).abs() > ARRIVE,
            "the test bug reached its destination inside the window"
        );
        assert_eq!(
            b.hits, HITS_TO_KILL,
            "a walking bug regenerated before arriving"
        );
    }

    /// A bug that reaches its destination while another bug is on the
    /// grass within [TJATTER_RANGE] of it chatters: [Bugs::step] reports
    /// exactly one tjatter clip index (0, 1 or 2) for the frame and
    /// nothing on the frame after, when no bug arrives; a bug that parks
    /// with every other bug far away chatters nothing.
    #[test]
    fn arriving_bug_chatters_only_when_a_bug_is_there() {
        let frame = frost::Shape::Sprite {
            data: std::sync::Arc::new([0u8; 1]),
            width: 10,
            height: 10,
            color: frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            alpha: 1.0,
        };
        let dt = 0.05;
        let scale = BUG_SIZE / 10.0;
        let ground = scale * 10.0 / 2.0;

        // Deterministic park spots: the spawn jitter is replaced by the
        // exact nominal offsets, so the siblings sit 36 px apart.
        let pin = |bugs: &mut Bugs, j: usize, dx: f32| {
            let b = &mut bugs.bugs[j];
            b.idle = [(j as f32 - 1.0) * BUG_SIZE * 0.9, 0.0];
            b.grow = GROW_TIME;
            b.pos = [
                ANCHORS[b.home][0] + b.idle[0] + dx,
                ANCHORS[b.home][1] + b.idle[1] + ground,
            ];
        };

        let mut bugs = Bugs::new([&frame; 3]);
        bugs.step(dt, &ANCHORS, 1);
        // Bug 1 parks on its spot; bugs 0 and 2 sit 900 px out, so
        // neither can arrive within this test's window.
        pin(&mut bugs, 1, 0.0);
        pin(&mut bugs, 0, 900.0);
        pin(&mut bugs, 2, -900.0);
        // Bug 1's own arrival finds no one within range: silent.
        let clips = bugs.step(dt, &ANCHORS, 1);
        assert!(
            bugs.bugs[1].arrived,
            "bug 1 never reached its destination"
        );
        assert!(clips.is_empty(), "a lone arrival chattered: {clips:?}");

        // Bug 0 lands exactly on its spot: bug 1's park spot is 36 px
        // away, inside [TJATTER_RANGE] — the arrival chatters once, on a
        // clip within the tjatter range.
        let b0 = &mut bugs.bugs[0];
        b0.pos = [
            ANCHORS[b0.home][0] + b0.idle[0],
            ANCHORS[b0.home][1] + b0.idle[1] + ground,
        ];
        let clips = bugs.step(dt, &ANCHORS, 1);
        assert!(
            bugs.bugs[0].arrived,
            "bug 0 never reached its destination"
        );
        assert_eq!(
            clips.len(),
            1,
            "the arrival chattered {} times, not once",
            clips.len()
        );
        assert!(
            (0..3).contains(&clips[0]),
            "clip index {clips:?} left the tjatter range"
        );

        // The next frame nobody arrives: nothing plays.
        let clips = bugs.step(dt, &ANCHORS, 1);
        assert!(clips.is_empty(), "a parked bug chattered again: {clips:?}");

        // And a bug that parks with no company chatters nothing.
        let mut alone = Bugs::new([&frame; 3]);
        alone.step(dt, &ANCHORS, 1);
        pin(&mut alone, 0, 0.0);
        pin(&mut alone, 1, 900.0);
        pin(&mut alone, 2, -900.0);
        let clips = alone.step(dt, &ANCHORS, 1);
        assert!(
            alone.bugs[0].arrived,
            "the lone bug never reached its destination"
        );
        assert!(
            clips.is_empty(),
            "a bug that parks alone chattered: {clips:?}"
        );
    }
}
