fn main() {
    env_logger::init();
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
            ..Default::default()
        }),
        |ctx: &mut frost::Context, _dt: f32| {
            let (w, h) = ctx.size();
            // Big circle at the window center.
        ctx.circle(
            0.0,
            0.0,
            h / 4.0,
            frost::Color {
                r: 0.9,
                g: 0.55,
                b: 0.25,
                a: 1.0,
            },
            0.0,
        );
        // Smaller circle towards the bottom-left corner, well clear of the
        // center circle (80 px margin from the edge, 40 px radius).
        ctx.circle(
            -w / 2.0 + 80.0,
            -h / 2.0 + 80.0,
            40.0,
            frost::Color {
                r: 0.95,
                g: 0.35,
                b: 0.4,
                a: 1.0,
            },
            0.0,
        );
        })
    {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
