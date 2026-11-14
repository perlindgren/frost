//! Shared settings, persistence, device discovery, and the machinery a standalone
//! host needs around them: the command endpoint between panel and audio owner,
//! and the rate conversion between two devices. A plugin host owns its devices and
//! its placement; only a standalone application enables `devices` and `standalone`.
#![deny(unsafe_code)]

#[cfg(feature = "standalone")]
pub mod control;
pub mod dropdown;
#[cfg(feature = "standalone")]
pub mod resample;
pub mod settings;
pub use settings::{OutputChannels, Preferences, Settings, WindowGeom};

#[cfg(feature = "devices")]
pub mod devices;

#[cfg(feature = "ui")]
pub mod ui;

#[cfg(feature = "widgets")]
pub mod knob;
pub mod meter;
#[cfg(feature = "widgets")]
pub mod tooltip;

#[cfg(feature = "audio-io")]
pub mod audio_io;

/// The sample formats an ASIO driver's buffers hold, and the arithmetic between
/// them and the `f32` the rings speak. Platform-neutral on purpose: the only
/// build that can hear it is the Windows transport, and the only gate that can
/// run its tests is Linux.
pub mod pcm;

/// The Windows transport: one ASIO driver, loaded through `azo`, both
/// directions in one `createBuffers`, no cpal and no Steinberg SDK in sight.
#[cfg(all(windows, feature = "asio-host"))]
pub mod asio_io;
