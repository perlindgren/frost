struct Demo {
    /// Drives the short line's rightward ping-pong. One leg is 1.0s long and
    /// covers 100px, i.e. a constant 100px/s.
    tween: frost::Tween,
}

impl frost::Process for Demo {
    fn process(&mut self, ctx: &mut frost::Canvas, dt: f32) {
        let (w, h) = ctx.size();
        // Diagonal from the top-left corner to the bottom-right corner.
        ctx.line(-w / 2.0, h / 2.0, w / 2.0, -h / 2.0);
        // Circle at the window center, radius = height / 4.
        ctx.circle(0.0, 0.0, h / 4.0);

        // Short line, tweened right by up to 100px and back.
        let dx = 100.0 * self.tween.tick(dt);
        ctx.line(200.0 + dx, 200.0, 250.0 + dx, 250.0);

        // Small circle.
        ctx.circle(-100.0, -100.0, 20.0);

        // Rectangle centered at (120, 120), 100 x 60 px.
        ctx.rectangle(120.0, 120.0, 50.0, 30.0);
        log::info!("process: dt {:?}", dt);
    }
}

fn main() {
    env_logger::init();
    log::info!("frost started");
    if let Err(err) = frost::run(Demo {
        tween: frost::Tween::new(1.0),
    }) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
