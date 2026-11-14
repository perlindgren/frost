//! Application-owned standalone host; shares its editor and DSP with CLAP/VST3.
#![deny(unsafe_code)]
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--list-devices") {
        let snapshot = nice_wah::devices::snapshot(
            // A listing typed by hand names no backend; the parent half of
            // this protocol always does. The fallback is the platform's own
            // path, spelled out on the constant.
            args.get(2)
                .filter(|s| !s.is_empty())
                .map(String::as_str)
                .unwrap_or(nice_wah::devices::LIST_BACKEND_WHEN_UNNAMED),
            args.get(3).filter(|s| !s.is_empty()).map(String::as_str),
            args.get(4).filter(|s| !s.is_empty()).map(String::as_str),
        );
        println!(
            "{}",
            snapshot
                .to_ron()
                .expect("device snapshot must serialize to RON")
        );
        return;
    }
    nice_wah::STANDALONE_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
    let (settings, warning) = nice_wah::settings::Settings::load();
    if let Some(warning) = warning {
        eprintln!("nice-wah: {warning}");
    }
    if let Err(error) = nice_wah::standalone::run(settings) {
        eprintln!("nice-wah: {error}");
        std::process::exit(1);
    }
}
