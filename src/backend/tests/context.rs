//! The input snapshot: held keys, mouse position, held buttons, wheel delta, gamepads.

use crate::objects::*;
use crate::{Canvas, Context, KeyCode, MouseButton};
use std::collections::{HashMap, HashSet};

#[test]
fn context_reports_held_keys() {
    let mut canvas = Canvas::new((100, 100), 1.0);
    let mut scene = Scene::default();
    let mut keys = HashSet::new();
    let typed = HashMap::new();
    let mouse_buttons = HashSet::new();
    {
        let mut blit_filter = SpriteFilter::Linear;
        let mut vsync = true;
        let ctx = Context {
            frame_diagnostic_draw_calls: 0,
            canvas: &mut canvas,
            scene: &mut scene,
            keys: &keys,
            typed: &typed,
            blit_filter: &mut blit_filter,
            vsync: &mut vsync,
            render_size: &mut None,
            window: None,
            expected_fps: None,
            mouse: None,
            mouse_buttons: &mouse_buttons,
            mouse_wheel: 0.0,
            exit: &mut false,
            keys_frozen: &mut false,
            close_requested: false,
            gilrs: None,
            frame_processing_ms: 0.0,
            frame_draw_calls: 0,
        };
        assert!(!ctx.key_down(KeyCode::KeyW));
    }
    keys.insert(KeyCode::KeyW);
    let mut blit_filter = SpriteFilter::Linear;
    let mut vsync = true;
    let ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        blit_filter: &mut blit_filter,
        vsync: &mut vsync,
        render_size: &mut None,
        window: None,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 0.0,
        exit: &mut false,
        keys_frozen: &mut false,
        close_requested: false,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert!(ctx.key_down(KeyCode::KeyW));
    assert!(!ctx.key_down(KeyCode::KeyA));
}

#[test]
fn context_reports_typed_character_of_held_key() {
    let mut canvas = Canvas::new((100, 100), 1.0);
    let mut scene = Scene::default();
    let keys = HashSet::new();
    // The Swedish `+` key: the physical position US calls `Minus`, typed
    // as '+'. char_down follows the character, key_down the position.
    let mut typed = HashMap::new();
    typed.insert(KeyCode::Minus, '+');
    let mouse_buttons = HashSet::new();
    let mut blit_filter = SpriteFilter::Linear;
    let mut vsync = true;
    let ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        blit_filter: &mut blit_filter,
        vsync: &mut vsync,
        render_size: &mut None,
        window: None,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 0.0,
        exit: &mut false,
        keys_frozen: &mut false,
        close_requested: false,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert!(ctx.char_down('+'));
    assert!(!ctx.char_down('-'));
}

#[test]
fn context_reports_mouse_position() {
    let mut canvas = Canvas::new((100, 100), 1.0);
    let mut scene = Scene::default();
    let keys = HashSet::new();
    let typed = HashMap::new();
    let mouse_buttons = HashSet::new();
    let mut blit_filter = SpriteFilter::Linear;
    let mut vsync = true;
    let ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        blit_filter: &mut blit_filter,
        vsync: &mut vsync,
        render_size: &mut None,
        window: None,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 0.0,
        exit: &mut false,
        keys_frozen: &mut false,
        close_requested: false,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert_eq!(ctx.mouse_position(), None);
    let mut blit_filter = SpriteFilter::Linear;
    let mut vsync = true;
    let ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        blit_filter: &mut blit_filter,
        vsync: &mut vsync,
        render_size: &mut None,
        window: None,
        expected_fps: None,
        mouse: Some([12.0, -34.0]),
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 0.0,
        exit: &mut false,
        keys_frozen: &mut false,
        close_requested: false,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert_eq!(ctx.mouse_position(), Some([12.0, -34.0]));
}

#[test]
fn context_reports_held_mouse_button() {
    let mut canvas = Canvas::new((100, 100), 1.0);
    let mut scene = Scene::default();
    let keys = HashSet::new();
    let typed = HashMap::new();
    let mut mouse_buttons = HashSet::new();
    {
        let mut blit_filter = SpriteFilter::Linear;
        let mut vsync = true;
        let ctx = Context {
            frame_diagnostic_draw_calls: 0,
            canvas: &mut canvas,
            scene: &mut scene,
            keys: &keys,
            typed: &typed,
            blit_filter: &mut blit_filter,
            vsync: &mut vsync,
            render_size: &mut None,
            window: None,
            expected_fps: None,
            mouse: None,
            mouse_buttons: &mouse_buttons,
            mouse_wheel: 0.0,
            exit: &mut false,
            keys_frozen: &mut false,
            close_requested: false,
            gilrs: None,
            frame_processing_ms: 0.0,
            frame_draw_calls: 0,
        };
        assert!(!ctx.mouse_button_down(MouseButton::Left));
    }
    mouse_buttons.insert(MouseButton::Left);
    let mut blit_filter = SpriteFilter::Linear;
    let mut vsync = true;
    let ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        blit_filter: &mut blit_filter,
        vsync: &mut vsync,
        render_size: &mut None,
        window: None,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 0.0,
        exit: &mut false,
        keys_frozen: &mut false,
        close_requested: false,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert!(ctx.mouse_button_down(MouseButton::Left));
    assert!(!ctx.mouse_button_down(MouseButton::Right));
}

#[test]
fn context_reports_mouse_wheel_delta() {
    let mut canvas = Canvas::new((100, 100), 1.0);
    let mut scene = Scene::default();
    let keys = HashSet::new();
    let typed = HashMap::new();
    let mouse_buttons = HashSet::new();
    let mut blit_filter = SpriteFilter::Linear;
    let mut vsync = true;
    let ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        blit_filter: &mut blit_filter,
        vsync: &mut vsync,
        render_size: &mut None,
        window: None,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 3.0,
        exit: &mut false,
        keys_frozen: &mut false,
        close_requested: false,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert_eq!(ctx.mouse_wheel(), 3.0);
}

#[test]
fn context_reports_no_gamepads_without_gilrs() {
    let mut canvas = Canvas::new((100, 100), 1.0);
    let mut scene = Scene::default();
    let keys = HashSet::new();
    let typed = HashMap::new();
    let mouse_buttons = HashSet::new();
    let mut blit_filter = SpriteFilter::Linear;
    let mut vsync = true;
    let ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        blit_filter: &mut blit_filter,
        vsync: &mut vsync,
        render_size: &mut None,
        window: None,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 0.0,
        exit: &mut false,
        keys_frozen: &mut false,
        close_requested: false,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    // Without a gamepad controller (gilrs could not open the platform's
    // input devices, or none is connected) the list is empty, not an
    // error.
    assert_eq!(ctx.gamepads().count(), 0);
}

#[test]
fn render_size_flips_mid_run() {
    // The quality lever a settings menu reaches for: the fixed buffer
    // the frame renders into is the engine's live state, not a startup
    // promise — the app can fix it, lift it, and refix it, frame by
    // frame, and the Context reads back exactly what it last set.
    let mut canvas = Canvas::new((100, 100), 1.0);
    let mut scene = Scene::default();
    let keys = HashSet::new();
    let typed = HashMap::new();
    let mouse_buttons = HashSet::new();
    let mut blit_filter = SpriteFilter::Linear;
    let mut vsync = true;
    let mut render_size: Option<[u32; 2]> = None;
    let mut ctx = Context {
        frame_diagnostic_draw_calls: 0,
        canvas: &mut canvas,
        scene: &mut scene,
        keys: &keys,
        typed: &typed,
        blit_filter: &mut blit_filter,
        vsync: &mut vsync,
        render_size: &mut render_size,
        window: None,
        expected_fps: None,
        mouse: None,
        mouse_buttons: &mouse_buttons,
        mouse_wheel: 0.0,
        exit: &mut false,
        keys_frozen: &mut false,
        close_requested: false,
        gilrs: None,
        frame_processing_ms: 0.0,
        frame_draw_calls: 0,
    };
    assert_eq!(ctx.render_size(), None, "open starts unpinned");
    ctx.set_render_size(Some([1920, 1080]));
    assert_eq!(ctx.render_size(), Some([1920, 1080]));
    ctx.set_render_size(None);
    assert_eq!(ctx.render_size(), None, "the fix lifts again");
    // The engine state the frame will render against really moved.
    assert_eq!(render_size, None);
}
