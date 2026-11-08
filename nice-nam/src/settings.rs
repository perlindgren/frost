//! Standalone NAM preferences; filenames are independent of nice-tuner.
pub use nice_common::settings::{OUR_INPUT_LATENCY_DEFAULT, OutputChannels, WindowGeom};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type Settings = nice_common::Settings<NamPreferences>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NamPreferences {
    pub gate_db: Option<f32>,
    pub model_name: Option<String>,
    pub model_path: Option<PathBuf>,
    pub cabinet_path: Option<PathBuf>,
}

impl nice_common::Preferences for NamPreferences {
    const FILE_NAME: &'static str = "nice-nam-settings.ron";
    const APP_NAME: &'static str = "nice-nam";

    fn is_blank(&self) -> bool {
        self.model_name.is_none()
            && self.model_path.is_none()
            && self.cabinet_path.is_none()
            && self.gate_db.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_preferences_round_trip_through_ron() {
        let path = frost::scratch::claim("nice-nam-ron").join("settings.ron");
        let settings = Settings {
            backend: Some("core-audio".into()),
            preferences: NamPreferences {
                model_path: Some("models/amp.nam".into()),
                gate_db: Some(-40.0),
                ..Default::default()
            },
            ..Default::default()
        };
        settings.save_to(&path).unwrap();
        let (restored, warning) = Settings::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(restored, settings);
        assert_eq!(Settings::default().gate_db.unwrap_or(-60.0), -60.0);
    }

    #[test]
    fn model_identity_and_audio_share_a_flat_configuration() {
        let settings = Settings {
            backend: Some("core-audio".into()),
            preferences: NamPreferences {
                model_name: Some("Amp".into()),
                model_path: Some("models/amp.nam".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let path = frost::scratch::claim("nice-nam-flat-ron").join("settings.ron");
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
            fields.get(&ron::Value::from("model_path")),
            Some(&ron::Value::from(Some("models/amp.nam")))
        );
        assert!(fields.get(&ron::Value::from("preferences")).is_none());
        assert!(Settings::path().ends_with("nice-nam-settings.ron"));
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn disabling_audio_preserves_model_preferences() {
        let settings = Settings {
            backend: Some("dummy".into()),
            preferences: NamPreferences {
                model_path: Some("models/amp.nam".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let resolved = nice_common::devices::Snapshot::default()
            .resolve_settings(&settings)
            .unwrap();
        assert_eq!(resolved.model_path, settings.model_path);
    }
}
