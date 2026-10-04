//! The static scene tree: every node the demo animates, built once
//! from the loaded assets. The `CHILD_*` indices below index this
//! tree's roots, so the contract between the two lives here.

use crate::*;

/// Builds the demo's scene: grass, items, plants, fruit, bugs, the
/// tool layer, the basket, the badge and the overlay, in declaration
/// order matching the `CHILD_*` constants.
pub fn build(assets: &Assets) -> frost::Scene {
    // One plant node, cloned for each plant: its origin is the root joint
    // (plant1's lower joint), positioned by the process every frame. The
    // fit scale is constant — each slice grows individually, riding the
    // node. The five slice children are followed by the FLOWER_N shapeless
    // flower slots, each a pivot that the process lays on its slice's
    // spawn point; a slot's children are the flower leaf and, on top of
    // it, a tomato pivot (the white fruit body leaf under the dark calyx-
    // and-stem leaf, both pinned so the body's top sits on the flower's
    // center and the fruit hangs below), so the blooms and fruit paint on
    // top of the slices and sway with the plant. The process grows and
    // tints the leaves. The clones are cheap `Arc` clones of the slice and
    // bloom pixel buffers.
    let mut plant_children: Vec<Box<frost::SceneNode>> = vec![
        Box::new(frost::SceneNode {
            shape: Some(assets.plant1.clone()),
            ..Default::default()
        }),
        Box::new(frost::SceneNode {
            shape: Some(assets.plant2.clone()),
            ..Default::default()
        }),
        Box::new(frost::SceneNode {
            shape: Some(assets.plant3.clone()),
            ..Default::default()
        }),
        Box::new(frost::SceneNode {
            shape: Some(assets.plant4.clone()),
            ..Default::default()
        }),
        Box::new(frost::SceneNode {
            shape: Some(assets.plant5.clone()),
            ..Default::default()
        }),
    ];
    plant_children.extend((0..plant::FLOWER_N).map(|_| {
        Box::new(frost::SceneNode {
            children: vec![
                // The flower leaf, under the tomato.
                Box::new(frost::SceneNode::default()),
                // The tomato pivot, on top of the flower: shapeless; the
                // process scales it to grow the fruit about the flower and
                // lays its two leaves so the body's top sits on the flower's
                // center, the fruit hanging below it.
                Box::new(frost::SceneNode {
                    children: vec![
                        // The fruit body (background), under the
                        // foreground.
                        Box::new(frost::SceneNode::default()),
                        // The calyx and stem (foreground), on top.
                        Box::new(frost::SceneNode::default()),
                    ],
                    ..Default::default()
                }),
            ],
            ..Default::default()
        })
    }));
    let plant_node = frost::SceneNode {
        scale: [PLANT_SCALE, PLANT_SCALE],
        children: plant_children,
        ..Default::default()
    };

    frost::Scene::new(frost::SceneNode {
        // Dark ground under the grass; only the sparse transparent gaps in
        // the grass texture show it.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.04,
                g: 0.1,
                b: 0.04,
                a: 1.0,
            },
        }),
        // The root's children, in draw order — the `CHILD_*` constants are
        // the indices into this list.
        children: vec![
            Box::new(frost::SceneNode {
                // Stretched to fill the window by the process, every frame;
                // pinned behind every ground-band object by the depth
                // scheme.
                shape: Some(assets.grass.clone()),
                order: zorder::GRASS,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The inventory panel, positioned and scaled by the process
                // every frame: mid left, `MARGIN` clear of the top, bottom,
                // and left borders. Its children are the four bays, one per
                // bay in bay order — a node paints its shape before its
                // children, so a resting tool renders on top of the panel —
                // and they ride its fit on resize. One tool per painted
                // shelf, as [SEEDED_SLOTS] lays them out — the tweezers, the
                // spade, the spray can, and the watering can, top to bottom
                // — each standing on its own board.
                shape: Some(assets.items.clone()),
                order: zorder::UI,
                children: (0..SLOTS)
                    .map(|i| {
                        let (shape, transform, scale) = match SEEDED_SLOTS[i] {
                            Some(tool) => {
                                let (t, s) = slot_rest(i, tool);
                                (
                                    Some(tool.sprite(assets)),
                                    frost::Transform::translate(t),
                                    [s, s],
                                )
                            }
                            None => (None, frost::Transform::translate(slot_local(i)), [1.0, 1.0]),
                        };
                        Box::new(frost::SceneNode {
                            // The tool at rest in its bay — or an empty bay,
                            // a shapeless node waiting for a tool to be
                            // parked in it.
                            transform,
                            scale,
                            shape,
                            ..Default::default()
                        })
                    })
                    .collect(),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The plants on the grass: one plant node per `PLANT_POS`
                // entry, in growth order. The group carries no shape or
                // scale of its own; each plant child holds the fit scale
                // and is positioned by the process every frame. The slices
                // paint on top of the grass; the group sits under the tool
                // node, so the cursor paints above the plants.
                children: (0..PLANT_POS.len())
                    .map(|_| Box::new(plant_node.clone()))
                    .collect(),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The fallen fruit: one child per dropped tomato, in drop
                // order — the overgrown fruit's pivot reparents here when
                // it lets go, and the process lays each fall out on its
                // child until it lands. The group carries no shape or
                // scale of its own; it sits under the held panel, the
                // bees, and the bugs, so the fallen fruit paints on top
                // of the grass and the plants, below everything else.
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The held-items panel in the bottom right corner, anchored
                // there by the process every frame: `MARGIN` clear of the
                // right and bottom borders, at its natural size. Its two
                // children mirror the mouse's tools — child 0 (the left
                // cell) the active one, child 1 (the right cell) the
                // stored one — each painted on top of the panel, since a
                // node paints its shape before its children; the cells are
                // synced by `set_active`.
                shape: Some(assets.held_items.clone()),
                order: zorder::UI,
                children: vec![
                    Box::new(frost::SceneNode {
                        // The left cell: the active tool, empty at start.
                        transform: frost::Transform::translate([
                            held_local(0)[0],
                            held_local(0)[1],
                        ]),
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        // The right cell: the stored tool, empty at start.
                        transform: frost::Transform::translate([
                            held_local(1)[0],
                            held_local(1)[1],
                        ]),
                        ..Default::default()
                    }),
                ],
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The vipers' swarm around the flower bench: one child per
                // bee, in swarm order; the process lays each spawned
                // bee's pose, flip, and wingbeat frame out on its child
                // every frame, and the unspawned slots — the ones no plant
                // layer has spawned yet — never draw. The group carries no
                // shape or scale of its own. It sits under the tool node,
                // so the cursor paints above the bees.
                children: (0..vipers::N)
                    .map(|_| Box::new(frost::SceneNode::default()))
                    .collect(),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The bugs' swarm on the grass: one shape-less child per
                // slot, in spawn order; the process lays each spawned
                // bug's pose, flip, growth, and walk frame out on its
                // child every frame, and the unspawned slots never draw.
                // The group carries no shape or scale of its own; it sits
                // under the tool node, so the cursor paints above the
                // bugs.
                children: (0..BUG_N)
                    .map(|_| Box::new(frost::SceneNode::default()))
                    .collect(),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The lice's swarm on the grass: the bugs' life in a
                // different skin, one shape-less child per slot; the
                // process lays each louse's pose, flip, growth, and walk
                // frame out on its child every frame. The group sits
                // under the tool node, so the cursor paints above.
                children: (0..LICE_N)
                    .map(|_| Box::new(frost::SceneNode::default()))
                    .collect(),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The worms' swarm under the bench: one shape-less child
                // per slot, in pool order; the process lays each worm's
                // pose, rotation, scale, and peristaltic frame out on its
                // child every frame, and the underground slots — the ones
                // in the soil — never draw. The group carries no shape or
                // scale of its own; it sits under the tool node, so the
                // cursor paints above the worms.
                children: (0..worms::N)
                    .map(|_| Box::new(frost::SceneNode::default()))
                    .collect(),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The active tool, starting with no shape: the mouse starts
                // holding no tool at all. Every switch — a slot swap or the
                // right-button switch — changes the shape — and the scale,
                // since the spray frames are drawn at their natural size —
                // on this same node; the stored tool is mirrored in the
                // held panel's right cell instead. It sits under the basket,
                // the badge, and the held-fruit node, so the cursor paints
                // above the panel, the plants, the bees, and the bugs, and
                // the carried fruit paints above the cursor.
                order: zorder::UI,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The harvest basket in the bottom left, just right of the
                // items panel, positioned and scaled by the process every
                // frame; its children — the back half, the fruit
                // container, and the front half — are built by
                // `basket_children`.
                children: basket::basket_children(
                    assets.basket_back.clone(),
                    assets.basket_front.clone(),
                )
                .into_iter()
                .map(Box::new)
                .collect(),
                order: zorder::UI,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The immortality badge in the top right: half its
                // natural size, `MARGIN` clear of the top and right
                // borders, tinted to a faint watermark, positioned by the
                // process every frame. It sits under the held-fruit node,
                // so a carried tomato still paints above it.
                shape: Some(assets.immortality.clone()),
                order: zorder::UI,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The held-fruit container: the carried tomato's pivot
                // reparents here while the mouse button is down, so the
                // fruit paints above everything — the panel, the plants,
                // the basket, the badge, the cursor. The group carries no
                // shape or scale of its own; the child rides the cursor in
                // window-centered user space.
                order: zorder::UI,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The game-over overlay, above everything: the process
                // gives the node its dim veil over the whole window — and
                // clears it again — every frame, and its three children
                // carry, in draw order, the "Game Over" title above the
                // window's center, the Play button's rectangle below it,
                // and the button's label on the button's center.
                children: vec![
                    Box::new(frost::SceneNode {
                        // The "Game Over" title.
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        // The Play button's rectangle.
                        ..Default::default()
                    }),
                    Box::new(frost::SceneNode {
                        // The button's label.
                        ..Default::default()
                    }),
                ],
                order: zorder::OVERLAY,
                ..Default::default()
            }),
        ],
        ..Default::default()
    })
}
