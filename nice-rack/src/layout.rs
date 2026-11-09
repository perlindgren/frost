//! A left-to-right series of stages; cards within each stage are parallel.
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

/// Moves an existing card, or a new card already appended as its own stage.
/// IDs are retained, empty stages collapse, and extraction from a parallel stage
/// creates a separate serial stage to the left/right of the target stage.
pub fn place(
    stages: &[Vec<NodeId>],
    moving: NodeId,
    target: Target,
) -> Result<Vec<Vec<NodeId>>, String> {
    if moving == target.card {
        return Ok(stages.to_vec());
    }
    let mut result = stages.to_vec();
    let (source, row) = find(&result, moving).ok_or("Dragged card no longer exists")?;
    result[source].remove(row);
    result.retain(|stage| !stage.is_empty());
    let (column, row) = find(&result, target.card).ok_or("Drop target no longer exists")?;
    match target.side {
        Side::Left => result.insert(column, vec![moving]),
        Side::Right => result.insert(column + 1, vec![moving]),
        Side::Above => result[column].insert(row, moving),
        Side::Below => result[column].insert(row + 1, moving),
    }
    Ok(result)
}
fn find(stages: &[Vec<NodeId>], id: NodeId) -> Option<(usize, usize)> {
    stages.iter().enumerate().find_map(|(column, stage)| {
        stage
            .iter()
            .position(|&candidate| candidate == id)
            .map(|row| (column, row))
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parallel_drop_and_serial_extraction_preserve_all_card_ids() {
        let original = vec![vec![NodeId(1)], vec![NodeId(3)], vec![NodeId(5)]];
        let parallel = place(
            &original,
            NodeId(3),
            Target {
                card: NodeId(1),
                side: Side::Below,
            },
        )
        .unwrap();
        assert_eq!(parallel, vec![vec![NodeId(1), NodeId(3)], vec![NodeId(5)]]);
        let serial = place(
            &parallel,
            NodeId(3),
            Target {
                card: NodeId(1),
                side: Side::Right,
            },
        )
        .unwrap();
        assert_eq!(serial, original);
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
            original
        );
    }
    #[test]
    fn moving_between_parallel_stages_collapses_an_empty_source() {
        let original = vec![vec![NodeId(1)], vec![NodeId(3), NodeId(5)]];
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
            vec![vec![NodeId(3), NodeId(1), NodeId(5)]]
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
