//! Ember Hill — a jam puzzle game on FROST (theme: *autumn* / *to keep warm*).
//!
//! Shepherd a growing trail of ants through a side-view ant hill. Walking
//! onto a stranded ant recruits it to your trail's tail; only the tail can
//! be released, and it only settles on its own colored bed — so the trail
//! is a stack, and the tunnels are the shuffle device. Every ant asleep
//! in its bed, yourself included, before the first frost.
//!
//! Run: `RUST_LOG=info cargo run --example antifrost`
//! Rules: `examples/antifrost/DESIGN.md`

mod game;
mod level;
#[cfg(test)]
mod solver;

use frost::{
    Color, Context, KeyCode, Particle, ParticleShape, ParticleSystem, Process, Repeat, Rng, Scene,
    SceneNode, Tween,
};
use std::collections::HashMap;
use std::sync::Arc;

const FONT: &[u8] = include_bytes!("../../assets/fonts/FiraCode-VariableFont_wght.ttf");
const LEVELS: &str = include_str!("levels.ron");

const TILE: f32 = 44.0;
const HOP: f32 = 0.11; // seconds, player pixel glide
const TRAIL_HOP: f32 = 0.14; // ants glide slightly lazier
const UI: f32 = 1_000_000.0;

const TAU: f32 = std::f32::consts::TAU;

/// The eleven leaf sprites — each sheet is one instanced particle batch.
const LEAVES: [&[u8]; 11] = [
    include_bytes!("assets/sprites/leaf1.png"),
    include_bytes!("assets/sprites/leaf2.png"),
    include_bytes!("assets/sprites/leaf3.png"),
    include_bytes!("assets/sprites/leaf4.png"),
    include_bytes!("assets/sprites/leaf5.png"),
    include_bytes!("assets/sprites/leaf6.png"),
    include_bytes!("assets/sprites/leaf7.png"),
    include_bytes!("assets/sprites/leaf8.png"),
    include_bytes!("assets/sprites/leaf9.png"),
    include_bytes!("assets/sprites/leaf10.png"),
    include_bytes!("assets/sprites/leaf11.png"),
];

/// Leaves drifting over the hill at any moment. Each owns a lane, so the
/// origins stay evenly spread across the window width as they recycle.
const N_LEAVES: usize = 30;

/// Leaves draw above the ants (z ≈ 1.5) and below the frost rim and every
/// UI pixel.
const LEAF_Z: f32 = 1.8;

/// Sprite half-widths in pixels: a leaf draws 12–24 px wide, its height
/// following each PNG's own aspect.
const LEAF_SIZE: (f32, f32) = (6.0, 12.0);

/// Grounded means resting somewhere in a band of one tenth of the window
/// height above the bottom edge.
const LEAF_BAND: f32 = 10.0;

/// A settled leaf lingers, then fades over this many seconds, and is
/// re-circled above the window when it vanishes.
const LEAF_FADE: (f32, f32) = (1.6, 3.0);

const C_BG: Color = Color {
    r: 0.045,
    g: 0.038,
    b: 0.033,
    a: 1.0,
};
const C_ROCK: Color = Color {
    r: 0.235,
    g: 0.205,
    b: 0.18,
    a: 1.0,
};
const C_DIRT: Color = Color {
    r: 0.44,
    g: 0.31,
    b: 0.165,
    a: 1.0,
};
const C_TUNNEL: Color = Color {
    r: 0.105,
    g: 0.085,
    b: 0.066,
    a: 1.0,
};
const C_TEXT: Color = Color {
    r: 0.93,
    g: 0.87,
    b: 0.76,
    a: 1.0,
};
const C_DIM: Color = Color {
    r: 0.62,
    g: 0.55,
    b: 0.45,
    a: 1.0,
};
const C_NOTE: Color = Color {
    r: 1.0,
    g: 0.72,
    b: 0.35,
    a: 1.0,
};

fn ant_color(c: game::AntColor) -> Color {
    match c {
        game::AntColor::White => Color {
            r: 0.97,
            g: 0.97,
            b: 0.93,
            a: 1.0,
        },
        game::AntColor::Red => Color {
            r: 0.93,
            g: 0.30,
            b: 0.24,
            a: 1.0,
        },
        game::AntColor::Yellow => Color {
            r: 0.95,
            g: 0.78,
            b: 0.22,
            a: 1.0,
        },
        game::AntColor::Green => Color {
            r: 0.45,
            g: 0.86,
            b: 0.36,
            a: 1.0,
        },
        game::AntColor::Blue => Color {
            r: 0.36,
            g: 0.62,
            b: 0.98,
            a: 1.0,
        },
    }
}

fn shade(mut c: Color, k: f32) -> Color {
    c.r *= k;
    c.g *= k;
    c.b *= k;
    c
}

fn closer(a: [f32; 2], b: [f32; 2]) -> bool {
    (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01
}

/// Per-ant pixel glide: where it is drawn, where it is heading.
struct Glide {
    at: [f32; 2],
    target: [f32; 2],
    tween: Tween<[f32; 2]>,
}

impl Glide {
    fn steer(&mut self, to: [f32; 2], dt: f32, speed: f32) {
        if !closer(self.target, to) {
            self.tween = Tween::new(self.at, to, speed).repeat(Repeat::Once);
            self.target = to;
        }
        self.at = self.tween.tick(dt);
    }
}

/// One drifting leaf — the simulation state the Demo owns. Fluttering is
/// deliberately *not* `ParticleSystem::update`: that is a gravity
/// integrator, while a leaf sways, flips edge-on, rides the gust, lands
/// at a random point of the ground band and fades there. The engine
/// leaves `size`, `angle` and `color` to the caller for exactly this —
/// the systems only carry the finished particles into the sprite batches.
struct Leaf {
    /// Which sprite sheet draws this leaf (index into `LEAVES`).
    sprite: usize,
    /// The lane (0..`N_LEAVES`) this leaf re-enters above when recycled.
    lane: usize,
    pos: [f32; 2],
    /// Descent speed in px/s, modulated by the flip.
    fall: f32,
    /// Sidesway amplitude in px/s.
    sway: f32,
    phase: f32,
    freq: f32,
    flip_phase: f32,
    flip_freq: f32,
    tilt: f32,
    angle: f32,
    size: f32,
    /// Where in the ground band this leaf rests, as a 0..1 fraction.
    ground: f32,
    grounded: bool,
    alpha: f32,
    fade: f32,
}

impl Leaf {
    /// A fresh leaf slightly above the window, jittered inside its lane.
    fn fresh(lane: usize, w: f32, h: f32, rng: &mut Rng) -> Self {
        let lane_w = w / N_LEAVES as f32;
        Self {
            sprite: (rng.next_f32() * LEAVES.len() as f32) as usize % LEAVES.len(),
            lane,
            pos: [
                -w / 2.0 + lane_w * (lane as f32 + 0.5 + rng.in_range(-0.4, 0.4)),
                h / 2.0 + rng.in_range(24.0, 140.0),
            ],
            fall: rng.in_range(38.0, 92.0),
            sway: rng.in_range(14.0, 46.0),
            phase: rng.in_range(0.0, TAU),
            freq: rng.in_range(1.3, 2.8),
            flip_phase: rng.in_range(0.0, TAU),
            flip_freq: rng.in_range(0.7, 1.9),
            tilt: rng.in_range(-0.7, 0.7),
            angle: 0.0,
            size: rng.in_range(LEAF_SIZE.0, LEAF_SIZE.1),
            ground: rng.in_range(0.04, 1.0),
            grounded: false,
            alpha: 1.0,
            fade: rng.in_range(LEAF_FADE.0, LEAF_FADE.1),
        }
    }
}

struct Demo {
    levels: Vec<level::Level>,
    idx: usize,
    game: Option<game::Game>,
    player_at: [f32; 2],
    player_target: [f32; 2],
    player_tween: Tween<[f32; 2]>,
    face: [f32; 2],
    glides: HashMap<usize, Glide>,
    font: Arc<[u8]>,
    note: String,
    note_t: f32,
    frost_t: f32,
    frost_bit: bool,
    was: HashMap<KeyCode, bool>,
    leaf_rng: Rng,
    leaves: Vec<Leaf>,
    leaves_seeded: bool,
    leaf_t: f32,
}

impl Demo {
    fn new() -> Self {
        let levels = level::load_levels(LEVELS);
        if levels.is_empty() {
            log::error!("antifrost: no levels loaded");
        }
        let mut demo = Self {
            levels,
            idx: 0,
            game: None,
            player_at: [0.0, 0.0],
            player_target: [0.0, 0.0],
            player_tween: Tween::new([0.0, 0.0], [0.0, 0.0], HOP).repeat(Repeat::Once),
            face: [1.0, 0.0],
            glides: HashMap::new(),
            font: Arc::from(FONT),
            note: String::new(),
            note_t: 0.0,
            frost_t: 0.0,
            frost_bit: false,
            was: HashMap::new(),
            leaf_rng: Rng::default(),
            leaves: Vec::new(),
            leaves_seeded: false,
            leaf_t: 0.0,
        };
        demo.load(0);
        demo
    }

    fn load(&mut self, idx: usize) {
        if self.levels.is_empty() {
            return;
        }
        self.idx = idx % self.levels.len();
        let level = self.levels[self.idx].clone();
        let start = self.cell_px(level.player_start);
        self.game = Some(game::Game::start(level));
        self.player_at = start;
        self.player_target = start;
        self.player_tween = Tween::new(start, start, HOP).repeat(Repeat::Once);
        self.glides.clear();
        self.note.clear();
        self.note_t = 0.0;
        self.frost_t = 0.0;
        self.frost_bit = false;
    }

    fn cell_px(&self, c: game::Cell) -> [f32; 2] {
        let (cols, rows) = (
            self.levels[self.idx].cols as f32,
            self.levels[self.idx].rows as f32,
        );
        [
            (c.col as f32 - cols / 2.0 + 0.5) * TILE,
            (rows / 2.0 - c.row as f32 - 0.5) * TILE,
        ]
    }

    /// Rising-edge key test (the repo idiom: remember last frame's state).
    fn pressed(&mut self, ctx: &Context, key: KeyCode) -> bool {
        let down = ctx.key_down(key);
        let edge = down && !*self.was.entry(key).or_insert(false);
        self.was.insert(key, down);
        edge
    }

    fn refresh(&mut self, dt: f32) {
        let Some(game) = &self.game else { return };
        // Player glide.
        let want = self.cell_px(game.player);
        if !closer(self.player_target, want) {
            self.player_tween = Tween::new(self.player_at, want, HOP).repeat(Repeat::Once);
            self.player_target = want;
        }
        self.player_at = self.player_tween.tick(dt);

        // Where every ant wants to be; trail ants read the packed history.
        let mut wanted: HashMap<usize, game::Cell> = HashMap::new();
        for i in 0..game.trail.len() {
            wanted.insert(game.trail[i], game.trail_target(i));
        }
        for (id, ant) in game.ants.iter().enumerate() {
            if let game::AntState::Wild(c) | game::AntState::Placed(c) | game::AntState::Asleep(c) =
                ant.state
            {
                wanted.insert(id, c);
            }
        }
        // Ants sharing a cell fan out a few pixels so all stay visible.
        let mut seen: HashMap<game::Cell, usize> = HashMap::new();
        for id in 0..game.ants.len() {
            let Some(cell) = wanted.get(&id).copied() else {
                continue;
            };
            let k = seen.entry(cell).or_insert(0);
            let offset = (*k as f32) * TILE * 0.14;
            *k += 1;
            let base = self.cell_px(cell);
            let to = [base[0] - offset, base[1] + offset * 0.7];
            self.glides
                .entry(id)
                .or_insert_with(|| Glide {
                    at: to,
                    target: to,
                    tween: Tween::new(to, to, TRAIL_HOP).repeat(Repeat::Once),
                })
                .steer(to, dt, TRAIL_HOP);
        }
        if let Some(note) = self.game.as_mut().and_then(|g| g.take_note()) {
            self.note = note.to_string();
            self.note_t = 2.4;
        }

        // The frost: over par and the rim of the world turns cold. Soft —
        // it never stops the walk, it only remembers that winter arrived.
        let over = self
            .game
            .as_ref()
            .is_some_and(|g| g.level.par > 0 && g.moves > g.level.par);
        if over && !self.frost_bit {
            self.frost_bit = true;
            self.note = "the frost bites — keep walking".to_string();
            self.note_t = 2.4;
        }
        self.frost_t += if over { dt / 1.2 } else { -dt / 1.2 };
        self.frost_t = self.frost_t.clamp(0.0, 1.0);
    }

    /// Autumn over the hill: thirty leaves flutter down, land somewhere
    /// in the ground band, fade into the litter, and re-enter above the
    /// window. One step per leaf, then the eleven sprite batches are
    /// rebuilt whole — with thirty particles, incremental bookkeeping
    /// would only be a place to hide bugs.
    fn update_leaves(&mut self, ctx: &mut Context, dt: f32) {
        let (w, h) = ctx.size();
        if !self.leaves_seeded {
            self.leaves_seeded = true;
            self.leaves = (0..N_LEAVES)
                .map(|lane| Leaf::fresh(lane, w, h, &mut self.leaf_rng))
                .collect();
            // Scatter the first volley through the air, so the hill
            // opens already in autumn rather than waiting for a fall.
            for leaf in &mut self.leaves {
                leaf.pos[1] = self.leaf_rng.in_range(-h / 2.0 + 20.0, h / 2.0 + 60.0);
            }
        }
        self.leaf_t += dt;
        // A slow shared gust all the leaves hang from.
        let wind = 16.0 * (0.11 * self.leaf_t).sin() + 7.0 * (0.37 * self.leaf_t + 2.1).sin();
        let mut batches = vec![ParticleSystem::default(); LEAVES.len()];
        for leaf in &mut self.leaves {
            let ground_y = -h / 2.0 + leaf.ground * h / LEAF_BAND;
            // Fully faded: the leaf is gone into the litter and
            // re-circles above the window.
            if leaf.grounded && leaf.alpha <= 0.0 {
                *leaf = Leaf::fresh(leaf.lane, w, h, &mut self.leaf_rng);
            }
            let vel;
            let size;
            if leaf.grounded {
                // Settled: it rests, creeps a breath on the wind, and
                // fades out over a few seconds.
                leaf.alpha -= dt / leaf.fade;
                leaf.pos[0] += wind * 0.2 * dt;
                vel = [wind * 0.2, 0.0];
                size = leaf.size;
            } else {
                leaf.phase += leaf.freq * dt;
                leaf.flip_phase += leaf.flip_freq * dt;
                let flip = leaf.flip_phase.sin();
                // Edge-on, a leaf falls faster and draws narrower — the
                // flutter lives in that pairing.
                let vx = leaf.sway * leaf.phase.cos() + wind;
                let vy = -leaf.fall * (0.62 + 0.38 * flip.abs());
                leaf.pos[0] += vx * dt;
                leaf.pos[1] += vy * dt;
                leaf.angle = leaf.tilt + 0.55 * leaf.phase.cos() + 0.3 * leaf.flip_phase.cos();
                vel = [vx, vy];
                size = leaf.size * (0.55 + 0.45 * flip.abs());
                // The gust may carry a leaf across the whole width.
                let m = 40.0;
                if leaf.pos[0] > w / 2.0 + m {
                    leaf.pos[0] -= w + 2.0 * m;
                } else if leaf.pos[0] < -w / 2.0 - m {
                    leaf.pos[0] += w + 2.0 * m;
                }
                if leaf.pos[1] <= ground_y {
                    leaf.pos[1] = ground_y;
                    leaf.grounded = true;
                    leaf.angle = leaf.tilt * 0.3;
                }
            }
            batches[leaf.sprite].spawn(Particle {
                pos: leaf.pos,
                vel,
                // Life-fade is the caller's: the leaf's alpha is its own,
                // and nothing here dies of old age.
                life: 1.0,
                max_life: 1.0,
                size,
                angle: leaf.angle,
                color: Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: leaf.alpha.clamp(0.0, 1.0),
                },
            });
        }
        for (node, batch) in ctx.scene().root.children.iter_mut().zip(batches) {
            if let Some(frost::Shape::Particles { system, .. }) = node.shape.as_mut() {
                *system = batch;
            }
        }
    }

    // ------------------------------------------------------------- draw

    fn draw_board(&self, ctx: &mut Context) {
        let Some(game) = &self.game else { return };
        for row in 0..game.level.rows as i32 {
            for col in 0..game.level.cols as i32 {
                let cell = game::Cell::new(col, row);
                let [x, y] = self.cell_px(cell);
                let noise = 1.0 + ((col * 31 + row * 17) % 3) as f32 * 0.045;
                let half = TILE / 2.0 - 0.75;
                match game.tile(cell) {
                    level::Tile::Rock => ctx.rectangle(x, y, half, half, shade(C_ROCK, noise), 0.0),
                    level::Tile::Dirt => ctx.rectangle(x, y, half, half, shade(C_DIRT, noise), 0.0),
                    level::Tile::Tunnel => ctx.rectangle(x, y, half, half, C_TUNNEL, 0.0),
                    level::Tile::Bed(color) => {
                        ctx.rectangle(x, y, half, half, C_TUNNEL, 0.0);
                        let c = ant_color(color);
                        ctx.rectangle(
                            x,
                            y - TILE * 0.13,
                            TILE * 0.36,
                            TILE * 0.17,
                            shade(c, 0.75),
                            0.05,
                        );
                        ctx.rectangle(
                            x,
                            y - TILE * 0.13,
                            TILE * 0.30,
                            TILE * 0.12,
                            shade(c, 0.55),
                            0.06,
                        );
                        ctx.rectangle(
                            x - TILE * 0.14,
                            y - TILE * 0.02,
                            TILE * 0.10,
                            TILE * 0.06,
                            shade(c, 1.3),
                            0.07,
                        );
                    }
                }
            }
        }
    }

    fn draw_ant(
        &self,
        ctx: &mut Context,
        at: [f32; 2],
        color: Color,
        facing: [f32; 2],
        asleep: bool,
        player: bool,
    ) {
        let (x, y) = (at[0], at[1]);
        let z = if player { 1.5 } else { 1.0 + y * 1e-4 };
        if asleep {
            ctx.circle(x, y - TILE * 0.12, TILE * 0.20, shade(color, 0.8), z);
            ctx.circle(
                x + TILE * 0.12,
                y - TILE * 0.08,
                TILE * 0.11,
                shade(color, 0.65),
                z + 0.01,
            );
            ctx.text(
                x + TILE * 0.22,
                y + TILE * 0.24,
                &self.font,
                "z",
                12.0,
                700.0,
                C_DIM,
                z + 0.02,
            );
            return;
        }
        let body = TILE * 0.22;
        let (fx, fy) = (facing[0], facing[1]);
        let hx = x + fx * TILE * 0.20;
        let hy = y + fy * TILE * 0.20;
        if player {
            ctx.circle(
                x,
                y,
                body * 1.30,
                Color {
                    r: 0.08,
                    g: 0.06,
                    b: 0.05,
                    a: 1.0,
                },
                1.48,
            );
        }
        ctx.circle(
            x - fx * TILE * 0.13,
            y - fy * TILE * 0.13,
            body * 0.86,
            shade(color, 0.85),
            z + 0.01,
        );
        ctx.circle(x, y, body, color, z + 0.02);
        ctx.circle(hx, hy, TILE * 0.125, shade(color, 1.10), z + 0.03);
        let ax = hx + fx * TILE * 0.06;
        let ay = hy + fy * TILE * 0.06;
        ctx.line(
            ax,
            ay,
            ax + fx * 5.0 + fy * 5.0,
            ay + fy * 5.0 + fx * 5.0,
            shade(color, 0.8),
            1.5,
            z + 0.04,
        );
        ctx.line(
            ax,
            ay,
            ax + fx * 5.0 - fy * 5.0,
            ay + fy * 5.0 - fx * 5.0,
            shade(color, 0.8),
            1.5,
            z + 0.04,
        );
    }

    fn draw_ants(&mut self, ctx: &mut Context) {
        let Some(game) = &self.game else { return };
        // Chain link line: player → head → … → tail.
        if !game.trail.is_empty() {
            let mut pts: Vec<[f32; 2]> = vec![self.player_at];
            for i in 0..game.trail.len() {
                if let Some(g) = self.glides.get(&game.trail[i]) {
                    pts.push(g.at);
                }
            }
            for w in pts.windows(2) {
                ctx.line(
                    w[0][0],
                    w[0][1],
                    w[1][0],
                    w[1][1],
                    Color {
                        r: 0.55,
                        g: 0.44,
                        b: 0.32,
                        a: 0.65,
                    },
                    2.0,
                    0.6,
                );
            }
        }
        // Facing for trail ants: toward whoever they follow.
        let mut facing: HashMap<usize, [f32; 2]> = HashMap::new();
        for i in 0..game.trail.len() {
            let id = game.trail[i];
            let ahead = if i == 0 {
                self.player_at
            } else {
                self.glides
                    .get(&game.trail[i - 1])
                    .map(|g| g.at)
                    .unwrap_or([0.0, 0.0])
            };
            let here = self.glides.get(&id).map(|g| g.at).unwrap_or([0.0, 0.0]);
            let (dx, dy) = (ahead[0] - here[0], ahead[1] - here[1]);
            let f = if dx.abs() >= dy.abs() {
                [dx.signum(), 0.0]
            } else {
                [0.0, dy.signum()]
            };
            facing.insert(id, if f == [0.0, 0.0] { [-1.0, 0.0] } else { f });
        }
        let mut order: Vec<usize> = (0..game.ants.len()).collect();
        order.sort_by(|a, b| {
            let ya = self.glides.get(a).map(|g| g.at[1]).unwrap_or(0.0);
            let yb = self.glides.get(b).map(|g| g.at[1]).unwrap_or(0.0);
            ya.partial_cmp(&yb).unwrap_or(std::cmp::Ordering::Equal)
        });
        for id in order {
            let Some(g) = self.glides.get(&id) else {
                continue;
            };
            let at = g.at;
            let asleep = matches!(game.ants[id].state, game::AntState::Asleep(_));
            let face = *facing.get(&id).unwrap_or(&[-1.0, 0.0]);
            let col = ant_color(game.ants[id].color);
            let col = if matches!(game.ants[id].state, game::AntState::Wild(_)) {
                shade(col, 0.88)
            } else {
                col
            };
            self.draw_ant(ctx, at, col, face, asleep, false);
        }
        self.draw_ant(
            ctx,
            self.player_at,
            ant_color(game::AntColor::White),
            self.face,
            false,
            true,
        );
    }

    /// A rim of ice creeping in from the edges once par is gone. Two
    /// nested bands of translucent blue, strong outside, dissolving
    /// inward; sits under every UI pixel on purpose.
    fn draw_frost(&self, ctx: &mut Context) {
        let f = self.frost_t * self.frost_t;
        if f <= 0.01 {
            return;
        }
        let (w, h) = ctx.size();
        let ice = |a: f32| Color {
            r: 0.66,
            g: 0.80,
            b: 0.94,
            a: a * f,
        };
        let band = (w.min(h) * 0.09).max(40.0);
        for (depth, tint) in [(band, ice(0.10)), (band * 0.45, ice(0.14))] {
            ctx.rectangle(
                0.0,
                h / 2.0 - depth / 2.0,
                w / 2.0,
                depth / 2.0,
                tint,
                UI - 1.0,
            );
            ctx.rectangle(
                0.0,
                -h / 2.0 + depth / 2.0,
                w / 2.0,
                depth / 2.0,
                tint,
                UI - 1.0,
            );
            ctx.rectangle(
                -w / 2.0 + depth / 2.0,
                0.0,
                depth / 2.0,
                h / 2.0,
                tint,
                UI - 1.0,
            );
            ctx.rectangle(
                w / 2.0 - depth / 2.0,
                0.0,
                depth / 2.0,
                h / 2.0,
                tint,
                UI - 1.0,
            );
        }
    }

    fn draw_hud(&self, ctx: &mut Context) {
        let Some(game) = &self.game else { return };
        let (w, h) = ctx.size();
        let (w, h) = (w, h);
        let lx = -w / 2.0 + 18.0;
        let rx = w / 2.0 - 190.0;
        let top = h / 2.0 - 26.0;
        ctx.text(
            lx,
            top,
            &self.font,
            format!(
                "{}  ·  hill {}/{}",
                game.level.name,
                self.idx + 1,
                self.levels.len()
            ),
            17.0,
            700.0,
            C_TEXT,
            UI,
        );
        ctx.text(
            rx,
            top,
            &self.font,
            format!(
                "warm {}/{}   digs {}   moves {}/{}",
                game.ants_asleep(),
                game.ants.len(),
                game.digs_left,
                game.moves,
                game.level.par
            ),
            15.0,
            400.0,
            C_TEXT,
            UI,
        );
        ctx.text(
            0.0,
            -h / 2.0 + 20.0,
            &self.font,
            "arrows move · collect · dig      shift+arrow push      space leave the last ant      Z undo   R restart   N/P hills",
            12.0,
            400.0,
            C_DIM,
            UI,
        );
        if self.note_t > 0.0 {
            let a = (self.note_t / 0.6).min(1.0);
            ctx.text(
                0.0,
                -h / 2.0 + 64.0,
                &self.font,
                self.note.clone(),
                15.0,
                600.0,
                Color {
                    r: C_NOTE.r,
                    g: C_NOTE.g,
                    b: C_NOTE.b,
                    a,
                },
                UI + 1.0,
            );
        }
        if game.won {
            ctx.text(
                0.0,
                40.0,
                &self.font,
                "every ant is warm — winter may come",
                26.0,
                700.0,
                C_TEXT,
                UI + 2.0,
            );
            ctx.text(
                0.0,
                4.0,
                &self.font,
                format!(
                    "{} moves · par {}{}",
                    game.moves,
                    game.level.par,
                    if self.frost_bit {
                        " · the frost bit"
                    } else {
                        ""
                    }
                ),
                15.0,
                400.0,
                C_DIM,
                UI + 2.0,
            );
            ctx.text(
                0.0,
                -26.0,
                &self.font,
                "N — the next hill",
                15.0,
                400.0,
                C_NOTE,
                UI + 2.0,
            );
        }
    }
}

impl Process for Demo {
    fn process(&mut self, ctx: &mut Context, dt: f32) {
        if self.game.is_some() {
            let shift = ctx.key_down(KeyCode::ShiftLeft) || ctx.key_down(KeyCode::ShiftRight);
            let mut action: Option<(game::Dir, bool)> = None;
            for (key, dir) in [
                (KeyCode::ArrowLeft, game::Dir::Left),
                (KeyCode::KeyA, game::Dir::Left),
                (KeyCode::ArrowRight, game::Dir::Right),
                (KeyCode::KeyD, game::Dir::Right),
                (KeyCode::ArrowUp, game::Dir::Up),
                (KeyCode::KeyW, game::Dir::Up),
                (KeyCode::ArrowDown, game::Dir::Down),
                (KeyCode::KeyS, game::Dir::Down),
            ] {
                if self.pressed(ctx, key) {
                    action = Some((dir, shift));
                }
            }
            let detach = self.pressed(ctx, KeyCode::Space);
            let undo = self.pressed(ctx, KeyCode::KeyZ);
            let restart = self.pressed(ctx, KeyCode::KeyR);
            let next = self.pressed(ctx, KeyCode::KeyN);
            let prev = self.pressed(ctx, KeyCode::KeyP);
            if next {
                self.load(self.idx + 1);
            }
            if prev {
                self.load(self.idx + self.levels.len().saturating_sub(1));
            }
            if let Some(game) = &mut self.game {
                if restart {
                    let level = game.level.clone();
                    *game = game::Game::start(level);
                } else if undo {
                    game.undo();
                } else if detach && !game.won {
                    let _ = game.detach();
                } else if let Some((dir, push)) = action.filter(|_| !game.won) {
                    self.face = match dir {
                        game::Dir::Left => [-1.0, 0.0],
                        game::Dir::Right => [1.0, 0.0],
                        game::Dir::Up => [0.0, 1.0],
                        game::Dir::Down => [0.0, -1.0],
                    };
                    if push {
                        let _ = game.push(dir);
                    } else {
                        let _ = game.move_player(dir);
                    }
                }
            }
        }
        self.note_t = (self.note_t - dt).max(0.0);
        self.refresh(dt);

        self.update_leaves(ctx, dt);
        self.draw_board(ctx);
        self.draw_ants(ctx);
        self.draw_frost(ctx);
        self.draw_hud(ctx);
    }
}

fn main() {
    env_logger::init();
    // One particle batch per leaf sheet, riding the identity transform so
    // the batches live in the window's user space, like every immediate
    // draw. A leaf PNG that fails to decode is a broken build: say so at
    // once, not at the first frame.
    let leaf_batches: Vec<Box<SceneNode>> = LEAVES
        .iter()
        .map(|png| {
            ParticleShape::sprite_bytes(*png).unwrap_or_else(|err| {
                log::error!("antifrost: bad leaf sprite: {err}");
                std::process::exit(1);
            })
        })
        .map(|shape| {
            Box::new(SceneNode {
                shape: Some(frost::Shape::Particles {
                    system: ParticleSystem::default(),
                    color: Color {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                    shape,
                }),
                order: LEAF_Z,
                ..Default::default()
            })
        })
        .collect();
    let scene = Scene::new(SceneNode {
        shape: Some(frost::Shape::Background { color: C_BG }),
        children: leaf_batches,
        ..Default::default()
    });
    if let Err(err) = frost::run_configured(
        scene,
        Demo::new(),
        frost::Config {
            window_size: Some([900, 660]),
            ..Default::default()
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
