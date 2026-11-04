# Review of coreaudio 0.6.2

Reviewed 2026-11-03 for replacing `nice-tuner/src/devices/core_audio.rs`.

**Recommendation: do not adopt this release yet.** Its typed property API covers
our queries, but the implementation has safety defects. A safe public API is
not sufficient evidence that the implementation maintains Rust's safety rules.
This review is focused on property access, not a complete audit of the crate.

## Artifact and scope

- Published package: https://crates.io/crates/coreaudio/0.6.2
- Source commit: `a33a6ffb61570a7f6a56d28f3ef5accd2b0281c2`
- Archive SHA256: `44f8d03f64a1e3a4fc093b793351ee901bceb52ee82a139072a808c2d4fec4bb`
- License: MIT; edition: 2024; macOS only.
- First published release: 2026-05-03; reviewed release: 2026-10-21.
- Repository API reported one contributor, one star and zero forks at review
  time. This indicates limited visible adoption, not proof of poor quality.

The published archive was extracted to a temporary directory. All 38 packaged
files were compared against the archive after testing and were unchanged.
Neither frost's dependency declarations nor dependency source were changed.

## Fit for nice-tuner

The following public APIs cover our current native queries without unsafe
blocks in nice-tuner:

| Query | API |
| --- | --- |
| Device enumeration | `AudioObject::<System>::default().devices()` |
| Device name | `get_property(DEVICE_NAME)` |
| Preferred input pair | `get_property(DEVICE_INPUT_PREFERRED_CHANNELS_FOR_STEREO)` |
| Preferred output pair | `get_property(DEVICE_OUTPUT_PREFERRED_CHANNELS_FOR_STEREO)` |
| Current buffer frames | `get_property(DEVICE_BUFFER_FRAME_SIZE)` |

`ChannelPair::as_array()` returns the one-based pair. We would preserve the
existing behavior of using its first input channel and retaining `None` on
unsupported queries. The preferred stereo pair is not a separate OS-designated
default mono input channel.

CPAL would continue owning streams; no reason to use this crate's IO procedures,
aggregate devices, listeners or process taps for the proposed replacement.
No optional crate features are needed for these queries. Its bindings largely
overlap dependencies already in our graph, although adopting it would expand
some binding features. Adding it would not resolve unsafe code in our separate
standalone plugin-buffer setup.

## Findings

### High: safe custom descriptors can cause invalid pointer dereferences

`CustomPropertyInfo` has a safe `From<AudioServerPlugInCustomPropertyInfo>`
implementation (`src/data_types/custom.rs:84`). All fields of the source struct
can be supplied by the caller. `AudioObject::get_custom` trusts that supplied
type (`src/object.rs:221–230`), and `read_string` converts the returned bytes
into a pointer and wraps/dereferences it as an owned CFString
(`src/read.rs:42–62`).

A safe caller can select the nominal sample-rate property, label its data as
CFString, and pass that descriptor to `get_custom`. On a device returning an
ordinary eight-byte sample rate, those floating-point bits become an invalid
CFString pointer. The type and length checks do not prevent this.

A reproducer using `#![forbid(unsafe_code)]` compiled successfully. It was
**not executed** because it deliberately requests an invalid dereference.
The essential construction is:

```rust,ignore
let info = coreaudio::CustomPropertyInfo::from(
    objc2_core_audio::AudioServerPlugInCustomPropertyInfo {
        mSelector: objc2_core_audio::kAudioDevicePropertyNominalSampleRate,
        mPropertyDataType:
            objc2_core_audio::kAudioServerPlugInCustomPropertyDataTypeCFString,
        mQualifierDataType: 0,
    },
);
device.get_custom(info, coreaudio::Scope::Global, 0, None)?;
```

Our planned typed queries do not use this API. Nevertheless, it demonstrates
that the crate's advertised safe boundary is not sound across its public API.

### Medium: typed pointer reads lack an alignment guarantee

Property storage is allocated as `Vec<u8>` (`src/object.rs:600–605`). Several
decoders cast byte pointers and use `std::ptr::read` for types requiring greater
alignment, for example `read_buffer_size_range` (`src/read.rs:135–144`) and
the sample-rate range decoder. Some encoders similarly use typed pointer writes
into byte allocations.

The allocation contract of `Vec<u8>` does not guarantee that alignment. Ordinary
macOS allocators may happen to provide it, so this review does not claim an
observed crash. The unsafe implementation still needs a guaranteed aligned
allocation, unaligned operations where appropriate, or bytewise decoding.
Our exact buffer-frame and channel-pair decoders use bytewise conversions;
the range/format APIs are where this additional concern appears.

### Low: convenience range methods need care

`BufferFrameSizeRange::valid_sizes()` doubles a `u32` without checked overflow
(`src/data_types/ranges.rs:42–48`). A safe public construction using a range
ending at `u32::MAX` reproducibly panicked in debug. With release overflow checks
disabled the loop can wrap to zero and fail to terminate. This is an artificial
boundary case, not evidence that normal hardware reports such a range.

That helper also enumerates only powers of two, and
`ValidSampleRates::valid_rates()` returns just each range's minimum. Neither
should be treated as a complete enumeration for arbitrary devices.

## Verification

On this Apple Silicon Mac, Rust 1.99.1:

- Published package's seven library tests passed.
- One documentation example compiled successfully (`no_run`).
- `clippy --all-targets -- -D warnings` failed with twelve instances of the
  newer `chunks_exact_to_as_chunks` style lint. Allowing that specific lint
  made the same check pass. These lint failures are not safety findings.
- An external probe with `#![forbid(unsafe_code)]` compiled all five query
  paths above. Running its read-only enumeration returned **zero devices**
  in this execution environment. Real device query results remain unverified.
- The published integration suite requires an aggregate device named
  `Test Subject` and changes rates, buffers and channel mappings. It was
  inspected but not run; it is not a hardware-independent test suite.
- The custom-descriptor misuse compiled; the overflow case was run and
  reproduced. No dependency implementation was modified to make tests pass.

## Decision

Functionally suitable, but not ready to recommend as the safety boundary for
nice-tuner at version 0.6.2. Reconsider after upstream addresses the unsafe
property-descriptor escape and alignment assumptions and adds regression tests.
Adoption would also need read-only checks on real devices, including unsupported
properties and disconnected devices. Do not patch or vendor it locally.

Source references, pinned to the published commit:

- [Custom property metadata](https://github.com/Keegan-Wilgermein/coreaudio/blob/a33a6ffb61570a7f6a56d28f3ef5accd2b0281c2/src/data_types/custom.rs#L84)
- [Property access and byte-buffer allocation](https://github.com/Keegan-Wilgermein/coreaudio/blob/a33a6ffb61570a7f6a56d28f3ef5accd2b0281c2/src/object.rs#L600)
- [Property decoding](https://github.com/Keegan-Wilgermein/coreaudio/blob/a33a6ffb61570a7f6a56d28f3ef5accd2b0281c2/src/read.rs#L42)
- [Range helpers](https://github.com/Keegan-Wilgermein/coreaudio/blob/a33a6ffb61570a7f6a56d28f3ef5accd2b0281c2/src/data_types/ranges.rs#L42)
- [Published package, dependencies and release history](https://docs.rs/crate/coreaudio/0.6.2)
