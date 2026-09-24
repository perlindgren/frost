struct SpriteUniforms {
    to_local: mat2x2<f32>,
    translation: vec2<f32>,
    size: vec2<f32>, // local extent of the quad in local units
    tint: vec4<f32>, // multiplied with every sampled pixel
    alpha: f32, // multiplied with the texture's own alpha
    lit: f32, // 1.0 when the sprite is lit by the frame's light field
    uv_rect: vec4<f32>, // [min_x, min_y, max_x, max_y] sub-rect of the texture
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
var<uniform> u: SpriteUniforms;

@group(0) @binding(1)
var tex: texture_2d<f32>;

@group(0) @binding(2)
var samp: sampler;

@group(0) @binding(3)
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
    // The tinted texture color, multiplied by the light mix when the
    // sprite is lit; the lights never touch the alpha.
    var rgb = t.rgb * u.tint.rgb;
    if (u.lit > 0.5) {
        rgb *= light_mix(frag_coord.xy);
    }
    // Emit the color, scaled by the sprite's alpha and the tint's own
    // alpha, and composited over the existing attachment via alpha
    // blending.
    return vec4<f32>(rgb, t.a * u.alpha * u.tint.a);
}
