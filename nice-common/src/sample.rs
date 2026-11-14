//! Turning a device's samples into the `f32` a plugin speaks, and back, without
//! naming cpal in the shared code.
//!
//! A standalone host is generic over the sample format its device asked for:
//! [`crate::audio_io`] and the two standalone hosts build a callback per format
//! and hand the buffer to code that wants `f32`. That conversion used to be
//! written against `cpal::Sample` and `cpal::FromSample`, which is fine up to
//! the build that cannot name them — a Windows standalone drives one ASIO
//! driver
//! through `azo`, whose buffers are `f32` and which is not built on cpal at
//! all. So the shared code names this trait, and cpal keeps being what it is
//! where it exists: the thing that decides what the numbers mean.
//!
//! `impl Level for f32` is unconditional, and that one is not a guess about
//! cpal. cpal's conversions live in `dasp_sample`, which it re-exports
//! (`cpal-0.18.2/src/lib.rs:194`, `src/sample_format.rs:30`), and there
//! `FromSample<S> for S` returns its argument (`dasp_sample-0.11.0/src/conv.rs:574-579`)
//! while `Sample::EQUILIBRIUM` for `f32` is `0.0`
//! (`dasp_sample-0.11.0/src/lib.rs:274`). Identity in, identity out, silence at
//! zero — the same three answers, so the transport that only ever speaks `f32`
//! hears exactly what it heard before.
//!
//! The integer formats are delegated to cpal rather than written out here, and
//! that is a deliberate refusal to duplicate: their scaling has details in it
//! that a second answer gets wrong quietly. `i16` divides by 32 768
//! (`conv.rs:229-231`); `u16` is reached through `i16`, and `i16`'s own way
//! there branches on the sign because `i16::MIN + 32_768` would overflow
//! (`conv.rs:193-200`). Re-deriving those rules in this crate would put two
//! definitions of the volume of a Mac's output in the tree, disagreeing by one
//! least-significant bit somewhere no test on this machine can hear.
//!
//! Separate from [`crate::pcm`] on purpose: that module answers what a *driver*
//! writes into the buffers this host allocated — bytes, per channel, byte order
//! included — while this one answers what to do when the *device API* picked a
//! Rust type for its own callback. Two questions with the same-looking answer.

/// A sample format a host can be built for: one that converts to and from the
/// `f32` a plugin processes, and knows its own silence.
pub trait Level: Copy {
    /// The resting point of this format, which is not zero for the unsigned
    /// ones — a `u16` output filled with `0` is a hard clip at the bottom of
    /// its range, not silence.
    const SILENCE: Self;

    /// This sample as the plugin's `f32`.
    fn level(self) -> f32;

    /// The plugin's `f32` in this format.
    fn from_level(value: f32) -> Self;
}

impl Level for f32 {
    const SILENCE: Self = 0.0;
    #[inline]
    fn level(self) -> f32 {
        self
    }
    #[inline]
    fn from_level(value: f32) -> Self {
        value
    }
}

/// The formats cpal can build a stream for, keeping cpal's own arithmetic.
#[cfg(all(not(windows), feature = "devices"))]
macro_rules! level_via_cpal {
    ($($t:ty),* $(,)?) => {
        $(
            impl Level for $t {
                const SILENCE: Self = <Self as cpal::Sample>::EQUILIBRIUM;
                #[inline]
                fn level(self) -> f32 {
                    <f32 as cpal::FromSample<Self>>::from_sample_(self)
                }
                #[inline]
                fn from_level(value: f32) -> Self {
                    <Self as cpal::FromSample<f32>>::from_sample_(value)
                }
            }
        )*
    };
}

// `devices` is what pulls cpal in (`Cargo.toml`), and on Windows this section's
// target does not exist: the whole `cfg(not(windows))` is the same one the
// manifest states and `devices.rs` and `audio_io.rs` obey.
#[cfg(all(not(windows), feature = "devices"))]
level_via_cpal!(f64, i8, i16, i32, i64, u8, u16, u32, u64);

#[cfg(all(test, not(windows), feature = "devices"))]
mod tests {
    use super::Level;

    #[test]
    fn f32_is_the_identity_cpal_makes_it() {
        // The one hand-written half of this module, checked against the
        // dependency it stands in for: cpal's `f32` conversion is the blanket
        // `FromSample<S> for S` and its equilibrium is `0.0`, so agreeing with
        // those two is what makes the levels of a transport that only speaks
        // `f32` the levels it had before.
        assert_eq!(f32::SILENCE, <f32 as cpal::Sample>::EQUILIBRIUM);
        for i in 0..=40 {
            let level = -0.9f32 + i as f32 * 0.045;
            assert_eq!(level, f32::from_level(level));
            assert_eq!(level, level.level());
        }
    }

    /// Silence in a format has to read as silence to the plugin, and a level put
    /// into the format has to come back out as itself - the two things `route`
    /// and a standalone `Processor::run` actually rely on.
    ///
    /// The tolerance is a parameter because the wide formats are not limited by
    /// their own step: an `i64` holds far more precision than the `f32` it was
    /// scaled from, so its round trip is as good as `f32`'s mantissa allows
    /// (about 6e-8 of full scale) while an `i8` is limited by its 1/128 step.
    fn round_trip<T: Level + std::fmt::Debug>(tolerance: f32) {
        let silence = T::SILENCE;
        assert_eq!(
            0.0,
            silence.level(),
            "the resting point of {silence:?} is not silence"
        );
        for i in 0..=40 {
            let level = -0.9f32 + i as f32 * 0.045;
            let back = T::from_level(level).level();
            assert!(
                (back - level).abs() <= tolerance,
                "{level} through {} and back is {back}",
                std::any::type_name::<T>()
            );
        }
    }

    #[test]
    fn every_format_a_cpal_stream_is_built_for_round_trips() {
        round_trip::<f64>(0.0);
        round_trip::<i8>(0.008);
        round_trip::<i16>(3.1e-5);
        round_trip::<i32>(1e-6);
        round_trip::<i64>(1e-6);
        round_trip::<u8>(0.008);
        round_trip::<u16>(3.1e-5);
        round_trip::<u32>(1e-6);
        round_trip::<u64>(1e-6);
    }

    #[test]
    fn silence_is_the_equilibrium_cpal_calls_a_silent_stream_resting() {
        // `route` clears a block with `SILENCE` before it writes the channels
        // the route names, so that value has to be what cpal itself calls the
        // resting point - including the unsigned formats, whose resting point is
        // the middle of their range and not zero.
        assert_eq!(<f64 as cpal::Sample>::EQUILIBRIUM, f64::SILENCE);
        assert_eq!(<i8 as cpal::Sample>::EQUILIBRIUM, i8::SILENCE);
        assert_eq!(<i16 as cpal::Sample>::EQUILIBRIUM, i16::SILENCE);
        assert_eq!(<i32 as cpal::Sample>::EQUILIBRIUM, i32::SILENCE);
        assert_eq!(<i64 as cpal::Sample>::EQUILIBRIUM, i64::SILENCE);
        assert_eq!(<u8 as cpal::Sample>::EQUILIBRIUM, u8::SILENCE);
        assert_eq!(<u16 as cpal::Sample>::EQUILIBRIUM, u16::SILENCE);
        assert_eq!(<u32 as cpal::Sample>::EQUILIBRIUM, u32::SILENCE);
        assert_eq!(<u64 as cpal::Sample>::EQUILIBRIUM, u64::SILENCE);
        assert_eq!(128u8, u8::SILENCE);
        assert_eq!(32_768u16, u16::SILENCE);
    }
}
