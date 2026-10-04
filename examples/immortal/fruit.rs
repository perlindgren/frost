//! Fruit: the carried tomato, its flight, basket hand-offs, planting,
//! and the falls of overgrown fruit. The methods here are `Demo`'s —
//! in a binary crate a module may extend the root's types — and they
//! read the `Demo` state (`picking`, `flying`, `falls`, `basket_seed`)
//! that the frame loop drives.

use crate::*;

/// The uniform scale a picked tomato rides at: the plant's fit scale on
/// the tomato's full growth, so a picked fruit is exactly the size it had
/// on the plant.
pub(crate) const TOMATO_PICK_SCALE: f32 = tomato::TOMATO_MAX_SCALE * PLANT_SCALE;

/// The carried tomato's body box's half extents, in window pixels, for a
/// `size`-pixel image: half the fruit's natural size at the pick scale.
pub(crate) fn tomato_pick_half(size: [f32; 2]) -> [f32; 2] {
    [
        size[0] * TOMATO_PICK_SCALE / 2.0,
        size[1] * TOMATO_PICK_SCALE / 2.0,
    ]
}

/// The seed tomato's flight time, in real seconds: the tween from its
/// release point to the nearest not-yet-planted slot's root anchor — or,
/// with every slot planted, back to its original spot in the basket.
pub(crate) const FLY_TIME: f32 = 0.4;

/// The starting tomato's body center in the basket's local space — the
/// basket node's origin at the sprite's center, y up: a ripe fruit
/// resting on the basket's floor — its body's bottom on the floor's top,
/// `basket::BASKET_FLOOR` plus the body's half height at the basket's fit
/// — mid-width.
pub(crate) const BASKET_SEED: [f32; 2] = [0.0, 23.5];

/// The full destination of a falling overgrown tomato, in user space: the
/// bottom anchor of its plant's root segment — the plant's root joint, the
/// plant node's own origin — the point the fruit's body center drops to, in
/// both x and y. The base rock pivots around that joint, so the sway leaves
/// it fixed and the anchor is the node's origin mapped through its
/// transform.
pub(crate) fn root_anchor(plant: &frost::SceneNode) -> [f32; 2] {
    plant.transform.apply([0.0, 0.0])
}

/// The tomato the mouse is carrying: ripe fruit picked off a plant's slot,
/// or a tomato picked up out of the basket.
#[derive(Clone, Copy)]
pub(crate) enum Pick {
    /// Ripe fruit picked off plant `0`'s bloom slot `1`: the slot is
    /// marked harvested while the fruit rides the cursor; the release
    /// keeps it in the basket (the slot regrows) or sends it back to the
    /// slot (the harvest clears).
    Plant(usize, usize),
    /// A tomato picked up out of the basket: `0` is its transform in the
    /// basket's local space — the spot it was picked from, where a release
    /// that does not plant flies back to it.
    Basket(frost::Transform),
}

/// A seed tomato in flight: released outside the basket, it tweens from
/// its release point — the fruit's body center, in user space — to its
/// landing over `FLY_TIME` real seconds. While it flies, the fruit's
/// pivot stays in the held-fruit node, posed every frame by
/// [`Demo::step_flight`]; on arrival a slot landing consumes the seed and
/// starts the slot's plant, a basket landing re-enters the basket.
#[derive(Clone, Copy)]
pub(crate) struct Fly {
    /// The body-center tween in user space, `Repeat::Once`: it clamps at
    /// its destination, so the last frame poses the fruit exactly on the
    /// landing.
    pub(crate) tween: frost::Tween<[f32; 2]>,
    /// The flight's elapsed time, in real seconds, since the drop: the
    /// landing settles when it reaches `FLY_TIME` — the tween's own
    /// elapsed time is private, so the flight keeps its own clock.
    pub(crate) time: f32,
    /// Where the flight lands.
    pub(crate) landing: Landing,
}

/// The landing of a seed's flight.
#[derive(Clone, Copy)]
pub(crate) enum Landing {
    /// The seed plants into this slot on arrival: the fruit is consumed
    /// and the slot's plant starts growing, full-watered.
    Slot(usize),
    /// Every slot is planted: the fruit flies back to the basket and
    /// re-enters it at this basket-local transform — the spot it was
    /// picked up from.
    Basket(frost::Transform),
}

impl Demo {
    /// Keeps the tomatoes out of the basket's walls: the carried fruit
    /// rides the cursor, pushed out of the basket's U every frame, and
    /// each dropped fruit stays exactly where it was released, none
    /// sitting inside a wall. There is no gravity, and no
    /// tomato-to-tomato collision. The seed's flight, while one is in
    /// flight, poses the held-fruit node's pivot itself, so the ride is
    /// skipped — the flight is brief and stays over the grass.
    pub(crate) fn constrain_basket_fruit(&mut self, ctx: &mut frost::Context) {
        // A carried tomato rides the cursor: the pivot under the pointer,
        // the fruit's top pinned to it, in window-centered user space on
        // top of everything. It is pushed out of the basket's U every
        // frame — the body's square matched against the walls mapped into
        // window space — so the fruit cannot be carried through a wall.
        // There is no gravity, and no collision with the fruit already in
        // the basket.
        if self.picking.is_some() {
            let basket = &ctx.scene().root.children[CHILD_BASKET];
            let s = basket.scale[0];
            let [bx, by] = basket.transform.apply([0.0, 0.0]);
            let [ox, oy] =
                tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
            let [hx, hy] = tomato_pick_half(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
            let mut body = [
                self.mouse[0] + TOMATO_PICK_SCALE * ox,
                self.mouse[1] + TOMATO_PICK_SCALE * oy,
            ];
            for wall in basket::basket_walls() {
                let wall_box = frost::Collider::Box(frost::OrientedBox::new(
                    [bx + s * wall.center[0], by + s * wall.center[1]],
                    [s * wall.half[0], s * wall.half[1]],
                ));
                let body_box = frost::Collider::Box(frost::OrientedBox::new(body, [hx, hy]));
                if let Some(push) = body_box.push_out(&wall_box) {
                    body = [
                        body[0] + push.dir[0] * push.depth,
                        body[1] + push.dir[1] * push.depth,
                    ];
                }
            }
            ctx.scene().root.children[CHILD_HELD_FRUIT].children[0].transform =
                frost::Transform::translate([
                    body[0] - TOMATO_PICK_SCALE * ox,
                    body[1] - TOMATO_PICK_SCALE * oy,
                ]);
        }

        // The fruit that has been dropped into the basket: each one stays
        // exactly where it was released — no gravity, no settling, no
        // tomato-to-tomato collision — but none may sit inside a wall:
        // every body's square is pushed out of the U's three walls, in
        // window space, every frame.
        {
            let basket = &ctx.scene().root.children[CHILD_BASKET];
            let s = basket.scale[0];
            let [bx, by] = basket.transform.apply([0.0, 0.0]);
            let [ox, oy] =
                tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
            let [hx, hy] = tomato_pick_half(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
            for fruit in
                &mut ctx.scene().root.children[CHILD_BASKET].children[basket::BASKET_FRUIT].children
            {
                let [tx, ty] = fruit.transform.apply([0.0, 0.0]);
                let mut body = [
                    bx + s * tx + TOMATO_PICK_SCALE * ox,
                    by + s * ty + TOMATO_PICK_SCALE * oy,
                ];
                for wall in basket::basket_walls() {
                    let wall_box = frost::Collider::Box(frost::OrientedBox::new(
                        [bx + s * wall.center[0], by + s * wall.center[1]],
                        [s * wall.half[0], s * wall.half[1]],
                    ));
                    let body_box = frost::Collider::Box(frost::OrientedBox::new(body, [hx, hy]));
                    if let Some(push) = body_box.push_out(&wall_box) {
                        body = [
                            body[0] + push.dir[0] * push.depth,
                            body[1] + push.dir[1] * push.depth,
                        ];
                    }
                }
                fruit.transform = frost::Transform::translate([
                    (body[0] - bx - TOMATO_PICK_SCALE * ox) / s,
                    (body[1] - by - TOMATO_PICK_SCALE * oy) / s,
                ]);
            }
        }
    }

    /// Drops the overgrown tomatoes to the ground: when a slot's fruit
    /// reaches its final dark red — its staleness full, past the ripe
    /// red — the slot's tomato pivot leaves the plant's tree for the
    /// fallen-fruit container and falls straight down from exactly where
    /// it was laid out this frame, while the bloom restarts under the
    /// same semantics as a fruit dropped in the basket: the flower is
    /// removed and regrows from zero, its tomato after it, on the same
    /// water-gated, slowed schedule, which holds the plant's completion
    /// back as it does for a picked fruit.
    pub(crate) fn fall_overgrown_tomatoes(&mut self, ctx: &mut frost::Context) {
        let drops = {
            let plant_nodes = &ctx.scene().root.children[CHILD_PLANTS].children;
            self.plants
                .iter()
                .enumerate()
                .flat_map(|(pi, p)| {
                    // Shared, `Copy` borrows the `move` closure can own
                    // without owning `self`.
                    let plant_node = &plant_nodes[pi];
                    let tomato = &self.tomato;
                    (0..plant::FLOWER_N).filter_map(move |si| {
                        if p.plant.is_harvested(si) || !p.plant.overgrown(si) {
                            return None;
                        }
                        let center = p.plant.tomato_center(
                            si,
                            &plant_node.children[plant::slot_index(si)],
                            tomato,
                        );
                        let world = frost::Transform::scale([PLANT_SCALE, PLANT_SCALE])
                            .compose(&plant_nodes[pi].transform);
                        let spawn = world.apply(center);
                        // The tomato drops to the ground: the bottom anchor
                        // of its plant's root segment, the root joint — not
                        // a height measured from the spawn, which would miss
                        // the sway and the flower's offset above the joint.
                        // It is targeted in both x and y, so the body's
                        // center lands on the anchor itself.
                        let dest = root_anchor(plant_node);
                        Some((pi, si, spawn, dest))
                    })
                })
                .collect::<Vec<_>>()
        };
        if !drops.is_empty() {
            let [ox, oy] =
                tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
            let root = &mut ctx.scene().root;
            for (pi, si, spawn, dest) in drops {
                self.plants[pi].plant.regrow(si);
                // Rehome the pivot — the slot itself stays in the plant's
                // children, as the picked-fruit paths leave it — out of
                // the slot, into the fallen-fruit container, at the pick
                // scale, pinned so the body's center sits exactly where
                // the tomato was laid out this frame.
                let slot =
                    &mut root.children[CHILD_PLANTS].children[pi].children[plant::slot_index(si)];
                let mut pivot = *slot.children.remove(1);
                pivot.scale = [TOMATO_PICK_SCALE, TOMATO_PICK_SCALE];
                pivot.transform = frost::Transform::translate([
                    spawn[0] - TOMATO_PICK_SCALE * ox,
                    spawn[1] - TOMATO_PICK_SCALE * oy,
                ]);
                // The slot takes a fresh, shapeless tomato pivot; the next
                // layout regrows the flower and the tomato from zero.
                slot.children.push(Box::new(frost::SceneNode {
                    children: vec![
                        Box::new(frost::SceneNode::default()),
                        Box::new(frost::SceneNode::default()),
                    ],
                    ..Default::default()
                }));
                root.children[CHILD_FALLEN_FRUIT]
                    .children
                    .push(Box::new(pivot));
                // The body's center is tracked and drops to the root anchor
                // in both x and y; the pivot's transform is re-derived from
                // it every frame, so the initial transform (pinned to the
                // spawn) is only the first layout.
                self.falls.push(Fall::new(dest, spawn));
            }
        }
    }

    /// Steps the fallen fruit: each dropped tomato drops from where it let
    /// go, at the fall speed, toward its plant's root anchor — closing in
    /// in both x and y — until its body center sits on it; then it rolls to
    /// one of the six plant anchors, turning like a wheel and hopping on
    /// its bumps, rests there catching its breath with a squash-and-stretch,
    /// and picks its next anchor to roll to — looping. The drop clip plays
    /// exactly once, on the landing frame; every pivot is re-laid out from
    /// its body position, scale factors, and wheel rotation each frame, so
    /// the breathing pivots on the body's center and the roll reads as a
    /// turning wheel.
    pub(crate) fn step_falls(
        &mut self,
        ctx: &mut frost::Context,
        dt: f32,
        anchors: &[[f32; 2]; PLANT_POS.len()],
    ) {
        let sprite = self.tomato.sprite_size().unwrap_or([0.0, 0.0]);
        let [ox, oy] = tomato::tomato_leaf_offset(sprite);
        // The wheel's rolling radius, in user pixels: half the fruit's
        // scaled width, the wheel's diameter, so the spin turns by the
        // distance it travels over it (the wheel equation).
        let radius = TOMATO_PICK_SCALE * sprite[0] / 2.0;
        let fallen = &mut ctx.scene().root.children[CHILD_FALLEN_FRUIT];
        for (fall, node) in self.falls.iter_mut().zip(fallen.children.iter_mut()) {
            let was_falling = fall.phase == FallPhase::Falling;
            fall.step(dt, &mut self.rng, anchors, radius);
            if was_falling && fall.phase != FallPhase::Falling {
                // The fall reached the ground this frame: the drop clip
                // plays exactly once, on this flip.
                self.sounds.device.play_once(&self.sounds.tomato_drop, None);
            }
            // Lay the pivot out from the body position, the scale factors,
            // and the wheel rotation: the body's center sits at the body
            // position, and the fruit turns about it — so the breathing
            // pivots on the body's center and the roll reads as a turning
            // wheel.
            let [sx, sy] = fall.scale_factors();
            let [bx, by] = fall.body_position();
            node.transform = frost::Transform::translate([
                -TOMATO_PICK_SCALE * sx * ox,
                -TOMATO_PICK_SCALE * sy * oy,
            ])
            .compose(&frost::Transform::rotate(fall.spin))
            .compose(&frost::Transform::translate([bx, by]));
            node.scale = [TOMATO_PICK_SCALE * sx, TOMATO_PICK_SCALE * sy];
            // The depth: a fallen tomato is a ground object, so it rides
            // the ground band, interleaved with the bugs, the worms, and
            // the plants' root slices by the body's y.
            node.order = zorder::ground(by);
        }
    }

    /// A fresh, shaped tomato pivot at the pick scale — the ripe fruit's
    /// body leaf, tinted `tint`, under its calyx-and-stem leaf — for the
    /// re-home's containers: the rides lay the pivot's transform from
    /// their state every frame.
    pub(crate) fn fruit_pivot(&self, tint: frost::Color) -> Box<frost::SceneNode> {
        let [ox, oy] = tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
        Box::new(frost::SceneNode {
            scale: [TOMATO_PICK_SCALE, TOMATO_PICK_SCALE],
            children: vec![
                Box::new(frost::SceneNode {
                    shape: Some(self.tomato.clone()),
                    transform: frost::Transform::translate([ox, oy]),
                    modulate: tint,
                    ..Default::default()
                }),
                Box::new(frost::SceneNode {
                    shape: Some(self.tomato_fg.clone()),
                    transform: frost::Transform::translate([ox, oy]),
                    ..Default::default()
                }),
            ],
            ..Default::default()
        })
    }

    /// A fresh basket fruit pivot at `center`, a body center in the
    /// basket's local space, at the basket's current fit `s`: the ripe
    /// fruit's body leaf, tinted red, under its calyx-and-stem leaf, at
    /// the pick scale over the basket's.
    pub(crate) fn basket_fruit_pivot(&self, center: [f32; 2], s: f32) -> frost::SceneNode {
        let [ox, oy] = tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
        let k = TOMATO_PICK_SCALE / s;
        frost::SceneNode {
            transform: frost::Transform::translate([center[0] - k * ox, center[1] - k * oy]),
            scale: [k, k],
            children: vec![
                Box::new(frost::SceneNode {
                    shape: Some(self.tomato.clone()),
                    transform: frost::Transform::translate([ox, oy]),
                    modulate: tomato::TOMATO_RED,
                    ..Default::default()
                }),
                Box::new(frost::SceneNode {
                    shape: Some(self.tomato_fg.clone()),
                    transform: frost::Transform::translate([ox, oy]),
                    ..Default::default()
                }),
            ],
            ..Default::default()
        }
    }

    /// Picks up the fully developed tomato a plant bears where the pointer
    /// rests — the first hit in plant, then slot, order — the first pick
    /// source tried, ahead of the basket's fruit, and starts carrying it:
    /// the fruit's pivot is reparented out of its plant's slot and into
    /// the held-fruit container (root's [`CHILD_HELD_FRUIT`] child), at
    /// the pick scale, under
    /// the pointer, so it paints above everything. The plant's slot is
    /// marked harvested while it is carried; the pointer must not be on a
    /// slot or holding a tool, and no seed may be in flight, which the
    /// caller checks. Returns the picked plant and slot.
    pub(crate) fn pick_tomato(&mut self, ctx: &mut frost::Context) -> Option<(usize, usize)> {
        // The hit test: every plant's unharvested, ripe slots, their body
        // centers mapped to window space through the plant node's current
        // transform (the sway included), against the pointer's square
        // around the cursor.
        let hit = {
            let plant_nodes = &ctx.scene().root.children[CHILD_PLANTS].children;
            let [hx, hy] = tomato_pick_half(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
            self.plants.iter().enumerate().find_map(|(pi, p)| {
                (0..plant::FLOWER_N)
                    .find(|&si| {
                        if p.plant.is_harvested(si) || !p.plant.ripe(si) {
                            return false;
                        }
                        let center = p.plant.tomato_center(
                            si,
                            &plant_nodes[pi].children[plant::slot_index(si)],
                            &self.tomato,
                        );
                        let world = frost::Transform::scale([PLANT_SCALE, PLANT_SCALE])
                            .compose(&plant_nodes[pi].transform);
                        let [cx, cy] = world.apply(center);
                        (cx - self.mouse[0]).abs() <= hx && (cy - self.mouse[1]).abs() <= hy
                    })
                    .map(|si| (pi, si))
            })
        };
        let (pi, si) = hit?;
        self.plants[pi].plant.harvest(si);
        // Rehome the pivot: out of the slot, into the held-fruit
        // container, at the pick scale under the pointer.
        let root = &mut ctx.scene().root;
        let pivot = root.children[CHILD_PLANTS].children[pi].children[plant::slot_index(si)]
            .children
            .remove(plant::SLOT_TOMATO);
        let mut pivot = *pivot;
        pivot.scale = [TOMATO_PICK_SCALE, TOMATO_PICK_SCALE];
        pivot.transform = frost::Transform::translate(self.mouse);
        root.children[CHILD_HELD_FRUIT]
            .children
            .push(Box::new(pivot));
        Some((pi, si))
    }

    /// Drops the carried tomato picked from (plant, slot) `(pi, si)` — the
    /// plant fruit's drop path; a basket fruit's goes through
    /// [`Self::drop_basket_tomato`] instead. With the body's center inside
    /// the basket's U — the mouth's x range and above the floor — the
    /// pivot is reparented into the basket's
    /// fruit container (the [`basket::BASKET_FRUIT`] child of the [`CHILD_BASKET`]
    /// group), so it renders behind the front half and in front of the
    /// back, and it stays
    /// exactly where it was released: no gravity, no tomato-to-tomato
    /// collision, the fruit piles freely — and the bloom it came from
    /// starts over: the flower is removed and regrows from zero, its
    /// tomato after it, on the same water-gated, slowed schedule.
    /// Otherwise it goes back to its plant — the harvest flag clears and
    /// the pivot is reparented into the slot — where the layout, which
    /// runs later in the same frame, reposes it.
    pub(crate) fn drop_tomato(&mut self, ctx: &mut frost::Context, pi: usize, si: usize) {
        // The body's center in window space, and its position in the
        // basket's local space, where the U is measured.
        let (w, h) = ctx.size();
        let s = basket::basket_scale(h);
        let center = basket::basket_center(w, h);
        let [ox, oy] = tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
        let body = [
            self.mouse[0] + TOMATO_PICK_SCALE * ox,
            self.mouse[1] + TOMATO_PICK_SCALE * oy,
        ];
        let lx = (body[0] - center[0]) / s;
        let ly = (body[1] - center[1]) / s;
        let kept = basket::basket_accepts(lx, ly);

        let root = &mut ctx.scene().root;
        let mut pivot = *root.children[CHILD_HELD_FRUIT].children.remove(0);
        if kept {
            // Scale the pivot into the basket's local space and pin it so
            // the body's center maps back to the release point.
            pivot.scale = [TOMATO_PICK_SCALE / s, TOMATO_PICK_SCALE / s];
            pivot.transform =
                frost::Transform::translate([(body[0] - center[0]) / s, (body[1] - center[1]) / s]);
            root.children[CHILD_BASKET].children[basket::BASKET_FRUIT]
                .children
                .push(Box::new(pivot));
            // The bloom starts over: the plant records the reset — which
            // re-arms the growth clock, the watering hitbox and the water
            // bar — and the slot takes a fresh, shapeless tomato pivot;
            // the next layout removes the flower and regrows it from
            // zero, the tomato after it.
            self.plants[pi].plant.regrow(si);
            root.children[CHILD_PLANTS].children[pi].children[plant::slot_index(si)]
                .children
                .push(Box::new(frost::SceneNode {
                    children: vec![
                        Box::new(frost::SceneNode::default()),
                        Box::new(frost::SceneNode::default()),
                    ],
                    ..Default::default()
                }));
        } else {
            // Snap back to the plant; the layout reposes the pivot this
            // same frame.
            self.plants[pi].plant.unharvest(si);
            root.children[CHILD_PLANTS].children[pi].children[plant::slot_index(si)]
                .children
                .push(Box::new(pivot));
        }
    }

    /// Picks up the tomato the pointer rests on in the basket — the
    /// topmost hit, the last child of the fruit container, the one drawn
    /// on top of the rest — and starts carrying it: the fruit's pivot is
    /// reparented out of the basket's fruit container and into the
    /// held-fruit container (root's [`CHILD_HELD_FRUIT`] child), at the
    /// pick scale under the pointer, so it paints on top of everything.
    /// Returns the pick — the fruit's transform in the basket's local
    /// space, the spot it was picked from.
    pub(crate) fn pick_basket_tomato(&mut self, ctx: &mut frost::Context) -> Option<Pick> {
        let [hx, hy] = tomato_pick_half(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
        let [ox, oy] = tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
        // The hit test: every fruit's body center — its basket-local
        // pivot position, the leaf offset at the basket's fit, and the
        // basket's origin, mapped into window space — against the
        // pointer's square around the cursor, topmost fruit first.
        let hit = {
            let basket = &ctx.scene().root.children[CHILD_BASKET];
            let s = basket.scale[0];
            let [bx, by] = basket.transform.apply([0.0, 0.0]);
            let fruits = &basket.children[basket::BASKET_FRUIT].children;
            fruits.iter().enumerate().rev().find(|(_, pivot)| {
                let [px, py] = pivot.transform.apply([0.0, 0.0]);
                let [cx, cy] = [
                    bx + s * px + TOMATO_PICK_SCALE * ox,
                    by + s * py + TOMATO_PICK_SCALE * oy,
                ];
                (cx - self.mouse[0]).abs() <= hx && (cy - self.mouse[1]).abs() <= hy
            })
        };
        let (i, _) = hit?;
        let root = &mut ctx.scene().root;
        let basket_fruit = &mut root.children[CHILD_BASKET].children[basket::BASKET_FRUIT];
        let origin = basket_fruit.children[i].transform;
        let mut pivot = *basket_fruit.children.remove(i);
        pivot.scale = [TOMATO_PICK_SCALE, TOMATO_PICK_SCALE];
        pivot.transform = frost::Transform::translate(self.mouse);
        root.children[CHILD_HELD_FRUIT]
            .children
            .insert(0, Box::new(pivot));
        Some(Pick::Basket(origin))
    }

    /// Drops the carried tomato picked out of the basket: a release with
    /// the body's center inside the basket's U — the mouth's x range and
    /// above the floor — keeps the fruit in the basket at the release
    /// point, the same reparent into the fruit container as a plant
    /// fruit's basket drop, without a bloom to regrow. A release anywhere
    /// else is a planting attempt: the seed flies to the nearest
    /// not-yet-planted slot's root anchor, consumed on arrival, the slot's
    /// plant starting from a fresh, full-watered seed — or, with every
    /// slot planted, it flies back to the fruit's original spot in the
    /// basket.
    pub(crate) fn drop_basket_tomato(
        &mut self,
        ctx: &mut frost::Context,
        origin: frost::Transform,
    ) {
        let (w, h) = ctx.size();
        let s = basket::basket_scale(h);
        let center = basket::basket_center(w, h);
        let [ox, oy] = tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
        // The body's center in window space, and its position in the
        // basket's local space, where the U is measured.
        let body = [
            self.mouse[0] + TOMATO_PICK_SCALE * ox,
            self.mouse[1] + TOMATO_PICK_SCALE * oy,
        ];
        let lx = (body[0] - center[0]) / s;
        let ly = (body[1] - center[1]) / s;
        if basket::basket_accepts(lx, ly) {
            // Keep the fruit in the basket, at the release point.
            let root = &mut ctx.scene().root;
            let mut pivot = *root.children[CHILD_HELD_FRUIT].children.remove(0);
            pivot.scale = [TOMATO_PICK_SCALE / s, TOMATO_PICK_SCALE / s];
            pivot.transform = frost::Transform::translate([lx, ly]);
            root.children[CHILD_BASKET].children[basket::BASKET_FRUIT]
                .children
                .push(Box::new(pivot));
            return;
        }
        // The release is off the basket: a planting attempt. The seed
        // flies to the nearest not-yet-planted slot's anchor — or, with
        // every slot planted, back to the fruit's original spot in the
        // basket, in window space.
        let planted = std::array::from_fn(|i| self.plants[i].planted);
        let (dest, landing) = match nearest_free_slot(body, w, h, &planted) {
            Some(i) => (plant_anchors(w, h)[i], Landing::Slot(i)),
            None => {
                let [px, py] = origin.apply([0.0, 0.0]);
                (
                    [
                        center[0] + s * px + TOMATO_PICK_SCALE * ox,
                        center[1] + s * py + TOMATO_PICK_SCALE * oy,
                    ],
                    Landing::Basket(origin),
                )
            }
        };
        self.flying = Some(Fly {
            tween: frost::Tween::new(body, dest, FLY_TIME).repeat(frost::Repeat::Once),
            time: 0.0,
            landing,
        });
        // The pivot stays in the held-fruit node for the flight;
        // step_flight poses it each frame and settles it on arrival.
    }

    /// Steps the seed tomato's flight, if one is in flight: the body's
    /// center tweens toward the landing over `FLY_TIME` real seconds, the
    /// pivot riding the held-fruit node at the pick scale, and on arrival
    /// the flight settles — a slot landing consumes the seed and starts
    /// the slot's plant, a basket landing re-enters the fruit into the
    /// basket at its original spot.
    pub(crate) fn step_flight(&mut self, ctx: &mut frost::Context, dt: f32) {
        let mut fly = match self.flying.take() {
            Some(fly) => fly,
            None => return,
        };
        fly.time += dt;
        let center = fly.tween.tick(dt);
        let [ox, oy] = tomato::tomato_leaf_offset(self.tomato.sprite_size().unwrap_or([0.0, 0.0]));
        ctx.scene().root.children[CHILD_HELD_FRUIT].children[0].transform =
            frost::Transform::translate([
                center[0] - TOMATO_PICK_SCALE * ox,
                center[1] - TOMATO_PICK_SCALE * oy,
            ]);
        if fly.time < FLY_TIME {
            self.flying = Some(fly);
            return;
        }
        let landing = fly.landing;
        let root = &mut ctx.scene().root;
        let pivot = *root.children[CHILD_HELD_FRUIT].children.remove(0);
        match landing {
            Landing::Slot(i) => {
                // The seed is consumed: the slot's plant starts growing
                // from a fresh seed, full-watered and white. The player
                // has planted: the bench is a living game now, and the
                // game-over overlay may come up when it goes bare again.
                self.plants[i].planted = true;
                self.plants[i].water = 1.0;
                self.plants[i].dryness = 0.0;
                self.ever_planted = true;
            }
            Landing::Basket(origin) => {
                // Back into the basket, at the fruit's original spot.
                let s = root.children[CHILD_BASKET].scale[0];
                let mut pivot = pivot;
                pivot.scale = [TOMATO_PICK_SCALE / s, TOMATO_PICK_SCALE / s];
                pivot.transform = origin;
                root.children[CHILD_BASKET].children[basket::BASKET_FRUIT]
                    .children
                    .push(Box::new(pivot));
            }
        }
    }

    /// Seeds the basket with its starting tomato — one ripe fruit
    /// resting on the basket's floor, mid-width: a fresh pivot in the
    /// basket's fruit container (the [`basket::BASKET_FRUIT`] child of
    /// the [`CHILD_BASKET`] group), at the basket's fit scale, the body's
    /// center on [`BASKET_SEED`]. The seed's scale depends on the
    /// basket's fit, so it lands on the first frame, after the chrome has
    /// laid the basket out.
    pub(crate) fn seed_basket_tomato(&mut self, ctx: &mut frost::Context) {
        let s = ctx.scene().root.children[CHILD_BASKET].scale[0];
        ctx.scene().root.children[CHILD_BASKET].children[basket::BASKET_FRUIT]
            .children
            .push(Box::new(self.basket_fruit_pivot(BASKET_SEED, s)));
    }
}
