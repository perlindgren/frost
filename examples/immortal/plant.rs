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
//! moves the most.
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
//! parent-space point and applying the base rock and the joint bends. The
//! plant's fit scale is the plant node's own `scale`, set once by the
//! caller: the node's scale applies before its transform, to its subtree,
//! so the whole plant sizes around the root joint while the joint itself
//! still lands exactly on the anchor. The [`layer_midpoints`] function
//! gives the five layer-segment midpoints along the static, fully grown
//! chain — the sway left out — for the vipers' orbit centers.

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

type TomatoSpawn = [Vec<(f32, f32)>; 4];

/// A five-slice plant that grows out of its root joint and sways in a
/// traveling wind. The shapes themselves live in the scene; the value only
/// keeps the growth clock and the slices' joints in node-local space.
#[derive(Clone)]
pub struct Plant {
    /// Elapsed time in seconds.
    t: f32,
    /// The five slices in chain order, with their joints in node-local
    /// space.
    links: [Link; 5],
    tomato_spawn: TomatoSpawn,
}

impl Plant {
    /// Builds the plant from the five slice shapes in chain order
    /// (`plant1`, `plant2`, `plant3`, `plant4`, `plant5`), with each slice's joints converted
    /// from `JOINTS` to node-local space against its texture's real size.
    /// The growth clock starts at zero.
    pub fn new(shapes: [&frost::Shape; 5]) -> Self {
        Plant {
            t: 0.0,
            links: [
                link(shapes[0], JOINTS[0]),
                link(shapes[1], JOINTS[1]),
                link(shapes[2], JOINTS[2]),
                link(shapes[3], JOINTS[3]),
                link(shapes[4], JOINTS[4]),
            ],
            tomato_spawn: [
                vec![(308.0, 615.0)],
                vec![(306.0, 496.0), (638.0, 428.0), (304.0, 392.0)],
                vec![
                    (315.0, 392.0),
                    (326.0, 315.0),
                    (297.0, 258.0),
                    (320.0, 203.0),
                    (300.0, 128.0),
                ],
                vec![
                    (339.0, 364.0),
                    (298.0, 318.0),
                    (356.0, 258.0),
                    (388.0, 228.0),
                    (334.0, 203.0),
                    (283.0, 182.0),
                ],
            ],
        }
    }

    /// Advances the growth clock by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        self.t += dt;
    }

    /// Whether the plant is fully grown: the last slice has reached its
    /// full length, and from here on the plant only keeps swaying.
    pub fn fully_grown(&self) -> bool {
        self.t >= FULL_GROW_TIME
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
        for i in 0..5 {
            let d = [
                self.links[i].to[0] - self.links[i].from[0],
                self.links[i].to[1] - self.links[i].from[1],
            ];
            midpoints[i] = [p[0] + d[0] / 2.0, p[1] + d[1] / 2.0];
            p = [p[0] + d[0], p[1] + d[1]];
        }
        midpoints
    }

    /// Lays the plant out in `node`, whose children — in chain order — are
    /// the five slice nodes.
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
    pub fn layout(&self, node: &mut frost::SceneNode, anchor: [f32; 2]) {
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
            // The next anchor: this slice's upper joint, measured from its
            // own lower joint, scaled by the slice's growth, and rotated
            // by the slice's bend.
            let d = [link.to[0] - link.from[0], link.to[1] - link.from[1]];
            let step = rot.apply([g * d[0], g * d[1]]);
            anchor = [anchor[0] + step[0], anchor[1] + step[1]];
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

    /// A plant with a settable growth clock, so [Plant::grown_layers]'s
    /// boundaries can be checked without stepping.
    fn plant_at(t: f32) -> Plant {
        let s = slice();
        let mut p = Plant::new([&s, &s, &s, &s, &s]);
        p.t = t;
        p
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
}
