//! The probe behind the frame-buffer bounds: it drives this adapter's
//! sRGB conversions through the whole 8-bit range and measures them
//! against the ideal curve, so the two bounds in `frame_buffer.rs`
//! (and the errata in BUGS.md) stay measurements of the driver that
//! answers, not folklore about the driver that calibrated.
//!
//! Two passes, one conversion each. The decode pass paints a row of
//! all 256 sRGB codes through a sampler into a `Rgba32Float` target —
//! the store there is identity, so the numbers that come back are the
//! driver's own `encoded → linear` table. The encode pass paints
//! linear values straight into an `Rgba8UnormSrgb` target, computed
//! by the fragment shader from the pixel's own index — the only
//! conversion in that pipeline is then the store's `linear →
//! encoded`, and no float-texture sampling (hence no
//! `FLOAT32_FILTERABLE`) is involved. The first 256 pixels are the
//! exact decode values of every code (the idempotence case: what
//! comes back must be the code that went in); the rest is a dense
//! sweep of linear values, where a one-step deviation near a rounding
//! tie is legal by construction and anywhere else is the number
//! worth reading.
//!
//! A third pass chains the first two — sampler fetch feeding an sRGB
//! store — which is the path every asset takes, and the one property
//! no implementation error excuses: a driver's decode and encode
//! tables must be one another's inverse at every code. That is what
//! this probe asserts, with monotonicity, exact endpoints, and a
//! deliberately generous catastrophe bound. Everything closer to the
//! ideal curve is *measured and printed*, not asserted: the spec
//! bounds the curve's shape, not an implementation's rounding of it,
//! and a number nobody verified belongs in the report, not in an
//! `assert`. The printed spread against `frame_buffer.rs`'s own f32
//! `encode` is the one its single-store bound consumes. Run by hand:
//!
//! ```text
//! cargo test --lib -- --ignored --nocapture
//! ```

#[test]
#[ignore = "a driver probe: measures this adapter's sRGB conversions against the ideal curve — run by hand with --ignored --nocapture (BUGS.md)"]
fn the_srgb_store_and_load_stay_inside_the_conformance_envelope() {
    use crate::backend::app::block_on;
    use std::borrow::Cow;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use wgpu::*;

    // The ideal curve, in f64 so the reference is not itself a
    // contestant: f32 approximates it to ~1e-7, two orders under the
    // half-step the 8-bit store rounds away.
    fn decode(c: f64) -> f64 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    fn encode(v: f64) -> f64 {
        if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }
    }
    // The shader's f32 decode, mirrored in f32 so the encode
    // expectations are computed from the value the GPU was actually
    // handed. A driver one ulp away from this curve still lands
    // thousands of times inside the narrowest code spacing.
    fn decode32(c: f32) -> f32 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    // frame_buffer.rs's reference, verbatim: the spread that probe
    // and bound consume is a spread against *this* curve, not the f64 one.
    fn reference(x: f32) -> u8 {
        let e = if x <= 0.0031308 {
            x * 12.92
        } else {
            1.055 * x.max(0.0).powf(1.0 / 2.4) - 0.055
        };
        (e.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    const CODES: u32 = 256;
    const SWEEP: u32 = 768;
    const SPAN: u32 = CODES + SWEEP;

    let instance = Instance::default();
    let adapter =
        block_on(instance.request_adapter(&RequestAdapterOptions::default())).expect("adapter");
    let info = adapter.get_info();
    let (device, queue) =
        block_on(adapter.request_device(&DeviceDescriptor::default())).expect("device");

    // The encode pass: a full-screen triangle, and a fragment value
    // built from the pixel's own index — the first 256 pixels are the
    // exact f32 decodes of every code, the rest a sweep of linear
    // values. The divisions are IEEE, so the CPU mirrors these f32s
    // bit for bit; only the powf is each side's own, and it moves a
    // value a rounding of no code spacing.
    const ENCODE_PROBE: &str = "
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4f {
    var tri = array<vec2f, 3>(vec2f(0.0, 0.0), vec2f(2.0, 0.0), vec2f(0.0, 2.0));
    return vec4f(tri[vi] * 2.0 - 1.0, 0.0, 1.0);
}

fn decode32(c: f32) -> f32 {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

@fragment
fn fs_main(@builtin(position) xy: vec4f) -> @location(0) vec4f {
    let x = i32(xy.x);
    var v: f32;
    if (x < 256) {
        v = decode32(f32(x) / 255.0);
    } else {
        v = f32(x - 256) / 767.0;
    }
    return vec4f(v, v, v, 1.0);
}
";

    // The decode pass: one row of codes through a sampler. The store
    // into the float target is identity, so every conversion in this
    // pipeline is the sampler's sRGB→linear.
    const DECODE_PROBE: &str = "
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var pt: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4f {
    var tri = array<vec2f, 3>(vec2f(0.0, 0.0), vec2f(2.0, 0.0), vec2f(0.0, 2.0));
    return vec4f(tri[vi] * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) xy: vec4f) -> @location(0) vec4f {
    let dims = vec2f(textureDimensions(src));
    // xy is already the pixel centre, so it divides into uv as it
    // stands; adding another half-pixel here would land on a texel
    // boundary and sample a column over.
    let uv = xy.xy / dims;
    return textureSampleLevel(src, pt, uv, 0.0);
}
";

    // One helper for both passes: paint a row of `width` pixels,
    // store it through `target_format`, read the bytes back. The
    // layout travels with the call because the decode pass binds a
    // texture and the encode pass binds nothing.
    let paint = |label: &'static str,
                 source: &str,
                 width: u32,
                 target_format: TextureFormat,
                 layout: Option<&PipelineLayout>,
                 bind: Option<&BindGroup>|
     -> Vec<u8> {
        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some(label),
            source: ShaderSource::Wgsl(Cow::Borrowed(source)),
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
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
                    format: target_format,
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
        let bpp = target_format.block_copy_size(None).expect("a block size");
        let target = device.create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: target_format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some(label),
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
            if let Some(bind) = bind {
                pass.set_bind_group(0, bind, &[]);
            }
            pass.draw(0..3, 0..1);
        }
        let stride = width * bpp;
        let readback = device.create_buffer(&BufferDescriptor {
            label: Some(label),
            size: u64::from(stride),
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
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(1),
                },
            },
            Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        let done = Arc::new(AtomicBool::new(false));
        {
            let done = Arc::clone(&done);
            readback.slice(..).map_async(MapMode::Read, move |r| {
                r.expect("map");
                done.store(true, Ordering::SeqCst);
            });
        }
        super::machine::drain(&device);
        assert!(done.load(Ordering::SeqCst), "the readback never mapped");
        let out = readback.get_mapped_range(..).expect("mapped").to_vec();
        readback.unmap();
        out
    };

    // The decode pass: every code through the sampler.
    let mut code_bytes = Vec::new();
    for k in 0..=255u8 {
        code_bytes.extend_from_slice(&[k, k, k, 255]);
    }
    let code_strip = device.create_texture(&TextureDescriptor {
        label: Some("srgb codes"),
        size: Extent3d {
            width: CODES,
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
            texture: &code_strip,
            mip_level: 0,
            origin: Origin3d::default(),
            aspect: TextureAspect::All,
        },
        &code_bytes,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(CODES * 4),
            rows_per_image: Some(1),
        },
        Extent3d {
            width: CODES,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let sampler = device.create_sampler(&SamplerDescriptor {
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        ..Default::default()
    });
    let decode_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
        label: Some("srgb decode probe"),
        entries: &[
            BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let decode_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
        label: Some("srgb decode probe"),
        bind_group_layouts: &[Some(&decode_layout)],
        ..Default::default()
    });
    let decode_bind = device.create_bind_group(&BindGroupDescriptor {
        label: Some("srgb decode probe"),
        layout: &decode_layout,
        entries: &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::TextureView(
                    &code_strip.create_view(&TextureViewDescriptor::default()),
                ),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Sampler(&sampler),
            },
        ],
    });
    let decoded = paint(
        "srgb decode probe",
        DECODE_PROBE,
        CODES,
        TextureFormat::Rgba32Float,
        Some(&decode_pipeline_layout),
        Some(&decode_bind),
    );

    // The third pass needs no new shader: the same sampler feeding
    // an sRGB target instead of a float one. Every asset the engine
    // paints travels exactly this path — the sampler decodes on
    // fetch, the store re-encodes — so whatever tables the driver
    // uses, they must be one another's inverse at every code. This
    // is the one property no implementation error excuses.
    let round_trip = paint(
        "srgb round trip probe",
        DECODE_PROBE,
        CODES,
        TextureFormat::Rgba8UnormSrgb,
        Some(&decode_pipeline_layout),
        Some(&decode_bind),
    );
    let mut rt_flips = 0usize;
    let mut rt_worst = 0u8;
    for k in 0..CODES {
        let got = round_trip[(k * 4) as usize];
        let d = got.abs_diff(k as u8);
        if d > 0 {
            rt_flips += 1;
        }
        if d > rt_worst {
            rt_worst = d;
        }
    }

    let mut worst_decode = 0.0f64;
    let mut worst_at = 0u32;
    let mut got_at_worst = 0.0f64;
    let mut worst_decode_codes = 0.0f64;
    let mut where_decode = 0u32;
    let mut worst_got = 0.0f64;
    let mut decode_slips = 0usize;
    let mut decode_breaks: Option<u32> = None;
    let mut last_decode: Option<(f32, f64)> = None;
    for k in 0..CODES {
        let i = (k * 16) as usize;
        let got =
            f32::from_le_bytes([decoded[i], decoded[i + 1], decoded[i + 2], decoded[i + 3]]) as f64;
        // The honest yardstick is the code's own spacing: linear is
        // steep near white, where one code is worth some twelve
        // five-hundredths of full scale, and flat near black. An
        // error of one full-scale step there would be many codes.
        let lo = decode(f64::from(k.saturating_sub(1)) / 255.0);
        let hi = decode(f64::from((k + 1).min(255)) / 255.0);
        let step = if k == 0 || k == 255 {
            hi - lo
        } else {
            (hi - lo) / 2.0
        };
        if let Some((last, last_step)) = last_decode {
            let drop = last - got as f32;
            if drop > 0.0 {
                decode_slips += 1;
                if drop as f64 >= 2.0 * step.min(last_step) {
                    decode_breaks.get_or_insert(k);
                }
            }
        }
        last_decode = Some((got as f32, step));
        let want = decode(f64::from(k) / 255.0);
        let d = (got - want).abs();
        if d > worst_decode {
            worst_decode = d;
            worst_at = k;
            got_at_worst = got;
        }
        // The honest yardstick is the code's own spacing: linear is
        // steep near white, where one code is worth some twelve
        // five-hundredths of full scale, and flat near black. An
        // error of one full-scale step there would be many codes.
        let lo = decode(f64::from(k.saturating_sub(1)) / 255.0);
        let hi = decode(f64::from((k + 1).min(255)) / 255.0);
        let step = if k == 0 || k == 255 {
            hi - lo
        } else {
            (hi - lo) / 2.0
        };
        if d / step > worst_decode_codes {
            worst_decode_codes = d / step;
            where_decode = k;
            worst_got = got;
        }
    }

    // The encode pass's expectations, in the same f32 the shader
    // computes: the pixel's value, the code the ideal curve wants,
    // and how far the ideal sits from a rounding tie (0.0) — on an
    // exact integer (0.5) only a wrong conversion disagrees, while
    // near a tie every legal implementation is free to round either
    // way and one step there is the curve talking, not the store.
    let mut want_code = vec![0u8; SPAN as usize];
    let mut sample_value = vec![0f32; SPAN as usize];
    let mut edge = vec![0.5f64; SPAN as usize];
    for k in 0..CODES as usize {
        sample_value[k] = decode32(k as f32 / 255.0);
        want_code[k] = k as u8;
    }
    for s in 0..SWEEP as usize {
        let x = CODES as usize + s;
        sample_value[x] = s as f32 / (SWEEP - 1) as f32;
        let exact = 255.0 * encode(f64::from(sample_value[x]));
        want_code[x] = exact.round().clamp(0.0, 255.0) as u8;
        edge[x] = (exact.fract() - 0.5).abs();
    }
    let stored = paint(
        "srgb encode probe",
        ENCODE_PROBE,
        SPAN,
        TextureFormat::Rgba8UnormSrgb,
        None,
        None,
    );

    let mut worst_ideal = 0u8;
    let mut flips = 0usize;
    let mut flips_far = 0usize;
    let mut worst_flip_edge = 0.0f64;
    let mut worst_reference = 0u8;
    let mut off_reference = 0usize;
    let mut encode_slips = 0usize;
    let mut encode_breaks: Option<u32> = None;
    for x in 0..SPAN {
        let i = (x * 4) as usize;
        if x > 0 && x != CODES {
            // Two separate ramps live in this span — the idempotence
            // values and the sweep — and only each ramp climbs.
            for ch in 0..3 {
                let (now, prev) = (stored[i + ch], stored[i - 4 + ch]);
                if now < prev {
                    encode_slips += 1;
                    if prev - now >= 2 {
                        encode_breaks.get_or_insert(x);
                    }
                }
            }
        }
        for ch in 0..3 {
            let got = stored[i + ch];
            let d_ideal = got.abs_diff(want_code[x as usize]);
            if d_ideal > 0 {
                flips += 1;
                if edge[x as usize] >= 0.02 {
                    flips_far += 1;
                }
                if edge[x as usize] > worst_flip_edge {
                    worst_flip_edge = edge[x as usize];
                }
            }
            if d_ideal > worst_ideal {
                worst_ideal = d_ideal;
            }
            let d_reference = got.abs_diff(reference(sample_value[x as usize]));
            if d_reference > 0 {
                off_reference += 1;
            }
            if d_reference > worst_reference {
                worst_reference = d_reference;
            }
        }
    }

    eprintln!("probe: adapter '{}' ({:?})", info.name, info.device_type);
    eprintln!(
        "probe: round trip (sampler fetch → sRGB store) — worst {rt_worst}/255, {rt_flips} \
         codes moved"
    );
    eprintln!(
        "probe: decode — worst absolute error at code {worst_at}: got {got_at_worst:.7}, ideal \
         {:.7}, {worst_decode:.7} linear",
        decode(f64::from(worst_at) / 255.0)
    );
    eprintln!(
        "probe: decode — worst code-relative error at code {where_decode}: got {worst_got:.7}, \
         ideal {:.7}, {worst_decode_codes:.3} of that code's own step",
        decode(f64::from(where_decode) / 255.0)
    );
    eprintln!(
        "probe: encode — worst {worst_ideal}/255 off the ideal; {flips} flips, {flips_far} of \
         them at or beyond 0.02 of a step from a rounding tie (worst such edge \
         {worst_flip_edge:.3}); {worst_reference}/255 worst and {off_reference} samples off \
         frame_buffer.rs's f32 reference"
    );
    eprintln!(
        "probe: monotonicity — decode slipped {decode_slips} times (breaks: {decode_breaks:?}), \
         encode slipped {encode_slips} times (breaks: {encode_breaks:?})"
    );

    // The asserted set needs no unverified tolerance: it is what any
    // implementation owes, however its tables round. A sampler's
    // decode and the store's encode are one contract — a texture
    // painted straight back must return its bytes. Neither table
    // may step backwards by two: one step of reversal is what
    // jitter around the ideal curve looks like when consecutive
    // samples sit a sixth of a step apart, two is a table crossing
    // itself. The two ends are exact, because the curve meets its
    // own endpoints. Beyond these, the probe *measures*: distance
    // from the ideal curve and the spread against `frame_buffer.rs`'s
    // reference are this driver's signature, printed for BUGS.md's
    // errata — and the bound named catastrophe below is not
    // conformance but the tripwire that says a conversion has given
    // up rather than approximated.
    assert_eq!(
        rt_worst, 0,
        "the sampler-then-store round trip moved {} codes by up to {rt_worst}/255 on '{}'; a \
         driver's decode and encode tables are one contract — this is an upstream bug, and \
         every asset painted on this driver drifts",
        rt_flips, info.name
    );
    assert!(
        decode_breaks.is_none() && encode_breaks.is_none(),
        "a conversion stepped backwards by two codes (decode at {:?}, encode at {:?}) on '{}'; \
         one step is jitter, two is a broken table",
        decode_breaks,
        encode_breaks,
        info.name
    );
    assert_eq!(
        (
            decoded[0],
            decoded[1],
            decoded[2],
            decoded[3],
            decoded[(255 * 16) as usize],
            decoded[(255 * 16 + 1) as usize],
            decoded[(255 * 16 + 2) as usize],
            decoded[(255 * 16 + 3) as usize],
        ),
        (0, 0, 0, 0, 0, 0, 128, 63),
        "the curve missed its own endpoints on '{}': code 0 must decode to 0.0 and code 255 \
         to 1.0",
        info.name
    );
    assert_eq!(
        (stored[0], stored[4 * (SPAN - 1) as usize]),
        (0, 255),
        "the encode pass missed an endpoint on '{}': linear 0 must store 0 and linear 1 must \
         store 255",
        info.name
    );
    assert!(
        worst_decode <= 5e-3 && worst_ideal <= 4,
        "a conversion moved by decode {worst_decode:.7} linear / encode {worst_ideal}/255 on \
         '{}' — past any approximation we have seen a working driver make; stop and look at \
         the driver, not at the bounds in BUGS.md",
        info.name
    );
}
