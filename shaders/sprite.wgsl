struct SpriteUniforms {
    to_local: mat2x2<f32>,
    translation: vec2<f32>,
    size: vec2<f32>, // local extent of the quad in local units
    tint: vec3<f32>, // multiplied with every sampled pixel
    alpha: f32, // multiplied with the texture's own alpha
    uv_rect: vec4<f32>, // [min_x, min_y, max_x, max_y] sub-rect of the texture
};

@group(0) @binding(0)
var<uniform> u: SpriteUniforms;

@group(0) @binding(1)
var tex: texture_2d<f32>;

@group(0) @binding(2)
var samp: sampler;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    // Full-screen triangle in NDC; the scissor is set on the CPU to the
    // sprite's bounding box, so fragments outside it are discarded and the
    // fragment shader only runs over the pixels the sprite can write.
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[i], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    // Convert the fragment's pixel coordinate into the sprite's local space
    // (the inverse of its world transform), where the sprite is centered at
    // the origin and extends by `size / 2` along each axis. Local +y is "up"
    // (the top row of the texture), so the v coordinate is flipped when
    // mapping to [0, 1] texture space.
    let p = u.to_local * frag_coord.xy + u.translation;
    let uv = vec2<f32>(p.x / u.size.x, -p.y / u.size.y) + vec2<f32>(0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return vec4<f32>(0.0);
    }
    // Sample the quad's [0, 1] uv remapped into the texture sub-rectangle it
    // covers; for a whole-texture sprite the remap is the identity.
    let t = textureSample(tex, samp, mix(u.uv_rect.xy, u.uv_rect.zw, uv));
    // Emit the tinted texture color, scaled by the sprite's alpha, and
    // composited over the existing attachment via alpha blending.
    return vec4<f32>(t.rgb * u.tint, t.a * u.alpha);
}
