// CRT — §6.2 de docs/ARCHITECTURE.md.
//
// Chaque calcul est la copie de sa référence en Rust (`shape/src/crt.rs`) :
// `barrel`, `scanline`, `mask_weights`, `vignette`. Toute modification ici doit y
// être reportée. L'image reste en linéaire prémultiplié (§21).

struct Params {
    size: vec2<f32>,
    time: f32,
    lines: f32,
    scanlines: f32,
    mask: f32,
    curvature: f32,
    vignette: f32,
    halo: f32,
    aberration: f32,
    noise: f32,
    flicker: f32,
    band: f32,
    keep: f32,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var glow: texture_2d<f32>;
@group(0) @binding(2) var previous: texture_2d<f32>;
@group(0) @binding(3) var linear: sampler;
@group(0) @binding(4) var<uniform> params: Params;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = pos.xy / params.size;

    // L'écran bombé : les bords vont chercher plus loin, et hors de l'image il n'y a rien.
    let c = uv * 2.0 - 1.0;
    let f = 1.0 + params.curvature * 0.25 * dot(c, c);
    let lu = c * f * 0.5 + 0.5;
    var couleur = vec4<f32>(0.0);
    if (lu.x >= 0.0 && lu.x <= 1.0 && lu.y >= 0.0 && lu.y <= 1.0) {
        // Rouge et bleu s'écartent vers les bords ; l'alpha garde le plus large des trois.
        let ecart = (lu - 0.5) * params.aberration * 0.02;
        let r = textureSampleLevel(source, linear, lu + ecart, 0.0);
        let g = textureSampleLevel(source, linear, lu, 0.0);
        let b = textureSampleLevel(source, linear, lu - ecart, 0.0);
        couleur = vec4<f32>(r.r, g.g, b.b, max(g.a, max(r.a, b.a)));

        // Le halo éclaire : l'alpha remonte au niveau de la couleur.
        if (params.halo > 0.0) {
            let lueur = textureSampleLevel(glow, linear, lu, 0.0).rgb * params.halo;
            couleur = vec4<f32>(couleur.rgb + lueur, couleur.a);
            couleur.a = clamp(max(couleur.a, max(couleur.r, max(couleur.g, couleur.b))), 0.0, 1.0);
        }

        // Lignes, masque et vignette assombrissent la couleur, jamais l'alpha.
        let creux = 0.5 - 0.5 * cos(6.2831853 * lu.y * params.lines);
        var rgb = couleur.rgb * (1.0 - params.scanlines * creux);
        let colonne = u32(pos.x) % 3u;
        var triade = vec3<f32>(0.25, 0.25, 1.0);
        if (colonne == 0u) {
            triade = vec3<f32>(1.0, 0.25, 0.25);
        } else if (colonne == 1u) {
            triade = vec3<f32>(0.25, 1.0, 0.25);
        }
        rgb = rgb * (vec3<f32>(1.0) + params.mask * (triade - vec3<f32>(1.0)));
        rgb = rgb * max(1.0 - params.vignette * 0.5 * dot(c, c), 0.0);

        // Le grain et le scintillement tirent leur hasard de `t` : un rendu offline reste identique.
        let grain = hash(pos.xy + vec2<f32>(params.time * 61.0, params.time * 17.0)) - 0.5;
        rgb = rgb + vec3<f32>(grain * params.noise * couleur.a);
        let image = floor(params.time * 60.0);
        rgb = rgb * (1.0 - params.flicker * hash(vec2<f32>(image, 3.7)));
        let y_bande = fract(uv.y - params.time * 0.25) - 0.5;
        rgb = rgb * (1.0 + params.band * 0.3 * exp(-y_bande * y_bande * 150.0));

        couleur = vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(couleur.a)), couleur.a);
    }

    // La rémanence : ce qui était allumé s'éteint lentement.
    let avant = textureSampleLevel(previous, linear, uv, 0.0) * params.keep;
    return max(couleur, avant);
}
