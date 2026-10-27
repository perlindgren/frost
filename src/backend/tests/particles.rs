//! Particle instance packing in pixel space, and the whole-surface scissor.

use super::super::*;
use super::*;
use crate::objects::*;
use crate::{Canvas, Particle};

#[test]
fn particles_scissor_covers_the_whole_surface() {
    let draw = Draw::Particles {
        diagnostic: false,
        data: vec![],
        count: 0,
        color: black(),
        kind: 0.0,
        aspect: 1.0,
        sprite_data: None,
        sprite_size: [0, 0],
        sprite_generation: 0,
        lit: 0.0,
        z: 0.0,

        lit_layers: LightLayers::default(),
    };
    // The batch's particles can be anywhere, so the scissor is the whole
    // render area; the per-particle quads are already tight.
    assert_eq!(draw.scissor_rect([100, 100]), Some([0, 0, 100, 100]));
}

#[test]
// The test exercises the deprecated immediate particle draw itself.
#[allow(deprecated)]
fn canvas_particles_packs_instances_in_pixel_space() {
    let mut canvas = Canvas::new((100, 100), 1.0);
    let tint = Color {
        r: 0.5,
        g: 0.25,
        b: 0.75,
        a: 0.5,
    };
    let particles = [
        // A live particle with half its lifetime left.
        Particle {
            pos: [10.0, 20.0],
            vel: [0.0, 0.0],
            life: 2.0,
            max_life: 4.0,
            size: 3.0,
            angle: 0.0,
            color: tint,
        },
        // A dead one: its life fraction clamps to zero.
        Particle {
            pos: [0.0, 0.0],
            vel: [0.0, 0.0],
            life: -1.0,
            max_life: 2.0,
            size: 1.0,
            angle: 0.0,
            color: tint,
        },
        // An over-living one: its life clamps to one, and its negative
        // size clamps to zero.
        Particle {
            pos: [-10.0, -10.0],
            vel: [0.0, 0.0],
            life: 6.0,
            max_life: 4.0,
            size: -2.0,
            angle: 0.0,
            color: tint,
        },
        // No max lifetime: the life fraction is zero, so it fades out
        // completely.
        Particle {
            pos: [5.0, -5.0],
            vel: [0.0, 0.0],
            life: 1.0,
            max_life: 0.0,
            size: 2.0,
            angle: 0.0,
            color: tint,
        },
    ];
    canvas.particles(&particles, tint, 2.5);
    let [
        Draw::Particles {
            data,
            count,
            color,
            z,
            ..
        },
    ] = &canvas.draws[..]
    else {
        panic!("expected one particle draw");
    };
    assert_eq!(data.len(), 4 * 32);
    assert_eq!(*count, 4);
    assert_eq!(*color, tint);
    assert_eq!(*z, 2.5);

    // Each particle packs two vec4s in pixel space (top-left origin, y
    // down): (px, py, size, life fraction), then (angle, tint r, tint g,
    // tint b).
    let f32_at = |particle: usize, field: usize| {
        let off = particle * 32 + field * 4;
        f32::from_le_bytes(data[off..off + 4].try_into().unwrap())
    };
    // user (10, 20) is (60, 30) in pixel space on a 100x100 window.
    assert_eq!(f32_at(0, 0), 60.0);
    assert_eq!(f32_at(0, 1), 30.0);
    assert_eq!(f32_at(0, 2), 3.0);
    assert_eq!(f32_at(0, 3), 0.5);
    assert_eq!(f32_at(1, 0), 50.0);
    assert_eq!(f32_at(1, 1), 50.0);
    assert_eq!(f32_at(1, 2), 1.0);
    assert_eq!(f32_at(1, 3), 0.0); // negative life clamps to zero
    // user (-10, -10) is (40, 60) in pixel space.
    assert_eq!(f32_at(2, 0), 40.0);
    assert_eq!(f32_at(2, 1), 60.0);
    assert_eq!(f32_at(2, 2), 0.0); // negative size clamps to zero
    assert_eq!(f32_at(2, 3), 1.0); // over-living clamps to one
    assert_eq!(f32_at(3, 0), 55.0);
    assert_eq!(f32_at(3, 1), 55.0);
    assert_eq!(f32_at(3, 2), 2.0);
    assert_eq!(f32_at(3, 3), 0.0); // no max lifetime means no life fraction
    // The angle is zero for every particle (circles), and the packed
    // tint is the particle's own color.
    for p in 0..4 {
        assert_eq!(f32_at(p, 4), 0.0);
        assert_eq!(f32_at(p, 5), tint.r);
        assert_eq!(f32_at(p, 6), tint.g);
        assert_eq!(f32_at(p, 7), tint.b);
    }
}

#[test]
// The test exercises the deprecated immediate particle draw itself.
#[allow(deprecated)]
fn canvas_particles_with_no_particles_adds_no_draw() {
    let mut canvas = Canvas::new((100, 100), 1.0);
    canvas.particles(&[], black(), 0.0);
    assert!(canvas.draws.is_empty());
}

/// The lit-particle path measured on the GPU, the way the shadow bridge
/// measures the lit-shape path: the antifrost leaves claim they "hang in
/// the lit world — night dims them, an ant's lamp picks them out as it
/// passes under", and that claim lives in `particles.wgsl`'s `lit` branch,
/// invisible to every CPU-side packing test. So: pack a real light field,
/// render a real circle-particle batch through the real shader, read the
/// pixels.
///
/// The scene makes the two claims distinguishable. Under a black ambient
/// and one lamp of radius 20, a *lit* batch is a pool of light around the
/// lamp fading into the dark; an *unlit* batch (the same draw with the
/// uniform's flag down) is a flat white sheet that ignores the lamp
/// entirely. One number per scene, red channel: the center sits in the
/// pool, the corner sits outside the lamp's reach.
#[test]
fn a_lit_particle_batch_is_picked_out_by_the_lamp() {
    use crate::shaders::PARTICLES_SHADER;
    use std::borrow::Cow;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use wgpu::util::{BufferInitDescriptor, DeviceExt};
    use wgpu::*;

    // 64 keeps every readback row a 256-byte multiple, as the copy demands.
    const SPAN: u32 = 64;

    // The scene is built from two colors: a white for the batch and the
    // lamp, and the black of the night ambient.
    let white = crate::objects::Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    let paint = |lit: f32| -> Option<Vec<u8>> {
        let lamp = Draw::Light {
            pos: [32.0, 32.5],
            radius: 20.0,
            intensity: 1.0,
            color: white,
            penumbra: 0.0,
            dir: [1.0, 0.0],
            cos_half: -1.0,
            feather: 0.0,
            z: 0.0,

            layers: LightLayers::default(),
        };
        let lights = pack_light_field(&[lamp], black());

        // The thread's shared GPU: see [`super::machine`].
        let machine = super::machine::machine()?;
        let (device, queue) = (&machine.device, &machine.queue);

        // One circle particle: centered on the lamp, its half-size (60)
        // larger than the whole target, at full life. Then the (angle, tint)
        // vec4: no spin, white.
        let mut instances = Vec::new();
        for v in [32.0f32, 32.5, 60.0, 1.0, 0.0, 1.0, 1.0, 1.0] {
            instances.extend_from_slice(&v.to_le_bytes());
        }
        // Padded to 256 bytes: a binding under the adapter's minimum
        // storage size is a validation error, and the shader's index
        // arithmetic never walks into the padding.
        instances.resize(256, 0);
        let mut light_bytes = lights.data;
        light_bytes.resize(256, 0);
        let occluder_bytes = vec![0u8; 256]; // count 0: nothing casts

        let instance_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("particle bridge instances"),
            contents: &instances,
            usage: BufferUsages::STORAGE,
        });
        let light_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("particle bridge lights"),
            contents: &light_bytes,
            usage: BufferUsages::STORAGE,
        });
        let occluder_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("particle bridge occluders"),
            contents: &occluder_bytes,
            usage: BufferUsages::STORAGE,
        });
        let uniform_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("particle bridge uniform"),
            contents: &particles_uniform_data(
                [SPAN as f32, SPAN as f32],
                white,
                0.0,
                1.0,
                lit,
                LightLayers::default(),
            ),
            usage: BufferUsages::UNIFORM,
        });
        // The circle kind never samples the texture; a 1x1 white placeholder
        // fills the binding the pipeline layout still demands.
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("particle bridge placeholder"),
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: Origin3d::default(),
                aspect: TextureAspect::All,
            },
            &[255, 255, 255, 255],
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
        let view = texture.create_view(&TextureViewDescriptor::default());
        let sampler = device.create_sampler(&SamplerDescriptor::default());
        // The shared unit-quad index pattern, the same [0,1,2,2,1,3] the
        // render loop's batch walks.
        let index_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("particle bridge indices"),
            contents: &le_bytes([0u32, 1, 2, 2, 1, 3]),
            usage: BufferUsages::INDEX,
        });

        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("particle bridge"),
            source: ShaderSource::Wgsl(Cow::Borrowed(PARTICLES_SHADER)),
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("particle bridge pipeline"),
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
                    format: TextureFormat::Rgba8Unorm,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("particle bridge bind"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: instance_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&view),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Sampler(&sampler),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: light_buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: occluder_buffer.as_entire_binding(),
                },
            ],
        });

        let target = device.create_texture(&TextureDescriptor {
            label: Some("particle bridge target"),
            size: Extent3d {
                width: SPAN,
                height: SPAN,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("particle bridge pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &target.create_view(&TextureViewDescriptor::default()),
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 0.0,
                        }),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.set_index_buffer(index_buffer.slice(..), IndexFormat::Uint32);
            pass.draw_indexed(0..6, 0, 0..1);
        }
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some("particle bridge readback"),
            size: u64::from(SPAN) * u64::from(SPAN) * 4,
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
                    bytes_per_row: Some(SPAN * 4),
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
        super::machine::drain(device);
        assert_eq!(done.load(Ordering::SeqCst), 1, "the readback never mapped");
        let shot = readback
            .get_mapped_range(..)
            .expect("readback mapped")
            .to_vec();
        readback.unmap();
        Some((0..SPAN * SPAN).map(|i| shot[4 * i as usize]).collect())
    };

    let Some(lit) = paint(1.0) else { return };
    let Some(unlit) = paint(0.0) else { return };
    let (center, corner) = (32 * SPAN + 32, 4 * SPAN + 4);
    // A lit batch hangs in the light field: bright under the lamp, dark
    // beyond its reach — the leaf drifting through a lamp pool.
    assert!(
        lit[center as usize] > 150 && lit[corner as usize] < 40,
        "a lit batch should pool light around the lamp: center {}, corner {}",
        lit[center as usize],
        lit[corner as usize]
    );
    // An unlit batch is its own flat color everywhere: the lamp changes
    // nothing, which is exactly what the uniform's flag must switch off.
    assert!(
        unlit[center as usize] > 150 && unlit[corner as usize] > 150,
        "an unlit batch should ignore the lamp entirely: center {}, corner {}",
        unlit[center as usize],
        unlit[corner as usize]
    );
}

/// Little-endian bytes of a fixed array of `u32`s, for index buffers.
fn le_bytes<const N: usize>(words: [u32; N]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
