//! Main model/cabinet view and standalone audio configuration.
use crate::NamSharedState;
#[cfg(feature = "standalone")]
use crate::settings::Settings;
use frost::{Color, Context, Process, Scene, SceneNode, Shape};
#[cfg(feature = "standalone")]
use nice_settings::ui::{AudioPanel, Event};
use std::sync::{Arc, atomic::Ordering};

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
        ctx.text(0.0, -y, &self.font, text, size, 400.0, color, 1.0);
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
    fn choose_model(&mut self) {
        #[cfg(not(windows))]
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("NAM model", &["nam", "namb"])
            .pick_file()
        {
            let mut paths = self.shared.engine.paths.lock().unwrap();
            paths.model_name = path.file_name().map(|s| s.to_string_lossy().into_owned());
            paths.model_path = Some(path);
            self.shared.engine.revision.fetch_add(1, Ordering::Release);
        }
    }
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
                settings.preferences = paths.clone();
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
                panel.draft.preferences = paths.clone();
                panel.saved.preferences = paths;
            }
            let size = self.shared.logical_size();
            let window = nice_settings::ui::window_geometry(
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
            rows.push(("LOAD MODEL".into(), !cfg!(windows), false));
            rows.push(("LOAD CABINET".into(), !cfg!(windows), false));
            rows.push(("REMOVE CABINET".into(), !cfg!(windows), false));
            let paths = self.shared.engine.paths.lock().unwrap().clone();
            self.text(
                ctx,
                15.0,
                &format!(
                    "Model: {}",
                    paths.model_name.as_deref().unwrap_or("None (bypass)")
                ),
                18.0,
                rgb(103, 237, 175),
            );
            let cab = paths
                .cabinet_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or("None".into());
            self.text(
                ctx,
                48.0,
                &format!("Cabinet: {cab}"),
                18.0,
                rgb(235, 230, 241),
            );
            let status = self.shared.engine.status.lock().unwrap().clone();
            for (i, line) in status
                .lines()
                .flat_map(|line| {
                    let chars: Vec<_> = line.chars().collect();
                    chars
                        .chunks(68)
                        .map(|c| c.iter().collect::<String>())
                        .collect::<Vec<_>>()
                })
                .take(5)
                .enumerate()
            {
                self.text(ctx, 95.0 + i as f32 * 23.0, &line, 15.0, rgb(235, 230, 241));
            }
        }
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
        self.pressed = down;
        if let Some(row) = click {
            if row == SETTINGS_BUTTON {
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
            265.0,
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
