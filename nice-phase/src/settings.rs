//! Standalone phaser preferences, kept in the same file as the audio settings.
//!
//! The five parameters are what the standalone remembers between runs. Each
//! is an `Option`: `None` means the knob was never moved and the parameter's
//! own default answers, so the file records the player's moves rather than a
//! second copy of the plugin that could drift from the first.
use serde::{Deserialize, Serialize};

pub use nice_common::settings::{OUR_INPUT_LATENCY_DEFAULT, OutputChannels, WindowGeom};

pub type Settings = nice_common::Settings<PhasePreferences>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhasePreferences {
    pub speed: Option<f32>,
    pub mode: Option<f32>,
    pub depth: Option<f32>,
    pub mix: Option<f32>,
    pub out_db: Option<f32>,
}

impl nice_common::Preferences for PhasePreferences {
    const FILE_NAME: &'static str = "nice-phase-settings.ron";
    const APP_NAME: &'static str = "nice-phase";

    fn is_blank(&self) -> bool {
        self.speed.is_none()
            && self.mode.is_none()
            && self.depth.is_none()
            && self.mix.is_none()
            && self.out_db.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_preferences_round_trip_through_ron() {
        let path = frost::scratch::claim("nice-phase-ron").join("settings.ron");
        let settings = Settings {
            backend: Some("core-audio".into()),
            preferences: PhasePreferences {
                speed: Some(0.35),
                mode: Some(1.0),
                depth: Some(0.6),
                out_db: Some(3.5),
                ..Default::default()
            },
            ..Default::default()
        };
        settings.save_to(&path).unwrap();
        let (restored, warning) = Settings::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(restored, settings);
        // Never touched means never claimed: the defaults stay the plugin's.
        assert!(Settings::default().mix.is_none());
        assert!(Settings::default().is_blank());
    }

    #[test]
    fn parameters_and_audio_share_a_flat_configuration() {
        let settings = Settings {
            backend: Some("core-audio".into()),
            preferences: PhasePreferences {
                speed: Some(0.7),
                mix: Some(0.9),
                ..Default::default()
            },
            ..Default::default()
        };
        let path = frost::scratch::claim("nice-phase-flat-ron").join("settings.ron");
        settings.save_to(&path).unwrap();
        let value: ron::Value = ron::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let ron::Value::Map(fields) = value else {
            panic!("expected settings map");
        };
        assert_eq!(
            fields.get(&ron::Value::from("backend")),
            Some(&ron::Value::from(Some("core-audio")))
        );
        assert_eq!(
            fields.get(&ron::Value::from("speed")),
            Some(&ron::Value::from(Some(0.7)))
        );
        assert!(fields.get(&ron::Value::from("preferences")).is_none());
        assert!(Settings::path().ends_with("nice-phase-settings.ron"));
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn disabling_audio_preserves_the_phaser() {
        let settings = Settings {
            backend: Some("dummy".into()),
            preferences: PhasePreferences {
                speed: Some(2.5),
                ..Default::default()
            },
            ..Default::default()
        };
        let resolved = nice_common::devices::Snapshot::default()
            .resolve_settings(&settings)
            .unwrap();
        assert_eq!(resolved.speed, Some(2.5));
    }
}
