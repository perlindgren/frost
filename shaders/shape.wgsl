struct ShapeUniforms {
    to_local: mat2x2<f32>,
    translation: vec2<f32>,
    center: vec2<f32>,
    params: vec2<f32>,
    color: vec4<f32>,
    misc: vec2<f32>, // x: anti-alias band (local units), y: kind (0.0 circle, 1.0 rectangle)
    lit: f32, // 1.0 when the shape is lit by the frame's light field
    glow: vec4<f32>, // self-emission: rgb, scaled by a, added to the light mix
};

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

@group(0) @binding(0)
var<uniform> u: ShapeUniforms;

@group(0) @binding(1)
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

@group(0) @binding(2)
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
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Full-screen triangle in NDC; the scissor is set on the CPU to the
    // shape's bounding box, so fragments outside it are discarded and the
    // fragment shader only runs over the pixels the shape can write.
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[i], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    // Convert the fragment's pixel coordinate into the shape's local space
    // (the inverse of its world transform), then evaluate the signed
    // distance there, so rotation and scaling apply for free. The matrix
    // is stored column-major, so it multiplies the coordinate as a column
    // vector, matching the Rust-side transform convention.
    let p = u.to_local * frag_coord.xy + u.translation;
    var d: f32;
    if (u.misc.y < 0.5) {
        d = length(p - u.center) - u.params.x;
    } else {
        // Signed distance in local units from the fragment to the
        // rectangle border: negative inside, positive outside.
        let q = abs(p - u.center) - u.params;
        d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
    }
    let alpha = 1.0 - smoothstep(-u.misc.x, u.misc.x, d);
    // The shape's color, multiplied by the light mix when the shape is
    // lit; the lights never touch the alpha. The shape's own glow joins
    // the mix as a floor of self-light: rgb scaled by its alpha, added
    // even where the light field is black — a lit, glowing surface is
    // never fully dark, and lights still add on top.
    var rgb = u.color.rgb;
    if (u.lit > 0.5) {
        rgb *= light_mix(frag_coord.xy) + u.glow.rgb * u.glow.a;
    }
    // Emit the shape's color and coverage; composited over the existing
    // attachment via alpha blending.
    return vec4<f32>(rgb, alpha * u.color.a);
}
