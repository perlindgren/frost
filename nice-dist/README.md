# Nice Dist

A TS808/TS9/OD-1/SD-1/RAT/Centaur overdrive/distortion with a Frost editor, CLAP/VST3 exports and a standalone
build using the same plugin processing implementation.

Drive, Tone and Level control the modeled circuit. The model selector switches
between TS808, TS9, OD-1, SD-1, RAT and Nice Clone; the **OS** button cycles Off, 2x, 4x and 8x oversampling.
8x is the default. Model/quality changes fade down over 64 host samples before
reinitializing the circuit and resampler, then fade up over 64 samples. Controls
are smoothed over approximately 10 ms, with network coefficients updated every
16 host samples. These paths and reset neither allocate nor free heap storage.
The box is mono — one signal through one circuit — and so is this model: a
single mono input enters the circuit and a single mono output leaves; folding
a stereo signal down, or widening the answer back out, is the host's work,
not the pedal's. One circuit runs at the selected oversampling rate, and
there is no second lane with state of its own.
Existing TS808/TS9
model indices are unchanged; SD-1 is appended as index 2 (standalone number 3).

The host receives a constant 32-sample latency in every mode (0.667 ms at 48 kHz).
Off adds an explicit 32-sample delay; the linear-phase up/down FIRs contribute
that delay at 2x/4x/8x. This keeps parallel rack branches aligned when quality
changes without requiring host latency renegotiation. The FIR implementation is
shared with nice-clone in `nice-common::oversample`; Clone retains its own
existing latency/switching policy.

## Models and references

The TS808/TS9 nominal signal path follows the buffered [Pedal Parts TS808/TS9 schematic](https://pedalparts.co.uk/docs/ToobScreamer.pdf):

- 22 nF input coupling and 510 kohm bias load, then the input follower and
  1 uF/10 kohm coupling into the noninverting clipper.
- 4.7 kohm/47 nF grounding branch, 51 kohm plus 500 kohm Drive feedback,
  a symmetric silicon-diode pair and 51 pF feedback capacitance. The feedback
  voltage is solved with a bounded Newton/bisection method, retaining the
  capacitor histories; this is not a waveshaper after a fixed linear gain.
- The connected active Tone network, including both 220 nF capacitors,
  20 kohm tone pot and second op-amp feedback. Level loading and its coupling
  capacitors are solved as a network rather than an isolated output multiplier.
- TS808 output: 100 ohm series resistor and 10 kohm shunt. TS9: 470 ohm and
  100 kohm. Both retain the 10 uF output capacitor and assume a 1 Mohm external
  load. [BYOC's build guide](https://byocelectronics.com/808instructions.pdf)
  independently documents the resistor substitutions.

The SD-1 follows the [BOSS March 3, 1981 service schematic](https://www.synfo.nl/servicemanuals/Boss/BOSS_SD-1_SERVICE_NOTES.pdf),
not the Waza Custom mode or the DS-1 distortion pedal:

- 47 nF/10 kohm/470 kohm input coupling, then 18 nF/100 kohm coupling into
  the clipper; 4.7 kohm/47 nF in its grounding branch.
- 33 kohm plus a linear 1 Mohm Drive pot. There is no TS-style 51 pF feedback
  capacitor. Two silicon diodes conduct for positive output-minus-input voltage,
  one for negative voltage. Each branch is solved as a series-diode string.
- 10 kohm into the tone stage, an 18 nF shunt, a 20 kohm tone pot, and a
  27 nF/470 ohm wiper branch. The op-amp feedback is 10 kohm in parallel
  with 10 nF. Capacitor histories are retained while enforcing the ideal
  op-amp voltage constraint separately from output-node KCL.
- 1 uF coupling, 4.7 kohm series, linear 10 kohm Level pot; a 22 kohm series/
  22 kohm shunt before 47 nF coupling into the output buffer's 1 Mohm bias
  load. Output coupling is 1 uF through 1 kohm into 100 kohm, plus the same
  assumed 1 Mohm external load.

Capacitors use trapezoidal companion models. The TS audio pot positions use a
square-law approximation; SD-1 Drive/Level follow the service schematic's linear
pots. Tone uses a linear approximation to the physical W taper for all models.
The input sample convention is one volt per full-scale sample. Buffers are
ideal unity followers with symmetric +/-3 V limiting around the 4.5 V bias;
op-amps use ideal feedback with the same approximate output rails. Finite beta,
transistor loading/output impedance, op-amp bandwidth/slew/saturation recovery,
noise, power-supply dynamics and JFET bypass switching are not modeled. The
silicon diode pair uses a nominal Shockley fit (Is=2.52 nA, nVt=45 mV), not a
measurement of a particular pedal. These approximations make this a nominal
circuit-derived model, not a claim of measured hardware identity.

HWV2 is not implemented. Its documented additional output boost is insufficient
information to reconstruct its complete circuit accurately.

## Build and validation

```sh
cargo run -p nice-dist --release --features standalone
cargo xtask bundle nice-dist --release --features standalone
cargo test -p nice-dist --features standalone
cargo run -p nice-dist --release --example cost
```

Bundles are `target/bundled/Nice Dist.clap` and
`target/bundled/Nice Dist.vst3`. CLAP ID: `com.frost.nice-dist`;
VST3 class ID: `NiceDistPlugin01`. The workspace bundler signs and verifies
completed macOS bundles after installing license resources.

`tests/reference/ac_reference.py` independently solves the linearized circuit
in complex frequency using the bilinear frequency substitution. Its CSV is
checked against settled, low-level time-domain responses at several drive/tone
positions and frequencies for all six models. Additional tests cover nonlinear
Kirchhoff residuals, distortion, level attenuation, folded-harmonic suppression,
reset equivalence, invalid inputs, rate/quality switching,
allocation-free processing/reset and the actual plugin block adapter. The alias
test is a targeted 9 kHz/21 kHz probe, not a full-band alias-free guarantee.
The 9 kHz probe measured 22.9 dB less normalized folded 21 kHz energy at 8x
than Off on the current build. On this aarch64 macOS host, stable-control stereo
processing at 48 kHz measured about 1.4% of a core Off, 3.5% at 2x, 6.8% at 4x
and 14.4% at 8x. These are host-specific measurements, not scheduling guarantees.
They were taken against the two-lane stereo model that preceded the mono one;
the box now runs one circuit per block instead of one per lane, so the figures
stand here as history rather than current cost.
A later run with the SD-1 model added measured about 11.1% of one core at
8x for SD-1, versus 13.6–13.8% for the TS models on this host. The cost example
now reports all six models.
The benchmark prints best-of-three mono DSP costs with stable controls; it
does not measure the GUI, device transport or worst-case scheduling.

## 64-frame stereo compute-time comparison

Run `cargo run -p nice-dist --release --example block_cost` to reproduce.
Raw results are in [`measurements/block_cost_64_8x.csv`](measurements/block_cost_64_8x.csv).
Measured on the current aarch64 macOS desktop on 2026-11-19: 48 kHz, 8x,
64 stereo frames per block, all controls at 0.5, a 750 Hz sine of peak 0.3
with opposite-polarity channels. These runs predate the box's move to mono and
describe the two-lane stereo model they measured. After 512 warm-up blocks, 8,192 blocks per model
were individually timed with a monotonic clock. The deadline is 1,333.33 us.

| Model | Minimum | Median | 95th percentile | 99th percentile | Maximum | Median / budget |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| TS808 | 178.208 us | 207.459 us | 317.125 us | 583.750 us | 4,874.209 us | 15.559% |
| TS9 | 178.000 us | 207.416 us | 241.083 us | 447.958 us | 3,951.750 us | 15.556% |
| SD-1 | 146.167 us | 168.333 us | 354.375 us | 515.917 us | 4,832.959 us | 12.625% |

This includes the DSP control update once per block and output stores. It excludes
plugin metering, device transport and GUI work. Controls remain stable, so it does
not measure coefficient retuning or model/quality transitions. Wall-clock timing
also includes any interruptions to the benchmark thread; the outlier causes were
not profiled. The measured maxima exceed the deadline, so these figures are not
a claim of guaranteed real-time operation. Model order, desktop load and CPU
frequency can affect comparisons, especially the upper percentiles.

## Folded-energy comparison

Measured with `cargo run -p nice-dist --release --example folded_energy`
on 2026-11-19. Raw amplitudes and powers are recorded in
[`measurements/folded_energy.csv`](measurements/folded_energy.csv).

The stimulus is a coherent 9 kHz sine, peak 0.7 (the model convention is volts),
at 48 kHz; Drive and Tone are 1.0 and Level is 0.8. After 32,768 settling samples,
32,768 output samples are measured with coherent single-bin DFTs. Combined
folded power is `(A3k^2 + A15k^2 + A21k^2) / 2`, in output sample-units squared.
The dBc column is `10 log10(folded_power / fundamental_power)`; more negative
means less folded energy relative to the 9 kHz fundamental.

| Model | Oversampling | Folded power | Folded energy (dBc) | Reduction versus Off |
| --- | --- | ---: | ---: | ---: |
| TS808 | Off | 1.927010e-03 | -13.472 | 0.000 dB |
| TS808 | 2x | 1.146706e-04 | -26.479 | 13.007 dB |
| TS808 | 4x | 5.078794e-05 | -30.181 | 16.709 dB |
| TS808 | 8x | 2.497681e-06 | -43.313 | 29.841 dB |
| TS9 | Off | 1.945959e-03 | -13.472 | 0.000 dB |
| TS9 | 2x | 1.157982e-04 | -26.479 | 13.007 dB |
| TS9 | 4x | 5.128740e-05 | -30.181 | 16.709 dB |
| TS9 | 8x | 2.522240e-06 | -43.313 | 29.841 dB |

These are the summed visible folded odd-harmonic bins for this particular
symmetric circuit/stimulus. Harmonics folding onto the fundamental cannot be
separated and are excluded, as are even-harmonic bins, DC and noise. This is not
an integrated broadband measurement or a physical-pedal comparison. SD-1 is
excluded from this TS-only probe because its asymmetric clipper adds even
harmonics that require a different measurement interpretation. The almost
identical normalized figures are expected here: only the linear output network
differs between models, and its frequency response is nearly flat at these bins.

The earlier 22.9 dB result measured only the folded 21 kHz bin; this combined-bin
metric is different. Individual bins need not improve monotonically: the CSV
records about -45.9 dBc at 2x and -43.5 dBc at 4x for 21 kHz, while combined
folded energy improves at every step. Aliased harmonics can add or cancel in a
particular bin, so neither one tone nor one bin establishes full-band behavior.

Host parameters are saved by the host. Standalone preferences, audio settings
and window geometry use `nice-dist-settings.ron` and nice-common's shared
audio panel, transport and atomic persistence. Audio device configuration belongs
to the host in hosted instances.

The application denies unsafe code. Native calls stay in the existing thin
project-owned wrappers; optional Windows placement uses the `unsafe` feature.
No third-party dependency sources or versions were changed.

### OD-1 revision

OD-1 follows the [December 26, 1980 BOSS service schematic](https://www.synfo.nl/servicemanuals/Boss/BOSS_OD-1_SERVICE_NOTES.pdf)
(dual-op-amp revision), not the earlier quad-op-amp version. It shares the
asymmetric clipping stage with SD-1, but uses a 1k input resistor, 4.7nF
clipping-stage coupling capacitor, and an inverting 10k/10k, 18nF fixed
low-pass stage. Its level wiper feeds the 47nF/1M buffer coupling without
the SD-1's 22k/22k divider. Tone has no effect in OD-1 mode.
Existing host model indices and saved settings numbers remain unchanged;
OD-1 is appended as host index 3 / standalone number 4. The selector displays
TS808, TS9, OD-1, SD-1, RAT. The same ideal-buffer and nominal-diode limitations apply.
Historical benchmark tables above predate OD-1; no OD-1 performance result is claimed.

### Classic silicon RAT

RAT uses a separate circuit path based on the stock settings of the
[Aion Helios RAT2 reconstruction](https://aionfx.com/app/files/docs/helios_legacy_documentation.pdf):
22nF input coupling, 1M bias, 1k/1nF RF filter; 100k Distortion with
100pF feedback and 47 ohm/2.2uF plus 560 ohm/4.7uF grounding branches;
4.7uF output coupling and 1k into symmetric silicon diodes to ground;
100k Filter plus 1.5k and 3.3nF, then 22nF into a 1M buffer input;
ideal unity JFET buffer, 1uF coupling and 100k Volume into a 1M external load.
Optional Sweep, alternate clipping and Softness modifications are not included.

The LM308 is approximated by a dominant pole (nominal 100k DC gain,
1MHz gain-bandwidth), 0.3V/us slew limit and +/-3V output swing around
mid-supply. The bandwidth and slew figures are listed in
[TI's LM110 comparison table](https://www.ti.com/jp/lit/pdf/snoa684).
These are modelling assumptions, not measurements of an LM308 at 9V.
The pole uses backward Euler for stability; passive capacitors use trapezoidal
companions. When the amplifier limits, feedback capacitor states are updated
from the constrained output. Diode clipping includes the downstream RC loading
through a scalar Thevenin reduction, rather than a fixed voltage clamp.
No detailed input protection, output impedance, saturation recovery, JFET
nonlinearity, noise or supply dynamics are modelled. This is not a calibrated
hardware or manufacturer SPICE emulation.

The GUI labels become Distortion / Filter / Volume in RAT mode. Filter turns
clockwise to remove treble. Host parameter IDs remain drive/tone/level, preserving
existing automation and saved states. RAT is host index 4 / settings number 5.
All previous model numbers remain unchanged. The 32-sample latency is unchanged;
the audio layout is now mono in, mono out, as stated above.
Processing and switching remain allocation-free and safe Rust.
Independent complex-response fixtures include the finite amplifier pole and
loaded clipping/filter network; tests also check slew/rails, recovery, filter
orientation, mute, reset and saved selections. Physical listening/measurement
and a dedicated RAT alias-energy comparison remain outstanding.

A subsequent release run of `block_cost` with all six models (same 64-frame,
48kHz, stereo, 8x configuration and stable mid-position controls) measured:

| Model | Median block | p95 block | Median share of block deadline |
|---|---:|---:|---:|
| TS808 | 178.209 us | 198.459 us | 13.366% |
| TS9 | 178.250 us | 201.375 us | 13.369% |
| OD-1 | 140.417 us | 158.584 us | 10.531% |
| SD-1 | 148.584 us | 166.958 us | 11.144% |
| RAT | 167.333 us | 188.667 us | 12.550% |

Raw data: `measurements/block_cost_64_8x_five_models.csv`. These are local
wall-clock measurements, not worst-case realtime guarantees. Compare models
within this run; scheduling and machine conditions differ from earlier tables.

### Nice Clone family model

Nice Dist replaces the provisional Lube Screecher package and plugin identity.
Run `cargo run -p nice-dist --release --features standalone`.
Its bundles are Nice Dist.clap / Nice Dist.vst3; settings are now
`nice-dist-settings.ron`. Provisional Lube Screecher host states and settings files
are not automatically migrated to this renamed plugin.

The Nice Clone model uses the same project-owned Centaur circuit as the separate
Nice Clone plugin, extracted into `nice-common::clone_drive`. There is one circuit
implementation, not a copied model or a nested plugin. Its Gain / Treble / Output
controls map to the existing drive / tone / level parameter IDs. Existing Nice Dist
model numbers remain unchanged; Clone is host index 5 / settings number 6.
The family engine provides smoothing, switching and oversampling once. Nice Dist
retains fixed 32-sample latency, including an explicit delay with oversampling Off;
separate Nice Clone retains its existing zero latency in Off mode. A regression
checks identical fixed-control samples against the existing Clone DSP after that
latency alignment. See [Nice Clone's circuit documentation](../nice-clone/README.md)
for component values, reference measurements and modelling limitations.
The separate Nice Clone package, plugin IDs and preferences remain available.
Historical timing tables above predate this sixth model.

The editor/window bridge is shared in [nice-plugin](../nice-plugin/README.md).
Plugin-specific view construction lives in `src/editor.rs`; standalone
activation follows the shared activate/reset sequence.
