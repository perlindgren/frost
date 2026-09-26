//! A diagnostics overlay: the `frost::Diagnostics` utility reports the
//! window size, the frame rate, and the frame time as text lines in the
//! window's top-left corner, with two scrolling ten-second strip charts of
//! the frame rate and frame time beneath, set in
//! `assets/fonts/Leofont-Regular.ttf`. The integration is one struct in the
//! demo state and one `process` call per frame — the overlay appends its
//! own nodes to the scene's root on the first frame, so the scene itself
//! only carries a swaying circle under them. See the module docs of
//! `frost::Diagnostics` for the whole utility.
//! Run with:
//!
//! ```text
//! cargo run --example diagnostics
//! ```

struct Demo {
    /// The diagnostics overlay, reporting the frame rate, frame time, and
    /// window size.
    diag: frost::Diagnostics,
    /// Elapsed time in seconds.
    t: f32,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        self.t += dt;

        // The overlay takes the same context and dt the demo uses: its
        // first call appends the text node to the root, and every later
        // call updates that node in place.
        self.diag.process(ctx, dt);

        // The circle, with the overlay riding on top of it, sways gently up
        // and down.
        let circle = &mut ctx.scene().root.children[0];
        circle.transform = frost::Transform::translate([0.0, (self.t * 0.5).sin() * 20.0]);
        log::trace!("process: dt {:?}", dt);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");

    // `CARGO_MANIFEST_DIR` pins the asset paths to the crate root, so the
    // example works no matter where it is run from.
    let root = std::env!("CARGO_MANIFEST_DIR");
    let diag = match frost::Diagnostics::new(format!(
        "{root}/assets/fonts/Leofont-Regular.ttf"
    )) {
        Ok(diagnostics) => diagnostics,
        Err(err) => {
            log::error!("failed to load the font: {err}");
            std::process::exit(1);
        }
    };

    let scene = frost::Scene::new(frost::SceneNode {
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
            // The identity transform keeps the circle at the screen center;
            // process() nudges it vertically each frame.
            shape: Some(frost::Shape::Circle {
                center: [0.0, 0.0],
                radius: 90.0,
                color: frost::Color {
                    r: 0.25,
                    g: 0.35,
                    b: 0.6,
                    a: 1.0,
                },
            }),
            ..Default::default()
        })],
        ..Default::default()
    });

    if let Err(err) = frost::run(scene, Demo { diag, t: 0.0 }) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
