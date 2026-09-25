//! A row of lit panels behind a row of tall occluder walls, with a single
//! lamp orbiting through the walls.
//!
//! Three stone walls stand in a row across the middle of the window, and a
//! bright lit panel rests directly behind each one, so every wall sits
//! between the lamp's orbit and its own panel. Each wall carries both flags:
//! `lit: true, occludes: true`, so light plays across the wall's own surface
//! while its rectangular silhouette cuts a hard shadow into everything
//! behind it.
//!
//! The lamp sweeps a wide ellipse centered on the row. As it travels, each
//! panel cycles through the shadow geometry a receiver sees: fully occluded
//! while the lamp hides behind the wall, corner-lit while the lamp crosses
//! the wall's edge, and fully lit while the lamp swings through the gap.
//! The three panels are never aligned with the lamp at the same moment, so
//! the different stages of that cycle stay visible side by side. The right
//! wall is leaned over: the shadow is cast from the rectangle's full
//! composed transform, so its silhouette leans exactly where the wall does.
//!
//! Two lit circles drift through the gaps between the walls. Only rectangles
//! occlude, so the circles are never in a shadow's path, however squarely
//! the lamp passes behind them, and the huge backdrop is a receiver only:
//! flagging it would shadow the whole scene.
//!
//! Lights are never drawn, so the lamp is marked by two immediate circles: a
//! soft tinted halo and a small white core. Immediate draws are unlit, so
//! the markers keep their full brightness wherever the lamp travels.
//!
//! Run with:
//!
//! ```text
//! cargo run --example occlusion
//! ```

/// The lamp's color: a warm white.
const LAMP: frost::Color = frost::Color {
    r: 1.0,
    g: 0.92,
    b: 0.75,
    a: 1.0,
};

/// The lamp's strength.
const INTENSITY: f32 = 1.8;

/// The lamp's falloff extent, in pixels: wide enough to pool over a panel
/// and its wall from anywhere on the orbit.
const RADIUS: f32 = 380.0;

/// The center of the lamp's elliptical orbit, in the window's user space.
const ORBIT_CENTER: [f32; 2] = [0.0, 40.0];

/// The half-widths of the lamp's elliptical orbit, in pixels: wide enough to
/// pass outside the outer walls, tall enough to swing through the wall row.
const ORBIT_RADII: [f32; 2] = [330.0, 150.0];

/// The lamp's angular speed along the orbit, in radians per second.
const ORBIT_RATE: f32 = 0.45;

/// The lean of the right wall, in radians, counter-clockwise. Its shadow
/// must lean with it.
const WALL_TILT: f32 = 0.32;

/// How much the per-frame breathing pulses the light's falloff extent, as a
/// fraction of [`RADIUS`].
const PULSE: f32 = 0.05;

/// The breathing rate of the pulse, in radians per second.
const PULSE_RATE: f32 = 1.3;

/// The core marker's radius, in pixels.
const CORE_RADIUS: f32 = 5.0;

/// The halo marker's radius, in pixels.
const HALO_RADIUS: f32 = 18.0;

/// The alpha of the halo marker: a soft tint around the core.
const HALO_ALPHA: f32 = 0.3;

struct Demo {
    /// The elapsed time, in seconds: the clock of the orbit and the pulse.
    t: f32,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        self.t += dt;

        // The lamp rides the ellipse, gently breathing so the pool's edge
        // visibly moves even where the shadow geometry stands still.
        let angle = self.t * ORBIT_RATE;
        let pos = [
            ORBIT_CENTER[0] + ORBIT_RADII[0] * angle.cos(),
            ORBIT_CENTER[1] + ORBIT_RADII[1] * angle.sin(),
        ];
        let pulse = 1.0 + PULSE * (self.t * PULSE_RATE).sin();
        ctx.light(pos[0], pos[1], LAMP, INTENSITY, RADIUS * pulse);

        // Its markers: immediate draws are unlit, so they stay bright
        // wherever they travel. The halo takes the lamp's own color at low
        // alpha, the core stays white-hot.
        ctx.circle(
            pos[0],
            pos[1],
            HALO_RADIUS,
            frost::Color {
                a: HALO_ALPHA,
                ..LAMP
            },
            2.0,
        );
        ctx.circle(
            pos[0],
            pos[1],
            CORE_RADIUS,
            frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            2.0,
        );
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    let mut scene = frost::Scene::new(frost::SceneNode {
        // Near-black background: the clear color behind the lit backdrop,
        // visible only outside the huge rectangle's reach.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.03,
                g: 0.03,
                b: 0.05,
                a: 1.0,
            },
        }),
        children: vec![
            Box::new(frost::SceneNode {
                // The backdrop: a huge lit rectangle far beyond the window's
                // edges, so the walls' shadows stretch out over it however
                // low the lamp swings. It is a receiver only: flagging it
                // would shadow the whole scene behind its own silhouette.
                order: -1.0,
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, 0.0],
                    extent: [2000.0, 2000.0],
                    color: frost::Color {
                        r: 0.28,
                        g: 0.28,
                        b: 0.38,
                        a: 1.0,
                    },
                }),
                lit: true,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The left panel, behind the left wall: a lit receiver that
                // the wall takes out of the lamp entirely while the lamp
                // hides behind it. Warm cream so its shadow edge reads hard.
                order: -0.5,
                shape: Some(frost::Shape::Rectangle {
                    center: [-260.0, -196.0],
                    extent: [105.0, 26.0],
                    color: frost::Color {
                        r: 0.85,
                        g: 0.68,
                        b: 0.45,
                        a: 1.0,
                    },
                }),
                lit: true,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The middle panel, behind the middle wall.
                order: -0.5,
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, -196.0],
                    extent: [105.0, 26.0],
                    color: frost::Color {
                        r: 0.55,
                        g: 0.68,
                        b: 0.95,
                        a: 1.0,
                    },
                }),
                lit: true,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The right panel, behind the leaned wall: its shadow band
                // slides and leans as the lamp swings past the tilted edge.
                order: -0.5,
                shape: Some(frost::Shape::Rectangle {
                    center: [260.0, -196.0],
                    extent: [105.0, 26.0],
                    color: frost::Color {
                        r: 0.50,
                        g: 0.85,
                        b: 0.62,
                        a: 1.0,
                    },
                }),
                lit: true,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The left wall: lit on its own face, and the rectangle
                // that cuts the lamp's light off from the left panel.
                shape: Some(frost::Shape::Rectangle {
                    center: [-260.0, 10.0],
                    extent: [26.0, 150.0],
                    color: frost::Color {
                        r: 0.34,
                        g: 0.34,
                        b: 0.42,
                        a: 1.0,
                    },
                }),
                lit: true,
                occludes: true,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The middle wall. The lamp's orbit crosses its band, so
                // this panel cycles through shadow and light every lap.
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, 10.0],
                    extent: [26.0, 150.0],
                    color: frost::Color {
                        r: 0.34,
                        g: 0.34,
                        b: 0.42,
                        a: 1.0,
                    },
                }),
                lit: true,
                occludes: true,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The right wall, leaned by [`WALL_TILT`] about its own
                // center: the shadow follows the composed transform, so
                // this is the tilted-silhouette case.
                transform: frost::Transform::rotate(WALL_TILT)
                    .compose(&frost::Transform::translate([260.0, 10.0])),
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, 0.0],
                    extent: [26.0, 150.0],
                    color: frost::Color {
                        r: 0.34,
                        g: 0.34,
                        b: 0.42,
                        a: 1.0,
                    },
                }),
                lit: true,
                occludes: true,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // A lit circle drifting in the left gap. Circles never
                // occlude, so the lamp shining past it throws no shadow.
                order: 0.4,
                shape: Some(frost::Shape::Circle {
                    center: [-130.0, 70.0],
                    radius: 34.0,
                    color: frost::Color {
                        r: 0.72,
                        g: 0.50,
                        b: 0.62,
                        a: 1.0,
                    },
                }),
                lit: true,
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // A second lit circle, right gap: same proof on the way
                // back.
                order: 0.4,
                shape: Some(frost::Shape::Circle {
                    center: [130.0, 30.0],
                    radius: 26.0,
                    color: frost::Color {
                        r: 0.50,
                        g: 0.72,
                        b: 0.68,
                        a: 1.0,
                    },
                }),
                lit: true,
                ..Default::default()
            }),
        ],
        ..Default::default()
    });
    // The ambient floor of the light field: a dark blue-gray, so the shadow
    // side of a wall stays dim but never fully black.
    scene.ambient = frost::Color {
        r: 0.08,
        g: 0.08,
        b: 0.11,
        a: 1.0,
    };

    if let Err(err) = frost::run(scene, Demo { t: 0.0 }) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
