//! Main-thread lifecycle controller. Device transport and DSP are separate owners.
use crate::{
    Settings,
    routing::{Channels, Connection, NodeId, NodeKind, RackGraph},
};
use crossbeam::channel::{Receiver, Sender, bounded};
use nice_clap_host::{Binding, Entry, LiveAudio, LiveSession, LiveStatus};
use nice_common::OutputChannels;
use nice_common::audio_io::{DeviceStreams, Endpoints};
use nice_common::true_peak::TruePeak;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const BLOCK: usize = 1024;
#[derive(Clone)]
pub struct Card {
    pub id: NodeId,
    pub stage: u64,
    /// Which branch of its stage this card chains along, keyed by the branch's
    /// first card. `None` means the card stands on its own branch — the shape
    /// every card had before chains existed, so a saved rack that never names
    /// a branch still lays out and wires exactly as it did.
    pub branch: Option<u64>,
    /// The columns this card's lane runs between, as `(start, end)`: the
    /// split the lane leaves and the join it returns to. `None` is the shape
    /// every lane had before local forks meant — the card runs the whole
    /// stage, from its left edge to its right — so a saved rack that never
    /// names a span still lays out and wires exactly as it did.
    pub span: Option<(u64, u64)>,
    pub name: String,
    pub details: nice_clap_host::Descriptor,
    pub path: PathBuf,
    pub binding: Binding,
    pub inputs: Channels,
    pub outputs: Channels,
}
impl Card {
    pub fn new(
        id: NodeId,
        path: PathBuf,
        entry: Entry,
        class: String,
        name: String,
    ) -> Result<Self, String> {
        let details = entry
            .descriptors()?
            .into_iter()
            .find(|d| d.id == class)
            .ok_or("Plugin class not found")?;
        let (inputs, outputs) = entry.main_channels(&class)?;
        Ok(Self {
            id,
            stage: id.0,
            branch: None,
            span: None,
            name,
            details,
            path,
            binding: Binding { entry, class },
            inputs,
            outputs,
        })
    }
}
/// One lane of a stage: its cards in chain order, and the columns it runs
/// between. `start` is the split its wire leaves and `end` the join it
/// returns to; `(0, the stage's width)` is the shape every lane had before
/// local forks meant, which is also what a card with no span of its own
/// reads as — so a saved rack that never names a span lays out, wires and
/// draws exactly as it did.
#[derive(Clone)]
pub struct Branch<'a> {
    pub cards: Vec<&'a Card>,
    pub start: usize,
    pub end: usize,
}
/// The rack's cards, grouped the way the wiring is built: a left-to-right
/// series of stages, each stage a set of lanes fed from the stage's wires,
/// each lane a serial chain of one or more cards running between two of the
/// stage's column boundaries.
///
/// Cards are grouped by consecutive equal `stage` values, exactly as stages
/// have always grouped; within a stage, consecutive cards sharing a `branch`
/// key chain into one lane, and a `None` key is always a lane of one. A
/// lane's span comes from its cards when every card of the lane carries the
/// same one — spans live on the cards, immune to the renumbering a stage
/// rename works on keys — and a lane whose cards disagree, or name nothing,
/// runs the whole stage like always.
pub fn branches(cards: &[Card]) -> Vec<Vec<Branch<'_>>> {
    let mut grouped: Vec<Vec<Vec<&Card>>> = Vec::new();
    for card in cards {
        let Some(stage) = grouped
            .last_mut()
            .filter(|stage| stage[0][0].stage == card.stage)
        else {
            grouped.push(vec![vec![card]]);
            continue;
        };
        if card.branch.is_some()
            && stage
                .last()
                .is_some_and(|branch| branch[0].branch == card.branch)
        {
            stage.last_mut().unwrap().push(card);
        } else {
            stage.push(vec![card]);
        }
    }
    grouped
        .into_iter()
        .map(|stage| {
            let width = stage.iter().map(Vec::len).max().unwrap_or(1);
            stage
                .into_iter()
                .map(|cards| {
                    let named = cards
                        .iter()
                        .map(|card| card.span)
                        .collect::<Option<Vec<_>>>();
                    let (start, end) = match named {
                        Some(spans) if spans.windows(2).all(|pair| pair[0] == pair[1]) => {
                            let (start, end) = spans[0];
                            (start as usize, end as usize)
                        }
                        _ => (0, width),
                    };
                    let lane = crate::layout::Lane::spanning(start, end, cards);
                    Branch {
                        cards: lane.chain,
                        start: lane.start,
                        end: lane.end,
                    }
                })
                .collect()
        })
        .collect()
}
pub fn arrange(
    cards: Vec<Card>,
    moving: NodeId,
    target: crate::layout::Target,
) -> Result<Vec<Card>, String> {
    let layout = branches(&cards)
        .iter()
        .map(|stage| {
            stage
                .iter()
                .map(|branch| crate::layout::Lane {
                    start: branch.start,
                    end: branch.end,
                    chain: branch.cards.iter().map(|card| card.id).collect(),
                })
                .collect()
        })
        .collect::<Vec<Vec<_>>>();
    let layout = crate::layout::place(&layout, moving, target)?;
    let mut cards = cards
        .into_iter()
        .map(|card| (card.id, card))
        .collect::<BTreeMap<_, _>>();
    let mut result = Vec::new();
    for stage in layout {
        let key = stage[0].chain[0];
        let width = stage.iter().map(|lane| lane.end).max().unwrap_or(1);
        for lane in stage {
            // A lane of one is just a card on its own: the key only needs
            // to exist to hold a chain together, and `arrange` has the whole
            // picture, so it writes the cleanest labels it can — and a span
            // is only worth keeping when the lane does not run the stage.
            let branch_key = (lane.chain.len() > 1).then_some(lane.chain[0]);
            let span = (lane.start != 0 || lane.end != width)
                .then_some((lane.start as u64, lane.end as u64));
            for id in lane.chain {
                let mut card = cards.remove(&id).unwrap();
                card.stage = key.0;
                card.branch = branch_key.map(|head| head.0);
                card.span = span;
                result.push(card);
            }
        }
    }
    Ok(result)
}
/// One stage's worth of card shapes: lanes top lane down, each lane a serial
/// chain of `(id, inputs, outputs)` running between the lane's own two
/// column boundaries. `plan` works on these rather than on loaded plugins,
/// so the width arithmetic stays witnessable.
type StagePlan = Vec<crate::layout::Lane<(NodeId, Channels, Channels)>>;

/// The rack's two ends, as the device settings describe them.
///
/// The rack used to carry a convention at its ends — stereo in, stereo out —
/// whatever the sound card had been told to do. These are those same two
/// numbers read off the settings instead, so the canvas can show what the
/// device actually delivers: a rack fed by one capture channel is one channel
/// wide at the `IN` jack, and a rack whose playback side names a single
/// destination is one channel wide at the `OUT` jack.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Route {
    /// Width the device hands the rack.
    pub input: Channels,
    /// Rate the capture runs at, once the settings carry one.
    pub input_rate: Option<f32>,
    /// Width the rack has to hand the device.
    pub output: Channels,
    /// Rate the playback side runs at, which may differ from the capture's.
    pub output_rate: Option<f32>,
}

impl Default for Route {
    /// A rack with no device settled yet: one channel coming, stereo going,
    /// no rate answered on either side. A live rack replaces it from settings.
    fn default() -> Self {
        Self {
            input: Channels::Mono,
            input_rate: None,
            output: Channels::Stereo,
            output_rate: None,
        }
    }
}

impl Route {
    /// The rack's ends as `settings` spells them. A rate stays `None` until
    /// the settings answer it, which is what the canvas prints as no route.
    pub fn of(settings: &Settings) -> Self {
        // A jack client names its playback destinations one at a time, so a
        // named right-hand destination is the only thing that makes its rack
        // output two channels wide. Every other backend picks a device and a
        // channel route, and a route naming two channels is a stereo output.
        let output = if settings.backend.as_deref() == Some("jack") {
            if settings.jack_output_right.is_some() {
                Channels::Stereo
            } else {
                Channels::Mono
            }
        } else {
            match settings.output_channels {
                Some(OutputChannels::Stereo(_)) => Channels::Stereo,
                Some(OutputChannels::Mono(_)) => Channels::Mono,
                // Nothing chosen yet: the transport still carries a stereo
                // frame to wherever the device ends up.
                None => Channels::Stereo,
            }
        };
        Self {
            // One capture channel is selected, whatever the device can send:
            // a second one would be a different choice, not a wider wire.
            input: Channels::Mono,
            input_rate: settings.sample_rate,
            output,
            output_rate: settings.output_sample_rate,
        }
    }
}

/// A card that cannot sit where the rack has put it: the wire that would feed
/// it is another width, and the rack converts only at its own two ends and
/// where a stage splits into parallel lanes. Anywhere else a wire is a single
/// conductor the operator can see, so the disagreement is refused rather than
/// quietly folded away — the engine has never converted implicitly, and a
/// card dropped into a chain is no reason to start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidthConflict {
    /// The card that cannot take the wire it is handed.
    pub card: NodeId,
    /// Width of the wire that would feed it.
    pub wire: Channels,
    /// Width the card needs.
    pub wants: Channels,
}

impl WidthConflict {
    /// The disagreement in one line, for whoever is holding the card. `name`
    /// is the card's plugin name when the caller has one.
    pub fn message(&self, name: Option<&str>) -> String {
        format!(
            "{} needs a {} input — the wire feeding it is {}",
            name.unwrap_or("This card"),
            self.wants,
            self.wire
        )
    }
}

/// A width conversion the rack had to make, which the canvas draws as a
/// circle with as many pins as it has channels on each side. Junctions are
/// read back out of a compiled graph rather than recomputed from the cards,
/// so the canvas can never draw a box the engine did not wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Junction {
    /// The conversion node itself.
    pub id: NodeId,
    pub inputs: Channels,
    pub outputs: Channels,
    /// What feeds it, or `None` when it feeds off the rack's `IN` jack.
    pub from: Option<NodeId>,
    /// What it feeds, or `None` when it hands to the rack's `OUT` jack.
    pub to: Option<NodeId>,
}

/// The rack's own `IN` jack and `OUT` jack, as `plan` names them.
const IN: NodeId = NodeId(0);
const OUT: NodeId = NodeId(u64::MAX);

/// Every width conversion in `graph`, in the order the rack is read.
pub fn junctions(graph: &RackGraph) -> Vec<Junction> {
    let mut found = graph
        .nodes
        .iter()
        .filter_map(|(id, kind)| match *kind {
            NodeKind::ChannelMap { inputs, outputs } => Some(Junction {
                id: *id,
                inputs,
                outputs,
                from: graph
                    .connections
                    .iter()
                    .find(|edge| edge.to == *id)
                    .map(|edge| edge.from)
                    .filter(|from| *from != IN),
                to: graph
                    .connections
                    .iter()
                    .find(|edge| edge.from == *id)
                    .map(|edge| edge.to)
                    .filter(|to| *to != OUT),
            }),
            _ => None,
        })
        .collect::<Vec<_>>();
    found.sort_by_key(|junction| {
        graph
            .priority
            .get(&junction.id)
            .copied()
            .unwrap_or_default()
    });
    found
}

/// Turn a refused arrangement into the line the rack answers with. The card
/// that cannot take its wire is named, so a refused drop says which card it
/// was rather than reporting an anonymous routing failure.
fn refuse(
    built: Result<(RackGraph, BTreeMap<NodeId, Binding>), WidthConflict>,
    cards: &[Card],
) -> Result<(RackGraph, BTreeMap<NodeId, Binding>), String> {
    built.map_err(|conflict| {
        let name = cards
            .iter()
            .find(|card| card.id == conflict.card)
            .map(|card| card.name.as_str());
        conflict.message(name)
    })
}

pub fn chain(
    cards: &[Card],
    route: Route,
) -> Result<(RackGraph, BTreeMap<NodeId, Binding>), WidthConflict> {
    let mut bindings = BTreeMap::new();
    let stages = branches(cards)
        .into_iter()
        .map(|stage| {
            stage
                .into_iter()
                .map(|lane| crate::layout::Lane {
                    start: lane.start,
                    end: lane.end,
                    chain: lane
                        .cards
                        .into_iter()
                        .map(|card| {
                            bindings.insert(card.id, card.binding.clone());
                            (card.id, card.inputs, card.outputs)
                        })
                        .collect(),
                })
                .collect()
        })
        .collect::<Vec<StagePlan>>();
    plan(&stages, route).map(|graph| (graph, bindings))
}
/// The wiring itself, carried out on card shapes — id, input count, output
/// count — rather than on loaded plugins, so the width arithmetic can be
/// witnessed without a single CLAP file on disk. The shapes arrive grouped
/// the way `branches` groups cards: stages left to right, each stage a set of
/// lanes with their own column spans, top lane down, each lane a serial chain.
///
/// A stage is read column by column. A wire enters the stage's left boundary;
/// at each interior boundary, the wires that *arrive* there — the tails of
/// lanes ending at it, the chain wires of cards seated inside it, and the one
/// wire still running straight through — are resolved: one arrival passes
/// through and stays the live wire, two or more join at a `Mixer` born at that
/// boundary, and a lane head seating on the boundary drinks the live wire
/// instead (the trunk forking to heads needs no node: it is a split, drawn as
/// a ring). Cards only convert widths where a split lets them: the rack's own
/// `IN` jack and a boundary where more than one lane head seats.
#[derive(Clone)]
enum Feed {
    /// A wire already built, carried by node and width.
    Wired(NodeId, Channels),
    /// The tail wire of a lane: its last card's output, running to the
    /// lane's end boundary.
    Tail(usize),
    /// The chain wire entering a card seated inside a boundary: the output
    /// of the card one step before it in its lane.
    Mid(usize, usize),
    /// The mixer born at a boundary, fed by that boundary's arrivals.
    Mixer(usize),
}
/// What arrives at one column boundary of a stage, as seen before any node
/// is born: the live wire heads seat on, the mixer (with its feeds, top lane
/// down) if two or more arrivals join, the count of lane heads seating here,
/// and whether this is the stage's own left edge.
struct Boundary {
    signal: Feed,
    mixer: Option<Vec<Feed>>,
    starts: usize,
    first: bool,
}
/// Sweep a stage's columns left to right, purely — no nodes, no birthing —
/// to work out what each boundary carries. `entry` is the wire entering the
/// stage's left edge.
fn boundaries(stage: &StagePlan, entry: Feed) -> Vec<Boundary> {
    let width = stage.iter().map(|lane| lane.end).max().unwrap_or(0);
    let mut live = Some(entry.clone());
    let mut sweep = Vec::with_capacity(width + 1);
    for column in 0..=width {
        let ends: Vec<usize> = (0..stage.len())
            .filter(|&lane| stage[lane].end == column)
            .collect();
        let starts = stage.iter().filter(|lane| lane.start == column).count();
        let mids: Vec<(usize, usize)> = (0..stage.len())
            .filter_map(|lane| {
                column
                    .checked_sub(stage[lane].start)
                    .filter(|&at| at > 0 && at < stage[lane].chain.len())
                    .map(|at| (lane, at))
            })
            .collect();
        let (signal, mixer) = if ends.is_empty() {
            let signal = live.clone().unwrap_or_else(|| entry.clone());
            if starts > 0 {
                live = None;
            }
            (signal, None)
        } else {
            let mut feeds: Vec<Feed> = ends.iter().map(|&lane| Feed::Tail(lane)).collect();
            feeds.extend(mids.iter().map(|&(lane, at)| Feed::Mid(lane, at - 1)));
            feeds.extend(live.take());
            if feeds.len() == 1 {
                let wire = feeds.into_iter().next().unwrap();
                live = (starts == 0 && mids.is_empty()).then(|| wire.clone());
                (wire, None)
            } else {
                live = None;
                (Feed::Mixer(column), Some(feeds))
            }
        };
        sweep.push(Boundary {
            signal,
            mixer,
            starts,
            first: column == 0,
        });
    }
    sweep
}
/// The graph under construction with its two counters: `born` ranks nodes in
/// reading order — the plan the DSP runs — and `next_id` hands out auxiliary
/// ids (maps and mixers) descending from just under `OUT`, exactly as before
/// lanes gave each lane its own span.
struct Wiring {
    graph: RackGraph,
    born: u64,
    next_id: u64,
}
impl Wiring {
    fn next(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id -= 1;
        id
    }
    fn bear(&mut self) -> u64 {
        let rank = self.born;
        self.born += 1;
        rank
    }
    fn map(&mut self, from: NodeId, inputs: Channels, outputs: Channels) -> NodeId {
        let map = self.next();
        let rank = self.bear();
        self.graph.priority.insert(map, rank);
        self.graph
            .nodes
            .insert(map, NodeKind::ChannelMap { inputs, outputs });
        self.graph.connections.push(Connection { from, to: map });
        map
    }
    fn card(&mut self, from: NodeId, id: NodeId, inputs: Channels, outputs: Channels) {
        let rank = self.bear();
        self.graph.priority.insert(id, rank);
        self.graph
            .nodes
            .insert(id, NodeKind::Plugin { inputs, outputs });
        self.graph.connections.push(Connection { from, to: id });
    }
    /// A mixer and its risers, feeds top lane down: every feed is raised to
    /// the join width — riser born before the mixer, so the plan still reads
    /// top-down — and then the mixer node itself is born.
    fn mixer(&mut self, feeds: Vec<(NodeId, Channels)>) -> (NodeId, Channels) {
        let join = feeds
            .iter()
            .map(|(_, width)| *width)
            .max()
            .unwrap_or(Channels::Mono);
        let mut raised = Vec::with_capacity(feeds.len());
        for (node, width) in feeds {
            raised.push(if width == join {
                node
            } else {
                self.map(node, width, join)
            });
        }
        let id = self.next();
        let rank = self.bear();
        self.graph.priority.insert(id, rank);
        self.graph.nodes.insert(id, NodeKind::Mixer(join));
        for riser in raised {
            self.graph.connections.push(Connection {
                from: riser,
                to: id,
            });
        }
        (id, join)
    }
}
/// What has been born inside one stage so far: each lane's card outputs, and
/// the mixers at boundaries already raised. A mixer is memoised so the card
/// that consumes it is born after it, whenever that consumption is reached.
#[derive(Default)]
struct StageWiring {
    cards: Vec<Vec<(NodeId, Channels)>>,
    mixers: BTreeMap<usize, (NodeId, Channels)>,
}
/// Resolve a boundary feed to the wire (node, width) it stands for, birthing
/// whatever it depends on first: a lane up to its tail (or up to a seated
/// card), and mixers recursively. Feeds always descend in column, so the
/// recursion terminates.
fn feed_wire(
    wiring: &mut Wiring,
    stage: &StagePlan,
    sweep: &[Boundary],
    lanes: &mut StageWiring,
    index: usize,
    feed: Feed,
) -> Result<(NodeId, Channels), WidthConflict> {
    match feed {
        Feed::Wired(node, width) => Ok((node, width)),
        Feed::Tail(lane) => {
            birth_lane(wiring, stage, sweep, lanes, index, lane, None)?;
            Ok(lanes.cards[lane][stage[lane].chain.len() - 1])
        }
        Feed::Mid(lane, at) => {
            birth_lane(wiring, stage, sweep, lanes, index, lane, Some(at))?;
            Ok(lanes.cards[lane][at])
        }
        Feed::Mixer(column) => {
            if let Some(wire) = lanes.mixers.get(&column) {
                return Ok(*wire);
            }
            let feeds = sweep[column].mixer.clone().unwrap_or_default();
            let mut raised = Vec::with_capacity(feeds.len());
            for feed in feeds {
                raised.push(feed_wire(wiring, stage, sweep, lanes, index, feed)?);
            }
            let wire = wiring.mixer(raised);
            lanes.mixers.insert(column, wire);
            Ok(wire)
        }
    }
}
/// Birth a lane's cards, chain head to tail, up to the whole lane or up to
/// (and including) one seated card. A head seats on its boundary's live
/// wire; a card seated at a boundary where a mixer joins is fed by that
/// mixer; anywhere else the chain carries the wire. Widths convert only at
/// a head seating on a split.
fn birth_lane(
    wiring: &mut Wiring,
    stage: &StagePlan,
    sweep: &[Boundary],
    lanes: &mut StageWiring,
    index: usize,
    lane: usize,
    upto: Option<usize>,
) -> Result<(), WidthConflict> {
    let target = upto.map_or(stage[lane].chain.len(), |at| {
        (at + 1).min(stage[lane].chain.len())
    });
    while lanes.cards[lane].len() < target {
        let at = lanes.cards[lane].len();
        let (id, inputs, outputs) = stage[lane].chain[at];
        let column = stage[lane].start + at;
        let (node, width) = if at == 0 {
            feed_wire(
                wiring,
                stage,
                sweep,
                lanes,
                index,
                sweep[column].signal.clone(),
            )?
        } else if sweep[column].mixer.is_some() {
            feed_wire(wiring, stage, sweep, lanes, index, Feed::Mixer(column))?
        } else {
            lanes.cards[lane][at - 1]
        };
        if inputs != width {
            let converts =
                at == 0 && ((sweep[column].first && index == 0) || sweep[column].starts > 1);
            if !converts {
                return Err(WidthConflict {
                    card: id,
                    wire: width,
                    wants: inputs,
                });
            }
            let map = wiring.map(node, width, inputs);
            wiring.card(map, id, inputs, outputs);
        } else {
            wiring.card(node, id, inputs, outputs);
        }
        lanes.cards[lane].push((id, outputs));
    }
    Ok(())
}
fn plan(stages: &[StagePlan], route: Route) -> Result<RackGraph, WidthConflict> {
    let mut wiring = Wiring {
        graph: RackGraph::default(),
        born: 0,
        next_id: u64::MAX - 1,
    };
    let input = IN;
    let output = OUT;
    // `branches` hands out the cards the way the rack is read: stage by
    // stage, top lane down, chain head to tail. Ranking every node at its
    // creation step is therefore the layout's reading order, and
    // `RackGraph::priority` makes the compile plan run lanes top-down — the
    // bottom lane starts its block work where the lane above ended.
    wiring
        .graph
        .nodes
        .insert(input, NodeKind::Input(route.input));
    let rank = wiring.bear();
    wiring.graph.priority.insert(input, rank);
    wiring
        .graph
        .nodes
        .insert(output, NodeKind::Output(route.output));
    let mut previous = input;
    let mut trunk = route.input;
    for (index, stage) in stages.iter().enumerate() {
        let sweep = boundaries(stage, Feed::Wired(previous, trunk));
        let width = stage.iter().map(|lane| lane.end).max().unwrap_or(0);
        let mut lanes = StageWiring {
            cards: vec![Vec::new(); stage.len()],
            ..StageWiring::default()
        };
        for lane in 0..stage.len() {
            birth_lane(&mut wiring, stage, &sweep, &mut lanes, index, lane, None)?;
        }
        // Mixers nothing consumed inside a lane still close the stage: a
        // legacy stage's end-boundary mixer lands here, after every lane's
        // cards, exactly as the old per-stage join did.
        for column in 0..=width {
            if sweep[column].mixer.is_some() {
                feed_wire(
                    &mut wiring,
                    stage,
                    &sweep,
                    &mut lanes,
                    index,
                    Feed::Mixer(column),
                )?;
            }
        }
        let (node, carried) = feed_wire(
            &mut wiring,
            stage,
            &sweep,
            &mut lanes,
            index,
            sweep[width].signal.clone(),
        )?;
        previous = node;
        trunk = carried;
    }
    if trunk != route.output {
        previous = wiring.map(previous, trunk, route.output);
    }
    wiring.graph.connections.push(Connection {
        from: previous,
        to: output,
    });
    // The output was in the graph from the start but is ranked last: the
    // master trim and metering belong after the last card, as they do on the
    // DSP timeline.
    let rank = wiring.bear();
    wiring.graph.priority.insert(output, rank);
    Ok(wiring.graph)
}
/// Master metering, published as f32 bits by the DSP thread and read by the
/// window. `input`/`output` carry linear RMS; the peak fields carry linear
/// *true* peak (dBTP), reconstructed between the samples, so a reading above
/// `1.0` means the signal would clip a converter downstream.
#[derive(Default)]
pub struct Levels {
    pub input: AtomicU32,
    pub output: AtomicU32,
    pub input_peak: AtomicU32,
    pub output_peak: AtomicU32,
    pub updated: AtomicU64,
}
struct Action {
    settings: Settings,
    cards: Vec<Card>,
    reply: Option<Sender<Result<(), String>>>,
    rollback: bool,
}
type OpenResult = Result<Option<(DeviceStreams, Endpoints, Settings)>, String>;
struct Opening {
    result: Receiver<OpenResult>,
    action: Action,
    /// When the thread asking for devices started. A driver that never answers
    /// would otherwise hold `busy()` — and with it every interaction the window
    /// offers, the drop queue first — for the rest of the session.
    since: Instant,
}
/// How long the rack waits for a thread that is opening devices before it says
/// so and lets the window be used again. A driver is entitled to take a moment
/// with a device it has never seen; it is not entitled to hold the application.
const OPENING_LIMIT: Duration = Duration::from_secs(20);
/// Streams nobody is waiting for any more. Devices opened after the rack
/// stopped waiting must not be dropped by the thread that opened them, because
/// releasing a driver from a thread nobody is watching is the fault the
/// deadline exists to stay out of, so they are kept mapped here instead.
static ABANDONED: Mutex<Vec<DeviceStreams>> = Mutex::new(Vec::new());
/// Whether two settings would drive the same streams. Everything counts but the
/// saved plugin list and the window, because neither of those is a device: a
/// different chain in the same window on the same interface is not a reason to
/// stop a driver and start it again.
fn same_devices(one: &Settings, other: &Settings) -> bool {
    one.backend == other.backend
        && one.input_device == other.input_device
        && one.output_device == other.output_device
        && one.jack_output_right == other.jack_output_right
        && one.output_channels == other.output_channels
        // A second output pair is a channel list the driver was opened with, so
        // adding or removing one is a different set of streams, not the same one
        // carrying a new line in a file. And a pair riding another driver is a
        // second driver loaded beside the first, which no reuse can arrange.
        && one.asio_second_output == other.asio_second_output
        && one.asio_second_device == other.asio_second_device
        && one.sample_rate == other.sample_rate
        && one.output_sample_rate == other.output_sample_rate
        && one.period_size == other.period_size
        && one.output_period_size == other.output_period_size
        && one.input_latency == other.input_latency
        && one.input_channel == other.input_channel
}
pub struct Runtime {
    pub cards: Vec<Card>,
    pub applied: Settings,
    pub message: String,
    pub levels: Arc<Levels>,
    /// Per-wire meter table shared with the DSP thread. Each wire of a
    /// metered plugin takes a slot when its chain is handed to the audio
    /// side, keyed `id << 1 | side`, and the GUI samples by the same key.
    /// The capacity is fixed at startup so a wider plan leaves its extra
    /// wires unmeasured instead of making the audio side allocate.
    pub wires: Arc<crate::routing::audio::WireLevels>,
    pub timings: Arc<crate::routing::audio::ProcessingTimes>,
    pub(crate) session: Option<LiveSession>,
    /// The session the chain before this one was built from, held between the
    /// moment it came off the transport and the moment the next chain is ready.
    /// Its instances are off the transport but they are still the plugins the
    /// cards name, so the ones a roster edit did not touch go forward into the
    /// new chain (`LiveSession::reusing`) instead of being saved, thrown away and
    /// rebuilt from their own state — which is also what their editor windows are
    /// attached to.
    retired: Option<LiveSession>,
    pub(crate) editor_geometry: BTreeMap<NodeId, crate::EditorGeometry>,
    last_save: Instant,
    pub(crate) persistence_path: Option<PathBuf>,
    saved_plugins: Vec<crate::SavedPlugin>,
    /// The gain staging the rack remembers: every wire's trim in dB, keyed
    /// `(card id, is output)`. The GUI holds the copy the operator drags and
    /// types; this is the copy the rack keeps, written by [`Runtime::set_trim`]
    /// and read back when a wire's meter is born, so a resumed rack starts at
    /// the levels it was left at. Card ids are never reused — `next_id` only
    /// ever grows, past every id a file named — so a remembered trim cannot
    /// drift onto a different card. What is *not* in here is clip state, for
    /// the reason [`crate::SavedPlugin`] spells out: a held clip is a report
    /// about audio that already passed.
    trims: BTreeMap<(u64, bool), f32>,
    /// The breaks the rack remembers: one entry per wire the operator left
    /// open, keyed `(card id, is output)` exactly as [`Runtime::trims`] is. A
    /// set rather than a map, because a break has no magnitude to carry — a
    /// cable is broken or it is not, and "how broken" is a question only a
    /// trim answers. The GUI asks for it the way it asks for a trim, on a
    /// wire's meter making its way out to the canvas, so a resumed rack starts
    /// with the switches as they were left. Card ids are never reused, so a
    /// remembered break cannot drift onto a different card.
    open_circuits: BTreeSet<(u64, bool)>,
    bypassed: BTreeSet<u64>,
    paused: BTreeSet<u64>,
    /// The level a wire's minimised meter has to reach to light up, in dBFS —
    /// the rack's one view preference, held next to the trims because it is
    /// remembered the same way: the GUI reads and writes this copy every frame
    /// and [`Runtime::persist`] is the only thing that puts it in a file.
    meter_gate_db: f32,
    /// The gate the file at `persistence_path` was last written with, for the
    /// same reason [`Runtime::saved_plugins`] exists: it is what tells
    /// `persist` a gate change is waiting.
    saved_meter_gate_db: f32,
    /// The OUT jack's trim the file was last written with. The live copy is
    /// not a field of its own: it rides in [`Runtime::trims`] under
    /// `(u64::MAX, false)`, the same map a card's wire trims live in, so the
    /// one place that clamps and remembers a trim serves the output stage
    /// too. This is only what `persist` compares against to know the master
    /// trim moved.
    saved_output_gain_db: f32,
    /// The IN jack's trim the file was last written with, for the same reason
    /// [`Runtime::saved_output_gain_db`] exists. The live copy rides in
    /// [`Runtime::trims`] under `(0, true)` — the capture node's id and its
    /// output side, the one side of that wire the engine meters.
    saved_input_gain_db: f32,
    worker: Option<JoinHandle<Endpoints>>,
    io: Option<DeviceStreams>,
    /// The rings of `io` while no worker owns them. `spawn_dsp` returns them
    /// when its owner stops, so an edit that only changes the chain can hand
    /// them to the next one and leave the streams — and the driver behind them
    /// — exactly as they are.
    idle: Option<Endpoints>,
    /// The settings the streams in `io` were opened from, as the resolver
    /// settled them, so a new chain is built at the rate they are running.
    live_settings: Option<Settings>,
    action: Option<Action>,
    opening: Option<Opening>,
    states: BTreeMap<NodeId, Vec<u8>>,
    counters: (u64, u64),
    last_audio: Instant,
    healthy_since: Instant,
    retries: u8,
    retry: Option<Instant>,
    /// Where a refused opening is going back to, kept because the first
    /// retreat may itself fail before it was ever applied, and the retries that
    /// follow it then have no other state to aim at.
    retreat: Option<(Settings, Vec<Card>)>,
    pub closing: bool,
    diagnostic: Option<String>,
    halted: bool,
}
/// The gain staging a saved rack starts with: both wires of every instance the
/// file names, in dB, clamped back into the range a trim can hold. A file
/// written before trims existed names none and every wire starts at unity,
/// which is the gain that rack was running; a hand-edited value beyond the end
/// of the track is brought back to the end, so the number the rack reads out is
/// a number it can also show and the next save writes the file back sane.
fn remembered_trims(plugins: &[crate::SavedPlugin]) -> BTreeMap<(u64, bool), f32> {
    plugins
        .iter()
        .flat_map(|saved| {
            [
                ((saved.id, false), saved.gain_in),
                ((saved.id, true), saved.gain_out),
            ]
        })
        // `f32::clamp` passes a `NaN` straight through, and a `NaN` trim is a
        // meter with no white line and a gain the engine will not apply, so a
        // number that is not a number is treated as no record at all.
        .filter(|(_, db)| db.is_finite())
        .map(|(key, db)| (key, db.clamp(-crate::TRIM_DB, crate::TRIM_DB)))
        .collect()
}
/// The breaks a saved rack starts with: one entry per wire the file says the
/// operator left open. A file written before wires could break says nothing and
/// every wire starts closed, which is how every rack in existence was wired, so
/// nothing here has to guess what a missing record meant. There is nothing to
/// clamp either: a break is a bool, not a number a hand-edited file can put
/// past the end of a track.
fn remembered_opens(plugins: &[crate::SavedPlugin]) -> BTreeSet<(u64, bool)> {
    plugins
        .iter()
        .flat_map(|saved| {
            [
                ((saved.id, false), saved.open_in),
                ((saved.id, true), saved.open_out),
            ]
        })
        .filter(|(_, open)| *open)
        .map(|(key, _)| key)
        .collect()
}
/// The gate a saved rack starts with, in dBFS. A file written before stubs had
/// a gate says nothing and the rack opens with the gate it ships — the level
/// that every stub was already resting behind. A hand-edited number is taken
/// as written, even one the dropdown does not offer, because a threshold is
/// only a comparison and the page shows whatever it reads; `NaN` and the
/// infinities are not comparisons, so they go back to the shipped default
/// rather than being saved out as a file the rack cannot read.
fn remembered_gate(db: f32) -> f32 {
    if db.is_finite() {
        db
    } else {
        crate::METER_GATE_DEFAULT_DB
    }
}
impl Runtime {
    pub fn new(settings: Settings) -> Self {
        let mut runtime = Self {
            cards: Vec::new(),
            applied: Settings::default(),
            message: String::new(),
            levels: Arc::default(),
            // Two slots per plugin; a few hundred cards is ample headroom.
            wires: Arc::new(crate::routing::audio::WireLevels::new(512)),
            timings: Arc::new(crate::routing::audio::ProcessingTimes::new(256)),
            session: None,
            retired: None,
            editor_geometry: settings
                .preferences
                .plugins
                .iter()
                .filter_map(|p| p.editor.map(|g| (NodeId(p.id), g)))
                .collect(),
            saved_plugins: settings.preferences.plugins.clone(),
            // The OUT jack's trim rides in the card trims' own map, under the
            // id no card can ever have, so a resumed rack starts the output
            // stage at the level it was left at. A file written before the
            // stage existed says 0 dB, which is what such a rack was already
            // running, and a hand-edited number is clamped and read the way
            // `remembered_trims` reads a card's. The IN jack's trim rides the
            // same map under the capture node's own id — 0 — with the input
            // half of the wire left unclaimed, exactly as the engine leaves it.
            trims: {
                let mut trims = remembered_trims(&settings.preferences.plugins);
                if settings.preferences.output_gain_db.is_finite() {
                    trims.insert(
                        (u64::MAX, false),
                        settings
                            .preferences
                            .output_gain_db
                            .clamp(-crate::TRIM_DB, crate::TRIM_DB),
                    );
                }
                if settings.preferences.input_gain_db.is_finite() {
                    trims.insert(
                        (0, true),
                        settings
                            .preferences
                            .input_gain_db
                            .clamp(-crate::TRIM_DB, crate::TRIM_DB),
                    );
                }
                trims
            },
            open_circuits: remembered_opens(&settings.preferences.plugins),
            bypassed: settings
                .preferences
                .plugins
                .iter()
                .filter(|saved| saved.bypassed)
                .map(|saved| saved.id)
                .collect(),
            paused: settings
                .preferences
                .plugins
                .iter()
                .filter(|saved| saved.paused)
                .map(|saved| saved.id)
                .collect(),
            meter_gate_db: remembered_gate(settings.preferences.meter_gate_db),
            // The file's own number, sane or not, so a gate the rack had to
            // look past is one `persist` writes back proper.
            saved_meter_gate_db: settings.preferences.meter_gate_db,
            // The file's own number again, so a trim the rack had to clamp
            // (or a `NaN` it had to ignore) is one `persist` writes back
            // proper, and once written the comparison rests.
            saved_output_gain_db: settings.preferences.output_gain_db,
            saved_input_gain_db: settings.preferences.input_gain_db,
            last_save: Instant::now(),
            persistence_path: None,
            worker: None,
            io: None,
            idle: None,
            live_settings: None,
            action: None,
            opening: None,
            states: BTreeMap::new(),
            counters: (0, 0),
            last_audio: Instant::now(),
            healthy_since: Instant::now(),
            retries: 0,
            retry: None,
            retreat: None,
            closing: false,
            diagnostic: None,
            halted: false,
        };
        runtime.action = Some(Action {
            settings,
            cards: Vec::new(),
            reply: None,
            rollback: false,
        });
        runtime
    }
    pub fn audio_running(&self) -> bool {
        self.io.is_some() && self.worker.is_some()
    }
    /// The route the rack is wired against, which is what the two endpoint
    /// cards are for: while streams run it is the settings they were opened
    /// from, and otherwise the configuration this process last opened — which
    /// at startup is the saved one, since asking the devices for it is the
    /// first thing a runtime does. A rack that has opened nothing yet reports
    /// the default route, because that is what it is wired against.
    pub fn route(&self) -> Route {
        Route::of(self.live_settings.as_ref().unwrap_or(&self.applied))
    }
    /// The width conversions the engine makes for the cards as they are
    /// arranged right now, read back off the graph `chain` builds rather than
    /// guessed at from the cards. The canvas draws one circle per entry, so an
    /// arrangement that refuses to wire — a card on a wire too narrow for it —
    /// answers with no circles at all instead of a half-drawn rack.
    pub fn junctions(&self) -> Option<Vec<Junction>> {
        chain(&self.cards, self.route())
            .ok()
            .map(|(graph, _)| junctions(&graph))
    }
    pub fn busy(&self) -> bool {
        self.action.is_some() || self.opening.is_some() || self.closing
    }
    pub fn apply(&mut self, settings: Settings) -> Result<Receiver<Result<(), String>>, String> {
        if self.busy() {
            return Err("Audio configuration is already changing".into());
        }
        let (reply, receiver) = bounded(1);
        self.retries = 0;
        self.retry = None;
        self.diagnostic = None;
        self.halted = false;
        self.begin(Action {
            settings,
            cards: self.cards.clone(),
            reply: Some(reply),
            rollback: false,
        });
        Ok(receiver)
    }
    /// Adopt a new arrangement, or refuse it where it stands. The engine
    /// never inserts a conversion between two neighbouring cards — a wire
    /// with no node on it has one width — so an arrangement the wiring
    /// cannot honour, a mono card dropped onto a stereo wire for one, is
    /// answered here at the drop instead of at the next device open: the
    /// name and the wire go into `message` for the canvas to show, the
    /// rack keeps the last arrangement that did wire, and whatever it was
    /// making keeps making. Device open refuses the same shapes for the
    /// settings route, so the check below is the same complaint heard one
    /// step earlier rather than a second rule.
    pub fn edit(&mut self, cards: Vec<Card>) {
        if self.busy() {
            return;
        }
        if let Err(error) = refuse(chain(&cards, self.route()), &cards) {
            self.message = error;
            return;
        }
        self.halted = false;
        self.retries = 0;
        self.begin(Action {
            settings: self.applied.clone(),
            cards,
            reply: None,
            rollback: false,
        });
    }
    fn begin(&mut self, action: Action) {
        // The editors stay where they are. A roster edit used to shut every one
        // of them here because it threw away every plugin instance and a CLAP
        // editor cannot outlive its instance; the instances this edit does not
        // touch now go forward into the new chain, so there is nothing of theirs
        // to close. The ones it does replace are closed in `Windows::service`,
        // on the frame the new chain arrives with them.
        self.message = "Stopping audio…".into();
        self.retry = None;
        if let Some(session) = &self.session {
            session.request_stop();
        }
        self.action = Some(action);
    }
    pub fn close(&mut self) {
        // Quitting is the one interruption nothing survives, so the instances
        // parked for a chain that will never be built go here, editors first.
        for session in [&mut self.session, &mut self.retired].into_iter().flatten() {
            session.close_editors();
        }
        self.retired = None;
        self.closing = true;
        self.retry = None;
        if let Some(session) = &self.session {
            session.request_stop();
        }
    }
    pub fn stopped(&self) -> bool {
        self.worker.is_none() && self.opening.is_none() && self.io.is_none()
    }
    /// Take the session off the running chain and hold it for the one on its way.
    ///
    /// Every roster edit brings the chain down, and the session that was running
    /// it holds the plugin instances — with their parameters, their state and the
    /// editors the user opened. Handing them to `LiveSession::reusing` is what
    /// lets the plugins an edit did not touch stay the same plugins, so a chain
    /// that comes down is never thrown away while a parked one is still waiting:
    /// the two are only in play together when the parked one is all that is left
    /// of those plugins, which is exactly the lot worth carrying.
    fn park(&mut self) {
        if let Some(session) = self.session.take() {
            self.retired = Some(session);
        }
    }
    pub fn tick(&mut self) {
        let mut native_error = None;
        if let Some(session) = &mut self.session
            && let Err(error) = session.service()
        {
            native_error = Some(error);
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            if let Some(worker) = self.worker.take() {
                // The rings come back with the owner that stops. Keeping them
                // is what lets the next chain have them without the rack
                // having to ask the device for anything at all.
                match worker.join() {
                    Ok(endpoints) => self.idle = Some(endpoints),
                    Err(_) => native_error = Some("Audio DSP worker panicked".into()),
                }
            }
            if let Some(session) = &mut self.session {
                if let Err(error) = session.service() {
                    native_error = Some(error);
                }
                match session.snapshot() {
                    Ok(states) => self.states.extend(states),
                    Err(error) => native_error = Some(error),
                }
            }
            // The chain came down because its DSP owner stopped. The plugins are
            // not gone with it.
            self.park();
            if self.action.is_none() && !self.closing && !self.halted {
                self.recover(
                    native_error.unwrap_or_else(|| "Plugin requested an audio restart".into()),
                    None,
                );
            }
        }
        // A thread that was asked for devices and never comes back holds
        // `busy()`, and every interaction the window offers waits behind it:
        // the dropped files first, then the settings page, then an editor.
        // Twenty seconds is far past any driver that is merely slow — this
        // machine's ASIO driver answers in half of one — and short enough that
        // a wedged one cannot keep the application. The thread is left to
        // itself: its sender has gone, and whatever it opens is parked.
        if self
            .opening
            .as_ref()
            .is_some_and(|opening| opening.since.elapsed() > OPENING_LIMIT)
        {
            let opening = self.opening.take().unwrap();
            let text = format!(
                "Audio off — opening the devices gave up after {} seconds",
                OPENING_LIMIT.as_secs()
            );
            if let Some(reply) = opening.action.reply {
                let _ = reply.send(Err(text.clone()));
            }
            if self.closing {
                self.message = text;
            } else {
                self.recover(
                    text,
                    Some((&opening.action.settings, &opening.action.cards)),
                );
            }
        }
        if self.closing {
            if self.worker.is_none() {
                self.persist();
                self.session = None;
                self.retired = None;
                self.action = None;
                if let Some(io) = self.io.take() {
                    let idle = self.idle.take();
                    self.live_settings = None;
                    std::thread::spawn(move || {
                        drop(idle);
                        drop(io);
                    });
                }
            }
            if let Some(opening) = &self.opening {
                match opening.result.try_recv() {
                    Ok(result) => {
                        drop(result);
                        self.opening = None;
                    }
                    Err(crossbeam::channel::TryRecvError::Disconnected) => self.opening = None,
                    _ => {}
                }
            }
            return;
        }
        if let Some(io) = &self.io {
            let counters = (
                io.counters.input.load(Ordering::Acquire),
                io.counters.output.load(Ordering::Acquire),
            );
            if counters.0 != self.counters.0 && counters.1 != self.counters.1 {
                self.last_audio = Instant::now();
                self.counters = counters;
            }
            let error = io.failure().or_else(|| {
                (self.last_audio.elapsed() > Duration::from_secs(3))
                    .then(|| "Audio devices stopped delivering input/output callbacks".into())
            });
            if let Some(error) = error
                && self.action.is_none()
                && self.opening.is_none()
            {
                self.recover(error, None);
            }
            if self.healthy_since.elapsed() > Duration::from_secs(10)
                && self.last_audio.elapsed() < Duration::from_secs(1)
            {
                self.retries = 0;
            }
        }
        if let Some(deadline) = self.retry
            && Instant::now() >= deadline
            && !self.busy()
        {
            self.retry = None;
            let (settings, cards) = self
                .retreat
                .clone()
                .unwrap_or_else(|| (self.applied.clone(), self.cards.clone()));
            self.begin(Action {
                settings,
                cards,
                reply: None,
                rollback: true,
            });
        }
        if self.worker.is_none()
            && self.opening.is_none()
            && let Some(action) = self.action.take()
        {
            if let Some(session) = &mut self.session {
                let _ = session.service();
                if let Ok(states) = session.snapshot() {
                    self.states.extend(states);
                }
            }
            // The chain comes down here, and its plugins come with it into
            // `retired` rather than out of the process: the ones the action does
            // not touch are about to be asked for by name.
            self.park();
            // Dropping a plugin onto a rack that is making sound changes the
            // chain, and the transport is not part of the chain: it carries
            // frames between two rings that the DSP owner on this thread feeds
            // and empties. So when the settings this action carries are the
            // ones the running streams were opened from, the streams answer the
            // action themselves. Nothing is stopped, freed or reloaded, which is
            // the point: releasing a driver while a library is being loaded into
            // the same process has this rack faulting inside the module that was
            // released, at one fixed instruction, for every plugin it can load.
            let reuse = self.io.is_some()
                && self.idle.is_some()
                && self.live_settings.is_some()
                && same_devices(&self.applied, &action.settings);
            let (sender, result) = bounded(1);
            if reuse {
                let opened = self
                    .io
                    .take()
                    .zip(self.idle.take())
                    .zip(self.live_settings.take())
                    .map(|((io, idle), resolved)| (io, idle, resolved));
                let _ = sender.send(Ok(opened));
                self.message = "Rebuilding the chain…".into();
            } else {
                let old = self.io.take();
                let idle = self.idle.take();
                self.live_settings = None;
                let settings = action.settings.clone();
                self.message = "Opening audio devices…".into();
                std::thread::spawn(move || {
                    drop(old);
                    drop(idle);
                    let result = (|| {
                        let backend = settings.backend.as_deref().unwrap_or("dummy");
                        if backend == "dummy" {
                            return Ok(None);
                        }
                        let snapshot = nice_common::devices::snapshot(
                            backend,
                            settings.input_device.as_deref(),
                            settings.output_device.as_deref(),
                        );
                        let resolved = snapshot.resolve_settings(&settings)?;
                        let (io, endpoints) = nice_common::audio_io::open(&resolved)?;
                        Ok(Some((io, endpoints, resolved)))
                    })();
                    if let Err(crossbeam::channel::SendError(Ok(Some((io, ..))))) =
                        sender.send(result)
                    {
                        // The rack stopped waiting and took the receiver with it.
                        // Whatever this thread went on to open is not its to hand
                        // back, and dropping the streams here would release a driver
                        // from a thread nobody is watching, so they are kept instead.
                        ABANDONED
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .push(io);
                    }
                });
            }
            self.opening = Some(Opening {
                result,
                action,
                since: Instant::now(),
            });
        }
        let result = self
            .opening
            .as_ref()
            .and_then(|opening| match opening.result.try_recv() {
                Ok(result) => Some(result),
                Err(crossbeam::channel::TryRecvError::Disconnected) => {
                    Some(Err("Audio setup worker stopped".into()))
                }
                Err(crossbeam::channel::TryRecvError::Empty) => None,
            });
        if let Some(result) = result {
            let opening = self.opening.take().unwrap();
            let prepared = result.and_then(|opened| {
                let Some((io, endpoints, resolved)) = opened else {
                    // Retain inactive instances for controls even with audio off.
                    let route = Route::of(&opening.action.settings);
                    let (graph, bindings) =
                        refuse(chain(&opening.action.cards, route), &opening.action.cards)?;
                    let (mut session, mut audio) = LiveSession::reusing(
                        self.retired.take(),
                        &graph,
                        &bindings,
                        48000.0,
                        BLOCK,
                        &self.states,
                    )?;
                    session.request_stop();
                    std::thread::spawn(move || {
                        let _ = audio.process(&[], &mut [], 0);
                    })
                    .join()
                    .map_err(|_| "Plugin retirement worker panicked")?;
                    session.service()?;
                    self.session = Some(session);
                    return Ok(());
                };
                let rate = resolved.sample_rate.unwrap() as f64;
                let route = Route::of(&resolved);
                let (graph, bindings) =
                    refuse(chain(&opening.action.cards, route), &opening.action.cards)?;
                let (session, mut audio) = match LiveSession::reusing(
                    self.retired.take(),
                    &graph,
                    &bindings,
                    rate,
                    BLOCK,
                    &self.states,
                ) {
                    Ok(built) => built,
                    // The transport is not what failed. Handing it back rather
                    // than letting it go here is the same release the rack
                    // avoids everywhere else, and the rollback two lines below
                    // is about to ask for those devices again.
                    Err(error) => {
                        self.io = Some(io);
                        self.idle = Some(endpoints);
                        self.live_settings = Some(resolved);
                        return Err(error);
                    }
                };
                // Register the wire meters before the plan crosses to the DSP
                // thread. The attach resets the table, which is safe here
                // because `park` joined the previous worker earlier in this
                // same tick, so no publish can still be in flight.
                audio.attach_wire_levels(self.wires.clone());
                for card in &opening.action.cards {
                    self.wires.set_bypassed(card.id, self.bypassed(card.id.0));
                    self.wires.set_paused(card.id, self.paused(card.id.0));
                }
                audio.attach_processing_times(self.timings.clone(), rate);
                self.worker = Some(spawn_dsp(audio, endpoints, self.levels.clone(), route));
                self.session = Some(session);
                self.io = Some(io);
                self.live_settings = Some(resolved);
                self.last_audio = Instant::now();
                self.healthy_since = Instant::now();
                self.counters = (0, 0);
                Ok(())
            });
            match prepared {
                Ok(()) => {
                    self.applied = opening.action.settings;
                    self.cards = opening.action.cards;
                    self.retreat = None;
                    self.states
                        .retain(|id, _| self.cards.iter().any(|card| card.id == *id));
                    self.message = if self.io.is_some() {
                        "Audio running".into()
                    } else {
                        "Audio off — choose a backend in settings".into()
                    };
                    if opening.action.rollback
                        && let Some(error) = &self.diagnostic
                    {
                        self.message = format!("{} — {error}", self.message);
                    }
                    if let Some(reply) = opening.action.reply {
                        let _ = reply.send(Ok(()));
                    }
                }
                Err(error) => {
                    self.message = error.clone();
                    self.diagnostic = Some(error.clone());
                    if let Some(reply) = opening.action.reply {
                        let _ = reply.send(Err(error));
                    }
                    if opening.action.rollback {
                        if self.retries < 3 {
                            self.retries += 1;
                            self.retry = Some(Instant::now() + Duration::from_secs(1));
                        }
                    } else {
                        self.retreat_to(&opening.action.settings, &opening.action.cards);
                    }
                }
            }
        }
        if !self.busy() && self.last_save.elapsed() > Duration::from_secs(2) {
            self.persist();
        }
    }
    /// Restore records through the same trusted entry loader used for file drops.
    pub(crate) fn restore(&mut self, cards: Vec<Card>, states: BTreeMap<NodeId, Vec<u8>>) {
        self.states = states;
        if let Some(action) = &mut self.action {
            action.cards = cards;
        }
    }
    pub(crate) fn paused(&self, id: u64) -> bool {
        self.paused.contains(&id)
    }
    pub(crate) fn set_paused(&mut self, id: u64, paused: bool) {
        if paused {
            self.paused.insert(id);
        } else {
            self.paused.remove(&id);
        }
        self.wires.set_paused(NodeId(id), paused);
    }
    pub(crate) fn bypassed(&self, id: u64) -> bool {
        self.bypassed.contains(&id)
    }
    pub(crate) fn set_bypassed(&mut self, id: u64, bypassed: bool) {
        if bypassed {
            self.bypassed.insert(id);
        } else {
            self.bypassed.remove(&id);
        }
        self.wires.set_bypassed(NodeId(id), bypassed);
    }
    /// What the rack remembers for one wire, in dB: the trim a resumed rack
    /// opens its meter at, and what gets saved back out. `None` means the rack
    /// holds no record of this wire at all, which is what a card dragged in
    /// this session looks like before its first trim.
    pub(crate) fn trim(&self, id: u64, side: bool) -> Option<f32> {
        self.trims.get(&(id, side)).copied()
    }
    /// Take ownership of a wire's trim. Every way a trim changes in the GUI —
    /// a drag, a typed number, a value arriving from a saved file — goes
    /// through the one place that writes the meter and the engine, and that
    /// place calls this, so [`Runtime::persist`] needs to know nothing about
    /// where a trim came from. A non-finite number is refused rather than
    /// remembered: the engine already declines to apply one, and a `NaN`
    /// written into the settings file would come back as a file the rack
    /// cannot read.
    pub(crate) fn set_trim(&mut self, id: u64, side: bool, gain_db: f32) {
        if gain_db.is_finite() {
            self.trims
                .insert((id, side), gain_db.clamp(-crate::TRIM_DB, crate::TRIM_DB));
        }
    }
    /// Whether the rack remembers this wire as an open circuit. `false` means it
    /// holds no break here, which is what every wire of a card dragged in this
    /// session looks like: a cable is closed until somebody throws its switch.
    pub(crate) fn open_circuit(&self, id: u64, side: bool) -> bool {
        self.open_circuits.contains(&(id, side))
    }
    /// Take ownership of a wire's break, for the same reason
    /// [`Runtime::set_trim`] takes ownership of a trim: the right-click that
    /// throws the switch is one of several ways it moves — a saved file and an
    /// undo are others — and [`Runtime::persist`] should not have to care which
    /// one moved it. Closing a wire takes its entry out again rather than
    /// recording a `false`, so the set stays a list of the cables actually
    /// broken and a rack with nothing broken saves out as one.
    pub(crate) fn set_open_circuit(&mut self, id: u64, side: bool, open: bool) {
        if open {
            self.open_circuits.insert((id, side));
        } else {
            self.open_circuits.remove(&(id, side));
        }
    }
    /// The level a minimised meter has to reach to be drawn in colour, in dB.
    /// The view reads this every frame it paints a stub box, so it lives here
    /// rather than being read out of the settings file over and over.
    pub(crate) fn meter_gate(&self) -> f32 {
        self.meter_gate_db
    }
    /// Take ownership of the meter gate, for the same reason [`Runtime::set_trim`]
    /// takes ownership of a trim: the settings page is one of several ways the
    /// number can move, and [`Runtime::persist`] should not have to care which
    /// one moved it. A non-finite number is refused; the gate stays where it
    /// was, and the next save writes the value the rack is actually using.
    pub(crate) fn set_meter_gate(&mut self, db: f32) {
        if db.is_finite() {
            self.meter_gate_db = db;
        }
    }
    pub(crate) fn persist(&mut self) {
        if self.action.is_some() || self.opening.is_some() {
            return;
        }
        let Some(path) = self.persistence_path.clone() else {
            return;
        };
        if let Some(session) = &mut self.session {
            match session.save_states() {
                Ok(states) => self.states.extend(states),
                Err(error) => {
                    self.message = error;
                    return;
                }
            }
        }
        let plugins: Vec<_> = self
            .cards
            .iter()
            .map(|card| crate::SavedPlugin {
                id: card.id.0,
                stage: card.stage,
                branch: card.branch,
                span: card.span,
                path: card.path.clone(),
                class: card.binding.class.clone(),
                name: card.name.clone(),
                state: self.states.get(&card.id).cloned(),
                editor: self.editor_geometry.get(&card.id).copied(),
                // A card carries its own trims and breaks out of the rack's
                // memory, so a card removed from the canvas takes them with it.
                // The maps themselves keep their entries, exactly as
                // `editor_geometry` does: an edit may be undone, and a trim or a
                // switch the operator set is the kind of thing worth having
                // back.
                gain_in: self.trim(card.id.0, false).unwrap_or_default(),
                gain_out: self.trim(card.id.0, true).unwrap_or_default(),
                open_in: self.open_circuit(card.id.0, false),
                open_out: self.open_circuit(card.id.0, true),
                bypassed: self.bypassed(card.id.0),
                paused: self.paused(card.id.0),
            })
            .collect();
        // One write covers every preference the rack keeps changing under the
        // operator's hands: the cards with their trims and breaks, the gate
        // the stub boxes are coloured against, and the two jack trims on the
        // ends of the rack. Any of them moving is enough to rewrite the file,
        // because a settings file holds one snapshot of all and cannot be
        // brought half up to date. A break needs no extra comparison of its
        // own: it travels on the card it belongs to, so a wire thrown open
        // makes the cards differ from the last thing written. The jack trims
        // need none either beyond their own numbers: each is one value, read
        // from the same map the cards' trims were just read out of.
        let output_gain_db = self.trim(u64::MAX, false).unwrap_or_default();
        let input_gain_db = self.trim(0, true).unwrap_or_default();
        if plugins != self.saved_plugins
            || self.meter_gate_db != self.saved_meter_gate_db
            || output_gain_db != self.saved_output_gain_db
            || input_gain_db != self.saved_input_gain_db
        {
            let (mut settings, warning) = Settings::load_from(&path);
            if let Some(warning) = warning {
                self.message = warning;
                return;
            }
            settings.preferences.plugins = plugins.clone();
            settings.preferences.meter_gate_db = self.meter_gate_db;
            settings.preferences.output_gain_db = output_gain_db;
            settings.preferences.input_gain_db = input_gain_db;
            match settings.save_to(&path) {
                Ok(()) => {
                    self.applied.preferences.plugins = plugins.clone();
                    self.applied.preferences.meter_gate_db = self.meter_gate_db;
                    self.applied.preferences.output_gain_db = output_gain_db;
                    self.applied.preferences.input_gain_db = input_gain_db;
                    self.saved_plugins = plugins;
                    self.saved_meter_gate_db = self.meter_gate_db;
                    self.saved_output_gain_db = output_gain_db;
                    self.saved_input_gain_db = input_gain_db;
                }
                Err(error) => self.message = error,
            }
        }
        self.last_save = Instant::now();
    }
    /// Ask for the state a refused opening goes back out of. Once something has
    /// been applied there is a state worth going back to, and that is what gets
    /// asked for. Before anything has been applied there is nothing to go back
    /// to but the empty rack [`Runtime::new`] started from — and the rack writes
    /// what it holds into its settings file, so going back to that writes out an
    /// empty rack and a saved chain is gone over a driver that would not open.
    /// So until something has been applied, the chain of the action that failed
    /// is kept and only its devices are dropped: the window keeps what it was
    /// given, and the message says what was refused.
    fn retreat_to(&mut self, asked: &Settings, cards: &[Card]) {
        let (settings, cards) = if self.session.is_some() {
            (self.applied.clone(), self.cards.clone())
        } else {
            (
                Settings {
                    backend: None,
                    ..asked.clone()
                },
                cards.to_vec(),
            )
        };
        self.retreat = Some((settings.clone(), cards.clone()));
        self.begin(Action {
            settings,
            cards,
            reply: None,
            rollback: true,
        });
    }
    fn recover(&mut self, error: String, asked: Option<(&Settings, &[Card])>) {
        self.message = error.clone();
        self.diagnostic = Some(error);
        if self.retries >= 3 {
            self.close_audio_only();
            return;
        }
        self.retries += 1;
        match asked {
            Some((settings, cards)) => self.retreat_to(settings, cards),
            None => {
                let (settings, cards) = self
                    .retreat
                    .clone()
                    .unwrap_or_else(|| (self.applied.clone(), self.cards.clone()));
                self.begin(Action {
                    settings,
                    cards,
                    reply: None,
                    rollback: true,
                });
            }
        }
    }
    fn close_audio_only(&mut self) {
        self.halted = true;
        if let Some(session) = &self.session {
            session.request_stop();
        }
        if let Some(io) = self.io.take() {
            let idle = self.idle.take();
            self.live_settings = None;
            std::thread::spawn(move || {
                drop(idle);
                drop(io);
            });
        }
        self.levels.input.store(0, Ordering::Relaxed);
        self.levels.output.store(0, Ordering::Relaxed);
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(session) = &self.session {
            session.request_stop();
        }
        // The DSP worker does not depend on device callbacks, so it will wake,
        // stop and return its graph even after both streams have died.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(session) = &mut self.session {
            let _ = session.service();
        }
    }
}
/// One block's linear true-peak reading of a stereo pair. Each channel keeps
/// its reconstruction history across blocks, so a crest that straddles a block
/// boundary is still found. A reading above `1.0` is a finding, not an
/// overflow: the waveform between the samples really is over full scale.
fn block_peak(meters: &mut [TruePeak; 2], left: &[f32], right: &[f32]) -> f32 {
    for (channel, meter) in [left, right].into_iter().zip(meters.iter_mut()) {
        meter.process(channel);
    }
    meters[0].take().max(meters[1].take())
}
/// Start the device-independent DSP owner. The matching main-thread session
/// must be serviced and stopped before joining this worker.
pub fn spawn_dsp(
    mut audio: LiveAudio,
    mut io: Endpoints,
    levels: Arc<Levels>,
    route: Route,
) -> JoinHandle<Endpoints> {
    std::thread::spawn(move || {
        // The transport carries a two-channel frame each way, which is as
        // wide as any route the settings can describe gets. Feeding the rack
        // fewer channels than that, and reading back only what it has to
        // hand on, is what makes the canvas's `IN`/`OUT` widths the widths
        // the device really moves.
        let feeds = route.input.count().clamp(1, 2);
        let drains = route.output.count().clamp(1, 2);
        let mut left = [0.0; BLOCK];
        let mut right = [0.0; BLOCK];
        let mut out_left = [0.0; BLOCK];
        let mut out_right = [0.0; BLOCK];
        // Both sides keep their reconstruction history from block to block, so
        // a crest that lands across a block boundary is still found.
        let mut meter_in = [TruePeak::new(), TruePeak::new()];
        let mut meter_out = [TruePeak::new(), TruePeak::new()];
        loop {
            let count = io.input.slots().min(BLOCK);
            let mut energy = 0.0f64;
            for frame in 0..count {
                let samples = io.input.pop().unwrap();
                left[frame] = samples[0];
                right[frame] = samples[1];
                energy += f64::from(samples[0]).powi(2);
            }
            // A one-channel route has one channel to measure, and the second
            // half of the transport's frame is that same channel duplicated —
            // reading it twice keeps the RMS of the signal rather than
            // averaging it down against silence.
            let (in_left, in_right) = if feeds == 2 {
                (&left[..count], &right[..count])
            } else {
                (&left[..count], &left[..count])
            };
            let input_peak = block_peak(&mut meter_in, in_left, in_right);
            let feed = [&left[..count], &right[..count]];
            let mut group = [&mut out_left[..count], &mut out_right[..count]];
            let status = audio.process(&feed[..feeds], &mut [&mut group[..drains]], count);
            if status == Ok(LiveStatus::Stopped) {
                break;
            }
            if count == 0 {
                std::thread::park_timeout(Duration::from_micros(250));
                continue;
            }
            if status.is_err() {
                continue;
            }
            let mut output_energy = 0.0f64;
            for frame in 0..count {
                output_energy += f64::from(out_left[frame])
                    .powi(2)
                    .max(f64::from(out_right[frame]).powi(2));
                // A mono rack output hands the same value to both hands of
                // the frame: a mono device route takes the first, and the
                // transport's own fold of a two-channel route back to one is
                // `(l + r) * 0.5`, which returns that value untouched.
                let right = if drains == 2 {
                    out_right[frame]
                } else {
                    out_left[frame]
                };
                let _ = io
                    .output
                    .push(nice_common::resample::StereoFrame([out_left[frame], right]));
            }
            let (out_left_meter, out_right_meter) = if drains == 2 {
                (&out_left[..count], &out_right[..count])
            } else {
                (&out_left[..count], &out_left[..count])
            };
            let output_peak = block_peak(&mut meter_out, out_left_meter, out_right_meter);
            levels
                .input_peak
                .store(input_peak.to_bits(), Ordering::Relaxed);
            levels
                .output_peak
                .store(output_peak.to_bits(), Ordering::Relaxed);
            levels.updated.store(crate::now_ms(), Ordering::Release);
            levels.input.store(
                ((energy / count as f64).sqrt() as f32).to_bits(),
                Ordering::Relaxed,
            );
            levels.output.store(
                ((output_energy / count as f64).sqrt() as f32).to_bits(),
                Ordering::Relaxed,
            );
        }
        levels.input.store(0, Ordering::Relaxed);
        levels.output.store(0, Ordering::Relaxed);
        io
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn endpoints() -> (
        Endpoints,
        rtrb::Producer<[f32; 2]>,
        rtrb::Consumer<nice_common::resample::StereoFrame>,
    ) {
        let (capture, input) = rtrb::RingBuffer::new(64);
        let (output, consumer) = rtrb::RingBuffer::new(64);
        (Endpoints { input, output }, capture, consumer)
    }
    /// A route wide at both ends: the convention the rack carried before the
    /// device was ever asked, and the shape the DSP tests feed it.
    fn wide() -> Route {
        Route {
            input: Channels::Stereo,
            input_rate: None,
            output: Channels::Stereo,
            output_rate: None,
        }
    }
    /// A stage written the way the rack was drawn before lanes carried
    /// spans of their own: every lane full width from the left edge.
    fn lanes(chains: Vec<Vec<(NodeId, Channels, Channels)>>) -> StagePlan {
        chains
            .into_iter()
            .map(|chain| crate::layout::Lane::from(0, chain))
            .collect()
    }
    /// The plan's width arithmetic, witnessed without any plugin: a map is
    /// born only where the rack is allowed to convert — its own two jacks and
    /// the head of a branch — so a pair of mono cards on a mono input costs
    /// exactly one box, the join on the way out to a stereo playback side.
    #[test]
    fn a_plan_carries_its_route_and_maps_only_the_devices_own_ends() {
        let mono = Channels::Mono;
        let graph = plan(
            &[lanes(vec![vec![
                (NodeId(2), mono, mono),
                (NodeId(3), mono, mono),
            ]])],
            Route::default(),
        )
        .expect("mono cards on a mono input");
        let tail = NodeId(u64::MAX - 1);
        assert_eq!(
            graph
                .connections
                .iter()
                .map(|c| (c.from, c.to))
                .collect::<Vec<_>>(),
            vec![
                (NodeId(0), NodeId(2)),
                (NodeId(2), NodeId(3)),
                (NodeId(3), tail),
                (tail, NodeId(u64::MAX)),
            ],
            "the two cards share one bare wire and the head needs no box"
        );
        assert_eq!(graph.nodes[&NodeId(0)], NodeKind::Input(mono));
        assert_eq!(
            graph.nodes[&NodeId(u64::MAX)],
            NodeKind::Output(Channels::Stereo)
        );
        assert_eq!(
            graph.nodes[&tail],
            NodeKind::ChannelMap {
                inputs: mono,
                outputs: Channels::Stereo,
            }
        );
        graph.compile().expect("the mono chain is a legal graph");
    }
    /// The picture on the canvas with nothing in the middle of it: one
    /// capture channel, a stereo playback side, so one circle doing 1 in /
    /// 2 out between the two cards.
    #[test]
    fn an_empty_rack_still_meets_the_playback_width() {
        let (graph, _) = chain(&[], Route::default()).expect("an empty rack still wires");
        let split = NodeId(u64::MAX - 1);
        assert_eq!(
            graph
                .connections
                .iter()
                .map(|c| (c.from, c.to))
                .collect::<Vec<_>>(),
            vec![(NodeId(0), split), (split, NodeId(u64::MAX))]
        );
        assert_eq!(
            junctions(&graph),
            vec![Junction {
                id: split,
                inputs: Channels::Mono,
                outputs: Channels::Stereo,
                from: None,
                to: None,
            }]
        );
        graph.compile().expect("the split is a legal graph");
    }
    /// Both ends narrow: the rack has nothing to say about width at all, and
    /// the input card hands its one channel straight to the output card.
    #[test]
    fn a_narrow_device_needs_no_box_at_all() {
        let route = Route {
            output: Channels::Mono,
            ..Route::default()
        };
        let (graph, _) = chain(&[], route).expect("a mono rack wires itself");
        assert_eq!(graph.connections.len(), 1);
        assert!(junctions(&graph).is_empty(), "no transition, no circle");
        graph
            .compile()
            .expect("the mono pass-through is a legal graph");
    }
    /// Branches that disagree on width meet at the widest tail: the mono
    /// lane climbs back to stereo on a riser of its own, the stereo lane —
    /// which split off the mono trunk at its own head — joins the mixer
    /// bare-handed, and the mixer is stereo so the trunk needs nothing more
    /// on the way to a stereo output.
    #[test]
    fn a_plan_lifts_each_branch_to_the_widest_tail_before_the_mixer() {
        let mono = Channels::Mono;
        let graph = plan(
            &[lanes(vec![
                vec![(NodeId(2), mono, mono)],
                vec![(NodeId(3), Channels::Stereo, Channels::Stereo)],
            ])],
            Route::default(),
        )
        .expect("a split stage converts at each branch head");
        let riser = graph
            .nodes
            .iter()
            .find(|(_, kind)| {
                matches!(
                    kind,
                    NodeKind::ChannelMap {
                        inputs,
                        outputs,
                    } if *inputs == mono && *outputs == Channels::Stereo
                )
            })
            .map(|(id, _)| *id)
            .expect("the mono branch carries a riser");
        let mixer = graph
            .nodes
            .iter()
            .find(|(_, kind)| matches!(kind, NodeKind::Mixer(_)))
            .map(|(id, _)| *id)
            .expect("the stage mixes");
        assert_eq!(graph.nodes[&mixer], NodeKind::Mixer(Channels::Stereo));
        let feeds = graph
            .connections
            .iter()
            .filter(|c| c.to == mixer)
            .map(|c| c.from)
            .collect::<Vec<_>>();
        assert_eq!(feeds, vec![riser, NodeId(3)]);
        let tail = graph
            .connections
            .iter()
            .find(|c| c.to == NodeId(u64::MAX))
            .unwrap()
            .from;
        assert_eq!(tail, mixer, "a stereo trunk to a stereo output is bare");
        graph.compile().expect("the mixed stage is a legal graph");
    }
    /// Where every branch agrees on mono there is nothing to lift: the mixer
    /// is mono too, and the single map back to stereo waits at the output,
    /// wherever the branches happened to meet.
    #[test]
    fn a_plan_holds_a_unanimous_branch_width_all_the_way_to_the_output() {
        let mono = Channels::Mono;
        let graph = plan(
            &[lanes(vec![
                vec![(NodeId(2), mono, mono)],
                vec![(NodeId(3), mono, mono)],
            ])],
            Route::default(),
        )
        .expect("two mono branches on a mono input");
        let maps = graph
            .nodes
            .values()
            .filter(|kind| matches!(kind, NodeKind::ChannelMap { .. }))
            .count();
        assert_eq!(maps, 1, "only the rack's own output jack converts");
        let mixer = graph
            .nodes
            .iter()
            .find(|(_, kind)| matches!(kind, NodeKind::Mixer(_)))
            .map(|(id, _)| *id)
            .expect("the stage mixes");
        assert_eq!(graph.nodes[&mixer], NodeKind::Mixer(mono));
        let last_map = graph
            .connections
            .iter()
            .find(|c| c.to == NodeId(u64::MAX))
            .unwrap()
            .from;
        assert_eq!(
            graph.nodes[&last_map],
            NodeKind::ChannelMap {
                inputs: mono,
                outputs: Channels::Stereo,
            }
        );
        graph.compile().expect("the mono stage is a legal graph");
    }
    /// A rack of stereo cards on a stereo device still carries no maps at
    /// all — the counts only ever meet as equals.
    #[test]
    fn a_plan_needs_no_crook_when_the_counts_already_agree() {
        let stereo = Channels::Stereo;
        let graph = plan(&[lanes(vec![vec![(NodeId(2), stereo, stereo)]])], wide()).unwrap();
        assert_eq!(graph.connections.len(), 2);
        assert!(
            graph
                .nodes
                .values()
                .all(|kind| !matches!(kind, NodeKind::ChannelMap { .. }))
        );
        graph.compile().expect("the stereo chain is a legal graph");
    }
    /// The shape of the one mono-input bundle this repo ships: it takes the
    /// capture's single channel and answers with two. On a stereo playback
    /// side that is the whole route met at both ends, so the card stands
    /// alone between the cards with no circle anywhere. Put it on a mono
    /// playback side and the rack still runs its mono in — only the way out
    /// is joined, and the join names the card it reads.
    #[test]
    fn a_card_wide_at_one_end_only_meets_the_route_it_was_given() {
        let mono = Channels::Mono;
        let stereo = Channels::Stereo;
        let wide_out = &[lanes(vec![vec![(NodeId(2), mono, stereo)]])][..];
        let graph = plan(wide_out, Route::default()).expect("the card meets both ends");
        assert!(
            junctions(&graph).is_empty(),
            "a mono input and a stereo output are already what the card wants"
        );
        assert_eq!(
            graph
                .connections
                .iter()
                .map(|c| (c.from, c.to))
                .collect::<Vec<_>>(),
            vec![(NodeId(0), NodeId(2)), (NodeId(2), NodeId(u64::MAX))],
            "both of its wires are bare"
        );
        graph.compile().expect("the wide card is a legal graph");

        let narrow = Route {
            output: mono,
            ..Route::default()
        };
        let graph = plan(wide_out, narrow).expect("a narrow playback still takes the card");
        assert_eq!(
            junctions(&graph),
            vec![Junction {
                id: NodeId(u64::MAX - 1),
                inputs: stereo,
                outputs: mono,
                from: Some(NodeId(2)),
                to: None,
            }],
            "the join sits behind the card, on the way to the output card"
        );
        graph.compile().expect("the joined card is a legal graph");
    }
    /// Between two cards the wire is one conductor with nothing in the middle
    /// of it, so a card that wants another width there is refused and named
    /// rather than quietly fed through a map. This is the drop the canvas has
    /// to answer with a reason: a mono card to the right of a stereo one, and
    /// a stereo card to the right of a mono one.
    #[test]
    fn a_card_on_the_wrong_wire_is_refused_not_mapped() {
        let mono = Channels::Mono;
        let stereo = Channels::Stereo;
        let refused = plan(
            &[lanes(vec![vec![
                (NodeId(2), stereo, stereo),
                (NodeId(3), mono, stereo),
            ]])],
            wide(),
        )
        .expect_err("a mono card cannot sit on the stereo wire");
        assert_eq!(
            refused,
            WidthConflict {
                card: NodeId(3),
                wire: stereo,
                wants: mono,
            }
        );
        assert_eq!(
            refused.message(Some("Nice NAM")),
            "Nice NAM needs a mono input — the wire feeding it is stereo"
        );
        assert_eq!(
            refused.message(None),
            "This card needs a mono input — the wire feeding it is stereo"
        );
        let refused = plan(
            &[lanes(vec![vec![
                (NodeId(2), mono, mono),
                (NodeId(3), stereo, stereo),
            ]])],
            wide(),
        )
        .expect_err("nor a stereo card on the wire a mono card leaves");
        assert_eq!(
            refused,
            WidthConflict {
                card: NodeId(3),
                wire: mono,
                wants: stereo,
            }
        );
    }
    /// The two endpoint cards read one thing: the route the audio side was
    /// actually opened with. A playback with no right port is a mono card
    /// whatever the file's channel list says, and while streams run it is
    /// what they run on that the cards report.
    #[test]
    fn the_cards_read_the_route_the_engine_was_opened_with() {
        let mut runtime = Runtime::new(Settings::default());
        assert_eq!(
            runtime.route(),
            Route::default(),
            "a rack that has opened nothing is wired against the default"
        );
        let jack = Settings {
            backend: Some("jack".into()),
            sample_rate: Some(48000.0),
            output_sample_rate: Some(48000.0),
            ..Settings::default()
        };
        runtime.applied = jack.clone();
        assert_eq!(
            runtime.route(),
            Route {
                input: Channels::Mono,
                input_rate: Some(48000.0),
                output: Channels::Mono,
                output_rate: Some(48000.0),
            },
            "a jack playback naming no right port carries one channel"
        );
        runtime.live_settings = Some(Settings {
            jack_output_right: Some("M Series Pro:playback_AUX1".into()),
            ..jack
        });
        assert_eq!(runtime.route().output, Channels::Stereo);
        runtime.applied = Settings::default();
        assert_eq!(
            runtime.route().output,
            Channels::Stereo,
            "the running streams outrank the last apply"
        );
    }
    /// The canvas draws its circles off the compiled graph, so a split and a
    /// join read back with their widths and their two ends — and nothing at
    /// all where the counts already agreed.
    #[test]
    fn junctions_are_read_back_off_the_compiled_graph() {
        let mono = Channels::Mono;
        let stereo = Channels::Stereo;
        let (graph, _) = chain(&[], Route::default()).unwrap();
        let split = NodeId(u64::MAX - 1);
        assert_eq!(
            junctions(&graph),
            vec![Junction {
                id: split,
                inputs: mono,
                outputs: stereo,
                from: None,
                to: None,
            }],
            "both of a bare rack's ends are the rack's own jacks"
        );
        // A stereo card on a mono playback side is the other way round, and
        // now both circles exist: the split names the card it feeds, the join
        // names the card it reads and no drain.
        let graph = plan(
            &[lanes(vec![vec![(NodeId(2), stereo, stereo)]])],
            Route {
                output: mono,
                ..Route::default()
            },
        )
        .unwrap();
        assert_eq!(
            junctions(&graph),
            vec![
                Junction {
                    id: split,
                    inputs: mono,
                    outputs: stereo,
                    from: None,
                    to: Some(NodeId(2)),
                },
                Junction {
                    id: NodeId(u64::MAX - 2),
                    inputs: stereo,
                    outputs: mono,
                    from: Some(NodeId(2)),
                    to: None,
                },
            ],
            "the circles come back in the order the rack is read"
        );
    }
    #[test]
    fn dsp_stops_and_returns_its_graph_when_no_device_callback_ever_arrives() {
        let (graph, bindings) = chain(&[], wide()).unwrap();
        let (mut session, audio) =
            LiveSession::prepare(&graph, &bindings, 48000.0, BLOCK, &BTreeMap::new()).unwrap();
        let (io, _capture, _output) = endpoints();
        let worker = spawn_dsp(audio, io, Arc::default(), wide());
        session.request_stop();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !worker.is_finished() {
            assert!(
                Instant::now() < deadline,
                "DSP did not stop independently of device callbacks"
            );
            std::thread::yield_now();
        }
        let _ = worker.join().unwrap();
        assert_eq!(session.service().unwrap(), LiveStatus::Stopped);
        assert!(session.snapshot().unwrap().is_empty());
    }
    #[test]
    fn dsp_preserves_stereo_and_retires_after_capture_stops() {
        let (graph, bindings) = chain(&[], wide()).unwrap();
        let (mut session, audio) =
            LiveSession::prepare(&graph, &bindings, 48000.0, BLOCK, &BTreeMap::new()).unwrap();
        let (io, mut capture, mut output) = endpoints();
        for sample in [[0.25, -0.5], [0.75, -1.0]] {
            capture.push(sample).unwrap();
        }
        let worker = spawn_dsp(audio, io, Arc::default(), wide());
        let deadline = Instant::now() + Duration::from_secs(2);
        while output.slots() < 2 {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        session.request_stop();
        let _ = worker.join().unwrap();
        assert_eq!(session.service().unwrap(), LiveStatus::Stopped);
        assert_eq!(output.pop().unwrap().0, [0.25, -0.5]);
        assert_eq!(output.pop().unwrap().0, [0.75, -1.0]);
    }
    /// One capture channel is one channel: the second hand of the transport's
    /// frame is dropped before the rack sees it, and the split the plan put at
    /// the `OUT` end is what fills both hands on the way back. The right-hand
    /// values the test pushed never reach the rack.
    #[test]
    fn dsp_feeds_the_rack_only_what_the_route_carries() {
        let route = Route::default();
        let (graph, bindings) = chain(&[], route).unwrap();
        let (session, audio) =
            LiveSession::prepare(&graph, &bindings, 48000.0, BLOCK, &BTreeMap::new()).unwrap();
        let (io, mut capture, mut output) = endpoints();
        for sample in [[0.25, -0.5], [0.75, -1.0]] {
            capture.push(sample).unwrap();
        }
        let worker = spawn_dsp(audio, io, Arc::default(), route);
        let deadline = Instant::now() + Duration::from_secs(2);
        while output.slots() < 2 {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        session.request_stop();
        let _ = worker.join().unwrap();
        assert_eq!(output.pop().unwrap().0, [0.25, 0.25]);
        assert_eq!(output.pop().unwrap().0, [0.75, 0.75]);
    }
    /// The other way round: a stereo rack onto a playback side that names one
    /// destination. The join inside the rack does the arithmetic — the mean of
    /// the two lanes, which is not the left lane the transport would have
    /// handed out had the rack stayed two-wide.
    #[test]
    fn dsp_joins_a_wide_rack_onto_a_narrow_route() {
        let route = Route {
            input: Channels::Stereo,
            input_rate: None,
            output: Channels::Mono,
            output_rate: None,
        };
        let (graph, bindings) = chain(&[], route).unwrap();
        assert_eq!(
            junctions(&graph).len(),
            1,
            "the join sits at the rack's own output jack"
        );
        let (session, audio) =
            LiveSession::prepare(&graph, &bindings, 48000.0, BLOCK, &BTreeMap::new()).unwrap();
        let (io, mut capture, mut output) = endpoints();
        for sample in [[0.25, -0.5], [0.75, -1.0]] {
            capture.push(sample).unwrap();
        }
        let worker = spawn_dsp(audio, io, Arc::default(), route);
        let deadline = Instant::now() + Duration::from_secs(2);
        while output.slots() < 2 {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        session.request_stop();
        let _ = worker.join().unwrap();
        assert_eq!(output.pop().unwrap().0, [-0.125, -0.125]);
        assert_eq!(output.pop().unwrap().0, [-0.125, -0.125]);
    }

    /// A quarter-rate sine whose every sample sits a quarter period away from
    /// a crest: the loudest sample is 0.707 while the signal itself reaches
    /// 1.0 between them. The meter has to report the signal, not the samples.
    #[test]
    fn published_peaks_are_true_peaks_not_loudest_samples() {
        let (graph, bindings) = chain(&[], wide()).unwrap();
        let (session, audio) =
            LiveSession::prepare(&graph, &bindings, 48000.0, BLOCK, &BTreeMap::new()).unwrap();
        let (io, mut capture, mut output) = endpoints();
        let tone: Vec<[f32; 2]> = (0..64)
            .map(|frame| {
                let sample = (std::f32::consts::FRAC_PI_4
                    + frame as f32 * std::f32::consts::FRAC_PI_2)
                    .sin();
                [sample, -sample]
            })
            .collect();
        for sample in &tone {
            capture.push(*sample).unwrap();
        }
        let levels = Arc::new(Levels::default());
        let worker = spawn_dsp(audio, io, Arc::clone(&levels), wide());
        let deadline = Instant::now() + Duration::from_secs(2);
        while levels.updated.load(Ordering::Acquire) == 0 {
            assert!(Instant::now() < deadline, "DSP never published levels");
            std::thread::yield_now();
        }
        session.request_stop();
        let _ = worker.join().unwrap();
        while output.pop().is_ok() {}
        let loudest_sample = tone
            .iter()
            .flatten()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        let input = f32::from_bits(levels.input_peak.load(Ordering::Relaxed));
        let out = f32::from_bits(levels.output_peak.load(Ordering::Relaxed));
        assert!((loudest_sample - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
        assert!(input > 0.95 && input < 1.01, "input peak read {input}");
        assert!(out > 0.95 && out < 1.01, "output peak read {out}");
    }
    #[test]
    fn closing_before_audio_start_cancels_pending_open() {
        let mut runtime = Runtime::new(Settings::default());
        runtime.close();
        runtime.tick();
        assert!(runtime.stopped());
        assert!(!runtime.audio_running());
    }
    #[test]
    fn failed_apply_reports_the_error_and_restores_the_previous_configuration() {
        let mut runtime = Runtime::new(Settings::default());
        runtime.action = None;
        let previous = runtime.applied.clone();
        let (reply, answer) = bounded(1);
        let (send, result) = bounded(1);
        let wanted = Settings {
            backend: Some("core-audio".into()),
            ..Default::default()
        };
        runtime.opening = Some(Opening {
            result,
            action: Action {
                settings: wanted,
                cards: Vec::new(),
                reply: Some(reply),
                rollback: false,
            },
            since: Instant::now(),
        });
        send.send(Err("Input device 'Missing mic': device disconnected".into()))
            .unwrap();
        runtime.tick();
        assert!(
            answer
                .recv()
                .unwrap()
                .unwrap_err()
                .contains("device disconnected")
        );
        assert_eq!(runtime.applied, previous);
        assert!(runtime.action.as_ref().unwrap().rollback);
        let deadline = Instant::now() + Duration::from_secs(2);
        while runtime.busy() {
            runtime.tick();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(runtime.applied, previous);
        assert!(runtime.message.contains("device disconnected"));
    }

    /// What a cold rack does with the first dropped file, with no settings
    /// file to give it a backend. Nothing was ever chosen, so nothing ever
    /// ran: the chain is built, stopped again before its first block, and
    /// then written out by the automatic save two seconds later, which asks
    /// instances that never saw a callback for their state. Every step of
    /// that has to end in a message rather than a dead process, because it
    /// is what a first run does the moment a plugin is dropped on it. A
    /// second drop then asks the same cold controller to retire the session
    /// it built for the first, which is the first time anything inactive is
    /// actually taken down.
    ///
    /// `NICE_RACK_TEST_CLAP_2` names the second export; unset, the first
    /// stands in. Load `Nice NAM.clap` among the two and the run can die at
    /// the very end, after every assertion here passed, with an access
    /// violation no harness can report: the Windows application log names the
    /// faulting module `Nice NAM.clap_unloaded`, code executing out of a
    /// library the loader had already unmapped. `nice-nam/src/engine.rs`
    /// spawns a detached thread per instance which sleeps twenty
    /// milliseconds a lap and is never joined, so whether that thread is
    /// still inside the module when its last card releases the library is a
    /// race, and which of two orders loses it moves with the drop order. The
    /// rack's own work is what this test witnesses; keeping a thread inside a
    /// module shorter than the thread is the plugin's promise to keep.
    #[test]
    #[ignore = "requires NICE_RACK_TEST_CLAP pointing to a freshly built repository CLAP export"]
    #[allow(unsafe_code)]
    fn a_dropped_plugin_is_saved_before_audio_has_ever_run() {
        let path = PathBuf::from(
            std::env::var_os("NICE_RACK_TEST_CLAP").expect("Set NICE_RACK_TEST_CLAP"),
        );
        // SAFETY: explicit test opt-in to this repository's own plugin export.
        let entry = unsafe { nice_clap_host::Entry::load(&path).unwrap() };
        let class = entry
            .descriptors()
            .unwrap()
            .first()
            .expect("the test export contains at least one plugin")
            .id
            .clone();
        let card = Card::new(
            NodeId(2),
            path.clone(),
            entry,
            class.clone(),
            "Test plugin".into(),
        )
        .unwrap();
        let file = frost::scratch::claim("rack-cold-drop").join("nice-rack-settings.ron");
        let _ = std::fs::remove_file(&file);

        let mut runtime = Runtime::new(Settings::default());
        settle(&mut runtime);
        assert!(!runtime.audio_running());
        assert_eq!(runtime.message, "Audio off — choose a backend in settings");

        runtime.edit(vec![card]);
        settle(&mut runtime);
        assert_eq!(runtime.cards.len(), 1);
        assert!(!runtime.audio_running());

        runtime.persistence_path = Some(file.clone());
        runtime.last_save = Instant::now() - Duration::from_secs(3);
        runtime.tick();
        let (saved, warning) = Settings::load_from(&file);
        assert_eq!(warning, None);
        assert_eq!(saved.preferences.plugins.len(), 1);
        assert_eq!(saved.preferences.plugins[0].class, class);
        assert!(!runtime.busy());

        // The second drop is its own case: the chain that never ran has to be
        // torn down and rebuilt around a second instance, so an inactive
        // session is retired for the first time rather than merely dropped,
        // and a second entry is loaded beside the first. `NICE_RACK_TEST_CLAP_2`
        // names another export; unset, the same file stands in for it.
        let second_path = std::env::var_os("NICE_RACK_TEST_CLAP_2")
            .map(PathBuf::from)
            .unwrap_or_else(|| path.clone());
        // SAFETY: explicit test opt-in to this repository's own plugin export.
        let second_entry = unsafe { nice_clap_host::Entry::load(&second_path).unwrap() };
        let second_class = second_entry
            .descriptors()
            .unwrap()
            .first()
            .expect("the second test export contains at least one plugin")
            .id
            .clone();
        let second = Card::new(
            NodeId(4),
            second_path.clone(),
            second_entry,
            second_class.clone(),
            "Second plugin".into(),
        )
        .unwrap();
        let mut cards = runtime.cards.clone();
        cards.push(second);
        runtime.edit(cards);
        settle(&mut runtime);
        assert_eq!(runtime.cards.len(), 2);
        assert!(!runtime.audio_running());
        runtime.last_save = Instant::now() - Duration::from_secs(3);
        runtime.tick();
        let (saved, warning) = Settings::load_from(&file);
        assert_eq!(warning, None);
        assert_eq!(saved.preferences.plugins.len(), 2);
        assert_eq!(saved.preferences.plugins[1].class, second_class);
        assert!(!runtime.busy());
        runtime.close();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !runtime.stopped() {
            runtime.tick();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    /// What refusing a wrong-width drop actually gives the operator: the
    /// rack answers at the drop, names the card, says which width it wanted
    /// and which wire it was handed, and the arrangement that was already
    /// playing is untouched — no sound stops to discover a wiring mistake
    /// the canvas could have pointed at. Then the same card dropped at the
    /// head, where the rack still carries the device's own mono, is taken
    /// as the mono plugin it is, and the rack's own split goes away.
    ///
    /// The two exports have to disagree about width, which is what
    /// `NICE_RACK_TEST_CLAP_WIDE` and `NICE_RACK_TEST_CLAP_NARROW` are
    /// for: in this repository those are `Nice Pi-Fase.clap`, the one
    /// bundle that takes stereo and answers in stereo, and `Nice NAM.clap`,
    /// the one bundle with a single input. Either one unset, there is
    /// nothing to refuse and the test declines rather than assert nothing.
    /// The accepted half builds one NAM instance, so the unload race
    /// documented on [`a_dropped_plugin_is_saved_before_audio_has_ever_run`]
    /// can kill the process after every assertion here has passed.
    #[test]
    #[ignore = "requires NICE_RACK_TEST_CLAP_WIDE, and NICE_RACK_TEST_CLAP_NARROW to witness the refusal"]
    #[allow(unsafe_code)]
    fn a_mono_card_dropped_on_a_stereo_wire_is_refused_where_it_stands() {
        let Some(wide_path) = std::env::var_os("NICE_RACK_TEST_CLAP_WIDE").map(PathBuf::from)
        else {
            return;
        };
        let Some(narrow_path) = std::env::var_os("NICE_RACK_TEST_CLAP_NARROW").map(PathBuf::from)
        else {
            return;
        };
        // SAFETY: explicit test opt-in to this repository's own plugin export.
        let wide_entry = unsafe { nice_clap_host::Entry::load(&wide_path).unwrap() };
        let wide_class = wide_entry
            .descriptors()
            .unwrap()
            .first()
            .expect("the wide test export contains at least one plugin")
            .id
            .clone();
        let wide = Card::new(
            NodeId(2),
            wide_path,
            wide_entry,
            wide_class,
            "Stereo card".into(),
        )
        .unwrap();
        // SAFETY: explicit test opt-in to this repository's own plugin export.
        let narrow_entry = unsafe { nice_clap_host::Entry::load(&narrow_path).unwrap() };
        let narrow_class = narrow_entry
            .descriptors()
            .unwrap()
            .first()
            .expect("the narrow test export contains at least one plugin")
            .id
            .clone();
        let narrow = Card::new(
            NodeId(3),
            narrow_path,
            narrow_entry,
            narrow_class,
            "Mono card".into(),
        )
        .unwrap();
        // Both halves of the test are statements about these bundles'
        // widths, so they are pinned rather than assumed: a harness that
        // pointed either variable at the wrong export would otherwise watch
        // a legal rack sail through the refusal assertion, or a refused one
        // fail the accepted one, with the rack nowhere in the complaint.
        assert_eq!(
            (wide.inputs, wide.outputs),
            (Channels::Stereo, Channels::Stereo),
            "NICE_RACK_TEST_CLAP_WIDE should name a stereo-in, stereo-out export, Nice Pi-Fase"
        );
        assert_eq!(
            (narrow.inputs, narrow.outputs),
            (Channels::Mono, Channels::Stereo),
            "NICE_RACK_TEST_CLAP_NARROW should name the mono-in export, Nice NAM"
        );

        let mut runtime = Runtime::new(Settings::default());
        settle(&mut runtime);
        // The rack the drop lands on is the default route: one channel in,
        // stereo out, so even with no cards at all it is already splitting
        // its wire — the place a drop can land "right of the split".
        assert_eq!(
            runtime.junctions(),
            Some(vec![Junction {
                id: NodeId(u64::MAX - 1),
                inputs: Channels::Mono,
                outputs: Channels::Stereo,
                from: None,
                to: None,
            }]),
            "the empty rack on the default route shows its split"
        );

        // Dropped last, the mono card sits on the stereo wire leaving the
        // stereo card. The rack refuses: nothing arrives, and the reason
        // names the card and both widths.
        runtime.edit(vec![wide.clone(), narrow.clone()]);
        assert!(
            runtime.cards.is_empty(),
            "a refused arrangement never arrives"
        );
        assert_eq!(
            runtime.message,
            format!(
                "{} needs a mono input — the wire feeding it is stereo",
                narrow.name
            )
        );

        // Dropped first, the same card takes the device's own mono before
        // anything has been split; its stereo pair is what the rack runs on
        // anyway, so the rack needs no conversion of its own at all.
        runtime.edit(vec![narrow, wide]);
        settle(&mut runtime);
        assert_eq!(runtime.cards.len(), 2);
        assert_eq!(runtime.junctions(), Some(vec![]));
        runtime.close();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !runtime.stopped() {
            runtime.tick();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }

    /// The gain staging belongs to the arrangement, so a rack whose output
    /// wire was pulled down 6 dB comes back with that wire at −6 dB, and the
    /// operator does not rediscover their levels by ear after every launch.
    /// This runs on the same cold controller the drop test above uses — no
    /// backend chosen, no audio ever started — because what is on trial is the
    /// record and not the sound.
    ///
    /// The shape of the test is deliberate: a trim is changed with nothing
    /// else about the rack touched, and the file is rewritten by the ordinary
    /// two second debounce inside [`Runtime::tick`], exactly as it happens
    /// when someone drags a white line and walks away. A save forced through
    /// `persist` would prove far less, because the GUI is the only thing that
    /// calls it and `persist` knows nothing about where a trim came from.
    ///
    /// The last part is the half of the request that says *without the
    /// clipping state*: a rack saved while every meter in it was red has to
    /// resume green. Nothing about a latch is written out, so the check is
    /// that the file does not mention one.
    #[test]
    #[ignore = "requires NICE_RACK_TEST_CLAP pointing to a freshly built repository CLAP export"]
    #[allow(unsafe_code)]
    fn the_gain_staging_comes_back_with_the_rack() {
        let path = PathBuf::from(
            std::env::var_os("NICE_RACK_TEST_CLAP").expect("Set NICE_RACK_TEST_CLAP"),
        );
        // SAFETY: explicit test opt-in to this repository's own plugin export.
        let entry = unsafe { nice_clap_host::Entry::load(&path).unwrap() };
        let class = entry
            .descriptors()
            .unwrap()
            .first()
            .expect("the test export contains at least one plugin")
            .id
            .clone();
        let card = Card::new(NodeId(2), path, entry, class.clone(), "Test plugin".into()).unwrap();
        let file = frost::scratch::claim("rack-gain-staging").join("nice-rack-settings.ron");
        let _ = std::fs::remove_file(&file);

        let mut runtime = Runtime::new(Settings::default());
        settle(&mut runtime);
        runtime.edit(vec![card]);
        settle(&mut runtime);
        runtime.persistence_path = Some(file.clone());
        runtime.last_save = Instant::now() - Duration::from_secs(3);
        runtime.tick();
        let (saved, warning) = Settings::load_from(&file);
        assert_eq!(warning, None);
        assert_eq!(saved.preferences.plugins.len(), 1);
        assert_eq!(
            (
                saved.preferences.plugins[0].gain_in,
                saved.preferences.plugins[0].gain_out
            ),
            (0.0, 0.0),
            "a card dragged in this session is remembered at unity"
        );

        runtime.set_trim(2, true, -6.0);
        runtime.set_trim(2, false, 3.5);
        runtime.last_save = Instant::now() - Duration::from_secs(3);
        runtime.tick();
        let (saved, warning) = Settings::load_from(&file);
        assert_eq!(warning, None);
        assert_eq!(saved.preferences.plugins.len(), 1, "one card, still");
        assert_eq!(saved.preferences.plugins[0].class, class);
        assert_eq!(saved.preferences.plugins[0].gain_out, -6.0);
        assert_eq!(saved.preferences.plugins[0].gain_in, 3.5);

        // Back into a controller that never saw the drag, which is what a
        // resume is: the trims arrive as file data and are clamped on the way
        // in, so a hand-edited file cannot smuggle a trim outside the range
        // the GUI can draw.
        let remembered = remembered_trims(&saved.preferences.plugins);
        assert_eq!(remembered.get(&(2, true)).copied(), Some(-6.0));
        assert_eq!(remembered.get(&(2, false)).copied(), Some(3.5));
        let wild = remembered_trims(&[crate::SavedPlugin {
            gain_in: 900.0,
            gain_out: f32::NAN,
            ..saved.preferences.plugins[0].clone()
        }]);
        assert_eq!(
            wild.get(&(2, false)).copied(),
            Some(crate::TRIM_DB),
            "a number past the range is drawn at the end of it"
        );
        assert_eq!(
            wild.get(&(2, true)),
            None,
            "and a `NaN` is no record at all, so the meter falls back to the engine"
        );

        // A break is remembered the same way, and needs no comparison of its
        // own to be noticed: it rides on the card it belongs to, so throwing a
        // switch is enough to make the cards differ from the last thing
        // written. This is the part of the request that says *of course* — a
        // rack left with a cable pulled is that rack, and a resume that
        // quietly closed it would be rewiring the thing behind the operator.
        runtime.set_open_circuit(2, true, true);
        runtime.last_save = Instant::now() - Duration::from_secs(3);
        runtime.tick();
        let (saved, warning) = Settings::load_from(&file);
        assert_eq!(warning, None);
        assert!(
            saved.preferences.plugins[0].open_out,
            "the cable left broken comes back broken"
        );
        assert!(
            !saved.preferences.plugins[0].open_in,
            "and its neighbour still says the other one carries"
        );
        assert_eq!(
            remembered_opens(&saved.preferences.plugins),
            [(2u64, true)].into_iter().collect::<BTreeSet<_>>()
        );
        runtime.set_open_circuit(2, true, false);
        assert!(
            !runtime.open_circuit(2, true),
            "closing a wire takes the record away rather than arguing with it"
        );

        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            !text.contains("clip") && !text.contains("latch"),
            "the saved rack holds gain staging and no clipping state: {text}"
        );

        runtime.close();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !runtime.stopped() {
            runtime.tick();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    /// The OUT jack's trim is remembered the way a card's wire trims are, and
    /// needs no plugin to prove it: it rides in the same map, goes through the
    /// same clamping door on the way in from a file, and is one of the numbers
    /// [`Runtime::persist`] writes back out. The rack here has no cards and no
    /// backend at all — what is on trial is the record, and a master trim
    /// before the first plugin is a record like any other.
    #[test]
    fn the_output_trim_comes_back_with_the_rack() {
        let file = frost::scratch::claim("rack-output-gain").join("nice-rack-settings.ron");
        let _ = std::fs::remove_file(&file);

        let mut runtime = Runtime::new(Settings::default());
        settle(&mut runtime);
        assert_eq!(
            runtime.trim(u64::MAX, false),
            Some(0.0),
            "every file states an output gain, defaulted or not, so even a \
             rack as new as this file holds its master record, at unity"
        );
        runtime.persistence_path = Some(file.clone());

        // The ordinary debounce, as when somebody drags the white line on
        // the OUT jack and walks away. Nothing else about the rack moved, so
        // it is the master trim alone that makes the file worth rewriting.
        runtime.set_trim(u64::MAX, false, -4.5);
        runtime.last_save = Instant::now() - Duration::from_secs(3);
        runtime.tick();
        let (saved, warning) = Settings::load_from(&file);
        assert_eq!(warning, None);
        assert!(saved.preferences.plugins.is_empty(), "no cards, still");
        assert_eq!(saved.preferences.output_gain_db, -4.5);

        // A resume: the number comes out of the file and into the same map a
        // card's trim arrives in, which is what the GUI reads every frame.
        let resumed = Runtime::new(saved);
        assert_eq!(resumed.trim(u64::MAX, false), Some(-4.5));

        // And the two rules a card's trims follow, one key over: a number
        // past the end of the track is drawn at the end of it, and a number
        // that is not a number is no record at all.
        let wild = Settings {
            preferences: crate::RackPreferences {
                output_gain_db: 900.0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            Runtime::new(wild).trim(u64::MAX, false),
            Some(crate::TRIM_DB)
        );
        let wild = Settings {
            preferences: crate::RackPreferences {
                output_gain_db: f32::NAN,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(Runtime::new(wild).trim(u64::MAX, false), None);
    }
    /// The IN jack's trim, kept the same way one jack over: it rides the card
    /// trims' map under the capture node's id, comes back out of the file on
    /// resume, clamps at the ends of the track and takes no `NaN` record —
    /// the same four rules [`Runtime`] already keeps for the master trim,
    /// checked here without a single card or backend.
    #[test]
    fn the_input_trim_comes_back_with_the_rack() {
        let file = frost::scratch::claim("rack-input-gain").join("nice-rack-settings.ron");
        let _ = std::fs::remove_file(&file);

        let mut runtime = Runtime::new(Settings::default());
        settle(&mut runtime);
        assert_eq!(
            runtime.trim(0, true),
            Some(0.0),
            "every file states an input gain, defaulted or not, so even a \
             rack as new as this file holds its capture record, at unity"
        );
        runtime.persistence_path = Some(file.clone());

        // The same ordinary debounce as the master's: nothing else about the
        // rack moved, so it is the input trim alone that rewrites the file.
        runtime.set_trim(0, true, 3.0);
        runtime.last_save = Instant::now() - Duration::from_secs(3);
        runtime.tick();
        let (saved, warning) = Settings::load_from(&file);
        assert_eq!(warning, None);
        assert!(saved.preferences.plugins.is_empty(), "no cards, still");
        assert_eq!(saved.preferences.input_gain_db, 3.0);
        assert_eq!(saved.preferences.output_gain_db, 0.0, "one jack moved");

        // A resume: the number comes out of the file and into the same map a
        // card's trim arrives in, under the capture's own key.
        let resumed = Runtime::new(saved);
        assert_eq!(resumed.trim(0, true), Some(3.0));

        // And the same two rules as everywhere else on the track.
        let wild = Settings {
            preferences: crate::RackPreferences {
                input_gain_db: -900.0,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(Runtime::new(wild).trim(0, true), Some(-crate::TRIM_DB));
        let wild = Settings {
            preferences: crate::RackPreferences {
                input_gain_db: f32::NAN,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(Runtime::new(wild).trim(0, true), None);
    }
    /// What a remembered break is made of, checked without a plugin: the file
    /// says which of a card's two cables it left broken, and the controller
    /// turns that into the set the GUI asks. The two sides are keyed apart the
    /// way the trims are, because they are different cables — breaking the wire
    /// into a card does not break the wire out of it, and a record that could
    /// not tell them apart could not be trusted to draw the gap in the right
    /// place.
    #[test]
    fn a_remembered_break_names_the_cable_and_not_the_card() {
        let saved = crate::SavedPlugin {
            id: 7,
            stage: 1,
            branch: None,
            span: None,
            path: "/plugins/tuner.clap".into(),
            class: "same-class".into(),
            name: "Tuner".into(),
            state: None,
            editor: None,
            gain_in: 0.0,
            gain_out: 0.0,
            open_in: true,
            open_out: false,
            bypassed: false,
            paused: false,
        };
        let opens = remembered_opens(std::slice::from_ref(&saved));
        assert!(opens.contains(&(7, false)), "the side the file names");
        assert!(!opens.contains(&(7, true)), "and only that side");
        assert_eq!(opens.len(), 1, "a closed cable is no record at all");

        // A file from before the feature names nothing, which is the empty set
        // and not a rack full of silent wires.
        let before = crate::SavedPlugin {
            open_in: false,
            ..saved
        };
        assert!(remembered_opens(&[before]).is_empty());

        // And the controller's own copy answers the same way the file does,
        // including on the way back to closed.
        let mut runtime = Runtime::new(Settings::default());
        assert!(
            !runtime.open_circuit(7, false),
            "a fresh rack breaks nothing"
        );
        runtime.set_open_circuit(7, false, true);
        assert!(runtime.open_circuit(7, false));
        assert!(!runtime.open_circuit(7, true));
        runtime.set_open_circuit(7, false, false);
        assert!(!runtime.open_circuit(7, false));
    }
    /// A window that stopped answering the moment a second plugin was dropped
    /// on it, with a backend chosen in Settings and the devices therefore
    /// open, is not a crashed program: every interaction the rack offers, the
    /// drop queue first of all, waits for the controller to stop being busy,
    /// so a controller that never settles is a window that answers nothing.
    /// This is the only test in the tree that hands the controller real
    /// devices instead of the dummy backend, and it opens whatever this
    /// machine has, which is why it stays opt-in. What it requires is not
    /// that this machine's audio driver answers, but that the controller
    /// always comes to rest and says why when it does not. The second file is
    /// opened with the first chain already processing, because that is the
    /// moment the report names.
    ///
    /// `NICE_RACK_TEST_LINGER` holds the process open that many milliseconds
    /// after the devices are at rest, because the answer it gives is not about
    /// this test: a run that prints the linger line and exits cleanly released
    /// this machine's ASIO driver safely, and a run that stops during the
    /// linger, after every assertion here passed, died in
    /// `nativeinstrumentsusbaudioasio_x` at an address the loader had already
    /// unmapped — the driver's own thread executing after its library was
    /// released, which nothing on this side can order.
    #[test]
    #[ignore = "opens this machine's real audio devices; needs NICE_RACK_TEST_CLAP"]
    #[allow(unsafe_code)]
    fn a_second_drop_while_the_devices_are_open_settles_again() {
        let path = PathBuf::from(
            std::env::var_os("NICE_RACK_TEST_CLAP").expect("Set NICE_RACK_TEST_CLAP"),
        );
        // SAFETY: explicit test opt-in to this repository's own plugin export.
        let entry = unsafe { nice_clap_host::Entry::load(&path).unwrap() };
        let class = entry
            .descriptors()
            .unwrap()
            .first()
            .expect("the test export contains at least one plugin")
            .id
            .clone();
        let first = Card::new(
            NodeId(2),
            path.clone(),
            entry,
            class.clone(),
            "First plugin".into(),
        )
        .unwrap();
        let second_path = std::env::var_os("NICE_RACK_TEST_CLAP_2")
            .map(PathBuf::from)
            .unwrap_or_else(|| path.clone());

        // The report is about the rack as it is actually run, on the devices
        // that were actually chosen, so the settings come from the file the
        // rack writes.
        let settings = rack_settings();
        let mut runtime = Runtime::new(settings);
        settle(&mut runtime);
        // A driver may refuse this machine for a reason of its own — no input,
        // no such driver — and a refusal is a message, which is the behaviour
        // under test as much as an opening is.
        let opened = runtime.audio_running();
        mark(&format!("devices asked for, audio = {opened}"), &runtime);

        runtime.edit(vec![first]);
        settle(&mut runtime);
        mark("first card applied", &runtime);
        assert_eq!(runtime.cards.len(), 1, "{}", runtime.message);

        // Isolate: a second drop rebuilds the chain, and this asks whether the
        // rebuild alone is enough, with the same one card in it and no second
        // library opened at all. Six of them, because the failure may be one
        // teardown in particular or simply the nth one.
        for lap in 2..=7 {
            let again = runtime.cards.clone();
            runtime.edit(again);
            settle(&mut runtime);
            mark(
                &format!("chain rebuilt with the same card, lap {lap}"),
                &runtime,
            );
        }

        // The report is a drop made while the first chain was already making
        // sound, so this is where the second file is opened: a native library
        // loaded, its plugins asked for, one of them instantiated, all of it
        // with the audio thread inside the first plugin. Opened here, on this
        // thread, because it is where the rack opens a dropped file: the event
        // loop drains the queue on the thread that draws, and a plugin loaded
        // somewhere else is not what the application does.
        // SAFETY: explicit test opt-in to this repository's own plugin export.
        let entry = unsafe { nice_clap_host::Entry::load(&second_path).unwrap() };
        let second_class = entry
            .descriptors()
            .unwrap()
            .first()
            .expect("the second test export contains at least one plugin")
            .id
            .clone();
        let second = Card::new(
            NodeId(4),
            second_path,
            entry,
            second_class,
            "Second plugin".into(),
        )
        .unwrap();
        mark("second plugin opened", &runtime);

        let mut cards = runtime.cards.clone();
        cards.push(second);
        runtime.edit(cards);
        settle(&mut runtime);
        mark("second card applied", &runtime);
        assert_eq!(runtime.cards.len(), 2, "{}", runtime.message);
        if opened {
            assert!(
                runtime.audio_running(),
                "the second drop took the audio down: {}",
                runtime.message
            );
        }

        mark("asking for the devices to close", &runtime);
        runtime.close();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !runtime.stopped() {
            runtime.tick();
            assert!(
                Instant::now() < deadline,
                "the devices never came to rest: {}",
                runtime.message
            );
            std::thread::yield_now();
        }
        mark("devices at rest", &runtime);
        // Where does the process die? The close above hands the streams to a
        // thread which releases the devices, and a driver whose own threads
        // outlive its module faults when it likes, in a library the loader has
        // already unmapped. Lingering tells the two apart from the output
        // alone: a run that stops here died with the driver it had just
        // released, a run that prints the last line died on its way out.
        let linger = std::env::var("NICE_RACK_TEST_LINGER")
            .ok()
            .and_then(|text| text.parse::<u64>().ok())
            .unwrap_or(0);
        if linger > 0 {
            std::thread::sleep(Duration::from_millis(linger));
            println!("  + linger of {linger} ms over");
        }
    }
    /// The rack writes what it holds into its settings file, so a chain that is
    /// still standing in the rack is what gets written out. A retreat that
    /// aimed at the state `Runtime::new` started from held nothing, and the
    /// save two seconds later wrote an empty rack over a saved one: a chain the
    /// user never removed went with a driver that had simply refused to open.
    /// This is the case a rack meets on a machine where another program holds
    /// the interface — this machine's ASIO driver belongs to REAPER while it
    /// runs — and it is the reason the rack came back empty after a launch that
    /// had said nothing but an error about devices. The chain the refused
    /// opening came with is what the rack keeps, on the devices it can name:
    /// none.
    #[test]
    #[ignore = "requires NICE_RACK_TEST_CLAP pointing to a freshly built repository CLAP export"]
    #[allow(unsafe_code)]
    fn a_refused_opening_keeps_the_chain_it_was_started_with() {
        let path = PathBuf::from(
            std::env::var_os("NICE_RACK_TEST_CLAP").expect("Set NICE_RACK_TEST_CLAP"),
        );
        // SAFETY: explicit test opt-in to this repository's own plugin export.
        let entry = unsafe { nice_clap_host::Entry::load(&path).unwrap() };
        let class = entry
            .descriptors()
            .unwrap()
            .first()
            .expect("the test export contains at least one plugin")
            .id
            .clone();
        let card = Card::new(
            NodeId(1),
            path.clone(),
            entry,
            class.clone(),
            "Test plugin".into(),
        )
        .unwrap();

        // A backend no host answers: the opening refuses on every machine, in
        // the same way a driver that is busy refuses, and no device of this one
        // is touched on the way.
        let mut settings = rack_settings();
        settings.backend = Some("no-such-backend".into());
        settings.input_device = None;
        settings.output_device = None;

        let file = frost::scratch::claim("rack-refused-opening").join("nice-rack-settings.ron");
        let _ = std::fs::remove_file(&file);

        // How a rack starts: the settings file, then the chain restored into the
        // action that was built from them.
        let mut runtime = Runtime::new(settings);
        runtime.restore(vec![card], BTreeMap::new());
        runtime.persistence_path = Some(file.clone());
        settle(&mut runtime);
        mark("the opening refused", &runtime);
        assert!(!runtime.audio_running());
        assert_eq!(runtime.cards.len(), 1, "{}", runtime.message);

        runtime.last_save = Instant::now() - Duration::from_secs(3);
        runtime.tick();
        let (saved, warning) = Settings::load_from(&file);
        assert_eq!(warning, None);
        assert_eq!(
            saved.preferences.plugins.len(),
            1,
            "the refused opening wrote an empty rack over the saved chain: {}",
            runtime.message
        );
        assert_eq!(saved.preferences.plugins[0].class, class);
        assert!(!runtime.busy());

        runtime.close();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !runtime.stopped() {
            runtime.tick();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    /// The control for the case above: the same two plugins, the same devices,
    /// the same rebuilds, with both libraries opened before the devices are
    /// asked for, so no code is loaded while a driver is streaming. If this
    /// passes where the drop-while-running dies, nothing about the plugins is
    /// at fault: it is the act of loading a library into a process whose audio
    /// driver is live that the driver cannot take.
    #[test]
    #[ignore = "opens this machine's real audio devices; needs NICE_RACK_TEST_CLAP"]
    #[allow(unsafe_code)]
    fn two_plugins_opened_before_the_devices_come_up() {
        let first_path = PathBuf::from(
            std::env::var_os("NICE_RACK_TEST_CLAP").expect("Set NICE_RACK_TEST_CLAP"),
        );
        let second_path = std::env::var_os("NICE_RACK_TEST_CLAP_2")
            .map(PathBuf::from)
            .unwrap_or_else(|| first_path.clone());
        let mut runtime = Runtime::new(rack_settings());
        let cards = [first_path, second_path]
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                // SAFETY: explicit test opt-in to this repository's own plugin export.
                let entry = unsafe { nice_clap_host::Entry::load(&path).unwrap() };
                let class = entry
                    .descriptors()
                    .unwrap()
                    .first()
                    .expect("the test export contains at least one plugin")
                    .id
                    .clone();
                Card::new(
                    NodeId(2 + index as u64),
                    path,
                    entry,
                    class,
                    format!("Plugin {}", index + 1),
                )
                .unwrap()
            })
            .collect();
        mark("both plugins opened, devices not asked for", &runtime);

        runtime.edit(cards);
        settle(&mut runtime);
        mark("two cards applied", &runtime);

        let again = runtime.cards.clone();
        runtime.edit(again);
        settle(&mut runtime);
        mark("chain rebuilt with both cards", &runtime);

        runtime.close();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !runtime.stopped() {
            runtime.tick();
            assert!(
                Instant::now() < deadline,
                "the devices never came to rest: {}",
                runtime.message
            );
            std::thread::yield_now();
        }
        mark("devices at rest", &runtime);
    }
    /// The device settings of the rack as this machine runs it.
    /// `NICE_RACK_TEST_SETTINGS` names the file and the
    /// `nice-rack-settings.ron` beside the working directory stands in, because
    /// a hand-written default asks for an input channel this machine may have
    /// no default of, and a controller answering "Select an input channel" has
    /// no chain to test at all. `NICE_RACK_TEST_BACKEND` overrides the backend
    /// and drops the saved device names with it, so that host picks its own
    /// defaults. The saved plugin list goes: the drops in these tests are the
    /// only way a card should reach the rack.
    fn rack_settings() -> Settings {
        let mut settings = match std::env::var_os("NICE_RACK_TEST_SETTINGS") {
            Some(file) => Settings::load_from(&PathBuf::from(file)).0,
            None => Settings::load_from(&Settings::path()).0,
        };
        if let Some(backend) = std::env::var_os("NICE_RACK_TEST_BACKEND") {
            settings.backend = Some(backend.to_string_lossy().into_owned());
            settings.input_device = None;
            settings.output_device = None;
        }
        if settings.backend.is_none() {
            // A rack that saved no backend still has to name one the backend
            // rules accept, and on Windows that is ASIO: every other key is
            // refused at resolve there, so naming the system host here would
            // hand these tests a settings file their own code rejects.
            let backend = if cfg!(windows) {
                "asio"
            } else if cfg!(target_os = "macos") {
                "core-audio"
            } else {
                "alsa"
            };
            settings.backend = Some(backend.into());
        }
        settings.preferences.plugins.clear();
        settings
    }
    /// Where the run got to and what the controller said there. A process that
    /// dies inside a third-party driver takes its test report down with it, so
    /// the last line left on the console is the only record of the call that
    /// did it; the elapsed time says which step was slow before it was fatal.
    fn mark(step: &str, runtime: &Runtime) {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let start = START.get_or_init(Instant::now);
        eprintln!(
            "  +{:>6} ms  {step}  |  {}",
            start.elapsed().as_millis(),
            runtime.message
        );
    }
    fn settle(runtime: &mut Runtime) {
        // Generous because a real driver is allowed to take its time: the
        // deadline only says how long the test waits before calling a
        // controller that never comes to rest stuck.
        let deadline = Instant::now() + Duration::from_secs(10);
        while runtime.busy() {
            runtime.tick();
            assert!(
                Instant::now() < deadline,
                "the controller never settled: {}",
                runtime.message
            );
            std::thread::yield_now();
        }
    }
    #[test]
    fn recovery_retries_are_bounded_and_keep_the_failure_text() {
        let mut runtime = Runtime::new(Settings::default());
        runtime.action = None;
        runtime.retries = 3;
        runtime.recover("Device sample rate changed".into(), None);
        assert!(runtime.action.is_none());
        assert!(runtime.retry.is_none());
        assert_eq!(runtime.message, "Device sample rate changed");
    }
}

#[cfg(test)]
mod card_mode_tests {
    use super::*;
    #[test]
    fn paused_and_bypassed_restore_independently_from_saved_cards() {
        let saved: crate::SavedPlugin = saved_card();
        let mut settings = Settings::default();
        settings.preferences.plugins.push(saved.clone());
        let mut runtime = Runtime::new(settings);
        assert!(runtime.paused(7));
        assert!(runtime.bypassed(7));
        runtime.set_bypassed(7, false);
        assert!(runtime.paused(7));
        assert!(!runtime.bypassed(7));
        runtime.set_paused(7, false);
        assert!(!runtime.paused(7));
        let mut before = saved;
        before.paused = false;
        before.bypassed = false;
        let settings = Settings {
            preferences: crate::RackPreferences {
                plugins: vec![before],
                ..Default::default()
            },
            ..Default::default()
        };
        let runtime = Runtime::new(settings);
        assert!(!runtime.paused(7) && !runtime.bypassed(7));
    }
    fn saved_card() -> crate::SavedPlugin {
        crate::SavedPlugin {
            id: 7,
            stage: 7,
            branch: None,
            span: None,
            path: "/plugins/test.clap".into(),
            class: "test".into(),
            name: "Test".into(),
            state: None,
            editor: None,
            gain_in: 3.0,
            gain_out: -2.0,
            open_in: false,
            open_out: false,
            bypassed: true,
            paused: true,
        }
    }
}
