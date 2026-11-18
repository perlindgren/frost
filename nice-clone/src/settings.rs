//! Standalone controls share nice-common's audio and window settings.
pub use nice_common::settings::{OUR_INPUT_LATENCY_DEFAULT, OutputChannels, WindowGeom};
use serde::{Deserialize, Serialize};
pub type Settings = nice_common::Settings<ClonePreferences>;
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClonePreferences {
    pub gain: Option<f32>,
    pub treble: Option<f32>,
    pub output: Option<f32>,
    /// Actual multiplier: 1 (Off), 2, 4, or 8. Missing retains the original 8x.
    pub oversampling: Option<u32>,
}
impl nice_common::Preferences for ClonePreferences {
    const FILE_NAME: &'static str = "nice-clone-settings.ron";
    const APP_NAME: &'static str = "nice-clone";
    fn is_blank(&self) -> bool {
        self.gain.is_none()
            && self.treble.is_none()
            && self.output.is_none()
            && self.oversampling.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClonePlugin, seed_standalone_params};

    #[test]
    fn controls_round_trip_with_audio_settings_and_seed_clamped_values() {
        let settings = Settings {
            backend: Some("dummy".into()),
            preferences: ClonePreferences {
                gain: Some(0.2),
                treble: Some(0.8),
                output: Some(0.6),
                oversampling: Some(2),
            },
            ..Default::default()
        };
        let path = frost::scratch::claim("nice-clone-settings").join("settings.ron");
        settings.save_to(&path).unwrap();
        let (restored, warning) = Settings::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(restored, settings);
        let plugin = ClonePlugin::default();
        seed_standalone_params(&plugin.shared, &plugin.params, &restored.preferences);
        assert_eq!(plugin.shared.oversampling.load(), 1);
        assert_eq!(plugin.shared.gain.load(), 0.2);
        assert_eq!(plugin.shared.treble.load(), 0.8);
        assert_eq!(plugin.shared.output.load(), 0.6);
        seed_standalone_params(
            &plugin.shared,
            &plugin.params,
            &ClonePreferences {
                gain: Some(9.0),
                treble: Some(-2.0),
                output: None,
                oversampling: None,
            },
        );
        assert_eq!(plugin.shared.gain.load(), 1.0);
        assert_eq!(plugin.shared.treble.load(), 0.0);
        assert_eq!(plugin.shared.output.load(), 0.6);
    }
}

#[cfg(test)]
mod oversampling_preferences_tests {
    use super::*;
    use crate::dsp::Oversampling;

    #[test]
    fn old_settings_keep_eightfold_and_every_selection_round_trips() {
        let old: ClonePreferences =
            ron::from_str("(gain: Some(0.2), treble: None, output: None)").unwrap();
        assert_eq!(old.oversampling, None);
        let plugin = crate::ClonePlugin::default();
        crate::seed_standalone_params(&plugin.shared, &plugin.params, &old);
        assert_eq!(plugin.shared.oversampling.load(), Oversampling::X8.index());
        for mode in Oversampling::ALL {
            let preferences = ClonePreferences {
                oversampling: Some(mode.factor() as u32),
                ..Default::default()
            };
            assert!(!nice_common::Preferences::is_blank(&preferences));
            let saved = ron::to_string(&preferences).unwrap();
            let restored: ClonePreferences = ron::from_str(&saved).unwrap();
            assert_eq!(restored, preferences);
            crate::seed_standalone_params(&plugin.shared, &plugin.params, &restored);
            assert_eq!(plugin.shared.oversampling.load(), mode.index());
        }
    }
}
