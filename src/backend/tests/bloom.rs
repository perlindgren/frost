//! Bloom is light arriving where the shape that emitted it is not. That is the
//! whole contract, and it is the one thing every previous test in this tree
//! could not check: the emission tests look at the pixel *inside* a plate, and
//! bloom by definition moves light to the pixels outside it.
//!
//! So this harness paints the frame the way the engine does — the shape shader
//! into a float frame buffer and its emissive target — then runs the real
//! [`Bloom`] chain over the real emissive texture and composites with the real
//! `blit.wgsl`, the way the present pass does. What comes back is what a window
//! would have held, and the assertions are about the pixels beside the lamp.
//!
//! The harness does not share emission.rs's. That one hands back the bytes of
//! one target, and this one needs the target alive afterwards to bloom it;
//! bending it into shape would have made both tests harder to read than the
//! duplication they are paying for.

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

/// A 64×64 frame: every readback row is 512 bytes of half-float, which is the
/// multiple of 256 a texture copy demands.
const SPAN: u32 = 64;
/// The centre, and the plate's size in it.
const MIDDLE: f32 = 32.0;

/// The one plate this scene has: a 12-pixel square of amber at the centre,
/// unlit, putting out twice its own colour. Unlit so that the frame's own
/// pixels are exactly the plate's colour and nothing else — the only light in
/// the frame is the light the bloom chain moves.
fn lamp(emission: f32) -> Draw {
    lamp_at([MIDDLE, MIDDLE], emission)
}

/// The same plate anywhere else in the frame, for the picture test's scene.
fn lamp_at(at: [f32; 2], emission: f32) -> Draw {
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
fn frame(
    draws: &[Draw],
    strength: f32,
    dump: Option<&str>,
    override_weights: Option<[f32; 3]>,
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
                    inv, *center, *params, *kind, *aa, *color, *emission, *lit,
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
    let bloom = Bloom::new(
        &device,
        &queue,
        float,
        [SPAN, SPAN],
        Some(&emissive_view),
        strength,
    );
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
        size: 32,
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let fit =
        crate::backend::windowing::fit_bytes((1.0, 1.0), override_weights.unwrap_or(bloom.weights));
    queue.write_buffer(&fit_buffer, 0, &fit);
    let window = new_target("bloom window", TextureUsages::empty());
    let window_view = window.create_view(&TextureViewDescriptor::default());
    let bind = device.create_bind_group(&BindGroupDescriptor {
        label: Some("bloom present bind"),
        layout: &blit_pipeline.get_bind_group_layout(0),
        entries: &[
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
            BindGroupEntry {
                binding: 3,
                resource: BindingResource::TextureView(&bloom.views[0]),
            },
            BindGroupEntry {
                binding: 4,
                resource: BindingResource::TextureView(&bloom.views[1]),
            },
            BindGroupEntry {
                binding: 5,
                resource: BindingResource::TextureView(&bloom.views[2]),
            },
            BindGroupEntry {
                binding: 6,
                resource: BindingResource::Sampler(&bloom.sampler),
            },
        ],
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
        pass.draw(0..3, 0..1);
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

/// Light arrives where the lamp is not, and thins out as it goes.
///
/// Three strengths of the same frame: no bloom, half, and full. The plate's
/// own pixels barely matter here — what is checked is a pixel eight places
/// outside its edge, a pixel twenty-four outside, and the difference between
/// the two, which is the whole of what a glow is.
#[test]
fn bloom_lands_beside_the_lamp_and_thins_as_it_travels() {
    let Some(quiet) = frame(&[lamp(4.0)], 0.0, None, None) else {
        return;
    };
    let Some(half) = frame(&[lamp(4.0)], 0.5, None, None) else {
        return;
    };
    let Some(full) = frame(&[lamp(4.0)], 1.0, None, None) else {
        return;
    };
    let at = |x: u32, y: u32| usize::try_from(y * SPAN + x).expect("in frame");
    // The plate spans 26..38; these are outside it, to the right, on its
    // middle row. `near` is eight pixels past the plate's edge, `far` twenty-one.
    let near = at(46, MIDDLE as u32);
    let far = at(62, MIDDLE as u32);
    let sum = |p: &[f32; 3]| p[0] + p[1] + p[2];

    // The pixel beside the lamp held nothing at all without bloom — the frame
    // is a black background with one plate on it, so any light there is light
    // the chain moved.
    assert!(
        sum(&quiet[near]) < 1e-3,
        "the pixel outside the plate held {:?} with bloom off, and the frame \
         has nothing else in it",
        quiet[near]
    );
    let glow_near = sum(&full[near]) - sum(&quiet[near]);
    let glow_far = sum(&full[far]) - sum(&quiet[far]);
    let glow_half = sum(&half[near]) - sum(&quiet[near]);
    // Measured here on an M4 Max: 0.0124 five pixels out, 0.0015 twenty-one
    // out. The numbers are small because this is a 12-pixel plate in a 64-pixel
    // frame and a glow spreads one plate's light over an area hundreds of times
    // its size; the assertions sit at a third of what was measured, which leaves
    // room for another GPU's filtering without letting a chain that produced
    // nothing pass.
    assert!(
        glow_near > 0.004,
        "a plate putting out 1.6 of radiance left only {glow_near} of light \
         five pixels away; bloom is not arriving"
    );
    // It thins on the way: a glow that is uniform across the frame is not a
    // glow, it is a haze over the whole picture, and that is what a chain
    // whose levels are weighted wrong turns into.
    assert!(
        glow_near > glow_far * 3.0,
        "the glow does not fall off: {glow_near} eight pixels out against \
         {glow_far} twenty-one out (eight to one here)"
    );
    // And the knob is linear in what it adds: half the strength, half the glow
    // (measured 0.0062 against 0.0124 — exactly half, because the weights are
    // multiplied by strength on the CPU and nothing clamps on the way).
    assert!(
        glow_half > glow_near * 0.4 && glow_half < glow_near * 0.6,
        "at half strength the glow should be about half: {glow_half} against \
         {glow_near}"
    );
}

/// Nothing emissive means nothing to bloom: the same frame with the same
/// strength comes back exactly as it went in.
///
/// This is the assertion that keeps the threshold honest. Bloom that adds
/// light to a scene with no emission is not a glow, it is a brightness change,
/// and the difference only shows up as a scene that got washed out for no
/// reason anyone can point at.
#[test]
fn a_frame_that_emits_nothing_gains_nothing() {
    let Some(quiet) = frame(&[lamp(0.0)], 0.0, None, None) else {
        return;
    };
    let Some(bloomed) = frame(&[lamp(0.0)], 1.0, None, None) else {
        return;
    };
    let worst = quiet.iter().zip(&bloomed).fold(0.0f32, |m, (a, b)| {
        m.max(
            (a[0] - b[0])
                .abs()
                .max((a[1] - b[1]).abs())
                .max((a[2] - b[2]).abs()),
        )
    });
    assert_eq!(
        worst, 0.0,
        "a frame with no emission changed by {worst} at its worst pixel"
    );
    // And the plate is still there, so the equality above is not two black
    // frames agreeing with each other.
    let centre = usize::try_from(MIDDLE as u32 * SPAN + MIDDLE as u32).expect("in frame");
    assert!(
        bloomed[centre][0] > 0.3,
        "the plate itself vanished: {:?}",
        bloomed[centre]
    );
}

/// Writes two pictures to look at, for whoever is judging the look rather than
/// the arithmetic. Ignored, because a test that writes files is not a test.
#[test]
#[ignore = "writes /tmp/frost_bloom_*.ppm for a human to look at"]
fn paint_two_pictures_to_look_at() {
    frame(&[lamp(2.0)], 0.0, Some("/tmp/frost_bloom_off.ppm"), None);
    frame(&[lamp(2.0)], 1.0, Some("/tmp/frost_bloom_on.ppm"), None);
    // Three lamps of different authored strength over a dim floor: the case a
    // look is actually judged on, where the threshold's ramp should show as one
    // lamp glowing hard, one softly, and one barely at all.
    let scene = [
        // A wall to stand behind them, unlit and dim.
        Draw::Shape {
            world: Transform::identity(),
            center: [MIDDLE, 50.0],
            params: [30.0, 8.0],
            kind: 1.0,
            aa: 1.0,
            color: objects::Color {
                r: 0.08,
                g: 0.09,
                b: 0.12,
                a: 1.0,
            },
            emission: objects::Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
            lit: 0.0,
            occludes: 0.0,
            z: 0.0,
            diagnostic: false,
        },
        lamp_at([14.0, 20.0], 0.4),
        lamp_at([32.0, 20.0], 1.5),
        lamp_at([50.0, 20.0], 4.0),
    ];
    frame(&scene, 0.0, Some("/tmp/frost_scene_off.ppm"), None);
    frame(&scene, 1.0, Some("/tmp/frost_scene_on.ppm"), None);
}
