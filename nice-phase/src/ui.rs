//! The phaser editor view: five knobs, a row of mode names, a meter pair
//! with the LFO walking between them, and the sweep rail where the stage
//! frequency actually sits.
//!
//! The layout is drawn in frost's window space: the origin sits at the
//! window's centre and the y axis points up, so hit tests compare against
//! `Context::mouse_position` in the same coordinates and nothing is ever
//! negated.
use crate::dsp::{self, Mode};
use crate::{PhaseParams, PhaseSharedState};
use crossbeam::atomic::AtomicCell;
use frost::{Color, Context, Process, Scene, SceneNode, Shape};
use nice_common::knob;
use nice_common::meter::PeakMeter;
use nice_plug::context::gui::GuiContext;
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[cfg(feature = "standalone")]
use crate::settings::{PhasePreferences, Settings};
#[cfg(feature = "standalone")]
use nice_common::ui::{AudioPanel, Event};

/// The heaviest cut the embedded font has: `FiraCode-VariableFont_wght.ttf`
/// carries a single `wght` axis spanning 300..700, so 700 is as black as it
/// gets.
const TEXT_WEIGHT: f32 = 700.0;

/// How long the audio thread may have gone quiet before the meters are
/// considered dead: a suspended host would otherwise keep the last frame
/// lit forever.
const METER_LIVE_MS: u64 = 250;

/// The knob drag that carries a parameter from one end of its range to the
/// other, in logical pixels; holding Shift stretches it for fine moves.
/// The two numbers — and the knob's whole look — live in
/// `nice_common::knob`, shared with the wah.
const KNOB_TRAVEL: f32 = knob::TRAVEL;
const KNOB_TRAVEL_FINE: f32 = knob::TRAVEL_FINE;

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
/// The face of every knob, button and rail.
const PANEL: Color = rgb(65, 57, 82);
/// The house green: where the sweep actually is.
const GREEN: Color = rgb(103, 237, 175);
/// The house amber: held peaks, the active mode, a grabbed control.
const AMBER: Color = rgb(250, 190, 115);
const TEXT: Color = rgb(235, 230, 241);
const MUTED: Color = rgb(150, 142, 166);

/// The five knobs, in the order the eye meets them left to right. Each
/// names itself and the parameter it owns, and nothing else about a knob
/// lives here — value, range and formatting all come off the parameter
/// itself, so a range change in `lib.rs` cannot leave the view disagreeing
/// with the DSP.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Knob {
    Speed,
    Mode,
    Depth,
    Mix,
    Out,
}

impl Knob {
    const ALL: [Knob; 5] = [Knob::Speed, Knob::Mode, Knob::Depth, Knob::Mix, Knob::Out];

    fn label(self) -> &'static str {
        match self {
            Knob::Speed => "SPEED",
            Knob::Mode => "MODE",
            Knob::Depth => "DEPTH",
            Knob::Mix => "MIX",
            Knob::Out => "OUT",
        }
    }

    fn param(self, params: &PhaseParams) -> &nice_plug::prelude::FloatParam {
        match self {
            Knob::Speed => &params.speed,
            Knob::Mode => &params.mode,
            Knob::Depth => &params.depth,
            Knob::Mix => &params.mix,
            Knob::Out => &params.out_db,
        }
    }

    /// The same parameter's cell in the shared state: what the value *is*
    /// in standalone, and what `process()` publishes it from in a host.
    fn cell(self, shared: &PhaseSharedState) -> &AtomicCell<f32> {
        match self {
            Knob::Speed => &shared.speed,
            Knob::Mode => &shared.mode,
            Knob::Depth => &shared.depth,
            Knob::Mix => &shared.mix,
            Knob::Out => &shared.out_db,
        }
    }

    /// The one-line value shown under the knob. Each parameter speaks its
    /// own unit: the sine in Hz, the mode by name, the summer in
    /// percentages, the trim in signed dB.
    fn readout(self, value: f32) -> String {
        match self {
            Knob::Speed => format_speed(value),
            Knob::Mode => dsp::mode_of(value).name.into(),
            Knob::Depth | Knob::Mix => format!("{:.0}%", value * 100.0),
            Knob::Out => format!("{value:+.1} dB"),
        }
    }

    /// The tooltip's sentence, because a knob captioned `DEPTH` deserves to
    /// say what it deepens.
    fn tip(self) -> &'static str {
        match self {
            Knob::Speed => "How fast the sweep walks the stage frequency",
            Knob::Mode => "Sweep range and voicing — one of the five below the title",
            Knob::Depth => "How deep the notches are swept from their rest position",
            Knob::Mix => "Dry/wet blend — the hardware summer keeps its half dry",
            Knob::Out => "Output level after the blend",
        }
    }
}

/// The sine's rate the way a player sets it: `0.25 Hz`, `3.5 Hz`, `12 Hz`.
fn format_speed(value: f32) -> String {
    if value < 1.0 {
        format!("{value:.2} Hz")
    } else if value < 10.0 {
        format!("{value:.1} Hz")
    } else {
        format!("{value:.0} Hz")
    }
}

/// A frequency the way a player says it: `340 Hz`, `4.0 kHz`.
fn format_hz(value: f32) -> String {
    if value.abs() >= 1000.0 {
        format!("{:.1} kHz", value / 1000.0)
    } else {
        format!("{value:.0} Hz")
    }
}

/// What the pointer is over, so hovering and tooltips speak one vocabulary.
#[derive(Clone, Copy, PartialEq)]
enum Hover {
    Knob(Knob),
    Mode(usize),
    /// The settings page's door, present only where the app owns its audio.
    Cog,
}

/// What the left button is currently dragging, if it is dragging anything.
/// The selector is a click, never a drag — its own knob drags, the name
/// buttons snap.
#[derive(Clone, Copy)]
enum Drag {
    Knob(Knob),
}

pub struct PhaseUi {
    shared: Arc<PhaseSharedState>,
    font: Arc<[u8]>,
    icon_font: Arc<[u8]>,
    /// The plugin's parameters, and the host's setter when there is a host.
    /// The parameters arrive for every window — their ranges shape every
    /// drag and readout — and the setter only arrives in a host, the one
    /// place where writing through it means anything.
    params: Option<Arc<PhaseParams>>,
    context: Option<GuiContext>,
    input_meter: PeakMeter,
    output_meter: PeakMeter,
    was_down: bool,
    drag: Option<Drag>,
    /// The pointer coordinate and parameter value at the moment a drag
    /// grabbed: moves are deltas from there, so the value under the finger
    /// never jumps, however the knob was last set.
    drag_anchor: f32,
    drag_start: f32,
    hovered: Option<Hover>,
    /// What the current press started on. A release over the same thing is
    /// a click; a release anywhere else was a drag that wandered off.
    armed: Option<Hover>,
    #[cfg(feature = "standalone")]
    settings_page: bool,
    #[cfg(feature = "standalone")]
    message: String,
    /// The settings page keeps its own button edge: the page is drawn (and
    /// the main view's bookkeeping skipped) before `was_down` is next read.
    #[cfg(feature = "standalone")]
    pressed: bool,
    #[cfg(feature = "standalone")]
    applied: Settings,
    #[cfg(feature = "standalone")]
    panel: Option<AudioPanel<PhasePreferences>>,
}

impl PhaseUi {
    pub fn new(shared: Arc<PhaseSharedState>, font: Arc<[u8]>) -> Self {
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

    /// Receives the parameters to read their ranges and formats. Every
    /// number this view shows is read back every frame, so host automation
    /// moves the interface whether or not the user is touching it.
    pub fn set_params(&mut self, params: Arc<PhaseParams>) {
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

    /// Where a knob's value is authoritative: the shared cell in standalone,
    /// where the view is the only writer and nice-plug gives a parameter no
    /// public setter; the host's parameter everywhere else, where automation
    /// writes it and a paused host would leave a mirrored cell stale.
    fn knob_value(&self, params: &PhaseParams, knob: Knob) -> f32 {
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            knob.cell(&self.shared).load()
        } else {
            knob.param(params).value()
        }
    }

    /// A mode button is one parameter write — that is the whole of it, which
    /// is why a button, a host preset and a hand-edit of the settings file
    /// all mean the same thing to this plugin.
    fn apply_mode(&mut self, params: &PhaseParams, index: usize) {
        if let Some(setter) = self.context.as_ref().map(|context| context.param_setter()) {
            setter.begin_set_parameter(&params.mode);
            setter.set_parameter(&params.mode, index as f32);
            setter.end_set_parameter(&params.mode);
        }
        // The cell takes the write either way: in a host the next block's
        // mirror confirms it, in standalone it is the sound.
        self.shared.mode.store(index as f32);
        self.save_preferences();
    }

    /// Standalone persistence: what the player sets is what the next launch
    /// seeds from. The audio fields ride along untouched because this only
    /// ever reads and writes the flattened preferences half of the file.
    fn save_preferences(&mut self) {
        #[cfg(feature = "standalone")]
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            let shared = &self.shared;
            let mut settings = Settings::load().0;
            settings.speed = Some(shared.speed.load());
            settings.mode = Some(shared.mode.load());
            settings.depth = Some(shared.depth.load());
            settings.mix = Some(shared.mix.load());
            settings.out_db = Some(shared.out_db.load());
            if let Err(error) = settings.save() {
                self.message = error;
            }
        }
    }

    /// A mode reads as active when the selector lands on it — by
    /// [`dsp::mode_of`], the same rounding the audio thread uses, so the
    /// lit name is always the name the notches are obeying.
    fn mode_active(&self, params: &PhaseParams, index: usize) -> bool {
        dsp::mode_index(self.knob_value(params, Knob::Mode)) == index
    }

    /// The shared audio-settings page: the same panel nice-nam, nice-tuner
    /// and nice-wah run, with this app's preferences flattened into it.
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

/// The scene behind the view: the same flat wash every nice plugin opens
/// on, so the editor and a screenshot of it agree on the background.
pub fn static_scene() -> Scene {
    Scene {
        root: SceneNode {
            shape: Some(Shape::Background { color: BG }),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Where the sweep rail lives this frame, shared between drawing and
/// reporting so the two can never drift apart. The rail spans the active
/// mode's sweep, and — because both a sweep control and a frequency
/// response are heard in octaves — its axis is logarithmic.
struct Rail {
    x_left: f32,
    half: f32,
    y: f32,
    half_height: f32,
    lo_hz: f32,
    hi_hz: f32,
}

impl Rail {
    fn new(x_left: f32, half: f32, y: f32, half_height: f32, mode: &Mode) -> Self {
        Self {
            x_left,
            half,
            y,
            half_height,
            lo_hz: mode.lo_hz,
            hi_hz: mode.hi_hz,
        }
    }

    /// The rail x for a frequency on the mode's log axis. Clamped: a sweep
    /// at the end of its travel parks at the end of the rail.
    fn x_of(&self, hz: f32) -> f32 {
        // A comparison that NaN loses is a comparison that parks the
        // needle mid-rail rather than dividing by its nonsense.
        if self.hi_hz.partial_cmp(&self.lo_hz) != Some(std::cmp::Ordering::Greater)
            || hz <= 0.0
            || self.lo_hz <= 0.0
        {
            return self.x_left + self.half;
        }
        let t = (hz / self.lo_hz).ln() / (self.hi_hz / self.lo_hz).ln();
        self.x_left + t.clamp(0.0, 1.0) * 2.0 * self.half
    }

    /// The frequency for a rail x — the exact inverse of `x_of` inside
    /// the rail, kept for the test that proves the mapping is
    /// invertible; the needle itself only ever travels outward from
    /// the audio thread's numbers.
    #[cfg(test)]
    fn hz_at(&self, x: f32) -> f32 {
        let t = ((x - self.x_left) / (2.0 * self.half)).clamp(0.0, 1.0);
        self.lo_hz * (self.hi_hz / self.lo_hz).powf(t)
    }
}

impl Process for PhaseUi {
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
        let fit = (w / 860.0).clamp(0.45, 1.0);
        let Some(params) = self.params.clone() else {
            self.text(ctx, 0.0, 0.0, "NICE PHASE".into(), 24.0, MUTED);
            return;
        };
        // Geometry first: every row's position comes from the window, and
        // both the drawing and the hit tests below read these numbers, so
        // what you can click is exactly what you can see.
        let y_title = h / 2.0 - 30.0 * fit;
        let y_meter = h / 2.0 - 92.0 * fit;
        let y_knob = 6.0 * fit;
        let knob_radius = 24.0 * fit;
        let knob_pitch = ((w - 80.0 * fit) / 5.0).min(132.0 * fit);
        let knob_x = |i: usize| (i as f32 - 2.0) * knob_pitch;
        let mode_value = self.knob_value(&params, Knob::Mode);
        let mode = dsp::mode_of(mode_value);
        let rail = Rail::new(
            -w / 2.0 + 84.0 * fit,
            w / 2.0 - 84.0 * fit - 84.0 * fit,
            -h / 2.0 + 52.0 * fit,
            7.0 * fit,
            &mode,
        );

        // The mode buttons are as wide as their names, right-aligned on the
        // title row.
        let mode_size = 13.0 * fit;
        let mode_height = 15.0 * fit;
        let mut mode_x = [0.0; dsp::MODES.len()];
        let mut mode_half = [0.0; dsp::MODES.len()];
        {
            let mut x = w / 2.0 - 24.0 * fit;
            for (i, candidate) in dsp::MODES.iter().enumerate().rev() {
                mode_half[i] = self.text_width(ctx, candidate.name, mode_size) / 2.0 + 11.0 * fit;
                mode_x[i] = x - mode_half[i];
                x -= 2.0 * mode_half[i] + 8.0 * fit;
            }
        }
        // The settings cog sits in the empty corner below the OUT knob —
        // the title row's right end belongs to the mode buttons — and only
        // where the app owns its audio: a plugin's devices belong to its
        // host, so there would be nothing behind the door to open.
        let cog = crate::STANDALONE_MODE.load(Ordering::Relaxed);
        let (cog_x, cog_y) = (w / 2.0 - 26.0, -h / 2.0 + 24.0);

        // Meter values: the audio thread publishes linear readings and
        // stamps the frame it published one; a thread that has gone quiet
        // (a suspended host) drops the meters to silence rather than
        // freezing them lit.
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
        // The sine's own position and where it parked the stage frequency,
        // both the audio thread's, because it is the one that ran them.
        let lfo = if live {
            self.shared.lfo.load().rem_euclid(1.0)
        } else {
            0.5
        };
        let center_hz = self.shared.center_hz.load();

        // Pointer bookkeeping: an edge on press, a delta against the same
        // frame's position for drags, hover straight from containment.
        let mouse = ctx.mouse_position();
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let pressed = down && !self.was_down;
        let released = !down && self.was_down;
        let fine =
            ctx.key_down(frost::KeyCode::ShiftLeft) || ctx.key_down(frost::KeyCode::ShiftRight);

        let mut hover = None;
        if let Some([mx, my]) = mouse {
            for (i, _) in dsp::MODES.iter().enumerate() {
                if (mx - mode_x[i]).abs() <= mode_half[i] && (my - y_title).abs() <= mode_height {
                    hover = Some(Hover::Mode(i));
                }
            }
            for (i, kind) in Knob::ALL.iter().enumerate() {
                if knob::contains([knob_x(i), y_knob], knob_radius, [mx, my]) {
                    hover = Some(Hover::Knob(*kind));
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
            && let Some(Hover::Knob(kind)) = hover
        {
            if let Some(context) = &self.context {
                context
                    .param_setter()
                    .begin_set_parameter(kind.param(&params));
            }
            self.drag = Some(Drag::Knob(kind));
            self.drag_anchor = my;
            self.drag_start = self.knob_value(&params, kind);
        }
        // Move. Where there is a host, values ride the host's setter — the
        // only door automation can hear, and the one the host may record or
        // veto — and every build also stores into the shared cell, because
        // in standalone the cell *is* the parameter: nice-plug gives a
        // FloatParam no public setter, and a hostless app has no host to
        // write through. The selector snaps to whole modes mid-drag, so the
        // host never records a mode number that only rounds into meaning.
        if let Some(drag) = self.drag
            && let Some([_, my]) = mouse
        {
            let setter = self.context.as_ref().map(|context| context.param_setter());
            let (param, cell, value) = match drag {
                Drag::Knob(kind) => {
                    let param = kind.param(&params);
                    let travel = if fine { KNOB_TRAVEL_FINE } else { KNOB_TRAVEL };
                    let range = param.range();
                    let t = (range.normalize(self.drag_start) + (my - self.drag_anchor) / travel)
                        .clamp(0.0, 1.0);
                    let value = range.unnormalize(t);
                    (
                        Some(param),
                        kind.cell(&self.shared),
                        if kind == Knob::Mode {
                            dsp::mode_index(value) as f32
                        } else {
                            value
                        },
                    )
                }
            };
            if let (Some(setter), Some(param)) = (&setter, param) {
                setter.set_parameter(param, value);
            }
            cell.store(value);
        }
        // Release closes the gesture the press opened, and a release over
        // what the press armed is a click on it.
        if released {
            if self.drag.is_some() {
                if let Some(context) = &self.context {
                    let setter = context.param_setter();
                    match self.drag {
                        Some(Drag::Knob(kind)) => setter.end_set_parameter(kind.param(&params)),
                        None => {}
                    }
                }
                self.drag = None;
                self.save_preferences();
            }
            if let Some(click) = self.armed.take().filter(|armed| Some(*armed) == hover) {
                match click {
                    Hover::Mode(i) => self.apply_mode(&params, i),
                    Hover::Cog => {
                        #[cfg(feature = "standalone")]
                        {
                            self.panel = None;
                            self.settings_page = true;
                        }
                    }
                    Hover::Knob(_) => {}
                }
            }
        }
        // Wheel: one percentile of the range per line, the same delta a
        // mouse's notch is worth everywhere else — except on the selector,
        // where a percentile of four modes is no motion at all, so a notch
        // is one mode.
        let wheel = ctx.mouse_wheel();
        if wheel != 0.0
            && let Some(Hover::Knob(kind)) = hover
        {
            let param = kind.param(&params);
            let range = param.range();
            let value = if kind == Knob::Mode {
                let last = (dsp::MODES.len() - 1) as f32;
                (self.knob_value(&params, kind) + wheel.signum()).clamp(0.0, last)
            } else {
                let t = (range.normalize(self.knob_value(&params, kind)) + wheel / 100.0)
                    .clamp(0.0, 1.0);
                range.unnormalize(t)
            };
            if let Some(context) = &self.context {
                let setter = context.param_setter();
                setter.begin_set_parameter(param);
                setter.set_parameter(param, value);
                setter.end_set_parameter(param);
            }
            kind.cell(&self.shared).store(value);
            self.save_preferences();
        }
        self.was_down = down;

        // ---- drawing, back to front ----
        let title_size = 21.0 * fit;
        let title = "NICE PHASE";
        let title_w = self.text_width(ctx, title, title_size);
        self.text(
            ctx,
            -w / 2.0 + 24.0 * fit + title_w / 2.0,
            y_title,
            title.into(),
            title_size,
            TEXT,
        );
        for (i, candidate) in dsp::MODES.iter().enumerate() {
            let active = self.mode_active(&params, i);
            let lit = active || matches!(self.hovered, Some(Hover::Mode(v)) if v == i);
            if active {
                ctx.rectangle(
                    mode_x[i],
                    y_title,
                    mode_half[i] + 2.0 * fit,
                    mode_height + 2.0 * fit,
                    AMBER,
                    0.49,
                );
            }
            ctx.rectangle(mode_x[i], y_title, mode_half[i], mode_height, PANEL, 0.5);
            // The name rides one notch above its panel — the only text in
            // this view that must not share its plate's layer.
            ctx.text(
                mode_x[i],
                y_title,
                &self.font,
                candidate.name,
                mode_size,
                TEXT_WEIGHT,
                if active {
                    AMBER
                } else if lit {
                    TEXT
                } else {
                    MUTED
                },
                0.51,
            );
        }

        // Meters, and the sine between them: input and output use the
        // shared dB scale; the LFO gets its own bare bar because a phase
        // angle is not a level anybody would want in dBFS. It swings from
        // the centre out, the way the sine itself leaves zero.
        let meter_half = (w * 0.17).min(165.0 * fit);
        let meter_x = w / 2.0 - 24.0 * fit - meter_half;
        self.input_meter
            .draw(ctx, &self.font, [-meter_x, y_meter], meter_half, fit, GREEN);
        self.output_meter
            .draw(ctx, &self.font, [meter_x, y_meter], meter_half, fit, GREEN);
        self.text(
            ctx,
            -meter_x,
            y_meter + 16.0 * fit,
            "INPUT".into(),
            13.0 * fit,
            MUTED,
        );
        self.text(
            ctx,
            meter_x,
            y_meter + 16.0 * fit,
            "OUTPUT".into(),
            13.0 * fit,
            MUTED,
        );
        let lfo_half = (w * 0.10).min(90.0 * fit);
        ctx.rectangle(
            0.0,
            y_meter,
            lfo_half,
            2.5 * fit,
            Color { a: 0.5, ..PANEL },
            0.5,
        );
        let swing = (lfo - 0.5) * 2.0; // −1 = trough, +1 = crest
        if swing.abs() > 0.005 {
            ctx.rectangle(
                lfo_half * swing / 2.0,
                y_meter,
                lfo_half * swing.abs() / 2.0,
                2.5 * fit,
                AMBER,
                0.55,
            );
        }
        self.text(
            ctx,
            0.0,
            y_meter + 16.0 * fit,
            "SPEED".into(),
            13.0 * fit,
            MUTED,
        );

        // Knobs: the family's shared draw — face, bore, ticks, pointer,
        // caption and readout, all of it in `nice_common::knob`, so the
        // phaser's row and the wah's move together forever.
        for (i, kind) in Knob::ALL.iter().enumerate() {
            let (kx, ky) = (knob_x(i), y_knob);
            let param = kind.param(&params);
            let value = self.knob_value(&params, *kind);
            let t = param.range().normalize(value).clamp(0.0, 1.0);
            let touched = matches!(self.hovered, Some(Hover::Knob(k)) if k == *kind)
                || matches!(self.drag, Some(Drag::Knob(k)) if k == *kind);
            knob::draw(
                ctx,
                &self.font,
                [kx, ky],
                knob_radius,
                t,
                touched,
                kind.label(),
                kind.readout(value),
                fit,
            );
        }

        // The sweep rail: the mode's whole sweep as the axis, pale ticks
        // where the filter says its cancellations actually stand this
        // block, and a green needle at the stage frequency. The
        // positions are the audio thread's exact arithmetic — it knows
        // the sample rate and the tap's live position, mid-flip included;
        // a zero means the chain has no such notch today (a two-stage
        // setting has one hole, not two) and nothing is drawn. A notch
        // outside the mode's own axis is likewise unmarked: the rail
        // draws what it spans.
        ctx.rectangle(
            rail.x_left + rail.half,
            rail.y,
            rail.half,
            rail.half_height,
            PANEL,
            0.5,
        );
        if live {
            let notches = [self.shared.notch_lo.load(), self.shared.notch_hi.load()];
            for notch in notches {
                if notch > rail.lo_hz && notch < rail.hi_hz {
                    ctx.rectangle(
                        rail.x_of(notch),
                        rail.y,
                        1.5 * fit,
                        rail.half_height + 5.0 * fit,
                        Color { a: 0.8, ..GREEN },
                        0.52,
                    );
                }
            }
        }
        let x_center = rail.x_of(center_hz);
        if live && center_hz > 0.0 {
            ctx.rectangle(
                x_center,
                rail.y,
                8.0 * fit,
                rail.half_height + 6.0 * fit,
                GREEN,
                0.55,
            );
        }
        let label_size = 12.0 * fit;
        self.text(
            ctx,
            rail.x_left - 30.0 * fit,
            rail.y,
            format_hz(rail.lo_hz),
            label_size,
            MUTED,
        );
        self.text(
            ctx,
            rail.x_left + 2.0 * rail.half + 30.0 * fit,
            rail.y,
            format_hz(rail.hi_hz),
            label_size,
            MUTED,
        );
        // The stage frequency is the audio thread's, because it is the one
        // that knows the sample rate; this is the player reading the sweep
        // back.
        if live && center_hz > 0.0 {
            self.text(
                ctx,
                rail.x_left + rail.half,
                rail.y + rail.half_height + 16.0 * fit,
                format_hz(center_hz),
                15.0 * fit,
                GREEN,
            );
        }
        self.text(
            ctx,
            rail.x_left + rail.half,
            rail.y - rail.half_height - 18.0 * fit,
            mode.name.into(),
            13.0 * fit,
            MUTED,
        );

        // The cog, last of the chrome: a plate, an icon, and — the one
        // door the plugin build does not have — the app's own audio.
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

        // The app's one line of news, and its drops counter: the two things
        // a host would otherwise be the player's only channel for.
        #[cfg(feature = "standalone")]
        {
            let message: String = self.message.chars().take(75).collect();
            if !message.is_empty() {
                let mx = cog_x - 140.0 * fit;
                self.text(ctx, mx, -h / 2.0 + 24.0, message, 12.0 * fit, AMBER);
            }
            let drops = self.shared.dropped.load(Ordering::Relaxed);
            if drops > 0 {
                self.text(
                    ctx,
                    cog_x,
                    -h / 2.0 + 6.0,
                    format!("DROPS {drops}"),
                    10.0 * fit,
                    MUTED,
                );
            }
        }

        // A tooltip last, above everything, for whatever the pointer rests
        // on while nothing is being dragged.
        if self.drag.is_none() {
            let tip = match hover {
                Some(Hover::Knob(kind)) => Some(format!(
                    "{}: {}",
                    kind.tip(),
                    kind.readout(self.knob_value(&params, kind))
                )),
                Some(Hover::Mode(i)) => Some(format!(
                    "{} — sweep {}, {} stages, {} notches{}: write the selector",
                    dsp::MODES[i].name,
                    sweep_range(&dsp::MODES[i]),
                    dsp::MODES[i].stages,
                    dsp::notch_count(dsp::MODES[i].stages),
                    if dsp::MODES[i].feedback > 0.0 {
                        ", feedback on"
                    } else {
                        ", no feedback"
                    },
                )),
                Some(Hover::Cog) => Some("Audio settings — devices, rates and buffers".into()),
                None => None,
            };
            if let Some(tip) = tip {
                nice_common::tooltip::draw(ctx, &self.font, &tip, fit);
            }
        }
    }
}

/// A mode's sweep spoken once, for the tooltip and for anyone else who
/// wants the range without unpacking the table by hand.
fn sweep_range(mode: &Mode) -> String {
    format!("{}–{}", format_hz(mode.lo_hz), format_hz(mode.hi_hz))
}

/// baseview's button names to frost's; anything frost has no word for is
/// simply not a button.
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
    use super::{Knob, dsp, format_hz, format_speed, sweep_range};

    #[test]
    fn hz_speaks_like_a_player() {
        assert_eq!(format_hz(340.0), "340 Hz");
        assert_eq!(format_hz(999.4), "999 Hz");
        assert_eq!(format_hz(1000.0), "1.0 kHz");
        assert_eq!(format_hz(4200.0), "4.2 kHz");
    }

    #[test]
    fn speed_speaks_like_a_metronome() {
        assert_eq!(format_speed(0.05), "0.05 Hz");
        assert_eq!(format_speed(0.999), "1.00 Hz");
        assert_eq!(format_speed(3.5), "3.5 Hz");
        assert_eq!(format_speed(12.4), "12 Hz");
    }

    #[test]
    fn every_knob_has_a_caption_a_tip_and_a_readout() {
        for kind in Knob::ALL {
            assert!(!kind.label().is_empty());
            assert!(!kind.tip().is_empty());
            assert!(!kind.readout(0.5).is_empty());
        }
    }

    #[test]
    fn the_selector_knob_reads_the_table_by_name() {
        // The MODE caption under the knob is the mode the audio thread runs,
        // not the mode number: the readout proves it.
        for (i, mode) in dsp::MODES.iter().enumerate() {
            assert_eq!(Knob::Mode.readout(i as f32 + 0.49), mode.name);
        }
    }

    /// The rail's two mappings must be inverses inside the rail and stay
    /// inside it outside: a needle at the end of the sweep parks at the end
    /// of the bar, and the axis is octaves, not hertz.
    #[test]
    fn the_rail_is_logarithmic_and_back_again() {
        use super::{Mode, Rail};
        let mode = Mode {
            name: "TEST",
            lo_hz: 100.0,
            hi_hz: 1600.0,
            stages: 4,
            feedback: 0.0,
            trim_db: 0.0,
        };
        let rail = Rail::new(-100.0, 100.0, 0.0, 7.0, &mode);
        assert_eq!(rail.x_of(100.0), -100.0);
        assert_eq!(rail.x_of(1600.0), 100.0);
        // The geometric middle of a log axis is its centre, not its average.
        assert_eq!(rail.x_of(400.0), 0.0);
        for hz in [150.0, 400.0, 900.0] {
            assert!((rail.hz_at(rail.x_of(hz)) - hz).abs() < 1e-3);
        }
        // Outside the sweep the needle parks; it never wraps or extrapolates.
        assert_eq!(rail.x_of(10.0), -100.0);
        assert_eq!(rail.x_of(1e6), 100.0);
    }

    #[test]
    fn every_mode_introduces_itself() {
        for mode in dsp::MODES {
            assert!(sweep_range(&mode).contains('–'));
        }
    }
}
