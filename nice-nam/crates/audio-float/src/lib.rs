//! Scoped flush-to-zero mode for NAM's native floating-point kernels.
#![deny(unsafe_op_in_unsafe_fn, clippy::undocumented_unsafe_blocks)]

/// Execute on the calling thread with denormals flushed. Restores the original
/// FP control register on return or panic. No state or guard escapes the call.
/// On x86_64, DAZ is enabled when AVX2 confirms a supporting CPU.
pub fn with_denormals_flushed<T>(f: impl FnOnce() -> T) -> T {
    #[cfg(target_arch = "aarch64")]
    {
        struct Restore(u64);
        impl Drop for Restore {
            fn drop(&mut self) {
                // SAFETY: Restore the same thread's unchanged, previously read
                // FPCR. The guard cannot escape this function or change threads.
                unsafe {
                    std::arch::asm!("msr fpcr, {}", in(reg) self.0, options(nostack, preserves_flags));
                }
            }
        }
        let original: u64;
        // SAFETY: FPCR is accessible in user mode. Only FZ (bit 24) is set;
        // all reserved and unrelated fields retain their original values.
        unsafe {
            std::arch::asm!("mrs {}, fpcr", out(reg) original, options(nostack, preserves_flags));
            std::arch::asm!("msr fpcr, {}", in(reg) original | (1u64 << 24), options(nostack, preserves_flags));
        }
        let _restore = Restore(original);
        f()
    }
    #[cfg(target_arch = "x86_64")]
    {
        struct Restore(u32);
        impl Drop for Restore {
            fn drop(&mut self) {
                // SAFETY: The pointer names a valid u32 containing MXCSR read
                // on this thread. Restoring it preserves all reserved bits.
                unsafe {
                    std::arch::asm!("ldmxcsr [{}]", in(reg) &self.0, options(nostack, preserves_flags));
                }
            }
        }
        let mut original = 0u32;
        // SAFETY: Both pointers name live u32 stack values. SSE2 is guaranteed
        // by x86_64; NAM additionally requires x86-64-v3, which supports DAZ.
        // Only DAZ (bit 6) and FTZ (bit 15) change, leaving reserved bits intact.
        unsafe {
            std::arch::asm!("stmxcsr [{}]", in(reg) &mut original, options(nostack, preserves_flags));
            let daz = if std::is_x86_feature_detected!("avx2") {
                1 << 6
            } else {
                0
            };
            std::arch::asm!("ldmxcsr [{}]", in(reg) &(original | daz | (1 << 15)), options(nostack, preserves_flags));
        }
        let _restore = Restore(original);
        f()
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        f()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flushes_subnormals_and_restores_after_return_and_unwind() {
        let half_minimum = || std::hint::black_box(f32::MIN_POSITIVE) * std::hint::black_box(0.5);
        let original = half_minimum();
        assert_eq!(with_denormals_flushed(half_minimum), 0.0);
        assert_eq!(half_minimum(), original);
        let _ = std::panic::catch_unwind(|| with_denormals_flushed(|| panic!("test unwind")));
        assert_eq!(half_minimum(), original);
    }
}
