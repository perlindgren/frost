//! A diagnostics overlay: the `frost::Diagnostics` utility reports the
//! window size and, for every statistic enabled in its
//! `frost::DiagnosticsFlags` (all of them here), a text line in the
//! window's top-left corner — the frame rate, the frame time, the last
//! frame's total processing time, that time excluding the overlay's own
//! update cost, and the last frame's GPU draw-call count — with four
//! scrolling ten-second strip charts beneath: the frame rate, the frame
//! time, the folded processing-time chart (total, app, and the overlay's
//! own cost as three series), and the folded draw-call chart the same way,
//! set in `assets/fonts/FiraCode-VariableFont_wght.ttf`. The integration is
//! one
//! struct in the demo state and one `process` call per frame — the overlay
//! appends its own nodes to the scene's root on the first frame, so the
//! scene itself only carries a swaying circle under them. The overlay's
//! parts can be toggled while it runs — Alt-0 the whole overlay,
//! Alt-1..Alt-4 the charts (top chart first), Alt-T the text, and Alt+'+' /
//! Alt+'-' grow and shrink the whole overlay — and the
//! layout reflows around whatever is hidden. See the module docs of
//! `frost::Diagnostics` for the whole utility.
//! Run with:
//!
//! ```text
//! cargo run --example diagnostics
//! ```

struct Demo {
    /// The diagnostics overlay, reporting the window size, frame rate,
    /// frame time, processing times, and draw-call count.
    diag: frost::Diagnostics,
    /// Elapsed time in seconds.
    t: f32,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Context, dt: f32) {
        self.t += dt;

        // The overlay takes the same context and dt the demo uses: its
        // first call appends its own nodes to the root, and every later
        // call updates those nodes in place.
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
    let diag = match frost::Diagnostics::new(
        format!("{root}/assets/fonts/FiraCode-VariableFont_wght.ttf"),
        frost::DiagnosticsFlags::all(),
    ) {
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
