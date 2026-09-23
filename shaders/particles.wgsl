// Batched particle rendering: every particle of one batch is one instance of
// a single quad, and the fragment shader decides what the particle looks
// like — a circle, a rectangle, or a sprite (the whole texture, centered on
// the particle) — from the batch's shape kind, the same branch pattern as
// shape.wgsl. One instanced draw_indexed per batch.
//
// The CPU packs two vec4 per particle in PIXEL space (top-left origin, y
// down): (px, py, size, life fraction) and (angle, tint.r, tint.g, tint.b),
// where size is the particle's half-width in pixels, angle is the particle's
// own rotation in pixel-space radians (the node's rotation already folded
// in), and the tint is the particle's own color. The whole batch shares the
// uniform's base color and shape.
struct ParticlesUniforms {
    size: vec2<f32>, // surface size in pixels
    color: vec4<f32>, // the batch's base tint (already node-modulated)
    misc: vec2<f32>, // x: shape kind (0.0 circle, 1.0 rectangle, 2.0 sprite), y: the shape's height/width factor
};

struct VertexOutput {
    @builtin(position)
    position: vec4<f32>,
    @location(0)
    inst: vec4<f32>, // (px, py, size, life fraction)
    @location(1)
    extra: vec4<f32>, // (angle, tint.r, tint.g, tint.b)
};

@group(0)
@binding(0)
var<uniform>
u: ParticlesUniforms;

@group(0)
@binding(1)
var<storage, read>
instances: array<vec4<f32>>;

// The sampled texture: the batch's image for a sprite batch, and a 1x1
// placeholder for the SDF kinds (circles and rectangles never sample it).
@group(0)
@binding(2)
var tex: texture_2d<f32>;

@group(0)
@binding(3)
var samp: sampler;

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @builtin(instance_index) ii: u32,
) -> VertexOutput {
    // Two vec4 per particle; the batch's draw count is the number of
    // particles, so the instance's data sits at 2 * ii.
    let inst = instances[2 * ii];
    let extra = instances[2 * ii + 1];
    // The quad's corners in the shape's own frame, before rotation: the
    // same four corners the shared index buffer [0, 1, 2, 2, 1, 3] walks.
    let corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0),
    );
    // The box's half-extents in pixels: the particle's size along x, the
    // size times the shape's height/width factor along y, plus the
    // anti-alias band the SDF shapes smooth over (a sprite's texture is its
    // own edge, so it needs no band).
    let band = mix(0.75, 0.0, step(1.5, u.misc.x));
    let half = inst.z * vec2<f32>(1.0, u.misc.y) + band;
    // Rotate the box about the particle's center by its pixel-space angle.
    let c = cos(extra.x);
    let s = sin(extra.x);
    let rot = mat2x2<f32>(c, s, -s, c);
    let p = inst.xy + rot * (corners[vi] * half);
    // The pixel space is top-left, y down; the NDC space is bottom-left, y
    // up, so the y axis flips when mapping.
    let ndc = vec2<f32>(p.x / u.size.x * 2.0 - 1.0, 1.0 - p.y / u.size.y * 2.0);
    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.inst = inst;
    out.extra = extra;
    return out;
}

@fragment
fn fs_main(
    @builtin(position) frag_coord: vec4<f32>,
    @location(0) inst: vec4<f32>,
    @location(1) extra: vec4<f32>,
) -> @location(0) vec4<f32> {
    // The fragment's offset from the particle's center, rotated back into
    // the shape's own frame, so rotation applies for free. In that frame
    // the texture's top row always sits at the negative y edge.
    let c = cos(extra.x);
    let s = sin(extra.x);
    let unrot = mat2x2<f32>(c, -s, s, c);
    let off = unrot * (frag_coord.xy - inst.xy);
    // The particle's color: the batch's base tint times its own tint, with
    // the alpha additionally scaled by its remaining life fraction.
    let rgb = u.color.rgb * extra.yzw;
    let alpha = u.color.a * inst.w;
    var coverage: f32;
    if (u.misc.x < 0.5) {
        // A circle with the particle's size as its radius.
        let d = length(off) - inst.z;
        coverage = 1.0 - smoothstep(-0.75, 0.75, d);
    } else if (u.misc.x < 1.5) {
        // A rectangle with half-extents (size, size * the shape's
        // height/width factor): the same SDF as shape.wgsl's branch.
        let q = abs(off) - vec2<f32>(inst.z, inst.z * u.misc.y);
        let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
        coverage = 1.0 - smoothstep(-0.75, 0.75, d);
    } else {
        // A sprite: the whole texture, centered on the particle, filling
        // the quad exactly.
        let uv = off / (2.0 * inst.z * vec2<f32>(1.0, u.misc.y)) + vec2<f32>(0.5);
        if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
            return vec4<f32>(0.0);
        }
        let t = textureSample(tex, samp, uv);
        // The texture's own alpha multiplies the particle's alpha, like a
        // Shape::Sprite.
        return vec4<f32>(t.rgb * rgb, t.a * alpha);
    }
    // Emit the tinted color and coverage; composited over the existing
    // attachment via alpha blending.
    return vec4<f32>(rgb, coverage * alpha);
}
