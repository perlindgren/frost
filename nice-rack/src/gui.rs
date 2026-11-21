mod flow_canvas;
mod timing;
use crate::{
    Settings,
    layout::{Side, Target},
    routing::NodeId,
    routing::audio::{MASTER_WIRE_KEY, card_wire_key},
    runtime::{Card, Junction, Runtime},
};
use frost::{Color, Context, Process, Scene, SceneNode, Shape};
use nice_clap_host::{Descriptor, Entry};
use nice_common::{
    meter::PeakMeter,
    ui::{AudioPanel, Event, PageScope},
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
    /// One peak-hold per drawn lane. A stereo wire — and the engine folds
    /// any wider wire into its second lane — shows lane 0 and lane 1 side
    /// by side; a mono wire has only lane 0 and its second lane stays
    /// silent, so every folded reading below, which the stub colour and
    /// the corner digits take, is exactly what a single meter used to say.
    peaks: [PeakMeter; 2],
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
impl WireMeter {
    /// The meter's level across its lanes: whatever is loudest. The rack
    /// reads this where it draws or reports one value per wire — the stub
    /// colour, the corner digits — and reads the lanes separately only
    /// where it draws the track.
    fn level_db(&self) -> f32 {
        self.peaks[0].level_db().max(self.peaks[1].level_db())
    }
    /// A clip on either lane clips the wire: the operator has to
    /// acknowledge the peak the wire caught, not the lane that caught it.
    fn clipping(&self) -> bool {
        self.peaks.iter().any(PeakMeter::clipping)
    }
    /// Acknowledging clears the latch on both lanes, so one click on the
    /// wire is enough however many channels it carries.
    fn clear_clip(&mut self) {
        for lane in &mut self.peaks {
            lane.clear_clip();
        }
    }
    /// Start both lanes fresh at `time`, as a latching meter is born.
    fn restart(&mut self, time: u64) {
        for lane in self.peaks.iter_mut() {
            *lane = PeakMeter::latching(time);
        }
    }
}
/// Track presses independently of the text editor: its first click opens the
/// editor, so the second press must still be able to reset that same value.
#[derive(Default)]
struct GainClick {
    previous: Option<((u64, bool), std::time::Instant)>,
}
impl GainClick {
    fn press(
        &mut self,
        target: Option<(u64, bool)>,
        time: std::time::Instant,
    ) -> Option<(u64, bool)> {
        let previous = self.previous.take();
        let target = target?;
        if previous.is_some_and(|(old, at)| {
            old == target
                && time
                    .checked_duration_since(at)
                    .is_some_and(|elapsed| elapsed <= std::time::Duration::from_millis(350))
        }) {
            Some(target)
        } else {
            self.previous = Some((target, time));
            None
        }
    }
}
struct DropCache {
    root: crate::layout::Flow,
    moving: Option<u64>,
    targets: crate::layout::DropTargets,
}
struct RackUi {
    drop_cache: Option<DropCache>,
    runtime: Runtime,
    loader: Loader,
    font: Arc<[u8]>,
    icons: Arc<[u8]>,
    settings: Option<AudioPanel<crate::RackPreferences>>,
    settings_page: bool,
    /// Which half of the device the open settings page is about: the two
    /// endpoint cards each open their own side, the cog opens the whole of it.
    settings_scope: PageScope,
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
    /// Per-wire meter state, keyed `(plugin id, is_output)` — plus the
    /// master output meter under `(u64::MAX, false)`. Each entry's engine
    /// slot is reached through `wire_slot_key`: a card's two sides use
    /// `card_wire_key`, the master jack holds the single master slot.
    /// Every wire the rack carries has an entry here from the frame its
    /// card appeared, which is `watch_wires`'s job: its dB readout stands
    /// in the card's corner whether or not the meter is expanded, and its
    /// peak-hold keeps ticking either way, so the minimised stub can
    /// colour with the signal — including red, before anyone has clicked
    /// it once. Clicking the stub only collapses or re-expands the track.
    /// The gain trim itself also lives in the engine's slot table, so it
    /// survives closing the meter; this copy is what the white line and
    /// the readout draw, and what gets re-stated to the engine every
    /// tick. A card's entry outlives its card: ids are never handed out
    /// twice, so a leftover meter can only ever read silence.
    wire_meters: std::collections::BTreeMap<(u64, bool), WireMeter>,
    timing_meters: std::collections::BTreeMap<NodeId, TimingMeter>,
    /// Gain slider grab: `(plugin id — or `u64::MAX` for the master
    /// output — is_output, pointer height last frame)`. The drag is
    /// *relative*: each frame's pointer travel is what moves the trim, so
    /// the grab tracks the pointer, and the rack can scroll mid-drag
    /// without the gain jumping.
    gain_drag: Option<(u64, bool, f32)>,
    /// Open dB editor: `(plugin id — or `u64::MAX` for the master
    /// output — is_output)`. Clicking a corner readout edits that trim in
    /// place; the field's rect is recomputed from the card's live slot
    /// (or the OUT jack's live column) every frame, so scrolling carries
    /// the open editor along with its meter.
    gain_edit: Option<(u64, bool)>,
    gain_click: GainClick,
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
    /// A conversion circle being slid along its wire: `(node, where the
    /// pointer grabbed it along the wire, the offset it had when the grab
    /// started)`. The conversion is the engine's, not the canvas's, so this
    /// moves nothing but the mark; it is kept for the stretch of wire the
    /// circle was seated on and dropped the moment that stretch goes away.
    mark_drag: Option<(NodeId, f32, f32)>,
    /// How far each conversion circle has been nudged along its own wire, by
    /// the node the compiled graph gave it. Purely a viewing preference: the
    /// graph is built without it.
    mark_offsets: std::collections::BTreeMap<NodeId, f32>,
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
const JUNCTION_GAP: f32 = 32.0;
/// How far above and below a wire a right-click still counts as aimed at it.
/// The line itself is 2 px wide, so without some generosity here the operator
/// would have to be exact about a target they cannot make much wider.
const BREAK_HALF: f32 = 5.0;
/// Fixed visible cable break length, measured from the meter face.
const BREAK_GAP: f32 = 4.0;
/// How much of a card's run is given up at an end that stands in a column more
/// than one wire passes through. The risers of a parallel stage carry every card
/// in it and are nobody's to break, and where a riser crosses a card's own row
/// the two are the same pixels; two cards meeting in a merge column are the same
/// overlap in miniature. Stepping back hands the column to the wire that ends
/// there, so the operator can aim at it and never open two circuits at once.
const RISER_KEEP: f32 = 3.0;
/// Radius of the circle the canvas draws for a width conversion, and of the
/// mixer's `+` ring it shares the look with. Seven is a hair under the mixer's
/// own ring because the gutter between two facing cards is the tightest seat a
/// circle ever gets: 38 px between the card edges, of which the two stub boxes
/// and their air take most of what a wire can cross.
const MARK_RADIUS: f32 = 7.0;
/// Air left between a circle and anything its click must not steal.
const MARK_AIR: f32 = 1.0;
/// How far from a circle's centre a click still counts as aimed at it. Held to
/// the ring plus a pixel of air on purpose: in the gutter between two facing
/// cards the stub boxes' centres are 13 px from the middle of the window and
/// their facing faces 9 px, so a hit box of `MARK_RADIUS + 2.0` or more would
/// cover both boxes and leave the operator no way to break the wire the circle
/// sits on.
const MARK_HIT: f32 = MARK_RADIUS + MARK_AIR;
/// What a circle is drawn in when the pointer is on it — the same lift the
/// cards and stubs get, from the same family of greys.
const MARK_HOVER: Color = rgb(163, 169, 182);
/// Radius of the mixer's ring, which the canvas draws wherever several
/// branches are joined into one wire.
const MIXER_RING: f32 = 8.5;
/// How near the ring a drop must land to count as aimed at it: the ring
/// itself plus enough air that a steady hand never has to thread the needle
/// between it and the lane tails standing in the same column.
#[cfg(test)]
const JOIN_HIT: f32 = MIXER_RING + 22.0;
/// Plain split/join intersections have no glyph to leave clearance for.
/// Channel-conversion circles retain their separate radius and masking.
const RING_CLEAR: f32 = 0.0;
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
/// card's input — and in the last stage too, where the run beyond the merge
/// belongs to the master wire feeding the rack's `OUT` card. Where the stage holds several
/// branches, the junction columns and the risers between them belong to the
/// stage and not to any one card: a branch's head takes its input from the
/// split column and its tail feeds the merge column, while the cards a chain
/// runs through in between only reach the junctions beside themselves, where
/// the chain hands over from one card to the next. The vertical connections
/// are never anybody's to break.
///
/// `feed` is the column the wire arrives from on the left (the head feed's `IN`
/// jack for the first stage) and `out` the column it leaves for on the right:
/// the next card's junction within a chain, the stage's merge column at a
/// branch's end.
/// A head card arriving at a shared stage's split column ignores `feed`, which
/// stands further left again.
///
/// The intervals here meet at the columns and may touch there; [`wire_click`]
/// is what decides which wire a touch belongs to. The drawing of a wire is a
/// separate matter — the parts of a stage's wiring that cross rows are drawn one
/// row at a time, each cut at its own height — so nothing here is used to put ink
/// down.
fn wire_run(x: f32, side: bool, shared: bool, feed: f32, out: f32) -> (f32, f32) {
    let edge = CARD_WIDTH / 2.0;
    if side {
        (x + edge, out)
    } else {
        (if shared { junction_x(x, false) } else { feed }, x - edge)
    }
}
/// [`wire_run`] as a click target: the same stretch, minus the few pixels at an
/// end that stands in a column two wires meet in.
///
/// A card's output finishes in a column, and a wire starts in one — the riser
/// where the stage is shared, otherwise the previous stage's merge column.
/// Wherever that is, the column itself is given to the wire that ends there and
/// the wire that starts there steps back by [`RISER_KEEP`], so a click opens
/// exactly one circuit: aiming at a riser hits neither of the wires that pass
/// it, and aiming where two cards' meters hand the signal over breaks the one
/// whose run finishes there. The two ends that stand at one of the rack's own
/// jacks — the head feed on the left, the master wire's run into the `OUT`
/// card on the right — are nobody's second wire, and are kept in full.
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
/// A fixed-length break on a horizontal cable, anchored at its meter face.
#[derive(Clone, Copy, Debug)]
struct WireGap {
    start: f32,
    end: f32,
    y: f32,
}
fn meter_gap(x: f32, y: f32, output: bool) -> WireGap {
    let face = x + if output { STUB_HALF } else { -STUB_HALF };
    if output {
        WireGap {
            start: face,
            end: face + BREAK_GAP,
            y,
        }
    } else {
        WireGap {
            start: face - BREAK_GAP,
            end: face,
            y,
        }
    }
}
fn for_connected_run(from: f32, to: f32, y: f32, gaps: &[WireGap], mut draw: impl FnMut(f32, f32)) {
    let mut cursor = from;
    while cursor < to {
        let next = gaps
            .iter()
            .filter(|gap| (gap.y - y).abs() < 0.5 && gap.end > cursor && gap.start < to)
            .min_by(|a, b| a.start.total_cmp(&b.start));
        let Some(gap) = next else {
            draw(cursor, to);
            break;
        };
        if gap.start > cursor {
            draw(cursor, gap.start);
        }
        cursor = cursor.max(gap.end);
    }
}
#[cfg(test)]
fn connected_runs(from: f32, to: f32, y: f32, gaps: &[WireGap]) -> Vec<(f32, f32)> {
    let mut runs = Vec::new();
    for_connected_run(from, to, y, gaps, |a, b| runs.push((a, b)));
    runs
}
/// The spacing between the parallel strokes that draw the channels of a
/// stereo wire. Deliberately small: two lines close together read as one
/// stereo wire, not as two wires somebody forgot to join, and at this gap
/// every click tolerance in the rack (`BREAK_HALF`, the stub boxes) still
/// covers both lanes, so a click opens the whole wire as it always did.
const WIRE_LANE_GAP: f32 = 4.0;
/// How far off the centre line lane `lane` of a `lanes`-wide wire sits.
/// The lanes straddle the wire's own line symmetrically, so a mono wire —
/// every wire before channel counts existed — sits exactly where it always
/// sat. Widths beyond two fold into the second lane the way the engine's
/// metering folds them, and the rack never draws more than two.
fn lane_offset(lane: usize, lanes: usize) -> f32 {
    (lane as f32 - (lanes.min(2) - 1) as f32 * 0.5) * WIRE_LANE_GAP
}
/// A stretch of wire, if there is one left to draw after the cut above.
/// A stereo wire is two parallel strokes a [`WIRE_LANE_GAP`] apart, the
/// pair centred on the line its mono self would have been drawn on; the
/// click targets in this file are still one per wire, lanes being a way
/// of seeing a wire and not a way of holding it.
fn wire_segment(
    ctx: &mut Context,
    from: f32,
    to: f32,
    y: f32,
    color: Color,
    lanes: usize,
    gaps: &[WireGap],
) {
    for_connected_run(from, to, y, gaps, |from, to| {
        for lane in 0..lanes.clamp(1, 2) {
            let y = y + lane_offset(lane, lanes);
            ctx.line(from, y, to, y, color, 2.0, 0.1);
        }
    });
}
/// A riser: a vertical stretch of wire at `x` between two heights, spread
/// into lanes the way [`wire_segment`] spreads a horizontal run. This is
/// what a stage's split and merge columns are: a stereo signal dropping
/// from the flow line onto a branch, or climbing back up to it, carries
/// its second channel with it as a second riser beside the first.
fn wire_riser(ctx: &mut Context, x: f32, from_y: f32, to_y: f32, color: Color, lanes: usize) {
    let (lo, hi) = if from_y <= to_y {
        (from_y, to_y)
    } else {
        (to_y, from_y)
    };
    for lane in 0..lanes.clamp(1, 2) {
        let x = x + lane_offset(lane, lanes);
        ctx.line(x, lo, x, hi, color, 2.0, 0.1);
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
/// and not a placeholder. A key the slot table does not hold is unity —
/// the value `corner_label` leaves without a corner at all.
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
        Some(meter) => (meter.gain_db, meter.clipping()),
        None => (engine_gain_db(engine_linear), false),
    }
}
/// What a card's corner prints for a trim, if it prints at all: the dB
/// value to one decimal, sign included — and nothing at unity, unless the
/// wire's own drag holds the pointer. While that drag runs the corner is
/// its live readout, so it prints straight through unity and falls blank
/// only when the release answers a wire at unity. The blank is decided on
/// the string the corner would have drawn, so a trim of −0.04 dB, which
/// would print `−0.0`, stands in the same blank as the exact `0.0` a
/// double-click reset leaves behind. The box follows the number: a corner
/// with nothing to print is not drawn at all, while its rect stays exactly
/// where it was — the click that opens the editor and the double-click
/// that resets never had to find a drawn box to land on.
fn corner_label(gain_db: f32, dragging: bool) -> Option<String> {
    let shown = format!("{:+.1}", gain_db);
    if shown == "+0.0" || shown == "-0.0" {
        dragging.then_some(shown)
    } else {
        Some(shown)
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
        peaks: [PeakMeter::latching(time); 2],
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
    if stub.is_some_and(|meter| meter.clipping()) {
        nice_common::knob::RED
    } else if stub.is_some_and(|meter| meter.open_circuit) {
        nice_common::knob::AMBER
    } else if stub.is_some_and(|meter| meter.open || meter.level_db() >= gate_db) {
        LEVEL
    } else {
        wire
    }
}
/// Put a meter on every wire the rack carries, keyed the way the GUI keys
/// its slots (`wire_slot_key`) — so the same entry that shows a card's wire
/// also holds its trim against the engine, and the master jack keyed
/// `(u64::MAX, false)` gets its one entry too. Meters are *born* here and not
/// at the first click on a stub:
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
        // The master jack is one wire — the rack's output. It has no
        // input side to meter: whatever feeds the jack is whatever the
        // mixer summed, and the jack's own meter reads the result.
        // The input jack is one wire too — the capture as it enters the
        // rack, after its trim — and it has no output side: the cards
        // downstream carry their own input meters from there.
        let sides: &[bool] = if id == MASTER_WIRE_KEY {
            &[false]
        } else if id == INPUT_JACK.0 {
            &[true]
        } else {
            &[false, true]
        };
        for &side in sides {
            let key = (id, side);
            if meters.contains_key(&key) {
                continue;
            }
            let gain_db = remembered_trim(
                remembered(id, side),
                wire_slot_key(id, side).and_then(&mut gain_of),
            );
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
/// given, so a value keeps the same pixels in the readout and in the
/// open field; the corner the readout blanks at unity is the one value
/// only the opened field spells out.
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
/// The key under which the engine registers one meter's wire slot,
/// shared by every place the GUI talks to `WireLevels`. A plugin card
/// owns two slots, one per side (`card_wire_key`); the rack's own
/// output jack — the meter keyed `(u64::MAX, false)` on the canvas —
/// owns the single master slot. The card ids the GUI hands out are far
/// below the engine's bound, so no card can reach the master key.
fn wire_slot_key(id: u64, side: bool) -> Option<u64> {
    if id == MASTER_WIRE_KEY {
        Some(MASTER_WIRE_KEY)
    } else {
        card_wire_key(id, side)
    }
}
/// The card's shape line (`stereo → stereo`), centred in the space the
/// name and the output readout leave: the exact midpoint of their two
/// anchors, so it reads as the middle voice of the card rather than as
/// a caption pinned under either number.
fn layout_text_y(y: f32) -> f32 {
    y + (NAME_TEXT_Y - GAIN_CORNER_Y) / 2.0
}
/// How far apart two neighbouring card columns stand. The gutter between
/// two facing stub boxes is this step less the 103 px to each box's outer
/// face, so it must leave real air — the wire run plus a whole conversion
/// circle with its pins (18 px across) — for the circle between two cards
/// to read as a circle on a wire and not as one piece of plumbing. The wider
/// step also leaves room between consecutive split/join columns.
const COLUMN_STEP: f32 = 288.0;
/// The capture jack the engine wires the rack's head against. The engine
/// names its own two jacks `NodeId(0)` and `NodeId(u64::MAX)`; the second is
/// the master wire the canvas already keys by [`MASTER_WIRE_KEY`], and this
/// is the first — which the canvas now draws as a card, so it needs the id
/// in its own terms. It carries one metered wire, keyed `(0, true)`: the
/// capture trimmed as it enters the rack, with a trim corner of its own and
/// an outgoing break switch for the feed to the whole graph.
const INPUT_JACK: NodeId = NodeId(0);
/// How an endpoint card says the rate it was opened at: kilohertz, the way
/// the device page spells it, and a dash where the rack has never opened a
/// stream at all — a rack that has applied nothing yet has no rate to show,
/// and inventing 48 kHz for it would be the card lying about the device.
fn rate_text(hz: Option<f32>) -> String {
    let Some(hz) = hz else {
        return "—".to_string();
    };
    let khz = hz / 1000.0;
    if (khz - khz.round()).abs() < 0.05 {
        format!("{} kHz", khz.round() as i64)
    } else {
        format!("{khz:.1} kHz")
    }
}
#[derive(Clone, Copy)]
struct Slot {
    id: NodeId,
    x: f32,
    y: f32,
}
#[derive(Clone)]
struct Geometry {
    slots: Vec<Slot>,
    /// The rack's own `IN` card, standing in the first column. Like `output`
    /// it is a slot as far as the canvas is concerned, and like it nothing
    /// places, deletes or drags it — but it carries no meter and no trim
    /// either, because nothing on the rack's side of it is measured.
    input: Slot,
    /// The rack's own `OUT` card: a slot like any other as far as the
    /// canvas is concerned — its own column, its own stub, its own
    /// corner readout — except that nothing ever places, deletes or
    /// drags it, so it stands apart from `slots` and every loop that
    /// walks them.
    output: Slot,
    top: f32,
    bottom: f32,
    flow: f32,
    max_vertical: f32,
}
/// One lane of a stage as the canvas sees it: its cards in order, and the
/// stage-local column span the lane occupies. A lane that runs the whole
/// stage spans `0..width`, exactly as every lane did before lanes learned
/// to start and stop partway along a stage; a shorter lane shares the stage
/// with its neighbours and opens or closes at one of the stage's interior
/// boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
struct GuiLane {
    /// The lane's cards, in the order the wire runs through them.
    ids: Vec<NodeId>,
    /// The stage-local column the lane's first card stands in.
    start: usize,
    /// One past the stage-local column of its last possible card — the
    /// boundary its tail runs out to.
    end: usize,
}
fn groups(cards: &[Card]) -> Vec<Vec<GuiLane>> {
    crate::runtime::branches(cards)
        .iter()
        .map(|stage| {
            stage
                .iter()
                .map(|branch| GuiLane {
                    ids: branch.cards.iter().map(|card| card.id).collect(),
                    start: branch.start,
                    end: branch.end,
                })
                .collect()
        })
        .collect()
}
/// How wide a stage is: one past the last column any of its lanes reaches.
fn stage_width(lanes: &[GuiLane]) -> usize {
    lanes.iter().map(|lane| lane.end).max().unwrap_or(1)
}
/// Where a stage's boundary ring stands. Boundary `0` and boundary `width`
/// are the stage's own split and merge columns, exactly as they always were;
/// an interior boundary sits in the air between the two columns it divides —
/// clear of both cards' stub boxes — so the ring reads as belonging to the
/// stage rather than to either card facing it.
fn ring_x(base_x: f32, boundary: usize, width: usize) -> f32 {
    if boundary == 0 {
        junction_x(base_x, false)
    } else if boundary >= width {
        junction_x(base_x + (width - 1) as f32 * COLUMN_STEP, true)
    } else {
        base_x + boundary as f32 * COLUMN_STEP - COLUMN_STEP / 2.0
    }
}
/// The boundaries where a shared stage wears a collar, with the collar's
/// shape: a bar (`true`) where lanes part or continue, a plus (`false`)
/// where lanes gather. The stage's own ends always wear one — the trunk
/// feeds boundary `0` and leaves from boundary `width`, so both need their
/// air gap whatever the lanes do in between. An interior boundary wears one
/// when lanes part there (two or more lanes start), gather there (two or
/// more end), or trade over (a lane ending or starting while another
/// crosses through). A boundary crossed by nothing and touched by only one
/// lane is no junction at all, and wears nothing.
fn stage_rings(lanes: &[GuiLane]) -> Vec<(usize, bool)> {
    if lanes.len() < 2 {
        return Vec::new();
    }
    let width = stage_width(lanes);
    (0..=width)
        .filter_map(|boundary| {
            let starts = lanes.iter().filter(|lane| lane.start == boundary).count();
            let ends = lanes.iter().filter(|lane| lane.end == boundary).count();
            let crossing = lanes
                .iter()
                .filter(|lane| lane.start < boundary && boundary < lane.end)
                .count();
            (boundary == 0
                || boundary == width
                || starts >= 2
                || ends >= 2
                || (starts + ends >= 1 && crossing >= 1))
                .then_some((boundary, ends < 2))
        })
        .collect()
}
/// The two columns one stage's risers stand in, worked out from the same
/// helpers that draw the wires so a conversion circle is placed by the same
/// arithmetic the wiring uses and never by a copy of it.
#[derive(Clone, Copy)]
struct StageColumn {
    /// The column its branches drop out of, left of the stage.
    split: f32,
    /// The column they gather back into, right of the stage — which is also
    /// where a join ring stands whenever the stage runs branches in parallel.
    merge: f32,
    /// Where the wire arriving at this stage begins: the previous stage's
    /// `merge`, or the face of the rack's own `IN` card for the first stage.
    feed: f32,
    /// The canvas column its boundary `0` stands in: the column its first
    /// cards share, which for a lane that starts mid-stage is no longer the
    /// leftmost slot the stage happens to own.
    base: f32,
    /// How many columns the stage runs.
    width: usize,
}
/// Where a stage's boundary ring stands, in the same columns the wiring
/// drawing uses.
fn boundary_ring_x(stage: &StageColumn, boundary: usize) -> f32 {
    ring_x(stage.base, boundary, stage.width)
}

fn stage_columns(stages: &[Vec<GuiLane>], slots: &[Slot], input_x: f32) -> Vec<StageColumn> {
    let mut columns: Vec<StageColumn> = Vec::with_capacity(stages.len());
    for stage in stages {
        // The stage's own first column, worked out from each lane's head
        // card by undoing the span that shifted it right; every lane agrees.
        let mut base: Option<f32> = None;
        for lane in stage {
            let Some(head) = lane.ids.first() else {
                continue;
            };
            let Some(slot) = slots.iter().find(|slot| slot.id == *head) else {
                continue;
            };
            base = Some(
                base.map_or(slot.x - lane.start as f32 * COLUMN_STEP, |past: f32| {
                    past.min(slot.x - lane.start as f32 * COLUMN_STEP)
                }),
            );
        }
        // Read before the push: a stage is fed from the column the stage
        // before it hands off in, and the first stage from the rack's `IN`.
        let feed = columns
            .last()
            .map_or(wire_box_x(input_x, true), |past| past.merge);
        // A stage with nothing to place keeps its index, because every later
        // stage is numbered against it, and borrows the rack's own column.
        let base = base.unwrap_or(input_x);
        let width = stage_width(stage);
        columns.push(StageColumn {
            split: junction_x(base, false),
            merge: ring_x(base, width, width),
            feed,
            base,
            width,
        });
    }
    columns
}

/// A stretch of drawn wire a conversion circle may be seated on: its two
/// ends in either order, the coordinate across the wire it sits on, whether
/// that wire runs up and down rather than across, and which way along it the
/// signal travels.
#[derive(Clone, Copy)]
struct Seat {
    from: f32,
    to: f32,
    cross: f32,
    vertical: bool,
    forward: f32,
}

/// One width conversion the engine actually wired, placed where the canvas
/// can draw it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WidthMark {
    /// The conversion node in the graph the rack just compiled. The id counts
    /// down from `u64::MAX` and belongs to that graph alone, so it is good
    /// for remembering a nudge and for nothing else.
    id: NodeId,
    /// Channels on the side the signal arrives on, and on the side it leaves:
    /// how many pins the circle carries at each end.
    inputs: usize,
    outputs: usize,
    /// The coordinate across the wire, which no drag ever changes.
    cross: f32,
    /// The ends of the stretch the circle may slide along, already shrunk so
    /// that neither the ring nor its click area reaches off the wire.
    lo: f32,
    hi: f32,
    /// Which way along the wire the signal runs, `1.0` or `-1.0`, so the pins
    /// point with the signal up a riser and down one alike.
    forward: f32,
    /// Whether the wire it rides runs up and down rather than across.
    vertical: bool,
}

impl WidthMark {
    /// Where the circle is drawn: the middle of its stretch of wire, shifted
    /// along by whatever the operator has dragged it, and never off it. The
    /// conversion itself does not move — only where its mark is parked.
    fn centre(&self, offsets: &std::collections::BTreeMap<NodeId, f32>) -> [f32; 2] {
        let nudge = offsets.get(&self.id).copied().unwrap_or_default();
        let shift = ((self.lo + self.hi) * 0.5 + nudge).clamp(self.lo, self.hi);
        if self.vertical {
            [self.cross, shift]
        } else {
            [shift, self.cross]
        }
    }

    /// The coordinate a drag reads: the one that runs along the wire,
    /// whichever way that wire happens to run. The circle follows the pointer
    /// in this coordinate only; the coordinate across the wire belongs to the
    /// wire itself.
    fn along(&self, at: [f32; 2]) -> f32 {
        if self.vertical { at[1] } else { at[0] }
    }
}

/// Where each conversion the engine wired can be drawn, in the layout
/// `frame` describes.
///
/// The conversions come from `Runtime::junctions`, which reads them back out
/// of the graph `chain` compiled, so a circle here is never drawn for a wire
/// the engine did not make. What is left to work out is only where it can
/// physically sit: a circle is a disc about `2 * MARK_RADIUS` across, and the
/// gap between two facing stub boxes is barely wider than that, so every
/// candidate seat is shrunk by `MARK_HIT` at both ends and the first one with
/// room left wins. That single rule is what leaves a stretch too short for a
/// circle of its own — see [`MARK_HIT`].
fn width_marks(found: &[Junction], stages: &[Vec<GuiLane>], frame: &Geometry) -> Vec<WidthMark> {
    let columns = stage_columns(stages, &frame.slots, frame.input.x);
    let slot_of = |id: NodeId| frame.slots.iter().find(|slot| slot.id == id);
    let stage_of = |id: NodeId| {
        stages
            .iter()
            .position(|stage| stage.iter().any(|lane| lane.ids.contains(&id)))
    };
    // Which lane of which stage a card belongs to, so a wire can be seated
    // at the boundary ring its own lane opens or closes at rather than at
    // the stage's outer pair.
    let place_of = |id: NodeId| -> Option<(usize, usize)> {
        stages.iter().enumerate().find_map(|(index, stage)| {
            stage
                .iter()
                .position(|lane| lane.ids.contains(&id))
                .map(|lane| (index, lane))
        })
    };
    // Risers are drawn clamped to the visible height of the rack, so a circle
    // riding one is clamped exactly the same way.
    let row = |y: f32| y.max(frame.bottom).min(frame.top);
    // Where a card's wire becomes visible on its left, and on its right: the
    // facing edge of its own stub box.
    let left_face = |x: f32| wire_box_x(x, false) - STUB_HALF;
    let right_face = |x: f32| wire_box_x(x, true) + STUB_HALF;
    // The `IN` card carries no stub box, and is treated as though it did, so
    // that a circle between it and the first card sits exactly where one
    // between two cards sits.
    let in_face = || right_face(frame.input.x);
    // The `OUT` card does carry one, and no circle may cover it: it is where
    // the master meter lives.
    let out_face = left_face(frame.output.x);
    // Where the wire arriving at a stage starts: after the join ring when the
    // stage before it ran branches in parallel — the ring is not a card and
    // has no slot, which is what the check for a multi-branch stage stands in
    // for — or else at the face of the stub box it was fed from. A lane of
    // that stage may have ended early, at one of the stage's interior rings;
    // the wire then starts at that ring's edge instead.
    let arrives = |index: usize, from: Option<NodeId>| -> f32 {
        let feed = columns
            .get(index)
            .map_or(wire_box_x(frame.input.x, true), |column| column.feed);
        if index > 0 && stages[index - 1].len() > 1 {
            let ring = from
                .and_then(place_of)
                .filter(|(past, _)| *past == index - 1)
                .map_or(feed, |(_, lane)| {
                    boundary_ring_x(&columns[index - 1], stages[index - 1][lane].end)
                });
            return ring + MIXER_RING;
        }
        match from {
            None => in_face(),
            Some(id) => slot_of(id).map(|src| right_face(src.x)).unwrap_or(feed),
        }
    };

    // Where the master wire begins: the merge column of the last stage, or
    // the face of the `IN` card's own stub box when the rack is empty. A
    // parallel last stage hands off from the edge of its join ring, which is
    // no card and so has no stub box to measure from.
    let master_from = || -> f32 {
        let mut start = columns
            .last()
            .map_or(wire_box_x(frame.input.x, true), |column| column.merge);
        if stages.last().is_some_and(|stage| stage.len() > 1) {
            start += MIXER_RING;
        }
        start
    };

    let seats = |junction: &Junction| -> Vec<Seat> {
        let mut seats = Vec::new();
        let Some(to) = junction.to else {
            // The conversion hands to the rack's `OUT` card, so it rides the
            // master wire: from wherever that wire starts as far as the face
            // of the card's own stub box.
            seats.push(Seat {
                from: master_from(),
                to: out_face,
                cross: frame.flow,
                vertical: false,
                forward: 1.0,
            });
            return seats;
        };
        let Some(index) = stage_of(to) else {
            // Nothing in the layout answers to it, so the wire lands on the
            // join ring: this is the conversion lifting one branch to the
            // width the others arrive at before the ring gathers them — the
            // ring its own lane closes at, wherever along the stage that is.
            let Some(from) = junction.from else {
                return seats;
            };
            let (Some(index), Some(slot), Some((_, lane))) =
                (stage_of(from), slot_of(from), place_of(from))
            else {
                return seats;
            };
            let ring = boundary_ring_x(&columns[index], stages[index][lane].end);
            let side = (row(slot.y) - frame.flow).signum();
            seats.push(Seat {
                from: row(slot.y),
                to: frame.flow + side * (MIXER_RING + MARK_AIR),
                cross: ring,
                vertical: true,
                forward: -side,
            });
            // The middle row of a three-deep stage lies exactly on the flow
            // line, so its riser has no length at all. The card's own output
            // run does reach the ring, so the circle rides that instead.
            seats.push(Seat {
                from: right_face(slot.x),
                to: ring - (MIXER_RING + MARK_AIR),
                cross: slot.y,
                vertical: false,
                forward: 1.0,
            });
            return seats;
        };
        let Some(slot) = slot_of(to) else {
            return seats;
        };
        let shared = stages[index].len() > 1;
        // The ring this card's lane opens at: the stage's split column when
        // the lane runs from the stage's very start, one of its interior
        // rings otherwise.
        let head_ring = place_of(to).map_or(columns[index].split, |(_, lane)| {
            boundary_ring_x(&columns[index], stages[index][lane].start)
        });
        // On the line the whole rack flows along, rather than one of a
        // parallel stage's other rows. This is a comparison and not simply
        // `!shared`, because the middle row of a three-deep stage sits on the
        // line exactly, and a stage on its own can be scrolled off it — while
        // the trunk it hangs from stays on the line either way.
        let on_line = (slot.y - frame.flow).abs() < 1.0;
        if shared && !on_line {
            // Fed by a riser out of the trunk: the conversion belongs on it,
            // in the column the stage drops out of — the column this lane
            // drops out of — which no other wire or box occupies at this
            // height.
            seats.push(Seat {
                from: frame.flow,
                to: row(slot.y),
                cross: head_ring,
                vertical: true,
                forward: (row(slot.y) - frame.flow).signum(),
            });
        }
        seats.push(Seat {
            from: if on_line {
                arrives(index, junction.from)
            } else {
                junction_x(slot.x, false)
            },
            to: if shared {
                // A lane opening at an interior ring has no trunk between
                // the ring and its first card for a circle to ride: the
                // stretch there is the card's own lead, twelve pixels wide.
                head_ring
                    .max(junction_x(slot.x, false))
                    .min(left_face(slot.x))
            } else {
                left_face(slot.x)
            },
            cross: slot.y,
            vertical: false,
            forward: 1.0,
        });
        seats
    };

    let mut marks = Vec::with_capacity(found.len());
    for junction in found {
        for seat in seats(junction) {
            // One rule for every seat: the circle needs its ring and its
            // click area inside the stretch it was offered, and a stretch
            // with nothing left after that is a stretch it cannot ride. A
            // circle is never floated off drawn wire.
            let (first, last) = if seat.from <= seat.to {
                (seat.from, seat.to)
            } else {
                (seat.to, seat.from)
            };
            let (lo, hi) = (first + MARK_HIT, last - MARK_HIT);
            if hi < lo {
                continue;
            }
            marks.push(WidthMark {
                id: junction.id,
                inputs: junction.inputs.count(),
                outputs: junction.outputs.count(),
                cross: seat.cross,
                lo,
                hi,
                forward: seat.forward,
                vertical: seat.vertical,
            });
            break;
        }
    }
    marks
}

/// The stretch of drawn wire `from..to`, cut where every conversion circle
/// stands on it, each piece as wide as the signal that piece actually
/// carries: a wire runs into a circle at the width the conversion takes in,
/// and leaves it at the width the conversion makes. Without the cut the
/// whole stretch runs at whatever width its owner assumed, and a mono wire
/// feeding a split reads as stereo already split — a cable that looks
/// broken where an operator should see channels dividing.
///
/// Only a circle drawn inside the stretch, at the stretch's own height,
/// cuts it. A vertical mark rides a riser, and how many risers a column
/// carries is the riser's own affair. The width the caller had (`lanes`)
/// stands for every stretch no circle reaches — which on an unmarked wire
/// is all of it.
fn width_runs(
    from: f32,
    to: f32,
    lanes: usize,
    y: f32,
    marks: &[WidthMark],
    offsets: &std::collections::BTreeMap<NodeId, f32>,
) -> Vec<(f32, f32, usize)> {
    let mut cuts = marks
        .iter()
        .filter(|mark| !mark.vertical)
        .map(|mark| (mark.centre(offsets), mark.inputs, mark.outputs))
        .filter(|([x, cross], _, _)| (*cross - y).abs() < 0.5 && *x > from && *x < to)
        .collect::<Vec<_>>();
    cuts.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
    if cuts.is_empty() {
        return vec![(from, to, lanes)];
    }
    let mut runs = Vec::with_capacity(cuts.len() + 1);
    let mut start = from;
    // What the first circle takes in is what the wire before it carries.
    let mut width = cuts[0].1;
    for ([x, _], _, outputs) in cuts {
        if x > start {
            runs.push((start, x, width));
        }
        start = x;
        width = outputs;
    }
    if to > start {
        runs.push((start, to, width));
    }
    runs
}

/// A meter the user can click, in the order the signal passes them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum MeterPos {
    /// The rack's own input meter: upstream of every card.
    MasterIn,
    /// One card's input (`!side`) or output (`side`) wire meter, or —
    /// keyed `(u64::MAX, false)` — the master output meter standing on
    /// the OUT jack, or — keyed `(0, true)` — the capture trim meter
    /// standing on the IN jack.
    Wire(u64, bool),
    /// The rack's own output meter: downstream of every card. The canvas
    /// face of the same reading is the master output meter, and the two
    /// acknowledge each other.
    MasterOut,
}

/// Every meter a click at `from` acknowledges: that meter, and the ones the
/// clipped signal reached after it.
///
/// "After" is the topology `runtime::chain` actually wires — stages in
/// series, each stage a set of parallel branches, each branch a serial
/// chain — read straight off `groups`, which is the same grouping the layout
/// and the graph are built from:
///
/// * a card's *output* meter owes nothing to its input meter, but a clip on
///   the way **in** came out of the same card, so the input meter owes its
///   output one;
/// * the cards further along **the same branch** are next in that chain, so
///   both of their meters go too;
/// * every card in a *later* stage sees that signal, so both of their meters
///   go as well, and so does the rack's output meter;
/// * a branch **beside** this one in the same stage keeps its report: the two
///   are parallel and that clip never travelled through it;
/// * the rack's input meter is upstream of everything, so it acknowledges the
///   whole rack;
/// * the output is last, and it has two faces — the OUT jack's meter on the
///   canvas and the panel meter above — reading the one summed signal, so
///   acknowledging either clears both, and neither owes anything further.
fn acknowledged(from: MeterPos, stages: &[Vec<GuiLane>]) -> Vec<MeterPos> {
    let mut out = vec![from];
    let mut first_downstream = 0;
    let mut rest_of_branch: &[NodeId] = &[];
    match from {
        MeterPos::MasterIn => {}
        MeterPos::MasterOut => {
            out.push(MeterPos::Wire(MASTER_WIRE_KEY, false));
            return out;
        }
        MeterPos::Wire(id, side) => {
            if !side {
                out.push(MeterPos::Wire(id, true));
            }
            let located = stages.iter().enumerate().find_map(|(stage, lanes)| {
                lanes.iter().find_map(|lane| {
                    lane.ids
                        .iter()
                        .position(|&card| card == NodeId(id))
                        .map(|row| (stage, &lane.ids[row + 1..]))
                })
            });
            match located {
                Some((stage, rest)) => {
                    first_downstream = stage + 1;
                    rest_of_branch = rest;
                }
                None => first_downstream = stages.len(),
            }
        }
    }
    for id in rest_of_branch {
        out.extend([MeterPos::Wire(id.0, false), MeterPos::Wire(id.0, true)]);
    }
    for stage in &stages[first_downstream.min(stages.len())..] {
        for lane in stage {
            for id in &lane.ids {
                out.extend([MeterPos::Wire(id.0, false), MeterPos::Wire(id.0, true)]);
            }
        }
    }
    // Everything upstream reaches the output, and the output is the one
    // meter with two faces: clear the jack's meter and the panel's
    // together, so the same clip is never acknowledged twice.
    out.extend([MeterPos::Wire(MASTER_WIRE_KEY, false), MeterPos::MasterOut]);
    out
}
fn structured_canvas(
    cards: &[Card],
    w: f32,
    h: f32,
    scroll: f32,
    vertical: f32,
    route: crate::runtime::Route,
) -> Option<flow_canvas::Canvas> {
    if !cards.iter().any(|card| card.routing.is_some()) {
        return None;
    }
    let root = crate::runtime::flow(cards).ok()?;
    let shapes = cards
        .iter()
        .map(|card| (card.id.0, (card.inputs.count(), card.outputs.count())))
        .collect();
    Some(flow_canvas::Canvas::new(
        &root,
        [w, h],
        [scroll, vertical],
        &shapes,
        [route.input.count(), route.output.count()],
    ))
}
fn geometry(groups: &[Vec<GuiLane>], w: f32, h: f32, scroll: f32, vertical: f32) -> Geometry {
    let top = h / 2.0 - 90.0;
    let bottom = -h / 2.0 + 150.0;
    let available = (top - bottom).max(112.0);
    let flow = (top + bottom) / 2.0;
    // Rows are branches: a stage's branches sit one above the other, while a
    // chain runs left to right through its own stage columns.
    let rows = groups.iter().map(Vec::len).max().unwrap_or(1);
    let row_step = if rows < 2 {
        140.0
    } else {
        ((available - 112.0) / (rows - 1) as f32).clamp(116.0, 140.0)
    };
    let max_vertical = ((rows.saturating_sub(1)) as f32 * row_step + 112.0 - available).max(0.0);
    let left = -w / 2.0 + 90.0 - scroll;
    // The `IN` card owns the first column, so a chain starts one step right
    // of it; every card, and the `OUT` card after them all, stand one column
    // further along than they did when the rack's jack was a bare label.
    let in_x = left + 90.0 + CARD_WIDTH / 2.0;
    let first_column = in_x + COLUMN_STEP;
    let mut slots = Vec::new();
    let mut column = 0.0;
    for stage in groups {
        let height = stage.len().saturating_sub(1) as f32 * row_step + 112.0;
        let first = if height > available {
            top - 56.0 + vertical.clamp(0.0, max_vertical)
        } else {
            flow + (stage.len() - 1) as f32 * row_step / 2.0
        };
        let mut width = 1;
        for (row, lane) in stage.iter().enumerate() {
            width = width.max(lane.end);
            for (index, &id) in lane.ids.iter().enumerate() {
                slots.push(Slot {
                    id,
                    x: first_column + (column + lane.start as f32 + index as f32) * COLUMN_STEP,
                    y: first - row as f32 * row_step,
                });
            }
        }
        column += width as f32;
    }
    // The `OUT` card owns the next column whole: one step right of
    // whichever column reaches furthest, and on the flow line, since
    // nothing ever moves it. An empty rack stands it in the first column
    // after `IN`.
    let out_x = slots
        .iter()
        .map(|slot| slot.x)
        .reduce(f32::max)
        .map_or(first_column, |x| x + COLUMN_STEP);
    Geometry {
        input: Slot {
            id: INPUT_JACK,
            x: in_x,
            y: flow,
        },
        output: Slot {
            id: NodeId(MASTER_WIRE_KEY),
            x: out_x,
            y: flow,
        },
        slots,
        top,
        bottom,
        flow,
        max_vertical,
    }
}
fn visible(slot: &Slot, top: f32, bottom: f32, w: f32) -> bool {
    slot.y + 56.0 <= top + 0.01
        && slot.y - 56.0 >= bottom - 0.01
        && slot.x + CARD_WIDTH / 2.0 >= -w / 2.0
        && slot.x - CARD_WIDTH / 2.0 <= w / 2.0
}
/// One of the two rings a fork draws on the trunk — where it fans out and
/// where it merges — seen as a drop target: its drawn point, plus one card
/// of its stage to anchor the placement. Any card will do — the ring belongs
/// to the fork and not to a lane — so long as it is not the dragged card,
/// which `place` removes before it looks for the anchor again.
#[derive(Clone, Copy)]
struct ForkMark {
    /// Centre of the adjacent wire insertion target, distinct from the ring.
    wire_target: Option<[f32; 2]>,
    scope: Option<usize>,
    anchor: NodeId,
    x: f32,
    y: f32,
    side: Side,
}

fn fork_drop_point(mark: &ForkMark) -> [f32; 2] {
    mark.wire_target.unwrap_or([
        mark.x
            + if mark.side == Side::Split {
                -(MIXER_RING + 18.0)
            } else {
                MIXER_RING + 18.0
            },
        mark.y,
    ])
}
/// The rings the canvas draws today, worked out from the same arithmetic:
/// two per parallel stage, standing where its lanes fan apart and where
/// they come back — every fork in the rack, however local, guards both of
/// its ends.
fn fork_marks(
    groups: &[Vec<GuiLane>],
    slots: &[Slot],
    flow: f32,
    drag: Option<NodeId>,
) -> Vec<ForkMark> {
    groups
        .iter()
        .filter(|stage| stage.len() > 1)
        .filter_map(|stage| {
            let anchor = stage
                .iter()
                .flat_map(|lane| lane.ids.iter())
                .copied()
                .find(|id| Some(*id) != drag)?;
            let slot_of = |id: NodeId| slots.iter().find(|slot| slot.id == id);
            // The stage's first column, the same arithmetic `stage_columns`
            // uses: each lane's head minus the columns it opens past.
            let base = stage
                .iter()
                .filter_map(|lane| {
                    slot_of(*lane.ids.first()?).map(|slot| slot.x - lane.start as f32 * COLUMN_STEP)
                })
                .min_by(|a, b| a.total_cmp(b))?;
            let width = stage_width(stage);
            // Only the outer rings are doors: a split or merge drop speaks
            // for the stage as a whole, and an interior ring belongs to one
            // lane's opening, not to a fork the drop would build.
            let marks = stage_rings(stage)
                .into_iter()
                .filter(|(boundary, _)| *boundary == 0 || *boundary == width)
                .map(|(boundary, _)| ForkMark {
                    wire_target: None,
                    scope: None,
                    anchor,
                    x: ring_x(base, boundary, width),
                    y: flow,
                    side: if boundary == 0 {
                        Side::Split
                    } else {
                        Side::Join
                    },
                })
                .collect::<Vec<_>>();
            (!marks.is_empty()).then_some(marks)
        })
        .collect::<Vec<_>>()
        .into_iter()
        .flatten()
        .collect()
}
/// Prefer the source meter for a direct meter-to-meter connection.
fn wire_gaps(
    groups: &[Vec<GuiLane>],
    frame: &Geometry,
    open: impl Fn(u64, bool) -> bool,
) -> Vec<WireGap> {
    let mut gaps = Vec::new();
    let slot = |id: NodeId| frame.slots.iter().find(|slot| slot.id == id);
    if open(INPUT_JACK.0, true) {
        gaps.push(meter_gap(wire_box_x(frame.input.x, true), frame.flow, true));
    }
    let mut previous = Some(&frame.input);
    for stage in groups {
        for lane in stage {
            let mut source = if stage.len() == 1 { previous } else { None };
            for id in &lane.ids {
                let Some(card) = slot(*id) else {
                    continue;
                };
                if open(id.0, false) {
                    gaps.push(
                        if let Some(source) = source.filter(|s| (s.y - card.y).abs() < 0.5) {
                            meter_gap(wire_box_x(source.x, true), card.y, true)
                        } else {
                            meter_gap(wire_box_x(card.x, false), card.y, false)
                        },
                    );
                }
                if open(id.0, true) {
                    gaps.push(meter_gap(wire_box_x(card.x, true), card.y, true));
                }
                source = Some(card);
            }
        }
        previous = if stage.len() == 1 {
            stage[0].ids.last().and_then(|id| slot(*id))
        } else {
            None
        };
    }
    if open(MASTER_WIRE_KEY, false) {
        gaps.push(if let Some(source) = previous {
            meter_gap(wire_box_x(source.x, true), frame.flow, true)
        } else {
            meter_gap(wire_box_x(frame.output.x, false), frame.flow, false)
        });
    }
    gaps
}

/// One stretch of wire leaving a card toward the next card of its lane,
/// seen as a drop zone: the shoulder of a lane's own run. A card held
/// above or below that shoulder joins the lane's parallel, bracketed by
/// the same split and merge that guard the run it was aimed at.
#[derive(Clone, Copy)]
struct WireRun {
    card: NodeId,
    to: NodeId,
    x0: f32,
    x1: f32,
    y: f32,
}

/// Every adjacent pair of a lane's cards offers one run — the wire that
/// leaves the first card's edge and reaches the next. Pairs that include
/// the dragged card are skipped: its own stub is not a landing.
fn wire_runs(groups: &[Vec<GuiLane>], slots: &[Slot], drag: Option<NodeId>) -> Vec<WireRun> {
    let mut runs = Vec::new();
    for lane in groups.iter().flat_map(|stage| stage.iter()) {
        for pair in lane.ids.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if Some(a) == drag || Some(b) == drag {
                continue;
            }
            let (Some(a_slot), Some(b_slot)) = (
                slots.iter().find(|slot| slot.id == a),
                slots.iter().find(|slot| slot.id == b),
            ) else {
                continue;
            };
            runs.push(WireRun {
                card: a,
                to: b,
                x0: a_slot.x + CARD_WIDTH / 2.0,
                x1: b_slot.x - CARD_WIDTH / 2.0,
                y: a_slot.y,
            });
        }
    }
    runs
}
fn serial_drop_point(target: &Target, slot: &Slot, runs: &[WireRun]) -> [f32; 2] {
    if let Some(run) = runs.iter().find(|run| match target.side {
        Side::Right => run.card == target.card,
        Side::Left => run.to == target.card,
        _ => false,
    }) {
        [(run.x0 + run.x1) / 2.0, run.y]
    } else {
        [
            slot.x
                + if target.side == Side::Left {
                    -106.0
                } else {
                    106.0
                },
            slot.y,
        ]
    }
}
fn parallel_drop_point(
    target: &Target,
    slot: &Slot,
    slots: &[Slot],
    targets: Option<&crate::layout::DropTargets>,
) -> [f32; 2] {
    let point = |target: &Target| {
        slots
            .iter()
            .find(|slot| slot.id == target.card)
            .map(|slot| {
                [
                    slot.x,
                    slot.y
                        + if target.side == Side::Above {
                            50.0
                        } else {
                            -50.0
                        },
                ]
            })
    };
    let peers = targets
        .map(|targets| targets.equivalents(*target))
        .unwrap_or(&[]);
    let points = peers
        .iter()
        .filter(|target| matches!(target.side, Side::Above | Side::Below))
        .filter_map(point)
        .collect::<Vec<_>>();
    if points.is_empty() {
        [
            slot.x,
            slot.y
                + if target.side == Side::Above {
                    50.0
                } else {
                    -50.0
                },
        ]
    } else {
        [
            points.iter().map(|p| p[0]).sum::<f32>() / points.len() as f32,
            points.iter().map(|p| p[1]).sum::<f32>() / points.len() as f32,
        ]
    }
}
/// Everything the wire space offers as a landing besides the card faces:
/// the rings of every fork, and the shoulder of every lane's run.
#[derive(Default)]
struct DropZones<'a> {
    joins: &'a [ForkMark],
    runs: &'a [WireRun],
}
fn target_at(
    mouse: Option<[f32; 2]>,
    slots: &[Slot],
    moving: Option<NodeId>,
    zones: &DropZones,
    top: f32,
    bottom: f32,
    w: f32,
) -> Option<Target> {
    let [x, y] = mouse?;
    if y > top || y < bottom {
        return None;
    }
    // Releasing on the original card is a cancellation, never a request
    // to edit whichever neighbour happens to be nearest after excluding it.
    if slots
        .iter()
        .any(|slot| Some(slot.id) == moving && hit(mouse, slot.x, slot.y, CARD_WIDTH / 2.0, 56.0))
    {
        return None;
    }

    // A direct serial cable is one insertion slot, regardless of which
    // endpoint is nearer. Canonicalize it as After(source).
    if let Some(run) = zones
        .runs
        .iter()
        .find(|run| run.x0 <= x && x <= run.x1 && (y - run.y).abs() <= 14.0)
    {
        return Some(Target {
            scope: None,
            card: if run.card == INPUT_JACK {
                run.to
            } else {
                run.card
            },
            side: if run.card == INPUT_JACK {
                Side::Left
            } else {
                Side::Right
            },
        });
    }

    // A point on a card's face is the loudest intent there is. Held low on
    // the face the lane claims the drop — chain into it, head or not; held
    // high, up or down and clear of the middle, it asks for a fork of its
    // own around that one column. The rings sit out in the wire space
    // between stages, so the face is consulted first and a graze of a
    // lane's corner stays inside its lane.
    if let Some(slot) = slots
        .iter()
        .filter(|slot| Some(slot.id) != moving && visible(slot, top, bottom, w))
        .filter(|slot| (x - slot.x).abs() <= CARD_WIDTH / 2.0 && (y - slot.y).abs() <= 35.0)
        .min_by(|a, b| {
            ((x - a.x).powi(2) + (y - a.y).powi(2))
                .total_cmp(&((x - b.x).powi(2) + (y - b.y).powi(2)))
        })
    {
        let dx = x - slot.x;
        let dy = y - slot.y;
        let side = if dy.abs() >= 14.0 && dy.abs() > dx.abs() {
            if dy > 0.0 { Side::Above } else { Side::Below }
        } else if dx < 0.0 {
            Side::Left
        } else {
            Side::Right
        };
        return Some(Target {
            scope: None,
            card: slot.id,
            side,
        });
    }
    // Directly under a card's column, in the gap between the faces, the
    // pull reads like the high face did: a fork of its own around that
    // one card's column, opening above or below the card it hangs under.
    if let Some(slot) = slots
        .iter()
        .filter(|slot| Some(slot.id) != moving && visible(slot, top, bottom, w))
        .filter(|slot| {
            let dx = (x - slot.x).abs();
            let dy = (y - slot.y).abs();
            dx <= CARD_WIDTH / 2.0 && dy > 35.0 && dy <= 76.0
        })
        .min_by(|a, b| {
            (y - a.y)
                .abs()
                .total_cmp(&(y - b.y).abs())
                .then((x - a.x).abs().total_cmp(&(x - b.x).abs()))
        })
    {
        let dy = y - slot.y;
        return Some(Target {
            scope: None,
            card: slot.id,
            side: if dy > 0.0 { Side::Above } else { Side::Below },
        });
    }
    // Whole-fork insertions belong to the wire before the split or after
    // the join. Ring centres are explicitly excluded from drag editing.
    if zones
        .joins
        .iter()
        .any(|mark| (x - mark.x).powi(2) + (y - mark.y).powi(2) <= (MIXER_RING + 1.0).powi(2))
    {
        return None;
    }
    if let Some(mark) = zones
        .joins
        .iter()
        .filter(|mark| {
            let [tx, ty] = fork_drop_point(mark);
            tx + 16.0 >= -w / 2.0
                && tx - 16.0 <= w / 2.0
                && (x - tx).abs() <= 16.0
                && (y - ty).abs() <= 18.0
        })
        .min_by(|a, b| {
            let a = fork_drop_point(a);
            let b = fork_drop_point(b);
            ((x - a[0]).powi(2) + (y - a[1]).powi(2))
                .total_cmp(&((x - b[0]).powi(2) + (y - b[1]).powi(2)))
        })
    {
        return Some(Target {
            scope: mark.scope,
            card: mark.anchor,
            side: mark.side,
        });
    }
    // Between two cards of one lane the wire runs with room above and
    // below it: a card held by that shoulder reaches for the whole lane's
    // parallel — the split opens before the run and the join closes after
    // it — while a card held level with the wire chains straight through.
    if let Some(run) = zones.runs.iter().find(|run| {
        run.x0 <= x && x <= run.x1 && (y - run.y).abs() > 14.0 && (y - run.y).abs() <= 50.0
    }) {
        return Some(Target {
            scope: None,
            card: if run.card == INPUT_JACK {
                run.to
            } else {
                run.card
            },
            side: if y > run.y { Side::Above } else { Side::Below },
        });
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
    if dx.abs() > CARD_WIDTH / 2.0 + 96.0 || dy.abs() > 140.0 {
        return None;
    }
    // In the wire space a fork's two rings sit on the trunk, so whichever
    // way the pull dominates decides: a vertical pull opens a parallel
    // lane beside the nearest card, a horizontal one chains into its lane.
    let side = if dy.abs() > dx.abs() && dy.abs() > 12.0 {
        if dy > 0.0 { Side::Above } else { Side::Below }
    } else if dx < 0.0 {
        Side::Left
    } else {
        Side::Right
    };
    Some(Target {
        scope: None,
        card: slot.id,
        side,
    })
}

/// A drop on either of a fork's rings belongs to the whole stage, not to
/// one lane, so its preview spans the fork too: the surviving head card of
/// every lane — while the ring is being drawn, that is every lane — so the
/// tick can stretch across all of them instead of marking one wire.
#[cfg(test)]
fn fork_lanes(
    stages: &[Vec<GuiLane>],
    drag: Option<NodeId>,
    target: &Target,
) -> Option<Vec<NodeId>> {
    if target.side != Side::Split && target.side != Side::Join {
        return None;
    }
    let stage = stages
        .iter()
        .find(|stage| stage.iter().any(|lane| lane.ids.contains(&target.card)))?;
    let heads: Vec<NodeId> = stage
        .iter()
        .filter_map(|lane| lane.ids.iter().copied().find(|id| Some(*id) != drag))
        .collect();
    (heads.len() > 1).then_some(heads)
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
        card.branch = saved.branch;
        card.span = saved.span;
        cards.push(card);
        if let Some(bytes) = &saved.state {
            states.insert(NodeId(saved.id), bytes.clone());
        }
        next_id = next_id.max(saved.id + 2);
    }
    if let Some(root) = settings
        .preferences
        .plugins
        .iter()
        .find_map(|saved| saved.routing.clone())
    {
        root.validate()?;
        let expected = cards
            .iter()
            .map(|c| c.id.0)
            .collect::<std::collections::BTreeSet<_>>();
        if root
            .ids()
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            != expected
        {
            return Err("Saved routing does not match the plugin roster".into());
        }
        let root = Arc::new(root);
        for card in &mut cards {
            card.routing = Some(root.clone());
        }
    }
    // Import an unambiguous legacy layout before the first frame, so the
    // diagram and drag targets do not switch representations after one edit.
    if cards.iter().all(|card| card.routing.is_none())
        && let Ok(root) = crate::runtime::flow(&cards)
    {
        let root = Arc::new(root);
        for card in &mut cards {
            card.routing = Some(root.clone());
        }
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
        drop_cache: None,
        runtime,
        loader: Box::new(loader),
        font,
        icons,
        settings: None,
        settings_page: false,
        settings_scope: PageScope::All,
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
        gain_click: GainClick::default(),
        gain_input: frost::TextInput::default(),
        size: None,
        windows: Default::default(),
        overlay: None,
        parameter_scroll: 0.0,
        press_origin: None,
        moved: false,
        mark_drag: None,
        mark_offsets: Default::default(),
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
    /// Compute semantic equivalence only when the graph or dragged ID changes;
    /// pointer movement and scrolling reuse the cached target identities.
    fn canonical_drop(&mut self, target: Option<Target>, moving: Option<NodeId>) -> Option<Target> {
        let target = target?;
        let root = crate::runtime::flow(&self.runtime.cards).ok()?;
        let moving = moving.map(|id| id.0);
        if self
            .drop_cache
            .as_ref()
            .is_none_or(|cache| cache.root != root || cache.moving != moving)
        {
            let targets = crate::layout::DropTargets::new(&root, moving);
            self.drop_cache = Some(DropCache {
                root,
                moving,
                targets,
            });
        }
        self.drop_cache.as_ref()?.targets.canonical(target)
    }
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
        if let Some(key) = wire_slot_key(id, side) {
            self.runtime.wires.set_gain(key, 10f32.powf(gain_db / 20.0));
        }
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
        if let Some(key) = wire_slot_key(id, side) {
            self.runtime.wires.set_open(key, open);
        }
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
        self.gain_click = GainClick::default();
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
            meter.restart(now());
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
        self.gain_click = GainClick::default();
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
        // The master's run into `OUT` names itself: no card stands behind
        // it, and the words say what falls silent.
        if id == MASTER_WIRE_KEY {
            self.runtime.message = if open {
                "Output wire is open — the rack falls silent".into()
            } else {
                "Output wire is closed again — sound reaches the jack".into()
            };
            return;
        }
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
                .is_some_and(|wire| wire.clipping()),
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
                        wire.clear_clip();
                    }
                }
            }
        }
    }

    /// How many channels a wire carries, as the GUI draws them: the
    /// card's own count on the stub's side — inputs for the lead into a
    /// card, outputs for the lead out of it — clamped to the two lanes
    /// the rack draws and the engine's metering folds into. The rack's own
    /// output jack is the route's far end, and anything the rack has no
    /// card for — a meter outliving its card — reads as whichever end that
    /// stub hangs off, which only ever keeps a gone meter's last lanes
    /// decaying in the right shape.
    fn wire_lanes(&self, id: u64, side: bool) -> usize {
        let route = self.runtime.route();
        if id == MASTER_WIRE_KEY {
            return route.output.count().clamp(1, 2);
        }
        // The `IN` jack carries the capture itself: however many channels
        // the device delivers, and no card arithmetic in between.
        if id == INPUT_JACK.0 {
            return route.input.count().clamp(1, 2);
        }
        self.runtime
            .cards
            .iter()
            .find(|card| card.id.0 == id)
            .map_or_else(
                || {
                    if side { route.output } else { route.input }
                        .count()
                        .clamp(1, 2)
                },
                |card| {
                    if side { card.outputs } else { card.inputs }
                        .count()
                        .clamp(1, 2)
                },
            )
    }
    /// The corner readout and the vertical meter of one wire slot, drawn at
    /// pre-computed geometry: `track` is the meter column's centre and
    /// `corner` the dB readout's centre. A card's pair hangs off its face;
    /// the master output's pair hangs off the rack's `OUT` jack, where the
    /// same gestures trim the summed signal and `paused` never holds.
    #[allow(clippy::too_many_arguments)]
    fn paint_wire_meter(
        &mut self,
        ctx: &mut Context,
        dt: f32,
        mouse: Option<[f32; 2]>,
        down: bool,
        clicked: bool,
        gain_reset: Option<(u64, bool)>,
        id: u64,
        side: bool,
        track: [f32; 2],
        corner: [f32; 2],
        paused: bool,
    ) {
        let [gx, gy] = corner;
        // The readout stands on every card the rack draws and is
        // drawn by the rack alone: entry or not, expanded or
        // collapsed, and whatever plugin the card happens to hold.
        // So every fact the rest of this body needs is copied out
        // once, here — the editor below writes through `&mut self`,
        // and a card whose stub was never clicked has no meter to
        // borrow at all, only the gain the engine is already
        // applying, which is the same number.
        let wire = self.wire_meters.get(&(id, side));
        let key = wire_slot_key(id, side);
        let (gain_db, clip) =
            corner_reading(wire, key.and_then(|key| self.runtime.wires.gain_of(key)));
        let open = wire.is_some_and(|meter| meter.open);
        // Only an open meter draws a track, and an open meter always
        // has an entry, so the defaults below are never read; −120 dB is
        // what `db` calls silence anyway. The track draws its lanes
        // separately — a stereo wire shows one bar and one peak line per
        // channel, the way its wiring draws two lines — so what is
        // carried down is the pair of lane readings, not one folded
        // number.
        let lanes = self.wire_lanes(id, side);
        let lane_readings: [(f32, f32, bool); 2] =
            wire.map_or([(-120.0, -120.0, false); 2], |meter| {
                std::array::from_fn(|lane: usize| {
                    (
                        meter.peaks[lane].level_db(),
                        meter.peaks[lane].peak_db(),
                        meter.peaks[lane].clipping(),
                    )
                })
            });
        let dragging = self
            .gain_drag
            .is_some_and(|(drag_id, grabbed, _)| drag_id == id && grabbed == side);
        if !paused {
            // The corner readout is the numeric handle: a click
            // adopts the trim's value into the field and the number
            // becomes editable in place. `clicked` is already false
            // while an editor stands, so one click opens exactly one
            // field, and the same press that opened it lands on the
            // field's own mouse handling below.
            if clicked
                && gain_reset != Some((id, side))
                && self.choosing.is_none()
                && self.gain_drag.is_none()
                && hit(mouse, gx, gy, GAIN_HALF_W, GAIN_HALF_H)
            {
                self.gain_edit = Some((id, side));
                self.gain_input.adopt(format!("{:+.1}", gain_db));
                self.gain_input.set_focus(true);
            }
            let editing = self.gain_edit == Some((id, side));
            // What the corner prints, and with it whether the corner
            // stands at all: the trim to one decimal, blank at unity
            // — except while this wire's drag holds the pointer, when
            // the corner is that drag's live readout and prints
            // through zero until the release answers a quiet wire.
            let label = corner_label(gain_db, dragging);
            // The box is the number's frame, so it goes with the
            // number, and it stays for the editor whose field stands
            // inside it. Only the drawn corner ever goes away: the
            // rect is still the click and double-click target at
            // unity, so the wire's gain stays reachable at exactly
            // the pixels it always stood on.
            if editing || label.is_some() {
                ctx.rectangle(
                    gx,
                    gy,
                    GAIN_HALF_W,
                    GAIN_HALF_H,
                    rgb(24, 20, 38),
                    if editing { 0.35 } else { 0.3 },
                );
            }
            // The gain in dB to one decimal, centred in that box —
            // the clickable numeric handle, no unit suffix. Above
            // full scale it turns red with the rest of the meter:
            // the readout is the part of the meter you are most
            // likely to be looking at, and a clip that only shows up
            // in the track is a clip you will miss.
            if !editing && let Some(label) = label {
                ctx.text(
                    gx,
                    gy,
                    &self.font,
                    label,
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
                            self.set_trim(id, side, db);
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
            // that draws, and the trim readout above it stands or
            // blanks on its own rule.
            // The peak-hold keeps ticking in the loop over
            // `wire_meters`, so re-opening finds the meter where it
            // was left.
            return;
        }
        if clicked
            && !paused
            && self.choosing.is_none()
            && self.gain_drag.is_none()
            && self.gain_edit.is_none()
            && let Some([_, my]) = mouse
            && hit(mouse, track[0], track[1], 4.5, TRACK_HALF)
            && !hit(mouse, track[0], track[1], STUB_HALF, STUB_TALL)
        {
            // Grabbing the track turns the meter into the gain
            // slider: the grab records the pointer height so the
            // drag is relative from here. The stub box sits at
            // the track centre and keeps its toggle, so it is
            // excluded from the grab.
            self.gain_drag = Some((id, side, my));
            // The track is the other half of the meter body, so it
            // acknowledges a clip just as the stub does — pulling the
            // gain down and clearing the report are the same gesture
            // on the same thing. The dB readout in the card corner is
            // deliberately not: it is where the operator goes to look
            // at the trim, and the clip should still be there when
            // they arrive.
            self.acknowledge_clip(MeterPos::Wire(id, side));
        }
        let hovered = dragging || hit(mouse, track[0], track[1], 4.5, TRACK_HALF);
        // The vertical meter carries the card's full height and
        // scales over the last 48 dB, screen y pointing up. The
        // track brightens while grabbed or hovered, saying "this
        // is the slider" before the first drag even starts.
        //
        // A wire wider than mono draws its lanes as parallel columns
        // close together, centred on the same line the wiring puts its
        // parallel strokes on: the background and the scale marks stay
        // one object — the clip and the hover colour the whole meter,
        // and the operator acknowledges the wire, not a channel of it —
        // while the level bars and the peak lines belong to a lane.
        // Mono keeps the exact column it always drew; stereo just
        // splits it.
        let lane_half = if lanes > 1 { 1.2 } else { 3.0 };
        let bg_half = if lanes > 1 { 2.75 } else { 3.0 };
        ctx.rectangle(
            track[0],
            track[1],
            bg_half,
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
        let lane_x = |lane: usize| track[0] + lane_offset(lane, lanes);
        for (lane, &(lane_level, _, lane_clip)) in lane_readings.iter().enumerate().take(lanes) {
            let t = ((lane_level.max(-48.0) + 48.0) / 48.0).clamp(0.0, 1.0);
            if t > 0.0 {
                ctx.rectangle(
                    lane_x(lane),
                    track[1] - 56.0 + 56.0 * t,
                    lane_half,
                    56.0 * t,
                    if lane_clip {
                        nice_common::knob::RED
                    } else {
                        LEVEL
                    },
                    0.3,
                );
            }
        }
        // Gray marks every 6 dB: the meter's own scale runs −48 to
        // 0 over the 112 px track, so a mark lands every 14 px.
        for step in 1..8 {
            ctx.rectangle(
                track[0],
                track[1] - 56.0 + 14.0 * step as f32,
                bg_half,
                0.5,
                if clip {
                    rgb(188, 62, 56)
                } else {
                    rgb(107, 114, 128)
                },
                0.3,
            );
        }
        for (lane, &(_, lane_peak, lane_clip)) in lane_readings.iter().enumerate().take(lanes) {
            let p = ((lane_peak.max(-48.0) + 48.0) / 48.0).clamp(0.0, 1.0);
            // Dimmed to slate so the white gain line stays the one
            // bright mark on the track. A stereo meter puts one peak
            // line on each lane; the trim rides the wire, so its white
            // line below spans both.
            ctx.rectangle(
                lane_x(lane),
                track[1] - 56.0 + 112.0 * p,
                if lanes > 1 { 1.45 } else { 4.5 },
                1.0,
                if lane_clip {
                    nice_common::knob::RED
                } else {
                    rgb(148, 163, 184)
                },
                0.3,
            );
        }
        if !paused {
            ctx.rectangle(
                track[0],
                gain_line_y(gain_db, track[1]),
                4.5,
                0.5,
                rgb(255, 255, 255),
                0.3,
            );
        }
    }
}
impl Process for RackUi {
    fn render_when_occluded(&self) -> bool {
        false
    }
    fn native_about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.windows.service(event_loop, &mut self.runtime);
        self.windows.capture(&mut self.runtime);
        self.runtime.tick();
        if self.runtime.closing && self.runtime.stopped() {
            event_loop.exit();
        }
    }
    fn native_window_event(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        id: winit::window::WindowId,
        event: &winit::event::WindowEvent,
    ) -> bool {
        if self.windows.event(id, event, &mut self.runtime) {
            return true;
        }
        if matches!(event, winit::event::WindowEvent::CloseRequested) {
            // Shutdown must also complete while hidden, without a draw frame.
            self.runtime.close();
            return true;
        }
        false
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
        // Every wire the rack carries gets its meter here, before anything is
        // drawn, so a clip shows up on a stub nobody has ever clicked. A wire
        // whose card came out of the settings file is born at the trim the rack
        // saved for it, which is the only gain a rack can know before its
        // audio has run a block.
        watch_wires(
            &mut self.wire_meters,
            // The cards' wires, and the two jacks' own: the input keyed
            // `0` and the output keyed `u64::MAX` get their meters here
            // too, born knowing the saved trim like any card wire knows
            // its own.
            self.runtime
                .cards
                .iter()
                .map(|card| card.id.0)
                .chain(std::iter::once(INPUT_JACK.0))
                .chain(std::iter::once(MASTER_WIRE_KEY)),
            |id, side| self.runtime.trim(id, side),
            |key| self.runtime.wires.gain_of(key),
            |id, side| self.runtime.open_circuit(id, side),
            now(),
        );
        self.timing_meters.retain(|id, _| {
            id.0 == MASTER_WIRE_KEY || self.runtime.cards.iter().any(|card| card.id == *id)
        });
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
        // The `OUT` card meters the whole rack. Every card's reading is
        // wall-clock spent since the block began, so the busiest card is
        // the last one the audio passed through, and its total is the
        // rack's whole DSP cost — the utilization the DAC runs at.
        let busiest = self
            .runtime
            .cards
            .iter()
            .filter_map(|card| self.runtime.timings.sample(card.id))
            .max_by(|left, right| left.total().total_cmp(&right.total()));
        self.timing_meters
            .entry(NodeId(MASTER_WIRE_KEY))
            .or_default()
            .tick(busiest, dt, false);
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
                panel.scope = self.settings_scope;
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
            let canvas = structured_canvas(
                &self.runtime.cards,
                w,
                h,
                self.scroll,
                self.vertical,
                self.runtime.route(),
            );
            let geometry = canvas
                .as_ref()
                .map(|c| c.frame.clone())
                .unwrap_or_else(|| geometry(&groups, w, h, self.scroll, self.vertical));
            let drop_forks = canvas
                .as_ref()
                .map(|c| c.fork_targets(None))
                .unwrap_or_else(|| fork_marks(&groups, &geometry.slots, geometry.flow, None));
            let drop_runs = canvas
                .as_ref()
                .map(|c| c.run_targets(None))
                .unwrap_or_else(|| wire_runs(&groups, &geometry.slots, None));
            let placement = target_at(
                point.or(mouse),
                &geometry.slots,
                None,
                &DropZones {
                    joins: &drop_forks,
                    runs: &drop_runs,
                },
                geometry.top,
                geometry.bottom,
                w,
            );
            let placement = self.canonical_drop(placement, None);
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
            self.settings_scope = PageScope::All;
            self.settings = None;
            self.drag = None;
            // The settings page takes the whole window; an open dB
            // editor would have no card corner left to draw in.
            self.gain_edit = None;
            self.gain_input.set_focus(false);
            return;
        }
        let groups = groups(&self.runtime.cards);
        let content_width = crate::runtime::flow(&self.runtime.cards).map_or_else(
            |_| (groups.iter().map(|g| stage_width(g)).sum::<usize>() + 2) as f32 * COLUMN_STEP,
            |root| flow_canvas::Canvas::content_width(&root),
        );
        let max_scroll = (content_width + 320.0 - w).max(0.0);
        let previous = structured_canvas(
            &self.runtime.cards,
            w,
            h,
            self.scroll,
            self.vertical,
            self.runtime.route(),
        )
        .map(|c| c.frame)
        .unwrap_or_else(|| geometry(&groups, w, h, self.scroll, self.vertical));
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
        let canvas = structured_canvas(
            &self.runtime.cards,
            w,
            h,
            self.scroll,
            self.vertical,
            self.runtime.route(),
        );
        let layout = canvas
            .as_ref()
            .map(|c| c.frame.clone())
            .unwrap_or_else(|| geometry(&groups, w, h, self.scroll, self.vertical));
        // The route the rack is wired against, read once a frame like the
        // meter gate: it is what the two endpoint cards say, and it is how
        // wide the wire into the first card and out of the last one is
        // drawn. Everything between the two cards keeps the width the card
        // beside it has, which is the engine's own rule for the graph.
        let route = self.runtime.route();
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
        // The width conversions the engine wired for the cards as they stand,
        // seated on the wires that carry them. Worked out before any wire is
        // drawn or broken: a circle stands on a wire, so a right-click has to
        // be able to choose between the two, and the same frame that finds
        // the break has to know whether a circle is in the way.
        let marks = match self.runtime.junctions() {
            Some(found) => canvas
                .as_ref()
                .map(|c| c.marks(&found))
                .unwrap_or_else(|| width_marks(&found, &groups, &layout)),
            None => Vec::new(),
        };
        // Where every circle stands this frame, read once: the break guards,
        // the pickup and a held drag all want the same point, and the point is
        // needed before the offsets below are allowed to move.
        let stood = marks
            .iter()
            .map(|mark| (mark, mark.centre(&self.mark_offsets)))
            .collect::<Vec<(&WidthMark, [f32; 2])>>();
        let on_mark = stood
            .iter()
            .any(|(_, at)| hit(mouse, at[0], at[1], MARK_HIT, MARK_HIT));
        // A circle slides along the wire it stands on and nowhere else: the
        // conversion itself stays exactly where the engine wired it, and the
        // mark only ever gets moved out of the way of something it happens to
        // cover. The nudge is remembered against the node the compiled graph
        // named the conversion by, and dropped as soon as that graph stops
        // describing one, so nothing is carried over from a rack that no
        // longer needs it. A circle whose wire has grown too short to hold it
        // is not drawn at all, which is what drops a held mark the moment its
        // wire goes.
        self.mark_drag = self
            .mark_drag
            .filter(|(id, ..)| marks.iter().any(|mark| mark.id == *id));
        if let (Some(point), true) = (mouse, down) {
            if let Some((id, grab, base)) = self.mark_drag {
                if let Some((mark, _)) = stood.iter().find(|(mark, _)| mark.id == id) {
                    let offset = base + (mark.along(point) - grab);
                    self.mark_offsets.insert(id, offset);
                }
            } else if clicked
                && self.drag.is_none()
                && self.choosing.is_none()
                && let Some((mark, _)) = stood
                    .iter()
                    .find(|(_, at)| hit(mouse, at[0], at[1], MARK_HIT, MARK_HIT))
            {
                self.mark_drag = Some((
                    mark.id,
                    mark.along(point),
                    self.mark_offsets.get(&mark.id).copied().unwrap_or_default(),
                ));
            }
        } else {
            self.mark_drag = None;
        }
        self.mark_offsets
            .retain(|id, _| marks.iter().any(|mark| mark.id == *id));
        // Hit-testing above is one frame behind on purpose: the frame that
        // picks a circle up reports the position it was drawn at, which is the
        // only position the operator had seen when they pressed.
        // The rack's own `OUT` card: it stands one column right of every
        // card in the rack, and its input stub carries the master meter.
        // Worked out before any of the wiring, and before the corner
        // readouts are looked for: the card carries the master output
        // meter, whose trim corner moves with it, and a double-click on it
        // has to find the live column. An empty rack has the card on its
        // own, with nothing feeding it.
        let out = layout.output;
        let out_stub = wire_box_x(out.x, false);
        let out_visible = visible(&out, layout.top, layout.bottom, w);
        let gain_reset = if press {
            let target =
                if self.overlay.is_none() && self.choosing.is_none() && self.gain_drag.is_none() {
                    layout
                        .slots
                        .iter()
                        .filter(|slot| {
                            visible(slot, layout.top, layout.bottom, w)
                                && !self.runtime.paused(slot.id.0)
                                && !self.runtime.bypassed(slot.id.0)
                        })
                        .find_map(|slot| {
                            [false, true]
                                .into_iter()
                                .find(|side| {
                                    let [gx, gy] = gain_center(slot.x, slot.y, *side);
                                    hit(mouse, gx, gy, GAIN_HALF_W, GAIN_HALF_H)
                                })
                                .map(|side| (slot.id.0, side))
                        })
                        // The master trim corner answers a double-click
                        // like any card corner: no card claims the click
                        // here, and the `OUT` card's input corner is a
                        // candidate whenever the card is on screen.
                        .or_else(|| {
                            let [gx, gy] = gain_center(out.x, layout.flow, false);
                            (out_visible && hit(mouse, gx, gy, GAIN_HALF_W, GAIN_HALF_H))
                                .then_some((MASTER_WIRE_KEY, false))
                        })
                        // And so is the `IN` card's output corner — the
                        // capture trim, the mirror of the master's.
                        .or_else(|| {
                            let input = layout.input;
                            let [gx, gy] = gain_center(input.x, layout.flow, true);
                            (visible(&input, layout.top, layout.bottom, w)
                                && hit(mouse, gx, gy, GAIN_HALF_W, GAIN_HALF_H))
                            .then_some((INPUT_JACK.0, true))
                        })
                } else {
                    None
                };
            self.gain_click.press(target, std::time::Instant::now())
        } else {
            None
        };
        if let Some((id, side)) = gain_reset {
            self.set_trim(id, side, 0.0);
            self.gain_edit = None;
            self.gain_input.set_focus(false);
        }
        // The rack's own `IN` card, standing in the first column and read
        // exactly like the `OUT` card: what the device actually delivers
        // here, rate and channel count, nothing invented. It is not a card
        // in the rack's sense — nothing is dropped on it, nothing drags off
        // it. It does carry one wire's meter: the capture as it enters the
        // rack, trimmed, measured on the stub at its right edge — the twin
        // of the master meter at the far end, and the reason the cable
        // leaving this card starts at a stub you can see. Its outgoing switch
        // cuts the feed after metering. The
        // face is where the input half of the device page is opened,
        // because the capture is the one thing about it an operator can
        // change.
        let input = layout.input;
        let in_visible = visible(&input, layout.top, layout.bottom, w);
        let in_hover = in_visible
            && self.choosing.is_none()
            && hit(mouse, input.x, layout.flow, CARD_WIDTH / 2.0, TRACK_HALF);
        if in_visible {
            ctx.rectangle(
                input.x,
                layout.flow,
                CARD_WIDTH / 2.0,
                56.0,
                if in_hover {
                    rgb(58, 51, 79)
                } else {
                    rgb(42, 36, 61)
                },
                0.2,
            );
            ctx.text(
                input.x,
                layout.flow + NAME_TEXT_Y,
                &self.font,
                "IN",
                18.0,
                650.0,
                rgb(103, 237, 175),
                0.2,
            );
            ctx.text(
                input.x,
                layout_text_y(layout.flow),
                &self.font,
                format!(
                    "{} · {} ch",
                    rate_text(route.input_rate),
                    route.input.count()
                ),
                12.0,
                500.0,
                rgb(150, 145, 164),
                0.21,
            );
        }
        // The trim corner at the card's right edge keeps its own clicks,
        // exactly like the `OUT` card's: without this veto, opening the
        // dB editor would also grab the face and open the device page.
        let [in_gain_x, in_gain_y] = gain_center(input.x, layout.flow, true);
        if clicked
            && in_hover
            && self.drag.is_none()
            && !hit(mouse, in_gain_x, in_gain_y, GAIN_HALF_W, GAIN_HALF_H)
        {
            self.settings_page = true;
            self.settings_scope = PageScope::Input;
            self.settings = None;
            // As with the cog: the page takes the whole window, so an open
            // dB editor would have no card corner left to draw in.
            self.gain_edit = None;
            self.gain_input.set_focus(false);
        }
        ctx.text(
            0.0,
            h / 2.0 - 72.0,
            &self.font,
            "Card top/bottom: split    Above/below a wire: parallel    Left/right: chain",
            14.0,
            500.0,
            rgb(150, 145, 164),
            0.2,
        );
        let wire = rgb(107, 114, 128);
        // One gate for the whole rack, read once a frame: every stub box on
        // every wire is measured against the same number.
        let gate = self.runtime.meter_gate();
        // The capture meter reads the source before its outgoing switch.
        let in_stub = wire_box_x(layout.input.x, true);
        let input_key = (INPUT_JACK.0, true);
        if in_visible
            && clicked
            && self.choosing.is_none()
            && self.drag.is_none()
            && hit(mouse, in_stub, layout.flow, STUB_HALF, STUB_TALL)
        {
            // `watch_wires` has run for this frame, so the entry is
            // there; the insert arm only keeps the click from being
            // swallowed in some state where it is not.
            let meter = self.wire_meters.entry(input_key).or_insert_with(|| {
                let gain_db = remembered_trim(
                    self.runtime.trim(INPUT_JACK.0, true),
                    wire_slot_key(INPUT_JACK.0, true)
                        .and_then(|key| self.runtime.wires.gain_of(key)),
                );
                WireMeter {
                    open_circuit: false,
                    ..new_wire_meter(gain_db, now())
                }
            });
            meter.open = !meter.open;
            self.acknowledge_clip(MeterPos::Wire(INPUT_JACK.0, true));
        }
        if in_visible {
            wire_segment(
                ctx,
                layout.input.x + CARD_WIDTH / 2.0,
                in_stub,
                layout.flow,
                wire,
                route.input.count().clamp(1, 2),
                &[],
            );
            ctx.rectangle(
                in_stub,
                layout.flow,
                STUB_HALF,
                STUB_TALL,
                stub_color(self.wire_meters.get(&input_key), gate, wire),
                0.25,
            );
        }
        // The wire arrives from the rack's `IN` card's own stub. The run
        // into the `OUT` card is the master wire, whoever it last passed
        // through: its switch sits on the card's own stub, and a right-click
        // anywhere on the card, its wire or its stub mutes the output, meter
        // still reading what arrives.
        let out_hover = hit(mouse, out.x, layout.flow, CARD_WIDTH / 2.0, TRACK_HALF);
        if let Some(canvas) = &canvas {
            let master_face = broke
                && pause_target.is_none()
                && bypass_target.is_none()
                && out_visible
                && out_hover;
            if master_face {
                self.toggle_wire(MASTER_WIRE_KEY, false);
            }
            canvas.paint(
                self,
                ctx,
                flow_canvas::PaintInput {
                    mouse,
                    broke: broke
                        && !master_face
                        && pause_target.is_none()
                        && bypass_target.is_none(),
                    clicked,
                    on_mark,
                    gate,
                },
                &marks,
            );
        } else {
            let mut previous_x = wire_box_x(layout.input.x, true);
            // How wide the trunk is on the flow line as it arrives at each
            // stage. The trunk begins as wide as the device's capture, and past
            // each stage the mixer — when there is one — has folded the branches
            // down to the widest of their tails, so the trunk runs on at that
            // width. This is the same arithmetic the rack applies to the graph
            // itself when it builds one, for the same reason: nothing else
            // changes a wire's channel count.
            let mut incoming = route.input.count().clamp(1, 2);
            // The stage's own columns, worked out once for the frame: the base
            // is where its first column stands however the lanes are offset
            // within it, and its split and merge are the rings at boundary `0`
            // and boundary `width`.
            let columns = stage_columns(&groups, &layout.slots, layout.input.x);
            let input_end = columns.first().map_or(wire_box_x(out.x, false), |column| {
                column.split - if groups[0].len() > 1 { RING_CLEAR } else { 0.0 }
            });
            let input_wire_toggled = broke
                && !on_mark
                && bypass_target.is_none()
                && pause_target.is_none()
                && wire_hit(mouse, in_stub, input_end, layout.flow, in_stub);
            if input_wire_toggled {
                self.toggle_wire(INPUT_JACK.0, true);
            }
            // Consume this click before the destination card inspects its lead.
            let broke = broke && !input_wire_toggled;
            let gaps = wire_gaps(&groups, &layout, |id, side| self.wire_open(id, side));
            for (index, group) in groups.iter().enumerate() {
                let stage_slots = layout
                    .slots
                    .iter()
                    .filter(|slot| {
                        group
                            .iter()
                            .flat_map(|lane| lane.ids.iter())
                            .any(|id| *id == slot.id)
                    })
                    .collect::<Vec<_>>();
                // The stage's cards by lane, in the order the lanes run down: a
                // lane's head stands at the ring its span opens on, not
                // necessarily at the stage's own split.
                let stage = group
                    .iter()
                    .map(|lane| {
                        lane.ids
                            .iter()
                            .filter_map(|id| {
                                stage_slots.iter().copied().find(|slot| slot.id == *id)
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                let column = columns[index];
                let base = column.base;
                let width = column.width;
                let shared = group.len() > 1;
                // Whether the wire arriving here comes out of an earlier stage rather
                // than out of the rack's `IN` jack, which decides whether the left end
                // of this stage's input is a column somebody else's wire ends in.
                let fed = index > 0;
                let split = column.split;
                // The merge column stands right of whichever lane's span reaches
                // furthest — the risers gather every lane there.
                let merge = column.merge;
                // The trunk, up to the column this stage splits at, at the height
                // signals flow at. A stage on its own sits on that very line, so its
                // trunk is the connector an operator sees running between two meters,
                // and it is that card's input: an open switch cuts a fixed gap at
                // the source meter. In a parallel stage the signals of
                // every card in it pass down into the risers it feeds, so the trunk
                // is the stage's and stays whole — like the risers themselves, which
                // are nobody's to break.
                //
                // Drawn on the flow line wherever it is, and not only where a card is
                // visible: a rack scrolled along its wiring still shows it running
                // past the edge.
                // A parallel stage's collar stands on the trunk, so the trunk
                // stops at its outer rim and starts off again at the opposite rim; the ring
                // is the crossing the wire runs through, not ink drawn over.
                let feed_to = if shared { split - RING_CLEAR } else { split };
                for (from, to, lanes) in width_runs(
                    previous_x,
                    feed_to,
                    incoming,
                    layout.flow,
                    &marks,
                    &self.mark_offsets,
                ) {
                    wire_segment(ctx, from, to, layout.flow, wire, lanes, &gaps);
                }
                if shared {
                    // The risers, branch by branch: each one drops from the
                    // flow line into its head card carrying the width that
                    // head takes, and climbs into the merge column carrying
                    // the width its tail puts out. A branch that joins a
                    // stereo trunk down to a mono head therefore draws one
                    // riser, not two — its second conductor stays at the
                    // split column, because the join happens at that
                    // column's mixer and nowhere on the way down.
                    let row = |y: f32| y.max(layout.bottom).min(layout.top);
                    for (branch, lane) in stage.iter().zip(group) {
                        // Every lane hands to its own rings: a lane that opens
                        // or closes mid-stage drops and climbs at the boundary
                        // its span stands on, not at the stage's outer collar.
                        let head_ring = ring_x(base, lane.start, width);
                        let tail_ring = ring_x(base, lane.end, width);
                        if let (Some(head), Some(tail)) = (branch.first(), branch.last()) {
                            // The riser starts outside the collar's air gap and
                            // never runs under the disc; a branch standing on the
                            // flow line itself has no riser to draw — its signal
                            // simply runs on through the gap beside the ring.
                            let drop = row(head.y) - layout.flow;
                            if drop.abs() > RING_CLEAR {
                                wire_riser(
                                    ctx,
                                    head_ring,
                                    layout.flow + drop.signum() * RING_CLEAR,
                                    row(head.y),
                                    wire,
                                    self.wire_lanes(head.id.0, false),
                                );
                            }
                            let climb = row(tail.y) - layout.flow;
                            if climb.abs() > RING_CLEAR {
                                wire_riser(
                                    ctx,
                                    tail_ring,
                                    row(tail.y),
                                    layout.flow + climb.signum() * RING_CLEAR,
                                    wire,
                                    self.wire_lanes(tail.id.0, true),
                                );
                            }
                        }
                    }
                }
                for (branch, lane) in stage.iter().zip(group) {
                    // The rings this lane hands off on: a full-span lane's are
                    // the stage's own split and merge; one that opens or closes
                    // mid-stage reaches only the boundary its span stands on.
                    let head_ring = ring_x(base, lane.start, width);
                    let tail_ring = ring_x(base, lane.end, width);
                    for (position, slot) in branch.iter().copied().enumerate() {
                        if !visible(slot, layout.top, layout.bottom, w) {
                            continue;
                        }
                        // Where the card stands in its branch decides the wire it owns:
                        // the head takes the ring its lane opens on, the tail feeds
                        // the ring its lane closes on, and a card in between reaches
                        // only the junctions its own chain runs through — the one the
                        // signal came from, and the one it goes on to.
                        let head = position == 0;
                        let tail = position + 1 == branch.len();
                        let x = slot.x;
                        let chain_in = junction_x(branch[position.saturating_sub(1)].x, true);
                        // A lane that opens mid-stage starts at the ring its
                        // riser foot stands under — for a full-span lane that
                        // ring is the stage's own split, and nothing moves.
                        let lead_in = if head { head_ring } else { chain_in };
                        let lead_out = if tail { tail_ring } else { junction_x(x, true) };
                        let feed_in = if head { previous_x } else { chain_in };
                        // A tail card hands its signal to the ring its lane
                        // closes on even in the last stage: from there the run
                        // into the `OUT` card is the master wire's own, cut
                        // short only by the card's switch.
                        let out_end = if !tail {
                            junction_x(x, true)
                        } else {
                            tail_ring
                        };
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
                            // The whole stretch this side owns, narrowed to what a click
                            // on it may claim; see [`wire_click`]. The wire is still drawn
                            // in full, so an open one stays as easy to close as it was to
                            // open.
                            let (from, to) = wire_click(
                                x,
                                side,
                                shared && (if side { tail } else { head }),
                                fed || !head,
                                feed_in,
                                out_end,
                            );
                            if broke
                            && bypass_target.is_none()
                            && pause_target.is_none()
                            // A circle standing on the wire is not a wire to
                            // break: it is the mark of a conversion, and it
                            // never sits over a stub box, so nothing else
                            // loses its click to it.
                            && !on_mark
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
                                    let gain_db = remembered_trim(
                                        self.runtime.trim(slot.id.0, side),
                                        wire_slot_key(slot.id.0, side)
                                            .and_then(|key| self.runtime.wires.gain_of(key)),
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
                            // This card's own row: in from the column feeding its chain
                            // to the card's edge, and out from the edge to the column its
                            // chain hands off to. These are the short pieces a branch
                            // leaves to each of the cards in it; what an operator sees as
                            // one long connector between two meters is the trunk above,
                            // which runs on the flow line — the very row a solo card sits
                            // on.
                            let bypassed = self.runtime.bypassed(slot.id.0);
                            let (inner, outer) = card_lead(x, side, lead_in, lead_out, bypassed);
                            let lanes = self.wire_lanes(slot.id.0, side);
                            for (from, to, width) in
                                width_runs(inner, outer, lanes, slot.y, &marks, &self.mark_offsets)
                            {
                                wire_segment(ctx, from, to, slot.y, wire, width, &gaps);
                            }
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
                }
                // Out of this stage the trunk carries whatever the join hands
                // on: a single chain keeps its tail's width, and a parallel
                // stage leaves on the mixer, which folds the branches to the
                // widest of their tails — the same width the rack builds its
                // graph at.
                incoming = stage
                    .iter()
                    .filter_map(|branch| branch.last())
                    .map(|tail| self.wire_lanes(tail.id.0, true))
                    .max()
                    .unwrap_or_else(|| route.output.count().clamp(1, 2));
                // The next trunk hands over to the ring too: it starts where
                // the collar's clearance ends, so no wire crosses the rim on
                // its way out of a shared stage.
                previous_x = if shared { merge + RING_CLEAR } else { merge };
            }
            // The master wire, drawn past the loop so it keeps running when
            // the card it feeds has scrolled away. It ends at the `OUT`
            // card's edge and its switch sits on the card's own stub: a
            // right-click on the card's face, on the wire, or on the stub
            // mutes the output, and the meter keeps reading what arrives
            // while the circuit is open.
            let (mfrom, mto) = wire_click(
                out.x,
                false,
                false,
                !groups.is_empty(),
                previous_x,
                out.x - CARD_WIDTH / 2.0,
            );
            let out_hover = hit(mouse, out.x, layout.flow, CARD_WIDTH / 2.0, TRACK_HALF);
            if broke
            && bypass_target.is_none()
            && pause_target.is_none()
            // As with a card's own wires: a circle parked on the master wire
            // is a mark, not a link to break. Only the card's face and the
            // card's stub can mute the output from here, and a circle is kept
            // clear of both.
            && !on_mark
            && ((out_visible && out_hover) || wire_hit(mouse, mfrom, mto, layout.flow, out_stub))
            {
                self.toggle_wire(MASTER_WIRE_KEY, false);
            }
            let out_head = out.x - CARD_WIDTH / 2.0;
            for (from, to, lanes) in width_runs(
                previous_x,
                out_head,
                // The master wire is what the rack hands the device, so it runs
                // as wide as the playback side is: the engine refuses a graph
                // whose tail is any other width, map aside. A conversion
                // standing on the wire steps it down or up at its circle
                // regardless — the mark's own widths say what each side carries.
                route.output.count().clamp(1, 2),
                layout.flow,
                &marks,
                &self.mark_offsets,
            ) {
                wire_segment(ctx, from, to, layout.flow, wire, lanes, &gaps);
            }
        }
        // The conversions themselves, drawn last among the wires so a circle
        // always reads as the thing standing on a wire and never as a wire
        // passing under it. The ring is the collar the mixer's join already
        // uses for the same reason — a crossing is hard to see against the
        // strokes it cuts — and the pins are the proof of what it does: as
        // many strokes as the wire carries, on the side it carries them on,
        // pointing with the signal. The count is read off the compiled graph,
        // so the number of pins can only ever be the number the engine wired.
        for mark in &marks {
            let [x, y] = mark.centre(&self.mark_offsets);
            let color = if hit(mouse, x, y, MARK_HIT, MARK_HIT) {
                MARK_HOVER
            } else {
                wire
            };
            ctx.circle(x, y, MARK_RADIUS, color, 0.15);
            ctx.circle(x, y, MARK_RADIUS - 2.0, rgb(23, 18, 38), 0.16);
            // The pins start just inside the ring and run past it, onto the
            // wire they count, so a pin and the lane it belongs to meet.
            for (pins, ahead) in [(mark.inputs, -mark.forward), (mark.outputs, mark.forward)] {
                let (root, tip) = (4.0 * ahead, (MARK_RADIUS + 2.0) * ahead);
                for lane in 0..pins.clamp(1, 2) {
                    let offset = lane_offset(lane, pins);
                    if mark.vertical {
                        ctx.line(x + offset, y + root, x + offset, y + tip, color, 1.8, 0.22);
                    } else {
                        ctx.line(x + root, y + offset, x + tip, y + offset, color, 1.8, 0.22);
                    }
                }
            }
        }
        // The `OUT` card's face is also its way into the device page: a
        // left-click opens the playback half of it, exactly as the `IN`
        // card opens the capture half. The two corners of the face keep
        // their own clicks — the trim at its input corner and the budget
        // bar along its top — and the stub beside it is a switch, so the
        // face answers only where nothing else on it reaches.
        let [out_gain_x, out_gain_y] = gain_center(out.x, layout.flow, false);
        if clicked
            && out_visible
            && out_hover
            && self.drag.is_none()
            && self.choosing.is_none()
            && !hit(mouse, out_gain_x, out_gain_y, GAIN_HALF_W, GAIN_HALF_H)
            && !hit(mouse, out.x, layout.flow - 54.0, CARD_WIDTH / 2.0, 5.0)
        {
            self.settings_page = true;
            self.settings_scope = PageScope::Output;
            self.settings = None;
            self.gain_edit = None;
            self.gain_input.set_focus(false);
        }
        // The `OUT` card's face: nothing is ever dropped on it, dragged
        // off it or deleted from it — the face is where the trim corner
        // lives and where a right-click finds the mute switch.
        if out_visible {
            ctx.rectangle(
                out.x,
                layout.flow,
                CARD_WIDTH / 2.0,
                56.0,
                if out_hover {
                    rgb(58, 51, 79)
                } else {
                    rgb(42, 36, 61)
                },
                0.2,
            );
            ctx.text(
                out.x,
                layout.flow + NAME_TEXT_Y,
                &self.font,
                "OUT",
                18.0,
                650.0,
                rgb(103, 237, 175),
                0.2,
            );
            ctx.text(
                out.x,
                layout_text_y(layout.flow),
                &self.font,
                match self
                    .timing_meters
                    .get(&NodeId(MASTER_WIRE_KEY))
                    .and_then(TimingMeter::reading)
                {
                    Some(total) => format!(
                        "{} · {} ch · {:.1}%",
                        rate_text(route.output_rate),
                        route.output.count(),
                        total * 100.0
                    ),
                    None => format!(
                        "{} · {} ch",
                        rate_text(route.output_rate),
                        route.output.count()
                    ),
                },
                12.0,
                500.0,
                rgb(150, 145, 164),
                0.21,
            );
            // The rack's whole audio budget, drawn exactly like a card's
            // own bar: purple is spent before the last card ran, green
            // is that card, and hovering opens the same budget tooltip.
            let out_timing_hover = hit(mouse, out.x, layout.flow - 54.0, CARD_WIDTH / 2.0, 5.0);
            if let Some(meter) = self.timing_meters.get(&NodeId(MASTER_WIRE_KEY)) {
                meter.draw(
                    ctx,
                    &self.font,
                    [out.x, layout.flow],
                    CARD_WIDTH,
                    out_timing_hover
                        && self.drag.is_none()
                        && self.choosing.is_none()
                        && self.overlay.is_none()
                        && self.gain_edit.is_none(),
                );
            }
        }
        // The master output's stub, folded and unfolded by a click
        // exactly like a card's, and the trim in the card's input corner.
        // It meters what leaves the rack — every branch summed, then
        // trimmed — and it carries a switch like any card's stub: the
        // right hand mutes the output above, and the meter keeps reading
        // what arrives while the circuit is open.
        let master_key = (MASTER_WIRE_KEY, false);
        if clicked
            && self.choosing.is_none()
            && hit(mouse, out_stub, layout.flow, STUB_HALF, STUB_TALL)
        {
            // `watch_wires` has run for this frame, so the entry is
            // there; the insert arm only keeps the click from being
            // swallowed in some state where it is not.
            let meter = self.wire_meters.entry(master_key).or_insert_with(|| {
                let gain_db = remembered_trim(
                    self.runtime.trim(MASTER_WIRE_KEY, false),
                    self.runtime.wires.gain_of(MASTER_WIRE_KEY),
                );
                WireMeter {
                    open_circuit: false,
                    ..new_wire_meter(gain_db, now())
                }
            });
            meter.open = !meter.open;
            self.acknowledge_clip(MeterPos::Wire(MASTER_WIRE_KEY, false));
        }
        ctx.rectangle(
            out_stub,
            layout.flow,
            STUB_HALF,
            STUB_TALL,
            stub_color(self.wire_meters.get(&master_key), gate, wire),
            0.25,
        );
        // A press on a corner readout belongs to that readout: it
        // stands inside its card's hover rect, so without this veto the
        // opening click would also grab the card and start a drag. The
        // readout's rect stands on every card the rack draws — entry or
        // not, expanded or collapsed, corner drawn or blanked at unity —
        // so the veto is the rect alone, and the number is never a way
        // to move a card.
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
                    format!("{} → {}", card.inputs, card.outputs)
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
        // Cancellation wins over a release delivered in the same frame.
        if ctx.key_down(frost::KeyCode::Escape) {
            self.drag = None;
            self.press_origin = None;
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
            let joins = canvas
                .as_ref()
                .map(|c| c.fork_targets(self.drag))
                .unwrap_or_else(|| fork_marks(&groups, &layout.slots, layout.flow, self.drag));
            let runs = canvas
                .as_ref()
                .map(|c| c.run_targets(self.drag))
                .unwrap_or_else(|| wire_runs(&groups, &layout.slots, self.drag));
            let target = target_at(
                preview_mouse,
                &layout.slots,
                self.drag,
                &DropZones {
                    joins: &joins,
                    runs: &runs,
                },
                layout.top,
                layout.bottom,
                w,
            );
            let target = self.canonical_drop(target, self.drag);
            if let Some(target) = target {
                let slot = layout
                    .slots
                    .iter()
                    .find(|slot| slot.id == target.card)
                    .unwrap();
                let parallel = matches!(target.side, Side::Above | Side::Below);
                // Mirrors `place`: a ring drop belongs to the whole fork,
                // so its tick spans every lane; any other drop belongs to
                // the hovered lane, at the position it was aimed at.
                match target.side {
                    Side::Split | Side::Join => {
                        if let Some(mark) = joins.iter().find(|mark| {
                            mark.scope == target.scope
                                && mark.anchor == target.card
                                && mark.side == target.side
                        }) {
                            let [x, y] = fork_drop_point(mark);
                            ctx.rectangle(x, y, 2.0, 70.0, rgb(103, 237, 175), 0.5);
                        }
                    }
                    Side::Left | Side::Right => {
                        let [x, y] = serial_drop_point(&target, slot, &runs);
                        ctx.rectangle(x, y, 2.0, 70.0, rgb(103, 237, 175), 0.5);
                    }
                    Side::Above | Side::Below => {
                        let [x, y] = parallel_drop_point(
                            &target,
                            slot,
                            &layout.slots,
                            self.drop_cache.as_ref().map(|cache| &cache.targets),
                        );
                        ctx.rectangle(x, y, 130.0, 2.0, rgb(103, 237, 175), 0.5);
                    }
                }
                if let Some([x, y]) = preview_mouse {
                    ctx.text(
                        x,
                        y + 30.0,
                        &self.font,
                        match target.side {
                            Side::Split => "Ahead of the split",
                            Side::Join => "After the merge",
                            Side::Above | Side::Below => "Split around the card",
                            _ if parallel => "Parallel branch",
                            _ => "Chain",
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
        // belongs to — and for the master output, on the `OUT` card's
        // input corner, a card that is never gone and only ever moves
        // with the layout.
        if let Some((id, side)) = self.gain_edit {
            let master = id == MASTER_WIRE_KEY;
            // The `IN` jack's corner is the same story as the master's:
            // no card is keyed `0`, so the card never goes — the field
            // belongs to a fixed corner of the canvas.
            let input_jack = !master && id == INPUT_JACK.0 && side;
            let gone = !master && !input_jack && !layout.slots.iter().any(|slot| slot.id.0 == id);
            if ctx.key_down(frost::KeyCode::Escape) || self.overlay.is_some() || gone {
                self.gain_edit = None;
                self.gain_input.set_focus(false);
            } else if press {
                let inside = if master {
                    let [gx, gy] = gain_center(out.x, layout.flow, false);
                    out_visible && hit(mouse, gx, gy, GAIN_HALF_W, GAIN_HALF_H)
                } else if input_jack {
                    let input = layout.input;
                    let [gx, gy] = gain_center(input.x, layout.flow, true);
                    visible(&input, layout.top, layout.bottom, w)
                        && hit(mouse, gx, gy, GAIN_HALF_W, GAIN_HALF_H)
                } else {
                    layout
                        .slots
                        .iter()
                        .find(|slot| slot.id.0 == id && visible(slot, layout.top, layout.bottom, w))
                        .is_some_and(|slot| {
                            let [gx, gy] = gain_center(slot.x, slot.y, side);
                            hit(mouse, gx, gy, GAIN_HALF_W, GAIN_HALF_H)
                        })
                };
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
            // A wire whose card was removed — and there is no card behind
            // the master jack's phantom input side, which `watch_wires`
            // never gives an entry either way — samples as gone, so its
            // meter decays to silence on its own. Nothing prunes the entry,
            // and nothing draws it either: its id is never handed out again.
            let key = wire_slot_key(*id, *side);
            // Re-state the trim every tick: a wire registered while audio
            // was stopped comes up at unity, and this re-claim is what
            // keeps the audible gain where the white line shows it.
            if let Some(key) = key {
                self.runtime
                    .wires
                    .set_gain(key, 10f32.powf(wire.gain_db / 20.0));
                // And the switch, for the same reason: a wire registered while
                // audio was stopped comes back closed, and a rack that was
                // saved with a cable out of it has to go quiet again where it
                // was quiet, whether or not the engine happened to be running
                // when it was switched. The master rides this same loop: the
                // jack's break lives only for the session, but while it is
                // open the audio side has to keep seeing it.
                self.runtime.wires.set_open(key, wire.open_circuit);
            }
            // The engine publishes both meter lanes for every wire; a
            // mono wire simply publishes silence on its second, and a
            // wider one has already been folded into two lanes by the
            // engine, so two ticks per wire is all the rack ever does.
            let lanes = key
                .and_then(|key| self.runtime.wires.sample_lanes(key))
                .unwrap_or_default();
            if *side && self.runtime.paused(*id) && !self.runtime.bypassed(*id) {
                wire.restart(time);
            } else {
                for (lane, meter) in wire.peaks.iter_mut().enumerate() {
                    meter.tick(db(lanes.rms[lane]), db(lanes.peak[lane]), time);
                }
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
                let paused = self.runtime.paused(slot.id.0);
                self.paint_wire_meter(
                    ctx,
                    dt,
                    mouse,
                    down,
                    clicked,
                    gain_reset,
                    slot.id.0,
                    side,
                    [wire_box_x(slot.x, side), slot.y],
                    gain_center(slot.x, slot.y, side),
                    paused,
                );
            }
        }
        // The `OUT` card carries the same meter for the summed signal, at
        // the same handles and with the same gestures; it is drawn
        // whenever its card is, never paused, and holds the mute switch
        // on its stub.
        if out_visible {
            self.paint_wire_meter(
                ctx,
                dt,
                mouse,
                down,
                clicked,
                gain_reset,
                MASTER_WIRE_KEY,
                false,
                [out_stub, layout.flow],
                gain_center(out.x, layout.flow, false),
                false,
            );
        }
        // The `IN` card carries its twin for the capture entering the
        // rack — same meter, same trim gestures, no switch behind it.
        if in_visible {
            self.paint_wire_meter(
                ctx,
                dt,
                mouse,
                down,
                clicked,
                gain_reset,
                INPUT_JACK.0,
                true,
                [in_stub, layout.flow],
                gain_center(layout.input.x, layout.flow, true),
                false,
            );
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
        format!("{} → {}", card.inputs, card.outputs),
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
    /// Fixture lanes spanning the whole stage — the shape the rack drew
    /// before a lane owned its own span. The stage runs as wide as its
    /// longest lane, so every lane reaches from split to merge exactly
    /// as the old bare chain-of-chains shape meant it did.
    fn spanning(rows: &[&[u64]]) -> Vec<GuiLane> {
        let width = rows.iter().map(|row| row.len()).max().unwrap_or(1).max(1);
        rows.iter()
            .map(|row| GuiLane {
                ids: row.iter().map(|id| NodeId(*id)).collect(),
                start: 0,
                end: width,
            })
            .collect()
    }
    #[test]
    fn a_ring_drop_names_the_whole_fork_and_a_lane_drop_its_lane() {
        let stages = vec![
            spanning(&[&[1]]),
            spanning(&[&[3, 4], &[5]]),
            spanning(&[&[7]]),
        ];
        let aim = |card: u64, side: Side| Target {
            scope: None,
            card: NodeId(card),
            side,
        };
        // Either ring claims the whole fork, at either end of it…
        assert_eq!(
            fork_lanes(&stages, None, &aim(3, Side::Split)),
            Some(vec![NodeId(3), NodeId(5)])
        );
        assert_eq!(
            fork_lanes(&stages, None, &aim(4, Side::Join)),
            Some(vec![NodeId(3), NodeId(5)])
        );
        // …while a left drop, on a head or not, is just a chain into
        // its lane and spans nothing.
        assert_eq!(fork_lanes(&stages, None, &aim(3, Side::Left)), None);
        // A fork whose lane is half-dragged still spans its survivors,
        // but dragging a whole lane away leaves a single chain: no fork,
        // and no span.
        assert_eq!(
            fork_lanes(&stages, Some(NodeId(4)), &aim(3, Side::Split)),
            Some(vec![NodeId(3), NodeId(5)])
        );
        assert_eq!(
            fork_lanes(&stages, Some(NodeId(5)), &aim(3, Side::Join)),
            None
        );
        // A lone lane is no fork to stand ahead of or behind.
        let chain = vec![spanning(&[&[1, 3]])];
        assert_eq!(fork_lanes(&chain, None, &aim(1, Side::Split)), None);
    }
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
            // High on a card's face, up or down and clear of the middle,
            // the drop asks for a fork of its own around that column.
            ([10.0, 20.0], Side::Above),
            ([-10.0, -20.0], Side::Below),
            ([0.0, 30.0], Side::Above),
            // Low on the face, on the card's own line, the middle still
            // chains — only the up-and-down pull breaks it out.
            ([0.0, 10.0], Side::Right),
            // Held out at the face's corners, the pull is sideways and the
            // lane keeps it: chain into that end.
            ([92.0, 34.0], Side::Right),
            ([-92.0, -34.0], Side::Left),
            // A hair past the face, still in the column, the fork answer
            // goes on: the gap above or below a card is its face's ask.
            ([0.0, 36.0], Side::Above),
            ([-100.0, 0.0], Side::Left),
            ([100.0, 0.0], Side::Right),
            ([0.0, 75.0], Side::Above),
            ([0.0, -75.0], Side::Below),
            // Flung well past a card's side but deeper still vertically,
            // the drop reads as the row it reaches for, not a chain.
            ([110.0, 130.0], Side::Above),
            ([-110.0, 130.0], Side::Above),
            ([-110.0, -130.0], Side::Below),
            ([110.0, -130.0], Side::Below),
            // A shallow diagonal beside a lone card still means chain.
            ([110.0, 40.0], Side::Right),
            ([-110.0, -40.0], Side::Left),
        ] {
            assert_eq!(
                target_at(
                    Some(point),
                    &slots,
                    None,
                    &DropZones::default(),
                    200.0,
                    -150.0,
                    1000.0,
                ),
                Some(Target {
                    scope: None,
                    card: NodeId(1),
                    side
                })
            );
            assert_eq!(
                target_at(
                    Some(point),
                    &slots,
                    Some(NodeId(3)),
                    &DropZones::default(),
                    200.0,
                    -150.0,
                    1000.0
                ),
                Some(Target {
                    scope: None,
                    card: NodeId(1),
                    side
                })
            );
        }
        assert!(
            target_at(
                Some([0.0, 250.0]),
                &slots,
                None,
                &DropZones::default(),
                200.0,
                -150.0,
                1000.0,
            )
            .is_none()
        );
    }
    #[test]
    fn both_rings_guard_every_fork() {
        // A two-lane stage with a card each, its trunk on the middle line:
        // one ring where the lanes fan apart, one where they merge back.
        let groups = vec![spanning(&[&[3], &[5]])];
        let slots = [
            Slot {
                id: NodeId(3),
                x: 224.0,
                y: 70.0,
            },
            Slot {
                id: NodeId(5),
                x: 224.0,
                y: -70.0,
            },
        ];
        let marks = fork_marks(&groups, &slots, 0.0, None);
        assert_eq!(marks.len(), 2);
        let split = junction_x(224.0, false);
        let merge = junction_x(224.0, true);
        assert_eq!(
            (marks[0].anchor, marks[0].x, marks[0].y, marks[0].side),
            (NodeId(3), split, 0.0, Side::Split)
        );
        assert_eq!(
            (marks[1].anchor, marks[1].x, marks[1].y, marks[1].side),
            (NodeId(3), merge, 0.0, Side::Join)
        );
        // The adjacent wire, rather than the circle, inserts outside the fork.
        for mark in &marks {
            let side = mark.side;
            assert!(
                target_at(
                    Some([mark.x, mark.y]),
                    &slots,
                    None,
                    &DropZones {
                        joins: &marks,
                        runs: &[]
                    },
                    200.0,
                    -150.0,
                    1000.0
                )
                .is_none()
            );
            assert_eq!(
                target_at(
                    Some(fork_drop_point(mark)),
                    &slots,
                    None,
                    &DropZones {
                        joins: &marks,
                        runs: &[],
                    },
                    200.0,
                    -150.0,
                    1000.0,
                ),
                Some(Target {
                    scope: None,
                    card: NodeId(3),
                    side
                })
            );
        }
        // Well right of the merge, level with the top lane, the drop chains
        // into that lane as ever.
        assert_eq!(
            target_at(
                Some([merge + 40.0, 70.0]),
                &slots,
                None,
                &DropZones {
                    joins: &marks,
                    runs: &[],
                },
                200.0,
                -150.0,
                1000.0
            ),
            Some(Target {
                scope: None,
                card: NodeId(3),
                side: Side::Right
            })
        );
        // A dragged card never anchors its own fork, and a single-lane stage
        // draws no rings at all.
        let marks = fork_marks(&groups, &slots, 0.0, Some(NodeId(3)));
        assert_eq!(marks.len(), 2);
        assert!(marks.iter().all(|mark| mark.anchor == NodeId(5)));
        assert!(fork_marks(&[spanning(&[&[3]])], &slots, 0.0, None).is_empty());
        // Two forked stages answer for themselves; every local fork gets
        // its own pair of doors.
        let twice = vec![spanning(&[&[3], &[5]]), spanning(&[&[7], &[9]])];
        let twice_slots = [
            Slot {
                id: NodeId(3),
                x: 0.0,
                y: 70.0,
            },
            Slot {
                id: NodeId(5),
                x: 0.0,
                y: -70.0,
            },
            Slot {
                id: NodeId(7),
                x: 224.0,
                y: 70.0,
            },
            Slot {
                id: NodeId(9),
                x: 224.0,
                y: -70.0,
            },
        ];
        let marks = fork_marks(&twice, &twice_slots, 0.0, None);
        assert_eq!(marks.len(), 4);
        assert_eq!(marks[2].anchor, NodeId(7));
        assert_eq!(marks[2].x, junction_x(224.0, false));
        assert_eq!(marks[3].x, junction_x(224.0, true));
        // The preview spans the fork at either door, one survivor per lane.
        let at = |card: u64, side: Side| Target {
            scope: None,
            card: NodeId(card),
            side,
        };
        assert_eq!(
            fork_lanes(&groups, None, &at(3, Side::Split)),
            Some(vec![NodeId(3), NodeId(5)])
        );
        assert_eq!(
            fork_lanes(&groups, None, &at(3, Side::Join)),
            Some(vec![NodeId(3), NodeId(5)])
        );
        // Dragging a whole lane away collapses the span to a chain.
        assert_eq!(
            fork_lanes(
                &[spanning(&[&[3, 4], &[5]])],
                Some(NodeId(5)),
                &at(3, Side::Join)
            ),
            None
        );
    }
    #[test]
    fn a_faces_graze_outranks_the_ring_it_nearests() {
        // The tightest stack of lanes still leaves the split ring's reach
        // clear of the lane faces — and wherever they could touch, the
        // face wins: a graze of a lane head stays inside its lane.
        let groups = vec![spanning(&[&[3], &[5]])];
        let slots = [
            Slot {
                id: NodeId(3),
                x: 224.0,
                y: 58.0,
            },
            Slot {
                id: NodeId(5),
                x: 224.0,
                y: -58.0,
            },
        ];
        let marks = fork_marks(&groups, &slots, 0.0, None);
        let split = junction_x(224.0, false);
        // Junction clearance now keeps the ring away from the card face.
        assert!((135.0 - split).powi(2) + 24.0f32.powi(2) > JOIN_HIT * JOIN_HIT);
        // …and on the near edge of the top lane's head, where it must read
        // as a chain into that lane, not a new stage ahead of the fork.
        assert_eq!(
            target_at(
                Some([135.0, 24.0]),
                &slots,
                None,
                &DropZones {
                    joins: &marks,
                    runs: &[],
                },
                200.0,
                -150.0,
                1000.0
            ),
            Some(Target {
                scope: None,
                card: NodeId(3),
                side: Side::Left
            })
        );
    }
    #[test]
    fn the_shoulder_of_a_lanes_wire_opens_a_lane_of_its_own() {
        // Two cards chained in one lane leave a wire between them with room
        // above and below. A card held by that shoulder reaches for the
        // run's upstream card: the pick answers with that card and the
        // side of the shoulder, and `place` opens the local fork around
        // the upstream card alone — no full-run bracket. A card held level
        // with the wire still threads straight through the lane.
        let groups = vec![spanning(&[&[1, 2]])];
        let slots = [
            Slot {
                id: NodeId(1),
                x: 0.0,
                y: 0.0,
            },
            Slot {
                id: NodeId(2),
                x: 248.0,
                y: 0.0,
            },
        ];
        let runs = wire_runs(&groups, &slots, None);
        assert_eq!(runs.len(), 1);
        assert_eq!(
            (runs[0].card, runs[0].x0, runs[0].x1, runs[0].y),
            (NodeId(1), 93.0, 155.0, 0.0)
        );
        let pick = |point: [f32; 2]| {
            target_at(
                Some(point),
                &slots,
                Some(NodeId(9)),
                &DropZones {
                    joins: &[],
                    runs: &runs,
                },
                200.0,
                -150.0,
                1000.0,
            )
        };
        assert_eq!(
            pick([124.0, 30.0]),
            Some(Target {
                scope: None,
                card: NodeId(1),
                side: Side::Above
            })
        );
        assert_eq!(
            pick([124.0, -30.0]),
            Some(Target {
                scope: None,
                card: NodeId(1),
                side: Side::Below
            })
        );
        // Level with the wire the chain answer stands: the shoulder is only
        // the air above and below it.
        assert_eq!(
            pick([124.0, 8.0]),
            Some(Target {
                scope: None,
                card: NodeId(1),
                side: Side::Right
            })
        );
        // Inside a column but past the run's shoulder, the card's own
        // gap answer rules: above the card is its fork, and a card's own
        // stub is never a landing: dragging one of the pair leaves no run.
        assert_eq!(
            pick([40.0, 70.0]),
            Some(Target {
                scope: None,
                card: NodeId(1),
                side: Side::Above
            })
        );
        assert!(wire_runs(&groups, &slots, Some(NodeId(1))).is_empty());
    }
    #[test]
    fn parallel_cards_are_stacked_and_stages_remain_left_to_right() {
        let groups = vec![spanning(&[&[1], &[3]]), spanning(&[&[5]])];
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
            peaks: [PeakMeter::new(0); 2],
            gain_db: -6.5,
            open: true,
            open_circuit: false,
        };
        let collapsed = WireMeter {
            peaks: [PeakMeter::new(0); 2],
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
    #[test]
    fn a_corner_prints_one_decimal_and_leaves_unity_blank() {
        // The blank is decided on the string the corner would have drawn,
        // not on the float behind it: a trim of −0.04 dB would print
        // `−0.0`, and so stands in the same blank as the exact 0.0 the
        // double-click reset leaves behind.
        assert_eq!(corner_label(0.0, false), None);
        assert_eq!(corner_label(-0.0, false), None);
        assert_eq!(corner_label(0.04, false), None);
        assert_eq!(corner_label(-0.04, false), None);
        // A drag is the exception: while it holds the pointer the corner
        // is the drag's live readout, so unity prints too — and the same
        // value with `dragging` back to false, as the release hands it
        // over, falls into the blank again.
        assert_eq!(corner_label(0.0, true), Some("+0.0".to_string()));
        assert_eq!(corner_label(-0.04, true), Some("-0.0".to_string()));
        // Everywhere else the corner shows one decimal, sign included,
        // and the clamp still prints at both ends of the trim range.
        assert_eq!(corner_label(-0.1, false), Some("-0.1".to_string()));
        assert_eq!(corner_label(2.0, false), Some("+2.0".to_string()));
        assert_eq!(corner_label(-6.53, false), Some("-6.5".to_string()));
        assert_eq!(
            corner_label(-crate::TRIM_DB, false),
            Some("-24.0".to_string())
        );
        assert_eq!(
            corner_label(crate::TRIM_DB, false),
            Some("+24.0".to_string())
        );
        assert_eq!(corner_label(-6.53, true), Some("-6.5".to_string()));
    }

    /// `in → [a] → [b ∥ c] → [d] → out`, the shape `runtime::chain` wires:
    /// stages in series, `b` and `c` in parallel inside the middle one —
    /// every lane spanning its stage, as the rack drew before a lane owned
    /// its own span.
    fn rack() -> Vec<Vec<GuiLane>> {
        vec![
            spanning(&[&[1]]),
            spanning(&[&[3], &[5]]),
            spanning(&[&[7]]),
        ]
    }
    fn covers(from: MeterPos) -> Vec<MeterPos> {
        acknowledged(from, &rack())
    }
    /// Where the rack's own `IN` card stands, in the same units as
    /// [`wire_targets`]: the rack's inset from `left`, plus half a card.
    fn in_x() -> f32 {
        -158.0 + 183.0
    }
    /// Where a stage's card stands, in the same units as [`wire_targets`]: one
    /// column right of the `IN` card, plus [`COLUMN_STEP`] for every column to
    /// its left.
    fn card_x(index: usize) -> f32 {
        in_x() + COLUMN_STEP + index as f32 * COLUMN_STEP
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
    /// out, the cards numbered in the order the pass walks them. `shape` is the
    /// rack's own nesting: one slice per stage, and in it one number per
    /// branch — how many cards that branch chains together — so
    /// `&[&[1], &[2, 1]]` reads `[a] → [(b → c) ∥ d]`. The geometry is the
    /// rack's: a stage on its own sits on the line signals flow at, and a
    /// stage holding several branches hangs them a row apart, centred on that
    /// line; a card in a chain stands a column right of the card feeding it.
    fn wire_targets(shape: &[&[usize]]) -> Vec<Click> {
        let flow = 0.0;
        let row_step = 128.0;
        let widest = |stage: &[usize]| stage.iter().max().copied().unwrap_or(1);
        let mut targets = Vec::new();
        let mut previous_x = wire_box_x(in_x(), true);
        let mut card = 0;
        let mut column = 0;
        for (index, stage) in shape.iter().enumerate() {
            let shared = stage.len() > 1;
            let fed = index > 0;
            let merge = junction_x(card_x(column + widest(stage) - 1), true);
            for (row, length) in stage.iter().enumerate() {
                let y = if shared {
                    flow + (row as f32 - (stage.len() as f32 - 1.0) / 2.0) * row_step
                } else {
                    flow
                };
                for position in 0..*length {
                    let head = position == 0;
                    let tail = position + 1 == *length;
                    let x = card_x(column + position);
                    let chain_in = junction_x(card_x(column + position.saturating_sub(1)), true);
                    let feed_in = if head { previous_x } else { chain_in };
                    let out_end = if !tail { junction_x(x, true) } else { merge };
                    for side in [false, true] {
                        let (from, to) = wire_click(
                            x,
                            side,
                            shared && (if side { tail } else { head }),
                            fed || !head,
                            feed_in,
                            out_end,
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
            }
            previous_x = if shared { merge + RING_CLEAR } else { merge };
            column += widest(stage);
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
            MeterPos::Wire(MASTER_WIRE_KEY, false),
            MeterPos::MasterOut,
        ] {
            assert!(
                out.contains(&pos),
                "{pos:?} is downstream of a clip at the first card's output, and should have been cleared with it"
            );
        }
        assert_eq!(out.len(), 9, "and nothing else is: {out:?}");
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
        assert_eq!(
            inwards.len(),
            11,
            "itself, every wire, the output jack's meter, and the output"
        );
        for id in [1, 3, 5, 7] {
            assert!(inwards.contains(&MeterPos::Wire(id, false)));
            assert!(inwards.contains(&MeterPos::Wire(id, true)));
        }
        assert!(inwards.contains(&MeterPos::Wire(MASTER_WIRE_KEY, false)));
        assert!(inwards.contains(&MeterPos::MasterOut));
        assert_eq!(
            covers(MeterPos::MasterOut),
            vec![MeterPos::MasterOut, MeterPos::Wire(MASTER_WIRE_KEY, false)],
            "nothing is after the end of the chain but the two faces of \
             that end, which acknowledge each other"
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
        // rack's own output — the jack's meter and the panel face above it —
        // still sits after wherever it was.
        let out = acknowledged(MeterPos::Wire(99, false), &rack());
        assert_eq!(
            out,
            vec![
                MeterPos::Wire(99, false),
                MeterPos::Wire(99, true),
                MeterPos::Wire(MASTER_WIRE_KEY, false),
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
            peaks: [peak, PeakMeter::new(0)],
            gain_db: 0.0,
            open: false,
            open_circuit: false,
        };
        assert!(
            corner_reading(Some(&collapsed), None).1,
            "the clip is still on record"
        );
        let mut cleared = collapsed;
        cleared.clear_clip();
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
            assert!(!wire.clipping());
        }
    }
    #[test]
    fn the_master_key_is_never_a_shifted_card_id() {
        // Every GUI talk to `WireLevels` goes through `wire_slot_key`, so
        // this pins its two rules: a card keeps the engine's own key, and
        // the OUT jack keeps the sentinel the engine publishes for it.
        // The old shift, `u64::MAX << 1`, wrapped straight through zero
        // into the card keys — and in a debug build it panics outright.
        for id in [0u64, 1, 4, 9, 1024, (u64::MAX >> 1) - 1] {
            for side in [false, true] {
                assert_eq!(wire_slot_key(id, side), card_wire_key(id, side));
            }
        }
        assert_eq!(wire_slot_key(MASTER_WIRE_KEY, false), Some(MASTER_WIRE_KEY));
        assert_eq!(MASTER_WIRE_KEY, u64::MAX);
    }
    #[test]
    fn the_master_id_is_watched_on_one_side_only() {
        use std::collections::BTreeMap;
        // A card is watched twice, for its two stubs; the jack has one
        // wire — the one leaving it — and an id of `u64::MAX` must not
        // conjure a phantom second meter beside it.
        let mut meters: BTreeMap<(u64, bool), WireMeter> = BTreeMap::new();
        watch_wires(
            &mut meters,
            [3, MASTER_WIRE_KEY],
            |_, _| None,
            |key| (key == MASTER_WIRE_KEY).then_some(0.25),
            |_, _| false,
            0,
        );
        let keys: Vec<(u64, bool)> = meters.keys().copied().collect();
        assert_eq!(keys, vec![(3, false), (3, true), (MASTER_WIRE_KEY, false)]);
        assert!(
            (meters[&(MASTER_WIRE_KEY, false)].gain_db + 12.0412).abs() < 0.02,
            "the jack's meter is born at the gain its wire carries: {:?}",
            meters[&(MASTER_WIRE_KEY, false)].gain_db
        );
    }
    #[test]
    fn the_input_jack_is_watched_on_one_side_only() {
        use std::collections::BTreeMap;
        // The IN stub is one wire too — the capture as it enters the rack,
        // after its trim — and id 0 must not conjure a phantom second meter
        // on the card's other face, where no stub is drawn at all. Like the
        // master's, its meter is born at the gain its wire carries, which is
        // the engine key the capture trims under.
        let mut meters: BTreeMap<(u64, bool), WireMeter> = BTreeMap::new();
        watch_wires(
            &mut meters,
            [INPUT_JACK.0, 3],
            |_, _| None,
            |key| (Some(key) == card_wire_key(INPUT_JACK.0, true)).then_some(0.5),
            |_, _| false,
            0,
        );
        let keys: Vec<(u64, bool)> = meters.keys().copied().collect();
        assert_eq!(keys, vec![(INPUT_JACK.0, true), (3, false), (3, true)]);
        assert!(
            (meters[&(INPUT_JACK.0, true)].gain_db + 6.0206).abs() < 0.02,
            "the capture meter is born at the trim already in force: {:?}",
            meters[&(INPUT_JACK.0, true)].gain_db
        );
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
        wire.peaks[0].tick(-18.0, 1.4, 16);
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
        assert!(wire.clipping(), "still reporting the clip");
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
        wire.peaks[0].tick(-18.0, 1.4, 16);
        for t in (16..4_000).step_by(16) {
            wire.peaks[0].tick(-60.0, -60.0, t);
        }
        assert!(
            wire.clipping() && !wire.open,
            "red on a meter nobody opened"
        );
        wire.clear_clip();
        assert!(!wire.clipping(), "and one click of that stub clears it");
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
            meter.peaks[0].tick(-20.0, -20.0, t);
        }
        let loud = meters.get(&(5, false)).unwrap();
        assert!(loud.level_db() > -21.0, "the bar got there");
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
        meter.peaks[0].tick(-20.0, 1.4, 200);
        for t in (216..4_000).step_by(16) {
            meter.peaks[0].tick(-120.0, -120.0, t);
        }
        let burnt = meters.get_mut(&(5, false)).unwrap();
        assert!(burnt.clipping() && burnt.level_db() < -60.0);
        assert_eq!(
            stub_color(Some(burnt), -60.0, wire),
            nice_common::knob::RED,
            "red while the bar is well under the gate"
        );
        burnt.clear_clip();
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
        meter.peaks[0].tick(-20.0, 1.4, 200);
        for t in (216..4_000).step_by(16) {
            meter.peaks[0].tick(-120.0, -120.0, t);
        }
        let burnt = meters.get_mut(&(5, false)).unwrap();
        assert!(burnt.open_circuit && burnt.clipping());
        assert_eq!(stub_color(Some(burnt), -60.0, wire), nice_common::knob::RED);
        burnt.clear_clip();
        assert_eq!(
            stub_color(Some(burnt), -60.0, wire),
            nice_common::knob::AMBER,
            "and the break is still there to be read once the clip has been"
        );
    }
    #[test]
    fn breaks_are_fixed_size_at_the_meter_not_the_length_of_the_cable() {
        for end in [30.0, 300.0, 3000.0] {
            let gap = meter_gap(10.0, 0.0, true);
            assert_eq!(gap.start, 10.0 + STUB_HALF);
            assert_eq!(gap.end - gap.start, BREAK_GAP);
            assert_eq!(
                connected_runs(0.0, end, 0.0, &[gap]),
                vec![(0.0, gap.start), (gap.end, end)]
            );
            assert_eq!(connected_runs(0.0, end, 20.0, &[gap]), vec![(0.0, end)]);
        }
        let gap = meter_gap(100.0, 0.0, false);
        assert_eq!(gap.end, 100.0 - STUB_HALF);
        assert_eq!(gap.end - gap.start, BREAK_GAP);
    }
    #[test]
    fn a_meter_to_meter_break_begins_at_the_output_meter() {
        let groups = [spanning(&[&[1, 3]])];
        let frame = geometry(&groups, 1000.0, 700.0, 0.0, 0.0);
        let source = frame
            .slots
            .iter()
            .find(|slot| slot.id == NodeId(1))
            .unwrap();
        let gaps = wire_gaps(&groups, &frame, |id, side| id == 3 && !side);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].start, wire_box_x(source.x, true) + STUB_HALF);
        assert_eq!(gaps[0].end - gaps[0].start, BREAK_GAP);
    }
    #[test]
    fn no_two_wires_ask_for_the_same_click() {
        // Stages in series, branches in parallel, chains within a branch:
        // every shape the draw pass can meet, walked for overlapping claims.
        let shapes: &[&[&[usize]]] = &[
            &[&[1]],
            &[&[1], &[1]],
            &[&[1], &[1], &[1]],
            &[&[1, 1]],
            &[&[1, 1], &[1]],
            &[&[1], &[1, 1]],
            &[&[1], &[1, 1], &[1]],
            &[&[1, 1], &[1, 1]],
            &[&[3]],
            &[&[2], &[1]],
            &[&[1], &[2, 1]],
            &[&[1, 2], &[1]],
            &[&[2, 3]],
            &[&[1], &[2, 1, 2], &[1]],
        ];
        for stages in shapes {
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
        let stages: &[&[usize]] = &[&[1], &[1, 1], &[1]];
        let targets = wire_targets(stages);
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
        // And the trunk that leaves the merge is one wire again — the
        // merged signal going into one card — so it is that card's input
        // to break. It starts where the collar's clearance ends, so the
        // claim is sought past the rim, and the rim's own air belongs to
        // no wire.
        assert!(
            !claimed(merge + RING_CLEAR / 2.0),
            "a wire crosses the merge collar's air"
        );
        assert!(
            claimed((merge + RING_CLEAR + wire_box_x(card_x(2), false)) / 2.0),
            "the run out of a merge is nobody's"
        );
    }

    #[test]
    fn a_chain_in_a_lone_branch_is_exactly_the_old_serial_stages() {
        // Chained cards stand, and hand their signal on, exactly where the
        // same cards dropped as stage after stage had them: a chain along a
        // lone branch is the old left-to-right run, not a new geometry.
        fn assert_drawn_alike(chained: &[&[usize]], serial: &[&[usize]]) {
            let chained = wire_targets(chained);
            let serial = wire_targets(serial);
            assert_eq!(chained.len(), serial.len());
            for (chained, serial) in chained.iter().zip(&serial) {
                assert_eq!(chained.card, serial.card);
                assert_eq!(chained.side, serial.side);
                assert_eq!((chained.x, chained.y), (serial.x, serial.y));
                assert_eq!(
                    (chained.from, chained.to),
                    (serial.from, serial.to),
                    "card {} {} moved between the chain and the stages",
                    chained.card,
                    chained.side,
                );
            }
        }
        assert_drawn_alike(&[&[2]], &[&[1], &[1]]);
        assert_drawn_alike(&[&[3]], &[&[1], &[1], &[1]]);
    }

    use crate::routing::Channels;
    use std::collections::BTreeMap;
    /// The layout the draw pass gives a rack in a 496 × 580 canvas: the rack's
    /// own `IN` card where [`in_x`] says it stands, the first stage one column
    /// right of it, and the line signals flow at 30 below the middle.
    fn frame(groups: &[Vec<GuiLane>]) -> Geometry {
        geometry(groups, 496.0, 580.0, 0.0, 0.0)
    }
    /// The stages carried along beside the layout, the way the draw pass
    /// keeps the lanes it laid out.
    fn flatten(groups: &[Vec<GuiLane>]) -> Vec<Vec<GuiLane>> {
        groups.to_vec()
    }
    /// A layout and the stages it was laid out from, which is everything
    /// [`width_marks`] asks for.
    fn placed(groups: &[Vec<GuiLane>]) -> (Geometry, Vec<Vec<GuiLane>>) {
        (frame(groups), flatten(groups))
    }
    /// One width conversion, as the engine wired it. A `to` of no card is the
    /// hand-off to the rack's `OUT` card; a `to` of an id no stage holds is a
    /// wire landing on a join ring, which is not a card and so has no place in
    /// the layout.
    fn conversion(
        id: u64,
        inputs: Channels,
        outputs: Channels,
        from: Option<u64>,
        to: Option<u64>,
    ) -> Junction {
        Junction {
            id: NodeId(id),
            inputs,
            outputs,
            from: from.map(NodeId),
            to: to.map(NodeId),
        }
    }
    /// A circle is only drawn where it was told it fits: its middle inside
    /// the stretch of wire its seat was offered, and the stretch not empty.
    fn fits_on_its_wire(mark: &WidthMark) -> bool {
        let [x, y] = mark.centre(&BTreeMap::new());
        let along = if mark.vertical { y } else { x };
        mark.lo <= mark.hi && along >= mark.lo && along <= mark.hi
    }
    #[test]
    fn a_split_at_the_head_of_the_chain_is_seated_on_the_wire_after_the_input_card() {
        // A mono device feeding a stereo card is mapped at the head of the
        // chain, and the only wire that conversion has to stand on is the
        // short one between the rack's own `IN` card and the card. The `IN`
        // card carries no stub box and is treated as though it did, so its
        // wire ends 103 px from the card's centre — exactly where the card's
        // own box faces it back. 42 px of drawn wire, 26 px more than a
        // circle and its click area need, which is why it sits dead centre.
        let (frame, stages) = placed(&[spanning(&[&[1]])]);
        let marks = width_marks(
            &[conversion(
                9,
                Channels::Mono,
                Channels::Stereo,
                None,
                Some(1),
            )],
            &stages,
            &frame,
        );
        assert_eq!(marks.len(), 1, "{marks:?}");
        let mark = &marks[0];
        assert_eq!((mark.inputs, mark.outputs), (1, 2));
        assert!(!mark.vertical, "this wire runs across the canvas");
        assert_eq!(mark.forward, 1.0, "the signal runs into the card");
        assert_eq!(mark.cross, frame.flow, "the card stands on the line");
        assert_eq!(mark.centre(&BTreeMap::new()), [169.0, 30.0]);
        assert!(fits_on_its_wire(mark));
    }
    #[test]
    fn a_split_before_a_card_hanging_off_the_flow_line_is_seated_on_its_riser() {
        // A branch that hangs off the line signals flow at is fed by a riser
        // out of the trunk, and the conversion belongs on the riser: the
        // column it drops out of carries no card at this height, so nothing
        // crowds it. Its pins point along the riser, the way the signal does.
        let (frame, stages) = placed(&[spanning(&[&[1]]), spanning(&[&[3], &[5]])]);
        let marks = width_marks(
            &[conversion(
                9,
                Channels::Mono,
                Channels::Stereo,
                Some(1),
                Some(5),
            )],
            &stages,
            &frame,
        );
        assert_eq!(marks.len(), 1, "{marks:?}");
        let mark = &marks[0];
        assert!(mark.vertical);
        assert_eq!(
            mark.cross,
            junction_x(card_x(1), false),
            "the column it drops out of"
        );
        assert_eq!(
            mark.forward, -1.0,
            "this branch hangs below the line, so the signal runs down it"
        );
        // Down the riser from the line at 30 to the card's row at −40, shrunk
        // by the click area at both ends: a seat of −32 to 22, on −5.
        assert_eq!(mark.centre(&BTreeMap::new()), [mark.cross, -5.0]);
        assert_eq!(
            mark.along([7.0, 99.0]),
            99.0,
            "a drag reads the coordinate running along the wire"
        );
        assert!(fits_on_its_wire(mark));
    }
    #[test]
    fn a_branch_lifted_into_the_join_ring_is_seated_on_its_riser() {
        // Before the ring gathers the branches, one that arrives narrower than
        // the others is lifted to their width on a riser of its own, in the
        // column the ring stands in. The signal travels that riser back down
        // to the ring, whichever side of the line the branch hangs on.
        let (frame, stages) = placed(&[spanning(&[&[1]]), spanning(&[&[3], &[5]])]);
        let marks = width_marks(
            &[conversion(
                9,
                Channels::Mono,
                Channels::Stereo,
                Some(3),
                Some(4096),
            )],
            &stages,
            &frame,
        );
        assert_eq!(marks.len(), 1, "{marks:?}");
        let mark = &marks[0];
        assert!(mark.vertical);
        assert_eq!(
            mark.cross,
            junction_x(card_x(1), true),
            "the column the ring stands in"
        );
        assert_eq!(mark.forward, -1.0, "the signal runs back down to the ring");
        // Up the riser from the card's row at 100 to the edge of the ring just
        // off the line, at 30 + 8.5 + 1: a seat of 47.5 to 92.
        assert_eq!(mark.centre(&BTreeMap::new()), [mark.cross, 69.75]);
        assert!(fits_on_its_wire(mark));
    }
    #[test]
    fn a_join_onto_the_master_wire_beside_the_join_ring_gets_the_gutter_after_the_ring() {
        // The ring hands the joined trunk to the `OUT` card, and the gutter
        // between them is one column step of 288, less the 125 to the ring's
        // column, less 8.5 of ring, less the 103 to the face of the card's
        // stub box — 51.5. A circle and its click ring fit with 35.5 of slack,
        // centred on the stretch: it stands clear of both the ring and the
        // stub box it runs to.
        let (frame, stages) = placed(&[spanning(&[&[1], &[3]])]);
        let marks = width_marks(
            &[conversion(
                9,
                Channels::Stereo,
                Channels::Mono,
                Some(3),
                None,
            )],
            &stages,
            &frame,
        );
        assert_eq!(marks.len(), 1, "{marks:?}");
        let mark = &marks[0];
        assert_eq!((mark.inputs, mark.outputs), (2, 1));
        assert_eq!(mark.hi - mark.lo, 35.5, "51.5 of wire less 16 of circle");
        assert_eq!(mark.centre(&BTreeMap::new()), [card_x(0) + 159.25, 30.0]);
        assert!(fits_on_its_wire(mark));
    }
    #[test]
    fn a_join_onto_the_master_wire_gets_the_gutter_before_the_output_card() {
        // The same join after a stage running no branches has a whole gutter
        // to itself: 288, less the 125 to the merge column, less the 103 to
        // the face of the card's stub box, is 60 — a circle rides it centred,
        // with 22 of wire showing on either side of its click ring.
        let (frame, stages) = placed(&[spanning(&[&[1]])]);
        let marks = width_marks(
            &[conversion(
                9,
                Channels::Stereo,
                Channels::Mono,
                Some(1),
                None,
            )],
            &stages,
            &frame,
        );
        assert_eq!(marks.len(), 1, "{marks:?}");
        let mark = &marks[0];
        assert_eq!((mark.inputs, mark.outputs), (2, 1));
        assert_eq!(mark.hi - mark.lo, 44.0, "60 of wire less 16 of circle");
        assert_eq!(mark.centre(&BTreeMap::new()), [card_x(0) + 155.0, 30.0]);
        assert!(fits_on_its_wire(mark));
    }
    #[test]
    fn an_empty_rack_still_shows_the_box_it_needs_to_play_back() {
        // Mono in, stereo out, nothing in between: the engine still maps one
        // channel to two, and the circle stands in the open wire the two
        // endpoint cards share — the only wire there is to stand on.
        let (frame, stages) = placed(&[]);
        let marks = width_marks(
            &[conversion(9, Channels::Mono, Channels::Stereo, None, None)],
            &stages,
            &frame,
        );
        assert_eq!(marks.len(), 1, "{marks:?}");
        let mark = &marks[0];
        assert_eq!((mark.inputs, mark.outputs), (1, 2));
        assert_eq!(mark.centre(&BTreeMap::new()), [in_x() + 142.0, 30.0]);
        assert!(fits_on_its_wire(mark));
    }
    #[test]
    fn a_branch_already_on_the_flow_line_lets_the_join_ring_speak_for_it() {
        // The middle branch of a three-deep stage lies exactly on the line
        // signals flow at, so the riser that would lift it to the ring has no
        // length at all — and the 2.5 of wire its output run does have is
        // drawn under the ring. No seat, so no circle.
        let groups = [spanning(&[&[1]]), spanning(&[&[3], &[4], &[5]])];
        let frame = geometry(&groups, 496.0, 700.0, 0.0, 0.0);
        let stages = flatten(&groups);
        assert_eq!(
            stages[1][1].ids[0],
            NodeId(4),
            "the middle row is the one on the line"
        );
        let marks = width_marks(
            &[conversion(
                9,
                Channels::Mono,
                Channels::Stereo,
                Some(4),
                Some(4096),
            )],
            &stages,
            &frame,
        );
        assert!(marks.is_empty(), "{marks:?}");
    }
    #[test]
    fn centre_branch_keeps_both_meter_targets_clear_of_the_junctions() {
        // Echo -> Dist -> Wah above Ecomplex above Tuner: the middle
        // card must retain its input AND its long output lead.
        let targets = wire_targets(&[&[3, 1, 1]]);
        let centre: Vec<_> = targets.iter().filter(|target| target.card == 3).collect();
        assert_eq!(centre.len(), 2);
        for target in centre {
            assert_eq!(target.y, 0.0);
            let stub = wire_box_x(target.x, target.side);
            assert!(stub - STUB_HALF > target.from.min(target.to));
            assert!(stub + STUB_HALF < target.from.max(target.to));
        }
        // Even an expanded meter's grab/peak line (half-width 4.5)
        // must stand clear of the ring, including on the flow line.
        let x = card_x(0);
        let gap = junction_x(x, true) - wire_box_x(x, true);
        assert!(gap > MIXER_RING + 4.5 + BREAK_GAP);
        assert!(junction_x(card_x(1), false) - junction_x(x, true) > 2.0 * MIXER_RING);
    }
    #[test]
    fn three_parallel_branches_are_each_seated_where_their_own_wire_runs() {
        // Three mono cards in parallel on a mono device: the engine splits the
        // trunk for each branch and lifts each branch again before the ring.
        // The two outer branches hang off the line and ride their risers, on
        // the split side and the merge side of the stage; the middle one is on
        // the line, so its split is seated in the gutter the trunk drops out
        // of, and its lift belongs to the ring.
        let groups = [spanning(&[&[1], &[2], &[3]])];
        let frame = geometry(&groups, 496.0, 700.0, 0.0, 0.0);
        let stages = flatten(&groups);
        let marks = width_marks(
            &[
                conversion(11, Channels::Mono, Channels::Stereo, None, Some(1)),
                conversion(12, Channels::Mono, Channels::Stereo, None, Some(2)),
                conversion(13, Channels::Mono, Channels::Stereo, None, Some(3)),
                conversion(14, Channels::Mono, Channels::Stereo, Some(1), Some(4096)),
                conversion(15, Channels::Mono, Channels::Stereo, Some(2), Some(4096)),
                conversion(16, Channels::Mono, Channels::Stereo, Some(3), Some(4096)),
            ],
            &stages,
            &frame,
        );
        for mark in &marks {
            assert!(fits_on_its_wire(mark), "{mark:?}");
        }
        // Five, because the branch on the line has no riser of its own.
        assert_eq!(marks.len(), 5, "{marks:?}");
        assert!(marks.iter().all(|mark| mark.id != NodeId(15)), "{marks:?}");
        let seated = |id: u64| {
            marks
                .iter()
                .find(|mark| mark.id == NodeId(id))
                .unwrap_or_else(|| panic!("{id}: {marks:?}"))
        };
        let split = junction_x(card_x(0), false);
        let merge = junction_x(card_x(0), true);
        for (id, forward) in [(11, 1.0), (13, -1.0)] {
            let mark = seated(id);
            assert!(mark.vertical, "{id} rides a riser");
            assert_eq!(mark.cross, split, "{id} rides the stage's split column");
            assert_eq!(mark.forward, forward, "{id} points with the signal");
        }
        for (id, forward) in [(14, -1.0), (16, 1.0)] {
            let mark = seated(id);
            assert!(mark.vertical, "{id} rides a riser");
            assert_eq!(mark.cross, merge, "{id} rides the ring's own column");
            assert_eq!(mark.forward, forward, "{id} points back at the ring");
        }
        let middle = seated(12);
        assert!(!middle.vertical, "the line runs across the canvas");
        assert_eq!(middle.cross, frame.flow);
        assert_eq!(
            middle.centre(&BTreeMap::new()),
            [in_x() + 133.0, frame.flow],
            "in the gutter, clear of both stub boxes and the split column"
        );
        // And no two circles are drawn on top of one another: several ride the
        // same column, but each a different row of its riser.
        for (left, right) in marks
            .iter()
            .enumerate()
            .flat_map(|(index, left)| marks[index + 1..].iter().map(move |right| (left, right)))
        {
            let [left_x, left_y] = left.centre(&BTreeMap::new());
            let [right_x, right_y] = right.centre(&BTreeMap::new());
            let apart = (left_x - right_x).abs() > 2.0 * MARK_RADIUS
                || (left_y - right_y).abs() > 2.0 * MARK_RADIUS;
            assert!(apart, "{left:?} and {right:?} overlap");
        }
    }
    #[test]
    fn dragging_a_circle_parks_it_on_its_wire_and_never_past_it() {
        // The offset a drag leaves behind is remembered per conversion and
        // clamped to the stretch its seat was offered, so a circle pulled off
        // the end of its wire stays on the wire — and the conversion itself
        // never moved, only its mark.
        let (frame, stages) = placed(&[spanning(&[&[1]])]);
        let marks = width_marks(
            &[conversion(
                9,
                Channels::Mono,
                Channels::Stereo,
                None,
                Some(1),
            )],
            &stages,
            &frame,
        );
        let mark = marks.first().expect("the split has a seat").to_owned();
        let mut offsets = BTreeMap::new();
        offsets.insert(mark.id, -500.0);
        assert_eq!(mark.centre(&offsets), [mark.lo, mark.cross]);
        offsets.insert(mark.id, 500.0);
        assert_eq!(mark.centre(&offsets), [mark.hi, mark.cross]);
        // 42 of wire is 26 of slack, thirteen each way.
        offsets.insert(mark.id, 1.0);
        assert_eq!(mark.centre(&offsets), [170.0, 30.0]);
        offsets.insert(mark.id, -1.0);
        assert_eq!(mark.centre(&offsets), [168.0, 30.0]);
        // A conversion that went away takes its offset with it: nothing here
        // moves because of an id another graph handed out.
        let other = conversion(10, Channels::Mono, Channels::Stereo, None, Some(1));
        let moved = width_marks(&[other], &stages, &frame);
        assert_eq!(
            moved.first().map(|mark| mark.centre(&offsets)),
            Some([169.0, 30.0]),
            "a stranger's nudge moves nothing"
        );
    }
    /// A circle already seated, parked wherever its stretch puts it: the
    /// tests below are about the wire's lanes, not about finding the seat.
    fn circle(
        id: u64,
        inputs: usize,
        outputs: usize,
        cross: f32,
        lo: f32,
        hi: f32,
        vertical: bool,
    ) -> WidthMark {
        WidthMark {
            id: NodeId(id),
            inputs,
            outputs,
            cross,
            lo,
            hi,
            forward: 1.0,
            vertical,
        }
    }
    #[test]
    fn a_wire_steps_its_lanes_at_the_circle_standing_on_it() {
        // The rack's master wire out of a mono chain into a stereo device:
        // the operator asked to see a mono wire, a split, and a stereo wire
        // to the sink — so the wire carries one lane up to the circle and
        // two past it, and never two lanes that were stereo before they
        // were made stereo.
        let mark = circle(9, 1, 2, 30.0, 100.0, 200.0, false);
        assert_eq!(
            width_runs(0.0, 300.0, 2, 30.0, &[mark], &BTreeMap::new()),
            vec![(0.0, 150.0, 1), (150.0, 300.0, 2)],
            "one lane in, two out, cut at the circle's centre"
        );
        // The widths belong to the circle, not to the caller: asked for one
        // lane the whole way, the stretch past it still carries two.
        assert_eq!(
            width_runs(0.0, 300.0, 1, 30.0, &[mark], &BTreeMap::new()),
            vec![(0.0, 150.0, 1), (150.0, 300.0, 2)]
        );
    }
    #[test]
    fn only_a_circle_drawn_on_the_stretch_itself_cuts_it() {
        let mark = circle(9, 1, 2, 30.0, 100.0, 200.0, false);
        // Same height, but the stretch ends before the circle: nothing cuts.
        assert_eq!(
            width_runs(0.0, 120.0, 2, 30.0, &[mark], &BTreeMap::new()),
            vec![(0.0, 120.0, 2)]
        );
        // Same stretch, another row: the circle rides a different wire.
        assert_eq!(
            width_runs(0.0, 300.0, 2, 40.0, &[mark], &BTreeMap::new()),
            vec![(0.0, 300.0, 2)]
        );
        // A circle on a riser cuts nothing here: a column's lanes are the
        // riser's own affair.
        let riser = circle(9, 1, 2, 150.0, 0.0, 300.0, true);
        assert_eq!(
            width_runs(0.0, 300.0, 2, 30.0, &[riser], &BTreeMap::new()),
            vec![(0.0, 300.0, 2)]
        );
        // And a circle dragged along its wire cuts where it now stands.
        let mut offsets = BTreeMap::new();
        offsets.insert(mark.id, -20.0);
        assert_eq!(
            width_runs(0.0, 300.0, 2, 30.0, &[mark], &offsets),
            vec![(0.0, 130.0, 1), (130.0, 300.0, 2)]
        );
    }
    #[test]
    fn two_circles_on_one_wire_step_its_lanes_from_one_to_the_next() {
        // A wire lifted to stereo and folded back to mono on the same run
        // carries three widths, and each belongs between its own pair of
        // circles: the second takes in what the first made.
        let wide = circle(9, 1, 2, 30.0, 100.0, 200.0, false);
        let narrow = circle(8, 2, 1, 30.0, 210.0, 310.0, false);
        assert_eq!(
            width_runs(0.0, 400.0, 1, 30.0, &[narrow, wide], &BTreeMap::new()),
            vec![(0.0, 150.0, 1), (150.0, 260.0, 2), (260.0, 400.0, 1)],
            "the cuts follow the circles left to right, whatever order the marks came in"
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

#[cfg(test)]
mod gain_double_click_tests {
    use super::GainClick;
    use std::time::{Duration, Instant};
    #[test]
    fn only_two_quick_presses_on_the_same_value_reset_and_a_third_starts_over() {
        let mut clicks = GainClick::default();
        let now = Instant::now();
        let target = Some((7, false));
        assert_eq!(clicks.press(target, now), None);
        assert_eq!(
            clicks.press(target, now + Duration::from_millis(150)),
            target
        );
        assert_eq!(clicks.press(target, now + Duration::from_millis(220)), None);
        assert_eq!(
            clicks.press(Some((7, true)), now + Duration::from_millis(250)),
            None
        );
        assert_eq!(
            clicks.press(Some((9, true)), now + Duration::from_millis(280)),
            None
        );
        assert_eq!(
            clicks.press(Some((9, true)), now + Duration::from_millis(700)),
            None
        );
        assert_eq!(clicks.press(None, now + Duration::from_millis(710)), None);
        assert_eq!(
            clicks.press(Some((9, true)), now + Duration::from_millis(720)),
            None
        );
        assert_eq!(
            clicks.press(Some((9, true)), now + Duration::from_millis(800)),
            Some((9, true))
        );
    }
}
