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
// and the scene's ambient color; the unsized array tail holds one 32-byte
// record per light, in call order — (x, y, radius, intensity) followed by
// (r, g, b, 1.0). The CPU sizes the bound buffer to fit the frame's lights.
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

// The light mix at a pixel: the scene's ambient plus every light's
// contribution — the light's color times its intensity, scaled by the
// quadratic falloff from the light's position out to its radius. The
// result multiplies the unlit color: pixel = base * (ambient + lights).
fn light_mix(p: vec2<f32>) -> vec3<f32> {
    var c = field.ambient.rgb;
    for (var i = 0u; i < field.count; i++) {
        let l = field.lights[2 * i];
        let lc = field.lights[2 * i + 1];
        let f = max(0.0, 1.0 - distance(p, l.xy) / max(l.z, 1e-4));
        c += lc.rgb * l.w * f * f;
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
