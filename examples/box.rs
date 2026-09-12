fn main() {
    env_logger::init();
    if let Err(err) = frost::run(|ctx: &mut frost::Canvas, _dt: f32| {
        // Dark blue background.
        ctx.set_background(frost::Color {
            r: 0.05,
            g: 0.06,
            b: 0.12,
        });
        // A 100 x 100 px square centered at the window origin (0, 0), teal.
        ctx.rectangle(
            0.0,
            0.0,
            50.0,
            50.0,
            frost::Color {
                r: 0.2,
                g: 0.8,
                b: 0.7,
            },
            0.0,
        );
    }) {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
