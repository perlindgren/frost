# Pitch detector evaluation — 2026-11-03

## Recommendation

`pitch-estimate 0.1.0`, initially its YIN detector, is the strongest candidate for
an integration trial. In this controlled setup it produced the same number of
estimates within five cents as our current detector, used about 1/17 of its
end-to-end processing time, and allocated nothing during detection. Its published
tests and source review support that allocation claim. This is a recommendation
for a trial, not evidence that it will outperform the current detector on actual
guitar recordings.

The current detector is already inexpensive: about 42 ms of processing wall time per
12 seconds of audio in this benchmark, around 0.35% of the audio duration. Replacing it
is therefore principally a maintenance and implementation choice, rather than an
urgent performance fix. Keep the current test suite and add recorded-input tests
before making an application dependency change.

No application manifests, application lockfile, or detector implementation were
changed for this evaluation. The benchmark is a separate Cargo workspace with
its own pinned dependencies and lockfile. No third-party source was patched or
vendored. Downloaded archives matched their lockfile checksums; extracted files
were checked byte-for-byte against those archives after the reviews. See
[the integrity record](results/source-integrity.json).

## Measured results

These are **our measurements**, not numbers copied from crate descriptions.
Release build on this Mac, Darwin/arm64, rustc 1.99.1. The precise CPU model was
not accessible in the restricted shell. Timings are from a shared desktop, not
an isolated performance lab; the final timing run had no benchmark compilation
or review-test jobs running alongside it.

| Implementation | Processing ms / 12 s audio | Median analysis-call µs | Processing allocations / analysis | Frames within ±5 cents |
|---|---:|---:|---:|---:|
| Current local YIN | 41.944 | 294.812 | 0 | 82.13% |
| pitch-estimate YIN | 2.407 | 8.875 | 0 | 82.13% |
| pitch-estimate McLeod | 2.600 | 10.062 | 0 | 80.74% |
| pitch-detection McLeod | 7.366 | 44.750 | 14 | 70.93% |
| pitch-detection YIN, current confidence gate | 4.813 | 25.688 | 14 | 0%* |
| pitch-detector windowed FFT | 5.604 | 33.541 | 22 | 73.98% |
| pitch-detector power cepstrum | 5.490 | 32.750 | about 27 | 5.65% |
| crates.io pitch / BCF, native-rate frames | 10.443 | 76.292 | 1 | 15.79% |

* `pitch-detection` YIN returned negative clarity on clean tones, despite good
frequency estimates. Our existing clarity gate rejects those results. Ignoring
its clarity **for diagnosis only** yields 84.81% within five cents; that diagnostic
result must not be presented as a detector compatible with our confidence gate.
The raw reproduction includes 110 Hz -> approximately 110.004 Hz with clarity
-0.32646. See [the direct public-API probe](results/raw-yin-probe.txt).

The accuracy column counts successful estimates out of **all expected analysis
windows**, including misses. It is not accuracy conditional on getting a result.
Each implementation had 2,160 voiced analysis opportunities. Thresholds and
failure behavior differ between APIs; the table is an integration comparison,
not a mathematical proof that one pitch algorithm is universally better.

The difficult upper-register cases substantially affect the aggregate. Restricted
to the tested fundamentals at or below 660 Hz, both local YIN and pitch-estimate
YIN reached 98.48% within five cents. Both passed all 192 clean-sine opportunities
in that range. They shared high-note and harmonic octave problems in the larger
suite. Neither implementation should be described as universally accurate from
these results.

The FFT and BCF APIs do not expose an equivalent clarity score. Their numeric
frequency results were evaluated without inventing a confidence algorithm. The
FFT detector reported pitch on all 16 noise opportunities; the BCF paths also
showed false positives on some unpitched/transient inputs. The YIN/McLeod paths
with functioning confidence gates rejected the tested noise. These results matter
for a tuner even when a frequency-only estimator appears fast.

## Method

- Pinned published packages: pitch-estimate 0.1.0, pitch-detection 0.3.0,
  pitch-detector 0.3.1, and pitch 0.2.0. Local comparison: the current repository's
  `nice-tuner/crates/pitch`, configured as the tuner uses it: 30–2100 Hz,
  downsampling factor four, minimum clarity 0.85.
- Host rates 44,100 and 48,000 Hz; mono f32 input in 256-sample blocks. For the
  buffered libraries the adapter reproduces our actual two-pole f64 IIR filtering
  and decimation, uses 2,048 analysis samples, and advances by 1,024 samples.
  Both algorithms in `pitch-detector` require f64; conversion uses preallocated
  storage and its work is included in the timings.
- Our code's actual cutoff is 0.45 times the **analysis sample rate**. The local
  crate's opening documentation describes a different cutoff; this harness
  follows the implementation, not that description. No source was changed.
- pitch-estimate YIN uses the local detector's 0.10 absolute threshold and an
  integration length of 2,048 minus floor(analysis_rate / 30). Its McLeod path
  searches through half the frame, leaving room to close low-frequency peak
  lobes, with clarity threshold 0.85. pitch-detection uses its public
  `get_pitch(..., power_threshold=0, clarity_threshold=0.85)` interface and
  2,048 padding samples. Its YIN integration length is fixed internally at half
  the frame and its absolute threshold is consequently 0.15. Those differences
  are part of the published APIs and were not patched away.
- Fifteen fundamentals from 30.87 to 2090 Hz; nine deterministic conditions:
  sine, seven cents sharp, seven harmonics, weak fundamental, missing fundamental,
  lower noise, higher noise, a plucked envelope, and a small tone on a DC offset.
  Each clip lasts 0.85 s. The sharp highest test remains below 2100 Hz. Noise is
  uniform, with peak amplitudes 0.025 and 0.08 added to the harmonic signal;
  the case names do not claim an exact calibrated SNR. The plucked envelope has
  a 6 ms attack and exponential decay.
- Separate silence, pure noise, DC, NaN, infinity and large-amplitude periodic
  stress cases. Large finite periodic input is a numerical-stability case, not
  an input that must be classified unvoiced. Finite out-of-range frequencies and
  invalid confidence values are rejected by the comparison adapter.
- BCF was checked both after the common decimator and at its intended native
  input rate. The primary table uses native f64 frames of 8,192 samples and
  4,096-sample hops, preserving the same observation time. Its public API
  hard-codes 48 kHz; returned Hz are rescaled for the actual input rate without
  changing the crate. The common-decimator variant is retained in the CSV as
  `registry_bcf_decimated` (15.09% within five cents).
- Timing: 12 s harmonic signal, 1 s warm-up, five trials per rate, rotating
  implementation order. Values above are medians over the ten trials. The
  analysis-call column excludes blocks that only buffer audio. Signal generation
  and detector construction are outside the timed region. Streaming frontend,
  preprocessing, output checks, timer overhead and allocation-counter overhead
  are included. The harness does not time the GUI, device drivers or resampling.
- The counting allocator records alloc, alloc_zeroed and realloc after warm-up,
  with caller-owned timing storage reserved beforehand. First-analysis allocation
  counts are also recorded separately; they exclude constructor allocations.
- Shared dependencies resolve through the isolated benchmark lockfile, including
  rustfft 6.4.1 and realfft 3.5.1. pitch-estimate's own published test lockfile uses
  realfft 3.5.0. The independent tests therefore also exercise its published lock.

Holding the same window and hop means the faster implementations **do not reduce
observation latency here**: first full windows arrive at about 171 ms at 48 kHz
and 186 ms at 44.1 kHz. Smaller-window configurations require another accuracy
study, especially for low bass notes.

## Code and test review

| Component | Tests actually run | Clippy with warnings denied | Main findings |
|---|---|---|---|
| Local pitch | 19 release tests/doctests passed | Passed | Preallocates buffers; readable streaming API and boundary handling. Direct YIN difference calculation costs more CPU. |
| pitch-estimate | 34 tests/doctests passed in debug and release | Passed | Owns FFT plans and all scratch storage; forbids unsafe code; validates constructor arguments and screens invalid/overflowing frames. |
| pitch-detection | 27 tests/doctests passed in debug and release | Failed: 6 lint findings | Buffer pooling is lazy, FFT plans are rebuilt during calls, and Rc/RefCell makes detectors unsuitable for moving between threads. YIN clarity regression is not caught by its frequency assertions. |
| pitch-detector | 3 unit tests passed; 4 failed because recordings are absent from the published package | Failed: 7 lint findings | Per-call FFT planning and spectrum/iterator allocations; no equivalent clarity gate. One cello test explicitly accepts the octave-high estimate; a noise test is disabled. |
| crates.io pitch | Published library has 0 unit tests; its sole doctest is ignored | Passed | Hard-coded sample rate, per-call bit-vector allocation, integer-period estimates, no confidence output; very limited validation evidence in the published package. |

The older libraries' style-lint failures alone are not correctness failures and
would normally be capped when consumed as dependencies. Their allocation and
confidence behavior are more important integration issues.

pitch-estimate's tests include f32/f64 signal conformance, an independent
FFT-versus-time-domain comparison, confidence bounds, low-frequency regressions,
DC/overflow handling and allocation counting. Those tests are useful evidence,
but it remains a young 0.1.0 project with little published history. Do not equate
passing tests with production maturity.

For packages without a usable published test lockfile, review manifests compiled
the original downloaded library/test source by absolute path, leaving those
source files unchanged. The pitch-detection driver points at its shipped WAV
fixtures. The pitch-detector archive does not contain its required `test_data`
recordings, so those tests could not be completed from the published artifact;
that packaging limitation is distinct from a demonstrated detector failure.
Benchmark-only upstream development dependencies were omitted from these test
manifests. Benchmarks/examples and all optional feature combinations were not
claimed as tested.

The additional libraries were reviewed in release mode; pitch-estimate and
pitch-detection also received debug test runs. See the raw logs, including failed
commands, under [results](results/).

## Limits and next step

This is a deterministic synthetic-signal benchmark plus published upstream test
execution, not a calibrated guitar/bass recording study. Sources' own recorded
fixtures help exercise integration but do not establish ground-truth cents errors
for our use case. This machine's timing noise, parameter choices, common filtering,
5-cent threshold, and intentionally difficult signal distribution affect rankings.

Before a replacement, run pitch-estimate YIN alongside the current detector on
real guitar/bass recordings, compare octave stability and time to a stable note,
and investigate their shared upper-register errors. Keep the preprocessing and
streaming wrapper in project-owned code and use only public dependency APIs.

## Reproduce

This directory is an independent Cargo workspace; its lockfile does not affect
the application. From the frost repository root:

```sh
BENCH=nice-tuner/docs/pitch-benchmark
cargo build --locked --release --manifest-path "$BENCH/Cargo.toml"
"$BENCH/target/release/frost-pitch-study" accuracy > accuracy.csv
"$BENCH/target/release/frost-pitch-study" allocations > allocations.csv
"$BENCH/target/release/frost-pitch-study" performance > performance.csv
"$BENCH/target/release/frost-pitch-study" probe
python "$BENCH/review_upstream.py"
```

Use an idle machine for timing. The review script creates a separate temporary
Cargo cache and preserves third-party source. The initial downloads need network
access. It records test/lint exit codes rather than hiding expected failures.

## Sources

- [pitch-estimate documentation and published test descriptions](https://docs.rs/crate/pitch-estimate/0.1.0)
- [pitch-estimate repository](https://github.com/hey-jj/pitch-estimate)
- [pitch-detection published source](https://docs.rs/crate/pitch-detection/0.3.0/source/)
- [pitch-detection YIN source](https://docs.rs/pitch-detection/0.3.0/src/pitch_detection/detector/yin.rs.html)
- [pitch-detector source](https://docs.rs/crate/pitch-detector/0.3.1/source/)
- [crates.io pitch source](https://docs.rs/crate/pitch/0.2.0/source/)
