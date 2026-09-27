//! The fall physics of a dropped, overgrown tomato: it falls straight down
//! to the ground, then rolls to one of the six plant anchors, resting there
//! catching its breath with a squash-and-stretch between rolls, and then
//! picks another anchor to roll to — looping, until the game restarts.
//!
//! The demo keeps its fallen fruit in [super::Demo::falls] — one [Fall] per
//! dropped tomato, in the same order as the fallen-fruit container's
//! pivots — and [super::Demo::step_falls] re-lays each pivot out from its
//! entry's body position, scale factors, and wheel rotation every frame.

use super::{dist2, PLANT_POS};

/// The fall speed of a dropped, overgrown tomato, in user pixels per
/// second: it drops from the point it let go, at this constant speed, along
/// the straight line to its plant's root anchor — the bottom joint of its
/// root segment, where it hits the ground — closing in on it in both x and
/// y until its body center sits on the anchor.
pub const FALL_SPEED: f32 = 400.0;

/// The roll speed of a landed, overgrown tomato, in user pixels per
/// second: once the fall reaches the ground, the fruit picks one of the six
/// plant anchors and rolls there, along the straight line between, at this
/// base pace.
pub const ROLL_SPEED: f32 = 150.0;

/// The range, as a fraction of the base roll speed, for the roll's actual
/// pace: each time the fruit lands back on its line from a bump, its speed
/// toward the target is re-rolled from this range, so the roll stumbles and
/// surges — now a little faster, now a little slower — reading as a loose,
/// unpredictable rattle rather than a metronome.
pub const ROLL_SPEED_RANGE: (f32, f32) = (0.6, 1.4);

/// The range, in seconds, for the wait between two of a rolling tomato's
/// bumps: after each bump — and after the roll starts — the next one is
/// armed a random interval in `[BUMP_IN.0, BUMP_IN.1)` later, so the hops
/// come at an uneven, off-cadence, fairly rapid rattle.
pub const BUMP_IN: (f32, f32) = (0.08, 0.3);

/// The duration of a single bump, in seconds: the hop lifts the fruit off
/// its rolling line and sets it back down over this many seconds, shaped
/// as a half-sine.
pub const BUMP_TIME: f32 = 0.1;

/// The height of a bump's hop, in user pixels: the peak of the half-sine,
/// the farthest the fruit lifts above its rolling line.
pub const BUMP_HEIGHT: f32 = 9.0;

/// The sentinel for [`Fall::bump`]: no bump is running, so the next one is
/// being counted down by [`Fall::bump_in`] instead.
pub const BUMP_NONE: f32 = f32::NEG_INFINITY;

/// The range, in seconds, for how long a tomato that has reached its
/// target rests there, catching its breath with a squash-and-stretch,
/// before it picks its next plant anchor to roll to.
pub const REST: (f32, f32) = (1.5, 3.0);

/// The period of a resting tomato's breathing, in seconds: one full cycle
/// — a squash out and a stretch back in — takes this long.
pub const BREATH_PERIOD: f32 = 2.0;

/// The amplitude of a resting tomato's breathing: the y-scale factor
/// swings between `1.0 + BREATH_AMP` (stretched in) and `1.0 - BREATH_AMP`
/// (squashed out), the x-scale compensating to hold the area.
pub const BREATH_AMP: f32 = 0.12;

/// The phase a fallen overgrown tomato is in.
///
/// A tomato falls straight down to the ground ([`FallPhase::Falling`]),
/// then rolls to one of the six plant anchors ([`FallPhase::Rolling`]),
/// rests there catching its breath with a squash-and-stretch
/// ([`FallPhase::Resting`]), and then picks another anchor to roll to —
/// looping, until the game restarts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FallPhase {
    /// The tomato is falling straight down to the ground.
    Falling,
    /// The tomato has landed and is rolling to a target plant anchor.
    Rolling,
    /// The tomato has reached its target and is resting (breathing) there.
    Resting,
}

/// One overgrown tomato on the ground: its pivot — a child of the
/// fallen-fruit container, in the same order as its [super::Demo::falls]
/// entry — is laid out from this entry's body position and scale factors
/// every frame by [`super::Demo::step_falls`].
///
/// The tomato's *body center* — the fruit's visual center, which hangs
/// below its stem (the pivot's origin) — is what is tracked here and what
/// the roll, the bumps, and the rest are measured from. The pivot's
/// transform is derived from it, so the breathing squash pivots on the
/// body's center, not the stem.
pub struct Fall {
    /// The phase the tomato is in.
    pub phase: FallPhase,
    /// The body center's position, in user space, where the fall stops:
    /// the bottom anchor of the plant's root segment, the root joint, where
    /// the fruit hits the ground — targeted in both x and y.
    pub land_pos: [f32; 2],
    /// Whether the drop clip has played for this fruit; it plays exactly
    /// once, on the frame the fall reaches the ground.
    pub landed: bool,
    /// The body center's position, in user space: the base position,
    /// without the bump's hop.
    pub body: [f32; 2],
    /// The target plant anchor, in user space, the fruit is rolling to.
    pub target: [f32; 2],
    /// The body position the tomato started its current roll from.
    pub start: [f32; 2],
    /// The roll progress, 0.0 (at `start`) to 1.0 (at `target`).
    pub progress: f32,
    /// The straight-line distance from `start` to `target`, in user
    /// pixels: the roll's progress is advanced by `ROLL_SPEED * dt` over
    /// it.
    pub distance: f32,
    /// The running bump's progress, 0.0 (take-off) to 1.0 (back on the
    /// line), or [`BUMP_NONE`] while no bump is running.
    pub bump: f32,
    /// The seconds left before the next bump arms, while one is not
    /// running.
    pub bump_in: f32,
    /// The tomato's wheel rotation, in radians, accumulated as it rolls:
    /// the fruit turns like a wheel, the angle advancing with the distance
    /// it travels, so it reads as rolling rather than sliding. Rolling to
    /// the right turns it clockwise (negative, the y-up frame), rolling to
    /// the left counter-clockwise (positive). It is zeroed whenever the
    /// fruit is not rolling, so it stands upright while it falls and rests.
    pub spin: f32,
    /// The tomato's current roll speed, in user pixels per second: the
    /// base roll pace re-rolled from [`ROLL_SPEED_RANGE`] each time the
    /// fruit lands back on its line from a bump, so the roll stumbles and
    /// surges toward the target.
    pub speed: f32,
    /// The seconds the tomato has spent resting at its target.
    pub rest: f32,
    /// How long this rest lasts, in seconds, before the next roll starts.
    pub rest_for: f32,
}

impl Fall {
    /// A freshly dropped tomato: it starts in the falling phase, with the
    /// body at the spawn position, heading for the root anchor, no roll
    /// running, no bump armed, and standing upright (no wheel rotation).
    pub fn new(land_pos: [f32; 2], body: [f32; 2]) -> Self {
        Self {
            phase: FallPhase::Falling,
            land_pos,
            landed: false,
            body,
            target: [0.0, 0.0],
            start: [0.0, 0.0],
            progress: 0.0,
            distance: 0.0,
            bump: BUMP_NONE,
            bump_in: 0.0,
            spin: 0.0,
            speed: ROLL_SPEED,
            rest: 0.0,
            rest_for: 0.0,
        }
    }

    /// Picks a random plant anchor — one that is not the tomato's current
    /// spot, so the roll always has somewhere to go — and starts rolling
    /// there: the roll restarts from the current body position, with a
    /// fresh bump countdown.
    pub fn start_roll(&mut self, rng: &mut frost::Rng, anchors: &[[f32; 2]; PLANT_POS.len()]) {
        let from = self.body;
        // Walk the anchor ring until one is far enough away; the ring is
        // only six, so a full pass always finds a distinct one.
        let mut i = rng.next_u64() as usize % anchors.len();
        let mut pass = 0;
        while dist2(anchors[i], from) < 1.0 && pass < anchors.len() {
            i = (i + 1) % anchors.len();
            pass += 1;
        }
        self.target = anchors[i];
        self.start = from;
        self.progress = 0.0;
        self.distance = dist2(self.start, self.target).sqrt();
        self.speed = ROLL_SPEED;
        self.bump_in = rng.in_range(BUMP_IN.0, BUMP_IN.1);
        self.bump = BUMP_NONE;
    }

    /// Advances the tomato one frame by `dt` seconds, driving the phase
    /// forward: the fall closes in on the root anchor in both x and y and
    /// the first roll starts on landing, the roll runs its bumps, turns
    /// like a wheel, and reaches its target, and the rest breathes out and
    /// the next roll starts. `radius` is the fruit's rolling radius, in
    /// user pixels — the wheel's spin turns by the distance traveled over
    /// it, clockwise rolling right and counter-clockwise rolling left.
    pub fn step(
        &mut self,
        dt: f32,
        rng: &mut frost::Rng,
        anchors: &[[f32; 2]; PLANT_POS.len()],
        radius: f32,
    ) {
        match self.phase {
            // The body closes in on the root anchor at the fall speed,
            // along the straight line between, in both x and y; on the
            // landing frame the first roll starts.
            FallPhase::Falling => {
                let dx = self.land_pos[0] - self.body[0];
                let dy = self.land_pos[1] - self.body[1];
                let dist = (dx * dx + dy * dy).sqrt();
                let step_len = FALL_SPEED * dt;
                if dist <= step_len || dist < 1e-6 {
                    self.body = self.land_pos;
                    self.landed = true;
                    self.phase = FallPhase::Rolling;
                    self.start_roll(rng, anchors);
                } else {
                    self.body[0] += dx / dist * step_len;
                    self.body[1] += dy / dist * step_len;
                }
            }
            // The body rolls along the straight line from `start` to
            // `target` at the roll speed, hopping on its bumps and turning
            // like a wheel; on the arrival frame the rest begins and the
            // fruit stands back upright.
            FallPhase::Rolling => {
                if self.distance > 0.0 {
                    self.progress = (self.progress + self.speed * dt / self.distance).min(1.0);
                } else {
                    self.progress = 1.0;
                }
                let from = self.body;
                self.body = [
                    self.start[0] + (self.target[0] - self.start[0]) * self.progress,
                    self.start[1] + (self.target[1] - self.start[1]) * self.progress,
                ];
                // The wheel turns with the distance it travels: a no-slip
                // roll, the angle advancing by the travel over the radius.
                // Rolling to the right turns it clockwise (a negative
                // angle in the y-up frame), rolling to the left turns it
                // counter-clockwise (a positive one).
                if radius > 0.0 {
                    let dx = self.body[0] - from[0];
                    let dir = if dx > 0.0 {
                        -1.0
                    } else if dx < 0.0 {
                        1.0
                    } else {
                        0.0
                    };
                    self.spin += dir * dist2(from, self.body).sqrt() / radius;
                }
                // The bump's half-sine hop runs for `BUMP_TIME`, then the
                // next one is counted down from a fresh random interval. On
                // the landing frame — the hop back on the line — the roll's
                // pace toward the target is re-rolled, so it stumbles and
                // surges.
                if self.bump != BUMP_NONE {
                    self.bump = (self.bump + dt / BUMP_TIME).min(1.0);
                    if self.bump >= 1.0 {
                        self.bump = BUMP_NONE;
                        self.speed =
                            ROLL_SPEED * rng.in_range(ROLL_SPEED_RANGE.0, ROLL_SPEED_RANGE.1);
                    }
                } else {
                    self.bump_in -= dt;
                    if self.bump_in <= 0.0 {
                        self.bump = 0.0;
                        self.bump_in = rng.in_range(BUMP_IN.0, BUMP_IN.1);
                    }
                }
                if self.progress >= 1.0 {
                    self.phase = FallPhase::Resting;
                    self.body = self.target;
                    self.spin = 0.0;
                    self.rest = 0.0;
                    self.rest_for = rng.in_range(REST.0, REST.1);
                }
            }
            // The body holds at the target, breathing upright, until the
            // rest runs out; then the next roll starts.
            FallPhase::Resting => {
                self.rest += dt;
                if self.rest >= self.rest_for {
                    self.phase = FallPhase::Rolling;
                    self.start_roll(rng, anchors);
                }
            }
        }
    }

    /// The body center's position, with the bump's hop added while a bump
    /// is running: the half-sine lifts the fruit off its rolling line and
    /// sets it back down.
    pub fn body_position(&self) -> [f32; 2] {
        if self.phase == FallPhase::Rolling && self.bump != BUMP_NONE {
            [
                self.body[0],
                self.body[1] + BUMP_HEIGHT * (std::f32::consts::PI * self.bump).sin(),
            ]
        } else {
            self.body
        }
    }

    /// The pivot's scale factors for this frame: neutral (no squash) while
    /// the fruit falls or rolls, and the breathing squash-and-stretch
    /// while it rests — the y-scale swings in and out, the x-scale
    /// compensating to hold the fruit's area.
    pub fn scale_factors(&self) -> [f32; 2] {
        if self.phase == FallPhase::Resting {
            let sy =
                1.0 + BREATH_AMP * (2.0 * std::f32::consts::PI * self.rest / BREATH_PERIOD).sin();
            [1.0 / sy, sy]
        } else {
            [1.0, 1.0]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Six simple plant anchors for the fall phase-machine tests: a 3×2
    /// grid, 100 px apart horizontally and 50 px apart vertically.
    fn test_anchors() -> [[f32; 2]; 6] {
        [
            [0.0, 0.0],
            [100.0, 0.0],
            [200.0, 0.0],
            [0.0, 50.0],
            [100.0, 50.0],
            [200.0, 50.0],
        ]
    }

    /// A simple wheel radius, in user pixels, for the fall phase-machine
    /// tests: any positive value drives the spin; its exact size only sets
    /// how fast the wheel turns.
    const TEST_RADIUS: f32 = 20.0;

    /// A fresh fall drops toward the root anchor at the fall speed, in both
    /// x and y, until its body center sits on it, where it lands, plays the
    /// drop clip, and starts rolling to a random anchor.
    #[test]
    fn the_fall_lands_at_the_root_anchor_and_starts_rolling() {
        let mut rng = frost::Rng::with_seed(42);
        let anchors = test_anchors();
        let mut fall = Fall::new([0.0, 0.0], [50.0, 100.0]);
        assert_eq!(fall.phase, FallPhase::Falling);
        let dt = 1.0 / 60.0;
        for _ in 0..600 {
            fall.step(dt, &mut rng, &anchors, TEST_RADIUS);
            if fall.phase != FallPhase::Falling {
                break;
            }
        }
        assert_eq!(fall.phase, FallPhase::Rolling, "the fall should land and start rolling");
        assert_eq!(fall.body, [0.0, 0.0], "the body should sit on the root anchor");
        assert!(fall.landed, "the drop clip should have played");
        assert!(anchors.contains(&fall.target), "the target should be a plant anchor");
    }

    /// A landed tomato rolls along the straight line from its start to its
    /// target at the roll speed, and begins its rest the frame it reaches
    /// the target, sitting exactly on it.
    #[test]
    fn the_roll_reaches_its_target_and_starts_resting() {
        let mut rng = frost::Rng::with_seed(7);
        let anchors = test_anchors();
        let mut fall = Fall::new([0.0, 0.0], [0.0, 0.0]);
        // One long frame drops it to the ground and starts the first roll.
        fall.step(1.0, &mut rng, &anchors, TEST_RADIUS);
        assert_eq!(fall.phase, FallPhase::Rolling);
        let target = fall.target;
        let dt = 1.0 / 60.0;
        for _ in 0..6000 {
            fall.step(dt, &mut rng, &anchors, TEST_RADIUS);
            if fall.phase != FallPhase::Rolling {
                break;
            }
        }
        assert_eq!(fall.phase, FallPhase::Resting, "the roll should reach its target");
        assert_eq!(fall.body, target, "the body should sit on the target");
    }

    /// A resting tomato breathes — its y-scale squashes below and stretches
    /// above 1.0, its x-scale compensating — and after its random rest it
    /// picks a new, different anchor to roll to.
    #[test]
    fn the_rest_breathes_and_picks_a_new_anchor() {
        let mut rng = frost::Rng::with_seed(99);
        let anchors = test_anchors();
        let mut fall = Fall::new([0.0, 0.0], [0.0, 0.0]);
        fall.step(1.0, &mut rng, &anchors, TEST_RADIUS);
        let dt = 1.0 / 60.0;
        // Roll to the first target and into the rest.
        for _ in 0..6000 {
            fall.step(dt, &mut rng, &anchors, TEST_RADIUS);
            if fall.phase != FallPhase::Rolling {
                break;
            }
        }
        assert_eq!(fall.phase, FallPhase::Resting);
        let first_target = fall.target;
        let mut saw_squash = false;
        let mut saw_stretch = false;
        // Run well past the longest rest (3 s) and a full breath (2 s).
        for _ in 0..400 {
            fall.step(dt, &mut rng, &anchors, TEST_RADIUS);
            if fall.phase != FallPhase::Resting {
                break;
            }
            let [sx, sy] = fall.scale_factors();
            if sy < 1.0 - 1e-3 {
                saw_squash = true;
            }
            if sy > 1.0 + 1e-3 {
                saw_stretch = true;
            }
            // The area is held: the x-scale is the reciprocal of the y.
            assert!((sx * sy - 1.0).abs() < 1e-4, "the breathing holds the area");
        }
        assert!(saw_squash, "the rest should squash out (sy < 1)");
        assert!(saw_stretch, "the rest should stretch in (sy > 1)");
        assert_eq!(fall.phase, FallPhase::Rolling, "the rest should end into a new roll");
        assert_ne!(fall.target, first_target, "the new anchor should differ from the old");
    }

    /// A running bump lifts the body off its rolling line by the half-sine
    /// hop, peaking at `BUMP_HEIGHT`, without touching the x position.
    #[test]
    fn the_bump_lifts_the_body_off_its_line() {
        let mut fall = Fall::new([0.0, 0.0], [0.0, 0.0]);
        fall.phase = FallPhase::Rolling;
        fall.start = [0.0, 0.0];
        fall.target = [100.0, 0.0];
        fall.body = [50.0, 0.0];
        // Mid-hop: the half-sine is at its peak, sin(π/2) = 1.
        fall.bump = 0.5;
        let base = fall.body;
        let pos = fall.body_position();
        assert_eq!(pos[0], base[0], "the bump should not move the x");
        assert!(
            (pos[1] - (base[1] + BUMP_HEIGHT)).abs() < 1e-4,
            "the mid-hop should peak at BUMP_HEIGHT"
        );
        // Take-off and landing: the half-sine is (within float noise) zero,
        // the body is on the line.
        fall.bump = 0.0;
        let take_off = fall.body_position();
        assert!(
            (take_off[1] - base[1]).abs() < 1e-6,
            "at take-off the body is on the line"
        );
        fall.bump = 1.0;
        let landing = fall.body_position();
        assert!(
            (landing[1] - base[1]).abs() < 1e-6,
            "at landing the body is on the line"
        );
    }

    /// A rolling tomato turns like a wheel: its spin advances by the
    /// distance it travels over the radius each frame (a no-slip roll),
    /// clockwise (negative) when it rolls right and counter-clockwise
    /// (positive) when it rolls left, and it stands back upright (spin
    /// zeroed) the frame it reaches its target and rests.
    #[test]
    fn the_roll_turns_the_tomato_like_a_wheel() {
        let mut rng = frost::Rng::with_seed(5);
        let anchors = test_anchors();
        let mut fall = Fall::new([0.0, 0.0], [0.0, 0.0]);
        // One long frame drops it to the ground and starts the first roll.
        fall.step(1.0, &mut rng, &anchors, TEST_RADIUS);
        assert_eq!(fall.phase, FallPhase::Rolling);
        let dt = 1.0 / 60.0;
        let mut saw_change = false;
        let mut prev_spin = fall.spin;
        let mut prev_body = fall.body;
        // Step the roll until it reaches its target; each frame the spin
        // should advance by the distance traveled over the radius, signed
        // by the roll's horizontal direction.
        for _ in 0..6000 {
            fall.step(dt, &mut rng, &anchors, TEST_RADIUS);
            if fall.phase == FallPhase::Rolling {
                let traveled = dist2(prev_body, fall.body).sqrt();
                if traveled > 1e-6 {
                    let dx = fall.body[0] - prev_body[0];
                    let dir = if dx > 0.0 {
                        -1.0
                    } else if dx < 0.0 {
                        1.0
                    } else {
                        0.0
                    };
                    let expected = prev_spin + dir * traveled / TEST_RADIUS;
                    assert!(
                        (fall.spin - expected).abs() < 1e-3,
                        "the spin should advance by the travel over the radius"
                    );
                    if fall.spin != prev_spin {
                        saw_change = true;
                    }
                    // Rolling right turns clockwise (spin falls), rolling
                    // left counter-clockwise (spin rises).
                    if dx > 0.0 {
                        assert!(
                            fall.spin < prev_spin,
                            "rolling right should turn the wheel clockwise"
                        );
                    } else if dx < 0.0 {
                        assert!(
                            fall.spin > prev_spin,
                            "rolling left should turn the wheel counter-clockwise"
                        );
                    }
                }
                prev_spin = fall.spin;
                prev_body = fall.body;
            } else {
                break;
            }
        }
        assert!(saw_change, "the spin should turn while the tomato rolls");
        assert_eq!(fall.phase, FallPhase::Resting, "the roll should reach its target");
        assert_eq!(fall.spin, 0.0, "the rest should stand the tomato upright");
    }

    /// The pivot's scale factors are neutral (no squash) while the tomato
    /// falls or rolls, and only the rest breathes.
    #[test]
    fn the_scale_factors_are_neutral_outside_the_rest() {
        let mut rng = frost::Rng::with_seed(1);
        let anchors = test_anchors();
        let mut fall = Fall::new([0.0, 0.0], [50.0, 100.0]);
        assert_eq!(fall.phase, FallPhase::Falling);
        assert_eq!(fall.scale_factors(), [1.0, 1.0], "the fall is neutral");
        fall.step(1.0, &mut rng, &anchors, TEST_RADIUS);
        assert_eq!(fall.phase, FallPhase::Rolling);
        assert_eq!(fall.scale_factors(), [1.0, 1.0], "the roll is neutral");
    }

    /// A new roll always targets an anchor that is not the tomato's current
    /// spot, restarts from there, and arms a fresh bump countdown.
    #[test]
    fn the_start_roll_picks_a_distinct_anchor() {
        let mut rng = frost::Rng::with_seed(3);
        let anchors = test_anchors();
        let mut fall = Fall::new([0.0, 0.0], [100.0, 0.0]);
        fall.start_roll(&mut rng, &anchors);
        assert_ne!(fall.target, [100.0, 0.0], "the target should not be the current spot");
        assert!(anchors.contains(&fall.target), "the target should be a plant anchor");
        assert_eq!(fall.start, [100.0, 0.0], "the roll should start at the current spot");
        assert_eq!(fall.progress, 0.0, "the roll should start at zero progress");
        assert!(
            (BUMP_IN.0..BUMP_IN.1).contains(&fall.bump_in),
            "a bump countdown should be armed"
        );
    }

    /// Each time the tomato lands back on its line from a bump, its roll
    /// speed is re-rolled from the speed range — the pace stumbles and
    /// surges — but always stays within the range's bounds.
    #[test]
    fn the_roll_speed_rerolls_on_each_bump_landing() {
        let mut rng = frost::Rng::with_seed(11);
        let anchors = test_anchors();
        let mut fall = Fall::new([0.0, 0.0], [0.0, 0.0]);
        // One long frame drops it to the ground and starts the first roll.
        fall.step(1.0, &mut rng, &anchors, TEST_RADIUS);
        assert_eq!(fall.phase, FallPhase::Rolling);
        assert_eq!(fall.speed, ROLL_SPEED, "the roll starts at the base pace");
        // Force the first bump to land early, so a re-roll is guaranteed to
        // happen during the roll.
        fall.bump_in = 0.0;
        let dt = 1.0 / 60.0;
        let lo = ROLL_SPEED * ROLL_SPEED_RANGE.0;
        let hi = ROLL_SPEED * ROLL_SPEED_RANGE.1;
        let mut saw_change = false;
        let mut prev_speed = fall.speed;
        // Step the roll until it reaches its target; each bump landing
        // re-rolls the speed, and it must always stay within the range.
        for _ in 0..6000 {
            fall.step(dt, &mut rng, &anchors, TEST_RADIUS);
            if fall.phase == FallPhase::Rolling {
                assert!(
                    fall.speed >= lo - 1e-3 && fall.speed <= hi + 1e-3,
                    "the roll speed should stay within the speed range"
                );
                if fall.speed != prev_speed {
                    saw_change = true;
                }
                prev_speed = fall.speed;
            } else {
                break;
            }
        }
        assert!(saw_change, "a bump landing should re-roll the roll speed");
    }
}
