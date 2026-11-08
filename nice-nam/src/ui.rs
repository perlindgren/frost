//! Main model/cabinet view and standalone audio configuration.
use crate::NamSharedState;
#[cfg(feature = "standalone")]
use crate::settings::Settings;
use frost::{Color, Context, Process, Scene, SceneNode, Shape};
#[cfg(feature = "standalone")]
use nice_common::ui::{AudioPanel, Event};
use std::sync::{Arc, atomic::Ordering};

/// What the model picker offers. A Windows build's engine cannot open a `.namb`
/// container at all, so it offers the format that loads there rather than a file
/// that would only come back refused — `Chain::load` checks the same thing.
#[cfg(not(windows))]
const MODEL_FILTERS: &[&str] = &["nam", "namb"];
#[cfg(windows)]
const MODEL_FILTERS: &[&str] = &["nam"];

/// The heaviest cut of the main view's font, which is the heaviest it has. The
/// embedded `FiraCode-VariableFont_wght.ttf` carries one variation axis, `wght`,
/// spanning 300 to 700 with a default of 300 — those are the file's own `fvar`
/// numbers — so 700 is as black as this font gets and a larger figure would ask
/// for the same pixels. The number moves a glyph only because that axis exists:
/// handed a static face, such as the icon font below, it is ignored outright.
const TEXT_WEIGHT: f32 = 700.0;

/// How far from each side of the window a line may reach before it is shortened.
/// The drawing unit is one logical pixel of the window and the origin is its
/// centre, so a line has the window's width minus twice this to live in.
const EDGE_INSET: f32 = 24.0;

/// What stands where the middle of a shortened line used to be.
const ELLIPSIS: char = '…';

/// The most characters of `chars` that `build` can turn into something `measure`
/// still calls no wider than `available`, between zero and the whole of `chars`.
/// Bisected, so a long line costs a handful of measurements rather than one for
/// each character it holds.
fn longest_fit(
    measure: &impl Fn(&str) -> f32,
    chars: &[char],
    available: f32,
    build: &impl Fn(usize) -> String,
) -> usize {
    let (mut keep, mut limit) = (0usize, chars.len());
    while keep < limit {
        let middle = keep + (limit - keep).div_ceil(2);
        if measure(&build(middle)) <= available {
            keep = middle;
        } else {
            limit = middle - 1;
        }
    }
    keep
}

/// `text` with its middle replaced by one `ELLIPSIS`, cut down until `measure`
/// says it fits `available`.
///
/// `measure` is a parameter because the width of a string belongs to the font and
/// its glyph atlas, not to this function: the view hands over `Context::text_width`,
/// which shapes a string through the same path that paints it, so what is measured
/// is what appears, and a test brings a ruler of its own.
///
/// The middle is what goes, not the end, because every line this serves keeps its
/// meaning at the end — a file name at the end of a path, a number or a reason at
/// the end of a sentence. Eliding the tail would cost precisely the part worth reading.
fn elide_middle(measure: impl Fn(&str) -> f32, text: &str, available: f32) -> String {
    if measure(text) <= available {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let build = |keep: usize| -> String {
        let mut shortened: String = chars[..keep.div_ceil(2)].iter().collect();
        shortened.push(ELLIPSIS);
        shortened.extend(chars[chars.len() - keep / 2..].iter());
        shortened
    };
    let keep = longest_fit(&measure, &chars, available, &build);
    // A window too narrow for even the ellipsis is not told anything truer by a
    // different answer, and offering one character of either end would only
    // misrepresent both.
    if keep == 0 {
        ELLIPSIS.to_string()
    } else {
        build(keep)
    }
}

/// `text` broken into lines that each measure no wider than `available` — at
/// spaces wherever there is a space to break at, and inside a word only when that
/// word cannot fit a line by itself.
///
/// This replaces sixty-eight characters per line, a number that was neither the
/// window's width nor the font's: sixty-eight of this face at fifteen points is
/// about as wide as the narrowest window the layout allows, so the end of a long
/// message was already in the margin, and widening the window bought it nothing.
fn wrap(measure: impl Fn(&str) -> f32, text: &str, available: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let joined = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if measure(&joined) <= available {
            line = joined;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if measure(word) <= available {
            line = word.to_owned();
            continue;
        }
        // One word wider than the window: a path segment or a long number, which
        // is cut because the alternative is drawing past the side of the screen.
        let mut rest: Vec<char> = word.chars().collect();
        while measure(&rest.iter().collect::<String>()) > available {
            let build = |keep: usize| rest[..keep].iter().collect::<String>();
            // At least one character per line, so a word of any width terminates.
            let keep = longest_fit(&measure, &rest, available, &build).max(1);
            lines.push(build(keep));
            rest = rest[keep..].to_vec();
        }
        line = rest.iter().collect();
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: 1.0,
    }
}
pub fn static_scene() -> Scene {
    Scene {
        root: SceneNode {
            shape: Some(Shape::Background {
                color: rgb(32, 28, 44),
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}
pub struct NamUi {
    shared: Arc<NamSharedState>,
    font: Arc<[u8]>,
    icon_font: Arc<[u8]>,
    input_meter: nice_common::meter::PeakMeter,
    output_meter: nice_common::meter::PeakMeter,
    gate_open: bool,
    gate_scroll: f32,
    gate_drag: bool,
    pressed: bool,
    armed: Option<usize>,
    settings_page: bool,
    message: String,
    #[cfg(feature = "standalone")]
    applied: Settings,
    #[cfg(feature = "standalone")]
    panel: Option<AudioPanel<crate::settings::NamPreferences>>,
    #[cfg(feature = "standalone")]
    saved_paths: Option<crate::settings::NamPreferences>,
    #[cfg(feature = "diag")]
    diag: frost::Diagnostics,
}
impl NamUi {
    pub fn new(shared: Arc<NamSharedState>, font: Arc<[u8]>) -> Self {
        Self {
            shared,
            font: font.clone(),
            icon_font: crate::gui::icon_font(),
            input_meter: nice_common::meter::PeakMeter::new(crate::now_ms()),
            output_meter: nice_common::meter::PeakMeter::new(crate::now_ms()),
            gate_open: false,
            gate_scroll: 0.0,
            gate_drag: false,
            pressed: false,
            armed: None,
            settings_page: false,
            message: String::new(),
            #[cfg(feature = "standalone")]
            applied: Settings::load().0,
            #[cfg(feature = "standalone")]
            panel: None,
            #[cfg(feature = "standalone")]
            saved_paths: None,
            #[cfg(feature = "diag")]
            diag: frost::Diagnostics::from_bytes(&font, frost::DiagnosticsFlags::all())
                .unwrap()
                .expose_vsync(false),
        }
    }
    // Layout y increases downward; frost drawing and pointer coordinates increase upward.
    fn text(&self, ctx: &mut Context, y: f32, text: &str, size: f32, color: Color) {
        ctx.text(0.0, -y, &self.font, text, size, TEXT_WEIGHT, color, 1.0);
    }
    fn button(&self, ctx: &mut Context, y: f32, label: &str, enabled: bool, default: bool) {
        ctx.rectangle(0.0, -y, 285.0, 18.0, rgb(65, 57, 82), 0.5);
        self.text(
            ctx,
            y,
            label,
            17.0,
            if !enabled {
                rgb(115, 111, 123)
            } else if default {
                rgb(103, 237, 175)
            } else {
                rgb(235, 230, 241)
            },
        );
    }
    /// Pick a model. Every platform offers this: the engine refuses the files it
    /// cannot run, and a dialog is the only way to reach those refusals.
    ///
    /// Windows has one caveat worth knowing before it is blamed on this plugin.
    /// `rfd`'s synchronous picker initialises COM as single-threaded on the
    /// calling thread; if the host already initialised that thread as
    /// multi-threaded, the call fails and `pick_file` returns `None` — exactly
    /// what it returns when the user presses Cancel. In such a host the button
    /// looks dead. Nothing in the API distinguishes the two, so the message says
    /// what the user can act on. The way out, if a host proves to need it, is
    /// `rfd`'s async picker, which builds the dialog on a thread of its own where
    /// the initialisation cannot conflict.
    fn choose_model(&mut self) {
        let path = rfd::FileDialog::new()
            .add_filter("NAM model", MODEL_FILTERS)
            .pick_file();
        let Some(path) = path else {
            #[cfg(windows)]
            {
                self.message =
                    "No model chosen — if this repeats, this host may refuse dialogs.".into();
            }
            return;
        };
        let mut paths = self.shared.engine.paths.lock().unwrap();
        paths.model_name = path.file_name().map(|s| s.to_string_lossy().into_owned());
        paths.model_path = Some(path);
        self.shared.engine.revision.fetch_add(1, Ordering::Release);
    }
    /// There is no cabinet engine in a Windows build, so there is nothing here
    /// to pick; the row that would call this is disabled there.
    fn choose_cab(&mut self) {
        #[cfg(not(windows))]
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Cabinet impulse response", &["wav"])
            .pick_file()
        {
            self.shared.engine.paths.lock().unwrap().cabinet_path = Some(path);
            self.shared.engine.revision.fetch_add(1, Ordering::Release);
        }
    }
}
impl Process for NamUi {
    fn process(&mut self, ctx: &mut Context, _dt: f32) {
        self.shared.gate_db.store(
            self.shared
                .engine
                .paths
                .lock()
                .unwrap()
                .gate_db
                .unwrap_or(-60.0),
        );
        #[cfg(feature = "standalone")]
        if let Some(error) = crate::standalone::control::take_error() {
            self.message = error;
            self.applied = Settings::default();
            self.panel = None;
        }
        #[cfg(feature = "standalone")]
        if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
            let accepted = self.shared.engine.accepted.lock().unwrap().clone();
            if let Some(paths) = accepted
                && self.saved_paths.as_ref() != Some(&paths)
            {
                let mut settings = Settings::load().0;
                let gate = settings.gate_db;
                settings.preferences = paths.clone();
                settings.gate_db = gate;
                match settings.save() {
                    Ok(()) => self.saved_paths = Some(paths),
                    Err(error) => self.message = error,
                }
            }
        }
        #[cfg(feature = "standalone")]
        if self.settings_page {
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
            if let Some(paths) = self.shared.engine.accepted.lock().unwrap().clone() {
                let gate = panel.draft.gate_db;
                panel.draft.preferences = paths.clone();
                panel.draft.gate_db = gate;
                panel.saved.preferences = paths;
                panel.saved.gate_db = gate;
            }
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
            #[cfg(feature = "diag")]
            self.diag.process(ctx, _dt);
            return;
        }
        self.text(ctx, -260.0, "NICE NAM", 32.0, rgb(235, 230, 241));
        let mut rows: Vec<(String, bool, bool)> = Vec::new();
        {
            rows.push(("LOAD MODEL".into(), true, false));
            // A Windows build has no cabinet engine to receive an impulse
            // response, so these two have nothing to do there — they are off
            // rather than silently ignoring the choice, and the status line says
            // as much if a session hands one over.
            rows.push(("LOAD CABINET".into(), !cfg!(windows), false));
            rows.push(("REMOVE CABINET".into(), !cfg!(windows), false));
            let paths = self.shared.engine.paths.lock().unwrap().clone();
            let model_line = format!(
                "Model: {}",
                paths.model_name.as_deref().unwrap_or("None (bypass)")
            );
            let cab = paths
                .cabinet_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or("None".into());
            let cabinet_line = format!("Cabinet: {cab}");
            let status = self.shared.engine.status.lock().unwrap().clone();
            // Measured before any of it is drawn. The window can be dragged to its
            // minimum, and a line wider than it is not clipped — it runs past the
            // side of the screen and is gone, which for a refusal message means the
            // reason and the number you were told about both disappearing together.
            let room = (self.shared.logical_size().width - 2.0 * EDGE_INSET).max(160.0);
            let (model_shown, cabinet_shown, status_shown) = {
                let width =
                    |text: &str, points: f32| ctx.text_width(&self.font, text, points, TEXT_WEIGHT);
                (
                    elide_middle(|text| width(text, 18.0), &model_line, room),
                    elide_middle(|text| width(text, 18.0), &cabinet_line, room),
                    status
                        .lines()
                        .flat_map(|line| wrap(|text| width(text, 15.0), line, room))
                        .take(5)
                        .collect::<Vec<String>>(),
                )
            };
            self.text(ctx, 15.0, &model_shown, 18.0, rgb(103, 237, 175));
            self.text(ctx, 48.0, &cabinet_shown, 18.0, rgb(235, 230, 241));
            // Only the standalone owns a device, so only it has gaps to report.
            if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
                let gaps = crate::DROP_OUTS.load(Ordering::Relaxed);
                self.text(
                    ctx,
                    71.0,
                    &format!("X-runs: {gaps}"),
                    15.0,
                    if gaps == 0 {
                        rgb(103, 237, 175)
                    } else {
                        rgb(250, 190, 115)
                    },
                );
            }
            for (i, line) in status_shown.iter().enumerate() {
                self.text(ctx, 95.0 + i as f32 * 23.0, line, 15.0, rgb(235, 230, 241));
            }
        }
        let (w, h) = ctx.size();
        let fit = (w / 900.0).clamp(0.45, 1.0);
        let now = crate::now_ms();
        let live = now.saturating_sub(self.shared.meter_updated.load(Ordering::Relaxed)) < 250;
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
        let meter_y = -h / 2.0 + 48.0 * fit;
        let half = w * 0.20;
        let input_hover = self.input_meter.draw(
            ctx,
            &self.font,
            [-w / 4.0, meter_y],
            half,
            fit,
            rgb(103, 237, 175),
        );
        self.output_meter.draw(
            ctx,
            &self.font,
            [w / 4.0, meter_y],
            half,
            fit,
            rgb(103, 237, 175),
        );
        for (x, label) in [(-w / 4.0, "INPUT"), (w / 4.0, "OUTPUT")] {
            ctx.text(
                x,
                meter_y + 20.0 * fit,
                &self.font,
                label,
                14.0 * fit,
                TEXT_WEIGHT,
                rgb(235, 230, 241),
                1.0,
            );
        }
        if !self.gate_open && input_hover {
            nice_common::tooltip::draw(
                ctx,
                &self.font,
                &format!("INPUT GATE  {:.0} dB", self.shared.gate_db.load()),
                fit,
            );
        }
        const GATE_BUTTON: usize = usize::MAX - 1;
        const SETTINGS_BUTTON: usize = usize::MAX;
        let cog = !self.settings_page && crate::STANDALONE_MODE.load(Ordering::Relaxed);
        let size = self.shared.logical_size();
        let (gx, gy) = (size.width / 2.0 - 26.0, size.height / 2.0 - 22.0);
        let cog_hover = cog
            && ctx
                .mouse_position()
                .is_some_and(|[x, y]| (x - gx).abs() <= 16.0 && (y - gy).abs() <= 16.0);
        if cog {
            ctx.rectangle(gx, gy, 16.0, 16.0, rgb(65, 57, 82), 0.5);
            ctx.text(
                gx,
                gy,
                &self.icon_font,
                "\u{f6aa}",
                24.0,
                400.0,
                if cog_hover {
                    rgb(250, 190, 115)
                } else {
                    rgb(180, 174, 193)
                },
                1.0,
            );
        }
        let hover = ctx.mouse_position().and_then(|[x, y]| {
            if self.gate_open {
                return None;
            }
            if input_hover {
                return Some(GATE_BUTTON);
            }
            if cog_hover {
                return Some(SETTINGS_BUTTON);
            }
            let y = -y;
            if x.abs() > 285.0 {
                return None;
            }
            rows.iter()
                .enumerate()
                .find(|(i, (_, enabled, _))| {
                    *enabled && (y - (-205.0 + *i as f32 * 40.0)).abs() < 18.0
                })
                .map(|(i, _)| i)
        });
        for (i, (label, enabled, default)) in rows.iter().enumerate() {
            self.button(ctx, -205.0 + i as f32 * 40.0, label, *enabled, *default);
        }
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        if down && !self.pressed {
            self.armed = hover;
        }
        let click = if !down && self.pressed && self.armed == hover {
            hover
        } else {
            None
        };
        let rising = down && !self.pressed;
        self.pressed = down;
        if self.gate_open {
            if ctx.key_down(frost::KeyCode::Escape) {
                self.gate_open = false;
            }
            let items: Vec<_> = [-80, -70, -60, -50, -40, -30, -20]
                .into_iter()
                .map(|db| (format!("{db} dB"), db.to_string()))
                .collect();
            let gate = self.shared.gate_db.load();
            let selected = items
                .iter()
                .position(|(_, raw)| raw.parse::<f32>().ok() == Some(gate))
                .unwrap_or(2);
            let response = nice_common::dropdown::draw_dropdown(
                ctx,
                &self.font,
                [w, h],
                fit,
                nice_common::dropdown::Placement::Centred([110.0 * fit, 11.0 * fit]),
                1.1,
                rising,
                &items,
                selected,
                "-60",
                "INPUT GATE",
                false,
                &mut self.gate_scroll,
                &mut self.gate_drag,
            );
            if let Some(index) = response.picked {
                let gate = items[index].1.parse::<f32>().unwrap();
                self.shared.gate_db.store(gate);
                let value = (gate != -60.0).then_some(gate);
                self.shared.engine.paths.lock().unwrap().gate_db = value;
                #[cfg(feature = "standalone")]
                if crate::STANDALONE_MODE.load(Ordering::Relaxed) {
                    let mut settings = Settings::load().0;
                    settings.gate_db = value;
                    if let Err(error) = settings.save() {
                        self.message = error;
                    }
                }
                self.gate_open = false;
            } else if rising && !response.consumed {
                self.gate_open = false;
            }
        }
        if let Some(row) = click {
            if row == GATE_BUTTON {
                self.gate_open = true;
                self.gate_scroll = 0.0;
                self.gate_drag = false;
            } else if row == SETTINGS_BUTTON {
                #[cfg(feature = "standalone")]
                {
                    self.panel = None;
                    self.settings_page = true;
                }
            } else {
                match row {
                    0 => self.choose_model(),
                    1 => self.choose_cab(),
                    2 => self.shared.engine.paths.lock().unwrap().cabinet_path = None,
                    _ => {}
                }
            }
        }
        self.text(
            ctx,
            h / 2.0 - 105.0,
            &self.message.chars().take(75).collect::<String>(),
            14.0,
            rgb(250, 190, 115),
        );
        #[cfg(feature = "diag")]
        self.diag.process(ctx, _dt);
    }
}

pub fn map_key(key: &keyboard_types::Key) -> Option<frost::KeyCode> {
    use keyboard_types::{Key, NamedKey};
    match key {
        Key::Character(s) => match s.as_str() {
            "a" | "A" => Some(frost::KeyCode::KeyA),
            "b" | "B" => Some(frost::KeyCode::KeyB),
            "c" | "C" => Some(frost::KeyCode::KeyC),
            "d" | "D" => Some(frost::KeyCode::KeyD),
            "e" | "E" => Some(frost::KeyCode::KeyE),
            "f" | "F" => Some(frost::KeyCode::KeyF),
            "g" | "G" => Some(frost::KeyCode::KeyG),
            "h" | "H" => Some(frost::KeyCode::KeyH),
            "i" | "I" => Some(frost::KeyCode::KeyI),
            "j" | "J" => Some(frost::KeyCode::KeyJ),
            "k" | "K" => Some(frost::KeyCode::KeyK),
            "l" | "L" => Some(frost::KeyCode::KeyL),
            "m" | "M" => Some(frost::KeyCode::KeyM),
            "n" | "N" => Some(frost::KeyCode::KeyN),
            "o" | "O" => Some(frost::KeyCode::KeyO),
            "p" | "P" => Some(frost::KeyCode::KeyP),
            "q" | "Q" => Some(frost::KeyCode::KeyQ),
            "r" | "R" => Some(frost::KeyCode::KeyR),
            "s" | "S" => Some(frost::KeyCode::KeyS),
            "t" | "T" => Some(frost::KeyCode::KeyT),
            "u" | "U" => Some(frost::KeyCode::KeyU),
            "v" | "V" => Some(frost::KeyCode::KeyV),
            "w" | "W" => Some(frost::KeyCode::KeyW),
            "x" | "X" => Some(frost::KeyCode::KeyX),
            "y" | "Y" => Some(frost::KeyCode::KeyY),
            "z" | "Z" => Some(frost::KeyCode::KeyZ),
            "0" => Some(frost::KeyCode::Digit0),
            "1" => Some(frost::KeyCode::Digit1),
            "2" => Some(frost::KeyCode::Digit2),
            "3" => Some(frost::KeyCode::Digit3),
            "4" => Some(frost::KeyCode::Digit4),
            "5" => Some(frost::KeyCode::Digit5),
            "6" => Some(frost::KeyCode::Digit6),
            "7" => Some(frost::KeyCode::Digit7),
            "8" => Some(frost::KeyCode::Digit8),
            "9" => Some(frost::KeyCode::Digit9),
            " " => Some(frost::KeyCode::Space),
            _ => None,
        },
        Key::Named(named) => Some(match named {
            NamedKey::Escape => frost::KeyCode::Escape,
            NamedKey::Enter => frost::KeyCode::Enter,
            NamedKey::Tab => frost::KeyCode::Tab,
            NamedKey::Backspace => frost::KeyCode::Backspace,
            NamedKey::Delete => frost::KeyCode::Delete,
            NamedKey::ArrowLeft => frost::KeyCode::ArrowLeft,
            NamedKey::ArrowRight => frost::KeyCode::ArrowRight,
            NamedKey::ArrowUp => frost::KeyCode::ArrowUp,
            NamedKey::ArrowDown => frost::KeyCode::ArrowDown,
            _ => return None,
        }),
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

#[cfg(test)]
mod tests {
    use super::{ELLIPSIS, elide_middle, wrap};

    /// A ruler on which every character is ten units wide, so that fitting is
    /// arithmetic. These hold the algorithm to its contract; what a string of this
    /// project's actual font actually measures is `Context::text_width`'s business,
    /// and the view hands it over as the measuring function.
    fn ruler(text: &str) -> f32 {
        text.chars().count() as f32 * 10.0
    }

    #[test]
    fn what_already_fits_is_handed_over_untouched() {
        assert_eq!(
            elide_middle(ruler, "Model: AC50.nam", 200.0),
            "Model: AC50.nam"
        );
        assert_eq!(wrap(ruler, "one two three", 200.0), ["one two three"]);
        // A status that says nothing draws nothing, rather than an empty line
        // taking up the room of one that says something.
        assert!(wrap(ruler, "", 200.0).is_empty());
    }

    #[test]
    fn a_shortened_line_keeps_its_end_and_never_overflows() {
        // The file name is what the line exists to show, so it survives the cut,
        // and so does the label in front of it. The ellipsis counts against the
        // room available instead of being added to a line already full.
        let path = r"Model: C:\Gear\Captures\AC50 Mk.I - Clean + Bright.nam";
        let shown = elide_middle(ruler, path, 300.0);
        assert!(
            shown.chars().count() < path.chars().count(),
            "not cut at all"
        );
        assert!(ruler(&shown) <= 300.0, "{shown} measures {}", ruler(&shown));
        assert!(shown.starts_with("Model:"), "lost the label: {shown}");
        assert!(
            shown.ends_with("Bright.nam"),
            "lost the capture's name: {shown}"
        );
        assert!(
            shown.contains(ELLIPSIS),
            "no mark where the middle was: {shown}"
        );
    }

    #[test]
    fn a_space_the_width_of_one_character_offers_the_ellipsis_alone() {
        // Nothing honest fits in less than the mark itself. What matters is that
        // the search terminates and says so, rather than promising a character it
        // cannot show or spinning on the question.
        assert_eq!(elide_middle(ruler, "abcdefghij", 5.0).chars().count(), 1);
    }

    #[test]
    fn wrapping_breaks_at_a_space_before_it_breaks_anywhere_else() {
        assert_eq!(
            wrap(ruler, "aaaaaaaaaa bbbbbbbbbb cccccccccc", 100.0),
            ["aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc"]
        );
    }

    #[test]
    fn a_word_wider_than_the_window_is_cut_rather_than_allowed_to_run_off() {
        let long = "z".repeat(25);
        let lines = wrap(ruler, &long, 100.0);
        assert_eq!(
            lines
                .iter()
                .map(|line| line.chars().count())
                .collect::<Vec<_>>(),
            [10, 10, 5],
            "{lines:?}"
        );
        assert_eq!(lines.concat(), long, "cutting lost characters");
    }
}
