//! Canonical UI drop slots. Two targets are equivalent only if the complete
//! normalized expression after the edit is identical, including branch order.
use super::{Edit, Flow, NodeId, Side, Target};
use std::collections::BTreeMap;

pub struct DropTargets {
    aliases: BTreeMap<Target, Target>,
    groups: BTreeMap<Target, Vec<Target>>,
}
impl DropTargets {
    pub fn new(root: &Flow, moving: Option<u64>) -> Self {
        let mut candidates = Vec::new();
        let ids = root.ids();
        for id in ids.iter().copied().filter(|id| Some(*id) != moving) {
            for side in [Side::Left, Side::Right, Side::Above, Side::Below] {
                candidates.push(Target {
                    scope: None,
                    card: NodeId(id),
                    side,
                });
            }
        }
        fn forks(root: &Flow, moving: Option<u64>, index: &mut usize, targets: &mut Vec<Target>) {
            if matches!(root, Flow::Parallel(_)) {
                let scope = *index;
                *index += 1;
                if let Some(id) = root.ids().into_iter().find(|id| Some(*id) != moving) {
                    for side in [Side::Split, Side::Join] {
                        targets.push(Target {
                            scope: Some(scope),
                            card: NodeId(id),
                            side,
                        });
                    }
                }
            }
            if let Flow::Series(c) | Flow::Parallel(c) = root {
                for child in c {
                    forks(child, moving, index, targets);
                }
            }
        }
        forks(root, moving, &mut 0, &mut candidates);
        let mut probe = 1;
        while ids.contains(&probe) {
            probe += 1;
        }
        let mut results: BTreeMap<Flow, Vec<Target>> = BTreeMap::new();
        for target in candidates {
            let edit = if let Some(id) = moving {
                Edit::Move {
                    id,
                    target: target.card.0,
                    placement: target.side.into(),
                }
            } else {
                Edit::Insert {
                    id: probe,
                    target: target.card.0,
                    placement: target.side.into(),
                }
            };
            if let Ok(result) = root.edit_scoped(edit, target.scope)
                && result != *root
            {
                results.entry(result).or_default().push(target);
            }
        }
        let rank = |target: &Target| {
            let role = match target.side {
                Side::Split | Side::Join => 0,
                Side::Right => 1,
                Side::Left => 2,
                Side::Above | Side::Below => 3,
            };
            (
                role,
                ids.iter()
                    .position(|id| *id == target.card.0)
                    .unwrap_or(usize::MAX),
                target.scope,
                target.side,
            )
        };
        let mut aliases = BTreeMap::new();
        let mut groups = BTreeMap::new();
        for mut group in results.into_values() {
            group.sort_by_key(rank);
            let canonical = group[0];
            for alias in &group {
                aliases.insert(*alias, canonical);
            }
            groups.insert(canonical, group);
        }
        Self { aliases, groups }
    }
    /// Missing entries include self/adjacent no-ops and invalid targets.
    pub fn canonical(&self, target: Target) -> Option<Target> {
        self.aliases.get(&target).copied()
    }
    pub fn equivalents(&self, target: Target) -> &[Target] {
        self.groups.get(&target).map_or(&[], Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn n(id: u64) -> Flow {
        Flow::Plugin(id)
    }
    fn s(c: Vec<Flow>) -> Flow {
        Flow::series(c)
    }
    fn p(c: Vec<Flow>) -> Flow {
        Flow::parallel(c)
    }
    fn target(id: u64, side: Side) -> Target {
        Target {
            scope: None,
            card: NodeId(id),
            side,
        }
    }
    fn fork(id: u64, side: Side, scope: usize) -> Target {
        Target {
            scope: Some(scope),
            card: NodeId(id),
            side,
        }
    }
    #[test]
    fn serial_ends_and_parallel_neighbours_each_have_one_slot() {
        let root = p(vec![s(vec![n(1), n(2)]), n(3), n(4)]);
        let map = DropTargets::new(&root, None);
        assert_eq!(
            map.canonical(target(1, Side::Right)),
            map.canonical(target(2, Side::Left))
        );
        assert_eq!(
            map.canonical(target(3, Side::Below)),
            map.canonical(target(4, Side::Above))
        );
        assert_ne!(
            map.canonical(target(1, Side::Below)),
            map.canonical(target(3, Side::Above)),
            "a local fork around a series member differs from a new outer branch"
        );
    }
    #[test]
    fn boundaries_next_to_forks_and_adjacent_forks_are_not_double_targets() {
        let root = s(vec![n(1), p(vec![n(2), n(3)]), p(vec![n(4), n(5)]), n(6)]);
        let map = DropTargets::new(&root, None);
        assert_eq!(
            map.canonical(target(1, Side::Right)),
            map.canonical(fork(2, Side::Split, 0))
        );
        assert_eq!(
            map.canonical(fork(2, Side::Join, 0)),
            map.canonical(fork(4, Side::Split, 1))
        );
        assert_eq!(
            map.canonical(fork(4, Side::Join, 1)),
            map.canonical(target(6, Side::Left))
        );
    }
    #[test]
    fn detach_is_accounted_for_and_unchanged_moves_have_no_drop_target() {
        let root = s(vec![n(1), n(2), n(3)]);
        let map = DropTargets::new(&root, Some(2));
        assert_eq!(map.canonical(target(1, Side::Right)), None);
        assert_eq!(map.canonical(target(3, Side::Left)), None);
        assert_eq!(map.canonical(target(2, Side::Above)), None);
        assert!(map.canonical(target(1, Side::Above)).is_some());
    }
    #[test]
    fn every_retained_target_is_unique_and_every_alias_preserves_the_result() {
        let roots = [
            s(vec![n(1), n(2), n(3), n(4)]),
            p(vec![n(1), n(2), n(3), n(4)]),
            p(vec![s(vec![n(1), p(vec![n(2), n(3)])]), n(4)]),
        ];
        for root in roots {
            for moving in [None, Some(1), Some(2), Some(3), Some(4)] {
                let map = DropTargets::new(&root, moving);
                let mut unique = std::collections::BTreeSet::new();
                let result = |target: Target| {
                    root.edit_scoped(
                        if let Some(id) = moving {
                            Edit::Move {
                                id,
                                target: target.card.0,
                                placement: target.side.into(),
                            }
                        } else {
                            Edit::Insert {
                                id: 5,
                                target: target.card.0,
                                placement: target.side.into(),
                            }
                        },
                        target.scope,
                    )
                    .unwrap()
                };
                for (canonical, aliases) in &map.groups {
                    let expected = result(*canonical);
                    assert_ne!(expected, root);
                    assert!(unique.insert(expected.clone()));
                    for alias in aliases {
                        assert_eq!(result(*alias), expected);
                        assert_eq!(map.canonical(*alias), Some(*canonical));
                    }
                }
            }
        }
    }
}
