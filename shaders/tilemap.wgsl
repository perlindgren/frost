// Tile map rendering: every tile of one map batch is one instance of a
// single quad, and the fragment stage samples the tile's own
// sub-rectangle out of the shared atlas — `mix` between the instance's
// UV bounds, the same remap sprite.wgsl does per draw, but per instance.
// One instanced draw_indexed per map, whatever the map holds; clipping
// is the CPU's per-draw scissor, so nothing here knows about the view.
//
// The CPU packs three vec4 per tile in PIXEL space (top-left origin, y
// down): (px, py, hw, hh) — the tile's center and half-extents in
// pixels; (min_u, min_v, max_u, max_v) — the atlas sub-rectangle in
// texture coordinates (top-left origin, the PNG's own orientation, so
// the cell's first row of texels lands on the quad's upper edge); and
// (r, g, b, a) — the tile's tint, the batch's color times the node's
// modulate times the tile's own color. The whole batch also shares the
// uniform's base color, multiplied again in the fragment stage.
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
    // Three vec4 per tile; the batch's draw count is the number of
    // tiles, so the instance's data sits at 3 * ii.
    let place = instances[3 * ii];
    let uv_span = instances[3 * ii + 1];
    let tint = instances[3 * ii + 2];
    // The quad's four corners, in the same order the shared index
    // buffer [0, 1, 2, 2, 1, 3] walks them (the particle batch's own
    // pattern).
    let corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vi];
    let p = place.xy + corner * place.zw;
    // The pixel space is top-left, y down; the NDC space is bottom-left,
    // y up, so the y axis flips when mapping.
    let ndc = vec2<f32>(p.x / u.size.x * 2.0 - 1.0, 1.0 - p.y / u.size.y * 2.0);
    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv_span = uv_span;
    // The quad's own 0..1 coordinates: the pixel corners mapped up. The
    // pixel space is y down, so quad_uv = (0, 0) lands on the quad's
    // upper-left corner — where the atlas cell's first texel row and
    // first column belong.
    out.quad_uv = corner * 0.5 + vec2<f32>(0.5, 0.5);
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
