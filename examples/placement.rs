//! Where do shapes actually land? Two boxes, one word, no game.
//!
//! The browser draws this engine's text with every glyph whole, correctly
//! sized and carrying the right pixels, and yet the page comes out in slanted
//! sheets; the desktop, running the same code, is crisp. Every theory that
//! survived contact with the evidence — a wrong row pitch, a scaled or sheared
//! presentation, a bad letterbox fit — predicted a *global* difference between
//! the two frames, and fitting the two frames against each other found none.
//! What is left is the last unmeasured hop: the numbers the CPU writes for a
//! shape's centre and extent, and the pixels those numbers put on screen.
//!
//! So: one box at the centre, one box away from it, one word of text. Each is
//! a colour nothing else uses, so its bounding box is a measurement rather than
//! an impression, and the same binary runs on both platforms. If a box lands
//! where the desktop puts it, the placement path is innocent and the search
//! moves; if it lands elsewhere, the offset is two numbers, and two numbers
//! name the broken mapping — `Canvas::user_to_pixels` or the transform bytes
//! that follow it.
//!
//! Read it with `cargo run --example placement --release` beside the browser
//! page under `web/min/`, or run this file's own `--dump` to print what was
//! asked for, so the expected boxes are on the record and not in memory.

use std::sync::Arc;

use frost::{Color, Config, Context, Process, Scene, SceneNode, Shape, run_configured};

const GAME_FONT: &[u8] = include_bytes!("../assets/fonts/JameGem08_2026-Regular.ttf");

/// Black ground, so the three probes are the only bright things on screen.
const C_BG: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
/// The centred box. Expected at the middle of the canvas, whatever the
/// coordinate convention turns out to be — the point is that both platforms
/// agree on where the middle is.
const C_CENTRE: Color = Color {
    r: 0.0,
    g: 1.0,
    b: 0.0,
    a: 1.0,
};
/// The off-centre box, at user (-300, -200). One probe measures the origin;
/// this one measures the mapping away from it, which is where a scale or a
/// transposed matrix has to show itself.
const C_OFF: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
/// The word, whose glyph boxes were already known to arrive whole.
const C_TEXT: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};

#[derive(Default)]
struct Probe {
    font: Arc<[u8]>,
    logged: bool,
}

impl Process for Probe {
    fn process(&mut self, ctx: &mut Context, _dt: f32) {
        let (w, h) = ctx.size();
        if !self.logged {
            self.logged = true;
            log::info!("placement: canvas {w}x{h}; green centre (0,0) 256x128");
            log::info!("placement: red (-300,-200) 200x100; text PLACE at (0,250) size 48");
        }
        let font = Arc::clone(&self.font);
        ctx.rectangle(0.0, 0.0, 256.0, 128.0, C_CENTRE, 0.0);
        ctx.rectangle(-300.0, -200.0, 200.0, 100.0, C_OFF, 0.0);
        ctx.text(0.0, 250.0, &font, "PLACE", 48.0, 700.0, C_TEXT, 0.0);
    }
}

fn main() {
    env_logger::init();
    if std::env::args().any(|a| a == "--dump") {
        println!("window 1024x768, black ground");
        println!("green box: centre (0,0),   extent 256x128");
        println!("red box:   centre (-300,-200), extent 200x100");
        println!("text:      PLACE at (0,250), size 48");
        return;
    }
    let scene = Scene::new(SceneNode {
        shape: Some(Shape::Background { color: C_BG }),
        ..Default::default()
    });
    if let Err(err) = run_configured(
        scene,
        Probe {
            font: Arc::from(GAME_FONT),
            logged: false,
        },
        Config {
            vsync: false,
            window_size: Some([1024, 768]),
            ..Default::default()
        },
    ) {
        eprintln!("placement: {err}");
        std::process::exit(1);
    }
}
