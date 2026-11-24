//! Shared project-owned adapters; dependency sources are never modified.
#![deny(unsafe_code)]
mod editor;
mod input;
pub mod lifecycle;
#[cfg(feature = "standalone")]
pub mod standalone;
mod window;
#[cfg(target_os = "linux")]
mod xdnd;
pub use baseview::WindowSize;
pub use editor::{Editor, EditorHandle, WindowConfig, WindowState};
use std::sync::{Arc, OnceLock};

/// The engine's OFL-licensed text font. The bundler ships its licence.
pub fn font() -> Arc<[u8]> {
    static FONT: OnceLock<Arc<[u8]>> = OnceLock::new();
    FONT.get_or_init(|| {
        Arc::from(&include_bytes!("../../assets/fonts/FiraCode-VariableFont_wght.ttf")[..])
    })
    .clone()
}
