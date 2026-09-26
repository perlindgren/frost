//! Diagnostics overlay: a small HUD pinned to the window's top-left corner,
//! reporting the frame state as four left-aligned lines and three scrolling
//! graphs — the window size on top (`800x600`), the smoothed frame rate
//! below it (`FPS 58`), the current frame time below that (`FT 16.7ms`),
//! the last frame's processing time below that (`PROC 0.52ms`), and under
//! the lines three strip charts covering the last ten seconds: frame rate
//! (orange) on top, frame time (green) in the middle, processing time
//! (blue) below — one polyline through the max of each 0.1 s column, drawn
//! in a single draw call per chart, with a reference line at 60 fps /
//! 16.7 ms on each.
//!
//! The processing time is not a measurement this overlay makes: it is the
//! engine's own probe of its per-frame CPU work, reported as
//! [`Context::frame_processing_ms`] — the time the backend spent producing
//! the last completed frame, the wait for the next frame excluded.
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
//! The first call appends the overlay's twelve nodes — four text lines,
//! three graph panels, three reference lines, and three chart polylines —
//! to the scene's root, and every later call updates those same nodes in
//! place, so
//! the demo's scene needs no other change: just leave the root's children
//! un-reordered, the overlay remembers where it put its nodes.
//!
//! The font is read once, at construction, and shared behind an `Arc`. Each
//! readout line is re-laid out only when the digits it shows change (the
//! frame-time line shows the raw last frame's gap, so it usually changes
//! every frame); each chart's polyline is re-placed every frame from a
//! history of (time, frame time, processing time) samples trimmed to the
//! last ten seconds,
//! binned into 100 slices of 0.1 s — the chart shows the max of each slice,
//! a worst-case 0.1 s envelope, so a stall is never hidden behind the
//! frames around it.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::objects::TextError;
use crate::{Color, Context, Process, Scene, SceneNode, Shape, Transform};

/// The readout's font size in pixels per em.
const SIZE: f32 = 32.0;

/// The inset of the readout's top-left corner from the window's, in pixels.
const MARGIN: f32 = 20.0;

/// The gap between the readout lines' inks, in pixels.
const LINE_GAP: f32 = 6.0;

/// The smoothing time constant, in seconds: how quickly the displayed frame
/// rate follows the real one.
const SMOOTH: f32 = 0.25;

/// The longest frame gap, in seconds, still counted as a measurement of
/// rendering speed; a longer gap is a stall (a focus loss, a debugger
/// pause, a window drag) and is skipped by the frame-rate smoothing.
const STALL: f32 = 0.25;

/// The width of each strip chart, in pixels.
const GRAPH_W: f32 = 200.0;

/// The height of each strip chart, in pixels.
const GRAPH_H: f32 = 40.0;

/// The gap between the last readout line and the first graph, and between
/// the two graphs, in pixels.
const GRAPH_GAP: f32 = 8.0;

/// The time columns per graph: one polyline point per column, 0.1 s each.
const COLUMNS: usize = 100;

/// The time span each graph covers, in seconds.
const SPAN: f32 = 10.0;

/// The frame time, in ms, at the top of the frame-time graph; longer frames
/// clip at the top.
const FT_TOP: f32 = 50.0;

/// The frame rate, in frames per second, at the top of the fps graph; faster
/// rates clip at the top.
const FPS_TOP: f32 = 120.0;

/// The processing time, in ms, at the top of the processing-time graph —
/// just past the 16.7 ms 60 fps frame budget, so the reference line stays
/// inside the panel; longer processing clips at the top.
const PROC_TOP: f32 = 20.0;

/// The graph panels' fill.
const PANEL: Color = Color { r: 0.03, g: 0.04, b: 0.07, a: 1.0 };

/// The fps chart's polyline.
const FPS_COLOR: Color = Color { r: 0.95, g: 0.62, b: 0.25, a: 1.0 };

/// The frame-time chart's polyline.
const FT_COLOR: Color = Color { r: 0.35, g: 0.78, b: 0.55, a: 1.0 };

/// The processing-time chart's polyline.
const PROC_COLOR: Color = Color { r: 0.45, g: 0.7, b: 0.95, a: 1.0 };

/// The 60 fps / 16.7 ms reference lines.
const REF_COLOR: Color = Color { r: 0.4, g: 0.4, b: 0.45, a: 1.0 };

/// The overlay's node slots, in append order: the four text lines, the
/// three graph panels, the three reference lines, and the three chart
/// polylines.
const N_TEXT: usize = 4;
const N_PANELS: usize = 3;
const N_REFS: usize = 3;
const N_LINES: usize = 3;
const N_NODES: usize = N_TEXT + N_PANELS + N_REFS + N_LINES;

/// The chart polylines' stroke width, in pixels.
const LINE_WIDTH: f32 = 1.5;

/// A diagnostics overlay: four left-aligned lines in the window's top-left
/// corner — the window size on top (`800x600`), the smoothed frame rate
/// (`FPS 58`), the current frame time (`FT 16.7ms`), and the last frame's
/// processing time (`PROC 0.52ms`) — with three scrolling ten-second strip
/// charts beneath: frame rate (orange) on top, frame time (green) in the
/// middle, processing time (blue) below.
///
/// Create one with [`Diagnostics::new`] (from a font file on disk) or
/// [`Diagnostics::from_bytes`] (from font data already in memory — the path
/// for environments without a file system), hold it in the demo state, and
/// call [`Process::process`] on it every frame. The first call appends its
/// nodes to the scene's root; each later call updates them in place — the
/// font is read once, and each line is re-laid out only when the digits it
/// shows change.
#[derive(Debug)]
pub struct Diagnostics {
    /// The font's bytes, shared with the readout's text shapes.
    font: Arc<[u8]>,
    /// The window-size line, drawn above the frame-rate line.
    size_line: Line,
    /// The frame-rate line, drawn below the window-size line.
    fps_line: Line,
    /// The frame-time line, drawn below the frame-rate line.
    ft_line: Line,
    /// The processing-time line, drawn below the frame-time line.
    proc_line: Line,
    /// The smoothed frame rate in frames per second.
    fps: f32,
    /// The current frame time in ms — the last real `dt`, unsmoothed, so a
    /// spike is visible in the readout as well as in the graphs.
    frame_ms: f32,
    /// The last completed frame's processing time in ms — the engine's own
    /// probe (`Context::frame_processing_ms`), unsmoothed, so a spike is
    /// visible in the readout as well as in the graph.
    proc_ms: f32,
    /// Elapsed time in seconds: the graphs' time axis, advanced by every
    /// real `dt` (stalls included — the graphs show real time).
    t: f32,
    /// The graphs' history: (elapsed time in seconds, frame time in ms,
    /// processing time in ms) samples, oldest first, trimmed to the last
    /// SPAN seconds.
    samples: VecDeque<(f32, f32, f32)>,
    /// The indices of the overlay's N_NODES nodes among the scene root's
    /// children, once appended (in slot order).
    nodes: Vec<Option<usize>>,
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

/// A text line's drawn state: its shape and the node origin that puts its
/// ink top at `ink_top` in user space.
struct LineDraw {
    /// The line's shape.
    shape: Shape,
    /// The node origin in user space (window centered on the origin, y up).
    pos: [f32; 2],
}

/// Copies `line`'s shape and computes its node origin for an ink top of
/// `ink_top`.
fn line_draw(line: &Line, ink_top: f32, w: f32) -> Option<LineDraw> {
    let shape = line.shape.clone()?;
    // The renderer centers a text block on the node's origin with the
    // baseline `baseline_offset` above it, so a line's node origin sits its
    // ink's top (plus the baseline offset) below the line's ink-top y, and
    // its ink's width (plus the margin) in from the left edge.
    Some(LineDraw {
        shape,
        pos: [
            -w / 2.0 + MARGIN + line.width / 2.0,
            ink_top - line.ink_top - line.baseline_offset,
        ],
    })
}

/// A chart's polyline points, in window user space: one per 0.1 s column,
/// at the column's center, rising from one pixel above the panel's bottom
/// edge, the value scaled to the panel height minus the two 1 px insets —
/// the stroke the chart's [`Shape::Polyline`] draws in one draw call.
fn chart_points(
    cols: &[f32; COLUMNS],
    x0: f32,
    panel_top: f32,
    top_value: f32,
) -> Vec<[f32; 2]> {
    let pitch = GRAPH_W / COLUMNS as f32;
    cols.iter()
        .enumerate()
        .map(|(c, &v)| {
            [
                x0 + (c as f32 + 0.5) * pitch,
                panel_top - GRAPH_H + 1.0 + (v / top_value).min(1.0) * (GRAPH_H - 2.0),
            ]
        })
        .collect()
}

/// Trims the history to the last SPAN seconds of real time ending at `now`.
fn trim_samples(samples: &mut VecDeque<(f32, f32, f32)>, now: f32) {
    while let Some(&(t0, _, _)) = samples.front() {
        if t0 < now - SPAN {
            samples.pop_front();
        } else {
            break;
        }
    }
}

/// The maximum of `value(frame_ms, proc_ms)` over the samples falling in
/// each of the COLUMNS time slices of the last SPAN seconds ending at
/// `now`, index 0 = the oldest slice. `samples` are (time in seconds, frame
/// time in ms, processing time in ms) triples, and a sample at exactly
/// `now` lands in the newest slice.
fn slice_max(
    samples: impl IntoIterator<Item = (f32, f32, f32)>,
    now: f32,
    mut value: impl FnMut(f32, f32) -> f32,
) -> [f32; COLUMNS] {
    let mut cols = [0.0f32; COLUMNS];
    let start = now - SPAN;
    for (t, frame_ms, proc_ms) in samples {
        let rel = (t - start) / SPAN;
        // rel > 1.0 is beyond the window; the epsilon absorbs the f32
        // rounding of `now - (now - SPAN)`, which can land just past SPAN.
        if rel <= 0.0 || rel > 1.0 + 1e-6 {
            continue;
        }
        let rel = rel.min(1.0);
        let c = (rel * COLUMNS as f32) as usize;
        let c = c.min(COLUMNS - 1);
        let v = value(frame_ms, proc_ms);
        if v > cols[c] {
            cols[c] = v;
        }
    }
    cols
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
            ft_line: Line::new(),
            proc_line: Line::new(),
            fps: 0.0,
            frame_ms: 0.0,
            proc_ms: 0.0,
            t: 0.0,
            samples: VecDeque::new(),
            nodes: vec![None; N_NODES],
        })
    }

    /// Finds the node for overlay slot `slot` (re-appending it if the demo
    /// removed it) and updates its shape and transform in place.
    fn place(&mut self, slot: usize, pos: [f32; 2], shape: Shape, scene: &mut Scene) {
        let index = match self.nodes[slot] {
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
        self.nodes[slot] = Some(index);
        let node = &mut scene.root.children[index];
        node.shape = Some(shape);
        node.transform = Transform::translate(pos);
    }
}

impl Process for Diagnostics {
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        // The engine's own probe of the last completed frame's CPU work.
        // This frame's own probe is still running, so the value is the
        // previous frame's by construction — a one-frame lag, one 0.1 s
        // column at 60 fps, invisible in the graphs.
        self.proc_ms = ctx.frame_processing_ms() as f32;
        // Record the frame in the graphs' history: the frame time is the
        // raw gap (a spike must be visible in the readout and the graphs),
        // and the time axis is real time — stalls included. The first
        // frame's dt is 0.0; nothing to record.
        if dt > 0.0 {
            self.frame_ms = dt * 1000.0;
            self.t += dt;
            self.samples.push_back((self.t, self.frame_ms, self.proc_ms));
            trim_samples(&mut self.samples, self.t);
        }
        // Smooth the frame rate: an exponential moving average of the
        // instantaneous rate, with a time constant of SMOOTH seconds — and
        // a snap to the first real measurement, so the first frame does not
        // flash a zero. A dt past STALL is a gap — a focus loss, a debugger
        // pause — not a measurement of rendering speed; letting it through
        // would drag the readout down to a few frames per second for a
        // frame or two.
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
        let ft_text = format!("FT {:.1}ms", self.frame_ms);
        if ft_text != self.ft_line.text {
            self.ft_line.text = ft_text;
            self.ft_line.refresh(&self.font);
        }
        let proc_text = format!("PROC {:.2}ms", self.proc_ms);
        if proc_text != self.proc_line.text {
            self.proc_line.text = proc_text;
            self.proc_line.refresh(&self.font);
        }
        // The size line's ink top sits MARGIN in from the window's top
        // edge; each line hangs below the previous, LINE_GAP under its ink
        // bottom. All lines' ink left edges sit MARGIN in from the window's
        // left edge.
        let size_ink_top = h / 2.0 - MARGIN;
        let fps_ink_top =
            size_ink_top - (self.size_line.ink_top - self.size_line.ink_bottom) - LINE_GAP;
        let ft_ink_top = fps_ink_top - (self.fps_line.ink_top - self.fps_line.ink_bottom) - LINE_GAP;
        let proc_ink_top =
            ft_ink_top - (self.ft_line.ink_top - self.ft_line.ink_bottom) - LINE_GAP;
        // The fps panel hangs LINE_GAP + GRAPH_GAP under the proc line's
        // ink bottom; the ft and proc panels one panel height plus
        // GRAPH_GAP under it.
        let proc_ink_bottom =
            proc_ink_top - (self.proc_line.ink_top - self.proc_line.ink_bottom);
        let fps_panel_top = proc_ink_bottom - LINE_GAP - GRAPH_GAP;
        let ft_panel_top = fps_panel_top - GRAPH_H - GRAPH_GAP;
        let proc_panel_top = ft_panel_top - GRAPH_H - GRAPH_GAP;
        let x0 = -w / 2.0 + MARGIN;
        // The newest sample (t == now) must land in the newest slice, so the
        // graphs show the frame just finished, not the one before.
        let fps_cols = slice_max(self.samples.iter().copied(), self.t, |ft, _| 1000.0 / ft);
        let ft_cols = slice_max(self.samples.iter().copied(), self.t, |ft, _| ft);
        let proc_cols = slice_max(self.samples.iter().copied(), self.t, |_, proc| proc);

        let scene = ctx.scene();
        // The four readout lines.
        let line_draws = [
            line_draw(&self.size_line, size_ink_top, w),
            line_draw(&self.fps_line, fps_ink_top, w),
            line_draw(&self.ft_line, ft_ink_top, w),
            line_draw(&self.proc_line, proc_ink_top, w),
        ];
        for (i, draw) in line_draws.into_iter().enumerate() {
            let Some(draw) = draw else {
                continue;
            };
            self.place(i, draw.pos, draw.shape, scene);
        }
        // The panels.
        self.place(
            N_TEXT,
            [x0 + GRAPH_W / 2.0, fps_panel_top - GRAPH_H / 2.0],
            Shape::Rectangle {
                center: [0.0, 0.0],
                extent: [GRAPH_W / 2.0, GRAPH_H / 2.0],
                color: PANEL,
            },
            scene,
        );
        self.place(
            N_TEXT + 1,
            [x0 + GRAPH_W / 2.0, ft_panel_top - GRAPH_H / 2.0],
            Shape::Rectangle {
                center: [0.0, 0.0],
                extent: [GRAPH_W / 2.0, GRAPH_H / 2.0],
                color: PANEL,
            },
            scene,
        );
        self.place(
            N_TEXT + 2,
            [x0 + GRAPH_W / 2.0, proc_panel_top - GRAPH_H / 2.0],
            Shape::Rectangle {
                center: [0.0, 0.0],
                extent: [GRAPH_W / 2.0, GRAPH_H / 2.0],
                color: PANEL,
            },
            scene,
        );
        // The charts: one polyline each, through the max of each 0.1 s
        // column — one draw call per chart instead of one per column.
        let chart_line = |points: Vec<[f32; 2]>, color: Color| {
            Shape::Polyline {
                points,
                width: LINE_WIDTH,
                color,
            }
        };
        self.place(
            N_TEXT + N_PANELS + N_REFS,
            [0.0, 0.0],
            chart_line(
                chart_points(&fps_cols, x0, fps_panel_top, FPS_TOP),
                FPS_COLOR,
            ),
            scene,
        );
        self.place(
            N_TEXT + N_PANELS + N_REFS + 1,
            [0.0, 0.0],
            chart_line(chart_points(&ft_cols, x0, ft_panel_top, FT_TOP), FT_COLOR),
            scene,
        );
        self.place(
            N_TEXT + N_PANELS + N_REFS + 2,
            [0.0, 0.0],
            chart_line(chart_points(&proc_cols, x0, proc_panel_top, PROC_TOP), PROC_COLOR),
            scene,
        );
        // The 60 fps / 16.7 ms reference lines, 1 px tall across the panel.
        let ref_line = |panel_top: f32, level: f32| {
            (
                panel_top - GRAPH_H + 1.0 + level * (GRAPH_H - 2.0),
                Shape::Rectangle {
                    center: [GRAPH_W / 2.0, 0.0],
                    extent: [GRAPH_W / 2.0, 0.5],
                    color: REF_COLOR,
                },
            )
        };
        let (y, shape) = ref_line(fps_panel_top, 60.0 / FPS_TOP);
        self.place(N_TEXT + N_PANELS, [x0, y], shape, scene);
        let (y, shape) = ref_line(ft_panel_top, (1000.0 / 60.0) / FT_TOP);
        self.place(N_TEXT + N_PANELS + 1, [x0, y], shape, scene);
        let (y, shape) = ref_line(proc_panel_top, (1000.0 / 60.0) / PROC_TOP);
        self.place(N_TEXT + N_PANELS + 2, [x0, y], shape, scene);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_keeps_only_the_last_span_seconds() {
        let mut samples = VecDeque::new();
        for t in 0..11 {
            samples.push_back((t as f32, 16.0, 1.0));
        }
        trim_samples(&mut samples, 11.0);
        // t < 11 - 10 = 1 is gone; t = 1..=11 remain.
        assert_eq!(samples.front().unwrap().0, 1.0);
        assert_eq!(samples.len(), 10);
    }

    #[test]
    fn slice_max_buckets_by_time_not_by_count() {
        // now = 10: the window is t = 0..10, 100 slices of 0.1 s.
        let samples = [
            (0.05, 100.0, 0.0), // oldest slice (c = 0)
            (5.00, 200.0, 0.0), // middle of the window (c = 50)
            (9.95, 100.0, 0.0), // newest slice (c = 99)
        ];
        let cols = slice_max(samples, 10.0, |ft, _| ft);
        assert_eq!(cols[0], 100.0);
        assert_eq!(cols[50], 200.0);
        assert_eq!(cols[99], 100.0);
        assert_eq!(cols[49], 0.0);
        assert_eq!(cols[98], 0.0);
    }

    #[test]
    fn slice_max_counts_the_newest_sample_at_now() {
        // t == now: the sample belongs to the newest slice, not beyond it.
        let samples = [(10.0, 300.0, 0.0)];
        let cols = slice_max(samples, 10.0, |ft, _| ft);
        assert_eq!(cols[99], 300.0);
    }

    #[test]
    fn slice_max_ignores_samples_outside_the_window() {
        let samples = [(15.0, 100.0, 0.0), (20.0, 100.0, 0.0)];
        let cols = slice_max(samples, 10.0, |ft, _| ft);
        assert!(cols.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn slice_max_fps_is_the_inverse_of_the_frame_time() {
        // Two frames in one slice: 10 ms and 40 ms → the slice's best rate
        // is 100 fps.
        let samples = [(9.95, 10.0, 0.0), (9.98, 40.0, 0.0)];
        let cols = slice_max(samples, 10.0, |ft, _| 1000.0 / ft);
        assert_eq!(cols[99], 100.0);
    }

    #[test]
    fn slice_max_proc_ignores_the_frame_time() {
        // Two frames in one slice: 10 ms and 40 ms of frame time, 2.5 ms
        // and 1.0 ms of processing — the proc column is the max of the
        // processing times, not the frame times.
        let samples = [(9.95, 10.0, 2.5), (9.98, 40.0, 1.0)];
        let cols = slice_max(samples, 10.0, |_, proc| proc);
        assert_eq!(cols[99], 2.5);
    }
}
