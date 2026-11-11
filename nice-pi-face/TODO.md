# nice-pi-face — what is left

The scaffold is in: the member, the manifest and features, the fourteen
parameters, the CLAP and VST3 exports, the frost editor, the standalone host on
`nice-common`'s transport and settings panel, and the gate and hook entries. The
sound is not.

## 1. Decide, before writing code

- **What `In A` means for a stereo plugin input.** The hardware's single input is
  one jack; the plugin's is two. Each lane taking its own side, or the summed
  input feeding both lanes? The question is recorded on `Controls::b_input` in
  `src/dsp.rs`, and the answer belongs there too.
- **What the mode selector looks like in the view.** There is nothing to decide
  about *whether* one exists: the three basic modes of operation are the three
  positions of the Phasor B Input switch, and `b_input` is that switch. What is
  open is presentation — the panel's word for the switch ("PHASOR B INPUT") is
  not the word a player uses for what it does, and a tooltip is carrying that
  explanation so far.
- **Whether phasor B's real feedback path is per-stage or per-branch.** Reading
  "A PHASE SHIFT": six cascade stages, branch 1 out of stage 3 to phasor B,
  branch 2 (the switchable feedback) taken from *after* the output stage 29B/211A
  and returned to stage 6's summing junction — not the textbook "output of the
  last all-pass back to the first". A diagram drawn from the textbook routing
  contradicts the sheet and should be corrected rather than copied.

## 2. The DSP, in the order it can be tested

- [ ] Six first-order all-pass sections per lane, unity gain at DC and Nyquist.
      Test: the group delay comes out in the documented order, and the magnitude
      stays at unity wherever the sweep sits.
- [ ] The lamp: a first-order lag on the drive current, corner well below 18 Hz,
      named in the code as the documented approximation. Test: a step of sweep
      voltage moves the corner exponentially and never instantly.
- [ ] Two sweep generators (.1–18 Hz), sine and square, with B able to run
      reversed against A. They must stay two oscillators — the beating is the
      product. Test: two rates a hair apart produce their difference as a beat.
- [ ] Depth, Feedback, and the headroom the numbers quote (4 V RMS one phasor no
      feedback; 2 V and 1 V both in series at 50% and 100%) — those three figures
      are mu-tron.com's, not the manual's, and are the only levels in play that no
      sheet in the pile carries.
- [ ] The three routings: `In A` feeding both phasors, `Out A` putting them in
      series for twelve stages, `In B` giving phasor B its own input.
- [ ] Sync: exact or reversed, and only when both phasors are swept by the same
      source — the manual is explicit that the switch has no effect otherwise, and
      that in `Rev` there is no double-depth phasing because the lanes move
      against each other.
- [ ] The behaviours the manual states in words, which are acceptance tests
      waiting to be written: square-wave sweep with feedback up "resembles an echo
      repeat"; the top of the rate range with feedback up gives "an interesting
      pseudo-ring modulation"; Phasor A's Sweep on `Ped` with a pedal that is not
      moving "changes the timbres (or tone) of the sound but does not sweep back
      and forth"; Rate Control on `Ped` with no pedal plugged in runs at "a very
      low rate".
- [ ] The pedal's double life. One `pedal` parameter feeds rate *or* sweep, and on
      the hardware the RATE CONTROL and Sweep switches decide which; the C-100
      "control pedal" is one jack, not two. The plugin cannot tell a player's
      intention from a CV, so it feeds every destination — name the simplification
      in the code rather than leaving it to be discovered.
- [ ] Per-phasor bypass, which the plugin does not have at all: "two foot switches
      … allow each of the two phasor sections to be turned on and off
      individually", each with an LED on the panel showing it is running. Decide
      whether that is two parameters, one, or nothing a host should not do with its
      own bypass — and settle the LEDs in the same breath. (The hardware's rule
      that the phasors do not work at all unless the footswitch box is plugged in
      is *not* worth reproducing.)
- [ ] `env_level`-class readouts for the editor, published from the block loop
      exactly as the metering is now.

Measure, never speculate: the assertions belong to acoustics (a Goertzel bin, in
nice-wah's manner), and a model states what it simplifies rather than being tuned
until it sounds right and then described as a model.

## 3. The face

The current editor is the scaffold's — bars, readouts, a cog. What the GUI phase
owes it: two lanes you can read as two lanes, the sweep drawn, and lamps that
visibly lag. The conventions are already in force: absolute coordinates scaled by
`fit`, hit tests against this frame's geometry, the two embedded fonts passed per
draw call, and one source for both the selection and the value it displays.

Two things the scan settles that the scaffold does not yet show:

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
