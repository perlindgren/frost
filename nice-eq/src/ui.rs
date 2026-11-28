//! The equaliser's editor: seven faders over the band ladder, the output trim
//! standing apart at the end of the row, and a spectrum analyser under both.
//!
//! The layout is drawn in frost's window space: the origin sits at the
//! window's centre and the y axis points up, so hit tests compare against
//! `Context::mouse_position` in the same coordinates and nothing is ever
//! negated.
//!
//! Two things about this view are worth saying where they are used. A fader is
//! a *position*, not a relative control: press anywhere on a rail and the cap
//! goes where the pointer is, because that is what a fader on a machine does
//! and a graphic equaliser whose faders drift under your hand is a lie about
//! the panel. And the analyser is a guest here, not a tenant: the transforms
//! run on this thread, from the ring the audio thread fills, so a 32 k window
//! may take as long as it likes and the audio is none the worse for it. When
//! the spectrum switch says `off` this view stops asking for samples and
//! `process()` stops making them — the counters on both sides agree that
//! nothing was computed, and A-4 records why the picture is drawn from maxima
//! rather than from an average.
use crate::dsp::{self, BANDS, MAX_DB, Scope, Tap};
use crate::spectrum::{self, Analyser, FLOOR_DB, FftSize, Slot, Style};
use crate::{EqParams, EqSharedState};
use crossbeam::atomic::AtomicCell;
use frost::{Color, Context, Process, Scene, SceneNode, Shape};
use nice_common::meter::PeakMeter;
use nice_plug::context::gui::GuiContext;
use nice_plug::prelude::{FloatParam, IntParam, IntRange};
use rtrb::Consumer;
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[cfg(feature = "standalone")]
use crate::settings::{EqPreferences, Settings};
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

/// Samples the view is willing to work through in one frame after a stall.
/// More than this and the newest window is what matters, so the head of the
/// queue is dropped rather than analysed: an equaliser that spends two thirds
/// of a second catching up on audio nobody can still hear is measuring the
/// past.
const MAX_DRAIN: usize = 32_768;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}

const BG: Color = {
    Color {
        r: 32.0 / 255.0,
        g: 28.0 / 255.0,
        b: 44.0 / 255.0,
        a: 1.0,
    }
};
/// The face of every cap, plate and button.
const PANEL: Color = rgb(65, 57, 82);
/// The groove a cap travels in.
const BORE: Color = rgb(46, 40, 60);
/// The house green: the input, and everything that is where the player left
/// it.
const GREEN: Color = rgb(103, 237, 175);
/// The house amber: the output, a grabbed control, a held peak.
const AMBER: Color = rgb(250, 190, 115);
const RED: Color = rgb(248, 81, 73);
const TEXT: Color = rgb(235, 230, 241);
const MUTED: Color = rgb(150, 142, 166);

/// One column of the panel: the seven bands, low on the left, and the output
/// trim standing apart from them. `BANDS` is the split — everything below it
/// is a band of the ladder, the rest is the box's own level.
const COLUMNS: usize = BANDS + 1;

/// Which column is which control. The bands are positions in the ladder so the
/// labels come from the ladder itself; a second mode with different frequencies
/// therefore renames its own faders without this file being told.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fader {
    Band(usize),
    Out,
}

impl Fader {
    const ALL: [Fader; COLUMNS] = {
        let mut list = [Fader::Out; COLUMNS];
        let mut index = 0;
        while index < BANDS {
            list[index] = Fader::Band(index);
            index += 1;
        }
        list
    };

    fn param(self, params: &EqParams) -> &FloatParam {
        match self {
            Fader::Band(index) => params.band(index),
            Fader::Out => &params.gain_db,
        }
    }

    fn cell(self, shared: &EqSharedState) -> &AtomicCell<f32> {
        match self {
            Fader::Band(index) => shared.band(index),
            Fader::Out => &shared.gain_db,
        }
    }

    /// What the panel prints under the rail: the frequency the machine prints
    /// on its escutcheon, `1.6k` for a band at 1 600 hertz, and `OUT` for the
    /// trim, which belongs to no band.
    fn label(self, params: &EqParams) -> String {
        match self {
            Fader::Out => "OUT".to_string(),
            Fader::Band(index) => {
                let hz = dsp::Mode::from_param(params.mode.value()).bands()[index].hz;
                if hz >= 1_000.0 {
                    format!("{:.1}k", hz / 1_000.0).replace(".0k", "k")
                } else {
                    format!("{hz:.0}")
                }
            }
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Fader::Band(_) => "This band's lift or cut — drag the rail, wheel for half dB",
            Fader::Out => "Output level after the seven bands",
        }
    }
}

/// The four stepped settings of the analyser and the one of the ladder. Each
/// one is a switch on the machine, and each one is drawn as the segments it
/// has, because a player should be able to see how many positions there are
/// without clicking through them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Setting {
    Mode,
    Scope,
    Tap,
    Style,
    Size,
}

impl Setting {
    const ALL: [Setting; 5] = [
        Setting::Mode,
        Setting::Scope,
        Setting::Tap,
        Setting::Style,
        Setting::Size,
    ];

    fn caption(self) -> &'static str {
        match self {
            Setting::Mode => "MODE",
            Setting::Scope => "SPECTRUM",
            Setting::Tap => "SOURCE",
            Setting::Style => "VIEW",
            Setting::Size => "WINDOW",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Setting::Mode => "Which band ladder the faders stand on",
            Setting::Scope => {
                "OFF computes nothing, TOP draws the selected window, ALL overlays all six"
            }
            Setting::Tap => "Which end of the box is measured: in green, out in amber",
            Setting::Style => "Bars, or the same maxima joined into a curve",
            Setting::Size => {
                "Transform length: longer means finer frequency, coarser time — click the plot"
            }
        }
    }

    /// The host's names for the positions, in the order the parameter's
    /// integers run: the same list the parameter was built from, so a position
    /// cannot exist on the panel and not in the automation.
    fn positions(self) -> &'static [&'static str] {
        match self {
            Setting::Mode => crate::MODE_LABELS,
            Setting::Scope => crate::SCOPE_LABELS,
            Setting::Tap => crate::TAP_LABELS,
            Setting::Style => crate::STYLE_LABELS,
            Setting::Size => crate::SIZE_LABELS,
        }
    }

    fn param<'a>(&self, params: &'a EqParams) -> &'a IntParam {
        match self {
            Setting::Mode => &params.mode,
            Setting::Scope => &params.scope,
            Setting::Tap => &params.tap,
            Setting::Style => &params.style,
            Setting::Size => &params.size,
        }
    }

    fn cell<'a>(&self, shared: &'a EqSharedState) -> &'a AtomicCell<i32> {
        match self {
            Setting::Mode => &shared.mode,
            Setting::Scope => &shared.scope,
            Setting::Tap => &shared.tap,
            Setting::Style => &shared.style,
            Setting::Size => &shared.size,
        }
    }

    /// The first and last integer this switch's parameter accepts.
    fn bounds(self, params: &EqParams) -> (i32, i32) {
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
    fn readout(self, params: &EqParams, value: i32) -> String {
        let (min, max) = self.bounds(params);
        let index = (value.clamp(min, max) - min) as usize;
        self.positions()
            .get(index)
            .copied()
            .unwrap_or("?")
            .to_string()
    }
}

/// A frequency the way a player says it: `340 Hz`, `4.2 kHz`.
fn format_hz(value: f32) -> String {
    if value.abs() >= 1_000.0 {
        format!("{:.1} kHz", value / 1_000.0)
    } else {
        format!("{value:.0} Hz")
    }
}

/// What the pointer is on, so hovering and tooltips speak one vocabulary.
#[derive(Clone, Copy, PartialEq)]
enum Hover {
    Fader(Fader),
    Setting(Setting),
    /// The analyser's window: a click steps its length, which is the way the
    /// request for this panel asked for it to be changed.
    Plot,
    Cog,
}

/// What the left button is dragging, if it is dragging anything. Only faders
/// drag: a switch has positions and no travel.
#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Fader(Fader),
}

/// Where one row of faders lives this frame, shared between the drawing and
/// the hit testing so the two can never drift apart.
struct Rails {
    /// Centre x of each column.
    x: [f32; COLUMNS],
    /// The travel's ends, in window space: the cap's centre stops here at each
    /// end of the range, so the cap never covers the column's caption.
    top: f32,
    bottom: f32,
    /// Half the width of a grabbed column's invisible hit strip, and of the
    /// visible rail and cap.
    strip_half: f32,
    rail_half: f32,
    cap_half_width: f32,
    cap_half_height: f32,
}

impl Rails {
    /// The y a fader's cap sits at. The range is symmetric, so the middle of
    /// the travel is 0 dB — the one number on this panel that is the same for
    /// every band and does not have to be looked up.
    fn y_of(&self, db: f32) -> f32 {
        let t = (db + MAX_DB) / (2.0 * MAX_DB);
        self.bottom + t.clamp(0.0, 1.0) * (self.top - self.bottom)
    }

    /// The level for a pointer y, clamped because a drag may leave the rail's
    /// ends and is expected to stop at them, not wrap.
    fn db_at(&self, y: f32) -> f32 {
        let t = ((y - self.bottom) / (self.top - self.bottom)).clamp(0.0, 1.0);
        -MAX_DB + t * 2.0 * MAX_DB
    }

    fn contains(&self, index: usize, x: f32, y: f32) -> bool {
        (x - self.x[index]).abs() <= self.strip_half
            && y >= self.bottom - 10.0
            && y <= self.top + 10.0
    }
}

/// The analyser's frame: the decades across, the decibels down. Everything
/// about the picture — gridlines, curves, the pointer's frequency — comes off
/// these four numbers, so the axis and the hit test are the same statement.
struct Plot {
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
}

impl Plot {
    fn width(&self) -> f32 {
        (self.right - self.left).max(1.0)
    }

    /// The x for a frequency, on the log axis the analyser was reduced on.
    fn x_of(&self, hz: f32) -> f32 {
        spectrum::hz_to_x(hz, self.left, self.width())
    }

    /// The frequency under a pointer x.
    fn hz_at(&self, x: f32) -> f32 {
        spectrum::x_to_hz(x, self.left, self.width())
    }

    /// The y for a level in dBFS: the top of the frame is full scale and the
    /// bottom is the analyser's floor, so the picture and the numbers on it
    /// cannot disagree about where silence was declared.
    fn y_of(&self, db: f32) -> f32 {
        let t = (db - FLOOR_DB) / (0.0 - FLOOR_DB);
        self.bottom + t.clamp(0.0, 1.0) * (self.top - self.bottom)
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x <= self.right && y >= self.bottom && y <= self.top
    }

    /// One value per horizontal pixel of the frame: the reduction the
    /// analyser offers is per column, and a column per pixel is what makes the
    /// interpolated view a curve rather than a fence.
    fn columns(&self) -> usize {
        (self.width().ceil() as usize).clamp(8, 2_048)
    }
}

/// Where everything goes, derived once from the window size. The drawing, the
/// hit tests and the tests all read this one derivation, so what can be clicked
/// is what was drawn and neither can drift from the other: the row gets a fixed
/// share of what the title leaves and the plot the rest, because seven faders
/// need a travel and a spectrum needs a decade-and-a-half of axis.
struct Layout {
    fit: f32,
    y_title: f32,
    y_labels: f32,
    y_settings: f32,
    rails: Rails,
    plot: Plot,
}

impl Layout {
    fn new(w: f32, h: f32) -> Self {
        let fit = (w / 900.0).min(h / 560.0).clamp(0.45, 1.0);
        let title_h = 44.0 * fit;
        let label_h = 20.0 * fit;
        let settings_h = 30.0 * fit;
        let pad = 14.0 * fit;
        let y_title = h / 2.0 - 26.0 * fit;
        let available = (h - title_h - label_h - settings_h - 2.0 * pad).max(120.0 * fit);
        let fader_h = available * 0.56;
        let plot_h = available - fader_h;
        let rails_top = h / 2.0 - title_h;
        let rails_bottom = rails_top - fader_h;
        let y_labels = rails_bottom - label_h * 0.6;
        let y_settings = y_labels - settings_h * 0.7;
        let margin = 34.0 * fit;
        // The trim stands apart from the ladder by a gap, so a pointer aiming
        // at 6.4 k cannot land on the output by accident.
        let spacer = 26.0 * fit;
        let pitch = ((w - 2.0 * margin - spacer) / COLUMNS as f32).min(96.0 * fit);
        let first = -w / 2.0 + margin + pitch * 0.5;
        Self {
            fit,
            y_title,
            y_labels,
            y_settings,
            rails: Rails {
                x: std::array::from_fn(|index| {
                    let extra = if index < BANDS { 0.0 } else { spacer };
                    first + index as f32 * pitch + extra
                }),
                top: rails_top - pad,
                bottom: rails_bottom + pad,
                strip_half: (pitch * 0.5).min(40.0 * fit),
                rail_half: 2.5 * fit,
                cap_half_width: (pitch * 0.34).min(16.0 * fit),
                cap_half_height: 8.0 * fit,
            },
            plot: Plot {
                left: -w / 2.0 + 26.0 * fit,
                right: w / 2.0 - 26.0 * fit,
                top: y_settings - settings_h * 0.6,
                bottom: y_settings - settings_h * 0.6 - plot_h,
            },
        }
    }
}

pub struct EqUi {
    shared: Arc<EqSharedState>,
    font: Arc<[u8]>,
    icon_font: Arc<[u8]>,
    /// The plugin's parameters, and the host's setter when there is a host.
    /// The parameters arrive for every window — their ranges shape every drag
    /// and readout — and the setter only arrives in a host, the one place
    /// where writing through it means anything.
    params: Option<Arc<EqParams>>,
    context: Option<GuiContext>,
    input_meter: PeakMeter,
    output_meter: PeakMeter,
    was_down: bool,
    drag: Option<Drag>,
    /// The pointer coordinate and parameter value at the moment a drag grabbed:
    /// the value is what an absolute drag writes and what a Shifted fine move
    /// counts from, so nothing jumps however the fader was last set.
    drag_anchor: f32,
    drag_start: f32,
    hovered: Option<Hover>,
    /// What the current press started on. A release over the same thing is a
    /// click; a release anywhere else was a drag that wandered off.
    armed: Option<Hover>,
    /// The analyser and its halves of the feed. The consumers are borrowed
    /// from the plugin for as long as this window exists and handed back on
    /// drop, so a closed editor leaves the audio thread a place to write that
    /// nobody is reading rather than a ring that grows.
    analyser: Analyser,
    feed: Option<(Consumer<f32>, Consumer<f32>)>,
    /// The scratch the drawing reads: the window list, and one curve at a
    /// time. Reused every frame because a frame that allocated was the whole
    /// point of putting the transforms here instead of on the audio thread.
    sizes: [FftSize; 6],
    curve: Vec<f32>,
    /// The rate the last measurement was taken at, and the scope the last
    /// frame drew: both transitions clear the histories, because a window
    /// sampled at another rate, or filled while the view was watching, is not
    /// this picture's past.
    rate: f32,
    last_scope: Scope,
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
    panel: Option<AudioPanel<EqPreferences>>,
}

impl EqUi {
    pub fn new(shared: Arc<EqSharedState>, font: Arc<[u8]>) -> Self {
        let feed = shared.take_feed();
        Self {
            shared,
            icon_font: nice_common::fonts::icon_font(),
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
            analyser: Analyser::new(),
            feed,
            sizes: [FftSize::DEFAULT; 6],
            curve: Vec::new(),
            rate: 0.0,
            last_scope: Scope::Off,
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
    pub fn set_params(&mut self, params: Arc<EqParams>) {
        self.params = Some(params);
    }

    /// Receives the host's write door on top of the parameters. Writes go
    /// through it wherever it exists; where it does not, the shared cells
    /// below are the truth and carry the writes instead.
    pub fn set_gui_context(&mut self, context: GuiContext) {
        self.context = Some(context);
    }

    fn text(&self, ctx: &mut Context, x: f32, y: f32, text: String, size: f32, color: Color) {
        ctx.text(x, y, &self.font, text, size, TEXT_WEIGHT, color, 0.5);
    }

    fn text_width(&self, ctx: &Context, text: &str, size: f32) -> f32 {
        ctx.text_width(&self.font, text, size, TEXT_WEIGHT)
    }

    /// Where a fader's value is authoritative: the shared cell in standalone,
    /// where the view is the only writer and nice-plug gives a parameter no
    /// public setter; the host's parameter everywhere else, where automation
    /// writes it and a paused host would leave a mirrored cell stale.
    fn fader_value(&self, params: &EqParams, fader: Fader) -> f32 {
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            fader.cell(&self.shared).load()
        } else {
            fader.param(params).value()
        }
    }

    fn setting_value(&self, params: &EqParams, setting: Setting) -> i32 {
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            setting.cell(&self.shared).load()
        } else {
            setting.param(params).value()
        }
    }

    /// A fader write: clamp through the parameter's own range, ride the host's
    /// automation door when there is one, and always leave the cell saying the
    /// same thing — in a host the next block's mirror confirms it, in
    /// standalone the cell *is* the sound.
    fn write_fader(&self, params: &EqParams, fader: Fader, value: f32) {
        let param = fader.param(params);
        let t = param.range().normalize(value).clamp(0.0, 1.0);
        let value = param.range().unnormalize(t);
        if let Some(context) = &self.context {
            context.param_setter().set_parameter(param, value);
        }
        fader.cell(&self.shared).store(value);
    }

    fn set_fader(&mut self, params: &EqParams, fader: Fader, value: f32) {
        let param = fader.param(params);
        if let Some(context) = &self.context {
            context.param_setter().begin_set_parameter(param);
        }
        self.write_fader(params, fader, value);
        if let Some(context) = &self.context {
            context.param_setter().end_set_parameter(param);
        }
        self.save_preferences();
    }

    /// A switch write: the same door, with the position clamped into the
    /// parameter's own range so a stale cell or a hand-edited file cannot put
    /// the analyser in a position it does not have.
    fn set_setting(&mut self, params: &EqParams, setting: Setting, position: i32) {
        let param = setting.param(params);
        let (min, max) = setting.bounds(params);
        let position = position.clamp(min, max);
        if let Some(context) = &self.context {
            let setter = context.param_setter();
            setter.begin_set_parameter(param);
            setter.set_parameter(param, position);
            setter.end_set_parameter(param);
        }
        setting.cell(&self.shared).store(position);
        self.save_preferences();
    }

    /// The next (or, with `step` of `-1`, previous) position of a switch,
    /// wrapping — the way a thumb comes back round a rotary switch, and the
    /// reason the plot can be clicked forever to walk the window lengths.
    fn step_position(params: &EqParams, setting: Setting, current: i32, step: i32) -> i32 {
        let (min, max) = setting.bounds(params);
        let span = max - min + 1;
        min + (current.clamp(min, max) - min + step).rem_euclid(span)
    }

    /// Standalone persistence: what the player sets is what the next launch
    /// seeds from. The audio fields ride along untouched because this only
    /// ever reads and writes the flattened preferences half of the file.
    fn save_preferences(&mut self) {
        #[cfg(feature = "standalone")]
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            let shared = &self.shared;
            let mut settings = Settings::load().0;
            settings.mode = Some(shared.mode.load());
            settings.bands = Some(std::array::from_fn(|index| shared.band(index).load()));
            settings.gain_db = Some(shared.gain_db.load());
            settings.scope = Some(shared.scope.load());
            settings.tap = Some(shared.tap.load());
            settings.style = Some(shared.style.load());
            settings.fft_size = Some(shared.size.load());
            if let Err(error) = settings.save() {
                self.message = error;
            }
        }
    }

    /// Take whatever the audio thread has handed over since the last frame.
    /// Both taps are drained whenever there is a feed to drain: the rings are
    /// empty unless `process()` decided this end was worth copying, so this
    /// loop costs a `slots()` call per frame when it has nothing to do.
    fn drain(&mut self) {
        let rate = self.shared.sample_rate.load();
        if rate > 0.0 && (self.rate - rate).abs() > 0.5 {
            // A rate change: every held sample was taken through a different
            // window in time, and the axis would be quietly wrong about all of
            // them.
            self.analyser.clear();
        }
        self.rate = rate;
        let Some((feed_in, feed_out)) = self.feed.as_mut() else {
            return;
        };
        const CHUNK: usize = 4_096;
        let mut buf = [0.0f32; CHUNK];
        for (slot, ring) in [(Slot::In, feed_in), (Slot::Out, feed_out)] {
            if ring.slots() > MAX_DRAIN {
                let skip = ring.slots() - MAX_DRAIN;
                for _ in 0..skip {
                    if ring.pop().is_err() {
                        break;
                    }
                }
            }
            while ring.slots() > 0 {
                let want = ring.slots().min(CHUNK);
                if ring.pop_entire_slice(&mut buf[..want]).is_err() {
                    break;
                }
                self.analyser.push(slot, &buf[..want]);
            }
        }
    }

    /// The shared audio-settings page: the same panel nice-nam, nice-tuner and
    /// the rack run, with this app's preferences flattened into it. Apply
    /// travels through the control channel to the audio owner, which resolves,
    /// opens and reports — this page only draws and asks.
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

impl Drop for EqUi {
    fn drop(&mut self) {
        // The rings belong to the plugin. Handing the consumers back is what
        // lets a reopened editor find a feed instead of a plugin with nowhere
        // to write, and it is why closing a window never costs the host audio.
        if let Some(feed) = self.feed.take() {
            self.shared.put_feed(feed);
        }
    }
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

impl Process for EqUi {
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
        let (w, h) = ctx.size();
        let Some(params) = self.params.clone() else {
            self.text(ctx, 0.0, 0.0, "NICE EQ".into(), 24.0, MUTED);
            return;
        };
        let view = self.shared.view();
        if view.scope != self.last_scope && matches!(view.scope, Scope::Off) {
            // Switching the analyser off is a promise that nothing is being
            // computed; the histories have to go with it, or the frame after
            // the switch would still be drawing a picture from them.
            self.analyser.clear();
        }
        self.last_scope = view.scope;
        self.drain();

        // Geometry first, from the one derivation everything else reads.
        let Layout {
            fit,
            y_title,
            y_labels,
            y_settings,
            rails,
            plot,
            ..
        } = Layout::new(w, h);
        // The settings row, measured rather than guessed: each group is its
        // caption and its segments, and the whole row is laid out from the
        // left of the plot so it reads as the plot's own controls.
        let segment = 17.0 * fit;
        let gap = 3.0 * fit;
        let caption_size = 11.0 * fit;
        let mut group_x = [0.0f32; Setting::ALL.len()];
        let mut group_caption_w = [0.0f32; Setting::ALL.len()];
        let mut group_strip_w = [0.0f32; Setting::ALL.len()];
        {
            let mut x = plot.left;
            for (index, setting) in Setting::ALL.iter().enumerate() {
                let caption_w = self.text_width(ctx, setting.caption(), caption_size);
                let strip_w = setting.positions().len() as f32 * segment
                    + (setting.positions().len() - 1) as f32 * gap;
                group_caption_w[index] = caption_w;
                group_strip_w[index] = strip_w;
                group_x[index] = x + caption_w + 8.0 * fit;
                x = group_x[index] + strip_w + 22.0 * fit;
            }
        }
        let cog = crate::STANDALONE_MODE.load(Ordering::Relaxed);
        let (cog_x, cog_y) = (w / 2.0 - 26.0 * fit, y_settings);

        // Meter values: the audio thread publishes linear readings and stamps
        // the frame it published one; a thread that has gone quiet (a
        // suspended host) drops the meters to silence rather than freezing
        // them lit.
        let now = crate::now_ms();
        let live =
            now.saturating_sub(self.shared.meter_updated.load(Ordering::Relaxed)) < METER_LIVE_MS;
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

        // Pointer bookkeeping: an edge on press, a delta against the same
        // frame's position for drags, hover straight from containment.
        let mouse = ctx.mouse_position();
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let pressed = down && !self.was_down;
        let released = !down && self.was_down;

        let mut hover = None;
        if let Some([mx, my]) = mouse {
            if plot.contains(mx, my) {
                hover = Some(Hover::Plot);
            }
            for (index, setting) in Setting::ALL.iter().enumerate() {
                let centre = group_x[index] + group_strip_w[index] * 0.5;
                if (mx - centre).abs() <= group_strip_w[index] * 0.5 + 6.0 * fit
                    && (my - y_settings).abs() <= 10.0 * fit
                {
                    hover = Some(Hover::Setting(*setting));
                }
                let caption_cx = group_x[index] - 8.0 * fit - group_caption_w[index] * 0.5;
                if (mx - caption_cx).abs() <= group_caption_w[index] * 0.5 + 6.0 * fit
                    && (my - y_settings).abs() <= 10.0 * fit
                {
                    hover = Some(Hover::Setting(*setting));
                }
            }
            for (index, fader) in Fader::ALL.iter().enumerate() {
                if rails.contains(index, mx, my) {
                    hover = Some(Hover::Fader(*fader));
                }
            }
            if cog && hover.is_none() && (mx - cog_x).abs() <= 16.0 && (my - cog_y).abs() <= 16.0 {
                hover = Some(Hover::Cog);
            }
        }
        self.hovered = hover;

        // Grab. A press takes the deepest thing under the pointer, begins a
        // gesture on its parameter, and remembers where both the finger and
        // the value started. The same press also arms a click: whatever the
        // release lands on, if it is still this, was clicked.
        if pressed {
            self.armed = hover;
        }
        if pressed
            && let Some([_, my]) = mouse
            && let Some(Hover::Fader(fader)) = hover
        {
            if let Some(context) = &self.context {
                context
                    .param_setter()
                    .begin_set_parameter(fader.param(&params));
            }
            self.drag = Some(Drag::Fader(fader));
            self.drag_anchor = my;
            self.drag_start = self.fader_value(&params, fader);
        }
        // Move. A fader is a position: the cap goes to the pointer, which is
        // what the machine's own faders do. Where there is a host, values ride
        // the host's setter — the only door automation can hear, and the one
        // the host may record or veto — and every build also stores into the
        // shared cell, because in standalone the cell *is* the parameter.
        if let Some(Drag::Fader(fader)) = self.drag
            && let Some([_mx, my]) = mouse
        {
            let value = if fine_shift(ctx) {
                // Shift asks for the same travel to be worth four times less:
                // the anchor and the value the press found become the datum, so
                // holding Shift mid-drag refines instead of jumping.
                let span = 2.0 * MAX_DB;
                self.drag_start + (my - self.drag_anchor) * span / (1_600.0 * fit)
            } else {
                rails.db_at(my)
            };
            self.write_fader(&params, fader, value);
        }
        // Release closes the gesture the press opened, and a release over what
        // the press armed is a click on it.
        if released {
            if let Some(Drag::Fader(fader)) = self.drag {
                if let Some(context) = &self.context {
                    context
                        .param_setter()
                        .end_set_parameter(fader.param(&params));
                }
                self.drag = None;
                self.save_preferences();
            }
            if let Some(click) = self.armed.take().filter(|armed| Some(*armed) == hover) {
                match click {
                    Hover::Setting(setting) => {
                        let current = self.setting_value(&params, setting);
                        let next = Self::step_position(&params, setting, current, 1);
                        self.set_setting(&params, setting, next);
                    }
                    // The request for this panel asked for the resolution to be
                    // chosen by clicking the picture itself. It still is, even
                    // though the same setting is also reachable from its strip.
                    Hover::Plot => {
                        let current = self.setting_value(&params, Setting::Size);
                        let next = Self::step_position(&params, Setting::Size, current, 1);
                        self.set_setting(&params, Setting::Size, next);
                    }
                    Hover::Cog => {
                        #[cfg(feature = "standalone")]
                        {
                            self.panel = None;
                            self.settings_page = true;
                        }
                    }
                    Hover::Fader(_) => {}
                }
            }
        }
        // Wheel: half a decibel per notch on a fader — the panel's own marks
        // are every 5 dB, so a notch is a fifth of one — and one position per
        // notch everywhere a switch lives.
        let wheel = ctx.mouse_wheel();
        if wheel != 0.0 {
            match hover {
                Some(Hover::Fader(fader)) => {
                    let step = if fine_shift(ctx) { 0.1 } else { 0.5 };
                    let value = self.fader_value(&params, fader) + wheel.signum() * step;
                    self.set_fader(&params, fader, value);
                }
                Some(Hover::Setting(setting)) => {
                    let current = self.setting_value(&params, setting);
                    let step = if wheel > 0.0 { 1 } else { -1 };
                    let next = Self::step_position(&params, setting, current, step);
                    self.set_setting(&params, setting, next);
                }
                Some(Hover::Plot) => {
                    let current = self.setting_value(&params, Setting::Size);
                    let step = if wheel > 0.0 { 1 } else { -1 };
                    let next = Self::step_position(&params, Setting::Size, current, step);
                    self.set_setting(&params, Setting::Size, next);
                }
                Some(Hover::Cog) | None => {}
            }
        }
        self.was_down = down;

        // ---- drawing, back to front ----
        let title_size = 20.0 * fit;
        let title = "NICE EQ";
        let title_w = self.text_width(ctx, title, title_size);
        self.text(
            ctx,
            -w / 2.0 + 26.0 * fit + title_w / 2.0,
            y_title,
            title.into(),
            title_size,
            TEXT,
        );
        let mode = dsp::Mode::from_param(self.setting_value(&params, Setting::Mode)).label();
        let mode_size = 12.0 * fit;
        self.text(
            ctx,
            -w / 2.0
                + 26.0 * fit
                + title_w
                + 16.0 * fit
                + self.text_width(ctx, mode, mode_size) / 2.0,
            y_title,
            mode.to_string(),
            mode_size,
            MUTED,
        );
        let meter_half = (w * 0.12).min(150.0 * fit);
        let out_x = w / 2.0 - 26.0 * fit - meter_half;
        let in_x = out_x - 2.0 * meter_half - 34.0 * fit;
        self.text(
            ctx,
            in_x - meter_half - 16.0 * fit,
            y_title,
            "IN".into(),
            11.0 * fit,
            MUTED,
        );
        self.input_meter
            .draw(ctx, &self.font, [in_x, y_title], meter_half, fit, GREEN);
        self.text(
            ctx,
            out_x - meter_half - 22.0 * fit,
            y_title,
            "OUT".into(),
            11.0 * fit,
            MUTED,
        );
        self.output_meter
            .draw(ctx, &self.font, [out_x, y_title], meter_half, fit, AMBER);

        // The scale, once for the whole row: the marks a fader's travel has,
        // with the middle one longer because zero is the only position on this
        // panel that means "leave it alone".
        for mark in [-MAX_DB, -10.0, -5.0, 0.0, 5.0, 10.0, MAX_DB] {
            let y = rails.y_of(mark);
            let long = mark == 0.0;
            ctx.rectangle(
                rails.x[0] - rails.strip_half - (if long { 11.0 } else { 7.0 }) * fit,
                y,
                (if long { 11.0 } else { 7.0 }) * fit,
                0.8 * fit,
                if long { MUTED } else { BORE },
                0.32,
            );
        }
        for value in [MAX_DB, 0.0, -MAX_DB] {
            self.text(
                ctx,
                rails.x[0] - rails.strip_half - 26.0 * fit,
                rails.y_of(value),
                format!("{value:+.0}"),
                10.0 * fit,
                MUTED,
            );
        }

        // The faders: rail, the part of the travel that says how far this band
        // is from flat, the cap, and the two numbers a player reads — the
        // band's name under it and its level above it.
        for (index, fader) in Fader::ALL.iter().enumerate() {
            let x = rails.x[index];
            let value = self.fader_value(&params, *fader);
            let held = self.drag == Some(Drag::Fader(*fader));
            let hot = held || self.hovered == Some(Hover::Fader(*fader));
            ctx.rectangle(
                x,
                (rails.top + rails.bottom) * 0.5,
                rails.rail_half,
                (rails.top - rails.bottom) * 0.5,
                BORE,
                0.4,
            );
            let y = rails.y_of(value);
            let zero = rails.y_of(0.0);
            if (y - zero).abs() > 0.5 {
                ctx.rectangle(
                    x,
                    (y + zero) * 0.5,
                    rails.rail_half,
                    (y - zero).abs() * 0.5,
                    if value >= 0.0 { GREEN } else { AMBER },
                    0.45,
                );
            }
            ctx.rectangle(
                x,
                y,
                rails.cap_half_width,
                rails.cap_half_height,
                PANEL,
                0.5,
            );
            ctx.rectangle(
                x,
                y,
                rails.cap_half_width * 0.7,
                1.0 * fit,
                if held {
                    AMBER
                } else if hot {
                    TEXT
                } else {
                    MUTED
                },
                0.52,
            );
            self.text(ctx, x, y_labels, fader.label(&params), 12.0 * fit, TEXT);
            self.text(
                ctx,
                x,
                rails.top + 11.0 * fit,
                format!("{value:+.1}"),
                10.5 * fit,
                if value.abs() < 0.05 {
                    MUTED
                } else if value > 0.0 {
                    GREEN
                } else {
                    AMBER
                },
            );
        }

        // The settings: caption, segments, and the position's name where a
        // strip of six says nothing on its own.
        for (index, setting) in Setting::ALL.iter().enumerate() {
            let value = self.setting_value(&params, *setting);
            let (min, max) = setting.bounds(&params);
            let active = value.clamp(min, max) - min;
            let hot = self.hovered == Some(Hover::Setting(*setting));
            self.text(
                ctx,
                group_x[index] - 8.0 * fit - group_caption_w[index] * 0.5,
                y_settings,
                setting.caption().into(),
                caption_size,
                if hot { AMBER } else { MUTED },
            );
            for position in 0..setting.positions().len() {
                let sx = group_x[index] + segment * 0.5 + position as f32 * (segment + gap);
                let on = position as i32 == active;
                ctx.rectangle(
                    sx,
                    y_settings,
                    segment * 0.5,
                    6.0 * fit,
                    if on { GREEN } else { BORE },
                    0.4,
                );
            }
            if setting.positions().len() > 3 {
                let readout = setting.readout(&params, value);
                let w_read = self.text_width(ctx, &readout, 11.0 * fit);
                self.text(
                    ctx,
                    group_x[index] + group_strip_w[index] + 8.0 * fit + w_read * 0.5,
                    y_settings,
                    readout,
                    11.0 * fit,
                    if hot { AMBER } else { TEXT },
                );
            }
        }

        self.draw_spectrum(ctx, &plot, &params, fit);

        // The cog, last of the chrome: a plate, an icon, and — the one door the
        // plugin build does not have — the app's own audio.
        if cog {
            ctx.rectangle(cog_x, cog_y, 16.0, 16.0, PANEL, 0.5);
            ctx.text(
                cog_x,
                cog_y,
                &self.icon_font,
                "\u{f6aa}",
                24.0,
                400.0,
                if matches!(hover, Some(Hover::Cog)) {
                    AMBER
                } else {
                    rgb(180, 174, 193)
                },
                1.0,
            );
        }

        // The app's one line of news, and its two counters: the things a host
        // would otherwise be the player's only channel for. The feed counter is
        // here because a paused picture is a finding worth showing.
        #[cfg(feature = "standalone")]
        {
            let message: String = self.message.chars().take(75).collect();
            if !message.is_empty() {
                self.text(
                    ctx,
                    cog_x - 160.0 * fit,
                    y_settings,
                    message,
                    11.0 * fit,
                    AMBER,
                );
            }
            let drops = self.shared.dropped.load(Ordering::Relaxed);
            if drops > 0 {
                self.text(
                    ctx,
                    plot.right - 40.0 * fit,
                    plot.bottom - 12.0 * fit,
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
                Some(Hover::Fader(fader)) => Some(format!(
                    "{}: {:+.1} dB",
                    fader.tip(),
                    self.fader_value(&params, fader)
                )),
                Some(Hover::Setting(setting)) => Some(format!(
                    "{} — {} ({} positions, click to step)",
                    setting.tip(),
                    setting.readout(&params, self.setting_value(&params, setting)),
                    setting.positions().len()
                )),
                Some(Hover::Plot) => {
                    let at = mouse.map(|[mx, _]| format_hz(plot.hz_at(mx)));
                    Some(match at {
                        Some(hz) => format!("Spectrum — {hz}, click to step the window"),
                        None => "Spectrum — click to step the window".to_string(),
                    })
                }
                Some(Hover::Cog) => Some("Audio settings — devices, rates and buffers".into()),
                None => None,
            };
            if let Some(tip) = tip {
                nice_common::tooltip::draw(ctx, &self.font, &tip, fit);
            }
        }
    }
}

/// Whether the pointer should move in half-decibels or in tenths: the same
/// Shift the rest of the set asks for when a control is being worked finer.
fn fine_shift(ctx: &Context) -> bool {
    ctx.key_down(frost::KeyCode::ShiftLeft) || ctx.key_down(frost::KeyCode::ShiftRight)
}

impl EqUi {
    /// The analyser: its frame, its axis, and the curves this frame measured.
    ///
    /// Nothing here is on the audio thread, and the order of the drawing is
    /// the whole of the `all` setting's idea: every window is measured from
    /// the same history, the short ones first and the selected one last, so
    /// the one the player asked for is the one on top and the others are the
    /// context around it. Both taps get their own ink and are never blended,
    /// because the question "what came in and what went out" only has an
    /// answer if the two stay two.
    fn draw_spectrum(&mut self, ctx: &mut Context, plot: &Plot, params: &EqParams, fit: f32) {
        let scope = Scope::from_param(self.setting_value(params, Setting::Scope));
        let tap = Tap::from_param(self.setting_value(params, Setting::Tap));
        let style = Style::from_param(self.setting_value(params, Setting::Style));
        let size = FftSize::from_param(self.setting_value(params, Setting::Size));
        let rate = self.rate;
        // Whether either watched end is over full scale right now. The top line
        // of this frame *is* zero dBFS, so a curve running off it is a finding,
        // and the line is drawn in the set's over-full colour while it is.
        let now = crate::now_ms();
        let live =
            now.saturating_sub(self.shared.meter_updated.load(Ordering::Relaxed)) < METER_LIVE_MS;
        let hot = live
            && ((tap.wants_input() && self.shared.input_peak.load() > 1.0)
                || (tap.wants_output() && self.shared.output_peak.load() > 1.0));

        // The frame and the grid first: a picture with no axis is a shape.
        let width = plot.width();
        let height = (plot.top - plot.bottom).max(1.0);
        ctx.rectangle(
            (plot.left + plot.right) * 0.5,
            (plot.top + plot.bottom) * 0.5,
            width * 0.5,
            height * 0.5,
            rgb(26, 22, 36),
            0.2,
        );
        for level in [0.0, -24.0, -48.0, -72.0] {
            let y = plot.y_of(level);
            let full = level == 0.0;
            ctx.rectangle(
                (plot.left + plot.right) * 0.5,
                y,
                width * 0.5,
                0.6 * fit,
                if full {
                    if hot { RED } else { BORE }
                } else {
                    rgb(40, 35, 52)
                },
                0.22,
            );
            self.text(
                ctx,
                plot.left + 16.0 * fit,
                y,
                format!("{level:.0}"),
                9.0 * fit,
                if full && hot { RED } else { MUTED },
            );
        }
        for (hz, label) in [(100.0, "100"), (1_000.0, "1k"), (10_000.0, "10k")] {
            let x = plot.x_of(hz);
            ctx.rectangle(
                x,
                (plot.top + plot.bottom) * 0.5,
                0.6 * fit,
                height * 0.5,
                rgb(40, 35, 52),
                0.22,
            );
            self.text(
                ctx,
                x,
                plot.bottom + 8.0 * fit,
                label.into(),
                9.0 * fit,
                MUTED,
            );
        }

        if matches!(scope, Scope::Off) {
            self.text(
                ctx,
                (plot.left + plot.right) * 0.5,
                (plot.top + plot.bottom) * 0.5,
                "SPECTRUM OFF".into(),
                13.0 * fit,
                MUTED,
            );
            return;
        }
        let count = spectrum::sizes_for(scope, size, &mut self.sizes);
        let columns = plot.columns();
        let mut measured = 0usize;
        // Input first, output on top of it: the amber curve is the answer, and
        // the green one is the question it is an answer to.
        for slot in Slot::BOTH {
            let wanted = match slot {
                Slot::In => tap.wants_input(),
                Slot::Out => tap.wants_output(),
            };
            if !wanted {
                continue;
            }
            let ink = match slot {
                Slot::In => GREEN,
                Slot::Out => AMBER,
            };
            for index in 0..count {
                let window = self.sizes[index];
                let selected = index + 1 == count;
                if !self.analyser.ready(slot, window) {
                    continue;
                }
                if self
                    .analyser
                    .measure(slot, window, rate, columns, &mut self.curve)
                {
                    measured += 1;
                }
                // An overlay the player did not select is context, so it goes
                // behind at a quarter of its own ink — never a third colour,
                // because a third colour would be a third thing being measured.
                let color = if selected {
                    ink
                } else {
                    Color { a: 0.25, ..ink }
                };
                self.draw_curve(ctx, plot, columns, style, color, fit);
            }
        }
        if measured == 0 {
            // The line says what it is waiting for, in the units the waiting is
            // counted in: the window that was asked for, and how much of it has
            // arrived since the stream started or its rate changed.
            let slot = if tap.wants_input() {
                Slot::In
            } else {
                Slot::Out
            };
            let (have, want) = (self.analyser.held(slot), size.len());
            self.text(
                ctx,
                (plot.left + plot.right) * 0.5,
                (plot.top + plot.bottom) * 0.5,
                format!("WAITING FOR SIGNAL — {have} OF {want} SAMPLES"),
                13.0 * fit,
                MUTED,
            );
        }
        // The legend, in the corner of the picture it belongs to: the taps are
        // colours, and a colour that is not said is a colour that is guessed.
        if matches!(tap, Tap::Both) {
            self.text(
                ctx,
                plot.right - 34.0 * fit,
                plot.top - 9.0 * fit,
                "IN".into(),
                10.0 * fit,
                GREEN,
            );
            self.text(
                ctx,
                plot.right - 12.0 * fit,
                plot.top - 9.0 * fit,
                "OUT".into(),
                10.0 * fit,
                AMBER,
            );
        }
        // The window's own resolution, at the right of the axis it describes.
        if rate > 0.0 {
            let bins = rate / size.len() as f32;
            self.text(
                ctx,
                plot.right - 46.0 * fit,
                plot.bottom + 8.0 * fit,
                format!("{:.1} Hz bins", bins),
                9.0 * fit,
                MUTED,
            );
        }
    }

    /// One measured curve, drawn either way. Both renderings read the same
    /// array of per-column maxima, which is the reason switching between them
    /// changes the picture's grammar and not its information.
    fn draw_curve(
        &self,
        ctx: &mut Context,
        plot: &Plot,
        columns: usize,
        style: Style,
        color: Color,
        fit: f32,
    ) {
        let step = plot.width() / columns as f32;
        match style {
            Style::Bars => {
                for (index, &level) in self.curve.iter().enumerate().take(columns) {
                    if level <= FLOOR_DB {
                        continue;
                    }
                    let x = plot.left + (index as f32 + 0.5) * step;
                    let y = plot.y_of(level);
                    ctx.rectangle(
                        x,
                        (y + plot.bottom) * 0.5,
                        (step * 0.38).max(0.7),
                        (y - plot.bottom) * 0.5,
                        color,
                        0.3,
                    );
                }
            }
            Style::Line => {
                let mut previous: Option<(f32, f32)> = None;
                for (index, &level) in self.curve.iter().enumerate().take(columns) {
                    let x = plot.left + (index as f32 + 0.5) * step;
                    let y = plot.y_of(level);
                    if let Some((px, py)) = previous {
                        ctx.line(px, py, x, y, color, (1.4 * fit).max(0.8), 0.32);
                    }
                    previous = Some((x, y));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row of rails wide enough to be the default window's.
    fn rails() -> Rails {
        Rails {
            x: std::array::from_fn(|index| -300.0 + index as f32 * 90.0),
            top: 100.0,
            bottom: -100.0,
            strip_half: 40.0,
            rail_half: 2.5,
            cap_half_width: 16.0,
            cap_half_height: 8.0,
        }
    }

    fn plot() -> Plot {
        Plot {
            left: -400.0,
            right: 400.0,
            top: 0.0,
            bottom: -160.0,
        }
    }

    #[test]
    fn the_faders_run_low_to_high_and_the_trim_stands_apart() {
        // The request put 100 Hz on the left, so the ladder's order is part of
        // the promise, and the output trim is not one more band — it has to
        // read as separate at a glance.
        let w = 900.0;
        let fit = 1.0f32;
        let margin = 34.0 * fit;
        let spacer = 26.0 * fit;
        let pitch = ((w - 2.0 * margin - spacer) / COLUMNS as f32).min(96.0 * fit);
        let first = -w / 2.0 + margin + pitch * 0.5;
        let x: Vec<f32> = (0..COLUMNS)
            .map(|index| {
                let extra = if index < BANDS { 0.0 } else { spacer };
                first + index as f32 * pitch + extra
            })
            .collect();
        for pair in x.windows(2) {
            assert!(pair[1] > pair[0], "the row runs right to left");
        }
        let band_gap = x[1] - x[0];
        let trim_gap = x[BANDS] - x[BANDS - 1];
        assert!(
            trim_gap > band_gap * 1.2,
            "the trim sits in the row, not beside it: {trim_gap} vs {band_gap}"
        );
        assert!(
            x[0] > -w / 2.0 && x[COLUMNS - 1] < w / 2.0,
            "a fader left the window"
        );
    }

    #[test]
    fn a_fader_maps_its_travel_and_parks_at_its_ends() {
        let rails = rails();
        assert_eq!(rails.y_of(0.0), 0.0, "flat is the middle of the travel");
        assert!(rails.y_of(MAX_DB) > rails.y_of(-MAX_DB), "up must be up");
        for db in [-MAX_DB, -7.5, 0.0, 3.0, MAX_DB] {
            assert!((rails.db_at(rails.y_of(db)) - db).abs() < 1e-4, "{db} dB");
        }
        // A drag that leaves the rail is expected to stop at the end of it.
        assert_eq!(rails.db_at(1e5), MAX_DB);
        assert_eq!(rails.db_at(-1e5), -MAX_DB);
    }

    #[test]
    fn the_plot_maps_its_axis_back_and_forth() {
        let plot = plot();
        for hz in [20.0, 100.0, 1_000.0, 6_400.0, 20_000.0] {
            let x = plot.x_of(hz);
            assert!((plot.hz_at(x) - hz).abs() / hz < 0.01, "{hz} Hz");
        }
        // A decade is a fixed distance on a log axis, wherever it is. Both
        // samples are taken from the ladder's own ground: the floor of the axis
        // is where the axis starts, not a point on it, so a distance measured
        // from there is a clamped run and not a decade.
        let low = plot.x_of(1_000.0) - plot.x_of(100.0);
        let high = plot.x_of(20_000.0) - plot.x_of(2_000.0);
        assert!(
            (low - high).abs() < 1.0,
            "the decades are not even: {low} vs {high}"
        );
        // And what is off the ends lands on them instead of beyond.
        assert_eq!(plot.x_of(5.0), plot.left);
        assert_eq!(plot.x_of(90_000.0), plot.right);
        assert_eq!(plot.y_of(0.0), plot.top);
        assert_eq!(plot.y_of(FLOOR_DB), plot.bottom);
        assert!(plot.y_of(-40.0) > plot.y_of(-80.0));
    }

    #[test]
    fn every_setting_names_its_positions_and_opens_on_one_of_them() {
        let params = EqParams::default();
        for setting in Setting::ALL {
            assert!(!setting.caption().is_empty());
            assert!(!setting.tip().is_empty());
            let (min, max) = setting.bounds(&params);
            let positions = setting.positions();
            assert_eq!(
                max - min + 1,
                positions.len() as i32,
                "{} has a position the parameter does not",
                setting.caption()
            );
            let value = setting.param(&params).value();
            assert!(
                (min..=max).contains(&value),
                "{} opens outside its own range",
                setting.caption()
            );
            assert_ne!(setting.readout(&params, value), "?");
        }
    }

    #[test]
    fn stepping_a_switch_walks_its_positions_and_comes_back_round() {
        let params = EqParams::default();
        let start = Setting::Size.param(&params).value();
        let mut at = start;
        for _ in 0..crate::SIZE_LABELS.len() {
            at = EqUi::step_position(&params, Setting::Size, at, 1);
            assert!(
                (0..crate::SIZE_LABELS.len() as i32).contains(&at),
                "a step left the list of windows"
            );
        }
        assert_eq!(at, start, "a full turn of six positions should arrive home");
        assert_eq!(
            EqUi::step_position(&params, Setting::Size, 0, -1),
            crate::SIZE_LABELS.len() as i32 - 1,
            "the first position steps back to the last"
        );
    }

    #[test]
    fn the_labels_are_the_ladders_own_numbers() {
        let params = EqParams::default();
        let labels: Vec<String> = Fader::ALL
            .iter()
            .map(|fader| fader.label(&params))
            .collect();
        assert_eq!(
            labels,
            ["100", "200", "400", "800", "1.6k", "3.2k", "6.4k", "OUT"]
        );
    }

    #[test]
    fn the_plot_sits_under_the_faders_and_inside_the_window() {
        // The analyser was asked for *under* the sliders, and both have to
        // survive the smallest window the editor will accept. The assertions
        // read the same derivation the drawing does, so a layout that drifts
        // fails here instead of on screen.
        for (w, h) in [(640.0f32, 460.0), (900.0, 560.0), (1_600.0, 1_000.0)] {
            let Layout {
                fit,
                rails,
                plot,
                y_settings,
                ..
            } = Layout::new(w, h);
            assert!(plot.top < rails.bottom, "the plot is beside the faders");
            assert!(
                y_settings < rails.bottom,
                "the switches are beside the faders"
            );
            assert!(
                rails.top - rails.bottom > 100.0 * fit,
                "the travel is too short to aim at"
            );
            assert!(
                plot.top - plot.bottom > 60.0 * fit,
                "the plot is too flat to read"
            );
            assert!(
                plot.bottom > -h / 2.0,
                "the plot left the window: {} of {}",
                plot.bottom,
                -h / 2.0
            );
            let left = rails.x[0] - rails.strip_half;
            let right = rails.x[COLUMNS - 1] + rails.strip_half;
            assert!(left > -w / 2.0 && right < w / 2.0, "a fader left the panel");
        }
    }
}
