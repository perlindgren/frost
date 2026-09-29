//! Reusable immediate-mode widgets: a [`Ui`] that draws [`button`](Ui::button),
//! [`checkbox`](Ui::checkbox), [`slider`](Ui::slider), [`label`](Ui::label)
//! and draggable [`panel`](Ui::panel) chrome straight onto the frame's
//! [`Canvas`], over whatever the game draws.
//!
//! The style follows the frame flow: declare your widgets every frame from
//! [`Process::process`](crate::Process::process), exactly like the immediate
//! canvas methods. The `Ui` keeps only the interaction state the immediate
//! style cannot re-derive: which widget holds the mouse, where each panel
//! has been dragged to, and how tall each panel's content grew.
//!
//! ```no_run
//! # struct Demo { ui: frost::Ui, volume: f32, sound: bool }
//! # impl frost::Process for Demo {
//! fn process(&mut self, ctx: &mut frost::Context, _dt: f32) {
//!     self.ui.begin(ctx);
//!     self.ui.panel(ctx, "Settings", [-300.0, 150.0], 260.0, |ui, ctx| {
//!         ui.slider(ctx, "Volume", &mut self.volume, 0.0, 1.0);
//!         ui.checkbox(ctx, "Sound on", &mut self.sound);
//!         if ui.button(ctx, "Reset") {
//!             self.volume = 0.5;
//!         }
//!     });
//! }
//! # }
//! ```
//!
//! **The input model.** Every widget hit-tests with the rects it registered
//! *last* frame, so press, drag and release resolve against stable geometry
//! even while a panel is moving under the cursor, and when two widgets
//! overlap the one declared last (drawn on top) wins the press. The cost is
//! one frame of latency on the press: a press that lands on a widget arms
//! the drag on the next frame, at 60 Hz an imperceptible 16 ms.
//!
//! **Identity.** Widget ids are hashed from the widget's label and the id
//! scope of the panel it is declared in, so the same label in two panels is
//! two widgets — but the same label *twice in one panel* is one widget hit
//! twice; give interactive widgets distinct labels.
//!
//! **Order.** Every draw the `Ui` records gets a `z` counting up from
//! [`UiStyle::base_z`] in declaration order, so the UI paints over the
//! scene (whose nodes default to `z = 0.0`) and panels stack in the order
//! they are declared.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::{Color, Context, TextError};

/// The FNV-1a parameters: a cheap, deterministic byte hash. Determinism
/// matters — widget ids key the drag capture and the panel positions, so
/// the same label must hash to the same id on every frame.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The look and metrics of a [`Ui`], all fields public: read [`Default`]
/// for the stock dark theme and overwrite what you want.
#[derive(Clone, Copy, Debug)]
pub struct UiStyle {
    /// The label text color.
    pub text: Color,
    /// The color of the dimmed value readout on the right of a slider.
    pub text_muted: Color,
    /// The panel body's background.
    pub panel_bg: Color,
    /// The panel title bar's background.
    pub title_bg: Color,
    /// The panel title text color.
    pub title_text: Color,
    /// The resting color of a button and a checkbox box.
    pub widget_bg: Color,
    /// The color of a hovered widget (and the title bar of a hovered panel).
    pub widget_hover: Color,
    /// The color of a widget held under the button.
    pub widget_press: Color,
    /// The accent: a checked box, a slider's filled track, its knob.
    pub accent: Color,
    /// A slider track's unfilled part.
    pub track: Color,
    /// The label and title text size in pixels per em.
    pub font_size: f32,
    /// The label and title text weight: the font's `wght` variation axis,
    /// where 400.0 is Regular and 700.0 a full bold. The default 520.0 is
    /// a Medium — a touch thicker than the plain Regular instance, which
    /// reads well at small sizes on a dark panel. A font without a weight
    /// axis (any static TTF) ignores it.
    pub font_weight: f32,
    /// The height of a widget row, a button and a title bar.
    pub row_h: f32,
    /// The vertical gap between two rows.
    pub row_gap: f32,
    /// The padding inside a panel, between body edge and rows.
    pub pad: f32,
    /// The side of a checkbox box.
    pub check_size: f32,
    /// The height of a slider's track.
    pub track_h: f32,
    /// The radius of a slider's knob.
    pub knob_r: f32,
    /// The width of the root column the widgets outside any panel stack in.
    pub root_width: f32,
    /// The root column's inset from the window's top-left corner.
    pub root_margin: f32,
    /// The `z` of the UI's first draw; every later draw counts up from it.
    pub base_z: f32,
}

impl Default for UiStyle {
    fn default() -> Self {
        Self {
            text: Color {
                r: 0.92,
                g: 0.94,
                b: 0.98,
                a: 1.0,
            },
            text_muted: Color {
                r: 0.62,
                g: 0.66,
                b: 0.74,
                a: 1.0,
            },
            panel_bg: Color {
                r: 0.10,
                g: 0.11,
                b: 0.15,
                a: 0.94,
            },
            title_bg: Color {
                r: 0.17,
                g: 0.20,
                b: 0.28,
                a: 1.0,
            },
            title_text: Color {
                r: 0.95,
                g: 0.96,
                b: 1.0,
                a: 1.0,
            },
            widget_bg: Color {
                r: 0.21,
                g: 0.24,
                b: 0.32,
                a: 1.0,
            },
            widget_hover: Color {
                r: 0.30,
                g: 0.35,
                b: 0.47,
                a: 1.0,
            },
            widget_press: Color {
                r: 0.15,
                g: 0.17,
                b: 0.23,
                a: 1.0,
            },
            accent: Color {
                r: 0.25,
                g: 0.45,
                b: 0.85,
                a: 1.0,
            },
            track: Color {
                r: 0.14,
                g: 0.15,
                b: 0.20,
                a: 1.0,
            },
            font_size: 15.0,
            font_weight: 520.0,
            row_h: 28.0,
            row_gap: 6.0,
            pad: 12.0,
            check_size: 20.0,
            track_h: 6.0,
            knob_r: 9.0,
            root_width: 240.0,
            root_margin: 16.0,
            base_z: 10_000.0,
        }
    }
}

/// A rectangle in user space: `left` and `top` are the top-left corner
/// (`top` being the *larger* y, user space being y-up), `w` and `h` its
/// extent.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    left: f32,
    top: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn from_center(cx: f32, cy: f32, w: f32, h: f32) -> Self {
        Self {
            left: cx - w / 2.0,
            top: cy + h / 2.0,
            w,
            h,
        }
    }

    fn center(&self) -> [f32; 2] {
        [self.left + self.w / 2.0, self.top - self.h / 2.0]
    }

    fn contains(&self, p: [f32; 2]) -> bool {
        p[0] >= self.left
            && p[0] <= self.left + self.w
            && p[1] <= self.top
            && p[1] >= self.top - self.h
    }
}

/// What one widget asks the `Ui` about its row rect: see [`Ui::interact`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Interaction {
    /// The pointer is over the widget, and no other widget holds it.
    hot: bool,
    /// This widget captured the press and is being dragged.
    held: bool,
    /// The button is down while this widget holds it.
    pressed: bool,
    /// The button was released over this widget, having been pressed on it.
    clicked: bool,
}

/// The row cursor a widget allocates from: the panel body, or the root
/// column outside any panel.
struct Layout {
    /// The content's left edge in user space.
    left: f32,
    /// The content's width: every row spans it.
    width: f32,
    /// The top edge of the next free row, moving down as rows are taken.
    cursor: f32,
    /// The id scope of the panel this layout belongs to (0 for the root).
    scope: u64,
}

/// What a panel keeps between frames: where it was dragged to and how much
/// room its content asked for, which is the body size the next frame draws.
#[derive(Clone, Copy, Debug)]
struct PanelState {
    /// The panel's center in user space.
    pos: [f32; 2],
    /// The panel's width as its `panel` call declared it.
    w: f32,
    /// The panel's height: title bar plus the content height it recorded
    /// last frame (the first frame shows the title bar only).
    h: f32,
}

/// The immediate-mode widget layer. Keep one per window, owned by your
/// [`Process`](crate::Process) alongside the state the widgets drive; call
/// [`Ui::begin`] once per frame, declare widgets, and mutate your game state
/// from their return values.
pub struct Ui {
    /// The font every label draws with, shared so all text hits one atlas.
    font: Arc<[u8]>,
    /// The look and metrics; public fields through [`Ui::style_mut`].
    pub style: UiStyle,
    // The frame's input snapshot, taken by `begin`.
    pointer: Option<[f32; 2]>,
    prev_pointer: Option<[f32; 2]>,
    down: bool,
    prev_down: bool,
    pressed: bool,
    released: bool,
    // The press bookkeeping spanning frames.
    /// Whether the previous frame saw a press edge.
    was_pressed: bool,
    /// The pointer where that press happened.
    press_pointer: Option<[f32; 2]>,
    /// The widget that captured the press and is being dragged.
    active: Option<u64>,
    /// The widget released this frame, if its press started on it.
    released_active: Option<u64>,
    /// The widget under the pointer, topmost first, resolved at `begin`
    /// from the rects the previous frame declared (see `hovering`).
    hover: Option<u64>,
    /// Every interactive rect declared so far this frame, in order.
    candidates: Vec<(u64, Rect)>,
    /// Each panel's retained position and size, keyed by its title id.
    panels: HashMap<u64, PanelState>,
    /// The layout stack: the root column, plus one frame per open panel.
    layout: Vec<Layout>,
    /// The next draw's `z`, counting up from `style.base_z`.
    z: f32,
    /// Measured text widths, keyed by the string and the bits of the size
    /// and weight.
    text_w: HashMap<(String, u32, u32), f32>,
}

impl Ui {
    /// Loads the UI font from `path`; the bytes live behind an [`Arc`] so
    /// every label shares one glyph atlas. A missing file or non-font
    /// arrives as [`TextError`].
    pub fn from_font(path: impl AsRef<Path>) -> Result<Self, TextError> {
        let bytes = std::fs::read(path.as_ref()).map_err(TextError::Io)?;
        Self::from_bytes(&bytes)
    }

    /// Builds the UI from font bytes already in memory, for example
    /// embedded with `include_bytes!` — the way to go where there is no
    /// file system, such as a web build.
    pub fn from_bytes(font: &[u8]) -> Result<Self, TextError> {
        swash::FontRef::from_index(font, 0).ok_or(TextError::InvalidFont)?;
        Ok(Self {
            font: Arc::from(font),
            style: UiStyle::default(),
            pointer: None,
            prev_pointer: None,
            down: false,
            prev_down: false,
            pressed: false,
            released: false,
            was_pressed: false,
            press_pointer: None,
            active: None,
            released_active: None,
            hover: None,
            candidates: Vec::new(),
            panels: HashMap::new(),
            layout: Vec::new(),
            z: 0.0,
            text_w: HashMap::new(),
        })
    }

    /// The style, for tweaking: the colors and metrics are plain fields.
    pub fn style_mut(&mut self) -> &mut UiStyle {
        &mut self.style
    }

    /// The panel's current center, `[x, y]` in user space, once the panel
    /// has been declared at least once.
    pub fn panel_position(&self, title: &str) -> Option<[f32; 2]> {
        self.panels.get(&panel_id(title)).map(|p| p.pos)
    }

    /// Starts a UI frame: snapshots the pointer and buttons, resolves the
    /// press that armed last frame onto its widget, drags the panel whose
    /// title bar holds the button, and resets the row cursor and the draw
    /// order. Call it once, before declaring any widget.
    pub fn begin(&mut self, ctx: &Context) {
        let input = UiInput {
            pointer: ctx.mouse_position(),
            down: ctx.mouse_button_down(crate::MouseButton::Left),
            size: ctx.size(),
        };
        self.begin_input(input);
    }

    /// Whether the pointer is over any widget of the UI: the same hit test
    /// the widgets themselves use, so a game that reads the mouse directly
    /// can tell a click on the UI from a click on its own scene. False
    /// until the frame after a panel first appears — the hit test runs on
    /// the previous frame's rects, as the press does.
    pub fn hovering(&self) -> bool {
        self.hover.is_some()
    }

    /// The input-free heart of [`Ui::begin`], separate so the interaction
    /// model is testable without a window.
    fn begin_input(&mut self, input: UiInput) {
        // A press seen last frame now resolves over the rects that frame
        // registered, last declared (topmost) first.
        let won = if self.was_pressed {
            self.press_pointer.and_then(|p| self.topmost(p))
        } else {
            None
        };
        // The hover resolves the same way, against the live pointer, and
        // feeds both the widget hover states and `hovering`.
        self.hover = input.pointer.and_then(|p| self.topmost(p));
        let pressed = input.down && !self.prev_down;
        let released = !input.down && self.prev_down;
        let drag = match (input.pointer, self.prev_pointer) {
            (Some(p), Some(q)) => [p[0] - q[0], p[1] - q[1]],
            _ => [0.0, 0.0],
        };
        if released {
            // The release belongs to the held widget — or, for a press of
            // a single frame, to the widget that just won the press.
            self.released_active = self.active.take().or(won);
        } else {
            self.released_active = None;
            if won.is_some() {
                self.active = won;
            }
        }
        if !input.down && !released {
            // Down state lost without a release edge (the cursor left the
            // window): drop the capture.
            self.active = None;
        }
        // A dragged panel's title bar holding the id: move with the pointer.
        if input.down
            && let Some(active) = self.active
            && let Some(panel) = self.panels.get_mut(&active)
        {
            panel.pos = [panel.pos[0] + drag[0], panel.pos[1] + drag[1]];
        }
        // The frame's snapshot.
        self.prev_pointer = input.pointer;
        self.pointer = input.pointer;
        self.down = input.down;
        self.prev_down = input.down;
        self.pressed = pressed;
        self.released = released;
        self.was_pressed = pressed;
        self.press_pointer = if pressed { input.pointer } else { None };
        // Per-frame scratch.
        self.candidates.clear();
        self.z = self.style.base_z;
        // The root column: widgets outside any panel stack from the
        // window's top-left corner downward.
        let margin = self.style.root_margin;
        let width = self.style.root_width;
        self.layout.clear();
        self.layout.push(Layout {
            left: -input.size.0 / 2.0 + margin,
            width,
            cursor: input.size.1 / 2.0 - margin,
            scope: 0,
        });
    }

    /// Takes a row `h` pixels tall from the current layout, moving the
    /// cursor below it.
    fn row(&mut self, h: f32) -> Rect {
        let layout = self
            .layout
            .last_mut()
            .expect("Ui layout stack is never empty");
        layout.cursor -= self.style.row_gap;
        layout.cursor -= h;
        Rect {
            left: layout.left,
            top: layout.cursor,
            w: layout.width,
            h,
        }
    }

    /// The last (topmost) rect of the previous frame that holds `p`, with
    /// its id — the frame's single hit test, shared by the press
    /// resolution and the hover.
    fn topmost(&self, p: [f32; 2]) -> Option<u64> {
        self.candidates
            .iter()
            .rev()
            .find(|(_, r)| r.contains(p))
            .map(|(id, _)| *id)
    }

    /// Asks whether the press, hold and release belong to `id` and its
    /// rect, and registers the rect so next frame's press can find it.
    fn interact(&mut self, id: u64, rect: Rect) -> Interaction {
        let held = self.active == Some(id);
        self.candidates.push((id, rect));
        Interaction {
            // Hover shows only while no other widget holds the press.
            hot: self.hover == Some(id) && (held || self.active.is_none()),
            held,
            pressed: held && self.down,
            clicked: self.released_active == Some(id) && self.hover == Some(id),
        }
    }

    /// The next draw's `z`.
    fn z(&mut self) -> f32 {
        let z = self.z;
        self.z += 1.0;
        z
    }

    /// Fills `rect` on the canvas at the next `z`.
    fn fill(&mut self, ctx: &mut Context, rect: Rect, color: Color) {
        let [cx, cy] = rect.center();
        ctx.rectangle(cx, cy, rect.w / 2.0, rect.h / 2.0, color, self.z());
    }

    /// Draws centered text at `p`, at the style's label weight.
    fn text(&mut self, ctx: &mut Context, p: [f32; 2], size: f32, color: Color, s: &str) {
        let z = self.z();
        let weight = self.style.font_weight;
        ctx.text(p[0], p[1], &self.font, s, size, weight, color, z);
    }

    /// Draws `s` centered on the x `center` of the line `y`.
    fn text_at_x(
        &mut self,
        ctx: &mut Context,
        center: f32,
        y: f32,
        size: f32,
        color: Color,
        s: &str,
    ) {
        self.text(ctx, [center, y], size, color, s);
    }

    /// The advance width of `s` at the given size and the style's label
    /// weight, measured once per (string, size, weight) and remembered.
    fn text_width(&mut self, s: &str, size: f32) -> f32 {
        let weight = self.style.font_weight;
        let key = (s.to_owned(), size.to_bits(), weight.to_bits());
        if let Some(w) = self.text_w.get(&key) {
            return *w;
        }
        let w = crate::text::layout(&self.font, s, size, weight)
            .map(|l| l.width)
            .unwrap_or(0.0);
        self.text_w.insert(key, w);
        w
    }

    /// The widget id: the label hashed into the current panel's scope.
    fn widget_id(&self, label: &str) -> u64 {
        let scope = self
            .layout
            .last()
            .expect("layout stack is never empty")
            .scope;
        widget_id(scope, label)
    }

    /// A plain line of text, no interaction.
    pub fn label(&mut self, ctx: &mut Context, text: &str) {
        let row = self.row(self.style.row_h);
        let [cx, cy] = row.center();
        let size = self.style.font_size;
        let color = self.style.text;
        self.text(ctx, [cx, cy], size, color, text);
    }

    /// Spacing of `h` pixels, pushing the rows below it down.
    pub fn space(&mut self, h: f32) {
        let layout = self.layout.last_mut().expect("layout stack is never empty");
        layout.cursor -= h;
    }

    /// A push button spanning the row; returns `true` on the frame it is
    /// clicked — the button pressed on it released over it, the same arming
    /// as [`Shape`](crate::Shape)-level buttons.
    pub fn button(&mut self, ctx: &mut Context, label: &str) -> bool {
        let id = self.widget_id(label);
        let row = self.row(self.style.row_h);
        let it = self.interact(id, row);
        let style = self.style;
        let color = if it.pressed {
            style.widget_press
        } else if it.hot {
            style.widget_hover
        } else {
            style.widget_bg
        };
        self.fill(ctx, row, color);
        let size = style.font_size;
        let text_color = style.text;
        let [cx, cy] = row.center();
        let label = label.to_owned();
        self.text(ctx, [cx, cy], size, text_color, &label);
        it.clicked
    }

    /// A checkbox with its label to the right of the box, driving `value`;
    /// returns `true` on the frame it toggles. The whole row is the click
    /// target, so the label toggles it too.
    pub fn checkbox(&mut self, ctx: &mut Context, label: &str, value: &mut bool) -> bool {
        let id = self.widget_id(label);
        let row = self.row(self.style.row_h.max(self.style.check_size));
        let it = self.interact(id, row);
        if it.clicked {
            *value = !*value;
        }
        let style = self.style;
        let side = style.check_size;
        let box_rect = Rect::from_center(row.left + side / 2.0, row.center()[1], side, side);
        let box_color = if *value {
            if it.pressed {
                style.widget_press.lerp(style.accent, 0.6)
            } else if it.hot {
                style.accent.lerp(
                    Color {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                    0.15,
                )
            } else {
                style.accent
            }
        } else if it.pressed {
            style.widget_press
        } else if it.hot {
            style.widget_hover
        } else {
            style.widget_bg
        };
        self.fill(ctx, box_rect, box_color);
        if *value {
            // The tick: a short leg down-right and a long leg up-right.
            let [cx, cy] = box_rect.center();
            let z = self.z();
            let dark = Color {
                r: 0.05,
                g: 0.07,
                b: 0.10,
                a: 1.0,
            };
            ctx.line(
                cx - 0.30 * side,
                cy - 0.02 * side,
                cx - 0.10 * side,
                cy - 0.24 * side,
                dark,
                2.5,
                z,
            );
            ctx.line(
                cx - 0.10 * side,
                cy - 0.24 * side,
                cx + 0.32 * side,
                cy + 0.24 * side,
                dark,
                2.5,
                z + 0.5,
            );
        }
        let size = style.font_size;
        let text_color = style.text;
        let label = label.to_owned();
        let w = self.text_width(&label, size);
        let x = row.left + side + 8.0;
        let cy = row.center()[1];
        self.text_at_x(ctx, x + w / 2.0, cy, size, text_color, &label);
        it.clicked
    }

    /// A horizontal slider labelled on its left, driving `value` clamped
    /// into `min..=max` and shown at the row's right; returns `true` on the
    /// frames its value changed. Pressing the track jumps the value to the
    /// press point, dragging follows the pointer.
    pub fn slider(
        &mut self,
        ctx: &mut Context,
        label: &str,
        value: &mut f32,
        min: f32,
        max: f32,
    ) -> bool {
        let id = self.widget_id(label);
        let row = self.row(self.style.row_h);
        let label_w = self.text_width(label, self.style.font_size);
        let value_s = format!("{value:.2}");
        let value_w = self.text_width(&value_s, self.style.font_size);
        let style = self.style;
        let pad = style.pad;
        let knob = style.knob_r;
        // The track between the label column and the value readout.
        let x0 = row.left + label_w + pad;
        let x1 = row.left + row.w - value_w - pad;
        let track = if x1 > x0 {
            Rect::from_center(
                (x0 + x1) / 2.0,
                row.center()[1],
                x1 - x0,
                style.track_h.max(2.0 * knob),
            )
        } else {
            row
        };
        let it = self.interact(id, track);
        // A range (or a track) too small to travel has nothing to drive,
        // but still draws.
        let travel = max > min && track.w > 2.0 * knob;
        // While held the knob follows the pointer, clamped to the knob's
        // travel; a click (a press-and-release of a frame) sets it too.
        let mut changed = false;
        if travel
            && (it.held || it.clicked)
            && let Some(p) = self.pointer
        {
            let (lo, hi) = (track.left + knob, track.left + track.w - knob);
            let t = ((p[0] - lo) / (hi - lo)).clamp(0.0, 1.0);
            let v = min + t * (max - min);
            if v != *value {
                *value = v;
                changed = true;
            }
        }
        // The readout shows the value as it now stands, after any drag.
        let value_s = format!("{value:.2}");
        let value_w = self.text_width(&value_s, style.font_size);
        let t = if travel {
            ((*value - min) / (max - min)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let knob_x = track.left + knob + t * (track.w - 2.0 * knob).max(0.0);
        self.fill(ctx, track, style.track);
        let fill = Rect {
            left: track.left + knob,
            top: track.top,
            w: (knob_x - track.left - knob).max(0.0),
            h: track.h,
        };
        if fill.w > 0.0 {
            self.fill(ctx, fill, style.accent);
        }
        // The knob.
        let kc = [knob_x, track.center()[1]];
        let knob_color = if it.held {
            style.accent.lerp(
                Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                0.35,
            )
        } else if it.hot {
            style.accent.lerp(
                Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                0.15,
            )
        } else {
            style.accent
        };
        let z = self.z();
        ctx.circle(kc[0], kc[1], knob, knob_color, z);
        // The label left-aligned, the value right-aligned.
        let size = style.font_size;
        let text_color = style.text;
        let muted = style.text_muted;
        let label = label.to_owned();
        let cy = row.center()[1];
        self.text_at_x(ctx, row.left + label_w / 2.0, cy, size, text_color, &label);
        self.text_at_x(
            ctx,
            row.left + row.w - value_w / 2.0,
            cy,
            size,
            muted,
            &value_s,
        );
        changed
    }

    /// A panel: a titled, draggable box at center `at` (user-space pixels,
    /// until the user drags it somewhere else) of the given `width`, running
    /// `content` with its rows laid out in the body.
    ///
    /// The body's height is what the content asked for *last* frame, so the
    /// first frame shows the title bar only and the body snaps open on the
    /// second. Dragging the title bar moves the panel; its position then
    /// survives every frame and `at` is ignored — see [`Ui::panel_position`]
    /// to read it back.
    ///
    /// The panel claims the space it covers: the body absorbs presses and
    /// counts as [`Ui::hovering`], so a game reading the mouse can tell a
    /// click on the panel from one on its scene, and widgets behind the
    /// panel are never hit through it.
    pub fn panel(
        &mut self,
        ctx: &mut Context,
        title: &str,
        at: [f32; 2],
        width: f32,
        content: impl FnOnce(&mut Ui, &mut Context),
    ) {
        let id = panel_id(title);
        let style = self.style;
        let title_h = style.row_h;
        let pad = style.pad;
        let state = self
            .panels
            .entry(id)
            .and_modify(|p| p.w = width)
            .or_insert(PanelState {
                pos: at,
                w: width,
                h: title_h,
            });
        let w = state.w;
        let h = state.h;
        let body = Rect::from_center(state.pos[0], state.pos[1], w, h);
        let title_bar = Rect {
            left: body.left,
            top: body.top,
            w,
            h: title_h,
        };
        // The body claims the panel's whole area first, so the title bar —
        // registered after it — wins the overlap, and neither a widget nor
        // the game behind this panel can be hit through it.
        self.interact(panel_body_id(title), body);
        // The title bar is a widget too: it claims the press for the drag.
        let it = self.interact(id, title_bar);
        self.fill(ctx, body, style.panel_bg);
        let bar = if it.held {
            style.widget_press
        } else if it.hot {
            style.widget_hover
        } else {
            style.title_bg
        };
        self.fill(ctx, title_bar, bar);
        let size = style.font_size;
        let title_color = style.title_text;
        let [tcx, tcy] = title_bar.center();
        let title_string = title.to_owned();
        self.text(ctx, [tcx, tcy], size, title_color, &title_string);
        // The content layout, and the height it uses.
        let top0 = body.top - title_h - pad;
        self.layout.push(Layout {
            left: body.left + pad,
            width: (w - 2.0 * pad).max(0.0),
            cursor: top0,
            scope: id,
        });
        content(self, ctx);
        let used = top0 - self.layout.pop().expect("just pushed").cursor;
        let state = self.panels.get_mut(&id).expect("entry was made above");
        state.h = title_h + pad * 2.0 + used;
    }
}

/// The frame's input snapshot [`Ui::begin_input`] works from.
struct UiInput {
    pointer: Option<[f32; 2]>,
    down: bool,
    size: (f32, f32),
}

/// The id of an interactive widget: its label hashed into its panel's
/// scope (0 for the root column).
fn widget_id(scope: u64, label: &str) -> u64 {
    let mut h = FNV_OFFSET;
    for b in scope.to_le_bytes() {
        h = (h ^ b as u64).wrapping_mul(FNV_PRIME);
    }
    for &b in label.as_bytes() {
        h = (h ^ b as u64).wrapping_mul(FNV_PRIME);
    }
    h
}

/// The id of a panel and of its draggable title bar, hashed from the title.
fn panel_id(title: &str) -> u64 {
    let mut h = FNV_OFFSET;
    for b in b"panel".iter().chain(title.as_bytes()) {
        h = (h ^ *b as u64).wrapping_mul(FNV_PRIME);
    }
    h
}

/// The id of a panel's non-interactive body claim: its own domain, so a
/// press on the panel background captures nothing that the drag loop
/// mistakes for a panel move.
fn panel_body_id(title: &str) -> u64 {
    let mut h = FNV_OFFSET;
    for b in b"panel-body".iter().chain(title.as_bytes()) {
        h = (h ^ *b as u64).wrapping_mul(FNV_PRIME);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repo's UI font, read once; the test binary runs from the crate
    /// root, so the asset path is stable.
    fn font() -> Arc<[u8]> {
        static FONT: std::sync::OnceLock<Arc<[u8]>> = std::sync::OnceLock::new();
        FONT.get_or_init(|| {
            let path = format!(
                "{}/assets/fonts/JameGem08_2026-Regular.ttf",
                std::env!("CARGO_MANIFEST_DIR")
            );
            Arc::from(
                std::fs::read(&path)
                    .unwrap_or_else(|e| panic!("failed to read {path}: {e}"))
                    .as_slice(),
            )
        })
        .clone()
    }

    fn ui() -> Ui {
        Ui::from_bytes(&font()).expect("test font")
    }

    /// Feeds one frame of input through `begin_input`.
    fn frame(ui: &mut Ui, pointer: Option<[f32; 2]>, down: bool) {
        ui.begin_input(UiInput {
            pointer,
            down,
            size: (800.0, 600.0),
        });
    }

    #[test]
    fn press_arms_next_frame_and_click_needs_release_over() {
        let mut ui = ui();
        // Frame 1: the widget registers its rect; a press lands on it.
        // Hover resolves from last frame's rects, so this frame it is not
        // hot yet — one frame of latency is the documented model.
        frame(&mut ui, Some([0.0, 0.0]), true);
        let id = widget_id(0, "go");
        let rect = Rect::from_center(0.0, 0.0, 100.0, 30.0);
        let it = ui.interact(id, rect);
        assert!(!it.hot, "the rect is only known next frame");
        assert!(!it.held, "a press arms with a frame of latency");
        // Frame 2: still held, no release; now hover sees the rect.
        frame(&mut ui, Some([0.0, 0.0]), true);
        let it = ui.interact(id, rect);
        assert!(it.hot);
        assert!(ui.hovering(), "the same resolution backs `hovering`");
        assert!(it.held);
        assert!(it.pressed);
        assert!(!it.clicked);
        // Frame 3: released over the widget — a click.
        frame(&mut ui, Some([0.0, 0.0]), false);
        let it = ui.interact(id, rect);
        assert!(it.clicked);
    }

    #[test]
    fn release_outside_the_widget_is_not_a_click() {
        let mut ui = ui();
        let id = widget_id(0, "go");
        let rect = Rect::from_center(0.0, 0.0, 100.0, 30.0);
        frame(&mut ui, Some([0.0, 0.0]), true);
        ui.interact(id, rect);
        frame(&mut ui, Some([900.0, 0.0]), true);
        let it = ui.interact(id, rect);
        assert!(it.held, "the drag keeps holding off-widget");
        frame(&mut ui, Some([900.0, 0.0]), false);
        let it = ui.interact(id, rect);
        assert!(!it.clicked, "release happened outside");
    }

    #[test]
    fn press_on_nothing_holds_nothing() {
        let mut ui = ui();
        frame(&mut ui, Some([500.0, 250.0]), true);
        ui.interact(widget_id(0, "go"), Rect::from_center(0.0, 0.0, 100.0, 30.0));
        frame(&mut ui, Some([500.0, 250.0]), true);
        assert_eq!(ui.active, None);
    }

    #[test]
    fn topmost_widget_wins_an_overlap_press() {
        let mut ui = ui();
        let below = widget_id(0, "below");
        let above = widget_id(0, "above");
        let r_below = Rect::from_center(0.0, 0.0, 100.0, 30.0);
        let r_above = Rect::from_center(0.0, 0.0, 40.0, 20.0);
        frame(&mut ui, Some([0.0, 0.0]), true);
        ui.interact(below, r_below);
        ui.interact(above, r_above);
        frame(&mut ui, Some([0.0, 0.0]), true);
        assert_eq!(ui.active, Some(above), "last declared (on top) wins");
    }

    #[test]
    fn a_press_of_one_frame_still_clicks() {
        let mut ui = ui();
        let id = widget_id(0, "go");
        let rect = Rect::from_center(0.0, 0.0, 100.0, 30.0);
        frame(&mut ui, Some([0.0, 0.0]), true);
        ui.interact(id, rect);
        frame(&mut ui, Some([0.0, 0.0]), false);
        let it = ui.interact(id, rect);
        assert!(it.clicked);
        assert_eq!(ui.active, None, "a never-held capture does not linger");
    }

    #[test]
    fn panel_drags_with_the_pointer_once_held() {
        let mut ui = ui();
        let id = panel_id("Settings");
        ui.panels.insert(
            id,
            PanelState {
                pos: [0.0, 0.0],
                w: 200.0,
                h: 60.0,
            },
        );
        let title = Rect::from_center(0.0, 30.0 - 14.0, 200.0, 28.0);
        frame(&mut ui, Some([0.0, 16.0]), true);
        ui.interact(id, title);
        // The panel moves by the pointer's delta, not to the pointer.
        frame(&mut ui, Some([10.0, 20.0]), true);
        assert_eq!(ui.panel_position("Settings"), Some([10.0, 4.0]));
        frame(&mut ui, Some([17.0, 26.0]), true);
        assert_eq!(ui.panel_position("Settings"), Some([17.0, 10.0]));
        frame(&mut ui, Some([17.0, 26.0]), false);
        frame(&mut ui, Some([90.0, 90.0]), false);
        assert_eq!(
            ui.panel_position("Settings"),
            Some([17.0, 10.0]),
            "the position is retained after the release"
        );
    }

    #[test]
    fn ids_are_stable_and_scoped() {
        assert_eq!(widget_id(0, "go"), widget_id(0, "go"));
        assert_ne!(widget_id(0, "go"), widget_id(1, "go"), "panels scope ids");
        assert_ne!(widget_id(0, "go"), widget_id(0, "stop"));
        assert_ne!(panel_id("go"), widget_id(0, "go"), "domains do not mix");
    }

    #[test]
    fn rect_contains_edges() {
        let r = Rect::from_center(0.0, 0.0, 100.0, 20.0);
        assert!(r.contains([50.0, 10.0]));
        assert!(r.contains([-50.0, -10.0]));
        assert!(!r.contains([50.01, 0.0]));
        assert!(!r.contains([0.0, 10.01]));
    }

    #[test]
    fn hovering_follows_the_previous_frames_rects() {
        let mut ui = ui();
        frame(&mut ui, Some([0.0, 0.0]), false);
        assert!(!ui.hovering());
        let rect = Rect::from_center(0.0, 0.0, 100.0, 30.0);
        ui.interact(widget_id(0, "go"), rect);
        // Next frame, same spot: the rect registered last frame is hit.
        frame(&mut ui, Some([0.0, 0.0]), false);
        assert!(ui.hovering());
        // A pointer outside every rect — and no pointer at all — hover
        // nothing.
        frame(&mut ui, Some([300.0, 100.0]), false);
        assert!(!ui.hovering());
        frame(&mut ui, None, false);
        assert!(!ui.hovering());
    }

    #[test]
    fn a_later_panel_blocks_the_earlier_one() {
        let mut ui = ui();
        let under = widget_id(0, "go");
        let over = panel_body_id("Cover");
        let r_under = Rect::from_center(0.0, 0.0, 100.0, 30.0);
        let r_over = Rect::from_center(0.0, 0.0, 200.0, 60.0);
        frame(&mut ui, Some([0.0, 0.0]), true);
        // Declared order: the covered widget first, the covering panel body
        // after it (panels draw over what came before).
        ui.interact(under, r_under);
        ui.interact(over, r_over);
        // The press resolves to the topmost claim — the panel body, whose
        // id is no panel's, so it captures the press and moves nothing.
        frame(&mut ui, Some([0.0, 0.0]), true);
        assert_eq!(ui.active, Some(over));
        let it = ui.interact(under, r_under);
        assert!(
            !it.hot,
            "the covered widget shows no hover through the panel"
        );
        assert!(!it.clicked);
    }
}
