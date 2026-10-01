//! The RON-subset parser and tree model, shared by the `ron_view` viewer
//! (its whole view) and `sprite_util` (sidecar panels for sprites).
//!
//! The parser is deliberately dependency-free like every other example
//! here: line and (nestable) block comments, `!` type markers, `Name(..)`
//! and `Name {..}` structs, named and positional fields, `(...)` anonymous
//! structs, arrays, maps with string-ish keys, strings and chars (kept
//! verbatim, escapes and all), integers, floats, exponents, underscores,
//! bools, and enum paths — unit (`Kind::Bee`), newtype (`Kind::Bug(Count(3))`)
//! and struct variants (`Watering::PingPong { .. }`) — with trailing
//! commas allowed everywhere. It errors with a line number rather than
//! guessing.
//!
//! Containers carry their fold state in the tree (`Val::open_to`,
//! `Val::toggle`); [`layout`] flattens the visible rows, each carrying the
//! fold-path it toggles. Pure std — every renderer colors and weights the
//! rows (`VKind`) as it likes.

/// What an atom's text is, for coloring and weighting it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Bool,
    Num,
    Str,
    Char,
    /// A path-like atom: an enum unit variant, a bare identifier.
    Path,
}

/// A parsed RON value. Containers carry their fold state: the view
/// mutates these flags in place, and flattens to rows from them.
#[derive(Debug, PartialEq)]
pub enum Val {
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
    pub fn display(&self) -> String {
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

    /// The children with their display keys — field names (empty for
    /// positional tuple entries) and map keys; sequence items carry no
    /// key. The read-only counterpart of [`Val::child_at`].
    pub fn kids(&self) -> Vec<(String, &Val)> {
        match self {
            Val::Atom(..) => Vec::new(),
            Val::Seq { items, .. } => items.iter().map(|v| (String::new(), v)).collect(),
            Val::Struct { fields, .. } => fields.iter().map(|(k, v)| (k.clone(), v)).collect(),
            Val::Map { entries, .. } => entries.iter().map(|(k, v)| (k.clone(), v)).collect(),
        }
    }

    /// The nth child value, whatever the container kind.
    pub fn child_at(&mut self, n: usize) -> Option<&mut Val> {
        match self {
            Val::Atom(..) => None,
            Val::Seq { items, .. } => items.get_mut(n),
            Val::Struct { fields, .. } => fields.get_mut(n).map(|(_, v)| v),
            Val::Map { entries, .. } => entries.get_mut(n).map(|(_, v)| v),
        }
    }

    /// Open a container or close it; atoms ignore the request.
    pub fn toggle(&mut self) {
        match self {
            Val::Seq { open, .. } | Val::Struct { open, .. } | Val::Map { open, .. } => {
                *open = !*open
            }
            Val::Atom(..) => {}
        }
    }

    /// Open every container down to depth `max`, fold what is deeper —
    /// the view's initial spread.
    pub fn open_to(&mut self, depth: usize, max: usize) {
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
pub fn walk<'v>(root: &'v mut Val, path: &[u16]) -> Option<&'v mut Val> {
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
pub fn parse(src: &str) -> Result<Val, String> {
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
pub enum VKind {
    /// A container head or its folded `…` form: heavy, amber.
    Head,
    /// A closing bracket: light.
    Close,
    /// An atom, by kind.
    Atom(Kind),
}

/// One visible row: a heavy prefix and a value side, and — for
/// containers and their closing brackets — the fold path they toggle.
pub struct Row {
    /// The fold-path from the root: child indices.
    pub path: Vec<u16>,
    /// The heavy prefix: indent, `[+]/[-]` marker, key and colon.
    pub key: String,
    /// The value side.
    pub val: String,
    pub vkind: VKind,
    /// `Some` for the rows a click folds: the flag it would flip.
    pub fold: Option<bool>,
}

/// The fold-path index of the nth child row of a container. Paths are
/// the child lists of `Val`, shared by `child_at` and the flattener.
pub fn flatten(v: &Val, path: &mut Vec<u16>, depth: u16, key: &str, rows: &mut Vec<Row>) {
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
pub fn layout(root: &Val) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut path = Vec::new();
    flatten(root, &mut path, 0, "", &mut rows);
    rows
}

// ---------------------------------------------------------------------------
// The writer: the parser's counterpart, for anything that saves

/// Full RON text for a value — [`to_text`'s] counterpart to [`parse`].
/// The layout is canonical: one element per line whenever a container
/// has children, trailing commas everywhere. Comments, the original
/// line breaks and the fold state do not survive; re-parsing the text
/// yields the same value.
pub fn to_text(root: &Val) -> String {
    let mut out = String::new();
    write_val(root, 0, &mut out);
    out.push('\n');
    out
}

/// One value at `depth`, into `out`.
fn write_val(v: &Val, depth: usize, out: &mut String) {
    let (head, o, c, kids): (String, char, char, Vec<String>) = match v {
        Val::Atom(s, _) => {
            out.push_str(s);
            return;
        }
        Val::Seq { items, .. } => (
            String::new(),
            '[',
            ']',
            items
                .iter()
                .map(|v| {
                    let mut t = String::new();
                    write_val(v, depth + 1, &mut t);
                    t
                })
                .collect(),
        ),
        Val::Struct {
            head,
            curly,
            fields,
            ..
        } => (
            head.clone(),
            if *curly { '{' } else { '(' },
            if *curly { '}' } else { ')' },
            fields
                .iter()
                .map(|(k, v)| {
                    let mut t = String::new();
                    if !k.is_empty() {
                        t.push_str(k);
                        t.push_str(": ");
                    }
                    write_val(v, depth + 1, &mut t);
                    t
                })
                .collect(),
        ),
        Val::Map { entries, .. } => (
            String::new(),
            '{',
            '}',
            entries
                .iter()
                .map(|(k, v)| {
                    let mut t = String::new();
                    t.push_str(k);
                    t.push_str(": ");
                    write_val(v, depth + 1, &mut t);
                    t
                })
                .collect(),
        ),
    };
    out.push_str(&head);
    if kids.is_empty() {
        out.push(o);
        out.push(c);
        return;
    }
    out.push(o);
    out.push('\n');
    let pad = "    ".repeat(depth + 1);
    for kid in &kids {
        out.push_str(&pad);
        out.push_str(kid);
        out.push_str(",\n");
    }
    out.push_str(&"    ".repeat(depth));
    out.push(c);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped sample, embedded so the parser is tested against its
    /// real target.
    const GARDEN: &str = include_str!("../../assets/ron/garden.ron");

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

    /// Every test's panic door when the tree is not the expected shape.
    fn bug() -> ! {
        panic!("unexpected shape");
    }

    #[test]
    fn to_text_round_trips_the_garden() {
        let one = parse(GARDEN).expect("the garden parses");
        let text = to_text(&one);
        let two = parse(&text).unwrap_or_else(|err| panic!("re-parse failed: {err}\n{text}"));
        assert_eq!(one, two);
    }
}
