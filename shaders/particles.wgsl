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
// uniform's base color and shape; when the uniform's `lit` flag is set,
// the frame's light field (ambient + the frame's lights) multiplies every
// particle's color.
struct ParticlesUniforms {
    size: vec2<f32>, // surface size in pixels
    color: vec4<f32>, // the batch's base tint (already node-modulated)
    misc: vec2<f32>, // x: shape kind (0.0 circle, 1.0 rectangle, 2.0 sprite), y: the shape's height/width factor
    lit: f32, // 1.0 when the batch is lit by the frame's light field
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

// The frame's light field: one global storage buffer shared by every lit
// pipeline. The 32-byte header holds the light count, 12 padding bytes,
// and the scene's ambient color; the unsized array tail holds one 48-byte
// record per light, in call order — (x, y, radius, intensity), then
// (r, g, b, penumbra), then (dir_x, dir_y, cos_half, feather): the
// light's color with its shadow penumbra radius in pixels, then the
// cone's unit axis, the cosine of its half opening angle, and the width
// of its edge feather in cosine units (0.0 for a hard edge; omni lights
// pass every gate). The CPU sizes the bound buffer
// to fit the frame's lights.
struct LightField {
    count: u32,
    pad: array<u32, 3>,
    ambient: vec4<f32>,
    lights: array<vec4<f32>>,
};

@group(0)
@binding(4)
var<storage, read>
field: LightField;

// The frame's occluder field: one global storage buffer shared by every lit
// pipeline. The 16-byte header holds the occluder count and 12 padding
// bytes; the unsized array tail holds one 48-byte record per occluder, in
// call order — (m00, m10, m01, m11), the inverse of the occluder's world
// transform, column-major (pixel space to the occluder's local space), then
// (tx, ty, cx, cy), the inverse transform's translation and the box's local
// center, then (hx, hy, 0, 0), the box's local half-extents. The CPU sizes
// the bound buffer to fit the frame's occluders.
struct OccluderField {
    count: u32,
    pad: array<u32, 3>,
    occluders: array<vec4<f32>>,
};

@group(0)
@binding(5)
var<storage, read>
occluders: OccluderField;

// Whether any occluder blocks the light at `l` from reaching the pixel
// `p`: true when the open segment from the light to the pixel passes
// through the interior of one of the occluders' boxes. The test runs in
// each box's own local space, where the box is axis-aligned around its
// center — the classic slab interval of a line against an axis-aligned box.
fn occluded(l: vec2<f32>, p: vec2<f32>) -> bool {
    for (var i = 0u; i < occluders.count; i++) {
        let m = occluders.occluders[3 * i];
        let tv = occluders.occluders[3 * i + 1];
        let half = occluders.occluders[3 * i + 2].xy;
        let center = tv.zw;
        // The light and the pixel in the box's local space.
        let ll = vec2<f32>(m.x * l.x + m.z * l.y + tv.x, m.y * l.x + m.w * l.y + tv.y);
        let lp = vec2<f32>(m.x * p.x + m.z * p.y + tv.x, m.y * p.x + m.w * p.y + tv.y);
        // The box never blocks the light from its own surface: a pixel on
        // or inside the box is lit directly, and the box casts its shadow
        // only on what lies behind it.
        if (all(abs(lp - center) <= half)) {
            continue;
        }
        // The interval of t in [0, 1] where the segment ll + t * (lp - ll)
        // lies inside the box, built slab by slab.
        let d = lp - ll;
        var tmin = 0.0;
        var tmax = 1.0;
        if (abs(d.x) > 1e-6) {
            let t1 = (center.x - half.x - ll.x) / d.x;
            let t2 = (center.x + half.x - ll.x) / d.x;
            tmin = max(tmin, min(t1, t2));
            tmax = min(tmax, max(t1, t2));
        } else if (ll.x < center.x - half.x || ll.x > center.x + half.x) {
            tmax = -1.0; // parallel to the slab, outside it: no crossing
        }
        if (abs(d.y) > 1e-6) {
            let t1 = (center.y - half.y - ll.y) / d.y;
            let t2 = (center.y + half.y - ll.y) / d.y;
            tmin = max(tmin, min(t1, t2));
            tmax = min(tmax, max(t1, t2));
        } else if (ll.y < center.y - half.y || ll.y > center.y + half.y) {
            tmax = -1.0;
        }
        // The segment cuts through the box when the interval has positive
        // length and reaches into the open segment (0, 1): the pixel is
        // strictly outside the box, so the exit lands before t = 1.
        if (tmax > tmin && tmax > 0.0 && tmin < 1.0) {
            return true;
        }
    }
    return false;
}

// The light mix at a pixel: the scene's ambient plus every light's
// contribution — the light's color times its intensity, scaled by the
// quadratic falloff from the light's position out to its radius, zero
// outside the light's cone — feathered to zero across its soft edges —
// and ramped to zero across its penumbra where an occluder blocks the
// light's path to the pixel. The result multiplies the unlit color:
// pixel = base * (ambient + lights).

// The shadow-ray taps on the light's penumbra disk: a 3x3 grid in unit
// space, scaled per pixel by the light's penumbra radius (the record's
// `lc.w`). A light of physical size is a disk, not a mathematical point,
// so every tap is its own shadow ray from a point of that disk.
const SHADOW_TAPS = array<vec2<f32>, 9>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(0.0, -1.0),
    vec2<f32>(1.0, -1.0),
    vec2<f32>(-1.0, 0.0),
    vec2<f32>(0.0, 0.0),
    vec2<f32>(1.0, 0.0),
    vec2<f32>(-1.0, 1.0),
    vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 1.0),
);

// How far back along a directional light's travel axis its virtual
// source sits (in pixels): shadow rays run from there to the pixel, so
// they are near-parallel across a window and the light reads as distant.
// A larger range makes shadows more parallel and the penumbra harder, as
// a source nearer the horizon's infinity would be.
const SHADOW_RANGE: f32 = 1500.0;
fn light_mix(p: vec2<f32>) -> vec3<f32> {
    var c = field.ambient.rgb;
    for (var i = 0u; i < field.count; i++) {
        let l = field.lights[3 * i];
        let lc = field.lights[3 * i + 1];
        let cone = field.lights[3 * i + 2];
        // A directional light is packed with a negative radius (see
        // `Light::directional`): it has no position and no falloff, so
        // every shadow ray runs parallel to the travel axis `cone.xy`
        // from a virtual source `src` placed `SHADOW_RANGE` back along
        // that axis through `p`. Everything downstream — the gate, the
        // penumbra taps, the occlusion ray — then treats `src` as if it
        // were an ordinary point light's position, so the parallel case
        // costs nothing new. A point or cone light (radius >= 0) keeps
        // `src` as its own position and the distance falloff unchanged.
        let directional = l.z < 0.0;
        let src = select(l.xy, p - cone.xy * SHADOW_RANGE, directional);
        let f = select(
            max(0.0, 1.0 - distance(p, l.xy) / max(l.z, 1e-4)),
            1.0,
            directional,
        );
        // The cone gate, as a 0..1 factor: omni and unfeathered cones
        // keep the hard test, without normalizing d so the pixel at the
        // light's own position stays inside the cone. An omni light's
        // cos_half is -1.0, which every pixel passes.
        let d = p - src;
        let len = length(d);
        let ad = dot(d, cone.xy);
        var gate = select(0.0, 1.0, ad >= cone.z * len);
        // A feathered cone (feather > 0) instead ramps the gate smoothly
        // from 0 at the cone edge up to 1 once the direction is `cone.w`
        // of cosine inside it — a soft edge in place of the hard cut.
        // The ramp needs d normalized; the light's own pixel keeps the
        // hard gate's answer above.
        if (cone.w > 0.0 && len > 1e-4) {
            gate = smoothstep(cone.z, cone.z + cone.w, ad / len);
        }
        if (gate > 0.0) {
            // How much of the light reaches this pixel past the
            // occluders. Outside the cone nothing contributes anyway, so
            // the raycasts are skipped entirely there. A penumbra radius
            // of 0 (a point-sized light) keeps the single hard ray;
            // otherwise nine rays issue from the light's disk and the
            // unoccluded fraction lights the pixel — the shadow's edge
            // ramps across the disk's width and widens the farther it
            // falls behind the occluder, like a real light patch. The
            // tap grid rotates per pixel (interleaved-gradient angle) so
            // the ten sample levels scatter into fine dither instead of
            // stacking into bands.
            var v = 1.0;
            if (lc.w > 0.0) {
                let ang = 6.2831853
                    * fract(52.9829 * fract(0.06711 * p.x + 0.005837 * p.y));
                let sa = sin(ang);
                let ca = cos(ang);
                var lit = 0.0;
                for (var t = 0u; t < 9u; t = t + 1u) {
                    let o = SHADOW_TAPS[t] * lc.w;
                    let q = src + vec2<f32>(ca * o.x - sa * o.y, sa * o.x + ca * o.y);
                    lit += select(0.0, 1.0, !occluded(q, p));
                }
                v = lit / 9.0;
            } else {
                // A point-sized light: the single ray from its position,
                // the original hard shadow. Without this branch the
                // default above would stand and the light would ignore
                // every occluder.
                v = select(0.0, 1.0, !occluded(src, p));
            }
            c += lc.rgb * l.w * f * f * v * gate;
        }
    }
    return c;
}

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
    // The particle's color: the batch's base tint times its own tint,
    // multiplied by the light mix when the batch is lit; the alpha is
    // additionally scaled by its remaining life fraction, and the lights
    // never touch it.
    var rgb = u.color.rgb * extra.yzw;
    if (u.lit > 0.5) {
        rgb *= light_mix(frag_coord.xy);
    }
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
