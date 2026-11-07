//! Shared standalone settings, persistence, and optional device discovery.
//! Plugin hosts own their devices; only standalone applications enable `devices`.
#![deny(unsafe_code)]

pub mod dropdown;
pub mod settings;
pub use settings::{OutputChannels, Preferences, Settings, WindowGeom};

#[cfg(feature = "devices")]
pub mod devices;

#[cfg(feature = "ui")]
pub mod ui;
