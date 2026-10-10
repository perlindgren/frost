//! The sidecar files: reading and writing <name>.ron beside the
//! PNG, and the atlas field inside them.
// The subjects this one reads.
use crate::art::*;
use crate::ron_panels::*;
use crate::ron_view;
use crate::strip::*;
use crate::world::*;
use frost::ron as ron_tree;

/// A sprite's parsed sidecar: the tree with its fold flags, the
/// flattened visible rows, and the panel's scroll and fold state.
#[derive(Clone)]
pub(crate) struct RonDoc {
    /// The sidecar's file name, for the panel's title.
    pub(crate) name: String,
    /// The comments outside the root value, kept verbatim so saving
    /// rewrites the file with them: the header above the tree and the
    /// trailer after it.
    pub(crate) header: String,
    pub(crate) trailer: String,
    /// The parsed tree — the containers' fold flags live here.
    pub(crate) root: ron_tree::Val,
    /// The visible rows, rebuilt whenever a fold flips.
    pub(crate) rows: Vec<ron_view::Row>,
    /// The scroll offsets in pixels: `scroll` down the rows, `sx`
    /// across them, 0 at the tree's top-left.
    pub(crate) scroll: f32,
    pub(crate) sx: f32,
    /// The position picked for editing, as an index into a fresh
    /// [`scan_spots`] of this tree: the next sprite click writes to it.
    pub(crate) edit: Option<usize>,
    /// The view's own size, as its resize handle left it: the panel's
    /// width and the tree viewport's height.
    pub(crate) vw: f32,
    pub(crate) vh: f32,
}

impl RonDoc {
    /// Re-pin both scroll offsets into THIS view's window, so folding
    /// a node or dragging a resize handle can never strand the tree
    /// past its rows or columns.
    pub(crate) fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.clamp(
            0.0,
            (self.rows.len() as f32 * RON_LINE + RON_PAD - self.vh).max(0.0),
        );
        let wide = ron_width(self) as f32 - ron_chars(self.vw) as f32;
        self.sx = self.sx.clamp(0.0, (wide * RON_ADV).max(0.0));
    }
}

/// Read a PNG from disk into a sprite: its working texture, its work-area
/// shape and its slot thumbnail (the original, minimized once so the GPU
/// never minifies the full texture on its own).
pub(crate) fn read_sprite(path: &std::path::Path) -> Result<Sprite, String> {
    let name = file_name_of(path);
    if path.extension().and_then(|e| e.to_str()) != Some("png") {
        return Err(format!("'{name}' is not a PNG file"));
    }
    let shape =
        frost::Shape::sprite_nearest(path).map_err(|e| format!("failed to load '{name}': {e}"))?;
    let tex = image::open(path)
        .map(|img| img.to_rgba8())
        .map_err(|e| format!("failed to decode '{name}': {e}"))?;
    let (iw, ih) = (tex.width(), tex.height());
    log::info!("loaded '{name}': {iw}x{ih} pixels");
    let k = THUMB_MAX / (iw.max(ih) as f32);
    let (tw, th) = (
        ((iw as f32) * k).round().max(1.0) as u32,
        ((ih as f32) * k).round().max(1.0) as u32,
    );
    let small = image::imageops::resize(&tex, tw, th, image::imageops::FilterType::Lanczos3);
    let thumb = frost::Shape::sprite_bytes_nearest(&png_bytes(&small)?)
        .map_err(|e| format!("failed to build the thumbnail: {e}"))?;
    let ron = load_ron(path);
    // The grid the sidecar named, if it named one — the sprite reloads
    // tiled the way it was saved.
    let atlas = ron.as_ref().and_then(|doc| atlas_of(&doc.root));
    if let Some((rows, cols)) = atlas {
        log::info!("atlas for '{name}': {rows} x {cols}");
    }
    // Both baselines start as the disk: the decode is the texture's
    // saved state, and the sidecar's canonical rendering is the tree's —
    // the exact text a save would write, so a fresh sprite opens clean.
    let saved_ron = ron
        .as_ref()
        .map(|doc| ron_tree::to_text_doc(&doc.header, &doc.root, &doc.trailer));
    Ok(Sprite {
        path: path.to_path_buf(),
        name,
        current: tex.clone(),
        saved_img: tex,
        shape,
        thumb,
        thumb_img: small,
        ron,
        saved_ron,
        atlas,
    })
}

/// The sprite's sidecar: `<name>.ron` beside `<name>.png`. Absent is the
/// normal case; unparseable is logged and skipped. Either way the sprite
/// itself has already loaded and loads unchanged.
pub(crate) fn load_ron(png: &std::path::Path) -> Option<RonDoc> {
    let side = png.with_extension("ron");
    let Ok(text) = std::fs::read_to_string(&side) else {
        return None;
    };
    let doc = match ron_tree::parse_doc(&text) {
        Ok(d) => d,
        Err(err) => {
            log::warn!("sidecar '{}': {err}", file_name_of(&side));
            return None;
        }
    };
    let ron_tree::Doc {
        header,
        trailer,
        mut root,
    } = doc;
    // The tree opens its first two levels; everything deeper waits.
    root.open_to(0, 1);
    let name = file_name_of(&side);
    log::info!("sidecar '{name}' loaded for '{}'", file_name_of(png));
    let rows = ron_view::layout(&root);
    Some(RonDoc {
        name,
        header,
        trailer,
        root,
        rows,
        scroll: 0.0,
        sx: 0.0,
        edit: None,
        vw: RON_W,
        vh: RON_VIEW_H,
    })
}

/// The sidecar a sprite gains on the first save that names a grid: a
/// fresh document holding just the `atlas` field, laid out and ready
/// for the panel — the shape [`load_ron`] builds for an existing file.
pub(crate) fn new_sidecar(name: String, atlas: (usize, usize)) -> RonDoc {
    let mut root = ron_tree::Val::Struct {
        open: true,
        head: String::new(),
        curly: false,
        fields: vec![(
            "atlas".to_string(),
            ron_tree::Item::plain(atlas_value(atlas.0, atlas.1)),
        )],
        tail: String::new(),
    };
    root.open_to(0, 1);
    let rows = ron_view::layout(&root);
    let mut doc = RonDoc {
        name,
        header: String::new(),
        trailer: String::new(),
        root,
        rows,
        scroll: 0.0,
        sx: 0.0,
        edit: None,
        vw: RON_W,
        vh: RON_VIEW_H,
    };
    doc.clamp_scroll();
    doc
}

/// The `atlas` field's value: the counts as a two-entry array,
/// `[rows, cols]`.
pub(crate) fn atlas_value(rows: usize, cols: usize) -> ron_tree::Val {
    ron_tree::Val::Seq {
        open: true,
        items: vec![
            ron_tree::Item::plain(ron_tree::Val::Atom(rows.to_string(), ron_tree::Kind::Num)),
            ron_tree::Item::plain(ron_tree::Val::Atom(cols.to_string(), ron_tree::Kind::Num)),
        ],
        tail: String::new(),
    }
}

/// The tile grid the sidecar's root names, if it names one: its
/// `atlas` field's two entries, `[rows, cols]`. A field of the wrong
/// shape — not a two-entry array, or a missing or zero count — reads
/// as no grid, the way a missing field does.
pub(crate) fn atlas_of(root: &ron_tree::Val) -> Option<(usize, usize)> {
    let ron_tree::Val::Struct { fields, .. } = root else {
        return None;
    };
    let (_, item) = fields.iter().find(|(key, _)| *key == "atlas")?;
    let ron_tree::Val::Seq { items, .. } = &item.val else {
        return None;
    };
    if items.len() != 2 {
        return None;
    }
    let counts: Vec<usize> = items
        .iter()
        .filter_map(|it| match &it.val {
            ron_tree::Val::Atom(text, ron_tree::Kind::Num) => text.parse().ok(),
            _ => None,
        })
        .collect();
    match counts.as_slice() {
        [rows, cols] if *rows > 0 && *cols > 0 => Some((*rows, *cols)),
        _ => None,
    }
}

/// The tile grid written into the sidecar's root: its `atlas` field
/// becomes `[rows, cols]`, or is dropped when the grid is off. An
/// array, not a tuple — a two-number tuple would read as a position
/// spot, and the grid's counts are not pixels. Whether the tree
/// changed.
pub(crate) fn set_atlas(root: &mut ron_tree::Val, atlas: Option<(usize, usize)>) -> bool {
    let ron_tree::Val::Struct { fields, .. } = root else {
        return false;
    };
    match atlas {
        Some((rows, cols)) => {
            let seq = atlas_value(rows, cols);
            match fields.iter_mut().find(|(key, _)| *key == "atlas") {
                Some((_, item)) => item.val = seq,
                None => fields.push(("atlas".to_string(), ron_tree::Item::plain(seq))),
            }
            true
        }
        None => {
            let before = fields.len();
            fields.retain(|(key, _)| *key != "atlas");
            fields.len() != before
        }
    }
}

/// Where a sidecar panel first opens: at the work area's lower right,
/// clear of the slot strip — later views cascade up and left from
/// there. `row_h` and `pad` are the UI style's, and the panel's height
/// is its title bar, twice the padding and the tree viewport.
pub(crate) fn ron_default(w: f32, h: f32, row_h: f32, pad: f32, cascade: usize) -> [f32; 2] {
    let panel_h = row_h + 2.0 * pad + RON_VIEW_H;
    let step = cascade as f32 * 34.0;
    [
        w / 2.0 - 20.0 - RON_W / 2.0 - step,
        floor_y(h) + STRIP_H + 12.0 + panel_h / 2.0 + step,
    ]
}

/// The bar's voice for the authored shapes: the reading's first refusal
/// in full, and a count of the ones behind it. The menu bar has one
/// line's width, and the first refusal is the one to fix first — the
/// reader hands the whole list to any caller who wants to show more,
/// and every message already names its entry, so even the second is
/// findable from the first's neighbourhood on screen.
pub(crate) fn shapes_note(errs: &frost::ShapeErrors) -> Option<String> {
    let first = errs.errors.first()?;
    Some(match errs.errors.len() {
        1 => first.message.clone(),
        n => format!("{} (+{} more)", first.message, n - 1),
    })
}

/// One authored shape's outline in window space, closed by the caller's
/// neighbour-walking: a rect's four corners, a circle's inscribed
/// [`frost::DISC_SIDES`]-gon — the very ring `Convex::disc` sends to the
/// occluder field, so the desk shows the edge the shadow will have —
/// and a polygon's own corners, added to its entry's `at` as authored.
/// The map is the desk's one, shared by the markers and the click math:
/// texture pixels measured y down from the image's top-left, the window
/// y up about its centre, every distance scaled by the zoom and shifted
/// by the pan.
pub(crate) fn shape_corners(
    sh: &frost::Authored,
    size: [f32; 2],
    view: [f32; 2],
    zoom: f32,
) -> Vec<[f32; 2]> {
    let wx = |px: f32| (px - size[0] / 2.0) * zoom + view[0];
    let wy = |py: f32| (size[1] / 2.0 - py) * zoom + view[1];
    match &sh.shape {
        frost::AuthoredShape::Rect { size: s } => {
            let (x0, x1) = (wx(sh.at[0] - s[0] / 2.0), wx(sh.at[0] + s[0] / 2.0));
            // Pixels measure y down, the window up: the *upper* pixel
            // edge is the *higher* window one.
            let (y0, y1) = (wy(sh.at[1] + s[1] / 2.0), wy(sh.at[1] - s[1] / 2.0));
            vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
        }
        frost::AuthoredShape::Circle { radius } => {
            let r = radius * zoom;
            let (cx, cy) = (wx(sh.at[0]), wy(sh.at[1]));
            let step = std::f32::consts::TAU / frost::DISC_SIDES as f32;
            (0..frost::DISC_SIDES)
                .map(|k| {
                    let (sin, cos) = (k as f32 * step).sin_cos();
                    [cx + r * cos, cy + r * sin]
                })
                .collect()
        }
        frost::AuthoredShape::Poly { points } => points
            .iter()
            .map(|p| [wx(sh.at[0] + p[0]), wy(sh.at[1] + p[1])])
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spots::scan_spots;

    #[test]
    fn authored_shapes_stay_in_the_textures_pixels_the_author_wrote() {
        let root = ron_tree::parse(
            "(atlas: [4, 4], shapes: [(kind: \"rect\", at: [16.0, 8.0], size: [24.0, 16.0], solid: true)])",
        )
        .expect("a small sidecar");
        let (authored, errs) = frost::read_shapes(&root);
        assert!(errs.is_empty(), "the reading said:\n{errs}");
        assert_eq!(authored.len(), 1);
        // What the desk draws are the pixels: no centring, no flip —
        // those belong to the baker's leg of the read, not the tool's.
        assert_eq!(authored[0].at, [16.0, 8.0]);
        assert_eq!(
            authored[0].shape,
            frost::AuthoredShape::Rect { size: [24.0, 16.0] }
        );
        assert!(authored[0].solid, "the roles travel with the geometry");
    }

    #[test]
    fn the_shapes_note_names_the_first_refusal_and_counts_the_rest() {
        let root = ron_tree::parse(
            "(shapes: [
                (kind: \"rect\", at: [8.0, 4.0], size: [8.0, 4.0]),
                (kind: \"blob\", at: [1.0, 2.0], occludes: true),
                (kind: \"capsule\", at: [1.0, 2.0], radius: 2.0, hit: true),
            ])",
        )
        .expect("a small sidecar");
        let (_, errs) = frost::read_shapes(&root);
        let note = shapes_note(&errs).expect("a bad sidecar gets a note");
        assert!(note.starts_with("shapes entry 0:"), "{note}");
        assert!(note.contains("all off"), "{note}");
        assert!(note.ends_with("(+2 more)"), "{note}");
        // A clean reading says nothing: no shapes key, no note.
        let clean = ron_tree::parse("(atlas: [4, 4])").expect("a small sidecar");
        let (_, errs) = frost::read_shapes(&clean);
        assert_eq!(shapes_note(&errs), None);
    }

    #[test]
    fn the_shapes_overlay_rides_the_map_the_markers_use() {
        // Texture pixel (16, 8) of a 32 x 16 sheet is its centre: the
        // shape's centre must land on the desk's centre, at any zoom
        // and any pan. The asymmetric corners pin the y-flip — the
        // *upper* pixel edge sits *higher* on the window, the opposite
        // of what the reader tells the baker, and the same flip the
        // markers and the click map already make.
        let rect = frost::Authored {
            entry: 0,
            at: [16.0, 8.0],
            shape: frost::AuthoredShape::Rect { size: [8.0, 4.0] },
            solid: true,
            bounce: 0.0,
            occludes: false,
            hit: false,
        };
        let view = [7.0, -3.0];
        let corners = shape_corners(&rect, [32.0, 16.0], view, 2.0);
        assert_eq!(
            corners,
            [[-1.0, -7.0], [15.0, -7.0], [15.0, 1.0], [-1.0, 1.0]]
        );

        // A disc draws as the ring `Convex::disc` sends to the field:
        // `DISC_SIDES` corners, the first at +x, radius scaled by the
        // zoom like everything else on the desk.
        let disc = frost::Authored {
            shape: frost::AuthoredShape::Circle { radius: 3.0 },
            ..rect.clone()
        };
        let corners = shape_corners(&disc, [32.0, 16.0], view, 2.0);
        assert_eq!(corners.len(), frost::DISC_SIDES);
        assert!((corners[0][0] - 13.0).abs() < 1e-3, "{corners:?}");
        assert!((corners[0][1] + 3.0).abs() < 1e-3, "{corners:?}");

        // A polygon's corners are relative to `at`, and stay that way:
        // the map adds the centre exactly once.
        let poly = frost::Authored {
            at: [10.0, 10.0],
            shape: frost::AuthoredShape::Poly {
                points: vec![[0.0, 0.0], [4.0, 0.0], [0.0, 4.0]],
            },
            ..rect
        };
        let corners = shape_corners(&poly, [32.0, 16.0], view, 2.0);
        // wx(10) = (10-16)*2+7 = -5; wy(10) = (8-10)*2-3 = -7 — the
        // authored corner (0, 0); (4, 0) moves right in x, and the
        // pixel-down corner (0, 4) moves DOWN on the window.
        assert_eq!(corners, [[-5.0, -7.0], [3.0, -7.0], [-5.0, -15.0]]);
    }

    #[test]
    fn comments_survive_the_sidecar_round_trip() {
        // The user's rule: saving a sprite must not shred the notes
        // beside its data.
        let src = "// planted in spring\n(\n    lower: (1.0, 2.0), // where it sits\n)\n";
        let doc = ron_tree::parse_doc(src).expect("parses");
        let text = ron_tree::to_text_doc(&doc.header, &doc.root, &doc.trailer);
        assert!(text.contains("// planted in spring"), "{text}");
        assert!(text.contains("// where it sits"), "{text}");
        assert_eq!(ron_tree::parse_doc(&text).expect("re-parses"), doc);
    }

    #[test]
    fn a_real_plant_sidecar_round_trips_with_its_comments() {
        let src = include_str!("../../assets/sprites/plant1.ron");
        let d = ron_tree::parse_doc(src).expect("plant1 parses");
        let text = ron_tree::to_text_doc(&d.header, &d.root, &d.trailer);
        assert_eq!(ron_tree::parse_doc(&text).expect("re-parses"), d);
        for line in src.lines().filter(|l| l.trim_start().starts_with("//")) {
            assert!(text.contains(line.trim_end()), "lost {line:?}");
        }
    }

    #[test]
    fn a_sidecar_parses_or_the_sprite_loads_alone() {
        // The tree panel speaks the shared parser: the plant files'
        // tuples, anchors and all — and a broken one is simply None.
        let dir = std::env::temp_dir().join(format!("sprite_util_ron_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("probe.png");
        std::fs::write(&png, b"not really a png, the sidecar loader never reads it").unwrap();
        assert!(load_ron(&png).is_none(), "no sidecar, no panel");
        std::fs::write(
            dir.join("probe.ron"),
            "(segment: 1, lower_anchor: (317.0, 671.0), flower_anchors: [(4.0, 2.0)])",
        )
        .unwrap();
        let doc = load_ron(&png).expect("the sidecar parses");
        assert_eq!(doc.name, "probe.ron");
        assert!(!doc.rows.is_empty());
        std::fs::write(dir.join("probe.ron"), "(segment: ,,,)").unwrap();
        assert!(
            load_ron(&png).is_none(),
            "an unparseable sidecar is no panel"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_atlas_reads_the_root_s_two_entry_array() {
        let root = ron_tree::parse("(segment: 1, atlas: [2, 3])").expect("a small sidecar");
        assert_eq!(atlas_of(&root), Some((2, 3)));
        // A field of the wrong shape reads as no grid, the way a
        // missing field does: one entry, a zero count, a float, a
        // tuple, a string.
        assert_eq!(atlas_of(&ron_tree::parse("(atlas: [2])").unwrap()), None);
        assert_eq!(atlas_of(&ron_tree::parse("(atlas: [0, 3])").unwrap()), None);
        assert_eq!(
            atlas_of(&ron_tree::parse("(atlas: [2.0, 3.0])").unwrap()),
            None
        );
        assert_eq!(atlas_of(&ron_tree::parse("(atlas: (2, 3))").unwrap()), None);
        assert_eq!(
            atlas_of(&ron_tree::parse("(atlas: \"grid\")").unwrap()),
            None
        );
        assert_eq!(atlas_of(&ron_tree::parse("(segment: 1)").unwrap()), None);
    }

    #[test]
    fn the_atlas_writes_and_rewrites_the_root_s_field() {
        let mut root = ron_tree::parse("(segment: 1, image: \"p.png\")").expect("a small sidecar");
        assert!(set_atlas(&mut root, Some((2, 3))), "appends the field");
        assert_eq!(atlas_of(&root), Some((2, 3)));
        assert!(set_atlas(&mut root, Some((4, 5))), "rewrites it in place");
        assert_eq!(atlas_of(&root), Some((4, 5)));
        // One `atlas` field, not one per write.
        let fields = match &root {
            ron_tree::Val::Struct { fields, .. } => fields,
            _ => panic!("a tuple-struct root"),
        };
        assert_eq!(fields.iter().filter(|(k, _)| *k == "atlas").count(), 1);
        assert!(set_atlas(&mut root, None), "drops the field");
        assert_eq!(atlas_of(&root), None);
        assert!(!set_atlas(&mut root, None), "nothing left to drop");
    }

    #[test]
    fn the_atlas_round_trips_through_text() {
        // What Save writes is what Load reads back.
        let mut root = ron_tree::parse("(segment: 1, image: \"p.png\")").expect("a small sidecar");
        set_atlas(&mut root, Some((3, 4)));
        let text = ron_tree::to_text_doc("", &root, "");
        let doc = ron_tree::parse_doc(&text).expect("re-parses");
        assert_eq!(atlas_of(&doc.root), Some((3, 4)));
    }

    #[test]
    fn the_atlas_array_is_not_a_position_spot() {
        // The grid's counts are not pixels: a two-entry array must not
        // read as a pickable position, the way a two-number tuple does.
        let root =
            ron_tree::parse("(atlas: [2, 3], lower: (317.0, 671.0))").expect("a small sidecar");
        let mut spots = Vec::new();
        scan_spots(&root, &mut Vec::new(), "", &mut spots);
        assert_eq!(
            spots.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(),
            ["lower"],
            "the array is invisible to the spot scan"
        );
    }

    #[test]
    fn the_atlas_restores_from_the_sidecar_on_load() {
        // The sprite reloads tiled the way it was saved: the sidecar's
        // `atlas` field reads into `Sprite.atlas` on the way in, and a
        // missing or broken field reads as no grid.
        let dir = std::env::temp_dir().join(format!("sprite_util_atlas_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("probe.png");
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
        std::fs::write(&png, png_bytes(&img).unwrap()).unwrap();
        assert_eq!(read_sprite(&png).unwrap().atlas, None, "no field, no grid");
        std::fs::write(dir.join("probe.ron"), "(segment: 1, atlas: [2, 3])").unwrap();
        assert_eq!(read_sprite(&png).unwrap().atlas, Some((2, 3)));
        std::fs::write(dir.join("probe.ron"), "(segment: 1, atlas: [0, 3])").unwrap();
        assert_eq!(
            read_sprite(&png).unwrap().atlas,
            None,
            "a broken field reads as no grid"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fresh_sidecar_holds_only_the_grid() {
        // The first save of a sprite that had no sidecar creates one:
        // it holds just the `atlas` field, and the grid reads back the
        // way a loaded file's does.
        let doc = new_sidecar("probe.ron".to_string(), (2, 3));
        assert_eq!(doc.name, "probe.ron");
        let fields = match &doc.root {
            ron_tree::Val::Struct { fields, .. } => fields,
            _ => panic!("a tuple-struct root"),
        };
        assert_eq!(
            fields.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            ["atlas"],
            "nothing but the grid"
        );
        assert!(!doc.rows.is_empty(), "the panel has rows to draw");
        let text = ron_tree::to_text_doc(&doc.header, &doc.root, &doc.trailer);
        let reparsed = ron_tree::parse(&text).expect("the fresh sidecar parses");
        assert_eq!(
            atlas_of(&reparsed),
            Some((2, 3)),
            "the grid survives the file round trip"
        );
    }

    #[test]
    fn the_canonical_sidecar_is_a_round_trip_fixed_point() {
        // The clean/dirty comparison compares canonical texts, so it is
        // only honest if canonicalizing is stable: the text Save writes
        // must re-parse to the very same text. Otherwise a freshly
        // loaded sprite would open dirty, or a save would never clear
        // its own mark.
        let fresh = new_sidecar("probe.ron".into(), (2, 3));
        let once = ron_tree::to_text_doc(&fresh.header, &fresh.root, &fresh.trailer);
        let back = ron_tree::parse_doc(&once).expect("the canonical text parses");
        let twice = ron_tree::to_text_doc(&back.header, &back.root, &back.trailer);
        assert_eq!(once, twice, "canonicalizing is idempotent");
    }

    #[test]
    fn comments_survive_the_canonical_round_trip() {
        // Header and trailer comments ride the same comparison: if the
        // rewrite dropped or reshaped them, the untouched file would
        // count as changed forever after the first save.
        let text = "// header note\n(\n    atlas: [1, 4],\n)\n// trailer note\n";
        let doc = ron_tree::parse_doc(text).expect("the commented sidecar parses");
        let once = ron_tree::to_text_doc(&doc.header, &doc.root, &doc.trailer);
        let back = ron_tree::parse_doc(&once).expect("the rewrite parses");
        let twice = ron_tree::to_text_doc(&back.header, &back.root, &back.trailer);
        assert_eq!(once, twice, "with comments, still idempotent");
        assert!(once.contains("header note") && once.contains("trailer note"));
    }
}
