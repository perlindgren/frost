//! Emission is the light a surface puts out, which means it cannot depend on
//! light reaching that surface. Until 2026-10-15 it did: both shaders applied
//! it only inside the `lit` branch, so a glowing shape in a scene with no
//! lights drew its flat color and showed nothing of the emission it carried.
//! This module pins the corrected contract — the arithmetic the field's doc
//! promises, as the GPU actually stores it, with a light field of exactly zero
//! lights in the unlit cases.

use super::super::*;
use super::*;
use crate::canvas::Occluder;
use crate::objects;
use crate::objects::*;
use crate::shaders::SHAPE_SHADER;
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::*;

/// The target's edge, in pixels, chosen so every readback row is exactly 256
/// bytes: the multiple a `copy_texture_to_buffer` row stride demands.
const SPAN: u32 = 64;

/// One painted rectangle, unlit or lit, carrying an emission of its own.
///
/// `world` is the identity on purpose: the shader measures the fragment's
/// pixel coordinate in the shape's local space, so an identity transform makes
/// local space and pixel space the same thing and every rectangle here can be
/// placed and sampled in pixels with no transform arithmetic in the way.
fn plate(at: [f32; 2], color: objects::Color, emission: objects::Color, lit: bool) -> Draw {
    Draw::Shape {
        glow: None,
        world: Transform::identity(),
        center: at,
        // Half-extents, and a rectangle: `misc.y` 1.0 (see shape.wgsl).
        params: [6.0, 6.0],
        kind: 1.0,
        // A one-pixel band, so the interior is nowhere near the edge and the
        // sampled pixel center is pure coverage.
        aa: 1.0,
        color,
        emission,
        lit: if lit { 1.0 } else { 0.0 },
        // The emission contract this module pins is layer-agnostic: the
        // plates hear every light and none of them occludes, so both sets
        // stay at the default (layer 0 — the set every layerless scene
        // shares, which is exactly what these plates were drawn in).
        lit_layers: LightLayers::default(),
        occludes: 0.0,
        occludes_layers: LightLayers::default(),
        z: 0.0,
        diagnostic: false,
    }
}

/// Which of the two outputs to bring back.
#[derive(Clone, Copy)]
enum Read {
    Color,
    Emissive,
}

/// Paint `draws` over a black clear with `ambient` as the whole light field,
/// and hand back one target's raw bytes.
///
/// `format` and `paired` are the two knobs the engine sets together in
/// production and this test sets apart: an 8-bit unorm target stores exactly
/// `round(value × 255)` with no transfer curve to undo, which is what the
/// single-target checks want, and a 32-bit float pair is what the two-output
/// entry point needs, because radiance past 1.0 has to survive the store to be
/// worth checking. Paired means the pipeline is built with both outputs and
/// `fs_emissive` as its fragment entry — the same coupling the real pipelines
/// get from one `Option` (`wgpu` refuses a one-target pipeline in a
/// two-attachment pass, so the two cannot be chosen independently).
fn paint(
    draws: &[Draw],
    ambient: objects::Color,
    format: TextureFormat,
    paired: bool,
    read: Read,
) -> Option<Vec<u8>> {
    let lights = pack_light_field(draws, ambient);
    let occluders = pack_occluder_field(draws, &[] as &[Occluder], [SPAN as f32, SPAN as f32]);

    // The thread's shared GPU: see [`super::machine`].
    let machine = super::machine::machine()?;
    let (device, queue) = (&machine.device, &machine.queue);

    // Padded to the minimum storage-binding size the same way the shadow
    // harness pads its fields; the loops stop at each header's count and never
    // read the padding.
    let padded = |field: &Vec<u8>| {
        let mut bytes = field.clone();
        if bytes.len() < 256 {
            bytes.resize(256, 0);
        }
        bytes
    };
    let light_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label: Some("emission lights"),
        contents: &padded(&lights.data),
        usage: BufferUsages::STORAGE,
    });
    let occluder_buffer = device.create_buffer_init(&BufferInitDescriptor {
        label: Some("emission occluders"),
        contents: &padded(&occluders.data),
        usage: BufferUsages::STORAGE,
    });

    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("emission"),
        source: ShaderSource::Wgsl(Cow::Borrowed(SHAPE_SHADER)),
    });
    // `layout: None` derives the bind group layout from the shader's own
    // bindings, so this test cannot drift from the uniform's real shape.
    let target_state = ColorTargetState {
        format,
        blend: Some(BlendState::ALPHA_BLENDING),
        write_mask: ColorWrites::ALL,
    };
    let targets = [
        Some(target_state),
        paired.then_some(ColorTargetState {
            format,
            blend: Some(BlendState::ALPHA_BLENDING),
            write_mask: ColorWrites::ALL,
        }),
    ];
    let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some("emission pipeline"),
        layout: None,
        vertex: VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            compilation_options: PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(FragmentState {
            module: &module,
            entry_point: Some(if paired { "fs_emissive" } else { "fs_main" }),
            compilation_options: PipelineCompilationOptions::default(),
            targets: &targets[..if paired { 2 } else { 1 }],
        }),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let new_target = |label: &str| {
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
            format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let target = new_target("emission target");
    let emissive_target = paired.then(|| new_target("emission second target"));
    let clear_ops = Operations {
        load: LoadOp::Clear(wgpu::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }),
        store: StoreOp::Store,
    };
    let color_view = target.create_view(&TextureViewDescriptor::default());
    let emissive_view = emissive_target
        .as_ref()
        .map(|t| t.create_view(&TextureViewDescriptor::default()));
    let attachments = [
        Some(RenderPassColorAttachment {
            view: &color_view,
            depth_slice: None,
            resolve_target: None,
            ops: clear_ops,
        }),
        emissive_view
            .as_ref()
            .map(|view| RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: clear_ops,
            }),
    ];
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("emission pass"),
            color_attachments: &attachments[..if paired { 2 } else { 1 }],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
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
                lit_layers,
                ..
            } = draw
            else {
                continue;
            };
            let inv = world.invert().expect("a non-degenerate world");
            let uniform = device.create_buffer_init(&BufferInitDescriptor {
                label: Some("emission uniform"),
                contents: &shape_uniform_data(
                    inv,
                    *center,
                    *params,
                    *kind,
                    *aa,
                    *color,
                    *emission,
                    *glow,
                    *lit,
                    *lit_layers,
                ),
                usage: BufferUsages::UNIFORM,
            });
            let bind = device.create_bind_group(&BindGroupDescriptor {
                label: Some("emission bind"),
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

    // A half-float target holds twice the bytes per pixel, and the copy's row
    // stride has to say so; 64 pixels of either format is already a multiple of
    // the 256-byte stride the copy demands.
    let texel: u32 = match format {
        TextureFormat::Rgba16Float => 8,
        _ => 4,
    };
    let readback = device.create_buffer(&BufferDescriptor {
        label: Some("emission readback"),
        size: u64::from(SPAN) * u64::from(SPAN) * u64::from(texel),
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let source = match read {
        Read::Color => &target,
        Read::Emissive => emissive_target
            .as_ref()
            .expect("reading the emissive target asks for a paired paint"),
    };
    encoder.copy_texture_to_buffer(
        TexelCopyTextureInfo {
            texture: source,
            mip_level: 0,
            origin: Origin3d::default(),
            aspect: TextureAspect::All,
        },
        TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SPAN * texel),
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
    Some(shot)
}

/// The pixel at the centre of a plate, as bytes.
fn pixel(shot: &[u8], at: [f32; 2]) -> [u8; 3] {
    let i = ((at[1] as usize) * SPAN as usize + at[0] as usize) * 4;
    [shot[i], shot[i + 1], shot[i + 2]]
}

/// `want` within one 8-bit step of `got` — the slack a float→unorm conversion
/// is allowed across drivers, which is not the thing under test.
fn close(got: [u8; 3], want: [f32; 3]) -> bool {
    got.iter().zip(want).all(|(g, w)| {
        let w = w * 255.0;
        (*g as f32 - w).abs() <= 1.0
    })
}

/// The four plates, one light field: an amber that is flat, the same amber
/// self-lit with no lights anywhere, the same amber lit by a grey ambient and
/// self-lit on top, and the same amber asked for more than a display holds.
#[test]
fn emission_is_what_a_surface_puts_out_not_what_reaches_it() {
    let amber = objects::Color {
        r: 0.4,
        g: 0.2,
        b: 0.1,
        a: 1.0,
    };
    let white = |a: f32| objects::Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a,
    };
    let flat = [16.0, 16.0];
    let self_lit = [48.0, 16.0];
    let lit_and_self_lit = [16.0, 48.0];
    let past_white = [48.0, 48.0];
    let draws = [
        plate(flat, amber, black(), false),
        plate(self_lit, amber, white(0.5), false),
        plate(lit_and_self_lit, amber, white(1.0), true),
        plate(past_white, amber, white(2.0), false),
    ];
    let ambient = objects::Color {
        r: 0.2,
        g: 0.2,
        b: 0.2,
        a: 1.0,
    };
    let Some(shot) = paint(
        &draws,
        ambient,
        TextureFormat::Rgba8Unorm,
        false,
        Read::Color,
    ) else {
        return;
    };

    // A flat plate is its own color: the `else` branch a shape now takes must
    // cost an unlit, unemissive surface nothing at all.
    assert!(
        close(pixel(&shot, flat), [0.4, 0.2, 0.1]),
        "an unlit plate with no emission changed its color: {:?}",
        pixel(&shot, flat)
    );
    // The case that used to draw nothing: no lights in the field, so `1 + a`
    // is the whole story, and it is the doc's formula for an unlit shape.
    assert!(
        close(pixel(&shot, self_lit), [0.6, 0.3, 0.15]),
        "emission with no light field should scale the surface by 1.5, got {:?}",
        pixel(&shot, self_lit)
    );
    // A lit plate keeps the old arithmetic exactly: the light mix and the
    // emission sum before multiplying, so `ambient + a` here.
    assert!(
        close(pixel(&shot, lit_and_self_lit), [0.48, 0.24, 0.12]),
        "a lit plate's emission left the light mix: got {:?}",
        pixel(&shot, lit_and_self_lit)
    );
    // Past 1.0 the store clamps, and it clamps per channel. The plate is
    // (0.4, 0.2, 0.1) asked for three times over: 1.2, 0.6, 0.3. Red saturates
    // and the other two are stored exactly, still in the plate's own ratio —
    // which is also proof the overshoot really happened, since an unclamped red
    // would be twice green and a clamped one is not.
    let hot = pixel(&shot, past_white);
    assert!(
        close(hot, [1.0, 0.6, 0.3]),
        "an emission of 2.0 should land at three times the plate's color, red clamped: got {hot:?}"
    );
    assert_eq!(
        hot[0], 255,
        "the channel that asked for 1.2 should saturate: {hot:?}"
    );
    assert!(
        hot[1] < 255 && hot[2] < 255,
        "the channels that stayed under 1.0 must not be clamped: {hot:?}"
    );
    // And the flat plate proves the plates did not bleed into each other.
    assert_ne!(
        pixel(&shot, flat),
        pixel(&shot, self_lit),
        "every plate read the same pixel: the plates are not where they were placed"
    );
}

/// The same four plates again, painted through the two-output entry point into
/// float targets, this time reading the emissive one.
///
/// What the second target holds is the whole point of the exercise, and it is
/// not the lit pixel: it is what each surface puts out, before the lights had
/// their say, in a form that can exceed 1.0. The three checks below are the
/// three ways that could go wrong — carrying the composited color instead,
/// letting the light field into a value that has nothing to do with it, and
/// clamping the overshoot that the target exists to keep.
#[test]
fn the_emissive_target_holds_what_the_surfaces_put_out() {
    let amber = objects::Color {
        r: 0.4,
        g: 0.2,
        b: 0.1,
        a: 1.0,
    };
    let white = |a: f32| objects::Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a,
    };
    let flat = [16.0, 16.0];
    let self_lit = [48.0, 16.0];
    let lit_and_self_lit = [16.0, 48.0];
    let past_white = [48.0, 48.0];
    let draws = [
        plate(flat, amber, black(), false),
        plate(self_lit, amber, white(0.5), false),
        plate(lit_and_self_lit, amber, white(1.0), true),
        plate(past_white, amber, white(2.0), false),
    ];
    let ambient = objects::Color {
        r: 0.2,
        g: 0.2,
        b: 0.2,
        a: 1.0,
    };
    let Some(shot) = paint(
        &draws,
        ambient,
        // The production format: whatever the second target is checked for,
        // it has to be true of the half-float the engine actually allocates.
        TextureFormat::Rgba16Float,
        true,
        Read::Emissive,
    ) else {
        return;
    };
    let glow = |at: [f32; 2]| {
        let i = ((at[1] as usize) * SPAN as usize + at[0] as usize) * 8;
        [
            half_to_f32(u16::from_le_bytes(shot[i..i + 2].try_into().unwrap())),
            half_to_f32(u16::from_le_bytes(shot[i + 2..i + 4].try_into().unwrap())),
            half_to_f32(u16::from_le_bytes(shot[i + 4..i + 6].try_into().unwrap())),
        ]
    };
    let close =
        |got: [f32; 3], want: [f32; 3]| got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 0.01);

    // A surface that emits nothing writes zero, and the zero is what keeps a
    // plate drawn later from inheriting the glow of one drawn under it.
    assert!(
        close(glow(flat), [0.0, 0.0, 0.0]),
        "a plate with no emission wrote {:?} into the emissive target",
        glow(flat)
    );
    // The radiance, in the surface's own hue: the plate's color times the
    // emission, with its alpha as the strength.
    assert!(
        close(glow(self_lit), [0.2, 0.1, 0.05]),
        "the emissive target should hold color * emission, got {:?}",
        glow(self_lit)
    );
    // The lit plate glows the same amount as it would in the dark: the ambient
    // is in the composited color (0.48, 0.24, 0.12 — the single-target test
    // above checks exactly that) and it is not in this one.
    assert!(
        close(glow(lit_and_self_lit), [0.4, 0.2, 0.1]),
        "the light field leaked into the emissive target: got {:?}, which is the composited color",
        glow(lit_and_self_lit)
    );
    // And past 1.0 the number is still there, whole. This is the whole reason
    // the target is float: 0.8 of radiance is not white yet, and an 8-bit or
    // clamped target would have thrown the difference away at the store.
    assert!(
        close(glow(past_white), [0.8, 0.4, 0.2]),
        "the float target should keep an emission of 2.0 unclamped, got {:?}",
        glow(past_white)
    );
}
