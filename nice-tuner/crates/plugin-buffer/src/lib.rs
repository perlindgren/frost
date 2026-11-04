//! Owned planar audio storage with safe scoped access to nice-plug's Buffer.
#![deny(
    unsafe_op_in_unsafe_fn,
    clippy::undocumented_unsafe_blocks,
    missing_docs
)]

use nice_plug_core::buffer::Buffer;

/// Fixed-capacity heapless sample storage, boxed once to keep its address
/// stable. Buffer references cannot escape a processing call. Moving this
/// value does not move its heap allocation.
///
/// ```compile_fail
/// let mut storage = plugin_buffer::PlanarBuffer::<32>::new(2, 16);
/// let escaped = storage.with_buffer(8, |buffer| std::mem::take(buffer));
/// drop(storage);
/// println!("{}", escaped.samples());
/// ```
///
/// ```compile_fail
/// let mut storage = plugin_buffer::PlanarBuffer::<32>::new(2, 16);
/// let escaped = storage.with_buffer(8, |buffer| {
///     std::mem::take(&mut buffer.as_slice()[0])
/// });
/// drop(storage);
/// println!("{}", escaped[0]);
/// ```
pub struct PlanarBuffer<const SAMPLES: usize> {
    // Declaration order drops borrowed slices before their backing storage.
    buffer: Buffer<'static>,
    storage: Box<heapless::Vec<f32, SAMPLES>>,
    channels: usize,
    capacity: usize,
}

impl<const SAMPLES: usize> PlanarBuffer<SAMPLES> {
    /// Allocate channel storage and slice metadata. Both dimensions must be
    /// nonzero and their product must fit SAMPLES; otherwise this panics.
    pub fn new(channels: usize, capacity: usize) -> Self {
        assert!(
            channels > 0 && capacity > 0,
            "Buffer dimensions must be nonzero"
        );
        let samples = channels
            .checked_mul(capacity)
            .expect("Buffer size overflow");
        assert!(samples <= SAMPLES, "Samples exceed fixed storage capacity");
        let mut storage = Box::new(heapless::Vec::new());
        storage
            .resize(samples, 0.0)
            .expect("Checked sample capacity");
        let mut buffer = Buffer::default();
        // SAFETY: No slices are installed, so all channel-length/liveness
        // requirements are vacuous. Only metadata capacity is reserved.
        unsafe { buffer.set_slices(0, |slices| slices.reserve(channels)) };
        Self {
            buffer,
            storage,
            channels,
            capacity,
        }
    }

    /// Run a callback with equally sized channel slices, at most `capacity`
    /// frames long. Panics if frames exceeds capacity. Normal processing reuses
    /// all allocations; replacing the Buffer can discard its metadata capacity.
    ///
    /// The higher-ranked callback prevents returning/storing references to the
    /// samples or the Buffer beyond this call, including through mem::take.
    pub fn with_buffer<R>(
        &mut self,
        frames: usize,
        process: impl for<'samples> FnOnce(&mut Buffer<'samples>) -> R,
    ) -> R {
        assert!(
            frames <= self.capacity,
            "Frame count exceeds buffer capacity"
        );
        // Clear stale metadata before accessing backing memory. Always rebuild
        // from our allocation, not from slices a callback may have shortened or
        // replaced. That also handles a previous callback unwinding.
        reset(&mut self.buffer);
        let pointer = self.storage.as_mut_ptr();
        // SAFETY: channels*capacity was checked when allocating. Each pointer
        // addresses a disjoint channel of that fixed allocation; frames is
        // bounded by capacity. Slices are initialized and equally sized. The
        // apparent static lifetime is private: the higher-ranked callback
        // cannot let these references escape, and Reset clears them before the
        // storage is reused or dropped, also when the callback panics.
        unsafe {
            self.buffer.set_slices(frames, |slices| {
                for channel in 0..self.channels {
                    slices.push(std::slice::from_raw_parts_mut(
                        pointer.add(channel * self.capacity),
                        frames,
                    ));
                }
            });
        }
        let reset = Reset(&mut self.buffer);
        process(reset.0)
    }
}

fn reset(buffer: &mut Buffer<'static>) {
    // SAFETY: Removing all references leaves a zero-frame empty Buffer. It
    // neither reads nor mutates sample storage, and cannot invoke user code.
    unsafe { buffer.set_slices(0, Vec::clear) };
}

struct Reset<'a>(&'a mut Buffer<'static>);
impl Drop for Reset<'_> {
    fn drop(&mut self) {
        reset(self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_are_disjoint_and_short_blocks_can_grow_again() {
        let mut storage = PlanarBuffer::<32>::new(2, 16);
        storage.with_buffer(1, |buffer| {
            buffer.as_slice()[0][0] = 0.25;
            buffer.as_slice()[1][0] = 0.75;
            // Safe caller mutations must not become our next backing pointer.
            buffer.as_slice()[0] = &mut [];
        });
        storage.with_buffer(16, |buffer| {
            assert_eq!(buffer.samples(), 16);
            assert_eq!(buffer.as_slice()[0].len(), 16);
            assert_eq!(buffer.as_slice()[0][0], 0.25);
            assert_eq!(buffer.as_slice()[1][0], 0.75);
        });
        storage.with_buffer(0, |buffer| {
            assert_eq!(buffer.channels(), 2);
            assert!(buffer.is_empty());
        });
    }

    #[test]
    fn panic_and_buffer_replacement_leave_storage_usable() {
        let mut storage = PlanarBuffer::<32>::new(2, 16);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            storage.with_buffer(2, |_| panic!("callback panic"));
        }));
        assert!(result.is_err());
        storage.with_buffer(2, |buffer| {
            let _ = std::mem::take(buffer);
        });
        storage.with_buffer(16, |buffer| assert_eq!(buffer.channels(), 2));
    }

    #[test]
    #[should_panic(expected = "Frame count exceeds buffer capacity")]
    fn oversized_blocks_are_rejected() {
        PlanarBuffer::<32>::new(2, 16).with_buffer(17, |_| {});
    }

    #[test]
    #[should_panic(expected = "Samples exceed fixed storage capacity")]
    fn insufficient_sample_capacity_is_rejected() {
        PlanarBuffer::<31>::new(2, 16);
    }

    #[test]
    fn normal_processing_does_not_allocate() {
        let mut storage = PlanarBuffer::<32>::new(2, 16);
        let probe = allocation_counter::measure(|| {
            std::hint::black_box(vec![0u8; 32]);
        });
        assert!(probe.count_total > 0);
        let count = allocation_counter::measure(|| {
            for frames in [0, 1, 16, 3, 16].into_iter().cycle().take(1000) {
                storage.with_buffer(frames, |buffer| {
                    for channel in buffer.as_slice() {
                        channel.fill(0.25);
                    }
                });
            }
        });
        assert_eq!(count.count_total, 0);
    }
}
