# Errata

Things measured, not assumed. Each entry: symptom, cause, evidence,
what we do about it. Read before debugging anything in this area.

## KWin 6.7.x — title-bar buttons silently dropped on relayout

**Symptom.** The rightmost decoration button (the close box) is absent
after the button set changes — e.g. a client pins `min==max` size
hints (KWin removes maximize, keeps close), then frees the hints
(maximize returns, close does not). A fresh window that never visits
the fixed state is unaffected. Any window redraw — resize, shade,
`qdbus6 org.kde.KWin /KWin reconfigure` — brings the buttons back.

**Cause.** KWin decoration repaint bug, not client state. Upstream:
[kde#521605](https://bugs.kde.org/show_bug.cgi?id=521605) (close
invisible until shaded/resized) and
[kde#522611](https://bugs.kde.org/show_bug.cgi?id=522611) (*rightmost*
button fails to draw; whichever it is, it disappears).

**Evidence.** `examples/minwin`, key R: native → fixed rung → native.
Hint round trip observed on the wire (winit source, `set_resizable`
wayland path = min==max; release = floor/ceiling cleared); frost's
policy emits nothing about buttons and no Wayland protocol message
exists to set a button set. Same transition, one button lost, inside
the compositor.

**What we do.** Nothing in code — the client's vocabulary cannot
re-request a button. If a band-aid is ever wanted: nudge the size by
one pixel once after freeing the hints (documented redraw trigger).

## winit 0.30 Wayland — configure dedup swallows wishes

**Symptom.** A resize completes visually but the app never receives
the final `Resized` event; engine state (surface size) can lag the
window behind.

**Cause.** winit drops any configure whose size equals the size it
last *wished for* (`new_size != self.inner_size()` gate,
`wayland/window/state.rs`). During an animated KWin resize the
intermediate configures arrive; the final one matches the wish and is
discarded. Also: a request issued inside a `Resized` handler stamps
surface geometry *without* sending a resize request — it can stamp a
fiction. Never request from inside the resize event.

**What we do.** `src/backend/app.rs` compares the window's reported
size against the surface's belief every frame and reconfigures the
surface on disagreement (A/B: free). Aspect correction is deferred
out of the event (`snap_aspect`).

## Measuring on this workstation

**Symptom.** Benchmark numbers from the agent sandbox are absurd and
inconsistent with any real run.

**Cause.** No `/dev/dri` in the sandbox: every adapter is llvmpipe
software rendering. All GPU numbers produced there are fiction.
Real numbers come only from `cargo run --release --example
benchsuite` in the user's terminal (`benchsuite.baseline` is such a
number).

## The float/sRGB frame-buffer test measures the driver's store

**Symptom.** There, `cargo test` fails
`float_and_srgb_frame_buffers_agree_where_the_window_can_tell`: the
stacked washes differ by 3/255 (deterministically, every run) and
1,792 pixels outside the wash differ by one step, where the
calibrated box passes both inside zero and one step. The panic
first names the wash bound; fix that one and the outside bound
fails too, so measure both before arguing about either.

**Cause.** Not the engine. The test stacks twenty 5% washes into an
8-bit sRGB attachment, so every blend step re-encodes linear→sRGB
and rounds, and the precision of that store conversion is the
implementation's own — the spec fixes the curve, not its rounding.
Both bounds are measurements of the calibrated GPU's fixed-function
unit. llvmpipe converts on the CPU: one code off the reference
curve on a single store, three codes accumulated across the
twenty.

**Evidence.** The sandbox, without `/dev/dri`: wgpu answers
`llvmpipe (LLVM 23.1.1, 256 bits)`. Worst inside the wash 3/255 at
(4, 36); outside, the worst is 1/255 — the 1,792 base pixels where
a lone opaque store rounds one code away from the test's own
encode — and every blended region lands identical. The 411
sibling tests pass.

**What we do.** The test asks `adapter.get_info()` which adapter
answered and holds two bounds per kind: a GPU must agree bit for
bit outside the wash and within one step inside; a CPU rasterizer
gets one step outside (measured) and four inside (measured three,
plus margin). The over-range clamp and the "the wash still
exercises rounding" pins stay exact for every adapter. Timing on
llvmpipe remains fiction as above — this whitelists precision,
not speed.

**The probe behind the numbers.**
`backend/tests/srgb_envelope.rs` (ignored; run it by hand like the
rest of the probes) measures the adapter's whole 8-bit sRGB
envelope. On llvmpipe it reports: the sampler-fetch→store round
trip is exact for all 256 codes — the driver's decode and encode
tables are one another's inverse, which is the contract the engine
actually lives on; the unpack table sits up to 0.0012 linear off
the ideal (0.34 of a local code step); the pack curve deviates up
to a rounding-tie distance of 0.125 of a step, which is where its
one-step-per-255 disagreements with the ideal curve, and with the
test's f32 reference, come from. Read those numbers as this
driver's signature: the probe asserts round-trip exactness,
two-step monotone breaks, and exact endpoints — properties no
implementation error excuses — and prints curve distances rather
than pinning them, because the spec bounds the curve's shape, not
whose rounding a driver picks. A future CI that pins exact bytes
is pinning one driver's opinion except at the endpoints.

## winit power preference

`RequestAdapterOptions::default()` is `LowPower`. On this box that
means the Raphael iGPU, with the discrete card idle, while the 4K
display hangs off the iGPU. Correct for the current display setup;
revisit if a monitor moves to the discrete card.
