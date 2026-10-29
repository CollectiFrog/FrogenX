// Compositeur — §22 de docs/ARCHITECTURE.md.
//
// Une passe par couche, en ping-pong : on lit l'accumulé et la couche, on
// écrit dans l'autre texture d'accumulation. Le mélange se fait ici plutôt
// qu'en BlendState matériel parce que `Difference` doit LIRE la destination.
//
// Couleurs en linéaire prémultiplié (§21). Les formules sont la copie exacte
// de `blend.rs`, qui sert de référence aux tests de pixels : toute
// modification ici doit y être reportée, et inversement.

struct Params {
    opacity: f32,
    // Indices de `frogenx_layer::BlendMode`.
    mode: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var accum: texture_2d<f32>;
@group(0) @binding(1) var layer: texture_2d<f32>;
@group(0) @binding(2) var<uniform> params: Params;

// Triangle plein écran, sans buffer de sommets.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let px = vec2<i32>(p.xy);
    let d = textureLoad(accum, px, 0);
    let s = textureLoad(layer, px, 0) * params.opacity;

    var rgb: vec3<f32>;
    switch params.mode {
        // Add
        case 1u: {
            rgb = s.rgb + d.rgb;
        }
        // Multiply
        case 2u: {
            rgb = s.rgb * d.rgb + s.rgb * (1.0 - d.a) + d.rgb * (1.0 - s.a);
        }
        // Screen
        case 3u: {
            rgb = s.rgb + d.rgb - s.rgb * d.rgb;
        }
        // Difference
        case 4u: {
            rgb = s.rgb + d.rgb - 2.0 * min(s.rgb * d.a, d.rgb * s.a);
        }
        // Normal, et tout indice inconnu : jamais d'image indéfinie.
        default: {
            rgb = s.rgb + d.rgb * (1.0 - s.a);
        }
    }
    return vec4<f32>(rgb, s.a + d.a * (1.0 - s.a));
}
