//! The shapes an author drew in the picture, read out of a sprite's `.ron`
//! sidecar: the `shapes` key's entries turned into [`Feature`]s for a
//! [`Baker`](crate::Baker), placed in the node's space.
//!
//! The vocabulary is one flat list, one entry per shape, with the kind as
//! a field and the three consumers' roles as flags on the same entry — a
//! wall is solid *and* occluding, and one role per entry would make the
//! author write its geometry twice:
//!
//! ```text
//! shapes: [
//!     (kind: "rect", at: [16.0, 8.0], size: [24.0, 16.0], solid: true),
//!     (kind: "circle", at: [16.0, 2.0], radius: 6.0, occludes: true),
//!     (kind: "poly", at: [0.0, 0.0], points: [[0.0, 0.0], [12.0, 4.0], [2.0, 14.0]], occludes: true),
//!     // one wall, both jobs: bouncy, and it throws a shadow
//!     (kind: "rect", at: [16.0, 30.0], size: [32.0, 8.0], solid: (bounce: 0.2), occludes: true),
//! ]
//! ```
//!
//! `kind` is `rect` (needs `size: [w, h]`, full pixels), `circle` (needs
//! `radius`) or `poly` (needs `points`, an array of `[x, y]` corners);
//! `capsule` is written into the grammar but refused for now, by name,
//! because there is no capsule shape anywhere to make of it. `solid` is
//! `true` or `(bounce: <number>)`, and a polygon is honoured as an
//! occluder only — pushing two polygons apart needs a separating axis and
//! a contact normal worth the name, which is a step of its own.
//!
//! This is a vocabulary for a document designed for additions. The
//! sidecar round-trips through `sprite_util` with its comments and its
//! unknown keys intact — the tool interprets `atlas` and two-number
//! structs and preserves everything else — so `shapes:` joins `flower_anchors`
//! and friends as just another key the tool will not touch, and the next
//! key can arrive the same way. Two rules get rediscovered expensively
//! otherwise, so they are stated here where the reader can also enforce
//! them:
//!
//! * **Positions are arrays.** `at: [16.0, 8.0]`, never `at: (16.0, 8.0)`:
//!   `sprite_util` reads any struct with exactly two numeric fields as a
//!   draggable position spot, so a tuple is a handle the tool lets you
//!   grab and a shape whose numbers move under the author's hands. The
//!   reader refuses a tuple with exactly this explanation.
//! * **Errors are returned, never logged.** The sidecar's standing policy
//!   — an unparseable file: the sprite simply loads — is right for an
//!   atlas grid and wrong for hit boxes: a sprite that silently loads with
//!   no collision is a bug nobody can see by playing. [`authored_shapes`]
//!   hands the caller every error it found, all of them in one pass, and
//!   the caller decides whether the sprite loads; the reader never
//!   logs-and-continues.
//!
//! Authored numbers are in the texture's pixels: x right, y *down*, origin
//! at the image's top-left. The engine's footprints stand centred on the
//! node in user space, which is y-*up*, one unit per pixel — so the flip
//! happens here, in [`pixel_to_node`] and nowhere else, the way
//! `docs/authored-shapes.md` records it must.
//! A polygon is convex or refused: [`Convex::new`] rejects a concave or
//! self-crossing outline, and this reader passes that refusal straight
//! through as its own message — nothing is decomposed, and nothing is
//! quietly hulled.

use crate::bake::{Feature, Footprint, POLY_NOT_PUSHED};
use crate::collision::{Convex, MAX_PLANES};
use crate::ron::{Doc, Kind, Val};

/// Every error one reading of a sidecar produced, at once.
///
/// An author fixing a sidecar wants the whole list, not the first
/// complaint and a rebuild per round trip, so [`authored_shapes`] reports
/// every defect it finds in the document in a single pass. The crate's
/// standing policy for sidecars is that a bad file still loads the
/// sprite; that is the right call for an atlas grid and the wrong one for
/// a hit box, so this travels to the caller, who owns the decision — a
/// missing hit box is invisible in play, and this must never be decided
/// by a quiet log line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[must_use = "an unreadable shape is authored geometry that does nothing; name the errors rather than dropping them"]
pub struct ShapeErrors {
    /// Every complaint, in document order: whole errors first, then each
    /// entry's own.
    pub errors: Vec<ShapeError>,
}

impl ShapeErrors {
    /// How many errors the reading found.
    pub fn len(&self) -> usize {
        self.errors.len()
    }

    /// Whether the reading found nothing to complain about.
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }
}

impl std::fmt::Display for ShapeErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, err) in self.errors.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{err}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ShapeErrors {}

/// One thing about the sidecar's `shapes` the reader could not make a
/// feature from, in the voice of [`crate::bake::Rejection`]: a sentence
/// that names where it came from, which an author can act on and a test
/// can name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeError {
    /// Which entry said it: `None` is a defect of the `shapes` key
    /// itself, `Some(i)` the `i`-th entry of the list. The RON tree is a
    /// preservation tree and carries no source lines, so the entry's
    /// position is the finest address there is to give.
    pub entry: Option<usize>,
    /// The complaint as a sentence, prefixed with where it came from.
    pub message: String,
}

impl std::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The names an entry may use, and nothing else: the vocabulary *inside*
/// an entry is closed, because the tool preserves faithfully whatever it
/// does not know, so a `solidss:` key would be kept forever and never
/// read — the mistake has to be caught while it can still be fixed.
const KNOWN_KEYS: [&str; 8] = [
    "kind", "at", "size", "radius", "points", "solid", "occludes", "hit",
];

/// Read the `shapes` key of an already-parsed sidecar into the features a
/// [`Baker`](crate::Baker) takes, with the authored pixels turned into the node's
/// space.
///
/// Opening the file is the caller's job — this takes the parsed
/// [`Doc`], never a path — and so is deciding what a refusal means: the
/// `Ok` arm carries every entry the document got right, the `Err` arm
/// every defect it got wrong, *all of them*, because an author fixing a
/// sidecar wants the whole list at once.
///
/// `size_px` is the image the shapes were drawn on, `[width, height]` in
/// pixels: authored positions are pixels of that image with y down from
/// its top-left, and the conversion lands each shape centred on the node,
/// y up.
///
/// No `shapes` key, or an empty list, is not an error and returns nothing
/// — which is where a caller's derive-from-bounds default takes over: a
/// sprite or tile with no authored shapes derives them from the bounds it
/// already has. The key is authoritative only once it is present, because
/// the moment an author draws a shape the picture has stopped describing
/// it.
pub fn authored_shapes(doc: &Doc, size_px: [f32; 2]) -> Result<Vec<Feature>, ShapeErrors> {
    let mut errors = Vec::new();
    let Some(shapes) = top_level_key(&doc.root, "shapes") else {
        return Ok(Vec::new());
    };
    let Val::Seq { items, .. } = shapes else {
        errors.push(problem(
            None,
            "the value of `shapes` should be a list of entries like (kind: \"rect\", ..)",
        ));
        return Err(ShapeErrors { errors });
    };
    let mut features = Vec::new();
    for (index, item) in items.iter().enumerate() {
        features.extend(read_entry(index, &item.val, size_px, &mut errors));
    }
    if errors.is_empty() {
        Ok(features)
    } else {
        Err(ShapeErrors { errors })
    }
}

/// Where one point authored in the texture's pixels stands in the node's
/// space: pixel `(x, y)` of a `w × h` image becomes
/// `(x - w / 2, h / 2 - y)` — centred on the node, one unit per pixel,
/// and y flipped from the image's down to the engine's up.
///
/// This is the whole conversion and the only place the flip lives, so it
/// cannot be applied for one shape and skipped for another; polygon
/// corners go through it like every other position. The flip reverses a
/// winding — a corner order that runs counter-clockwise in pixels runs
/// clockwise in node space — and nothing here fixes that, because
/// [`Convex::new`] infers the winding from the signed area instead of
/// assuming it.
fn pixel_to_node(px: [f32; 2], size_px: [f32; 2]) -> [f32; 2] {
    [px[0] - size_px[0] / 2.0, size_px[1] / 2.0 - px[1]]
}

/// The value of `name` under the document's root, whether the root is a
/// struct (the shape a sidecar has) or a map, whose keys the parser
/// renders to display strings — a quoted key arrives quoted.
fn top_level_key<'v>(root: &'v Val, name: &str) -> Option<&'v Val> {
    match root {
        Val::Struct { fields, .. } => fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, it)| &it.val),
        Val::Map { entries, .. } => entries
            .iter()
            .find(|(k, _)| k == name || k == &format!("\"{name}\""))
            .map(|(_, it)| &it.val),
        _ => None,
    }
}

/// Read one entry of the list, collecting every complaint it earns into
/// `errors` and returning the features it deserves — none at all if any
/// of them structural, because a half-read shape is the silently-wrong
/// kind this reader exists to stop.
fn read_entry(
    index: usize,
    entry: &Val,
    size_px: [f32; 2],
    errors: &mut Vec<ShapeError>,
) -> Vec<Feature> {
    let start = errors.len();
    let Val::Struct { fields, .. } = entry else {
        errors.push(problem(
            Some(index),
            "an entry is a struct like (kind: \"rect\", at: [16.0, 8.0], size: [24.0, 16.0], solid: true)",
        ));
        return Vec::new();
    };
    let field = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, it)| &it.val);

    // A closed vocabulary: what the tool preserves, this reader must at
    // least notice it was not asked for.
    for (name, _) in fields {
        if name.is_empty() {
            errors.push(problem(
                Some(index),
                "every field of an entry is named, `key: value`",
            ));
        } else if !KNOWN_KEYS.contains(&name.as_str()) {
            errors.push(problem(
                Some(index),
                format!(
                    "unknown key `{name}`: an entry knows kind, at, size, radius, points, solid, occludes and hit, and nothing else"
                ),
            ));
        }
    }

    let kind = match field("kind") {
        Some(v) => match word(v) {
            Some(k @ ("rect" | "circle" | "poly" | "capsule")) => Some(k),
            Some(other) => {
                errors.push(problem(
                    Some(index),
                    format!("kind `{other}` is not a shape: the vocabulary is rect, circle, poly (capsule is not implemented yet)"),
                ));
                None
            }
            None => {
                errors.push(problem(
                    Some(index),
                    "kind is a quoted word: \"rect\", \"circle\" or \"poly\"",
                ));
                None
            }
        },
        None => {
            errors.push(problem(
                Some(index),
                "no kind: an entry says what it is, \"rect\", \"circle\" or \"poly\"",
            ));
            None
        }
    };
    if kind == Some("capsule") {
        errors.push(problem(
            Some(index),
            "kind `capsule` is refused: capsules are in the vocabulary but implemented nowhere yet, so there is no shape to make of the entry",
        ));
        return Vec::new();
    }

    let mut at_px = None;
    match field("at") {
        None => errors.push(problem(
            Some(index),
            "no `at`: a shape stands at its centre, [x, y] in the image's pixels",
        )),
        Some(v) => {
            if let Some(p) = pair(v) {
                at_px = Some(p);
            } else if is_two_number_struct(v) {
                errors.push(problem(
                    Some(index),
                    "`at` is a struct with two numbers, which sprite_util reads as a draggable position spot and lets you move: a shape that slides under the author's hands is the bug, so authored positions are arrays, [x, y], and stay where they were written",
                ));
            } else {
                errors.push(problem(
                    Some(index),
                    "`at` is an array of two numbers, [x, y], in the image's pixels",
                ));
            }
        }
    }

    // The geometry, per kind. Each kind's own key or nothing else: the
    // hints name what was found when the document confused one for the
    // other, because "a circle needs a radius, found `size`" is the
    // error that teaches the vocabulary.
    let mut shape = None;
    match (kind, at_px) {
        (Some("rect"), Some(at)) => match field("size").and_then(pair) {
            Some(size) => {
                // Full pixels in; half-extents are the engine's convention,
                // taken up here and nowhere else.
                shape = Some(Footprint::rect(
                    pixel_to_node(at, size_px),
                    [size[0] / 2.0, size[1] / 2.0],
                ));
            }
            None if field("size").is_some() => errors.push(problem(
                Some(index),
                "`size` is an array of two numbers, [width, height] in full pixels; half-extents are the engine's, not the author's",
            )),
            None => errors.push(problem(
                Some(index),
                "a rect needs `size`: [width, height] in full pixels",
            )),
        },
        (Some("circle"), Some(at)) => match field("radius").and_then(number) {
            Some(radius) => shape = Some(Footprint::disc(pixel_to_node(at, size_px), radius)),
            None if field("radius").is_some() => errors.push(problem(
                Some(index),
                "`radius` is a single number, in pixels",
            )),
            None if field("size").is_some() => errors.push(problem(
                Some(index),
                "a circle needs `radius`: found `size` — the round shape wants one number",
            )),
            None => errors.push(problem(
                Some(index),
                "a circle needs `radius`: one number, in pixels",
            )),
        },
        (Some("poly"), Some(at)) => shape = read_poly(index, at, field("points"), size_px, errors),
        // `kind` and `at` each reported their own problem already.
        (Some(_), None) | (None, _) => {}
        // `capsule` returned above; nothing else is a known kind.
        (Some(_), Some(_)) => {}
    }

    // The three questions, on the same entry: they are what a consumer
    // asks of a shape, not properties of the drawing.
    let mut bounce = 0.0;
    let mut solid = false;
    match field("solid") {
        None => {}
        Some(Val::Struct { fields: sf, .. }) => {
            let bounces = sf.len() == 1 && sf[0].0 == "bounce";
            match bounces.then(|| number(&sf[0].1.val)).flatten() {
                Some(b) => {
                    bounce = b;
                    solid = true;
                }
                None => errors.push(problem(
                    Some(index),
                    "`solid` is `true`, `false`, or `(bounce: <number>)` — a material with nothing in it is no material",
                )),
            }
        }
        Some(v) => match boolean(v) {
            Some(b) => solid = b,
            None => errors.push(problem(
                Some(index),
                "`solid` is `true`, `false`, or `(bounce: <number>)`",
            )),
        },
    }
    let occludes = flag(field("occludes"), "occludes", index, errors);
    let mut hit = flag(field("hit"), "hit", index, errors);

    // A polygon occludes; the roles that push bodies apart get the bake's
    // own reason, quoted from the same constant so both ends say it
    // alike. The entry's other roles still stand: a wall authored solid
    // *and* occluding keeps the shadow while the author learns the rest.
    let mut refused = false;
    if kind == Some("poly") {
        if solid {
            errors.push(problem(Some(index), POLY_NOT_PUSHED));
            solid = false;
            refused = true;
        }
        if hit {
            errors.push(problem(Some(index), POLY_NOT_PUSHED));
            hit = false;
            refused = true;
        }
    }

    let wanted_any = solid || occludes || hit;
    if !wanted_any && !refused {
        errors.push(problem(
            Some(index),
            "`solid`, `occludes` and `hit` are all off: an authored shape nobody wants is a mistake, not a comment",
        ));
    }

    if errors.len() > start {
        return Vec::new();
    }
    let Some(shape) = shape else {
        // Unreachable: every arm that leaves `shape` unset pushed an
        // error above. Left as a fall-through rather than an unwrap,
        // because an unreadable entry staying silent about a feature is
        // worse than the reader trusting itself one level too far.
        return Vec::new();
    };
    let mut made = Vec::new();
    if solid {
        made.push(Feature::solid(shape, bounce));
    }
    if occludes {
        made.push(Feature::occluding(shape));
    }
    if hit {
        made.push(Feature::hit(shape));
    }
    made
}

/// The polygon's outline: the authored corners, flipped into the node's
/// space through the one conversion, handed to [`Convex::new`] — whose
/// refusal is reported as the concavity it means rather than repaired.
fn read_poly(
    index: usize,
    at_px: [f32; 2],
    points: Option<&Val>,
    size_px: [f32; 2],
    errors: &mut Vec<ShapeError>,
) -> Option<Footprint> {
    let Some(Val::Seq { items, .. }) = points else {
        errors.push(problem(
            Some(index),
            "a poly needs `points`: an array of [x, y] corners in the image's pixels, around the entry's `at`",
        ));
        return None;
    };
    let mut corners: Vec<[f32; 2]> = Vec::with_capacity(items.len());
    for item in items {
        let Some(p) = pair(&item.val) else {
            errors.push(problem(
                Some(index),
                "a poly's corners are arrays [x, y] of two numbers",
            ));
            return None;
        };
        corners.push(p);
    }
    if corners.len() < 3 {
        errors.push(problem(
            Some(index),
            format!(
                "a poly needs at least three corners; this outline has {}",
                corners.len()
            ),
        ));
        return None;
    }
    if corners.len() > MAX_PLANES {
        errors.push(problem(
            Some(index),
            format!(
                "that outline has {} corners and the cap is {MAX_PLANES}: the GPU pays per edge, per shadow ray, per penumbra tap, so a wide shape is the author's to simplify",
                corners.len()
            ),
        ));
        return None;
    }
    // The corners travel through the same single conversion as every
    // other authored position, `at` included: a polygon's points are
    // relative to the entry's centre, and adding before flipping keeps
    // the flip the only translation between the two spaces. The flip
    // reverses the winding; `Convex::new` infers winding from the signed
    // area, so it is not corrected here.
    let moved: Vec<[f32; 2]> = corners
        .iter()
        .map(|p| pixel_to_node([at_px[0] + p[0], at_px[1] + p[1]], size_px))
        .collect();
    match Convex::new(&moved) {
        Some(outline) => Some(Footprint::poly(outline)),
        None => {
            errors.push(problem(
                Some(index),
                "that outline is concave or self-crossing: a polygon is convex or refused — it is not decomposed, and not hulled",
            ));
            None
        }
    }
}

/// A `true` / `false` role flag: absent is off, and anything that is not
/// a bool says so rather than being guessed at.
fn flag(value: Option<&Val>, name: &str, index: usize, errors: &mut Vec<ShapeError>) -> bool {
    match value {
        None => false,
        Some(v) => match boolean(v) {
            Some(b) => b,
            None => {
                errors.push(problem(
                    Some(index),
                    format!("`{name}` is `true` or `false`"),
                ));
                false
            }
        },
    }
}

/// A number, from the text the parser kept verbatim — underscores are
/// digit separators and belong out of the way before the parse.
fn number(v: &Val) -> Option<f32> {
    match v {
        Val::Atom(t, Kind::Num) => t.replace('_', "").parse().ok(),
        _ => None,
    }
}

/// A bool, by kind: the parser calls `true` and `false` bools, and a
/// `1.0` where a flag belongs stays the error it is.
fn boolean(v: &Val) -> Option<bool> {
    match v {
        Val::Atom(t, Kind::Bool) => match t.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// A quoted word, quotes off — the text arrives verbatim from the source.
fn word(v: &Val) -> Option<&str> {
    match v {
        Val::Atom(t, Kind::Str) => t.strip_prefix('"')?.strip_suffix('"'),
        _ => None,
    }
}

/// An array of exactly two numbers: every position the vocabulary takes.
fn pair(v: &Val) -> Option<[f32; 2]> {
    let Val::Seq { items, .. } = v else {
        return None;
    };
    if items.len() != 2 {
        return None;
    }
    Some([number(&items[0].val)?, number(&items[1].val)?])
}

/// Whether this is what `sprite_util` would grab as a draggable position
/// spot: a struct with exactly two numeric-atom fields, named or not.
fn is_two_number_struct(v: &Val) -> bool {
    matches!(v, Val::Struct { fields, .. }
        if fields.len() == 2 && fields.iter().all(|(_, it)| matches!(&it.val, Val::Atom(_, Kind::Num))))
}

/// One complaint, in the voice of [`crate::bake::Rejection`], addressed
/// to the whole `shapes` key or to one entry of it.
fn problem(entry: Option<usize>, body: impl Into<String>) -> ShapeError {
    let body = body.into();
    let message = match entry {
        Some(i) => format!("shapes entry {i}: {body}"),
        None => format!("shapes: {body}"),
    };
    ShapeError { entry, message }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::{Baker, Placement};
    use crate::canvas::Canvas;
    use crate::collision::{Circle, Collider, DISC_SIDES};
    use crate::objects::Transform;
    use crate::ron;

    /// Read a sidecar written as text: every test starts where the tool
    /// hands the engine off, with a parsed `Doc` and a pixel size.
    fn read(text: &str, size_px: [f32; 2]) -> Result<Vec<Feature>, ShapeErrors> {
        let doc = ron::parse_doc(text).expect("the test's sidecar parses");
        authored_shapes(&doc, size_px)
    }

    /// Read, insisting the document was fine: a test that quietly read
    /// errors instead of features would be asserting nothing.
    fn read_ok(text: &str, size_px: [f32; 2]) -> Vec<Feature> {
        read(text, size_px).unwrap_or_else(|errs| panic!("expected features, got:\n{errs}"))
    }

    /// Read, insisting it refused, and hand back the whole report.
    fn read_err(text: &str, size_px: [f32; 2]) -> ShapeErrors {
        match read(text, size_px) {
            Ok(features) => panic!("expected refusals, got {features:?}"),
            Err(errs) => errs,
        }
    }

    /// The vocabulary's triangle entry, in a 32x16 image.
    const TRIANGLE: &str =
        "(kind: \"poly\", at: [0.0, 0.0], points: [[0.0, 0.0], [12.0, 4.0], [2.0, 14.0]]";
    const SHEET: [f32; 2] = [32.0, 16.0];

    #[test]
    fn a_top_left_pixel_lands_on_the_upper_side_of_the_node() {
        // The asymmetric case the seam exists for: pixels run y down from
        // the image's top-left, the node's space runs y up about its
        // centre, so the top-left corner is the *highest* point of the
        // node, not the lowest. Drop the flip and the y lands on -8.
        assert_eq!(pixel_to_node([0.0, 0.0], [32.0, 16.0]), [-16.0, 8.0]);
        assert_eq!(pixel_to_node([32.0, 16.0], [32.0, 16.0]), [16.0, -8.0]);
        assert_eq!(pixel_to_node([16.0, 8.0], [32.0, 16.0]), [0.0, 0.0]);
    }

    #[test]
    fn a_rect_entry_bakes_as_a_solid_at_its_place_in_the_nodes_space() {
        let features = read_ok(
            "(shapes: [(kind: \"rect\", at: [8.0, 4.0], size: [8.0, 4.0], solid: true)])",
            SHEET,
        );
        // The centre is the pixel [8, 4] of a 32x16 sheet: eight pixels
        // left of the middle and four above it; `size` is full extents,
        // so the half-extents are half of those.
        assert_eq!(
            features,
            vec![Feature::solid(
                Footprint::rect([-8.0, 4.0], [4.0, 2.0]),
                0.0
            )]
        );
        let mut baker = Baker::new();
        let rejected = baker.add(Placement::IDENTITY, &features);
        assert!(rejected.is_empty(), "the bake refused {rejected:?}");
        let baked = baker.finish();
        assert_eq!(baked.solids.len(), 1);
        assert_eq!(
            baked.solids[0].shape,
            Collider::Box(crate::collision::OrientedBox::new([-8.0, 4.0], [4.0, 2.0]))
        );
    }

    #[test]
    fn a_circle_flagged_occludes_stays_round_all_the_way_to_the_field() {
        // The reader's leg of the path the bake test
        // `an_occluding_circle_stays_round_all_the_way_to_the_field`
        // walks from a hand-built disc: an authored circle must reach the
        // occluder field as the polygon of a *circle at the flipped
        // centre*, never flattened or shifted anywhere upstream.
        let features = read_ok(
            "(shapes: [(kind: \"circle\", at: [16.0, 2.0], radius: 6.0, occludes: true)])",
            SHEET,
        );
        assert_eq!(
            features,
            vec![Feature::occluding(Footprint::disc([0.0, 6.0], 6.0))]
        );
        let mut baker = Baker::new();
        let rejected = baker.add(Placement::IDENTITY, &features);
        assert!(rejected.is_empty(), "the bake refused {rejected:?}");
        let baked = baker.finish();
        assert_eq!(
            baked.disc_occluders,
            [Circle {
                center: [0.0, 6.0],
                radius: 6.0
            }]
        );
        let mut canvas = Canvas::new((64, 64), 1.0);
        baked.declare_occluders(&mut canvas, Transform::identity());
        assert_eq!(canvas.occluders.len(), 1);
        let declared = canvas.occluders[0]
            .polygon
            .expect("a disc declares itself as a polygon");
        assert_eq!(declared.planes().len(), DISC_SIDES);
        assert_eq!(
            declared.bounds_as_box().center,
            [0.0, 6.0],
            "the polygon is the circle the author drew at the node's [0, 6]"
        );
    }

    #[test]
    fn a_polygon_flagged_occludes_reaches_the_field_as_its_own_edges() {
        // Declared, not packed through the backend from here — the
        // packing of these edges is `a_polygon_record_carries_its_edges_
        // and_the_next_record_follows_them`'s claim over in
        // `src/backend/tests/fields.rs`; what this test owns is that the
        // authored triangle arrives at the field's input as itself.
        let features = read_ok(&format!("(shapes: [{TRIANGLE}, occludes: true)])"), SHEET);
        let want = Convex::new(&[[-16.0, 8.0], [-4.0, 4.0], [-14.0, -6.0]])
            .expect("the authored triangle is convex");
        let mut baker = Baker::new();
        let rejected = baker.add(Placement::IDENTITY, &features);
        assert!(rejected.is_empty(), "the bake refused {rejected:?}");
        let baked = baker.finish();
        assert_eq!(baked.poly_occluders.len(), 1);
        let mut canvas = Canvas::new((64, 64), 1.0);
        baked.declare_occluders(&mut canvas, Transform::identity());
        assert_eq!(canvas.occluders.len(), 1);
        let declared = canvas.occluders[0]
            .polygon
            .expect("a polygon occluder carries edges of its own");
        assert_eq!(
            declared.planes(),
            want.planes(),
            "the field's input holds the triangle's three half-planes, \
             flipped into node space and no one's decomposition"
        );
    }

    #[test]
    fn a_polygon_asked_for_solid_is_refused_with_the_separating_axis_reason() {
        let errs = read_err(&format!("(shapes: [{TRIANGLE}, solid: true)])"), SHEET);
        assert_eq!(errs.len(), 1, "the whole report was:\n{errs}");
        assert!(
            errs.errors[0].message.contains("separating axis"),
            "the refusal must name what is missing, got {}",
            errs.errors[0]
        );
    }

    #[test]
    fn a_concave_outline_is_refused_and_not_repaired() {
        // An L drawn in pixels: `Convex::new` refuses it and the reader
        // says so — no decomposition, no hull, nothing shipped.
        let errs = read_err(
            "(shapes: [(kind: \"poly\", at: [0.0, 0.0], points: [[0.0, 0.0], [40.0, 0.0], [40.0, 20.0], [20.0, 20.0], [20.0, 40.0], [0.0, 40.0]], occludes: true)])",
            [64.0, 64.0],
        );
        assert_eq!(errs.len(), 1, "the whole report was:\n{errs}");
        assert!(
            errs.errors[0].message.contains("concave"),
            "the refusal should say the outline is concave, got {}",
            errs.errors[0]
        );
    }

    #[test]
    fn a_tuple_position_explains_the_tools_spot_rule() {
        let errs = read_err(
            "(shapes: [(kind: \"rect\", at: (4.0, 2.0), size: [8.0, 4.0], solid: true)])",
            SHEET,
        );
        assert_eq!(errs.len(), 1, "the whole report was:\n{errs}");
        let message = &errs.errors[0].message;
        assert!(
            message.contains("spot"),
            "no spot-rule explanation: {message}"
        );
        assert!(
            message.contains("array"),
            "the explanation should say what to write instead: {message}"
        );
    }

    #[test]
    fn an_entry_with_every_flag_off_is_refused() {
        // Twice over: explicitly false, and simply saying nothing. An
        // authored shape nobody wants is a mistake, not a comment.
        let errs = read_err(
            "(shapes: [
                (kind: \"rect\", at: [8.0, 4.0], size: [8.0, 4.0], solid: false, occludes: false, hit: false),
                (kind: \"rect\", at: [8.0, 4.0], size: [8.0, 4.0]),
            ])",
            SHEET,
        );
        assert_eq!(errs.len(), 2, "the whole report was:\n{errs}");
        for err in &errs.errors {
            assert!(err.message.contains("all off"), "{err}");
        }
    }

    #[test]
    fn an_unknown_key_inside_an_entry_is_refused() {
        let errs = read_err(
            "(shapes: [(kind: \"rect\", at: [8.0, 4.0], size: [8.0, 4.0], solid: true, solidss: true)])",
            SHEET,
        );
        assert_eq!(errs.len(), 1, "the whole report was:\n{errs}");
        assert!(
            errs.errors[0].message.contains("`solidss`"),
            "the refusal should name the key, got {}",
            errs.errors[0]
        );
    }

    #[test]
    fn an_unknown_key_outside_the_shapes_list_is_left_alone() {
        // The sidecar legitimately carries keys no Rust code reads; the
        // tool preserves them and this reader has no opinion on them.
        let features = read_ok(
            "(
                flower_anchors: [(317.0, 671.0)],
                shapes: [(kind: \"rect\", at: [8.0, 4.0], size: [8.0, 4.0], solid: true)],
            )",
            SHEET,
        );
        assert_eq!(features.len(), 1);
    }

    #[test]
    fn a_capsule_is_refused_as_unimplemented_and_the_reader_does_not_panic() {
        let errs = read_err(
            "(shapes: [(kind: \"capsule\", at: [8.0, 4.0], radius: 3.0, solid: true)])",
            SHEET,
        );
        assert_eq!(errs.len(), 1, "the whole report was:\n{errs}");
        assert!(errs.errors[0].message.contains("capsule"));
    }

    #[test]
    fn every_defect_in_the_document_is_reported_not_just_the_first() {
        // One entry with two defects and a second bad entry behind it:
        // the author fixing this file wants the whole list now.
        let errs = read_err(
            "(shapes: [
                (kind: \"rect\", at: [8.0, 4.0], size: [16.0], solid: true, hit: 7.0),
                (kind: \"blob\", at: [1.0, 2.0], occludes: true),
            ])",
            SHEET,
        );
        assert_eq!(errs.len(), 3, "the whole report was:\n{errs}");
        assert!(
            errs.errors[0].message.contains("`size`"),
            "{}",
            errs.errors[0]
        );
        assert!(
            errs.errors[1].message.contains("`hit`"),
            "{}",
            errs.errors[1]
        );
        assert_eq!(errs.errors[2].entry, Some(1));
    }

    #[test]
    fn no_shapes_key_at_all_is_not_an_error() {
        // Absent and empty both mean "nothing authored" — where the
        // caller's derive-from-bounds default takes over.
        let plain = read_ok("(atlas: [4, 4])", SHEET);
        assert!(plain.is_empty());
        let empty = read_ok("(shapes: [])", SHEET);
        assert!(empty.is_empty());
    }

    #[test]
    fn a_whole_sidecar_read_end_to_end_declares_the_authored_field() {
        // The seam's proof, all four vocabulary lines at once: text as
        // an author writes it (comments and foreign keys included),
        // through `parse_doc`, through `authored_shapes`, through a real
        // `Baker`, out through `declare_occluders` — and the field the
        // GPU would receive is what the sidecar said.
        let features = read_ok(
            "// garden sprite
            (
                flower_anchors: [(317.0, 671.0)],
                shapes: [
                    (kind: \"rect\", at: [16.0, 8.0], size: [24.0, 16.0], solid: true),
                    (kind: \"circle\", at: [16.0, 2.0], radius: 6.0, occludes: true),
                    (kind: \"poly\", at: [0.0, 0.0], points: [[0.0, 0.0], [12.0, 4.0], [2.0, 14.0]], occludes: true),
                    // one wall, both jobs: bouncy, and it throws a shadow
                    (kind: \"rect\", at: [16.0, 30.0], size: [32.0, 8.0], solid: (bounce: 0.2), occludes: true),
                ],
            )",
            [32.0, 32.0],
        );
        assert_eq!(
            features.len(),
            5,
            "one solid, two round/poly shadows, and one wall both ways"
        );

        let mut baker = Baker::new();
        let rejected = baker.add(Placement::IDENTITY, &features);
        assert!(rejected.is_empty(), "the bake refused {rejected:?}");
        let baked = baker.finish();
        assert_eq!(baked.solids.len(), 2);
        let bouncy = baked.solids.iter().find(|s| s.bounce == 0.2);
        assert!(bouncy.is_some(), "the (bounce: 0.2) material rode along");
        assert_eq!(baked.occluders.len(), 1, "one occluding rectangle");
        assert_eq!(baked.occluders[0].center, [0.0, -14.0]);
        assert_eq!(baked.disc_occluders.len(), 1);
        assert_eq!(baked.poly_occluders.len(), 1);
        assert!(baked.hits.is_empty());

        let mut canvas = Canvas::new((64, 64), 1.0);
        baked.declare_occluders(&mut canvas, Transform::identity());
        assert_eq!(
            canvas.occluders.len(),
            3,
            "the box record, the cut circle, and the authored triangle"
        );
        assert!(canvas.occluders[0].polygon.is_none());
        assert_eq!(canvas.occluders[0].half, [16.0, 4.0]);
        assert_eq!(
            canvas.occluders[1].polygon.as_ref().unwrap().planes().len(),
            DISC_SIDES
        );
        let want = Convex::new(&[[-16.0, 16.0], [-4.0, 12.0], [-14.0, 2.0]])
            .expect("the authored triangle is convex");
        let triangle = canvas.occluders[2]
            .polygon
            .as_ref()
            .expect("the polygon occluder carries its own edges");
        assert_eq!(triangle.planes(), want.planes());
    }
}
