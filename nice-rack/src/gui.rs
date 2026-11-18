mod timing;
use crate::{
    Settings,
    layout::{Side, Target},
    routing::NodeId,
    runtime::{Card, Runtime},
};
use frost::{Color, Context, Process, Scene, SceneNode, Shape};
use nice_clap_host::{Descriptor, Entry};
use nice_common::{
    meter::PeakMeter,
    ui::{AudioPanel, Event},
};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
};
use timing::TimingMeter;

type Loader = Box<dyn FnMut(&Path) -> Result<Entry, String>>;
struct Choosing {
    entry: Entry,
    path: PathBuf,
    descriptors: Vec<Descriptor>,
    placement: Option<Target>,
}
struct Dropped {
    path: PathBuf,
    point: Option<[f32; 2]>,
    captured: bool,
}
/// One wire's meter: its peak-hold, the gain trim (dB) the user sets by
/// dragging the expanded meter as a slider, and whether the meter is
/// expanded (`open`) or minimised to the stub box on the wire. Closing
/// collapses the track only — the peak-hold keeps ticking, so the stub
/// can show activity by colour, and the dB readout stands on. The
/// peak-hold latches: once it has read over full scale it stays red until
/// the meter is clicked, because in a rack the meter that caught the clip
/// is often not the one the operator is looking at. The engine keeps its
/// own linear copy so the trim survives a closed meter and rack rebuilds.
struct WireMeter {
    peak: PeakMeter,
    gain_db: f32,
    /// Whether the track is expanded. The rack's word for an opened meter.
    open: bool,
    /// Whether the operator left this wire as an open circuit, so that the
    /// signal stops at the stub box. Called `open_circuit` and not `open`
    /// because in this file `open` already means a meter someone expanded,
    /// and the two have nothing to do with each other: a collapsed track can
    /// be carrying nothing and an expanded one can be broken.
    open_circuit: bool,
}
struct RackUi {
    runtime: Runtime,
    loader: Loader,
    font: Arc<[u8]>,
    icons: Arc<[u8]>,
    settings: Option<AudioPanel<crate::RackPreferences>>,
    settings_page: bool,
    pressed: bool,
    /// Last frame's right button, so a break toggles on the press and not on
    /// every frame the button is held.
    pressed_right: bool,
    drag: Option<crate::routing::NodeId>,
    scroll: f32,
    drops: VecDeque<Dropped>,
    external_drag: bool,
    vertical: f32,
    choosing: Option<Choosing>,
    dropdown_scroll: f32,
    dropdown_drag: bool,
    next_id: u64,
    input: PeakMeter,
    output: PeakMeter,
    /// Per-wire meter state, keyed `(plugin id, is_output)` — the same key
    /// the engine registers its wire slots under (`id << 1 | side`). Every
    /// wire the rack carries has an entry here from the frame its card
    /// appeared, which is `watch_wires`'s job: its dB readout stands in the
    /// card's corner whether or not the meter is expanded, and its peak-hold
    /// keeps ticking either way, so the minimised stub can colour with the
    /// signal — including red, before anyone has clicked it once. Clicking
    /// the stub only collapses or re-expands the track. The gain trim itself
    /// also lives in the engine's slot table, so it survives closing the
    /// meter; this copy is what the white line and the readout draw, and
    /// what gets re-stated to the engine every tick. An entry outlives its
    /// card: ids are never handed out twice, so a leftover meter can only
    /// ever read silence.
    wire_meters: std::collections::BTreeMap<(u64, bool), WireMeter>,
    timing_meters: std::collections::BTreeMap<NodeId, TimingMeter>,
    /// Gain slider grab: `(plugin id, is_output, pointer height last
    /// frame)`. The drag is *relative*: each frame's pointer travel is
    /// what moves the trim, so the grab tracks the pointer, and the rack
    /// can scroll mid-drag without the gain jumping.
    gain_drag: Option<(u64, bool, f32)>,
    /// Open dB editor: `(plugin id, is_output)`. Clicking a card-corner
    /// readout edits that trim in place; the field's rect is recomputed
    /// from the card's live slot every frame, so scrolling carries the
    /// open editor along with its card.
    gain_edit: Option<(u64, bool)>,
    /// The editor's field: frost's `TextInput` logic — buffer, caret,
    /// selection, blink — adopted from the trim's dB text when the edit
    /// opens. The rack draws it itself at the card's corner, mirroring
    /// frost's own field drawing at the `GAIN_` metrics.
    gain_input: frost::TextInput,
    size: Option<(u32, u32)>,
    windows: crate::editors::Windows,
    overlay: Option<NodeId>,
    parameter_scroll: f32,
    press_origin: Option<[f32; 2]>,
    moved: bool,
}
const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}
/// What a *level* is drawn in: the bar of an expanded track, the `main` both
/// master meters are drawn with — and so, through the shared widget's
/// `MeterColors`, their bars too. It is the house green, named once here
/// instead of repeating the same three numbers wherever a level gets a
/// rectangle, because a minimised meter colours itself with it (see
/// [`stub_color`]) and the two colours have to stay one colour: a stub that
/// brightens is saying what its own bar would say if it were drawn.
const LEVEL: Color = nice_common::knob::GREEN;
fn hit(mouse: Option<[f32; 2]>, x: f32, y: f32, w: f32, h: f32) -> bool {
    mouse.is_some_and(|p| (p[0] - x).abs() <= w && (p[1] - y).abs() <= h)
}
/// Half-width of a stub box, in pixels. It is the width the box is drawn at,
/// the width a click is accepted over, and the distance a broken wire's line
/// stops short of its box, all from one number — so the gap a break opens can
/// never be a hair narrower or wider than the box that marks it.
const STUB_HALF: f32 = 4.0;
/// Half-height of a stub box. Taller than wide so the box reads as a terminal
/// on the wire rather than a dot on it.
const STUB_TALL: f32 = 6.0;
/// How far a stub box stands off the card's vertical edge.
const STUB_GAP: f32 = 6.0;
/// How much further out the columns are that a parallel stage's risers stand
/// in — the split on the left and the merge on the right. The gap is what
/// leaves room for a card's own short run of wire between its stub box and
/// the column its stage shares with the other cards in it, which is the only
/// part of a parallel stage's wiring that belongs to that one card.
const JUNCTION_GAP: f32 = 12.0;
/// How far above and below a wire a right-click still counts as aimed at it.
/// The line itself is 2 px wide, so without some generosity here the operator
/// would have to be exact about a target they cannot make much wider.
const BREAK_HALF: f32 = 5.0;
/// How far a broken wire's line pulls back from its stub box, on top of the
/// box's own half-width. The box is 8 px across and sits between the card and
/// the column its wire runs to, so stopping the line at the box's face would
/// leave a gap barely wider than the mark inside it. This is the air that makes
/// an open circuit read as a hole in the wire and not as a box painted a
/// different colour.
const BREAK_GAP: f32 = 4.0;
/// How much of a card's run is given up at an end that stands in a column more
/// than one wire passes through. The risers of a parallel stage carry every card
/// in it and are nobody's to break, and where a riser crosses a card's own row
/// the two are the same pixels; two cards meeting in a merge column are the same
/// overlap in miniature. Stepping back hands the column to the wire that ends
/// there, so the operator can aim at it and never open two circuits at once.
const RISER_KEEP: f32 = 3.0;
fn wire_box_x(x: f32, side: bool) -> f32 {
    // Stub box centres for one card: `STUB_GAP` out from each vertical card
    // edge, on the input wire for `side == false` and the output wire
    // otherwise.
    let edge = CARD_WIDTH / 2.0 + STUB_GAP;
    if side { x + edge } else { x - edge }
}
/// The column a stage's risers stand in — where its wires split out to the
/// cards of a parallel stage and merge back together.
fn junction_x(x: f32, side: bool) -> f32 {
    let edge = CARD_WIDTH / 2.0 + JUNCTION_GAP;
    if side { x + edge } else { x - edge }
}
/// The stretch of wire that belongs to one side of one card: the signal's
/// whole journey between the card's edge and the junction it reaches, so that
/// clicking anywhere along it means the same thing — this card's input, or
/// this card's output. [`wire_click`] turns it into a target no two wires can
/// share.
///
/// Where the stage has only this card, the junction is the previous stage's
/// merge column and the next one's split, so the input stretch takes in the
/// entire connector between the two cards' meters: the wire the operator
/// actually sees, from an output meter to an input meter. The output stretch
/// ends at its own merge column, because the connector beyond it is the next
/// card's input — except in the last stage, where there is no next card and the
/// stretch runs on into the rack's `OUT` jack. Where the stage holds several
/// cards in parallel, both junction columns and the risers between them belong
/// to the stage and not to any one card, so each card keeps only its short run
/// beside its stub box — and the vertical connections are never anybody's to
/// break.
///
/// `feed` is the column the wire arrives from on the left (the head feed's `IN`
/// jack for the first stage) and `out` the column it leaves for on the right
/// (the `OUT` jack's run for the last stage). Each is ignored where the other
/// side owns that stretch instead: a shared stage's junction columns, or the
/// next card's input.
///
/// The intervals here meet at the columns and may touch there; [`wire_click`]
/// is what decides which wire a touch belongs to. The drawing of a wire is a
/// separate matter — the parts of a stage's wiring that cross rows are drawn one
/// row at a time, each cut at its own height — so nothing here is used to put ink
/// down.
fn wire_run(x: f32, side: bool, shared: bool, feed: f32, out: f32) -> (f32, f32) {
    let edge = CARD_WIDTH / 2.0;
    if side {
        (x + edge, if shared { junction_x(x, true) } else { out })
    } else {
        (if shared { junction_x(x, false) } else { feed }, x - edge)
    }
}
/// [`wire_run`] as a click target: the same stretch, minus the few pixels at an
/// end that stands in a column two wires meet in.
///
/// A card's output finishes in a column, and a stage's input starts in one — its
/// own riser where the stage is shared, otherwise the previous stage's merge
/// column. Wherever that is, the column itself is given to the wire that ends
/// there and the wire that starts there steps back by [`RISER_KEEP`], so a click
/// opens exactly one circuit: aiming at a riser hits neither of the wires that
/// pass it, and aiming where two cards' meters hand the signal over breaks the
/// one whose run finishes there. The two ends that stand at one of the rack's
/// own jacks — the head feed on the left, the last card's run into `OUT` on the
/// right — are nobody's second wire, and are kept in full.
fn wire_click(x: f32, side: bool, shared: bool, fed: bool, feed: f32, out: f32) -> (f32, f32) {
    let (from, to) = wire_run(x, side, shared, feed, out);
    if side {
        (from, if shared { to - RISER_KEEP } else { to })
    } else {
        (
            if shared || fed {
                from + RISER_KEEP
            } else {
                from
            },
            to,
        )
    }
}
/// Where a stretch of wire stops when the switch on it is open: short of its
/// stub box, with [`BREAK_GAP`] of air between the two. Closed, it runs to the
/// column it was drawn to anyway.
///
/// The far side of the hole is simply not drawn, which is what makes a break
/// read as an open circuit: a wire with nothing beyond it is the same inkless
/// nothing as a wire nobody has connected, and the box standing alone in the
/// gap says which of the two it is.
fn wire_end(to: f32, box_x: f32, open: bool) -> f32 {
    if open {
        (box_x - STUB_HALF - BREAK_GAP).min(to)
    } else {
        to
    }
}
/// A stretch of wire, if there is one left to draw after the cut above.
fn wire_segment(ctx: &mut Context, from: f32, to: f32, y: f32, color: Color) {
    if from <= to {
        ctx.line(from, y, to, y, color, 2.0, 0.1);
    }
}
/// Whether a pointer is on a wire: anywhere along the stretch it owns within
/// [`BREAK_HALF`] of the line, or on its stub box, which is the part that is
/// always this wire's and nobody else's.
fn wire_hit(mouse: Option<[f32; 2]>, from: f32, to: f32, y: f32, box_x: f32) -> bool {
    hit(mouse, box_x, y, STUB_HALF, STUB_TALL)
        || mouse.is_some_and(|p| (from..=to).contains(&p[0]) && (p[1] - y).abs() <= BREAK_HALF)
}
/// A meter body outranks a cable hit. Collapsed meters only own their stub;
/// expanded meters also own the full vertical track outside the card.
fn meter_hit(mouse: Option<[f32; 2]>, x: f32, y: f32, side: bool, expanded: bool) -> bool {
    hit(
        mouse,
        wire_box_x(x, side),
        y,
        if expanded { 4.5 } else { STUB_HALF },
        if expanded { TRACK_HALF } else { STUB_TALL },
    )
}
fn card_lead(x: f32, side: bool, split: f32, merge: f32, bypassed: bool) -> (f32, f32) {
    let endpoint = if bypassed {
        wire_box_x(x, side)
    } else {
        x + if side {
            CARD_WIDTH / 2.0
        } else {
            -CARD_WIDTH / 2.0
        }
    };
    if side {
        (endpoint, merge)
    } else {
        (split, endpoint)
    }
}
fn bypass_path(x: f32, y: f32, clearance: f32) -> [[f32; 2]; 4] {
    let left = wire_box_x(x, false);
    let right = wire_box_x(x, true);
    [
        [left, y],
        [left, y + TRACK_HALF + clearance],
        [right, y + TRACK_HALF + clearance],
        [right, y],
    ]
}
const TRACK_HALF: f32 = 56.0;
/// Relative drag mapping: each pixel of pointer travel turns the trim by
/// 0.05 dB, and that rate grows exponentially with the drag's speed (px
/// per frame), so a slow, careful drag keeps fractions-of-a-dB control
/// while a fast flick sweeps the range in a swipe. The result clamps to
/// the ±[`crate::TRIM_DB`] dB the track spans.
fn gain_drag_delta(px: f32, gain_db: f32) -> f32 {
    (gain_db + px * 0.05 * (px.abs() / 8.0).exp()).clamp(-crate::TRIM_DB, crate::TRIM_DB)
}
/// Parse the editor's draft: a comma stands in for the decimal point,
/// and anything that is not a plain, finite dB value is no value at all.
fn parse_gain(text: &str) -> Option<f32> {
    text.trim()
        .replace(',', ".")
        .parse::<f32>()
        .ok()
        .filter(|db| db.is_finite())
        .map(|db| db.clamp(-crate::TRIM_DB, crate::TRIM_DB))
}
/// The track height a gain dB sits at — the inverse mapping the white
/// gain line is drawn from.
fn gain_line_y(gain_db: f32, wire_y: f32) -> f32 {
    wire_y - TRACK_HALF
        + 2.0 * TRACK_HALF * ((gain_db + crate::TRIM_DB) / (2.0 * crate::TRIM_DB)).clamp(0.0, 1.0)
}
/// The trim a wire shows *before* its meter exists: the engine's own
/// linear factor in dB, over the ±[`crate::TRIM_DB`] dB the track spans, so
/// the number in the card's corner is the gain the wire is actually applying
/// and not a placeholder. A key the slot table does not hold is unity, and
/// unity reads `+0.0`.
fn engine_gain_db(linear: Option<f32>) -> f32 {
    linear.map_or(0.0, |gain| {
        (20.0f32 * gain.log10()).clamp(-crate::TRIM_DB, crate::TRIM_DB)
    })
}
/// What a card's corner reads: the trim the rack is applying on that wire,
/// and whether the meter there is over full scale — the clip travels with
/// the reading, so a corner turns red together with the track it belongs
/// to, and not one element of a meter at a time.
///
/// The number comes out of the rack's own state: the wire's meter where the
/// rack has one, the engine's slot table where it has none, and the two are
/// kept in step because every commit writes both. Nothing here is asked of
/// a plugin, so the readout stands on a card for a plugin the rack knows
/// nothing about beyond its name and its pin count. `open` is not an input
/// at all: a collapsed meter and a wire whose stub was never clicked read
/// the same number as an expanded one.
fn corner_reading(wire: Option<&WireMeter>, engine_linear: Option<f32>) -> (f32, bool) {
    match wire {
        Some(meter) => (meter.gain_db, meter.peak.clipping()),
        None => (engine_gain_db(engine_linear), false),
    }
}
/// Where a meter born on this wire opens: the trim the rack remembered for it,
/// if the rack remembered one, otherwise the gain the engine is applying, and
/// unity where it has no word from either. The remembered value comes first
/// because on a resumed rack the audio has not run — the slot table holds no
/// key until the chain reaches the audio side — so a meter that opened at the
/// engine would open at unity and then, a frame later, be pushed to the
/// operator's own number by the tick pass that re-states every meter. Reading
/// the file first is what lets a resumed rack show the trim it was left at
/// before it has made a sound.
fn remembered_trim(remembered: Option<f32>, engine_linear: Option<f32>) -> f32 {
    remembered.unwrap_or_else(|| engine_gain_db(engine_linear))
}
/// A meter for one wire, born collapsed at a trim already known in dB. It is
/// built latching: a clip it catches stays on the stub until somebody clicks it
/// away, which is the point of a meter nobody is watching.
fn new_wire_meter(gain_db: f32, time: u64) -> WireMeter {
    WireMeter {
        peak: PeakMeter::latching(time),
        gain_db,
        open: false,
        open_circuit: false,
    }
}
/// What colour a stub box is drawn in, for a wire whose meter may or may not
/// exist yet.
///
/// A clip outranks the rest: a wire that went over full scale stays red until
/// somebody acknowledges it, whatever it is doing now, because the one thing a
/// meter nobody is watching has to survive is the peak it was not watching for.
///
/// Everything else bright is one colour, [`LEVEL`], the green the rack's meters
/// draw their level bars in. An expanded track has always worn it: there the
/// colour says "the meter is out" and the bar beside it says how much. A
/// collapsed meter joins it at or above the rack's meter gate, because signal
/// coming through this wire right now is worth knowing on a canvas of stubs
/// nobody has opened — and it is worth knowing *in the meter's own colour*, so
/// that an operator who has seen a green bar on an opened track reads a green
/// stub the same way, with nothing new to learn. Borrowing the stub a colour of
/// its own would have made the rack say one thing two ways.
///
/// Strictly below the gate, the stub keeps the plain wire colour it has always
/// worn, so a rack at rest is as quiet as it looks. The live bar's level is the
/// one compared, not the held peak: the gate asks what is passing now, and a
/// peak the release long since walked down would keep a wire bright for a second
/// after the sound stopped.
///
/// A wire the operator left open sits between the clip and the level, in amber.
/// It is not a report about the audio — nothing is coming through, so the gate
/// has nothing to say about it — it is a control someone moved, and amber is
/// what this rack already wears for that: [`nice_common::knob::AMBER`] is the
/// colour a knob takes on once it is off its default, and the rack's knobs
/// define the colour by being touched, not by being loud. Reading a stub amber
/// therefore costs the operator nothing new, and it outranks green because a
/// silent stub that is silent *by hand* must not be mistaken for one that is
/// silent because nothing is playing. The clip still outranks the break: a wire
/// broken after it clipped has both to report, and the peak nobody has read yet
/// is the one that cannot be allowed to disappear under a colour the operator
/// chose themselves.
fn stub_color(stub: Option<&WireMeter>, gate_db: f32, wire: Color) -> Color {
    if stub.is_some_and(|meter| meter.peak.clipping()) {
        nice_common::knob::RED
    } else if stub.is_some_and(|meter| meter.open_circuit) {
        nice_common::knob::AMBER
    } else if stub.is_some_and(|meter| meter.open || meter.peak.level_db() >= gate_db) {
        LEVEL
    } else {
        wire
    }
}
/// Put a meter on every wire the rack carries, keyed the way the engine keys
/// its slots. Meters are *born* here and not at the first click on a stub:
/// peak detection may not wait for someone to open a track, because a stub
/// with no meter behind it has nothing to colour itself with and reads green
/// through any amount of clipping.
///
/// A meter is also born knowing whether its wire is an open circuit, for the
/// same reason the trim is read from the rack and not from the engine: on a
/// resumed rack the audio has not run yet, so the slot table holds no key, and
/// a stub that asked the engine would draw one frame of unbroken wire before
/// the tick pass re-stated the break.
///
/// An entry that is already there is left exactly where the operator left it
/// — the trim they set, whether the track is expanded, whether the wire is
/// broken, and a clip they have not acknowledged — so this is called every
/// frame and changes nothing except for the wires that are new.
fn watch_wires(
    meters: &mut std::collections::BTreeMap<(u64, bool), WireMeter>,
    ids: impl IntoIterator<Item = u64>,
    mut remembered: impl FnMut(u64, bool) -> Option<f32>,
    mut gain_of: impl FnMut(u64) -> Option<f32>,
    mut open_of: impl FnMut(u64, bool) -> bool,
    time: u64,
) {
    for id in ids {
        for side in [false, true] {
            let key = (id, side);
            if meters.contains_key(&key) {
                continue;
            }
            let gain_db = remembered_trim(remembered(id, side), gain_of(id << 1 | u64::from(side)));
            meters.insert(
                key,
                WireMeter {
                    open_circuit: open_of(id, side),
                    ..new_wire_meter(gain_db, time)
                },
            );
        }
    }
}
fn short(text: &str, count: usize) -> String {
    if text.chars().count() <= count {
        text.into()
    } else {
        format!(
            "{}…",
            text.chars()
                .take(count.saturating_sub(1))
                .collect::<String>()
        )
    }
}
const CARD_WIDTH: f32 = 186.0;
/// The corner readout's type size. frost's `TextInput` lays its cursor,
/// clicks and caret out in its own metrics — a 0.6 em advance, padded
/// 10 px, 16 visible columns — and the rack's field reuses that logic
/// as it is, so the readout draws at the same size the field will.
const GAIN_TEXT: f32 = 18.0;
const GAIN_CHAR_W: f32 = GAIN_TEXT * 0.6;
const GAIN_PAD: f32 = 10.0;
const GAIN_COLS: usize = 16;
/// Half-extents of the field's rect: the value's five characters and a
/// pad on each side, and room for the caret. The value is drawn centred
/// in the rect, not pinned to its left padding as frost's own
/// popup-sized field does — `ctx.text` centres the layout on the point
/// given, so a four-character value (the common `+0.0`) keeps the same
/// pixels in the readout and in the open field.
const GAIN_HALF_W: f32 = 5.0 * GAIN_CHAR_W / 2.0 + GAIN_PAD;
const GAIN_HALF_H: f32 = 12.0;
/// How far a corner box sits inside the card's edges, on both axes: the
/// card keeps a margin of its own colour around every box, so each box
/// reads as its corner without crowding the card's border.
const GAIN_INSET: f32 = 6.0;
/// How far a corner readout's centre sits from the card's centre,
/// vertically: the box tucked `GAIN_INSET` inside the card's top or
/// bottom edge. The card's shape line is measured against it, so the
/// two anchors cannot drift apart.
const GAIN_CORNER_Y: f32 = TRACK_HALF - GAIN_HALF_H - GAIN_INSET;
/// The card's name, standing above its centre.
const NAME_TEXT_Y: f32 = 10.0;
/// The centre of one wire's dB readout: the output trim in the card's
/// bottom-right corner, the input trim in its top-left — each box
/// tucked `GAIN_INSET` inside the corner of the edge its wire leaves
/// (or enters).
fn gain_center(x: f32, y: f32, side: bool) -> [f32; 2] {
    let sx = CARD_WIDTH / 2.0 - GAIN_HALF_W - GAIN_INSET;
    if side {
        [x + sx, y - GAIN_CORNER_Y]
    } else {
        [x - sx, y + GAIN_CORNER_Y]
    }
}
/// The card's shape line (`stereo → stereo`), centred in the space the
/// name and the output readout leave: the exact midpoint of their two
/// anchors, so it reads as the middle voice of the card rather than as
/// a caption pinned under either number.
fn layout_text_y(y: f32) -> f32 {
    y + (NAME_TEXT_Y - GAIN_CORNER_Y) / 2.0
}
const COLUMN_STEP: f32 = 224.0;
#[derive(Clone, Copy)]
struct Slot {
    id: NodeId,
    x: f32,
    y: f32,
}
struct Geometry {
    slots: Vec<Slot>,
    top: f32,
    bottom: f32,
    flow: f32,
    left: f32,
    max_vertical: f32,
}
fn groups(cards: &[Card]) -> Vec<Vec<NodeId>> {
    crate::runtime::stages(cards)
        .iter()
        .map(|stage| stage.iter().map(|card| card.id).collect())
        .collect()
}
/// A meter the user can click, in the order the signal passes them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum MeterPos {
    /// The rack's own input meter: upstream of every card.
    MasterIn,
    /// One card's input (`!side`) or output (`side`) wire meter.
    Wire(u64, bool),
    /// The rack's own output meter: downstream of every card.
    MasterOut,
}

/// Every meter a click at `from` acknowledges: that meter, and the ones the
/// clipped signal reached after it.
///
/// "After" is the topology `runtime::chain` actually wires — stages in
/// series, the cards inside a stage in parallel, each multi-card stage joined
/// by a mixer — read straight off `groups`, which is the same grouping the
/// layout and the graph are built from:
///
/// * a card's *output* meter owes nothing to its input meter, but a clip on
///   the way **in** came out of the same card, so the input meter owes its
///   output one;
/// * every card in a *later* stage sees that signal, so both of their meters
///   go too, and so does the rack's output meter;
/// * a card **beside** this one in the same stage keeps its report: the two
///   are parallel branches and that clip never travelled through it;
/// * the rack's input meter is upstream of everything, so it acknowledges the
///   whole rack, while its output meter is last and acknowledges only itself.
fn acknowledged(from: MeterPos, stages: &[Vec<NodeId>]) -> Vec<MeterPos> {
    let mut out = vec![from];
    let first_downstream = match from {
        MeterPos::MasterIn => 0,
        MeterPos::MasterOut => return out,
        MeterPos::Wire(id, side) => {
            if !side {
                out.push(MeterPos::Wire(id, true));
            }
            stages
                .iter()
                .position(|stage| stage.contains(&NodeId(id)))
                .map_or(stages.len(), |stage| stage + 1)
        }
    };
    for stage in &stages[first_downstream.min(stages.len())..] {
        for id in stage {
            out.extend([MeterPos::Wire(id.0, false), MeterPos::Wire(id.0, true)]);
        }
    }
    out.push(MeterPos::MasterOut);
    out
}
fn geometry(groups: &[Vec<NodeId>], w: f32, h: f32, scroll: f32, vertical: f32) -> Geometry {
    let top = h / 2.0 - 90.0;
    let bottom = -h / 2.0 + 150.0;
    let available = (top - bottom).max(112.0);
    let flow = (top + bottom) / 2.0;
    let rows = groups.iter().map(Vec::len).max().unwrap_or(1);
    let row_step = if rows < 2 {
        140.0
    } else {
        ((available - 112.0) / (rows - 1) as f32).clamp(116.0, 140.0)
    };
    let max_vertical = ((rows.saturating_sub(1)) as f32 * row_step + 112.0 - available).max(0.0);
    let left = -w / 2.0 + 90.0 - scroll;
    let mut slots = Vec::new();
    for (column, stage) in groups.iter().enumerate() {
        let height = stage.len().saturating_sub(1) as f32 * row_step + 112.0;
        let first = if height > available {
            top - 56.0 + vertical.clamp(0.0, max_vertical)
        } else {
            flow + (stage.len() - 1) as f32 * row_step / 2.0
        };
        for (row, &id) in stage.iter().enumerate() {
            slots.push(Slot {
                id,
                x: left + 90.0 + CARD_WIDTH / 2.0 + column as f32 * COLUMN_STEP,
                y: first - row as f32 * row_step,
            });
        }
    }
    Geometry {
        slots,
        top,
        bottom,
        flow,
        left,
        max_vertical,
    }
}
fn visible(slot: &Slot, top: f32, bottom: f32, w: f32) -> bool {
    slot.y + 56.0 <= top + 0.01
        && slot.y - 56.0 >= bottom - 0.01
        && slot.x + CARD_WIDTH / 2.0 >= -w / 2.0
        && slot.x - CARD_WIDTH / 2.0 <= w / 2.0
}
fn target_at(
    mouse: Option<[f32; 2]>,
    slots: &[Slot],
    moving: Option<NodeId>,
    top: f32,
    bottom: f32,
    w: f32,
) -> Option<Target> {
    let [x, y] = mouse?;
    if y > top || y < bottom {
        return None;
    }
    let slot = slots
        .iter()
        .filter(|slot| Some(slot.id) != moving && visible(slot, top, bottom, w))
        .min_by(|a, b| {
            ((x - a.x).powi(2) + (y - a.y).powi(2))
                .total_cmp(&((x - b.x).powi(2) + (y - b.y).powi(2)))
        })?;
    let dx = x - slot.x;
    let dy = y - slot.y;
    let side = if dx.abs() > 60.0 {
        if dx < 0.0 { Side::Left } else { Side::Right }
    } else if dy.abs() > 12.0 {
        if dy > 0.0 { Side::Above } else { Side::Below }
    } else if dx < 0.0 {
        Side::Left
    } else {
        Side::Right
    };
    Some(Target {
        card: slot.id,
        side,
    })
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn run(
    mut loader: impl FnMut(&Path) -> Result<Entry, String> + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    let (settings, warning) = Settings::load();
    let size = settings
        .window
        .map(|window| [window.width as u32, window.height as u32])
        .unwrap_or([1040, 580]);
    let mut cards = Vec::new();
    let mut states = std::collections::BTreeMap::new();
    let mut next_id = 1;
    for saved in &settings.preferences.plugins {
        if saved.id == 0 || saved.id > 1_000_000 || cards.iter().any(|c: &Card| c.id.0 == saved.id)
        {
            return Err("Invalid or duplicate saved plugin instance ID".into());
        }
        let entry = loader(&saved.path)
            .map_err(|e| format!("Could not restore '{}': {e}", saved.path.display()))?;
        let mut card = Card::new(
            NodeId(saved.id),
            saved.path.clone(),
            entry,
            saved.class.clone(),
            saved.name.clone(),
        )?;
        card.stage = saved.stage;
        cards.push(card);
        if let Some(bytes) = &saved.state {
            states.insert(NodeId(saved.id), bytes.clone());
        }
        next_id = next_id.max(saved.id + 2);
    }
    let mut runtime = Runtime::new(settings);
    runtime.restore(cards, states);
    runtime.persistence_path = Some(Settings::path());
    if let Some(warning) = warning {
        runtime.message = warning;
    }
    let font: Arc<[u8]> =
        Arc::from(include_bytes!("../../assets/fonts/FiraCode-VariableFont_wght.ttf").as_slice());
    let icons = nice_common::fonts::icon_font();
    let ui = RackUi {
        runtime,
        loader: Box::new(loader),
        font,
        icons,
        settings: None,
        settings_page: false,
        pressed: false,
        pressed_right: false,
        drag: None,
        scroll: 0.0,
        drops: VecDeque::new(),
        external_drag: false,
        vertical: 0.0,
        choosing: None,
        dropdown_scroll: 0.0,
        dropdown_drag: false,
        next_id,
        input: PeakMeter::latching(now()),
        output: PeakMeter::latching(now()),
        wire_meters: Default::default(),
        timing_meters: Default::default(),
        gain_drag: None,
        gain_edit: None,
        gain_input: frost::TextInput::default(),
        size: None,
        windows: Default::default(),
        overlay: None,
        parameter_scroll: 0.0,
        press_origin: None,
        moved: false,
    };
    let scene = Scene::new(SceneNode {
        shape: Some(Shape::Background {
            color: rgb(23, 18, 38),
        }),
        ..Default::default()
    });
    frost::run_configured(
        scene,
        ui,
        frost::Config {
            window_size: Some(size),
            escape_exits: false,
            close_exits: false,
            ..Default::default()
        },
    )
}
impl RackUi {
    fn add(&mut self, choosing: Choosing, index: usize) {
        if self.runtime.cards.len() >= 64 {
            self.runtime.message = "The first rack version supports at most 64 plugins".into();
            return;
        }
        let descriptor = &choosing.descriptors[index];
        match Card::new(
            crate::routing::NodeId(self.next_id),
            choosing.path,
            choosing.entry,
            descriptor.id.clone(),
            descriptor.name.clone(),
        ) {
            Ok(card) => {
                self.next_id += 2;
                let mut cards = self.runtime.cards.clone();
                let id = card.id;
                cards.push(card);
                if let Some(target) = choosing.placement {
                    match crate::runtime::arrange(cards, id, target) {
                        Ok(cards) => self.runtime.edit(cards),
                        Err(error) => self.runtime.message = error,
                    }
                } else {
                    self.runtime.edit(cards);
                }
            }
            Err(error) => self.runtime.message = error,
        }
    }
    /// Writes a wire's trim wherever that trim lives: the meter's own copy,
    /// which is what the white line draws and the tick loop re-states, the
    /// engine's slot table, which is what the audio path actually applies, and
    /// the rack's record, which is what a resumed rack opens with. A card that
    /// is no longer on the canvas has no meter to write, so all three are
    /// written on one call and none of them is trusted alone, because the
    /// readout shows a single value and this is what makes it true. The dB→linear
    /// conversion is the one the tick loop uses, so the two paths can never
    /// drift apart.
    fn set_trim(&mut self, id: u64, side: bool, gain_db: f32) {
        if let Some(wire) = self.wire_meters.get_mut(&(id, side)) {
            wire.gain_db = gain_db;
        }
        self.runtime.set_trim(id, side, gain_db);
        self.runtime
            .wires
            .set_gain(id << 1 | u64::from(side), 10f32.powf(gain_db / 20.0));
    }
    /// Open or close one wire, written to the same three places a trim is
    /// written to: the rack's meter, which draws the amber stub and the gap in
    /// the wire; the rack's record, which is what a resumed rack opens with; and
    /// the engine's slot table, which is what actually stops the audio. The
    /// meter may be missing — a wire whose card has left the canvas keeps its
    /// entry, so in practice it is there — but nothing here depends on it, for
    /// the same reason `set_trim` does not.
    ///
    /// A break is not a trim at zero: the meter on the source side of the break
    /// keeps measuring what arrives at it, so a card that is switched off still
    /// reads as alive, which is the whole difference between a wire that is
    /// unplugged and an amplifier that is turned down.
    fn set_open_circuit(&mut self, id: u64, side: bool, open: bool) {
        if let Some(wire) = self.wire_meters.get_mut(&(id, side)) {
            wire.open_circuit = open;
        }
        self.runtime.set_open_circuit(id, side, open);
        self.runtime.wires.set_open(id << 1 | u64::from(side), open);
    }
    /// Whether the switch on one wire stands open. The meter is asked rather
    /// than the record, because the meter is what the gap and the amber stub are
    /// drawn from: the two have to agree on the same frame, and a wire on the
    /// canvas has had its meter since the frame its card appeared.
    fn wire_open(&self, id: u64, side: bool) -> bool {
        !self.runtime.bypassed(id)
            && self
                .wire_meters
                .get(&(id, side))
                .is_some_and(|wire| wire.open_circuit)
    }
    fn toggle_pause(&mut self, id: u64) {
        let paused = !self.runtime.paused(id);
        self.runtime.set_paused(id, paused);
        if self.gain_drag.is_some_and(|(card, _, _)| card == id) {
            self.gain_drag = None;
        }
        if self.gain_edit.is_some_and(|(card, _)| card == id) {
            self.gain_edit = None;
            self.gain_input.set_focus(false);
        }
        if paused
            && !self.runtime.bypassed(id)
            && let Some(meter) = self.wire_meters.get_mut(&(id, true))
        {
            meter.peak = PeakMeter::latching(now());
        }
        self.runtime.message = if paused {
            if self.runtime.bypassed(id) {
                "Plugin paused; dry bypass remains active"
            } else {
                "Plugin paused: output muted"
            }
        } else {
            "Plugin resumed"
        }
        .into();
    }
    fn toggle_bypass(&mut self, id: u64) {
        let bypassed = !self.runtime.bypassed(id);
        self.runtime.set_bypassed(id, bypassed);
        if bypassed {
            for side in [false, true] {
                if let Some(meter) = self.wire_meters.get_mut(&(id, side)) {
                    meter.open = false;
                }
            }
            if self.gain_drag.is_some_and(|(card, _, _)| card == id) {
                self.gain_drag = None;
            }
            if self.gain_edit.is_some_and(|(card, _)| card == id) {
                self.gain_edit = None;
                self.gain_input.set_focus(false);
            }
        }
        self.runtime.message = if bypassed {
            "Card bypassed: unity dry signal"
        } else {
            "Card enabled"
        }
        .into();
    }
    /// A right-click on cable outside the meter flips one wire and updates the rack's
    /// message line. Every way of pointing at a wire ends here — the long
    /// connector between two meters, a card's own short run, the head feed from
    /// `IN`, the run into `OUT`, the stub box — so they cannot answer with
    /// different words, or worse, different states.
    fn toggle_wire(&mut self, id: u64, side: bool) {
        let open = !self.wire_open(id, side);
        self.set_open_circuit(id, side, open);
        // Named by the card it hangs on, since that is what an operator is
        // looking at when they reach for a wire. A card that has just been
        // removed leaves the line alone: there is nothing left to point at.
        let Some(card) = self.runtime.cards.iter().find(|card| card.id.0 == id) else {
            return;
        };
        let name = short(&card.name, 20);
        let side = if side { "output" } else { "input" };
        self.runtime.message = if open {
            format!("{name} {side} wire is open — nothing passes it")
        } else {
            format!("{name} {side} wire is closed again")
        };
    }
    /// Whether the meter at `pos` is red. Every wire the rack carries has a
    /// meter from the frame its card appeared, so a stub that has never been
    /// clicked can be red here and owes an acknowledgement like any other;
    /// only a wire that is gone has no meter to ask.
    fn meter_clipping(&self, pos: MeterPos) -> bool {
        match pos {
            MeterPos::MasterIn => self.input.clipping(),
            MeterPos::MasterOut => self.output.clipping(),
            MeterPos::Wire(id, side) => self
                .wire_meters
                .get(&(id, side))
                .is_some_and(|wire| wire.peak.clipping()),
        }
    }
    /// Acknowledge a clip: the meter clicked goes back to its resting colour
    /// and so does every meter downstream of it, which is the rule for a rack
    /// rather than for a meter — a clip three stages upstream has already been
    /// measured again by everything after it, and clearing one of them by hand
    /// while the rest stay red would describe a signal that no longer exists.
    ///
    /// A click on a meter that is *not* red acknowledges nothing: there is
    /// nothing there to report, and wiping the rack's downstream reports to
    /// open a fresh meter would hide a clip nobody has read.
    fn acknowledge_clip(&mut self, clicked: MeterPos) {
        if !self.meter_clipping(clicked) {
            return;
        }
        let covered = acknowledged(clicked, &groups(&self.runtime.cards));
        for pos in covered {
            match pos {
                MeterPos::MasterIn => self.input.clear_clip(),
                MeterPos::MasterOut => self.output.clear_clip(),
                MeterPos::Wire(id, side) => {
                    if let Some(wire) = self.wire_meters.get_mut(&(id, side)) {
                        wire.peak.clear_clip();
                    }
                }
            }
        }
    }
}
impl Process for RackUi {
    fn native_about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.windows.service(event_loop, &mut self.runtime);
    }
    fn native_window_event(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        id: winit::window::WindowId,
        event: &winit::event::WindowEvent,
    ) -> bool {
        self.windows.event(id, event, &mut self.runtime)
    }

    fn file_hovered(&mut self, path: Option<PathBuf>) {
        self.external_drag = path.is_some();
    }
    fn file_dropped(&mut self, path: PathBuf) {
        if self.drops.len() < 16 {
            self.drops.push_back(Dropped {
                path,
                point: None,
                captured: false,
            });
        } else {
            self.runtime.message = "Too many queued files; drop them in smaller groups".into();
        }
    }
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        self.windows.probe_shell(ctx.window());
        self.windows.capture(&mut self.runtime);
        self.runtime.tick();
        // Every wire the rack carries gets its meter here, before anything is
        // drawn, so a clip shows up on a stub nobody has ever clicked. A wire
        // whose card came out of the settings file is born at the trim the rack
        // saved for it, which is the only gain a rack can know before its
        // audio has run a block.
        watch_wires(
            &mut self.wire_meters,
            self.runtime.cards.iter().map(|card| card.id.0),
            |id, side| self.runtime.trim(id, side),
            |key| self.runtime.wires.gain_of(key),
            |id, side| self.runtime.open_circuit(id, side),
            now(),
        );
        self.timing_meters
            .retain(|id, _| self.runtime.cards.iter().any(|card| card.id == *id));
        for card in &self.runtime.cards {
            let bypassed = self.runtime.bypassed(card.id.0);
            self.runtime.wires.set_bypassed(card.id, bypassed);
            self.runtime
                .wires
                .set_paused(card.id, self.runtime.paused(card.id.0));
            if bypassed {
                for side in [false, true] {
                    if let Some(meter) = self.wire_meters.get_mut(&(card.id.0, side)) {
                        meter.open = false;
                    }
                }
            }
            self.timing_meters.entry(card.id).or_default().tick(
                self.runtime.timings.sample(card.id),
                dt,
                self.runtime.paused(card.id.0),
            );
        }
        let (w, h) = ctx.size();
        let mouse = ctx.mouse_position();
        let native_mouse = ctx.window().and_then(rack_window_input::cursor).or(mouse);
        for drop in &mut self.drops {
            if !drop.captured {
                drop.point = native_mouse;
                drop.captured = true;
            }
        }
        if let Some(window) = ctx.window() {
            window.set_title("Nice Rack");
        }
        if ctx.close_requested() {
            self.runtime.close();
        }
        if self.runtime.closing && self.runtime.stopped() {
            ctx.exit();
            return;
        }
        let size = (w as u32, h as u32);
        if self.size != Some(size) {
            self.size = Some(size);
            let geometry = nice_common::WindowGeom {
                x: None,
                y: None,
                width: w,
                height: h,
            };
            if let Err(error) = Settings::save_window(geometry) {
                self.runtime.message = error;
            }
        }
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let press = down && !self.pressed;
        let released = !down && self.pressed;
        self.pressed = down;
        let overlay_clicked = press;
        // While the dB editor stands, a press belongs to it alone: the
        // field's own mouse handling places the caret, or the editor
        // lifecycle below commits or drops the draft — and the rack
        // never also selects or drags something underneath the field.
        let editor_open = self.gain_edit.is_some() && self.overlay.is_none();
        let clicked = press && self.overlay.is_none() && !editor_open;
        // The right button is the rack's other hand, and it is tracked on its
        // own so that breaking a wire neither selects a card nor disturbs a
        // drag in progress: left-drag still moves cards and still nothing else,
        // and a right-click beside a card being dragged breaks nothing. While a
        // plugin's window covers the rack, or the dB editor is taking the
        // keyboard, the button does nothing at all.
        let right_down = ctx.mouse_button_down(frost::MouseButton::Right);
        let broke = right_down
            && !self.pressed_right
            && self.overlay.is_none()
            && !editor_open
            && self.choosing.is_none();
        self.pressed_right = right_down;
        if self.settings_page {
            let saved = Settings::load().0;
            let panel = self.settings.get_or_insert_with(|| {
                let mut panel =
                    AudioPanel::new(saved.clone(), self.runtime.applied.clone(), saved, false);
                panel.refresh();
                panel
            });
            panel.applied = if !self.runtime.busy()
                && !self.runtime.audio_running()
                && self
                    .runtime
                    .applied
                    .backend
                    .as_deref()
                    .is_some_and(|backend| backend != "dummy")
            {
                Settings::default()
            } else {
                self.runtime.applied.clone()
            };
            // The panel shows the transport's live block size whenever the
            // draft agrees with what is applied on JACK; the standalone
            // plugins stamp their own pump quantum there, and this is the
            // rack's turn at the same slot: the trial size the running
            // transport was opened with. Without it the panel answers with
            // the server's own period (`buffer_pref`) instead, which made a
            // successfully applied request look like it had reverted to the
            // JACK server's frames on the next look.
            panel.live_frames = self
                .runtime
                .audio_running()
                .then_some(self.runtime.applied.period_size)
                .flatten();
            // One rack preference rides below the audio block: the level a
            // minimised meter has to reach before it is drawn in colour. Its
            // options are labelled with the numbers they stand for, because
            // the page titles the default row with the raw it is the default
            // for — a unit written into an option would be painted over on
            // that one row. The unit belongs to the label instead.
            let gate = self.runtime.meter_gate();
            let extras = vec![nice_common::ui::ExtraRow {
                label: "Meter gate (dB)".into(),
                options: crate::METER_GATES
                    .iter()
                    .map(|db| {
                        let text = crate::meter_gate_text(*db);
                        (text.clone(), text)
                    })
                    .collect(),
                default: crate::meter_gate_text(crate::METER_GATE_DEFAULT_DB),
                current: (gate != crate::METER_GATE_DEFAULT_DB)
                    .then(|| crate::meter_gate_text(gate)),
            }];
            let runtime = &mut self.runtime;
            let event = panel.process(
                ctx,
                &self.font,
                clicked,
                extras,
                Some(nice_common::WindowGeom {
                    x: None,
                    y: None,
                    width: w,
                    height: h,
                }),
                !runtime.busy(),
                |settings| runtime.apply(settings),
            );
            // The gate is the only extra on this page, so the row it reports
            // an index for is that one. A pick of the default arrives as no
            // value at all — the page keeps a setting at its default unspoken
            // — and that is how a rack picked up at -12 finds its way back to
            // -60. Anything unparseable is refused by the runtime, which
            // leaves the gate exactly where it was.
            if let Some(Event::ExtraPicked(0, raw)) = &event {
                let gate = raw
                    .as_deref()
                    .and_then(|text| text.parse::<f32>().ok())
                    .unwrap_or(crate::METER_GATE_DEFAULT_DB);
                self.runtime.set_meter_gate(gate);
            }
            if matches!(event, Some(Event::Back | Event::Applied)) {
                self.settings_page = false;
                self.settings = None;
            }
            return;
        }
        if !self.runtime.busy()
            && self.overlay.is_none()
            && self.choosing.is_none()
            && let Some(Dropped { path, point, .. }) = self.drops.pop_front()
        {
            let groups = groups(&self.runtime.cards);
            let geometry = geometry(&groups, w, h, self.scroll, self.vertical);
            let placement = target_at(
                point.or(mouse),
                &geometry.slots,
                None,
                geometry.top,
                geometry.bottom,
                w,
            );
            if !path
                .extension()
                .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("clap"))
            {
                self.runtime.message = "Drop a CLAP file or .clap bundle".into();
            } else {
                match (self.loader)(&path) {
                    Ok(entry) => match entry.descriptors() {
                        Ok(descriptors) if descriptors.len() == 1 => self.add(
                            Choosing {
                                entry,
                                path,
                                descriptors,
                                placement,
                            },
                            0,
                        ),
                        Ok(descriptors) if descriptors.is_empty() => {
                            self.runtime.message = "This CLAP entry contains no plugins".into()
                        }
                        Ok(descriptors) => {
                            self.choosing = Some(Choosing {
                                entry,
                                path,
                                descriptors,
                                placement,
                            })
                        }
                        Err(error) => self.runtime.message = error,
                    },
                    Err(error) => self.runtime.message = error,
                }
            }
        }
        ctx.text(
            -w / 2.0 + 110.0,
            h / 2.0 - 36.0,
            &self.font,
            "NICE RACK",
            28.0,
            650.0,
            rgb(235, 230, 241),
            0.2,
        );
        let gx = w / 2.0 - 30.0;
        let gy = h / 2.0 - 32.0;
        let cog = hit(mouse, gx, gy, 18.0, 18.0);
        ctx.text(
            gx,
            gy,
            &self.icons,
            "\u{f6aa}",
            28.0,
            400.0,
            if cog {
                rgb(232, 163, 61)
            } else {
                rgb(180, 174, 193)
            },
            0.2,
        );
        if clicked && cog && self.choosing.is_none() {
            self.settings_page = true;
            self.settings = None;
            self.drag = None;
            // The settings page takes the whole window; an open dB
            // editor would have no card corner left to draw in.
            self.gain_edit = None;
            self.gain_input.set_focus(false);
            return;
        }
        let groups = groups(&self.runtime.cards);
        let max_scroll = (groups.len() as f32 * COLUMN_STEP + 320.0 - w).max(0.0);
        let previous = geometry(&groups, w, h, self.scroll, self.vertical);
        if self.drag.is_none() && self.overlay.is_none() {
            if previous.max_vertical > 0.0
                && !ctx.key_down(frost::KeyCode::ShiftLeft)
                && !ctx.key_down(frost::KeyCode::ShiftRight)
            {
                self.vertical =
                    (self.vertical - ctx.mouse_wheel() * 36.0).clamp(0.0, previous.max_vertical);
            } else {
                self.scroll = (self.scroll - ctx.mouse_wheel() * 36.0).clamp(0.0, max_scroll);
            }
        }
        if self.drag.is_some()
            && down
            && let Some([x, y]) = mouse
        {
            let horizontal = if x > w / 2.0 - 35.0 {
                1.0
            } else if x < -w / 2.0 + 35.0 {
                -1.0
            } else {
                0.0
            };
            let vertical = if y < previous.bottom + 25.0 {
                1.0
            } else if y > previous.top - 25.0 {
                -1.0
            } else {
                0.0
            };
            self.scroll =
                (self.scroll + horizontal * dt.clamp(0.0, 0.1) * 350.0).clamp(0.0, max_scroll);
            self.vertical = (self.vertical + vertical * dt.clamp(0.0, 0.1) * 250.0)
                .clamp(0.0, previous.max_vertical);
        }
        self.scroll = self.scroll.clamp(0.0, max_scroll);
        let layout = geometry(&groups, w, h, self.scroll, self.vertical);
        let bypass_target = if broke && self.drag.is_none() && self.gain_drag.is_none() {
            layout
                .slots
                .iter()
                .filter(|slot| visible(slot, layout.top, layout.bottom, w))
                .find(|slot| {
                    [false, true].into_iter().any(|side| {
                        let expanded = !self.runtime.bypassed(slot.id.0)
                            && self
                                .wire_meters
                                .get(&(slot.id.0, side))
                                .is_some_and(|meter| meter.open);
                        meter_hit(mouse, slot.x, slot.y, side, expanded)
                    })
                })
                .map(|slot| slot.id.0)
        } else {
            None
        };
        if let Some(id) = bypass_target {
            self.toggle_bypass(id);
        }

        let pause_target = if broke
            && bypass_target.is_none()
            && self.drag.is_none()
            && self.gain_drag.is_none()
        {
            layout
                .slots
                .iter()
                .find(|slot| {
                    visible(slot, layout.top, layout.bottom, w)
                        && hit(mouse, slot.x, slot.y, CARD_WIDTH / 2.0, TRACK_HALF)
                })
                .map(|slot| slot.id.0)
        } else {
            None
        };
        if let Some(id) = pause_target {
            self.toggle_pause(id);
        }
        ctx.text(
            layout.left,
            layout.flow,
            &self.font,
            "IN",
            18.0,
            650.0,
            rgb(103, 237, 175),
            0.2,
        );
        ctx.text(
            0.0,
            h / 2.0 - 72.0,
            &self.font,
            "Above/below: parallel    Left/right: serial",
            14.0,
            500.0,
            rgb(150, 145, 164),
            0.2,
        );
        let wire = rgb(107, 114, 128);
        // One gate for the whole rack, read once a frame: every stub box on
        // every wire is measured against the same number.
        let gate = self.runtime.meter_gate();
        // The rack's own `OUT` jack: the column its label stands in, and the
        // column the wire running into it stops at. Worked out before any of the
        // wiring, because that run is not the jack's wire — it is the last card's
        // output, and a click on it has to reach that card's output meter and the
        // switch on that wire. An empty rack has the jack on its own, with no
        // wire on it to break.
        let out_x = layout
            .slots
            .last()
            .map_or(layout.left + 130.0, |slot| junction_x(slot.x, true) + 60.0);
        let out_wire = out_x - 25.0;
        // The wire arrives from the rack's `IN` jack. Neither jack is a break
        // target in itself: a master meter has no stub box, and the engine gives
        // a node that is not a plugin no wire slot at all. But the two wires that
        // lead to them are, and each belongs to the card it touches — the head
        // feed to the first card's input, the run to `OUT` to the last card's
        // output.
        let mut previous_x = layout.left + 25.0;
        // The last stage that stood on its own, with its column: that card's
        // output owns the run into `OUT`, and by the time the run is drawn the
        // cards are out of scope. A last stage in parallel has no such card, and
        // its run to the jack belongs to the stage as a whole.
        let mut tail: Option<(u64, f32)> = None;
        for (index, group) in groups.iter().enumerate() {
            let stage_slots = layout
                .slots
                .iter()
                .filter(|slot| group.contains(&slot.id))
                .collect::<Vec<_>>();
            let x = stage_slots[0].x;
            let shared = group.len() > 1;
            let last = index + 1 == groups.len();
            // Whether the wire arriving here comes out of an earlier stage rather
            // than out of the rack's `IN` jack, which decides whether the left end
            // of this stage's input is a column somebody else's wire ends in.
            let fed = index > 0;
            let split = junction_x(x, false);
            let merge = junction_x(x, true);
            let low = stage_slots
                .iter()
                .map(|slot| slot.y)
                .fold(layout.flow, f32::min)
                .max(layout.bottom);
            let high = stage_slots
                .iter()
                .map(|slot| slot.y)
                .fold(layout.flow, f32::max)
                .min(layout.top);
            // The trunk, up to the column this stage splits at, at the height
            // signals flow at. A stage on its own sits on that very line, so its
            // trunk is the connector an operator sees running between two meters,
            // and it is that card's input: an open switch cuts it short of the
            // stub box like any other wire. In a parallel stage the signals of
            // every card in it pass down into the risers it feeds, so the trunk
            // is the stage's and stays whole — like the risers themselves, which
            // are nobody's to break.
            //
            // Drawn on the flow line wherever it is, and not only where a card is
            // visible: a rack scrolled along its wiring still shows it running
            // past the edge.
            let feed_open = !shared && self.wire_open(stage_slots[0].id.0, false);
            wire_segment(
                ctx,
                previous_x,
                wire_end(split, wire_box_x(x, false), feed_open),
                layout.flow,
                wire,
            );
            if shared {
                ctx.line(split, low, split, high, wire, 2.0, 0.1);
                ctx.line(merge, low, merge, high, wire, 2.0, 0.1);
                ctx.text(
                    merge,
                    layout.flow,
                    &self.font,
                    "+",
                    15.0,
                    650.0,
                    rgb(103, 237, 175),
                    0.22,
                );
            }
            for slot in &stage_slots {
                if !visible(slot, layout.top, layout.bottom, w) {
                    continue;
                }
                if self.runtime.bypassed(slot.id.0) {
                    let gap = stage_slots
                        .iter()
                        .filter(|other| other.y > slot.y)
                        .map(|other| other.y - slot.y - 2.0 * TRACK_HALF)
                        .fold(24.0, f32::min);
                    let path = bypass_path(slot.x, slot.y, (gap / 2.0).min(12.0));
                    for segment in path.windows(2) {
                        ctx.line(
                            segment[0][0],
                            segment[0][1],
                            segment[1][0],
                            segment[1][1],
                            nice_common::knob::AMBER,
                            2.0,
                            0.18,
                        );
                    }
                }
                for side in [false, true] {
                    let bx = wire_box_x(x, side);
                    let key = (slot.id.0, side);
                    let open = self.wire_open(slot.id.0, side);
                    // The whole stretch this side owns, narrowed to what a click
                    // on it may claim; see [`wire_click`]. The wire is still drawn
                    // in full, so an open one stays as easy to close as it was to
                    // open.
                    let (from, to) = wire_click(
                        x,
                        side,
                        shared,
                        fed,
                        previous_x,
                        if last { out_wire } else { merge },
                    );
                    if broke
                        && bypass_target.is_none()
                        && pause_target.is_none()
                        && !self.runtime.bypassed(slot.id.0)
                        && wire_hit(mouse, from, to, slot.y, bx)
                    {
                        // The wire as a whole is the target, not just its box:
                        // the run between two meters is long, and an operator
                        // points at it rather than at a terminal on it. Breaking
                        // is the right hand's only job, so it takes no card and
                        // starts no drag, and it leaves an expanded meter
                        // expanded — a meter that is open can still be broken.
                        self.toggle_wire(slot.id.0, side);
                    }
                    if clicked
                        && !self.runtime.bypassed(slot.id.0)
                        && self.choosing.is_none()
                        && hit(mouse, bx, slot.y, STUB_HALF, STUB_TALL)
                    {
                        // Toggle. The meter has been running since the
                        // card appeared, so this only folds the track
                        // away or brings it back and the peak-hold runs
                        // straight through the collapse. The insert arm
                        // is a fallback so a click can never be
                        // swallowed, not the normal way in — but it asks
                        // the same question `watch_wires` asks, so a meter
                        // born here opens at the saved trim, and on a wire
                        // remembered open.
                        let meter = self.wire_meters.entry(key).or_insert_with(|| {
                            let key = slot.id.0 << 1 | u64::from(side);
                            let gain_db = remembered_trim(
                                self.runtime.trim(slot.id.0, side),
                                self.runtime.wires.gain_of(key),
                            );
                            WireMeter {
                                open_circuit: self.runtime.open_circuit(slot.id.0, side),
                                ..new_wire_meter(gain_db, now())
                            }
                        });
                        meter.open = !meter.open;
                        // A click on the meter body is also the rack's
                        // "yes, I saw it": a red meter here, and every
                        // meter the clipped signal reached after it, go
                        // back to their resting colours.
                        self.acknowledge_clip(MeterPos::Wire(slot.id.0, side));
                    }
                    // This card's own row: in from the column its stage shares to
                    // the card's edge, and out from the edge to that column again.
                    // These are the short pieces a parallel stage leaves to each
                    // of the cards in it; what an operator sees as one long
                    // connector between two meters is the trunk above, which runs
                    // on the flow line — the very row a solo card sits on.
                    let bypassed = self.runtime.bypassed(slot.id.0);
                    let (inner, outer) = card_lead(x, side, split, merge, bypassed);
                    let outer = if bypassed {
                        outer
                    } else {
                        wire_end(outer, bx, open)
                    };
                    wire_segment(ctx, inner, outer, slot.y, wire);
                    // The stub box is the whole of a minimised meter's
                    // face, so it carries everything the rack knows about
                    // the wire: red for a clip nobody has acknowledged, amber
                    // for a circuit somebody has opened, the meter's own level
                    // green for a track that is expanded or is carrying signal
                    // over the gate, and the plain wire colour while nothing is
                    // passing.
                    let stub_color = stub_color(self.wire_meters.get(&key), gate, wire);
                    ctx.rectangle(bx, slot.y, STUB_HALF, STUB_TALL, stub_color, 0.25);
                }
            }
            previous_x = merge;
            tail = (!shared).then(|| (stage_slots[0].id.0, x));
        }
        // The run from the last stage into the jack, drawn past the loop so it
        // keeps running when the card it belongs to has scrolled away — and cut
        // like any other wire, by the switch on the output of that last card
        // standing on its own. Where the last stage is a parallel one there is no
        // such card, and the run into `OUT` is the stage's: whole.
        let tail_end = tail.map_or(out_wire, |(id, x)| {
            wire_end(out_wire, wire_box_x(x, true), self.wire_open(id, true))
        });
        wire_segment(ctx, previous_x, tail_end, layout.flow, wire);
        ctx.text(
            out_x,
            layout.flow,
            &self.font,
            "OUT",
            18.0,
            650.0,
            rgb(103, 237, 175),
            0.2,
        );
        // A press on a corner readout belongs to that readout: it
        // stands inside its card's hover rect, so without this veto the
        // opening click would also grab the card and start a drag. The
        // readout stands on every card the rack draws — entry or not,
        // expanded or collapsed — so the veto is the rect alone, and
        // the number is never a way to move a card.
        let readout_press = press
            && self.overlay.is_none()
            && self.gain_edit.is_none()
            && self.choosing.is_none()
            && self.gain_drag.is_none()
            && layout.slots.iter().any(|slot| {
                visible(slot, layout.top, layout.bottom, w)
                    && !self.runtime.bypassed(slot.id.0)
                    && !self.runtime.paused(slot.id.0)
                    && [false, true].into_iter().any(|side| {
                        let [gx, gy] = gain_center(slot.x, slot.y, side);
                        hit(mouse, gx, gy, GAIN_HALF_W, GAIN_HALF_H)
                    })
            });
        let mut remove = None;
        for slot in &layout.slots {
            if !visible(slot, layout.top, layout.bottom, w) {
                continue;
            }
            let card = self
                .runtime
                .cards
                .iter()
                .find(|card| card.id == slot.id)
                .unwrap();
            let x = slot.x;
            let y = slot.y;
            let hover = hit(mouse, x, y, CARD_WIDTH / 2.0, 56.0);
            let paused = self.runtime.paused(card.id.0);
            ctx.rectangle(
                x,
                y,
                CARD_WIDTH / 2.0,
                56.0,
                if paused {
                    rgb(26, 24, 33)
                } else if self.drag == Some(card.id) {
                    rgb(75, 62, 100)
                } else if hover {
                    rgb(58, 51, 79)
                } else {
                    rgb(42, 36, 61)
                },
                0.2,
            );
            ctx.text(
                x,
                y + NAME_TEXT_Y,
                &self.font,
                short(&card.name, 16),
                18.0,
                600.0,
                if paused {
                    nice_common::knob::MUTED
                } else {
                    rgb(235, 230, 241)
                },
                0.21,
            );
            ctx.text(
                x,
                layout_text_y(y),
                &self.font,
                if paused {
                    "PAUSED".into()
                } else {
                    format!("{:?} → {:?}", card.inputs, card.outputs)
                },
                12.0,
                500.0,
                rgb(150, 145, 164),
                0.21,
            );
            let timing_hover = hit(mouse, x, y - 54.0, CARD_WIDTH / 2.0, 5.0);
            if let Some(meter) = self.timing_meters.get(&card.id) {
                meter.draw(
                    ctx,
                    &self.font,
                    [x, y],
                    CARD_WIDTH,
                    timing_hover
                        && self.drag.is_none()
                        && self.choosing.is_none()
                        && self.overlay.is_none()
                        && self.gain_edit.is_none(),
                );
            }
            let rx = x + CARD_WIDTH / 2.0 - 14.0;
            let ry = y + 42.0;
            ctx.text(
                rx,
                ry,
                &self.font,
                "×",
                18.0,
                600.0,
                rgb(220, 160, 160),
                0.22,
            );
            if !self.runtime.busy() && self.choosing.is_none() && !readout_press {
                if clicked && hit(mouse, rx, ry, 12.0, 12.0) {
                    remove = Some(card.id);
                } else if clicked && hover {
                    self.drag = Some(card.id);
                    self.press_origin = mouse;
                    self.moved = false;
                }
            }
            // The hover tooltip draws far above the rack, where an open
            // corner field would sit, so it stands aside while the
            // editor is up.
            if hover
                && !timing_hover
                && self.drag.is_none()
                && self.choosing.is_none()
                && self.overlay.is_none()
                && self.gain_edit.is_none()
            {
                draw_details(ctx, &self.font, card);
            }
        }
        if self.runtime.cards.is_empty() {
            ctx.text(
                0.0,
                layout.flow - 90.0,
                &self.font,
                "Drop a CLAP file here",
                23.0,
                550.0,
                rgb(160, 154, 175),
                0.2,
            );
        }
        if let Some(id) = remove {
            let cards = self
                .runtime
                .cards
                .iter()
                .filter(|card| card.id != id)
                .cloned()
                .collect();
            self.runtime.edit(cards);
            self.drag = None;
        }
        if self.drag.is_some()
            && let (Some(a), Some(b)) = (self.press_origin, mouse)
        {
            self.moved |= (a[0] - b[0]).hypot(a[1] - b[1]) > 5.0;
        }
        if released
            && !self.moved
            && let Some(id) = self.drag.take()
        {
            if self
                .runtime
                .session
                .as_mut()
                .is_some_and(|s| s.editor_supported(id))
            {
                self.windows.open(id);
            } else {
                self.overlay = Some(id);
                self.parameter_scroll = 0.0;
            }
        }
        let preview_mouse = if self.external_drag {
            native_mouse
        } else {
            mouse
        };
        if (self.drag.is_some() && self.moved) || self.external_drag {
            let target = target_at(
                preview_mouse,
                &layout.slots,
                self.drag,
                layout.top,
                layout.bottom,
                w,
            );
            if let Some(target) = target {
                let slot = layout
                    .slots
                    .iter()
                    .find(|slot| slot.id == target.card)
                    .unwrap();
                let parallel = matches!(target.side, Side::Above | Side::Below);
                match target.side {
                    Side::Left | Side::Right => ctx.rectangle(
                        slot.x
                            + if target.side == Side::Left {
                                -106.0
                            } else {
                                106.0
                            },
                        slot.y,
                        2.0,
                        70.0,
                        rgb(103, 237, 175),
                        0.5,
                    ),
                    Side::Above | Side::Below => ctx.rectangle(
                        slot.x,
                        slot.y
                            + if target.side == Side::Above {
                                66.0
                            } else {
                                -66.0
                            },
                        93.0,
                        2.0,
                        rgb(103, 237, 175),
                        0.5,
                    ),
                }
                if let Some([x, y]) = preview_mouse {
                    ctx.text(
                        x,
                        y + 30.0,
                        &self.font,
                        if parallel {
                            "Parallel branch"
                        } else {
                            "Serial stage"
                        },
                        15.0,
                        550.0,
                        rgb(103, 237, 175),
                        0.6,
                    );
                }
                if released && let Some(id) = self.drag {
                    match crate::runtime::arrange(self.runtime.cards.clone(), id, target) {
                        Ok(cards) => self.runtime.edit(cards),
                        Err(error) => self.runtime.message = error,
                    }
                    self.drag = None;
                }
            } else if released || mouse.is_none() {
                self.drag = None;
            }
            if ctx.key_down(frost::KeyCode::Escape) {
                self.drag = None;
            }
        }
        let time = now();
        let input = f32::from_bits(self.runtime.levels.input.load(Ordering::Relaxed));
        let output = f32::from_bits(self.runtime.levels.output.load(Ordering::Relaxed));
        let fresh = time.saturating_sub(self.runtime.levels.updated.load(Ordering::Acquire)) < 250;
        let db = |sample: f32| {
            if fresh && sample > 1e-7 {
                20.0 * sample.log10()
            } else {
                -120.0
            }
        };
        let input_peak = f32::from_bits(self.runtime.levels.input_peak.load(Ordering::Relaxed));
        let output_peak = f32::from_bits(self.runtime.levels.output_peak.load(Ordering::Relaxed));
        self.input.tick(db(input), db(input_peak), time);
        self.output.tick(db(output), db(output_peak), time);
        // Dragging an open meter with the left button trims its gain
        // *relative* to where the trim stands: each frame's pointer step
        // nudges the gain by a step-scaled amount, so a slow drag keeps
        // fractions-of-a-dB control while a fast flick sweeps the range
        // (see gain_drag_delta). The grab tracks the pointer itself, so
        // the drag survives the card scrolling out of view, and it lets
        // go the moment the button, an editor overlay or Escape takes
        // the pointer away.
        if let Some((id, side, last_y)) = self.gain_drag {
            if ctx.key_down(frost::KeyCode::Escape) || !down || self.overlay.is_some() {
                self.gain_drag = None;
            } else if let Some([_, my]) = ctx.mouse_position()
                && let Some(db) = self
                    .wire_meters
                    .get(&(id, side))
                    .map(|wire| gain_drag_delta(my - last_y, wire.gain_db))
            {
                self.set_trim(id, side, db);
                self.gain_drag = Some((id, side, my));
            } else {
                self.gain_drag = None;
            }
        }
        // The dB editor stands until it is answered: Escape abandons the
        // draft, an editor whose card vanished — deleted, or scrolled out of
        // the layout, or an overlay taking the window — dies with it, and a
        // press outside the field commits the draft if it parses and drops it
        // otherwise. Testing "outside" needs the field's live rect, so its
        // card is looked up again here; a card scrolled off-screen has no
        // rect, which makes any press an outside one. (Enter is answered
        // where the field itself draws, in the meter loop.) The field's
        // survival never depended on the meter being open, or on there being
        // a meter: the readout is on the card, so the card is what the field
        // belongs to.
        if let Some((id, side)) = self.gain_edit {
            let gone = !layout.slots.iter().any(|slot| slot.id.0 == id);
            if ctx.key_down(frost::KeyCode::Escape) || self.overlay.is_some() || gone {
                self.gain_edit = None;
                self.gain_input.set_focus(false);
            } else if press {
                let inside = layout
                    .slots
                    .iter()
                    .find(|slot| slot.id.0 == id && visible(slot, layout.top, layout.bottom, w))
                    .is_some_and(|slot| {
                        let [gx, gy] = gain_center(slot.x, slot.y, side);
                        hit(mouse, gx, gy, GAIN_HALF_W, GAIN_HALF_H)
                    });
                if !inside {
                    let draft = self.gain_input.buffer().to_string();
                    if let Some(db) = parse_gain(&draft) {
                        self.set_trim(id, side, db);
                    }
                    self.gain_edit = None;
                    self.gain_input.set_focus(false);
                }
            }
        }
        for ((id, side), wire) in &mut self.wire_meters {
            // A wire whose card was removed samples as gone, so its meter
            // decays to silence on its own. Nothing prunes the entry, and
            // nothing draws it either: its id is never handed out again.
            let key = *id << 1 | u64::from(*side);
            // Re-state the trim every tick: a wire registered while audio
            // was stopped comes up at unity, and this re-claim is what
            // keeps the audible gain where the white line shows it.
            self.runtime
                .wires
                .set_gain(key, 10f32.powf(wire.gain_db / 20.0));
            // And the switch, for the same reason: a wire registered while audio
            // was stopped comes back closed, and a rack that was saved with a
            // cable out of it has to go quiet again where it was quiet, whether
            // or not the engine happened to be running when it was switched.
            self.runtime.wires.set_open(key, wire.open_circuit);
            let (rms, peak) = self.runtime.wires.sample(key).unwrap_or((0.0, 0.0));
            if *side && self.runtime.paused(*id) && !self.runtime.bypassed(*id) {
                wire.peak = PeakMeter::latching(time);
            } else {
                wire.peak.tick(db(rms), db(peak), time);
            }
        }
        for slot in &layout.slots {
            if self.runtime.bypassed(slot.id.0) || !visible(slot, layout.top, layout.bottom, w) {
                continue;
            }
            for side in [false, true] {
                // The readout stands on every card the rack draws and is
                // drawn by the rack alone: entry or not, expanded or
                // collapsed, and whatever plugin the card happens to hold.
                // So every fact the rest of this body needs is copied out
                // once, here — the editor below writes through `&mut self`,
                // and a card whose stub was never clicked has no meter to
                // borrow at all, only the gain the engine is already
                // applying, which is the same number.
                let wire = self.wire_meters.get(&(slot.id.0, side));
                let key = slot.id.0 << 1 | u64::from(side);
                let (gain_db, clip) = corner_reading(wire, self.runtime.wires.gain_of(key));
                let open = wire.is_some_and(|meter| meter.open);
                // Only an open meter draws a track, and an open meter always
                // has an entry, so the default below is never read; −120 dB is
                // what `db` calls silence anyway.
                let level_db = wire.map_or(-120.0, |meter| meter.peak.level_db());
                let peak_db = wire.map_or(-120.0, |meter| meter.peak.peak_db());
                let bx = wire_box_x(slot.x, side);
                let paused = self.runtime.paused(slot.id.0);
                if !paused {
                    let [gx, gy] = gain_center(slot.x, slot.y, side);
                    // The corner readout is the numeric handle: a click
                    // adopts the trim's value into the field and the number
                    // becomes editable in place. `clicked` is already false
                    // while an editor stands, so one click opens exactly one
                    // field, and the same press that opened it lands on the
                    // field's own mouse handling below.
                    if clicked
                        && self.choosing.is_none()
                        && self.gain_drag.is_none()
                        && hit(mouse, gx, gy, GAIN_HALF_W, GAIN_HALF_H)
                    {
                        self.gain_edit = Some((slot.id.0, side));
                        self.gain_input.adopt(format!("{:+.1}", gain_db));
                        self.gain_input.set_focus(true);
                    }
                    let editing = self.gain_edit == Some((slot.id.0, side));
                    // The corner box is always there — the readout's
                    // frame, not just the editor's — so clicking the
                    // number opens the field around it without the card
                    // changing shape underneath.
                    ctx.rectangle(
                        gx,
                        gy,
                        GAIN_HALF_W,
                        GAIN_HALF_H,
                        rgb(24, 20, 38),
                        if editing { 0.35 } else { 0.3 },
                    );
                    // The gain in dB, centred in that box — the clickable
                    // numeric handle, no unit suffix. Above full scale it turns
                    // red with the rest of the meter: the readout is the part of
                    // the meter you are most likely to be looking at, and a clip
                    // that only shows up in the track is a clip you will miss.
                    if !editing {
                        ctx.text(
                            gx,
                            gy,
                            &self.font,
                            format!("{:+.1}", gain_db),
                            GAIN_TEXT,
                            500.0,
                            if clip {
                                nice_common::knob::RED
                            } else {
                                rgb(226, 232, 240)
                            },
                            0.3,
                        );
                    }
                    // The field, drawn exactly where the readout stands.
                    // frost's `TextInput` owns the editing — keys, caret,
                    // selection, blink — and these calls draw its state at
                    // the box's own anchors: the value's visible block is
                    // centred on `gx` and the caret and selection columns
                    // share that origin, so view and editor are the same
                    // pixels. frost anchors its popup-sized field's text to
                    // its left padding instead; the rack's 24 px box
                    // centres, on purpose. A draft wider than the rect
                    // spills past it, as frost's own field does; a dB value
                    // never needs one.
                    if editing {
                        let left = gx - GAIN_HALF_W;
                        let bottom = gy - GAIN_HALF_H;
                        let right = gx + GAIN_HALF_W;
                        let top = gy + GAIN_HALF_H;
                        let accent = rgb(103, 237, 175);
                        let committed = self.gain_input.process_keys(ctx, dt);
                        self.gain_input
                            .process_mouse(mouse, down, [left, bottom, right, top]);
                        let draft = self.gain_input.buffer().to_string();
                        let chars: Vec<char> = draft.chars().collect();
                        let scroll = self.gain_input.scroll_offset();
                        let shown = chars.len().min(scroll + GAIN_COLS);
                        // The visible block's columns, measured from the
                        // block's centred left edge — the anchor the static
                        // readout draws its string around.
                        let half = (shown - scroll) as f32 * GAIN_CHAR_W / 2.0;
                        let col_x = |col: usize| gx - half + (col - scroll) as f32 * GAIN_CHAR_W;
                        if let Some((start, end)) = self.gain_input.selection_range() {
                            let a = start.max(scroll);
                            let b = end.min(scroll + GAIN_COLS).min(chars.len());
                            if a < b {
                                let x0 = col_x(a);
                                let x1 = col_x(b);
                                ctx.rectangle(
                                    (x0 + x1) / 2.0,
                                    gy,
                                    (x1 - x0) / 2.0,
                                    GAIN_HALF_H,
                                    Color { a: 0.30, ..accent },
                                    0.35,
                                );
                            }
                        }
                        if shown > scroll {
                            let visible: String = chars[scroll..shown].iter().collect();
                            ctx.text(
                                gx,
                                gy,
                                &self.font,
                                visible,
                                GAIN_TEXT,
                                500.0,
                                rgb(226, 232, 240),
                                0.35,
                            );
                        }
                        if self.gain_input.focused() {
                            if self.gain_input.blink_on() {
                                let cx = col_x(self.gain_input.cursor_column());
                                ctx.line(cx, bottom + 5.0, cx, top - 5.0, accent, 1.5, 0.35);
                            }
                            for [x0, y0, x1, y1] in [
                                [left, bottom, right, bottom],
                                [left, top, right, top],
                                [left, bottom, left, top],
                                [right, bottom, right, top],
                            ] {
                                ctx.line(x0, y0, x1, y1, accent, 2.0, 0.35);
                            }
                        }
                        if committed {
                            // Enter answers the editor: a number is written
                            // back and the field closes; a draft that is not
                            // one keeps the field open and says what it
                            // wanted.
                            match parse_gain(&draft) {
                                Some(db) => {
                                    self.set_trim(slot.id.0, side, db);
                                    self.gain_edit = None;
                                    self.gain_input.set_focus(false);
                                }
                                None => self.runtime.message = "Type a dB number".to_string(),
                            }
                        }
                    }
                }
                // Everything the corner needed is drawn; the rest of this
                // body is the meter proper, which only an open box has.
                if !open {
                    // Collapsed: the stub box on the wire is all of the meter
                    // that draws, and the trim readout above keeps standing.
                    // The peak-hold keeps ticking in the loop over
                    // `wire_meters`, so re-opening finds the meter where it
                    // was left.
                    continue;
                }
                let dragging = self
                    .gain_drag
                    .is_some_and(|(id, grabbed, _)| id == slot.id.0 && grabbed == side);
                if clicked
                    && !paused
                    && self.choosing.is_none()
                    && self.gain_drag.is_none()
                    && self.gain_edit.is_none()
                    && let Some([_, my]) = mouse
                    && hit(mouse, bx, slot.y, 4.5, TRACK_HALF)
                    && !hit(mouse, bx, slot.y, STUB_HALF, STUB_TALL)
                {
                    // Grabbing the track turns the meter into the gain
                    // slider: the grab records the pointer height so the
                    // drag is relative from here. The stub box sits at
                    // the track centre and keeps its toggle, so it is
                    // excluded from the grab.
                    self.gain_drag = Some((slot.id.0, side, my));
                    // The track is the other half of the meter body, so it
                    // acknowledges a clip just as the stub does — pulling the
                    // gain down and clearing the report are the same gesture
                    // on the same thing. The dB readout in the card corner is
                    // deliberately not: it is where the operator goes to look
                    // at the trim, and the clip should still be there when
                    // they arrive.
                    self.acknowledge_clip(MeterPos::Wire(slot.id.0, side));
                }
                let hovered = dragging || hit(mouse, bx, slot.y, 4.5, TRACK_HALF);
                // The vertical meter carries the card's full height and
                // scales over the last 48 dB, screen y pointing up. The
                // track brightens while grabbed or hovered, saying "this
                // is the slider" before the first drag even starts.
                ctx.rectangle(
                    bx,
                    slot.y,
                    3.0,
                    56.0,
                    if clip {
                        if hovered {
                            rgb(97, 35, 45)
                        } else {
                            rgb(78, 28, 36)
                        }
                    } else if hovered {
                        rgb(45, 38, 64)
                    } else {
                        rgb(30, 25, 45)
                    },
                    0.3,
                );
                let t = ((level_db.max(-48.0) + 48.0) / 48.0).clamp(0.0, 1.0);
                if t > 0.0 {
                    ctx.rectangle(
                        bx,
                        slot.y - 56.0 + 56.0 * t,
                        3.0,
                        56.0 * t,
                        if clip { nice_common::knob::RED } else { LEVEL },
                        0.3,
                    );
                }
                // Gray marks every 6 dB: the meter's own scale runs −48 to
                // 0 over the 112 px track, so a mark lands every 14 px.
                for step in 1..8 {
                    ctx.rectangle(
                        bx,
                        slot.y - 56.0 + 14.0 * step as f32,
                        3.0,
                        0.5,
                        if clip {
                            rgb(188, 62, 56)
                        } else {
                            rgb(107, 114, 128)
                        },
                        0.3,
                    );
                }
                let p = ((peak_db.max(-48.0) + 48.0) / 48.0).clamp(0.0, 1.0);
                // Dimmed to slate so the white gain line stays the one
                // bright mark on the track.
                ctx.rectangle(
                    bx,
                    slot.y - 56.0 + 112.0 * p,
                    4.5,
                    1.0,
                    if clip {
                        nice_common::knob::RED
                    } else {
                        rgb(148, 163, 184)
                    },
                    0.3,
                );
                if !paused {
                    ctx.rectangle(
                        bx,
                        gain_line_y(gain_db, slot.y),
                        4.5,
                        0.5,
                        rgb(255, 255, 255),
                        0.3,
                    );
                }
            }
        }
        let fit = (w / 1000.0).clamp(0.45, 1.0);
        let y = -h / 2.0 + 58.0;
        let over_input = self
            .input
            .draw(ctx, &self.font, [-w / 4.0, y], w * 0.20, fit, LEVEL);
        let over_output = self
            .output
            .draw(ctx, &self.font, [w / 4.0, y], w * 0.20, fit, LEVEL);
        // The two master meters take part in the same acknowledgement rule as
        // the wire meters: the input one is upstream of every card, so
        // acknowledging it clears the rack end to end, and the output one is
        // last, so it clears nothing but itself.
        if clicked && over_input {
            self.acknowledge_clip(MeterPos::MasterIn);
        }
        if clicked && over_output {
            self.acknowledge_clip(MeterPos::MasterOut);
        }
        for (x, label) in [(-w / 4.0, "INPUT"), (w / 4.0, "OUTPUT")] {
            ctx.text(
                x,
                y + 24.0,
                &self.font,
                label,
                13.0,
                550.0,
                rgb(180, 174, 193),
                0.2,
            );
        }
        let columns = (w / 9.0).max(20.0) as usize;
        let characters = self.runtime.message.chars().collect::<Vec<_>>();
        for (line, chunk) in characters.chunks(columns).take(3).enumerate() {
            ctx.text(
                0.0,
                -h / 2.0 + 112.0 + line as f32 * 18.0,
                &self.font,
                chunk.iter().collect::<String>(),
                14.0,
                500.0,
                rgb(232, 163, 61),
                0.2,
            );
        }
        if self.choosing.is_some() {
            let choices = self.choosing.as_ref().unwrap();
            let items = choices
                .descriptors
                .iter()
                .map(|descriptor| (descriptor.name.clone(), descriptor.id.clone()))
                .collect::<Vec<_>>();
            let response = nice_common::dropdown::draw_dropdown(
                ctx,
                &self.font,
                [w, h],
                1.0,
                nice_common::dropdown::Placement::Centred([200.0, 14.0]),
                0.8,
                clicked,
                &items,
                0,
                "",
                "SELECT PLUGIN",
                false,
                &mut self.dropdown_scroll,
                &mut self.dropdown_drag,
            );
            if let Some(index) = response.picked {
                let choosing = self.choosing.take().unwrap();
                self.add(choosing, index);
            } else if ctx.key_down(frost::KeyCode::Escape) || (clicked && !response.consumed) {
                self.choosing = None;
            }
        }
        if let Some(id) = self.overlay {
            self.draw_parameters(ctx, id, overlay_clicked, down);
        }
    }
}

impl RackUi {
    fn draw_parameters(&mut self, ctx: &mut Context, id: NodeId, clicked: bool, down: bool) {
        let (w, h) = ctx.size();
        ctx.rectangle(
            0.0,
            0.0,
            w / 2.0,
            h / 2.0,
            Color {
                a: 0.94,
                ..rgb(22, 18, 32)
            },
            0.7,
        );
        let name = self
            .runtime
            .cards
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.as_str())
            .unwrap_or("Plugin");
        ctx.text(
            0.0,
            h / 2.0 - 36.0,
            &self.font,
            format!("{} — controls", short(name, 36)),
            22.0,
            650.0,
            rgb(235, 230, 241),
            0.8,
        );
        ctx.text(
            w / 2.0 - 35.0,
            h / 2.0 - 35.0,
            &self.font,
            "×",
            24.0,
            600.0,
            rgb(235, 230, 241),
            0.8,
        );
        if ctx.key_down(frost::KeyCode::Escape)
            || (clicked
                && hit(
                    ctx.mouse_position(),
                    w / 2.0 - 35.0,
                    h / 2.0 - 35.0,
                    20.0,
                    20.0,
                ))
        {
            self.overlay = None;
            self.runtime.persist();
            return;
        }
        let parameters = self
            .runtime
            .session
            .as_mut()
            .map(|s| s.parameters(id))
            .unwrap_or_default();
        let visible = ((h - 145.0) / 56.0).max(1.0) as usize;
        self.parameter_scroll = (self.parameter_scroll - ctx.mouse_wheel())
            .clamp(0.0, parameters.len().saturating_sub(visible) as f32);
        let start = self.parameter_scroll as usize;
        if parameters.is_empty() {
            ctx.text(
                0.0,
                0.0,
                &self.font,
                "No exposed parameters",
                20.0,
                500.0,
                rgb(170, 164, 185),
                0.8,
            );
        }
        let x = 90.0;
        let half = (w / 2.0 - 130.0).max(30.0);
        for (row, param) in parameters.iter().skip(start).take(visible).enumerate() {
            let y = h / 2.0 - 104.0 - row as f32 * 56.0;
            let label = if param.module.is_empty() {
                param.name.clone()
            } else {
                format!("{}/{}", param.module, param.name)
            };
            ctx.text(
                -w / 2.0 + 125.0,
                y,
                &self.font,
                short(&label, 22),
                15.0,
                550.0,
                rgb(220, 215, 232),
                0.8,
            );
            let normalized = if param.max > param.min {
                ((param.value - param.min) / (param.max - param.min)).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            ctx.rectangle(x, y - 10.0, half, 3.0, rgb(65, 57, 82), 0.8);
            ctx.circle(
                x - half + 2.0 * half * normalized,
                y - 10.0,
                6.0,
                if param.readonly {
                    rgb(120, 115, 130)
                } else {
                    rgb(103, 237, 175)
                },
                0.81,
            );
            ctx.text(
                x,
                y + 11.0,
                &self.font,
                short(&param.text, 42),
                14.0,
                500.0,
                rgb(220, 215, 232),
                0.81,
            );
            if down
                && !param.readonly
                && hit(ctx.mouse_position(), x, y - 10.0, half, 12.0)
                && param.max > param.min
            {
                let mx = ctx.mouse_position().unwrap()[0];
                let value = param.min
                    + (((mx - x + half) / (2.0 * half)).clamp(0.0, 1.0) as f64)
                        * (param.max - param.min);
                if let Some(session) = &mut self.runtime.session
                    && let Err(error) = session.set_parameter(id, param.id, value)
                {
                    self.runtime.message = error;
                }
            }
        }
        ctx.text(
            0.0,
            -h / 2.0 + 28.0,
            &self.font,
            short(&self.runtime.message, 85),
            14.0,
            500.0,
            rgb(232, 163, 61),
            0.8,
        );
    }
}

fn draw_details(ctx: &mut Context, font: &Arc<[u8]>, card: &Card) {
    let Some([mx, my]) = ctx.mouse_position() else {
        return;
    };
    let (w, h) = ctx.size();
    let rows = [
        format!("{}  {}", card.name, card.details.version),
        format!("Vendor: {}", card.details.vendor),
        format!("{:?} → {:?}", card.inputs, card.outputs),
        card.details.description.clone(),
        card.path.display().to_string(),
    ];
    let half = (w / 2.0 - 16.0).min(330.0);
    let x = (mx + half + 15.0).clamp(-w / 2.0 + half + 8.0, w / 2.0 - half - 8.0);
    let y = (my + 75.0).clamp(-h / 2.0 + 70.0, h / 2.0 - 70.0);
    ctx.rectangle(x, y, half, 65.0, rgb(13, 10, 22), 0.9);
    for (i, row) in rows.iter().enumerate() {
        ctx.text(
            x,
            y + 44.0 - i as f32 * 22.0,
            font,
            short(row, (half / 4.6) as usize),
            14.0,
            550.0,
            rgb(230, 230, 234),
            0.91,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn internal_and_external_drops_use_the_same_four_zones() {
        let slots = [
            Slot {
                id: NodeId(1),
                x: 0.0,
                y: 0.0,
            },
            Slot {
                id: NodeId(3),
                x: 224.0,
                y: 0.0,
            },
        ];
        for (point, side) in [
            ([-100.0, 0.0], Side::Left),
            ([100.0, 0.0], Side::Right),
            ([0.0, 75.0], Side::Above),
            ([0.0, -75.0], Side::Below),
        ] {
            assert_eq!(
                target_at(Some(point), &slots, None, 200.0, -150.0, 1000.0),
                Some(Target {
                    card: NodeId(1),
                    side
                })
            );
            assert_eq!(
                target_at(Some(point), &slots, Some(NodeId(3)), 200.0, -150.0, 1000.0),
                Some(Target {
                    card: NodeId(1),
                    side
                })
            );
        }
        assert!(target_at(Some([0.0, 250.0]), &slots, None, 200.0, -150.0, 1000.0).is_none());
    }
    #[test]
    fn parallel_cards_are_stacked_and_stages_remain_left_to_right() {
        let groups = vec![vec![NodeId(1), NodeId(3)], vec![NodeId(5)]];
        let layout = geometry(&groups, 1040.0, 580.0, 0.0, 0.0);
        assert_eq!(layout.slots[0].x, layout.slots[1].x);
        assert!(layout.slots[0].y > layout.slots[1].y);
        assert!(layout.slots[2].x > layout.slots[0].x);
    }
    #[test]
    fn a_slow_drag_trims_finer_than_a_fast_one() {
        // Standing still leaves the trim exactly where it was.
        assert_eq!(gain_drag_delta(0.0, 3.0), 3.0);
        // A slow step — a couple of pixels a frame — stays sub-decibel
        // by design, which is the whole point of the relative drag.
        assert!((1..=2).all(|px| gain_drag_delta(px as f32, 0.0).abs() < 0.15));
        // The response grows super-linearly with drag speed: eight times
        // the pixels land far more than eight times the trim, so a flick
        // sweeps the range a careful drag would take seconds to cross.
        let slow = gain_drag_delta(2.0, 0.0);
        let fast = gain_drag_delta(16.0, 0.0);
        assert!(fast > 8.0 * slow);
        // The trim answers either direction, evenly and monotonically.
        assert!(gain_drag_delta(1.0, 0.0) > 0.0);
        assert_eq!(gain_drag_delta(-1.0, 0.0), -gain_drag_delta(1.0, 0.0));
        for px in 1..20 {
            let px = px as f32;
            assert!(gain_drag_delta(px + 1.0, 0.0) > gain_drag_delta(px, 0.0));
        }
        // And the ±24 dB the track spans bounds any flick.
        assert_eq!(gain_drag_delta(500.0, 20.0), 24.0);
        assert_eq!(gain_drag_delta(-500.0, -20.0), -24.0);
    }
    #[test]
    fn the_editor_accepts_plain_decibel_numbers() {
        assert_eq!(parse_gain("6"), Some(6.0));
        assert_eq!(parse_gain("-3,5"), Some(-3.5));
        assert_eq!(parse_gain(" 30 "), Some(24.0));
        assert_eq!(parse_gain("dB"), None);
        assert_eq!(parse_gain(""), None);
        // "inf" parses as a float, but not as a gain.
        assert_eq!(parse_gain("inf"), None);
    }
    #[test]
    fn the_gain_line_marks_the_trim_on_the_meters_scale() {
        // 6 dB of gain moves the white line 14 px — the same distance
        // the meter's own fill moves for 6 dB, and where the gray marks
        // sit, so the trim reads against the meter's scale directly.
        assert_eq!(gain_line_y(6.0, 0.0) - gain_line_y(0.0, 0.0), 14.0);
        assert_eq!(gain_line_y(0.0, 0.0), 0.0);
        // The line lands where the drag or the editor put it, at any
        // card height.
        for (db, wire_y) in [(-24.0, -140.0), (-12.0, 0.0), (12.0, 7.5), (24.0, 200.0)] {
            let y = gain_line_y(db, wire_y);
            // The line sits exactly where the meter's own 48 dB over
            // 112 px scale puts that gain.
            let scale = wire_y - TRACK_HALF + 2.0 * TRACK_HALF * ((db + 24.0) / 48.0);
            assert!((y - scale).abs() < 1e-4, "{db} dB at {wire_y}");
        }
        // Values past the track ends draw at the ends, like the drag
        // clamps them.
        assert_eq!(gain_line_y(99.0, 0.0), gain_line_y(24.0, 0.0));
        assert_eq!(gain_line_y(-99.0, 0.0), gain_line_y(-24.0, 0.0));
    }
    #[test]
    fn the_trim_readouts_sit_in_the_card_corners() {
        // The output trim lives in the card's bottom-right corner and
        // the input trim in its top-left — each number at the corner of
        // the edge its wire leaves (or enters) — and the two sit exactly
        // opposite each other across the card's centre.
        let out = gain_center(0.0, 0.0, true);
        let input = gain_center(0.0, 0.0, false);
        assert!(out[0] > 0.0 && out[1] < 0.0);
        assert!(input[0] < 0.0 && input[1] > 0.0);
        assert_eq!([out[0], out[1]], [-input[0], -input[1]]);

        // Each box sits a uniform inset inside its corner, on both
        // axes: the card's own colour shows on every side, so the box
        // reads as part of the corner instead of spilling over its
        // border.
        assert!((CARD_WIDTH / 2.0 - out[0].abs() - GAIN_HALF_W - GAIN_INSET).abs() < 1e-3);
        assert!((TRACK_HALF - out[1].abs() - GAIN_HALF_H - GAIN_INSET).abs() < 1e-3);

        // And the field stops short of the meter track it sits under, so
        // a press on the track's inner edge still starts a drag instead
        // of opening the editor.
        let track_inner = wire_box_x(0.0, true).abs() - 4.5;
        assert!(out[0].abs() + GAIN_HALF_W < track_inner);
    }
    #[test]
    fn the_layout_line_hangs_midway_under_the_name() {
        // The `stereo → stereo` line is centred on the midpoint between
        // the plugin's name and the gain readout that shares the card:
        // two lines pinned to the corners, the third hanging between them
        // rather than drifting up to one or down to the other.
        for card_y in [-140.0, 0.0, 7.5, 200.0] {
            let name = card_y + NAME_TEXT_Y;
            let readout = gain_center(0.0, card_y, true)[1];
            let line = layout_text_y(card_y);
            assert!((line - (name + readout) / 2.0).abs() < 1e-4, "at {card_y}");
            // Strictly between the two, so the line can never land on an
            // anchor it is measured between, whatever the card's height.
            assert!(line < name && line > readout, "at {card_y}");
            // The gap is measured inside the card, so scrolling carries
            // name, line and readout along as one piece.
            assert!((line - layout_text_y(0.0) - card_y).abs() < 1e-4);
        }
    }
    #[test]
    fn an_unopened_meter_reads_its_gain_from_the_engine() {
        // The readout stands on every card, and a card whose stub was
        // never clicked has no meter to ask — so its number comes out of
        // the engine's slot table, which is what is actually applying the
        // gain. No wire at all is unity, and a reading is clamped to the
        // ±24 dB the white line has room to draw.
        assert_eq!(engine_gain_db(None), 0.0);
        assert_eq!(engine_gain_db(Some(1.0)), 0.0);
        assert!((engine_gain_db(Some(0.5)) + 6.0206).abs() < 1e-3);
        assert_eq!(engine_gain_db(Some(100.0)), 24.0);
        // A silent wire is `0.0` linear, whose dB is −∞; that is the floor
        // of the scale, not a number to print.
        assert_eq!(engine_gain_db(Some(0.0)), -24.0);
        assert_eq!(engine_gain_db(Some(0.0001)), -24.0);
    }
    #[test]
    fn a_collapsed_card_reads_the_same_trim_as_an_expanded_one() {
        // The corner is the rack's own furniture: it is drawn from the
        // rack's state, never from anything a plugin is asked about, and
        // the meter's expansion is not an input to it. So a wire shows its
        // trim before its stub was ever clicked and while its track is
        // collapsed, and the number does not move when the track opens.
        let expanded = WireMeter {
            peak: PeakMeter::new(0),
            gain_db: -6.5,
            open: true,
            open_circuit: false,
        };
        let collapsed = WireMeter {
            peak: PeakMeter::new(0),
            gain_db: -6.5,
            open: false,
            open_circuit: false,
        };
        // The engine's copy is deliberately wrong here: where a meter
        // exists, the meter is what the rack reads and re-states.
        assert_eq!(corner_reading(Some(&expanded), Some(9.9)).0, -6.5);
        assert_eq!(corner_reading(Some(&collapsed), Some(9.9)).0, -6.5);
        // And with no meter at all the corner still reads the applied gain.
        assert!((corner_reading(None, Some(0.5)).0 + 6.0206).abs() < 1e-3);
        assert_eq!(corner_reading(None, None), (0.0, false));
    }

    /// `in → [a] → [b ∥ c] → [d] → out`, the shape `runtime::chain` wires:
    /// stages in series, `b` and `c` in parallel inside the middle one.
    fn rack() -> Vec<Vec<NodeId>> {
        vec![vec![NodeId(1)], vec![NodeId(3), NodeId(5)], vec![NodeId(7)]]
    }
    fn covers(from: MeterPos) -> Vec<MeterPos> {
        acknowledged(from, &rack())
    }
    /// Where a stage's card stands, in the same units as [`wire_targets`]: the
    /// rack's own inset from `left`, plus 224 px for every stage to its left.
    fn card_x(index: usize) -> f32 {
        -158.0 + 183.0 + index as f32 * 224.0
    }
    /// One side of one card's wiring, as the operator can aim at it.
    struct Click {
        card: usize,
        /// The card's own centre, so a test can find its stub box.
        x: f32,
        side: bool,
        y: f32,
        from: f32,
        to: f32,
    }
    /// Every click target in a rack laid out the way the draw pass lays one
    /// out, the cards numbered in the order the pass walks them. `stages` is how
    /// many cards stand in each stage, from left to right. The geometry is the
    /// rack's: a stage on its own sits on the line signals flow at, and a stage
    /// holding several cards hangs them a row apart, centred on that line.
    fn wire_targets(stages: &[usize]) -> Vec<Click> {
        let flow = 0.0;
        let row_step = 128.0;
        let out_wire = junction_x(card_x(stages.len() - 1), true) + 60.0 - 25.0;
        let mut targets = Vec::new();
        let mut previous_x = -158.0 + 25.0;
        let mut card = 0;
        for (index, count) in stages.iter().enumerate() {
            let x = card_x(index);
            let shared = *count > 1;
            let last = index + 1 == stages.len();
            let fed = index > 0;
            let merge = junction_x(x, true);
            for row in 0..*count {
                let y = if shared {
                    flow + (row as f32 - (*count as f32 - 1.0) / 2.0) * row_step
                } else {
                    flow
                };
                for side in [false, true] {
                    let (from, to) = wire_click(
                        x,
                        side,
                        shared,
                        fed,
                        previous_x,
                        if last { out_wire } else { merge },
                    );
                    targets.push(Click {
                        card,
                        x,
                        side,
                        y,
                        from,
                        to,
                    });
                }
                card += 1;
            }
            previous_x = merge;
        }
        targets
    }
    #[test]
    fn a_clip_reports_itself_and_everything_the_signal_reaches_after() {
        let out = covers(MeterPos::Wire(1, true));
        for pos in [
            MeterPos::Wire(1, true),
            MeterPos::Wire(3, false),
            MeterPos::Wire(3, true),
            MeterPos::Wire(5, false),
            MeterPos::Wire(5, true),
            MeterPos::Wire(7, false),
            MeterPos::Wire(7, true),
            MeterPos::MasterOut,
        ] {
            assert!(
                out.contains(&pos),
                "{pos:?} is downstream of a clip at the first card's output, and should have been cleared with it"
            );
        }
        assert_eq!(out.len(), 8, "and nothing else is: {out:?}");
        assert!(
            !out.contains(&MeterPos::Wire(1, false)),
            "the input meter sits before this one, so it never carried the clip"
        );
    }
    #[test]
    fn a_parallel_card_keeps_its_report() {
        // `c` beside `b` in the same stage: the two are branches, and the
        // signal that clipped through `b` never passed through `c`.
        let out = covers(MeterPos::Wire(3, false));
        assert!(
            out.contains(&MeterPos::Wire(3, true)),
            "in and out of the same card are the same signal"
        );
        assert!(out.contains(&MeterPos::Wire(7, true)) && out.contains(&MeterPos::MasterOut));
        for pos in [
            MeterPos::Wire(5, false),
            MeterPos::Wire(5, true),
            MeterPos::Wire(1, false),
            MeterPos::Wire(1, true),
            MeterPos::MasterIn,
        ] {
            assert!(
                !out.contains(&pos),
                "{pos:?} is not downstream of a clip inside card 3"
            );
        }
    }
    #[test]
    fn the_input_meter_answers_for_the_rack_and_the_output_for_nothing() {
        let inwards = covers(MeterPos::MasterIn);
        assert_eq!(inwards.len(), 10, "itself, every wire, and the output");
        for id in [1, 3, 5, 7] {
            assert!(inwards.contains(&MeterPos::Wire(id, false)));
            assert!(inwards.contains(&MeterPos::Wire(id, true)));
        }
        assert!(inwards.contains(&MeterPos::MasterOut));
        assert_eq!(
            covers(MeterPos::MasterOut),
            vec![MeterPos::MasterOut],
            "nothing is after the end of the chain"
        );
    }
    #[test]
    fn no_meter_is_answered_for_twice() {
        for from in [
            MeterPos::MasterIn,
            MeterPos::MasterOut,
            MeterPos::Wire(1, false),
            MeterPos::Wire(3, true),
            MeterPos::Wire(7, false),
        ] {
            let out = covers(from);
            let mut sorted = out.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(
                sorted.len(),
                out.len(),
                "the set for {from:?} repeats: {out:?}"
            );
        }
    }
    #[test]
    fn a_meter_no_longer_in_the_layout_owes_only_what_follows_it() {
        // Its card is gone, so there is no stage to be downstream of; the
        // rack's own output meter still sits after wherever it was.
        let out = acknowledged(MeterPos::Wire(99, false), &rack());
        assert_eq!(
            out,
            vec![
                MeterPos::Wire(99, false),
                MeterPos::Wire(99, true),
                MeterPos::MasterOut
            ]
        );
    }
    #[test]
    fn a_collapsed_meter_still_reports_the_clip_it_remembered() {
        // The corner number is red off `clipping`, so a latch has to reach it
        // even though the track is hidden — that is the point of a latch.
        let mut peak = PeakMeter::latching(0);
        peak.tick(-18.0, 1.4, 16);
        for t in (16..4_000).step_by(16) {
            peak.tick(-60.0, -60.0, t);
        }
        assert!(peak.peak_db() < 0.0, "the marker is back in the scale");
        let collapsed = WireMeter {
            peak,
            gain_db: 0.0,
            open: false,
            open_circuit: false,
        };
        assert!(
            corner_reading(Some(&collapsed), None).1,
            "the clip is still on record"
        );
        let mut cleared = collapsed;
        cleared.peak.clear_clip();
        assert!(
            !corner_reading(Some(&cleared), None).1,
            "and clicking the meter body is what takes it off"
        );
    }
    #[test]
    fn every_wire_carries_a_meter_before_anyone_opens_one() {
        use std::collections::BTreeMap;
        // The bug this pins: a stub whose track was never clicked had no
        // meter behind it, so it could not detect its own clip and stayed
        // green through anything. Watching a wire is what the rack does on
        // its own; opening a track is the operator's, and the two are not
        // the same act.
        let mut meters: BTreeMap<(u64, bool), WireMeter> = BTreeMap::new();
        watch_wires(&mut meters, [1, 3], |_, _| None, |_| None, |_, _| false, 0);
        let keys: Vec<(u64, bool)> = meters.keys().copied().collect();
        assert_eq!(keys, vec![(1, false), (1, true), (3, false), (3, true)]);
        for wire in meters.values() {
            assert!(!wire.open, "metered, not on screen");
            assert!(!wire.peak.clipping());
        }
    }
    #[test]
    fn a_meter_nobody_asked_for_adopts_the_trim_already_in_force() {
        use std::collections::BTreeMap;
        // Key 9 is `4 << 1 | true`: the output wire of card 4, held at half
        // by the engine. A meter born around it opens where the gain is, so
        // the white line does not have to be chased down after the fact.
        let mut meters: BTreeMap<(u64, bool), WireMeter> = BTreeMap::new();
        watch_wires(
            &mut meters,
            [4],
            |_, _| None,
            |key| (key == 9).then_some(0.5),
            |_, _| false,
            0,
        );
        assert_eq!(meters[&(4, false)].gain_db, 0.0, "an untrimmed wire");
        assert!(
            (meters[&(4, true)].gain_db + 6.0206).abs() < 0.01,
            "and a trimmed one reads its trim: {:?}",
            meters[&(4, true)].gain_db
        );
    }
    #[test]
    fn a_saved_trim_beats_the_engine_when_a_meter_is_born() {
        use std::collections::BTreeMap;
        // Two sources for one number, and the order between them has to be
        // written down somewhere: the rack remembers trims in its settings
        // file, while the engine only knows what it is applying right now.
        // On a cold start the engine is at unity for every wire, so a
        // remembered trim has to win or the restored gain staging would be
        // drawn at 0 dB while the file says otherwise. An unrecorded wire
        // still follows the engine, and one with neither stays at unity.
        let mut meters: BTreeMap<(u64, bool), WireMeter> = BTreeMap::new();
        watch_wires(
            &mut meters,
            [7, 8],
            |id, side| (id == 7 && !side).then_some(-9.0),
            |key| (key == (7 << 1 | 1) || key == (8 << 1)).then_some(0.25),
            |_, _| false,
            0,
        );
        assert!(
            (meters[&(7, false)].gain_db + 9.0).abs() < 1e-6,
            "the file's number, not the engine's: {:?}",
            meters[&(7, false)].gain_db
        );
        assert!(
            (meters[&(7, true)].gain_db + 12.0412).abs() < 0.01,
            "unrecorded, so the engine's quarter gain: {:?}",
            meters[&(7, true)].gain_db
        );
        assert!(
            (meters[&(8, false)].gain_db + 12.0412).abs() < 0.01,
            "same for a card the file never mentioned"
        );
        assert_eq!(
            meters[&(8, true)].gain_db,
            0.0,
            "and unity where neither spoke"
        );
    }
    #[test]
    fn watching_a_wire_leaves_an_expanded_meter_where_it_was() {
        use std::collections::BTreeMap;
        // Called every frame, so it may only ever fill a gap: the trim the
        // operator set, the track they left open and a clip they have not
        // acknowledged are all theirs, not this pass's to reset.
        let mut meters: BTreeMap<(u64, bool), WireMeter> = BTreeMap::new();
        watch_wires(&mut meters, [2], |_, _| None, |_| None, |_, _| false, 0);
        let wire = meters.get_mut(&(2, true)).unwrap();
        wire.open = true;
        wire.gain_db = -12.0;
        wire.peak.tick(-18.0, 1.4, 16);
        watch_wires(
            &mut meters,
            [2],
            |_, _| Some(4.0),
            |_| None,
            |_, _| false,
            1_000,
        );
        let wire = &meters[&(2, true)];
        assert!(wire.open, "still expanded");
        assert_eq!(wire.gain_db, -12.0, "still the operator's trim");
        assert!(wire.peak.clipping(), "still reporting the clip");
        assert_eq!(meters.len(), 2, "and nothing new was invented");
    }
    #[test]
    fn a_stub_never_opened_still_catches_the_clip_and_holds_it() {
        use std::collections::BTreeMap;
        // Both halves of the report in one: the meter is there from the
        // start, and it is the latching kind, so the red it earns in the
        // dark is still on the stub when the operator finally looks.
        let mut meters: BTreeMap<(u64, bool), WireMeter> = BTreeMap::new();
        watch_wires(&mut meters, [6], |_, _| None, |_| None, |_, _| false, 0);
        let wire = meters.get_mut(&(6, false)).unwrap();
        wire.peak.tick(-18.0, 1.4, 16);
        for t in (16..4_000).step_by(16) {
            wire.peak.tick(-60.0, -60.0, t);
        }
        assert!(
            wire.peak.clipping() && !wire.open,
            "red on a meter nobody opened"
        );
        wire.peak.clear_clip();
        assert!(
            !wire.peak.clipping(),
            "and one click of that stub clears it"
        );
    }
    #[test]
    fn a_minimised_meter_colours_itself_against_the_gate() {
        use std::collections::BTreeMap;
        let wire = rgb(107, 114, 128);
        let mut meters: BTreeMap<(u64, bool), WireMeter> = BTreeMap::new();
        watch_wires(&mut meters, [5], |_, _| None, |_| None, |_, _| false, 0);
        // Nothing has been played through the rack: a meter that never heard a
        // sample sits at digital silence, well under any gate on offer, and
        // the stub is as bare as it was before the gate existed. A wire with
        // no meter behind it looks exactly the same, which is the point — the
        // stub's colour means "signal", not "meter".
        let quiet = meters.get(&(5, false)).unwrap();
        assert_eq!(stub_color(Some(quiet), -60.0, wire), wire);
        assert_eq!(stub_color(None, -60.0, wire), wire, "meter or none");
        // Something is coming through the input wire now.
        let meter = meters.get_mut(&(5, false)).unwrap();
        for t in (16..=160).step_by(16) {
            meter.peak.tick(-20.0, -20.0, t);
        }
        let loud = meters.get(&(5, false)).unwrap();
        assert!(loud.peak.level_db() > -21.0, "the bar got there");
        assert_eq!(
            stub_color(Some(loud), -60.0, wire),
            LEVEL,
            "and the stub says so"
        );
        // The same stub, the same signal, a gate moved above it: the number is
        // doing the work and not the mere fact that a meter exists.
        assert_eq!(stub_color(Some(loud), -12.0, wire), wire);
        // At the gate is over it. Silence against a gate of -120 dBFS is the
        // case where the two are exactly equal, and it reads as signal.
        let idle = new_wire_meter(0.0, 0);
        assert_eq!(stub_color(Some(&idle), -120.0, wire), LEVEL);
        assert_eq!(stub_color(Some(&idle), -119.0, wire), wire);
        // An expanded track keeps the colour it has always had. The bar beside
        // it already says how much is coming through; the stub's job there is
        // to say the track is open. The gate goes above the signal so that the
        // only thing left that could brighten this stub is the open flag.
        let meter = meters.get_mut(&(5, false)).unwrap();
        meter.open = true;
        assert_eq!(
            stub_color(meters.get(&(5, false)), -12.0, wire),
            LEVEL,
            "green for open, under the gate"
        );
        // A clip outranks both. It was latched while the meter was loud, and
        // the red has to survive the long way down through the gate and past
        // it — that fall is exactly the interval in which a plain level rule
        // would have quietly painted the stub grey again.
        let meter = meters.get_mut(&(5, false)).unwrap();
        meter.peak.tick(-20.0, 1.4, 200);
        for t in (216..4_000).step_by(16) {
            meter.peak.tick(-120.0, -120.0, t);
        }
        let burnt = meters.get_mut(&(5, false)).unwrap();
        assert!(burnt.peak.clipping() && burnt.peak.level_db() < -60.0);
        assert_eq!(
            stub_color(Some(burnt), -60.0, wire),
            nice_common::knob::RED,
            "red while the bar is well under the gate"
        );
        burnt.peak.clear_clip();
        assert_eq!(
            stub_color(Some(burnt), -60.0, wire),
            LEVEL,
            "and the track it was drawn on is still open"
        );
        // Close it and the red is gone, not remembered: the clip was the only
        // thing holding the stub bright and it has drained far below the gate.
        burnt.open = false;
        assert_eq!(
            stub_color(Some(burnt), -60.0, wire),
            wire,
            "red retired with the clip"
        );
    }
    #[test]
    fn a_meter_is_born_knowing_its_wire_is_open() {
        use std::collections::BTreeMap;
        let mut meters: BTreeMap<(u64, bool), WireMeter> = BTreeMap::new();
        // A rack resumed with a cable left open has heard no audio yet, so the
        // engine's slot table holds no key at all. What the file remembered is
        // the only place a stub can learn it is an open circuit, and a meter
        // that had to wait for the first sample would draw one frame of whole
        // wire before the tick pass re-stated the break.
        watch_wires(
            &mut meters,
            [5],
            |_, _| None,
            |_| None,
            |id, side| id == 5 && !side,
            0,
        );
        assert!(meters.get(&(5, false)).unwrap().open_circuit);
        assert!(
            !meters.get(&(5, true)).unwrap().open_circuit,
            "and the wire nobody broke is whole"
        );
        // Called again the next frame — as it is, every frame — this leaves
        // what is already there alone. `open_of` reports what was *remembered*;
        // since the first frame the operator may have closed the wire, and the
        // rack's own state is the live one.
        meters.get_mut(&(5, false)).unwrap().open_circuit = false;
        watch_wires(
            &mut meters,
            [5],
            |_, _| None,
            |_| None,
            |id, side| id == 5 && !side,
            16,
        );
        assert!(
            !meters.get(&(5, false)).unwrap().open_circuit,
            "a birth reading does not overrule the operator"
        );
    }
    #[test]
    fn a_wire_left_open_wears_amber_and_a_clip_outranks_it() {
        use std::collections::BTreeMap;
        let wire = rgb(107, 114, 128);
        let mut meters: BTreeMap<(u64, bool), WireMeter> = BTreeMap::new();
        watch_wires(&mut meters, [5], |_, _| None, |_| None, |_, side| !side, 0);
        // Grey is what a wire that is passing nothing has always looked like, so
        // it cannot also mean "someone opened this circuit on purpose" — the
        // operator would have no way to tell a break from a rack at rest.
        let broken = meters.get(&(5, false)).unwrap();
        assert_eq!(
            stub_color(Some(broken), -60.0, wire),
            nice_common::knob::AMBER
        );
        assert_eq!(
            stub_color(meters.get(&(5, true)), -60.0, wire),
            wire,
            "the wire still closed next door keeps the plain wire colour"
        );
        // Amber outranks green: an expanded track whose wire is open must not
        // read as merely idle, and the gate is set above anything this meter has
        // heard so that the open flag is the only rung left to decide it.
        let meter = meters.get_mut(&(5, false)).unwrap();
        meter.open = true;
        assert_eq!(
            stub_color(meters.get(&(5, false)), -12.0, wire),
            nice_common::knob::AMBER,
            "open by hand, not by volume"
        );
        // A clip outranks amber. The break is a colour the operator chose and
        // can look up again in a second; the peak nobody has acknowledged yet is
        // the one that must not disappear underneath it.
        let meter = meters.get_mut(&(5, false)).unwrap();
        meter.peak.tick(-20.0, 1.4, 200);
        for t in (216..4_000).step_by(16) {
            meter.peak.tick(-120.0, -120.0, t);
        }
        let burnt = meters.get_mut(&(5, false)).unwrap();
        assert!(burnt.open_circuit && burnt.peak.clipping());
        assert_eq!(stub_color(Some(burnt), -60.0, wire), nice_common::knob::RED);
        burnt.peak.clear_clip();
        assert_eq!(
            stub_color(Some(burnt), -60.0, wire),
            nice_common::knob::AMBER,
            "and the break is still there to be read once the clip has been"
        );
    }
    #[test]
    fn a_broken_wire_leaves_a_hole_its_own_box_cannot_fill() {
        // The input wire of a card standing at the origin: the signal arrives
        // from the left, the stub box sits 6 px inside the card's edge.
        let to = -CARD_WIDTH / 2.0;
        let box_x = wire_box_x(0.0, false);
        assert_eq!(wire_end(to, box_x, false), to, "closed, it runs on anyway");
        let open = wire_end(to, box_x, true);
        assert_eq!(open, box_x - STUB_HALF - BREAK_GAP, "cut short of the box");
        assert!(
            to - open > 2.0 * STUB_HALF + BREAK_GAP,
            "the hole is {hole} px for an {} px box, so the mark sits in air and \
             not on a line",
            2.0 * STUB_HALF,
            hole = to - open,
        );
        // The cut only ever shortens a stretch. Where a wire stops before it even
        // reaches its box, breaking it must not draw it further than it was going
        // — that would hang a piece of wire past a column it never touched.
        let short = box_x - 20.0;
        assert!(short < open);
        assert_eq!(wire_end(short, box_x, true), short);
        assert!(wire_end(to, box_x, true) <= to);
    }
    #[test]
    fn no_two_wires_ask_for_the_same_click() {
        for stages in [
            &[1usize][..],
            &[1, 1],
            &[1, 1, 1],
            &[2, 1],
            &[1, 2],
            &[1, 2, 1],
            &[2, 2],
        ] {
            let targets = wire_targets(stages);
            for (index, wire) in targets.iter().enumerate() {
                let Click {
                    card,
                    x,
                    side,
                    from,
                    to,
                    ..
                } = *wire;
                // A target at least as wide as its own box, so a switch is
                // never reachable only by the two pixels beside it.
                assert!(
                    to - from >= 2.0 * STUB_HALF,
                    "card {card} {side} in {stages:?} has only {} px of wire to aim at",
                    to - from,
                );
                // The box stands inside its own stretch. The box is a target in
                // its own right too, so this is what guarantees an open wire
                // stays as easy to close as it was to open even where the break
                // swallows the line drawn beside it.
                let box_x = wire_box_x(x, side);
                assert!(
                    from <= box_x && box_x <= to,
                    "card {card} {side} in {stages:?} cannot be reached at its own box",
                );
                for other in &targets[index + 1..] {
                    let same_row = (wire.y - other.y).abs() <= 2.0 * BREAK_HALF;
                    let overlaps = from <= other.to && other.from <= to;
                    assert!(
                        !(same_row && overlaps),
                        "{stages:?}: card {card} {side} [{from}..={to}] and card \
                         {} {} [{:?}..={:?}] both claim the same touch",
                        other.card,
                        other.side,
                        other.from,
                        other.to,
                    );
                }
            }
        }
    }
    #[test]
    fn a_parallel_stages_columns_and_spine_belong_to_no_card() {
        // `in → [a] → [b ∥ c] → [d] → out`, with the middle stage shared.
        let stages = [1usize, 2, 1];
        let targets = wire_targets(&stages);
        let claimed = |column: f32| targets.iter().any(|t| t.from <= column && column <= t.to);
        let split = junction_x(card_x(1), false);
        let merge = junction_x(card_x(1), true);
        // The risers stand in these two columns and carry every card in the
        // stage, so neither column may be a wire's: a click aimed at the vertical
        // connection has to open nothing rather than guess which card meant.
        assert!(!claimed(split), "the split column is claimed");
        assert!(!claimed(merge), "the merge column is claimed");
        // The spine feeding them is the same matter one level up: that trunk
        // carries the signal that is about to be shared out, and breaking it
        // would be breaking both cards at once.
        let head = junction_x(card_x(0), true);
        assert!(
            !claimed((head + split) / 2.0),
            "the shared spine is claimed"
        );
        // A card's own short run beside its stub box is still its own.
        let meter = wire_box_x(card_x(1), false);
        assert!(claimed(meter), "a parallel card cannot reach its own box");
        // And the trunk that leaves the merge is one wire again — the merged
        // signal going into one card — so it is that card's input to break.
        let on = junction_x(card_x(2), false);
        assert!(
            claimed((merge + on) / 2.0),
            "the run out of a merge is nobody's"
        );
    }
}

#[cfg(test)]
mod bypass_tests {
    use super::*;
    #[test]
    fn bypass_wires_stay_outside_the_card_and_leads_end_at_the_minimised_meters() {
        let (x, y) = (200.0, 100.0);
        let path = bypass_path(x, y, 12.0);
        assert_eq!(path[0], [wire_box_x(x, false), y]);
        assert_eq!(path[3], [wire_box_x(x, true), y]);
        for pair in path.windows(2) {
            if pair[0][1] == pair[1][1] {
                assert!(pair[0][1] > y + TRACK_HALF);
            } else {
                assert!((pair[0][0] - x).abs() > CARD_WIDTH / 2.0);
            }
        }
        let tight = bypass_path(x, y, 2.0);
        assert!(
            tight[1][1] > y + TRACK_HALF && tight[1][1] < y + 116.0 - TRACK_HALF,
            "the bypass must also fit between densely stacked parallel cards"
        );
        for side in [false, true] {
            let (start, end) = card_lead(x, side, junction_x(x, false), junction_x(x, true), true);
            if side {
                assert_eq!(start, wire_box_x(x, true));
            } else {
                assert_eq!(end, wire_box_x(x, false));
            }
            let bx = wire_box_x(x, side);
            assert!(meter_hit(Some([bx, y]), x, y, side, false));
            assert!(!meter_hit(Some([bx, y + 35.0]), x, y, side, false));
            assert!(meter_hit(Some([bx, y + 35.0]), x, y, side, true));
        }
    }
}
