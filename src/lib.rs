//! frost — a minimal winit + wgpu immediate-mode drawing library.
//!
//! Provide a [`Process`]: a function called once per frame with a [`Canvas`]
//! and the delta time in seconds since the previous frame.
//! Draw with window-centered pixel coordinates: the origin is the window
//! center, y points up, so the top-left corner is `(-width/2, height/2)`.
//!
//! ```no_run
//! frost::run(|ctx: &mut frost::Canvas, _dt: f32| {
//!     let (w, h) = ctx.size();
//!     ctx.set_background(frost::Color { r: 0.05, g: 0.06, b: 0.12 });
//!     ctx.line(
//!         -w / 2.0, h / 2.0, w / 2.0, -h / 2.0,
//!         frost::Color { r: 1.0, g: 1.0, b: 1.0 },
//!         0.0,
//!     );
//!     ctx.circle(
//!         0.0, 0.0, h / 4.0,
//!         frost::Color { r: 0.9, g: 0.4, b: 0.2 },
//!         1.0,
//!     );
//! });
//! ```
//!
//! Draw order is set by each object's `z`: lower `z` is drawn first (further
//! back). Objects with the same `z` are drawn in call order, so the last one
//! drawn is on top.
//!
//! Key presses are logged and Escape closes the window.

use std::borrow::Cow;
use std::error::Error;
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Instant;

use wgpu::{
    Adapter, BindGroup, BindGroupDescriptor, BindGroupEntry, BindingResource, Buffer,
    BufferBinding, BufferDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    CommandEncoderDescriptor, CurrentSurfaceTexture, Device, DeviceDescriptor, FragmentState,
    Instance, MultisampleState, PipelineCompilationOptions, PrimitiveState, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor,
    RequestAdapterOptions, ShaderModule, ShaderModuleDescriptor, ShaderSource, StoreOp, Surface,
    SurfaceTexture, TextureFormat, TextureViewDescriptor, VertexState,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::NamedKey;
use winit::window::{Window, WindowId};

/// The background color when the user does not override it with
/// [`Canvas::set_background`].
const DEFAULT_BACKGROUND: Color = Color {
    r: 0.07,
    g: 0.09,
    b: 0.14,
};
const DEFAULT_LINE_WIDTH: f32 = 2.0;

// ============================ public API ============================

/// An RGB color with channels in `0.0..=1.0`, used for the background and for
/// the color of each drawn object.
#[derive(Clone, Copy, Debug)]
pub struct Color {
    /// Red channel.
    pub r: f32,
    /// Green channel.
    pub g: f32,
    /// Blue channel.
    pub b: f32,
}

impl Color {
    /// The channels as `[r, g, b]`.
    fn channels(&self) -> [f32; 3] {
        [self.r, self.g, self.b]
    }
}

/// The drawing surface handed to [`Process::process`] each frame.
///
/// Coordinates are in pixels with the origin at the window center and the y
/// axis pointing up: the top-left corner is `(-width/2, height/2)` and the
/// bottom-right corner is `(width/2, -height/2)`.
pub struct Canvas {
    size: (f32, f32),
    draws: Vec<Draw>,
    background: Color,
}

impl Canvas {
    fn new(pixel_size: (u32, u32)) -> Self {
        Self {
            size: (pixel_size.0 as f32, pixel_size.1 as f32),
            draws: Vec::new(),
            background: DEFAULT_BACKGROUND,
        }
    }

    /// The window size in pixels as `(width, height)`.
    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    /// Sets the background color for this frame.
    ///
    /// The default is a dark blue. The setting only lasts for the current
    /// frame, since a fresh canvas is created each frame, so call it again
    /// each frame to keep a custom background.
    pub fn set_background(&mut self, color: Color) {
        self.background = color
    }

    /// Draws a line from `(x0, y0)` to `(x1, y1)` in `color`, default width
    /// 2 px.
    ///
    /// `z` is the draw order: lower `z` is drawn first (further back). Lines
    /// with the same `z` are drawn in call order, so the last one drawn is on
    /// top.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Color, z: f32) {
        self.draws.push(Draw::Line {
            a: self.user_to_pixels(x0, y0),
            b: self.user_to_pixels(x1, y1),
            width: DEFAULT_LINE_WIDTH,
            color,
            z,
        });
    }

    /// Draws a filled circle centered at `(cx, cy)` with `radius` in pixels,
    /// in `color`.
    ///
    /// `z` is the draw order: lower `z` is drawn first (further back). Circles
    /// with the same `z` are drawn in call order, so the last one drawn is on
    /// top.
    pub fn circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color, z: f32) {
        self.draws.push(Draw::Circle {
            center: self.user_to_pixels(cx, cy),
            radius: radius.max(0.0),
            color,
            z,
        });
    }

    /// Draws a filled rectangle centered at `(cx, cy)` with `dx` and `dy`
    /// extents (half-width and half-height) in pixels, in `color`.
    ///
    /// `z` is the draw order: lower `z` is drawn first (further back).
    /// Rectangles with the same `z` are drawn in call order, so the last one
    /// drawn is on top.
    pub fn rectangle(&mut self, cx: f32, cy: f32, dx: f32, dy: f32, color: Color, z: f32) {
        self.draws.push(Draw::Rectangle {
            center: self.user_to_pixels(cx, cy),
            extent: [dx.max(0.0), dy.max(0.0)],
            color,
            z,
        });
    }

    /// Converts user coordinates (window center origin, y up) to the pixel
    /// coordinates the shaders use (top-left origin, y down).
    fn user_to_pixels(&self, x: f32, y: f32) -> [f32; 2] {
        [x + self.size.0 / 2.0, self.size.1 / 2.0 - y]
    }
}

/// A drawing recorded for the current frame, in pixel space.
///
/// `z` is the draw order: lower `z` is drawn first (further back). Drawings
/// with the same `z` are drawn in call order, so the last one drawn is on top.
#[derive(Clone, Copy)]
enum Draw {
    Line {
        a: [f32; 2],
        b: [f32; 2],
        width: f32,
        color: Color,
        z: f32,
    },
    Circle {
        center: [f32; 2],
        radius: f32,
        color: Color,
        z: f32,
    },
    Rectangle {
        center: [f32; 2],
        extent: [f32; 2],
        color: Color,
        z: f32,
    },
}

impl Draw {
    fn z(&self) -> f32 {
        match self {
            Draw::Line { z, .. } => *z,
            Draw::Circle { z, .. } => *z,
            Draw::Rectangle { z, .. } => *z,
        }
    }
}

/// Called once per frame; draw into `canvas`.
///
/// `dt` is the time in seconds since the previous frame (`0.0` on the first
/// frame, clamped to at most `1.0`s to absorb stalls). Use it to advance
/// animation state such as a [`Tween`].
pub trait Process {
    fn process(&mut self, canvas: &mut Canvas, dt: f32);
}

/// Any closure `FnMut(&mut Canvas, f32)` is a [`Process`].
impl<F> Process for F
where
    F: FnMut(&mut Canvas, f32),
{
    fn process(&mut self, canvas: &mut Canvas, dt: f32) {
        self(canvas, dt);
    }
}

/// A linear ping-pong progress value driven by a time step.
///
/// Each [`Tween::tick`] advances from `0.0` toward `1.0` over `duration`
/// seconds and then back toward `0.0`, in an endless loop. Multiplied by a
/// target distance it yields a constant-rate position that eases linearly
/// between two endpoints.
#[derive(Clone, Copy)]
pub struct Tween {
    /// Position within the current round trip, in `0.0..2.0`.
    phase: f32,
    /// Seconds for one leg (the `0.0 -> 1.0` travel).
    duration: f32,
}

impl Tween {
    /// Creates a tween whose one-way travel takes `duration` seconds.
    pub fn new(duration: f32) -> Self {
        Self {
            phase: 0.0,
            duration: duration.max(1e-6),
        }
    }

    /// Advances by `dt` seconds and returns the current progress in `0.0..1.0`
    /// (ping-pong: `0.0 -> 1.0 -> 0.0 -> 1.0 -> ...`).
    pub fn tick(&mut self, dt: f32) -> f32 {
        self.phase = (self.phase + dt / self.duration) % 2.0;
        if self.phase < 1.0 {
            self.phase
        } else {
            2.0 - self.phase
        }
    }
}

/// Opens the window and runs the event loop, calling `process` once per frame.
///
/// The window closes on Escape or when the user requests it.
pub fn run<P: Process>(process: P) -> Result<(), Box<dyn Error>> {
    log::info!("frost starting up");

    let instance = Instance::default();
    let adapter = block_on(instance.request_adapter(&RequestAdapterOptions::default()))
        .expect("no suitable GPU adapter found");
    log::info!("using adapter: {:?}", adapter.get_info().name);

    let (device, queue) = block_on(adapter.request_device(&DeviceDescriptor::default()))
        .expect("failed to create GPU device");

    let event_loop = EventLoop::new()?;
    let mut app = Frost {
        instance,
        adapter,
        device,
        queue,
        window_id: None,
        window: None,
        logical_size: (0, 0),
        scale: 1.0,
        surface: None,
        line_pipeline: None,
        circle_pipeline: None,
        rect_pipeline: None,
        format: None,
        last_time: None,
        process,
    };
    event_loop.run_app(&mut app)?;

    log::info!("event loop finished");
    Ok(())
}

// ============================ internal machinery ============================

const SHADER: &str = r#"
struct Uniforms {
    a: vec2<f32>,
    b: vec2<f32>,
    color: vec3<f32>,
    width: f32,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Full-screen triangle in NDC: the fragment shader must evaluate over
    // the whole window, wherever the line is.
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[i], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let p = frag_coord.xy;
    // Distance in pixels from the fragment to the segment a-b.
    let ab = u.b - u.a;
    let t = clamp(dot(p - u.a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
    let dist = length(p - (u.a + ab * t));
    let half_width = u.width * 0.5;
    let aa = 0.75; // ~1px anti-alias band.
    let alpha = 1.0 - smoothstep(half_width - aa, half_width + aa, dist);
    // Emit the line's color and coverage; composited over the existing
    // attachment via alpha blending.
    return vec4<f32>(u.color, alpha);
}
"#;

const CIRCLE_SHADER: &str = r#"
struct CircleUniforms {
    center: vec2<f32>,
    color: vec3<f32>,
    radius: f32,
};

@group(0) @binding(0)
var<uniform> u: CircleUniforms;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Full-screen triangle in NDC so the fragment shader runs everywhere.
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[i], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let p = frag_coord.xy;
    // Distance in pixels from the fragment to the circle center.
    let d = length(p - u.center);
    let aa = 0.75; // ~1px anti-alias band.
    let alpha = 1.0 - smoothstep(u.radius - aa, u.radius + aa, d);
    // Emit the circle's color and coverage; composited over the existing
    // attachment via alpha blending.
    return vec4<f32>(u.color, alpha);
}
"#;

const RECT_SHADER: &str = r#"
struct RectUniforms {
    center: vec2<f32>,
    extent: vec2<f32>,
    color: vec3<f32>,
};

@group(0) @binding(0)
var<uniform> u: RectUniforms;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Full-screen triangle in NDC so the fragment shader runs everywhere.
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[i], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let p = frag_coord.xy;
    // Signed distance in pixels from the fragment to the rectangle border:
    // negative inside, positive outside.
    let d = max(
        abs(p.x - u.center.x) - u.extent.x,
        abs(p.y - u.center.y) - u.extent.y,
    );
    let aa = 0.75; // ~1px anti-alias band.
    let alpha = 1.0 - smoothstep(-aa, aa, d);
    // Emit the rectangle's color and coverage; composited over the existing
    // attachment via alpha blending.
    return vec4<f32>(u.color, alpha);
}
"#;

struct Frost<P: Process> {
    instance: Instance,
    adapter: Adapter,
    device: Device,
    queue: Queue,
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
    /// The surface format the current pipelines were built for; they are only
    /// rebuilt when this changes.
    format: Option<TextureFormat>,
    /// Timestamp of the previous rendered frame, used to compute `dt`.
    last_time: Option<Instant>,
    process: P,
}

impl<P: Process> ApplicationHandler for Frost<P> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }

        let window = Arc::new(
            event_loop
                .create_window(Window::default_attributes().with_title("frost"))
                .expect("failed to create window"),
        );
        // winit (Wayland) only delivers RedrawRequested after a compositor
        // frame callback, so explicitly request the first frame; otherwise
        // the window is never mapped and nothing is ever drawn.
        window.request_redraw();
        self.window_id = Some(window.id());
        self.logical_size = (window.inner_size().width, window.inner_size().height);
        self.scale = window.scale_factor() as f32;
        log::info!(
            "window created ({}x{} @ {:.2}x)",
            self.logical_size.0,
            self.logical_size.1,
            self.scale
        );

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
        let config = surface
            .get_default_config(&self.adapter, pixel_size.0, pixel_size.1)
            .expect("no compatible surface format");
        surface.configure(&self.device, &config);
        log::info!(
            "surface configured at {}x{}, format {:?}",
            config.width,
            config.height,
            config.format
        );

        self.set_up_pipelines(config.format);
        self.format = Some(config.format);
        self.surface = Some(surface);
        // Commit the first frame immediately so the compositor maps the window.
        self.render();
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
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    self.logical_size = (size.width, size.height);
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

impl<P: Process> Frost<P> {
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
        let config = surface
            .get_default_config(&self.adapter, pixel_size.0, pixel_size.1)
            .expect("no compatible surface format");
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
            source: ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
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
    }

    /// Builds a render pipeline for a full-screen-triangle shader whose single
    /// uniform is bound at binding 0.
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

    fn render(&mut self) {
        let (output, reconfigure) = self.acquire_frame();
        if reconfigure {
            self.resize();
        }
        let Some(output) = output else {
            return;
        };
        log::trace!("render: acquired surface texture, submitting frame");

        // Ask the user what to draw this frame, in their coordinate system.
        let mut canvas = Canvas::new(self.pixel_size());
        let now = Instant::now();
        let dt = self
            .last_time
            .map(|last| now.duration_since(last).as_secs_f32().min(1.0))
            .unwrap_or(0.0);
        self.last_time = Some(now);
        self.process.process(&mut canvas, dt);

        // Paint order: ascending z, lower z behind. `sort_by` is stable, so
        // draws with equal z keep call order and the last drawn is on top.
        canvas.draws.sort_by(|a, b| {
            a.z()
                .partial_cmp(&b.z())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let (Some(line_pipeline), Some(circle_pipeline), Some(rect_pipeline)) = (
            self.line_pipeline.as_ref(),
            self.circle_pipeline.as_ref(),
            self.rect_pipeline.as_ref(),
        ) else {
            return;
        };

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
                            r: canvas.background.r as f64,
                            g: canvas.background.g as f64,
                            b: canvas.background.b as f64,
                            a: 1.0,
                        }),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // A fresh uniform buffer (and bind group) per draw call, since all
            // write_buffer copies complete before any draw executes.
            for draw in canvas.draws {
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

/// Writes `value` as little-endian f32 bytes into `data` at byte `offset`.
fn write_f32_at(data: &mut [u8], offset: usize, value: f32) {
    data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// Line uniform data, 32 bytes, matching the WGSL uniform-space layout of
/// the `Uniforms` Wgsl struct: `a` @ 0, `b` @ 8, `color` @ 16 (a vec3<f32>
/// is 16-byte aligned in uniform space), `width` @ 28 (the next member is
/// aligned to its own alignment, so the f32 follows the vec3 without a gap);
/// the struct size rounds up to 32.
fn line_uniform_data(a: [f32; 2], b: [f32; 2], color: Color, width: f32) -> Vec<u8> {
    let mut data = vec![0u8; 32];
    write_f32_at(&mut data, 0, a[0]);
    write_f32_at(&mut data, 4, a[1]);
    write_f32_at(&mut data, 8, b[0]);
    write_f32_at(&mut data, 12, b[1]);
    write_f32_at(&mut data, 16, color.r);
    write_f32_at(&mut data, 20, color.g);
    write_f32_at(&mut data, 24, color.b);
    write_f32_at(&mut data, 28, width);
    data
}

/// Circle uniform data, 32 bytes, matching the WGSL uniform-space layout of
/// the `CircleUniforms` Wgsl struct: `center` @ 0, `color` @ 16 (a
/// vec3<f32> is 16-byte aligned in uniform space, leaving an 8-byte gap),
/// `radius` @ 28 (the next member is aligned to its own alignment, so the
/// f32 follows the vec3 without a gap); the struct size rounds up to 32.
fn circle_uniform_data(center: [f32; 2], color: Color, radius: f32) -> Vec<u8> {
    let mut data = vec![0u8; 32];
    write_f32_at(&mut data, 0, center[0]);
    write_f32_at(&mut data, 4, center[1]);
    write_f32_at(&mut data, 16, color.r);
    write_f32_at(&mut data, 20, color.g);
    write_f32_at(&mut data, 24, color.b);
    write_f32_at(&mut data, 28, radius);
    data
}

/// Rectangle uniform data, matching the `RectUniforms` Wgsl struct: center,
/// extent, color. The 28 bytes of data are zero-padded to the struct's
/// 32-byte minimum binding size.
fn rect_uniform_data(center: [f32; 2], extent: [f32; 2], color: Color) -> Vec<u8> {
    let mut data: Vec<u8> = center
        .iter()
        .chain(extent.iter())
        .chain(color.channels().iter())
        .flat_map(|f| f.to_le_bytes())
        .collect();
    data.resize(32, 0);
    data
}

struct NoopWaker;

impl Wake for NoopWaker {
    fn wake(self: Arc<Self>) {}
}

fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(NoopWaker));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
}
