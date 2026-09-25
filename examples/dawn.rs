//! A day broken by a single directional light: the sun rises, climbs,
//! sets, and the world warms and cools with it — and you can see it.
//!
//! A [`Light::directional`] is light from a source so far away that its
//! rays are parallel: no position, no falloff — only a direction of
//! travel, a color, and a strength. It is never drawn… but nothing
//! stops you from staging a visible face for it: the frame's sun is
//! that one `Shape::Light` node *plus* an unlit disk with two halo
//! rings walked around the sky opposite its travel — where the source
//! of parallel light "is" is yours to place, and here it is placed
//! where it tells the time.
//!
//! The whole day is driven by rewriting one node set per frame: the
//! sun's elevation climbs its arc; the disk — blood-red cresting the
//! ridge at dawn, white-hot at noon — sinks behind it again at dusk;
//! the background itself is the animated sky, keyed from night-navy
//! through dawn-mauve to a noon blue; and the directional light's own
//! color, strength, and the scene's ambient slide with it.
//!
//! Because the rays are parallel, the pillars' shadows are all alike:
//! long, raking spears across the floor at dawn, short stubs at noon,
//! spears again from the other side at dusk — sweeping like the hand
//! of a clock whose dial is the whole ground. The penumbra keeps their
//! edges soft, as a wide sun's shadows are.
//!
//! One full day runs on a `DAY`-second loop. Run with:
//!
//! ```text
//! cargo run --example dawn
//! ```

use frost::{Color, Light, Process, Scene, SceneNode, Shape, Transform};

/// Seconds for a full sunrise-to-sunset loop. Short enough to watch a
/// whole day, slow enough to see the shadows sweep.
const DAY: f32 = 64.0;

/// The sun's softness: its disk, read as an angle at the source's
/// impossible distance — generous values here, so the long dawn and
/// dusk shadows are visibly soft and the noon ones crisp.
const SUN_PENUMBRA: f32 = 26.0;

/// The sun's elevation (radians above the horizon) sweeping linearly
/// across the day: it starts just below the eastern horizon and ends
/// just below the western one — and since strength is zero at both
/// ends, the overnight jump back is invisible.
const SUN_RISE: f32 = -0.18;
const SUN_SET: f32 = std::f32::consts::PI + 0.18;

/// The horizon line, in user pixels: where the floor's top edge lies.
/// The sun's path is an ellipse standing on it, so a zero-elevation
/// sun sits exactly at the horizon, half-sunk behind the ridge.
const HORIZON: f32 = -260.0;

/// The visible sun: a core disk and two halo rings. Unlit, so they
/// burn in their own color whatever the light field says — a face has
/// no business being in shadow. The rings' alpha stacks over the dark
/// sky into a glow, and the sun's color carries through them: a red
/// dawn sun haloed in red.
const SUN_RADIUS: f32 = 42.0;

/// One waypoint of the day: its moment (0..1), the sun's color and
/// strength then, the sky's color behind it all, and the ambient the
/// world sits in.
#[derive(Clone, Copy)]
struct Sky {
    at: f32,
    sun: Color,
    intensity: f32,
    sky: Color,
    ambient: Color,
}

/// The day's score, written as waypoints between which everything
/// eases: night giving way to a low red sun, the morning gold, the
/// high white noon, the sinking red again, and the dark closing in.
const SKY: [Sky; 5] = [
    Sky {
        at: 0.0,
        sun: Color {
            r: 0.9,
            g: 0.22,
            b: 0.08,
            a: 1.0,
        },
        intensity: 0.0,
        sky: Color {
            r: 0.03,
            g: 0.035,
            b: 0.07,
            a: 1.0,
        },
        ambient: Color {
            r: 0.025,
            g: 0.03,
            b: 0.055,
            a: 1.0,
        },
    },
    Sky {
        at: 0.16,
        sun: Color {
            r: 1.0,
            g: 0.34,
            b: 0.12,
            a: 1.0,
        },
        intensity: 0.95,
        sky: Color {
            r: 0.36,
            g: 0.2,
            b: 0.24,
            a: 1.0,
        },
        ambient: Color {
            r: 0.085,
            g: 0.06,
            b: 0.085,
            a: 1.0,
        },
    },
    Sky {
        at: 0.38,
        sun: Color {
            r: 1.0,
            g: 0.62,
            b: 0.3,
            a: 1.0,
        },
        intensity: 1.35,
        sky: Color {
            r: 0.3,
            g: 0.42,
            b: 0.6,
            a: 1.0,
        },
        ambient: Color {
            r: 0.13,
            g: 0.11,
            b: 0.11,
            a: 1.0,
        },
    },
    Sky {
        at: 0.62,
        sun: Color {
            r: 1.0,
            g: 0.86,
            b: 0.66,
            a: 1.0,
        },
        intensity: 1.5,
        sky: Color {
            r: 0.46,
            g: 0.62,
            b: 0.84,
            a: 1.0,
        },
        ambient: Color {
            r: 0.165,
            g: 0.155,
            b: 0.145,
            a: 1.0,
        },
    },
    Sky {
        at: 0.9,
        sun: Color {
            r: 0.95,
            g: 0.3,
            b: 0.1,
            a: 1.0,
        },
        intensity: 0.35,
        sky: Color {
            r: 0.32,
            g: 0.17,
            b: 0.21,
            a: 1.0,
        },
        ambient: Color {
            r: 0.06,
            g: 0.05,
            b: 0.075,
            a: 1.0,
        },
    },
];

/// Easing between waypoints: smoothstep, so the day never snaps a
/// corner at a key.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The day at fraction `p` in 0..1: the bracketing waypoints, eased
/// between. Past the last waypoint, the night key stands — the sun is
/// down, and the loop's wrap back to dawn happens unseen.
fn sky_at(p: f32) -> Sky {
    let mut prev = &SKY[0];
    for key in &SKY[1..] {
        if p < key.at {
            let k = ease((p - prev.at) / (key.at - prev.at));
            return Sky {
                at: p,
                sun: prev.sun.lerp(key.sun, k),
                intensity: prev.intensity + (key.intensity - prev.intensity) * k,
                sky: prev.sky.lerp(key.sky, k),
                ambient: prev.ambient.lerp(key.ambient, k),
            };
        }
        prev = key;
    }
    Sky { at: p, ..SKY[0] }
}

/// The demo: the clock the day runs on.
struct Dawn {
    t: f32,
}

impl Process for Dawn {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let dt = dt.min(1.0 / 30.0);
        self.t += dt;
        let p = (self.t % DAY) / DAY;
        let day = sky_at(p);
        let (w, h) = ctx.size();

        // The sun's elevation sweeps its arc; the light *travels* the
        // opposite way, so its direction is the elevation plus PI — and
        // the visible face sits the other way again: right back at the
        // elevation, out on the sky.
        let elevation = SUN_RISE + (SUN_SET - SUN_RISE) * p;
        let scene = ctx.scene();
        scene.root.children[0].shape = Some(Shape::Light {
            light: Light::directional(day.sun, day.intensity, elevation + std::f32::consts::PI)
                .with_penumbra(SUN_PENUMBRA),
        });

        // The face: a true circle standing on the horizon's center,
        // sized to the window's shorter axis so the whole arc stays in
        // view — and a *circle*, deliberately: the parallel shadows
        // point exactly along the sun's angle, so the face must ride
        // that same angle or the sun visibly drifts off the axis its
        // own shadows point away from. (An ellipse would buy height on
        // a wide window at the cost of exactly that alignment.) Below
        // the horizon the ridge and the floor simply cover it — an
        // unlit node hides behind geometry like any other, so "the sun
        // has set" needs no special case.
        let sun_r = w.min(h) / 2.0 - 60.0;
        let sun_pos = [elevation.cos() * sun_r, HORIZON + elevation.sin() * sun_r];
        let face = [
            (SUN_RADIUS * 2.6, 0.14),
            (SUN_RADIUS * 1.65, 0.32),
            (SUN_RADIUS, 1.0),
        ];
        for (i, (radius, alpha)) in face.into_iter().enumerate() {
            let node = &mut scene.root.children[1 + i];
            node.transform = Transform::translate(sun_pos);
            node.shape = Some(Shape::Circle {
                center: [0.0, 0.0],
                radius,
                color: Color {
                    a: alpha,
                    ..day.sun
                },
            });
        }

        // The sky itself: the background is a node like any other —
        // rewrite it and the whole backdrop turns from night to dawn.
        scene.root.shape = Some(Shape::Background { color: day.sky });
        // The world underneath warms and cools with its sky.
        scene.ambient = day.ambient;

        log::trace!("process: day {p:.3}");
    }
}

/// A pillar: unlit, so it draws in its own color, and occluding — the
/// gnomon throwing this sundial's shadow.
fn pillar(x: f32, height: f32, tilt: f32) -> Box<SceneNode> {
    Box::new(SceneNode {
        transform: Transform::rotate(tilt)
            .compose(&Transform::translate([x, HORIZON + height / 2.0])),
        shape: Some(Shape::Rectangle {
            center: [0.0, 0.0],
            extent: [26.0, height],
            color: Color {
                r: 0.34,
                g: 0.32,
                b: 0.4,
                a: 1.0,
            },
        }),
        lit: false,
        occludes: true,
        ..Default::default()
    })
}

/// A lit panel standing on the floor beyond the pillars: the surface
/// the long shadows climb up at dawn and dusk.
fn panel(x: f32, extent: [f32; 2], color: Color) -> Box<SceneNode> {
    Box::new(SceneNode {
        lit: true,
        transform: Transform::translate([x, HORIZON + extent[1] / 2.0]),
        shape: Some(Shape::Rectangle {
            center: [0.0, 0.0],
            extent,
            color,
        }),
        ..Default::default()
    })
}

/// The visible sun as it starts: three unlit disks waiting at the
/// bottom of the sky to be walked up it. They draw below the floor
/// and the ridge (`order` beneath both), so while below the horizon
/// they simply are not there.
fn sun_face() -> [Box<SceneNode>; 3] {
    let face = |radius: f32| {
        Box::new(SceneNode {
            order: -3.0,
            shape: Some(Shape::Circle {
                center: [0.0, 0.0],
                radius,
                color: Color {
                    r: 0.9,
                    g: 0.22,
                    b: 0.08,
                    a: 1.0,
                },
            }),
            ..Default::default()
        })
    };
    [
        face(SUN_RADIUS * 2.6),
        face(SUN_RADIUS * 1.65),
        face(SUN_RADIUS),
    ]
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    let mut face = sun_face().into_iter();
    let mut scene = Scene::new(SceneNode {
        shape: Some(Shape::Background { color: SKY[0].sky }),
        // The sun light first — the process drives `children[0]`, and
        // [1..4] are its visible face.
        children: vec![
            Box::new(SceneNode {
                shape: Some(Shape::Light {
                    light: Light::directional(
                        Color {
                            r: 1.0,
                            g: 0.6,
                            b: 0.3,
                            a: 1.0,
                        },
                        0.0,
                        std::f32::consts::PI,
                    )
                    .with_penumbra(SUN_PENUMBRA),
                }),
                ..Default::default()
            }),
            face.next().unwrap(),
            face.next().unwrap(),
            face.next().unwrap(),
            // The floor: a wide lit band the whole day sweeps across —
            // its color is deliberately bright, so the difference
            // between the night ambient and the noon sun is the world
            // filling with light. Its top edge is the horizon.
            Box::new(SceneNode {
                order: -2.0,
                lit: true,
                shape: Some(Shape::Rectangle {
                    center: [0.0, HORIZON - 220.0],
                    extent: [2400.0, 440.0],
                    color: Color {
                        r: 0.62,
                        g: 0.55,
                        b: 0.46,
                        a: 1.0,
                    },
                }),
                ..Default::default()
            }),
            // The far ridge on the horizon: the sun crests it at dawn
            // and disappears behind it at dusk.
            Box::new(SceneNode {
                order: -1.0,
                lit: true,
                shape: Some(Shape::Rectangle {
                    center: [0.0, HORIZON + 90.0],
                    extent: [2400.0, 180.0],
                    color: Color {
                        r: 0.3,
                        g: 0.32,
                        b: 0.44,
                        a: 1.0,
                    },
                }),
                ..Default::default()
            }),
            panel(
                -640.0,
                [110.0, 64.0],
                Color {
                    r: 0.9,
                    g: 0.5,
                    b: 0.6,
                    a: 1.0,
                },
            ),
            panel(
                560.0,
                [80.0, 96.0],
                Color {
                    r: 0.5,
                    g: 0.85,
                    b: 0.62,
                    a: 1.0,
                },
            ),
            // The sundial's gnomons.
            pillar(-560.0, 260.0, 0.0),
            pillar(-250.0, 340.0, 0.02),
            pillar(60.0, 300.0, 0.0),
            pillar(380.0, 380.0, -0.03),
            pillar(660.0, 240.0, 0.0),
        ],
        ..Default::default()
    });
    scene.ambient = SKY[0].ambient;

    match frost::run(scene, Dawn { t: 0.0 }) {
        Ok(()) => {}
        Err(err) => {
            log::error!("frost exited with an error: {err}");
            std::process::exit(1);
        }
    }
}
