//! A four-frame monster flipbook: `Monster1.png` through `Monster4.png`
//! (from this folder's `assets/sprites`) are loaded up front and swapped
//! into one sprite node every eighth of a second, cycling forever — the
//! classic sprite-sheet animation done with one whole image per frame.
//! The cycle plays a 1-2-1-4 gait: `Monster3.png` is an identical copy of
//! `Monster1.png` *by design* — the repeated stand pose is the two "up"
//! beats between the half-squat (frame 2) and the deep crouch (frame 4),
//! so the walk's squash lives in the art and the process only adds a
//! gentle float, sway, and breathing on top. Because a sprite's pixels
//! live behind an `Arc`, advancing the flipbook is just an `Arc` clone
//! per frame, not a re-decode. The window opens at a true physical
//! 1920x1080 on every display (`Config::window_size_px` — a 960x540
//! point window on a 2x Retina screen), and since frost's user space is
//! physical panel pixels, the frames land one texel per pixel on all of
//! them; the fit scale comes from the live window size, so resizing or a
//! different panel still fits the art inside.
//! Run with:
//!
//! ```text
//! cargo run --example dogs_name
//! ```

/// The window's initial size in physical pixels: a real 1920x1080 panel
/// window on every display (960x540 points where the scale factor is 2),
/// 16:9 like the monster art, so a frame at fit scale fills the window
/// edge to edge.
const WINDOW: [u32; 2] = [1920, 1080];

/// How long each frame of the flipbook is held, in seconds — 0.125 is
/// exactly 8 frames per second, a brisk walk cycle pace.
const FRAME_SECONDS: f32 = 0.125;

struct Demo {
    /// Elapsed time in seconds, driving the bob, sway, and breathing.
    t: f32,
    /// Seconds elapsed within the current flipbook cycle; the frame index
    /// falls out of dividing it by `FRAME_SECONDS`.
    cycle: f32,
    /// The four frames in play order, loaded once at startup. The plain
    /// 1, 2, 3, 4 cycle walks the 1-2-1-4 gait because frame 3 is an
    /// intentional duplicate of frame 1 — the gait's second "up" beat.
    frames: Vec<frost::Shape>,
    /// The frames' texture size in pixels: the process divides it into
    /// the live window size to get each frame's fit scale, so resizing
    /// re-fits the art by itself.
    texture: [f32; 2],
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        self.t += dt;
        self.cycle += dt;

        // The fit scale comes from the live window, read before the scene
        // borrow below: frost's user space is physical panel pixels, so
        // the 1920x1080 art fits a 1920x1080 window at exactly 1.0 and a
        // smaller window proportionally.
        let (width, height) = ctx.size();
        let fit = (width / self.texture[0]).min(height / self.texture[1]);

        // A gentle float under the flipbook: the gait's squash and rise
        // already live in the poses, so the motion here only stops the
        // monster from sitting dead still — a slow float, a small sway,
        // and breathing that stretches up as it rises and squashes wide
        // as it settles, all around the fit scale. The bob is authored in
        // window pixels, and the node's scale multiplies its local space,
        // so it is divided back out.
        let bob = (self.t * 2.5).sin() * 8.0 / fit;
        let sway = (self.t * 1.25).sin() * 0.04;
        let breathe = (self.t * 2.5).sin() * 0.03;

        // The flipbook frame: the cycle clock wraps through the frames at
        // FRAME_SECONDS each. Swapping the node's shape is cheap — the
        // sprite's pixels are an `Arc` clone, and the shape is cloned once
        // per node per frame by the renderer anyway.
        let frame = (self.cycle / FRAME_SECONDS) as usize % self.frames.len();
        let monster = &mut ctx.scene().root.children[0];
        monster.shape = Some(self.frames[frame].clone());
        monster.transform =
            frost::Transform::rotate(sway).compose(&frost::Transform::translate([0.0, bob]));
        monster.scale = [fit * (1.0 - breathe), fit * (1.0 + breathe)];

        log::trace!("process: dt {:?}, frame {}", dt, frame);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset paths to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let frames: Vec<frost::Shape> = (1..=4)
        .map(|i| {
            frost::Shape::sprite(format!(
                "{root}/examples/dogs_name/assets/sprites/Monster{i}.png"
            ))
            .unwrap_or_else(|err| panic!("failed to load Monster{i}.png: {err}"))
        })
        .collect();

    // The frames' texture size, the divisor of the process's fit scale.
    let texture = frames[0].sprite_size().expect("the frames are sprites");

    let scene = frost::Scene::new(frost::SceneNode {
        // A moonlit grey-green night behind the monster.
        shape: Some(frost::Shape::Background {
            color: frost::Color {
                r: 0.07,
                g: 0.09,
                b: 0.11,
                a: 1.0,
            },
        }),
        children: vec![Box::new(frost::SceneNode {
            // Frames are centered on the node's origin, so the origin sits
            // at the window's center and the bob and sway move the whole
            // image around the middle of the screen. The process scales
            // the node to fit every frame, so the node starts unscaled.
            shape: Some(frames[0].clone()),
            ..Default::default()
        })],
        ..Default::default()
    });

    if let Err(err) = frost::run_configured(
        scene,
        Demo {
            t: 0.0,
            cycle: 0.0,
            frames,
            texture,
        },
        frost::Config {
            vsync: true,
            window_size: None,
            window_size_px: Some(WINDOW),
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
