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
Cargo.toml            deps: gilrs 0.11 (gamepads), image 0.25 (png),
                      log 0.4 (release_max_level_off), rfd 0.17 (native
                      file dialogs), swash 0.2 (text), wgpu 30.0.1,
                      winit 0.30.13; native-only: rodio 0.22 (audio);
                      wasm32-only: web-sys, console_error_panic_hook;
                      dev-deps: clap, env_logger, naga 30.0.0 (shader
                      validation, no GPU needed), serde + ron 0.8 (the
                      immortal example's save file)
src/lib.rs            crate docs + public API: Canvas, Context, Process, Config,
                      run / run_configured, draw_scene, paint_order, expand_text,
                      gamepad re-exports (gilrs Axis / Button / Gamepad)
src/objects.rs        Color, Transform, Shape, Light, ParticleShape,
                      SpriteFilter, Node, SceneNode, Layer, Scene, NodePath,
                      world_at / camera_world
src/tween.rs          Tween<T> (f32 / [f32;2]) with Repeat modes
src/particles.rs      Particle, ParticleSystem (pure simulation)
src/collision.rs      OrientedBox, Circle, Collider, push_out, reflect (pure math)
src/rng.rs            Rng — seedable splitmix64, state get/set for save games
src/diagnostics.rs    Diagnostics (flag-gated FPS/FT/PROC/DRAW HUD overlay
                      with folded strip charts), DiagnosticsFlags
src/ui.rs             Ui immediate-mode widget layer (button, checkbox,
                      slider, label, draggable panel), UiStyle
src/audio.rs          Audio (device + loop player), Sound (decoded buffer),
                      AudioError — native-only, rodio-based
src/text.rs           CPU text shaping/rasterization on swash, glyph shelf atlas
src/shaders.rs        include_str! of the 7 WGSL sources + naga layout tests
src/backend/mod.rs    `app`, `frame`, `wasm` (wasm32 only), `tests` (test only)
src/backend/app.rs    Frost<P>: winit app handler + GPU state + render loop
src/backend/frame.rs  Draw list model: Draw enum, scissor rects, uniform writers
src/backend/wasm.rs   WebFrost: deferred async GPU setup + fallback DOM helpers
src/backend/tests.rs  GPU-free backend tests (uniform layout, canvas behavior)
shaders/*.wgsl        line, circle, rectangle, shape (SDF circle+rect), sprite
examples/             45 runnable demos, filed into folders (see table below)
assets/               sprites/*.png (+ .pxo sidecars; immortal art, worm
                      crops, …), fonts/ (FiraCode Variable, JameGem08,
                      Leofont + licenses), audio/*.wav (swoof, waterflow,
                      bug/viper clips, …), sprites/plant1..5.ron (the
                      immortal plant's anchor files), ron/garden.ron
                      (ron_view)
src/TODO.md           next planned feature (Body / rigid bodies)
```

The library uses only the `log` facade; consumers wire up their own logger
(`env_logger` in the examples). No randomness crate either: `frost::Rng`
(`src/rng.rs`) is a splitmix64 — pure 64-bit arithmetic, identical on every
target; `Rng::new()` seeds from the clock, `Rng::with_seed(seed)` fixes the
stream for reproducible runs and tests, and `state()` / `set_state()` let a
save file carry the stream across (`immortal` does exactly that).

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
  - `mouse_wheel() -> f32` — vertical wheel movement since the previous
    frame, in lines (positive = up). It sums every wheel event of the frame
    (`LineDelta` as-is, `PixelDelta` at 120 px/line) and is reset after the
    frame's `process`, so an unread frame discards its delta.
  - `window() -> Option<&Window>` — the engine's winit `Window` (`None` only
    before the first frame exists): parent native dialogs to it (e.g.
    `rfd::FileDialog::set_parent`, as `sprite_util` does) and read any window
    state the platform exposes — `inner_size()` / `scale_factor()` for
    pixel-exact math (the `worm` example wraps at the window's *physical*
    edges converted through the scale factor).
  - `gamepads() -> impl Iterator<Item = Gamepad>` — the connected
    controllers (gilrs): each reports its pressed buttons and axis values,
    updated with the frame's events; empty when none is connected or the
    platform's input devices could not be opened at startup. `Axis`,
    `Button`, and `Gamepad` are re-exported from gilrs.
  - `expected_fps() -> Option<f32>` — monitor refresh rate when vsync is on.
  - `frame_processing_ms() -> f64` — the engine's CPU time for the last
    completed frame (its own probe; see "Diagnostics" below).
  - `frame_draw_calls() -> u32` — the engine's GPU draw calls for the last
    completed frame (its own probe; see "Diagnostics" below).
  - `frame_diagnostic_draw_calls() -> u32` — the subset of those spent on
    the `Diagnostics` overlay's own nodes (its own probe; see "Diagnostics"
    below).
  - `size() -> (f32, f32)` — window size in pixels.
- Re-exports: `KeyCode`, `MouseButton`, `Axis`/`Button`/`Gamepad` (gilrs),
  `Rng`, `Tween`/`Repeat`, `ParticleSystem`/`Particle`, `Ui`/`UiStyle`, and
  everything from `objects` (`Scene`, `SceneNode`, `Node`, `Shape`, `Light`,
  `Color`, `Transform`, `Layer`, `Canvas`).
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

- `Scene { root: SceneNode, layers: Vec<Layer>, camera: Option<NodePath>,
  ambient: Color }` — `ambient` is the light floor every lit receiver adds
  under itself (a dim neutral gray by default; see "Lighting" below).
  `Scene::visit()` updates the root subtree and every layer subtree.
- `SceneNode` — a `transform` + `scale` + `modulate` + `order`, an optional
  `shape`, and `children`. All four are **relative to the parent**. Three
  per-node lighting flags (`lit`, `occludes`, `glow`) join them; unlike these
  four they apply to the node's own shape only and never propagate (see
  "Lighting" below):
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
  - `Polyline { points, width, color }` — straight segments through the
    points (local space), stroked at `width`, one draw call for the whole
    chain (max 128 points; fewer than two draws nothing). Width scales with
    the node's geometric-mean scale axis.
  - `Particles { system, color, shape: ParticleShape }` — an instanced
    particle batch (one draw per batch; the game updates `system` each
    frame). `ParticleShape` picks the primitive each instance draws:
    `Circle` (radius = the particle's size), `Rectangle { aspect }`
    (`size × 2` wide, `size × aspect × 2` tall), or a sprite. Each
    `Particle` carries its own `angle`, so instances rotate individually —
    spinning squares just advance it (see
    `examples/particles/square_fountains.rs`).
  - `Background { color }` — fills the window, **ignores all transforms**,
    drawn at the very back: at render time it becomes the frame's clear color
    (the last one in depth-first call order wins; default is a dark blue-gray
    when none exists). It costs no draw call.
  - `Sprite { data, width, height, color, alpha, filter }` — RGBA8 PNG from
    `Shape::sprite(path)` / `sprite_bytes(bytes)` / `sprite_nearest` /
    `sprite_bytes_nearest` (decoded eagerly; `SpriteError` on failure).
    `filter: SpriteFilter::{Linear, Nearest}` picks the GPU sampler —
    Nearest keeps pixel art crisp when scaled.
    Centered on the node origin, 1 texture pixel per scene pixel. `color` tints
    every pixel (white = unchanged); overall opacity = texture alpha × `alpha`
    × `color.a`.
  - `Light { light: Light }` — contributes the node's [`Light`] to the
    frame's light field; never drawn itself (see "Lighting" below).
  - `Text { text, font: Arc<[u8]>, size, weight, color, alpha }` — from
    `Shape::text`/`text_bytes` (swash, CPU-only), thickened with
    `Shape::with_weight` (the `wght` variation axis; 400 Regular, 700 bold —
    inert on static fonts). Centered on the node origin;
    each glyph becomes a tinted quad over a shared atlas.
  - Color model: plain shapes emit `coverage * a`; sprites/text emit
    `texture_alpha * opacity * a` — all composited with `ALPHA_BLENDING`.

## Lighting (src/objects.rs + the backend)

A per-pixel light field, evaluated inside the *existing* shape/sprite/particle
pipelines — no extra pass, pipeline, or draw call, and skipped light draws
never count (`Diagnostics` excludes them like backgrounds).

- A `Shape::Light` node contributes a `Light { color, intensity, radius,
  direction, spread, softness, penumbra }` at the node's local origin: the
  node's transform (and ancestors') places it, rotates its beam, and tints it
  through the composed modulate. Three kinds: **omni** (`spread ≥ TAU`, the
  default — lights every pixel within `radius`, fading to zero at the edge),
  **cone** (narrower `spread` around `direction`, edges feathered by
  `softness` — a torch), and **directional** (`Light::directional`, marked by
  a negative `radius` sentinel: parallel rays, no falloff, position ignored —
  a sun; a scene may carry any number). `penumbra` is the light disk's radius:
  shadow rays sample it (nine taps, rotated per pixel), so shadows fade
  across an edge instead of dropping hard.
- Receivers opt in per node, flag applies to that shape only:
  `SceneNode::lit` multiplies the shape's color by `ambient + Σ lights +
  glow` per pixel; `SceneNode::glow` (a `Color`, alpha-scaled) is the shape's
  own emission — visible in the dark, never shadowed, never spilling onto
  neighbors (attach a `Shape::Light` child for that); `SceneNode::occludes`
  makes a rectangle's transformed silhouette block every light's path to
  pixels behind it — a shadow caster. A wall can be `lit + occludes` at once.
- The backend packs the frame into two **storage buffers** bound to every
  draw: the light field (32-byte header — count and the scene's ambient —
  plus one 48-byte record per light) and the occluder field (16-byte header
  plus 48 bytes per occluder). Both start at their binding minimum and grow
  only to the peak count, so a lighter frame writes a shorter slice into the
  same buffer; the shaders read ambient and the lists from these fields.

## The draw list (src/backend/frame.rs)

`Canvas` holds `draws` (the **base group**: immediate methods + every scene's
root subtree, mixed) plus one list per explicit layer (`layer_draws`,
`layer_orders`).

- Immediate methods: `line`, `circle`, `rectangle` (pixel args converted to
  pixel space via `user_to_pixels`), `text` (records a `Draw::Text` with a
  translate-only user-space transform, expanded by `expand_text` like a
  node's text), and `draw_scene`.
- `Draw` enum: `Line`, `Polyline` (the batched line: `points: Vec<[f32; 2]>`
  in pixel space, `width`, `color` — consecutive pairs joined by straight
  segments, stroked by one full-screen-triangle draw against a fixed
  128-point uniform array — the points are packed as vec4s (x, y, 0, 0)
  because the uniform address space requires an array stride that is a
  multiple of 16 bytes; fewer than two points draws nothing, more than 128
  draw the first 128), `Circle`, `Rectangle`, `Shape` (transformed SDF
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
- **Uniform writers** (`line_uniform_data`, `polyline_uniform_data`,
  `circle_uniform_data`, `rect_uniform_data`, `shape_uniform_data`,
  `sprite_uniform_data`) hand-write
  little-endian bytes at explicit offsets matching the WGSL **uniform-space**
  layout (e.g. ShapeUniforms: `to_local` mat2x2 @0 (16B, 8-aligned columns),
  `translation` @16, `center` @24, `params` @32, `color` vec4 @48, `misc` @64;
  total 80. Sprite the same with `size`/tint/`alpha`/`uv_rect`). Line/circle
  are 48 bytes, rectangle 32. The polyline writer packs 2080 bytes:
  `points` @0 (128 × vec4 — 2048 bytes), `color` @2048, `count` @2064 (u32),
  `width` @2068.

## The GPU side (src/backend/app.rs, shaders/)

- `Frost<P>` holds the wgpu instance/adapter/device/queue, `vsync` and
  `window_size` (the `Config` values), the window (`Arc<Window>`), logical
  size + scale factor, the surface, the seven pipelines
  (line/polyline/circle/rect/shape/sprite/particles), `sprite_resources` (a
  `HashMap<(u64, u64), (TextureView, Sampler)>` keyed by the sprite pixel-data
  `Arc`'s pointer plus its generation — `0` for static images, bumped by the
  glyph atlas on every repack — so a freed-and-recycled buffer address can
  never hit a stale texture; sprites from the same file, and glyph quads from
  the same atlas, **share one uploaded texture**), `text_atlases`
  (`HashMap<(font ptr, size bits), text::Atlas>`, kept between frames so
  unchanged text never re-rasterizes), held keys/mouse, and the user's
  `process` + `scene`.
- All seven pipelines are the same shape: a **full-screen triangle** vertex
  shader (no vertex buffer), one `@binding(0)` uniform (plus texture + sampler
  for sprites, plus an instance buffer for particles), alpha blending, no
  depth. `present_mode_for(vsync)` maps to `AutoVsync`/`AutoNoVsync`.
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
  buttons, mouse wheel (accumulated in lines per frame, reset after the
  frame's `process`), `CursorLeft`/`Focused(false)` **clear** the held state
  (no stuck controls), resize/scale-factor reconfigure the surface (pipelines
  rebuild only if the format changed), `RedrawRequested` → `render()` +
  re-request.
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

- `layout(font: &[u8], text, size, weight) -> Option<TextLayout>` shapes the string into
  `PlacedGlyph { id, x, y }` (pen origin at baseline left, y up) and returns
  metrics `{ ascent, descent, width, glyphs }`.
- Each unique glyph is rasterized **once per (font, size, weight)** into a shared 512px
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
the established pattern (e.g. `examples/inout/cursor.rs`) is to **rebuild the
tween**
on the press/release edge with the current value as the new `from`.

## Particles (src/particles.rs)

`Particle { pos, vel, life, max_life, size, angle }`; `ParticleSystem { particles: Vec }`
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

Seven small WGSL files, all full-screen-triangle + SDF/sampling:

| file        | pipeline   | uniforms (byte layout)                                              |
|-------------|------------|---------------------------------------------------------------------|
| line.wgsl   | line       | `a`@0 `b`@8 `color`@16 `width`@32 → 48                             |
| polyline.wgsl | polyline | `points`@0 (128 × vec4, 2048B) `color`@2048 `count`@2064 (u32) `width`@2068 → 2080 |
| circle.wgsl | circle     | `center`@0 `color`@16 `radius`@32 → 48                             |
| rectangle.wgsl | rectangle | `center`@0 `extent`@8 `color`@16 → 32                            |
| shape.wgsl  | shape      | `to_local`@0 `translation`@16 `center`@24 `params`@32 `color`@48 `misc`@64 → 80 |
| sprite.wgsl | sprite     | `to_local`@0 `translation`@16 `size`@24 `tint`@32 `alpha`@48 `uv_rect`@64 → 80 |
| particles.wgsl | particles | `size`@0 `color`@16 `misc`@32 `lit`@40 → 48 (one instanced draw per batch) |

The polyline's fragment shader walks its 128-point uniform array and takes the
minimum point-to-segment distance — a stroke of any length is still one draw
call; its scissor is the point bounding box plus half the stroke width and the
AA band, so the fragment loop only runs over the pixels the stroke can write.

`src/shaders.rs` includes them and has tests that **parse all seven with
`naga::front::wgsl::parse_str`** (the same frontend wgpu uses, so validity is
pinned without a GPU) and assert the `ShapeUniforms`/`SpriteUniforms` member
offsets are `[0,16,24,32,48,64]` spanning 80 bytes, the `ParticlesUniforms`
offsets are `[0,16,32,40]` spanning 48, and the `PolylineUniforms` offsets are
`[0,2048,2064,2068]` spanning 2080 with a fixed 128-point vec4 array — the **CPU
uniform writers must mirror these layouts**; the GPU-side mirror tests live in
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

## Diagnostics (src/diagnostics.rs)

`Diagnostics` is a HUD overlay whose statistics are each gated by a bit of
the `DiagnosticsFlags` set passed to the constructor. The window size line
(`"{w}x{h}"`) is always shown, and below it the enabled statistics in fixed
order, all at 32 px: the smoothed frame rate (`"FPS {fps}"`, the `FPS`
bit), the current frame time (`"FT {ms}ms"`, the `FT` bit), the last
frame's total processing time (`"PROC {ms}ms"`, the `PROC` bit), that time
excluding the overlay's own update cost (`"APP {ms}ms"` — the companion line
to `PROC`, on the same bit), and the last frame's GPU draw-call count
(`"DRAW {n}"`, the `DRAW` bit). Under the lines, one scrolling ten-second
strip chart per enabled statistic group, top to bottom: frame rate (orange),
frame time (green), the folded processing-time chart, and the folded
draw-call chart. The two folded charts show three polylines on one shared
scale — the total (blue / magenta), the app's share (violet / pink), and
the overlay's own share (cyan / deep rose) — so the overlay's cost is
readable as the vertical gap between the total and the app lines; the FPS
and FT charts are single series, and the three time charts carry a
reference line at 60 fps / 16.7 ms (the draw-call chart has none — draw
counts have no universal budget). The processing times and the draw-call
counts are the engine's own probes; the APP lines and the diagnostic series
are the overlay subtracting its own measured costs from those probes — see
the probe bullets below. The whole integration is one struct in the demo
state plus one `self.diag.process(ctx, dt)` call per frame — the overlay
itself is a `Process`.

- `DiagnosticsFlags` is a hand-rolled four-bit set — `FPS (1 << 0)`,
  `FT (1 << 1)`, `PROC (1 << 2)`, `DRAW (1 << 3)` — with `ALL`, `NONE`,
  `none()` / `all()`, `contains()` / `is_empty()` / `insert()` /
  `remove()`, and `|` / `|=` (no dependency). `Diagnostics::new(path,
  flags)` reads the font file up front (checked with swash, so a bad path
  or file is a `TextError` at construction); `from_bytes(&[u8], flags)` is
  the no-filesystem twin (embedded `include_bytes!`, the wasm path). The
  bytes live behind an `Arc` shared with the readout's `Shape::Text`. The
  engine's probes always run; the flags gate only what the overlay
  displays, and an off flag costs nothing on the display side.
- `Process::process` smooths the frame rate as an exponential moving average
  of `1.0 / dt` (time constant 0.25 s, snapping to the first real
  measurement so the first frame does not flash a zero; `dt` 0.0 on the
  first frame is skipped; `dt` past a 0.25 s stall is skipped by the
  smoothing), records the raw frame time for the `FT` line, reads the
  engine's processing-time and draw-call probes (total and diagnostic),
  derives the `APP` line and the diagnostic series from the previous call
  (see the probe bullets), and appends a `Sample` (elapsed time, frame
  time, total/diagnostic/app processing times, total/diagnostic/app
  draw-call counts) to a history trimmed to the last 10 s. Each line's
  shape is rebuilt only when its text changes — the frame-time line shows
  the raw last gap, so it usually changes every frame.
- The processing-time probe lives in the backend, because the demo's
  `process` callback runs *inside* the frame — a demo cannot time its own
  frame from inside it. `Frost` measures `now_millis()` around its frame
  work in `render` (from just after the surface acquire — the acquire may
  wait on the display's back buffer, and that wait is not processing — to
  the `queue.submit` of the frame's command buffer; the present handoff and
  the wait for the next frame are excluded) and stores the result as
  `frame_processing_ms`, which the next frame's `Context` reports as
  `Context::frame_processing_ms()`. The draw-call probe counts in the same
  pass: one increment per `pass.draw` / `pass.draw_indexed` that actually
  executes (seven sites in the render loop — line, polyline, circle,
  rectangle, shape, sprite, particle batch), so draws skipped as off-screen,
  background, light, or empty (a degenerate transform, an empty particle
  batch) never count. It keeps two counters — `frame_draw_calls` and
  `frame_diagnostic_draw_calls`, reported as
  `Context::frame_draw_calls()` and `Context::frame_diagnostic_draw_calls()`
  — and the diagnostic one increments only when the draw's node carries the
  `SceneNode::diagnostic` marker, which the overlay sets on the nodes it
  appends to the scene. The marker is a public but engine-managed field
  (public so node literals with `..Default::default()` keep working from
  outside the crate; demo code leaves it at the default `false`), and it
  rides onto every draw the tagged nodes produce — the per-glyph quads of
  the text lines included — so the diagnostic count is a true subset of
  the total and the app's share is `total − diagnostic`. The overlay
  therefore always shows the last *completed* frame's numbers — a one-frame
  lag, one 0.1 s column at 60 fps, invisible in the graphs. `0.0` / `0`
  until the first frame completes (one `PROC 0.00ms` / `DRAW 0` flash, same
  as the `FT 0.0ms` flash).
- The APP value is the one probe the overlay makes itself, and it must be
  one frame behind for the same reason: it times its own `process` call
  with the engine's `now_millis` (crate-internal, shared with the backend)
  and stores the duration as `prev_self_ms`; the next call subtracts it from
  the engine's total probe of the *same* completed frame, clamped at zero
  against timer jitter, so PROC and APP always describe the same frame. It
  removes the overlay's per-frame *update* work (readout layout, sample
  binning, node placement); the overlay's own shape draws still count in
  the total — they go through the engine's per-draw path like everything
  else — which is what the diagnostic series on the folded charts shows.
- The strip charts are the history binned by time: `slice_max` takes the max
  of a per-sample projection over each 0.1 s column across the last 10 s
  (the fps column is the max of `1000 / frame_time`, i.e. the frame's
  fastest rate; the other columns project their own field), so a spike is
  visible as a high point, and the newest frame lands in the newest column.
  Each series' 100 column maxima become one `Shape::Polyline` (a point per
  column center, the value scaled to the panel height with a 1 px inset) —
  one draw call per series instead of 200 per-frame rectangle draws, because
  the cost of a draw in this engine is on the CPU side (a fresh uniform
  buffer + bind group + scissor/pipeline state per draw), not in the
  fragment work. Each folded chart keeps one scale: processing time is
  0–20 ms (the reference line at 16.7 ms), draw calls a fixed 0–128 (clipped
  above it — the readout always shows the true count).
- Node management: the slot lists are built once at construction from the
  flags — the lines in order (the size line first, then the enabled
  statistics), one panel per enabled chart, one reference line per chart
  except the draw-call chart, one polyline per series (one for the FPS and
  FT charts, three for the folded ones) — so all four flags give at most 21
  nodes (six lines, four panels, three references, eight polylines), and
  `none()` gives a single line. The first `process` creates a **dedicated
  topmost `Layer`** — ordered one past the highest existing layer (or above
  the root subtree when there are none) — and appends the nodes to that
  layer's root, remembering the indices; later calls update each node's
  shape and transform in place. A stale index (the demo rebuilt the scene's
  layers) recreates the layer and re-appends the nodes. Because a `Layer` is
  a hard draw partition, the overlay can never be buried under a
  high-order app node, and the demo is free to reorder its root's children.
  Placement uses crate-internal `text::layout` for each line's width; the
  lines' vertical extent is measured once, at construction, over every
  character a readout line can display, because neither the current text's
  ink (Fira Code's "8" and "9" carry a pixel of antialiasing below the
  baseline where the "4" does not) nor the font's metrics (both bundled
  fonts report degenerate ones — Leofont's ascent plus descent is about a
  pixel at 32 px) is a stable extent — so the topmost *visible* line's ink
  sits 20 px in from the window's top-left corner, and the visible parts
  reflow into that corner whenever one of them is hidden.
- Keyboard shortcuts: the overlay reads six key combinations from the
  engine's key state on every `process`, acting on press edges only (a
  held combination toggles once): Alt-0 is the master switch — while off,
  every node draws nothing and the per-part toggles are remembered as-is —
  Alt-1..Alt-4 toggle the charts by position among the enabled charts (top
  chart first; the extra keys are no-ops when fewer than four charts are
  enabled), and Alt-T toggles all the readout lines, the window-size line
  included. The charts are independent of the text toggle. The layout
  reflows around whatever is hidden — the visible lines and charts re-pack
  into the corner — while the node indices stay fixed to the construction
  slot order; a hidden node keeps its place as a bare pivot (`shape:
  None`). While the text is hidden, the lines are not rebuilt, and a
  re-shown line re-lays out its last text the next visible frame. The
  shortcuts are read, not consumed: a demo that wants the same
  combinations still sees them. They are also available as the methods
  `toggle_all()`, `toggle_chart(index)`, and `toggle_text()`.
- Pure CPU work (no device I/O), so it compiles on `wasm32` too — no cfg
  gate, unlike `audio`.

`examples/diagnostics.rs` shows it over a swaying circle, set in
`assets/fonts/FiraCode-VariableFont_wght.ttf`, with `DiagnosticsFlags::all()`;
`examples/particles/square_fountains.rs` runs it over a live particle scene.

## UI (src/ui.rs)

`Ui` is an **immediate-mode widget layer** over the frame's `Canvas`: the
app owns one `Ui` in its `Process` struct, calls `Ui::begin(ctx)` at the top
of `process` (snapshot the mouse, resolve last frame's press, move the
dragged panel), then declares widgets each frame — `button` (armed-release
click, as in `examples/button.rs`), `checkbox` (drives `&mut bool`),
`slider` (label left, value readout right, drag-follows-pointer, drives
`&mut f32`), `label`, `space`, and `panel`. Returns: `true` on the frame a
button clicks, a checkbox toggles, a slider's value changes.

- **Panels** are titled boxes anchored at a center `at` until the user
  **drags them by the title bar**; the dragged position is retained per
  title (hashed id). The body is as tall as the content asked for *last*
  frame (first frame shows the title bar only, then it snaps open), and it
  grows **downward from the anchored title bar**. A **click** on the title
  bar (a release that barely moved — measured from where the press began,
  so dragging never folds) **folds** the panel to just its bar and folds it
  back; a chevron on the bar shows the state. Widgets outside any panel
  stack in a root column at the window's top-left. A
  panel **claims its whole area** (body included): presses land on the
  panel, never on a widget or the game behind it, and `Ui::hovering()` —
  the pointer over any widget or panel — lets a raw-input game tell UI
  clicks from scene clicks (see `examples/sprite_util.rs`). Two accessors
  reach a panel from outside: `Ui::panel_position(title)` reads where it
  sits, `Ui::panel_rect(title)` reports the plate as it was PAINTED last
  call (the height the drawing used — content laid over a plate must
  trust this, not its own same-frame request, or it floats a frame
  ahead of the pixels; see `examples/sprite_util.rs`), and
  `Ui::set_folded(title, folded)` is the program's hand on the title-bar
  click — code that closes or restores a panel uses it so a reopened
  title never greets you folded.
- **The input model** is one frame delayed: a press is resolved at the next
  `begin` against the rects the previous frame registered, last-declared
  (topmost) first — so overlapping panels route presses to the visible one,
  and a press that leaves a widget keeps its drag until release (sliders,
  panel drags). Hover, click and hold all read this resolved state.
- **Tables** (`Ui::table(ctx, id, cols, content)`) lay the widgets declared
  in `content` into a grid: each consumes the next cell, wrapping to a new
  row after `cols.len()`. Each `Col` has a [`ColSize`] — `Px` (fixed),
  `Auto` (as wide as its widest cell, from last frame, matching the UI's
  one-frame model) or `Stretch(w)` (a weighted share of the leftover) — and
  an [`Align`] (`Left`/`Center`/`Right`) honored by text. `slider_track` and
  `readout` are the label-less pieces for a `[label | track | value]` row
  (see `examples/sprite_util.rs`); `id` scopes the retained auto widths so
  two tables in one panel keep their own columns.
- **Identity**: widget ids are FNV-1a hashes of the label scoped by the
  containing panel's id, so the same label in two panels is two widgets.
- **Text** goes through `Canvas::text` (see above) with one shared
  `Arc<[u8]>` font (`Ui::from_font` / `from_bytes`, validated at
  construction), so all UI text shares one cached glyph atlas; measured
  widths are cached per `(string, size, weight)`; the weight is
  `UiStyle::font_weight` (default 600 — a SemiBold on a variable font like
  FiraCode, inert on static fonts).
- **Order**: every UI draw gets a `z` counting up from
  `UiStyle::base_z` (default 10 000) in declaration order — the UI paints
  over the scene, panels stack in declaration order.
- `UiStyle` is one `Copy` struct of colors and metrics (dark default),
  reachable through `ui.style`.

## Testing

Baseline: **183 tests + 5 doctests** passing, `cargo build --examples`
clean. Notable test areas:

- `src/shaders.rs` — naga parse + device-side validation (the
  `Validator` stage wgpu runs in `create_shader_module` — this is what
  catches uniform-address-space layout rules the parser never checks) +
  uniform-offset assertions (above).
- `src/backend/tests.rs` — GPU-free: uniform-layout mirrors, scissor math,
  `paint_order`, `expand_text` (splicing + atlas reuse across frames),
  `context_reports_held_mouse_button`, `context_reports_mouse_wheel_delta`.
- `src/collision.rs` — push-out separation, reflect restitution semantics.
- `src/tween.rs`, `src/particles.rs`, `src/text.rs` — behavior unit tests.
- `src/rng.rs` — a fixed seed replays its stream; `state()`/`set_state()`
  round-trips continue it exactly.
- `src/audio.rs` — device-free decode tests (synthetic WAV, bundled
  `swoof.wav`, error variants) + the `load_bytes` doctest.
- `src/ui.rs` — the interaction model driven frame by frame through
  `begin_input` (press arming, armed release, hold past the rect, topmost
  overlap wins, one-frame click, panel drag moves by the pointer delta and
  retains), plus id hashing and rect math.

## Examples (examples/)

45 demos, filed into subject folders — `shapes/`, `lighting/`, `inout/`,
`layers/`, `particles/`, `physics/`, `text/`, the `immortal/` game, and the
tools (`sprite_util`, `worm`, …) at the top level. Cargo only auto-discovers
`examples/*.rs` and `examples/*/main.rs`, so every folder member is named
with an explicit `[[example]]` entry in `Cargo.toml` — the short names keep
working: `cargo run --example parallax`. `ron_view/` is the exception that
proves the rule: a `main.rs` folder target needs no entry, and its
`tree.rs` — the RON-subset parser, tree model and row flattener (every
value carries its source comments through parse, edit and save) — is
path-included by `sprite_util` too (`#[path = "ron_view/tree.rs"]`), so
both examples parse sidecars and samples with one shared, dependency-free
parser. Most load their
assets from disk at runtime, pinned to the crate root via
`CARGO_MANIFEST_DIR`; `immortal` and `text_web` embed their assets into the
binary with `include_bytes!` instead, so they run with no asset files on
disk. Run with `cargo run --example <name>`
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
| text_leo     | the same text set in `Leofont-Regular.ttf` — one of the three     |
|              | bundled faces (with JameGem08 and FiraCode Variable)                  |
| player       | WASD drives a Button sprite from `(0, 0)`                          |
| layers       | obstacles + rendering layers                                       |
| parallax     | layers + camera: infinite parallax scrolling                       |
| repeat       | a layer tiling itself (`Layer::repeat`)                            |
| scene        | an in-place animated scene tree (orbiting children)                |
| satellite    | the `scene` tree refactored into reusable nodes                    |
| sprites      | PNG sprites through the scene tree                                 |
| tomato       | a plant assembled from three sprite slices                         |
| grow         | the tomato plant growing slice by slice                            |
| collision    | the player with `push_out`/`reflect` collision                     |
| cone_collider| the `cone` hall with physics added: the walls that cut shadows out  |
|              | of the beam also push the player back (lighting × collision)        |
| lighting     | a lit floor, a lit ball with an ember plume, three lights sweeping  |
|              | across them — the feature tour                                      |
| lit_unlit    | two same-color rectangles — one `lit: true` — and a light on the    |
|              | mouse: the receiver flag in isolation                               |
| bouncing_lights | three lights ricocheting around the window like billiard balls  |
| cone         | a square explorer carries a torch (a cone `Light`) through a dark   |
|              | hall of occluding walls                                             |
| dawn         | one directional light as a day cycle: sunrise, climb, sunset, the   |
|              | world warming and cooling with it                                   |
| near_sun     | the same day told by a *near* sun — a point light on the visible    |
|              | disk, so shadows fan out radially                                   |
| occlusion    | lit panels behind tall occluder walls, one lamp orbiting through    |
|              | them: hard vs. penumbra shadows                                     |
| twostars     | a planet under two stars: two directional lights, both on stage     |
| eyes         | glowing eyes drifting through the dark — a tour of `SceneNode::glow`|
| eyes_light   | the companion: some eyes are real emitters (`Shape::Light` too)     |
| particles    | a `ParticleSystem` fountain with life-fraction alpha               |
| fountains    | three overlapping fountains + a full-screen waterfall, each one    |
|              | CPU-simulated batch (one instanced draw per `Shape::Particles` node)  |
| square_fountains | the middle fountain spits rotating squares, each in its own hue: |
|              | per-particle `Particle.angle` advanced by a side `spins` vec kept in |
|              | lockstep with the sim's dead-particle removal; runs the full        |
|              | `Diagnostics` overlay over the scene                                |
| cursor       | a watering-can cursor: mouse-following sprite, press-to-tilt tween, |
|              | spout-emitted water particles over a full-screen grass field       |
| torch        | two kinds of particles side by side: embers that ride the torch and |
|              | sparks that keep flying once they leave it                          |
| worm         | one sprite crawling by peristalsis — a stretch anchored at the rear |
|              | and a contract anchored at the front (matched so the center glides  |
|              | smoothly), a 2 px bob at twice the beat, wrapping at the window's   |
|              | physical edges (scale-factor honest); 960×540 via `Config`          |
| sound        | one-shot + looping playback of a decoded WAV: Space re-triggers     |
|              | (pulsing circle), L toggles the loop (wobbling ring), +/- the bar   |
| audio_test   | a raw rodio scratchpad: plays WAV/MP3 straight through rodio,      |
|              | beside (not through) frost's `Audio` wrapper                        |
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
| immortal     | 1920x1080 (Config window_size); four tools ride the six-slot shelf: |
|              | items.png split 0..5 top-down, the tweezers, spade, spray can, and  |
|              | watering can resting in slots 2-5 and 0-1 left free to park one, a  |
|              | left click on a slot swapping it with the held tool and the right   |
|              | button trading held for stored; each sprite is fitted to its cell by|
|              | its drawn alpha box, not its canvas; a fresh left click digs one    |
|              | spade stroke - `dig_pose` twines a rotation about the handle's grip |
|              | with a lunge along the blade's own line, starting from the angle the|
|              | art is drawn at (point 34.9 deg under level, so the plunge that puts|
|              | it straight down is 55.1 deg, not 90); the tweezers ride level by   |
|              | their jaws and stay inert until there are pinch frames to pick with;|
|              | plants grow one at a time, each slice opening its flowers once fully|
|              | grown (the four lower slices' spawn points, the top slice bearing   |
|              | none, populated with `flower.png`); one viper per fully grown plant |
|              | layer orbits the row; bug and louse swarms pop up out of the grass in|
|              | batches of three per plant each (the bugs module is generic over a  |
|              | Species: the lice of Lice1/Lice2.png are smaller, untinted, left-   |
|              | facing art whose facing flip mirrors the bugs'), growing over 0.75 s;|
|              | each pop-up - a batch spawn or a respawn - plops on a random plopp  |
|              | clip (bugs_plopp1/2/3.wav); spray mist touching a bug wounds it -   |
|              | three starting hits, one per 0.2 s per bug, a bug at its park spot  |
|              | regaining one hit every 5 s, a louse two hits every 10 s (lice start|
|              | at two health), both capped at six, a white pip above the bug per hit|
|              | it can still take; a bug reaching its park spot while another bug is|
|              | within 50 px of it chatters - a random tjatter clip (TjatterLow/Mid/|
|              | High.wav) plays - and the killing blow starts the two-phase death:  |
|              | over 0.5 s it bounces up off the grass, flipping upside down in the |
|              | air (y scale to fully inverted, position/growth/facing/frame frozen),|
|              | landing on its back; then over 0.5 s the inverted sprite evaporates,|
|              | shrinking to nothing while its center sinks through the grass; after|
|              | a random 5-10 s delay the bug pops back up at its spawn spot, fully |
|              | healed; the five slices' joints and flower anchors load at startup  |
|              | from the embedded `assets/sprites/plant1..5.ron` files (serde + ron —|
|              | the hand-picked constants are gone); a 14-slot worm swarm burrows up|
|              | and walks the bench on the `worm` example's stride — the rear grips |
|              | while the body reaches, the front holds while the contract slides it|
|              | up — two poses riding the stride's halves, hull-picked spawn/target |
|              | spots off a seedable `Rng`, and digs back in; every draw order      |
|              | composes through the `zorder` module's bands (ground = −y, bench    |
|              | slots on top, grass/UI/overlay pinned outside); F5 / F9 save & reload|
|              | the whole game as versioned RON under `~/.frost/immortal/` — the    |
|              | snapshot carries the random streams, so a reload continues each     |
|              | stream exactly (`--load` loads at startup)                          |
| sprite_util  | up to seven picked PNGs (rfd dialogs): the window splits into a     |
|              | work area and a bottom strip of 100x100 slots holding the           |
|              | minimized originals (click a slot to activate, drag one onto        |
|              | another to swap). The active sprite shows over a checkerboard with  |
|              | `Ui` View and Operations panels (Open / Crop / Save / Save As /     |
|              | Close): a left-drag draws a crop selection (a still click logs the  |
|              | texture pixel), Ctrl-O opens into the next slot, Ctrl-Z undoes      |
|              | crops, Save/Save As write PNG over a native confirm. An Animation   |
|              | panel builds frames — each a set of layers with its own hold-time —  |
|              | played on a clock that loops or ping-pongs (space plays), previewed |
|              | live in the work area. A sprite with a sidecar `<name>.ron` beside  |
|              | its PNG gets its own `Ui` panel titled by the file — one view per   |
|              | open file, cascading from the lower right, each dragging, folding,  |
|              | scrolling by its own pair of handles (wheel too, Shift-wheel         |
|              | sideways) and resizing by a corner grip; the boxed × in a view's    |
|              | bar closes file and panel together — laid out on the plate the      |
|              | UI actually painted, never a frame ahead of it                      |
|              | Every `(x, y)` the file names is marked on the sprite in the colour |
|              | its rows carry: pick a row and the sprite centres the point;        |
|              | right-click a marker to pick it back — the tree unfolds to the      |
|              | row and scrolls it to centre. Left-click the sprite to move the     |
|              | position, Escape lets go; other rows fold on click. Save /          |
|              | Save As write the tree back beside the PNG with its comments —      |
|              | the parser keeps leads, trails, tails and the header, the writer    |
|              | puts them back. Sequences wear + / × row buttons and entries        |
|              | reorder by press-drag; a list entry's marker shows its seat         |
|              | number on the sprite                                              |
| widgets      | the `Ui` layer: a draggable Tomato panel (three color sliders, a    |
|              | Spin checkbox, a Speed slider, a Reset button) and an About label   |
|              | panel drive a spinning face's color and rotation                    |
| ron_view     | a foldable, scrollable tree view of a `.ron` file (`tree.rs`,       |
|              | shared with `sprite_util`'s sidecar panels): click `[-]/[+]`        |
|              | rows to fold, drag the two scroll handles (or wheel / shift-wheel), |
|              | text in FiraCode Variable — keys & heads at weight 700, values at   |
|              | 500; ships with a dependency-free RON-subset parser (comments,      |
|              | enums, maps, escapes; line-numbered errors) and `assets/ron/        |
|              | garden.ron` as its sample; `-i` opens any .ron                      |

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
6. **Sprite/atlas texture sharing** is keyed by the `Arc<[u8]>` pointer plus
   the buffer's generation (0 for static images, bumped by `text::Atlas` on
   every repack) — the pixel buffer must be the same allocation (clone the
   `Arc`, don't copy bytes) for the sharing to work. When an atlas repacks,
   its stale texture is evicted from `sprite_resources` right after
   `expand_text`, so the fresh buffer uploads in the same frame.
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
cargo test               # expect 183 passed + 5 doctests
cargo test --examples    # expect 137 passed (unit tests inside the examples;
                           # ron_view/tree.rs compiles into both targets, so its 9 parser tests run twice)
cargo run --example cursor   # visual check; closing the window exits 0
cargo run --example worm     # peristaltic crawl, edge wrap; exits 0
cargo run --example ron_view # fold rows, drag both scroll handles; exits 0
cargo run --example sound    # Space/L/+/- check; closing the window exits 0
cargo run --example button   # hover-scale + click-swoosh check; window exits 0
cargo run --example widgets  # panels drag, widgets drive the face; exits 0
cargo run --example tomato_sprite   # overlay check; window exits 0
cargo run --example immortal   # 1920x1080 window: grass fills it 1:1; exits 0
```
