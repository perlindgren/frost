//! Standalone Ecomplex preferences, kept in the same file as the audio
//! settings.
//!
//! The seven controls are what the standalone remembers between runs. Each is
//! an `Option`: `None` means the control was never moved and the parameter's
//! own default answers, so the file records the player's moves rather than a
//! second copy of the plugin that could drift from the first. Same arrangement
//! as nice-echo and nice-wah, and the same flat RON shape the shared panel
//! writes — application preferences and audio settings side by side, because a
//! player editing this file by hand is editing one thing: their setup.
use serde::{Deserialize, Serialize};

pub use nice_common::settings::{OUR_INPUT_LATENCY_DEFAULT, OutputChannels, WindowGeom};

pub type Settings = nice_common::Settings<EcomplexPreferences>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EcomplexPreferences {
    pub mic_level: Option<f32>,
    pub gtr_level: Option<f32>,
    pub color: Option<f32>,
    pub delay_time: Option<f32>,
    pub recount: Option<f32>,
    pub timbre: Option<f32>,
    pub echo_volume: Option<f32>,
}

impl nice_common::Preferences for EcomplexPreferences {
    const FILE_NAME: &'static str = "nice-ecomplex-settings.ron";
    const APP_NAME: &'static str = "nice-ecomplex";

    fn is_blank(&self) -> bool {
        self.mic_level.is_none()
            && self.gtr_level.is_none()
            && self.color.is_none()
            && self.delay_time.is_none()
            && self.recount.is_none()
            && self.timbre.is_none()
            && self.echo_volume.is_none()
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
        let path = frost::scratch::claim("nice-ecomplex-ron").join("settings.ron");
        let settings = Settings {
            backend: Some("core-audio".into()),
            preferences: EcomplexPreferences {
                delay_time: Some(0.6),
                recount: Some(0.9),
                echo_volume: Some(0.5),
                ..Default::default()
            },
            ..Default::default()
        };
        settings.save_to(&path).unwrap();
        let (restored, warning) = Settings::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(restored, settings);
        // Never touched means never claimed: the defaults stay the plugin's.
        assert!(Settings::default().delay_time.is_none());
        assert!(Settings::default().is_blank());
    }

    #[test]
    fn parameters_and_audio_share_a_flat_configuration() {
        let settings = Settings {
            backend: Some("core-audio".into()),
            preferences: EcomplexPreferences {
                delay_time: Some(0.45),
                color: Some(0.3),
                ..Default::default()
            },
            ..Default::default()
        };
        let path = frost::scratch::claim("nice-ecomplex-flat-ron").join("settings.ron");
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
        for key in ["backend", "delay_time", "color"] {
            assert!(
                fields.get(&ron::Value::from(key)).is_some(),
                "{key} is missing from the file: {text}"
            );
        }
        assert!(fields.get(&ron::Value::from("preferences")).is_none());
        // Values compared through the loader the panel uses, not against a
        // hand-built `ron::Value`, so a width difference in how ron writes a
        // number cannot make the same setting read as a different one.
        let (back, _) = Settings::load_from(&path);
        assert_eq!(back.backend.as_deref(), Some("core-audio"));
        assert_eq!(back.delay_time, Some(0.45));
        assert_eq!(back.color, Some(0.3));
        assert!(Settings::path().ends_with("nice-ecomplex-settings.ron"));
    }

    /// `is_blank` decides the settings panel's cold-start question, so a field
    /// the check forgets is a saved move that gets read back as a first run and
    /// overwritten on the next Apply. The list below is the same set of
    /// controls the check walks — a new preference belongs in both, and this is
    /// the test that says so.
    #[test]
    fn every_saved_move_reads_as_a_move() {
        type Moved = fn(&mut EcomplexPreferences);
        let moved: [(&str, Moved); 7] = [
            ("mic_level", |p| p.mic_level = Some(0.5)),
            ("gtr_level", |p| p.gtr_level = Some(0.5)),
            ("color", |p| p.color = Some(0.5)),
            ("delay_time", |p| p.delay_time = Some(0.5)),
            ("recount", |p| p.recount = Some(0.5)),
            ("timbre", |p| p.timbre = Some(0.5)),
            ("echo_volume", |p| p.echo_volume = Some(0.5)),
        ];
        for (name, set) in moved {
            let mut prefs = EcomplexPreferences::default();
            set(&mut prefs);
            assert!(!prefs.is_blank(), "{name} is invisible to is_blank");
        }
        assert!(EcomplexPreferences::default().is_blank());
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn disabling_audio_preserves_the_machine_settings() {
        let settings = Settings {
            backend: Some("dummy".into()),
            preferences: EcomplexPreferences {
                recount: Some(0.72),
                ..Default::default()
            },
            ..Default::default()
        };
        let resolved = nice_common::devices::Snapshot::default()
            .resolve_settings(&settings)
            .unwrap();
        assert_eq!(resolved.recount, Some(0.72));
    }
}
