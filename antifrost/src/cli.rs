//! How the hill launches: the two logging shapes (stderr on the
//! desktop, the devtools console in the page), the beat
//! sentinel's clock and stutter line, and the command line the
//! native build answers to.

use crate::*;

/// A browser has no stderr: route whatever log survives to the
/// devtools console, and turn panics into console errors. (The crate
/// compiles `log` calls out of release builds — `release_max_level_off`
/// — and wasm builds are release builds, so in the shipped page this
/// mostly catches panics; `web/index.html` carries a page-side probe
/// for the rest.)
#[cfg(target_arch = "wasm32")]
pub(crate) fn init_logging() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        console_error_panic_hook::set_once();
        struct Console;
        impl log::Log for Console {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                let line = format!("[{:?}] {}", record.level(), record.args());
                web_sys::console::log_1(&line.into());
            }
            fn flush(&self) {}
        }
        static LOGGER: Console = Console;
        if log::set_logger(&LOGGER).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
        }
    });
}

/// Milliseconds on the wall clock: the browser's own clock in the
/// page, epoch milliseconds on the desktop — one shape for the beat
/// sentinel, whichever side of the wire it keeps time on.
#[cfg(target_arch = "wasm32")]
pub(crate) fn now_ms() -> f64 {
    js_sys::Date::now()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now_ms() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

/// One stutter line, said where the line lands: the devtools console
/// on the web — where `log` calls have been compiled out of the
/// release build the page ships — and the ordinary log on the
/// desktop, where they have not.
#[cfg(target_arch = "wasm32")]
pub(crate) fn report_gap(gap: f64, page: Page) {
    web_sys::console::warn_1(
        &format!("antifrost: {gap:.2} s since the last frame ({page:?} up)").into(),
    );
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn report_gap(gap: f64, page: Page) {
    log::warn!("{gap:.2} s since the last frame ({page:?} up)");
}

/// The command line's arguments.
#[cfg(not(target_arch = "wasm32"))]
#[derive(clap::Parser)]
pub(crate) struct Cli {
    /// Present frames in sync with the display's vertical blank. `off`
    /// uncaps the frame rate — the hill has the headroom (the experiment
    /// hit 200 fps), but the cadence judders, and the tearing present
    /// (`Immediate`) bypasses VRR, so vsync stays on unless this flag
    /// says otherwise.
    ///
    /// An `Option`: a bare `bool` field is clap's zero-arg `--flag`,
    /// which has no value to parse.
    #[arg(long, value_name = "on|off", value_parser = parse_on_off)]
    pub(crate) vsync: Option<bool>,

    /// Run for SECONDS, log the mean frame rate, and quit on its own
    /// — the A/B instrument the benchmarking notes ask of every
    /// performance claim. Frames count only after the first two
    /// seconds, so the clocks have time to wake; everything else the
    /// launch decides, settings file included.
    #[arg(long, value_name = "SECONDS")]
    pub(crate) bench: Option<f32>,

    /// Cycle the resolution row once on a timer, logging the window's
    /// size before, answered, and settled — the diagnostic for
    /// "did the window actually move". Not a feature; a probe.
    #[arg(long)]
    pub(crate) resize_probe: bool,

    /// Walk any hill from the first launch: every hill unlocks and
    /// the unlock is saved, so the flag need not return. Nothing
    /// else moves — scores, routes, and the descent's kept depth
    /// stay exactly as they were. This opens doors; it does not
    /// wipe a ledger.
    #[arg(long)]
    pub(crate) unlock: bool,
}

/// clap's value parser for the vsync flag: `on`/`off`, any case.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn parse_on_off(s: &str) -> Result<bool, String> {
    if s.eq_ignore_ascii_case("on") {
        Ok(true)
    } else if s.eq_ignore_ascii_case("off") {
        Ok(false)
    } else {
        Err(format!("expected `on` or `off`, got `{s}`"))
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod cli_tests {
    use super::*;

    fn vsync_of(args: &[&str]) -> bool {
        <Cli as clap::Parser>::parse_from(std::iter::once("antifrost").chain(args.iter().copied()))
            .vsync
            .unwrap_or(true)
    }

    #[test]
    fn vsync_defaults_to_on() {
        assert!(
            vsync_of(&[]),
            "no flag: the display's cadence stays the pin"
        );
    }

    #[test]
    fn vsync_flag_toggles_either_way() {
        assert!(!vsync_of(&["--vsync", "off"]), "off uncaps the frame rate");
        assert!(vsync_of(&["--vsync", "on"]), "on is spellable out loud");
        assert!(
            !vsync_of(&["--vsync", "OFF"]),
            "case is not part of the deal"
        );
    }

    #[test]
    fn any_other_vsync_value_is_rejected() {
        let err = <Cli as clap::Parser>::try_parse_from(["antifrost", "--vsync", "maybe"]);
        assert!(err.is_err(), "the flag speaks on or off, nothing else");
    }

    #[test]
    fn unlock_is_a_bare_flag_defaulting_shut() {
        let open = <Cli as clap::Parser>::try_parse_from(["antifrost", "--unlock"])
            .expect("--unlock takes no value");
        assert!(open.unlock, "the flag opens every door");
        let shut = <Cli as clap::Parser>::try_parse_from(["antifrost"]).expect("no args");
        assert!(
            !shut.unlock,
            "without the flag the lock stands as the ledger left it"
        );
    }
}
