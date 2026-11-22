//! Input translation is separate from GPU resources so the complete event
//! contract can be exercised without opening a window.
use baseview::{Event as B, MouseEvent as M, WindowEvent as W};
use frost::{KeyCode, MouseButton};
use keyboard_types::KeyState;

trait Sink {
    fn key(&mut self, key: KeyCode, down: bool);
    fn text(&mut self, ch: char);
    fn modifiers(&mut self, shift: bool, ctrl: bool, alt: bool, meta: bool);
    fn cursor(&mut self, x: f64, y: f64);
    fn button(&mut self, button: MouseButton, down: bool);
    fn scroll(&mut self, x: f64, y: f64);
    fn leave(&mut self);
    fn focus(&mut self, focused: bool);
}
impl<U: frost::Process> Sink for frost::embedded::Embedded<U> {
    fn key(&mut self, key: KeyCode, down: bool) {
        self.set_key(key, down);
    }
    fn text(&mut self, ch: char) {
        self.add_text(ch);
    }
    fn modifiers(&mut self, s: bool, c: bool, a: bool, m: bool) {
        self.set_modifiers(s, c, a, m);
    }
    fn cursor(&mut self, x: f64, y: f64) {
        self.set_cursor(x, y);
    }
    fn button(&mut self, b: MouseButton, down: bool) {
        self.set_mouse_button(b, down);
    }
    fn scroll(&mut self, x: f64, y: f64) {
        self.scroll(x, y);
    }
    fn leave(&mut self) {
        self.cursor_left();
    }
    fn focus(&mut self, value: bool) {
        self.set_focused(value);
    }
}
pub(crate) fn forward<U: frost::Process>(
    sink: &mut frost::embedded::Embedded<U>,
    event: B,
    scale: f64,
) {
    dispatch(sink, event, scale);
}
fn dispatch(sink: &mut impl Sink, event: B, scale: f64) {
    match event {
        B::Keyboard(ke) => {
            let down = ke.state == KeyState::Down;
            if let Some(code) = map_key(&ke.key) {
                sink.key(code, down);
            }
            if down && let keyboard_types::Key::Character(s) = &ke.key {
                for ch in s.chars() {
                    sink.text(ch);
                }
            }
            use keyboard_types::Modifiers as K;
            sink.modifiers(
                ke.modifiers.contains(K::SHIFT),
                ke.modifiers.contains(K::CONTROL),
                ke.modifiers.contains(K::ALT),
                ke.modifiers.contains(K::META),
            );
        }
        B::Mouse(mouse) => match mouse {
            M::CursorMoved { position, .. } => sink.cursor(position.x / scale, position.y / scale),
            M::ButtonPressed { button, .. } => {
                if let Some(b) = map_mouse(button) {
                    sink.button(b, true);
                }
            }
            M::ButtonReleased { button, .. } => {
                if let Some(b) = map_mouse(button) {
                    sink.button(b, false);
                }
            }
            M::WheelScrolled { delta, .. } => {
                let y = match delta {
                    baseview::ScrollDelta::Lines { y, .. } => y as f64,
                    baseview::ScrollDelta::Pixels { y, .. } => y as f64 / 120.0,
                };
                sink.scroll(0.0, y);
            }
            M::CursorLeft => sink.leave(),
            _ => {}
        },
        B::Window(W::Focused) => sink.focus(true),
        B::Window(W::Unfocused) => sink.focus(false),
        _ => {}
    }
}
pub fn map_key(key: &keyboard_types::Key) -> Option<frost::KeyCode> {
    use keyboard_types::{Key, NamedKey};
    match key {
        Key::Character(s) => match s.as_str() {
            "a" | "A" => Some(frost::KeyCode::KeyA),
            "b" | "B" => Some(frost::KeyCode::KeyB),
            "c" | "C" => Some(frost::KeyCode::KeyC),
            "d" | "D" => Some(frost::KeyCode::KeyD),
            "e" | "E" => Some(frost::KeyCode::KeyE),
            "f" | "F" => Some(frost::KeyCode::KeyF),
            "g" | "G" => Some(frost::KeyCode::KeyG),
            "h" | "H" => Some(frost::KeyCode::KeyH),
            "i" | "I" => Some(frost::KeyCode::KeyI),
            "j" | "J" => Some(frost::KeyCode::KeyJ),
            "k" | "K" => Some(frost::KeyCode::KeyK),
            "l" | "L" => Some(frost::KeyCode::KeyL),
            "m" | "M" => Some(frost::KeyCode::KeyM),
            "n" | "N" => Some(frost::KeyCode::KeyN),
            "o" | "O" => Some(frost::KeyCode::KeyO),
            "p" | "P" => Some(frost::KeyCode::KeyP),
            "q" | "Q" => Some(frost::KeyCode::KeyQ),
            "r" | "R" => Some(frost::KeyCode::KeyR),
            "s" | "S" => Some(frost::KeyCode::KeyS),
            "t" | "T" => Some(frost::KeyCode::KeyT),
            "u" | "U" => Some(frost::KeyCode::KeyU),
            "v" | "V" => Some(frost::KeyCode::KeyV),
            "w" | "W" => Some(frost::KeyCode::KeyW),
            "x" | "X" => Some(frost::KeyCode::KeyX),
            "y" | "Y" => Some(frost::KeyCode::KeyY),
            "z" | "Z" => Some(frost::KeyCode::KeyZ),
            "0" => Some(frost::KeyCode::Digit0),
            "1" => Some(frost::KeyCode::Digit1),
            "2" => Some(frost::KeyCode::Digit2),
            "3" => Some(frost::KeyCode::Digit3),
            "4" => Some(frost::KeyCode::Digit4),
            "5" => Some(frost::KeyCode::Digit5),
            "6" => Some(frost::KeyCode::Digit6),
            "7" => Some(frost::KeyCode::Digit7),
            "8" => Some(frost::KeyCode::Digit8),
            "9" => Some(frost::KeyCode::Digit9),
            " " => Some(frost::KeyCode::Space),
            _ => None,
        },
        Key::Named(named) => Some(match named {
            NamedKey::Escape => frost::KeyCode::Escape,
            NamedKey::Enter => frost::KeyCode::Enter,
            NamedKey::Tab => frost::KeyCode::Tab,
            NamedKey::Backspace => frost::KeyCode::Backspace,
            NamedKey::Delete => frost::KeyCode::Delete,
            NamedKey::ArrowLeft => frost::KeyCode::ArrowLeft,
            NamedKey::ArrowRight => frost::KeyCode::ArrowRight,
            NamedKey::ArrowUp => frost::KeyCode::ArrowUp,
            NamedKey::ArrowDown => frost::KeyCode::ArrowDown,
            _ => return None,
        }),
    }
}

pub fn map_mouse(button: baseview::MouseButton) -> Option<frost::MouseButton> {
    Some(match button {
        baseview::MouseButton::Left => frost::MouseButton::Left,
        baseview::MouseButton::Right => frost::MouseButton::Right,
        baseview::MouseButton::Middle => frost::MouseButton::Middle,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyboard_types::{Key, KeyboardEvent, Modifiers, NamedKey};
    #[derive(Default)]
    struct Input {
        keys: Vec<(KeyCode, bool)>,
        text: String,
        mods: [bool; 4],
        cursor: [f64; 2],
        buttons: Vec<(MouseButton, bool)>,
        scroll: [f64; 2],
        focused: bool,
        left: bool,
    }
    impl Sink for Input {
        fn key(&mut self, k: KeyCode, d: bool) {
            self.keys.push((k, d));
        }
        fn text(&mut self, c: char) {
            self.text.push(c);
        }
        fn modifiers(&mut self, s: bool, c: bool, a: bool, m: bool) {
            self.mods = [s, c, a, m];
        }
        fn cursor(&mut self, x: f64, y: f64) {
            self.cursor = [x, y];
        }
        fn button(&mut self, b: MouseButton, d: bool) {
            self.buttons.push((b, d));
        }
        fn scroll(&mut self, x: f64, y: f64) {
            self.scroll = [x, y];
        }
        fn leave(&mut self) {
            self.left = true;
        }
        fn focus(&mut self, f: bool) {
            self.focused = f;
        }
    }
    #[test]
    fn escape_enter_navigation_and_delete_forward_both_edges() {
        for (named, expected) in [
            (NamedKey::Escape, KeyCode::Escape),
            (NamedKey::Enter, KeyCode::Enter),
            (NamedKey::Tab, KeyCode::Tab),
            (NamedKey::Backspace, KeyCode::Backspace),
            (NamedKey::Delete, KeyCode::Delete),
            (NamedKey::ArrowLeft, KeyCode::ArrowLeft),
            (NamedKey::ArrowRight, KeyCode::ArrowRight),
            (NamedKey::ArrowUp, KeyCode::ArrowUp),
            (NamedKey::ArrowDown, KeyCode::ArrowDown),
        ] {
            let mut sink = Input::default();
            for state in [KeyState::Down, KeyState::Up] {
                dispatch(
                    &mut sink,
                    B::Keyboard(KeyboardEvent {
                        key: Key::Named(named),
                        state,
                        ..Default::default()
                    }),
                    1.0,
                );
            }
            assert_eq!(sink.keys, vec![(expected, true), (expected, false)]);
            assert!(sink.text.is_empty());
        }
    }
    #[test]
    fn typing_records_keys_and_characters_separately_and_only_types_on_press() {
        let mut sink = Input::default();
        for state in [KeyState::Down, KeyState::Up] {
            dispatch(
                &mut sink,
                B::Keyboard(KeyboardEvent {
                    key: Key::Character("A".into()),
                    state,
                    modifiers: Modifiers::SHIFT | Modifiers::CONTROL,
                    ..Default::default()
                }),
                1.0,
            );
        }
        assert_eq!(
            sink.keys,
            vec![(KeyCode::KeyA, true), (KeyCode::KeyA, false)]
        );
        assert_eq!(sink.text, "A");
        assert_eq!(sink.mods, [true, true, false, false]);
    }
    #[test]
    fn mouse_scaling_scroll_and_focus_use_the_same_path_for_every_editor() {
        let mut sink = Input::default();
        dispatch(
            &mut sink,
            B::Mouse(M::CursorMoved {
                position: baseview::dpi::PhysicalPosition::new(120.0, 80.0),
                modifiers: Modifiers::empty(),
            }),
            2.0,
        );
        assert_eq!(sink.cursor, [60.0, 40.0]);
        dispatch(
            &mut sink,
            B::Mouse(M::WheelScrolled {
                delta: baseview::ScrollDelta::Pixels { x: 0.0, y: 240.0 },
                modifiers: Modifiers::empty(),
            }),
            2.0,
        );
        assert_eq!(sink.scroll, [0.0, 2.0]);
        dispatch(&mut sink, B::Window(W::Focused), 1.0);
        assert!(sink.focused);
        dispatch(&mut sink, B::Window(W::Unfocused), 1.0);
        assert!(!sink.focused);
        dispatch(&mut sink, B::Mouse(M::CursorLeft), 1.0);
        assert!(sink.left);
    }
}
