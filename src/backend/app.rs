use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::future::Future;
#[cfg(not(target_arch = "wasm32"))]
use std::task::{Context as TaskContext, Poll, Waker};

use wgpu::{
    Adapter, AddressMode, BindGroup, BindGroupDescriptor, BindGroupEntry, BindingResource, Buffer,
    BufferBinding, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    CommandEncoderDescriptor, CurrentSurfaceTexture, Device, Extent3d, FilterMode, FragmentState,
    Instance, MipmapFilterMode, MultisampleState, PipelineCompilationOptions, PresentMode,
    PrimitiveState, Queue, RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, Sampler, SamplerDescriptor, ShaderModule, ShaderModuleDescriptor,
    ShaderSource, StoreOp, Surface, SurfaceTexture, TexelCopyBufferLayout, TexelCopyTextureInfo,
    TextureAspect, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor, VertexState,
};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
#[cfg(not(target_arch = "wasm32"))]
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::backend::frame::*;
use crate::objects::*;
use crate::shaders::*;
use crate::text;
use crate::{Canvas, Context, KeyCode, Process};
#[cfg(target_arch = "wasm32")]
use crate::backend::wasm::hide_fallback;

/// The surface presentation mode for the user's vsync request: `AutoVsync`
/// presents once per vertical blank (capped at the display's refresh rate),
/// `AutoNoVsync` presents as soon as frames are rendered. Both `Auto*`
/// modes fall back to a supported mode when their preference is unavailable,
/// so they are safe to request on every platform.
pub(crate) fn present_mode_for(vsync: bool) -> PresentMode {
    if vsync {
        PresentMode::AutoVsync
    } else {
        PresentMode::AutoNoVsync
    }
}

pub(crate) struct Frost<P: Process> {
    instance: Instance,
    adapter: Adapter,
    device: Device,
    queue: Queue,
    /// The user's vsync request; maps to the surface's presentation mode
    /// (`PresentMode::AutoVsync` / `AutoNoVsync`) when the surface is
    /// (re)configured.
    vsync: bool,
    /// The `Config`'s initial window inner size in logical pixels, or
    /// `None` for the platform default; used once in `create_window`.
    window_size: Option<[u32; 2]>,
    #[allow(dead_code)]
    window_id: Option<WindowId>,
    /// The winit window (shared), kept so we can call `request_redraw` for
    /// continuous per-frame rendering.
    window: Option<Arc<Window>>,
    logical_size: (u32, u32),
    scale: f32,
    surface: Option<Surface<'static>>,
    line_pipeline: Option<RenderPipeline>,
    circle_pipeline: Option<RenderPipeline>,
    rect_pipeline: Option<RenderPipeline>,
    shape_pipeline: Option<RenderPipeline>,
    sprite_pipeline: Option<RenderPipeline>,
    particle_pipeline: Option<RenderPipeline>,
    /// The index buffer for particle batches: one quad per instance
    /// (`[0, 1, 2, 2, 1, 3]`), created once and shared by every batched
    /// draw in every frame.
    particle_index_buffer: Option<Buffer>,
    /// A 1x1 white placeholder texture and its sampler, bound by every
    /// non-sprite particle batch: the particle pipeline's bind group always
    /// carries a texture (binding 2) and a sampler (binding 3), but the SDF
    /// kinds — circles and rectangles — never sample it.
    particle_placeholder: (TextureView, Sampler),
    /// The frame's light field storage buffer: a 32-byte header plus one
    /// 32-byte record per light at the peak count so far. It starts at the
    /// header's size (a lightless frame still binds it — the shaders read
    /// the field for the ambient) and only ever grows: the WGSL field's
    /// tail is an unsized array sized by the bound buffer, so a bigger
    /// buffer simply holds more records, and a lighter frame writes a
    /// shorter slice into the same buffer.
    field_buffer: Buffer,
    /// The frame's occluder field storage buffer: a 16-byte header plus one
    /// 48-byte record per occluder at the peak count so far. Same
    /// grow-only rationale as [`Frost::field_buffer`]; a frame with no
    /// occluders still binds it (the shaders read the field's count), and
    /// the WGSL field's tail is an unsized array sized by the bound buffer.
    occluder_buffer: Buffer,
    /// The GPU resources for each distinct sprite image, keyed by the
    /// pointer of its pixel-data `Arc`. Sprites sharing one file share one
    /// texture, so the map stays bounded by the number of distinct images.
    /// A `TextureView` keeps its texture alive, so only the view and the
    /// sampler are stored.
    sprite_resources: HashMap<*const (), (TextureView, Sampler)>,
    /// The rasterized glyph atlas for each distinct `(font, size)` pair,
    /// keyed by the font buffer's pointer and the size's bits. Kept between
    /// frames so unchanged text never re-rasterizes and its pixel buffer —
    /// and therefore the GPU texture in `sprite_resources` — keeps a stable
    /// identity.
    text_atlases: HashMap<(u64, u32), text::Atlas>,
    /// The surface format the current pipelines were built for; they are only
    /// rebuilt when this changes.
    format: Option<TextureFormat>,
    /// Millisecond timestamp of the previous rendered frame, used to
    /// compute `dt` (see `now_millis`).
    last_millis: Option<f64>,
    /// The expected frame rate in Hz: the refresh rate of the monitor the
    /// window sits on, as winit reports it. `None` when vsync is off
    /// (presentation is uncapped) or the rate is unknown; see
    /// `Context::expected_fps`.
    expected_fps: Option<f32>,
    /// The scene drawn every frame, after the process runs.
    scene: Scene,
    /// The physical keys currently held down, updated as keyboard events arrive.
    keys: HashSet<KeyCode>,
    /// The mouse cursor's position in physical pixels, `(0, 0)` at the
    /// window's upper-left corner, or `None` when the cursor is outside the
    /// window. Updated as cursor events arrive; the frame's `Canvas` works
    /// in the same physical-pixel space.
    mouse: Option<[f32; 2]>,
    /// The mouse buttons currently held down, updated as mouse input
    /// events arrive.
    mouse_buttons: HashSet<MouseButton>,
    process: P,
}

impl<P: Process> Frost<P> {
    /// The app's initial state: the GPU is ready, but there is no window
    /// and no surface yet — `attach_window` creates both.
    #[allow(clippy::too_many_arguments)] // the full GPU setup and config arrive at once
    pub(crate) fn new(
        instance: Instance,
        adapter: Adapter,
        device: Device,
        queue: Queue,
        vsync: bool,
        window_size: Option<[u32; 2]>,
        scene: Scene,
        process: P,
    ) -> Self {
        // The particle batch's index buffer is surface-format independent,
        // so it is built here: every instance is one quad,
        // `[0, 1, 2, 2, 1, 3]`. It is written before any draw can run, so a
        // `write_buffer` upload has no hazard.
        let particle_index_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("particle index buffer"),
            size: 24,
            usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut index_bytes = [0u8; 24];
        for (i, v) in [0u32, 1, 2, 2, 1, 3].iter().enumerate() {
            index_bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        queue.write_buffer(&particle_index_buffer, 0, &index_bytes);

        // The light field buffer starts at the header's size: even a frame
        // with no lights binds it (the shaders read the field to fetch the
        // ambient), and it only grows when the light count peaks higher.
        let field_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("light field buffer"),
            size: LIGHT_FIELD_HEADER as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // The occluder field buffer starts at the header's size: even a
        // frame with no occluders binds it (the shaders read the field's
        // count), and it only grows when the occluder count peaks higher.
        let occluder_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("occluder field buffer"),
            size: OCCLUDER_FIELD_HEADER as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // The particle pipeline's bind group always carries a texture and a
        // sampler: a sprite batch binds its image, and the SDF kinds —
        // circles and rectangles, which never sample — bind this 1x1 white
        // placeholder. It is surface-format independent, so it is built
        // here, like the particle index buffer.
        let placeholder_texture = device.create_texture(&TextureDescriptor {
            label: Some("particle placeholder texture"),
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &placeholder_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &[255, 255, 255, 255],
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let particle_placeholder = (
            placeholder_texture.create_view(&TextureViewDescriptor::default()),
            device.create_sampler(&SamplerDescriptor {
                label: Some("particle placeholder sampler"),
                address_mode_u: AddressMode::ClampToEdge,
                address_mode_v: AddressMode::ClampToEdge,
                address_mode_w: AddressMode::ClampToEdge,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                mipmap_filter: MipmapFilterMode::Nearest,
                lod_min_clamp: 0.0,
                lod_max_clamp: f32::MAX,
                compare: None,
                anisotropy_clamp: 1,
                border_color: None,
            }),
        );

        Self {
            instance,
            adapter,
            device,
            queue,
            vsync,
            window_size,
            window_id: None,
            window: None,
            logical_size: (0, 0),
            scale: 1.0,
            surface: None,
            line_pipeline: None,
            circle_pipeline: None,
            rect_pipeline: None,
            shape_pipeline: None,
            sprite_pipeline: None,
            particle_pipeline: None,
            particle_index_buffer: Some(particle_index_buffer),
            particle_placeholder,
            field_buffer,
            occluder_buffer,
            sprite_resources: HashMap::new(),
            text_atlases: HashMap::new(),
            format: None,
            last_millis: None,
            expected_fps: None,
            scene,
            keys: HashSet::new(),
            mouse: None,
            mouse_buttons: HashSet::new(),
            process,
        }
    }
}

/// Creates the frost window and requests its first frame.
///
/// `window_size` is the `Config`'s initial inner size in logical pixels;
/// `None` keeps the platform default. On the web, winit's canvas is
/// neither appended to the page nor sized by default: without the append
/// it is invisible, and without a size it stays the browser's 300x150
/// default — so an unset size falls back to 900x600 there.
pub(crate) fn create_window(
    event_loop: &ActiveEventLoop,
    window_size: Option<[u32; 2]>,
) -> Arc<Window> {
    let mut attributes = Window::default_attributes().with_title("frost");
    #[cfg(not(target_arch = "wasm32"))]
    if let Some([width, height]) = window_size {
        attributes = attributes.with_inner_size(LogicalSize::new(width, height));
    }
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::WindowAttributesExtWebSys;

        let [width, height] = window_size.unwrap_or([900, 600]);
        attributes = attributes
            .with_append(true)
            .with_inner_size(LogicalSize::new(width, height));
    }
    let window = Arc::new(
        event_loop
            .create_window(attributes)
            .expect("failed to create window"),
    );
    // Center the window on its monitor so frost always opens in the middle
    // of the screen. Native only: on the web the page decides where the
    // canvas sits.
    #[cfg(not(target_arch = "wasm32"))]
    center_on_screen(event_loop, &window);
    // winit (Wayland) only delivers RedrawRequested after a compositor
    // frame callback, so explicitly request the first frame; otherwise
    // the window is never mapped and nothing is ever drawn.
    window.request_redraw();
    window
}

/// Centers `window` on the monitor it sits on, so frost always opens in
/// the middle of the screen. Both the monitor's size and the window's
/// outer size (title bar and borders included) are in physical pixels, so
/// the position is computed in physical pixels too — no scale-factor
/// conversion. When the window is larger than the monitor, it is clamped
/// to the monitor's top-left.
#[cfg(not(target_arch = "wasm32"))]
fn center_on_screen(event_loop: &ActiveEventLoop, window: &Window) {
    let Some(monitor) = window
        .current_monitor()
        .or_else(|| event_loop.primary_monitor())
    else {
        return;
    };
    let screen = monitor.size();
    let frame = window.outer_size();
    let x = ((screen.width as i64 - frame.width as i64) / 2).max(0) as i32;
    let y = ((screen.height as i64 - frame.height as i64) / 2).max(0) as i32;
    window.set_outer_position(PhysicalPosition::new(x, y));
    log::info!(
        "window centered at ({x}, {y}) px on a {}x{} monitor",
        screen.width,
        screen.height
    );
}

impl<P: Process> ApplicationHandler for Frost<P> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }
        self.attach_window(create_window(event_loop, self.window_size));
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
                // Track the held physical keys so `Context::key_down` can
                // report them to the process on the next frame.
                if let PhysicalKey::Code(code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed => {
                            self.keys.insert(code);
                        }
                        ElementState::Released => {
                            self.keys.remove(&code);
                        }
                    }
                }
                if event.state == ElementState::Pressed && event.logical_key == NamedKey::Escape {
                    log::info!("escape pressed, exiting");
                    event_loop.exit();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                // `position` is already in physical pixels, the same space
                // the frame's `Canvas` works in; flip it to user coordinates
                // when building the `Context`.
                self.mouse = Some([position.x as f32, position.y as f32]);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                match state {
                    ElementState::Pressed => {
                        self.mouse_buttons.insert(button);
                    }
                    ElementState::Released => {
                        self.mouse_buttons.remove(&button);
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.mouse = None;
                // A release outside the window may never arrive, so drop
                // the held buttons rather than stick them.
                self.mouse_buttons.clear();
            }
            WindowEvent::Focused(false) => {
                // The window lost focus: key and button releases may never
                // arrive, so drop the held state rather than stick the
                // controls. The cursor position may likewise be stale.
                self.keys.clear();
                self.mouse = None;
                self.mouse_buttons.clear();
            }
            WindowEvent::CloseRequested => {
                log::info!("window close requested, exiting");
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                // `size` is the physical client size; store the logical size
                // so `pixel_size()` reproduces the true physical pixels.
                if size.width > 0 && size.height > 0 {
                    let scale = (self.scale as f64).max(0.01);
                    self.logical_size = (
                        (size.width as f64 / scale).max(1.0).round() as u32,
                        (size.height as f64 / scale).max(1.0).round() as u32,
                    );
                    self.resize();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor as f32;
                self.resize();
            }
            WindowEvent::RedrawRequested => {
                self.render();
                // winit only delivers RedrawRequested after we ask for it, so
                // request the next frame to keep animation running continuously.
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

/// Monotonic milliseconds since an arbitrary process-start epoch:
/// `std::time::Instant` on native, and the browser's `performance.now()`
/// on wasm — `Instant::now()` panics on wasm32-unknown-unknown, where the
/// clock has to come from the page. Only differences between two calls
/// matter (see `Frost::last_millis`), so the epochs may differ.
fn now_millis() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        // `Instant` has no absolute epoch, so pin one at the first call;
        // `elapsed` stays monotonic.
        static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        let epoch = *EPOCH.get_or_init(std::time::Instant::now);
        epoch.elapsed().as_secs_f64() * 1000.0
    }
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|window| window.performance())
            .map(|performance| performance.now())
            .unwrap_or(0.0)
    }
}
impl<P: Process> Frost<P> {
    /// Sizes the surface from `window`, builds the pipelines and renders
    /// the first frame.
    ///
    /// Called from [`Self::resumed`] once a window exists; on the web the
    /// window is created before the async GPU setup completes, and this runs
    /// when it does.
    pub(crate) fn attach_window(&mut self, window: Arc<Window>) {
        self.window_id = Some(window.id());
        // winit reports the client area in *physical* pixels, so convert it
        // to logical here; `pixel_size()` multiplies by the scale factor.
        let inner = window.inner_size();
        let scale = window.scale_factor();
        self.scale = scale as f32;
        self.logical_size = (
            (inner.width as f64 / scale).max(1.0).round() as u32,
            (inner.height as f64 / scale).max(1.0).round() as u32,
        );
        log::info!(
            "window created ({}x{} @ {:.2}x)",
            self.logical_size.0,
            self.logical_size.1,
            self.scale
        );

        // With vsync, presentation runs once per vertical blank, so the
        // expected frame rate is the refresh rate of the monitor the window
        // sits on, as winit reports it (unknown on some displays and in the
        // browser). Without vsync there is no expected rate at all.
        self.expected_fps = if self.vsync {
            window
                .current_monitor()
                .and_then(|monitor| monitor.refresh_rate_millihertz())
                .map(|millihertz| millihertz as f32 / 1000.0)
        } else {
            None
        };

        // An `Arc<Window>` is passed by value to `create_surface`, so the
        // resulting `Surface` is 'static without borrowing `self.window`. We
        // keep a clone of the `Arc` for per-frame `request_redraw`.
        let surface = self
            .instance
            .create_surface(window.clone())
            .expect("failed to create wgpu surface");
        // Keep the window for per-frame `request_redraw` (continuous frames).
        self.window = Some(window);
        let pixel_size = self.pixel_size();
        let mut config = surface
            .get_default_config(&self.adapter, pixel_size.0, pixel_size.1)
            .expect("no compatible surface format");
        // The default config picks the first supported presentation mode;
        // apply the user's vsync request explicitly.
        config.present_mode = present_mode_for(self.vsync);
        surface.configure(&self.device, &config);
        log::info!(
            "surface configured at {}x{}, format {:?}, present_mode {:?}",
            config.width,
            config.height,
            config.format,
            config.present_mode
        );

        self.set_up_pipelines(config.format);
        self.format = Some(config.format);
        self.surface = Some(surface);
        // Commit the first frame immediately so the compositor maps the window.
        self.render();
        // On the web, the page shows a "Loading frost…" placeholder until the
        // first frame lands; remove it now that it has.
        #[cfg(target_arch = "wasm32")]
        hide_fallback();
    }

    fn pixel_size(&self) -> (u32, u32) {
        (
            (self.logical_size.0 as f64 * self.scale as f64).max(1.0) as u32,
            (self.logical_size.1 as f64 * self.scale as f64).max(1.0) as u32,
        )
    }

    fn resize(&mut self) {
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let pixel_size = self.pixel_size();
        let mut config = surface
            .get_default_config(&self.adapter, pixel_size.0, pixel_size.1)
            .expect("no compatible surface format");
        config.present_mode = present_mode_for(self.vsync);
        surface.configure(&self.device, &config);
        log::info!(
            "window resized to {}x{} ({}x{} px)",
            self.logical_size.0,
            self.logical_size.1,
            config.width,
            config.height
        );
        // Pipelines depend only on the surface format, not the size, so they
        // are rebuilt only if the format actually changed.
        if self.format != Some(config.format) {
            self.set_up_pipelines(config.format);
            self.format = Some(config.format);
        }
    }

    /// Creates the shared line and circle pipelines. Uniform buffers and bind
    /// groups are created per draw call, see `primitive_uniform`.
    fn set_up_pipelines(&mut self, format: TextureFormat) {
        let device = &self.device;

        let line_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("line shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(LINE_SHADER)),
        });
        self.line_pipeline = Some(Self::create_pipeline(
            device,
            &line_module,
            format,
            "line pipeline",
        ));

        let circle_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("circle shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(CIRCLE_SHADER)),
        });
        self.circle_pipeline = Some(Self::create_pipeline(
            device,
            &circle_module,
            format,
            "circle pipeline",
        ));

        let rect_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("rectangle shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(RECT_SHADER)),
        });
        self.rect_pipeline = Some(Self::create_pipeline(
            device,
            &rect_module,
            format,
            "rectangle pipeline",
        ));

        let shape_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("shape shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(SHAPE_SHADER)),
        });
        self.shape_pipeline = Some(Self::create_pipeline(
            device,
            &shape_module,
            format,
            "shape pipeline",
        ));

        let sprite_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("sprite shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(SPRITE_SHADER)),
        });
        self.sprite_pipeline = Some(Self::create_pipeline(
            device,
            &sprite_module,
            format,
            "sprite pipeline",
        ));

        let particles_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("particle shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(PARTICLES_SHADER)),
        });
        self.particle_pipeline = Some(Self::create_pipeline(
            device,
            &particles_module,
            format,
            "particle pipeline",
        ));
    }

    /// Builds a render pipeline for a shader with no vertex buffers
    /// (full-screen triangles, or instanced quads generated in the vertex
    /// shader), one bind group at group 0, and a single alpha-blended color
    /// target.
    fn create_pipeline(
        device: &Device,
        module: &ShaderModule,
        format: TextureFormat,
        label: &str,
    ) -> RenderPipeline {
        device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some(label),
            layout: None,
            vertex: VertexState {
                module,
                entry_point: Some("vs_main"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(FragmentState {
                module,
                entry_point: Some("fs_main"),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }

    /// Creates the uniform buffer for one draw call plus its bind group.
    ///
    /// One buffer per draw call is required: `write_buffer` copies are flushed
    /// as a batch *before any draw executes*, so a buffer shared between draws
    /// would make every draw read the last-written parameters. The buffer and
    /// bind group may be dropped as soon as the command buffer is submitted;
    /// wgpu keeps them alive until the GPU is finished with them.
    fn primitive_uniform(
        &self,
        pipeline: &RenderPipeline,
        label: &str,
        data: &[u8],
    ) -> (Buffer, BindGroup) {
        let device = &self.device;
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: data.len() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, data);
        let layout = pipeline.get_bind_group_layout(0);
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some(label),
            layout: &layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: None,
                }),
            }],
        });
        (buffer, bind_group)
    }

    /// Creates the sprite's texture, view and sampler for one image.
    ///
    /// The pixels arrive as tightly packed RGBA8 bytes (one per texel), so
    /// the upload is a single `write_texture` of `width * height * 4` bytes
    /// with `bytes_per_row = width * 4`. The texture is created in
    /// `Rgba8UnormSrgb` so the sample lands in linear space and the
    /// sRGB blending state produces the same colors the file was authored
    /// in. The texture itself is kept alive by the view: `TextureView`
    /// holds a reference to its texture, so storing the view in
    /// `sprite_resources` is enough.
    fn sprite_texture(&self, data: &[u8], size: [f32; 2]) -> (TextureView, Sampler) {
        let width = (size[0] as u32).max(1);
        let height = (size[1] as u32).max(1);
        let texture = self.device.create_texture(&TextureDescriptor {
            label: Some("sprite texture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        self.queue.write_texture(
            TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            data,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: None,
            },
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let sampler = self.device.create_sampler(&SamplerDescriptor {
            label: Some("sprite sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: f32::MAX,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        });
        (view, sampler)
    }

    /// Creates the uniform buffer and five-entry bind group (uniform,
    /// texture view, sampler, light field, occluder field) for one sprite
    /// draw call. Same per-draw buffer rationale as
    /// [`Frost::primitive_uniform`].
    // The bind-group entries are deliberately flat: each takes the
    // per-draw resources it binds, and the frame fields grow with the
    // lighting features.
    #[allow(clippy::too_many_arguments)]
    fn sprite_uniform(
        &self,
        pipeline: &RenderPipeline,
        label: &str,
        view: &TextureView,
        sampler: &Sampler,
        data: &[u8],
        field_buffer: &Buffer,
        occluder_buffer: &Buffer,
    ) -> (Buffer, BindGroup) {
        let device = &self.device;
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: data.len() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, data);
        let layout = pipeline.get_bind_group_layout(0);
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some(label),
            layout: &layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(sampler),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: field_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: occluder_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        });
        (buffer, bind_group)
    }

    /// Creates the per-draw buffers and bind group for one particle batch:
    /// the batch's uniform (the surface size, the base color, and the shape
    /// kind and aspect) at binding 0, the packed per-particle instance data
    /// as a storage buffer read by the vertex stage at binding 1, the
    /// sampled texture and sampler — the batch's image for a sprite batch,
    /// the 1x1 placeholder for the SDF kinds — at bindings 2 and 3, the
    /// frame's light field at binding 4, and the frame's occluder field at
    /// binding 5. Same per-draw buffer rationale as
    /// [`Frost::primitive_uniform`].
    // The bind-group entries are deliberately flat: each takes the
    // per-draw resources it binds, and the frame fields grow with the
    // lighting features.
    #[allow(clippy::too_many_arguments)]
    fn particle_uniform(
        &self,
        pipeline: &RenderPipeline,
        data: &[u8],
        uniform_data: &[u8],
        view: &TextureView,
        sampler: &Sampler,
        field_buffer: &Buffer,
        occluder_buffer: &Buffer,
    ) -> (Buffer, Buffer, BindGroup) {
        let device = &self.device;
        let uniform_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("particle uniform buffer"),
            size: uniform_data.len() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&uniform_buffer, 0, uniform_data);
        let instance_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("particle instance buffer"),
            size: data.len() as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&instance_buffer, 0, data);
        let layout = pipeline.get_bind_group_layout(0);
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("particle bind group"),
            layout: &layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &uniform_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &instance_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(view),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Sampler(sampler),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: field_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: occluder_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        });
        (uniform_buffer, instance_buffer, bind_group)
    }

    /// The shape draw's per-draw uniform buffer and bind group: the uniform
    /// at binding 0, the frame's light field at binding 1, and the frame's
    /// occluder field at binding 2. Shapes are light receivers, so the
    /// fields are bound even for an unlit shape — the shader's `lit` guard
    /// lives in the uniform, not the binding.
    fn shape_uniform(
        &self,
        pipeline: &RenderPipeline,
        label: &str,
        data: &[u8],
        field_buffer: &Buffer,
        occluder_buffer: &Buffer,
    ) -> (Buffer, BindGroup) {
        let uniform_buffer = self.device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: data.len() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&uniform_buffer, 0, data);
        let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some(label),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &uniform_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: field_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: occluder_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        });
        (uniform_buffer, bind_group)
    }

    /// The frame's light field buffer, grown to hold the header plus
    /// `count` light records when the buffer is still too small. The
    /// buffer only ever grows: its size is the peak light count so far,
    /// and lighter frames reuse the bigger buffer, writing a shorter
    /// slice.
    fn field_buffer_for(&mut self, count: u32) -> Buffer {
        let size = (LIGHT_FIELD_HEADER + count as usize * LIGHT_RECORD) as u64;
        if size > self.field_buffer.size() {
            self.field_buffer = self.device.create_buffer(&BufferDescriptor {
                label: Some("light field buffer"),
                size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.field_buffer.clone()
    }

    /// The frame's occluder field buffer, grown to hold the header plus
    /// `count` occluder records when the buffer is still too small. Same
    /// grow-only rationale as [`Frost::field_buffer_for`].
    fn occluder_buffer_for(&mut self, count: u32) -> Buffer {
        let size = (OCCLUDER_FIELD_HEADER + count as usize * OCCLUDER_RECORD) as u64;
        if size > self.occluder_buffer.size() {
            self.occluder_buffer = self.device.create_buffer(&BufferDescriptor {
                label: Some("occluder field buffer"),
                size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.occluder_buffer.clone()
    }

    fn render(&mut self) {
        let (output, reconfigure) = self.acquire_frame();
        if reconfigure {
            self.resize();
        }
        let Some(output) = output else {
            return;
        };
        log::trace!("render: acquired surface texture, submitting frame");

        // Let the user update the scene and draw this frame, in their
        // coordinate system.
        let pixel_size = self.pixel_size();
        let mut canvas = Canvas::new(pixel_size);
        let now = now_millis();
        let dt = self
            .last_millis
            .map(|last| ((now - last).max(0.0) / 1000.0).min(1.0) as f32)
            .unwrap_or(0.0);
        self.last_millis = Some(now);
        // Flip the cursor from window (upper-left, y-down) to user
        // coordinates: origin at the window's center, y up.
        let mouse = self.mouse.map(|[mx, my]| {
            [
                mx - pixel_size.0 as f32 / 2.0,
                pixel_size.1 as f32 / 2.0 - my,
            ]
        });
        let process = &mut self.process;
        let scene = &mut self.scene;
        let keys = &self.keys;
        let mouse_buttons = &self.mouse_buttons;
        {
            let mut ctx = Context {
                canvas: &mut canvas,
                scene,
                keys,
                expected_fps: self.expected_fps,
                mouse,
                mouse_buttons,
            };
            process.process(&mut ctx, dt);
        }
        // The user's process ran; now update the scene tree itself: every
        // node's `Node::process`, children before their parent.
        self.scene.visit();
        // The scene was just updated; draw it into the frame's draw list.
        canvas.draw_scene(&self.scene);
        // Expand the text into per-glyph sprite quads before the sort, so
        // each glyph keeps its node's position in the paint order.
        canvas.expand_text(&mut self.text_atlases);

        // Paint order: the draw groups (the base group and the scene's
        // layers) by ascending layer order, higher order on top; within each
        // group, ascending z, lower z behind. The sorts are stable, so
        // draws with equal keys keep call order and the last drawn is on
        // top.
        let draws = canvas.paint_order();

        // The frame's light field: the scene's lights packed in call order
        // with the scene's ambient. The field buffer is grow-only — it is
        // re-created only when this frame's light count beats the peak it
        // was last sized for.
        let field = pack_light_field(&draws, self.scene.ambient);
        let field_buffer = self.field_buffer_for(field.count);
        self.queue.write_buffer(&field_buffer, 0, &field.data);

        // The frame's occluder field: the flagged rectangle shapes packed
        // in call order. Same grow-only buffer as the light field.
        let occluders = pack_occluder_field(&draws);
        let occluder_buffer = self.occluder_buffer_for(occluders.count);
        self.queue.write_buffer(&occluder_buffer, 0, &occluders.data);

        let (
            Some(line_pipeline),
            Some(circle_pipeline),
            Some(rect_pipeline),
            Some(shape_pipeline),
            Some(sprite_pipeline),
            Some(particle_pipeline),
        ) = (
            self.line_pipeline.as_ref(),
            self.circle_pipeline.as_ref(),
            self.rect_pipeline.as_ref(),
            self.shape_pipeline.as_ref(),
            self.sprite_pipeline.as_ref(),
            self.particle_pipeline.as_ref(),
        ) else {
            return;
        };
        let Some(particle_index_buffer) = &self.particle_index_buffer else {
            return;
        };

        // The frame's clear color: the last background node in paint order,
        // or the default when the frame has none.
        let clear = clear_color(&draws);

        let view = output
            .texture
            .create_view(&TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear.r as f64,
                            g: clear.g as f64,
                            b: clear.b as f64,
                            a: clear.a as f64,
                        }),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // One pass for the whole frame. The background is cleared once
            // for the full surface, then each draw sets the scissor to its
            // tight bounding box, so fragments outside it are discarded and
            // the fragment shader only runs over the pixels the object can
            // write. The viewport is left at the full surface, so the
            // shaders' pixel coordinates stay absolute.
            let render_area = [output.texture.width(), output.texture.height()];
            for draw in draws {
                let Some([x, y, w, h]) = draw.scissor_rect(render_area) else {
                    // Fully outside the surface, or a background (which
                    // became the clear color); nothing to draw.
                    continue;
                };
                pass.set_scissor_rect(x, y, w, h);
                // A fresh uniform buffer (and bind group) per draw call, since
                // all write_buffer copies complete before any draw executes.
                match draw {
                    Draw::Line {
                        a, b, width, color, ..
                    } => {
                        let (_buffer, bind_group) = self.primitive_uniform(
                            line_pipeline,
                            "line uniforms",
                            &line_uniform_data(a, b, color, width),
                        );
                        pass.set_pipeline(line_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Circle {
                        center,
                        radius,
                        color,
                        ..
                    } => {
                        let (_buffer, bind_group) = self.primitive_uniform(
                            circle_pipeline,
                            "circle uniforms",
                            &circle_uniform_data(center, color, radius),
                        );
                        pass.set_pipeline(circle_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Rectangle {
                        center,
                        extent,
                        color,
                        ..
                    } => {
                        let (_buffer, bind_group) = self.primitive_uniform(
                            rect_pipeline,
                            "rectangle uniforms",
                            &rect_uniform_data(center, extent, color),
                        );
                        pass.set_pipeline(rect_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Shape {
                        world,
                        center,
                        params,
                        kind,
                        aa,
                        color,
                        lit,
                        ..
                    } => {
                        let Some(inv) = world.invert() else {
                            // Degenerate transform; the shape collapses to a
                            // line or a point and its inverse does not exist.
                            continue;
                        };
                        let (_buffer, bind_group) = self.shape_uniform(
                            shape_pipeline,
                            "shape uniforms",
                            &shape_uniform_data(inv, center, params, kind, aa, color, lit),
                            &field_buffer,
                            &occluder_buffer,
                        );
                        pass.set_pipeline(shape_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Sprite {
                        world,
                        data,
                        size,
                        texture_size,
                        tint,
                        alpha,
                        lit,
                        uv_rect,
                        ..
                    } => {
                        let Some(inv) = world.invert() else {
                            // Degenerate transform; the sprite collapses to a
                            // line or a point and its inverse does not exist.
                            continue;
                        };
                        // Look up the GPU resources for this image, creating
                        // them on first use. Two sprites from the same file
                        // share one texture, so the image is uploaded once
                        // per file; glyph quads from the same atlas share
                        // one atlas texture the same way.
                        let key = Arc::as_ptr(&data) as *const ();
                        let (view, sampler) = match self.sprite_resources.get(&key) {
                            Some((view, sampler)) => (view.clone(), sampler.clone()),
                            None => {
                                let (view, sampler) =
                                    self.sprite_texture(&data, [texture_size[0] as f32, texture_size[1] as f32]);
                                self.sprite_resources
                                    .insert(key, (view.clone(), sampler.clone()));
                                (view, sampler)
                            }
                        };
                        let (_buffer, bind_group) = self.sprite_uniform(
                            sprite_pipeline,
                            "sprite uniforms",
                            &view,
                            &sampler,
                            &sprite_uniform_data(inv, size, tint, alpha, lit, uv_rect),
                            &field_buffer,
                            &occluder_buffer,
                        );
                        pass.set_pipeline(sprite_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Particles {
                        data,
                        count,
                        color,
                        kind,
                        aspect,
                        sprite_data,
                        sprite_size,
                        lit,
                        ..
                    } => {
                        if count == 0 {
                            continue;
                        }
                        // The sampled texture: for a sprite batch, the
                        // batch's image — shared with the sprite pipeline's
                        // cache, so a particle image and a sprite from the
                        // same file upload once — and for the SDF kinds,
                        // which never sample, the 1x1 placeholder.
                        let (view, sampler) =
                            match sprite_data.filter(|_| kind >= 1.5) {
                                Some(image) => {
                                    let key = Arc::as_ptr(&image) as *const ();
                                    match self.sprite_resources.get(&key) {
                                        Some((view, sampler)) => {
                                            (view.clone(), sampler.clone())
                                        }
                                        None => {
                                            let (view, sampler) =
                                                self.sprite_texture(&image, [
                                                    sprite_size[0] as f32,
                                                    sprite_size[1] as f32,
                                                ]);
                                            self.sprite_resources
                                                .insert(key, (view.clone(), sampler.clone()));
                                            (view, sampler)
                                        }
                                    }
                                }
                                None => self.particle_placeholder.clone(),
                            };
                        // The whole batch is one instanced draw: the shared
                        // index buffer expands every instance into a tight
                        // quad, and the fragment stage evaluates the batch's
                        // shape per pixel. The batch's uniform and instance
                        // data get fresh per-draw buffers, the same
                        // rationale as `primitive_uniform`.
                        let uniform_data = particles_uniform_data(
                            [render_area[0] as f32, render_area[1] as f32],
                            color,
                            kind,
                            aspect,
                            lit,
                        );
                        let (_uniform, _instances, bind_group) = self.particle_uniform(
                            particle_pipeline,
                            &data,
                            &uniform_data,
                            &view,
                            &sampler,
                            &field_buffer,
                            &occluder_buffer,
                        );
                        pass.set_pipeline(particle_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        pass.set_index_buffer(
                            particle_index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..6, 0, 0..count);
                    }
                    // A background's scissor rect is `None`, so it continued
                    // above; this arm keeps the match exhaustive.
                    Draw::Background { .. } => {}
                    // A light's scissor rect is `None`, so it continued
                    // above; this arm keeps the match exhaustive.
                    Draw::Light { .. } => {}
                    // Text is expanded into glyph sprites before the render
                    // loop, so it never reaches the match; this arm keeps it
                    // exhaustive.
                    Draw::Text { .. } => {}
                }
            }
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(output);
        log::trace!("render: frame presented");
    }

    /// Acquires the current surface texture.
    ///
    /// Returns the texture and whether the surface should be reconfigured
    /// because it is only suboptimal.
    fn acquire_frame(&self) -> (Option<SurfaceTexture>, bool) {
        let Some(surface) = self.surface.as_ref() else {
            return (None, false);
        };
        match surface.get_current_texture() {
            CurrentSurfaceTexture::Success(tex) => (Some(tex), false),
            CurrentSurfaceTexture::Suboptimal(tex) => {
                log::info!("suboptimal surface texture, reconfiguring");
                (Some(tex), true)
            }
            CurrentSurfaceTexture::Timeout => {
                log::info!("surface timeout, skipping frame");
                (None, false)
            }
            _ => {
                log::info!("no surface texture available, skipping frame");
                (None, false)
            }
        }
    }
}

/// Drives `future` on this thread until it completes.
///
/// Native only: in the browser the main thread cannot block on a future,
/// because the browser only resolves it once the thread is free. See
/// [`run`].
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
    let mut cx = TaskContext::from_waker(Waker::noop());
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
}
