use std::error::Error;

use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::NamedKey;
use winit::window::{Window, WindowId};

struct Frost {
    // Keeps the window alive for the lifetime of the event loop.
    #[allow(dead_code)]
    window: Option<Window>,
}

impl ApplicationHandler for Frost {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window = event_loop
            .create_window(Window::default_attributes().with_title("frost"))
            .expect("failed to create window");
        log::info!("window created");
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                log::info!("key event: {event:?}");
                if event.state == ElementState::Pressed && event.logical_key == NamedKey::Escape {
                    log::info!("escape pressed, exiting");
                    event_loop.exit();
                }
            }
            WindowEvent::CloseRequested => {
                log::info!("window close requested, exiting");
                event_loop.exit();
            }
            _ => {}
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    log::info!("frost starting up");

    let event_loop = EventLoop::new()?;
    let mut app = Frost { window: None };
    event_loop.run_app(&mut app)?;

    log::info!("event loop finished");
    Ok(())
}
