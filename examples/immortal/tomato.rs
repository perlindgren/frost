//! The tomato that grows on a plant's bloom slots: the fruit body grows
//! out of a slot's flower center from zero to [TOMATO_MAX_SCALE] of its
//! natural size over [TOMATO_GROW_TIME] seconds after the flower
//! finishes, the white fruit body (`tomato.png`) tinted dark green to
//! red, with the dark calyx and stem (`tomato_fg.png`) drawn on top, both
//! pinned to [TOMATO_TOP] so the body's top sits on the flower's center
//! and the fruit hangs below it.
//!
//! A [`Tomato`] value is one slot's fruit: its picked state — whether its
//! pivot has been reparented out of the plant's tree — and, once it has
//! ripened, the aging-clock moment of its ripe moment, the stamp its
//! staleness runs from. A fully grown fruit stales on the plant's aging
//! clock — the one [plant::Plant::age] advances every frame, with or
//! without water: [STALE_DELAY] seconds after its ripe moment, its body
//! modulates from ripe red to the dark red of [TOMATO_STALE] over
//! [STALE_TIME]; at full staleness the fruit is overgrown, the moment the
//! example drops it and regrows the slot. A regrown fruit runs the
//! slot's restarted schedule and stamps its own ripe moment.

use super::plant::FLOWER_GROW_TIME;

/// How long a tomato takes to grow from zero to its full size, in seconds
/// of the plant's growth clock: it starts the moment its flower is fully
/// grown and reaches full size [TOMATO_GROW_TIME] seconds later, growing
/// out of the same point.
pub const TOMATO_GROW_TIME: f32 = 10.0;

/// How long a fully grown tomato holds its ripe red before it starts to
/// stale, in seconds of the plant's aging clock — the one
/// [plant::Plant::age] advances every frame, water or not — so a ripe
/// fruit waits and stales even while the plant's growth clock withers
/// backward on a dry reserve.
pub const STALE_DELAY: f32 = 8.0;

/// How long the staleness takes, in seconds of the plant's aging clock:
/// once [STALE_DELAY] has passed after full growth, the fruit's body
/// modulates from ripe red to [TOMATO_STALE] over this time.
pub const STALE_TIME: f32 = 8.0;

/// The tomato's full growth scale, in its own image's units: the fruit
/// body grows to a third of its natural size, so at full growth it hangs
/// below the flower, not over it.
pub const TOMATO_MAX_SCALE: f32 = 1.0 / 3.0;

/// The tomato's [TOMATO_TOP] pixel, in the tomato image's own pixels:
/// the body's top at its horizontal center, which the
/// [tomato_leaf_offset] translate pins to its leaf's parent's origin, so
/// the body's top sits on the flower's center and the fruit hangs below
/// it.
pub const TOMATO_TOP: (f32, f32) = (315.0, 90.0);

/// The tomato background's color at the start of its growth: dark green.
pub const TOMATO_GREEN: frost::Color = frost::Color {
    r: 0.13,
    g: 0.4,
    b: 0.13,
    a: 1.0,
};

/// The tomato background's color at full growth: red.
pub const TOMATO_RED: frost::Color = frost::Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

/// The tomato background's color at full staleness: dark red.
pub const TOMATO_STALE: frost::Color = frost::Color {
    r: 0.4,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

/// The tomato body's child index in a slot's tomato pivot — the first
/// leaf — a white `tomato.png` sprite, tinted by the growth.
pub const TOMATO_BG: usize = 0;

/// The tomato's calyx-and-stem child index in a slot's tomato pivot — the
/// second leaf — the dark `tomato_fg.png` sprite, drawn on top of the
/// body, un-tinted.
pub const TOMATO_FG: usize = 1;

/// The tomato's growth, 0..1, at plant clock `t`, for a bloom that
/// started at `bloom_start`: zero until the flower is fully grown — the
/// start plus [plant::FLOWER_GROW_TIME] — then rising to 1 over
/// [TOMATO_GROW_TIME].
fn tomato_growth(t: f32, bloom_start: f32) -> f32 {
    ((t - bloom_start - FLOWER_GROW_TIME) / TOMATO_GROW_TIME).clamp(0.0, 1.0)
}

/// The tomato body's tint at growth `g`, in 0..1: the dark green of
/// [TOMATO_GREEN] at zero, running through the mix to the ripe red of
/// [TOMATO_RED] at one.
fn tomato_color(g: f32) -> frost::Color {
    TOMATO_GREEN.lerp(TOMATO_RED, g)
}

/// The translate that puts the tomato's [TOMATO_TOP] pixel — the body's
/// top at its horizontal center — on its leaf's origin, for an image of
/// the given size: the negation of that point's node-local offset — with
/// the same y flip as the slice joints, so the body's top sits on the
/// leaf's parent's origin and the fruit hangs below it.
pub fn tomato_leaf_offset(size: [f32; 2]) -> [f32; 2] {
    let a = [TOMATO_TOP.0 - size[0] / 2.0, size[1] / 2.0 - TOMATO_TOP.1];
    [-a[0], -a[1]]
}

/// One bloom slot's tomato: its picked state and, once it has ripened,
/// the aging-clock moment of its ripe moment.
#[derive(Clone, Copy)]
pub struct Tomato {
    /// Whether the fruit is currently picked: its pivot has been
    /// reparented out of the plant's tree — out of the slot's children —
    /// so the plant's layout must skip it; a failed drop unharvests it
    /// and restores the pivot, and a drop kept in the basket regrows the
    /// bloom, the pivot gone and a fresh one in its place.
    harvested: bool,
    /// The aging-clock moment the fruit first reached full growth,
    /// stamped by [Tomato::stamp_ripe] on the frame it ripens — the stamp
    /// its staleness runs from — and reset to `None` by [Tomato::regrow],
    /// so the regrown fruit stamps its own ripe moment.
    ripened_at: Option<f32>,
}

impl Tomato {
    /// A fresh, unharvested fruit that has not ripened: staleness zero,
    /// the ripe stamp unset.
    pub const fn new() -> Self {
        Tomato {
            harvested: false,
            ripened_at: None,
        }
    }

    /// Whether the fruit is currently picked: its pivot has been
    /// reparented out of the plant's tree, so the plant's layout must skip
    /// the slot's tomato pivot.
    pub fn is_harvested(&self) -> bool {
        self.harvested
    }

    /// Marks the fruit picked, now that its pivot has been reparented out
    /// of the plant's tree, so the plant's layout skips it.
    pub fn harvest(&mut self) {
        self.harvested = true;
    }

    /// Clears the picked mark, now that the pivot has been reparented back
    /// into the slot, so the plant's layout reposes it.
    pub fn unharvest(&mut self) {
        self.harvested = false;
    }

    /// The growth-clock moment the fruit ripens, for a bloom that starts
    /// at `bloom_start`: the flower's [plant::FLOWER_GROW_TIME] plus this
    /// [TOMATO_GROW_TIME] after it.
    pub fn ripe_time(bloom_start: f32) -> f32 {
        bloom_start + FLOWER_GROW_TIME + TOMATO_GROW_TIME
    }

    /// Whether the fruit is ripe at growth clock `t`, for a bloom that
    /// starts at `bloom_start`: its growth has reached full — the clock
    /// has passed [ripe_time].
    pub fn is_ripe(&self, t: f32, bloom_start: f32) -> bool {
        t >= Self::ripe_time(bloom_start)
    }

    /// Stamps the ripe moment, if it has not been stamped yet and the
    /// fruit has just ripened at growth clock `t`, aging clock `age`: the
    /// stamp is the aging clock's value at the fruit's ripe moment on the
    /// growth clock — [ripe_time] — exact for any frame size, the two
    /// clocks running in lockstep within a frame, so the ripe moment maps
    /// to `age + (ripe_time - t)`.
    pub fn stamp_ripe(&mut self, t: f32, age: f32, bloom_start: f32) {
        if self.ripened_at.is_none() {
            let ripe_time = Self::ripe_time(bloom_start);
            if t >= ripe_time {
                self.ripened_at = Some(age + (ripe_time - t));
            }
        }
    }

    /// The aging-clock moment the fruit first reached full growth, if it
    /// has ripened: the stamp its staleness runs from.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn ripened_at(&self) -> Option<f32> {
        self.ripened_at
    }

    /// The fruit's growth, 0..1, at growth clock `t`, for a bloom that
    /// starts at `bloom_start`: zero until the flower is fully grown, then
    /// rising to 1 over [TOMATO_GROW_TIME].
    pub fn growth(&self, t: f32, bloom_start: f32) -> f32 {
        tomato_growth(t, bloom_start)
    }

    /// The staleness, 0..1, at aging clock `age`: zero until
    /// [STALE_DELAY] seconds after the ripe moment — the [ripened_at]
    /// stamp — then rising to 1 over [STALE_TIME]; unripened fruit is
    /// unstale.
    pub fn stale(&self, age: f32) -> f32 {
        match self.ripened_at {
            Some(r) => ((age - r - STALE_DELAY) / STALE_TIME).clamp(0.0, 1.0),
            None => 0.0,
        }
    }

    /// Whether the fruit is fully overgrown at aging clock `age`: its
    /// staleness has reached full, so it has reached its final dark red
    /// and the example drops it, regrowing the slot.
    pub fn is_overgrown(&self, age: f32) -> bool {
        self.stale(age) >= 1.0
    }

    /// Resets the fruit for a regrown bloom: the picked mark clears, and
    /// the ripe stamp clears, so the regrown fruit grows from zero on the
    /// slot's restarted schedule and stamps its own ripe moment.
    pub fn regrow(&mut self) {
        self.harvested = false;
        self.ripened_at = None;
    }

    /// Lays the fruit out in `pivot`, a shapeless pivot that is the slot's
    /// tomato child — whose origin is the flower's center, which the
    /// caller has laid out on the slot's spawn point — at growth clock
    /// `t`, aging clock `age`, for a bloom that started at
    /// `bloom_start`: the pivot's scale is the growth times
    /// [TOMATO_MAX_SCALE], the body's [TOMATO_BG] leaf and the calyx's
    /// [TOMATO_FG] leaf sit at [tomato_leaf_offset] of the loaded shapes,
    /// shaped only from growth on, and the body's tint runs [tomato_color]
    /// from dark green to ripe red, modulated toward [TOMATO_STALE] by
    /// the staleness. A pivot sent back from a failed drop gets its stale
    /// carry transform reset to the plant's own.
    pub fn layout(
        &self,
        pivot: &mut frost::SceneNode,
        t: f32,
        age: f32,
        bloom_start: f32,
        body: &frost::Shape,
        fg: &frost::Shape,
    ) {
        let g = self.growth(t, bloom_start);
        let [ox, oy] = tomato_leaf_offset(body.sprite_size().expect("the tomato is a sprite"));
        pivot.transform = frost::Transform::identity();
        pivot.scale = [
            g * TOMATO_MAX_SCALE,
            g * TOMATO_MAX_SCALE,
        ];
        let bg = &mut pivot.children[TOMATO_BG];
        bg.transform = frost::Transform::translate([ox, oy]);
        if g > 0.0 {
            if bg.shape.is_none() {
                bg.shape = Some(body.clone());
            }
            bg.modulate = tomato_color(g).lerp(TOMATO_STALE, self.stale(age));
        } else {
            bg.shape = None;
        }
        // The foreground: the dark calyx and stem, drawn on top,
        // unmodulated.
        let fg_leaf = &mut pivot.children[TOMATO_FG];
        fg_leaf.transform = frost::Transform::translate([ox, oy]);
        if g > 0.0 {
            if fg_leaf.shape.is_none() {
                fg_leaf.shape = Some(fg.clone());
            }
        } else {
            fg_leaf.shape = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fruit stamped ripe at aging-clock moment `ripened`, for the
    /// staleness tests: they run the aging clock on from a ripe fruit.
    fn stamped(ripened: f32) -> Tomato {
        let mut t = Tomato::new();
        t.ripened_at = Some(ripened);
        t
    }

    /// The [Tomato::stale] curve: zero until [STALE_DELAY] seconds after
    /// the ripe moment, then rising to 1 over [STALE_TIME], holding;
    /// unripened fruit is unstale, whatever the aging clock.
    #[test]
    fn the_stale_curve_runs_on_the_aging_clock() {
        for (dt, st) in [
            (0.0, 0.0),
            (STALE_DELAY * 0.99, 0.0),
            (STALE_DELAY + 0.001, 0.001 / STALE_TIME),
            (STALE_DELAY + STALE_TIME * 0.5, 0.5),
            (STALE_DELAY + STALE_TIME, 1.0),
            (STALE_DELAY + STALE_TIME + 50.0, 1.0),
        ] {
            let t = stamped(0.0);
            assert!((t.stale(dt) - st).abs() < 1e-6, "stale at dt = {dt}");
        }
        assert_eq!(Tomato::new().stale(1000.0), 0.0);
    }

    /// [Tomato::is_overgrown] turns on at the end of the stale period —
    /// the ripe moment plus [STALE_DELAY] and [STALE_TIME] — and stays on.
    #[test]
    fn overgrown_is_the_end_of_the_stale_period() {
        let over_at = STALE_DELAY + STALE_TIME;
        assert!(!stamped(0.0).is_overgrown(over_at - 0.001));
        assert!(stamped(0.0).is_overgrown(over_at));
        assert!(stamped(0.0).is_overgrown(over_at + 100.0));
    }

    /// [Tomato::stamp_ripe] stamps the ripe moment exactly, whatever the
    /// frame size: before it, nothing is stamped; a frame that crosses
    /// the ripe moment stamps the aging clock's value at the ripe moment
    /// — not the frame's end — and the stamp sticks.
    #[test]
    fn stamp_ripe_stamps_the_ripe_moment_mid_frame() {
        let start = 13.0;
        let ripe_time = Tomato::ripe_time(start);

        // Before the ripe moment, nothing is stamped.
        let mut t = Tomato::new();
        t.stamp_ripe(ripe_time - 1.0, ripe_time - 1.0, start);
        assert_eq!(t.ripened_at(), None, "an unripe fruit stays unstamped");

        // One frame crosses the ripe moment: the stamp is the ripe
        // moment, not the frame's end.
        t.stamp_ripe(ripe_time + 4.0, ripe_time + 4.0, start);
        assert!(
            (t.ripened_at().expect("the fruit ripened this frame") - ripe_time).abs() < 1e-6,
        );

        // The stamp sticks: a later frame does not restamp.
        t.stamp_ripe(ripe_time + 9.0, ripe_time + 9.0, start);
        assert!((t.ripened_at().unwrap() - ripe_time).abs() < 1e-6);
    }

    /// A [Tomato::regrow] clears the picked mark and the ripe stamp, and
    /// the regrown fruit stamps its own ripe moment, on the slot's
    /// restarted schedule.
    #[test]
    fn a_regrown_fruit_stamps_its_own_ripe_moment() {
        let mut t = Tomato::new();
        t.harvest();
        assert!(t.is_harvested());

        let t1 = 10.0;
        t.stamp_ripe(Tomato::ripe_time(t1), Tomato::ripe_time(t1), t1);
        assert_eq!(t.ripened_at(), Some(Tomato::ripe_time(t1)));

        t.regrow();
        assert!(!t.is_harvested(), "the regrow clears the picked mark");
        assert_eq!(t.ripened_at(), None, "the regrow clears the stamp");

        // The restarted schedule: not ripe at the old ripe moment, and
        // the new stamp is the new ripe moment.
        let t2 = 50.0;
        let ripe_time = Tomato::ripe_time(t2);
        t.stamp_ripe(ripe_time - 10.0, ripe_time - 10.0, t2);
        assert_eq!(t.ripened_at(), None, "the new fruit is not ripe yet");
        t.stamp_ripe(ripe_time + 5.0, ripe_time + 5.0, t2);
        assert_eq!(t.ripened_at(), Some(ripe_time));
    }

    /// [Tomato::growth] is zero until the flower is fully grown — the
    /// bloom's start plus [plant::FLOWER_GROW_TIME] — then rises to 1 over
    /// [TOMATO_GROW_TIME], holding.
    #[test]
    fn growth_starts_when_the_flower_is_full() {
        let start = 13.0;
        let t = Tomato::new();
        assert_eq!(t.growth(start + FLOWER_GROW_TIME - 0.001, start), 0.0);
        assert_eq!(t.growth(start + FLOWER_GROW_TIME, start), 0.0);
        let mid = start + FLOWER_GROW_TIME + TOMATO_GROW_TIME / 2.0;
        assert!((t.growth(mid, start) - 0.5).abs() < 1e-6);
        assert_eq!(t.growth(start + FLOWER_GROW_TIME + TOMATO_GROW_TIME, start), 1.0);
        assert_eq!(
            t.growth(start + FLOWER_GROW_TIME + TOMATO_GROW_TIME + 100.0, start),
            1.0,
        );
    }

    /// [tomato_leaf_offset] pins the body's [TOMATO_TOP] pixel to its
    /// leaf's origin: for the 638×469 tomato image the body center hangs
    /// 144.5 px below the stem, 4 px to its right.
    #[test]
    fn the_leaf_offset_pins_the_body_top() {
        let [ox, oy] = tomato_leaf_offset([638.0, 469.0]);
        assert!((ox - 4.0).abs() < 1e-6, "stem x offset");
        assert!((oy + 144.5).abs() < 1e-6, "stem y offset");
    }

    /// [tomato_color] runs from the dark green of [TOMATO_GREEN] at zero
    /// growth to the ripe red of [TOMATO_RED] at full growth.
    #[test]
    fn the_body_tint_runs_dark_green_to_red() {
        assert_eq!(tomato_color(0.0), TOMATO_GREEN);
        assert_eq!(tomato_color(1.0), TOMATO_RED);
    }
}
