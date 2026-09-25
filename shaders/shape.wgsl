struct ShapeUniforms {
    to_local: mat2x2<f32>,
    translation: vec2<f32>,
    center: vec2<f32>,
    params: vec2<f32>,
    color: vec4<f32>,
    misc: vec2<f32>, // x: anti-alias band (local units), y: kind (0.0 circle, 1.0 rectangle)
    lit: f32, // 1.0 when the shape is lit by the frame's light field
};

// The frame's light field: one global storage buffer shared by every lit
// pipeline. The 32-byte header holds the light count, 12 padding bytes,
// and the scene's ambient color; the unsized array tail holds one 48-byte
// record per light, in call order — (x, y, radius, intensity), then
// (r, g, b, 1.0), then (dir_x, dir_y, cos_half, 0): the cone's unit axis
// and the cosine of its half opening angle. The CPU sizes the bound buffer
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
// quadratic falloff from the light's position out to its radius, and zero
// outside the light's cone and where an occluder blocks the light's path
// to the pixel. The result multiplies the unlit color:
// pixel = base * (ambient + lights).
fn light_mix(p: vec2<f32>) -> vec3<f32> {
    var c = field.ambient.rgb;
    for (var i = 0u; i < field.count; i++) {
        let l = field.lights[3 * i];
        let lc = field.lights[3 * i + 1];
        let cone = field.lights[3 * i + 2];
        let f = max(0.0, 1.0 - distance(p, l.xy) / max(l.z, 1e-4));
        // The cone gate, without normalizing d so the pixel at the
        // light's own position stays inside the cone. An omni light's
        // cos_half is -1.0, which every pixel passes.
        let d = p - l.xy;
        if (dot(d, cone.xy) >= cone.z * length(d)) {
            // A light contributes only when no occluder blocks the light from
            // this pixel — a hard shadow, with no penumbra. Outside the cone
            // the light contributes nothing anyway, so skip the raycast.
            let v = select(0.0, 1.0, !occluded(l.xy, p));
            c += lc.rgb * l.w * f * f * v;
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
    // The shape's color, multiplied by the light mix when the shape is lit;
    // the lights never touch the alpha.
    var rgb = u.color.rgb;
    if (u.lit > 0.5) {
        rgb *= light_mix(frag_coord.xy);
    }
    // Emit the shape's color and coverage; composited over the existing
    // attachment via alpha blending.
    return vec4<f32>(rgb, alpha * u.color.a);
}
