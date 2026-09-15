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
                },
            }),
            ..Default::default()
        }),
        |ctx: &mut frost::Context, _dt: f32| {
            let (w, h) = ctx.size();
            // Diagonal from the top-left corner to the bottom-right corner.
        ctx.line(
            -w / 2.0,
            h / 2.0,
            w / 2.0,
            -h / 2.0,
            frost::Color {
                r: 0.6,
                g: 0.7,
                b: 0.9,
            },
            2.0, // 2 px long line
            0.0,
        );
        // Circle at the window center, radius = height / 4.
        ctx.circle(
            0.0,
            0.0,
            h / 4.0,
            frost::Color {
                r: 0.9,
                g: 0.55,
                b: 0.25,
            },
            0.0,
        );

        // short line, on top
        ctx.line(
            200.0,
            200.0,
            250.0,
            250.0,
            frost::Color {
                r: 0.4,
                g: 0.9,
                b: 0.5,
            },
            5.0, // 5 px short line
            1.0,
        );

        // small circle
        ctx.circle(
            -100.0,
            -100.0,
            20.0,
            frost::Color {
                r: 0.95,
                g: 0.35,
                b: 0.4,
            },
            0.0,
        );

        // rectangle centered at (120, 120), 100 x 60 px, on top
        ctx.rectangle(
            120.0,
            120.0,
            50.0,
            30.0,
            frost::Color {
                r: 0.95,
                g: 0.85,
                b: 0.3,
            },
            1.0,
        );
        })
    {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
