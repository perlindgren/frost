//! One renderer/window lifecycle for the entire plugin family.
use crate::editor::{Factory, WindowConfig, WindowState};
use baseview::{EventStatus, HandlerError, WindowContext, WindowHandler};
use frost::embedded::{Embedded, EmbeddedConfig, FrameError};
use nice_common::WindowGeom;
use nice_plug::context::gui::GuiContext;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

pub(crate) struct PluginWindow<U: frost::Process + 'static> {
    window: WindowContext,

    frame: Rc<FrameDriver<U>>,
    #[cfg(target_os = "macos")]
    display_sync: RefCell<Option<macos_frame_sync::DisplaySync>>,
    #[cfg(target_os = "macos")]
    frame_error: Rc<RefCell<Option<HandlerError>>>,
    state: Arc<dyn WindowState>,
    standalone: bool,

    /// Last geometry successfully written; size changes persist on every OS.
    geom_saved: Cell<Option<WindowGeom>>,
}

struct Engine<U: frost::Process + 'static> {
    /// The instance that the surface, adapter, and device were all made
    /// from. wgpu 30 scopes resource ids to their instance's registry, so
    /// a surface-lost repair MUST re-create the surface on this very
    /// instance — a fresh one would make every stored id cross registries.
    instance: wgpu::Instance,
    /// Owns the `wgpu::Surface` internally; resize handled by `Embedded`.
    /// A *lost* surface is detected via `Embedded::surface_lost()` and
    /// repaired with `replace_surface(fresh, format)`.
    embedded: Embedded<U>,
    format: wgpu::TextureFormat,
}

impl<U: frost::Process + 'static> PluginWindow<U> {
    pub fn new(
        window: WindowContext,
        state: Arc<dyn WindowState>,
        factory: Factory<U>,
        config: WindowConfig,
        gui_context: Option<GuiContext>,
        standalone: bool,
    ) -> Result<Self, HandlerError> {
        let frame = Rc::new(FrameDriver {
            window: window.clone(),
            engine: RefCell::new(None),
            factory,
            config,
            gui_context,
        });
        let this = Self {
            window,
            frame,
            #[cfg(target_os = "macos")]
            display_sync: RefCell::new(None),
            #[cfg(target_os = "macos")]
            frame_error: Rc::new(RefCell::new(None)),
            state,
            standalone,
            geom_saved: Cell::new(None),
        };
        this.frame.spawn_engine()?;
        #[cfg(target_os = "macos")]
        this.start_display_sync();
        #[cfg(all(windows, feature = "unsafe"))]
        if this.standalone && this.state.allow_placement() {
            let _ = window_position::restore(
                &this.window.platform_handle(),
                this.state.pending_position(),
            );
        }
        this.state.resized(this.window.size());
        this.persist_geometry();
        Ok(this)
    }

    /// Poll placement where the platform wrapper exposes it. Size persistence
    /// runs directly in the resize path on all platforms.
    #[cfg(all(windows, feature = "unsafe"))]
    fn track_geometry(&self) {
        if !self.standalone {
            return;
        }
        if !self.state.allow_placement() {
            return;
        }
        if let Ok(position) = window_position::top_left(&self.window.platform_handle()) {
            self.state.set_position(position);
            self.persist_geometry();
        }
    }

    fn persist_geometry(&self) {
        if !self.standalone {
            return;
        }
        let size = self.state.logical_size();
        if size.width <= 0.0 || size.height <= 0.0 {
            return; // Minimization must not replace the last usable size.
        }
        let position = self.state.position();
        let geometry = WindowGeom {
            x: position.map(|point| point.0),
            y: position.map(|point| point.1),
            width: size.width,
            height: size.height,
        };
        if self.geom_saved.get() == Some(geometry) {
            return;
        }
        match self.state.save_geometry(geometry) {
            Ok(()) => self.geom_saved.set(Some(geometry)),
            Err(error) => eprintln!(
                "{}: could not save window geometry: {error}",
                self.frame.config.title
            ),
        }
    }

    #[cfg(target_os = "macos")]
    fn start_display_sync(&self) {
        let frame = Rc::downgrade(&self.frame);
        let errors = Rc::clone(&self.frame_error);
        match macos_frame_sync::DisplaySync::new(&self.window.platform_handle(), move || {
            let Some(frame) = frame.upgrade() else {
                return false;
            };
            match frame.tick() {
                Ok(()) => true,
                Err(error) => {
                    *errors.borrow_mut() = Some(error);
                    false
                }
            }
        }) {
            Ok(sync) => {
                *self.display_sync.borrow_mut() = Some(sync);
                eprintln!(
                    "{}: editor rendering is display-synchronized",
                    self.frame.config.title
                );
            }
            Err(error) => eprintln!(
                "{}: {error}; using baseview frame timer",
                self.frame.config.title
            ),
        }
    }
}

impl<U: frost::Process + 'static> WindowHandler for PluginWindow<U> {
    fn on_frame(&self) -> Result<(), HandlerError> {
        #[cfg(all(windows, feature = "unsafe"))]
        self.track_geometry();
        #[cfg(target_os = "macos")]
        if let Some(sync) = self.display_sync.borrow().as_ref() {
            if let Some(error) = self.frame_error.borrow_mut().take() {
                return Err(error);
            }
            if sync.has_panicked() {
                return Err(HandlerError::from_boxed("display callback panicked".into()));
            }
            return Ok(()); // AppKit drives rendering; don't render twice.
        }
        self.frame.tick()
    }

    fn on_event(&self, event: baseview::Event) -> EventStatus {
        #[cfg(target_os = "macos")]
        if matches!(
            &event,
            baseview::Event::Window(baseview::WindowEvent::WillClose)
        ) {
            self.display_sync.borrow_mut().take();
        }
        if let Ok(mut guard) = self.frame.engine.try_borrow_mut()
            && let Some(engine) = guard.as_mut()
        {
            crate::input::forward(&mut engine.embedded, event, self.window.size().scale_factor);
        }
        EventStatus::Captured
    }

    fn resized(&self, new_size: baseview::WindowSize) -> Result<(), HandlerError> {
        self.state.resized(new_size);
        self.persist_geometry();
        #[cfg(target_os = "macos")]
        macos_metal_view::prepare(&self.window.platform_handle(), new_size.scale_factor)
            .map_err(|error| HandlerError::from(std::io::Error::other(error)))?;

        let Ok(mut guard) = self.frame.engine.try_borrow_mut() else {
            return Ok(());
        };
        if let Some(engine) = guard.as_mut() {
            // Embedded debounces swap-chain rebuilds internally (settles 100ms
            // after the last size event; scale changes apply instantly).
            engine.embedded.resize(
                (new_size.physical.width, new_size.physical.height),
                new_size.scale_factor,
            );
        }
        Ok(())
    }
}

// Shared only on the UI thread. The display callback holds a Weak reference,
// so dropping an editor cannot leave a retained renderer or callback cycle.
struct FrameDriver<U: frost::Process + 'static> {
    window: WindowContext,
    engine: RefCell<Option<Engine<U>>>,
    factory: Factory<U>,
    config: WindowConfig,
    gui_context: Option<nice_plug::context::gui::GuiContext>,
}

impl<U: frost::Process + 'static> FrameDriver<U> {
    fn spawn_engine(&self) -> Result<(), HandlerError> {
        let size = self.window.size();
        let scale = size.scale_factor;

        #[cfg(target_os = "macos")]
        macos_metal_view::prepare(&self.window.platform_handle(), scale)
            .map_err(|error| HandlerError::from(std::io::Error::other(error)))?;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = pollster::block_on(async {
            let surface = instance.create_surface(self.window.platform_handle())?;
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    force_fallback_adapter: false,
                    compatible_surface: Some(&surface),
                    ..Default::default()
                })
                .await
                .map_err(|e| HandlerError::from_boxed(format!("{e}").into()))?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some(self.config.title),
                    required_features: wgpu::Features::empty(),
                    required_limits: frost::embedded::device_limits(&adapter),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    ..Default::default()
                })
                .await
                .map_err(|e| HandlerError::from_boxed(e.to_string().into()))?;
            Ok::<_, HandlerError>((surface, adapter, device, queue))
        })?;
        let (surface, adapter, device, queue) = surface;

        let mut surface_config = surface
            .get_default_config(
                &adapter,
                size.physical.width.max(1),
                size.physical.height.max(1),
            )
            .ok_or_else(|| HandlerError::from_boxed("no surface config".into()))?;
        // Plugins must never block the host main thread:
        surface_config.present_mode = wgpu::PresentMode::AutoNoVsync;
        surface.configure(&device, &surface_config);
        let format = surface_config.format;

        let (scene, ui) = (self.factory)(self.gui_context.clone());

        // The instance and the adapter move in: frost keeps both for capability
        // queries, and both must be the very ones this device and surface came
        // from — wgpu 30 scopes every id to its instance's registry.
        let embedded = Embedded::new(
            &instance,
            device.clone(),
            queue.clone(),
            adapter,
            surface,
            format,
            (size.physical.width, size.physical.height),
            scale,
            scene,
            ui,
            EmbeddedConfig::default(),
        );

        *self.engine.borrow_mut() = Some(Engine {
            instance,
            embedded,
            format,
        });
        Ok(())
    }

    fn tick(&self) -> Result<(), HandlerError> {
        let Ok(mut guard) = self.engine.try_borrow_mut() else {
            return Ok(());
        };
        match guard.as_mut() {
            Some(engine) => match engine.embedded.frame() {
                // frame() may return Ok while having drawn nothing: the
                // honest loss signal is surface_lost(). Repair in place with
                // a fresh surface — on the SAME instance as device/adapter
                // (see Engine::instance).
                Ok(()) => {
                    if engine.embedded.surface_lost() {
                        let fresh = engine
                            .instance
                            .create_surface(self.window.platform_handle())
                            .map_err(|e| HandlerError::from_boxed(e.to_string().into()))?;
                        engine.embedded.replace_surface(fresh, engine.format);
                    }
                    Ok(())
                }
                // SizeInvalid = zero-sized panel mid-collapse: nothing was
                // ever presented, so there is nothing "lost"; `resize`
                // resumes. Never surface it to baseview — a HandlerError
                // tears the window down.
                Err(FrameError::SizeInvalid) => Ok(()),
                // The device is gone: rebuild device+surface+scene wholesale;
                // the baseview window itself is still alive.
                Err(FrameError::DeviceLost) | Err(_) => {
                    drop(guard);
                    *self.engine.borrow_mut() = None;
                    self.spawn_engine()
                }
            },
            None => self.spawn_engine(),
        }
    }
}

impl<U: frost::Process + 'static> Drop for PluginWindow<U> {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        self.display_sync.get_mut().take();
    }
}
