# Nice Phase

A phase shifter in the shape of the pedal that named the effect: four
first-order allpass sections chained, a sine wave walking their corner
frequency up and down an octave-and-a-half of logarithmic rail, and the
shifted copy rejoined with the dry at the summer's equal split — the place
where four sections at −360° cancel two holes into the mids and sail
everything else through untouched. Five named settings choose what the
chain is wired as: four sections or two, and whether the block era's
feedback resistor closes the loop. The notches are not fitted curves;
their arithmetic is exact, and the test suite measures them to it.

The plugin and the standalone application are one codebase: the same
`Plugin::process` and the same editor run in both, as in nice-wah,
nice-nam and nice-tuner. Only the answer to "who owns the audio and the
numbers" changes.

## Build

At the workspace root:

```sh
cargo xtask bundle nice-phase --release
```

Bundles land in `target/bundled/` as `Nice Phase.clap` and `Nice Phase.vst3`.
No `RUSTFLAGS` are needed: like nice-wah, nothing in this crate's graph
refuses rustc's default x86-64 baseline. On Windows, the workspace
`.cargo/config.toml` adds `+crt-static` to the MSVC target — a plugin loaded
into someone's host generally should not carry its own C runtime, so build a
distributable there with `$env:RUSTFLAGS = ""` first, and load it in a host
before trusting it.

Rust 1.99+ (edition 2024).

## Standalone

```sh
cargo run -p nice-phase --features standalone
cargo run -p nice-phase --features standalone -- --list-devices
```

The binary is an app-owned host: nice-plug's own standalone wrapper is
neither built nor patched. The device transport is `nice_common::audio_io`
— ASIO on Windows, cpal's hosts and JACK elsewhere, the same opener the rack
runs — and the audio settings page behind the corner cog is the same panel
nice-nam, nice-tuner, nice-wah and the rack show, so device names, rates and
buffer choices mean one thing across the workspace. `--list-devices` prints what the current
backend sees and exits, without a window and without opening anything.

Settings are `nice-phase-settings.ron` in the working directory: the audio
choices, the window geometry, and every knob the player last left the
filter on. A refused device change rolls the app back to the last pair of
streams that opened and says so in the view; the `DROPS` counter under the
cog counts capture-side overruns.

Without a host the editor owns the parameters: nice-plug's `FloatParam` has
no public setter outside a host's `GuiContext`, so in standalone the shared
cells the audio thread reads *are* the truth, written straight from drags,
wheel notches and the five mode buttons. In a host the same writes ride
`ParamSetter`, and a mirror keeps the cells a frame behind the parameters,
so automation stays the authority and the filter still reads one place.

## The controls

| Parameter | Range | What it is |
| --- | --- | --- |
| SPEED | 0.05–20 Hz | The sine's rate. At 0 — its bottom — the sweep freezes wherever it stood. |
| MODE | 5 positions | The wiring: SCRIPT 90, SCRIPT 45, BLOCK 90, BLOCK 45, SIGNATURE 90. |
| DEPTH | 0–100% | How far the summer leans toward the chain. At 0 the dry passes bit for bit. |
| MIX | 0–100% | The blend after the phaser. At 0 the output is flat and unity. |
| OUT | ±12 dB | Output trim, after everything. |

Every one is a plain parameter: automation, modulation and the editor all
read and write the same numbers, and the view is redrawn from the
parameters every frame, so host automation visibly moves the knobs. The
MODE knob snaps to whole settings under the drag and the wheel; the rail
between the meters shows where the sweep is, where its two cancellation
holes actually sit right now (the green ticks), and the rail's rail ends.

## The five settings

All five share one sweep — 100 Hz to 1.6 kHz of stage frequency. No
setting plate of the machine this models ever printed a different range;
the published analyses agree, and a table that disagreed would be inventing
hardware.

| Setting | Sections | Feedback | What it chases |
| --- | --- | --- | --- |
| SCRIPT 90 | 4 | off | The mellow original: two open notches, all headroom. |
| SCRIPT 45 | 2 | off | Half the chain: one wide hole, sailing the mids. |
| BLOCK 90 | 4 | on | The later-era whoosh: the same two holes, ringing. |
| BLOCK 45 | 2 | on | The single hole filled to a singing dip instead of a null. |
| SIGNATURE 90 | 4 | off | The two-switch superset pedal, parked at the corner its namesake is known for. |

The names are ours. SCRIPT and BLOCK name the two enclosure-silkscreen
eras of the circuit by their typeface, the way players have always named
them to each other; SIGNATURE stands where a signature model's name would,
because a player's name is the one trademark this table has no business
wearing. No manufacturer makes or endorsed anything here.

`SIGNATURE 90` is honest about being a reading: the real two-switch pedal
contains the other four settings' wiring as its switch positions, and a
table row can only be one corner of that, so this one is SCRIPT 90's
numbers with a name on it — see `DECISIONS.md`.

Every number in the table that *could* be measured from published circuit
analyses is: the two-notches-per-four-sections physics is exact, the
'45's one-notch-per-two-sections follows from the same arithmetic, and the
feedback's effect at the stage frequency has a closed form the tests assert
to the thousandth. The numbers nobody ever published are labelled where
they sit: the 100–1.6 kHz ends are a modelled guess at "the low end of the
travel, the top short of an air band" (a common mid-70s resistor value
places the notches' rest position near 58 Hz and 340 Hz, which the bottom
end of our rail matches), and the feedback ratio 0.25 is taste, not
measurement. `TODO.md` keeps the ledger.

## How it sounds, mechanically

Each section is the analog phase splitter's exact digital twin:
`H = (a + z⁻¹)/(1 + a z⁻¹)`, `a = (t − 1)/(t + 1)`, `t = tan(π fc/fs)` —
a bilinear-warped first-order allpass, constant unity gain at every
frequency, its phase at warped frequency g being −2·atan(g/t). Four of
them sum their phase, and where the chain totals −180° or −540° — at
`fc · tan 22.5°` and `fc · tan 67.5°` in warped frequency — the summer's
two halves meet out of phase and cancel. Two sections cover only −180°
once, at the stage frequency itself. `a` never leaves the unit circle for
any positive fc, so the filter cannot ring without the feedback wire, and
the feedback wire itself closes through a one-sample register, which caps
its loop gain under 1: it cannot run away either.

The mode parameter is continuous underneath its five names, and a flip
crossfades stages *and* feedback on the same 20 ms slew — the 2↔4 move
blends a two-stage chain's output with a four-stage chain's, sample by
sample, which is click-free because both sides are unity-gain at every
frequency. Coefficients are computed per block and blended per sample;
nothing inside the sample loop touches an atomic, and
`tests/no_alloc.rs` proves the allocator never sees a block, an
automation move, or a mode flip.

The feedback wire samples the block-era resistor's *function*, not its
literal wiring: the wet chain's output, one sample late, joins the
network's input. Closed inside the wet path, it leaves both invariants
whole — DEPTH 0 is the dry signal bit for bit and MIX 0 is flat whatever
the mode. At the stage frequency four sections arrive back in phase, so
there the loop is positive and blooms to 0.5 + 0.5/(1−fb) ≈ +1.3 dB at
fb = 0.25; two sections arrive inverted, so the loop is negative and the
notch fills to exactly 0.5·fb/(1+fb) = 0.1. Both numbers are in the test
suite, and both are the reason the block settings ring without whistling.

## Tests

```sh
cargo test -p nice-phase
cargo test -p nice-phase --features standalone
```

The filter tests ask acoustics: a Goertzel bin answers "how loud does a
pure tone come out" exactly, and the assertions are the circuit's identity
— the two notches at their tan-ratios for four stages, the one notch at
the stage frequency itself for two, unity at the stage frequency for four,
the sweep walking a fixed probe from open air to null, each setting's gain
at its own stage frequency matching its topology's closed form (trim for
the open fours, 1.167 for the ringed fours, null for the open twos, 0.1
for the ringed two). `tests/no_alloc.rs` is the audio thread's witness; it
deliberately links without nice-plug, whose debug allocator and the
counter's allocator cannot share a binary. The standalone tests need no
hardware: the pump moves ring buffers through a real plugin instance, the
settings file round-trips, and the dummy backend resolves to exactly the
silence it promises.
