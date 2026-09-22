//! A tomato plant that grows slice by slice and sways in a travelling wind,
//! factored out of the `grow` example so the `immortal` example can grow one
//! on its grass: five slices (`plant1.png` the base, `plant2.png` the low
//! middle, `plant3.png` the middle, `plant4.png` the high middle,
//! `plant5.png` the top, chained through hand-picked joint positions),
//! where each slice grows from zero to its full size on its own timetable —
//! the base over 3 seconds, the low middle over 6, the middle over 9, the
//! high middle over 12, the top over 15 — all starting at the same time.
//! Each slice grows out
//! of its lower joint, the joint it attaches to the previous slice through,
//! so the chain stays connected while it grows, the root joint (plant1's
//! lower joint) stays in place, and each upper slice sprouts from the moving
//! top of the one below it. From 15 seconds on the plant is at full size and
//! keeps swaying: the base rock turns the whole plant around its lower
//! joint, and the two joints above it bend a little more each, so the tip
//! moves the most. The clock keeps ticking past full size — a finished
//! slice opens its blooms one flower at a time, the first the frame the
//! slice is fully grown and each next a random [BLOOM_GAP_MIN] to
//! [BLOOM_GAP_MAX] seconds after the previous, each flower and its tomato
//! then riding its own start, the whole plant complete at [bloom_time].
//! A tomato placed in the basket starts its bloom over:
//! the flower is removed and regrows from zero on the same timetable —
//! [FLOWER_GROW_TIME] for the flower, [TOMATO_GROW_TIME] for the tomato —
//! and the plant is complete again only when every regrown bloom is full.
//!
//! The joints are given in each image's own pixel space: `(0, 0)` is the
//! image's upper-left corner, `x` grows to the right and `y` grows down. A
//! sprite is centered on its node's origin, and the scene's y axis points
//! up, so a joint at image pixels `(jx, jy)` of an `w`x`h` image is
//! converted to the node-local offset `(jx - w/2, h/2 - jy)` — the y flip
//! included.
//!
//! The plant's layout is driven by a [`Plant`] value: `new` builds the
//! chain from the five slice shapes, `step` advances the growth clock, and
//! `layout` lays the chain out in a plant node whose children — in chain
//! order — are the five slice nodes, anchoring the root joint at a given
//! parent-space point and applying the base rock and the joint bends.
//! Once a slice is fully grown, its blooms open: the four lower slices'
//! hand-picked spawn points — [FLOWER_SPAWNS]; the top slice bears none —
//! each get a flower slot, in the flower children that follow the slice
//! nodes. The slice's flowers start one at a time — the first the frame
//! the slice is fully grown, the next a random [BLOOM_GAP_MIN] to
//! [BLOOM_GAP_MAX] seconds after the previous, until every flower on the
//! slice is growing — and a slot's flower grows from zero to full size
//! over [FLOWER_GROW_TIME] seconds from its start, its sprite tinted
//! light green to yellow, and when the flower is fully grown its tomato
//! grows out of the same point — the fruit's own growth, tint, and
//! staleness live in the [tomato] module: it grows from zero to
//! [tomato::TOMATO_MAX_SCALE] of its natural size over
//! [tomato::TOMATO_GROW_TIME] seconds, the white fruit body (`tomato.png`)
//! tinted dark green to red, with the dark calyx and stem
//! (`tomato_fg.png`) drawn on top, and a fully grown fruit stales on the
//! plant's aging clock — the one [Plant::age] advances every frame, with
//! or without water: [tomato::STALE_DELAY] seconds after full growth, its
//! body modulates from ripe red to the dark red of [tomato::TOMATO_STALE],
//! over [tomato::STALE_TIME]. Each slot rides its slice's transform so the
//! whole bloom sways with the plant.
//! The plant's fit scale is the plant node's own `scale`, set once by the
//! caller: the node's scale applies before
//! its transform, to its subtree, so the whole plant sizes around the root
//! joint while the joint itself still lands exactly on the anchor. The
//! [`layer_midpoints`] function gives the five layer-segment midpoints
//! along the static, fully grown chain — the sway left out — for the
//! vipers' orbit centers.

use super::tomato::{tomato_leaf_offset, Tomato, TOMATO_GROW_TIME, TOMATO_MAX_SCALE};

#[cfg(test)]
use super::tomato::{STALE_DELAY, STALE_TIME, TOMATO_BG, TOMATO_FG, TOMATO_RED, TOMATO_STALE};

/// The hand-picked joints of the five slices, in each image's pixel space:
/// `(0, 0)` at the upper-left, `x` right, `y` down. `[0]` is where the
/// slice attaches to the previous one, `[1]` where the next slice attaches.
pub const JOINTS: [[(f32, f32); 2]; 5] = [
    [(317.0, 671.0), (317.0, 578.0)], // plant1, the base
    [(319.0, 581.0), (318.0, 351.0)], // plant2, the low middle
    [(318.0, 463.0), (309.0, 89.0)],  // plant3, the middle
    [(306.0, 412.0), (301.0, 166.0)], // plant4, the high middle
    [(124.0, 196.0), (124.0, 100.0)], // plant5, the top
];

/// How long each slice takes to grow from zero to its full size, in
/// seconds, in chain order: the base over 3 s, the next over 6, etc.
/// All slices start at the same time, each grows linearly out of
/// its own lower joint, and after the last slice is done the plant stays
/// at full size and keeps swaying.
pub const GROW_TIMES: [f32; 5] = [3.0, 6.0, 9.0, 12.0, 15.0];

/// The time at which the plant is fully grown: the last slice's growth
/// time — from then on every slice holds at full length while the plant
/// keeps swaying.
pub const FULL_GROW_TIME: f32 = *GROW_TIMES.last().expect("GROW_TIMES is non-empty");

/// The flower spawn points of the four lower slices — the top slice bears
/// none — in each slice image's pixel space: `(0, 0)` at the upper-left,
/// `x` right, `y` down; `FLOWER_SPAWNS[i]` holds slice `i`'s points. `new`
/// converts them to node-local space against each texture's real size, and
/// `layout` opens them on the frame their slice reaches full size.
pub const FLOWER_SPAWNS: [&[(f32, f32)]; 4] = [
    &[(308.0, 615.0)], // plant1, the base
    &[(306.0, 496.0), (638.0, 428.0), (304.0, 392.0)], // plant2, the low middle
    &[
        (315.0, 392.0),
        (326.0, 315.0),
        (297.0, 258.0),
        (320.0, 203.0),
        (300.0, 128.0),
    ], // plant3, the middle
    &[
        (339.0, 364.0),
        (298.0, 318.0),
        (356.0, 258.0),
        (388.0, 228.0),
        (334.0, 203.0),
        (283.0, 182.0),
    ], // plant4, the high middle
];

/// The flower slots per plant: one shapeless slot per [FLOWER_SPAWNS]
/// point, in flattened slice order — the plant node's children after the
/// [SLICE_N] slice nodes.
pub const FLOWER_N: usize =
    FLOWER_SPAWNS[0].len()
        + FLOWER_SPAWNS[1].len()
        + FLOWER_SPAWNS[2].len()
        + FLOWER_SPAWNS[3].len();

/// The slices per plant: the plant node's children are the [SLICE_N] slice
/// nodes first, in chain order, and the [FLOWER_N] flower slots after them.
pub const SLICE_N: usize = JOINTS.len();

/// The plant node's child index of flower slot `slot`: the slots follow
/// the slice children, in flattened [FLOWER_SPAWNS] order.
pub fn slot_index(slot: usize) -> usize {
    SLICE_N + slot
}

/// How long a flower takes to grow from zero to its full size, in seconds:
/// it starts when its slice's staggered opening sets it in motion — one
/// flower at a time, the first the frame the slice is fully grown, each
/// next [BLOOM_GAP_MIN] to [BLOOM_GAP_MAX] seconds after the previous —
/// and reaches full size [FLOWER_GROW_TIME] seconds later, growing out of
/// its spawn point.
pub const FLOWER_GROW_TIME: f32 = 10.0;

/// The span, in seconds, between two flowers starting to grow on the same
/// finished slice: the first starts the frame the slice is fully grown,
/// the next starts a random [BLOOM_GAP_MIN, BLOOM_GAP_MAX) seconds after
/// it — and so on, one at a time, until every flower on the slice is
/// growing.
pub const BLOOM_GAP_MIN: f32 = 2.0;
pub const BLOOM_GAP_MAX: f32 = 5.0;

/// The flower sprite's color at the start of its growth: light green.
pub const FLOWER_BUD: frost::Color = frost::Color {
    r: 0.6,
    g: 0.85,
    b: 0.4,
    a: 1.0,
};

/// The flower sprite's color at full growth: yellow.
pub const FLOWER_BLOOM: frost::Color = frost::Color {
    r: 1.0,
    g: 0.85,
    b: 0.2,
    a: 1.0,
};

/// The flower slot's children, in draw order: the flower leaf first, then
/// the tomato pivot — so the fruit paints on top of the bloom.
pub const SLOT_FLOWER: usize = 0;
pub const SLOT_TOMATO: usize = 1;

/// The travelling wind's angular frequency, in radians per second: the base
/// rock and the two joint bends lag each other by a fixed phase.
const SWAY_FREQ: f32 = 1.2;

/// The base rock's amplitude, in radians: it turns the whole plant around
/// its lower joint.
const SWAY_BASE: f32 = 0.025;

/// The joints' extra bends, in radians, each lagging the base rock: each
/// entry is a phase lag and an amplitude. `layout` uses the first two —
/// the first joint above the base bends by `[0]`, the next by `[0] + [1]`,
/// a little more — and the top slices follow the base rock and those two
/// bends as one, so the tip moves the most.
const SWAY_BENDS: [(f32, f32); 4] = [(0.8, 0.04), (1.6, 0.07), (2.4, 0.1), (3.2, 0.12)];

/// One slice's two joints, already converted to node-local coordinates.
#[derive(Clone)]
struct Link {
    /// The local position of the joint that attaches to the previous slice.
    from: [f32; 2],
    /// The local position of the joint that the next slice attaches to.
    to: [f32; 2],
}

/// Converts a joint from the image's pixel space — `(0, 0)` at the upper-
/// left, `y` down — to node-local coordinates: the sprite is centered on
/// the node's origin and the scene's y axis points up.
fn local_joint(jx: f32, jy: f32, size: [f32; 2]) -> [f32; 2] {
    [jx - size[0] / 2.0, size[1] / 2.0 - jy]
}

/// The loaded sprite's texture size in pixels.
fn sprite_size(shape: &frost::Shape) -> [f32; 2] {
    match shape {
        frost::Shape::Sprite { width, height, .. } => [*width as f32, *height as f32],
        _ => unreachable!("the slice is a sprite"),
    }
}

/// A slice's joints in node-local space, from the hand-picked pixel
/// coordinates and the texture's real size.
fn link(shape: &frost::Shape, joints: [(f32, f32); 2]) -> Link {
    let size = sprite_size(shape);
    Link {
        from: local_joint(joints[0].0, joints[0].1, size),
        to: local_joint(joints[1].0, joints[1].1, size),
    }
}

/// A slice's flower spawn points in node-local space, from the hand-
/// picked pixel coordinates and the texture's real size.
fn flower_points(shape: &frost::Shape, pts: &[(f32, f32)]) -> Vec<[f32; 2]> {
    let size = sprite_size(shape);
    pts.iter().map(|&(jx, jy)| local_joint(jx, jy, size)).collect()
}

/// The flower's growth, 0..1, at plant-clock `t` for a slice that finishes
/// growing at `grow_time`: zero before, one [FLOWER_GROW_TIME] seconds
/// after, linear in between.
fn flower_growth(t: f32, grow_time: f32) -> f32 {
    ((t - grow_time) / FLOWER_GROW_TIME).clamp(0.0, 1.0)
}

/// Lerps two colors channel by channel: `a` at `t = 0`, `b` at `t = 1`.
fn mix(a: frost::Color, b: frost::Color, t: f32) -> frost::Color {
    frost::Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

/// The flower sprite's color at growth `g`: light green to yellow.
fn flower_color(g: f32) -> frost::Color {
    mix(FLOWER_BUD, FLOWER_BLOOM, g)
}

/// The staggered first-bloom starts in flattened [FLOWER_SPAWNS] order,
/// drawn from a fresh, clock-seeded [frost::Rng]: for each slice, a
/// random permutation of its flower slots — the first starting on the
/// slice's [GROW_TIMES] entry, each next a random [BLOOM_GAP_MIN,
/// BLOOM_GAP_MAX) seconds after the previous one — so a finished slice
/// opens its blooms one flower at a time until all of them are growing.
fn staggered_starts() -> [f32; FLOWER_N] {
    let mut rng = frost::Rng::new();
    let mut starts = [0.0f32; FLOWER_N];
    let mut slot = 0usize;
    for (i, spawns) in FLOWER_SPAWNS.iter().enumerate() {
        // A random order for this slice's flowers: a Fisher-Yates shuffle.
        let mut order: Vec<usize> = (0..spawns.len()).collect();
        for k in (1..order.len()).rev() {
            let j = (rng.next_f32() * (k + 1) as f32) as usize;
            order.swap(k, j);
        }
        let mut at = GROW_TIMES[i];
        for (k, &s) in order.iter().enumerate() {
            if k > 0 {
                at += rng.in_range(BLOOM_GAP_MIN, BLOOM_GAP_MAX);
            }
            starts[slot + s] = at;
        }
        slot += spawns.len();
    }
    starts
}

/// A five-slice plant that grows out of its root joint and sways in a
/// traveling wind. The shapes themselves live in the scene; the value only
/// keeps the growth clock, the slices' joints, the flower spawn points —
/// in node-local space — the staggered first-bloom starts, and each slot's
/// [tomato::Tomato].
#[derive(Clone)]
pub struct Plant {
    /// Elapsed time in seconds.
    t: f32,
    /// The aging clock, in seconds: [Plant::age] advances it every frame
    /// at the growth's slowed pace — water or not — and it is the clock
    /// ripe tomatoes age on: [tomato::STALE_DELAY] after their ripe
    /// moment, stamped in [tomato::Tomato::ripened_at], they modulate from
    /// ripe red to [tomato::TOMATO_STALE] over [tomato::STALE_TIME], so
    /// the wait and the stale proceed even while the growth clock is
    /// frozen on a dry reserve.
    age: f32,
    /// The five slices in chain order, with their joints in node-local
    /// space.
    links: [Link; 5],
    /// The four lower slices' flower spawn points in node-local space, in
    /// flattened slice order: `new` converts [FLOWER_SPAWNS] against each
    /// texture's real size.
    tomato_spawn: [Vec<[f32; 2]>; 4],
    /// The plant-clock moment each bloom's schedule was restarted by
    /// [Plant::regrow] after its tomato was placed in the basket: `None`
    /// keeps the bloom on its slice's first-bloom timetable, where the
    /// flower starts at the staggered [first_bloom_start] moment; at the
    /// stamped moment the flower starts growing from zero over
    /// [FLOWER_GROW_TIME], its tomato over [TOMATO_GROW_TIME] after it.
    regrow_at: [Option<f32>; FLOWER_N],
    /// The plant-clock moment each slot's flower starts growing on the
    /// first bloom: [Plant::new] draws them slice by slice, one flower at
    /// a time — the first on the slice's [GROW_TIMES] entry, each next a
    /// random [BLOOM_GAP_MIN, BLOOM_GAP_MAX) seconds after the previous —
    /// so [Plant::regrow] can restart a bloom against its own staggered
    /// start rather than the slice's.
    first_bloom_start: [f32; FLOWER_N],
    /// Each bloom slot's tomato: its picked state — whether its pivot has
    /// been reparented out of the plant's tree, so [Plant::layout] skips
    /// it — and, once it has ripened, the aging-clock moment of its ripe
    /// moment, stamped by [Plant::age] on the frame the fruit ripens and
    /// cleared by [Plant::regrow], so the regrown fruit stamps its own
    /// ripe moment.
    tomatoes: [Tomato; FLOWER_N],
}

impl Plant {
    /// Builds the plant from the five slice shapes in chain order
    /// (`plant1`, `plant2`, `plant3`, `plant4`, `plant5`), with each
    /// slice's joints converted from `JOINTS` and the flower spawn points
    /// from `FLOWER_SPAWNS` to node-local space against the textures' real
    /// sizes. The growth clock starts at zero.
    pub fn new(shapes: [&frost::Shape; 5]) -> Self {
        Plant {
            t: 0.0,
            age: 0.0,
            links: [
                link(shapes[0], JOINTS[0]),
                link(shapes[1], JOINTS[1]),
                link(shapes[2], JOINTS[2]),
                link(shapes[3], JOINTS[3]),
                link(shapes[4], JOINTS[4]),
            ],
            tomato_spawn: [
                flower_points(shapes[0], FLOWER_SPAWNS[0]),
                flower_points(shapes[1], FLOWER_SPAWNS[1]),
                flower_points(shapes[2], FLOWER_SPAWNS[2]),
                flower_points(shapes[3], FLOWER_SPAWNS[3]),
            ],
            regrow_at: [None; FLOWER_N],
            first_bloom_start: staggered_starts(),
            tomatoes: [Tomato::new(); FLOWER_N],
        }
    }

    /// Advances the growth clock by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        self.t += dt;
    }

    /// Advances the aging clock by `dt` seconds — the caller steps it
    /// every frame at the growth's slowed pace, water or not — and stamps
    /// the ripe moment of every bloom whose tomato has just reached full
    /// growth: from its stamp the fruit waits [tomato::STALE_DELAY] and
    /// stales over [tomato::STALE_TIME] on this clock, so a ripe fruit
    /// ages on while the growth clock is frozen on a dry reserve. The
    /// stamp is the aging clock's value at the fruit's ripe moment on the
    /// growth clock, exact for any `dt`: within a frame the two clocks run
    /// in lockstep, so the ripe moment — the schedule's ripe time — maps
    /// to `age + (ripe_time - t)`.
    pub fn age(&mut self, dt: f32) {
        self.age += dt;
        for slot in 0..FLOWER_N {
            self.tomatoes[slot]
                .stamp_ripe(self.t, self.age, self.bloom_start(slot));
        }
    }

    /// Whether the plant is fully grown: the last slice has reached its
    /// full length; from here on every slice holds at full length while the
    /// plant keeps swaying and its blooms keep growing, until it is
    /// complete at [bloom_time].
    pub fn fully_grown(&self) -> bool {
        self.t >= FULL_GROW_TIME
    }

    /// The plant-clock moment the plant's last first bloom reaches full
    /// size: the latest [first_bloom_start] entry — the last flower of the
    /// latest slice's staggered opening — plus [FLOWER_GROW_TIME] plus
    /// [TOMATO_GROW_TIME]; from that moment the plant is complete on its
    /// first-bloom timetable, and every bloom restarted by [Plant::regrow]
    /// after a harvest into the basket holds the completion back to its
    /// own stamp plus both growth times.
    fn bloom_time(&self) -> f32 {
        *self
            .first_bloom_start
            .iter()
            .max_by(|a, b| f32::total_cmp(a, b))
            .expect("FLOWER_N is non-empty") + FLOWER_GROW_TIME + TOMATO_GROW_TIME
    }

    /// Whether the plant is complete: every slice is fully grown and every
    /// bloom — each flower and its tomato — has reached full size, from
    /// [bloom_time] on, and every bloom reset by [Plant::regrow] after a
    /// harvest into the basket has finished its second run — its flower
    /// and tomato grown over their [FLOWER_GROW_TIME] and
    /// [TOMATO_GROW_TIME] from the reset; before that, fully grown slices
    /// keep opening blooms as the clock advances.
    pub fn complete(&self) -> bool {
        self.t >= self.bloom_time()
            && self
                .regrow_at
                .iter()
                .all(|r| r.is_none_or(|r| self.t >= r + FLOWER_GROW_TIME + TOMATO_GROW_TIME))
    }

    /// The layers the plant has fully grown: how many of [GROW_TIMES] the
    /// growth clock has reached — one per slice, from the base up, and
    /// [GROW_TIMES.len()] once the plant is fully grown.
    pub fn grown_layers(&self) -> usize {
        GROW_TIMES.iter().filter(|&&g| self.t >= g).count()
    }

    /// The midpoints of the five layer segments along the static, fully
    /// grown chain — the sway left out — in the plant node's local space:
    /// the origin at the root joint, y up, before the caller's fit scale.
    /// Segment `i` runs from the joint the slice attaches to the previous
    /// slice through to the joint the next slice attaches to, so the
    /// midpoint sits half a grown slice on from the chain's running joint,
    /// and the next segment continues from the previous slice's upper
    /// joint.
    pub fn layer_midpoints(&self) -> [[f32; 2]; 5] {
        let mut midpoints = [[0.0, 0.0]; 5];
        let mut p = [0.0f32, 0.0];
        for (m, link) in midpoints.iter_mut().zip(&self.links) {
            let d = [link.to[0] - link.from[0], link.to[1] - link.from[1]];
            *m = [p[0] + d[0] / 2.0, p[1] + d[1] / 2.0];
            p = [p[0] + d[0], p[1] + d[1]];
        }
        midpoints
    }

    /// The cumulative fall height, in the plant node's local space — y up,
    /// before the caller's fit scale — for a tomato on slice `slice`: the
    /// sum of the vertical joint deltas of the segments from the root
    /// through the slice's own, so a tomato on the root segment falls one
    /// segment's height, one on the second segment the sum of the first
    /// two, and so on.
    pub fn fall_height(&self, slice: usize) -> f32 {
        self.links[..=slice]
            .iter()
            .map(|l| l.to[1] - l.from[1])
            .sum()
    }

    /// The plant-clock moment the bloom `slot`'s flower starts growing:
    /// the slot's staggered [first_bloom_start] entry on the first bloom,
    /// or the [Plant::regrow] stamp after a harvest into the basket.
    fn bloom_start(&self, slot: usize) -> f32 {
        self.regrow_at[slot].unwrap_or(self.first_bloom_start[slot])
    }

    /// Whether the bloom `slot` carries a ripe tomato: its tomato's growth
    /// has reached full size — the fruit is fully developed and red — so
    /// the player may pick it; a regrown bloom ripens on its restarted
    /// schedule.
    pub fn ripe(&self, slot: usize) -> bool {
        self.tomatoes[slot].is_ripe(self.t, self.bloom_start(slot))
    }

    /// Whether the bloom `slot` carries a fully overgrown tomato: its
    /// staleness — the mix toward [tomato::TOMATO_STALE] on the aging
    /// clock — has reached full, so the fruit has reached its final dark
    /// red and drops off the plant; a regrown bloom overgrows on its
    /// restarted schedule.
    pub fn overgrown(&self, slot: usize) -> bool {
        self.tomatoes[slot].is_overgrown(self.age)
    }

    /// Whether the bloom `slot`'s tomato is currently picked: its pivot has
    /// been reparented out of the plant's tree, and [Plant::layout] skips
    /// the slot's tomato while this holds.
    pub fn is_harvested(&self, slot: usize) -> bool {
        self.tomatoes[slot].is_harvested()
    }

    /// Marks the bloom `slot`'s tomato as picked, so [Plant::layout] skips
    /// its pivot — which the caller has reparented out of the tree.
    pub fn harvest(&mut self, slot: usize) {
        self.tomatoes[slot].harvest();
    }

    /// Clears the bloom `slot`'s picked mark, so the next [Plant::layout]
    /// restores its pivot — which the caller has reparented back into the
    /// tree — with its growth.
    pub fn unharvest(&mut self, slot: usize) {
        self.tomatoes[slot].unharvest();
    }

    /// Resets the bloom `slot` now that its tomato has been placed in the
    /// basket: the picked mark clears — the caller gives the slot a fresh,
    /// shapeless tomato pivot — and the bloom's schedule restarts from the
    /// growth clock's current value, so the flower grows from zero to full
    /// size over [FLOWER_GROW_TIME] and its tomato from zero to full size
    /// over [TOMATO_GROW_TIME] after it, the same conditions as the first
    /// bloom off the slice; the ripe stamp clears, so the staleness runs
    /// from the regrown fruit's own ripe moment. The next [Plant::layout]
    /// removes the flower and starts the regrowth.
    pub fn regrow(&mut self, slot: usize) {
        self.tomatoes[slot].regrow();
        self.regrow_at[slot] = Some(self.t);
    }

    /// The body center of the bloom `slot`'s tomato, in the plant node's
    /// local space — y up, before the caller's fit scale — for the slot
    /// node whose transform lays the bloom's origin on its spawn point:
    /// the pivot origin — the [tomato::TOMATO_TOP] stem point, on the
    /// flower's center — shifted by the pivot scale times the
    /// [tomato::tomato_leaf_offset] leaf offset, mapped through the slot's
    /// translate.
    pub fn tomato_center(
        &self,
        slot: usize,
        slot_node: &frost::SceneNode,
        tomato: &frost::Shape,
    ) -> [f32; 2] {
        let s = self.tomatoes[slot].growth(self.t, self.bloom_start(slot)) * TOMATO_MAX_SCALE;
        let [ox, oy] = tomato_leaf_offset(sprite_size(tomato));
        slot_node.transform.apply([ox * s, oy * s])
    }

    /// Lays the plant out in `node`, whose children — in chain order — are
    /// the [SLICE_N] slice nodes followed by the [FLOWER_N] flower slots,
    /// in flattened [FLOWER_SPAWNS] order — each slot at [slot_index].
    /// Each flower slot is a pivot whose
    /// children, in draw order, are the `flower` leaf and a tomato pivot on
    /// top of it, which the slot's [tomato::Tomato] lays out. `layout` owns
    /// the slots' and their leaves' visibility, growth, and tints from that
    /// frame on: a slot's flower grows from zero to full size over
    /// [FLOWER_GROW_TIME] seconds from its staggered [first_bloom_start],
    /// tinted light green to yellow, and its tomato grows from zero to
    /// [tomato::TOMATO_MAX_SCALE] over [tomato::TOMATO_GROW_TIME] seconds
    /// after the flower finishes, its background tinted dark green to red —
    /// and a fully grown fruit stales on the plant's aging clock, which
    /// [Plant::age] advances with or without water: [tomato::STALE_DELAY]
    /// seconds after the fruit's ripe moment, its background modulates from
    /// ripe red to [tomato::TOMATO_STALE] over [tomato::STALE_TIME].
    ///
    /// `node`'s origin is the plant's root joint (plant1's lower joint):
    /// the base rock turns the whole plant around that joint, and its
    /// transform is set to that rotation composed with a translation to
    /// `anchor` — the root joint's position in the node's parent space — so
    /// the joint never moves. The node's own `scale` (set once by the
    /// caller) sizes the whole plant around the root joint. Each slice is
    /// then scaled by its growth factor around its own lower joint, so it
    /// grows out of the joint it attaches to the previous slice through,
    /// bent by its share of the wind, and the next slice sprouts from its
    /// moving upper joint.
    pub fn layout(
        &self,
        node: &mut frost::SceneNode,
        anchor: [f32; 2],
        flower: &frost::Shape,
        tomato: &frost::Shape,
        tomato_fg: &frost::Shape,
    ) {
        // A gentle traveling wind: the base rock and the two joint bends
        // lag each other, and each bend is a little stronger than the last,
        // so the tip of the plant moves the most.
        let base = (self.t * SWAY_FREQ).sin() * SWAY_BASE;
        let bend1 = (self.t * SWAY_FREQ - SWAY_BENDS[0].0).sin() * SWAY_BENDS[0].1;
        let bend2 = (self.t * SWAY_FREQ - SWAY_BENDS[1].0).sin() * SWAY_BENDS[1].1;

        // The growth factor of each slice: 0 at startup, 1 from its own
        // GROW_TIMES entry on, linear in between. All slices grow
        // concurrently.
        let grown = [
            (self.t / GROW_TIMES[0]).min(1.0),
            (self.t / GROW_TIMES[1]).min(1.0),
            (self.t / GROW_TIMES[2]).min(1.0),
            (self.t / GROW_TIMES[3]).min(1.0),
            (self.t / GROW_TIMES[4]).min(1.0),
        ];

        // The plant node anchors the chain: its origin is plant1's lower
        // joint, and the base rock turns everything around that joint. The
        // origin maps to itself under the rotation, so the root joint
        // never moves. The fit scale is constant — the growth is applied
        // to each slice individually below.
        node.transform = frost::Transform::rotate(base)
            .compose(&frost::Transform::translate(anchor[0], anchor[1]));

        // Lay the chain out in the plant's own (unscaled) space: the first
        // slice's lower joint sits at the plant's origin, and each next
        // slice's lower joint sits on the previous slice's upper joint,
        // rotated by that slice's bend. Each slice is scaled by its growth
        // factor around its own lower joint, so it grows out of the joint
        // it attaches to the previous slice through — and the upper joint
        // the next slice attaches to moves with it.
        let mut anchor = [0.0f32, 0.0];
        let mut slice_tf = [frost::Transform::identity(); 5];
        for (i, (link, node)) in self.links.iter().zip(&mut node.children).enumerate() {
            let bend = match i {
                0 => 0.0,
                1 => bend1,
                _ => bend1 + bend2,
            };
            let rot = frost::Transform::rotate(bend);
            let g = grown[i];
            node.transform = frost::Transform::translate(-link.from[0], -link.from[1])
                .compose(&frost::Transform::scale_uniform(g))
                .compose(&rot)
                .compose(&frost::Transform::translate(anchor[0], anchor[1]));
            slice_tf[i] = node.transform;
            // The next anchor: this slice's upper joint, measured from its
            // own lower joint, scaled by the slice's growth, and rotated
            // by the slice's bend.
            let d = [link.to[0] - link.from[0], link.to[1] - link.from[1]];
            let step = rot.apply([g * d[0], g * d[1]]);
            anchor = [anchor[0] + step[0], anchor[1] + step[1]];
        }

        // The four lower slices' blooms, on the flower slots that follow
        // the five slice children, in flattened spawn order. A slot's
        // flower grows from zero to full size over [FLOWER_GROW_TIME]
        // seconds after its slice finishes, and its tomato grows from zero
        // to full size over [TOMATO_GROW_TIME] seconds after the flower
        // finishes; a slot regrown after a harvest into the basket runs
        // the same timetable from its reset. The slot is a pure pivot: it carries no shape, scale,
        // or tint of its own (a tint there would leak onto the tomato),
        // only the translate that lays its origin on the spawn point
        // mapped through the slice's current transform, so the whole bloom
        // sways with the plant.
        let mut slot = 0usize;
        for (i, spawns) in self.tomato_spawn.iter().enumerate() {
            for &pt in spawns {
                let child = &mut node.children[slot_index(slot)];
                let start = self.bloom_start(slot);
                let fg = flower_growth(self.t, start);
                if fg > 0.0 {
                    let p = slice_tf[i].apply(pt);
                    child.transform = frost::Transform::translate(p[0], p[1]);
                }
                // The flower leaf, under the tomato: grows about the
                // spawn point, tinted light green to yellow.
                let flower_leaf = &mut child.children[SLOT_FLOWER];
                if fg > 0.0 {
                    if flower_leaf.shape.is_none() {
                        flower_leaf.shape = Some(flower.clone());
                    }
                    flower_leaf.modulate = flower_color(fg);
                } else {
                    flower_leaf.shape = None;
                }
                flower_leaf.scale = [fg, fg];
                // The tomato pivot, on top of the flower: the slot's
                // [tomato::Tomato] lays it out — its growth about the same
                // point, its two leaves, its tint, and its staleness; a
                // harvested slot's pivot is reparented out of the tree, so
                // it is skipped here, and a pivot sent back from a failed
                // drop gets its stale carry transform reset to the plant's
                // own.
                if !self.tomatoes[slot].is_harvested() {
                    let tomato_pivot = &mut child.children[SLOT_TOMATO];
                    self.tomatoes[slot].layout(
                        tomato_pivot,
                        self.t,
                        self.age,
                        start,
                        tomato,
                        tomato_fg,
                    );
                }
                slot += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-pixel sprite: `Plant::new` only needs the slice shapes'
    /// texture size.
    fn slice() -> frost::Shape {
        frost::Shape::Sprite {
            data: std::sync::Arc::new([0u8; 4]),
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

    /// The pinned first-bloom schedule of the test plants: each slice's
    /// flowers start in slot order, a fixed [BLOOM_GAP_MIN] apart, so
    /// every slot's start — and the plant's [Plant::bloom_time] — is a
    /// known function of [GROW_TIMES] and [BLOOM_GAP_MIN].
    fn pinned_starts() -> [f32; FLOWER_N] {
        let mut starts = [0.0f32; FLOWER_N];
        let mut slot = 0usize;
        for (i, spawns) in FLOWER_SPAWNS.iter().enumerate() {
            for k in 0..spawns.len() {
                starts[slot] = GROW_TIMES[i] + k as f32 * BLOOM_GAP_MIN;
                slot += 1;
            }
        }
        starts
    }

    /// A plant with a settable growth clock and the pinned [pinned_starts]
    /// first-bloom schedule, so the growth and the blooms' boundaries can
    /// be checked without stepping.
    fn plant_at(t: f32) -> Plant {
        let s = slice();
        let mut p = Plant::new([&s, &s, &s, &s, &s]);
        p.first_bloom_start = pinned_starts();
        p.t = t;
        p
    }

    /// A test plant at [Plant::bloom_time] plus one second: every first
    /// bloom fully grown and ripe, the plant complete — the state the
    /// harvest and regrow tests start from.
    fn plant_complete() -> Plant {
        plant_at(plant_at(0.0).bloom_time() + 1.0)
    }

    /// A test plant whose growth and aging clocks both read `t`, watered
    /// through: every slot whose fruit ripened by `t` is stamped at its
    /// ripe moment, so the staleness the tests check is the watered one —
    /// the value the aging clock reproduces while the growth clock runs.
    fn aged_at(t: f32) -> Plant {
        let mut p = plant_at(t);
        p.age = t;
        for slot in 0..FLOWER_N {
            p.tomatoes[slot].stamp_ripe(p.t, p.age, p.bloom_start(slot));
        }
        p
    }

    /// A plant whose bloom `slot` was reset by [Plant::regrow] at clock
    /// `stamp`, so the regrowth's boundaries can be checked without
    /// stepping.
    fn plant_regrown(slot: usize, stamp: f32) -> Plant {
        let mut p = plant_at(stamp);
        p.regrow(slot);
        p
    }

    /// A plant node built the way the example builds it: the five slice
    /// shapes followed by the [FLOWER_N] flower slots, each a pivot holding
    /// a flower leaf and, on top of it, a tomato pivot (background leaf,
    /// foreground leaf).
    fn plant_node() -> frost::SceneNode {
        let s = slice();
        let mut children: Vec<Box<frost::SceneNode>> = (0..SLICE_N)
            .map(|_| {
                Box::new(frost::SceneNode {
                    shape: Some(s.clone()),
                    ..Default::default()
                })
            })
            .collect();
        children.extend((0..FLOWER_N).map(|_| {
            Box::new(frost::SceneNode {
                children: vec![
                    Box::new(frost::SceneNode::default()),
                    Box::new(frost::SceneNode {
                        children: vec![
                            Box::new(frost::SceneNode::default()),
                            Box::new(frost::SceneNode::default()),
                        ],
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            })
        }));
        frost::SceneNode {
            children,
            ..Default::default()
        }
    }

    /// [Plant::grown_layers] counts the [GROW_TIMES] entries the growth
    /// clock has reached — one per slice, from the base up: a layer is
    /// grown on the frame the clock first reaches its entry, and the count
    /// saturates at [GROW_TIMES.len()] once the plant is fully grown.
    #[test]
    fn grown_layers_counts_reached_slice_times() {
        assert_eq!(plant_at(0.0).grown_layers(), 0);
        assert_eq!(plant_at(GROW_TIMES[0]).grown_layers(), 1);
        assert_eq!(plant_at(GROW_TIMES[1] - 0.001).grown_layers(), 1);
        assert_eq!(plant_at(GROW_TIMES[1]).grown_layers(), 2);
        for (k, g) in GROW_TIMES.iter().enumerate() {
            assert_eq!(
                plant_at(*g).grown_layers(),
                k + 1,
                "layer {k} counted wrong at t = {g}"
            );
        }
        assert_eq!(plant_at(FULL_GROW_TIME).grown_layers(), GROW_TIMES.len());
        assert_eq!(plant_at(FULL_GROW_TIME + 100.0).grown_layers(), GROW_TIMES.len());
    }

    /// [Plant::new] staggers each slice's first blooms: exactly one flower
    /// starts the frame the slice is fully grown, and each next starts a
    /// random [BLOOM_GAP_MIN, BLOOM_GAP_MAX) seconds after the previous
    /// one — so within a slice the starts are strictly increasing, the
    /// gaps in range, and the earliest is the slice's [GROW_TIMES] entry,
    /// and the plant's [Plant::bloom_time] is the latest start plus the
    /// flower and tomato growth times.
    #[test]
    fn the_first_blooms_stagger_across_each_slice() {
        let s = slice();
        let p = Plant::new([&s, &s, &s, &s, &s]);
        let mut slot = 0usize;
        for (i, spawns) in FLOWER_SPAWNS.iter().enumerate() {
            let mut starts: Vec<f32> = (slot..slot + spawns.len())
                .map(|k| p.first_bloom_start[k])
                .collect();
            starts.sort_by(f32::total_cmp);
            assert_eq!(
                starts[0],
                GROW_TIMES[i],
                "no flower on slice {i} starts the frame it finishes"
            );
            for w in starts.windows(2) {
                let gap = w[1] - w[0];
                assert!(
                    (BLOOM_GAP_MIN..BLOOM_GAP_MAX).contains(&gap),
                    "the gap {gap} on slice {i} is out of [2, 5)"
                );
            }
            slot += spawns.len();
        }
        let last = *p
            .first_bloom_start
            .iter()
            .max_by(|a, b| f32::total_cmp(a, b))
            .expect("FLOWER_N is non-empty");
        assert_eq!(
            p.bloom_time(),
            last + FLOWER_GROW_TIME + TOMATO_GROW_TIME,
            "bloom_time off the latest start"
        );
    }

    /// [Plant::complete] is false while the plant is still growing — the
    /// slices, and the blooms that open off the finished slices — and true
    /// from the last bloom's full growth on: the latest staggered flower's
    /// tomato, at [Plant::bloom_time]. Being fully grown is not being
    /// complete: the blooms keep growing after the last slice is done.
    #[test]
    fn complete_tracks_the_last_bloom() {
        let t = plant_at(0.0).bloom_time();
        assert!(!plant_at(0.0).complete());
        assert!(!plant_at(FULL_GROW_TIME).complete());
        assert!(!plant_at(t - 0.001).complete());
        assert!(plant_at(t).complete());
        assert!(plant_at(t + 100.0).complete());
    }

    /// [Plant::layer_midpoints] walks the static, fully grown chain — the
    /// sway left out: midpoint `i` is half a slice on from the chain's
    /// running joint, and each next midpoint continues from the previous
    /// slice's upper joint.
    #[test]
    fn layer_midpoints_walks_the_static_chain() {
        let s = slice();
        let mut p = Plant::new([&s, &s, &s, &s, &s]);
        // One unit link per slice: the chain runs along +x, one step at a
        // time, from the root joint at the origin.
        for i in 0..5 {
            p.links[i].from = [i as f32, 0.0];
            p.links[i].to = [i as f32 + 1.0, 0.0];
        }
        assert_eq!(
            p.layer_midpoints(),
            [
                [0.5, 0.0],
                [1.5, 0.0],
                [2.5, 0.0],
                [3.5, 0.0],
                [4.5, 0.0],
            ]
        );
    }

    /// [Plant::layout] grows a slice's blooms, slice by slice from the base
    /// up: a slice's flower starts growing from zero the frame its slice
    /// reaches full size and is full size, yellow, [FLOWER_GROW_TIME]
    /// seconds later; its tomato starts when the flower is done and is
    /// ripe, red, at [TOMATO_MAX_SCALE] of its natural size,
    /// [TOMATO_GROW_TIME] seconds after that.
    #[test]
    fn flowers_and_tomatoes_grow_as_each_slice_finishes_growing() {
        let s = slice();
        let mut node = plant_node();

        // Before the base slice finishes, nothing has started to grow:
        // every slot's flower and tomato leaves are shapeless, at zero
        // growth.
        let p = plant_at(GROW_TIMES[0] - 0.001);
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        for c in node.children.iter().skip(SLICE_N) {
            let f = &c.children[SLOT_FLOWER];
            let t = &c.children[SLOT_TOMATO];
            assert!(f.shape.is_none(), "a flower started early");
            assert_eq!(f.scale, [0.0, 0.0]);
            assert_eq!(t.scale, [0.0, 0.0]);
            for leaf in t.children.iter() {
                assert!(leaf.shape.is_none(), "a tomato started early");
            }
        }

        // Mid flower growth, the flower is present but half sized, tinted
        // between green and yellow; the tomato has not started.
        let p = plant_at(GROW_TIMES[0] + FLOWER_GROW_TIME / 2.0);
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        let f = &node.children[slot_index(0)].children[SLOT_FLOWER];
        assert!(f.shape.is_some());
        assert!((f.scale[0] - 0.5).abs() < 1e-6, "flower scale mid-growth");
        let t = &node.children[slot_index(0)].children[SLOT_TOMATO];
        assert_eq!(t.scale, [0.0, 0.0], "the tomato started early");
        for leaf in t.children.iter() {
            assert!(leaf.shape.is_none());
        }

        // At full flower growth the flower is full size and yellow; the
        // tomato has not started.
        let bloom = GROW_TIMES[0] + FLOWER_GROW_TIME;
        let p = plant_at(bloom);
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        let f = &node.children[slot_index(0)].children[SLOT_FLOWER];
        assert_eq!(f.scale, [1.0, 1.0]);
        assert_eq!(f.modulate, FLOWER_BLOOM, "flower color at full growth");
        let t = &node.children[slot_index(0)].children[SLOT_TOMATO];
        assert_eq!(t.scale, [0.0, 0.0]);

        // Mid tomato growth, both tomato leaves are present and half sized;
        // the background is tinted between green and red.
        let p = plant_at(bloom + TOMATO_GROW_TIME / 2.0);
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        let t = &node.children[slot_index(0)].children[SLOT_TOMATO];
        assert!(
            (t.scale[0] - TOMATO_MAX_SCALE / 2.0).abs() < 1e-6,
            "tomato scale mid-growth"
        );
        assert!(t.children[TOMATO_BG].shape.is_some());
        assert!(t.children[TOMATO_FG].shape.is_some());

        // At full tomato growth the tomato is ripe — red, at
        // [TOMATO_MAX_SCALE] of its natural size — and the foreground is
        // unmodulated.
        let p = plant_at(bloom + TOMATO_GROW_TIME);
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        let t = &node.children[slot_index(0)].children[SLOT_TOMATO];
        assert_eq!(t.scale, [TOMATO_MAX_SCALE, TOMATO_MAX_SCALE]);
        assert_eq!(
            t.children[TOMATO_BG].modulate,
            TOMATO_RED,
            "tomato color at full growth"
        );
        assert_eq!(
            t.children[TOMATO_FG].modulate,
            frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0
            }
        );
    }

    /// [Plant::layout] lays each flower on its slice's spawn point mapped
    /// through the slice's current transform: at a swayed, fully grown
    /// plant every flower's laid-out position equals its slice's transform
    /// applied to the spawn point, and one frame later the wind has moved
    /// the plant — the flowers moved with it.
    #[test]
    fn flowers_ride_their_slices_through_the_sway() {
        let s = slice();
        // Past the latest staggered start: every flower is up, so each
        // can be checked against its slice's spawn point.
        let t = plant_at(0.0).bloom_time() + 1.0;
        let p = plant_at(t);
        let mut node = plant_node();
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);

        // Each flower sits on its slice: the child's translate is the
        // slice's transform applied to the spawn point.
        let mut offset = 0usize;
        for (i, spawns) in p.tomato_spawn.iter().enumerate() {
            for &pt in spawns {
                let flower = node.children[slot_index(offset)].transform.apply([0.0, 0.0]);
                let on_slice = node.children[i].transform.apply(pt);
                assert!(
                    (flower[0] - on_slice[0]).abs() <= 1e-4
                        && (flower[1] - on_slice[1]).abs() <= 1e-4,
                    "flower slot {} of slice {i} off its spawn point",
                    offset
                );
                offset += 1;
            }
        }
        assert_eq!(offset, FLOWER_N);

        // One frame later the wind has turned the plant — the base flower's
        // composed position (node transform then slot transform) moved.
        let p = plant_at(t + 1.0);
        let mut node2 = plant_node();
        p.layout(&mut node2, [0.0, 0.0], &s, &s, &s);
        let composed = |node: &frost::SceneNode| {
            node.transform.apply(node.children[slot_index(0)].transform.apply([0.0, 0.0]))
        };
        let a = composed(&node);
        let b = composed(&node2);
        assert!(
            (a[0] - b[0]).abs() + (a[1] - b[1]).abs() > 1e-3,
            "the base flower did not move with the sway"
        );
    }

    /// [Plant::tomato_center] puts the tomato's body center in the plant
    /// node's local space, at full growth: the [TOMATO_TOP] stem point,
    /// pinned to the slot origin, shifted by the pivot scale times the
    /// [TOMATO_TOP] leaf offset — and for the 638×469 tomato image that
    /// offset is (4, −144.5), so the body hangs 144.5 px below the stem.
    #[test]
    fn tomato_center_hangs_the_body_below_the_stem() {
        // A shapeless stand-in with the real tomato image's size: only the
        // width and height feed the offset.
        let tomato = frost::Shape::Sprite {
            data: std::sync::Arc::new([0u8; 4]),
            width: 638,
            height: 469,
            color: frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            alpha: 1.0,
        };
        let [ox, oy] = tomato_leaf_offset([638.0, 469.0]);
        assert!((ox - 4.0).abs() < 1e-6, "stem x offset");
        assert!((oy + 144.5).abs() < 1e-6, "stem y offset");

        let p = plant_complete();
        let slot_tf = frost::Transform::translate(10.0, -20.0);
        let slot_node = frost::SceneNode {
            transform: slot_tf,
            ..Default::default()
        };
        let c = p.tomato_center(0, &slot_node, &tomato);
        let want = slot_tf.apply([
            ox * TOMATO_MAX_SCALE,
            oy * TOMATO_MAX_SCALE,
        ]);
        assert!(
            (c[0] - want[0]).abs() < 1e-6 && (c[1] - want[1]).abs() < 1e-6,
            "body center off the offset: got {c:?}, want {want:?}"
        );
    }

    /// [Plant::layout] skips a harvested slot's tomato pivot — which the
    /// caller has reparented out of the tree — so it may run with the slot
    /// down to its single flower leaf child, and it leaves the other
    /// slots' pivots alone.
    #[test]
    fn layout_skips_a_harvested_slot() {
        let s = slice();
        let mut node = plant_node();
        let mut p = plant_complete();

        // Pick slot 0: mark it harvested and take its pivot out of the
        // tree, the way the example's pick does.
        p.harvest(0);
        let _pivot = node.children[slot_index(0)].children.remove(SLOT_TOMATO);

        // The layout runs with the pivot gone and does not touch the
        // picked slot's remaining child.
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        assert_eq!(
            node.children[slot_index(0)].children.len(),
            1,
            "layout touched the picked slot"
        );
        for c in node.children.iter().skip(slot_index(1)) {
            assert_eq!(c.children.len(), 2, "a whole slot lost a child");
        }
    }

    /// A pivot sent back from a failed drop is reposed by the next
    /// [Plant::layout]: its stale carry transform is reset to the
    /// identity, and its growth is restored.
    #[test]
    fn layout_reposes_a_sent_back_pivot() {
        let s = slice();
        let mut node = plant_node();
        let mut p = plant_complete();

        // Pick slot 0, carry the pivot with a stale transform, send it
        // back, and clear the harvested mark — the failed-drop path.
        p.harvest(0);
        let mut pivot = *node.children[slot_index(0)].children.remove(SLOT_TOMATO);
        pivot.transform = frost::Transform::translate(123.0, -45.0);
        pivot.scale = [0.15, 0.15];
        node.children[slot_index(0)].children.push(Box::new(pivot));
        p.unharvest(0);

        // The layout resets the pivot to the plant's own pose: the
        // transform is the identity again, and the scale is the
        // full-growth scale.
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        let t = &node.children[slot_index(0)].children[SLOT_TOMATO];
        let o = t.transform.apply([0.0, 0.0]);
        let ex = t.transform.apply([1.0, 0.0]);
        let ey = t.transform.apply([0.0, 1.0]);
        assert!(
            o[0].abs() < 1e-6 && o[1].abs() < 1e-6,
            "the carry translation survived: {o:?}"
        );
        assert!(
            (ex[0] - 1.0).abs() < 1e-6 && ex[1].abs() < 1e-6
                && (ey[0]).abs() < 1e-6 && (ey[1] - 1.0).abs() < 1e-6,
            "the carry rotation or scale survived: {ex:?} {ey:?}"
        );
        assert_eq!(
            t.scale,
            [TOMATO_MAX_SCALE, TOMATO_MAX_SCALE],
            "the growth was not restored"
        );
    }

    /// [Plant::ripe] follows the regrown slot's restarted schedule — it is
    /// false from the [Plant::regrow] stamp and true again [FLOWER_GROW_TIME]
    /// plus [TOMATO_GROW_TIME] after it — while the other slots keep their
    /// first-bloom ripeness.
    #[test]
    fn ripe_follows_the_regrown_schedule() {
        let r = plant_at(0.0).bloom_time() + 1.0;
        let done = r + FLOWER_GROW_TIME + TOMATO_GROW_TIME;
        let mut p = plant_regrown(2, r);

        assert!(p.ripe(0), "an untouched slot stays ripe");
        assert!(!p.ripe(2), "the regrown slot is ripe right away");
        p.step(done - r - 0.001);
        assert!(!p.ripe(2), "the regrown slot ripens early");
        p.step(0.001);
        assert!(p.ripe(2), "the regrown slot ripens on schedule");
    }

    /// [Plant::complete] is false while a regrown bloom is still growing and
    /// true from its second tomato's full growth — the [Plant::regrow] stamp
    /// plus [FLOWER_GROW_TIME] plus [TOMATO_GROW_TIME] — the gate the
    /// example's growth loop and watering hitbox run on.
    #[test]
    fn complete_waits_for_regrown_blooms() {
        let r = plant_at(0.0).bloom_time() + 1.0;
        let done = r + FLOWER_GROW_TIME + TOMATO_GROW_TIME;
        let mut p = plant_regrown(2, r);

        assert!(!p.complete(), "complete right after the regrow");
        p.step(done - r - 0.001);
        assert!(!p.complete(), "complete before the regrown tomato is full");
        p.step(0.001);
        assert!(p.complete(), "the regrown bloom finishes on schedule");
    }

    /// [Plant::regrow] restarts a slot's bloom after its tomato has been
    /// placed in the basket: the next [Plant::layout] removes the flower
    /// and the fresh, shapeless tomato pivot starts at zero; the flower
    /// regrows from zero over [FLOWER_GROW_TIME], the tomato from zero over
    /// [TOMATO_GROW_TIME] after it, and the slot is ripe again and the plant
    /// complete when the second tomato is full — the other slots untouched.
    #[test]
    fn regrow_removes_the_flower_and_restarts_the_bloom() {
        let s = slice();
        let mut node = plant_node();
        let mut p = plant_complete();
        assert!(p.complete(), "the plant starts complete");

        // Harvest slot 0 into the basket: mark it and take its pivot out of
        // the tree, the way the pick does, then reset the bloom and give
        // the slot a fresh, shapeless tomato pivot, the way the example
        // does.
        p.harvest(0);
        let _pivot = *node.children[slot_index(0)].children.remove(SLOT_TOMATO);
        p.regrow(0);
        node.children[slot_index(0)].children.push(Box::new(frost::SceneNode {
            children: vec![
                Box::new(frost::SceneNode::default()),
                Box::new(frost::SceneNode::default()),
            ],
            ..Default::default()
        }));

        // At the reset moment the flower is gone and the tomato has not
        // started — the bloom is removed.
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        let f = &node.children[slot_index(0)].children[SLOT_FLOWER];
        let t = &node.children[slot_index(0)].children[SLOT_TOMATO];
        assert!(f.shape.is_none(), "the flower survived the harvest");
        assert_eq!(f.scale, [0.0, 0.0]);
        assert_eq!(t.scale, [0.0, 0.0]);
        for leaf in t.children.iter() {
            assert!(leaf.shape.is_none(), "a regrown leaf is shaped early");
        }
        assert!(!p.ripe(0), "the regrown fruit ripens early");
        assert!(!p.complete(), "the plant is complete while regrowing");
        // The other slots keep their first-bloom state.
        let f1 = &node.children[slot_index(1)].children[SLOT_FLOWER];
        assert!(f1.shape.is_some());
        assert_eq!(f1.scale, [1.0, 1.0]);

        // Mid flower growth, the flower is back at half size and the tomato
        // has not started.
        p.step(FLOWER_GROW_TIME / 2.0);
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        let f = &node.children[slot_index(0)].children[SLOT_FLOWER];
        assert!(f.shape.is_some(), "the flower did not come back");
        assert!((f.scale[0] - 0.5).abs() < 1e-6, "the flower is not half grown");
        assert_eq!(
            node.children[slot_index(0)].children[SLOT_TOMATO].scale,
            [0.0, 0.0],
            "the tomato starts before the flower is full"
        );

        // The tomato starts when the flower is full and ripens
        // [TOMATO_GROW_TIME] after, red at full size.
        p.step(FLOWER_GROW_TIME / 2.0);
        assert!(!p.ripe(0), "the regrown fruit ripens early");
        p.step(TOMATO_GROW_TIME / 2.0);
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        let t = &node.children[slot_index(0)].children[SLOT_TOMATO];
        assert!(
            (t.scale[0] - TOMATO_MAX_SCALE / 2.0).abs() < 1e-6,
            "the regrown tomato is not half grown"
        );
        assert!(t.children[TOMATO_BG].shape.is_some());
        p.step(TOMATO_GROW_TIME / 2.0);
        assert!(p.ripe(0), "the regrown fruit does not ripen");
        assert!(p.complete(), "the plant does not complete again");
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        let t = &node.children[slot_index(0)].children[SLOT_TOMATO];
        assert_eq!(t.scale, [TOMATO_MAX_SCALE, TOMATO_MAX_SCALE]);
        assert_eq!(t.children[TOMATO_BG].modulate, TOMATO_RED);
    }

    /// A fully grown tomato holds its ripe red for [STALE_DELAY] seconds of
    /// the aging clock, then modulates from red to [TOMATO_STALE] over
    /// [STALE_TIME] and holds there — and a regrown fruit stales on its new
    /// schedule, not the old one's.
    #[test]
    fn a_ripe_tomato_stales() {
        let s = slice();
        let mut node = plant_node();
        let ripe_at = plant_at(0.0).bloom_start(0) + FLOWER_GROW_TIME + TOMATO_GROW_TIME;
        for (dt, st) in [
            (0.0, 0.0),
            (STALE_DELAY * 0.99, 0.0),
            (STALE_DELAY + 0.001, 0.001 / STALE_TIME),
            (STALE_DELAY + STALE_TIME * 0.5, 0.5),
            (STALE_DELAY + STALE_TIME, 1.0),
            (STALE_DELAY + STALE_TIME + 50.0, 1.0),
        ] {
            let p = aged_at(ripe_at + dt);
            p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
            let bg = &node.children[slot_index(0)].children[SLOT_TOMATO].children[TOMATO_BG];
            let want = mix(TOMATO_RED, TOMATO_STALE, st);
            assert!(
                (bg.modulate.r - want.r).abs() < 1e-6
                    && (bg.modulate.g - want.g).abs() < 1e-6
                    && (bg.modulate.b - want.b).abs() < 1e-6,
                "t = {dt}: modulate {:?}, want {want:?}",
                bg.modulate
            );
        }

        // A regrown fruit stales on its new schedule: at its regrown full
        // growth, well past where the old fruit would be fully stale, its
        // background is ripe red again — the ripe stamp cleared by the
        // regrow, so no stale has started on the new fruit.
        let mut p = aged_at(ripe_at);
        p.regrow(0);
        p.t = ripe_at + FLOWER_GROW_TIME + TOMATO_GROW_TIME;
        p.age = p.t;
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        let bg = &node.children[slot_index(0)].children[SLOT_TOMATO].children[TOMATO_BG];
        assert_eq!(
            bg.modulate, TOMATO_RED,
            "the regrown fruit stales on the old schedule"
        );
    }

    /// A ripe fruit waits [STALE_DELAY] and stales over [STALE_TIME] on
    /// the aging clock alone — the growth clock frozen on a dry reserve:
    /// the wait and the stale need no water, and [Plant::overgrown] turns
    /// on at the end of the stale period.
    #[test]
    fn a_ripe_tomato_stales_on_a_dry_plant() {
        let s = slice();
        let mut node = plant_node();
        let ripe_at = plant_at(0.0).bloom_start(0) + FLOWER_GROW_TIME + TOMATO_GROW_TIME;
        let stale_done = STALE_DELAY + STALE_TIME;
        for (dt, st) in [
            (0.0, 0.0),
            (STALE_DELAY * 0.99, 0.0),
            (STALE_DELAY + 0.001, 0.001 / STALE_TIME),
            (STALE_DELAY + STALE_TIME * 0.5, 0.5),
            (stale_done, 1.0),
            (stale_done + 50.0, 1.0),
        ] {
            let mut p = aged_at(ripe_at);
            // The reserve is dry: the growth clock never advances again —
            // only the aging clock runs.
            p.age = ripe_at + dt;
            p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
            let bg = &node.children[slot_index(0)].children[SLOT_TOMATO].children[TOMATO_BG];
            let want = mix(TOMATO_RED, TOMATO_STALE, st);
            assert!(
                (bg.modulate.r - want.r).abs() < 1e-6
                    && (bg.modulate.g - want.g).abs() < 1e-6
                    && (bg.modulate.b - want.b).abs() < 1e-6,
                "dt = {dt}: modulate {:?}, want {want:?}",
                bg.modulate
            );
            assert_eq!(
                p.overgrown(0),
                dt >= stale_done,
                "overgrown at dt = {dt}"
            );
        }
    }

    /// [Plant::age] stamps the ripe moment exactly, whatever the frame
    /// size: the stamp is the aging clock's value at the fruit's ripe
    /// moment on the growth clock, so a frame that crosses the ripe moment
    /// mid-way stamps the mid-way value, not the frame's end.
    #[test]
    fn age_stamps_the_ripe_moment_mid_frame() {
        let ripe_at = plant_at(0.0).bloom_start(0) + FLOWER_GROW_TIME + TOMATO_GROW_TIME;
        let mut p = plant_at(ripe_at - 1.0);
        p.age = ripe_at - 1.0;
        // One big frame crosses the ripe moment: the growth clock and the
        // aging clock both advance by five.
        p.step(5.0);
        p.age(5.0);
        assert!(
            (p.tomatoes[0].ripened_at().expect("the fruit ripened this frame") - ripe_at).abs()
                < 1e-6,
            "the stamp is the ripe moment, not the frame's end"
        );
    }

    /// [Plant::overgrown] turns on the frame the fruit passes the full
    /// stale period on the aging clock — its ripe moment plus
    /// [STALE_DELAY] and [STALE_TIME] — and stays on: that is the moment
    /// the example drops the tomato and regrows the slot.
    #[test]
    fn overgrown_is_the_end_of_the_stale_period() {
        let start = plant_at(0.0).bloom_start(0);
        let over_at =
            start + FLOWER_GROW_TIME + TOMATO_GROW_TIME + STALE_DELAY + STALE_TIME;
        assert!(!aged_at(over_at - 0.001).overgrown(0), "not yet fully stale");
        assert!(aged_at(over_at).overgrown(0));
        assert!(aged_at(over_at + 100.0).overgrown(0), "holds past the period");
    }

    /// [Plant::overgrown] follows the [Plant::regrow]-restarted schedule:
    /// a slot that is overgrown on its first bloom is not overgrown right
    /// after the reset, and turns overgrown again only after its new
    /// bloom's full growth and stale period — the ripe stamp clears with
    /// the regrow, and the new fruit overgrows from its own ripe moment.
    #[test]
    fn overgrown_follows_the_regrown_schedule() {
        let cycle =
            FLOWER_GROW_TIME + TOMATO_GROW_TIME + STALE_DELAY + STALE_TIME;
        let over_at = plant_at(0.0).bloom_start(0) + cycle;
        let mut p = aged_at(over_at);
        assert!(p.overgrown(0), "the first bloom is fully overgrown");
        p.regrow(0);
        assert!(!p.overgrown(0), "the regrown bloom restarts its cycle");
        p.t = over_at + cycle;
        p.age = p.t;
        p.tomatoes[0].stamp_ripe(p.t, p.age, p.bloom_start(0));
        assert!(p.overgrown(0), "the new bloom overgrows on its own schedule");
    }

    /// [Plant::fall_height] is the cumulative vertical span of the
    /// segments from the root down to the slice: a tomato on the root
    /// segment falls one segment's height (93 node-local px), one on the
    /// second the sum of the first two (93 + 230), and so on — before the
    /// caller's fit scale.
    #[test]
    fn fall_height_sums_the_segments_from_the_root() {
        let p = plant_at(0.0);
        // Cross-checked against the hand-picked joint table.
        let mut sum = 0.0;
        for slice in 0..4 {
            sum += JOINTS[slice][0].1 - JOINTS[slice][1].1;
            assert!((p.fall_height(slice) - sum).abs() < 1e-6, "slice {slice}");
        }
        let want = [93.0, 323.0, 697.0, 943.0];
        for slice in 0..4 {
            assert!((p.fall_height(slice) - want[slice]).abs() < 1e-6, "slice {slice}");
        }
    }
}
