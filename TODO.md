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
