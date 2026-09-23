//! The harvest basket: its two aligned sprite halves, the U of invisible
//! walls that holds the dropped fruit inside the cavity, and where the
//! basket group rides in the scene.
//!
//! The basket sits in the scene's bottom left, just right of the items
//! panel — [BASKET_GAP] clear of the panel's right edge and the window's
//! `MARGIN` clear of the bottom border — and rides the same uniform fit
//! scale the panel rides, [basket_scale], so it keeps its proportions
//! across resizes; [basket_center] is the group's center in
//! window-centered user space.
//!
//! A dropped tomato is kept when its body's center is inside the U —
//! [basket_accepts] — the mouth's x range and above the floor, with no
//! upper bound, so fruit dropped above the rim is kept and can pile high.
//! The kept fruit reparents into the shapeless fruit container, the
//! [BASKET_FRUIT] child that [basket_children] builds in draw order, where
//! the [basket_walls] U holds each body inside the cavity.

use frost::{OrientedBox, SceneNode, Shape};

use super::{ITEMS_SIZE, MARGIN};

/// `BasketBack.png` / `BasketFront.png`'s texture size in pixels: the two
/// aligned halves of the harvest basket.
pub const BASKET_IMAGE: [f32; 2] = [271.0, 251.0];

/// The clearance the basket keeps from the items panel's right edge, in
/// pixels.
pub const BASKET_GAP: f32 = 24.0;

/// The basket group's children, in draw order — the order in which
/// [`basket_children`] builds them: the back half under the fruit, the
/// shapeless fruit container the dropped tomatoes reparent into, and the
/// front half over the fruit.
pub const BASKET_BACK: usize = 0;
pub const BASKET_FRUIT: usize = 1;
pub const BASKET_FRONT: usize = 2;

/// The basket's U, in the basket node's local space (unscaled, y up,
/// origin at the sprite's center): three invisible axis-aligned boxes —
/// the left wall, the right wall, and the floor — that hold the dropped
/// fruit inside the cavity, the mouth left open at the top.
pub fn basket_walls() -> [OrientedBox; 3] {
    [
        OrientedBox::new([-124.0, 30.0], [16.0, 70.0]),
        OrientedBox::new([128.0, 30.0], [16.0, 70.0]),
        OrientedBox::new([2.0, -25.0], [130.0, 15.0]),
    ]
}

/// The basket's cavity mouth's x bounds in local space: the inner faces of
/// the [basket_walls] walls.
const BASKET_MOUTH_X: [f32; 2] = [-108.0, 112.0];

/// The basket's floor's top in local space: the inner face of the
/// [basket_walls] bottom box.
const BASKET_FLOOR: f32 = -10.0;

/// The uniform scale the basket rides at: the same fit the items panel
/// uses, so the basket keeps its proportions across resizes.
pub fn basket_scale(h: f32) -> f32 {
    (h - 2.0 * MARGIN) / ITEMS_SIZE[1]
}

/// The basket group's center in window-centered user space, for a window
/// of the given size: in the bottom left, just right of the items panel —
/// [BASKET_GAP] clear of its right edge — and `MARGIN` clear of the
/// bottom border.
pub fn basket_center(w: f32, h: f32) -> [f32; 2] {
    let s = basket_scale(h);
    [
        -(w / 2.0) + MARGIN + ITEMS_SIZE[0] * s + BASKET_GAP + BASKET_IMAGE[0] * s / 2.0,
        -h / 2.0 + MARGIN + BASKET_IMAGE[1] * s / 2.0,
    ]
}

/// Whether a drop at the basket-local point `(lx, ly)` is kept: the body's
/// center is inside the U — the mouth's x range and above the floor. There
/// is no upper bound: fruit may be dropped above the rim, so it can pile
/// high.
pub fn basket_accepts(lx: f32, ly: f32) -> bool {
    BASKET_MOUTH_X[0] < lx && lx < BASKET_MOUTH_X[1] && ly > BASKET_FLOOR
}

/// The basket group's children, in draw order: the back half under the
/// fruit, the shapeless fruit container the dropped tomatoes reparent into
/// — piled on top of each other, with no gravity and no tomato-to-tomato
/// collision — and the front half over the fruit, so a dropped tomato
/// renders behind the front rim and in front of the back.
pub fn basket_children(back: Shape, front: Shape) -> [SceneNode; 3] {
    let mut children = [
        SceneNode::default(),
        SceneNode::default(),
        SceneNode::default(),
    ];
    children[BASKET_BACK].shape = Some(back);
    children[BASKET_FRONT].shape = Some(front);
    children
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The basket rides in the bottom left corner of a 1920×1080 window:
    /// at the panel's fit scale, `BASKET_GAP` clear of the items panel's
    /// right edge and `MARGIN` clear of the bottom border.
    #[test]
    fn basket_fits_just_right_of_the_items_panel() {
        let (w, h) = (1920.0, 1080.0);
        let s = basket_scale(h);
        let c = basket_center(w, h);

        // The same fit the items panel uses.
        assert!((s - (h - 2.0 * MARGIN) / ITEMS_SIZE[1]).abs() < 1e-9);
        // The panel's right edge, from its mid-left placement.
        let panel_right = -(w / 2.0) + MARGIN + ITEMS_SIZE[0] * s;
        assert!(
            (c[0] - BASKET_IMAGE[0] * s / 2.0 - (panel_right + BASKET_GAP)).abs() < 1e-6,
            "the basket's left edge is not BASKET_GAP right of the panel"
        );
        assert!(
            (c[1] - BASKET_IMAGE[1] * s / 2.0 - (-h / 2.0 + MARGIN)).abs() < 1e-6,
            "the basket's bottom edge is not MARGIN off the bottom border"
        );
    }

    /// [basket_accepts] keeps a drop only inside the U's mouth: the x
    /// range between the walls' inner faces, above the floor — with no
    /// upper bound, so fruit dropped above the rim is kept and can pile
    /// high.
    #[test]
    fn basket_accepts_inside_the_u() {
        assert!(basket_accepts(0.0, 0.0), "the cavity's middle");
        assert!(basket_accepts(-107.0, -9.0), "just inside the left mouth");
        assert!(basket_accepts(111.0, -9.0), "just inside the right mouth");
        assert!(basket_accepts(0.0, 500.0), "above the rim, for the pile");
        assert!(!basket_accepts(-108.0, 0.0), "on the left mouth's edge");
        assert!(!basket_accepts(112.0, 0.0), "on the right mouth's edge");
        assert!(!basket_accepts(-109.0, 0.0), "outside the left mouth");
        assert!(!basket_accepts(113.0, 0.0), "outside the right mouth");
        assert!(!basket_accepts(0.0, -10.0), "on the floor");
        assert!(!basket_accepts(0.0, -11.0), "below the floor");
    }

    /// The [basket_walls] U encloses its acceptance region: a small body
    /// centered well inside the mouth — at least its radius clear of the
    /// walls' inner faces, the floor, and the walls' tops — clears every
    /// wall, while a body at the left wall's face sits in it.
    #[test]
    fn basket_walls_enclose_the_acceptance_region() {
        for lx in [-103.0, -60.0, 0.0, 60.0, 107.0] {
            for ly in [-5.0, 20.0, 60.0, 95.0] {
                assert!(basket_accepts(lx, ly));
                let body = frost::Collider::Box(frost::OrientedBox::new([lx, ly], [4.0, 4.0]));
                for wall in basket_walls() {
                    assert!(
                        body.push_out(&frost::Collider::Box(wall)).is_none(),
                        "accepted point ({lx}, {ly}) inside a wall"
                    );
                }
            }
        }
        // The left wall's face: inside it.
        let body = frost::Collider::Box(frost::OrientedBox::new([-124.0, 30.0], [4.0, 4.0]));
        let left = frost::Collider::Box(basket_walls()[0]);
        assert!(
            body.push_out(&left).is_some(),
            "the wall's face must collide"
        );
    }
}
