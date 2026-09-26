//! Diagnostics overlay: a small HUD that reports the window size and the
//! frame rate as two left-aligned text lines pinned to the window's
//! top-left corner — the size on top (`800x600`), the frame rate below
//! (`FPS 58`).
//!
//! The [`Diagnostics`] struct is the whole integration: create one before
//! [`crate::run`], hold it in the demo state, and call its
//! [`Process::process`] every frame alongside the demo's own update —
//!
//! ```no_run
//! struct Demo {
//!     diag: frost::Diagnostics,
//!     time: f32,
//! }
//!
//! impl frost::Process for Demo {
//!     fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
//!         self.diag.process(ctx, dt);
//!         self.time += dt;
//!         // ...the rest of the demo...
//!     }
//! }
//! ```
//!
//! The first call appends the overlay's two text nodes to the scene's root,
//! and every later call updates those same nodes in place, so the demo's
//! scene needs no other change — just leave the root's children un-reordered:
//! the overlay remembers where it put its nodes.
//!
//! The font is read once, at construction, and shared behind an `Arc`; each
//! readout line is re-laid out only when the digits it shows change, so the
//! overlay's steady-state cost per frame is two string comparisons.

use std::sync::Arc;

use crate::objects::TextError;
use crate::{Context, Process, SceneNode, Shape, Transform};

/// The readout's font size in pixels per em.
const SIZE: f32 = 32.0;

/// The inset of the readout's top-left corner from the window's, in pixels.
const MARGIN: f32 = 20.0;

/// The gap between the two readout lines' inks, in pixels.
const LINE_GAP: f32 = 6.0;

/// The smoothing time constant, in seconds: how quickly the displayed frame
/// rate follows the real one.
const SMOOTH: f32 = 0.25;

/// The longest frame gap, in seconds, still counted as a measurement of
/// rendering speed; a longer gap is a stall (a focus loss, a debugger
/// pause, a window drag) and is skipped by the frame-rate smoothing.
const STALL: f32 = 0.25;

/// A diagnostics overlay: two left-aligned lines in the window's top-left
/// corner — the window size on top (`800x600`) and the smoothed frame rate
/// below (`FPS 58`).
///
/// Create one with [`Diagnostics::new`] (from a font file on disk) or
/// [`Diagnostics::from_bytes`] (from font data already in memory — the path
/// for environments without a file system), hold it in the demo state, and
/// call [`Process::process`] on it every frame. The first call appends its
/// two text nodes to the scene's root; each later call updates those nodes —
/// the font is read once, and each line is re-laid out only when the digits
/// it shows change.
#[derive(Debug)]
pub struct Diagnostics {
    /// The font's bytes, shared with the readout's text shapes.
    font: Arc<[u8]>,
    /// The window-size line, drawn above the frame-rate line.
    size_line: Line,
    /// The frame-rate line, drawn below the window-size line.
    fps_line: Line,
    /// The smoothed frame rate in frames per second.
    fps: f32,
    /// The indices of the overlay's two nodes among the scene root's
    /// children, once appended (size line first).
    nodes: [Option<usize>; 2],
}

/// One line of the readout: its text and the laid-out measurements the
/// corner placement needs.
#[derive(Debug)]
struct Line {
    /// The line's last text, guarding the per-frame rebuild.
    text: String,
    /// The laid-out shape of `text`, kept until `text` changes.
    shape: Option<Shape>,
    /// The laid-out width of `text`, in pixels.
    width: f32,
    /// The y of the highest ink pixel of `text` above its baseline, in
    /// pixels (measured from the glyphs' ink, because a font's vertical
    /// metrics can be degenerate).
    ink_top: f32,
    /// The y of the lowest ink pixel of `text` above its baseline, in
    /// pixels (negative when the text has descenders).
    ink_bottom: f32,
    /// The renderer's baseline offset from the node's origin — half the
    /// font's ascent minus descent — in pixels.
    baseline_offset: f32,
}

impl Line {
    /// An empty, not-yet-laid-out line.
    fn new() -> Self {
        Self {
            text: String::new(),
            shape: None,
            width: 0.0,
            ink_top: 0.0,
            ink_bottom: 0.0,
            baseline_offset: 0.0,
        }
    }

    /// Re-lays out `text`, which must have just changed.
    fn refresh(&mut self, font: &Arc<[u8]>) {
        let Some(layout) = crate::text::layout(font, &self.text, SIZE) else {
            return;
        };
        self.shape = Shape::text_bytes(font, &self.text, SIZE).ok();
        self.width = layout.width;
        self.baseline_offset = (layout.ascent - layout.descent) / 2.0;
        // A font's vertical metrics can be degenerate (the bundled Leofont's
        // ascent plus descent is about a pixel at 32 px), so place the line
        // by the glyphs' real ink: the highest and lowest ink pixels, over
        // every glyph that has one.
        let (mut ink_top, mut ink_bottom) = (layout.ascent, -layout.descent);
        for glyph in &layout.glyphs {
            if let Some(raster) = crate::text::rasterize(font, glyph.id, SIZE) {
                let top = glyph.y + raster.top as f32;
                ink_top = ink_top.max(top);
                ink_bottom = ink_bottom.min(top - raster.height as f32);
            }
        }
        self.ink_top = ink_top;
        self.ink_bottom = ink_bottom;
    }
}

impl Diagnostics {
    /// Creates an overlay reading its font from the file at `path`.
    ///
    /// Like [`Shape::text`], the file is read and checked to be a
    /// TrueType/OpenType font immediately, so a missing file or a non-font
    /// fails here, not at the first frame.
    pub fn new(path: impl AsRef<std::path::Path>) -> Result<Self, TextError> {
        let bytes = std::fs::read(path.as_ref()).map_err(TextError::Io)?;
        Self::from_bytes(&bytes)
    }

    /// Creates an overlay from font data already in memory, for example
    /// bytes embedded into the binary with `include_bytes!`.
    ///
    /// Like [`Shape::text_bytes`], the bytes are checked to be a
    /// TrueType/OpenType font up front and shared behind an `Arc`, but no
    /// file is read — this is how an overlay is created in environments
    /// without a file system, such as a web browser.
    pub fn from_bytes(font: &[u8]) -> Result<Self, TextError> {
        swash::FontRef::from_index(font, 0).ok_or(TextError::InvalidFont)?;
        Ok(Self {
            font: Arc::from(font),
            size_line: Line::new(),
            fps_line: Line::new(),
            fps: 0.0,
            nodes: [None, None],
        })
    }
}

impl Process for Diagnostics {
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        // Smooth the frame rate: an exponential moving average of the
        // instantaneous rate, with a time constant of SMOOTH seconds — and
        // a snap to the first real measurement, so the first frame does not
        // flash a zero. The first frame's dt is 0.0; skip it. A dt past
        // STALL is a gap — a focus loss, a debugger pause — not a
        // measurement of rendering speed; letting it through would drag the
        // readout down to a few frames per second for a frame or two.
        if dt > 0.0 && dt <= STALL {
            let inst = 1.0 / dt;
            if self.fps == 0.0 {
                self.fps = inst;
            } else {
                let w = (1.0 - (-dt / SMOOTH).exp()).min(1.0);
                self.fps += (inst - self.fps) * w;
            }
        }
        let (w, h) = ctx.size();
        // Rebuild each line's shape only when its text actually changes.
        let size_text = format!("{}x{}", w as u32, h as u32);
        if size_text != self.size_line.text {
            self.size_line.text = size_text;
            self.size_line.refresh(&self.font);
        }
        let fps_text = format!("FPS {:.0}", self.fps.round());
        if fps_text != self.fps_line.text {
            self.fps_line.text = fps_text;
            self.fps_line.refresh(&self.font);
        }
        // The size line's ink top sits MARGIN in from the window's top
        // edge; the fps line hangs below it, LINE_GAP under the size line's
        // ink bottom. Both lines' ink left edges sit MARGIN in from the
        // window's left edge.
        let size_ink_top = h / 2.0 - MARGIN;
        let fps_ink_top =
            size_ink_top - (self.size_line.ink_top - self.size_line.ink_bottom) - LINE_GAP;
        // The renderer centers a text block on the node's origin with the
        // baseline `baseline_offset` above it, so a line's node origin sits
        // its ink's top (plus the baseline offset) below the line's ink-top
        // y, and its ink's width (plus the margin) in from the left edge.
        let origin = |line: &Line, ink_top: f32| [
            -w / 2.0 + MARGIN + line.width / 2.0,
            ink_top - line.ink_top - line.baseline_offset,
        ];
        let lines = [
            (
                &self.size_line,
                origin(&self.size_line, size_ink_top),
            ),
            (
                &self.fps_line,
                origin(&self.fps_line, fps_ink_top),
            ),
        ];
        // Find each line's node (re-appending it if the demo removed it),
        // and update it in place.
        let scene = ctx.scene();
        for (i, (line, pos)) in lines.into_iter().enumerate() {
            let Some(shape) = line.shape.clone() else {
                continue;
            };
            let index = match self.nodes[i] {
                Some(idx) if idx < scene.root.children.len() => idx,
                _ => {
                    scene.root.children.push(Box::new(SceneNode {
                        transform: Transform::translate(pos),
                        shape: Some(shape.clone()),
                        ..Default::default()
                    }));
                    scene.root.children.len() - 1
                }
            };
            self.nodes[i] = Some(index);
            let node = &mut scene.root.children[index];
            node.shape = Some(shape);
            node.transform = Transform::translate(pos);
        }
    }
}
