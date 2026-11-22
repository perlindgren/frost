use crate::controls::Variant;
use crate::{DistParams, DistSharedState};
use crossbeam::atomic::AtomicCell;
use frost::{Color, Context, Process, Scene, SceneNode, Shape};
use nice_common::dropdown::{Placement, draw_dropdown};
use nice_common::knob;
use nice_common::meter::PeakMeter;
use nice_common::oversample::Oversampling;
use nice_plug::context::gui::GuiContext;
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[cfg(feature = "standalone")]
use crate::settings::{DistPreferences, Settings};
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
    Drive,
    Tone,
    Output,
}
impl Knob {
    const ALL: [Self; 3] = [Self::Drive, Self::Tone, Self::Output];
    fn label(self, model: Variant) -> &'static str {
        if model == Variant::Clone {
            return match self {
                Self::Drive => "GAIN",
                Self::Tone => "TREBLE",
                Self::Output => "OUTPUT",
            };
        }
        if model == Variant::Cat {
            return match self {
                Self::Drive => "DISTORTION",
                Self::Tone => "FILTER",
                Self::Output => "VOLUME",
            };
        }
        match self {
            Self::Drive => "DRIVE",
            Self::Tone => "TONE",
            Self::Output => "LEVEL",
        }
    }
    fn param(self, params: &DistParams) -> &nice_plug::prelude::FloatParam {
        match self {
            Self::Drive => &params.drive,
            Self::Tone => &params.tone,
            Self::Output => &params.output,
        }
    }
    fn cell(self, shared: &DistSharedState) -> &AtomicCell<f32> {
        match self {
            Self::Drive => &shared.drive,
            Self::Tone => &shared.tone,
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
    Variant,
    Oversampling,
}

#[derive(Clone, Copy)]
enum Drag {
    Knob(Knob),
}

pub struct DistUi {
    shared: Arc<DistSharedState>,
    font: Arc<[u8]>,
    icon_font: Arc<[u8]>,
    params: Option<Arc<DistParams>>,
    context: Option<GuiContext>,
    input_meter: PeakMeter,
    output_meter: PeakMeter,
    was_down: bool,
    drag: Option<Drag>,
    drag_anchor: f32,
    drag_start: f32,
    armed: Option<Hover>,
    variant_open: bool,
    variant_scroll: f32,
    variant_dragging: bool,
    settings_page: bool,
    #[cfg(feature = "standalone")]
    message: String,
    #[cfg(feature = "standalone")]
    pressed: bool,
    #[cfg(feature = "standalone")]
    applied: Settings,
    #[cfg(feature = "standalone")]
    panel: Option<AudioPanel<DistPreferences>>,
}

impl DistUi {
    pub fn new(shared: Arc<DistSharedState>, font: Arc<[u8]>) -> Self {
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
            variant_open: false,
            variant_scroll: 0.0,
            variant_dragging: false,
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

    pub fn set_params(&mut self, params: Arc<DistParams>) {
        self.params = Some(params);
    }

    pub fn set_gui_context(&mut self, context: GuiContext) {
        self.context = Some(context);
    }

    fn text(&self, ctx: &mut Context, x: f32, y: f32, text: String, size: f32, color: Color) {
        ctx.text(x, y, &self.font, text, size, TEXT_WEIGHT, color, 0.5);
    }

    fn knob_value(&self, params: &DistParams, knob: Knob) -> f32 {
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            knob.cell(&self.shared).load()
        } else {
            knob.param(params).value()
        }
    }

    fn variant(&self, params: &DistParams) -> Variant {
        Variant::from_index(if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            self.shared.variant.load()
        } else {
            params.variant.value()
        })
    }

    fn oversampling(&self, params: &DistParams) -> Oversampling {
        Oversampling::from_index(if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            self.shared.oversampling.load()
        } else {
            params.oversampling.value()
        })
    }
    fn cycle_oversampling(&mut self, params: &DistParams) {
        let mode = Oversampling::ALL[(self.oversampling(params).index() as usize + 1) % 4];
        if let Some(context) = &self.context {
            let setter = context.param_setter();
            setter.begin_set_parameter(&params.oversampling);
            setter.set_parameter(&params.oversampling, mode.index());
            setter.end_set_parameter(&params.oversampling);
        }
        self.shared.oversampling.store(mode.index());
        self.save_preferences();
    }
    fn set_variant(&mut self, params: &DistParams, mode: Variant) {
        if let Some(context) = &self.context {
            let setter = context.param_setter();
            setter.begin_set_parameter(&params.variant);
            setter.set_parameter(&params.variant, mode.index());
            setter.end_set_parameter(&params.variant);
        }
        self.shared.variant.store(mode.index());
        self.save_preferences();
    }

    fn save_preferences(&mut self) {
        #[cfg(feature = "standalone")]
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            let shared = &self.shared;
            let mut settings = Settings::load().0;
            settings.drive = Some(shared.drive.load());
            settings.tone = Some(shared.tone.load());
            settings.output = Some(shared.output.load());
            settings.oversampling =
                Some(Oversampling::from_index(shared.oversampling.load()).factor() as u32);
            settings.variant = Some(Variant::from_index(shared.variant.load()).number());
            if let Err(error) = settings.save() {
                self.message = error;
            }
        }
    }

    /// Device-independent settings are available inside a plugin host too.
    fn draw_plugin_settings(&mut self, ctx: &mut Context) {
        let Some(params) = self.params.clone() else {
            return;
        };
        let (w, h) = ctx.size();
        let fit = (w / 860.0).min(h / 440.0).clamp(0.45, 1.5);
        let mouse = ctx.mouse_position();
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let pressed = down && !self.was_down;
        self.was_down = down;
        let was_open = self.variant_open;
        let at = [60.0 * fit, 25.0 * fit];
        let half = [80.0 * fit, 16.0 * fit];
        let inside = |at: [f32; 2], half: [f32; 2]| {
            mouse
                .is_some_and(|p| (p[0] - at[0]).abs() <= half[0] && (p[1] - at[1]).abs() <= half[1])
        };
        self.text(
            ctx,
            0.0,
            h / 2.0 - 45.0 * fit,
            "NICE DIST SETTINGS".into(),
            23.0 * fit,
            TEXT,
        );
        self.text(ctx, -100.0 * fit, at[1], "Model".into(), 14.0 * fit, MUTED);
        ctx.rectangle(at[0], at[1], half[0], half[1], knob::PANEL, 0.48);
        self.text(
            ctx,
            at[0],
            at[1],
            format!("{} ▾", self.variant(&params).label()),
            16.0 * fit,
            knob::AMBER,
        );
        self.text(
            ctx,
            0.0,
            -30.0 * fit,
            "BS808 / BS9 / OH-1 / SH-1 / CAT / CLONE — oversampled circuit models.".into(),
            12.0 * fit,
            MUTED,
        );
        let back = [0.0, -h / 2.0 + 40.0 * fit];
        ctx.rectangle(back[0], back[1], 55.0 * fit, 16.0 * fit, knob::PANEL, 0.48);
        self.text(ctx, back[0], back[1], "BACK".into(), 14.0 * fit, TEXT);
        if pressed && !was_open {
            if inside(at, half) {
                self.variant_open = true;
            }
            if inside(back, [55.0 * fit, 16.0 * fit]) {
                self.settings_page = false;
            }
        }
        if self.variant_open {
            let items: Vec<_> = Variant::ALL
                .iter()
                .map(|mode| (mode.label().to_owned(), mode.index().to_string()))
                .collect();
            let response = draw_dropdown(
                ctx,
                &self.font,
                [w, h],
                fit,
                Placement::Row(at, half),
                0.9,
                pressed && was_open,
                &items,
                Variant::ALL
                    .iter()
                    .position(|v| *v == self.variant(&params))
                    .unwrap_or(0),
                "",
                "Model",
                false,
                &mut self.variant_scroll,
                &mut self.variant_dragging,
            );
            if let Some(index) = response.picked {
                self.set_variant(&params, Variant::ALL[index]);
                self.variant_open = false;
            } else if pressed && was_open && !response.consumed {
                self.variant_open = false;
            }
        }
        if ctx.key_down(frost::KeyCode::Escape) {
            if was_open {
                self.variant_open = false;
            } else {
                self.settings_page = false;
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

impl DistUi {
    fn set_value(&self, params: &DistParams, kind: Knob, value: f32) {
        if let Some(context) = &self.context {
            context
                .param_setter()
                .set_parameter(kind.param(params), value);
        }
        kind.cell(&self.shared).store(value);
    }
}

impl Process for DistUi {
    fn process(&mut self, ctx: &mut Context, _dt: f32) {
        #[cfg(feature = "standalone")]
        if let Some(error) = crate::standalone::control::take_error() {
            self.message = error;
            self.applied = Settings::default();
            self.panel = None;
        }
        if self.settings_page {
            #[cfg(feature = "standalone")]
            if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
                self.draw_settings_page(ctx);
                return;
            }
            self.draw_plugin_settings(ctx);
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
        let variant_at = [80.0 * fit, h / 2.0 - 85.0 * fit];
        let variant_half = [60.0 * fit, 15.0 * fit];
        let menu_was_open = self.variant_open;
        let mouse = ctx.mouse_position();
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let pressed = down && !self.was_down;
        let released = !down && self.was_down;
        let fine =
            ctx.key_down(frost::KeyCode::ShiftLeft) || ctx.key_down(frost::KeyCode::ShiftRight);
        let mut hover = None;
        if let Some(at) = mouse
            && !menu_was_open
        {
            for (i, kind) in Knob::ALL.iter().enumerate() {
                if knob::contains([x(i), y], radius, at) {
                    hover = Some(Hover::Knob(*kind));
                }
            }
            if (at[0] - cog_at[0]).abs() < 20.0 * fit && (at[1] - cog_at[1]).abs() < 20.0 * fit {
                hover = Some(Hover::Cog);
            }
        }
        if !menu_was_open
            && mouse.is_some_and(|at| {
                (at[0] - variant_at[0]).abs() <= variant_half[0]
                    && (at[1] - variant_at[1]).abs() <= variant_half[1]
            })
        {
            hover = Some(Hover::Variant);
        }
        if !menu_was_open
            && mouse.is_some_and(|at| {
                (at[0] + 260.0 * fit).abs() <= 65.0 * fit
                    && (at[1] - variant_at[1]).abs() <= 15.0 * fit
            })
        {
            hover = Some(Hover::Oversampling);
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
            let clicked = self.armed.take().filter(|armed| Some(*armed) == hover);
            if clicked == Some(Hover::Oversampling) {
                self.cycle_oversampling(&params);
            }
            if clicked == Some(Hover::Variant) {
                self.variant_open = true;
            }
            if clicked == Some(Hover::Cog) {
                self.settings_page = true;
                self.variant_open = false;
                #[cfg(feature = "standalone")]
                {
                    self.panel = None;
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
            "NICE DIST".into(),
            25.0 * fit,
            TEXT,
        );
        self.text(
            ctx,
            0.0,
            -h / 2.0 + 28.0 * fit,
            "GERMANIUM OVERDRIVE".into(),
            12.0 * fit,
            MUTED,
        );
        {
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
        self.text(
            ctx,
            -70.0 * fit,
            variant_at[1],
            "MODEL".into(),
            12.0 * fit,
            MUTED,
        );
        ctx.rectangle(
            variant_at[0],
            variant_at[1],
            variant_half[0],
            variant_half[1],
            knob::PANEL,
            0.48,
        );
        self.text(
            ctx,
            variant_at[0],
            variant_at[1],
            format!("{} ▾", self.variant(&params).label()),
            14.0 * fit,
            if hover == Some(Hover::Variant) || self.variant_open {
                knob::AMBER
            } else {
                TEXT
            },
        );
        ctx.rectangle(
            -260.0 * fit,
            variant_at[1],
            65.0 * fit,
            15.0 * fit,
            knob::PANEL,
            0.48,
        );
        self.text(
            ctx,
            -260.0 * fit,
            variant_at[1],
            format!("OS {} ↻", self.oversampling(&params).label()),
            13.0 * fit,
            TEXT,
        );
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
                kind.label(self.variant(&params)),
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
        if hover == Some(Hover::Variant) && !self.variant_open {
            let model = self.variant(&params);
            nice_common::tooltip::draw(
                ctx,
                &self.font,
                &format!(
                    "{}: inspired by {}. Independent; no affiliation.",
                    model.label(),
                    model.inspiration()
                ),
                fit,
            );
        }
        // Shared selector draws last and owns all pointer/wheel input while open.
        if self.variant_open {
            let items: Vec<_> = Variant::ALL
                .iter()
                .map(|mode| (mode.label().to_owned(), mode.index().to_string()))
                .collect();
            let response = draw_dropdown(
                ctx,
                &self.font,
                [w, h],
                fit,
                Placement::Row(variant_at, variant_half),
                0.9,
                pressed,
                &items,
                Variant::ALL
                    .iter()
                    .position(|v| *v == self.variant(&params))
                    .unwrap_or(0),
                "",
                "Model",
                false,
                &mut self.variant_scroll,
                &mut self.variant_dragging,
            );
            if let Some(index) = response.picked {
                self.set_variant(&params, Variant::ALL[index]);
                self.variant_open = false;
            } else if (pressed && !response.consumed) || ctx.key_down(frost::KeyCode::Escape) {
                self.variant_open = false;
            }
        }
    }
}
