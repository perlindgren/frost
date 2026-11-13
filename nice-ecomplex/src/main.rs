//! Application-owned standalone host; shares its editor and DSP with CLAP/VST3.
#![deny(unsafe_code)]
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--list-devices") {
        let snapshot = nice_ecomplex::devices::snapshot(
            args.get(2).map(String::as_str).unwrap_or("dummy"),
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
    nice_ecomplex::STANDALONE_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
    let (settings, warning) = nice_ecomplex::settings::Settings::load();
    if let Some(warning) = warning {
        eprintln!("nice-ecomplex: {warning}");
    }
    if let Err(error) = nice_ecomplex::standalone::run(settings) {
        eprintln!("nice-ecomplex: {error}");
        std::process::exit(1);
    }
}
