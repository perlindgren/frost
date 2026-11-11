//! The audio thread's contract with the allocator: a block of processing
//! allocates nothing, ever.
//!
//! Isolated from nice-plug's debug allocator: a test binary that linked
//! `nice_plug` (with `assert_process_allocs`) would carry two global allocator
//! declarations — its disabler and `allocation_counter`'s counter — and rustc
//! refuses the pair outright. So this binary includes the DSP source as a module
//! of its own and measures against nothing but the filter.
//!
//! It carries no `cfg` at all, and that is deliberate. `nice-nam` gates its RT
//! test `#![cfg(not(windows))]` because the NAM engine behind that DSP has no
//! Windows build; `nice-wah` copied the line along with the file's shape, and
//! this file came from the wah. Nothing here earns an exclusion — the counter is
//! a plain `#[global_allocator]` over `System` with no platform code in it, and
//! the pass-through measures zero allocations on MSVC like anywhere else. The
//! line cost more than a wrong cast did: it made a file the Linux gate lints
//! read as empty on the machine doing the work. If the real filter ever earns an
//! exclusion, it should arrive as a failing test with a reason, not as this line.
//!
//! It is worth running now, before there is a filter: the pass-through is the
//! floor, and the day the real one lands this file is the thing that says what
//! it cost.
// This binary sees only the slice of the filter it measures; the rest of
// `impl PiFace` — the rate retune, the editor's readouts — has no caller here.
#[allow(dead_code)]
#[path = "../src/dsp.rs"]
mod dsp;

use dsp::*;

#[test]
fn a_block_costs_nothing_to_the_allocator() {
    // Every routing the machine has: the three modes of operation are the three
    // positions of the Phasor B Input switch, so the day the filter branches on
    // them, each branch gets measured.
    for routing in [Routing::InA, Routing::OutA, Routing::InB] {
        let controls = Controls {
            b_input: routing.to_param(),
            gen1_rate: 4.0,
            gen2_rate: 6.5,
            a_depth: 0.8,
            b_depth: 0.6,
            ..Default::default()
        };
        let mut pi_face = PiFace::new(48_000.0, &controls);
        let input = [0.25f32; BLOCK];
        let mut mono = [0.0f32; BLOCK];
        let mut left = [0.0f32; BLOCK];
        let mut right = [0.0f32; BLOCK];
        // The first block through any filter is the one that used to be allowed
        // to prepare coefficients; measure after that, the way a host's real
        // blocks arrive.
        pi_face.process_mono(&controls, &input, &mut mono);
        let counted = allocation_counter::measure(|| {
            for _ in 0..64 {
                pi_face.process_mono(&controls, &input, &mut mono);
                pi_face.process_stereo(
                    &controls,
                    [&input[..], &input[..]],
                    [&mut left[..], &mut right[..]],
                );
            }
        });
        assert_eq!(counted.count_total, 0, "the audio thread allocates");
    }
}

/// Automation is the other thing that happens on the audio thread: the controls
/// change between blocks, and that path must not allocate either.
#[test]
fn moving_the_controls_costs_nothing_either() {
    let mut controls = Controls::default();
    let mut pi_face = PiFace::new(48_000.0, &controls);
    let input = [0.25f32; BLOCK];
    let mut output = [0.0f32; BLOCK];
    let counted = allocation_counter::measure(|| {
        // `i32` on purpose: two of the four controls below are switch positions,
        // and a `0..64` whose element type is only defaulted to `i32` makes the
        // positions read as casts to the type they already have.
        for i in 0i32..64 {
            controls.a_depth = (i % 16) as f32 / 15.0;
            controls.gen1_rate = 0.1 + (i % 12) as f32;
            controls.b_sweep = i % 3;
            controls.b_input = i % 3;
            pi_face.process_mono(&controls, &input, &mut output);
        }
    });
    assert_eq!(counted.count_total, 0, "automation allocates");
}
