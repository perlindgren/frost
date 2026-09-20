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
//! Bugs are spawned into a fixed-size pool: the scene carries one
//! shape-less child per slot, and [Bugs::layout] drives only the spawned
//! prefix, so unspawned slots never draw. Like the vipers module, no random
//! crate is needed — a small splitmix64 [Rng] seeded from the clock
//! decides spawn points, speeds and wobbles.

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
    /// The frame last written into the node; the shape is swapped only when
    /// the two differ.
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
}

/// The whole swarm: the spawned prefix of the scene's bug slot pool.
pub struct Bugs {
    /// The spawned bugs, in spawn order.
    bugs: Vec<Bug>,
    /// Total slots in the scene pool; the population cap.
    max: usize,
    /// How many plants have received their batch of bugs.
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
    /// `max` is the number of scene slots (BUGS_PER_PLANT times the plant
    /// count); the population only ever grows toward it.
    pub fn new(frames: [&frost::Shape; 3], max: usize) -> Self {
        let (mut w, mut h) = (0.0f32, 0.0f32);
        for frame in frames {
            let [fw, fh] = sprite_size(frame);
            w = w.max(fw);
            h = h.max(fh);
        }
        let scale = BUG_SIZE / w;
        Self {
            bugs: Vec::new(),
            max,
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
    pub fn step(&mut self, dt: f32, anchors: &[[f32; 2]], active: usize) {
        if dt < 0.0 {
            return;
        }
        self.t += dt;

        let want = (BUGS_PER_PLANT * active.min(anchors.len())).min(self.max);
        while self.bugs.len() < want {
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
                };
                self.bugs.push(bug);
            }
        }

        for bug in &mut self.bugs {
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
                [
                    target[0] + (self.t * bug.wob_freq + bug.wob_phase).sin() * 1.5,
                    target[1],
                ]
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
    }

    /// Write the spawned prefix of `node`'s children.
    ///
    /// Each slot's transform is the bug position, its scale is the uniform
    /// sprite scale with the x component carrying the facing flip and the
    /// y component the 0→1 spawn growth, its modulate is the tint, and its
    /// shape swaps only when the walk frame changed. Unspawned slots are
    /// left untouched (shape-less, so they draw nothing).
    pub fn layout(&mut self, node: &mut SceneNode, frames: [&frost::Shape; 3]) {
        for (bug, child) in self.bugs.iter_mut().zip(&mut node.children) {
            let g = bug.grow / GROW_TIME;
            child.transform = frost::Transform::translate(bug.pos[0], bug.pos[1]);
            child.scale = [bug.facing * self.scale, self.scale * g];
            child.modulate = TINT;
            if bug.shown != bug.frame {
                child.shape = Some(frames[bug.frame as usize].clone());
                bug.shown = bug.frame;
            }
        }
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
        let mut bugs = Bugs::new([&frame; 3], 3);
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
}
