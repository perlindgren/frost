//! The audio thread's contract with the allocator: a block of processing
//! allocates nothing, ever.
//!
//! Isolated from nice-plug's debug allocator, as in nice-nam's RT tests:
//! a test binary that linked `nice_plug` (with `assert_process_allocs`)
//! would carry two global allocator declarations — its disabler and
//! `allocation_counter`'s counter — and rustc refuses the pair outright.
//! So this binary includes the DSP source as a module of its own and
//! measures against nothing but the filter.
//!
//! No platform `cfg`, whatever the file's shape suggests. nice-nam gates its
//! RT tests off Windows because the NAM engine behind *that* DSP has no
//! Windows build, and the line travelled into this file along with everything
//! else about it. Nothing here earns it: the counter is a plain
//! `#[global_allocator]` over `System`, and a one-pole envelope is the same
//! arithmetic on every target. With the line in place this binary reported
//! `running 0 tests` on the platform the plugin actually gets built on; the
//! witnesses now measure zero on MSVC too.
// This binary sees only the slice of the filter it measures; the rest of
// `impl Wah` — the reset, the editor's readout — has no caller here.
#[allow(dead_code)]
#[path = "../src/dsp.rs"]
mod dsp;

use dsp::*;

#[test]
fn a_block_costs_nothing_to_the_allocator() {
    for voice in VOICES {
        let controls = Controls::from(voice);
        let mut wah = Wah::new(48_000.0, &controls);
        let input = [0.25f32; BLOCK];
        let mut mono = [0.0f32; BLOCK];
        let mut left = [0.0f32; BLOCK];
        let mut right = [0.0f32; BLOCK];
        // The first block through any filter is the one that used to be
        // allowed to prepare coefficients; measure after that, the way a
        // host's real blocks arrive.
        wah.process_mono(&controls, &input, &mut mono);
        let counted = allocation_counter::measure(|| {
            for _ in 0..64 {
                wah.process_mono(&controls, &input, &mut mono);
                wah.process_stereo(
                    &controls,
                    [&input[..], &input[..]],
                    [&mut left[..], &mut right[..]],
                );
            }
        });
        assert_eq!(counted.count_total, 0, "the audio thread allocates");
    }
}

/// Automation is the other thing that happens on the audio thread: the
/// controls change between blocks, and that path must not allocate either.
#[test]
fn moving_the_controls_costs_nothing_either() {
    let mut controls = Controls::default();
    let mut wah = Wah::new(48_000.0, &controls);
    let input = [0.25f32; BLOCK];
    let mut output = [0.0f32; BLOCK];
    let counted = allocation_counter::measure(|| {
        for i in 0..64 {
            controls.position = (i % 16) as f32 / 15.0;
            controls.q = 2.0 + (i % 8) as f32;
            wah.process_mono(&controls, &input, &mut output);
        }
    });
    assert_eq!(counted.count_total, 0, "automation allocates");
}
