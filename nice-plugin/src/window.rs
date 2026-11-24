//! One renderer/window lifecycle for the entire plugin family.
use crate::editor::{Factory, WindowConfig, WindowState};
use baseview::{EventStatus, HandlerError, WindowContext, WindowHandler};
use frost::embedded::{Embedded, EmbeddedConfig, FrameError};
use nice_common::WindowGeom;
use nice_plug::context::gui::GuiContext;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

/// The Linux shadow XDND receiver's live session; a no-op type elsewhere,
/// where no such receiver exists to arm.
#[cfg(target_os = "linux")]
type Xdnd = crate::xdnd::Session;
#[cfg(not(target_os = "linux"))]
type Xdnd = ();

pub(crate) struct PluginWindow<U: frost::Process + 'static> {
    window: WindowContext,

    frame: FrameDriver<U>,
    state: Arc<dyn WindowState>,
    standalone: bool,

    /// Last geometry successfully written; size changes persist on every OS.
    geom_saved: Cell<Option<WindowGeom>>,

    /// Shadow XDND target on a private Xlib connection (Linux only; see
    /// [`crate::xdnd`]). Its thread drops the paths into a channel, and
    /// [`Self::drain_xdnd`] collects them on the window thread.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    xdnd: RefCell<Option<Xdnd>>,
    /// Last delivered drop, for collapsing the duplicate that arrives when
    /// baseview and the shadow receiver both answer one drag.
    last_drop: RefCell<Option<(Instant, Vec<PathBuf>)>>,
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
        let frame = FrameDriver {
            window: window.clone(),
            engine: RefCell::new(None),
            factory,
            config,
            gui_context,
        };
        let this = Self {
            window,
            frame,
            state,
            standalone,
            geom_saved: Cell::new(None),
            xdnd: RefCell::new(None),
            last_drop: RefCell::new(None),
        };
        this.frame.spawn_engine()?;
        #[cfg(target_os = "linux")]
        this.arm_xdnd();
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

    /// Hand one dropped batch to the plugin, exactly once per drag. Both
    /// delivery paths — baseview's own XDND handling, and the Linux shadow
    /// receiver — funnel here, and on KDE one drag answers both: the shadow
    /// completes the conversion first (baseview may still be waiting on its
    /// never-answered request, or a stricter sender may answer both). The
    /// short identical-batch guard turns that into one load, not two.
    fn deliver_files(&self, paths: Vec<PathBuf>) {
        let now = Instant::now();
        let duplicate = self
            .last_drop
            .borrow()
            .as_ref()
            .is_some_and(|(then, last)| {
                *last == paths && now.duration_since(*then) < std::time::Duration::from_millis(500)
            });
        if duplicate {
            return;
        }
        *self.last_drop.borrow_mut() = Some((now, paths.clone()));
        self.state.files_dropped(paths);
        self.window.request_redraw();
    }

    /// Move shadow-receiver drops onto the window thread. No-op off Linux,
    /// where there is nothing to receive.
    fn drain_xdnd(&self) {
        #[cfg(target_os = "linux")]
        if let Some(session) = self.xdnd.borrow_mut().as_mut() {
            while let Ok(paths) = session.paths.try_recv() {
                self.deliver_files(paths);
            }
        }
    }

    /// Shadow this window as an XDND target on a private connection.
    /// Baseview stays installed as a second target; see [`crate::xdnd`]
    /// for why it needs the partner, and [`Self::deliver_files`] for how a
    /// drop both of them answer lands once. Failure simply means baseview's
    /// answer is the only one — the pre-existing behavior.
    #[cfg(target_os = "linux")]
    fn arm_xdnd(&self) {
        if !self.state.accepts_files() {
            return;
        }
        use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
        let handle = self.window.platform_handle();
        let Ok(window) = handle.window_handle() else {
            return;
        };
        // Baseview publishes an XCB window id when it owns the window and
        // an Xlib one when the plugin runs inside a host; both name the
        // same X11 window, and the display pointer is irrelevant here
        // because the receiver opens its own connection.
        let window = match window.as_raw() {
            RawWindowHandle::Xcb(handle) => u64::from(handle.window.get()),
            RawWindowHandle::Xlib(handle) => handle.window,
            _ => return,
        };
        if let Ok(session) = crate::xdnd::spawn(window) {
            *self.xdnd.borrow_mut() = Some(session);
        }
    }
}

/// Stop the shadow receiver thread together with the window it shadows:
/// dropping the session flips its stop flag and joins (see
/// [`crate::xdnd::Receiver`]).
#[cfg(target_os = "linux")]
impl<U: frost::Process + 'static> Drop for PluginWindow<U> {
    fn drop(&mut self) {
        let _ = self.xdnd.borrow_mut().take();
    }
}

impl<U: frost::Process + 'static> WindowHandler for PluginWindow<U> {
    fn draw(&self) -> Result<(), HandlerError> {
        #[cfg(all(windows, feature = "unsafe"))]
        self.track_geometry();
        self.drain_xdnd();
        self.frame.tick()?;
        // Meters and waveforms can change without parameter notifications.
        // Request the next animation frame through Baseview's coalesced redraw
        // API rather than posting private timer messages from the host.
        self.window.request_redraw();
        Ok(())
    }

    fn on_event(&self, event: baseview::Event) -> EventStatus {
        // A file drop is not a pointer event the frost input machine can
        // route: it carries paths, not coordinates with meaning. Plugins
        // that opt in (`WindowState::accepts_files`) get them handed to the
        // shared state to drain on the UI thread; every other plugin falls
        // through to the old catch-all, which Windows and macOS read as a
        // refused drop. X11 ignores this return value entirely — baseview
        // and, on X11, our shadow receiver ([`crate::xdnd`]) answer the
        // source themselves — so on Linux a drop lands here whether or not
        // the plugin wants it, and one that does not simply drops it on
        // the floor.
        if let baseview::Event::Mouse(drag) = &event {
            let accepted = match drag {
                baseview::MouseEvent::DragEntered { .. }
                | baseview::MouseEvent::DragMoved { .. }
                | baseview::MouseEvent::DragDropped { .. }
                | baseview::MouseEvent::DragLeft => self.state.accepts_files(),
                _ => false,
            };
            if accepted {
                if let baseview::MouseEvent::DragDropped {
                    data: baseview::DropData::Files(paths),
                    ..
                } = drag
                {
                    self.deliver_files(paths.clone());
                }
                return EventStatus::AcceptDrop(baseview::DropEffect::Copy);
            }
        }
        if let Ok(mut guard) = self.frame.engine.try_borrow_mut()
            && let Some(engine) = guard.as_mut()
        {
            crate::input::forward(&mut engine.embedded, event, self.window.size().scale_factor);
        }
        self.window.request_redraw();
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

// Owned by Baseview's window thread. Baseview controls redraw scheduling and
// cancels its callbacks when the editor window is destroyed.
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
        // Avoid explicitly VSync-paced presentation on the host UI thread.
        // Acquisition and driver submission can still wait; AutoNoVsync is
        // not a non-blocking guarantee.
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
