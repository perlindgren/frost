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

## winit power preference

`RequestAdapterOptions::default()` is `LowPower`. On this box that
means the Raphael iGPU, with the discrete card idle, while the 4K
display hangs off the iGPU. Correct for the current display setup;
revisit if a monitor moves to the discrete card.
