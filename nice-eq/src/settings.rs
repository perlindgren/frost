//! Standalone equaliser preferences.
//!
//! The seven faders, the output trim, the mode, and the three view settings
//! are what the standalone remembers between runs. Each is an `Option`:
//! `None` means the control was never moved and the parameter's own default
//! answers, so the file records the player's moves rather than a second copy
//! of the plugin that could drift from the first.
use serde::{Deserialize, Serialize};

use crate::dsp::BANDS;

pub use nice_common::settings::{OUR_INPUT_LATENCY_DEFAULT, OutputChannels, WindowGeom};

pub type Settings = nice_common::Settings<EqPreferences>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EqPreferences {
    /// The mode as the host sees it: integer 0 is `G7`.
    pub mode: Option<i32>,
    /// The seven faders in decibels, low band first. One array, because the
    /// seven are one control surface: a file that named them apart would
    /// invite someone to edit six of them.
    pub bands: Option<[f32; BANDS]>,
    /// The output trim, in decibels.
    pub gain_db: Option<f32>,
    /// What the analyser is allowed to do: 0 off, 1 top, 2 all.
    pub scope: Option<i32>,
    /// Which end of the box the analyser reads: 0 in, 1 out, 2 both.
    pub tap: Option<i32>,
    /// Bars or interpolated line: 0 or 1.
    pub style: Option<i32>,
    /// The selected transform length as a position in the list: 0 is 1 k.
    pub fft_size: Option<i32>,
}

impl nice_common::Preferences for EqPreferences {
    const FILE_NAME: &'static str = "nice-eq-settings.ron";
    const APP_NAME: &'static str = "nice-eq";

    fn is_blank(&self) -> bool {
        self.mode.is_none()
            && self.bands.is_none()
            && self.gain_db.is_none()
            && self.scope.is_none()
            && self.tap.is_none()
            && self.style.is_none()
            && self.fft_size.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_preferences_round_trip_through_ron() {
        let path = frost::scratch::claim("nice-eq-ron").join("settings.ron");
        let settings = Settings {
            backend: Some("core-audio".into()),
            preferences: EqPreferences {
                mode: Some(0),
                bands: Some([0.0, 3.0, 6.0, 7.5, -6.0, -3.0, 1.5]),
                gain_db: Some(-4.25),
                scope: Some(2),
                tap: Some(0),
                style: Some(1),
                fft_size: Some(3),
            },
            ..Default::default()
        };
        settings.save_to(&path).unwrap();
        let (restored, warning) = Settings::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(restored, settings);
        // Never touched means never claimed: the defaults stay the plugin's.
        assert!(Settings::default().bands.is_none());
        assert!(Settings::default().is_blank());
    }

    #[test]
    fn parameters_and_audio_share_a_flat_configuration() {
        let settings = Settings {
            backend: Some("core-audio".into()),
            preferences: EqPreferences {
                gain_db: Some(2.5),
                ..Default::default()
            },
            ..Default::default()
        };
        let path = frost::scratch::claim("nice-eq-flat-ron").join("settings.ron");
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
            fields.get(&ron::Value::from("gain_db")),
            Some(&ron::Value::from(Some(2.5f32)))
        );
        assert!(fields.get(&ron::Value::from("preferences")).is_none());
        assert!(Settings::path().ends_with("nice-eq-settings.ron"));
    }

    /// A file from an older build of the same plugin is the common case, not
    /// an error: a field nobody wrote reads as `None`, and the plugin's own
    /// default answers for it.
    #[test]
    fn a_file_missing_a_setting_still_reads() {
        let path = frost::scratch::claim("nice-eq-partial-ron").join("settings.ron");
        std::fs::write(&path, "(bands:(0.0,0.0,0.0,0.0,0.0,0.0,0.0))").unwrap();
        let (restored, warning) = Settings::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(restored.preferences.bands, Some([0.0; BANDS]));
        assert_eq!(restored.preferences.scope, None);
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn disabling_audio_preserves_the_equaliser() {
        let settings = Settings {
            backend: Some("dummy".into()),
            preferences: EqPreferences {
                gain_db: Some(-3.0),
                ..Default::default()
            },
            ..Default::default()
        };
        let resolved = nice_common::devices::Snapshot::default()
            .resolve_settings(&settings)
            .unwrap();
        assert_eq!(resolved.gain_db, Some(-3.0));
    }
}
