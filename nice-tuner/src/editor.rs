//! Plugin-specific view construction; native editor behavior lives in nice-plugin.
use crate::{MIN_SIZE, RESIZE_HINT, TunerParams, TunerSharedState};
use std::sync::{Arc, atomic::Ordering};

pub type TunerEditor = nice_plugin::Editor<crate::tuner_ui::TunerUi>;
#[cfg(feature = "standalone")]
pub type TunerEditorHandle = nice_plugin::EditorHandle;

pub(crate) fn create(params: Arc<TunerParams>, shared: Arc<TunerSharedState>) -> TunerEditor {
    nice_plugin::Editor::new(
        shared.clone(),
        nice_plugin::WindowConfig {
            title: "Nice Tuner",
            min_size: MIN_SIZE,
            resize_hint: RESIZE_HINT,
        },
        move |context| {
            let mut ui = crate::tuner_ui::TunerUi::new(Arc::clone(&shared), nice_plugin::font());
            if let Some(context) = context {
                ui.set_host_controls(crate::host_controls::HostControls::new(
                    Arc::clone(&params),
                    context,
                ));
            }
            (crate::tuner_ui::static_scene(), ui)
        },
    )
}

impl nice_plugin::WindowState for TunerSharedState {
    fn logical_size(&self) -> nice_plug::editor::dpi::LogicalSize<f32> {
        TunerSharedState::logical_size(self)
    }
    fn scale_factor(&self) -> f64 {
        TunerSharedState::scale_factor(self)
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
    #[cfg(feature = "standalone")]
    fn prepare(&self, parented: bool) {
        if !self.standalone() || parented {
            return;
        }
        if let Some(g) = crate::settings::Settings::load().0.window {
            self.logical_size
                .store(nice_plug::editor::dpi::LogicalSize::new(
                    g.width.max(MIN_SIZE.width),
                    g.height.max(MIN_SIZE.height),
                ));
            *self.pending_geom.lock().unwrap() = g.x.zip(g.y);
        }
    }
}
