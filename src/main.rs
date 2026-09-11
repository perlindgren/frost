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
    RequestAdapterOptions, ShaderModuleDescriptor, ShaderSource, StoreOp, Surface, SurfaceTexture,
    TextureFormat, TextureViewDescriptor, VertexState,
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
const LINE_WIDTH: f32 = 2.0;

const SHADER: &str = r#"
struct Uniforms {
    size: vec2<f32>,
    line_width: f32,
};

@group(0) @binding(0)
var<uniform> u: Uniforms;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Full-screen triangle in NDC: covers the entire [-1,1] clip area, its
    // hypotenuse passing exactly through the top-right corner (1,1).
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
    );
    return vec4<f32>(positions[i], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let p = frag_coord.xy;
    // Distance in pixels from the point to the line (0,0)-(size.x,size.y).
    let dist = abs(u.size.y * p.x - u.size.x * p.y) / length(u.size);
    let half_width = u.line_width * 0.5;
    let aa = 0.75 / u.size.x;
    let alpha = 1.0 - smoothstep(half_width - aa, half_width + aa, dist);
    // Fall back to the window background where the line is not present.
    let bg = vec3<f32>(0.07, 0.09, 0.14);
    let color = mix(bg, vec3<f32>(1.0), alpha);
    return vec4<f32>(color, 1.0);
}
"#;

struct Frost {
    instance: Instance,
    adapter: Adapter,
    device: Device,
    queue: Queue,
    #[allow(dead_code)]
    window_id: Option<WindowId>,
    logical_size: (u32, u32),
    scale: f32,
    surface: Option<Surface<'static>>,
    pipeline: Option<RenderPipeline>,
    bind_group: Option<BindGroup>,
    uniform_buffer: Option<Buffer>,
}

impl ApplicationHandler for Frost {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }

        let window = event_loop
            .create_window(Window::default_attributes().with_title("frost"))
            .expect("failed to create window");
        // winit (Wayland) only delivers RedrawRequested after a compositor frame
        // callback, so explicitly request the first frame; otherwise the window
        // is never mapped and nothing is ever drawn.
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

        self.set_up_pipeline(config.format, pixel_size);
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

impl Frost {
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
        self.set_up_pipeline(config.format, pixel_size);
    }

    fn set_up_pipeline(&mut self, format: TextureFormat, pixel_size: (u32, u32)) {
        let device = &self.device;

        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("line shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });

        let uniform_buffer = device.create_buffer(&BufferDescriptor {
            label: Some("uniforms"),
            size: 16,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("line pipeline"),
            layout: None,
            vertex: VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: None,
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
            label: Some("line bind group"),
            layout: &layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &uniform_buffer,
                    offset: 0,
                    size: None,
                }),
            }],
        });

        self.write_uniform(&uniform_buffer, pixel_size);
        self.uniform_buffer = Some(uniform_buffer);
        self.pipeline = Some(pipeline);
        self.bind_group = Some(bind_group);
    }

    fn write_uniform(&self, uniform_buffer: &Buffer, pixel_size: (u32, u32)) {
        let data = [
            (pixel_size.0 as f32).to_le_bytes(),
            (pixel_size.1 as f32).to_le_bytes(),
            LINE_WIDTH.to_le_bytes(),
        ]
        .concat();
        self.queue.write_buffer(uniform_buffer, 0, &data);
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
        let (Some(pipeline), Some(bind_group)) = (self.pipeline.as_ref(), self.bind_group.as_ref())
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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(output);
        log::info!("render: frame presented");
    }

    /// Acquires the current surface texture.
    ///
    /// Returns the texture and whether the surface should be reconfigured because it
    /// is only suboptimal.
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

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
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
        pipeline: None,
        bind_group: None,
        uniform_buffer: None,
    };
    event_loop.run_app(&mut app)?;

    log::info!("event loop finished");
    Ok(())
}
