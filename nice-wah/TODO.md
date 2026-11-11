# Nice Wah — what is not yet true

The README's disclaimer — the voice numbers are "the measured consensus of
published circuit analyses, not datasheet transcriptions" — is the honest
ceiling of what ships. This file records where that ceiling sits: the gap
between *answering measurements* and *being the unit*, tiered by how audible
each simplification is, followed by the sources the code and the tiers were
built from.

What is modeled today: one constant-peak-gain RBJ bandpass, Q fixed across
the sweep, a single-exponent taper, perfectly linear, unaware of what is
plugged into it. Every one of those is a promise a real circuit does not
make.

## Tier 1 — audible, and honestly modelable from published measurements

**Q is a scalar; in real tanks it is a curve.** A wah's resonance does not
hold its width as the pedal travels: tank losses and the pot's coupling move
with position. The Vox's high-DCR Shakespeare inductor damps the peak
hardest where the sweep asks most of it; a Cry Baby's Fasel rings most in
mid-travel. `Voice.q` is one number applied heel to toe. The truthful shape
is `q(t)` — three or four control points interpolated along the sweep — and
it is the biggest "which wah is this" cue after heel and toe. The
architecture already pays for this: `Controls.q` is read every sample, the
coefficient crossfader absorbs the movement, and the sweep costs one curve
evaluation.

**Peak gain should move with the pedal — by design.** The README currently
boasts constant peak gain, "never a level bump", which is the opposite of a
real wah: a tuned tank's peak height is proportional to Q, so as the peak
travels and its Q changes, the resonance rises above the passband floor, and
players chase that bump with the treadle. The faithful change is to let peak
level follow Q(t) (the physics derives it; a curve only fits what the
physics misses) and re-base OUT trim against it.

**The inductor is nonlinear.** Core losses make Q frequency-dependent, and
level-dependent — play hard, the core saturates, the resonance compresses
and damps. That is the audible difference between a Fasel and a CDC, and it
is what makes a real wah feel alive under dynamics, independent of the
envelope follower. Model as a saturation curve folding drive into the
instantaneous Q; the added state is one flux value per channel, well inside
what `tests/no_alloc.rs` polices.

**The '70's voice partly lives in the cable.** The Thomas Organ circuit's
high-impedance input forms a resonator with guitar-cable capacitance; the
weepy, pre-swept low-pass of a real VOX through fifteen feet of cable is
that interaction, not the filter. A truthful TOX '70 voice either models source
impedance — one input pole, a `cable_pf` control, roughly 100 kΩ — or the
README says so plainly and the voice stops pretending.

## Tier 2 — real, but each needs a source of truth before code

- **The taper is not one exponent.** Ganged capacitors and the pot's law
  make position-to-frequency an irregular curve — the reason CCIT's
  replacement pots exist is that the stock curve places its vowels wrongly at
  the extremes. `t^(2^taper)` is a good two-parameter family; a small
  per-voice table beats it.
- **Group delay tracks Q.** A high-Q band lags audibly in fast sweeps; the
  20 ms slew approximates a constant of the delay rather than the
  Q-dependent one.
- **Front-end asymmetry.** Clean-ish transistor gain stages that soft-clip
  asymmetrically when hot. The rest of "Slash's sound" is the wah feeding a
  cranked amplifier, which no filter owes anyone; the in-pedal part is real
  and the least measurable of the lot.

## Tier 3 — the step that makes any of the above *truthful*

The numbers were read out of published circuit analyses, not from units on a
bench. The repo already holds the instrument to change that: the Goertzel
harness in `src/dsp.rs` answers "how loud does a pure tone come out"
exactly. The procedure is one swept sine (or the `tools/noisefloor` rig)
through real units — or published netlists of the Plunkett lineage where
units are unavailable — then a per-voice fit: `q(t)` control points, the
peak-level curve, a five-point taper table, the cable pole for the TOX '70, and
the saturation knee. After that the voices are measured; before it, they are
argued.

**The boundary no TODO here crosses.** The '70 sound is inductor, cable and
amplifier. A filter can answer measurements; it cannot be the unit. Every
claim a voice makes should remain one a bench could check.

## References — implementation

Things read while writing the code, not while arguing about pedals:

- R. Bristow-Johnson, *The Audio EQ Cookbook* —
  <https://www.musicdsp.org/en/latest/Filters/197-rbj-audio-eq-cookbook.html>
  (the original W3C-hosted page has since gone dark; this mirror is what a
  link check confirmed live). The bandpass coefficients in `dsp::bandpass`,
  in their constant-peak-gain form; the code comment there says so.
- Will Pirkle, *Designing Audio Effect Plugins* (2nd ed., Focal Press 2019) —
  the auto-wah chapter's envelope-follower shape: a dB-law one-pole with
  attack and release, which is the follower in `Wah::env_follow`.
- nice-plug 0.4.2 — the `FloatParam`/`ParamSetter` asymmetry (no public
  setter outside a host) that forces the shared cells to be the parameters'
  truth in standalone; read from the registry sources under
  `nice-plug-core`.
- nice-common and the sibling apps — `audio_io`, `devices`, `Settings`,
  `AudioPanel` reused verbatim; `nice-nam/src/standalone/` and
  `nice-rack/src/main.rs` as the resolve-then-open and audio-owner patterns;
  `nice-tuner/crates/plugin-buffer` for the planar process buffer.
- J. O. Smith III, *Physical Audio Signal Processing* —
  <https://ccrma.stanford.edu/~jos/pasp/> — virtual-analog framing: the
  model owes measured behavior, not schematic fidelity.
- `tests/no_alloc.rs` and the Goertzel assertions in `src/dsp.rs` — the
  contract the next tier must keep passing while improving on it.

## References — the tiers

Where the "measured consensus" of the README came from. Cited by name where
URLs have rotted; anything not re-measured here inherits the README's
disclaimer, and Tier 3 exists to retire that sentence.

- B. J. Plunkett and L. L. Kushner, *Foot Controlled Continuously Variable
  Preference Circuit for Musical Instruments*, US patent US3530224A (Thomas
  Organ, filed 1967, granted 1970) —
  <https://patents.google.com/patent/US3530224A> — the original Cry-Baby
  circuit: the tuned inductor, the ganged capacitors, the pot in the tuning
  network, which is why position moves Q and level, not just frequency.
- ElectroSmash — <https://www.electrosmash.com> — the Cry Baby GCB95 page and
  schematic, and the mod guide's records of what the ganged capacitors
  (.022/.01) and the inductor's losses do to peak and taper.
- The circuit analyses in the modding literature — R. G. Kean's wah and
  guitar-cable writings (long circulating via his About.com column and its
  mirrors), and John Brown's analysis of the wah-pedal family — the source of
  the Q-damped-at-heel behavior attributed to the Vox inductor, and of the
  cable-capacitance resonance attributed to the high-impedance input.
- Fasel inductors (Fasel Elektronik) and the inductor-shootout literature
  the modding community keeps — the LTH1402/"Shakespeare" choke's high DCR
  and self-damping, Fasel-versus-CDC differences that are mostly loss curves
  and saturation, the physical basis of tier 1's third item.
- Dunlop 535Q documentation and the CCIT replacement-pot datasheet — the
  Q control is a marketed confirmation that Q and sweep shape are separate,
  voice-sized choices, and the pot literature is why the taper is modeled as
  a parameter at all.
- Mu-Tron III (1972) — the envelope-filter idea in its first famous form;
  the reason this plugin says *the auto-wah is the plain wah with ENV up*
  rather than shipping a second, dumber filter next to it.
