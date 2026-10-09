# Physics examples

Three demos — the folder is where the physics-related examples live:

| Example | What it shows | Run with |
|---|---|---|
| `collision` | the `player` example with collision: the button is an oriented box the size of its sprite, steered like a small car (`W`/`A`/`S`/`D` accelerate it relative to the way it faces, `Q`/`E` turn it, and with no key held it coasts and slows), pushed out of four random obstacle squares and of the window edges, its velocity reflected off each surface it meets (restitution 0.6); a thin outline shows its actual collision box | `cargo run --example collision` |
| `authored_hall` | a dark hall of brick sprites whose shadows *and* collisions come from data, not code: one `bake_sidecar` call reads `brick.ron` beside `brick.png` at load, and its single baked footprint rides every brick — `declare_occluders` under each transform cuts the player's glow *and* mouse-aimed torch beam into shadows, the baked solids push the player out with the sidecar's own `(bounce: 0.35)`; a mis-authored sidecar refuses to start the example (`docs/authored-shapes.md`) | `cargo run --example authored_hall` |
| `bodies` | the `physics` module drawn: three elastic `Body` rectangles (weights 1, 2, 4) at random non-overlapping starts with 50–100 px/s velocities, rattling in a box of four 50 px walls — every contact a perfectly elastic impulse, and a live kinetic-energy readout that stays flat while the velocities trade (momentum *does* move: a wall is an outside force); the `frost::Diagnostics` overlay rides on top — frame time, processing time and draw calls, charted — to put a number under any perceived jitter (Alt-0 hides it) | `cargo run --example bodies` |

Both files sit in this folder rather than at the examples' top level, so
each is declared explicitly in `Cargo.toml` (cargo only auto-discovers
`examples/*.rs` and `examples/*/main.rs`); the example names are unchanged.
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

The `frost::physics` module moves those shapes: a `Body` is a `Collider`
plus a velocity and a `weight`, `advance(dt)` integrates the center,
`resolve_pair` trades two bodies' momentum along their `push_out` normal
with a perfectly elastic impulse (penetration split by inverse weight,
already-parting pairs never kicked), and `resolve_static` bounces a body
off a shape that never moves. `kinetic_energy` and `momentum` read the two
ledgers the impulse cannot spend; `bodies` puts the first on screen.

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
- **One sidecar, one bake, every brick.** `authored_hall` reads
  `assets/sprites/brick.ron` exactly once, at boot, and the one
  [`Baked`](../../src/bake.rs) it gets back serves every brick in the
  room: declared shadows and collided solids are the same records under
  different transforms. The brick sprites themselves are *not* marked
  `occludes` — the silhouette the light field gets is the one the sidecar
  drew, inset two pixels from the picture's edge.
- **Move, then resolve — pairs, then walls.** `bodies` advances every body,
  settles every pair, then settles every body against every wall: one pass,
  each contact exact on its own, in an order that is a choice and not a law.
- **The obstacles are a one-time random draw.** The four squares are
  generated once from the first frame's window size (with random colors),
  whole square kept inside the visible area, and never moved again — the
  world is static; only the button is dynamic.
