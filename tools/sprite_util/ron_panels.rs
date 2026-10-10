//! The sidecar's foldable tree panels: one `Ui` per open file,
//! their layout, rows, and list edits.
// The subjects this one reads.
use crate::desk::*;
use crate::ron_view;
use crate::sidecar::*;
use crate::spots::*;
use crate::theme::*;
use crate::world::*;
use frost::ron as ron_tree;

/// The scene's layer nodes — and the cap on layers per animation frame.
/// The pool shows the animation's current frame, or the active sprite
/// alone when there are no frames.
/// The sidecar panel: text size and row pitch, panel width, body
/// viewport height, the viewport's top padding, the rows' text inset,
/// and the row pool's capacity. The panel itself is a `Ui` panel — its
/// title bar drags it around and folds it — and its body is one tall
/// [`frost::Ui::space`] viewport these constants lay the tree out in.
pub(crate) const RON_SIZE: f32 = 16.0;

/// The amount every panel text origin drops below the y it should sit
/// at, as a fraction of its size. This FiraCode reports degenerate
/// vertical metrics to the shaper — under a quarter pixel of ascent
/// plus descent at row size, exactly what `frost`'s diagnostics note
/// about it — so the renderer's text centring collapses onto the
/// baseline: text placed at a row's centre prints its ink a cap's
/// worth ABOVE that centre, visibly riding its highlight band. Cap
/// height is 1374 units per 2000 em; half of it centres the ink.
pub(crate) const RON_LIFT: f32 = 0.344;

pub(crate) const RON_LINE: f32 = 20.0;

pub(crate) const RON_W: f32 = 300.0;

pub(crate) const RON_VIEW_H: f32 = 240.0;

pub(crate) const RON_PAD: f32 = 6.0;

pub(crate) const RON_INSET: f32 = 12.0;

/// FiraCode's advance in pixels: the horizontal scroll slices rows by
/// characters, so a view's width in characters is its own arithmetic.
pub(crate) const RON_ADV: f32 = RON_SIZE * ADVANCE_EM;

/// FiraCode's advance: exactly 600/1000 em, identical at every weight —
/// so row widths are pure character arithmetic.
pub(crate) const ADVANCE_EM: f32 = 0.6;

/// The panel's value-side color: container heads amber, closing
/// brackets muted, atoms by kind — `ron_view`'s palette on the dark
/// plate.
pub(crate) fn ron_color(k: ron_view::VKind) -> frost::Color {
    let c = |r, g, b| frost::Color { r, g, b, a: 1.0 };
    match k {
        // The closing bracket is its opening one's twin: same amber,
        // and bold like any head (see the line's `head` flag).
        ron_view::VKind::Head | ron_view::VKind::Close => c(0.93, 0.78, 0.44),
        ron_view::VKind::Atom(ron_tree::Kind::Str) => c(0.72, 0.86, 0.66),
        ron_view::VKind::Atom(ron_tree::Kind::Char) => c(0.66, 0.80, 0.78),
        ron_view::VKind::Atom(ron_tree::Kind::Num) => c(0.60, 0.78, 0.96),
        ron_view::VKind::Atom(ron_tree::Kind::Bool) => c(0.83, 0.68, 0.95),
        ron_view::VKind::Atom(ron_tree::Kind::Path) => c(0.64, 0.86, 0.84),
    }
}

/// Open every container along a fold-path, so a row found on the sprite
/// can actually show itself in the tree.
pub(crate) fn unfold_path(root: &mut ron_tree::Val, path: &[u16]) {
    for k in 0..path.len() {
        if let Some(v) = ron_tree::walk(root, &path[..k]) {
            match v {
                ron_tree::Val::Seq { open, .. }
                | ron_tree::Val::Struct { open, .. }
                | ron_tree::Val::Map { open, .. } => *open = true,
                ron_tree::Val::Atom(..) => {}
            }
        }
    }
}

/// The list-edit buttons a row carries: `(add, del)` — an add on an
/// open sequence's head row, a delete on one of a sequence's OWN item
/// rows (a head or an atom; the closing bracket row never gets one).
pub(crate) fn row_buttons(root: &ron_tree::Val, row: &ron_view::Row) -> (bool, bool) {
    let add =
        matches!(row.vkind, ron_view::VKind::Head) && row.fold == Some(true) && row.val == "[";
    let del = !row.path.is_empty()
        && !matches!(row.vkind, ron_view::VKind::Close)
        && ron_tree::find(root, &row.path[..row.path.len() - 1])
            .is_some_and(|p| matches!(p, ron_tree::Val::Seq { .. }));
    (add, del)
}

/// Append to the sequence at `path` a copy of its last entry — the
/// shape fits, the comments do not travel — or a fresh `0` when the
/// sequence is empty; the new entry's index back.
pub(crate) fn seq_add(root: &mut ron_tree::Val, path: &[u16]) -> Option<usize> {
    let ron_tree::Val::Seq { items, .. } = ron_tree::walk(root, path)? else {
        return None;
    };
    let fresh = match items.last() {
        Some(it) => ron_tree::Item::plain(zero_like(&it.val)),
        // An empty list keeps no shape to copy; in this sidecar's world
        // a list is a list of positions, so the first entry starts as
        // an unplaced one at the corner.
        None => ron_tree::Item::plain(ron_tree::Val::Struct {
            open: true,
            head: String::new(),
            curly: false,
            fields: (0..2)
                .map(|_| {
                    (
                        String::new(),
                        ron_tree::Item::plain(ron_tree::Val::Atom(
                            String::from("0.0"),
                            ron_tree::Kind::Num,
                        )),
                    )
                })
                .collect(),
            tail: String::new(),
        }),
    };
    items.push(fresh);
    Some(items.len() - 1)
}

/// The emptied shape of a value: an atom to a fresh zero of its kind,
/// a container to the same skeleton with every child emptied — folds
/// open, comments left out. A sequence's new entry takes this shape
/// from its neighbour's, never a copy of one: an exact twin of the
/// last marker would land on the very pixel its twin already holds,
/// indistinguishable, unclickable, shaky.
pub(crate) fn zero_like(v: &ron_tree::Val) -> ron_tree::Val {
    use ron_tree::{Item, Val};
    match v {
        Val::Atom(_, kind) => Val::Atom(
            match kind {
                ron_tree::Kind::Num => String::from("0.0"),
                ron_tree::Kind::Bool => String::from("false"),
                ron_tree::Kind::Str => String::from("\"\""),
                ron_tree::Kind::Char => String::from("'\\''"),
                // A path is its variant's name: there is no emptier one.
                ron_tree::Kind::Path => v_atom_text(v),
            },
            *kind,
        ),
        Val::Seq { .. } => Val::Seq {
            open: true,
            items: Vec::new(),
            tail: String::new(),
        },
        Val::Struct {
            head,
            curly,
            fields,
            ..
        } => Val::Struct {
            open: true,
            head: head.clone(),
            curly: *curly,
            fields: fields
                .iter()
                .map(|(name, it)| (name.clone(), Item::plain(zero_like(&it.val))))
                .collect(),
            tail: String::new(),
        },
        Val::Map { entries, .. } => Val::Map {
            open: true,
            entries: entries
                .iter()
                .map(|(k, it)| (k.clone(), Item::plain(zero_like(&it.val))))
                .collect(),
            tail: String::new(),
        },
    }
}

/// The lone atom's own text — a path keeps its name when emptied.
pub(crate) fn v_atom_text(v: &ron_tree::Val) -> String {
    match v {
        ron_tree::Val::Atom(text, _) => text.clone(),
        _ => String::new(),
    }
}

/// Remove the nth entry of the sequence at `path`; true when it was there.
pub(crate) fn seq_remove(root: &mut ron_tree::Val, path: &[u16], n: usize) -> bool {
    if let Some(ron_tree::Val::Seq { items, .. }) = ron_tree::walk(root, path)
        && n < items.len()
    {
        items.remove(n);
        return true;
    }
    false
}

/// Slide one entry of the sequence to another entry's seat; the drag
/// takes single steps from the entry's current seat each frame. The
/// true seat it landed on, when anything moved.
pub(crate) fn seq_move(
    root: &mut ron_tree::Val,
    path: &[u16],
    from: usize,
    to: usize,
) -> Option<usize> {
    if let Some(ron_tree::Val::Seq { items, .. }) = ron_tree::walk(root, path)
        && from < items.len()
    {
        let to = to.min(items.len() - 1);
        if to != from {
            let it = items.remove(from);
            items.insert(to, it);
            return Some(to);
        }
    }
    None
}

/// Whether a row wears the selection band: the row describes the very
/// position picked for editing. Both-sides-`None` is NOT a match — no
/// position picked means no band, or every key and bracket row in the
/// tree would light up at once.
pub(crate) fn row_selected(owner: Option<usize>, editing: Option<usize>) -> bool {
    owner.is_some() && owner == editing
}

/// One sidecar view's tree spec, built from its document: the rows
/// re-labelled for the widget — a row that owns a position wears its
/// spot's palette colour, every other row its kind's — with the add
/// and delete buttons the list-edit rules give it, and the selection
/// band on the rows of the picked position, in this view alone.
pub(crate) fn ron_spec<'a>(
    sprites: &'a [Sprite],
    active: usize,
    v: &RonView,
    lines: &'a mut Vec<frost::TreeLine<'a>>,
) -> Option<frost::TreeSpec<'a>> {
    let doc = sprites.get(v.slot)?.ron.as_ref()?;
    let mut spots = Vec::new();
    scan_spots(&doc.root, &mut Vec::new(), "", &mut spots);
    lines.clear();
    for row in doc.rows.iter() {
        let owner = spot_of_row(&spots, &row.path);
        let val_color = owner.map(|pi| PALETTE[pi % PALETTE.len()]).map_or(
            ron_color(row.vkind),
            |(r, g, b)| frost::Color { r, g, b, a: 1.0 },
        );
        let (add, del) = row_buttons(&doc.root, row);
        lines.push(frost::TreeLine {
            key: row.key.as_str(),
            val: row.val.as_str(),
            head: matches!(row.vkind, ron_view::VKind::Head | ron_view::VKind::Close),
            val_color,
            add,
            del,
            band: row_selected(owner, doc.edit) && v.slot == active,
        });
    }
    Some(frost::TreeSpec {
        id: v.slot as u64,
        rows: lines.as_slice(),
        panel: v.panel,
        body: (v.body != [0.0; 4] && !v.folded).then_some(v.body),
        close: v.close,
        folded: v.folded,
        scroll: doc.scroll,
        sx: doc.sx,
        size: [doc.vw, doc.vh],
        style: frost::TreeStyle::default(),
        z: RON_ORDER,
    })
}

/// One open sidecar's panel as the UI laid it out this frame: whose
/// slot it shows, its body viewport and whole-panel rectangles, the
/// close button's centre, and whether the title bar folded it.
pub(crate) struct RonView {
    pub(crate) slot: usize,
    pub(crate) body: [f32; 4],
    pub(crate) panel: [f32; 4],
    pub(crate) close: [f32; 2],
    pub(crate) folded: bool,
}

/// Close one slot's place in a set of views laid out before the close:
/// the closed slot's view goes with the file, and every later view now
/// speaks for the slot its sprite shifted into. The frames after the
/// close rebuild the views from the sprite list; this keeps the one
/// that still draws after the close honest.
pub(crate) fn close_view_slot(views: &mut Vec<RonView>, slot: usize) {
    views.retain(|v| v.slot != slot);
    for v in views.iter_mut() {
        if v.slot > slot {
            v.slot -= 1;
        }
    }
}

/// How many characters a view `w` pixels wide shows side to side.
pub(crate) fn ron_chars(w: f32) -> usize {
    ((w - 2.0 * RON_INSET) / RON_ADV).max(8.0) as usize
}

/// The widest row of the tree, in characters — its horizontal extent.
pub(crate) fn ron_width(doc: &RonDoc) -> usize {
    doc.rows
        .iter()
        .map(|r| r.key.chars().count() + r.val.chars().count())
        .max()
        .unwrap_or(0)
}

/// The active slot after swapping slots `i` and `j`: the marker follows
/// the sprite it was on, which may have moved.
pub(crate) fn swapped_active(active: usize, i: usize, j: usize) -> usize {
    if active == i {
        j
    } else if active == j {
        i
    } else {
        active
    }
}

impl Demo {
    /// A still click on a sidecar view's row. A row that describes a
    /// position picks it for editing — its file becomes the active one
    /// first, since the sprite click that moves the position lands on
    /// the work area; a second click lets go. Any other foldable row
    /// folds its node, and the view re-flattens. Folding a panel whole
    /// is the UI's own title-bar click, away from the rows.
    pub(crate) fn row_click(&mut self, ctx: &mut frost::Context, slot: usize, row: usize) {
        let Some(doc) = self.sprites.get(slot).and_then(|sp| sp.ron.as_ref()) else {
            return;
        };
        let Some(row) = doc.rows.get(row) else {
            return;
        };
        // The row's facts out of the tree first — the path it names,
        // the fold flag it carries, the position it describes and the
        // work size the glide will use — so the borrows below can
        // flip to mutable.
        let path = row.path.clone();
        let foldable = row.fold.is_some();
        let mut spots = Vec::new();
        scan_spots(&doc.root, &mut Vec::new(), "", &mut spots);
        let owner = spot_of_row(&spots, &path)
            .map(|pi| (pi, spots[pi].label.clone(), spots[pi].x, spots[pi].y));
        let wsz = [
            self.sprites[slot].current.width() as f32,
            self.sprites[slot].current.height() as f32,
        ];
        if owner.is_some() && self.active != slot {
            self.active = slot;
            self.selection = None;
            self.last_click = None;
            self.sync_work(ctx);
        }
        let mut say = None;
        if let Some(doc) = self.sprites[slot].ron.as_mut() {
            if let Some((pi, label, sx, sy)) = owner {
                doc.edit = if doc.edit == Some(pi) { None } else { Some(pi) };
                if doc.edit.is_some() {
                    // The sprite glides the picked position to its
                    // centre: the point being edited sits under the eye.
                    self.offset = center_offset((sx, sy), wsz, self.zoom);
                }
                say = Some(if doc.edit.is_some() {
                    format!("editing '{label}' — click the sprite to move it")
                } else {
                    String::from("edit: released")
                });
            } else if foldable {
                if let Some(v) = ron_tree::walk(&mut doc.root, &path) {
                    v.toggle();
                }
                doc.rows = ron_view::layout(&doc.root);
                doc.clamp_scroll();
            }
        }
        if let Some(t) = say {
            self.status = t;
        }
    }

    /// A sidecar view's [+]: append an entry to the sequence the row
    /// opens — a copy of its last entry, or a fresh `0` in an empty
    /// one. A fresh copy of a position is picked up right away, ready
    /// for the sprite click that places it.
    pub(crate) fn row_add(&mut self, slot: usize, row: usize) {
        let Some(doc) = self.sprites.get(slot).and_then(|sp| sp.ron.as_ref()) else {
            return;
        };
        let Some(row) = doc.rows.get(row) else {
            return;
        };
        let cpath = row.path.clone();
        // The [+] lives only on sequence rows, so this is always a
        // real step: the world before the fresh entry goes down.
        self.stamp();
        let Some(doc) = self.sprites[slot].ron.as_mut() else {
            return;
        };
        if let Some(n) = seq_add(&mut doc.root, &cpath) {
            doc.rows = ron_view::layout(&doc.root);
            doc.clamp_scroll();
            let mut fresh = Vec::new();
            scan_spots(&doc.root, &mut Vec::new(), "", &mut fresh);
            let mut want = cpath.clone();
            want.push(n as u16);
            doc.edit = fresh.iter().position(|x| x.path == want);
            self.status = if doc.edit.is_some() {
                format!("added entry #{n} — click the sprite to place it")
            } else {
                format!("added entry #{n} to the list")
            };
        }
    }

    /// A sidecar view's [-]: remove the entry the row names from its
    /// sequence — the picked editing goes with it.
    pub(crate) fn row_del(&mut self, ctx: &mut frost::Context, slot: usize, row: usize) {
        let Some(row) = self
            .sprites
            .get(slot)
            .and_then(|sp| sp.ron.as_ref())
            .and_then(|doc| doc.rows.get(row))
        else {
            return;
        };
        let path = row.path.clone();
        if path.is_empty() {
            return;
        }
        // The removal is coming: remember the entry while it lives.
        self.stamp();
        let mut spots = Vec::new();
        if let Some(doc) = self.sprites[slot].ron.as_ref() {
            scan_spots(&doc.root, &mut Vec::new(), "", &mut spots);
        }
        if spot_of_row(&spots, &path).is_some() && self.active != slot {
            self.active = slot;
            self.selection = None;
            self.last_click = None;
            self.sync_work(ctx);
        }
        let Some(doc) = self.sprites[slot].ron.as_mut() else {
            return;
        };
        let n = *path.last().expect("a non-empty path's last index") as usize;
        let cpath = &path[..path.len() - 1];
        if seq_remove(&mut doc.root, cpath, n) {
            doc.rows = ron_view::layout(&doc.root);
            doc.clamp_scroll();
            doc.edit = None;
            self.status = format!("removed entry #{n} from the list");
        }
    }

    /// A press in a sidecar view's tree that travels past the click
    /// tolerance becomes a reorder: the entry the press started on
    /// follows the pointer, sliding one seat per crossed row.
    pub(crate) fn ron_drag_start(&mut self, slot: usize, row: usize) {
        // A reorder is one command however many rows it shuffles: the
        // world before the drag goes down now.
        self.stamp();
        let Some(doc) = self.sprites.get(slot).and_then(|sp| sp.ron.as_ref()) else {
            return;
        };
        let Some(row) = doc.rows.get(row) else {
            return;
        };
        let mut path = row.path.clone();
        let Some(seat) = path.pop() else {
            return;
        };
        self.ron_drag = Some((slot, path, seat as usize));
    }

    /// The pointer's row while a sidecar reorder rides: the dragged
    /// entry slides to the seat the pointer crosses, one seat at a
    /// time, and the status follows each landing.
    pub(crate) fn ron_drag_move(&mut self, slot: usize, row: Option<usize>) {
        let Some((_, cpath, from)) = self.ron_drag.clone() else {
            return;
        };
        let Some(j) = row else {
            return;
        };
        let Some(doc) = self.sprites.get(slot).and_then(|sp| sp.ron.as_ref()) else {
            return;
        };
        let Some(r) = doc.rows.get(j) else {
            return;
        };
        let to = (r.path.len() == cpath.len() + 1
            && r.path.starts_with(&cpath)
            && !matches!(r.vkind, ron_view::VKind::Close))
        .then(|| r.path[cpath.len()] as usize);
        let Some(doc) = self.sprites[slot].ron.as_mut() else {
            return;
        };
        if let Some(to) = to
            && let Some(to) = seq_move(&mut doc.root, &cpath, from, to)
        {
            doc.rows = ron_view::layout(&doc.root);
            doc.clamp_scroll();
            self.ron_drag = Some((slot, cpath, to));
            self.status = format!("reordering: entry #{from} -> #{to}");
        }
    }
}

#[cfg(test)]
pub(crate) fn view(slot: usize) -> RonView {
    RonView {
        slot,
        body: [0.0; 4],
        panel: [0.0; 4],
        close: [0.0; 2],
        folded: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strip::{STRIP_H, floor_y};

    #[test]
    fn swapping_slots_moves_the_active_marker_with_its_sprite() {
        // The active sprite sits in slot 2; swapping 1 and 2 moves the
        // marker, swapping 0 and 1 leaves it.
        assert_eq!(swapped_active(2, 1, 2), 1);
        assert_eq!(swapped_active(1, 1, 2), 2);
        assert_eq!(swapped_active(0, 1, 2), 0);
    }

    #[test]
    fn a_new_sidecar_panel_opens_at_the_lower_right() {
        let (w, h) = (800.0, 600.0);
        let (row_h, pad) = (28.0, 12.0); // the UI style's defaults
        let c = ron_default(w, h, row_h, pad, 0);
        let panel_h = row_h + 2.0 * pad + RON_VIEW_H;
        assert!(
            c[1] - panel_h / 2.0 >= floor_y(h) + STRIP_H,
            "a fresh panel should clear the slot strip"
        );
        assert!(
            c[1] + panel_h / 2.0 < h / 2.0,
            "a fresh panel should stay inside"
        );
        assert!((c[0] + RON_W / 2.0 - (w / 2.0 - 20.0)).abs() < 1e-3);
    }
    #[test]
    fn a_fresh_position_never_lands_on_its_twin() {
        // The shape the sidecar writes: a list of two-number tuples.
        let mut root =
            ron_tree::parse("(flower_anchors: [(308.0, 615.0)])").expect("a small sidecar");
        let n = seq_add(&mut root, &[0]).expect("a sequence");
        assert_eq!(n, 1);
        let found = spots_of(&root);
        // Two positions, and the fresh one is an unplaced corner —
        // never an exact copy of the anchor it grew beside.
        assert_eq!(found.len(), 2);
        assert_eq!((found[1].1, found[1].2), (0.0, 0.0));
        assert_eq!((found[0].1, found[0].2), (308.0, 615.0));
    }

    #[test]
    fn the_first_entry_of_an_empty_list_is_a_placeable_position() {
        // An empty list has no neighbour to take shape from: the [+]
        // must still put down a position the click can claim, not a
        // bare `0` the tree shows but no marker or edit can reach.
        let mut root = ron_tree::parse("(flower_anchors: [])").expect("an empty list");
        let n = seq_add(&mut root, &[0]).expect("a sequence");
        assert_eq!(n, 0);
        let found = spots_of(&root);
        assert_eq!(found.len(), 1, "the new entry is a position, not an atom");
        assert_eq!((found[0].1, found[0].2), (0.0, 0.0));
    }

    #[test]
    fn a_fresh_number_joins_a_list_of_numbers() {
        // The same honesty for the shapes that are not positions: the
        // zero of a number list is a number.
        let mut root = ron_tree::parse("(seats: [1, 2])").expect("numbers");
        assert_eq!(seq_add(&mut root, &[0]), Some(2));
        let Some(ron_tree::Val::Seq { items, .. }) = ron_tree::walk(&mut root, &[0]) else {
            panic!("the seats list is not a sequence");
        };
        let ron_tree::Val::Atom(text, ron_tree::Kind::Num) = &items[2].val else {
            panic!("the fresh seat is not an atom: {:?}", items[2].val);
        };
        assert_eq!(text, "0.0");
    }

    #[test]
    fn sequence_rows_carry_their_buttons() {
        let mut root = ron_tree::parse("(a: [1, (2.0, 3.0)], b: 4)").expect("parses");
        root.open_to(0, 9);
        let rows = ron_view::layout(&root);
        let row = |want: &str| {
            rows.iter()
                .find(|r| r.val == want && !(want == "(" && r.path.is_empty()))
                .expect(want)
        };
        // The open sequence's head row gets the add; it is a field, not
        // an entry, so it gets no delete.
        assert_eq!(row_buttons(&root, row("[")), (true, false));
        // Both kinds of entry row — a bare atom and a tuple's head —
        // get the delete, and neither is a sequence to add into.
        assert_eq!(row_buttons(&root, row("1")), (false, true));
        assert_eq!(row_buttons(&root, row("(")), (false, true));
        // A plain field row belongs to no list at all — and neither
        // does the root's own `(` head.
        assert_eq!(row_buttons(&root, row("4")), (false, false));
        let root_head = rows.iter().find(|r| r.path.is_empty()).expect("root row");
        assert_eq!(row_buttons(&root, root_head), (false, false));
    }

    #[test]
    fn lists_add_zero_delete_and_slide() {
        let mut root = ron_tree::parse("[10, 20, 30]").expect("parses");
        // Add puts down an emptied shape — a fresh 0.0 beside their
        // numbers — and comments stay behind.
        assert_eq!(seq_add(&mut root, &[]), Some(3));
        assert!(seq_remove(&mut root, &[], 0));
        assert!(!seq_remove(&mut root, &[], 9));
        assert_eq!(seq_move(&mut root, &[], 2, 0), Some(0));
        assert_eq!(seq_move(&mut root, &[], 1, 1), None);
        assert_eq!(
            ron_tree::to_text(&root),
            "[\n    0.0,\n    20,\n    30,\n]\n"
        );
        // Nested sequences answer to their path.
        let mut nest = ron_tree::parse("(ups: [(1.0, 1.0)])").expect("parses");
        assert_eq!(seq_add(&mut nest, &[0]), Some(1));
        assert_eq!(seq_add(&mut nest, &[1]), None);
    }

    #[test]
    fn a_path_unfolds_from_root_to_row() {
        let mut root = ron_tree::parse("(a: [(1.0, 2.0), (3.0, 4.0)])").expect("parses");
        root.open_to(0, 0);
        let mut spots = Vec::new();
        scan_spots(&root, &mut Vec::new(), "", &mut spots);
        let deep = spots[1].path.clone();
        unfold_path(&mut root, &deep);
        let rows = ron_view::layout(&root);
        // The list opened; both entries show, and the path's own row
        // (the tuple head, folded, previewing its numbers) is there to
        // be centred.
        assert!(rows.iter().any(|r| r.path == deep && r.val == "(3.0, 4.0)"));
        assert!(rows.iter().any(|r| r.path == vec![0, 0]));
    }

    #[test]
    fn only_the_picked_positions_row_is_selected() {
        // The bug this nails: an unpicked view (`None`) must match no
        // row — and plain `owner == editing` matched every row that
        // owns no position, banding the head and bracket rows at the
        // tree's top and bottom the moment a file loaded.
        assert!(!row_selected(None, None));
        assert!(row_selected(Some(2), Some(2)));
        assert!(!row_selected(Some(1), Some(2)));
        assert!(!row_selected(None, Some(2)));
        assert!(!row_selected(Some(1), None));
    }

    #[test]
    fn closing_a_slot_shifts_the_views_behind_it() {
        // The bug this nails: a close lands AFTER the frame's views
        // were laid out, and the frame's draw still walks them — the
        // closed file's view must go with it, and every later view must
        // speak for the slot its sprite shifted into.
        let mut views = vec![view(0), view(1), view(2)];
        close_view_slot(&mut views, 1);
        assert_eq!(views.len(), 2);
        assert_eq!(views[0].slot, 0, "the front view keeps its seat");
        assert_eq!(
            views[1].slot, 1,
            "the later view shifts into the freed seat"
        );
    }

    #[test]
    fn closing_the_last_view_leaves_no_dangling_slot() {
        // Closing the last sprite used to leave its view behind,
        // pointing one slot past the end of the shrunken list — the
        // panic in the frame's draw.
        let mut views = vec![view(0), view(1)];
        close_view_slot(&mut views, 1);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].slot, 0);
        // A slot with no view of its own still shifts the later ones.
        let mut views = vec![view(0), view(2)];
        close_view_slot(&mut views, 1);
        assert_eq!(views.len(), 2);
        assert_eq!((views[0].slot, views[1].slot), (0, 1));
    }
}
