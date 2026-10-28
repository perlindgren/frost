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
//! * `polys-60`, `taps-9-polys`, `taps-9-discs` — the casters' *shape*:
//!   sixty hexagons and sixty 16-sided discs at the boxes' own area, so the
//!   rows against `taps-1` and `taps-9` say what a polygon's edges cost on
//!   top of the box's bounds. Without them the polygon path is unmeasured.
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

use std::path::PathBuf;

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
        // Sixty circles, declared as circles. This is the row §34 measured at
        // −75.7 % against the same sixty as a sixteen-sided polygon, and it keeps
        // its name so that number stays comparable down the years: the covered
        // area is unchanged, only the shape's form.
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
            if s.poly_sides == 0 {
                ctx.occluder(Transform::translate([x, y]), [0.0, 0.0], [90.0, 60.0]);
                continue;
            }
            // Same area as the suite's 180x120 box, so the rows differ in
            // their shape and not in how much of the screen they cover: a
            // regular polygon's area is (1/2) n R^2 sin(2 pi / n).
            if s.circle {
                // Same covered area as the suite's 180x120 box — a circle's area
                // is pi r squared — so the disc row and the box row shade the same
                // amount of screen and differ only in what the field has to know
                // about the shape.
                let radius = (21600.0 / std::f32::consts::PI).sqrt();
                ctx.occluder_circle(Transform::translate([x, y]), [0.0, 0.0], radius);
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
            ctx.scene().root.children = build(next).into_iter().map(Box::new).collect();
        }
    }
}

impl Suite {
    fn finish(&mut self, ctx: &mut Context) {
        self.done = true;
        // Print the box the frames came out of beside the frames themselves.
        // A Mac's request for 1920×1080 is not what arrives (Retina doubles
        // it), and a fullscreen request settles wherever the platform likes
        // — so the geometry is part of the record, not a detail to infer
        // afterwards. `docs/benchmarking.md` §17 is what inferring cost.
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
            println!(
                "benchsuite: drew into {}x{} px ({}x{} at {:.2}x) {}, fullscreen: {}, visible: {:?}",
                inner.width,
                inner.height,
                logical_w,
                logical_h,
                scale,
                monitor,
                window.fullscreen().is_some(),
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
        if let Some(path) = &self.save {
            match std::fs::write(path, serialize(&medians)) {
                Ok(()) => println!("benchsuite: baseline written to {}", path.display()),
                Err(err) => eprintln!("benchsuite: cannot write baseline: {err}"),
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

/// The baseline file: one `name fps` pair per line. Plain, diffable,
/// and readable by hand in a commit.
fn serialize(results: &[(String, f32)]) -> String {
    let mut out = String::from("# benchsuite baseline — one name and median fps per line\n");
    for (name, fps) in results {
        out.push_str(&format!("{name} {fps:.1}\n"));
    }
    out
}

fn parse_baseline(text: &str) -> Vec<(String, f32)> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let (name, fps) = l.rsplit_once(' ')?;
            Some((name.trim().to_string(), fps.trim().parse().ok()?))
        })
        .collect()
}

/// `--only` filtering: names in the order the suite gave them.
fn select<'a>(only: &Option<String>, scenarios: &'a [Scenario]) -> Vec<&'a Scenario> {
    match only {
        None => scenarios.iter().collect(),
        Some(list) => {
            let want: Vec<&str> = list.split(',').map(str::trim).collect();
            scenarios
                .iter()
                .filter(|s| want.contains(&s.name))
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
    let baseline_path = cli
        .baseline
        .clone()
        .unwrap_or_else(|| PathBuf::from("benchsuite.baseline"));
    let baseline = if cli.save_baseline.is_none() {
        std::fs::read_to_string(&baseline_path)
            .map(|t| {
                let b = parse_baseline(&t);
                println!(
                    "benchsuite: judging against {} ({})",
                    baseline_path.display(),
                    b.len()
                );
                b
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut scene = Scene::new(SceneNode::default());
    scene.ambient = Color {
        r: 0.15,
        g: 0.16,
        b: 0.20,
        a: 1.0,
    };
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
            baseline,
            save: cli.save_baseline,
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
        let rows = vec![
            ("fill-1080p".to_string(), 86.7),
            ("taps-9".to_string(), 4.4),
        ];
        let back = parse_baseline(&serialize(&rows));
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].0, "fill-1080p");
        assert!((back[1].1 - 4.4).abs() < 0.01);
    }

    #[test]
    fn baseline_comments_and_blanks_are_skipped() {
        let b = parse_baseline("# a comment\n\nfill 80.0\n");
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn only_selects_by_name_in_suite_order() {
        let chosen = select(&Some("taps-9, fill-1080p".into()), SCENARIOS)
            .into_iter()
            .map(|s| s.name)
            .collect::<Vec<_>>();
        assert_eq!(chosen, ["fill-1080p", "taps-9"]);
    }

    #[test]
    fn scatter_stays_inside_the_window() {
        for i in 0..64 {
            let [x, y] = scatter(i);
            assert!(x.abs() < W / 2.0 && y.abs() < H / 2.0);
        }
    }
}
