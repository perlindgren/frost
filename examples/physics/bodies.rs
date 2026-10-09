//! Three elastic bodies in a rattlebox: the TODO that asked for a `Body`
//! type, drawn. Four rigid walls fifty pixels thick stand at the top,
//! bottom, left and right; three rectangular bodies take random,
//! non-overlapping positions near the center and random velocities —
//! each axis fifty to a hundred pixels a second, in a random direction —
//! and from there the whole picture is [`frost::resolve_pair`] and
//! [`frost::resolve_static`] doing the only physics here: every contact
//! trades momentum and spends no energy.
//!
//! That claim is on the screen, not just in the docs: the readout at the
//! top left is the system's total kinetic energy, and it does not move.
//! Watch it across the moment two bodies meet — the velocities change
//! visibly, the sum does not. The momentum readout under it *does*
//! move: a wall is an outside force, and it takes momentum (never
//! energy) out of the body that hits it.
//!
//! Each body carries a tick from its center along its velocity — the
//! direction is easier to watch than the numbers. The frame-rate overlay
//! rides on top (Alt-0 hides it, Alt-1..Alt-4 the charts, Alt-T the
//! text): the frame-time and processing-time charts are there to show
//! whether any visible jitter has a processing spike underneath it. `L`
//! hides and shows the readout lines, live — an A/B for whether those
//! text draws ride the cadence or disturb it. The three weights are
//! deliberately different (1, 2 and 4): the heavy one shoves and barely
//! deflects, the light one gets flung.
//!
//! Run with:
//!
//! ```text
//! cargo run --example bodies
//! ```
//!
//! With `FROST_NO_VSYNC=1` the frame rate runs uncapped — the control
//! for asking whether a visible hitch belongs to the display's cadence
//! (which vanishes without vsync) or to the frame's own work (which
//! does not).

/// The walls' thickness, in pixels, as the TODO asked for them.
const WALL: f32 = 50.0;

/// How many bodies rattle in the box.
const BODIES: usize = 3;

/// The slowest and fastest any velocity axis may start at, in pixels
/// per second — the TODO's fifty to a hundred.
const SPEED_MIN: f32 = 50.0;
const SPEED_MAX: f32 = 100.0;

/// The bodies' half-sizes, one per body, deliberately unlike each
/// other so a hit is legible: wide, square, and long.
const HALVES: [[f32; 2]; BODIES] = [[26.0, 18.0], [20.0, 20.0], [30.0, 14.0]];

/// The bodies' weights: 1, 2 and 4, so the shoving has a pecking order.
const WEIGHTS: [f32; BODIES] = [1.0, 2.0, 4.0];

/// The bodies' colors: ember, mint and periwinkle against the dark.
const COLORS: [frost::Color; BODIES] = [
    frost::Color {
        r: 0.95,
        g: 0.62,
        b: 0.30,
        a: 1.0,
    },
    frost::Color {
        r: 0.55,
        g: 0.85,
        b: 0.60,
        a: 1.0,
    },
    frost::Color {
        r: 0.60,
        g: 0.66,
        b: 0.95,
        a: 1.0,
    },
];

/// The walls' grey: present, and no competition for the bodies.
const GREY: frost::Color = frost::Color {
    r: 0.16,
    g: 0.16,
    b: 0.20,
    a: 1.0,
};

/// The readouts' ink.
const INK: frost::Color = frost::Color {
    r: 0.85,
    g: 0.85,
    b: 0.90,
    a: 1.0,
};

/// The readouts' type: size, and the font loaded once at compile time.
const TEXT_SIZE: f32 = 16.0;
fn font() -> std::sync::Arc<[u8]> {
    std::sync::Arc::from(&include_bytes!("../../assets/fonts/JameGem08_2026-Regular.ttf")[..])
}

/// The four walls of a window: centers placed so each wall's inner
/// face sits one `WALL` in from the window edge, leaving the frame's
/// outer band for the grey the player sees.
fn walls(w: f32, h: f32) -> [frost::Collider; 4] {
    [
        frost::Collider::Box(frost::OrientedBox::new(
            [0.0, h / 2.0 - WALL / 2.0],
            [w / 2.0, WALL / 2.0],
        )),
        frost::Collider::Box(frost::OrientedBox::new(
            [0.0, -h / 2.0 + WALL / 2.0],
            [w / 2.0, WALL / 2.0],
        )),
        frost::Collider::Box(frost::OrientedBox::new(
            [-w / 2.0 + WALL / 2.0, 0.0],
            [WALL / 2.0, h / 2.0],
        )),
        frost::Collider::Box(frost::OrientedBox::new(
            [w / 2.0 - WALL / 2.0, 0.0],
            [WALL / 2.0, h / 2.0],
        )),
    ]
}

/// One velocity axis, drawn the way the TODO asked: fifty to a hundred
/// pixels a second, in a direction the rng picks.
fn axis(rng: &mut frost::Rng) -> f32 {
    let speed = rng.in_range(SPEED_MIN, SPEED_MAX);
    if rng.next_f32() < 0.5 { -speed } else { speed }
}

/// The three bodies at random non-overlapping positions near the
/// center: a candidate is rejected if it lands on a body already
/// placed, or crosses the inner face of a wall, or comes too near one
/// — a body born touching a wall is born with a bounce it did not earn.
fn scatter(rng: &mut frost::Rng, w: f32, h: f32) -> Vec<frost::Body> {
    let mut bodies: Vec<frost::Body> = Vec::with_capacity(BODIES);
    for i in 0..BODIES {
        let half = HALVES[i];
        let edge = [
            w / 2.0 - WALL - half[0] - 4.0,
            h / 2.0 - WALL - half[1] - 4.0,
        ];
        loop {
            let center = [
                rng.in_range(-edge[0], edge[0]),
                rng.in_range(-edge[1], edge[1]),
            ];
            let shape = frost::Collider::Box(frost::OrientedBox::new(center, half));
            if bodies.iter().any(|b| shape.intersects(&b.collider)) {
                continue;
            }
            bodies.push(frost::Body::new(shape, [axis(rng), axis(rng)], WEIGHTS[i]));
            break;
        }
    }
    bodies
}

/// The bench: the bodies, the font for the readouts, and whether the
/// first frame has laid the box out yet.
struct Rattle {
    bodies: Vec<frost::Body>,
    /// The frame-rate overlay: fps and frame time with their ten-second
    /// strip charts, the last frame's total processing time split into
    /// the demo's share and the overlay's own, and the draw-call count
    /// — the instrument for asking whether the motion jitter lives in
    /// the process's own time.
    diag: frost::Diagnostics,
    font: std::sync::Arc<[u8]>,
    laid_out: bool,
    // The readouts tick four times a second into these strings: a
    // momentum figure rewritten 120 times a second shimmers like a
    // broken gauge, and the eye reads that flicker as stuttering.
    readout: [String; 3],
    readout_t: f32,
    // The last three hundred raw `dt`s, for the cadence line: the
    // overlay draws the frame-time story as a chart; this states it as
    // numbers — typical, worst, and the share of frames the display
    // delivered one vsync late.
    ft: std::collections::VecDeque<f32>,
    // `L` hides the two readout lines, live, for A/B-ing whether the
    // per-frame text draws are in any visible way part of the frame
    // cadence; the overlay charts keep recording while the lines are
    // away. The last frame's `L` state, for edge detection.
    ledger: bool,
    was_l: bool,
}

impl frost::Process for Rattle {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // The overlay reports the truth about the frames, so it gets the
        // raw `dt`; the simulation clamps its own below.
        self.diag.process(ctx, dt);

        // The cadence sample, before any clamping: this measures the
        // delivery, not the simulation.
        self.ft.push_back(dt);
        if self.ft.len() > 300 {
            self.ft.pop_front();
        }

        // `L` toggles the ledger lines.
        let l = ctx.key_down(frost::KeyCode::KeyL);
        if l && !self.was_l {
            self.ledger = !self.ledger;
        }
        self.was_l = l;

        // The engine clamps `dt` to a second, but a dragged window or a
        // stall hands the simulation a whole second at once: every body
        // would leap ~100 pixels, tunnel, and get yanked back — a warp
        // that looks like a frame-rate catastrophe. The neighbour
        // `collision` demo clamps the same way: a hitch costs a few slow
        // frames, never a teleport.
        let dt = dt.min(1.0 / 30.0);
        let (w, h) = ctx.size();
        if !self.laid_out {
            let mut rng = frost::Rng::new();
            self.bodies = scatter(&mut rng, w, h);
            self.laid_out = true;
        }
        let walls = walls(w, h);

        // The frame's rhythm: move every body by its velocity, then
        // settle the contacts — every pair, then every body against
        // every wall. Order within a frame is a choice, not a law:
        // each contact is exact on its own, and one pass is the whole
        // simulation this bench needs.
        for body in self.bodies.iter_mut() {
            body.advance(dt);
        }
        for i in 0..self.bodies.len() {
            for j in (i + 1)..self.bodies.len() {
                let (head, tail) = self.bodies.split_at_mut(j);
                frost::resolve_pair(&mut head[i], &mut tail[0]);
            }
            for wall in &walls {
                frost::resolve_static(&mut self.bodies[i], wall);
            }
        }
        // A window shrunk out from under a body: stand it back inside
        // the inner box — inside the *walls*, so its half-size counts
        // and the clamp never parks a body straddling a wall face.
        for body in self.bodies.iter_mut() {
            let frost::Collider::Box(b) = &body.collider else {
                continue;
            };
            let (edge_x, edge_y) = (
                (w / 2.0 - WALL - b.half[0]).max(0.0),
                (h / 2.0 - WALL - b.half[1]).max(0.0),
            );
            let c = body.center();
            let (cx, cy) = (c[0].clamp(-edge_x, edge_x), c[1].clamp(-edge_y, edge_y));
            body.collider.translate([cx - c[0], cy - c[1]]);
        }

        // The walls, in the frame's outer band.
        for wall in &walls {
            let frost::Collider::Box(b) = wall else {
                continue;
            };
            ctx.rectangle(b.center[0], b.center[1], b.half[0], b.half[1], GREY, 0.0);
        }

        // The bodies and their heading ticks.
        for (i, body) in self.bodies.iter().enumerate() {
            let frost::Collider::Box(b) = &body.collider else {
                continue;
            };
            ctx.rectangle(
                b.center[0],
                b.center[1],
                b.half[0],
                b.half[1],
                COLORS[i],
                1.0,
            );
            let speed = (body.velocity[0].hypot(body.velocity[1])).max(1.0);
            let tip = [
                b.center[0] + body.velocity[0] / speed * (b.half[0].max(b.half[1]) + 8.0),
                b.center[1] + body.velocity[1] / speed * (b.half[0].max(b.half[1]) + 8.0),
            ];
            ctx.line(
                b.center[0],
                b.center[1],
                tip[0],
                tip[1],
                COLORS[i],
                2.0,
                2.0,
            );
        }

        // The ledger, read out: energy flat, momentum traded to the
        // walls — refreshed on a quarter-second tick, not every frame.
        self.readout_t += dt;
        if self.readout[0].is_empty() || self.readout_t >= 0.25 {
            self.readout_t = 0.0;
            let p = frost::momentum(&self.bodies);
            let mut d: Vec<f32> = self.ft.iter().copied().collect();
            d.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let (med, p99, max) = (d[d.len() / 2], d[d.len() * 99 / 100], d[d.len() - 1]);
            let late = d.iter().filter(|x| **x > med * 1.5).count();
            self.readout = [
                format!("energy  {:.0}", frost::kinetic_energy(&self.bodies)),
                format!("momentum  {:.0} {:.0}", p[0], p[1]),
                // Whole milliseconds only: the font has no decimal
                // point, and a cadence readout earns none.
                format!(
                    "ft med {:.0} p99 {:.0} max {:.0} late {:.0}%",
                    med * 1000.0,
                    p99 * 1000.0,
                    max * 1000.0,
                    100.0 * late as f32 / d.len() as f32
                ),
            ];
        }
        // The top-right corner: the overlay owns the top-left. Hidden
        // while `L` has the ledger away.
        if self.ledger {
            // Far enough from the edge that even the ft line — the
            // longest of the three — stays inside the wall.
            let right = w / 2.0 - WALL - 160.0;
            for (line, text) in self.readout.iter().enumerate() {
                ctx.text(
                    right,
                    h / 2.0 - WALL - 14.0 - line as f32 * 20.0,
                    &self.font,
                    text.clone(),
                    TEXT_SIZE,
                    400.0,
                    INK,
                    3.0,
                );
            }
        }
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    let root = std::env!("CARGO_MANIFEST_DIR");
    let diag = match frost::Diagnostics::new(
        format!("{root}/assets/fonts/FiraCode-VariableFont_wght.ttf"),
        frost::DiagnosticsFlags::all(),
    ) {
        Ok(diagnostics) => diagnostics,
        Err(err) => {
            log::error!("failed to load the font: {err}");
            std::process::exit(1);
        }
    };

    let scene = frost::Scene::new(frost::SceneNode {
        // The dark: the three colors read bright against it, and the
        // grey walls sit just shy of the frame.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.03,
                g: 0.03,
                b: 0.04,
                a: 1.0,
            },
        }),
        ..Default::default()
    });

    let mut config = frost::Config::default();
    if std::env::var("FROST_NO_VSYNC").is_ok() {
        config.vsync = false;
    }
    if let Err(err) = frost::run_configured(
        scene,
        Rattle {
            bodies: Vec::new(),
            diag,
            font: font(),
            laid_out: false,
            readout: [String::new(), String::new(), String::new()],
            readout_t: 0.0,
            ft: std::collections::VecDeque::with_capacity(300),
            ledger: true,
            was_l: false,
        },
        config,
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
