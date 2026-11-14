# Ecomplex

A model of the **Binson Echoplex T.E.C. 1** as a CLAP/VST3 plugin: one
motor turning one drum, one playback head on it, and a tube front end in
front of the whole thing. The Delay Time knob changes the *speed of the
tape* — a longer delay is literally slower tape, and slower tape wobbles
more, so delay and wow are one control, not two. Recount feeds the echo
back toward the record head, Timbre shapes the repeats, and Echo Volume
sits the echo against a dry signal that is *always there*. *Ecomplex* is
this crate's own name; "Binson Echoplex T.E.C. 1" is the name of the
machine being modelled, used here only to say what the subject is — the
repository rule is that brand names never become our product names.

**Current state: the sound is in, the panel is drawn, and the division
between the known and the chosen is in the open.** `dsp.rs` runs the whole
path — input trims, the saturating tube-style front end, the drum with its
speed-coupled wow, the single head, the tone stage in the feedback loop,
and the wet/dry mix — with a test for each claim it makes and an allocation
witness over the block and over a sweep of every control. Every number that
is not a quoted fact about the machine carries a label, **A-1** through
**A-8**, in the source and in this file, so a reader can tell the known
shape ("slower tape, longer delay, deeper wow") from this model's tape
range (A-1) at a glance. `TODO.md` keeps the list of what a machine on a
bench would settle; until then those labels are the crate's honest debts.
The repository rule is *measure, never speculate*, and where there is
nothing to measure, the next best thing is to write down which numbers were
chosen rather than found.

Like its siblings, plugin and application are one codebase: the same
`Plugin::process` and the same editor run in both. Only the answer to "who
owns the audio and the numbers" changes.

## What the original is

The Binson Echoplex — Binson & Cerrone, France, the Echo Drive of 1964
developed into the T.E.C. around 1970 — is the tape echo that preceded the
Roland RE-201 in this workspace's other crate, and it differs from that
machine in exactly the ways this model is shaped by: a **tube** front end
(the familiar 12AX7 stage) with its own high-gain microphone input, where
the RE-201 is solid-state; **one** playback head on a drum turned by **one**
motor, where the RE-201 has a bank of heads and a mode switch; and a delay
time that comes from varying tape speed, so the transport's wobble deepens
as the delay lengthens instead of being an independent effect.

**No service document for the machine was consulted for this model, and
this crate quotes no source text.** The facts above are the machine's
common panel-and-reputation description — what its controls are called and
what players and repair notes have always said the box does — and this file
says so plainly rather than dressing tier-three knowledge in a citation it
does not have. Where the common description stops — the exact delay range,
the drum's rotation rate, the flutter depth, the crossover of Recount, the
tone stage's real topology, the front end's exact curve — the model needed
a number anyway, and those numbers are named assumptions, **A-1** through
**A-8**, carried in `dsp.rs` beside the constants themselves:

| Label | The assumption | Why it is needed |
|-------|----------------|------------------|
| **A-1** | Delay range 40 ms – 3 s on an exponential taper | The real range came from the installed loop length and the motor's speed range; only a service sheet could say more. |
| **A-2** | Drum reference speed 5 Hz at the fastest setting, wow rate tracking the drum as it slows | The direction (same motor, slower drum, slower wow) is the machine's design; the rate is chosen. |
| **A-3** | Flutter depth 0.8 ms at the fastest setting, growing toward ≈ 3.17 ms at the slowest | A constant physical wobble is a bigger share of a longer delay window as the tape slows; the magnitude is chosen. |
| **A-4** | The loop reaches unity gain at ≈ 0.75 of Recount and the model stops at 1.15×, with a feedback-path soft limiter standing in for the machine's clipping | Self-oscillation at high Recount is common description; the threshold is this model's. |
| **A-5** | The repeat band is one low-pass (300 Hz–12 kHz, exponential, under Timbre) over one fixed 90 Hz high-pass, both in the feedback path | The machine's tone stage is a real circuit; one-pole filters are this model's shorthand for it. |
| **A-6** | The front-end curve `x·(1+g)/(1+g·\|x\|)` with `g = Color² × 6`; mic trim up to ×2, guitar trim up to ×1 | The tube stage softening and soft-clipping when driven is the reputation; the curve and the trim spans are chosen. |
| **A-7** | One plugin input bus feeding both trims, as one source run into both jacks would | The machine has two jacks that mix inside it; a plugin has one bus. The trims therefore set *drive*, which is what they do on the panel. |
| **A-8** | The wet taper `Echo Volume² × 1.2` | A perceptually even knob with a little more level at the top. The dry beside it is machine behaviour, not an assumption: it never fades out. |

## Controls

Seven parameters — exactly what the machine has knobs for, plus one honest
addition:

| Parameter | Range | What it does |
|-----------|-------|--------------|
| `mic_level` "Mic Level" | float 0–1 | The high-gain input trim: drive into the tube front end. Scales up to ×2 (A-6, A-7). |
| `gtr_level` "Gtr Level" | float 0–1 | The instrument input trim. Scales up to ×1 (A-6, A-7). |
| `color` "Color" | float 0–1 | **Not a panel control.** This model's knob for how hard the tube stage is driven — the machine only lets you reach that by overlevelling the input. Zero is clean; more softens the peaks (A-6). |
| `delay_time` "Time" | float 0–1 | Tape speed: slower = longer. The knob is linear; the seconds it means are exponential (A-1), and slowing it deepens and slows the wow (A-2, A-3). |
| `recount` "Recount" | float 0–1 | Repeat count; the loop passes unity near three quarters of the travel (A-4) and past there it runs away, finite, as a real loop does. |
| `timbre` "Timbre" | float 0–1 | The tone of the repeats — the low-pass every pass through the loop goes through (A-5). |
| `echo_volume` "Volume" | float 0–1 | The level of the repeats against the dry, **which is always present**. The machine has no wet/dry crossfade and no dry-kill, and neither has this model (A-8). |

### Where the controls sit on a fresh instance

The input trims (0.25 mic, 0.5 guitar) sum to unity through the mixer at
full scale (0.25 × 2 + 0.5 × 1 = 1, A-6), so a hot input arrives at the
tube stage at its clean edge — not clipped, not timid. Color 0.2 for the
same reason: audible warmth, no distortion a player did not ask for. Delay
Time 0.5 centres the exponential range at its geometric mean, ≈ 350 ms —
"medium" on a machine whose whole charm is that its delays live where tape
lives. Recount 0.4 sits deliberately under the A-4 crossover: a first
listen should hear repeats *decay*, and the howling quarter of the knob
stays for whoever reaches for it. Timbre 0.5, Echo Volume 0.5. There is no
factory preset for a 1964 tape echo; every number here is this model's
taste, and a test pins them so the plugin and the model can never disagree
about the factory sound.

## Build

At the workspace root:

```sh
cargo xtask bundle nice-ecomplex --release
```

That is the bundling command — `cargo build` does not produce a plugin. It
lands `Ecomplex.clap` and `Ecomplex.vst3` in `target/bundled/`, with the
font licences inside the VST3 and beside the CLAP (a CLAP is deliberately
one file, so its licence text travels next to it). No `RUSTFLAGS` are
needed: nothing in this crate's graph refuses rustc's default x86-64
baseline. On Windows, the workspace `.cargo/config.toml` adds `+crt-static`
to the MSVC target, so build a distributable with `$env:RUSTFLAGS = ""`
first and load it in a host before trusting it either way.

## Standalone

```sh
cargo run -p nice-ecomplex --features standalone
cargo run -p nice-ecomplex --features standalone -- --list-devices
```

On Windows in a confined sandbox one more variable is needed: a `TEMP`/`TMP`
inside the tree for the linker. Nothing else. ASIO is `azo`, which declares the
ASIO COM interfaces in Rust, so there is no SDK to unpack and no
`CPAL_ASIO_DIR` to point at one — `.cargo/config.toml` says why the repository
sets nothing.

The binary is an app-owned host — nice-plug's own standalone wrapper is
neither built nor patched. The device transport is `nice_common::audio_io`,
the same opener the rack runs (ASIO on Windows, cpal's hosts elsewhere), and
the audio settings page behind the corner cog is the same shared panel
nice-nam, nice-tuner, nice-wah, nice-phase, nice-pi-face, nice-echo and the
rack show, so device names, rates and buffer choices mean one thing across the
workspace. `--list-devices` prints what the current backend sees and exits.

Settings are `nice-ecomplex-settings.ron`: the audio choices, the window
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
cargo test -p nice-ecomplex
cargo test -p nice-ecomplex --features standalone
```

The interface is what the scaffold could promise before there was sound,
and it still does: that the host sees exactly the seven parameters
promised, that the factory state is the resting place `dsp` itself names,
that publishing leaves the cells saying what the transport runs on, that a
saved move seeds the audio thread and a control never moved keeps the
plugin's own default, that the settings file round-trips, that the meter
counts channels and samples in the right order, that every control has a
caption, a tip and a slot of its own and that the view reaches every
parameter the host sees, that the layout stays inside the window at every
size, that a block and a sweep of every control cost the allocator nothing
(`tests/no_alloc.rs`, which links without nice-plug for that reason and
carries no `cfg`, so it runs on Windows too).

The tape path is asked with rulers rather than tuned by ear and then
described — measured arrivals, measured spectra, measured loop gain —
against arithmetic the model has no standing to dispute:

* the Delay Time knob spans exactly the A-1 range on the exponential
  taper, to the millisecond the assumption names;
* slower tape wobbles deeper *and* more slowly — A-2 and A-3 measured at
  both ends of the knob, one drum under both numbers;
* a burst arrives once, after exactly the selected delay, with nothing in
  front of it;
* **Echo Volume at zero leaves the dry bit-identical** — the dry path is
  always present, and a test says so at the bit level, because "there is
  no dry-kill on this machine" is a fact, not a taste;
* past the A-4 crossover the loop runs away but stays finite — the soft
  limiter stands in for the machine's clipping, and nothing reaches `inf`
  or `NaN` at any knob combination;
* Color softens a hot input and passes small signals clean (A-6), and a
  darker Timbre measurably thins the repeats (A-5 as behaviour);
* the fixed 90 Hz high-pass is a tilt, not a mute — low frequencies come
  back around, they do not vanish;
* one tape is heard in both lanes and the dry keeps its own lane: a stereo
  input shares one drum but never merges its dry.

What none of that tests is the eight named assumptions — the range, the
drum's rate, the flutter's depth, the crossover, the tone stage's
shorthand, the front-end curve and its trim spans, the one-bus reading of
two jacks, the wet taper. No test can check an assumption against a
document that does not state it; those belong to `TODO.md`, and they are
what an afternoon with the machine and a bench would retire.
