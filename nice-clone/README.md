# nice-clone

Klon-style overdrive scaffolding, using the same frost editor and nice-common
widgets, metering, audio settings, and standalone transport as the other pedals.

This is a separate Cargo workspace member with CLAP/VST3 exports and an optional
standalone application. Gain, Treble, and Output are host-automatable normalized
controls (0–100%, initially 50%). The standalone remembers their positions in
`nice-clone-settings.ron`, alongside the shared audio and window settings.

Audio passes through unchanged in mono and stereo. The controls currently store
positions only: no clipping, filtering, gain, or other effect DSP is connected.
Ranges and defaults are UI placeholders until the model is implemented.

```sh
cargo check -p nice-clone
cargo run -p nice-clone --features standalone --bin nice-clone-standalone
cargo test -p nice-clone --features standalone
```

Drag a knob vertically; hold Shift for finer movement. The mouse wheel also
adjusts knobs. In standalone, the settings cog opens nice-common's audio panel.
Plugin audio devices remain owned by the host.
