//! What the light field costs the GPU, measured off the present.
//!
//! `examples/benchsuite` counts *presented* frames, and a presented frame
//! belongs as much to the compositor as to the GPU: on a machine that paces
//! every present to the display refresh — macOS paces all of them, wgpu's
//! vsync-off or no — the suite's floor row reports the refresh rate and the
//! lighting could double or halve beneath it unseen. This bench answers the
//! same cost model where no compositor can lie: it paints the frame to a
//! texture and drains the device per frame, and the wall between submit and
//! drain is the frame.
//!
//! The scenes are the suite's own, line for line: the same golden-ratio
//! scatter, the same reach and colours and penumbra rungs, the same
//! full-window lit receiver, the same declared boxes re-declared every
//! frame, the same lamp drift. The pipeline is the one the shadow tests
//! drive — the real `SHAPE_SHADER` over packed light and occluder fields —
//! and the fields are re-packed every frame as a live frame re-packs them.
//! The names are the suite's too, so an A/B reads across; the numbers are
//! *not* the baseline's, because this frame has no bloom, no tone map, no
//! blit and no present. It measures the light's share of a frame, which is
//! the share a lighting change can move.
//!
//! The house discipline, unchanged from `docs/benchmarking.md` and the
//! draw-cost benches: warm up untimed, measure more than one pass, report a
//! median, and print the noise floor beside the numbers it qualifies. Run
//! it in release — a debug build's own CPU work caps a fill-bound frame and
//! the bench would measure the encode:
//!
//! ```text
//! cargo test --release --lib light_cost -- --ignored --nocapture
//! ```
//!
//! The bench compiles on both sides of the light-layers seam: the fields
//! the layers change added ride on lines that name them, so against a
//! checkout from before the merge
//!
//! ```text
//! sed -i '' -e 's/LightLayers, //' \
//!     -e 's/const HAS_LAYERS: bool = true;/const HAS_LAYERS: bool = false;/' \
//!     -e '/layers: LightLayers::/d' \
//!     -e '/\*lit_layers,/d' -e '/^ *lit_layers,$/d' \
//!     src/backend/tests/light_cost.rs
//! ```
//!
//! leaves the same picture and every row still a workload — which is the
//! whole claim layers made, said back by the compiler.

use super::black;

/// The window every scenario paints to: the suite's 1080p, so a row here
/// and a row there are the same picture at the same pixel count.
const W: f32 = 1920.0;
const H: f32 = 1080.0;

/// One workload, shaped exactly like `benchsuite`'s `Scenario` — plus
/// the three masks layers added: receiver, lamps and occluders each name
/// a raw bit set (bit `n` is layer `n`, so `1` is the default layer 0).
/// The layered rows are the feature's own scenes: the fold skipping a
/// light before any distance is taken (`layers-off-40`), and the whole
/// slab arithmetic of a sixty-box penumbra walk collapsing to mask skips
/// (`taps-9-layers`, and `taps-9-half` with the boxes split across the
/// seam). These are rows the suite cannot have — a pre-merge tree has no
/// masks to name — so `HAS_LAYERS` tells them apart from the ten shared
/// ones, and a stripped checkout runs ten rows, not twelve.
#[derive(Clone, Copy)]
struct Scenario {
    name: &'static str,
    lamps: usize,
    occluders: usize,
    /// Penumbra disk radius in pixels; 0 is the single hard ray.
    penumbra: f32,
    /// Taps sweeping the disk when the penumbra is open (1..=9), carried
    /// in the omni light's `feather` slot as the canvas carries it.
    taps: u32,
    /// Lamp radius: how much of the screen each light's loop can reach.
    reach: f32,
    moving: bool,
    /// The receiver's light-layer mask, raw bits.
    recv: u32,
    /// The lamps' mask: they shine only where the receiver listens.
    lamp: u32,
    /// The occluders' mask: their shadows land only on listeners.
    occ: u32,
    /// A second mask carried by the odd-numbered occluders when nonzero
    /// — the half-scene that shows the filter asks every record.
    occ2: u32,
    /// Marks a row that needs the layers' fields to exist at all.
    layered: bool,
}

/// Whether this checkout can name layers. The strip against a pre-merge
/// tree flips it — see the recipe in the module doc.
const HAS_LAYERS: bool = true;

const LAMP_REACH: f32 = 380.0;
const LAMP_PENUMBRA: f32 = 6.0;

/// The suite's ten, same order: cheap floors first, cliffs last, so a
/// ctrl-C leaves a meaningful prefix.
const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "fill-1080p",
        lamps: 0,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        recv: 1,
        lamp: 1,
        occ: 1,
        occ2: 0,
        layered: false,
    },
    Scenario {
        name: "lamps-4",
        lamps: 4,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        recv: 1,
        lamp: 1,
        occ: 1,
        occ2: 0,
        layered: false,
    },
    Scenario {
        name: "lamps-10",
        lamps: 10,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        recv: 1,
        lamp: 1,
        occ: 1,
        occ2: 0,
        layered: false,
    },
    Scenario {
        name: "lamps-40",
        lamps: 40,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: true,
        recv: 1,
        lamp: 1,
        occ: 1,
        occ2: 0,
        layered: false,
    },
    Scenario {
        name: "lamps-40-static",
        lamps: 40,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        recv: 1,
        lamp: 1,
        occ: 1,
        occ2: 0,
        layered: false,
    },
    Scenario {
        name: "occluders-10",
        lamps: 10,
        occluders: 10,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        recv: 1,
        lamp: 1,
        occ: 1,
        occ2: 0,
        layered: false,
    },
    Scenario {
        name: "occluders-100",
        lamps: 10,
        occluders: 100,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        recv: 1,
        lamp: 1,
        occ: 1,
        occ2: 0,
        layered: false,
    },
    Scenario {
        name: "taps-1",
        lamps: 10,
        occluders: 60,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: false,
        recv: 1,
        lamp: 1,
        occ: 1,
        occ2: 0,
        layered: false,
    },
    Scenario {
        name: "taps-4",
        lamps: 10,
        occluders: 60,
        penumbra: LAMP_PENUMBRA,
        taps: 4,
        reach: LAMP_REACH,
        moving: false,
        recv: 1,
        lamp: 1,
        occ: 1,
        occ2: 0,
        layered: false,
    },
    Scenario {
        name: "taps-9",
        lamps: 10,
        occluders: 60,
        penumbra: LAMP_PENUMBRA,
        taps: 9,
        reach: LAMP_REACH,
        moving: false,
        recv: 1,
        lamp: 1,
        occ: 1,
        occ2: 0,
        layered: false,
    },
    // The three rows the merge bought: same pixel counts, same fields,
    // the masks pointed away.
    Scenario {
        name: "layers-off-40",
        lamps: 40,
        occluders: 0,
        penumbra: 0.0,
        taps: 1,
        reach: LAMP_REACH,
        moving: true,
        recv: 1,
        lamp: 1 << 7,
        occ: 1,
        occ2: 0,
        layered: true,
    },
    Scenario {
        name: "taps-9-layers",
        lamps: 10,
        occluders: 60,
        penumbra: LAMP_PENUMBRA,
        taps: 9,
        reach: LAMP_REACH,
        moving: false,
        recv: 1,
        lamp: 1,
        occ: 1 << 1,
        occ2: 0,
        layered: true,
    },
    Scenario {
        name: "taps-9-half",
        lamps: 10,
        occluders: 60,
        penumbra: LAMP_PENUMBRA,
        taps: 9,
        reach: LAMP_REACH,
        moving: false,
        recv: 1,
        lamp: 1,
        occ: 1,
        occ2: 1 << 1,
        layered: true,
    },
];

/// The suite's scatter, copied so the pictures match: deterministic,
/// uniform, identical across runs and across both sides of an A/B.
fn scatter(i: usize) -> [f32; 2] {
    let k = i as f32 + 0.5;
    let gx = (k * 0.7548777_f32).fract();
    let gy = (k * 0.5698403_f32).fract();
    [gx * W * 0.94 - W * 0.47, gy * H * 0.94 - H * 0.47]
}

/// One frame's draw list: the full-window lit receiver (the suite's own
/// grey), the lamps hung at the scatter with the suite's warm white, and
/// — at the suite's own rate — the lamps' drift. Positions ride in pixel
/// space: the scatter centres on the window's middle, the harness paints
/// into its top-left.
fn frame_draws(s: &Scenario, t: f32) -> Vec<crate::backend::frame::Draw> {
    use crate::backend::frame::Draw;
    use crate::objects::{Color, LightLayers, Transform};
    let grey = Color {
        r: 0.55,
        g: 0.55,
        b: 0.58,
        a: 1.0,
    };
    let lamp = Color {
        r: 1.0,
        g: 0.95,
        b: 0.85,
        a: 1.0,
    };
    let mut draws = vec![Draw::Shape {
        world: Transform::translate([W / 2.0, H / 2.0]),
        center: [0.0, 0.0],
        params: [W, H],
        kind: 1.0,
        aa: 1.0,
        color: grey,
        emission: black(),
        glow: None,
        lit: 1.0,
        occludes: 0.0,
        z: 0.0,
        diagnostic: false,

        lit_layers: LightLayers::from_bits(s.recv),
        occludes_layers: LightLayers::default(),
    }];
    for i in 0..s.lamps {
        let [x, y] = scatter(i);
        // The suite's drift, second for second: nothing caches a shadow
        // today, and the drift keeps any tomorrow honest.
        let drift = if s.moving {
            [
                (t + i as f32).sin() * 60.0,
                (t * 0.8 + i as f32).cos() * 40.0,
            ]
        } else {
            [0.0, 0.0]
        };
        draws.push(Draw::Light {
            pos: [x + W / 2.0 + drift[0], y + H / 2.0 + drift[1]],
            radius: s.reach,
            intensity: 1.0,
            color: lamp,
            penumbra: s.penumbra,
            dir: [1.0, 0.0],
            cos_half: -1.0,
            feather: s.taps as f32,
            z: 0.0,

            layers: LightLayers::from_bits(s.lamp),
        });
    }
    draws
}

/// The frame's declared boxes, the suite's own: re-declared every frame as
/// a moving camera must, at the scatter's occluder offsets, the same
/// 180-by-120 pixel footprints.
fn declared(s: &Scenario) -> Vec<crate::canvas::Occluder> {
    use crate::canvas::Occluder;
    use crate::objects::{LightLayers, Transform};
    (0..s.occluders)
        .map(|i| {
            let [x, y] = scatter(i + s.lamps * 7919);
            // The half-scene: odd boxes ride the second mask, even ones
            // the first. The row exists to show the filter asks every
            // record — and measured, it prices well under half the
            // walk's saving, because a masked record is skipped without
            // ending the search while a matching hit ends it: filtering
            // out a record costs the walk a step it used to stop at.
            let mask = if s.occ2 != 0 && i % 2 == 1 {
                s.occ2
            } else {
                s.occ
            };
            Occluder {
                world: Transform::translate([x + W / 2.0, y + H / 2.0]),
                center: [0.0, 0.0],
                half: [90.0, 60.0],
                polygon: None,

                layers: LightLayers::from_bits(mask),
            }
        })
        .collect()
}

/// The weather the bench ran in, printed beside the numbers it qualifies —
/// the draw-cost benches' `Weather`, same rule: a bench has no baseline to
/// fail, but its milliseconds travel no further than its weather did.
struct Weather {
    canary: noisefloor::Canary,
}

impl Weather {
    fn start() -> Self {
        Self {
            canary: noisefloor::Canary::start(),
        }
    }

    fn report(self) {
        let floor = self.canary.report();
        eprintln!("light cost: noise floor {floor}");
        if floor.too_loud() {
            eprintln!(
                "light cost: too loud to judge by — the ratios travel, these milliseconds do not"
            );
        }
    }
}

/// Frames per pass, passes, and the untimed warm-up before each row — and
/// one full untimed block of the heaviest row before the first number, the
/// suite's own conditioning burn, so "what ran before the bench" is not
/// part of a number's meaning.
const WARM: u32 = 15;
const PASSES: u32 = 3;
const FRAMES: u32 = 20;

/// The suite offscreen: every scenario painted, drained per frame, the
/// median of three passes reported in fps beside its milliseconds.
#[test]
#[ignore = "a timing bench: run by hand with --ignored --nocapture"]
fn the_light_field_pays_its_fill_offscreen() {
    let weather = Weather::start();
    use crate::backend::app::block_on;
    use crate::backend::frame::{Draw, shape_uniform_data};
    use crate::backend::frame::{pack_light_field, pack_occluder_field};
    use crate::shaders::SHAPE_SHADER;
    use std::borrow::Cow;
    use std::time::Instant;
    use wgpu::util::{BufferInitDescriptor, DeviceExt};
    use wgpu::*;

    let instance = Instance::default();
    let Ok(adapter) = block_on(instance.request_adapter(&RequestAdapterOptions::default())) else {
        eprintln!("light cost: no adapter on this machine — the bench goes unrun!");
        return;
    };
    let name = adapter.get_info().name;
    let Ok((device, queue)) = block_on(adapter.request_device(&DeviceDescriptor::default())) else {
        eprintln!("light cost: no device on this machine — the bench goes unrun!");
        return;
    };

    // The lit-shape pipeline, exactly the shadow tests' bridge: the real
    // shader under an auto layout — a static per-draw uniform, as the
    // render loop's per-draw recipe binds it — over a target cleared to
    // black.
    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some("light cost shape"),
        source: ShaderSource::Wgsl(Cow::Borrowed(SHAPE_SHADER)),
    });
    let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some("light cost pipeline"),
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
        label: Some("light cost target"),
        size: Extent3d {
            width: W as u32,
            height: H as u32,
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

    // One frame, the shadow bridge's recipe end to end: pack both fields,
    // upload, draw every lit shape, submit, drain. The pack and the
    // per-draw objects are rebuilt every frame on purpose — a live frame
    // pays them, and both sides of an A/B pay them alike.
    let one_frame = |t: f32, s: &Scenario| {
        let draws = frame_draws(s, t);
        let declared = declared(s);
        let lights = pack_light_field(&draws, crate::objects::AMBIENT);
        let occluders = pack_occluder_field(&draws, &declared, [W, H]);
        let padded = |field: &Vec<u8>| {
            let mut bytes = field.clone();
            if bytes.len() < 256 {
                bytes.resize(256, 0);
            }
            bytes
        };
        let light_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("light cost field"),
            contents: &padded(&lights.data),
            usage: BufferUsages::STORAGE,
        });
        let occluder_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("light cost occluders"),
            contents: &padded(&occluders.data),
            usage: BufferUsages::STORAGE,
        });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("light cost pass"),
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
            pass.set_pipeline(&pipeline);
            for draw in &draws {
                // `Draw::Light` paints nothing itself and steps past, as
                // the render loop steps past it.
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
                    label: Some("light cost uniform"),
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
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                });
                let bind = device.create_bind_group(&BindGroupDescriptor {
                    label: Some("light cost bind"),
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
        queue.submit(Some(encoder.finish()));
        let _ = device.poll(PollType::wait_indefinitely());
    };

    eprintln!("light cost: adapter {name}, {W:.0}x{H:.0} offscreen, drained per frame");

    // The conditioning burn: the suite's heaviest row, once, untimed.
    let burn = &SCENARIOS[3];
    for i in 0..FRAMES {
        one_frame(i as f32 * 0.016, burn);
    }

    for s in SCENARIOS {
        // The layered rows need fields a pre-merge tree never had —
        // a stripped checkout skips them and prices only the ten rows
        // both trees can paint.
        if s.layered && !HAS_LAYERS {
            continue;
        }
        for i in 0..WARM {
            one_frame(i as f32 * 0.016, s);
        }
        let mut passes: Vec<f64> = Vec::with_capacity(PASSES as usize);
        for _ in 0..PASSES {
            let t = Instant::now();
            for f in 0..FRAMES {
                one_frame(f as f32 * 0.016, s);
            }
            passes.push(t.elapsed().as_secs_f64() * 1000.0 / f64::from(FRAMES));
        }
        passes.sort_by(|a, b| a.total_cmp(b));
        let ms = passes[passes.len() / 2];
        println!(
            "light cost: {:<16} {:>7.1} fps  {:>7.3} ms/frame",
            s.name,
            1000.0 / ms,
            ms
        );
    }

    weather.report();
}
