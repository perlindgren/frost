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
  610M floor halved (2136.9 → ~1002 fps, stamped into
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
   same window — *on the 610M*, the 2-CU iGPU of the campaign box
   (benchsuite.baseline was walked there; §14 is its re-walk): every
   existing measurement of this cost was taken on machines with fill
   to spare, and the whole reason to build the gate is that the
   scarce machine sees more.

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

## The web port's mosaic: detail decimated between the scene and the page

`docs/benchmarking.md` §16 carries the measurements; this is the open end of them. The web
build compiles, makes its device, clears the loading card and draws a frame whose **detail is
decimated**: a rectangle asked for 256×128 arrives the correct height, twice the width, half
covered, as a square wave of 8 pixels on and 8 off at period 16, with its pixel count
preserved. Flat opaque output through the same pipeline is correct on both platforms, which is
why the breakage reads as "text is torn" rather than "the frame is wrong".

The old entry here blamed the light field's float bit-pun. It is dead, and worth keeping as a
note about method: the theory died because `examples/placement.rs` draws *unlit* rectangles and
shreds them, and because `5d69399` — the last commit before the emissive target existed — shreds
identically in a real browser. Suspects that nothing has questioned are rumours.

- **What is settled, by measurement.** Canvas size (CSS, buffer and style agree, dpr 1); CPU
  geometry (`user_to_pixels` is diagonal with a centred translation; the desktop probe lands
  user (-300,-200) at pixel (211,583) and the browser's *rows* are exact); uploads (texel for
  texel); the blit's filter; bloom and the float frame buffer (`hdr_frames = false` changes not
  one pixel); and the fragment coordinate's source — `@builtin(position)` replaced by a vertex
  computed varying, hardcoded geometry, a hard step for `smoothstep`, an exact viewport quad for
  the oversized triangle, all producing **byte-identical** frames.
- **Where the fault is, and where it is not.** Firefox 157 — rust-wgpu, the very library this engine
  builds on — draws the probe **correctly** on this same machine, GPU and driver; it needs
  `dom.webgpu.enabled` and `gfx.webgpu.ignore-blocklist` in `about:config` on this distro. `wgpu`
  native here is clean as well, which is what §14 is measured on, and no Chromium switch moves the
  frame by a pixel — not the shader disk cache, not GL compositing, not partial raster. **Dawn is the
  fault**, and the reproducer is `examples/placement.rs` with `web/place/index.html`: two boxes, a
  word, ninety lines, nothing else from the engine.
- **The hole to start from.** Four fragment-stage edits produced byte-identical frames, which is
  either proof that the fragment stage is not what reaches the page or proof that three of those four
  builds never carried the edit — only the coordinate probe's string was checked against the shipped
  wasm. Check the artifact's bytes before believing any null result in §16, then hunt the construct
  Dawn mishandles: a flat opaque colour comes through untouched while anything position-dependent is
  decimated at a period of 16, and that difference is the shortest road to a reduced repro.
- **The port has never run here.** §1 to §14 of the benchmarking document are Apple M4 Max; §15
  and §16 are today. `git bisect` from `592e8a1 web dev` (11 October, where the page first
  exists) to here finds no commit that judges clean, so there is no regression to find and no
  earlier good frame to read a fix against.
- **The instruments, so the next hour starts here.** `examples/placement.rs` draws two boxes and
  a word in colours nothing else uses, so a bounding box is a measurement and not an impression
  (`--dump` prints what was asked for). `tools/web-render-judge.sh` builds `minwin` for the
  browser, photographs it headless and scores the 16-pixel signature — 0.41 on the desktop
  frame, 2.10 on the broken one, line at 1.0 — and is built to be run under `git bisect run`.
  `web/min/` and `web/place/` are the pages that serve them; their `pkg/` is generated.
- **Cheap insurance, unrelated to the mosaic now.** The bit-pun is still load-bearing in three
  places (light mask, receiver mask, and the `lit` flag riding an `f32`) and is documented at
  the WGSL end but not at the Rust end, where a reader only sees floats. Whatever else happens
  to the port, *"this integer must survive a float's bit pattern"* belongs above the struct that
  does it — the lesson of this entry is that an undocumented pun is a suspect that never gets
  cleared.

## The host-role test driver (frost plays the guest)

**The possibility.** A test that behaves like a stranger's application rather than like a game:
it asks for the device and the surface itself, constructs the embeddable engine through the
public API, feeds input as data, calls `frame()` on a rhythm of its own choosing, and asserts
what the engine did with all of it. Not a mock of a DAW — a mock of *any* host, which is the
only kind of host this repo can test against, and the only one whose bad behaviour has to be
survivable.

**Why it matters.**

- The embedded path is where frost stops owning the process. It landed with
  three unit tests (`src/backend/tests/embed.rs`) that measure the adapter and
  lay out the pipeline's bind group — enough to catch a wrong limits claim, not
  enough to catch a wrong frame. nice-tuner is a workspace member now and its
  tests run in the gate, but none of them drives a frame: what is still missing
  is a host in this repository that calls `frame()` and reads the consequences,
  which is the thing a plugin cannot contribute because the plugin is the
  stranger being tested against.
- The churn will come from this side. A change that breaks only the guest — a setter that stops
  clearing held keys, resize settling that honours every call instead of the quiet tenth of a
  second, a present mode that starts waiting — fails nothing here today, and surfaces as a
  frozen GUI in someone else's process.
- Being in `tests/` rather than `src/backend/tests/` is the substance, not the location: an
  integration test cannot reach `pub(crate)`, so a green run proves an outsider can drive the
  engine, which is what `pub` promises and what a unit test could quietly fail to establish.
- Panics are the case no other test can make. In a host a panic is the host's crash, and the
  fork's `Embedded::new` *asserts* on the host's device limits — a third party's configuration
  mistake, detected correctly and then reported by unwinding through their event loop. The
  driver's first assertion is that a device requested with downlevel limits yields `Err`, not a
  panic.

**What already stands.**

- `tests/monster_means.rs` and `tests/monster_sprites.rs`: driving the engine from outside the
  crate is already a shape this repo uses, so this extends a pattern rather than opening a
  directory.
- The input state is the seam the port extracts anyway: the `WindowEvent` arms become the
  setters both drivers share, so a fake host feeds the same code the winit loop feeds instead of
  a copy of it that can drift.
- Item 1 of this file is the enabling half of the stronger assertions. *The same scene through
  the guest path produces the same bytes as through the offscreen meter* needs exactly the
  readback machinery the headless harness wants and `docs/benchmarking.md` §38's ceiling check
  wants — one buildable thing, three debts paid.
- `winit` is already a dependency, so the driver can open a window nobody sees and hand its
  surface to the engine as a dev-dependency, costing consumers nothing.

**What to build.**

- A `FakeHost` in `tests/host_role.rs`: instance, adapter, device requested via
  `frost::embedded::device_limits`, a hidden window for the surface, and a clock the test owns,
  with `resize`, `key`, `pointer`, `scroll`, `focus`, `occluded`, `frame`, `advance(Duration)`.
- The assertions that need no pixel, which is most of the contract: `dt` measured rather than
  assumed (drive at 7 ms, then at 90 ms, and check what the `Process` was told); a resize
  settling after the motion stops rather than once per call; losing focus clearing held keys and
  buttons; a zero-sized surface and a lost surface reporting the documented `FrameError`s; an
  exit request standing until the host clears it.
- The pixel assertions behind item 1 and behind the per-chip fingerprint guard (`b33fc3c`),
  because a byte-exact claim is conditioned on the GPU exactly as a frame rate is.
- The constructor failures, each an `Err` carrying a sentence a host can act on and each
  reachable only from outside the crate: downlevel limits, an adapter from a different
  `Instance`, a surface format whose sRGB twin the hardware does not offer.
- **Deliberately not here.** baseview and nice-plug specifics, editor-open latency, a DAW moving
  the panel to another display: those belong in nice-tuner's gate, where the DAW is. The line
  worth holding is one sentence — frost tests that the contract holds when a stranger drives it,
  nice-tuner tests that one particular stranger keeps its end — and the contract itself goes in
  `docs/embedding.md` when the port lands, so both suites cite one text instead of each keeping
  its own guess.

## nice-tuner: migrate native CoreAudio queries to a safe wrapper

The app currently uses our project-owned `nice-tuner/crates/core-audio-device`
wrapper, which isolates the native calls and exposes a safe, read-only API.
Its [safety review](nice-tuner/crates/core-audio-device/SAFETY.md) documents the
contracts and checks. The following remains a future upstream migration:

- [ ] Replace `nice-tuner/src/devices/core_audio.rs` with public safe APIs from
  the `coreaudio` crate once upstream has addressed the safety issues identified
  in the [review of version 0.6.2](nice-tuner/docs/coreaudio-review.md).
  Re-review the fixed release and its regression tests before adopting it;
  do not patch or vendor the dependency locally.

  The review identified:
  - Safe construction of custom property descriptors allows a caller to label
    numeric data as a CFString. `get_custom` trusts that label and can interpret
    the returned number as an invalid pointer. The misuse compiled without
    unsafe code; it was deliberately not executed.
  - Several range/format decoders and encoders use typed pointer reads/writes
    on `Vec<u8>` storage without guaranteeing the required alignment. This is
    a source-level safety defect; no resulting crash was demonstrated.
  - `BufferFrameSizeRange::valid_sizes()` can overflow while doubling a `u32`:
    an artificial boundary case reproduced a debug panic and can loop after
    wrapping with release overflow checks disabled. Its power-of-two buffer
    list and the sample-rate helper's range minima also need care when
    presenting device options.

  Preserve queried input/output channel defaults, current buffer sizes and
  `None` for unsupported properties. Validate the replacement with read-only
  queries on real devices, including missing/disconnected devices. This
  migration replaces our local CoreAudio wrapper only. The standalone
  audio-buffer setup is isolated in `nice-tuner/crates/plugin-buffer`, and
  optional Windows placement in `nice-tuner/crates/window-position`;
  nice-tuner itself denies unsafe code in every feature configuration.

## What joining the workspace left open

Three things the member move deliberately did not finish, each with the shape of
its verification written down so the next person does not mistake it for a detail.

- **`docs/embedding.md`, now that the port has landed.** The paragraph above
  asks for one text both suites cite: what a host owns and what frost owns, the
  two rules (never block the caller's thread, never panic at a bad frame), what
  `surface_lost()` means and what ends it, which units each setter takes and
  why the cursor's differ from winit's, and that `dt` is measured. Today that
  contract exists in module documentation spread over `backend/embed.rs` and in
  a commit message, which is where a contract goes when nobody has to agree to
  it. The `FakeHost` above should be written against this file.
- **The font licence is in the bundle; the archive is somebody else's hands.**
  `nice-tuner/src/gui.rs` embeds `assets/fonts/FiraCode-VariableFont_wght.ttf`,
  which is OFL: embedding is permitted on condition that the licence text travels
  with the font. Closed on the build side — `cargo xtask bundle` writes
  `assets/fonts/OFL.txt` into the VST3 bundle at `Contents/Resources/OFL.txt` and
  lays a copy at `target/bundled/OFL.txt`, and `nice-tuner/scripts/check.sh` fails
  if either is missing or is not the tree's own copy byte for byte. What stays open
  cannot be closed here: a CLAP is deliberately one file, so its text reaches a user
  only if whoever packs the installer takes it along, and no check in this repository
  can see into the artefact someone eventually zips.
- **The plugin now inherits `-C target-feature=+crt-static`.** That flag is in
  `.cargo/config.toml` at the workspace root for the engine's reason — a game
  that ships as one file with no VC redist — and it applies to every target in
  the workspace, including a VST3 that loads into somebody else's process, where
  a second static CRT and heap is generally the wrong shape. `check.ps1` and
  `check.sh` clear `RUSTFLAGS` for the plugin's own build, and the bundle in this
  session was built that way; neither fact has been measured in a host. Load
  `target/bundled/Nice Tuner.vst3` in a DAW both ways and then decide which is
  right, rather than keeping whichever was inherited.
- **Quote one path upstream and a build-time setup disappears.** History, kept
  because the upstream bug is unfixed and this workspace no longer builds the
  crate it stops: `azo` replaced cpal's ASIO host, and the `windows` job of
  `.github/workflows/gate.yml` fails a build whose graph contains `asio-sys`.
  The report itself still belongs in RustAudio/cpal, which is where the crate
  lives. `asio-sys`
  0.4.0 — a member of the cpal workspace, so the report belongs in
  RustAudio/cpal — downloads the ASIO SDK when it cannot find one:
  `Command::new("powershell").arg("-Command").arg(format!("Invoke-WebRequest
  -Uri {URL} -OutFile {}", path.display()))` at `build.rs:303-311`. The path is
  interpolated **unquoted**, so when `%TEMP%` lives under a user name containing
  a space — the majority of Windows machines, including the one measured here —
  PowerShell splits the argument, answers `PositionalParameterNotFound`, and
  asio-sys turns that into `panic!("Failed to download ASIO SDK")` with nothing
  about the space in it. Measured twice on one machine, same folder, two
  spellings: `C:\Users\Per Lindgren\…` fails, `C:\Users\PERLIN~1\…` downloads
  8,910,208 bytes, unpacks 48 files with `asio.h`, and builds in 32.9 s. The fix
  is `{}` → `"{}"`. It is worth a pull request rather than a local workaround
  because the failure looks like a missing SDK, and the person it stops is likely
  to go and configure a path they never needed — which is how
  `.cargo/config.toml` grew a `CPAL_ASIO_DIR` default that quietly disabled the
  fallback for everyone and had to be deleted.

## nice-tuner: own the ASIO host side of the stream

**Where this stands.** Half of it is built, in a different shape. The owner is
`nice-common/src/asio_io.rs` over `azo`, whose `azo-sys` declares the COM
interfaces in Rust — so there is no SDK, no build script of ours, and no
`frost-asio` crate to write; cpal's ASIO host is out of the Windows graph, so
nothing else holds the driver, and the enumeration this section's "already
stands" paragraph describes lives in `nice-common/src/devices.rs` now. One
`createBuffers` carries input and output in one call, and each channel's format
is read from that channel instead of inferred from channel 0.

What is **not** done is the port naming this section opens with: the buffers
still go to the driver's first *n* channels, because `Wanted` carries counts and
`asio_io.rs` maps `0..wanted.inputs` over them. The experiment below (mutating
`channel_num` on a carried cpal stream) no longer applies — the stream is ours,
and the open question is now smaller and harder to answer from a desk: what a
real driver makes of a buffer list that names input 3 and no other. That needs a
multi-input interface and someone feeding input 3.

**The possibility.** nice-tuner opens its ASIO stream itself — one
`ASIOCreateBuffers` carrying input and output together, each buffer bound to the
port the user named — instead of asking cpal for *n* channels and accepting
whichever ports that happens to mean. The shape to design against: a guitar
through a preamp on input 3, tuned with the stream holding that one input port
and no other. The ASIO SDK's own sample host is the reference.

**Why it matters.**

- It is the difference between a tuner that can follow a signal and one that
  cannot. On a six-input driver under ASIO today, inputs 3 to 6 are unreachable:
  cpal's ASIO host binds buffers to `channel_num: 0..n` and nothing in
  `asio-sys` 0.4.0 lets a caller say otherwise, so the panel's channel list runs
  to six while the stream layer accepts one or two.
- It removes a class of restart-shaped bugs rather than one instance. With one
  owner, no other code path can be holding a driver handle whose `Drop` shuts
  the running stream down — the defect now recorded in `BUGS.md`, found while
  chasing the channel-2 silence, and guarded today only by a flag the rest of
  the process has to remember to respect.
- It puts the SDK under our own build script: one place that quotes its paths,
  verifies the headers it needs, and says one sentence when they are absent.

**What already stands.** The settings panel no longer asks cpal anything on the
ASIO path — names come from the registry, and channels, rate and buffer sizes from
one `asio_caps` load per driver per pass, answered by a child process whenever a
stream owns the driver (`nice-tuner/src/devices.rs`). The
output device, buffer and rate rows follow the input under ASIO, because one driver
owns them. The review that maps the terrain, including a public-API experiment that
might deliver most of the feature with no new code at all, is
`nice-tuner/docs/asio-host-review.md`; the upstream quoting fix for the SDK download
is the bullet immediately above.

**What to build.** First the experiment, because it is cheap and it decides the
rest: mutate `channel_num` on a carried `AsioStream` and see whether a real driver
delivers on the port named, which needs a multi-input interface and someone able to
feed input 3. If it holds, depend on nothing new and expose real port selection. If
it does not, the crate: `frost-asio` beside the four existing `nice-tuner/crates`
members, carrying SDK discovery and compilation, driver load, the duplex buffer
setup, the sample-type conversions for whatever format the driver reports, and a
`SAFETY.md` like its neighbours — with cpal's ASIO host compiled out so exactly one
thing owns the driver, and `devices.rs` no longer mapping `"asio"` onto a cpal
`HostId`. In parallel, the small upstream ask: an explicit-port variant of
`prepare_input_stream` and `prepare_output_stream`, so the durable version of this
is a public API rather than our own FFI.
