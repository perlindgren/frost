//! A left-to-right series of stages; each stage is a set of branches fed the
//! same signal, and each branch is a serial chain of one or more cards.
use crate::routing::NodeId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    /// A lane's run, above the wire: the same local fork as [`Side::Over`]
    /// around the card the run starts at — the moving card splits from
    /// that lane before the upstream card, runs above it for that one
    /// column and joins back right after it; no wider bracket is drawn.
    Above,
    /// A lane's run, below the wire, as [`Side::Above`] is above it.
    Below,
    /// Onto a card's face, pulling up: a fork of its own around that one
    /// column — the moving card runs beside the target alone, splitting
    /// before it and joining right after it.
    Over,
    /// Onto a card's face, pulling down, as [`Side::Over`] is above it.
    Under,
    /// The ring a fork draws where its trunk fans out into lanes: dropping
    /// on it lands the card in a stage of its own ahead of that whole fork.
    Split,
    /// The ring a split stage draws where its lanes merge: dropping on
    /// it lands the card in a stage of its own behind the whole fork.
    Join,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub card: NodeId,
    pub side: Side,
}
/// One lane of a stage: its serial chain of cards, and the columns of the
/// stage it runs across. `start` is the column the head sits in; `end` is
/// the first column past the tail, where the lane's wire joins back. A lane
/// that runs the whole stage spans `(0, width)` — the shape every lane had
/// before local forks existed — while a lane of a local fork starts after
/// the stage's split or ends before its merge, and a lane longer than its
/// chain carries its tail's wire across the columns it spans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lane<T = NodeId> {
    pub start: usize,
    pub end: usize,
    pub chain: Vec<T>,
}
impl<T> Lane<T> {
    /// A lane as given, with its span clamped to be legal: no shorter than
    /// the chain it carries.
    pub fn spanning(start: usize, end: usize, chain: Vec<T>) -> Self {
        let mut lane = Self { start, end, chain };
        lane.settle();
        lane
    }
    /// A lane exactly as long as its chain, starting at `start`.
    pub fn from(start: usize, chain: Vec<T>) -> Self {
        let end = start + chain.len();
        Self { start, end, chain }
    }
    fn settle(&mut self) {
        self.end = self.end.max(self.start + self.chain.len());
    }
}
impl Lane<NodeId> {
    /// A stage's newest lane: the card on a lane of its own at the front.
    pub fn one(id: NodeId) -> Self {
        Self::from(0, vec![id])
    }
}
/// A stage as read off cards whose lanes carry no spans of their own yet —
/// every lane runs the whole stage, and the stage is as wide as its longest
/// chain. This is what a file written before local forks meant.
pub fn legacy_lanes(chains: Vec<Vec<NodeId>>) -> Vec<Lane> {
    let width = chains.iter().map(Vec::len).max().unwrap_or(1);
    chains
        .into_iter()
        .map(|chain| Lane::spanning(0, width, chain))
        .collect()
}
/// The widest a stage gets: one column past the last lane's last card.
fn width(stage: &[Lane]) -> usize {
    stage
        .iter()
        .map(|lane| lane.start + lane.chain.len())
        .max()
        .unwrap_or(0)
}
/// Pull the stage's right edge back to the last column a card still stands
/// in: a removal that leaves the last column empty takes it off the stage,
/// and every lane that ran past the new edge is shortened to reach it.
fn settle(stage: &mut [Lane]) {
    loop {
        let edge = width(stage);
        if edge == 0 {
            return;
        }
        let occupied = stage
            .iter()
            .any(|lane| lane.start < edge && edge - 1 < lane.start + lane.chain.len());
        if occupied {
            for lane in stage.iter_mut() {
                lane.end = lane.end.max(lane.start + lane.chain.len()).min(edge);
            }
            return;
        }
        for lane in stage.iter_mut() {
            lane.end = edge - 1;
            lane.settle();
        }
    }
}

/// Moves an existing card, or a new card already appended as a stage and a
/// branch of its own. IDs are retained, and empty branches and stages
/// collapse. Dropping on either side of a card chains the moving card into
/// that card's branch at that position, whichever lane of a split it is and
/// whether or not it is the lane's head; the lane grows a column and every
/// lane that ran across that column grows with it, while a lane standing
/// right of the new column slides along. Dropping above or below a lane's
/// wire, or onto a card's face pulling up or down, opens a local fork
/// around the one card that was aimed at (for a wire, the card the run
/// starts at): the mover splits from that card's own lane before its
/// column, runs beside it alone above or below, and joins back right after
/// that column, leaving every other lane exactly as it was; dropping on a
/// fork's front ring lands the card in a new stage ahead of
/// that whole fork, and on its rear ring behind it.
pub fn place(
    stages: &[Vec<Lane>],
    moving: NodeId,
    target: Target,
) -> Result<Vec<Vec<Lane>>, String> {
    if moving == target.card {
        return Ok(stages.to_vec());
    }
    let mut result = stages.to_vec();
    // Lift the card out of every lane it stands in — a rack only ever holds
    // it once, but the lift is written as "the card leaves the stage", not
    // as "the card leaves the one place we found it", so no stale copy of a
    // hand-edited file can survive the drop.
    let mut lifted = false;
    for stage in &mut result {
        for lane in &mut *stage {
            let before = lane.chain.len();
            lane.chain.retain(|id| *id != moving);
            lifted |= lane.chain.len() != before;
        }
        stage.retain(|lane| !lane.chain.is_empty());
        settle(stage);
    }
    if !lifted {
        return Err("Dragged card no longer exists".to_string());
    }
    result.retain(|stage| !stage.is_empty());
    let (column, branch, row) = find(&result, target.card).ok_or("Drop target no longer exists")?;
    match target.side {
        // A ring is the fork's own door, not a lane's: the card steps out
        // of the trunk into a stage of its own, ahead of the fan-out or
        // behind the merge, whatever lane the ring's anchor stands in. A
        // stage that is no longer a fork merely gains its next stage here —
        // the same column a chain would have grown.
        Side::Split => result.insert(column, vec![Lane::one(moving)]),
        Side::Join => result.insert(column + 1, vec![Lane::one(moving)]),
        // Beside any card, head included, the moving card steps into the
        // lane it was aimed at at that card's position: the lane grows the
        // column the card lands in, and the stage's other lanes follow it —
        // one that ran across the new column grows over it, one standing
        // right of the head of the stage stays where it is, and one
        // standing wholly right of the new column slides right with it.
        Side::Left | Side::Right => {
            let at = if target.side == Side::Left {
                row
            } else {
                row + 1
            };
            let lane = &mut result[column][branch];
            let position = lane.start + at;
            lane.chain.insert(at, moving);
            lane.end += 1;
            for (index, other) in result[column].iter_mut().enumerate() {
                if index == branch {
                    continue;
                }
                if other.start > position || (other.start == position && position > 0) {
                    other.start += 1;
                    other.end += 1;
                } else if other.end > position || (other.end == position && other.start == 0) {
                    other.end += 1;
                }
                other.settle();
            }
        }
        // Above or below a lane's wire the card opens the very same local
        // fork as a face drop — around the card the wire starts at, which
        // is the upstream end of the run it was aimed at: the lane splits
        // before that card, the mover runs beside it across that column
        // alone, above or below the target's row, and they join back right
        // after it. No other lane is cut and no wider bracket ever opens.
        // Onto the card itself the drop opens a fork of its own around the
        // target alone: the target's own lane splits before the target's
        // column — everything before it keeps running as the lane it was —
        // and the moving card runs beside the target across that column
        // and joins back right after it. No other lane is cut, moved or
        // re-sliced: a lane that ran the stage runs the stage still, and
        // it is that lane's wire, undisturbed, that meets the local join.
        Side::Above | Side::Below | Side::Over | Side::Under => {
            let stage = &mut result[column];
            let lane = stage[branch].clone();
            let column_at = lane.start + row;
            let head = lane.chain[..row].to_vec();
            let tail = lane.chain[row + 1..].to_vec();
            let mover_above = matches!(target.side, Side::Above | Side::Over);
            let mut replacement = Vec::new();
            if !head.is_empty() {
                replacement.push(Lane::spanning(lane.start, column_at, head));
            }
            if mover_above {
                replacement.push(Lane::spanning(column_at, column_at + 1, vec![moving]));
                replacement.push(Lane::spanning(column_at, column_at + 1, vec![target.card]));
            } else {
                replacement.push(Lane::spanning(column_at, column_at + 1, vec![target.card]));
                replacement.push(Lane::spanning(column_at, column_at + 1, vec![moving]));
            }
            if !tail.is_empty() {
                replacement.push(Lane::spanning(column_at + 1, lane.end, tail));
            }
            stage.splice(branch..branch + 1, replacement);
        }
    }
    settle(&mut result[column]);
    Ok(result)
}
fn find(stages: &[Vec<Lane>], id: NodeId) -> Option<(usize, usize, usize)> {
    stages.iter().enumerate().find_map(|(column, stage)| {
        stage.iter().enumerate().find_map(|(branch, lane)| {
            lane.chain
                .iter()
                .position(|&candidate| candidate == id)
                .map(|row| (column, branch, row))
        })
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    /// A stage rack read the old way: every chain is a lane spanning the
    /// whole stage, the stage as wide as its longest chain.
    fn rack(stages: Vec<Vec<Vec<NodeId>>>) -> Vec<Vec<Lane>> {
        stages.into_iter().map(legacy_lanes).collect()
    }
    fn lane(start: usize, end: usize, chain: &[u64]) -> Lane {
        Lane::spanning(start, end, chain.iter().map(|id| NodeId(*id)).collect())
    }
    #[test]
    fn above_makes_a_branch_and_a_side_chains_onto_one() {
        // A plain serial rack: three stages, each a branch of one.
        let original = rack(vec![
            vec![vec![NodeId(1)]],
            vec![vec![NodeId(3)]],
            vec![vec![NodeId(5)]],
        ]);
        // Below card 1 puts card 3 beside it, in a branch of its own.
        let parallel = place(
            &original,
            NodeId(3),
            Target {
                card: NodeId(1),
                side: Side::Below,
            },
        )
        .unwrap();
        assert_eq!(
            parallel,
            rack(vec![
                vec![vec![NodeId(1)], vec![NodeId(3)]],
                vec![vec![NodeId(5)]]
            ])
        );
        // Right of card 1 chains card 3 onto card 1's branch instead.
        let chained = place(
            &parallel,
            NodeId(3),
            Target {
                card: NodeId(1),
                side: Side::Right,
            },
        )
        .unwrap();
        assert_eq!(
            chained,
            rack(vec![
                vec![vec![NodeId(1), NodeId(3)]],
                vec![vec![NodeId(5)]]
            ])
        );
        // Left of card 3 chains onto the front of card 3's branch, and an
        // emptied branch collapses with the card that left it.
        assert_eq!(
            place(
                &parallel,
                NodeId(1),
                Target {
                    card: NodeId(3),
                    side: Side::Left
                }
            )
            .unwrap(),
            chained
        );
    }
    #[test]
    fn a_chain_inserts_between_two_cards_and_keeps_the_order() {
        let original = rack(vec![vec![vec![NodeId(1), NodeId(2)], vec![NodeId(3)]]]);
        let chained = place(
            &original,
            NodeId(3),
            Target {
                card: NodeId(1),
                side: Side::Right,
            },
        )
        .unwrap();
        assert_eq!(
            chained,
            rack(vec![vec![vec![NodeId(1), NodeId(3), NodeId(2)]]])
        );
    }
    #[test]
    fn left_of_a_split_lane_head_prepends_inside_that_lane() {
        // S feeds a split, and card 7 arrives from the plugin list. The
        // fork keeps every lane it had; the card steps into the lane it
        // was aimed at, ahead of that lane's head.
        let original = rack(vec![
            vec![vec![NodeId(1)]],
            vec![vec![NodeId(3)], vec![NodeId(5)]],
            vec![vec![NodeId(7)]],
        ]);
        assert_eq!(
            place(
                &original,
                NodeId(7),
                Target {
                    card: NodeId(3),
                    side: Side::Left
                }
            )
            .unwrap(),
            rack(vec![
                vec![vec![NodeId(1)]],
                vec![vec![NodeId(7), NodeId(3)], vec![NodeId(5)]]
            ])
        );
        assert_eq!(
            place(
                &original,
                NodeId(7),
                Target {
                    card: NodeId(5),
                    side: Side::Left
                }
            )
            .unwrap(),
            rack(vec![
                vec![vec![NodeId(1)]],
                vec![vec![NodeId(3)], vec![NodeId(7), NodeId(5)]]
            ])
        );
    }
    #[test]
    fn a_card_dropped_on_a_split_ring_lands_ahead_of_the_fork() {
        // The ring belongs to the stage, not the lane: whichever card it
        // is anchored to, the card steps in front of the whole fan-out,
        // between the stage before the fork and the fork itself.
        let original = rack(vec![
            vec![vec![NodeId(1)]],
            vec![vec![NodeId(3)], vec![NodeId(5)]],
            vec![vec![NodeId(7)]],
        ]);
        for anchor in [NodeId(3), NodeId(5)] {
            assert_eq!(
                place(
                    &original,
                    NodeId(1),
                    Target {
                        card: anchor,
                        side: Side::Split
                    }
                )
                .unwrap(),
                rack(vec![
                    vec![vec![NodeId(1)]],
                    vec![vec![NodeId(3)], vec![NodeId(5)]],
                    vec![vec![NodeId(7)]]
                ])
            );
        }
        // A lane head pulled onto its own stage's front ring steps out
        // ahead of the fork too: the emptied lane collapses first, and the
        // card stands as a stage of its own right before the fork.
        let forked = rack(vec![vec![vec![NodeId(3), NodeId(1)], vec![NodeId(5)]]]);
        assert_eq!(
            place(
                &forked,
                NodeId(1),
                Target {
                    card: NodeId(3),
                    side: Side::Split
                }
            )
            .unwrap(),
            rack(vec![
                vec![vec![NodeId(1)]],
                vec![vec![NodeId(3)], vec![NodeId(5)]]
            ])
        );
    }
    #[test]
    fn left_of_a_lane_middles_or_a_single_lane_still_chains() {
        // Deep inside a lane, left inserts into that lane's chain.
        let original = rack(vec![
            vec![vec![NodeId(1), NodeId(2)], vec![NodeId(3)]],
            vec![vec![NodeId(7)]],
        ]);
        assert_eq!(
            place(
                &original,
                NodeId(7),
                Target {
                    card: NodeId(2),
                    side: Side::Left
                }
            )
            .unwrap(),
            rack(vec![vec![
                vec![NodeId(1), NodeId(7), NodeId(2)],
                vec![NodeId(3)]
            ]])
        );
        // Pulling one lane of a split and dropping it left of the other
        // head empties the source lane first, so the stage is a plain
        // chain by the time the drop resolves.
        let split = rack(vec![vec![vec![NodeId(3)], vec![NodeId(5)]]]);
        assert_eq!(
            place(
                &split,
                NodeId(5),
                Target {
                    card: NodeId(3),
                    side: Side::Left
                }
            )
            .unwrap(),
            rack(vec![vec![vec![NodeId(5), NodeId(3)]]])
        );
    }
    #[test]
    fn right_of_a_split_lane_chains_into_that_lane_whichever_tail_it_is() {
        // The fork feeds card 7, and card 1 wants to join a lane. Right of
        // either lane's trailing card chains into that lane's sequence —
        // the merge is not a drop point of its own.
        let original = rack(vec![
            vec![vec![NodeId(1)]],
            vec![vec![NodeId(3)], vec![NodeId(5)]],
            vec![vec![NodeId(7)]],
        ]);
        assert_eq!(
            place(
                &original,
                NodeId(1),
                Target {
                    card: NodeId(3),
                    side: Side::Right
                }
            )
            .unwrap(),
            rack(vec![
                vec![vec![NodeId(3), NodeId(1)], vec![NodeId(5)]],
                vec![vec![NodeId(7)]]
            ])
        );
        assert_eq!(
            place(
                &original,
                NodeId(1),
                Target {
                    card: NodeId(5),
                    side: Side::Right
                }
            )
            .unwrap(),
            rack(vec![
                vec![vec![NodeId(3)], vec![NodeId(5), NodeId(1)]],
                vec![vec![NodeId(7)]]
            ])
        );
    }
    #[test]
    fn right_of_a_lane_middles_or_a_single_lane_still_chains() {
        // Before a lane's tail, right inserts into that lane's chain.
        let original = rack(vec![
            vec![vec![NodeId(1), NodeId(2)], vec![NodeId(3)]],
            vec![vec![NodeId(7)]],
        ]);
        assert_eq!(
            place(
                &original,
                NodeId(7),
                Target {
                    card: NodeId(1),
                    side: Side::Right
                }
            )
            .unwrap(),
            rack(vec![vec![
                vec![NodeId(1), NodeId(7), NodeId(2)],
                vec![NodeId(3)]
            ]])
        );
        // Pulling one lane of a split and dropping it right of the other
        // tail empties the source lane first, so the stage is a plain
        // chain by the time the drop resolves.
        let split = rack(vec![vec![vec![NodeId(3)], vec![NodeId(5)]]]);
        assert_eq!(
            place(
                &split,
                NodeId(5),
                Target {
                    card: NodeId(3),
                    side: Side::Right
                }
            )
            .unwrap(),
            rack(vec![vec![vec![NodeId(3), NodeId(5)]]])
        );
    }
    #[test]
    fn moving_between_stages_collapses_an_empty_source() {
        let original = rack(vec![
            vec![vec![NodeId(1)]],
            vec![vec![NodeId(3)], vec![NodeId(5)]],
        ]);
        // Above card 5's branch: its own stage is emptied and pruned, and the
        // new branch slots between the target's neighbours.
        assert_eq!(
            place(
                &original,
                NodeId(1),
                Target {
                    card: NodeId(5),
                    side: Side::Above
                }
            )
            .unwrap(),
            rack(vec![vec![
                vec![NodeId(3)],
                vec![NodeId(1)],
                vec![NodeId(5)]
            ]])
        );
        assert!(
            place(
                &original,
                NodeId(1),
                Target {
                    card: NodeId(9),
                    side: Side::Below
                }
            )
            .is_err()
        );
    }
    #[test]
    fn a_card_dropped_on_a_join_lands_behind_the_fork() {
        let original = rack(vec![
            vec![vec![NodeId(1)]],
            vec![vec![NodeId(3)], vec![NodeId(5)]],
            vec![vec![NodeId(7)]],
        ]);
        // The ring belongs to the stage, not the lane: whichever card it
        // is anchored to, the card walks out behind the whole merge.
        for anchor in [NodeId(3), NodeId(5)] {
            assert_eq!(
                place(
                    &original,
                    NodeId(1),
                    Target {
                        card: anchor,
                        side: Side::Join
                    }
                )
                .unwrap(),
                rack(vec![
                    vec![vec![NodeId(3)], vec![NodeId(5)]],
                    vec![vec![NodeId(1)]],
                    vec![vec![NodeId(7)]]
                ])
            );
        }
        // A lane tail pulled onto its own stage's ring steps out behind
        // the merge too: the emptied lane collapses first, and the card
        // stands as a stage of its own right after the fork.
        let forked = rack(vec![vec![vec![NodeId(3)], vec![NodeId(5), NodeId(1)]]]);
        assert_eq!(
            place(
                &forked,
                NodeId(1),
                Target {
                    card: NodeId(5),
                    side: Side::Join
                }
            )
            .unwrap(),
            rack(vec![
                vec![vec![NodeId(3)], vec![NodeId(5)]],
                vec![vec![NodeId(1)]]
            ])
        );
    }
    #[test]
    fn over_a_plain_lanes_card_forks_the_lane_around_that_card() {
        // The fork brackets one column inside the lane's own stage: what
        // ran before the card stays in the stage, the card and the
        // newcomer run side by side across that one column, and what ran
        // after it carries on to the right in the same stage. Nothing is
        // re-sliced into new stages — the split ring stands before the
        // target's column and the join ring right after it.
        let original = rack(vec![
            vec![vec![NodeId(1), NodeId(2), NodeId(3)]],
            vec![vec![NodeId(9)]],
        ]);
        assert_eq!(
            place(
                &original,
                NodeId(9),
                Target {
                    card: NodeId(2),
                    side: Side::Over
                }
            )
            .unwrap(),
            vec![vec![
                lane(0, 1, &[1]),
                lane(1, 2, &[9]),
                lane(1, 2, &[2]),
                lane(2, 3, &[3]),
            ]]
        );
        assert_eq!(
            place(
                &original,
                NodeId(9),
                Target {
                    card: NodeId(2),
                    side: Side::Under
                }
            )
            .unwrap(),
            vec![vec![
                lane(0, 1, &[1]),
                lane(1, 2, &[2]),
                lane(1, 2, &[9]),
                lane(2, 3, &[3]),
            ]]
        );
        // A head card's fork opens at the stage's own left edge and a tail
        // card's closes at its right edge; the rest of the lane is untouched.
        assert_eq!(
            place(
                &original,
                NodeId(9),
                Target {
                    card: NodeId(1),
                    side: Side::Over
                }
            )
            .unwrap(),
            vec![vec![
                lane(0, 1, &[9]),
                lane(0, 1, &[1]),
                lane(1, 3, &[2, 3]),
            ]]
        );
        assert_eq!(
            place(
                &original,
                NodeId(9),
                Target {
                    card: NodeId(3),
                    side: Side::Under
                }
            )
            .unwrap(),
            vec![vec![
                lane(0, 2, &[1, 2]),
                lane(2, 3, &[3]),
                lane(2, 3, &[9]),
            ]]
        );
    }
    #[test]
    fn a_card_pulled_from_its_lane_and_laid_over_a_later_card_re_splits_it() {
        // Removing the mover first is what makes this read honestly: the
        // lane closes over the gap, and the fork brackets where the card
        // stands by the time the drop resolves.
        let original = rack(vec![vec![vec![NodeId(1), NodeId(2), NodeId(3)]]]);
        assert_eq!(
            place(
                &original,
                NodeId(1),
                Target {
                    card: NodeId(3),
                    side: Side::Over
                }
            )
            .unwrap(),
            vec![vec![lane(0, 1, &[2]), lane(1, 2, &[1]), lane(1, 2, &[3]),]]
        );
    }
    #[test]
    fn under_a_card_inside_a_fork_splits_that_lane_alone() {
        // The user's rack: a fork whose top lane runs Dist -> Ecomplex and
        // whose bottom lane is Clone. Dropping the Tuner just below
        // Ecomplex must NOT re-slice the stage: the outer fork stands, the
        // top lane splits before Ecomplex's column, the Tuner runs beside
        // it and joins right after, and Clone still spans the whole stage.
        let original = rack(vec![
            vec![vec![NodeId(1), NodeId(2)], vec![NodeId(3)]],
            vec![vec![NodeId(9)]],
        ]);
        assert_eq!(
            place(
                &original,
                NodeId(9),
                Target {
                    card: NodeId(2),
                    side: Side::Under
                }
            )
            .unwrap(),
            vec![vec![
                lane(0, 1, &[1]),
                lane(1, 2, &[2]),
                lane(1, 2, &[9]),
                lane(0, 2, &[3]),
            ]]
        );
        // Pulling up instead runs the newcomer above the target's card;
        // every other lane, Clone included, stands exactly where it was.
        assert_eq!(
            place(
                &original,
                NodeId(9),
                Target {
                    card: NodeId(2),
                    side: Side::Over
                }
            )
            .unwrap(),
            vec![vec![
                lane(0, 1, &[1]),
                lane(1, 2, &[9]),
                lane(1, 2, &[2]),
                lane(0, 2, &[3]),
            ]]
        );
    }
    #[test]
    fn a_side_drop_grows_a_local_lanes_span_and_shifts_the_stage_right() {
        // Chain-inserting into a local fork's short lane grows just that
        // lane and every lane that ran across the new column; a lane that
        // ended at the split stands untouched.
        let case_one: Vec<Lane> = vec![
            lane(0, 1, &[1]),
            lane(1, 2, &[2]),
            lane(1, 2, &[9]),
            lane(0, 2, &[3]),
        ];
        let original = vec![case_one, vec![Lane::one(NodeId(7))]];
        assert_eq!(
            place(
                &original,
                NodeId(7),
                Target {
                    card: NodeId(2),
                    side: Side::Right
                }
            )
            .unwrap(),
            vec![vec![
                lane(0, 1, &[1]),
                lane(1, 3, &[2, 7]),
                lane(1, 2, &[9]),
                lane(0, 3, &[3]),
            ]]
        );
    }
    #[test]
    fn a_face_drop_below_a_lane_shorter_card_splits_at_that_card() {
        // The user's other rack: a forked stage whose bottom lane ends
        // early. Dropping card 7 just below card 1 splits the top lane at
        // card 1's column; the short lane has no card there and never
        // crossed it, so it runs on untouched.
        let original = rack(vec![
            vec![vec![NodeId(5), NodeId(1)], vec![NodeId(3)]],
            vec![vec![NodeId(7)]],
        ]);
        assert_eq!(
            place(
                &original,
                NodeId(7),
                Target {
                    card: NodeId(1),
                    side: Side::Under
                }
            )
            .unwrap(),
            vec![vec![
                lane(0, 1, &[5]),
                lane(1, 2, &[1]),
                lane(1, 2, &[7]),
                lane(0, 2, &[3]),
            ]]
        );
    }
    #[test]
    fn pulling_a_local_fork_member_out_keeps_the_stages_spans() {
        // Lifting the Tuner off the rack and parking it above Dist closes
        // its lane, and the stage settles by the cards that still stand
        // in it: the local join column still holds Ecomplex, so it stays,
        // Dist keeps its short span, and Clone keeps the full width it
        // always had. Nothing silently widens back to the old stage edge.
        let case_one: Vec<Lane> = vec![
            lane(0, 1, &[1]),
            lane(1, 2, &[2]),
            lane(1, 2, &[9]),
            lane(0, 2, &[3]),
        ];
        let original = vec![case_one, vec![Lane::one(NodeId(9))]];
        let lifted = place(
            &original,
            NodeId(9),
            Target {
                card: NodeId(1),
                side: Side::Above,
            },
        )
        .unwrap();
        assert_eq!(
            lifted,
            vec![vec![
                lane(0, 1, &[9]),
                lane(0, 1, &[1]),
                lane(1, 2, &[2]),
                lane(0, 2, &[3]),
            ]]
        );
    }
    #[test]
    fn a_wire_drop_below_a_run_forks_around_the_upstream_card() {
        // The user's rack: Dist → Clone → Tuner run the stage in one lane
        // while Ecomplex stands in a stage of its own. Dropping Ecomplex
        // under the Clone→Tuner wire — the pick answers with the run's
        // upstream card, Clone — opens a fork of Clone's own: the split
        // sits on the wire entering Clone, Clone and Ecomplex share one
        // column side by side, the join closes right after them and the
        // Tuner keeps running solo. No full-stage bracket ever appears.
        let original = vec![
            vec![Lane::from(0, vec![NodeId(1), NodeId(3), NodeId(7)])],
            vec![Lane::one(NodeId(5))],
        ];
        let forked = place(
            &original,
            NodeId(5),
            Target {
                card: NodeId(3),
                side: Side::Below,
            },
        )
        .unwrap();
        assert_eq!(
            forked,
            vec![vec![
                lane(0, 1, &[1]),
                lane(1, 2, &[3]),
                lane(1, 2, &[5]),
                lane(2, 3, &[7]),
            ]]
        );
    }
    #[test]
    fn a_wire_drop_above_a_run_lifts_the_mover_over_the_upstream_card() {
        // The same aim from the shoulder above the wire parks the mover
        // above the target's row instead; the topology is otherwise the
        // very same local bracket around the upstream card.
        let original = vec![
            vec![Lane::from(0, vec![NodeId(1), NodeId(3), NodeId(7)])],
            vec![Lane::one(NodeId(5))],
        ];
        let forked = place(
            &original,
            NodeId(5),
            Target {
                card: NodeId(3),
                side: Side::Above,
            },
        )
        .unwrap();
        assert_eq!(
            forked,
            vec![vec![
                lane(0, 1, &[1]),
                lane(1, 2, &[5]),
                lane(1, 2, &[3]),
                lane(2, 3, &[7]),
            ]]
        );
    }
    #[test]
    fn a_wire_drop_in_one_lane_of_a_fork_never_touches_the_others() {
        // A rack already carrying a local fork (Dist, then Ecomplex beside
        // the Tuner's column, Clone spanning) gains one more lane of its
        // own beside Ecomplex when card 5 is dropped above that lane's
        // head. The outer stage, the neighbouring local lane and the
        // spanning Clone are left byte for byte as they were.
        let original = vec![
            vec![
                lane(0, 1, &[1]),
                lane(1, 2, &[2]),
                lane(1, 2, &[9]),
                lane(0, 2, &[3]),
            ],
            vec![Lane::one(NodeId(5))],
        ];
        let forked = place(
            &original,
            NodeId(5),
            Target {
                card: NodeId(2),
                side: Side::Above,
            },
        )
        .unwrap();
        assert_eq!(
            forked,
            vec![vec![
                lane(0, 1, &[1]),
                lane(1, 2, &[5]),
                lane(1, 2, &[2]),
                lane(1, 2, &[9]),
                lane(0, 2, &[3]),
            ]]
        );
    }
}
