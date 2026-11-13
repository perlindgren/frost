# nice-echo — what is left

The tape path is in: a transport whose speed is the Repeat Rate, one record
head and three playback heads with distance-dependent HF loss, a saturating
record path the Intensity knob walks to self-oscillation, a spring tank in
parallel, the tone shelves, the three service-manual output trims, and the
twelve-position mode selector driving all of it — with a measured test for
each claim and an allocation witness over every mode and every control. The
member, the manifest and features, the nine parameters, the CLAP and VST3
exports, the frost editor with its mode knob, the standalone host on
`nice-common`'s transport and settings panel, and the gate and hook entries
are all in place. What is left is the twelve named assumptions the
documents cannot settle — A-1 through A-12, catalogued in the README —
and the things a machine on a bench would retire: the hiss and wear
magnitudes, the driven record corner, the second lane, the footswitch,
and a taper.

## 1. Decided, and where the answer lives

- **What one tape means for a stereo plugin input: one echo, heard twice.**
  The hardware has one tape; the plugin has two lanes. Both lanes read the
  same taps, because a mono-in, mono-out tape machine with a mono output
  stage is what the sources describe, and `one_tape_is_heard_in_both_lanes`
  pins the behaviour so the simplification cannot quietly become a bug.
  Real RE-201s *can* be coaxed into stereo (two record paths on some
  units' wiring, and players splice stereo heads in), which is §2's second
  item — but the documented machine is one tape.
- **The panel number is the integer plus one.** The host's integer range is
  0–11, the labels are "1"…"12", and the test that pins it is
  `the_mode_dial_shows_the_panels_numbers` — because a mode table that
  drifts one place off silently rewires the whole machine.
- **Reverb Volume is live exactly where the walk-through says it is.**
  Positions 5–12 (the spring-carrying ones, "between 5 and 11 and reverb
  only" plus the reverb-only position itself); elsewhere the knob moves and
  nothing else happens, which is the machine's behaviour, not a bug to
  "fix" by letting the knob do something in mode 2.
- **The output trims enter as relative steps.** −8 and −20 dB from H, the
  service notes' intervals; the absolute dBm ratings describe the jack into
  its rated load and are cited in the README, not modelled.

Still to decide:

- **Whether Mode should expose the heads and the spring as separate
  switches alongside the knob.** The twelve-position table (A-4) is one
  integer because the machine has one selector, and a host automating an
  integer automates fine. But a player who wants "heads 1+3 without the
  spring" has to hunt the knob for a position that does not exist — the
  wiring was chosen, not read. Leaving it is a defensible fidelity call;
  it should be *made* rather than inherited. The caption under the knob
  softens this rather than solving it: it says what a position routes, so
  the hunting at least reads honestly.

## 2. The DSP

Done, with the test that says so:

- [x] The transport: tape speed is the Repeat Rate, slew-limited, and
      turning the knob smears the delay on the way instead of jumping
      (`turning_the_repeat_rate_smears_the_delay_on_the_way`).
- [x] Three arrivals at three distances, and silence at the unselected
      heads; the slowest taps on the A-1 anchors, the span on the A-3
      ratio (`a_burst_walks_to_the_selected_head_and_back`,
      `all_three_heads_hear_three_arrivals`, `slower_tape_is_longer_delay`).
- [x] Heads duller the farther the tape travels (A-8 as measured spectra,
      `heads_get_duller_the_farther_the_tape_travels`), two-pole
      and stable near Nyquist.
- [x] The saturating record path: odd, bounded, and the loop never leaves
      full scale however Intensity is ridden (`saturation_is_odd_and_bounded`).
- [x] Self-oscillation where the sources promise it, near the A-10
      crossover: full Intensity outlives the input (`full_intensity_outlives_the_input`),
      and Intensity adds repeats monotonically below that.
- [x] The spring: smears, bounded, alone in position 12, in parallel where
      the mode table says (`the_spring_smears_without_escaping_its_bound`,
      `reverb_only_mode_is_the_spring_alone`); waits out its pre-delay
      before the first arrival, and rings down treble-first
      (`the_spring_lags_before_it_rings`,
      `the_spring_rings_down_bright_first`).
- [x] The tone stack a wire through when centred — bit-identical — and
      riding highs and lows where the knobs say (the four tone tests).
- [x] The wow and flutter inside their A-5 depths — widened once for the
      wear wander (A-11) — and never wider
      (`the_transport_wobbles_a_little_and_no_more`), and the free-loop
      argument for why the depths are modest is in the header, not folklore.
- [x] The transport wanders on a minutes timescale, shared between the
      lanes because one tape wanders once, bending tap spacing, wow depth
      and HF loss together inside a bound
      (`the_transport_wanders_slowly_and_stays_bounded`), and the hiss
      breathes with the wow (A-9, `the_hiss_breathes_with_the_wow`).
- [x] The record path loses its top as it is driven (A-12): the driven
      corner walks toward A-12's floor with a slew of tens of
      milliseconds, measured about −3 dB of treble on a pushed input
      against the clean run (`the_record_path_loses_top_when_driven`).
- [x] The output steps exactly the service notes' 8 and 20 dB
      (`output_steps_are_the_service_notes_numbers`).
- [x] The mode table the README ships is the table the DSP runs, spring
      admitted from panel 5, out-of-range automation clamped
      (`the_mode_table_is_the_table_the_readme_ships`).
- [x] Zero allocations per block and per control sweep, in a test file that
      links without nice-plug and so runs on Windows too
      (`tests/no_alloc.rs`).

Left:

- [ ] **The tank's remaining realism (A-6).** The comb garden landed:
      input low-pass, short pre-delay, four parallel combs of unequal
      length each low-passing its own feedback, summed into a tail
      low-pass, and both measurable shadows are tested (Done above,
      `the_spring_lags_before_it_rings`,
      `the_spring_rings_down_bright_first`). The all-pass sections the
      first tank inherited from nice-phase are retired from this crate;
      that is the history this line carries. What a bench would still
      retire: the comb lengths and gains are named (A-6), not measured;
      multiple taps per delay line and the growl that tracks input
      level are not modelled.
- [ ] **One tape in both lanes (§1).** The cheap stereo read is two record
      paths a few milliseconds apart writing two tapes — which is what
      players report dual-head-wired machines do — but no source in the
      pile documents a stereo RE-201, so it enters as another labelled
      assumption, not a "fix".
- [ ] **The hiss floor (A-9) is a stability measure wearing a sound's
      clothes.** Its magnitude — ±1.5e-4 — exists first to kill the
      digital loop's silence fixed point and has never been compared
      with a tape transfer's measured noise. The breathing half has
      landed (the floor rides the wow, Done above); the number itself
      waits for a transfer.
- [ ] **The echo-switch crossfade is a hard read, not a transition.** The
      machine's switch is a switch; fine. But the A-7 reading — NORMAL is
      dry + echo, ECHO is the echoed input alone — should be checked
      against a recording of the real switch mid-throw, because "100 %
      echo" from the walk-through could be the *wiper* position with a
      crossfade either side, and the test would have to move with it.
- [ ] **Editor readouts, like the rack's.** `EchoParams` already publishes
  nothing observable; the loop gain near unity and the oscillation state
      are the two numbers a player would pay to see (`env_level`-class
      readouts published from the block loop, the meter's manner).

## 3. The face

The editor is the family's: dials on grid-first slots, tooltips, the mode
knob that detents through the twelve positions and names what the selected
one routes, the meter row, the cog. What the GUI phase owes it:

- [x] **The mode control should read as a selector, not twelve buttons.** It
  is a stepped rotary: twelve marks on the family's 270° sweep with the
  current one lit, a pointer reading the same cell the DSP reads, turned by
  drag or by wheel — one detent a notch, the ends stopping rather than
  wrapping — with the panel number and a caption built from the mixer's own
  mask stacked under it. A release that never turned the knob says nothing,
  so a stray click cannot move the machine.
- [ ] **A dead Reverb Volume the eye can see.**
  The behaviour is right and tested; the panel currently gives no hint
  that the knob is inert in modes 1–4, which reads as a broken knob to
  anyone coming from the machine. Dimming, not disabling — the machine's
  knob does turn.
- [ ] **A wow-and-flutter readout, or the tape drawn moving.**
  `Echo`'s transport already knows its instantaneous speed; the face
  has never been told. Nice Pi-Face owes its sweep the same admission.

## 4. The assumption still in the defaults

Repeat Rate 0.45 is the weakest number on the panel, and `lib.rs` says so
where the default is written: the sources give the direction of travel
(slower tape, longer delay) and never a taper, so 0.45 is "medium delay"
read off a linear span of the A-3 speed range — a *position*, not a
measurement. Anyone with a real machine's tap-time-vs-knob curve should
replace the number with the curve and the comment with the measurement;
the A-1/A-3 tests pin the endpoints, so only the middle of the travel
moves, which is exactly the part that was guessed.

Intensity 0.4 sits deliberately *below* the A-10 crossover — the factory
sound repeats and decays rather than howling. That is a choice about what
a first listen should demonstrate, not a claim about the machine's noon
position, and it is commented as such beside the default.

## 5. Loose ends outside this directory

- The gate legs (`.githooks/pre-commit`, `.githooks/pre-push`,
  `.github/workflows/gate.yml`) deliberately run nice-echo clippy *without*
  `--features standalone`: the standalone lane pulls the ASIO host behind
  `cfg(windows)` and the Linux gate cannot exercise that path, so the
  standalone clippy leg is run locally before any standalone change, and
  the comments in both files say so. If a future CI matrix grows a Windows
  runner, that leg belongs there and the local-workaround comments should
  be deleted rather than left as archaeology.
- The VST3 class id `FrostNiceEcho001` is 16 bytes of our own making (the
  family's pattern; nice-pi-face owns `FrostNicePiFace001`). Nothing
  outside this crate can collide with it today; if a registry of Frost
  class ids ever exists, nice-echo's entry is this line.
- `target/bundled/` accumulates bundles from every member's bundling run
  and the licence pass sweeps every bundle present — nice-pi-face's TODO
  carries the same warning about stale binaries wearing fresh licences.
  Check timestamps before copying the folder anywhere a host can see it.
- The gate now pins the Rust toolchain (`.github/workflows/gate.yml`
  explains why). This branch paid the lesson: rustc 1.100.0 shipped the
  day our first gate run ran, and its clippy caught six
  `needless_range_loop` sites in `antifrost`, `immortal` and
  `tools/sprite_util` — code byte-identical to `main`, so `main` would
  have failed its next run the same way. The fixes (iterator loops,
  no `#[allow]`) ride this branch only because the branch must pass the
  gate they unblock; they change no behaviour. Bumping the pin means
  installing the new toolchain locally, running the gate, and fixing
  what the new clippy says — in the commit that changes the number.
