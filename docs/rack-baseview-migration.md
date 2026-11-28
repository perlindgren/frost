# Rack baseline / Windows rendering integration

This branch merges `baseview-0.4.2` at `f507eba` (including `win-work` at
`48a97c9`) into `codex/rack-baseline` at `7b71d7b`. It includes Nice EQ,
the rack's input-clip acknowledgement fix, text caches, rendering diagnostics,
and the Windows drag/redraw changes. It does not restore process isolation or
GUI-replica experiments. Dependency source and existing dependency versions are
unchanged; the EQ branch's declared FFT dependencies are included.

## Scheduling and safety decisions

- Keep the baseline's Baseview 0.4.2 redraw scheduling on macOS. Its own
  view-bound display link invokes `draw`; do not restore the successor branch's
  second display link. Keep the macOS 14 availability guard before view creation.
- On Windows, retain a helper per editor to request paints when a busy host loop
  starves ordinary paint generation. It uses public `InvalidateRect` and
  `SendMessageTimeoutW(WM_PAINT)` operations, not Baseview's private timer id or
  `WM_USER` offsets. Only direct Baseview children of our editor parent qualify.
  A successful send acknowledges the paint handler's return. The helper waits at
  most 100 ms for a handler; after failure it does not request another paint until
  the outstanding update region has been validated. Closing an editor cancels
  its helper before CLAP destroys the GUI. The UI thread never joins the helper.
- The helper does not move plugin rendering off the owning GUI thread. Audio
  still uses the existing DSP worker and topological graph order.
- Backend selection never mutates the process environment. The rack and Nice
  editors use the same surface-independent adapter query, then explicitly
  configure their own wgpu instances. Separate plugin libraries have separate
  caches; identical choices are a preference, not a guarantee across devices
  or toolkits. `WGPU_BACKEND` supplied by the launcher is honoured explicitly.
  Arbitrary third-party plugins remain responsible for their own graphics API.
- Ordinary Frost applications do not opt into backend pinning automatically.
  The upstream changes to text caching and drag handling do affect them and
  need runtime regression checks too. Frost retains the frame-latency default
  of 2; Nice Rack explicitly selects 1. Embedded plugin surfaces are unchanged.

The measurements in [windows-editor-frames.md](windows-editor-frames.md) describe
its earlier implementation. They are not performance measurements of this
migration's public paint helper or explicit backend selection. Likewise,
[macos-plugin-threading-findings.md](macos-plugin-threading-findings.md) records
the earlier isolation experiment, not new benchmarks of this merged tree.

## Presentation latency: current policy and proposed experiment

`Config::frame_latency` is selectable in application code. It is a desired
maximum number of frames queued for graphics presentation, not an audio buffer
size or a guarantee of a particular input-to-display delay. The per-frame time
budget does not change. Actual buffering, CPU/GPU overlap and acquisition waits
depend on the backend, driver and present mode, including vsync.

The current policy is:

| Window/surface | Requested latency | Who configures it |
| --- | --- | --- |
| Ordinary Frost application, including games | 2 by default | Application's `frost::Config` |
| Nice Rack's main window | 1 explicitly | Rack's `frost::Config` |
| Each Nice plugin editor, hosted or standalone | Existing surface configuration retained | Shared `nice-plugin` renderer |
| Third-party plugin editor | Plugin-defined | The third-party plugin |

Each editor owns its own GPU surface. Setting 1 on the rack does not propagate
to editors, even though their GUI work may run on the same main thread. The
embedded Frost renderer leaves the supplied surface configuration in place;
changing Frost's application default therefore does not change plugin surfaces.

A request of 1 can reduce queued frames and potential input latency, but gives
CPU/GPU work less buffering to absorb timing variations. A request of 2 retains
the previous application default and can provide better overlap or smoother
throughput for demanding games. Neither choice is universally faster. A shallow
queue may cause longer waits when acquiring the next surface texture, which
matters when several editors share a GUI thread. Disabling vsync does not
establish that acquisition or presentation cannot block.

**Deferred experiment:** compare explicit latency requests of 1 and 2 in the
shared `nice-plugin` surface setup. No editor-latency change has been made as
part of this discussion. The comparison should:

1. Keep the rack at 1 and vary only the editor-surface setting. Rebuild and load
   the plugin bundles for each variant; changing only the rack is insufficient.
2. Use the same backend, display refresh, present mode, window sizes, DSP chain
   and build mode. Record the effective surface configuration as well as the
   requested value. Test one, two and four open editors on macOS and Windows.
3. Compare acquisition/presentation waits, frame-time variation, CPU use and
   input responsiveness while idle, dragging/resizing, and minimizing/restoring.
   Rack FPS alone does not establish each editor's FPS or input latency.
4. Run a separate Frost/game comparison at 2 versus an explicit 1 before making
   a recommendation for games. Keep the general default at 2 in the meantime.

Third-party editors cannot be assumed to honour this setting. These experiments
concern presentation only; audio buffer configuration and DSP deadlines remain
independent, though excessive GUI contention can still affect overall behavior.

## Build and test

Rebuild plugin bundles as well as the rack: rebuilding the host alone does not
update the code in previously built `.clap` files. Close the rack/DAW first,
especially on Windows, where loaded DLLs cannot be replaced.

From the repository root, on macOS:

```sh
for plugin in nice-tuner nice-nam nice-dist nice-phase nice-clone nice-echo nice-ecomplex nice-wah nice-pi-face nice-eq; do
    cargo xtask bundle "$plugin" --release || break
done
cargo build -p nice-rack-standalone --release
cargo run -p nice-rack-standalone --release
```

On Windows, use a Visual Studio Developer PowerShell (with MSVC on PATH):

```powershell
$env:RUSTFLAGS = '-Ctarget-cpu=x86-64-v3 -Ctarget-feature=+crt-static'
foreach ($plugin in @('nice-tuner', 'nice-nam', 'nice-dist', 'nice-phase', 'nice-clone', 'nice-echo', 'nice-ecomplex', 'nice-wah', 'nice-pi-face', 'nice-eq')) {
    cargo xtask bundle $plugin --release
    if ($LASTEXITCODE -ne 0) { throw "Bundle failed: $plugin" }
}
cargo build -p nice-rack-standalone --release
cargo run -p nice-rack-standalone --release
```

Bundles are written under `target/bundled`, or `$CARGO_TARGET_DIR/bundled` when
that variable is set. Load those freshly rebuilt CLAP bundles in the rack;
saved rack documents can still reference an older bundle elsewhere.

On either platform:

1. Open two, then four editors, including EQ. Confirm meters/analyser animate,
   parameters affect audio, and the rack and editors remain responsive.
2. Drag/resize rack and editor windows individually. Other editors should keep
   responding. Minimize/restore the rack and individual editors; observe CPU
   usage and confirm redraw resumes.
3. Repeatedly close/reopen an editor, remove its plugin with its editor open,
   and change audio settings. Check for stale windows, hangs, or lost state.
4. Quit with several editors open. Reopen the rack and confirm saved states and
   geometry restore. Test a third-party CLAP editor as well.
5. Check an ordinary Frost animation/game for smooth rendering and input: shared
   rendering changes must not silently trade game responsiveness for rack speed.

For editor heartbeat/backend messages set `NICE_EDITOR_TRACE=1` before launch;
for frame and host-loop timings set `FROST_PROFILE` to a report path. To compare
backends, set `WGPU_BACKEND` before launch (for example `dx12` or `vulkan` on
Windows, `metal` on macOS). Remove it to test automatic selection again.

The existing native multi-editor probe is also available. Set
`NICE_RACK_TEST_CLAPS` to full bundle paths separated by the platform's path-list
separator (`:` on macOS, `;` on Windows), then run:

```sh
cargo test --release -p nice-rack --lib editors::tests::many_plugin_editors_do_not_drown_the_window_thread -- --ignored --exact --test-threads=1 --nocapture
```

This probe requires a desktop. Its measured loop gaps are not editor FPS.

## Validation recorded on this branch

Host: macOS/aarch64, Rust 1.100.0. The agent build uses
`CARGO_TARGET_DIR=target/agent-build` to preserve the existing cache.

- Rack, rack engine, CLAP adapter, shared plugin adapter, and EQ library tests:
  220 passed, 12 explicit desktop/manual tests ignored.
- EQ allocation-test target: 18 passed, one manual report ignored.
- Frost library tests excluding `backend::tests`: 321 passed; the 151 tests in
  that module were excluded, so this is not a GPU-regression verdict.
- Native release rack and EQ standalone builds, and all ten release CLAP/VST3
  bundle builds passed.
- Windows MSVC cross-target check and strict Clippy passed for the rack, shared
  editor, CLAP adapter and EQ, including EQ standalone and Windows placement.
  This checks Rust compilation; it does not link or run Windows executables.
- Frost's WebAssembly check passed, with the existing unused
  `console_error_panic_hook` dependency warning. No dependency was removed.

Visible-editor interaction/performance, game rendering, and Windows runtime
behavior remain to be tested on the actual desktops. The earlier Windows timing
numbers are not claimed for this revised implementation.
