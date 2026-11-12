# Nice Echo

A model of the **Roland RE-201 Space Echo** as a CLAP/VST3 plugin: a tape
loop with one record head and three playback heads, a spring reverb tank
riding beside it, and the machine's twelve-position mode selector that
decides which heads and whether the spring reach the output. Wow and
flutter live in the transport, saturation lives in the record path, and the
tone stack shapes what comes back. *Nice Echo* is this crate's own name;
"Roland RE-201 Space Echo" is the name of the machine being modelled, used
here only to say what the subject is — the repository rule is that brand
names never become our product names.

**Current state: the sound is in, the panel is drawn, and the division
between the known and the chosen is in the open.** `dsp.rs` runs the whole
tape path — transport, heads with distance-dependent HF loss, the
saturating record path, the spring tank, tone, and the three output trims —
with a test for each claim it makes and an allocation witness over every
mode and every control. The quotes that anchor it are below, and every
number that is *not* one of those quotes carries a label, **A-1** through
**A-10**, in the source and in this file, so a reader can tell the quoted
law ("the slower the tape, the longer the delay") from this model's tape
speeds (A-3) at a glance. `TODO.md` keeps the list of what a tape machine
on a bench would settle; until then those labels are the crate's honest
debts. The repository rule is *measure, never speculate*, and where there
is nothing to measure, the next best thing is to write down which numbers
were chosen rather than found.

Like its siblings, plugin and application are one codebase: the same
`Plugin::process` and the same editor run in both. Only the answer to "who
owns the audio and the numbers" changes.

## What the original is

From *Inside a Space Echo* (Red Bull Music Academy, archived at
<http://web.archive.org/web/20190912173023/https://daily.redbullmusicacademy.com/specials/2016-inside-a-space-echo/>),
which is the walk-through this model leans on hardest: the mode selector is
a "combination of three playback heads and the spring reverb unit,
achieving 12 different echo variations", and it "includes a reverb only
mode"; the repeat rate works by "varying the tape speed… the slower the
tape, the longer the delay"; Intensity sets the repeat count and "high
intensity levels allow for the echo to self-oscillate"; the Reverb Volume
works "at mode selections between 5 and 11 and reverb only"; on the
echo/normal switch, "echo" means the input is echoed 100 %; and the machine
runs a free "tape loop", which "cuts down on tape wear and playback speed
irregularities" — the last being the source's own reason for this model's
deliberately modest wow and flutter (A-5).

From the RE-101/RE-201 Service Notes
(<http://manuals.fdiskc.com/flat/Roland%20RE-101%20&%20RE-201%20Service%20Manual.pdf>):
the output jack is specified at −15 dBm (H), −23 dBm (M) and −35 dBm (L) —
8 dB from H to M and 20 dB from H to L, which is exactly what
`dsp::OUTPUT_TRIM_DB` carries. The absolute dBm figures say what the jack
delivers into its rated load; what travels into a modern interface is
whatever the host does with it, so the numbers enter the DSP as relative
trims only. The same manual specifies the INSTRUMENT input at −24 dBm into
16 kΩ; this model does not build an input gain stage, so that figure is
cited for completeness and modelled not at all.

Wikipedia (<https://en.wikipedia.org/wiki/Roland_Space_Echo>) is the
reference for the machine's identity — a Roland box combining tape delay
and spring reverb, the sound half a generation of records is built on.

**What neither source gives is a delay time, a head spacing, a wow-and-
flutter figure, a hiss level or a self-oscillation threshold.** Everything
in `dsp.rs` that rests on such a number carries one of the labels below —
the same labels appear in the source beside the constants themselves, and
the rule is that a named assumption is worth more than a smoothed-over one:

| Label | The assumption | Why it is needed |
|-------|----------------|------------------|
| **A-1** | Head taps at 110 / 220 / 330 ms at the slowest tape speed | Anchored to the commonly-cited "up to ~340 ms" figure; no service document states a spacing. |
| **A-2** | The three heads are equally spaced | Nothing states the spacing pattern. |
| **A-3** | Repeat Rate spans tape speeds 1.0× to 3.25× (fastest taps ≈ 34 / 68 / 102 ms) | The sources give the direction of travel, never the range. |
| **A-4** | The twelve-position mode table below | Twelve positions and a spring-only position are quoted; the wiring of the other eleven is this model's, stated so it can be argued with. |
| **A-5** | Wow 0.6 Hz ±0.25 ms, flutter 6.1 Hz ±0.03 ms | No figure exists to quote; the free-running loop is the sources' own argument for a steady transport. |
| **A-6** | The spring tank's exact topology (low-pass, three all-pass/low-pass sections, tail low-pass, fed in parallel) | "Spring reverb unit" is all the sources say; the all-pass maths is the family's own, from nice-phase. |
| **A-7** | NORMAL = dry + echo; ECHO = dry gone, 100 % echo | Which integer means which position is this model's reading of the quote. |
| **A-8** | Per-head two-pole HF loss at 9 / 6 / 4 kHz | Duller with distance is a fact of tape heads; the corners are chosen. |
| **A-9** | A deterministic ±1.5e-4 bias noise in the record path | No hiss figure exists; and a noise-free digital loop makes silence a fixed point that re-records itself forever — the model needs *some* floor to stay alive. |
| **A-10** | Intensity crosses unity loop gain at ≈ 0.74 of its travel | Self-oscillation at high settings is quoted; the threshold is this model's. |

## Controls

Nine parameters, exactly as the host sees them:

| Parameter | Type | Range | What it does |
|-----------|------|-------|--------------|
| Mode | stepped, 0–11, labelled "1"–"12" | twelve positions | Which heads and whether the spring reach the output (A-4 table below). Panel number = integer + 1, pinned by a test. |
| Repeat Rate | float 0–1 | 0 = slowest tape = longest delay | The quoted law: slower tape, longer delay. Slew-limited, and the smear on the way is the trick the control is *for*. |
| Intensity | float 0–1 | | Repeat count; top quarter self-oscillates (A-10). |
| Echo Volume | float 0–1 | | How much echo in the output. |
| Reverb Volume | float 0–1 | | How much spring, on the positions that have the spring in — "between 5 and 11 and reverb only", as the walk-through puts it. |
| Treble | float 0–1 | centred = passthrough | One of the two tone knobs. |
| Bass | float 0–1 | centred = passthrough | The other. |
| Echo Switch | stepped, 0–1, "Normal"/"Echo" | | ECHO is the 100 %-echo position (A-7). |
| Output Level | stepped, 0–2, "H"/"M"/"L" | 0 / −8 / −20 dB | The service manual's output jacks. |

### Where the controls sit on a fresh instance

Mode "11" — all three heads and the spring, the widest position the dial
has, so the factory sound has both effects in it and both knobs audible.
Repeat Rate 0.45, the weakest number here: the sources never give a taper,
so this is "medium" on a linear span, rounded to what the evidence
supports. Intensity 0.4, deliberately under the A-10 crossover — a first
listen should hear repeats *decay*, and the howling quarter sits at the top
of the knob for whoever wants it. Echo Volume 0.5, Reverb Volume 0.3, both
tone knobs centred, Echo Switch on NORMAL (the machine's playing position
is not 100 % wet), Output Level H.

## The twelve positions

The mode selector, as this model wires it (**A-4** — the quoted facts are
that there are twelve positions, that they combine the three heads and the
spring, and that one position is the spring alone; the spring appearing
from position 5 upward is the walk-through's "between 5 and 11 and reverb
only"):

| Panel | Selection | | Panel | Selection |
|-------|-----------|-|-------|-----------|
| 1 | head 1 | | 7 | head 3 + spring |
| 2 | head 2 | | 8 | heads 1+2 + spring |
| 3 | head 3 | | 9 | heads 2+3 + spring |
| 4 | heads 1+2+3 | | 10 | heads 1+3 + spring |
| 5 | head 1 + spring | | 11 | heads 1+2+3 + spring |
| 6 | head 2 + spring | | 12 | spring only |

The feedback around the loop is the sum of the *selected* heads — the mixer
the table describes — so switching from "1" to "4" does not just add
arrivals, it changes how much of the echo is re-recorded.

The face shows the same sentence under its mode knob — `heads 1+2 + spring` —
and builds it from the mask the mixer itself reads rather than from a second
table, so the caption and the sound have no way to disagree.

## Build

At the workspace root:

```sh
cargo xtask bundle nice-echo --release
```

That is the bundling command — `cargo build` does not produce a plugin. It
lands `Nice Echo.clap` and `Nice Echo.vst3` in `target/bundled/`, with the
font licences inside the VST3 and beside the CLAP (a CLAP is deliberately
one file, so its licence text travels next to it). No `RUSTFLAGS` are
needed: unlike nice-nam, nothing in this crate's graph refuses rustc's
default x86-64 baseline. On Windows, the workspace `.cargo/config.toml`
adds `+crt-static` to the MSVC target, so build a distributable with
`$env:RUSTFLAGS = ""` first and load it in a host before trusting it either
way.

## Standalone

```sh
cargo run -p nice-echo --features standalone
cargo run -p nice-echo --features standalone -- --list-devices
```

On Windows in a confined sandbox two more variables are needed:
`CPAL_ASIO_DIR` pointing at an unpacked ASIO SDK (a relocated `TEMP`
defeats asio-sys's own SDK lookup), and a `TEMP`/`TMP` inside the tree for
the linker.

The binary is an app-owned host — nice-plug's own standalone wrapper is
neither built nor patched. The device transport is `nice_common::audio_io`,
the same cpal, JACK and ASIO-guarded opener the rack runs, and the audio
settings page behind the corner cog is the same shared panel nice-nam,
nice-tuner, nice-wah, nice-phase, nice-pi-face and the rack show, so device
names, rates and buffer choices mean one thing across the workspace.
`--list-devices` prints what the current backend sees and exits.

Settings are `nice-echo-settings.ron`: the audio choices, the window
geometry, and every control the player last moved — `None` for a control
never touched, so the file records moves rather than restating the plugin.
A refused device change rolls back to the last pair of streams that opened
and says so in the view; the `DROPS` counter under the cog counts
capture-side overruns.

Without a host the editor owns the parameters: nice-plug gives a parameter
no public setter outside a host's `GuiContext`, so in standalone the shared
cells the audio thread reads *are* the truth. In a host the same writes
ride `ParamSetter` and `process()` mirrors the parameters into the cells
each block, so automation stays the authority and the tape path still reads
one place.

## Tests

```sh
cargo test -p nice-echo
cargo test -p nice-echo --features standalone
```

The interface is what the scaffold could promise before there was sound,
and it still does: that the host sees exactly the nine parameters promised,
that every switch offers exactly the positions its labels claim — and that
typing a label back means what the host's own parser says it means, because
the registered parser and the test call the same function — that the panel
number is always the integer plus one, that the factory state is a
configuration the sources actually describe, that a saved move seeds the
audio thread and a control never moved keeps the plugin's own default, that
the settings file round-trips, that the meter counts channels and samples
in the right order, that the pump moves frames through a real plugin
instance without adding or losing anything, and that a block and a sweep of
every control cost the allocator nothing (`tests/no_alloc.rs`, which links
without nice-plug for that reason and carries no `cfg`, so it runs on
Windows too).

The tape path is asked with rulers rather than tuned by ear and then
described — measured arrivals, measured spectra, measured loop gain —
against arithmetic the model has no standing to dispute:

* the mode table the README ships *is* the table the DSP runs, twelve
  positions, spring admitted from panel 5 upward, panel 12 the spring
  alone — and automation outside the selector clamps to it;
* the caption under the mode knob is that same table read back in words: a
  caption names a head exactly when its mask routes that head, and says
  "spring" exactly when its mask routes the spring;
* slower tape is longer delay for every head, the slowest taps sit exactly
  on the A-1 anchors, and the knob's span is exactly the A-3 ratio;
* a burst arrives at each selected head at its tap time and at no
  unselected one — three heads selected, three arrivals heard;
* heads get duller the farther the tape has travelled (A-8 as behaviour:
  the same noise through each head's pole, wiggles measured);
* Intensity adds repeats, full Intensity outlives the input — the quoted
  self-oscillation — and the loop gain crosses unity near the A-10 point;
* the spring smears without escaping its bound, and position 12 is the
  spring alone, echo gone;
* the output steps are the service notes' numbers: −8 and −20 dB;
* the tone stack is a wire through when centred, rides the highs and the
  lows where the knobs say, and centred-plus-flat is bit-identical passthrough;
* the transport wobbles a little and no more — the wow and flutter sit
  inside their A-5 depths and never wider;
* turning Repeat Rate smears the delay on the way instead of jumping, and
  the saturation is odd, bounded, and never lets the loop out of full scale;
* one tape is heard in both lanes: a mono input through stereo taps is the
  same echo twice, because there is one tape, not two.

What none of that tests is the ten named assumptions — the spacing, the
speeds, the wiring of eleven mode positions, the transport's depths, the
tank's topology, the corners, the hiss floor, the crossover. No test can
check an assumption against a document that does not state it; those
belong to `TODO.md`, and they are what an afternoon with a tape machine and
a bench would retire.
