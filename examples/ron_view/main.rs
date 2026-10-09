//! ron_view: a foldable, scrollable view of a `.ron` file.
//!
//! The window is one page: the parsed RON file rendered as a tree of
//! text rows, every container foldable with a click, and the whole
//! content scrolled by dragging the two scroll handles — a vertical
//! thumb at the right edge, a horizontal one along the bottom — or by
//! the wheel (Shift turns it sideways). The text is FiraCode Variable:
//! keys, container heads and titles carry a heavy 700 weight, values a
//! medium 500, so the tree's spine stands out from its leaves.
//!
//! The parser is the crate's own `frost::ron` — a hand-rolled RON
//! subset with no parser crate behind it:
//! line and (nestable) block comments, `!` type markers, `Name(...)`
//! and `Name {...}` structs, named and positional fields, `(...)`
//! anonymous structs, arrays, maps with string-ish keys, strings and
//! chars (kept verbatim, escapes and all), integers, floats,
//! exponents, underscores, bools, and enum paths — unit (`Kind::Bee`),
//! newtype (`Kind::Bug(Count(3))`) and struct variants
//! (`Watering::PingPong { .. }`) — with trailing commas allowed
//! everywhere. It errors with a line number rather than guessing.
//!
//! The parser, the tree model and the writer live in the crate, shared
//! with `sprite_util`'s sidecar panels; what stays in this folder is
//! the row flattener, `view.rs` — the same file `sprite_util`
//! path-includes — which flattens a tree into the rows both viewers
//! draw.
//!
//! Each visible row owns two text shapes: the heavy prefix (indent,
//! `[+]/[-]` marker, key) and the value side (head, atom, or the `…`
//! of a folded container). The shapes live in a fixed node pool; a
//! fold rebuilds the row list, a scroll just re-places (and
//! shows/hides) the shapes of the rows that cross the page edges.
//!
//! Run with:
//!
//! ```text
//! cargo run --example ron_view                              # the garden
//! cargo run --example ron_view -- -i assets/ron/garden.ron  # or any .ron
//! ```

use frost::ron::{Kind, Val, parse, walk};
use std::sync::Arc;

/// The row view: the tree flattened into visible rows. The data side —
/// parser, tree model, writer — is the crate's own `frost::ron`;
/// `sprite_util` compiles this same file for its sidecar panels.
mod view;
use view::{Row, VKind, layout};

// ---------------------------------------------------------------------------
// The view

/// One font pixel's advance: FiraCode is monospaced at exactly 600/1000
/// em, so a row's width is its character count times this — and weight
/// never changes it, which keeps the two weights perfectly aligned.
const ADVANCE_EM: f32 = 0.6;

/// The text size, in pixels per em.
const SIZE: f32 = 18.0;
/// Half FiraCode's cap height (1374 units per 2000 em): the shaper gets
/// degenerate vertical metrics from this font, so the renderer prints a
/// text node with its baseline AT the node origin and the ink above it.
/// Row text drops by this much so its ink centres in its line — and in
/// the hover stripe that highlights it.
const LIFT: f32 = 0.344 * SIZE;

/// One row's height.
const LINE_H: f32 = 25.0;

/// The scene node pool: two text shapes per row, for this many rows.
const ROWS: usize = 256;

/// The pool's text nodes, plus the title and status lines.
const NODES: usize = ROWS * 2 + 2;

const TITLE_NODE: usize = ROWS * 2;
const STATUS_NODE: usize = ROWS * 2 + 1;

/// The page's margins: text inset, bar thickness, bar gutters.
const PAD: f32 = 8.0;
const BAR: f32 = 10.0;
const GUTTER: f32 = 6.0;

fn rgb(r: f32, g: f32, b: f32) -> frost::Color {
    frost::Color { r, g, b, a: 1.0 }
}

/// The heavy weight: keys, heads, titles.
const HEAVY: f32 = 700.0;
/// The medium weight: values, chrome lines.
const MEDIUM: f32 = 500.0;

/// The scroll handle's thumb: size and start along a track `track`
/// long, showing viewport `view` of content `total` at offset
/// `scroll`. A content that fits gets a full-length thumb.
fn thumb(scroll: f32, view: f32, total: f32, track: f32) -> (f32, f32) {
    if total <= view || track <= 0.0 {
        return (0.0, track.max(0.0));
    }
    let len = (view / total * track).max(24.0).min(track);
    let start = (scroll / (total - view)).clamp(0.0, 1.0) * (track - len);
    (start, len)
}

/// The view's state.
struct Demo {
    /// The parsed tree, fold flags included.
    root: Val,
    /// The font bytes, shared by every row's shapes.
    font: Arc<[u8]>,
    /// The flattened visible rows.
    rows: Vec<Row>,
    /// The content's extent beyond the page, the scroll ranges.
    content_w: f32,
    content_h: f32,
    /// The scroll offsets, in pixels, both 0 at the content's top-left.
    sx: f32,
    sy: f32,
    /// Which handle is grabbed, and where inside the thumb the grab
    /// landed (along the track, from the track's far end — the y axis
    /// runs down the rows, the x axis rightwards).
    grab: Grab,
    /// Whether the left button was held last frame: its edges are the
    /// press and the release.
    was_down: bool,
    /// The current press, if it began on the page: its position, for
    /// the click-versus-drag choice.
    press: Option<[f32; 2]>,
    /// The hovered row, for the stripe.
    hover: Option<usize>,
    /// The page's edges, refreshed every frame.
    page: Page,
    /// The name shown in the title.
    name: String,
    /// Whether Q was held last frame.
    was_q: bool,
}

/// Which scroll handle the mouse holds.
#[derive(Clone, Copy, PartialEq)]
enum Grab {
    None,
    Y { off: f32, len: f32 },
    X { off: f32, len: f32 },
}

/// The page's edges at a window of `w` x `h`: inside the title and
/// status lines and the two bar gutters.
#[derive(Clone, Copy)]
struct Page {
    l: f32,
    r: f32,
    t: f32,
    b: f32,
}

impl Page {
    fn new(w: f32, h: f32) -> Self {
        Self {
            l: -w / 2.0 + GUTTER,
            r: w / 2.0 - GUTTER - BAR - 2.0,
            t: h / 2.0 - 34.0,
            b: -h / 2.0 + 30.0 + BAR + 2.0,
        }
    }

    fn vw(&self) -> f32 {
        (self.r - self.l).max(1.0)
    }

    fn vh(&self) -> f32 {
        (self.t - self.b).max(1.0)
    }
}

impl Demo {
    fn relayout(&mut self) {
        self.rows = layout(&self.root);
        let widest = self
            .rows
            .iter()
            .map(|r| (r.key.chars().count() + r.val.chars().count()) as f32)
            .fold(0.0f32, f32::max);
        self.content_w = widest * SIZE * ADVANCE_EM + PAD;
        self.content_h = self.rows.len() as f32 * LINE_H + PAD;
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        let page = self.page;
        self.sx = self.sx.clamp(0.0, (self.content_w - page.vw()).max(0.0));
        self.sy = self.sy.clamp(0.0, (self.content_h - page.vh()).max(0.0));
    }

    /// A text shape from the shared font bytes: a clone keeps the Arc,
    /// never re-reads the file.
    fn text(&self, s: String, weight: f32, color: frost::Color) -> frost::Shape {
        frost::Shape::Text {
            text: s,
            font: Arc::clone(&self.font),
            size: SIZE,
            weight,
            color,
            alpha: 1.0,
        }
    }

    /// The row under a page point, given the scroll.
    fn row_at(&self, p: [f32; 2]) -> Option<usize> {
        let page = self.page;
        let i = ((page.t - PAD - p[1] + self.sy) / LINE_H).floor();
        if i < 0.0 {
            return None;
        }
        let i = i as usize;
        (i < self.rows.len() && i < ROWS).then_some(i)
    }

    /// The value side's color for its kind.
    fn vcolor(k: VKind) -> frost::Color {
        match k {
            VKind::Head => rgb(0.93, 0.78, 0.44),
            VKind::Close => rgb(0.55, 0.55, 0.60),
            VKind::Atom(Kind::Str) => rgb(0.72, 0.86, 0.66),
            VKind::Atom(Kind::Char) => rgb(0.66, 0.80, 0.78),
            VKind::Atom(Kind::Num) => rgb(0.60, 0.78, 0.96),
            VKind::Atom(Kind::Bool) => rgb(0.83, 0.68, 0.95),
            VKind::Atom(Kind::Path) => rgb(0.64, 0.86, 0.84),
        }
    }
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, _dt: f32) {
        let (w, h) = ctx.size();
        let page = Page::new(w, h);
        self.page = page;

        // --- input -------------------------------------------------------
        let down = ctx.mouse_button_down(frost::MouseButton::Left);
        let pressed = down && !self.was_down;
        let released = !down && self.was_down;
        let pos = ctx.mouse_position();
        self.was_down = down;

        // Q quits, on its rising edge.
        let q = ctx.key_down(frost::KeyCode::KeyQ);
        if q && !self.was_q {
            std::process::exit(0);
        }
        self.was_q = q;

        let shift =
            ctx.key_down(frost::KeyCode::ShiftLeft) || ctx.key_down(frost::KeyCode::ShiftRight);
        let wheel = ctx.mouse_wheel();
        if wheel != 0.0 {
            if shift {
                self.sx -= wheel * 4.0 * SIZE * ADVANCE_EM;
            } else {
                self.sy -= wheel * 3.0 * LINE_H;
            }
            self.clamp_scroll();
        }

        let adv = SIZE * ADVANCE_EM;
        let track_v = page.vh();
        let track_h = page.vw();
        let (ty, tly) = thumb(self.sy, page.vh(), self.content_h, track_v);
        let (tx, tlx) = thumb(self.sx, page.vw(), self.content_w, track_h);

        // The handles' rectangles, in window space: the vertical track
        // hangs from the page's top edge, the horizontal from its left.
        let ybar = [page.r + 2.0, page.b, page.r + 2.0 + BAR, page.t];
        let ythumb = [ybar[0], page.t - ty - tly, ybar[2], page.t - ty];
        let xbar = [page.l, page.b - 2.0 - BAR, page.r, page.b - 2.0];
        let xthumb = [page.l + tx, xbar[1], page.l + tx + tlx, xbar[3]];
        let in_rect =
            |r: [f32; 4], p: [f32; 2]| p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3];

        if pressed {
            self.press = None;
            if let Some(p) = pos {
                if in_rect(ythumb, p) {
                    self.grab = Grab::Y {
                        off: page.t - p[1] - ty,
                        len: tly,
                    };
                } else if in_rect(xthumb, p) {
                    self.grab = Grab::X {
                        off: p[0] - page.l - tx,
                        len: tlx,
                    };
                } else if in_rect(ybar, p) {
                    // A track click jumps a page toward the click.
                    self.sy = (self.sy + (page.t - p[1] - tly - page.vh() * 0.5)).max(0.0);
                    self.clamp_scroll();
                } else if in_rect(xbar, p) {
                    self.sx = (self.sx + (p[0] - page.l - tlx - page.vw() * 0.5)).max(0.0);
                    self.clamp_scroll();
                } else if in_rect([page.l, page.b, page.r, page.t], p) {
                    self.press = Some(p);
                }
            }
        }
        if down && self.grab != Grab::None {
            if let (Grab::Y { off, len }, Some(p)) = (self.grab, pos) {
                let travel = (track_v - len).max(f32::EPSILON);
                let at = (page.t - p[1] - off).clamp(0.0, travel);
                self.sy = at / travel * (self.content_h - page.vh()).max(0.0);
            }
            if let (Grab::X { off, len }, Some(p)) = (self.grab, pos) {
                let travel = (track_h - len).max(f32::EPSILON);
                let at = (p[0] - page.l - off).clamp(0.0, travel);
                self.sx = at / travel * (self.content_w - page.vw()).max(0.0);
            }
            self.clamp_scroll();
        }
        if released {
            self.grab = Grab::None;
            // A still click on the page toggles the row under it.
            if let (Some(from), Some(p)) = (self.press.take(), pos)
                && ((p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2)).sqrt() < 4.0
                && let Some(i) = self.row_at(p)
                && self.rows[i].fold.is_some()
            {
                let path = self.rows[i].path.clone();
                if let Some(v) = walk(&mut self.root, &path) {
                    v.toggle();
                }
                self.relayout();
            }
        }

        // The hovered row's stripe, when the cursor is over the page.
        self.hover = match pos {
            Some(p) if in_rect([page.l, page.b, page.r, page.t], p) && self.grab == Grab::None => {
                self.row_at(p)
            }
            _ => None,
        };

        // --- the page, the bars and the stripe ----------------------------
        // The page plate the rows sit on (z 0), the hover stripe (0.2),
        // the rows (1), then the chrome above: bars (2), title, status.
        ctx.rectangle(
            (page.l + page.r) / 2.0,
            (page.b + page.t) / 2.0,
            (page.r - page.l) / 2.0,
            (page.t - page.b) / 2.0,
            rgb(0.13, 0.13, 0.17),
            0.0,
        );
        if let Some(i) = self.hover
            && self.rows[i].fold.is_some()
        {
            let y = page.t - PAD - (i as f32 + 0.5) * LINE_H + self.sy;
            ctx.rectangle(
                (page.l + page.r) / 2.0,
                y,
                (page.r - page.l) / 2.0,
                LINE_H / 2.0 - 1.0,
                rgb(0.21, 0.22, 0.30),
                0.2,
            );
        }

        // --- the rows ------------------------------------------------------
        // Row i owns nodes 2i and 2i+1; the pool caps the rows shown.
        let shown = self.rows.len().min(ROWS);
        {
            let nodes = &mut ctx.scene().root.children;
            for i in 0..shown {
                let r = &self.rows[i];
                let y = page.t - PAD - (i as f32 + 0.5) * LINE_H + self.sy;
                let visible = y + LINE_H / 2.0 > page.b && y - LINE_H / 2.0 < page.t;
                let kl = r.key.chars().count() as f32;
                let kx = page.l + PAD + kl * adv / 2.0 - self.sx;
                let vx = kx + (kl + r.val.chars().count() as f32) * adv / 2.0;
                if visible {
                    nodes[i * 2].shape = (!r.key.is_empty())
                        .then(|| self.text(r.key.clone(), HEAVY, rgb(0.88, 0.88, 0.91)));
                    nodes[i * 2].transform = frost::Transform::translate([kx, y - LIFT]);
                    let heavy = matches!(r.vkind, VKind::Head);
                    nodes[i * 2 + 1].shape = Some(self.text(
                        r.val.clone(),
                        if heavy { HEAVY } else { MEDIUM },
                        Self::vcolor(r.vkind),
                    ));
                    nodes[i * 2 + 1].transform = frost::Transform::translate([vx, y - LIFT]);
                } else {
                    nodes[i * 2].shape = None;
                    nodes[i * 2 + 1].shape = None;
                }
            }
            // Retire the pool's unused nodes.
            for node in &mut nodes[shown * 2..TITLE_NODE] {
                node.shape = None;
            }
        }

        // --- the chrome ----------------------------------------------------
        ctx.rectangle(
            (ybar[0] + ybar[2]) / 2.0,
            (ybar[1] + ybar[3]) / 2.0,
            BAR / 2.0,
            (ybar[3] - ybar[1]) / 2.0,
            rgb(0.20, 0.20, 0.25),
            2.0,
        );
        let bright = matches!(self.grab, Grab::Y { .. });
        ctx.rectangle(
            (ythumb[0] + ythumb[2]) / 2.0,
            (ythumb[1] + ythumb[3]) / 2.0,
            BAR / 2.0,
            (ythumb[3] - ythumb[1]) / 2.0,
            if bright {
                rgb(0.68, 0.68, 0.80)
            } else {
                rgb(0.46, 0.46, 0.55)
            },
            2.1,
        );
        ctx.rectangle(
            (xbar[0] + xbar[2]) / 2.0,
            (xbar[1] + xbar[3]) / 2.0,
            (xbar[2] - xbar[0]) / 2.0,
            BAR / 2.0,
            rgb(0.20, 0.20, 0.25),
            2.0,
        );
        let bright = matches!(self.grab, Grab::X { .. });
        ctx.rectangle(
            (xthumb[0] + xthumb[2]) / 2.0,
            (xthumb[1] + xthumb[3]) / 2.0,
            (xthumb[2] - xthumb[0]) / 2.0,
            BAR / 2.0,
            if bright {
                rgb(0.68, 0.68, 0.80)
            } else {
                rgb(0.46, 0.46, 0.55)
            },
            2.1,
        );

        // Text shapes center on their node, so each line is placed at
        // its own half-width.
        let title = format!("ron_view — {}", self.name);
        let status = format!(
            "{} rows · click [-]/[+] to fold · drag the handles, or the wheel (shift = sideways) · q quits",
            self.rows.len()
        );
        let nodes = &mut ctx.scene().root.children;
        nodes[TITLE_NODE].shape = Some(self.text(title.clone(), HEAVY, rgb(0.92, 0.92, 0.95)));
        nodes[TITLE_NODE].transform = frost::Transform::translate([
            page.l + PAD + title.chars().count() as f32 * adv / 2.0,
            h / 2.0 - 20.0,
        ]);
        nodes[STATUS_NODE].shape = Some(self.text(status.clone(), MEDIUM, rgb(0.60, 0.60, 0.66)));
        nodes[STATUS_NODE].transform = frost::Transform::translate([
            status.chars().count() as f32 * adv / 2.0 * -0.6,
            -h / 2.0 + 14.0,
        ]);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    let root_dir = std::env!("CARGO_MANIFEST_DIR");
    // -i picks the file; the garden ships as the default.
    let mut args = std::env::args();
    let mut path = format!("{root_dir}/assets/ron/garden.ron");
    while let Some(a) = args.next() {
        if a == "-i"
            && let Some(p) = args.next()
        {
            path = if std::path::Path::new(&p).is_relative() {
                format!("{root_dir}/{p}")
            } else {
                p
            };
        }
    }
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(err) => {
            log::error!("cannot read {path}: {err}");
            std::process::exit(1);
        }
    };
    let mut root = match parse(&src) {
        Ok(v) => v,
        Err(err) => {
            log::error!("cannot parse {path}: {err}");
            std::process::exit(1);
        }
    };
    root.open_to(0, 1);
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(path.clone());

    // The font, read once: every row clones the Arc, never the bytes.
    let font_bytes = match std::fs::read(format!(
        "{root_dir}/assets/fonts/FiraCode-VariableFont_wght.ttf"
    )) {
        Ok(b) => b,
        Err(err) => {
            log::error!("cannot read the font: {err}");
            std::process::exit(1);
        }
    };
    // Validate once through the engine's own door, then keep the Arc.
    if let Err(err) = frost::Shape::text_bytes(&font_bytes, "0", SIZE) {
        log::error!("not a usable font: {err}");
        std::process::exit(1);
    }
    let font: Arc<[u8]> = Arc::from(font_bytes);

    let mut demo = Demo {
        root,
        font,
        rows: Vec::new(),
        content_w: 0.0,
        content_h: 0.0,
        sx: 0.0,
        sy: 0.0,
        grab: Grab::None,
        was_down: false,
        press: None,
        hover: None,
        page: Page {
            l: -400.0,
            r: 400.0,
            t: 270.0,
            b: -270.0,
        },
        name,
        was_q: false,
    };
    demo.relayout();

    let children: Vec<Box<frost::SceneNode>> = (0..NODES)
        .map(|_| {
            Box::new(frost::SceneNode {
                order: 1.0,
                ..Default::default()
            })
        })
        .collect();

    if let Err(err) = frost::run_configured(
        frost::Scene::new(frost::SceneNode {
            shape: Some(frost::Shape::Background {
                color: rgb(0.09, 0.09, 0.11),
            }),
            children,
            ..Default::default()
        }),
        demo,
        frost::Config {
            window_size: Some([900, 640]),
            ..Default::default()
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thumb_maps_the_scroll_to_the_track() {
        // Half the content shown: a half-length thumb at half travel.
        let (start, len) = thumb(50.0, 100.0, 200.0, 100.0);
        assert!((len - 50.0).abs() < 1e-4);
        assert!((start - 25.0).abs() < 1e-4);
        // Content that fits: a full thumb at the top.
        let (start, len) = thumb(0.0, 100.0, 90.0, 100.0);
        assert_eq!((start, len), (0.0, 100.0));
        // Both extremes pin inside the track.
        let (s0, _) = thumb(0.0, 10.0, 1000.0, 100.0);
        let (s1, l1) = thumb(990.0, 10.0, 1000.0, 100.0);
        assert_eq!(s0, 0.0);
        // The thumb's 24-pixel minimum shortens the travel; the far
        // extreme pins the thumb to the track's end.
        assert!((l1 - 24.0).abs() < 1e-3, "{l1}");
        assert!((s1 - 76.0).abs() < 1e-3, "{s1}");
    }
}
