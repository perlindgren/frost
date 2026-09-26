# GPU-accelerating the particle system

Plan for moving `frost`'s particle system from the current host-side
implementation (pure CPU simulation + one draw call per particle) to a
GPU-resident, batched system. Web/wasm32 compatibility is explicitly out of
scope for now.

## 1. Where time is spent today

The particle system has two halves, and the cost is not where it looks:

**Simulation** (`src/particles.rs`) — `ParticleSystem` is a plain
`Vec<Particle>` stepped by semi-implicit Euler under constant gravity, then
`retain(|p| p.life > 0.0)` compacts the dead. This is trivial math: 100,000
particles integrate in well under a millisecond in Rust. It is *not* the
bottleneck.

**Rendering** — the example (`examples/particles/particles.rs`) draws every particle
with `ctx.circle`, and in this crate each circle costs:

- one `queue.create_buffer` + `write_buffer` (48-byte uniform),
- one `create_bind_group`,
- one scissor setup,
- one full-screen-triangle draw (the SDF fragment shader is bounded by the
  scissor, so the GPU side is fine — the churn is all CPU-side wgpu
  allocation),

plus the frame's O(N log N) z-sort over all `Draw`s. So N particles cost
N × (buffer alloc + bind-group alloc + draw call) of CPU time. At the
example's ~160 live particles that is invisible; at 10⁴–10⁵ particles the
per-draw allocation churn dominates the frame.

**The fix is therefore: batch the rendering into one instanced draw, and
(Phase 2) move the simulation into a compute pass.**

## 2. Goals and non-goals

**Goals**

- 10k–100k+ particles at 60 fps with *flat* CPU cost (no per-particle
  allocations, no per-particle draw calls).
- Keep the crate's style: small public API, pure-CPU `ParticleSystem`
  untouched (it is the deterministic, testable reference and stays useful
  for small effects).
- Follow existing crate conventions (uniform writers with layout tests in
  `src/shaders.rs`, resources keyed by identity in `Frost`, one render pass
  per frame).

**Non-goals (for now)**

- wasm32/WebGL: WGSL compute is unavailable there (deferring keeps the plan
  simple; see §6 for how it would be attacked later).
- Inter-particle forces / collision on the GPU.
- Texture-sprite particles (see §6).
- GPU-side spawn *patterns* — the user still owns when and how much to
  spawn on the CPU, exactly like today's example owns the emission
  accumulator.

## 3. Phasing

### Phase 1 — GPU batched rendering, CPU simulation

The 90% win for the least change. The simulation stays on the CPU; only the
drawing changes from N draw calls to one instanced draw.

**New public API** (in `Context`, mirroring `ctx.circle`):

```rust
impl Context {
    /// Draws every particle in `particles` as one instanced GPU batch at
    /// `z`, tinted `color`. Each particle's alpha is scaled by its
    /// remaining life fraction (`life / max_life`), so a batch fades out
    /// as its members die.
    pub fn particles(&mut self, particles: &[Particle], color: Color, z: f32);
}
```

One base color per batch covers the common case (fountains, sparks, smoke
with one tint). Users needing per-particle colors call the method several
times with different colors/z — each call is still one instanced draw.
(The per-particle-color variant arrives with Phase 2's GPU pool, where the
color is stored in the particle itself.)

**`Draw` variant** (`src/backend/frame.rs`):

```rust
Particles {
    /// Packed instance data: one `vec4<f32>` per particle —
    /// `(pos.x, pos.y, size, life/max_life)`.
    data: Vec<f32>,
    count: u32,
    color: Color,
    z: f32,
}
```

- `z()` returns `z`; the batch sorts in the paint order as a single unit.
- `scissor_rect()` returns the whole render area: the batch's particles can
  be anywhere, so there is no single tight box; the per-instance quads bound
  the fragment work instead (each quad is a tight `(2·size + 2·AA_BAND)²`
  box), which is the standard trade when batching.
- `Draw::Text`/`Draw::Background` handling is untouched; text expansion and
  the stable z-sort already work for any variant.

**New shader** `shaders/particles.wgsl` (as shipped — see the naga note
below for the struct-based vertex output):

```wgsl
struct ParticlesUniforms {
    size: vec2<f32>,   // surface size in pixels (width, height)
    color: vec4<f32>,  // base tint
};

// The vertex output: the NDC position, and the particle's circle
// parameters (center, size, life fraction) in pixel space.
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) inst: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: ParticlesUniforms;
@group(0) @binding(1) var<storage, read> instances: array<vec4<f32>>;

// Static index buffer on the CPU: [0,1,2, 2,1,3] (two triangles, one quad).

@vertex
fn vs_main(@builtin(vertex_index) vi: u32,
           @builtin(instance_index) ii: u32) -> VertexOutput {
    let inst = instances[ii];            // (px, py, size, life_frac)
    let corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0,  1.0), vec2<f32>(1.0,  1.0));
    let half = inst.z + 0.75;            // size + AA band
    let p = inst.xy + corners[vi] * half;
    // pixels -> NDC with the same transform the other shapes use
    let ndc = vec2<f32>(p.x / u.size.x * 2.0 - 1.0,
                        1.0 - p.y / u.size.y * 2.0);
    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.inst = inst;                     // the circle's parameters
    return out;
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>,
           @location(0) inst: vec4<f32>) -> @location(0) vec4<f32> {
    // circle SDF in pixel space, same smoothstep AA band as circle.wgsl
    let d = length(frag_coord.xy - inst.xy);
    let coverage = 1.0 - smoothstep(inst.z - 0.75, inst.z + 0.75, d);
    return vec4<f32>(u.color.rgb, coverage * u.color.a * inst.w);
}
```

Notes:

- Instance data is a **storage buffer** (`storage, read` at the vertex
  stage), not a vertex buffer — no 64 KB uniform limit, no vertex-buffer
  layout plumbing, and `VertexState { layout: None }` (no `@location`
  attributes). `draw_indexed(6, count)` does the instancing.
- The vertex shader passes the whole `inst` vec4 to the fragment stage as a
  `@location(0)` varying; the fragment does not need to re-read the instance
  buffer. The center and size are affine across the tight quad, so the
  interpolated varying reproduces them exactly at every fragment.
- **naga 30.0.1 constraint:** the rewritten WGSL frontend (naga 30.x) cannot
  parse the WGSL vertex-output *list* syntax
  `-> (@location(0) vec4<f32>, @builtin(position) vec4<f32>)` (it rejects
  the `(` with `expected identifier`). The vertex function must instead
  return a **decorated struct** — a `struct` whose members carry the
  `@location`/`@builtin` decorations (as `VertexOutput` above). This is also
  valid per the WGSL spec, so no compatibility is lost; but keep the struct
  form for any future vertex→fragment varyings in this crate (and note that
  the other frost shaders avoid varyings entirely via the full-screen-
  triangle + `frag_coord` recompute convention, which is what kept this
  naga gap dormant).
- Blending reuses the frame's existing alpha-blend color target state, same
  as the circle pipeline.
- The quad may render up to ~2.86× the circle's area (square vs circle) —
  negligible; the tight quad already replaces the full-screen triangle.

**Backend wiring** (`src/backend/app.rs`):

- `particle_pipeline: Option<RenderPipeline>` built in
  `set_up_pipelines` alongside the other pipelines (same rebuild-on-format
  change path), following the existing construction pattern.
- One persistent instance buffer (`STORAGE | COPY_DST`), grown on demand
  (recreate when the batch is >2× the current allocation, keeping the old
  until the new is ready — same "grow as needed" idea as the sprite
  resources); one small uniform buffer for the batch color (per-draw like
  the others — one allocation per *batch*, not per particle, is the whole
  point); one static 6-index index buffer created once.
- In the render loop, when the paint order reaches a `Draw::Particles`:
  `write_buffer` the instance data (one `queue.write_buffer` of
  `count × 16` bytes), bind, `draw_indexed(6, count)`.
- `Canvas::particles` packs `pos, size, life/max_life` on the CPU into the
  `Draw`'s `data` — one `Vec` fill per batch per frame (1.6 MB for 100k
  particles; trivial).

**Files touched (Phase 1)**

| File | Change |
|---|---|
| `shaders/particles.wgsl` | new |
| `src/shaders.rs` | embed + naga parse test + `ParticlesUniforms` layout test (existing pattern) |
| `src/backend/frame.rs` | `Draw::Particles` variant, `z()`, `scissor_rect()` → whole surface, `particles_uniform_data` |
| `src/lib.rs` | `Canvas::particles` |
| `src/backend/app.rs` | pipeline, buffers, render-loop arm |
| `examples/particles/particles.rs` | replace the per-particle `ctx.circle` loop with one `ctx.particles(...)` call |
| `src/particles.rs` | **unchanged** — the pure-CPU system remains the reference |

**Expected effect:** the example looks identical; frame CPU cost becomes
independent of particle count (one upload + one draw instead of N draws +
2N allocations).

**Status (Phase 1, as shipped):** implemented and passing — all 110 lib
tests green (including the naga parse test and the `ParticlesUniforms`
byte-layout tests for both the WGSL offsets and the CPU-side
`particles_uniform_data`), all examples build, and
`cargo run --example particles` renders the fading fountain in one instanced
draw. `examples/immortal/`'s water and spray streams were also migrated to the
batched path (one `ctx.particles` call per stream) — pixel-identical, since
both stream colors have alpha 1.0 and the per-particle fade is exactly the
one the batch computes from `life / max_life`; the simulation stays CPU-side
there, because the drops and mist drive gameplay hit-tests on the CPU.
Deviations from the sketch above: the uniform carries the surface
`size` (the vertex shader does the pixels→NDC transform, matching the other
shaders), `scissor_rect()` returns the whole surface, and the vertex
function returns the decorated `VertexOutput` struct (naga constraint, see
the note above).

### Phase 2 — GPU simulation (compute pass)

Now the simulation itself moves to the GPU. The CPU `ParticleSystem` stays
(reference, tests, small effects); a new GPU-resident pool type is added.

**New public API**

```rust
/// A GPU-resident particle pool with a fixed `capacity`.
pub struct GpuParticles { /* capacity + per-frame spawn staging */ }

impl GpuParticles {
    pub fn new(capacity: u32) -> Self;
    /// Queues a spawn; takes effect on the next `Context::particles` call.
    /// Dropped if the pool is full.
    pub fn spawn(&mut self, Particle, /* color */ Color);
    pub fn clear(&mut self);
    pub fn capacity(&self) -> u32;
}

impl Context {
    /// Steps the pool on the GPU — applies queued spawns, integrates under
    /// `gravity` for `dt` seconds (same semi-implicit Euler as the CPU
    /// system), culls the dead — and draws the pool at `z`.
    pub fn particles(&mut self, pool: &mut GpuParticles, dt: f32,
                     gravity: [f32; 2], z: f32);
}
```

**GPU data layout** — one storage buffer of 48-byte particles:

```wgsl
struct GpuParticle {
    pos: vec2<f32>,     // @0
    vel: vec2<f32>,     // @8
    life: f32,          // @16
    max_life: f32,      // @20
    size: f32,          // @24
    pad: f32,           // @28
    color: vec4<f32>,   // @32 (16-byte aligned) → 48 bytes total
};
```

**Per-pool GPU resources** (owned by `Frost`, keyed by the pool's identity
in a `HashMap<*const (), Pool>` — the exact precedent of
`sprite_resources` keyed by the `Arc` pointer):

- `particles`: `STORAGE | COPY_DST`, `capacity × 48` bytes — the pool,
  single buffer (swap-remove, below).
- `count`: `STORAGE | COPY_DST`, 4 bytes — a `u32` live count.
- `spawns`: `STORAGE | COPY_DST`, grown to the frame's spawn bytes.
- `spawn_count`: small uniform buffer (4 bytes).
- Pipelines: `spawn_pipeline` (compute), `step_pipeline` (compute),
  `draw_pipeline` (render, same shape as Phase 1's but reading
  `GpuParticle` and computing the fade from `life/max_life`).
- Bind groups cached, rebuilt only when buffer sizes change.

**Per-frame flow** — recorded into the frame's existing command encoder,
*before* the render pass (wgpu executes encoder commands in order, so the
compute passes complete before the draw reads the pool):

1. `Context::particles` moves the pool's queued spawns into a
   `Draw::GpuParticles { key, dt, gravity, z, spawn_bytes }`; the render
   loop does `queue.write_buffer(spawns, ...)` + `spawn_count`.
2. **Dispatch A — spawn** (`workgroup_size` 64, `invocation_id < spawn_count`):
   ```
   let slot = atomicAdd(count, 1u);
   if slot < capacity { particles[slot] = GpuParticle(spawns[j]); }
   // pool full: spawn is dropped (documented)
   ```
   Spawn runs *before* step so a spawned particle is integrated in the same
   frame — matching the CPU example's order (spawn, then `update`).
3. **Dispatch B — step + cull** (`invocation_id < count`, `count` read once
   per thread):
   ```
   p = particles[i];
   p.vel += gravity * dt;
   p.pos += p.vel * dt;      // same semi-implicit Euler as the CPU update
   p.life -= dt;
   if p.life > 0.0 { particles[i] = p; }
   else {
       let last = atomicSub(count, 1u);
       if last != i { particles[i] = particles[last]; }  // swap-remove
   }
   ```
   The swap-remove is correct under concurrency: the killer's copy source
   (`last = N−k`) is always at or beyond the final valid range
   `[0, N−K)` (its own decrement is the k-th and `K ≥ k`), so the
   surviving copy in slot `i` is the only instance of that particle — no
   loss, no duplication. The one caveat: `particles[last]` is read while
   thread `last` may still be writing it, so one particle per death can be
   stepped one frame early or late. Visually irrelevant for particles;
   documented, and it is why the CPU `ParticleSystem` remains the exact
   reference. (A ping-pong double-buffer compaction would remove even that
   nondeterminism at 2× memory + an extra pass — not worth it here.)
   Note the buffer is bound **once, read-write** — legal; wgpu only
   forbids the same buffer bound both read-only *and* read-write in the
   same pass.
4. **Draw** — the Phase 1 instanced draw, but instance count comes from
   the GPU. The CPU does not know the post-cull count in-frame, so the
   draw always uses `capacity` instances and the vertex shader culls
   `instance_index >= count` (reading `count` from the GPU) by emitting
   degenerate/clipped geometry. That is `capacity × 4` trivial vertex
   invocations (e.g. 1M for a 256k pool) — sub-millisecond on any real
   GPU and it avoids all readback plumbing. (Follow-up optimization:
   read the 4-byte count back after submit and draw
   `prev_count + this_frame_spawns` instead — tighter bound, one
   `map_async`.)

**Determinism** — per-particle integration is identical to the CPU
formulas (same operation order → same rounding for the same inputs), except
the swap-remove race above. The CPU tests stay as the ground truth; a
CPU/GPU parity test (§5) pins the GPU behavior.

**Files touched (Phase 2, additive)**

| File | Change |
|---|---|
| `shaders/particle_step.wgsl` | new compute shader (spawn + step, two entry points or two modules) |
| `shaders/particles.wgsl` | draw variant reads `GpuParticle`, fade from `life/max_life`, vertex-side count cull |
| `src/particles.rs` (or new `src/gpu_particles.rs`) | `GpuParticles` pool type (pure CPU staging, no device needed to construct) |
| `src/backend/frame.rs` | `Draw::GpuParticles` variant |
| `src/lib.rs` | `Context::particles(&mut GpuParticles, dt, gravity, z)` |
| `src/backend/app.rs` | pool map, compute pipelines, encoder wiring (compute before render pass) |
| `examples/particles_gpu.rs` | new: pool-based fountain at high rates |

### Phase 3 — follow-ups (not planned in detail)

- Texture-sprite particles: per-instance uv + a sampled texture (mirrors
  the `Sprite` draw).
- Per-particle spin: `angle + spin` in the struct; the quad corners already
  come from the vertex shader, so this is a 2D rotation there.
- CPU readback of the pool (collisions, camera shake, audio) via a
  mapped `map_async` copy of the buffer.
- Many pools: merge into one instanced draw with per-pool offsets (only
  needed if apps use several pools).
- wasm32: compute is unavailable in WebGL2 — either a render-pass-based
  simulation (fragment-shader ping-pong) or a JS-side fallback; revisit
  when web support is back in scope.

## 4. wgpu-30 specifics to get right

- `ComputePipelineDescriptor` takes `workgroup_size: u32` (uniform workgroup
  size, no per-dimension) and `entry_point: Option<&str>`; `layout: None`
  + auto bind group layouts, same as the existing render pipelines.
- A `STORAGE` buffer bound at the **vertex** stage is supported — this is
  what lets the instanced draw read instance data without a vertex buffer.
- `queue.write_buffer` for both the per-frame instance upload and the spawn
  upload; persistent buffers with `COPY_DST`, grown as needed (no per-frame
  buffer creation — the thing we are eliminating).
- Pass ordering: one `CommandEncoder` per frame already exists; compute
  passes recorded before the render pass in it execute first.
- The existing `multiview_mask` / `occlusion_query_set` fields used by the
  current pipeline constructors are carried over unchanged.

## 5. Verification

- **Shader tests** (existing pattern in `src/shaders.rs`): naga
  `parse_str` for the new WGSL + a struct-layout test pinning the member
  offsets against the CPU-side writers (this is how the crate already
  catches WGSL↔CPU layout drift).
- **CPU tests**: `src/particles.rs` tests unchanged and still passing.
- **Headless parity test** (Phase 2, the valuable one): create a wgpu
  device with no surface (compute-only test — `Instance` +
  `request_adapter` + `request_device`, no window needed), run the CPU
  `ParticleSystem` and the GPU pool with identical spawns/gravity/dt
  sequences, read the pool back, and compare with a small tolerance —
  allowing the one-step swap-remove discrepancy per death. Runs in
  `cargo test` on native.
- **Example + stress**: update `examples/particles/particles.rs` to the batched path;
  make the emission rate settable (e.g. env var `PARTICLE_RATE`) and run at
  10k / 50k / 200k; log per-frame CPU time (an `Instant` around the
  process+render in the example, at `trace`) before and after to show the
  CPU cost flattening.
- **Visual**: the fountain should look identical at 160 particles; at high
  rates the fade/AA must match the per-circle look (same 0.75 px band, same
  smoothstep).

## 6. Risks and accepted trade-offs

- **Blending order inside a batch**: alpha blending is order-dependent;
  the per-particle pass blended in spawn order, the batch blends in pool
  order (arbitrary after compaction). For small, mostly non-overlapping
  dots the difference is below visibility — accepted and documented.
- **One z for the whole batch**: a batch is one unit in the paint order.
  A user who needs particles interleaved with other z-layers issues several
  batches — each is still one instanced draw.
- **Pool-full drops spawns** (Phase 2): documented; a "pool is full"
  counter for the future.
- **Memory**: a 256k pool is ~12 MB (48 B × capacity) — trivial.
- **`Draw` carries a `Vec<f32>`**: it is `Clone`, so the frame's clone
  paths copy it; fine at batch scale (or swap to `Arc<[f32]>` if profiling
  ever says otherwise).
