//! Glowing eyes drifting through a dark hall: a tour of the node-level
//! [`glow`](frost::SceneNode::glow) — a surface's own emission.
//!
//! The hall is a lit backdrop, a few lit panels, and an occluder wall,
//! all under one dim lamp near the center — just enough light to read
//! the room. The eyes are `Getingeye1.png` sprites with `lit: true` and
//! a warm glow, so each is drawn as
//! `texture * (ambient + lamp + glow)`: the lamp shapes them where it
//! reaches, and the glow is the floor of light that keeps them burning
//! where it does not. Each eye breathes its glow in and out on its own
//! slow pulse.
//!
//! Glow is receiver-side self-light, and the hall is built to show what
//! that means. Glow does not light the room around an eye, and it is
//! never shadowed: send an eye behind the wall, into the lamp's shadow,
//! and it keeps burning at full strength while the wall and everything
//! behind it drop to the ambient floor. (To light the neighborhood too,
//! an emitter would need a real [`Shape::Light`](frost::Shape::Light)
//! child node — glow is only the surface's own pixels.)
//!
//! The eyes wander on their own smooth-curved headings, flip to face
//! where they are going, and bounce off the window edges. Run with:
//!
//! ```text
//! cargo run --example eyes
//! ```

/// The eyes' texture: a ~200x180 RGBA sprite — black outline, amber
/// iris, pale sclera — which the glow shows through: emission rides the
/// texture, so the ink stays dark and the iris burns.
const EYE: &str = "assets/sprites/Getingeye1.png";

/// The backdrop's color: a cold slate, dim enough that under the lamp
/// only its middle reads as a room at all.
const BACKDROP: frost::Color = frost::Color {
    r: 0.3,
    g: 0.3,
    b: 0.4,
    a: 1.0,
};

/// The walls' color: darker, unlit — they draw in their own color so
/// their silhouettes, and the shadows they cut, stay readable.
const WALL: frost::Color = frost::Color {
    r: 0.38,
    g: 0.36,
    b: 0.46,
    a: 1.0,
};

/// The lamp's color: a cold moonlight.
const LAMP: frost::Color = frost::Color {
    r: 0.7,
    g: 0.75,
    b: 0.9,
    a: 1.0,
};

/// The eyes' glow: the sprite's own amber, pushed past full so the
/// pulse can dim from over-bright down to a dying ember.
const EYE_GLOW: frost::Color = frost::Color {
    r: 1.0,
    g: 0.55,
    b: 0.15,
    a: 1.0,
};

/// The lamp's reach, in pixels, and its strength: deliberately low —
/// the hall should feel dark, and the eyes should clearly outshine it.
const LAMP_RADIUS: f32 = 650.0;
const LAMP_INTENSITY: f32 = 1.1;

/// The wandering speed range, in pixels per second, and the rate the
/// heading's wandering curve sweeps.
const MIN_SPEED: f32 = 55.0;
const MAX_SPEED: f32 = 105.0;
const TURN: f32 = 1.3;

/// One eye's state: where it is, where it is heading, and the phases of
/// its wander curve and glow pulse.
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
    /// The glow pulse: `sin(t * pulse_freq + pulse_phase)` breathes the
    /// glow's alpha between a dying ember and an over-bright flare.
    pulse_freq: f32,
    pulse_phase: f32,
    /// Sprite scale (the texture is ~200 px wide; eyes are smaller than
    /// walls) and the horizontal flip toward the direction of travel.
    scale: f32,
    facing: f32,
}

impl Eye {
    /// Advances one eye: the heading drifts on its wander curve, the eye
    /// walks and bounces off the window edges, flips to face its
    /// travel, and breathes its glow.
    fn step(&mut self, t: f32, dt: f32, limit_x: f32, limit_y: f32) {
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
    /// pulse — `0.15..1.15`, embers to flare.
    fn glow_at(&self, t: f32) -> frost::Color {
        let pulse = (t * self.pulse_freq + self.pulse_phase).sin();
        frost::Color {
            a: 0.65 + 0.5 * pulse,
            ..EYE_GLOW
        }
    }
}

/// The demo: all the eyes, and the clock their wander and pulses run on.
struct Eyes {
    eyes: Vec<Eye>,
    t: f32,
}

impl frost::Process for Eyes {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let dt = dt.min(1.0 / 30.0);
        self.t += dt;
        let (w, h) = ctx.size();
        let (limit_x, limit_y) = (w / 2.0, h / 2.0);

        // The eyes ride the root's leading children — one transform,
        // scale, and glow write each.
        for (i, eye) in self.eyes.iter_mut().enumerate() {
            eye.step(self.t, dt, limit_x, limit_y);
            let node = &mut ctx.scene().root.children[i];
            node.transform = frost::Transform::translate(eye.pos);
            node.scale = [eye.scale * eye.facing, eye.scale];
            node.glow = eye.glow_at(self.t);
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

/// A wall node: unlit, so it draws in its own color, occluding — and
/// the thing the eyes prove their glow against.
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

fn main() {
    env_logger::init();
    log::info!("frost started");

    let root = std::env!("CARGO_MANIFEST_DIR");
    let eye = frost::Shape::sprite(format!("{root}/{EYE}"))
        .expect("failed to load assets/sprites/Getingeye1.png");

    // Six eyes, each with its own place, wander curve, pulse, and size.
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
            scale: 0.22 + 0.2 * hash(i, 11),
            facing: 1.0,
        })
        .collect::<Vec<_>>();
    let eye_count = eyes.len();

    // Eyes first among the root's children — the process drives them as
    // `children[0..eye_count]` — and on top of the room by their `order`.
    let eye_nodes = (0..eye_count)
        .map(|_| {
            Box::new(frost::SceneNode {
                order: 1.0,
                lit: true,
                glow: EYE_GLOW,
                shape: Some(eye.clone()),
                ..Default::default()
            })
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
        // The room: a huge lit backdrop the lamp barely reaches, a few
        // lit panels near its edges, and two occluder walls between the
        // lamp and the dark.
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
            // The lamp: one dim omni light at the hall's heart.
            shape: Some(frost::Shape::Light {
                light: frost::Light::point(LAMP, LAMP_INTENSITY, LAMP_RADIUS),
            }),
            ..Default::default()
        }),
        wall([-330.0, 20.0], [24.0, 150.0], 0.0),
        wall([280.0, -40.0], [24.0, 170.0], -0.35),
    ]);
    // Far below the lamp's reach, so unlit corners read as the void the
    // eyes own.
    scene.ambient = frost::Color {
        r: 0.04,
        g: 0.04,
        b: 0.06,
        a: 1.0,
    };

    match frost::run(scene, Eyes { eyes, t: 0.0 }) {
        Ok(()) => {}
        Err(err) => {
            log::error!("frost exited with an error: {err}");
            std::process::exit(1);
        }
    }
}
