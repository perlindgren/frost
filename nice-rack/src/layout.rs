//! Structural rack editing. Layout coordinates are a projection, never wiring.
mod flow;
use crate::routing::NodeId;
pub use flow::{Edit, Flow, Placement};

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
    /// Preorder index of the exact Parallel group whose ring was targeted.
    pub scope: Option<usize>,
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
/// Compatibility adapter for layout clients; edits are performed on the tree.
pub fn place(
    stages: &[Vec<Lane>],
    moving: NodeId,
    target: Target,
) -> Result<Vec<Vec<Lane>>, String> {
    let flow = Flow::from_lanes(stages)?;
    flow.edit_scoped(
        Edit::Move {
            id: moving.0,
            target: target.card.0,
            placement: target.side.into(),
        },
        target.scope,
    )
    .map(|flow| flow.lanes())
}
impl From<Side> for Placement {
    fn from(side: Side) -> Self {
        match side {
            Side::Left => Self::Before,
            Side::Right => Self::After,
            Side::Above | Side::Over => Self::Above,
            Side::Below | Side::Under => Self::Below,
            Side::Split => Self::BeforeFork,
            Side::Join => Self::AfterFork,
        }
    }
}
