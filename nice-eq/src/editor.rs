//! Plugin-specific view construction; native editor behavior lives in nice-plugin.
use crate::{EqParams, EqSharedState, MIN_SIZE, RESIZE_HINT};
use std::sync::{Arc, atomic::Ordering};

pub type EqEditor = nice_plugin::Editor<crate::ui::EqUi>;

pub(crate) fn create(params: Arc<EqParams>, shared: Arc<EqSharedState>) -> EqEditor {
    nice_plugin::Editor::new(
        shared.clone(),
        nice_plugin::WindowConfig {
            title: "Nice Eq",
            min_size: MIN_SIZE,
            resize_hint: RESIZE_HINT,
        },
        move |context| {
            let mut ui = crate::ui::EqUi::new(Arc::clone(&shared), nice_plugin::font());
            ui.set_params(Arc::clone(&params));
            if let Some(context) = context {
                ui.set_gui_context(context);
            }
            (crate::ui::static_scene(), ui)
        },
    )
}

impl nice_plugin::WindowState for EqSharedState {
    fn logical_size(&self) -> nice_plug::editor::dpi::LogicalSize<f32> {
        EqSharedState::logical_size(self)
    }
    fn scale_factor(&self) -> f64 {
        EqSharedState::scale_factor(self)
    }
    fn resized(&self, size: nice_plugin::WindowSize) {
        self.logical_size.store(size.logical.cast());
        self.scale_factor.store(size.scale_factor);
    }
    fn set_open(&self, value: bool) {
        self.open.store(value, Ordering::Release);
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
