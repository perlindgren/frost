//! The audio thread's contract with the allocator: a block of equalising
//! allocates nothing, ever.
//!
//! Isolated from nice-plug's debug allocator: a test binary that linked
//! `nice_plug` (with `assert_process_allocs`) would carry two global allocator
//! declarations — its disabler and `allocation_counter`'s counter — and rustc
//! refuses the pair outright. So this binary includes the DSP source as a module
//! of its own and measures against nothing but the cascade.
//!
//! It carries no `cfg` at all, and that is deliberate; see the same paragraph in
//! nice-pi-face's copy. The counter is a plain `#[global_allocator]` over
//! `System` with no platform code in it, and seven biquads measure zero
//! allocations on MSVC like anywhere else.
//!
//! What this is the evidence for, in the words the plugin's own promises use:
//! the coefficient blend at the start of a block, the memory the cascade carries
//! from one block to the next, the retune a rate change triggers, and the sweep a
//! host writes between blocks all happen inside this thread's work. A `Vec` that
//! appeared in any of those paths would not show up as a glitch a listener could
//! point at; it would show up as the occasional block that arrives too late,
//! which is why the claim is measured here instead of argued.

// This binary sees only the slice of the filter it measures: the analyser's
// enums are read by the UI and by `process()`, neither of which is here.
#[allow(dead_code)]
#[path = "../src/dsp.rs"]
mod dsp;

use dsp::*;

/// A block of something worth equalising: a tone whose phase advances within
/// the block, so no sample is the same as the one before it.
fn signal() -> [f32; BLOCK] {
    let mut block = [0.0f32; BLOCK];
    for (index, sample) in block.iter_mut().enumerate() {
        *sample = 0.5 * (index as f32 * 0.1).sin();
    }
    block
}

#[test]
fn a_block_costs_nothing_to_the_allocator() {
    let controls = Controls {
        mode: 0,
        bands: [3.0, -2.0, 6.5, 0.0, -7.5, 1.5, -0.5],
        gain_db: -3.0,
    };
    let mut eq = Eq::new(48_000.0, &controls);
    let input = signal();
    let mut output = [0.0f32; BLOCK];
    // The first block through any filter is the one that used to be allowed to
    // prepare coefficients; measure after that, the way a host's real blocks
    // arrive.
    eq.process_mono(&controls, &input, &mut output);
    let counted = allocation_counter::measure(|| {
        for _ in 0..128 {
            eq.process_mono(&controls, &input, &mut output);
        }
    });
    assert_eq!(counted.count_total, 0, "the audio thread allocates");
}

/// Automation is the other thing that happens on the audio thread: a host writes
/// a fader between blocks, the cascade retunes its blend, and that path must not
/// allocate either. A graphic equaliser is the plugin this matters most for —
/// seven faders is seven sources of curves a host can draw.
#[test]
fn moving_the_faders_costs_nothing_either() {
    let mut controls = Controls::default();
    let mut eq = Eq::new(48_000.0, &controls);
    let input = signal();
    let mut output = [0.0f32; BLOCK];
    let counted = allocation_counter::measure(|| {
        // Positions on purpose, not floats: the mode and the clamp a host may
        // hand in are part of the same path, and a mode change is exactly the
        // moment a naive implementation would go and look a band table up in a
        // collection.
        for position in 0i32..64 {
            for (index, band) in controls.bands.iter_mut().enumerate() {
                *band = MAX_DB * (((position + index as i32) % 13) as f32 / 6.0 - 1.0);
            }
            controls.gain_db = MAX_DB - position as f32 * 0.5;
            controls.mode = position;
            eq.process_mono(&controls, &input, &mut output);
        }
    });
    assert_eq!(counted.count_total, 0, "automation allocates");
}

/// A rate change is a lifecycle call made from the host's thread, and it is the
/// one place the coefficients are rebuilt rather than blended.
#[test]
fn a_rate_change_costs_nothing_either() {
    let controls = Controls {
        mode: 0,
        bands: [-4.0, 0.0, 4.0, 8.0, -8.0, 0.5, -0.5],
        gain_db: 2.0,
    };
    let mut eq = Eq::new(48_000.0, &controls);
    let input = signal();
    let mut output = [0.0f32; BLOCK];
    let counted = allocation_counter::measure(|| {
        for rate in [44_100.0f32, 48_000.0, 88_200.0, 96_000.0, 192_000.0] {
            eq.set_rate(rate);
            eq.process_mono(&controls, &input, &mut output);
            eq.reset();
            eq.process_mono(&controls, &input, &mut output);
        }
    });
    assert_eq!(counted.count_total, 0, "a rate change allocates");
}
