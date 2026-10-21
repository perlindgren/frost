// The dig's dissolve: the soil tile the player just walked into,
// eaten from the edges inward over the length of one hop and gone
// by the time the step lands. A game-authored frost program —
// compiled behind the engine's lighting prelude, bound exactly like
// `sprite.wgsl`, and drawn in place of the sprite pipeline wherever
// its host attaches it (`Shape::with_program`); the smallest host is
// `frost_shaders/examples/dissolve.rs`.
//
// The uniforms are the sprite's, 112 bytes, and the engine packed
// them from a scene node; what they mean here is the game's own
// saying: `u.glow` carries the dirt tint (modulate rides white, so
// it arrives whole), and `u.emission` carries the effect:
//     r = progress, 0 at the issued move, 1 as the step lands;
//     g = a per-cell seed, so no two holes eat alike;
//     b = edge softness of the eat band;
//     a = noise cycles across the node — the grain, at the host's
//         scale (antifrost's dig cells ride 5.0).

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

/// Modulus that keeps the simplex permutation honest at the
/// magnitudes the skew touches — floor arithmetic, so it behaves
/// identically on every GPU.
fn mod289_2(x: vec2<f32>) -> vec2<f32> {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}

fn mod289_3(x: vec3<f32>) -> vec3<f32> {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}

fn permute3(x: vec3<f32>) -> vec3<f32> {
    return mod289_3(((x * 34.0) + 1.0) * x);
}

/// Simplex noise in [-1, 1] (Ashima's 2D snoise, transcribed to
/// WGSL one swizzle at a time). Chosen over value noise because
/// value noise blends lattice values with a smoothstep, whose zero
/// derivative along cell edges makes the field sit on plateaus with
/// right-angle borders — a coarse-grained dissolve dissolves into a
/// quilt, every seam axis-aligned. Simplex samples a triangular
/// lattice of gradients instead: no plateaus, no axes, contours
/// wander like rivers however far the host zooms the field.
fn simplex(p: vec2<f32>) -> f32 {
    let skew = 0.366025403784439; // (sqrt(3) - 1) / 2
    let offset = 0.211324865405187; // (3 - sqrt(3)) / 6
    let i = floor(p + vec2<f32>(dot(p, vec2<f32>(skew, skew))));
    let x0 = p - i + vec2<f32>(dot(i, vec2<f32>(offset, offset)));
    let i1 = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), x0.x > x0.y);
    let x1 = x0 - i1 + vec2<f32>(offset, offset);
    let x2 = x0 + vec2<f32>(offset + offset - 1.0, offset + offset - 1.0);
    let ii = mod289_2(i);
    let p0 = permute3(
        permute3(vec3<f32>(0.0, i1.y, 1.0) + vec3<f32>(ii.y, ii.y, ii.y))
            + vec3<f32>(0.0, i1.x, 1.0)
            + vec3<f32>(ii.x, ii.x, ii.x),
    );
    var m = max(
        vec3<f32>(0.5)
            - vec3<f32>(dot(x0, x0), dot(x1, x1), dot(x2, x2)),
        vec3<f32>(0.0),
    );
    m = m * m;
    m = m * m;
    let xg = 2.0 * fract(p0 * 0.024390243902439) - 1.0;
    let h = abs(xg) - 0.5;
    let a0 = xg - floor(xg + 0.5);
    m = m * (vec3<f32>(1.79284291400159) - vec3<f32>(0.85373472095314)
        * (a0 * a0 + h * h));
    let g = vec3<f32>(
        a0.x * x0.x + h.x * x0.y,
        a0.y * x1.x + h.y * x1.y,
        a0.z * x2.x + h.z * x2.y,
    );
    return 130.0 * dot(m, g);
}

/// The dirt's field: three octaves of simplex — a broad bite with a
/// medium and a fine one riding it — lifted and scaled into
/// [0, 1]. Simplex's own octave halving is gentler than value
/// noise's, and nothing in it knows which way the axes are.
fn clumps(p: vec2<f32>) -> f32 {
    let a = 0.5 + 0.5 * simplex(p);
    let b = 0.5 + 0.5 * simplex(2.31 * p + vec2<f32>(7.3, 3.7));
    let c = 0.5 + 0.5 * simplex(5.7 * p + vec2<f32>(1.9, 8.1));
    return a * 0.62 + b * 0.26 + c * 0.12;
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
    // The lattice frequency — cycles of noise across the quad — is the
    // host's to choose, carried in `emission.a`: one node may be a
    // game tile or a whole window, and the bite stays the same physical
    // grain when the host scales the cycles with the node. Clamped so
    // a defaulted alpha (the sprite packs it like any colour) still
    // eats in real clumps rather than one flat slab.
    // The lattice's grain is the host's to choose, cycles of noise
    // across the quad: antifrost rides 5.0 through a tile's hundred
    // pixels — a clump the width of a thumb's nail. Hosts at another
    // size class carry the contract by keeping that physical clump
    // size, whatever the grain figure comes to; the field is the
    // same earth everywhere, and never a visible lattice of cells.
    let grain = clamp(u.emission.a, 2.0, 96.0);
    let q = uv * grain + vec2<f32>(u.emission.g * 57.0, u.emission.g * 91.0);
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
