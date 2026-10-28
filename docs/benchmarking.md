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

*Eleven days later §37 is what this paragraph was always pointing at: on the M4 the
committed file judges nothing at all, and the save path could erase this provenance with
one `--only` run. A baseline now names its chip, a box keeps its own file once it has
written one, and a cross-chip comparison prints a note where it used to print a verdict.*

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
`8b05319a3961a004` identical to the committed build. **§32: nothing in this tree can produce those three
numbers again** — the claim stands as reported, and the instrument that would check the next one is
`src/backend/tests/polygon.rs`. Then the time:

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
`b370c6a09a07e5bf`, `8b05319a3961a004`, byte-identical to the committed build. Not reproducible here;
see §32, which is the same promise made checkable. That was worth
expecting less of than it went: restructuring dot products can change how a compiler fuses
multiply into add, which moves the last bit of a crossing, and the fact that it did not here
is a fact about this driver, not a proof. Fullscreen 4112×2572, two interleaved passes,
median of three, canary 2.6×–4.1×:

| row | committed build (§22) | paired | change |
| --- | --- | --- | --- |
| `fill-1080p` | 2.61 ms | 2.68 ms | +3 % |
| `polys-60` — hexagons, one hard ray | 4.43 ms | 4.44 ms | 0 % |
| `taps-9` — boxes, nine rays | 5.21 ms | 4.86 ms | −7 %, withdrawn by §27, **re-established at −5.6 % by §31** |
| `taps-9-polys` | 14.78 ms | **12.32 ms** | **−17 %** |
| `taps-9-discs` | 30.53 ms | **23.67 ms** | **−22 %** |

The `fill-1080p` row has no casters in it at all, so its +3 % belongs with the ±5 % the nulls
in these runs have been drifting by; everything else moved in the direction the mechanism
predicts and by more than that. The boxes gaining 7 % was not aimed at: pairing halves the
loop's bookkeeping whether or not there are edges to walk, and a box's answer is still the
slab alone.

**That 7 % was withdrawn as a measurement (§27) and later re-established as a smaller one
(§31).** Those two columns were different processes, and the same scenario across five processes on
this machine spans 4.22–4.53 ms, so the spread alone was 7 %: as measured, the row could have been
noise either way, and for the four days between the two sections the honest claim about boxes was that the
paired walk left them unchanged within measurement. Five alternating passes across the two builds settled it — every
paired pass below every unpaired one, a −0.269 ms gap over a 0.002 ms spread — and the effect is
real at **−5.6 %**, not −7 %. The columns had both drifted about 0.35 ms from today's, which is why
the ratio they produced came out a fifth too large. The two shape rows are 17 % and 22 %, both far outside that band, and they stand.

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
−7 % was not.** It was measured across processes, it sat inside the 7 % spread, and it was therefore
*not established*: pairing halves the loop's bookkeeping on a box, which is a mechanism worth
believing, but the number quoted for it that day could have been noise in either direction. It asked
to be re-measured across more alternating passes, where it would either earn its place or lose it, and
**§31 ran them: it earned its place, at −5.6 %.** The rule that struck it stays exactly as written —
and §31 sharpens it, since ten passes back to back agreed to 0.01 ms where five passes across a day
spanned 7 %, so what the rule really protects is closeness in time, not the alternation itself.

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

Every silhouette half-width here is 0.50 px low, and the umbra halves in §29 by 0.02 to 0.18 px: the
meter's lower flank was read one row high on a *step* and less than that on a ramp. Corrected, the
point-light rows are 14.00 measured against 14.00 from the CPU walk at probe 40 and 21.00 against 21.00 at
probe 56, and the two silhouettes are 27.99 and 30.51. §32 has the cause, and retires this table's
`computed` column in favour of asking the collision code what it would shade.
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
axis, the CPU union of rows the tap region can shade, the umbra width and the light eaten for all nine
rungs at both probes;
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
commit of its own rather than being asserted here. §30 priced it and found nothing.

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

---

## 30. The ladder's cost, priced: nothing (2026-10-26, Apple M4 Max)

§29 predicted the new ladder would be a little cheaper at two rungs, because the
whole-disk reject now inflates an occluder by less at rung 1 (reach 1 → 0) and rung 5
(√2 → 1), and a smaller inflation discards more casters before the walk. It is worth
pricing rather than keeping, and pricing it means two builds, because §27 forbids
comparing a number from one process with a number from another.

**Method.** Two binaries — this build, and the same tree with `32fe6bc`'s shader
dropped back in — run alternately, four passes each, fullscreen 4112×2572 at 2.00×,
three rows per pass, median of the row's reps within its own invocation. Weather from
a pass of the same pair: noise floor p50 0.21 / p99 0.22 / worst 0.91 ms, burn p50 56
µs at 1.8× — a cool machine, which is the least the GPU will admit.

| row | reach, old → new | old median (spread) | new median (spread) | delta |
|---|---|---|---|---|
| `taps-1` | 1 → 0 | 3.85 ms (3.83–3.86) | 3.85 ms (3.82–3.87) | +0.007 ms (+0.2 %) |
| `taps-4` | 1 → 1, control | 4.24 ms (4.24–4.26) | 4.21 ms (4.21–4.22) | −0.037 ms (−0.9 %) |
| `taps-9` | √2 → √2, control | 4.92 ms (4.81–5.11) | 4.75 ms (4.63–5.23) | −0.177 ms (−3.6 %) |

**The prediction did not survive.** Rung 1, the rung whose reach went to nothing, moved
by seven microseconds on a 3.85 ms frame, with the two sides' four passes interleaved
and overlapping end to end. There is no there there.

**The interesting row is the one that was supposed to be a control.** `taps-4` kept its
reach at one radius, so nothing in the prediction applies to it, yet all four new passes
came in below all four old ones — the only complete separation in the experiment, which
is a stronger shape than §27's single cross-process comparison. The mechanism available
to it is composition rather than reach: the old fourth rung was the cross, four rim
points and no centre, and the new one is the centre plus three axis points, and the
centre tap is the one most likely to be deep in shadow. `occluded_taps` stops when every
unresolved ray has found a blocker, so a rung that blocks a ray sooner walks fewer
records. That is a story that fits, and 0.037 ms is exactly the size §27 measured as the
agreement *within* one invocation, so it is not a number. It is a hypothesis with a row
worth re-running if this path ever gets hot.

**`taps-9` is noise wearing a result's clothes.** −3.6 %, against one side's four passes
spanning 4.63 to 5.23 ms — 13 % — and both sides sagging through the passes in the same
order, which is the machine warming rather than either build being slower. A row whose
within-side spread is four times the between-side difference cannot carry a claim, and
this one carries none.

**And the rung with the bigger reach change is unmeasured entirely.** The suite has
`taps-1`, `taps-4` and `taps-9`; there is no rung-5 row, so the √2 → 1 change that
motivated the prediction never ran. If the reject's reach ever needs to be argued about,
the row to add is five rays over the 60-caster cliff, not another pass at these three.

**What this leaves the ladder standing on: geometry, not money.** The umbra is monotone
in the tap count, one ray means the light's own point, and the sample mean is off centre
by half what it was. On cost the honest sentence is that nothing moved that measurement
could see — which is also the third piece of evidence that at 60 box casters the reject
is already discarding nearly everything there is to discard, so shrinking what it
inflates by has little left to win. §20 found the walk dominates when there is a walk to
be had; this finds that on a box cliff there is hardly one left.

---

## 31. §23's box row, settled: it is real, and it is −5.6 %, not −7 % (2026-10-28, Apple M4 Max)

§23 measured the paired walk's effect on a box scene at −7 % and §27 took the number back, because its
two columns came from two processes and the same untouched row spans 4.22–4.53 ms across five processes.
What was left standing was the mechanism — pairing halves the loop's bookkeeping whether or not there are
edges to walk — and the honest claim, that the paired walk left boxes unchanged within measurement. §27
ended by saying the row either earns its place on alternating passes or loses it. This section runs those
passes, and the row earns its place.

**Method, the way §27 demands it.** Two binaries, built from the two trees either side of the change:
`f8fcd26`, the build §23 measured as *committed*, whose own commit message closes by calling the pairing "a
claim about a mechanism and not a number until the four rows above say otherwise"; and `8d1402a`, the
pairing itself. Five alternating passes, fullscreen 4112×2572 at 2.00×, `--reps 2`, every median taken
inside its own invocation. Three rows: the disputed box row, the polygon row as calibration, and a fill row
with no casters at all as the null.

| row | unpaired | paired | change | passes |
|---|---|---|---|---|
| `fill-1080p` — null | 1.222 ms (1.22–1.54) | 1.229 ms (1.22–1.24) | +0.008 ms (+0.6 %) | overlap |
| `taps-9` — boxes, nine rays | 4.798 ms (4.80–4.80) | 4.529 ms (4.53–4.53) | **−0.269 ms (−5.6 %)** | separated |
| `taps-9-polys` — calibration | 14.728 ms | 12.315 ms | **−2.412 ms (−16.4 %)** | separated |

**The box row is established.** Every one of the five paired passes came in below every one of the five
unpaired ones, and the within-side spread was 0.002 ms — the unpaired row read 4.798 ms five times over. A
−0.269 ms gap with a 0.002 ms spread underneath it is not noise, and the direction is the one the mechanism
predicts. §23's claim comes back, with its number corrected.

**Calibration, which is what makes the verdict worth reading.** The polygon row measured 14.728 → 12.315 ms
against §23's 14.78 → 12.32 — the paired side lands five thousandths of a millisecond from a figure taken
four days earlier on a different build of the whole rest of the tree. If this method could not reproduce the
effect everyone already believed in, nothing in the row above would mean anything. The null does what a null
should: +0.6 %, distributions overlapping, with one cold first sample at 1.54 ms inside an otherwise flat
column.

**−7 % was too big, and why.** Today's unpaired build reads 4.798 ms where §23 read 5.21 ms for the same
tree, and the paired side 4.529 where §23 read 4.86 — both columns about 0.35 ms low, moving together, which
is machine state rather than a different build. A ratio measured between two columns that have both shifted
is a ratio of the shift as well as of the change, so the sensible reading is that §23's −7 % contained a
little of its own weather. The number to quote is this one: **−5.6 %, −0.269 ms**, measured the way the
document now requires.

**The finding that matters more than the row.** §27's 7 % was real, and it is not a property of processes or
of rows. Ten runs back to back inside four minutes agree to a hundredth of a millisecond; the same row
across five processes spread over a day spans 7 %; §30's longer run sagged 13 % as the machine warmed, on
both sides equally. So the force of *alternate the passes* is not the alternation, it is that alternating
forces the passes close together in time. A cross-build comparison run in the morning and the evening would
reproduce §27's spread exactly and prove nothing, and the rule in §27 wants reading with that sharpened:
compare inside one invocation, and where two sides cannot share one, keep the passes near enough to each
other that the clock has nothing to contribute.

Unchanged by this: §23's −17 % and −22 % shape rows, which always stood well outside the band; §30's
conclusion that the tap ladder changed nothing measurable in cost; and the rest of §27's ledger, which
stands as written.

---

## 32. The polygon path gets a guard, and the meter turns out to be a pixel out (2026-10-28, Apple M4 Max)

§22 and §23 both rest on the same assurance: *bit-for-bit the same picture*. The same three polygon
scenes — hexagons at nine taps, hexagons at four, sixteen-sided discs at nine — fingerprint
`2c192f9ffdba54a9`, `b370c6a09a07e5bf`, `8b05319a3961a004`, byte-identical to the committed build. Those
sixteen digits are in the prose of this document and nowhere else. `benchsuite` has no hashing, no frame
dump, no checksum path; nothing in the tree computes them, and the thing that did is not in this
repository. So the claim that the two largest restructurings in the shadow path preserved the image
cannot be re-checked by anyone — not by a reader, not by me, and not by whoever writes the next one. That
last is the case that matters: the next one is the angular wedge, aimed squarely at this walk because
nine rays over sixty discs cost 23.67 ms against boxes' 4.86 ms (§23), and an optimisation that quietly
drops a ray is precisely what a fingerprint was supposed to catch.

I went to re-run the fingerprints before filing that as a queue item. They cannot be re-run in either
direction: there is no instrument, and the scenes were benchsuite's fullscreen ones, so even hashing the
test tree's scenes would not speak to them. What can be built is an instrument for what comes next, and
`src/backend/tests/polygon.rs` is it. It points §28's meter — unchanged in its reasoning, generalised in
its plumbing, now taking any `Caster` rather than a box — at casters with edges, and asserts two things.

**Geometry, answered by the engine's own collision code.** The hard shadow must land on the rows
`Convex::occludes` says it lands on; one ray through a disk must be that same hard shadow, because the
ladder starts at the disk's centre; more rays may never widen the umbra, because the ladder is nested
and the blocked set is convex; the outline must stay centred on the axis the point light measures; and
the outline must sit inside what the disk can actually shade. That last one is the interesting shape of
the test. For every row it asks ninety-seven origins spread over the tap region whether their segment
meets the caster, and asserts only where the answer is unanimous: a row no origin can shade must read
fully lit, a row every origin shades must read dark. Both directions are sound for *any* subset of the
region, and nine taps are a subset, so it holds whatever the tap count, the order, and the per-pixel
rotation — which is the assertion §20's per-pixel CPU twin could not make, and the reason this is the
form the oracle takes rather than a second implementation of the shader.

**Fingerprints, pinned in the test itself.** The three scenes above, hashed FNV-1a over 64 bits across
the whole target, lit frame and shadowed frame both, so a change to the lighting that leaves the shadow
alone still moves the number. `3cd0d60c58d79dce`, `33a2a3a4bbf090c5`, `c7e013957ba77c16`, taken today
with the geometry tests all passing. They are a fact about this driver, exactly as §23 said its own were,
and they are *not* the old numbers' successors: the scenes are this file's — a hexagon of radius 4 on the
same post, so that the widest shadow still fits inside the 64-px readback — where §22's were benchsuite's
across a 4112×2572 frame. Nothing here retroactively verifies the two withdrawn-looking rows above it.
What it does is make the next such claim checkable, and make re-pinning a deliberate act:
`cargo test --lib the_polygon_fingerprints -- --ignored --nocapture` prints the numbers *and* the
outlines, umbra widths and light eaten beside them, so that the reading happens before the constants move.

**The guard found a bug in the meter on its first day.** The collision walk puts the hexagon's hard
shadow on rows 24 through 39; `silhouette()` reported the lower edge at 38.5 when the profile's own step
is between rows 39 and 40. `first_drop` interpolated the crossing as `i − 1 + frac` in both directions,
and that is right walking *down* a column, where the bracketing rows are *i − 1* and *i*, and one row low
walking *up*, where they are *i* and *i + 1*. Every lower flank this meter ever measured sat one row high
on the shadow. The size of the error is `2 − 2·frac` rows, where `frac` is where the crossing lands
between the two bracketing samples, and that is why it does not land evenly: on a *step* the crossing sits
midway and the error is a full row, and on a *shallow ramp* the crossing sits near the brighter sample and
the error nearly vanishes. So the two things this meter measured are corrected by different amounts.
Silhouette edges are steps at half light: the axis is at 31.50, not 31.00, and every silhouette half-width
is 0.50 px larger — the point light's shadow is 21.00 px either side at probe 56 with the CPU walk putting
it at 21.00, against §28's 20.50 measured against 20.57 computed, and 14.00 against 14.00 at probe 40; the
two silhouettes §28 compared against its rim arithmetic, 27.50 at four taps and 29.99 at nine, are 27.99
and 30.51. Umbra edges are ramps at 0.02 light, and the correction there is small and grows with the rung
as the penumbra deepens: §29's ladder row at probe 56, 20.50, 20.49, 18.00, 16.98, 13.50, 13.53, 12.97,
12.46, 11.50, becomes 20.52, 20.53, 18.06, 17.06, 13.60, 13.59, 13.10, 12.62, 11.68 — from +0.02 px at one
ray to +0.18 at nine, monotone before and monotone after, which is what §29 was actually claiming. Because
the two flanks shift by *different* amounts, the axis-relative offsets move as well, by up to about a
pixel: the umbra's offset is now at most 0.50 px at probe 40 and 1.96 px at probe 56, inside both files'
bounds. `darkness` is untouched, being a sum over rows rather than a crossing. The engine is not a
different engine than it was. The ruler was, and it was a ruler I trusted.

**Two claims die with the ruler, and one of them cheerfully.** The `rim()` formula — a half-width of
(half + reach·(1 − u)) / u — is deleted, along with the test that compared to it. It treated a tap's reach
as a purely lateral offset, and that is wrong: a tap displaced *along* the axis, nearer the caster,
subtends the caster at a steeper angle and casts a wider shadow, which at nine taps, where the corners
sit √2 out, is worth about a pixel. The formula was ~0.5 px low and the meter was ~0.5 px low in the same
direction, and that is the entire reason they agreed to a quarter pixel through §28 and §29: correcting
the ruler broke an agreement that had been holding two errors up, and the oracle — which models nothing
and simply asks — showed the shadow sitting exactly where the geometry puts it. The bound is the oracle's
now, per row, for both shapes. The other claim is the softer one, in both files: *the light a caster eats
cannot depend on the tap count*, by Fubini, because each ray sweeps a shadow as wide as the point light's,
only displaced. On a box the whole spread is a quarter row and the claim looked like a law. On a
sixteen-sided disc it is 16.00 rows at one ray and 17.50 at six — 16.00, 16.50, 16.66, 17.24, 17.20,
17.50, 17.14, 16.88, 17.11 — and the reason is in the premise again: *only displaced* is a parallel-
projection sentence, magnification varies across a disk under this one, and nine taps at three radii are
not a quadrature of a disk. Both files now say jitter where they said law, and point at the row oracle as
the instrument with no premise of that kind to fail.

**Whether it bites was tested, because a guard that does not bite is decoration.** Two perturbations to
`shaders/lighting.wgsl`, each reverted. First the under-blocker, which is the wedge's shape: polygon taps
4 through 8 discarded, so the walk keeps the centre and three axis taps and drops every corner — what an
angular reject leaves on a pixel whose disk is spun one particular way, and enough to break the ladder's
nesting at rung 6. It failed four tests: the umbra widened between rungs 5 and 6 (a rung may not grow the
dark core when every prefix is a superset of the last), the oracle's must-be-dark rows lit up (`row 29
reads 0.20 of its light although every origin in the tap region is behind the caster`), the light eaten
moved further than the rungs explain, and the fingerprints moved. It left alone the hard shadow, the
one-ray-is-the-point-light identity, and the outline bounds, all correctly: none of those is a statement
about rays that were never walked. Second the opposite failure: polygons occluding as their bounds box, edges
never consulted — an over-blocker. It failed the oracle per row (`row 23 reads 0.00 of its light
although no origin in the tap region reaches it`), the outline's containment in the CPU union, the
one-ray identity, and the fingerprints — and it *passed* the CPU hard-shadow comparison, because a
hexagon's widest chord and its bounding box cast the same shadow at the axis row. That is the whole
argument for the oracle in one line: checking the shadow against the collision code at one ray is not a
guard on the other eight.

**What is now safe to attempt.** The angular wedge has an oracle, a monotone umbra, and three pinned
frames. If it changes the picture, some test here says which property moved and by how many rows, and
re-pinning is a sentence in a commit message rather than a sixteen-digit number nobody can produce again.
*(§33 priced that attempt and found the wedge inert — not too weak to bother with, incapable in principle
of rejecting anything the slab in front of the walk has not already rejected. The guard did its job here
too: it is what proved the point, by failing to change a byte.)*
Also corrected in passing: this file's `silhouette` said "the column is all shadow" when its walk found
no dark row at all, which sent one debugging session the wrong way through a scene whose reference frame
was the thing actually at fault — the reference render was carrying the caster, because a painted box is
skipped when the frame is unshadowed and a declared polygon was not.

---

## 33. The wedge is priced, and the pricing kills it (2026-10-28, Apple M4 Max)

§32 ended by calling the angular wedge safe to attempt. That was worth doing in the other order — price
it first, §19 is the lesson — and the pricing says it is not safe so much as empty, for a reason that is
not a matter of degree.

**What the walk is worth, measured by deleting it.** Side B is a build that is wrong on purpose: polygons
occlude as their bounding box, so `clip_planes` is never reached for a shape. Its output is not to be
trusted a second — §32's own over-blocker test says exactly which rows it falsifies — but its *time* is
the ceiling of anything that only avoids plane walks. Five alternating passes, side A unmodified, canary
rows in the same invocation each time:

| row | walk runs | walk removed | delta |
|---|---|---|---|
| `taps-9-polys` — hexagons, nine rays | 11.848 ms | 4.890 ms | **−6.96 ms, −59 %** |
| `taps-9-discs` — sixteen-sided, nine rays | 23.419 ms | 5.033 ms | **−18.39 ms, −78 %** |
| `taps-9` — boxes, nine rays *(cannot be affected)* | 4.417 ms | 4.333 ms | −1.9 % |
| `polys-60` — hexagons, one ray | 3.761 ms | 3.772 ms | +0.3 % |

Spread inside a side was under 1 % for the first four passes and rising in the fifth; the canary says how
much of the −1.9 % is the machine rather than the code, which is to say most of it. Two things fall out.
The deltas are **6.96 ms for six edges and 18.39 ms for sixteen, a ratio of 2.64 against the shapes'
2.67** — the walk costs the plane count and nothing else, to within one per cent. And with the walk gone a
sixteen-sided disc costs what a hexagon costs, 5.03 against 4.89, so the whole 11.57 ms that separates the
two rows is the number of edges a *round* shape is charged for. `polys-60` did not move at all: at one ray
the walk is not where the time is.

**The wedge, written.** An angular reject for polygons, in the shape's own space: how far the shape's
centre stands from the *line* each ray runs along, past the bounds' circumscribed radius and the ray cannot
meet the shape. Per record that is a cross product and two lengths; the tap's own contribution comes
through the triangle inequality as the single scalar |w|, and the ladder has only three values of that —
nothing, one radius, a corner's √2 — so a record decides two booleans and a tap costs one comparison
against a walk of four flops an edge, sixteen edges, twice over for a pair.

It changed **no pixel in any of the three guarded scenes** — all three fingerprints byte-identical — and
it was 2 to 3 % slower on the polygon rows. That is not a loose bound. It is an inert one, and here is the
proof, which is short enough that it should have been done before any of the code:

> The plane walk runs only if some tap's *segment* cuts the record's bounding box, because that is what
> `a_open` and `b_open` mean. Every point of that box lies within |half| of its centre — the radius the test
> uses — so such a segment meets the circle, so its line meets the circle, so the test keeps that tap. And
> if no tap's segment cuts the box, the walk does not run whatever the test concludes.

Two cases, identical behaviour in both. And it generalises past my particular algebra: an angular test can
only reject by putting the shape *outside* a cone from the ray, which is the same as putting the ray outside
the shape's enclosing circle, and the bounding box already contains that circle. The slab sitting in front
of the walk therefore dominates every rejection an angular test could deliver. §32 named this the biggest
remaining lever on the polygon path. **It is not a lever; it is not smaller than one; it cannot be one.**
Withdrawn, and the shader is back as it was.

**Where the money the pricing actually found has gone.** The walk is not being asked to reject things it
should have rejected — it is being *asked*, a great many times, by shapes that genuinely are in the way. So
the levers left are about how much an answer costs and who pays it:

* **A circle should not be sixteen planes.** *(§34 built this one and it held: −75.7 % on the disc row, and
  the disc/hexagon cost ratio went from 2.01 to 0.45. Read that section for the tax that came with it.)*
  `DISC_SIDES` is `MAX_PLANES`, so every drawn circle in the
  occluder field is a sixteen-sided polygon and pays 2.67 times a hexagon to be round, which on this suite
  is 11.57 ms of the disc row. A record that says *circle* — centre, radius, ray-versus-circle interval, no
  plane list at all — is aimed at the largest single number in this section. Predicted, not measured: the
  walk's cost is measured to be linear in edge count, and a circle is the edge count going to zero, but the
  interval test has its own cost and the guard would have to grow a circle caster before the claim means
  anything.
* **A tighter volume could pay, but not for these shapes.** An oriented box per polygon is cheaper than the
  walk it short-circuits and, unlike a cone, tighter than the axis-aligned bounds — which is the only thing
  an extra volume can be to earn its place. It would win on a rotated plank or a fence rail and do nothing
  for a hexagon, and benchsuite contains no rotated thin shape at all: the suite cannot currently see the
  lever, so the honest next step there is a row, not an optimisation.
* **The drift in these runs is the whole reason the numbers above are deltas.** Side A's own polygon row
  went 11.85 → 12.02 → 13.11 ms across three passes two minutes apart, and the box canary with it, 4.42 →
  4.91 ms, 11 %. §31's sharpening again, seen from inside: the levels are not comparable across passes, and
  only the paired rows within a pass are.

## 34. The circle record: §33's prediction, paid for, and the tax it comes with (2026-10-28, Apple M4 Max)

§33 left one lever standing and labelled it *predicted, not measured*: a circle should not be sixteen
planes. It is built now, guarded, and measured, and it is worth what §33's linearity said it would be —
minus about three quarters of the row. It also came with two things I had not predicted: a cost on scenes
with no circles in them, and a bug that every test in the suite let through.

**What a circle record is.** `Convex::disc`'s result now remembers the radius it was cut from
([`Convex::circle`](../src/collision.rs)), the occluder packer writes a negative edge count instead of
sixteen planes, and both occlusion walks answer the ray with one quadratic. The marker is `-1` in the slot
that holds the edge count because that is a count no polygon can have, which leaves the record's stride the
box's three vec4s, and because the radius was already sitting in the half-extents the record carries.
Nothing new is stored; a disc's record went from nineteen vec4s to three, and the cull and the cheap
whole-disk reject answer for it exactly as they did for a box, since the bounds are the square around it.
What the GPU does per ray changed from sixteen edge clips to two dot products, a multiply and a square
root — and, this being a quadratic and not an approximation of one, from a shape 1.9 % inside the circle
the author drew to the circle.

**What it is worth.** Four alternating passes, side A unmodified, side D the circle record, canary rows in
the same invocation every time. Medians, with the paired delta taken inside each pass:

| row | unmodified | circle record | median paired delta |
|---|---|---|---|
| `taps-9-discs` | 34.07 ms | 8.08 ms | **−25.94 ms (−76.1 %)**, range −26.7 … −18.6 |
| `taps-9-polys` | 17.23 ms | 17.79 ms | +0.00 … +1.25 ms |
| `taps-9` | 6.43 ms | 7.13 ms | +0.16 … +1.25 ms |
| `polys-60` | 5.46 ms | 5.98 ms | +0.02 … +0.83 ms |

The number I would show a stranger is the ratio, because a ratio taken inside one side cannot be touched by
the drift this machine has been showing. On side A a disc row costs **1.98 times** the hexagon row. On side
D it costs **0.45 times** it. The ordering of the two shapes inverted, and the only thing that changed
between the sides is how a circle is written down. Within side D, sixty discs sweeping nine rays per pixel
now cost about a millisecond more than sixty boxes do — which is roughly what a circle should cost, since a
box is two slab tests and a circle is one quadratic.

*Two arithmetic slips in this table, corrected here rather than in the pass notes where they came from. The
paired deltas this section prints further down are −25.56, −26.73, −26.32 and −18.59, whose median is −25.94
and not the −25.56 first written in the row above: what was quoted there was the smallest-magnitude pass of
the three that agreed, which flattered nothing and understated the effect by 0.38 ms, and against the side-A
median of 34.07 it is −76.1 %, not −75.7 %. The ratio is 34.07 / 17.23 = 1.98, not 2.01. Both were found by
recomputing the section's own arithmetic, which is the cheapest audit there is and the one nobody schedules.*

Side B from §33 — the build that skipped the walk but kept the planes — is not in this table on purpose. It
was measured on a machine running this suite one and a half to two times faster than it runs today, and
levels do not cross sessions. What can be said about B without a number is that D beats it in kind: B still
streamed sixteen planes per disc record and declined to clip against them; D has no planes in the frame at
all, which is also why the row stopped moving between passes (D's disc row: 8.22, 8.11, 8.04, 6.35 ms; A's:
33.78, 34.84, 34.36, 24.94).

**The tax, because it is there.** The control rows contain no circles: their field bytes are identical on
both sides, so any difference in them is the added code existing — a `rec.z < 0.0` and a `select` per record
per pixel in each walk, and a tenth argument and an early return in `tap_interval`. It measures real:
`polys-60` came out positive in **eleven of eleven** paired readings across two runs, `taps-9` in nine of
ten, and the medians are +3.8 % and +2.5 %. Those are the *conservative* medians — the first run measured
side A first in every pass, which is an artefact waiting to happen, so the second run flipped the order per
pass. The tax survived the flip (A-first +5.45 %, D-first +3.84 % on `polys-60`); the flip accounts for a
point or so, not for the effect. Mechanism not isolated: the build that would isolate it is this shader with
the packer forbidden from ever emitting a circle, and until someone runs it the sentence above is a
candidate list, not an explanation.

**What the guard says the picture did.** Not "nothing", which is what I was going to write. Before-state
taken from a worktree at HEAD, after-state from the working tree, same ignored printer:

| scene | before | after |
|---|---|---|
| hexagon, nine rays | `3cd0d60c58d79dce`, umbra 3.13 / 2.68, darkness 16.23 / 24.67 | byte-identical |
| hexagon, four rays | `33a2a3a4bbf090c5`, umbra 4.08 / 6.08, darkness 16.24 / 24.24 | byte-identical |
| disc, outline | 17.51..44.51 and 7.51..53.51 | identical at both probes |
| disc, umbra half | 3.68 at probe 40, 4.68 at probe 56 | identical at both probes |
| disc, darkness | 17.11 and 25.99 rows | 17.23 and 26.22 rows |
| disc, fingerprint | `c7e013957ba77c16` | `c0760736e3128d33` |

So neither hexagon moved at all, which is the control, and the disc's shadow kept its outline and its umbra
and removed 0.12 more rows of light near the caster and 0.23 more far from it — +0.70 % and +0.89 %. That is
the right size for the change: the sixteen-sided coin holds 97.4 % of the disc's area, and only the part of
that missing 2.6 % which lies inside the penumbra band the probe column crosses can have been doing
anything. The fingerprint moved because a hash over every byte of two frames notices a tenth of a row; that
is the instrument working, not the picture changing underneath it.

**The correction I nearly shipped.** I had the disc's old umbra at 2.68 rows and its old darkness at 24.67.
Those are the *hexagon's* numbers, two lines above the disc's in the same printer output; I read down the
wrong row and the sentence was already written. The real before-state existed only in a build that was no
longer in the tree, so the fix was to measure it: a worktree at HEAD, its own target directory, the printer
run there. Worth recording as method, not just as a slip — when a claim needs a before-state, the
before-state is a thing you can still go and build, and reading it out of your own scrollback is how the
neighbouring line gets promoted to a fact.

**The bug the change brought in.** The circle branch went in beside the box's slab tests, and the slabs went
behind a guard that reads perfectly sensibly — only walk them if the ray has a direction at all — which is
wrong in exactly one case: a segment of no length. That case is the pixel a light stands on, and once the
slabs' parallel branches stop running, the interval stays the whole of [0, 1], the "cuts through the
bounds" test passes, and *a box anywhere in the scene shadows the pixel under its own light*. All 446 tests
passed with it in. Fixed by restoring the slabs verbatim behind a plain `else`, which is what the guard now
says out loud in a comment.

`wherever_a_light_lands_in_a_column_that_column_keeps_its_light` owns it: the lamp is swept across a pixel
and the column it stands in must keep its light, on both interval roads (the box's slabs, the circle's
quadratic) and both ends of the tap ladder. It fails on the broken build — *the box at 1 rays dims the
column of a light set 0.5 by 0.5 off its pixel to 0* — and passes on this one. Two things it also settled:
the degenerate landing is at half-integer coordinates, so that is the pixel-centre convention, written down
somewhere at last; and the first version of the test did not work. I had put the lamp on an integer, which
is a texel's corner, so the ray still had a direction and both the broken and the fixed build passed it. A
test that has not been seen to fail is a decoration.

**Collision, since the question came up mid-flight.** Unchanged, and unchanged in kind: `Collider` has had an
analytic `Circle` since it existed, and `Convex` is still documented as the occluder's shape rather than a
physics body. What changed is a divergence worth knowing about. An occluding disc's *shadow* is now the true
circle, while an author's collision outline built from `Convex::disc` is still the 16-gon, so the two answers
about one object can be up to 1.9 % of the radius apart on the flats. The right way round, on the grounds
that the shadow has to match the thing being drawn and the physics code was already living with the polygon —
but a difference all the same, and three doc comments that asserted the old arrangement have been rewritten:
§33's "the shadow is the only consumer that cannot have it", bake's "tessellated here and nowhere earlier",
and `MAX_PLANES`'s roundness argument, which now governs only shapes that genuinely have edges.

**Withdrawn one section later, and by itself.** §35 deleted `Convex::disc`, so there is no authored 16-gon
standing in for a circle anywhere in the engine and no divergence to describe: a round body and its shadow
are answers about one shape again. The paragraph stays because the reasoning in it — that the shadow must
match the thing being drawn — is what made the deletion worth doing, and because "the two answers can be
1.9 % apart" was true and measured for the span it was written.

**What is now safe to attempt.**

* Price the tax with the isolation build above. If it is the per-record parse, it wants hoisting out of the
  per-record loop, and that is a change whose cost can be measured on these same control rows.
* The rotated thin shape row, before any oriented-box-per-record idea (§33): the suite still cannot see that
  lever at all.
* The box scene's per-row oracle test (§32). The polygon scenes have the strong per-row check; the box scene
  has outline containment, and §34's degenerate-ray bug is precisely the class of thing containment misses.
* A circle *is* now the cheap shape, which is worth saying to anyone tempted to "optimise" a drawn circle
  into a polygon somewhere upstream: that would make it 2.2 times more expensive in the field, and wrong.

## 35. The tessellation deleted, and the price of an approximation once someone can trace one (2026-10-28, Apple M4 Max)

§34 bought the circle record by way of a polygon that remembered being a circle: `Convex::disc` cut a disc
into `DISC_SIDES` edges, `Convex::circle` handed the radius back to the packer, and the packer wrote three
vec4s instead of nineteen. That worked, and it left behind a shape with two answers — the shadow got a
circle, anyone who built a collision outline the same way got a 16-gon — plus a generator whose only honest
remaining customers were a bench row, a test fixture and the editor's amber outline. §35 is the smaller
version: the generator goes, the circle becomes a kind of occluder rather than a memory a polygon carries,
and the one thing worth keeping out of the generator — the bench row — is rebuilt by hand so the plane walk
does not lose its worst case.

**What changed, in the API and in the type.** `Convex::disc`, `Convex::circle` and `DISC_SIDES` are gone:
three public items out, `Canvas::occluder_circle` and `Canvas::occluder_circle_layers` in. A declared
occluder now holds `OccluderShape::{Box, Polygon, Circle(f32)}` instead of `polygon: Option<Convex>`, so
"this is a circle" is a fact about the occluder rather than a note attached to a list of edges, and the
packer's `match` replaced a `(center, shape)` tuple that existed only to route the memory past the polygon.
`MAX_PLANES` keeps its two-percent roundness argument, because that claim is about polygons someone traced
and is still true; the test for it now builds its ring from points instead of asking for a disc, which is
what the claim always ought to have rested on. The desk keeps a ring too — `OUTLINE_SIDES = 32` in
`sprite_util`, purely for stroking, since a line primitive has no opinion about curvature — and its comment
says that instead of the old sentence about showing the edge the shadow will have, which is no longer true
of any ring.

**What was worth measuring, and is.** One binary, both shapes in it, eight passes, run order flipped every
pass, `--reps 3`, 4112 × 2572 fullscreen, sixty occluders and ten lamps at penumbra 6 in every row. Both
round rows cover 21600 px² per occluder — the circle by `πr²`, the polygon by `½nR²sin(2π/n)` — so the rows
differ in what the field has to know about a shape and in nothing else. `taps-9-discs` is the declared
circle, `taps-9-poly16` is new and is a traced 16-gon: the plane walk at its cap, which is the row §34's
change quietly removed from the suite when a circle stopped being a polygon.

**Withdrawn the next day, by §36, and what went was the row rather than the arithmetic.** `taps-9-discs`
declared boxes. Its scenario sets `poly_sides: 0`, because a circle has no sides, and the builder tested the
box branch before it ever looked at the `circle` flag — so the circle row and the box row were one and the
same scene, and the delta of −0.00 ms below is a control that was read as a result. The `taps-9-poly16` row
is real and its numbers are its own, but every comparison in this table that says "the circle" is a
comparison against boxes. Two other claims in the same paragraph go with it: the order was not flipped every
pass, because `select()` sorted a named list back into suite order, so `--only a,b` and `--only b,a` were
the same command; and `polys-60` below has been sixty hexagons, not boxes, since §21. What this section
measured is what a traced polygon costs against a box, and on that question it still stands. What it says
about circles was never measured, and §36 measures it.*

| row | the casters, nine rays | median ms | paired vs `taps-9` | paired vs the circle |
|---|---|---|---|---|
| `polys-60` | sixty hexagons, one ray | 3.97 | −0.62 | — |
| `taps-9` | boxes, nine rays | 4.59 | — | — |
| `taps-9-discs` | sixty circles | 4.59 | −0.00 (1/8 positive, at zero) | — |
| `taps-9-polys` | sixty traced hexagons | 11.97 | +7.38 (+161 %) | +7.38 |
| `taps-9-poly16` | sixty traced 16-gons | 23.36 | +18.77 (+409 %) | +18.78 (+409 %, 8/8) |

Every row repeated to within 0.02 ms across all eight passes, which is the quietest this suite has been
all day, so the deltas below are not a machine-state story.

* **~~A circle costs a scene nothing over a box.~~** *Withdrawn by §36: both rows were boxes. Measured
  properly, one binary, eight paired passes, a declared circle costs **+0.77 ms over a box at nine taps,
  +16 %**, in all eight passes, spread 0.00 ms — which is the same order §34 read across builds and about
  what a quadratic per tap against a box's whole-disk reject should cost.*
* **The polygon walk still costs the plane count and nothing else.** `(23.36 − 4.59) / (11.97 − 4.59) =
  2.54` against the shapes' own `16/6 = 2.67` — five per cent under, the same direction and a little further
  than §33's 2.64-against-2.67. Two sections and one deleted abstraction later, the law held.
* **Tracing a round shape by hand instead of declaring it costs 80.4 % of the row**, or 5.09× the other way
  round. *The 80.4 % and the 5.09× were taken against a box wearing a circle's name. Against the circle the
  row names, §36: the traced 16-gon is **4.30× the circle**, +18.03 ms on a 5.47 ms row, and the +409 %
  above is correctly read as 16-gons over **boxes**. The direction never moved; the size does by a seventh.*
  That is the number §34's closing bullet gestured at with a 2.2, which was six edges' worth: with the cap in
  play the gap is twice as wide again.
* One asymmetry, against the number rather than for it: the 16-gon's bounds square is 2.6 % larger than the
  circle's (28 224 px² against 27 502, `4r²` with `r = 82.92`), so marginally more of its taps survive the reject and reach the
  walk. The polygon is paying for a slightly bigger box as well as for its edges, and the honest split
  between those two is not in this table.

**§34's −75.7 % and this −80.4 % are not the same measurement.** That one was a generated disc beside its
own circle, and it settled whether the record was worth building. This one is what an author chooses
between — a traced round outline against a declared circle — which is why it had to be taken after the
generator was gone rather than inferred from the row before. The directions agree, and the older number is
untouched.

**The guard got stronger for free, which is the part no timing row shows.**
`a_disc_shadows_the_gpu_the_way_the_cpu_says_it_should` used to need two comparisons and a piece of geometry
argument to be honest: the GPU against the CPU's own 16-gon exactly, *and* the 16-gon's shadow sandwiched
between the shadows of the circles it inscribes and is inscribed in, with a band of disagreeing pixels
counted and upper-bounded because the tessellation was expected to differ from the round shape and "by how
little" was the only fair question. With one shape on both sides the test is a sentence: the shader agrees
with `Collider::Circle` exactly, everywhere the shadow edge is not crossing between neighbouring pixels.
The band, the sandwich and the containment argument are gone, and what replaced them is stricter.

*Annotated two days later, by the section that found the sentence was only true of one of the two roads a
light can take: §36. The test above runs at penumbra 0 and walks the single ray, where the claim always held.
At any penumbra above 0 the picture disagreed with `Collider::Circle` over four pockets, and did so inside
the frame this section pinned.*

**A fixture retyped from memory is a bug, and here is the receipt.** Writing that test I typed the scene's
two `Draw` literals again instead of taking them from the file, and got `lit: 0.0` on the receiver instead of
`1.0`. The surface then drew its own colour, lit or not, so the frame had no shadow in it at all; the test
reported 678 pixels where the CPU said shade and the picture said 255, which reads exactly like the shader
having lost the circle. It had not: `git show HEAD:…` and splice, and the same assertions passed untouched.
Two things to keep — a suite that fails because a fixture lost a field is doing its job, and a diff that
rewrites an unrelated literal is the thing to look at first, not the thing you assume about the change.

**What is now safe to attempt.**

* The tax, still unpriced and now cheaper to isolate: the circle row sits level with the box row, so the
  isolation build (§34's "shader parses, circles never emitted") can only be about the parse.
* The box scene's per-row oracle test, unchanged from §32 and §34's lists, and now the only scene in the
  suite without the strong check.
* `taps-9-poly16` is a row worth keeping an eye on for a different reason than cost: it is the only place
  the suite still measures a polygon at `MAX_PLANES`, so any change to the walk's shape — an oriented box
  per record, a coarser reject — shows up there first and shows up large.


## 36. The soft shadow and the hard shadow disagreed about a circle, in the four corners of a square (2026-10-28, Apple M4 Max)

**This one came out of reading, not out of a frame.** No timing row moved, no canary coughed, and nothing on
screen was reported wrong. During a sweep of the repo for places where the code and its description had
drifted apart, the two shadow walks in `lighting.wgsl` went side by side on screen and said different things
about the same question.

**The question is where the pixel is standing.** An occluder does not shadow its own surface — a pixel on or
inside a shape is lit by a light on the other side of it, which is why a wall does not cast a shadow across
its own bricks. Both walks open with the cheap form of that test, `all(abs(lp - center) <= half)`, and `half`
is a *half-extent*: for a box it is the shape, and for a circle it is half the square drawn around it. Each
walk then refines the answer for a shape that is not its own bounds. The polygon's refinement consults its
planes. §34 gave the single-ray walk a refinement for a circle too, the exact one:

```wgsl
if (round) { let to = lp - center; inside = dot(to, to) <= half.x * half.x; }
```

The fused nine-tap walk never got it. Its refinement runs `if (inside && edges > 0u)`, and a circle record
carries no planes, so on that road the answer for a circle stayed *inside the square*.

**The pocket.** Square minus disc is `(4 − π) r²`, about a fifth of the square, all of it at the diagonals —
four pockets, since a circle's bounds square has corners and a circle does not. A receiver pixel in a pocket
is outside the disc, so the disc can shadow it, and the single-ray walk shadows it. The nine-tap walk told it
that it was standing on the occluder and handed it its direct light back. It is not a softness effect and not
a light-shape effect: the receiver can be a floor, a sprite or a lit particle, and all the light does is pick
which walk runs. `lc.w > 0.0` takes the fused road, so a disc's shadow changes discontinuously as a penumbra
is dragged from `0` to `0.25`.

**Measured, in pixels rather than milliseconds.** One process, one 80×80 offscreen scene, lamp at `[8, 8.5]`,
a caster at user `[6, −10]` turned 0.4 rad, the rung forced to nine through the feather slot. The column that
matters is how many pixels are more than 64 levels apart between the frame at penumbra 0 and the frame at
penumbra 0.25 — with a disk a quarter of a pixel across, the two frames are nine rays from the same point
against one ray, and a pixel they resolve differently is the walk answering differently.

| caster | radius | as shipped | with the fix |
|---|---|---|---|
| circle | 14 | 95 px | 10 px |
| circle | 40 | 380 px | 0 px |
| hexagon | 14 | 11 px | 11 px |
| hexagon | 40 | 0 px | 0 px |

The hexagon is the control and it is why the table is readable: its containment is exact on both roads
because its planes are consulted, so the 11 pixels it shows at radius 14 are the umbra's own edge crossing
between neighbouring samples. The circle's 10 after the fix are that same edge, which is parity. Before the
fix the circle's 95 and 380 were all in pockets, at distances 14.1 to 16.0 from a centre whose radius was 14.

**The number the shipped guard reports** is sharper, because it counts only pixels the hard ray actually
shadows: 49 pocket pixels were lit at radius 14 and 100 at radius 40, where the hexagon lit 2 and 1. The
guard, `both_roads_shadow_the_pocket_between_a_disc_and_its_bounds`, asserts a circle gets no more slack than
a hexagon of the same radius measured in the same invocation, so it needs no threshold borrowed from a
machine, and it refuses to pass on an empty region — it requires more than 50 pocket pixels in the frame and
more than a quarter of them in shadow.

**Why nothing had seen it, in three parts.** Every profile in `penumbra.rs` and `polygon.rs` reads one column
of pixels along the lamp's axis, and a pocket is on a diagonal. The disc's exactness test runs at penumbra 0,
which is the walk that was right. And the fingerprint moved once already, legitimately, in §34 — so when this
change moved it again the reader of that section has already been shown a disc pin moving for a reason, which
is exactly the situation a pinned frame is worst at: it receipts whatever the renderer did, including being
wrong in four corners that no probe reads.

**What it costs, measured the next hour.** The branch was hoisted first, so the circle row declares what its
name says — `taps-9-discs: 60 occluders declared as circles`, which the suite now prints per row, because the
row names in a table are prose and that line is the fixture. Then two runs, `--reps 3`, 4112 × 2572
fullscreen, canary in every invocation.

One binary, this build, eight passes, the four rows asked in one order on the odd passes and the reverse on
the even ones — an order the suite now honours, having previously sorted a named list back into table order,
which is why §35's "flipped every pass" flipped nothing:

| row | the casters, nine rays | median ms | paired Δ vs `taps-9` | spread |
|---|---|---|---|---|
| `lamps-10` | no casters | 1.36 | −3.33 | 0.01 ms |
| `taps-9` | sixty boxes | 4.70 | — | 0.00 ms |
| `taps-9-discs` | **sixty circles** | 5.47 | **+0.77 ms (+16 %)** | 0.00 ms |
| `taps-9-poly16` | sixty traced 16-gons | 23.50 | +18.80 ms (+400 %) | 0.06 ms |

All eight passes paired and every one of them agreeing, which is why the §35 numbers that this replaces were
not a machine-state story either — they were a fixture story. Three readings:

* **A declared circle costs +0.77 ms over a box at nine taps, +16 %.** Not nothing, and not a mystery: a box
  answers a tap with two slabs and can be dropped whole by the disk reject, while a circle answers every tap
  that reaches it with a quadratic and a square root. §34 read "about a millisecond more" across two builds;
  §34 was right about the size and this is the same quantity measured inside one.
* **Tracing the same area by hand is 4.30× the circle** — 23.50 against 5.47, +18.03 ms — where §35 printed
  5.09× against what was actually a box. The +400 % in the table above is the 16-gon over boxes and should be
  quoted as that.
* The one-ray polygon law of §33 and §35, `(poly16 − box) / (hexagon − box) ≈ 2.54` against `16/6 = 2.67`,
  uses the box and hexagon rows and is untouched by any of this.

Two builds, six pairs, alternating which goes first, the before-side a worktree at `f901c80` with this same
bench compiled against it, so the only difference between the columns is §36's three lines in the walk:

| row | with the fix | without | paired Δ | per pair |
|---|---|---|---|---|
| `lamps-10` | 1.37 | 1.37 | −0.002 ms | six at ±0.01 |
| `taps-9` | 4.70 | 4.59 | **+0.106 ms (+2.3 %)** | +0.10 … +0.11, 6/6 |
| `taps-9-discs` | 5.47 | 5.49 | **−0.015 ms** | −0.02, 6/6 |

So the fix is free on the row it acts on — 0.015 ms is under the 0.03 ms that the reported fps can resolve at
183 frames — and it is **not** free on a scene of boxes, where it costs 0.106 ms in all six pairs and in both
orders. That was not predicted, and the prediction it replaces said "nothing", so the miss is the finding:
for a box the new `round` branch never executes its body, the arithmetic is untouched, and a single extra
test per record still moved a 4.6 ms row by 2.3 % repeatably. The plausible mechanism is not the compare —
it is the fused walk's register budget, which the extra live `to` vector raises for every record and not just
the round ones. That is a guess about a mechanism and is written as one; the number needs no mechanism to be
true. It also lands inside the +2.5…4 % circle-free tax §34 could not price, which makes that tax's follow-up
more interesting than it was this morning: the walk is apparently paying for the *shape of its own code*, and
the first thing to try is spelling the same rule so a box's path is textually what it was — `round ? disc
test : (bounds && planes)` — and re-running these six pairs.

**The reorder was tried, and it answered a different question than the one it was aimed at.** Six passes,
three binaries, rotating which goes first: `A` the shipped spelling, `B` a worktree at `f901c80`, `C` the same
rule written so a box's path is textually what it was — `if (inside && edges > 0u)` first, `else if (round)`
after. Semantics identical, a circle having no planes to consult: the pocket guard, both fingerprints and the
two disc tests pass on `C` with no edit.

| row | `A` shipped | `B` no fix | `C` reordered | A−B | C−B | C−A |
|---|---|---|---|---|---|---|
| `lamps-10` | 1.36 | 1.37 | 1.37 | −0.002 | −0.001 | +0.002 |
| `taps-9` | 4.70 | 4.59 | 4.67 | +0.108 | +0.085 | −0.022 |
| `taps-9-discs` | 5.47 | 5.49 | 5.60 | −0.015 | **+0.111** | +0.126 |
| `taps-9-clear` | 1.42 | 1.42 | 1.41 | +0.002 | −0.003 | −0.003 |

* **The arm order is not the tax.** A box pays +0.085 ms in the reordered spelling against +0.108 in the
  shipped one — same order of size, six pairs out of six, in both starting orders. Rearranging the tests did
  not buy the box path back, so `C` does not ship.
* **The reorder costs circles 0.111 ms, and that says something for the shipped one.** Written
  `bounds → planes → round`, the bounds test feeds the first condition and cannot be removed, so a circle
  pays the square *and* the disc. Written the shipped way the `round` arm overwrites `inside` before the
  bounds value is ever read, and the compiler drops the redundant test — which is why the shipped circle row
  is indistinguishable from the unfixed one at −0.015 ms. The dot implies the bounds test for a circle; only
  one of the two spellings lets the compiler see it.
* **`taps-9-clear` carries no tax in any build** (+0.002, −0.003). Nine rays over an empty block visit no
  records, and there the cost is not. The charge is levied per record visited — not per pixel, and not per
  shape reached.

**The mechanism, corrected, including what this section got wrong.** The paragraph above claimed one added
operation could not account for 2.3 % of a frame because a record's body runs a hundred-odd operations. That
counted the whole body. Most record *visits* never reach it: the whole-disk reject `continue`s past the tap
loop, and §22 found the walk dominating the cliff precisely because there are so many visits. Sixty casters
at ten lamps bound the visits per pixel at six hundred before the per-light blocks and the reject cut them,
and at 10.6 Mpx one extra op per visit lands at about 0.1 ms on this machine — which is the number measured,
in both spellings, and on the circle row as well the moment a spelling forces the extra test. So the tax is
the arithmetic after all, charged where the walk runs most often rather than where it does most work, and the
register-pressure story this section reached for is not needed.

Which is as far as §34's unpriced tax goes today: a test added per record visit costs on the order of 0.1 ms
at this screen size and record count, no circle has to exist for it to be paid, and that is the same 2–4 %
§34 measured on scenes with nothing round in them. The lever it points at is not cleverness about the circle
case — it is fewer tests per visit, the shape test folded into the reject that already ran rather than a
second gate standing after it.

Canary throughout both phases: `p50 0.34 / p99 0.37 ms`, excess 0.01–0.03, once 0.14, nothing near the 3.5 ms
gate. Two invocations of the second phase carried a `worst` of 17.24 and 11.43 ms — single wakes, with p50
and p99 unmoved and the paired deltas of those two pairs identical to the other four, which is the pattern
the gate is drawn to catch rather than the one it is drawn to stop.

**The invariant to keep.** A new kind in `OccluderShape` has to refine `inside` on *both* roads. One walk is
written as a loop over taps and one as a single ray, they were derived from each other, and nothing in the
build makes them agree — only the pocket test does, and only for as long as a circle is the only round kind.

## 37. The baseline belonged to another GPU, and the save path could not tell you (2026-10-28, Apple M4 Max)

§36 ended with a shape row worth +0.77 ms and an open item saying the committed
baseline judged ten rows of twenty-seven. Going to look at that number was the
mistake, because ten was optimistic.

**Every row on this box was unjudged.** `benchsuite.baseline` is a ten-row file
whose own header says it was walked on 2026-10-19 on a Radeon 610M iGPU, and
`taps-9 16.0` in it is true of that chip. This machine reads the same row at
213.1. Judged against it, every number here is a twelve-to-thirteen-fold
improvement on a build that changed nothing, which is why the documented recipe
says `--baseline none` and why I have been typing it for two days without
asking what it meant: the suite had no ruler for the machine it was running on,
and the way to avoid a wrong verdict was to take none.

**The save path made that unfixable rather than merely unfixed.** `--save-baseline`
built its output from scratch: one generic comment line, then only the rows the
run had measured. So the obvious remedy — walk the missing rows — was the one
thing guaranteed to destroy evidence: `--only taps-9 --save-baseline
benchsuite.baseline` would have left §14's ten-row provenance header and nine of
its rows on the floor, under a line claiming the file was a baseline, with a
13× "improvement" as the reward for the person who next trusted it. This is the
same defect as §36's dead bench row one layer up: an instrument storing less
than the operator believes it stored, and reporting success while doing it.

**What the tool does now.** A baseline is a type that keeps what it finds — the
header verbatim, the rows, and a count of lines it could not represent, and it
refuses with exit 3 to overwrite a file it cannot reproduce. Saving *merges*: a
measured row replaces itself where it stands, an unmeasured one keeps its
number and its place, a new name is appended. A baseline can therefore be walked
in pieces, which is not a convenience — it is what a noisy afternoon and a
26-row suite require. The file to use is now the GPU's own when it has written
one (`benchsuite.baseline.apple-m4-max`), the shared default otherwise, and an
explicit `--baseline` beats both. Whatever gets chosen prints with its own
`# walked` line, and a file that does not name the GPU under it says so:

```
benchsuite: judging against benchsuite.baseline (10 row(s))
benchsuite:   # walked 2026-10-19 on the Radeon 610M iGPU (RADV RAPHAEL_MENDOCINO), reps
benchsuite:   NOTE this GPU is Apple M4 Max and the file above does not name it — a
              cross-chip diff is not a regression; see docs/benchmarking.md §14
```

That note is the whole point; everything else is plumbing in service of it. The
engine had to be able to say which GPU it got, which it previously only wrote to
the log at startup and never exposed — hence [`frost::adapter_name`], recorded
once where the adapter is requested, `None` before that.

**The first full walk failed, and correctly.** All twenty-six rows in one
invocation is five and a half minutes, and the canary called it: burn quanta
p50 97 µs against p99 412 µs, 4.3× — cores rationed, whatever the wake side
said — and the run refused to write its own file. Five and a half minutes of
numbers printed, no ruler, exit 3. That is the gate doing the job, and the fix
is not a looser gate: the same twenty-six rows walked as four chunks of seven
came in at 3.1×, 1.6×, 2.0× and 1.8×, every chunk under the 3.5 limit, each
saving into the same file and each merging with what was already there. A
longer run is a bigger window for someone else's workload, and chunking is what
shrinks the window without shrinking the ruler — and it also means a loud
afternoon now costs one chunk rather than the suite.

**The result agrees with the paired experiment, which is new.** Four rows in the
new file can be checked against §36's within-invocation deltas, measured hours
earlier on binaries built before any of this, and they land inside a
three-hundredth of a millisecond:

| row | this box's baseline | §36's paired run |
|---|---|---|
| `taps-9` | 213.1 fps — 4.693 ms | 4.70 ms |
| `taps-9-discs` | 182.9 fps — 5.468 ms | 5.47 ms |
| `taps-9-poly16` | 42.6 fps — 23.474 ms | 23.50 ms |
| `taps-9-clear` | 703.5 fps — 1.421 ms | 1.42 ms |

That is not proof the baseline is right — a baseline is a ruler, and §26's rule
still stands that only within-invocation pairs are quotable as effects. It is
evidence that the box was behaving, which is the most a single sitting can
establish, and it is the first time this suite has had the two agree at all
because the first time this suite had a ruler for the box holding it.

**Two defects in the new code, found by the new code.** The baseline loaded
every frame instead of once, because a flag was read and never set — six
identical lines in the smoke test before it was visible; and the provenance
probe matched the word `walked` anywhere in a header line, which in the
historical file's wrapped third paragraph matches "this box re-walked them on",
so the tool quoted a footnote about a second walk as the account of the first.
Both are now pinned by tests, including one that a file gains a provenance line
exactly once and never stacks a second claim under the first.

**What is still open, named as such.** Only this chip has a file. The 610M's
stands as §14's and nobody here can re-walk it today; the Radeon 7900 XTX and
the RTX 4090 columns in §18 are still empty, and this is the mechanism by which
they get filled — run the suite there once, in chunks, and the box writes its
own ruler rather than lending someone else's. A CI performance testbed is the
same design at larger scale: a box is a baseline file, so a farm is a directory
of them, and nothing in this needs to change when that arrives. And the row
count stays honest: twenty-six rows, since `warm` is a warmup and not a row.

## 38. The 4090 box wrote its own ruler, and its first number was a fiction (2026-10-28, Windows 11, Ryzen 9 5900X + RTX 4090)

§37 ended by saying how the empty boxes get filled — run the suite there once,
in chunks, and the box writes its own ruler. This is that walk, on the box §17
left open, and it very nearly wrote a lie. It is also the first section of this
document not written on the M4 Max since §16, which is worth saying out loud in a
file that has spent twenty-one sections meaning one machine by "this box".

**The first number this box ever wrote was a fiction.** `taps-9 117.1`, its three
reps 117.2 / 117.0 / 117.1 — agreeing to 0.17 %, which is the figure you want to
see. The noise floor reported 1.4×, quiet, and was entirely correct: the canary
measures wake latency and the duration of a fixed quantum of arithmetic, and a
box can be perfectly silent while every frame it presents is held back by
something else. What held them was the NVIDIA driver's Max Frame Rate, set to
117, outvoting the suite's own `vsync: false`. The row that gave it away was
`fill-1080p`, a near-blank frame that a Radeon 610M renders at 1002.7 fps: it
read **117.2**, three reps identical to the first decimal.

**Repeatability was the camouflage.** The test everyone applies to a suspicious
number — run it three times and see whether it agrees — is the test a clamp
passes best, because a clamp is the most repeatable thing on the machine. Those
reps agreed to 0.17 % against the 1.8 % the same row shows unpaced. What caught
it instead was dividing two rows that share nothing, which the suite had been
printing for years without anyone comparing: `fill-1080p ÷ taps-9` is **62.7×**
on the 610M, **3.81×** on the M4 Max, **1.39×** on this box unpaced, and
**1.0009×** under the limiter. A machine that renders a blank frame and a
nine-ray shadow field at the same rate is not a fast machine; it is a machine
being paced by something that is not frost. Note that the honest ratio on this
box is already down at 1.39, so the gap from the clamp is 39 % and not an order
of magnitude: such a check has to be a check, with a threshold written down, and
not a glance at a number that looks low.

**The whole thing was a checkbox.** Max Frame Rate off, Vertical sync off, and
`fill-1080p` on the unchanged binary went from 117.2 to **3004.7** — twenty-five
fold, rising to 3369.5 again once the window was genuinely on screen — from a
driver setting that no program here could have read, which is what
§17 meant when it said the refresh setting cannot be read from inside a shell
and has to be recorded like any other condition. The conditions of this file are
therefore: windowed 1080p, three reps, a 3840×2160 panel at 60 Hz, frame limiter
off, vertical sync off, **the window actually on screen** and the machine
otherwise idle; the canary stayed between 1.4× and 1.9× on every run taken here,
mine and the terminal's alike. The rows land between 536 and 3369 fps, nine to
fifty-six times the panel rate, which is the evidence that nothing is pacing them
— and the reason the 60 Hz still belongs on the record is that the moment someone
turns sync back on every row in the file reads 60.0, which is the most misleading
number available here because it looks like a dead engine rather than a paced one.

**The ladder, in milliseconds, with the caveat printed beside it.**

| row | RTX 4090, 1080p window | M4 Max, 4112×2572 fullscreen |
| --- | --- | --- |
| `fill-1080p` | 0.297 | 1.232 |
| `lamps-40` | 0.309 | 4.523 |
| `taps-1` | 0.411 | 2.994 |
| `taps-9` | 0.414 | 4.693 |
| `taps-9-discs` | 0.420 | 5.467 |
| `taps-9-polys` | 1.113 | 12.09 |
| `taps-9-poly16` | 1.866 | 23.47 |
| `parts-100k` | 1.127 | 1.404 |
| `parts-huge` | 1.790 | 8.230 |

The Mac rows were walked at 5.1× the pixel count in a different present mode, so
those columns are both measured and neither comparable — §37's defect again, this
time arriving in the display rather than the judgement, which is how one knows
it was never about the file format.

**Nine taps turned out not to cost anything, here.** 2431.4, 2431.9 and 2418.3
fps at one, four and nine taps: 0.411 ms at both ends of the range, and a spread
of 0.5 % between nine taps and one. The Mac falls 334.0 → 263.6 → 213.1 across the
same three rows, a 36 % loss. Subtracting the blank frame, the whole shadow walk
costs this box 0.117 ms and the Mac 3.46 ms; normalising the Mac's 5.1× pixels
leaves it nearly six times dearer per pixel. So "nine taps cost 36 %" was never a
property of the engine — it was a property of two machines, and this one, a third
data point, disagrees with both. It does not make §22 wrong, it makes it a claim
about hardware, which is the thing this section keeps being about. It is also the
one finding here that a worse-conditioned walk and a better one both agree on,
because a cost that is zero relative to its neighbour survives the 40 µs that
moved everything else.

**The cliff is the polygon walk on both vendors, and only its multiplier
survives.** 0.414 → 1.866 ms here, 4.693 → 23.47 ms there: 4.5× and 5.0×, which the
re-take below tightened from 4.2× — the agreement between the two machines got
better when the conditions got honest, which is not what one expects from noise.
The absolute cost is twelve-fold apart, the shape is not, and that is the strongest
argument this document has never quite made for reading its own tables as ratios
first and megahertz second. And §23's other finding reproduces on hardware that was
not in the room when it was written: discs are nearly free (0.420 ms against 0.414)
while the same frame carrying sixty polys instead costs 1.113 ms.

**The row I most suspected is the one that moved most.** The first version of this
section reported `parts-100k` at 1.326 ms against the Mac's 1.404 — a suspicious
agreement between machines twelve-fold apart everywhere else — and guessed that
something in that row was not being paid for by the GPU. Re-taken properly it reads
1.127 ms, the largest single movement in the file at +17.7 %, so the agreement was
an artefact of the worse walk and the claim goes with it. What survives is smaller
and better founded: a hundred thousand particles cost 1.13 ms against a blank
frame's 0.30, and the particle row that barely moved across the two walks is
`parts-huge` at 1.79 ms, which is the one place a machine-independent cost might
still be hiding. The offscreen meter is where to ask it, not a window.

**The window's state was the variable, and it moved the file by thirteen percent.**
What stood here before explained a lone `--only taps-9` run reading 2438.0 against
the 2156.9 then on file as the difference between a row taken by itself and a row
taken beside its neighbours, with clock state named as the suspect. That explanation
was wrong, and the re-take is what proves it: **2438.0 was the good measurement.**
The whole file was walked again from a terminal, window on screen and the machine
otherwise idle, and twenty-two of the twenty-six rows came in 8.7 to 17.7 % faster —
`taps-9` at 2418.3 where the file had said 2156.9. The spot check was right and the
walk was wrong, which is the whole trouble with a condition nobody records: every
one of those runs printed `visible: Some(true)`, and in winit that means only *not
explicitly hidden*.

And what changed is not a clock. Sorted by movement, three rows did not move at all
— `taps-9-polys` 1.120 → 1.113 ms, `parts-huge` 1.807 → 1.790, `parts-10k-cliff-lit`
0.575 → 0.569 — and those are the rows carrying the most GPU work per frame, while
each of the twenty-two that did move differs from the old file by a near-constant
0.03 to 0.065 ms rather than by a ratio. A clock or a power state scales with the
work: `parts-huge` at 1.79 ms would have gained near 0.2 ms where it gained 0.017.
What fits is a fixed cost of roughly 40 µs on a presented frame, on the critical
path only when the frame is short enough for it to be there at all — the signature
of a present path doing different work when nothing is compositing the window, the
same machinery §17 found making a windowed Mac read differently from a fullscreen
one. `taps-9-poly16` (+4.7 %) and `parts-100k` (+17.7 %) move by more than the
constant and are not accounted for: the evidence supports *something fixed per frame
left the measurement*, not *every row is now explained*.

So this file is a ruler for a window that is **on screen**. Run it from anything
whose window ends up behind something else — an agent's shell, a scheduled task, a
runner with no console — and every row fast enough to be present-bound reports up to
13 % of free improvement against a threshold of 5 %, in the direction that reads as
success rather than as trouble. What the tool should do is record `is_minimized()`
and `has_focus()` beside the box size, and refuse to write a baseline from a window
that is not up the way it refuses a loud box; it does neither yet, so for now the
condition lives in this paragraph and in the file's header, which is exactly how
§17's refresh rate had to be carried too.

The effect replicates from the other side now that the file is conditioned
properly: the same build, run minutes later from a shell whose window is not
composited, reads `taps-9` at 2169.0 against the 2418.3 above, and the tool says
**REGRESSED −10.3 %** and exits non-zero. Nothing changed in the engine, nothing
changed in the driver, and a false regression is as damaging as a false pass — the
difference being that this one appears in the conservative direction, which is
precisely why nobody would stop to check whether the GPU had really got slower.

**One hazard the walk fell next to, which §37 half fixed.** §37's title names the
save path, and the save path now merges instead of overwriting — but it merges
into whatever name it is given, and the note saying *this GPU's own file would be
X* prints **after** the write. So on a box whose chip file does not exist yet, a walk that names the
historical default loads §14's ten rows, writes this box's numbers over them,
and then mentions the chip file as a suggestion afterwards. The default stays
that way on purpose — `chip_file`'s own comment argues that quietly moving the
ruler under a run that never asked for one is a worse surprise than a warning —
so the duty sits with whoever walks a new box: name the chip file — this walk's
is `benchsuite.baseline.nvidia-geforce-rtx-4090` — which
does not exist yet and so starts from empty and says so out loud. What the tool
could still do is print which file it merged *from* before the write, rather than
only noting the better target after it.

**What is still open, named as such.** §18's RTX 4090 row is still empty: what
this box has now is frost's ladder, and §18 wants the same ladder built in Godot,
with §14's gate met on the machine doing it. The ceiling check above is described
and not written — the canary says whether a box is quiet, and nothing yet says
whether it is unbounded, which is the difference between this file and the one
this section nearly wrote. A fair cross-machine row needs `--fullscreen
--render-size` matched on both sides per §17, which neither column in that table
has. And the same chip-dependence reached the test tree a day earlier: the
fingerprint guard gained a row per chip in `b33fc3c`, because a byte-exact claim
is conditioned on the GPU exactly as a frame rate is — which is why this box
arrived with a failing test and no guilty commit behind it.
