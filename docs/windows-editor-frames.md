# Why a rack with two editors open stopped answering its own windows

> Historical measurements from the Windows branches. The baseline integration
> replaces the private-message frame pump and process-environment mutation;
> see [the migration decisions and test procedure](rack-baseview-migration.md).
> These measurements have not been repeated for that implementation.


> **Status.** Written 2026-11-24, after a night spent measuring the Windows
> rack under the stress protocol it failed: all editors open, some minimized,
> windows moved, parameters edited. Two causes, both measured before and after,
> both fixed. The numbers are from one machine (Windows, NVIDIA GeForce
> RTX 4090) and every one of them comes from a probe in §5 that can be run
> again. Nothing here was inferred from a code read alone; where a code read
> came first, the sentence says so.

## 1. What was reported, and what it was not

Three complaints, in the operator's words:

* "moving a window is super jerky, editors might be really unresponsive,
  though it seems that the UI updates are typically fine";
* "with just two editors open, moving the rack is super laggy";
* "opening the brain lag window crashed the whole application" — *Nice Brain
  Lag Multi* is the display name of `nice-echo`, so this was one of our own
  editors opening.

The last one is worth being exact about, because it sent the hunt looking for
an access violation. The process had **not** crashed. Taken while it was
frozen: 77 threads, `main` `Running` at about one core, the rack window and all
four editor windows still `vis=True`, no Windows Error Reporting entry, no
application-error event, no dump, nothing on stderr. A window that exists and
is visible and never again answers a message is not a crash; it is a thread
with more to do than it can do, which is what both causes below produce.

"UI updates are fine but input is terrible" is the signature of the same thing:
whatever the loop gets round to drawing happens, and whatever is waiting in the
message queue behind it — a move, a click, a knob — is what feels broken.

## 2. Cause one: one thread, three graphics APIs

A plugin GUI is a child window of a rack editor window, and its frames are
drawn on the **rack's UI thread** — the plugin's `on_frame` runs inside the
same `pump_app_events` call that moves the rack's own windows. There is no
plugin UI thread to blame. (The only extra GUI-side thread in the process is
the frame pump of §3.)

`nice-plugin` asked wgpu for its editor adapter with
`PowerPreference::LowPower`. On a machine with an iGPU that means an editor
gets a different device than the window it is embedded in, and usually a
different *API*: the rack drew on Vulkan/RTX 4090 while each editor drew on GL
or D3D12. Every frame of every editor is then a device-context switch in the
middle of one thread's loop.

Measured, two editors open, one pass of the host's UI loop:

| adapters in play | one pass |
| --- | --- |
| as picked (Vulkan + GL/D3D12) | **261 ms** |
| all Vulkan | 1.78 ms |
| all OpenGL | 1.08 ms |
| all D3D12 | 0.11 ms |

The fix is to make the question the same for everybody, and to make the answer
one word for the whole process:

* [`frost::pin_gpu_backend`](../src/lib.rs#L1027) asks for the adapter the host
  would pick, and writes its backend into `WGPU_BACKEND` before any window or
  plugin exists. Every wgpu instance in the process — ours, a plugin's, one a
  foreign plugin makes — honours that variable, so the whole process draws on
  one API whatever order things were loaded in. An operator who sets
  `WGPU_BACKEND` themselves wins: the pin declines and their list stands.
* [`nice-rack-standalone/src/main.rs`](../nice-rack/crates/standalone/src/main.rs#L50)
  calls it first thing, before `nice_rack::run`, because `run` scans plugins and
  a scan would otherwise get its say first.
* [`nice-plugin/src/window.rs`](../nice-plugin/src/window.rs#L287) asks with
  `compatible_surface: Some(&surface)` and no power preference: the same
  question the host asked, so the same answer.

With that, four editors report the pick themselves and agree with the host:
`[editor] 'Nice Wah': drawing on Vulkan, adapter "NVIDIA GeForce RTX 4090"
(DiscreteGpu)` ([`trace_backend`](../nice-plugin/src/window.rs#L264),
`NICE_EDITOR_TRACE=1`).

## 3. Cause two: frame requests that never coalesce

This is the one that made the rack unreadable rather than merely slow, and it
was invisible in every log we had.

frost's UI thread never goes idle, so a plugin's own timer never fires (timer
messages are generated inside `GetMessage` only when the queue is otherwise
empty), and under baseview 0.4.2 neither does its `WM_PAINT`, which is where
drawing now happens: `WM_TIMER` merely invalidates, and only while baseview
believes a redraw is outstanding. A thread of ours therefore drives the whole
chain by hand — `WM_USER + 2` (baseview's request-redraw), `WM_TIMER` with its
private id `0x42691`, then `WM_PAINT` — which is what
[`start_editor_frame_pump`](../nice-clap-host/src/controls.rs#L563-L831)
exists for. Until the fix it posted one tick every 15 ms per editor, forever,
whether or not the last one had been drawn.

A posted message, unlike a timer, never coalesces. So:

* each editor demands 66 frames a second of one thread;
* a frame costs more than 15 ms once it is drawn among other windows;
* every unanswered tick is another message the loop must drain before it may
  wait, and draining is what makes the next frame late.

The demand therefore grows faster than the supply, without bound. Measured with
four editors open and all four drawing on the same API — so this is not cause
one again — the gap between two turns of the host's loop reached
**50 548.9 ms**, in a single pass of 3 619.5 ms. That is the "hang". The loop
was not deadlocked: winit's pump timeout is a wait-timeout, not a deadline, so
it drains until the queue is empty, and the queue was never empty.

The pump now keeps **one** tick outstanding per editor
([`controls.rs#L563-L831`](../nice-clap-host/src/controls.rs#L563-L831)):

* it posts, and waits for the proof the tick was dispatched — one pass of the
  host's UI loop since the post, counted by
  [`editor_passes`](../nice-clap-host/src/lib.rs#L29), which
  [`LiveSession::service`](../nice-clap-host/src/graph/live.rs#L123) advances
  once per call and which the rack calls every tick of its idle hook;
* no second post until that proof, and never sooner than the 15 ms tick period;
* after 100 ms with no proof it posts again, which is also how a host gone
  genuinely idle is woken up;
* a hidden editor, a minimized rack, or a window that has not appeared yet is
  looked at twenty times a second instead of two hundred;
* and the window is proved to belong to this process at the moment of the post,
  by `GetWindowThreadProcessId`, before any of the three messages go. A handle
  is a number, a destroyed window's number goes back into circulation, and the
  private request-redraw is a `WM_USER` offset rather than a reservation —
  refused in that case, because every editor this pump can see was created in
  this process, so a disagreement means a recycled number and nothing else.

One request outstanding per editor means the queue holds at most three messages
per editor — the count of a tick — instead of as many as the calendar allows.
The loop can always empty it, and
the aggregate frame rate becomes whatever the thread actually completes rather
than whatever four plugins were promised.

## 4. Measured after

`many_plugin_editors_do_not_drown_the_window_thread`
([`nice-rack/src/editors.rs#L943`](../nice-rack/src/editors.rs#L943)) opens one
editor per plugin, pumps six seconds to let them come up, then measures three
seconds of steady state, counting the gap between consecutive opportunities to
answer a message. Budget 33 ms at p95; a gap of two seconds or more is treated
as the runaway signature and stops the run.

| editors, one API | gap p50 | gap p95 | gap max | loop iterations in 3 s |
| --- | --- | --- | --- | --- |
| 2 | 0.00 ms | 0.00 ms | 53.2 ms | 689 617 |
| 4 | 0.00 ms | 0.00 ms | 45.9 ms | 365 333 |

Against the 50.5 s above. The worst remaining gaps are tens of milliseconds,
i.e. one late frame, not a queue. Both rows were taken on baseview 0.4.2 with
the three-message tick; the run that produced them is §5 verbatim. Re-running
the two-editor row after the process check went in moved the maximum by
0.5 ms — 53.7 ms, both editors to frame 700 again — which is the expected
result of a check that is supposed to change nothing except a case that never
arises in a working rack.

That the bound did not simply switch the editors off is answered from inside
the plugin: [`trace_frame`](../nice-plugin/src/window.rs#L242) prints a
heartbeat every hundred ticks, and the two editors of the traced two-editor run
each walked to frame 700 inside its nine seconds — about 78 frames a second
each, from one thread, with the loop still answering messages at p95 < 1 ms.

The opening phase is reported by the same probe and is still slow: worst gap
1.9 s for two editors, 3.9 s for four, because each plugin builds its whole GUI
on the UI thread — the rack's own heartbeat measured "attached its editor in
876–944 ms" per plugin cold, and 147–166 ms for a warm rack. §6.

The shipped binary agrees with the probe: a fourteen-second run of the rebuilt
`nice-rack.exe` with `FROST_PROFILE` names a file reports

```
=== 182 frames in 2.00 s (90.9 fps) … AutoVsync …, latency 1, Vulkan
```

— one API for the rack's own window, which is what the plugins then match.

## 5. Measuring it yourself

```powershell
$env:TMP = $env:TEMP = "$PWD\target\tmp"
$env:RUSTFLAGS = "-Ctarget-cpu=x86-64-v3 -Ctarget-feature=+crt-static"
cargo xtask bundle nice-echo --release   # one package per call
$t = "$PWD\target\bundled"
$env:NICE_EDITOR_TRACE = "1"
$env:NICE_RACK_TEST_CLAPS = "$t\Nice Echo.clap;$t\Nice Wah.clap;$t\Nice Phase.clap;$t\Nice Dist.clap"
cargo test --release -p nice-rack --lib -- --ignored --test-threads=1 `
  --exact editors::tests::many_plugin_editors_do_not_drown_the_window_thread --nocapture
```

Three environment switches, all off by default:

* `NICE_EDITOR_TRACE=1` — the rack's editor-open heartbeat, each plugin's
  adapter pick, and the plugin's frame heartbeat.
* `FROST_PROFILE=<path>` — frost's two-second frame blocks plus the rack's idle
  hook split (service / capture / tick / event), in one file.
* `WGPU_BACKEND=<list>` — the escape hatch; setting it declines the pin.

Useful to know when reading a run: release binaries are mute
(`release_max_level_off`, see [benchmarking.md](benchmarking.md)), which is why
these three speak with `eprintln!` instead of `log`.

## 6. What is still slow

Said plainly, so the next reader does not mistake these for the bug:

1. **A redraw that is not needed.** frost's UI loop repaints about 117 times a
   second with nothing on screen — a live idle rack measured 117.0 fps and
   120 478 draws in one two-second report window, 11.2 s of CPU in fourteen
   seconds of wall time, about three quarters of a core — and it is the reason
   the plugin's own timer starves and the pump of §3 is needed at all.
   Damage-driven redraw would remove both the cost and the pump. The redraw
   gating in [`src/backend/app.rs`](../src/backend/app.rs) trims it and was
   measured not to be the cause of the freeze.
2. **No cap on aggregate editor frames.** Four editors now cost roughly a
   hundred and thirty frames a second in one thread. Bounded, so no longer
   fatal, but the honest limit belongs next to the 15 ms tick period.
3. **~0.9 s per editor to open, serially**, spent in the plugin's first frame:
   its own `wgpu` device and pipeline compile. Sharing one device between the
   host and our own editors is the fix and has not been tried.
4. **The rack has no automated window drag.** Everything above is measured from
   the probe's loop, which is a stand-in for the real rack's. Nobody has moved
   a real rack window under a stopwatch; the probe is the closest thing to one
   and is `#[ignore]`d because it needs a desktop and plugins.
5. **`cargo test -p frost` still has a GPU test that spins** (over a thousand
   seconds of CPU, 273 threads) unrelated to any of this, still undiagnosed.
