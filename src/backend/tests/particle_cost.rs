//! What a particle batch costs the **CPU**, measured rather than assumed.
//!
//! Every other instrument in this directory is fragment-bound: `light_cost`
//! and the benchsuite ask how many pixels a frame shades, and their answers
//! are milliseconds of GPU. A particle batch spends most of its life before
//! the GPU ever sees it. The simulation is a loop over every live particle
//! ([`ParticleSystem::update`]), and the frame rebuilds the batch's instance
//! bytes from scratch every frame — a fresh `Vec`, a user-to-pixel transform
//! per particle, 32 bytes appended per particle — inside
//! [`Canvas::draw_scene`], which is the call a game makes and the door this
//! bench uses. None of that shows up in a pixels-per-second number, and none
//! of it needs a display, an adapter, or a quiet machine: it is arithmetic on
//! one thread, so it can be measured where a lighting number cannot be.
//!
//! Three numbers, for three different costs:
//!
//! * `sim` — `ParticleSystem::update`, all particles surviving. The integrate
//!   step plus the `retain` pass with nothing to remove.
//! * `pack` — `Canvas::draw_scene` on a scene whose only node is one
//!   [`Shape::Particles`] batch, minus the same call on an empty batch. That
//!   control is measured too, as `walk`, because the scene walk and the fresh
//!   `Canvas` belong to the frame whether or not any particle is alive; the
//!   subtraction leaves the pack, and the two rows are printed side by side so
//!   the subtraction is visible instead of buried.
//! * `sim-dying` — a step where half the particles expire, which is when
//!   `retain` does its real work of shifting the survivors down. It cannot be
//!   timed directly: the particles do not come back, so the timed window
//!   includes re-seeding the system. `reseed` is therefore measured on its
//!   own and the row reports the difference of the two medians — which is
//!   *not* the median of the differences, and is read as an estimate of the
//!   compaction rather than as a measurement of it.
//!
//! `alloc` prices one 320 KB allocation and free on its own, which is what a
//! 100 000-particle batch asks the allocator for every frame. It is not the
//! allocation's share of the pack — the pack's copies and transforms are
//! interleaved with it — it is the standalone price, an upper bound to be read
//! against `pack`.
//!
//! Not a gate. A bench has no baseline to fail, and `--ignored` keeps it out of
//! the suite's way. Read the spread printed beside each median before believing
//! any of it: these are one thread on a machine that may be doing something
//! else, and unlike the frame benches there is no canary here to say so —
//! the closest thing is `walk`, which should not move.
//!
//! ```text
//! cargo test --release --config 'profile.release.debug-assertions=true' \
//!     --lib particle_cost -- --ignored --nocapture
//! ```
//!
//! `--release`, unconditionally. These are per-particle costs of tens of
//! nanoseconds, and an unoptimised build turns eight slice copies into eight
//! function calls: the first version of this bench was run without it and
//! reported ~90 ns for work that takes a tenth of that, a number that would
//! have sent someone off to rewrite a packing function that is fine.

use crate::backend::frame::particle_instance;
use crate::canvas::Canvas;
use crate::objects::{Color, ParticleShape, Scene, SceneNode, Shape};
use crate::particles::{Particle, ParticleSystem};
use std::hint::black_box;
use std::time::Instant;

/// The surface the scene is walked for. Only its scale matters here — no
/// drawing happens, so the size costs nothing beyond one matrix.
const SURFACE: (u32, u32) = (1920, 1080);

fn white() -> Color {
    Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    }
}

/// A deterministic system of `n` particles scattered across a 900×500 box,
/// with sizes and angles so the pack has real work to do rather than a
/// constant it could in principle hoist. `life` decides whether they survive
/// a step: anything above 0.1 s survives the bench's single step, a small
/// value dies on it.
fn seeded(n: usize, life: f32) -> ParticleSystem {
    let mut sys = ParticleSystem::new();
    // Park–Miller-ish on u32: cheap, reproducible, no dependency, and the
    // same sequence every run so two machines' numbers are comparable.
    let mut bits = 2463534242u32;
    let mut next = move || {
        bits ^= bits << 13;
        bits ^= bits >> 17;
        bits ^= bits << 5;
        bits
    };
    for _ in 0..n {
        let a = next() as f32 / 4294967296.0;
        let b = next() as f32 / 4294967296.0;
        sys.spawn(Particle {
            pos: [a * 900.0 - 450.0, b * 500.0 - 250.0],
            vel: [(a - 0.5) * 240.0, (b - 0.5) * 240.0],
            life,
            max_life: 2.0,
            size: 3.0 + a * 9.0,
            angle: b * std::f32::consts::TAU,
            color: white(),
        });
    }
    black_box(&sys);
    sys
}

/// A scene whose only content is one particle batch, the way a game holds it:
/// a node with the system attached, lit so the batch is a real receiver.
fn scene_with(system: ParticleSystem) -> Scene {
    scene_lit(system, true)
}

/// The same scene with the batch's `lit` flag chosen, so a row can ask whether
/// the receiver's lit state costs anything on the CPU side of the frame.
fn scene_lit(system: ParticleSystem, lit: bool) -> Scene {
    Scene {
        root: SceneNode {
            children: vec![Box::new(SceneNode {
                shape: Some(Shape::Particles {
                    system,
                    color: white(),
                    shape: ParticleShape::Circle,
                }),
                lit,
                ..Default::default()
            })],
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Warm, then time `reps` runs; returns (median, min, max) in milliseconds.
fn timed(reps: usize, warm: usize, mut run: impl FnMut()) -> (f64, f64, f64) {
    for _ in 0..warm {
        run();
    }
    let mut samples = Vec::with_capacity(reps);
    for _ in 0..reps {
        let t = Instant::now();
        run();
        samples.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (
        samples[samples.len() / 2],
        samples[0],
        samples[samples.len() - 1],
    )
}

/// One printed row: the raw median in ms, and what that is per particle.
fn row(what: &str, n: usize, ms: f64, lo: f64, hi: f64) {
    if n == 0 {
        println!(
            "particle cost: {what:<14} {ms:>9.4} ms/frame   (control)   reps {lo:.4}-{}",
            hi
        );
    } else {
        println!(
            "particle cost: {what:<14} {ms:>9.4} ms/frame   {:>8.2} ns/particle   reps {lo:.4}-{}",
            ms * 1e6 / n as f64,
            hi
        );
    }
}

#[test]
#[ignore = "a timing bench: run by hand with --ignored --nocapture"]
fn a_particle_batch_costs_the_cpu_this_much() {
    let cores = std::thread::available_parallelism()
        .map(|c| c.get())
        .unwrap_or(0);
    println!(
        "particle cost: cpu bench, {} cores, {}",
        cores,
        std::env::consts::ARCH
    );

    // The timer's own cost at this granularity, so a reader can see the floor
    // every row sits on.
    let (floor, flo, fhi) = timed(9, 100, || {
        black_box(0u64);
    });
    println!("particle cost: timer floor      {floor:.6} ms   reps {flo:.6}-{fhi:.6}");

    // --- the walk, and the pack riding on it -------------------------------
    let empty = scene_with(ParticleSystem::new());
    let (walk, wlo, whi) = timed(9, 3, || {
        let mut canvas = Canvas::new(SURFACE, 1.0);
        canvas.draw_scene(&empty);
        black_box(&canvas);
    });
    row("walk", 0, walk, wlo, whi);

    for n in [1_000usize, 10_000, 100_000] {
        let scene = scene_with(seeded(n, 9.0));
        let (ms, lo, hi) = timed(9, 3, || {
            let mut canvas = Canvas::new(SURFACE, 1.0);
            canvas.draw_scene(&scene);
            black_box(&canvas);
        });
        row(&format!("pack-{n}"), n, ms, lo, hi);
    }
    println!("particle cost: pack rows are net of `walk` only in the reader's head — subtract it");

    // The unlit control: the same batch with `lit: false` on its node. If the
    // batch's receiver state costs anything per particle on the way out, this
    // row says so; if it does not, the pack is the pack either way.
    let unlit = scene_lit(seeded(10_000, 9.0), false);
    let (ms, lo, hi) = timed(9, 3, || {
        let mut canvas = Canvas::new(SURFACE, 1.0);
        canvas.draw_scene(&unlit);
        black_box(&canvas);
    });
    row("pack-10000-off", 10_000, ms, lo, hi);

    // --- bisecting the pack -------------------------------------------------
    // The instance bytes alone, into a buffer that is allocated once and never
    // grows: what one particle costs before any `Vec` growth or scene walk.
    let n = 10_000;
    let sys = seeded(n, 9.0);
    let (ms, lo, hi) = timed(9, 3, || {
        let mut buf: Vec<u8> = Vec::with_capacity(n * 32);
        for p in &sys.particles {
            buf.extend_from_slice(&black_box(particle_instance(p, 1.0, 2.0, 3.0, 0.5)));
        }
        black_box(&buf);
    });
    row(&format!("instance-{n}"), n, ms, lo, hi);
    // The same buffer filled with a constant: the append and the memory, with
    // the instance math taken out.
    let (ms, lo, hi) = timed(9, 3, || {
        let mut buf: Vec<u8> = Vec::with_capacity(n * 32);
        for _ in 0..n {
            buf.extend_from_slice(&[7u8; 32]);
        }
        black_box(&buf);
    });
    row(&format!("append-{n}"), n, ms, lo, hi);

    // --- the simulation -----------------------------------------------------
    for n in [1_000usize, 10_000, 100_000] {
        let mut sys = seeded(n, 9.0);
        let (ms, lo, hi) = timed(9, 3, || sys.update(1.0 / 60.0, [0.0, 400.0]));
        row(&format!("sim-{n}"), n, ms, lo, hi);
        assert_eq!(sys.len(), n, "the surviving system must stay full");
    }

    // --- compaction, as the difference of two medians -----------------------
    // One step, then the system is rebuilt inside the timed window because it
    // has to come back for the next rep; `reseed` measures that rebuild alone.
    let n = 10_000;
    let (together, tlo, thi) = timed(9, 3, || {
        let mut sys = seeded(n, 0.01);
        sys.update(1.0 / 60.0, [0.0, 400.0]);
        black_box(sys.len());
    });
    row("seed+die", n, together, tlo, thi);
    let (alone, alo, ahi) = timed(9, 3, || {
        let len = seeded(n, 0.01).len();
        black_box(len);
    });
    row("reseed", n, alone, alo, ahi);
    println!(
        "particle cost: {n} dying particles, est. {:-.4} ms (difference of medians, not a median \
         of differences — an estimate of the compaction, not a measurement of it)",
        together - alone
    );

    // --- the batch's per-frame allocation, standing alone -------------------
    let bytes = 100_000usize * 32;
    let (ms, lo, hi) = timed(9, 3, || {
        let mut buf: Vec<u8> = Vec::with_capacity(bytes);
        buf.resize(1, 7);
        black_box(&buf);
    });
    println!(
        "particle cost: alloc-{bytes} bytes   {ms:.4} ms   reps {lo:.4}-{hi:.4}   \
         (the standalone price of what a 100k batch allocates each frame)"
    );
}
