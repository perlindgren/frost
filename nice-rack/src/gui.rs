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
    open: bool,
}
struct RackUi {
    runtime: Runtime,
    loader: Loader,
    font: Arc<[u8]>,
    icons: Arc<[u8]>,
    settings: Option<AudioPanel<crate::RackPreferences>>,
    settings_page: bool,
    pressed: bool,
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
fn hit(mouse: Option<[f32; 2]>, x: f32, y: f32, w: f32, h: f32) -> bool {
    mouse.is_some_and(|p| (p[0] - x).abs() <= w && (p[1] - y).abs() <= h)
}
fn wire_box_x(x: f32, side: bool) -> f32 {
    // Stub box centres for one card: 6 px out from each vertical card edge,
    // on the input wire for `side == false` and the output wire otherwise.
    let edge = CARD_WIDTH / 2.0 + 6.0;
    if side { x + edge } else { x - edge }
}
const TRACK_HALF: f32 = 56.0;
/// Relative drag mapping: each pixel of pointer travel turns the trim by
/// 0.05 dB, and that rate grows exponentially with the drag's speed (px
/// per frame), so a slow, careful drag keeps fractions-of-a-dB control
/// while a fast flick sweeps the range in a swipe. The result clamps to
/// the ±24 dB the track spans.
fn gain_drag_delta(px: f32, gain_db: f32) -> f32 {
    (gain_db + px * 0.05 * (px.abs() / 8.0).exp()).clamp(-24.0, 24.0)
}
/// Parse the editor's draft: a comma stands in for the decimal point,
/// and anything that is not a plain, finite dB value is no value at all.
fn parse_gain(text: &str) -> Option<f32> {
    text.trim()
        .replace(',', ".")
        .parse::<f32>()
        .ok()
        .filter(|db| db.is_finite())
        .map(|db| db.clamp(-24.0, 24.0))
}
/// The track height a gain dB sits at — the inverse mapping the white
/// gain line is drawn from.
fn gain_line_y(gain_db: f32, wire_y: f32) -> f32 {
    wire_y - TRACK_HALF + 2.0 * TRACK_HALF * ((gain_db + 24.0) / 48.0).clamp(0.0, 1.0)
}
/// The trim a wire shows *before* its meter exists: the engine's own
/// linear factor in dB, over the ±24 dB the track spans, so the number
/// in the card's corner is the gain the wire is actually applying and
/// not a placeholder. A key the slot table does not hold is unity, and
/// unity reads `+0.0`.
fn engine_gain_db(linear: Option<f32>) -> f32 {
    linear.map_or(0.0, |gain| (20.0f32 * gain.log10()).clamp(-24.0, 24.0))
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
/// A meter for one wire, born collapsed. It opens at whatever trim the
/// engine already applies to that wire, so the white line stands where the
/// gain actually is, and it is built latching: a clip it catches stays on the
/// stub until somebody clicks it away, which is the point of a meter nobody
/// is watching.
fn new_wire_meter(gain_linear: Option<f32>, time: u64) -> WireMeter {
    WireMeter {
        peak: PeakMeter::latching(time),
        gain_db: engine_gain_db(gain_linear),
        open: false,
    }
}
/// Put a meter on every wire the rack carries, keyed the way the engine keys
/// its slots. Meters are *born* here and not at the first click on a stub:
/// peak detection may not wait for someone to open a track, because a stub
/// with no meter behind it has nothing to colour itself with and reads green
/// through any amount of clipping.
///
/// An entry that is already there is left exactly where the operator left it
/// — the trim they set, whether the track is expanded, and a clip they have
/// not acknowledged — so this is called every frame and changes nothing
/// except for the wires that are new.
fn watch_wires(
    meters: &mut std::collections::BTreeMap<(u64, bool), WireMeter>,
    ids: impl IntoIterator<Item = u64>,
    mut gain_of: impl FnMut(u64) -> Option<f32>,
    time: u64,
) {
    for id in ids {
        for side in [false, true] {
            let key = (id, side);
            if meters.contains_key(&key) {
                continue;
            }
            let gain_linear = gain_of(id << 1 | u64::from(side));
            meters.insert(key, new_wire_meter(gain_linear, time));
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
    /// which is what the white line draws and the tick loop re-states, and the
    /// engine's slot table, which is what the audio path actually applies. A
    /// card that is no longer on the canvas has no meter to write, so both are
    /// written on one call and neither is trusted alone, because the readout
    /// shows a single value and this is what makes it true. The dB→linear
    /// conversion is the one the tick loop uses, so the two paths can never
    /// drift apart.
    fn set_trim(&mut self, id: u64, side: bool, gain_db: f32) {
        if let Some(wire) = self.wire_meters.get_mut(&(id, side)) {
            wire.gain_db = gain_db;
        }
        self.runtime
            .wires
            .set_gain(id << 1 | u64::from(side), 10f32.powf(gain_db / 20.0));
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
        // drawn, so a clip shows up on a stub nobody has ever clicked.
        watch_wires(
            &mut self.wire_meters,
            self.runtime.cards.iter().map(|card| card.id.0),
            |key| self.runtime.wires.gain_of(key),
            now(),
        );
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
            let runtime = &mut self.runtime;
            let event = panel.process(
                ctx,
                &self.font,
                clicked,
                Vec::new(),
                Some(nice_common::WindowGeom {
                    x: None,
                    y: None,
                    width: w,
                    height: h,
                }),
                !runtime.busy(),
                |settings| runtime.apply(settings),
            );
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
        let mut previous_x = layout.left + 25.0;
        for group in &groups {
            let stage_slots = layout
                .slots
                .iter()
                .filter(|slot| group.contains(&slot.id))
                .collect::<Vec<_>>();
            let x = stage_slots[0].x;
            let split = x - CARD_WIDTH / 2.0 - 12.0;
            let merge = x + CARD_WIDTH / 2.0 + 12.0;
            ctx.line(previous_x, layout.flow, split, layout.flow, wire, 2.0, 0.1);
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
            if group.len() > 1 {
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
            for slot in stage_slots {
                if visible(slot, layout.top, layout.bottom, w) {
                    ctx.line(split, slot.y, x - CARD_WIDTH / 2.0, slot.y, wire, 2.0, 0.1);
                    ctx.line(x + CARD_WIDTH / 2.0, slot.y, merge, slot.y, wire, 2.0, 0.1);
                    for side in [false, true] {
                        let bx = wire_box_x(x, side);
                        let key = (slot.id.0, side);
                        if clicked && self.choosing.is_none() && hit(mouse, bx, slot.y, 4.0, 6.0) {
                            // Toggle. The meter has been running since the
                            // card appeared, so this only folds the track
                            // away or brings it back and the peak-hold runs
                            // straight through the collapse. The insert arm
                            // is a fallback so a click can never be
                            // swallowed, not the normal way in.
                            let meter = self.wire_meters.entry(key).or_insert_with(|| {
                                new_wire_meter(
                                    self.runtime.wires.gain_of(slot.id.0 << 1 | u64::from(side)),
                                    now(),
                                )
                            });
                            meter.open = !meter.open;
                            // A click on the meter body is also the rack's
                            // "yes, I saw it": a red meter here, and every
                            // meter the clipped signal reached after it, go
                            // back to their resting colours.
                            self.acknowledge_clip(MeterPos::Wire(slot.id.0, side));
                        }
                        // The stub says "meter on this wire" in green while
                        // the track is expanded, and in red when that meter
                        // reads above full scale — the whole meter carries the
                        // clip, stub included.
                        let stub = self.wire_meters.get(&key);
                        let stub_color = if stub.is_some_and(|meter| meter.peak.clipping()) {
                            nice_common::knob::RED
                        } else if stub.is_some_and(|meter| meter.open) {
                            rgb(103, 237, 175)
                        } else {
                            wire
                        };
                        ctx.rectangle(bx, slot.y, 4.0, 6.0, stub_color, 0.25);
                    }
                }
            }
            previous_x = merge;
        }
        let out_x = if groups.is_empty() {
            layout.left + 130.0
        } else {
            previous_x + 60.0
        };
        ctx.line(
            previous_x,
            layout.flow,
            out_x - 25.0,
            layout.flow,
            wire,
            2.0,
            0.1,
        );
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
            ctx.rectangle(
                x,
                y,
                CARD_WIDTH / 2.0,
                56.0,
                if self.drag == Some(card.id) {
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
                rgb(235, 230, 241),
                0.21,
            );
            ctx.text(
                x,
                layout_text_y(y),
                &self.font,
                format!("{:?} → {:?}", card.inputs, card.outputs),
                12.0,
                500.0,
                rgb(150, 145, 164),
                0.21,
            );
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
            } else if let Some(wire) = self.wire_meters.get_mut(&(id, side))
                && let Some([_, my]) = ctx.mouse_position()
            {
                wire.gain_db = gain_drag_delta(my - last_y, wire.gain_db);
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
            let (rms, peak) = self.runtime.wires.sample(key).unwrap_or((0.0, 0.0));
            wire.peak.tick(db(rms), db(peak), time);
        }
        for slot in &layout.slots {
            if !visible(slot, layout.top, layout.bottom, w) {
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
                    && self.choosing.is_none()
                    && self.gain_drag.is_none()
                    && self.gain_edit.is_none()
                    && let Some([_, my]) = mouse
                    && hit(mouse, bx, slot.y, 4.5, TRACK_HALF)
                    && !hit(mouse, bx, slot.y, 4.0, 6.0)
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
                        if clip {
                            nice_common::knob::RED
                        } else {
                            rgb(103, 237, 175)
                        },
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
        let fit = (w / 1000.0).clamp(0.45, 1.0);
        let y = -h / 2.0 + 58.0;
        let over_input = self.input.draw(
            ctx,
            &self.font,
            [-w / 4.0, y],
            w * 0.20,
            fit,
            rgb(103, 237, 175),
        );
        let over_output = self.output.draw(
            ctx,
            &self.font,
            [w / 4.0, y],
            w * 0.20,
            fit,
            rgb(103, 237, 175),
        );
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
        };
        let collapsed = WireMeter {
            peak: PeakMeter::new(0),
            gain_db: -6.5,
            open: false,
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
        watch_wires(&mut meters, [1, 3], |_| None, 0);
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
        watch_wires(&mut meters, [4], |key| (key == 9).then_some(0.5), 0);
        assert_eq!(meters[&(4, false)].gain_db, 0.0, "an untrimmed wire");
        assert!(
            (meters[&(4, true)].gain_db + 6.0206).abs() < 0.01,
            "and a trimmed one reads its trim: {:?}",
            meters[&(4, true)].gain_db
        );
    }
    #[test]
    fn watching_a_wire_leaves_an_expanded_meter_where_it_was() {
        use std::collections::BTreeMap;
        // Called every frame, so it may only ever fill a gap: the trim the
        // operator set, the track they left open and a clip they have not
        // acknowledged are all theirs, not this pass's to reset.
        let mut meters: BTreeMap<(u64, bool), WireMeter> = BTreeMap::new();
        watch_wires(&mut meters, [2], |_| None, 0);
        let wire = meters.get_mut(&(2, true)).unwrap();
        wire.open = true;
        wire.gain_db = -12.0;
        wire.peak.tick(-18.0, 1.4, 16);
        watch_wires(&mut meters, [2], |_| Some(4.0), 1_000);
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
        watch_wires(&mut meters, [6], |_| None, 0);
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
}
