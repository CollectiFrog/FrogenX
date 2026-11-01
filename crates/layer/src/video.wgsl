// Vidéo — §9.3 de docs/ARCHITECTURE.md.
//
// Lit trois plans YUV et rend du linéaire prémultiplié, comme toute couche (§21).
// La conversion est la copie exacte de `yuv_to_linear`, qui sert de référence aux
// tests : toute modification ici doit y être reportée.

struct Params {
    // Les deux lignes de la rotation divisée par l'échelle.
    row0: vec2<f32>,
    row1: vec2<f32>,
    offset: vec2<f32>,
    // La moitié du rendu, puis de l'image, dans l'espace du §17.
    extent: vec2<f32>,
    image: vec2<f32>,
    size: vec2<f32>,
};

@group(0) @binding(0) var plane_y: texture_2d<f32>;
@group(0) @binding(1) var plane_u: texture_2d<f32>;
@group(0) @binding(2) var plane_v: texture_2d<f32>;
@group(0) @binding(3) var linear: sampler;
@group(0) @binding(4) var<uniform> params: Params;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

fn to_linear(c: f32) -> f32 {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy / params.size;
    // L'espace du §17 : le haut est positif.
    let p = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0) * params.extent;
    let d = p - params.offset;
    let q = vec2<f32>(dot(params.row0, d), dot(params.row1, d)) / params.image;
    let lu = vec2<f32>(q.x * 0.5 + 0.5, 0.5 - q.y * 0.5);
    // Hors de l'image, la couche est transparente : celles du dessous se voient.
    if (lu.x < 0.0 || lu.x > 1.0 || lu.y < 0.0 || lu.y > 1.0) {
        return vec4<f32>(0.0);
    }
    let y = (textureSampleLevel(plane_y, linear, lu, 0.0).r - 16.0 / 255.0) * 255.0 / 219.0;
    let u = (textureSampleLevel(plane_u, linear, lu, 0.0).r - 128.0 / 255.0) * 255.0 / 224.0;
    let v = (textureSampleLevel(plane_v, linear, lu, 0.0).r - 128.0 / 255.0) * 255.0 / 224.0;
    let r = clamp(y + 1.5748 * v, 0.0, 1.0);
    let g = clamp(y - 0.1873 * u - 0.4681 * v, 0.0, 1.0);
    let b = clamp(y + 1.8556 * u, 0.0, 1.0);
    return vec4<f32>(to_linear(r), to_linear(g), to_linear(b), 1.0);
}
