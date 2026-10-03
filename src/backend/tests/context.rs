//! The input snapshot: held keys, mouse position, held buttons, wheel delta, gamepads.

use crate::objects::*;
use crate::{Canvas, Context, KeyCode, MouseButton};
use std::collections::{HashMap, HashSet};

#[test]
fn context_reports_held_keys() {
    let mut canvas = Canvas::new((100, 100));
    let mut scene = Scene::default();
    let mut keys = HashSet::new();
    let typed = HashMap::new();
    let mouse_buttons = HashSet::new();
    {
        let ctx = Context {
            frame_diagnostic_draw_calls: 0,
            canvas: &mut canvas,
            scene: &mut scene,
            keys: &keys,
            typed: &typed,
            window: None,
            expected_fps: None,
            mouse: None,
            mouse_buttons: &mouse_buttons,
            mouse_wheel: 0.0,
            gilrs: None,
            frame_processing_ms: 0.0,
            frame_draw_calls: 0,
        };
        assert!(!ctx.key_down(KeyCode::KeyW));
    }
    keys.insert(KeyCode::KeyW);
    let ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        window: None,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 0.0,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert!(ctx.key_down(KeyCode::KeyW));
    assert!(!ctx.key_down(KeyCode::KeyA));
}

#[test]
fn context_reports_typed_character_of_held_key() {
    let mut canvas = Canvas::new((100, 100));
    let mut scene = Scene::default();
    let keys = HashSet::new();
    // The Swedish `+` key: the physical position US calls `Minus`, typed
    // as '+'. char_down follows the character, key_down the position.
    let mut typed = HashMap::new();
    typed.insert(KeyCode::Minus, '+');
    let mouse_buttons = HashSet::new();
    let ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        window: None,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 0.0,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert!(ctx.char_down('+'));
    assert!(!ctx.char_down('-'));
}

#[test]
fn context_reports_mouse_position() {
    let mut canvas = Canvas::new((100, 100));
    let mut scene = Scene::default();
    let keys = HashSet::new();
    let typed = HashMap::new();
    let mouse_buttons = HashSet::new();
    let ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        window: None,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 0.0,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert_eq!(ctx.mouse_position(), None);
    let ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        window: None,
        expected_fps: None,
        mouse: Some([12.0, -34.0]),
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 0.0,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert_eq!(ctx.mouse_position(), Some([12.0, -34.0]));
}

#[test]
fn context_reports_held_mouse_button() {
    let mut canvas = Canvas::new((100, 100));
    let mut scene = Scene::default();
    let keys = HashSet::new();
    let typed = HashMap::new();
    let mut mouse_buttons = HashSet::new();
    {
        let ctx = Context {
            frame_diagnostic_draw_calls: 0,
            canvas: &mut canvas,
            scene: &mut scene,
            keys: &keys,
            typed: &typed,
            window: None,
            expected_fps: None,
            mouse: None,
            mouse_buttons: &mouse_buttons,
            mouse_wheel: 0.0,
            gilrs: None,
            frame_processing_ms: 0.0,
            frame_draw_calls: 0,
        };
        assert!(!ctx.mouse_button_down(MouseButton::Left));
    }
    mouse_buttons.insert(MouseButton::Left);
    let ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        window: None,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 0.0,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert!(ctx.mouse_button_down(MouseButton::Left));
    assert!(!ctx.mouse_button_down(MouseButton::Right));
}

#[test]
fn context_reports_mouse_wheel_delta() {
    let mut canvas = Canvas::new((100, 100));
    let mut scene = Scene::default();
    let keys = HashSet::new();
    let typed = HashMap::new();
    let mouse_buttons = HashSet::new();
    let ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        window: None,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 3.0,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert_eq!(ctx.mouse_wheel(), 3.0);
}

#[test]
fn context_reports_no_gamepads_without_gilrs() {
    let mut canvas = Canvas::new((100, 100));
    let mut scene = Scene::default();
    let keys = HashSet::new();
    let typed = HashMap::new();
    let mouse_buttons = HashSet::new();
    let ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        window: None,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 0.0,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    // Without a gamepad controller (gilrs could not open the platform's
    // input devices, or none is connected) the list is empty, not an
    // error.
    assert_eq!(ctx.gamepads().count(), 0);
}
