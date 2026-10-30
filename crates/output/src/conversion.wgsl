// Passe de conversion de sortie — §8.0, §8.3, §21 de docs/ARCHITECTURE.md.
//
// Linéaire prémultiplié (Rgba16Float) → cible 8 bits. En alpha droit, même
// ordre que `linear_premul_to_srgb8` (crate render), sa référence CPU dans les
// tests : on dé-prémultiplie EN LINÉAIRE, puis on encode. Toute modification
// ici doit y être reportée.

// Faux si la cible est `*Srgb` : le matériel encode à l'écriture, et encoder
// aussi ici délaverait l'image (§21).
override ENCODE_SRGB: bool = true;
// Faux pour une cible opaque : le RGB prémultiplié se lit alors sur du noir.
override STRAIGHT_ALPHA: bool = true;

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Triangle plein écran, sans buffer de sommets.
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> Vertex {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return Vertex(vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0), vec2<f32>(x, 1.0 - y));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let bas = c * 12.92;
    let haut = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(haut, bas, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(v: Vertex) -> @location(0) vec4<f32> {
    let px = textureSampleLevel(source, source_sampler, v.uv, 0.0);
    var a = 1.0;
    var rgb = px.rgb;
    if (STRAIGHT_ALPHA) {
        a = saturate(px.a);
        // Un pixel transparent : dé-prémultiplier diviserait par zéro.
        if (a <= 0.0) {
            return vec4<f32>(0.0);
        }
        rgb = px.rgb / a;
    }
    // Un mode `Add` peut dépasser 1 : saturé ici, chez le consommateur (§18.4).
    rgb = saturate(rgb);
    if (ENCODE_SRGB) {
        rgb = linear_to_srgb(rgb);
    }
    return vec4<f32>(rgb, a);
}
