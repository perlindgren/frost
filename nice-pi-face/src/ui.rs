//! The Pi-Fase editor view.
//!
//! **This is the family's face, not the Bi-Phase's.** It gives every control a
//! place on screen and a readout, so the plugin, its parameters, the host's
//! automation and the standalone settings page can all be exercised. The filter it
//! drives is the real one — two lanes, twelve sections, two generators — and what
//! this layout cannot yet show is the sweep itself. The GUI phase in `TODO.md` owns
//! the lamps, the two lanes, and the moving element that reads
//! `PiFace::sweep_position`; the conventions it will use are already the ones in
//! force here: absolute coordinates scaled by a `fit` factor, hit tests run against
//! the same frame's geometry the drawing used, and a value read from the same place
//! the control is written, so the two cannot disagree.
//!
//! frost's window space has its origin at the window's centre and the y axis
//! pointing up, so hit tests compare against `Context::mouse_position` in the
//! same coordinates and nothing is ever negated.
use crate::{PiFaceParams, PiFaceSharedState};
use crossbeam::atomic::AtomicCell;
use frost::{Color, Context, Process, Scene, SceneNode, Shape};
use nice_common::knob::{self, AMBER, BORE, GREEN, MUTED, PANEL, TEXT};
use nice_common::meter::PeakMeter;
use nice_plug::context::gui::GuiContext;
use nice_plug::prelude::{FloatParam, IntParam, IntRange};
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[cfg(feature = "standalone")]
use crate::settings::{PiFacePreferences, Settings};
#[cfg(feature = "standalone")]
use nice_common::ui::{AudioPanel, Event};

/// The heaviest cut the embedded font has: `FiraCode-VariableFont_wght.ttf`
/// carries a single `wght` axis spanning 300..700, so 700 is as black as it
/// gets.
const TEXT_WEIGHT: f32 = 700.0;

/// How long the audio thread may have gone quiet before the meters are
/// considered dead: a suspended host would otherwise keep the last frame lit
/// forever.
const METER_LIVE_MS: u64 = 250;

/// The pointer travel that carries a dial from one end of its range to the
/// other, in logical pixels; Shift stretches it for fine moves. The family's
/// numbers rather than this window's: a drag should feel the same in any of
/// these plugins whatever it is dragging, and this file had drifted to 220 and
/// 880 by nothing more deliberate than my hand on the slider while tuning it.
const DIAL_TRAVEL: f32 = knob::TRAVEL;
const DIAL_TRAVEL_FINE: f32 = knob::TRAVEL_FINE;

/// The three groups across the window. The controls name the group they belong
/// to (`Control::group`), so this list is only the captions — a control cannot
/// be left out of the layout by forgetting to add it to a table.
const GROUPS: [&str; 3] = ["SWEEP GENERATORS", "PHASOR A", "PHASOR B"];

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

/// The window's ground — the one colour in this file `knob` does not hand out,
/// because the family paints its backgrounds its own way. `rgb` stays for this
/// and for the cog's resting tint, a hand-mixed third of the way from MUTED to
/// TEXT that no name in the palette covers.
const BG: Color = rgb(32, 28, 44);

/// The continuous controls. Each names itself, the parameter it owns and its
/// place in the layout, and nothing else about a dial lives here — value, range
/// and formatting come off the parameter itself, so a range change in `lib.rs`
/// cannot leave the view disagreeing with the DSP.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dial {
    Gen1Rate,
    Gen2Rate,
    ADepth,
    AFeedback,
    BDepth,
    BFeedback,
    Pedal,
}

impl Dial {
    const ALL: [Dial; 7] = [
        Dial::Gen1Rate,
        Dial::Gen2Rate,
        Dial::ADepth,
        Dial::AFeedback,
        Dial::BDepth,
        Dial::BFeedback,
        Dial::Pedal,
    ];

    fn label(self) -> &'static str {
        match self {
            Dial::Gen1Rate => "GEN 1 RATE",
            Dial::Gen2Rate => "GEN 2 RATE",
            Dial::ADepth => "A DEPTH",
            Dial::AFeedback => "A FEEDBACK",
            Dial::BDepth => "B DEPTH",
            Dial::BFeedback => "B FEEDBACK",
            Dial::Pedal => "PEDAL",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Dial::Gen1Rate => "Sweep generator 1 — .1 to 18 Hz, as the specification has it",
            Dial::Gen2Rate => "Sweep generator 2 — free to drift against generator 1",
            Dial::ADepth => "Phasor A depth — how far the sweep travels",
            Dial::AFeedback => "Phasor A feedback — what the all-pass chain returns",
            Dial::BDepth => "Phasor B depth",
            Dial::BFeedback => "Phasor B feedback",
            Dial::Pedal => "This plugin's port of the expression pedal input at J6-2",
        }
    }

    fn param<'a>(&self, params: &'a PiFaceParams) -> &'a FloatParam {
        match self {
            Dial::Gen1Rate => &params.gen1_rate,
            Dial::Gen2Rate => &params.gen2_rate,
            Dial::ADepth => &params.a_depth,
            Dial::AFeedback => &params.a_feedback,
            Dial::BDepth => &params.b_depth,
            Dial::BFeedback => &params.b_feedback,
            Dial::Pedal => &params.pedal,
        }
    }

    fn cell<'a>(&self, shared: &'a PiFaceSharedState) -> &'a AtomicCell<f32> {
        match self {
            Dial::Gen1Rate => &shared.gen1_rate,
            Dial::Gen2Rate => &shared.gen2_rate,
            Dial::ADepth => &shared.a_depth,
            Dial::AFeedback => &shared.a_feedback,
            Dial::BDepth => &shared.b_depth,
            Dial::BFeedback => &shared.b_feedback,
            Dial::Pedal => &shared.pedal,
        }
    }

    fn readout(self, value: f32) -> String {
        match self {
            Dial::Gen1Rate | Dial::Gen2Rate => format!("{value:.2} Hz"),
            _ => format!("{:.0}%", value * 100.0),
        }
    }
}

/// The switches — the complete set of what the original builds as physical
/// two- and three-way switching. The panel drawings in the manual show SHAPE,
/// RATE CONTROL and the two Sweep/Sync selectors as rockers, and PHASOR B SWEEP
/// and PHASOR B INPUT as three-position rotaries; a GUI that wants to look like
/// the machine starts from that. There is no Mode control in this list because
/// the machine has none: `BInput`'s three positions are its three basic modes of
/// operation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Switch {
    Gen1Shape,
    Gen1RateSource,
    Gen2Shape,
    ASweep,
    BSync,
    BSweep,
    BInput,
}

impl Switch {
    const ALL: [Switch; 7] = [
        Switch::Gen1Shape,
        Switch::Gen1RateSource,
        Switch::Gen2Shape,
        Switch::ASweep,
        Switch::BSync,
        Switch::BSweep,
        Switch::BInput,
    ];

    fn label(self) -> &'static str {
        match self {
            Switch::Gen1Shape => "GEN 1 SHAPE",
            Switch::Gen1RateSource => "GEN 1 RATE SRC",
            Switch::Gen2Shape => "GEN 2 SHAPE",
            Switch::ASweep => "A SWEEP",
            Switch::BSync => "B SYNC",
            Switch::BSweep => "B SWEEP",
            Switch::BInput => "B INPUT",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Switch::Gen1Shape => "S3 — the shape network on generator 1",
            Switch::Gen1RateSource => "S2 — rate from the knob, or from the pedal",
            Switch::Gen2Shape => "S4 — the shape network on generator 2",
            Switch::ASweep => "S5 — what sweeps phasor A",
            Switch::BSync => "S7 — reversed, phasor B moves against phasor A",
            Switch::BSweep => "S6 — what sweeps phasor B",
            Switch::BInput => {
                "S1 — the machine's three basic modes of operation: In A feeds both phasors (stereo), Out A puts them in series (twelve stages), In B gives phasor B its own input"
            }
        }
    }

    /// The names on the positions, in the order the parameter's integers run.
    fn positions(self) -> &'static [&'static str] {
        match self {
            Switch::Gen1Shape | Switch::Gen2Shape => &crate::SHAPES[..],
            Switch::Gen1RateSource => &crate::RATE_SOURCES[..],
            Switch::ASweep => &crate::A_SWEEPS[..],
            Switch::BSync => &crate::SYNC[..],
            Switch::BSweep => &crate::B_SWEEPS[..],
            Switch::BInput => &crate::B_INPUTS[..],
        }
    }

    fn param<'a>(&self, params: &'a PiFaceParams) -> &'a IntParam {
        match self {
            Switch::Gen1Shape => &params.gen1_shape,
            Switch::Gen1RateSource => &params.gen1_rate_source,
            Switch::Gen2Shape => &params.gen2_shape,
            Switch::ASweep => &params.a_sweep,
            Switch::BSync => &params.b_sync,
            Switch::BSweep => &params.b_sweep,
            Switch::BInput => &params.b_input,
        }
    }

    fn cell<'a>(&self, shared: &'a PiFaceSharedState) -> &'a AtomicCell<i32> {
        match self {
            Switch::Gen1Shape => &shared.gen1_shape,
            Switch::Gen1RateSource => &shared.gen1_rate_source,
            Switch::Gen2Shape => &shared.gen2_shape,
            Switch::ASweep => &shared.a_sweep,
            Switch::BSync => &shared.b_sync,
            Switch::BSweep => &shared.b_sweep,
            Switch::BInput => &shared.b_input,
        }
    }

    /// The first and last integer this switch's parameter accepts.
    fn bounds(self, params: &PiFaceParams) -> (i32, i32) {
        match self.param(params).range() {
            IntRange::Linear { min, max } => (min, max),
            IntRange::Reversed(range) => match *range {
                IntRange::Linear { min, max } => (min, max),
                IntRange::Reversed(inner) => match *inner {
                    IntRange::Linear { min, max } => (min, max),
                    IntRange::Reversed(_) => (0, 0),
                },
            },
        }
    }

    /// The name of the position a value is on, clamped because a hand-edited
    /// settings file may hold anything.
    fn readout(self, params: &PiFaceParams, value: i32) -> String {
        let (min, max) = self.bounds(params);
        let index = (value.clamp(min, max) - min) as usize;
        self.positions()
            .get(index)
            .copied()
            .unwrap_or("?")
            .to_string()
    }
}

/// What the pointer is on. A control of the view's own making: the cog is not a
/// parameter in this sense, so it is named beside them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    Dial(Dial),
    Switch(Switch),
    Cog,
}

/// Where a control sits, and what it is: the layout as data. Each control names
/// its own group and its own row within it, so the compiler — not a table that
/// can be forgotten — is what makes a new parameter appear on screen. `None`
/// means the control sits outside the three-column grid: the expression pedal,
/// which belongs to neither lane nor generator, gets a row of its own.
trait Placement {
    fn slot(&self) -> Option<(usize, usize)>;
}

impl Placement for Dial {
    fn slot(&self) -> Option<(usize, usize)> {
        match self {
            Dial::Gen1Rate => Some((0, 0)),
            Dial::Gen2Rate => Some((0, 1)),
            Dial::ADepth => Some((1, 0)),
            Dial::AFeedback => Some((1, 1)),
            Dial::BDepth => Some((2, 0)),
            Dial::BFeedback => Some((2, 1)),
            Dial::Pedal => None,
        }
    }
}

impl Placement for Switch {
    fn slot(&self) -> Option<(usize, usize)> {
        match self {
            Switch::Gen1Shape => Some((0, 2)),
            Switch::Gen2Shape => Some((0, 3)),
            Switch::Gen1RateSource => Some((0, 4)),
            Switch::ASweep => Some((1, 2)),
            Switch::BSweep => Some((2, 2)),
            Switch::BSync => Some((2, 3)),
            Switch::BInput => Some((2, 4)),
        }
    }
}

/// One control as it sits on screen this frame: where it is, what it says, and
/// what owns it. Drawing and hit testing read the same list, so what can be
/// clicked is what can be seen.
struct Row {
    label: &'static str,
    readout: String,
    x: f32,
    y: f32,
    half: f32,
    /// The fraction of the bar to fill: a dial's place in its range.
    fill: f32,
    /// A switch draws one segment per position instead of a fill; a dial has
    /// no positions.
    positions: Option<usize>,
    active: usize,
    hit: Hit,
}

#[derive(Clone, Copy)]
enum Drag {
    Dial(Dial),
}

pub struct PiFaceUi {
    shared: Arc<PiFaceSharedState>,
    icon_font: Arc<[u8]>,
    font: Arc<[u8]>,
    params: Option<Arc<PiFaceParams>>,
    context: Option<GuiContext>,
    input_meter: PeakMeter,
    output_meter: PeakMeter,
    was_down: bool,
    drag: Option<Drag>,
    /// The pointer coordinate and control value at the moment a drag grabbed:
    /// moves are deltas from there, so the value under the finger never jumps,
    /// however the control was last set.
    drag_anchor: f32,
    drag_start: f32,
    hovered: Option<Hit>,
    /// What the current press started on. A release over the same thing is a
    /// click; a release anywhere else was a drag that wandered off.
    armed: Option<Hit>,
    #[cfg(feature = "standalone")]
    settings_page: bool,
    #[cfg(feature = "standalone")]
    message: String,
    /// The settings page keeps its own button edge: the page is drawn (and the
    /// main view's bookkeeping skipped) before `was_down` is next read.
    #[cfg(feature = "standalone")]
    pressed: bool,
    #[cfg(feature = "standalone")]
    applied: Settings,
    #[cfg(feature = "standalone")]
    panel: Option<AudioPanel<PiFacePreferences>>,
}

impl PiFaceUi {
    pub fn new(shared: Arc<PiFaceSharedState>, font: Arc<[u8]>) -> Self {
        Self {
            shared,
            icon_font: crate::gui::icon_font(),
            font,
            params: None,
            context: None,
            input_meter: PeakMeter::new(crate::now_ms()),
            output_meter: PeakMeter::new(crate::now_ms()),
            was_down: false,
            drag: None,
            drag_anchor: 0.0,
            drag_start: 0.0,
            hovered: None,
            armed: None,
            #[cfg(feature = "standalone")]
            settings_page: false,
            #[cfg(feature = "standalone")]
            message: String::new(),
            #[cfg(feature = "standalone")]
            pressed: false,
            #[cfg(feature = "standalone")]
            applied: Settings::load().0,
            #[cfg(feature = "standalone")]
            panel: None,
        }
    }

    /// Receives the parameters to read their ranges and formats. Every number
    /// this view shows is read back every frame, so host automation moves the
    /// interface whether or not the user is touching it.
    pub fn set_params(&mut self, params: Arc<PiFaceParams>) {
        self.params = Some(params);
    }

    /// Receives the host's write door on top of the parameters. Writes go
    /// through it wherever it exists; where it does not, the shared cells are
    /// the truth and carry the writes instead.
    pub fn set_gui_context(&mut self, context: GuiContext) {
        self.context = Some(context);
    }

    fn text(&self, ctx: &mut Context, x: f32, y: f32, text: String, size: f32, color: Color) {
        ctx.text(x, y, &self.font, text, size, TEXT_WEIGHT, color, 0.5);
    }

    fn text_width(&self, ctx: &Context, text: &str, size: f32) -> f32 {
        ctx.text_width(&self.font, text, size, TEXT_WEIGHT)
    }

    /// Where a control's value is authoritative: the shared cell in standalone,
    /// where the view is the only writer and nice-plug gives a parameter no
    /// public setter; the host's parameter everywhere else, where automation
    /// writes it and a paused host would leave a mirrored cell stale.
    fn dial_value(&self, params: &PiFaceParams, dial: Dial) -> f32 {
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            dial.cell(&self.shared).load()
        } else {
            dial.param(params).value()
        }
    }

    fn switch_value(&self, params: &PiFaceParams, switch: Switch) -> i32 {
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            switch.cell(&self.shared).load()
        } else {
            switch.param(params).value()
        }
    }

    /// A dial write: clamp through the parameter's own range, ride the host's
    /// automation door when there is one, and always leave the cell saying the
    /// same thing — in a host the next block's mirror confirms it, in standalone
    /// the cell *is* the sound.
    fn set_dial(&mut self, params: &PiFaceParams, dial: Dial, value: f32) {
        let param = dial.param(params);
        let t = param.range().normalize(value).clamp(0.0, 1.0);
        let value = param.range().unnormalize(t);
        if let Some(context) = &self.context {
            context.param_setter().set_parameter(param, value);
        }
        dial.cell(&self.shared).store(value);
    }

    fn set_switch(&mut self, params: &PiFaceParams, switch: Switch, position: i32) {
        let param = switch.param(params);
        let (min, max) = switch.bounds(params);
        let position = position.clamp(min, max);
        if let Some(context) = &self.context {
            let setter = context.param_setter();
            setter.begin_set_parameter(param);
            setter.set_parameter(param, position);
            setter.end_set_parameter(param);
        }
        switch.cell(&self.shared).store(position);
        self.save_preferences();
    }

    /// Standalone persistence: what the player sets is what the next launch
    /// seeds from. The audio fields ride along untouched because this only ever
    /// reads and writes the flattened preferences half of the file.
    fn save_preferences(&mut self) {
        #[cfg(feature = "standalone")]
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            let shared = &self.shared;
            let mut settings = Settings::load().0;
            settings.gen1_rate = Some(shared.gen1_rate.load());
            settings.gen1_shape = Some(shared.gen1_shape.load());
            settings.gen1_rate_source = Some(shared.gen1_rate_source.load());
            settings.gen2_rate = Some(shared.gen2_rate.load());
            settings.gen2_shape = Some(shared.gen2_shape.load());
            settings.a_depth = Some(shared.a_depth.load());
            settings.a_feedback = Some(shared.a_feedback.load());
            settings.a_sweep = Some(shared.a_sweep.load());
            settings.b_depth = Some(shared.b_depth.load());
            settings.b_feedback = Some(shared.b_feedback.load());
            settings.b_sync = Some(shared.b_sync.load());
            settings.b_sweep = Some(shared.b_sweep.load());
            settings.b_input = Some(shared.b_input.load());
            settings.pedal = Some(shared.pedal.load());
            if let Err(error) = settings.save() {
                self.message = error;
            }
        }
    }

    /// The next position of a switch, wrapping — the way a thumb comes back
    /// round a rotary switch.
    fn next_position(params: &PiFaceParams, switch: Switch, current: i32) -> i32 {
        let (min, max) = switch.bounds(params);
        let span = max - min + 1;
        min + (current.clamp(min, max) - min + 1).rem_euclid(span)
    }

    /// This frame's rows, read from the controls' own slots: three groups across
    /// the window, each a caption and its controls. Built once and read by both
    /// the hit tests and the drawing, so the two cannot drift apart.
    fn rows(&self, params: &PiFaceParams, layout: &Layout) -> Vec<Row> {
        let mut rows: Vec<Row> = Vec::new();
        for dial in Dial::ALL {
            let Some((column, index)) = dial.slot() else {
                continue;
            };
            let value = self.dial_value(params, dial);
            let fill = dial.param(params).range().normalize(value).clamp(0.0, 1.0);
            rows.push(Row {
                label: dial.label(),
                readout: dial.readout(value),
                x: layout.column_x(column),
                y: layout.row_y(index),
                half: layout.bar_half,
                fill,
                positions: None,
                active: 0,
                hit: Hit::Dial(dial),
            });
        }
        for switch in Switch::ALL {
            let Some((column, index)) = switch.slot() else {
                continue;
            };
            let (min, max) = switch.bounds(params);
            let value = self.switch_value(params, switch).clamp(min, max);
            let span = (max - min).max(1) as f32;
            rows.push(Row {
                label: switch.label(),
                readout: switch.readout(params, value),
                x: layout.column_x(column),
                y: layout.row_y(index),
                half: layout.bar_half,
                fill: (value - min) as f32 / span,
                positions: Some((max - min + 1) as usize),
                active: (value - min) as usize,
                hit: Hit::Switch(switch),
            });
        }
        // Top row first, then left to right: the order the eye meets them, and
        // the order a tabbing pointer should follow.
        rows.sort_by(|a, b| {
            b.y.partial_cmp(&a.y)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
        });
        rows
    }

    /// The shared audio-settings page: the same panel nice-nam, nice-tuner,
    /// nice-wah and the rack run, with this app's preferences flattened into it.
    /// Apply travels through the control channel to the audio owner, which
    /// resolves, opens and reports — this page only draws and asks.
    #[cfg(feature = "standalone")]
    fn draw_settings_page(&mut self, ctx: &mut Context) {
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let clicked = down && !self.pressed;
        self.pressed = down;
        let mut panel = self.panel.take().unwrap_or_else(|| {
            let saved = Settings::load().0;
            let mut panel = AudioPanel::new(
                saved.clone(),
                self.applied.clone(),
                saved,
                !Settings::exists(),
            );
            panel.refresh();
            panel.note = (!self.message.is_empty()).then(|| self.message.clone());
            panel
        });
        panel.live_frames = Some(self.shared.buffer_frames.load(Ordering::Relaxed));
        let size = self.shared.logical_size();
        let window = nice_common::ui::window_geometry(
            [size.width, size.height],
            *self.shared.window_pos.lock().unwrap(),
            panel.draft.window,
        );
        // No extra rows. Every application control here is a plugin parameter,
        // and a parameter is set on the main view — where the host's automation
        // door is — and remembered in this same file. Giving the panel a second
        // door to one number is how a settings file ends up disagreeing with the
        // sound, which is why nice-nam's `extras` is empty too.
        let event = panel.process(
            ctx,
            &self.font,
            clicked,
            Vec::new(),
            window,
            crate::standalone::control::available(),
            crate::standalone::control::request,
        );
        self.applied = panel.applied.clone();
        if matches!(event, Some(Event::Back | Event::Applied)) {
            self.settings_page = false;
            self.message.clear();
        }
        self.panel = Some(panel);
    }
}

/// The frame's geometry, computed once so the drawing and the hit tests cannot
/// drift apart.
struct Layout {
    width: f32,
    fit: f32,
    y_head: f32,
    column_half: f32,
    bar_half: f32,
}

impl Layout {
    fn new(width: f32, height: f32) -> Self {
        let fit = (width / 900.0).clamp(0.45, 1.0);
        let span = width / 2.0 - 22.0 * fit;
        Self {
            width,
            fit,
            y_head: height / 2.0 - 150.0 * fit,
            column_half: span / GROUPS.len() as f32 - 10.0 * fit,
            bar_half: (span / GROUPS.len() as f32 - 10.0 * fit) * 0.42,
        }
    }

    fn column_x(&self, index: usize) -> f32 {
        let span = self.width / 2.0 - 22.0 * self.fit;
        (index as f32 + 0.5) / GROUPS.len() as f32 * 2.0 * span - span
    }

    fn row_y(&self, index: usize) -> f32 {
        self.y_head - (index as f32 + 1.0) * 33.0 * self.fit
    }

    /// Lay the meter row out from both borders inward.
    ///
    /// `in_label` and `out_label` are the measured widths of the two labels and
    /// `numeral` half the width of the widest numeral the meter paints — the
    /// widget centres a numeral on its tick, so the `-48` at one end and the `0`
    /// at the other reach half a numeral past the bar. A bar drawn to the border
    /// therefore hangs outside the window, and that is exactly what the output
    /// meter did: its end was 147 px past the right border at the design width,
    /// because its place had been accumulated from the centre as though the row
    /// began there instead of at the left border.
    fn meter_row(&self, in_label: f32, out_label: f32, numeral: f32) -> MeterRow {
        // The control columns' own gutter, so `IN` starts where the first
        // column starts.
        let edge = 22.0 * self.fit;
        let gap = 16.0 * self.fit;
        // What the row has always wanted: 250 px of bar at the 900 px design
        // size. A window too narrow for that, its labels and its numerals takes
        // the difference out of both bars, which is what keeps the right one on
        // screen instead of over the edge.
        let wanted = self.width / 4.0 - 100.0 * self.fit;
        let available =
            (self.width - 2.0 * edge - 3.0 * gap - in_label - out_label - 4.0 * numeral) / 4.0;
        let half = wanted.min(available).max(0.0);
        let left = -self.width / 2.0 + edge;
        let right = self.width / 2.0 - edge;
        MeterRow {
            half,
            in_label: left + in_label / 2.0,
            in_bar: left + in_label + gap + numeral + half,
            out_bar: right - numeral - half,
            out_label: right - 2.0 * numeral - 2.0 * half - gap - out_label / 2.0,
        }
    }
}

/// The meter row's four places, both bars the same length by construction. A
/// bar's scale reaches `numeral` past its end — the width the caller handed to
/// [`Layout::meter_row`], which is what it draws there.
struct MeterRow {
    /// Half the length of either bar.
    half: f32,
    in_label: f32,
    in_bar: f32,
    out_label: f32,
    out_bar: f32,
}

/// The scene behind the view: the same flat wash every nice plugin opens on, so
/// the editor and a screenshot of it agree on the background.
pub fn static_scene() -> Scene {
    Scene {
        root: SceneNode {
            shape: Some(Shape::Background { color: BG }),
            ..Default::default()
        },
        ..Default::default()
    }
}

impl Process for PiFaceUi {
    fn process(&mut self, ctx: &mut Context, _dt: f32) {
        // A refusal from the audio owner arrives in a slot the panel wrote
        // nothing to; it belongs to the view until the view shows it.
        #[cfg(feature = "standalone")]
        if let Some(error) = crate::standalone::control::take_error() {
            self.message = error;
            self.applied = Settings::default();
            self.panel = None;
        }
        #[cfg(feature = "standalone")]
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) && self.settings_page {
            self.draw_settings_page(ctx);
            return;
        }
        let Some(params) = self.params.clone() else {
            self.text(ctx, 0.0, 0.0, "NICE PI-FASE".into(), 24.0, MUTED);
            return;
        };
        let (w, h) = ctx.size();
        let layout = Layout::new(w, h);
        let fit = layout.fit;

        // ---- geometry, once, shared by drawing and hit testing ----
        let y_title = h / 2.0 - 26.0 * fit;
        let y_meter = h / 2.0 - 78.0 * fit;
        let cog = crate::STANDALONE_MODE.load(Ordering::Relaxed);
        let cog_x = w / 2.0 - 26.0 * fit;
        let pedal_x = 0.0;
        let pedal_y = -h / 2.0 + 66.0 * fit;
        let pedal_half = w / 2.0 - 120.0 * fit;

        let rows = self.rows(&params, &layout);
        let pedal_value = self.dial_value(&params, Dial::Pedal);
        let pedal_fill = Dial::Pedal
            .param(&params)
            .range()
            .normalize(pedal_value)
            .clamp(0.0, 1.0);

        // ---- pointer bookkeeping: an edge on press, a delta against this
        // frame's position for drags, hover straight from containment ----
        let mouse = ctx.mouse_position();
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let pressed = down && !self.was_down;
        let released = !down && self.was_down;
        let fine =
            ctx.key_down(frost::KeyCode::ShiftLeft) || ctx.key_down(frost::KeyCode::ShiftRight);

        let mut hover = None;
        if let Some([mx, my]) = mouse {
            for row in &rows {
                if (mx - row.x).abs() <= layout.column_half && (my - row.y).abs() <= 11.0 * fit {
                    hover = Some(row.hit);
                }
            }
            if hover.is_none()
                && (mx - pedal_x).abs() <= pedal_half
                && (my - pedal_y).abs() <= 11.0 * fit
            {
                hover = Some(Hit::Dial(Dial::Pedal));
            }
            if cog && hover.is_none() && (mx - cog_x).abs() <= 16.0 && (my - y_title).abs() <= 16.0
            {
                hover = Some(Hit::Cog);
            }
        }
        self.hovered = hover;

        if pressed {
            self.armed = hover;
            self.drag_anchor = mouse.map(|[mx, _]| mx).unwrap_or(0.0);
            if let Some(Hit::Dial(dial)) = hover {
                self.drag_start = dial
                    .param(&params)
                    .range()
                    .normalize(self.dial_value(&params, dial));
                self.drag = Some(Drag::Dial(dial));
                if let Some(context) = &self.context {
                    context
                        .param_setter()
                        .begin_set_parameter(dial.param(&params));
                }
            }
        }
        // A drag is a delta from where the press started, in normalised space,
        // so the number under the pointer never jumps however the control was
        // last set.
        if let Some(Drag::Dial(dial)) = self.drag
            && let Some([mx, _]) = mouse
        {
            let travel = if fine { DIAL_TRAVEL_FINE } else { DIAL_TRAVEL } * fit;
            let param = dial.param(&params);
            let t = (self.drag_start + (mx - self.drag_anchor) / travel).clamp(0.0, 1.0);
            let value = param.range().unnormalize(t);
            self.set_dial(&params, dial, value);
        }
        if released {
            if let Some(Drag::Dial(dial)) = self.drag.take() {
                if let Some(context) = &self.context {
                    context
                        .param_setter()
                        .end_set_parameter(dial.param(&params));
                }
                self.save_preferences();
            }
            if let Some(click) = self.armed.take().filter(|armed| Some(*armed) == hover) {
                match click {
                    Hit::Cog => {
                        #[cfg(feature = "standalone")]
                        {
                            self.panel = None;
                            self.settings_page = true;
                        }
                    }
                    Hit::Switch(switch) => {
                        let current = self.switch_value(&params, switch);
                        let next = Self::next_position(&params, switch, current);
                        self.set_switch(&params, switch, next);
                    }
                    Hit::Dial(_) => {}
                }
            }
        }
        // Wheel: one percentile of the range per line, the same delta a mouse's
        // notch is worth everywhere else.
        let wheel = ctx.mouse_wheel();
        if wheel != 0.0
            && let Some(Hit::Dial(dial)) = hover
        {
            let param = dial.param(&params);
            let t = (param.range().normalize(self.dial_value(&params, dial)) + wheel / 100.0)
                .clamp(0.0, 1.0);
            let value = param.range().unnormalize(t);
            self.set_dial(&params, dial, value);
            self.save_preferences();
        }
        self.was_down = down;

        // ---- drawing, back to front ----
        let title_size = 21.0 * fit;
        let title = "NICE PI-FASE";
        let title_w = self.text_width(ctx, title, title_size);
        self.text(
            ctx,
            -w / 2.0 + 22.0 * fit + title_w / 2.0,
            y_title,
            title.into(),
            title_size,
            TEXT,
        );
        let subtitle = "MODELLED ON THE BI-PHASE";
        let subtitle_w = self.text_width(ctx, subtitle, 12.0 * fit);
        self.text(
            ctx,
            -w / 2.0 + 22.0 * fit + subtitle_w / 2.0,
            y_title - 22.0 * fit,
            subtitle.into(),
            12.0 * fit,
            MUTED,
        );

        // Meter values: the audio thread publishes linear readings and stamps
        // the frame it published one; a thread that has gone quiet (a suspended
        // host) drops the meters to silence rather than freezing them lit.
        let now = crate::now_ms();
        let live = now.saturating_sub(
            self.shared
                .meter_updated
                .load(std::sync::atomic::Ordering::Relaxed),
        ) < METER_LIVE_MS;
        let db = |value: f32| {
            if live && value > 1e-7 {
                20.0 * value.log10()
            } else {
                -120.0
            }
        };
        self.input_meter.tick(
            db(self.shared.input_rms.load()),
            db(self.shared.input_peak.load()),
            now,
        );
        self.output_meter.tick(
            db(self.shared.output_rms.load()),
            db(self.shared.output_peak.load()),
            now,
        );
        // The meter row, measured before it is placed: `PeakMeter::draw` paints
        // its scale at 16 px with `-48` as its widest numeral, and a width taken
        // from the same shaping is what the row subtracts at each end.
        let (in_label, out_label) = ("IN", "OUT");
        let scale = 12.0 * fit;
        let row = layout.meter_row(
            self.text_width(ctx, in_label, scale),
            self.text_width(ctx, out_label, scale),
            self.text_width(ctx, "-48", 16.0 * fit) / 2.0,
        );
        self.text(ctx, row.in_label, y_meter, in_label.into(), scale, MUTED);
        self.input_meter
            .draw(ctx, &self.font, [row.in_bar, y_meter], row.half, fit, GREEN);
        self.text(ctx, row.out_label, y_meter, out_label.into(), scale, MUTED);
        self.output_meter.draw(
            ctx,
            &self.font,
            [row.out_bar, y_meter],
            row.half,
            fit,
            AMBER,
        );

        for (index, head) in GROUPS.iter().enumerate() {
            let head_w = self.text_width(ctx, head, 13.0 * fit);
            self.text(
                ctx,
                layout.column_x(index) - layout.column_half + head_w / 2.0,
                layout.y_head,
                (*head).into(),
                13.0 * fit,
                MUTED,
            );
        }
        for row in &rows {
            let held = matches!(self.drag, Some(Drag::Dial(dial)) if row.hit == Hit::Dial(dial));
            let hot = held || self.hovered == Some(row.hit);
            let lw = self.text_width(ctx, row.label, 13.0 * fit);
            self.text(
                ctx,
                row.x - layout.column_half + lw / 2.0,
                row.y,
                row.label.into(),
                13.0 * fit,
                if hot { AMBER } else { TEXT },
            );
            match row.positions {
                Some(positions) => {
                    // A switch: one segment per position, the current one lit.
                    let gap = 3.0 * fit;
                    let segment =
                        (2.0 * row.half - gap * (positions - 1) as f32) / positions as f32;
                    for position in 0..positions {
                        let sx =
                            row.x - row.half + segment / 2.0 + position as f32 * (segment + gap);
                        let on = position == row.active;
                        ctx.rectangle(
                            sx,
                            row.y,
                            segment / 2.0,
                            6.0 * fit,
                            if on { GREEN } else { BORE },
                            0.4,
                        );
                    }
                }
                None => {
                    ctx.rectangle(row.x, row.y, row.half, 3.0 * fit, BORE, 0.35);
                    let fill = row.fill.clamp(0.0, 1.0);
                    if fill > 0.0 {
                        ctx.rectangle(
                            row.x - row.half + row.half * fill,
                            row.y,
                            row.half * fill,
                            3.0 * fit,
                            if hot { AMBER } else { GREEN },
                            0.45,
                        );
                    }
                }
            }
            let rw = self.text_width(ctx, &row.readout, 13.0 * fit);
            self.text(
                ctx,
                row.x + layout.column_half - rw / 2.0,
                row.y,
                row.readout.clone(),
                13.0 * fit,
                if hot { AMBER } else { TEXT },
            );
        }

        // The pedal, across the bottom, because it belongs to neither lane nor
        // generator.
        let pedal_label = Dial::Pedal.label();
        let pedal_label_w = self.text_width(ctx, pedal_label, 13.0 * fit);
        self.text(
            ctx,
            -w / 2.0 + 22.0 * fit + pedal_label_w / 2.0,
            pedal_y,
            pedal_label.into(),
            13.0 * fit,
            if self.hovered == Some(Hit::Dial(Dial::Pedal)) {
                AMBER
            } else {
                TEXT
            },
        );
        ctx.rectangle(pedal_x, pedal_y, pedal_half, 3.0 * fit, BORE, 0.35);
        if pedal_fill > 0.0 {
            ctx.rectangle(
                pedal_x - pedal_half + pedal_half * pedal_fill,
                pedal_y,
                pedal_half * pedal_fill,
                3.0 * fit,
                GREEN,
                0.45,
            );
        }
        let pedal_readout = Dial::Pedal.readout(pedal_value);
        let pedal_readout_w = self.text_width(ctx, &pedal_readout, 13.0 * fit);
        self.text(
            ctx,
            pedal_x + pedal_half + 30.0 * fit + pedal_readout_w / 2.0,
            pedal_y,
            pedal_readout,
            13.0 * fit,
            TEXT,
        );

        // The cog, last of the chrome: a plate, an icon, and — the one door the
        // plugin build does not have — the app's own audio.
        if cog {
            ctx.rectangle(cog_x, y_title, 16.0, 16.0, PANEL, 0.5);
            ctx.text(
                cog_x,
                y_title,
                &self.icon_font,
                "\u{f6aa}",
                24.0,
                400.0,
                if hover == Some(Hit::Cog) {
                    AMBER
                } else {
                    rgb(180, 174, 193)
                },
                1.0,
            );
        }

        // The app's one line of news, and its drops counter: the two things a
        // host would otherwise be the player's only channel for.
        #[cfg(feature = "standalone")]
        {
            let message: String = self.message.chars().take(75).collect();
            if !message.is_empty() {
                self.text(
                    ctx,
                    cog_x - 140.0 * fit,
                    -h / 2.0 + 46.0 * fit,
                    message,
                    12.0 * fit,
                    AMBER,
                );
            }
            let drops = self.shared.dropped.load(Ordering::Relaxed);
            if drops > 0 {
                self.text(
                    ctx,
                    cog_x,
                    -h / 2.0 + 10.0 * fit,
                    format!("DROPS {drops}"),
                    10.0 * fit,
                    MUTED,
                );
            }
        }

        // A tooltip last, above everything, for whatever the pointer rests on
        // while nothing is being dragged.
        if self.drag.is_none() {
            let tip = match hover {
                Some(Hit::Dial(dial)) => Some(format!(
                    "{}: {}",
                    dial.tip(),
                    dial.readout(self.dial_value(&params, dial))
                )),
                Some(Hit::Switch(switch)) => Some(format!(
                    "{}: {}",
                    switch.tip(),
                    switch.readout(&params, self.switch_value(&params, switch))
                )),
                Some(Hit::Cog) => Some("Audio settings — devices, rates and buffers".into()),
                None => None,
            };
            if let Some(tip) = tip {
                nice_common::tooltip::draw(ctx, &self.font, &tip, fit);
            }
        }
    }
}

/// baseview's button names to frost's; anything frost has no word for is simply
/// not a button.
pub fn map_mouse(button: baseview::MouseButton) -> Option<frost::MouseButton> {
    Some(match button {
        baseview::MouseButton::Left => frost::MouseButton::Left,
        baseview::MouseButton::Right => frost::MouseButton::Right,
        baseview::MouseButton::Middle => frost::MouseButton::Middle,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{Dial, Layout, Placement, Switch};

    #[test]
    fn every_control_has_a_caption_a_tip_and_a_place() {
        for dial in Dial::ALL {
            assert!(!dial.label().is_empty());
            assert!(!dial.tip().is_empty());
            assert!(!dial.readout(0.5).is_empty());
        }
        for switch in Switch::ALL {
            assert!(!switch.label().is_empty());
            assert!(!switch.tip().is_empty());
            assert!(!switch.positions().is_empty());
        }
    }

    #[test]
    fn no_two_controls_share_a_slot() {
        // The layout is the controls' own choice of group and row. Two claiming
        // one slot would draw one over the other, and a player would spend the
        // rest of the session dragging a control they cannot see.
        let mut slots: Vec<(usize, usize)> = Dial::ALL
            .iter()
            .filter_map(|dial| dial.slot())
            .chain(Switch::ALL.iter().filter_map(|switch| switch.slot()))
            .collect();
        let count = slots.len();
        slots.sort_unstable();
        slots.dedup();
        assert_eq!(slots.len(), count, "two controls were given the same slot");
        // The pedal is the one control outside the grid, and it is drawn by
        // name — so exactly one control may opt out.
        let outside = Dial::ALL
            .iter()
            .filter(|dial| dial.slot().is_none())
            .count();
        assert_eq!(outside, 1);
    }

    #[test]
    fn the_view_reaches_every_parameter_the_host_sees() {
        // The dials and the switches are what a player can reach without a
        // host. A parameter missing from them is a number only a host can move;
        // two controls driving one parameter is a pair that fights when either
        // is dragged. The pairing is by the parameter object itself, so it
        // cannot be satisfied by a name that happens to match.
        use nice_plug::prelude::{Param, Params};

        let plugin = crate::PiFacePlugin::default();
        let params = &plugin.params;
        let map = params.param_map();

        let mut view: Vec<&str> = Dial::ALL
            .iter()
            .map(|dial| dial.param(params).name())
            .collect();
        view.extend(Switch::ALL.iter().map(|switch| switch.param(params).name()));
        assert_eq!(
            view.len(),
            map.len(),
            "the view and the host do not offer the same number of controls"
        );
        let mut names = view.clone();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), view.len(), "two controls share one parameter");
    }

    #[test]
    fn each_control_drives_the_cell_of_its_own_parameter() {
        // A dial wired to the wrong cell would look exactly right and move
        // something else, and the two truths — the parameter and the cell — are
        // only reconciled at construction. Seeded from the defaults, every pair
        // must agree, control by control.
        let plugin = crate::PiFacePlugin::default();
        for dial in Dial::ALL {
            assert_eq!(
                dial.param(&plugin.params).value(),
                dial.cell(&plugin.shared).load(),
                "{} does not drive its own cell",
                dial.label()
            );
        }
        for switch in Switch::ALL {
            assert_eq!(
                switch.param(&plugin.params).value(),
                switch.cell(&plugin.shared).load(),
                "{} does not drive its own cell",
                switch.label()
            );
        }
    }

    #[test]
    fn the_layout_stays_inside_the_window_at_every_size() {
        // These rows are absolute coordinates scaled by `fit`; a small window
        // must still land them inside itself, or the first thing a player meets
        // is controls they cannot reach.
        for (width, height) in [(620.0, 400.0), (900.0, 520.0), (2_000.0, 1_200.0)] {
            let layout = Layout::new(width, height);
            for column in 0..super::GROUPS.len() {
                let left = layout.column_x(column) - layout.column_half;
                let right = layout.column_x(column) + layout.column_half;
                assert!(left > -width / 2.0, "column {column} leaves the left");
                assert!(right < width / 2.0, "column {column} leaves the right");
            }
            assert!(layout.row_y(4) > -height / 2.0, "the last row falls off");
        }
    }

    #[test]
    fn the_meter_row_stays_inside_the_window_at_every_size() {
        // What the output meter used to do: `PeakMeter::draw` centres a numeral
        // on its tick, so the scale reaches half a numeral past the bar's end,
        // and the bar had been placed as if the row started at the centre. The
        // row is now built from the borders, which makes staying inside a
        // property of the layout rather than of one lucky window width.
        //
        // The widths are the ones the view measures at the design size, scaled
        // by `fit` as the drawn text is; the second pass carries three times
        // the numeral, because a row that cannot hold its own scale is the case
        // that must shorten the bars rather than cross the border.
        for (width, height) in [
            (405.0, 400.0),
            (620.0, 400.0),
            (900.0, 520.0),
            (2_000.0, 1_200.0),
        ] {
            let layout = Layout::new(width, height);
            for wide in [1.0, 3.0] {
                let numeral = 15.0 * wide * layout.fit;
                let row = layout.meter_row(21.0 * layout.fit, 30.0 * layout.fit, numeral);
                let right = row.out_bar + row.half + numeral;
                let left = row.in_bar - row.half - numeral;
                assert!(
                    right <= width / 2.0,
                    "the output meter leaves the right at {width}: {right} > {}",
                    width / 2.0
                );
                assert!(
                    left >= -width / 2.0,
                    "the input meter leaves the left at {width}: {left}"
                );
                assert!(
                    row.in_bar + row.half < row.out_bar - row.half,
                    "the two bars cross at {width}"
                );
                assert!(
                    row.in_label < row.in_bar - row.half && row.out_label < row.out_bar,
                    "a label sits on top of its own bar at {width}"
                );
            }
        }
        // And at the size the row was designed for, nothing was given up to make
        // room: the bars are the 250 px they have always been.
        let design = Layout::new(900.0, 520.0).meter_row(21.0, 30.0, 15.0);
        assert!(
            (design.half - 125.0).abs() < 0.01,
            "the design width lost meter length: {}",
            design.half
        );
    }
}
