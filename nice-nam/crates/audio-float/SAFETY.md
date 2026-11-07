# Floating-point boundary

Review: 2026-11-07. No dependency source is modified.

`with_denormals_flushed` temporarily sets FPCR.FZ on aarch64, or MXCSR.DAZ/FTZ
on NAM's x86-64-v3 baseline. Instructions use live stack storage of the exact
register width. Values come from the calling thread's hardware register; only
specified mode bits are added. Reserved bits, rounding modes, exception masks
and other fields are preserved. The private restoration guard never leaves the
function; Rust unwinding restores the original register on the same thread.
No raw pointers or guards reach callers. Aborting a process needs no restoration.
The assembly intentionally retains the compiler's default memory side effects
and is not marked pure, so arithmetic cannot be freely moved across the scope.

This addresses NAM's documented denormal-mode precondition for both direct
standalone processing and DAW processing without changing the host's lasting FP
policy. It is source-reviewed; this is not an independent safety audit.
