# Nice Eq

A seven-band graphic equaliser with a spectrum analyser under the faders, as a
CLAP/VST3 plugin and a standalone application from one codebase. The ladder is
100, 200, 400, 800, 1.6 k, 3.2 k and 6.4 kHz, ±15 dB each, lowest at the left,
with ±15 dB of output trim standing apart at the right. The mode selector has
one position today, **G7**, named for the 1987 BOSS GE-7 — the machine that
inspired it, named the way every other card here names its source. The name on
this plugin is ours.

The plugin and the standalone application are one codebase: the same
`Plugin::process` and the same editor run in both. Only the answer to "who owns
the audio and the numbers" changes.

## Build

At the workspace root:

```sh
cargo xtask bundle nice-eq --release
```

Bundles land in `target/bundled/` as `Nice Eq.clap` and `Nice Eq.vst3`, the
display name coming from [`bundler.toml`](../bundler.toml). No `RUSTFLAGS` are
needed: the one dependency this crate adds that the other cards do not is
`realfft` over `rustfft`, which is plain Rust choosing its plan at run time, so
nothing in this graph refuses rustc's default x86-64 baseline. That was checked
rather than assumed — `cargo clippy -p nice-eq --all-targets -- -D warnings`
with no `RUSTFLAGS` at all is clean. On Windows, the workspace
`.cargo/config.toml` adds `+crt-static` to the MSVC target; a plugin loaded into
somebody's host generally should not carry its own C runtime, so build a
distributable there with `$env:RUSTFLAGS = ""` first and load it in a host
before trusting it.

Rust 1.99+ (edition 2024).

## Standalone

```sh
cargo run -p nice-eq --features standalone
cargo run -p nice-eq --features standalone -- --list-devices
```

The binary is an app-owned host. The device transport is
`nice_common::audio_io` — ASIO on Windows, cpal's hosts and JACK elsewhere, the
same opener the rack runs — and the audio page behind the corner cog is the same
panel nice-nam, nice-tuner and the rack show, so device names, rates and buffer
choices mean one thing across the workspace. `--list-devices` prints what the
current backend sees and exits, without a window and without opening anything.

Settings are `nice-eq-settings.ron` in the working directory: the audio choices,
the window geometry, the seven faders, the trim, and the four analyser
settings. A refused device change rolls back to the last pair of streams that
opened and says so in the view instead of dying.

Without a host the editor owns the parameters: nice-plug's `FloatParam` has no
public setter outside a host's `GuiContext`, so in standalone the shared cells
the audio thread reads *are* the truth. In a host the same writes ride
`ParamSetter` and a mirror keeps the cells a frame behind the parameters, so
automation stays the authority and the filter still reads one place.

## The controls

| Parameter | Range | What it is |
| --- | --- | --- |
| MODE | G7 | Which band ladder the faders stand on. |
| 100 Hz … 6.4 kHz | ±15 dB each | The seven faders, lowest at the left. The top band is a shelf; the other six are bells. |
| OUTPUT | ±15 dB | Level after the seven bands. The last word, and the one control that is not a filter. |
| SPECTRUM | Off / Top / All | `Off` computes nothing at all, `Top` draws the selected window, `All` overlays all six at once. |
| SOURCE | In / Out / Both | Which end of the box the analyser reads. Input is green, output is amber. |
| VIEW | Bars / Line | Two drawings of the same measured array. |
| WINDOW | 1k … 32k | The transform length, in samples. Clicking the picture steps it. |

Every one is a plain parameter: automation, modulation and the editor all read
and write the same numbers, and the view is drawn from the parameters every
frame, so host automation visibly moves the faders. The five stepped ones carry
their position names, so a host shows `Top` and `Both` rather than 1 and 2.

A fader is a position: the cap goes to the pointer. Shift makes the same travel
worth four times less — about 1 600 pixels across the full ±15 dB — and the
wheel is half a decibel a notch, a tenth with Shift, so a band parks on exactly
0.0 without a pixel being aimed at.

## The analyser

The picture under the faders is 20 Hz to 20 kHz on a logarithmic axis and 0 to
−84 dBFS vertically, whatever the sample rate. Bin magnitudes come from a Hann
window scaled so a full-scale sine reads 0 dBFS, and every pixel column gets the
**loudest** bin in it rather than an average, which is what keeps partials
visible when 16 385 bins arrive for 600 columns. Bars and interpolated are then
two drawings of that one column array: switching the view changes nothing that
was measured, and a test asserts exactly that.

The WINDOW settings are transform lengths, and the arithmetic is the honest
kind: a 4 k transform of 4 096 samples yields 2 049 bins and about 11.7 Hz of
spacing at 48 kHz, at the price of a window 85 ms long. `All` exists because
that trade does not have to be chosen. It transforms all six lengths from one
snapshot of the same samples, so the agile picture and the resolved one are the
same instant rather than six different ones, and draws the selected window last
and brightest over the other five.

The transform runs on the UI thread. The audio path copies channel 0 of the
watched end into a preallocated ring — the nice-tuner handoff — and the editor
does the windowing, the transform and the reduction. So an FFT can never glitch
audio at any size; with the editor closed no plan, buffer or scratch exists at
all; and `Off` is not a blacked-out picture but the absence of the work, which
one test proves by counting transforms over three frames: zero, three, eighteen.

`SOURCE` decides what gets copied, never what gets computed for the audio: one
lane copy per block for either single end, two for `Both`, none for `Off`.
`Both` is the default because it is the useful one — the equaliser's own curve
appears as the gap between the two spectra, with the input behind it as the
thing that was there first. Green and amber are the set's existing meter
colours, and the 0 dBFS line turns red only while the watched end has actually
peaked past it.

## How it sounds, mechanically

Six RBJ peaking bells at Q = 1.4 and one RBJ high shelf at Q = 0.707, computed
from the Audio-EQ-Cookbook formulae in normalised form, cascaded in ladder
order. Stereo in, stereo out, the same coefficients through separate memory per
lane: the pedal is mono and this plugin does not contradict that — feeding one
input answers on one output — but nothing is ever summed, because folding two
channels together would be the plugin making a host's mix decision.

Coefficients are solved once per block and crossfaded per sample against the
previous set, and the trim slews by the same route, through zero as a
multiplier. A mode change takes that same path, which is why a mode is a band
table and not a second plugin. Where the sample rate cannot hold a corner — the
6.4 kHz shelf at the 8 kHz rate the standalone dialog offers — the corner folds
to 45 % of the rate rather than putting its poles on the unit circle, and the
panel keeps saying 6.4 kHz because the parameter did not move.

What the shapes are worth, measured with the house Goertzel probe:

- one band pushed to +15 dB moves its own label by 15.0 dB, the next label by
  3.4 dB, and any label two steps off by less than 0.8 dB. One slider reads as
  one slider.
- the top band reads +7.5 dB at 6.4 kHz — a shelf's label is the middle of its
  transition — and +14.9 dB at 16 kHz, while sitting at 0 dB down at 800 Hz.
  The bells reach their full lift at their own labels, which is what a bell's
  label is.
- all seven at +15 dB runs from +23.5 dB at the labels to +11.2 dB in the
  hand-over valley near 5.7 kHz. The ladder lifts a contour and never cancels
  anything: the deepest point of the all-up curve is still 11 dB *up*.

## Tests

```sh
cargo test -p nice-eq
cargo test -p nice-eq --features standalone
```

Response questions are asked acoustically, in dB, at frequencies a reader can
check; stability is the Jury conditions over every band at every rate the
settings dialog offers. The analyser is measured rather than trusted: a
full-scale sine reads 0 dB at its own bin at every length, a column reports the
loudest bin in it, the two taps are kept apart, and `Off` asks for no
transforms. `tests/no_alloc.rs` proves a block, a fader sweep, a mode switch and
a rate change cost the allocator nothing; it deliberately links without
nice-plug, whose debug allocator and the counter's allocator cannot share a
binary. The standalone tests need no hardware: the pump moves ring buffers
through a real plugin instance and the settings file round-trips.

The host cost is metered rather than estimated: in the rack's live probe, one
instance of the bundle takes 9.4 µs of a 128-frame block at 48 kHz, 0.36 % of
the block, and four instances take 35.7 µs, 1.34 %.

The editor/window bridge is shared in [nice-plugin](../nice-plugin/README.md).
Plugin-specific view construction lives in `src/editor.rs`; standalone
activation follows the shared activate/reset sequence. Design reasoning, the
assumptions with numbers in them, and what the reference document does and does
not testify to are in [DECISIONS.md](DECISIONS.md).

## Inspiration and non-affiliation

These are independent frost projects, not products sponsored, approved or
endorsed by the manufacturers or authors of referenced equipment. Third-party
names identify sources and inspiration only; trademarks belong to their
respective owners. An "inspired by" description does not claim an exact circuit,
measured sonic match, or trademark clearance. See the shared
[inspiration and reference notes](../docs/plugin-inspiration.md).
