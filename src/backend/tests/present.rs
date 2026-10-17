//! The present path, painted: the scene pass into a float frame buffer and its
//! emissive target, the real [`Bloom`] chain over that, and the real
//! `blit.wgsl` compositing the result into a window-sized float target, which
//! is then read back a half-float at a time.
//!
//! Every test that asks *what does the screen hold* comes through here,
//! because bloom and the tone map are both defined by what arrives away from
//! the shape that produced them — bloom in the pixels beside a lamp, a tone
//! map in what a value past white turns into — and no test that reads the
//! pixel inside a plate can see either one. It lives on its own rather than
//! inside the bloom tests for that reason: two topics read it now, and a
//! harness in one of their files would make the other one reach.
//!
//! It does not share emission.rs's. That one hands back the bytes of a single
//! target, and this one needs the targets alive afterwards to post-process
//! them; bending it into shape would have made both tests harder to read than
//! the duplication they are paying for.

use super::super::*;
use crate::backend::app::block_on;
use crate::backend::bloom::Bloom;
use crate::canvas::Occluder;
use crate::objects;
use crate::objects::*;
use crate::shaders::{BLIT_SHADER, SHAPE_SHADER};
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::*;
pub(crate) const SPAN: u32 = 64;
/// The centre, and the plate's size in it.
pub(crate) const MIDDLE: f32 = 32.0;

/// The one plate this scene has: a 12-pixel square of amber at the centre,
/// unlit, putting out twice its own colour. Unlit so that the frame's own
/// pixels are exactly the plate's colour and nothing else — the only light in
/// the frame is the light the bloom chain moves.
pub(crate) fn lamp(emission: f32) -> Draw {
    lamp_at([MIDDLE, MIDDLE], emission)
}

/// The same plate anywhere else in the frame, for the picture test's scene.
pub(crate) fn lamp_at(at: [f32; 2], emission: f32) -> Draw {
    Draw::Shape {
        world: Transform::identity(),
        center: at,
        params: [6.0, 6.0],
        kind: 1.0,
        aa: 1.0,
        color: objects::Color {
            r: 0.4,
            g: 0.2,
            b: 0.1,
            a: 1.0,
        },
        emission: objects::Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: emission,
        },
        glow: None,
        lit: 0.0,
        occludes: 0.0,
        z: 0.0,
        diagnostic: false,
    }
}

/// The frame a window would have held: `draws` painted, the emissive target
/// bloomed at `strength`, and the pair composited by the present pass.
///
/// The composite target is `Rgba16Float` rather than the window's 8 bits,
/// because what is being measured here is a faint addition a long way from the
/// lamp, and quantising to 1/255 before asking whether it is there would be
/// measuring the format rather than the effect.
pub(crate) fn frame(
    draws: &[Draw],
    strength: f32,
    tone: f32,
    dump: Option<&str>,
    override_weights: Option<[f32; crate::backend::bloom::MAX_LEVELS]>,
) -> Option<Vec<[f32; 3]>> {
    let ambient = objects::Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    let lights = pack_light_field(draws, ambient);
    let occluders = pack_occluder_field(draws, &[] as &[Occluder], [SPAN as f32, SPAN as f32]);

    let instance = Instance::default();
    let Ok(adapter) = block_on(instance.request_adapter(&RequestAdapterOptions::default())) else {
        eprintln!("bloom: no adapter on this machine — the test goes unverified!");
        return None;
    };
    let Ok((device, queue)) = block_on(adapter.request_device(&DeviceDescriptor::default())) else {
        eprintln!("bloom: no device — the test goes unverified!");
        return None;
    };
    let padded = |field: &Vec<u8>| {
        let mut bytes = field.clone();
        if bytes.len() < 256 {
            bytes.resize(256, 0);
        }
        bytes
    };
    let light_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label: Some("bloom lights"),
        contents: &padded(&lights.data),
        usage: BufferUsages::STORAGE,
    });
    let occluder_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label: Some("bloom occluders"),
        contents: &padded(&occluders.data),
        usage: BufferUsages::STORAGE,
    });

    // The scene pass, exactly as the engine configures it when the frame has an
    // emissive target: two float outputs, blended, `fs_emissive`.
    let scene_module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("bloom scene"),
        source: ShaderSource::Wgsl(Cow::Borrowed(SHAPE_SHADER)),
    });
    let float = TextureFormat::Rgba16Float;
    let scene_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some("bloom scene pipeline"),
        layout: None,
        vertex: VertexState {
            module: &scene_module,
            entry_point: Some("vs_main"),
            compilation_options: PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(FragmentState {
            module: &scene_module,
            entry_point: Some("fs_emissive"),
            compilation_options: PipelineCompilationOptions::default(),
            targets: &[
                Some(ColorTargetState {
                    format: float,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                }),
                Some(ColorTargetState {
                    format: float,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                }),
            ],
        }),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let new_target = |label: &str, extra: TextureUsages| {
        device.create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width: SPAN,
                height: SPAN,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: float,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC | extra,
            view_formats: &[],
        })
    };
    let frame = new_target("bloom frame", TextureUsages::TEXTURE_BINDING);
    let emissive = new_target("bloom emissive", TextureUsages::TEXTURE_BINDING);
    let frame_view = frame.create_view(&TextureViewDescriptor::default());
    let emissive_view = emissive.create_view(&TextureViewDescriptor::default());

    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    {
        let clear = Operations {
            load: LoadOp::Clear(wgpu::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }),
            store: StoreOp::Store,
        };
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("bloom scene pass"),
            color_attachments: &[
                Some(RenderPassColorAttachment {
                    view: &frame_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: clear,
                }),
                Some(RenderPassColorAttachment {
                    view: &emissive_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: clear,
                }),
            ],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&scene_pipeline);
        for draw in draws {
            let Draw::Shape {
                world,
                center,
                params,
                kind,
                aa,
                color,
                emission,
                glow,
                lit,
                ..
            } = draw
            else {
                continue;
            };
            let inv = world.invert().expect("a non-degenerate world");
            let uniform = device.create_buffer_init(&BufferInitDescriptor {
                label: Some("bloom plate uniform"),
                contents: &shape_uniform_data(
                    inv, *center, *params, *kind, *aa, *color, *emission, *glow, *lit,
                ),
                usage: BufferUsages::UNIFORM,
            });
            let bind = device.create_bind_group(&BindGroupDescriptor {
                label: Some("bloom plate bind"),
                layout: &scene_pipeline.get_bind_group_layout(0),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::Buffer(BufferBinding {
                            buffer: &uniform,
                            offset: 0,
                            size: None,
                        }),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Buffer(BufferBinding {
                            buffer: &light_buffer,
                            offset: 0,
                            size: None,
                        }),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::Buffer(BufferBinding {
                            buffer: &occluder_buffer,
                            offset: 0,
                            size: None,
                        }),
                    },
                ],
            });
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    // The chain itself, and the present pass that adds it back: the same two
    // pieces of the engine, in the same order, at the same sizes.
    // Built the way the engine builds it now, which is not the way `Bloom::new`
    // reads if you only want a chain: it comes up dormant and is woken by the
    // strength this frame asks for. Every bloom and tone test in the tree
    // therefore runs the same path a mid-run `Context::set_bloom` takes, and a
    // wake that produced an empty chain would darken every one of them.
    let mut bloom = Bloom::new(
        &device,
        &queue,
        float,
        [SPAN, SPAN],
        Some(&emissive_view),
        0.0,
    );
    bloom.set_strength(strength);
    if bloom.needs_building() {
        bloom.build(&device, &queue, float, [SPAN, SPAN], &emissive_view);
    }
    bloom.run(&mut encoder);

    let blit_module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("bloom present"),
        source: ShaderSource::Wgsl(Cow::Borrowed(BLIT_SHADER)),
    });
    let blit_pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some("bloom present pipeline"),
        layout: None,
        vertex: VertexState {
            module: &blit_module,
            entry_point: Some("vs_main"),
            compilation_options: PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(FragmentState {
            module: &blit_module,
            entry_point: Some("fs_main"),
            compilation_options: PipelineCompilationOptions::default(),
            targets: &[Some(ColorTargetState {
                format: float,
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
    let sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("bloom frame sampler"),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..Default::default()
    });
    let fit_buffer = device.create_buffer(&BufferDescriptor {
        label: Some("bloom fit"),
        size: 64,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let fit = crate::backend::windowing::fit_bytes(
        (1.0, 1.0),
        override_weights.unwrap_or(bloom.weights),
        tone,
    );
    queue.write_buffer(&fit_buffer, 0, &fit);
    let window = new_target("bloom window", TextureUsages::empty());
    let window_view = window.create_view(&TextureViewDescriptor::default());
    let mut entries = vec![
        BindGroupEntry {
            binding: 0,
            resource: BindingResource::TextureView(&frame_view),
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
    ];
    entries.extend(bloom.views.iter().enumerate().map(|(i, v)| BindGroupEntry {
        binding: 3 + i as u32,
        resource: BindingResource::TextureView(v),
    }));
    // These entries are the engine's own, copied: `App::blit_bind_group`
    // builds the same shape and cannot be called from here without an
    // `App`. wgpu counts the entries against the layout the shader generated,
    // so when the plumbing widens this block has to widen with it and the
    // count mismatch is the error.
    entries.push(BindGroupEntry {
        binding: 3 + crate::backend::bloom::MAX_LEVELS as u32,
        resource: BindingResource::Sampler(&bloom.sampler),
    });
    let bind = device.create_bind_group(&BindGroupDescriptor {
        label: Some("bloom present bind"),
        layout: &blit_pipeline.get_bind_group_layout(0),
        entries: &entries,
    });
    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("bloom present pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &window_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(wgpu::Color::BLACK),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&blit_pipeline);
        pass.set_bind_group(0, &bind, &[]);
        // The letterbox quad's six vertices, as `render` draws them.
        pass.draw(0..6, 0..1);
    }

    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("bloom readback"),
        size: u64::from(SPAN) * u64::from(SPAN) * 8,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        TexelCopyTextureInfo {
            texture: &window,
            mip_level: 0,
            origin: Origin3d::default(),
            aspect: TextureAspect::All,
        },
        TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SPAN * 8),
                rows_per_image: Some(SPAN),
            },
        },
        Extent3d {
            width: SPAN,
            height: SPAN,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let done = Arc::new(AtomicU32::new(0));
    let signal = done.clone();
    readback.slice(..).map_async(MapMode::Read, move |_| {
        signal.fetch_add(1, Ordering::SeqCst);
    });
    let _ = device.poll(PollType::wait_indefinitely());
    assert_eq!(done.load(Ordering::SeqCst), 1, "the readback never mapped");
    let bytes = readback
        .get_mapped_range(..)
        .expect("readback mapped")
        .to_vec();
    readback.unmap();
    let shot: Vec<[f32; 3]> = (0..(SPAN * SPAN) as usize)
        .map(|i| {
            let half = |c: usize| {
                let r = bytes[i * 8 + c * 2..i * 8 + c * 2 + 2]
                    .try_into()
                    .expect("a half-float channel");
                super::half_to_f32(u16::from_le_bytes(r))
            };
            [half(0), half(1), half(2)]
        })
        .collect();
    if let Some(path) = dump {
        // An 8-bit picture of what the frame holds, gamma-encoded for looking
        // at rather than for measuring: the numbers above are the ones tests
        // read, and this file exists so a human can see the same frame.
        let mut bytes = Vec::with_capacity(4 + SPAN as usize * SPAN as usize * 3);
        bytes.extend_from_slice(b"P6\n64 64\n255\n");
        for c in &shot {
            bytes.extend(c.iter().map(|v| {
                let v = v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0;
                v.round().clamp(0.0, 255.0) as u8
            }));
        }
        std::fs::write(path, bytes).expect("the dump path is writable");
    }
    Some(shot)
}
