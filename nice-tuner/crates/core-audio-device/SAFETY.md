# Safety review of core-audio-device

This project-owned crate wraps only the read-only CoreAudio properties currently
needed by nice-tuner. It is not a general CoreAudio wrapper or a replacement for
CPAL. Its public API is safe; unsafe code is confined to the native boundary.
Review date: 2026-11-04.

## Native contracts checked

The installed Apple SDK headers `CoreAudio.framework/Headers/AudioHardware.h`
and `AudioHardwareBase.h` specify these representations:

| Property | Object / scope | Storage |
| --- | --- | --- |
| `kAudioHardwarePropertyDevices` | System / global | Array of `AudioObjectID` (`UInt32`) |
| `kAudioObjectPropertyName` | Device / global | `CFStringRef`; caller releases the returned object |
| `kAudioDevicePropertyPreferredChannelsForStereo` | Device / input or output | Two `UInt32` channel indices |
| `kAudioDevicePropertyBufferFrameSize` | Device / global | One `UInt32` frame count |

Each address uses the main element and no qualifier. Native calls are synchronous
and obey the caller-provided `ioDataSize` capacity. As with any FFI wrapper,
memory safety relies on CoreAudio and installed native drivers honoring these
contracts. A post-call size check cannot undo a native buffer overrun.

## Unsafe boundaries

1. **Numeric property reads:** a private closed enum selects only the numeric
   properties above. Output is initialized `[u32; N]` or `Vec<u32>`, with native
   UInt32 alignment and valid values for every bit pattern. The byte capacity
   is checked to fit a `u32` before the call. All pointers refer to live storage
   for the duration of that call; CoreAudio does not retain them. Returned byte
   counts must fit the supplied capacity and be divisible by four. Fixed reads
   must also return exactly the requested number of words.
2. **Device-list size:** the only native output is a live, initialized `u32`.
   Partial device identifiers are rejected. Empty lists do not request data.
   Reads use fallible allocation, truncate to the actual count on shrink, and
   retry at most three times on the native bad-property-size error when growing.
3. **Name read:** a fixed name selector writes to an initialized, aligned
   pointer-sized slot. Native errors and incorrect sizes are rejected. A null
   successful result is rejected before dereferencing anything.
4. **Name ownership:** the private unsafe `take_name` primitive requires a live
   CFString with one transferred retain, as guaranteed by the successful native
   name query. It adopts that retain exactly once using `CFRetained::from_raw`,
   copies into an owned Rust `String`, and releases the retain on drop (including
   unwinding). On malformed-size/error responses, no purported object pointer is
   followed or released because its validity/ownership cannot be established.

There are no byte-buffer casts to more-aligned numeric types, uninitialized
reads, public raw pointers, caller-defined property descriptors, callbacks,
listeners, setters, or native references tied to a device's lifetime. A Device
stores only an ID; removal produces a subsequent CoreAudio error. Name lookup
skips unreadable names and retains first-match behavior for duplicate names.

`unsafe_op_in_unsafe_fn` and `clippy::undocumented_unsafe_blocks` are denied.
The nice-tuner adapter itself uses `forbid(unsafe_code)`.

## Verification and limits

Unit tests exercise malformed byte sizes, capacity overruns in returned counts,
native error propagation, invalid channel indices, empty/shrinking/growing device
lists, bounded retries, failed allocations and native invalid-device queries.
An ownership test repeatedly transfers a separate retain on a real Unicode
CFString to the name primitive and checks that the original owner remains valid;
null and malformed-size name results are also tested.

Verification on this Mac: 13 wrapper tests and one compile-only documentation
example passed; all 51 nice-tuner tests passed with `standalone,unsafe` enabled.
Strict Clippy checks passed for all targets of both packages. The ordinary plugin
check passed with the unsafe-code guard enabled. The standalone device-list
helper linked and ran, reporting zero input/output devices in this execution
environment. The standalone buffer setup has since moved to the separate
`plugin-buffer` wrapper; both nice-tuner targets now unconditionally deny unsafe
code, including when Windows placement's `unsafe` feature is enabled.

Tests and source review are evidence, not a formal proof or an independent
security audit. Actual successful hardware queries still need a read-only check
in an environment where CoreAudio devices are visible. No audio settings or
streams are changed by this crate or its tests.
