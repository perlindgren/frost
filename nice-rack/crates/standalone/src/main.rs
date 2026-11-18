//! Executable trust boundary; the nice-rack application library denies unsafe.
#![deny(unsafe_op_in_unsafe_fn)]
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--list-devices") {
        let snapshot = nice_common::devices::snapshot(
            // A listing typed by hand names no backend; the parent half of
            // this protocol always does. The fallback is the platform's own
            // path, spelled out on the constant.
            args.get(2)
                .filter(|s| !s.is_empty())
                .map(String::as_str)
                .unwrap_or(nice_common::devices::LIST_BACKEND_WHEN_UNNAMED),
            args.get(3).filter(|s| !s.is_empty()).map(String::as_str),
            args.get(4).filter(|s| !s.is_empty()).map(String::as_str),
        );
        match snapshot.to_ron() {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    // The two registry commands, answered without a window or an audio
    // device: they only write the current user's registry, so they are
    // safe to run while REAPER sits open on the real driver.
    #[cfg(windows)]
    if args.get(1).map(String::as_str) == Some("--register-asio")
        || args.get(1).map(String::as_str) == Some("--unregister-asio")
    {
        let unregister = args[1] == "--unregister-asio";
        let code = if unregister {
            match niceroute::register::unregister() {
                Ok(()) => {
                    println!("unregistered '{}'", niceroute::DRIVER_NAME);
                    0
                }
                Err(error) => {
                    eprintln!("nice-rack: unregister: {error}");
                    1
                }
            }
        } else {
            // The device lives next to this executable; there is no
            // install directory to guess.
            let dll = std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("niceroute.dll")))
                .unwrap_or_else(|| std::path::PathBuf::from("niceroute.dll"));
            // `--real NAME` picks the driver the master should host; its
            // absence is not a choice, so the setting is left alone and
            // the master keeps whatever the user configured before.
            let flag = |name: &str| {
                args.iter()
                    .position(|arg| arg == name)
                    .and_then(|at| args.get(at + 1))
                    .cloned()
            };
            let number = |name: &str| flag(name).and_then(|text| text.parse::<u32>().ok());
            let real: Option<&'static str> = if args.iter().any(|arg| arg == "--no-real") {
                Some("")
            } else {
                flag("--real").map(|name| -> &'static str { name.leak() })
            };
            match niceroute::register::register(&dll, real, number("--hw-in"), number("--hw-out")) {
                Ok(()) => {
                    println!(
                        "registered '{}' -> {}",
                        niceroute::DRIVER_NAME,
                        dll.display()
                    );
                    match niceroute::register::Config::load().real_driver {
                        Some(name) => println!("the master will host '{name}'"),
                        None => println!("no real driver configured: the device is virtual-only"),
                    }
                    0
                }
                Err(error) => {
                    eprintln!("nice-rack: register: {error}");
                    1
                }
            }
        };
        std::process::exit(code);
    }
    // The second question this executable can answer without a window: the
    // settings are applied to real drivers and the two drivers' own callback
    // counts are printed once a second, with a 1 kHz tone where the rack's audio
    // would be. "The second output is open and silent" is three different faults
    // and a panel cannot tell them apart; the counts can.
    #[cfg(all(windows, feature = "asio-host"))]
    if args.get(1).map(String::as_str) == Some("--audio-probe") {
        let seconds = args
            .get(2)
            .and_then(|arg| arg.parse::<u64>().ok())
            .filter(|n| (1..=600).contains(n))
            .unwrap_or(10);
        // Peak amplitude of the tone, which is an argument because the useful form
        // of this test is several runs with one thing changed in each, and every run
        // plays through the monitors as well as into the second driver. The default
        // is the level it has always used, about -24 dBFS: loud enough to hear across
        // a room, quiet enough not to be an event.
        let level = args
            .get(3)
            .and_then(|arg| arg.parse::<f32>().ok())
            .unwrap_or(0.06);
        let (settings, warning) = nice_rack::Settings::load();
        if let Some(warning) = warning {
            eprintln!("nice-rack: {warning}");
        }
        let shape = match nice_common::audio_io::wanted(&settings) {
            Ok(shape) => shape,
            Err(error) => {
                eprintln!("nice-rack: {error}");
                std::process::exit(1);
            }
        };
        let route = settings
            .output_channels
            .map_or_else(|| "no route".to_string(), |route| format!("{route:?}"));
        println!(
            "'{}': {route}, {outputs} channels, {} Hz, {} frames; copy starts at {}, on '{}'",
            shape.output,
            shape.input_rate,
            shape.input_frames,
            shape.second_output.unwrap_or(0),
            shape.second_device.unwrap_or("the same driver"),
            outputs = shape.outputs,
        );
        // Said before the drivers open rather than after, because the first thing a
        // person running this hears is a tone coming out of their own monitors — the
        // probe fills one block and both paths carry it, so the live output plays it
        // as surely as the copy does. Stating the level rather than a bare
        // "a tone will play" is the point: at zero there is nothing to hear, and the
        // run is worth repeating with one thing changed each time.
        println!(
            "tone: 1 kHz at {:.1} dBFS out of '{}' and any second driver",
            20.0 * f64::from(level.clamp(0.0, 0.5).max(1e-9)).log10(),
            shape.output
        );
        let started = std::time::Instant::now();
        let counted = nice_common::audio_io::probe(&settings, seconds, level, |_, probe| {
            let copy = probe.copy.as_ref().map_or_else(
                || "no second driver".to_string(),
                |pulse| {
                    // The announcements ride along because "wrote blocks" and
                    // "told the driver about them" are two claims, and the
                    // silence on the far side sounds the same either way — and
                    // a refusal without its code says only "no".
                    let (told, refused) = probe.copy_announced;
                    format!(
                        "{} arrivals, {} blocks, writing {}, announced {}{}",
                        pulse.arrivals,
                        pulse.served,
                        probe.copy_formats.join("+"),
                        told,
                        if refused > 0 {
                            format!(
                                ", {refused} refused {}",
                                probe.copy_refusal.as_deref().unwrap_or("?"),
                            )
                        } else {
                            String::new()
                        },
                    )
                },
            );
            println!(
                "live {} arrivals, {} blocks | copy {copy} | tone {} blocks | overloads {}",
                probe.live.arrivals, probe.live.served, probe.blocks, probe.overloads,
            );
        });
        println!("{:.1} s of callbacks", started.elapsed().as_secs_f32());
        if let Err(error) = counted {
            eprintln!("nice-rack: {error}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = nice_rack::run(|path| {
        // SAFETY: dropping a CLAP entry into this explicitly in-process host
        // selects native code to execute. The operator must trust that entry
        // and its CLAP contract; paths/metadata cannot establish this property.
        // This is the same explicit executable boundary as clap-probe.
        unsafe { nice_clap_host::Entry::load(path) }
    }) {
        eprintln!("nice-rack: {error}");
        std::process::exit(1);
    }
}
