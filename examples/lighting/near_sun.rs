//! The same day as `dawn`, told by a *near* sun: the light is a point
//! light pinned to the visible disk, so the shadows fan out from the
//! sun you can actually see.
//!
//! A [`Light::directional`] is the honest model of a real sun — but
//! honest and visible disagree: its shadows are parallel, while the
//! drawn disk is a point in the sky, and eyes read points as sources.
//! The alternative is already in the engine: a point light with a
//! generous radius, *translated to the disk's own position* every
//! frame. Now light and face are literally the same node position, and
//! the scene gets everything perspective implies:
//!
//! * every pillar's shadow points exactly away from the disk, and the
//!   shadows **fan** — adjacent pillars' shadows splay apart instead
//!   of lying perfectly parallel;
//! * the disk moves, and every shadow on screen rotates to follow it,
//!   around the scene like spokes on a wheel;
//! * penumbrae are now geometric in the fullest sense: near an
//!   occluder its shadow edge is crisp, farther behind it the same
//!   edge has widened — the disk's angular size shrinking with
//!   distance, exactly as a real light's penumbra behaves.
//!
//! The price is what `directional` exists to avoid: a point light
//! falls off, so the side of the scene nearer the disk sits a little
//! brighter than the far side (at `SUN_RANGE`, a gentle gradient
//! across a window rather than a visible dimming), and the light has
//! a position to maintain — below the horizon, its strength is cut to
//! zero outright, since a sun shining up through the floor is a
//! different kind of off.
//!
//! Run side by side with `cargo run --example dawn` to watch the
//! difference: parallel spears against fanning spokes, same sunrise.
//!
//! ```text
//! cargo run --example near_sun
//! ```

use frost::{Color, Light, Process, Scene, SceneNode, Shape, Transform};

/// Seconds for a full sunrise-to-sunset loop: the same clock as
/// `dawn`, so the two days run in step.
const DAY: f32 = 64.0;

/// How far the near sun's light reaches: a radius so generous that
/// the linear falloff only ever draws its last few percent across a
/// window — the gradient that gives the scene its near-side warmth,
/// and the whole cost of the perspective.
const SUN_RANGE: f32 = 18000.0;

/// The sun's softness — the disk it shines from, in pixels. On a
/// source this near, it behaves completely geometrically: a shadow's
/// edge is crisp where the occluder is close and spreads as the
/// shadow falls farther behind it.
const SUN_PENUMBRA: f32 = 26.0;

/// The sun's elevation (radians above the horizon) sweeping linearly
/// across the day, starting and ending just below it — strength is
/// zero at both ends, so the overnight jump back is invisible.
const SUN_RISE: f32 = -0.18;
const SUN_SET: f32 = std::f32::consts::PI + 0.18;

/// The horizon line, in user pixels: where the floor's top edge lies
/// and where the sun's path stands.
const HORIZON: f32 = -260.0;

/// The visible sun — which, unlike `dawn`'s staged face, is now also
/// the light's own position: core disk and two halo rings, all
/// unlit, burning in their own color.
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

/// The day's score — the same waypoints as `dawn`, so the two suns
/// rise and set in lockstep.
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
/// between. Past the last waypoint, the night key stands.
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
struct NearSun {
    t: f32,
}

impl Process for NearSun {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        let dt = dt.min(1.0 / 30.0);
        self.t += dt;
        let p = (self.t % DAY) / DAY;
        let day = sky_at(p);
        let (w, h) = ctx.size();

        // The sun rides a true circle on the horizon's center — and
        // this time the light rides it too: one position for face and
        // source both.
        let elevation = SUN_RISE + (SUN_SET - SUN_RISE) * p;
        let sun_r = w.min(h) / 2.0 - 60.0;
        let sun_pos = [elevation.cos() * sun_r, HORIZON + elevation.sin() * sun_r];
        // A point light has a position to be honest about: below the
        // horizon it would shine up through the floor, so its
        // strength closes as the sun sinks, well before it goes under.
        let above = (elevation * 4.0).clamp(0.0, 1.0);
        let scene = ctx.scene();
        let sun = &mut scene.root.children[0];
        sun.transform = Transform::translate(sun_pos);
        sun.shape = Some(Shape::Light {
            light: Light::point(day.sun, day.intensity * above * above, SUN_RANGE)
                .with_penumbra(SUN_PENUMBRA),
        });

        // The face follows its source to the pixel: the halo rings and
        // the light's own penumbra disk are the same ball of light now.
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

        // The sky and the world, as in `dawn`.
        scene.root.shape = Some(Shape::Background { color: day.sky });
        scene.ambient = day.ambient;

        log::trace!("process: day {p:.3}");
    }
}

/// A pillar: unlit, so it draws in its own color, and occluding — its
/// shadow one spoke in the fan the near sun turns behind it.
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

/// A lit panel standing on the floor beyond the pillars: its nearer
/// edge catches a touch more of the near sun than its farther one —
/// the falloff gradient, made visible on a single surface.
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

/// The sun's face as it starts: three unlit disks below the ridge's
/// `order`, waiting to be walked up the sky.
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
        // The sun light first — the process drives `children[0]`
        // (position *and* shape now), and [1..4] are its face.
        children: vec![
            Box::new(SceneNode {
                shape: Some(Shape::Light {
                    light: Light::point(
                        Color {
                            r: 1.0,
                            g: 0.6,
                            b: 0.3,
                            a: 1.0,
                        },
                        0.0,
                        SUN_RANGE,
                    )
                    .with_penumbra(SUN_PENUMBRA),
                }),
                ..Default::default()
            }),
            face.next().unwrap(),
            face.next().unwrap(),
            face.next().unwrap(),
            // The floor, its top edge the horizon — lit, and now lit
            // *unevenly*: the end nearer the sun runs warmer than the
            // far end, the near sun's signature across open ground.
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
            // and disappears behind it at dusk. It does not occlude —
            // a ridge between the light and the whole stage would put
            // the world in permanent shadow; it is scenery, and
            // scenery draws, it does not block.
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
            // The sundial's gnomons — their shadows now spokes, each
            // pointing away from the disk, the fan splaying as the
            // sun swings past.
            pillar(-560.0, 260.0, 0.0),
            pillar(-250.0, 340.0, 0.02),
            pillar(60.0, 300.0, 0.0),
            pillar(380.0, 380.0, -0.03),
            pillar(660.0, 240.0, 0.0),
        ],
        ..Default::default()
    });
    scene.ambient = SKY[0].ambient;

    match frost::run(scene, NearSun { t: 0.0 }) {
        Ok(()) => {}
        Err(err) => {
            log::error!("frost exited with an error: {err}");
            std::process::exit(1);
        }
    }
}
