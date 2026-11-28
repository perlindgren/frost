# nice-eq design decisions

What this crate is, why each part is shaped the way it is, and what is an
assumption rather than a fact. The behavioural contract lives in
[README.md](README.md); this file is the reasoning behind it, in the order a
reader hits it.

Numbered `A-n` labels mark design assumptions: quantities nobody published,
chosen by us, and the reason for the choice.

## Why a mode selector at all

The ask was a set of equalisers under one plugin, with `g7` first. A mode is
here a **band table**, nothing more: seven centre frequencies, seven shapes,
seven widths. Every parameter a host sees — the seven band gains, the output
trim, the analyser settings — exists whatever the mode says, so adding a mode
later cannot rename a parameter, orphan an automation lane, or move a fader on
someone's control surface. `mode` is a stepped parameter whose positions are
appended, never inserted, which is the rule every stepped selector in this
workspace follows.

That is the whole design: a mode is data read by the DSP, not a second plugin
sharing a crate. A mode change mid-playback is therefore a coefficient
crossfade, the same path a fader drag takes — no fade-to-silence machinery,
because the blend already exists for the smaller case.

## The g7 grid, and what the reference document is

`g7` is the seven-band octave ladder, ±15 dB per band, ±15 dB of output trim:
100, 200, 400, 800, 1.6 k, 3.2 k, 6.4 kHz, lowest at the left. The subject is
the 1987 BOSS GE-7; the name on this crate is ours.

The one primary document in hand is a PDF of a **redrawn GE-7 circuit
schematic** (`BOSS GE-7.pdf`, title block "BOSS GE-7", drawn by an individual,
carrying a mod note and a private e-mail address — so a fan's redraw, not a
manufacturer's service sheet, and its provenance is stated rather than
assumed). It was read geometrically, because it has no text layer at all:
Distiller outlined every label, and the file is one Letter-size page of black
ink on a 9.32 pt CAD grid. Three facts came out of it, and they are the only
things taken from it:

- the seven frequency-determining potentiometers are labelled **"10K Lin"** —
  linear-taper parts — at octave-spaced centres **100 Hz … 6.4 kHz**;
- one note reads **"Arrow indicates Up/Boost"**, which is why every fader here
  boosts upward;
- the ladder runs to **6.4 kHz** at the top, and one of its labels does not
  match the specification: the potentiometer the published figures put at
  **1.6 kHz** is written **1.5 kHz** at VR5 in this drawing. The table says
  1.6 kHz, because a drawing that renames one band cannot overrule the six
  that agree.

The published specification — the same 100 Hz … 6.4 kHz octave ladder at ±15 dB
that every retailer and manual restates — is what the table is written against,
and the drawing agrees with six of its seven labels. The manufacturer's own
service notes for the unit are linked, with the RBJ cookbook the coefficients
actually come from, in
[docs/plugin-inspiration.md](../docs/plugin-inspiration.md); those service notes
were not read part by part to derive anything here, and this crate claims no
measured match to the machine.

What the document does *not* contain is the front panel: no slider, no slot, no
knob outline anywhere in 17 046 stroke segments. So the physical pitch of the
original's faders is not something this file can testify to. The panel
layout here — eight equal-spaced vertical faders, seven bands and the trim —
follows the photographs of the unit that everyone knows, not the schematic.

Per-band topology is **not** derived from the schematic either: six TL022
triangles and a wire net do not identify which section is which without
component values read off a drawing we did not read part by part. Hence:

**A-1 — the bands are textbook sections at nominal centre frequencies.** Six
RBJ peaking bells and one high shelf, computed from the Audio-EQ-Cookbook
formulae in normalised form. No attempt is made to reproduce the GE-7's
transistor-buffered op-amp network, its headroom, or its +4 dBu output stage.
This is an equaliser *in the shape of* the GE-7's controls, not a circuit
translation.

**A-2 — bell Q = 1.4, shelf Q = 0.707.** At octave spacing, Q = 1.4 puts a
band's half-gain points roughly ±0.4 octave from its centre, so two neighbouring
bands overlap rather than standing apart. That number is chosen against a
listening criterion written as a test, and measured against it: pushing one band
to +15 dB moves its own label by 15.0 dB, the next label by 3.4 dB and any label
two steps off by less than 0.8 dB — one slider reads as one slider. With every
band at +15 dB the curve runs from +23.5 dB at the labels down to +11.2 dB in
the hand-over valley between the 3.2 kHz bell and the shelf, and no third-octave
step is larger than 4 dB. So the ladder lifts a contour and never cancels
anything anywhere: the deepest point of the all-up curve is still 11 dB *up*.
Q = 0.707 on the shelf is the maximally-flat value of the shelf's own family: a
shelving section with a higher Q overshoots just below its corner, which prints
a hump nothing asked for.

**A-3 — the shelf's corner is the band's nominal frequency, and the bell's
centre is too.** Some hardware puts a top band's corner a third of an octave
below its label so that "6.4 kHz" means "the boost you hear at 6.4 kHz" rather
than "halfway up the slope". Here the label is the corner, and what that means
differs by shape, which the measurements state plainly: a bell at its label
stands at the full lift (+15.0 dB), because a bell's label is its peak; a shelf
at its label is half way up (+7.5 dB of its +15) and reaches the full lift about
a decade higher, because a shelf's label is the middle of its transition. The
first draft of the tests asserted +15 dB at 6.4 kHz for the shelf, measured
+7.5 dB, and the tests were wrong: the number is what a corner is.

**A-7 — a corner the sample rate cannot hold is folded to 45 % of it.** At the
8 kHz rate the standalone dialog offers, 6.4 kHz is above Nyquist: designing a
section there puts its poles on the unit circle at −1 and the band rings at 4 kHz
forever. The guard is on the design frequency, not on the arithmetic: the top
band becomes a 3.6 kHz shelf, which is what "the top of the band" means at that
rate, and every section keeps an α built from a sine of 0.31 rather than one of
1e-4. The label on the panel does not move, because the parameter did not.

## Band shapes: six bells and a shelf

The top band is a high shelf, the low six are peaking bells. Three reasons, in
order of weight:

1. There is nothing above 6.4 kHz in the ladder to hand the bell's upper skirt
   back to. A bell boosts a hill and returns to unity, so a +15 dB top band
   leaves everything above 8 kHz flat: the slider named for the highest
   frequency in the chain would visibly do least at the frequencies it is named
   for, and the air of the signal would be untouched.
2. A shelf at the top of an octave ladder and a bell at the bottom are not
   symmetric choices, and they should not be. The bottom band *is* usually a
   bell in these units — the ladder continues down to 50 Hz of program material
   in a way the top does not continue above 20 kHz — and the ask keeps it a
   bell, which the response measurement supports: a 100 Hz bell still reaches
   its half-gain point near 70 Hz and leaves sub-bass rumble alone, which is
   what the slider is for.
3. The analyser shows the difference, which is the point of drawing the
   spectrum under the sliders at all: the bells read as seven hills and the top
   band reads as a step that stays up to Nyquist.

## The analyser: where the work happens

**The FFT runs on the UI thread, not the audio thread.** The audio path copies
the block into a preallocated SPSC ring — the nice-tuner handoff, which exists
for exactly this — and the editor's frame drain does the windowing, the
transform and the reduction. Four consequences, all wanted:

- an FFT can never glitch audio, at any size, because there is no FFT in the
  audio callback to glitch it with;
- with the editor closed there is no analysis cost at all, and no plan, buffer
  or scratch exists until an editor opens;
- the "agile versus resolved" tension the off/top/all switch exists to resolve
  is resolved by the consumer, which may refresh each overlay on its own
  schedule, rather than by the audio thread paying for six sizes whether or not
  anyone is watching;
- the real-time rule that decides it — `assert_process_allocs` plus
  `tests/no_alloc.rs` — stays provable: a rustfft plan allocates, so a plan
  must never be built, resized or dropped on the audio thread, and here it
  cannot be.

**A-4 — one snapshot, six sizes.** The consumer keeps the most recent 32 768
samples, and every size transforms its own tail of that one snapshot. A short
window taken from the newest samples is the agile picture; the same instant
taken through a long window is the resolved picture; overlaying them is the
point of `all`, and doing it from one snapshot is what keeps the six curves
comparable instead of six different moments.

**The three positions.** `off` copies nothing into the ring and runs no
transform — the analyser is not drawn dimmed, and there is no cost behind a
picture nobody asked for. `top` transforms the selected size only. `all`
transforms all six from the one snapshot and overlays them, selected last and
brightest. The click-to-cycle gesture lives on the spectrum area itself,
because the size is a question about that picture.

**A-8 — the view can be pointed at the input, the output, or both, and the
audio thread copies only what is asked for.** The analyser's third selector is
`in | out | both`: one snapshot per tap, two rings, and a copy made only at the
end that is being watched — one copy per block for either single tap, two
for `both`, none at all for `off`. Two rings rather than one interleaved one
because the two ends of the box have nothing to say to each other's timing: a
half-filled input ring and a half-filled output ring are two honest states,
while one ring carrying both would be a mixture that means nothing on its own.
The cost is one extra preallocated ring of the same 128 KiB, and the saving is
that watching one end pays for one end.

`both` is the useful position, and the reason it is the default: the picture is
the equaliser's own curve drawn as the difference between two spectra, with the
input behind it as the thing that was there first. So the two are drawn in the
two house inks that mean exactly that pair — green for what went in, amber for
what comes out — never blended, never the same colour at different opacity, and
the watched size last so it is the one the eye keeps. The dimmed overlay curves
of `all` keep their own tap's ink at a quarter of its strength: size and end stay
distinguishable at a glance, which is two facts per line and the reason not to
invent a third palette.

The tap is a view setting, not a sound: it changes what is copied, never what is
computed for the audio. It lives beside `off/top/all` in the header and in the
settings file, and the DSP's `Tap` type owns the copy decision because that
decision is made on the audio thread, where a boolean is a fact about cost.

**A-10 — the analyser borrows the meter's inks, and nothing else's.** Green and
amber are not invented for this picture: they are the two colours every card in
the set already paints its input bar and output bar with — `nice-echo`,
`nice-ecomplex` and `nice-pi-face` all draw input green and output amber — so a
spectrum drawn in the same pair is read as the same sentence, and the fader row's
own level meters above it agree. Red stays what it has always meant in this set:
a reading that is over full scale. It appears in the plot in exactly one place,
on the 0 dBFS line and its label, and only while the watched tap has peaked above
it; at rest the line is the boring grey of a gridline, because a permanent red
rule is a decoration and a warning that is always on says nothing.

**The sizes are labelled by transform length: 1 k … 32 k.** A length of N real
samples yields N/2 + 1 magnitude bins, so "1 k bins" as the ask has it is a
1024-point transform; the label names the transform, and the README says so
rather than leaving the arithmetic to the reader. 32 768 samples is 0.68 s at
48 kHz — that is the low-frequency resolution limit the largest setting buys,
and also the reason the ring is that size.

**Bins are reduced to one value per pixel column by maximum.** 16 385 bins in
about 600 pixels is a blur, not a picture: drawing every bin would alias and
hide the partials that make a spectrum readable. One pass taking the loudest
bin per column keeps every peak the eye can resolve and costs nothing per
extra bin. Bar and interpolated are then two renderings of the same column
array, which is why switching presentation does not change what is measured —
and why a test can assert it.

**A-5 — Hann window, amplitude-correct scaling.** Hann: −31 dB sidelobes, a
6 dB/octave skirt, coherent gain 0.5, and every magnitude divided by
N·0.5/2 so a full-scale sine reads 0 dBFS. A Blackman window would hide
adjacent partials more gently than it hides the ones we want to see; a
rectangular window would put the picket fence back. The scaling is asserted in
a test with a known tone rather than left to be discovered by eye.

**A-6 — the frequency axis is 20 Hz … 20 kHz logarithmic whatever the sample
rate, and the dB axis is 0 … −84 dBFS.** Above Nyquist there is nothing to
draw, and a picture whose x-axis silently changes meaning when the interface
changes rate is worse than a picture that wastes some width at 44.1 kHz.

## The signal path

**A-11 — one input, one output.** The port map was a stereo pair until it was
reversed, and the first argument is worth keeping because it was a reasonable
one that did not survive being examined. It ran: a graphic equaliser belongs on
a stereo bus as much as in front of an amp, so be ready for one — and the
stereo declaration was never a mix, because each lane kept its own filter memory
and nothing was ever summed. What it was is a promise of four jacks, and a
promise that costs something to keep for no return: two histories of seven
biquads, a second pass of the same seven sections on every block, and one
routing decision taken in this crate that the host was always better placed to
take. Two lanes of a mono box on a stereo bus are two instances, a duplicated
send, or a downmix made before the box, and none of those three needs a second
lane inside it. The assumption record cannot settle how the hardware is wired —
the retained drawing was read for three facts and none of them is about jacks —
so this is stated as this model's own choice, and the sentence the earlier
drafts carried, that the pedal is mono, was a claim this file cannot testify to
and goes with the reversal.

So: one signal through seven sections in ladder order, one metered input, one
metered output, one analyser tap at each end. A host that hands over a wider
buffer than the layout declared gets channel 0 equalised and the other lanes
silent, which is the answer the other mono cards in the set give: silence
admits that there is no second input, where a copy would pretend there is one
and a sum would invent one.

Coefficients are recomputed once per block and crossfaded per sample against
the previous set, which is the nice-wah discipline: a stepped automation lane
moving a fader at 20 Hz would otherwise step the poles and print a click
through the speakers. The output trim slews by the same route, through zero as
a multiplier rather than as a phase inversion.

Every change arrives at the DSP as a block-wide `Controls` read, so the sample
loop touches no parameter and no atomic; the cascade's memory is `[Biquad; 7]`,
an array with a size the compiler knows, because the audio thread's allocation
is asserted to be zero and a `Vec` in that loop is how such a proof gets
quietly rewritten.

## What the view promises

One mapping function, `hz_to_x`, decides where a frequency is drawn, where its
gridline stands, which band tick lights up, and what the pointer's tooltip
says — the nice-tuner rule that what answers a click must move with what is on
screen. A draft of this file also promised that Escape aborts a drag. It does
not, and nothing in the editor reads a key: a fader is absolute, so the worst a
lost pointer can do is leave the band where the pointer last was, which is the
value that was under it. A promise the code does not keep was deleted rather
than implemented, because an editor that swallows Escape takes it away from
whatever else wanted it.

**A-9 — a fader is a position, not a relative control.** Pressing a rail moves
the cap to the pointer and the drag follows the finger along it, because that is
what the machine's own faders do and a slider you have to circle is a lie about
the panel. Shift asks for the same travel to be worth four times less — the
press's own position and value become the datum, so holding Shift mid-drag
refines instead of jumping — which makes the full ±15 dB span cost about 1 600
pixels of pointer travel at the reference window size, roughly 100 pixels per
decibel. The wheel is the arithmetic route beside it: half a decibel a notch, a
tenth with Shift, so a band can be parked at exactly 0.0 or exactly −7.5 without
a pixel being aimed at. One mapping, `rails.db_at`, answers the drag, the drawn
cap and the readout, so what you grab is what the number says.

Every fader, switch and dropdown writes through the host's parameter API
bracketed by one begin/end gesture, and mirrors the shared cell so standalone
— where there is no host and the cells *are* the parameters — behaves exactly
like the plugin it shares its code with.

## Tests as the contract

Response questions are asked acoustically with the house Goertzel probe, in
dB, at frequencies a reader can check: a bell's centre and its half-gain
points, a shelf's floor below and plateau above, seven bands at full boost
with no hole between them, transparency at 0 dB (where the peaking section's
numerator equals its denominator *exactly*, so "flat means flat" is not a
tolerance but an identity). Stability is the Jury conditions over every band
at every rate the settings dialog offers. `tests/no_alloc.rs` proves a block, a
fader sweep, a mode switch and a rate change cost the allocator nothing, in
its own binary because a counting allocator and nice-plug's disabler cannot
share one. And one test counts transforms: `off` must ask for zero of them,
because "off does no FFT at all" is a promise about cost, not about pixels.
