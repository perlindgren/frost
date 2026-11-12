# nice-pi-face — what is left

The filter is in: two free-running generators, six lamp-swept all-pass sections a
lane, the lamp's lag on the control signal, the feedback loop, the three routings,
and the pedal's two jobs — with a unit test for each claim and an allocation witness
over all three modes. The member, the manifest and features, the fourteen
parameters, the CLAP and VST3 exports, the frost editor, the standalone host on
`nice-common`'s transport and settings panel, and the gate and hook entries are all
still what the scaffold put there. What is left is what the documents do not settle,
the two readings of the sheet that disagree with the model, the bypass the plugin
still does not have, and a face with no sweep to show.

## 1. Decided, and where the answer lives

- **What `In A` means for a stereo plugin input: summed.** The hardware's single
  input is one jack; the plugin's is two. Both lanes hear `(L+R)/2`, because that is
  what the manual's sentence says they hear — "the single input source at Input A is
  the input to both phasors" — and the dual-mono reading already has a name on the
  panel, `In B`. `summed` in `src/dsp.rs` carries the argument, and
  `one_summed_source_that_is_its_own_negative_reaches_no_lane` carries the cost: two
  sides that are exact negatives cancel, here as in a Y-cord.
- **Depth is travel from the bottom of the band**, not a span about a centre: zero
  leaves a lane sitting at `SWEEP_LO_HZ`, which is what makes a held pedal the held
  filter the manual describes rather than a slow one.
- **No Mode parameter.** The three basic modes of operation are the three positions
  of `b_input`, and `dsp::Routing` is a reading of that integer.

Still to decide:

- **What the mode selector looks like in the view.** There is nothing to decide
  about *whether* one exists: the three basic modes of operation are the three
  positions of the Phasor B Input switch, and `b_input` is that switch. What is
  open is presentation — the panel's word for the switch ("PHASOR B INPUT") is not
  the word a player uses for what it does, and a tooltip is carrying that
  explanation so far.
- **Whether phasor B's real feedback path is per-stage or per-branch, and where the
  series tap comes from.** Reading "A PHASE SHIFT": six cascade stages, branch 1 out
  of stage 3 to phasor B, branch 2 (the switchable feedback) taken from *after* the
  output stage 29B/211A and returned to stage 6's summing junction — not the
  textbook "output of the last all-pass back to the first". The model does take the
  textbook route: the tap is the end of the six-section chain and the return is the
  lane's summing node, and `Out A` feeds phasor B from phasor A's *end* rather than
  from stage 3 because the manual's own "12-stage phase shifter" is what the series
  position has to add up to. Both readings are on the sheet, and the sheet is a scan
  of a hand drawing: re-read it, then either move the tap or write down why the
  twelve-stage sentence wins. A diagram drawn from the textbook routing and offered
  as the sheet's own should be corrected rather than copied.

## 2. The DSP

Done, with the test that says so:

- [x] Six first-order all-pass sections a lane, flat wherever the sweep sits, and
      the coefficient inside the unit circle at every rate and position
      (`six_sections_a_lane_and_arithmetic_behind_the_floor`,
      `the_coefficient_always_lives_in_the_unit_circle`,
      `a_lane_without_feedback_is_flat_and_unity`).
- [x] The lamp: a first-order lag on the *control* signal, named in the code as the
      approximation it is. Its corner sits **above** the top sweep rate, not below —
      the premise of this item when it was written would have strangled the top third
      of a dial the manual calls the loudest.
- [x] Two generators (.1–18 Hz), sine and square, B reversible against A, and stayed
      two oscillators: `two_generators_beat_against_each_other` watches them meet and
      part, and `sync_normal_walks_together_and_reversed_walks_against` is the same
      argument on one source.
- [x] Depth and feedback slewed, and the feedback loop bounded at the top of a
      control that reaches 100% (`a_lane_at_full_feedback_stops_growing_instead_of_never_stopping`).
      The headroom is a knee at half scale with a ceiling at full scale, because the
      4 V/2 V/1 V figures have no volt-to-full-scale mapping to translate; the
      ceiling is applied at both ends of a lane, so nothing the switches reach leaves
      full scale.
- [x] The three routings, on both taps and in a mono insert
      (`in_a_puts_one_summed_source_into_both_phasors`,
      `out_a_hands_phasor_a_to_phasor_b_and_in_a_does_not`,
      `in_b_gives_phasor_b_a_input_of_its_own`).
- [x] Sync, exact or reversed, and the manual's reason for it: in `Rev` there is no
      double-depth phasing because the lanes move against each other.
- [x] Four of the manual's sentences in words, as tests: the held pedal that changes
      the timbre without sweeping, Rate Control on `Ped` at "a very low rate", the
      square sweep that jumps from one extreme to the other, and the closed forms for
      where six sections put their peaks and dips.
- [x] The pedal's double life, named in the code rather than left to be discovered:
      one `pedal` parameter, and the switches decide whether it is a rate or a sweep.
      The plugin cannot read a player's intention out of a CV, so it feeds every
      destination the hardware's switches can select.

Left:

- [ ] **Three assumptions, and no document to check them against.** `SWEEP_HI_HZ`
      (4 kHz, where a PH3A lamp's hot resistance should be), `LAMP_LAG_S` (4 ms,
      where a filament's thermal time constant should be) and `LINEAR_LEVEL` (half
      scale, where the rails should be). These are the crate's debts; each is named
      where it is defined, and none of them is tuned until it sounds right.
- [ ] **Ideal sections, or lossy ones.** Every section here is a flat all-pass. The
      drawing's have 47K in and a 220K-lamp across, so a real section's gain may tilt
      as its lamp heats — which the model would hear as a lane that colours as well
      as delays. The sheet as scanned does not settle the topology well enough to
      write the transfer function down, and inventing one would dress a guess in a
      drawing's authority. This is the first thing a scope across a real lane settles,
      and an afternoon is enough to falsify the whole question.
- [ ] The feedback tap and the series tap, as §1 describes them.
- [ ] The two sentences the manual states that no test yet speaks to: square-wave
      sweep with feedback up "resembles an echo repeat", and the top of the rate range
      with feedback up giving "an interesting pseudo-ring modulation". Both are
      descriptions of a sound rather than a number; what is worth having is whatever
      measurable shadow they cast — a modulation sideband, say — and if none can be
      found, that is worth knowing too.
- [ ] Per-phasor bypass, which the plugin does not have at all: "two foot switches
      … allow each of the two phasor sections to be turned on and off
      individually", each with an LED on the panel showing it is running. Decide
      whether that is two parameters, one, or nothing a host should not do with its
      own bypass — and settle the LEDs in the same breath. (The hardware's rule
      that the phasors do not work at all unless the footswitch box is plugged in
      is *not* worth reproducing.)
- [ ] What the model leaves out of the sound: the filament's own level-dependent
      compression, which is a large part of what the pedal's resistance does; the
      output-stage lamps PH1/PH2 at J3 and J4; and a noise floor, which the
      specification puts 86 dB below a 2 V input.
- [ ] `env_level`-class readouts for the editor, published from the block loop
      exactly as the metering is now. `PiFace::corner_hz` and
      `PiFace::sweep_position` already answer the question and are marked
      `allow(dead_code)` until something on screen asks it.

Measure, never speculate: the assertions belong to acoustics (a Goertzel bin, in
nice-wah's manner), and a model states what it simplifies rather than being tuned
until it sounds right and then described as a model.

## 3. The face

The current editor is the family's — bars, readouts, a cog — and it drives the real
filter. What the GUI phase owes it: two lanes you can read as two lanes, the sweep
drawn, and lamps that visibly lag. `PiFace::sweep_position` is the number to draw,
and it is already there. The conventions are already in force: absolute coordinates
scaled by `fit`, hit tests against this frame's geometry, the two embedded fonts
passed per draw call, and one source for both the selection and the value it
displays.

Two things the scan settles that the editor does not yet show:

- The Rate and Depth knobs are numbered **1 to 10** and the Feedback knobs **0 to
  10**, and the manual talks about settings only in those numbers ("about 8", "1
  to 3 on the dial"). A readout that only says `0.30 Hz` cannot be compared with
  what the manual tells a player to set, so the readout owes both.
- Each phasor has an LED that is lit while it is running — with the bypass above,
  that is the panel's only feedback about what is actually in circuit.

## 4. The assumption still in the defaults

The factory rates — 0.3 Hz and 2.0 Hz — are the manual's "slow speed (1 to 3 on
the dial)" and "moderately fast (about 6)" read through a taper the manual does
not publish. An exponential sweep of the .1–18 Hz range is what puts dial 3 near
a third of a hertz and dial 6 near two; the taper is this crate's guess, and the
code says so where the defaults are written. Anyone who measures a real machine's
Rate knob should replace the guess with the curve and the comment with the
measurement.

## 5. Loose ends outside this directory

- The standalone's choice of input channel is a device channel *number* on the
  way in and a channel *count* on the way out: `nice-common`'s
  `audio_io::open` refuses a number outside the device's layout, while
  `audio_io::config` hands the same number to cpal as the requested channel
  count. Picking channel 3 therefore opens a three-channel capture and reads the
  third — right channel, three times the traffic — and a stereo-in plugin like
  this one only ever receives one real channel, duplicated into both sides.
  Harmless for a device whose input is one jack, which is what the hardware has;
  worth separating before a plugin needs two.
- `target/bundled/nice-nam.vst3` was created by neither bundling run yet still
  receives the licence files, because the licence pass sweeps every bundle
  present: a stale binary wearing fresh licence text. Check its timestamp and
  remove it before anything copies the folder into a host.
