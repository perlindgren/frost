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
    SecondDevice,
    SecondOutput,
    InputBuffer,
    OutputBuffer,
    Rate,
    OutputRate,
    Extra(usize),
}
const AUDIO_ROWS: [(&str, Field); 11] = [
    ("Backend", Field::Backend),
    ("Input device", Field::Input),
    ("Input channel", Field::InputCh),
    ("Output device", Field::Output),
    ("Output channels", Field::OutputCh),
    ("Second device", Field::SecondDevice),
    ("Second output", Field::SecondOutput),
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
    /// How far up the second driver's own outputs a copy can reach, which is the
    /// one question this page asks that driver: its channels are not the first
    /// driver's, so a copy riding it is measured on it. `None` means it has not
    /// answered, which is not the same as having no channels.
    second_reach: Option<u32>,
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
        && a.asio_second_device == b.asio_second_device
        && a.asio_second_output == b.asio_second_output
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
            second_reach: None,
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
    /// Only audio settings belong to this panel's Apply action. App preferences
    /// are controlled live by the main view and must not enable Apply.
    pub fn audio_dirty(&self) -> bool {
        !audio_equal(&self.saved, &self.draft) || !audio_equal(&self.applied, &self.draft)
    }

    fn cancel_audio_draft(&mut self) {
        let preferences = self.draft.preferences.clone();
        self.draft = self.applied.clone();
        self.draft.preferences = preferences;
    }

    fn reset_audio_draft(&mut self) {
        self.draft = Settings {
            window: self.draft.window,
            preferences: self.draft.preferences.clone(),
            ..Default::default()
        };
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
            // A second driver named that is the driver the copy is copied from,
            // which is what naming nothing means.
            self.same_driver_is_no_driver();
            self.ask_second_reach();
            self.prune_second_output();
        } else {
            self.second_reach = None;
        }
        self.sync_linked();
    }
    /// Write the copy's carrier back the short way: the row offers "Same as
    /// output" and a list of other drivers, and a file naming this one says the
    /// first of those in the second row's words.
    fn same_driver_is_no_driver(&mut self) {
        if self
            .draft
            .asio_second_device
            .as_deref()
            .is_some_and(|driver| Some(driver) == self.draft.output_device.as_deref())
        {
            self.draft.asio_second_device = None;
        }
    }
    /// The driver the copy rides on, or `None` when it rides the one carrying the
    /// live stream. Only a name other than the output row's is another driver.
    fn copy_driver(&self) -> Option<&str> {
        let driver = self.draft.asio_second_device.as_deref()?;
        (Some(driver) != self.draft.output_device.as_deref()).then_some(driver)
    }
    /// Ask the driver the copy rides on how many output channels it has, as its
    /// own output and not as anybody's input. A second snapshot is what that
    /// question costs, and it is a question the first snapshot cannot answer: the
    /// channels of the live driver are not the ones the copy lands on.
    fn ask_second_reach(&mut self) {
        let Some(driver) = self.copy_driver().map(str::to_owned) else {
            self.second_reach = None;
            return;
        };
        let snapshot = if devices::asio_stream_live() {
            devices::snapshot_via_child(self.backend(), None, Some(&driver))
        } else {
            Some(devices::snapshot(self.backend(), None, Some(&driver)))
        };
        self.second_reach = snapshot.and_then(|snapshot| Self::reach(&snapshot));
    }
    /// How far up channels a copy can reach, counted on whichever driver carries
    /// it: the second one's own channels when there is a second one.
    fn copy_reach(&self) -> Option<u32> {
        if self.copy_driver().is_some() {
            self.second_reach
        } else {
            self.output_reach()
        }
    }
    /// Drop a second output start the carrier cannot host: a row offering a span
    /// the driver does not have is not a choice, and one landing on the first
    /// group would copy nothing. A driver that will not say how wide it is gets no
    /// verdict here — this page is beyond knowing, and the driver refuses the open
    /// in its own words when the channels are absent.
    fn prune_second_output(&mut self) {
        let width = self.output_width();
        let first = if self.copy_driver().is_some() {
            1
        } else {
            width + 1
        };
        let keep = self.draft.asio_second_output.is_some_and(|start| {
            self.copy_reach().is_none_or(|reach| {
                start >= first && start.checked_add(width - 1).is_some_and(|end| end <= reach)
            })
        });
        if !keep {
            self.draft.asio_second_output = None;
        }
    }
    /// The width of the first output group, which is what a second one copies:
    /// the route as chosen, or as the device reported it, or one while neither is
    /// known.
    fn output_width(&self) -> u32 {
        self.draft
            .output_channels
            .or(self.snapshot.default_output_channels)
            .map_or(1, |route| route.required_channels().max(1))
    }
    /// How far up the live driver's output channels a stream can reach.
    fn output_reach(&self) -> Option<u32> {
        Self::reach(&self.snapshot)
    }
    /// How far up the outputs of one snapshot's device a stream can reach, which
    /// is the same question as how many output channels it has. `None` is a device
    /// that has not answered, not a device with none.
    fn reach(snapshot: &Snapshot) -> Option<u32> {
        snapshot
            .output_formats
            .iter()
            .map(|format| format.channels)
            .max()
            .filter(|&channels| channels > 0)
    }
    /// The settings page as rows: the shared audio rows, with the two ASIO-only
    /// copy rows only where they apply, then the application's own rows.
    /// The second value is how many rows are audio, which is where the gap in
    /// front of the application rows belongs.
    fn page_rows(&self) -> (Vec<(String, Field)>, usize) {
        let mut rows: Vec<(String, Field)> = AUDIO_ROWS
            .iter()
            // The copy is an ASIO idea: no other backend shares one device
            // whose channels can be handed out twice, nor has a second driver
            // this host could load beside the first.
            .filter(|(_, field)| {
                !matches!(field, Field::SecondDevice | Field::SecondOutput)
                    || self.backend() == "asio"
            })
            .map(|(label, field)| (label.to_string(), *field))
            .collect();
        if self.backend() == "jack" {
            rows[1].0 = "Input source".into();
            rows[3].0 = "Output left / mono".into();
            rows[4].0 = "Output right".into();
        }
        let audio = rows.len();
        rows.extend(
            self.extras
                .iter()
                .enumerate()
                .map(|(i, row)| (row.label.clone(), Field::Extra(i))),
        );
        (rows, audio)
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
        // Persist the concrete output route reported for this device. It is
        // an actual channel configuration, even when it matches the default.
        // JACK routes through named ports instead of physical channel indices.
        if self.backend() != "jack" {
            self.draft.output_channels = self
                .draft
                .output_channels
                .or(self.snapshot.default_output_channels);
        }
        self.sync_linked();
    }
    /// The drivers a copy can ride on: every ASIO driver this machine has apart
    /// from the one the Output row already names, because naming that one is what
    /// the leading "Same as output" row means.
    fn second_device_rows(&self) -> Vec<(String, String)> {
        let output = self
            .draft
            .output_device
            .as_deref()
            .or(self.snapshot.def_out.as_deref());
        self.snapshot
            .outputs
            .iter()
            .filter(|driver| Some(driver.as_str()) != output)
            .map(|driver| (driver.clone(), driver.clone()))
            .collect()
    }
    /// Every start whose copy fits: the channels the carrier has after the first
    /// group, each labelled with the span it lands on — "3–4" for a route reaching
    /// channel 2, and plain "3" for a mono one, because a span of one is not a
    /// span. On the driver carrying the live stream the first group is in the way,
    /// so the choices start behind it; on a driver of its own, channel 1 is as
    /// good as any.
    fn second_output_rows(&self) -> Vec<(String, String)> {
        let width = self.output_width();
        let Some(tail) = self.copy_reach().and_then(|reach| reach.checked_sub(width)) else {
            return Vec::new();
        };
        let first = if self.copy_driver().is_some() {
            1
        } else {
            width + 1
        };
        let last = tail + 1;
        if last < first {
            return Vec::new();
        }
        (first..=last)
            .map(|start| {
                (
                    if width == 1 {
                        start.to_string()
                    } else {
                        format!("{start}–{}", start + width - 1)
                    },
                    start.to_string(),
                )
            })
            .collect()
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
            // One output group is the default everywhere: a second one is a
            // choice, not something a device reports. The same for the driver it
            // rides on, which is the one carrying the first group until told.
            Field::SecondDevice | Field::SecondOutput => None,
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
        if field == Field::SecondOutput {
            // The empty value means one output group. "Off" says that better
            // than the dash this panel otherwise uses for "no default".
            return Some("Off".into());
        }
        if field == Field::SecondDevice {
            // And the empty value there means the driver the first group is
            // already on, which is the whole alternative to choosing.
            return Some("Same as output".into());
        }
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
            Field::SecondDevice => self.second_device_rows(),
            Field::SecondOutput => self.second_output_rows(),
            Field::Extra(i) => self.extras[i].options.clone(),
        };
        // The default row keeps its natural place in the snapshot order: a
        // dropdown must show its alternatives in the order they were built,
        // never with the current default hoisted to the top. Only a default
        // that no option carries — a server period outside the fixed
        // choices, or the empty "—" row meaning "no default" — has no place
        // to sit, and then it leads the list as an extra row.
        let raw = self.default_raw(field);
        let label = self.default_label(field);
        let home = (!raw.is_empty())
            .then(|| options.iter().position(|(_, value)| *value == raw))
            .flatten();
        let mut rows = Vec::with_capacity(options.len() + 1);
        if let Some(at) = home {
            for (index, (option_label, value)) in options.into_iter().enumerate() {
                if index != at && value == raw {
                    continue; // one row per value, as the old filter gave
                }
                rows.push(if index == at {
                    (label.clone().unwrap_or(option_label), value)
                } else {
                    (option_label, value)
                });
            }
        } else {
            rows.push((label.unwrap_or_else(|| "—".into()), raw.clone()));
            rows.extend(options);
        }
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
            Field::SecondDevice => self.draft.asio_second_device.clone(),
            Field::SecondOutput => self.draft.asio_second_output.map(|v| v.to_string()),
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
            || (field == Field::SecondDevice && self.second_device_rows().is_empty())
            || (field == Field::SecondOutput && self.second_output_rows().is_empty())
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
                self.draft.asio_second_device = None;
                self.draft.asio_second_output = None;
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
                // Which driver the copy rides on is decided again in here: this
                // driver may be the one its row names.
                self.refresh();
                self.adopt_stream_defaults();
            }
            Field::Output => {
                self.draft.output_device = value;
                self.draft.output_channels = None;
                self.draft.output_period_size = None;
                self.draft.output_sample_rate = None;
                // A start channel only means something on the driver it was
                // counted on.
                self.draft.asio_second_output = None;
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
                // An explicitly selected default route must survive saving.
                // An unknown default still has an empty raw value and stays None.
                let before = self.output_width();
                self.draft.output_channels = OutputChannels::parse(&raw);
                if before != self.output_width() {
                    // The copy is as wide as the route it copies, so a start
                    // counted for another width would name another span.
                    self.draft.asio_second_output = None;
                }
            }
            Field::SecondDevice => {
                self.draft.asio_second_device = value;
                // A start channel is a channel on a particular driver, and this
                // row has just said which driver that is.
                self.draft.asio_second_output = None;
                // The live driver is untouched by this choice, so the only question
                // worth asking again is the one about the carrier.
                self.same_driver_is_no_driver();
                self.ask_second_reach();
                self.prune_second_output();
            }
            Field::SecondOutput => {
                self.draft.asio_second_output = value.and_then(|v| v.parse().ok());
                if self.draft.asio_second_output.is_none() {
                    // Off means no copy at all, and a driver named to carry one
                    // would then be a name with nothing behind it.
                    self.draft.asio_second_device = None;
                }
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
        let (rows, default) = self.rows(field);
        // An empty draft means "follow the backend default", so the default
        // row is highlighted wherever the option order puts it.
        let want = if raw.is_empty() { &default } else { &raw };
        rows.iter().position(|(_, v)| v == want).unwrap_or(0)
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
        let (rows, audio_rows) = self.page_rows();
        // Application rows sit a little lower than the audio block, so the gap
        // follows however many audio rows are on the page right now.
        let row_y = |i: usize| {
            h / 2.0
                - 32.0 * fit
                - i as f32 * 24.0 * fit
                - if i >= audio_rows { 10.0 * fit } else { 0.0 }
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
                self.cancel_audio_draft();
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
        let dirty = (self.audio_dirty()
            || (!self.extras.is_empty() && self.saved.preferences != self.draft.preferences))
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
                    self.cancel_audio_draft();
                    return Some(Event::Back);
                }
                if reset {
                    self.reset_audio_draft();
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
    fn default_keeps_its_sorted_place_and_follow_is_stored_as_none() {
        let mut p = panel();
        // Default channel 1 already sat first, which hides a hoist. Move the
        // default so the old behaviour would reorder: rows stay in order.
        p.snapshot.default_input_channel = Some(2);
        assert_eq!(
            p.rows(Field::InputCh).0,
            vec![("1".into(), "1".into()), ("2".into(), "2".into())]
        );
        p.pick(Field::InputCh, 0);
        assert_eq!(p.draft.input_channel, Some(1));
        p.pick(Field::InputCh, 1); // the default row: follow, stored as None
        assert_eq!(p.draft.input_channel, None);
        assert_eq!(p.selected_index(Field::InputCh), 1); // follows the default row
        p.snapshot.default_input_channel = None;
        assert_eq!(p.rows(Field::InputCh).0[0].0, "—");
        p.snapshot.input_channels = None;
        assert_eq!(p.rows(Field::InputCh).0.len(), 1);
    }
    #[test]
    fn an_in_list_default_does_not_jump_to_the_top_of_the_dropdown() {
        let mut p = panel();
        // buffer_pref 64 sits mid-list: the dropdown keeps [32, 64, 128].
        assert_eq!(p.default_raw(Field::InputBuffer), "64");
        assert_eq!(
            p.rows(Field::InputBuffer).0,
            vec![
                ("32".into(), "32".into()),
                ("64".into(), "64".into()),
                ("128".into(), "128".into()),
            ]
        );
        assert_eq!(p.default_raw(Field::Rate), "48000");
        assert_eq!(
            p.rows(Field::Rate).0,
            vec![
                ("44100 Hz".into(), "44100".into()),
                ("48000 Hz".into(), "48000".into())
            ]
        );
        // A default no option carries (a JACK server period beyond the
        // fixed choices) leads the list instead of reordering anything.
        p.draft.backend = Some("jack".into());
        p.applied.backend = Some("jack".into());
        p.live_frames = Some(256);
        assert_eq!(
            p.rows(Field::InputBuffer).0,
            vec![
                ("256".into(), "256".into()),
                ("32".into(), "32".into()),
                ("64".into(), "64".into()),
                ("128".into(), "128".into()),
            ]
        );
    }
    #[test]
    fn independent_output_defaults_and_linked_controls_follow_the_same_rules() {
        let mut p = panel();
        assert_eq!(p.default_raw(Field::Rate), "48000");
        assert_eq!(p.default_raw(Field::OutputRate), "44100");
        p.pick(Field::InputBuffer, 0); // 32; default 64 keeps its sorted place
        assert_eq!(p.draft.period_size, Some(32));
        assert_eq!(p.draft.output_period_size, None);
        p.snapshot.buffers_linked = true;
        p.pick(Field::InputBuffer, 2);
        assert_eq!(p.draft.period_size, p.draft.output_period_size);
        assert!(p.disabled(Field::OutputBuffer));
        p.snapshot.rates_linked = true;
        p.pick(Field::Rate, 0); // 44100; default 48000 sits second now
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
    /// The settings page as an ASIO user sees it: one driver holding both
    /// directions, and a device that says how many output channels it has.
    fn asio_panel(channels: u32) -> AudioPanel<NoPreferences> {
        let mut p = panel();
        p.draft.backend = Some("asio".into());
        p.applied.backend = Some("asio".into());
        let route = Some(OutputChannels::Stereo([1, 2]));
        p.draft.output_channels = route;
        p.applied.output_channels = route;
        p.saved.output_channels = route;
        p.snapshot.output_formats = vec![devices::StreamFormat {
            channels,
            min_rate: 44100,
            max_rate: 48000,
            buffer_range: None,
        }];
        p
    }
    #[test]
    fn a_second_output_offers_the_spans_that_fit_after_the_first_one() {
        let mut p = asio_panel(8);
        // Every start whose copy lands inside the driver, not one per aligned
        // pair: a stereo route on 1–2 has a second pair waiting on 4–5 too.
        assert_eq!(
            p.rows(Field::SecondOutput).0,
            vec![
                ("Off".into(), String::new()),
                ("3–4".into(), "3".into()),
                ("4–5".into(), "4".into()),
                ("5–6".into(), "5".into()),
                ("6–7".into(), "6".into()),
                ("7–8".into(), "7".into()),
            ]
        );
        p.pick(Field::SecondOutput, 5);
        assert_eq!(p.draft.asio_second_output, Some(7));
        assert_eq!(p.value_text(Field::SecondOutput), "7–8");
        // The row on top means one output group, and stores nothing.
        p.pick(Field::SecondOutput, 0);
        assert_eq!(p.draft.asio_second_output, None);
        assert_eq!(p.value_text(Field::SecondOutput), "Off");
        assert!(audio_equal(&p.draft, &p.applied));
        p.pick(Field::SecondOutput, 1);
        assert!(!audio_equal(&p.draft, &p.applied));
    }
    #[test]
    fn a_device_without_room_for_a_second_group_offers_no_start() {
        // Four channels, stereo route: 3–4 is the last pair the driver has.
        let p = asio_panel(4);
        assert_eq!(p.rows(Field::SecondOutput).0.len(), 2);
        assert!(!p.disabled(Field::SecondOutput));
        // A driver whose channels the first group already fills, and one leaving
        // a single channel over, both have no span to hand out.
        for width in [2, 3] {
            let p = asio_panel(width);
            assert!(p.disabled(Field::SecondOutput));
            assert_eq!(p.rows(Field::SecondOutput).0.len(), 1);
            assert_eq!(p.value_text(Field::SecondOutput), "Off");
        }
    }
    #[test]
    fn a_second_output_is_as_wide_as_the_route_it_copies() {
        let mut p = asio_panel(8);
        p.pick(Field::SecondOutput, 1); // 3–4 behind a stereo route
        assert_eq!(p.draft.asio_second_output, Some(3));
        // On a mono route the same numbers mean different spans, so the rows are
        // counted again and nothing carries over from the pair.
        p.pick(Field::OutputCh, 0);
        assert_eq!(p.draft.output_channels, Some(OutputChannels::Mono(1)));
        assert_eq!(p.draft.asio_second_output, None);
        assert_eq!(
            p.rows(Field::SecondOutput).0,
            vec![
                ("Off".into(), String::new()),
                ("2".into(), "2".into()),
                ("3".into(), "3".into()),
                ("4".into(), "4".into()),
                ("5".into(), "5".into()),
                ("6".into(), "6".into()),
                ("7".into(), "7".into()),
                ("8".into(), "8".into()),
            ]
        );
    }
    #[test]
    fn a_start_the_device_cannot_host_is_dropped_and_an_unknown_width_is_left_alone() {
        let mut p = asio_panel(8);
        p.draft.asio_second_output = Some(7);
        p.prune_second_output();
        assert_eq!(p.draft.asio_second_output, Some(7));
        // The same number behind a smaller driver is not a choice.
        p.snapshot.output_formats[0].channels = 4;
        p.prune_second_output();
        assert_eq!(p.draft.asio_second_output, None);
        // Nor is one landing on the group it should copy.
        p.snapshot.output_formats[0].channels = 8;
        p.draft.asio_second_output = Some(2);
        p.prune_second_output();
        assert_eq!(p.draft.asio_second_output, None);
        // A driver that will not say how wide it is gets no verdict from here.
        p.draft.asio_second_output = Some(7);
        p.snapshot.output_formats.clear();
        p.prune_second_output();
        assert_eq!(p.draft.asio_second_output, Some(7));
    }
    #[test]
    fn the_second_output_row_shares_the_page_only_with_asio() {
        let names = |p: &AudioPanel<NoPreferences>| -> Vec<String> {
            p.page_rows()
                .0
                .iter()
                .map(|(label, _)| label.clone())
                .collect()
        };
        let mut p = panel();
        assert!(!names(&p).contains(&"Second output".to_string()));
        assert!(!names(&p).contains(&"Second device".to_string()));
        assert_eq!(names(&p).len(), 9);
        p.draft.backend = Some("asio".into());
        assert_eq!(
            names(&p),
            [
                "Backend",
                "Input device",
                "Input channel",
                "Output device",
                "Output channels",
                "Second device",
                "Second output",
                "Input buffer size",
                "Output buffer size",
                "Input sample rate",
                "Output sample rate",
            ]
        );
        // The driver is named before the channel that rides on it, and the pair
        // sits with the output rows it belongs to, not below the buffers.
        assert_eq!(
            names(&p)[5..7],
            ["Second device".to_string(), "Second output".to_string()]
        );
    }
    /// A second driver is a second set of channels: the starts are counted on it,
    /// from its channel 1, and the first driver's width has nothing to say about
    /// them.
    #[test]
    fn a_copy_on_a_second_driver_starts_where_that_driver_starts() {
        let mut p = asio_panel(8);
        // One driver on the machine, so the row says what the copy rides on and
        // offers nothing else.
        p.snapshot.outputs = vec!["First driver".into()];
        p.draft.output_device = Some("First driver".into());
        assert_eq!(
            p.rows(Field::SecondDevice).0,
            vec![("Same as output".into(), String::new())]
        );
        assert!(p.disabled(Field::SecondDevice));
        assert_eq!(p.value_text(Field::SecondDevice), "Same as output");
        // A second driver appears in the row, and the live one does not: naming it
        // would say what "Same as output" says.
        p.snapshot.outputs.push("Second driver".into());
        assert!(!p.disabled(Field::SecondDevice));
        assert_eq!(
            p.rows(Field::SecondDevice).0,
            vec![
                ("Same as output".into(), String::new()),
                ("Second driver".into(), "Second driver".into()),
            ]
        );
        // Eight channels on the carrier, a stereo route: the copy may sit anywhere
        // from its channel 1 to its channel 7, because nothing is in the way. The
        // first driver's eight channels would have started the list at 3.
        p.second_reach = Some(8);
        p.draft.asio_second_device = Some("Second driver".into());
        let mut expected = vec![("Off".into(), String::new())];
        expected
            .extend((1..=7u32).map(|start| (format!("{start}–{}", start + 1), start.to_string())));
        assert_eq!(p.rows(Field::SecondOutput).0, expected);
        // A start counted for the live driver's channels is dropped when the copy
        // moves, because channel 2 of another driver is another socket.
        p.draft.asio_second_output = Some(9);
        p.prune_second_output();
        assert_eq!(p.draft.asio_second_output, None);
        p.draft.asio_second_output = Some(1);
        p.prune_second_output();
        assert_eq!(p.draft.asio_second_output, Some(1));
        // And the copy's channel is remembered per carrier: turning the choice back
        // to "same as output" starts the counting behind the first group again.
        p.draft.asio_second_device = None;
        p.prune_second_output();
        assert_eq!(p.draft.asio_second_output, None);
    }
    /// The two rows are one decision: turning the copy off leaves no driver named
    /// to carry it, and naming the driver the copy comes from is what naming
    /// nothing means.
    #[test]
    fn the_two_copy_rows_answer_for_each_other() {
        let mut p = asio_panel(8);
        p.snapshot.outputs = vec!["First driver".into(), "Second driver".into()];
        p.draft.output_device = Some("First driver".into());
        // Chosen as the row would have it chosen; asking a driver that is not here
        // for its width is not part of what these two rows decide.
        p.second_reach = Some(2);
        p.draft.asio_second_device = Some("Second driver".into());
        assert_eq!(p.value_text(Field::SecondDevice), "Second driver");
        // A carrier with one pair of its own has exactly one place for a stereo
        // copy, and the row says so in that driver's numbers: 1–2, not 3–4.
        assert_eq!(
            p.rows(Field::SecondOutput).0,
            vec![("Off".into(), String::new()), ("1–2".into(), "1".into())]
        );
        p.pick(Field::SecondOutput, 1);
        assert_eq!(p.draft.asio_second_output, Some(1));
        // Off means no copy, so the carrier goes with it.
        p.pick(Field::SecondOutput, 0);
        assert_eq!(p.draft.asio_second_output, None);
        assert_eq!(p.draft.asio_second_device, None);
        assert_eq!(p.value_text(Field::SecondDevice), "Same as output");
        // And the row's own empty answer means the same thing: no copy rides
        // anywhere but where the first group already is.
        p.draft.asio_second_output = Some(3);
        p.pick(Field::SecondDevice, 0);
        assert_eq!(p.draft.asio_second_device, None);
        assert_eq!(p.draft.asio_second_output, None);
        // A file naming the driver the copy is copied from says the same thing in
        // the other row's words, and the page writes it back the short way.
        p.draft.asio_second_device = Some("First driver".into());
        p.same_driver_is_no_driver();
        assert_eq!(p.draft.asio_second_device, None);
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
        const FILE_NAME: &'static str = "nice-common-ui-tests.ron";
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
    fn main_view_preferences_do_not_dirty_apply_or_get_reverted_by_audio_navigation() {
        let mut panel = AudioPanel::<App>::new(
            Settings::default(),
            Settings::default(),
            Settings::default(),
            false,
        );
        panel.draft.preferences.value = Some(42);
        assert!(!panel.audio_dirty());
        panel.draft.backend = Some("dummy".into());
        assert!(panel.audio_dirty());
        panel.cancel_audio_draft();
        assert!(!panel.audio_dirty());
        assert_eq!(panel.draft.preferences.value, Some(42));
        panel.applied.backend = Some("dummy".into());
        panel.saved.backend = Some("dummy".into());
        panel.draft.backend = Some("dummy".into());
        panel.reset_audio_draft();
        assert!(panel.audio_dirty());
        assert_eq!(panel.draft.preferences.value, Some(42));
        // Successfully running but unsaved audio must still enable a save retry.
        panel.draft = panel.applied.clone();
        panel.saved.backend = None;
        assert!(panel.audio_dirty());
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

#[cfg(test)]
mod output_route_persistence_tests {
    use super::*;
    use crate::settings::NoPreferences;
    #[test]
    fn selected_default_output_route_is_serialized_as_a_concrete_configuration() {
        let mut panel = AudioPanel::<NoPreferences>::new(
            Settings::default(),
            Settings::default(),
            Settings::default(),
            false,
        );
        panel.draft.backend = Some("core-audio".into());
        panel.draft.output_device = Some("Speakers".into());
        let stereo = OutputChannels::Stereo([1, 2]);
        panel.snapshot.default_output_channels = Some(stereo);
        panel.snapshot.output_channel_options = vec![OutputChannels::Mono(1), stereo];
        panel.adopt_stream_defaults();
        assert_eq!(panel.draft.output_channels, Some(stereo));
        panel.draft.output_channels = None;
        panel.pick(Field::OutputCh, 1); // [Mono, Stereo(default)]: order kept
        assert_eq!(panel.draft.output_channels, Some(stereo));
        assert_eq!(panel.selected_index(Field::OutputCh), 1);
        let window = WindowGeom {
            x: None,
            y: None,
            width: 900.0,
            height: 620.0,
        };
        assert!(matches!(
            panel.save_with(panel.draft.clone(), Some(window), |settings| {
                let path = frost::scratch::claim("output-route-ron").join("settings.ron");
                settings.save_to(&path)?;
                let (restored, warning): (Settings, _) = Settings::load_from(&path);
                assert_eq!(warning, None);
                assert_eq!(restored.output_channels, Some(stereo));
                Ok(())
            }),
            Some(Event::Applied)
        ));
        panel.snapshot.default_output_channels = None;
        panel.draft.output_channels = None;
        panel.adopt_stream_defaults();
        panel.pick(Field::OutputCh, 0);
        assert_eq!(panel.draft.output_channels, None);
    }
}
