# Physics examples

One demo for now — the folder is where the physics-related examples live:

| Example | What it shows | Run with |
|---|---|---|
| `collision` | the `player` example with collision: the button is an oriented box the size of its sprite, steered like a small car (`W`/`A`/`S`/`D` accelerate it relative to the way it faces, `Q`/`E` turn it, and with no key held it coasts and slows), pushed out of four random obstacle squares and of the window edges, its velocity reflected off each surface it meets (restitution 0.6); a thin outline shows its actual collision box | `cargo run --example collision` |

The file sits in this folder rather than at the examples' top level, so it
is declared explicitly in `Cargo.toml` (cargo only auto-discovers
`examples/*.rs` and `examples/*/main.rs`); the example name is unchanged.
For the pure-input version of the same button, see `../inout/player.rs`.

## The model

The `frost::collision` module is pure math — no winit, no wgpu, no state:
shapes, overlap tests, and push-out resolution, unit-tested without a GPU
and compiling identically for `wasm32-unknown-unknown`. Coordinates are the
same window-centered, y-up space the rest of frost draws in.

- `OrientedBox` — a box with a `center`, `half` extents (the
  `Canvas::rectangle` convention: the full size is `2 * half`), and a
  counter-clockwise `angle` in radians; `new` is axis-aligned, `rotated`
  adds the angle, and `corners()` gives the four world-space corners.
- `Collider` — the collision shapes: `Box(OrientedBox)` or `Circle(Circle)`.
- `Collider::push_out(other)` — the minimum translation that separates the
  two overlapping shapes: move by `dir * depth`, where `dir` is a unit
  vector pointing away from the other shape, and they no longer overlap;
  `None` when they are apart or merely touching. `intersects` is just
  `push_out(...).is_some()`.
- `frost::reflect(vel, n, e)` — reflects a velocity about the unit
  collision normal `n` (pointing the way the body was pushed, i.e.
  `PushOut::dir`) with restitution `e` in `[0, 1]`: `0` kills the
  velocity's normal component (the body slides along the surface), `1` is
  a perfectly elastic bounce; a velocity already moving away from the
  surface comes back unchanged.

The module answers questions; it owns no world and does no integration.
The demo's `Process` keeps the position and velocity, builds a `Collider`
from that state each frame, applies `push_out` together with `reflect`,
and writes the corrected position back into the scene node's transform.
When a game outgrows this — stacking, joints, ragdolls, dozens of mutually
dynamic bodies that need a broadphase, or continuous collision for very
fast projectiles — the module's named replacement is `rapier2d`: move the
state into its rigid bodies and swap these two call sites; the scene
bridge and the rendering stay the same.

## Patterns the demo follows

- **Move first, then resolve.** Each frame applies the input as an
  acceleration (the WASD direction in the button's own frame, rotated into
  the world by its facing), drags the velocity down, caps the speed,
  advances the position — and only then resolves the collisions the move
  caused.
- **One collider at a time, rebuilt after each push.** The colliders are
  resolved in a loop, and the player's box is rebuilt from the corrected
  position after each push-out, so every test sees the correction from the
  last one.
- **Four walls, never one enclosing box.** The window edges are four thin
  `OrientedBox`es just outside each edge, reaching past every corner so
  nothing slips between two of them: for a box that has already crossed an
  edge, the SAT least-penetration answer is the single face it crossed
  least, which would be the wrong push away from the other three.
- **Clamp `dt` so nothing tunnels.** The engine clamps `dt` to a second,
  but a full-second step after a stall would move the button 100 pixels in
  one frame and straight through an obstacle, so the demo clamps to 1/30.
- **The physics body *is* the sprite.** The collision box is exactly the
  sprite's size (Button.png is 164 x 195, so half extents `[82, 97.5]`),
  and the outline is drawn at z = 1.0, on top of the button, so the box
  you see is the box the physics uses.
- **The obstacles are a one-time random draw.** The four squares are
  generated once from the first frame's window size (with random colors),
  whole square kept inside the visible area, and never moved again — the
  world is static; only the button is dynamic.
