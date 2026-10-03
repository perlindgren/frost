//! The centipede's rig: the pure geometry that turns the six cut-out
//! `Monster_*` parts into an animal that walks, **seen from above**.
//!
//! Nothing here touches the scene graph or a GPU: [`pose`] takes the
//! segments the animal is built from and a time, and hands back a flat list
//! of [`Placement`]s — one per sprite — in the order they should be drawn.
//! The example turns those into scene nodes; a software renderer can just as
//! well composite them into a picture, which is how the layout was checked
//! without opening a window.
//!
//! # The space
//!
//! Everything is in the **monster's own space**: `x` along the body with `+x`
//! pointing *backward*, because the animal faces `-x` and crawls to the left,
//! and `y` across it, which on screen is simply up and down. The straight
//! line it tracks along is `y = 0`. There is no height and no ground plane to
//! stand on: the picture *is* the ground, and one unit is one texture pixel
//! of the leg part, so the example scales the whole animal once, at the end.
//!
//! Seen this way the parts read differently from the sheet they were cut
//! from. The chevrons are the plates of a back, the skull leads it, the spike
//! cluster trails behind — and each leg is a whole leg, hip bar to clay foot
//! in one sprite, turned about its hip like the hand of a clock. The row of
//! legs along the bottom of the frame is one side of the animal and the row
//! along the top the other, drawn from the same sprite with a negative `y`
//! scale.
//!
//! # The legs drive the walk
//!
//! A leg is rigid: one sprite, one pivot, one angle. Its foot can therefore
//! only ever be somewhere on a circle of radius [`Rig::leg_len`] around its
//! hip, and that constraint is the gait rather than a limit on it.
//! [`foot_target`] says where a foot is allowed to be and the leg's angle
//! falls out of it; no sprite is animated on its own.
//!
//! A foot alternates between
//!
//! - **stance** — planted on the ground and sliding *backward* through
//!   monster space, and
//! - **swing** — released, coming forward again on a shortened radius, which
//!   from straight above is what a lifted foot looks like: nearer the eye,
//!   further from the ground it was pressing on, so drawn foreshortened.
//!
//! A planted foot is stationary in the *world*, and monster space slides over
//! the world at [`CRAWL_SPEED`], so the foot has to slide backward through
//! monster space at exactly that speed. That is what [`STRIDE`] =
//! `CRAWL_SPEED × DUTY × CYCLE` means: one stance covers precisely the ground
//! the animal travels while it lasts, so the feet grip instead of skating.
//!
//! And because the leg cannot stretch, a foot sliding backward at a steady
//! speed does **not** swing at a steady angle: it sweeps fastest as it passes
//! under the hip and slows as the leg flattens out at either end of the
//! stance. That is not a flaw to be smoothed away — it is what a rigid leg
//! pushing an animal along does, and it is why the body's speed and the legs'
//! angles are one fact here instead of two that have to be reconciled frame
//! by frame.
//!
//! The same rigidity is also why a foot's path across the ground is an arc
//! and not a straight line: the leg's length fixes how far out the foot
//! stands, so the foot comes in toward the body at both ends of its stance,
//! and the hip it hangs from is itself riding the body's sideways wave. What
//! stays exact is the fore-and-aft grip, which is the half of a step the eye
//! actually judges a walk by.
//!
//! Legs do not all step at once: leg `i` lags leg `i - 1` by [`LAG`] of a
//! cycle, which is a metachronal wave — the ripple of steps runs from the
//! head to the tail, the same direction the body's [`WAVE_AMP`] sideways
//! ripple travels in, so the animal and its legs move as one creature. The
//! two sides are [`SIDE_PHASE`] of a cycle apart from each other, which is
//! what makes the body rock from side to side as it goes instead of sliding
//! along square to its track.

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
/// A whole leg: hip bar, thigh, shin and clay foot in one sprite, turned
/// about its hip.
pub const LEG: usize = 5;
/// How many parts the rig knows, and how long the example's shape table is.
pub const PART_COUNT: usize = 6;

/// The part files, in the order of the indices above.
///
/// `Monster_leg1.png` is not one of the sheet's crops. The art's leg was
/// drawn in two pieces, thigh and shin, with a hundred rows of its middle
/// hidden behind a body plate — and the two pieces were left in two
/// byte-identical files. They are welded here tip to tip, on the row where
/// each tapers to its broken end, into one complete leg: the same join the
/// two-sprite rig used to make every frame by pivoting them at those tips,
/// now baked into the texture.
pub const PART_FILES: [&str; PART_COUNT] = [
    "Monster_head.png",
    "Monster_body1.png",
    "Monster_body2.png",
    "Monster_body3.png",
    "Monster_tail.png",
    "Monster_leg1.png",
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
/// rides the body's wave. Per variant, because the three chevrons are cut
/// from the sheet at different heights.
const SPINE_ANCHOR: [Px; 3] = [[247.0, 408.0], [225.0, 336.0], [198.0, 362.0]];

/// The hip: inside the leg's horizontal bar, where the bar meets the thigh.
/// The one pivot a whole leg gets.
const HIP: Px = [88.0, 86.0];

/// Where the foot works the ground: the middle of the flat clay pad at the
/// leg's end. The leg is swung so that this point lands where the gait wants
/// a foot, which is why the pad's own tilt comes out of the leg's angle
/// instead of having to be posed.
const TOE: Px = [223.0, 895.0];

// --------------------------------------------------------------- layout --

/// The body scale: the head, the segments and the tail are drawn at this
/// fraction of their texture size. The chevrons are about as tall as a leg is
/// long in the art, which would make a body with no reach to its legs; at
/// `0.7` the plates still overlap each other as they do in the sheet, and the
/// animal is a band of back with room on either side for legs to be seen
/// reaching out of it.
pub const BODY_SCALE: f32 = 0.7;

/// The head's scale on top of [`BODY_SCALE`]. The skull is drawn with a ruff
/// of spikes as wide as a plate is tall, which from the side reads as a heavy
/// head; from above the same ruff is simply wider than the body, so the head
/// is kept a little smaller than the segment it leads.
pub const HEAD_SCALE: f32 = 0.86;

/// The legs' scale. From above a leg's whole length lies across the body's
/// width instead of hanging clear of it below, and the clay leg is longer than
/// the body is wide: at its drawn size a leg would throw a ring of limbs twice
/// the animal's breadth, which reads as a wreath and not as a centipede.
///
/// `0.3` is a leg barely long enough to clear the plates it grows out of — the
/// feet land just outside the body's edge, the way a centipede's do, and the
/// animal stays a body with a fringe of legs instead of a body standing on a
/// paddling pool of them.
pub const LEG_SCALE: f32 = 0.3;

/// How far apart the segments sit along the body, in monster units — about
/// half a chevron, so consecutive plates overlap by a head's width.
pub const PITCH: f32 = 220.0;

/// The hip sits this far behind the middle of its segment, matching where the
/// clay legs were planted in the sheet. At that distance along the body the
/// plate *ahead* of the hip is still near its widest, which is what covers the
/// hip bar.
pub const HIP_BACK: f32 = 25.0;

/// How far each side's hips sit out from the spine, in monster units: barely
/// half the distance to the body's edge, so the hip bar and most of the thigh
/// are buried under the overlapping plates and each leg seems to grow straight
/// out of the animal's side.
pub const HIP_OUT: f32 = 120.0;

/// The slack between where a foot stands and how far its clay pad overruns
/// that: what the animal's lateral span is measured with.
pub const FOOT_PAD: f32 = 90.0;

/// The head's neck sits this many segments along the body from segment 0,
/// positive being backward — so past the first plate's apex, not in front of
/// it. [`pose`] emits the head last, and the art draws the skull overlapping
/// the plate it leads: the neck has to be behind the apex for the ruff of
/// spikes to lie over it instead of butting up against the tip.
pub const HEAD_AT: f32 = 0.3;

/// The tail's root sits this far beyond the rear edge of the plate it tucks
/// behind, so it trails the body without either gapping or burying the plate's
/// own edge.
pub const TAIL_BACK: f32 = 24.0;

// ----------------------------------------------------------------- gait --

/// How fast the animal travels along its track, monster units per second — a
/// little more than one segment per second.
pub const CRAWL_SPEED: f32 = 260.0;

/// One full step cycle of a single foot, in seconds.
///
/// This is not free with a short leg: the cycle sets the stride at a given
/// speed, and a foot can only be dragged as far along the ground as its leg is
/// long. Half a stride has to stay a comfortable share of [`Rig::leg_len`] or
/// the leg would lie flat fore and aft at the ends of its stance, so a short
/// leg means a quick cycle — which is the right way round anyway, the shorter
/// an animal's legs the faster it has to move them to go anywhere.
pub const CYCLE: f32 = 1.1;

/// The share of a cycle a foot spends planted on the ground. The rest is the
/// swing, and six tenths planted keeps several feet gripping the ground at all
/// times.
pub const DUTY: f32 = 0.6;

/// The distance a planted foot travels relative to the body during its stance,
/// in monster units: exactly the ground covered while it lasts, so a planted
/// foot does not slide. See the module docs.
pub const STRIDE: f32 = CRAWL_SPEED * DUTY * CYCLE;

/// How much shorter a swinging leg is drawn, as a fraction of its length. The
/// lift of the gait, in the only currency a top-down view has: a foot raised
/// off the ground is nearer the eye and shorter in projection, so the leg
/// contracts as the foot comes forward and snaps back to full length as it
/// lands.
pub const SWING_LIFT: f32 = 0.16;

/// The gait's lag between one segment's legs and the next, as a fraction of
/// [`CYCLE`]: the step ripples from head to tail, about eight segments per
/// full wave.
pub const LAG: f32 = 0.125;

/// How far apart the two sides' steps are, as a fraction of [`CYCLE`]: half a
/// cycle, so when one side's foot is planted its partner's is swinging, and
/// the body rocks along its track instead of sliding square to it.
pub const SIDE_PHASE: f32 = 0.5;

/// The body wave's amplitude, in monster units: how far a segment rides to one
/// side or the other of the track's centre line. Kept well short of the
/// distance from a hip to the body's edge, so a hip stays under its plate as
/// the wave rolls, and the swing of a short leg is not lost in the sway.
pub const WAVE_AMP: f32 = 32.0;

/// The body wave's wavelength in segments — eight of them, the same span
/// [`LAG`] spreads the step wave over, so the body's ripple and the ripple of
/// steps beside it travel together.
pub const WAVE_SEGS: f32 = 8.0;

/// The smallest leg scale a segment's legs are born at, right after the animal
/// regrows a segment; they reach full size with the segment.
pub const GROW_START: f32 = 0.55;

/// How long a new segment takes to reach full size, in seconds.
pub const GROW_SECONDS: f32 = 1.2;

// ---------------------------------------------------------------- types --

/// One body segment of the animal: which of the three chevron variants it
/// is (a [`BODIES`] index), and how far it has grown — `0.0` just born, `1.0`
/// full size.
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
    /// centred on its node, so this is what has to be subtracted to hang the
    /// part from its joint instead of its middle.
    pub pivot: [f32; 2],
    /// The tilt in radians, counter-clockwise, `0.0` being the pose the part
    /// was cut in.
    pub angle: f32,
    /// The scale about the joint, and the side of the body this part belongs
    /// to: a negative `y` mirrors the sprite about the body's line, which is
    /// how the legs on one side of the frame are the same sprite as the legs
    /// on the other.
    pub scale: [f32; 2],
}

/// The rig's derived numbers, from the textures' real sizes.
#[derive(Clone, Copy, Debug)]
pub struct Rig {
    /// Each part's joint in its own local space, indexed by part. The body
    /// variants each carry their own spine point.
    pub pivots: [[f32; 2]; PART_COUNT],
    /// The leg's length: hip to the middle of its foot pad, in local units.
    /// Rigid — every foot the animal puts down is exactly this far from its
    /// hip, and that is what shapes the walk.
    pub leg_len: f32,
    /// The direction the leg points as it was drawn, in radians: what the
    /// leg's rotation is measured from.
    pub leg_rest: f32,
    /// How far the nose reaches in front of the neck, at [`HEAD_SCALE`].
    pub nose: f32,
    /// How far the tail reaches behind its root, at [`BODY_SCALE`].
    pub tail_reach: f32,
    /// How far each plate's rear edge lies behind its spine point, at
    /// [`BODY_SCALE`], per variant: where the tail has to tuck up against.
    pub plate_back: [f32; 3],
}

impl Rig {
    /// Derives the joints and the leg's length from the parts' texture sizes,
    /// so the anchor table above stays in image pixels and nothing has to be
    /// recomputed by hand when the art is re-cropped.
    pub fn new(sizes: [[f32; 2]; PART_COUNT]) -> Self {
        let pivot = |px: Px, size: [f32; 2]| frost::Transform::anchor(px, size);
        let pivots = [
            pivot(NECK, sizes[HEAD]),
            pivot(SPINE_ANCHOR[0], sizes[BODIES[0]]),
            pivot(SPINE_ANCHOR[1], sizes[BODIES[1]]),
            pivot(SPINE_ANCHOR[2], sizes[BODIES[2]]),
            pivot(TAIL_ROOT, sizes[TAIL]),
            pivot(HIP, sizes[LEG]),
        ];
        // The leg points from its hip to its foot, as drawn, in local y-up
        // units: the one vector a rigid leg is defined by.
        let hip = pivots[LEG];
        let toe = pivot(TOE, sizes[LEG]);
        let toe = [toe[0] - hip[0], toe[1] - hip[1]];
        Self {
            pivots,
            leg_len: (toe[0] * toe[0] + toe[1] * toe[1]).sqrt(),
            leg_rest: toe[1].atan2(toe[0]),
            nose: (NECK[0] - 3.0) * BODY_SCALE * HEAD_SCALE,
            tail_reach: (sizes[TAIL][0] - TAIL_ROOT[0]) * BODY_SCALE,
            plate_back: BODIES.map(|b| (sizes[b][0] - SPINE_ANCHOR[b - BODIES[0]][0]) * BODY_SCALE),
        }
    }

    /// The radius a leg of the given scale can put its foot at, measured from
    /// its hip. Rigid, so this is the only reach there is.
    pub fn reach(&self, scale: f32) -> f32 {
        self.leg_len * scale
    }

    /// How far out from the track's centre line a leg of the given scale
    /// stands its foot when the leg points straight out: the hip's own offset
    /// plus the leg's length, so the animal's stance — and with it the window
    /// fit — follows the art's leg length instead of being a second, unrelated
    /// number. A foot's actual distance falls short of this as the leg swings;
    /// see [`foot_target`].
    pub fn foot_out(&self, scale: f32) -> f32 {
        HIP_OUT + self.reach(scale)
    }

    /// Half the animal's lateral span: the outermost reach of a full-size
    /// foot, pad included. The example maps this onto the window's height,
    /// because from above that is the dimension the animal cannot exceed.
    pub fn half_span(&self) -> f32 {
        self.foot_out(LEG_SCALE) + FOOT_PAD * LEG_SCALE
    }
}

// ----------------------------------------------------------------- pose --

/// Where the body's wave puts segment position `i`, which may be fractional
/// (the head rides it a little way behind the first plate's apex): the
/// sideways offset from the track's centre line.
fn spine_off(i: f32, t: f32) -> f32 {
    WAVE_AMP * (TAU * (i / WAVE_SEGS - t / CYCLE)).sin()
}

/// The tilt the body wave's slope implies at segment position `i`: a plate
/// turns into the curve it is riding.
fn spine_angle(i: f32, t: f32) -> f32 {
    (WAVE_AMP * (TAU / (WAVE_SEGS * PITCH)) * (TAU * (i / WAVE_SEGS - t / CYCLE)).cos()).atan()
}

/// Where a foot of the given phase sits: `(how far fore or aft of its hip,
/// positive being forward; how far out from its hip the foot stands)`.
///
/// `phase` is the fraction through the foot's own cycle, in `[0, 1)`. The
/// stance sweeps from half a [`STRIDE`] ahead of the hip to half a stride
/// behind it at exactly [`CRAWL_SPEED`]; the swing returns to the front over
/// the rest of the cycle on a radius shortened by [`SWING_LIFT`], so the foot
/// reads as lifted. Both halves meet at the same point at each end, so the
/// foot never teleports.
///
/// Only the fore-and-aft figure is chosen. The outward one is *solved*: a leg
/// of length `reach` with its foot `x` off to one side must have that foot
/// `√(reach² − x²)` out, so a foot bows in toward the body at both ends of its
/// stance and reaches full stretch as it passes under the hip. Set both by
/// hand and the clay leg would visibly stretch and shrink twice a cycle.
fn foot_target(rig: &Rig, phase: f32, scale: f32) -> (f32, f32) {
    let swing = phase >= DUTY;
    let x = if swing {
        STRIDE * (0.5 - (phase - DUTY) / (1.0 - DUTY))
    } else {
        STRIDE * (phase / DUTY - 0.5)
    };
    let reach = rig.reach(scale)
        * if swing {
            1.0 - SWING_LIFT * ((phase - DUTY) / (1.0 - DUTY) * PI).sin()
        } else {
            1.0
        };
    // A stride longer than a leg is tall could not be walked at all, so the
    // foot stops at the leg's own reach: a mis-set constant becomes a
    // flattened stance rather than a square root of a negative.
    let x = x.clamp(-reach * 0.999, reach * 0.999);
    (x, (reach * reach - x * x).sqrt())
}

/// Places one leg: appends its single sprite to `out`, swung so that its foot
/// lands where [`foot_target`] puts it for that `phase`.
///
/// `side` is `1.0` for the row of legs along the top of the frame and `-1.0`
/// for the row along the bottom. There is one leg in this file, not two: the
/// foot is placed out on the `+y` side of its hip, and `side` mirrors the
/// outcome — the joint's `y`, the angle, and the sign of the `y` scale all turn
/// with it. That is the whole trick behind the second row: the same sprite,
/// reflected about the body's line.
fn place_leg(
    out: &mut Vec<Placement>,
    rig: &Rig,
    hip: [f32; 2],
    side: f32,
    phase: f32,
    scale: f32,
) {
    // `hip` arrives in monster space, already on its own side; `h` is the same
    // hip in the solve's frame, always out on `+y`.
    let h = [hip[0], side * hip[1]];
    let (x, out_from_hip) = foot_target(rig, phase, scale);
    // The foot, and then the one angle that puts it there: a leg drawn
    // pointing along `leg_rest` has to turn to the leg's real direction to
    // land its foot where the gait wants it.
    let foot = [h[0] - x, h[1] + out_from_hip];
    let angle = (foot[1] - h[1]).atan2(foot[0] - h[0]) - rig.leg_rest;

    out.push(Placement {
        part: LEG,
        joint: [h[0], side * h[1]],
        pivot: rig.pivots[LEG],
        angle: side * angle,
        scale: [scale, side * scale],
    });
}

/// Wraps a phase into `[0, 1)`.
fn wrap(phase: f32) -> f32 {
    phase - phase.floor()
}

/// Fills `out` with the whole animal at time `t`, back to front: both rows of
/// legs first, then the tail, the segments from the rear in, and the head last
/// so its spikes lap over the plate it leads.
///
/// The legs go under the plates because that is where the art puts them: the
/// hip bar belongs to the animal, and the plate that crosses it belongs on
/// top.
pub fn pose(rig: &Rig, segments: &[Segment], t: f32, out: &mut Vec<Placement>) {
    out.clear();

    // Both rows of legs, and within each row the rear segments first, so that
    // where two segments' legs overlap the one nearer the head wins.
    for side in [-1.0, 1.0] {
        // The two sides half a cycle apart: a foot planted on one side while
        // its partner swings on the other.
        let offset = if side > 0.0 { 0.0 } else { SIDE_PHASE };
        for (i, seg) in segments.iter().enumerate().rev() {
            let f = i as f32;
            place_leg(
                out,
                rig,
                [
                    f * PITCH + HIP_BACK,
                    side * (HIP_OUT * seg.grow + spine_off(f, t)),
                ],
                side,
                wrap(t / CYCLE - f * LAG + offset),
                seg.grow * LEG_SCALE,
            );
        }
    }

    // The tail trails the last segment, tucked under it.
    let last = segments.len().saturating_sub(1) as f32;
    let grow = segments.last().map_or(1.0, |s| s.grow);
    let back = segments
        .last()
        .map_or(0.0, |s| rig.plate_back[s.body - BODIES[0]] * s.grow);
    out.push(Placement {
        part: TAIL,
        joint: [last * PITCH + back + TAIL_BACK, spine_off(last + 0.6, t)],
        pivot: rig.pivots[TAIL],
        angle: spine_angle(last + 0.6, t) + 0.05 * (TAU * t / CYCLE).sin(),
        scale: [BODY_SCALE * grow, BODY_SCALE * grow],
    });

    // The plates, rear first, each turning into the wave it rides.
    for (i, seg) in segments.iter().enumerate().rev() {
        let f = i as f32;
        out.push(Placement {
            part: seg.body,
            joint: [f * PITCH, spine_off(f, t)],
            pivot: rig.pivots[seg.body],
            angle: spine_angle(f, t),
            scale: [BODY_SCALE * seg.grow, BODY_SCALE * seg.grow],
        });
    }

    // The head leads, nodding on top of the wave it rides — and last of all,
    // so its ruff lies over the first plate instead of stopping at its tip.
    let head = spine_angle(HEAD_AT, t) + 0.05 * (TAU * t / CYCLE).sin();
    out.push(Placement {
        part: HEAD,
        joint: [HEAD_AT * PITCH, spine_off(HEAD_AT, t)],
        pivot: rig.pivots[HEAD],
        angle: head,
        scale: [BODY_SCALE * HEAD_SCALE, BODY_SCALE * HEAD_SCALE],
    });
}

/// How far behind the front segment's middle the animal's rear tip sits, in
/// monster units: the example needs it to know when the whole animal has left
/// the window, and the tail is the last of it to go.
pub fn rear(rig: &Rig, segments: &[Segment]) -> f32 {
    let last = segments.len().saturating_sub(1) as f32;
    let back = segments
        .last()
        .map_or(0.0, |s| rig.plate_back[s.body - BODIES[0]] * s.grow);
    last * PITCH + back + TAIL_BACK + rig.tail_reach
}
