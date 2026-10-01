//! Diagnostics overlay: a small HUD pinned to the window's top-left corner,
//! reporting the frame state as left-aligned lines and scrolling graphs —
//! the window size on top (`800x600`, always shown), and, for each
//! statistic enabled in the overlay's [`DiagnosticsFlags`], a readout line
//! and a scrolling ten-second strip chart beneath the lines: the smoothed
//! frame rate (`FPS 58`, orange) and the current frame time
//! (`FT 16.7ms`, green), each with a reference line at 60 fps / 16.7 ms;
//! the last frame's total processing time (`PROC 0.52ms`, blue) with its
//! cost split folded into one chart — the total, the app's share (violet),
//! and this overlay's own share (cyan); and the last frame's draw-call
//! count (`DRAW 48/52`, magenta) — the app's share over the total — with
//! its count split folded the same way: the total, the app's share (pink),
//! and this overlay's own count (rose).
//!
//! The processing times and the draw-call counts are not measurements this
//! overlay makes: they are the engine's own probes of the last completed
//! frame, reported as [`Context::frame_processing_ms`] — the time the
//! backend spent producing the frame, the wait for the next frame excluded
//! — and [`Context::frame_draw_calls`] — the GPU draw calls it issued, the
//! draws it skipped (off-screen, background, light, or empty) not counted.
//! The engine also counts the overlay's own share of the draw calls: the
//! overlay flags the scene nodes it appends with a crate-internal marker,
//! the marker rides onto every draw the nodes produce (the text's glyph
//! quads included), and the backend reports the tagged subset as
//! [`Context::frame_diagnostic_draw_calls`]. From those probes each frame
//! derives the split the charts show: the app values are the totals with
//! the overlay's own update cost (the overlay times itself with the
//! engine's millisecond clock, one frame behind, so both numbers describe
//! the same completed frame) and its own draw calls removed, clamped at
//! zero against timer jitter.
//!
//! The [`Diagnostics`] struct is the whole integration: create one before
//! [`crate::run`], choosing the statistics to display with
//! [`DiagnosticsFlags`], hold it in the demo state, and call its
//! [`Process::process`] every frame alongside the demo's own update —
//!
//! ```no_run
//! let _diag = frost::Diagnostics::new("font.ttf", frost::DiagnosticsFlags::all()).unwrap();
//!
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
//! The flags gate only what the overlay draws: the engine's probes run
//! every frame either way. With every flag set the overlay owns up to
//! twenty-one nodes — six text lines, four graph panels, three reference
//! lines, and eight chart polylines; the first call creates a dedicated
//! topmost layer in the scene and appends them to that layer's root, and
//! every later call updates those same nodes in place, so the demo's scene
//! needs no other change: the overlay's own layer is a hard draw partition
//! above the root subtree, so the readout is always on top, and the overlay
//! remembers where it put its nodes.
//!
//! The overlay also answers to keyboard shortcuts, read from the engine's
//! key state every frame: Alt-0 toggles the overlay as a whole, Alt-1..
//! Alt-4 toggle the charts by position among the enabled ones (top chart
//! first), and Alt-T toggles the readout lines. A hidden part draws
//! nothing, and the layout reflows around it — the remaining lines and
//! charts pack back into the top-left corner. The shortcuts are read, not
//! consumed: a demo that wants the same combinations still sees them.
//!
//! The font is read once, at construction, and shared behind an `Arc`. Each
//! readout line is re-laid out only when the digits it shows change (the
//! frame-time line shows the raw last frame's gap, so it usually changes
//! every frame); each chart's polylines are re-placed every frame from a
//! history of [`Sample`]s — one per completed frame, holding its frame
//! time, its processing and draw-call totals with their app and diagnostic
//! splits — trimmed to the last ten seconds, binned into 100 slices of
//! 0.1 s — the charts show the max of each slice, a worst-case 0.1 s
//! envelope, so a stall is never hidden behind the frames around it.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::backend::now_millis;
use crate::objects::TextError;
use crate::{Color, Context, KeyCode, Layer, Process, Scene, SceneNode, Shape, Transform};

/// The readout's font size in pixels per em.
const SIZE: f32 = 32.0;

/// The readout's weight on the font's `wght` variation axis: the plain
/// Regular instance. The overlay's measurements below pair with
/// `Shape::text_bytes`, whose shapes carry this same default weight.
const WEIGHT: f32 = 400.0;

/// The inset of the readout's top-left corner from the window's, in pixels.
const MARGIN: f32 = 20.0;

/// The gap between consecutive readout lines, in pixels.
const LINE_GAP: f32 = 6.0;

/// The readout's probe text: every character a readout line can display —
/// the digits, the "x", the "/", the ".", the "ms", and the fixed labels.
/// The lines' vertical extent is measured over it once, at construction,
/// because neither the current text's ink (Fira Code's "8" and "9" carry a
/// pixel of antialiasing below the baseline where the "4" does not) nor
/// the font's metrics (both bundled fonts report degenerate ones —
/// Leofont's ascent plus descent is about a pixel at 32 px, Fira Code's
/// about two) is a stable extent for the lines.
const PROBE: &str = "0123456789.x/ FPS FT PROC APP DRAW ms";

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
/// the graphs, in pixels.
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
/// inside the panel; longer processing clips at the top. The app and
/// diagnostic series share the same panel, so the same scale.
const PROC_TOP: f32 = 20.0;

/// The GPU draw-call count at the top of the draw-call graph: draw calls
/// have no physical maximum, so this is a soft budget — a frame issuing
/// more clips at the top of the panel (the readout still shows the true
/// count). The app and diagnostic series share the same panel, so the same
/// scale.
const DRAW_TOP: f32 = 128.0;

/// The graph panels' fill.
const PANEL: Color = Color {
    r: 0.03,
    g: 0.04,
    b: 0.07,
    a: 1.0,
};

/// The fps chart's polyline.
const FPS_COLOR: Color = Color {
    r: 0.95,
    g: 0.62,
    b: 0.25,
    a: 1.0,
};

/// The frame-time chart's polyline.
const FT_COLOR: Color = Color {
    r: 0.35,
    g: 0.78,
    b: 0.55,
    a: 1.0,
};

/// The processing-time chart's total series.
const PROC_COLOR: Color = Color {
    r: 0.45,
    g: 0.7,
    b: 0.95,
    a: 1.0,
};

/// The processing-time chart's app series.
const APP_COLOR: Color = Color {
    r: 0.7,
    g: 0.55,
    b: 0.95,
    a: 1.0,
};

/// The processing-time chart's diagnostic series: this overlay's own
/// update cost, the gap between the total and the app series.
const DIAG_PROC_COLOR: Color = Color {
    r: 0.35,
    g: 0.85,
    b: 0.95,
    a: 1.0,
};

/// The draw-call chart's total series.
const DRAW_COLOR: Color = Color {
    r: 0.9,
    g: 0.4,
    b: 0.75,
    a: 1.0,
};

/// The draw-call chart's app series.
const APP_DRAW_COLOR: Color = Color {
    r: 1.0,
    g: 0.7,
    b: 0.9,
    a: 1.0,
};

/// The draw-call chart's diagnostic series: this overlay's own draw calls,
/// the gap between the total and the app series.
const DIAG_DRAW_COLOR: Color = Color {
    r: 0.55,
    g: 0.15,
    b: 0.4,
    a: 1.0,
};

/// The 60 fps / 16.7 ms reference lines.
const REF_COLOR: Color = Color {
    r: 0.4,
    g: 0.4,
    b: 0.45,
    a: 1.0,
};

/// The chart polylines' stroke width, in pixels.
const LINE_WIDTH: f32 = 1.5;

/// Which of the overlay's statistics to display: the frame rate, the frame
/// time, the processing time, and the GPU draw-call count.
///
/// A flag enables the statistic's readout line and its chart together; the
/// processing-time flag also enables the app-time line, since the app's
/// share is the total minus the overlay's own cost. The flags gate only
/// what the overlay draws — the engine's probes run every frame either
/// way, and the window-size line is always shown.
///
/// Combine the constants with `|`:
///
/// ```
/// # use frost::DiagnosticsFlags;
/// let flags = DiagnosticsFlags::FPS | DiagnosticsFlags::DRAW;
/// # assert!(flags.contains(DiagnosticsFlags::FPS));
/// # assert!(!flags.contains(DiagnosticsFlags::FT));
/// ```
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticsFlags(u32);

impl DiagnosticsFlags {
    /// The frame-rate line and its chart.
    pub const FPS: Self = Self(1 << 0);
    /// The frame-time line and its chart.
    pub const FT: Self = Self(1 << 1);
    /// The processing-time line, the app-time line, and the folded
    /// processing-time chart — the total, the app, and the overlay's own
    /// cost as three series.
    pub const PROC: Self = Self(1 << 2);
    /// The draw-call line and the folded draw-call chart — the total, the
    /// app, and the overlay's own count as three series.
    pub const DRAW: Self = Self(1 << 3);

    /// Every statistic.
    pub const ALL: Self = Self(0b1111);

    /// No statistic — the window-size line alone, no charts.
    pub const NONE: Self = Self(0);

    /// No flags set.
    pub const fn none() -> Self {
        Self(0)
    }

    /// All the flags set.
    pub const fn all() -> Self {
        Self::ALL
    }

    /// Whether this set contains every flag in `other`.
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Whether no flags are set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Returns the set with the given flags added.
    pub const fn insert(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Returns the set with the given flags removed.
    pub const fn remove(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl std::ops::BitOr for DiagnosticsFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for DiagnosticsFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// A diagnostics overlay: left-aligned lines in the window's top-left
/// corner — the window size on top (`800x600`), then, for each statistic
/// enabled in [`DiagnosticsFlags`], its readout line: the smoothed frame
/// rate (`FPS 58`), the current frame time (`FT 16.7ms`), the last frame's
/// total processing time (`PROC 0.52ms`), that time excluding this
/// overlay's own update cost (`APP 0.41ms`), and the last frame's
/// draw-call count (`DRAW 48/52`, the app's share over the total) — with
/// up to four scrolling ten-second strip charts beneath: frame rate
/// (orange) on top, frame time (green) second, the folded
/// processing-time chart third — the total in blue, the app's share in
/// violet, this overlay's own cost in cyan — and the folded draw-call
/// chart below — the total in magenta, the app's share in pink, this
/// overlay's own count in rose.
///
/// Create one with [`Diagnostics::new`] (from a font file on disk) or
/// [`Diagnostics::from_bytes`] (from font data already in memory — the path
/// for environments without a file system) and the statistics to display,
/// hold it in the demo state, and call [`Process::process`] on it every
/// frame. The first call appends its nodes to the scene's root; each later
/// call updates them in place — the font is read once, and each line is
/// re-laid out only when the digits it shows change.
///
/// The parts can be hidden at runtime — Alt-0 the overlay as a whole,
/// Alt-1..Alt-4 the charts by position among the enabled ones, Alt-T the
/// readout lines, or the [`Self::toggle_all`], [`Self::toggle_chart`], and
/// [`Self::toggle_text`] methods — and the layout reflows around whatever
/// is hidden.
#[derive(Debug)]
pub struct Diagnostics {
    /// Which statistics the overlay displays: the window-size line is
    /// always shown, whatever the flags.
    flags: DiagnosticsFlags,
    /// The font's bytes, shared with the readout's text shapes.
    font: Arc<[u8]>,
    /// The readout lines' vertical extent, in pixels: the highest ink top
    /// minus the lowest ink bottom over [`PROBE`], measured once at
    /// construction, so the digits' ink differences never change it.
    line_extent: f32,
    /// The window-size line, drawn above the frame-rate line.
    size_line: Line,
    /// The frame-rate line, drawn below the window-size line.
    fps_line: Line,
    /// The frame-time line, drawn below the frame-rate line.
    ft_line: Line,
    /// The total processing-time line, drawn below the frame-time line.
    proc_line: Line,
    /// The app-time line, drawn below the processing-time line.
    app_line: Line,
    /// The draw-call line, drawn below the app-time line.
    draw_line: Line,
    /// The readout lines' slots, in append order: the size line first,
    /// then the enabled statistic lines, FPS, FT, PROC, APP, DRAW order.
    line_slots: Vec<LineSlot>,
    /// The charts' slots, in append order: the enabled statistics, FPS, FT,
    /// PROC, DRAW order.
    chart_slots: Vec<ChartSlot>,
    /// The smoothed frame rate in frames per second.
    fps: f32,
    /// The current frame time in ms — the last real `dt`, unsmoothed, so a
    /// spike is visible in the readout as well as in the graphs.
    frame_ms: f32,
    /// The last completed frame's total processing time in ms — the
    /// engine's own probe (`Context::frame_processing_ms`), unsmoothed, so
    /// a spike is visible in the readout as well as in the graph.
    proc_ms: f32,
    /// The overlay's own update cost in ms for the last completed frame —
    /// `prev_self_ms`, one frame behind, so it describes the same frame as
    /// `proc_ms`; the diagnostic series of the folded processing-time
    /// chart.
    diag_ms: f32,
    /// The last completed frame's processing time in ms with this overlay's
    /// own update cost removed — `proc_ms` minus `diag_ms`, clamped at
    /// zero.
    app_ms: f32,
    /// This overlay's own update cost in ms, measured during the previous
    /// frame's `process` — the term subtracted from the engine's total
    /// probe to get the APP value, kept one frame behind so both numbers
    /// describe the same completed frame.
    prev_self_ms: f64,
    /// The last completed frame's GPU draw-call count — the engine's own
    /// probe (`Context::frame_draw_calls`), so a spike is visible in the
    /// readout as well as in the graph.
    draw_calls: u32,
    /// The last completed frame's draw calls produced by this overlay's
    /// own nodes — the engine's own probe
    /// (`Context::frame_diagnostic_draw_calls`); the diagnostic series of
    /// the folded draw-call chart.
    diag_draws: u32,
    /// The last completed frame's draw calls minus this overlay's own —
    /// `draw_calls` minus `diag_draws`, clamped at zero; the app series of
    /// the folded draw-call chart.
    app_draws: u32,
    /// Elapsed time in seconds: the graphs' time axis, advanced by every
    /// real `dt` (stalls included — the graphs show real time).
    t: f32,
    /// The graphs' history: one [`Sample`] per completed frame, oldest
    /// first, trimmed to the last SPAN seconds.
    samples: VecDeque<Sample>,
    /// The indices of the overlay's nodes among its layer's root children,
    /// once appended (in slot order: the lines, the panels, the reference
    /// lines, the chart polylines).
    nodes: Vec<Option<usize>>,
    /// The index of the overlay's own layer in `scene.layers`, once created:
    /// the overlay draws in a dedicated topmost layer — a hard draw
    /// partition above the scene's root subtree and any other layer — so its
    /// lines and charts are always on top, never buried under the app's
    /// highest-order node. `None` until the first `process` creates it.
    layer: Option<usize>,
    /// Whether the overlay is shown at all — Alt-0 toggles this, or
    /// [`Diagnostics::toggle_all`] does. While it is off, every node draws
    /// nothing, and the per-part toggles below are remembered as-is.
    all_on: bool,
    /// Whether the readout lines (the text, the window-size line included)
    /// are shown — Alt-T toggles this, or [`Diagnostics::toggle_text`]
    /// does. The charts are independent of the lines.
    text_on: bool,
    /// Whether each chart is shown, parallel to `chart_slots` — Alt-1..
    /// Alt-4 toggle them by position among the enabled charts, top chart
    /// first, or [`Diagnostics::toggle_chart`] does.
    chart_on: Vec<bool>,
    /// The previous frame's toggle combinations — Alt-0, Alt-1, Alt-2,
    /// Alt-3, Alt-4, Alt-T, in that order — for the press edge detection
    /// in `process`.
    prev_combos: [bool; 6],
}

/// The readout's line slots, in append order: the window-size line first,
/// then the enabled statistic lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineSlot {
    /// The window-size line, always shown.
    Size,
    /// The frame-rate line.
    Fps,
    /// The frame-time line.
    Ft,
    /// The total processing-time line.
    Proc,
    /// The app-time line (enabled with the processing-time flag).
    App,
    /// The draw-call line.
    Draw,
}

/// The charts' slots, in append order: the enabled statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChartSlot {
    /// The frame-rate chart.
    Fps,
    /// The frame-time chart.
    Ft,
    /// The folded processing-time chart: total, app, and diagnostic.
    Proc,
    /// The folded draw-call chart: total, app, and diagnostic.
    Draw,
}

/// A text line's drawn state: its text and the laid-out measurements the
/// corner placement needs.
#[derive(Debug)]
struct Line {
    /// The line's last text, guarding the per-frame rebuild.
    text: String,
    /// The laid-out shape of `text`, kept until `text` changes.
    shape: Option<Shape>,
    /// The laid-out width of `text`, in pixels.
    width: f32,
    /// The line's ink top, in pixels above its baseline: the highest ink
    /// pixel, over every glyph that has one. It anchors the line's ink in
    /// its box; the stacking advances by the font-level extent
    /// (`Diagnostics::line_extent`) instead, so a line's own ink never
    /// moves the lines below it.
    ink_top: f32,
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
            baseline_offset: 0.0,
        }
    }

    /// Re-lays out `text`, which must have just changed.
    fn refresh(&mut self, font: &Arc<[u8]>) {
        let Some(layout) = crate::text::layout(font, &self.text, SIZE, WEIGHT) else {
            return;
        };
        self.shape = Shape::text_bytes(font, &self.text, SIZE).ok();
        self.width = layout.width;
        self.baseline_offset = (layout.ascent - layout.descent) / 2.0;
        // The line's ink top, the highest ink pixel over every glyph that
        // has one — initialized at the font's ascent so an all-space text
        // still gets a finite value. It anchors this line's ink in its
        // box; the stacking's advance is the font-level extent
        // (`Diagnostics::line_extent`), so the digits' ink differences
        // never move the lines below.
        let mut ink_top = layout.ascent;
        for glyph in &layout.glyphs {
            if let Some(raster) = crate::text::rasterize(font, glyph.id, SIZE, WEIGHT) {
                ink_top = ink_top.max(glyph.y + raster.top as f32);
            }
        }
        self.ink_top = ink_top;
    }
}

/// A text line's drawn state: its shape and the node origin that puts its
/// ink top at `line_top` in user space.
struct LineDraw {
    /// The line's shape.
    shape: Shape,
    /// The node origin in user space (window centered on the origin, y up).
    pos: [f32; 2],
}

/// Copies `line`'s shape and computes its node origin for a line top of
/// `line_top`.
fn line_draw(line: &Line, line_top: f32, w: f32) -> Option<LineDraw> {
    let shape = line.shape.clone()?;
    // The renderer centers a text block on the node's origin with the
    // baseline `baseline_offset` above it, so a line's node origin sits its
    // top (plus the baseline offset) below the line's top y, and its width
    // (plus the margin) in from the left edge.
    Some(LineDraw {
        shape,
        pos: [
            -w / 2.0 + MARGIN + line.width / 2.0,
            line_top - line.ink_top - line.baseline_offset,
        ],
    })
}

/// A chart's polyline points, in window user space: one per 0.1 s column,
/// at the column's center, rising from one pixel above the panel's bottom
/// edge, the value scaled to the panel height minus the two 1 px insets —
/// the stroke the chart's [`Shape::Polyline`] draws in one draw call.
fn chart_points(cols: &[f32; COLUMNS], x0: f32, panel_top: f32, top_value: f32) -> Vec<[f32; 2]> {
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

/// One sample in the graphs' history: the metrics of one completed frame,
/// recorded when that frame's `process` ran — so, by construction, one
/// frame behind the frame being produced.
#[derive(Debug, Clone, Copy)]
struct Sample {
    /// The time axis: elapsed seconds of real time (stalls included).
    t: f32,
    /// The frame time in ms — the raw `dt`.
    frame_ms: f32,
    /// The frame's total processing time in ms — the engine's probe.
    proc_ms: f32,
    /// The overlay's own update cost in ms for this frame — the
    /// one-frame-behind `prev_self_ms`, the diagnostic series of the
    /// folded processing-time chart.
    diag_ms: f32,
    /// The frame's processing time in ms with the overlay's own update
    /// cost removed.
    app_ms: f32,
    /// The frame's GPU draw-call count — the engine's probe.
    draw_calls: f32,
    /// The overlay's own draw calls in this frame — the engine's
    /// `Context::frame_diagnostic_draw_calls` probe, the diagnostic series
    /// of the folded draw-call chart.
    diag_draws: f32,
    /// The frame's draw calls minus the overlay's own — the app series of
    /// the folded draw-call chart.
    app_draws: f32,
}

/// Trims the history to the last SPAN seconds of real time ending at `now`.
fn trim_samples(samples: &mut VecDeque<Sample>, now: f32) {
    while let Some(sample) = samples.front() {
        if sample.t < now - SPAN {
            samples.pop_front();
        } else {
            break;
        }
    }
}

/// The maximum of `value(sample)` over the samples falling in each of the
/// COLUMNS time slices of the last SPAN seconds ending at `now`, index 0 =
/// the oldest slice; a sample at exactly `now` lands in the newest slice.
fn slice_max(
    samples: impl IntoIterator<Item = Sample>,
    now: f32,
    mut value: impl FnMut(&Sample) -> f32,
) -> [f32; COLUMNS] {
    let mut cols = [0.0f32; COLUMNS];
    let start = now - SPAN;
    for s in samples {
        let rel = (s.t - start) / SPAN;
        // rel > 1.0 is beyond the window; the epsilon absorbs the f32
        // rounding of `now - (now - SPAN)`, which can land just past SPAN.
        if rel <= 0.0 || rel > 1.0 + 1e-6 {
            continue;
        }
        let rel = rel.min(1.0);
        let c = (rel * COLUMNS as f32) as usize;
        let c = c.min(COLUMNS - 1);
        let v = value(&s);
        if v > cols[c] {
            cols[c] = v;
        }
    }
    cols
}

/// The number of series a chart shows: one for the single-value charts,
/// three — total, app, diagnostic — for the folded ones.
fn series_count(chart: ChartSlot) -> usize {
    match chart {
        ChartSlot::Fps | ChartSlot::Ft => 1,
        ChartSlot::Proc | ChartSlot::Draw => 3,
    }
}

/// The value, in the chart's units, of `chart`'s `k`-th series for the
/// sample.
fn series_value(chart: ChartSlot, k: usize, s: &Sample) -> f32 {
    match (chart, k) {
        (ChartSlot::Fps, _) => 1000.0 / s.frame_ms,
        (ChartSlot::Ft, _) => s.frame_ms,
        (ChartSlot::Proc, 0) => s.proc_ms,
        (ChartSlot::Proc, 1) => s.app_ms,
        (ChartSlot::Proc, 2) => s.diag_ms,
        (ChartSlot::Draw, 0) => s.draw_calls,
        (ChartSlot::Draw, 1) => s.app_draws,
        (ChartSlot::Draw, 2) => s.diag_draws,
        _ => unreachable!("k out of range for {chart:?}"),
    }
}

/// The color of `chart`'s `k`-th series.
fn series_color(chart: ChartSlot, k: usize) -> Color {
    match (chart, k) {
        (ChartSlot::Fps, _) => FPS_COLOR,
        (ChartSlot::Ft, _) => FT_COLOR,
        (ChartSlot::Proc, 0) => PROC_COLOR,
        (ChartSlot::Proc, 1) => APP_COLOR,
        (ChartSlot::Proc, 2) => DIAG_PROC_COLOR,
        (ChartSlot::Draw, 0) => DRAW_COLOR,
        (ChartSlot::Draw, 1) => APP_DRAW_COLOR,
        (ChartSlot::Draw, 2) => DIAG_DRAW_COLOR,
        _ => unreachable!("k out of range for {chart:?}"),
    }
}

/// The top of `chart`'s scale, in the chart's units: the value that reaches
/// the panel's top edge.
fn chart_top(chart: ChartSlot) -> f32 {
    match chart {
        ChartSlot::Fps => FPS_TOP,
        ChartSlot::Ft => FT_TOP,
        ChartSlot::Proc => PROC_TOP,
        ChartSlot::Draw => DRAW_TOP,
    }
}

impl Diagnostics {
    /// Creates an overlay reading its font from the file at `path`,
    /// displaying the statistics enabled in `flags`.
    ///
    /// Like [`Shape::text`], the file is read and checked to be a
    /// TrueType/OpenType font immediately, so a missing file or a non-font
    /// fails here, not at the first frame.
    pub fn new(
        path: impl AsRef<std::path::Path>,
        flags: DiagnosticsFlags,
    ) -> Result<Self, TextError> {
        let bytes = std::fs::read(path.as_ref()).map_err(TextError::Io)?;
        Self::from_bytes(&bytes, flags)
    }

    /// Creates an overlay from font data already in memory, for example
    /// bytes embedded into the binary with `include_bytes!`, displaying the
    /// statistics enabled in `flags`.
    ///
    /// Like [`Shape::text_bytes`], the bytes are checked to be a
    /// TrueType/OpenType font up front and shared behind an `Arc`, but no
    /// file is read — this is how an overlay is created in environments
    /// without a file system, such as a web browser.
    pub fn from_bytes(font: &[u8], flags: DiagnosticsFlags) -> Result<Self, TextError> {
        swash::FontRef::from_index(font, 0).ok_or(TextError::InvalidFont)?;
        // The slots in append order: the readout lines (the size line
        // first, then the enabled lines, FPS, FT, PROC, APP, DRAW order —
        // the app line rides on the processing flag), the enabled chart
        // panels, the reference lines (the draw chart has none), and the
        // chart polylines (three per folded chart).
        let mut line_slots = vec![LineSlot::Size];
        let mut chart_slots = Vec::new();
        if flags.contains(DiagnosticsFlags::FPS) {
            line_slots.push(LineSlot::Fps);
            chart_slots.push(ChartSlot::Fps);
        }
        if flags.contains(DiagnosticsFlags::FT) {
            line_slots.push(LineSlot::Ft);
            chart_slots.push(ChartSlot::Ft);
        }
        if flags.contains(DiagnosticsFlags::PROC) {
            line_slots.push(LineSlot::Proc);
            line_slots.push(LineSlot::App);
            chart_slots.push(ChartSlot::Proc);
        }
        if flags.contains(DiagnosticsFlags::DRAW) {
            line_slots.push(LineSlot::Draw);
            chart_slots.push(ChartSlot::Draw);
        }
        let n_refs = chart_slots
            .iter()
            .filter(|c| !matches!(c, &ChartSlot::Draw))
            .count();
        let n_polys: usize = chart_slots.iter().map(|c| series_count(*c)).sum();
        let n_nodes = line_slots.len() + chart_slots.len() + n_refs + n_polys;
        let chart_on = vec![true; chart_slots.len()];
        // The readout lines' vertical extent, measured once over the probe —
        // see `PROBE`. The font just validated, so the probe shapes; a
        // missing ink (an all-space probe) falls back to the font's
        // ascent-plus-descent.
        let line_extent = if let Some(probe) = crate::text::layout(font, PROBE, SIZE, WEIGHT) {
            let (mut top, mut bottom) = (f32::NEG_INFINITY, f32::INFINITY);
            for glyph in &probe.glyphs {
                if let Some(raster) = crate::text::rasterize(font, glyph.id, SIZE, WEIGHT) {
                    let ink_top = glyph.y + raster.top as f32;
                    top = top.max(ink_top);
                    bottom = bottom.min(ink_top - raster.height as f32);
                }
            }
            if top.is_finite() {
                top - bottom
            } else {
                probe.ascent + probe.descent
            }
        } else {
            SIZE
        };
        Ok(Self {
            flags,
            font: Arc::from(font),
            line_extent,
            size_line: Line::new(),
            fps_line: Line::new(),
            ft_line: Line::new(),
            proc_line: Line::new(),
            app_line: Line::new(),
            draw_line: Line::new(),
            line_slots,
            chart_slots,
            fps: 0.0,
            frame_ms: 0.0,
            proc_ms: 0.0,
            diag_ms: 0.0,
            app_ms: 0.0,
            prev_self_ms: 0.0,
            draw_calls: 0,
            diag_draws: 0,
            app_draws: 0,
            t: 0.0,
            samples: VecDeque::new(),
            nodes: vec![None; n_nodes],
            layer: None,
            all_on: true,
            text_on: true,
            chart_on,
            prev_combos: [false; 6],
        })
    }

    /// The line for `slot`.
    fn line(&self, slot: LineSlot) -> &Line {
        match slot {
            LineSlot::Size => &self.size_line,
            LineSlot::Fps => &self.fps_line,
            LineSlot::Ft => &self.ft_line,
            LineSlot::Proc => &self.proc_line,
            LineSlot::App => &self.app_line,
            LineSlot::Draw => &self.draw_line,
        }
    }

    /// The mutable line for `slot`.
    fn line_mut(&mut self, slot: LineSlot) -> &mut Line {
        match slot {
            LineSlot::Size => &mut self.size_line,
            LineSlot::Fps => &mut self.fps_line,
            LineSlot::Ft => &mut self.ft_line,
            LineSlot::Proc => &mut self.proc_line,
            LineSlot::App => &mut self.app_line,
            LineSlot::Draw => &mut self.draw_line,
        }
    }

    /// Re-lays out the line for `slot` when `text` differs from its last
    /// text — the per-frame rebuild guard.
    fn refresh_line(&mut self, slot: LineSlot, text: String) {
        if self.line(slot).text == text {
            return;
        }
        // The font is a separate field, but `line_mut` borrows the whole
        // struct, so hand the layout a clone of the shared bytes.
        let font = Arc::clone(&self.font);
        let line = self.line_mut(slot);
        line.text = text;
        line.refresh(&font);
    }

    /// The index of the overlay's own layer in `scene.layers`, creating it on
    /// first use. The overlay draws in a dedicated topmost layer — a hard
    /// draw partition above the scene's root subtree and any other layer — so
    /// its lines and charts are always on top, never buried under the app's
    /// highest-order node. The layer's order is set above every layer already
    /// in the scene (or `1.0`, above the root subtree, when there are none),
    /// and a stale index (the scene lost or rebuilt its layers) recreates it,
    /// re-appending the overlay's nodes into the fresh layer.
    fn ensure_layer(&mut self, scene: &mut Scene) -> usize {
        let valid = self.layer.is_some_and(|idx| idx < scene.layers.len());
        if valid {
            return self.layer.expect("checked above");
        }
        let order = scene
            .layers
            .iter()
            .map(|l| l.order)
            .max_by(|a, b| a.total_cmp(b))
            .map(|m| m + 1.0)
            .unwrap_or(1.0);
        let mut layer = Layer::new(SceneNode::default());
        layer.order = order;
        let idx = scene.layers.len();
        scene.layers.push(layer);
        self.layer = Some(idx);
        // The node indices were relative to the previous group's children;
        // reset so every node is re-appended into the fresh layer.
        self.nodes.fill(None);
        idx
    }

    /// Finds the node for overlay slot `slot` (re-appending it if the demo
    /// removed it) and updates its shape and transform in place, in the
    /// overlay's own layer. Every node the overlay owns is flagged
    /// `diagnostic`, so the engine counts its draws separately
    /// (`Context::frame_diagnostic_draw_calls`).
    fn place(&mut self, slot: usize, pos: [f32; 2], shape: Shape, scene: &mut Scene, layer: usize) {
        let children = &mut scene.layers[layer].root.children;
        let index = match self.nodes[slot] {
            Some(idx) if idx < children.len() => idx,
            _ => {
                children.push(Box::new(SceneNode {
                    transform: Transform::translate(pos),
                    shape: Some(shape.clone()),
                    diagnostic: true,
                    ..Default::default()
                }));
                children.len() - 1
            }
        };
        self.nodes[slot] = Some(index);
        let node = &mut children[index];
        node.diagnostic = true;
        node.shape = Some(shape);
        node.transform = Transform::translate(pos);
    }

    /// Toggles the overlay as a whole — the Alt-0 shortcut. While the
    /// overlay is off, every one of its nodes draws nothing; the per-part
    /// toggles below are remembered and apply when it comes back.
    pub fn toggle_all(&mut self) {
        self.all_on = !self.all_on;
    }

    /// Toggles the readout lines (the text, the window-size line included)
    /// — the Alt-T shortcut. The charts are independent of the lines.
    pub fn toggle_text(&mut self) {
        self.text_on = !self.text_on;
    }

    /// Toggles the `index`-th enabled chart, top chart first — the
    /// Alt-1..Alt-4 shortcuts. An index past the last enabled chart is a
    /// no-op.
    pub fn toggle_chart(&mut self, index: usize) {
        if let Some(on) = self.chart_on.get_mut(index) {
            *on = !*on;
        }
    }

    /// Whether the readout lines are shown: the overlay as a whole on and
    /// the text on.
    fn lines_on(&self) -> bool {
        self.all_on && self.text_on
    }

    /// Whether the `index`-th enabled chart is shown: the overlay as a
    /// whole on and that chart's own toggle on.
    fn chart_on_at(&self, index: usize) -> bool {
        self.all_on && self.chart_on.get(index).copied().unwrap_or(false)
    }

    /// Applies the press edges of the six toggle combinations — Alt-0,
    /// Alt-1, Alt-2, Alt-3, Alt-4, Alt-T, in the order of `combos` — to the
    /// toggle state: a combination toggles its part once when it goes from
    /// released to held, and holding or releasing it does nothing. Called
    /// from `process` with the current key states.
    fn apply_key_edges(&mut self, combos: [bool; 6]) {
        for (i, &combo) in combos.iter().enumerate() {
            if combo && !self.prev_combos[i] {
                match i {
                    0 => self.toggle_all(),
                    1..=4 => self.toggle_chart(i - 1),
                    _ => self.toggle_text(),
                }
            }
        }
        self.prev_combos = combos;
    }

    /// Hides the node for overlay slot `slot`: its shape is cleared,
    /// leaving a bare pivot that draws nothing, until the matching toggle
    /// shows it again and `place` restores the shape. A node the demo
    /// removed is left removed — `place` re-appends it on the way back.
    fn hide(&mut self, slot: usize, scene: &mut Scene, layer: usize) {
        let Some(&idx) = self.nodes[slot].as_ref() else {
            return;
        };
        let children = &mut scene.layers[layer].root.children;
        if idx < children.len() {
            children[idx].shape = None;
        }
    }
}

impl Process for Diagnostics {
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        // The first stop on the clock is this overlay's own update cost:
        // the last statement stores it, and the next frame subtracts it
        // from the engine's total probe — one frame behind, so both
        // numbers describe the same completed frame.
        let t0 = now_millis();
        // The keyboard shortcuts, on press edges only so a held key
        // toggles once: Alt-0 the overlay as a whole, Alt-1..Alt-4 the
        // charts by position among the enabled ones (top chart first), and
        // Alt-T the readout lines. The layout reflows around whatever is
        // hidden. The demo still sees the same keys — they are read here,
        // not consumed.
        let alt = ctx.key_down(KeyCode::AltLeft) || ctx.key_down(KeyCode::AltRight);
        self.apply_key_edges([
            alt && ctx.key_down(KeyCode::Digit0),
            alt && ctx.key_down(KeyCode::Digit1),
            alt && ctx.key_down(KeyCode::Digit2),
            alt && ctx.key_down(KeyCode::Digit3),
            alt && ctx.key_down(KeyCode::Digit4),
            alt && ctx.key_down(KeyCode::KeyT),
        ]);
        // The engine's own probes of the last completed frame: its CPU
        // work, its total GPU draw calls, and the draw calls this overlay's
        // own nodes produced. This frame's own probes are still running, so
        // the values are the previous frame's by construction — a one-frame
        // lag, one 0.1 s column at 60 fps, invisible in the graphs.
        self.proc_ms = ctx.frame_processing_ms() as f32;
        self.draw_calls = ctx.frame_draw_calls();
        let diag_draws = ctx.frame_diagnostic_draw_calls();
        // The overlay's own share of that frame, and the app values — the
        // totals with the share removed: the update cost (measured in the
        // previous call, clamped at zero against timer jitter) and the
        // tagged draw calls.
        self.diag_ms = self.prev_self_ms as f32;
        self.app_ms = ((self.proc_ms as f64) - self.prev_self_ms).max(0.0) as f32;
        self.diag_draws = diag_draws;
        self.app_draws = self.draw_calls.saturating_sub(diag_draws);
        // Record the frame in the graphs' history: the frame time is the
        // raw gap (a spike must be visible in the readout and the graphs),
        // and the time axis is real time — stalls included. The first
        // frame's dt is 0.0; nothing to record.
        if dt > 0.0 {
            self.frame_ms = dt * 1000.0;
            self.t += dt;
            self.samples.push_back(Sample {
                t: self.t,
                frame_ms: self.frame_ms,
                proc_ms: self.proc_ms,
                diag_ms: self.diag_ms,
                app_ms: self.app_ms,
                draw_calls: self.draw_calls as f32,
                diag_draws: diag_draws as f32,
                app_draws: self.app_draws as f32,
            });
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
        // Whether the readout lines show: the overlay as a whole on and the
        // text on (Alt-T).
        let lines_on = self.lines_on();
        // Rebuild each line's shape only when its text actually changes —
        // the size line always, a statistic line only when its flag is on;
        // while the text is hidden the lines are not rebuilt at all.
        if lines_on {
            self.refresh_line(LineSlot::Size, format!("{}x{}", w as u32, h as u32));
            if self.flags.contains(DiagnosticsFlags::FPS) {
                self.refresh_line(LineSlot::Fps, format!("FPS {:.0}", self.fps.round()));
            }
            if self.flags.contains(DiagnosticsFlags::FT) {
                self.refresh_line(LineSlot::Ft, format!("FT {:.1}ms", self.frame_ms));
            }
            if self.flags.contains(DiagnosticsFlags::PROC) {
                self.refresh_line(LineSlot::Proc, format!("PROC {:.2}ms", self.proc_ms));
                self.refresh_line(LineSlot::App, format!("APP {:.2}ms", self.app_ms));
            }
            if self.flags.contains(DiagnosticsFlags::DRAW) {
                self.refresh_line(
                    LineSlot::Draw,
                    format!("DRAW {}/{}", self.app_draws, self.draw_calls),
                );
            }
        }
        // Lay the visible parts out top to bottom, so hiding a line or a
        // chart reflows the rest into the corner: the topmost visible line's
        // ink top sits MARGIN in from the window's top edge; each visible
        // line hangs LINE_GAP under the previous, the advance being the
        // font-level line extent (measured over the readout's alphabet, so
        // the digits' ink differences never move the lines or the charts);
        // the first visible panel's top sits MARGIN from the top edge when
        // no line is visible, otherwise LINE_GAP + GRAPH_GAP under the last
        // visible line's bottom; each visible panel hangs one panel
        // height plus GRAPH_GAP under the previous; all visible lines' ink
        // left edges sit MARGIN in from the window's left edge. The slot
        // lists are tiny — at most six lines and four charts — so clone
        // them, letting the loops below borrow the struct freely while
        // placing nodes.
        let line_slots = self.line_slots.clone();
        let chart_slots = self.chart_slots.clone();
        // The visible slots' positions in the full slot lists — the node
        // indices were assigned by the full order at first append, so the
        // visibility never changes the slot arithmetic.
        let visible_lines: Vec<usize> = if lines_on {
            (0..line_slots.len()).collect()
        } else {
            Vec::new()
        };
        let visible_charts: Vec<usize> = (0..chart_slots.len())
            .filter(|j| self.chart_on_at(*j))
            .collect();
        let mut next_line_top = h / 2.0 - MARGIN;
        let mut line_tops = vec![0.0f32; line_slots.len()];
        for &i in &visible_lines {
            line_tops[i] = next_line_top;
            next_line_top -= self.line_extent + LINE_GAP;
        }
        let mut panel_tops = vec![0.0f32; chart_slots.len()];
        let mut next_panel_top = if visible_lines.is_empty() {
            h / 2.0 - MARGIN
        } else {
            next_line_top - GRAPH_GAP
        };
        for &j in &visible_charts {
            panel_tops[j] = next_panel_top;
            next_panel_top -= GRAPH_H + GRAPH_GAP;
        }
        let x0 = -w / 2.0 + MARGIN;

        let scene = ctx.scene();
        // The overlay's own topmost layer, created on the first frame.
        let layer = self.ensure_layer(scene);
        // The readout lines: place the visible ones, hide the rest.
        for (i, &slot) in line_slots.iter().enumerate() {
            if !lines_on {
                self.hide(i, scene, layer);
                continue;
            }
            let Some(draw) = line_draw(self.line(slot), line_tops[i], w) else {
                continue;
            };
            self.place(i, draw.pos, draw.shape, scene, layer);
        }
        // The panels: one per visible chart, in slot order; a hidden
        // chart's panel is cleared.
        for (j, _) in chart_slots.iter().enumerate() {
            if self.chart_on_at(j) {
                let top = panel_tops[j];
                self.place(
                    line_slots.len() + j,
                    [x0 + GRAPH_W / 2.0, top - GRAPH_H / 2.0],
                    Shape::Rectangle {
                        center: [0.0, 0.0],
                        extent: [GRAPH_W / 2.0, GRAPH_H / 2.0],
                        color: PANEL,
                    },
                    scene,
                    layer,
                );
            } else {
                self.hide(line_slots.len() + j, scene, layer);
            }
        }
        // The 60 fps / 16.7 ms reference lines, 1 px tall across the panel
        // — the draw-call chart has no budget, so no reference.
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
        let mut ref_slot = line_slots.len() + chart_slots.len();
        for (j, &chart) in chart_slots.iter().enumerate() {
            if matches!(chart, ChartSlot::Draw) {
                continue;
            }
            if self.chart_on_at(j) {
                let level = match chart {
                    ChartSlot::Fps => 60.0 / FPS_TOP,
                    ChartSlot::Ft => (1000.0 / 60.0) / FT_TOP,
                    ChartSlot::Proc => (1000.0 / 60.0) / PROC_TOP,
                    ChartSlot::Draw => unreachable!(),
                };
                let (y, shape) = ref_line(panel_tops[j], level);
                self.place(ref_slot, [x0, y], shape, scene, layer);
            } else {
                self.hide(ref_slot, scene, layer);
            }
            ref_slot += 1;
        }
        // The charts' polylines: one per series, through the max of each
        // 0.1 s column — one draw call per series, three per folded chart;
        // a hidden chart's polylines are cleared without computing their
        // columns.
        let mut poly_slot = ref_slot;
        for (j, &chart) in chart_slots.iter().enumerate() {
            for k in 0..series_count(chart) {
                if self.chart_on_at(j) {
                    let cols = slice_max(self.samples.iter().copied(), self.t, |s| {
                        series_value(chart, k, s)
                    });
                    self.place(
                        poly_slot,
                        [0.0, 0.0],
                        Shape::Polyline {
                            points: chart_points(&cols, x0, panel_tops[j], chart_top(chart)),
                            width: LINE_WIDTH,
                            color: series_color(chart, k),
                        },
                        scene,
                        layer,
                    );
                } else {
                    self.hide(poly_slot, scene, layer);
                }
                poly_slot += 1;
            }
        }
        // This frame's own update cost, for the next frame's APP value.
        self.prev_self_ms = now_millis() - t0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test sample with the given metrics.
    #[allow(clippy::too_many_arguments)]
    fn sample(
        t: f32,
        frame_ms: f32,
        proc_ms: f32,
        diag_ms: f32,
        app_ms: f32,
        draw_calls: f32,
        diag_draws: f32,
        app_draws: f32,
    ) -> Sample {
        Sample {
            t,
            frame_ms,
            proc_ms,
            diag_ms,
            app_ms,
            draw_calls,
            diag_draws,
            app_draws,
        }
    }

    #[test]
    fn flags_combine_and_test_independently() {
        let none = DiagnosticsFlags::none();
        assert!(none.is_empty());
        assert!(!none.contains(DiagnosticsFlags::FPS));
        let flags = DiagnosticsFlags::FPS | DiagnosticsFlags::DRAW;
        assert!(flags.contains(DiagnosticsFlags::FPS));
        assert!(flags.contains(DiagnosticsFlags::DRAW));
        assert!(!flags.contains(DiagnosticsFlags::FT));
        assert!(!flags.contains(DiagnosticsFlags::PROC));
        assert!(
            flags
                .insert(DiagnosticsFlags::FT)
                .contains(DiagnosticsFlags::FT)
        );
        assert!(
            !flags
                .remove(DiagnosticsFlags::FPS)
                .contains(DiagnosticsFlags::FPS)
        );
        let all = DiagnosticsFlags::all();
        assert!(all.contains(flags));
        assert!(!flags.contains(all));
    }

    #[test]
    fn trim_keeps_only_the_last_span_seconds() {
        let mut samples = VecDeque::new();
        for t in 0..11 {
            samples.push_back(sample(t as f32, 16.0, 1.0, 0.25, 0.75, 4.0, 1.0, 3.0));
        }
        trim_samples(&mut samples, 11.0);
        // t < 11 - 10 = 1 is gone; t = 1..=11 remain.
        assert_eq!(samples.front().unwrap().t, 1.0);
        assert_eq!(samples.len(), 10);
    }

    #[test]
    fn slice_max_buckets_by_time_not_by_count() {
        // now = 10: the window is t = 0..10, 100 slices of 0.1 s.
        let samples = [
            sample(0.05, 100.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0), // oldest slice (c = 0)
            sample(5.00, 200.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0), // middle of the window (c = 50)
            sample(9.95, 100.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0), // newest slice (c = 99)
        ];
        let cols = slice_max(samples, 10.0, |s| s.frame_ms);
        assert_eq!(cols[0], 100.0);
        assert_eq!(cols[50], 200.0);
        assert_eq!(cols[99], 100.0);
        assert_eq!(cols[49], 0.0);
        assert_eq!(cols[98], 0.0);
    }

    #[test]
    fn slice_max_counts_the_newest_sample_at_now() {
        // t == now: the sample belongs to the newest slice, not beyond it.
        let samples = [sample(10.0, 300.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)];
        let cols = slice_max(samples, 10.0, |s| s.frame_ms);
        assert_eq!(cols[99], 300.0);
    }

    #[test]
    fn slice_max_ignores_samples_outside_the_window() {
        let samples = [
            sample(15.0, 100.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
            sample(20.0, 100.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        ];
        let cols = slice_max(samples, 10.0, |s| s.frame_ms);
        assert!(cols.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn slice_max_fps_is_the_inverse_of_the_frame_time() {
        // Two frames in one slice: 10 ms and 40 ms → the slice's best rate
        // is 100 fps.
        let samples = [
            sample(9.95, 10.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
            sample(9.98, 40.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        ];
        let cols = slice_max(samples, 10.0, |s| 1000.0 / s.frame_ms);
        assert_eq!(cols[99], 100.0);
    }

    #[test]
    fn slice_max_proc_ignores_the_frame_time() {
        // Two frames in one slice: 10 ms and 40 ms of frame time, 2.5 ms
        // and 1.0 ms of processing — the proc column is the max of the
        // processing times, not the frame times.
        let samples = [
            sample(9.95, 10.0, 2.5, 0.5, 2.0, 0.0, 0.0, 0.0),
            sample(9.98, 40.0, 1.0, 0.25, 0.75, 0.0, 0.0, 0.0),
        ];
        let cols = slice_max(samples, 10.0, |s| s.proc_ms);
        assert_eq!(cols[99], 2.5);
    }

    #[test]
    fn slice_max_app_ignores_the_total() {
        // Two frames in one slice: 2.5 ms and 1.0 ms of total processing,
        // 2.0 ms and 0.75 ms excluding the overlay — the app column is the
        // max of the overlay-excluded times, not the totals.
        let samples = [
            sample(9.95, 10.0, 2.5, 0.5, 2.0, 0.0, 0.0, 0.0),
            sample(9.98, 40.0, 1.0, 0.25, 0.75, 0.0, 0.0, 0.0),
        ];
        let cols = slice_max(samples, 10.0, |s| s.app_ms);
        assert_eq!(cols[99], 2.0);
    }

    #[test]
    fn slice_max_draw_ignores_the_times() {
        // Two frames in one slice: 12 and 7 total draw calls — the draw
        // column is the max of the counts, not of any time.
        let samples = [
            sample(9.95, 10.0, 2.5, 0.5, 2.0, 12.0, 4.0, 8.0),
            sample(9.98, 40.0, 1.0, 0.25, 0.75, 7.0, 2.0, 5.0),
        ];
        let cols = slice_max(samples, 10.0, |s| s.draw_calls);
        assert_eq!(cols[99], 12.0);
    }

    #[test]
    fn slice_max_projects_the_draw_split_independently() {
        // Two frames in one slice: 12 and 7 total draw calls, 4 and 2 of
        // them the overlay's own — the app column is the max of the
        // overlay-free counts (12 - 4 = 8), and the diagnostic column the
        // max of the overlay's own counts (4).
        let samples = [
            sample(9.95, 10.0, 0.0, 0.0, 0.0, 12.0, 4.0, 8.0),
            sample(9.98, 40.0, 0.0, 0.0, 0.0, 7.0, 2.0, 5.0),
        ];
        let cols = slice_max(samples, 10.0, |s| s.app_draws);
        assert_eq!(cols[99], 8.0);
        let cols = slice_max(samples, 10.0, |s| s.diag_draws);
        assert_eq!(cols[99], 4.0);
    }

    /// A real TrueType font, embedded so the toggle tests can construct an
    /// overlay without a file system.
    const FONT: &[u8] = include_bytes!("../assets/fonts/Leofont-Regular.ttf");

    /// A diagnostics overlay with every flag, for the toggle tests.
    fn overlay() -> Diagnostics {
        Diagnostics::from_bytes(FONT, DiagnosticsFlags::all()).unwrap()
    }

    #[test]
    fn toggles_start_everything_on() {
        let d = overlay();
        assert!(d.all_on && d.text_on);
        assert_eq!(d.chart_on, [true, true, true, true]);
    }

    #[test]
    fn key_edges_fire_once_per_press() {
        let mut d = overlay();
        // Nothing pressed: no change.
        d.apply_key_edges([false; 6]);
        assert!(d.all_on && d.text_on && d.chart_on.iter().all(|&b| b));
        // Alt-1 (slot 1): the top chart toggles off.
        d.apply_key_edges([false, true, false, false, false, false]);
        assert_eq!(d.chart_on, [false, true, true, true]);
        // Holding the combination: no further change.
        d.apply_key_edges([false, true, false, false, false, false]);
        assert_eq!(d.chart_on, [false, true, true, true]);
        // Release, then Alt-4: the bottom chart toggles off.
        d.apply_key_edges([false, false, false, false, true, false]);
        assert_eq!(d.chart_on, [false, true, true, false]);
        // Alt-T: the lines toggle off, the charts untouched.
        d.apply_key_edges([false, false, false, false, false, true]);
        assert!(!d.text_on);
        assert_eq!(d.chart_on, [false, true, true, false]);
        // Alt-0: the overlay as a whole toggles off.
        d.apply_key_edges([true, false, false, false, false, false]);
        assert!(!d.all_on);
    }

    #[test]
    fn visibility_composes_the_toggles() {
        let mut d = overlay();
        assert!(d.lines_on());
        assert!(d.chart_on_at(0));
        // A chart toggle hides only that chart.
        d.toggle_chart(0);
        assert!(d.lines_on());
        assert!(!d.chart_on_at(0));
        assert!(d.chart_on_at(1));
        // A text toggle hides the lines, not the charts.
        d.toggle_text();
        assert!(!d.lines_on());
        assert!(d.chart_on_at(1));
        // The global toggle hides everything, remembering the per-part
        // state.
        d.toggle_all();
        assert!(!d.lines_on());
        assert!(!d.chart_on_at(1));
        d.toggle_all();
        assert!(!d.lines_on());
        assert!(!d.chart_on_at(0));
        assert!(d.chart_on_at(1));
    }

    #[test]
    fn toggle_chart_is_a_noop_past_the_last_chart() {
        let mut d = overlay();
        d.toggle_chart(4);
        d.toggle_chart(usize::MAX);
        assert!(d.chart_on.iter().all(|&b| b));
    }

    /// A real monospaced TrueType font, embedded so the line tests can
    /// shape text without a file system.
    const FIRA: &[u8] = include_bytes!("../assets/fonts/FiraCode-VariableFont_wght.ttf");

    /// The readout's ink is not uniform below the baseline — Fira Code's
    /// "8" and "9" carry a pixel of antialiasing under it, where the "4"
    /// does not — and a font's vertical metrics can be degenerate, so the
    /// overlay measures its line extent once, at construction, over the
    /// whole readout alphabet (see `PROBE`): the stacking's advance is a
    /// constant, and no digits change can ever move a line or the charts
    /// below it.
    #[test]
    fn line_extent_covers_every_readouts_ink() {
        let d = Diagnostics::from_bytes(FIRA, DiagnosticsFlags::all()).unwrap();
        for text in [
            "8",
            "9",
            "18",
            "19",
            "59",
            "61",
            "1920x1080",
            "FPS 60",
            "FT 16.7ms",
            "PROC 12.34ms",
            "APP 12.12ms",
            "DRAW 38/42",
        ] {
            let layout = crate::text::layout(FIRA, text, SIZE, WEIGHT).unwrap();
            let (mut top, mut bottom) = (f32::NEG_INFINITY, f32::INFINITY);
            for glyph in &layout.glyphs {
                if let Some(raster) = crate::text::rasterize(FIRA, glyph.id, SIZE, WEIGHT) {
                    let ink_top = glyph.y + raster.top as f32;
                    top = top.max(ink_top);
                    bottom = bottom.min(ink_top - raster.height as f32);
                }
            }
            assert!(
                top - bottom <= d.line_extent + 1e-3,
                "{text}: the ink extent {} must fit the line extent {}",
                top - bottom,
                d.line_extent
            );
        }
    }
}
