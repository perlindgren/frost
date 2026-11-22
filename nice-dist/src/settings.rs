//! Standalone controls share nice-common's audio and window settings.
pub use nice_common::settings::{OUR_INPUT_LATENCY_DEFAULT, OutputChannels, WindowGeom};
use serde::{Deserialize, Serialize};
pub type Settings = nice_common::Settings<DistPreferences>;
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DistPreferences {
    pub drive: Option<f32>,
    pub tone: Option<f32>,
    pub output: Option<f32>,
    /// Circuit number: 1 = BS808, 2 = BS9, 3 = SH-1, 4 = OH-1, 5 = CAT, 6 = Nice Clone. Missing selects BS808.
    pub variant: Option<u32>,
    /// 1 (Off), 2, 4 or 8; defaults to 8x.
    pub oversampling: Option<u32>,
}
impl nice_common::Preferences for DistPreferences {
    const FILE_NAME: &'static str = "nice-dist-settings.ron";
    const APP_NAME: &'static str = "nice-dist";
    fn is_blank(&self) -> bool {
        self.drive.is_none()
            && self.tone.is_none()
            && self.output.is_none()
            && self.variant.is_none()
            && self.oversampling.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standalone_controls_and_variant_round_trip_and_seed() {
        for variant in crate::controls::Variant::ALL {
            let settings = Settings {
                preferences: DistPreferences {
                    drive: Some(0.2),
                    tone: Some(0.8),
                    output: Some(0.6),
                    variant: Some(variant.number()),
                    oversampling: Some(8),
                },
                ..Default::default()
            };
            let path = frost::scratch::claim("nice-dist-settings").join("settings.ron");
            settings.save_to(&path).unwrap();
            let (restored, warning) = Settings::load_from(&path);
            assert_eq!(warning, None);
            assert_eq!(restored, settings);
            let plugin = crate::DistPlugin::default();
            crate::seed_standalone_params(&plugin.shared, &plugin.params, &restored.preferences);
            assert_eq!(plugin.shared.drive.load(), 0.2);
            assert_eq!(plugin.shared.tone.load(), 0.8);
            assert_eq!(plugin.shared.output.load(), 0.6);
            assert_eq!(plugin.shared.variant.load(), variant.index());
        }
    }
}
