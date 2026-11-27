//! Plugin-specific setup for the shared standalone owner.
pub mod control;
#[cfg(test)]
mod process;
use crate::{EqPlugin, MIN_SIZE, settings::Settings};
use std::sync::{Arc, atomic::Ordering};

pub fn run(settings: Settings) -> Result<(), String> {
    let plugin = EqPlugin::default();
    if let Some(window) = settings.window {
        plugin
            .shared
            .logical_size
            .store(nice_plug::editor::dpi::LogicalSize::new(
                window.width.max(MIN_SIZE.width),
                window.height.max(MIN_SIZE.height),
            ));
        // Size rides in through `logical_size`, which baseview understands.
        // Position does not exist in baseview at all, so it queues here and
        // the window handler applies it once, or centres the window when the
        // file never recorded one.
        *plugin.shared.pending_geom.lock().unwrap() = window.x.zip(window.y);
    }
    let editor = crate::editor::create(Arc::clone(&plugin.params), Arc::clone(&plugin.shared));
    let buffers = Arc::clone(&plugin.shared);
    let drops = Arc::clone(&plugin.shared);
    // The owner's fixed buffer is sized in samples, not frames: this plugin
    // takes two lanes of `BLOCK` through every block, so the capacity is twice
    // the block. Get that ratio wrong and the standalone panics on its first
    // block, in a binary no test runs — the same construction is what
    // `process::tests` build their pump from.
    nice_plugin::standalone::run::<_, _, _, _, _, { crate::dsp::BLOCK }, { 2 * crate::dsp::BLOCK }>(
        plugin,
        editor,
        settings,
        &control::CONTROL,
        move |count| buffers.buffer_frames.store(count as u32, Ordering::Relaxed),
        move |count| drops.dropped.store(count, Ordering::Relaxed),
    )
}
