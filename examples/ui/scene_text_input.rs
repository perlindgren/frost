//! A text-input demo where the field is a node in the scene tree.
//!
//! Like `text_input`, but the field's background, selection, text, cursor,
//! and focus box are [`frost::SceneNode`]s in the scene tree, mutated every
//! frame via [`frost::Context::scene`], instead of immediate-mode draws.
//!
//! The editing logic lives in the shared [`frost::TextInput`] core (the same
//! type the immediate-mode [`frost::Ui::text_input`] widget drives); this
//! example only feeds it the frame's keys and mouse and syncs its state into
//! the scene tree with [`frost::TextInput::update_scene`].
//!
//! Click the field to focus it, type, and press **Enter** to commit the
//! text to the label below. Click outside the field (or press **Escape**)
//! to unfocus it. The focused field's cursor blinks.
//!
//! The field has the same editing capabilities as the `text_input` example:
//! drag the mouse (or hold **Shift** with the arrow keys) to mark a range,
//! **Ctrl-C** copies the mark (or the whole buffer), **Ctrl-X** cuts it,
//! **Ctrl-V** pastes the clipboard, and a right-click pastes too.

use std::sync::Arc;

use frost::{Color, Process};

// --- Layout and metrics -------------------------------------------------

/// The field's font size, in pixels per em.
const SIZE: f32 = 18.0;
/// FiraCode's advance width, in ems (the font is monospaced).
const ADVANCE_EM: f32 = 0.6;
/// One character's width, in pixels.
const CHAR_W: f32 = SIZE * ADVANCE_EM;
/// The number of characters the field shows at once.
const COLS: usize = 16;
/// The field's inner padding, in pixels.
const PAD: f32 = 10.0;
/// The field's height, in pixels.
const H: f32 = 40.0;
/// The field's width, in pixels.
const W: f32 = COLS as f32 * CHAR_W + 2.0 * PAD;
/// FiraCode's degenerate vertical metrics: text placed at a center prints
/// its ink a cap above it, so every text origin is dropped this far (in
/// ems) below the intended center.
const RON_LIFT: f32 = 0.344;

// --- Layout positions (window-centered, y up) ---------------------------

/// The field's center y.
const INPUT_CY: f32 = 0.0;
/// The label's center y (below the field).
const LABEL_CY: f32 = -62.0;
/// The primary hint's center y (above the field).
const HINT_CY: f32 = 62.0;
/// The secondary hint's center y (just above the field).
const HINT2_CY: f32 = 42.0;
/// The hints' font size.
const HINT_SIZE: f32 = 14.0;
/// The label's font size.
const LABEL_SIZE: f32 = 16.0;

// --- Colors -------------------------------------------------------------

const BG: Color = Color {
    r: 0.08,
    g: 0.09,
    b: 0.12,
    a: 1.0,
};
const FIELD_BG: Color = Color {
    r: 0.10,
    g: 0.11,
    b: 0.15,
    a: 1.0,
};
const TEXT: Color = Color {
    r: 0.92,
    g: 0.94,
    b: 0.98,
    a: 1.0,
};
const DIM: Color = Color {
    r: 0.55,
    g: 0.58,
    b: 0.64,
    a: 1.0,
};
const CURSOR: Color = Color {
    r: 0.35,
    g: 0.72,
    b: 1.0,
    a: 1.0,
};
/// The selection highlight: the cursor's blue at a third of its strength.
const SEL: Color = Color {
    r: 0.35,
    g: 0.72,
    b: 1.0,
    a: 0.30,
};

// --- The demo -----------------------------------------------------------

struct Demo {
    /// The field's editing state: the buffer, cursor, mark, and blink.
    input: frost::TextInput,
    /// The last committed text (set on Enter).
    label: String,
    /// The font the label and the hints are drawn with.
    font: Arc<[u8]>,
}

/// The field's rect, centered on the window's x axis at `INPUT_CY`.
fn field_rect() -> [f32; 4] {
    [-W / 2.0, INPUT_CY - H / 2.0, W / 2.0, INPUT_CY + H / 2.0]
}

impl Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let rect = field_rect();
        let pos = ctx.mouse_position();
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let rdown = ctx.mouse_button_down(frost::MouseButton::Right);

        // The mouse: focus, drag-select, and right-click paste.
        self.input.process_mouse(pos, down, rdown, rect);

        // The field's keys act only while it is focused; Enter commits the
        // buffer to the label.
        if self.input.process_keys(ctx, dt) {
            self.label = self.input.buffer().to_string();
        }

        // Update the field's scene-tree node: the visible characters, the
        // marked range, the cursor, and the focus box.
        let field = &mut ctx.scene().root.children[0];
        self.input.update_scene(field, self.input.blink_on());

        // The label: the last committed text, or a placeholder.
        let (label_text, label_color) = if self.label.is_empty() {
            ("(nothing committed yet)", DIM)
        } else {
            (self.label.as_str(), TEXT)
        };
        ctx.text(
            0.0,
            LABEL_CY - RON_LIFT * LABEL_SIZE,
            &self.font,
            label_text,
            LABEL_SIZE,
            500.0,
            label_color,
            1.0,
        );

        // The hints above the field.
        ctx.text(
            0.0,
            HINT_CY - RON_LIFT * HINT_SIZE,
            &self.font,
            "Type, then press Enter to commit",
            HINT_SIZE,
            400.0,
            DIM,
            1.0,
        );
        ctx.text(
            0.0,
            HINT2_CY - RON_LIFT * HINT_SIZE,
            &self.font,
            "Drag or Shift+Arrows to select, Ctrl-C/X/V to copy/cut/paste, right-click to paste",
            HINT_SIZE,
            400.0,
            DIM,
            1.0,
        );
    }
}

fn main() {
    env_logger::init();
    let root = std::env!("CARGO_MANIFEST_DIR");
    let font_path = format!("{root}/assets/fonts/FiraCode-VariableFont_wght.ttf");
    let font: Arc<[u8]> = Arc::from(std::fs::read(&font_path).expect("failed to read the font"));

    // The field's text shape: the visible characters, updated every frame.
    let mut text_shape = frost::Shape::text_bytes(&font, "", SIZE)
        .expect("the font is valid")
        .with_weight(600.0);
    if let frost::Shape::Text { color, .. } = &mut text_shape {
        *color = TEXT;
    }

    // The field: a rectangle background with the selection, text, cursor,
    // and focus box as children, positioned in the scene tree. The node's
    // origin is the field's center, so the children's local space is
    // centered on the field.
    let field = frost::SceneNode {
        transform: frost::Transform::translate([0.0, INPUT_CY]),
        shape: Some(frost::Shape::Rectangle {
            center: [0.0, 0.0],
            extent: [W / 2.0, H / 2.0],
            color: FIELD_BG,
        }),
        children: vec![
            // The selection highlight, behind the text.
            Box::new(frost::SceneNode {
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, 0.0],
                    extent: [0.0, 0.0],
                    color: SEL,
                }),
                ..Default::default()
            }),
            // The visible characters.
            Box::new(frost::SceneNode {
                shape: Some(text_shape),
                ..Default::default()
            }),
            // The cursor.
            Box::new(frost::SceneNode {
                shape: Some(frost::Shape::Polyline {
                    points: Vec::new(),
                    width: 1.5,
                    color: CURSOR,
                }),
                ..Default::default()
            }),
            // The focus box.
            Box::new(frost::SceneNode {
                shape: Some(frost::Shape::Polyline {
                    points: Vec::new(),
                    width: 2.0,
                    color: CURSOR,
                }),
                ..Default::default()
            }),
        ],
        ..Default::default()
    };

    let scene = frost::Scene::new(frost::SceneNode {
        shape: Some(frost::Shape::Background { color: BG }),
        children: vec![Box::new(field)],
        ..Default::default()
    });

    if let Err(err) = frost::run_configured(
        scene,
        Demo {
            input: frost::TextInput::new(),
            label: String::new(),
            font,
        },
        frost::Config {
            window_size: Some([720, 420]),
            ..Default::default()
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
