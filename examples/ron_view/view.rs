//! The row view: what a tree of `frost::ron` values looks like when a
//! viewer draws it. The data side — parser, tree model, writer — lives
//! in the crate as `frost::ron`; this file keeps only the flattener
//! that turns a tree into visible rows ([`layout`], [`Row`]) and the
//! kinds that tell a renderer how to dress each row ([`VKind`]). The
//! fold state it reads and toggles is the tree's own (`Val::toggle`,
//! `Val::open_to`); the fold paths the rows carry are this view's
//! addresses into the tree. Shared by `ron_view` (its whole view) and,
//! through a `#[path]` include, `sprite_util`'s sidecar panels — every
//! renderer colors and weights the rows as it likes.

use frost::ron::{Kind, Val};

/// What the value side of a row is, for its color and weight.
#[derive(Clone, Copy)]
pub enum VKind {
    /// A container head or its folded `…` form: heavy, amber.
    Head,
    /// A closing bracket: dressed like its opening one — the view
    /// paints it amber and bold, the Head's own colors.
    Close,
    /// An atom, by kind.
    Atom(Kind),
}

/// One visible row: a heavy prefix and a value side, and — for
/// containers and their closing brackets — the fold path they toggle.
#[derive(Clone)]
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
                    v.preview()
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
                    .map(|(n, it)| (format!("{n}: "), &it.val))
                    .collect(),
                Val::Struct { fields, .. } => fields
                    .iter()
                    .map(|(k, it)| {
                        (
                            if k.is_empty() {
                                String::new()
                            } else {
                                format!("{k}: ")
                            },
                            &it.val,
                        )
                    })
                    .collect(),
                Val::Map { entries, .. } => entries
                    .iter()
                    .map(|(k, it)| (format!("{k}: "), &it.val))
                    .collect(),
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

#[cfg(test)]
mod tests {
    use super::layout;
    use frost::ron::{parse, walk};

    #[test]
    fn folding_hides_the_children_and_the_close_row() {
        let mut root = parse("(a: Foo(b: 1), c: 2)").unwrap();
        root.open_to(0, 1);
        let rows = layout(&root);
        // root, Foo( open, its b row, Foo's ')' row, c: 2, and the
        // root's own ')' row — six.
        assert_eq!(rows.len(), 6);
        // Fold the Foo container by its path [0]: its row keeps the
        // one-line preview, its b and ')' rows vanish.
        walk(&mut root, &[0]).unwrap().toggle();
        let folded = layout(&root);
        assert_eq!(folded.len(), 4); // root, Foo(b: 1), c: 2, ')'
        assert_eq!(folded[1].val, "Foo(b: 1)");
        // And unfolding brings them back.
        walk(&mut root, &[0]).unwrap().toggle();
        assert_eq!(layout(&root).len(), 6);
    }

    #[test]
    fn a_folded_container_previews_its_whole_line() {
        // open_to(0, 0) opens the root and folds everything deeper.
        let mut root = parse("(a: [1, 2])").unwrap();
        root.open_to(0, 0);
        let rows = layout(&root);
        assert_eq!(rows.len(), 3); // root, a: [1, 2], ')'
        assert_eq!(rows[0].key, "[-] ");
        assert_eq!(rows[0].val, "(");
        assert_eq!(rows[1].key, "    [+] a: ");
        assert_eq!(rows[1].val, "[1, 2]");
        // Too long to preview inline — the bracketed ellipsis returns.
        let mut big = parse("(a: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13])").unwrap();
        big.open_to(0, 0);
        assert_eq!(layout(&big)[1].val, "[…]");
        assert_eq!(rows[2].val, ")");
    }
}
