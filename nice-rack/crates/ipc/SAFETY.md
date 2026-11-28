# Shared mapping boundary

Only `AudioMemory` forms references into the mmap. The checked immutable layout
bounds every address, mmap supplies page alignment, and all cells are initialized
as AtomicU32 before publication. Both processes use atomic accesses, including
f32 sample bit patterns; no plugin receives pointers into this mapping. Sample
writes precede release publication; the reader acquires the sequence before
consuming them. Request sequences are nonzero and cannot be reused while a block
is outstanding. Failed endpoints are poisoned and never recycled for later jobs.

The mapping file is private to the supervised helper pair, created exclusively
with owner-only Unix permissions. It must not be truncated until both mappings
are gone. Native helpers are trusted to follow this protocol; isolation is not a
security sandbox against malicious native code or same-user file tampering.
OS process exit and unmapping are supervised outside the audio path.
