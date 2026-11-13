//! The audio thread's contract with the allocator: a block of processing
//! allocates nothing, ever.
//!
//! Isolated from nice-plug's debug allocator: a test binary that linked
//! `nice_plug` (with `assert_process_allocs`) would carry two global allocator
//! declarations — its disabler and `allocation_counter`'s counter — and rustc
//! refuses the pair outright. So this binary includes the DSP source as a module
//! of its own and measures against nothing but the tape machine.
//!
//! It carries no `cfg` at all, and that is deliberate. `nice-nam` gates its RT
//! test `#![cfg(not(windows))]` because the NAM engine behind that DSP has no
//! Windows build; `nice-wah` copied the line along with the file's shape, and
//! this file came down through the wah, the Pi-Fase and the echo. Nothing here
//! earns an exclusion — the counter is a plain `#[global_allocator]` over
//! `System` with no platform code in it, and the filter measures zero
//! allocations on MSVC like anywhere else. The line cost more than a wrong cast
//! did: it made a file the Linux gate lints read as empty on the machine doing
//! the work. If a future DSP earns an exclusion, it should arrive as a failing
//! test with a reason, not as this line.
//!
//! This is what the filter cost the allocator: nothing. The tube-style
//! pre-emphasis, the guarded feedback loop with its one playback head's
//! HF-loss poles, the wow generator steering the read position and the output
//! trimming run a block between them without asking for a byte, and they keep
//! doing it while every control moves — which is the moment a hidden allocation
//! would surface, because automation does not arrive on a schedule the audio
//! thread can plan for.
// This binary sees only the slice of the machine it measures; the rest of
// `impl Ecomplex` — the rate retune, the editor's readouts — has no caller here.
#[allow(dead_code)]
#[path = "../src/dsp.rs"]
mod dsp;

use dsp::*;

#[test]
fn a_block_costs_nothing_to_the_allocator() {
    // Two points of the loop, because they are different machines: a short,
    // tight delay rides the last fractions of the tape loop and its wow at full
    // depth, while a long one stretches the delay line and shallows the flutter.
    // Both paths get measured.
    for delay in [0.15f32, 0.85f32] {
        let controls = Controls {
            delay_time: delay,
            recount: 0.6,
            ..Controls::default()
        };
        let mut ecomplex = Ecomplex::new(48_000.0, &controls);
        let input = [0.25f32; BLOCK];
        let mut mono = [0.0f32; BLOCK];
        let mut left = [0.0f32; BLOCK];
        let mut right = [0.0f32; BLOCK];
        // The first block through any filter is the one that used to be allowed
        // to prepare coefficients; measure after that, the way a host's real
        // blocks arrive.
        ecomplex.process_mono(&controls, &input, &mut mono);
        let counted = allocation_counter::measure(|| {
            for _ in 0..64 {
                ecomplex.process_mono(&controls, &input, &mut mono);
                ecomplex.process_stereo(
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
    let mut ecomplex = Ecomplex::new(48_000.0, &controls);
    let input = [0.25f32; BLOCK];
    let mut output = [0.0f32; BLOCK];
    let counted = allocation_counter::measure(|| {
        // Every control moves, on walks with different periods so no pair of
        // them repeats its combination twice while the sweep lasts: the front
        // end's two trims and its drive slew against the loop's time, feed and
        // tone, with the output volume crossing them all. A hidden allocation
        // hiding behind one specific combination is what this sweep is for.
        for i in 0u32..64 {
            controls.mic_level = (i % 9) as f32 / 8.0;
            controls.gtr_level = 1.0 - (i % 7) as f32 / 6.0;
            controls.color = (i % 5) as f32 / 4.0;
            controls.delay_time = (i % 13) as f32 / 12.0;
            controls.recount = (i % 11) as f32 / 10.0;
            controls.timbre = 1.0 - (i % 17) as f32 / 16.0;
            controls.echo_volume = (i % 21) as f32 / 20.0;
            ecomplex.process_mono(&controls, &input, &mut output);
        }
    });
    assert_eq!(counted.count_total, 0, "automation allocates");
}
