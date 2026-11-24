//! Application-owned standalone host; shares its editor and DSP with CLAP/VST3.
#![deny(unsafe_code)]
// GUI subsystem: double-clicked, a console-subsystem build spends all of start-up
// showing the console Windows allocated for it, before any window of ours exists. The
// reason and the cost are spelled out in `nice-rack/crates/standalone/src/main.rs`.
// From a terminal the process still inherits that console, so `--list-devices` and any
// error still reach a screen.
#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--list-devices") {
        let snapshot = nice_nam::devices::snapshot(
            // A listing typed by hand names no backend; the parent half of
            // this protocol always does. The fallback is the platform's own
            // path, spelled out on the constant.
            args.get(2)
                .filter(|s| !s.is_empty())
                .map(String::as_str)
                .unwrap_or(nice_nam::devices::LIST_BACKEND_WHEN_UNNAMED),
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
    nice_nam::STANDALONE_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
    let (settings, warning) = nice_nam::settings::Settings::load();
    if let Some(warning) = warning {
        eprintln!("nice-nam: {warning}");
    }
    if let Err(error) = nice_nam::standalone::run(settings) {
        eprintln!("nice-nam: {error}");
        std::process::exit(1);
    }
}
