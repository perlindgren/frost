//! Shared standalone audio settings controller and frost dropdown panel.
use crate::dropdown::DropdownLayout;
use crate::{
    OutputChannels, Preferences, Settings, WindowGeom,
    devices::{self, Snapshot},
};
use crossbeam::channel::{Receiver, TryRecvError};
use frost::{Color, Context};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Backend,
    Input,
    InputCh,
    Output,
    OutputCh,
    InputBuffer,
    OutputBuffer,
    Rate,
    OutputRate,
    Extra(usize),
}
const AUDIO_ROWS: [(&str, Field); 9] = [
    ("Backend", Field::Backend),
    ("Input device", Field::Input),
    ("Input channel", Field::InputCh),
    ("Output device", Field::Output),
    ("Output channels", Field::OutputCh),
    ("Input buffer size", Field::InputBuffer),
    ("Output buffer size", Field::OutputBuffer),
    ("Input sample rate", Field::Rate),
    ("Output sample rate", Field::OutputRate),
];
/// Application controls appended below the shared audio rows.
#[derive(Clone)]
pub struct ExtraRow {
    pub label: String,
    pub options: Vec<(String, String)>,
    pub default: String,
    pub current: Option<String>,
}
pub enum Event {
    Back,
    Applied,
    ExtraPicked(usize, Option<String>),
    Reset,
}
struct Pending<P> {
    result: Receiver<Result<(), String>>,
    settings: Settings<P>,
}
pub struct AudioPanel<P> {
    pub draft: Settings<P>,
    pub applied: Settings<P>,
    pub saved: Settings<P>,
    pub snapshot: Snapshot,
    pub note: Option<String>,
    pub live_frames: Option<u32>,
    pub cold: bool,
    pending: Option<Pending<P>>,
    open_dropdown: Option<Field>,
    list_scroll: f32,
    dropdown_scroll_drag: bool,
    esc_held: bool,
    extras: Vec<ExtraRow>,
}
/// Current geometry for a pending save, without re-reading the configuration each frame.
pub fn window_geometry(
    size: [f32; 2],
    position: Option<(i32, i32)>,
    previous: Option<WindowGeom>,
) -> Option<WindowGeom> {
    let [width, height] = size;
    (width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0).then(|| WindowGeom {
        width,
        height,
        x: position.map(|p| p.0).or(previous.and_then(|p| p.x)),
        y: position.map(|p| p.1).or(previous.and_then(|p| p.y)),
    })
}
pub fn audio_equal<P>(a: &Settings<P>, b: &Settings<P>) -> bool {
    a.backend == b.backend
        && a.input_device == b.input_device
        && a.output_device == b.output_device
        && a.jack_output_right == b.jack_output_right
        && a.output_channels == b.output_channels
        && a.input_channel == b.input_channel
        && a.sample_rate == b.sample_rate
        && a.output_sample_rate == b.output_sample_rate
        && a.period_size == b.period_size
        && a.output_period_size == b.output_period_size
        && a.input_latency == b.input_latency
}
impl<P: Preferences + PartialEq> AudioPanel<P> {
    pub fn new(draft: Settings<P>, applied: Settings<P>, saved: Settings<P>, cold: bool) -> Self {
        Self {
            draft,
            applied,
            saved,
            snapshot: Snapshot::default(),
            note: None,
            live_frames: None,
            cold,
            pending: None,
            open_dropdown: None,
            list_scroll: 0.0,
            dropdown_scroll_drag: false,
            esc_held: false,
            extras: Vec::new(),
        }
    }
    pub fn is_applying(&self) -> bool {
        self.pending.is_some()
    }
    fn backend(&self) -> &str {
        self.draft.backend.as_deref().unwrap_or("dummy")
    }
    pub fn refresh(&mut self) {
        let input = self.draft.input_device.as_deref();
        let output = self.draft.output_device.as_deref();
        self.snapshot = if self.backend() == "asio" && devices::asio_stream_live() {
            devices::snapshot_via_child(self.backend(), input, output)
                .unwrap_or_else(|| self.snapshot.clone())
        } else {
            devices::snapshot(self.backend(), input, output)
        };
        let count = self.snapshot.input_channels;
        self.snapshot.default_input_channel = self
            .snapshot
            .default_input_channel
            .filter(|v| *v > 0 && count.is_some_and(|n| *v <= n));
        self.draft.input_channel = self
            .draft
            .input_channel
            .filter(|v| *v > 0 && count.is_some_and(|n| *v <= n));
        if self.backend() == "asio" {
            self.draft.output_device = self
                .draft
                .input_device
                .clone()
                .or(self.snapshot.def_in.clone());
        }
        self.sync_linked();
    }
    fn sync_linked(&mut self) {
        if self.snapshot.buffers_linked {
            self.draft.output_period_size = self.draft.period_size;
        }
        if self.snapshot.rates_linked {
            self.draft.output_sample_rate = self.draft.sample_rate;
        }
    }
    fn adopt_stream_defaults(&mut self) {
        self.draft.input_channel = self
            .draft
            .input_channel
            .or(self.snapshot.default_input_channel);
        self.draft.sample_rate = self.draft.sample_rate.or(self.snapshot.driver_rate);
        self.draft.output_sample_rate = self
            .draft
            .output_sample_rate
            .or(self.snapshot.default_output_rate);
        self.draft.period_size = self.draft.period_size.or(self.snapshot.buffer_pref);
        self.draft.output_period_size = self
            .draft
            .output_period_size
            .or(self.snapshot.output_buffer_pref);
        self.sync_linked();
    }
    fn buffer_default(&self, field: Field) -> Option<u32> {
        if self.backend() == "jack"
            && self.applied.backend.as_deref() == Some("jack")
            && !self.is_applying()
            && let Some(frames) = self.live_frames.filter(|v| *v > 0)
        {
            return Some(frames);
        }
        if field == Field::InputBuffer || self.snapshot.buffers_linked {
            self.snapshot.buffer_pref
        } else {
            self.snapshot.output_buffer_pref
        }
    }
    pub fn default_raw(&self, field: Field) -> String {
        match field {
            Field::Backend => None,
            Field::Input => self.snapshot.def_in.clone(),
            Field::Output => self.snapshot.def_out.clone(),
            Field::InputCh => self.snapshot.default_input_channel.map(|v| v.to_string()),
            Field::OutputCh if self.backend() == "jack" => None,
            Field::OutputCh => self.snapshot.default_output_channels.map(|v| v.raw()),
            Field::Rate => self
                .snapshot
                .driver_rate
                .or_else(|| {
                    self.draft
                        .sample_rate
                        .filter(|r| self.applied.sample_rate == Some(*r))
                })
                .map(rate),
            Field::OutputRate => self.snapshot.default_output_rate.map(rate),
            Field::InputBuffer | Field::OutputBuffer => {
                self.buffer_default(field).map(|v| v.to_string())
            }
            Field::Extra(i) => Some(self.extras[i].default.clone()),
        }
        .unwrap_or_default()
    }
    fn default_label(&self, field: Field) -> Option<String> {
        if field == Field::OutputCh && self.backend() != "jack" {
            return self.snapshot.default_output_channels.map(|v| v.label());
        }
        if matches!(field, Field::Rate | Field::OutputRate) {
            let raw = self.default_raw(field);
            return (!raw.is_empty()).then(|| format!("{raw} Hz"));
        }
        let raw = self.default_raw(field);
        (!raw.is_empty()).then_some(raw)
    }
    pub fn rows(&self, field: Field) -> (Vec<(String, String)>, String) {
        let s = &self.snapshot;
        if self.backend() == "jack" && field == Field::OutputCh {
            let left = self.draft.output_device.as_ref().or(s.def_out.as_ref());
            let mut rows = vec![("Mono (no right output)".into(), String::new())];
            rows.extend(
                s.outputs
                    .iter()
                    .filter(|p| Some(*p) != left)
                    .map(|v| (v.clone(), v.clone())),
            );
            return (rows, String::new());
        }
        let options = match field {
            Field::Backend => devices::available_backends()
                .into_iter()
                .map(|o| (o.label, o.id))
                .collect(),
            Field::Input | Field::Output => if field == Field::Input {
                &s.inputs
            } else {
                &s.outputs
            }
            .iter()
            .map(|v| (v.clone(), v.clone()))
            .collect(),
            Field::InputCh => (1..=s.input_channels.unwrap_or(0))
                .map(|v| (v.to_string(), v.to_string()))
                .collect(),
            Field::OutputCh => s
                .output_channel_options
                .iter()
                .map(|v| (v.label(), v.raw()))
                .collect(),
            Field::Rate | Field::OutputRate => if field == Field::Rate {
                &s.rates
            } else {
                &s.output_rates
            }
            .iter()
            .map(|v| (format!("{} Hz", rate(*v)), rate(*v)))
            .collect(),
            Field::InputBuffer | Field::OutputBuffer => if field == Field::InputBuffer {
                &s.buffers
            } else {
                &s.output_buffers
            }
            .iter()
            .map(|v| (v.to_string(), v.to_string()))
            .collect(),
            Field::Extra(i) => self.extras[i].options.clone(),
        };
        let raw = self.default_raw(field);
        let mut rows = vec![(self.default_label(field).unwrap_or("—".into()), raw.clone())];
        rows.extend(
            options
                .into_iter()
                .filter(|(_, value)| value.is_empty() || *value != raw),
        );
        (rows, raw)
    }
    fn current_raw(&self, field: Field) -> String {
        match field {
            Field::Backend => self.draft.backend.clone(),
            Field::Input => self.draft.input_device.clone(),
            Field::Output => self.draft.output_device.clone(),
            Field::InputCh => self.draft.input_channel.map(|v| v.to_string()),
            Field::OutputCh if self.backend() == "jack" => self.draft.jack_output_right.clone(),
            Field::OutputCh => self.draft.output_channels.map(|v| v.raw()),
            Field::Rate => self.draft.sample_rate.map(rate),
            Field::OutputRate => self.draft.output_sample_rate.map(rate),
            Field::InputBuffer => self.draft.period_size.map(|v| v.to_string()),
            Field::OutputBuffer => self.draft.output_period_size.map(|v| v.to_string()),
            Field::Extra(i) => self.extras[i].current.clone(),
        }
        .unwrap_or_else(|| self.default_raw(field))
    }
    fn value_text(&self, field: Field) -> String {
        let raw = if self.cold && !matches!(field, Field::Extra(_)) {
            // Cold settings remain unset until a user chooses a backend.
            if self.draft.backend.is_none() {
                return "—".into();
            }
            self.current_raw(field)
        } else {
            self.current_raw(field)
        };
        if matches!(field, Field::InputBuffer | Field::OutputBuffer)
            && self.backend() == "jack"
            && self.draft.period_size == self.applied.period_size
            && self.draft.output_period_size == self.applied.output_period_size
        {
            return self
                .buffer_default(field)
                .map_or("—".into(), |v| v.to_string());
        }

        self.rows(field)
            .0
            .into_iter()
            .find(|(_, v)| *v == raw)
            .map(|(label, _)| label)
            .unwrap_or(if raw.is_empty() { "—".into() } else { raw })
    }
    pub fn disabled(&self, field: Field) -> bool {
        self.is_applying()
            || (field == Field::InputCh && self.backend() == "jack")
            || (field == Field::Output && self.backend() == "asio")
            || (field == Field::OutputBuffer && self.snapshot.buffers_linked)
            || (field == Field::OutputRate && self.snapshot.rates_linked)
            || (matches!(field, Field::OutputCh | Field::OutputRate)
                && (self
                    .draft
                    .output_device
                    .as_ref()
                    .or(self.snapshot.def_out.as_ref())
                    .is_none()
                    || (field == Field::OutputCh
                        && self.backend() != "jack"
                        && self.snapshot.output_channel_options.is_empty())
                    || (field == Field::OutputRate && self.snapshot.output_rates.is_empty())))
    }
    pub fn pick(&mut self, field: Field, index: usize) -> Option<Event> {
        if self.disabled(field) {
            return None;
        }
        let (rows, default) = self.rows(field);
        let (_, raw) = rows.get(index)?.clone();
        let follow = raw.is_empty() || raw == default;
        let value = (!follow).then_some(raw.clone());
        self.note = None;
        match field {
            Field::Backend => {
                self.draft.backend = value;
                self.draft.input_device = None;
                self.draft.output_device = None;
                self.draft.input_channel = None;
                self.draft.jack_output_right = None;
                self.draft.output_channels = None;
                self.draft.sample_rate = None;
                self.draft.output_sample_rate = None;
                self.draft.period_size = None;
                self.draft.output_period_size = None;
                self.refresh();
                self.draft.input_device = self.snapshot.def_in.clone();
                self.draft.output_device = if self.backend() == "asio" {
                    self.draft.input_device.clone()
                } else {
                    self.snapshot.def_out.clone()
                };
                self.refresh();
                self.adopt_stream_defaults();
            }
            Field::Input => {
                self.draft.input_device = value;
                self.draft.input_channel = None;
                self.draft.period_size = None;
                self.draft.sample_rate = None;
                if self.backend() == "asio" {
                    self.draft.output_device = self.draft.input_device.clone();
                }
                self.refresh();
                self.adopt_stream_defaults();
            }
            Field::Output => {
                self.draft.output_device = value;
                self.draft.output_channels = None;
                self.draft.output_period_size = None;
                self.draft.output_sample_rate = None;
                if self.draft.jack_output_right.as_ref()
                    == self
                        .draft
                        .output_device
                        .as_ref()
                        .or(self.snapshot.def_out.as_ref())
                {
                    self.draft.jack_output_right = None;
                }
                self.refresh();
                self.adopt_stream_defaults();
            }
            Field::InputCh => self.draft.input_channel = value.and_then(|v| v.parse().ok()),
            Field::OutputCh if self.backend() == "jack" => self.draft.jack_output_right = value,
            Field::OutputCh => {
                self.draft.output_channels = value.and_then(|v| OutputChannels::parse(&v))
            }
            Field::Rate => self.draft.sample_rate = value.and_then(|v| v.parse().ok()),
            Field::OutputRate => self.draft.output_sample_rate = value.and_then(|v| v.parse().ok()),
            Field::InputBuffer => self.draft.period_size = value.and_then(|v| v.parse().ok()),
            Field::OutputBuffer => {
                self.draft.output_period_size = value.and_then(|v| v.parse().ok())
            }
            Field::Extra(i) => {
                self.open_dropdown = None;
                self.list_scroll = 0.0;
                return Some(Event::ExtraPicked(i, value));
            }
        }
        self.sync_linked();
        self.open_dropdown = None;
        self.list_scroll = 0.0;
        None
    }
    fn dropdown_items(&self, field: Field) -> Vec<String> {
        self.rows(field)
            .0
            .into_iter()
            .map(|(label, _)| label)
            .collect()
    }
    fn selected_index(&self, field: Field) -> usize {
        let raw = self.current_raw(field);
        self.rows(field)
            .0
            .iter()
            .position(|(_, v)| *v == raw)
            .unwrap_or(0)
    }
    fn save(&mut self, settings: Settings<P>, window: Option<WindowGeom>) -> Option<Event> {
        self.save_with(settings, window, Settings::save)
    }
    fn save_with(
        &mut self,
        mut settings: Settings<P>,
        window: Option<WindowGeom>,
        persist: impl FnOnce(&Settings<P>) -> Result<(), String>,
    ) -> Option<Event> {
        settings.window = window
            .or_else(|| Settings::<P>::load().0.window)
            .or(settings.window);
        match persist(&settings) {
            Ok(()) => {
                self.draft = settings.clone();
                self.applied = settings.clone();
                self.saved = settings;
                self.cold = false;
                self.note = None;
                Some(Event::Applied)
            }
            Err(error) => {
                self.note = Some(format!("Could not save settings: {error}"));
                None
            }
        }
    }
    pub fn poll(&mut self, window: Option<WindowGeom>) -> Option<Event> {
        self.poll_with_save(window, Settings::save)
    }
    fn poll_with_save(
        &mut self,
        window: Option<WindowGeom>,
        persist: impl FnOnce(&Settings<P>) -> Result<(), String>,
    ) -> Option<Event> {
        let result = match self.pending.as_ref()?.result.try_recv() {
            Ok(value) => value,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                Err("The audio worker stopped while applying settings".into())
            }
        };
        let pending = self.pending.take().unwrap();
        match result {
            Ok(()) => {
                let mut settings = pending.settings;
                settings.preferences = self.draft.preferences.clone();
                self.applied = settings.clone();
                self.save_with(settings, window, persist)
            }
            Err(e) => {
                self.note = Some(format!("Could not apply audio settings: {e}"));
                None
            }
        }
    }
    pub fn blocker(&self, available: bool) -> Option<String> {
        if self.is_applying() {
            Some("Applying audio settings…".into())
        } else if audio_equal(&self.applied, &self.draft) {
            None
        } else if !available {
            Some("Live audio configuration is unavailable".into())
        } else {
            self.snapshot.resolve_settings(&self.draft).err()
        }
    }
    /// Validate and begin an asynchronous Apply. No device work runs in this panel.
    pub fn apply(
        &mut self,
        window: Option<WindowGeom>,
        request: impl FnOnce(Settings<P>) -> Result<Receiver<Result<(), String>>, String>,
    ) -> Option<Event> {
        if self.is_applying() {
            return None;
        }
        if audio_equal(&self.applied, &self.draft) {
            return self.save(self.draft.clone(), window);
        }
        let result = self
            .snapshot
            .resolve_settings(&self.draft)
            .and_then(request);
        match result {
            Ok(result) => {
                self.pending = Some(Pending {
                    result,
                    settings: self.draft.clone(),
                })
            }
            Err(error) => self.note = Some(error),
        }
        None
    }

    /// Draw and interact with the shared panel. Application-specific controls
    /// emit events; device opening remains owned by the application's worker.
    #[allow(clippy::too_many_arguments)] // frame inputs plus application-owned Apply callback
    pub fn process(
        &mut self,
        ctx: &mut Context,
        font: &Arc<[u8]>,
        clicked: bool,
        extras: Vec<ExtraRow>,
        window: Option<WindowGeom>,
        available: bool,
        request: impl FnOnce(Settings<P>) -> Result<Receiver<Result<(), String>>, String>,
    ) -> Option<Event> {
        if self.backend() == "jack"
            && self.applied.backend.as_deref() == Some("jack")
            && !self.is_applying()
            && let Some(frames) = self.live_frames.filter(|v| *v > 0)
        {
            self.snapshot.buffer_pref = Some(frames);
            self.snapshot.output_buffer_pref = Some(frames);
        }
        self.extras = extras;
        if let Some(event) = self.poll(window) {
            return Some(event);
        }
        let (w, h) = ctx.size();
        let fit = (w / 720.0).min(h / 440.0).max(0.45);
        let mouse = ctx.mouse_position();
        let font = font.clone();
        let z = 0.4;
        let mut rows: Vec<(String, Field)> = AUDIO_ROWS
            .iter()
            .map(|(label, field)| (label.to_string(), *field))
            .collect();
        if self.backend() == "jack" {
            rows[1].0 = "Input source".into();
            rows[3].0 = "Output left / mono".into();
            rows[4].0 = "Output right".into();
        }
        rows.extend(
            self.extras
                .iter()
                .enumerate()
                .map(|(i, row)| (row.label.clone(), Field::Extra(i))),
        );
        let row_y = |i: usize| {
            h / 2.0 - 32.0 * fit - i as f32 * 24.0 * fit - if i >= 9 { 10.0 * fit } else { 0.0 }
        };
        let lx = -w / 2.0 + 18.0 * fit;
        let charw = ctx
            .text_width(&font, "0", 23.4 * fit, 550.0)
            .max(14.0 * fit);
        let bhw = 165.0 * fit + 3.0 * charw;
        let bx = w / 2.0 - 18.0 * fit - bhw;
        let bhh = 11.0 * fit;
        let esc = ctx.key_down(frost::KeyCode::Escape);
        if esc && !self.esc_held && !self.is_applying() {
            self.esc_held = true;
            if self.open_dropdown.take().is_some() {
                self.list_scroll = 0.0;
            } else {
                self.draft = self.applied.clone();
                return Some(Event::Back);
            }
        }
        self.esc_held = esc;
        for (i, (label, field)) in rows.iter().enumerate() {
            let y = row_y(i);
            let disabled = self.disabled(*field);
            let highlighted = !disabled
                && (self.open_dropdown == Some(*field) || point_in(mouse, bx, y, bhw, bhh));
            let lw = ctx.text_width(&font, label, 21.6 * fit, 550.0);
            ctx.text(lx + lw / 2.0, y, &font, label, 21.6 * fit, 550.0, DIM, z);
            ctx.rectangle(bx, y, bhw, bhh, if highlighted { HOVER } else { TRACK }, z);
            let value = self.value_text(*field);
            let color = if disabled {
                DIM
            } else if value != "—"
                && self.current_raw(*field) == self.default_raw(*field)
                && !self.default_raw(*field).is_empty()
            {
                GREEN
            } else if highlighted {
                AMBER
            } else {
                FG
            };
            ctx.text(bx, y, &font, value, 23.4 * fit, 550.0, color, z + 0.01);
        }
        let blocker = self.blocker(available);
        let dirty = (!audio_equal(&self.saved, &self.draft)
            || self.saved.preferences != self.draft.preferences
            || !audio_equal(&self.applied, &self.draft))
            && blocker.is_none();
        let by = -h / 2.0 + 30.0 * fit;
        let aw = (ctx.text_width(&font, "APPLY", 23.4 * fit, 700.0) + 26.0 * fit) / 2.0;
        let ax = w / 2.0 - 32.0 * fit - aw;
        let back = !self.is_applying()
            && point_in(mouse, -w / 2.0 + 90.0 * fit, by, 65.0 * fit, 15.0 * fit);
        let reset = !self.is_applying() && point_in(mouse, 0.0, by, 65.0 * fit, 15.0 * fit);
        let apply = dirty && point_in(mouse, ax, by, aw, 15.0 * fit);
        for (x, half, label, enabled, hover) in [
            (
                -w / 2.0 + 90.0 * fit,
                65.0 * fit,
                "BACK",
                !self.is_applying(),
                back,
            ),
            (0.0, 65.0 * fit, "RESET", !self.is_applying(), reset),
            (ax, aw, "APPLY", dirty, apply),
        ] {
            ctx.rectangle(
                x,
                by,
                half,
                15.0 * fit,
                if hover { HOVER } else { TRACK },
                z,
            );
            ctx.text(
                x,
                by,
                &font,
                label,
                23.4 * fit,
                650.0,
                if !enabled {
                    DIM
                } else if hover {
                    AMBER
                } else if label == "APPLY" {
                    GREEN
                } else {
                    FG
                },
                z + 0.01,
            );
        }
        if let Some(note) = self.note.as_ref().or(blocker.as_ref()) {
            ctx.text(
                0.0,
                by + 34.0 * fit,
                &font,
                clip(note, 90),
                21.6 * fit,
                550.0,
                AMBER,
                z,
            );
        }
        // Open dropdown: drawn last (on top), items hit first.
        if let Some(dd) = self.open_dropdown {
            let items = self.dropdown_items(dd);
            let row = rows.iter().position(|(_, d)| *d == dd).unwrap_or(0);
            let layout =
                DropdownLayout::new([w, h], [bx, row_y(row)], [bhw, bhh], fit, items.len());
            let max_scroll = items.len().saturating_sub(layout.visible) as f32;
            self.list_scroll = (self.list_scroll + ctx.mouse_wheel() * 0.5).clamp(0.0, max_scroll);
            if !ctx.mouse_button_down(frost::MouseButton::Left) {
                self.dropdown_scroll_drag = false;
            }
            let scrollable = max_scroll > 0.0;
            let thumb_half = if scrollable {
                (layout.track_half[1] * layout.visible as f32 / items.len() as f32)
                    .max(6.0 * fit)
                    .min(layout.track_half[1])
            } else {
                0.0
            };
            let travel = (2.0 * (layout.track_half[1] - thumb_half)).max(0.0);
            let over_track = scrollable
                && point_in(
                    mouse,
                    layout.track_x,
                    layout.center[1],
                    layout.track_half[0],
                    layout.track_half[1],
                );
            if clicked && over_track {
                self.dropdown_scroll_drag = true;
            }
            if self.dropdown_scroll_drag
                && scrollable
                && travel > 0.0
                && let Some([_, mouse_y]) = mouse
            {
                let fraction = ((layout.center[1] + layout.track_half[1] - thumb_half - mouse_y)
                    / travel)
                    .clamp(0.0, 1.0);
                self.list_scroll = (fraction * max_scroll).round();
            }
            let skip = self.list_scroll.floor() as usize;
            let [px, py] = layout.center;
            let [pw, ph] = layout.half;
            ctx.rectangle(px, py, pw, ph, HOVER, 0.44);
            ctx.rectangle(
                px,
                py,
                (pw - layout.border).max(0.0),
                (ph - layout.border).max(0.0),
                MENU_BG,
                0.445,
            );
            if scrollable {
                ctx.rectangle(
                    layout.track_x,
                    py,
                    layout.track_half[0],
                    layout.track_half[1],
                    TRACK,
                    0.45,
                );
                let thumb_y =
                    py + layout.track_half[1] - thumb_half - skip as f32 / max_scroll * travel;
                ctx.rectangle(
                    layout.track_x,
                    thumb_y,
                    layout.track_half[0],
                    thumb_half,
                    if over_track || self.dropdown_scroll_drag {
                        AMBER
                    } else {
                        DIM
                    },
                    0.46,
                );
            }
            let menu_size = (18.7 * fit).min(layout.item_half[1] * (18.7 / 13.0));
            let char_width = ctx.text_width(&font, "0", menu_size, 700.0).max(fit);
            let max_chars = ((2.0 * layout.item_half[0] - 4.0 * fit) / char_width)
                .floor()
                .max(1.0) as usize;
            let selected = self.selected_index(dd);
            for (slot, (idx, label)) in items
                .iter()
                .enumerate()
                .skip(skip)
                .take(layout.visible)
                .enumerate()
            {
                let iy = layout.row_y(slot);
                let ix = layout.item_x;
                let [iw, ih] = layout.item_half;
                let sel = selected == idx;
                let hov = point_in(mouse, ix, iy, iw, ih);
                ctx.rectangle(
                    ix,
                    iy,
                    iw,
                    ih,
                    if hov {
                        HOVER
                    } else if sel {
                        TRACK
                    } else {
                        Color {
                            a: 0.0,
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                        }
                    },
                    z + 0.05,
                );
                ctx.text(
                    ix,
                    iy,
                    &font,
                    if self.draft.backend.as_deref() == Some("jack")
                        && matches!(dd, Field::Input | Field::Output | Field::OutputCh)
                    {
                        clip_port(label, max_chars)
                    } else {
                        clip(label, max_chars)
                    },
                    menu_size,
                    if sel { 700.0 } else { 550.0 },
                    if idx == 0 && self.default_label(dd).is_some() {
                        GREEN
                    } else if sel || hov {
                        AMBER
                    } else {
                        FG
                    },
                    z + 0.06,
                );
                if clicked && hov {
                    return self.pick(dd, idx);
                }
            }
            if clicked && point_in(mouse, px, py, pw, ph) {
                return None; // The popup surface consumes clicks between its rows too.
            }
        }

        if clicked {
            if self.open_dropdown.take().is_some() {
                self.list_scroll = 0.0;
            } else {
                for (i, (_, field)) in rows.iter().enumerate() {
                    if !self.disabled(*field) && point_in(mouse, bx, row_y(i), bhw, bhh) {
                        if self.backend() == "jack"
                            && matches!(field, Field::Input | Field::Output | Field::OutputCh)
                        {
                            self.refresh();
                        }
                        self.open_dropdown = Some(*field);
                        self.list_scroll = self.selected_index(*field) as f32;
                        self.dropdown_scroll_drag = false;
                        return None;
                    }
                }
                if back {
                    self.draft = self.applied.clone();
                    return Some(Event::Back);
                }
                if reset {
                    let window = self.draft.window;
                    self.draft = Settings {
                        window,
                        ..Default::default()
                    };
                    self.refresh();
                    return Some(Event::Reset);
                }
                if apply {
                    return self.apply(window, request);
                }
            }
        }
        None
    }
}
fn rate(value: f32) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}
fn point_in(p: Option<[f32; 2]>, x: f32, y: f32, w: f32, h: f32) -> bool {
    p.is_some_and(|[px, py]| (px - x).abs() <= w && (py - y).abs() <= h)
}
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        text.chars()
            .take(max.saturating_sub(1))
            .chain(['…'])
            .collect()
    } else {
        text.into()
    }
}
const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}
const FG: Color = rgb(235, 230, 241);
const DIM: Color = rgb(170, 164, 183);
const GREEN: Color = rgb(103, 237, 175);
const AMBER: Color = rgb(250, 190, 115);
const TRACK: Color = rgb(65, 57, 82);
const HOVER: Color = rgb(83, 73, 106);
const MENU_BG: Color = rgb(45, 39, 59);

fn clip_port(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        return s.to_owned();
    }
    // Keep the channel/port suffix visible, where JACK names usually differ.
    let head = n / 3;
    let tail = n.saturating_sub(head + 1);
    format!(
        "{}…{}",
        chars[..head].iter().collect::<String>(),
        chars[chars.len() - tail..].iter().collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::NoPreferences;
    fn panel() -> AudioPanel<NoPreferences> {
        let mut panel = AudioPanel::new(
            Settings::default(),
            Settings::default(),
            Settings::default(),
            false,
        );
        panel.draft.backend = Some("core-audio".into());
        panel.snapshot = Snapshot {
            input_channels: Some(2),
            default_input_channel: Some(1),
            driver_rate: Some(48000.0),
            default_output_rate: Some(44100.0),
            rates: vec![44100.0, 48000.0],
            output_rates: vec![44100.0, 48000.0],
            buffers: vec![32, 64, 128],
            output_buffers: vec![32, 64, 128],
            buffer_pref: Some(64),
            output_buffer_pref: Some(128),
            outputs: vec!["Speakers".into()],
            def_out: Some("Speakers".into()),
            output_channel_options: vec![OutputChannels::Mono(1), OutputChannels::Stereo([1, 2])],
            default_output_channels: Some(OutputChannels::Stereo([1, 2])),
            ..Default::default()
        };
        panel
    }
    #[test]
    fn default_row_is_first_unique_and_follow_is_stored_as_none() {
        let mut p = panel();
        assert_eq!(
            p.rows(Field::InputCh).0,
            vec![("1".into(), "1".into()), ("2".into(), "2".into())]
        );
        p.pick(Field::InputCh, 1);
        assert_eq!(p.draft.input_channel, Some(2));
        p.pick(Field::InputCh, 0);
        assert_eq!(p.draft.input_channel, None);
        p.snapshot.default_input_channel = None;
        assert_eq!(p.rows(Field::InputCh).0[0].0, "—");
        p.snapshot.input_channels = None;
        assert_eq!(p.rows(Field::InputCh).0.len(), 1);
    }
    #[test]
    fn independent_output_defaults_and_linked_controls_follow_the_same_rules() {
        let mut p = panel();
        assert_eq!(p.default_raw(Field::Rate), "48000");
        assert_eq!(p.default_raw(Field::OutputRate), "44100");
        p.pick(Field::InputBuffer, 1); // 32; default 64 is not duplicated
        assert_eq!(p.draft.period_size, Some(32));
        assert_eq!(p.draft.output_period_size, None);
        p.snapshot.buffers_linked = true;
        p.pick(Field::InputBuffer, 2);
        assert_eq!(p.draft.period_size, p.draft.output_period_size);
        assert!(p.disabled(Field::OutputBuffer));
        p.snapshot.rates_linked = true;
        p.pick(Field::Rate, 1);
        assert_eq!(p.draft.sample_rate, p.draft.output_sample_rate);
        assert!(p.disabled(Field::OutputRate));
        p.draft.backend = Some("asio".into());
        assert!(p.disabled(Field::Output));
    }
    #[test]
    fn device_defaults_are_adopted_without_inventing_channel_or_route_defaults() {
        let mut p = panel();
        p.adopt_stream_defaults();
        assert_eq!(p.draft.input_channel, Some(1));
        assert_eq!(p.draft.period_size, Some(64));
        assert_eq!(p.draft.output_period_size, Some(128));
        p.snapshot.default_input_channel = None;
        p.draft.input_channel = None;
        p.adopt_stream_defaults();
        assert_eq!(p.draft.input_channel, None);
    }
    #[test]
    fn failed_and_disconnected_apply_keep_the_running_configuration() {
        let mut p = panel();
        p.draft.backend = Some("dummy".into());
        let original = p.applied.clone();
        let (sender, reply) = crossbeam::channel::bounded(1);
        p.apply(None, |_| Ok(reply));
        assert!(p.is_applying());
        assert!(p.disabled(Field::Backend));
        sender
            .send(Err("device refused the format".into()))
            .unwrap();
        assert!(p.poll(None).is_none());
        assert!(!p.is_applying());
        assert_eq!(p.applied, original);
        assert!(p.note.as_deref().unwrap().contains("device refused"));
        let (sender, reply) = crossbeam::channel::bounded(1);
        p.apply(None, |_| Ok(reply));
        drop(sender);
        p.poll(None);
        assert!(!p.is_applying());
        assert_eq!(p.applied, original);
        assert!(p.note.as_deref().unwrap().contains("worker stopped"));
    }
    #[test]
    fn jack_named_right_output_excludes_left_and_live_frames_win() {
        let mut p = panel();
        p.draft.backend = Some("jack".into());
        p.applied.backend = Some("jack".into());
        p.draft.output_device = Some("left".into());
        p.snapshot.outputs = vec!["left".into(), "right".into()];
        p.live_frames = Some(256);
        assert_eq!(p.default_raw(Field::InputBuffer), "256");
        assert_eq!(p.rows(Field::OutputCh).0.len(), 2);
        p.pick(Field::OutputCh, 1);
        assert_eq!(p.draft.jack_output_right.as_deref(), Some("right"));
        p.pick(Field::OutputCh, 0);
        assert_eq!(p.draft.jack_output_right, None);
        assert!(p.disabled(Field::InputCh));
    }
    #[test]
    fn geometry_does_not_dirty_audio_and_app_preferences_remain_owned_by_the_app() {
        let mut p = panel();
        p.draft.window = Some(WindowGeom {
            x: None,
            y: None,
            width: 900.0,
            height: 620.0,
        });
        let mut saved = p.draft.clone();
        saved.window = None;
        assert!(audio_equal(&p.draft, &saved));
        p.extras = vec![ExtraRow {
            label: "Preference".into(),
            default: "3".into(),
            current: None,
            options: vec![("4".into(), "4".into())],
        }];
        assert!(
            matches!(p.pick(Field::Extra(0), 1), Some(Event::ExtraPicked(0, Some(value))) if value == "4")
        );
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[derive(Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    struct App {
        value: Option<u32>,
    }
    impl Preferences for App {
        const FILE_NAME: &'static str = "nice-settings-ui-tests.json";
        const APP_NAME: &'static str = "test";
        fn is_blank(&self) -> bool {
            self.value.is_none()
        }
    }
    #[test]
    fn apply_preserves_latest_preferences_and_geometry_and_keeps_audio_on_save_failure() {
        let mut p = AudioPanel::<App>::new(
            Settings::default(),
            Settings::default(),
            Settings::default(),
            true,
        );
        p.draft.backend = Some("dummy".into());
        p.draft.preferences.value = Some(1);
        let (sender, reply) = crossbeam::channel::bounded(1);
        p.apply(None, |_| Ok(reply));
        p.draft.preferences.value = Some(2); // an off-thread model load completed
        sender.send(Ok(())).unwrap();
        let window = WindowGeom {
            x: None,
            y: None,
            width: 900.0,
            height: 620.0,
        };
        assert!(
            p.poll_with_save(Some(window), |settings| {
                assert_eq!(settings.preferences.value, Some(2));
                assert_eq!(settings.window, Some(window));
                Err("disk is read-only".into())
            })
            .is_none()
        );
        assert_eq!(p.applied.backend.as_deref(), Some("dummy"));
        assert_eq!(p.saved.backend, None);
        assert!(p.note.as_deref().unwrap().contains("read-only"));
        assert!(matches!(
            p.save_with(p.draft.clone(), Some(window), |_| Ok(())),
            Some(Event::Applied)
        ));
        assert_eq!(p.saved.preferences.value, Some(2));
        assert_eq!(p.applied.preferences.value, Some(2));
        assert!(!p.cold);
    }
    #[test]
    fn preference_only_changes_do_not_require_a_device_or_an_audio_worker() {
        let mut p = AudioPanel::<App>::new(
            Settings::default(),
            Settings::default(),
            Settings::default(),
            true,
        );
        p.draft.preferences.value = Some(1);
        assert!(p.blocker(false).is_none());
        assert!(matches!(
            p.save_with(p.draft.clone(), None, |_| Ok(())),
            Some(Event::Applied)
        ));
        assert_eq!(p.applied.preferences.value, Some(1));
    }
}
