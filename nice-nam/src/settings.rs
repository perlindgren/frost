//! Standalone NAM preferences; filenames are independent of nice-tuner.
pub use nice_settings::settings::{OUR_INPUT_LATENCY_DEFAULT, OutputChannels, WindowGeom};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub type Settings = nice_settings::Settings<NamPreferences>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NamPreferences {
    pub model_name: Option<String>,
    pub model_path: Option<PathBuf>,
    pub cabinet_path: Option<PathBuf>,
}

impl nice_settings::Preferences for NamPreferences {
    const FILE_NAME: &'static str = "nice-nam-settings.json";
    const APP_NAME: &'static str = "nice-nam";

    fn is_blank(&self) -> bool {
        self.model_name.is_none() && self.model_path.is_none() && self.cabinet_path.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let value = serde_json::to_value(&settings).unwrap();
        assert_eq!(value["backend"], "core-audio");
        assert_eq!(value["model_path"], "models/amp.nam");
        assert!(value.get("preferences").is_none());
        let restored: Settings = serde_json::from_value(value).unwrap();
        assert_eq!(restored, settings);
        assert!(Settings::path().ends_with("nice-nam-settings.json"));
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
        let resolved = nice_settings::devices::Snapshot::default()
            .resolve_settings(&settings)
            .unwrap();
        assert_eq!(resolved.model_path, settings.model_path);
    }
}
