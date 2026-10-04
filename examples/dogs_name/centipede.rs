//! A centipede built out of the `dogs_name` monster's cut-out parts and
//! walked **seen from above**, which is the view the parts' own leg sprites
//! imply once the ground plane is the whole picture.
//!
//! [`Monster_head.png`](assets/sprites/Monster_head.png), the three
//! [`Monster_body`](assets/sprites/Monster_body1.png) chevrons,
//! [`Monster_tail.png`](assets/sprites/Monster_tail.png) and
//! [`Monster_leg2.png`](assets/sprites/Monster_leg2.png) are loaded once and
//! assembled by the [`rig`] module: the skull leads, the plates overlap down
//! the back, the spike cluster lies over the last of them the way the skull
//! lies over the first, and one sprite-leg comes out of each segment on **both**
//! sides — a row of legs along the top of the body and a row along the bottom,
//! the far row the same sprite with a negative `y` scale. A leg is one fragment
//! of the sheet's leg, the piece that carries the clay foot, swung from its
//! broken upper end; the art's other fragment, the thigh, is not drawn at all.
//!
//! Those legs are not decoration hung off a moving body: each is a rigid
//! limb on one pivot, so its foot can only ever sit on a circle around its
//! hip, and the body goes only where that allows. A foot is planted for
//! [`rig::DUTY`] of its cycle and dragged backward through the body at
//! exactly the speed the body moves, which is what keeps the feet gripping
//! rather than the whole animal skating along on shuffling feet; the angle a
//! rigid leg has to swing through to do that is not uniform, and the
//! resulting sweep — fast under the hip, crawling at the ends of the stance —
//! is the gait. Nothing is keyframed: the whole pose is a pure function of
//! the clock.
//!
//! Every foot that lifts presses a print: [`Monster_foot.png`](
//! assets/sprites/Monster_foot.png), the square clay footprint, is stamped
//! into the soil at the frame the foot leaves its stance, centred on the
//! pad's own middle and turned with the pressing leg's own angle — so the
//! print of a mirrored far-side foot is mirrored too, exactly as the foot
//! that left it is. Prints are world-fixed: the animal crawls on and the
//! trail stays where it was pressed, drying out and fading over
//! [`PRINT_LIFE`] seconds. The rig's [`rig::pressed_prints`] is the whole
//! contract: it reports the lifts, this file pins them down.
//!
//! The animal starts as three segments drawn at random from the three chevron
//! variants, at a fifth of the old monster's size — a tiny thing in a big
//! window — and it never leaves: [`Vagabond`] is its brain, cruising at a
//! pace around the speed the gait is strided for, stopping dead mid-thought,
//! and dashing off like something frightened it. Every turn it steers is
//! bought by the speed itself — a turn per second is a circle of
//! `speed / turn` pixels, and the brain may never spend more of it than
//! [`MIN_TURN_SPANS`] half-spans buy — so it cannot loop through its own
//! body, and a crawling head simply cannot flick. The leash shrinks the frame by the animal's own span, and a body
//! that follows a path inside a box (see [`TrackPath`]) can never leave
//! that box: a head that stays in bounds cannot drag its body out. The
//! gait clock follows the *ground*, not the wall clock: legs cycle at
//! whatever pace the world goes by, so the feet grip at a crawl, hold one
//! breath at a stop, and whirl through a dash. Every [`GROW_EVERY`]
//! seconds it puts on one more segment, in the open at the rear, until
//! [`MAX_SEGMENTS`] — no crossings anymore, this one just keeps walking.
//!
//! The window opens at a 1920x1080 logical size like `dogs_name`
//! ([`frost::Config::window_size`]), scaled with the display's density and
//! clamped to the monitor if the screen is smaller, and
//! `Config::render_size` pins the frame to those pixels — the animal is
//! scaled to the render size each frame — by its leg span, since from
//! above the distance between its two rows of feet is the dimension it
//! cannot exceed. Run with:
//!
//! A `frost::Diagnostics` overlay reports the window size, frame rate,
//! frame and processing times and draw calls from the window's top-left
//! corner: Alt-0 hides and shows it, Alt-1..4 toggle the charts, Alt-T
//! the readout lines, and Alt+'+' / Alt+'-' size the whole overlay.
//!
//! ```text
//! cargo run --example centipede
//! ```

mod rig;

use rig::{Placement, Segment};

/// The render size the animal is scaled to, as in `dogs_name`, and the
/// window's initial logical size: the frame lands one texel per pixel at
/// that size and any other window size is a stretch of that finished
/// frame, its 16:9 shape preserved.
const WINDOW: [u32; 2] = [1920, 1080];

/// How many segments the animal is born with. Each trip across the window
/// adds one.
const START_SEGMENTS: usize = 3;

/// The longest the animal grows. A centipede is longer than the window long
/// before this, so the cap exists only to stop a very patient viewer from
/// growing the scene forever: at the cap the animal holds its length and
/// keeps crawling.
const MAX_SEGMENTS: usize = 26;

/// The animal's span across its track — the reach of both rows of feet,
/// pads included — as a fraction of the window's height. The fit scale maps
/// [`rig::Rig::half_span`] onto this. Deliberately small: this is a
/// wanderer in a terrarium, not the full-frame monster of the old crossing
/// act, and the whole frame is its ground.
const BODY_SPAN: f32 = 0.15;

/// A dash is this many times the pace the gait is strided for. Legs cycle
/// at the pace the ground goes by, so dashing is legs-as-a-whirlwind —
/// which is exactly how a startled centipede looks.
const DASH_PACE: f32 = 3.4;

/// Extra window pixels kept between the animal's full leg span and the
/// edge: the hard fail-safe the leash steers well clear of.
const WALL_PAD: f32 = 14.0;

/// The smallest turning radius the brain may lay, in half-spans of the
/// animal's own leg span. Curvature is the one thing a long body cannot
/// cheat: a circle whose circumference is shorter than the body is a loop
/// the head crawls through its own back. Three half-spans put a leg
/// span's clearance between the two limbs of any hairpin the animal
/// draws — no crawling on itself, and no in-place pivot either, because
/// the budget that buys a turn is the speed itself (see [`Vagabond`]).
const MIN_TURN_SPANS: f32 = 3.0;

/// Time between growths, in seconds. Crossings used to pace the growth;
/// the animal never leaves now, so the clock takes over: one segment,
/// put on out in the open, every this many seconds.
const GROW_EVERY: f32 = 6.0;

/// How far past an edge a pressing foot must be for its print to be
/// discarded: ground the viewer cannot see is soil not worth pressing.
const MARGIN: f32 = 60.0;

/// The trail's node, the root's first child — under the animal, over the
/// ground: from above there is no horizon, no soil band and nothing to
/// stand on, so the background is the ground it crawls over, the prints
/// press into that ground, and the animal is all else the scene has.
const TRAIL: usize = 0;

/// The animal's node.
const ANIMAL: usize = 1;

/// How long a print stays in the soil, in seconds, and the last stretch
/// of that over which it fades: the clay gives a while, then the dust
/// settles it away. A trail that only ever grew would bury the ground in
/// clay by the third crossing.
const PRINT_LIFE: f32 = 7.0;
const PRINT_FADE: f32 = 2.5;

/// A print's life is drawn between this fraction of [`PRINT_LIFE`] and
/// the whole of it: old soil does not age in lockstep, and a print that
/// died young reads as ground the pad only grazed.
const PRINT_MIN_LIFE: f32 = 0.1;

/// How far either way of the lift the shutter may fall, as a fraction of
/// [`rig::CYCLE`]: a photograph of the step taken a heartbeat early (the
/// pad still dragging aft) or late (the foot already swinging forward)
/// stands where the foot stood then and wears the turn it had then —
/// never the frozen instant of lift.
const PRINT_JITTER: f32 = 0.06;

/// The most prints the soil holds. A full-grown animal presses some
/// forty feet a second, and this is about seven crossings' worth — the
/// cap only bites so that a very long crawl cannot grow the scene
/// forever between cleanups; the oldest print goes first.
const MAX_PRINTS: usize = 500;

#[cfg(test)]
mod tests {
    use super::{DASH_PACE, Mood, TrackPath, Vagabond, print_transform};

    /// The phantom X, pinned out of existence: a print's node maps its
    /// own centre — the sprite's middle, the node's origin — exactly
    /// onto the spot that was pressed, whatever the foot's turn, and
    /// its art turns about that spot at a fixed distance. Compose the
    /// pin before the turn and both halves of this break together.
    #[test]
    fn a_print_lands_where_it_was_pressed_and_turns_in_place() {
        for (world, angle) in [
            ([-640.0, 310.0], 2.35),
            ([912.0, -295.0], -1.94),
            ([0.0, 0.0], 0.79),
        ] {
            let tf = print_transform(world, angle);
            let at = tf.apply([0.0, 0.0]);
            assert!(
                (at[0] - world[0]).abs() < 1e-3 && (at[1] - world[1]).abs() < 1e-3,
                "a print pressed at {world:?} rendered at {at:?}: the pin must survive the turn"
            );
            let off = [50.0, -20.0];
            let p = tf.apply(off);
            let spun = [p[0] - world[0], p[1] - world[1]];
            let r0 = (off[0] * off[0] + off[1] * off[1]).sqrt();
            let r1 = (spun[0] * spun[0] + spun[1] * spun[1]).sqrt();
            assert!(
                (r0 - r1).abs() < 1e-2,
                "the sprite turned about the screen's origin, not its own print: {r0} vs {r1}"
            );
        }
    }

    /// A straight track reads back exactly as it was laid: distances
    /// measured behind the head land where they should, ahead of it the
    /// nose's extrapolation runs the line on, and nothing turns.
    #[test]
    fn a_straight_track_reads_back_as_it_was_laid() {
        let path = TrackPath::straight([120.0, -37.0], 3000.0);
        let (p, a) = path.at(500.0);
        assert!(
            (p[0] - 620.0).abs() < 1.0 && (p[1] + 37.0).abs() < 1.0,
            "500 behind the head landed at {p:?}"
        );
        assert!(a.abs() < 1e-3, "a straight track turned by {a}");
        let (q, aq) = path.at(-80.0);
        assert!(
            (q[0] - 40.0).abs() < 1.0 && aq.abs() < 1e-3,
            "the nose sits at {q:?} turned {aq}"
        );
    }

    /// The whole point of the trail: laid around a bend, a follower at
    /// any distance behind the head stays on the bend, facing along it —
    /// the body follows where the head has been, cutting neither across
    /// the curve nor off the end of the memory.
    #[test]
    fn the_body_follows_the_head_around_a_bend() {
        let (r, c) = (600.0f32, [0.0f32, 0.0]);
        let mut path = TrackPath::default();
        let n = 400i32;
        for k in 0..=n {
            let th = std::f32::consts::TAU * k as f32 / n as f32;
            let heading = th + std::f32::consts::FRAC_PI_2;
            path.advance([c[0] + r * th.cos(), c[1] + r * th.sin()], heading, 0.01);
        }
        for s in [100.0, 700.0, 2400.0] {
            let (p, a) = path.at(s);
            let rad = ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt();
            assert!(
                (rad - r).abs() < 6.0,
                "a follower left the circle at s {s}: radius {rad}"
            );
            let dot = ((p[0] - c[0]) * a.cos() + (p[1] - c[1]) * a.sin()) / rad;
            assert!(
                dot.abs() < 0.02,
                "the follower at s {s} cut across the bend: {dot}"
            );
        }
        // And past the remembered ground it runs straight on, not wild.
        let (p, _) = path.at(std::f32::consts::TAU * 600.0 + 900.0);
        assert!(p[0].is_finite() && p[1].is_finite(), "past memory: {p:?}");
    }

    /// The leash holds, and the temperament shows. Five simulated minutes
    /// of a seeded brain must visit every mood, really stop, really dash,
    /// cover real ground — never put the head outside its box, which is
    /// the containment promise — and never spend a turn it did not pay
    /// for: every frame's heading change must fit the circle its speed
    /// bought, which is what there is no crawling over itself.
    #[test]
    fn the_vagabond_keeps_inside_its_leash() {
        let mut rng = frost::Rng::with_seed(11);
        let lim = [860.0, 440.0];
        let pace = 50.0;
        let r_min = 240.0;
        let dt = 1.0 / 60.0;
        let mut v = Vagabond::new([-300.0, 0.0], std::f32::consts::PI);
        let mut prev = v.pos;
        let mut prev_heading = v.heading;
        let mut travelled = 0.0;
        let mut seen = [false; 3];
        let mut fastest = 0.0f32;
        let mut slowest = f32::MAX;
        for _ in 0..30000 {
            v.step(dt, &mut rng, lim, pace, r_min);
            let dh = (v.heading - prev_heading + std::f32::consts::PI)
                .rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            assert!(
                dh.abs() <= 2.0 * v.speed * dt / r_min + 1e-4,
                "turn bought beyond the radius at clock {:.1}: {:.4} rad in a frame",
                v.clock,
                dh.abs()
            );
            prev_heading = v.heading;
            assert!(
                v.pos[0].abs() <= lim[0] + 0.5 && v.pos[1].abs() <= lim[1] + 0.5,
                "the head left its box at clock {:.1}: {:?}",
                v.clock,
                v.pos
            );
            assert!(
                v.speed >= 0.0 && v.speed <= pace * (DASH_PACE + 0.5),
                "speed {} outside stop..dash at clock {:.1}",
                v.speed,
                v.clock
            );
            travelled += ((v.pos[0] - prev[0]).powi(2) + (v.pos[1] - prev[1]).powi(2)).sqrt();
            prev = v.pos;
            seen[match v.mood {
                Mood::Cruising => 0,
                Mood::Pausing => 1,
                Mood::Dashing => 2,
            }] = true;
            fastest = fastest.max(v.speed);
            slowest = slowest.min(v.speed);
        }
        assert!(
            seen.iter().all(|s| *s),
            "a whole career with no cruise, pause or dash: {seen:?}"
        );
        assert!(
            fastest > pace * 2.0,
            "never really dashed: fastest {fastest}"
        );
        assert!(
            slowest < pace * 0.05,
            "never really stopped: slowest {slowest}"
        );
        assert!(
            travelled > 5000.0,
            "that is no way to spend five minutes: {travelled} px"
        );
    }

    /// The smoothness contract, stated where it belongs: a follower
    /// sliding along the remembered ground must never *snap* at a corner
    /// of the polyline. A couple of brain-minutes of wandering lay real
    /// track; then the follower's turn is sampled either side of every
    /// vertex, and the jump across a vertex has to be nothing — the jump
    /// a raw neighbour-chord would take is degrees, tens of times a
    /// second, which is precisely the shiver this replaced.
    #[test]
    fn a_follower_turns_continuously_along_the_track() {
        let mut rng = frost::Rng::with_seed(3);
        let mut v = Vagabond::new([0.0, 0.0], 0.0);
        let mut path = TrackPath::default();
        let lim = [600.0, 300.0];
        for _ in 0..1500 {
            v.step(1.0 / 60.0, &mut rng, lim, 60.0, 240.0);
            path.advance(v.pos, v.heading, 0.05);
        }
        let head_d = path.pts.last().map_or(0.0, |last| last.d);
        // Well above the f32 floor at these remembered distances, and far
        // under a step: what survives the crossing is a snap, not the
        // turning of a genuinely curving path, which shrinks with `eps`.
        let eps = 0.005;
        let mut worst = 0.0f32;
        for q in &path.pts[1..path.pts.len() - 1] {
            let s = head_d - q.d;
            let (_, before) = path.at(s - eps);
            let (_, after) = path.at(s + eps);
            let jump = (after - before + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            worst = worst.max(jump.abs());
        }
        assert!(worst < 0.01, "the track snaps at its corners: {worst} rad");
    }
}

/// What the head is feeling right now.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mood {
    /// Putting along, changing its mind about direction from time to time.
    Cruising,
    /// Stopped dead mid-thought, holding one stride. No glancing around:
    /// with every foot planted, a head that turns in place is a body
    /// folded at the fold's first step — turning is paid for in ground
    /// crossed, like everything else.
    Pausing,
    /// Off like a shot, for reasons known only to it.
    Dashing,
}

/// The little brain driving the head: a heading, a speed, and a temperament
/// drifting between cruising, stopping dead to nobody knows why, and dashing
/// off — on a wall leash. The leash keeps the head inside a box, and the
/// body needs no leash of its own: it only ever occupies ground the head
/// has already crossed ([`TrackPath`]), and legs reach no farther off that
/// path than the span the box was shrunk by. A head that never leaves a box
/// cannot drag its body out of it.
struct Vagabond {
    pos: [f32; 2],
    heading: f32,
    speed: f32,
    mood: Mood,
    /// Seconds of this mood left. Moods live on the brain's own clock, not
    /// the gait's or the demo's.
    mood_left: f32,
    clock: f32,
    /// The cruise pace this spell settled on, as a fraction of the pace.
    base: f32,
    /// The turn per second the current whim is steering with — glided
    /// toward `turn_target`, so a change of mind bends the path over a
    /// moment instead of kinking it.
    turn: f32,
    turn_target: f32,
}

impl Vagabond {
    fn new(pos: [f32; 2], heading: f32) -> Self {
        Vagabond {
            pos,
            heading,
            speed: 0.0,
            mood: Mood::Cruising,
            mood_left: 0.0,
            clock: 0.0,
            base: 1.0,
            turn: 0.2,
            turn_target: 0.2,
        }
    }

    /// One frame of wanting: roll the mood when its time is up, glide the
    /// speed toward whatever the mood wants, and spend the frame's
    /// steering budget — on the current whim first, then on whatever the
    /// walls demand. `pace` is the speed the gait is strided for in window
    /// pixels; `lim` is the half-box the head may visit; `r_min` is the
    /// smallest turning radius the body can follow without crawling over
    /// itself.
    fn step(&mut self, dt: f32, rng: &mut frost::Rng, lim: [f32; 2], pace: f32, r_min: f32) {
        self.clock += dt;
        self.mood_left -= dt;
        if self.mood_left <= 0.0 {
            self.mood = match (self.mood, rng.next_f32()) {
                // A dash spends everything: most dash-ends are a collapse.
                (Mood::Dashing, r) if r < 0.6 => Mood::Pausing,
                (_, r) if r < 0.17 => Mood::Dashing,
                (_, r) if r < 0.4 => Mood::Pausing,
                _ => Mood::Cruising,
            };
            let (left, turn) = match self.mood {
                Mood::Cruising => (rng.in_range(2.5, 6.5), rng.in_range(-1.0, 1.0)),
                Mood::Pausing => (rng.in_range(0.6, 2.0), 0.0),
                Mood::Dashing => (rng.in_range(0.4, 1.0), rng.in_range(-0.3, 0.3)),
            };
            self.mood_left = left;
            self.turn_target = turn;
            self.base = rng.in_range(0.45, 1.35);
        }
        let target = match self.mood {
            // Cruising breathes — the pace swells and settles, so even the
            // plain stretches of the walk never tick along metronomic.
            Mood::Cruising => pace * self.base * (1.0 + 0.22 * (self.clock * 2.6).sin()),
            Mood::Pausing => 0.0,
            Mood::Dashing => pace * DASH_PACE,
        };
        let glide = match self.mood {
            Mood::Cruising => 1.8,
            Mood::Pausing => 4.5,
            Mood::Dashing => 9.0,
        };
        self.speed += (target - self.speed) * (glide * dt).min(1.0);

        // The whole steering budget of a frame, whim and wall alike:
        // a turn of `w` radians a second at `speed` pixels a second lays
        // a circle of `speed / w` pixels, and a circle tighter than
        // `r_min` is a loop the animal crawls through its own back. So
        // the budget is `speed / r_min` — turning, like walking, is
        // something the feet pay for, and a crawling head physically
        // cannot flick. This is also what makes the moods read right:
        // the dash carves wide, the crawl wanders in slow arcs, and the
        // stop turns into nothing at all.
        let cap = self.speed / r_min;
        self.turn += (self.turn_target - self.turn) * (2.5 * dt).min(1.0);
        self.heading += self.turn.clamp(-cap, cap) * dt;
        if self.mood == Mood::Cruising && rng.next_f32() < dt * 0.6 {
            self.turn_target = rng.in_range(-1.0, 1.0);
        }

        // The wall leash, felt early: the pull fades in over two `r_min`
        // before the hard edge — the room the widest legal turn needs to
        // come back — and spends the same capped budget on it. A force
        // that snaps on at the edge is a jolt the head oscillates against
        // frame by frame; a force that fades in is a curve.
        let band = |pos: f32, lim: f32| {
            let band = 2.0 * r_min;
            ((pos.abs() - (lim - band)) / band).clamp(0.0, 1.0)
        };
        let pull = band(self.pos[0], lim[0]).max(band(self.pos[1], lim[1]));
        if pull > 0.0 {
            let want = (-self.pos[1]).atan2(-self.pos[0]);
            let diff = (want - self.heading + std::f32::consts::PI)
                .rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            let max = cap * pull * dt;
            self.heading += diff.clamp(-max, max);
        }
        self.pos[0] = (self.pos[0] + self.heading.cos() * self.speed * dt).clamp(-lim[0], lim[0]);
        self.pos[1] = (self.pos[1] + self.heading.sin() * self.speed * dt).clamp(-lim[1], lim[1]);
    }
}

/// Pin a print to the soil, turned about its own middle. The order is
/// the legs' own idiom — turn first, pin last, because `compose` runs
/// left to right and the last translate lands the node. The one time
/// these two were swapped (`translate` first, `rotate` after), the
/// rotation wrapped the pin instead of the sprite: a print at `world`
/// rendered at `R·world`, every print of a row swung onto the ray its
/// row's lift angle points along, both rows mirrored about the body —
/// and the window filled with two perfect diagonals through its centre:
/// an X, unrelated to where any foot had ever stood. The print's centre
/// must land on the pressed spot, exactly, always.
fn print_transform(world: [f32; 2], angle: f32) -> frost::Transform {
    frost::Transform::rotate(angle).compose(&frost::Transform::translate(world))
}

/// One print in the trail, pinned to the soil in window pixels at the
/// frame it was pressed — the ground does not move; the animal does.
struct Track {
    /// Where the pressing pad's middle stood, in window pixels.
    world: [f32; 2],
    /// The pressing leg's rotation, so the print wears the foot's angle —
    /// the mirrored row's prints mirrored with it.
    angle: f32,
    /// The pressing leg's scale, mirroring and all, with the fit baked in:
    /// the print is the pad's size in window pixels, not a second,
    /// unrelated number — and not art units either, since the trail is no
    /// child of the animal's scaled node.
    scale: [f32; 2],
    /// Seconds since it was pressed.
    age: f32,
    /// How long this print will last: drawn at pressing between
    /// [`PRINT_MIN_LIFE`] and all of [`PRINT_LIFE`].
    life: f32,
}

/// A lifted foot whose photo has not been taken yet: its print will be
/// pressed at `due`, from whatever pose the leg wears by then.
#[derive(Clone, Copy)]
struct Pending {
    side: f32,
    segment: usize,
    due: f32,
}

/// One remembered spot of the ground the head has crossed, with the
/// distance along the track from its oldest remembered point.
#[derive(Clone, Copy)]
struct PathPoint {
    p: [f32; 2],
    d: f32,
    /// The unit vector *aft* along the head's own heading when it stood
    /// here — remembered, not later re-measured off the chord between
    /// neighbours, which a stuttering clock makes a noisy bearing.
    dir: [f32; 2],
}

/// The ground the head has crossed, remembered as a polyline. The head
/// leads; everything else — plates, tail, legs, and every print the legs
/// press — is placed at its own distance *behind* the head along this
/// line, turned to the line's direction there. That is how an animal
/// follows its own trail: not by aiming its body at where it is going,
/// but by having its body remember where its head has been.
/// The turn followers read is interpolated between these remembered
/// headings, so it glides along the track instead of snapping at every
/// corner of the polyline — a head whose heading is continuous cannot
/// hand its body a shiver.
#[derive(Default)]
struct TrackPath {
    pts: Vec<PathPoint>,
}

impl TrackPath {
    /// A brand-new crossing starts on ground straight back from the
    /// entry point — the animal has not wandered any of it yet, but its
    /// tail still has to stand somewhere.
    fn straight(head: [f32; 2], len: f32) -> Self {
        let step = 24.0f32;
        let count = (len / step).ceil() as usize + 1;
        let mut pts = Vec::with_capacity(count + 1);
        for k in (0..=count).rev() {
            pts.push(PathPoint {
                p: [head[0] + k as f32 * step, head[1]],
                d: (count - k) as f32 * step,
                dir: [1.0, 0.0],
            });
        }
        Self { pts }
    }

    /// The head moved, wearing `heading`: extend the remembered ground,
    /// but only by steps worth remembering, each keeping the heading the
    /// head had when it crossed.
    fn advance(&mut self, head: [f32; 2], heading: f32, min_step: f32) {
        let aft = [-heading.cos(), -heading.sin()];
        let last = match self.pts.last() {
            Some(last) => *last,
            None => {
                self.pts.push(PathPoint {
                    p: head,
                    d: 0.0,
                    dir: aft,
                });
                return;
            }
        };
        let dx = head[0] - last.p[0];
        let dy = head[1] - last.p[1];
        let step = (dx * dx + dy * dy).sqrt();
        if step >= min_step {
            self.pts.push(PathPoint {
                p: head,
                d: last.d + step,
                dir: aft,
            });
        }
    }

    /// Ground farther behind the head than the animal can ever reach is
    /// ground nothing stands on any more: forget it.
    fn trim(&mut self, keep: f32) {
        let head_d = self.pts.last().map_or(0.0, |last| last.d);
        let floor = head_d - keep;
        let mut cut = self.pts.partition_point(|q| q.d < floor);
        cut = cut.max(self.pts.len().saturating_sub(MAX_PATH_PTS));
        self.pts.drain(..cut.min(self.pts.len().saturating_sub(2)));
    }

    /// The ground pose `s` window-pixels behind the head: the spot, and
    /// the turn the track has there — the head's own remembered heading
    /// at that ground, in window radians (which is monster space's `+x`,
    /// aft along the body), blended between the two steps it falls
    /// between and so continuous along the whole track. Ahead of the
    /// head (the nose lives there) and beyond the memory, the nearest
    /// remembered step simply runs straight.
    fn at(&self, s: f32) -> ([f32; 2], f32) {
        let n = self.pts.len();
        let head = match self.pts.last() {
            Some(head) => *head,
            None => return ([s, 0.0], 0.0),
        };
        if n == 1 {
            return ([head.p[0] + s, head.p[1]], head.dir[1].atan2(head.dir[0]));
        }
        let target = head.d - s;
        if target <= self.pts[0].d {
            let tail = self.pts[0];
            let back = tail.d - target;
            return (
                [
                    tail.p[0] + tail.dir[0] * back,
                    tail.p[1] + tail.dir[1] * back,
                ],
                tail.dir[1].atan2(tail.dir[0]),
            );
        }
        if target >= head.d {
            let ahead = target - head.d;
            return (
                [
                    head.p[0] - head.dir[0] * ahead,
                    head.p[1] - head.dir[1] * ahead,
                ],
                head.dir[1].atan2(head.dir[0]),
            );
        }
        let i = self.pts.partition_point(|q| q.d <= target) - 1;
        let (a, b) = (self.pts[i], self.pts[i + 1]);
        let w = (target - a.d) / (b.d - a.d);
        (
            [
                a.p[0] + (b.p[0] - a.p[0]) * w,
                a.p[1] + (b.p[1] - a.p[1]) * w,
            ],
            (a.dir[1] + (b.dir[1] - a.dir[1]) * w).atan2(a.dir[0] + (b.dir[0] - a.dir[0]) * w),
        )
    }
}

/// The remembered steps' point budget on top of the distance budget:
/// the crawl lays many short steps, and the memory is for following,
/// not for hoarding. At the slowest real crawl this still remembers
/// several body lengths of ground.
const MAX_PATH_PTS: usize = 8000;

/// How much ground the animal's own length can still reach behind the
/// head — everything the trail must remember, and nothing more.
fn path_keep(fit: f32) -> f32 {
    MAX_SEGMENTS as f32 * rig::PITCH * fit + 400.0
}

/// The centipede: the segments it is built from, the clock its gait reads,
/// and where it stands.
struct Centipede {
    /// The wall clock, in seconds: moods, growth and the prints' aging
    /// read it. The pose is a pure function of the *gait* clock instead —
    /// see [`Centipede::gait`].
    t: f32,
    /// The body, head first: a chevron variant and how far that segment has
    /// grown, `1.0` being full size.
    segments: Vec<Segment>,
    /// The head's brain: where it has put the head, at what heading and
    /// speed, and in what mood.
    head: Vagabond,
    /// The gait clock: `rig::pose` and the prints read this, not the wall
    /// clock. It advances with the ground going by — speed divided by the
    /// pace the stride was measured for — which is what keeps the feet
    /// gripping at any pace, holds one mid-stride breath at a stop, and
    /// whirls the legs through a dash.
    gait: f32,
    /// The ground the head has crossed, and the lifted feet whose prints
    /// await their shutters.
    path: TrackPath,
    pending: Vec<Pending>,
    /// The rig's numbers, derived once from the parts' texture sizes.
    rig: rig::Rig,
    /// The parts, indexed by [`rig`] part index.
    shapes: Vec<frost::Shape>,
    /// The scratch buffer [`rig::pose`] writes into, kept out of the scene.
    placements: Vec<Placement>,
    /// The source of the segments it picks, and of the order it picks them.
    rng: frost::Rng,
    /// Whether the animal's nodes have to be rebuilt: set whenever its body
    /// changes, which is not only when it changes length.
    rebuild: bool,
    /// The next moment the animal puts on a segment: crossings paced the
    /// growth once; now it is simply every [`GROW_EVERY`] seconds.
    next_growth: f32,
    /// The frame of [`rig::pressed_prints`]'s memory: each foot's phase as of
    /// the last frame, which is what turns the gait's pure clock into the
    /// single frame a foot lifts.
    lift_phase: Vec<f32>,
    /// The trail: the prints still in the soil, oldest first.
    prints: Vec<Track>,
    /// The diagnostics overlay: frame rate, frame and processing times,
    /// and draw calls, top-left, drawn on its own topmost layer.
    diag: frost::Diagnostics,
}

impl Centipede {
    /// One of the three chevron variants, at random.
    fn random_body(&mut self) -> usize {
        let n = rig::BODIES.len() as u64;
        rig::BODIES[(self.rng.next_u64() % n) as usize]
    }

    /// The fit scale: half the animal's lateral span onto half of
    /// [`BODY_SPAN`] of the window. Uniform — a centipede squashed along its
    /// own length is just a wrong centipede — and independent of how many
    /// segments it has, so the animal never changes size as it grows.
    fn fit(&self, height: f32) -> f32 {
        BODY_SPAN * height / (2.0 * self.rig.half_span())
    }

    /// Adds one segment at the rear, at the size and the slot a segment is
    /// born with: [`rig::GROW_START`] of its final plate, and no slot at all —
    /// on top of the segment in front of it, which hides it, since the plates
    /// are drawn rear first. Everything after this is the growth ramp.
    fn grow_a_segment(&mut self) {
        let body = self.random_body();
        self.segments.push(Segment {
            body,
            grow: rig::GROW_START,
        });
        self.rebuild = true;
        log::info!("the centipede grew: {} segments", self.segments.len());
    }

    /// Map a point of monster space — `u` aft along the body, `v`
    /// across it, both in art units — onto the window: the spot on the
    /// remembered ground, and the track's own turn there. Every part,
    /// and every print, adds that turn to its own angle; nothing gets a
    /// direction of its own.
    fn spine(&self, u: f32, v: f32, fit: f32) -> ([f32; 2], f32) {
        let (p, turn) = self.path.at(u * fit);
        (
            [p[0] - turn.sin() * v * fit, p[1] + turn.cos() * v * fit],
            turn,
        )
    }

    /// The animal, fresh: `count` full-size segments — a new animal picks a
    /// fresh set of chevron variants even at the cap — with the head placed
    /// inside the frame facing left, and the remembered ground laid straight
    /// out behind it. There are no crossings any more: one animal, one long
    /// walkabout.
    fn enter(&mut self, count: usize, width: f32, fit: f32) {
        self.pending.clear();
        self.lift_phase.clear();
        self.prints.clear();
        self.segments.clear();
        for _ in 0..count {
            let body = self.random_body();
            self.segments.push(Segment { body, grow: 1.0 });
        }
        let head = [-width * 0.15, 0.0];
        self.head = Vagabond::new(head, std::f32::consts::PI);
        self.gait = 0.0;
        self.next_growth = GROW_EVERY;
        self.rebuild = true;
        self.path = TrackPath::straight(head, path_keep(fit));
        self.lift_phase.clear();
    }
}

impl frost::Process for Centipede {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // The overlay reads the completed frame through the engine's own
        // probes, so it goes first and sees every node the demo is about
        // to change as the state it is reporting on.
        self.diag.process(ctx, dt);
        self.t += dt;
        // A frame that took much longer than a fraction of a gait cycle —
        // a shader-compile hitch on the first frames, a window drag, a
        // stop elsewhere in the process — proves nothing about which feet
        // lifted. Whole cycles came and went unseen; every phase looks
        // like it crossed stance at once, and stamping that presses a
        // frozen full-length cluster at whatever spot the animal happens
        // to occupy — which is the phantom X at the window's edge. The
        // honest answer is to forget the gap: clearing the phase memory
        // makes the next reading retake the phases, and the trail picks
        // up on the following lift, as if the stall had been a blink.
        if dt > 0.1 {
            self.lift_phase.clear();
        }
        let (width, height) = ctx.size();
        let fit = self.fit(height);
        // The leash: the window shrunk by the animal's own lateral span —
        // legs cannot reach further off the head's track than half of
        // [`BODY_SPAN`], by the way [`Centipede::fit`] is defined — and a
        // pad. Everything follows from the head, so confining the head
        // confines the animal.
        let span = 0.5 * BODY_SPAN * height;
        let leash = [
            (width / 2.0 - (span + WALL_PAD)).max(24.0),
            (height / 2.0 - (span + WALL_PAD)).max(24.0),
        ];
        let r_min = MIN_TURN_SPANS * span;

        // The first frame knows the window size, which is when the animal
        // can be placed at all: build it inside the frame, head first.
        if self.segments.is_empty() {
            self.enter(START_SEGMENTS, width, fit);
        }

        // The growth. Crossings used to pace it; the clock does now: one
        // fresh segment every [`GROW_EVERY`] seconds, put on out in the
        // open at the rear, until [`MAX_SEGMENTS`] and walking is all
        // that is left.
        if self.t >= self.next_growth && self.segments.len() < MAX_SEGMENTS {
            self.grow_a_segment();
            self.next_growth += GROW_EVERY;
        }

        // Every segment stretches toward full size; only a newcomer is ever
        // short of it, and it gets there in `GROW_SECONDS`. Its legs go with
        // it, since a leg's whole stance — hip height included — is scaled
        // by the same number, and so does its slot: [`rig`] pulls a growing
        // segment forward toward the one in front by how much it has left to
        // grow, which is what turns the addition into one long stretch.
        for segment in &mut self.segments {
            segment.grow = (segment.grow + dt / rig::GROW_SECONDS).min(1.0);
        }

        // The brain, then the ground it has crossed, then the gait that
        // ground implies: the head wanders under its own whims — cruise,
        // dead stop, dash — the remembered path grows behind it, and the
        // legs cycle at whatever pace the world actually goes by. Feet
        // grip at a crawl, hold one breath at a stop, whirl through a
        // dash; the stride never skates, at any of them.
        self.head
            .step(dt, &mut self.rng, leash, rig::CRAWL_SPEED * fit, r_min);
        // Every frame is remembered, however short the step: a crawling
        // head whose turns are only committed every few pixels gets its
        // smooth heading back as a staircase.
        self.path.advance(self.head.pos, self.head.heading, 0.05);
        self.path.trim(path_keep(fit));
        self.gait += dt * self.head.speed / (rig::CRAWL_SPEED * fit);

        // The pose, in art units, back to front: both rows of legs, the
        // tail, the plates from the rear in, and the head.
        rig::pose(&self.rig, &self.segments, self.gait, &mut self.placements);

        // The trail, in two beats. First the presses: every foot whose
        // phase left stance this frame queues its photo, the shutter to
        // fall a heartbeat early or late — never exactly at lift. Then
        // the shutter: every queued print whose moment has come is taken
        // from the leg's pose *now*, somewhere inside the foot's stride,
        // so the print stands where the foot stood then and wears the
        // turn it had then — mapped along the [`TrackPath`] like every
        // other part of the animal, its life drawn afresh with it.
        let pressed =
            rig::pressed_prints(&self.rig, &self.segments, self.gait, &mut self.lift_phase);
        for print in pressed {
            self.pending.push(Pending {
                side: print.side,
                segment: print.segment,
                due: self.t + PRINT_JITTER * rig::CYCLE * self.rng.in_range(-1.0, 1.0),
            });
        }
        let legs = self.segments.len();
        let mut shutter = Vec::new();
        self.pending.retain(|shot| {
            if self.t >= shot.due {
                shutter.push(*shot);
                false
            } else {
                true
            }
        });
        for shot in shutter {
            // The same leg, wherever it stands in the row now: a row is
            // laid rear to front, so a segment keeps its seat by rank.
            let row = if shot.side < 0.0 { 0 } else { legs };
            let leg = &self.placements[row + (legs - 1 - shot.segment)];
            let pad = self.rig.pad_of(leg);
            let (world, ground) = self.spine(pad[0], pad[1], fit);
            // A foot lifted off the edge of the window presses ground the
            // viewer cannot see; the wander will press better soil soon.
            if world[0].abs() > width / 2.0 + MARGIN || world[1].abs() > height / 2.0 + MARGIN {
                continue;
            }
            log::trace!(
                "print taken: side {:>1}, segment {}, at ({world:?})",
                shot.side,
                shot.segment
            );
            self.prints.push(Track {
                world,
                angle: ground + leg.angle,
                // The trail hangs off the root, not off the animal, so
                // this is the one place the print's size meets the fit:
                // the foot's art-unit scale, converted to window pixels
                // at the moment of pressing.
                scale: [leg.scale[0] * fit, leg.scale[1] * fit],
                age: 0.0,
                life: PRINT_LIFE * self.rng.in_range(PRINT_MIN_LIFE, 1.0),
            });
        }

        if self.prints.len() > MAX_PRINTS {
            let overflow = self.prints.len() - MAX_PRINTS;
            self.prints.drain(..overflow);
        }

        // The soil itself: one node per print, oldest first, each drying
        // out over its own drawn life and fading over the last slice of
        // it — the same fraction of a short print's as of a long one's.
        let trail = &mut ctx.scene().root.children[TRAIL];
        for track in &mut self.prints {
            track.age += dt;
        }
        self.prints.retain(|track| track.age < track.life);
        while trail.children.len() < self.prints.len() {
            trail.children.push(Box::new(frost::SceneNode::default()));
        }
        trail.children.truncate(self.prints.len());
        for (node, track) in trail.children.iter_mut().zip(&self.prints) {
            node.shape = Some(self.shapes[rig::FOOT].clone());
            // Turn about the print's own middle, then pin it to the soil:
            // the same idiom the legs hang from their joints with, read the
            // other way — innermost transform first.
            node.transform = print_transform(track.world, track.angle);
            node.scale = track.scale;
            let dry = ((track.life - track.age) / (track.life * (PRINT_FADE / PRINT_LIFE)))
                .clamp(0.0, 1.0);
            node.modulate = frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: dry,
            };
        }

        // The animal: one node per placement. The node carries only the
        // fit now — the parts hang at their own spots on the remembered
        // ground, converted back to art units — because a body strung
        // along a winding trail has no single place and turn left to
        // carry.
        let animal = &mut ctx.scene().root.children[ANIMAL];
        animal.transform = frost::Transform::translate([0.0, 0.0]);
        animal.scale = [fit, fit];

        // The tree is rebuilt only when the body changes — which the rig
        // reports as a placement count, plus the one case the count cannot
        // see: an animal at the length cap comes back just as long, but out
        // of a different set of chevron variants.
        if self.rebuild || animal.children.len() != self.placements.len() {
            self.rebuild = false;
            animal.children = self
                .placements
                .iter()
                .map(|p| {
                    Box::new(frost::SceneNode {
                        shape: Some(self.shapes[p.part].clone()),
                        ..Default::default()
                    })
                })
                .collect();
        }

        for (node, p) in animal.children.iter_mut().zip(&self.placements) {
            // A part is centred on its node, so its joint has to be moved
            // back onto the node's origin — and since frost applies a node's
            // scale *before* its transform, that offset has to be scaled by
            // the same factor to keep the joint where the rig put it. The
            // scale's `y` is negative for the top row of legs, which is the
            // mirror: it folds the reflection into the same transform the
            // bottom row uses.
            let (spot, ground) = self.spine(p.joint[0], p.joint[1], fit);
            let joint = [spot[0] / fit, spot[1] / fit];
            node.transform =
                frost::Transform::translate([-p.pivot[0] * p.scale[0], -p.pivot[1] * p.scale[1]])
                    .compose(&frost::Transform::rotate(ground + p.angle))
                    .compose(&frost::Transform::translate(joint));
            node.scale = [p.scale[0], p.scale[1]];
        }

        log::trace!(
            "process: dt {dt:?}, {} segments at ({:.0}, {:.0}), {} draws",
            self.segments.len(),
            self.head.pos[0],
            self.head.pos[1],
            self.placements.len()
        );
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset paths to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let diag = match frost::Diagnostics::new(
        format!("{root}/assets/fonts/FiraCode-VariableFont_wght.ttf"),
        frost::DiagnosticsFlags::all(),
    ) {
        Ok(diagnostics) => diagnostics,
        Err(err) => {
            log::error!("failed to load the font: {err}");
            std::process::exit(1);
        }
    };
    let shapes: Vec<frost::Shape> = rig::PART_FILES
        .iter()
        .map(|file| {
            frost::Shape::sprite(format!("{root}/examples/dogs_name/assets/sprites/{file}"))
                .unwrap_or_else(|err| panic!("failed to load {file}: {err}"))
        })
        .collect();

    // The rig reads the parts' real texture sizes, so the joints in `rig`
    // can stay in image pixels.
    let mut sizes = [[0.0f32; 2]; rig::PART_COUNT];
    for (slot, shape) in sizes.iter_mut().zip(&shapes) {
        *slot = shape
            .sprite_size()
            .expect("every part is loaded as a sprite");
    }

    let scene = frost::Scene::new(frost::SceneNode {
        // Seen from above there is no sky, so the clear colour is the
        // ground: the same moonlit grey-green the flipbook sits in, taken
        // for soil instead of for air.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.07,
                g: 0.09,
                b: 0.11,
                a: 1.0,
            },
        }),
        // The trail's node and the animal's, both empty until the first
        // frame builds them — the trail first, so the animal crawls over
        // the soil it marks.
        children: vec![
            Box::new(frost::SceneNode::default()),
            Box::new(frost::SceneNode::default()),
        ],
        ..Default::default()
    });

    if let Err(err) = frost::run_configured(
        scene,
        Centipede {
            t: 0.0,
            segments: Vec::new(),
            head: Vagabond::new([0.0, 0.0], std::f32::consts::PI),
            gait: 0.0,
            path: TrackPath::default(),
            pending: Vec::new(),
            rig: rig::Rig::new(sizes),
            shapes,
            placements: Vec::new(),
            rng: frost::Rng::new(),
            rebuild: true,
            next_growth: GROW_EVERY,
            lift_phase: Vec::new(),
            prints: Vec::new(),
            diag,
        },
        frost::Config {
            // The fixed 1920x1080 render target, as in `dogs_name`, and a
            // window that opens at the same 16:9 shape in logical units:
            // the same picture on every display, stretched, keeping its
            // shape as the window moves.
            window_size: Some(WINDOW),
            render_size: Some(WINDOW),
            ..Default::default()
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
