use crate::{CloneParams, CloneSharedState};
use crossbeam::atomic::AtomicCell;
use frost::{Color, Context, Process, Scene, SceneNode, Shape};
use nice_common::knob;
use nice_common::meter::PeakMeter;
use nice_plug::context::gui::GuiContext;
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[cfg(feature = "standalone")]
use crate::settings::{ClonePreferences, Settings};
#[cfg(feature = "standalone")]
use nice_common::ui::{AudioPanel, Event};

const TEXT_WEIGHT: f32 = 700.0;

const METER_LIVE_MS: u64 = 250;

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

const GREEN: Color = rgb(103, 237, 175);

const TEXT: Color = rgb(235, 230, 241);
const MUTED: Color = rgb(150, 142, 166);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Knob {
    Gain,
    Treble,
    Output,
}
impl Knob {
    const ALL: [Self; 3] = [Self::Gain, Self::Treble, Self::Output];
    fn label(self) -> &'static str {
        match self {
            Self::Gain => "GAIN",
            Self::Treble => "TREBLE",
            Self::Output => "OUTPUT",
        }
    }
    fn param(self, params: &CloneParams) -> &nice_plug::prelude::FloatParam {
        match self {
            Self::Gain => &params.gain,
            Self::Treble => &params.treble,
            Self::Output => &params.output,
        }
    }
    fn cell(self, shared: &CloneSharedState) -> &AtomicCell<f32> {
        match self {
            Self::Gain => &shared.gain,
            Self::Treble => &shared.treble,
            Self::Output => &shared.output,
        }
    }
    fn readout(self, value: f32) -> String {
        format!("{:.0}%", value * 100.0)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Hover {
    Knob(Knob),
    Cog,
}

#[derive(Clone, Copy)]
enum Drag {
    Knob(Knob),
}

pub struct CloneUi {
    shared: Arc<CloneSharedState>,
    font: Arc<[u8]>,
    icon_font: Arc<[u8]>,
    params: Option<Arc<CloneParams>>,
    context: Option<GuiContext>,
    input_meter: PeakMeter,
    output_meter: PeakMeter,
    was_down: bool,
    drag: Option<Drag>,
    drag_anchor: f32,
    drag_start: f32,
    armed: Option<Hover>,
    #[cfg(feature = "standalone")]
    settings_page: bool,
    #[cfg(feature = "standalone")]
    message: String,
    #[cfg(feature = "standalone")]
    pressed: bool,
    #[cfg(feature = "standalone")]
    applied: Settings,
    #[cfg(feature = "standalone")]
    panel: Option<AudioPanel<ClonePreferences>>,
}

impl CloneUi {
    pub fn new(shared: Arc<CloneSharedState>, font: Arc<[u8]>) -> Self {
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

    pub fn set_params(&mut self, params: Arc<CloneParams>) {
        self.params = Some(params);
    }

    pub fn set_gui_context(&mut self, context: GuiContext) {
        self.context = Some(context);
    }

    fn text(&self, ctx: &mut Context, x: f32, y: f32, text: String, size: f32, color: Color) {
        ctx.text(x, y, &self.font, text, size, TEXT_WEIGHT, color, 0.5);
    }

    fn knob_value(&self, params: &CloneParams, knob: Knob) -> f32 {
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            knob.cell(&self.shared).load()
        } else {
            knob.param(params).value()
        }
    }

    fn save_preferences(&mut self) {
        #[cfg(feature = "standalone")]
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            let shared = &self.shared;
            let mut settings = Settings::load().0;
            settings.gain = Some(shared.gain.load());
            settings.treble = Some(shared.treble.load());
            settings.output = Some(shared.output.load());
            if let Err(error) = settings.save() {
                self.message = error;
            }
        }
    }

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

pub fn static_scene() -> Scene {
    Scene {
        root: SceneNode {
            shape: Some(Shape::Background { color: BG }),
            ..Default::default()
        },
        ..Default::default()
    }
}

impl CloneUi {
    fn set_value(&self, params: &CloneParams, kind: Knob, value: f32) {
        if let Some(context) = &self.context {
            context
                .param_setter()
                .set_parameter(kind.param(params), value);
        }
        kind.cell(&self.shared).store(value);
    }
}

impl Process for CloneUi {
    fn process(&mut self, ctx: &mut Context, _dt: f32) {
        #[cfg(feature = "standalone")]
        if let Some(error) = crate::standalone::control::take_error() {
            self.message = error;
            self.applied = Settings::default();
            self.panel = None;
        }
        #[cfg(feature = "standalone")]
        if self.settings_page {
            self.draw_settings_page(ctx);
            return;
        }
        let Some(params) = self.params.clone() else {
            return;
        };
        let (w, h) = ctx.size();
        let fit = (w / 860.0).min(h / 440.0).clamp(0.45, 1.5);
        let radius = 43.0 * fit;
        let x = |i: usize| (i as f32 - 1.0) * 190.0 * fit;
        let y = 5.0 * fit;
        let cog_at = [w / 2.0 - 30.0 * fit, h / 2.0 - 30.0 * fit];
        let standalone = crate::STANDALONE_MODE.load(Ordering::Relaxed);
        let mouse = ctx.mouse_position();
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let pressed = down && !self.was_down;
        let released = !down && self.was_down;
        let fine =
            ctx.key_down(frost::KeyCode::ShiftLeft) || ctx.key_down(frost::KeyCode::ShiftRight);
        let mut hover = None;
        if let Some(at) = mouse {
            for (i, kind) in Knob::ALL.iter().enumerate() {
                if knob::contains([x(i), y], radius, at) {
                    hover = Some(Hover::Knob(*kind));
                }
            }
            if standalone
                && (at[0] - cog_at[0]).abs() < 20.0 * fit
                && (at[1] - cog_at[1]).abs() < 20.0 * fit
            {
                hover = Some(Hover::Cog);
            }
        }
        if pressed {
            self.armed = hover;
            if let (Some(Hover::Knob(kind)), Some([_, my])) = (hover, mouse) {
                if let Some(context) = &self.context {
                    context
                        .param_setter()
                        .begin_set_parameter(kind.param(&params));
                }
                self.drag = Some(Drag::Knob(kind));
                self.drag_anchor = my;
                self.drag_start = self.knob_value(&params, kind);
            }
        }
        if let (Some(Drag::Knob(kind)), Some([_, my])) = (self.drag, mouse) {
            let range = kind.param(&params).range();
            let travel = if fine { KNOB_TRAVEL_FINE } else { KNOB_TRAVEL };
            let t = (range.normalize(self.drag_start) + (my - self.drag_anchor) / travel)
                .clamp(0.0, 1.0);
            self.set_value(&params, kind, range.unnormalize(t));
        }
        if released {
            if let Some(Drag::Knob(kind)) = self.drag.take() {
                if let Some(context) = &self.context {
                    context
                        .param_setter()
                        .end_set_parameter(kind.param(&params));
                }
                self.save_preferences();
            }
            if self.armed.take() == Some(Hover::Cog) && hover == Some(Hover::Cog) {
                #[cfg(feature = "standalone")]
                {
                    self.panel = None;
                    self.settings_page = true;
                    self.pressed = false;
                }
            }
        }
        let wheel = ctx.mouse_wheel();
        if self.drag.is_none()
            && wheel != 0.0
            && let Some(Hover::Knob(kind)) = hover
        {
            let param = kind.param(&params);
            let range = param.range();
            let t = (range.normalize(self.knob_value(&params, kind))
                + wheel / if fine { 400.0 } else { 100.0 })
            .clamp(0.0, 1.0);
            if let Some(context) = &self.context {
                context.param_setter().begin_set_parameter(param);
            }
            self.set_value(&params, kind, range.unnormalize(t));
            if let Some(context) = &self.context {
                context.param_setter().end_set_parameter(param);
            }
            self.save_preferences();
        }
        self.was_down = down;
        self.text(
            ctx,
            0.0,
            h / 2.0 - 44.0 * fit,
            "NICE CLONE".into(),
            25.0 * fit,
            TEXT,
        );
        self.text(
            ctx,
            0.0,
            -h / 2.0 + 28.0 * fit,
            "Audio passes through unchanged".into(),
            12.0 * fit,
            MUTED,
        );
        if standalone {
            ctx.text(
                cog_at[0],
                cog_at[1],
                &self.icon_font,
                "\u{f6aa}",
                20.0 * fit,
                TEXT_WEIGHT,
                TEXT,
                0.5,
            );
        }
        for (i, kind) in Knob::ALL.iter().enumerate() {
            let value = self.knob_value(&params, *kind);
            let touched = hover == Some(Hover::Knob(*kind))
                || matches!(self.drag, Some(Drag::Knob(k)) if k == *kind);
            knob::draw(
                ctx,
                &self.font,
                [x(i), y],
                radius,
                kind.param(&params).range().normalize(value),
                touched,
                kind.label(),
                kind.readout(value),
                fit,
            );
        }
        let now = crate::now_ms();
        let live =
            now.saturating_sub(self.shared.meter_updated.load(Ordering::Relaxed)) < METER_LIVE_MS;
        let db = |v: f32| {
            if live && v > 1e-7 {
                20.0 * v.log10()
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
        let meter_y = -h / 2.0 + 77.0 * fit;
        self.input_meter.draw(
            ctx,
            &self.font,
            [-190.0 * fit, meter_y],
            120.0 * fit,
            fit,
            GREEN,
        );
        self.output_meter.draw(
            ctx,
            &self.font,
            [190.0 * fit, meter_y],
            120.0 * fit,
            fit,
            GREEN,
        );
        self.text(
            ctx,
            -190.0 * fit,
            meter_y + 18.0 * fit,
            "INPUT".into(),
            11.0 * fit,
            MUTED,
        );
        self.text(
            ctx,
            190.0 * fit,
            meter_y + 18.0 * fit,
            "OUTPUT".into(),
            11.0 * fit,
            MUTED,
        );
    }
}

pub fn map_mouse(button: baseview::MouseButton) -> Option<frost::MouseButton> {
    Some(match button {
        baseview::MouseButton::Left => frost::MouseButton::Left,
        baseview::MouseButton::Right => frost::MouseButton::Right,
        baseview::MouseButton::Middle => frost::MouseButton::Middle,
        _ => return None,
    })
}
