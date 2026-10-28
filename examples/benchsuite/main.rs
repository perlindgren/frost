//! The lighting bench suite: synthetic GPU workloads with named axes,
//! so a number says *what* it weighs.
//!
//! This is not a feature showcase — that is what every other example
//! here is for. The suite exists to referee lighting optimisation:
//! each scenario isolates one term of the light field's cost, a run
//! prints one median frame rate per scenario, and a committed
//! baseline turns every change into a diff table with a verdict.
//!
//! The cost model the axes walk:
//!
//! ```text
//! per lit pixel  ×  lights  ×  (rays  ×  occluders-in-block)
//! ```
//!
//! * `fill-1080p`        — the floor: a lit screen, an empty field. The
//!   cheapest frame the suite can paint, so every other row is this plus
//!   work; `docs/benchmarking.md` calls these two ends the floor and the
//!   cliff, and divides one by the other to ask which of the machine's
//!   parts answered a run.
//! * `lamps-4/10/40`     — the light loop and its falloff, no shadows.
//! * `occluders-10/100`  — the shadow-ray walk, ten lamps over it.
//! * `taps-1/4/9`        — the penumbra ladder and its divergence; the
//!   ninth rung is the cliff, heaviest by a mile and the row where a GPU's
//!   real throughput has to show.
//! * `polys-60`, `taps-9-polys`, `taps-9-discs`, `taps-9-poly16` — the
//!   casters' *shape*: hexagons at six sides, a traced ring at `MAX_PLANES`,
//!   and a declared circle, all at the boxes' own area, so the rows against
//!   `taps-1` and `taps-9` say what a shape costs on top of the box's bounds.
//!   Without them the polygon path is unmeasured, and without the last two
//!   there is nothing to say what *not* tracing a circle is worth.
//! * `taps-9-clear`      — a control, not a rung: nine rays a pixel with
//!   an empty block to walk, so `taps-9` minus it is the walk and it minus
//!   `lamps-10` is the rays. It says whether the penumbra ladder is spent
//!   issuing rays or walking records, which is the difference between two
//!   very different optimizations.
//! * `lamps-40-static`   — the caching prize: equal to `lamps-40`
//!   today *by definition* (nothing is cached); any future gap is a
//!   win already paid for.
//!
//! The conditioned baseline, this iGPU at 1080p, uncapped, three
//! reps — the table every later number is read against. The reps
//! agreed to within half a percent, so this machine's steady state
//! is a real thing, and the same run *unconditioned* (the chip's
//! own weather included) reads a uniform 4–6 % kinder: warm-up is
//! not decoration, it is the difference between a number and a
//! reading.
//!
//! ```text
//! fill-1080p       2136.9   (0.47 ms — the empty-field floor)
//! lamps-4           385.6   (0.52 ms per lamp, arithmetic alone)
//! lamps-10          173.8   (0.53 ms per lamp — linear to the noise)
//! lamps-40           46.9   (0.52 ms per lamp — still linear)
//! lamps-40-static    46.9   (equal BY DEFINITION: nothing is cached)
//! occluders-10       29.4   (ten boxes multiply ten lamps ~6x)
//! occluders-100      15.4   (blocks grow; early-outs soften it)
//! taps-1             16.1   (one hard ray, sixty boxes)
//! taps-4              4.2
//! taps-9              1.8   (556 ms a frame; the cliff, isolated)
//! ```
//!
//! Discipline the machine taught: two identical builds once measured
//! 82 and 36 fps minutes apart, so one number is never the answer —
//! `--reps N` interleaves full passes over the scenarios and the
//! suite reports the **median**, and comparisons only ever run
//! against a baseline captured on a day the whole suite agrees with.
//!
//! Run: `cargo run --release --example benchsuite -- [--seconds S]
//! [--reps N] [--only name,name] [--baseline FILE] [--save-baseline
//! FILE]`. With `--save-baseline` the run writes the file instead of
//! judging itself; with a baseline present (file or `--baseline`)
//! each line becomes `name base -> now (+x%)` and the exit code is 1
//! if any scenario fell more than 5 % below its baseline.
//!
//! Two more flags exist for displays that will not let the fps column
//! mean what it says: `--fullscreen` and `--render-size WxH`. Both are
//! off by default, and what they are for — and what a Retina Mac
//! produced with and without them — is `docs/benchmarking.md` §17.
//!
//! Every run also prints a **noise floor** line — how late a wake landed
//! and how long a fixed quantum of arithmetic took, sampled throughout
//! (`tools/noisefloor`) — and a run whose floor is too loud prints its
//! numbers and exits **3** without judging or saving a baseline. A machine
//! that is busy being something else is allowed to report, not to conclude.
//!
//! Quiet is not the same condition as composited, and the floor cannot tell the
//! two apart, so every run also prints the window's `minimized` and `focused`
//! state, and a minimized or unfocused window prints its numbers and exits **3**
//! the same way. A window nothing composites goes down a different present path,
//! worth 8 to 18 % on the rows short enough to notice it — and it is the state a
//! background shell produces without anyone choosing it
//! (`docs/benchmarking.md` §38).

use std::path::{Path, PathBuf};

use frost::{
    Color, Config, Context, Convex, Light, Particle, ParticleShape, ParticleSystem, Process, Scene,
    SceneNode, Shape, Transform, run_configured,
};

/// The window every scenario is measured in: 1080p, the resolution
/// this suite's history is written against.
const W: f32 = 1920.0;
const H: f32 = 1080.0;

/// One workload: a count of lamps and occluders, the rung of the
/// shadow ladder, and whether the lamps drift (they always drift a
/// little today — nothing caches a shadow).
#[derive(Clone, Copy, Debug)]
struct Scenario {
    name: &'static str,
    lamps: usize,
    occluders: usize,
    /// Penumbra disk radius in pixels; 0 is the single hard ray.
    penumbra: f32,
    /// Taps sweeping the disk when the penumbra is open (1..=9).
    taps: u32,
    /// Lamp radius: how much of the screen each light's loop touches.
    reach: f32,
    moving: bool,
    /// Live particles in a batch on the node, and the area they cover: as a
    /// fraction of the window when `p_radius` is 0, so a row's particle count
    /// can move without its coverage moving, or in user-space pixels when it
    /// is not, so coverage can move without the count moving. `p_lit` decides
    /// whether the batch is a lighting receiver — the one place a particle
    /// frame and a lighting frame are the same question.
    particles: usize,
    p_cover: f32,
    p_radius: f32,
    p_lit: bool,
    /// The casters' shape: 0 is the suite's box, otherwise the side count of a
    /// traced regular polygon of the same area — 16 being `MAX_PLANES`, the plane
    /// cap, and therefore the polygon walk's worst case. Traced matters: a shape
    /// an author rounds off by hand really does pay per edge, which is the thing
    /// this row exists to price now that a circle no longer arrives that way.
    poly_sides: u8,
    /// Whether the casters are declared circles instead, at the same covered area.
    /// A circle is the field's third shape and its cheapest: one quadratic per
    /// ray against `poly_sides` edge clips, which is the comparison §34 measured
    /// and §35 kept as a row so neither number can go stale quietly.
    circle: bool,
}

const LAMP_REACH: f32 = 380.0;
const LAMP_PENUMBRA: f32 = 6.0;

/// The conditioning burn, run once before anything is measured: an
/// iGPU's clocks sag when a run starts cold and drift when it starts
/// hot, and "what ran before the suite" must not be part of a
/// number's meaning. One full measurement of the suite's own heaviest
/// loop brings every run to the same warm state, from any starting point
/// — its result is named `warm`, excluded from every report, and
/// lives nowhere the baseline can see.
const WARM: Scenario = Scenario {
    name: "warm",
    lamps: 40,
    occluders: 60,
    penumbra: 0.0,
    taps: 1,
    reach: LAMP_REACH,
    moving: true,
    poly_sides: 0,
    circle: false,
    particles: 0,
    p_cover: 0.0,
    p_radius: 0.0,
    p_lit: false,
};

/// The suite. Ordered: the cheap floors first, the cliffs last, so a
/// ctrl-C at any point leaves a meaningful prefix.
const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "fill-1080p",
        lamps: 0,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "lamps-4",
        lamps: 4,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "lamps-10",
        lamps: 10,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "lamps-40",
        lamps: 40,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: true,
        poly_sides: 0,
        circle: false,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "lamps-40-static",
        lamps: 40,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "occluders-10",
        lamps: 10,
        occluders: 10,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "occluders-100",
        lamps: 10,
        occluders: 100,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "taps-1",
        lamps: 10,
        occluders: 60,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "taps-4",
        lamps: 10,
        occluders: 60,
        penumbra: LAMP_PENUMBRA,
        taps: 4,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "taps-9",
        lamps: 10,
        occluders: 60,
        penumbra: LAMP_PENUMBRA,
        taps: 9,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    // A control, not a rung: the same nine rays a pixel with nothing to
    // walk — the packer leaves a light's block empty when no occluder
    // reaches it, and an empty block answers "nothing stands in the way"
    // without a record load. `taps-9` minus this row is what the walk
    // costs; this row minus `lamps-10` is what the rays cost when no
    // occluder is in front of them.
    Scenario {
        name: "taps-9-clear",
        lamps: 10,
        occluders: 0,
        penumbra: LAMP_PENUMBRA,
        taps: 9,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    // The shape rows. `taps-1` and `taps-9` say what sixty *boxes* cost;
    // these say what sixty shapes cost, same count, same area, same lamps —
    // and a polygon's cost is not the box's: its bounds are only the cheap
    // reject in front of a walk over the shape's edges, up to sixteen of
    // them, and `clip_planes` is run per ray. Without these rows the shape
    // is unmeasured, and an optimisation aimed at it would be the guess
    // `docs/benchmarking.md` §19 is a lesson against.
    Scenario {
        name: "polys-60",
        lamps: 10,
        occluders: 60,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 6,
        circle: false,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "taps-9-polys",
        lamps: 10,
        occluders: 60,
        penumbra: LAMP_PENUMBRA,
        taps: 9,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 6,
        circle: false,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        // Sixty circles, declared as circles — through `Canvas::occluder_circle`,
        // which is the road an author takes now that nothing traces one.
        //
        // The name is §34's, and it outlived what it named for exactly one
        // section: under §34 this row reached the field as a circle only because
        // the packer recognised the radius a hand-built disc remembered, and
        // from §35 until this comment it drew boxes instead, its own circle
        // branch sitting below the `poly_sides == 0` return it never got past.
        // Sixty boxes is not a comparison, so read §36 for what this row is
        // worth and §35's withdrawal for what it was mistakenly said to be worth.
        name: "taps-9-discs",
        lamps: 10,
        occluders: 60,
        penumbra: LAMP_PENUMBRA,
        taps: 9,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: true,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        // Sixty traced sixteen-gons: the plane walk at its cap, which is what an
        // author gets for rounding a shape off by hand and is the row this suite
        // lost when a circle stopped being a polygon. `taps-9-discs` beside it is
        // the same covered area answered without edges, so the gap between these
        // two rows *is* the price of an approximation, measured rather than argued.
        name: "taps-9-poly16",
        lamps: 10,
        occluders: 60,
        penumbra: LAMP_PENUMBRA,
        taps: 9,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 16,
        circle: false,
        particles: 0,
        p_cover: 0.0,
        p_radius: 0.0,
        p_lit: false,
    },
    // The particle probe. Two ladders that each hold one thing fixed, because
    // a batch costs two different things: it costs per particle (instances
    // emitted, 32 bytes packed — §24 measured that column at ~15 ns each on
    // the CPU) and it costs per covered pixel (shaded, and shaded again per
    // light if the batch is lit). `parts-1k/10k/100k` hold the *coverage*
    // fixed at a quarter of the window while the count runs up two orders of
    // magnitude, so their slope is the per-particle cost; `parts-10k-big`
    // holds the count fixed and quadruples the coverage, so the step from
    // `parts-10k` is the per-pixel cost; and `parts-10k-lit` is the same
    // frame with the batch as a receiver, which is where §19-§23's work
    // reaches a particle scene at all.
    Scenario {
        name: "parts-1k",
        lamps: 0,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 1_000,
        p_cover: 0.25,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "parts-10k",
        lamps: 0,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 10_000,
        p_cover: 0.25,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "parts-100k",
        lamps: 0,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 100_000,
        p_cover: 0.25,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "parts-10k-big",
        lamps: 0,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 10_000,
        p_cover: 1.0,
        p_radius: 0.0,
        p_lit: false,
    },
    // A canary for the rows above: 200 circles of 500 user pixels each is a
    // hundred times the window's area in one batch. If the batch is drawn at
    // all this row is dramatically slower than `fill-1080p`; if it is not,
    // the two read the same and every other particle row here is measuring
    // an empty frame.
    Scenario {
        name: "parts-huge",
        lamps: 0,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 200,
        p_cover: 0.0,
        p_radius: 500.0,
        p_lit: false,
    },
    // The two rows that ask the one question particles and lighting share: the
    // same batch over the same ten lamps, once as an unlit quad layer and once
    // as a receiver that pays `light_mix` per covered pixel. Their difference
    // is what §19-§23 bought a particle scene. A `lit` row with no lamps in it
    // — the first version of this row — asks nothing at all, which is worth
    // recording because that is how it shipped for an hour.
    // The lit batch over the cliff, which is the frame §25 predicts is the most
    // expensive thing in the engine and had no right to predict it. `light_mix`
    // is the same function for a particle as for a wall, so a lit batch in
    // front of sixty casters at nine taps should pay the occluder walk per
    // covered pixel — and §22 and §23's savings land there too, since there is
    // no second lighting path for particles to miss them. `parts-10k-shadow-lit`
    // holds the casters and drops the disk to one hard ray, so the step up to
    // the nine-tap row is the penumbra's own price on particle pixels; the
    // unlit twins say how much of any of it is lighting at all. Their controls
    // are `taps-9` and `taps-1`, the same scenes with no batch in them.
    Scenario {
        name: "parts-10k-cliff",
        lamps: 10,
        occluders: 60,
        penumbra: LAMP_PENUMBRA,
        taps: 9,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 10_000,
        p_cover: 0.25,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "parts-10k-cliff-lit",
        lamps: 10,
        occluders: 60,
        penumbra: LAMP_PENUMBRA,
        taps: 9,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 10_000,
        p_cover: 0.25,
        p_radius: 0.0,
        p_lit: true,
    },
    // The twin `parts-10k-shadow-lit` was missing: the same batch, unlit, over
    // the same sixty casters at one hard ray. Without it the lighting-only cost
    // of a lit batch in this scene could not be subtracted, and §25's open-scene
    // residual had nothing to be compared against.
    Scenario {
        name: "parts-10k-shadow",
        lamps: 10,
        occluders: 60,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 10_000,
        p_cover: 0.25,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "parts-10k-shadow-lit",
        lamps: 10,
        occluders: 60,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 10_000,
        p_cover: 0.25,
        p_radius: 0.0,
        p_lit: true,
    },
    Scenario {
        name: "parts-10k-lamps",
        lamps: 10,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 10_000,
        p_cover: 0.25,
        p_radius: 0.0,
        p_lit: false,
    },
    Scenario {
        name: "parts-10k-lit",
        lamps: 10,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        poly_sides: 0,
        circle: false,
        particles: 10_000,
        p_cover: 0.25,
        p_radius: 0.0,
        p_lit: true,
    },
];

/// A static batch of `n` particles, scattered over the window by the suite's
/// own scatter and sized so the batch covers `cover` of the window's area: a
/// circle's area is pi r^2, shared over n of them.
fn batch(n: usize, cover: f32, radius: f32) -> ParticleSystem {
    let r = if radius > 0.0 {
        radius
    } else {
        (W * H * cover / (n as f32 * std::f32::consts::PI)).sqrt()
    };
    let mut sys = ParticleSystem::new();
    let mut bits = 2463534242u32;
    for _ in 0..n {
        bits ^= bits << 13;
        bits ^= bits >> 17;
        bits ^= bits << 5;
        let a = (bits >> 8) as f32 / 16777216.0;
        bits ^= bits << 13;
        bits ^= bits >> 17;
        bits ^= bits << 5;
        let b = (bits >> 8) as f32 / 16777216.0;
        sys.spawn(Particle {
            pos: [a * W - W / 2.0, b * H - H / 2.0],
            vel: [0.0; 2],
            life: 9.0,
            max_life: 9.0,
            size: r,
            angle: 0.0,
            color: Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
        });
    }
    sys
}

/// The scatter both lamps and occluders use: deterministic, uniform,
/// and identical across runs so no A/B ever compares two different
/// pictures. The golden-ratio sequence needs no RNG and no state.
fn scatter(i: usize) -> [f32; 2] {
    let k = i as f32 + 0.5;
    let gx = (k * 0.7548777_f32).fract();
    let gy = (k * 0.5698403_f32).fract();
    [gx * W * 0.94 - W * 0.47, gy * H * 0.94 - H * 0.47]
}

/// One scenario's scene: a full-window lit receiver with the lamps
/// hung above it. Occluders are immediate — they are registered per
/// frame, exactly as a game's would be.
fn build(s: &Scenario) -> Vec<SceneNode> {
    // Index 0 is always the receiver: the full-window lit plane every
    // light loop is evaluated on. The lamps hang above it; occluders
    // are immediate, registered per frame from `process` like a live
    // camera's would be.
    let mut nodes = vec![SceneNode {
        shape: Some(Shape::Rectangle {
            center: [0.0, 0.0],
            extent: [W, H],
            color: Color {
                r: 0.55,
                g: 0.55,
                b: 0.58,
                a: 1.0,
            },
        }),
        lit: true,
        ..Default::default()
    }];
    for i in 0..s.lamps {
        let light = Light::point(
            Color {
                r: 1.0,
                g: 0.95,
                b: 0.85,
                a: 1.0,
            },
            1.0,
            s.reach,
        )
        .with_penumbra(s.penumbra)
        .with_shadow_taps(s.taps);
        nodes.push(SceneNode {
            shape: Some(Shape::Light { light }),
            transform: Transform::translate(scatter(i)),
            ..Default::default()
        });
    }
    if s.particles > 0 {
        nodes.push(SceneNode {
            shape: Some(Shape::Particles {
                system: batch(s.particles, s.p_cover, s.p_radius),
                color: Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                shape: ParticleShape::Circle,
            }),
            lit: s.p_lit,
            ..Default::default()
        });
    }
    nodes
}

struct Suite {
    /// The scenarios this run walks, in order, `--reps` passes deep.
    queue: Vec<&'static Scenario>,
    idx: usize,
    /// Clock and frames of the measurement in hand; the first second
    /// of every measurement warms the clocks and goes uncounted.
    t: f32,
    frames: u32,
    secs: f32,
    results: Vec<(&'static str, f32)>,
    baseline: Vec<(String, f32)>,
    save: Option<PathBuf>,
    /// Where the ruler lives, settled on the first frame — see
    /// [`Suite::load_baseline`].
    baseline_path: PathBuf,
    /// Whether `--baseline` named a file. Asked-for wins; the tool only gets to
    /// prefer this GPU's own file when nobody pointed at one.
    baseline_asked: bool,
    /// What the file we write to already holds, so a run adds its rows to it
    /// instead of replacing it.
    existing: Baseline,
    loaded: bool,
    /// `--reps`, and whether the frames came out of a fullscreen box: a
    /// baseline header says how its numbers were arrived at, not only what.
    reps: usize,
    fullscreen: bool,
    /// The witness, awake from before the first scenario to after the last.
    /// `Option` because it is handed in at construction and taken once at the
    /// verdict; nothing else may look at it, and a taken canary must not be
    /// reportable twice.
    canary: Option<noisefloor::Canary>,
    done: bool,
}

impl Process for Suite {
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        if self.done {
            return;
        }
        if !self.loaded {
            self.load_baseline();
        }
        let s = self.queue[self.idx];
        // The lamps drift — the little drift a living game has: awake
        // ants breathe, and here their positions do too.
        if s.moving {
            // Index 0 is the receiver; only the lamps drift.
            for (i, child) in ctx.scene().root.children[1..].iter_mut().enumerate() {
                let [x, y] = scatter(i);
                child.transform = Transform::translate([
                    x + (self.t + i as f32).sin() * 60.0,
                    y + (self.t * 0.8 + i as f32).cos() * 40.0,
                ]);
            }
        }
        // Occluders are re-declared every frame, as a game with a
        // moving camera must; positions ride the same scatter.
        for i in 0..s.occluders {
            let [x, y] = scatter(i + s.lamps * 7919);
            // The form is asked before the cheap case, not after it. A circle has
            // no sides, so it arrives with `poly_sides: 0`, and a box branch
            // tested first returned sixty boxes from the circle row, and the
            // number reached the table under a shape the fixture had never
            // declared — which is how §35 came to print a control, the same
            // scene measured twice, as a result of −0.00 ms.
            if s.circle {
                // Same covered area as the suite's 180x120 box — a circle's area
                // is pi r squared — so the disc row and the box row shade the
                // same amount of screen and differ only in what the field has to
                // know about the shape.
                let radius = (21600.0 / std::f32::consts::PI).sqrt();
                ctx.occluder_circle(Transform::translate([x, y]), [0.0, 0.0], radius);
                continue;
            }
            if s.poly_sides == 0 {
                ctx.occluder(Transform::translate([x, y]), [0.0, 0.0], [90.0, 60.0]);
                continue;
            }
            // Same area as the suite's 180x120 box, so the rows differ in their
            // shape and not in how much of the screen they cover: a regular
            // polygon's area is (1/2) n R^2 sin(2 pi / n).
            let n = s.poly_sides as f32;
            let radius = (2.0 * 21600.0 / (n * (2.0 * std::f32::consts::PI / n).sin())).sqrt();
            let points: Vec<[f32; 2]> = (0..s.poly_sides as usize)
                .map(|k| {
                    let a = 2.0 * std::f32::consts::PI * k as f32 / n;
                    [a.cos() * radius, a.sin() * radius]
                })
                .collect();
            let shape = Convex::new(&points).expect("a regular polygon is convex");
            ctx.occluder_polygon(Transform::translate([x, y]), &shape);
        }
        self.t += dt;
        if self.t > 1.0 {
            self.frames += 1;
        }
        if self.t >= self.secs + 1.0 {
            let fps = self.frames as f32 / (self.t - 1.0);
            if s.name != "warm" {
                // Results ride stdout, not `log`: the library ships
                // `release_max_level_off`, so a release bench that
                // logged its numbers would measure and say nothing.
                println!("benchsuite: {} {:.1} fps", s.name, fps);
                self.results.push((s.name, fps));
            }
            self.idx += 1;
            self.t = 0.0;
            self.frames = 0;
            if self.idx == self.queue.len() {
                self.finish(ctx);
                return;
            }
            let next = self.queue[self.idx];
            log::info!(
                "{}: {} occluders declared as {}",
                next.name,
                next.occluders,
                form(next)
            );
            let next = self.queue[self.idx];
            ctx.scene().root.children = build(next).into_iter().map(Box::new).collect();
        }
    }
}

impl Suite {
    /// Read the baseline once, on the first frame — which is the first moment
    /// the process knows the GPU it actually got, and per-chip files only work
    /// if the lookup waits for that.
    ///
    /// Precedence is the human's, then this GPU's own file, then the shared
    /// default: `--baseline` (or `--save-baseline`, whose target is the file to
    /// merge into) wins outright; else `benchsuite.baseline.<chip>` when it
    /// exists; else `benchsuite.baseline`, the historical file the notes cite.
    /// Whatever gets chosen is printed with its own provenance line, and a
    /// header that does not name this GPU is called out — that mismatch is how
    /// an M4 came to be judged by a 610M's numbers for a week without anyone
    /// meaning it to.
    fn load_baseline(&mut self) {
        self.loaded = true;
        if !self.baseline_asked
            && self.save.is_none()
            && let Some(chip) = frost::adapter_name()
        {
            let own = chip_file(&self.baseline_path, chip);
            if own.exists() {
                self.baseline_path = own;
            }
        }
        let target = self
            .save
            .clone()
            .unwrap_or_else(|| self.baseline_path.clone());
        let text = match std::fs::read_to_string(&target) {
            Ok(text) => text,
            Err(_) => {
                println!(
                    "benchsuite: nothing at {} — printing numbers with no verdict",
                    target.display()
                );
                self.existing = Baseline::empty();
                return;
            }
        };
        let read = Baseline::read(&text);
        if self.save.is_some() {
            println!(
                "benchsuite: merging into {} ({} row(s) already on file)",
                target.display(),
                read.rows.len()
            );
        } else {
            println!(
                "benchsuite: judging against {} ({} row(s))",
                target.display(),
                read.rows.len()
            );
            if let Some(line) = read.provenance_line() {
                println!("benchsuite:   {line}");
            }
            if let Some(chip) = frost::adapter_name().filter(|chip| !read.names_chip(chip)) {
                println!(
                    "benchsuite:   NOTE this GPU is {chip} and the file above does not name it — a cross-chip diff is not a regression; see docs/benchmarking.md §14"
                );
            }
            self.baseline = read.rows.clone();
        }
        self.existing = read;
    }

    fn finish(&mut self, ctx: &mut Context) {
        self.done = true;
        // Print the box the frames came out of beside the frames themselves.
        // A Mac's request for 1920×1080 is not what arrives (Retina doubles
        // it), and a fullscreen request settles wherever the platform likes
        // — so the geometry is part of the record, not a detail to infer
        // afterwards. `docs/benchmarking.md` §17 is what inferring cost.
        // The window's state as well as its shape: `(minimized, focused)`, or
        // `None` when there was no window at all — which is its own honest case,
        // since a present path cannot bias a frame that is never presented.
        let mut composed: Option<(bool, bool)> = None;
        if let Some(window) = ctx.window() {
            let inner = window.inner_size();
            let scale = window.scale_factor().max(0.01);
            let logical_w = (inner.width as f64 / scale).round() as u32;
            let logical_h = (inner.height as f64 / scale).round() as u32;
            let monitor = window
                .current_monitor()
                .or_else(|| window.primary_monitor())
                .map(|m| {
                    let size = m.size();
                    format!(
                        "in a {w}x{h} px monitor{rate}",
                        w = size.width,
                        h = size.height,
                        rate = m
                            .refresh_rate_millihertz()
                            .map(|hz| format!(" @ {:.1} Hz", hz as f64 / 1000.0))
                            .unwrap_or_default()
                    )
                })
                .unwrap_or_else(|| "with no monitor reported".into());
            // Focus belongs in the record beside size, because a window nothing
            // composites goes down a different present path: §38 measured 8 to
            // 18 % on the rows short enough for that path to be on the critical
            // path. `is_visible` is not that fact and never was — winit means by
            // it only that nobody has hidden the window — so it stays printed and
            // stops being relied on.
            // Winit answers `None` for minimization where a platform will not
            // say, which reads here as "not proven minimized" and leaves focus to
            // carry the check — refusing on a platform's silence would mean the
            // gate could never be run there at all.
            let minimized = window.is_minimized().unwrap_or(false);
            let focused = window.has_focus();
            composed = Some((minimized, focused));
            println!(
                "benchsuite: drew into {}x{} px ({}x{} at {:.2}x) {}, fullscreen: {}, minimized: {}, focused: {}, visible: {:?}",
                inner.width,
                inner.height,
                logical_w,
                logical_h,
                scale,
                monitor,
                window.fullscreen().is_some(),
                minimized,
                focused,
                window.is_visible()
            );
        }
        let floor = self
            .canary
            .take()
            .expect("started beside the app, reported once")
            .report();
        let medians = medians(&self.results);
        for (name, fps) in &medians {
            println!("benchsuite: {name} median {:.1} fps", fps);
        }
        // The weather, printed after the numbers and before any verdict on
        // them, which is the order a reader needs it in.
        println!("benchsuite: noise floor {floor}");
        if floor.too_loud() && (self.save.is_some() || !self.baseline.is_empty()) {
            // Refusing here protects both halves of the comparison. Judging a
            // loud run against a quiet baseline invents a regression; writing
            // a loud run *as* a baseline invents one that lasts until someone
            // next notices — and a poisoned baseline is worse than a missing
            // one, because a missing one is obvious. Exit 3 says "this run
            // measured, and declined to conclude"; 1 stays reserved for a real
            // regression, so a script can tell them apart.
            println!(
                "benchsuite: too loud to judge or to save a baseline — these numbers are the machine's mood, not its speed; see tools/noisefloor"
            );
            std::process::exit(3);
        }
        if let Some(reason) =
            uncomposed_reason(composed).filter(|_| self.save.is_some() || !self.baseline.is_empty())
        {
            // The same refusal as a loud floor, for the same reason: this run
            // measured something, just not the thing a baseline records. §38
            // measured the cost on the RTX 4090 — 8 to 18 % on every row short
            // enough for the present path to show in it, close to a constant 40
            // microseconds a frame — and the mirror image is worse than the
            // inflation: a run from an unfocused window reads −10.3 % against a
            // file walked from a focused one, so the tool would report a
            // regression that landed in the safe direction, which is precisely
            // the one nobody stops to question. Reporting stays allowed, exactly
            // as it does for a busy machine; concluding does not.
            println!("benchsuite: {reason} — printing the numbers, declining to judge or to save");
            println!(
                "benchsuite: a window nothing composites is a different present path, not a slower engine — docs/benchmarking.md §38"
            );
            std::process::exit(3);
        }
        if let Some(path) = &self.save {
            if self.existing.unparsed > 0 {
                // Refusing beats representing. A file with lines this tool
                // cannot hold would come back shorter and tidier than it went
                // in, and the rows it lost would be missing in the way only a
                // green run produces: nobody the wiser. Exit 3 is the same
                // "measured, declined to conclude" the loud floor gets.
                println!(
                    "benchsuite: {} holds {} line(s) this tool cannot represent — not overwriting a baseline it cannot reproduce",
                    path.display(),
                    self.existing.unparsed
                );
                std::process::exit(3);
            }
            let box_of_it = ctx
                .window()
                .map(|w| {
                    let size = w.inner_size();
                    format!("{}x{} px", size.width, size.height)
                })
                .unwrap_or_else(|| "an offscreen box".into());
            let chip = frost::adapter_name().unwrap_or("a GPU the engine never named");
            // The state that opened the gate is written down beside the rest of
            // it, so a file says whether a window was composited while it was
            // being made instead of leaving that to whoever remembers.
            let window_state = match composed {
                None => "with no window",
                Some((true, _)) => "minimized",
                Some((false, false)) => "unfocused",
                Some((false, true)) => "focused",
            };
            let provenance = format!(
                "# walked {} on {}, {} in {}, {} reps per row, window {} — see docs/benchmarking.md",
                utc_today(),
                chip,
                box_of_it,
                if self.fullscreen {
                    "fullscreen"
                } else {
                    "a window"
                },
                self.reps.max(1),
                window_state
            );
            let merged = Baseline {
                header: self.existing.header.clone(),
                rows: merge_rows(&self.existing.rows, &medians),
                unparsed: 0,
            };
            let kept = merged.rows.len();
            let wrote = medians.len();
            match std::fs::write(path, merged.write(&provenance)) {
                Ok(()) => println!(
                    "benchsuite: baseline written to {} — {wrote} row(s) walked on {chip}, {kept} on file",
                    path.display()
                ),
                Err(err) => eprintln!("benchsuite: cannot write baseline: {err}"),
            }
            if chip_file(&PathBuf::from("benchsuite.baseline"), chip) != *path
                && frost::adapter_name().is_some()
            {
                println!(
                    "benchsuite: this GPU's own file would be {} — a baseline walked elsewhere judges nothing here",
                    chip_file(&PathBuf::from("benchsuite.baseline"), chip).display()
                );
            }
        } else if !self.baseline.is_empty() {
            let mut regressed = false;
            for (name, now) in &medians {
                match self.baseline.iter().find(|(b, _)| b == name) {
                    Some((_, base)) => {
                        let delta = (now / base - 1.0) * 100.0;
                        let line = format!("{name} {base:.1} -> {now:.1} ({delta:+.1}%)");
                        if delta <= -REGRESSION_PCT {
                            regressed = true;
                            eprintln!("benchsuite: REGRESSED {line}");
                        } else {
                            println!("benchsuite: ok {line}");
                        }
                    }
                    None => println!("benchsuite: {name} is new to the baseline"),
                }
            }
            if regressed {
                std::process::exit(1);
            }
        }
        ctx.exit();
    }
}

/// A regression is a regression at five percent; below that the
/// machine's own weather is the louder voice.
const REGRESSION_PCT: f32 = 5.0;

/// One median per scenario, in first-seen order.
fn medians(results: &[(&'static str, f32)]) -> Vec<(String, f32)> {
    let mut names: Vec<String> = Vec::new();
    let mut out: Vec<(String, f32)> = Vec::new();
    for (name, _) in results {
        if !names.iter().any(|n| n == name) {
            names.push((*name).to_string());
        }
    }
    for name in names {
        let mut v: Vec<f32> = results
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, f)| *f)
            .collect();
        v.sort_by(f32::total_cmp);
        let mid = v.len() / 2;
        let m = if v.len().is_multiple_of(2) {
            (v[mid - 1] + v[mid]) / 2.0
        } else {
            v[mid]
        };
        out.push((name, m));
    }
    out
}

/// A baseline file, kept as it was found.
///
/// One `name fps` pair per line, plain and diffable — and a header, which is
/// the only thing in the file saying *whose* numbers these are. That is not
/// decoration: the committed `benchsuite.baseline` was walked on a Radeon 610M
/// and says `taps-9 16.0`, while an M4 Max reads the same row at 212.9, so
/// judging one against the other reports a twelve-fold speedup on a build that
/// changed nothing. A file whose header got overwritten in a rewrite is a
/// number with no owner, and the next box to read it has no way to know.
struct Baseline {
    /// Every `#` line, in the order found. Written back verbatim.
    header: String,
    /// The rows, in file order.
    rows: Vec<(String, f32)>,
    /// Lines that were neither comment nor row. Kept to refuse with, not to
    /// guess at: rewriting a file the tool cannot fully represent is how
    /// provenance dies.
    unparsed: usize,
}

impl Baseline {
    const TITLE: &'static str = "# benchsuite baseline — one name and median fps per line";

    fn read(text: &str) -> Self {
        let mut header = String::new();
        let mut rows = Vec::new();
        let mut unparsed = 0;
        for line in text.lines() {
            if line.trim_start().starts_with('#') {
                header.push_str(line);
                header.push('\n');
            } else if line.trim().is_empty() {
                continue;
            } else {
                match line
                    .rsplit_once(' ')
                    .and_then(|(n, f)| Some((n.trim().to_string(), f.trim().parse::<f32>().ok()?)))
                {
                    Some(row) => rows.push(row),
                    None => unparsed += 1,
                }
            }
        }
        Self {
            header,
            rows,
            unparsed,
        }
    }

    fn empty() -> Self {
        Self {
            header: format!("{}\n", Self::TITLE),
            rows: Vec::new(),
            unparsed: 0,
        }
    }

    /// The file as it goes back to disk: header, then this walk's provenance
    /// line, then the rows. The provenance line is where the chip, the
    /// resolution and the rep count live, because a number without them is a
    /// rumour — see `docs/benchmarking.md` §14 for what inferring cost.
    fn write(&self, provenance: &str) -> String {
        let mut out = String::new();
        if self.header.is_empty() {
            out.push_str(Self::TITLE);
            out.push('\n');
        } else {
            out.push_str(&self.header);
        }
        if !out.lines().any(|l| l.trim_start().starts_with("# walked")) {
            out.push_str(provenance);
            out.push('\n');
        }
        for (name, fps) in &self.rows {
            out.push_str(&format!("{name} {fps:.1}\n"));
        }
        out
    }

    /// The header's own account of where it came from, for printing beside a
    /// verdict: the line that says when and on what.
    fn provenance_line(&self) -> Option<&str> {
        // `# walked`, at the start of a line, rather than the word anywhere:
        // the historical header is a wrapped paragraph and its third line
        // reads "this box re-walked them on", which is a footnote about a
        // second walk, not the account of the first.
        let mut walked = None;
        for line in self.header.lines() {
            if line.trim_start().starts_with("# walked") {
                walked = Some(line);
            }
        }
        walked.or_else(|| self.header.lines().last())
    }

    /// Whether the header names the GPU this process is drawing on. The test
    /// is on normalised words rather than the slug, because driver strings
    /// arrive punctuated every way a driver can: the 610M's file says
    /// `RADV RAPHAEL_MENDOCINO` and the driver says the same, and neither has
    /// to agree with how a file name would have spelled it.
    fn names_chip(&self, chip: &str) -> bool {
        let words = |t: &str| {
            t.to_lowercase().chars().fold(String::new(), |mut a, c| {
                if c.is_ascii_alphanumeric() {
                    a.push(c);
                } else {
                    a.push(' ');
                }
                a
            })
        };
        let hay = words(&self.header);
        let needle = words(chip);
        !needle.is_empty() && hay.contains(needle.trim())
    }
}

/// Fold a run's medians into the rows already on file: names just measured are
/// replaced where they stand, names not measured keep their numbers, and names
/// the file has never seen are appended.
///
/// This is what makes a baseline walkable in pieces. A loud day retires one
/// row; `--only that-row` puts it back without touching the other twenty-six —
/// which is the whole reason a partial run may not simply rewrite the file,
/// the way it used to, reducing a ten-row ruler to the one row asked for.
fn merge_rows(existing: &[(String, f32)], measured: &[(String, f32)]) -> Vec<(String, f32)> {
    let mut out = existing.to_vec();
    for (name, fps) in measured {
        match out.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = *fps,
            None => out.push((name.clone(), *fps)),
        }
    }
    out
}

/// `Apple M4 Max` becomes `apple-m4-max`, for a file that says which chip it
/// was walked on. Driver strings do change between updates, which is why the
/// full name also lives inside the file and not only in its name.
fn adapter_slug(chip: &str) -> String {
    let mut out = String::new();
    for c in chip.chars() {
        if c.is_ascii_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Which baseline file to use: the one asked for, else this GPU's own file when
/// it has written one, else the shared default.
///
/// The default stays `benchsuite.baseline` — the historical file the
/// benchmarking notes cite — because quietly moving the ruler under a run that
/// never asked for a new one is a worse surprise than a warning. Once a box
/// has written `benchsuite.baseline.apple-m4-max`, judging against it is the
/// obvious intent, and it is the tool's job to know that rather than leave it
/// to the command line.
fn chip_file(default: &Path, chip: &str) -> PathBuf {
    let slug = adapter_slug(chip);
    if slug.is_empty() {
        return default.to_path_buf();
    }
    let stem = default
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "benchsuite.baseline".into());
    default.with_file_name(format!("{stem}.{slug}"))
}

/// Today, UTC, as `YYYY-MM-DD`, out of the clock alone.
///
/// The date sits in a baseline header beside the chip for the same reason the
/// chip does: numbers are conditioned, and "when" is one of the conditions.
/// Standard days-from-civil inversion rather than a date crate, so an example
/// stays an example; the fixed epochs in the tests below are what say it works.
fn utc_today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    civil_from_days(secs.div_euclid(86_400))
}

/// The inverse of Howard Hinnant's `days_from_civil`: epoch day zero is
/// 1970-01-01, and the shifts put the year at the point where the leap
/// exceptions fall at the end rather than the start.
fn civil_from_days(days: i64) -> String {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

/// `--only` filtering: names in the order the suite gave them.
/// What a row's casters actually are, in three words. The table's row names are
/// prose; this is read off the fixture's own flags, which is the difference
/// between a claim and a receipt — `taps-9-discs` was declared boxes for one
/// whole section while its name said circles.
fn form(s: &Scenario) -> &'static str {
    if s.circle {
        "circles"
    } else if s.poly_sides == 0 {
        "boxes"
    } else {
        "polygons"
    }
}

/// The rows to run, in the order they were asked for.
///
/// The order is the answer and not a detail: the house discipline asks the same
/// rows in both orders inside one invocation, because a row measured second in a
/// process reads differently from the same row measured first. This used to sort
/// the request back into suite order, so `--only a,b` and `--only b,a` were the
/// same command, and every pass that believed it had flipped its rows had
/// measured one order twice — the §36 pass that found this is the receipt. An
/// absent `--only` still runs the table's own order, which is the only order the
/// full suite has.
fn select<'a>(only: &Option<String>, scenarios: &'a [Scenario]) -> Vec<&'a Scenario> {
    match only {
        None => scenarios.iter().collect(),
        Some(list) => {
            let want: Vec<&str> = list.split(',').map(str::trim).collect();
            want.iter()
                .filter_map(|name| scenarios.iter().find(|s| s.name == *name))
                .collect()
        }
    }
}

/// `--render-size` parsing: `1920x1080`, nothing else. Returns `None` on
/// anything that is not two numbers around one `x`, so the flag refuses at
/// the command line rather than sizing a window by accident.
fn parse_size(spec: &str) -> Option<[u32; 2]> {
    let (w, h) = spec.split_once('x')?;
    Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
}

#[derive(clap::Parser)]
struct Cli {
    /// Seconds measured per scenario (after a one-second warm-up).
    #[arg(long, default_value_t = 6.0)]
    seconds: f32,
    /// Full passes over the scenario list; the median of N is the
    /// number the suite reports and judges.
    #[arg(long, default_value_t = 3)]
    reps: usize,
    /// Comma-separated scenario names; default is the whole suite.
    #[arg(long)]
    only: Option<String>,
    /// Compare against this baseline file (default: `benchsuite.baseline`
    /// beside the launch directory, when present).
    #[arg(long)]
    baseline: Option<PathBuf>,
    /// Write this run's medians to the file as the new baseline,
    /// instead of judging the run against one.
    #[arg(long)]
    save_baseline: Option<PathBuf>,
    /// Open the window fullscreen — the platform's native fullscreen Space
    /// on macOS, borderless elsewhere.
    ///
    /// It exists because a windowed run on the M4 Max presented exactly
    /// 60.000 frames a second for thirteen seconds under
    /// `present_mode AutoNoVsync`, floor and cliff together at 60.2 and
    /// 60.0: the fps column was the panel, not the GPU (see
    /// `docs/benchmarking.md` §6 and §14's ratio test). Whether a Space of
    /// its own lets the suite outrun the display is a measurement, not a
    /// promise — read `--only fill-1080p,taps-9` and divide before trusting
    /// anything this flag produces.
    #[arg(long)]
    fullscreen: bool,
    /// Render at a fixed physical size, `WxH`, whatever the window becomes.
    ///
    /// The suite asks for 1920×1080 logical pixels, and a Retina display
    /// doubles that: the same `fill-1080p` frame is 8.3 Mpx on a Mac and
    /// 2.07 Mpx on the Linux box the committed table was walked on, so a
    /// Mac row read straight off the window would be weighing four times
    /// the pixels. This pins the work to the pixels the name promises,
    /// through the engine's `Config::render_size`, and the window stretches
    /// the result to whatever it became — one sampling pass over the
    /// window's own pixels, which is why it is a flag and not the default.
    #[arg(long)]
    render_size: Option<String>,
}

fn main() {
    // A row that asks for two forms at once would have to be given one of them
    // by the builder's order, and the order is exactly the thing that went wrong
    // once. The fixture refuses instead of choosing.
    for s in SCENARIOS {
        assert!(
            !(s.circle && s.poly_sides > 0),
            "`{}` sets circle and poly_sides {}; the builder would have to pick, \
             and the row would mean whichever it lost",
            s.name,
            s.poly_sides
        );
    }
    env_logger::init();
    let cli: Cli = clap::Parser::parse();
    let chosen = select(&cli.only, SCENARIOS);
    if chosen.is_empty() {
        eprintln!("benchsuite: --only matched no scenario");
        std::process::exit(2);
    }
    let render_size = cli.render_size.as_deref().map(|spec| {
        parse_size(spec).unwrap_or_else(|| {
            eprintln!("benchsuite: --render-size wants WxH, like 1920x1080");
            std::process::exit(2);
        })
    });
    if render_size.is_some() || cli.fullscreen {
        // The two flags exist to change what a number means, so say which
        // of them are in force before the first frame costs anything.
        eprintln!(
            "benchsuite: running {}{}",
            if cli.fullscreen {
                "fullscreen"
            } else {
                "windowed"
            },
            match render_size {
                Some([w, h]) => format!(", rendering {w}x{h} physical pixels"),
                None => ", rendering at the window's own pixels".into(),
            }
        );
    }
    let mut queue = vec![&WARM];
    for _ in 0..cli.reps.max(1) {
        queue.extend(chosen.iter().copied());
    }
    let first = queue[0];
    // Which file, and what it holds, is settled on the first frame rather than
    // here: the process does not know its own GPU until startup has asked for
    // one, and per-chip baselines are the whole point of the lookup.
    let baseline_path = cli
        .baseline
        .clone()
        .unwrap_or_else(|| PathBuf::from("benchsuite.baseline"));
    let baseline_asked = cli.baseline.is_some();
    let mut scene = Scene::new(SceneNode::default());
    scene.ambient = Color {
        r: 0.15,
        g: 0.16,
        b: 0.20,
        a: 1.0,
    };
    log::info!(
        "{}: {} occluders declared as {}",
        first.name,
        first.occluders,
        form(first)
    );
    scene.root.children = build(first).into_iter().map(Box::new).collect();
    if let Err(err) = run_configured(
        scene,
        Suite {
            queue,
            idx: 0,
            t: 0.0,
            frames: 0,
            secs: cli.seconds,
            results: Vec::new(),
            baseline: Vec::new(),
            save: cli.save_baseline,
            baseline_path,
            baseline_asked,
            existing: Baseline::empty(),
            loaded: false,
            reps: cli.reps,
            fullscreen: cli.fullscreen,
            canary: Some(noisefloor::Canary::start()),
            done: false,
        },
        Config {
            vsync: false,
            window_size: Some([W as u32, H as u32]),
            fullscreen: cli.fullscreen,
            render_size,
            ..Default::default()
        },
    ) {
        eprintln!("benchsuite: {err}");
        std::process::exit(1);
    }
}

/// Why this run cannot be trusted to have measured a presented frame, or `None`
/// when it can.
///
/// The input is the window's `(minimized, focused)` state, or `None` when the run
/// had no window at all — and no window is a complete answer rather than a missing
/// one, because a frame that is never presented cannot be paced or clamped by a
/// present path. That is the offscreen meter's whole argument, and
/// `docs/benchmarking.md` §38 is what assuming visibility instead cost: twenty-two
/// of twenty-six rows moved 8 to 18 % when the window went from uncomposited to on
/// screen, by a near-constant 40 microseconds a frame.
///
/// Focus is a proxy and is treated as one. Occlusion is not queryable portably, so
/// this refuses the two states that are, and names which one it saw. A focused
/// window with something drawn over it remains possible and remains unrecordable —
/// the point of these numbers is to make a reader distrust that case, not to
/// pretend the check covers it.
fn uncomposed_reason(window: Option<(bool, bool)>) -> Option<&'static str> {
    match window {
        None => None,
        Some((true, _)) => Some(
            "the window is minimized, so what the GPU drew here was never a frame the screen was asked to show",
        ),
        Some((false, false)) => Some(
            "the window has no focus, so nothing may be compositing it and the present path is doing different work",
        ),
        Some((false, true)) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `--render-size` flag exists so a Retina run can be asked for the
    /// pixels its row name promises, so the shape it accepts is a fact:
    /// numbers around one `x`, nothing else.
    #[test]
    fn the_render_size_flag_reads_wxh_and_refuses_the_rest() {
        assert_eq!(parse_size("1920x1080"), Some([1920, 1080]));
        assert_eq!(parse_size(" 1920 x 1080 "), Some([1920, 1080]));
        assert_eq!(parse_size("1920"), None);
        assert_eq!(parse_size("1920x"), None);
        assert_eq!(parse_size("widexhigh"), None);
        assert_eq!(parse_size("1920:1080"), None);
    }

    #[test]
    fn every_scenario_has_a_unique_name() {
        for (i, a) in SCENARIOS.iter().enumerate() {
            for b in &SCENARIOS[i + 1..] {
                assert_ne!(a.name, b.name);
            }
        }
    }

    #[test]
    fn the_ladder_rungs_open_the_disk_before_sweeping_it() {
        // The suite's own story, asserted so the table cannot drift:
        // rung 1 keeps the disk closed (hard ray), rungs 4 and 9 open
        // it and sweep it with ever more taps.
        let find = |n: &str| SCENARIOS.iter().find(|s| s.name == n).unwrap();
        assert_eq!(find("taps-1").penumbra, 0.0);
        assert!(find("taps-4").penumbra > 0.0);
        assert!(find("taps-9").penumbra > 0.0);
        assert!(find("taps-4").taps < find("taps-9").taps);
        // And taps are only ever asked of an open disk.
        for s in SCENARIOS {
            assert!(
                s.taps == 1 || s.penumbra > 0.0,
                "{}: taps without a disk",
                s.name
            );
        }
    }

    #[test]
    fn medians_take_the_middle_of_the_reps() {
        let r = [
            ("fill", 80.0),
            ("lamps", 60.0),
            ("fill", 20.0),
            ("lamps", 62.0),
            ("fill", 82.0),
            ("lamps", 58.0),
        ];
        let m = medians(&r);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].0, "fill");
        assert_eq!(m[0].1, 80.0);
        assert_eq!(m[1].0, "lamps");
        assert_eq!(m[1].1, 60.0);
    }

    #[test]
    fn medians_average_the_middle_pair_of_even_reps() {
        let r = [("fill", 70.0), ("fill", 80.0)];
        assert_eq!(medians(&r)[0].1, 75.0);
    }

    #[test]
    fn baselines_round_trip_through_their_own_text() {
        let on_file = Baseline::read(
            "# benchsuite baseline — one name and median fps per line\n\ntaps-9 16.0\n",
        );
        let written = on_file.write("# walked 2026-10-28 on a test box");
        let back = Baseline::read(&written);
        assert_eq!(back.rows.len(), 1, "the row survives its own text");
        assert_eq!(back.rows[0].0, "taps-9");
        assert!((back.rows[0].1 - 16.0).abs() < 0.01);
        assert!(
            back.header.contains("one name and median fps"),
            "writing a file must not cost it its title"
        );
        assert!(
            back.header.contains("walked 2026-10-28"),
            "a file with no provenance gains this walk's"
        );
        // And only ever once: provenance describes where the rows came from,
        // so a second walk must leave the first account standing rather than
        // stack a second claim under it.
        let again = back.write("# walked 2027-01-01 on a different box");
        assert_eq!(
            again.lines().filter(|l| l.contains("walked ")).count(),
            1,
            "one provenance line, the original one: {} / {}",
            again.lines().filter(|l| l.contains("walked ")).count(),
            again
        );
    }

    /// The defect this replaces: `--save-baseline` wrote one generic comment
    /// line and only the rows just measured, so a partial run cut the file
    /// down to itself. Merging is what lets a baseline be walked in pieces —
    /// one loud row re-asked with `--only`, twenty-five neighbours untouched.
    #[test]
    fn a_partial_walk_adds_to_a_baseline_instead_of_replacing_it() {
        let on_file = vec![
            ("fill-1080p".to_string(), 1002.7),
            ("taps-9".to_string(), 16.0),
            ("lamps-10".to_string(), 354.9),
        ];
        let today = vec![
            ("taps-9".to_string(), 14.2),
            ("taps-9-discs".to_string(), 12.8),
        ];
        let merged = merge_rows(&on_file, &today);
        assert_eq!(merged.len(), 4, "one row re-measured, one row new");
        assert_eq!(
            merged[0],
            ("fill-1080p".to_string(), 1002.7),
            "an unmeasured row keeps its number and its place"
        );
        assert_eq!(
            merged[1].1, 14.2,
            "a measured row is replaced where it stood"
        );
        assert_eq!(merged.last().unwrap().0, "taps-9-discs");
    }

    /// A line that is neither comment nor row is counted, not guessed at and
    /// not quietly dropped on the way back out — the save path refuses to
    /// overwrite a file it cannot fully represent.
    #[test]
    fn a_baseline_counts_what_it_cannot_read() {
        let b = Baseline::read("# header\n\ntaps-9 16.0\nregressed on a tuesday\n");
        assert_eq!(b.rows.len(), 1);
        assert_eq!(b.unparsed, 1);
        assert!(b.header.starts_with("# header"));
    }

    /// Whose numbers a file holds has to be readable from it. The 610M's own
    /// file names the chip with underscores and a vendor prefix, so the test
    /// is on normalised words rather than on the spelling a file name would
    /// have chosen — and a chip that is genuinely absent must still fail it.
    #[test]
    fn the_chip_that_wrote_a_baseline_is_readable_from_it() {
        let igpu = Baseline::read(
            "# walked 2026-10-19 on the Radeon 610M iGPU (RADV RAPHAEL_MENDOCINO)\n",
        );
        assert!(igpu.names_chip("RADV RAPHAEL_MENDOCINO"));
        assert!(igpu.names_chip("radv raphael mendocino"));
        assert!(
            !igpu.names_chip("Apple M4 Max"),
            "another chip's file must not pass for this one's"
        );
        assert!(!igpu.names_chip(""), "an empty name matches nothing");
        assert!(
            igpu.provenance_line()
                .unwrap()
                .contains("walked 2026-10-19")
        );
    }

    #[test]
    fn a_chip_gets_a_file_named_after_it() {
        let d = PathBuf::from("benchsuite.baseline");
        assert_eq!(adapter_slug("Apple M4 Max"), "apple-m4-max");
        assert_eq!(
            chip_file(&d, "Apple M4 Max"),
            PathBuf::from("benchsuite.baseline.apple-m4-max")
        );
        assert_eq!(
            chip_file(&d, "NVIDIA GeForce RTX 4090"),
            PathBuf::from("benchsuite.baseline.nvidia-geforce-rtx-4090")
        );
        assert_eq!(
            chip_file(&d, "  "),
            d,
            "a name with nothing in it gets the shared file, not a file called `.`"
        );
    }

    /// The header's date comes from the clock and no dependency, so the
    /// conversion is pinned to epochs whose answers are known rather than to
    /// today. Day 0 is the epoch; day −1 is the day before it, which is where
    /// a naive `/ 86400` would land on 1970 instead.
    #[test]
    fn the_clock_gives_a_date_a_header_can_carry() {
        assert_eq!(civil_from_days(0), "1970-01-01");
        assert_eq!(civil_from_days(-1), "1969-12-31");
        assert_eq!(civil_from_days(20_023), "2024-10-27");
        assert_eq!(civil_from_days(20_754), "2026-10-28");
        assert_eq!(civil_from_days(0).len(), 10);
        assert_eq!(utc_today().len(), 10, "today parses as YYYY-MM-DD");
        assert_eq!(&utc_today()[4..5], "-");
    }

    #[test]
    fn baseline_comments_and_blanks_are_skipped() {
        let b = Baseline::read("# a comment\n\nfill 80.0\n");
        assert_eq!(b.rows.len(), 1);
        assert_eq!(b.unparsed, 0);
    }

    #[test]
    fn only_runs_the_rows_in_the_order_they_were_asked_for() {
        let order = |list: &str| {
            select(&Some(list.into()), SCENARIOS)
                .into_iter()
                .map(|s| s.name)
                .collect::<Vec<_>>()
        };
        // The one row that matters is that these two differ. Sorted back into
        // suite order they were the same command, and an order effect could not
        // be seen even by someone looking for it.
        assert_eq!(order("taps-9, fill-1080p"), ["taps-9", "fill-1080p"]);
        assert_eq!(order("fill-1080p, taps-9"), ["fill-1080p", "taps-9"]);
        // Without `--only` the suite runs itself in table order, and a name that
        // is not in the suite is dropped — loudly, by the caller, which refuses
        // an empty selection.
        // `warm` is not a scenario — it rides the front of the queue in `main`,
        // so it cannot be selected, skipped or judged.
        assert_eq!(select(&None, SCENARIOS)[0].name, "fill-1080p");
        assert!(select(&Some("nonsense".into()), SCENARIOS).is_empty());
    }

    #[test]
    fn scatter_stays_inside_the_window() {
        for i in 0..64 {
            let [x, y] = scatter(i);
            assert!(x.abs() < W / 2.0 && y.abs() < H / 2.0);
        }
    }

    /// §38: whether anything was compositing the window moved the
    /// present-bound rows 8 to 18 %, so a run in that state may print its
    /// numbers and must not judge or write. Minimized outranks focus because
    /// it says something stronger — not that composition is doubtful, but that
    /// there was no presented frame at all.
    #[test]
    fn a_window_nothing_composites_may_report_but_not_conclude() {
        assert!(uncomposed_reason(Some((true, false))).is_some());
        assert!(uncomposed_reason(Some((true, true))).is_some());
        assert!(uncomposed_reason(Some((false, false))).is_some());
        assert_eq!(uncomposed_reason(Some((false, true))), None);
    }

    /// The case a present path cannot reach: with no window there is nothing
    /// to pace, so the absence is not an unknown to refuse on. This is why the
    /// offscreen meter is the instrument §38 sends the reader to instead.
    #[test]
    fn an_offscreen_run_has_no_present_path_to_distrust() {
        assert_eq!(uncomposed_reason(None), None);
    }
}
