// Feedback — §6.1 de docs/ARCHITECTURE.md.
//
// Relit l'image finale de la frame d'avant, déplacée et affaiblie. Le calcul de la
// position source est la copie exacte de `FeedbackStep::source_of`, qui sert de
// référence aux tests : toute modification ici doit y être reportée.

struct Params {
    // Les deux lignes de la rotation divisée par l'échelle.
    row0: vec2<f32>,
    row1: vec2<f32>,
    offset: vec2<f32>,
    // Largeur sur hauteur : l'espace du §17 a [-1, 1] sur le petit côté.
    extent: vec2<f32>,
    size: vec2<f32>,
    keep: f32,
    _pad: f32,
};

@group(0) @binding(0) var history: texture_2d<f32>;
@group(0) @binding(1) var linear: sampler;
@group(0) @binding(2) var<uniform> params: Params;

// Triangle plein écran, sans buffer de sommets.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy / params.size;
    // L'espace du §17 : le haut est positif.
    let p = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0) * params.extent;
    let d = p - params.offset;
    let source = vec2<f32>(dot(params.row0, d), dot(params.row1, d)) / params.extent;
    let lu = vec2<f32>(source.x * 0.5 + 0.5, 0.5 - source.y * 0.5);
    // Hors de l'image d'avant, il n'y a rien : pas de bords étirés.
    if (lu.x < 0.0 || lu.x > 1.0 || lu.y < 0.0 || lu.y > 1.0) {
        return vec4<f32>(0.0);
    }
    return textureSampleLevel(history, linear, lu, 0.0) * params.keep;
}
