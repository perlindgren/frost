# Lighting examples

Nine demos of frost's lighting system: the `frost::Light` record, the
`Shape::Light` scene node, the immediate `ctx.light` / `ctx.light_cone`
calls, and the three node flags that make a surface take part in it —
`lit`, `occludes`, and `glow`.

| Example | What it shows | Run with |
|---|---|---|
| `lighting` | the basic demo: a lit floor, a lit ball, and a lit particle plume under one node light (orbiting) and two immediate lights (cursor-chasing, static); shows that a particle batch can be a lit receiver | `cargo run --example lighting` |
| `lit_unlit` | the minimal contrast: two rectangles in the same color, one with `lit: true`, and a light stuck to the mouse with an immediate `ctx.light` call | `cargo run --example lit_unlit` |
| `bouncing_lights` | three point lights (amber, azure, mint) bouncing DVD-style off the window edges over lit receivers, with the rectangular receivers flagged `occludes` so they cut hard shadows into the light | `cargo run --example bouncing_lights` |
| `cone` | a square explorer carrying a torch: a small omni point light and a narrow cone light as **child nodes** of the player, so they ride its position and turn with it; a dark hall of unlit occluder walls | `cargo run --example cone` |
| `dawn` | a full day (64-second loop) under one `Light::directional`: the sun's color, strength, direction of travel, the sky, and the scene's ambient all slide together; parallel shadows sweep from dawn spears to noon stubs | `cargo run --example dawn` |
| `near_sun` | the same day told by a *near* sun — a point light translated to the visible disk every frame — so shadows fan out from the disk like spokes on a wheel instead of lying parallel | `cargo run --example near_sun` |
| `twostars` | a binary star system: two directional lights, each aimed, colored, and orbiting at its own rate; every lit pixel receives both, and every occluder cuts each of them separately | `cargo run --example twostars` |
| `eyes` | a tour of node-level `glow` — a surface's own emission: glowing eyes that burn inside a wall's shadow, because glow is never shadowed | `cargo run --example eyes` |
| `eyes_light` | `eyes`' companion: the larger eyes carry a `Shape::Light` child node, so their light spills onto the room — and gets cut off at the walls — while their glow keeps burning | `cargo run --example eyes_light` |

The nine files sit in this folder rather than at the examples' top level,
so each one is declared explicitly in `Cargo.toml` (cargo only
auto-discovers `examples/*.rs` and `examples/*/main.rs`); the example names
are unchanged. `cone_collider` — the physics version of `cone`'s hall —
stays at the top level, since its topic is collision.

## The model

Every lit receiver's pixel is shaded per frame as

```text
pixel = color * (ambient + lights + glow)
```

where `ambient` is the scene's ambient color (`Scene::ambient` — the floor
of light when no light reaches), `lights` is the sum of the
contributions of every light in the frame, each falling off with distance
(point and cone) or not at all (directional), and `glow` is the node's own
emission. The three node flags opt a surface in:

- **`lit: true`** — the node's own shape is shaded by that mix. It does
  not propagate to children; each node carries its own. `false` (the
  default) draws the shape in full color, untouched by the light field.
  The true window background (the clear color) is never shaded, which is
  why the dark-hall examples use a huge *lit* rectangle as their
  "floor and walls" instead.
- **`occludes: true`** — the node's own shape cuts shadows: the light's
  path to every lit pixel behind it is blocked. Only rectangles occlude,
  and the silhouette is evaluated in the frame's pixel space from the
  full composed transform, so a rotated, scaled, or translated node casts
  the shadow of wherever it actually sits. A wall can be `lit: true,
  occludes: true` at once — lit on its own surface and still blocking
  what lies behind it.
- **`glow: Color`** — the node's self-emission, added to the mix, so the
  surface stays visible in the dark while lights still add on top. It
  requires `lit` (an unlit shape is already drawn in full), lights only
  the surface itself — it never spills onto neighbors and is never
  shadowed — and is scaled by the composed modulate. The color's `a`
  scales the emission (rgb times `a`), so fading a glow is an alpha ramp
  on the color. Black (the default) is off.

## Kinds of light

`frost::Light` is one record with three constructor shapes:

- **`Light::point(color, intensity, radius)`** — an omni-directional
  point light: full circle, reaching exactly `radius` pixels and fading
  to zero at the edge. `radius` is clamped to zero or more.
- **`Light::cone(color, intensity, radius, direction, spread, softness)`**
  — a beam of full opening angle `spread` radians (`PI` is a half-plane,
  `TAU` or more is omni, `0.0` degenerates to the axis), pointing
  `direction` radians counterclockwise from the local +x axis.
  `softness` feathers each edge over that many radians of angle —
  `0.0` is a hard edge — the cheap stand-in for a penumbra.
- **`Light::directional(color, intensity, direction)`** — light from a
  source so far away that its rays are parallel: no position, no
  falloff. `direction` is the direction the light *travels* (a dawn sun
  rising in the east travels toward `-x`, near `PI`). The node carrying
  it only aims it — position and scale do not matter — and a scene may
  hold any number: two directional lights light the world from two
  directions at once, each casting its own parallel shadows.

`light.with_penumbra(radius)` gives any of them a soft shadow edge: the
shadow rays sample a disk of that radius (nine taps, rotated per pixel)
and the light passes in proportion to how much of the disk an occluder
leaves uncovered, so the shadow fades across an edge as wide as the disk
instead of ending on a hard silhouette. `0.0` (the constructor default)
is the hard shadow of a mathematical point light. A directional light's
penumbra reads geometrically — values around 20 and up give suns and
stars shadows worth calling soft.

## Placing a light

Two ways, with different coordinate spaces:

**On a node** — give the node `Shape::Light { light }`. The light is
never drawn; it sits at the node's local origin, so the node's transform
positions it in the scene, its rotation turns the cone, and the composed
modulate tints its color, exactly as for a shape. This is how the torch
rides the player in `cone`, how the orbiting light is carried in
`lighting`, and how the lamp-eyes' lights ride the eyes in
`eyes_light`. A node light can be re-aimed or retinted each frame by
rewriting its `Shape::Light`.

**In the window's user space** — the immediate calls
`ctx.light(x, y, color, intensity, radius)` and
`ctx.light_cone(x, y, color, intensity, radius, direction, spread)`.
No node's transform, scale, or modulate applies; the light is placed
where you say, in window coordinates (window-centered, y up), with a
point-sized shadow and a hard cone edge. The frame's light field is
rebuilt from that frame's calls, so to *move* an immediate light you call
it again every frame with the new position — which is what `lit_unlit`
does with the cursor, and what `lighting` does with its chasing light.

## Patterns the examples share

- **A light is never drawn, so stage a face for it.** A point light is a
  position — `bouncing_lights` marks each one with two unlit immediate
  circles (a tinted halo and a white core) so you can see where the
  pools come from. A directional light has *no* position, so where its
  source "is" is yours to place: `dawn` and `twostars` walk an unlit
  disk with halo rings around the sky — in `twostars` riding the very
  angle the shadows fall along, so the light and the visible face are
  driven by the same angle and the face always points directly at its
  shadow family.
- **The near-sun trick.** `near_sun` pins a point light to the visible
  disk and translates it there every frame: light and face are literally
  the same node position, and the scene gets everything perspective
  implies — shadows that fan from the disk and rotate with it — at the
  price of falloff and a position to maintain (below the horizon the
  strength is cut to zero outright). `dawn` and `near_sun` share the
  same 64-second clock on purpose; run them side by side to compare
  parallel spears against fanning spokes.
- **Unlit occluders stay readable in the dark.** `cone`'s walls are
  deliberately unlit — they draw in their own color no matter where the
  lights are, so you can always see where they are — while the backdrop
  and panels are lit receivers the beam plays over.
- **A low ambient makes a dark scene.** `Scene::ambient` is the floor
  every lit pixel sits on; `cone`, `eyes`, and `eyes_light` set it low so
  unlit regions read as near-black and the lights are what reveals the
  room, while `dawn` and `twostars` slide it with the sky.
- **Glow vs. light, side by side.** `eyes` proves the receiver half: the
  eyes burn inside the wall's shadow because glow is never shadowed.
  `eyes_light` adds the emitter half to the same hall: the lamp-eye's
  pool of light sweeps the floor and gets cut off at the wall, feathered
  by its penumbra, while the glow-only eye keeps burning — "glow is the
  surface; the child light is the neighborhood."
- **A particle batch can be a lit receiver.** `lighting`'s ember plume
  is a `Shape::Particles` node with `lit: true`, so every ember is
  shaded by the same light field as the floor and the ball.

## Notes

- Lights are evaluated per pixel at render time, per frame: the light
  field has no state of its own, so every frame re-declares it — node
  lights persist in the scene graph, immediate lights persist only by
  being called again.
- Only rectangles occlude; circles, sprites, text, and particle batches
  never cast shadows (flagging the huge backdrop would shadow the whole
  scene).
- A node light's `radius` is in user units, mapped one to one to pixels;
  a directional light ignores its position and radius entirely (the
  negative radius is its sentinel).
- The immediate cone (`ctx.light_cone`) has no softness knob — hard
  edges — and point-sized shadows; for feathered beams and soft shadows,
  use a node's `Shape::Light`.
