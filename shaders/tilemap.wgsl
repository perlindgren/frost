// Tile map rendering: every tile of one map batch is one instance of a
// single quad, and the fragment stage samples the tile's own
// sub-rectangle out of the shared atlas — `mix` between the instance's
// UV bounds, the same remap sprite.wgsl does per draw, but per instance.
// One instanced draw_indexed per map, whatever the map holds; clipping
// is the CPU's per-draw scissor, so nothing here knows about the view.
//
// The CPU packs four vec4 per tile in PIXEL space (top-left origin, y
// down): (px, py) — the tile's center in pixels — and (ux, uy), the
// tile's mapped x-axis half-edge; (vx, vy), the mapped y-axis half-edge
// (the quad spans center ± u ± v, so the node's rotation rides the
// edges exactly as its scale does); (min_u, min_v, max_u, max_v) — the
// atlas sub-rectangle in texture coordinates (top-left origin, the
// PNG's own orientation, so the cell's first row of texels lands on the
// quad's u-negative edge); and (r, g, b, a) — the tile's own color,
// the batch color and node modulate riding the uniform instead. The
// fragment stage multiplies texel, tint, and uniform together.
struct TileMapUniforms {
    size: vec2<f32>, // surface size in pixels
    color: vec4<f32>, // the batch's base tint (already node-modulated)
};

struct VertexOutput {
    @builtin(position)
    position: vec4<f32>,
    @location(0)
    uv_span: vec4<f32>, // the instance's atlas sub-rectangle
    @location(1)
    quad_uv: vec2<f32>, // 0..1 across the quad, y toward its lower edge
    @location(2)
    tint: vec4<f32>,
};

@group(0)
@binding(0)
var<uniform>
u: TileMapUniforms;

@group(0)
@binding(1)
var<storage, read>
instances: array<vec4<f32>>;

// The atlas: the map's image, shared with any sprite or particle batch
// sampling the same buffer.
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
    // Four vec4 per tile; the batch's draw count is the number of
    // tiles, so the instance's data sits at 4 * ii.
    let place = instances[4 * ii]; // (center, u)
    let edge = instances[4 * ii + 1]; // (v, flips, quarter-turns)
    let uv_span = instances[4 * ii + 2];
    let tint = instances[4 * ii + 3];
    // The quad's four corners, in the same order the shared index
    // buffer [0, 1, 2, 2, 1, 3] walks them (the particle batch's own
    // pattern).
    let corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vi];
    let p = place.xy + corner.x * place.zw + corner.y * edge.xy;
    // The pixel space is top-left, y down; the NDC space is bottom-left,
    // y up, so the y axis flips when mapping.
    let ndc = vec2<f32>(p.x / u.size.x * 2.0 - 1.0, 1.0 - p.y / u.size.y * 2.0);
    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv_span = uv_span;
    // The quad's own 0..1 coordinates: the ±1 corners mapped up. At
    // rest the u-negative, v-negative corner is the quad's upper-left —
    // where the atlas cell's first texel row and column belong — and
    // under rotation that corner keeps carrying uv = (0, 0) with it.
    //
    // The cell's own orientation now steers where each corner looks in
    // the atlas. A clockwise quarter-turn moves the picture so the
    // quad's upper-left shows the source's LOWER-left corner: sampling
    // (v, 1 - u) walks that corner map; the counterclockwise turn is
    // its inverse, (1 - v, u). Both are true rotations — the corner
    // cycle goes one way around the quad, never mirrored through a
    // diagonal, which is what transposes like (1 - v, 1 - u) would
    // have done. The flips apply after, on screen axes — edge.z packs
    // them as bits, x is 1 and y is 2 — so an x-flip is always
    // sideways, whatever the orientation. The CPU swapped the quad's
    // extents for odd turns, so the walk lands on a quad that already
    // fits.
    var q = corner * 0.5 + vec2<f32>(0.5, 0.5);
    switch (u32(edge.w + 0.5) % 4u) {
        case 1u: {
            q = vec2<f32>(q.y, 1.0 - q.x);
        }
        case 2u: {
            q = vec2<f32>(1.0 - q.x, 1.0 - q.y);
        }
        case 3u: {
            q = vec2<f32>(1.0 - q.y, q.x);
        }
        default: {}
    }
    let flips = u32(edge.z + 0.5);
    if ((flips & 1u) != 0u) {
        q.x = 1.0 - q.x;
    }
    if ((flips & 2u) != 0u) {
        q.y = 1.0 - q.y;
    }
    out.quad_uv = q;
    out.tint = tint;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // The tile's slice of the atlas: the quad's 0..1 run stretched
    // between the cell's UV bounds.
    let uv = mix(in.uv_span.xy, in.uv_span.zw, in.quad_uv);
    let t = textureSample(tex, samp, uv);
    // The texel's own color times the tile's tint times the batch's —
    // like a sprite's tint and alpha: the texture's alpha carries through
    // multiplied, never tinted toward opacity.
    let tint = u.color * in.tint;
    return vec4<f32>(t.rgb * tint.rgb, t.a * tint.a);
}
