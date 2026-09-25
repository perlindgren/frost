//! `assets/audio/swoof.wav` decoded once at startup into a
//! [`frost::Sound`], played through a [`frost::Audio`]: Space triggers the
//! sound as a one-shot — a copy of its samples is added to the mixer, so
//! rapid presses overlap freely and each one fires a pulse of the center
//! circle — while L starts or stops the single loop, the same sound
//! repeating behind a wobbling outline ring. The `+` and `-` keys move the
//! master volume, shown as a bar at the bottom of the window: it applies
//! to the loop live and to one-shots from the next trigger on. Esc closes
//! the window. Run with:
//!
//! ```text
//! cargo run --example sound
//! ```

/// The center circle's radius in pixels, at rest.
const BASE_RADIUS: f32 = 40.0;

/// The pulse's extra radius at full strength, in pixels.
const PULSE_RADIUS: f32 = 160.0;

/// Seconds for the pulse to fade after a retrigger.
const PULSE_TIME: f32 = 0.4;

/// The looping ring's base radius in pixels.
const RING_RADIUS: f32 = 260.0;

/// The ring's wobble amplitude in pixels.
const RING_WOBBLE: f32 = 14.0;

/// The ring's wobble lobes around the full circle.
const RING_LOBES: f32 = 6.0;

/// The ring's wobble speed, in radians of phase per second.
const RING_SPEED: f32 = 2.5;

/// The volume bar's full width in pixels.
const BAR_WIDTH: f32 = 240.0;

/// The volume bar's height in pixels.
const BAR_HEIGHT: f32 = 10.0;

/// The volume bar's inset from the window's left and bottom edges, in
/// pixels.
const BAR_MARGIN: f32 = 30.0;

/// The demo's state.
struct Demo {
    /// The open audio output; dropping it (on exit) stops everything.
    audio: frost::Audio,
    /// The decoded swoosh, loaded once before `frost::run`.
    swoosh: frost::Sound,
    /// The path to the bundled font, for the help line.
    font: String,
    /// The one-shot's pulse: 1.0 at retrigger, decaying to 0 over
    /// `PULSE_TIME`.
    pulse: f32,
    /// Whether the loop is playing.
    looping: bool,
    /// The master volume, mirrored from `audio` for the bar.
    volume: f32,
    /// Time in seconds, driving the ring's wobble.
    time: f32,
    /// Set once the first frame has placed the help text.
    built: bool,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        // The help text needs the window size, so it is placed on the first
        // frame, the same way the other demos build their screen-fixed HUD.
        if !self.built {
            self.built = true;
            let (w, h) = ctx.size();
            let help = frost::Shape::text(
                &self.font,
                "Space: swoosh   L: loop   +/-: volume   Esc: quit",
                24.0,
            )
            .expect("failed to load assets/fonts/JameGem08_2026-Regular.ttf");
            ctx.scene().root.children.push(Box::new(frost::SceneNode {
                transform: frost::Transform::translate([-w / 2.0 + 40.0, h / 2.0 - 40.0]),
                shape: Some(help),
                ..Default::default()
            }));
        }

        self.time += dt;
        self.pulse = (self.pulse - dt / PULSE_TIME).max(0.0);

        // Space: retrigger the one-shot and fire the pulse.
        if ctx.key_down(frost::KeyCode::Space) {
            self.audio.play_once(&self.swoosh, None);
            self.pulse = 1.0;
        }
        // L: start the loop, or stop it if it is already playing.
        if ctx.key_down(frost::KeyCode::KeyL) {
            if self.looping {
                self.audio.stop_loop();
            } else {
                self.audio.play_loop(&self.swoosh);
            }
            self.looping = !self.looping;
        }
        // +/-: nudge the master volume by a tenth.
        if ctx.key_down(frost::KeyCode::Equal) {
            self.volume = (self.volume + 0.1).min(1.0);
            self.audio.set_volume(self.volume);
        }
        if ctx.key_down(frost::KeyCode::Minus) {
            self.volume = (self.volume - 0.1).max(0.0);
            self.audio.set_volume(self.volume);
        }

        let (w, h) = ctx.size();

        // The pulse circle: radius and alpha grow with the pulse.
        ctx.circle(
            0.0,
            0.0,
            BASE_RADIUS + PULSE_RADIUS * self.pulse,
            frost::Color {
                r: 0.95,
                g: 0.75,
                b: 0.25,
                a: 0.25 + 0.75 * self.pulse,
            },
            0.0,
        );

        // The wobbling ring, only while the loop plays.
        if self.looping {
            let segments = 96;
            for i in 0..segments {
                let a0 = i as f32 / segments as f32 * std::f32::consts::TAU;
                let a1 = (i + 1) as f32 / segments as f32 * std::f32::consts::TAU;
                let r0 =
                    RING_RADIUS + RING_WOBBLE * (a0 * RING_LOBES + self.time * RING_SPEED).sin();
                let r1 =
                    RING_RADIUS + RING_WOBBLE * (a1 * RING_LOBES + self.time * RING_SPEED).sin();
                ctx.line(
                    r0 * a0.cos(),
                    r0 * a0.sin(),
                    r1 * a1.cos(),
                    r1 * a1.sin(),
                    frost::Color {
                        r: 0.35,
                        g: 0.65,
                        b: 0.95,
                        a: 0.9,
                    },
                    2.0,
                    1.0,
                );
            }
        }

        // The volume bar: a dim track, and a bright fill from its left edge
        // to the current volume.
        let bar_x = -w / 2.0 + BAR_MARGIN + BAR_WIDTH / 2.0;
        let bar_y = -h / 2.0 + BAR_MARGIN;
        ctx.rectangle(
            bar_x,
            bar_y,
            BAR_WIDTH / 2.0,
            BAR_HEIGHT / 2.0,
            frost::Color {
                r: 0.2,
                g: 0.2,
                b: 0.25,
                a: 1.0,
            },
            2.0,
        );
        if self.volume > 0.0 {
            ctx.rectangle(
                -w / 2.0 + BAR_MARGIN + BAR_WIDTH * self.volume / 2.0,
                bar_y,
                BAR_WIDTH * self.volume / 2.0,
                BAR_HEIGHT / 2.0,
                frost::Color {
                    r: 0.35,
                    g: 0.85,
                    b: 0.45,
                    a: 1.0,
                },
                3.0,
            );
        }
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");
    // `CARGO_MANIFEST_DIR` pins the asset path to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let font = format!("{root}/assets/fonts/JameGem08_2026-Regular.ttf");
    // The swoosh decodes once, at load time: a missing file or a bad format
    // fails here with an AudioError, not at play time.
    let swoosh = frost::Sound::load(format!("{root}/assets/audio/swoof.wav"))
        .expect("failed to load assets/audio/swoof.wav");
    let audio = frost::Audio::new().expect("failed to open the audio output device");

    if let Err(err) = frost::run(
        frost::Scene {
            // The background lives in the base group (the scene's root);
            // its transform is ignored anyway, so it always fills the
            // window.
            root: frost::SceneNode {
                shape: Some(frost::Shape::Background {
                    color: frost::Color {
                        r: 0.08,
                        g: 0.09,
                        b: 0.12,
                        a: 1.0,
                    },
                }),
                ..Default::default()
            },
            ..Default::default()
        },
        Demo {
            audio,
            swoosh,
            font,
            pulse: 0.0,
            looping: false,
            volume: 1.0,
            time: 0.0,
            built: false,
        },
    ) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
