//! Safe, read-only access to the CoreAudio properties needed by nice-tuner.
//!
//! Only available on macOS. No raw property descriptors, pointers or setters
//! are exposed. Device disappearance and unsupported properties return errors.
//! See `SAFETY.md` for the native contracts behind the unsafe implementation.
//!
//! ```no_run
//! # #[cfg(target_os = "macos")]
//! # fn main() -> Result<(), core_audio_device::Error> {
//! use core_audio_device::Device;
//! if let Some(device) = Device::find_by_name("MacBook Pro Microphone")? {
//!     let channels = device.preferred_input_channels()?;
//!     let frames = device.buffer_frames()?;
//!     println!("{channels:?}, {frames} frames");
//! }
//! # Ok(())
//! # }
//! # #[cfg(not(target_os = "macos"))]
//! # fn main() {}
//! ```

#![cfg(target_os = "macos")]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(missing_docs)]
#![deny(clippy::undocumented_unsafe_blocks)]

use objc2_core_audio::*;
use objc2_core_foundation::{CFRetained, CFString};
use std::fmt;
use std::ptr::{NonNull, null};

/// A failed read-only device query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// CoreAudio returned this OSStatus unchanged.
    System(i32),
    /// Returned data was too large, incomplete, or not the documented size.
    UnexpectedSize,
    /// A successful name query returned a null CFString.
    NullName,
    /// The device returned zero for a channel index or buffer frame count.
    InvalidValue,
    /// The device list changed during every bounded read attempt.
    DeviceListChanged,
    /// Storage for the device list could not be allocated.
    AllocationFailed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::System(status) => write!(f, "CoreAudio returned OSStatus {status}"),
            Self::UnexpectedSize => f.write_str("CoreAudio returned an unexpected property size"),
            Self::NullName => f.write_str("CoreAudio returned a null device name"),
            Self::InvalidValue => f.write_str("CoreAudio returned a zero channel or buffer size"),
            Self::DeviceListChanged => f.write_str("CoreAudio's device list kept changing"),
            Self::AllocationFailed => f.write_str("Could not allocate the CoreAudio device list"),
        }
    }
}

impl std::error::Error for Error {}

/// A device identifier, not ownership of a stream or a guarantee of liveness.
///
/// Each query checks CoreAudio's result again, so unplugging a device does not
/// leave a Rust reference to a destroyed native object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device(u32);

impl Device {
    /// Enumerate currently visible devices. Does not open or modify any stream.
    pub fn all() -> Result<Vec<Self>, Error> {
        Ok(device_ids()?.into_iter().map(Self).collect())
    }

    /// Find the first device whose name matches exactly.
    ///
    /// Devices whose names cannot be read (for example after unplugging) are
    /// skipped. Duplicate names retain CoreAudio's enumeration order.
    pub fn find_by_name(name: &str) -> Result<Option<Self>, Error> {
        Ok(Self::all()?
            .into_iter()
            .find(|device| device.name().is_ok_and(|candidate| candidate == name)))
    }

    /// Read the device's display name and release the returned native string.
    pub fn name(self) -> Result<String, Error> {
        let address = address(kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal);
        // An actual pointer-sized, aligned output slot, initialized even if the
        // native query fails or writes fewer bytes than requested.
        let mut pointer: *mut CFString = std::ptr::null_mut();
        let mut bytes = size_of_val(&pointer) as u32;
        // SAFETY: This fixed selector returns a caller-owned CFStringRef per
        // AudioHardwareBase.h. The output is exactly one aligned pointer slot;
        // address, byte count and output slot live throughout this synchronous
        // call. No qualifiers are required, hence the null/zero pair.
        let status = unsafe {
            AudioObjectGetPropertyData(
                self.0,
                NonNull::from(&address),
                0,
                null(),
                NonNull::from(&mut bytes),
                NonNull::from(&mut pointer).cast(),
            )
        };
        check_status(status)?;
        // SAFETY: A successful read of the fixed name selector transfers one
        // retain on a CFString. take_name validates its output size and null
        // result before adopting it; this is the only consumer of that retain.
        unsafe { take_name(pointer, bytes) }
    }

    /// Read the preferred one-based stereo input channel indices.
    ///
    /// This is a preferred stereo mapping, not an independent mono-channel
    /// default. The caller may choose its first member as its mono policy.
    pub fn preferred_input_channels(self) -> Result<[u32; 2], Error> {
        self.channels(WordsProperty::InputChannels)
    }

    /// Read the preferred one-based stereo output channel indices.
    pub fn preferred_output_channels(self) -> Result<[u32; 2], Error> {
        self.channels(WordsProperty::OutputChannels)
    }

    /// Read the device's current buffer frame count, not its supported range.
    pub fn buffer_frames(self) -> Result<u32, Error> {
        let [frames] = read_fixed_words(self.0, WordsProperty::BufferFrames)?;
        if frames == 0 {
            return Err(Error::InvalidValue);
        }
        Ok(frames)
    }

    fn channels(self, property: WordsProperty) -> Result<[u32; 2], Error> {
        let channels = read_fixed_words(self.0, property)?;
        validate_channels(channels)
    }
}

// A closed list of properties whose native representation consists of u32s.
// Callers cannot forge a descriptor or substitute a pointer-valued property.
#[derive(Clone, Copy)]
enum WordsProperty {
    Devices,
    InputChannels,
    OutputChannels,
    BufferFrames,
}

impl WordsProperty {
    fn address(self) -> AudioObjectPropertyAddress {
        let (selector, scope) = match self {
            Self::Devices => (
                kAudioHardwarePropertyDevices,
                kAudioObjectPropertyScopeGlobal,
            ),
            Self::InputChannels => (
                kAudioDevicePropertyPreferredChannelsForStereo,
                kAudioObjectPropertyScopeInput,
            ),
            Self::OutputChannels => (
                kAudioDevicePropertyPreferredChannelsForStereo,
                kAudioObjectPropertyScopeOutput,
            ),
            Self::BufferFrames => (
                kAudioDevicePropertyBufferFrameSize,
                kAudioObjectPropertyScopeGlobal,
            ),
        };
        address(selector, scope)
    }
}

fn address(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: kAudioObjectPropertyElementMain,
    }
}

fn check_status(status: i32) -> Result<(), Error> {
    if status == 0 {
        Ok(())
    } else {
        Err(Error::System(status))
    }
}

/// Adopt and copy a successful CoreAudio name result.
///
/// # Safety
/// If `bytes` equals the pointer size and `pointer` is non-null, it must point
/// to a live CFString with one owned retain transferred to this call. No other
/// owner may consume that retain. Other size/null results are rejected without
/// dereferencing the pointer.
unsafe fn take_name(pointer: *mut CFString, bytes: u32) -> Result<String, Error> {
    if bytes as usize != size_of_val(&pointer) {
        return Err(Error::UnexpectedSize);
    }
    let pointer = NonNull::new(pointer).ok_or(Error::NullName)?;
    // SAFETY: The caller supplies a live +1 CFString. Adopt that retain once;
    // CFRetained releases it on drop, including unwinding during conversion.
    // Only an owned Rust String escapes this function.
    let string = unsafe { CFRetained::from_raw(pointer) };
    Ok(string.to_string())
}

fn validate_channels(channels: [u32; 2]) -> Result<[u32; 2], Error> {
    if channels.contains(&0) {
        Err(Error::InvalidValue)
    } else {
        Ok(channels)
    }
}

fn word_count(bytes: usize) -> Result<usize, Error> {
    if bytes.is_multiple_of(size_of::<u32>()) {
        Ok(bytes / size_of::<u32>())
    } else {
        Err(Error::UnexpectedSize)
    }
}

// Validate the native size result before any caller consumes output. The safe
// callback seam also lets tests simulate failure, truncation and list changes
// without supplying fake pointers to an unsafe native API.
fn read_words_with(
    output: &mut [u32],
    call: impl FnOnce(&mut [u32], &mut u32) -> i32,
) -> Result<usize, Error> {
    let capacity = u32::try_from(size_of_val(output)).map_err(|_| Error::UnexpectedSize)?;
    let mut bytes = capacity;
    check_status(call(output, &mut bytes))?;
    if bytes > capacity {
        return Err(Error::UnexpectedSize);
    }
    word_count(bytes as usize)
}

fn read_words(id: u32, property: WordsProperty, output: &mut [u32]) -> Result<usize, Error> {
    let address = property.address();
    read_words_with(output, |output, bytes| {
        // SAFETY: The closed WordsProperty enum selects only arrays/scalars of
        // UInt32. The initialized u32 slice has native alignment and every bit
        // pattern is valid. ioDataSize starts at its exact byte capacity; the
        // synchronous native API may write at most that many bytes. Even an
        // empty slice has a non-null aligned pointer and advertises zero bytes.
        // The address and byte count remain live; no qualifiers are needed.
        unsafe {
            AudioObjectGetPropertyData(
                id,
                NonNull::from(&address),
                0,
                null(),
                NonNull::from(bytes),
                NonNull::new(output.as_mut_ptr()).unwrap().cast(),
            )
        }
    })
}

fn read_fixed_words<const N: usize>(id: u32, property: WordsProperty) -> Result<[u32; N], Error> {
    let mut output = [0; N];
    let count = read_words(id, property, &mut output)?;
    if count != N {
        return Err(Error::UnexpectedSize);
    }
    Ok(output)
}

fn device_list_size() -> Result<usize, Error> {
    let address = WordsProperty::Devices.address();
    let mut bytes = 0;
    // SAFETY: The system object and fixed device-list address are valid input
    // values. The only output is this live, initialized, aligned UInt32 slot.
    // CoreAudio retains no pointers, and the property takes no qualifiers.
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            kAudioObjectSystemObject as u32,
            NonNull::from(&address),
            0,
            null(),
            NonNull::from(&mut bytes),
        )
    };
    check_status(status)?;
    word_count(bytes as usize)
}

fn device_ids() -> Result<Vec<u32>, Error> {
    device_ids_with(device_list_size, |output| {
        read_words(
            kAudioObjectSystemObject as u32,
            WordsProperty::Devices,
            output,
        )
    })
}

fn device_ids_with(
    mut size: impl FnMut() -> Result<usize, Error>,
    mut read: impl FnMut(&mut [u32]) -> Result<usize, Error>,
) -> Result<Vec<u32>, Error> {
    // A device can appear between the size query and the read. Re-query and
    // reallocate on that specific native error; never retry forever.
    for _ in 0..3 {
        let count = size()?;
        if count == 0 {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        ids.try_reserve_exact(count)
            .map_err(|_| Error::AllocationFailed)?;
        ids.resize(count, 0);
        match read(&mut ids) {
            Ok(written) if written <= count => {
                ids.truncate(written);
                return Ok(ids);
            }
            Ok(_) => return Err(Error::UnexpectedSize),
            Err(Error::System(status)) if status == kAudioHardwareBadPropertySizeError => continue,
            Err(error) => return Err(error),
        }
    }
    Err(Error::DeviceListChanged)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_name_is_copied_without_consuming_another_owners_retain() {
        let original = CFString::from_str("Microphone 🎸 – 入力");
        for _ in 0..64 {
            let owned = CFRetained::into_raw(original.clone());
            // SAFETY: clone adds a retain, into_raw transfers it, and only
            // take_name consumes that retain. The original owns a separate one.
            let name = unsafe { take_name(owned.as_ptr(), size_of::<*mut CFString>() as u32) };
            assert_eq!(name.unwrap(), "Microphone 🎸 – 入力");
        }
        assert_eq!(original.to_string(), "Microphone 🎸 – 入力");
    }

    #[test]
    fn null_or_wrong_sized_names_are_rejected_before_dereference() {
        // SAFETY: A null pointer requires no ownership and is explicitly
        // permitted by take_name's contract for error results.
        let wrong_size = unsafe { take_name(std::ptr::null_mut(), 0) };
        assert_eq!(wrong_size, Err(Error::UnexpectedSize));
        // SAFETY: Same permitted null result, now with the correct slot size.
        let null_name =
            unsafe { take_name(std::ptr::null_mut(), size_of::<*mut CFString>() as u32) };
        assert_eq!(null_name, Err(Error::NullName));
    }

    #[test]
    fn partial_or_excess_native_lengths_are_rejected() {
        for bytes in [1, 3, 5, 9, u32::MAX] {
            assert_eq!(
                read_words_with(&mut [0; 2], |_, size| {
                    *size = bytes;
                    0
                }),
                Err(Error::UnexpectedSize)
            );
        }
    }

    #[test]
    fn shrinking_native_list_only_exposes_written_words() {
        let mut output = [0; 2];
        let written = read_words_with(&mut output, |words, bytes| {
            assert_eq!(*bytes, 8);
            words[0] = 123;
            *bytes = 4;
            0
        })
        .unwrap();
        assert_eq!(&output[..written], &[123]);
    }

    #[test]
    fn native_error_is_preserved_without_consuming_output() {
        assert_eq!(
            read_words_with(&mut [0; 2], |_, _| -50),
            Err(Error::System(-50))
        );
    }

    #[test]
    fn channel_pairs_are_one_based_and_may_repeat() {
        assert_eq!(validate_channels([0, 2]), Err(Error::InvalidValue));
        assert_eq!(validate_channels([2, 0]), Err(Error::InvalidValue));
        assert_eq!(validate_channels([1, 1]), Ok([1, 1]));
        assert_eq!(validate_channels([5, 8]), Ok([5, 8]));
    }

    #[test]
    fn empty_list_never_reads_a_zero_capacity_buffer() {
        assert_eq!(
            device_ids_with(|| Ok(0), |_| panic!("unexpected read")),
            Ok(vec![])
        );
    }

    #[test]
    fn device_list_shrink_discards_unused_slots() {
        assert_eq!(
            device_ids_with(
                || Ok(3),
                |output| {
                    output[0] = 42;
                    Ok(1)
                }
            ),
            Ok(vec![42])
        );
    }

    #[test]
    fn device_list_growth_requeries_capacity() {
        let mut reads = 0;
        let mut sizes = [1, 2].into_iter();
        let ids = device_ids_with(
            || Ok(sizes.next().unwrap()),
            |output| {
                reads += 1;
                if reads == 1 {
                    Err(Error::System(kAudioHardwareBadPropertySizeError))
                } else {
                    output.copy_from_slice(&[42, 43]);
                    Ok(2)
                }
            },
        )
        .unwrap();
        assert_eq!(reads, 2);
        assert_eq!(ids, [42, 43]);
    }

    #[test]
    fn continuous_device_list_growth_is_bounded() {
        let mut reads = 0;
        assert_eq!(
            device_ids_with(
                || Ok(1),
                |_| {
                    reads += 1;
                    Err(Error::System(kAudioHardwareBadPropertySizeError))
                }
            ),
            Err(Error::DeviceListChanged)
        );
        assert_eq!(reads, 3);
    }

    #[test]
    fn unrelated_list_errors_are_not_retried() {
        let mut reads = 0;
        assert_eq!(
            device_ids_with(
                || Ok(1),
                |_| {
                    reads += 1;
                    Err(Error::System(-50))
                }
            ),
            Err(Error::System(-50))
        );
        assert_eq!(reads, 1);
    }

    #[test]
    fn oversized_lists_fail_before_allocation_or_reading() {
        assert_eq!(
            device_ids_with(|| Ok(usize::MAX), |_| panic!("unexpected read")),
            Err(Error::AllocationFailed)
        );
        assert_eq!(
            device_ids_with(|| Ok(1), |_| Ok(2)),
            Err(Error::UnexpectedSize)
        );
    }

    #[test]
    fn invalid_device_returns_errors_without_opening_streams() {
        let missing = Device(u32::MAX);
        assert!(matches!(missing.name(), Err(Error::System(_))));
        assert!(matches!(missing.buffer_frames(), Err(Error::System(_))));
        assert!(matches!(
            missing.preferred_input_channels(),
            Err(Error::System(_))
        ));
        assert!(matches!(
            missing.preferred_output_channels(),
            Err(Error::System(_))
        ));
    }
}
