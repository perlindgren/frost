# Plugin inspiration and references

The frost nice-* effects are independent approximations inspired by publicly
available descriptions and circuit information. They are not affiliated with,
sponsored, approved or endorsed by the manufacturers of referenced equipment.
Third-party names identify the reference equipment and belong to their respective
owners. Our labels and "inspired by" wording do not constitute trademark
clearance, permission to copy artwork, or a claim of exact electrical/sonic
reproduction. Public availability of a document is not a redistribution license;
we link to drawings/manuals rather than package their images or scans.

## Names and compatibility

| Our displayed name | Inspiration/reference | Saved model index |
|---|---|---|
| Nice Dist / BS808 | Ibanez TS808 | 0 |
| Nice Dist / BS9 | Ibanez TS9 | 1 |
| Nice Dist / SH-1 | BOSS SD-1 | 2 |
| Nice Dist / OH-1 | BOSS OD-1 | 3 |
| Nice Dist / CAT | Pro Co RAT / RAT2 circuit literature | 4 |
| Nice Dist / Nice Clone; Nice Clone | Klon Centaur circuit literature; Behringer Centaur control descriptions | 5 in Nice Dist |
| Nice Brain Lag Multi | Roland RE-201 Space Echo descriptions and service literature | unchanged |
| Nice Pi Face | Mu-Tron Bi-Phase manual and service-document notes | unchanged |
| Nice Phase | MXR Phase 90/45 family and Phase 95 switching descriptions | unchanged |
| Nice Wah | General wah/filter methods and documented voice approximations | unchanged |
| Nice Eq | BOSS GE-7 graphic equaliser (1987): control layout, band ladder and range | unchanged |
| Nice Brain Lag Tube | Original single-head tape-delay approximation; hardware attribution withdrawn | unchanged |

Display names, Rust model identifiers and descriptions changed; DSP equations,
parameter IDs, numeric model indices, CLAP IDs and VST3 class IDs did not.
Legacy TS808/TS9/SD-1/OD-1/RAT text input remains accepted by Nice Dist. Package
names, settings filenames and bundle filenames remain stable: Nice Brain Lag Multi
still builds from `nice-echo` into `Nice Echo.clap` / `Nice Echo.vst3`, and Nice
Pi Face from `nice-pi-face` into `Nice Pi-Fase.clap` / `Nice Pi-Fase.vst3`.
Brain Lag Tube retains the `nice-ecomplex` package and `Nice Ecomplex` bundle filenames.
This preserves installed paths referenced by existing rack documents. The old
names in historical benchmark rows and technical references identify the
original source or earlier measurements, not additional branded products.

## Distortion family and Nice Clone

These links were already cited by the implementation READMEs:

- [Pedal Parts TS808/TS9 schematic and build notes](https://pedalparts.co.uk/docs/ToobScreamer.pdf):
  buffered signal path and component/output-network differences for BS808/BS9.
- [BYOC overdrive instructions](https://byocelectronics.com/808instructions.pdf):
  supporting construction/component information for the same reference family.
- [BOSS SD-1 service notes, March 1981, hosted by Synfo](https://www.synfo.nl/servicemanuals/Boss/BOSS_SD-1_SERVICE_NOTES.pdf):
  asymmetric clipping and tone-network reference for SH-1.
- [BOSS OD-1 service notes, December 1980, hosted by Synfo](https://www.synfo.nl/servicemanuals/Boss/BOSS_OD-1_SERVICE_NOTES.pdf):
  asymmetric clipping and filtering reference for OH-1.
- [Aion FX Helios legacy RAT2 documentation](https://aionfx.com/app/files/docs/helios_legacy_documentation.pdf):
  circuit topology and nominal values for CAT.
- [Texas Instruments LM110 application note](https://www.ti.com/jp/lit/pdf/snoa684):
  the LM308 comparison information cited by CAT's simplified amplifier model.
- [Martin Chittum's corrected Centaur trace, April 2009, hosted by Aion FX](https://aionfx.com/app/files/schematics/klon-centaur-schematic.png):
  topology/component reference for the project-owned shared Nice Clone DSP.
- [Behringer Centaur quick-start guide, hosted by device.report](https://device.report/m/3c0da66d0014aa9bd88f7b0df9d63e48fb6b2a01470f90098acef8c5f8f544d8.pdf):
  control descriptions/specifications, not evidence of a measured match.
- [Texas Instruments TL072 information](https://www.ti.com/product/TL072):
  nominal amplifier characteristics; not a transistor-level reproduction.
- [Jatin Chowdhury, A Comparison of Virtual Analog Modelling Techniques for Desktop and Embedded Implementations (2020)](https://arxiv.org/abs/2009.02833):
  circuit-modelling context already cited by Nice Clone. A citation does not
  imply incorporation of that paper's implementation or trained models.

The per-plugin READMEs retain component choices, assumptions and historical
benchmarks. No physical-unit calibration or exact sonic match is asserted.

## Brain Lag Multi

The `nice-echo` implementation already cited:

- [Inside a Space Echo, Red Bull Music Academy, archived 2019 capture](http://web.archive.org/web/20190912173023/https://daily.redbullmusicacademy.com/specials/2016-inside-a-space-echo/):
  the multi-head tape transport and control descriptions.
- [Roland RE-101/RE-201 service manual, existing fdiskc mirror](http://manuals.fdiskc.com/flat/Roland%20RE-101%20&%20RE-201%20Service%20Manual.pdf):
  service/control reference already recorded in the source. This mirror timed
  out during the 2026-11-22 review; availability is not guaranteed.
- [Roland Space Echo overview](https://en.wikipedia.org/wiki/Roland_Space_Echo):
  secondary historical context, not a primary circuit specification.

The saturation, modulation and spring approximations include explicitly chosen
parameters. Source access is not a claim that these algorithms reproduce a
particular unit's measured response.

## Pi Face

The existing implementation notes identify a six-page Bi-Phase owner's manual
and service drawings 77204 sheet D (BD1 BI-PHASOR), 77206 sheet B (power supply),
and an A/B phase-shift drawing. The original notes did not preserve exact URLs
for those service drawings; that provenance gap remains open. We do not invent
a historical download location or claim a newly found document was used before.

[Soundgas's Bi-Phase owner's-manual page](https://soundgas.com/blogs/resources/mutron-biphase-owners-manual)
was located during this review as a public route to the manual identified by the
notes. It is a retrieval aid, not evidence that this was the original host.
Lamp dynamics, headroom and other chosen quantities remain model assumptions.

## Nice Phase and Nice Wah

The existing `nice-phase/TODO.md` source ledger names ElectroSmash, manufacturer
product descriptions, GEOFEX/Wayback, Stinkfoot, Tonepad and secondary reviews.
Some exact historical URLs were not preserved. Public retrieval routes for the
identified material include:

- [ElectroSmash Phase 90 analysis](https://www.electrosmash.com/mxr-phase-90/pedals/delay/mxr-phase-90.html):
  all-pass circuit blocks; the original site has intermittent availability.
- [Dunlop Phase 95 description](https://www.jimdunlop.com/mxr-phase-95/):
  the manufacturer describes two/four-stage and script/modern switching. This
  supports the reference lineage, not our selected sweep/feedback constants.

The `nice-wah/TODO.md` implementation ledger already cites:

- [Robert Bristow-Johnson's Audio EQ Cookbook, MusicDSP mirror](https://www.musicdsp.org/en/latest/Filters/197-rbj-audio-eq-cookbook.html):
  bandpass filter equations.
- [Julius O. Smith III, Physical Audio Signal Processing](https://ccrma.stanford.edu/~jos/pasp/):
  virtual-analog context; automated retrieval was blocked during this review.
- [US3530224A, foot-controlled variable preference circuit](https://patents.google.com/patent/US3530224A):
  historical circuit reference already identified by the notes, not a patent
  clearance opinion.
- Will Pirkle, *Designing Audio Effect Plugins*, second edition (2019):
  the envelope-follower reference already recorded in the notes. This is a
  bibliographic reference, not a claim that the book is freely redistributable.

## Nice Eq (nice-eq)

Nice Eq is an independent graphic equaliser inspired by the control layout of the
1987 **BOSS GE-7**: seven octave-spaced bands from 100 Hz to 6.4 kHz, ±15 dB
each, a ±15 dB output trim, vertical faders with the lowest at the left. The
mode is named `G7` for that machine; the displayed name, the crate name, the
CLAP ID and the VST3 class ID are ours.

The DSP is not a circuit translation. Its sections are Robert Bristow-Johnson's
peaking and shelving biquads at the ladder's nominal frequencies, with chosen
Q values; the GE-7's transistor-buffered op-amp network, its headroom and its
+4 dBu output stage are not modelled and are not claimed. Retrieval routes for
the reference material:

- [Robert Bristow-Johnson's Audio EQ Cookbook, MusicDSP mirror](https://www.musicdsp.org/en/latest/Filters/197-rbj-audio-eq-cookbook.html):
  the actual equations the filter coefficients come from.
- [BOSS GE-7 service notes, hosted by Synfo](https://www.synfo.nl/servicemanuals/Boss/BOSS_GE-7_SERVICE_NOTES.pdf):
  a public route to the reference unit, listed here the way the SD-1 and OD-1
  service notes are listed above. It is the manufacturer's document about a
  machine; it was not read part by part to derive these coefficients, and its
  existence is not evidence of a measured match.
- The band ladder itself — 100 Hz … 6.4 kHz by octaves at ±15 dB — is the
  published specification of the unit, the same figures a retailer or manual
  restates.

A redrawn GE-7 circuit schematic was also read while choosing the ladder and the
drag directions; it is a hobbyist's drawing by an individual, not a
manufacturer's document, and it is not redistributed here. What it said, where
it disagrees with the published specification, and what it could not say — it
contains no panel geometry at all — are recorded in `nice-eq/DECISIONS.md`.
No artwork, logo, panel graphics or trade dress of any manufacturer is copied or
reproduced; the fader row's arrangement follows the widely seen photographs of
such units rather than any drawing.

## Brain Lag Tube (nice-ecomplex)

Brain Lag Tube is an original tube-style saturator/delay approximation with
A-1..A-7 design assumptions and unchanged DSP. No hardware schematic or measured
unit response establishes a particular commercial counterpart. It is not a
Binson model.

## Early related work: Binson Echorec

The **Binson Echorec** belongs to the historical background of electromechanical
echo. It used a magnetic drum rather than a tape loop. The current
[T-Rex Binson Echorec description](https://www.t-rex-effects.com/binson-echorec)
provides a public account of that architecture and its recreation. This link is
historical context added during documentation review, not a claim that it was
used to derive our DSP.

Neither Brain Lag Multi nor Brain Lag Tube replicates the Echorec's magnetic
drum, head arrangement or measured response. Binson is mentioned only as early
related work, not as a model name, implemented counterpart or endorsement.

## Distribution

Release packages include this file as `PLUGIN-INSPIRATION.md` and the companion
`NON-AFFILIATION.txt`. Those statements supplement, and do not replace, software
licenses, copyright notices, or third-party source-availability obligations.
