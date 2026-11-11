# Nice Pi-Fase

A model of the **Mu-Tron Bi-Phase** as a CLAP/VST3 plugin: two phasor lanes of
six all-pass sections each — "two independent six-stage phase shifters", as the
owner's manual puts it — two sweep generators that are free to drift against
each other, and the switching the original has: Shape, Sweep source, Sweep Sync,
and the Phasor B Input switch, whose three positions *are* the machine's three
basic modes of operation. The name is the user's spelling, *Pi-Fase*, chosen to
clear any trademark; the thing it models is the machine documented below.

**Current state: the chassis, not the sound.** The workspace member, the
manifest and features, the parameter set, the CLAP and VST3 exports, the frost
editor and the standalone host are all in place and compile; `dsp.rs` passes
audio through and says so on screen. No filter, lamp or oscillator exists yet, so
nothing in this crate should be quoted as a measurement of anything. The
repository rule is *measure, never speculate*, and a pass-through at least claims
nothing.

Like its siblings, plugin and application are one codebase: the same
`Plugin::process` and the same editor run in both. Only the answer to "who owns
the audio and the numbers" changes.

## What the original is

From the **owner's manual** (the complete six-page scan: the control
descriptions, three wiring figures with the panel drawn for each, and the
specification table) and from the service documents (drawing 77204 sheet D
"SCHEMATIC, BD1 BI-PHASOR", drawing 77206 sheet B for the mains supply, and the
hand-drawn "A PHASE SHIFT" / "B PHASE SHIFT" page). Where the two disagree the
manual wins on what a control does and the schematic wins on what a component
is; the headroom figures at the end of the list are the only ones that came from
neither and they say so. These are scans of hand-drawn and typewritten
documents, so a designator or value may be misread; each number below is worth
checking against the image before a release depends on it.

* **Six first-order all-pass sections per lane, not four.** Lane A is op-amps
  1A,1B,2A,2B,3A,3B; lane B is 4A..6B. Each: 0.0068 µF coupling (10%), 47K
  series, 220K feedback resistor in parallel with an incandescent lamp
  (PH3A..PH3F on lane A, PH4A..PH4F on lane B), .47 coupling out.
* **The variable element is a lamp, not a JFET.** The sweep reaches the lamps
  through a voltage-to-current converter (Q1/Q2, C1852, R98 220K, CR16/CR17
  1N4148, PM3/PM4). Filament temperature lags the drive current; that lag is what
  makes the sweep smooth and is a large part of the pedal's compression. The
  model will represent it as a first-order lag on the *control* signal, with its
  corner well below the LFO's top rate, and will name it as the approximation it
  is rather than tune it until it sounds right.
* **Two sweep generators, one per lane.** Lane A from 21A/21B (C33 .47,
  R70/R71/R72 shape network); lane B from 2HA/2HB (C34 .47, R90/R91/R94). S5/S6
  are SWEEP, S7 is SWEEP SYNC NORM/REV, S3/S4 are SHAPE, and S2 selects RATE
  CONTROL MAN/FXD with the pedal at J6-2. Two oscillators rather than one means
  the lanes drift, and that beating is where the wide sweeps come from — so the
  model keeps two.
* **Two independent paths, and a switch between them.** Input A and Input B;
  output A at J3 and output B at J4, each through its own 29B/211A stage with
  PH1/PH2 lamps. S1 selects which of three signals — input B, input A, or phasor
  A's own output — is applied to the input of phasor B. Stereo or dual-mono falls
  out of the circuit. The panel prints `IN A`, `OUT A`, `IN B` round that one
  rotary and the manual calls its positions "the three basic modes of operation"
  — see below.
* **Two footswitches, not one.** "Two foot switches are provided in a single
  housing to allow each of the two phasor sections to be turned on and off
  individually", each phasor with an LED to show it is running. That bypass is
  per phasor, and the plugin has no control for it yet: recorded in `TODO.md` as
  a gap rather than quietly dropped. The manual's note that "the phasor circuits
  will not operate unless the foot switch unit … is plugged in via the back panel
  connector" is a quirk of the hardware a plugin does not inherit.
* **Specification**, as the manual's table prints it: input impedance 180K ohms
  (Parallel Mode), 390K ohms (Single Mode), minimum; output impedance 600 ohms,
  unbalanced; insertion loss less than 1 dB; frequency response (bypass) 20 Hz to
  20 KHz, ± 1 dB; signal-to-noise typically 86 db below a 2 V input; power
  requirements 115 VAC, 60 Hz, 15 W. The phasing rate, .1 Hz to 18 Hz, is in the
  Rate Control description above rather than in this table.
* **Headroom** — the one figure here that neither document carries: 4 V RMS
  through one phasor with no feedback, 2 V with both in series at 50% feedback,
  1 V at 100%. Those are mu-tron.com's numbers for the machine, kept because
  nothing in the manual says what feedback does to level, and marked as coming
  from outside this pile of paper.

### The other sheet in the pile

The documents also contain a **"BI-PHASER" sheet: 4558 op-amps, BF245A JFETs,
47nF and 10k, four stages, dual 9–18 V, with a note about selecting FETs with
matching characteristics**. That is a JFET-based clone of the idea, **not the ARP
original**. It is a useful simpler reference and a wrong source of truth. If a
number arrives from a four-stage JFET circuit, it came from that sheet and needs
re-deriving.

## Controls

The parameter ids are the wire format of every automation a host writes, so a
test asserts the list and a rename is a deliberate act.

| Parameter | Range | The original's control |
| --- | --- | --- |
| `gen1_rate` | 0.1–18 Hz | Sweep Generator 1 Rate |
| `gen1_shape` | Sine / Square | S3 |
| `gen1_rate_source` | Manual / Pedal | S2 (RATE CONTROL) |
| `gen2_rate` | 0.1–18 Hz | Sweep Generator 2 Rate |
| `gen2_shape` | Sine / Square | S4 |
| `a_depth` | 0–100% | Phasor A Depth |
| `a_feedback` | 0–100% | Phasor A Feedback |
| `a_sweep` | Gen 1 / Pedal | S5 (PHASOR A SWEEP) |
| `b_depth` | 0–100% | Phasor B Depth |
| `b_feedback` | 0–100% | Phasor B Feedback |
| `b_sync` | Norm / Rev | S7 (SWEEP SYNC) |
| `b_sweep` | Gen 1 / Gen 2 / Pedal | S6 (PHASOR B SWEEP) |
| `b_input` | In A / Out A / In B | S1 (PHASOR B INPUT) — the mode selector |
| `pedal` | 0–100% | The Mu-Tron C-100 control pedal, at J6-2 |

Fourteen parameters, one per control on the panel that a plugin can have. What is
missing is deliberate: the two per-phasor bypass footswitches, which the plugin
does not model yet — `TODO.md` carries that as a gap. The S-numbers are the
service documents'; the panel prints the words.

`pedal` is the plugin's port of a jack: the hardware reads a CV, a plugin has no
jack, so this is the parameter a host automates or maps a controller onto. On the
machine one pedal jack serves two jobs — the rate of generator 1, or the sweep of
a phasor — and the switches decide which; the parameter feeds all of them, which
is the simplification `TODO.md` names.

Feedback reaches 100%, because the machine does: the manual describes what full
feedback sounds like ("the Bi-Phase will pick out the individual notes and
harmonics of a chord as it sweeps up and down") and the schematic has no diode
stopping at ninety-five. Whether the model behaves there is the filter's problem,
not a reason to shorten the control.

### Where the controls sit on a fresh instance

The manual has a starting configuration — "Start with this" — and the plugin's
defaults are that, plus the one switch change the manual itself names:

| Control | Default | Why |
| --- | --- | --- |
| Rate Control | Manual | "Set Sweep Generator 1 'Rate Control' switch on 'Man'" |
| Both Shapes | Sine | "Set both 'Shape' switches for sine-wave sweep" |
| Phasor A Sweep | Gen 1 | "Set Phasor A 'Sweep' switch on 'Gen 1'" |
| Phasor B Input | Out A | "…on 'Out A'", the manual's normal setting for one instrument and one amplifier |
| Sweep Sync | Norm | The position its double-depth figure draws |
| Phasor B Sweep | Gen 2 | "For two-speed phasing, the only different setting is…" |
| Both Depths | 80% | "Both Depth controls should be set equally"; "the smoothest-sounding phasing is found at about 8" |
| Both Feedbacks | 0 | "Start with both 'Feedback' controls turned down" |

Choosing two-speed over double-depth as the factory state is the one judgement in
that table; every position in it is quoted. The two rates are the weakest numbers
in the crate: the manual's starter recipe says generator 1 at "a slow speed (1 to
3 on the dial)" and generator 2 "moderately fast (about 6)", the dial is numbered
1 to 10, and no mapping from dial to hertz is published anywhere. Reading those
as 0.3 Hz and 2.0 Hz needs a taper, and the taper — exponential across the .1–18
Hz range — is an assumption, labelled as one in `src/lib.rs` where the defaults
are written.

## The three modes of operation

There is no Mode control on a Bi-Phase, and the plugin has none. The machine's
three basic modes of operation are the three positions of one rotary switch:

> **PHASOR B INPUT SWITCH:** This switch effectively selects one of the three
> basic modes of operation by selecting which of three signals is applied to the
> input of Phasor B.

* **`In A`** — the one input at Input A is applied to each phasor, "as if you were
  using a Y-cord": the manual's normal setting for **stereo** effects, and the one
  its stereo figure draws (with Sync on Rev, both Depths at 10 and both Feedbacks
  at "8 or so").
* **`Out A`** — phasor A's output feeds phasor B, in series: "a 12-stage phase
  shifter", which independently confirms the six stages per lane the schematic
  shows. "The normal setting for using the Bi-Phase with one instrument and one
  amplifier", and the plugin's default.
* **`In B`** — phasor B takes its own jack: a second instrument, or a stereo one,
  run through two phasors that need nothing from each other.

The three wiring figures in the manual are these three positions, and each of
them is a test the filter will eventually have to pass. What the earlier scaffold
called "Model I" is simply this machine, and `dsp::Routing` is a reading of
`b_input`, not a parameter of its own — which is also why the parameter count went
from fifteen to fourteen: a Mode control that duplicates a switch that already
selects the modes is one control too many, and the I/II/III numbering belongs to
the Mu-Tron III, a different pedal.

## Build

At the workspace root:

```sh
cargo xtask bundle nice-pi-face --release
```

That is the bundling command — `cargo build` does not produce a plugin. It lands
`Nice Pi-Fase.clap` and `Nice Pi-Fase.vst3` in `target/bundled/`, with the font
licences inside the VST3 and beside the CLAP (a CLAP is deliberately one file, so
its licence text travels next to it). No `RUSTFLAGS` are needed: unlike nice-nam,
nothing in this crate's graph refuses rustc's default x86-64 baseline. On
Windows, the workspace `.cargo/config.toml` adds `+crt-static` to the MSVC target,
so build a distributable with `$env:RUSTFLAGS = ""` first and load it in a host
before trusting it either way.

## Standalone

```sh
cargo run -p nice-pi-face --features standalone
cargo run -p nice-pi-face --features standalone -- --list-devices
```

On Windows in a confined sandbox two more variables are needed: `CPAL_ASIO_DIR`
pointing at an unpacked ASIO SDK (a relocated `TEMP` defeats asio-sys's own SDK
lookup), and a `TEMP`/`TMP` inside the tree for the linker.

The binary is an app-owned host — nice-plug's own standalone wrapper is neither
built nor patched. The device transport is `nice_common::audio_io`, the same
cpal, JACK and ASIO-guarded opener the rack runs, and the audio settings page
behind the corner cog is the same shared panel nice-nam, nice-tuner, nice-wah and
the rack show, so device names, rates and buffer choices mean one thing across
the workspace. `--list-devices` prints what the current backend sees and exits.

Settings are `nice-pi-face-settings.ron`: the audio choices, the window geometry,
and every control the player last moved — `None` for a control never touched, so
the file records moves rather than restating the plugin. A refused device change
rolls back to the last pair of streams that opened and says so in the view; the
`DROPS` counter under the cog counts capture-side overruns.

Without a host the editor owns the parameters: nice-plug gives a parameter no
public setter outside a host's `GuiContext`, so in standalone the shared cells the
audio thread reads *are* the truth. In a host the same writes ride `ParamSetter`
and `process()` mirrors the parameters into the cells each block, so automation
stays the authority and the filter still reads one place.

## Tests

```sh
cargo test -p nice-pi-face
cargo test -p nice-pi-face --features standalone
```

What is testable before there is a filter is the interface: that the host sees
exactly the fourteen parameters promised, that every switch offers exactly the
positions its labels claim, that the factory state is a configuration the manual
actually describes, that a saved move seeds the audio thread and a control
never moved keeps the plugin's own default, that the settings file round-trips,
that the pump moves frames through a real plugin instance without adding or
losing anything, and that a block and an automation move cost the allocator
nothing (`tests/no_alloc.rs`, which links without nice-plug for that reason).

None of that is a test of a phaser. The acoustics arrive with the DSP, and they
will be asked with rulers — a Goertzel bin, in nice-wah's manner — rather than
tuned by ear and then described.
