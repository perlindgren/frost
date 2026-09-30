# Shape examples

Eight demos of frost's basic and sprite shapes — `Shape::Background`,
`Shape::Rectangle`, `Shape::Circle`, and `Shape::Sprite` — built as a
progression: the smallest possible scene, immediate rectangle and circle
draws with explicit z-ordering, an animated scene tree and the same tree
refactored into a self-animating node, sprites with tint and alpha, a
two-sprite overlay aligned by an anchor point, and a plant assembled from
three chained sprite slices.

| Example | What it shows | Run with |
|---|---|---|
| `one_rect` | the smallest possible scene: a `Shape::Background` on the root node and one static `Shape::Rectangle` centered in the window — the user-space origin is the window center (y up), so `center: [0.0, 0.0]` sits in the middle of the screen — and a process that does nothing | `cargo run --example one_rect` |
| `box_z` | z-ordering on immediate draws: four overlapping squares drawn with `ctx.rectangle`, each with an explicit `z` — red and white tie at `0.0` (call order decides: white is the later call, so it is on top), blue at `1.0`, green at `2.0` above all | `cargo run --example box_z` |
| `circles` | immediate circle draws, redeclared each frame: a big orange circle at the window center with radius a quarter of the window height, and a smaller red circle 80 px in from the bottom-left corner — the window size read with `ctx.size()`, both at `z: 0.0` | `cargo run --example circles` |
| `scene` | a scene tree animated in place through `ctx.scene()`: the root eases from a point to full size (a smoothstep) as the orbiting rectangle first bottoms out, the rectangle orbits while it tumbles (a rotation composed over a translation), and a small circle riding the rectangle counter-rotates | `cargo run --example scene` |
| `satellite` | `scene` refactored: the orbiting, tumbling rectangle and its counter-rotating child become a `Satellite` node implementing `frost::Node`, which drives itself with one `visit()` per frame — no `dt` in `Node::process`, so the spin is a fixed step per frame (frame-rate dependent) — and its own scene is drawn into the frame with `ctx.draw_scene`; the rectangle's `order: 1.0` sorts it in front of the center circle | `cargo run --example satellite` |
| `sprites` | PNG sprites through the scene tree (`Shape::sprite`): a brick bobbing while it fades out and back in on a 1 s `Tween` alpha cycle, a button swaying and breathing in scale with its per-pixel tint drifting over time, and a small brick riding the button as a child node that counter-rotates | `cargo run --example sprites` |
| `tomato_sprite` | `tomato.png` with its `tomato_fg.png` foreground overlay as siblings under a shapeless dummy parent: each child puts the image pixel `(308, 411)` on the parent's origin via `Transform::anchor`, the parent carries the inverse so the 638×469 image is centered and fits the window, and both children share draw order `0.0` — the tie resolves in tree order, so the later sibling (the overlay) draws on top | `cargo run --example tomato_sprite` |
| `tomato` | a plant assembled from three sprite slices (`plant1.png` the base, `plant2.png` the middle, `plant3.png` the top) chained by hand-picked joints: each slice's lower joint sits exactly on the previous slice's upper joint, the pixel-space joints converted with `Transform::anchor`; the 969 px plant is scaled to fit the window and sways in a travelling wind — a base rock plus two phase-lagged bends, each a little stronger than the last, so the tip moves the most | `cargo run --example tomato` |

The eight files sit in this folder rather than at the examples' top level, so
each one is declared explicitly in `Cargo.toml` (cargo only auto-discovers
`examples/*.rs` and `examples/*/main.rs`); the example names are unchanged.
`button` and `widgets` stay at the top level — their topics are input and
audio and the `Ui` widget layer, not shapes.

## Patterns the examples share

- **Two ways to draw.** `one_rect`, `scene`, `sprites`, and the two tomato
  demos build their content once as a scene tree (`frost::Scene`) and pass
  it to `frost::run`, which draws it every frame; the process mutates the
  nodes' transforms in place through `ctx.scene()`. `box_z` and `circles`
  do the opposite: they redeclare their shapes each frame with immediate
  `ctx.rectangle` and `ctx.circle` calls inside the process. `satellite`
  mixes the two: its main scene stays in the tree, while the orbiting
  rectangle lives in a second scene that the process draws into the frame
  with `ctx.draw_scene`.
- **A node that animates itself.** `satellite` moves the animation out of
  the top-level process and into a `Satellite` node implementing
  `frost::Node`: one `visit()` per frame advances its spin and recomputes
  the orbit, tumble, and counter-rotation. Because `Node::process` gets no
  `dt`, the spin advances a fixed step per frame — 0.25 rad/s at 60 fps —
  so its speed is frame-rate dependent, unlike the `dt`-scaled spin in
  `scene`.
- **User space is window-centered, y up.** The origin is the window's
  center, so `one_rect`'s rectangle at `center: [0.0, 0.0]` is dead
  centered. A `Shape::Background` node's transform is ignored — it only
  supplies the window's clear color — so by convention it hangs on the
  root node.
- **Sprites are centered shapes.** `Shape::sprite` loads a PNG; the texture
  sits centered on its node's origin, one texture pixel per scene pixel, and
  the node's transform and scale apply to it exactly as they do to the other
  shapes. A sprite carries two per-pixel channels — `alpha` and `color`
  (a tint multiplied over every sampled pixel) — which `sprites` animates.
  `shape.sprite_size()` reports the texture's real size in pixels.
- **The anchor convention.** When a sprite has a meaningful pixel point — a
  joint, an anchor, an attachment — it is given in the image's own pixel
  space: `(0, 0)` at the upper-left, `x` right, `y` down.
  `Transform::anchor([jx, jy], size)` converts that point to a node-local
  offset, `(jx - w/2, h/2 - jy)`, the y flip included. A child then
  translates by the *negated* anchor, so the anchor point lands on the
  parent's origin — `tomato` chains its slices that way — and a shapeless
  parent can carry the *positive* offset to re-center the composition on
  the window — what `tomato_sprite` does with its dummy parent.
- **Ties resolve in tree (call) order.** Equal local `order` values draw in
  tree order — declare order for scene children, call order for immediate
  draws — which is why `tomato_sprite`'s overlay, the later sibling, draws
  on top of the base, and `box_z`'s white square, the later call, draws on
  top of the red one.
