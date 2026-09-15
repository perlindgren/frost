fn main() {
    env_logger::init();
    if let Err(err) = frost::run(
        frost::Scene::new(frost::SceneNode {
            // Dark blue background; the node's transform is ignored.
            shape: Some(frost::Shape::Background {
                color: frost::Color {
                    r: 0.05,
                    g: 0.06,
                    b: 0.12,
                },
            }),
            ..Default::default()
        }),
        |ctx: &mut frost::Context, _dt: f32| {
            // A 100 x 100 px square centered at the window origin (0, 0), red.
        ctx.rectangle(
            0.0,
            0.0,
            50.0,
            50.0,
            frost::Color {
                r: 1.0,
                g: 0.0 ,
                b: 0.0,
            },
            0.0, // back
        );
         // A 100 x 100 px square centered at the window origin (10, 10), green.
        ctx.rectangle(
            10.0,
            10.0,
            50.0,
            50.0,
            frost::Color {
                r: 0.0,
                g: 1.0,
                b: 0.0,
            },
            2.0, // top
        );
        // A 100 x 100 px square centered at the window origin (20, 20), blue.
        ctx.rectangle(
            20.0,
            20.0,
            50.0,
            50.0,
            frost::Color {
                r: 0.0,
                g: 0.0,
                b: 1.0
                ,
            },
            1.0, // mid
        );
        // A 100 x 100 px square centered at the window origin (-10, -10), white.
        ctx.rectangle(
            -10.0,
            -10.0,
            50.0,
            50.0,
            frost::Color {
                r: 1.0,
                g: 1.0,
                b: 1.0
                ,
            },
            0.0,
        );
        })
    {
        log::error!("frost failed: {err}");
        std::process::exit(1);
    }
}
