//! The audio thread's contract with the allocator: a block of processing
//! allocates nothing, ever.
//!
//! Isolated from nice-plug's debug allocator, as in nice-wah's and
//! nice-nam's RT tests: a test binary that linked `nice_plug` (with
//! `assert_process_allocs`) would carry two global allocator declarations
//! — its disabler and `allocation_counter`'s counter — and rustc refuses
//! the pair outright. So this binary includes the DSP source as a module
//! of its own and measures against nothing but the filter.
#![cfg(not(windows))]
// This binary sees only the slice of the filter it measures; the rest of
// `impl Phase` — the reset, the editor's readout — has no caller here.
#[allow(dead_code)]
#[path = "../src/dsp.rs"]
mod dsp;

use dsp::*;

#[test]
fn a_block_costs_nothing_to_the_allocator() {
    for mode in MODES {
        let controls = Controls {
            mode: MODES.iter().position(|m| *m == mode).expect("in the table") as f32,
            ..Controls::default()
        };
        let mut phaser = Phase::new(48_000.0, &controls);
        let input = [0.25f32; BLOCK];
        let mut mono = [0.0f32; BLOCK];
        let mut left = [0.0f32; BLOCK];
        let mut right = [0.0f32; BLOCK];
        // The first block through any filter is the one that used to be
        // allowed to prepare coefficients; measure after that, the way a
        // host's real blocks arrive.
        phaser.process_mono(&controls, &input, &mut mono);
        let counted = allocation_counter::measure(|| {
            for _ in 0..64 {
                phaser.process_mono(&controls, &input, &mut mono);
                phaser.process_stereo(
                    &controls,
                    [&input[..], &input[..]],
                    [&mut left[..], &mut right[..]],
                );
            }
        });
        assert_eq!(counted.count_total, 0, "{} allocates", mode.name);
    }
}

/// Automation is the other thing that happens on the audio thread: the
/// controls change between blocks, and that path must not allocate
/// either. The mode parameter walks its whole table mid-block — the
/// stage crossfade's `Vec`-shaped temptation — and every knob sweeps
/// while it is at it.
#[test]
fn moving_the_controls_costs_nothing_either() {
    let mut controls = Controls::default();
    let mut phaser = Phase::new(48_000.0, &controls);
    let input = [0.25f32; BLOCK];
    let mut output = [0.0f32; BLOCK];
    let counted = allocation_counter::measure(|| {
        for i in 0..256 {
            // Walk the fractional part of the mode continuously, flipping
            // whole settings every sixteen blocks: the slewing path sees
            // both a mid-crossfade block and a settings jump.
            controls.mode = (i / 16) as f32 % 5.0 + (i % 16) as f32 / 32.0;
            controls.speed = 0.1 + (i % 40) as f32 * 0.5;
            controls.depth = (i % 16) as f32 / 15.0;
            controls.mix = 1.0 - (i % 16) as f32 / 15.0;
            controls.gain_db = (i % 9) as f32 - 4.0;
            phaser.process_mono(&controls, &input, &mut output);
        }
    });
    assert_eq!(counted.count_total, 0, "automation allocates");
}
