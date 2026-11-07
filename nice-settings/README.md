# nice-settings

Shared standalone audio configuration, persistence, and device discovery for
nice-tuner and nice-nam. With the `ui` feature it also provides their shared
frost audio-settings panel. It contains no audio processing and denies unsafe code. Native queries use the existing project-owned CoreAudio
wrapper. Each application's editor shell and stream owner stay in that application.

`Settings<P>` holds backend, input/output devices, channels, independent sample
rates and buffer sizes, monitoring latency, and window geometry. An application
implements `Preferences` for its own serializable fields and filename. Those
fields are flattened into the same JSON object, preserving nice-tuner's existing
`nice-tuner-settings.json` format. Nice NAM uses `nice-nam-settings.json` with
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
implement the `--list-devices <backend> <input> <output>` JSON protocol in its own
standalone executable, as nice-tuner does. Nice NAM has no executable yet.

```sh
cargo test -p nice-settings --features devices
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

Enable with `nice-settings/ui` in standalone builds; it includes `devices`.
