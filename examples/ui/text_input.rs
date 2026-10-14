//! A text-input demo in the immediate-mode [`frost::Ui`] layer: one
//! [`frost::Ui::text_input`] widget and one label.
//!
//! Click the field to focus it, type, and press **Enter** to commit the
//! text to the label below. Click outside the field (or press **Escape**)
//! to unfocus it. The focused field's cursor blinks.
//!
//! The field has the same editing capabilities as the scene-tree variant
//! (`scene_text_input`): drag the mouse (or hold **Shift** with the arrow
//! keys) to mark a range, **Ctrl-C** copies the mark (or the whole buffer),
//! **Ctrl-X** cuts it, **Ctrl-V** pastes the clipboard, and a right-click
//! pastes too.
//!
//! The field's state — buffer, cursor, mark, blink — lives in the engine's
//! [`frost::TextInput`]; the widget is a thin adapter that draws it and
//! keeps one state per widget id.
//!
//! Run with:
//!
//! ```text
//! cargo run --example text_input
//! ```

use frost::{Color, Context, Scene, SceneNode, Shape};

/// The demo's state: the widget layer plus the values its widgets drive.
struct Demo {
    /// The immediate-mode widget layer; owns the UI font.
    ui: frost::Ui,
    /// The field's live text, kept in sync with the widget's retained state.
    buffer: String,
    /// The text committed by the last **Enter**.
    committed: String,
}

impl Default for Demo {
    fn default() -> Self {
        // The UI font is loaded once, from disk here; `Ui::from_bytes`
        // takes `include_bytes!` data for a web build.
        let root = std::env!("CARGO_MANIFEST_DIR");
        let ui = frost::Ui::from_font(format!(
            "{root}/assets/fonts/FiraCode-VariableFont_wght.ttf"
        ))
        .expect("failed to load assets/fonts/FiraCode-VariableFont_wght.ttf");
        Self {
            ui,
            buffer: String::new(),
            committed: String::new(),
        }
    }
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        // Start the UI frame: snapshot the mouse, resolve last frame's
        // press, drag the panel under the button.
        self.ui.begin(ctx);

        // The hint above the field.
        self.ui.label(ctx, "Type, then press Enter to commit");

        // The field. It returns `true` on the frame Enter is pressed, so
        // commit the buffer to the label then.
        let committed = self.ui.text_input(ctx, "Name", &mut self.buffer, dt);
        if committed {
            self.committed = self.buffer.clone();
        }

        // The committed text below the field.
        let shown = if self.committed.is_empty() {
            "(nothing committed yet)"
        } else {
            self.committed.as_str()
        };
        self.ui.label(ctx, &format!("Committed: {shown}"));
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    let scene = Scene {
        root: SceneNode {
            shape: Some(Shape::Background {
                color: Color {
                    r: 0.08,
                    g: 0.09,
                    b: 0.12,
                    a: 1.0,
                },
            }),
            ..Default::default()
        },
        ..Default::default()
    };

    if let Err(err) = frost::run_configured(
        scene,
        Demo::default(),
        frost::Config {
            window_size: Some([480, 320]),
            ..Default::default()
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
