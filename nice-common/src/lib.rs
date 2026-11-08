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

pub mod meter;
#[cfg(feature = "widgets")]
pub mod tooltip;
