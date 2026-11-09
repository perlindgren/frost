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
}
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
}
