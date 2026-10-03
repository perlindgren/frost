// The fixed-render-size stretch: samples the frame's offscreen render
// buffer across the window — or, when the window's shape does not match
// the buffer's aspect, across the largest centered rectangle that does:
// the uniform scales the triangle down to that letterbox rectangle and
// the cleared surface shows black bars around it. One triangle, one
// sample per window pixel — plain resampling, nothing smart.
@group(0) @binding(0)
var frame_buf: texture_2d<f32>;

@group(0) @binding(1)
var frame_sampler: sampler;

struct BlitUniforms {
    fit: vec2<f32>, // content extent as a fraction of the surface; (1, 1) fills it
    pad: vec2<f32>,
};

@group(0) @binding(2)
var<uniform> u: BlitUniforms;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // Full-screen triangle in NDC — the same three vertices every
    // full-screen pass uses, so the window's edges cut it to a quad.
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let p = positions[i];
    var out: VsOut;
    // Scale the triangle into the letterbox rectangle: with fit (1, 1) it
    // is the full window, and with a smaller fit the window's edges clip
    // it to the centered content rectangle, leaving the cleared black
    // bars. The uv below keeps the *unscaled* vertex, so the rectangle
    // still receives the buffer's full [0, 1] range.
    out.clip = vec4<f32>(p * u.fit, 0.0, 1.0);
    // Texture v runs from the buffer's first row (its top edge) down, and
    // NDC y = +1 is that same top edge — so v is the flip of the clip y.
    out.uv = vec2<f32>((p.x + 1.0) / 2.0, (1.0 - p.y) / 2.0);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // The buffer was fully cleared before the frame was drawn into it, so
    // the sampled color always covers the pixel: no blending needed, the
    // pass writes it straight to the window.
    return textureSample(frame_buf, frame_sampler, in.uv);
}
