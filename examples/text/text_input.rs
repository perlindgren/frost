//! A text-input demo: one text field and one label.
//!
//! Click the field to focus it, type, and press **Enter** to commit the
//! text to the label below. Click outside the field (or press **Escape**)
//! to unfocus it. The focused field's cursor blinks.
//!
//! The field has the same editing capabilities as `sprite_util`'s text
//! inputs: drag the mouse (or hold **Shift** with the arrow keys) to mark
//! a range, **Ctrl-C** copies the mark (or the whole buffer), **Ctrl-X**
//! cuts it, **Ctrl-V** pastes the clipboard, and a right-click pastes too.

use std::collections::{HashMap, HashSet};
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
/// The buffer's capacity, in characters.
const MAX: usize = 64;
/// FiraCode's degenerate vertical metrics: text placed at a center prints
/// its ink a cap above it, so every text origin is dropped this far (in
/// ems) below the intended center.
const RON_LIFT: f32 = 0.344;
/// The cursor blink period, in seconds: visible for the first half, hidden
/// for the second.
const BLINK: f32 = 1.0;
/// The first repeat delay, in seconds, after a key is held.
const INITIAL_DELAY: f32 = 0.3;
/// The repeat interval, in seconds, while a key is held.
const REPEAT: f32 = 0.075;

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

// --- The text field -----------------------------------------------------

/// A single-line text field: a fixed-width buffer that shows `COLS`
/// characters at a time and scrolls the rest past as the cursor moves. It
/// keeps a marked range (the selection) that the clipboard and the editing
/// keys act on.
struct TextInput {
    buffer: String,
    cursor: usize,
    scroll: usize,
    focused: bool,
    /// The marked range `(start, end)`, `start <= end`, the cursor at one
    /// end; `None` when nothing is marked.
    sel: Option<(usize, usize)>,
}

impl TextInput {
    fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            scroll: 0,
            focused: false,
            sel: None,
        }
    }

    /// Sets the focus; losing it clears the mark.
    fn set_focus(&mut self, focused: bool) {
        self.focused = focused;
        if !focused {
            self.sel = None;
        }
    }

    /// Places the cursor at the character boundary under `x` (the field's
    /// left edge is `rect[0]`), clearing any mark.
    fn click_at(&mut self, x: f32, rect: [f32; 4]) {
        let len = self.buffer.chars().count();
        let col = (((x - rect[0]) - PAD) / CHAR_W).floor() as usize;
        let cell = col.min(COLS);
        self.cursor = (self.scroll + cell).min(len);
        self.sel = None;
        self.clamp_scroll();
    }

    /// Extends the mark from `anchor` to the cell under `x`, the cursor
    /// riding the far end.
    fn drag_to(&mut self, anchor: usize, x: f32, rect: [f32; 4]) {
        let len = self.buffer.chars().count();
        let col = (((x - rect[0]) - PAD) / CHAR_W).floor() as usize;
        let cell = col.min(COLS);
        let idx = (self.scroll + cell).min(len);
        let (s, e) = if anchor <= idx {
            (anchor, idx)
        } else {
            (idx, anchor)
        };
        self.sel = Some((s, e));
        self.cursor = idx;
        self.clamp_scroll();
    }

    /// The marked range, if it spans at least one character.
    fn selection_range(&self) -> Option<(usize, usize)> {
        self.sel.filter(|&(s, e)| s < e)
    }

    /// The marked characters, or the empty string when nothing is marked.
    fn selected_text(&self) -> String {
        let Some((s, e)) = self.selection_range() else {
            return String::new();
        };
        self.buffer.chars().skip(s).take(e - s).collect()
    }

    /// Removes the marked range, if any, and reports whether it did.
    fn delete_selection(&mut self) -> bool {
        let Some((s, e)) = self.selection_range() else {
            return false;
        };
        let mut chars: Vec<char> = self.buffer.chars().collect();
        chars.drain(s..e);
        self.buffer = chars.into_iter().collect();
        self.cursor = s;
        self.sel = None;
        self.clamp_scroll();
        true
    }

    /// Inserts one character at the cursor, replacing any mark and taking
    /// only as many as fit under `MAX`.
    fn insert(&mut self, ch: char) {
        self.delete_selection();
        if self.buffer.chars().count() < MAX {
            let mut chars: Vec<char> = self.buffer.chars().collect();
            chars.insert(self.cursor, ch);
            self.buffer = chars.into_iter().collect();
            self.cursor += 1;
        }
        self.clamp_scroll();
    }

    /// Inserts a run of characters at the cursor, replacing any mark and
    /// taking only as many as fit under `MAX`.
    fn insert_str(&mut self, s: &str) {
        self.delete_selection();
        let mut chars: Vec<char> = self.buffer.chars().collect();
        let room = MAX.saturating_sub(chars.len());
        let take: Vec<char> = s.chars().take(room).collect();
        chars.splice(self.cursor..self.cursor, take.iter().cloned());
        self.cursor += take.len();
        self.buffer = chars.into_iter().collect();
        self.clamp_scroll();
    }

    /// Deletes the mark, or the character before the cursor when nothing
    /// is marked.
    fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor > 0 {
            let mut chars: Vec<char> = self.buffer.chars().collect();
            chars.remove(self.cursor - 1);
            self.buffer = chars.into_iter().collect();
            self.cursor -= 1;
        }
        self.clamp_scroll();
    }

    /// Deletes the mark, or the character at the cursor when nothing is
    /// marked.
    fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }
        let len = self.buffer.chars().count();
        if self.cursor < len {
            let mut chars: Vec<char> = self.buffer.chars().collect();
            chars.remove(self.cursor);
            self.buffer = chars.into_iter().collect();
        }
        self.clamp_scroll();
    }

    /// Moves the cursor one cell left, clearing the mark.
    fn move_left(&mut self) {
        self.sel = None;
        self.cursor = self.cursor.saturating_sub(1);
        self.clamp_scroll();
    }

    /// Moves the cursor one cell right, clearing the mark.
    fn move_right(&mut self) {
        self.sel = None;
        self.cursor = (self.cursor + 1).min(self.buffer.chars().count());
        self.clamp_scroll();
    }

    /// Marks one more character to the left: the cursor walks left, the
    /// selection growing behind it, or shrinking when it walks back onto
    /// the anchor.
    fn select_left(&mut self) {
        match self.sel {
            Some((s, e)) if self.cursor == e && e > s => {
                self.sel = Some((s, e - 1));
                self.cursor = e - 1;
            }
            Some((s, e)) if self.cursor == s && s > 0 => {
                self.sel = Some((s - 1, e));
                self.cursor = s - 1;
            }
            _ => {
                if self.cursor > 0 {
                    self.sel = Some((self.cursor - 1, self.cursor));
                    self.cursor -= 1;
                }
            }
        }
        self.clamp_scroll();
    }

    /// Marks one more character to the right: the cursor walks right, the
    /// selection growing behind it, or shrinking when it walks back onto
    /// the anchor.
    fn select_right(&mut self) {
        let len = self.buffer.chars().count();
        match self.sel {
            Some((s, e)) if self.cursor == s && s < e => {
                self.sel = Some((s + 1, e));
                self.cursor = s + 1;
            }
            Some((s, e)) if self.cursor == e && e < len => {
                self.sel = Some((s, e + 1));
                self.cursor = e + 1;
            }
            _ => {
                if self.cursor < len {
                    self.sel = Some((self.cursor, self.cursor + 1));
                    self.cursor += 1;
                }
            }
        }
        self.clamp_scroll();
    }

    /// Adjusts the scroll so the cursor stays within the visible window:
    /// the field follows the cursor when it would otherwise leave.
    fn clamp_scroll(&mut self) {
        let len = self.buffer.chars().count();
        let visible = len.min(COLS);
        let max_scroll = len.saturating_sub(visible);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor > self.scroll + visible {
            self.scroll = self.cursor - visible;
        }
        self.scroll = self.scroll.min(max_scroll);
    }

    /// Draws the field at `rect` (its `[left, bottom, right, top]`), with
    /// the visible characters, the marked range, and the cursor.
    fn draw(&self, ctx: &mut frost::Context, rect: [f32; 4], font: &Arc<[u8]>, blink_on: bool) {
        // The field's background.
        ctx.rectangle(
            (rect[0] + rect[2]) / 2.0,
            (rect[1] + rect[3]) / 2.0,
            (rect[2] - rect[0]) / 2.0,
            (rect[3] - rect[1]) / 2.0,
            FIELD_BG,
            0.0,
        );
        let chars: Vec<char> = self.buffer.chars().collect();
        // The marked range, if any: a highlight behind the selected cells.
        if let Some((s, e)) = self.selection_range() {
            let vis_start = s.max(self.scroll);
            let vis_end = e.min(self.scroll + COLS).min(chars.len());
            if vis_start < vis_end {
                let x0 = rect[0] + PAD + (vis_start - self.scroll) as f32 * CHAR_W;
                let x1 = rect[0] + PAD + (vis_end - self.scroll) as f32 * CHAR_W;
                ctx.rectangle(
                    (x0 + x1) / 2.0,
                    (rect[1] + rect[3]) / 2.0,
                    (x1 - x0) / 2.0,
                    (rect[3] - rect[1]) / 2.0,
                    SEL,
                    0.5,
                );
            }
        }
        // The visible characters, one per cell from the field's left pad.
        let end = (self.scroll + COLS).min(chars.len());
        for (i, &ch) in chars[self.scroll..end].iter().enumerate() {
            let x = rect[0] + PAD + (i as f32 + 0.5) * CHAR_W;
            let y = (rect[1] + rect[3]) / 2.0 - RON_LIFT * SIZE;
            ctx.text(x, y, font, ch.to_string(), SIZE, 600.0, TEXT, 1.0);
        }
        // The cursor: a vertical line at the cursor's cell boundary, shown
        // only while the field is focused and the blink is in its visible
        // half.
        if self.focused && blink_on {
            let cx = rect[0] + PAD + (self.cursor - self.scroll) as f32 * CHAR_W;
            ctx.line(cx, rect[1] + 5.0, cx, rect[3] - 5.0, CURSOR, 1.5, 2.0);
        }
        // The focus box: a border around the field while it has the focus.
        if self.focused {
            ctx.line(rect[0], rect[1], rect[2], rect[1], CURSOR, 2.0, 2.0);
            ctx.line(rect[0], rect[3], rect[2], rect[3], CURSOR, 2.0, 2.0);
            ctx.line(rect[0], rect[1], rect[0], rect[3], CURSOR, 2.0, 2.0);
            ctx.line(rect[2], rect[1], rect[2], rect[3], CURSOR, 2.0, 2.0);
        }
    }
}

// --- The demo -----------------------------------------------------------

struct Demo {
    input: TextInput,
    label: String,
    blink_timer: f32,
    /// The left mouse button's state last frame (for press/release edges).
    was_down: bool,
    /// The right mouse button's state last frame (for the paste edge).
    was_rdown: bool,
    /// The arrow keys' and editing keys' state last frame.
    was_left: bool,
    was_right: bool,
    was_backspace: bool,
    was_delete: bool,
    was_enter: bool,
    was_esc: bool,
    /// The typed characters held last frame.
    was_typed: HashSet<char>,
    /// The clipboard shortcuts' state last frame.
    was_copy: bool,
    was_paste: bool,
    was_cut: bool,
    /// The repeat accumulators for the held keys.
    repeat_left: f32,
    repeat_right: f32,
    repeat_backspace: f32,
    repeat_delete: f32,
    repeat_typed: HashMap<char, f32>,
    /// The mouse-drag anchor (the cursor when the drag started), if any.
    text_drag: Option<usize>,
    /// The system clipboard the field's Ctrl-C / Ctrl-X / Ctrl-V read and
    /// write, created once (recreating it on every press would be wasteful).
    clipboard: Option<arboard::Clipboard>,
    font: Arc<[u8]>,
}

/// Whether a held key should act this frame: the press edge acts at once
/// and arms the first repeat, a held key repeats every `REPEAT` seconds
/// after an `INITIAL_DELAY` pause, and a release resets the accumulator.
fn key_tick(held: bool, was: bool, acc: &mut f32, dt: f32) -> bool {
    if !held {
        *acc = 0.0;
        return false;
    }
    if !was {
        *acc = REPEAT - INITIAL_DELAY;
        return true;
    }
    *acc += dt;
    if *acc >= REPEAT {
        *acc -= REPEAT;
        true
    } else {
        false
    }
}

/// The characters the field accepts, in the order `char_down` is polled.
const TYPABLE: &[char] = &[
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L',
    'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '0', '1', '2', '3', '4',
    '5', '6', '7', '8', '9', ' ', '.', ',', '!', '?', '-', '_', '(', ')', '/',
];

/// The field's rect, centered on the window's x axis at `INPUT_CY`.
fn field_rect() -> [f32; 4] {
    [-W / 2.0, INPUT_CY - H / 2.0, W / 2.0, INPUT_CY + H / 2.0]
}

/// Whether `p` is inside the rect `r` (`[left, bottom, right, top]`).
fn in_rect(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3]
}

impl Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let rect = field_rect();

        // The mouse buttons' edges.
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let rdown = ctx.mouse_button_down(frost::MouseButton::Right);
        let rpressed = rdown && !self.was_rdown;
        self.was_rdown = rdown;
        let pressed = down && !self.was_down;
        let released = !down && self.was_down;
        self.was_down = down;
        let pos = ctx.mouse_position();

        // Focus: a press in the field focuses it, places the cursor and
        // starts a mouse drag that marks from the cursor; a press
        // elsewhere unfocuses it.
        if pressed && let Some(p) = pos {
            if in_rect(rect, p) {
                self.input.set_focus(true);
                self.input.click_at(p[0], rect);
                self.text_drag = Some(self.input.cursor);
                self.blink_timer = 0.0;
            } else {
                self.input.set_focus(false);
                self.text_drag = None;
            }
        }
        // A held left button in the focused field drags the selection from
        // its anchor to the cursor's cell; releasing the button ends the
        // drag.
        if down
            && self.input.focused
            && let Some(anchor) = self.text_drag
            && let Some(p) = pos
            && in_rect(rect, p)
        {
            self.input.drag_to(anchor, p[0], rect);
        }
        if released {
            self.text_drag = None;
        }
        // A right-click in the focused field pastes the clipboard.
        if rpressed
            && self.input.focused
            && let Some(p) = pos
            && in_rect(rect, p)
            && let Some(clip) = self.clipboard.as_mut()
            && let Ok(text) = clip.get_text()
        {
            let text: String = text.chars().filter(|c| *c != '\r' && *c != '\n').collect();
            self.input.insert_str(&text);
        }

        // The cursor blink clock.
        self.blink_timer = (self.blink_timer + dt) % BLINK;
        let blink_on = self.blink_timer < BLINK / 2.0;

        // Escape's rising edge: a focused field loses its focus.
        let esc = ctx.key_down(frost::KeyCode::Escape);
        if esc && !self.was_esc && self.input.focused {
            self.input.set_focus(false);
            self.text_drag = None;
        }
        self.was_esc = esc;

        // The field's keys act only while it is focused. Each held key acts
        // on its press and then repeats after the initial delay, every
        // `REPEAT` seconds; the ticks are computed here, outside the focus
        // check, so a key held while unfocused never fires a stale repeat
        // on refocus.
        let shift =
            ctx.key_down(frost::KeyCode::ShiftLeft) || ctx.key_down(frost::KeyCode::ShiftRight);
        let ctrl =
            ctx.key_down(frost::KeyCode::ControlLeft) || ctx.key_down(frost::KeyCode::ControlRight);
        let left = ctx.key_down(frost::KeyCode::ArrowLeft);
        let right = ctx.key_down(frost::KeyCode::ArrowRight);
        let backspace = ctx.key_down(frost::KeyCode::Backspace);
        let delete = ctx.key_down(frost::KeyCode::Delete);
        let enter = ctx.key_down(frost::KeyCode::Enter);
        let tick_left = key_tick(left, self.was_left, &mut self.repeat_left, dt);
        let tick_right = key_tick(right, self.was_right, &mut self.repeat_right, dt);
        let tick_backspace = key_tick(
            backspace,
            self.was_backspace,
            &mut self.repeat_backspace,
            dt,
        );
        let tick_delete = key_tick(delete, self.was_delete, &mut self.repeat_delete, dt);
        let mut tick_typed = HashSet::new();
        for &ch in TYPABLE {
            let held = !ctrl && ctx.char_down(ch);
            let was = self.was_typed.contains(&ch);
            let acc = self.repeat_typed.entry(ch).or_insert(0.0);
            if key_tick(held, was, acc, dt) {
                tick_typed.insert(ch);
            }
        }
        let copy_key = ctrl && ctx.key_down(frost::KeyCode::KeyC);
        let paste_key = ctrl && ctx.key_down(frost::KeyCode::KeyV);
        let cut_key = ctrl && ctx.key_down(frost::KeyCode::KeyX);

        if self.input.focused {
            // Enter commits the buffer to the label.
            if enter && !self.was_enter {
                self.label = self.input.buffer.clone();
            }
            if tick_left {
                if shift {
                    self.input.select_left();
                } else {
                    self.input.move_left();
                }
            }
            if tick_right {
                if shift {
                    self.input.select_right();
                } else {
                    self.input.move_right();
                }
            }
            if tick_backspace {
                self.input.backspace();
            }
            if tick_delete {
                self.input.delete();
            }
            for &ch in &tick_typed {
                self.input.insert(ch);
            }
            // Ctrl-C copies the mark (or the whole buffer when nothing is
            // marked) to the clipboard.
            if copy_key && !self.was_copy {
                let text = self.input.selected_text();
                let text = if text.is_empty() {
                    self.input.buffer.clone()
                } else {
                    text
                };
                if let Some(clip) = self.clipboard.as_mut() {
                    let _ = clip.set_text(text);
                }
            }
            // Ctrl-V pastes the clipboard at the cursor, replacing any mark.
            if paste_key
                && !self.was_paste
                && let Some(clip) = self.clipboard.as_mut()
                && let Ok(text) = clip.get_text()
            {
                let text: String = text.chars().filter(|c| *c != '\r' && *c != '\n').collect();
                self.input.insert_str(&text);
            }
            // Ctrl-X cuts the mark to the clipboard and deletes it.
            if cut_key
                && !self.was_cut
                && let Some(clip) = self.clipboard.as_mut()
            {
                let text = self.input.selected_text();
                if !text.is_empty() {
                    let _ = clip.set_text(text);
                    self.input.delete_selection();
                }
            }
        }

        // Update the edge state for next frame.
        self.was_enter = enter;
        self.was_left = left;
        self.was_right = right;
        self.was_backspace = backspace;
        self.was_delete = delete;
        self.was_copy = copy_key;
        self.was_paste = paste_key;
        self.was_cut = cut_key;
        self.was_typed = TYPABLE
            .iter()
            .copied()
            .filter(|&ch| !ctrl && ctx.char_down(ch))
            .collect();

        // Draw the field.
        self.input.draw(ctx, rect, &self.font, blink_on);

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

    let scene = frost::Scene::new(frost::SceneNode {
        shape: Some(frost::Shape::Background { color: BG }),
        ..Default::default()
    });

    if let Err(err) = frost::run_configured(
        scene,
        Demo {
            input: TextInput::new(),
            label: String::new(),
            blink_timer: 0.0,
            was_down: false,
            was_rdown: false,
            was_left: false,
            was_right: false,
            was_backspace: false,
            was_delete: false,
            was_enter: false,
            was_esc: false,
            was_typed: HashSet::new(),
            was_copy: false,
            was_paste: false,
            was_cut: false,
            repeat_left: 0.0,
            repeat_right: 0.0,
            repeat_backspace: 0.0,
            repeat_delete: 0.0,
            repeat_typed: HashMap::new(),
            text_drag: None,
            clipboard: arboard::Clipboard::new().ok(),
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
