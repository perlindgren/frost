//! A planet under two stars: one directional light is a sun; two are a
//! binary system — and the stars are on stage to prove it.
//!
//! The light field never cared how many lights a scene holds, and a
//! [`Light::directional`] is just one more record in it — so a world
//! lit from two far-away directions needs nothing special: two
//! `Shape::Light` nodes, each aimed, each colored, each orbiting at
//! its own rate. Every lit pixel receives both stars summed; every
//! occluder cuts each of them separately, so the monoliths throw two
//! families of parallel shadows at once, crossing and recrossing as
//! the stars sweep at their different speeds. Where a shadow from one
//! star lies, the other's light still falls — amber shade meeting
//! ice-blue light, and the umbra where both are blocked going truly
//! dark.
//!
//! A directional light has no position — so where you draw its source
//! is yours to stage. Here each star's face is an unlit core disk with
//! two halo rings (alpha stacked into a glow), walked around a true
//! circle fitted to the window: riding the very angle the shadows fall
//! along, they rise and set behind the planet and the plain, transit
//! its face, and drag their shadow families around with them, always
//! pointing directly away from the visible disk — because the light
//! and the face are driven by the same angle.
//!
//! Run with:
//!
//! ```text
//! cargo run --example twostars
//! ```

use frost::{Color, Light, Process, Scene, SceneNode, Shape, Transform};

/// The amber star: color, strength, and how fast its direction of
/// travel sweeps the sky (radians per second — a slow orbit).
const AMBER: Color = Color {
    r: 1.0,
    g: 0.66,
    b: 0.28,
    a: 1.0,
};
const AMBER_RATE: f32 = 0.055;

/// The ice star: cooler, weaker, sweeping the other way and at a rate
/// the amber one will never sync with — the shadow crossings never
/// repeat on any watchable timescale.
const ICE: Color = Color {
    r: 0.45,
    g: 0.62,
    b: 1.0,
    a: 1.0,
};
const ICE_RATE: f32 = -0.038;

/// Both stars are wide, low disks seen from very far: a generous
/// penumbra, so each family of shadows fades across a soft edge and
/// the crossings blend instead of stacking knife edges.
const STAR_PENUMBRA: f32 = 30.0;

/// A star's whole stage kit: its travel rate, starting phase, color,
/// strength, and the core radius of its visible face.
struct Star {
    rate: f32,
    phase: f32,
    color: Color,
    intensity: f32,
    radius: f32,
}

const STARS: [Star; 2] = [
    Star {
        rate: AMBER_RATE,
        phase: 0.0,
        color: AMBER,
        intensity: 1.25,
        radius: 40.0,
    },
    Star {
        rate: ICE_RATE,
        phase: 2.4,
        color: ICE,
        intensity: 0.95,
        radius: 28.0,
    },
];

/// The demo: the clock the two orbits run on.
struct TwoStars {
    t: f32,
}

impl Process for TwoStars {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let dt = dt.min(1.0 / 30.0);
        self.t += dt;
        let (w, h) = ctx.size();
        let scene = ctx.scene();

        // Each star's travel direction sweeps a full turn at its own
        // rate. The light travels one way; its face — three sky nodes
        // (halo, halo, core) — sits the other way, on a true circle
        // fitted to the window's shorter axis: a circle because the
        // parallel shadow families point exactly along the travel
        // angle, and the face must ride that same angle to sit on the
        // axis they point away from — an ellipse would drift it off
        // that line on any window that isn't square.
        let r = w.min(h) / 2.0 - 70.0;
        for (i, star) in STARS.iter().enumerate() {
            let travel = star.rate * self.t + star.phase;
            scene.root.children[i].shape = Some(Shape::Light {
                light: Light::directional(star.color, star.intensity, travel)
                    .with_penumbra(STAR_PENUMBRA),
            });
            let face = [
                (star.radius * 2.8, 0.12),
                (star.radius * 1.7, 0.30),
                (star.radius, 1.0),
            ];
            for (j, (radius, alpha)) in face.into_iter().enumerate() {
                let node = &mut scene.root.children[2 + i * 3 + j];
                node.transform = Transform::translate([-travel.cos() * r, 20.0 - travel.sin() * r]);
                node.shape = Some(Shape::Circle {
                    center: [0.0, 0.0],
                    radius,
                    color: Color {
                        a: alpha,
                        ..star.color
                    },
                });
            }
        }

        log::trace!("process: t {}", self.t);
    }
}

/// A monolith: unlit, occluding — thrown between the planet and the
/// stars, each one the origin of two parallel shadow families.
fn monolith(center: [f32; 2], extent: [f32; 2], tilt: f32) -> Box<SceneNode> {
    Box::new(SceneNode {
        transform: Transform::rotate(tilt).compose(&Transform::translate(center)),
        shape: Some(Shape::Rectangle {
            center: [0.0, 0.0],
            extent,
            color: Color {
                r: 0.3,
                g: 0.28,
                b: 0.36,
                a: 1.0,
            },
        }),
        lit: false,
        occludes: true,
        ..Default::default()
    })
}

/// One star's visible face: three unlit disks, halo behind halo behind
/// core, all drawing below the plain and the planet so the star
/// "sets" behind them. They burn in their own color whatever the sky
/// does — a face has no business being in shadow.
fn star_face(color: Color, radius: f32) -> [Box<SceneNode>; 3] {
    let face = |scale: f32, alpha: f32| {
        Box::new(SceneNode {
            order: -1.75,
            shape: Some(Shape::Circle {
                center: [0.0, 0.0],
                radius: radius * scale,
                color: Color { a: alpha, ..color },
            }),
            ..Default::default()
        })
    };
    [face(2.8, 0.12), face(1.7, 0.30), face(1.0, 1.0)]
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    let mut amber_face = star_face(AMBER, STARS[0].radius).into_iter();
    let mut ice_face = star_face(ICE, STARS[1].radius).into_iter();
    let mut scene = Scene::new(SceneNode {
        shape: Some(Shape::Background {
            color: Color {
                r: 0.015,
                g: 0.015,
                b: 0.03,
                a: 1.0,
            },
        }),
        children: vec![
            // Two star lights — first two children, rewritten per
            // frame — each followed by its three-piece visible face.
            Box::new(SceneNode {
                shape: Some(Shape::Light {
                    light: Light::directional(AMBER, STARS[0].intensity, STARS[0].phase)
                        .with_penumbra(STAR_PENUMBRA),
                }),
                ..Default::default()
            }),
            Box::new(SceneNode {
                shape: Some(Shape::Light {
                    light: Light::directional(ICE, STARS[1].intensity, STARS[1].phase)
                        .with_penumbra(STAR_PENUMBRA),
                }),
                ..Default::default()
            }),
            amber_face.next().unwrap(),
            amber_face.next().unwrap(),
            amber_face.next().unwrap(),
            ice_face.next().unwrap(),
            ice_face.next().unwrap(),
            ice_face.next().unwrap(),
            // The planet: one big lit receiver for both stars and all
            // their shadows to play across.
            Box::new(SceneNode {
                order: -0.5,
                lit: true,
                shape: Some(Shape::Circle {
                    center: [0.0, -160.0],
                    radius: 300.0,
                    color: Color {
                        r: 0.55,
                        g: 0.5,
                        b: 0.52,
                        a: 1.0,
                    },
                }),
                ..Default::default()
            }),
            // The monoliths standing on it.
            monolith([-210.0, 20.0], [30.0, 170.0], 0.0),
            monolith([-60.0, 90.0], [26.0, 220.0], 0.05),
            monolith([140.0, 60.0], [34.0, 190.0], -0.06),
            monolith([300.0, -10.0], [24.0, 140.0], 0.0),
            // And a plain behind, so the shadow families are visible
            // off the planet too — and a horizon the stars set behind.
            Box::new(SceneNode {
                order: -1.5,
                lit: true,
                shape: Some(Shape::Rectangle {
                    center: [0.0, -520.0],
                    extent: [2400.0, 420.0],
                    color: Color {
                        r: 0.4,
                        g: 0.38,
                        b: 0.44,
                        a: 1.0,
                    },
                }),
                ..Default::default()
            }),
        ],
        ..Default::default()
    });
    // Space itself is nearly dark: everything that reads, reads by
    // starlight.
    scene.ambient = Color {
        r: 0.03,
        g: 0.032,
        b: 0.05,
        a: 1.0,
    };

    match frost::run(scene, TwoStars { t: 0.0 }) {
        Ok(()) => {}
        Err(err) => {
            log::error!("frost exited with an error: {err}");
            std::process::exit(1);
        }
    }
}
