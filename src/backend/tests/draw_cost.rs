//! What a draw call costs the CPU, measured rather than assumed: a fresh
//! uniform buffer and bind group per draw — the shape every draw in
//! [`crate::backend::app`] uses today — against one arena buffer and one
//! bind group for the whole frame, with the same shader, the same
//! geometry, and the same pixels.
//!
//! The question is narrow on purpose. The engine's rationale for a buffer
//! per draw is sound where it was written (`write_buffer` copies all land
//! before any draw executes, so two draws reading one buffer would both
//! read the last write), but a *dynamic* offset binding is the exception
//! the API provides for exactly this: one buffer, one bind group, and a
//! per-draw offset that moves the uniform window. If that costs the frame
//! nothing, the per-draw allocations are the frame's to keep or drop — and
//! `Diagnostics`' `PROC` line says they are worth something: a demo whose
//! whole scene is a spinning square, a handful of shapes, and an overlay of
//! a few hundred glyph quads spends milliseconds a frame on nothing else.
//!
//! So the bench paints the same field of glyph-like quads twice. Scheme
//! `per-draw` is the render loop's own recipe, line for line: `create_buffer`,
//! `write_buffer`, `create_bind_group`, a scissor, a full-screen triangle.
//! Scheme `arena` packs every draw's uniform bytes into one vector, uploads
//! it once, and per draw calls `set_bind_group` with that draw's offset.
//! Both build their uniform bytes per frame — that CPU work belongs to the
//! scene, not to the scheme, and both pay it.
//!
//! Two numbers per scheme. `frame` drains the device after every frame, as
//! the real loop's present does: it is the whole cost. `submit` measures
//! encode-and-submit with a single drain at the end, which is where the
//! per-draw allocations show undiluted — the number that lives inside
//! `PROC`, whose window closes before the frame waits for vsync.
//!
//! The GPU work is the same in both by construction: the same scissor
//! boxes, the same full-screen triangles, the same texel fetches. This
//! bench therefore says nothing about the *other* batching idea — one
//! instanced draw per run — because that one moves work off the CPU by
//! changing what the GPU rasterises, and needs its own measurement and its
//! own argument.
//!
//! ```text
//! cargo test --lib draw_cost -- --ignored --nocapture
//! ```

/// The arena's one hazard, tested: two draws sharing one uniform buffer must
/// still paint their own parameters. `write_buffer` copies all land before
/// any draw executes — the reason the engine gave every draw its own buffer
/// — and the arena answers that by giving each draw its own *slot*, reached
/// by a dynamic offset. If the slots were ever to overlap, or the window to
/// bind more than a slot, or the offsets to be computed against a different
/// stride than the upload used, both quads below would come out the second
/// draw's colour, and this test is where that shows.
#[test]
fn an_arena_slot_keeps_two_draws_apart() {
    use crate::backend::app::{block_on, sprite_layouts};
    use crate::backend::frame::{SPRITE_UNIFORM_BYTES, sprite_uniform_data};
    use crate::objects::{Color, Transform};
    use crate::shaders::SPRITE_SHADER;
    use std::borrow::Cow;
    use std::num::NonZeroU64;
    use wgpu::*;

    /// Two sprite boxes, 16 pixels on a side, far enough apart that one
    /// readback settles whether each kept its own tint.
    const SPAN: u32 = 64;
    const BOX: [f32; 2] = [16.0, 16.0];
    let off = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    let instance = Instance::default();
    let Ok(adapter) = block_on(instance.request_adapter(&RequestAdapterOptions::default())) else {
        eprintln!("GPU bridge: no adapter on this machine — the arena goes unverified!");
        return;
    };
    let Ok((device, queue)) = block_on(adapter.request_device(&DeviceDescriptor::default())) else {
        eprintln!("GPU bridge: no device on this machine — the arena goes unverified!");
        return;
    };
    let stride = {
        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        SPRITE_UNIFORM_BYTES.max(align).next_multiple_of(align)
    };

    // A white, opaque sheet: whatever the pixel comes out as, it came from
    // the sprite's own parameters.
    let atlas = device.create_texture(&TextureDescriptor {
        label: Some("arena atlas"),
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
    let view = atlas.create_view(&TextureViewDescriptor::default());
    let sampler = device.create_sampler(&SamplerDescriptor {
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        ..Default::default()
    });
    let fields = device.create_buffer(&BufferDescriptor {
        label: Some("arena fields"),
        size: 48,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&fields, 0, &[0u8; 48]);

    // The engine's own layout, so the offsets this test slides are the
    // offsets the render loop slides.
    let (_bind_layout, pipeline_layout) = sprite_layouts(&device);
    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("arena sprite"),
        source: ShaderSource::Wgsl(Cow::Borrowed(SPRITE_SHADER)),
    });
    let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some("arena sprite pipeline"),
        layout: Some(&pipeline_layout),
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

    // Two slots, the render loop's own packing, one upload.
    let slot = |center: [f32; 2], tint: Color| {
        sprite_uniform_data(
            Transform::translate(center)
                .invert()
                .expect("a translation"),
            BOX,
            tint,
            1.0,
            off,
            None,
            0.0,
            [0.0, 0.0, 1.0, 1.0],
        )
    };
    let red = slot(
        [16.0, 16.0],
        Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        },
    );
    let blue = slot(
        [48.0, 32.0],
        Color {
            r: 0.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
        },
    );
    let mut staging = vec![0u8; (stride * 2) as usize];
    staging[..red.len()].copy_from_slice(&red);
    let at = stride as usize;
    staging[at..at + blue.len()].copy_from_slice(&blue);
    let arena = device.create_buffer(&BufferDescriptor {
        label: Some("arena"),
        size: stride * 2,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&arena, 0, &staging);

    let entries = [
        BindGroupEntry {
            binding: 0,
            resource: BindingResource::Buffer(BufferBinding {
                buffer: &arena,
                offset: 0,
                size: NonZeroU64::new(SPRITE_UNIFORM_BYTES),
            }),
        },
        BindGroupEntry {
            binding: 1,
            resource: BindingResource::TextureView(&view),
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
    ];
    let bind = device.create_bind_group(&BindGroupDescriptor {
        label: Some("arena bind"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &entries,
    });

    let target = device.create_texture(&TextureDescriptor {
        label: Some("arena target"),
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
            label: Some("arena pass"),
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
        // The two draws, one bind group, each slid to its own slot. No
        // scissor: outside a sprite's box the shader returns transparent
        // black, which the blend leaves untouched — so a missed box shows up
        // as a wrong colour rather than a cropped one.
        pass.set_bind_group(0, &bind, &[0]);
        pass.draw(0..3, 0..1);
        pass.set_bind_group(0, &bind, &[stride as u32]);
        pass.draw(0..3, 0..1);
    }
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("arena readback"),
        size: u64::from(SPAN * 4 * SPAN),
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

    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let done = Arc::new(AtomicBool::new(false));
    {
        let done = Arc::clone(&done);
        readback.slice(..).map_async(MapMode::Read, move |r| {
            r.expect("map the readback");
            done.store(true, Ordering::SeqCst);
        });
    }
    let _ = device.poll(PollType::wait_indefinitely());
    assert!(done.load(Ordering::SeqCst), "the readback never mapped");
    let shot = readback
        .get_mapped_range(..)
        .expect("readback mapped")
        .to_vec();
    readback.unmap();

    let pixel = |x: u32, y: u32| {
        let at = ((y * SPAN + x) * 4) as usize;
        [shot[at], shot[at + 1], shot[at + 2], shot[at + 3]]
    };
    // Inside each box: its own colour, unmixed. If the two draws ever shared
    // a slot, the later write would own both boxes and the first assert is
    // the one that fails.
    assert_eq!(pixel(16, 16), [255, 0, 0, 255], "the first slot's box");
    assert_eq!(pixel(48, 32), [0, 0, 255, 255], "the second slot's box");
    // And outside both, nothing at all was written.
    assert_eq!(pixel(4, 60), [0, 0, 0, 0], "outside both boxes");
}

/// The weather the bench ran in, printed beside the numbers it qualifies.
///
/// A millisecond with no condition on it is an anecdote. What survives a busy
/// host is the *ratio* between schemes — 25.2x per-draw-over-arena measured on
/// battery, 23.8x on the charger, the absolute milliseconds drifting around
/// them — and the only way a reader months from now can tell which kind of
/// number they are holding is if the bench said so at the time it was taken.
/// This is not a gate: a bench has no baseline to fail, and `--ignored` means
/// someone is already reading the output.
struct Weather {
    canary: noisefloor::Canary,
    /// Which bench, so the line says what it qualifies.
    label: &'static str,
}

impl Weather {
    fn start(label: &'static str) -> Self {
        Self {
            canary: noisefloor::Canary::start(),
            label,
        }
    }

    fn report(self) {
        let floor = self.canary.report();
        eprintln!("draw cost [{}]: noise floor {floor}", self.label);
        if floor.too_loud() {
            eprintln!(
                "draw cost [{}]: too loud to judge by — the ratios travel, these milliseconds do not",
                self.label
            );
        }
    }
}

#[test]
#[ignore = "a timing bench: run by hand with --ignored --nocapture"]
fn a_uniform_per_draw_versus_an_arena() {
    let weather = Weather::start("glyph quads, 1600x1600");
    use crate::backend::app::block_on;
    use crate::backend::frame::{SPRITE_UNIFORM_BYTES, sprite_uniform_data};
    use crate::objects::{Color, Transform};
    use crate::shaders::SPRITE_SHADER;
    use std::borrow::Cow;
    use std::num::NonZeroU64;
    use std::time::Instant;
    use wgpu::*;

    /// The target's edge in pixels, near the window the field numbers were
    /// taken from (1501x1170) so the fragment cost is of the same order.
    const SPAN: u32 = 1600;
    /// The atlas: 32x40-pixel cells in a 512x512 RGBA8 sheet, 16 across and
    /// 12 down — the shape a glyph atlas has, and the reason glyph quads
    /// share one texture at all.
    const ATLAS: u32 = 512;
    const CELL: [u32; 2] = [32, 40];
    /// The quad: a glyph's box in pixels, and the pitch its line walks at.
    const GLYPH: [f32; 2] = [20.0, 26.0];
    const PITCH: [u32; 2] = [22, 30];
    const COLUMNS: u32 = 44;
    /// Frames per measured pass, and the untimed warm-up before it. The
    /// house rule from `docs/benchmarking.md`: warm up untimed, run the
    /// scheme more than once, report a later pass.
    const WARM: u32 = 12;
    const PASSES: u32 = 3;
    const FRAMES: u32 = 30;

    /// The three ways of getting a draw's uniforms in front of the GPU.
    #[derive(Clone, Copy)]
    enum Scheme {
        PerDraw,
        Arena,
        Pool,
    }

    /// One draw's worth: the uniform bytes, rebuilt every frame as the
    /// render loop rebuilds them, and the scissor box it would set.
    struct Item {
        bytes: Vec<u8>,
        box_: [u32; 4],
    }

    /// The scene: `count` glyph quads in lines of `COLUMNS`, each one a
    /// cell of the atlas, each with its own box.
    fn items(count: u32) -> Vec<Item> {
        let white = Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };
        let off = Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        };
        (0..count)
            .map(|i| {
                let cx = 24.0 + (i % COLUMNS) as f32 * PITCH[0] as f32 + GLYPH[0] / 2.0;
                let cy = 24.0 + (i / COLUMNS) as f32 * PITCH[1] as f32 + GLYPH[1] / 2.0;
                let cell = i % (ATLAS * ATLAS / (CELL[0] * CELL[1]));
                let cu = (cell % (ATLAS / CELL[0])) as f32 * CELL[0] as f32;
                let cv = (cell / (ATLAS / CELL[0])) as f32 * CELL[1] as f32;
                let uv = [
                    cu / ATLAS as f32,
                    cv / ATLAS as f32,
                    (cu + CELL[0] as f32) / ATLAS as f32,
                    (cv + CELL[1] as f32) / ATLAS as f32,
                ];
                let world = Transform::translate([cx, cy]);
                let bytes = sprite_uniform_data(
                    world.invert().expect("a translation"),
                    GLYPH,
                    white,
                    1.0,
                    off,
                    None,
                    0.0,
                    uv,
                );
                let box_ = [
                    (cx - GLYPH[0] / 2.0) as u32,
                    (cy - GLYPH[1] / 2.0) as u32,
                    GLYPH[0] as u32,
                    GLYPH[1] as u32,
                ];
                Item { bytes, box_ }
            })
            .collect()
    }

    /// A render pass over the bench's target, cleared.
    fn begin<'a>(encoder: &'a mut CommandEncoder, target_view: &TextureView) -> RenderPass<'a> {
        encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("draw cost pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target_view,
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
        })
    }

    /// The bind group entries a sprite draw needs around one uniform buffer:
    /// the draw's own bytes, the atlas, the sampler, and the frame's two
    /// fields — everything but the first identical for every draw of the
    /// frame, which is the fact both batching schemes are built on.
    fn sprite_entries<'a>(
        uniform: &'a Buffer,
        atlas_view: &'a TextureView,
        sampler: &'a Sampler,
        fields: &'a Buffer,
    ) -> Vec<BindGroupEntry<'a>> {
        [
            BindingResource::Buffer(BufferBinding {
                buffer: uniform,
                offset: 0,
                size: None,
            }),
            BindingResource::TextureView(atlas_view),
            BindingResource::Sampler(sampler),
            BindingResource::Buffer(BufferBinding {
                buffer: fields,
                offset: 0,
                size: None,
            }),
            BindingResource::Buffer(BufferBinding {
                buffer: fields,
                offset: 0,
                size: None,
            }),
        ]
        .into_iter()
        .enumerate()
        .map(|(binding, resource)| BindGroupEntry {
            binding: binding as u32,
            resource,
        })
        .collect()
    }

    /// One frame, scheme `per-draw`: the render loop's recipe, a buffer and
    /// a bind group to every draw.
    #[allow(clippy::too_many_arguments)]
    fn frame_per_draw(
        device: &Device,
        queue: &Queue,
        pipeline: &RenderPipeline,
        atlas_view: &TextureView,
        sampler: &Sampler,
        fields: &Buffer,
        target_view: &TextureView,
        items: &[Item],
    ) -> CommandEncoder {
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut pass = begin(&mut encoder, target_view);
            pass.set_pipeline(pipeline);
            for item in items {
                let uniform = device.create_buffer(&BufferDescriptor {
                    label: Some("sprite uniforms"),
                    size: item.bytes.len() as u64,
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&uniform, 0, &item.bytes);
                let entries = sprite_entries(&uniform, atlas_view, sampler, fields);
                let bind = device.create_bind_group(&BindGroupDescriptor {
                    label: Some("sprite bind"),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &entries,
                });
                pass.set_scissor_rect(item.box_[0], item.box_[1], item.box_[2], item.box_[3]);
                pass.set_bind_group(0, &bind, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        encoder
    }

    /// One frame, scheme `arena`: the draws' uniform bytes end to end in one
    /// buffer behind one bind group, each draw reaching its own bytes with a
    /// dynamic offset. `packed` is the frame's staging vector, reused.
    #[allow(clippy::too_many_arguments)]
    fn frame_arena(
        device: &Device,
        queue: &Queue,
        pipeline: &RenderPipeline,
        arena: &Buffer,
        bind: &BindGroup,
        stride: u64,
        packed: &mut Vec<u8>,
        target_view: &TextureView,
        items: &[Item],
    ) -> CommandEncoder {
        // The frame's uniforms, end to end, each at its aligned slot: one
        // vector built on the CPU, one upload, and no per-draw object.
        packed.clear();
        for item in items {
            packed.extend_from_slice(&item.bytes);
            // The slot is padded to the alignment a dynamic offset must
            // honour; the bytes past a draw's 96 are never read.
            let pad = stride as usize - item.bytes.len();
            packed.resize(packed.len() + pad, 0);
        }
        queue.write_buffer(arena, 0, packed);
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut pass = begin(&mut encoder, target_view);
            pass.set_pipeline(pipeline);
            for (i, item) in items.iter().enumerate() {
                pass.set_scissor_rect(item.box_[0], item.box_[1], item.box_[2], item.box_[3]);
                pass.set_bind_group(0, bind, &[i as u32 * stride as u32]);
                pass.draw(0..3, 0..1);
            }
        }
        encoder
    }

    /// One frame, scheme `pool`: the buffers and bind groups a frame needs
    /// already exist — made once, reused every frame after — so the frame
    /// only writes each draw's bytes into the buffer it was handed. The
    /// pipeline keeps its auto layout: no hand-written layout to keep in
    /// step with the WGSL, which is why this scheme earns its own number
    /// next to the arena's rather than being argued away.
    fn frame_pool(
        device: &Device,
        queue: &Queue,
        pipeline: &RenderPipeline,
        pool: &[(Buffer, BindGroup)],
        target_view: &TextureView,
        items: &[Item],
    ) -> CommandEncoder {
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut pass = begin(&mut encoder, target_view);
            pass.set_pipeline(pipeline);
            for (item, (uniform, bind)) in items.iter().zip(pool) {
                queue.write_buffer(uniform, 0, &item.bytes);
                pass.set_scissor_rect(item.box_[0], item.box_[1], item.box_[2], item.box_[3]);
                pass.set_bind_group(0, bind, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        encoder
    }

    let instance = Instance::default();
    let Ok(adapter) = block_on(instance.request_adapter(&RequestAdapterOptions::default())) else {
        eprintln!("draw cost: no adapter on this machine — the bench goes unrun!");
        return;
    };
    let name = adapter.get_info().name;
    let Ok((device, queue)) = block_on(adapter.request_device(&DeviceDescriptor::default())) else {
        eprintln!("draw cost: no device on this machine — the bench goes unrun!");
        return;
    };
    // The stride an arena slot needs: a dynamic offset must be a multiple of
    // the device's uniform-offset alignment — 256 on every backend so far,
    // against the sprite uniform's 96 bytes — so an arena carries the frame
    // at up to 2.7 times the uniforms' own bytes, and uploads them once.
    let stride = {
        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        SPRITE_UNIFORM_BYTES.max(align).next_multiple_of(align)
    };

    // The atlas texture, opaque everywhere on purpose: a transparent texel
    // exits through the shader's discard, and a bench whose fragments mostly
    // quit measures less GPU work than the frame it stands for.
    let atlas = device.create_texture(&TextureDescriptor {
        label: Some("draw cost atlas"),
        size: Extent3d {
            width: ATLAS,
            height: ATLAS,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let pixels: Vec<u8> = (0..(u64::from(ATLAS) * u64::from(ATLAS)))
        .flat_map(|i| {
            let x = (i % u64::from(ATLAS)) as u8;
            let y = (i / u64::from(ATLAS)) as u8;
            [x, x.rotate_left(3), y, 255]
        })
        .collect();
    queue.write_texture(
        TexelCopyTextureInfo {
            texture: &atlas,
            mip_level: 0,
            origin: Origin3d::default(),
            aspect: TextureAspect::All,
        },
        &pixels,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(ATLAS * 4),
            rows_per_image: Some(ATLAS),
        },
        Extent3d {
            width: ATLAS,
            height: ATLAS,
            depth_or_array_layers: 1,
        },
    );
    let view = atlas.create_view(&TextureViewDescriptor::default());
    let sampler = device.create_sampler(&SamplerDescriptor {
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        ..Default::default()
    });
    // The frame's fields: bound by every draw, never read (every quad is
    // unlit), and present in the layout because the shader asks for them.
    let fields = device.create_buffer(&BufferDescriptor {
        label: Some("draw cost fields"),
        size: 48,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&fields, 0, &[0u8; 48]);
    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("draw cost sprite"),
        source: ShaderSource::Wgsl(Cow::Borrowed(SPRITE_SHADER)),
    });
    let target = device.create_texture(&TextureDescriptor {
        label: Some("draw cost target"),
        size: Extent3d {
            width: SPAN,
            height: SPAN,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&TextureViewDescriptor::default());
    let pipeline = |layout: Option<&PipelineLayout>, label: &str| {
        device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some(label),
            layout,
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
        })
    };
    // Scheme one builds its pipeline the way the engine does: auto layout.
    let plain = pipeline(None, "draw cost per-draw");
    // Scheme two needs an explicit layout — an auto layout reads the
    // shader's binding as `dynamic: false`, so a dynamic offset has nowhere
    // to go. The arena's price is this layout, built once.
    let arena_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("draw cost arena layout"),
        entries: &[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: NonZeroU64::new(SPRITE_UNIFORM_BYTES),
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 3,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 4,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let arena_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("draw cost arena pipeline layout"),
        bind_group_layouts: &[Some(&arena_layout)],
        immediate_size: 0,
    });
    let arena_pipeline = pipeline(Some(&arena_pipeline_layout), "draw cost arena");

    eprintln!("draw cost: adapter {name}, arena slot stride {stride} bytes");
    for count in [243_u32, 1000, 2000] {
        let scene = items(count);
        let arena = device.create_buffer(&BufferDescriptor {
            label: Some("draw cost arena"),
            size: stride * u64::from(count),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&BindGroupDescriptor {
            label: Some("draw cost arena bind"),
            layout: &arena_layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    // One slot, not the arena: a dynamic offset *shifts* the
                    // bound window, so wgpu allows offsets only within the
                    // slack the binding leaves — bind the whole buffer and
                    // the maximum offset is zero. The window is the uniform's
                    // own 96 bytes, and each draw slides it to its slot.
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &arena,
                        offset: 0,
                        size: NonZeroU64::new(SPRITE_UNIFORM_BYTES),
                    }),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&view),
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
        let mut packed = vec![0u8; (stride as usize) * count as usize];
        // The pool: the frame's buffers and bind groups, made once for this
        // run — the equivalent of an engine that built them on its first
        // frame and has reused them ever since. `count` entries, because a
        // frame's draws may all be live at once and two of them may never
        // share a buffer.
        let pool: Vec<(Buffer, BindGroup)> = (0..count)
            .map(|_| {
                let uniform = device.create_buffer(&BufferDescriptor {
                    label: Some("pooled uniform"),
                    size: 96,
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let entries = sprite_entries(&uniform, &view, &sampler, &fields);
                let bind = device.create_bind_group(&BindGroupDescriptor {
                    label: Some("pooled bind"),
                    layout: &plain.get_bind_group_layout(0),
                    entries: &entries,
                });
                (uniform, bind)
            })
            .collect();
        // One timed run of a scheme: warm up, then PASSES measured passes of
        // FRAMES frames each, keeping the last. `drain` waits for the device
        // after every frame; without it the run measures the CPU's own work
        // and lets the GPU trail behind, as a fast frame does.
        let mut run = |label: &str, scheme: Scheme, drain: bool| -> f64 {
            let one = |packed: &mut Vec<u8>| {
                let encoder = match scheme {
                    Scheme::PerDraw => frame_per_draw(
                        &device,
                        &queue,
                        &plain,
                        &view,
                        &sampler,
                        &fields,
                        &target_view,
                        &scene,
                    ),
                    Scheme::Arena => frame_arena(
                        &device,
                        &queue,
                        &arena_pipeline,
                        &arena,
                        &bind,
                        stride,
                        packed,
                        &target_view,
                        &scene,
                    ),
                    Scheme::Pool => {
                        frame_pool(&device, &queue, &plain, &pool, &target_view, &scene)
                    }
                };
                queue.submit(Some(encoder.finish()));
            };
            for _ in 0..WARM {
                one(&mut packed);
                if drain {
                    let _ = device.poll(PollType::wait_indefinitely());
                }
            }
            let _ = device.poll(PollType::wait_indefinitely());
            let mut last = 0.0;
            for _ in 0..PASSES {
                let t = Instant::now();
                for _ in 0..FRAMES {
                    one(&mut packed);
                    if drain {
                        let _ = device.poll(PollType::wait_indefinitely());
                    }
                }
                if !drain {
                    let _ = device.poll(PollType::wait_indefinitely());
                }
                last = t.elapsed().as_secs_f64() * 1000.0 / f64::from(FRAMES);
            }
            eprintln!(
                "draws {count:>5}  {label:<9} {:<7} {:>7.3} ms/frame",
                if drain { "frame" } else { "submit" },
                last,
            );
            last
        };
        let a_frame = run("per-draw", Scheme::PerDraw, true);
        let b_frame = run("arena", Scheme::Arena, true);
        let c_frame = run("pool", Scheme::Pool, true);
        let a_submit = run("per-draw", Scheme::PerDraw, false);
        let b_submit = run("arena", Scheme::Arena, false);
        let c_submit = run("pool", Scheme::Pool, false);
        eprintln!(
            "           vs per-draw: arena {:.2}x frame / {:.2}x submit, pool {:.2}x frame / {:.2}x submit",
            a_frame / b_frame.max(0.001),
            a_submit / b_submit.max(0.001),
            a_frame / c_frame.max(0.001),
            a_submit / c_submit.max(0.001),
        );
    }

    weather.report();
}

/// The same question asked of a frame that draws everything, not only glyphs.
///
/// The bench above measures the mechanism on sprite quads. This one walks a
/// frame that cycles through every kind the arena took over — a line, a
/// polyline, a circle, a rectangle, a shape, a glyph — because those are the
/// draws whose buffer and bind group the engine no longer builds per draw, and
/// because a polyline's uniform is sixty-five times a rectangle's, which is
/// what the arena's variable-width runs exist for: a fixed slot sized for the
/// polyline would carry two kilobytes of padding on every other kind.
///
/// The draws are the engine's own [`Draw`] values and its bytes come from the
/// engine's own [`uniform_bytes`], and the arena walk here is the render
/// loop's walk — so this measures the packing the frame actually does, not an
/// imitation of it. The two schemes are the render loop's two shapes: a
/// buffer, a write and a bind group per draw, or one arena, one write, and one
/// bind group per pipeline.
#[test]
#[ignore = "a timing bench: run by hand with --ignored --nocapture"]
fn a_frame_of_every_kind_packs_into_the_arena_too() {
    let weather = Weather::start("every kind, 960x720");
    use crate::backend::app::{block_on, shape_layouts, slot_layouts, sprite_layouts};
    use crate::backend::frame::{
        CIRCLE_UNIFORM_BYTES, Draw, LINE_UNIFORM_BYTES, POLYLINE_UNIFORM_BYTES, RECT_UNIFORM_BYTES,
        SHAPE_UNIFORM_BYTES, SPRITE_UNIFORM_BYTES, uniform_bytes,
    };
    use crate::objects::{Color, SpriteFilter, Transform};
    use crate::shaders::{
        CIRCLE_SHADER, LINE_SHADER, POLYLINE_SHADER, RECT_SHADER, SHAPE_SHADER, SPRITE_SHADER,
    };
    use std::borrow::Cow;
    use std::num::NonZeroU64;
    use std::sync::Arc;
    use std::time::Instant;
    use wgpu::*;

    const WARM: usize = 12;
    const PASSES: usize = 3;
    const FRAMES: usize = 30;

    let grey = Color {
        r: 0.7,
        g: 0.7,
        b: 0.7,
        a: 1.0,
    };
    let off = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    /// One kind of draw per slot of the cycle, with the uniform size and the
    /// bindings that follow the uniform in its bind group: none for the plain
    /// kinds, the two frame fields for a shape, and the image, the sampler and
    /// the fields for a sprite.
    struct Kind {
        name: &'static str,
        source: &'static str,
        uniform: u64,
        fields: bool,
        image: bool,
    }
    let kinds = [
        Kind {
            name: "line",
            source: LINE_SHADER,
            uniform: LINE_UNIFORM_BYTES,
            fields: false,
            image: false,
        },
        Kind {
            name: "polyline",
            source: POLYLINE_SHADER,
            uniform: POLYLINE_UNIFORM_BYTES,
            fields: false,
            image: false,
        },
        Kind {
            name: "circle",
            source: CIRCLE_SHADER,
            uniform: CIRCLE_UNIFORM_BYTES,
            fields: false,
            image: false,
        },
        Kind {
            name: "rectangle",
            source: RECT_SHADER,
            uniform: RECT_UNIFORM_BYTES,
            fields: false,
            image: false,
        },
        Kind {
            name: "shape",
            source: SHAPE_SHADER,
            uniform: SHAPE_UNIFORM_BYTES,
            fields: true,
            image: false,
        },
        Kind {
            name: "sprite",
            source: SPRITE_SHADER,
            uniform: SPRITE_UNIFORM_BYTES,
            fields: true,
            image: true,
        },
    ];
    // One white sheet, shared by every sprite in the frame the way an atlas is
    // shared by every glyph.
    let sheet: Arc<[u8]> = vec![255u8; 16 * 16 * 4].into();
    let count_of = |kind: usize, i: usize| -> Draw {
        let x = ((i % 40) as f32 + 0.5) * 22.0;
        let y = ((i / 40) as f32 + 0.5) * 30.0;
        match kind {
            0 => Draw::Line {
                a: [x, y],
                b: [x + 20.0, y + 26.0],
                width: 2.0,
                color: grey,
                z: 0.0,
                diagnostic: false,
            },
            1 => Draw::Polyline {
                points: (0..8)
                    .map(|p| [x + p as f32 * 4.0, y + (p % 2) as f32 * 6.0])
                    .collect(),
                width: 2.0,
                color: grey,
                z: 0.0,
                diagnostic: false,
            },
            2 => Draw::Circle {
                center: [x, y],
                radius: 10.0,
                color: grey,
                z: 0.0,
                diagnostic: false,
            },
            3 => Draw::Rectangle {
                center: [x, y],
                extent: [12.0, 16.0],
                color: grey,
                z: 0.0,
                diagnostic: false,
            },
            4 => Draw::Shape {
                world: Transform::translate([x, y]),
                center: [0.0, 0.0],
                params: [10.0, 10.0],
                kind: 0.0,
                aa: 1.0,
                color: grey,
                emission: off,
                glow: None,
                lit: 0.0,
                occludes: 0.0,
                z: 0.0,
                diagnostic: false,
            },
            _ => Draw::Sprite {
                world: Transform::translate([x, y]),
                data: Arc::clone(&sheet),
                generation: 0,
                size: [20.0, 26.0],
                texture_size: [16, 16],
                filter: SpriteFilter::Linear,
                aa: 0.0,
                tint: grey,
                alpha: 1.0,
                uv_rect: [0.0, 0.0, 1.0, 1.0],
                emission: off,
                glow: None,
                lit: 0.0,
                z: 0.0,
                diagnostic: false,
            },
        }
    };

    let instance = Instance::default();
    let Ok(adapter) = block_on(instance.request_adapter(&RequestAdapterOptions::default())) else {
        eprintln!("GPU bridge: no adapter on this machine — nothing to measure!");
        return;
    };
    let Ok((device, queue)) = block_on(adapter.request_device(&DeviceDescriptor::default())) else {
        eprintln!("GPU bridge: no device on this machine — nothing to measure!");
        return;
    };
    let stride = {
        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        SPRITE_UNIFORM_BYTES.max(align).next_multiple_of(align)
    };

    // One sampler and one field buffer for the shape and sprite bindings,
    // beside the sheet the draws already share.
    let atlas = device.create_texture(&TextureDescriptor {
        label: Some("mixed atlas"),
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
        &sheet,
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
    let view = atlas.create_view(&TextureViewDescriptor::default());
    let sampler = device.create_sampler(&SamplerDescriptor::default());
    let fields = device.create_buffer(&BufferDescriptor {
        label: Some("mixed fields"),
        size: 48,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&fields, 0, &[0u8; 48]);

    let target = device.create_texture(&TextureDescriptor {
        label: Some("mixed target"),
        size: Extent3d {
            width: 960,
            height: 720,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target_view = target.create_view(&TextureViewDescriptor::default());

    fn pipeline(
        device: &Device,
        source: &str,
        layout: Option<&PipelineLayout>,
        label: &str,
    ) -> RenderPipeline {
        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some(label),
            source: ShaderSource::Wgsl(Cow::Borrowed(source)),
        });
        device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some(label),
            layout,
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
        })
    }
    // The per-draw scheme derives its layouts from the shaders; the arena's
    // layouts are the engine's own, because a dynamic offset has nowhere to go
    // in a derived one.
    let plain: Vec<RenderPipeline> = kinds
        .iter()
        .map(|k| pipeline(&device, k.source, None, k.name))
        .collect();
    let arena_pipelines: Vec<RenderPipeline> = kinds
        .iter()
        .map(|k| {
            let layout = if k.image {
                sprite_layouts(&device).1
            } else if k.fields {
                shape_layouts(&device, k.name)
            } else {
                slot_layouts(&device, k.name, k.uniform)
            };
            pipeline(&device, k.source, Some(&layout), k.name)
        })
        .collect();

    // The bind group entries, in the order the shaders bind them: the uniform,
    // then the image and sampler for a sprite, then the two frame fields. A
    // function rather than a closure because its lifetime — the entries borrow
    // the buffers they name — is one a closure cannot state.
    fn entries<'a>(
        uniform: &'a Buffer,
        kind: &Kind,
        view: &'a TextureView,
        sampler: &'a Sampler,
        fields: &'a Buffer,
    ) -> Vec<BindGroupEntry<'a>> {
        let mut entries = vec![BindGroupEntry {
            binding: 0,
            resource: BindingResource::Buffer(BufferBinding {
                buffer: uniform,
                offset: 0,
                size: None,
            }),
        }];
        let mut next = 1;
        if kind.image {
            entries.push(BindGroupEntry {
                binding: next,
                resource: BindingResource::TextureView(view),
            });
            entries.push(BindGroupEntry {
                binding: next + 1,
                resource: BindingResource::Sampler(sampler),
            });
            next += 2;
        }
        if kind.fields {
            entries.push(BindGroupEntry {
                binding: next,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: fields,
                    offset: 0,
                    size: None,
                }),
            });
            entries.push(BindGroupEntry {
                binding: next + 1,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: fields,
                    offset: 0,
                    size: None,
                }),
            });
        }
        entries
    }

    for count in [243usize, 1000, 2000] {
        let draws: Vec<Draw> = (0..count).map(|i| count_of(i % kinds.len(), i)).collect();
        // Per draw, as the engine used to do it: a buffer for the draw's own
        // bytes, a write into it, and a bind group built around both.
        let frame_per_draw = || -> CommandEncoder {
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
            {
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("mixed per-draw"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &target_view,
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
                for (i, draw) in draws.iter().enumerate() {
                    let kind = &kinds[i % kinds.len()];
                    let bytes = uniform_bytes(draw).expect("every kind here has a uniform");
                    let buffer = device.create_buffer(&BufferDescriptor {
                        label: Some("mixed uniform"),
                        size: bytes.len() as u64,
                        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
                    queue.write_buffer(&buffer, 0, &bytes);
                    let pipeline = &plain[i % kinds.len()];
                    let entries = entries(&buffer, kind, &view, &sampler, &fields);
                    let bind = device.create_bind_group(&BindGroupDescriptor {
                        label: Some("mixed bind"),
                        layout: &pipeline.get_bind_group_layout(0),
                        entries: &entries,
                    });
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &bind, &[]);
                    pass.draw(0..3, 0..1);
                }
            }
            encoder
        };

        // The arena: the render loop's own walk — pack each draw's bytes into
        // an aligned run, remember where it landed, upload the whole frame
        // once, and hand every draw an offset into the one bind group its
        // pipeline shares with the draws of its kind.
        let mut arena = device.create_buffer(&BufferDescriptor {
            label: Some("mixed arena"),
            size: stride,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut packed: Vec<u8> = Vec::new();
        let mut offsets: Vec<u32> = Vec::new();
        for draw in draws.iter() {
            let bytes = uniform_bytes(draw).expect("every kind here has a uniform");
            let at = (packed.len() as u64).next_multiple_of(stride);
            packed.resize(at as usize, 0);
            packed.extend_from_slice(&bytes);
            offsets.push(at as u32);
        }
        if arena.size() < packed.len() as u64 {
            arena = device.create_buffer(&BufferDescriptor {
                label: Some("mixed arena"),
                size: (packed.len() as u64).next_multiple_of(stride),
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        let binds: Vec<BindGroup> = kinds
            .iter()
            .enumerate()
            .map(|(k, kind)| {
                // Same entries as the per-draw scheme, except the first: a
                // dynamic offset slides the *bound window*, so the arena is
                // bound one run wide — exactly what the engine binds.
                let mut entries = entries(&arena, kind, &view, &sampler, &fields);
                entries[0].resource = BindingResource::Buffer(BufferBinding {
                    buffer: &arena,
                    offset: 0,
                    size: NonZeroU64::new(kind.uniform),
                });
                device.create_bind_group(&BindGroupDescriptor {
                    label: Some("mixed arena bind"),
                    layout: &arena_pipelines[k].get_bind_group_layout(0),
                    entries: &entries,
                })
            })
            .collect();
        let frame_arena = || -> CommandEncoder {
            let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
            queue.write_buffer(&arena, 0, &packed);
            {
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("mixed arena"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &target_view,
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
                for (i, at) in offsets.iter().enumerate() {
                    let k = i % kinds.len();
                    pass.set_pipeline(&arena_pipelines[k]);
                    pass.set_bind_group(0, &binds[k], &[*at]);
                    pass.draw(0..3, 0..1);
                }
            }
            encoder
        };

        let run = |label: &str, arena_scheme: bool, drain: bool| -> f64 {
            let one = || {
                let encoder = if arena_scheme {
                    frame_arena()
                } else {
                    frame_per_draw()
                };
                queue.submit(Some(encoder.finish()));
            };
            for _ in 0..WARM {
                one();
                if drain {
                    let _ = device.poll(PollType::wait_indefinitely());
                }
            }
            let _ = device.poll(PollType::wait_indefinitely());
            let mut last = 0.0;
            for _ in 0..PASSES {
                let t = Instant::now();
                for _ in 0..FRAMES {
                    one();
                    if drain {
                        let _ = device.poll(PollType::wait_indefinitely());
                    }
                }
                if !drain {
                    let _ = device.poll(PollType::wait_indefinitely());
                }
                last = t.elapsed().as_secs_f64() * 1000.0 / f64::from(FRAMES as u32);
            }
            eprintln!(
                "draws {count:>5}  {label:<9} {:<7} {:>7.3} ms/frame",
                if drain { "frame" } else { "submit" },
                last,
            );
            last
        };
        let a_frame = run("per-draw", false, true);
        let b_frame = run("arena", true, true);
        let a_submit = run("per-draw", false, false);
        let b_submit = run("arena", true, false);
        eprintln!(
            "           vs per-draw: arena {:.2}x frame / {:.2}x submit (bytes packed {:>7})",
            a_frame / b_frame.max(0.001),
            a_submit / b_submit.max(0.001),
            packed.len(),
        );
    }

    weather.report();
}
