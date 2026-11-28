//! Executable trust boundary; the nice-rack application library denies unsafe.
#![deny(unsafe_op_in_unsafe_fn)]
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--clap-helper") {
        let result = match (args.get(2), args.get(3)) {
            // SAFETY: the operator selected this native plugin through the rack.
            (Some(address), Some(token)) => unsafe {
                nice_clap_host::isolated::run_helper(address, token)
            },
            _ => Err("Missing helper connection arguments".into()),
        };
        if let Err(error) = result {
            eprintln!("nice-rack helper: {error}");
            std::process::exit(1);
        }
        return;
    }
    let isolated = args.iter().any(|arg| arg == "--isolate");
    let executable = std::env::current_exe().expect("Locate nice-rack executable");
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
    let result = nice_rack::run(move |path| {
        // SAFETY: dropping a CLAP entry into this explicitly in-process host
        // selects native code to execute. The operator must trust that entry
        // and its CLAP contract; paths/metadata cannot establish this property.
        // This is the same explicit executable boundary as clap-probe.
        if isolated {
            nice_clap_host::Entry::isolated(path, &executable)
        } else {
            unsafe { nice_clap_host::Entry::load(path) }
        }
    });
    nice_clap_host::isolated::finish_shutdown();
    if let Err(error) = result {
        eprintln!("nice-rack: {error}");
        std::process::exit(1);
    }
}
