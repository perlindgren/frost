# Authored shapes: hit boxes, occluders, and the sidecar that carries them

> **Status.** The *engine* half has shipped: [`src/collision.rs`](../src/collision.rs)
> answers every question a shape can be asked (`OrientedBox`, `Circle`,
> `Collider`, and `Convex` — a convex polygon held as its edges' half-planes),
> [`src/bake.rs`](../src/bake.rs) turns authored features into
> [`Baked`](../src/bake.rs) with per-consumer fusion, the occluder field carries
> polygons as variable-stride records, and
> [`src/backend/tests/shadows.rs`](../src/backend/tests/shadows.rs) proves the
> GPU shadows exactly where the CPU says it should. What has **not** shipped is
> the half that lets an author *say* any of this: there is no textual schema for
> a shape, so a shape can only reach the engine from code — a
> `Canvas::occluder` / `occluder_polygon` call, or a `Baker` fed by hand. This
> document records what is already true of the files involved and what has been
> decided, so the next step does not have to rediscover either.

## The sidecar already exists

A sprite's `<name>.png` may have a `<name>.ron` beside it. `sprite_util` reads
it on load ([`examples/sprite_util.rs:3911`](../examples/sprite_util.rs#L3911))
and rewrites it on save ([` :1560 `](../examples/sprite_util.rs#L1560)), and it
is a far better host for shape data than a new file would be:

- **Comments and unknown keys survive a round-trip.** The reader is a
  hand-written RON-subset tree now living in the crate as
  [`src/ron.rs`](../src/ron.rs) (`frost::ron`, consumed by both
  examples) that stores every field verbatim
  with its surrounding comment runs, and writes them back
  (`to_text_doc`).
  `sprite_util` itself interprets exactly two things — `atlas`, and any struct
  with two numeric fields — and passes everything else through untouched.
  Canonicalisation is a fixed point, tested against a real shipped sidecar
  (`a_real_plant_sidecar_round_trips_with_its_comments`,
  [`:6098`](../examples/sprite_util.rs#L6098);
  `the_canonical_sidecar_is_a_round_trip_fixed_point`,
  [`:6301`](../examples/sprite_util.rs#L6301)).
- **It is already extended by hand.** The keys in the shipped sidecars are
  `atlas`, `segment`, `image`, `lower_anchor`, `upper_anchor`, `flower_anchors`
  — none of them written by any Rust code, all of them read by *nobody* in the
  engine. A `shapes: [...]` key joins them as just another part of the document
  the tool will not touch.

Two things follow from that. The work is not "invent a format" but "add a
vocabulary to a document designed for additions". And the **location**
blocker is gone: the parser moved into the crate as `src/ron.rs`, split as
a move plus a seam — `Val`, `Item`, `parse_doc`, `to_text` are the engine's;
`Row`, `flatten`, `layout`, `VKind` belong to the viewer and live in
`examples/ron_view/view.rs` (`toggle`, `open_to`, `preview`, `inline`
stayed in the crate as methods on `Val` — Rust's orphan rule leaves no
other home for them).

## Three constraints the file imposes

**Positions are arrays, never tuples.** `sprite_util` treats any *struct with
exactly two numeric-atom fields* as a draggable position marker
(`spot_coords`, [`:4130`](../examples/sprite_util.rs#L4130), capped at
`SPOTS_MAX`). That is why `atlas` is deliberately an array, per the comment at
[`:4021`](../examples/sprite_util.rs#L4021): *"a two-number tuple would read as a
position spot"*. So `center: [4.0, 2.0]` stays data, while `center: (4.0, 2.0)`
becomes a handle the tool lets you drag — a shape whose numbers move under the
author's hands, which is the class of bug the rest of this document is about.
When the tool learns to drag *shapes*, that arrives as an explicit feature over
a known key, not as a coincidence of arity.

**"An unparseable sidecar: the sprite simply loads" is wrong for shapes.** That
is the existing policy ([`:3918`](../examples/sprite_util.rs#L3918): `log::warn!`
then load without it), and for an atlas grid it is right — a missing grid is a
whole-sprite sprite. A hit box is different: a sprite that silently loads with
*no* collision is a bug no one can see by playing, which is the exact failure
`Baker::add` answers with a `#[must_use]` list of [`Rejection`](../src/bake.rs)s.
So the reader returns a typed error for a malformed shape block and the *caller*
chooses; the engine must not be the place that quietly decides a character has
no body today.

**A sidecar shape is drawn in the texture's pixels; a baked shape stands in the
node's space.** `sprite_util` paints coordinates right-down and y-*down* from the
top-left of the PNG; [`Footprint`](../src/bake.rs) is centred on the node in user
space, which is y-*up* — the same flip that reverses a polygon's winding on the
way to a framebuffer, and the same one
[`shadows.rs`](../src/backend/tests/shadows.rs) exists to catch. The conversion
belongs in the reader, in one function, with a test that would fail if it were
skipped. Do not "fix" it by authoring in pixel space throughout: keeping shapes
local to their own space is invariant 9, and it is why a sheared instance still
throws square-cornered shadows.

## The vocabulary (proposed, not yet agreed in name)

One authored shape is one `Feature` in the engine's sense — a role, a footprint,
and the placement that puts it in the node — so the text says the same three
things. Field *names* below are a proposal; the three parts are not negotiable,
because they are what `Baker` consumes:

```ron
shapes: [
    (role: "solid", kind: "rect", at: [16.0, 8.0], size: [24.0, 16.0]),
    (role: "occlude", kind: "circle", at: [16.0, 2.0], radius: 6.0),
    (role: "hit", kind: "poly", at: [0.0, 0.0], points: [[0.0, 0.0], [12.0, 4.0], [2.0, 14.0]]),
]
```

- `role` is `occlude`, `solid` or `hit` — [`Role`](../src/bake.rs), unchanged,
  because it is the question the shape answers, not a property of the drawing.
  `solid` carries an optional `bounce` (a [`Material`](../src/bake.rs)).
- `kind` is `rect`, `circle`, `capsule` or `poly`. `capsule` is not implemented
  anywhere yet; it is in the grammar so adding it is not a format change.
- **Derive from the object, allow the override.** With no `shapes` key a sprite
  or tile derives its shapes from what it already is — its bounds — which is
  what makes the common case free. With the key present it is authoritative,
  because the moment an author draws a shape the picture has stopped describing
  it. This mirrors how a declared occluder joins the painted ones rather than
  replacing them.
- A polygon is **convex or refused**. [`Convex::new`](../src/collision.rs)
  rejects a concave outline rather than decomposing it, and `sprite_util` will
  refuse to accept one for the same reason: a decomposition is a decision the
  renderer would make silently, on the author's behalf. Sixteen edges is the
  cap, because the GPU cost is per edge, per shadow ray, per penumbra tap.

**Keyed by tile id, when tiles have a file.** `tiles: { 3: [...], 17: [...] }`
was agreed over a positional list, and it stays right — a map edit that inserts
a column must not silently move every shape in the level. But it cannot be built
now, because tile maps have *no file format at all*: `MapLayer`
([`:267`](../examples/sprite_util.rs#L267)) holds `cells: Vec<u32>` indexed by a
row-major formula
([`at`, `:4216`](../examples/sprite_util.rs#L4216)) and lives only in memory and
in undo snapshots. Keyed tile shapes arrive *with* the map format, as its second
key — inventing one to hold the other would smuggle a format decision in through
the back door.

## Order of work

1. **`tree.rs` into `src/`**, data separated from view, both examples
   re-pointed, its parser tests coming with it. Mechanical, and the only
   thing on this list that cannot be done incrementally. **Done:**
   `src/ron.rs` is the data (its tests with it), `examples/ron_view/view.rs`
   the row view the two viewers share.
2. **The reader**: the vocabulary above, in `src/`, producing `Feature`s for a
   `Baker` — plus the pixel-to-node conversion, the typed errors, and the
   round-trip and space tests. This is where the schema is finally agreed by
   being written down in Rust.
3. **`sprite_util` draws them**: rects, circles, polygons, with the convexity
   refusal and the existing `status:` line as the voice
   (`"shapes: that outline is concave"`).
4. **The map file**, with keyed tile shapes as part of it.
5. **Capsules**, and convex polygons as `solid` / `hit` — the latter needing the
   separating axis of a curved boundary and a contact normal worth the name.
   They occlude already; pushing things apart is the step that has not been
   taken.
