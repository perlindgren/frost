use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

#[cfg(not(target_arch = "wasm32"))]
use std::future::Future;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::task::{Context as TaskContext, Poll, Waker};

use gilrs::{EventType, Gilrs};
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
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::backend::frame::*;
#[cfg(target_arch = "wasm32")]
use crate::backend::wasm::hide_fallback;
use crate::objects::*;
use crate::shaders::*;
use crate::text;
use crate::{Canvas, Context, KeyCode, Process};

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

/// The offscreen render target of fixed render size mode (see
/// `Config::render_size`): a texture of the requested size that the frame
/// renders into, paired with its blit bind group (the texture as shader
/// source, the bilinear sampler that stretches it, and the letterbox fit
/// uniform), so the stretch pass is a `set_bind_group` and three
/// vertices. The blit samples the surface format directly, so the stretch
/// never re-encodes colors.
struct BlitTarget {
    /// The size this target was built for; a surface of exactly this size
    /// renders to directly and leaves the target unused.
    size: [u32; 2],
    /// The texture behind `view`: nothing reads it after construction (the
    /// view and bind group keep the GPU resource alive on their own), but
    /// holding it makes the target's ownership obvious.
    _texture: wgpu::Texture,
    view: TextureView,
    /// The 16-byte letterbox fit (`vec2` + padding), rewritten by `render`
    /// whenever the surface's aspect may have moved.
    fit_buffer: Buffer,
    bind_group: BindGroup,
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
    /// The `Config`'s initial inner size in physical pixels; used once in
    /// `create_window`, and it wins over `window_size` there.
    window_size_px: Option<[u32; 2]>,
    /// The `Config`'s resizability request; applied once in
    /// `attach_window`.
    resizable: bool,
    /// The `Config`'s fixed render size in physical pixels, or `None` to
    /// render at the window's own resolution (see `Config::render_size`).
    render_size: Option<[u32; 2]>,
    #[allow(dead_code)]
    window_id: Option<WindowId>,
    /// The winit window (shared), kept so we can call `request_redraw` for
    /// continuous per-frame rendering.
    window: Option<Arc<Window>>,
    logical_size: (u32, u32),
    /// The client size seen by the last `Resized` event, physical pixels;
    /// the baseline [`Self::snap_aspect`] measures the drag against.
    /// `(0, 0)` until the first resize lands.
    last_resized_px: (u32, u32),
    scale: f32,
    surface: Option<Surface<'static>>,
    line_pipeline: Option<RenderPipeline>,
    polyline_pipeline: Option<RenderPipeline>,
    circle_pipeline: Option<RenderPipeline>,
    rect_pipeline: Option<RenderPipeline>,
    shape_pipeline: Option<RenderPipeline>,
    sprite_pipeline: Option<RenderPipeline>,
    particle_pipeline: Option<RenderPipeline>,
    /// Stretches the fixed render size buffer over the window (see
    /// `Config::render_size`); unused when the frame renders at the
    /// window's own resolution.
    blit_pipeline: Option<RenderPipeline>,
    /// The offscreen render target for fixed render size mode, built on the
    /// first frame that needs one: a surface already exactly the render
    /// size skips both the buffer and the stretch pass.
    blit_target: Option<BlitTarget>,
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
    /// 48-byte record per light at the peak count so far. It starts at
    /// the buffer's minimum size — the header plus one vec4, the WGSL
    /// layout's minimum binding size — because a lightless frame still
    /// binds it (the shaders read the field for the ambient), and it only
    /// ever grows: the WGSL field's tail is an unsized array sized by the
    /// bound buffer, so a bigger buffer simply holds more records, and a
    /// lighter frame writes a shorter slice into the same buffer.
    field_buffer: Buffer,
    /// The frame's occluder field storage buffer: a 16-byte header plus one
    /// 48-byte record per occluder at the peak count so far. Same
    /// grow-only rationale as [`Frost::field_buffer`]; a frame with no
    /// occluders still binds it (the shaders read the field's count), and
    /// the WGSL field's tail is an unsized array sized by the bound buffer.
    occluder_buffer: Buffer,
    /// The GPU resources for each distinct sprite image, keyed by the
    /// `(pointer, generation)` of its pixel-data `Arc`: the pointer alone
    /// is not enough, because a freed buffer's address can be reused for a
    /// different buffer (an atlas repack, a rebuilt in-memory image).
    /// Sprite constructors stamp a new generation on every construction,
    /// and atlases bump theirs on a repack, so a stale texture is never
    /// served for new pixels. Sprites sharing one file share one texture,
    /// so the map stays bounded by the number of distinct images. A
    /// `TextureView` keeps its texture alive, so only the view and the
    /// sampler are stored. When an atlas repacks, its stale entry is
    /// evicted right after the text is expanded; a sprite buffer's stale
    /// entry is evicted lazily, when a new buffer recycles its address.
    sprite_resources: HashMap<(u64, u64), (TextureView, Sampler)>,
    /// The rasterized glyph atlas for each distinct `(font, size, weight)`
    /// triple, keyed by the font buffer's pointer, the size's bits and the
    /// weight's bits. Kept between
    /// frames so unchanged text never re-rasterizes and its pixel buffer —
    /// and therefore the GPU texture in `sprite_resources` — keeps a stable
    /// identity. A repack (a newly packed glyph) replaces the buffer and
    /// bumps its generation; the stale texture is evicted from
    /// `sprite_resources` right after the text is expanded.
    text_atlases: HashMap<(u64, u32, u32), text::Atlas>,
    /// The surface format the current pipelines were built for; they are only
    /// rebuilt when this changes.
    format: Option<TextureFormat>,
    /// Millisecond timestamp of the previous rendered frame, used to
    /// compute `dt` (see `now_millis`).
    last_millis: Option<f64>,
    /// The CPU time in milliseconds the engine spent processing the last
    /// completed frame: from just after the frame's surface acquire (the
    /// demo's `process` callback, the scene update, the draw list, and the
    /// command encoding included) to the submission of the frame's command
    /// buffer to the queue — the surface acquire, the present handoff, and
    /// the wait for the next frame excluded. `0.0` before the first frame
    /// completes. Reported to the next frame's `Context` (see
    /// `Context::frame_processing_ms`).
    frame_processing_ms: f64,
    /// The GPU draw calls the engine issued for the last completed frame:
    /// one per `pass.draw` / `pass.draw_indexed` that actually executed —
    /// draws skipped as off-screen, background, light, or empty (a
    /// degenerate transform, an empty particle batch) are not counted.
    /// `0` before the first frame completes. Reported to the next frame's
    /// `Context` (see `Context::frame_draw_calls`).
    frame_draw_calls: u32,
    /// The GPU draw calls the last completed frame spent on the
    /// `Diagnostics` overlay's own nodes: the tagged subset of
    /// `frame_draw_calls` (see `Context::frame_diagnostic_draw_calls`).
    frame_diagnostic_draw_calls: u32,
    /// The expected frame rate in Hz: the refresh rate of the monitor the
    /// window sits on, as winit reports it. `None` when vsync is off
    /// (presentation is uncapped) or the rate is unknown; see
    /// `Context::expected_fps`.
    expected_fps: Option<f32>,
    /// The scene drawn every frame, after the process runs.
    scene: Scene,
    /// The physical keys currently held down, updated as keyboard events arrive.
    keys: HashSet<KeyCode>,
    /// The character each held physical key typed as it went down, under
    /// the user's current keyboard layout — `Context::char_down` reads it.
    /// Keyed by the physical code, so a release removes exactly what its
    /// press recorded no matter which modifiers moved in between, and the
    /// character a shortcut matches never changes while the key is held.
    typed: HashMap<KeyCode, char>,
    /// The mouse cursor's position in physical pixels, `(0, 0)` at the
    /// window's upper-left corner, or `None` when the cursor is outside the
    /// window. Updated as cursor events arrive; `render` maps it into the
    /// frame's user space (logical pixels, or the fixed render size) on
    /// every frame.
    mouse: Option<[f32; 2]>,
    /// The mouse buttons currently held down, updated as mouse input
    /// events arrive.
    mouse_buttons: HashSet<MouseButton>,
    /// The mouse wheel's vertical movement in lines, accumulated as wheel
    /// events arrive since the previous frame and reset after the frame's
    /// `Context` reports it (see `Context::mouse_wheel`).
    mouse_wheel: f32,
    /// The gamepad controller, or `None` when gilrs could not open the
    /// platform's input devices (then `Context::gamepads` is empty). The
    /// buttons and axes of every connected gamepad are tracked by gilrs
    /// itself and refreshed as its events are drained in `about_to_wait`.
    gilrs: Option<Gilrs>,
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
        window_size_px: Option<[u32; 2]>,
        resizable: bool,
        render_size: Option<[u32; 2]>,
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

        // The light field buffer starts at its minimum size — the header
        // plus one vec4, the WGSL layout's minimum binding size: even a
        // frame with no lights binds it (the shaders read the field to
        // fetch the ambient), and it only grows when the light count peaks
        // higher.
        let field_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("light field buffer"),
            size: LIGHT_FIELD_BUFFER_MIN as u64,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // The occluder field buffer starts at its minimum size, the same
        // WGSL minimum binding size as the light field buffer: even a
        // frame with no occluders binds it (the shaders read the field's
        // count), and it only grows when the occluder count peaks higher.
        let occluder_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("occluder field buffer"),
            size: OCCLUDER_FIELD_BUFFER_MIN as u64,
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

        // The gamepad controller: gilrs opens the platform's input devices
        // and connects every gamepad already present. When it cannot open
        // them (no access to the devices), the app runs without gamepad
        // support rather than failing to start.
        let gilrs = match Gilrs::new() {
            Ok(gilrs) => {
                log::info!(
                    "gamepads: {} connected at startup",
                    gilrs.gamepads().count()
                );
                Some(gilrs)
            }
            Err(err) => {
                log::warn!("gamepad support unavailable: {err}");
                None
            }
        };

        Self {
            instance,
            adapter,
            device,
            queue,
            vsync,
            window_size,
            window_size_px,
            resizable,
            render_size,
            window_id: None,
            window: None,
            logical_size: (0, 0),
            last_resized_px: (0, 0),
            scale: 1.0,
            surface: None,
            line_pipeline: None,
            polyline_pipeline: None,
            circle_pipeline: None,
            rect_pipeline: None,
            shape_pipeline: None,
            sprite_pipeline: None,
            particle_pipeline: None,
            blit_pipeline: None,
            blit_target: None,
            particle_index_buffer: Some(particle_index_buffer),
            particle_placeholder,
            field_buffer,
            occluder_buffer,
            sprite_resources: HashMap::new(),
            text_atlases: HashMap::new(),
            format: None,
            last_millis: None,
            frame_processing_ms: 0.0,
            frame_draw_calls: 0,
            frame_diagnostic_draw_calls: 0,
            expected_fps: None,
            scene,
            keys: HashSet::new(),
            typed: HashMap::new(),
            mouse: None,
            mouse_buttons: HashSet::new(),
            mouse_wheel: 0.0,
            gilrs,
            process,
        }
    }
}

/// Creates the frost window and requests its first frame.
///
/// `window_size` is the `Config`'s initial inner size in logical pixels
/// and `window_size_px` its size in physical pixels; a set physical size
/// wins, and winit converts it through the monitor's scale factor, so the
/// requested panel-pixel window opens at the same physical size on every
/// display — including Retina Macs, where a logical request would double.
/// `None` for both keeps the platform default. An oversized request is
/// clamped into the primary monitor, aspect preserved (see
/// [`fit_into_monitor`]). On the web, winit's canvas
/// is neither appended to the page nor sized by default: without the
/// append it is invisible, and without a size it stays the browser's
/// 300x150 default — so an unset size falls back to 900x600 there, and
/// the physical request is ignored (the canvas lives in CSS pixels).
pub(crate) fn create_window(
    event_loop: &ActiveEventLoop,
    window_size: Option<[u32; 2]>,
    window_size_px: Option<[u32; 2]>,
) -> Arc<Window> {
    let mut attributes = Window::default_attributes().with_title("frost");
    #[cfg(not(target_arch = "wasm32"))]
    {
        let monitor = event_loop.primary_monitor();
        if let Some([width, height]) = window_size_px {
            // Physical request: compare against the monitor in physical
            // pixels, and hand back a clamped physical size.
            let size = match monitor.as_ref() {
                Some(m) => fit_into_monitor([width, height], [m.size().width, m.size().height]),
                None => [width, height],
            };
            attributes = attributes.with_inner_size(PhysicalSize::new(size[0], size[1]));
        } else if let Some([width, height]) = window_size {
            // Logical request: the monitor's own scale factor turns its
            // physical size into the logical budget to fit into.
            let size = match monitor.as_ref() {
                Some(m) => {
                    let s = m.scale_factor().max(0.01);
                    fit_into_monitor(
                        [width, height],
                        [
                            (m.size().width as f64 / s).round().max(1.0) as u32,
                            (m.size().height as f64 / s).round().max(1.0) as u32,
                        ],
                    )
                }
                None => [width, height],
            };
            attributes = attributes.with_inner_size(LogicalSize::new(size[0], size[1]));
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::WindowAttributesExtWebSys;

        let _ = window_size_px;
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

/// Fits a requested opening size into the space available on the primary
/// monitor, aspect preserved: a request that fits passes through
/// untouched, an oversized one shrinks to the largest fitting size of its
/// own aspect — a window that opens whole and centered beats one that
/// hangs off the screen, and squashing the shape is not the same as
/// making it smaller. `size` and `avail` are in the same unit (physical
/// or logical, as the caller chose); with no monitor info the request
/// passes through.
#[cfg(not(target_arch = "wasm32"))]
fn fit_into_monitor(size: [u32; 2], avail: [u32; 2]) -> [u32; 2] {
    let (req_w, req_h) = (size[0] as f64, size[1] as f64);
    let (max_w, max_h) = (avail[0].max(1) as f64, avail[1].max(1) as f64);
    if req_w <= max_w && req_h <= max_h {
        return size;
    }
    let factor = (max_w / req_w).min(max_h / req_h);
    [
        (req_w * factor).round().max(1.0) as u32,
        (req_h * factor).round().max(1.0) as u32,
    ]
}

/// Packs a letterbox fit into the blit uniform's 16 bytes: two floats of
/// extent followed by the struct's padding.
fn fit_bytes(fit: (f32, f32)) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[0..4].copy_from_slice(&fit.0.to_le_bytes());
    bytes[4..8].copy_from_slice(&fit.1.to_le_bytes());
    bytes
}

/// The fraction `(width, height)` of a `surface` that a fixed
/// `render_size` buffer fills when its aspect is preserved and the result
/// is centered: `(1.0, 1.0)` when there is no fixed buffer or the aspects
/// match (the picture covers the window), and a letterbox fraction
/// otherwise — `(1.0, 0.5625)` for a 16:9 buffer on a square surface,
/// bars top and bottom included. The blit pass scales its triangle by
/// exactly this, and the mouse mapping divides it back out.
pub(crate) fn content_fit(render_size: Option<[u32; 2]>, surface: (u32, u32)) -> (f32, f32) {
    let Some([bw, bh]) = render_size else {
        return (1.0, 1.0);
    };
    let (sw, sh) = (surface.0 as f32, surface.1 as f32);
    if sw <= 0.0 || sh <= 0.0 {
        return (1.0, 1.0);
    }
    if (bw as f64) * (sh as f64) > (bh as f64) * (sw as f64) {
        // The buffer is wider than the surface: width-limited, with
        // letterbox bars above and below.
        (
            1.0,
            (bh as f64 * sw as f64 / (bw as f64 * sh as f64)) as f32,
        )
    } else {
        // Taller (or exactly as wide): pillarbox bars left and right.
        (
            (bw as f64 * sh as f64 / (bh as f64 * sw as f64)) as f32,
            1.0,
        )
    }
}

impl<P: Process> ApplicationHandler for Frost<P> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }
        self.attach_window(create_window(
            event_loop,
            self.window_size,
            self.window_size_px,
        ));
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Drain the gamepad events accumulated since the last poll, so the
        // state gilrs keeps for its connected gamepads is current before
        // the loop waits for new events. gilrs applies every input event
        // to that state itself; only the connect and disconnect
        // transitions need a log line here.
        if let Some(gilrs) = &mut self.gilrs {
            while let Some(event) = gilrs.next_event() {
                match event.event {
                    EventType::Connected => log::info!("gamepad {:?} connected", event.id),
                    EventType::Disconnected => {
                        log::info!("gamepad {:?} disconnected", event.id)
                    }
                    _ => {}
                }
            }
        }
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
                            self.typed.remove(&code);
                        }
                    }
                    // Record what a press typed so shortcuts can follow the
                    // character of the user's layout instead of the
                    // US-anchored physical position: the Swedish `+` key,
                    // for one, reports the physical code of the US `-` key.
                    // Only the first character of the press is kept —
                    // shortcuts are single characters, and dead-key or IME
                    // sequences start with the one the keycap shows.
                    if event.state == ElementState::Pressed
                        && let winit::keyboard::Key::Character(text) = &event.logical_key
                        && let Some(ch) = text.chars().next()
                    {
                        self.typed.insert(code, ch);
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
            WindowEvent::MouseInput { state, button, .. } => match state {
                ElementState::Pressed => {
                    self.mouse_buttons.insert(button);
                }
                ElementState::Released => {
                    self.mouse_buttons.remove(&button);
                }
            },
            WindowEvent::MouseWheel { delta, .. } => {
                // Accumulate the vertical movement in wheel lines:
                // `LineDelta` is already in lines, and `PixelDelta` is
                // converted at the nominal 120 pixels per line. The frame's
                // `Context` reports the accumulated delta and resets it, so
                // several notches in one frame add up and an unread frame
                // discards them.
                match delta {
                    MouseScrollDelta::LineDelta(_, y) => self.mouse_wheel += y,
                    MouseScrollDelta::PixelDelta(position) => {
                        self.mouse_wheel += position.y as f32 / 120.0
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
                self.typed.clear();
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
                    self.snap_aspect(size);
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
/// matter (see `Frost::last_millis`), so the epochs may differ. Shared
/// with the diagnostics overlay, which times its own per-frame update
/// cost with the same clock (see `Diagnostics`).
pub(crate) fn now_millis() -> f64 {
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

        // The resizability request. The style hint is enough on the
        // well-behaved platforms; XFCE ignores it (per winit), so a
        // non-resizable window also pins its size with hard min/max
        // bounds. No-op on the web, where the page owns the canvas size.
        if !self.resizable {
            window.set_resizable(false);
            window.set_min_inner_size(Some(inner));
            window.set_max_inner_size(Some(inner));
        }

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

    /// The engine-side aspect lock of fixed render size mode (see
    /// `Config::render_size`): a window dragged to a shape the buffer
    /// does not fill answers with a request for the corrected size, so
    /// the corner slides along the diagonal instead of opening letterbox
    /// bars. winit 0.30 exposes no platform aspect hint (its X11 hint
    /// field exists but nothing fills it, and the other platforms have no
    /// code path at all), so the lock rides on the resize events
    /// themselves: the corrected request re-enters the handler within
    /// tolerance and the loop stops, while a compositor that declines the
    /// request (Wayland may, fullscreen does) simply keeps the letterbox
    /// the blit already draws. Maximizing is the user asking for a
    /// specific rectangle, so it goes unfought.
    fn snap_aspect(&mut self, size: PhysicalSize<u32>) {
        let prev = std::mem::replace(&mut self.last_resized_px, (size.width, size.height));
        let Some(render) = self.render_size else {
            return;
        };
        if !self.resizable {
            // A pinned window keeps the shape it opened with; the guard
            // matters because a fixed-size window can still be resized
            // programmatically, and that should letterbox, not fight.
            return;
        }
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if window.is_maximized() {
            return;
        }
        let (w, h) = (size.width as f64, size.height as f64);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let aspect = render[0] as f64 / render[1] as f64;
        // The correction keeps the axis the user moved: an edge drag
        // adjusts the other dimension along the diagonal, and the
        // dragged one tracks the cursor. With no baseline (the first
        // event) or a balanced corner drag, fit inside the requested
        // rectangle rather than growing past it.
        let dw = size.width.abs_diff(prev.0) as f64;
        let dh = size.height.abs_diff(prev.1) as f64;
        let (w2, h2) = if prev == (0, 0) || (dw - dh).abs() < 2.0 {
            if w / h > aspect {
                (h * aspect, h)
            } else {
                (w, w / aspect)
            }
        } else if dw > dh {
            (w, w / aspect)
        } else {
            (h * aspect, h)
        };
        let (w2, h2) = (w2.round().max(1.0) as u32, h2.round().max(1.0) as u32);
        // Only correct a visibly wrong shape: the corrected request comes
        // back within the tolerance, and the conversation ends.
        if w2.abs_diff(size.width) < 2 && h2.abs_diff(size.height) < 2 {
            return;
        }
        log::debug!("aspect lock: {w}x{h} → {w2}x{h2}");
        // The return value reports a size applied immediately on the
        // platforms that do; every platform that applies the request
        // (and the ones that only deliver it, or decline it outright,
        // leaving the letterbox to cope) reports through `Resized`,
        // which is where our own state follows.
        let _ = window.request_inner_size(PhysicalSize::new(w2, h2));
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
            true,
        ));

        let polyline_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("polyline shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(POLYLINE_SHADER)),
        });
        self.polyline_pipeline = Some(Self::create_pipeline(
            device,
            &polyline_module,
            format,
            "polyline pipeline",
            true,
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
            true,
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
            true,
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
            true,
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
            true,
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
            true,
        ));

        // The fixed render size stretch: a full-screen sample of the
        // render target onto the surface. It replaces the surface pixel
        // instead of blending onto it — the buffer is the frame, not a
        // layer over it.
        let blit_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("blit shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(BLIT_SHADER)),
        });
        self.blit_pipeline = Some(Self::create_pipeline(
            device,
            &blit_module,
            format,
            "blit pipeline",
            false,
        ));
    }

    /// Builds a render pipeline for a shader with no vertex buffers
    /// (full-screen triangles, or instanced quads generated in the vertex
    /// shader), one bind group at group 0, and a single color target —
    /// alpha-blended when `blend` (every draw of the scene itself),
    /// replaced otherwise (the fixed-render-size stretch, whose source is
    /// the finished frame).
    fn create_pipeline(
        device: &Device,
        module: &ShaderModule,
        format: TextureFormat,
        label: &str,
        blend: bool,
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
                    blend: blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
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

    /// Builds the fixed render size buffer: a texture of exactly `size`
    /// pixels in the surface's own format — so the blit's samples need no
    /// re-encoding — usable as both a render attachment and a sampled
    /// texture, with its bind group pairing the view and a bilinear
    /// sampler. Built once per size change, which for a fixed size means
    /// once (the target survives resizes; only the requested size rebuilds
    /// it), and dropped whenever the surface itself becomes that size.
    fn new_blit_target(&mut self, size: [u32; 2]) -> BlitTarget {
        let format = self
            .format
            .expect("a surface is attached before the first frame renders");
        let texture = self.device.create_texture(&TextureDescriptor {
            label: Some("render target"),
            size: Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        let sampler = self.device.create_sampler(&SamplerDescriptor {
            label: Some("blit sampler"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        // The letterbox fit uniform, seeded to fill the window; `render`
        // rewrites it every frame. A zero-initialized buffer would draw the
        // triangle at zero size, so the (1, 1) seed matters for the first
        // frame if it ever ran before the per-frame write.
        let fit_buffer = self.device.create_buffer(&BufferDescriptor {
            label: Some("blit fit"),
            size: 16,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue
            .write_buffer(&fit_buffer, 0, &fit_bytes((1.0, 1.0)));
        let bind_group = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("blit bind group"),
            layout: &self
                .blit_pipeline
                .as_ref()
                .expect("the blit pipeline is built with the surface")
                .get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&sampler),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &fit_buffer,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        });
        log::info!(
            "fixed render size: frames render to a {}x{} buffer, stretched to the window",
            size[0],
            size[1]
        );
        BlitTarget {
            size,
            _texture: texture,
            view,
            fit_buffer,
            bind_group,
        }
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
    fn sprite_texture(
        &self,
        data: &[u8],
        size: [f32; 2],
        filter: SpriteFilter,
    ) -> (TextureView, Sampler) {
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
        let filter_mode = match filter {
            SpriteFilter::Linear => FilterMode::Linear,
            SpriteFilter::Nearest => FilterMode::Nearest,
        };
        let sampler = self.device.create_sampler(&SamplerDescriptor {
            label: Some("sprite sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            // The texture's own sampling filter: bilinear for photographic
            // sprites, nearest-neighbor for pixel-art-style ones.
            mag_filter: filter_mode,
            min_filter: filter_mode,
            mipmap_filter: MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: f32::MAX,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        });
        (view, sampler)
    }

    /// Evicts a stale entry from the sprite texture cache: two live
    /// buffers can never share an address, so a cache entry at the same
    /// pointer with a different generation belongs to a freed buffer whose
    /// address the new one has recycled, and its texture is dead weight.
    /// An associated function (not a method) so the render loop can call
    /// it while it still holds the immutable pipeline borrows.
    fn evict_recycled_buffer(
        resources: &mut HashMap<(u64, u64), (TextureView, Sampler)>,
        key: (u64, u64),
    ) {
        if let Some(stale) = resources
            .iter()
            .find(|(k, _)| k.0 == key.0)
            .map(|(k, _)| *k)
        {
            resources.remove(&stale);
        }
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
    /// buffer only ever grows: its size is the peak light count so far
    /// (floored at the WGSL minimum binding size), and lighter frames
    /// reuse the bigger buffer, writing a shorter slice.
    fn field_buffer_for(&mut self, count: u32) -> Buffer {
        let size = light_field_buffer_size(count);
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
        let size = occluder_field_buffer_size(count);
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
        // The frame's processing probe: the engine's CPU time for this
        // frame, from here (the acquire is done — it may have waited on the
        // display's back buffer, and that wait is not processing) to the
        // queue submission below. The present handoff is display time.
        let t0 = now_millis();

        // Let the user update the scene and draw this frame, in their
        // coordinate system: the window's logical pixels at the display's
        // scale factor — or, in fixed render size mode (`Config::render_size`),
        // the render buffer's pixels one-for-one, the stretch pass carrying
        // them to whatever the window is.
        let surface_px = self.pixel_size();
        let (user_size, user_scale) = match self.render_size {
            Some([w, h]) => ((w, h), 1.0f32),
            None => (self.logical_size, self.scale),
        };
        // The fraction of the surface the render fills once its aspect is
        // preserved: (1, 1) unless fixed render size mode meets a window of
        // a different shape, in which case the content is letterboxed and
        // both the cursor mapping and the blit pass account for it.
        let fit = content_fit(self.render_size, surface_px);
        let mut canvas = Canvas::new(user_size, user_scale);
        let now = now_millis();
        let dt = self
            .last_millis
            .map(|last| ((now - last).max(0.0) / 1000.0).min(1.0) as f32)
            .unwrap_or(0.0);
        self.last_millis = Some(now);
        // Flip the cursor from window pixels (upper-left, y-down, physical)
        // to user coordinates (origin at the window's center, y up): the
        // surface-to-user ratio maps the density away in the default mode
        // (1/scale) and the surface-to-buffer ratio in fixed render size
        // mode, and one formula covers both. Dividing by the letterbox fit
        // maps the cursor across the content rectangle, not the whole
        // surface, so a pointer over a bar reads past the play edge.
        let mouse = self.mouse.map(|[mx, my]| {
            [
                (mx - surface_px.0 as f32 / 2.0) * user_size.0 as f32
                    / (surface_px.0 as f32 * fit.0),
                (surface_px.1 as f32 / 2.0 - my) * user_size.1 as f32
                    / (surface_px.1 as f32 * fit.1),
            ]
        });
        let process = &mut self.process;
        let scene = &mut self.scene;
        let keys = &self.keys;
        let typed = &self.typed;
        let mouse_buttons = &self.mouse_buttons;
        let gilrs = self.gilrs.as_ref();
        {
            let mut ctx = Context {
                canvas: &mut canvas,
                scene,
                keys,
                typed,
                // The window is attached before the first frame renders, so
                // this is `Some` for every frame the process sees; the
                // `Option` is the type the field has before attach.
                window: self.window.as_deref(),
                expected_fps: self.expected_fps,
                mouse,
                mouse_buttons,
                gilrs,
                frame_processing_ms: self.frame_processing_ms,
                frame_draw_calls: self.frame_draw_calls,
                frame_diagnostic_draw_calls: self.frame_diagnostic_draw_calls,
                mouse_wheel: self.mouse_wheel,
            };
            process.process(&mut ctx, dt);
        }
        // The wheel delta belongs to this frame only: reset the accumulator
        // so a frame that never reads it discards the delta rather than
        // leaking it into the next.
        self.mouse_wheel = 0.0;
        // The user's process ran; now update the scene tree itself: every
        // node's `Node::process`, children before their parent.
        self.scene.visit();
        // The scene was just updated; draw it into the frame's draw list.
        canvas.draw_scene(&self.scene);
        // Expand the text into per-glyph sprite quads before the sort, so
        // each glyph keeps its node's position in the paint order.
        canvas.expand_text(&mut self.text_atlases);
        // An atlas that packed new glyphs this frame replaced its pixel
        // buffer, so the texture keyed to the old buffer is stale: drop it
        // before the sprite draws, so the fresh buffer uploads and a
        // recycled address can never serve the old texture.
        for atlas in self.text_atlases.values_mut() {
            if let Some(stale) = atlas.take_stale_key() {
                self.sprite_resources.remove(&stale);
            }
        }

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
        self.queue
            .write_buffer(&occluder_buffer, 0, &occluders.data);

        // The frame's destination: the swapchain texture — or, in fixed
        // render size mode when the surface is not already exactly that
        // size, the offscreen buffer the blit pass stretches over the
        // window afterwards. A surface that already matches the render
        // size needs neither buffer nor pass. The target is taken out of
        // `self` for the frame — the draw loop needs `&mut self` — and
        // stored back after the blit. (This runs before the pipeline
        // borrows below for the same reason: building it needs `&mut self`.)
        let surface_view = output
            .texture
            .create_view(&TextureViewDescriptor::default());
        let target = match self.render_size {
            Some(size) if size != [surface_px.0, surface_px.1] => {
                let cached = self.blit_target.take().filter(|t| t.size == size);
                match cached {
                    Some(t) => Some(t),
                    None => Some(self.new_blit_target(size)),
                }
            }
            _ => {
                self.blit_target = None;
                None
            }
        };
        let frame_area = match &target {
            Some(t) => [t.size[0], t.size[1]],
            None => [output.texture.width(), output.texture.height()],
        };
        let view = match &target {
            Some(t) => &t.view,
            None => &surface_view,
        };

        let (
            Some(line_pipeline),
            Some(polyline_pipeline),
            Some(circle_pipeline),
            Some(rect_pipeline),
            Some(shape_pipeline),
            Some(sprite_pipeline),
            Some(particle_pipeline),
        ) = (
            self.line_pipeline.as_ref(),
            self.polyline_pipeline.as_ref(),
            self.circle_pipeline.as_ref(),
            self.rect_pipeline.as_ref(),
            self.shape_pipeline.as_ref(),
            self.sprite_pipeline.as_ref(),
            self.particle_pipeline.as_ref(),
        )
        else {
            return;
        };
        let Some(particle_index_buffer) = &self.particle_index_buffer else {
            return;
        };

        // The frame's clear color: the last background node in paint order,
        // or the default when the frame has none.
        let clear = clear_color(&draws);

        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor { label: None });
        // The frame's GPU draw-call count: incremented once per
        // `pass.draw` / `pass.draw_indexed` below, so draws that are skipped
        // (off-screen, background, light, empty) never count. The second
        // counter keeps the same increments for the draws tagged by the
        // `Diagnostics` overlay — its share of the frame's draw calls.
        let mut draw_calls: u32 = 0;
        let mut diagnostic_draws: u32 = 0;
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(RenderPassColorAttachment {
                    view,
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
            // for the full target, then each draw sets the scissor to its
            // tight bounding box, so fragments outside it are discarded and
            // the fragment shader only runs over the pixels the object can
            // write. The viewport is left at the full target, so the
            // shaders' pixel coordinates stay absolute.
            let render_area = frame_area;
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
                        a,
                        b,
                        width,
                        color,
                        diagnostic,
                        ..
                    } => {
                        let (_buffer, bind_group) = self.primitive_uniform(
                            line_pipeline,
                            "line uniforms",
                            &line_uniform_data(a, b, color, width),
                        );
                        pass.set_pipeline(line_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        draw_calls += 1;
                        if diagnostic {
                            diagnostic_draws += 1;
                        }
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Polyline {
                        points,
                        width,
                        color,
                        diagnostic,
                        ..
                    } => {
                        let (_buffer, bind_group) = self.primitive_uniform(
                            polyline_pipeline,
                            "polyline uniforms",
                            &polyline_uniform_data(&points, color, width),
                        );
                        pass.set_pipeline(polyline_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        draw_calls += 1;
                        if diagnostic {
                            diagnostic_draws += 1;
                        }
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Circle {
                        center,
                        radius,
                        color,
                        diagnostic,
                        ..
                    } => {
                        let (_buffer, bind_group) = self.primitive_uniform(
                            circle_pipeline,
                            "circle uniforms",
                            &circle_uniform_data(center, color, radius),
                        );
                        pass.set_pipeline(circle_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        draw_calls += 1;
                        if diagnostic {
                            diagnostic_draws += 1;
                        }
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Rectangle {
                        center,
                        extent,
                        color,
                        diagnostic,
                        ..
                    } => {
                        let (_buffer, bind_group) = self.primitive_uniform(
                            rect_pipeline,
                            "rectangle uniforms",
                            &rect_uniform_data(center, extent, color),
                        );
                        pass.set_pipeline(rect_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        draw_calls += 1;
                        if diagnostic {
                            diagnostic_draws += 1;
                        }
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Shape {
                        world,
                        center,
                        params,
                        kind,
                        aa,
                        color,
                        glow,
                        lit,
                        diagnostic,
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
                            &shape_uniform_data(inv, center, params, kind, aa, color, glow, lit),
                            &field_buffer,
                            &occluder_buffer,
                        );
                        pass.set_pipeline(shape_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        draw_calls += 1;
                        if diagnostic {
                            diagnostic_draws += 1;
                        }
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Sprite {
                        world,
                        data,
                        size,
                        texture_size,
                        filter,
                        tint,
                        alpha,
                        glow,
                        lit,
                        uv_rect,
                        generation,
                        diagnostic,
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
                        let key = (Arc::as_ptr(&data) as *const () as u64, generation);
                        let (view, sampler) = match self.sprite_resources.get(&key) {
                            Some((view, sampler)) => (view.clone(), sampler.clone()),
                            None => {
                                // This buffer may have recycled a freed
                                // buffer's address: evict the stale entry
                                // before inserting, so it can never serve
                                // pixels to a live buffer.
                                Self::evict_recycled_buffer(&mut self.sprite_resources, key);
                                let (view, sampler) = self.sprite_texture(
                                    &data,
                                    [texture_size[0] as f32, texture_size[1] as f32],
                                    filter,
                                );
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
                            &sprite_uniform_data(inv, size, tint, alpha, glow, lit, uv_rect),
                            &field_buffer,
                            &occluder_buffer,
                        );
                        pass.set_pipeline(sprite_pipeline);
                        pass.set_bind_group(0, &bind_group, &[]);
                        draw_calls += 1;
                        if diagnostic {
                            diagnostic_draws += 1;
                        }
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
                        sprite_generation,
                        lit,
                        diagnostic,
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
                        let (view, sampler) = match sprite_data.filter(|_| kind >= 1.5) {
                            Some(image) => {
                                // The image's stamped generation identifies
                                // the buffer: a rebuilt in-memory image
                                // gets a fresh one, so a recycled address
                                // can never hit a stale texture.
                                let key =
                                    (Arc::as_ptr(&image) as *const () as u64, sprite_generation);
                                match self.sprite_resources.get(&key) {
                                    Some((view, sampler)) => (view.clone(), sampler.clone()),
                                    None => {
                                        Self::evict_recycled_buffer(
                                            &mut self.sprite_resources,
                                            key,
                                        );
                                        // Particle images keep bilinear
                                        // sampling: `ParticleShape` carries
                                        // no filter.
                                        let (view, sampler) = self.sprite_texture(
                                            &image,
                                            [sprite_size[0] as f32, sprite_size[1] as f32],
                                            SpriteFilter::Linear,
                                        );
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
                        draw_calls += 1;
                        if diagnostic {
                            diagnostic_draws += 1;
                        }
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
        // Fixed render size mode: stretch the buffer over the window — one
        // full-screen triangle, one bilinear sample per window pixel, scaled
        // down to the letterbox rectangle when the shapes differ (black bars
        // from the clear around it). The surface needs no clear when
        // rendering direct.
        if let (Some(t), Some(blit_pipeline)) = (target.as_ref(), self.blit_pipeline.as_ref()) {
            self.queue.write_buffer(&t.fit_buffer, 0, &fit_bytes(fit));
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("blit pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &surface_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(blit_pipeline);
            pass.set_bind_group(0, &t.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        // Put the target back for the next frame — or leave `self` empty,
        // so a surface that has since resized to exactly the render size
        // finds nothing to reuse and drops the buffer for good.
        self.blit_target = target;
        self.queue.submit([encoder.finish()]);
        // Record this frame's processing time and draw-call count before the
        // present: the frame's own `Context` already went out without them,
        // so these values reach the next frame's, where they are the "last
        // frame's" values.
        self.frame_processing_ms = now_millis() - t0;
        self.frame_draw_calls = draw_calls;
        self.frame_diagnostic_draw_calls = diagnostic_draws;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_monitor_fit_leaves_sizes_that_fit_untouched() {
        assert_eq!(fit_into_monitor([1280, 720], [1512, 982]), [1280, 720]);
        // Exactly the budget is not an overflow.
        assert_eq!(fit_into_monitor([1512, 982], [1512, 982]), [1512, 982]);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_monitor_fit_shrinks_an_oversized_request_aspect_preserved() {
        // 1920x1080 on a 1512-wide MacBook: the width binds, and the
        // height falls by the same factor — the 16:9 shape survives.
        let [w, h] = fit_into_monitor([1920, 1080], [1512, 982]);
        assert_eq!(w, 1512);
        assert!((w as f64 / h as f64 - 16.0 / 9.0).abs() < 0.01);
        // A tall request on a wide monitor is height-bound instead.
        let [w, h] = fit_into_monitor([1080, 1920], [1512, 982]);
        assert_eq!(h, 982);
        assert!((w as f64 / h as f64 - 1080.0 / 1920.0).abs() < 0.01);
    }

    #[test]
    fn a_matching_aspect_fills_the_surface() {
        assert_eq!(content_fit(None, (100, 100)), (1.0, 1.0));
        assert_eq!(content_fit(Some([1920, 1080]), (1920, 1080)), (1.0, 1.0));
        // The same shape at a different size fills too: 800x450 is 16:9.
        let (fx, fy) = content_fit(Some([1920, 1080]), (800, 450));
        assert!((fx - 1.0).abs() < 1e-4 && (fy - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_wider_surface_than_buffer_pillarboxes_horizontally() {
        // A square window with a 16:9 buffer: the buffer is wider, so it
        // spans the full width and shrinks in height.
        let (fx, fy) = content_fit(Some([1920, 1080]), (1000, 1000));
        assert_eq!(fx, 1.0);
        assert!((fy - 1000.0 * 1080.0 / (1920.0 * 1000.0)).abs() < 1e-4);
        assert!(fy > 0.55 && fy < 0.57);
    }

    #[test]
    fn a_taller_surface_than_buffer_letterboxes_vertically() {
        // A square window with a 1:2 (portrait) buffer: the buffer is
        // taller, so it spans the full height and shrinks in width.
        let (fx, fy) = content_fit(Some([540, 1080]), (1000, 1000));
        assert_eq!(fy, 1.0);
        assert!((fx - 1000.0 * 540.0 / (1080.0 * 1000.0)).abs() < 1e-4);
        assert!((fx - 0.5).abs() < 1e-4);
    }

    #[test]
    fn a_zero_sized_surface_falls_back_to_filling() {
        // A zero surface would divide by zero; the guard returns (1, 1).
        assert_eq!(content_fit(Some([1920, 1080]), (0, 0)), (1.0, 1.0));
    }

    #[test]
    fn fit_bytes_writes_the_two_extents_and_pads() {
        let bytes = fit_bytes((0.5, 0.25));
        assert_eq!(f32::from_le_bytes(bytes[0..4].try_into().unwrap()), 0.5);
        assert_eq!(f32::from_le_bytes(bytes[4..8].try_into().unwrap()), 0.25);
        assert_eq!(&bytes[8..], &[0u8; 8]);
    }
}
