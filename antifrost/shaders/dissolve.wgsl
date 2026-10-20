// The dig's dissolve: the soil tile the player just walked into,
// eaten from the edges inward over the length of one hop and gone
// by the time the step lands. A game-authored frost program —
// compiled behind the engine's lighting prelude, bound exactly like
// `sprite.wgsl`, and drawn in place of the sprite pipeline wherever
// antifrost attaches it (`Shape::with_program`).
//
// The uniforms are the sprite's, 112 bytes, and the engine packed
// them from a scene node; what they mean here is the game's own
// saying: `u.glow` carries the dirt tint (modulate rides white, so
// it arrives whole), and `u.emission` carries the effect:
//     r = progress, 0 at the issued move, 1 as the step lands;
//     g = a per-cell seed, so no two holes eat alike;
//     b = edge softness of the eat band.

struct SpriteUniforms {
    to_local: mat2x2<f32>,
    translation: vec2<f32>,
    size: vec2<f32>, // local extent of the quad in local units
    tint: vec4<f32>, // white here — the program is its own tint
    alpha: f32, // multiplied with the texture's own alpha
    lit: f32, // 1.0 when the sprite is lit by the frame's light field
    lit_layers: f32, // receiver's light-layer mask, bits in an f32
    uv_rect: vec4<f32>, // the whole texture, as always for a sprite
    emission: vec4<f32>, // the effect: progress, seed, softness, —
    glow: vec4<f32>, // the dirt tint, per cell, noise and all
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
    // The engine's full-screen triangle: the scissor already stands at
    // this tile's box, so the fragment stage only ever runs over the
    // soil that could change.
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[i], 0.0, 1.0);
}

/// Hash of a point, the same one every grain of antifrost's noise
/// could have used: no tables, no texture, just bit-scatter.
fn hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(127.1, 311.7));
    return fract(sin(dot(q, vec2<f32>(12.9898, 78.233)) + q.x * 43.7) * 43758.5453);
}

/// Two octaves of value noise: coarse grain with a finer bite, the
/// dirt's own scale of clumps. The band wants noise in [0, 1] that
/// varies smoothly across the tile; two octaves is a clump with
/// texture in it, three would be mush at a tile's hundred-odd pixels.
fn clumps(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let a = mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), w.x);
    let b = mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), w.x);
    let fine = mix(hash(i * 2.7), hash(i * 2.7 + vec2<f32>(1.3, 2.1)), w.x) * 0.25;
    return mix(a, b, w.y) * 0.75 + fine;
}

struct Shaded {
    color: vec4<f32>,
    emissive: vec4<f32>,
};

struct SceneOut {
    @location(0) color: vec4<f32>,
    @location(1) emissive: vec4<f32>,
};

fn shade(frag_coord: vec4<f32>) -> Shaded {
    // Into the tile's local space, then into uv: the engine's own walk,
    // because the arena was written by the engine's own hand.
    let p = u.to_local * frag_coord.xy + u.translation;
    let uv = vec2<f32>(p.x / u.size.x, -p.y / u.size.y) + vec2<f32>(0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) {
        return Shaded(vec4<f32>(0.0), vec4<f32>(0.0));
    }
    let t = textureSample(tex, samp, mix(u.uv_rect.xy, u.uv_rect.zw, uv));

    // The eat band. Progress drives a threshold through the noise
    // field, and the threshold runs its band from before the field
    // starts to after it ends — so progress 0 means the whole tile
    // stands (the band has not bitten yet) and progress 1 means none
    // of it does (the band has swept past every grain). The seed
    // shifts the field, not the tile: one hill, a thousand holes, no
    // two patterns.
    let prog = clamp(u.emission.r, 0.0, 1.0);
    let soft = max(u.emission.b, 0.01);
    let q = uv * 5.0 + vec2<f32>(u.emission.g * 57.0, u.emission.g * 91.0);
    let n = clumps(q);
    let cut = -soft + prog * (1.0 + 2.0 * soft);
    let mask = smoothstep(cut - soft, cut + soft, n);
    let alpha = t.a * u.alpha * mask;
    if (alpha == 0.0) {
        discard;
    }

    // The rim: where the band is mid-bite, soil glows warm — the edge
    // that is disappearing is the edge that is burning through. Peaked
    // at mask half, silent on either side.
    let rim = mask * (1.0 - mask) * 4.0;

    var rgb = t.rgb * u.glow.rgb;
    if (u.lit > 0.5) {
        rgb *= light_mix(frag_coord.xy, bitcast<u32>(u.lit_layers));
    }
    // The rim rides the color as a hot brown, whether lights or none —
    // disappearing earth is its own light source.
    rgb += vec3<f32>(0.9, 0.42, 0.14) * rim * 0.55;
    // And out to the bloom: the same rim, past 1.0, so the eat edge
    // throws a halo onto the dark around the new hole.
    let glow = vec3<f32>(1.0, 0.5, 0.16) * rim * 1.8 + rgb * rim * 0.3;
    return Shaded(vec4<f32>(rgb, alpha), vec4<f32>(glow, alpha));
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4<f32>) -> @location(0) vec4<f32> {
    return shade(frag_coord).color;
}

@fragment
fn fs_emissive(@builtin(position) frag_coord: vec4<f32>) -> SceneOut {
    let s = shade(frag_coord);
    return SceneOut(s.color, s.emissive);
}
