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

use super::Frost;
use super::windowing::create_window;
use crate::Process;
use crate::objects::{Scene, SpriteFilter};

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
    /// The physical-pixel size request, carried for symmetry with the
    /// native config; `create_window` ignores it on the web.
    window_size_px: Option<[u32; 2]>,
    /// The resizability request; `set_resizable` is a no-op on the web,
    /// where the page owns the canvas size, but it is carried so
    /// `attach_window` runs the same code as native.
    resizable: bool,
    /// The fixed render size, honored on the web like anywhere: the frame
    /// renders to a buffer of this size and the blit stretches it over the
    /// canvas.
    render_size: Option<[u32; 2]>,
    /// The stretch pass's resampling filter (see `Config::blit_filter`).
    blit_filter: SpriteFilter,
    /// Whether Escape, on its own, ends the app (see
    /// `Config::escape_exits`).
    escape_exits: bool,
    /// Whether the close button, on its own, ends the app (see
    /// `Config::close_exits`).
    close_exits: bool,
}

/// The in-flight async GPU setup on the web.
#[cfg(target_arch = "wasm32")]
enum GpuInit {
    /// Not started yet (before the first `resumed`).
    Idle,
    /// The combined `request_adapter` + `request_device` future.
    Init(GpuFuture),
    /// The setup failed and the page shows the error.
    Failed,
}

/// The combined `request_adapter` + `request_device` future: resolves to
/// the GPU trio, or to the reason it failed, as a ready-made object
/// because the event loop owns it across polls.
#[cfg(target_arch = "wasm32")]
type GpuFuture = Pin<Box<dyn Future<Output = Result<(Adapter, Device, Queue), String>>>>;

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
    /// A close request that arrived before the app did — delivered
    /// the moment there is a `Frost` to hear it.
    pending_close: bool,
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
            // `and_then`, not `map`: `core.window_size` is itself an
            // `Option`, so `map` would hand the window creator a doubled
            // one.
            let window_size = self.core.as_ref().and_then(|core| core.window_size);
            let window_size_px = self.core.as_ref().and_then(|core| core.window_size_px);
            self.window = Some(create_window(event_loop, window_size, window_size_px));
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
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && event.state == ElementState::Pressed
            && event.logical_key == NamedKey::Escape
            && self.core.as_ref().is_none_or(|core| core.escape_exits)
        {
            log::info!("escape pressed, exiting");
            event_loop.exit();
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                if self.core.as_ref().is_none_or(|core| core.close_exits) {
                    log::info!("window close requested, exiting");
                    event_loop.exit();
                    return;
                }
                // The app owns the question, and no app exists until
                // the GPU finishes: the request is kept and delivered
                // the moment there is someone to ask.
                log::info!("window close requested, waiting for the app");
                self.pending_close = true;
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
    #[allow(clippy::too_many_arguments)] // the whole `Config` arrives at once
    pub(crate) fn new(
        instance: Instance,
        scene: Scene,
        process: P,
        vsync: bool,
        window_size: Option<[u32; 2]>,
        window_size_px: Option<[u32; 2]>,
        resizable: bool,
        render_size: Option<[u32; 2]>,
        blit_filter: SpriteFilter,
        escape_exits: bool,
        close_exits: bool,
    ) -> Self {
        Self {
            instance: Some(instance),
            core: Some(Core {
                scene,
                process,
                vsync,
                window_size,
                window_size_px,
                resizable,
                render_size,
                blit_filter,
                escape_exits,
                close_exits,
            }),
            window: None,
            gpu: GpuInit::Idle,
            frost: None,
            pending_close: false,
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
            core.window_size_px,
            core.resizable,
            core.render_size,
            core.blit_filter,
            core.escape_exits,
            core.close_exits,
            core.scene,
            core.process,
        );
        frost.attach_window(window);
        if self.pending_close {
            frost.note_close_request();
        }
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
        // `remove` detaches the element and returns nothing; the DOM
        // keeps no error path here — a missing node is the success case.
        fallback.remove();
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
