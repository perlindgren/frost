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

4. **Ask the machine whether it is fit to be measured.** Every number in this
   file was taken on a machine that was also a working computer, and its own
   moods move results by more than most of the changes being measured — the
   two identical builds that read 82 and 36 fps are the cautionary tale above.
   [`tools/noisefloor`](../tools/noisefloor/) is the witness:

   ```sh
   cargo run --release -p noisefloor -- --seconds 10 && <the bench>
   ```

   It reports two readings from one thread — how late a 1 ms wake lands above
   that machine's own median, and how long a fixed quantum of arithmetic runs
   relative to its own median — and exits 1 when either is too loud, so
   `noisefloor && <bench>` reads as it looks. `benchsuite` carries the same
   witness through a whole run and, when the floor is too loud, prints its
   numbers and exits **3**: no verdict, and no `--save-baseline` either,
   because a poisoned baseline is worse than a missing one. The two
   `draw_cost` benches print the line and carry on, since a bench has no
   baseline to fail and someone is reading its output anyway.

   What it is good at is measured, not hoped at, and its range is narrower
   than one would like. On the 14-core M4 Max, nothing running and four gzip
   streams both read 0.15 ms of excess lateness and a 2.0× quantum spread;
   twenty and forty spinners oversubscribing the cores read 0.36 and 0.32 ms,
   2.9× and 2.6×; sixty — four times oversubscribed — read 3.23 ms and 3.7×
   and tripped both gates. **Below that it is blind, and largely correctly
   so**: with cores to spare, one thread genuinely is not being rationed, so a
   laptop with a browser and a chat client open is not a run the canary can
   disqualify. The proof of the limitation is in §6, where a span measured
   while its owner was using the machine cost 3.66 ms against 2.87 ms
   unattended, under a floor the instrument had no objection to. Read the
   printed `worst` with your own eyes: a stall rare enough to fall outside a
   p99 is not rare enough to fall outside a frame budget.

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
- **Say what the weather was.** Both benches below print a noise floor line
  beside their numbers (§3 step 4). Read it before believing the
  milliseconds, because the two do not travel alike: the ratio between
  schemes is steady — 8.45×, 13.85×, 17.89× on the charger one evening and
  8.19×, 14.05×, 17.41× on the same machine two hours later, and 25.2×
  against 23.8× at the margin across a battery swap — while the absolute
  microseconds under them move ten percent or more with whatever the machine
  is doing. Quote the ratio, condition the milliseconds, and rerun when the
  line says too loud.
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
no external panel. The first pass ran on battery at 14%; the re-read came the
same evening on the charger, and power state turned out to be close to
irrelevant for the millisecond tables. It mattered for one claim, which is why
it is spelled out. `benchsuite --only fill-1080p,lamps-10` reads **90.0 and
88.9 fps** on battery and **89.7 and 91.9** on AC, against §4's 2136.9 and
173.8: two workloads a universe apart in GPU work, pinned to the same ~90 in
both power states. So that ceiling is the present path, not the battery and not
the machine — and not the present mode either, since Fifo and `AutoNoVsync`
agree within their jitter there, 85.4 and 92.1 fps inside the same minute. The
same window has also been timed at exactly 120.0, which is ProMotion moving
between its locked rates underneath the app rather than anything the engine
does. Read it as: on this Mac the fps column of any bench is the display's, and
the drained millisecond tables below are the machine's.

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

Re-run on the charger, and this time with `--test-threads=1` — the two
`draw_cost` benches share a process and contend for the GPU when cargo runs
them side by side, which inflates every row and is worth knowing before
trusting a number out of a bare `draw_cost` run:

```text
  draws    per draw    arena    pooled    arena ratio
     243      2.252    0.267     0.910        8.45x
    1000      7.602    0.549     3.892       13.85x
    2000     16.556    0.925     7.732       17.89x
```

Marginals **9.0 µs**, **3.8 µs**, **0.38 µs** against the battery run's 7.7,
3.9, 0.31. The absolute microseconds moved with the machine; the ratio between
the schemes did not — per-draw over arena is **25.2×** on battery and **23.8×**
on AC. The mixed bench agrees, 1.46×/1.50×/1.55× against 1.96×/1.51×/1.47×,
marginal 18.1 µs against 11.2, the polyline keeping its share either way. That
is the pair worth quoting: the marginal, and the ratio between schemes. Not a
frame time.

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

The re-read on AC, with the machine in use, came out **3.66 ms and 0.91 ms**,
4.0×. Slower on both sides with more power available, because the host was busy
with a person on it — so the demo's multiplier is a property of the moment, and
these two drained benches are the durable measurement. What did not move is the
shape: about a third of a microsecond per draw with the arena, wherever the
machine happens to be.

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

## 7. What the float frame buffer costs (2026-10-15, Apple M4 Max)

Stage one of the high-dynamic-range work: the scene renders into
`Rgba16Float`, and the blit pass carries it to the window, where the frame gets
clamped and quantised to 8 bits. The claim being tested is the uncomfortable
one — that this changes nothing the eye can see — so it was tested twice, once
per pixel and once per frame.

**Per pixel** ([`frame_buffer`](../src/backend/tests/frame_buffer.rs), the same
draws through the same shader into both attachments): an opaque base, a
translucent pane, a pane over a pane, and two lights stacked on one pixel all
come out **bit-identical, 0 of 4096 pixels**. Twenty successive 5% washes — the
one case where a store that rounds can accumulate — move **14% of pixels by
exactly 1/255**, none by more. A base at 2.4, out of an 8-bit buffer's reach
entirely, reaches the window as the same `[255, 255, 212]` from both paths.

Two predictions died in that measurement, and they are recorded so nobody
re-derives them. First, that the old buffer composited in encoded space: it did
not — hardware decodes an sRGB attachment before blending, so the 8-bit buffer
was only ever rounding what it *stored*, never warping what it composited.
Second, that values above 1.0 would start behaving differently: with a clamp at
the end, a value clamped at the attachment and a value clamped at the window
are the same pixel. The float buffer's range is kept, not shown, until a pass
reads it — which is why it precedes bloom rather than following it.

**Per frame**, the drained encode span of §1's recipe — encode, submit,
`PollType::wait_indefinitely()` — on two builds, interleaved:

| scene | 8-bit surface | float buffer |
|---|---|---|
| antifrost, 3840×2160 fullscreen, `--vsync off` | 1.044 / 1.042 ms | 1.045 / 1.043 ms |
| vrr, 1600×1200 windowed, `--vsync off` | 1.417 / 1.834 ms | 1.462 / 1.791 ms |

Four to five thousand frames per cell. At 4K the difference is 0.1%, well
inside the pair-to-pair spread of the same binary; on the light scene the drift
between pairs (0.4 ms) is eight times the effect, which is the honest way to
report a difference that isn't there. The float path writes twice the bytes of
the target and adds one pass over the window, so on a GPU short on fill — the
610M at 4K in §4's table, already down at 27 fps — this is the row that might
move, and nobody has run it there yet.

## 8. What an unread target costs (2026-10-15, Apple M4 Max)

The emissive attachment — the scene's second color output, §7's float buffer
holding what each surface puts out — landed before anything reads it. That
order is deliberate: the cost of a look nobody has argued about yet is worth
knowing while it is still cheap to walk back, and one full-resolution
half-float clear and write per frame is exactly the kind of thing that stops
looking free the moment a phone is involved.

Same recipe as §7 — the drained encode span, two configurations of one binary
interleaved (a temporary env knob, since a second build would have moved more
than the attachment), 9 s per cell at `--vsync off`:

| scene | one target | two targets | difference |
|---|---|---|---|
| antifrost, 3840×2160 fullscreen | 1.044 / 1.044 ms | 1.056 / 1.050 ms | +0.6% to +1.1% |
| vrr, 3200×2400 physical window | 1.973 / 1.884 ms | 2.039 / 1.984 ms | inside the drift |

Four to five thousand frames per cell. At 4K both emissive runs sit above both
single-target runs, so the direction is real, and the size is about a hundredth
of a millisecond — a third of the §7 float buffer's own noise floor, and much
less than one frame in sixty at any frame rate this machine is asked to hold.
On the windowed scene the pair-to-pair drift (0.09 ms) is the same size as the
apparent effect, which is the honest way to report that this instrument cannot
see it.

What the money buys per frame: 33 MB cleared to black, then read-modify-written
by whatever the scene draws — 8 bytes per pixel blended, where the color target
already pays that for itself. A GPU short on fill pays it twice, and this is
the second row after §7's that would move on the 610M; both stay unrun there.

When bloom arrives and reads this target, the question changes shape: the cost
then is the downsample chain, which runs at a quarter or an eighth of the
window and should cost a fraction of what the attachment does. If it doesn't,
this table is the baseline to compare against.

## 9. What bloom costs (2026-10-16, Apple M4 Max)

Same instrument as §8: antifrost with `--vsync off`, the span from "the frame's
commands are assembled" to "the GPU says it is done with them", medians of
4300–5100 frames, bloom-off and bloom-on runs interleaved in pairs. One
difference from §7 and §8, and it matters: those rows were *attachments*, which
cost per pixel, and they were nearly free. This one is *passes*, and it is not.

The chain was nine render passes — three levels, each with a downsample and a
blur across and down. (§12 added a fourth level, so it is twelve now.) Level 0 is a quarter of the window, level 1 a sixteenth,
level 2 a sixty-fourth, and each is written three times, so the whole chain
touches about as many pixels as one frame does. That is the theory; this is what
it costs at the buffer antifrost actually ships with — 1920×1080, because the
default quality setting fixes `render_size` there. (A `Config::window_size` of
1920×1080 logical on this machine's built-in display puts the *surface* at
4112×2572; a row further down measures that size. An earlier version of this
section said 3840×2160, which was an assumption about the display scale factor
and never a measurement.)

| chain | bloom off | bloom on | delta |
|---|---|---|---|
| 3 levels, 5-tap blur run twice (15 passes) | 1.044 / 1.053 | 1.348 / 1.348 | **+0.30 ms** |
| 3 levels, 5-tap blur once (9 passes) | 1.069 | 1.263 | **+0.19 ms** |
| 3 levels, 13-tap blur once (9 passes) | 1.057 | 1.263 | **+0.21 ms** |

The bloom-off medians drift 0.025 ms across the four runs, which is the noise
floor here; the deltas are five to twelve times that, so they are real.

**A render pass costs something of its own, and the pixels cost something on
top.** Two pieces of evidence:

- Six extra passes over the same textures at the same size cost +0.09 to
  +0.11 ms, about 16 µs per pass. This one holds: same session, same buffer,
  same everything but the pass count.
- The bigger buffer costs more. Nine passes over a 4112×2572 buffer measured
  **+0.500 and +0.571 ms** in two independent runs (§11's paired method),
  against +0.19 to +0.21 ms at 1920×1080 above. Five times the pixels, about
  two and a half times the bill.

A correction, because the first version of this section got the second point
backwards. It claimed the same chain against a quarter-size buffer cost the same
— a fourth of the texels, the same bill — and the two runs really did agree to
within 0.005 ms. They agreed because they were *the same size*: the scratch knob
that was supposed to shrink the buffer set `render_size` to 1920×1080, which is
what the default quality setting already sets it to. The check that would have
caught it, printing the extent of the texture being built, was written this
session and printed 1920×1080 for a run that had been recorded as 3840×2160. So
the flat-cost claim was never tested, and the honest reading of the two bullets
above is a fixed part and a growing part: roughly 0.15 ms of nine passes' cost is
there whatever they draw into, and the rest scales with the frame. Every figure
in this section belongs to the three-level chain; §12 has the fourth.

The shape of the conclusion did not change, only its strength: a pass carries
~16 µs before it draws a thing, so
the shape of a cheap chain is *few passes with wide kernels inside them*, not
many cheap ones. It is why `bloom.wgsl` folds thirteen Gaussian taps into six
bilinear fetches and blurs once: measured, the wide-kernel nine-pass chain is
+0.21 ms where the narrow-kernel fifteen-pass one is +0.30 ms, and the wider
kernel is the *better-looking* of the two. A second blur iteration widens the
glow by √2 for +0.10 ms; a 13-tap kernel widens it by more for three extra
fetches.

Two honest caveats. antifrost has nothing emissive in it, so every one of those
passes read black and wrote black: the +0.21 ms above is what bloom costs when
it does nothing, which is the worst case and the one a game will hit on the
frames where nothing is bright. Skipping the chain when nothing emitted would
need either a readback or a CPU-side hint about what the scene contains, and
neither is built. Second, this is fill-bound work measured on the machine with
the most fill here; §7 and §8 both note that the 7900 XTX row is unrun, and a
lower-end GPU pays for attachments *and* passes, so the same chain on the 610M
is expected to look worse than this table does, not the same.

Against the frame it is asked to fit into: 0.21 ms is 1.9% of a 90 Hz budget
(11.1 ms) and 2.9% of 60 Hz's. It is not free, and unlike §8's attachment it is
not a rounding error — which is the right reason for `Config::bloom` to be off
until an app asks for it.

## 10. What a tone map costs (2026-10-16, Apple M4 Max)

Same instrument again, same antifrost at its default 1920×1080 buffer, medians
of ~4700
frames per run, runs interleaved. The tone map is a handful of arithmetic ops
in the stretch pass — one `max`, one branch, a divide — so this is the row that
says what *computation* costs here, after §9 said what passes cost.

| config | median |
|---|---|
| nothing extra | 1.070 / 1.061 |
| headroom 0.15 | 1.073 / 1.083 / 1.067 |
| headroom 0.15 + bloom | 1.279 (bloom alone: 1.263) |

The apparent difference is **+0.007 ms**, and the run-to-run spread within each
group is ±0.011 ms, which is wider than the difference. The honest report is
that this instrument cannot see a tone map at all, and the three runs with the
headroom on straddle the two without it.

That is not luck, it is the shape of the work: nine extra render passes cost
+0.21 ms (§9) because every one of them costs about 16 µs whatever it draws,
while forty million extra arithmetic ops across one pass cost nothing, because
the pass was waiting on memory anyway. The rule this leaves for the present
path — the only place a per-pixel look effect can live without a pass of its
own — is that arithmetic is free and texture reads are not: bloom's fold costs
three samples per pixel and is the reason that branch is there, and the tone
map's six extra lines are not measurable next to it.

## 11. Turning the look knobs at runtime (2026-10-16, Apple M4 Max)

`Context::set_bloom` and `Context::set_tone_map` make the two look values live,
which raises a cost question the startup versions never had: what does a frame
pay for *asking* — for a glow that its own chain cannot make yet — and can the
difference between the two states be measured at all on this machine?

**A better instrument, first.** §9 and §10 compared whole runs, which is only as
good as the machine's concentration. This time the toggle is one frame on, one
frame off, inside one process, and the number is the median of the differences
between adjacent frames. Slow drift in load cancels; the machine can be doing
what it likes as long as it does it to both halves. It is also the first
instrument here that measures the two states as an app actually meets them: with
the strength at zero the chain does not run at all, so the "off" frames below
pay no bloom passes, which is what an app toggling the glow on and off gets.

**The wake, measured eight times**: six textures, nine bind groups, their
uniforms and a fresh blit bind group, on the one frame that asks.

| buffer | chain | rebind |
|---|---|---|
| 960×540 | 0.099 / 0.206 / 0.439 ms | 0.008 / 0.016 / 0.022 ms |
| 1920×1080 | 1.086 ms | 0.034 ms |
| 4112×2572 | 0.115 / 0.126 / 0.130 / 0.465 ms | 0.009 / 0.010 / 0.009 / 0.027 ms |

Median about 0.13 ms, and nothing in the table grows with the buffer: the cost
is bookkeeping, not texels, and 4112×2572 is no more expensive to ask for than
960×540. The spread is the machine (§9's numbers and these were not taken on the
same kind of day — see below), but even the worst observation is an eighth of a
60 Hz frame. Building a chain mid-run is not a thing to design around.

**The look deltas, paired.** At 4112×2572, nine bloom passes: **+0.500 and
+0.571 ms** in two independent runs (41 and 51 pairs). The tone map, same
method, three buffer sizes: **−0.018, +0.054, +0.103 ms**, against interquartile
ranges of ±0.15 to ±0.40 ms. §10 could not see a tone map with a cruder
instrument and four thousand frames; this one cannot see it either, which is the
same answer from a better method, and the more interesting claim from §10 —
that per-pixel arithmetic in the present pass is free next to a texture read —
survives it.

**Conditions, because they mattered.** This session the machine was not quiet:
load average 2.2–2.6 throughout, and the same binary that reported a 1.05 ms
median frame in §9 reported 3.5–8 ms here, six to eight times slower, at the same
settings. Absolute spans are therefore not comparable between sections, and no
new absolute span is quoted above — only paired differences and the wake, which
is CPU-side bookkeeping. One arm (bloom at 960×540) reported a +8.8 ms paired
median, which is eight times the whole frame it was measured inside and is
certainly not a bloom cost; I have no explanation for it, and the honest handling
is to report that the arm exists and to leave it out of the conclusion rather than
to re-run until it agrees. Thousands of toggled frames produced no validation
errors and no warnings.

Still open, and this session is the reason: the size sweep for bloom with a quiet
machine — three sizes, paired, is the experiment that would replace §9's two-bullet
reconstruction with a curve — and the 7900 XTX row of both sections.

## 12. Making the glow wider (2026-10-16, Apple M4 Max)

Not a timing section: this one is about what actually moves the width of the
halo, because "the bloom is too subtle" turned out to be two separate asks — more
brightness, which is the app's strength, and more reach, which is the engine's —
and the obvious lever for the second one is a trap. Measured with the present
harness, which renders a real frame and reads pixels back, in a 64×64 buffer,
with a three-pixel dot and two larger shapes for company. Readings are eighths
of full scale from the window texture, straight outward from the dot.

**Widening the blur's step is worse than not doing it.** The kernel is thirteen
taps folded into six bilinear fetches, so its taps are what the step spaces
apart, and they are sparse enough that spacing them shows through. Fall-off
profile by multiple of the current step:

| step | profile, 2 to 27 px out | shape |
| --- | --- | --- |
| 1.0 | 45 42 41 38 33 29 28 25 20 17 15 14 … 6 | monotone |
| 1.5 | 47 43 38 36 **39 40** 37 33 29 27 28 28 … | a bump at +6 px |
| 2.0 | 46 45 40 35 29 29 **32 35 37** 35 31 27 23 22 **24 25 26** … | two bumps |
| 3.0 | 20 22 24 25 24 23 20 18 17 17 17 18 19 20 22 22 … | no shape at all |

Those bumps are rings around the source: the taps arriving individually instead
of as a kernel. At three times the step there is no halo left to speak of, just
a flat lift — fog, not glow. So `spread` stays where it is, and the finding is
written next to it in `bloom.rs`, because reaching for the step is the first
thing anyone tries.

**Levels are the width control, and they were one short.** A level is a halving
of an already-averaged texture, so its taps stay well sampled however far down it
sits; the chain went from three levels to four. Before and after, same scene,
same strength:

| | 2 | 5 | 8 | 11 | 14 | 17 | 20 | 23 | 26 | px out |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| three levels | 45 | 38 | 28 | 20 | 15 | 14 | 13 | 11 | 8 | |
| four levels | 41 | 35 | 27 | 20 | 16 | 14 | 13 | 12 | 9 | |

The rim gives a little and the tail takes it — +25% at 26 pixels out — and the
profile stays monotone, which is the whole reason for choosing levels over step.
The rim's giving is not a loss: the fine level is also what adds a source's light
back onto its own face. A black patch inside a bright plate had +0.625 of
radiance handed back into it before this change and +0.556 after, at the same
strength, and a wider patch went +0.290 to +0.276. That fill-back is what people
describe as the glow blurring the thing that glows, and it lives almost entirely
in the fine level.

**The weights are not a width control, which was the surprise.** Moving the
balance outward — `[0.32, 0.26, 0.22, 0.20]` against `[0.40, 0.28, 0.20, 0.12]` —
changed the tail 20 pixels out from 10 to 11 and left the frame's overall
brightness where it was. The pyramid averages on the way down, so a coarse level
holds the *mean* radiance of its footprint: a small source contributes almost
nothing to the wide levels no matter how they are weighted, and a large one
contributes nearly all of it. Hence the rule worth remembering about this chain,
and the reason a small emissive sprite in a big window will never be given a
great halo by any set of numbers here — **reach in frost's bloom belongs to big
sources.** The eyes in the lighting demo are 40 to 80 pixels wide, which is why
the fourth level shows up there at all.

**What it costs, and what that figure is.** Twelve passes where nine were
measured at +0.21 ms at 1920×1080 (§9). Three more of the same, at the same fixed
cost per pass, says about +0.07 ms — that is arithmetic performed on a
measurement, not a measurement, and it is the only number in this document that
is not one. The paired A/B method of §11 cannot compare two level counts inside
one binary, since the count is baked into the textures the chain builds, and this
session's machine was loaded. It goes on the open list with the size sweep.
Everything else here is the harness and a pixel readback, and 408 lib tests are
unchanged by the change, including the one that holds the glow's fall-off.
