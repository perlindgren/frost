//! nice-plug and baseview adaptation shared by hosted and standalone views.
use baseview::{HandlerError, WindowContext, WindowSettings};
use frost::{Process, Scene};
use nice_common::WindowGeom;
use nice_plug::context::gui::GuiContext;
use nice_plug::editor::dpi::{LogicalSize, NativeSize};
use nice_plug::editor::{HostMethods, ParentWindowHandle, ResizeHint, SpawnedEditor};
use std::{error::Error, sync::Arc};

/// Plugin-owned window preferences, kept separate from DSP/parameter state.
/// Standalone persistence hooks default to no-ops for hosted instances.
pub trait WindowState: Send + Sync + 'static {
    fn logical_size(&self) -> LogicalSize<f32>;
    fn scale_factor(&self) -> f64;
    fn resized(&self, size: baseview::WindowSize);
    fn set_open(&self, open: bool);
    fn standalone(&self) -> bool {
        false
    }
    fn allow_placement(&self) -> bool {
        false
    }
    fn prepare(&self, _parented: bool) {}
    fn position(&self) -> Option<(i32, i32)> {
        None
    }
    fn pending_position(&self) -> Option<(i32, i32)> {
        None
    }
    fn set_position(&self, _position: (i32, i32)) {}
    fn save_geometry(&self, _geometry: WindowGeom) -> Result<(), String> {
        Ok(())
    }
}
#[derive(Clone, Copy)]
pub struct WindowConfig {
    pub title: &'static str,
    pub min_size: LogicalSize<f32>,
    pub resize_hint: ResizeHint,
}
pub(crate) type Factory<U> = Arc<dyn Fn(Option<GuiContext>) -> (Scene, U) + Send + Sync>;

pub struct Editor<U: Process + 'static> {
    state: Arc<dyn WindowState>,
    config: WindowConfig,
    factory: Factory<U>,
}
impl<U: Process + 'static> Editor<U> {
    pub fn new(
        state: Arc<dyn WindowState>,
        config: WindowConfig,
        factory: impl Fn(Option<GuiContext>) -> (Scene, U) + Send + Sync + 'static,
    ) -> Self {
        Self {
            state,
            config,
            factory: Arc::new(factory),
        }
    }
    pub fn spawn_window(
        &self,
        parent: Option<ParentWindowHandle>,
        wait_for_parent: bool,
        fallback_scale_factor: Option<f64>,
        host: Option<HostMethods>,
        gui_context: Option<GuiContext>,
    ) -> Result<SpawnedEditor<EditorHandle>, Box<dyn Error>> {
        #[cfg(target_os = "macos")]
        macos_metal_view::require_editor_support()?;
        self.state.prepare(parent.is_some());
        // The host is a re-implementation of baseview's `HostCallbacks` /
        // `HostMainThreadCaller` to keep `baseview` out of nice-plug-core.
        // Adapt it to baseview's traits (same adapter as nice-nam).
        let host = {
            struct HostCallbackAdapter {
                host: Box<dyn nice_plug::editor::HostCallbacks>,
            }
            impl baseview::host::HostCallbacks for HostCallbackAdapter {
                fn request_resize(
                    &mut self,
                    new_size: baseview::WindowSize,
                ) -> Result<(), HandlerError> {
                    self.host
                        .request_resize(new_size.physical.into(), new_size.scale_factor)
                        .map_err(HandlerError::from_boxed)
                }

                fn destroyed(&mut self) {
                    self.host.destroyed();
                }
            }
            struct HostMainThreadCallerAdapter {
                host: Box<dyn nice_plug::editor::HostMainThreadCaller>,
            }
            impl baseview::host::HostMainThreadCaller for HostMainThreadCallerAdapter {
                fn call_main_thread(&mut self) {
                    self.host.call_main_thread();
                }
            }
            host.map(|host| {
                baseview::host::Host::new()
                    .with_callbacks(HostCallbackAdapter {
                        host: host.callbacks,
                    })
                    .with_main_thread(HostMainThreadCallerAdapter {
                        host: host.main_thread_caller,
                    })
            })
        };

        let state = Arc::clone(&self.state);
        let factory = Arc::clone(&self.factory);
        let config = self.config;
        let standalone = state.standalone() && parent.is_none();
        let window = baseview::Window::create_with_host(
            WindowSettings::new()
                .with_title(config.title)
                .with_size(state.logical_size())
                .with_min_size::<LogicalSize<f32>>(Some(config.min_size))
                .with_resizable(config.resize_hint.can_resize)
                .with_parent(parent.as_ref())
                .with_wait_for_parent(wait_for_parent)
                .with_fallback_scale_factor(fallback_scale_factor),
            move |window: WindowContext| {
                crate::window::PluginWindow::new(
                    window,
                    state,
                    factory,
                    config,
                    gui_context,
                    standalone,
                )
            },
            host,
        )?;
        let waker = window.waker();
        self.state.set_open(true);
        Ok(SpawnedEditor {
            handle: EditorHandle {
                state: self.state.clone(),
                waker,
                hint: config.resize_hint,
            },
            window,
        })
    }
}
impl<U: Process + 'static> nice_plug::editor::Editor for Editor<U> {
    type Handle = EditorHandle;
    fn spawn(
        &self,
        parent: Option<ParentWindowHandle>,
        wait_for_parent: bool,
        fallback_scale_factor: Option<f64>,
        gui_context: GuiContext,
        host: Option<HostMethods>,
    ) -> Result<SpawnedEditor<EditorHandle>, Box<dyn Error>> {
        self.spawn_window(
            parent,
            wait_for_parent,
            fallback_scale_factor,
            host,
            Some(gui_context),
        )
    }
    fn size(&self) -> NativeSize<u32> {
        NativeSize::from_size(self.state.logical_size().into(), self.state.scale_factor())
    }
    fn resize_hint(&self) -> ResizeHint {
        self.config.resize_hint
    }
}
pub struct EditorHandle {
    state: Arc<dyn WindowState>,
    waker: baseview::WindowWaker,
    hint: ResizeHint,
}
impl nice_plug::editor::EditorHandle for EditorHandle {
    type Window = baseview::Window;
    type Error = baseview::Error;

    fn run_until_closed(window: Self::Window) -> Result<(), Self::Error> {
        window.run_until_closed()
    }

    fn set_parent(
        &self,
        parent: ParentWindowHandle,
        window: &Self::Window,
    ) -> Result<(), Self::Error> {
        window.set_parent(&parent)
    }

    fn show(&self, window: &Self::Window) -> Result<(), Self::Error> {
        let res = window.show();

        // A host can show an editor without changing any parameters.
        self.waker.request_redraw();

        res
    }

    fn hide(&self, window: &Self::Window) -> Result<(), Self::Error> {
        window.hide()
    }

    fn host_main_thread_callback(&self, window: &Self::Window) {
        window.host_main_thread_callback();
    }

    fn set_size(
        &self,
        new_size: NativeSize<u32>,
        window: &Self::Window,
    ) -> Result<(), Self::Error> {
        window.resize(new_size)
    }

    fn set_fallback_scale_factor(
        &self,
        scale_factor: f64,
        window: &Self::Window,
    ) -> Result<(), Self::Error> {
        window.suggest_fallback_scale_factor(scale_factor)
    }

    /// Return the closest supported size.
    fn adjust_size(
        &self,
        new_size: NativeSize<u32>,
        window: &Self::Window,
    ) -> Option<NativeSize<u32>> {
        let current_size = window.size();
        Some(self.hint.adjust_size(
            new_size,
            NativeSize::from_logical_or_physical(current_size.logical, current_size.physical),
            current_size.scale_factor,
        ))
    }

    fn state_changed(&self) {
        self.waker.request_redraw();
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
        self.waker.request_redraw();
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {
        self.waker.request_redraw();
    }
}

impl Drop for EditorHandle {
    fn drop(&mut self) {
        self.state.set_open(false);
    }
}
