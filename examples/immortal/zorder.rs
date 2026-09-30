//! The scene's depth (draw-order) scheme, for the 2.5D impression that
//! things lower on the screen sit closer to the camera.
//!
//! Frost paints a scene's nodes by their composed draw order — a node's own
//! [`order`](frost::SceneNode::order) plus everything inherited from its
//! ancestors, ascending, with the tree order breaking ties — so this module
//! turns the game's world positions into those order values. The depth
//! convention: a point's order is the negation of its user-y, because the
//! user-y axis points up. A point higher on the screen (larger y) is further
//! back (lower order); a point lower on the screen (smaller y) is closer to
//! the camera (higher order) and paints on top.
//!
//! The orders are laid out in three bands, spaced far enough apart that two
//! different kinds of object can never collide across a band boundary,
//! whatever the window size:
//!
//! * the ground band, [`ground`]: the bugs, the worms, the plants' root
//!   segments, and the fallen fruit — the objects that sit in the soil. They
//!   interleave by their y, so a creature in front of a plant's base paints
//!   over it and one behind paints under it;
//! * the upper band, [`upper`]: the plants' other segments — the stalk
//!   slices above the root and the flower and fruit slots. They sit above
//!   every ground-band object, and are ordered plant by plant so bench slot
//!   0 is furthest back and slot 5 the frontmost;
//! * the chrome, [`GRASS`], [`UI`], and [`OVERLAY`]: the static grass
//!   underlay, the always-on-top panels and held items, and the game-over
//!   overlay.

/// The grass underlay: behind every ground-band object.
pub const GRASS: f32 = -1_000_000.0;

/// The UI chrome — the items panel, the held panel, the active tool, the
/// basket, the immortality badge, and the carried fruit: in front of every
/// ground- and upper-band object.
pub const UI: f32 = 1_000_000.0;

/// The game-over overlay: in front of everything, the UI included.
pub const OVERLAY: f32 = 2_000_000.0;

/// The upper band's floor: far above the highest ground-band order, so every
/// plant segment paints over every bug, worm, and fallen fruit.
const UPPER_BASE: f32 = 100_000.0;

/// The per-slot spacing in the upper band: far above any between-slot y
/// spread, so a later bench slot's segments always paint over an earlier
/// slot's — slot 0 furthest back, slot 5 the frontmost.
const SLOT_GAP: f32 = 10_000.0;

/// The ground band's order for a point at user-y `y`: the negation of the
/// y, so a point lower on the screen (smaller y) sorts above one higher up.
/// The bugs, the worms, the plants' root segments, and the fallen fruit use
/// it, which interleaves the creatures with the plant bases by their screen
/// height.
pub fn ground(y: f32) -> f32 {
    -y
}

/// The upper band's order for the segments of the plant in bench slot
/// `slot`, keyed by the slot's root-anchor user-y `y`: the slot's base —
/// [UPPER_BASE] plus the slot's share of the [SLOT_GAP] spacing — with the
/// anchor's depth folded in. A later slot's base outruns every earlier
/// slot's anchor, so slot 0's segments sit behind slot 5's, and the whole
/// band sits above the ground band.
pub fn upper(slot: usize, y: f32) -> f32 {
    UPPER_BASE + slot as f32 * SLOT_GAP - y
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The six plant anchors in `grass.png`'s pixel space (`y` down), in
    /// bench order — the same points `main.rs` maps to user space.
    const ANCHORS: [[f32; 2]; 6] = [
        [923.0, 514.0],
        [1248.0, 546.0],
        [1633.0, 603.0],
        [739.0, 571.0],
        [1081.0, 640.0],
        [1463.0, 719.0],
    ];

    /// The anchor's user-space y at window height `h`, the way `main.rs`
    /// maps it: `h/2 - pixel_y * h / 1080`.
    fn user_y(anchor: [f32; 2], h: f32) -> f32 {
        h / 2.0 - anchor[1] * h / 1080.0
    }

    /// The band orders for the six anchors at height `h`: the ground band
    /// (the roots) and the upper band (the stalks), one per plant.
    fn bands(h: f32) -> (Vec<f32>, Vec<f32>) {
        let ground = ANCHORS.iter().map(|a| ground(user_y(*a, h))).collect();
        let upper = ANCHORS
            .iter()
            .enumerate()
            .map(|(slot, a)| upper(slot, user_y(*a, h)))
            .collect();
        (ground, upper)
    }

    #[test]
    fn ground_is_ordered_by_depth() {
        // A point lower on the screen (smaller user-y) sorts higher — it
        // paints on top. The back plant's root (slot 0) sits behind the
        // front plant's root (slot 5).
        let (ground, _) = bands(1080.0);
        assert!(ground[0] < ground[5]);
        // And the ordering follows the screen height, not the bench slot,
        // which is the rule for the ground band.
        assert!(ground[4] > ground[3]);
    }

    #[test]
    fn upper_band_sits_above_the_ground_band() {
        // For a range of window heights, the lowest upper-band order (the
        // back plant's stalk) is above the highest ground-band order (the
        // front plant's root): no plant segment ever dips into the ground
        // band, and no bug, worm, or fallen fruit ever rises into the upper
        // band.
        for &h in &[1080.0, 1440.0, 4320.0, 10_800.0] {
            let (ground, upper) = bands(h);
            let max_ground = ground.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            let min_upper = upper.iter().cloned().fold(f32::INFINITY, f32::min);
            assert!(min_upper > max_ground, "h={h}: {min_upper} !> {max_ground}");
        }
    }

    #[test]
    fn upper_band_is_ordered_by_slot() {
        // The front bench slot's stalk sits in front of the back slot's
        // stalk for every pair, at a range of window heights — slot 0
        // behind slot 5 — even though the anchor y's are not monotone in
        // the slot (slot 2 sits lower on the screen than slot 3).
        for &h in &[1080.0, 4320.0, 10_800.0] {
            let (_, upper) = bands(h);
            for i in 0..upper.len() {
                for j in i + 1..upper.len() {
                    assert!(upper[i] < upper[j], "h={h}: slot {i} !< slot {j}");
                }
            }
        }
    }

    #[test]
    fn viper_offset_stays_in_band() {
        // A viper rides one unit above its segment's order on the near side
        // of its orbit (flying right) and one below on the far side (flying
        // left); the offset never pushes it across a band boundary, at any
        // window height.
        for &h in &[1080.0, 4320.0, 10_800.0] {
            let (ground, upper) = bands(h);
            let max_ground = ground.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            let min_upper = upper.iter().cloned().fold(f32::INFINITY, f32::min);
            assert!(max_ground + 1.0 < min_upper, "h={h}: near side crosses");
            assert!(min_upper - 1.0 > max_ground, "h={h}: far side crosses");
        }
    }

    #[test]
    fn chrome_pins_sit_outside_the_world_bands() {
        // The chrome pins sit outside the world bands even at an extreme
        // window size: the grass behind every ground object, the UI in front
        // of every plant segment, and the overlay above the UI.
        let y = 108_000.0; // a full half-height of an extreme window.
        assert!(GRASS < ground(y));
        assert!(UI > upper(5, -y));
        assert!(OVERLAY > UI);
    }
}
