//! The centipede's rig: the pure geometry that turns the seven cut-out
//! `Monster_*` parts into an animal that walks.
//!
//! Nothing here touches the scene graph or a GPU: [`pose`] takes the
//! segments the animal is built from and a time, and hands back a flat
//! list of [`Placement`]s — one per sprite — in the order they should be
//! drawn. The example turns those into scene nodes; a software renderer
//! can just as well composite them into a picture, which is how the
//! layout was checked without opening a window.
//!
//! # The space
//!
//! Everything is in the **monster's own space**: `y` up, the ground at
//! `y = 0`, and `+x` pointing *backward*, because the animal faces `-x`
//! and crawls to the left. One unit is one texture pixel of the leg parts
//! (the legs are the one part drawn at their texture's own size), so the
//! example scales the whole animal once, at the end.
//!
//! # How a walk comes out of still parts
//!
//! Each segment carries a two-link leg — the thigh piece pivots at the
//! hip, the shin-and-foot piece at the knee — and the foot is driven, not
//! animated: [`pose`] decides where each foot *is*, and [`solve_leg`]
//! inverse-kinematics the two links onto it. A foot alternates between
//!
//! - **stance** — planted on the ground and sliding *backward* through
//!   monster space, and
//! - **swing** — lifted and reaching forward again.
//!
//! A planted foot is stationary in the *world*, and monster space slides
//! over the world at [`CRAWL_SPEED`], so the foot has to slide backward
//! through monster space at exactly that speed. That is what
//! [`STRIDE`] = `CRAWL_SPEED × DUTY × CYCLE` means: one stance covers
//! precisely the ground the animal travels while it lasts, so the feet
//! grip instead of skating. The stride is scaled by the duty factor
//! because a foot is only allowed to travel during its stance.
//!
//! Legs do not all step at once: leg `i` lags leg `i - 1` by [`LAG`] of a
//! cycle, which is a metachronal wave — the ripple of steps runs from the
//! head to the tail, the same direction the spine's [`WAVE_AMP`] ripple
//! travels in, so the body and its legs move as one animal. The far-side
//! legs are a half cycle out of step with their near neighbours and drawn
//! darker and smaller, which is how a flat side view reads as two rows of
//! legs.

use std::f32::consts::{PI, TAU};

/// A point in a part's own texture pixels: `(0, 0)` at the upper-left and
/// `y` **down**, the way an image is stored — as opposed to the monster
/// space of [`Placement`], which is `y` up. [`frost::Transform::anchor`]
/// converts between them.
pub type Px = [f32; 2];

// ---------------------------------------------------------------- parts --

/// The head: the spiky clay skull with the two yellow eyes, facing left.
pub const HEAD: usize = 0;
/// The three body segments — folded-paper chevrons, apex forward — that a
/// monster is randomly built from.
pub const BODIES: [usize; 3] = [1, 2, 3];
/// The tail: the small rear spike cluster.
pub const TAIL: usize = 4;
/// The upper leg: the hip bar and the thigh, tapering down to the knee.
pub const THIGH: usize = 5;
/// The lower leg: the shin and the flat clay foot.
pub const SHIN: usize = 6;
/// How many parts the rig knows, and how long the example's shape table is.
pub const PART_COUNT: usize = 7;

/// The part files, in the order of the indices above. `Monster_leg1.png`
/// and `Monster_leg2.png` were byte-identical rasters of one leg cut in
/// two where the body covered its middle, so they are cropped as the two
/// halves of that leg: `leg1` keeps the hip bar and thigh, `leg2` the shin
/// and foot.
pub const PART_FILES: [&str; PART_COUNT] = [
    "Monster_head.png",
    "Monster_body1.png",
    "Monster_body2.png",
    "Monster_body3.png",
    "Monster_tail.png",
    "Monster_leg1.png",
    "Monster_leg2.png",
];

// -------------------------------------------------------------- joints --
// Measured on the cropped textures, in the texture pixels of `Px`. The
// numbers are the visible tips and corners of each part, so a joint can be
// checked against the art by eye.

/// The head's neck, at the middle of its rear edge: where the skull meets
/// the first segment.
const NECK: Px = [672.0, 470.0];

/// The tail's root, at the middle of its left edge, where it tucks behind
/// the last segment.
const TAIL_ROOT: Px = [22.0, 122.0];

/// Each body variant's spine point: the apex of its chevron, which is what
/// rides the spine's wave. Per variant, because the three chevrons are cut
/// from the sheet at different heights.
const SPINE_ANCHOR: [Px; 3] = [[247.0, 408.0], [225.0, 336.0], [198.0, 362.0]];

/// The hip: inside the thigh part's horizontal bar, where the bar meets
/// the leg.
const HIP: Px = [85.0, 85.0];

/// The knee, seen from the thigh: the tapered tip at the bottom of the
/// thigh piece.
const THIGH_KNEE: Px = [45.0, 440.0];

/// The same knee seen from the shin: the narrow tip at the top of the shin
/// piece. Rotating either part about its own tip is what bends the leg.
const SHIN_KNEE: Px = [28.0, 16.0];

/// The ankle, where the shin meets its foot.
const ANKLE: Px = [86.0, 367.0];

/// The row of the shin texture the sole of the foot rests on: the bottom of
/// the clay pad, two rows above the texture's own bottom edge.
const SOLE_ROW: f32 = 471.2;

// --------------------------------------------------------------- layout --

/// The body scale: the head, the segments and the tail are drawn at this
/// fraction of their texture size, while the legs stay at `1.0`. The
/// chevrons are the same height as a whole leg in the art, which stacks a
/// blob on top of stunted legs; at `0.8` the segments still overlap each
/// other as they do in the sheet, and the legs become long enough that the
/// gait is actually visible.
pub const BODY_SCALE: f32 = 0.8;

/// How far apart the segments sit along the body, in monster units — about
/// half a chevron, so consecutive segments overlap by a head's width.
pub const PITCH: f32 = 220.0;

/// How much of a standing leg's reach the hip uses — see [`Rig::spine_y`].
/// Below `1.0` the knee keeps a permanent bend, which is what a clay leg
/// mid-stride looks like, and every step has somewhere to bend further
/// into; at `1.0` the knees would lock straight and the walk would read as
/// a rolling barrel.
pub const HIP_BEND: f32 = 0.82;

/// The hip sits this far behind the middle of its segment, matching where
/// the clay legs were planted in the sheet.
pub const HIP_BACK: f32 = 25.0;

/// The head's neck rides the spine this many segments in front of segment
/// 0 (negative is forward).
pub const HEAD_AT: f32 = -0.75;

/// The tail's root sits this far behind the last segment's middle.
pub const TAIL_BACK: f32 = 215.0;

/// The far-side legs' ground line, above the near one: two rows of feet on
/// one flat plane read as one row at the near edge and one a little way
/// behind it.
pub const FAR_GROUND: f32 = 34.0;

/// The far-side legs' extra scale, on top of their segment's growth.
pub const FAR_SCALE: f32 = 0.9;

/// The far-side legs' modulate: the same clay, in shadow.
pub const FAR_SHADE: f32 = 0.45;

// ----------------------------------------------------------------- gait --

/// How fast the animal travels along the ground, monster units per second
/// — a little more than one segment per second.
pub const CRAWL_SPEED: f32 = 260.0;

/// One full step cycle of a single foot, in seconds. Slow on purpose: the
/// cycle sets the stride at a given speed, and a slow cycle means a long
/// stride, which is the half of a gait the eye actually reads.
pub const CYCLE: f32 = 2.2;

/// The share of a cycle a foot spends planted on the ground. The rest is
/// the swing, and six tenths planted keeps at least a couple of feet
/// gripping the ground at all times.
pub const DUTY: f32 = 0.6;

/// The distance a planted foot travels relative to the body during its
/// stance, in monster units: exactly the ground covered while it lasts, so
/// a planted foot does not slide. See the module docs.
pub const STRIDE: f32 = CRAWL_SPEED * DUTY * CYCLE;

/// How far a swinging foot lifts off its ground line, in monster units.
pub const LIFT: f32 = 130.0;

/// The gait's lag between one segment's leg and the next, as a fraction of
/// [`CYCLE`]: the step ripples from head to tail, about nine segments per
/// full wave.
pub const LAG: f32 = 0.125;

/// The spine wave's amplitude, in monster units: how far a segment rides
/// above and below the spine's standing height, [`Rig::spine_y`].
pub const WAVE_AMP: f32 = 42.0;

/// The spine wave's wavelength in segments — eight of them, the same span
/// [`LAG`] spreads the step wave over, so the body's ripple and the ripple
/// of steps under it travel together.
pub const WAVE_SEGS: f32 = 8.0;

/// Which way the knee points, as the sign of the IK bend: `-1.0` puts the
/// knee forward, toward `-x`, the way the sheet's legs are bent.
pub const KNEE_BEND: f32 = -1.0;

/// The smallest leg scale a segment's legs are born at, right after the
/// animal regrows a segment; they reach full size with the segment.
pub const GROW_START: f32 = 0.55;

/// How long a new segment takes to reach full size, in seconds.
pub const GROW_SECONDS: f32 = 1.2;

// ---------------------------------------------------------------- types --

/// One body segment of the animal: which of the three chevron variants it
/// is (a [`BODIES`] index), and how far it has grown — `0.0` just born,
/// `1.0` full size.
#[derive(Clone, Copy, Debug)]
pub struct Segment {
    /// The part index of this segment's chevron, one of [`BODIES`].
    pub body: usize,
    /// The growth fraction, ramped from [`GROW_START`] to `1.0`.
    pub grow: f32,
}

/// One sprite to draw, in the order the list is built.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// Which part: an index into the example's shape table.
    pub part: usize,
    /// Where the part's joint lands in monster space.
    pub joint: [f32; 2],
    /// That joint's offset inside the part's own local space — the part is
    /// centred on its node, so this is what has to be subtracted to hang
    /// the part from its joint instead of its middle.
    pub pivot: [f32; 2],
    /// The tilt in radians, counter-clockwise, `0.0` being the pose the
    /// part was cut in.
    pub angle: f32,
    /// A uniform scale about the joint.
    pub scale: f32,
    /// The modulate to tint the part with: `1.0` is the part's own colour,
    /// [`FAR_SHADE`] a leg on the far side.
    pub shade: f32,
}

/// The rig's derived numbers, from the textures' real sizes.
#[derive(Clone, Copy, Debug)]
pub struct Rig {
    /// Each part's joint in its own local space, indexed by part. The body
    /// variants each carry their own spine point.
    pub pivots: [[f32; 2]; PART_COUNT],
    /// The thigh's length: hip to knee, in local units.
    pub l1: f32,
    /// The shin's length: knee to ankle, in local units.
    pub l2: f32,
    /// The direction the thigh points as it was drawn, in radians.
    pub rest1: f32,
    /// The direction the shin points as it was drawn, in radians.
    pub rest2: f32,
    /// How far the sole of the foot sits below the ankle, as drawn.
    pub sole: f32,
    /// Three points along the sole of the foot, in the shin part's local
    /// space measured from its knee pivot: under the ankle, mid-foot and
    /// at the rear edge of the clay pad. The lowest of them after the shin
    /// turns is what has to rest on the ground — see [`place_leg`].
    pub sole_points: [[f32; 2]; 3],
    /// How far the nose reaches in front of the neck, at [`BODY_SCALE`].
    pub nose: f32,
    /// How far the tail reaches behind its root, at [`BODY_SCALE`].
    pub tail_reach: f32,
    /// How far the skull rises above its neck, at [`BODY_SCALE`]: what the
    /// animal's height is measured with, on top of [`Rig::spine_y`].
    pub skull: f32,
}

impl Rig {
    /// Derives the joints and the limb lengths from the parts' texture
    /// sizes, so the anchor table above stays in image pixels and nothing
    /// has to be recomputed by hand when the art is re-cropped.
    pub fn new(sizes: [[f32; 2]; PART_COUNT]) -> Self {
        let pivot = |px: Px, size: [f32; 2]| frost::Transform::anchor(px, size);
        let pivots = [
            pivot(NECK, sizes[HEAD]),
            pivot(SPINE_ANCHOR[0], sizes[BODIES[0]]),
            pivot(SPINE_ANCHOR[1], sizes[BODIES[1]]),
            pivot(SPINE_ANCHOR[2], sizes[BODIES[2]]),
            pivot(TAIL_ROOT, sizes[TAIL]),
            pivot(HIP, sizes[THIGH]),
            pivot(SHIN_KNEE, sizes[SHIN]),
        ];
        // The thigh points from its hip to its tip; the shin from its knee
        // tip to its ankle. Both as drawn, in local y-up units.
        let knee = pivot(THIGH_KNEE, sizes[THIGH]);
        let thigh = [knee[0] - pivots[THIGH][0], knee[1] - pivots[THIGH][1]];
        let ankle = pivot(ANKLE, sizes[SHIN]);
        let shin = [ankle[0] - pivots[SHIN][0], ankle[1] - pivots[SHIN][1]];
        // Straight down from the ankle to the sole, as drawn: negative in
        // local units, stored as the positive drop it is.
        let sole = pivot([ANKLE[0], SOLE_ROW], sizes[SHIN])[1] - ankle[1];
        // Three taps along the foot pad, from the knee pivot: it is the
        // foot's lowest point that stands on the ground, not its ankle, so
        // the rig needs to know where the pad is.
        let sole_points = [ANKLE[0], 200.0, sizes[SHIN][0] - 4.0].map(|x| {
            let p = pivot([x, SOLE_ROW], sizes[SHIN]);
            [p[0] - pivots[SHIN][0], p[1] - pivots[SHIN][1]]
        });
        Self {
            pivots,
            l1: (thigh[0] * thigh[0] + thigh[1] * thigh[1]).sqrt(),
            l2: (shin[0] * shin[0] + shin[1] * shin[1]).sqrt(),
            rest1: thigh[1].atan2(thigh[0]),
            rest2: shin[1].atan2(shin[0]),
            sole: -sole,
            sole_points,
            nose: (NECK[0] - 3.0) * BODY_SCALE,
            tail_reach: (sizes[TAIL][0] - TAIL_ROOT[0]) * BODY_SCALE,
            skull: NECK[1] * BODY_SCALE,
        }
    }

    /// The full reach of a leg of the given scale, knee to sole: the height
    /// a hip at that scale can sit at and still touch the ground.
    pub fn reach(&self, scale: f32) -> f32 {
        (self.l1 + self.l2 + self.sole) * scale
    }

    /// The spine's height above the ground, which is every hip's height:
    /// [`HIP_BEND`] of a full-size leg's reach, so re-cropping the art moves
    /// the body with the legs instead of leaving them over-stretched or
    /// swimming. For the parts as cut, a leg reaches `816` and the spine
    /// sits at `669`.
    pub fn spine_y(&self) -> f32 {
        self.reach(1.0) * HIP_BEND
    }
}

// ----------------------------------------------------------------- pose --

/// The spine's height above the ground at segment position `i`, which may
/// be fractional (the head rides the wave three quarters of a segment ahead
/// of the first segment).
fn spine_y(rig: &Rig, i: f32, t: f32) -> f32 {
    rig.spine_y() + WAVE_AMP * (TAU * (i / WAVE_SEGS - t / CYCLE)).sin()
}

/// The tilt the spine's slope implies at segment position `i`: a chevron
/// leans with the wave it is riding.
fn spine_angle(i: f32, t: f32) -> f32 {
    (WAVE_AMP * (TAU / (WAVE_SEGS * PITCH)) * (TAU * (i / WAVE_SEGS - t / CYCLE)).cos()).atan()
}

/// Where a foot of the given phase sits: `(x relative to the hip, height
/// above its ground line)`.
///
/// `phase` is the fraction through the foot's own cycle, in `[0, 1)`. The
/// stance sweeps from half a [`STRIDE`] ahead of the hip to half a stride
/// behind it at exactly [`CRAWL_SPEED`]; the swing returns to the front
/// along an arc of [`LIFT`]. Both halves meet at the same point at each
/// end, so the foot never teleports.
fn foot_target(phase: f32) -> (f32, f32) {
    if phase < DUTY {
        (STRIDE * (phase / DUTY - 0.5), 0.0)
    } else {
        let v = (phase - DUTY) / (1.0 - DUTY);
        (STRIDE * (0.5 - v), LIFT * (v * PI).sin())
    }
}

/// Solves the two-link leg: the angles that put the shin's ankle on
/// `target` when the hip is at `hip`, at the given `scale`.
///
/// Returns the knee's position and the two parts' absolute tilts. Each
/// tilt is measured against the direction the part was cut in, because
/// rotating a part by `θ` turns its rest direction by `θ` — so the angle
/// that points a link along `d` is `atan2(d) - rest`. The two links of
/// fixed length reach `target` from two knees, mirror images across the
/// hip-to-target line; [`KNEE_BEND`] picks the one with the knee forward.
fn solve_leg(rig: &Rig, hip: [f32; 2], target: [f32; 2], scale: f32) -> ([f32; 2], f32, f32) {
    let (l1, l2) = (rig.l1 * scale, rig.l2 * scale);
    let dx = target[0] - hip[0];
    let dy = target[1] - hip[1];
    let raw = (dx * dx + dy * dy).sqrt();
    // A foot may be asked for further than the leg can stretch (a segment
    // still growing, say); clamp along the same ray instead of snapping.
    let dist = raw
        .max(1e-3)
        .clamp((l1 - l2).abs() + 1.0, (l1 + l2) * 0.999);
    let base = dy.atan2(dx);
    let bend = (((l1 * l1 + dist * dist - l2 * l2) / (2.0 * l1 * dist)).clamp(-1.0, 1.0)).acos();
    let thigh = base + KNEE_BEND * bend;
    let knee = [hip[0] + l1 * thigh.cos(), hip[1] + l1 * thigh.sin()];
    let shin = (target[1] - knee[1]).atan2(target[0] - knee[0]);
    (
        knee,
        thigh - rig.rest1,
        // A part asked to reach exactly its full length folds flat; the
        // shin's own tilt then has nothing to say, which is fine: nothing
        // but a fully folded leg looks at that.
        shin - rig.rest2,
    )
}

/// The height of the lowest point of a foot standing at `knee`, its shin
/// tilted by `shin`, at the given `scale`.
pub fn sole_height(rig: &Rig, knee: [f32; 2], shin: f32, scale: f32) -> f32 {
    let (cos, sin) = shin.sin_cos();
    rig.sole_points
        .iter()
        .map(|p| knee[1] + scale * (sin * p[0] + cos * p[1]))
        .fold(f32::INFINITY, f32::min)
}

/// One leg: the hip's position, its phase, and the scale it stands on.
///
/// The foot is aimed at the ground as a whole, not by its ankle: the clay
/// pad hangs off the shin, so a tilted shin drags the toe down with it. The
/// leg is solved once to find out how the shin ends up tilted, the aim is
/// lifted by the shortfall, and the leg is solved again — which is exact to
/// under half a unit, the tilt having barely changed in between.
fn place_leg(
    rig: &Rig,
    out: &mut Vec<Placement>,
    hip: [f32; 2],
    phase: f32,
    scale: f32,
    ground: f32,
    shade: f32,
) {
    let (x, lift) = foot_target(phase);
    // A shorter leg takes a shorter stride and lifts less: scaling the
    // foot's target with the leg keeps a growing segment's legs in step
    // with its own size. Only a full-size leg is stride-matched to the
    // crawl speed, so a sprout's feet skate a little for the second it
    // takes it to grow up.
    let sole_wanted = ground + lift * scale;
    let target_x = hip[0] + x * scale;
    let mut raise = rig.sole * scale;
    let mut solved = solve_leg(rig, hip, [target_x, sole_wanted + raise], scale);
    for _ in 0..2 {
        let low = sole_height(rig, solved.0, solved.2, scale);
        let err = sole_wanted - low;
        if err.abs() < 0.5 {
            break;
        }
        raise += err;
        solved = solve_leg(rig, hip, [target_x, sole_wanted + raise], scale);
    }
    let (knee, thigh, shin) = solved;
    out.push(Placement {
        part: THIGH,
        joint: hip,
        pivot: rig.pivots[THIGH],
        angle: thigh,
        scale,
        shade,
    });
    out.push(Placement {
        part: SHIN,
        joint: knee,
        pivot: rig.pivots[SHIN],
        angle: shin,
        scale,
        shade,
    });
}

/// Builds the whole animal at time `t`: one [`Placement`] per sprite, in
/// back-to-front draw order — the far legs, the near legs, the tail, the
/// segments from the back forward so each chevron overlaps the one behind
/// it as in the sheet, and the head last.
///
/// The legs are emitted before the body on purpose: the sheet's legs are
/// cut where a chevron covered them, so a segment is supposed to hide its
/// own knee.
pub fn pose(rig: &Rig, segments: &[Segment], t: f32, out: &mut Vec<Placement>) {
    out.clear();
    let count = segments.len() as f32;

    // The far side first, all of them, so no far leg can end up in front
    // of a body segment.
    for (i, seg) in segments.iter().enumerate() {
        let f = i as f32;
        let scale = seg.grow * FAR_SCALE;
        let hip = [f * PITCH + HIP_BACK, spine_y(rig, f, t)];
        place_leg(
            rig,
            out,
            hip,
            wrap(t / CYCLE - f * LAG + 0.5),
            scale,
            FAR_GROUND,
            FAR_SHADE,
        );
    }
    // Then the near side, at full size and its own colour.
    for (i, seg) in segments.iter().enumerate() {
        let f = i as f32;
        let hip = [f * PITCH + HIP_BACK, spine_y(rig, f, t)];
        place_leg(rig, out, hip, wrap(t / CYCLE - f * LAG), seg.grow, 0.0, 1.0);
    }

    // The tail trails the last segment, riding the wave one step further
    // back than the segment it hangs on.
    let last = count - 1.0;
    if count > 0.0 {
        let tail = &segments[segments.len() - 1];
        out.push(Placement {
            part: TAIL,
            joint: [last * PITCH + TAIL_BACK, spine_y(rig, last + 0.6, t)],
            pivot: rig.pivots[TAIL],
            angle: spine_angle(last + 0.6, t),
            scale: BODY_SCALE * tail.grow,
            shade: 1.0,
        });
    }

    // Segments back to front: the front one on top.
    for (i, seg) in segments.iter().enumerate().rev() {
        let f = i as f32;
        out.push(Placement {
            part: seg.body,
            joint: [f * PITCH, spine_y(rig, f, t)],
            pivot: rig.pivots[seg.body],
            angle: spine_angle(f, t),
            scale: BODY_SCALE * seg.grow,
            shade: 1.0,
        });
    }

    // The head leads, nodding on top of the wave it rides.
    let head = spine_angle(HEAD_AT, t) + 0.05 * (TAU * t / CYCLE).sin();
    out.push(Placement {
        part: HEAD,
        joint: [HEAD_AT * PITCH, spine_y(rig, HEAD_AT, t)],
        pivot: rig.pivots[HEAD],
        angle: head,
        scale: BODY_SCALE,
        shade: 1.0,
    });
}

/// Wraps a phase into `[0, 1)`.
fn wrap(phase: f32) -> f32 {
    phase - phase.floor()
}

/// How far behind the last segment's middle the animal ends: the tail's
/// root plus the tail's own reach, or the last chevron's edge once the
/// tail is still growing. The example needs it to tell when the animal has
/// really left the window, not just its head.
pub fn rear(rig: &Rig, segments: &[Segment]) -> f32 {
    let Some(last) = segments.last() else {
        return 0.0;
    };
    (segments.len() as f32 - 1.0) * PITCH + TAIL_BACK + rig.tail_reach * last.grow
}
