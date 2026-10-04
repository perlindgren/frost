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
//! cluster trails behind — and a leg is one sprite, one pivot, turned like the
//! hand of a clock: the art's shin with its clay foot, swung from its broken
//! upper end where the sheet's body used to cover it. The row of legs along
//! the bottom of the frame is one side of the animal and the row along the top
//! the other, drawn from the same sprite with a negative `y` scale.
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
/// A leg: one sprite, turned about its upper end.
pub const LEG: usize = 5;
/// The pressed print: [`Monster_foot.png`](assets/sprites/Monster_foot.png),
/// the square clay footprint a lifted foot leaves in the soil. The rig
/// never poses it — it only measures it — and the example stamps these
/// world-side where the feet lifted.
pub const FOOT: usize = 6;
/// How many parts the rig knows, and how long the example's shape table is.
pub const PART_COUNT: usize = 7;

/// The part files, in the order of the indices above.
///
/// The art's leg is in two pieces, thigh in one file and shin with its clay
/// foot in the other, the middle of the limb hidden behind a body plate in the
/// sheet. The centipede draws one sprite per leg and takes the piece that
/// carries the foot, because a gait is a thing a foot does: `Monster_leg1.png`
/// is the thigh, and nothing in this example draws it.
pub const PART_FILES: [&str; PART_COUNT] = [
    "Monster_head.png",
    "Monster_body1.png",
    "Monster_body2.png",
    "Monster_body3.png",
    "Monster_tail.png",
    "Monster_leg2.png",
    "Monster_foot.png",
];

// -------------------------------------------------------------- joints --
// Measured on the cropped textures, in the texture pixels of `Px`. The
// numbers are the visible tips and corners of each part, so a joint can be
// checked against the art by eye.

/// The head's neck, at the middle of its rear edge: where the skull meets
/// the first segment.
const NECK: Px = [672.0, 470.0];

/// The tail's root, at the middle of its left edge, where it is laid over the
/// last segment — see [`TAIL_BACK`].
const TAIL_ROOT: Px = [22.0, 122.0];

/// Each body variant's spine point: the apex of its chevron for `x`, and the
/// middle of the cut for `y`. Per variant, because the three chevrons are cut
/// from the sheet at different heights and different sizes.
///
/// The `y` is the middle of the cut rather than the apex row, which is where
/// the sheet draws the ridge. A plate hung on its ridge hangs by less than
/// half: the largest chevron reaches 185 units above the spine and only 158
/// below it, so the same legs would be buried 27 units deeper on one side than
/// on the other, and the animal would crawl with one row of legs tucked under
/// its flanks and the other row hanging off them. Hung by the middle instead,
/// the three variants reach 170/173, 152/152 and 156/156 above and below, and
/// both rows of hips sit the same depth under the same plate.
const SPINE_ANCHOR: [Px; 3] = [[247.0, 386.5], [225.0, 347.5], [198.0, 358.5]];

/// The hip: the leg sprite's broken upper end, a few pixels inside the tip
/// where the fragment tapers to four pixels across. It is the only pivot the
/// leg has, and the sheet's body plate is what hides it.
const HIP: Px = [22.0, 12.0];

/// Where the foot works the ground: the middle of the flat clay pad at the
/// leg's end. The leg is swung so that this point lands where the gait wants
/// a foot, which is why the pad's own tilt comes out of the leg's angle
/// instead of having to be posed.
const TOE: Px = [201.0, 452.0];

/// The middle of the clay pad as *muchness*, not as contact: the centroid
/// of the pad band at the leg's foot, measured off `Monster_leg2.png`
/// (the band's alpha centroid is `(198, 431)` — three pixels from the
/// contact point above, the rig's `TOE` sitting a little low in the clay
/// where the foot presses hardest). This is where a print is stamped:
/// the pressed footprint belongs under the pad's bulk, not merely under
/// its lowest point.
///
/// The pad's long axis measures 1.4 degrees off the sprite's own x, and
/// the print sprite's axis measures 1.9 degrees off its x the same way —
/// half a degree between them, which is nothing against the tens of
/// degrees a swinging leg swings through. So a print laid down with the
/// pressing leg's own rotation matches the foot that pressed it: no
/// separate foot angle exists to pose.
const PAD_CENTROID: Px = [198.0, 431.0];

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
/// `0.46` is a leg barely long enough to clear the plates it grows out of —
/// the feet land just outside the body's edge, the way a centipede's do, and
/// the animal stays a body with a fringe of legs instead of a body standing on
/// a paddling pool of them. The number looks larger than the leg is: this
/// sprite is the shin and foot alone, some three-fifths of the limb the sheet
/// shows, so it is drawn nearer its own size to reach as far as the whole limb
/// did at `0.3`.
pub const LEG_SCALE: f32 = 0.46;

/// How far apart the segments sit along the body, in monster units — about
/// half a chevron, so consecutive plates overlap by a head's width.
pub const PITCH: f32 = 220.0;

/// The hip sits this far behind the middle of its segment. Not where the sheet
/// planted the clay legs — 25 back is right behind the apex, where a chevron is
/// still a wedge and a hip there pokes out of the plate's edge. At 55 the plate
/// is roughly a fifth broader on both sides, and a hip buried under it stays
/// under it while the body waves and the plate tilts.
pub const HIP_BACK: f32 = 55.0;

/// How far each side's hips sit out from the spine, in monster units: two
/// thirds of the way to the plate's edge, so each leg's root is buried under
/// the plate it grows from and the leg appears to start at the animal's flank
/// rather than a hand's width below it. This distance is measured from the
/// spine and never altered by the wave, which moves the spine and the legs
/// along with it.
pub const HIP_OUT: f32 = 100.0;

/// The slack between where a foot stands and how far its clay pad overruns
/// that: what the animal's lateral span is measured with.
pub const FOOT_PAD: f32 = 90.0;

/// The head's neck sits this many segments along the body from segment 0,
/// positive being backward — so past the first plate's apex, not in front of
/// it. [`pose`] emits the head last, and the art draws the skull overlapping
/// the plate it leads: the neck has to be behind the apex for the ruff of
/// spikes to lie over it instead of butting up against the tip.
pub const HEAD_AT: f32 = 0.3;

/// The tail's root sits this far *inside* the rear edge of the plate it lies
/// over — negative, because a tail parked clear of the body it belongs to reads
/// as a separate object trailing a fan of spikes. [`pose`] emits the tail after
/// the plates, so the overlap is drawn rather than hidden: the stem lies across
/// the last plate the way the skull's ruff lies across the first one, which is
/// the only thing that makes the cluster look attached.
///
/// `60` is about the width of the tail's narrow stem — the first ~40 units of it
/// — plus room for the join to stay closed while the tail rides the wave a
/// fraction of a segment behind the plate it rests on. Less and the stem's break
/// shows on top of the plate; more and the cluster starts climbing the body.
///
/// It is a fixed distance and the tail is a fixed size, unlike a segment's
/// plate and legs, and that is the point: the tail is one organ attached to the
/// rear of the body, not a part of whichever segment happens to be last. Were
/// it scaled by that segment's growth, adding a segment would shrink the tail
/// to [`GROW_START`] in one frame and move it forward with it — a pop at the
/// worst possible moment, right where the growth is supposed to be read.
pub const TAIL_BACK: f32 = -60.0;

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
/// side or the other of the track's centre line, carrying its plate and both of
/// its legs the same distance. Kept well short of the distance from a hip to
/// the body's edge, so a hip stays under its plate as the wave rolls, and the
/// swing of a short leg is not lost in the sway. Because the whole animal rides
/// this far off the centre line at the crest of a wave, it is also what
/// [`Rig::half_span`] measures the fit by.
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
/// is (a [`BODIES`] index), and how far it has grown, from [`GROW_START`] just
/// born to `1.0` full size — which also decides how far out of the body it has
/// come, through [`slide`].
#[derive(Clone, Copy, Debug)]
pub struct Segment {
    /// The part index of this segment's chevron, one of [`BODIES`].
    pub body: usize,
    /// The growth fraction, ramped from [`GROW_START`] to `1.0`.
    pub grow: f32,
}

/// How far a segment has come out from the one in front of it: `0.0` at its
/// birth, when it sits on its predecessor's slot and is hidden under that
/// plate, `1.0` when it has taken its own.
///
/// This is derived from [`Segment::grow`] rather than carried beside it, so the
/// two cannot drift apart: a segment slides out of the body at exactly the rate
/// it swells, which is what makes a new segment read as growing rather than as
/// arriving.
fn slide(seg: &Segment) -> f32 {
    ((seg.grow - GROW_START) / (1.0 - GROW_START)).clamp(0.0, 1.0)
}

/// Where the middle of segment `i` sits along the body, in monster units, with
/// the head-end segment at zero.
///
/// Every gap between two segments is a fraction of [`PITCH`]: the whole pitch
/// once both have settled, less while the one behind is still arriving. Without
/// this the body lengthens by a full pitch the instant a segment is added, the
/// tail jumps back with it, and the growth is a hiccup; with it the animal
/// stretches backwards, one smooth span, across the whole of [`GROW_SECONDS`].
fn slot(segments: &[Segment], i: usize) -> f32 {
    segments
        .get(1..=i)
        .map(|behind| behind.iter().map(|s| PITCH * slide(s)).sum())
        .unwrap_or(0.0)
}

/// The rearmost edge of the body's plates — of whichever segment is currently
/// the last one out, which during a growth is still the segment the newcomer
/// was born behind, because a half-size plate does not reach as far back as a
/// whole one. The tail hangs off this rather than off the last segment's slot,
/// so adding a segment cannot move it: the newcomer has to grow far enough out
/// to overtake the plate in front before the tail starts travelling at all.
fn rear_edge(rig: &Rig, segments: &[Segment]) -> f32 {
    let mut rear = 0.0f32;
    for (i, seg) in segments.iter().enumerate() {
        let edge = slot(segments, i) + rig.plate_back[seg.body - BODIES[0]] * seg.grow;
        rear = rear.max(edge);
    }
    rear
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
    /// The pad's middle in the leg sprite's own local space, from
    /// [`PAD_CENTROID`]: the point a print is stamped at, through the same
    /// scale-turn-and-hang the leg node gives every other point of the art.
    pub pad_centroid: [f32; 2],
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
            // The print hangs from its own middle: the pad's centroid in
            // the foot sprite measures a hair off its texture's centre.
            pivot([sizes[FOOT][0] / 2.0, sizes[FOOT][1] / 2.0], sizes[FOOT]),
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
            pad_centroid: pivot(PAD_CENTROID, sizes[LEG]),
            nose: (NECK[0] - 3.0) * BODY_SCALE * HEAD_SCALE,
            tail_reach: (sizes[TAIL][0] - TAIL_ROOT[0]) * BODY_SCALE,
            plate_back: BODIES.map(|b| (sizes[b][0] - SPINE_ANCHOR[b - BODIES[0]][0]) * BODY_SCALE),
        }
    }

    /// The pressing leg's pad middle, in monster space: the point of the
    /// leg's art that [`PAD_CENTROID`] names, taken through the same
    /// scale-then-turn the leg's own node gives every point of that art —
    /// so it stands exactly under the pad, mirrored row included, and it
    /// is the spot a print is stamped at.
    pub fn pad_of(&self, leg: &Placement) -> [f32; 2] {
        let pad = self.pad_centroid;
        let turned = frost::Transform::rotate(leg.angle).apply([
            (pad[0] - leg.pivot[0]) * leg.scale[0],
            (pad[1] - leg.pivot[1]) * leg.scale[1],
        ]);
        [leg.joint[0] + turned[0], leg.joint[1] + turned[1]]
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
        // The feet, the clay pads that overrun them, and the whole ripple the
        // legs ride on: the row is carried as far as [`WAVE_AMP`] off the track
        // centre, so at the crest of a wave the outermost foot stands further
        // out than it does on the level.
        self.foot_out(LEG_SCALE) + FOOT_PAD * LEG_SCALE + WAVE_AMP
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
/// The fore-and-aft sense is the whole grip, and a sign here is worth
/// stating plainly because its inversion is invisible in the walk and fatal
/// to the trail: the planted foot must move *aft* through monster space —
/// `x` falling with the phase — because the body crawls forward over it at
/// exactly that rate in the world. Reverse it and the stance drags the foot
/// forward at twice the animal's speed, the foot leaves the ground a stride
/// ahead of where it pressed, and the prints land in front of the animal
/// that is about to walk into them. [`a_planted_foot_holds_the_ground`]
/// holds the line.
///
/// Only the fore-and-aft figure is chosen. The outward one is *solved*: a leg
/// of length `reach` with its foot `x` off to one side must have that foot
/// `√(reach² − x²)` out, so a foot bows in toward the body at both ends of its
/// stance and reaches full stretch as it passes under the hip. Set both by
/// hand and the clay leg would visibly stretch and shrink twice a cycle.
fn foot_target(rig: &Rig, phase: f32, scale: f32) -> (f32, f32) {
    let swing = phase >= DUTY;
    // Aft is negative and the stance must fall: from half a stride ahead
    // of the hip (at `phase` 0, where the swing just set the foot down)
    // to half a stride behind it (at `DUTY`, where it lifts) — the exact
    // drag the crawl runs over. See the grip note above.
    let x = if swing {
        STRIDE * ((phase - DUTY) / (1.0 - DUTY) - 0.5)
    } else {
        STRIDE * (0.5 - phase / DUTY)
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

/// One leg this frame: the phase its foot is at, and the placement that
/// draws it.
#[derive(Clone, Copy, Debug)]
pub struct LegState {
    /// Which side of the body the leg grows from: `1.0` the top row of the
    /// frame, `-1.0` the bottom.
    pub side: f32,
    /// Which segment it grows from, head-end at zero.
    pub segment: usize,
    /// The foot's cycle phase in `[0, 1)`, the same number [`place_leg`]
    /// swung it with.
    pub phase: f32,
    /// Whether the segment this leg grows from is still swelling — a
    /// growing leg stands short of its full stance, so its prints would
    /// land inside the rows ([`pressed_prints`] passes on them).
    pub growing: bool,
    /// The leg's placement.
    pub placement: Placement,
}

/// Both rows of legs at time `t`, in the order [`pose`] draws them, each
/// with its phase beside it. [`pose`] draws its legs from here, so nothing
/// can drift between the walk and whatever reads the phases — the print
/// trail in [`pressed_prints`], or a test counting lifts.
pub fn leg_states(rig: &Rig, segments: &[Segment], t: f32) -> Vec<LegState> {
    let mut states = Vec::with_capacity(2 * segments.len());
    for side in [-1.0, 1.0] {
        // The two sides half a cycle apart: a foot planted on one side while
        // its partner swings on the other.
        let offset = if side > 0.0 { 0.0 } else { SIDE_PHASE };
        for (i, seg) in segments.iter().enumerate().rev() {
            let x = slot(segments, i);
            let f = x / PITCH;
            let phase = wrap(t / CYCLE - f * LAG + offset);
            let mut one = Vec::with_capacity(1);
            place_leg(
                &mut one,
                rig,
                [
                    x + HIP_BACK,
                    // The wave *carries* the leg row sideways, exactly as it
                    // carries the plate the leg grows from: the hip sits out
                    // `HIP_OUT` from a spine that has moved, and the same
                    // `spine_off` is added to both sides.
                    side * HIP_OUT * seg.grow + spine_off(f, t),
                ],
                side,
                phase,
                seg.grow * LEG_SCALE,
            );
            states.push(LegState {
                side,
                segment: i,
                phase,
                growing: seg.grow < 1.0,
                placement: one[0],
            });
        }
    }
    states
}

/// A print pressed since the last reading: the placement of the leg whose
/// foot lifted — its joint, rotation and scale are what the print is
/// stamped with — and which of the animal's feet it was.
#[derive(Clone, Copy, Debug)]
pub struct Print {
    /// The pressing leg's side, `1.0` top, `-1.0` bottom.
    pub side: f32,
    /// The segment its leg grows from.
    pub segment: usize,
    /// The pressing leg's placement, at the frame of the lift.
    pub placement: Placement,
}

/// Every foot that lifted between the frame `prev` remembers and this
/// one: a print is pressed the frame a phase leaves stance, `prev` short
/// of [`DUTY`] and the phase now past it — and only that frame, since
/// `prev` is retaken to the current phases on the way out. A phase that
/// wrapped past `1.0` crossed nothing; a body whose leg count changed
/// (a segment grown, or a new animal) is retaken whole without pressing,
/// so a print never lands on a leg that was not there to lift.
///
/// A leg whose segment is still growing walks but does not print: its
/// whole stance — hip height included — is scaled by the growth, so a
/// print pressed mid-swell lands short of its row, an outlier the
/// animal never stood at. The newcomer's feet join the rows with the
/// segment's first full-size step.
pub fn pressed_prints(rig: &Rig, segments: &[Segment], t: f32, prev: &mut Vec<f32>) -> Vec<Print> {
    let legs = leg_states(rig, segments, t);
    let mut prints = Vec::new();
    if prev.len() == legs.len() {
        for (leg, was) in legs.iter().zip(prev.iter()) {
            if leg.phase >= DUTY && *was < DUTY && !leg.growing {
                prints.push(Print {
                    side: leg.side,
                    segment: leg.segment,
                    placement: leg.placement,
                });
            }
        }
    }
    *prev = legs.iter().map(|leg| leg.phase).collect();
    prints
}

/// Fills `out` with the whole animal at time `t`, back to front: both rows of
/// legs, then the plates from the rear in, then the tail, and the head last — so
/// the two organs end up on top at their own ends of the body, the skull's ruff
/// lapping over the plate it leads and the spike cluster over the plate it
/// trails, while every plate laps the one behind it.
///
/// The legs go under the plates because that is where the art puts them: the
/// hip bar belongs to the animal, and the plate that crosses it belongs on
/// top.
pub fn pose(rig: &Rig, segments: &[Segment], t: f32, out: &mut Vec<Placement>) {
    out.clear();

    // Both rows of legs, and within each row the rear segments first, so that
    // where two segments' legs overlap the one nearer the head wins — the
    // order [`leg_states`] builds, which is where the trail reads its lifts
    // from, so the walk and the prints are one fact.
    out.extend(
        leg_states(rig, segments, t)
            .into_iter()
            .map(|leg| leg.placement),
    );

    // The plates, rear first, each turning into the wave it rides.
    for (i, seg) in segments.iter().enumerate().rev() {
        let x = slot(segments, i);
        let f = x / PITCH;
        out.push(Placement {
            part: seg.body,
            joint: [x, spine_off(f, t)],
            pivot: rig.pivots[seg.body],
            angle: spine_angle(f, t),
            scale: [BODY_SCALE * seg.grow, BODY_SCALE * seg.grow],
        });
    }

    // The tail last of the body, laid over the rear of it: the animal ends the
    // way it begins, with the skull lying over the first plate at one end and
    // the spike cluster over the last at the other. Emitted after the plates is
    // what makes that so — the list is drawn in order, so order is the only
    // thing standing between a tail attached to the body and one floating
    // behind it.
    let last = slot(segments, segments.len().saturating_sub(1));
    let wave = last / PITCH;
    out.push(Placement {
        part: TAIL,
        joint: [
            rear_edge(rig, segments) + TAIL_BACK,
            spine_off(wave + 0.6, t),
        ],
        pivot: rig.pivots[TAIL],
        angle: spine_angle(wave + 0.6, t) + 0.05 * (TAU * t / CYCLE).sin(),
        scale: [BODY_SCALE, BODY_SCALE],
    });

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
    rear_edge(rig, segments) + TAIL_BACK + rig.tail_reach
}

/// How far each body variant's plate reaches above and below the spine line at
/// the one column its hip sits in, in monster units, measured off the cropped
/// textures and used by nothing but the test below. It is the art's geometry,
/// not the rig's: the three chevrons are cut from the sheet with their ridge
/// above the middle of the cut, so hung by the ridge they hang deeper on one
/// side than the other, and hung at the wrong depth they uncover a row of hips.
#[cfg(test)]
const PLATE_EDGES: [[f32; 2]; 3] = [[201.9, 199.1], [173.2, 178.1], [181.6, 186.5]];

#[cfg(test)]
mod tests {
    use super::*;

    /// A leg grows out of a plate, not out of the air next to one, so its hip
    /// has to stay under the plate on the side of the flank the plate is
    /// shallowest on — and both sides have to be shallow by about the same
    /// amount, or one row of legs is tucked under the body while the other
    /// hangs below it, which reads as an animal with a broken hip.
    #[test]
    fn the_hips_are_under_the_plates_they_grow_from() {
        for (variant, [above, below]) in PLATE_EDGES.iter().enumerate() {
            assert!(
                HIP_OUT < above.min(*below) - 60.0,
                "the hip sits {HIP_OUT} out and variant {} of the plate reaches only {} from the spine to its edge at the hip column: the root of the leg would sit in the open",
                variant + 1,
                above.min(*below),
            );
            assert!(
                (above - below).abs() < 10.0,
                "variant {} of the plate reaches {above} above the spine and {below} below it: hung by its middle it should be about as broad one way as the other, and at {HIP_OUT} a hip is buried {} deep on one flank and {} on the other",
                variant + 1,
                above - HIP_OUT,
                below - HIP_OUT,
            );
        }
    }

    /// A rigid leg puts its foot on a circle it cannot stretch, and the stride
    /// is written down in front of that: half a stride is how far a planted
    /// foot travels from under its hip before it is lifted. Half way round the
    /// circle is where a leg of this design stops being a leg and becomes a
    /// line drawn along the ground, so the stride has to stay a fraction of it.
    /// A new segment has to arrive, not appear. It is put on at the slot of the
    /// segment in front of it — where the plate already there hides it, plates
    /// being emitted rear first — and ends on a slot of its own, so the body
    /// gives way by exactly one pitch across the growth instead of snapping out
    /// by it in front of the viewer.
    #[test]
    fn a_new_segment_starts_hidden_and_slides_into_its_slot() {
        let plate = || Segment {
            body: BODIES[0],
            grow: 1.0,
        };
        let settled = 4;
        let mut body = (0..settled).map(|_| plate()).collect::<Vec<_>>();
        let under = slot(&body, settled - 1);
        assert_eq!(under, (settled as f32 - 1.0) * PITCH);

        body.push(Segment {
            body: BODIES[0],
            grow: GROW_START,
        });
        assert!(
            (slot(&body, settled) - under).abs() < 1e-3,
            "a segment born at {GROW_START} sits {} along the body while the plate that is to hide it sits at {under}: they should coincide, or the newcomer shows up in the open behind the last plate",
            slot(&body, settled),
        );

        body[settled].grow = 1.0;
        let gained = slot(&body, settled) - under;
        assert!(
            (gained - PITCH).abs() < 1e-3,
            "a newcomer that has finished growing should have opened exactly one pitch of body behind the last plate, and opened {gained}",
        );
    }

    /// The tail is attached to the back of the animal, not to the last segment,
    /// so putting a segment on must not disturb it: no shrink, no slide, no
    /// twitch. It is allowed to start travelling only once the newcomer's plate
    /// reaches further back than the plate the tail was resting on.
    /// A rig with the real part sizes, for the tests that need a whole one.
    fn rig() -> Rig {
        Rig::new([
            [702.0, 955.0], // the skull
            [495.0, 773.0], // the three chevrons
            [450.0, 695.0],
            [397.0, 717.0],
            [322.0, 268.0], // the tail
            [320.0, 474.0], // a leg
            [195.0, 113.0], // a pressed print
        ])
    }

    /// There is no depth buffer here: what is drawn later is what is on top. The
    /// tail is an organ laid over the rear of the body, not a piece of the rear
    /// edge, so it has to come after every plate — the same relation the skull
    /// has at the front, its ruff lying over the first plate rather than
    /// stopping at its tip.
    #[test]
    fn the_tail_is_laid_over_the_body_and_not_behind_it() {
        let rig = rig();
        let body = (0..4)
            .map(|_| Segment {
                body: BODIES[0],
                grow: 1.0,
            })
            .collect::<Vec<_>>();
        let mut out = Vec::new();
        pose(&rig, &body, 0.37, &mut out);

        let backmost_plate = out
            .iter()
            .rposition(|p| BODIES.contains(&p.part))
            .expect("a pose with four segments should emit plates");
        let tail = out
            .iter()
            .rposition(|p| p.part == TAIL)
            .expect("a pose with segments should emit a tail");
        assert!(
            tail > backmost_plate,
            "the tail is emitted at {tail} and the rearmost plate at {backmost_plate}: drawn in that order the last plate paints over the tail's stem, and the cluster floats off the end of the body",
        );
        let head = out
            .iter()
            .rposition(|p| p.part == HEAD)
            .expect("a pose with segments should emit a head");
        assert!(
            head > backmost_plate,
            "the head is emitted at {head} and the rearmost plate at {backmost_plate}: the skull has to lie over the plate it leads",
        );
    }

    #[test]
    fn adding_a_segment_leaves_the_tail_where_it_was() {
        let rig = rig();
        let plate = |grow| Segment {
            body: BODIES[0],
            grow,
        };
        let body = (0..4).map(|_| plate(1.0)).collect::<Vec<_>>();
        let before = rear_edge(&rig, &body) + TAIL_BACK;

        let longer = body
            .iter()
            .copied()
            .chain([plate(GROW_START)])
            .collect::<Vec<_>>();
        let after = rear_edge(&rig, &longer) + TAIL_BACK;
        assert!(
            (after - before).abs() < 1e-3,
            "adding a segment moved the tail from {before} to {after}: it is one organ on the back of the animal, not a part of whichever segment is last, so it may not twitch when a segment is put on",
        );

        let longer = body.iter().copied().chain([plate(1.0)]).collect::<Vec<_>>();
        assert!(
            rear_edge(&rig, &longer) > before + PITCH - 1e-3,
            "a newcomer that has finished growing should have pushed the rear of the body back by its whole pitch, and pushed it by {}",
            rear_edge(&rig, &longer) - before,
        );
    }

    /// A print is pressed once per cycle per foot — the frame the phase
    /// leaves stance, no other — and the trail knows every foot: ten
    /// cycles of a four-legged frame is ten lifts of each, and a body
    /// that changes between frames is retaken without pressing.
    #[test]
    fn every_foot_presses_one_print_per_cycle() {
        let rig = rig();
        let body = (0..2)
            .map(|_| Segment {
                body: BODIES[0],
                grow: 1.0,
            })
            .collect::<Vec<_>>();
        let mut prev = Vec::new();
        // The first reading only learns the phases: nothing lifts the
        // frame a leg is first seen.
        assert!(
            pressed_prints(&rig, &body, 0.0, &mut prev).is_empty(),
            "a first reading must take the phases, not stamp on them"
        );
        let mut per_foot = [0usize; 4];
        let dt = 1.0 / 60.0;
        for frame in 1..=(CYCLE * 10.0 / dt) as u32 {
            for print in pressed_prints(&rig, &body, frame as f32 * dt, &mut prev) {
                per_foot[(print.side > 0.0) as usize * 2 + print.segment] += 1;
            }
        }
        for (foot, lifts) in per_foot.iter().enumerate() {
            assert!(
                (9..=11).contains(lifts),
                "foot {foot} lifted {lifts} times in ten cycles: each foot lifts exactly once a cycle, on the frame its phase crosses stance"
            );
        }
    }

    /// A leg that is added does not lift — it was never down — and its
    /// neighbours are not disturbed either: the phase memory is retaken
    /// whole, not half-shifted.
    #[test]
    fn a_body_that_changed_its_length_presses_nothing() {
        let rig = rig();
        let plate = || Segment {
            body: BODIES[0],
            grow: 1.0,
        };
        let mut prev = Vec::new();
        let short = [plate()];
        pressed_prints(&rig, &short, 0.41, &mut prev);
        let long = [plate(), plate()];
        assert!(
            pressed_prints(&rig, &long, 0.42, &mut prev).is_empty(),
            "a frame that grew a segment must retake its phases, not stamp with shifted ones"
        );
        // ...and the very next frame walks on normally: the trail is not
        // disabled, just unfooled.
        let mut lifted = false;
        for frame in 1..=60 {
            if !pressed_prints(&rig, &long, 0.42 + frame as f32 / 60.0, &mut prev).is_empty() {
                lifted = true;
            }
        }
        assert!(lifted, "the trail must resume after the body settled");
    }

    /// The print stands where the pad pressed: for every leg of every
    /// frame, the stamp point — the pad's centroid through the node's own
    /// transform — sits within a toe's width of the planted foot the gait
    /// solved for, on both rows. If the stamp ever drifted off the art's
    /// own geometry, this is where it shows.
    #[test]
    fn the_print_stands_where_the_pad_pressed() {
        let rig = rig();
        let body = [Segment {
            body: BODIES[0],
            grow: 1.0,
        }];
        // The pad centroid sits a toe's short step from the contact point
        // the gait plants (they measured three and twenty-one pixels apart
        // in the art): the stamp may be that far off the planted foot and
        // no farther, once the leg's scale is on it.
        let give = (3.0f32.powi(2) + 21.0f32.powi(2)).sqrt() * LEG_SCALE + 1.0;
        for frame in 0..400 {
            let t = frame as f32 * 0.005;
            for leg in leg_states(&rig, &body, t) {
                let pad = rig.pad_of(&leg.placement);
                // The planted foot, by the rig's own definitions: the toe
                // is `leg_len` from the hip along `leg_rest`, taken through
                // the same turn.
                let toe_rel = [
                    rig.leg_len * rig.leg_rest.cos() * leg.placement.scale[0],
                    rig.leg_len * rig.leg_rest.sin() * leg.placement.scale[1],
                ];
                let turned = frost::Transform::rotate(leg.placement.angle).apply(toe_rel);
                let foot = [
                    leg.placement.joint[0] + turned[0],
                    leg.placement.joint[1] + turned[1],
                ];
                let d = ((pad[0] - foot[0]).powi(2) + (pad[1] - foot[1]).powi(2)).sqrt();
                assert!(
                    d <= give,
                    "the print stands {d:.1} from the planted foot at t {t:.3} on side {}: the stamp has left the pad that pressed it",
                    leg.side
                );
            }
        }
    }

    /// The grip, in one line: a planted foot holds the ground. The body's
    /// own crawl carries the hip forward at [`CRAWL_SPEED`] and the stance
    /// drags the foot aft through monster space at the very same rate, so
    /// the foot's world point — hip travel minus its forward reach — never
    /// moves for the whole stance. This is the fact the prints are stamped
    /// on: a foot that lifted pressed the ground where it stands, not a
    /// stride ahead of itself.
    /// A newcomer's feet walk without printing while it swells — one
    /// print short of the row per crossing is the outlier this keeps out
    /// of the soil — and join the rows the frame the segment is full
    /// size.
    #[test]
    fn a_growing_foot_walks_but_does_not_print() {
        let rig = rig();
        let grown = || Segment {
            body: BODIES[0],
            grow: 1.0,
        };
        let mut body = [grown(), grown(), grown()];
        let mut prev = Vec::new();
        pressed_prints(&rig, &body, 0.0, &mut prev);
        // Put on a newcomer and watch the two seconds it swells through:
        // every print belongs to a settled segment.
        let mut body = body.to_vec();
        body.push(Segment {
            body: BODIES[0],
            grow: GROW_START,
        });
        let mut seen_newcomer = false;
        for frame in 1..=150 {
            body[3].grow =
                (((frame as f32 / 60.0) / GROW_SECONDS) * (1.0 - GROW_START) + GROW_START).min(1.0);
            for print in pressed_prints(&rig, &body, frame as f32 / 60.0, &mut prev) {
                if print.segment == 3 {
                    if body[3].grow < 1.0 {
                        seen_newcomer = true; // must never happen
                    }
                }
            }
        }
        assert!(
            !seen_newcomer,
            "a growing segment's foot pressed a print: it stood short of its row, and the soil remembers an outlier the animal never made"
        );
    }

    #[test]
    fn a_planted_foot_holds_the_ground() {
        let rig = rig();
        // Two frames deep inside one stance, sampled by the phase the
        // gait itself would read at those times.
        let (t1, t2) = (0.15 * CYCLE, 0.55 * CYCLE);
        let (p1, p2) = (wrap(t1 / CYCLE), wrap(t2 / CYCLE));
        assert!(p1 < DUTY && p2 < DUTY, "both samples must sit in stance");
        let (reach1, _) = foot_target(&rig, p1, LEG_SCALE);
        let (reach2, _) = foot_target(&rig, p2, LEG_SCALE);
        // World fore-aft = the hip's crawl under the foot, minus the
        // foot's forward reach; constant means the ground feels no slide.
        let a = -CRAWL_SPEED * t1 - reach1;
        let b = -CRAWL_SPEED * t2 - reach2;
        assert!(
            (a - b).abs() < 1.0,
            "a planted foot slid {} units across the ground between stance frames: the grip is the gait",
            a - b
        );
        // And it holds across the whole stance, not just the pair.
        for k in 0..60 {
            let t = CYCLE * (0.02 + 0.55 * k as f32 / 59.0);
            let (r, _) = foot_target(&rig, wrap(t / CYCLE), LEG_SCALE);
            let w = -CRAWL_SPEED * t - r;
            assert!(
                (w - a).abs() < 1.0,
                "the planted foot drifted at t {t:.3}: {w} vs {a}"
            );
        }
    }

    #[test]
    fn the_stride_fits_inside_the_leg() {
        let len = ((TOE[0] - HIP[0]).powi(2) + (TOE[1] - HIP[1]).powi(2)).sqrt() * LEG_SCALE;
        assert!(
            STRIDE / 2.0 < 0.45 * len,
            "half a stride is {} against a leg that reaches {len:.1}: the foot would be asked to stand where the leg cannot",
            STRIDE / 2.0,
        );
    }
}
