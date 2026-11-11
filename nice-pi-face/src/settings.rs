//! Standalone Pi-Fase preferences, kept in the same file as the audio
//! settings.
//!
//! The fourteen parameters are what the standalone remembers between runs. Each
//! is an `Option`: `None` means the control was never moved and the parameter's
//! own default answers, so the file records the player's moves rather than a
//! second copy of the plugin that could drift from the first. Same arrangement
//! as nice-wah and nice-nam, and the same flat RON shape the shared panel
//! writes — application preferences and audio settings side by side, because a
//! player editing this file by hand is editing one thing: their setup.
use serde::{Deserialize, Serialize};

pub use nice_common::settings::{OUR_INPUT_LATENCY_DEFAULT, OutputChannels, WindowGeom};

pub type Settings = nice_common::Settings<PiFacePreferences>;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PiFacePreferences {
    pub gen1_rate: Option<f32>,
    pub gen1_shape: Option<i32>,
    pub gen1_rate_source: Option<i32>,
    pub gen2_rate: Option<f32>,
    pub gen2_shape: Option<i32>,
    pub a_depth: Option<f32>,
    pub a_feedback: Option<f32>,
    pub a_sweep: Option<i32>,
    pub b_depth: Option<f32>,
    pub b_feedback: Option<f32>,
    pub b_sync: Option<i32>,
    pub b_sweep: Option<i32>,
    pub b_input: Option<i32>,
    pub pedal: Option<f32>,
}

impl nice_common::Preferences for PiFacePreferences {
    const FILE_NAME: &'static str = "nice-pi-face-settings.ron";
    const APP_NAME: &'static str = "nice-pi-face";

    fn is_blank(&self) -> bool {
        self.gen1_rate.is_none()
            && self.gen1_shape.is_none()
            && self.gen1_rate_source.is_none()
            && self.gen2_rate.is_none()
            && self.gen2_shape.is_none()
            && self.a_depth.is_none()
            && self.a_feedback.is_none()
            && self.a_sweep.is_none()
            && self.b_depth.is_none()
            && self.b_feedback.is_none()
            && self.b_sync.is_none()
            && self.b_sweep.is_none()
            && self.b_input.is_none()
            && self.pedal.is_none()
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
        let path = frost::scratch::claim("nice-pi-face-ron").join("settings.ron");
        let settings = Settings {
            backend: Some("core-audio".into()),
            preferences: PiFacePreferences {
                gen1_rate: Some(2.5),
                b_sweep: Some(2),
                b_feedback: Some(0.8),
                b_input: Some(0),
                ..Default::default()
            },
            ..Default::default()
        };
        settings.save_to(&path).unwrap();
        let (restored, warning) = Settings::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(restored, settings);
        // Never touched means never claimed: the defaults stay the plugin's.
        assert!(Settings::default().gen1_rate.is_none());
        assert!(Settings::default().is_blank());
    }

    #[test]
    fn parameters_and_audio_share_a_flat_configuration() {
        let settings = Settings {
            backend: Some("core-audio".into()),
            preferences: PiFacePreferences {
                a_depth: Some(0.4),
                b_sync: Some(1),
                ..Default::default()
            },
            ..Default::default()
        };
        let path = frost::scratch::claim("nice-pi-face-flat-ron").join("settings.ron");
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
        // parameters are written even when `None`, which is how the loader tells
        // "never moved" from "moved to the default".
        for key in ["backend", "a_depth", "b_sync"] {
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
        assert_eq!(back.a_depth, Some(0.4));
        assert_eq!(back.b_sync, Some(1));
        assert!(Settings::path().ends_with("nice-pi-face-settings.ron"));
    }

    /// `is_blank` decides the settings panel's cold-start question, so a field
    /// the check forgets is a saved move that gets read back as a first run and
    /// overwritten on the next Apply. The list below is the same set of controls
    /// the check walks — a new preference belongs in both, and this is the test
    /// that says so.
    #[test]
    fn every_saved_move_reads_as_a_move() {
        type Moved = fn(&mut PiFacePreferences);
        let moved: [(&str, Moved); 14] = [
            ("gen1_rate", |p| p.gen1_rate = Some(2.0)),
            ("gen1_shape", |p| p.gen1_shape = Some(1)),
            ("gen1_rate_source", |p| p.gen1_rate_source = Some(1)),
            ("gen2_rate", |p| p.gen2_rate = Some(2.0)),
            ("gen2_shape", |p| p.gen2_shape = Some(1)),
            ("a_depth", |p| p.a_depth = Some(0.5)),
            ("a_feedback", |p| p.a_feedback = Some(0.5)),
            ("a_sweep", |p| p.a_sweep = Some(1)),
            ("b_depth", |p| p.b_depth = Some(0.5)),
            ("b_feedback", |p| p.b_feedback = Some(0.5)),
            ("b_sync", |p| p.b_sync = Some(1)),
            ("b_sweep", |p| p.b_sweep = Some(1)),
            ("b_input", |p| p.b_input = Some(1)),
            ("pedal", |p| p.pedal = Some(0.5)),
        ];
        for (name, set) in moved {
            let mut prefs = PiFacePreferences::default();
            set(&mut prefs);
            assert!(!prefs.is_blank(), "{name} is invisible to is_blank");
        }
        assert!(PiFacePreferences::default().is_blank());
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn disabling_audio_preserves_the_pedal() {
        let settings = Settings {
            backend: Some("dummy".into()),
            preferences: PiFacePreferences {
                b_depth: Some(0.6),
                ..Default::default()
            },
            ..Default::default()
        };
        let resolved = nice_common::devices::Snapshot::default()
            .resolve_settings(&settings)
            .unwrap();
        assert_eq!(resolved.b_depth, Some(0.6));
    }
}
