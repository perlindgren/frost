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
    -p antifrost --features diag
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

Escalation is the whole machine for the length of one command, and the
session's permission preset is the whole machine all the time; neither is
the narrow answer. The narrow answer is the device node itself, which the
sandbox does not deny but *erases*: the profile is a literal argv list in
`dsh-sandbox-local/lib/index.js` that mounts `--dev /dev` — bwrap's own
minimal tree — so inside it `/dev/dri` is not a path that was forbidden,
it is a path that does not exist.
[`tools/dsh-dev-dri-patch.sh`](../tools/dsh-dev-dri-patch.sh) adds one
opt-in variable to that list, `DSH_SANDBOX_DEV_BINDS=/dev/dri`
(colon-separated, empty by default, so an untouched deployment behaves as
before), which appends `--dev-bind /dev/dri /dev/dri` after the tmpfs and
honours only existing paths under `/dev/` so it cannot be steered into a
general mount knob. Verified here by building the profile by hand: without
it `ls /dev/dri` answers *No such file or directory*; with it,
`vulkaninfo --summary` inside the same profile names `RADV
RAPHAEL_MENDOCINO` and `RADV NAVI31`. Two catches, both of which have
already cost an hour in the abstract: it edits a **built bundle inside the
`npx` cache**, which the next `npx` resolution overwrites without a word —
re-run the script after any harness update — and a running server holds
the old module in memory, so it takes effect only for a newly started
`dsh web`. Check with `ls /dev/dri`.

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
       -p antifrost --features diag
   ```

2. **Run uncapped from your session** (`--vsync off` is the demo's flag;
   the overlay now reads `VSync: off` next to the resolution, and Alt-V
   flips it live):

   ```sh
   XDG_RUNTIME_DIR=/run/user/$UID WAYLAND_DISPLAY=wayland-0 \
   RUST_LOG=trace timeout 14 \
       ./target/release/antifrost --vsync off > bench.log 2>&1
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
   WxH … present_mode` (right size, `AutoNoVsync`) — plus, since the Mac walk
   of §17, the closing `benchsuite: drew into WxH px … monitor @ Hz, fullscreen,
   visible` line, which is the geometry the window *settled* at and not the one
   it asked for. Then:

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
session's machine was loaded. It goes on the open list with the size sweep. (A
game-side reading of the whole twelve-pass chain arrived later the same day and is
§13; it is not the paired one asked for here, and does not retire this sentence.)
Everything else here is the harness and a pixel readback, and 408 lib tests are
unchanged by the change, including the one that holds the glow's fall-off.

## 13. What the glow costs in the game (2026-10-16, Apple M4 Max)

antifrost grew an ants' glow and a settings row — `Settings › lamp glow` — that
switches the chain between 1.0 and nothing. It shipped quoting twelve render passes
and no number; here is the number, in the game that ships the row.

§3's instrument rather than §9's: the release binary built with
`debug-assertions=true` and `diag`, run uncapped (`--vsync off`) with `--bench 12`
so it exits on its own, at the buffer the hill fixes itself to (1920×1080,
`Quality::High`). The reading is a frame's whole cadence — frames counted per
wall-clock second out of the `frame presented` trace lines, medians of the steady
middle seconds, first and last second dropped. That is coarser than §9's command
span and a different quantity: it carries present pacing and everything else a frame
is made of, and it is the one a player feels as frame rate. `noisefloor` read 0.06 ms
of excess lateness and a 1.3× quantum spread before the runs, so the machine was fit
to be measured and says so.

| chain | pair 1 | pair 2 | pair 3 | median | spread |
| --- | --- | --- | --- | --- | --- |
| off | 9.76 | 9.71 | 9.62 | **9.71 ms** | 0.14 ms |
| twelve passes at strength 1.0 | 10.10 | 10.10 | 10.00 | **10.10 ms** | 0.10 ms |

**+0.39 ms a frame**, two and three-quarter times the off side's own spread, four
percent of the frame it runs in. Pairs were interleaved off/on, off/on, off/on, and
that is not decoration: the first attempt at this was a debug build reading 97 fps
and then 88 fps from two *identical* settings, which is a drift the interleaving
strangles and a debug build's scene traversal otherwise passes off as the cost of a
render pass. The on side came back 10.10 / 10.10 / 10.00.

Two readings of the chain now exist and they do not have to agree. §9's +0.21 ms is
the span from a frame's commands assembled to the GPU done with them, in a frame
whose whole span was 1.05 ms; this one is 9.7 ms of everything. Both put the chain
at a fraction of a millisecond per pass, and this is the number the row quotes.

**This does not retire §12's prediction.** Subtracting a nine-pass span from a
twelve-pass whole frame is arithmetic between two instruments, which is the thing
this document has argued against all day. The paired comparison still wants two
binaries built at different `LEVELS`, and the prediction stays the only figure here
that is not a measurement.

**What is still not measured**: any of it in a browser. antifrost also runs there,
the chain's twelve passes have never been looked at on WebGPU, and a phone-class
iGPU is the hardware the row exists for. That is the honest reason the row is a row.

## 14. The suite on the machine the suite was written on (2026-10-23, Ryzen 9 7950X3D)

§6 through §13 were measured on the M4 Max. This is the return to the campaign box of
§4, and it is here because `benchsuite.baseline` was rewritten on the evening of
2026-10-19 with a header naming a GPU this document has never seen and this project
does not own.

**The instrument.** `benchsuite`, whole suite: ten scenarios × 6 s × 3 interleaved
reps, 1920×1080, vsync off, the release build with `debug-assertions=true` so the
adapter line exists at all (§1a). `noisefloor` read 0.00 ms of excess lateness and a
1.1–1.2× quantum spread before each run, and each run's three reps agreed within
0.2%. Both GPUs answer only under escalated access — in a plain shell this box is
llvmpipe alone, exactly as §1c warns — so `using adapter:` was read before a single
number was.

**Two names this document leans on, defined once here.** The suite's ten rows are an
ordered cost ladder (the list and what each row isolates is
[`examples/benchsuite/main.rs`](../examples/benchsuite/main.rs)), and the document calls
its two ends by name:

- **the floor** — `fill-1080p`: a screen of lit shapes under an *empty* light field. The
  cheapest frame the suite can paint, so everything else in the table is this plus work.
- **the cliff** — `taps-9`: ten lamps and sixty occluders under nine penumbra taps, the
  heaviest row by a mile — 64.5 ms a frame on the iGPU below, against the floor's 1.03.
  The row where a GPU's real throughput has to show, and the reason the other nine exist.

Both come out of **one run, vsync off**: the two names are two *scenes*, not two
settings. And they are scenario names, not §3's *noise floor* — where this document says
"the floor is too loud" it means the witness, and where it divides a floor by a cliff it
means these two rows.

fps, medians of three reps, against the committed table:

| scenario | `benchsuite.baseline` | Radeon 610M (iGPU) | Δ | RX 7900 XTX (`DRI_PRIME=1`) |
| --- | --- | --- | --- | --- |
| fill-1080p | 1002.7 | 970.5 | −3.2 % | 822.6 |
| lamps-4 | 610.6 | 579.5 | −5.1 % | 821.6 |
| lamps-10 | 354.9 | 333.8 | −5.9 % | 821.3 |
| lamps-40 | 117.4 | 110.3 | −6.0 % | 820.5 |
| lamps-40-static | 117.1 | 109.9 | −6.1 % | 819.7 |
| occluders-10 | 227.1 | 214.7 | −5.5 % | 820.6 |
| occluders-100 | 102.9 | 97.4 | −5.3 % | 818.1 |
| taps-1 | 109.7 | 103.5 | −5.7 % | 819.0 |
| taps-4 | 33.4 | 32.0 | −4.2 % | 814.4 |
| taps-9 | 16.0 | 15.5 | −3.1 % | 781.2 |

**The iGPU row is weather, not a regression.** Ten lines whose per-frame cost spans
63× — 1.03 ms to 64.5 ms — moved between −3 % and −6 %, with nothing specific to any
one of them, and §3 already prices that band: the same run *unconditioned* reads a
uniform 4–6 % kinder. A shift that even across that span is the machine's mood; the
suite has no claim against the tree at `bb22d87`.

**The XTX row is not a measurement of the XTX.** 820 fps on every line, from the
floor with an empty light field to `taps-9` — the line the iGPU renders at 15.5 fps —
1.22 ms a frame everywhere, dipping 5 % only at the cliff, which is where the GPU
finally costs more than the host does. That is §6's finding, *the fps column of any
bench is the display's, not the machine's*, reproduced on a desktop, and it is the
same wall as §4's XTX-at-4K "~120 fps, the cap is host-side — not chased" seen from
the side where the cap is 820 rather than 120. Read it as a standing limit on the
instrument: **at 1080p `benchsuite` cannot price lighting on a GPU that is not the
bottleneck.** Weighing the XTX means work that outruns the present path — §5's
drained spans, or a buffer and a field dense enough to pass 1.2 ms on their own.

**There is no 780M.** The header claimed the table was "walked 2026-10-19 on the
780M". That chip appears nowhere else in the project and cannot have produced those
numbers:

- *Vocabulary.* `780M` occurs twice in the tree — that header and two TODO.md lines —
  in no commit message, and nowhere in `docs/`, which names the M4 Max ten times,
  this box's XTX three and its 610M five. A machine that produced the central table
  of the project is documented never to have existed.
- *Physics.* The 610M is RDNA2 with 2 CUs; a Radeon 780M is RDNA3 with 12. Those two
  do not finish 3 % apart on a fill floor *and* 3 % apart on 64 ms/frame of shadow-ray
  walking. The cliff is precisely where six times the CUs has to show.
- *Provenance.* The file was written here. `benchsuite.baseline` carries the mtime of
  the rebase that landed it (17:05, same afternoon), the commit bearing it was made in
  this checkout at 16:49, and this checkout's reflog runs unbroken back to its initial
  commit on 2026-09-11 on the NVMe of the machine this section is typed on. The one
  remaining suspect was §1c's accidental fallback, ruled out by measurement rather than
  argument: forcing lavapipe alone — `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json`,
  which is how to get llvmpipe on a box that has real GPUs — reads **fill-1080p 206.6,
  taps-9 12.8**, a universe under the table's 1002.7 and 16.0.

The fleet is this box, a 5900X with an RTX 4090, and the Apple M4 of §6–13. The name
was invented; the numbers were not, and the header now says 610M.

**What the correction is worth.** The committed table *is* this box's baseline, so the
suite referees from here today with no re-walk and no second file. And TODO's standing
instruction to price the emission gate, which read *on the 780M* until this section was
written — work on a machine nobody has — now reads *on the 610M*, which is what it
always was: the 2-CU iGPU of the box this document was written on, the scarcest fill
rate in the fleet and the one that answers `DRI_PRIME` today.

**What a new row needs.** Two of the fleet's machines have no column here — the M4 Max
of §6–13 and the 5900X with an RTX 4090 — and the sessions that fill them will not have
been in this one, so what made these two rows trustworthy is written down instead of
remembered:

- **Condition it.** Ten scenarios × 6 s × 3 interleaved reps at 1080p, vsync off, the
  `debug-assertions=true` release build (§1a). One pass is a number, not a reading; the
  reps are what makes the 0.2% agreement above a fact worth quoting.
- **Print the witness.** `noisefloor` before the run, and the run's own noise-floor
  line after it, in the same record. A loud machine reports and declines to conclude
  (§3 step 4), and `benchsuite` exits 3 to enforce that.
- **Read the adapter before any number.** `using adapter:` out of the log, from an
  escalated shell, every single time (§1c). On the 4090 box this is the entire risk:
  NVK and the proprietary driver both answer, both produce a tidy table, and only the
  log says which one a reader is looking at.
- **Ask who answered before believing the column.** Run
  `--only fill-1080p,taps-9` first and divide: this box's floor and cliff differ by
  **62.6×**, and a machine reporting anything near **1×** has answered with its present
  path rather than its GPU. That is how the XTX column above reads 820 everywhere, and
  how §6's Mac read 90.0 on a fill floor and 88.9 on ten lamps; a column that fails
  this goes in with that said next to it, or stays out. §6's two Mac cells are not a
  half-filled row either — they predate the float-frame era, and belong beside this
  table as a glimpse rather than inside it as a cell.
- **Read the panel's number before the ratio.** The divide has a hole the Mac found
  (§17): a ratio above 1 only proves that *something* moved off the ceiling. With that
  panel pinned at its highest fixed rate, 60 Hz, the floor and the cliff both reported 60
  and the test failed as designed. Free the panel — macOS offers no fixed 120, only
  **ProMotion**, its adaptive 24–120 mode, as the setting above 60 — and the prediction is
  a floor near the panel's new rate against a cliff near 39 fps, its own ~26 ms of work at
  the pixels a Retina window paints, which could divide to **~3×** and pass on a floor
  that is still nothing but refresh rate. The tell is cheaper than the test and survives
  whatever the panel does: **a floor that equals the panel's rate to the last digit is the
  panel**, and that is exactly how 60.000 for thirteen seconds gave this up. The
  above-60 half is written to be tested, not believed: the panel will be at 60 for a
  static window even under ProMotion if that is what it decides, which is itself worth
  knowing. No shell can read or set the rate, but the app can — `benchsuite` prints it
  beside its numbers now, `in a 4112x2658 px monitor @ 60.0 Hz` — so the rate is a thing
  to record rather than to argue about, and this document recorded it from both sources
  available: the owner, and winit.

The M4 Max seat has since been walked twice, and §17 is why it stays empty rather than
filled with a number that would be read as silicon: windowed, that machine reports its
panel to six digits, and its two conditioned walks disagree with each other by more than
the suite's verdict threshold.

## 15. Two laws the browser enforces and the driver never mentions (2026-10-23, Ryzen 9 7950X3D, Chromium + RAPHAEL)

Nothing in §1 to §14 is a way for a web measurement to *fail to run*. This is, and it
cost an evening, so the laws go here beside the others — the failure each produces has a
shape, and knowing the shape is most of the diagnosis.

**A `textureSample` is illegal in non-uniform control flow.** The mip level comes from
screen-space derivatives, and a 2×2 quad has derivatives only when all four of its threads
ran the same path. Every frost shader sampled *above* its discard — a discard is chosen per
pixel, so that is exactly the branch the rule forbids — and Dawn refuses the module, taking
the pipeline, its bind group layout and every command buffer touching them down with it.
The result on screen was **505 console messages from one rule**, all of them downstream,
all of them complaining about buffers and none about the shader. **Naga never checks this**,
so the same shaders build and run all day on Vulkan and Metal; native had never seen the
bug. The fix is `textureSampleLevel(…, 0.0)`, exactly equivalent here because every texture
in the engine has `mip_level_count: 1` (`src/backend/app.rs:1860`), and it is now the only
form in the tree: 20 sites converted, `grep 'textureSample(' shaders/*.wgsl` finds none.

**`bytesPerRow` in a texture copy is meant to be a multiple of 256** (`COPY_BYTES_PER_ROW_
ALIGNMENT`), and native wgpu-core accepts any pitch, so the engine passed `width * 4` for
months. Measured rather than believed, and the measurement says the opposite of what the
speculation did: `web/width-check.html` uploads a self-labelled ramp at widths 4 to 966,
draws it, reads it back through the compositor, and every width comes home **0/W wrong,
tight pitch and 256-aligned alike, with the browser silent**. So the padding now applied at
the three upload sites (`src/backend/copy.rs`) is **not a bug fix and must not be described
as one** — it is a hedge against a build that enforces the rule this one doesn't. What it
did buy is the cheap knowledge that the pitch is not the web bug, which ended one line of
inquiry in ten minutes instead of another evening.

**And getting an adapter at all on this box takes three flags.** Chromium's Linux default
is not Vulkan: without `--enable-features=Vulkan` the answer is `No available adapters.`,
and on Wayland the stack says so outright — `'--ozone-platform=wayland' is not compatible
with Vulkan` — so it needs `--ozone-platform=x11`; the Mesa here (26.2.4) is newer than the
browser's blocklist data, so `--ignore-gpu-blocklist` as well. Headless needs
`--use-angle=vulkan` on top, or it reports no adapter while appearing to run perfectly.
Two of the evening's readings came from a headless run that had no adapter at all and was
therefore showing the page's fallback `<div>`, not a frame — which is the same trap as §1c
in a different costume: **an instrument that always answers looks like an instrument that
answered.**

**Read what the GPU drew, not what your readback did.** Two of this section's findings
were fabricated by the page's own reading path and are not in the tree: a nine-argument
`drawImage` that rescaled a 24-px-tall WebGPU canvas into a single row returned a
permutation with a period of 32 columns, *identical* through `textureSampleLevel` and
through `textureLoad` — which is what convinced me the sampler had been cleared by
measurement when nothing of the kind had been measured. One-row canvases and a
two-argument `drawImage` report `0/966` on the same textures, same pitches, same device.
The tell, once seen, is obvious: a **permutation is not what a stride bug looks like**. A
stride shears monotonically, and any reading that repeats is reading its own resampling.

The web build is still broken as of stamp `0f3ea69b`, and this section says so plainly:
shaders compile, the device is made, the loading card clears, and the frame is a mosaic of
8×8 blocks. Two things are settled because they were measured and not inferred — uploads
land texel for texel (above), and the geometry is right: a probe that returned a sprite's
own alpha with flat green where its texture colour went drew the hill, the mounds and every
plant in their correct places. What was left then — what reaches the light — was the wrong
question. The suspect named here at the time (the field's layer masks travelling as **raw bits
inside an `f32`**, `shaders/lighting.wgsl:257`, blamed for the blocks because the occluder walk
is tile-aligned) is **dead**: unlit rectangles with no light field anywhere near them shred the
same way, and the build from before the light field existed shreds the same way in a real
browser. §16 carries what is known now. The lesson worth keeping from this section is the
smaller one: **a suspect stays a suspect until something asks it a question it cannot answer**,
and the question here cost one example with two coloured boxes in it.

## 16. The port has never run here, and the mosaic is not the light field (2026-10-24, Ryzen 9 7950X3D, Chromium + RAPHAEL)

§15 ended blaming the light field's float bit-pun. It was wrong, and two measurements
killed it. First, `examples/placement.rs` draws **unlit rectangles** — no texture, no light
field, no emissive target, `lit` off — and they shred exactly as the hill does. Second, the
build of `5d69399` (2026-10-15, the last commit before `441f65c the frame gets somewhere to
keep more than it can show`) shreds identically **in a real browser**, and that build has no
emissive target, no bloom and no layer-mask pun to lose. The pun is not the bug; a suspect
that survives because nobody asked it a question it cannot answer is a rumour, which is what
§15's last paragraph was.

What the breakage actually is, measured on the probe with the canvas at 1024×768 and dpr 1.
A rectangle asked for at user centre `(0,0)` with extent 256×128 arrives **the correct height,
twice the width, and half covered**: coverage is a square wave, 8 pixels on and 8 off, period
16 along x with autocorrelation 1.0, and the pixel count is *preserved* — 126486 against the
native build's 126704. Nothing was scaled, sheared or blurred. Detail was **decimated**, and
a flat colour survives decimation perfectly, which is the whole reason this hid: the same
shader asked to write magenta with alpha 1 through the same full-screen triangle, the same
scissor and the same arena produced 208650 of the 211072 pixels asked for, correctly placed on
both platforms.

Everything below was eliminated by measurement rather than by reasoning, which is the only
reason it is written down. The drawing buffer's size (CSS box, buffer and style all 1024×768,
dpr 1). The CPU geometry: `Canvas::user_to_pixels` is a diagonal matrix with a centred
translation, and the native probe puts the off-centre box at pixel (211,583) for user
(-300,-200) while the browser's **rows** are pixel-exact. The atlas uploads, which land texel
for texel. The blit's filter, bloom (absent in the pre-HDR build that shreds anyway), and the
float frame buffer (`hdr_frames = false` changes not one pixel). The source of the fragment
coordinate: replacing `@builtin(position)` with a `@location(0)` varying computed in the vertex
stage left the frame **identical to the byte**, as did hardcoding the geometry, as did a
hard step in place of `smoothstep`, as did swapping the oversized triangle for an exact
viewport quad. Three different fragment-stage edits and an HDR switch producing byte-identical
frames is not a shader result — it is the result of a shader whose output is not what reaches
the page, and every one of those edits was reported as a finding before the magenta test
showed what to make of them.

`wgpu`-native on this GPU and driver is clean: the game runs here, which is what §14 is
measured on. Dawn is not. Nothing in this document before §15 was measured on Linux at all —
§1 to §14 are Apple M4 Max — so there is no record of this port ever having been run here, and
`git bisect` from `592e8a1 web dev` (11 October, where the page first exists) to here finds
**no commit that judges clean**: the range is uniformly broken on this machine, which is what
a port that was developed on Metal and only ever photographed on Metal looks like from the
other side. The judge is `tools/web-render-judge.sh`; it builds `minwin` for the browser,
photographs it headless, and measures energy at a period of exactly 16 pixels against the
neighbouring periods, which scores 0.41 on the known-good desktop frame and 2.10 on the known-
broken browser frame. It judged all eleven days broken. Two of its early verdicts were
identical to the byte across commits, which should have been the tell it was measuring its own
harness rather than the tree: a metric that never moves is not a metric.

**The other implementation draws it correctly, which settles where the fault is not.** Firefox 157
— rust-wgpu, the same library this engine builds on natively — with `dom.webgpu.enabled` and
`gfx.webgpu.ignore-blocklist` set in `about:config`, opens the identical page on the identical
machine, GPU and driver, and draws it **right**: PLACE crisp and centred, the green box 512×256 in
the middle of the canvas, the red box 400×200 up and left of it, no stripes, no tiled ghosts.
Chromium 153 — Dawn — shreds the same bytes. `wgpu`-native on this GPU is clean, which is what §14 is
measured on, and no Chromium switch moves the frame by a single pixel: not
`--disable-gpu-shader-disk-cache`, not `--use-angle=gl --use-gl=angle`, not
`--disable-partial-raster --disable-skia-runtime-opts`; `--disable-gpu-compositing` removes WebGPU
rather than routing around the damage. Chromium here will not hand out WebGPU on any device but the
real one — six attempts at SwiftShader and at the OpenGL backend, `requestAdapter()` resolving to
`null` every time, which `web/place/index.html` now says out loud in the page instead of leaving it
to be inferred from a black screenshot — so the second implementation had to come from another engine
rather than another driver.

So the port's state is stated, not guessed: geometry, uploads, layout and shader maths are correct on
this hardware; **the damage is Chromium's**, in Dawn's device work, and an implementation that agrees
with the desktop exonerates the engine. What remains belongs upstream, and the reproducer is
`examples/placement.rs` with `web/place/index.html` — two boxes and a word, 90 lines, nothing else in
the engine needed to show it. One hole in the account stays open, and it is where to start looking:
four fragment-stage formulations produced byte-identical frames, which is either evidence that the
fragment stage is not what reaches the page or evidence that three of those four edits were never in
the wasm that was measured — only the coordinate probe's string was checked in the shipped binary. The
next person to look at this should verify the bytes of the artifact before believing a single null
result in this section.
**The app itself runs, on this machine, in a browser.** `tools/firefox-shot.py` drives headless
Firefox over WebDriver BiDi, waits, and photographs: thirty seconds into `http://localhost:8081/`
the menu is there complete — FIRST WARM, the stove, leaves mid-fall, the termite, `HILL 1 OF 50 /
WARM 0 OF 1 / MOVES 0 OF 5` in lettering that is crisp and the right height, 12333 distinct colours
over a fully lit canvas. The probe page measured the same way is **pixel-identical to the desktop**:
green box 512×256 with 126704 lit pixels, the native count exactly, red box 400×198. So the port is
not broken; one browser's implementation cannot display it, and the reason nobody knew is that this
document had no way to ask a browser anything until today. That is the actual defect §16 ends on —
not a shader, but a missing instrument.

## 17. The suite on the Retina Mac: the display answers three times (2026-10-24, Apple M4 Max)

§14 left a seat for this machine and wrote down what would have to be done to fill it.
Two conditioned walks later the answer is that **the M4 Max has no cell in §14's table**,
and the reason is measured rather than assumed, which is the more useful thing to leave
behind. Machine: the 14-core M4 Max of §6–13 (32 GPU cores, 36 GB), built-in Liquid
Retina XDR panel and nothing attached, **set to a fixed 60 Hz in System Settings**:
ProMotion off on the owner's word, and 60.0 Hz on winit's, which is the difference that
matters — no shell can read the rate (`com.apple.windowserver` answers to no query here,
this platform's version of §1c's missing device node), but the app can ask, and the suite
now prints `in a 4112x2658 px monitor @ 60.0 Hz`. Note that mode is *larger* than the
3456×2234 System Information prints: the desktop runs a scaled HiDPI mode, so every
megapixel figure below is read from a log line and not from the panel's spec. On AC at
100 %, tree `3e9b433`, rustc 1.99.0, the `debug-assertions=true` release build. `using adapter: "Apple M4 Max"` before every run below; on this platform there
is no `/dev/dri` to be hidden from and no llvmpipe costume for §1c to wear, so the
adapter line is cheap and read every time anyway.

**Two flags had to be bought first**, both on `examples/benchsuite`, both off by
default so the committed table's meaning is untouched: `--fullscreen`, because a window
on macOS is paced by the window server, and `--render-size WxH`, because the suite asks
for 1920×1080 *logical* pixels and a Retina display doubles that request — so
`fill-1080p` arrives as 8.3 Mpx against the 2.07 Mpx the row's name has always meant.

Each row below is the two named scenes of §14 — **the floor** (`fill-1080p`, a lit screen
under an empty light field) and **the cliff** (`taps-9`, ten lamps and sixty occluders
under nine penumbra taps) — measured in the same run with vsync off, which is what makes
dividing them a test of who answered rather than a comparison of two settings:

| asked for | surface | the floor `fill-1080p` | the cliff `taps-9` | floor ÷ cliff |
| --- | --- | --- | --- | --- |
| windowed, as shipped | 3840×2160 | 60.2 | 60.0 | **1.003×** |
| `--fullscreen` | 4112×2572 | 381.0 | 59.8 | 6.4× |
| `--fullscreen --render-size 1920x1080` | 4112×2572, rendering 1920×1080 | 368.1 | 167.1 | 2.2× |

**The windowed answer is exact, and that is the finding.** §6 wrote *the fps column of
any bench is the display's* from a pair reading 90.0 and 88.9; this is the same wall to
six digits. At `RUST_LOG=trace` the histogram of §3's awk over `frame presented` lines
counts **60 presented frames in each of thirteen consecutive seconds** — 60.000, not
59.8 and not 61 — with the floor and the cliff reporting 60.2 and 60.0 together. And it
is not the swap chain holding the door: the line after `surface configured` reads
**`present modes: [Fifo, Immediate], resolved: Immediate`** — the least paced mode this
backend offers, asking the driver never to wait — and it presented 60.000 frames a second
anyway. `Config::vsync = false` did exactly what it promises and bought nothing, because
the pacing is upstream of presentation: the engine renders when winit delivers
`RedrawRequested`, it asks for the next one at the end of each frame
(`src/backend/app.rs:1052`), and a macOS window is invited once per refresh. There is no
overrun to measure, because there is no attempt to overrun. §14's ratio test reads
**1.003×**, which is its own definition of a machine answering with its present path.
And because the panel is pinned at 60 Hz, there is no drift to blame: that is a hard
ceiling, not a variable refresh caught mid-move. §6's 90.0, 88.9 and "exactly 120.0" are
the same instrument against a panel running faster — 90 and 120 are the tiers that mode
offers, and its non-round 85.4, 88.9, 91.9 and 92.1 are what a ~90 Hz pace reads with
window-boundary noise on top — which is a claim about that evening's setting, not about
a ceiling, and it was written before anyone thought to record the setting.

**A Space of its own uncaps it, and moves the question.** In a native fullscreen Space
the floor jumps 60.2 → 381.0 without one line of work changing, so the throttle was the
window server and not the machine. But the Space's framebuffer is 4112×2572 — 10.6 Mpx,
five times the pixels every row of the committed table paints — and its cliff reads
59.8, a number close enough to the old one to look like a second ceiling. It is not one,
and the same line refutes it: a Space presenting its floor at 381.0 is not pacing
anybody at 60, so 16.7 ms there is the scenario working rather than waiting. Pinning the
render to 1920×1080 puts the work back where the row names it, and the cliff answers to
another instrument for once: 167.1 fps is 5.98 ms, against the 6.45 ms
the drained offscreen meter charges for the same scenario at the same pixels (the table
at the end of this section), a 7 % difference between two meters that share nothing but
the shader.

**What the geometry is, printed instead of inferred.** Every run now ends with the box it
drew into — `drew into 4112x2572 px (2056x1286 at 2.00x) in a 4112x2658 px monitor @
60.0 Hz, fullscreen: true, visible: Some(true)` — and the event loop logs `window
occluded`. That readout exists because two inferences about this space were wrong. The
suspected one was a swap chain left behind by the fullscreen transition; it was not left
behind — it *followed*, and that was a fault of its own. AppKit animates a window into its
Space and reports every frame of the flight as a resize (2056×1286 → 1329 → 1286 → 1357 →
1286 logical, one of them a window taller than the display it was being animated into),
and honouring each one meant allocating drawables for a size no screen has ever had. The
engine now believes a size only once the platform stops changing its mind —
`PendingResize`, one rebuild per settle, a tenth of a second after the last event, with
the measured flight in its test — so that same transition prints **one** `window resized
to` line, for 4112×2572, and the animation is nobody's allocation any more. What is true is quieter and
cosmetic: the window settles at the display's 2658 px less 86 — the menu bar's 43 points
doubled — so a Space reporting `fullscreen: true` draws 3.3 % fewer pixels than the panel
it fills, which is enough to keep a megapixel count honest and nowhere near enough to
move a number here. `occluded: false` and `visible: Some(true)` say the frames really
were the display's, which matters because the check that tried to prove it failed: a
mid-run `screencapture -x` came back as wallpaper under a black band, because a process
without Screen Recording permission is shown nothing else. Treat such an image as silence,
not as a picture.

The band is not invisible-but-real either: the owner watched a run and saw it, its height
matching the camera notch this panel carries — the same 43 points AppKit reserves. Two of
winit's knobs were fitted to it and measured, one at a time: `with_borderless_game(true)`,
which hides the bar and dock instead of auto-hiding them, and `with_fullsize_content_view(true)`,
which pushes content behind the titlebar. The settled surface stayed **4112×2572** both
times and the band stayed visible; the second knob changed only the resize sequence, the
excursion past the panel's height dropping out of it. Closing the band needs AppKit past
what winit exposes for it. It costs 3.3 % of the pixels, moves no number in this document,
and is therefore left named rather than guessed at — but it is the reason every fullscreen
figure here is a 10.6 Mpx figure and never a 10.9 Mpx one.

**Every line then pays for the display, and the tax is not the stretch.** Pinned, the
floor costs 2.81 ms where the offscreen meter charges 0.231 ms for the same pixels — and
unpinned, with no stretch pass at all, the floor costs 2.63 ms, so the ~2.5 ms is the
present of a 10.6 Mpx surface into its own Space, about 0.25 ns per panel pixel, not the
blit. Nothing about that is additive: the suite's taps-9 frame costs *less* in total
(6.02 ms) than the offscreen meter's bare lit-shape frame (6.445 ms) despite also
carrying the bloom chain, the tone map, the stretch and the present, because the
offscreen meter drains the device every frame and this path lets frame N's scene overlap
frame N−1's present. The ten rows arrive compressed into 2.7–6.0 ms, where the same ten
on the 610M span 1.03–64.5 ms.

**Two conditioned walks, taken as §14 asks** — ten scenarios × 6 s × 3 interleaved reps,
vsync off, `noisefloor` before each (0.46 ms excess and a 2.0× spread, then 0.52 ms and
1.7×; the box's quietest reading all evening was 0.16 ms and 2.2×) and the run's own
witness after. Both were judged against no baseline at all, with `--baseline none`,
because `benchsuite.baseline` belongs to the 610M and judging a Mac run against it
invents a verdict in both directions at once: the floor would read −65 % against a
committed line it was never asked to weigh, the cliff +938 %, and the suite's own rule —
fail if any line falls more than 5 % below its baseline — would call the run broken on
the strength of the display path:

| scenario | 610M (§14) | walk 1 | walk 2 | walk 2 ms | reps, walk 1 | reps, walk 2 | walk to walk |
| --- | --- | --- | --- | --- | --- | --- | --- |
| fill-1080p | 970.5 | 356.5 | 345.2 | 2.90 | 0.4 % | 1.4 % | 3.2 % |
| lamps-4 | 579.5 | 373.5 | 369.4 | 2.71 | 4.6 % | 5.5 % | 1.1 % |
| lamps-10 | 333.8 | 362.4 | 350.6 | 2.85 | 2.4 % | 2.8 % | 3.3 % |
| lamps-40 | 110.3 | 286.4 | 324.1 | 3.09 | 13.6 % | 12.8 % | 12.4 % |
| lamps-40-static | 109.9 | 322.0 | 327.4 | 3.05 | 13.6 % | 5.9 % | 1.7 % |
| occluders-10 | 214.7 | 334.8 | 352.4 | 2.84 | 6.3 % | 6.4 % | 5.1 % |
| occluders-100 | 97.4 | 323.3 | 347.2 | 2.88 | 13.9 % | 11.8 % | 7.1 % |
| taps-1 | 103.5 | 311.1 | 324.1 | 3.09 | 11.0 % | 2.3 % | 4.1 % |
| taps-4 | 32.0 | 228.8 | 225.8 | 4.43 | 4.2 % | 2.2 % | 1.3 % |
| taps-9 | 15.5 | 166.1 | 166.1 | 6.02 | 0.6 % | 0.7 % | 0.0 % |

**The weather is the verdict.** §14's rows were quoted on reps agreeing within 0.2 %;
this machine's reps agree to 0.4–0.7 % at the two ends of the table — the floor, where a
fixed cost holds it, and the cliff, where there is nothing but work — and to 2–14 %
through the middle, and two whole walks differ by as much as 12.4 % on a single line.
The suite's own invariant fails inside that band: `lamps-40-static` is `lamps-40` by
definition and the two came apart by 12.4 % in walk 1 and 1.0 % in walk 2. And
`fill-1080p`, which is the floor, reads *slower* than `lamps-4` and `lamps-10` in both
walks, and slower than `occluders-10` in the second — orderings no engine can produce,
since four lamps over an empty field is the empty field plus work. Against a 5 % verdict
threshold that is settled: a machine that disagrees with itself by 12 % cannot referee a
lighting change, and §3's witness agrees — inside the runs the canary's quanta spread
5.0× and 4.6× against a 3.5× gate, where §3's calibration table says even 4.3×
oversubscription only reaches 3.7×. Those walks printed a table and exited 0 only because
they were asked to judge nothing; with a baseline in hand `benchsuite` would have refused
them, which is exactly the discipline §3 step 4 buys.

**The Mac's real meter is the one no compositor can reach.** Run the drained offscreen
bench (`cargo test --release --lib the_light_field_pays_its_fill_offscreen -- --ignored
--nocapture`) and the same ten scenarios at exactly 1920×1080 answer with a witness of
0.01 ms excess and a 1.3× quantum spread — §14's conditioned walks, and quieter than
anything this box managed inside a run — over a span of 27.9× from floor to cliff, which
is the shape §14 asks for:

| scenario | offscreen ms | fps | | scenario | offscreen ms | fps |
| --- | --- | --- | --- | --- | --- | --- |
| fill-1080p | 0.231 | 4335.8 | | occluders-100 | 1.219 | 820.2 |
| lamps-4 | 0.316 | 3165.0 | | taps-1 | 1.121 | 892.4 |
| lamps-10 | 0.437 | 2290.1 | | taps-4 | 3.116 | 320.9 |
| lamps-40 | 1.159 | 862.8 | | taps-9 | 6.445 | 155.2 |
| lamps-40-static | 1.171 | 853.7 | | | | |
| occluders-10 | 0.624 | 1601.5 | | | | |

That frame has no bloom, no tone map and no present — `b04a597` said so when the meter
landed, and it is why these are a price list and nobody's baseline. It is also the only
place this machine says what the ten rows cost each other.

**So the seat stays empty, on purpose,** and what a later Mac session should take from
this is shorter than the walk that produced it: read `drew into` — and
`surface configured at` beside it — before believing any Mac number, because the suite's
1080p is 3840×2160 here and a fullscreen request settles at the display less its menu
bar; take
`--fullscreen --render-size 1920x1080` if a number off a window is wanted at all, and
divide floor by cliff before showing it to anyone — 1.003× windowed, 2.2× pinned, against
the 62.6× that the box the table belongs to reports. The two Mac cells §6 left from
2026-10-15 are explained by this rather than half-filled by it: they are this section's
windowed reading, taken against a panel running at some rate above the 60 Hz this machine
is pinned to. Record the setting next time — it is the first condition of a Mac number,
and it cannot be read from inside a shell. And when the question is what the light field
costs rather than what the panel can present, run the offscreen meter. A row for the 4090
box is still open, and §14's four instructions stand as written for whoever fills it.

## 18. The same ladder in Godot 4.7.1 (2026-10-24, Apple M4 Max)

§17 left the Mac without a cell and with a question it could not answer alone: is frost's
lighting expensive, or is per-pixel lighting just expensive, and what do the neighbours
charge for the same job? Unity and Unreal cannot be answered cheaply — one needs a Hub and
a license dance, the other has no 2D lighting of this class at all — but Godot was already
in `/Applications`, so this section measures the one neighbour that was reachable, and
stops there rather than implying the other two.

**The harness is `tools/godot-compare`, and it is a mirror, not an approximation of one.**
Same ladder of scenarios, same 1 s warm-up and 6 s measurement, same three interleaved
reps with the median reported, so the tables below can be read across. `run.sh` exists
because Godot aborts — not complains, aborts — when `~/Library/Application Support/Godot`
is unwritable, and because its own flags must follow `--`. Four things had to be measured
before any of it could be trusted, and they are the same four §17 spent its pages on: an
empty Godot scene in a window reads **60.1 fps** with vsync off, the panel again; the same
scene with the window unfocused reads **30.2 fps**, macOS throttling an unseen window to
half refresh, which is why the harness prints `can draw` beside every scenario number
rather than once at the end; `--fullscreen` puts it in a **4112×2572** Space, the same
settled surface §17 argued about, to the pixel, including the notch band; and Godot draws
at content scale **1.00**, so its window pixels are physical pixels and both engines paint
the same 10.58 Mpx raster in this configuration. That last point is what makes the
comparison worth a table rather than a shrug.

**The models are not the same, and the scenarios are named for the work they ask for, not
for an equality that does not hold.** Godot's 2D lights blit a light texture over a
darkened canvas and build shadows by rendering occluder silhouettes into a mask; frost
traces rays per lit pixel against every occluder. So `shadows-1x/4x/8x` is MSAA on that
mask standing in for frost's `taps-1/4/9` penumbra samples — a filter against a trace —
and the honest reading of those three rows is *how each engine sells softness*, not a
same-work race. Measured on the same M4 Max, same surface, `visible: Some(true)`,
`can draw true` on all twenty-seven of Godot's scenarios:

| what it asks for | frost | Godot 4.7.1 |
| --- | --- | --- |
| a lit screen, no lights | **2.60 ms** (385.1 fps) | **3.43 ms** (291.5 fps) |
| four lights | 2.91 ms | 3.49 ms |
| ten lights | 2.90 ms | 3.43 ms |
| forty lights | 4.88 ms | **4.55 ms** |
| forty lights, static twin | 4.89 ms | not built |
| ten occluders | 3.43 ms | 3.66 ms |
| one hundred occluders | 4.43 ms | **3.72 ms** |
| soft shadows, cheapest | 4.42 ms (`taps-1`) | 3.73 ms (MSAA off) |
| soft shadows, middling | 8.20 ms (`taps-4`) | 4.10 ms (4×) |
| soft shadows, dearest | **16.69 ms** (`taps-9`) | **4.09 ms** (8×) |

**Two curves, and the shapes are the finding.** Godot's spans **1.19×** from its empty
frame to its most expensive; frost's spans **6.4×** over the same surface. Read gently:
frost's floor is 0.8 ms cheaper and its many-lights case is a wash — forty lights costs
4.88 ms here against 4.55 ms there, and ten lights is 2.90 against 3.43 — while the soft
shadow is where the money goes: **4.1× more** for nine penumbra taps than for Godot's
softest-possible-with-8×-MSAA. That is not a bug and not an accident: a mask with a filter
on it costs roughly the same however soft it looks, and a per-pixel trace costs what it
looks like. frost is selling an exact penumbra with no resolution ceiling, no bake step
and no light budget to tune; Godot is selling one texture fetch per pixel per light and a
blurred silhouette. §17's measured slope says the same thing from the inside — frost's
shadow term is `taps × occluder work`, linear and unhideable, ~68 µs per tap per sixty
occluders at 1080p.

**The weather, because neither ladder passes §3's gate.** Godot's own reps agreed to
about 4 %, and frost's twin control — `lamps-40` against `lamps-40-static`, identical by
definition — came within 0.4 % of each other, which the pinned walks of §17 never managed.
But the canary inside the frost run read burn spread **3.8×**, past the 3.5 limit, and the
witness taken seconds after the Godot ladder read the same way — **too loud to judge** —
while a witness run alone a minute later read a quiet 2.6×. So these are indicative
numbers with the weather stated, not §14-grade verdicts: nothing here touches a baseline,
nothing here is a row, and §14's four instructions still stand unexercised for this
comparison.

**One coincidence to disarm before it bites anyone.** frost's `taps-9` at this surface
reads **59.9 fps**, which is 60.0 Hz to within a digit and looks exactly like the trap §17
is full of. It is not that trap: the same Space in the same run presents the floor at
385.1 fps, so nothing is pacing anyone at 60 — the work genuinely is ~16.7 ms, and the
ratio test gives 385.1 ÷ 59.9 = **6.4×**, the same figure §17 measured unpinned. The
lesson is the one §14 now carries in both directions: a Mac number at 60 is guilty until
the floor says otherwise.

Still open after this: Godot on a machine where §14's gate can actually be met, Unity and
Unreal unmeasured rather than dismissed, and the RTX 4090 row, which remains the most
useful number any of these sections can still be given.

## 19. What the shadow slope is charged against, and one walk instead of nine (2026-10-24, Apple M4 Max)

§17 measured frost's shadow term as `0.44 + taps × 0.68 ms` at 1080p and §18 priced it
against a neighbour, and neither asked the obvious question: what is a *tap* being charged
for? Two answers were possible and they want opposite fixes — either the walk visits
occluders that cannot possibly block the ray, which is a culling problem, or it visits the
right ones and re-does work that does not vary between taps, which is a loop-order problem.

**The cull already exists.** The packer builds a block per light holding exactly the
occluders whose bounds reach some ray that light fires at a pixel it reaches, and an empty
block answers "nothing stands in the way" without a single record load
(`shaders/lighting.wgsl`, the walk in `occluded`). No win was available there, and the
hypothesis that sent this looking died in the first five minutes, which is the correct
outcome for a hypothesis.

**The walk is the cliff, and the rays are nothing.** A control row was added to both
tables — `taps-9-clear`, ten lamps, nine rays a pixel, and *no* occluders, so the penumbra
ladder runs against empty blocks. Offscreen at 1080p: eight extra rays over ten lamps cost
**0.43 ms**, while the same nine rays over sixty occluders cost **5.62 ms** — **87 % of the
cliff is the walk**, and it is linear in taps to within measurement (first walk 0.669 ms,
nine of them 5.62, i.e. 0.624 each). Nothing was hiding.

**The change: one walk resolves the whole disk.** The tap loop sat outside the walk, so per
occluder, per light, per pixel, nine times over, the shader re-loaded the same 48-byte
record, re-multiplied the occluder's transform for the *pixel* — which does not move between
taps — and re-ran the "is the pixel sitting on this shape" test. `occluded_taps` walks the
block once and returns a mask of which taps found a blocker; the record loads, the pixel's
trip through the transform and the inside test happen once per record, and only the slab
interval, which genuinely depends on where on the disk a ray started, still runs per tap.
It cannot visit more records than the nine separate walks did: each of those stopped at its
own ray's first blocker, and this stops when every unresolved ray has found one — the
longest of the nine prefixes rather than their sum. Semantics are unchanged by construction
and the 429 lib tests, including the shadow ones, still pass. The point-light path — the
single hard ray from a light with no penumbra — was left exactly as it was.

**Measured with the suite, not the meter, in fullscreen at 4112×2572.** Two interleaved
passes, each side's own median of three reps, fused and plain alternating with 15 s of
settle between sides; the two passes of a side agreed to 0.2 % on the heavy rows:

| row | plain | fused | change | the two passes, plain → fused (ms) |
| --- | --- | --- | --- | --- |
| `fill-1080p` | 2.63 ms | 2.46 ms | −6 % | 2.63 → 2.46 |
| `lamps-10` | 2.91 ms | 2.59 ms | −11 % | 2.91 → 2.59 |
| `taps-1` | 4.17 ms | 4.28 ms | +3 % | 4.17 → 4.28 |
| `taps-4` | 8.19 ms | 7.05 ms | **−14 %** | 8.19 → 7.05 |
| `taps-9` | 16.71 ms | 12.53 ms | **−25 %** | 16.72, 16.69 → 12.53, 12.53 |
| `taps-9-clear` | 4.18 ms | 2.79 ms | **−33 %** | 4.18 → 2.79 |

**Three of those rows are null controls and they are the reason the other three mean
anything.** `fill-1080p` runs no lights, `lamps-10` and `taps-1` take the point-light path
the change never touches: they read −6 %, −11 % and +3 %, which is the band this
arrangement leaves for order and heat — fused always ran first in a pass, so the second
side runs hotter. Anything inside roughly ±11 % is not a result here. `taps-9` at −25 % and
`taps-9-clear` at −33 % sit outside it with no overlap between sides; `taps-4` at −14 % is
real but close to the edge. The canary read 4.5× on these runs — loud, self-inflicted, the
suite measuring itself — which is why the agreement between the two passes is quoted
alongside the medians rather than instead of them.

**The offscreen meter got demoted by this experiment, and that belongs in the record.**
The same A/B run on `light_cost`'s drained table produced `fill-1080p` "40 % slower" on a
row that executes none of the changed code, `taps-9` reps of `[6.51, 10.96, 6.53]`, and +43 %
on `taps-9-clear`, a row with no occluders at all. The reason is in its own numbers: the
whole table is ~2.4 s of GPU work with no burn, so every row rides the clock ramp, and
benchsuite's 6 s burn exists precisely to stop that. The meter is still the right
instrument for *shape* — it is what found the 87 % split above, and it said the masked rows
move most (`taps-9-layers` 4.66 → 1.19 ms, `taps-9-half` 6.12 → 4.19 ms, both explicable:
records skipped for a layer mismatch were being paid nine times) — but those four numbers
are directional until the suite repeats them, and none of them belongs in a table next to
conditioned figures.

**One consequence for §18, stated rather than quietly applied.** That section priced the
cliff at 16.69 ms against Godot's 4.09 ms and called it 4.1×. The frost column there
predates this change; at 12.53 ms the same comparison is **3.1×**. The conclusion — a filter
costs almost nothing however soft it looks, a trace costs what it looks like — survives
intact, because the trace got 25 % cheaper and is still three times the mask. The number
does not survive, and anyone reading §18's table should read it as the state of the engine
on the morning of 2026-10-24 rather than as a property of frost.

## 20. Nine rays, one decision: the whole-disk reject (2026-10-24, Apple M4 Max)

§19 left the cliff at 12.53 ms with the tap-invariant work gone, which pinned the rest: the
slab interval, per record, per tap. That looks irreducible until you notice what the nine
rays have in common. Every point of a tap's segment sits within the disk's reach of the
matching point of the centre ray's segment — the two segments differ by `(1 - t) · (q - c)`
and `|q - c|` is at most the reach — so **if the centre segment misses the occluder's bounds
inflated by that reach, no ray of the disk can touch it**, and the nine slab tests below it
would answer "not here" nine times over. One test now decides all nine. In the occluder's
own space the disk is an ellipse, so the inflation is the reach scaled by the transform's
longest column, which bounds the local offset from above whatever orientation the disk ends
up in.

**Boxes only, deliberately.** A polygon's edges have the last word on what blocked means,
and a reject that reads the bounds cannot be trusted to agree with them in the last bits, so
polygons keep the per-tap path and get no speedup. The suite's casters are all boxes, which
means every number below is the flattering case, and a game of convex polygons will see
none of it. That is a limitation of the current reject, not of the argument.

Fullscreen, 4112×2572, two interleaved passes against the §19 build, each side its own
median of three:

| row | §19 build | with the reject | change | reps, before → after (ms) |
| --- | --- | --- | --- | --- |
| `fill-1080p` | 2.97 ms | 2.81 ms | −5 % | [2.90, 3.03] → [2.88, 2.75] |
| `lamps-10` | 2.88 ms | 2.88 ms | 0 % | identical |
| `taps-1` | 4.61 ms | 4.45 ms | −4 % | [4.56, 4.67] → [4.39, 4.51] |
| `taps-9-clear` | 2.89 ms | 2.89 ms | 0 % | identical |
| `taps-4` | 7.08 ms | 4.56 ms | **−36 %** | [7.08, 7.08] → [4.49, 4.62] |
| `taps-9` | 12.53 ms | 5.29 ms | **−58 %** | [12.53, 12.53] → [5.36, 5.22] |

The nulls this time sit inside 5 % — `lamps-10` and `taps-9-clear` came back byte-for-byte
the same twice — which makes a −58 % hard to explain as weather. Subtracting the control row
isolates the walk as §19 did: **9.74 ms became 2.40 ms**. Against the pre-everything build
of §19 — 16.71 ms — the cliff is now **5.29 ms, a 68 % reduction**, which is past the 40 %
§19 predicted and by a wider margin than that prediction's mechanism can claim: the reject
is not an amortisation of work already being done, it is work that stops happening for most
records in most rays, and §19's estimate never contemplated that.

**How the change was checked, and the honest size of that checking.** The suite does *not*
cover this path: every shadow test runs a point-sized light (`penumbra: 0.0`, said outright
at `shadows.rs:380`), so `light_mix` takes the single-ray branch and neither the fused walk
nor the reject is ever executed. That also means §19's "the 429 lib tests pass" was weaker
evidence than it read — true, and about a different code path. What was done instead: the
same open-penumbra scene — six rotated casters at six distances, rendered at nine taps and
again at four — was fingerprinted under all three builds, the pre-change shader, the fused
one and this one, and **the output bytes are identical**: FNV-1a `a800aa6aa18701a1` at nine
taps and `2944b3989634ba89` at four, in all three builds. Bit-exact agreement on one scene
on one GPU is a strong result and not a substitute for a test.

A CPU twin of the nine-ray average was written to do better and withdrawn: it disagreed with
the GPU on 692 pixels of 4096 — and disagreed by *exactly the same 692 pixels* under the
unmodified shader, so the model was wrong and not the engine. Every pixel the model called
fully shadowed was dark on the GPU at every disk radius tried; the failures were all pixels
the model called fully lit, which is the signature of a misplaced caster rather than a wrong
ray — most likely the canvas's y-flip mirroring the occluder's rotation, where a box turned
by +0.55 is not the box the packer shipped. That is a guess, and the guess is not in the
tree. **The coverage gap stays open, and it is now the most valuable untested path in the
engine**: two shipped optimisations rest on a byte comparison and an argument.

**What §18 now costs.** Its frost column has moved twice since it was written — 16.69 ms at
the cliff, then 12.53 in §19, now 5.29 — and the comparison with Godot's soft mask, measured
on the same 4112×2572 surface at 4.09 ms, has gone from 4.1× to 1.3×. §18's conclusion about
*why* the two engines differ still stands; its arithmetic no longer does, and anyone quoting
the gap should quote this section's number instead.

## 21. What a shape costs: the polygon baseline (2026-10-24, Apple M4 Max)

§20 ends with boxes only, and that is a limitation with a consequence: nothing in the suite
had ever measured a caster that was not a rectangle, so the polygon path — where a ray that
passes the bounds goes on to walk the shape's edges, up to sixteen of them, once *per ray* —
was optimised by argument alone. Before aiming anything at it, the rows.

Three new ones, in both tables, named for what they add: `polys-60` (ten lamps, sixty
hexagons, one hard ray), `taps-9-polys` (the same at nine) and `taps-9-discs` (sixty
`DISC_SIDES`-sided discs, the packer's plane cap, at nine). The casters carry the suite's
own screen area — a regular polygon of 21 600 px², the box's 180×120 — so a row differs from
`taps-1` and `taps-9` in its edges and not in its coverage, and the comparison is about shape.
Fullscreen at 4112×2572, median of three reps, one pass; the canary read 3.6×, loud by §3's
gate, so this is a baseline in the sense of *the first measurement of something*, not a
verdict against another build:

| row | ms/frame | above the 2.82 ms floor | per extra ray |
| --- | --- | --- | --- |
| `taps-1` — sixty boxes, one ray | 4.44 | 1.62 | — |
| `polys-60` — sixty hexagons, one ray | 4.49 | 1.67 | — |
| `taps-9` — sixty boxes, nine rays | **5.83** | 3.01 | 0.17 ms |
| `taps-9-polys` — sixty hexagons, nine rays | **21.98** | 19.16 | **2.19 ms** |
| `taps-9-discs` — sixty discs, nine rays | **38.46** | 35.64 | **4.25 ms** |

**At one ray a hexagon is free: 4.49 ms against a box's 4.44.** The shape only starts to
cost when the disk opens — nine rays take it to 21.98 ms, 3.8× the box row's 5.83, and a
disc's sixteen edges take it to 38.46 ms. Per extra ray, a hexagon is 12.6× a box and a disc
24×. Two things follow, and it is worth being clear about which is which. Some of the gap is
the §20 reject working as designed on boxes and being denied to polygons, so boxes get one
whole-disk test per record while a polygon tests its bounds nine times and then walks its
edges. And some of it is the plane walk itself, which `clip_planes` still runs **once per
ray**, reloading every plane each time and recomputing the pixel's side of it — `n · lp`,
which cannot change between taps — even though the disk's nine rays share it.

So the polygon path is now the cliff in this engine, larger than the box cliff that §19 and
§20 spent a day halving, and a scene of discs costs six times what the same scene of boxes
costs. Two candidate fixes are the same two the baseline was written to size: fusing the
plane walk over taps, which is §19's argument applied to the edges instead of the record, and
the plane-based whole-disk reject, which needs the packer's plane derivation read before it
can be trusted. **Neither is measured yet, and this section's whole purpose is that the next
number written about polygons is compared against these ones rather than against a guess.**

## 22. Two attempts at the polygon walk: one lost, one won (2026-10-24, Apple M4 Max)

§21 left two candidates and both were tried the same way, against the same build, two
interleaved passes each, five rows, median of three reps at 4112×2572. The canary read
2.7×–3.5× on every side-run, loud by §3's gate. The HEAD column here reproduces §21's
numbers — 22.00 ms and 38.46 ms against 21.98 and 38.46 from a run an hour earlier — which
is as good a check on §21 as anyone is likely to get.

**The fusion lost, and lost badly.** The idea was §19's argument moved from the record to the
edge: load each plane once per record instead of once per ray, compute `db` once because the
pixel's side of an edge cannot change between taps, carry the nine intervals and narrow them
inside the plane loop, and let one edge that puts both endpoints of every remaining ray
outside it end the record for all nine at once. The arithmetic was bit-for-bit the same and
was checked to be so — three polygon scenes, hexagons at nine taps, hexagons at four,
sixteen-sided discs at nine, fingerprints `2c192f9ffdba54a9`, `b370c6a09a07e5bf` and
`8b05319a3961a004` identical to the committed build. Then the time:

| row | committed build | with the fusion | change |
| --- | --- | --- | --- |
| `fill-1080p` | 2.60 ms | 2.59 ms | −1 % |
| `polys-60` | 4.37 ms | 4.42 ms | +1 % |
| `taps-9` — boxes, nine rays | 5.16 ms | 6.91 ms | **+34 %** |
| `taps-9-polys` | 22.03 ms | 36.63 ms | **+66 %** |
| `taps-9-discs` | 38.46 ms | 71.43 ms | **+86 %** |

Reps within 0.01 ms of each other, so this is not weather. The number that explains it is
the box row: a box never enters the plane loop, so the plane walk cannot be what got slower.
What got slower is carrying nine `(ll, tmin, tmax)` triples indexed by a loop counter — a
local array with a dynamic index, which this driver puts in private memory rather than
registers, so every step of every plane became a memory round-trip instead of a register
read. **Fewer operations, more time.** It is reverted, and the general lesson for this engine
is that "the same expressions, used fewer times" is not an argument for a GPU change on its
own; only the measurement is.

**The bounds reject generalised, and won.** §20 had confined the whole-disk reject to boxes
on the worry that a reject reading the record's bounds could not be trusted to agree with a
shape's edges. `canvas.rs:306` settles it: a polygon's record box *is*
`polygon.bounds_as_box()`, the shape's own bounding box, so the box contains the shape by
construction and a disk that misses the box inflated by the reach misses the shape no matter
what the edges then say about a disk that does reach it. The change is one condition
widened, `edges == 0u &&` dropped:

| row | committed build | with the polygon reject | change |
| --- | --- | --- | --- |
| `fill-1080p` | 2.70 ms | 2.69 ms | 0 % |
| `polys-60` — one hard ray | 4.43 ms | 4.43 ms | 0 % |
| `taps-9` — boxes, nine rays | 5.19 ms | 5.14 ms | −1 % |
| `taps-9-polys` | 22.00 ms | **14.77 ms** | **−33 %** |
| `taps-9-discs` | 38.46 ms | **30.58 ms** | **−20 %** |

Same three scenes fingerprinted byte-for-byte as the committed build, which is the same
evidence §20 accepted and carries the same caveat: one scene family, one GPU, and no test in
the suite covers the path. The saving itself is the evidence that the reject fires — a reject
that never triggered would have changed nothing at all, and −33 % is nothing changing very
much. A hexagon gains more than a disc because the reject removes work proportional to the
edges, and a disc has more edges to lose.

**What is left, and what the next attempt should look like.** A disc scene still costs 30.58 ms
against a box scene's 5.14, so the plane walk per ray remains the polygon cliff and this
section has not closed it, only cut a third off it. The fusion showed what *not* to do; the
shape that keeps the win and avoids the private memory is a fixed unrolled grouping — taps in
pairs, six scalars of state instead of an array of nine, the plane list walked five times
instead of nine. That is a prediction about a mechanism, not a number, and it will be
measured against the four rows above before anyone believes it.

**One protocol note for the next person.** A fullscreen run started while the display is
asleep produced no output and never returned — no frames, no error, no exit, twenty minutes of
silence, and the process was invisible to `pgrep`. Every run since has been wrapped in
`caffeinate -di` with a per-side timeout that kills the benchmark by its own pid. If a
fullscreen suite goes quiet, suspect the display before the shader.

## 23. Taps in pairs: the shape walk, five times instead of nine (2026-10-24, Apple M4 Max)

§22 ended with a shape that avoids the mistake rather than a number: keep the disk's state in
scalars and walk the plane list for two rays at a time, so nine rays cost five walks instead
of nine and the pixel's side of each edge is computed once per walk instead of once per ray.
Three pieces, in service of that. The slab test for one ray moved into `tap_interval`, which
returns its origin and its interval as one `vec4` — four floats a caller can hold, which is
the whole reason it returns a vec4 and not nothing else. The tap loop steps by two. And the
plane walk runs inside the pair, `db` hoisted out of the ray loop, with a plane that strands
both rays' endpoints ending the pair and a pair with nothing left ending the walk.

**Bit-for-bit the same picture again.** The same three polygon scenes as §22 — hexagons at
nine taps, hexagons at four, sixteen-sided discs at nine — fingerprint `2c192f9ffdba54a9`,
`b370c6a09a07e5bf`, `8b05319a3961a004`, byte-identical to the committed build. That was worth
expecting less of than it went: restructuring dot products can change how a compiler fuses
multiply into add, which moves the last bit of a crossing, and the fact that it did not here
is a fact about this driver, not a proof. Fullscreen 4112×2572, two interleaved passes,
median of three, canary 2.6×–4.1×:

| row | committed build (§22) | paired | change |
| --- | --- | --- | --- |
| `fill-1080p` | 2.61 ms | 2.68 ms | +3 % |
| `polys-60` — hexagons, one hard ray | 4.43 ms | 4.44 ms | 0 % |
| `taps-9` — boxes, nine rays | 5.21 ms | 4.86 ms | −7 %, *not established* — see §27 |
| `taps-9-polys` | 14.78 ms | **12.32 ms** | **−17 %** |
| `taps-9-discs` | 30.53 ms | **23.67 ms** | **−22 %** |

The `fill-1080p` row has no casters in it at all, so its +3 % belongs with the ±5 % the nulls
in these runs have been drifting by; everything else moved in the direction the mechanism
predicts and by more than that. The boxes gaining 7 % was not aimed at: pairing halves the
loop's bookkeeping whether or not there are edges to walk, and a box's answer is still the
slab alone.

**That 7 % has since been withdrawn as a measurement (§27).** Those two columns are different
processes, and the same scenario across five processes on this machine spans 4.22–4.53 ms, so
the spread alone is 7 %: the box row could be noise in either direction. What is left is the
mechanism — pairing halves the bookkeeping on a box too — and the honest claim about boxes is
that the paired walk left them unchanged within measurement, until more alternating passes say
otherwise. The two shape rows are 17 % and 22 %, both far outside that band, and they stand.

Taken from §21's baseline rather than from §22's build, the two shape rows are **21.98 → 12.32
ms and 38.46 → 23.67 ms**, and the box cliff that started this is 16.71 → 4.86. Cutting the
walks from nine to five took roughly a fifth off a hexagon and a quarter off a disc, which
locates the remaining cost: the plane loads were a large share of a shape's walk and not the
whole of it, and what is left is the per-ray slab and dot work that no amount of pairing can
share, since every ray genuinely has its own.

**The next lever this exposes, and it is a mechanism and not a number.** Two are visible. A
pair can become three or four, taking nine rays to three walks; the same private-memory
objection applies, so it is measured or nothing. And the interval a plane leaves behind is
monotone — `tmin` only ever rises, `tmax` only ever falls — so a pair whose interval is
already empty cannot be revived by a later edge, and the walk can end for it at that plane
rather than at the last one. That one is provably outcome-preserving rather than merely
bit-preserving, which makes it the cheaper of the two to trust. **The §20 caveat still
applies to everything above: no test in the suite runs an open penumbra, and all three of
these changes rest on byte-identical frames from one scene family on one GPU.**

## 24. The CPU side of a particle batch (2026-10-24, Apple M4 Max, 14 cores)

Every row so far shades pixels, so every cost so far has been a GPU cost. A particle batch
spends real time before the GPU sees it: the simulation steps every live particle, and the
frame rebuilds the batch's instance bytes from nothing — a fresh `Vec`, a transform per
particle, 32 bytes appended each. None of that appears in a pixels-per-second number, and all
of it is one thread doing arithmetic, so it can be measured without a display, an adapter, or
a quiet machine. The instrument is `src/backend/tests/particle_cost.rs`, run by hand:

```text
cargo test --release --config 'profile.release.debug-assertions=true' \
    --lib particle_cost -- --ignored --nocapture
```

It goes through [`Canvas::draw_scene`](crate::Canvas::draw_scene), the call a game makes, over
a scene whose only content is one batch, and it prints the controls beside the costs rather
than netting them out of sight:

| what | median | per particle | what it is |
| --- | --- | --- | --- |
| `walk` | 0.0001 ms | — | the scene walk and a fresh canvas, batch empty |
| `pack-1000` | 0.0124 ms | 12.4 ns | |
| `pack-10000` | 0.171 ms | 17.1 ns | reps 0.109–0.285, the widest row here |
| `pack-100000` | **1.545 ms** | 15.5 ns | |
| `instance-10000` | 0.118 ms | 11.8 ns | the instance bytes alone, into a buffer that never grows |
| `append-10000` | 0.034 ms | 3.4 ns | the same buffer, filled with a constant |
| `sim-100000` | 0.210 ms | 2.1 ns | `ParticleSystem::update`, nothing dying |
| compaction, 10 000 dying | est. 0.025 ms | — | difference of two medians, so an estimate |
| one 3.2 MB allocation | 0.0043 ms | — | what a 100 000-particle batch asks the allocator per frame |

**About 15 ns a particle, linear, with no fixed cost worth naming.** A hundred thousand of
them cost ~1.75 ms of one frame — 1.55 to pack, 0.21 to simulate — which is real but is not a
cliff: it is a third of the lighting cliff of §23, at a particle count a 2D game would call
heavy. The pack is the instance function and not much else; the buffer append is a quarter of
it; the per-frame allocation, the scene walk and the `retain` compaction are all under 1 % of
the row they sit in. A batch's `lit` flag made no difference I could see, though its spread
overlaps the lit row's, so that is "not visible", not "free".

**The trap this bench walked into, recorded so the next runner doesn't.** The first run of it
was `cargo test --lib particle_cost`, without `--release`. It read ~90 ns a particle, six
times the truth, and every row's shape still looked plausible — sub-linear, allocation-flavoured,
easy to explain to oneself. An unoptimised build turns eight slice copies in a packing function
into eight function calls, and there is no canary in a CPU bench to say *the machine is not the
problem, the build is*. `--release` is now in the bench's own header, and the row that would
have been the finding — a packing function needing work — was an artifact of the measurement.

**What this bench cannot say.** Nothing about the GPU: how many quads a frame instances, what
their coverage costs, or what a *lit* batch pays through `light_mix` per covered pixel — which
is the one place particles and lighting genuinely meet, and the subject of the probe that
follows.

## 25. Particles meet the frame: cheap until they are lit (2026-10-24, Apple M4 Max)

§24 measured the CPU phase of a particle batch and could say nothing about the frame it lands
in. This is the GPU side, and it went into the benchsuite rather than a hand-built bridge,
because the suite renders through the real renderer and a second implementation of the particle
pipeline would have drifted from the first. The rows: a batch's count, its coverage, and its
`lit` flag, with the two ladders built to hold one thing fixed each — `parts-1k/10k/100k`
keep the *coverage* at a quarter of the window while the count runs up two orders of
magnitude, and `parts-10k-big` holds the count and quadruples the coverage. `parts-huge` is
the canary the whole set needed: 200 circles of 500 user pixels, about a hundred times the
window's area.

Two findings, one anomaly, and one design flaw worth naming.

**A batch is cheap unlit and expensive lit.** Run twice in opposite row orders, median of two
reps each, fullscreen 4112×2572, ten lamps, no occluders:

| row | order A | order B | above the control |
| --- | --- | --- | --- |
| `lamps-10` — no batch | 2.88 ms | 2.87 ms | — |
| `parts-10k-lamps` — 10 000 particles, unlit | 2.92 ms | 2.91 ms | **+0.04 ms** |
| `parts-10k-lit` — the same batch, `lit: true` | 3.45 ms | 3.47 ms | **+0.57 ms** |

Twenty-five percent of the window's coverage costs 1.4 % of the frame as an unlit quad layer
and 20 % of it as a lighting receiver — **fourteen times the difference for identical pixels**,
because a lit particle pays `light_mix` per covered pixel per light like any other receiver.
That is the one place a particle frame and a lighting frame are the same question, and it is
the reason §19–§23's work reaches a particle scene at all. *Prediction, unmeasured:* a lit
batch over the cliff scene of §23 should pay the occluder walk per covered pixel too, which
would make a particle effect in front of a penumbra the most expensive frame in the engine.

**Coverage, not count, is where a particle frame dies.** `parts-huge` read 8.22 ms against the
2.75 ms of the same frame without it — +5.5 ms, two orders of magnitude outside the noise of
everything else here. Count, at fixed coverage, barely moves: 10 000 particles are 0.04 ms
unlit, and §24's ~15 ns a particle says why 100 000 of them only reach ~1.5 ms of CPU.

**The control inversion I could not explain, and the count ladder it disqualifies.** In the
first ladder, `fill-1080p` — the same scene with no batch at all, same lamp count, same
everything — read *slower* than the rows carrying particles: 2.74 ms against `parts-10k`'s
2.50 ms, in both passes, with reps tight inside each row. Rows that ought to be equal differed
by 0.3 ms, which is larger than the whole effect the ladder was built to see. The swapped run
above rules out row order as the mechanism: the control read 2.88 first and 2.87 last. So the
`parts-1k/10k/100k` numbers stay in the table and out of the conclusions — **a control that
disagrees with its treatment by more than the treatment's effect is not an instrument, and
these rows need that anomaly explained before they are quoted.**

**The flaw, because it is the instructive kind.** `parts-10k-lit` first shipped with `lamps: 0`
in it: a receiver row in a scene with nothing to receive. It read 2.48 ms, it looked like
evidence that lighting barely costs a batch, and it answered a question nobody had asked. The
row now carries ten lamps and has an unlit twin beside it, and the answer changed by a factor
of fourteen.

**One apparent contradiction, resolved.** §24 measured the pack at ~17 ns a particle —
0.17 ms for these 10 000 — yet the unlit row adds 0.04 ms to the frame. Both are right: the
frame here is GPU-bound at 2.88 ms, so the CPU phase hides underneath the GPU's work instead of
lengthening the frame. §24's number bites when the CPU phase becomes the critical path — a
short frame, a heavy batch, or a scene too cheap to keep the GPU busy — and not before.

## 26. A lit particle batch over the cliff (2026-10-24, Apple M4 Max)

§25 predicted that a lit batch in front of the §23 cliff would pay the occluder walk per
covered pixel, and said so without measuring it. Three rows close that: the cliff scene —
ten lamps, sixty box casters, an open penumbra at nine taps — with a 10 000-particle batch
over a quarter of the window, once unlit, once `lit: true`, and once with the same lit batch
while the disk is shut to a single hard ray. Run in both row orders, median of two reps each;
the two orders agreed to 0.03 ms on every row, so nothing below is order or drift.

| row | ms/frame | over the same scene with no batch |
| --- | --- | --- |
| `taps-9` — cliff, no batch | 4.91 | — |
| `parts-10k-cliff` — unlit batch | 5.28 | +0.37 |
| `parts-10k-cliff-lit` — **lit batch, nine taps** | **6.66** | **+1.75** |
| `taps-1` — cliff, one hard ray, no batch | 4.50 | — |
| `parts-10k-shadow-lit` — lit batch, one hard ray | 4.91 | +0.41 |

**The prediction was right, and the disk is the whole of it.** Against its own unlit twin the
lit batch costs **+1.38 ms**, and with the penumbra shut to one ray the same batch adds about
+0.04 over its own drawing. A quarter-screen of particle pixels walked nine times in front of
sixty casters is therefore worth more than a third of the entire frame, which is the cost
§19–§23 spent two days cutting — and since `particles.wgsl` calls that same `light_mix`
([line 107](shaders/particles.wgsl)), the polygon reject and the paired walk were already
applied to it, for free, with nothing particle-specific to do.

Two footnotes that are really corrections to how §25 should be read. Its rows carry
`penumbra: 0.0, taps: 1`, so its +0.57 ms was falloff and cone with a single hard ray and no
occluders in the scene — a fifth of a frame for a quarter of its pixels, but nothing to do with
shadows; the shadow half is this section's, not that one's. And its rows had no casters, which
is why the question came up at all.

**What does not add up.** Take the lighting-only cost of the same quarter-screen at one hard
ray: §25's open scene leaves +0.53 ms after subtracting the unlit batch, this section's
sixty-caster scene leaves about +0.04. Same lamp count, same single ray, strictly more work,
cheaper by an order of magnitude. The unlit batch's own cost also moves ninefold between the
two scenes — +0.04 where the GPU has slack and +0.37 where it is the critical path, which is
the §25 resolution of the CPU pack and entirely expected. That explains a great deal of the
sensitivity here and none of the sign of the discrepancy. **So treat the small residuals in
these two sections as unsettled**; the +1.38 ms above is the one particle-lighting number
large enough to survive the instrument, and the experiment that would settle the rest is the
same lit batch at one ray measured with and without casters in a single pass, several reps,
both orders.

## 27. Two comparisons that were never comparisons (2026-10-24, Apple M4 Max)

§26 left a residual with the wrong sign and §25 left a control reading slower than its
treatments. Both came from the same place, and neither was the engine.

**The residual was arithmetic.** A row was added — `parts-10k-shadow`, the unlit batch over
sixty casters at one hard ray, the twin `parts-10k-shadow-lit` had been missing — so that
lighting-only cost could be subtracted in both scenes instead of one. Measured in one
invocation, in both row orders, median of three:

| scene | unlit batch | lit batch | lighting only |
| --- | --- | --- | --- |
| ten lamps, no casters (§25's) | 2.91 ms | 3.48 ms | **+0.57 ms** |
| ten lamps, sixty casters | 4.30 ms | 4.91 ms | **+0.61 ms** |

They agree. The +0.04 ms that couldn't be explained was `parts-10k-shadow-lit` minus `taps-1`
— a batch row minus a *no-batch* row, which charges the batch's own drawing to the lighting.
With its proper twin the sign is right and the two scenes say the same thing.

**The inversion was the process, not the row.** §25's control read 2.74 ms against a treatment
at 2.50 ms, and row order was ruled out then and again here: the same rows in opposite orders
inside one invocation agree to 0.03–0.07 ms, about 1 %. What differs is one invocation from the
next. `taps-1`, the same scenario with the same flags, read **4.22, 4.29, 4.47, 4.50 and 4.53 ms**
across five processes today — a 7 % spread on a row nobody changed. Compare that row with the
batch row inside one process and the batch costs +0.04 and +0.14 ms, the right sign; compare
them across processes — 4.50 from one run, 4.30 from another — and a batch appears to make a
frame 0.20 ms *faster*, which is what §25 recorded as a mystery.

**The rule, then: two numbers are comparable when they came from one invocation, and not
otherwise.** That is what `--reps` interleaving is for, and it is why the suite runs a row
several times through a process rather than once each in several processes. For an A/B across
builds it cannot be honoured — the two sides are by definition different processes — so those
comparisons lean on the alternating passes and on the effect being larger than the band.

**Which of this document's numbers that touches.** Everything large is fine: §20's −58 % and
−36 %, §22's +66 % and −33 %, §23's −17 % and −22 %, §25's +0.57 ms and §26's +1.38 ms, all of
them either measured inside one invocation or many times the width of the band. **§23's box-row
−7 % is not.** It was measured across processes, it sits inside the 7 % spread, and it is
therefore *not established*: pairing halves the loop's bookkeeping on a box, which is a
mechanism worth believing, but the number that was quoted for it today could be noise in
either direction. Re-measured across more alternating passes it either earns its place or
loses it, and until then the honest claim about boxes is that the paired walk left them
unchanged within measurement.

And the anomaly §25 asked to be left open is closed by this section, with the explanation
being the instrument rather than the renderer — the more useful of the two possible outcomes.

---

## 28. The penumbra, measured against geometry instead of against itself (2026-10-26, Apple M4 Max)

Everything §20 through §27 measured about the soft edge was measured *around* it: frame times of
scenes whose lights ran `penumbra: 0.0`, and a CPU twin that disagreed with the GPU on 692 of 4096
pixels and was dropped (§20). So the disk of shadow rays — the most expensive code path in the
renderer, nine occluder walks per pixel per light — had no test that entered it. This section closes
that, and it is worth recording that three of the four bugs it found were in the measuring.

**The route: properties, not a twin.** `src/backend/tests/penumbra.rs` renders one scene through
the bridge `shadows.rs` already had (`paint`, made visible to the module) and asserts what the
geometry forces however the disk is sampled: an umbra that is exactly dark, open light that is
exactly full, an outer silhouette at the distance similar triangles put it, a shadow centred on the
light's own axis, and a total amount of darkness that does not care how many rays drew it. Nothing
in the file re-implements the shader, so nothing in it can be wrong in the same direction as the
shader — which is the whole reason §20's twin could not settle anything.

**Meter bug one: the light's own falloff is a shadow-shaped hole.** The first draft normalised each
column by its brightest byte and looked for the first row to fall half a tap below full light. Along
a vertical column the light's falloff does exactly that, some pixels before any shadow begins, so
the silhouette came out a quarter wider than the geometry allows at every rung above two. The point
light, whose level sits far below the falloff, measured flawless throughout — the instrument was
biased in a way that only appeared once the thing being measured got soft enough to need measuring.
Fixed by rendering each column twice, with the caster and without, and dividing row by row: the
falloff cancels because it was recorded in both frames rather than assumed away.

**Meter bug two: the two flanks were answering different questions.** Half light on the way in from
the top, half light on the way in from the bottom — on a coarse rung those are different boundaries,
and a symmetric shadow reported itself up to 3.5 px lopsided. Both flanks now use the same
criterion, walked inward from full light.

**Model bug, mine: the silhouette grazes the near edge.** My first prediction had the outermost ray
grazing the caster's *far* edge and continuing on. It grazes the *near* edge — the segment starts on
the light's disk, enters the box's near face and the box is 4 px deep, so the ray that just clears
it is the one clipping the near face at the far corner. With that corrected the engine matches
arithmetic, and these are the numbers, all of them absolute:

| quantity | measured | computed | where |
|---|---|---|---|
| point light's shadow, half-width | 13.50 px | 13.71 px | probe 40, `penumbra: 0` |
| point light's shadow, half-width | 20.50 px | 20.57 px | probe 56, `penumbra: 0` |
| cross's silhouette, half-width | 27.50 px | 27.86 px | probe 56, taps 4, reach = rad |
| grid's silhouette, half-width | 29.99 px | 30.87 px | probe 56, taps 9, reach = √2·rad |
| one ray, soft band | −0.48 px | 0 | no penumbra is possible |
| one ray, umbra width vs silhouette | 54.00 vs 54.00 px | equal | a ray's core *is* its shadow |
| light eaten, rungs 2…9 | 27.34–28.59 rows | rung-independent | probe 40 |
| light eaten, rungs 2…9 | 42.00–43.50 rows | rung-independent | probe 56 |

The point-light rows are the ones §20 was after: readback row order, light-record layout and the
occluder-space `m` are all confirmed against arithmetic, to a quarter pixel, without a second
implementation anywhere. The silhouette residuals are one-sided by construction and stay that way —
along a column the rotation rarely puts a tap exactly on the rim, so a measured shadow can fall
short of its rim's shadow and must never exceed it, which is asserted as its own clause. And the
disk does eat slightly more than a point, 28.0 rows against 27.0: the disk extends *along* the axis
toward the caster, and a ray nearer the caster magnifies more, so its hard shadow is wider. Measured
first, explained after.

**A claim of mine, withdrawn.** The tap tables take a *prefix* of a row-major 3×3 grid, and a prefix
is off-centre — mean sample offset 1.00 radii at one tap, 0.63 at five, 0.32 at seven, balanced only
at two, four and nine. I said this displaced the penumbra edge by up to two thirds of the light's
radius, worst on soft lights. **It does not, and the measurement is why.** Across all nine rungs the
silhouette stays within 1.38 px of the axis and the umbra within 2.48 px, and the sign of the
umbra's offset *flips* between the two probe columns (+1.47 px at probe 40, −1.56 px at probe 56,
same rung). The reason is the dither: the per-pixel rotation advances 111° per row down a column and 200° per
pixel across one (`360° · 52.9829 · 0.005837`, and the same with 0.06711 for the axis along it), so
an off-centre sample set is spun in a different direction in every
pixel and its bias has no direction left to accumulate toward. The umbra, being the region where
every pixel is dark for every rotation, is set by the worst case and so comes out symmetric. The
arithmetic bias is real; its consequence is per-pixel quadrature error, not a shifted edge, and I
should not have quoted pixels for the latter.

**What the prefix does cost, and this one is visible.** The umbra half-width by rung at probe 56,
taps 2 through 9: 18.00, 16.98, 13.50, **18.46**, 16.99, 12.00, 12.00, 11.50. More rays sampling the
same disk can only find rays that escape where a coarser set saw none, so the umbra may shrink and
must not widen — and going from four rays to five *grows it by 4.96 px*, 37 %. The cause is prefix
semantics: the fourth step leaves the grid and starts a new row, so rung 5 surveys a wider patch of
the disk than rung 4 does. To a user it reads as a light whose core shadow gets bigger when they ask
for a better estimate. `more_rays_must_not_widen_the_umbra_and_today_five_of_them_do` pins that step
instead of asserting it away, so rebalancing the tables trips the test rather than passing it quietly.

**What this changes in the plan.** The lattice rebalance is still worth doing — monotonic umbra across
rungs is a property worth having, and unlike the "shifted edge" story it is measured. It is not worth
doing for the reason I gave on 2026-10-24, and it still changes output at rungs 1, 3, 5, 6, 7 and 8,
so it goes in only alongside this file, which now says by how much it changes it. `taps: 1` was a
hard shadow that wandered by up to a penumbra radius, never a soft one — asserted here on
2026-10-26, and superseded three sections later by §29, which moved the first tap onto the disk's
centre so that one ray means the light's own point.

Re-measure with `cargo test --lib the_penumbra_profiles -- --ignored --nocapture`, which prints the
axis, the rim comparison, the umbra width and the light eaten for all nine rungs at both probes;
`the_penumbra_dumps_its_columns` prints the profiles themselves. Every tolerance in the file came
from that output, and none of them should be edited without reading it.

---

## 29. The tap ladder, rebuilt so the umbra can only shrink (2026-10-26, Apple M4 Max)

§28 closed with a defect it had measured but not mended: the tap count walked a
*prefix* of a row-major 3×3 grid, so the fifth rung held no sample on one side of
the disk at all and the core shadow it resolved was 4.96 px — 37 % — wider than
the fourth rung's. This section is the mending, and the argument that decided the
new order.

**The property to build for is not centring, it is nesting.** For a given pixel,
the set of disk points whose shadow ray is blocked is *convex*: the slab test
narrows an interval as it goes, so an origin blocked on two counts blocks every
origin between them. Give that, if rung N+1's sample points are a superset of rung
N's, a pixel dark at N+1 rays was already dark at N — the umbra can shrink and
cannot grow, at every pixel, by construction rather than on average. Centring is
worth having too, but it is the secondary object, and it cannot always be had: a
nested chain of even length drawn from a 3×3 lattice ends on an unmatched point, so
some rung must be off-centre. The old order optimised the wrong thing and got both
wrong.

**The new order**, one table, `SHADOW_TAPS` in `shaders/lighting.wgsl`, walked
first-N as before:

| rung | adds | mean offset (radii) | reach |
|---|---|---|---|
| 1 | centre | 0.000 balanced | 0 |
| 2 | −x | 0.500 | 1 |
| 3 | +x | 0.000 balanced | 1 |
| 4 | −y | 0.250 | 1 |
| 5 | +y | 0.000 balanced | 1 |
| 6 | corner (+x,+y) | 0.236 | √2 |
| 7 | corner (−x,−y) | 0.000 balanced | √2 |
| 8 | corner (+x,−y) | 0.177 | √2 |
| 9 | corner (−x,+y) | 0.000 balanced | √2 |

Every prefix is a superset of the last, so the umbra is monotone by the convexity
argument. The worst mean offset is 0.500 at rung 2 where the old table's worst was
1.000 at rung 1 and 0.632 at rung 5. The `SHADOW_TAPS4` cross is gone: one ladder
replaces both tables, which retires the `four` flag from `tap_interval` and
`occluded_taps` and with it the branch that chose between them.

**One tap is now the point light, exactly.** The ladder starts at the disk's centre,
so `taps: 1` fires the ray a penumbra-less light fires. Its measured silhouette is
13.50 px half-width at probe 40 and 20.50 px at probe 56 — the same numbers the
point light measured in §28, to the quarter pixel, which is what `one_ray_therefore_no_penumbra_at_all`
now asserts. Before this it was a rim point spun to a random angle per pixel: a hard
shadow that wandered by up to a penumbra radius, which was never a sensible meaning
for "one ray" and is now forbidden by a test.

**The umbra ladder, measured before and after** (half-width, px):

| probe | rung 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |
|---|---|---|---|---|---|---|---|---|---|
| 40, row-major | 15.50 | 11.50 | 10.98 | 10.00 | **12.50** | 11.00 | 9.55 | 9.55 | 8.50 |
| 40, ladder | 13.50 | 13.49 | 11.50 | 10.98 | 10.00 | 9.47 | 9.00 | 9.00 | 8.50 |
| 56, row-major | 27.00 | 18.00 | 16.98 | 13.50 | **18.46** | 16.99 | 12.00 | 12.00 | 11.50 |
| 56, ladder | 20.50 | 20.49 | 18.00 | 16.98 | 13.50 | 13.53 | 12.97 | 12.46 | 11.50 |

Both new rows fall monotonically. The +0.03 px between rungs 5 and 6 at probe 56 is
the crossing interpolation stepping across a quantization level, so the test bound is
a third of a pixel rather than zero. The old row's rise at rung 5 — the artefact this
whole section exists to remove — is gone at both probes. `more_rays_must_not_widen_the_umbra_and_today_five_of_them_do`
became `more_rays_never_widen_the_umbra`: the pinned step is deleted and the property
asserted at every rung, which is the swap §28 promised in advance.

**The reach follows the ladder now, rung by rung.** The whole-disk reject inflates an
occluder by the farthest a tap's origin can sit from the centre; that used to be one
constant per table. It is 0 at one tap — where the reject becomes the exact centre-ray
test that existed before disks — one radius through rung 5, and √2 above. This is a
soundness number, not a tuning one: understating it lets the reject discard an occluder
a ray does touch. *Cost, unmeasured:* two rungs changed reach. Rung 1 went from the
cross's 1 to nothing, and rung 5 from the grid's √2 to one radius, so on both the
reject inflates its occluder less and should discard more casters before the walk —
cheapest on the row that asks for five rays over many occluders. Rungs 2 to 4 and 6 to
9 pay exactly what they paid before. That is a prediction about a mechanism, and per §27
it needs alternating passes across the two builds before it is a number; it gets a
commit of its own rather than being asserted here.

**Silhouettes moved with the reach**, which is the arithmetic noticing: rung 5's outer
edge at probe 56 was 29.12 px from the axis when rung 5 surveyed a √2 corner, and is
27.50 px now that it does not — against the rim formula's 27.86, closer than before.
Rung 9 is unchanged at 29.99 against 30.87, as it must be.

**The pictures regenerate from the table now.** `python3 tools/penumbra-taps.py`
reads `SHADOW_TAPS` out of the shader and redraws both diagrams in `docs/`, so §28's
claim that the pictures and the tables cannot drift apart is a property of the tool
rather than a hope: it takes the prefixes the shader walks, marks each rung's mean
offset, and frames the rungs that average on the light's centre in green. The rotated
sheet draws each rung's mean at the five angles a column passes through — a balanced
rung shows one dot at the centre, an off-centre one a fan, which is the picture of why
a bias spun every 111° cannot add up across an edge.

**What a scene sees.** Pixels change at every rung but 5 and 9: soft edges shift by
sub-pixel to a couple of pixels, `taps: 1` stops wandering, and the core shadow stops
growing when the tap count rises. Nothing else in the renderer moved — this is one
`const` array, one `select`, and the deletion of a second table.
