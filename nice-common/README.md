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
explicit and must run off the audio thread. ASIO needs the optional `asio-host`
feature and its existing SDK requirements. A caller of `snapshot_via_child` must
implement the `--list-devices <backend> <input> <output>` RON protocol in its own
standalone executable, as nice-tuner does. Nice NAM has no executable yet.

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

Settings are read and written exclusively as RON. Legacy `.json` files are
ignored and left untouched. A resize will not overwrite a malformed RON file.
The device-discovery helper protocol uses RON.

Settings serialize directly through RON; geometry edits use `ron::Value`.
`nice-common` has no `serde_json` dependency.
