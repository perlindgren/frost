// Batched particle rendering. Every particle of a batch is drawn in one
// instanced draw call: the vertex shader expands each instance into a tight
// quad around the particle's center, and the fragment shader evaluates the
// same circle SDF as circle.wgsl, with the alpha scaled by the particle's
// remaining life fraction so dying particles fade out.
struct ParticlesUniforms {
    size: vec2<f32>,  // the surface size in pixels (width, height)
    color: vec4<f32>, // the batch's base tint
};

// The vertex output: the NDC position, and the particle's circle parameters
// (center, size, life fraction) in pixel space. The center and size are
// affine across the tight quad, so interpolation reproduces them exactly at
// every fragment.
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) inst: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> u: ParticlesUniforms;

// One vec4 per particle, packed by the CPU in pixel space (top-left origin,
// y down): (px, py, size, life fraction).
@group(0) @binding(1)
var<storage, read> instances: array<vec4<f32>>;

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @builtin(instance_index) ii: u32,
) -> VertexOutput {
    let inst = instances[ii];
    let corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0,  1.0), vec2<f32>(1.0,  1.0),
    );
    // A tight quad: the circle plus its anti-alias band.
    let half = inst.z + 0.75;
    let p = inst.xy + corners[vi] * half;
    // Pixels to NDC: pixel y points down, NDC y points up.
    let ndc = vec2<f32>(p.x / u.size.x * 2.0 - 1.0, 1.0 - p.y / u.size.y * 2.0);
    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.inst = inst; // the circle's parameters for the fragment stage
    return out;
}

@fragment
fn fs_main(
    @builtin(position) frag_coord: vec4<f32>,
    @location(0) inst: vec4<f32>,
) -> @location(0) vec4<f32> {
    let d = length(frag_coord.xy - inst.xy);
    let aa = 0.75; // ~1px anti-alias band, matching circle.wgsl.
    let coverage = 1.0 - smoothstep(inst.z - aa, inst.z + aa, d);
    // The batch's base color, faded by the particle's remaining life.
    return vec4<f32>(u.color.rgb, coverage * u.color.a * inst.w);
}
