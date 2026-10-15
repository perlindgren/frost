//! The bloom chain: the emissive target in, three blurred levels out.
//!
//! Three levels, each half the linear size of the one above, each made of
//! three full-screen passes — a tent downsample, then a separable Gaussian
//! across and down. Each step down is a quarter of the area, so the whole
//! chain costs about a third of one full-resolution pass however big the window
//! is, and the widest glow is the cheapest of the three — the opposite of what
//! one wide blur at full resolution costs.
//!
//! The levels come out *separate* rather than accumulated into one, because
//! the present pass wants them separate: a glow's width is a look, and three
//! weights chosen where the frame is composited are easier to reason about than
//! one blur radius chosen where it is generated. [`Bloom::weights`] carries them
//! to `blit.wgsl`, already multiplied by the user's strength.
//!
//! Nothing here knows what bloom is for. It takes a texture of radiance and
//! returns the same picture smeared at three scales; the engine hands it the
//! emissive target ([`crate::SceneNode::emission`]) and the present pass adds
//! the result back. Change either end and this module does not notice.
//!
//! [`crate::SceneNode::emission`]: crate::SceneNode::emission

use crate::shaders::BLOOM_SHADER;
use std::borrow::Cow;
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindingResource, Buffer, BufferBinding,
    BufferDescriptor, BufferUsages, Color, CommandEncoder, Device, Extent3d, FilterMode,
    FragmentState, LoadOp, PipelineCompilationOptions, Queue, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, Sampler, SamplerDescriptor,
    ShaderModule, ShaderModuleDescriptor, ShaderSource, StoreOp, TexelCopyBufferLayout, Texture,
    TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor, VertexState,
};

/// How far the chain goes down: halves of the frame's linear size, so a 4K
/// frame ends at 480×270. Three is roughly where a glow stops reading as a halo
/// around the lamp and starts reading as the room being lit by it; a fourth
/// level costs nearly nothing and mostly makes the whole frame grey.
pub(crate) const LEVELS: usize = 3;

/// Radiance under this contributes nothing to the glow, and [`KNEE`] decides
/// how widely the band above it arrives. Below 1.0 on purpose: an emission
/// authored at exactly white is common, and a threshold at 1.0 would leave it
/// out of the effect altogether — a lamp would bloom only if someone remembered
/// that a colour is allowed to hold `2.0`.
const THRESHOLD: f32 = 0.8;

/// How wide a band the threshold ramps across, in radiance units. A hard
/// threshold draws a contour *on* the light — the lamp's rim falls below it
/// while its core stays above — so this ramp is where the behaviour the docs
/// promise lives: authored past 1.0 blooms hard, ordinary bright blooms a
/// little, and nothing jumps between the two.
const KNEE: f32 = 0.4;

/// What each level adds to the window, fine to coarse, summing to 1 so that a
/// frame of bloom cannot double the frame's brightness. The fine level leads
/// because it is the one that still holds the lamp's shape; the coarse ones are
/// the haze, and too much haze reads as a dirty lens.
const WEIGHTS: [f32; LEVELS] = [0.5, 0.3, 0.2];

/// One level: two textures — the one the level is read from and the scratch the
/// horizontal blur lands in — plus the three bind groups that write them.
struct Level {
    /// The level's texture: written by the downsample, written again by the
    /// vertical blur (in place, from the scratch), and sampled by the present
    /// pass. Held beside its view so the ownership is obvious, as everywhere
    /// else in the backend.
    _fine: Texture,
    fine_view: TextureView,
    /// The scratch the horizontal blur lands in, so the vertical pass never
    /// reads a texture it is also writing.
    _scratch: Texture,
    scratch_view: TextureView,
    /// Emissive target, or the level above → `fine`.
    down: BindGroup,
    /// `fine` → `scratch`, across.
    across: BindGroup,
    /// `scratch` → `fine`, down.
    along: BindGroup,
}

/// The frame's bloom chain, built for one render target's size.
///
/// It lives and dies with the target: the levels are fractions of its extent, so
/// a resize means a new chain, not a scaled one. A chain with no bloom to do is
/// still a `Bloom` ([`Bloom::dormant`]), because the present pass binds three
/// bloom textures whatever the answer is; what differs is whether its weights
/// are zero, which is what keeps the samples out of the frame.
pub(crate) struct Bloom {
    /// The levels, fine first. Empty when dormant, which is how `run` knows to
    /// draw nothing.
    levels: Vec<Level>,
    /// `[downsample, blur]`, in that order.
    pipelines: [RenderPipeline; 2],
    /// One sampler for the chain's fetches and the present pass's: linear,
    /// clamped. Clamping matters more than the filtering — a glow at the
    /// frame's edge samples outside it constantly, and a wrapping edge drags
    /// the far side of the picture around it.
    pub(crate) sampler: Sampler,
    /// What the present pass binds, fine first: the three levels, or one black
    /// texel bound three times.
    pub(crate) views: [TextureView; LEVELS],
    /// The present pass's weights, strength already multiplied in: all zero is
    /// how "no bloom" reaches the shader, whose branch on it keeps the three
    /// samples off the frame entirely.
    pub(crate) weights: [f32; LEVELS],
    /// The passes' uniforms, one each, written once here and never again. Held
    /// because a bind group keeps its buffers alive but the chain wants to say
    /// how much it weighs.
    _uniforms: Vec<Buffer>,
}

impl Bloom {
    /// Builds the chain for a frame buffer of `extent` reading `emissive`.
    ///
    /// `strength` is [`Config::bloom`](crate::Config::bloom). Anything at or
    /// below zero — and any frame with no float target to read, which is the
    /// same answer the emissive target itself gets — gives a
    /// [dormant](Self::dormant) chain.
    pub(crate) fn new(
        device: &Device,
        queue: &Queue,
        format: TextureFormat,
        extent: [u32; 2],
        emissive: Option<&TextureView>,
        strength: f32,
    ) -> Self {
        let module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("bloom shaders"),
            source: ShaderSource::Wgsl(Cow::Borrowed(BLOOM_SHADER)),
        });
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("bloom sampler"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..Default::default()
        });
        let pipelines = [
            Self::pipeline(device, &module, format, "bloom downsample", "fs_down"),
            Self::pipeline(device, &module, format, "bloom blur", "fs_blur"),
        ];
        let Some(emissive) = emissive.filter(|_| strength > 0.0) else {
            return Self::dormant(device, queue, format, pipelines, sampler, strength);
        };
        let mut levels: Vec<Level> = Vec::with_capacity(LEVELS);
        let mut uniforms = Vec::with_capacity(LEVELS * 3);
        // What the next level reads: the emissive target at the top of the
        // chain, this level's blurred result below that. Blurred, not raw —
        // the light is chosen once, at the top.
        let mut source = emissive;
        let mut source_extent = extent;
        for level_index in 0..LEVELS {
            let extent = [(source_extent[0] / 2).max(1), (source_extent[1] / 2).max(1)];
            let new = |label: &str| {
                device.create_texture(&TextureDescriptor {
                    label: Some(label),
                    size: Extent3d {
                        width: extent[0],
                        height: extent[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format,
                    // `COPY_SRC` for one reason: a test reads a level back to
                    // see what the chain actually made, which is the only way
                    // to tell a dim glow from an empty one.
                    usage: TextureUsages::RENDER_ATTACHMENT
                        | TextureUsages::TEXTURE_BINDING
                        | TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            };
            let fine = new("bloom level");
            let scratch = new("bloom blur scratch");
            let fine_view = fine.create_view(&TextureViewDescriptor::default());
            let scratch_view = scratch.create_view(&TextureViewDescriptor::default());
            let src_texel = [1.0 / source_extent[0] as f32, 1.0 / source_extent[1] as f32];
            let texel = [1.0 / extent[0] as f32, 1.0 / extent[1] as f32];
            let mut bind = |label: &'static str,
                            pipeline: &RenderPipeline,
                            source: &TextureView,
                            bytes: [u8; 32]|
             -> BindGroup {
                let uniform = device.create_buffer(&BufferDescriptor {
                    label: Some(label),
                    size: 32,
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&uniform, 0, &bytes);
                let group = device.create_bind_group(&BindGroupDescriptor {
                    label: Some(label),
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[
                        BindGroupEntry {
                            binding: 0,
                            resource: BindingResource::TextureView(source),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: BindingResource::Sampler(&sampler),
                        },
                        BindGroupEntry {
                            binding: 2,
                            resource: BindingResource::Buffer(BufferBinding {
                                buffer: &uniform,
                                offset: 0,
                                size: None,
                            }),
                        },
                    ],
                });
                uniforms.push(uniform);
                group
            };
            let down = bind(
                "bloom downsample bind",
                &pipelines[0],
                source,
                // The threshold rides at the top of the chain only: below it
                // the bright parts have already been chosen, and thresholding
                // again would redraw the same contour at every scale.
                bloom_bytes(
                    src_texel,
                    if level_index == 0 { THRESHOLD } else { -1.0 },
                    KNEE,
                    [0.0, 0.0],
                ),
            );
            // One axis each: the step for the across pass is the horizontal
            // texel alone, and for the along pass the vertical one. Passing
            // both makes the "separable" blur a diagonal smear, which is the
            // single easiest way to get this wrong and the single easiest way
            // to see it: the glow leans.
            let across = bind(
                "bloom blur across bind",
                &pipelines[1],
                &fine_view,
                bloom_bytes(texel, -1.0, KNEE, [texel[0], 0.0]),
            );
            // One pass per axis: the kernel is wide enough that a second
            // iteration would only cost another pass, which is the expensive
            // thing about this chain.
            let along = bind(
                "bloom blur along bind",
                &pipelines[1],
                &scratch_view,
                bloom_bytes(texel, -1.0, KNEE, [0.0, texel[1]]),
            );
            levels.push(Level {
                _fine: fine,
                fine_view,
                _scratch: scratch,
                scratch_view,
                down,
                across,
                along,
            });
            source_extent = extent;
            // The borrow checker's version of "the chain feeds itself": the
            // next level reads the `fine` view that just moved into `levels`.
            source = &levels[level_index].fine_view;
        }
        let views = [
            levels[0].fine_view.clone(),
            levels[1].fine_view.clone(),
            levels[2].fine_view.clone(),
        ];
        let weights = [
            WEIGHTS[0] * strength,
            WEIGHTS[1] * strength,
            WEIGHTS[2] * strength,
        ];
        Self {
            levels,
            pipelines,
            sampler,
            views,
            weights,
            _uniforms: uniforms,
        }
    }

    /// A chain that does nothing, for a frame with no float target or an app
    /// that asked for no bloom. The present pass still binds three textures, so
    /// three exist — one black texel, three times — and the weights are zero, so
    /// nothing looks at them.
    fn dormant(
        device: &Device,
        queue: &Queue,
        format: TextureFormat,
        pipelines: [RenderPipeline; 2],
        sampler: Sampler,
        strength: f32,
    ) -> Self {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("bloom placeholder"),
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // One black texel, actually written. A texture that exists only to be
        // bound could be left uninitialised — nothing samples it, its weights
        // are zero — but "nothing samples it" is a promise about a branch, and
        // a texel costs four bytes.
        queue.write_texture(
            texture.as_image_copy(),
            &[0, 0, 0, 0, 0, 0, 0, 0],
            TexelCopyBufferLayout {
                bytes_per_row: Some(8),
                rows_per_image: Some(1),
                ..Default::default()
            },
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&TextureViewDescriptor::default());
        if strength > 0.0 {
            log::info!(
                "bloom asked for at {strength} on a frame buffer that clips at 1.0: ignored"
            );
        }
        Self {
            levels: Vec::new(),
            pipelines,
            sampler,
            views: [view.clone(), view.clone(), view],
            weights: [0.0; LEVELS],
            _uniforms: Vec::new(),
        }
    }

    /// A full-screen pass: no vertex buffers, one float target it overwrites
    /// whole. Nothing blends here — the chain computes, it does not composite.
    fn pipeline(
        device: &Device,
        module: &ShaderModule,
        format: TextureFormat,
        label: &str,
        entry_point: &str,
    ) -> RenderPipeline {
        device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some(label),
            layout: None,
            vertex: VertexState {
                module,
                entry_point: Some("vs_main"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(FragmentState {
                module,
                entry_point: Some(entry_point),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            cache: None,
            multiview_mask: None,
        })
    }

    /// Runs the chain: emissive in, three blurred levels out, nine full-screen
    /// passes over a third of one frame's pixels. Does nothing when dormant.
    pub(crate) fn run(&self, encoder: &mut CommandEncoder) {
        for level in &self.levels {
            let passes = [
                (&self.pipelines[0], &level.down, &level.fine_view),
                (&self.pipelines[1], &level.across, &level.scratch_view),
                (&self.pipelines[1], &level.along, &level.fine_view),
            ];
            for (pipeline, bind_group, target) in passes {
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("bloom pass"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            // The triangle covers every pixel of every level,
                            // so clearing is a tiler's convenience rather than
                            // something the frame depends on.
                            load: LoadOp::Clear(Color::BLACK),
                            store: StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
    }
}

/// The chain's uniform: source texel size, threshold (negative to skip), knee,
/// blur step. 32 bytes, written once when the chain is built and never again —
/// which is what keeps nine passes a frame from costing any staging.
fn bloom_bytes(texel: [f32; 2], threshold: f32, knee: f32, spread: [f32; 2]) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[0..4].copy_from_slice(&texel[0].to_le_bytes());
    b[4..8].copy_from_slice(&texel[1].to_le_bytes());
    b[8..12].copy_from_slice(&threshold.to_le_bytes());
    b[12..16].copy_from_slice(&knee.to_le_bytes());
    b[16..20].copy_from_slice(&spread[0].to_le_bytes());
    b[20..24].copy_from_slice(&spread[1].to_le_bytes());
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The uniform's layout, pinned against `bloom.wgsl`'s `BloomUniforms`:
    /// texel at 0, threshold at 8, knee at 12, spread at 16. A drift here is a
    /// chain that blurs by the wrong step or never thresholds at all, and every
    /// symptom is a picture that merely looks slightly wrong.
    #[test]
    fn the_bloom_uniform_lands_where_the_shader_reads_it() {
        let b = bloom_bytes([0.25, 0.5], 0.8, 0.4, [0.001, 0.002]);
        let f = |r: std::ops::Range<usize>| f32::from_le_bytes(b[r].try_into().unwrap());
        assert_eq!((f(0..4), f(4..8)), (0.25, 0.5));
        assert_eq!((f(8..12), f(12..16)), (0.8, 0.4));
        assert_eq!((f(16..20), f(20..24)), (0.001, 0.002));
        assert_eq!(&b[24..], &[0u8; 8]);
    }

    /// The weights sum to the frame they are added to: at a strength of 1.0 the
    /// chain returns roughly the light it was given, spread out, rather than
    /// tripling it.
    #[test]
    fn the_levels_add_up_to_one_light() {
        let sum: f32 = WEIGHTS.iter().sum();
        assert!(
            (sum - 1.0).abs() < 1e-6,
            "the three levels add to {sum}, which would change the frame's \
             total brightness at a strength of 1.0"
        );
        assert!(
            WEIGHTS.windows(2).all(|w| w[0] > w[1]),
            "the fine level should lead: {WEIGHTS:?}"
        );
    }
}
