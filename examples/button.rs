//! A "play" button at the window's center. Hovering it tweens the button
//! node's scale up by 10% (the node's `scale` applies to the whole subtree,
//! so the label grows with it); leaving tweens it back to its original
//! size. Pressing the left mouse button on the button and then releasing it
//! while still hovering plays `assets/audio/swoof.wav` once through a
//! [`frost::Audio`]. Both the press and the release must
//! happen over the button: the press edge arms the click, and only an armed
//! release over the button fires the swoosh. Esc closes the window. Run
//! with:
//!
//! ```text
//! cargo run --example button
//! ```

/// The button's base width in pixels, before the hover scale.
const BUTTON_W: f32 = 240.0;

/// The button's base height in pixels, before the hover scale.
const BUTTON_H: f32 = 80.0;

/// The label's font size in pixels.
const TEXT_SIZE: f32 = 40.0;

/// The button's scale factor while hovered: 10% above its base size.
const HOVER_SCALE: f32 = 1.1;

/// Seconds for the hover scale to travel between its two sizes, on hover-in
/// and hover-out alike.
const HOVER_TIME: f32 = 0.15;

/// The demo's state.
struct Demo {
    /// The open audio output; dropping it (on exit) stops everything.
    audio: frost::Audio,
    /// The decoded swoosh, loaded once before `frost::run`.
    swoosh: frost::Sound,
    /// Whether the cursor was over the button on the previous frame.
    hovered: bool,
    /// The button's live scale factor, driven by `hover_tween`.
    scale: f32,
    /// The hover-scale tween; rebuilt on every hover edge (a `Tween` has no
    /// re-target), the same idiom as `cursor.rs`' press/release rotation.
    hover_tween: frost::Tween<f32>,
    /// Whether the left mouse button was held on the previous frame.
    pressed: bool,
    /// Whether the last press edge happened over the button; only such a
    /// press may fire the swoosh on release.
    armed: bool,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // Read the input state first: while the scene borrow below is live,
        // `ctx` cannot be borrowed again.
        //
        // Hover: hit-test against the button's base (unscaled) extents, so
        // the trigger region stays put while the button animates and can
        // never flicker at its growing edge.
        let hovered = ctx
            .mouse_position()
            .is_some_and(|[mx, my]| mx.abs() <= BUTTON_W / 2.0 && my.abs() <= BUTTON_H / 2.0);
        let down = ctx.mouse_button_down(frost::MouseButton::Left);

        // A hover edge rebuilds the tween from the live scale, so an edge
        // mid-animation picks up where the button currently is.
        if hovered != self.hovered {
            self.hover_tween = frost::Tween::new(
                self.scale,
                if hovered { HOVER_SCALE } else { 1.0 },
                HOVER_TIME,
            )
            .repeat(frost::Repeat::Once);
            self.hovered = hovered;
        }
        self.scale = self.hover_tween.tick(dt);
        ctx.scene().root.children.get_mut(0).unwrap().scale = [self.scale, self.scale];

        // Click: the press edge arms the button when it happens over it,
        // and an armed release over the button plays the swoosh once.
        if down && !self.pressed {
            self.armed = hovered;
        }
        if !down && self.pressed {
            if self.armed && hovered {
                self.audio.play_once(&self.swoosh, None);
            }
            self.armed = false;
        }
        self.pressed = down;
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    let root = std::env!("CARGO_MANIFEST_DIR");
    let swoosh = frost::Sound::load(format!("{root}/assets/audio/swoof.wav"))
        .expect("failed to load assets/audio/swoof.wav");
    let audio = frost::Audio::new().expect("failed to open the audio output device");
    let label = frost::Shape::text(
        format!("{root}/assets/fonts/JameGem08_2026-Regular.ttf"),
        "play",
        TEXT_SIZE,
    )
    .expect("failed to load assets/fonts/JameGem08_2026-Regular.ttf");

    let scene = frost::Scene {
        root: frost::SceneNode {
            // The background hangs on the base group (the scene's root);
            // its transform is ignored anyway, so it always fills the
            // window.
            shape: Some(frost::Shape::Background {
                color: frost::Color {
                    r: 0.08,
                    g: 0.09,
                    b: 0.12,
                    a: 1.0,
                },
            }),
            children: vec![Box::new(frost::SceneNode {
                // The button sits on the scene's origin, the window's
                // center; its scale grows the rectangle and the label
                // together around that center.
                scale: [1.0, 1.0],
                shape: Some(frost::Shape::Rectangle {
                    center: [0.0, 0.0],
                    extent: [BUTTON_W / 2.0, BUTTON_H / 2.0],
                    color: frost::Color {
                        r: 0.25,
                        g: 0.45,
                        b: 0.85,
                        a: 1.0,
                    },
                }),
                children: vec![Box::new(frost::SceneNode {
                    // The text block is centered on its node's origin, so
                    // the label sits dead-center in the rectangle with no
                    // offset math.
                    shape: Some(label),
                    ..Default::default()
                })],
                ..Default::default()
            })],
            ..Default::default()
        },
        ..Default::default()
    };

    if let Err(err) = frost::run(
        scene,
        Demo {
            audio,
            swoosh,
            hovered: false,
            scale: 1.0,
            // A 1.0→1.0 tween that never moves, until the first hover edge
            // replaces it.
            hover_tween: frost::Tween::new(1.0, 1.0, 1.0).repeat(frost::Repeat::Once),
            pressed: false,
            armed: false,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
