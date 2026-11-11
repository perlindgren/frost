//! Editor plumbing: the nice-plug `Editor` trait and the baseview window
//! factory. Standalone hosting arrives with the standalone feature; the window
//! is the same either way, which is the whole point of this file — the editor
//! a host embeds and the window the standalone opens are built by one
//! `spawn_window`, as in nice-wah and nice-nam.
use crate::*;
use baseview::{HandlerError, WindowContext, WindowSettings};
use nice_plug::context::gui::GuiContext;
use nice_plug::editor::dpi::NativeSize;
use nice_plug::editor::{EditorHandle, HostMethods, SpawnedEditor};
use std::error::Error;

pub struct PiFaceEditor {
    pub(crate) params: Arc<PiFaceParams>,
    pub(crate) shared: Arc<PiFaceSharedState>,
}

impl PiFaceEditor {
    pub(crate) fn spawn_window(
        &self,
        parent: Option<ParentWindowHandle>,
        wait_for_parent: bool,
        fallback_scale_factor: Option<f64>,
        host: Option<HostMethods>,
        gui_context: Option<GuiContext>,
    ) -> Result<SpawnedEditor<PiFaceEditorHandle>, Box<dyn Error>> {
        let params = Arc::clone(&self.params);
        let shared = Arc::clone(&self.shared);

        let redraw_requested = Arc::new(AtomicBool::new(true));
        let redraw_requested_2 = Arc::clone(&redraw_requested);

        // The host is a re-implementation of baseview's `HostCallbacks` /
        // `HostMainThreadCaller` to keep `baseview` out of nice-plug-core.
        // Adapt it to baseview's traits (the same adapter nice-wah runs).
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

        let window = baseview::Window::create_with_host(
            WindowSettings::new()
                .with_title("Nice Pi-Fase")
                .with_size(self.shared.logical_size())
                .with_min_size::<LogicalSize<f32>>(Some(MIN_SIZE))
                .with_resizable(RESIZE_HINT.can_resize)
                .with_parent(parent.as_ref())
                .with_wait_for_parent(wait_for_parent)
                .with_fallback_scale_factor(fallback_scale_factor),
            move |window: WindowContext| -> Result<gui::PiFaceWindow, HandlerError> {
                gui::PiFaceWindow::new(window, params, shared, redraw_requested_2, gui_context)
            },
            host,
        )?;

        self.shared.open.store(true, Ordering::Release);

        Ok(SpawnedEditor {
            handle: PiFaceEditorHandle {
                shared: Arc::clone(&self.shared),
                redraw_requested,
            },
            window,
        })
    }
}

impl Editor for PiFaceEditor {
    type Handle = PiFaceEditorHandle;

    fn spawn(
        &self,
        parent: Option<ParentWindowHandle>,
        wait_for_parent: bool,
        fallback_scale_factor: Option<f64>,
        gui_context: GuiContext,
        host: Option<HostMethods>,
    ) -> Result<SpawnedEditor<Self::Handle>, Box<dyn Error>> {
        self.spawn_window(
            parent,
            wait_for_parent,
            fallback_scale_factor,
            host,
            Some(gui_context),
        )
    }

    fn size(&self) -> NativeSize<u32> {
        NativeSize::from_size(
            self.shared.logical_size().into(),
            self.shared.scale_factor(),
        )
    }

    fn resize_hint(&self) -> ResizeHint {
        RESIZE_HINT
    }
}

/// A handle to a spawned instance of our editor.
pub struct PiFaceEditorHandle {
    pub(crate) shared: Arc<PiFaceSharedState>,
    redraw_requested: Arc<AtomicBool>,
}

impl EditorHandle for PiFaceEditorHandle {
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

        // baseview quirk: `WindowEvent::Focused` does not always fire after
        // `show()` — force a redraw manually, as the other editors in the
        // family do.
        self.redraw_requested.store(true, Ordering::Relaxed);

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
        Some(RESIZE_HINT.adjust_size(
            new_size,
            NativeSize::from_logical_or_physical(current_size.logical, current_size.physical),
            current_size.scale_factor,
        ))
    }

    fn state_changed(&self) {
        self.redraw_requested.store(true, Ordering::Relaxed);
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
        self.redraw_requested.store(true, Ordering::Relaxed);
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {
        self.redraw_requested.store(true, Ordering::Relaxed);
    }
}

impl Drop for PiFaceEditorHandle {
    fn drop(&mut self) {
        self.shared.open.store(false, Ordering::Release);
    }
}
