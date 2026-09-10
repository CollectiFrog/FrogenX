//! Bruit cohérent 1D — value noise interpolé, à graine explicite.
//!
//! **Pourquoi une implémentation maison plutôt que la crate `noise` :** le §20.3
//! exige un déterminisme reproductible, graine stockée dans le patch. Garantir
//! cela suppose de connaître exactement l'algorithme et son état ; c'est plus
//! sûr que d'auditer le déterminisme d'une dépendance à travers ses versions.
//!
//! Le bruit *cohérent* diffère du bruit blanc : deux instants proches donnent
//! des valeurs proches. C'est ce qui produit un mouvement organique plutôt
//! qu'un grésillement.

use frogenx_core::{Ctx, NodeState, Param, Signal};

/// Hachage entier déterministe — même valeur sur toutes les plateformes.
///
/// Constantes de Wang/Jenkins. Rien de cryptographique : on veut de la
/// décorrélation bon marché et strictement reproductible.
fn hash_u32(mut x: u32) -> u32 {
    x = (x ^ 61) ^ (x >> 16);
    x = x.wrapping_add(x << 3);
    x ^= x >> 4;
    x = x.wrapping_mul(0x27d4_eb2d);
    x ^= x >> 15;
    x
}

/// Valeur pseudo-aléatoire dans `[-1, 1]` pour un entier et une graine.
fn value_at(i: i32, seed: u32) -> f32 {
    let h = hash_u32((i as u32).wrapping_mul(0x9e37_79b9) ^ hash_u32(seed));
    // 24 bits de mantisse : suffisant, et évite les surprises de conversion.
    (h >> 8) as f32 / 8_388_608.0 - 1.0
}

/// Interpolation lissée (smoothstep) — dérivée nulle aux extrémités.
///
/// Une interpolation linéaire produirait des cassures visibles à chaque
/// entier, précisément ce qu'on cherche à éviter pour l'organique.
fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Bruit cohérent 1D évalué en un point quelconque.
///
/// Fonction pure : même `(x, seed)` → même valeur, toujours. C'est ce qui rend
/// le rendu déterministe (§20.3) et les tests à valeur exacte possibles.
pub fn value_noise_1d(x: f32, seed: u32) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    let i = x.floor();
    let f = x - i;
    let i = i as i32;
    let a = value_at(i, seed);
    let b = value_at(i.wrapping_add(1), seed);
    a + (b - a) * smoothstep(f)
}

/// Bruit fractal — plusieurs octaves de bruit superposées.
///
/// Chaque octave double la fréquence et réduit l'amplitude de moitié
/// (persistance). C'est ce qui donne le détail à plusieurs échelles
/// caractéristique du naturel.
pub fn fbm_1d(x: f32, seed: u32, octaves: u32, persistence: f32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut freq = 1.0;
    let mut norm = 0.0;
    // Borne dure : au-delà, les octaves n'apportent plus rien de visible et
    // le coût grimpe linéairement.
    for o in 0..octaves.min(16) {
        sum += value_noise_1d(x * freq, seed.wrapping_add(o * 1013)) * amp;
        norm += amp;
        amp *= persistence;
        freq *= 2.0;
    }
    if norm > 0.0 {
        sum / norm
    } else {
        0.0
    }
}

/// Signal de bruit cohérent, avançant dans le temps.
///
/// Le temps interne avance à `rate` unités par seconde. Il est accumulé plutôt
/// que dérivé de `t`, pour la même raison que la phase d'un oscillateur : une
/// modulation de `rate` ne doit pas provoquer de saut.
pub struct Noise {
    /// Vitesse de défilement, en unités de bruit par seconde. Modulable.
    pub rate: Param,
    pub amp: Param,
    /// Graine — stockée dans le patch, jamais tirée au hasard (§20.3).
    pub seed: u32,
    pub octaves: u32,
    pub persistence: f32,
}

impl Noise {
    pub fn new(rate: f32, seed: u32) -> Self {
        Self {
            rate: Param::fixed(rate),
            amp: Param::fixed(1.0),
            seed,
            octaves: 1,
            persistence: 0.5,
        }
    }

    /// Bruit fractal à plusieurs octaves.
    pub fn fractal(mut self, octaves: u32, persistence: f32) -> Self {
        self.octaves = octaves;
        self.persistence = persistence;
        self
    }
}

impl Signal for Noise {
    // slots[0] = position accumulée dans l'espace du bruit
    fn eval(&self, _t: f64, ctx: &Ctx, state: &mut NodeState) -> f32 {
        let rate = self.rate.eval(ctx);
        let advance = if rate.is_finite() {
            rate * ctx.dt as f32
        } else {
            0.0
        };
        let x = state.get(0) + advance;
        // Un x non fini gèlerait le bruit définitivement.
        let x = if x.is_finite() { x } else { 0.0 };
        state.set(0, x);

        let v = if self.octaves <= 1 {
            value_noise_1d(x, self.seed)
        } else {
            fbm_1d(x, self.seed, self.octaves, self.persistence)
        };
        v * self.amp.eval(ctx)
    }

    fn state_size(&self) -> usize {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frogenx_core::Graph;

    #[test]
    fn le_bruit_est_deterministe() {
        // §20.3 : deux appels identiques donnent exactement la même valeur.
        for i in 0..100 {
            let x = i as f32 * 0.37;
            assert_eq!(value_noise_1d(x, 42), value_noise_1d(x, 42));
        }
    }

    #[test]
    fn deux_graines_donnent_des_bruits_differents() {
        let a: Vec<f32> = (0..50).map(|i| value_noise_1d(i as f32 * 0.5, 1)).collect();
        let b: Vec<f32> = (0..50).map(|i| value_noise_1d(i as f32 * 0.5, 2)).collect();
        assert_ne!(a, b, "des graines différentes doivent décorréler");
    }

    #[test]
    fn le_bruit_reste_borne() {
        for i in 0..10_000 {
            let x = i as f32 * 0.013;
            let v = value_noise_1d(x, 7);
            assert!((-1.0..=1.0).contains(&v), "x={x} : got {v}");
        }
    }

    #[test]
    fn le_bruit_est_coherent_pas_blanc() {
        // La propriété qui distingue le bruit cohérent : deux points proches
        // donnent des valeurs proches. C'est ce qui rend le mouvement organique.
        let mut max_saut = 0.0f32;
        for i in 0..1000 {
            let x = i as f32 * 0.01;
            let saut = (value_noise_1d(x + 0.01, 3) - value_noise_1d(x, 3)).abs();
            max_saut = max_saut.max(saut);
        }
        assert!(
            max_saut < 0.2,
            "saut max sur 0.01 : {max_saut} — bruit non cohérent"
        );
    }

    #[test]
    fn le_bruit_passe_par_ses_valeurs_aux_entiers() {
        // Aux entiers, l'interpolation rend exactement la valeur du nœud.
        for i in -5..5 {
            let attendu = value_at(i, 11);
            let obtenu = value_noise_1d(i as f32, 11);
            assert!(
                (obtenu - attendu).abs() < 1e-6,
                "i={i} : {obtenu} vs {attendu}"
            );
        }
    }

    #[test]
    fn fbm_reste_borne_et_deterministe() {
        for i in 0..1000 {
            let x = i as f32 * 0.05;
            let v = fbm_1d(x, 5, 4, 0.5);
            assert!((-1.0..=1.0).contains(&v), "x={x} : got {v}");
            assert_eq!(v, fbm_1d(x, 5, 4, 0.5));
        }
    }

    #[test]
    fn fbm_a_une_octave_egale_le_bruit_simple() {
        for i in 0..50 {
            let x = i as f32 * 0.3;
            let a = fbm_1d(x, 9, 1, 0.5);
            let b = value_noise_1d(x, 9);
            assert!((a - b).abs() < 1e-6, "x={x} : {a} vs {b}");
        }
    }

    #[test]
    fn le_signal_de_bruit_avance_et_reste_fini() {
        let mut g = Graph::new();
        let n = g.add(Box::new(Noise::new(1.0, 123)));
        let mut valeurs = Vec::new();
        for i in 0..300 {
            g.eval_frame(i as f64 / 60.0, 1.0 / 60.0);
            let v = g.value(n);
            assert!(v.is_finite(), "frame {i}");
            assert!((-1.0..=1.0).contains(&v), "frame {i} : got {v}");
            valeurs.push(v);
        }
        // Le bruit doit effectivement évoluer, pas rester figé.
        let premier = valeurs[0];
        assert!(
            valeurs.iter().any(|v| (v - premier).abs() > 0.05),
            "le bruit ne bouge pas"
        );
        assert!(g.diagnostics().is_clean());
    }

    #[test]
    fn rate_nul_fige_le_bruit() {
        let mut g = Graph::new();
        let n = g.add(Box::new(Noise::new(0.0, 55)));
        g.eval_frame(0.0, 1.0 / 60.0);
        let premier = g.value(n);
        for i in 1..20 {
            g.eval_frame(i as f64 / 60.0, 1.0 / 60.0);
            assert_eq!(g.value(n), premier, "frame {i}");
        }
    }

    #[test]
    fn valeurs_non_finies_neutralisees() {
        assert_eq!(value_noise_1d(f32::NAN, 1), 0.0);
        assert_eq!(value_noise_1d(f32::INFINITY, 1), 0.0);

        let mut g = Graph::new();
        let n = g.add(Box::new(Noise::new(f32::NAN, 1)));
        g.eval_frame(0.0, 1.0 / 60.0);
        assert!(g.value(n).is_finite());
    }
}
