//! The letterbox regression: paints the blit's stretched quad over a surface
//! the way `render` does — the real `blit.wgsl`, the real `fit_bytes`
//! packing, a clear as black as the present pass's — and reads the frame
//! back to ask where the bars actually end. The claim it pins is one
//! sentence each: every column's black ends at the bar, and the bar is
//! black.
//!
//! Both halves were measured false once, and in a way no state-machine test
//! could see. A browser fullscreen (a 16:9 buffer on a taller display) showed
//! the top bar not as a clean bar but as a band of stretched pixels — the
//! blit's one oversized full-screen triangle, scaled by the fit, bulges its
//! hypotenuse above the content rectangle's top edge everywhere the
//! rectangle meets the surface's top, and the sampler's clamp paints the
//! buffer's top row across the whole band. Only where the hypotenuse at last
//! sinks below the surface's top does the cleared black appear: a diagonal
//! notch bitten into the bar's right end, photographed at the top of a
//! fullscreen hill. The quad `blit.wgsl` now builds — six vertices, four
//! real edges — cannot do this; this test is the pin.

use super::super::windowing::{content_fit, fit_bytes};
use crate::backend::app::block_on;
use crate::shaders::BLIT_SHADER;
use std::borrow::Cow;
use wgpu::*;

/// One painted frame stretched over a `surface` with the given letterbox fit,
/// read back as the surface's red channel, row-major from the top. `None`
/// when the machine has no GPU to ask, as in every other bridge here.
fn stretch(surface: (u32, u32), fit: (f32, f32)) -> Option<Vec<u32>> {
    let instance = Instance::default();
    let Ok(adapter) = block_on(instance.request_adapter(&RequestAdapterOptions::default())) else {
        eprintln!("letterbox bridge: no adapter on this machine — the bars go unverified!");
        return None;
    };
    let Ok((device, queue)) = block_on(adapter.request_device(&DeviceDescriptor::default())) else {
        eprintln!("letterbox bridge: no device — the bars go unverified!");
        return None;
    };
    let (w, h) = surface;

    // One white texel stands in for the frame buffer and for every dormant
    // bloom slot alike: coverage is the whole question, colour is the witness.
    let texel = device.create_texture(&TextureDescriptor {
        label: Some("letterbox texel"),
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
            texture: &texel,
            mip_level: 0,
            origin: Origin3d::default(),
            aspect: TextureAspect::All,
        },
        &[0xff, 0xff, 0xff, 0xff],
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let view = texel.create_view(&TextureViewDescriptor::default());
    let sampler = device.create_sampler(&SamplerDescriptor::default());
    let uniform = device.create_buffer(&BufferDescriptor {
        label: Some("letterbox fit"),
        size: 64,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(
        &uniform,
        0,
        &fit_bytes(fit, [0.0; 6], 0.0, 0.0, (0.0, 0.0), (-1.0, -1.0)),
    );

    let texture_entry = BindingType::Texture {
        sample_type: TextureSampleType::Float { filterable: true },
        view_dimension: TextureViewDimension::D2,
        multisampled: false,
    };
    let sampler_entry = BindingType::Sampler(SamplerBindingType::Filtering);
    let entry = |binding: u32, ty: BindingType| BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty,
        count: None,
    };
    let bgl = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("letterbox bgl"),
        entries: &[
            entry(0, texture_entry),
            entry(1, sampler_entry),
            // The fit is read by the vertex shader — the quad is scaled
            // there — and by the fragment's bloom guards, so its visibility
            // carries both stages.
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // The six bloom slots and their sampler: the weights are all
            // zero, so the shader never fetches — but the layout asks every
            // declared binding to be bound.
            entry(3, texture_entry),
            entry(4, texture_entry),
            entry(5, texture_entry),
            entry(6, texture_entry),
            entry(7, texture_entry),
            entry(8, texture_entry),
            entry(9, sampler_entry),
        ],
    });
    let bind = device.create_bind_group(&BindGroupDescriptor {
        label: Some("letterbox bind"),
        layout: &bgl,
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
                resource: uniform.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 3,
                resource: BindingResource::TextureView(&view),
            },
            BindGroupEntry {
                binding: 4,
                resource: BindingResource::TextureView(&view),
            },
            BindGroupEntry {
                binding: 5,
                resource: BindingResource::TextureView(&view),
            },
            BindGroupEntry {
                binding: 6,
                resource: BindingResource::TextureView(&view),
            },
            BindGroupEntry {
                binding: 7,
                resource: BindingResource::TextureView(&view),
            },
            BindGroupEntry {
                binding: 8,
                resource: BindingResource::TextureView(&view),
            },
            BindGroupEntry {
                binding: 9,
                resource: BindingResource::Sampler(&sampler),
            },
        ],
    });

    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("letterbox blit"),
        source: ShaderSource::Wgsl(Cow::Borrowed(BLIT_SHADER)),
    });
    let format = TextureFormat::Rgba8UnormSrgb;
    let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some("letterbox blit pipeline"),
        // An explicit layout, as the engine's blit pipeline has: the auto
        // layout would be a different, incompatible object from this bind
        // group layout.
        layout: Some(&device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("letterbox layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        })),
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
        // The defaults are the engine's: the blit pipeline comes from
        // `create_pipeline`, which sets nothing on the primitive state.
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let target = device.create_texture(&TextureDescriptor {
        label: Some("letterbox surface"),
        size: Extent3d {
            width: w,
            height: h,
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
            label: Some("letterbox blit pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &target.create_view(&TextureViewDescriptor::default()),
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
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        // The quad's six vertices, as `render` draws them.
        pass.draw(0..6, 0..1);
    }
    let pitch = (w * 4).div_ceil(256) * 256;
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("letterbox readback"),
        size: u64::from(pitch) * u64::from(h),
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
                bytes_per_row: Some(pitch),
                rows_per_image: Some(h),
            },
        },
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    readback
        .slice(..)
        .map_async(MapMode::Read, |r| r.expect("map"));
    let _ = device.poll(PollType::wait_indefinitely());
    let bytes = readback.get_mapped_range(..).expect("mapped").to_vec();
    readback.unmap();
    let mut red = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let i = usize::try_from(u64::from(y) * u64::from(pitch) + u64::from(x) * 4)
                .expect("in range");
            red.push(u32::from(bytes[i])); // the red channel: white or black
        }
    }
    Some(red)
}

/// Walks one column from the top and returns the row where the black ends.
fn first_white(shot: &[u32], surface: (u32, u32), x: u32) -> u32 {
    let (w, h) = surface;
    (0..h)
        .find(|&y| shot[(y * w + x) as usize] > 8)
        .unwrap_or(h)
}

/// A 16:9 buffer on two taller surfaces — the letterbox of the photographed
/// fullscreen and of a square one: the bar above the picture must end at the
/// same row in every column (no diagonal notch), and every pixel above it
/// must be the cleared black (no stretched row painted across the bar).
#[test]
fn the_letterbox_bars_are_flat_and_black() {
    // A 64×40 and a 40×40 stand in for the photographed 4084×2469 and
    // company: the geometry is scale-free, and a readback this size is a
    // rounding error next to the full surface.
    for surface in [(64, 40), (40, 40), (4084, 2469)] {
        let render_size = [1920, 1080];
        let fit = content_fit(Some(render_size), surface);
        assert!(
            fit.1 < 1.0 && (fit.0 - 1.0).abs() < 1e-6,
            "a letterbox surface was expected: {fit:?}"
        );
        let bar = ((1.0 - fit.1) * surface.1 as f32 / 2.0).round();
        let Some(shot) = stretch(surface, fit) else {
            return; // no GPU: the bars go unverified on this machine
        };
        let (w, h) = surface;
        let (mut worst, mut wedge_from) = (0u32, None);
        for x in 0..w {
            let at = first_white(&shot, surface, x);
            if at > worst {
                worst = at;
            }
            if at.abs_diff(bar as u32) > 1 && wedge_from.is_none() {
                wedge_from = Some((x, at));
            }
        }
        assert!(
            worst <= bar as u32 + 1,
            "the top bar is not flat on a {surface:?} surface: a column keeps black down to \
             row {worst} while the bar ends at {bar} (first such column: {wedge_from:?})"
        );
        assert!(
            wedge_from.is_none(),
            "the top bar floats off its row on a {surface:?} surface: column {wedge_from:?} \
             has white at a row the fit forbids (bar {bar}) — stretched pixels, not black"
        );
        // And the picture itself arrives: the centre is the buffer's white.
        assert!(
            shot[((h / 2) * w + w / 2) as usize] > 8,
            "the middle is not painted white: the blit painted nothing"
        );
    }
}
