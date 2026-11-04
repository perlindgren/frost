//! Standalone target for testing the tuner without a DAW.

#![deny(unsafe_code)]

use nice_tuner::settings::{self, Settings};
use std::sync::atomic::Ordering;

fn main() {
    // Helper mode for the settings panel (see devices::snapshot_via_child):
    // enumerate one backend, print JSON on stdout, exit — WITHOUT opening
    // a window or a stream. The whole point is a process that holds no
    // ASIO driver, so it can see drivers the live app cannot. This flag
    // is the app's INTERNAL parent-child protocol, not a user interface:
    // the tuner itself reads device settings from its settings file.
    {
        let argv: Vec<String> = std::env::args().collect();
        if argv.get(1).map(String::as_str) == Some("--list-devices") {
            let backend = argv
                .get(2)
                .filter(|s| !s.is_empty())
                .cloned()
                .unwrap_or_else(|| "auto".to_owned());
            let inp = argv.get(3).filter(|s| !s.is_empty()).map(String::as_str);
            let outp = argv.get(4).filter(|s| !s.is_empty()).map(String::as_str);
            let snap = nice_tuner::devices::snapshot(&backend, inp, outp);
            print!("{}", serde_json::to_string(&snap).unwrap_or_default());
            return;
        }
    }

    nice_tuner::STANDALONE_MODE.store(true, Ordering::Relaxed);

    let (settings, warning) = Settings::load();
    if let Some(warning) = warning {
        eprintln!("nice-tuner: {warning}");
    }

    // Preserve the existing saved input-rate preference. Unset output fields
    // continue to follow the output device's independently queried defaults.
    let warm = Settings::exists() && !settings.is_blank();
    let mut settings = settings;
    if settings.sample_rate.is_none()
        && let Some(boot_backend) = settings.backend.as_deref()
        && let Some(rate) = nice_tuner::devices::driver_default_rate(
            boot_backend,
            settings.input_device.as_deref(),
            settings.output_device.as_deref(),
        )
    {
        settings.sample_rate = Some(rate);
        if warm && let Err(e) = settings.save() {
            eprintln!("nice-tuner: could not remember driver rate: {e}");
        }
    }

    // A device-level buffer must have one value even when settings were
    // edited externally. Resolve implicit default devices before comparing.
    if let Some(backend) = settings.backend.as_deref() {
        let (default_in, default_out) = nice_tuner::devices::default_device_names(backend);
        if settings::buffers_must_match(
            backend,
            settings.input_device.as_deref().or(default_in.as_deref()),
            settings.output_device.as_deref().or(default_out.as_deref()),
        ) {
            settings.output_period_size = settings.period_size;
        }
    }

    let boot = match settings.backend.as_deref() {
        None | Some("dummy") => Ok(settings.clone()),
        Some(backend) => nice_tuner::devices::snapshot(
            backend,
            settings.input_device.as_deref(),
            settings.output_device.as_deref(),
        )
        .resolve_settings(&settings),
    };
    let applied = match boot {
        Ok(applied) => {
            let mut reported = settings.clone();
            reported.input_channel = applied.input_channel;
            let _ = nice_tuner::STANDALONE_BOOT.set((reported, None));
            applied
        }
        Err(error) => {
            eprintln!("nice-tuner: {error}; opening settings with audio stopped");
            let silent = Settings::default();
            let _ = nice_tuner::STANDALONE_BOOT.set((silent.clone(), Some(error)));
            silent
        }
    };
    let _ = tracing_subscriber::fmt().with_target(false).try_init();
    if let Err(error) = nice_tuner::standalone::run(applied) {
        eprintln!("nice-tuner: {error}");
        std::process::exit(1);
    }
}
