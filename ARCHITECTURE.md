# ARCHITECTURE

A map of the `frost` crate for getting up to speed quickly: what each file is for,
how a frame flows through the library, and the invariants the code relies on.
The doc comments in the source are the ground truth — this file is the index and
the "why".

## What frost is

A **minimal winit + wgpu immediate-mode drawing library** for games in Rust
(edition 2024). You give it a `Scene` (a tree of nodes) and a `Process` (a
per-frame callback), and it runs an event loop: each frame your `Process` mutates
the scene and issues immediate draws, then the scene tree is updated and drawn.

Design stance (see `README.md`): a game-engine experiment by N65 Game Research
Station. Deliberately **not** Bevy-style ECS with shared mutability — plain Rust
ownership, a scene tree, and immediate drawing. Goals: a realistic reasoning
challenge for a local-hosted AI workflow, human learning of engine internals, and
(if it works) the core mechanics of a library-based engine.

## Crate layout

```
Cargo.toml            deps: image 0.25 (png), log 0.4, swash 0.2 (text),
                      wgpu 30.0.1, winit 0.30.13; wasm32-only: web-sys,
                      console_error_panic_hook; dev-deps: clap, env_logger,
                      naga 30.0.0 (shader validation, no GPU needed)
src/lib.rs            crate docs + public API: Canvas, Context, Process, Config,
                      run / run_configured, draw_scene, paint_order, expand_text
src/objects.rs        Color, Transform, Shape, Node, SceneNode, Layer, Scene,
                      NodePath, world_at / camera_world
src/tween.rs          Tween<T> (f32 / [f32;2]) with Repeat modes
src/particles.rs      Particle, ParticleSystem (pure simulation)
src/collision.rs      OrientedBox, Circle, Collider, push_out, reflect (pure math)
src/audio.rs          Audio (device + loop player), Sound (decoded buffer),
                      AudioError — native-only, rodio-based
src/text.rs           CPU text shaping/rasterization on swash, glyph shelf atlas
src/shaders.rs        include_str! of the 5 WGSL sources + naga layout tests
src/backend/mod.rs    `app`, `frame`, `wasm` (wasm32 only), `tests` (test only)
src/backend/app.rs    Frost<P>: winit app handler + GPU state + render loop
src/backend/frame.rs  Draw list model: Draw enum, scissor rects, uniform writers
src/backend/wasm.rs   WebFrost: deferred async GPU setup + fallback DOM helpers
src/backend/tests.rs  GPU-free backend tests (uniform layout, canvas behavior)
shaders/*.wgsl        line, circle, rectangle, shape (SDF circle+rect), sprite
examples/             25 runnable demos (see table below)
assets/               sprites/*.png (+ .pxo sidecars for brick, water_can),
                      fonts/JameGem08_2026-Regular.ttf, audio/swoof.wav
src/TODO.md           next planned feature (Body / rigid bodies)
```

The library uses only the `log` facade; consumers wire up their own logger
(`env_logger` in the examples). No randomness crate — examples that need random
numbers use a hand-rolled splitmix64 `Rng(u64)`.

## The public API (src/lib.rs)

- `frost::run(scene: Scene, process: P) -> Result<(), ...>` — the entry point.
  `run_configured` takes a `Config { vsync: bool, window_size: Option<[u32;2]> }`
  (vsync default on, window size default platform) to turn vsync off for
  uncapped frame rates or open the window at a specific inner size in logical
  pixels (the user can still resize afterwards). Both exist for native and
  wasm32.
- `Process` — `fn process(&mut self, ctx: &mut Context, dt: f32)`; `dt` is
  seconds since the previous frame (`0.0` on the first, clamped to 1.0s).
  **Any `FnMut(&mut Context, f32)` closure is a `Process`.**
- `Context` — derefs to the frame's `Canvas` (immediate draws), and:
  - `scene() -> &mut Scene` — mutable access to the scene passed to `run`;
    mutate it here to animate it; it is drawn after the process returns.
  - `key_down(KeyCode) -> bool` — held physical keys (scancodes, layout-free).
  - `mouse_position() -> Option<[f32;2]>` — user coords, `None` when the cursor
    is outside the window (last known position is deliberately dropped).
  - `mouse_button_down(MouseButton) -> bool`
  - `expected_fps() -> Option<f32>` — monitor refresh rate when vsync is on.
  - `size() -> (f32, f32)` — window size in pixels.
- Re-exports: `KeyCode`, `MouseButton`, `Tween`/`Repeat`,
  `ParticleSystem`/`Particle`, and everything from `objects` (`Scene`,
  `SceneNode`, `Node`, `Shape`, `Color`, `Transform`, `Layer`, `Canvas`).
- Native only (no wasm32): `Audio`, `Sound`, `AudioError` (see
  "Audio (native only)" below).

**Coordinate system.** User space is in **pixels**, origin at the **window
center**, **y pointing up**: top-left = `(-w/2, h/2)`. The canvas works in
physical pixels internally; the only conversion is the cursor flip done when
building the `Context` (physical y-down → user y-up).

### Frame flow (the whole library in one pass)

`Frost::render()` (src/backend/app.rs) is the heart:

1. Acquire the surface texture (`Timeout` → log and skip the frame).
2. Build a fresh `Canvas` for the pixel size; compute `dt` from a monotonic
   millisecond clock (`Instant` native, `performance.now()` wasm), clamped to
   1.0s.
3. Build the `Context` (canvas + scene + held keys + mouse state) and call
   `process.process(&mut ctx, dt)`.
4. `scene.visit()` — every node's `Node::process`, **children before parent**
   (post-order), on the root tree and every layer tree.
5. `canvas.draw_scene(&scene)` — walks each group depth-first, composing
   transforms/modulates/orders, and records `Draw`s (see below).
6. `canvas.expand_text(&mut text_atlases)` — lays out every `Draw::Text` and
   splices one `Draw::Sprite` per glyph **in place**, so glyphs keep their
   node's position in the paint order.
7. `canvas.paint_order()` — merges groups (base group first by implicit order
   0.0, layers by `Layer::order`, higher on top; stable) and **stable-sorts
   each group by `z`** (lower first; ties keep call order, so the last drawn is
   on top).
8. One render pass for the whole frame: clear with the frame's background color,
   then per draw: set the scissor to the draw's **tight bounding box**, build a
   fresh uniform buffer + bind group, draw a full-screen triangle.
9. `queue.submit`, `queue.present`. `RedrawRequested` re-requests the next
   frame, so animation runs continuously.

## The scene graph (src/objects.rs)

- `Scene { root: SceneNode, layers: Vec<Layer>, camera: Option<NodePath> }`.
  `Scene::visit()` updates the root subtree and every layer subtree.
- `SceneNode` — a `transform` + `scale` + `modulate` + `order`, an optional
  `shape`, and `children`. All four are **relative to the parent**:
  - transform/scale: `local = scale(node.scale).compose(&node.transform)`,
    `world = local.compose(&parent)`. **Scale is innermost** — it applies to the
    node's own shape *and* composes onto descendants.
  - `modulate` multiplies channel-wise into the node's shape color and composes
    onto descendants' (a descendant's color is the product of modulates on its
    root-to-leaf path). `Color` fields are 0..=1; `WHITE` is the identity.
  - `order` adds to the subtree's draw `z` (inherited `order + node.order`).
  - A node without a shape is a pure group/pivot; `..SceneNode::default()`
    fills identity transform, no scale, white modulate, order 0.
- `Node` trait — `visit()` (post-order walk) and `process()` (default no-op),
  so custom node types can participate; `SceneNode` implements it.
- `Layer { order, speed, root }` — a **hard draw partition** painted as a whole
  at `order` (its internal `z` ordering only matters inside the layer), with a
  parallax `speed` under a camera. `Layer::repeat` tiles the layer's content
  with a period ≥ the window size, splitting boundary-crossing objects into
  wrapping slices (drawing the same object twice aborts with an error).
- `camera: Option<NodePath>` — when set, every group is drawn from the camera
  node's coordinate space: the scene is transformed by the **inverse of the
  camera's world transform**, with the translation scaled per group (base 1.0,
  each layer by its `speed`) — that's the parallax: higher speed reads as
  closer. `NodePath` names a node by (group index?, child indices).
  `Scene::world_at(path)` / `Scene::camera_world()` compute the composed world
  transform (src/objects.rs:620–641).
- `Transform` — `p' = m * p + t` with rows `m[0]`, `m[1]`; `rotate(angle)` is
  CCW from +x toward +y; `a.compose(&b)` means **a first, then b**
  (matrix `b.m * a.m`); `invert()` for the render path; `scales()` for the AA
  band.
- `Shape` (attached to a node, drawn in the node's local space before its
  children, so parents paint under descendants):
  - `Circle { center, radius, color }`, `Rectangle { center, extent (half
    widths), color }` — the node's world transform applies.
  - `Background { color }` — fills the window, **ignores all transforms**,
    drawn at the very back: at render time it becomes the frame's clear color
    (the last one in depth-first call order wins; default is a dark blue-gray
    when none exists). It costs no draw call.
  - `Sprite { data: Arc<[u8]>, width, height, color, alpha }` — RGBA8 PNG from
    `Shape::sprite(path)` (decoded eagerly; `SpriteError` on failure).
    Centered on the node origin, 1 texture pixel per scene pixel. `color` tints
    every pixel (white = unchanged); overall opacity = texture alpha × `alpha`
    × `color.a`.
  - `Text { text, font: Arc<[u8]>, size, color, alpha }` — from
    `Shape::text`/`text_bytes` (swash, CPU-only). Centered on the node origin;
    each glyph becomes a tinted quad over a shared atlas.
  - Color model: plain shapes emit `coverage * a`; sprites/text emit
    `texture_alpha * opacity * a` — all composited with `ALPHA_BLENDING`.

## The draw list (src/backend/frame.rs)

`Canvas` holds `draws` (the **base group**: immediate methods + every scene's
root subtree, mixed) plus one list per explicit layer (`layer_draws`,
`layer_orders`).

- Immediate methods: `line`, `circle`, `rectangle` (pixel args converted to
  pixel space via `user_to_pixels`), and `draw_scene`.
- `Draw` enum: `Line`, `Circle`, `Rectangle`, `Shape` (transformed SDF
  circle/rectangle: `world`, `center`, `params` (circle `[r,0]`, rect `[hx,hy]`),
  `kind` 0/1, `aa`, `color`), `Sprite` (`world`, `data: Arc<[u8]>`, `size`,
  `texture_size`, `aa`, `tint`, `alpha`, `uv_rect`, `z`), `Text` (expanded
  before rendering — never reaches the render loop), `Background` (becomes the
  clear color — never drawn).
- **Scissoring**: every draw's fragment work is clipped to its tight pixel
  bounding box (`scissor_rect`, including the AA band), so **GPU cost scales
  with on-screen object area, not window size**. Rotated shapes/sprites get the
  AABB of their four transformed corners (`aabb_of_box`).
- **Anti-aliasing**: `AA_BAND = 0.75` px on the CPU; the same constant appears
  as `aa` in the shader sources — keep them in sync. Under node scaling the
  band is divided by the transform's scale so it stays a constant screen width.
- **Uniform writers** (`line_uniform_data`, `circle_uniform_data`,
  `rect_uniform_data`, `shape_uniform_data`, `sprite_uniform_data`) hand-write
  little-endian bytes at explicit offsets matching the WGSL **uniform-space**
  layout (e.g. ShapeUniforms: `to_local` mat2x2 @0 (16B, 8-aligned columns),
  `translation` @16, `center` @24, `params` @32, `color` vec4 @48, `misc` @64;
  total 80. Sprite the same with `size`/tint/`alpha`/`uv_rect`). Line/circle
  are 48 bytes, rectangle 32.

## The GPU side (src/backend/app.rs, shaders/)

- `Frost<P>` holds the wgpu instance/adapter/device/queue, `vsync` and
  `window_size` (the `Config` values), the window (`Arc<Window>`), logical
  size + scale factor, the surface, the five pipelines
  (line/circle/rect/shape/sprite), `sprite_resources` (a
  `HashMap<*const (), (TextureView, Sampler)>` keyed by the sprite pixel-data
  `Arc`'s pointer — sprites from the same file, and glyph quads from the same
  atlas, **share one uploaded texture**), `text_atlases`
  (`HashMap<(font ptr, size bits), text::Atlas>`, kept between frames so
  unchanged text never re-rasterizes), held keys/mouse, and the user's
  `process` + `scene`.
- All five pipelines are the same shape: a **full-screen triangle** vertex
  shader (no vertex buffer), one `@binding(0)` uniform (plus texture + sampler
  for sprites), alpha blending, no depth. `present_mode_for(vsync)` maps to
  `AutoVsync`/`AutoNoVsync`.
- **One fresh uniform buffer per draw call** — required because
  `queue.write_buffer` copies are flushed as a batch *before any draw
  executes*, so a shared buffer would make every draw read the last parameters.
- Sprite textures are `Rgba8UnormSrgb` (samples land in linear space; sRGB
  blending matches authored colors), linear-filtered, clamp-to-edge.
- The shape/sprite shaders evaluate an **SDF in the object's local space**: the
  fragment's pixel coordinate is transformed by the inverse world transform
  (uploaded as `to_local` + `translation`; a non-invertible transform is
  skipped), the signed distance is measured in local units, and
  `smoothstep(-aa, aa, d)` gives the coverage. Rotation and scaling "apply for
  free".
- Events (native): keyboard held-set (Escape exits), cursor position, mouse
  buttons, `CursorLeft`/`Focused(false)` **clear** the held state (no stuck
  controls), resize/scale-factor reconfigure the surface (pipelines rebuild
  only if the format changed), `RedrawRequested` → `render()` + re-request.
  The window opens at the `Config`'s `window_size` (or the platform default),
  centered on the monitor, and requests its first redraw explicitly (Wayland
  won't deliver one otherwise).
- `block_on` (native only) drives the adapter/device futures with a no-op
  waker — the browser can't block on them, hence `WebFrost`.

### wasm32 (src/backend/wasm.rs)

In the browser `request_adapter`/`request_device` are JS promises that only
resolve once the main thread is free, so `run` defers GPU setup: `WebFrost`
creates the canvas immediately (900×600, appended to the page), keeps the
scene/process in a `Core`, and on every `resumed` polls the in-flight setup
future (`GpuInit::{Idle, Init, Failed}`); when it completes it hands everything
to a real `Frost`. `hide_fallback`/`show_fallback` swap the page's "Loading
frost…" placeholder with the first frame or a red error message.

## Text (src/text.rs)

CPU-only, built on **swash** (pure-Rust Fontation shaping/rasterization):

- `layout(font: &[u8], text, size) -> Option<TextLayout>` shapes the string into
  `PlacedGlyph { id, x, y }` (pen origin at baseline left, y up) and returns
  metrics `{ ascent, descent, width, glyphs }`.
- Each unique glyph is rasterized **once per (font, size)** into a shared 512px
  shelf atlas (`ATLAS_SIZE: u32 = 512`) — first-fit row packing; oversize
  glyphs are skipped but the cursor still advances. Output is RGBA8 with alpha
  replicated across channels.
- `Canvas::expand_text` (src/lib.rs) consumes the `Draw::Text` entries, lays
  them out, and replaces each with per-glyph `Draw::Sprite`s sampling the
  atlas sub-rectangle (`uv_rect`) — this is why sprite `uv_rect` exists.

## Tween (src/tween.rs)

`Tween<T: Tweenable>` for `f32` and `[f32;2]`: `new(from, to, duration)`
(default `Repeat::PingPong`), `repeat(Repeat::{Once, Loop, PingPong})`,
`tick(dt) -> T`. Durations are floored at 1e-6. **There is no re-target API** —
the established pattern (e.g. `examples/cursor.rs`) is to **rebuild the tween**
on the press/release edge with the current value as the new `from`.

## Particles (src/particles.rs)

`Particle { pos, vel, life, max_life, size }`; `ParticleSystem { particles: Vec }`
with `spawn(p)` and `update(dt, gravity)` — semi-implicit Euler
(`vel += g*dt`, then `pos += vel*dt`), `life -= dt`, `retain(|p| p.life > 0.0)`.
Pure simulation, renderer-independent; **drawing is the caller's job**
(typically `ctx.circle` per particle, alpha ∝ remaining life). Emission is
accumulated in the caller (`acc += rate*dt; while acc >= 1.0 { acc -= 1.0; spawn }`)
so the spawn rate is independent of frame rate.

## Collision (src/collision.rs)

Pure math — no winit/wgpu, identical on wasm, no dependencies. Intentionally
**minimal by design**; the documented escape hatch when the game outgrows it is
`rapier2d` (the scene bridge and rendering would stay the same):

- `OrientedBox { center, half, angle }` (full size = 2×half, CCW angle) with
  `axis(i)` and `corners()` (CCW from lower-left); `Circle { center, radius }`;
  `Collider::{Box, Circle}`.
- `push_out(&other) -> Option<PushOut { dir, depth }>` — the minimum separating
  translation to resolve the overlap (SAT for boxes, direct for circles).
- `reflect(vel, normal, restitution)` — kills the normal component (scaled by
  restitution), leaves the tangent; no-op for velocity already leaving the
  surface.
- The intended usage pattern: the game's `Process` owns all state, builds
  colliders from node transforms each frame, applies `push_out` + `reflect`,
  and writes the results back into the node transforms.

## Shaders (shaders/*.wgsl, src/shaders.rs)

Five small WGSL files, all full-screen-triangle + SDF/sampling:

| file        | pipeline   | uniforms (byte layout)                                              |
|-------------|------------|---------------------------------------------------------------------|
| line.wgsl   | line       | `a`@0 `b`@8 `color`@16 `width`@32 → 48                             |
| circle.wgsl | circle     | `center`@0 `color`@16 `radius`@32 → 48                             |
| rectangle.wgsl | rectangle | `center`@0 `extent`@8 `color`@16 → 32                            |
| shape.wgsl  | shape      | `to_local`@0 `translation`@16 `center`@24 `params`@32 `color`@48 `misc`@64 → 80 |
| sprite.wgsl | sprite     | `to_local`@0 `translation`@16 `size`@24 `tint`@32 `alpha`@48 `uv_rect`@64 → 80 |

`src/shaders.rs` includes them and has tests that **parse all five with
`naga::front::wgsl::parse_str`** (the same frontend wgpu uses, so validity is
pinned without a GPU) and assert the `ShapeUniforms`/`SpriteUniforms` member
offsets are `[0,16,24,32,48,64]` spanning 80 bytes — the **CPU uniform writers
must mirror these layouts**; the GPU-side mirror tests live in
`src/backend/tests.rs`. If you change a shader uniform struct, update the
writer and both test sides together.

## Audio (native only)

`src/audio.rs` plays sounds through **rodio** on the host's default output
device. The module is `#[cfg(not(target_arch = "wasm32"))]`-gated in
`lib.rs` (rodio's cpal device layer is native-only), so wasm builds never
compile it; `Audio`/`Sound`/`AudioError` are re-exported from the root only
on native targets.

- `Sound::load(path)` / `Sound::load_bytes(bytes)` — **load-time decode**:
  the file (WAV, FLAC, MP3, OGG Vorbis, AAC, M4A) is read and decoded once,
  into an in-memory `rodio::buffer::SamplesBuffer` of f32 samples, so a bad
  path or format is an `AudioError` up front, not at play time. The samples
  sit behind an `Arc`, so re-triggering a `Sound` shares one buffer and
  clones are cheap. Accessors: `channels()`, `sample_rate()`, `duration()`.
- `Audio::new()` — opens the default output device
  (`DeviceSinkBuilder::open_default_sink`), silences the drop log
  (`log_on_drop(false)`), and builds the single loop `Player` on the device's
  mixer. `Audio` owns the device; dropping it stops everything.
- `Audio::play_once(&sound)` — **one-shot, parallel overlap**: a copy of the
  buffer is `amplify`-ed by the master volume and `add`-ed to the mixer, so
  the same sound re-triggers freely and copies overlap. The volume is frozen
  at trigger time.
- `Audio::play_loop(&sound)` / `stop_loop()` — **loop, single sequential
  player**: `play_loop` `clear()`s, `append`s `buffer.repeat_infinite()`,
  `play()`s (a `clear` leaves the player paused, so the `play()` is
  load-bearing), and sets the loop's volume. Only one sound loops at a time;
  `play_loop` on a running loop restarts with the new sound. `stop_loop`
  `clear()`s. The master volume applies to the loop live.
- `Audio::set_volume(v)` — clamps to `0.0..=1.0` and stores the f32 bits in
  an `AtomicU32` (no cast needed: rodio's `Float`/`Sample` are `f32`); it
  updates the loop player immediately and one-shots from the next trigger.
- `AudioError` — hand-rolled `Debug` enum with a manual `Display`/`source()`
  in the project's `TextError`/`SpriteError` style: `Io(std::io::Error)`,
  `Decode(rodio::decoder::DecoderError)` (held by value; `Clone`), and
  `Device(Box<rodio::DeviceSinkError>)` (boxed because the device error is
  not `Clone`).

Tests are **device-free** (no `DeviceSinkBuilder`/`Audio::new()`), so
`cargo test` runs headless: a synthetic in-code PCM16 WAV decodes to the
right channel/rate/length, the bundled `swoof.wav` decodes, a missing path is
`AudioError::Io`, garbage bytes are `AudioError::Decode`, and the
`clamp01`/f32-bits helpers round-trip. `Sound::load_bytes` carries a doctest
that decodes an in-code WAV.

## Testing

Baseline: **102 tests + 2 doctests** passing, `cargo build --examples`
clean. Notable test areas:

- `src/shaders.rs` — naga parse + uniform-offset assertions (above).
- `src/backend/tests.rs` — GPU-free: uniform-layout mirrors, scissor math,
  `paint_order`, `expand_text` (splicing + atlas reuse across frames),
  `context_reports_held_mouse_button`.
- `src/collision.rs` — push-out separation, reflect restitution semantics.
- `src/tween.rs`, `src/particles.rs`, `src/text.rs` — behavior unit tests.
- `src/audio.rs` — device-free decode tests (synthetic WAV, bundled
  `swoof.wav`, error variants) + the `load_bytes` doctest.

## Examples (examples/)

All load assets via `CARGO_MANIFEST_DIR`. Run with `cargo run --example <name>`
(`RUST_LOG=info` for logs; on Windows PowerShell:
`RUST_LOG=info cargo run --example gizmos 2>&1 | Out-String`).

| example      | shows                                                              |
|--------------|--------------------------------------------------------------------|
| one_rect     | smallest scene: background + one static rectangle                  |
| circles      | immediate circle draws                                              |
| gizmos       | the README's canonical demo (mixed immediate + scene draws)        |
| box_z        | overlapping squares with explicit `z` values (z-ordering)          |
| tween        | one scalar `Tween` driving a line                                   |
| sub_zero     | text shapes ("SUB0") through the scene tree                        |
| text         | text shapes ("Sub") through the scene tree                         |
| text_web     | the `text` scene for the wasm target                               |
| player       | WASD drives a Button sprite from `(0, 0)`                          |
| obstacles    | player + four fixed obstacles                                      |
| layers       | obstacles + rendering layers                                       |
| parallax     | layers + camera: infinite parallax scrolling                       |
| repeat       | a layer tiling itself (`Layer::repeat`)                            |
| scene        | an in-place animated scene tree (orbiting children)                |
| satellite    | the `scene` tree refactored into reusable nodes                    |
| sprites      | PNG sprites through the scene tree                                 |
| tomato       | a plant assembled from three sprite slices                         |
| grow         | the tomato plant growing slice by slice                            |
| collision    | the player with `push_out`/`reflect` collision                     |
| particles    | a `ParticleSystem` fountain with life-fraction alpha               |
| cursor       | a watering-can cursor: mouse-following sprite, press-to-tilt tween, |
|              | spout-emitted water particles over a full-screen grass field       |
| sound        | one-shot + looping playback of a decoded WAV: Space re-triggers     |
|              | (pulsing circle), L toggles the loop (wobbling ring), +/- the bar   |
| button       | a "play" button: hovering tweens the node's `scale` to 1.1 (rebuilt |
|              | on every hover edge) so rect and label grow together; a press edge  |
|              | over the button arms it, and an armed release over the button plays |
|              | the swoosh once                                                     |
| tomato_sprite| the tomato sprite with its `tomato_fg.png` overlay, both siblings   |
|              | under a shapeless dummy parent node, both at the same draw order:   |
|              | the tie resolves in tree order, so the later sibling (the overlay)  |
|              | draws on top; each child puts the image px `(308, 411)` on          |
|              | the parent's origin; the parent carries the inverse, so the         |
|              | image's center sits on the window's center                          |
| immortal     | 1920x1080 (Config window_size); right button toggles can / spray    |

`cursor.rs` is the most complete reference demo: `CAN_IMAGE [331,247]` scaled
to 100 px, a 90° CCW tilt tween (0.5 s, rebuilt on press/release edges),
spout at image px (0,25), a 120/s emission accumulator, semi-implicit water
droplets under gravity 420, life-fraction alpha, and a splitmix64 `Rng` for
spread/life/size randomization.

## Load-bearing invariants (don't break these)

1. **Node transform order**: `scale` innermost, then the node's `transform`,
   then the parent's world — `world_at` and `draw_node` both follow it, and
   the `can_transform`-style compositions in examples match it (e.g.
   `cursor.rs` matches the unrotated pointer-follow exactly at angle 0).
2. **Uniform layout**: CPU byte offsets in `frame.rs` ↔ WGSL uniform structs ↔
   naga tests must all agree (`[0,16,24,32,48,64]`, span 80 for shape/sprite).
3. **AA band**: `AA_BAND = 0.75` in `frame.rs` and the `aa` constant in the
   shader sources must stay equal, or scissor boxes stop covering every pixel
   the shaders can write.
4. **`Tween` has no re-target** — rebuild on edges.
5. **Per-draw uniform buffers** are mandatory (write_buffer batch semantics).
6. **Sprite/atlas texture sharing** is keyed by the `Arc<[u8]>` pointer — the
   pixel buffer must be the same allocation (clone the `Arc`, don't copy bytes)
   for the sharing to work.
7. **Text expansion happens before sorting**, so glyphs inherit the node's
   paint position.
8. The **last** `Background` in call order wins as the clear color.

## Direction

`src/TODO.md` is the queue: a `Body` type (velocity, weight, shape) with
energy-conserving collisions via the collision normal. Demo: 50 px-wide wall
rectangles on all four sides plus three random non-overlapping rectangle bodies
at the center with random initial velocities of 50–100 px/s. The collision
module's documented escape hatch remains `rapier2d` if this outgrows it.

## Verification loop

```
cargo build --examples   # expect EXIT 0
cargo test               # expect 102 passed + 2 doctests
cargo run --example cursor   # visual check; closing the window exits 0
cargo run --example sound    # Space/L/+/- check; closing the window exits 0
cargo run --example button   # hover-scale + click-swoosh check; window exits 0
cargo run --example tomato_sprite   # overlay check; window exits 0
cargo run --example immortal   # 1920x1080 window: grass fills it 1:1; exits 0
```
