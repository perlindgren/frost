//! Self-lit eyes drifting through a dark hall: a tour of the node-level
//! [`emission`](frost::SceneNode::emission) — a surface's own emission.
//!
//! The hall is a lit backdrop, a few lit panels, and an occluder wall,
//! all under one dim lamp near the center — just enough light to read
//! the room. The eyes are `Waspeye1.png` sprites with `lit: true` and
//! a warm emission, so each is drawn as
//! `texture * (ambient + lamp + emission)`: the lamp shapes them where it
//! reaches, and the emission is the floor of light that keeps them burning
//! where it does not. Each eye breathes its emission in and out on its own
//! slow pulse.
//!
//! Emission is receiver-side self-light, and the hall is built to show what
//! that means. Emission does not light the room around an eye, and it is
//! never shadowed: send an eye behind the wall, into the lamp's shadow,
//! and it keeps burning at full strength while the wall and everything
//! behind it drop to the ambient floor. (To light the neighborhood too,
//! an emitter would need a real [`Shape::Light`](frost::Shape::Light)
//! child node — emission is only the surface's own pixels.)
//!
//! The eyes wander on their own smooth-curved headings, flip to face
//! where they are going, and bounce off the window edges.
//!
//! The hall is also where the two look knobs get their workout, because it is
//! the scene in the tree that has light past white in it. The frame starts in
//! bloom and folded; **B** turns the glow off and on, **T** the fold, and **D**
//! dims the hall itself in three steps.
//!
//! ```text
//! cargo run --example eyes
//! ```
//!
//! What each one can and cannot do here follows from what it is, and the keys
//! are there to see exactly that. The glow spills the eyes' amber into the dark
//! around them — emission is what the chain reads, and this hall has nothing
//! else emissive, so the eyes are the only thing that blooms. The fold touches
//! the pixels that were already too bright for the display and nothing else:
//! the backdrop under the lamp comes to about a third of white, far under the
//! knee a headroom of 0.15 puts at 0.85, so it is byte for byte the same room
//! whether the fold is on or off, while the eyes' cores, which pass 1.0 on the
//! peak of every pulse, arrive as amber instead of as white.
//!
//! **D** is there because the question "would the glow be more striking in a
//! darker hall" has a measured answer, and it is: more striking, by ratio and
//! not by amount. Taking the room's light away changes not one bit of what the
//! glow adds — the emissive channel has no light term in it — but the same
//! added light is a lift of seventy percent on a wall at 0.03 and of ten percent
//! on one at 0.28, which is the difference between a halo and a rumour. The
//! darkest step is deliberately not black: the wide levels of the chain lift
//! every pixel in the frame a little, and in a hall that is nearly black that
//! lift is what you see first.

/// The eyes' texture: a ~200x180 RGBA sprite — black outline, amber
/// iris, pale sclera — which the emission shows through: emission rides the
/// texture, so the ink stays dark and the iris burns.
const EYE: &str = "assets/sprites/Waspeye1.png";

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

/// The eyes' emission: the sprite's own amber, pushed past full so the
/// pulse can dim from over-bright down to a dying ember.
const EYE_EMISSION: frost::Color = frost::Color {
    r: 1.0,
    g: 0.55,
    b: 0.15,
    a: 1.0,
};

/// The glow's strength and the fold's headroom: the two numbers this example
/// exists to let you look at, both live on `B` and `T` rather than fixed,
/// because the comparison is the point and it is free. 0.15 reserves fifteen
/// percent of the display range for everything above white.
///
/// The strength is deliberately past the restrained end. The chain spreads a
/// source over a fixed number of pixels — about six at the fine level, twelve
/// and twenty-four at the coarser ones — and an eye here is forty to eighty
/// pixels wide in a sixteen-hundred-pixel window, so the halo lands close on a
/// small thing and reads as a smudge at the restrained end. Rendered at the
/// eyes' own emission, five pixels out measured 11, 16 and 21 eighths of full
/// scale at strengths 0.7, 1.4 and 2.8: the middle is the first that looks like
/// a glow rather than a smudge, and there is room above it before the halo
/// starts lighting the room instead of the eye.
const BLOOM_STRENGTH: f32 = 1.6;
const HEADROOM: f32 = 0.15;

/// The hall's ambient floor, as authored — [`ROOM`] scales it along with the
/// lamp, so a dimmed hall is dimmer everywhere and not just under the light.
const AMBIENT: frost::Color = frost::Color {
    r: 0.04,
    g: 0.04,
    b: 0.06,
    a: 1.0,
};

/// The room's lighting levels, on `D`. The lamp goes to nothing at the last
/// step and the ambient to a fifth, because the point is to see the glow
/// against a room rather than against a wall: an emissive surface's reach is a
/// ratio, and a bright hall is a busy denominator. The walls are unlit, so the
/// room's geometry survives its own lights going out, and the eyes' cores do
/// too — a lit node is `texture * (ambient + light + emission)`, and only two
/// of those three terms are on this key.
///
/// The last step keeps some ambient rather than going black for a reason
/// measured this session: the chain's wide levels lift every pixel of the frame
/// by about 0.008 of radiance, which is 3% of a lit room and thirty percent of
/// a nearly black one. Below here the haze is the effect, and the glow is
/// whatever is left of it.
const ROOM: [(f32, f32); 3] = [(1.0, 1.0), (0.5, 0.5), (0.0, 0.2)];

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
/// its wander curve and emission pulse.
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
    /// The emission pulse: `sin(t * pulse_freq + pulse_phase)` breathes the
    /// emission's alpha between a dying ember and an over-bright flare.
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
    /// travel, and breathes its emission.
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

    /// The emission for this instant: the amber's alpha breathing on the
    /// pulse — `0.15..1.15`, embers to flare.
    fn emission_at(&self, t: f32) -> frost::Color {
        let pulse = (t * self.pulse_freq + self.pulse_phase).sin();
        frost::Color {
            a: 0.65 + 0.5 * pulse,
            ..EYE_EMISSION
        }
    }
}

/// Whether `key` was pressed since the last frame. The context reports the
/// held state, which is what a movement wants and not what a toggle wants, so
/// the example remembers last frame's held state itself and looks for the step
/// up: one press, one change, however long the key is held.
fn pressed(ctx: &frost::Context, key: frost::KeyCode, held: &mut bool) -> bool {
    let down = ctx.key_down(key);
    let rose = down && !*held;
    *held = down;
    rose
}

/// The demo: all the eyes, the clock their wander and pulses run on, and the
/// two look knobs with the held state each needs to notice a press.
#[derive(Default)]
struct Eyes {
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

impl frost::Process for Eyes {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let dt = dt.min(1.0 / 30.0);
        self.t += dt;
        let (w, h) = ctx.size();
        let (limit_x, limit_y) = (w / 2.0, h / 2.0);

        // The eyes ride the root's leading children — one transform,
        // scale, and emission write each.
        for (i, eye) in self.eyes.iter_mut().enumerate() {
            eye.step(self.t, dt, limit_x, limit_y);
            let node = &mut ctx.scene().root.children[i];
            node.transform = frost::Transform::translate(eye.pos);
            node.scale = [eye.scale * eye.facing, eye.scale];
            node.emission = eye.emission_at(self.t);
        }

        // The two knobs, read every frame so a held key cannot repeat them,
        // then written every frame: both are per-frame values the engine reads
        // out of the stretch pass's uniform, so writing the same number back is
        // not a change of any kind and costs nothing.
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

        // The room's own light, on the same footing as the look knobs: it is
        // the denominator the glow is judged against, and it is the one thing
        // in here that dims the hall without touching what the eyes emit.
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
/// positions and phases, fixed so every run shows the same hall.
fn hash(i: usize, salt: usize) -> f32 {
    let x = ((i + 1) * 374761393 + salt * 668265263) as u64;
    let x = (x ^ (x >> 13)).wrapping_mul(1274126177);
    ((x ^ (x >> 16)) as u32 % 1000) as f32 / 1000.0
}

/// A wall node: unlit, so it draws in its own color, occluding — and
/// the thing the eyes prove their emission against.
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
        .expect("failed to load assets/sprites/Waspeye1.png");

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
                emission: EYE_EMISSION,
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
    // eyes own — and the process scales it with the lamp when `D` dims the hall.
    scene.ambient = AMBIENT;

    match frost::run_configured(
        scene,
        Eyes {
            eyes,
            bloom_on: true,
            tone_on: true,
            ..Eyes::default()
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
