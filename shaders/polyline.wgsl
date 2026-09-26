// The maximum number of points the CPU may pack into one polyline draw.
// The points are stored as vec4s (x, y, 0, 0): the uniform address space
// requires an array member's stride to be a multiple of 16 bytes, and a
// vec2's stride is only 8.
const MAX_POINTS: u32 = 128u;

struct PolylineUniforms {
    // The vertices in pixel space (top-left origin, y down), connected in
    // order; only the first `count` are drawn, each packed as (x, y, 0, 0)
    // — see the MAX_POINTS note.
    points: array<vec4<f32>, MAX_POINTS>,
    color: vec4<f32>,
    count: u32,
    width: f32,
};

@group(0) @binding(0)
var<uniform> u: PolylineUniforms;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Full-screen triangle in NDC; the scissor is set on the CPU to the
    // polyline's bounding box, so fragments outside it are discarded and
    // the fragment shader only runs over the pixels the stroke can write.
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[i], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    let p = frag_coord.xy;
    let half_width = u.width * 0.5;
    // Distance in pixels from the fragment to the nearest segment: the
    // minimum over every consecutive pair of the packed points. 1e9 is far
    // beyond any screen, so it stands in for infinity.
    var dist = 1e9;
    for (var i: u32 = 0u; i + 1u < u.count; i = i + 1u) {
        let a = u.points[i].xy;
        let b = u.points[i + 1u].xy;
        let ab = b - a;
        let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
        dist = min(dist, length(p - (a + ab * t)));
    }
    let aa = 0.75; // ~1px anti-alias band, like the other shapes.
    let alpha = 1.0 - smoothstep(half_width - aa, half_width + aa, dist);
    // Emit the stroke's color and coverage; composited over the existing
    // attachment via alpha blending.
    return vec4<f32>(u.color.rgb, alpha * u.color.a);
}
