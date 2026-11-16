//! Standalone frost rack GUI and safe main-thread audio controller.
#![deny(unsafe_code)]

pub use nice_rack_engine as routing;
use serde::{Deserialize, Serialize};

/// Local device configuration, rack instances, native states and editor windows.
pub type Settings = nice_common::Settings<RackPreferences>;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RackPreferences {
    pub plugins: Vec<SavedPlugin>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedPlugin {
    pub id: u64,
    pub stage: u64,
    pub path: std::path::PathBuf,
    pub class: String,
    pub name: String,
    pub state: Option<Vec<u8>>,
    pub editor: Option<EditorGeometry>,
    /// Gain trim, in dB, the rack applies on this instance's input wire and on
    /// its output wire. Gain staging is part of how a rack was left, so it is
    /// saved with the rack and resumed on start. Both fields are
    /// `#[serde(default)]` on their own — the attribute on
    /// [`RackPreferences`] does not reach in here — so a file written before
    /// trims existed still loads, and a trim the file does not name reads
    /// unity, which is the gain that rack was running.
    ///
    /// What the meters *read* is deliberately absent. A meter measures the
    /// signal in front of it and holds a clip until somebody clicks it away;
    /// a clip is a report about audio that has already gone past, so carrying
    /// one into a resumed rack would paint the master and half the wires red
    /// over silence, and would destroy the one thing the latched red is for —
    /// the upstream-most red names the first stage that clipped. So the
    /// record holds the trims and not the meters, a resumed rack's meters
    /// start green (`PeakMeter::latching` is born unlatched, and nothing here
    /// asks it to remember more), and every red is earned again from the
    /// first block.
    #[serde(default)]
    pub gain_in: f32,
    #[serde(default)]
    pub gain_out: f32,
}
/// Half the range a wire's gain trim spans, in dB: the track is drawn across
/// ±24 dB, the drag clamps to it, a typed number is answered at it, the
/// restored value is clamped back to it, and the engine's own ceiling of ×16
/// linear sits just outside, so a trim can never be shown at a gain the audio
/// path would not apply.
pub const TRIM_DB: f32 = 24.0;
/// Physical screen position and logical editor content size. Optional position
/// accommodates platforms which do not expose top-level placement (Wayland).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditorGeometry {
    pub position: Option<[i32; 2]>,
    pub size: [u32; 2],
}
impl nice_common::Preferences for RackPreferences {
    const FILE_NAME: &'static str = "nice-rack-settings.ron";
    const APP_NAME: &'static str = "nice-rack";
    fn is_blank(&self) -> bool {
        self.plugins.is_empty()
    }
}

mod editors;
mod gui;
pub mod runtime;
pub use gui::run;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub mod layout;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn instance_states_and_editor_geometry_round_trip_independently() {
        let path = frost::scratch::claim("rack-settings").join("rack.ron");
        let mut settings = Settings::default();
        for (id, state, x) in [(1, vec![0, 255, 1], 20), (3, vec![42, 8], 840)] {
            settings.preferences.plugins.push(SavedPlugin {
                id,
                stage: 1,
                path: "/plugins/tuner.clap".into(),
                class: "same-class".into(),
                name: "Tuner".into(),
                state: Some(state),
                editor: Some(EditorGeometry {
                    position: Some([x, 40]),
                    size: [640, 400],
                }),
                gain_in: -6.0,
                gain_out: 2.5,
            });
        }
        settings.save_to(&path).unwrap();
        let (loaded, warning) = Settings::load_from(&path);
        assert_eq!(warning, None);
        assert_eq!(loaded, settings);
        Settings::save_window_to(
            &path,
            nice_common::WindowGeom {
                x: None,
                y: None,
                width: 1000.0,
                height: 600.0,
            },
        )
        .unwrap();
        assert_eq!(
            Settings::load_from(&path).0.preferences,
            settings.preferences
        );
    }

    #[test]
    fn a_settings_file_from_before_trims_still_loads_at_unity() {
        // The point of this file is what it does not say. A rack saved before
        // wires had trims carries no gain for them, and the whole reason the
        // two fields default per field instead of the record requiring them is
        // that such a rack still has to open — at unity, the gain it was
        // actually running, not at whatever `Default` happened to invent.
        let path = frost::scratch::claim("rack-settings-legacy").join("rack.ron");
        std::fs::write(
            &path,
            r#"(
    plugins: [
        (
            id: 2,
            stage: 1,
            path: "/plugins/tuner.clap",
            class: "same-class",
            name: "Tuner",
            state: Some([0, 255, 1]),
            editor: Some((
                position: Some([20, 40]),
                size: (640, 400),
            )),
        ),
    ],
)"#,
        )
        .unwrap();
        let (loaded, warning) = Settings::load_from(&path);
        assert_eq!(warning, None, "a legacy file is not a damaged one");
        assert_eq!(loaded.preferences.plugins[0].gain_in, 0.0);
        assert_eq!(loaded.preferences.plugins[0].gain_out, 0.0);
        // And what it loads is what gets written back: the trim joins the
        // record on the next save rather than replacing the file's own words.
        assert_eq!(loaded.preferences.plugins[0].name, "Tuner");
        assert_eq!(loaded.preferences.plugins[0].state, Some(vec![0, 255, 1]));
    }
}
