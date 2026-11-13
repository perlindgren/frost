# nice-ecomplex — what is left

The tape path is in: a transport whose speed is the Delay Time, a tube-style
front end the trims and Color walk from clean to soft-clipped, one playback
head on one drum with the wow welded to the motor, a tone stage sitting in
the feedback loop where the machine's tone control actually is, the Recount
loop running away past its crossover but staying finite, and an Echo Volume
that mixes against a dry which — pinned at bit level — never fades out. The
member, the manifest and features, the seven parameters, the CLAP and VST3
exports, the frost editor, the standalone host on `nice-common`'s transport
and settings panel, and the gate and hook entries are all in place. What is
left is the eight named assumptions the documents cannot settle — A-1
through A-8, catalogued in the README — and the things a machine on a bench
would retire: the delay taper, the drum's true rate, the flutter's shape,
repeat-to-repeat saturation, tape hiss, and the pitch smear the model
currently declines to do.

## 1. Decided, and where the answer lives

- **One drum, heard twice.** The hardware has one tape; the plugin has two
  lanes. Both lanes read the same drum, because a mono tape machine is what
  the machine is — but the *dry* keeps its own lane, because the dry is
  whatever arrived at the input, not something the drum plays back.
  `stereo_keeps_its_dry_lanes_and_shares_one_tape` pins both halves so the
  simplification cannot quietly become a bug.
- **The dry is always present, and that is a fact, not a taste.** The
  machine's Echo Volume (its panel's "Volume") sets the repeats against a
  dry that never fades; there is no wet/dry crossfade and no dry-kill
  anywhere in the machine, and `echo_volume_zero_leaves_the_dry_untouched`
  asserts bit-identical dry at zero so no future "wet/dry knob" refactor
  can erase the fact. The `² × 1.2` taper beside it (A-8) *is* a taste, and
  is labelled.
- **One bus stands in for two jacks (A-7).** The machine mixes its mic and
  guitar inputs inside itself; a plugin has one input bus. The model feeds
  both trims from the same arriving signal, exactly as one source run into
  both jacks would, so the trims set *drive* — which is what they do on the
  panel. A host with two mono feeds could eventually route them for real;
  until then the behaviour is honest, not a placeholder.
- **Color is disclosed, not smuggled.** It is not a panel control; it is
  this model's knob for a property the machine only lets you reach by
  overlevelling the input (A-6). The parameter doc, the tooltip and the
  README all say so, and `the_host_sees_exactly_the_parameters_we_promised`
  keeps the id set honest about its seven members.
- **There is no factory preset, so "default" is declared, not recalled.**
  A 1964 tape echo has nothing to recall; the resting place is named in
  `Controls::default` and pinned by
  `the_defaults_are_a_configuration_the_sources_describe`, which asserts
  the plugin and the model agree — the mixer sums to unity, Recount sits
  under the crossover, the dry is present.

Still to decide:

- **Whether Color should exist as a parameter at all.** Its effect is
  reachable by pushing the trims (that *is* the machine's way), and a
  purist port would offer only the seven panel knobs; the counter-case is
  that a plugin's input level is the host's gain, not the player's hand,
  and the drive deserves a handle of its own. Both positions are defensible;
  the knob currently takes the second and says why. It should be *re-made*
  at each review, not inherited.

## 2. The DSP

Done, with the test that says so:

- [x] The transport: tape speed is the Delay Time, spanning the A-1 range
      on the exponential taper to the millisecond
      (`delay_knob_spans_a_named_range_on_an_exponential_taper`), and the
      wow rides the same motor — slower tape wobbles deeper *and* more
      slowly, measured at both ends of the knob (A-2, A-3,
      `slower_tape_wobbles_deeper_and_more_slowly`).
- [x] One head, one arrival: a burst comes back exactly once, exactly
      after the selected delay, with nothing in front of it
      (`a_single_repeat_arrives_after_the_selected_delay`).
- [x] The Recount loop: unity near the A-4 crossover, running away above it
      but finite, the feedback-path soft limiter standing in for the
      machine's clipping (`past_the_crossover_the_loop_runs_away_but_stays_finite`,
      and `every_knob_at_every_position_stays_finite` over the whole knob
      space).
- [x] The front end softens what is pushed and passes small signals clean
      (A-6, `color_softens_a_hot_input_and_passes_small_signals_clean`).
- [x] The tone stage in the loop path: darker Timbre measurably thins the
      repeats (A-5 as behaviour, `darker_timbre_thins_the_repeats`), and
      the fixed 90 Hz high-pass is a tilt, not a mute
      (`the_high_pass_is_a_tilt_not_a_mute`).
- [x] The mix: Echo Volume zero leaves the dry bit-identical (A-8,
      `echo_volume_zero_leaves_the_dry_untouched`); one drum in both lanes,
      dry lanes kept separate (`stereo_keeps_its_dry_lanes_and_shares_one_tape`).
- [x] Zero allocations per block and per control sweep, in a test file that
      links without nice-plug and so runs on Windows too
      (`tests/no_alloc.rs`: `a_block_costs_nothing_to_the_allocator`,
      `moving_the_controls_costs_nothing_either`).

Left:

- [ ] **The TIME knob should smear pitch on the way, like the drum does.**
      On the machine, changing Delay Time changes tape speed, so the repeats
      audibly glide in pitch while the delay settles — the famous
      echoplex "wow down into the new tempo". This model keeps repeat pitch
      stable and lets the delay jump within its slew; the honesty ledger in
      `dsp.rs` says so rather than hiding it. The fix is a delay-line whose
      read position integrates speed (varispeed read), not a position
      re-based per block. Until then this is the crate's biggest known
      difference from the machine.
- [ ] **Repeat-to-repeat saturation is not modelled.** Driven tape
      flattens its peaks a little more with every pass; here the darkening
      across repeats comes from the tone stage alone. The front end clips
      on the way *in*, not in the loop. A bench would say how much; the
      loop's `loop_guard` currently guards instead of colouring.
- [ ] **No hiss, no crosstalk, no irregularity.** The transport's wow is
      one clean sine at one rate (A-2/A-3); the machine's is wow *plus*
      flutter plus bearing slop, and its silence is not silent.
      nice-echo's A-9 argument — a noise-free digital loop makes silence a
      fixed point — does not bite here the same way (the guarded loop is
      sub-unity most of its travel and runs away loud past the crossover),
      but characteristic hiss is half of what people mean by "that sound",
      and its magnitude would have to enter as a labelled assumption, not
      a garnish.
- [ ] **The interpolation is linear.** Fractional reads roll off a little,
      which reads as tape smear and is stable (it is commented as such),
      but the colour depends on the delay time in a way a bench would not
      accept as the machine's. A windowed-sinc or at least cubic read is
      the retirement path; the no-alloc budget survives either.
- [ ] **Editor readouts, like the rack's.** The loop gain near unity, the
      front end's instantaneous drive, and the drum's instantaneous speed
      are the three numbers a player would pay to see; the DSP knows all of
      them and the face has never been told.

## 3. The face

The editor is the family's: dials on grid-first slots, captions, tooltips,
the meter row, the cog and shared audio panel. What is in, with the test:

- [x] Every control has a caption, a tip and a slot; no two share a slot;
      the view reaches every parameter the host sees and each control drives
      the cell of its own parameter
      (`every_control_has_a_caption_a_tip_and_a_place`,
      `no_two_controls_share_a_slot`,
      `the_view_reaches_every_parameter_the_host_sees`,
      `each_control_drives_the_cell_of_its_own_parameter`), and the layout
      and meter row stay inside the window at every size
      (`the_layout_stays_inside_the_window_at_every_size`,
      `the_meter_row_stays_inside_the_window_at_every_size`).
- [x] The TIME dial reads milliseconds, computed through
      `dsp::delay_seconds` — the same function the transport uses, so the
      readout and the sound have no way to disagree.

What the GUI phase owes it:

- [ ] **The drum drawn turning.** The face says nothing about the one fact
      the machine is built on — a drum whose speed is the knob. A tape
      strip or drum mark that visibly slows as Time lengthens would teach
      the A-2 coupling without a paragraph; nice-echo owes its face the
      same admission.
- [ ] **A Color that shows when it is working.** The knob moves; whether
      the front end is actually softening depends on the input's level.
      A drive readout (or a lit ring) would separate "the knob is up" from
      "the tube is doing something" — the §2 readout item's first client.

## 4. The assumption still in the defaults

Delay Time 0.5 is the geometric centre of the A-1 exponential span, ≈ 350
ms — a *position*, not a measurement. The sources of this model's tier
never give a taper; the real knob's seconds-vs-position curve could be
log-linear, could be detent-stepped by loop selection, could be anything
the drum did. Anyone with a real machine's curve should replace the taper
and the comment with the measurement; the A-1 tests pin the endpoints, so
only the middle of the travel moves, which is exactly the part that was
guessed.

Recount 0.4 sits deliberately *below* the A-4 crossover — the factory sound
repeats and decays rather than howling. That is a choice about what a first
listen should demonstrate, not a claim about any machine's noon position,
and it is commented as such beside the default.

## 5. Loose ends outside this directory

- The gate legs (`.githooks/pre-commit`, `.githooks/pre-push`,
  `.github/workflows/gate.yml`) deliberately run nice-ecomplex clippy
  *without* `--features standalone`: the standalone lane pulls the ASIO
  host behind `cfg(windows)` and the Linux gate cannot exercise that path,
  so the standalone clippy leg is run locally before any standalone change,
  and the comments in both files say so. If a future CI matrix grows a
  Windows runner, that leg belongs there and the local-workaround comments
  should be deleted rather than left as archaeology.
- The VST3 class id `FrostNiceEcmplx1` is 16 bytes of our own making (the
  family's pattern; nice-echo owns `FrostNiceEcho001`). Nothing outside
  this crate can collide with it today; if a registry of Frost class ids
  ever exists, nice-ecomplex's entry is this line.
- `target/bundled/` accumulates bundles from every member's bundling run
  and the licence pass sweeps every bundle present — the family's other
  TODOs carry the same warning about stale binaries wearing fresh licences.
  Check timestamps before copying the folder anywhere a host can see it.
- The gate pins the Rust toolchain (`.github/workflows/gate.yml` explains
  why, and nice-echo's TODO carries the full lesson — the pin travels with
  `components: rustfmt, clippy`). The same bump ritual applies to this
  crate's legs: install the new toolchain locally, run the gate, fix what
  the new clippy says, in the commit that changes the number.
