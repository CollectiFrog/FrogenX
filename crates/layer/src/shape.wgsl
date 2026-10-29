// ShapeLayer — §22 de docs/ARCHITECTURE.md.
//
// Volontairement trivial : les positions arrivent déjà en NDC (correction de
// proportion faite en `prepare`, §17) et les couleurs déjà en linéaire
// prémultiplié (§21). Tout ce qui demande une décision est fait côté CPU, où
// les tests l'atteignent sans GPU.

struct VsIn {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    o.position = vec4<f32>(v.position, 0.0, 1.0);
    o.color = v.color;
    return o;
}

@fragment
fn fs_main(i: VsOut) -> @location(0) vec4<f32> {
    return i.color;
}
