//! 2D collision math: shapes, overlap tests, and push-out resolution.
//!
//! Pure math — no winit, no wgpu, no state — so it is unit-tested without a
//! GPU and compiles identically for `wasm32-unknown-unknown`. Coordinates
//! are the same window-centered, y-up space the rest of frost draws in: x to
//! the right, y up, angles counter-clockwise in radians.
//!
//! This module answers questions; it owns no world and does no integration.
//! The game's [`Process`](crate::Process) keeps the positions and
//! velocities, builds a [`Collider`] from that state each frame, applies
//! [`Collider::push_out`] together with [`reflect`], and writes the new
//! position back into the scene node's transform.
//!
//! When a game outgrows this — stacking, joints, ragdolls, dozens of
//! mutually dynamic bodies that need a broadphase, or continuous collision
//! for very fast projectiles — the replacement is `rapier2d`: move the state
//! into its rigid bodies and swap these two call sites; the scene bridge and
//! the rendering stay the same.

/// The f32 epsilon: shapes whose gap is this or larger are touching, not
/// overlapping.
const EPS: f32 = 1e-6;

/// The dot product of two 2D vectors.
fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}

/// The length of a 2D vector.
fn len(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

/// A box centered at `center` with half-extents `half`, rotated
/// counter-clockwise by `angle` radians about its center.
///
/// `half` uses the same convention as `Canvas::rectangle` (dx/dy): the
/// full size of the box is `2 * half`. An `angle` of `0.0` is axis-aligned.
#[derive(Clone, Copy, Debug)]
pub struct OrientedBox {
    /// The box's center in window-centered pixels.
    pub center: [f32; 2],
    /// Half the box's extent along each of its own axes.
    pub half: [f32; 2],
    /// Rotation in radians, counter-clockwise from the axis-aligned pose.
    pub angle: f32,
}

impl OrientedBox {
    /// An axis-aligned box.
    pub fn new(center: [f32; 2], half: [f32; 2]) -> Self {
        Self { center, half, angle: 0.0 }
    }

    /// A box rotated counter-clockwise by `angle` radians.
    pub fn rotated(center: [f32; 2], half: [f32; 2], angle: f32) -> Self {
        Self { center, half, angle }
    }

    /// The box's unit axes in world coordinates: `axis(0)` along the box's
    /// own x (the `half[0]` extent) and `axis(1)` along its own y.
    pub fn axis(&self, i: usize) -> [f32; 2] {
        let (sin, cos) = self.angle.sin_cos();
        if i == 0 {
            [cos, sin]
        } else {
            [-sin, cos]
        }
    }

    /// The box's four corners in world coordinates, counter-clockwise from
    /// the box's own lower-left corner.
    pub fn corners(&self) -> [[f32; 2]; 4] {
        let (sin, cos) = self.angle.sin_cos();
        let [hw, hh] = self.half;
        [[-hw, -hh], [-hw, hh], [hw, hh], [hw, -hh]].map(|[x, y]| {
            [
                self.center[0] + x * cos - y * sin,
                self.center[1] + x * sin + y * cos,
            ]
        })
    }
}

/// A circle.
#[derive(Clone, Copy, Debug)]
pub struct Circle {
    /// The circle's center in window-centered pixels.
    pub center: [f32; 2],
    /// The circle's radius in pixels.
    pub radius: f32,
}

/// A collision shape.
#[derive(Clone, Copy, Debug)]
pub enum Collider {
    /// A (possibly rotated) box.
    Box(OrientedBox),
    /// A circle.
    Circle(Circle),
}

/// The minimum translation that separates two overlapping shapes: move the
/// shape by `dir * depth` and it no longer overlaps the other.
#[derive(Clone, Copy, Debug)]
pub struct PushOut {
    /// A unit vector, pointing the way the shape should be moved — away
    /// from the other shape.
    pub dir: [f32; 2],
    /// The distance to move along `dir`.
    pub depth: f32,
}

impl Collider {
    /// Whether the two shapes overlap: `true` when one intrudes into the
    /// other's interior by more than the f32 epsilon, `false` when they are
    /// apart or merely touching.
    pub fn intersects(&self, other: &Self) -> bool {
        self.push_out(other).is_some()
    }

    /// The minimum translation vector that pushes `self` out of `other`, or
    /// `None` when they do not overlap.
    pub fn push_out(&self, other: &Self) -> Option<PushOut> {
        match (self, other) {
            (Collider::Box(a), Collider::Box(b)) => box_push_out(a, b),
            (Collider::Circle(a), Collider::Circle(b)) => circle_push_out(a, b),
            (Collider::Circle(a), Collider::Box(b)) => circle_box_push_out(a, b),
            (Collider::Box(a), Collider::Circle(b)) => {
                // Push the circle out of the box, then flip: the relative
                // separation is symmetric, so the box moves the other way by
                // the same amount.
                circle_box_push_out(b, a).map(|po| PushOut {
                    dir: [-po.dir[0], -po.dir[1]],
                    depth: po.depth,
                })
            }
        }
    }
}

/// The box–box minimum translation vector, by the separating-axis theorem:
/// two boxes overlap only when their projections overlap on every one of
/// the four candidate axes (two per box), and the axis of least penetration
/// is the minimum translation that separates them.
fn box_push_out(a: &OrientedBox, b: &OrientedBox) -> Option<PushOut> {
    let axes = [a.axis(0), a.axis(1), b.axis(0), b.axis(1)];
    let mut best: Option<([f32; 2], f32)> = None; // (axis, penetration)

    for &axis in &axes {
        let ra = a.half[0] * dot(a.axis(0), axis).abs() + a.half[1] * dot(a.axis(1), axis).abs();
        let rb = b.half[0] * dot(b.axis(0), axis).abs() + b.half[1] * dot(b.axis(1), axis).abs();
        let d = dot([b.center[0] - a.center[0], b.center[1] - a.center[1]], axis);
        let pen = ra + rb - d.abs();
        if pen <= EPS {
            return None; // A separating axis: no overlap.
        }
        match best {
            Some((_, p)) if p <= pen => {}
            _ => best = Some((axis, pen)),
        }
    }

    let (axis, pen) = best?; // Every axis overlapped.
    // Orient the axis so it points from `b` toward `a`: when `b` lies on
    // the + side of `a` along the axis, `a` must be pushed along −.
    let s = dot([b.center[0] - a.center[0], b.center[1] - a.center[1]], axis);
    let dir = if s > 0.0 {
        [-axis[0], -axis[1]]
    } else {
        axis
    };
    Some(PushOut { dir, depth: pen })
}

/// The circle–circle minimum translation vector.
fn circle_push_out(a: &Circle, b: &Circle) -> Option<PushOut> {
    let d = [a.center[0] - b.center[0], a.center[1] - b.center[1]];
    let dist = len(d);
    let pen = a.radius + b.radius - dist;
    if pen <= EPS {
        return None;
    }
    let dir = if dist > EPS {
        [d[0] / dist, d[1] / dist]
    } else {
        [1.0, 0.0] // Concentric: a deterministic, arbitrary axis.
    };
    Some(PushOut { dir, depth: pen })
}

/// The circle–box minimum translation vector, pushing the circle out of the
/// box. The circle's center is transformed into the box's own frame, where
/// the box is an axis-aligned square about the origin.
fn circle_box_push_out(c: &Circle, b: &OrientedBox) -> Option<PushOut> {
    let (sin, cos) = b.angle.sin_cos();
    let rel = [c.center[0] - b.center[0], c.center[1] - b.center[1]];
    // Rotate `rel` by `-angle` into the box's frame.
    let lx = rel[0] * cos + rel[1] * sin;
    let ly = -rel[0] * sin + rel[1] * cos;

    let (dir_local, depth) = if lx.abs() <= b.half[0] && ly.abs() <= b.half[1] {
        // The center is inside the box: the circle is clear only once it has
        // moved entirely past the nearest face.
        let dx = b.half[0] - lx.abs();
        let dy = b.half[1] - ly.abs();
        let sx = if lx < 0.0 { -1.0 } else { 1.0 };
        let sy = if ly < 0.0 { -1.0 } else { 1.0 };
        if dx <= dy {
            ([sx, 0.0], c.radius + dx)
        } else {
            ([0.0, sy], c.radius + dy)
        }
    } else {
        // The center is outside: push away from the closest point on the box.
        let cx = lx.clamp(-b.half[0], b.half[0]);
        let cy = ly.clamp(-b.half[1], b.half[1]);
        let d = [lx - cx, ly - cy];
        let dist = len(d);
        let pen = c.radius - dist;
        if pen <= EPS {
            return None;
        }
        let dir = if dist > EPS {
            [d[0] / dist, d[1] / dist]
        } else {
            [1.0, 0.0]
        };
        (dir, pen)
    };

    // Rotate the local direction by `+angle` back to world.
    Some(PushOut {
        dir: [
            dir_local[0] * cos - dir_local[1] * sin,
            dir_local[0] * sin + dir_local[1] * cos,
        ],
        depth,
    })
}

/// Reflect a velocity about a collision normal, with restitution.
///
/// `n` is the unit collision normal, pointing the way the body was pushed —
/// away from the surface it hit (see [`PushOut::dir`]). `e` is the
/// restitution in `[0, 1]`: `0.0` kills the velocity's normal component (the
/// body slides along the surface), `1.0` is a perfectly elastic bounce. A
/// velocity already moving away from the surface is returned unchanged.
pub fn reflect(vel: [f32; 2], n: [f32; 2], e: f32) -> [f32; 2] {
    let vn = dot(vel, n);
    if vn >= 0.0 {
        return vel;
    }
    let k = (1.0 + e) * vn;
    [vel[0] - k * n[0], vel[1] - k * n[1]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, SQRT_2};

    /// Whether two 2D vectors are within `eps` component-wise.
    fn close(a: [f32; 2], b: [f32; 2], eps: f32) -> bool {
        (a[0] - b[0]).abs() < eps && (a[1] - b[1]).abs() < eps
    }

    // -- OrientedBox basics --

    #[test]
    fn unrotated_box_axes_and_corners() {
        let b = OrientedBox::new([10.0, 20.0], [30.0, 40.0]);
        assert_eq!(b.axis(0), [1.0, 0.0]);
        assert_eq!(b.axis(1), [0.0, 1.0]);
        assert_eq!(
            b.corners(),
            [[-20.0, -20.0], [-20.0, 60.0], [40.0, 60.0], [40.0, -20.0]]
        );
    }

    #[test]
    fn quarter_turn_rotates_box() {
        let b = OrientedBox::rotated([0.0, 0.0], [10.0, 20.0], FRAC_PI_2);
        assert!(close(b.axis(0), [0.0, 1.0], 1e-5));
        assert!(close(b.axis(1), [-1.0, 0.0], 1e-5));
        let expected = [[20.0, -10.0], [-20.0, -10.0], [-20.0, 10.0], [20.0, 10.0]];
        for (got, want) in b.corners().iter().zip(expected.iter()) {
            assert!(close(*got, *want, 1e-4));
        }
    }

    // -- Box–box --

    #[test]
    fn aabb_overlap_and_miss() {
        let a = Collider::Box(OrientedBox::new([0.0, 0.0], [50.0, 50.0]));
        let near = Collider::Box(OrientedBox::new([80.0, 0.0], [50.0, 50.0]));
        assert!(a.intersects(&near)); // a spans ±50, near spans 30..130.
        let far = Collider::Box(OrientedBox::new([100.1, 0.0], [50.0, 50.0]));
        assert!(!a.intersects(&far)); // A 0.1 gap.
    }

    #[test]
    fn exact_touching_is_not_overlap() {
        let a = Collider::Box(OrientedBox::new([0.0, 0.0], [50.0, 50.0]));
        let b = Collider::Box(OrientedBox::new([100.0, 0.0], [50.0, 50.0]));
        assert!(!a.intersects(&b));
    }

    #[test]
    fn subpixel_gap_is_not_overlap() {
        let a = Collider::Box(OrientedBox::new([0.0, 0.0], [1.0, 1.0]));
        let b = Collider::Box(OrientedBox::new([1.999999, 0.0], [1.0, 1.0]));
        assert!(!a.intersects(&b)); // A ~1e-6 gap: within the epsilon.
    }

    #[test]
    fn aabb_push_out_from_four_sides() {
        let a = Collider::Box(OrientedBox::new([0.0, 0.0], [10.0, 10.0]));
        let from_right = Collider::Box(OrientedBox::new([15.0, 0.0], [10.0, 10.0]));
        let po = a.push_out(&from_right).unwrap();
        assert!(close(po.dir, [-1.0, 0.0], 1e-5));
        assert!((po.depth - 5.0).abs() < 1e-5);

        let from_left = Collider::Box(OrientedBox::new([-15.0, 0.0], [10.0, 10.0]));
        let po = a.push_out(&from_left).unwrap();
        assert!(close(po.dir, [1.0, 0.0], 1e-5));
        assert!((po.depth - 5.0).abs() < 1e-5);

        let from_above = Collider::Box(OrientedBox::new([0.0, 15.0], [10.0, 10.0]));
        let po = a.push_out(&from_above).unwrap();
        assert!(close(po.dir, [0.0, -1.0], 1e-5));
        assert!((po.depth - 5.0).abs() < 1e-5);

        let from_below = Collider::Box(OrientedBox::new([0.0, -15.0], [10.0, 10.0]));
        let po = a.push_out(&from_below).unwrap();
        assert!(close(po.dir, [0.0, 1.0], 1e-5));
        assert!((po.depth - 5.0).abs() < 1e-5);
    }

    #[test]
    fn rotated_bar_overlaps_horizontal_bar() {
        let a = Collider::Box(OrientedBox::rotated([0.0, 0.0], [40.0, 10.0], 0.0));
        let b = Collider::Box(OrientedBox::rotated(
            [20.0, 0.0],
            [10.0, 40.0],
            FRAC_PI_4,
        ));
        assert!(a.intersects(&b));
    }

    #[test]
    fn rotated_box_half_pixel_clear_is_not_overlap() {
        // A diamond whose left vertex is a half pixel past the square's
        // right edge: apart.
        let a = Collider::Box(OrientedBox::new([0.0, 0.0], [10.0, 10.0]));
        let b = Collider::Box(OrientedBox::rotated(
            [10.0 + 10.0 * SQRT_2 + 0.5, 0.0],
            [10.0, 10.0],
            FRAC_PI_4,
        ));
        assert!(!a.intersects(&b));
    }

    #[test]
    fn rotated_box_half_pixel_in_overlaps_and_pushes_along_x() {
        let a = Collider::Box(OrientedBox::new([0.0, 0.0], [10.0, 10.0]));
        let b = Collider::Box(OrientedBox::rotated(
            [10.0 + 10.0 * SQRT_2 - 0.5, 0.0],
            [10.0, 10.0],
            FRAC_PI_4,
        ));
        let po = a.push_out(&b).unwrap();
        assert!(close(po.dir, [-1.0, 0.0], 1e-4));
        assert!((po.depth - 0.5).abs() < 1e-4);
    }

    #[test]
    fn rotated_push_out_picks_least_penetration_axis() {
        // A diamond nudged half a pixel above a horizontal bar: the bar's
        // own y axis is the least-penetrated one, so that is the MTV.
        let a = Collider::Box(OrientedBox::rotated([0.0, 0.0], [40.0, 10.0], 0.0));
        let b = Collider::Box(OrientedBox::rotated(
            [30.0, 1.0],
            [10.0, 10.0],
            FRAC_PI_4,
        ));
        let po = a.push_out(&b).unwrap();
        assert!(close(po.dir, [0.0, -1.0], 1e-4));
        assert!((po.depth - (9.0 + 10.0 * SQRT_2)).abs() < 1e-4);
    }

    #[test]
    fn push_out_actually_separates() {
        let ab = OrientedBox::rotated([0.0, 0.0], [30.0, 10.0], 0.3);
        let bb = OrientedBox::rotated([25.0, 5.0], [40.0, 8.0], -0.7);
        let a = Collider::Box(ab);
        let b = Collider::Box(bb);
        assert!(a.intersects(&b));

        let po = a.push_out(&b).unwrap();
        let moved = OrientedBox::rotated(
            [ab.center[0] + po.dir[0] * po.depth, ab.center[1] + po.dir[1] * po.depth],
            ab.half,
            ab.angle,
        );
        assert!(!Collider::Box(moved).intersects(&b));
    }

    // -- Circle–circle --

    #[test]
    fn circle_overlap_touch_miss() {
        let a = Collider::Circle(Circle { center: [0.0, 0.0], radius: 30.0 });
        let b = Collider::Circle(Circle { center: [40.0, 0.0], radius: 30.0 });
        assert!(a.intersects(&b));
        let po = a.push_out(&b).unwrap();
        assert!(close(po.dir, [-1.0, 0.0], 1e-5));
        assert!((po.depth - 20.0).abs() < 1e-4);

        let c = Collider::Circle(Circle { center: [60.0, 0.0], radius: 30.0 });
        assert!(!a.intersects(&c)); // Exactly touching.
    }

    #[test]
    fn concentric_circles_push_out_is_deterministic() {
        let a = Collider::Circle(Circle { center: [0.0, 0.0], radius: 10.0 });
        let b = Collider::Circle(Circle { center: [0.0, 0.0], radius: 10.0 });
        let po = a.push_out(&b).unwrap();
        assert!(close(po.dir, [1.0, 0.0], 1e-6));
        assert!((po.depth - 20.0).abs() < 1e-5);
    }

    // -- Circle–box --

    #[test]
    fn circle_aabb_edge_push_out() {
        let sq = Collider::Box(OrientedBox::new([0.0, 0.0], [50.0, 50.0]));
        let c = Collider::Circle(Circle { center: [55.0, 0.0], radius: 20.0 });
        assert!(c.intersects(&sq));
        let po = c.push_out(&sq).unwrap();
        assert!(close(po.dir, [1.0, 0.0], 1e-5));
        assert!((po.depth - 15.0).abs() < 1e-4); // 50 + 20 - 55.
    }

    #[test]
    fn circle_aabb_corner_push_out() {
        let sq = Collider::Box(OrientedBox::new([0.0, 0.0], [50.0, 50.0]));
        let c = Collider::Circle(Circle { center: [60.0, 60.0], radius: 15.0 });
        let po = c.push_out(&sq).unwrap();
        assert!(close(po.dir, [SQRT_2 / 2.0, SQRT_2 / 2.0], 1e-4));
        let depth = 15.0 - (200.0_f32).sqrt();
        assert!((po.depth - depth).abs() < 1e-4);
    }

    #[test]
    fn circle_aabb_touching_is_not_overlap() {
        let sq = Collider::Box(OrientedBox::new([0.0, 0.0], [50.0, 50.0]));
        let c = Collider::Circle(Circle { center: [70.0, 0.0], radius: 20.0 });
        assert!(!c.intersects(&sq)); // Closest point at distance 20 = radius.
    }

    #[test]
    fn circle_center_inside_box_pushes_out_past_nearest_face() {
        let sq = Collider::Box(OrientedBox::new([0.0, 0.0], [50.0, 30.0]));
        let c = Collider::Circle(Circle { center: [10.0, 0.0], radius: 20.0 });
        let po = c.push_out(&sq).unwrap();
        // The nearest face is the top one (30 away, not 40): the circle
        // clears once it has moved radius + face distance along +y.
        assert!(close(po.dir, [0.0, 1.0], 1e-6));
        assert!((po.depth - 50.0).abs() < 1e-4);
    }

    #[test]
    fn circle_fully_inside_box_pushes_out_full_clearance() {
        let sq = Collider::Box(OrientedBox::new([0.0, 0.0], [50.0, 50.0]));
        let c = Collider::Circle(Circle { center: [0.0, 0.0], radius: 10.0 });
        let po = c.push_out(&sq).unwrap();
        assert!(close(po.dir, [1.0, 0.0], 1e-6));
        assert!((po.depth - 60.0).abs() < 1e-4); // radius + face distance.
    }

    #[test]
    fn circle_rotated_box_push_out() {
        // A horizontal bar turned a quarter turn is a vertical bar with the
        // half-extents swapped.
        let bar = Collider::Box(OrientedBox::rotated(
            [0.0, 0.0],
            [40.0, 10.0],
            FRAC_PI_2,
        ));
        let c = Collider::Circle(Circle { center: [0.0, 45.0], radius: 20.0 });
        let po = c.push_out(&bar).unwrap();
        assert!(close(po.dir, [0.0, 1.0], 1e-4));
        assert!((po.depth - 15.0).abs() < 1e-4);
    }

    // -- Box–circle symmetry --

    #[test]
    fn box_push_out_of_circle_is_inverse_of_circle() {
        let a = Collider::Box(OrientedBox::new([0.0, 0.0], [30.0, 20.0]));
        let c = Collider::Circle(Circle { center: [40.0, 0.0], radius: 25.0 });
        let ba = a.push_out(&c).unwrap();
        let cb = c.push_out(&a).unwrap();
        assert!(close(ba.dir, [-cb.dir[0], -cb.dir[1]], 1e-6));
        assert!((ba.depth - cb.depth).abs() < 1e-5);
        assert!((ba.depth - 15.0).abs() < 1e-4); // 30 + 25 - 40.
    }

    // -- reflect --

    #[test]
    fn reflect_zero_restitution_slides() {
        // Hitting a wall whose normal points +x while moving toward it:
        // the normal component is killed, the tangential one survives.
        let v = reflect([-100.0, 50.0], [1.0, 0.0], 0.0);
        assert!(close(v, [0.0, 50.0], 1e-4));
    }

    #[test]
    fn reflect_full_restitution_bounces() {
        let v = reflect([-100.0, 50.0], [1.0, 0.0], 1.0);
        assert!(close(v, [100.0, 50.0], 1e-4));
    }

    #[test]
    fn reflect_ignores_velocity_away_from_surface() {
        let v = reflect([100.0, 50.0], [1.0, 0.0], 0.5);
        assert!(close(v, [100.0, 50.0], 1e-6));
    }
}
