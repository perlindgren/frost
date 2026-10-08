//! Sprite loading and construction: PNG round-trips, sampling filters, buffer generations.

use crate::backend::frame::Draw;
use crate::objects::*;
use std::sync::Arc;

/// The path a test PNG is written to: under the crate's `target/`
/// directory, not the global temp dir. `cargo test` proves `target/`
/// is writable (the build just wrote it), while some machines block or
/// fill `%TEMP%`, which would fail these tests for unrelated reasons.
fn sprite_test_path(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    std::fs::create_dir_all(&dir).expect("the build just wrote to target/");
    dir.join(format!("frost-sprite-{name}-{}.png", std::process::id()))
}

#[test]
fn sprite_loader_round_trips_a_png_file() {
    let path = sprite_test_path("test");
    let buf: Vec<u8> = [
        [255u8, 0, 0, 255],
        [0, 255, 0, 128],
        [0, 0, 255, 0],
        [10, 20, 30, 40],
    ]
    .iter()
    .flat_map(|pixel| pixel.iter().copied())
    .collect();
    image::save_buffer(&path, &buf, 2, 2, image::ColorType::Rgba8).expect("writing the test png");
    let shape = match Shape::sprite(&path) {
        Ok(shape) => shape,
        Err(err) => panic!("failed to load the test png: {err}"),
    };
    let Shape::Sprite {
        data,
        width,
        height,
        color,
        alpha,
        filter,
        generation,
    } = shape
    else {
        panic!("expected a sprite shape");
    };
    assert_eq!((width, height), (2, 2));
    // Every construction stamps a buffer generation (never 0): the
    // backend's texture cache keys on `(pointer, generation)`, so a
    // freed-and-recycled buffer address can never hit a stale texture.
    assert_ne!(generation, 0);
    // The decoded RGBA8 buffer matches the file's pixels byte for byte.
    assert_eq!(&data[..], &buf[..]);
    // The default tint is white and the default opacity is 1.0.
    assert_eq!(
        color,
        Color {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0
        }
    );
    assert_eq!(alpha, 1.0);
    // The default constructor bilinear-samples the texture.
    assert_eq!(filter, SpriteFilter::Linear);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn sprite_constructors_set_their_sampling_filter() {
    // The default constructors bilinear-sample; the `*_nearest` ones
    // nearest-sample.
    let buf = [255u8, 0, 0, 255];
    let path = sprite_test_path("filter");
    image::save_buffer(&path, &buf, 1, 1, image::ColorType::Rgba8).expect("writing the test png");
    let shape = Shape::sprite(&path).expect("a valid png loads");
    let Shape::Sprite { filter, .. } = &shape else {
        panic!("expected a sprite shape");
    };
    assert_eq!(*filter, SpriteFilter::Linear);
    let shape = Shape::sprite_nearest(&path).expect("a valid png loads");
    let Shape::Sprite { filter, .. } = &shape else {
        panic!("expected a sprite shape");
    };
    assert_eq!(*filter, SpriteFilter::Nearest);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn sprite_buffers_get_unique_generations() {
    // The backend's texture cache keys on `(pointer, generation)`, and a
    // rebuilt, same-sized buffer is likely to be allocated at a freed
    // buffer's old address: without a fresh generation per construction
    // the cache would hand out the stale texture. So every construction
    // must stamp its own, and a clone (which shares the buffer) must keep
    // it.
    let buf = [10u8, 20, 30, 40];
    let path = sprite_test_path("gen");
    image::save_buffer(&path, &buf, 1, 1, image::ColorType::Rgba8).expect("writing the test png");
    let a = Shape::sprite(&path).expect("a valid png loads");
    let b = Shape::sprite(&path).expect("a valid png loads");
    let Shape::Sprite { generation: ga, .. } = &a else {
        panic!("expected a sprite shape");
    };
    let Shape::Sprite { generation: gb, .. } = &b else {
        panic!("expected a sprite shape");
    };
    assert_ne!(*ga, *gb, "two constructions need distinct generations");
    assert!(*ga >= 1, "a stamped generation is never 0");
    let Shape::Sprite { generation: gc, .. } = &a.clone() else {
        panic!("expected a sprite shape");
    };
    assert_eq!(*ga, *gc, "a clone shares its buffer and its generation");
    // Particle sprite buffers are stamped the same way.
    let p = ParticleShape::sprite(&path).expect("a valid png loads");
    let ParticleShape::Sprite { generation: pg, .. } = &p else {
        panic!("expected a sprite particle shape");
    };
    assert!(*pg >= 1, "a stamped generation is never 0");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn cloned_sprites_share_their_pixel_buffer() {
    // The pixels live behind an `Arc`: cloning a sprite shape must not
    // copy the buffer.
    let path = sprite_test_path("share");
    let buf = [255u8, 0, 0, 255, 0, 255, 0, 255];
    image::save_buffer(&path, &buf, 2, 1, image::ColorType::Rgba8).expect("writing the test png");
    let shape = Shape::sprite(&path).unwrap();
    let Shape::Sprite { data: a, .. } = &shape else {
        panic!("expected a sprite shape");
    };
    let Shape::Sprite { data: b, .. } = &shape.clone() else {
        panic!("expected a sprite shape");
    };
    assert!(Arc::ptr_eq(a, b));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn sprite_loader_reports_a_missing_file_as_io() {
    let err = Shape::sprite("frost-missing-sprite-file.png").unwrap_err();
    assert!(matches!(
        err,
        SpriteError::Io(err) if err.kind() == std::io::ErrorKind::NotFound
    ));
}

// ── The GPU bridge: sprite pixels ────────────────────────────────────
// The third bridge across the model/screen gap (the tilemap file holds
// the first two). Sprites are where the engine's oldest convention —
// local +y up, `uv` paid for by `-p.y`, sub-rect sampling for every
// text glyph — lives, and until now no pixel of any sprite had ever
// been measured in CI. Here a real scene goes through the real
// `Canvas::draw_scene`; each `Draw::Sprite` is inverted and packed
// exactly as the render loop packs it (`world.invert()`,
// `sprite_uniform_data`), rendered through the real `sprite.wgsl`
// bytes, and compared byte-exact to the contract: the upright file,
// sampled through `to_local`, remapped through `uv_rect`, tinted
// before blending. Five sprites pin the five seams: identity, mirror
// (a negative-determinant `to_local`), a text-style `uv_rect`, a tint
// multiply, and a quarter turn (a general rotation matrix — its
// inverse lands on texel centers, so f32 noise cannot flip a sample).
// No adapter: skip, loudly.
#[test]
fn the_sprite_paints_what_the_contract_promise() {
    use crate::backend::app::block_on;
    use crate::backend::frame::sprite_uniform_data;
    use crate::shaders::SPRITE_SHADER;
    use std::borrow::Cow;
    use std::sync::{Arc, atomic::AtomicU32};
    use wgpu::util::{BufferInitDescriptor, DeviceExt};
    use wgpu::*;

    const SPAN: u32 = 128;
    const A: u32 = 16;
    // Every texel unique (red names x, green names y), every value a
    // multiple of 8 so the tinted sprite's products stay exact bytes.
    let mut atlas = vec![0u8; (A * A * 4) as usize];
    for y in 0..A {
        for x in 0..A {
            let i = ((y * A + x) * 4) as usize;
            atlas[i] = (x * 8 + 8) as u8;
            atlas[i + 1] = (y * 8 + 8) as u8;
            atlas[i + 2] = ((x * 7 + y * 13) % 31) as u8 * 8;
            atlas[i + 3] = 255;
        }
    }
    let data: Arc<[u8]> = atlas.clone().into_boxed_slice().into();
    let white = crate::objects::Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    let shape = |color: crate::objects::Color| Shape::Sprite {
        data: data.clone(),
        width: A,
        height: A,
        generation: 1,
        color,
        alpha: 1.0,
        filter: SpriteFilter::Nearest,
    };
    // The five placements, 16px sprites, user space 1:1 to pixels.
    let nodes = |color: crate::objects::Color, transform: Transform, scale: [f32; 2]| SceneNode {
        transform,
        scale,
        shape: Some(shape(color)),
        ..Default::default()
    };
    let at = |x: f32, y: f32| Transform::translate([x, y]);
    let root = SceneNode {
        children: vec![
            // A: identity. B: a mirrored node (negative determinant).
            Box::new(nodes(white, at(-32.0, 32.0), [1.0, 1.0])),
            Box::new(nodes(white, at(32.0, 32.0), [-1.0, 1.0])),
            // C: identity, but its draw's uv_rect is retargeted below
            // to a text-glyph-style sub-rectangle.
            Box::new(nodes(white, at(-32.0, -32.0), [1.0, 1.0])),
            // D: a tint multiply. E: a quarter turn.
            Box::new(nodes(
                crate::objects::Color {
                    r: 0.5,
                    g: 0.25,
                    b: 1.0,
                    a: 1.0,
                },
                at(32.0, -32.0),
                [1.0, 1.0],
            )),
            // E: a 30-degree turn under non-uniform scale — the
            // only asymmetric matrix in the family, so a transposed
            // to_local packing cannot hide behind a symmetric one
            // (every other matrix here is its own transpose). The
            // angle was scanned, not guessed: its worst pixel-center
            // distance to any texel boundary is 0.0014 — 100x the
            // f32 noise of the pack-versus-shader path. 45 degrees
            // was tried first and its diagonals land ON the
            // boundaries: the test then flaps between neighbors.
            Box::new(nodes(
                white,
                Transform::rotate(std::f32::consts::FRAC_PI_6),
                [2.0, 1.0],
            )),
        ],
        ..Default::default()
    };
    let mut canvas = crate::Canvas::new((SPAN, SPAN), 1.0);
    canvas.draw_scene(&Scene::new(root));
    assert_eq!(canvas.draws.len(), 5, "five sprite nodes, five draws");
    // The glyph seam: only text ever asks for a sub-rect, so the
    // scene shape cannot carry one — retarget C's draw the way
    // expand_text does, before packing.
    let Draw::Sprite { uv_rect, .. } = &mut canvas.draws[2] else {
        panic!("draw two is a sprite");
    };
    *uv_rect = [0.25, 0.25, 0.75, 0.75];

    let instance = Instance::default();
    let Ok(adapter) = block_on(instance.request_adapter(&RequestAdapterOptions::default())) else {
        eprintln!("GPU bridge: no adapter on this machine — the shader goes unverified!");
        return;
    };
    let Ok((device, queue)) = block_on(adapter.request_device(&DeviceDescriptor::default())) else {
        eprintln!("GPU bridge: no device — the shader goes unverified!");
        return;
    };
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("sprite bridge atlas"),
        size: Extent3d {
            width: A,
            height: A,
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
        &atlas,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(A * 4),
            rows_per_image: Some(A),
        },
        Extent3d {
            width: A,
            height: A,
            depth_or_array_layers: 1,
        },
    );
    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("sprite bridge"),
        source: ShaderSource::Wgsl(Cow::Borrowed(SPRITE_SHADER)),
    });
    let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some("sprite bridge pipeline"),
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
    let view = texture.create_view(&TextureViewDescriptor::default());
    let sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("sprite bridge sampler"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        mipmap_filter: MipmapFilterMode::Nearest,
        ..Default::default()
    });
    // The light and occluder fields: never read (every sprite is
    // unlit), but the bind layout demands them.
    let dark = device.create_buffer_init(&BufferInitDescriptor {
        label: Some("sprite bridge fields"),
        contents: &[0u8; 48],
        usage: BufferUsages::STORAGE,
    });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
    let target = device.create_texture(&TextureDescriptor {
        label: Some("sprite bridge target"),
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
    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("sprite bridge pass"),
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
        for draw in &canvas.draws {
            let Draw::Sprite {
                world,
                size,
                tint,
                alpha,
                glow,
                lit,
                uv_rect,
                ..
            } = draw
            else {
                panic!("every draw is a sprite");
            };
            // The render loop's own recipe, line for line.
            let inv = world.invert().expect("a non-degenerate world");
            let uniform = device.create_buffer_init(&BufferInitDescriptor {
                label: Some("sprite bridge uniform"),
                contents: &sprite_uniform_data(inv, *size, *tint, *alpha, *glow, *lit, *uv_rect),
                usage: BufferUsages::UNIFORM,
            });
            let bind = device.create_bind_group(&BindGroupDescriptor {
                label: Some("sprite bridge bind"),
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
                        resource: BindingResource::TextureView(&view),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::Sampler(&sampler),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::Buffer(BufferBinding {
                            buffer: &dark,
                            offset: 0,
                            size: None,
                        }),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: BindingResource::Buffer(BufferBinding {
                            buffer: &dark,
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
        label: Some("sprite bridge readback"),
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
    static DONE: AtomicU32 = AtomicU32::new(0);
    DONE.store(0, std::sync::atomic::Ordering::SeqCst);
    readback.slice(..).map_async(MapMode::Read, |_| {
        DONE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let _ = device.poll(PollType::wait_indefinitely());
    assert_eq!(
        DONE.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the readback never mapped"
    );
    let shot = readback
        .get_mapped_range(..)
        .expect("readback mapped")
        .to_vec();
    readback.unmap();

    // The contract: the shader's own composition, written down from
    // the screen side — pixel centers through to_local, uv with the
    // v-flip the file demands, the sub-rect remap, the tint, and an
    // early return outside the unit square.
    let mut want = vec![0u8; (SPAN * SPAN * 4) as usize];
    for draw in &canvas.draws {
        let Draw::Sprite {
            world,
            size,
            tint,
            uv_rect,
            ..
        } = draw
        else {
            panic!("every draw is a sprite");
        };
        let inv = world.invert().expect("a non-degenerate world");
        for py in 0..SPAN {
            for px in 0..SPAN {
                let p = inv.apply([px as f32 + 0.5, py as f32 + 0.5]);
                let uv = [p[0] / size[0] + 0.5, -p[1] / size[1] + 0.5];
                if uv[0] < 0.0 || uv[0] > 1.0 || uv[1] < 0.0 || uv[1] > 1.0 {
                    continue;
                }
                let s = [
                    uv_rect[0] + (uv_rect[2] - uv_rect[0]) * uv[0],
                    uv_rect[1] + (uv_rect[3] - uv_rect[1]) * uv[1],
                ];
                let tx = (s[0] * A as f32) as u32;
                let ty = (s[1] * A as f32) as u32;
                let src = ((ty.min(A - 1) * A + tx.min(A - 1)) * 4) as usize;
                let dst = ((py * SPAN + px) * 4) as usize;
                want[dst] = (atlas[src] as f32 * tint.r) as u8;
                want[dst + 1] = (atlas[src + 1] as f32 * tint.g) as u8;
                want[dst + 2] = (atlas[src + 2] as f32 * tint.b) as u8;
                want[dst + 3] = 255;
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
        "the sprite drifted from the contract:\n{wrong:?}"
    );
}
