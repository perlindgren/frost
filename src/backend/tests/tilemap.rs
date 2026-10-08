//! The tile map batch: scissor arithmetic and the packed byte layouts.
//! The collection side lives with the canvas tests.

use super::black;
use crate::backend::frame::{Draw, tilemap_instance, tilemap_uniform_data};
use crate::objects::{Color, SpriteFilter};
use std::sync::Arc;

fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color { r, g, b, a }
}

/// A tile-map draw with the given pixel bounds and optional clip, over
/// one blank tile.
fn map_draw(bounds: ([f32; 2], [f32; 2]), clip: Option<([f32; 2], [f32; 2])>) -> Draw {
    Draw::TileMap {
        data: vec![0u8; 48],
        count: 1,
        color: black(),
        atlas_data: Arc::from(vec![0u8; 4].into_boxed_slice()),
        atlas_size: [1, 1],
        atlas_generation: 1,
        filter: SpriteFilter::Nearest,
        bounds,
        clip,
        z: 1.0,
        diagnostic: false,
    }
}

#[test]
fn tilemap_scissor_is_the_batch_box() {
    // Unclipped, the batch's own pixel box, as usual floored to the
    // containing pixel and spanned to the ceiling.
    let draw = map_draw(([10.0, 20.0], [50.0, 40.0]), None);
    assert_eq!(draw.scissor_rect([100, 100]), Some([10, 20, 40, 20]));
}

#[test]
fn tilemap_scissor_intersects_the_view_clip() {
    // The clip cuts the batch box; the map's tiles outside it rasterize
    // nowhere, which is exactly the point of the view rectangle.
    let draw = map_draw(
        ([10.0, 20.0], [50.0, 40.0]),
        Some(([30.0, 0.0], [70.0, 60.0])),
    );
    assert_eq!(draw.scissor_rect([100, 100]), Some([30, 20, 20, 20]));
}

#[test]
fn tilemap_clip_off_surface_skips_the_draw() {
    // A map whose view window is wholly off-screen draws nothing — the
    // scissor is None and the render loop moves on without a draw call.
    let draw = map_draw(
        ([10.0, 20.0], [50.0, 40.0]),
        Some(([60.0, 60.0], [80.0, 80.0])),
    );
    assert_eq!(draw.scissor_rect([100, 100]), None);
}

#[test]
fn tilemap_instances_pack_four_vec4s() {
    // Center and x-half-edge, y-half-edge (padded), UV bounds, tint —
    // sixteen little-endian floats, 64 bytes, in the order
    // tilemap.wgsl reads them. The edges are plain vectors: a rotated
    // node's negative or swiveled directions pass through untouched —
    // they carry the rotation.
    let bytes = tilemap_instance(
        65.0,
        25.0,
        [16.0, -16.0],
        [16.0, 16.0],
        [0.25, 0.5, 0.75, 1.0],
        &rgba(1.0, 0.5, 0.25, 0.8),
        [3.0, 1.0],
    );
    let (words, tail) = bytes.as_chunks::<4>();
    assert!(tail.is_empty());
    let floats: Vec<f32> = words.iter().map(|b| f32::from_le_bytes(*b)).collect();
    assert_eq!(
        floats,
        vec![
            65.0, 25.0, 16.0, -16.0, 16.0, 16.0, 3.0, 1.0, 0.25, 0.5, 0.75, 1.0, 1.0, 0.5, 0.25,
            0.8
        ]
    );
    // The edge vec4's two floats of slack are no longer slack: they
    // carry the cell's own orientation — flip bits (both here) and the
    // clockwise quarter-turn count — and nothing else rides there.
    assert_eq!(f32::from_le_bytes(bytes[24..28].try_into().unwrap()), 3.0);
    assert_eq!(f32::from_le_bytes(bytes[28..32].try_into().unwrap()), 1.0);
}
#[test]
fn tilemap_uniforms_layout_is_size_then_color() {
    // vec2 @ 0, vec4 @ 16 — 32 bytes, the WGSL-mirrored layout pinned by
    // the shaders test.
    let data = tilemap_uniform_data([1440.0, 810.0], rgba(1.0, 0.5, 0.0, 0.25));
    assert_eq!(data.len(), 32);
    let f = |offset: usize| f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
    assert_eq!(f(0), 1440.0);
    assert_eq!(f(4), 810.0);
    assert_eq!(f(16), 1.0);
    assert_eq!(f(20), 0.5);
    assert_eq!(f(24), 0.0);
    assert_eq!(f(28), 0.25);
}

// ── The GPU bridge ───────────────────────────────────────────────────
// The tests above pin the CPU side: the packer's bytes, the scissor's
// arithmetic. None of them can see the shader. The tileset-on-desk
// road learned that the hard way: a fragment-stage v flip moved the
// GPU's picture away from the CPU's model, and every model-vs-model
// test stayed green while the screen drifted. The two tests below are
// the bridge across that gap: each renders the real `tilemap.wgsl`
// bytes into a real framebuffer and compares every pixel against the
// contract — the walk, flips, and upright-v conventions the drawn
// expectations define — re-derived here from the screen's own
// coordinates.
//
// The first bridge crosses the packer's seam: instance bytes packed
// straight through `tilemap_instance`. The second crosses the
// geometry seam: the bytes come from the real `Canvas::draw_scene`
// collection, so the user-space → pixel-space flip, the odd-turn
// extent swap, and the multi-row atlas bands of `cell_uv` (a seam no
// measurement had ever touched: every atlas on this road was a single
// row) all ride into the shader the way the app hands them over.
// Without an adapter (headless CI with no software fallback) both
// skip, loudly.

/// The render target's edge, for both bridges.
const SPAN: u32 = 128;

/// The contract's dress, read off the screen: for a point `q` across
/// a tile (x rightward, y downward, both 0..1 from the quad's visual
/// top-left), where in the source cell the picture shows it from.
/// Each quarter-turn walks the q the drawn contract defines — the
/// clockwise turn shows the source's left edge along the top, and so
/// on — and the flips mirror on screen axes after the walk.
fn dress(q: [f32; 2], flips: u32, rot: u32) -> [f32; 2] {
    let mut q = match rot % 4 {
        1 => [q[1], 1.0 - q[0]],
        2 => [1.0 - q[0], 1.0 - q[1]],
        3 => [1.0 - q[1], q[0]],
        _ => q,
    };
    if flips & 1 != 0 {
        q[0] = 1.0 - q[0];
    }
    if flips & 2 != 0 {
        q[1] = 1.0 - q[1];
    }
    q
}

/// The GPU side of the bridge: a device, the pipeline compiled from
/// the very bytes the app compiles, and a `paint` that renders packed
/// tile-map instances over a given atlas into a `SPAN`-square
/// transparent target and hands back the framebuffer.
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
}

impl Gpu {
    /// The adapter and device, or `None` — with a shout — if this
    /// machine cannot run shaders at all. A green skip there is a
    /// hole, never a pass; hence the warning.
    fn new() -> Option<Gpu> {
        use crate::backend::app::block_on;
        use crate::shaders::TILEMAP_SHADER;
        use std::borrow::Cow;
        use wgpu::*;
        let instance = Instance::default();
        let Ok(adapter) = block_on(instance.request_adapter(&RequestAdapterOptions::default()))
        else {
            eprintln!("GPU bridge: no adapter on this machine — the shader goes unverified!");
            return None;
        };
        let Ok((device, queue)) = block_on(adapter.request_device(&DeviceDescriptor::default()))
        else {
            eprintln!("GPU bridge: no device — the shader goes unverified!");
            return None;
        };
        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("bridge tilemap"),
            source: ShaderSource::Wgsl(Cow::Borrowed(TILEMAP_SHADER)),
        });
        // The app's own pipeline shape: no explicit layout, no vertex
        // buffers (the quad comes from the vertex index), alpha
        // blending, and — like every texture in the engine, atlas and
        // target alike — plain unorm bytes, so the round trip is
        // exact to the byte.
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("bridge tilemap pipeline"),
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
        Some(Gpu {
            device,
            queue,
            pipeline,
        })
    }

    /// Render `count` packed instances over an `aw × ah` RGBA8 atlas
    /// with the batch `color`, and return the framebuffer as
    /// row-major RGBA bytes.
    fn paint(
        &self,
        instances: &[u8],
        count: u32,
        atlas: &[u8],
        aw: u32,
        ah: u32,
        color: Color,
    ) -> Vec<u8> {
        use wgpu::util::{BufferInitDescriptor, DeviceExt};
        use wgpu::*;
        let texture = |label: &str, w: u32, h: u32, usage: TextureUsages| {
            self.device.create_texture(&TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                usage,
                view_formats: &[],
            })
        };
        let atlas_tex = texture(
            "bridge atlas",
            aw,
            ah,
            TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        );
        self.queue.write_texture(
            TexelCopyTextureInfo {
                texture: &atlas_tex,
                mip_level: 0,
                origin: Origin3d::default(),
                aspect: TextureAspect::All,
            },
            atlas,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(aw * 4),
                rows_per_image: Some(ah),
            },
            Extent3d {
                width: aw,
                height: ah,
                depth_or_array_layers: 1,
            },
        );
        let target = texture(
            "bridge target",
            SPAN,
            SPAN,
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        );
        let uniform = self.device.create_buffer_init(&BufferInitDescriptor {
            label: Some("bridge uniform"),
            contents: &tilemap_uniform_data([SPAN as f32, SPAN as f32], color),
            usage: BufferUsages::UNIFORM,
        });
        let inst_buf = self.device.create_buffer_init(&BufferInitDescriptor {
            label: Some("bridge instances"),
            contents: instances,
            usage: BufferUsages::STORAGE,
        });
        let index_bytes: Vec<u8> = [0u32, 1, 2, 2, 1, 3]
            .iter()
            .flat_map(|x| x.to_ne_bytes())
            .collect();
        let index_buf = self.device.create_buffer_init(&BufferInitDescriptor {
            label: Some("bridge indices"),
            contents: &index_bytes,
            usage: BufferUsages::INDEX,
        });
        let view = atlas_tex.create_view(&TextureViewDescriptor::default());
        let sampler = self.device.create_sampler(&SamplerDescriptor {
            label: Some("bridge sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            mipmap_filter: MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let bind = self.device.create_bind_group(&BindGroupDescriptor {
            label: Some("bridge bind"),
            layout: &self.pipeline.get_bind_group_layout(0),
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
                        buffer: &inst_buf,
                        offset: 0,
                        size: None,
                    }),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&view),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Sampler(&sampler),
                },
            ],
        });
        let readback = self.device.create_buffer(&BufferDescriptor {
            label: Some("bridge readback"),
            size: u64::from(SPAN) * u64::from(SPAN) * 4,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("bridge pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &target.create_view(&TextureViewDescriptor::default()),
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Color {
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.set_index_buffer(index_buf.slice(..), IndexFormat::Uint32);
            pass.draw_indexed(0..6, 0, 0..count);
        }
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
        self.queue.submit(Some(encoder.finish()));
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
        let _ = self.device.poll(PollType::wait_indefinitely());
        assert!(done.load(Ordering::SeqCst), "the readback never mapped");
        let shot = readback
            .get_mapped_range(..)
            .expect("readback mapped")
            .to_vec();
        readback.unmap();
        shot
    }
}

/// Bridge one: packer to shader. Sixteen tiles, a 4×4 grid of 32px
/// quads, one per dress code, over a fully asymmetric 32×32 atlas —
/// every texel unique and nothing symmetric, so no mirrored or
/// turned sample can alias to a passing answer.
#[test]
fn the_shader_paints_what_the_contract_promise() {
    let Some(gpu) = Gpu::new() else { return };
    const A: u32 = 32;
    let white = rgba(1.0, 1.0, 1.0, 1.0);
    let mut atlas = vec![0u8; (A * A * 4) as usize];
    for y in 0..A {
        for x in 0..A {
            let i = ((y * A + x) * 4) as usize;
            atlas[i] = (x * 7 + y * 3 + 5) as u8;
            atlas[i + 1] = ((x * 3) ^ (y * 11)) as u8;
            atlas[i + 2] = ((x * x + y * y * 3) % 251) as u8;
            atlas[i + 3] = 255;
        }
    }
    let mut instances = Vec::new();
    for code in 0..16u32 {
        let (flips, rot) = (code & 3, code >> 2);
        let (col, row) = (code % 4, code / 4);
        instances.extend_from_slice(&tilemap_instance(
            col as f32 * 32.0 + 16.0,
            row as f32 * 32.0 + 16.0,
            [16.0, 0.0],
            [0.0, -16.0], // y-up tile in the y-down pixel space, as to_pixel packs it
            [0.0, 0.0, 1.0, 1.0],
            &white,
            [flips as f32, rot as f32],
        ));
    }
    let shot = gpu.paint(&instances, 16, &atlas, A, A, white);
    let mut wrong = Vec::new();
    for code in 0..16u32 {
        let (flips, rot) = (code & 3, code >> 2);
        let (qc, qr) = (code % 4, code / 4);
        for py in 0..32u32 {
            for pxi in 0..32u32 {
                let q = dress(
                    [(pxi as f32 + 0.5) / 32.0, (py as f32 + 0.5) / 32.0],
                    flips,
                    rot,
                );
                let sx = ((q[0] * A as f32) as u32).min(A - 1);
                let sy = ((q[1] * A as f32) as u32).min(A - 1);
                let want = ((sy * A + sx) * 4) as usize;
                let x = qc * 32 + pxi;
                let y = qr * 32 + py;
                let got = ((y * SPAN + x) * 4) as usize;
                if shot[got..got + 4] != atlas[want..want + 4] && wrong.len() < 4 {
                    wrong.push(format!(
                        "code {code} px ({x},{y}): got {:?} want {:?}",
                        &shot[got..got + 4],
                        &atlas[want..want + 4]
                    ));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "the shader drifted from the contract:\n{wrong:?}"
    );
}

/// Bridge two: the real geometry to the shader. The instance bytes
/// are not hand-packed here — a scene over a multi-row atlas (2 cols
/// × 3 rows, the row bands no measurement ever reached) goes through
/// the engine's own `Canvas::draw_scene`, and what it packs renders
/// to the pixels the contract promises. The tiles are deliberately
/// rectangular, so the odd-turn extent swap must show as a turned
/// footprint, and deliberately in a permuted cell order, so a wrong
/// band or a wrong row order has nowhere to hide; the node carries a
/// translation, so the user-space y-flip rides every quad.
#[test]
fn the_scene_paints_what_the_contract_promise() {
    use crate::objects::{Scene, SceneNode, Shape, Tile, Transform};
    let Some(gpu) = Gpu::new() else { return };
    // The atlas: 4×6 texels — cells of 2×2 in a 2×3 grid, every
    // texel's red and green unique across the whole atlas, so each
    // sample names its source texel exactly.
    const AW: u32 = 4;
    const AH: u32 = 6;
    let mut atlas = vec![0u8; (AW * AH * 4) as usize];
    for y in 0..AH {
        for x in 0..AW {
            let i = ((y * AW + x) * 4) as usize;
            atlas[i] = (x * 61 + y * 13) as u8;
            atlas[i + 1] = (x * 5 + y * 37) as u8;
            atlas[i + 2] = (x * 29 + y * 91) as u8;
            atlas[i + 3] = 255;
        }
    }
    let place = Transform::translate([8.0, -12.0]);
    // Six placements, a 2-wide × 3-tall layout seen top-down; codes
    // covering every quarter-turn and the flips; atlas rows visited
    // in a permuted order.
    let codes = [0u32, 1, 6, 13, 5, 10];
    let cell_rows = [2usize, 0, 1];
    let mut tiles = Vec::new();
    for (p, &code) in codes.iter().enumerate() {
        let (g, j) = (p % 2, p / 2);
        let (r, c) = (cell_rows[j], g);
        let mut tile = Tile::new(
            [-26.0 + g as f32 * 40.0, 22.0 - j as f32 * 30.0],
            [16.0, 10.0],
            [
                c as f32 / 2.0,
                r as f32 / 3.0,
                (c + 1) as f32 / 2.0,
                (r + 1) as f32 / 3.0,
            ],
        );
        tile.rot = (code >> 2) as u8;
        tile.flip_x = code & 1 != 0;
        tile.flip_y = code & 2 != 0;
        tiles.push(tile);
    }
    let scene = Scene::new(SceneNode {
        transform: place,
        shape: Some(Shape::TileMap {
            data: atlas.clone().into_boxed_slice().into(),
            width: AW,
            height: AH,
            generation: 1,
            filter: SpriteFilter::Nearest,
            tiles,
            clip: None,
            color: rgba(1.0, 1.0, 1.0, 1.0),
        }),
        ..Default::default()
    });
    let mut canvas = crate::Canvas::new((SPAN, SPAN), 1.0);
    canvas.draw_scene(&scene);
    let (packed, count, color) = match canvas.draws.as_slice() {
        [
            Draw::TileMap {
                data, count, color, ..
            },
        ] if *count == 6 => (data.clone(), *count, *color),
        other => panic!(
            "expected one six-tile tile-map draw, got {} draw(s)",
            other.len()
        ),
    };
    let shot = gpu.paint(&packed, count, &atlas, AW, AH, color);
    // The contract's picture, composed in pixel space from the same
    // user-space placements: the canvas maps user (ux, uy) to pixel
    // (ux + W/2, H/2 − uy) at scale 1 — the y-flip is the whole point
    // of measuring this — and an odd turn shows the cell in the
    // swapped footprint.
    let mut want = vec![0u8; (SPAN * SPAN * 4) as usize];
    for (p, &code) in codes.iter().enumerate() {
        let (g, j) = (p % 2, p / 2);
        let (r, c) = (cell_rows[j], g);
        let (flips, rot) = (code & 3, code >> 2);
        let [ucx, ucy] = place.apply([-26.0 + g as f32 * 40.0, 22.0 - j as f32 * 30.0]);
        let (pcx, pcy) = (ucx + SPAN as f32 / 2.0, SPAN as f32 / 2.0 - ucy);
        let (w, h) = if rot % 2 == 1 {
            (10.0f32, 16.0)
        } else {
            (16.0, 10.0)
        };
        let (left, top) = (pcx - w / 2.0, pcy - h / 2.0);
        let (u0, v0) = (c as f32 / 2.0, r as f32 / 3.0);
        let (du, dv) = (0.5, 1.0 / 3.0);
        for py in top as i32..(top + h) as i32 {
            for px in left as i32..(left + w) as i32 {
                let q = dress(
                    [(px as f32 - left + 0.5) / w, (py as f32 - top + 0.5) / h],
                    flips,
                    rot,
                );
                let tx = (((u0 + q[0] * du) * AW as f32) as u32).min(AW - 1);
                let ty = (((v0 + q[1] * dv) * AH as f32) as u32).min(AH - 1);
                let src = ((ty * AW + tx) * 4) as usize;
                let dst = ((py as u32 * SPAN + px as u32) * 4) as usize;
                want[dst..dst + 4].copy_from_slice(&atlas[src..src + 4]);
            }
        }
    }
    let mut wrong = Vec::new();
    for i in 0..(SPAN * SPAN) as usize {
        if shot[i * 4..i * 4 + 4] != want[i * 4..i * 4 + 4] && wrong.len() < 4 {
            let (x, y) = (i as u32 % SPAN, i as u32 / SPAN);
            wrong.push(format!(
                "px ({x},{y}): got {:?} want {:?}",
                &shot[i * 4..i * 4 + 4],
                &want[i * 4..i * 4 + 4]
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "the real geometry drifted from the contract:\n{wrong:?}"
    );
}
