//! A left-to-right series of stages; each stage is a set of branches fed the
//! same signal, and each branch is a serial chain of one or more cards.
use crate::routing::NodeId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Above,
    Below,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub card: NodeId,
    pub side: Side,
}

/// Moves an existing card, or a new card already appended as a stage and a
/// branch of its own. IDs are retained, and empty branches and stages
/// collapse. Dropping on the left or right of a card chains the moving card
/// into that card's branch, before or after the target; dropping above or
/// below puts it in a branch of its own alongside the target's branch.
pub fn place(
    stages: &[Vec<Vec<NodeId>>],
    moving: NodeId,
    target: Target,
) -> Result<Vec<Vec<Vec<NodeId>>>, String> {
    if moving == target.card {
        return Ok(stages.to_vec());
    }
    let mut result = stages.to_vec();
    let (source_stage, source_branch, row) =
        find(&result, moving).ok_or("Dragged card no longer exists")?;
    result[source_stage][source_branch].remove(row);
    if result[source_stage][source_branch].is_empty() {
        result[source_stage].remove(source_branch);
    }
    result.retain(|stage| !stage.is_empty());
    let (column, branch, row) = find(&result, target.card).ok_or("Drop target no longer exists")?;
    match target.side {
        Side::Left => result[column][branch].insert(row, moving),
        Side::Right => result[column][branch].insert(row + 1, moving),
        Side::Above => result[column].insert(branch, vec![moving]),
        Side::Below => result[column].insert(branch + 1, vec![moving]),
    }
    Ok(result)
}
fn find(stages: &[Vec<Vec<NodeId>>], id: NodeId) -> Option<(usize, usize, usize)> {
    stages.iter().enumerate().find_map(|(column, stage)| {
        stage.iter().enumerate().find_map(|(branch, chain)| {
            chain
                .iter()
                .position(|&candidate| candidate == id)
                .map(|row| (column, branch, row))
        })
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn above_makes_a_branch_and_a_side_chains_onto_one() {
        // A plain serial rack: three stages, each a branch of one.
        let original = vec![
            vec![vec![NodeId(1)]],
            vec![vec![NodeId(3)]],
            vec![vec![NodeId(5)]],
        ];
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
            vec![
                vec![vec![NodeId(1)], vec![NodeId(3)]],
                vec![vec![NodeId(5)]]
            ]
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
            vec![vec![vec![NodeId(1), NodeId(3)]], vec![vec![NodeId(5)]]]
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
        let original = vec![vec![vec![NodeId(1), NodeId(2)], vec![NodeId(3)]]];
        let chained = place(
            &original,
            NodeId(3),
            Target {
                card: NodeId(1),
                side: Side::Right,
            },
        )
        .unwrap();
        assert_eq!(chained, vec![vec![vec![NodeId(1), NodeId(3), NodeId(2)]]]);
    }
    #[test]
    fn moving_between_stages_collapses_an_empty_source() {
        let original = vec![
            vec![vec![NodeId(1)]],
            vec![vec![NodeId(3)], vec![NodeId(5)]],
        ];
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
            vec![vec![vec![NodeId(3)], vec![NodeId(1)], vec![NodeId(5)]]]
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
}
