//! A single worm crawling left to right on a single sprite.
//!
//! The scene is one node holding the one `Worm1_crop.png` sprite, and the
//! crawl is drawn with nothing but the node's position and its x scale —
//! no animation frames, no extra body parts. Once per [PERIOD] the x
//! scale breathes between [S_MIN] and [S_MAX], and that same breath drives
//! the position, illustrating the peristaltic stride:
//!
//! - **Stretch** — the x scale rises from [S_MIN] to [S_MAX]: the rear
//!   end grips the soil and stays put while the front end reaches
//!   forward, and the node's center surges after it.
//! - **Contract** — the x scale falls back to [S_MIN]: now the front end
//!   holds while the rear end slides forward to meet it.
//!
//! A full stretch-contract cycle is one stride of exactly [WORM_LEN] ×
//! ([S_MAX] − [S_MIN]) pixels — the difference between the stretched and
//! the contracted body length — and the center's speed is continuous
//! throughout, zero at each end of the beat, so the worm surges and
//! settles twice per stride, like a real peristaltic crawl. The worm
//! wraps from the right edge back to the left, so the crawl loops
//! forever. Run with:
//!
//! ```text
//! cargo run --example worm
//! ```

/// The native width of `Worm1_crop.png`, in texture pixels: the base
/// scale divides [WORM_LEN] by it to render the body at its neutral
/// length.
const SPRITE_WIDTH: f32 = 413.0;

/// The worm's rendered body length at neutral scale, in scene pixels.
const WORM_LEN: f32 = 240.0;

/// The peristaltic beat's amplitude: the body's x scale sweeps
/// [1.0 - AMP] (contracted) to [1.0 + AMP] (stretched) of the neutral
/// length, once per [PERIOD].
const AMP: f32 = 0.2;

/// One full stride — stretch plus contract — in seconds.
const PERIOD: f32 = 1.0;

/// The beat's contracted extreme.
const S_MIN: f32 = 1.0 - AMP;

/// The beat's stretched extreme.
const S_MAX: f32 = 1.0 + AMP;

/// The ground gained per stride, in scene pixels: stretching the body
/// from [S_MIN] to [S_MAX] reaches exactly [WORM_LEN] × ([S_MAX] −
/// [S_MIN]) farther than the contracted body holds.
const STRIDE: f32 = WORM_LEN * (S_MAX - S_MIN);

/// How far past the window edge the worm's front end must be before the
/// wrap shifts it back in from the left.
const MARGIN: f32 = 40.0;

/// The worm's starting offset: at the default 960 px window, the front
/// end begins [MARGIN] pixels off the left edge, so the crawl starts
/// with the worm entering the scene.
const START_OFFSET: f32 = -712.0;

/// The worm's centerline height, in scene pixels (y up, origin at the
/// window's center).
const GROUND_Y: f32 = -160.0;

/// The worm node's index among the root's children: the soil band is the
/// first child (the earlier sibling, so it draws under the worm), the worm
/// the second, and the caption the third.
const WORM_INDEX: usize = 1;

/// The demo's per-frame state.
#[derive(Default)]
struct Demo {
    /// Elapsed time, in seconds.
    t: f32,
    /// The position offset added to the analytic stride: shifted left by
    /// one screen each time the worm leaves the right edge.
    offset: f32,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        self.t += dt;

        // The peristaltic beat: the body's x scale starts contracted
        // (S_MIN) at each stride's beginning, sweeps to S_MAX (the stretch
        // half), and falls back to S_MIN (the contract half).
        let cycle = self.t / PERIOD;
        let u = cycle.fract();
        let s = 1.0 - AMP * (2.0 * std::f32::consts::PI * u).cos();

        // The stride index, and this frame's half of it.
        let k = cycle.floor();

        // The rear end's position. During the stretch (u < 0.5) it grips
        // the soil at the stride's start point; during the contract the
        // front end holds instead, so the rear slides forward by exactly
        // how much the body has shortened since full stretch.
        let rear = if u < 0.5 {
            k * STRIDE
        } else {
            k * STRIDE + WORM_LEN * (S_MAX - s)
        };

        // The sprite is centered on the node's origin, so the node's
        // center sits half a (scaled) body length ahead of the rear end.
        let x = rear + self.offset + WORM_LEN * s / 2.0;

        // Wrap: when the front end leaves the right edge, shift the whole
        // walk left so the worm re-enters from the left, fully off
        // screen.
        // `inner_size()` reports the client area in *physical* pixels, so
        // divide by the scale factor for scene (logical) pixels.
        let half_w = ctx
            .window()
            .map(|w| {
                let inner = w.inner_size();
                (inner.width as f64 / w.scale_factor()) as f32 / 2.0
            })
            .unwrap_or(480.0);
        let front = x + WORM_LEN * s / 2.0;
        if front > half_w + MARGIN {
            self.offset -= front + half_w + MARGIN;
        }

        log::trace!("worm: x {x}, beat {s}");
        let worm = &mut ctx.scene().root.children[WORM_INDEX];
        // A 2 px bob at twice the beat's rate keeps the crawl lively.
        let bob = 2.0 * (4.0 * std::f32::consts::PI * u).sin();
        worm.transform = frost::Transform::translate([x, GROUND_Y + bob]);
        // The base scale maps the 413 px sprite onto [WORM_LEN]; the beat
        // multiplies only the x axis, so the body's length pulses while
        // its thickness stays put.
        let base = WORM_LEN / SPRITE_WIDTH;
        worm.scale = [base * s, base];
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset paths to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let worm = frost::Shape::sprite(format!("{root}/assets/sprites/Worm1_crop.png"))
        .expect("failed to load assets/sprites/Worm1_crop.png");
    let caption = match frost::Shape::text(
        format!("{root}/assets/fonts/JameGem08_2026-Regular.ttf"),
        "one sprite, no frames: stretch to reach, contract to pull",
        20.0,
    ) {
        Ok(shape) => shape,
        Err(err) => {
            log::error!("failed to load the font: {err}");
            std::process::exit(1);
        }
    };

    let scene = frost::Scene::new(frost::SceneNode {
        // Dark soil; the node's transform is ignored.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.10,
                g: 0.08,
                b: 0.06,
                a: 1.0,
            },
        }),
        children: vec![
            Box::new(frost::SceneNode {
                // The soil band the worm crawls on: a very wide rectangle
                // whose top edge sits just below the worm's belly, so the
                // body sinks a few pixels into the ground.
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, GROUND_Y - 90.0],
                    extent: [10_000.0, 60.0],
                    color: frost::Color {
                        r: 0.17,
                        g: 0.13,
                        b: 0.08,
                        a: 1.0,
                    },
                }),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                // The worm: the only moving shape, driven purely by its
                // position and x scale in process().
                transform: frost::Transform::translate([0.0, GROUND_Y]),
                shape: Some(worm),
                ..Default::default()
            }),
            Box::new(frost::SceneNode {
                transform: frost::Transform::translate([0.0, 230.0]),
                shape: Some(caption),
                ..Default::default()
            }),
        ],
        ..Default::default()
    });

    if let Err(err) = frost::run_configured(
        scene,
        Demo {
            offset: START_OFFSET,
            ..Demo::default()
        },
        frost::Config {
            window_size: Some([960, 540]),
            ..Default::default()
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
