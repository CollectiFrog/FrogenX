// Mélange de deux clips — §3bis.2 de docs/ARCHITECTURE.md.
//
// Couleurs en linéaire prémultiplié (§21) : le mélange est une simple
// interpolation. `coverage` est la copie exacte de `mix_coverage`, qui sert de
// référence aux tests : toute modification ici doit y être reportée.

struct Params {
    amount: f32,
    // Indices de `MixKind`.
    kind: u32,
    size: vec2<f32>,
};

const EDGE: f32 = 0.02;

@group(0) @binding(0) var incoming: texture_2d<f32>;
@group(0) @binding(1) var outgoing: texture_2d<f32>;
@group(0) @binding(2) var<uniform> params: Params;

// Triangle plein écran, sans buffer de sommets.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

fn coverage(uv: vec2<f32>) -> f32 {
    var position = 0.0;
    switch params.kind {
        case 0u: { return params.amount; }
        case 1u: { position = 1.0 - uv.x; } // vers la gauche
        case 2u: { position = uv.x; }       // vers la droite
        case 3u: { position = 1.0 - uv.y; } // vers le haut
        default: { position = uv.y; }       // vers le bas
    }
    let front = params.amount * (1.0 + 2.0 * EDGE) - EDGE;
    return 1.0 - clamp((position - front + EDGE) / (2.0 * EDGE), 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(pos.xy);
    let a = textureLoad(outgoing, texel, 0);
    let b = textureLoad(incoming, texel, 0);
    return mix(a, b, coverage(pos.xy / params.size));
}
