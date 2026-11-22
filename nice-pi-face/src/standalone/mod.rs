//! Plugin-specific setup for the shared stereo standalone owner.
pub mod control;
#[cfg(test)]
mod process;
use crate::{MIN_SIZE, PiFacePlugin, settings::Settings};
use std::sync::{Arc, atomic::Ordering};

pub fn run(settings: Settings) -> Result<(), String> {
    let plugin = PiFacePlugin::default();
    if let Some(window) = settings.window {
        plugin
            .shared
            .logical_size
            .store(nice_plug::editor::dpi::LogicalSize::new(
                window.width.max(MIN_SIZE.width),
                window.height.max(MIN_SIZE.height),
            ));
        // Size rides in through `logical_size`, which baseview understands.
        // Position does not exist in baseview at all, so it queues here and the
        // window handler applies it once, or centres the window when the file
        // never recorded one.
        *plugin.shared.pending_geom.lock().unwrap() = window.x.zip(window.y);
    }
    let editor = crate::editor::create(Arc::clone(&plugin.params), Arc::clone(&plugin.shared));
    let buffers = Arc::clone(&plugin.shared);
    let drops = Arc::clone(&plugin.shared);
    nice_plugin::standalone::run::<_, _, _, _, _, { crate::dsp::BLOCK }, { 2 * crate::dsp::BLOCK }>(
        plugin,
        editor,
        settings,
        &control::CONTROL,
        move |count| buffers.buffer_frames.store(count as u32, Ordering::Relaxed),
        move |count| drops.dropped.store(count, Ordering::Relaxed),
    )
}
