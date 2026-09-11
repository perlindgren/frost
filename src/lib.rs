//! frost — a minimal winit + wgpu immediate-mode drawing library.
//!
//! Provide a [`Process`]: a function called once per frame with a [`Canvas`].
//! Draw with window-centered pixel coordinates: the origin is the window
//! center, y points down, so the top-left corner is `(-width/2, height/2)`.
//!
//! ```no_run
//! frost::run(|ctx: &mut frost::Canvas| {
//!     let (w, h) = ctx.size();
//!     ctx.line(-w / 2.0, h / 2.0, w / 2.0, -h / 2.0);
//!     ctx.circle(0.0, 0.0, h / 4.0);
//! });
//! ```
//!
//! Key presses are logged and Escape closes the window.

use std::borrow::Cow;
use std::error::Error;
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use wgpu::{
    Adapter, BindGroup, BindGroupDescriptor, BindGroupEntry, BindingResource, Buffer,
    BufferBinding, BufferDescriptor, BufferUsages, Color, ColorTargetState, ColorWrites,
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

const BACKGROUND: Color = Color {
    r: 0.07,
    g: 0.09,
    b: 0.14,
    a: 1.0,
};
const DEFAULT_LINE_WIDTH: f32 = 2.0;

// ============================ public API ============================

/// The drawing surface handed to [`Process::process`] each frame.
///
/// Coordinates are in pixels with the origin at the window center and the y
/// axis pointing down: the top-left corner is `(-width/2, height/2)` and the
/// bottom-right corner is `(width/2, -height/2)`.
pub struct Canvas {
    size: (f32, f32),
    draws: Vec<Draw>,
}

impl Canvas {
    fn new(pixel_size: (u32, u32)) -> Self {
        Self {
            size: (pixel_size.0 as f32, pixel_size.1 as f32),
            draws: Vec::new(),
        }
    }

    /// The window size in pixels as `(width, height)`.
    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    /// Draws a line from `(x0, y0)` to `(x1, y1)`, default width 2 px.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.draws.push(Draw::Line {
            a: self.to_pixels(x0, y0),
            b: self.to_pixels(x1, y1),
            width: DEFAULT_LINE_WIDTH,
        });
    }

    /// Draws a filled circle centered at `(cx, cy)` with `radius` in pixels.
    pub fn circle(&mut self, cx: f32, cy: f32, radius: f32) {
        self.draws.push(Draw::Circle {
            center: self.to_pixels(cx, cy),
            radius: radius.max(0.0),
        });
    }

    /// Window-centered user coordinates → pixel coordinates (top-left origin,
    /// y down), for the shaders.
    fn to_pixels(&self, x: f32, y: f32) -> [f32; 2] {
        [x + self.size.0 / 2.0, self.size.1 / 2.0 - y]
    }
}

/// A drawing recorded for the current frame, in pixel space.
#[derive(Clone, Copy)]
enum Draw {
    Line {
        a: [f32; 2],
        b: [f32; 2],
        width: f32,
    },
    Circle {
        center: [f32; 2],
        radius: f32,
    },
}

/// Called once per frame; draw into `canvas`.
pub trait Process {
    fn process(&mut self, canvas: &mut Canvas);
}

/// Any closure `FnMut(&mut Canvas)` is a [`Process`].
impl<F> Process for F
where
    F: FnMut(&mut Canvas),
{
    fn process(&mut self, canvas: &mut Canvas) {
        self(canvas);
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
        logical_size: (0, 0),
        scale: 1.0,
        surface: None,
        line_pipeline: None,
        line_bind_group: None,
        line_uniform: None,
        circle_pipeline: None,
        circle_bind_group: None,
        circle_uniform: None,
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
    return vec4<f32>(vec3<f32>(1.0), alpha);
}
"#;

const CIRCLE_SHADER: &str = r#"
struct CircleUniforms {
    center: vec2<f32>,
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
    return vec4<f32>(vec3<f32>(1.0), alpha);
}
"#;

struct Frost<P: Process> {
    instance: Instance,
    adapter: Adapter,
    device: Device,
    queue: Queue,
    #[allow(dead_code)]
    window_id: Option<WindowId>,
    logical_size: (u32, u32),
    scale: f32,
    surface: Option<Surface<'static>>,
    line_pipeline: Option<RenderPipeline>,
    line_bind_group: Option<BindGroup>,
    line_uniform: Option<Buffer>,
    circle_pipeline: Option<RenderPipeline>,
    circle_bind_group: Option<BindGroup>,
    circle_uniform: Option<Buffer>,
    process: P,
}

impl<P: Process> ApplicationHandler for Frost<P> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }

        let window = event_loop
            .create_window(Window::default_attributes().with_title("frost"))
            .expect("failed to create window");
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

        let surface = self
            .instance
            .create_surface(window)
            .expect("failed to create wgpu surface");
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
            WindowEvent::RedrawRequested => self.render(),
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
        self.set_up_pipelines(config.format);
    }

    /// Creates the shared line and circle pipelines, with one uniform buffer
    /// (and bind group) each, reused for every draw call of the frame.
    fn set_up_pipelines(&mut self, format: TextureFormat) {
        let device = &self.device;

        let line_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("line shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let line_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("line uniforms"),
            size: 24,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (line_pipeline, line_bind_group) =
            Self::create_pipeline(device, &line_module, format, &line_buffer, "line pipeline");

        let circle_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("circle shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(CIRCLE_SHADER)),
        });
        let circle_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("circle uniforms"),
            size: 16,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (circle_pipeline, circle_bind_group) = Self::create_pipeline(
            device,
            &circle_module,
            format,
            &circle_buffer,
            "circle pipeline",
        );

        self.line_uniform = Some(line_buffer);
        self.line_pipeline = Some(line_pipeline);
        self.line_bind_group = Some(line_bind_group);
        self.circle_uniform = Some(circle_buffer);
        self.circle_pipeline = Some(circle_pipeline);
        self.circle_bind_group = Some(circle_bind_group);
    }

    /// Builds a render pipeline and matching bind group for a full-screen-triangle
    /// shader whose single uniform is bound at binding 0.
    fn create_pipeline(
        device: &Device,
        module: &ShaderModule,
        format: TextureFormat,
        buffer: &Buffer,
        label: &str,
    ) -> (RenderPipeline, BindGroup) {
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
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
        });
        let layout = pipeline.get_bind_group_layout(0);
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some(label),
            layout: &layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer,
                    offset: 0,
                    size: None,
                }),
            }],
        });
        (pipeline, bind_group)
    }

    fn write_line_uniform(&self, buffer: &Buffer, a: [f32; 2], b: [f32; 2], width: f32) {
        let data = a
            .iter()
            .chain(b.iter())
            .chain(std::iter::once(&width))
            .flat_map(|f| f.to_le_bytes())
            .collect::<Vec<u8>>();
        self.queue.write_buffer(buffer, 0, &data);
    }

    fn write_circle_uniform(&self, buffer: &Buffer, center: [f32; 2], radius: f32) {
        let data = center
            .iter()
            .chain(std::iter::once(&radius))
            .flat_map(|f| f.to_le_bytes())
            .collect::<Vec<u8>>();
        self.queue.write_buffer(buffer, 0, &data);
    }

    fn render(&mut self) {
        let (output, reconfigure) = self.acquire_frame();
        if reconfigure {
            self.resize();
        }
        let Some(output) = output else {
            return;
        };
        log::info!("render: acquired surface texture, submitting frame");

        // Ask the user what to draw this frame, in their coordinate system.
        let mut canvas = Canvas::new(self.pixel_size());
        self.process.process(&mut canvas);

        let (
            Some(line_pipeline),
            Some(line_bind_group),
            Some(line_uniform),
            Some(circle_pipeline),
            Some(circle_bind_group),
            Some(circle_uniform),
        ) = (
            self.line_pipeline.as_ref(),
            self.line_bind_group.as_ref(),
            self.line_uniform.as_ref(),
            self.circle_pipeline.as_ref(),
            self.circle_bind_group.as_ref(),
            self.circle_uniform.as_ref(),
        )
        else {
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
                        load: wgpu::LoadOp::Clear(BACKGROUND),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // One uniform buffer per primitive kind, rewritten per draw call;
            // write_buffer flushes before the command buffer executes, so each
            // draw sees its own parameters.
            for draw in canvas.draws {
                match draw {
                    Draw::Line { a, b, width } => {
                        self.write_line_uniform(line_uniform, a, b, width);
                        pass.set_pipeline(line_pipeline);
                        pass.set_bind_group(0, line_bind_group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                    Draw::Circle { center, radius } => {
                        self.write_circle_uniform(circle_uniform, center, radius);
                        pass.set_pipeline(circle_pipeline);
                        pass.set_bind_group(0, circle_bind_group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                }
            }
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(output);
        log::info!("render: frame presented");
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
