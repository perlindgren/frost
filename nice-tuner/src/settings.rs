//! Tuner preferences layered on the shared standalone audio configuration.
pub use nice_common::settings::{
    BACKEND_WHEN_UNSET, OUR_INPUT_LATENCY_DEFAULT, OutputChannels, WindowGeom, buffers_must_match,
    effective_args, rates_must_match,
};
use serde::{Deserialize, Serialize};
pub const FILE_NAME: &str = "nice-tuner-settings.json";
pub type Settings = nice_common::Settings<TunerPreferences>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TunerPreferences {
    pub a4_ref: Option<f32>,
    pub gate_db: Option<f32>,
    pub in_tune_cents: Option<f32>,
    pub gauge_knee_cents: Option<f32>,
}
impl nice_common::Preferences for TunerPreferences {
    const FILE_NAME: &'static str = FILE_NAME;
    const APP_NAME: &'static str = "nice-tuner";
    fn is_blank(&self) -> bool {
        self.a4_ref.is_none()
            && self.gate_db.is_none()
            && self.in_tune_cents.is_none()
            && self.gauge_knee_cents.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder this tree can vouch for, not one it hopes for. These two
    /// tests `unwrap()` every write, so an unusable `%TEMP%` — redirected,
    /// locked by a sync client, denied by the confinement a test runner
    /// happens to live under, or simply full — reports a failure of the
    /// settings code when the only thing wrong is the folder; measured here
    /// as `Access is denied. (os error 5)` on a machine that never denied a
    /// permission. `frost::scratch` claims a directory rather than assuming a
    /// name, and falls back to the workspace's own `target` when temp refuses
    /// (see `src/scratch.rs`, and the note in `.gitignore` about `**/target/`).
    fn geometry_test_path() -> std::path::PathBuf {
        frost::scratch::claim("nice-tuner-geometry").join("settings.json")
    }

    #[test]
    fn resizing_updates_only_geometry_and_preserves_saved_and_unknown_preferences() {
        let path = geometry_test_path();
        let initial = serde_json::json!({
            "backend": "jack", "a4_ref": 432.0, "future_option": {"enabled": true},
            "window": {"x": 10, "y": 20, "width": 640.0, "height": 400.0, "future_window_option": 7}
        });
        std::fs::write(&path, initial.to_string()).unwrap();
        let size = WindowGeom {
            x: None,
            y: None,
            width: 900.0,
            height: 600.0,
        };
        Settings::save_window_to(&path, size).unwrap();
        let updated: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let mut expected = initial;
        expected["window"]["width"] = 900.0.into();
        expected["window"]["height"] = 600.0.into();
        assert_eq!(updated, expected);
        // An unchanged size must leave even the file's formatting untouched.
        let compact = updated.to_string();
        std::fs::write(&path, &compact).unwrap();
        Settings::save_window_to(&path, size).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), compact);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn resizing_can_create_a_geometry_only_config_and_never_overwrites_corrupt_json() {
        let path = geometry_test_path();
        let size = WindowGeom {
            x: None,
            y: None,
            width: 800.0,
            height: 500.0,
        };
        Settings::save_window_to(&path, size).unwrap();
        let restored: Settings =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(restored.window, Some(size));
        assert!(restored.is_blank());
        std::fs::write(&path, "broken config").unwrap();
        assert!(Settings::save_window_to(&path, size).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken config");
        assert!(Settings::save_window_to(&path, WindowGeom { width: 0.0, ..size }).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn output_sample_rate_round_trips_and_is_independent_of_input() {
        let settings = Settings {
            sample_rate: Some(48000.0),
            output_sample_rate: Some(44100.0),
            ..Default::default()
        };
        assert_eq!(
            serde_json::from_str::<Settings>(&serde_json::to_string(&settings).unwrap()).unwrap(),
            settings
        );
        assert_eq!(
            serde_json::from_str::<Settings>(r#"{"sample_rate":48000}"#)
                .unwrap()
                .output_sample_rate,
            None
        );
        assert!(
            !Settings {
                output_sample_rate: Some(44100.0),
                ..Default::default()
            }
            .is_blank()
        );
    }

    #[test]
    fn output_channel_configuration_round_trips_and_legacy_files_follow_default() {
        for route in [OutputChannels::Mono(2), OutputChannels::Stereo([3, 4])] {
            assert_eq!(OutputChannels::parse(&route.raw()), Some(route));
            let settings = Settings {
                output_channels: Some(route),
                ..Default::default()
            };
            let restored: Settings =
                serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
            assert_eq!(restored.output_channels, Some(route));
            assert!(!restored.is_blank());
            let args = effective_args(&restored);
            let at = args
                .iter()
                .position(|arg| arg == "--output-channels")
                .unwrap();
            assert_eq!(args[at + 1], route.raw());
        }
        assert_eq!(
            serde_json::from_str::<Settings>("{}")
                .unwrap()
                .output_channels,
            None
        );
        assert_eq!(OutputChannels::parse("0"), None);
        assert_eq!(OutputChannels::parse("2,2"), None);
    }

    #[test]
    fn buffer_sizes_are_saved_and_passed_independently() {
        let settings = Settings {
            period_size: Some(64),
            output_period_size: Some(256),
            ..Default::default()
        };
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
        let args = effective_args(&restored);
        let input = args
            .iter()
            .position(|arg| arg == "--input-period-size")
            .unwrap();
        let output = args.iter().position(|arg| arg == "--period-size").unwrap();
        assert_eq!(args[input + 1], "64");
        assert_eq!(args[output + 1], "256");
        let output_only = effective_args(&Settings {
            output_period_size: Some(256),
            ..Default::default()
        });
        let input = output_only
            .iter()
            .position(|arg| arg == "--input-period-size")
            .unwrap();
        assert_eq!(output_only[input + 1], "512");
        let legacy: Settings = serde_json::from_str(r#"{"period_size":128}"#).unwrap();
        assert_eq!(legacy.period_size, Some(128));
        assert_eq!(legacy.output_period_size, None);
        assert!(
            !Settings {
                output_period_size: Some(64),
                ..Default::default()
            }
            .is_blank()
        );
    }

    #[test]
    fn buffer_linking_requires_a_shared_device_and_backend_rule() {
        for backend in ["core-audio", "coreaudio", "asio"] {
            assert!(buffers_must_match(
                backend,
                Some("Interface"),
                Some("Interface")
            ));
            assert!(!buffers_must_match(
                backend,
                Some("Microphone"),
                Some("Speakers")
            ));
            assert!(!buffers_must_match(backend, None, None));
        }
        assert!(!buffers_must_match(
            "wasapi",
            Some("Interface"),
            Some("Interface")
        ));
    }

    /// Rate-linking is ASIO's alone: one driver, one clock. CoreAudio keeps
    /// the independent output rate its picker exists for, even where the two
    /// names are the same device.
    #[test]
    fn rate_linking_is_asio_alone() {
        assert!(rates_must_match(
            "asio",
            Some("Interface"),
            Some("Interface")
        ));
        assert!(!rates_must_match("asio", Some("Mic"), Some("Speakers")));
        assert!(!rates_must_match("asio", None, None));
        assert!(!rates_must_match(
            "core-audio",
            Some("Interface"),
            Some("Interface")
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn jack_buffers_follow_one_server_across_different_virtual_devices() {
        assert!(buffers_must_match(
            "jack",
            Some("JACK Input"),
            Some("JACK Output")
        ));
    }

    #[test]
    fn input_channel_loads_legacy_auto_as_unset() {
        for json in ["{}", r#"{"input_channel":null}"#, r#"{"input_channel":0}"#] {
            let settings: Settings = serde_json::from_str(json).unwrap();
            assert_eq!(settings.input_channel, None);
        }
        let settings: Settings = serde_json::from_str(r#"{"input_channel":3}"#).unwrap();
        assert_eq!(settings.input_channel, Some(3));
        let unset = serde_json::to_value(Settings::default()).unwrap();
        assert!(unset["input_channel"].is_null());
    }

    #[test]
    fn window_geometry_round_trips_and_defaults_absent() {
        let s = Settings {
            window: Some(WindowGeom {
                x: Some(-1200),
                y: Some(80),
                width: 900.0,
                height: 560.0,
            }),
            ..Default::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        // Old files (no window key) load as "never saved" → the window
        // centres itself on the next start.
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.window, None);
        // Size-only entries (platforms without a position API omit x/y).
        let size_only: Settings =
            serde_json::from_str(r#"{"window":{"width":800.0,"height":500.0}}"#).unwrap();
        assert_eq!(
            size_only.window,
            Some(WindowGeom {
                x: None,
                y: None,
                width: 800.0,
                height: 500.0
            })
        );
    }

    #[test]
    fn blank_means_no_decision_geometry_never_counts() {
        // A file born from merely moving the window holds no decision:
        // still a cold slate.
        let geom_only = Settings {
            window: Some(WindowGeom {
                x: Some(10),
                y: Some(20),
                width: 800.0,
                height: 500.0,
            }),
            ..Default::default()
        };
        assert!(geom_only.is_blank());
        assert!(Settings::default().is_blank());
        // One value of any class and the slate is no longer blank.
        let one_choice = Settings {
            period_size: Some(512),
            ..Default::default()
        };
        assert!(!one_choice.is_blank());
        let look_only = Settings {
            preferences: crate::settings::TunerPreferences {
                in_tune_cents: Some(3.0),
                ..Default::default()
            },

            ..Default::default()
        };
        assert!(!look_only.is_blank());
    }

    #[test]
    fn settings_file_is_the_only_voice() {
        // The panel's file names everything it decided; nothing else
        // speaks. (The app ignores its own argv entirely — see main.)
        let s = Settings {
            backend: Some("asio".into()),
            input_device: Some("Komplete Audio ASIO Driver".into()),
            output_device: Some("Komplete Audio ASIO Driver".into()),
            sample_rate: Some(48000.0),
            period_size: Some(64),
            ..Default::default()
        };
        let args = effective_args(&s);
        let value_of = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .map(|i| args[i + 1].clone())
        };
        assert_eq!(value_of("--backend").as_deref(), Some("asio"));
        assert_eq!(
            value_of("--input-device").as_deref(),
            Some("Komplete Audio ASIO Driver")
        );
        assert_eq!(
            value_of("--output-device").as_deref(),
            Some("Komplete Audio ASIO Driver")
        );
        assert_eq!(value_of("--sample-rate").as_deref(), Some("48000"));
        assert_eq!(value_of("--input-period-size").as_deref(), Some("64"));
        assert_eq!(value_of("--period-size"), None);
    }

    #[test]
    fn settings_file_beats_our_input_latency_default() {
        let s = Settings {
            input_latency: Some(0.02),
            ..Default::default()
        };
        let args = effective_args(&s);
        let i = args.iter().position(|a| a == "--input-latency").unwrap();
        assert_eq!(args[i + 1], "0.02");
        assert!(!args.iter().any(|a| a == "0.005"));
    }

    #[test]
    fn no_opinions_yet_boots_silent_not_default() {
        let args = effective_args(&Settings::default());
        let i = args.iter().position(|a| a == "--input-latency").unwrap();
        assert_eq!(args[i + 1], "0.005");
        // Unset backend is a CHOICE PENDING, not a default: dummy audio,
        // nothing connected, nothing probed.
        let b = args.iter().position(|a| a == "--backend").unwrap();
        assert_eq!(args[b + 1], "dummy");
        // Nothing else invented.
        assert_eq!(args.len(), 5);
    }

    #[test]
    fn unset_backend_never_touches_jack() {
        // The rule: no device is touched until the user speaks. An unset
        // backend must name `dummy` — never nice-plug's `Auto` (which
        // tries jackd first), never `jack`, never a system host.
        let args = effective_args(&Settings::default());
        assert!(!args.iter().any(|a| a == "auto"));
        assert!(!args.iter().any(|a| a == "jack"));
        assert!(!args.iter().any(|a| a == "wasapi"));
        assert!(!args.iter().any(|a| a == "core-audio"));
        assert!(!args.iter().any(|a| a == "alsa"));
    }

    #[test]
    fn chosen_auto_passes_through() {
        // Explicit `auto` in the file is a CHOICE — it passes through
        // untouched (that is the one road to jack-if-present, and the
        // user opted into it with their own hand).
        let s = Settings {
            backend: Some("auto".into()),
            ..Default::default()
        };
        let args = effective_args(&s);
        let b = args.iter().position(|a| a == "--backend").unwrap();
        assert_eq!(args[b + 1], "auto");
    }

    #[test]
    fn json_round_trip_and_lenient_reading() {
        let s = Settings {
            preferences: crate::settings::TunerPreferences {
                a4_ref: Some(432.0),
                in_tune_cents: Some(2.0),
                gauge_knee_cents: Some(7.0),
                ..Default::default()
            },
            backend: Some("asio".into()),
            input_device: Some("Komplete Input 1/2".into()),
            period_size: Some(64),
            input_channel: Some(1),

            ..Default::default()
        };
        let text = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back, s);
        // Unknown fields ignored, missing fields None — forward compatible.
        let loose: Settings =
            serde_json::from_str(r#"{"backend":"wasapi","future_thing":42}"#).unwrap();
        assert_eq!(loose.backend.as_deref(), Some("wasapi"));
        assert_eq!(loose.period_size, None);
    }

    #[test]
    fn normalize_repairs_legacy_coreaudio_key() {
        let mut s = Settings {
            backend: Some("coreaudio".into()),
            ..Default::default()
        };
        s.normalize();
        assert_eq!(s.backend.as_deref(), Some("core-audio"));
        // Correct values pass through untouched.
        let mut ok = Settings {
            backend: Some("wasapi".into()),
            ..Default::default()
        };
        ok.normalize();
        assert_eq!(ok.backend.as_deref(), Some("wasapi"));
    }

    #[test]
    fn named_jack_destinations_round_trip_and_legacy_virtual_devices_migrate() {
        let mut settings = Settings {
            backend: Some("jack".into()),
            input_device: Some("JACK Input".into()),
            output_device: Some("JACK Output".into()),
            input_channel: Some(2),
            output_channels: Some(OutputChannels::Stereo([1, 2])),
            jack_output_right: Some("interface:playback_R".into()),
            ..Default::default()
        };
        settings.normalize();
        assert_eq!(settings.input_device, None);
        assert_eq!(settings.output_device, None);
        assert_eq!(settings.input_channel, None);
        assert_eq!(settings.output_channels, None);
        let serialized = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            serde_json::from_str::<Settings>(&serialized).unwrap(),
            settings
        );
        assert_eq!(
            settings.jack_output_right.as_deref(),
            Some("interface:playback_R")
        );
    }
}
