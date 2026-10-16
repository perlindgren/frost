//! The two halls in one: eyes that emit, eyes that light, and a renderer
//! that spreads what is past white across the pixels around it.
//!
//! `eyes` is the receiver half — self-lit eyes that keep burning inside a
//! wall's shadow, plus the two renderer-wide looks. `eyes_light` is the
//! emitter half — the larger eyes carry a `Shape::Light` child node whose
//! pool sweeps the floor and is cut off at the walls. This is both, with
//! the glow and the fold switched on, which is the only way to see the
//! thing this file exists to show:
//!
//! **A halo hangs in a patch of floor its own light cannot reach.** Walk a
//! lamp-eye behind a wall and the two halves separate in front of you. The
//! pool of light it drags across the room stops dead at the occluder,
//! feathered by its penumbra, because a light is geometry — rays run from
//! it and the wall is in the way. The glow around the eye does not stop:
//! the bloom chain reads the *emissive* channel, and that channel is the
//! surface's own pixels, with no light term in it and no shadow term
//! either. So the far side of the wall gets a halo with no light under it.
//! Neither of the two parent examples can show that, because each of them
//! has only one of the two channels to lose.
//!
//! **Bloom the glow off with `B`, the fold off with `T`, and dim the hall
//! with `D`** — all three exactly as `eyes` has them, for the same reason:
//! the comparison is free, so it belongs under a key rather than in a
//! constant someone rebuilds to change. Note what `D` does not touch: it
//! dims the hall's lamp and the ambient floor, and the eyes keep their own
//! lights at full, because those are the eyes' output and not the room's.
//!
//! One asymmetry to expect, and it is the flip side of the point above. The
//! bright *pools* the lamp-eyes throw on the floor do not bloom, however
//! bright they get — a lit surface is `texture * (ambient + light +
//! emission)`, and only the last term is what the chain reads. Light landing
//! on a wall makes the wall brighter; it does not make it glow. That is why
//! the eyes bloom and their light does not, and it is the same rule antifrost
//! leans on when an ant glows and the hill it walks on does not.
//!
//! ```text
//! cargo run --example eyes_light_bloom
//! ```

/// The eyes' texture, shared with the `eyes` examples: black outline,
/// amber iris, pale sclera.
const EYE: &str = "assets/sprites/Waspeye1.png";

/// The backdrop's color: a cold slate, dim enough to read as a room
/// only where a light reaches it.
const BACKDROP: frost::Color = frost::Color {
    r: 0.3,
    g: 0.3,
    b: 0.4,
    a: 1.0,
};

/// The walls' color: unlit, so their silhouettes — and the shadows
/// they cut, from every light including the eyes' — stay readable even
/// when the hall's own lights are down at the last `D` step.
const WALL: frost::Color = frost::Color {
    r: 0.38,
    g: 0.36,
    b: 0.46,
    a: 1.0,
};

/// The panels' colors: lit, near the room's edges, placed where the
/// lamp-eyes sweep them as they wander.
const ROSE: frost::Color = frost::Color {
    r: 0.9,
    g: 0.5,
    b: 0.6,
    a: 1.0,
};
const TEAL: frost::Color = frost::Color {
    r: 0.5,
    g: 0.85,
    b: 0.62,
    a: 1.0,
};
const CREAM: frost::Color = frost::Color {
    r: 0.85,
    g: 0.68,
    b: 0.45,
    a: 1.0,
};

/// The hall's lamp: deliberately weaker here than in `eyes` — the
/// lamp-eyes should own the lighting.
const LAMP: frost::Color = frost::Color {
    r: 0.7,
    g: 0.75,
    b: 0.9,
    a: 1.0,
};
const LAMP_RADIUS: f32 = 520.0;
const LAMP_INTENSITY: f32 = 0.8;

/// The eyes' emission and the light their child node casts: the same
/// amber, so an eye, the halo around it, and the pool it drags over the
/// room read as one source. The emission pushes past full so the pulse can
/// over-brighten; the penumbra softens the shadows the eye-light casts.
const EYE_COLOR: frost::Color = frost::Color {
    r: 1.0,
    g: 0.55,
    b: 0.15,
    a: 1.0,
};
const EYE_LIGHT_RADIUS: f32 = 240.0;
const EYE_PENUMBRA: f32 = 5.0;

/// The glow's strength and the fold's headroom, taken from `eyes` along
/// with the reasons for both: the strength is measured there (five pixels
/// out from an eye, 11, 16 and 21 eighths of full scale at 0.7, 1.4 and
/// 2.8, and 1.6 is where it stops being a smudge on a forty-to-eighty-pixel
/// sprite without going on to light the room itself), and 0.15 reserves
/// fifteen percent of the display range for everything above white.
///
/// The fold matters here more than it does in either parent. A lamp-eye's
/// pixel is `texture * (ambient + its own light + emission)`, which clears
/// white by more than the emission-only eyes do — without the fold, that
/// core does not get brighter as the pulse rises, it just gets whiter.
const BLOOM_STRENGTH: f32 = 1.6;
const HEADROOM: f32 = 0.15;

/// The hall's ambient floor, as authored — [`ROOM`] scales it along with
/// the lamp, so a dimmed hall is dimmer everywhere and not just under a
/// light.
const AMBIENT: frost::Color = frost::Color {
    r: 0.04,
    g: 0.04,
    b: 0.06,
    a: 1.0,
};

/// The room's lighting levels, on `D`: lamp and ambient multipliers, as
/// `eyes` has them. The last step keeps a fifth of the ambient rather than
/// going black for a reason measured there — the chain's wide levels lift
/// every pixel of the frame by about 0.008 of radiance, which is 3% of a
/// lit room and thirty percent of a nearly black one, so below this floor
/// the haze *is* the effect and the glow is whatever is left of it.
const ROOM: [(f32, f32); 3] = [(1.0, 1.0), (0.5, 0.5), (0.0, 0.2)];

/// The wandering speed range, in pixels per second, and the rate the
/// heading's wandering curve sweeps.
const MIN_SPEED: f32 = 55.0;
const MAX_SPEED: f32 = 105.0;
const TURN: f32 = 1.3;

/// One eye's state: where it is, where it is heading, the phases of
/// its wander curve and emission pulse, and whether it carries a light.
#[derive(Default)]
struct Eye {
    pos: [f32; 2],
    heading: f32,
    speed: f32,
    /// The two incommensurate frequencies and phases the heading drifts
    /// on — summing them gives a smooth, non-repeating wander.
    wander_a: f32,
    wander_b: f32,
    phase_a: f32,
    phase_b: f32,
    /// The pulse this eye breathes, recomputed each step and read by
    /// both the emission and (for lamp-eyes) the child light's intensity.
    pulse_freq: f32,
    pulse_phase: f32,
    pulse: f32,
    scale: f32,
    facing: f32,
    /// Whether this eye carries the `Shape::Light` child node — the
    /// lamp-eyes are the larger ones.
    lamp: bool,
}

impl Eye {
    /// Advances one eye: the pulse and the heading drift on their
    /// curves, the eye walks, bounces off the window edges, and flips
    /// to face its travel.
    fn step(&mut self, t: f32, dt: f32, limit_x: f32, limit_y: f32) {
        self.pulse = (t * self.pulse_freq + self.pulse_phase).sin();
        let margin = self.scale * 100.0;
        let (lx, ly) = (limit_x - margin, limit_y - margin);
        self.heading += (self.wander_a * (self.wander_a * t + self.phase_a).sin()
            + self.wander_b * (self.wander_b * t + self.phase_b).sin())
            * TURN
            * dt;
        self.pos[0] += self.heading.cos() * self.speed * dt;
        self.pos[1] += self.heading.sin() * self.speed * dt;
        // Bounce off the window edges by mirroring the heading — and
        // clamp, so a long frame cannot leave an eye stuck outside.
        if self.pos[0] > lx || self.pos[0] < -lx {
            self.heading = std::f32::consts::PI - self.heading;
            self.pos[0] = self.pos[0].clamp(-lx, lx);
        }
        if self.pos[1] > ly || self.pos[1] < -ly {
            self.heading = -self.heading;
            self.pos[1] = self.pos[1].clamp(-ly, ly);
        }
        // Eyes watch where they are going.
        self.facing = if self.heading.cos() >= 0.0 { 1.0 } else { -1.0 };
    }

    /// The emission for this instant: the amber's alpha breathing on the
    /// pulse — embers to flare.
    fn emission(&self) -> frost::Color {
        frost::Color {
            a: 0.65 + 0.5 * self.pulse,
            ..EYE_COLOR
        }
    }

    /// The child light for this instant: the same amber, its intensity
    /// riding the same pulse, so the pool an eye drags over the room
    /// brightens exactly when the eye itself flares — and glows exactly
    /// when it glows.
    fn light(&self) -> frost::Light {
        frost::Light::point(
            EYE_COLOR,
            (0.5 + 0.55 * self.pulse).max(0.05),
            EYE_LIGHT_RADIUS,
        )
        .with_penumbra(EYE_PENUMBRA)
    }
}

/// Whether `key` was pressed since the last frame. The context reports the
/// held state, which is what a movement wants and not what a toggle wants,
/// so the example remembers last frame's held state itself and looks for the
/// step up: one press, one change, however long the key is held.
fn pressed(ctx: &frost::Context, key: frost::KeyCode, held: &mut bool) -> bool {
    let down = ctx.key_down(key);
    let rose = down && !*held;
    *held = down;
    rose
}

/// The demo: all the eyes, the clock their wander and pulses run on, and
/// the two look knobs plus the room's own dimmer — each with the held state
/// it needs to notice a press.
#[derive(Default)]
struct EyesLightBloom {
    eyes: Vec<Eye>,
    t: f32,
    bloom_on: bool,
    bloom_held: bool,
    tone_on: bool,
    tone_held: bool,
    /// Which of [`ROOM`]'s lighting levels the hall is at.
    room: usize,
    room_held: bool,
}

impl frost::Process for EyesLightBloom {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let dt = dt.min(1.0 / 30.0);
        self.t += dt;
        let (w, h) = ctx.size();
        let (limit_x, limit_y) = (w / 2.0, h / 2.0);

        // The eyes ride the root's leading children — one transform,
        // scale, and emission write each, and a light write for the
        // lamp-eyes, whose light is their first child node.
        for (i, eye) in self.eyes.iter_mut().enumerate() {
            eye.step(self.t, dt, limit_x, limit_y);
            let node = &mut ctx.scene().root.children[i];
            node.transform = frost::Transform::translate(eye.pos);
            node.scale = [eye.scale * eye.facing, eye.scale];
            node.emission = eye.emission();
            if eye.lamp {
                // The light rides the parent: only its content changes,
                // the parenting places and pulses it along with the eye.
                node.children[0].shape = Some(frost::Shape::Light { light: eye.light() });
            }
        }

        // The two look knobs, read every frame so a held key cannot repeat
        // them, then written every frame: both are per-frame values the
        // engine reads out of the stretch pass's uniform, so writing the
        // same number back is not a change of any kind and costs nothing.
        if pressed(ctx, frost::KeyCode::KeyB, &mut self.bloom_held) {
            self.bloom_on = !self.bloom_on;
            log::info!("bloom {}", if self.bloom_on { "on" } else { "off" });
        }
        if pressed(ctx, frost::KeyCode::KeyT, &mut self.tone_held) {
            self.tone_on = !self.tone_on;
            log::info!("tone map {}", if self.tone_on { "on" } else { "off" });
        }
        ctx.set_bloom(if self.bloom_on { BLOOM_STRENGTH } else { 0.0 });
        ctx.set_tone_map(if self.tone_on { HEADROOM } else { 0.0 });

        // The hall's own light: the denominator the glow is judged
        // against. Only the hall's — this walks the root's children, and the
        // eyes' lights are grandchildren, so `D` dims the room without
        // touching what the eyes put out.
        if pressed(ctx, frost::KeyCode::KeyD, &mut self.room_held) {
            self.room = (self.room + 1) % ROOM.len();
            log::info!("room level {} of {}", self.room, ROOM.len() - 1);
        }
        let (lamp, ambient) = ROOM[self.room];
        let scene = ctx.scene();
        for node in &mut scene.root.children {
            if let Some(frost::Shape::Light { light }) = &mut node.shape {
                light.intensity = LAMP_INTENSITY * lamp;
            }
        }
        scene.ambient = frost::Color {
            r: AMBIENT.r * ambient,
            g: AMBIENT.g * ambient,
            b: AMBIENT.b * ambient,
            a: AMBIENT.a,
        };

        log::trace!("process: t {} eyes {}", self.t, self.eyes.len());
    }
}

/// A pseudo-random value in `0..1` from an index — the eyes' starting
/// positions and phases, fixed so every run shows the same hall, and the
/// same hall the two parent examples walk.
fn hash(i: usize, salt: usize) -> f32 {
    let x = ((i + 1) * 374761393 + salt * 668265263) as u64;
    let x = (x ^ (x >> 13)).wrapping_mul(1274126177);
    ((x ^ (x >> 16)) as u32 % 1000) as f32 / 1000.0
}

/// A wall node: unlit so it draws in its own color, occluding against
/// every light — the hall's lamp and the lamp-eyes alike. It is also the
/// reason the halo and the pool come apart, so it is the node to watch.
fn wall(center: [f32; 2], extent: [f32; 2], tilt: f32) -> Box<frost::SceneNode> {
    Box::new(frost::SceneNode {
        transform: frost::Transform::rotate(tilt).compose(&frost::Transform::translate(center)),
        shape: Some(frost::Shape::Rectangle {
            center: [0.0, 0.0],
            extent,
            color: WALL,
        }),
        lit: false,
        occludes: true,
        ..Default::default()
    })
}

/// A lit panel near the room's edges: the surfaces the lamp-eyes sweep.
fn panel(center: [f32; 2], extent: [f32; 2], color: frost::Color) -> Box<frost::SceneNode> {
    Box::new(frost::SceneNode {
        lit: true,
        shape: Some(frost::Shape::Rectangle {
            center: [0.0, 0.0],
            extent,
            color,
        }),
        transform: frost::Transform::translate(center),
        ..Default::default()
    })
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    let root = std::env!("CARGO_MANIFEST_DIR");
    let eye_sprite = frost::Shape::sprite(format!("{root}/{EYE}"))
        .expect("failed to load assets/sprites/Waspeye1.png");

    // Six eyes: every other one is a lamp-eye — larger, and carrying
    // the child light. Keeping both kinds on stage is what lets one
    // window show the halo that has a pool and the one that has none.
    let eyes = (0..6)
        .map(|i| Eye {
            pos: [(hash(i, 1) - 0.5) * 900.0, (hash(i, 2) - 0.5) * 520.0],
            heading: hash(i, 3) * std::f32::consts::TAU,
            speed: MIN_SPEED + (MAX_SPEED - MIN_SPEED) * hash(i, 4),
            wander_a: 0.5 + 0.7 * hash(i, 5),
            wander_b: 1.3 + 1.1 * hash(i, 6),
            phase_a: hash(i, 7) * std::f32::consts::TAU,
            phase_b: hash(i, 8) * std::f32::consts::TAU,
            pulse_freq: 0.7 + 1.6 * hash(i, 9),
            pulse_phase: hash(i, 10) * std::f32::consts::TAU,
            // Lamp-eyes are the larger ones — half a hint, so the two
            // kinds read as the same species at a glance.
            scale: 0.22 + 0.2 * hash(i, 11) + if i % 2 == 0 { 0.08 } else { 0.0 },
            facing: 1.0,
            lamp: i % 2 == 0,
            ..Eye::default()
        })
        .collect::<Vec<_>>();

    // Eyes first among the root's children — the process drives them as
    // the leading children, one per eye — and on top of the room by
    // their `order`.
    // A lamp-eye's subtree is the sprite node with its light child; the
    // light rides the eye for free.
    let eye_nodes = eyes
        .iter()
        .map(|eye| {
            let mut node = frost::SceneNode {
                order: 1.0,
                lit: true,
                emission: EYE_COLOR,
                shape: Some(eye_sprite.clone()),
                ..Default::default()
            };
            if eye.lamp {
                node.children.push(Box::new(frost::SceneNode {
                    shape: Some(frost::Shape::Light {
                        light: frost::Light::point(EYE_COLOR, 0.5, EYE_LIGHT_RADIUS)
                            .with_penumbra(EYE_PENUMBRA),
                    }),
                    ..Default::default()
                }));
            }
            Box::new(node)
        })
        .collect::<Vec<_>>();

    let mut scene = frost::Scene::new(frost::SceneNode {
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.02,
                g: 0.02,
                b: 0.035,
                a: 1.0,
            },
        }),
        ..Default::default()
    });
    scene.root.children = eye_nodes;
    scene.root.children.extend([
        // The room: a huge lit backdrop, three colored panels near its
        // edges for the lamp-eyes to sweep, the hall's one dim lamp, and
        // two occluder walls to cut everything — including the eyes' own
        // light, and only their light.
        Box::new(frost::SceneNode {
            order: -1.0,
            lit: true,
            shape: Some(frost::Shape::Rectangle {
                center: [0.0, 0.0],
                extent: [2000.0, 2000.0],
                color: BACKDROP,
            }),
            ..Default::default()
        }),
        Box::new(frost::SceneNode {
            shape: Some(frost::Shape::Light {
                light: frost::Light::point(LAMP, LAMP_INTENSITY, LAMP_RADIUS),
            }),
            ..Default::default()
        }),
        panel([-430.0, 190.0], [90.0, 55.0], ROSE),
        panel([60.0, -230.0], [110.0, 40.0], TEAL),
        panel([430.0, 150.0], [70.0, 80.0], CREAM),
        wall([-260.0, -20.0], [24.0, 150.0], 0.0),
        wall([240.0, -60.0], [24.0, 170.0], -0.35),
    ]);
    // Far below any light's reach, so unlit corners read as the void —
    // and the process scales it with the lamp when `D` dims the hall.
    scene.ambient = AMBIENT;

    match frost::run_configured(
        scene,
        EyesLightBloom {
            eyes,
            bloom_on: true,
            tone_on: true,
            ..EyesLightBloom::default()
        },
        frost::Config {
            bloom: BLOOM_STRENGTH,
            tone_map: HEADROOM,
            ..Default::default()
        },
    ) {
        Ok(()) => {}
        Err(err) => {
            log::error!("frost exited with an error: {err}");
            std::process::exit(1);
        }
    }
}
