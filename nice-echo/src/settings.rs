//! Standalone Nice Echo preferences, kept in the same file as the audio
//! settings.
//!
//! The nine controls are what the standalone remembers between runs. Each is
//! an `Option`: `None` means the control was never moved and the parameter's
//! own default answers, so the file records the player's moves rather than a
//! second copy of the plugin that could drift from the first. Same arrangement
//! as nice-wah and nice-pi-face, and the same flat RON shape the shared panel
//! writes — application preferences and audio settings side by side, because a
//! player editing this file by hand is editing one thing: their setup.
use serde::{Deserialize, Serialize};

pub use nice_common::settings::{OUR_INPUT_LATENCY_DEFAULT, OutputChannels, WindowGeom};

pub type Settings = nice_common::Settings<EchoPreferences>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EchoPreferences {
    pub mode: Option<i32>,
    pub repeat_rate: Option<f32>,
    pub intensity: Option<f32>,
    pub echo_volume: Option<f32>,
    pub reverb_volume: Option<f32>,
    pub treble: Option<f32>,
    pub bass: Option<f32>,
    pub echo_switch: Option<i32>,
    pub output_level: Option<i32>,
}

impl nice_common::Preferences for EchoPreferences {
    const FILE_NAME: &'static str = "nice-echo-settings.ron";
    const APP_NAME: &'static str = "nice-echo";

    fn is_blank(&self) -> bool {
        self.mode.is_none()
            && self.repeat_rate.is_none()
            && self.intensity.is_none()
            && self.echo_volume.is_none()
            && self.reverb_volume.is_none()
            && self.treble.is_none()
            && self.bass.is_none()
            && self.echo_switch.is_none()
            && self.output_level.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `is_blank` is a trait method, and the flattened preferences half of a
    // `Settings` reaches it only through a deref — which does not carry a trait
    // into scope with it.
    use nice_common::Preferences;

    #[test]
    fn application_preferences_round_trip_through_ron() {
        let path = frost::scratch::claim("nice-echo-ron").join("settings.ron");
        let settings = Settings {
            backend: Some("core-audio".into()),
            preferences: EchoPreferences {
                mode: Some(11),
                intensity: Some(0.8),
                echo_switch: Some(1),
                ..Default::default()
            },
            ..Default::default()
        };
        settings.save_to(&path).unwrap();
        let (restored, warning) = Settings::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(restored, settings);
        // Never touched means never claimed: the defaults stay the plugin's.
        assert!(Settings::default().mode.is_none());
        assert!(Settings::default().is_blank());
    }

    #[test]
    fn parameters_and_audio_share_a_flat_configuration() {
        let settings = Settings {
            backend: Some("core-audio".into()),
            preferences: EchoPreferences {
                repeat_rate: Some(0.45),
                output_level: Some(1),
                ..Default::default()
            },
            ..Default::default()
        };
        let path = frost::scratch::claim("nice-echo-flat-ron").join("settings.ron");
        settings.save_to(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let value: ron::Value = ron::from_str(&text).unwrap();
        let ron::Value::Map(fields) = value else {
            panic!("expected settings map");
        };
        // One flat map: the shared panel's audio fields and this plugin's
        // parameters as siblings. A nested `preferences` group is a file the
        // panel would read as empty and overwrite on the next Apply. Only the
        // audio fields that were actually chosen are written; the plugin's own
        // parameters are written even when `None`, which is how the loader
        // tells "never moved" from "moved to the default".
        for key in ["backend", "repeat_rate", "output_level"] {
            assert!(
                fields.get(&ron::Value::from(key)).is_some(),
                "{key} is missing from the file: {text}"
            );
        }
        assert!(fields.get(&ron::Value::from("preferences")).is_none());
        // Values compared through the loader the panel uses, not against a
        // hand-built `ron::Value`: ron writes a small non-negative `i32` as the
        // narrowest unsigned number that fits it, so two `Value` trees can
        // differ in width while describing the same setting.
        let (back, _) = Settings::load_from(&path);
        assert_eq!(back.backend.as_deref(), Some("core-audio"));
        assert_eq!(back.repeat_rate, Some(0.45));
        assert_eq!(back.output_level, Some(1));
        assert!(Settings::path().ends_with("nice-echo-settings.ron"));
    }

    /// `is_blank` decides the settings panel's cold-start question, so a field
    /// the check forgets is a saved move that gets read back as a first run and
    /// overwritten on the next Apply. The list below is the same set of
    /// controls the check walks — a new preference belongs in both, and this is
    /// the test that says so.
    #[test]
    fn every_saved_move_reads_as_a_move() {
        type Moved = fn(&mut EchoPreferences);
        let moved: [(&str, Moved); 9] = [
            ("mode", |p| p.mode = Some(1)),
            ("repeat_rate", |p| p.repeat_rate = Some(0.5)),
            ("intensity", |p| p.intensity = Some(0.5)),
            ("echo_volume", |p| p.echo_volume = Some(0.5)),
            ("reverb_volume", |p| p.reverb_volume = Some(0.5)),
            ("treble", |p| p.treble = Some(0.5)),
            ("bass", |p| p.bass = Some(0.5)),
            ("echo_switch", |p| p.echo_switch = Some(1)),
            ("output_level", |p| p.output_level = Some(1)),
        ];
        for (name, set) in moved {
            let mut prefs = EchoPreferences::default();
            set(&mut prefs);
            assert!(!prefs.is_blank(), "{name} is invisible to is_blank");
        }
        assert!(EchoPreferences::default().is_blank());
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn disabling_audio_preserves_the_pedal_settings() {
        let settings = Settings {
            backend: Some("dummy".into()),
            preferences: EchoPreferences {
                intensity: Some(0.72),
                ..Default::default()
            },
            ..Default::default()
        };
        let resolved = nice_common::devices::Snapshot::default()
            .resolve_settings(&settings)
            .unwrap();
        assert_eq!(resolved.intensity, Some(0.72));
    }
}
