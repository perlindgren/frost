# nice-rack

A standalone frost application for an in-process CLAP audio-effects chain. The
GUI flows left to right through serial stages, with parallel branches stacked
vertically. VST3 hosting remains a future step.

## Run

```sh
cargo run -p nice-rack-standalone --release
```

The small `crates/window-input` wrapper reads the macOS and Windows cursor during
OS file drags, which winit 0.30 does not include in its drop events. Its native
access is documented in its SAFETY.md.

On Linux, drops arrive through winit under X11. winit 0.30 has no Wayland
file-drop support, so under Wayland the standalone receives the compositor's
data-device messages directly (`src/backend/wayland_drop.rs`) and feeds the
same drop path as X11. The module is written to be deleted once winit 0.31
ships native Wayland drag-and-drop.

Editors on Linux arrive through X11 too, for a different reason. The plugin
side implements the X11, Cocoa and Win32 embedding APIs only, so handing it a
Wayland window makes it refuse the attach and no editor opens. The CLAP ABI
does name a `"wayland"` window API, but its window-handle union carries no
Wayland member, so there is nothing to hand a plugin even where one asks —
whatever the host wanted. So when the rack window is a Wayland window, each
editor
window is instead built on a second winit event loop pinned to X11
(`src/editors/x11.rs`) and the plugin is given that Xlib handle through the
same attach path that already works under native X11. The editor windows are
XWayland windows; on a fractional-scaling desktop KDE scales them by
rasterising, not natively. Like the drop module, this file is written to be
deleted once plugins attach natively.

The small `crates/standalone` executable is the native-loading trust boundary.
The nice-rack application library denies unsafe code; the executable makes the
single unsafe call needed to load the operator's selected native CLAP entry.
Dropping a plugin runs native code in this process. It does not provide crash
isolation or make an arbitrary third-party plugin safe.

## Main view

- Drop a `.clap` file or macOS `.clap` bundle onto the window to add an effect.
  If the entry contains multiple classes, a selector asks which one to add.
- Drop above or below a card to create a parallel branch; drop to its left or
  right to create a serial stage. The same placement rules apply to files from
  the desktop and cards dragged within the rack. Insertion markers preview the
  routing. Parallel outputs sum at unity gain.
- Dragging near an edge scrolls the rack. The wheel scrolls vertically; hold
  Shift to scroll horizontally.
- Click a card's × to remove it. Graph edits rebuild through the same stop and
  state-preservation path as device changes. Other edits are disabled while a
  reconfiguration is pending. A failed edit restores the previous chain.
- Hover a card for name, vendor, version, description, channel counts and path.
- Click a card to open its native editor in a separate rack-owned window. When
  no supported embedded editor exists, a frost overlay exposes the plugin's
  parameters. Drag the sliders, scroll for more controls, and close with × or
  Escape. A drag beyond five logical points moves the card instead of opening it.
- Double-click either gain value in normal mode to reset that trim to 0 dB
  (unity). A single click still opens its numeric editor. Each corner shows
  its trim to one decimal; at unity the box and its value are both gone,
  and a drag on the meter's track brings them back — through unity and all
  — until it releases there. The corner's place stays the click target for
  that wire's gain either way.
- Right-click either meter beside a card, expanded or minimized, to toggle
  **bypass**. Both meters collapse and an amber wire goes up from the input,
  across above the card, and down to the output. The short leads into the card
  and the gain readouts disappear. Bypass keeps the processor running on actual
  input while discarding its wet output. It skips both gain trims, passing unity
  dry audio with the card's latency preserved for parallel
  alignment. The previous trims and cable breaks return when bypass is removed.
- Right-click the **card body** to toggle **pause**. A paused card is dimmed and
  marked PAUSED in its status line, keeping its name. Both gain readouts and
  gain handles are hidden and inactive; the saved trims return on resume. Its
  processor is not called and its output signal and output meter are zero,
  including any previous tail. Pause and bypass are separate:
  enabling bypass around a paused plugin passes dry audio; removing bypass
  returns to silence until the card is resumed. Its utilization bar immediately
  removes the added DSP segment while keeping incoming usage; hovering reports
  DSP 0.0% and separately identifies any remaining rack overhead.
  Both switches are saved with
  the rack. Right-clicking bare cable still toggles its existing wire break.
- Input and output RMS/peak-hold meters sit at the bottom.
- Every effect card has its own bottom-edge audio-budget meter. Muted purple
  fills from the left for time already spent before that card; green adds that
  card's contribution. It turns amber from 85% and red at 100%; a red end marker
  holds a detected overrun briefly. Hover over the line for the percentage split.
  The full card width is one actual audio block's deadline (`frames / sample rate`),
  including short blocks. This is live processing time, not plugin latency in
  samples or total machine CPU usage. Routing, trims and wire meters belong to
  each card's measured cost; the incoming portion includes earlier graph work.
  Parallel branches execute on the same audio thread, so already-executed peers
  count toward the budget too. Device transport and work outside graph execution
  are excluded. Readings are gently smoothed and clear when audio stops.


The first version accepts effects with one mono/stereo main bus in each direction.
The rack input/output is stereo. Explicit channel-map nodes duplicate mono into
stereo or average stereo into mono as required between plugin buses. No limiter
or mixer gain normalization is inserted.

## Audio settings

The upper-right cog opens the exact `nice-common::ui::AudioPanel` used by
nice-tuner/nice-nam. Its selected backend, devices, input channel, output route,
separate rates and separate buffer sizes dictate the streams. Apply opens the
requested configuration asynchronously and preserves supported plugin state.
Settings are written to `nice-rack-settings.ron` in the working directory; window
geometry is saved independently. Missing settings boot silently.

Which transports exist is the platform's, and the panel's backend row is where
that shows. On Windows the row offers `ASIO` and `no audio (dummy)` and nothing
else, and what Apply opens is `nice_common::asio_io`: one ASIO driver carrying
both directions on one clock, its callbacks on the driver's own thread. There is
no WASAPI stream there to fall back to — `nice-common` declares no cpal on
Windows at all, so nothing in this build could open one — and a settings file
naming the retired `wasapi` or `auto` key is refused in words rather than opened
on a host nobody chose. On macOS the row offers the system default and macOS
Core Audio, and the transport is the two cpal streams, one per direction. On
Linux the same panel also offers named JACK ports: the rack registers one
`input` and one `left` port under its own client and connects them to the
selected JACK source and destination, or a `right` port as well when a second,
different destination is selected. A single destination carries the mono average
because JACK ports are mono; the pair carries the true stereo graph output. The
server's sample rate and quantum govern both directions, and the rack requests
its buffer size from the offered quanta. The rack never offers its own audio
ports to itself. The existing tuner/NAM JACK implementations are unchanged.
Whichever backend is open, capture selects the configured physical mono channel
and duplicates it into the rack input. The processed stereo signal is routed to
the selected output pair, or averaged if a mono destination was explicitly
selected. Stereo resampling uses one shared clock and interpolation phase for
both channels.

The standalone settings include a vector of plugin instances: stable instance ID,
serial/parallel stage, entry path, class ID, native serialized state (when CLAP
state is supported), and editor position/size. Duplicate instances remain
independent. State is checked every two seconds and at editor/application close;
unchanged records do not rewrite the file. Saved entries are loaded again on
launch and must remain trusted native code. A missing/unloadable entry reports
an error rather than silently overwriting the saved rack.

Editor positions use physical screen coordinates; content sizes use logical
points. Positions stranded on disconnected displays are discarded. Editors close
before graph replacement and do not stop audio when closed normally.
Floating-only editors currently use generic controls instead; their
window placement cannot be managed through this embedded-window path. Plugins
requiring timer/FD host extensions need further integration. Native editor
placement still needs desktop verification.

Separate rack document files, instruments/MIDI and sidechains remain future work.
The shared hosting adapter can be reused by a future rack plugin, but this build
only wires editor-window events through the standalone frost event loop.

## Shutdown and recovery

Device callbacks only transport samples through bounded queues. A dedicated DSP
worker owns the processing endpoint and polls even when capture supplies no data.
It can therefore stop and return the graph after a device stops calling back.
Native activation, state I/O, deactivation and instance destruction stay on the
main thread. Stream destruction and reopening happen away from GUI rendering.

Native stream errors and stalled input/output callbacks trigger bounded recovery.
The rack retains the graph and supported plugin state, closes old streams before
reopening, and tries the saved configuration again. Repeated failure leaves audio
stopped and allows a manual Apply retry. Failure to apply a new configuration
attempts rollback to the previous configuration. Errors retain the system's
message and device context. The window closes after the DSP owner has stopped;
cleanup does not require another device callback.

This handles device loss, not an arbitrary native plugin hanging forever inside
process or corrupting the host. In-process hosting cannot forcibly terminate a
hung plugin safely. Plugins without the state extension currently return to their
defaults when instances are replaced. Graph edits can cause a short interruption;
there is no crossfade or seamless live graph swap yet.

## Validation

```sh
cargo test -p nice-rack -p nice-rack-engine
cargo test -p nice-common --features audio-io,ui
cargo build -p nice-tuner --lib
NICE_RACK_TEST_CLAP="$PWD/target/debug/libnice_tuner.dylib" \
  cargo test -p nice-rack-standalone --test recovery -- --ignored
```

The opt-in integration uses this repository's own freshly built plugin. It checks
native processing and retirement without more capture callbacks, plus add/reorder/
remove controller behavior without opening devices or a GPU window. It does not
replace desktop drag/drop and hardware listening checks.

The same opt-in run on Windows, in PowerShell. The first line writes the `.dll`
the variable names:

```powershell
cargo build -p nice-tuner --lib
$env:NICE_RACK_TEST_CLAP = "$PWD\target\debug\nice_tuner.dll"
cargo test -p nice-clap-host --test tuner_roundtrip -- --ignored
cargo test -p nice-rack-standalone --test recovery -- --ignored
cargo test -p nice-rack --lib -- --ignored `
  runtime::tests::a_dropped_plugin_is_saved_before_audio_has_ever_run `
  runtime::tests::a_refused_opening_keeps_the_chain_it_was_started_with
```

Nine tests wait on that variable. Seven of them need no device, and all seven
pass on Windows as written above. The two left over open this machine's real
audio devices, so they stay a deliberate run rather than a line in a script.
What the nine are not is a cover for the rest: the rack's other 23 tests now
run by name in `.githooks/pre-push`, on the machine doing the pushing, as the
gate has run them on both runners since pull request 31.

The UI embeds frost's Fira Code and the same Fluent settings font as nice-tuner.
Distributable packaging must include their existing license notices.

When the main rack window is occluded or minimized, its UI stops drawing.
Audio continues independently, and the event loop services plugin callbacks,
recovery and shutdown on a 20 ms maintenance wake. Restoring the window resumes
rendering. This is an opt-in Frost policy; games keep their existing behavior.
