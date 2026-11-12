//! The Nice Echo editor view.
//!
//! **This is the family's face, worn by this machine.** It gives every control
//! a place on screen and a readout, so the plugin, its parameters, the host's
//! automation and the standalone settings page can all be exercised. The tape
//! path it drives is the real one — three heads, wow and flutter, the saturating
//! record loop, the spring tank — and what this layout shows of the machine is
//! honest: the Mode knob points at the panel position the DSP is actually
//! using and names in words what that position routes, and every readout is
//! the parameter's own value, read back each frame. The
//! conventions are the family's: absolute coordinates scaled by a `fit` factor,
//! hit tests run against the same frame's geometry the drawing used, and a
//! value read from the same place the control is written, so the two cannot
//! disagree.
//!
//! frost's window space has its origin at the window's centre and the y axis
//! pointing up, so hit tests compare against `Context::mouse_position` in the
//! same coordinates and nothing is ever negated.
use crate::{EchoParams, EchoSharedState};
use crossbeam::atomic::AtomicCell;
use frost::{Color, Context, Process, Scene, SceneNode, Shape};
use nice_common::knob::{self, AMBER, BORE, GREEN, MUTED, PANEL, TEXT};
use nice_common::meter::PeakMeter;
use nice_plug::context::gui::GuiContext;
use nice_plug::prelude::{FloatParam, IntParam, IntRange};
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[cfg(feature = "standalone")]
use crate::settings::{EchoPreferences, Settings};
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
/// numbers rather than this window's, so a drag feels the same in any of these
/// plugins whatever it is dragging.
const DIAL_TRAVEL: f32 = knob::TRAVEL;
const DIAL_TRAVEL_FINE: f32 = knob::TRAVEL_FINE;

/// The three groups across the window. The controls name their own slot, so
/// this list is only the captions — a control cannot be left out of the layout
/// by forgetting to add it to a table. Reverb Volume sits in the output column
/// because in this model that is where it works: the spring is summed at the
/// output and never re-enters the tape loop, so its knob belongs beside the
/// other two that decide what leaves the machine.
const GROUPS: [&str; 3] = ["ECHO", "TONE", "OUTPUT"];

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
    RepeatRate,
    Intensity,
    EchoVolume,
    Treble,
    Bass,
    ReverbVolume,
}

impl Dial {
    const ALL: [Dial; 6] = [
        Dial::RepeatRate,
        Dial::Intensity,
        Dial::EchoVolume,
        Dial::Treble,
        Dial::Bass,
        Dial::ReverbVolume,
    ];

    fn label(self) -> &'static str {
        match self {
            Dial::RepeatRate => "REPEAT RATE",
            Dial::Intensity => "INTENSITY",
            Dial::EchoVolume => "ECHO VOLUME",
            Dial::Treble => "TREBLE",
            Dial::Bass => "BASS",
            Dial::ReverbVolume => "REVERB VOL",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Dial::RepeatRate => "Tape speed — slow tape stretches the delay, fast tape shortens it",
            Dial::Intensity => {
                "How much of the echo returns to the record head; past about three quarters the loop feeds itself and howls"
            }
            Dial::EchoVolume => "How much echo head in the output",
            Dial::Treble => "Treble on the record path — centred is passthrough",
            Dial::Bass => "Bass on the record path — centred is passthrough",
            Dial::ReverbVolume => {
                "How much spring tank in the output; the mode knob decides whether there is any to hear"
            }
        }
    }

    fn param<'a>(&self, params: &'a EchoParams) -> &'a FloatParam {
        match self {
            Dial::RepeatRate => &params.repeat_rate,
            Dial::Intensity => &params.intensity,
            Dial::EchoVolume => &params.echo_volume,
            Dial::Treble => &params.treble,
            Dial::Bass => &params.bass,
            Dial::ReverbVolume => &params.reverb_volume,
        }
    }

    fn cell<'a>(&self, shared: &'a EchoSharedState) -> &'a AtomicCell<f32> {
        match self {
            Dial::RepeatRate => &shared.repeat_rate,
            Dial::Intensity => &shared.intensity,
            Dial::EchoVolume => &shared.echo_volume,
            Dial::Treble => &shared.treble,
            Dial::Bass => &shared.bass,
            Dial::ReverbVolume => &shared.reverb_volume,
        }
    }

    fn readout(self, value: f32) -> String {
        match self {
            // The far playback head in milliseconds: the number the machine's
            // own delay time is quoted as (A-1 puts it at about 340 ms at the
            // slowest speed), so the knob reads in the unit the talk is in.
            Dial::RepeatRate => format!("{:.0} ms", crate::dsp::tap_seconds(value, 2) * 1000.0),
            _ => format!("{:.0}%", value * 100.0),
        }
    }
}

/// The switches — what the original builds as physical switching: the twelve-
/// position Mode selector, the two-position Echo switch, and the three-way
/// output-level selector for the machine's H, M and L jacks. The Mode selector
/// has no slot: twelve names do not fit a column, so it gets a stepped rotary
/// of its own at the bottom — turned by drag or wheel, detenting through the
/// twelve positions the way the panel switch does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Switch {
    Mode,
    Echo,
    OutputLevel,
}

impl Switch {
    const ALL: [Switch; 3] = [Switch::Mode, Switch::Echo, Switch::OutputLevel];

    fn label(self) -> &'static str {
        match self {
            Switch::Mode => "MODE",
            Switch::Echo => "ECHO SWITCH",
            Switch::OutputLevel => "OUTPUT LEVEL",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Switch::Mode => {
                "Which playback heads and whether the spring reach the output, on the panel's own twelve positions"
            }
            Switch::Echo => "NORMAL keeps the dry signal in with the echo; ECHO is 100 % echo",
            Switch::OutputLevel => "The three output jacks, trimmed eight dB apart: H, M, L",
        }
    }

    /// The names on the positions, in the order the parameter's integers run.
    fn positions(self) -> &'static [&'static str] {
        match self {
            Switch::Mode => &crate::MODE_LABELS[..],
            Switch::Echo => &crate::ECHO_SWITCH_LABELS[..],
            Switch::OutputLevel => &crate::OUTPUT_LEVEL_LABELS[..],
        }
    }

    fn param<'a>(&self, params: &'a EchoParams) -> &'a IntParam {
        match self {
            Switch::Mode => &params.mode,
            Switch::Echo => &params.echo_switch,
            Switch::OutputLevel => &params.output_level,
        }
    }

    fn cell<'a>(&self, shared: &'a EchoSharedState) -> &'a AtomicCell<i32> {
        match self {
            Switch::Mode => &shared.mode,
            Switch::Echo => &shared.echo_switch,
            Switch::OutputLevel => &shared.output_level,
        }
    }

    /// The first and last integer this switch's parameter accepts.
    fn bounds(self, params: &EchoParams) -> (i32, i32) {
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
    fn readout(self, params: &EchoParams, value: i32) -> String {
        let (min, max) = self.bounds(params);
        let index = (value.clamp(min, max) - min) as usize;
        self.positions()
            .get(index)
            .copied()
            .unwrap_or("?")
            .to_string()
    }
}

/// Which panel position a mode-knob drag has wound to: the drag's travel
/// since the press, added in normalised space to the position the press
/// started on and snapped to the nearest detent — a stepped rotary refuses
/// to settle between two positions. Clamped at both ends, because the
/// switch has stops and a hand dragged past either should stay on the last
/// real position, not wrap or run off the panel.
fn mode_detent(travel: f32, delta_px: f32, start: f32) -> usize {
    let last = (crate::dsp::MODES - 1) as f32;
    let t = (start + delta_px / travel).clamp(0.0, 1.0);
    ((t * last).round() as usize).min(crate::dsp::MODES - 1)
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
/// its own column and row, so the compiler — not a table that can be forgotten
/// — is what makes a new parameter appear on screen. `None` means the control
/// sits outside the three-column grid: the Mode selector, a stepped rotary at
/// the bottom centre, turned by drag or wheel rather than clicked.
trait Placement {
    fn slot(&self) -> Option<(usize, usize)>;
}

impl Placement for Dial {
    fn slot(&self) -> Option<(usize, usize)> {
        match self {
            Dial::RepeatRate => Some((0, 0)),
            Dial::Intensity => Some((0, 1)),
            Dial::EchoVolume => Some((0, 2)),
            Dial::Treble => Some((1, 0)),
            Dial::Bass => Some((1, 1)),
            Dial::ReverbVolume => Some((2, 0)),
        }
    }
}

impl Placement for Switch {
    fn slot(&self) -> Option<(usize, usize)> {
        match self {
            Switch::Mode => None,
            Switch::Echo => Some((2, 1)),
            Switch::OutputLevel => Some((2, 2)),
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
    /// The Mode rotary: a drag winds through the twelve detents, and every
    /// detent it crosses is written as it is crossed — the way turning a
    /// panel switch makes the machine change as your hand moves.
    Mode,
}

pub struct EchoUi {
    shared: Arc<EchoSharedState>,
    icon_font: Arc<[u8]>,
    font: Arc<[u8]>,
    params: Option<Arc<EchoParams>>,
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
    panel: Option<AudioPanel<EchoPreferences>>,
}

impl EchoUi {
    pub fn new(shared: Arc<EchoSharedState>, font: Arc<[u8]>) -> Self {
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
    pub fn set_params(&mut self, params: Arc<EchoParams>) {
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
    fn dial_value(&self, params: &EchoParams, dial: Dial) -> f32 {
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            dial.cell(&self.shared).load()
        } else {
            dial.param(params).value()
        }
    }

    fn switch_value(&self, params: &EchoParams, switch: Switch) -> i32 {
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
    fn set_dial(&mut self, params: &EchoParams, dial: Dial, value: f32) {
        let param = dial.param(params);
        let t = param.range().normalize(value).clamp(0.0, 1.0);
        let value = param.range().unnormalize(t);
        if let Some(context) = &self.context {
            context.param_setter().set_parameter(param, value);
        }
        dial.cell(&self.shared).store(value);
    }

    fn set_switch(&mut self, params: &EchoParams, switch: Switch, position: i32) {
        let param = switch.param(params);
        if let Some(context) = &self.context {
            context.param_setter().begin_set_parameter(param);
        }
        self.write_switch(params, switch, position);
        if let Some(context) = &self.context {
            context.param_setter().end_set_parameter(param);
        }
        self.save_preferences();
    }

    /// A switch write without the automation door's begin/end pair or the
    /// settings file. A drag brackets its whole travel in one pair of those
    /// and saves once at the release, rather than announcing every detent
    /// it crosses as a separate gesture the host would have to undo one by
    /// one.
    fn write_switch(&mut self, params: &EchoParams, switch: Switch, position: i32) {
        let param = switch.param(params);
        let (min, max) = switch.bounds(params);
        let position = position.clamp(min, max);
        if let Some(context) = &self.context {
            context.param_setter().set_parameter(param, position);
        }
        switch.cell(&self.shared).store(position);
    }

    /// Standalone persistence: what the player sets is what the next launch
    /// seeds from. The audio fields ride along untouched because this only ever
    /// reads and writes the flattened preferences half of the file.
    fn save_preferences(&mut self) {
        #[cfg(feature = "standalone")]
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            let shared = &self.shared;
            let mut settings = Settings::load().0;
            settings.mode = Some(shared.mode.load());
            settings.repeat_rate = Some(shared.repeat_rate.load());
            settings.intensity = Some(shared.intensity.load());
            settings.echo_volume = Some(shared.echo_volume.load());
            settings.reverb_volume = Some(shared.reverb_volume.load());
            settings.treble = Some(shared.treble.load());
            settings.bass = Some(shared.bass.load());
            settings.echo_switch = Some(shared.echo_switch.load());
            settings.output_level = Some(shared.output_level.load());
            if let Err(error) = settings.save() {
                self.message = error;
            }
        }
    }

    /// The next (or, with `step` of `-1`, previous) position of a switch,
    /// wrapping — the way a thumb comes back round a rotary switch.
    fn step_position(params: &EchoParams, switch: Switch, current: i32, step: i32) -> i32 {
        let (min, max) = switch.bounds(params);
        let span = max - min + 1;
        min + (current.clamp(min, max) - min + step).rem_euclid(span)
    }

    /// This frame's rows, read from the controls' own slots: three groups across
    /// the window, each a caption and its controls. Built once and read by both
    /// the hit tests and the drawing, so the two cannot drift apart. The Mode
    /// knob is not in here — it has no slot, and gets its own geometry.
    fn rows(&self, params: &EchoParams, layout: &Layout) -> Vec<Row> {
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
    /// The Mode knob, bottom centre: the one control outside the grid, with
    /// a number and a caption stacked under it the way the family's knobs
    /// stack a readout, so the geometry lives with the rest of the geometry.
    mode_radius: f32,
    mode_y: f32,
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
            mode_radius: 34.0 * fit,
            mode_y: -height / 2.0 + 120.0 * fit,
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
    /// therefore hangs outside the window, so the row is built from the borders
    /// in and what does not fit comes off the bars, never over the edge.
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

impl Process for EchoUi {
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
            self.text(ctx, 0.0, 0.0, "NICE ECHO".into(), 24.0, MUTED);
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
        let mode_x = 0.0;
        let mode_y = layout.mode_y;
        let mode_radius = layout.mode_radius;

        let rows = self.rows(&params, &layout);

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
            if hover.is_none() && knob::contains([mode_x, mode_y], mode_radius, [mx, my]) {
                hover = Some(Hit::Switch(Switch::Mode));
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
            } else if hover == Some(Hit::Switch(Switch::Mode)) {
                // The knob is grabbed where it points: the drag is a delta
                // from this detent, exactly as a dial's is from its value.
                let last = crate::dsp::MODES as i32 - 1;
                self.drag_start =
                    self.switch_value(&params, Switch::Mode).clamp(0, last) as f32 / last as f32;
                self.drag = Some(Drag::Mode);
                if let Some(context) = &self.context {
                    context.param_setter().begin_set_parameter(&params.mode);
                }
            }
        }
        // A drag is a delta from where the press started, in normalised space,
        // so the number under the pointer never jumps however the control was
        // last set.
        if let Some(dragged) = self.drag
            && let Some([mx, _]) = mouse
        {
            let travel = if fine { DIAL_TRAVEL_FINE } else { DIAL_TRAVEL } * fit;
            match dragged {
                Drag::Dial(dial) => {
                    let param = dial.param(&params);
                    let t = (self.drag_start + (mx - self.drag_anchor) / travel).clamp(0.0, 1.0);
                    let value = param.range().unnormalize(t);
                    self.set_dial(&params, dial, value);
                }
                Drag::Mode => {
                    // Every detent the hand crosses is written as it is
                    // crossed; the sound follows the knob mid-turn.
                    let detent = mode_detent(travel, mx - self.drag_anchor, self.drag_start);
                    if detent as i32 != self.switch_value(&params, Switch::Mode) {
                        self.write_switch(&params, Switch::Mode, detent as i32);
                    }
                }
            }
        }
        if released {
            if let Some(dragged) = self.drag.take() {
                if let Some(context) = &self.context {
                    let setter = context.param_setter();
                    match dragged {
                        Drag::Dial(dial) => setter.end_set_parameter(dial.param(&params)),
                        Drag::Mode => setter.end_set_parameter(&params.mode),
                    }
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
                        // A release over the mode knob that never turned
                        // it says nothing: a rotary switches nothing until
                        // it is turned, and every detent a real turn
                        // crossed was written while the drag was still
                        // moving. The column switches cycle, the way a
                        // small switch under a thumb does.
                        if switch != Switch::Mode {
                            let current = self.switch_value(&params, switch);
                            let position = Self::step_position(&params, switch, current, 1);
                            self.set_switch(&params, switch, position);
                        }
                    }
                    Hit::Dial(_) => {}
                }
            }
        }
        // Wheel: one percentile of the range per line for a dial, the same
        // delta a mouse's notch is worth everywhere else; for the mode knob,
        // one detent per line, wrapping.
        let wheel = ctx.mouse_wheel();
        if wheel != 0.0 {
            match hover {
                Some(Hit::Dial(dial)) => {
                    let param = dial.param(&params);
                    let t = (param.range().normalize(self.dial_value(&params, dial))
                        + wheel / 100.0)
                        .clamp(0.0, 1.0);
                    let value = param.range().unnormalize(t);
                    self.set_dial(&params, dial, value);
                    self.save_preferences();
                }
                Some(Hit::Switch(switch)) => {
                    let current = self.switch_value(&params, switch);
                    let step = if wheel > 0.0 { 1 } else { -1 };
                    let next = Self::step_position(&params, switch, current, step);
                    self.set_switch(&params, switch, next);
                }
                Some(Hit::Cog) | None => {}
            }
        }
        self.was_down = down;

        // ---- drawing, back to front ----
        let title_size = 21.0 * fit;
        let title = "NICE ECHO";
        let title_w = self.text_width(ctx, title, title_size);
        self.text(
            ctx,
            -w / 2.0 + 22.0 * fit + title_w / 2.0,
            y_title,
            title.into(),
            title_size,
            TEXT,
        );
        let subtitle = "MODELLED ON THE ROLAND RE-201 SPACE ECHO";
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

        // The Mode knob, bottom centre, because twelve positions belong to
        // no one group. The pointer and the lit detent both mark the
        // position the DSP is using — knob and sound read the same cell —
        // and the two lines under it name the position and say, in words
        // the mode table itself writes, what that position routes.
        let mode_hot = self.hovered == Some(Hit::Switch(Switch::Mode))
            || matches!(self.drag, Some(Drag::Mode));
        let mode_value = self
            .switch_value(&params, Switch::Mode)
            .clamp(0, crate::dsp::MODES as i32 - 1) as usize;
        let mode_angle = |t: f32| (t * 270.0 - 135.0).to_radians();
        self.text(
            ctx,
            mode_x,
            mode_y + mode_radius + 14.0 * fit,
            Switch::Mode.label().into(),
            13.0 * fit,
            if mode_hot { AMBER } else { MUTED },
        );
        // The detent ring: twelve marks on the family's sweep, the current
        // one lit the way the panel's own switch clicks into a notch.
        for position in 0..crate::dsp::MODES {
            let a = mode_angle(position as f32 / (crate::dsp::MODES - 1) as f32);
            let on = position == mode_value;
            ctx.line(
                mode_x + a.sin() * mode_radius * 1.06,
                mode_y + a.cos() * mode_radius * 1.06,
                mode_x + a.sin() * mode_radius * 1.22,
                mode_y + a.cos() * mode_radius * 1.22,
                if on {
                    if mode_hot { AMBER } else { GREEN }
                } else {
                    MUTED
                },
                if on { 2.5 * fit } else { 2.0 * fit },
                0.45,
            );
        }
        ctx.circle(mode_x, mode_y, mode_radius, PANEL, 0.5);
        ctx.circle(mode_x, mode_y, mode_radius * 0.66, BORE, 0.52);
        let a = mode_angle(mode_value as f32 / (crate::dsp::MODES - 1) as f32);
        ctx.line(
            mode_x,
            mode_y,
            mode_x + a.sin() * mode_radius * 0.9,
            mode_y + a.cos() * mode_radius * 0.9,
            if mode_hot { AMBER } else { TEXT },
            3.0 * fit,
            0.55,
        );
        // The panel number, then what the position routes — the caption
        // built from the mask the mixer itself reads, so label and sound
        // cannot disagree.
        self.text(
            ctx,
            mode_x,
            mode_y - mode_radius - 18.0 * fit,
            Switch::Mode.readout(&params, mode_value as i32),
            16.0 * fit,
            if mode_hot { AMBER } else { TEXT },
        );
        self.text(
            ctx,
            mode_x,
            mode_y - mode_radius - 36.0 * fit,
            crate::dsp::mode_caption(mode_value as i32),
            12.0 * fit,
            if mode_hot { AMBER } else { MUTED },
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
                // The mode knob carries its own sentence under the dial, and a
                // pointer that rests there should hear the same sentence: the
                // number the panel is on *and* what that position routes.
                Some(Hit::Switch(Switch::Mode)) => Some(format!(
                    "{}: {} — {}",
                    Switch::Mode.tip(),
                    Switch::Mode.readout(&params, self.switch_value(&params, Switch::Mode)),
                    crate::dsp::mode_caption(self.switch_value(&params, Switch::Mode))
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
    use super::{Dial, Layout, Placement, Switch, mode_detent};

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
        // The layout is the controls' own choice of column and row. Two claiming
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
        // The Mode knob is the one control outside the grid, and it is drawn
        // by name — so exactly one control may opt out.
        let outside = Dial::ALL
            .iter()
            .filter(|dial| dial.slot().is_none())
            .count()
            + Switch::ALL
                .iter()
                .filter(|switch| switch.slot().is_none())
                .count();
        assert_eq!(outside, 1);
    }

    #[test]
    fn the_mode_knob_lands_on_the_detent_a_drag_points_at() {
        // The knob turns by pixels, but the panel it stands for clicks between
        // twelve positions. If the rounding were off by one detent, the pointer
        // and the sound would part company — the dial would point at a
        // position the mixer is not routing, the one disagreement a stepped
        // knob can have with its panel, and the rounding is where it lives.
        let travel = 200.0; // DIAL_TRAVEL at the design width's fit
        let at = |position: usize| position as f32 / (crate::dsp::MODES - 1) as f32;

        // A drag shorter than half a detent (200 px over 11 of them) has not
        // reached the next one, and must not pretend it has.
        assert_eq!(mode_detent(travel, 0.0, at(5)), 5, "a still knob holds");
        assert_eq!(mode_detent(travel, 5.0, at(5)), 5, "5 px is not a detent");
        assert_eq!(mode_detent(travel, -5.0, at(5)), 5);
        // Past half a detent, it has arrived.
        assert_eq!(mode_detent(travel, 10.0, at(5)), 6);
        assert_eq!(mode_detent(travel, -10.0, at(5)), 4);
        // Ends stop instead of wrapping: the panel switch has no position above
        // twelve, and a drag that wrapped would put the sound somewhere the
        // pointer never passed. A turn past the end parks on the end.
        assert_eq!(mode_detent(travel, -400.0, at(5)), 0);
        assert_eq!(mode_detent(travel, 400.0, at(5)), crate::dsp::MODES - 1);
        assert_eq!(
            mode_detent(travel, -50.0, at(0)),
            0,
            "position 1 has no below"
        );
        assert_eq!(mode_detent(travel, 50.0, at(11)), crate::dsp::MODES - 1);
    }

    #[test]
    fn the_mode_knob_stays_inside_the_window_at_every_size() {
        // The knob sits below the grid with a number and a caption stacked under
        // it, so it has three things to keep in the window: its own label above,
        // its caption below, and the grid's last row it must not crowd. The
        // margins are the drawn extents, not guesses — `ctx.text` centres a
        // block on its anchor, so half a cap height reaches past each one.
        for (width, height) in [(620.0, 400.0), (900.0, 520.0), (2_000.0, 1_200.0)] {
            let layout = Layout::new(width, height);
            let fit = layout.fit;
            // MODE label: anchored a label's gap above the rim, half a 13 px
            // glyph past that.
            let label_top = layout.mode_y + layout.mode_radius + 14.0 * fit + 7.0 * fit;
            let last_row = layout.row_y(2);
            assert!(
                label_top < last_row - 6.0 * fit,
                "the mode knob crowds the last grid row at {width}x{height}: {label_top} >= {last_row}"
            );
            // Caption: anchored under the number, half a 12 px glyph below it.
            let caption_bottom = layout.mode_y - layout.mode_radius - 36.0 * fit - 6.0 * fit;
            assert!(
                caption_bottom > -height / 2.0,
                "the mode caption falls off the bottom at {width}x{height}: {caption_bottom}"
            );
        }
    }

    #[test]
    fn the_view_reaches_every_parameter_the_host_sees() {
        // The dials and the switches are what a player can reach without a
        // host. A parameter missing from them is a number only a host can move;
        // two controls driving one parameter is a pair that fights when either
        // is dragged. The pairing is by the parameter object itself, so it
        // cannot be satisfied by a name that happens to match.
        use nice_plug::prelude::{Param, Params};

        let plugin = crate::EchoPlugin::default();
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
        let plugin = crate::EchoPlugin::default();
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
            assert!(layout.row_y(2) > -height / 2.0, "the last row falls off");
        }
    }

    #[test]
    fn the_meter_row_stays_inside_the_window_at_every_size() {
        // `PeakMeter::draw` centres a numeral on its tick, so the scale reaches
        // half a numeral past the bar's end. The row is built from the borders,
        // which makes staying inside a property of the layout rather than of one
        // lucky window width.
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
