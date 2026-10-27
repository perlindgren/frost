//! The two answers to how a draw lands in the frame: it covers, or it adds.
//!
//! Every scene draw arrives by alpha blending, which is the entire working half
//! of the rule that rgb is radiance and alpha is coverage — a wall over a lamp
//! takes the lamp's glow away with its color, and that is right for walls. A
//! glow sprite needs the other answer, because its alpha is a falloff and a
//! falloff that covers is a hole punched in the light behind it. The renderer
//! carries exactly one pipeline that answers differently
//! ([`ADDITIVE_BLEND`]), which makes it worth asking the one question that
//! matters: does the light go *up* where the covering draw takes it down — on
//! both attachments, since the additive pipeline earns its keep only if a sprite
//! lights the frame and the channel the bloom chain reads.
//!
//! So the same two sprites are painted twice, once covering and once adding, and
//! four pixels come back. The expected numbers are written out below as the blend
//! definitions spell them, so a change to a factor fails here instead of quietly
//! moving a threshold somewhere else.

use super::super::*;
use super::half_to_f32;
use crate::backend::app::ADDITIVE_BLEND;
use crate::objects;
use crate::shaders::SPRITE_SHADER;
use std::borrow::Cow;
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::*;

/// Big enough that a 4×4 texture under nearest sampling puts eight pixels behind
/// every texel, and small enough that one row of half-floats is exactly a
/// 256-byte block — which is what a readback's row stride has to be a multiple
/// of.
const SPAN: u32 = 32;

/// The falloff sprite's side: a quarter of the target's area, big enough that
/// its four-by-four texture puts four pixels behind every texel.
const SIZE: f32 = 16.0;

/// The wall: one texel of opaque white, tinted to 0.6 and emitting white at
/// unity. Unlit, so its frame color is `0.6 × (1 + 1)`, and its emissive channel
/// is the 0.6 the emission never doubled.
fn wall() -> Vec<u8> {
    vec![255, 255, 255, 255]
}

/// The sprite laid over it: a 4×4 texture whose centre two-by-two carries the
/// falloff's shoulder (alpha 128/255) and whose border carries its edge (alpha
/// 64/255), rgb full white in both so the tint is the only thing scaling the
/// light. Neither is transparent enough for the shader's discard, so both
/// measure the blend rather than the discard.
fn falloff() -> Vec<u8> {
    let mut texel = Vec::with_capacity(4 * 4 * 4);
    for row in 0..4 {
        for col in 0..4 {
            let inner = (1..3).contains(&row) && (1..3).contains(&col);
            texel.extend_from_slice(&[255, 255, 255, if inner { 128 } else { 64 }]);
        }
    }
    texel
}

/// What one pixel of each attachment holds after the two sprites are painted,
/// the second of them covering or adding as asked.
#[derive(Debug)]
struct Pixel {
    frame: [f32; 4],
    emissive: [f32; 4],
}

/// A whole painting: the falloff's shoulder texel and its edge texel, read out
/// of both attachments.
#[derive(Debug)]
struct Painting {
    /// The wall where the falloff sprite never reaches, so its blend cannot
    /// have touched it.
    bare: Pixel,
    /// The falloff's centre texels, half opaque.
    shoulder: Pixel,
    /// Its border, a quarter opaque.
    edge: Pixel,
}

/// Paints the wall over the whole target and the falloff sprite over its middle
/// sixteen pixels, then reads the wall outside the sprite and the sprite's two
/// alpha steps inside it.
fn paint(additive: bool) -> Option<Painting> {
    // The thread's shared GPU: see [`super::machine`].
    let machine = super::machine::machine()?;
    let (device, queue) = (&machine.device, &machine.queue);

    let upload = |label: &str, bytes: Vec<u8>, size: [u32; 2]| {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width: size[0],
                height: size[1],
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
            &bytes,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: Some(size[1]),
            },
            Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
        );
        texture.create_view(&TextureViewDescriptor::default())
    };
    let wall = upload("blend wall", wall(), [1, 1]);
    let glow = upload("blend falloff", falloff(), [4, 4]);

    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("blend sprite"),
        source: ShaderSource::Wgsl(Cow::Borrowed(SPRITE_SHADER)),
    });
    // Both targets, both formats, the emissive entry point: exactly the pair the
    // scene pass offers, because the claim is about both attachments. The bind
    // layout is the pipeline's own (auto) rather than the arena's, since what is
    // under test here is how a draw lands, not how its parameters are addressed.
    let pipeline = |label: &'static str, blend: BlendState| {
        device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some(label),
            layout: None,
            vertex: VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(FragmentState {
                module: &module,
                entry_point: Some("fs_emissive"),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[
                    Some(ColorTargetState {
                        format: TextureFormat::Rgba16Float,
                        blend: Some(blend),
                        write_mask: ColorWrites::ALL,
                    }),
                    Some(ColorTargetState {
                        format: TextureFormat::Rgba16Float,
                        blend: Some(blend),
                        write_mask: ColorWrites::ALL,
                    }),
                ],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    };
    let covering = pipeline("blend covering", BlendState::ALPHA_BLENDING);
    let adding = pipeline("blend adding", ADDITIVE_BLEND);

    let sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("blend sampler"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        mipmap_filter: MipmapFilterMode::Nearest,
        ..Default::default()
    });
    // The light and occluder fields: never read (both sprites are unlit), but
    // the bind layout demands them.
    let fields = device.create_buffer_init(&BufferInitDescriptor {
        label: Some("blend fields"),
        contents: &[0u8; 48],
        usage: BufferUsages::STORAGE,
    });

    // Everything is centred on the target; the falloff's quad is a quarter of
    // its area, which is what leaves a strip of bare wall to measure against.
    let centre = [SPAN as f32 / 2.0, SPAN as f32 / 2.0];
    let sprite = |view: &TextureView, size: f32, tint: objects::Color, emission: objects::Color| {
        let uniform = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("blend uniform"),
            contents: &sprite_uniform_data(
                objects::Transform::translate(centre)
                    .invert()
                    .expect("a translation"),
                [size, size],
                tint,
                1.0,
                emission,
                None,
                0.0,
                objects::LightLayers::default(),
                [0.0, 0.0, 1.0, 1.0],
            ),
            usage: BufferUsages::UNIFORM,
        });
        (uniform, view.clone())
    };
    let grey = |v: f32| objects::Color {
        r: v,
        g: v,
        b: v,
        a: 1.0,
    };
    let white = objects::Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    let wall = sprite(&wall, SPAN as f32, grey(0.6), white);
    let glow = sprite(&glow, SIZE, grey(0.2), white);
    let bind = |pipeline: &RenderPipeline, draw: &(Buffer, TextureView)| {
        device.create_bind_group(&BindGroupDescriptor {
            label: Some("blend bind"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer: &draw.0,
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&draw.1),
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
        })
    };

    let target = |label: &str| {
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
            format: TextureFormat::Rgba16Float,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let frame_target = target("blend frame");
    let emissive_target = target("blend emissive");

    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("blend pass"),
            color_attachments: &[
                Some(RenderPassColorAttachment {
                    view: &frame_target.create_view(&TextureViewDescriptor::default()),
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color::BLACK),
                        store: StoreOp::Store,
                    },
                }),
                Some(RenderPassColorAttachment {
                    view: &emissive_target.create_view(&TextureViewDescriptor::default()),
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        // A clear color is a color, not a light: the emissive
                        // target starts dark, exactly as the scene pass leaves
                        // it, so everything below is light some sprite put there.
                        load: LoadOp::Clear(wgpu::Color::BLACK),
                        store: StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        // The wall first, covering either way: it is the room the glow is held
        // up against, not the thing under test.
        pass.set_pipeline(&covering);
        pass.set_bind_group(0, &bind(&covering, &wall), &[]);
        pass.draw(0..3, 0..1);
        let second = if additive { &adding } else { &covering };
        pass.set_pipeline(second);
        pass.set_bind_group(0, &bind(second, &glow), &[]);
        pass.draw(0..3, 0..1);
    }

    // Both attachments copied out in one encoder, then submitted: the two maps
    // are in flight together and one poll waits for both.
    let readback = |label: &str| {
        device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: u64::from(SPAN) * u64::from(SPAN) * 8,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        })
    };
    let frame_read = readback("blend frame readback");
    let emissive_read = readback("blend emissive readback");
    let mut copy_out = |texture: &Texture, buffer: &Buffer| {
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: Origin3d::default(),
                aspect: TextureAspect::All,
            },
            TexelCopyBufferInfo {
                buffer,
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
    };
    copy_out(&frame_target, &frame_read);
    copy_out(&emissive_target, &emissive_read);
    queue.submit(Some(encoder.finish()));

    static DONE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    DONE.store(0, std::sync::atomic::Ordering::SeqCst);
    let mapped = |_| {
        DONE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    };
    frame_read.slice(..).map_async(MapMode::Read, mapped);
    emissive_read.slice(..).map_async(MapMode::Read, mapped);
    super::machine::drain(device);
    assert_eq!(
        DONE.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the readbacks never both mapped"
    );
    let take = |buffer: &Buffer| {
        let bytes = buffer
            .get_mapped_range(..)
            .expect("readback mapped")
            .to_vec();
        buffer.unmap();
        bytes
    };
    let frame = take(&frame_read);
    let emissive = take(&emissive_read);

    let pixel = |bytes: &[u8], at: [u32; 2]| {
        let index = usize::try_from(at[1] * SPAN + at[0]).expect("in frame") * 8;
        std::array::from_fn(|channel| {
            half_to_f32(u16::from_le_bytes([
                bytes[index + channel * 2],
                bytes[index + channel * 2 + 1],
            ]))
        })
    };

    // Four pixels to a texel: (14, 14) is well inside a centre texel, (9, 9)
    // well inside a border one, and (2, 2) past the sprite entirely.
    Some(Painting {
        bare: Pixel {
            frame: pixel(&frame, [2, 2]),
            emissive: pixel(&emissive, [2, 2]),
        },
        shoulder: Pixel {
            frame: pixel(&frame, [14, 14]),
            emissive: pixel(&emissive, [14, 14]),
        },
        edge: Pixel {
            frame: pixel(&frame, [9, 9]),
            emissive: pixel(&emissive, [9, 9]),
        },
    })
}

/// A glow adds light where an ordinary draw takes it away — and by the number
/// the blend says, not approximately.
///
/// The wall holds 1.2 of frame color and 0.6 of emissive. Over it goes a sprite
/// tinted to 0.2 whose texels are 128/255 opaque in the shoulder: covering, it
/// leaves `0.4 × 0.502 + 1.2 × 0.498` of frame color and `0.2 × 0.502 + 0.6 ×
/// 0.498` of emissive, so the light falls under the wall on both attachments —
/// a dark bell, which is precisely what a falloff must not be. Adding, it leaves
/// `1.2 + 0.4 × 0.502` and `0.6 + 0.2 × 0.502`: above the wall on both, which is
/// what lets one sprite be a lamp's core and the light the chain goes on to
/// blur.
#[test]
fn an_additive_sprite_lights_the_wall_a_covering_one_darkens() {
    let Some(cover) = paint(false) else { return };
    let Some(add) = paint(true) else { return };
    let near = |got: f32, want: f32| (got - want).abs() < 0.01;

    // The wall where no glow reaches: 0.6 of tint doubled by its own emission,
    // and 0.6 in the channel — identical in both frames, since a blend choice
    // cannot reach past the quad it draws.
    for painted in [&cover, &add] {
        assert!(
            near(painted.bare.frame[0], 1.2) && near(painted.bare.emissive[0], 0.6),
            "the wall behind the sprite came back differently depending on the
             sprite's blend: {:?}",
            painted.bare
        );
    }
    // Covering: the light goes down, on both attachments.
    assert!(
        near(cover.shoulder.frame[0], 0.798) && near(cover.shoulder.emissive[0], 0.399),
        "a covering draw over a 1.2 wall left {:?} of color and {:?} of \
         emissive; the expected blend is 0.4a + 1.2(1-a) and 0.2a + 0.6(1-a) \
         with a = 128/255",
        cover.shoulder.frame,
        cover.shoulder.emissive
    );
    // Adding: the same bytes go up, by the same arithmetic with `One` where
    // `OneMinusSrcAlpha` was.
    assert!(
        near(add.shoulder.frame[0], 1.401) && near(add.shoulder.emissive[0], 0.700),
        "an additive draw over the same wall left {:?} of color and {:?} of \
         emissive; a glow that does not raise the light it lands on is a hole \
         in it",
        add.shoulder.frame,
        add.shoulder.emissive
    );
    // And the source's rgb is scaled by its own alpha on the way in. The edge
    // texel is a quarter opaque with the same rgb behind it, so it adds
    // `0.4 × 64/255` — where a premultiplied assumption, which is what wgpu's
    // own `ADDITIVE` is, would add the whole `0.4` and light a square around
    // the glow.
    assert!(
        near(add.edge.frame[0], 1.300) && near(add.edge.emissive[0], 0.650),
        "the falloff's edge added {:?} of color and {:?} of emissive over a \
         1.2 / 0.6 wall; only a src factor of SrcAlpha lands 0.4 × 64/255 there",
        add.edge.frame,
        add.edge.emissive
    );
    // Coverage adds too, which is the one thing an additive draw does that a
    // scene can see past the color: the frame's alpha passes one under a glow,
    // and the window clamps what it cannot show.
    assert!(
        near(cover.edge.frame[3], 1.0) && near(add.edge.frame[3], 1.251),
        "covering left alpha {:?} and adding left {:?}",
        cover.edge.frame[3],
        add.edge.frame[3]
    );
}
