# Layer examples

Three demos of frost's rendering layers — `frost::Layer` in
`src/objects.rs` — built as a progression: hard draw partitions, the same
partitions with a camera (parallax), and a layer that tiles itself
(`Layer::repeat`) for infinite scrolling.

| Example | What it shows | Run with |
|---|---|---|
| `layers` | the obstacles demo with three layers added (orders -1, 0, 1): layers are hard draw partitions — a whole layer draws after every lower-ordered layer and before every higher-ordered one, no matter the local z values — with the background in the scene's root | `cargo run --example layers` |
| `parallax` | `layers` with a camera added: four layers at different parallax speeds (far squares at half speed, middle at full, the player in its own layer, near squares at double) and the square layers repeating, so the player stays at the window origin while the layers sweep past at different rates | `cargo run --example parallax` |
| `repeat` | a layer that tiles itself: `repeat` set to the window size on both axes, a hero circle straddling a tile boundary split into wrapping slices, a screen-fixed HUD layer (speed 0, no repeat), and `--repeat_x` / `--repeat_y` command-line overrides | `cargo run --example repeat` |

The three files sit in this folder rather than at the examples' top level,
so each one is declared explicitly in `Cargo.toml` (cargo only
auto-discovers `examples/*.rs` and `examples/*/main.rs`); the example names
are unchanged.

## The model

A `Scene` holds its root subtree plus zero or more rendering layers, and
the root subtree is itself a draw group — the *base group*, at the implicit
layer order `0.0`, declared before every explicit layer. Groups are painted
by ascending order — higher order is closer to the camera, drawn later, on
top — and within a group the local ordering applies: lower local `order`
first, ties resolved in tree order.

A layer is a hard draw partition: every node of a layer draws after every
node of a layer with a lower `order`, and before every node of a layer with
a higher one, no matter what the nodes' local `order` values are. That is
the difference between a layer and a plain z value: local order only
competes within its own group. A `Shape::Background` inside a layer keeps
its background behavior — it never draws, it only contributes to the frame's
clear color.

The `Layer` record:

```rust
pub struct Layer {
    pub order: f32,      // higher is closer to the camera (drawn on top)
    pub speed: f32,      // parallax speed: multiple of the camera's motion
    pub repeat: [f32; 2],// (repeat_x, repeat_y); (0.0, 0.0) draws once
    pub root: SceneNode, // the layer's own tree
}
```

with `Layer::new(root)` for the defaults (order `0.0`, speed `1.0`, no
repeat).

**`speed`** is the parallax factor: `1.0` (the default) follows the scene's
camera exactly, a higher speed moves the layer's contents faster than the
camera (the layer reads as closer), and a lower speed slower (farther
away). It is ignored when the scene has no `camera`. The base group always
renders at the full speed `1.0`.

**`repeat`** tiles the layer: a non-zero component repeats the layer in
both directions along that axis with a period of `abs(component)` pixels,
so a moving camera can scroll through it infinitely. The layer is drawn
once per copy of its content that can overlap the window — with a
pure-translation camera, only the few copies the window's box and the
content's span around it reach, because the minimum period is the window's
width/height (a non-zero period below the window size aborts with an
error). An object crossing a tile boundary is split into wrapping slices —
the part past the boundary appears on the opposite side of the window —
and each copy is re-used, with just a displacement, clipped by the
per-object scissor test, so no object is ever drawn twice. The
repetition extends the layer's *current* content: each copy carries the
content as it is now, displaced, so content that moves through the layer's
space is re-tiled as it moves. An object whose extent in a repeating axis
exceeds its period (it would be drawn twice) also aborts with an error.

**The camera** (`Scene::camera`) is a `NodePath` — the group (the base
group or the `i`th layer) and the child-index path from that group's root.
When set, the scene is drawn in that node's coordinate space instead of the
fixed window-centered user space: the camera node stays at the window
origin while the rest of the scene moves and rotates around it, so a camera
hung under a moving node follows it. Each group renders from the camera
scaled by its speed.

## Patterns the examples share

- **Generate on the first frame.** The window size is only known once the
  first frame runs, so `layers` and `parallax` build their random
  rectangles (and `parallax`'s repeat offsets) in the first frame's
  process, seeded from the current time — each run gets a different set,
  and the set is kept for the whole run.
- **Unique objects get their own non-repeating layer.** The player is a
  unique object, not part of a pattern that should tile, so `parallax`
  gives it a layer of its own with no repeat. (A camera-followed object in
  a *repeating* layer would stay on screen too — its wrapped copies sit
  one full period away, off the window's edge — but it would still be
  drawn as a tiled copy wherever its period puts one.)
- **Screen-fixed HUDs are a speed-0 layer.** `repeat`'s HUD sits in a
  layer at `order: 10, speed: 0.0` with no repeat: it never moves with the
  camera and never tiles, so it is pinned to the window.
- **The background lives in the base group.** In all three demos the
  background is in the scene's root, not in a layer: the base group's
  transform is ignored for a `Shape::Background`, so it always fills the
  window, and at the implicit order `0.0` it renders behind every layer
  ordered `0.0` or higher.
- **Minimum repeat offset.** `parallax` repeats its square layers at the
  window size in both axes — the smallest period the engine allows — and
  `repeat` does the same by default, with `--repeat_x 500 --repeat_y 0`
  showing an overridden period (`0.0` disables the axis; a non-zero
  offset below the window size aborts the program with an error).
