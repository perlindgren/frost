# Authored shapes: hit boxes, occluders, and the sidecar that carries them

> **Status.** The *engine* half has shipped: [`src/collision.rs`](../src/collision.rs)
> answers every question a shape can be asked (`OrientedBox`, `Circle`,
> `Collider`, and `Convex` — a convex polygon held as its edges' half-planes),
> [`src/bake.rs`](../src/bake.rs) turns authored features into
> [`Baked`](../src/bake.rs) with per-consumer fusion, the occluder field carries
> polygons as variable-stride records, and
> [`src/backend/tests/shadows.rs`](../src/backend/tests/shadows.rs) proves the
> GPU shadows exactly where the CPU says it should. The reader has shipped too
> ([step 2 below](#order-of-work)), in two arms: [`src/shapes.rs`](../src/shapes.rs)
> reads the vocabulary below out of a parsed `frost::ron` tree in one pass and
> hands back both `frost::read_shapes` — the entries in the texture's own
> pixels, what a drawing tool paints — and `frost::authored_shapes`, the
> [`Feature`](../src/bake.rs)s a [`Baker`](../src/bake.rs) takes, in the node's
> space — plus every error the document holds. The drawing half has shipped
> with it ([step 3 below](#order-of-work)): `sprite_util` overlays the shapes
> over the sprite and voices the refusals on its menu bar. The **load-time
> seam** has shipped too: [`bake_sidecar`](../src/shapes.rs) finds
> `<name>.ron` beside the `<name>.png` a game is about to draw, parses, reads
> and bakes it once — and a mis-authored sidecar *refuses the load* with the
> whole report, which is the policy the seam exists to make possible. Its
> twin `bake_sidecar_text` serves wasm games, whose files come out of the
> binary rather than a directory; a sprite without a sidecar bakes nothing,
> which is where a caller's derive-from-bounds default takes over. The
> `authored_hall` example ([`examples/physics/authored_hall.rs`](../examples/physics/authored_hall.rs))
> runs the seam once at boot and rides the one baked set onto every brick of
> a dark hall. What has *not* shipped is the map file (step 4) and shapes
> that push bodies around (step 5). This document records what is true of
> the files involved and what has been decided, so the next step does not
> have to rediscover either.

## The sidecar already exists

A sprite's `<name>.png` may have a `<name>.ron` beside it. `sprite_util` reads
it on load ([`tools/sprite_util/main.rs:3915`](../tools/sprite_util/main.rs#L3915))
and rewrites it on save ([`:1564`](../tools/sprite_util/main.rs#L1564)), and it
is a far better host for shape data than a new file would be:

- **Comments and unknown keys survive a round-trip.** The reader is a
  hand-written RON-subset tree now living in the crate as
  [`src/ron.rs`](../src/ron.rs) (`frost::ron`, consumed by both
  examples) that stores every field verbatim
  with its surrounding comment runs, and writes them back
  (`to_text_doc`).
  `sprite_util` itself interprets two things — `atlas`, and any struct with
  two numeric fields — and passes everything else through untouched; the
  `shapes:` key is now *read* for the overlay and its validation, but the
  save path never rewrites it, so the file round-trips with the same
  verbatim preservation as before.
  Canonicalisation is a fixed point, tested against a real shipped sidecar
  (`a_real_plant_sidecar_round_trips_with_its_comments`,
  [`:6147`](../tools/sprite_util/main.rs#L6147);
  `the_canonical_sidecar_is_a_round_trip_fixed_point`,
  [`:6350`](../tools/sprite_util/main.rs#L6350)).
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
(`spot_coords`, [`:4134`](../tools/sprite_util/main.rs#L4134), capped at
`SPOTS_MAX`). That is why `atlas` is deliberately an array, per the comment at
[`:4025`](../tools/sprite_util/main.rs#L4025): *"a two-number tuple would read as a
position spot"*. So `center: [4.0, 2.0]` stays data, while `center: (4.0, 2.0)`
becomes a handle the tool lets you drag — a shape whose numbers move under the
author's hands, which is the class of bug the rest of this document is about.
When the tool learns to drag *shapes*, that arrives as an explicit feature over
a known key, not as a coincidence of arity.

**"An unparseable sidecar: the sprite simply loads" is wrong for shapes.** That
is the existing policy ([`:3922`](../tools/sprite_util/main.rs#L3922): `log::warn!`
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

## The vocabulary

One authored shape is one `Feature` in the engine's sense — a role, a footprint,
and the placement that puts it in the node — so the text says the same three
things. The agreed shape of it is **one flat list, with the kind as a field and the
roles as flags on the same entry**. The alternative that looked best for a while
was a single `role` field, and the reason it lost is the commonest shape an
author draws: a wall is solid *and* occluding, and one role per entry means
writing its geometry twice, in two entries that will not stay identical. A
flag per consumer says what the shape actually is, and lets the reader make the
one-to-many step into [`Baker`](../src/bake.rs) — up to three `add` calls from
one authored entry — instead of pushing that split into the file. Kind is still
a string, so a misspelling is possible, which is exactly why the reader's errors
are typed rather than defaulted: a shape nobody typed correctly must fail
loudly, not vanish. A misspelling *inside* an entry is caught for the same
reason, and is the reason the roles are not top-level keys — the tool preserves
what it does not know, so a `solidss:` key would be kept faithfully and never
read.

```ron
shapes: [
    (kind: "rect", at: [16.0, 8.0], size: [24.0, 16.0], solid: true),
    (kind: "circle", at: [16.0, 2.0], radius: 6.0, occludes: true),
    (kind: "poly", at: [0.0, 0.0], points: [[0.0, 0.0], [12.0, 4.0], [2.0, 14.0]], occludes: true),
    // one wall, both jobs: bouncy, and it throws a shadow
    (kind: "rect", at: [16.0, 30.0], size: [32.0, 8.0], solid: (bounce: 0.2), occludes: true),
]
```

- The flags are the three questions of [`Role`](../src/bake.rs), unchanged,
  because they are what a consumer asks of a shape and not properties of the
  drawing. All three off is a refusal to accept the entry — an authored shape
  nobody wants is a mistake, not a comment. `solid` takes `true` or, when the
  author means a particular surface, `(bounce: 0.2)` for a
  [`Material`](../src/bake.rs).
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

**Frames are not in this yet.** A `shapes` list describes the sprite in texture
pixels and applies to every frame of it. A per-frame override was designed and
deliberately **not** reserved: the animation system is not in its final shape,
and a key invented to hold something the host does not yet model tends to be
unmade later at the cost of every file that used it. Adding `frames: {2: [...]}`
later is a compatible change, because the tool already preserves keys it does
not know.

**Not serde, at least not yet.** Two separate questions hide here. Could the
sidecar *be* serde's RON? No: `sprite_util` rewrites these files and must keep
comments, unknown keys and numbers exactly as written, and serde's model is
"deserialize into a type, serialize a fresh document" — comments and unknown
keys are gone by construction, which is the whole reason the hand-written tree
exists. Could the *engine* read the tree with serde, given a `Deserializer` over
`Val`? Yes, and it is the right refactor once there are several typed keys —
`#[derive(Deserialize)]` for physics, maps and animations is worth a few hundred
lines of Visitor dispatch, plus promoting serde from dev-dependency to a real
one. Not for this key: what matters here is errors with judgement in them
(*"occlude: a circle needs a radius, found `size`"*, *"that outline is
concave"*), and a direct read of `Val` produces those while serde's errors over
a dynamic tree are at their weakest. The option to refuse outright is the `ron`
crate engine-side while the tool keeps its own parser: two parsers with
different accepted subsets means the tool can canonicalise a file that then
means something else to the engine, since raw strings, tuples and `!Type`
markers are precisely where the two would part company. One parser, always.

**Keyed by tile id, when tiles have a file.** `tiles: { 3: [...], 17: [...] }`
was agreed over a positional list, and it stays right — a map edit that inserts
a column must not silently move every shape in the level. But it cannot be built
now, because tile maps have *no file format at all*: `MapLayer`
([`:271`](../tools/sprite_util/main.rs#L271)) holds `cells: Vec<u32>` indexed by a
row-major formula
([`at`, `:4220`](../tools/sprite_util/main.rs#L4220)) and lives only in memory and
in undo snapshots. Keyed tile shapes arrive *with* the map format, as its second
key — inventing one to hold the other would smuggle a format decision in through
the back door.

## Tilesets, and what a tool can promise

> **Decided.** A tileset is its own `.ron` file, and a tile map will be too —
> not a key on a sprite's sidecar, and not a new format. Both are written in the
> same RON subset the sidecars use ([`src/ron.rs`](../src/ron.rs)), so a map and
> a tileset get the same parser, the same comments surviving a rewrite, and the
> same errors with a line number in them; a second format would have been a
> second parser, and two parsers is the one thing this document rules out.
> `frost` loads both, since `frost` is the library and the tile tools are things
> it ships with.

A tileset is its own asset that **refers to a sprite** — a source path, a grid,
and the per-tile annotations — rather than being a key on the sprite's sidecar.
The older reading is the degenerate case of it: one tileset referring to one
sprite with a plain grid is exactly what `atlas` means today, so nothing has to
migrate. What the indirection buys is the only sensible home for keyed per-tile
shapes, and the ability to carve two sets out of one sheet. What it costs is a
reference that can go stale, which is the part worth deciding on purpose.

Tile identity is the whole of that problem. A flat id (`3`) breaks when the grid
arithmetic changes — change the column count and id 3 names different pixels. A
coordinate (`(3, 2)`) survives that and breaks when the sheet is relaid out. And
both survive the case that actually matters: an artist repaints tile 3 in place,
the reference stays valid, and every shape annotated on it is quietly wrong for
the new art. So the tileset carries explicit ids that the tool carries across a
relayout by matching positions, and records a digest per tile id.

The division of labour follows from what each side can actually see. At load
time the engine reports what is definitely broken: a tileset that does not
resolve, an id past the end of its tileset. The digest question is not a
load-time question at all — a repainted tile is not an error — so it belongs to
the tool, at the moment the author is looking: *"you changed tile 3; it has a
collision shape and four maps use it."* No editor can make that safe, and it is
worth being blunt about why: the file is not malformed, the reference is
correct, and the thing that changed is whether the art still suits its hit box.
That is a judgement, and a tool's job is to put the judgement in front of the
person making it.

## Order of work

1. **`tree.rs` into `src/`**, data separated from view, both examples
   re-pointed, its parser tests coming with it. Mechanical, and the only
   thing on this list that cannot be done incrementally. **Done:**
   `src/ron.rs` is the data (its tests with it), `examples/ron_view/view.rs`
   the row view the two viewers share.
2. **The reader**: the vocabulary above, in `src/`, reading `frost::ron::Val`
   directly (no serde) into `Feature`s for a `Baker` — plus the pixel-to-node
   conversion, the typed errors, and the round-trip and space tests. **Done:**
   `src/shapes.rs` is the reader — `authored_shapes` hands back the feature
   list or every error the document holds, the flip lives in one function
   pinned by its asymmetric test, and `Footprint::poly` carried the convex
   outline into `Baker`, as an occluder only, until step 5 gives the roles
   that push their separating axis.
3. **`sprite_util` draws them**: rects, circles, polygons, with the convexity
   refusal and the existing `status:` line as the voice
   (`"shapes: that outline is concave"`). **Done:** the tool reads the active
   sidecar's tree every frame with `frost::read_shapes` and strokes what it
   read in amber over the sprite — a rect by its four edges, a circle as the
   very `DISC_SIDES`-gon `Convex::disc` sends to the occluder field (the desk
   shows the edge the shadow will have), a polygon by its authored corners —
   all through the same texture→window map the position markers use, pinned
   against it by a test. An entry whose *geometry* read draws even when the
   reading refused something else about it, and the first refusal — with a
   count of its siblings — rides the menu bar's notes beside the view name,
   whether or not the View ▸ Shapes checkbox is drawing anything. Two things
   the step deliberately left out, for the same reason the step itself is
   only a drawing: **a crop does not shift authored positions** — the markers
   ride the cut, the shapes do not, so cropping drifts them off the art;
   visible with the overlay on, and to be decided with the tool's first move
   of authored numbers (which is also where dragging arrives). And no editing:
   the shapes are drawn, not grabbed.
4. **The map file**, with keyed tile shapes as part of it — promoted from a
   tool's convenience to shipped surface: `frost` is a library that comes with
   tile-set and tile-map tools (`docs/tooling.md`), so the format is something
   users write and the engine must load. That argues the map model itself moves
   into `src/` *before* the format is designed, rather than being promoted from
   a tool's editor state afterwards.
5. **Capsules**, and convex polygons as `solid` / `hit` — the latter needing the
   separating axis of a curved boundary and a contact normal worth the name.
   They occlude already; pushing things apart is the step that has not been
   taken.
