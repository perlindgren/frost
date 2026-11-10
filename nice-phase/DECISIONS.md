# Nice Phase — the decisions, and whose taste they are

This crate was mostly built unattended, overnight, on the owner's explicit
"take the reasonable route forward and record your decisions." This file is
that record: every place a judgment call was made, what was chosen, what
was rejected and why, and how much of each choice is measurement versus
taste. Where the answer is "guess," it says guess.

## 1. The crate is `nice-phase`, the plugin is Nice Phase

The working name was `nice-phase90`. "Phase 90" is not a word the English
language needed — it is a trademarked model number, and a crate, bundle
name and window title are exactly where a mark is used as a source
indicator. The number was dropped along with any manufacturer name;
"phase" alone is the name of an effect, as generic as "wah." The internal
git branch kept its old name through the work and was retitled to
`nice-phase` before the first commit so nothing with `90` in a
trademark-shaped position survives even in refs.

## 2. What the five settings are — the research, and the shape it agreed on

The owner listed five settings to chase — Script90/40, Block90/40 and
EVH90 — and guessed "it could be that EVH90 is a subset." The research
leg (report absorbed into `TODO.md`'s sources) landed on:

- **Script vs Block is one circuit and one resistor.** Both eras are the
  same four JFET allpass stages with the same sweep; the later boards add
  a feedback resistor (published values cluster at 22–24 K) from the
  output summer back into the phase network. Everything the two eras'
  reputations say — mellow with headroom versus ringing whoosh — follows
  from that wire. So the table's eras differ only in the `feedback`
  field, which is falsifiable: `feedback = 0` is bit-identical to the
  open chain, and the mod community's "open the loop and you get the
  script sound back" is the direct evidence.
- **"Phase 40" does not exist** — no manufacturer ever made one. The
  owner's "40" was read as the real 45, and the 45/'95's own published
  switch ("two of four stages") is exactly this DSP's tap: the '45 rows
  run the first two sections of the same chain.
- **The signature model is a two-switch box** whose switches select
  stage count and feedback — the owner's superset hypothesis confirmed:
  it is not a sixth circuit, it is the other four's corners with a
  faceplate. A table row can only be one corner, and this one takes four
  stages, no feedback: the side the player it is named for is recorded
  using. **That is a reading of the record, not a specification** — if a
  better-documented side surfaces, the row's two fields are one line
  deep.
- Negatives worth repeating, because they killed candidate rows: no
  "M680" phase pedal exists; the CA3080-in-a-90 story is lore the
  published schematics contradict (discrete JFETs); the "6.5 s / 150 ms"
  speed pairing has never acquired a primary source.

## 3. One sweep for all five settings

An earlier draft gave each mode its own sweep range. The research said
nothing supports per-mode ranges — the trimmer and pot are shared
hardware across every wiring the table models — so all five share
100 Hz–1.6 kHz, and the differences are the tap and the wire. The
numbers themselves are **labelled guesses**: the consensus resistor and
capacitor values put the notches' rest position near 58 Hz / 341 Hz,
which the rail brackets; nobody published the rail's ends, the top least
of all. `README.md` and `TODO.md` are pointed at this admission rather
than the other way around.

## 4. The feedback wire samples the resistor's function, not its layout

The literal R28 runs from the summer's output back into the JFET gates.
The model instead feeds the **wet chain's output, one sample late, into
the network's input** — closed inside the wet path rather than around
the whole dry-and-wet sum. Two reasons. Invariants: DEPTH 0 stays the
dry signal bit for bit and MIX 0 stays flat, tests included, and neither
promise survives a loop that wraps the dry path. Stability: through a
one-sample register with allpasses bounded by unity, a `|feedback| < 1`
loop cannot grow, no matter what automation does. What was rejected: the
literal tap point (breaks the invariants), and a bidirectional or
state-variable loop (models a circuit nobody documented here). The
**value 0.25 is taste, not measurement** — published analyses give the
resistor, not its digital equivalent ratio; it was set where the ring is
audible as character and the closed-form blooms (≈ +1.3 dB at the stage
frequency) stay modest. TODO tier 1 keeps the unmodeled half: the
coupling caps that make the ringing breathe, and the JFET softening that
makes it compress.

## 5. The 45↔90 morph is deliberate, not a bug against the stompbox

The mode parameter is continuous; a flip crossfades a two-stage chain's
output with a four-stage chain's over 20 ms, and crosses the feedback
axis on the same slew. A hardware switch pops and has no sound between
positions; a plugin parameter under automation must not click, and the
blend is click-free because both chains are unity-gain at every
frequency. `TODO.md` tier 2 states the trade plainly for anyone who
wanted the pop.

## 6. Names, and one name the owner suggested that was not used

The owner's directive: rename anything trademark-shaped — Nice
Phase 90 → nice-phase, and the wah's voices to "Tox, Kry Baby, and Slop,"
later amended to keep "Slash → Clash." Executed as: the wah's three
voices are **TOX '70, SKY BABY, CLASH** (numbers untouched; the names
were the only thing at risk), and the phaser's fifth row, working name
EVH 90, became **SIGNATURE 90** — a player's name is the most
protectable thing in the room, alive and licensed.

**"Kry Baby" was rejected.** It is phonetically identical to the mark it
would be evoking, and trademark protection reaches phonetic equivalents;
a deliberate homophone is the pattern courts call confusion-in-sound, and
there is no honest argument that "sounds like but is not" improves with a
spelling change. SKY BABY carries the same sky imagery without wearing
the mark. TOX and CLASH — the owner's own inventions — stood as offered.

The doctrine the whole workspace now runs on, from these two renames:
brand names are allowed in prose as nominative references (the TODO's
source notes name the pedals they read), never in anything a user
clicks, loads or ships.

## 7. Small calls the code makes, so nobody has to re-litigate them

- **`mode_index` is a total function** (clamp, round, index): host
  automation can hand any f32, including the absurd; a NaN arrives as
  setting zero rather than as a panic or an unwrap. The test
  `every_mode_fits_the_parameter_that_carries_it` pins the absurd values
  down.
- **The filter publishes its notch frequencies** (a pair of shared
  cells) instead of the view recomputing them: the rail's green ticks
  then mark where the *actual* filter has holes, mid-crossfade included,
  and a view/DSP disagreement becomes structurally impossible. Notches
  beyond the rail's ends simply do not draw — TODO tier 3 prefers that
  over clipped ticks that lie about frequency.
- **The mode knob's wheel moves one whole setting per notch, the drag
  snaps to whole settings**, while the parameter itself stays
  continuous: players get a switch, automation keeps the morph.
- **The knob widget moved from nice-wah into `nice_common::knob`**
  (widgets feature) with its drawing unchanged, so the family draws one
  knob; its `draw` keeps nine arguments under the root canvas's own
  immediate-mode `too_many_arguments` precedent rather than a
  speculative options struct.
- **Standalone parameter truth lives in the shared cells.** nice-plug
  0.4.2's `FloatParam` has no public setter outside a host, so the
  standalone writes the cells the audio thread reads, and the
  `STANDALONE_MODE` flag tells the one `process` which authority to
  honor; in a host the cells trail the parameters by a frame and
  automation stays the truth. This is the same shape nice-wah ships.
- **Tests assert closed forms, not neighborhoods.** Each setting's gain
  at its own stage frequency has an exact predicted value per topology
  (trim for the open fours, 0.5 + 0.5/(1−fb) for the ringed fours,
  silence for the open twos, 0.5·fb/(1+fb) for the ringed two), and the
  suite checks to the thousandth — a future feedback change will find
  out immediately if it moved the loop's physics, which a loose bound
  would hide. The sweep-walk test windows the LFO's true extremes
  (crest at 1 s, bottom at 3 s of a 0.25 Hz sine starting mid-sweep and
  climbing), which is the kind of off-by-a-quarter-cycle detail the
  first draft of that test got wrong and the corrected numbers prove
  right.
