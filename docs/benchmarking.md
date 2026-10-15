# Benchmarking frame rates on real hardware

> **Status.** Written after the first campaign (2026-10-12), which measured
> `antifrost` at 4K on a Ryzen 9 7950X3D — its iGPU (Radeon 610M, RDNA2,
> 2 CUs), the discrete RX 7900 XTX, and llvmpipe — and which cost a whole
> afternoon to the silence of `release_max_level_off`. The traps in §1 are
> the reason this document exists; the recipe in §3 is what we wish we had
> started with.

Measuring a frost game's frame rate from a shell — rather than eyeballing
the [`Diagnostics`](../src/diagnostics.rs) overlay — means making the
engine log every presented frame and counting the lines. Three things get
in the way, and all three fail *silently*.

## 1. Three silent traps

**a. Release builds cannot speak.** The `log` dependency carries
[`release_max_level_off`](../Cargo.toml) (line 22), so every `log::*!`
call — frost's, wgpu's, the demos' — is compiled *out* of
`--release` binaries. `RUST_LOG` is then irrelevant: the calls do not
exist, not merely filtered. A release binary that "hangs" at startup and
logs nothing is almost always just mute. The escape hatch needs no
`Cargo.toml` edit — flip `debug_assertions` from the command line and the
feature stops applying, at full optimization:

```sh
cargo build --release --config 'profile.release.debug-assertions=true' \
    --example antifrost --features diag
```

(A plain debug build also speaks, but measures the debug build's CPU, not
the shipped one; the `--config` flip keeps `opt-level = 3` and costs only
assert checks plus the debug-only shader-dir poll, negligible on a
GPU-bound run. Do remember the numbers carry debug-assertions.)

**b. `env_logger` buffers when stderr is not a tty.** Redirected to a
file, its records sit in an ~8 KB buffer and a `kill`/`timeout` loses the
tail — or *everything*, if the run never produced 8 KB. At `RUST_LOG=trace`
the per-frame lines self-flush roughly twice a second, which is exactly
the level the recipe needs, so the trap mostly dodges itself; just don't
trust the last partial second of a histogram, and expect a clean exit
(`--help`, an error, `Context::exit`) to flush everything.

**c. A sandboxed or headless shell has no GPUs.** If `/dev/dri` is not
visible (DSH command sandboxes, most containers, SSH with a stripped
environment), the Vulkan loader hides RADV and wgpu silently falls back
to **llvmpipe** — Mesa's CPU rasterizer, which runs the game at
single-digit-to-tens of fps at 4K. Nothing errors; only the startup log
tells: `frost` prints
[`using adapter: <name>`](../src/lib.rs) before anything renders, and
`"llvmpipe (LLVM ...)"` means you are about to benchmark the CPU. In a
DSH-style sandbox, re-run the same command with escalated access and
check that line again.

## 2. Which GPU answers

frost requests its adapter with [`RequestAdapterOptions::default()`](../src/lib.rs)
— no power preference, no knob — so Mesa decides. On the campaign machine
(Raphael iGPU *driving the monitor*, RX 7900 XTX idle beside it):

- the default, and `VULKAN_DEVICE=0` and `=1`, all resolved to
  `RADV RAPHAEL_MENDOCINO`: the display-attached GPU wins;
- `DRI_PRIME=1` selects the render-only GPU and produced
  `RADV NAVI31` (the XTX).

A game launched normally on that desktop therefore runs on the *iGPU*.
If a `--gpu`/power-preference knob is ever wanted, that single
`RequestAdapterOptions` is the choke point.

## 3. The recipe

1. **Build with a voice and the overlay:**

   ```sh
   cargo build --release --config 'profile.release.debug-assertions=true' \
       --example antifrost --features diag
   ```

2. **Run uncapped from your session** (`--vsync off` is the demo's flag;
   the overlay now reads `VSync: off` next to the resolution, and Alt-V
   flips it live):

   ```sh
   XDG_RUNTIME_DIR=/run/user/$UID WAYLAND_DISPLAY=wayland-0 \
   RUST_LOG=trace timeout 14 \
       ./target/release/examples/antifrost --vsync off > bench.log 2>&1
   ```

   The window opens fullscreen on the connected display — by the user's
   request, it *is* the measurement. If a session refuses to map a window
   (an X11 fallback usually works: `-u WAYLAND_DISPLAY DISPLAY=:1
   XAUTHORITY=/run/user/$UID/xauth_*` — find the live socket with
   `ss -xl | grep X11`, the cookie from a running client's
   `/proc/PID/environ`), the log says so instead of stalling: check for
   `frame presented` lines before blaming the compositor.

3. **Verify the run, then histogram it.** First lines to check:
   `using adapter:` (right GPU, not llvmpipe) and `surface configured at
   WxH … present_mode` (right size, `AutoNoVsync`). Then:

   ```sh
   awk '/frame presented/ {split($1,a,"T"); split(a[2],b,"Z"); split(b[1],c,":");
        s=c[1]*3600+c[2]*60+c[3]; n[s]++}
        END {for (k in n) printf "%02d:%02d:%02d  %d fps\n",
             int(k/3600), int(k%3600/60), k%60, n[k]}' bench.log | sort
   ```

   Steady middle seconds are the number; the first second carries warm-up
   and the last truncates at the buffer boundary.

## 4. The campaign's numbers (2026-10-12)

antifrost, native 3840×2160 fullscreen, `--vsync off`, same debug-flipped
binary:

| Runner | Steady fps | Note |
| --- | --- | --- |
| Radeon 610M iGPU (RADV RAPHAEL) | **27–28** | the default adapter — this is what 4K on the iGPU is |
| RX 7900 XTX (RADV NAVI31) | ~120 | ~4× the iGPU; the cap is host-side (CPU/submit or compositor pacing — not chased) |
| llvmpipe (16-core CPU) | ~18 | the accidental baseline from trap (c) |
| iGPU, fixed 1080p buffer (`Config::render_size`) | **57–59** | same 4K fullscreen, 2.1× — and a clean attribution, below |
| iGPU, 1080p + discard-before-lighting shaders | **73–75** | [`sprite.wgsl`](../shaders/sprite.wgsl)/[`particles.wgsl`](../shaders/particles.wgsl) skip the light loop for exactly-zero-alpha fragments; +26 % |
| iGPU, 1080p + per-light occluder sublists | **73–75** | the occluder field split into per-light blocks (hull/SAT cull); **no change here, deliberately reported** — antifrost's density (tens of lights, tens of merged boxes, single-ray walks ending at the first blocker) sits below where the old global walk hurt; the feature bounds how that cost grows with `lights × occluders`, it does not shave this scene |

Where the cull *does* show: the density bench
(`cargo test --lib occluder_walk_density_bench -- --ignored --nocapture`,
[shadows.rs](../src/backend/tests/shadows.rs)) paints a 64² lit sheet under
24 lamps with a 240-box field most of whose walls sit off-screen, once with
per-light blocks (6 records each) and once with every block holding every
record the region could still see (108 each — the old global walk). The
iGPU frame: **2.48 ms → 0.24 ms, 10.4×**; llvmpipe: 54.9 → 2.2 ms, 24.8×.
Read it as the growth curve, not as a demo win: at antifrost's density both
numbers are noise against the 13.5 ms frame, and the feature's job is that
the first number stays that way as the world fills up.

Follow-up the same day: antifrost switched to
[`Config::render_size`](../src/lib.rs) at 1920×1080 with the stretch pass
carrying the frame to the 4K surface. The payoff (28 → 58 fps) was only
2×, not the 4× that a quarter of the pixels suggests — and the gap is
itself the measurement: solving the two-point system (per-pixel work
scales ¼, the frame time went 35.7 → 17.2 ms) puts **per-pixel work
(fill + the per-fragment light loop of `lighting.wgsl`) at ≈ 25 ms of the
native-4K frame, and a flat ≈ 11 ms that no buffer size touches** —
KWin compositing and presenting a 3840×2160 surface on the same 2 CUs,
the CPU-side submission, the present cadence. The per-pixel light loop
is confirmed as the dominant term (antifrost already runs it in its
cheap branch — `LAMP_PENUMBRA = 0.0`, one hard ray per light — so this
is what the single-ray walks still cost at 8.3 Mpx); the composite of
the rest explains why 1080p lands at 58 rather than 100+. With vsync on
a 4K60 panel, the fixed buffer holds the 60 pin.

Calibration for future estimates: the iGPU delivered ~0.5–0.55 GPix/s of
*effective* textured-blend fill on this workload (soil sheet + tile map +
lit sprites ≈ 1.8 screens of overdraw), about a third of its nominal ROP
rate and half the floor of the pre-measurement guess (45–100 fps).
Blended layers, not triangle setup, set the pace — and on a 4K@60 panel
vsync cannot rescue a 28 fps hill: expect ~25–28 fps on an uneven
2–3-vblank cadence with vsync on.

## 5. Proving an optimization pays

A shipped demo that runs at the same frame rate before and after an
optimization proves only that nothing got worse. So the house rule, set
the day the per-light occluder sublists landed: **every optimization
arrives with a bench that confirms its gain, not just a measurement that
denies its regression.** The pattern that worked, in full:

- **A/B the mechanism, not the demo.** Build the two fields (or buffers,
  or vertex counts) the old and new scheme would produce for one scene,
  and paint them through the same pipeline on the same device — the only
  difference between the two timings is what the change altered. The
  occluder bench ([`shadows.rs`](../src/backend/tests/shadows.rs),
  `occluder_walk_density_bench`, `#[ignore]`d and run by hand) packs one
  occluder field with the real viewport and one with a million-pixel
  viewport, so every block holds every record: the old global walk, byte
  for byte, same shader, same fragments.
- **Synthesize the density that stresses it.** If the shipping scene sits
  below the threshold where the old cost hurt, the bench raises the
  scene, not the claim: 24 lights, a 240-box field, most of it
  off-screen. The demo's "no change" and the bench's "10.4× on the
  iGPU" are then both honest numbers — one is the present, the other the
  growth curve — and the doc table says so in those words.
- **Time it where it counts.** Warm up untimed, time each frame with a
  `PollType::wait_indefinitely()` drain between submit and stamp, run
  each scheme twice and report the second pass. Software rasters confirm
  the mechanism cheaply (24.8× under llvmpipe) but overweight shader
  ALU; the hardware number is the one that ships in the sentence (10.4×
  on the iGPU).
- **Keep the simulation honest about itself.** The bench's "global"
  field held 108 of the 240 boxes per block, not all of them — the
  million-pixel viewport is a quadrant, and the old field would have
  walked more. Say so next to the ratio, and the ratio is read as the
  floor it is.

## 6. What a draw call costs the CPU (2026-10-15, Apple M4 Max)

The ask was fewer draw calls, "to reduce CPU load — the GPU burden should
stay essentially the same". The GPU part is right and worth saying first: a
frame's pixels do not care which buffer its uniforms arrived from. What the
change bought is the other half — **the count did not fall; the cost of each
call did.** Nothing here is batched or instanced; a 243-call frame still
issues 243 calls.

Measured on the M4 Max through Metal, in a release build
(`cargo test --release --lib draw_cost -- --ignored --nocapture`), with the
bench of §5's shape: the two schemes paint the same scene through the same
shaders on the same device, warmed up untimed, each frame drained with
`PollType::wait_indefinitely()`, three passes, the last one reported.
This is a layout test — a uniform buffer and bind group per draw, against one
arena of runs with dynamic offsets, plus a third scheme that pools the
per-draw buffers and reuses them.

**The conditioning, because it is not a footnote here:** built-in display only,
no external panel, on battery — at 14% charge for the in-demo numbers. That
matters far more to a number measured in frames than to one measured in
milliseconds. `benchsuite --only fill-1080p,lamps-10` run in that state reads
**90.0 and 88.9 fps**, against §4's 2136.9 and 173.8 for the same scenarios:
two workloads a universe apart in GPU work, both held to the same ~90, so what
that reading measures is the display's pacing and not the machine. It is also
why the uncapped demo read *slower* than the capped one — a swap chain asking
for `AutoNoVsync` on this panel follows the drawable cadence the panel settled
on, and ProMotion settles low when nothing pins it, 90 being one of its locked
rates. The encode spans below are timed inside the process with `Instant`, both
sides interleaved in the same power state, so their ratio survives the battery;
the absolute milliseconds are conditioned, and a plugged-in re-read is owed.

Glyph quads over 1600×1600, ms/frame:

```text
  draws    per draw    arena    pooled    arena ratio
     243      2.065    0.294     0.922        7.01x
    1000      7.941    0.535     3.912       14.83x
    2000     15.707    0.840     7.835       18.70x
```

Marginal cost per draw, which is the number that generalises — the averages
above carry the frame's fixed fill: **7.7 µs** a draw per-draw, **7.8 µs** for
the pooled scheme's un-avoidable `write_buffer` half of it measured at 3.9 µs,
and **0.31 µs** with the arena. Pooling loses because the copy survives; it is
over half the cost, which is the finding that decided the design.

The same bench over a frame that cycles line, polyline, circle, rectangle,
shape and glyph, into 960×720:

```text
  draws    per draw    arena   bytes packed    ratio
     243      5.765    2.947        145,968    1.96x
    1000     18.094   12.014        597,792    1.51x
    2000     36.616   24.923      1,195,808    1.47x
```

Far narrower, and the polyline is talking: one part in six of this frame
carries its whole point list — 2080 bytes — and those bytes cross the bus
either way. Marginal per draw here: 18.5 µs against 12.9, so the arena still
removes 5.6 µs of the 18.5. Read the glyph table as a text-heavy frame and
this one as its opposite; a real frame is between them, weighted by how much
of it is two kilobytes of chart.

In situ, unbenchmarked, the same story in object counts. The engine's own
trace over 548 frames of the diagnostics overlay: **16.4 bind groups and 16.0
buffers created per frame** before, **1.5 and 0.02** after.

And the number the change was for, measured in the demo that asked it. The
`vrr` demo's frame — 243 draw calls, the diagnostics overlay included — with
two temporary lines in `render()`: an `Instant` before
`create_command_encoder`, and an average printed every 60 frames just after
`queue.submit`. Two release binaries, one from each side of the change, run
interleaved three times for eight seconds each, medians of the last three
60-frame windows of each run:

```text
  per draw    2.870   2.998   2.831 ms/frame
  arena       0.331   0.294   0.327 ms/frame
```

**2.87 ms to 0.33 ms, 8.8×**, in the span that covers the encode and the
submit and nothing else. Three things to keep in mind while reading it. It is
2.5 ms of CPU per frame in a demo that also spends about 4.6 ms in the app's
own `process` — the overlay's `PROC` stat — so this is roughly a third of the
frame's CPU time, not most of it. It is larger than the 243 × 7.7 µs the bench
predicts (1.8 ms), because a real frame spreads its calls over six pipelines
with different bind groups, where the bench exercised one; treat the bench as
the mechanism and this as the frame. And the instrumentation is not in the
tree — it is two lines and a counter, applied by hand whenever the number is
wanted again.

**The 7900 XTX row is unrun.** §4's campaign numbers are that machine's, and
these are not a reproduction of them: no AMD figure is claimed here, and the
ratio may not travel — a driver's per-object cost is exactly the kind of thing
that differs between backends.

**What is not measured here, because it was not done:** one instanced draw per
run of adjacent same-texture draws, which is the change that would lower the
*count*. It was sketched and parked. Two reasons: glyphs are drawn as quads
whose edges are anti-aliased by the sprite shader's own path, and a batched
instance path redraws those edges from a different model, so the text changes
appearance — a decision for the owner of the look, not for a CPU budget; and
WebGPU leaves the order of primitives within one draw undefined, so a batch
would rely on an ordering the spec does not promise. The painter-order
blending of adjacent runs is the other reason batching has to be careful: only
*consecutive* same-texture draws may ever merge.
