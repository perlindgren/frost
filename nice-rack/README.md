# nice-rack

A standalone frost application for an in-process CLAP audio-effects chain. The
GUI flows left to right through serial stages, with parallel branches stacked
vertically. VST3 hosting remains a future step.

## Run

```sh
cargo run -p nice-rack-standalone --release
```

The small `crates/window-input` wrapper reads the macOS cursor during OS file
drags, which winit 0.30 does not include in its drop events. Its native access
is documented in its SAFETY.md.

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
- Input and output RMS/peak-hold meters sit at the bottom.

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

On Windows and macOS the transport is built from the selected physical CPAL
input and output devices. On Linux the same panel also offers named JACK
ports: the rack registers one `input` and one `left` port under its own
client and connects them to the selected JACK source and destination, or a
`right` port as well when a second, different destination is selected. A
single destination carries the mono average because JACK ports are mono; the
pair carries the true stereo graph output. The server's sample rate and
quantum govern both directions, and the rack requests its buffer size from
the offered quanta. The rack never offers its own audio ports to itself. The
existing tuner/NAM JACK implementations are unchanged. Capture selects
the configured physical mono channel and duplicates it into the rack input. The
processed stereo signal is routed to the selected output pair, or averaged if a
mono destination was explicitly selected. Stereo resampling uses one shared clock
and interpolation phase for both channels.

The standalone settings include a vector of plugin instances: stable instance ID,
serial/parallel stage, entry path, class ID, native serialized state (when CLAP
state is supported), and editor position/size. Duplicate instances remain
independent. State is checked every two seconds and at editor/application close;
unchanged records do not rewrite the file. Saved entries are loaded again on
launch and must remain trusted native code. A missing/unloadable entry reports
an error rather than silently overwriting the saved rack.

Editor positions use physical screen coordinates; content sizes use logical
points. Positions stranded on disconnected displays are discarded. Editors close
before graph replacement and do not stop audio when closed normally. Native
Wayland and floating-only editors currently use generic controls instead; their
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

The UI embeds frost's Fira Code and the same Fluent settings font as nice-tuner.
Distributable packaging must include their existing license notices.
