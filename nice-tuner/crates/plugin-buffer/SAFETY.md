# Scoped plugin buffer boundary

Review date: 2026-11-04. This project-owned crate adapts owned audio storage to
nice-plug 0.4.2's public unsafe `Buffer::set_slices` API. The `Plugin::process`
call is safe and remains in nice-tuner. No third-party source is modified.

`PlanarBuffer<SAMPLES>` owns a `Box<heapless::Vec<f32, SAMPLES>>` and preallocates
the Buffer's channel-slice metadata. The box is allocated once during setup so
the inline sample array has a stable address when its owner moves. The heapless
capacity is a compile-time bound; nice-tuner reserves 2 × 1024 samples for its
mono/stereo layouts. Dimension products are checked for overflow and against
that bound, and dimensions must be nonzero. Before each call it checks the frame
count against capacity, clears old slices, and constructs equal-length, disjoint mutable
channel slices directly from the backing allocation. It never grows a slice
from a pointer supplied or shortened by a previous callback.

The internally cached Buffer uses a private apparent static lifetime. This is
not exposed as `Buffer<'static>` to callers: access requires a callback valid
for **every** sample lifetime, with a return type independent of that lifetime.
That prevents safe callers from extracting the Buffer or its sample references
beyond the call. Two compile-fail documentation tests check those cases.

A drop guard clears all slice references on normal return and on callback panic.
Backing storage is neither resized nor directly exposed. The Buffer field drops
before the storage, and moving the owner leaves the boxed allocation in place.
All unsafe blocks have checked invariants documented at their call sites;
undocumented blocks and implicit unsafe operations inside unsafe functions are
denied by lint.

Tests cover disjoint channels, changing frame sizes, a caller replacing or
shortening slices/the Buffer, zero-frame calls, rejection of oversized blocks,
unwinding/reuse, and allocation-free normal processing after initialization.
Replacing the Buffer can discard its preallocated metadata capacity; nice-tuner
does not do that. Its activation and processing still use the same plugin
implementation as the DAW wrappers.

The Buffer type comes from `nice-plug-core`'s public API; it is the same type
re-exported by nice-plug. This avoids importing nice-plug's host wrappers and
their global allocation guard into this crate's allocation-measurement tests.

All five unit tests and both compile-fail examples pass in debug and release.
The allocation measurement includes a positive control and stays active in
release. The tuner's 51 standalone tests also pass using this adapter, and
strict Clippy checks pass for the wrapper and app. The full app checks pass for
macOS and Windows/MSVC with the unsafe-code ban enabled unconditionally.

These checks are evidence from source review and tests, not a formal proof or an
independent safety audit. The wrapper relies on nice-plug's safe Buffer methods
honoring their Rust lifetime contracts.
