//! Safe routing validation and prepared audio graph processing for nice-rack.
#![deny(unsafe_code)]

pub mod audio;

use std::collections::{BTreeMap, BTreeSet};

/// Stable identity within a rack document, independent of processing order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channels {
    Mono,
    Stereo,
}

/// Each node initially has one main bus per direction. Fan-out is expressed by
/// multiple outgoing connections; fan-in requires an explicit mixer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Input(Channels),
    Output(Channels),
    Plugin {
        inputs: Channels,
        outputs: Channels,
    },
    Mixer(Channels),
    /// Explicit mono/stereo conversion; never inserted implicitly.
    ChannelMap {
        inputs: Channels,
        outputs: Channels,
    },
}
impl NodeKind {
    fn inputs(&self) -> Option<Channels> {
        match *self {
            Self::Input(_) => None,
            Self::Output(channels) | Self::Mixer(channels) => Some(channels),
            Self::Plugin { inputs, .. } | Self::ChannelMap { inputs, .. } => Some(inputs),
        }
    }
    fn outputs(&self) -> Option<Channels> {
        match *self {
            Self::Output(_) => None,
            Self::Input(channels) | Self::Mixer(channels) => Some(channels),
            Self::Plugin { outputs, .. } | Self::ChannelMap { outputs, .. } => Some(outputs),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Connection {
    pub from: NodeId,
    pub to: NodeId,
}

#[derive(Clone, Debug, Default)]
pub struct RackGraph {
    pub nodes: BTreeMap<NodeId, NodeKind>,
    pub connections: Vec<Connection>,
    /// The order the rack is *read* in, so the plan matches what the operator
    /// sees: among nodes whose inputs are all done, the lowest priority runs
    /// first. GUI racks rank nodes by layout reading order — stage by stage,
    /// top lane down, chain head to tail — so a lower lane starts its block
    /// work where the lane above it ended. A node with no entry is ranked by
    /// its NodeId, which is the whole-rack behaviour before lanes existed.
    pub priority: BTreeMap<NodeId, u64>,
}

/// A validated routing order, not an activated audio plan. Plugin instances,
/// buffer allocation and latency compensation belong to a later preparation step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutingPlan {
    pub order: Vec<NodeId>,
    pub connections: Vec<Connection>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    MissingNode(NodeId),
    DuplicateConnection(Connection),
    NoOutputBus(NodeId),
    NoInputBus(NodeId),
    ChannelMismatch(Connection),
    MixerRequired(NodeId),
    UnconnectedInput(NodeId),
    Feedback,
}
impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Invalid rack routing: {self:?}")
    }
}
impl std::error::Error for GraphError {}

impl RackGraph {
    /// Validate outside the audio callback. Ordering is deterministic even when
    /// independent branches or connection insertion order change: ready nodes
    /// run in `priority` order, falling back to NodeId for unranked graphs.
    pub fn compile(&self) -> Result<RoutingPlan, GraphError> {
        let mut incoming: BTreeMap<_, usize> = self.nodes.keys().map(|&id| (id, 0)).collect();
        let mut outgoing: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
        let mut unique = BTreeSet::new();
        for &connection in &self.connections {
            let from = self
                .nodes
                .get(&connection.from)
                .ok_or(GraphError::MissingNode(connection.from))?;
            let to = self
                .nodes
                .get(&connection.to)
                .ok_or(GraphError::MissingNode(connection.to))?;
            if !unique.insert(connection) {
                return Err(GraphError::DuplicateConnection(connection));
            }
            let outputs = from
                .outputs()
                .ok_or(GraphError::NoOutputBus(connection.from))?;
            let inputs = to.inputs().ok_or(GraphError::NoInputBus(connection.to))?;
            if outputs != inputs {
                return Err(GraphError::ChannelMismatch(connection));
            }
            *incoming.get_mut(&connection.to).unwrap() += 1;
            outgoing
                .entry(connection.from)
                .or_default()
                .push(connection.to);
        }
        for (&id, kind) in &self.nodes {
            match incoming[&id] {
                0 if kind.inputs().is_some() => return Err(GraphError::UnconnectedInput(id)),
                n if n > 1 && !matches!(kind, NodeKind::Mixer(_)) => {
                    return Err(GraphError::MixerRequired(id));
                }
                _ => {}
            }
        }
        let rank = |id: NodeId| self.priority.get(&id).copied().unwrap_or(id.0);
        let mut ready: BTreeSet<(u64, NodeId)> = incoming
            .iter()
            .filter_map(|(&id, &count)| (count == 0).then_some((rank(id), id)))
            .collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some((_, id)) = ready.pop_first() {
            order.push(id);
            for &destination in outgoing.get(&id).into_iter().flatten() {
                let remaining = incoming.get_mut(&destination).unwrap();
                *remaining -= 1;
                if *remaining == 0 {
                    ready.insert((rank(destination), destination));
                }
            }
        }
        if order.len() != self.nodes.len() {
            return Err(GraphError::Feedback);
        }
        Ok(RoutingPlan {
            order,
            connections: unique.into_iter().collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn graph() -> RackGraph {
        RackGraph {
            nodes: [
                (NodeId(1), NodeKind::Input(Channels::Mono)),
                (
                    NodeId(2),
                    NodeKind::Plugin {
                        inputs: Channels::Mono,
                        outputs: Channels::Mono,
                    },
                ),
                (
                    NodeId(3),
                    NodeKind::Plugin {
                        inputs: Channels::Mono,
                        outputs: Channels::Mono,
                    },
                ),
                (NodeId(4), NodeKind::Mixer(Channels::Mono)),
                (NodeId(5), NodeKind::Output(Channels::Mono)),
            ]
            .into(),
            connections: [(1, 2), (1, 3), (2, 4), (3, 4), (4, 5)]
                .map(|(from, to)| Connection {
                    from: NodeId(from),
                    to: NodeId(to),
                })
                .into(),
            ..RackGraph::default()
        }
    }
    #[test]
    fn parallel_branches_finish_before_the_mixer_and_output() {
        let mut graph = graph();
        let expected = graph.compile().unwrap();
        assert_eq!(expected.order, [1, 2, 3, 4, 5].map(NodeId));
        graph.connections.reverse();
        assert_eq!(graph.compile().unwrap(), expected);
    }
    #[test]
    fn a_lane_finishes_before_the_lane_below_it_starts() {
        // The rack as laid out: Input feeds a top chain 1 -> 3 and a bottom
        // card 2, both ending in a mixer before the output. The reading order
        // ranks the bottom card after the whole top chain, so it runs last
        // even though its NodeId sits between the chain's two ids; with no
        // ranks at all the old NodeId tie-break interleaves the lanes instead.
        let mut graph = RackGraph {
            nodes: [
                (NodeId(0), NodeKind::Input(Channels::Stereo)),
                (
                    NodeId(1),
                    NodeKind::Plugin {
                        inputs: Channels::Stereo,
                        outputs: Channels::Stereo,
                    },
                ),
                (
                    NodeId(2),
                    NodeKind::Plugin {
                        inputs: Channels::Stereo,
                        outputs: Channels::Stereo,
                    },
                ),
                (
                    NodeId(3),
                    NodeKind::Plugin {
                        inputs: Channels::Stereo,
                        outputs: Channels::Stereo,
                    },
                ),
                (NodeId(4), NodeKind::Mixer(Channels::Stereo)),
                (NodeId(5), NodeKind::Output(Channels::Stereo)),
            ]
            .into(),
            connections: [(0, 1), (1, 3), (0, 2), (3, 4), (2, 4), (4, 5)]
                .map(|(from, to)| Connection {
                    from: NodeId(from),
                    to: NodeId(to),
                })
                .into(),
            priority: [(0, 0), (1, 1), (3, 2), (2, 3), (4, 4), (5, 5)]
                .map(|(id, rank)| (NodeId(id), rank))
                .into(),
        };
        assert_eq!(
            graph.compile().unwrap().order,
            [0, 1, 3, 2, 4, 5].map(NodeId)
        );
        graph.priority.clear();
        assert_eq!(
            graph.compile().unwrap().order,
            [0, 1, 2, 3, 4, 5].map(NodeId)
        );
    }
    #[test]
    fn serial_chain_and_explicit_channel_map_are_supported() {
        let mut graph = graph();
        graph.nodes.remove(&NodeId(3));
        graph.nodes.insert(
            NodeId(4),
            NodeKind::ChannelMap {
                inputs: Channels::Mono,
                outputs: Channels::Stereo,
            },
        );
        graph
            .nodes
            .insert(NodeId(5), NodeKind::Output(Channels::Stereo));
        graph
            .connections
            .retain(|edge| edge.from != NodeId(3) && edge.to != NodeId(3));
        assert_eq!(graph.compile().unwrap().order, [1, 2, 4, 5].map(NodeId));
    }
    #[test]
    fn feedback_is_rejected_even_when_a_cycle_has_an_input_feed() {
        let mut graph = graph();
        graph
            .nodes
            .insert(NodeId(2), NodeKind::Mixer(Channels::Mono));
        graph.connections.push(Connection {
            from: NodeId(4),
            to: NodeId(2),
        });
        assert_eq!(graph.compile(), Err(GraphError::Feedback));
    }
    #[test]
    fn invalid_connections_are_reported_instead_of_silently_adapted() {
        let mut graph = graph();
        graph
            .nodes
            .insert(NodeId(5), NodeKind::Output(Channels::Stereo));
        assert!(matches!(
            graph.compile(),
            Err(GraphError::ChannelMismatch(_))
        ));
        graph.nodes.remove(&NodeId(5));
        assert_eq!(graph.compile(), Err(GraphError::MissingNode(NodeId(5))));
    }
    #[test]
    fn fan_in_requires_a_mixer_and_duplicate_edges_are_rejected() {
        let mut graph = graph();
        graph.connections.push(graph.connections[0]);
        assert!(matches!(
            graph.compile(),
            Err(GraphError::DuplicateConnection(_))
        ));
        graph.connections.pop();
        graph.nodes.insert(
            NodeId(4),
            NodeKind::Plugin {
                inputs: Channels::Mono,
                outputs: Channels::Mono,
            },
        );
        assert_eq!(graph.compile(), Err(GraphError::MixerRequired(NodeId(4))));
        graph.connections.retain(|edge| edge.to != NodeId(2));
        assert_eq!(
            graph.compile(),
            Err(GraphError::UnconnectedInput(NodeId(2)))
        );
    }
}
