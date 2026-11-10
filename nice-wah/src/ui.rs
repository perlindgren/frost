//! The wah editor view: a pedal bar you drag, an envelope strip, six knobs,
//! and three factory voices that are only ever a set of parameter writes.
//!
//! The layout is drawn in frost's window space: the origin sits at the
//! window's centre and the y axis points up, so hit tests compare against
//! `Context::mouse_position` in the same coordinates and nothing is ever
//! negated.
use crate::dsp::{VOICES, Voice};
use crate::{WahParams, WahSharedState};
use crossbeam::atomic::AtomicCell;
use frost::{Color, Context, Process, Scene, SceneNode, Shape};
use nice_common::knob;
use nice_common::meter::PeakMeter;
use nice_plug::context::gui::GuiContext;
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[cfg(feature = "standalone")]
use crate::settings::{Settings, WahPreferences};
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
/// The two numbers — and the knob's whole look — now live in
/// `nice_common::knob`, shared with the phaser and every plugin to come.
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
/// The house green: the position the pedal and envelope actually put the
/// filter at.
const GREEN: Color = rgb(103, 237, 175);
/// The house amber: held peaks, the active voice, a grabbed control.
const AMBER: Color = rgb(250, 190, 115);
const TEXT: Color = rgb(235, 230, 241);
const MUTED: Color = rgb(150, 142, 166);

/// The six knobs, in the order the eye meets them left to right. Each names
/// itself and the parameter it owns, and nothing else about a knob lives
/// here — value, range and formatting all come off the parameter itself, so
/// a range change in `lib.rs` cannot leave the view disagreeing with the
/// DSP.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Knob {
    EnvAmount,
    Heel,
    Toe,
    Resonance,
    Taper,
    Out,
}

impl Knob {
    const ALL: [Knob; 6] = [
        Knob::EnvAmount,
        Knob::Heel,
        Knob::Toe,
        Knob::Resonance,
        Knob::Taper,
        Knob::Out,
    ];

    fn label(self) -> &'static str {
        match self {
            Knob::EnvAmount => "ENV",
            Knob::Heel => "HEEL",
            Knob::Toe => "TOE",
            Knob::Resonance => "RESO",
            Knob::Taper => "TAPER",
            Knob::Out => "OUT",
        }
    }

    fn param(self, params: &WahParams) -> &nice_plug::prelude::FloatParam {
        match self {
            Knob::EnvAmount => &params.env_amount,
            Knob::Heel => &params.heel_hz,
            Knob::Toe => &params.toe_hz,
            Knob::Resonance => &params.resonance,
            Knob::Taper => &params.taper,
            Knob::Out => &params.out_db,
        }
    }

    /// The same parameter's cell in the shared state: what the value *is*
    /// in standalone, and what `process()` publishes it from in a host.
    fn cell(self, shared: &WahSharedState) -> &AtomicCell<f32> {
        match self {
            Knob::EnvAmount => &shared.env_amount,
            Knob::Heel => &shared.heel_hz,
            Knob::Toe => &shared.toe_hz,
            Knob::Resonance => &shared.resonance,
            Knob::Taper => &shared.taper,
            Knob::Out => &shared.out_db,
        }
    }

    /// The one-line value shown under the knob. Each parameter speaks its
    /// own unit: percentages, Hz with a kHz turn-over, signed dB.
    fn readout(self, value: f32) -> String {
        match self {
            Knob::EnvAmount => format!("{:.0}%", value * 100.0),
            Knob::Heel | Knob::Toe => format_hz(value),
            Knob::Resonance => format!("Q {value:.1}"),
            Knob::Taper => format!("{value:+.2}"),
            Knob::Out => format!("{value:+.1} dB"),
        }
    }

    /// The tooltip's sentence, because a knob captioned `HEEL` deserves an
    /// explanation of what it heels *to*.
    fn tip(self) -> &'static str {
        match self {
            Knob::EnvAmount => "How far playing loudness walks the pedal",
            Knob::Heel => "Filter frequency with the pedal rocked back",
            Knob::Toe => "Filter frequency with the pedal shoved forward",
            Knob::Resonance => "How narrow and how loud the filter band is",
            Knob::Taper => "Sweep shape: negative heels early, positive toews",
            Knob::Out => "Output level after the filter",
        }
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
    Voice(usize),
    Pedal,
    /// The settings page's door, present only where the app owns its audio.
    Cog,
}

/// What the left button is currently dragging, if it is dragging anything.
#[derive(Clone, Copy)]
enum Drag {
    Knob(Knob),
    Pedal,
}

pub struct WahUi {
    shared: Arc<WahSharedState>,
    font: Arc<[u8]>,
    icon_font: Arc<[u8]>,
    /// The plugin's parameters, and the host's setter when there is a host.
    /// The parameters arrive for every window — their ranges shape every
    /// drag and readout — and the setter only arrives in a host, the one
    /// place where writing through it means anything.
    params: Option<Arc<WahParams>>,
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
    panel: Option<AudioPanel<WahPreferences>>,
}

impl WahUi {
    pub fn new(shared: Arc<WahSharedState>, font: Arc<[u8]>) -> Self {
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
    pub fn set_params(&mut self, params: Arc<WahParams>) {
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
    fn knob_value(&self, params: &WahParams, knob: Knob) -> f32 {
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            knob.cell(&self.shared).load()
        } else {
            knob.param(params).value()
        }
    }

    fn position_value(&self, params: &WahParams) -> f32 {
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            self.shared.position.load()
        } else {
            params.position.value()
        }
    }

    /// A voice is five parameter writes — that is the whole of it, which is
    /// why a voice button, a host preset and a hand-edit of the settings
    /// file all mean the same thing to this plugin.
    fn apply_voice(&mut self, voice: Voice) {
        let setter = self.context.as_ref().map(|context| context.param_setter());
        if let Some(params) = &self.params {
            let moves: &[(&nice_plug::prelude::FloatParam, f32)] = &[
                (&params.heel_hz, voice.heel_hz),
                (&params.toe_hz, voice.toe_hz),
                (&params.resonance, voice.q),
                (&params.taper, voice.taper),
                (&params.out_db, voice.trim_db),
            ];
            for (param, value) in moves {
                if let Some(setter) = &setter {
                    setter.begin_set_parameter(*param);
                    setter.set_parameter(*param, *value);
                    setter.end_set_parameter(*param);
                }
            }
            // The cells take the same writes either way: in a host the next
            // block's mirror confirms them, in standalone they are the sound.
            self.shared.heel_hz.store(voice.heel_hz);
            self.shared.toe_hz.store(voice.toe_hz);
            self.shared.resonance.store(voice.q);
            self.shared.taper.store(voice.taper);
            self.shared.out_db.store(voice.trim_db);
        }
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
            settings.position = Some(shared.position.load());
            settings.env_amount = Some(shared.env_amount.load());
            settings.heel_hz = Some(shared.heel_hz.load());
            settings.toe_hz = Some(shared.toe_hz.load());
            settings.resonance = Some(shared.resonance.load());
            settings.taper = Some(shared.taper.load());
            settings.out_db = Some(shared.out_db.load());
            if let Err(error) = settings.save() {
                self.message = error;
            }
        }
    }

    /// A voice reads as active when every parameter it owns already sits at
    /// the voice's number. Host automation can drag a voice away, and then
    /// the button honestly stops being lit — which is exactly what it means.
    fn voice_active(&self, params: &WahParams, voice: &Voice) -> bool {
        let close = |a: f32, b: f32| (a - b).abs() <= 0.01f32.max(a.abs() * 0.001);
        close(self.knob_value(params, Knob::Heel), voice.heel_hz)
            && close(self.knob_value(params, Knob::Toe), voice.toe_hz)
            && close(self.knob_value(params, Knob::Resonance), voice.q)
            && close(self.knob_value(params, Knob::Taper), voice.taper)
            && close(self.knob_value(params, Knob::Out), voice.trim_db)
    }

    /// The shared audio-settings page: the same panel nice-nam, nice-tuner
    /// and the rack run, with this app's preferences flattened into it.
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

/// Where the pedal bar lives this frame, shared between drawing and hit
/// testing so the two can never drift apart.
struct PedalBar {
    x_left: f32,
    half: f32,
    y: f32,
    half_height: f32,
}

impl PedalBar {
    /// The rail x for a position in `0.0..=1.0`.
    fn x_of(&self, t: f32) -> f32 {
        self.x_left + t * 2.0 * self.half
    }

    /// The position for a rail x, clamped because the drag may leave the
    /// bar's ends and is expected to stop at them.
    fn t_at(&self, x: f32) -> f32 {
        ((x - self.x_left) / (2.0 * self.half)).clamp(0.0, 1.0)
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x_left - 12.0
            && x <= self.x_left + 2.0 * self.half + 12.0
            && (y - self.y).abs() <= self.half_height + 10.0
    }
}

impl Process for WahUi {
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
            self.text(ctx, 0.0, 0.0, "NICE WAH".into(), 24.0, MUTED);
            return;
        };
        // Geometry first: every row's position comes from the window, and
        // both the drawing and the hit tests below read these numbers, so
        // what you can click is exactly what you can see.
        let y_title = h / 2.0 - 30.0 * fit;
        let y_meter = h / 2.0 - 92.0 * fit;
        let y_knob = 6.0 * fit;
        let pedal = PedalBar {
            x_left: -w / 2.0 + 84.0 * fit,
            half: w / 2.0 - 84.0 * fit - 84.0 * fit,
            y: -h / 2.0 + 52.0 * fit,
            half_height: 15.0 * fit,
        };
        let knob_radius = 24.0 * fit;
        let knob_pitch = ((w - 80.0 * fit) / 6.0).min(132.0 * fit);
        let knob_x = |i: usize| (i as f32 - 2.5) * knob_pitch;

        // The voice buttons are as wide as their names, right-aligned on
        // the title row.
        let voice_size = 13.0 * fit;
        let voice_height = 15.0 * fit;
        let mut voice_x = [0.0; VOICES.len()];
        let mut voice_half = [0.0; VOICES.len()];
        {
            let mut x = w / 2.0 - 24.0 * fit;
            for (i, voice) in VOICES.iter().enumerate().rev() {
                voice_half[i] = self.text_width(ctx, voice.name, voice_size) / 2.0 + 11.0 * fit;
                voice_x[i] = x - voice_half[i];
                x -= 2.0 * voice_half[i] + 8.0 * fit;
            }
        }
        // The settings cog sits in the empty corner below the TOE label —
        // the title row's right end belongs to the voice buttons — and only
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
        let env = if live {
            self.shared.env.load().clamp(0.0, 1.0)
        } else {
            0.0
        };
        let effective = if live {
            self.shared.effective.load().clamp(0.0, 1.0)
        } else {
            self.position_value(&params).clamp(0.0, 1.0)
        };
        let corner_hz = self.shared.corner_hz.load();

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
            for (i, _) in VOICES.iter().enumerate() {
                if (mx - voice_x[i]).abs() <= voice_half[i] && (my - y_title).abs() <= voice_height
                {
                    hover = Some(Hover::Voice(i));
                }
            }
            for (i, knob) in Knob::ALL.iter().enumerate() {
                if knob::contains([knob_x(i), y_knob], knob_radius, [mx, my]) {
                    hover = Some(Hover::Knob(*knob));
                }
            }
            if hover.is_none() && pedal.contains(mx, my) {
                hover = Some(Hover::Pedal);
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
        if pressed && let Some([mx, my]) = mouse {
            let grab = match hover {
                Some(Hover::Knob(knob)) => {
                    Some((Drag::Knob(knob), my, self.knob_value(&params, knob)))
                }
                Some(Hover::Pedal) => Some((Drag::Pedal, mx, self.position_value(&params))),
                _ => None,
            };
            if let Some((drag, anchor, start)) = grab {
                if let Some(context) = &self.context {
                    let setter = context.param_setter();
                    match drag {
                        Drag::Knob(knob) => setter.begin_set_parameter(knob.param(&params)),
                        Drag::Pedal => setter.begin_set_parameter(&params.position),
                    }
                }
                self.drag = Some(drag);
                self.drag_anchor = anchor;
                self.drag_start = start;
            }
        }
        // Move. Where there is a host, values ride the host's setter — the
        // only door automation can hear, and the one the host may record or
        // veto — and every build also stores into the shared cell, because
        // in standalone the cell *is* the parameter: nice-plug gives a
        // FloatParam no public setter, and a hostless app has no host to
        // write through.
        if let Some(drag) = self.drag
            && let Some([mx, my]) = mouse
        {
            let setter = self.context.as_ref().map(|context| context.param_setter());
            let (param, cell, value) = match drag {
                Drag::Knob(knob) => {
                    let param = knob.param(&params);
                    let travel = if fine { KNOB_TRAVEL_FINE } else { KNOB_TRAVEL };
                    let range = param.range();
                    let t = (range.normalize(self.drag_start) + (my - self.drag_anchor) / travel)
                        .clamp(0.0, 1.0);
                    (Some(param), knob.cell(&self.shared), range.unnormalize(t))
                }
                Drag::Pedal => (
                    Some(&params.position),
                    &self.shared.position,
                    pedal.t_at(mx),
                ),
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
                        Some(Drag::Knob(knob)) => setter.end_set_parameter(knob.param(&params)),
                        Some(Drag::Pedal) => setter.end_set_parameter(&params.position),
                        None => {}
                    }
                }
                self.drag = None;
                self.save_preferences();
            }
            if let Some(click) = self.armed.take().filter(|armed| Some(*armed) == hover) {
                match click {
                    Hover::Voice(i) => self.apply_voice(VOICES[i]),
                    Hover::Cog => {
                        #[cfg(feature = "standalone")]
                        {
                            self.panel = None;
                            self.settings_page = true;
                        }
                    }
                    Hover::Knob(_) | Hover::Pedal => {}
                }
            }
        }
        // Wheel: one percentile of the range per line, the same delta a
        // mouse's notch is worth everywhere else.
        let wheel = ctx.mouse_wheel();
        if wheel != 0.0
            && let Some(Hover::Knob(knob)) = hover
        {
            let param = knob.param(&params);
            let range = param.range();
            let t =
                (range.normalize(self.knob_value(&params, knob)) + wheel / 100.0).clamp(0.0, 1.0);
            let value = range.unnormalize(t);
            if let Some(context) = &self.context {
                let setter = context.param_setter();
                setter.begin_set_parameter(param);
                setter.set_parameter(param, value);
                setter.end_set_parameter(param);
            }
            knob.cell(&self.shared).store(value);
            self.save_preferences();
        }
        self.was_down = down;

        // ---- drawing, back to front ----
        let title_size = 21.0 * fit;
        let title = "NICE WAH";
        let title_w = self.text_width(ctx, title, title_size);
        self.text(
            ctx,
            -w / 2.0 + 24.0 * fit + title_w / 2.0,
            y_title,
            title.into(),
            title_size,
            TEXT,
        );
        for (i, voice) in VOICES.iter().enumerate() {
            let active = self.voice_active(&params, voice);
            let lit = active || matches!(self.hovered, Some(Hover::Voice(v)) if v == i);
            if active {
                ctx.rectangle(
                    voice_x[i],
                    y_title,
                    voice_half[i] + 2.0 * fit,
                    voice_height + 2.0 * fit,
                    AMBER,
                    0.49,
                );
            }
            ctx.rectangle(voice_x[i], y_title, voice_half[i], voice_height, PANEL, 0.5);
            // The name rides one notch above its panel — the only text in
            // this view that must not share its plate's layer.
            ctx.text(
                voice_x[i],
                y_title,
                &self.font,
                voice.name,
                voice_size,
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

        // Meters, and the envelope between them: input and output use the
        // shared dB scale, the envelope its own bare 0..1 bar because it is
        // not a level anybody would want in dBFS.
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
        let env_half = (w * 0.10).min(90.0 * fit);
        ctx.rectangle(
            0.0,
            y_meter,
            env_half,
            2.5 * fit,
            Color { a: 0.5, ..PANEL },
            0.5,
        );
        if env > 0.001 {
            ctx.rectangle(
                -env_half + env_half * env,
                y_meter,
                env_half * env,
                2.5 * fit,
                AMBER,
                0.55,
            );
        }
        self.text(
            ctx,
            0.0,
            y_meter + 16.0 * fit,
            "ENVELOPE".into(),
            13.0 * fit,
            MUTED,
        );

        // Knobs: the family's shared draw — face, bore, ticks, pointer,
        // caption and readout, all of it in `nice_common::knob`, so the
        // wah's row and the phaser's move together forever.
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

        // The pedal bar: the filled part is where the filter actually sits
        // (position plus the envelope's walk), and the pale tick is where
        // the pedal itself is — the difference between the two *is* the
        // envelope's answer, drawn.
        let grab_pedal =
            matches!(self.drag, Some(Drag::Pedal)) || matches!(self.hovered, Some(Hover::Pedal));
        ctx.rectangle(
            pedal.x_left + pedal.half,
            pedal.y,
            pedal.half,
            pedal.half_height,
            PANEL,
            0.5,
        );
        let x_eff = pedal.x_of(effective);
        if x_eff > pedal.x_left + 0.5 {
            ctx.rectangle(
                pedal.x_left + (x_eff - pedal.x_left) / 2.0,
                pedal.y,
                (x_eff - pedal.x_left) / 2.0,
                pedal.half_height,
                GREEN,
                0.52,
            );
        }
        let x_raw = pedal.x_of(self.position_value(&params).clamp(0.0, 1.0));
        ctx.rectangle(
            x_raw,
            pedal.y,
            8.0 * fit,
            pedal.half_height + 6.0 * fit,
            if grab_pedal { AMBER } else { TEXT },
            0.55,
        );
        let label_size = 12.0 * fit;
        self.text(
            ctx,
            pedal.x_left - 30.0 * fit,
            pedal.y,
            "HEEL".into(),
            label_size,
            MUTED,
        );
        self.text(
            ctx,
            pedal.x_left + 2.0 * pedal.half + 30.0 * fit,
            pedal.y,
            "TOE".into(),
            label_size,
            MUTED,
        );
        // The corner frequency is the audio thread's, because it is the one
        // that knows the sample rate; this is the player reading its own
        // pedal back.
        if live && corner_hz > 0.0 {
            self.text(
                ctx,
                pedal.x_left + pedal.half,
                pedal.y + pedal.half_height + 16.0 * fit,
                format_hz(corner_hz),
                15.0 * fit,
                GREEN,
            );
        }
        self.text(
            ctx,
            pedal.x_left + pedal.half,
            pedal.y - pedal.half_height - 18.0 * fit,
            format!("{:.0}%", self.position_value(&params) * 100.0),
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
                Some(Hover::Knob(knob)) => Some(format!(
                    "{}: {}",
                    knob.tip(),
                    knob.readout(self.knob_value(&params, knob))
                )),
                Some(Hover::Voice(i)) => Some(format!(
                    "{} — write this voice's heel, toe, reso, taper and level",
                    VOICES[i].name
                )),
                Some(Hover::Pedal) => Some("Wah position — drag; Shift-free, absolute".into()),
                Some(Hover::Cog) => Some("Audio settings — devices, rates and buffers".into()),
                None => None,
            };
            if let Some(tip) = tip {
                nice_common::tooltip::draw(ctx, &self.font, &tip, fit);
            }
        }
    }
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
    use super::{Knob, format_hz};

    #[test]
    fn hz_speaks_like_a_player() {
        assert_eq!(format_hz(340.0), "340 Hz");
        assert_eq!(format_hz(999.4), "999 Hz");
        assert_eq!(format_hz(1000.0), "1.0 kHz");
        assert_eq!(format_hz(4200.0), "4.2 kHz");
    }

    #[test]
    fn every_knob_has_a_caption_a_tip_and_a_readout() {
        for knob in Knob::ALL {
            assert!(!knob.label().is_empty());
            assert!(!knob.tip().is_empty());
            assert!(!knob.readout(0.5).is_empty());
        }
    }

    /// The pedal's two mappings must be inverses on the rail's interior and
    /// stay inside the rail outside it: a drag that leaves the bar's end is
    /// expected to park at the end, not wrap.
    #[test]
    fn pedal_maps_back_and_forth() {
        use super::PedalBar;
        let bar = PedalBar {
            x_left: -100.0,
            half: 100.0,
            y: 0.0,
            half_height: 15.0,
        };
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert!((bar.t_at(bar.x_of(t)) - t).abs() < 1e-6);
        }
        assert_eq!(bar.t_at(-1e6), 0.0);
        assert_eq!(bar.t_at(1e6), 1.0);
    }
}
