//! The hill as it breathes: the animation vocabulary the walkers
//! are drawn with — hops, glides, sleep, splashes — the autumn
//! weather that drifts past the window, the ants and their lamps,
//! their shadows and their flares, and the verdict that bursts
//! over the hill when the last ant is home.

use crate::*;

/// Per-ant pixel glide: where it is drawn, where it is heading.
/// One hop's path: a cubic between two cells whose control arms lie
/// along the directions of travel at either end. Entering and leaving
/// the same way, the arms fall on the line and the hop is straight;
/// at a corner they stand out and the hop bends into a quarter
/// circle — position and facing curve together, because both read the
/// same curve.
#[derive(Clone, Copy)]
pub(crate) struct Hop {
    pub(crate) a: [f32; 2],
    pub(crate) b: [f32; 2],
    pub(crate) c: [f32; 2],
    pub(crate) d: [f32; 2],
    pub(crate) t: f32,
    pub(crate) dur: f32,
}

impl Hop {
    /// A finished hop parked at `p`, going nowhere — for snaps and spawns.
    pub(crate) fn flat(p: [f32; 2], dur: f32) -> Self {
        Self {
            a: p,
            b: p,
            c: p,
            d: p,
            t: 1.0,
            dur,
        }
    }
    /// Re-armed from `from` to `to`, entering along `entry` and leaving
    /// along `exit` (unit axes): perpendicular, the arms bend the
    /// corner; aligned, the arms lie flat and it is a straight line.
    pub(crate) fn to(
        &mut self,
        from: [f32; 2],
        to: [f32; 2],
        entry: [f32; 2],
        exit: [f32; 2],
        dur: f32,
    ) {
        let dot = entry[0] * exit[0] + entry[1] * exit[1];
        let k = if dot.abs() < 0.5 {
            HOP_ARC_K * TILE
        } else {
            0.0
        };
        *self = Self {
            a: from,
            b: [from[0] + entry[0] * k, from[1] + entry[1] * k],
            c: [to[0] - exit[0] * k, to[1] - exit[1] * k],
            d: to,
            t: 0.0,
            dur,
        };
    }
    pub(crate) fn tick(&mut self, dt: f32) -> [f32; 2] {
        self.t = (self.t + dt / self.dur).min(1.0);
        let t = self.t;
        let u = 1.0 - t;
        let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
        [
            w[0] * self.a[0] + w[1] * self.b[0] + w[2] * self.c[0] + w[3] * self.d[0],
            w[0] * self.a[1] + w[1] * self.b[1] + w[2] * self.c[1] + w[3] * self.d[1],
        ]
    }
}

/// Turns `from` toward `to` by at most `max` radians.
pub(crate) fn turn_toward(from: [f32; 2], to: [f32; 2], max: f32) -> [f32; 2] {
    let a = (from[0] * to[1] - from[1] * to[0])
        .atan2(from[0] * to[0] + from[1] * to[1])
        .clamp(-max, max);
    let (s, c) = a.sin_cos();
    [from[0] * c - from[1] * s, from[0] * s + from[1] * c]
}

/// One z off a sleeper's breath: it drifts up, wanders a little on the
/// way, and its glyph grows from a small z to a full Z before it goes.
pub(crate) struct Zzz {
    pub(crate) at: [f32; 2],
    pub(crate) vel: [f32; 2],
    pub(crate) age: f32,
    pub(crate) ttl: f32,
    pub(crate) seed: f32,
}

/// The verdict splash: when the last ant lies down, a word spins up
/// from a speck until it fills half the screen, rests a heartbeat,
/// and fades to the hill's own quiet. The growth time is drawn
/// randomly, so the spin — one clean turn and the landing — comes to
/// rest on a random tilt anywhere within 45 degrees of level.
pub(crate) struct Splash {
    pub(crate) text: String,
    pub(crate) t: f32,
    pub(crate) grow: f32,
    pub(crate) end: f32,
    /// Par met or missed, decided at the spawn: yellow or red.
    pub(crate) color: Color,
}

impl Splash {
    const HOLD: f32 = 0.35;
    const FADE: f32 = 2.475;
    pub(crate) fn total(&self) -> f32 {
        self.grow + Self::HOLD + Self::FADE
    }
}

pub(crate) struct Glide {
    pub(crate) at: [f32; 2],
    pub(crate) target: [f32; 2],
    /// The direction this walker travels — the next hop's entry tangent.
    pub(crate) dir: [f32; 2],
    /// Walk-cycle clock, in seconds. It turns continuously — ants mill
    /// about even where they stand — and only an asleep ant's drawing
    /// ignores it.
    pub(crate) anim: f32,
    pub(crate) hop: Hop,
}

impl Glide {
    pub(crate) fn step(&mut self, to: [f32; 2], dt: f32) {
        if !closer(self.target, to) {
            // The exit direction reads off the old target — the cell
            // just left; a walker that has never left anywhere reads it
            // off the hop itself.
            let mut ex = [to[0] - self.target[0], to[1] - self.target[1]];
            if ex[0].abs() + ex[1].abs() < 1e-3 {
                ex = [to[0] - self.at[0], to[1] - self.at[1]];
            }
            let exit = if ex[0].abs() >= ex[1].abs() {
                [ex[0].signum(), 0.0]
            } else {
                [0.0, ex[1].signum()]
            };
            self.hop.to(self.at, to, self.dir, exit, TRAIL_HOP);
            self.dir = exit;
            self.target = to;
        }
        self.at = self.hop.tick(dt);
        self.anim += dt;
    }
}

/// One drifting leaf — the simulation state the Demo owns. Fluttering is
/// deliberately *not* `ParticleSystem::update`: that is a gravity
/// integrator, while a leaf sways, flips edge-on and rides the gust down
/// and clean out of the world — the wind keeps no litter pile. The
/// engine leaves `size`, `angle` and `color` to the caller for exactly
/// this — the systems only carry the finished particles into the sprite
/// batches.
pub(crate) struct Leaf {
    /// Which sprite sheet draws this leaf (index into `LEAVES`).
    pub(crate) sprite: usize,
    pub(crate) pos: [f32; 2],
    /// Descent speed in px/s, modulated by the flip.
    pub(crate) fall: f32,
    /// Sidesway amplitude in px/s.
    pub(crate) sway: f32,
    pub(crate) phase: f32,
    pub(crate) freq: f32,
    pub(crate) flip_phase: f32,
    pub(crate) flip_freq: f32,
    pub(crate) tilt: f32,
    /// Integrated rotation at a wandering speed: `spin_rate` relaxes
    /// toward `spin_target`, which is redrawn at random near every
    /// `spin_next` — so each leaf turns at speeds no clock repeats.
    pub(crate) spin: f32,
    pub(crate) spin_rate: f32,
    pub(crate) spin_target: f32,
    pub(crate) spin_next: f32,
    pub(crate) angle: f32,
    pub(crate) size: f32,
}

/// Everything a fresh draw needs to know about the current sky.
pub(crate) struct Sky {
    pub(crate) w: f32,
    pub(crate) h: f32,
    /// The canopy's parallax offset — spawn and wrap live in the
    /// leaves' own space, which keeps every re-entry at a visible edge.
    pub(crate) off: [f32; 2],
    /// The frost's odds of dealing a snowflake.
    pub(crate) snow: f32,
    /// The windward emission reach.
    pub(crate) reach: f32,
}

impl Leaf {
    /// A fresh leaf slightly above the visible window — which, in the
    /// parallax-shifted space the leaf lane lives in, is the window at
    /// `-off` — jittered inside its lane. `lane` of `lanes`.
    pub(crate) fn fresh(lane: usize, lanes: usize, sky: &Sky, rng: &mut Rng) -> Self {
        // The deal: with the frost's own odds, the wind turns a leaf
        // face down into a snowflake — the sheet changes, the
        // behaviour does not: a flake sways, flips and spins exactly
        // like any leaf.
        let sprite = if rng.next_f32() < sky.snow {
            SNOW_SHEET
        } else {
            (rng.next_f32() * SNOW_SHEET as f32) as usize % SNOW_SHEET.max(1)
        };
        // The emission width: the window, plus the windward reach a
        // gale needs upwind so its drift still paints the whole sky.
        let lane_w = (sky.w + sky.reach) / lanes.max(1) as f32;
        Self {
            sprite,
            pos: [
                -sky.w / 2.0 - sky.off[0] + lane_w * (lane as f32 + 0.5 + rng.in_range(-0.4, 0.4)),
                // The band's low end clears the fattest leaf's half-height
                // (`size` is a half-width), so nothing hangs into the window
                // and pops at birth.
                sky.h / 2.0 - sky.off[1] + rng.in_range(LEAF_SIZE.1, 140.0),
            ],
            fall: rng.in_range(38.0, 92.0),
            sway: rng.in_range(26.0, 85.0),
            phase: rng.in_range(0.0, TAU),
            freq: rng.in_range(1.3, 2.8),
            flip_phase: rng.in_range(0.0, TAU),
            flip_freq: rng.in_range(0.7, 1.9),
            tilt: rng.in_range(-0.25, 0.25),
            spin: rng.in_range(0.0, TAU),
            spin_rate: rng.in_range(-0.9, 0.9),
            spin_target: rng.in_range(-1.8, 1.8),
            spin_next: 0.0,
            angle: 0.0,
            size: rng.in_range(LEAF_SIZE.0, LEAF_SIZE.1),
        }
    }
}

impl Demo {
    /// Autumn over the hill: thirty leaves flutter down and drift clean
    /// out past the bottom edge, re-entering above the window — the
    /// wind keeps no litter. One step per leaf, then the eleven sprite
    /// batches are rebuilt whole — with thirty particles, incremental
    /// bookkeeping would only be a place to hide bugs.
    pub(crate) fn update_leaves(&mut self, ctx: &mut Context, dt: f32) {
        let (w, h) = self.view(ctx);
        // The canopy's parallax: the batches ride a node offset, so the
        // leaves' own space shows the window shifted by `-off` — spawn,
        // wrap, and ground all live in that space, which keeps every
        // re-entry at a visible edge.
        let off = [
            -self.cam[0] * ZOOM * LEAF_PARALLAX,
            -self.cam[1] * ZOOM * LEAF_PARALLAX,
        ];
        self.leaf_t += dt;
        // The weather: past par the gauge climbs, the wind leans out
        // of the right, and the snow creeps into the deck — all three
        // capped at double par, where the storm holds its note.
        let wind_mag = self.frost_t * WIND_MAX;
        // The emission width answers the wind: upwind reach equal to
        // the drift a flake performs across its whole fall, so the
        // gale paints an even sky instead of a bare right edge.
        let reach = wind_mag * h / WIND_MAX;
        // A slow shared gust all the leaves hang from — and under it,
        // once it comes, the gale out of the right.
        let wind =
            16.0 * (0.11 * self.leaf_t).sin() + 7.0 * (0.37 * self.leaf_t + 2.1).sin() - wind_mag;
        let sky = Sky {
            w,
            h,
            off,
            snow: self.frost_t,
            reach,
        };
        if !self.leaves_seeded {
            self.leaves_seeded = true;
            // The deck deals all eighty, but the calm-weather ledger
            // draws only the first thirty — so seed on the *drawn* lane
            // grid, not the deck's size. Seeding `lane of N_MAX` put the
            // visible thirty at the sky's left 3/8; the modulo fills
            // every drawn lane three times over, so the open frame is
            // wide even before the storm calls in the rest (who are
            // re-dealt on activation anyway).
            self.leaves = (0..N_MAX)
                .map(|lane| Leaf::fresh(lane % N_LEAVES, N_LEAVES, &sky, &mut self.leaf_rng))
                .collect();
            // Scatter the first volley through the air, so the hill
            // opens already in autumn rather than waiting for a fall.
            for leaf in &mut self.leaves {
                leaf.pos[1] = self
                    .leaf_rng
                    .in_range(-h / 2.0 - off[1] + 20.0, h / 2.0 - off[1] + 60.0);
            }
        }
        let mut batches = vec![ParticleSystem::default(); LEAVES.len()];
        // The storm's ledger: thirty in calm weather, swelling toward
        // eighty as the frost deepens — newcomers are called in from
        // above, dealt like everything else.
        let active = (N_LEAVES as f32 + (N_MAX - N_LEAVES) as f32 * self.frost_t) as usize;
        if active > self.leaves_active {
            for i in self.leaves_active..active {
                self.leaves[i] = Leaf::fresh(i, active, &sky, &mut self.leaf_rng);
            }
        }
        self.leaves_active = active;
        for (i, leaf) in self.leaves.iter_mut().enumerate().take(active) {
            leaf.phase += leaf.freq * dt;
            leaf.flip_phase += leaf.flip_freq * dt;
            let flip = leaf.flip_phase.sin();
            // A wandering spin: the rate chases a target that is
            // redrawn at random moments — speeding up, easing off,
            // sometimes reversing — never a metronome.
            if self.leaf_t >= leaf.spin_next {
                leaf.spin_target = self.leaf_rng.in_range(-1.8, 1.8);
                leaf.spin_next = self.leaf_t + self.leaf_rng.in_range(0.6, 2.4);
            }
            leaf.spin_rate += (leaf.spin_target - leaf.spin_rate) * (dt * 2.2).min(1.0);
            leaf.spin += leaf.spin_rate * dt;
            // Edge-on, a leaf falls faster; the scale barely notices
            // (a 6 % whisper), and the size "zoom" the flip used to
            // pump is gone entirely.
            let vx = leaf.sway * leaf.phase.cos() + wind;
            let vy = -leaf.fall * (0.62 + 0.38 * flip.abs());
            leaf.pos[0] += vx * dt;
            leaf.pos[1] += vy * dt;
            leaf.angle = leaf.tilt + 0.45 * leaf.phase.cos() + leaf.spin;
            // The gust may carry a leaf across the whole width. The
            // margin clears the fattest leaf's half-width so nothing
            // pops at an edge.
            let m = 100.0;
            if leaf.pos[0] > w / 2.0 - off[0] + m + reach {
                leaf.pos[0] -= w + 2.0 * m + reach;
            } else if leaf.pos[0] < -w / 2.0 - off[0] - m {
                // Taken by the gale: the flake walks back onto the
                // windward edge, anywhere along the reach and at its
                // own height — drift and feed stay even. The landing
                // stays under the edge that wraps the other way: a
                // range reaching past it would hand the flake straight
                // back, ping-ponging between margins instead of
                // walking in.
                leaf.pos[0] = w / 2.0 - off[0] + self.leaf_rng.in_range(0.0, m + reach);
            }
            // Past the bottom and the leaf is simply gone — recycled to
            // its lane above the window. No pile, no fade.
            if leaf.pos[1] < -h / 2.0 - off[1] - m {
                *leaf = Leaf::fresh(i, active, &sky, &mut self.leaf_rng);
            }
            batches[leaf.sprite].spawn(Particle {
                pos: leaf.pos,
                vel: [vx, vy],
                // Nothing here dies of old age; nothing here fades.
                life: 1.0,
                max_life: 1.0,
                size: leaf.size * (0.94 + 0.06 * flip.abs()),
                angle: leaf.angle,
                color: Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
            });
        }
        for (node, batch) in ctx.scene().root.children.iter_mut().zip(batches) {
            node.transform = Transform::translate(off);
            if let Some(frost::Shape::Particles { system, .. }) = node.shape.as_mut() {
                *system = batch;
            }
        }
    }

    /// Advance one lamp's breath: the phase turns at the lamp's rate,
    /// and the exertion chases the speed the lamp is actually
    /// travelling at — brisk on the way up, gradual on the way down,
    /// so stopping slows the pace by degrees rather than by switch.
    /// Phases start scattered on the golden angle, so a dorm of
    /// sleepers never breathes in chorus. A resting lamp also blooms
    /// its zs: a burst of two to five, spread over the eighth of a
    /// breath that leads to the crest, the last z landing exactly on
    /// the peak of the breath.
    pub(crate) fn breathe(&mut self, id: usize, at: [f32; 2], resting: bool, dt: f32) {
        let e = self
            .breath
            .entry(id)
            .or_insert((id as f32 * 2.399_963_3, 0.0, at, resting));
        if resting && !e.3 {
            // Sleep does not strike up mid-breath. It starts three
            // quarters into one, so the first bloom is soon behind
            // the sleeper — soon, not instant, the way a first yawn
            // still has to gather.
            e.0 = 3.0 * std::f32::consts::FRAC_PI_2;
        }
        e.3 = resting;
        let dx = at[0] - e.2[0];
        let dy = at[1] - e.2[1];
        e.2 = at;
        let speed = (dx * dx + dy * dy).sqrt() / dt.max(1e-3);
        let target = (speed / BREATH_FULL_SPEED).min(1.0);
        let tau = if target > e.1 {
            BREATH_UP_TAU
        } else {
            BREATH_DOWN_TAU
        };
        e.1 += (target - e.1) * (1.0 - (-(dt / tau)).exp());
        let rate = if resting {
            BREATH_SLEEP_RATE
        } else {
            BREATH_CALM_RATE + (BREATH_WORK_RATE - BREATH_CALM_RATE) * e.1
        };
        let prev = e.0;
        e.0 = (e.0 + dt * rate * std::f32::consts::TAU) % std::f32::consts::TAU;
        if !resting {
            self.zbloom.remove(&id);
            return;
        }
        if e.0 < prev {
            // A breath has just begun: draw the next bloom's lot — how
            // many zs, and how wide a window. The window is an eighth
            // of the breath, ± a quarter of itself redrawn every
            // cycle, always ending its last z on the crest.
            let n = (self.leaf_rng.in_range(2.0, 5.0).round() as u32).clamp(2, 5);
            let w = std::f32::consts::FRAC_PI_4 * self.leaf_rng.in_range(0.75, 1.25);
            self.zbloom
                .insert(id, (n, std::f32::consts::FRAC_PI_2 - w, w / (n - 1) as f32));
        }
        // Angles run from an eighth-turn out to the quarter-turn that
        // crowns the crest — an eighth of the period, to the last z.
        let mut fire = 0u32;
        if let Some(b) = self.zbloom.get_mut(&id) {
            while b.0 > 0 && prev < b.1 && b.1 <= e.0 {
                b.0 -= 1;
                b.1 += b.2;
                fire += 1;
            }
        }
        for _ in 0..fire {
            self.spawn_z(at);
        }
    }

    /// One z off a sleeper's breath, into the air above its head:
    /// tilted a random way and never quite the same twice.
    pub(crate) fn spawn_z(&mut self, at: [f32; 2]) {
        let jx = self.leaf_rng.in_range(-6.0, 6.0);
        // Three fifths of the old drift and rise — a sleepy exhale —
        // and a life stretched by the same factor, so each z still
        // climbs as high as it ever did, only slower.
        let vx = self.leaf_rng.in_range(-18.0, 18.0) * 0.6;
        let vy = self.leaf_rng.in_range(30.0, 48.0) * 0.6;
        let ttl = self.leaf_rng.in_range(1.6, 2.3) / 0.6;
        let seed = self.leaf_rng.in_range(0.0, std::f32::consts::TAU);
        self.zs.push(Zzz {
            at: [at[0] + jx, at[1] + TILE * 0.34],
            vel: [vx, vy],
            age: 0.0,
            ttl,
            seed,
        });
    }

    // ------------------------------------------------------------- draw

    /// The painted shadow's foot: the ant's place, shifted
    /// down-right by two world pixels at a walk and one when the
    /// ant sleeps — `ZOOM`-scaled like the ant's own geometry, so
    /// the lean is a fixed share of the ant at any window size.
    pub(crate) fn shadow_at(&self, at: [f32; 2], asleep: bool) -> [f32; 2] {
        let lean = if asleep {
            SHADOW_LEAN_ASLEEP
        } else {
            SHADOW_LEAN_AWAKE
        };
        [at[0] + lean * ZOOM, at[1] - lean * ZOOM]
    }

    /// Places one baked ant frame as a scene node: centered on `at`,
    /// turned so the head points along `face`, mirrored for a left walk,
    /// tinted by `tint`, drawn at `order` — the same z ladder the immediate
    /// draws use (trail ants ≈ 1.0, the player's silhouette 1.48, the
    /// player 1.5). `shape` is a walk-cycle frame or its answering
    /// stencil: a shadow is a real ant laid out exactly as the ant it
    /// shadows. Cloning the shape just bumps an `Arc`; only the node's
    /// numbers change from frame to frame.
    pub(crate) fn put_ant(
        &self,
        node: &mut SceneNode,
        shape: &frost::Shape,
        at: [f32; 2],
        face: [f32; 2],
        tint: Color,
        lamp: f32,
    ) {
        node.shape = Some(shape.clone());
        // The frames face right. Walking left mirrors the sheet; walking
        // up or down turns it a quarter, and a mirrored sheet turns the
        // other way so its head still points where the ant walks.
        let (sx, ang) = if face[0] < 0.0 {
            (-1.0, (-face[1]).atan2(-face[0]))
        } else {
            (1.0, face[1].atan2(face[0].max(0.0)))
        };
        let s = ANT_W * ZOOM / shape.sprite_size().map_or(1.0, |sz| sz[0]).max(1.0);
        node.transform = Transform::rotate(ang).compose(&Transform::translate(at));
        node.scale = [sx * s, s];
        node.modulate = tint;
        // An ant is not only lit by the night — it carries its own
        // ember, so the lamp it casts always has a face to stand on
        // even at the very floor of the ambient. And the ember breathes
        // with that lamp: the same number the light node was just given,
        // scaled down, which is why one pulse crosses the snow and the
        // ant together and why the `light fx` slider takes the glow with
        // it when it takes the lamps down.
        node.lit = true;
        node.emission = shade(tint, EMBER_FLOOR + EMBER_PER_LAMP * lamp);
        // The halo is the light's color, not the shell's — the core a lamp
        // throws rather than the cuticle it came from, so an ant's glow is a
        // whiter, warmer thing than the ant. It rides the same number, and at
        // no lamp it is black, which the blend answers exactly as it answers
        // no halo at all: the dark copy under the player asks for neither.
        node.glow = Some(shade(lamp_core(tint), GLOW_PER_LAMP * lamp));
    }

    pub(crate) fn draw_ants(&mut self, ctx: &mut Context, dt: f32) {
        let Some(game) = &self.game else { return };
        // Chain link line: player → head → … → tail.
        if !game.trail.is_empty() {
            let mut pts: Vec<[f32; 2]> = vec![self.screen(self.player_at)];
            for i in 0..game.trail.len() {
                if let Some(g) = self.glides.get(&game.trail[i]) {
                    pts.push(self.screen(g.at));
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
        // One scene node per ant, one painted shadow under each, and
        // a pair more for the queen, hung on the root after the leaf
        // batches. Frame index is the walk-cycle clock: every awake
        // ant cycles, standing still or striding; an asleep ant
        // always draws frame one, dimmed.
        let frame_of = |anim: f32, asleep: bool| {
            if asleep {
                0
            } else {
                ((anim * ANT_WALK_FPS) as usize) % ANT_FRAMES.len()
            }
        };
        // The hill's solid cells are soil too: the same sprite the
        // ground wears, shrunk from a thousand pixels to a tile's two
        // hundred, so the grain the background can only whisper, the
        // walls shout — one material at two distances is the whole
        // depth trick. Sprites only ride scene nodes, so the hill's
        // rock and dirt are rebuilt each frame from a pool sized to
        // the count the level holds — and as nodes they join the light
        // field: they receive the ants' lamps, and they occlude them,
        // so a wall between an ant and a pixel puts that pixel in
        // shadow. Dirt is the same sprite, tinted toward the old C_DIRT
        // brown: loose earth, kin to the wall.
        let tiles: Vec<(i32, i32, bool)> = (0..game.level.rows as i32)
            .flat_map(|row| (0..game.level.cols as i32).map(move |col| (row, col)))
            .filter_map(|(row, col)| {
                let rock = matches!(game.tile(game::Cell::new(col, row)), level::Tile::Rock);
                let dirt = matches!(game.tile(game::Cell::new(col, row)), level::Tile::Dirt);
                (rock || dirt).then_some((row, col, rock))
            })
            .collect();
        // Slots: leaves, ants, a painted shadow per ant, the player
        // pair, one lamp per ant plus the player's, the single tile
        // map holding every solid, a beacon and a sprite per bed
        // (two apiece), the dig band — one slot per dirt cell the
        // hill began with, the ceiling on simultaneous dissolves —
        // and the two backdrop sprites.
        let slots =
            LEAVES.len() + game.ants.len() * 3 + 6 + game.level.beds.len() * 2 + self.dig_cap;
        fit_pool(&mut ctx.scene().root, slots);
        // A hill swap moves every band: the pool grew or shrank, so a
        // node that wore an ember or a dissolve last hill can stand
        // under the backdrop today. The sheet blocks dress shape,
        // colour and light but never `emission` or `glow` — a
        // backdrop wearing a dead dig's glow reads as sky on fire —
        // so once per swap the pool is washed to its resting state,
        // before any band re-dresses what it wears this frame.
        if self.pool_reset {
            self.pool_reset = false;
            wash_pool(&mut ctx.scene().root);
        }
        // Every ant is a lamp, and every lamp breathes: at rest a
        // gentle sway, working a deeper faster pulse, and stopping
        // winds the pace down by degrees. Asleep keeps only a faint
        // slow pulse on the dark — the dorm breathes, it does not go
        // out — and the tuck-in spike burns loud over it. The player
        // is an ant too, and the queen's bed is her target: same
        // breath, same flare.
        // Every lamp's strength for this frame, in the units the light nodes
        // get — index `game.ants.len()` is the player's, who is an ant too.
        // The shells read it back below, because an ember and a halo are the
        // breath the snow already sees, not a second pulse to keep in step.
        let mut lamps = vec![0.0; game.ants.len() + 1];
        {
            let reach = TILE * ZOOM * LAMP_REACH;
            let base = LEAVES.len() + game.ants.len() + 2;
            for (id, lamp) in lamps[..game.ants.len()].iter_mut().enumerate() {
                let node = &mut ctx.scene().root.children[base + id];
                let Some(g) = self.glides.get(&id) else {
                    node.shape = None;
                    continue;
                };
                let asleep = matches!(game.ants[id].state, game::AntState::Asleep(_));
                let flare = self.landing.get(&id).copied().unwrap_or(0.0);
                let (phase, exertion, ..) = self
                    .breath
                    .get(&id)
                    .copied()
                    .unwrap_or((0.0, 0.0, [0.0; 2], false));
                let intensity = if asleep {
                    BREATH_SLEEP_BASE + BREATH_SLEEP_AMP * phase.sin() + flare * 3.0
                } else {
                    BREATH_LAMP_BASE
                        + (BREATH_CALM_AMP + (BREATH_WORK_AMP - BREATH_CALM_AMP) * exertion)
                            * phase.sin()
                };
                if intensity <= 0.0 {
                    node.shape = None;
                    continue;
                }
                // A little white in the core, the ant's own hue in the
                // throw of it.
                let col = lamp_core(ant_color(game.ants[id].color));
                node.shape = Some(frost::Shape::Light {
                    light: frost::Light::point(col, intensity * self.settings.light, reach)
                        .with_penumbra(self.settings.shadows.penumbra())
                        .with_shadow_taps(self.settings.shadows.taps()),
                });
                node.transform = Transform::translate(self.screen(g.at));
                *lamp = intensity * self.settings.light;
            }
            // The player breathes with the rest: a working pulse on
            // the walk, the queen's bed a loud spike, and then the
            // faint slow sway of the tucked-in.
            let node = &mut ctx.scene().root.children[base + game.ants.len()];
            let (phase, exertion, ..) = self
                .breath
                .get(&game.ants.len())
                .copied()
                .unwrap_or((0.0, 0.0, [0.0; 2], false));
            let intensity = match self.player_home {
                Some(t) => BREATH_SLEEP_BASE + BREATH_SLEEP_AMP * phase.sin() + t * 4.5,
                None => {
                    BREATH_LAMP_BASE
                        + (BREATH_CALM_AMP + (BREATH_WORK_AMP - BREATH_CALM_AMP) * exertion)
                            * phase.sin()
                }
            };
            if intensity > 0.0 {
                node.shape = Some(frost::Shape::Light {
                    light: frost::Light::point(
                        Color {
                            r: 1.0,
                            g: 0.86,
                            b: 0.66,
                            a: 1.0,
                        },
                        intensity * self.settings.light,
                        reach,
                    )
                    .with_penumbra(self.settings.shadows.penumbra())
                    .with_shadow_taps(self.settings.shadows.taps()),
                });
            } else {
                node.shape = None;
            }
            node.transform = Transform::translate(self.screen(self.player_at));
            lamps[game.ants.len()] = intensity.max(0.0) * self.settings.light;
        }
        // The map as a window, in two sprites. Behind: the far
        // background — the map's own rectangle, plus the one slack its
        // own drift demands — drifting at `DEEP_PARALLAX` of the
        // camera. The drift is camera-absolute and the camera chases
        // the player anywhere on the hill, so the hole can sit off the
        // sprite's centre by `(1 − DEEP_PARALLAX)` of the camera's
        // reach — up to that fraction of half the map. Size for the
        // worst case and the slack lies under the ground sheet,
        // forever invisible; undersize it and the hole outruns the
        // sprite into a band of `C_BG`, which is what launched.
        // In front: the ground sheet — flat, in the window's original
        // `Background` colour — welded to the board's speed at 1.0,
        // whose punched hole hides every far edge the hole does not
        // contain.
        {
            let [tw, th] = self.soil.sprite_size().unwrap_or([1.0, 1.0]);
            let cw = game.level.cols as f32 * TILE * ZOOM;
            let ch = game.level.rows as f32 * TILE * ZOOM;
            let slack = |m: f32| m * (1.0 - DEEP_PARALLAX) * 0.5 * 1.05;
            let node = &mut ctx.scene().root.children[slots - 1];
            node.shape = Some(self.soil.clone());
            node.transform = Transform::translate([
                -self.cam[0] * ZOOM * DEEP_PARALLAX,
                -self.cam[1] * ZOOM * DEEP_PARALLAX,
            ]);
            node.scale = [(cw + 2.0 * slack(cw)) / tw, (ch + 2.0 * slack(ch)) / th];
            node.modulate = Color {
                r: 0.45,
                g: 0.4,
                b: 0.38,
                a: 1.0,
            };
            node.lit = true;
            // The deep parallax sheet listens on the shadow layer beside
            // the default one: the lamps still reach it (layer 0 is in the
            // set), and it is the only surface the hill's tile shadows land
            // on (see [`BG_SHADOW_LAYER`]).
            node.lit_layers = LightLayers::default() | BG_SHADOW_LAYER;
            node.order = -0.5;
        }
        if let Some((shape, span)) = &self.ground {
            let [gw, gh] = shape.sprite_size().unwrap_or([1.0, 1.0]);
            let node = &mut ctx.scene().root.children[slots - 2];
            node.shape = Some(shape.clone());
            node.transform = Transform::translate(self.screen([0.0, 0.0]));
            node.scale = [span[0] / gw, span[1] / gh];
            node.modulate = Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            };
            // Lit but not occluding — and, listening on plain layer 0
            // like everything but the sheet, not shadowed either: a
            // lamp near the edge throws its emission onto the
            // surrounding ground through the border hairlines, and
            // the tiles' shadows stay where they belong, on the sky
            // behind the hill.
            node.lit = true;
            node.order = -0.4;
        }
        // Painted occlusion is a rectangle's privilege — a sprite's
        // body casts no shadow on the flag alone — so the hill's solid
        // cells declare their boxes here. But every occluder the field
        // carries is tested by every fragment any light reaches, so
        // one box per tile at 4K is a frame budget in chains: instead,
        // the cells merge into a small cover of boxes — row runs, equal
        // spans stacked, and a fixpoint pass over the result (see
        // [`merge_boxes`]). A merged cover casts the same shadow its
        // tiles ever did — the tile hairlines that used to leak a thread
        // of light measure a pixel, and a merged edge is invisible
        // against the row it belongs to.
        //
        // The merge itself is cached on the cell list: the hill is
        // static between digs, so the run scan, the sort, and the walk
        // re-run only when the cells actually change. What every frame
        // does instead is re-pack the cached rectangles into screen
        // space — the camera pan forces fresh pixel-space boxes either
        // way, but packing cached runs is arithmetic where merging is
        // search, and the frames between digs are the overwhelming
        // majority of the game.
        {
            let ts = TILE * ZOOM;
            if tiles != self.occ_key {
                self.occ_boxes = merge_boxes(&tiles);
                self.occ_key.clone_from(&tiles);
            }
            // Off, the field never learns of a single occluder: the
            // packer hands every light an empty block, `occluded`
            // answers without one record load, and the hill pays only
            // for the lamps' arithmetic — the weak-hardware shape of
            // this renderer, decided at the menu rather than in the
            // shader.
            for &(col_s, col_e, row0, row1) in self
                .occ_boxes
                .iter()
                .filter(|_| self.settings.shadows != Shadow::None)
            {
                let [x0, y0] = self.screen(self.cell_px(game::Cell::new(col_s, row0)));
                let [x1, y1] = self.screen(self.cell_px(game::Cell::new(col_e, row1)));
                // Full cells, not drawn tiles: the hairline inset is a
                // detail of the grain, while the hill's body is solid
                // to the grid — so the shadow must be too. And the
                // shadow goes only where [`BG_SHADOW_LAYER`] reaches:
                // the merged boxes darken the far background behind the
                // hill, never the tiles that cast them, never an ant
                // walking beside them — a wall that shades the sky it
                // stands against while the floor it stands on sees no
                // shadow at all is exactly the hill's demand.
                ctx.occluder_layers(
                    Transform::translate([(x0 + x1) * 0.5, (y0 + y1) * 0.5]),
                    [0.0, 0.0],
                    [((x1 - x0) + ts) * 0.5, ((y0 - y1) + ts) * 0.5],
                    BG_SHADOW_LAYER,
                );
            }
        }
        {
            // The hill's solids draw as one tile map: every wall and
            // dirt cell is a placement sharing the soil atlas, drawn
            // instanced in a single call — the engine's answer to the
            // per-cell node pool this replaces. Placements ride the
            // node's space, which is the screen's here; tint per tile
            // — rock bare soil, dirt the old C_DIRT, both keeping the
            // standing jitter — and the whole map lights as one
            // surface. The shadows are the merged boxes declared
            // above: batched art and shadow arithmetic are separate
            // problems, and only one of them is a draw call.
            let mut map = self.tile_atlas.clone();
            let inset = TILE * ZOOM - 1.5;
            let wall = Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            };
            let dirt_tint = Color {
                r: C_DIRT.r * 2.2,
                g: C_DIRT.g * 2.2,
                b: C_DIRT.b * 2.2,
                a: 1.0,
            };
            map.set_tiles(tiles.iter().map(|(row, col, rock)| {
                let [x, y] = self.screen(self.cell_px(game::Cell::new(*col, *row)));
                let noise = 1.0 + ((col * 31 + row * 17) % 3) as f32 * 0.045;
                let mut tile = Tile::new([x, y], [inset, inset], [0.0, 0.0, 1.0, 1.0]);
                tile.color = shade(if *rock { wall } else { dirt_tint }, noise);
                tile
            }));
            let node = &mut ctx.scene().root.children[LEAVES.len() + game.ants.len() * 2 + 3];
            node.shape = Some(map);
            node.transform = Transform::translate([0.0, 0.0]);
            node.scale = [1.0, 1.0];
            node.modulate = wall;
            node.lit = true;
            node.order = 0.0;
        }
        // The beds are the hill's beacons: each carries a faint light
        // of its own colour — enough to find across a dark room and to
        // aim a push by, never enough to light one. They burn always:
        // a taken bed stays self-lit, the target outlives the ant.
        {
            let base = LEAVES.len() + game.ants.len() * 2 + 4;
            for (i, (cell, color)) in game.level.beds.iter().enumerate() {
                let node = &mut ctx.scene().root.children[base + i];
                node.shape = Some(frost::Shape::Light {
                    light: frost::Light::point(
                        ant_color(*color),
                        0.5 * self.settings.light,
                        TILE * ZOOM * 1.3,
                    )
                    .with_penumbra(self.settings.shadows.penumbra())
                    .with_shadow_taps(self.settings.shadows.taps()),
                });
                node.transform = Transform::translate(self.screen(self.cell_px(*cell)));
            }
        }
        // And the beds receive as well as give: the user's own bed
        // sprite, square on the cell, lit by the beacon it sleeps
        // under and every lamp that reaches it.
        {
            let base = LEAVES.len() + game.ants.len() * 2 + 4 + game.level.beds.len();
            let [tw, th] = self.bed.sprite_size().unwrap_or([1.0, 1.0]);
            let s = (TILE * ZOOM - 2.0) / tw.max(th);
            for (i, (cell, color)) in game.level.beds.iter().enumerate() {
                let node = &mut ctx.scene().root.children[base + i];
                node.shape = Some(self.bed.clone());
                node.transform = Transform::translate(self.screen(self.cell_px(*cell)));
                node.scale = [s, s];
                // Each bed wears its sleeper's colour — the sprite is
                // the modulate's base cloth, not the other way round.
                node.modulate = ant_color(*color);
                node.lit = true;
                node.order = 0.05;
            }
        }
        // The shadow band: the pool grew one slot per ant, and the
        // beds ended where the backdrops' two-anchor tail begins —
        // so the new slots land between, and every expression above
        // this line keeps counting as it always did.
        let sh_base = LEAVES.len() + game.ants.len() * 2 + 4 + game.level.beds.len() * 2;
        for (id, &lamp) in lamps[..game.ants.len()].iter().enumerate() {
            let Some(g) = self.glides.get(&id) else {
                continue;
            };
            let asleep = matches!(game.ants[id].state, game::AntState::Asleep(_));
            let face = *facing.get(&id).unwrap_or(&[1.0, 0.0]);
            let col = ant_color(game.ants[id].color);
            let col = if asleep {
                // Tucked in: blaze loud, then fade to the dim sleeping
                // variant. The flare's modulate runs past white and
                // lets the framebuffer clamp it — the clamp is what
                // makes loud look loud. The hold-then-fade curve keeps
                // the blaze a heartbeat before it dims.
                let dim = shade(col, 0.55);
                let k = (self.landing.get(&id).copied().unwrap_or(0.0) * 1.7).min(1.0);
                blaze(
                    dim,
                    Color {
                        r: col.r * 2.4 + 0.9,
                        g: col.g * 2.4 + 0.9,
                        b: col.b * 2.4 + 0.9,
                        a: 1.0,
                    },
                    k,
                )
            } else if matches!(game.ants[id].state, game::AntState::Wild(_)) {
                shade(col, 0.88)
            } else {
                col
            };
            let z = 1.0 + g.at[1] * 1e-4;
            let node = &mut ctx.scene().root.children[LEAVES.len() + id];
            let at = self.screen(g.at);
            // Facings turn, never snap: toward travel on foot, toward
            // the ceiling when tucked in.
            let want_face = if asleep { [0.0, 1.0] } else { face };
            let rf = self.render_face.entry(id).or_insert(want_face);
            *rf = turn_toward(*rf, want_face, TURN_RATE * dt);
            let rf = *rf;
            self.put_ant(
                node,
                &self.ant_frames[frame_of(g.anim, asleep)],
                at,
                rf,
                col,
                lamp,
            );
            node.order = z;
            // And the painted shadow: the queen's trick for every
            // ant — its own stencil, silhouette in pure ink, leaned
            // down-right (two world pixels walking, one asleep),
            // parked a hair under its ant's
            // rung on the z ladder so every shadow rides its own
            // ant.
            let sh = &mut ctx.scene().root.children[sh_base + id];
            self.put_ant(
                sh,
                &self.ant_stencils[frame_of(g.anim, asleep)],
                self.shadow_at(at, asleep),
                rf,
                ANT_SHADOW,
                0.0,
            );
            sh.order = z - 2e-4;
        }
        // The dig band: one dissolve quad per live countdown, right
        // where the shadow band ends and the tail begins. The tile
        // map has already forgotten the cell — the dig took it the
        // same frame the countdown started — so this node is the last
        // dirt it was, eating itself over the length of the hop.
        // `modulate` rides white and the program does its own
        // tinting: `glow` carries the cell's dirt color, noise and
        // all (the same tint the tile map wore), and `emission`
        // carries the eat — progress across the hop, the cell's seed,
        // the band's softness. One rung above the tile map, below the
        // beds, under every ant.
        {
            let base = LEAVES.len() + game.ants.len() * 3 + 4 + game.level.beds.len() * 2;
            let inset = TILE * ZOOM - 1.5;
            let [tw, th] = self.dissolve.sprite_size().unwrap_or([1.0, 1.0]);
            let dirt_tint = Color {
                r: C_DIRT.r * 2.2,
                g: C_DIRT.g * 2.2,
                b: C_DIRT.b * 2.2,
                a: 1.0,
            };
            for (i, &(col, row, left, seed)) in self.digs.iter().enumerate() {
                let node = &mut ctx.scene().root.children[base + i];
                let noise = 1.0 + ((col * 31 + row * 17) % 3) as f32 * 0.045;
                node.shape = Some(self.dissolve.clone());
                node.transform =
                    Transform::translate(self.screen(self.cell_px(game::Cell::new(col, row))));
                node.scale = [inset / tw, inset / th];
                node.modulate = Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                };
                node.emission = Color {
                    r: 1.0 - (left / DIG_HOP).clamp(0.0, 1.0),
                    g: seed,
                    b: 0.14,
                    // The grain, as the shelf counts it: five noise
                    // cycles across a dig cell — the lattice hides
                    // inside one organic bite at a tile's scale.
                    a: 5.0,
                };
                node.glow = Some(shade(dirt_tint, noise));
                node.lit = true;
                node.lit_layers = LightLayers::default();
                node.order = 0.02;
            }
            for node in &mut ctx.scene().root.children[base + self.digs.len()..base + self.dig_cap]
            {
                // Released digs go back to the pool whole: a node
                // that only stopped drawing still wears its last
                // frame's fx, and any band that inherits this slot
                // next would wear a dissolve it never asked for.
                node.shape = None;
                rest_light(node);
            }
        }
        // The zs: one born at each breath's crest, adrift and growing
        // from a small z to a full Z, wandering one way and then the
        // other, and thinning out as it climbs. Immediate text rides
        // above the dark like every other word in the game.
        for zz in &self.zs {
            let k = (zz.age / zz.ttl).min(1.0);
            let [x, y] = self.screen([
                zz.at[0] + 8.0 * (zz.seed + zz.age * 1.56).sin() * k,
                zz.at[1],
            ]);
            ctx.text(
                x,
                y,
                &self.font,
                "z",
                // Whole points only: the glyph atlas is keyed by size,
                // and a z that grew through fractional sizes would
                // bake itself a fresh atlas every frame.
                (16.0 + 36.0 * k).round(),
                700.0,
                Color {
                    r: C_DIM.r,
                    g: C_DIM.g,
                    b: C_DIM.b,
                    a: 1.0 - k * k,
                },
                1.56,
            );
        }
        // The player: a translucent twin of the same frame a couple
        // of pixels off — the same painted shadow the whole colony
        // wears, since the queen reads best when she reads like them
        // — with her own frame on top.
        let n = game.ants.len();
        let frame = frame_of(self.player_anim, false);
        {
            let root = &mut ctx.scene().root;
            let back = n;
            let player = n + 1;
            let at = self.screen(self.player_at);
            let shadowed = &mut root.children[LEAVES.len() + back];
            self.put_ant(
                shadowed,
                &self.ant_stencils[frame],
                self.shadow_at(at, self.player_home.is_some()),
                self.player_face,
                ANT_SHADOW,
                0.0,
            );
            shadowed.order = 1.48;
            // The queen carries a lamp of her own — the breath written on her
            // node above — so she wears the same ember and the same halo as
            // any ant, at her light's own strength. And she flashes on the bed
            // like one: her spike always reached her lamp and never her pixels,
            // which is why her tuck-in lit the snow and left her out of it. The
            // same hold-then-drain curve the ants ride, on the amber.
            let queen = &mut root.children[LEAVES.len() + player];
            self.put_ant(
                queen,
                &self.ant_frames[frame],
                at,
                self.player_face,
                blaze(
                    ant_color(game::AntColor::White),
                    CROWN_BLAZE,
                    (self.player_home.unwrap_or(0.0) * 1.7).min(1.0),
                ),
                lamps[n],
            );
            queen.order = 1.5;
        }
    }

    /// Draw the verdict: one clean spin from a speck, eased so the
    /// speck lingers tiny before it blooms to half the screen; a
    /// heartbeat at full size; then the fade to the hill's own words.
    /// The four held-centre messages, one screen and one voice: the
    /// tour's answer to its own praise — in the praise words' own
    /// case, twice the banner's size, over the still won hill while
    /// `better_t` counts down, the hill dealing itself again on the
    /// better script as the hold runs out — the oops deck's two
    /// admissions over the hill sulking on its single mistake and
    /// then on its pair — and the cheap dig's send-off to a pupil
    /// who has just taken the dig. All four hang at the same
    /// height: a quarter of the screen down from the top, clear of
    /// the queen's marks and the foot words below.
    pub(crate) fn draw_better(&self, ctx: &mut Context) {
        let note = if self.better_t > 0.0 {
            BETTER_NOTE
        } else if self.mistake_t > 0.0 {
            if self.disaster_standing() {
                NOT_AGAIN_NOTE
            } else {
                MISTAKE_NOTE
            }
        } else if self.send_t > 0.0 {
            SEND_OFF_NOTE
        } else {
            return;
        };
        let (_, h) = self.view(ctx);
        ctx.text(
            0.0,
            h * 0.25,
            &self.font,
            note.to_string(),
            BETTER_EM,
            600.0,
            C_TEXT,
            UI + 1.0,
        );
    }

    pub(crate) fn draw_splash(&self, ctx: &mut Context) {
        let Some(s) = &self.splash else { return };
        let (w, _) = self.view(ctx);
        let k = (s.t / s.grow).min(1.0);
        let e = k * k * (3.0 - 2.0 * k);
        // The word is baked once, at one fixed size, and the growth
        // rides the node's scale: the whole splash needs exactly one
        // glyph atlas, and the bloom is smooth at every step.
        const SPLASH_EM: f32 = 200.0;
        // Full growth: the word fills half the screen's width.
        let scale = (e * w / (1.3 * SPLASH_EM * s.text.len().max(4) as f32)).max(0.002);
        // Five full turns — a quarter faster spinning than four —
        // landing on the drawn tilt within 45 degrees of level; the
        // random growth time keeps the landing unpredictable, the
        // angle is simply where the turn stops.
        let angle = (5.0 * std::f32::consts::TAU + s.end) * e;
        let alpha = if s.t <= s.grow + Splash::HOLD {
            1.0
        } else {
            (1.0 - (s.t - s.grow - Splash::HOLD) / Splash::FADE).max(0.0)
        };
        // The example's own font Arc, handed over whole: the glyph
        // atlas is keyed on the buffer's pointer, and a fresh Arc per
        // frame (what `text_bytes` builds) would mint a fresh atlas
        // every frame — the flicker this line is careful about.
        let mut shape =
            match frost::Shape::text_shared(Arc::clone(&self.font), s.text.clone(), SPLASH_EM) {
                Ok(shape) => shape,
                Err(_) => return,
            };
        if let frost::Shape::Text {
            color, alpha: a, ..
        } = &mut shape
        {
            *color = s.color;
            *a = alpha;
        }
        ctx.scene().root.children.push(Box::new(SceneNode {
            shape: Some(shape),
            transform: Transform::rotate(angle),
            scale: [scale, scale],
            order: UI + 1.0,
            ..Default::default()
        }));
    }

    /// One banner row from measured pieces laid end to end, the whole
    /// line centred as a string would be. The widths come from the
    /// engine's shaping — measured additive, checked — so measure and
    /// paint cannot drift apart. Runs carry no spaces: Barrfont's own
    /// space is 7.5 px at 30, nearly invisible under small caps, so
    /// every gap here is an explicit fraction of the size instead.
    pub(crate) fn banner(
        &self,
        ctx: &mut Context,
        cy: f32,
        size: f32,
        weight: f32,
        runs: &[(&str, Color, f32)],
    ) {
        // Measured first, drawn after: the shaping asks the canvas
        // read-only, the paint takes it back.
        let widths: Vec<f32> = runs
            .iter()
            .map(|(t, _, _)| ctx.text_width(&self.font, t, size, weight))
            .collect();
        let total: f32 =
            widths.iter().sum::<f32>() + runs.iter().map(|(_, _, gap)| gap * size).sum::<f32>();
        let mut x = -total / 2.0;
        for ((text, colour, gap), w) in runs.iter().zip(widths) {
            let half = w / 2.0;
            ctx.text(
                x + half,
                cy,
                &self.font,
                (*text).to_string(),
                size,
                weight,
                // One over the veil's head (its base, below the plate
                // and rows): the hill's record rides above the fade,
                // fully lit while any menu is up — a simple z, where
                // colours pushed past 1.0 would only meet the 8-bit
                // buffer's clamp.
                *colour,
                UI + 10.5,
            );
            x += w + gap * size;
        }
    }
}
