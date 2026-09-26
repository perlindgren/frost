# Particle examples

Three demos of `frost::ParticleSystem`, the engine's particle simulation
(`src/particles.rs`): a minimal fountain, three beating fountains with a
waterfall, and a variant of that where the middle plume is a batch of
rotating squares.

| Example | What it shows | Run with |
|---|---|---|
| `particles` | the minimal fountain: emission accumulator, cone spread, gravity, life-fraction fade | `cargo run --example particles` |
| `fountains` | three fountains (green, blue, and a cosine-palette cycle) plus a full-screen waterfall; per-fountain incommensurate pulse clocks; launch speed solved so the tallest surge just crests the middle of the screen; z-layering of free batches | `cargo run --example fountains` |
| `square_fountains` | a variant of `fountains`: the middle plume is a node-attached `Shape::Particles` batch of `Rectangle` squares, each with its own hue and its own spin; shows keeping per-particle render state (the spin rates) in a parallel `Vec` and pruning it in the same frame the particles die | `cargo run --example square_fountains` |

The three files sit in this folder rather than at the examples' top level,
so each one is declared explicitly in `Cargo.toml` (cargo only
auto-discovers `examples/*.rs` and `examples/*/main.rs`); the example names
are unchanged.

## The particle system

`ParticleSystem` is a plain `Vec<Particle>`, advanced by
`ParticleSystem::update`. The simulation is pure math: nothing in it knows
about the renderer, it is deterministic, and it has no capacity — the
caller controls the spawn rate.

A `Particle` holds:

- **Simulated** (touched by `update`): `pos` (a `[f32; 2]`), `vel` (pixels
  per second), and `life` (remaining seconds; the particle dies at zero).
- **Recorded**: `max_life`, the lifetime the particle was spawned with, so
  a caller can compute the life fraction `life / max_life` for a fade.
- **Render only** (never touched by `update`): `size` (the radius when
  drawn as a circle, the half-width for a rectangle or sprite), `angle`
  (the particle's own rotation, in radians, on top of the node's), and
  `color` (multiplied with the batch's base color). Because `update`
  leaves these alone, the caller can animate them between frames — spin an
  `angle`, fade a `color` — with no engine support.

`update(dt, gravity)` steps every particle with a semi-implicit Euler —
the velocity takes the gravity first, then the position takes the new
velocity, which stays stable under gravity — decrements the lifetimes, and
removes the dead ones with a `retain`. The other API: `new()`, `spawn()`,
`len()`, `is_empty()`, and the public `particles` field to read the batch
back.

Coordinates depend on where the batch is drawn: the node's local space
when the system is held in the node's `Shape::Particles` shape, or the
window's user space (window-centered, y up) for the immediate draw.

## Drawing a batch

The simulation and the drawing are split: the library simulates, the
caller draws. There are two ways to draw.

**Node-attached batch** — give a scene node the shape

```rust
frost::Shape::Particles {
    system: frost::ParticleSystem::new(),
    color: /* the batch's base tint; white leaves the particles uncolored by the batch */,
    shape: frost::ParticleShape::Circle, // or Rectangle { aspect }, or Sprite (see below)
}
```

The batch rides the node's world transform and scale exactly like the
other shapes, is tinted by the batch's `color` multiplied with the node's
composed modulate and with each particle's own `color`, draws at the node's
composed order, and fades each particle by its remaining life fraction.
Every particle shares the batch's `ParticleShape`, but keeps its own
position, scale, rotation, and tint — `ParticleShape::Circle` (the default)
draws circles, `Rectangle { aspect }` a rectangle `size * 2` wide and
`size * aspect * 2` tall, and `Sprite` (created by
`ParticleShape::sprite` from a PNG path, or `ParticleShape::sprite_bytes`
from in-memory bytes, for environments without a file system) a texture
scaled so its width is `2 * size`. The engine never advances the
simulation: the node's owner mutates the system through the scene —
`if let Some(frost::Shape::Particles { system, .. }) =
ctx.scene().root.children[i].shape.as_mut() { system.update(dt, gravity); }`
— in the same local space the particles are drawn in. `square_fountains`
is built this way.

**Free batch** — for batches that live directly in the window's user
space, with no node to attach them to, the (deprecated) immediate draw
`ctx.particles(&system.particles, color, z)` queues the whole system as one
instanced draw at z-order `z`; the frame z-sorts it against the other
immediate draws. It is kept — with a deprecation note pointing at
`Shape::Particles` — precisely for this case, which is what
`particles` and the circle fountains in `fountains`/`square_fountains`
use.

## Patterns the examples share

- **Constant-rate emission via an accumulator.** `acc += RATE * dt` each
  frame, then spawn one particle per whole unit and carry the fraction
  over, so the rate holds at any `dt`:

  ```rust
  acc += RATE * dt;
  while acc >= 1.0 {
      acc -= 1.0;
      system.spawn(frost::Particle { pos, vel, life, max_life: life, size, angle: 0.0, color });
  }
  ```

- **A step under gravity.** `system.update(dt, [0.0, -GRAVITY])` — y
  points up, so gravity pulls down with a negative y.

- **Life-fraction fade.** Set `max_life = life` at spawn; the draw fades
  each particle's alpha by its remaining `life / max_life`, so plumes
  dissolve as their drops die. No per-particle fade code is needed.

- **Per-particle spin.** `angle` is a render property the simulation never
  touches, so a spinning rectangle or a tumbling sprite is just
  `particle.angle += spin * dt;` in the process. `square_fountains` keeps
  one spin rate per live square in a parallel `Vec`, advances every square
  each frame, and — because `update` removes the dead in place — prunes the
  same rates in the same frame, with the same liveness test the update
  applies (`life - dt > 0.0`).

- **Per-particle color.** Set each particle's `color` at spawn and give
  the batch a white base color, and each particle's own tint shows through
  untouched — the way `square_fountains` picks each square's hue from a
  cosine palette at spawn.

- **Projectile sizing.** The apex of a launch at speed `v` under gravity
  `g` is `v^2 / (2 g)` above the source, so the launch speed that crests a
  height `rise` is `sqrt(2 * g * rise)` — the math `fountains` uses to keep
  the tallest surge at the middle of the screen as the window resizes.

- **Randomness from `frost::Rng`.** `rng.in_range(lo, hi)` for a uniform
  share (speeds, lifetimes, sizes, cone jitter), `rng.next_f32()` for a
  sign coin flip — one `frost::Rng` per demo, seeded from the clock, so
  the fountains differ between runs.

## Notes

- The simulation is pure CPU: a few hundred to a few thousand live
  particles per system is comfortably cheap. `docs/gpu-particles.md`
  sketches the GPU path for 10⁴–10⁵ particles.
- `update` never touches `size`, `angle`, or `color` — anything you want
  animated (a spin, a fade, a growing size) is yours to step in the
  process, one line per particle.
- Dead particles are removed inside `update`, so any per-particle side
  state (like `square_fountains`' spin rates) has to be pruned in the same
  frame, with the same liveness test the update applies.
