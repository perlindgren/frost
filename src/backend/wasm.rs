use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll, Waker};

use wgpu::{Adapter, Device, DeviceDescriptor, Instance, Queue, RequestAdapterOptions};
use winit::application::ApplicationHandler;
use winit::event::StartCause;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::NamedKey;
use winit::window::{Window, WindowId};

use super::{create_window, Frost};
use crate::objects::Scene;
use crate::Process;

// ==================== web startup (wasm32 only) ====================
//
// In the browser, `request_adapter` and `request_device` are JS promises:
// the first poll of each returns `Pending`, and the browser only runs the
// resolution microtask once the main thread is free — so the main thread
// cannot block on them. `run` on wasm therefore defers the GPU setup: the
// canvas is created immediately, and the event loop (which never blocks on
// the web) polls the setup future on every iteration. When it completes,
// the real `Frost` app takes over and drives the frame loop.

/// The scene and process, held until the GPU setup completes on the web.
#[cfg(target_arch = "wasm32")]
struct Core<P: Process> {
    scene: Scene,
    process: P,
    vsync: bool,
    window_size: Option<[u32; 2]>,
}

/// The in-flight async GPU setup on the web.
#[cfg(target_arch = "wasm32")]
enum GpuInit {
    /// Not started yet (before the first `resumed`).
    Idle,
    /// The combined `request_adapter` + `request_device` future.
    Init(Pin<Box<dyn Future<Output = Result<(Adapter, Device, Queue), String>>>>),
    /// The setup failed and the page shows the error.
    Failed,
}

/// The web event-loop handler: creates the window immediately, drives the
/// async GPU setup, and hands everything to `Frost` once it completes.
#[cfg(target_arch = "wasm32")]
pub(crate) struct WebFrost<P: Process> {
    instance: Option<Instance>,
    /// The scene and process, held until the GPU setup completes.
    core: Option<Core<P>>,
    /// The canvas, created before the GPU is ready.
    window: Option<Arc<Window>>,
    /// The in-flight async GPU setup.
    gpu: GpuInit,
    /// The real app, once the adapter and device are ready.
    frost: Option<Frost<P>>,
}

#[cfg(target_arch = "wasm32")]
impl<P: Process> ApplicationHandler for WebFrost<P> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.frost.is_some() || matches!(self.gpu, GpuInit::Failed) {
            return;
        }
        // Create the canvas immediately so the page shows *something* while
        // the GPU setup is in flight.
        if self.window.is_none() {
            let window_size = self.core.as_ref().map(|core| core.window_size);
            self.window = Some(create_window(event_loop, window_size));
        }
        if matches!(self.gpu, GpuInit::Idle) {
            // The async block owns its own `Instance` clone (a cheap
            // refcounted bump), so the future borrows nothing from `self`.
            let instance = self
                .instance
                .clone()
                .expect("the instance is created in `run`");
            self.gpu = GpuInit::Init(Box::pin(async move {
                let adapter = instance
                    .request_adapter(&RequestAdapterOptions::default())
                    .await
                    .map_err(|error| {
                        format!(
                            "no suitable GPU adapter found: {error} \
                             (is WebGPU enabled in this browser?)"
                        )
                    })?;
                log::info!("using adapter: {:?}", adapter.get_info().name);
                let (device, queue) = adapter
                    .request_device(&DeviceDescriptor::default())
                    .await
                    .map_err(|error| format!("failed to create GPU device: {error}"))?;
                Ok((adapter, device, queue))
            }));
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: StartCause) {
        // On the web the loop runs continuously while the GPU is coming up
        // (PollStrategy::Scheduler), so poll the setup future every
        // iteration until it is ready.
        if self.frost.is_none() {
            self.poll_gpu();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if let Some(frost) = self.frost.as_mut() {
            frost.window_event(event_loop, window_id, event);
            return;
        }
        // The GPU is still coming up: keep the window alive, let Escape and
        // a close request still exit, and keep asking for frames.
        if let WindowEvent::KeyboardInput { event, .. } = &event {
            if event.state == ElementState::Pressed && event.logical_key == NamedKey::Escape {
                log::info!("escape pressed, exiting");
                event_loop.exit();
                return;
            }
        }
        match event {
            WindowEvent::CloseRequested => {
                log::info!("window close requested, exiting");
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
        self.poll_gpu();
    }
}

#[cfg(target_arch = "wasm32")]
impl<P: Process> WebFrost<P> {
    /// The web app's initial state: nothing has happened yet — no window,
    /// the GPU setup not started, and no `Frost` to hand the canvas to.
    pub(crate) fn new(
        instance: Instance,
        scene: Scene,
        process: P,
        vsync: bool,
        window_size: Option<[u32; 2]>,
    ) -> Self {
        Self {
            instance: Some(instance),
            core: Some(Core {
                scene,
                process,
                vsync,
                window_size,
            }),
            window: None,
            gpu: GpuInit::Idle,
            frost: None,
        }
    }

    /// Polls the in-flight GPU setup; when it is ready, builds the `Frost`
    /// app and hands it the canvas.
    fn poll_gpu(&mut self) {
        let GpuInit::Init(future) = &mut self.gpu else {
            return; // Idle (before `resumed`) or already failed
        };
        // The event loop itself keeps running between polls (the web
        // ControlFlow::Poll strategy reschedules every frame), so a waker
        // that never wakes is fine: the next iteration polls again.
        let mut cx = TaskContext::from_waker(Waker::noop());
        let outcome = match future.as_mut().poll(&mut cx) {
            Poll::Pending => return, // the loop polls again on the next iteration
            Poll::Ready(outcome) => outcome,
        };

        let (adapter, device, queue) = match outcome {
            Ok(gpu) => gpu,
            Err(message) => {
                // No WebGPU (or the browser refused the device): surface the
                // error on the page instead of a blank canvas.
                self.gpu = GpuInit::Failed;
                log::error!("{message}");
                show_fallback(&message);
                return;
            }
        };
        self.gpu = GpuInit::Idle;

        // Invariant: `resumed` always ran first (winit bootstraps the loop
        // with a `Resumed` event before any poll iteration), so the window
        // and the core were created by then.
        let window = self
            .window
            .take()
            .expect("the window is created in `resumed` before the GPU setup completes");
        let instance = self
            .instance
            .take()
            .expect("the instance is created in `run`");
        let core = self
            .core
            .take()
            .expect("`core` is only taken once, when the GPU setup completes");

        let mut frost = Frost::new(
            instance,
            adapter,
            device,
            queue,
            core.vsync,
            core.window_size,
            core.scene,
            core.process,
        );
        frost.attach_window(window);
        self.frost = Some(frost);
    }
}

/// Removes the page's "Loading frost…" placeholder; the first frame has
/// landed.
#[cfg(target_arch = "wasm32")]
pub(crate) fn hide_fallback() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    if let Some(fallback) = document.get_element_by_id("fallback") {
        let _ = fallback.remove();
    }
}

/// Replaces the page's "Loading frost…" placeholder with `message` (in red)
/// so a failed GPU setup is visible instead of a blank canvas.
#[cfg(target_arch = "wasm32")]
fn show_fallback(message: &str) {
    use web_sys::wasm_bindgen::prelude::{JsCast, Upcast};

    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(fallback) = document.get_element_by_id("fallback") else {
        return;
    };
    // `set_text_content` lives on `Node`, so upcast from `Element`.
    let node: &web_sys::Node = fallback.upcast();
    node.set_text_content(Some(message));
    if let Some(fallback) = fallback.dyn_ref::<web_sys::HtmlElement>() {
        let _ = fallback.style().set_property("color", "#e5695e");
    }
}
