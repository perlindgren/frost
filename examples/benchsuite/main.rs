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
//! * `fill-1080p`        — the floor: a lit screen, an empty field.
//! * `lamps-4/10/40`     — the light loop and its falloff, no shadows.
//! * `occluders-10/100`  — the shadow-ray walk, ten lamps over it.
//! * `taps-1/4/9`        — the penumbra ladder and its divergence.
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
//! Every run also prints a **noise floor** line — how late a wake landed
//! and how long a fixed quantum of arithmetic took, sampled throughout
//! (`tools/noisefloor`) — and a run whose floor is too loud prints its
//! numbers and exits **3** without judging or saving a baseline. A machine
//! that is busy being something else is allowed to report, not to conclude.

use std::path::PathBuf;

use frost::{
    Color, Config, Context, Light, Process, Scene, SceneNode, Shape, Transform, run_configured,
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
    },
    Scenario {
        name: "lamps-4",
        lamps: 4,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
    },
    Scenario {
        name: "lamps-10",
        lamps: 10,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
    },
    Scenario {
        name: "lamps-40",
        lamps: 40,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: true,
    },
    Scenario {
        name: "lamps-40-static",
        lamps: 40,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
    },
    Scenario {
        name: "occluders-10",
        lamps: 10,
        occluders: 10,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
    },
    Scenario {
        name: "occluders-100",
        lamps: 10,
        occluders: 100,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
    },
    Scenario {
        name: "taps-1",
        lamps: 10,
        occluders: 60,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
    },
    Scenario {
        name: "taps-4",
        lamps: 10,
        occluders: 60,
        penumbra: LAMP_PENUMBRA,
        taps: 4,
        reach: LAMP_REACH,
        moving: false,
    },
    Scenario {
        name: "taps-9",
        lamps: 10,
        occluders: 60,
        penumbra: LAMP_PENUMBRA,
        taps: 9,
        reach: LAMP_REACH,
        moving: false,
    },
];

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
            ctx.occluder(Transform::translate([x, y]), [0.0, 0.0], [90.0, 60.0]);
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
}

fn main() {
    env_logger::init();
    let cli: Cli = clap::Parser::parse();
    let chosen = select(&cli.only, SCENARIOS);
    if chosen.is_empty() {
        eprintln!("benchsuite: --only matched no scenario");
        std::process::exit(2);
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
