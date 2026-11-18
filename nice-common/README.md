# nice-common

Shared standalone audio configuration, persistence, and device discovery for
nice-tuner and nice-nam. With the `ui` feature it also provides their shared
frost audio-settings panel. It contains no audio processing and denies unsafe code. Native queries use the existing project-owned CoreAudio
wrapper. Each application's editor shell and stream owner stay in that application.

`Settings<P>` holds backend, input/output devices, channels, independent sample
rates and buffer sizes, monitoring latency, and window geometry. An application
implements `Preferences` for its own serializable fields and filename. Those
fields are flattened into the same RON map in `nice-tuner-settings.ron`. Nice NAM uses `nice-nam-settings.ron` with
model name/path and optional cabinet path. Access application fields explicitly
through `.preferences`, or through the convenience `Deref` implementation.
Application field names must not overlap shared fields.

`Settings::load/save`, backend normalization, geometry-only writes, channel
routes, and buffer/rate linking policy are shared. A missing backend stays
silent; unknown device defaults remain unset.

Enable `devices` only in standalone applications. It provides the existing CPAL,
CoreAudio and Linux JACK queries, ASIO stream ownership guard, serialized
`Snapshot` protocol, and `Snapshot::resolve_settings` validation. Discovery is
explicit and must run off the audio thread. ASIO needs the `asio-host` feature,
which pulls [`azo`](https://github.com/LastExceed/azo) — declared under
`cfg(windows)`, so enabling the feature off Windows compiles no ASIO and asks for
nothing: no SDK, no `CPAL_ASIO_DIR`, no C++ compiler. A caller of
`snapshot_via_child` must implement the `--list-devices <backend> <input>
<output>` RON protocol in its own standalone executable, as nice-tuner and Nice
NAM both do.

```sh
cargo test -p nice-common --features devices
```

## Shared settings panel

`ui::AudioPanel<P>` owns the standalone draft, applied and saved settings,
queries, default-first dropdown contents, scroll/drag popup interactions, green
default values, ASIO/JACK rules, linked output fields, validation, Reset, Back,
Escape, asynchronous Apply, disconnected-worker handling and persistence.
Both applications render this exact component; nice-nam has no separate cycling
picker implementation. The popup geometry and scrolling come from nice-tuner.

Applications provide their font, current window geometry and an Apply request
callback to their audio owner. No opening or restarting of devices occurs in the
UI crate. `ExtraRow`/`Event` let nice-tuner append its four live preferences without
putting tuner-specific fields in this crate. Hosted audio remains owned by the
DAW; nice-tuner's hosted read-only operating-condition display stays in its editor.

Enable with `nice-common/ui` in standalone builds; it includes `devices`.

## Reusable dropdown selector

Enable `widgets` to use `dropdown::draw_dropdown` without enabling standalone
audio discovery. It renders the selector extracted from nice-tuner: anchored or
centered placement, an optional centered title, scrolling and scrollbar dragging,
selected/hover colors, green defaults, and optional port-suffix preservation.
The caller supplies labeled values and keeps the selected value, open state,
scroll position and drag state. The response reports the picked row and whether
the popup consumed a click; application-specific actions stay with the caller.
The `ui` feature also enables `widgets`.

## Input level display and tooltips

`meter::PeakMeter` owns RMS smoothing and peak-hold ballistics, independently of
audio transport or a UI. Feed it levels in dBFS and a millisecond clock through
`tick`. With `widgets`, its `draw` method renders the input bar, peak marker and
dBFS scale at a caller-specified position, width, scale and bar color, returning
the hover state. `tooltip::draw` provides the pointer-relative tooltip used by
the tuner, keeping it inside the window. Applications supply the tooltip text
and decide whether hovering or clicking opens a control.

## Settings file format

Both standalone applications write pretty-printed RON in their working directory.
The schema is a map with quoted field names and RON `Some(...)` / `None`
options. The original bare-value / `()` spelling remains readable. This representation
keeps application preferences flat and preserves unknown fields during geometry
updates. For example:

```ron
{
    "backend": Some("core-audio"),
    "gate_db": Some(-40.0),
    "input_channel": Some(1),
    "output_channels": Some((kind: "stereo", channels: (1, 2))),
    "output_sample_rate": None,
}
```

Settings are read and written exclusively as RON. The loader opens exactly the
one path it is given — `current_dir()` joined with the `Preferences::FILE_NAME`
its caller supplies, every one of which ends in `.ron` — and a cold start, with
no file there yet, is defaults and no warning. A resize will not overwrite a
malformed RON file. The device-discovery helper protocol uses RON.

`asio_second_output` names the first channel (1-based, as this file spells them)
of a second output group carrying an exact copy of the first: `Some(5)` behind a
stereo route on 1–2 plays the same signal on 5–6. It belongs to ASIO, the one
backend where a single driver owns both groups, so `resolve_settings` refuses it
elsewhere and refuses a start that lands on the channels the first group already
plays. The copy is as wide as the route it copies, so the file carries a start
and never a second channel count.

`asio_second_device` names the driver carrying that copy. Absent, or spelled
exactly like the output, is the case above: one driver, two groups, one clock.
Any other name is a second driver, loaded beside the first and running on its own
clock, and the start channel is then counted on it — `Some(1)` is that interface's
first pair, not the first driver's, which is why moving a copy to another driver
drops a start that no longer means anything. A start beyond what that driver
answers for is refused; a driver chosen without a start channel is refused, since
guessing where a second interface's sockets are would be a guess about someone's
speaker cables.

`Settings::second_group` answers the one question those two rows leave open: which
driver the copy ends up on, and which of that driver's channels — `3–4` for a
stereo route starting at 3. nice-rack writes that into its status line
(`Audio running — copy to 'ReaRoute ASIO (x64)' 1–2`) because a second driver with
nothing on the other end looks, from inside this process, exactly like one carrying
a copy nobody asked for: the second driver's own callback is what drains the
crossing, so a copy going nowhere never falls behind and never says so. The line
says where to look; only the program on the far side can say whether anything is
listening there.

What "nothing is listening" means was measured rather than guessed, and the
measurement changed the code. `asio_io::Transport::pulse` and
`Transport::copy_pulse` answer, for each open driver, how many `bufferSwitch` calls
it has fired and how many of them moved a block, and the standalone's
`--audio-probe <seconds>` opens the saved settings' drivers with a 1 kHz tone in
place of a guitar and prints those numbers once a second. ReaRoute, with REAPER
running: 1.68 million callbacks a second on the copy, every one of them naming
buffer half zero, and one block served in total. A driver that never names the other
half is not double-buffered, and a guard that drops every repeat of an index mutes
such a driver for the life of the stream. So a repeat now counts as a repeat only
while the block it repeats could still be being answered — three quarters of that
driver's own period, chosen so a driver that returns its buffers early still gets
its second block and one that repeats a half microseconds later does not — and after
that gap it is the next block. Three quarters rather than the whole period because
being early is recoverable and being late is not, and no driver tells a host how
fast it actually reads.

Settings serialize directly through RON; geometry edits use `ron::Value`.
`nice-common` has no `serde_json` dependency.

## Plugin-independent stereo device transport

`audio-io` enables `audio_io::open` for resolved settings. Device callbacks move
samples through bounded rings and own no plugin handles. The caller runs DSP on
a separate worker, allowing it to stop without another device callback. The
transport reports native error text and callback counters for control-thread
health checks. It currently requires physical CPAL input/output devices.

The selected input channel is duplicated to stereo; processed stereo frames use
one SPSC queue and one `Monitor<StereoFrame>` clock/phase for independent-rate
playback. Existing `Monitor<f32>` callers remain source-compatible. The rack uses
this transport; the established tuner/NAM stream owners remain unchanged.

`asio_io::open` is the ASIO shape of the same promise. Its `Wanted` names the
driver, the channel counts and an optional `second_output` — the first channel of
a group that carries a copy of the first one — and every channel is asked for in
the single `create_buffers` a driver is given before it starts. The copy belongs
to the transport, not to a pump: a callback is still handed the block width it
was built for, and the extra channels are filled from that same scratch block in
the sample format each of them reports.

A copy named onto another driver is a second stream inside the first, and this
host is the only thing holding the two apart. `azo` will hand one apartment as
many drivers as it is asked for, so each driver here gets its own `Host`, which is
its own thread and its own single-threaded COM apartment; the SDK's one-global-
driver-per-process rule is stepped around rather than obeyed, and that is what the
pair is buying. The two are torn down in the order they are declared: the copy
stopped and its buffers given back before the live driver is touched. The second
driver is asked for its own rate and period first, since 64 frames on one
interface is not 64 frames on another, and a period the live stream was built
around is never traded for it.

Between the two clocks, every channel pair crosses one lock-free ring and one
`Monitor<StereoFrame>` — one phase and one clock correction for a pair, so the
copy's left ear is never corrected a frame away from its right — and an odd last
channel crosses alone. Each crossing holds two blocks of the reading driver, the
writer never waits for a reader that has stopped, and a copy that falls behind
loses its newest frames, says so once, and leaves the live stream running. Two
drivers set to 48 kHz are not at the same rate, so the crossing resamples even
when the two numbers agree, and the copy arrives one frame behind the one it was
made from.
