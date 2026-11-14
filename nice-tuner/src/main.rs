//! Standalone target for testing the tuner without a DAW.

#![deny(unsafe_code)]

use nice_tuner::settings::{self, Settings};
use std::sync::atomic::Ordering;

/// What `--list-devices` enumerates when the command line names no backend.
/// The settings panel always names one — it asks about the backend the user is
/// looking at — so this default only serves the internal protocol typed by
/// hand, and it follows the platform's own rule rather than inventing a third
/// one: on Windows there is no system host to enumerate (`devices::open_host`
/// answers `None` for `"auto"`), so a hand-run listing without a backend names
/// the one path Windows has, whose driver list the registry gives at no cost.
/// Elsewhere "no backend named" means the system default host.
#[cfg(windows)]
const LIST_BACKEND_WHEN_UNNAMED: &str = "asio";
#[cfg(not(windows))]
const LIST_BACKEND_WHEN_UNNAMED: &str = "auto";

fn main() {
    // Helper mode for the settings panel (see devices::snapshot_via_child):
    // enumerate one backend, print RON on stdout, exit — WITHOUT opening
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
                .unwrap_or_else(|| LIST_BACKEND_WHEN_UNNAMED.to_owned());
            let inp = argv.get(3).filter(|s| !s.is_empty()).map(String::as_str);
            let outp = argv.get(4).filter(|s| !s.is_empty()).map(String::as_str);
            let snap = nice_tuner::devices::snapshot(&backend, inp, outp);
            print!(
                "{}",
                snap.to_ron()
                    .expect("device snapshot must serialize to RON")
            );
            return;
        }
    }

    let (test_frequency, note_frequency) = match parse_frequencies(std::env::args().skip(1)) {
        Ok(value) => value,
        Err(error) => {
            eprintln!(
                "nice-tuner: {error}\nUsage: nice-tuner-standalone [--test_freq <Hz>] [--note_freq <Hz>]"
            );
            std::process::exit(2);
        }
    };
    if let Some(hz) = test_frequency {
        if let Err(error) = nice_tuner::standalone::set_test_frequency(hz) {
            eprintln!("nice-tuner: {error}");
            std::process::exit(2);
        }
        eprintln!("nice-tuner: overriding audio input with a {hz} Hz sine (amplitude 1.0)");
    }

    if let Some(hz) = note_frequency {
        if let Err(error) = nice_tuner::standalone::set_note_frequency(hz) {
            eprintln!("nice-tuner: {error}");
            std::process::exit(2);
        }
        eprintln!("nice-tuner: bypassing pitch detection with {hz} Hz");
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

fn parse_frequencies(
    args: impl IntoIterator<Item = String>,
) -> Result<(Option<f64>, Option<f64>), String> {
    let mut args = args.into_iter();
    let (mut test, mut note) = (None, None);
    while let Some(arg) = args.next() {
        let (name, inline) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(n, v)| (n, Some(v)));
        let destination = match name {
            "--test_freq" => &mut test,
            "--note_freq" => &mut note,
            _ => return Err(format!("Unknown argument '{arg}'")),
        };
        if destination.is_some() {
            return Err(format!("{name} may only be specified once"));
        }
        let value = inline
            .map(str::to_owned)
            .or_else(|| args.next())
            .ok_or_else(|| format!("{name} requires a frequency in Hz"))?;
        let hz: f64 = value
            .parse()
            .map_err(|_| format!("{name} must be a number in Hz"))?;
        if !hz.is_finite() || hz <= 0.0 {
            return Err(format!("{name} must be finite and greater than zero"));
        }
        *destination = Some(hz);
    }
    Ok((test, note))
}

#[cfg(test)]
mod tests {
    use super::parse_frequencies;
    #[test]
    fn frequency_options_are_independent_and_validate_values() {
        let parse = |args: &[&str]| parse_frequencies(args.iter().map(|s| (*s).to_owned()));
        assert_eq!(parse(&[]), Ok((None, None)));
        assert_eq!(parse(&["--test_freq", "440.5"]), Ok((Some(440.5), None)));
        assert_eq!(parse(&["--note_freq=110"]), Ok((None, Some(110.0))));
        assert_eq!(
            parse(&["--test_freq=440", "--note_freq", "440"]),
            Ok((Some(440.0), Some(440.0)))
        );
        for flag in ["--note_freq", "--test_freq"] {
            assert!(parse(&[flag]).is_err());
            for value in ["NaN", "inf", "0", "-1", "abc"] {
                assert!(parse(&[flag, value]).is_err());
            }
            assert!(parse(&[flag, "440", flag, "220"]).is_err());
        }
    }
}
