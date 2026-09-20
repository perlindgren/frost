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
//! parent-space point and applying the base rock and the joint bends.
//! Once a slice is fully grown, its blooms open: the four lower slices'
//! hand-picked spawn points — [FLOWER_SPAWNS]; the top slice bears none —
//! each get a flower slot, in the flower children that follow the slice
//! nodes. A slot's flower grows from zero to full size over
//! [FLOWER_GROW_TIME] seconds, its sprite tinted light green to yellow,
//! and when the flower is fully grown its tomato grows out of the same
//! point from zero to full size over [TOMATO_GROW_TIME] seconds: the white
//! fruit body (`tomato.png`) tinted dark green to light red, with the dark
//! calyx and stem (`tomato_fg.png`) drawn on top, both pinned to
//! [TOMATO_ANCHOR]. Each slot rides its slice's transform so the whole
//! bloom sways with the plant. The plant's fit scale is the plant node's
//! own `scale`, set once by the caller: the node's scale applies before
//! its transform, to its subtree, so the whole plant sizes around the root
//! joint while the joint itself still lands exactly on the anchor. The
//! [`layer_midpoints`] function gives the five layer-segment midpoints
//! along the static, fully grown chain — the sway left out — for the
//! vipers' orbit centers.

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
/// five slice nodes.
pub const FLOWER_N: usize =
    FLOWER_SPAWNS[0].len()
        + FLOWER_SPAWNS[1].len()
        + FLOWER_SPAWNS[2].len()
        + FLOWER_SPAWNS[3].len();

/// How long a flower takes to grow from zero to its full size, in seconds:
/// it starts the moment its slice is fully grown and reaches full size
/// [FLOWER_GROW_TIME] seconds later, growing out of its spawn point.
pub const FLOWER_GROW_TIME: f32 = 10.0;

/// How long a tomato takes to grow from zero to its full size, in seconds:
/// it starts the moment its flower is fully grown and reaches full size
/// [TOMATO_GROW_TIME] seconds later, growing out of the same point.
pub const TOMATO_GROW_TIME: f32 = 10.0;

/// The tomato's attachment anchor in the tomato images' pixel space:
/// `(0, 0)` at the upper-left, `x` right, `y` down. Both `tomato.png` and
/// `tomato_fg.png` are the same size and share this anchor, so the dark
/// foreground lines up pixel-for-pixel on the white background; the anchor
/// is where the stem meets the flower, and it stays pinned to the flower
/// for the whole growth.
pub const TOMATO_ANCHOR: (f32, f32) = (308.0, 411.0);

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

/// The tomato background's color at the start of its growth: dark green.
pub const TOMATO_GREEN: frost::Color = frost::Color {
    r: 0.13,
    g: 0.4,
    b: 0.13,
    a: 1.0,
};

/// The tomato background's color at full growth: light red.
pub const TOMATO_RED: frost::Color = frost::Color {
    r: 0.95,
    g: 0.45,
    b: 0.4,
    a: 1.0,
};

/// The flower slot's children, in draw order: the tomato pivot first — so
/// the flower leaf paints on top of the stem — then the flower leaf.
const SLOT_TOMATO: usize = 0;
const SLOT_FLOWER: usize = 1;

/// The tomato pivot's children, in draw order: the white fruit body
/// (background) first, then the dark calyx and stem (foreground) on top.
const TOMATO_BG: usize = 0;
const TOMATO_FG: usize = 1;

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

/// The tomato's growth, 0..1, at plant-clock `t` for a slice that finishes
/// growing at `grow_time`: it starts when the flower is fully grown
/// ([FLOWER_GROW_TIME] seconds after the slice) and reaches one
/// [TOMATO_GROW_TIME] seconds later, linear in between.
fn tomato_growth(t: f32, grow_time: f32) -> f32 {
    ((t - grow_time - FLOWER_GROW_TIME) / TOMATO_GROW_TIME).clamp(0.0, 1.0)
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

/// The tomato background's color at growth `g`: dark green to light red.
fn tomato_color(g: f32) -> frost::Color {
    mix(TOMATO_GREEN, TOMATO_RED, g)
}

/// The translate that puts the tomato's [TOMATO_ANCHOR] pixel on its leaf's
/// origin, for an image of the given size: the negation of the anchor's
/// node-local offset, with the same y flip as the slice joints.
fn tomato_leaf_offset(size: [f32; 2]) -> [f32; 2] {
    let a = local_joint(TOMATO_ANCHOR.0, TOMATO_ANCHOR.1, size);
    [-a[0], -a[1]]
}

/// A five-slice plant that grows out of its root joint and sways in a
/// traveling wind. The shapes themselves live in the scene; the value only
/// keeps the growth clock, the slices' joints, and the flower spawn
/// points — all in node-local space.
#[derive(Clone)]
pub struct Plant {
    /// Elapsed time in seconds.
    t: f32,
    /// The five slices in chain order, with their joints in node-local
    /// space.
    links: [Link; 5],
    /// The four lower slices' flower spawn points in node-local space, in
    /// flattened slice order: `new` converts [FLOWER_SPAWNS] against each
    /// texture's real size.
    tomato_spawn: [Vec<[f32; 2]>; 4],
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
    /// the five slice nodes followed by the [FLOWER_N] flower slots, in
    /// flattened [FLOWER_SPAWNS] order. Each flower slot is a pivot whose
    /// children, in draw order, are a tomato pivot — whose children are the
    /// `tomato` background leaf and the `tomato_fg` foreground leaf, both
    /// pinned to [TOMATO_ANCHOR] — and the `flower` leaf on top. `layout`
    /// owns the slots' and their leaves' visibility, growth, and tints from
    /// that frame on: a slot's flower grows from zero to full size over
    /// [FLOWER_GROW_TIME] seconds after its slice finishes, tinted light
    /// green to yellow, and its tomato grows from zero to full size over
    /// [TOMATO_GROW_TIME] seconds after the flower finishes, its background
    /// tinted dark green to light red.
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
        // finishes. The slot is a pure pivot: it carries no shape, scale,
        // or tint of its own (a tint there would leak onto the tomato),
        // only the translate that lays its origin on the spawn point
        // mapped through the slice's current transform, so the whole bloom
        // sways with the plant.
        let mut slot = 0usize;
        for (i, spawns) in self.tomato_spawn.iter().enumerate() {
            for &pt in spawns {
                let child = &mut node.children[5 + slot];
                let fg = flower_growth(self.t, GROW_TIMES[i]);
                let tg = tomato_growth(self.t, GROW_TIMES[i]);
                if fg > 0.0 {
                    let p = slice_tf[i].apply(pt);
                    child.transform = frost::Transform::translate(p[0], p[1]);
                }
                // The flower leaf, on top of the tomato: grows about the
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
                // The tomato pivot: grows about the same point, a little
                // later — it starts when the flower is fully grown — and
                // its two leaves keep the stem's anchor pinned to the
                // flower for the whole growth.
                let tomato_pivot = &mut child.children[SLOT_TOMATO];
                tomato_pivot.scale = [tg, tg];
                let [ox, oy] = tomato_leaf_offset(sprite_size(tomato));
                // The background: the tinted fruit body, dark green to
                // light red.
                let bg = &mut tomato_pivot.children[TOMATO_BG];
                bg.transform = frost::Transform::translate(ox, oy);
                if tg > 0.0 {
                    if bg.shape.is_none() {
                        bg.shape = Some(tomato.clone());
                    }
                    bg.modulate = tomato_color(tg);
                } else {
                    bg.shape = None;
                }
                // The foreground: the dark calyx and stem, drawn on top,
                // unmodulated.
                let fg_leaf = &mut tomato_pivot.children[TOMATO_FG];
                fg_leaf.transform = frost::Transform::translate(ox, oy);
                if tg > 0.0 {
                    if fg_leaf.shape.is_none() {
                        fg_leaf.shape = Some(tomato_fg.clone());
                    }
                } else {
                    fg_leaf.shape = None;
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

    /// A plant with a settable growth clock, so [Plant::grown_layers]'s
    /// boundaries can be checked without stepping.
    fn plant_at(t: f32) -> Plant {
        let s = slice();
        let mut p = Plant::new([&s, &s, &s, &s, &s]);
        p.t = t;
        p
    }

    /// A plant node built the way the example builds it: the five slice
    /// shapes followed by the [FLOWER_N] flower slots, each a pivot holding
    /// a tomato pivot (background leaf, foreground leaf) and a flower leaf.
    fn plant_node() -> frost::SceneNode {
        let s = slice();
        let mut children: Vec<Box<frost::SceneNode>> = (0..5)
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
                    Box::new(frost::SceneNode {
                        children: vec![
                            Box::new(frost::SceneNode::default()),
                            Box::new(frost::SceneNode::default()),
                        ],
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode::default()),
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
    /// seconds later; its tomato starts when the flower is done and is full
    /// size, light red, [TOMATO_GROW_TIME] seconds after that.
    #[test]
    fn flowers_and_tomatoes_grow_as_each_slice_finishes_growing() {
        let s = slice();
        let mut node = plant_node();

        // Before the base slice finishes, nothing has started to grow:
        // every slot's flower and tomato leaves are shapeless, at zero
        // growth.
        let p = plant_at(GROW_TIMES[0] - 0.001);
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        for c in node.children.iter().skip(5) {
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
        let f = &node.children[5].children[SLOT_FLOWER];
        assert!(f.shape.is_some());
        assert!((f.scale[0] - 0.5).abs() < 1e-6, "flower scale mid-growth");
        let t = &node.children[5].children[SLOT_TOMATO];
        assert_eq!(t.scale, [0.0, 0.0], "the tomato started early");
        for leaf in t.children.iter() {
            assert!(leaf.shape.is_none());
        }

        // At full flower growth the flower is full size and yellow; the
        // tomato has not started.
        let bloom = GROW_TIMES[0] + FLOWER_GROW_TIME;
        let p = plant_at(bloom);
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        let f = &node.children[5].children[SLOT_FLOWER];
        assert_eq!(f.scale, [1.0, 1.0]);
        assert_eq!(f.modulate, FLOWER_BLOOM, "flower color at full growth");
        let t = &node.children[5].children[SLOT_TOMATO];
        assert_eq!(t.scale, [0.0, 0.0]);

        // Mid tomato growth, both tomato leaves are present and half sized;
        // the background is tinted between green and red.
        let p = plant_at(bloom + TOMATO_GROW_TIME / 2.0);
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        let t = &node.children[5].children[SLOT_TOMATO];
        assert!((t.scale[0] - 0.5).abs() < 1e-6, "tomato scale mid-growth");
        assert!(t.children[TOMATO_BG].shape.is_some());
        assert!(t.children[TOMATO_FG].shape.is_some());

        // At full tomato growth the tomato is full size and light red; the
        // foreground is unmodulated.
        let p = plant_at(bloom + TOMATO_GROW_TIME);
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);
        let t = &node.children[5].children[SLOT_TOMATO];
        assert_eq!(t.scale, [1.0, 1.0]);
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
        let t = FULL_GROW_TIME + 1.0;
        let p = plant_at(t);
        let mut node = plant_node();
        p.layout(&mut node, [0.0, 0.0], &s, &s, &s);

        // Each flower sits on its slice: the child's translate is the
        // slice's transform applied to the spawn point.
        let mut offset = 0usize;
        for (i, spawns) in p.tomato_spawn.iter().enumerate() {
            for &pt in spawns {
                let flower = node.children[5 + offset].transform.apply([0.0, 0.0]);
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
            node.transform.apply(node.children[5].transform.apply([0.0, 0.0]))
        };
        let a = composed(&node);
        let b = composed(&node2);
        assert!(
            (a[0] - b[0]).abs() + (a[1] - b[1]).abs() > 1e-3,
            "the base flower did not move with the sway"
        );
    }
}
