fn main() {
    env_logger::init();
    if let Err(err) = frost::run(|ctx: &mut frost::Canvas| {
        let (w, h) = ctx.size();
        // Diagonal from the top-left corner to the bottom-right corner.
        ctx.line(-w / 2.0, h / 2.0, w / 2.0, -h / 2.0);
        // Circle at the window center, radius = height / 4.
        ctx.circle(0.0, 0.0, h / 4.0);

        // short line
        ctx.line(200.0, 200.0, 250.0, 250.0);

        // small circle
        ctx.circle(-100.0, -100.0, 20.0);

        // rectangle centered at (120, 120), 100 x 60 px
        ctx.rectangle(120.0, 120.0, 50.0, 30.0);
    }) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
