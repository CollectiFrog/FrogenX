// Le flou du halo CRT — §6.2 de docs/ARCHITECTURE.md.
//
// Séparable : une passe horizontale, puis une verticale, en demi-résolution.

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

struct Blur {
    direction: vec2<f32>,
    // La taille de la cible : la source peut être deux fois plus grande.
    size: vec2<f32>,
};

@group(0) @binding(0) var blur_source: texture_2d<f32>;
@group(0) @binding(1) var blur_sampler: sampler;
@group(0) @binding(2) var<uniform> blur: Blur;

// Un flou gaussien séparable, sur 9 points, pour le halo.
@fragment
fn fs_blur(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let taille = blur.size;
    let uv = pos.xy / taille;
    var poids = array<f32, 5>(0.227027, 0.1945946, 0.1216216, 0.054054, 0.016216);
    var somme = textureSampleLevel(blur_source, blur_sampler, uv, 0.0) * poids[0];
    for (var i = 1; i < 5; i = i + 1) {
        let d = blur.direction * f32(i) * 2.0 / taille;
        somme = somme + textureSampleLevel(blur_source, blur_sampler, uv + d, 0.0) * poids[i];
        somme = somme + textureSampleLevel(blur_source, blur_sampler, uv - d, 0.0) * poids[i];
    }
    return somme;
}
