//! A single-line text input: the buffer, the cursor, the mark, and the
//! keyboard and mouse logic, shared by two thin adapters that put it on
//! screen.
//!
//! The core [`TextInput`] owns the editing state and the input handling but
//! draws nothing. The adapters are:
//! - [`Ui::text_input`] — an immediate-mode widget in the [`Ui`] layer,
//!   which draws the field with the canvas and keeps one [`FieldState`] per
//!   widget id.
//! - [`TextInput::update_scene`] — syncs a scene-tree node's shapes (a
//!   rectangle, a text node, and two polylines) from the same state.
//!
//! The field shows `COLS` characters at a time and scrolls the rest past as
//! the cursor moves. It keeps a marked range (the selection) that the
//! clipboard and the editing keys act on.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::{Color, Context, KeyCode, SceneNode, Shape, Transform};
use super::{Rect, Ui, UiStyle};

/// The field's font size, in user units.
const SIZE: f32 = 18.0;
/// The monospace advance, in ems of `SIZE` (the font is monospaced).
const ADVANCE_EM: f32 = 0.6;
/// One character's width, in user units.
const CHAR_W: f32 = SIZE * ADVANCE_EM;
/// How many characters fit in the field at once.
const COLS: usize = 16;
/// The padding between the field's edge and its text.
const PAD: f32 = 10.0;
/// The field's height.
const H: f32 = 40.0;
/// The field's width.
const W: f32 = COLS as f32 * CHAR_W + 2.0 * PAD;
/// The buffer's capacity, in characters.
const MAX: usize = 64;
/// The text's lift below the field's centerline, in user units: a font
/// placed at a center prints its ink a cap above it, so the origin is
/// dropped this far to bring the text to the visual middle.
const RON_LIFT: f32 = 0.344;
/// The cursor blink period, in seconds: visible for the first half, hidden
/// for the second.
const BLINK: f32 = 1.0;
/// The first repeat delay, in seconds, after a key is held.
const INITIAL_DELAY: f32 = 0.3;
/// The repeat interval, in seconds, while a key is held.
const REPEAT: f32 = 0.075;

/// The characters the field accepts, in the order `char_down` is polled.
const TYPABLE: &[char] = &[
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L',
    'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '0', '1', '2', '3', '4',
    '5', '6', '7', '8', '9', ' ', '.', ',', '!', '?', '-', '_', '(', ')', '/',
];

/// A single-line text field: a fixed-width buffer that shows `COLS`
/// characters at a time and scrolls the rest past as the cursor moves. It
/// keeps a marked range (the selection) that the clipboard and the editing
/// keys act on.
///
/// The state and the input handling live here; drawing does not. Feed it
/// each frame with [`process_keys`](Self::process_keys) and
/// [`process_mouse`](Self::process_mouse), then read it back through
/// [`buffer`](Self::buffer) or push it onto the screen with
/// [`update_scene`](Self::update_scene) or the [`Ui::text_input`] widget.
pub struct TextInput {
    buffer: String,
    cursor: usize,
    scroll: usize,
    focused: bool,
    /// The marked range `(start, end)`, `start <= end`, the cursor at one
    /// end; `None` when nothing is marked.
    sel: Option<(usize, usize)>,
    /// The cursor blink clock, in seconds, wrapped at `BLINK`.
    blink: f32,
    /// The keyboard's edge and repeat state, spanning frames.
    keys: KeyState,
    /// The mouse's edge and drag state, spanning frames.
    mouse: MouseState,
    /// The system clipboard, created lazily on first use: `None` where the
    /// platform has no clipboard (a web build, a headless test).
    clipboard: Option<arboard::Clipboard>,
}

/// The keyboard's state spanning frames: the previous frame's key edges and
/// the per-key repeat accumulators.
#[derive(Default)]
struct KeyState {
    was_left: bool,
    was_right: bool,
    was_backspace: bool,
    was_delete: bool,
    was_enter: bool,
    was_esc: bool,
    was_copy: bool,
    was_paste: bool,
    was_cut: bool,
    /// The typable characters held last frame.
    was_typed: HashSet<char>,
    repeat_left: f32,
    repeat_right: f32,
    repeat_backspace: f32,
    repeat_delete: f32,
    /// Each typable character's repeat accumulator.
    repeat_typed: HashMap<char, f32>,
}

/// The mouse's state spanning frames: the previous frame's button edges and
/// the in-flight selection drag.
#[derive(Default)]
struct MouseState {
    was_down: bool,
    was_rdown: bool,
    /// The cursor column a held left button is dragging the selection from.
    drag: Option<usize>,
}

impl TextInput {
    /// An empty, unfocused field.
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            scroll: 0,
            focused: false,
            sel: None,
            blink: 0.0,
            keys: KeyState::default(),
            mouse: MouseState::default(),
            clipboard: None,
        }
    }

    /// Gains or loses the focus. Gaining it restarts the cursor blink;
    /// losing it clears the mark.
    pub fn set_focus(&mut self, focused: bool) {
        if focused && !self.focused {
            self.blink = 0.0;
        }
        self.focused = focused;
        if !focused {
            self.sel = None;
        }
    }

    /// Places the cursor at the character under `x` (in the field's
    /// `[left, bottom, right, top]` rect) and clears the mark.
    pub fn click_at(&mut self, x: f32, rect: [f32; 4]) {
        let col = (((x - rect[0]) - PAD) / CHAR_W).floor() as usize;
        let cell = col.min(COLS);
        self.cursor = (self.scroll + cell).min(self.buffer.chars().count());
        self.sel = None;
        self.clamp_scroll();
    }

    /// Extends the mark from `anchor` to the character under `x`, putting
    /// the cursor at the new end.
    pub fn drag_to(&mut self, anchor: usize, x: f32, rect: [f32; 4]) {
        let col = (((x - rect[0]) - PAD) / CHAR_W).floor() as usize;
        let cell = col.min(COLS);
        let idx = (self.scroll + cell).min(self.buffer.chars().count());
        let (s, e) = if anchor <= idx {
            (anchor, idx)
        } else {
            (idx, anchor)
        };
        self.sel = Some((s, e));
        self.cursor = idx;
    }

    /// The marked range, or `None` when nothing (or only a zero-width
    /// point) is marked.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.sel.filter(|&(s, e)| s < e)
    }

    /// The marked characters, or the empty string when nothing is marked.
    pub fn selected_text(&self) -> String {
        match self.selection_range() {
            Some((s, e)) => self.buffer.chars().skip(s).take(e - s).collect(),
            None => String::new(),
        }
    }

    /// Deletes the mark, if any; reports whether it did.
    pub fn delete_selection(&mut self) -> bool {
        match self.selection_range() {
            Some((s, e)) => {
                let mut chars: Vec<char> = self.buffer.chars().collect();
                chars.drain(s..e);
                self.buffer = chars.into_iter().collect();
                self.cursor = s;
                self.sel = None;
                self.clamp_scroll();
                true
            }
            None => false,
        }
    }

    /// Inserts `ch` at the cursor, replacing the mark if there is one, up
    /// to the buffer's capacity.
    pub fn insert(&mut self, ch: char) {
        self.delete_selection();
        if self.buffer.chars().count() < MAX {
            let mut chars: Vec<char> = self.buffer.chars().collect();
            chars.insert(self.cursor, ch);
            self.buffer = chars.into_iter().collect();
            self.cursor += 1;
        }
        self.clamp_scroll();
    }

    /// Inserts the whole string `s` at the cursor, replacing the mark if
    /// there is one, truncated to the buffer's capacity.
    pub fn insert_str(&mut self, s: &str) {
        self.delete_selection();
        let mut chars: Vec<char> = self.buffer.chars().collect();
        let room = MAX.saturating_sub(chars.len());
        let take: Vec<char> = s.chars().take(room).collect();
        chars.splice(self.cursor..self.cursor, take.iter().cloned());
        self.cursor += take.len();
        self.buffer = chars.into_iter().collect();
        self.clamp_scroll();
    }

    /// Deletes the mark, or the character before the cursor.
    pub fn backspace(&mut self) {
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

    /// Deletes the mark, or the character at the cursor.
    pub fn delete(&mut self) {
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

    /// Moves the cursor left, clearing the mark.
    pub fn move_left(&mut self) {
        self.sel = None;
        self.cursor = self.cursor.saturating_sub(1);
        self.clamp_scroll();
    }

    /// Moves the cursor right, clearing the mark.
    pub fn move_right(&mut self) {
        self.sel = None;
        self.cursor = (self.cursor + 1).min(self.buffer.chars().count());
        self.clamp_scroll();
    }

    /// Extends the mark one character left.
    pub fn select_left(&mut self) {
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

    /// Extends the mark one character right.
    pub fn select_right(&mut self) {
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

    /// Keeps the cursor inside the visible window of `COLS` characters.
    pub fn clamp_scroll(&mut self) {
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

    /// Replaces the buffer with `s`, putting the cursor at the end and
    /// clearing the mark.
    pub fn adopt(&mut self, s: String) {
        self.buffer = s;
        self.cursor = self.buffer.chars().count();
        self.sel = None;
        self.clamp_scroll();
    }

    /// The buffer's text.
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// Whether the field has the focus.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// Whether the cursor is in its visible half of the blink cycle.
    pub fn blink_on(&self) -> bool {
        self.blink < BLINK / 2.0
    }

    /// The system clipboard, created on first use and `None` where the
    /// platform has none.
    fn clipboard_mut(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    /// Handles the frame's keyboard: advances the cursor blink, reads the
    /// modifier and editing keys, and applies their actions to the buffer
    /// when the field is focused. Returns `true` on the frame **Enter** is
    /// pressed, so the caller can commit the buffer.
    ///
    /// The repeat ticks are computed even when the field is unfocused, so a
    /// key held while unfocused does not fire a stale repeat the moment it
    /// regains the focus.
    pub fn process_keys(&mut self, ctx: &Context, dt: f32) -> bool {
        // The cursor blink clock.
        self.blink = (self.blink + dt) % BLINK;

        // Escape's rising edge: a focused field loses its focus.
        let esc = ctx.key_down(KeyCode::Escape);
        if esc && !self.keys.was_esc && self.focused {
            self.set_focus(false);
            self.mouse.drag = None;
        }
        self.keys.was_esc = esc;

        // The modifier and editing keys' states.
        let shift = ctx.key_down(KeyCode::ShiftLeft) || ctx.key_down(KeyCode::ShiftRight);
        let ctrl = ctx.key_down(KeyCode::ControlLeft) || ctx.key_down(KeyCode::ControlRight);
        let left = ctx.key_down(KeyCode::ArrowLeft);
        let right = ctx.key_down(KeyCode::ArrowRight);
        let backspace = ctx.key_down(KeyCode::Backspace);
        let delete = ctx.key_down(KeyCode::Delete);
        let enter = ctx.key_down(KeyCode::Enter);

        // The repeat ticks, computed outside the focus check so a key held
        // while the field is unfocused doesn't fire a stale repeat on focus.
        let tick_left = key_tick(left, self.keys.was_left, &mut self.keys.repeat_left, dt);
        let tick_right = key_tick(right, self.keys.was_right, &mut self.keys.repeat_right, dt);
        let tick_backspace =
            key_tick(backspace, self.keys.was_backspace, &mut self.keys.repeat_backspace, dt);
        let tick_delete = key_tick(delete, self.keys.was_delete, &mut self.keys.repeat_delete, dt);
        let mut tick_typed: HashSet<char> = HashSet::new();
        for &ch in TYPABLE {
            let held = !ctrl && ctx.char_down(ch);
            let was = self.keys.was_typed.contains(&ch);
            let tick = key_tick(held, was, self.keys.repeat_typed.entry(ch).or_insert(0.0), dt);
            if tick {
                tick_typed.insert(ch);
            }
        }
        let copy_key = ctrl && ctx.key_down(KeyCode::KeyC);
        let paste_key = ctrl && ctx.key_down(KeyCode::KeyV);
        let cut_key = ctrl && ctx.key_down(KeyCode::KeyX);

        let mut committed = false;
        if self.focused {
            // Enter commits the buffer.
            if enter && !self.keys.was_enter {
                committed = true;
            }
            // Arrow keys move or extend the mark.
            if tick_left {
                if shift {
                    self.select_left();
                } else {
                    self.move_left();
                }
            }
            if tick_right {
                if shift {
                    self.select_right();
                } else {
                    self.move_right();
                }
            }
            if tick_backspace {
                self.backspace();
            }
            if tick_delete {
                self.delete();
            }
            // Typed characters insert at the cursor.
            for &ch in &tick_typed {
                self.insert(ch);
            }
            // Clipboard shortcuts. The text is read before the clipboard is
            // borrowed, so the two borrows of `self` never overlap.
            if copy_key && !self.keys.was_copy {
                let text = if self.selection_range().is_some() {
                    self.selected_text()
                } else {
                    self.buffer.clone()
                };
                if let Some(clip) = self.clipboard_mut() {
                    let _ = clip.set_text(text);
                }
            }
            if paste_key && !self.keys.was_paste {
                if let Some(clip) = self.clipboard_mut()
                    && let Ok(text) = clip.get_text()
                {
                    let text: String = text.chars().filter(|c| *c != '\r' && *c != '\n').collect();
                    self.insert_str(&text);
                }
            }
            if cut_key && !self.keys.was_cut {
                let text = self.selected_text();
                if !text.is_empty() {
                    if let Some(clip) = self.clipboard_mut() {
                        let _ = clip.set_text(text);
                    }
                    self.delete_selection();
                }
            }
        }

        // Update the edge state for next frame.
        self.keys.was_enter = enter;
        self.keys.was_left = left;
        self.keys.was_right = right;
        self.keys.was_backspace = backspace;
        self.keys.was_delete = delete;
        self.keys.was_copy = copy_key;
        self.keys.was_paste = paste_key;
        self.keys.was_cut = cut_key;
        self.keys.was_typed = TYPABLE
            .iter()
            .copied()
            .filter(|&ch| !ctrl && ctx.char_down(ch))
            .collect();

        committed
    }

    /// Handles the frame's mouse: a press in the field focuses it, places
    /// the cursor, and starts a selection drag; a press elsewhere
    /// unfocuses; a held left button in the focused field drags the mark;
    /// and a right-click in the focused field pastes the clipboard.
    ///
    /// `rect` is the field's `[left, bottom, right, top]` rect in the same
    /// space as `pos`.
    pub fn process_mouse(
        &mut self,
        pos: Option<[f32; 2]>,
        down: bool,
        right_down: bool,
        rect: [f32; 4],
    ) {
        let rpressed = right_down && !self.mouse.was_rdown;
        self.mouse.was_rdown = right_down;
        let pressed = down && !self.mouse.was_down;
        let released = !down && self.mouse.was_down;
        self.mouse.was_down = down;

        // A press in the field focuses it, places the cursor, and starts a
        // drag; a press elsewhere unfocuses.
        if pressed {
            if let Some(p) = pos {
                if in_rect(rect, p) {
                    self.set_focus(true);
                    self.click_at(p[0], rect);
                    self.mouse.drag = Some(self.cursor);
                } else {
                    self.set_focus(false);
                    self.mouse.drag = None;
                }
            }
        }
        // A held left button in the focused field drags the selection.
        if down
            && self.focused
            && let Some(anchor) = self.mouse.drag
            && let Some(p) = pos
            && in_rect(rect, p)
        {
            self.drag_to(anchor, p[0], rect);
        }
        if released {
            self.mouse.drag = None;
        }
        // A right-click in the focused field pastes the clipboard.
        if rpressed
            && self.focused
            && let Some(p) = pos
            && in_rect(rect, p)
            && let Some(clip) = self.clipboard_mut()
            && let Ok(text) = clip.get_text()
        {
            let text: String = text.chars().filter(|c| *c != '\r' && *c != '\n').collect();
            self.insert_str(&text);
        }
    }

    /// Pushes the field's state onto a scene-tree node: the node's own
    /// rectangle is the field's background, and its four children, in
    /// order, are the selection highlight (a rectangle), the visible text
    /// (a text node), the cursor (a polyline), and the focus box (a
    /// polyline). The node's transform should place its origin at the
    /// field's center.
    pub fn update_scene(&self, field: &mut SceneNode, blink_on: bool) {
        let chars: Vec<char> = self.buffer.chars().collect();
        let end = (self.scroll + COLS).min(chars.len());
        let n = end - self.scroll;

        // The visible text, centered on the block it fills.
        if let Some(Shape::Text { text, .. }) = field.children[1].shape.as_mut() {
            *text = chars[self.scroll..end].iter().collect();
        }
        field.children[1].transform =
            Transform::translate([-W / 2.0 + PAD + n as f32 * CHAR_W / 2.0, -RON_LIFT * SIZE]);

        // The selection highlight.
        if let Some(Shape::Rectangle { center, extent, .. }) = field.children[0].shape.as_mut() {
            match self.selection_range() {
                Some((s, e)) => {
                    let vis_start = s.max(self.scroll);
                    let vis_end = e.min(self.scroll + COLS).min(chars.len());
                    if vis_start < vis_end {
                        let span = (vis_end - vis_start) as f32 * CHAR_W;
                        *center = [
                            -W / 2.0 + PAD + (vis_start - self.scroll) as f32 * CHAR_W + span / 2.0,
                            0.0,
                        ];
                        *extent = [span / 2.0, H / 2.0];
                    } else {
                        *extent = [0.0, 0.0];
                    }
                }
                None => {
                    *extent = [0.0, 0.0];
                }
            }
        }

        // The cursor.
        if let Some(Shape::Polyline { points, .. }) = field.children[2].shape.as_mut() {
            if self.focused && blink_on {
                let cx = -W / 2.0 + PAD + (self.cursor - self.scroll) as f32 * CHAR_W;
                *points = vec![[cx, -H / 2.0 + 5.0], [cx, H / 2.0 - 5.0]];
            } else {
                points.clear();
            }
        }

        // The focus box.
        if let Some(Shape::Polyline { points, .. }) = field.children[3].shape.as_mut() {
            if self.focused {
                *points = vec![
                    [-W / 2.0, -H / 2.0],
                    [W / 2.0, -H / 2.0],
                    [W / 2.0, H / 2.0],
                    [-W / 2.0, H / 2.0],
                    [-W / 2.0, -H / 2.0],
                ];
            } else {
                points.clear();
            }
        }
    }
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

/// Whether `p` is inside the rect `r` (`[left, bottom, right, top]`).
fn in_rect(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3]
}

/// The per-widget state the [`Ui::text_input`] widget keeps in its
/// `text_fields` map: the field's [`TextInput`] plus the in-flight drag.
pub(super) struct FieldState {
    pub(super) input: TextInput,
    pub(super) drag: Option<usize>,
}

impl Default for FieldState {
    fn default() -> Self {
        Self {
            input: TextInput::new(),
            drag: None,
        }
    }
}

impl Ui {
    /// A single-line text input field. The field's text lives in `buffer`,
    /// which the widget keeps in sync with its own retained state: it
    /// adopts an out-of-band change to `buffer` and writes the edited text
    /// back each frame. Click the field to focus it, type, and press
    /// **Enter** to commit — the widget returns `true` on that frame.
    ///
    /// `dt` is the frame's duration in seconds, which the widget needs for
    /// the cursor blink and the key repeat (the [`Context`] carries no
    /// clock of its own).
    pub fn text_input(
        &mut self,
        ctx: &mut Context,
        label: &str,
        buffer: &mut String,
        dt: f32,
    ) -> bool {
        let id = self.widget_id(label);
        let row = self.row(H.max(self.style.row_h));
        let field = Rect::from_center(row.center()[0], row.center()[1], W, H);
        let it = self.interact(id, field);
        let st = self.text_fields.entry(id).or_default();

        // Adopt the caller's buffer if it changed out-of-band.
        if st.input.buffer != *buffer {
            st.input.adopt(std::mem::take(buffer));
        }

        // Focus: a press that lands in the field focuses it, places the
        // cursor, and starts a drag; a press elsewhere unfocuses.
        if self.pressed && self.press_pointer.is_some_and(|p| field.contains(p)) {
            st.input.set_focus(true);
            if let Some(p) = self.press_pointer {
                st.input.click_at(p[0], field.to_array());
            }
            st.drag = Some(st.input.cursor);
        } else if self.pressed {
            st.input.set_focus(false);
            st.drag = None;
        }

        // A held press in the focused field drags the selection.
        if it.held
            && it.pressed
            && let Some(anchor) = st.drag
            && let Some(p) = self.pointer
            && field.contains(p)
        {
            st.input.drag_to(anchor, p[0], field.to_array());
        }
        if !self.down {
            st.drag = None;
        }

        // The keyboard.
        let committed = st.input.process_keys(ctx, dt);

        // An unfocused field can't be dragging.
        if !st.input.focused {
            st.drag = None;
        }

        // Draw the field.
        let blink_on = st.input.blink_on();
        let mut z = self.z;
        draw_field(ctx, &field, &self.font, &st.input, blink_on, &mut z, &self.style);
        self.z = z;

        // Write the buffer back to the caller.
        *buffer = std::mem::take(&mut st.input.buffer);
        committed
    }
}

/// Draws the field: its background, the selection highlight, the visible
/// text, the cursor, and the focus box. Each primitive consumes one step of
/// the `z` counter, which the caller threads through so the field stacks
/// with the rest of the UI.
fn draw_field(
    ctx: &mut Context,
    field: &Rect,
    font: &Arc<[u8]>,
    input: &TextInput,
    blink_on: bool,
    z: &mut f32,
    style: &UiStyle,
) {
    let [cx, cy] = field.center();

    // The field's background.
    ctx.rectangle(cx, cy, field.w / 2.0, field.h / 2.0, style.widget_bg, *z);
    *z += 1.0;

    // The selection highlight.
    if let Some((s, e)) = input.selection_range() {
        let chars: Vec<char> = input.buffer.chars().collect();
        let vis_start = s.max(input.scroll);
        let vis_end = e.min(input.scroll + COLS).min(chars.len());
        if vis_start < vis_end {
            let x0 = field.left + PAD + (vis_start - input.scroll) as f32 * CHAR_W;
            let x1 = field.left + PAD + (vis_end - input.scroll) as f32 * CHAR_W;
            let sel = Color {
                a: 0.30,
                ..style.accent
            };
            ctx.rectangle((x0 + x1) / 2.0, cy, (x1 - x0) / 2.0, field.h / 2.0, sel, *z);
            *z += 1.0;
        }
    }

    // The visible text, one call for the whole block it fills.
    let chars: Vec<char> = input.buffer.chars().collect();
    let end = (input.scroll + COLS).min(chars.len());
    let n = end - input.scroll;
    if n > 0 {
        let visible: String = chars[input.scroll..end].iter().collect();
        let block_cx = field.left + PAD + n as f32 * CHAR_W / 2.0;
        let y = cy - RON_LIFT * SIZE;
        ctx.text(block_cx, y, font, visible, SIZE, style.font_weight, style.text, *z);
        *z += 1.0;
    }

    // The cursor.
    if input.focused && blink_on {
        let cx = field.left + PAD + (input.cursor - input.scroll) as f32 * CHAR_W;
        ctx.line(cx, field.top - field.h + 5.0, cx, field.top - 5.0, style.accent, 1.5, *z);
        *z += 1.0;
    }

    // The focus box.
    if input.focused {
        let l = field.left;
        let b = field.top - field.h;
        let r = field.left + field.w;
        let t = field.top;
        ctx.line(l, b, r, b, style.accent, 2.0, *z);
        *z += 1.0;
        ctx.line(l, t, r, t, style.accent, 2.0, *z);
        *z += 1.0;
        ctx.line(l, b, l, t, style.accent, 2.0, *z);
        *z += 1.0;
        ctx.line(r, b, r, t, style.accent, 2.0, *z);
        *z += 1.0;
    }
}
