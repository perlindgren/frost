//! The `eyes` example's companion: some eyes are real *emitters*.
//!
//! Same dark hall, same wandering glowing eyes — but the larger eyes
//! carry a `Shape::Light` point light as a **child node**, riding the
//! eye's transform exactly as the torch rides the square in `cone.rs`.
//! Those eyes now do both halves of the emitter story:
//!
//! * their surface glows (the receiver-side `glow`, unshadowable,
//!   local — see the `eyes` example), and
//! * their light spills onto the room: it sweeps across the backdrop
//!   and the colored panels as they wander, its own shadows cut by the
//!   walls — with a penumbra, so those shadows have soft edges.
//!
//! Watch a lamp-eye and a glow-only eye pass the same wall. The
//! glow-only eye keeps burning while the wall's shadow swallows
//! everything behind it; the lamp-eye, however, cannot shine through
//! its own wall — the pool of light it drags across the floor cuts off
//! at the occluder, feathered by its penumbra. Glow is the surface;
//! the child light is the neighborhood.
//!
//! The pulse each eye breathes drives both halves together: as an eye
//! flares, its glow and the pool of light it casts over the room
//! brighten in lockstep. Run with:
//!
//! ```text
//! cargo run --example eyes_light
//! ```

/// The eyes' texture, shared with the `eyes` example: black outline,
/// amber iris, pale sclera.
const EYE: &str = "assets/sprites/Getingeye1.png";

/// The backdrop's color: a cold slate, dim enough to read as a room
/// only where a light reaches it.
const BACKDROP: frost::Color = frost::Color {
    r: 0.3,
    g: 0.3,
    b: 0.4,
    a: 1.0,
};

/// The walls' color: unlit, so their silhouettes — and the shadows
/// they cut, from every light including the eyes' — stay readable.
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

/// The eyes' glow and the light their child node casts: the same
/// amber, so an eye and the pool it drags over the room read as one
/// source. The glow pushes past full so the pulse can over-brighten;
/// the penumbra softens the shadows the eye-light casts.
const EYE_GLOW: frost::Color = frost::Color {
    r: 1.0,
    g: 0.55,
    b: 0.15,
    a: 1.0,
};
const EYE_LIGHT_RADIUS: f32 = 240.0;
const EYE_PENUMBRA: f32 = 5.0;

/// The wandering speed range, in pixels per second, and the rate the
/// heading's wandering curve sweeps.
const MIN_SPEED: f32 = 55.0;
const MAX_SPEED: f32 = 105.0;
const TURN: f32 = 1.3;

/// One eye's state: where it is, where it is heading, the phases of
/// its wander curve and glow pulse, and whether it carries a light.
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
    /// both the glow and (for lamp-eyes) the child light's intensity.
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

    /// The glow for this instant: the amber's alpha breathing on the
    /// pulse — embers to flare.
    fn glow(&self) -> frost::Color {
        frost::Color {
            a: 0.65 + 0.5 * self.pulse,
            ..EYE_GLOW
        }
    }

    /// The child light for this instant: the same amber, its intensity
    /// riding the same pulse, so the pool an eye drags over the room
    /// brightens exactly when the eye itself flares.
    fn light(&self) -> frost::Light {
        frost::Light::point(
            EYE_GLOW,
            (0.5 + 0.55 * self.pulse).max(0.05),
            EYE_LIGHT_RADIUS,
        )
        .with_penumbra(EYE_PENUMBRA)
    }
}

/// The demo: all the eyes, and the clock their wander and pulses run on.
struct EyesLight {
    eyes: Vec<Eye>,
    t: f32,
}

impl frost::Process for EyesLight {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let dt = dt.min(1.0 / 30.0);
        self.t += dt;
        let (w, h) = ctx.size();
        let (limit_x, limit_y) = (w / 2.0, h / 2.0);

        // The eyes ride the root's leading children — one transform,
        // scale, and glow write each, and a light write for the
        // lamp-eyes, whose light is their first child node.
        for (i, eye) in self.eyes.iter_mut().enumerate() {
            eye.step(self.t, dt, limit_x, limit_y);
            let node = &mut ctx.scene().root.children[i];
            node.transform = frost::Transform::translate(eye.pos);
            node.scale = [eye.scale * eye.facing, eye.scale];
            node.glow = eye.glow();
            if eye.lamp {
                // The light rides the parent: only its content changes,
                // the parenting places and pulses it along with the eye.
                node.children[0].shape = Some(frost::Shape::Light { light: eye.light() });
            }
        }

        log::trace!("process: t {} eyes {}", self.t, self.eyes.len());
    }
}

/// A pseudo-random value in `0..1` from an index — the eyes' starting
/// positions and phases, fixed so every run shows the same hall.
fn hash(i: usize, salt: usize) -> f32 {
    let x = ((i + 1) * 374761393 + salt * 668265263) as u64;
    let x = (x ^ (x >> 13)).wrapping_mul(1274126177);
    ((x ^ (x >> 16)) as u32 % 1000) as f32 / 1000.0
}

/// A wall node: unlit so it draws in its own color, occluding against
/// every light — the hall's lamp and the lamp-eyes alike.
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
        .expect("failed to load assets/sprites/Getingeye1.png");

    // Six eyes: every other one is a lamp-eye — larger, and carrying
    // the child light.
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
            pulse: 0.0,
            // Lamp-eyes are the larger ones — half a hint, so the two
            // kinds read as the same species at a glance.
            scale: 0.22 + 0.2 * hash(i, 11) + if i % 2 == 0 { 0.08 } else { 0.0 },
            facing: 1.0,
            lamp: i % 2 == 0,
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
                glow: EYE_GLOW,
                shape: Some(eye_sprite.clone()),
                ..Default::default()
            };
            if eye.lamp {
                node.children.push(Box::new(frost::SceneNode {
                    shape: Some(frost::Shape::Light {
                        light: frost::Light::point(EYE_GLOW, 0.5, EYE_LIGHT_RADIUS)
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
        children: Vec::new(),
        ..Default::default()
    });
    scene.root.children = eye_nodes;
    scene.root.children.extend([
        // The room: a huge lit backdrop, three colored panels near its
        // edges for the lamp-eyes to sweep, the hall's one dim lamp, and
        // two occluder walls to cut everything — including the eyes' own
        // light.
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
    // Far below any light's reach, so unlit corners read as the void.
    scene.ambient = frost::Color {
        r: 0.04,
        g: 0.04,
        b: 0.06,
        a: 1.0,
    };

    match frost::run(scene, EyesLight { eyes, t: 0.0 }) {
        Ok(()) => {}
        Err(err) => {
            log::error!("frost exited with an error: {err}");
            std::process::exit(1);
        }
    }
}
