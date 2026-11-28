use nice_clap_host::Binding;
use nice_rack_engine::{Channels, Connection, NodeId, NodeKind, RackGraph};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Mode {
    InProcess,
    Isolated,
    GuiIsolated,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Topology {
    Serial,
    Parallel,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub plugins: Vec<PathBuf>,
    pub mode: Mode,
    pub topology: Topology,
    pub frames: usize,
    pub rate: f64,
    pub seconds: f64,
    pub warmup: f64,
    pub editors: bool,
    #[serde(default)]
    pub editor_replicas: bool,
    pub trial: usize,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Distribution {
    pub count: usize,
    pub median_us: f64,
    pub p95_us: f64,
    pub p99_us: f64,
    pub max_us: f64,
}
impl Distribution {
    pub fn new(mut samples: Vec<f64>) -> Self {
        samples.sort_by(f64::total_cmp);
        if samples.is_empty() {
            return Self::default();
        }
        let at = |q: f64| {
            samples[((samples.len() as f64 * q).ceil() as usize)
                .saturating_sub(1)
                .min(samples.len() - 1)]
        };
        Self {
            count: samples.len(),
            median_us: at(0.5),
            p95_us: at(0.95),
            p99_us: at(0.99),
            max_us: *samples.last().unwrap(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AudioResult {
    pub successful: Distribution,
    pub wake_lateness: Distribution,
    pub warmup_completed: usize,
    pub attempted: usize,
    pub deadline_misses: usize,
    pub warmup_deadline_misses: usize,
    pub failed_call_us: Option<f64>,
    pub failed_during_warmup: bool,
    pub checksum: f64,
    pub error: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ResultRow {
    pub job: Job,
    pub classes: Vec<String>,
    pub setup_ms: Option<f64>,
    pub editor_open_ms: Option<f64>,
    pub heartbeat_lateness: Distribution,
    pub heartbeat_gap: Distribution,
    pub missed_heartbeats: usize,
    pub audio: Option<AudioResult>,
    pub error: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub struct Report {
    pub architecture: String,
    pub os: String,
    pub machine: String,
    pub rustc: String,
    pub compile_flags: String,
    pub rows: Vec<ResultRow>,
    pub comparisons: Vec<Comparison>,
}

/// Same explicit width conversions in both modes. A parallel graph has one
/// plugin per lane; a serial graph uses the supplied plugin order.
pub fn graph(job: &Job, bindings: &BTreeMap<NodeId, Binding>) -> Result<RackGraph, String> {
    let ports = bindings
        .iter()
        .map(|(&id, binding)| {
            let (input, output) = binding.entry.main_channels(&binding.class)?;
            Ok((id, input, output))
        })
        .collect::<Result<Vec<_>, String>>()?;
    graph_ports(job.topology, &ports)
}
fn graph_ports(
    topology: Topology,
    ports: &[(NodeId, Channels, Channels)],
) -> Result<RackGraph, String> {
    let mut graph = RackGraph::default();
    let mut next = 1000;
    let input = NodeId(0);
    graph.nodes.insert(input, NodeKind::Input(Channels::Stereo));
    let mut previous = (input, Channels::Stereo);
    let mut tails = Vec::new();
    for &(id, inputs, outputs) in ports {
        graph.nodes.insert(id, NodeKind::Plugin { inputs, outputs });
        let source = if matches!(topology, Topology::Serial) {
            previous
        } else {
            (input, Channels::Stereo)
        };
        connect(&mut graph, &mut next, source, (id, inputs));
        previous = (id, outputs);
        tails.push(previous);
    }
    let output = NodeId(999);
    graph
        .nodes
        .insert(output, NodeKind::Output(Channels::Stereo));
    if matches!(topology, Topology::Parallel) {
        let mixer = NodeId(998);
        graph.nodes.insert(mixer, NodeKind::Mixer(Channels::Stereo));
        for tail in tails {
            connect(&mut graph, &mut next, tail, (mixer, Channels::Stereo));
        }
        graph.connections.push(Connection {
            from: mixer,
            to: output,
        });
    } else {
        connect(&mut graph, &mut next, previous, (output, Channels::Stereo));
    }
    graph.compile().map_err(|e| e.to_string())?;
    Ok(graph)
}
fn connect(
    graph: &mut RackGraph,
    next: &mut u64,
    from: (NodeId, Channels),
    to: (NodeId, Channels),
) {
    let source = if from.1 != to.1 {
        let map = NodeId(*next);
        *next += 1;
        graph.nodes.insert(
            map,
            NodeKind::ChannelMap {
                inputs: from.1,
                outputs: to.1,
            },
        );
        graph.connections.push(Connection {
            from: from.0,
            to: map,
        });
        map
    } else {
        from.0
    };
    graph.connections.push(Connection {
        from: source,
        to: to.0,
    });
}
pub fn period(job: &Job) -> Duration {
    Duration::from_secs_f64(job.frames as f64 / job.rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn percentiles_use_nearest_rank_and_empty_is_explicit() {
        assert_eq!(Distribution::new(vec![]).count, 0);
        let d = Distribution::new((1..=100).rev().map(f64::from).collect());
        assert_eq!(
            (d.median_us, d.p95_us, d.p99_us, d.max_us),
            (50., 95., 99., 100.)
        );
        assert_eq!(Distribution::new(vec![3.]).p99_us, 3.);
    }
    #[test]
    fn width_changes_are_explicit_and_compile() {
        let mut graph = RackGraph::default();
        graph
            .nodes
            .insert(NodeId(0), NodeKind::Input(Channels::Stereo));
        graph
            .nodes
            .insert(NodeId(1), NodeKind::Output(Channels::Mono));
        connect(
            &mut graph,
            &mut 100,
            (NodeId(0), Channels::Stereo),
            (NodeId(1), Channels::Mono),
        );
        assert!(graph.compile().is_ok());
        assert_eq!(graph.nodes.len(), 3);
    }
}

#[derive(Serialize, Deserialize)]
pub struct Comparison {
    pub mode: Mode,
    pub frames: usize,
    pub topology: Topology,
    pub trial: usize,
    pub dsp_median_ratio: Option<f64>,
    pub dsp_p99_ratio: Option<f64>,
    pub host_p99_delta_ms: Option<f64>,
    pub unavailable_reason: Option<String>,
}
pub fn compare(a: &ResultRow, b: &ResultRow) -> Comparison {
    let (native, isolated) = if matches!(a.job.mode, Mode::InProcess) {
        (a, b)
    } else {
        (b, a)
    };
    let mut result = Comparison {
        mode: isolated.job.mode,
        frames: a.job.frames,
        topology: a.job.topology,
        trial: a.job.trial,
        dsp_median_ratio: None,
        dsp_p99_ratio: None,
        host_p99_delta_ms: None,
        unavailable_reason: None,
    };
    let enough = |row: &ResultRow| {
        row.error.is_none()
            && row.audio.as_ref().is_some_and(|audio| {
                audio.error.is_none()
                    && audio.successful.count >= 20
                    && audio.successful.count as f64
                        >= 0.95 * (row.job.seconds * row.job.rate / row.job.frames as f64).floor()
            })
    };
    if native.classes != isolated.classes
        || native.job.editor_replicas != isolated.job.editor_replicas
        || !enough(native)
        || !enough(isolated)
    {
        result.unavailable_reason = Some(
            "Failed/censored trial or insufficient samples/coverage; no speedup claimed".into(),
        );
        return result;
    }
    let n = &native.audio.as_ref().unwrap().successful;
    let i = &isolated.audio.as_ref().unwrap().successful;
    if n.median_us > 0. {
        result.dsp_median_ratio = Some(i.median_us / n.median_us);
    }
    if n.p99_us > 0. {
        result.dsp_p99_ratio = Some(i.p99_us / n.p99_us);
    }
    if native.heartbeat_lateness.count > 0 && isolated.heartbeat_lateness.count > 0 {
        result.host_p99_delta_ms =
            Some((isolated.heartbeat_lateness.p99_us - native.heartbeat_lateness.p99_us) / 1000.);
    }
    result
}

#[cfg(test)]
mod comparison_tests {
    use super::*;
    fn row(mode: Mode) -> ResultRow {
        let job = Job {
            plugins: vec!["fixture.clap".into()],
            mode,
            topology: Topology::Serial,
            frames: 64,
            rate: 48000.,
            seconds: 1.,
            warmup: 0.,
            editors: false,
            editor_replicas: false,
            trial: 1,
        };
        ResultRow {
            job,
            classes: vec!["fixture".into()],
            setup_ms: Some(1.),
            editor_open_ms: None,
            heartbeat_lateness: Distribution::new(vec![1.; 60]),
            heartbeat_gap: Distribution::default(),
            missed_heartbeats: 0,
            audio: Some(AudioResult {
                successful: Distribution::new(vec![10.; 750]),
                ..AudioResult::default()
            }),
            error: None,
        }
    }
    #[test]
    fn live_editors_and_inactive_replicas_are_not_comparable() {
        let mut native = row(Mode::InProcess);
        let mut gui = row(Mode::GuiIsolated);
        gui.job.editor_replicas = true;
        assert!(compare(&native, &gui).dsp_median_ratio.is_none());
        native.job.editor_replicas = true;
        let pair = compare(&native, &gui);
        assert!(matches!(pair.mode, Mode::GuiIsolated));
        assert_eq!(pair.dsp_median_ratio, Some(1.));
    }
    #[test]
    fn aborted_and_low_coverage_runs_never_claim_a_speedup() {
        let native = row(Mode::InProcess);
        let mut isolated = row(Mode::Isolated);
        isolated.audio.as_mut().unwrap().error = Some("deadline exceeded".into());
        assert!(compare(&native, &isolated).dsp_median_ratio.is_none());
        isolated.audio.as_mut().unwrap().error = None;
        isolated.audio.as_mut().unwrap().successful = Distribution::new(vec![1.; 30]);
        assert!(compare(&native, &isolated).unavailable_reason.is_some());
    }
    #[test]
    fn paired_ratios_are_mode_order_independent_and_ron_round_trips() {
        let native = row(Mode::InProcess);
        let mut isolated = row(Mode::Isolated);
        isolated.audio.as_mut().unwrap().successful = Distribution::new(vec![20.; 750]);
        let pair = compare(&isolated, &native);
        assert_eq!(pair.dsp_median_ratio, Some(2.));
        assert_eq!(pair.dsp_p99_ratio, Some(2.));
        let encoded = ron::to_string(&pair).unwrap();
        let decoded: Comparison = ron::from_str(&encoded).unwrap();
        assert_eq!(decoded.dsp_median_ratio, Some(2.));
        isolated.classes[0] = "other".into();
        assert!(compare(&native, &isolated).dsp_median_ratio.is_none());
    }
    #[test]
    fn serial_and_parallel_workloads_compile_with_width_adapters() {
        let ports = [
            (NodeId(1), Channels::Mono, Channels::Mono),
            (NodeId(2), Channels::Stereo, Channels::Mono),
        ];
        for topology in [Topology::Serial, Topology::Parallel] {
            let graph = graph_ports(topology, &ports).unwrap();
            assert_eq!(
                graph
                    .nodes
                    .values()
                    .filter(|node| matches!(node, NodeKind::Plugin { .. }))
                    .count(),
                2
            );
            assert_eq!(
                graph
                    .nodes
                    .values()
                    .filter(|node| matches!(node, NodeKind::Mixer(_)))
                    .count(),
                usize::from(matches!(topology, Topology::Parallel))
            );
            assert!(graph.compile().is_ok());
        }
    }
}

#[cfg(test)]
mod gui_tests {
    use super::*;
    #[test]
    fn gui_only_workload_identity_survives_report_roundtrip() {
        let job = Job {
            plugins: vec!["fixture.clap".into()],
            mode: Mode::GuiIsolated,
            topology: Topology::Serial,
            frames: 64,
            rate: 48000.,
            seconds: 1.,
            warmup: 0.,
            editors: true,
            editor_replicas: true,
            trial: 1,
        };
        let round: Job = ron::from_str(&ron::to_string(&job).unwrap()).unwrap();
        assert!(matches!(round.mode, Mode::GuiIsolated));
        assert!(round.editor_replicas);
    }
}
