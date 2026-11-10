# Nice Wah

A wah-wah and auto-wah in one CLAP/VST3: one bandpass whose corner a treadle
sweeps exponentially, and an envelope follower wired to the same parameter a
foot would move. There is no mode switch. The auto-wah is the plain wah with
**ENV** turned up, because that is what an auto-wah always was.

The plugin and the standalone application are one codebase: the same
`Plugin::process` and the same editor run in both, as in nice-nam and
nice-tuner. Only the answer to "who owns the audio and the numbers" changes.

## Build

At the workspace root:

```sh
cargo xtask bundle nice-wah --release
```

Bundles land in `target/bundled/` as `Nice Wah.clap` and `Nice Wah.vst3`. No
`RUSTFLAGS` are needed: unlike nice-nam, nothing in this crate's graph refuses
rustc's default x86-64 baseline. On Windows, the workspace `.cargo/config.toml`
adds `+crt-static` to the MSVC target — a plugin loaded into someone's host
generally should not carry its own C runtime, so build a distributable there
with `$env:RUSTFLAGS = ""` first, and load it in a host before trusting it.

Rust 1.99+ (edition 2024).

## Standalone

```sh
cargo run -p nice-wah --features standalone
cargo run -p nice-wah --features standalone -- --list-devices
```

The binary is an app-owned host: nice-plug's own standalone wrapper is neither
built nor patched. The device transport is `nice_common::audio_io` — the same
cpal, JACK and ASIO-guarded opener the rack runs — and the audio settings page
behind the corner cog is the same panel nice-nam, nice-tuner and the rack
show, so device names, rates and buffer choices mean one thing across the
workspace. `--list-devices` prints what the current backend sees and exits,
without a window and without opening anything.

Settings are `nice-wah-settings.ron` in the working directory: the audio
choices, the window geometry, and every knob the player last left the filter
on. A refused device change rolls the app back to the last pair of streams
that opened, and says so in the view instead of dying; the small `DROPS`
counter under the cog counts capture-side overruns, the things a host would
have been the player's only way to hear about.

Without a host the editor owns the parameters: nice-plug's `FloatParam` has
no public setter outside a host's `GuiContext`, so in standalone the shared
cells the audio thread reads *are* the truth, written straight from drags,
wheel notches and the three voice buttons. In a host the same writes ride
`ParamSetter`, and a mirror keeps the cells a frame behind the parameters,
so automation stays the authority and the filter still reads one place.

## The controls

| Parameter | Range | What it is |
| --- | --- | --- |
| POSITION | 0–100% | The treadle. Heel to toe. |
| ENV | 0–100% | How far playing loudness walks the treadle up from where you left it. |
| HEEL | 40–1000 Hz | The corner with the pedal rocked back. |
| TOE | 1.5–12 kHz | The corner with the pedal shoved forward. |
| RESONANCE | Q 0.7–12 | How narrow — and how vowel-like — the band is. |
| TAPER | −1…+1 | Where the treadle's resolution lives: negative gathers at the toe, positive at the heel. |
| OUT | ±12 dB | Output trim, after the filter. |

Every one is a plain parameter: automation, modulation and the editor all read
and write the same numbers, and the view is redrawn from the parameters every
frame, so host automation visibly moves the knobs.

The pedal bar at the bottom fills green to where the filter *actually* sits —
treadle plus envelope — while a pale tick marks where the treadle itself is.
The gap between them is the envelope's answer, drawn.

## The three voices

The buttons write five parameters each — HEEL, TOE, RESONANCE, TAPER, OUT —
through the host's own parameter API, so a voice change records as a knob
move and stays automatable. No voice is a mode; the DSP is the one filter
whatever the buttons did last.

| Voice | Heel | Toe | Q | Taper | Trim | The shape it chases |
| --- | --- | --- | --- | --- | --- | --- |
| VOX '70 | 350 Hz | 2.2 kHz | 2.2 | +0.6 | −4 dB | The Shakespeare inductor's damped, heel-biased choke wah. |
| CRY BABY | 90 Hz | 4 kHz | 4.0 | 0 | 0 dB | The wide-open Dunlop sweep, the archetype the defaults ship at. |
| SLASH | 350 Hz | 4.5 kHz | 3.5 | −0.4 | +3 dB | Heel raised out of the mud, the singing band gathered toward the toe. |

No manufacturer has ever published these curves. The numbers are the measured
consensus of published circuit analyses — the Vox's high-DCR inductor self-
damping its Q near 2, the Cry Baby's 500 mH Fasel choke sweeping a narrow
band a full four decades, the hot-rodded mod's raised heel and toe-weighted
taper — not datasheet transcriptions, and nobody should pretend otherwise.

## How it sounds, mechanically

A constant-peak-gain RBJ bandpass, so the sweep is a vowel travelling up the
spectrum and never a level bump; the corner travels exponentially between heel
and toe (`heel · (toe/heel)^(t^2^taper)`), coefficients are crossfaded per
sample across every change, and the corner slews 20 ms behind its target so a
stepped automation lane cannot zipper. The follower is a dB-law one-pole
(2 ms attack, 300 ms release) floored at −60 dBFS; stereo channels share one
follower on the louder of the two, the way one hand works one pedal.

A heel set above its toe is a silent guard rail: the sweep parks at the heel
rather than inverting.

## Tests

```sh
cargo test -p nice-wah
cargo test -p nice-wah --features standalone
```

The filter tests ask acoustics: a Goertzel bin answers "how loud does a pure
tone come out" exactly, and the assertions are the pedal's identity — unity at
the corner whatever the Q, a wall an octave off it, the treadle walking the
passband up the spectrum, two decades of level parting the envelope's answers.
`tests/no_alloc.rs` is the audio thread's witness that a block, and an
automation move between blocks, cost the allocator nothing; it deliberately
links without nice-plug, whose debug allocator and the counter's allocator
cannot share a binary. The standalone tests need no hardware: the pump moves
ring buffers through a real plugin instance, the settings file round-trips,
and the dummy backend resolves to exactly the silence it promises.
