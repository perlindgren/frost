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
    /// independent branches or connection insertion order change.
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
        let mut ready: BTreeSet<_> = incoming
            .iter()
            .filter_map(|(&id, &count)| (count == 0).then_some(id))
            .collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(id) = ready.pop_first() {
            order.push(id);
            for &destination in outgoing.get(&id).into_iter().flatten() {
                let remaining = incoming.get_mut(&destination).unwrap();
                *remaining -= 1;
                if *remaining == 0 {
                    ready.insert(destination);
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
