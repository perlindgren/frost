# Nice Phase — what is not yet true

The README's disclaimer is the honest ceiling of what ships: the notch
arithmetic is exact and measured by the suite, but *no manufacturer ever
printed a sweep table for this family*, and two numbers in the mode table
— the 100–1.6 kHz rail and the feedback ratio 0.25 — are labelled guesses
wearing measured clothes. This file records where the ceiling sits,
tiered by how audible each simplification is, followed by the sources the
code and the tiers were built from.

What is modeled today: four (or two) ideal first-order allpass sections
on one bilinear-warped corner, a memoryless feedback wire inside the wet
path, a perfect sine as the sweep, everything linear at every level. Each
of those is a promise the hardware does not quite make.

## Tier 1 — audible, and honestly modelable from published material

**The feedback wire is a resistor; in hardware it is a wired pair.** The
block-era path from the summer back into the phase network runs through
coupling capacitors and into JFET gates whose resistance is
voltage-dependent. Two audible consequences the model does not have: the
ringing *breathes* along the sweep — the loop's phase crosses the
cancelling angles unevenly as the corner travels, so the block's resonance
is loudest somewhere in the middle of the rail and gentler at the ends —
and the ringing *compresses* when hit hard, because the JFETs are the
loop's gain and they saturate. The first is a one-pole leak in the
feedback register (`fb_bus = ρ·fb_bus + tap`, ρ from a corner and the
sample rate) where the model today has ρ = 1; the second is a mild
nonlinearity inside the loop, which is also the drive stage's front door.
Both stay invariant-safe if they sit exactly where `feedback` sits now —
closed inside the wet path, dormant at DEPTH 0.

**The loop's threshold story is the drive stage.** Related but distinct:
players describe the block-era sound partly as "it growls when you hit
it," and the whole of that is level-dependence the linear model cannot
show. A tanh or diode pair at the network input — with the dry path left
clean, as the hardware's buffer stage leaves it — is the smallest honest
step, and the tests already know how to ask level questions of the
filter.

**The sweep's ends are a labelled guess, and the labels should keep their
teeth.** 100–1.6 kHz of stage frequency places the four-stage notches'
rest at roughly 58 Hz and 341 Hz — matching the frequency the published
resistor/capacitor consensus (24 K, 47 nF per section, one trimmer in the
middle of its travel) implies for the hardware at rest — and the rail is
wide enough to cover the sweep everyone agrees on by ear. But nobody has
published what the trimmer at either stop, or the pot at either end,
actually does; the top end in particular is chosen short of any air band
because the family is a midrange effect, not because a measurement says
1.6 kHz. A community-measured sweep capture of a real unit would move
these numbers to the measured column; until then the mode table's
comment, this file, and `DECISIONS.md` all say *guess*.

**The speed knob's range is generous on purpose.** 0.05–20 Hz brackets
the slow crawl and the fast swirl the recordings show. The specific lore
— "6.5 seconds at minimum, 150 ms at maximum" — repeats across forum
posts without ever acquiring a source, and the two script/block eras
probably differed; the range covers both readings rather than picking
one. What is *not* modeled: the hardware's LFO is a plain oscillator
whose waveform near the ends is gentler than a sine's tangent, and some
units' trimmer makes the bottom of the pot a freeze at a corner rather
than a crawl.

## Tier 2 — true of the model in a way the hardware is not

**The 45↔90 move is a morph; the hardware's is a pop.** The real
two-switch pedal relays or switches stages by lifting them out of the
chain; the transition is a thump, and the sound between positions does
not exist. Nice Phase crossfades a two-stage chain's output with a
four-stage chain's over 20 ms, blended with the feedback on the same
slew — click-free by construction and continuous under automation, which
a stompbox switch is not. Someone wanting the hardware's behavior wants
the opposite trade (a block-boundary hard flip); the crossfade is a
deliberate differentiator, recorded in `DECISIONS.md`, not an oversight.

**The sections are identical; four real JFET stages are not.** The model
runs four allpasses at exactly the same coefficient; a built chain has
part spread and gate leakage that smear the two notches a few percent
into slightly asymmetric holes. The effect is subtle — it fills a few dB
of the notch floor and roughens the whoosh — and would cost a per-stage
coefficient table for a benefit best heard against a null.

**Stereo is one sweep in two ears.** The plugin runs its own allpass
memories per channel with the same LFO, because the hardware had one
signal path and one hand on the pedal; true stereo phasing (two sweeps at
a quarter period apart, or decorrelated sections) is a real studio trick
that belongs to a different circuit than the one this table chases. The
same-tau version is honest; the two-sweep version would be a mode, not a
fix.

## Tier 3 — the plugin surface

**MODE is a float pretending to be a switch, because that is what the
host gives.** It is a continuous `FloatParam` that the UI snaps and the
DSP crossfades; CLAP and VST3 both have better words for enumerations
than a bare float, and a host's parameter list would read better with
the five names attached. Until nice-plug exposes integer-ish or
enumerated parameters, the snapping lives in the UI and the automation
lane keeps the morph for free — which some players will want anyway.

**The rail's notch ticks hide beyond the rail's ends.** During a
2↔4-stage crossfade the blended chain's holes can sit slightly outside
100–1.6 kHz; rather than draw ticks clipped at the rail's edges and lie
about the frequency, they simply do not draw. A bracket at the edge — "a
hole is off-rail that way" — would be the honest upgrade.

**No presets, no A/B.** A mode flip is one parameter write (automatable,
undoable, host-preserved), which is the preset's whole job for five
named rows; but there is no snapshot store, no compare, and the settings
file keeps one last-known state. When the rack's preset plumbing reaches
the small plugins, this row is plumbing, not design.

## Sources — what was actually consulted

The mode table, the feedback story and the tiers above were built from
published circuit analyses, each used for what it is:

- **ElectroSmash, MXR Phase 90 page** — the family's control surface and
  the script-vs-block silkscreen distinction that named four of the five
  rows; its schematic tracing of the four JFET stages.
- **Wikipedia, MXR Phase 90 and MXR pages** — era dates, the script/block
  enclosure history, the Phase 45/95 lineage the '45 rows follow.
- **George C. Kean's phaser article (via the Wayback Machine of geofex
  archives)** — the R28 feedback-resistor story for block-era units and
  the "same topology, one resistor" consensus that made the mode table a
  tap-and-wire table instead of five separate circuits.
- **stinkfoot.org, the Phase 90 R28 mod page** — the resistor's value
  neighborhood (~22–24 K) and the observation that opening the loop
  restores the script sound — the direct evidence that `feedback = 0` is
  the script row, not a stylistic approximation.
- **J. Dunlop's own Phase 95, Phase 45 and signature-model product
  pages** — the two-of-four stages-bypassed switch ("Phase 95: two or
  four stages"), which is exactly this crate's tap design, and the
  existence of the two-switch signature box that makes the superset row a
  corner rather than a sixth circuit.
- **Tonepad pages 7 and 13** — the allpass-chain analysis tradition the
  DSP's math follows, and the caution that stage counts of later units
  (Phase 100's six, with optoisolators) are *not* this family's.
- **Pedal Authority's signature-model review** — confirmation that the
  signature box's switches select the script/block wiring rather than
  adding a third voice, which is why its row takes a side, not a shape.

The record's negatives matter as much, and are why certain rows do not
exist: there is no "Phase 40" in any manufacturer's catalogue (the '45
rows answer to the real Phase 45/'95 two-of-four switch), there is no
"M680" phase pedal, the claim that the 90's core is a CA3080 OTA is lore
contradicted by the published schematics (it is discrete JFETs), and the
speed numbers' "6.5 s / 150 ms" pairing has never acquired a primary
source. Anything in this crate that leans on lore says so where it leans.
