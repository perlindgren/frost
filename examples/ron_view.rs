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
//! The parser is a hand-rolled RON subset, deliberately dependency-free
//! like every other example here: line and (nestable) block comments,
//! `!` type markers, `Name(...)` and `Name {...}` structs, named and
//! positional fields, `(...)` anonymous structs, arrays, maps with
//! string-ish keys, strings and chars (kept verbatim, escapes and all),
//! integers, floats, exponents, underscores, bools, and enum paths —
//! unit (`Kind::Bee`), newtype (`Kind::Bug(Count(3))`) and struct
//! variants (`Watering::PingPong { .. }`) — with trailing commas
//! allowed everywhere. It errors with a line number rather than
//! guessing.
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

use std::sync::Arc;

// ---------------------------------------------------------------------------
// The parsed model

/// What an atom's text is, for coloring and weighting it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Bool,
    Num,
    Str,
    Char,
    /// A path-like atom: an enum unit variant, a bare identifier.
    Path,
}

/// A parsed RON value. Containers carry their fold state: the view
/// mutates these flags in place, and flattens to rows from them.
#[derive(Debug)]
enum Val {
    /// Text, verbatim from the source (quotes and escapes included).
    Atom(String, Kind),
    /// `[..]`.
    Seq { open: bool, items: Vec<Val> },
    /// `Name(..)`, `(..)`, `Name {..}` or `{..}` with named fields;
    /// field names are empty for positional (tuple) entries.
    Struct {
        open: bool,
        head: String,
        curly: bool,
        fields: Vec<(String, Val)>,
    },
    /// `{ key: value, .. }` — keys rendered to display strings.
    Map {
        open: bool,
        entries: Vec<(String, Val)>,
    },
}

impl Val {
    /// The value's one-line text: atoms verbatim, containers as a
    /// bracketed ellipsis — enough for map keys, which the view shows
    /// flat.
    fn display(&self) -> String {
        match self {
            Val::Atom(s, _) => s.clone(),
            Val::Seq { .. } => "[…]".into(),
            Val::Struct { head, curly, .. } => {
                format!(
                    "{head}{}…{}",
                    if *curly { " {" } else { "(" },
                    if *curly { "}" } else { ")" }
                )
            }
            Val::Map { .. } => "{…}".into(),
        }
    }

    /// The nth child value, whatever the container kind.
    fn child_at(&mut self, n: usize) -> Option<&mut Val> {
        match self {
            Val::Atom(..) => None,
            Val::Seq { items, .. } => items.get_mut(n),
            Val::Struct { fields, .. } => fields.get_mut(n).map(|(_, v)| v),
            Val::Map { entries, .. } => entries.get_mut(n).map(|(_, v)| v),
        }
    }

    /// Open a container or close it; atoms ignore the request.
    fn toggle(&mut self) {
        match self {
            Val::Seq { open, .. } | Val::Struct { open, .. } | Val::Map { open, .. } => {
                *open = !*open
            }
            Val::Atom(..) => {}
        }
    }

    /// Open every container down to depth `max`, fold what is deeper —
    /// the view's initial spread.
    fn open_to(&mut self, depth: usize, max: usize) {
        let children: Vec<&mut Val> = match self {
            Val::Atom(..) => return,
            Val::Seq { open, items, .. } => {
                *open = depth <= max;
                items.iter_mut().collect()
            }
            Val::Struct { open, fields, .. } => {
                *open = depth <= max;
                fields.iter_mut().map(|(_, v)| v).collect()
            }
            Val::Map { open, entries, .. } => {
                *open = depth <= max;
                entries.iter_mut().map(|(_, v)| v).collect()
            }
        };
        for c in children {
            c.open_to(depth + 1, max);
        }
    }
}

/// Follow a fold-path of child indices from the root.
fn walk<'v>(root: &'v mut Val, path: &[u16]) -> Option<&'v mut Val> {
    let mut v = root;
    for &i in path {
        v = v.child_at(i as usize)?;
    }
    Some(v)
}

// ---------------------------------------------------------------------------
// The parser: a pragmatic RON subset

/// A recursive-descent reader over the source bytes.
struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

/// How deep the parser nests before it cries stack overflow.
const MAX_DEPTH: usize = 96;

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            s: src.as_bytes(),
            i: 0,
        }
    }

    /// The 1-based line of the cursor, for error messages.
    fn line(&self) -> usize {
        1 + self.s[..self.i].iter().filter(|&&b| b == b'\n').count()
    }

    fn err<T>(&self, m: &str) -> Result<T, String> {
        Err(format!("line {}: {m}", self.line()))
    }

    /// Skip whitespace, `//` lines and nestable `/* */` blocks.
    fn ws(&mut self) -> Result<(), String> {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"//") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
            } else if self.s[self.i..].starts_with(b"/*") {
                let mut depth = 0;
                while self.i < self.s.len() {
                    if self.s[self.i..].starts_with(b"/*") {
                        depth += 1;
                        self.i += 2;
                    } else if self.s[self.i..].starts_with(b"*/") {
                        depth -= 1;
                        self.i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        self.i += 1;
                    }
                }
                if depth > 0 {
                    return self.err("unterminated block comment");
                }
            } else {
                return Ok(());
            }
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    /// A `path::like::Ident`, or `None` (cursor unmoved) if one starts
    /// here.
    fn ident(&mut self) -> Option<String> {
        let start = self.i;
        let first = self.peek()?;
        if !(first.is_ascii_alphabetic() || first == b'_') {
            return None;
        }
        self.i += 1;
        loop {
            while let Some(b) = self.peek() {
                if b.is_ascii_alphanumeric() || b == b'_' {
                    self.i += 1;
                } else {
                    break;
                }
            }
            // A `::` continues the path only when an identifier follows.
            if self.s[self.i..].starts_with(b"::")
                && self
                    .s
                    .get(self.i + 2)
                    .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
            {
                self.i += 2;
            } else {
                break;
            }
        }
        Some(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }

    /// A number: sign, digits and `_`, a fraction, an exponent, and a
    /// type suffix — kept verbatim.
    fn number(&mut self) -> Option<String> {
        let start = self.i;
        self.eat(b'-');
        self.eat(b'+');
        let mut seen = false;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() || b == b'_' || b == b'.' {
                self.i += 1;
                seen = true;
            } else if (b == b'e' || b == b'E') && seen {
                self.i += 1;
                self.eat(b'-');
                self.eat(b'+');
            } else if b.is_ascii_alphabetic() && seen {
                // A type suffix (f32, u64, ...): swallow the run.
                self.i += 1;
            } else {
                break;
            }
        }
        if seen {
            Some(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
        } else {
            self.i = start;
            None
        }
    }

    /// A `"string"` or `'c'`, captured verbatim — quotes included,
    /// escapes untouched.
    fn quoted(&mut self, q: u8) -> Result<String, String> {
        let start = self.i;
        self.i += 1; // the opening quote
        loop {
            let Some(b) = self.peek() else {
                return self.err("unterminated string");
            };
            self.i += 1;
            match b {
                b'\\' => {
                    self.i += 1;
                }
                _ if b == q => break,
                b'\n' => return self.err("unterminated string"),
                _ => {}
            }
        }
        Ok(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }

    /// One value at nesting `depth`.
    fn value(&mut self, depth: usize) -> Result<Val, String> {
        if depth > MAX_DEPTH {
            return self.err("nested too deeply");
        }
        self.ws()?;
        // RON's `!` type marker: `!Type(value)` and `!(...)` show the
        // value; the marker is the viewer's blind spot by design.
        if self.eat(b'!') {
            self.ident();
            return self.value(depth);
        }
        match self.peek() {
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws()?;
                    if self.eat(b']') {
                        break;
                    }
                    items.push(self.value(depth + 1)?);
                    self.ws()?;
                    if self.eat(b']') {
                        break;
                    }
                    if !self.eat(b',') {
                        return self.err("expected ',' or ']'");
                    }
                }
                Ok(Val::Seq { open: false, items })
            }
            Some(b'(') => {
                self.i += 1;
                let fields = self.fields(b')', depth)?;
                Ok(Val::Struct {
                    open: false,
                    head: String::new(),
                    curly: false,
                    fields,
                })
            }
            Some(b'{') => {
                self.i += 1;
                let mut entries = Vec::new();
                loop {
                    self.ws()?;
                    if self.eat(b'}') {
                        break;
                    }
                    let key = self.value(depth + 1)?;
                    self.ws()?;
                    if !self.eat(b':') {
                        return self.err("expected ':' after a map key");
                    }
                    let v = self.value(depth + 1)?;
                    entries.push((key.display(), v));
                    self.ws()?;
                    if self.eat(b'}') {
                        break;
                    }
                    if !self.eat(b',') {
                        return self.err("expected ',' or '}'");
                    }
                }
                Ok(Val::Map {
                    open: false,
                    entries,
                })
            }
            Some(b'"') => {
                let s = self.quoted(b'"')?;
                Ok(Val::Atom(s, Kind::Str))
            }
            Some(b'\'') => {
                let s = self.quoted(b'\'')?;
                Ok(Val::Atom(s, Kind::Char))
            }
            Some(b) if b == b'-' || b == b'+' || b.is_ascii_digit() => {
                let Some(n) = self.number() else {
                    return self.err("bad number");
                };
                Ok(Val::Atom(n, Kind::Num))
            }
            _ => {
                let Some(p) = self.ident() else {
                    return self.err("expected a value");
                };
                self.ws()?;
                if p == "true" || p == "false" {
                    return Ok(Val::Atom(p, Kind::Bool));
                }
                if self.eat(b'(') {
                    let fields = self.fields(b')', depth)?;
                    return Ok(Val::Struct {
                        open: false,
                        head: p,
                        curly: false,
                        fields,
                    });
                }
                if self.eat(b'{') {
                    let fields = self.fields(b'}', depth)?;
                    return Ok(Val::Struct {
                        open: false,
                        head: p,
                        curly: true,
                        fields,
                    });
                }
                Ok(Val::Atom(p, Kind::Path))
            }
        }
    }

    /// The comma-run of entries until `close`: named (`k: v`) or
    /// positional (`v`), trailing comma allowed.
    fn fields(&mut self, close: u8, depth: usize) -> Result<Vec<(String, Val)>, String> {
        let mut fields = Vec::new();
        loop {
            self.ws()?;
            if self.eat(close) {
                break;
            }
            // Named entry? An ident followed by a lone `:` — `::` is a
            // path, not a colon.
            let save = self.i;
            let named = match self.ident() {
                Some(p) => {
                    self.ws()?;
                    if self.peek() == Some(b':') && self.s.get(self.i + 1) != Some(&b':') {
                        self.i += 1;
                        Some(p)
                    } else {
                        self.i = save;
                        None
                    }
                }
                None => {
                    self.i = save;
                    None
                }
            };
            let value = self.value(depth + 1)?;
            fields.push((named.unwrap_or_default(), value));
            self.ws()?;
            if self.eat(b',') {
                continue;
            }
            if self.eat(close) {
                break;
            }
            return self.err("expected ',' or a closing bracket");
        }
        Ok(fields)
    }
}

/// Parse a RON document into the view's tree.
fn parse(src: &str) -> Result<Val, String> {
    let mut p = Parser::new(src);
    let v = p.value(0)?;
    p.ws()?;
    if p.i < p.s.len() {
        return p.err("unexpected text after the document");
    }
    Ok(v)
}

// ---------------------------------------------------------------------------
// Rows: the flattened, folded view of the tree

/// What the value side of a row is, for its color and weight.
#[derive(Clone, Copy)]
enum VKind {
    /// A container head or its folded `…` form: heavy, amber.
    Head,
    /// A closing bracket: light.
    Close,
    /// An atom, by kind.
    Atom(Kind),
}

/// One visible row: a heavy prefix and a value side, and — for
/// containers and their closing brackets — the fold path they toggle.
struct Row {
    /// The fold-path from the root: child indices.
    path: Vec<u16>,
    /// The heavy prefix: indent, `[+]/[-]` marker, key and colon.
    key: String,
    /// The value side.
    val: String,
    vkind: VKind,
    /// `Some` for the rows a click folds: the flag it would flip.
    fold: Option<bool>,
}

/// The fold-path index of the nth child row of a container. Paths are
/// the child lists of `Val`, shared by `child_at` and the flattener.
fn flatten(v: &Val, path: &mut Vec<u16>, depth: u16, key: &str, rows: &mut Vec<Row>) {
    let ind = "    ".repeat(depth as usize);
    match v {
        Val::Atom(s, k) => rows.push(Row {
            path: path.clone(),
            key: format!("{ind}{key}"),
            val: s.clone(),
            vkind: VKind::Atom(*k),
            fold: None,
        }),
        _ => {
            let (open, head, o, c) = match v {
                Val::Atom(..) => unreachable!(),
                Val::Seq { open, .. } => (*open, String::new(), '[', ']'),
                Val::Struct {
                    open, head, curly, ..
                } => (
                    *open,
                    head.clone(),
                    if *curly { '{' } else { '(' },
                    if *curly { '}' } else { ')' },
                ),
                Val::Map { open, .. } => (*open, String::new(), '{', '}'),
            };
            let marker = if open { "[-]" } else { "[+]" };
            rows.push(Row {
                path: path.clone(),
                key: format!("{ind}{marker} {key}"),
                val: if open {
                    format!("{head}{o}")
                } else {
                    format!("{head}{o}…{c}")
                },
                vkind: VKind::Head,
                fold: Some(open),
            });
            if !open {
                return;
            }
            // The children, then the closing bracket row at the
            // container's own depth.
            let kids: Vec<(String, &Val)> = match v {
                Val::Seq { items, .. } => items
                    .iter()
                    .enumerate()
                    .map(|(n, v)| (format!("{n}: "), v))
                    .collect(),
                Val::Struct { fields, .. } => fields
                    .iter()
                    .map(|(k, v)| {
                        (
                            if k.is_empty() {
                                String::new()
                            } else {
                                format!("{k}: ")
                            },
                            v,
                        )
                    })
                    .collect(),
                Val::Map { entries, .. } => {
                    entries.iter().map(|(k, v)| (format!("{k}: "), v)).collect()
                }
                Val::Atom(..) => Vec::new(),
            };
            for (n, (k, child)) in kids.iter().enumerate() {
                path.push(n as u16);
                flatten(child, path, depth + 1, k, rows);
                path.pop();
            }
            rows.push(Row {
                path: path.clone(),
                key: ind.clone(),
                val: c.to_string(),
                vkind: VKind::Close,
                fold: Some(open),
            });
        }
    }
}

/// Rebuild the row list from the tree.
fn layout(root: &Val) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut path = Vec::new();
    flatten(root, &mut path, 0, "", &mut rows);
    rows
}

// ---------------------------------------------------------------------------
// The view

/// One font pixel's advance: FiraCode is monospaced at exactly 600/1000
/// em, so a row's width is its character count times this — and weight
/// never changes it, which keeps the two weights perfectly aligned.
const ADVANCE_EM: f32 = 0.6;

/// The text size, in pixels per em.
const SIZE: f32 = 18.0;

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
                    nodes[i * 2].transform = frost::Transform::translate([kx, y]);
                    let heavy = matches!(r.vkind, VKind::Head);
                    nodes[i * 2 + 1].shape = Some(self.text(
                        r.val.clone(),
                        if heavy { HEAVY } else { MEDIUM },
                        Self::vcolor(r.vkind),
                    ));
                    nodes[i * 2 + 1].transform = frost::Transform::translate([vx, y]);
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

    /// The shipped sample, embedded so the parser is tested against its
    /// real target.
    const GARDEN: &str = include_str!("../assets/ron/garden.ron");

    #[test]
    fn the_garden_parses_with_every_flavour() {
        let root = parse(GARDEN).expect("the sample must parse");
        let Val::Struct { head, fields, .. } = &root else {
            panic!("the root is an anonymous struct");
        };
        assert!(head.is_empty());
        let names: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["gardener", "bench", "watering", "chores"]);
    }

    #[test]
    fn enums_arrive_as_paths_heads_and_variants() {
        let root = parse(GARDEN).unwrap();
        // A unit enum is a path atom; a newtype enum is a one-field
        // struct named by its path.
        let Val::Struct { fields, .. } = &root else {
            bug()
        };
        let chores = &fields[3].1;
        let Val::Map { entries, .. } = chores else {
            bug()
        };
        assert_eq!(entries[0].0, "\"mon\"");
        let Val::Struct { head, fields, .. } = &entries[0].1 else {
            bug()
        };
        assert_eq!(head, "Chores");
        let weed = &fields[1].1;
        let Val::Struct { head, fields, .. } = weed else {
            bug()
        };
        assert_eq!(head, "Kind::Bug");
        assert_eq!(fields.len(), 1);
        let Val::Struct { head, .. } = &fields[0].1 else {
            bug()
        };
        assert_eq!(head, "Count");
        // And a struct variant with curly fields.
        let Val::Struct { fields, .. } = &root else {
            bug()
        };
        let Val::Struct {
            head,
            curly,
            fields,
            ..
        } = &fields[2].1
        else {
            bug()
        };
        assert_eq!(head, "Watering::PingPong");
        assert!(curly);
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn atoms_keep_their_text_and_kind() {
        let v = parse(
            "!(name: \"Per \\\"Q\\\"\", n: -1_000, f: 6.5e-2, t: true, u: Kind::Bee, c: 'x')",
        )
        .unwrap();
        let Val::Struct { fields, .. } = &v else {
            bug()
        };
        let atom = |i: usize| match &fields[i].1 {
            Val::Atom(s, k) => (s.clone(), *k),
            _ => bug(),
        };
        assert_eq!(atom(0), ("\"Per \\\"Q\\\"\"".into(), Kind::Str));
        assert_eq!(atom(1), ("-1_000".into(), Kind::Num));
        assert_eq!(atom(2), ("6.5e-2".into(), Kind::Num));
        assert_eq!(atom(3), ("true".into(), Kind::Bool));
        assert_eq!(atom(4), ("Kind::Bee".into(), Kind::Path));
        assert_eq!(atom(5), ("'x'".into(), Kind::Char));
    }

    #[test]
    fn comments_and_trailing_commas_are_noise() {
        let v = parse("/* a /* nested */ block */ ( a: 1, /* inline */ b: [1, 2,], ) // trailing")
            .unwrap();
        let Val::Struct { fields, .. } = &v else {
            bug()
        };
        assert_eq!(fields.len(), 2);
        let Val::Seq { items, .. } = &fields[1].1 else {
            bug()
        };
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn errors_carry_a_line() {
        let err = parse("(a: 1,\nb: [1,,])\n").unwrap_err();
        assert!(err.starts_with("line 2:"), "{err}");
        let err = parse("(a: 1) junk").unwrap_err();
        assert!(err.starts_with("line 1:"), "{err}");
        assert!(parse("/* open").is_err());
    }

    #[test]
    fn folding_hides_the_children_and_the_close_row() {
        let mut root = parse("(a: Foo(b: 1), c: 2)").unwrap();
        root.open_to(0, 1);
        let rows = layout(&root);
        // root, Foo( open, its b row, Foo's ')' row, c: 2, and the
        // root's own ')' row — six.
        assert_eq!(rows.len(), 6);
        // Fold the Foo container by its path [0]: its row stays as an
        // ellipsis, its b and ')' rows vanish.
        walk(&mut root, &[0]).unwrap().toggle();
        let folded = layout(&root);
        assert_eq!(folded.len(), 4); // root, Foo(…), c: 2, ')'
        assert!(folded[1].val.contains('…'));
        // And unfolding brings them back.
        walk(&mut root, &[0]).unwrap().toggle();
        assert_eq!(layout(&root).len(), 6);
    }

    #[test]
    fn a_folded_container_shows_its_ellipsis_and_marker() {
        // open_to(0, 0) opens the root and folds everything deeper.
        let mut root = parse("(a: [1, 2])").unwrap();
        root.open_to(0, 0);
        let rows = layout(&root);
        assert_eq!(rows.len(), 3); // root, a: […], ')'
        assert_eq!(rows[0].key, "[-] ");
        assert_eq!(rows[0].val, "(");
        assert_eq!(rows[1].key, "    [+] a: ");
        assert_eq!(rows[1].val, "[…]");
        assert_eq!(rows[2].val, ")");
    }

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

    /// Every test's panic door when the tree is not the expected shape.
    fn bug() -> ! {
        panic!("unexpected shape");
    }
}
