//! The player example with four fixed obstacles added: `SIZE` x `SIZE`
//! squares at random positions (fully inside the visible area) with random
//! colors, all at the default draw order. The player is unchanged from
//! player.rs.
//!
//! The obstacles are generated once, on the first frame — the window size is
//! only known once the first frame runs — and keep their generated positions
//! and colors for the whole run. Each run gets a different set, seeded from
//! the current time.
//!
//! Run with:
//!
//! ```text
//! cargo run --example obstacles
//! ```

/// The full width and height of each obstacle square, in pixels.
const SIZE: f32 = 100.0;

/// How many obstacles the frame has.
const OBSTACLES: usize = 4;

/// A fixed obstacle: a square centered at `pos` in `color`.
struct Obstacle {
    /// The square's center in window-centered pixels.
    pos: [f32; 2],
    color: frost::Color,
}

/// A tiny deterministic random source (splitmix64), so the example needs no
/// external random crate: seeded from the current time, it gives a different
/// obstacle set on each run.
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Self(nanos)
    }

    /// The next uniform value in [0, 1).
    fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u32 << 24) as f32
    }

    /// The next uniform value in [lo, hi]. When `hi <= lo` — the window is
    /// smaller than an obstacle on that axis — the midpoint instead, so the
    /// value stays as central as it can be.
    fn in_range(&mut self, lo: f32, hi: f32) -> f32 {
        if hi <= lo {
            (lo + hi) / 2.0
        } else {
            lo + self.next_f32() * (hi - lo)
        }
    }
}

/// The direction along one axis from two held keys: `+1` if only `positive`
/// is down, `-1` if only `negative` is, `0` if both or neither is.
fn axis(ctx: &frost::Context, positive: frost::KeyCode, negative: frost::KeyCode) -> f32 {
    (ctx.key_down(positive) as i32 - ctx.key_down(negative) as i32) as f32
}

struct Demo {
    /// The button's position in window-centered pixels.
    pos: [f32; 2],
    /// The button's facing angle in radians, counter-clockwise from +x.
    rot: f32,
    /// The four fixed obstacles, generated once from the first frame's
    /// window size and kept unchanged for the whole run.
    obstacles: Option<Vec<Obstacle>>,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // The player, unchanged from player.rs.
        // `Q` turns counter-clockwise (a positive angle), `E` clockwise.
        let drot = axis(ctx, frost::KeyCode::KeyQ, frost::KeyCode::KeyE);
        self.rot += drot * std::f32::consts::TAU * dt;

        // The WASD direction in the button's own frame, rotated into the
        // world by the facing angle. At `rot == 0` this is the unrotated
        // direction, i.e. the original behavior.
        let (sin, cos) = self.rot.sin_cos();
        let dx = axis(ctx, frost::KeyCode::KeyD, frost::KeyCode::KeyA);
        let dy = axis(ctx, frost::KeyCode::KeyW, frost::KeyCode::KeyS);
        self.pos[0] += (dx * cos - dy * sin) * 100.0 * dt;
        self.pos[1] += (dx * sin + dy * cos) * 100.0 * dt;

        let button = &mut ctx.scene().root.children[0];
        // Rotate about the button's center, then place it at `pos`.
        button.transform = frost::Transform::rotate(self.rot)
            .compose(&frost::Transform::translate(self.pos[0], self.pos[1]));

        // The obstacles: generated once, at random positions with the whole
        // square inside the visible area, in random colors.
        let (w, h) = ctx.size();
        let obstacles = self.obstacles.get_or_insert_with(|| {
            let mut rng = Rng::new();
            (0..OBSTACLES)
                .map(|_| {
                    let half = SIZE / 2.0;
                    Obstacle {
                        pos: [
                            rng.in_range(-w / 2.0 + half, w / 2.0 - half),
                            rng.in_range(-h / 2.0 + half, h / 2.0 - half),
                        ],
                        color: frost::Color {
                            r: rng.next_f32(),
                            g: rng.next_f32(),
                            b: rng.next_f32(),
                            a: 1.0,
                        },
                    }
                })
                .collect()
        });

        // All at the default draw order (z = 0.0). The scene's button is
        // drawn after the process, so at equal z it ends up on top of them.
        for o in obstacles {
            ctx.rectangle(o.pos[0], o.pos[1], SIZE / 2.0, SIZE / 2.0, o.color, 0.0);
        }

        log::trace!("process: dt {:?} pos {:?} rot {:?}", dt, self.pos, self.rot);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset path to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let button = frost::Shape::sprite(format!("{root}/assets/sprites/Button.png"))
        .expect("failed to load assets/sprites/Button.png");

    if let Err(err) = frost::run(
        frost::Scene::new(frost::SceneNode {
            // Deep indigo background; the node's transform is ignored.
            shape: Some(frost::Shape::Background {
                color: frost::Color {
                    r: 0.09,
                    g: 0.06,
                    b: 0.16,
                    a: 1.0,
                },
            }),
            children: vec![Box::new(frost::SceneNode {
                // The button starts at the window center, facing +x.
                shape: Some(button),
                ..Default::default()
            })],
            ..Default::default()
        }),
        Demo {
            pos: [0.0, 0.0],
            rot: 0.0,
            obstacles: None,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
