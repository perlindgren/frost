# nice-clone

A component-based Centaur-style overdrive, with CLAP/VST3 exports and the same
frost editor, nice-common knobs, meters, audio settings, and standalone transport
as the other pedals. Gain, Treble, and Output are normalized physical knob
positions, not generic gain/EQ/mix controls. Existing parameter IDs and saved
settings remain compatible with the scaffold.

## Circuit basis and accuracy

The target is the behavior of the Behringer Centaur Overdrive copy. Its manual
confirms the dual-ganged gain control, germanium clipping, and three controls,
but does not supply a schematic or diode measurements. This implementation uses
Martin Chittum's corrected April 2009 Centaur trace as the circuit basis. It is
not a verified trace or measured reproduction of Behringer's implementation.

The model includes:

- Input coupling and buffer headroom, both halves of the 100k gain pot, and the
  frequency-dependent U1B feedback network.
- A nominal TL072 gain-stage model with finite open-loop gain (200k), a 3 MHz
  dominant-pole gain-bandwidth product, rail saturation, and feedback-capacitor
  state retained during saturation.
- Antiparallel germanium diodes solved with their surrounding impedances and
  coupling capacitors. Both filtered clean paths remain electrically connected
  to the wet-path network; there is no arbitrary dry/wet crossfade.
- The summing stage, component-derived treble transfer, output coupling, linear
  10k output pot, 100k load, and effect-on leakage through the bypass bridge.
- Selectable Off, 2×, 4×, or 8× oversampling. The Blackman-windowed sinc
  interpolation/decimation filters use 65, 129, or 257 taps respectively.
  Reported latency is zero with oversampling Off and 32 host samples in each
  oversampled mode (0.67 ms at 48 kHz), excluding circuit phase response.
- Independent left/right circuits, mono support, and 10 ms control smoothing.
  Resistance matrices update every 16 host samples, independently of host block
  boundaries; capacitor voltages and currents survive control changes.

The diode fit uses Is = 200 nA and nVt = 41 mV, approximately 0.35 V at 1 mA.
That is a nominal fit, not a measured Behringer or vintage diode pair. U1 signal
headroom is approximated as +/-3 V about the bias; U2 output headroom as +/-11.5 V.
The buffer, summer, and tone op-amps otherwise use ideal transfer functions with
output clamps. The summer/tone clamps do not feed saturation back into their
linear filter state. Slew limiting, higher-order op-amp poles, output impedance,
phase reversal, component tolerances, temperature, junction capacitance, noise,
and charge-pump ripple/sag are not modeled. The nominal model is deterministic.

The treble midpoint follows the traced component network, including its slight
boost; it is not forcibly equalized to an ideal flat response. Output positions
use the traced linear pot law. Neither pot tapers nor tolerances have been
verified on a Behringer unit.

## Oversampling and CPU

The **Oversampling** dropdown above the knobs is available in the plugin and
standalone. In CLAP/VST3, the upper-right settings cog also opens a plugin
settings page with the same dropdown. Device settings remain standalone-only. **8×** remains the default, including when loading older sessions or
settings files. Lower factors reduce both circuit evaluations and resampling
work. **Off** runs the nonlinear circuit directly at the host sample rate and
skips all resampling filters. Lower settings trade alias suppression for CPU.

The choice is saved in the host's plugin state or the standalone preferences.
It is a manual setting rather than an automatable control because it can change
processing latency. Live changes fade down over 64 host samples, rebuild the
rate-dependent circuit/filter state at silence, then fade up over 64 samples.
The host receives a latency update when the active mode changes to or from Off.
Changing sample rate or resetting the plugin preserves the selected factor.

## Signal calibration

A digital sample of 1.0 is treated as **1 volt peak**, both in and out. Therefore,
a sine at -20 dBFS peak represents 100 mV peak at the pedal input. This explicit
convention determines clipping onset. Match the recording interface's voltage
calibration with host input gain when comparing recordings to a physical pedal.
The output is not normalized or limited to digital full scale.

The standalone remembers knob positions in `nice-clone-settings.ron`, alongside
shared audio and window settings. Drag vertically or use the mouse wheel; Shift
provides finer movement. The settings cog opens nice-common's audio panel.

```sh
cargo check -p nice-clone
cargo run -p nice-clone --features standalone --bin nice-clone-standalone
cargo xtask bundle nice-clone --release
cargo test -p nice-clone --features standalone
cargo clippy -p nice-clone --features standalone --all-targets -- -D warnings
```

## Validation

The checked-in reference generators use only Python's standard library. They
build a separate full-circuit, 27-unknown modified nodal system directly from the
schematic, independently of the Rust circuit partitioning. AC fixtures cover
multiple gain/treble/output positions, frequencies, and rates. Transient fixtures
exercise the diode nonlinearity and gain-stage saturation using bisection rather
than the production Newton solver. Both references share the stated nominal
component/device assumptions, so agreement validates the implementation, not a
particular hardware unit. This is not a SPICE macro-model comparison.

```sh
python3 nice-clone/tests/reference/ac_reference.py
python3 nice-clone/tests/reference/transient_reference.py
```

Tests also check nonlinear equation residuals, gain-dependent distortion, treble
and output response, oversampling delay and image rejection, folded distortion
harmonics, stereo separation, reset, host block boundaries, unusual inputs, and
allocation-free processing while controls move or oversampling switches, all
four factors, saved settings migration, and host latency notifications.

No hardware measurement or listening match is claimed. A captured calibration
signal, swept tones, and guitar DI/reamped pairs at known knob settings would be
needed to establish and refine a match to a particular Behringer pedal.

## Sources

- [Behringer Centaur Overdrive quick-start guide](https://device.report/m/3c0da66d0014aa9bd88f7b0df9d63e48fb6b2a01470f90098acef8c5f8f544d8.pdf):
  manufacturer control descriptions and specifications.
- [Martin Chittum's corrected April 2009 trace, hosted by Aion FX](https://aionfx.com/app/files/schematics/klon-centaur-schematic.png):
  topology, component values, pot connections, and diode voltage note. The
  original schematic image is linked, not redistributed here.
- [Texas Instruments TL072 information and datasheet](https://www.ti.com/product/TL072):
  nominal gain-bandwidth and device characteristics; the model is a simplified
  dominant-pole approximation, not the manufacturer's transistor model.
- [Jatin Chowdhury, A Comparison of Virtual Analog Modelling Techniques for
  Desktop and Embedded Implementations (2020)](https://arxiv.org/abs/2009.02833):
  prior circuit-modeling work and modeling context. No third-party implementation
  or trained model is incorporated.
