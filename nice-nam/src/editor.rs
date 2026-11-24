//! Plugin-specific view construction; native editor behavior lives in nice-plugin.
use crate::{MIN_SIZE, NamParams, NamSharedState, RESIZE_HINT};
use std::sync::{Arc, atomic::Ordering};

pub type NamEditor = nice_plugin::Editor<crate::ui::NamUi>;
#[cfg(feature = "standalone")]
pub type NamEditorHandle = nice_plugin::EditorHandle;

pub(crate) fn create(params: Arc<NamParams>, shared: Arc<NamSharedState>) -> NamEditor {
    nice_plugin::Editor::new(
        shared.clone(),
        nice_plugin::WindowConfig {
            title: "Nice Nam",
            min_size: MIN_SIZE,
            resize_hint: RESIZE_HINT,
        },
        move |context| {
            let mut ui = crate::ui::NamUi::new(Arc::clone(&shared), nice_plugin::font());
            if let Some(context) = context {
                ui.set_host(Arc::clone(&params), context);
            }
            (crate::ui::static_scene(), ui)
        },
    )
}

impl nice_plugin::WindowState for NamSharedState {
    fn logical_size(&self) -> nice_plug::editor::dpi::LogicalSize<f32> {
        NamSharedState::logical_size(self)
    }
    fn scale_factor(&self) -> f64 {
        NamSharedState::scale_factor(self)
    }
    fn resized(&self, size: nice_plugin::WindowSize) {
        self.logical_size.store(size.logical.cast());
        self.scale_factor.store(size.scale_factor);
    }
    fn set_open(&self, value: bool) {
        self.open.store(value, Ordering::Release);
    }
    fn accepts_files(&self) -> bool {
        true
    }
    fn files_dropped(&self, paths: Vec<std::path::PathBuf>) {
        self.dropped.lock().unwrap().extend(paths);
    }
    fn standalone(&self) -> bool {
        cfg!(feature = "standalone") && crate::STANDALONE_MODE.load(Ordering::Relaxed)
    }
    fn allow_placement(&self) -> bool {
        cfg!(all(windows, feature = "standalone", feature = "unsafe"))
    }
    #[cfg(feature = "standalone")]
    fn position(&self) -> Option<(i32, i32)> {
        *self.window_pos.lock().unwrap()
    }
    #[cfg(feature = "standalone")]
    fn pending_position(&self) -> Option<(i32, i32)> {
        *self.pending_geom.lock().unwrap()
    }
    #[cfg(feature = "standalone")]
    fn set_position(&self, value: (i32, i32)) {
        *self.window_pos.lock().unwrap() = Some(value);
    }
    #[cfg(feature = "standalone")]
    fn save_geometry(&self, geometry: nice_common::WindowGeom) -> Result<(), String> {
        crate::settings::Settings::save_window(geometry)
    }
}
