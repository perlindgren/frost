//! STAGING (M4): replaces the `TunerWindow` half of src/lib.rs once
//! `frost::embedded::Embedded` (M3) lands. See stage/integration_notes.md.
//!
//! Responsibilities:
//! - own the wgpu surface on the plugin's own `wgpu::Instance`
//! - build the frost static scene + `Embedded` engine once
//! - drive frames from AppKit display callbacks, with a baseview timer fallback
//! - reconfigure on `resized`, recreate on surface `Lost`
//! - translate `baseview::Event` → frost input injection (docs/SCENE.md table)

use crate::{TunerParams, TunerSharedState};
use baseview::{EventStatus, HandlerError, WindowContext, WindowHandler};
use frost::embedded::{Embedded, EmbeddedConfig, FrameError};
#[cfg(feature = "standalone")]
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, atomic::AtomicBool};

// Native placement is opt-in and isolated in the wrapper crate.
#[cfg(all(windows, feature = "standalone", feature = "unsafe"))]
use window_position as place;

/// The engine's own OFL-licensed Fira Code, one copy for both: the plugin
/// draws its readout with the font frost ships, so the bytes come from the
/// repository's `assets/fonts/` rather than a vendored duplicate of the engine.
/// `Arc<[u8]>` for cheap shape sharing.
///
/// The path leaves this package, which a manifest cannot express as a
/// dependency — the build sees it through rustc's own dep-info, so editing the
/// font still rebuilds this crate, and `publish = false` means no tarball ever
/// has to carry a file from outside the package.
///
/// Our xtask ships the font licences inside VST3 bundles and beside CLAP files.
pub fn font() -> Arc<[u8]> {
    use std::sync::OnceLock;
    static BYTES: &[u8] = include_bytes!("../../assets/fonts/FiraCode-VariableFont_wght.ttf");
    static FONT: OnceLock<Arc<[u8]>> = OnceLock::new();
    FONT.get_or_init(|| Arc::from(BYTES)).clone()
}

/// Unmodified Fluent System Icons Regular; its MIT notice ships with bundles.
pub fn icon_font() -> Arc<[u8]> {
    use std::sync::OnceLock;
    static BYTES: &[u8] = include_bytes!("../assets/fonts/FluentSystemIcons-Regular.ttf");
    static FONT: OnceLock<Arc<[u8]>> = OnceLock::new();
    FONT.get_or_init(|| Arc::from(BYTES)).clone()
}

pub struct TunerWindow {
    window: WindowContext,

    frame: Rc<FrameDriver>,
    #[cfg(target_os = "macos")]
    display_sync: RefCell<Option<macos_frame_sync::DisplaySync>>,
    #[cfg(target_os = "macos")]
    frame_error: Rc<RefCell<Option<HandlerError>>>,
    shared: Arc<TunerSharedState>,
    _redraw_requested: Arc<AtomicBool>,

    /// Last geometry successfully written; size changes persist on every OS.
    #[cfg(feature = "standalone")]
    geom_saved: Cell<Option<crate::settings::WindowGeom>>,
}

struct TunerEngine {
    /// The instance that the surface, adapter, and device were all made
    /// from. wgpu 30 scopes resource ids to their instance's registry, so
    /// a surface-lost repair MUST re-create the surface on this very
    /// instance — a fresh one would make every stored id cross registries
    /// (regression: "Cannot get non-existent resource AdapterId").
    instance: wgpu::Instance,
    /// Owns the `wgpu::Surface` internally; resize handled by `Embedded`.
    /// A *lost* surface is detected via `Embedded::surface_lost()` and
    /// repaired with `replace_surface(fresh, format)`.
    embedded: Embedded<crate::tuner_ui::TunerUi>,
    format: wgpu::TextureFormat,
}

impl TunerWindow {
    pub fn new(
        window: WindowContext,
        params: Arc<TunerParams>,
        shared: Arc<TunerSharedState>,
        redraw: Arc<AtomicBool>,
        gui_context: Option<nice_plug::context::gui::GuiContext>,
    ) -> Result<Self, HandlerError> {
        let frame = Rc::new(FrameDriver {
            window: window.clone(),
            engine: RefCell::new(None),
            shared: Arc::clone(&shared),
            params,
            gui_context,
        });
        let this = Self {
            window,
            frame,
            #[cfg(target_os = "macos")]
            display_sync: RefCell::new(None),
            #[cfg(target_os = "macos")]
            frame_error: Rc::new(RefCell::new(None)),
            shared,
            _redraw_requested: redraw,
            #[cfg(feature = "standalone")]
            geom_saved: Cell::new(None),
        };
        this.frame.spawn_engine()?;
        #[cfg(target_os = "macos")]
        this.start_display_sync();
        // Standalone top-level: honour the saved spot, or centre when no
        // geometry was ever saved. A host-embedded child is left exactly
        // where the host put it (STANDALONE_MODE gate inside).
        #[cfg(all(windows, feature = "standalone", feature = "unsafe"))]
        if crate::STANDALONE_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            let saved = *this.shared.pending_geom.lock().unwrap();
            let _ = place::restore(&this.window.platform_handle(), saved);
        }
        this.shared
            .logical_size
            .store(this.window.size().logical.cast());
        #[cfg(feature = "standalone")]
        this.persist_geometry();
        Ok(this)
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
                eprintln!("nice-tuner: editor rendering is display-synchronized");
            }
            Err(error) => eprintln!("nice-tuner: {error}; using baseview frame timer"),
        }
    }

    /// Poll placement where the platform wrapper exposes it. Size persistence
    /// runs directly in resized() on all platforms.
    #[cfg(all(windows, feature = "standalone", feature = "unsafe"))]
    fn track_geometry(&self) {
        if !crate::STANDALONE_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        if let Ok(position) = place::top_left(&self.window.platform_handle()) {
            *self.shared.window_pos.lock().unwrap() = Some(position);
            self.persist_geometry();
        }
    }

    #[cfg(feature = "standalone")]
    fn persist_geometry(&self) {
        use crate::settings::{Settings, WindowGeom};
        if !crate::STANDALONE_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let size = self.shared.logical_size();
        if size.width <= 0.0 || size.height <= 0.0 {
            return; // Minimization must not replace the last usable size.
        }
        let position = *self.shared.window_pos.lock().unwrap();
        let geometry = WindowGeom {
            x: position.map(|point| point.0),
            y: position.map(|point| point.1),
            width: size.width,
            height: size.height,
        };
        if self.geom_saved.get() == Some(geometry) {
            return;
        }
        match Settings::save_window(geometry) {
            Ok(()) => self.geom_saved.set(Some(geometry)),
            Err(error) => eprintln!("nice-tuner: could not save window geometry: {error}"),
        }
    }
}

impl WindowHandler for TunerWindow {
    fn on_frame(&self) -> Result<(), HandlerError> {
        #[cfg(all(windows, feature = "standalone", feature = "unsafe"))]
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
        use baseview::{Event as B, MouseEvent as M, WindowEvent as W};
        use keyboard_types::KeyState;
        #[cfg(target_os = "macos")]
        if matches!(&event, B::Window(W::WillClose)) {
            self.display_sync.borrow_mut().take();
        }
        let mut guard = self.frame.engine.borrow_mut();
        if let Some(engine) = guard.as_mut() {
            let size = self.window.size();
            let scale = size.scale_factor;
            match event {
                B::Keyboard(ke) => {
                    let down = ke.state == KeyState::Down;
                    if let Some(code) = crate::tuner_ui::map_key(&ke.key) {
                        engine.embedded.set_key(code, down);
                    }
                    // keyboard_types 0.8: typed characters ride in
                    // `key: Key::Character` on key-down events
                    if down && let keyboard_types::Key::Character(s) = &ke.key {
                        for ch in s.chars() {
                            engine.embedded.add_text(ch);
                        }
                    }
                    use keyboard_types::Modifiers;
                    engine.embedded.set_modifiers(
                        ke.modifiers.contains(Modifiers::SHIFT),
                        ke.modifiers.contains(Modifiers::CONTROL),
                        ke.modifiers.contains(Modifiers::ALT),
                        ke.modifiers.contains(Modifiers::META),
                    );
                }
                B::Mouse(mouse) => match mouse {
                    // baseview 0.3.3 macOS delivers PHYSICAL px (view.rs
                    // to_physical); frost's set_cursor wants logical px,
                    // y-down from the surface top-left -> just un-scale.
                    M::CursorMoved { position, .. } => {
                        engine
                            .embedded
                            .set_cursor(position.x / scale, position.y / scale);
                    }
                    M::ButtonPressed { button, .. } => {
                        if let Some(btn) = crate::tuner_ui::map_mouse(button) {
                            engine.embedded.set_mouse_button(btn, true);
                        }
                    }
                    M::ButtonReleased { button, .. } => {
                        if let Some(btn) = crate::tuner_ui::map_mouse(button) {
                            engine.embedded.set_mouse_button(btn, false);
                        }
                    }
                    M::WheelScrolled { delta, .. } => {
                        // frost's scroll dy is in LINES (raw); pixels /120
                        let (dx, dy) = match delta {
                            baseview::ScrollDelta::Lines { y, .. } => (0.0, y as f64),
                            baseview::ScrollDelta::Pixels { y, .. } => (0.0, y as f64 / 120.0),
                        };
                        engine.embedded.scroll(dx, dy);
                    }
                    M::CursorLeft => engine.embedded.cursor_left(),
                    _ => {}
                },
                B::Window(W::Focused) => engine.embedded.set_focused(true),
                B::Window(W::Unfocused) => engine.embedded.set_focused(false),
                // window lifecycle stays with the host
                B::Window(W::WillClose) => {
                    #[cfg(feature = "standalone")]
                    self.persist_geometry();
                }
                _ => {}
            }
            // Rendering is driven by AppKit on supported macOS versions,
            // otherwise by baseview's on_frame timer.
        }
        baseview::EventStatus::Captured
    }

    fn resized(&self, new_size: baseview::WindowSize) -> Result<(), HandlerError> {
        self.shared.logical_size.store(new_size.logical.cast());
        self.shared.scale_factor.store(new_size.scale_factor);
        #[cfg(feature = "standalone")]
        self.persist_geometry();

        let mut guard = self.frame.engine.borrow_mut();
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
struct FrameDriver {
    window: WindowContext,
    engine: RefCell<Option<TunerEngine>>,
    shared: Arc<TunerSharedState>,
    params: Arc<TunerParams>,
    gui_context: Option<nice_plug::context::gui::GuiContext>,
}
impl FrameDriver {
    fn spawn_engine(&self) -> Result<(), HandlerError> {
        let size = self.window.size();
        let scale = size.scale_factor;

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
                    label: Some("nice-tuner"),
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

        let mut ui = crate::tuner_ui::TunerUi::new(Arc::clone(&self.shared), font());
        if let Some(context) = &self.gui_context {
            ui.set_host_controls(crate::host_controls::HostControls::new(
                Arc::clone(&self.params),
                context.clone(),
            ));
        }
        let scene = crate::tuner_ui::static_scene();

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

        *self.engine.borrow_mut() = Some(TunerEngine {
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
                // SizeInvalid = zero-sized panel mid-collapse: nothing to
                // draw, `resize` resumes. Never surface it to baseview — a
                // HandlerError would tear the window down.
                // frame() may return Ok while having drawn nothing: the
                // honest loss signal is surface_lost(). Repair in place with
                // a fresh surface — on the SAME instance as device/adapter
                // (see TunerEngine::instance).
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
                // Zero-size panel mid-collapse: nothing was ever presented,
                // so there is nothing "lost"; `resize` resumes. Never
                // surface it to baseview — a HandlerError tears the window
                // down.
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

impl Drop for TunerWindow {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        self.display_sync.get_mut().take();
    }
}
