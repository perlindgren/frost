//! The render backend: the winit window, the wgpu device, and the frame
//! pipeline that turns a frame's draw list into scissored draw calls.
//!
//! Everything here is internal to the crate; the public API (`Canvas`,
//! `Context`, `Process`, [`run`](crate::run)) lives in the crate root.
//! Each frame the backend runs the user's [`Process`] through a
//! [`Context`], updates the scene tree, draws it into a fresh draw list,
//! expands text into glyph quads, sorts the frame's [`Draw`]ings by z, and
//! renders them one scissored draw call at a time to the window's surface.


mod app;
pub(crate) use app::*;

mod frame;
pub(crate) use frame::*;

#[cfg(target_arch = "wasm32")]
mod wasm;
#[cfg(target_arch = "wasm32")]
pub(crate) use wasm::*;

#[cfg(test)]
mod tests;
