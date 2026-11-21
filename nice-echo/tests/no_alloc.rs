//! The audio thread's contract with the allocator: a block of processing
//! allocates nothing, ever.
//!
//! Isolated from nice-plug's debug allocator: a test binary that linked
//! `nice_plug` (with `assert_process_allocs`) would carry two global allocator
//! declarations — its disabler and `allocation_counter`'s counter — and rustc
//! refuses the pair outright. So this binary includes the DSP source as a module
//! of its own and measures against nothing but the tape machine.
//!
//! It carries no `cfg` at all, and that is deliberate. `nice-nam` gated its RT
//! test `#![cfg(not(windows))]` for as long as the engine behind that DSP had no
//! Windows build; `nice-wah` copied the line along with the file's shape, and
//! this file came down through the wah and the Pi-Fase. Nothing here earns an
//! exclusion — the counter is a plain `#[global_allocator]` over `System` with
//! no platform code in it, and the filter measures zero allocations on MSVC
//! like anywhere else. The line cost more than a wrong cast did: it made a file
//! the Linux gate lints read as empty on the machine doing the work. If a future
//! DSP earns an exclusion, it should arrive as a failing test with a reason,
//! not as this line.
//!
//! This is what the filter cost the allocator: nothing. The saturating record
//! loop, three playback heads with their HF-loss poles, the wow and flutter
//! generators steering the read position, the spring tank and the output
//! trimming run a block between them without asking for a byte, and they keep
//! doing it while every control moves — which is the moment a hidden allocation
//! would surface, because automation does not arrive on a schedule the audio
//! thread can plan for.
// This binary sees only the slice of the machine it measures; the rest of
// `impl Echo` — the rate retune, the editor's readouts — has no caller here.
#[allow(dead_code)]
#[path = "../src/dsp.rs"]
mod dsp;

use dsp::*;

#[test]
fn a_block_costs_nothing_to_the_allocator() {
    // Every mode the machine has: the twelve positions pick different head and
    // spring combinations, so if the path ever branches on them, each branch
    // gets measured. The spring-only position rides last in the same sweep.
    for mode in 0..MODES as i32 {
        let controls = Controls {
            mode,
            intensity: 0.6,
            ..Controls::default()
        };
        let mut echo = Echo::new(48_000.0, &controls);
        let input = [0.25f32; BLOCK];
        let mut mono = [0.0f32; BLOCK];
        // The first block through any filter is the one that used to be allowed
        // to prepare coefficients; measure after that, the way a host's real
        // blocks arrive.
        echo.process_mono(&controls, &input, &mut mono);
        let counted = allocation_counter::measure(|| {
            for _ in 0..64 {
                echo.process_mono(&controls, &input, &mut mono);
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
    let mut echo = Echo::new(48_000.0, &controls);
    let input = [0.25f32; BLOCK];
    let mut output = [0.0f32; BLOCK];
    let counted = allocation_counter::measure(|| {
        // `i32` on purpose: three of the nine controls are switch positions,
        // and a `0..64` whose element type is only defaulted to `i32` makes the
        // positions read as casts to the type they already have. The mode walks
        // the whole twelve-position panel; the speed slews hard twice a lap so
        // the transport's smoothing path rides in the measurement too.
        for i in 0i32..64 {
            controls.mode = i % MODES as i32;
            controls.repeat_rate = (i % 16) as f32 / 15.0;
            controls.intensity = (i % 21) as f32 / 20.0;
            controls.echo_volume = (i % 11) as f32 / 10.0;
            controls.reverb_volume = 1.0 - (i % 11) as f32 / 10.0;
            controls.treble = (i % 7) as f32 / 6.0;
            controls.bass = 1.0 - (i % 7) as f32 / 6.0;
            controls.echo_switch = i % 2;
            controls.output_level = i % 3;
            echo.process_mono(&controls, &input, &mut output);
        }
    });
    assert_eq!(counted.count_total, 0, "automation allocates");
}

/// Reset must clear the tape, the spring, the noise/modulation and the
/// transport history even after controls have changed, without touching the
/// allocator.
#[test]
fn reset_is_allocation_free_and_matches_a_fresh_machine() {
    for rate in [8_000.0, 48_000.0, 96_000.0] {
        let mut controls = Controls::default();
        let mut machine = Echo::new(rate, &controls);
        let input = [0.25; BLOCK];
        let mut out = [0.0; BLOCK];
        for pass in 0..2 {
            for _ in 0..128 {
                machine.process_mono(&controls, &input, &mut out);
            }
            if pass == 0 {
                controls.repeat_rate = 0.91;
                controls.mode = 11;
                controls.intensity = 0.8;
            }
            let counted = allocation_counter::measure(|| {
                machine.reset(&controls);
            });
            assert_eq!(counted.count_total, 0, "reset allocates");
            assert_eq!(counted.count_current, 0, "reset frees storage");
            assert_eq!(counted.bytes_current, 0, "reset changes allocated bytes");
            let mut fresh = Echo::new(rate, &controls);
            let mut expected = [0.0; BLOCK];
            // Long enough to expose old tape/spring contents, not just the
            // immediate dry signal: one impulse through the loop, then the
            // tail ringing out against silence.
            for block in 0..(rate as usize * 4 / BLOCK) {
                let mut impulse = [0.0; BLOCK];
                if block == 0 {
                    impulse[0] = 1.0;
                }
                let silence = [0.0; BLOCK];
                let fed = if block % 2 == 0 { &impulse } else { &silence };
                machine.process_mono(&controls, fed, &mut out);
                fresh.process_mono(&controls, fed, &mut expected);
                assert_eq!(
                    out, expected,
                    "output after reset at {rate} Hz, block {block}"
                );
            }
        }
    }
}
