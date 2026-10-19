# TODO

A parking lot for engine possibilities that are worth having but not yet
scheduled. Each item states what it is, why it matters, and what standing
work already covers — or fails to cover — it.

## Headless GUI test harness (offscreen render + golden images)

**The possibility.** Run a `Process` — a game, an example, a widget
scenario — with no window at all: adapter → device → render into an
offscreen target → read back → PNG. Feed input as data (pointer
positions, button edges, virtual `dt`), capture frames, and diff them
against committed golden images in CI. The GUI gets tested where it
actually lives: the render path, not just the state machine.

**Why it matters.**

- GPU rendering is currently the one untested layer. UI behavior is well
  pinned by input-level tests (`Ui::begin_input`, `menu_frame` — 7 tests
  for the menu bar alone exercise open/toggle/track/dismiss frame by
  frame, no rendering), but nothing asserts that a frame *looks* right:
  layout drift, an off-by-one marker, a widget painted at the wrong `z`
  all pass today.
- Golden images need reproducibility, and software rendering provides it:
  the same llvmpipe code runs bit-identically on every CI machine and
  every run, while hardware drivers vary by vendor and version. Goldens
  pin *changes* (a refactor that moves a pixel fails loudly); an
  occasional cross-check of one golden on real hardware closes the loop
  with the GPU's slightly different rounding.
- CI needs no GPU: a plain container plus the software Vulkan driver is
  enough. (Verified on the dev box: `vulkan-swrast`'s lavapipe gives
  wgpu a presentable `llvmpipe` Vulkan adapter where the Mesa *GL*
  fallback cannot even create a window surface without `/dev/dri`.)

**What already stands.**

- `Config::render_size` (fixed virtual resolution): an offscreen buffer
  of exactly the given physical pixels at scale 1.0 — the determinism
  contract golden tests want, and a blit path already in the engine.
- `Ui::begin_input` and `menu_frame`: the input-driven half of the UI is
  already a pure state machine, testable with synthesized frames and no
  window; the harness extends the same idea upward, not from scratch.
- Immediate-mode discipline everywhere (declare-every-frame) means a
  captured frame plus the input history fully describes the scenario.

**What to build.**

1. A headless run path: adapter→device without `create_surface` today
   `run_configured` panics at `no compatible surface format` without a
   presentation-capable surface; a `run_headless(process, size, n_frames)`
   (or `Config::headless`) skips the window and renders to the offscreen
   target only.
2. A virtual clock: the harness scripts `dt` per frame instead of the
   wall clock, so animations replay identically forever.
3. Frame readback: `copy_texture_to_buffer` → `Frame` → PNG writer.
4. A golden-image test tool: committed PNGs + tolerance (RMS/lsb-aware)
   diff — never bit-exact promises; float rasterizers drift a lsb even
   under llvmpipe.
5. CI recipe: any container + `mesa-vulkan-drivers` (Ubuntu/Debian
   lavapipe); `VK_ICD_FILENAMES` if the loader needs pointing.

**Lessons from the field** (why the obvious shortcuts fail):

- Screen-poking with synthetic X11 input (XTEST/xdotool) is the weakest
  driver of GUI tests: KDE's Remote Control gate silently blocks it,
  window placement varies, and the compositor gets a vote. Drive the
  app's own input path instead — deterministic and desktop-free.
- The Mesa **GL** backend (llvmpipe via EGL) cannot present window
  surfaces without a DRM render node — it configures as off-screen only
  (`wgpu` then correctly reports no surface formats). The **Vulkan**
  software driver (lavapipe) does not have that limitation, so headless
  paths should prefer Vulkan, and offscreen mode sidesteps WSI entirely.

## Content-gated glow plumbing (emissive plate, bloom chain, HDR frames)

**The possibility.** Make the scene's second color output, the bloom
chain's always-on plumbing, and the float frame itself answer to the
*content* of the frame rather than the capability of the adapter: a
frame in which nothing declared emission and no surface asked for glow
draws with the single-output scene entry point, and the emissive target
is never cleared nor written. The app-level sibling: `hdr_frames`
becomes an opt-out word, and an app that never asks for light above
white renders an unorm8 scene target — half the scene write, half the
present read, no tone fold.

**Why it matters.**

- Measured in the field: walking the baseline on 2026-10-19 caught the
  780M floor halved (2136.9 → ~1002 fps, stamped into
  `benchsuite.baseline`) by the era that landed the float buffer (§7),
  the plate (§8) and the tone-mapped present — about 0.56 ms/frame of
  fixed cost, and the fill floor is the one line with nothing else to
  hide it in. The bandwidth arithmetic says shared-memory iGPUs pay
  most, exactly as §7 and §8 guessed ("a lower-end GPU pays for
  attachments *and* passes"); the M4 Max got away with +0.6 %.
- The bench floor is the theater case, but every menu page, letterbox
  plate and glow-off frame pays the same, and a non-HDR app pays it
  whole. The plate's own comment admits the principle: a target nobody
  writes is a second full-res allocation spent for a look that never
  arrived.
- §8's caveat named this gap outright: *"Skipping the chain when
  nothing emitted would need either a readback or a CPU-side hint
  about what the scene contains, and neither is built."* The CPU-side
  hint is nearly free here, because emission and glow are declared per
  node per frame — the frame builder knows before it encodes anything.

**What already stands.**

- All four scene shaders already carry both entry points — `fs_main`
  returns the color, `fs_emissive` the `SceneOut` pair — so standing
  the plate down needs no new WGSL, only a pipeline variant keyed on
  the frame's emission fact.
- `Bloom::dormant`, the blit's per-level `if (u.bloom_a.x != 0.0)`
  guards and the `u.tone > 0.0` fold already stand down at run time;
  what never stands down is the scene pass itself.
- The gamma substitution and the linear-surface fallback already teach
  the present two shapes of world, so a float-versus-unorm frame
  format is not new plumbing.

**What to build.**

1. One per-frame fact from the draw list (any emission non-zero, any
   glow asked), one pipeline key bit, and the plate's clear skipped on
   frames that need no plate.
2. A policy choice first: per-frame gating (a toggling game gets dark
   frames cheap) versus an app-level opt-out (simpler, covers bench,
   menus and every non-HDR app; misses a game that toggles). Start
   with the opt-out word and let the flag decide later.
3. The honest price, §7/§8 style — interleaved pair of one binary,
   same window — *on the 780M*: every existing measurement of this
   cost was taken on machines with fill to spare, and the whole reason
   to build the gate is that the scarce machine sees more.

## The lighting loop's next multipliers

**The possibility.** Three levers beyond the reach reject, each with a
suite line to price it against:

- **A light prepass.** `light_mix` evaluates per pipeline, per covered
  pixel, per overdraw layer. A compute pass evaluating the field once
  per pixel into a float buffer the scene pipelines add would make the
  cost scale with pixels covered, not pixels times overdraw — paid for
  with one full-res read and write per frame, so it wins only with
  many lights and a deep scene. `lamps-40` is the line that prices it.
- **A strided occluder walk.** `occluded()` chases a block's records
  through their bounds — a dependent-load chain; a fixed-stride
  parallel array breaks the chase. The packer's reach disk already
  made the chains short, so this lever is smaller than it looks;
  re-measure against `occluders-100` before building it.
- **Per-light tile visibility.** Lights breathe in intensity, not in
  position; with static occluders a per-tile visibility answer could
  persist across frames and resolve most pixels' blocks from cache.
  The suite's static-vs-drifting pair exists to show exactly this
  difference.

**Why it matters.** The hill reached its 60 on the reach reject; these
are the levers for the *next* ask — more lamps, soft taps where
`taps-9` still stands as the cliff, or a bigger world — and they are
listed now while the measurements that price them are fresh and the
era's fixed cost is attributed and out of the way.

**What already stands.** The reject and the reach disk (`5dc7ba1`)
reduced the per-pixel cost to *lights that reach* × *occluders that
are near*; every lever here compounds on that shape, and the walked
baseline is the meter.
