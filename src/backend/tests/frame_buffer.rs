//! The frame buffer's format, pinned to what a window can actually show: the
//! same draws painted into 8-bit sRGB and into the `Rgba16Float` buffer the
//! engine now prefers, each read back and put through the transfer curve the
//! present pass applies, so the two are comparable as bytes the display would
//! receive.
//!
//! The float buffer is a change to every frame with no visible change, and a
//! claim like that needs a test rather than an argument. Two things fall out.
//! In-range compositing is untouched — an sRGB attachment is decoded before
//! hardware blends, so the old buffer never composited in encoded values and
//! only ever *stored* them at 8 bits — so the worst a pixel can move is one
//! step, and only where many low-alpha washes stack and every store rounds.
//! (That one step is the calibrated GPU's measure: the store's sRGB
//! conversion is the implementation's own, so a CPU rasterizer is held to
//! its own measured bound — the errata in BUGS.md carries the number.)
//! And over-range stays invisible: a base at 2.4 clamps at the attachment or
//! at the window and reaches the same 255 either way. That second half is the
//! caveat to read before arguing the float buffer is an improvement on its
//! own. Its range is worth something only to a pass that looks for it — a
//! bloom bright-pass, a tone map — and until one exists the buffer buys the
//! right to keep a value above 1.0, and nothing you can see.

#[test]
fn float_and_srgb_frame_buffers_agree_where_the_window_can_tell() {
    use crate::backend::app::{block_on, sprite_layouts};
    use crate::backend::frame::{SPRITE_UNIFORM_BYTES, sprite_uniform_data};
    use crate::objects::{Color, LightLayers, Transform};
    use crate::shaders::SPRITE_SHADER;
    use std::borrow::Cow;
    use std::num::NonZeroU64;
    use wgpu::*;

    const SPAN: u32 = 64;
    let black = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    let off = black;

    let instance = Instance::default();
    let adapter =
        block_on(instance.request_adapter(&RequestAdapterOptions::default())).expect("adapter");
    let (device, queue) =
        block_on(adapter.request_device(&DeviceDescriptor::default())).expect("device");

    // Which adapter answered decides the two bounds this test cannot fix
    // from the WGSL side: the sRGB store conversion is the
    // implementation's own, so one store can land a code off the
    // reference curve and twenty stacked washes amplify that into a run
    // the final encode cannot undo. The calibrated GPU's fixed-function
    // unit reproduces the curve exactly on a single store and moves a
    // pixel at most one step even through the washes; llvmpipe converts
    // on its own — measured one step on single stores and three through
    // the stack (BUGS.md) — so it gets those measurements with a step of
    // margin on the accumulation. Everything else stays exact for every
    // adapter.
    let info = adapter.get_info();
    let software = info.device_type == DeviceType::Cpu
        || [
            "llvmpipe",
            "swrast",
            "softpipe",
            "swiftshader",
            "basic render",
        ]
        .iter()
        .any(|mark| info.name.to_lowercase().contains(mark));
    let worst_allowed = if software { 4 } else { 1 };
    let outside_allowed = if software { 1 } else { 0 };

    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("sprites"),
        source: ShaderSource::Wgsl(Cow::Borrowed(SPRITE_SHADER)),
    });
    let (_bind_layout, layout) = sprite_layouts(&device);

    let pipeline = |format: TextureFormat| {
        device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("sprite pipeline"),
            layout: Some(&layout),
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
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    };

    // A stack that exercises exactly what the frame buffer format decides:
    // an opaque base, a translucent pane over it, a second translucent pane
    // over that (two blend steps), and a bright pair whose sum exceeds 1.0
    // (the over-range the 8-bit buffer cannot hold).
    let box_at = |center: [f32; 2], size: [f32; 2], tint: Color| {
        sprite_uniform_data(
            Transform::translate(center)
                .invert()
                .expect("a translation"),
            size,
            tint,
            1.0,
            off,
            None,
            0.0,
            LightLayers::default(),
            [0.0, 0.0, 1.0, 1.0],
        )
    };
    let c = |r: f32, g: f32, b: f32, a: f32| Color { r, g, b, a };
    let stack = vec![
        // Opaque base covering the whole target.
        box_at([32.0, 32.0], [64.0, 64.0], c(0.02, 0.02, 0.03, 1.0)),
        // A soft pane: 40% white over the base, then again at 40%.
        box_at([16.0, 16.0], [24.0, 24.0], c(0.7, 0.7, 0.7, 0.4)),
        box_at([16.0, 16.0], [24.0, 24.0], c(0.7, 0.7, 0.7, 0.4)),
        // Two lights stacked: each under 1.0, their sum well over.
        box_at([48.0, 16.0], [24.0, 24.0], c(0.8, 0.5, 0.1, 1.0)),
        box_at([48.0, 16.0], [24.0, 24.0], c(0.0, 0.6, 0.7, 0.9)),
        // Twenty successive 5% washes: the accumulation case, where an
        // 8-bit buffer rounds at every step and a float buffer at none.
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        box_at([16.0, 48.0], [24.0, 24.0], c(0.25, 0.45, 0.9, 0.05)),
        // A base the 8-bit buffer cannot hold, then a wash over it: the
        // over-range case, where the clamp either happens early or late.
        box_at([48.0, 48.0], [24.0, 24.0], c(2.4, 1.8, 0.6, 1.0)),
        box_at([48.0, 48.0], [24.0, 24.0], c(1.0, 1.0, 1.0, 0.15)),
    ];

    let align = device.limits().min_uniform_buffer_offset_alignment as u64;
    let stride = SPRITE_UNIFORM_BYTES.max(align).next_multiple_of(align);
    let pad = stride as usize - SPRITE_UNIFORM_BYTES as usize;
    let mut staging = Vec::new();
    for bytes in &stack {
        staging.extend_from_slice(bytes);
        staging.resize(staging.len() + pad, 0);
    }

    let atlas = device.create_texture(&TextureDescriptor {
        label: Some("ab atlas"),
        size: Extent3d {
            width: 16,
            height: 16,
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
            texture: &atlas,
            mip_level: 0,
            origin: Origin3d::default(),
            aspect: TextureAspect::All,
        },
        &vec![255u8; 16 * 16 * 4],
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(16 * 4),
            rows_per_image: Some(16),
        },
        Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
    );
    let sheet = atlas.create_view(&TextureViewDescriptor::default());
    let sampler = device.create_sampler(&SamplerDescriptor {
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        ..Default::default()
    });
    let fields = device.create_buffer(&BufferDescriptor {
        label: Some("ab fields"),
        size: 48,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let render = |format: TextureFormat| -> Vec<u8> {
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("stack"),
            size: stride * stack.len() as u64,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        buffer
            .slice(..)
            .get_mapped_range_mut()
            .expect("mapped")
            .copy_from_slice(&staging);
        buffer.unmap();
        let pipeline = pipeline(format);
        let bind = device.create_bind_group(&BindGroupDescriptor {
            label: Some("stack"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &buffer,
                        offset: 0,
                        size: NonZeroU64::new(SPRITE_UNIFORM_BYTES),
                    }),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&sheet),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&sampler),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &fields,
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &fields,
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        });
        let bpp: u64 = if format == TextureFormat::Rgba16Float {
            8
        } else {
            4
        };
        let target = device.create_texture(&TextureDescriptor {
            label: Some("ab target"),
            size: Extent3d {
                width: SPAN,
                height: SPAN,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("ab pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &target.create_view(&TextureViewDescriptor::default()),
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            for (i, _) in stack.iter().enumerate() {
                pass.set_bind_group(0, &bind, &[(i as u64 * stride) as u32]);
                pass.draw(0..3, 0..1);
            }
        }
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("ab readback"),
            size: u64::from(SPAN * SPAN) * bpp,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: Origin3d::default(),
                aspect: TextureAspect::All,
            },
            TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SPAN * bpp as u32),
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
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let done = Arc::new(AtomicBool::new(false));
        {
            let done = Arc::clone(&done);
            readback.slice(..).map_async(MapMode::Read, move |r| {
                r.expect("map");
                done.store(true, Ordering::SeqCst);
            });
        }
        let _ = device.poll(PollType::wait_indefinitely());
        assert!(done.load(Ordering::SeqCst));
        let out = readback.get_mapped_range(..).expect("mapped").to_vec();
        readback.unmap();
        out
    };

    /// The half-float of `Rgba16Float`, back to a float.
    fn half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
        let exp = ((bits >> 10) & 0x1f) as i32;
        let frac = (bits & 0x3ff) as f32;
        match exp {
            0 => sign * frac * 2f32.powi(-24),
            31 => f32::INFINITY,
            _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
        }
    }
    /// The transfer curve the present pass applies, and the 8-bit rounding
    /// the window imposes — the same `encode_srgb` blit.wgsl carries.
    fn encode(x: f32) -> u8 {
        let e = if x <= 0.0031308 {
            x * 12.92
        } else {
            1.055 * x.max(0.0).powf(1.0 / 2.4) - 0.055
        };
        (e.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    let srgb = render(TextureFormat::Rgba8UnormSrgb);
    let raw = render(TextureFormat::Rgba16Float);
    let mut float_bytes = vec![0u8; (SPAN * SPAN * 4) as usize];
    for i in 0..(SPAN * SPAN) as usize {
        let at = i * 4;
        for ch in 0..3 {
            let bits = u16::from_le_bytes([raw[at * 2 + ch * 2], raw[at * 2 + ch * 2 + 1]]);
            float_bytes[at + ch] = encode(half(bits));
        }
        let ab = u16::from_le_bytes([raw[at * 2 + 6], raw[at * 2 + 7]]);
        float_bytes[at + 3] = encode(half(ab));
    }

    let mut n = 0usize;
    let mut worst = 0u8;
    let mut worst_outside = 0u8;
    let mut where_worst = (0u32, 0u32);
    for i in 0..(SPAN * SPAN) as usize {
        let at = i * 4;
        let d = (0..3)
            .map(|ch| srgb[at + ch].abs_diff(float_bytes[at + ch]))
            .max()
            .unwrap_or(0);
        if d > 0 {
            n += 1;
        }
        if d > worst {
            worst = d;
            where_worst = ((i as u32) % SPAN, (i as u32) / SPAN);
        }
        let (x, y) = ((i as u32) % SPAN, (i as u32) / SPAN);
        let wash = (4..28).contains(&x) && (36..60).contains(&y);
        if !wash && d > worst_outside {
            worst_outside = d;
        }
    }
    let at = |x: u32, y: u32| {
        let i = ((y * SPAN + x) * 4) as usize;
        (
            [srgb[i], srgb[i + 1], srgb[i + 2]],
            [float_bytes[i], float_bytes[i + 1], float_bytes[i + 2]],
        )
    };
    // The worst a pixel may move is one step of 8-bit output — on a GPU;
    // a CPU rasterizer holds to its measured bound instead.
    let (base_srgb, base_float) = at(where_worst.0, where_worst.1);
    assert!(
        worst <= worst_allowed,
        "the frame buffer's format moved a pixel by {worst}/255 at \
         {where_worst:?} on '{}' (a {worst_allowed}-step bound): srgb \
         {base_srgb:?} against float {base_float:?}",
        info.name
    );
    // ...and only inside the stacked washes, the one place a rounding per
    // store can accumulate. Outside them every pixel is a single store:
    // the calibrated GPU reproduces the curve bit for bit, and the CPU
    // rasterizer measured one code away from it on the dark base and
    // identical everywhere a blend was involved (BUGS.md). Anywhere else,
    // the two buffers are the same frame.
    assert!(
        worst_outside <= outside_allowed,
        "{worst_outside} steps off outside the wash on '{}': outside the stacked \
         washes the two buffers must be the same frame (this adapter's bound: \
         {outside_allowed})",
        info.name
    );
    // The over-range base reaches the window identically, because both paths
    // clamp before it: the float buffer's extra range is kept, not shown.
    // If this stops holding, something started to read that range — which is
    // the point of the buffer, and the day this claim needs re-arguing.
    assert_eq!(
        at(48, 48),
        ([255, 255, 212], [255, 255, 212]),
        "an over-range base reached the window differently, so the clamp moved"
    );
    assert!(
        n > 0,
        "the two buffers agreed exactly; the wash stack stopped exercising the rounding it is here for"
    );
}
