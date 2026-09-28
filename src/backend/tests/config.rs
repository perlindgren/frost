//! Window config: the vsync default and the present-mode mapping.

use super::super::*;
use wgpu::PresentMode;

#[test]
fn present_mode_follows_the_vsync_flag() {
    assert_eq!(present_mode_for(true), PresentMode::AutoVsync);
    assert_eq!(present_mode_for(false), PresentMode::AutoNoVsync);
}

#[test]
fn config_defaults_to_vsync_on() {
    assert!(crate::Config::default().vsync);
}
