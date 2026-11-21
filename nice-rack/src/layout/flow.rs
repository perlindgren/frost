//! A series-parallel signal expression with ordered branches. Each plugin occurs
//! exactly once. Editing is transactional; removing a node closes its old lane.
use super::{Lane, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Flow {
    #[default]
    Empty,
    Plugin(u64),
    Series(Vec<Flow>),
    Parallel(Vec<Flow>),
}
// Settings pass through ron::Value when window geometry is updated. Store
// plain string tags and records: RON enum identifiers do not survive that path.
impl Serialize for Flow {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut record =
            serializer.serialize_struct("Flow", if matches!(self, Self::Empty) { 1 } else { 2 })?;
        match self {
            Self::Empty => record.serialize_field("kind", "empty")?,
            Self::Plugin(id) => {
                record.serialize_field("kind", "plugin")?;
                record.serialize_field("id", id)?;
            }
            Self::Series(children) => {
                record.serialize_field("kind", "series")?;
                record.serialize_field("children", children)?;
            }
            Self::Parallel(children) => {
                record.serialize_field("kind", "parallel")?;
                record.serialize_field("children", children)?;
            }
        }
        record.end()
    }
}
impl<'de> Deserialize<'de> for Flow {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Record {
            kind: String,
            #[serde(default)]
            id: Option<u64>,
            #[serde(default)]
            children: Vec<Flow>,
        }
        let record = Record::deserialize(deserializer)?;
        match record.kind.as_str() {
            "empty" if record.id.is_none() && record.children.is_empty() => Ok(Self::Empty),
            "plugin" if record.children.is_empty() => record
                .id
                .map(Self::Plugin)
                .ok_or_else(|| serde::de::Error::custom("Missing plugin ID")),
            "series" if record.id.is_none() => Ok(Self::series(record.children)),
            "parallel" if record.id.is_none() => Ok(Self::parallel(record.children)),
            _ => Err(serde::de::Error::custom("Invalid routing expression")),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Before,
    After,
    Above,
    Below,
    BeforeFork,
    AfterFork,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Move {
        id: u64,
        target: u64,
        placement: Placement,
    },
    Insert {
        id: u64,
        target: u64,
        placement: Placement,
    },
    Append {
        id: u64,
    },
    Remove {
        id: u64,
    },
}
impl Flow {
    pub fn series(children: Vec<Self>) -> Self {
        Self::Series(children).normalized()
    }
    pub fn parallel(children: Vec<Self>) -> Self {
        Self::Parallel(children).normalized()
    }
    pub fn normalized(self) -> Self {
        let parallel = matches!(self, Self::Parallel(_));
        let children = match self {
            Self::Series(c) | Self::Parallel(c) => c,
            leaf => return leaf,
        };
        let mut result = Vec::new();
        for child in children.into_iter().map(Self::normalized) {
            match child {
                Self::Empty => (),
                Self::Series(c) if !parallel => result.extend(c),
                Self::Parallel(c) if parallel => result.extend(c),
                c => result.push(c),
            }
        }
        match result.len() {
            0 => Self::Empty,
            1 => result.pop().unwrap(),
            _ if parallel => Self::Parallel(result),
            _ => Self::Series(result),
        }
    }
    pub fn ids(&self) -> Vec<u64> {
        match self {
            Self::Empty => Vec::new(),
            Self::Plugin(id) => vec![*id],
            Self::Series(c) | Self::Parallel(c) => c.iter().flat_map(Self::ids).collect(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for id in self.ids() {
            if id == 0 || id >= u64::MAX / 2 {
                return Err("Reserved plugin ID".into());
            }
            if !seen.insert(id) {
                return Err("Duplicate plugin ID in routing".into());
            }
        }
        Ok(())
    }
    pub fn contains(&self, id: u64) -> bool {
        self.ids().contains(&id)
    }
    fn erase(&mut self, id: u64) {
        match self {
            Self::Plugin(candidate) if *candidate == id => *self = Self::Empty,
            Self::Series(c) | Self::Parallel(c) => {
                for child in c {
                    child.erase(id);
                }
            }
            _ => (),
        }
    }
    /// Keep placeholders until insertion: this preserves the target's fork
    /// scope even when lifting a sibling temporarily leaves one live branch.
    fn insert(&mut self, id: u64, target: u64, placement: Placement) -> bool {
        let group = matches!(placement, Placement::BeforeFork | Placement::AfterFork);
        if group && matches!(self, Self::Parallel(_)) && self.contains(target) {
            // The innermost enclosing fork owns its ring.
            let children = match self {
                Self::Parallel(c) => c,
                _ => unreachable!(),
            };
            let child = children.iter_mut().find(|c| c.contains(target)).unwrap();
            if child.has_fork(target) && child.insert(id, target, placement) {
                return true;
            }
            let old = std::mem::take(self);
            *self = if placement == Placement::BeforeFork {
                Self::Series(vec![Self::Plugin(id), old])
            } else {
                Self::Series(vec![old, Self::Plugin(id)])
            };
            return true;
        }
        match self {
            Self::Plugin(candidate) if *candidate == target => {
                let old = Self::Plugin(target);
                let new = Self::Plugin(id);
                *self = match placement {
                    Placement::Before | Placement::BeforeFork => Self::Series(vec![new, old]),
                    Placement::After | Placement::AfterFork => Self::Series(vec![old, new]),
                    Placement::Above => Self::Parallel(vec![new, old]),
                    Placement::Below => Self::Parallel(vec![old, new]),
                };
                true
            }
            Self::Series(c) | Self::Parallel(c) => {
                c.iter_mut().any(|c| c.insert(id, target, placement))
            }
            _ => false,
        }
    }
    fn has_fork(&self, target: u64) -> bool {
        match self {
            Self::Parallel(_) => self.contains(target),
            Self::Series(c) => c.iter().any(|c| c.has_fork(target)),
            _ => false,
        }
    }
    fn scope_path(&self, scope: usize) -> Option<Vec<usize>> {
        fn visit(
            node: &Flow,
            wanted: usize,
            index: &mut usize,
            path: &mut Vec<usize>,
        ) -> Option<Vec<usize>> {
            if matches!(node, Flow::Parallel(_)) {
                let at = *index;
                *index += 1;
                if at == wanted {
                    return Some(path.clone());
                }
            }
            if let Flow::Series(children) | Flow::Parallel(children) = node {
                for (i, child) in children.iter().enumerate() {
                    path.push(i);
                    if let Some(found) = visit(child, wanted, index, path) {
                        return Some(found);
                    }
                    path.pop();
                }
            }
            None
        }
        visit(self, scope, &mut 0, &mut Vec::new())
    }
    fn at_mut(&mut self, path: &[usize]) -> Option<&mut Flow> {
        if path.is_empty() {
            return Some(self);
        }
        match self {
            Flow::Series(c) | Flow::Parallel(c) => c.get_mut(path[0])?.at_mut(&path[1..]),
            _ => None,
        }
    }
    pub fn edit_scoped(&self, edit: Edit, scope: Option<usize>) -> Result<Self, String> {
        let Some(scope) = scope else {
            return self.edit(edit);
        };
        self.validate()?;
        let (id, target, placement, moving) = match edit {
            Edit::Move {
                id,
                target,
                placement,
            } => (id, target, placement, true),
            Edit::Insert {
                id,
                target,
                placement,
            } => (id, target, placement, false),
            _ => return Err("Only ring insertions have a group scope".into()),
        };
        if !matches!(placement, Placement::BeforeFork | Placement::AfterFork) {
            return Err("Unexpected group scope".into());
        }
        if moving != self.contains(id) {
            return Err("Invalid moved/inserted plugin".into());
        }
        let path = self
            .scope_path(scope)
            .ok_or("Drop group no longer exists")?;
        let mut result = self.clone();
        if !result
            .at_mut(&path)
            .is_some_and(|group| group.contains(target))
        {
            return Err("Drop target is outside its group".into());
        }
        if moving {
            result.erase(id);
        }
        let group = result.at_mut(&path).unwrap();
        let old = std::mem::take(group);
        *group = if placement == Placement::BeforeFork {
            Flow::Series(vec![Flow::Plugin(id), old])
        } else {
            Flow::Series(vec![old, Flow::Plugin(id)])
        };
        let result = result.normalized();
        result.validate()?;
        Ok(result)
    }
    pub fn edit(&self, edit: Edit) -> Result<Self, String> {
        self.validate()?;
        let mut result = self.clone();
        match edit {
            Edit::Move {
                id,
                target,
                placement,
            }
            | Edit::Insert {
                id,
                target,
                placement,
            } => {
                if !self.contains(target) {
                    return Err("Drop target no longer exists".into());
                }
                let moving = matches!(edit, Edit::Move { .. });
                if moving != self.contains(id) {
                    return Err(if moving {
                        "Dragged card no longer exists"
                    } else {
                        "Plugin ID already exists"
                    }
                    .into());
                }
                if moving && id == target {
                    return Ok(self.clone());
                }
                if moving {
                    result.erase(id);
                }
                if !result.insert(id, target, placement) {
                    return Err("Drop target no longer exists".into());
                }
            }
            Edit::Remove { id } => {
                if !self.contains(id) {
                    return Err("Removed card no longer exists".into());
                }
                result.erase(id);
            }
            Edit::Append { id } => {
                if self.contains(id) {
                    return Err("Plugin ID already exists".into());
                }
                result = Self::Series(vec![result, Self::Plugin(id)]);
            }
        }
        let result = result.normalized();
        result.validate()?;
        Ok(result)
    }
    /// Reconcile a document with its roster (remove deleted instances and append
    /// newly loaded ones). Never infer rewiring from card positions.
    pub fn roster(&self, ids: &[u64]) -> Result<Self, String> {
        self.validate()?;
        let mut result = self.clone();
        for id in self.ids() {
            if !ids.contains(&id) {
                result.erase(id);
            }
        }
        result = result.normalized();
        for id in ids {
            if !result.contains(*id) {
                result = result.edit(Edit::Append { id: *id })?;
            }
        }
        Ok(result)
    }
    pub fn width(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Plugin(_) => 1,
            Self::Series(c) => c.iter().map(Self::width).sum(),
            Self::Parallel(c) => c.iter().map(Self::width).max().unwrap_or(0),
        }
    }
    pub fn height(&self) -> usize {
        match self {
            Self::Empty => 1,
            Self::Plugin(_) => 1,
            Self::Series(c) => c.iter().map(Self::height).max().unwrap_or(1),
            Self::Parallel(c) => c.iter().map(Self::height).sum(),
        }
    }
    /// Compatibility projection for old files and UI metadata. The tree remains
    /// authoritative: overlapping column spans cannot encode every nested fork.
    pub fn lanes(&self) -> Vec<Vec<Lane>> {
        fn project(flow: &Flow, start: usize, end: usize, lanes: &mut Vec<Lane>) {
            match flow {
                Flow::Plugin(id) => lanes.push(Lane::spanning(start, end, vec![NodeId(*id)])),
                Flow::Series(c) if c.iter().all(|c| matches!(c, Flow::Plugin(_))) => {
                    lanes.push(Lane::spanning(
                        start,
                        end,
                        c.iter().flat_map(Flow::ids).map(NodeId).collect(),
                    ))
                }
                Flow::Series(c) => {
                    let mut x = start;
                    for child in c {
                        project(child, x, x + child.width(), lanes);
                        x += child.width();
                    }
                }
                Flow::Parallel(c) => {
                    for child in c {
                        project(child, start, end, lanes);
                    }
                }
                Flow::Empty => (),
            }
        }
        let mut stages = Vec::new();
        let children = match self {
            Self::Series(c) => c.as_slice(),
            _ => std::slice::from_ref(self),
        };
        for child in children {
            let mut lanes = Vec::new();
            project(child, 0, child.width(), &mut lanes);
            if !lanes.is_empty() {
                stages.push(lanes);
            }
        }
        stages
    }
    /// Import the old span format. Properly nested intervals have a unique
    /// structured reading; crossing intervals are refused rather than guessed.
    pub fn from_lanes(stages: &[Vec<Lane>]) -> Result<Self, String> {
        fn region(lanes: &[(usize, &Lane)], start: usize, end: usize) -> Result<Flow, String> {
            if lanes.is_empty() {
                return Ok(Flow::Empty);
            }
            let (full, rest): (Vec<_>, Vec<_>) = lanes
                .iter()
                .copied()
                .partition(|(_, l)| l.start == start && l.end == end);
            if !full.is_empty() {
                let mut children = full
                    .iter()
                    .map(|(i, l)| {
                        (
                            *i,
                            Flow::series(l.chain.iter().map(|n| Flow::Plugin(n.0)).collect()),
                        )
                    })
                    .collect::<Vec<_>>();
                if !rest.is_empty() {
                    children.push((rest[0].0, region(&rest, start, end)?));
                }
                children.sort_by_key(|(i, _)| *i);
                return Ok(Flow::parallel(
                    children.into_iter().map(|(_, c)| c).collect(),
                ));
            }
            let mut x = start;
            let mut children = Vec::new();
            while x < end {
                let edge = lanes
                    .iter()
                    .filter(|(_, l)| l.start == x)
                    .map(|(_, l)| l.end)
                    .max()
                    .ok_or("Legacy routing has an unconnected span")?;
                if edge > end
                    || lanes
                        .iter()
                        .any(|(_, l)| l.start < x && l.end > x || l.start < edge && l.end > edge)
                {
                    return Err("Legacy routing has crossing spans; restore a structured layout before editing".into());
                }
                let part = lanes
                    .iter()
                    .copied()
                    .filter(|(_, l)| l.start >= x && l.end <= edge)
                    .collect::<Vec<_>>();
                children.push(region(&part, x, edge)?);
                x = edge;
            }
            Ok(Flow::series(children))
        }
        let mut result = Vec::new();
        for stage in stages {
            let width = stage.iter().map(|l| l.end).max().unwrap_or(0);
            if stage
                .iter()
                .any(|l| l.chain.is_empty() || l.start >= l.end || l.start + l.chain.len() > l.end)
            {
                return Err("Invalid legacy lane".into());
            }
            result.push(region(
                &stage.iter().enumerate().collect::<Vec<_>>(),
                0,
                width,
            )?);
        }
        let result = Self::series(result);
        result.validate()?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn n(id: u64) -> Flow {
        Flow::Plugin(id)
    }
    fn s(children: Vec<Flow>) -> Flow {
        Flow::series(children)
    }
    fn p(children: Vec<Flow>) -> Flow {
        Flow::parallel(children)
    }
    fn mv(root: &Flow, id: u64, target: u64, placement: Placement) -> Flow {
        root.edit(Edit::Move {
            id,
            target,
            placement,
        })
        .unwrap()
    }
    #[test]
    fn the_entire_agreed_drag_session() {
        use Placement::*;
        // 1 Tuner, 2 Dist, 3 Echo, 4 Ecomplex, 5 Wah.
        let root = s(vec![n(1), p(vec![s(vec![n(2), n(4)]), n(3)])]);
        let root = mv(&root, 3, 2, After);
        assert_eq!(root, s(vec![n(1), n(2), n(3), n(4)]));
        let root = mv(&root, 2, 4, Below);
        assert_eq!(root, s(vec![n(1), n(3), p(vec![n(4), n(2)])]));
        let root = mv(&root, 1, 2, Before);
        assert_eq!(root, s(vec![n(3), p(vec![n(4), s(vec![n(1), n(2)])])]));
        let root = mv(&root, 2, 4, After);
        assert_eq!(root, s(vec![n(3), p(vec![s(vec![n(4), n(2)]), n(1)])]));
        let root = root
            .edit(Edit::Insert {
                id: 5,
                target: 1,
                placement: Before,
            })
            .unwrap();
        assert_eq!(
            root,
            s(vec![
                n(3),
                p(vec![s(vec![n(4), n(2)]), s(vec![n(5), n(1)])])
            ])
        );
        let root = mv(&root, 5, 2, Below);
        assert_eq!(
            root,
            s(vec![
                n(3),
                p(vec![s(vec![n(4), p(vec![n(2), n(5)])]), n(1)])
            ])
        );
        let root = mv(&root, 3, 4, Above);
        assert_eq!(
            root,
            p(vec![
                s(vec![p(vec![n(3), n(4)]), p(vec![n(2), n(5)])]),
                n(1)
            ])
        );
        let root = mv(&root, 2, 3, After);
        assert_eq!(
            root,
            p(vec![
                s(vec![p(vec![s(vec![n(3), n(2)]), n(4)]), n(5)]),
                n(1)
            ])
        );
        let root = mv(&root, 5, 2, After);
        assert_eq!(root, p(vec![s(vec![n(3), n(2), n(5)]), n(4), n(1)]));
        assert_eq!(root.ids(), vec![3, 2, 5, 4, 1]);
        assert_eq!(root.clone().normalized(), root);
    }
    #[test]
    fn local_parallelism_does_not_touch_the_other_branch_at_the_same_column() {
        let root = p(vec![
            s(vec![n(1), n(2), n(3)]),
            s(vec![n(4), n(5), n(6)]),
            n(7),
        ]);
        let edited = mv(&root, 7, 2, Placement::Below);
        assert_eq!(
            edited,
            p(vec![
                s(vec![n(1), p(vec![n(2), n(7)]), n(3)]),
                s(vec![n(4), n(5), n(6)])
            ])
        );
    }
    #[test]
    fn detach_closes_the_old_position_and_empty_forks_disappear() {
        let root = s(vec![n(1), p(vec![n(2), s(vec![n(3), n(4)])]), n(5)]);
        assert_eq!(
            mv(&root, 2, 1, Placement::Before),
            s(vec![n(2), n(1), n(3), n(4), n(5)])
        );
        assert_eq!(
            root.edit(Edit::Remove { id: 3 }).unwrap(),
            s(vec![n(1), p(vec![n(2), n(4)]), n(5)])
        );
        assert_eq!(n(1).edit(Edit::Remove { id: 1 }).unwrap(), Flow::Empty);
    }
    #[test]
    fn fork_ring_operations_keep_the_original_scope_while_lifting_a_sibling() {
        let root = s(vec![
            n(1),
            p(vec![n(2), s(vec![n(3), p(vec![n(4), n(5)])])]),
            n(6),
        ]);
        assert_eq!(
            mv(&root, 2, 3, Placement::BeforeFork),
            s(vec![n(1), n(2), n(3), p(vec![n(4), n(5)]), n(6)])
        );
        assert_eq!(
            mv(&root, 4, 5, Placement::AfterFork),
            s(vec![n(1), p(vec![n(2), s(vec![n(3), n(5), n(4)])]), n(6)])
        );
        assert_eq!(
            mv(&root, 6, 4, Placement::BeforeFork),
            s(vec![
                n(1),
                p(vec![n(2), s(vec![n(3), n(6), p(vec![n(4), n(5)])])])
            ])
        );
    }
    #[test]
    fn errors_are_transactional_and_self_drops_are_noops() {
        let root = s(vec![n(1), n(2)]);
        let copy = root.clone();
        for op in [
            Edit::Move {
                id: 9,
                target: 1,
                placement: Placement::Before,
            },
            Edit::Move {
                id: 1,
                target: 9,
                placement: Placement::Before,
            },
            Edit::Remove { id: 9 },
            Edit::Insert {
                id: 1,
                target: 2,
                placement: Placement::After,
            },
            Edit::Append { id: 0 },
        ] {
            assert!(root.edit(op).is_err());
            assert_eq!(root, copy);
        }
        for placement in [
            Placement::Before,
            Placement::After,
            Placement::Above,
            Placement::Below,
            Placement::BeforeFork,
            Placement::AfterFork,
        ] {
            assert_eq!(mv(&root, 1, 1, placement), root);
        }
        assert!(
            Flow::Series(vec![n(1), n(1)])
                .edit(Edit::Remove { id: 1 })
                .is_err()
        );
    }
    #[test]
    fn canonicalization_folds_only_equivalent_junctions() {
        assert_eq!(
            Flow::Parallel(vec![n(1), Flow::Parallel(vec![n(2), n(3)])]).normalized(),
            p(vec![n(1), n(2), n(3)])
        );
        // Two serial parallel groups must NOT distribute: that duplicates plugins.
        let root = s(vec![p(vec![n(1), n(2)]), p(vec![n(3), n(4)])]);
        assert_eq!(root.clone().normalized(), root);
        assert_eq!(
            Flow::Series(vec![Flow::Empty, Flow::Series(vec![n(1)])]).normalized(),
            n(1)
        );
    }
    #[test]
    fn adjacent_moves_preserve_their_expected_order() {
        let root = s(vec![n(1), n(2), n(3)]);
        assert_eq!(mv(&root, 1, 2, Placement::Before), root);
        assert_eq!(mv(&root, 2, 1, Placement::After), root);
        assert_eq!(mv(&root, 1, 3, Placement::After), s(vec![n(2), n(3), n(1)]));
        assert_eq!(
            mv(&root, 3, 1, Placement::Before),
            s(vec![n(3), n(1), n(2)])
        );
    }
    #[test]
    fn legacy_full_span_and_local_forks_import_and_project_without_loss() {
        let stages = vec![
            vec![Lane::one(NodeId(1))],
            super::super::legacy_lanes(vec![vec![NodeId(2), NodeId(3)], vec![NodeId(4)]]),
        ];
        let root = Flow::from_lanes(&stages).unwrap();
        assert_eq!(root, s(vec![n(1), p(vec![s(vec![n(2), n(3)]), n(4)])]));
        assert_eq!(Flow::from_lanes(&root.lanes()).unwrap(), root);
        let root = p(vec![s(vec![n(1), p(vec![n(2), n(3)])]), n(4)]);
        assert_eq!(Flow::from_lanes(&root.lanes()).unwrap(), root);
        assert!(
            Flow::from_lanes(&[vec![
                Lane::spanning(0, 2, vec![NodeId(1)]),
                Lane::spanning(1, 3, vec![NodeId(2)])
            ]])
            .is_err()
        );
    }
    #[test]
    fn every_move_in_small_nested_graphs_preserves_ids_and_normal_form() {
        let graphs = [
            s(vec![n(1), n(2), n(3), n(4)]),
            p(vec![s(vec![n(1), n(2)]), n(3), n(4)]),
            s(vec![n(1), p(vec![n(2), n(3)]), n(4)]),
            p(vec![s(vec![n(1), p(vec![n(2), n(3)])]), n(4)]),
        ];
        for root in graphs {
            for id in 1..=4 {
                for target in 1..=4 {
                    for placement in [
                        Placement::Before,
                        Placement::After,
                        Placement::Above,
                        Placement::Below,
                        Placement::BeforeFork,
                        Placement::AfterFork,
                    ] {
                        let edited = mv(&root, id, target, placement);
                        edited.validate().unwrap();
                        let mut ids = edited.ids();
                        ids.sort();
                        assert_eq!(ids, vec![1, 2, 3, 4]);
                        assert_eq!(edited.clone().normalized(), edited);
                    }
                }
            }
        }
    }
    #[test]
    fn external_insert_append_and_roster_reconciliation_use_the_same_rules() {
        let root = Flow::Empty.edit(Edit::Append { id: 1 }).unwrap();
        let root = root
            .edit(Edit::Insert {
                id: 2,
                target: 1,
                placement: Placement::Above,
            })
            .unwrap();
        assert_eq!(root, p(vec![n(2), n(1)]));
        assert_eq!(root.roster(&[1, 3]).unwrap(), s(vec![n(1), n(3)]));
    }
}

#[cfg(test)]
mod scoped_errors {
    use super::*;
    #[test]
    fn stale_or_unrelated_ring_scopes_cannot_modify_the_document() {
        let root = Flow::series(vec![
            Flow::parallel(vec![Flow::Plugin(1), Flow::Plugin(2)]),
            Flow::parallel(vec![Flow::Plugin(3), Flow::Plugin(4)]),
        ]);
        let edit = Edit::Move {
            id: 4,
            target: 1,
            placement: Placement::BeforeFork,
        };
        assert!(root.edit_scoped(edit, Some(99)).is_err());
        assert!(root.edit_scoped(edit, Some(1)).is_err());
        assert!(root.edit_scoped(Edit::Remove { id: 1 }, Some(0)).is_err());
        assert_eq!(root.ids(), vec![1, 2, 3, 4]);
        assert_eq!(root.clone().normalized(), root);
    }
}
