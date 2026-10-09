//! The shadow path measured end to end, on the GPU rather than in the model.
//!
//! An occluder reaches a pixel by a long route: authored as a local box,
//! packed by `pack_occluder_field` into a 48-byte record holding the inverse
//! of its transform, uploaded, then rebuilt per fragment by `occluded()` in
//! `lighting.wgsl`, which clips the light-to-pixel segment against the box in
//! the box's own space. `collision::occluded` answers the same question on the
//! CPU from the same box. The two implementations are written separately and
//! agree by construction so far — a drift between them is the bug this file
//! exists to catch, since it shows up on screen as a shadow that disagrees
//! with the physics and is invisible to every test that only checks one side.
//!
//! So: pack the fields with the real packers, render them through the real
//! assembled `shape.wgsl`, read the pixels back, and compare them with the CPU
//! answer for the same geometry. No adapter: skip, loudly.

use super::super::*;
use super::*;
use crate::Canvas;
use crate::backend::app::block_on;
use crate::canvas::Occluder;
use crate::collision::{Collider, OrientedBox, occluded};
use crate::objects;
use crate::objects::*;
use crate::shaders::SHAPE_SHADER;
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::*;

/// The render target's edge, in pixels. 64 keeps every readback row exactly
/// 256 bytes, the copy's required multiple, so no row padding is involved.
const SPAN: u32 = 64;

/// A box pushed from its own space into pixel space, measured from its four
/// corners rather than by folding the transform's rotation into its angle.
///
/// The corner route is deliberate. Deriving `center + angle + half` from the
/// transform's polar decomposition would repeat the arithmetic the packer
/// performs, and a test that shares its subject's arithmetic proves nothing;
/// transforming four corners and reading the rectangle back off the edges
/// shares only the transform itself. It also survives a mirrored transform —
/// the canvas's user-to-pixel map flips y — which turns a counter-clockwise
/// box into a clockwise one while leaving the points it covers alone.
fn box_in_pixels(world: &Transform, center: [f32; 2], half: [f32; 2]) -> OrientedBox {
    let corners = OrientedBox::rotated(center, half, 0.0).corners();
    let px: Vec<[f32; 2]> = corners.iter().map(|c| world.apply(*c)).collect();
    let center = [
        px.iter().map(|c| c[0]).sum::<f32>() / 4.0,
        px.iter().map(|c| c[1]).sum::<f32>() / 4.0,
    ];
    let edge = |a: usize, b: usize| [px[b][0] - px[a][0], px[b][1] - px[a][1]];
    let (e1, e2) = (edge(0, 1), edge(0, 3));
    let length = |e: [f32; 2]| (e[0] * e[0] + e[1] * e[1]).sqrt();
    OrientedBox::rotated(
        center,
        [length(e1) / 2.0, length(e2) / 2.0],
        e1[1].atan2(e1[0]),
    )
}

/// The frame's own recipe for one lit rectangle and whatever occluders the
/// scene carries: pack both fields, render every `Draw::Shape` in order with
/// `shape_uniform_data`, and hand back the red channel of every pixel.
///
/// The scene is built so the red byte *is* the light mix: the surface is
/// white, unglowing and lit, the light is white at intensity 1 with a radius
/// that barely falls off across 64 pixels, and the ambient is black. A pixel
/// the shadow test blocks therefore reads 0; one it does not reads most of
/// full scale. Nothing else in the frame can write a pixel, so there is no
/// compositing arithmetic between the shadow answer and the byte.
fn paint(draws: &[Draw], declared: &[Occluder]) -> Option<Vec<u8>> {
    let lights = pack_light_field(draws, black());
    let occluders = pack_occluder_field(draws, declared);

    let instance = Instance::default();
    let Ok(adapter) = block_on(instance.request_adapter(&RequestAdapterOptions::default())) else {
        eprintln!("shadow bridge: no adapter on this machine — the shadow test goes unverified!");
        return None;
    };
    let Ok((device, queue)) = block_on(adapter.request_device(&DeviceDescriptor::default())) else {
        eprintln!("shadow bridge: no device — the shadow test goes unverified!");
        return None;
    };

    // Padded well past what the two fields hold: a binding shorter than the
    // adapter's minimum storage-binding size is an outright validation error,
    // and the padding is dead weight the shaders' loops never reach, since
    // both loops stop at the header's count.
    let padded = |field: &Vec<u8>| {
        let mut bytes = field.clone();
        bytes.resize(256, 0);
        bytes
    };
    let light_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label: Some("shadow bridge lights"),
        contents: &padded(&lights.data),
        usage: BufferUsages::STORAGE,
    });
    let occluder_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label: Some("shadow bridge occluders"),
        contents: &padded(&occluders.data),
        usage: BufferUsages::STORAGE,
    });

    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("shadow bridge"),
        source: ShaderSource::Wgsl(Cow::Borrowed(SHAPE_SHADER)),
    });
    let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some("shadow bridge pipeline"),
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

    let target = device.create_texture(&TextureDescriptor {
        label: Some("shadow bridge target"),
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
            label: Some("shadow bridge pass"),
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
        for draw in draws {
            // The frame's draw list is the one list both packers and the
            // render loop walk: a `Draw::Light` carries the light into the
            // field and paints nothing itself, so the loop steps past it just
            // as the real render loop does.
            let Draw::Shape {
                world,
                center,
                params,
                kind,
                aa,
                color,
                glow,
                lit,
                ..
            } = draw
            else {
                continue;
            };
            // The render loop's recipe: the inverse of the world transform as
            // the uniform, and the full-screen triangle under it.
            let inv = world.invert().expect("a non-degenerate world");
            let uniform = device.create_buffer_init(&BufferInitDescriptor {
                label: Some("shadow bridge uniform"),
                contents: &shape_uniform_data(
                    inv, *center, *params, *kind, *aa, *color, *glow, *lit,
                ),
                usage: BufferUsages::UNIFORM,
            });
            let bind = device.create_bind_group(&BindGroupDescriptor {
                label: Some("shadow bridge bind"),
                layout: &pipeline.get_bind_group_layout(0),
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

    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("shadow bridge readback"),
        size: u64::from(SPAN) * u64::from(SPAN) * 4,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    // The same encoder that held the render pass: one submission, and the
    // copy cannot overtake the drawing it reads.
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
    let _ = device.poll(PollType::wait_indefinitely());
    assert_eq!(done.load(Ordering::SeqCst), 1, "the readback never mapped");
    let shot = readback
        .get_mapped_range(..)
        .expect("readback mapped")
        .to_vec();
    readback.unmap();
    let red: Vec<u8> = (0..SPAN * SPAN).map(|i| shot[4 * i as usize]).collect();
    Some(red)
}

/// The shadow test, end to end: what the GPU darkens is what the CPU says is
/// blocked.
///
/// Two occluders travel by the two routes an occluder can take. One is a
/// painted rectangle carrying the `occludes` flag, which is what a scene does
/// today; the other is declared through `Canvas::occluder`, which is the
/// channel for geometry nothing paints — the boxes a bake produces, a lamp
/// that should cast without being drawn. The declared one is turned by a little
/// under a third of a turn, so the record's inverse matrix is genuinely a
/// rotation and the slab test genuinely runs in a tilted space, and it passes
/// through the canvas's user-to-pixel map, which flips y.
#[test]
fn the_gpu_shadows_exactly_where_the_cpu_says() {
    // Both occluders are named by the transform they carry: the CPU side must
    // derive its box from the very transform the packer is handed, or the two
    // sides would be answering about different rectangles.
    let crate_world = Transform::identity();
    // The lit surface: white, unglowing, lit, and far larger than the target,
    // so every pixel sits deep inside it and no anti-aliased edge is in play.
    let surface = Draw::Shape {
        world: Transform::identity(),
        center: [32.0, 32.0],
        params: [80.0, 80.0],
        kind: 1.0,
        aa: 1.0,
        color: objects::Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        },
        glow: black(),
        lit: 1.0,
        occludes: 0.0,
        z: 0.0,
        diagnostic: false,
    };
    // A painted crate: invisible (alpha 0, so it composites nothing) and a
    // shadow caster all the same, since the field is built from the flag.
    let crate_box = Draw::Shape {
        world: crate_world,
        center: [30.0, 22.0],
        params: [8.0, 6.0],
        kind: 1.0,
        aa: 1.0,
        color: objects::Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 0.0,
        },
        glow: black(),
        lit: 0.0,
        occludes: 1.0,
        z: 1.0,
        diagnostic: false,
    };
    let lamp = [10.0, 32.5];
    let light = Draw::Light {
        pos: lamp,
        radius: 1000.0,
        intensity: 1.0,
        color: objects::Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        },
        // A point-sized light: one hard shadow ray per pixel, which is the
        // one question `collision::occluded` answers exactly. A penumbra would
        // average nine rays and put a soft ramp along every shadow edge, which
        // is a different claim to test.
        penumbra: 0.0,
        dir: [1.0, 0.0],
        cos_half: -1.0,
        feather: 0.0,
        z: 0.0,
    };
    let draws = [light, surface, crate_box];

    // The declared occluder, in user space: the canvas's map turns it into
    // pixel space, flipping y on the way, and its own turn survives the flip.
    let mut canvas = Canvas::new((SPAN, SPAN), 1.0);
    canvas.occluder(
        Transform::rotate(0.55).compose(&Transform::translate([14.0, -12.0])),
        [0.0, 0.0],
        [4.0, 10.0],
    );
    assert_eq!(canvas.occluders.len(), 1, "the declaration was dropped");

    let Some(red) = paint(&draws, &canvas.occluders) else {
        // The only legitimate way out, and it says so on the way: an
        // all-black frame would look exactly like this from here, so the
        // harness reports "no adapter" rather than returning empty pixels.
        return;
    };

    // The same geometry as the CPU knows it: each local box through the very
    // transform the packer was given.
    let mut occluders = vec![Collider::Box(box_in_pixels(
        &crate_world,
        [30.0, 22.0],
        [8.0, 6.0],
    ))];
    let declared = &canvas.occluders[0];
    occluders.push(Collider::Box(box_in_pixels(
        &declared.world,
        declared.center,
        declared.half,
    )));

    let blocked = |x: u32, y: u32| occluded(lamp, [x as f32 + 0.5, y as f32 + 0.5], &occluders);
    // A pixel counts as decided by the shadow test only when its whole 3×3
    // neighbourhood agrees with it. Along a shadow's edge the two sides may
    // differ by a float of rounding — the GPU clips the segment in the box's
    // space through the packed inverse matrix, the CPU in the box's space
    // through the box — and a test that samples the boundary tests the
    // quantisation rather than the geometry. Away from it, any difference at
    // all is a real disagreement.
    let settled = |x: u32, y: u32| {
        let here = blocked(x, y);
        (-1i32..=1).all(|dy| {
            (-1i32..=1).all(|dx| {
                let (cx, cy) = (x as i32 + dx, y as i32 + dy);
                cx < 0
                    || cy < 0
                    || cx as u32 >= SPAN
                    || cy as u32 >= SPAN
                    || blocked(cx as u32, cy as u32) == here
            })
        })
    };

    let mut wrong = vec![];
    let (mut checked, mut dark, mut lit) = (0usize, 0usize, 0usize);
    for y in 0..SPAN {
        for x in 0..SPAN {
            let blocked_here = blocked(x, y);
            if red[(y * SPAN + x) as usize] < 128 {
                dark += 1;
            } else {
                lit += 1;
            }
            if !settled(x, y) {
                continue;
            }
            checked += 1;
            let shadowed = red[(y * SPAN + x) as usize] < 128;
            if shadowed != blocked_here {
                wrong.push((x, y, blocked_here, red[(y * SPAN + x) as usize]));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} pixels disagree with the CPU, first five of {:?}: the shader's \
         slab test and `collision::occluded` no longer answer the same question",
        wrong.len(),
        &wrong[..wrong.len().min(5)]
    );
    // The guards that keep the comparison meaning something: a scene that
    // casts no shadow, or whose every pixel sits on an edge, would satisfy any
    // number of assertions about agreement.
    assert!(
        dark > 200 && lit > 200,
        "the scene is {dark} pixels dark and {lit} lit: it tests the agreement \
         between two implementations of a shadow only if it has both"
    );
    assert!(
        checked > (SPAN * SPAN) as usize / 2,
        "only {checked} of {} pixels sat away from a shadow edge: the \
         neighbourhood filter discarded the test rather than its noise",
        SPAN * SPAN
    );
}
