struct SpriteUniforms {
    to_local: mat2x2<f32>,
    translation: vec2<f32>,
    size: vec2<f32>, // local extent of the quad in local units
    tint: vec4<f32>, // multiplied with every sampled pixel
    alpha: f32, // multiplied with the texture's own alpha
    lit: f32, // 1.0 when the sprite is lit by the frame's light field
    uv_rect: vec4<f32>, // [min_x, min_y, max_x, max_y] sub-rect of the texture
    glow: vec4<f32>, // self-emission: rgb, scaled by a, added to the light mix
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

@group(0) @binding(4)
var<storage, read>
occluders: OccluderField;

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
    // The final alpha: the texture's own, scaled by the sprite's alpha and
    // the tint's. Computed here, before the lights, for one exact reason:
    // a fragment that would blend (rgb, 0) is the blend equation's
    // identity, so discarding it changes no pixel of the attachment — and
    // the whole per-fragment light loop (every light's falloff, every
    // occlusion walk) is skipped along with it. PNG margins are exactly
    // zero, so every transparent texel of a padded sprite takes this exit;
    // an epsilon would be an image change dressed as an optimization, so
    // the test is exact. The sample deliberately sits above the discard:
    // WGSL derives mip levels from the surviving threads of each 2x2 quad,
    // and discarding before the fetch would poison the survivors'
    // derivatives. Note what this does NOT recover: the texel fetch and
    // the rasterized slot are spent; only the arithmetic after them is.
    //
    // For tile-based targets (mobile tilers under WebGL, Apple's own
    // tiles): some drivers keep whole-draw fast paths that a program
    // containing `discard` can opt out of, fire or no fire. The classic
    // companion penalty — early-depth demotion — is a non-event here
    // because frost renders with no depth attachment at all, and desktop
    // tilers and immediate renderers run this alpha-masked pattern as
    // bread and butter; but if a tiler ever measures slower with the
    // discard than without, the answer is to revert this block, not to
    // rework it. Phone and tablet targets are outside the current
    // audience; a field report from one is the reason this note exists.
    let alpha = t.a * u.alpha * u.tint.a;
    if (alpha == 0.0) {
        discard;
    }
    // The tinted texture color, multiplied by the light mix when the
    // sprite is lit; the lights never touch the alpha. The sprite's own
    // glow joins the mix as a floor of self-light: rgb scaled by its
    // alpha, added even where the light field is black — a glowing sprite
    // is visible in the dark, and lights still add on top.
    var rgb = t.rgb * u.tint.rgb;
    if (u.lit > 0.5) {
        rgb *= light_mix(frag_coord.xy) + u.glow.rgb * u.glow.a;
    }
    // Emit the color, scaled by the sprite's alpha and the tint's own
    // alpha, and composited over the existing attachment via alpha
    // blending.
    return vec4<f32>(rgb, alpha);
}
