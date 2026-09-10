//! Le trait `Signal`, son contexte et son état — §16.1, §16.2, §18 de
//! `docs/ARCHITECTURE.md`.
//!
//! Invariant central : **tout est un `Signal`**. Oscillateur, LFO, enveloppe,
//! constante, mixeur, bande de fréquence audio — tous interchangeables, donc
//! n'importe quelle source module n'importe quelle destination.

/// Instantané des features audio, en lecture seule pendant la frame (§7).
///
/// Rempli par le thread d'analyse via un triple buffer ; le thread principal
/// ne bloque jamais dessus. Reste vide tant que l'étape 7 n'est pas faite.
#[derive(Debug, Clone, Default)]
pub struct AudioFeatures {
    /// Loudness global, `[0, 1]` (§17 : les features audio sont unipolaires).
    pub rms: f32,
    /// Bandes spectrales grave / médium / aigu, `[0, 1]`.
    pub bands: [f32; 3],
    /// Centroïde spectral normalisé, `[0, 1]`.
    pub centroid: f32,
    /// Un transitoire a été détecté pendant cette frame.
    pub onset: bool,
}

/// Contexte de frame — construit **une fois par frame**, passé en lecture seule.
///
/// Ne contient jamais l'horloge système : `t` est un paramètre séparé de `eval`,
/// précisément pour qu'un test puisse le fixer (§4).
#[derive(Debug, Clone, Copy)]
pub struct Ctx<'a> {
    /// Durée de la frame, en secondes. Nécessaire aux modules à état : une
    /// enveloppe ne peut pas déduire le temps écoulé de `t` seul.
    pub dt: f64,
    /// Compteur de frames depuis le début de la session.
    pub frame: u64,
    /// Fréquence d'échantillonnage audio.
    pub sample_rate: f64,
    /// Features audio de la frame.
    pub audio: &'a AudioFeatures,
    /// Cache d'évaluation de la frame, indexé par `NodeId::index()`.
    ///
    /// **Pourquoi le cache et non `&Graph` :** pendant `eval_frame`, le graphe
    /// mute le `NodeState` du nœud courant. Se prêter lui-même en `&Graph` au
    /// même moment viole l'emprunteur. Le cache et l'état sont deux champs
    /// distincts de `Graph`, donc les deux emprunts coexistent.
    ///
    /// C'est aussi plus honnête : un `Signal` n'a **pas** à voir la topologie
    /// du graphe, seulement les valeurs déjà calculées de ses dépendances.
    pub cache: &'a [f32],
}

impl<'a> Ctx<'a> {
    /// Contexte minimal pour les tests et les cas sans audio ni modulation.
    pub fn new(dt: f64, frame: u64, audio: &'a AudioFeatures) -> Self {
        Self {
            dt,
            frame,
            sample_rate: 48_000.0,
            audio,
            cache: &[],
        }
    }

    /// Contexte complet, construit par `Graph::eval_frame`.
    pub fn with_cache(dt: f64, frame: u64, audio: &'a AudioFeatures, cache: &'a [f32]) -> Self {
        Self {
            dt,
            frame,
            sample_rate: 48_000.0,
            audio,
            cache,
        }
    }
}

/// Tranche de mémoire persistante d'un nœud, prêtée par le graphe (§16.1).
///
/// Les modules n'ont **aucun champ mutable** : leur état vit ici. Bénéfices —
/// modules purs et testables, sérialisation et reset gratuits, et un nœud
/// partagé par plusieurs destinations reste trivial (pas d'emprunt mutable).
#[derive(Debug)]
pub struct NodeState<'a> {
    pub slots: &'a mut [f32],
}

impl<'a> NodeState<'a> {
    pub fn new(slots: &'a mut [f32]) -> Self {
        Self { slots }
    }

    /// Lecture tolérante : un slot hors bornes rend `0.0` plutôt que de paniquer.
    ///
    /// Le régime « jamais de panique en évaluation » du §18.1 s'applique aussi
    /// ici : un `state_size()` sous-dimensionné ne doit pas tuer la session.
    pub fn get(&self, i: usize) -> f32 {
        self.slots.get(i).copied().unwrap_or(0.0)
    }

    pub fn set(&mut self, i: usize, v: f32) {
        if let Some(s) = self.slots.get_mut(i) {
            *s = v;
        }
    }
}

/// Tout module produisant une valeur au cours du temps.
///
/// **Convention (§17) :** la sortie est nominalement `[-1, 1]` mais **non
/// bornée** — une somme de deux oscillateurs sort naturellement de
/// l'intervalle, et l'écrêter détruirait la FM. C'est au consommateur de
/// saturer son propre domaine (§18.4).
pub trait Signal {
    /// Évalue le signal à l'instant `t` (secondes).
    ///
    /// `state` est la mémoire persistante du nœud. Un module sans état l'ignore.
    fn eval(&self, t: f64, ctx: &Ctx, state: &mut NodeState) -> f32;

    /// Nombre de `f32` de mémoire persistante réclamés. `0` = sans état.
    ///
    /// **Convention :** documenter chaque slot en tête d'implémentation
    /// (`// slots[0] = phase accumulée`). Un slot non documenté est un bug
    /// de relecture.
    fn state_size(&self) -> usize {
        0
    }
}

/// Compteurs d'incohérences, remis à zéro à chaque frame (§18.1).
///
/// L'évaluation ne renvoie jamais d'erreur — à 60 fps, un flot d'erreurs est
/// inexploitable. Les incohérences sont comptées et affichées dans l'UI, sans
/// interrompre la performance. Les tests assertent que tout est à zéro (§18.5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Diagnostics {
    /// Un `NodeId` a été résolu vers rien (nœud supprimé, génération périmée).
    pub missing_node: u32,
    /// Un module a produit un `NaN` ou un infini.
    pub non_finite: u32,
    /// Une valeur a été saturée par son consommateur.
    pub clamped: u32,
}

impl Diagnostics {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Vrai si aucune incohérence n'a été relevée.
    pub fn is_clean(&self) -> bool {
        *self == Self::default()
    }
}

/// Signal constant — le plus simple des `Signal`, et le plus utile en test.
#[derive(Debug, Clone, Copy)]
pub struct Constant(pub f32);

impl Signal for Constant {
    fn eval(&self, _t: f64, _ctx: &Ctx, _state: &mut NodeState) -> f32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constante_ignore_le_temps() {
        let audio = AudioFeatures::default();
        let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
        let mut slots: [f32; 0] = [];
        let mut st = NodeState::new(&mut slots);

        let c = Constant(0.42);
        assert_eq!(c.eval(0.0, &ctx, &mut st), 0.42);
        assert_eq!(c.eval(123.456, &ctx, &mut st), 0.42);
        assert_eq!(c.state_size(), 0);
    }

    #[test]
    fn node_state_tolere_les_bornes() {
        // §18.1 : jamais de panique en évaluation, même si state_size() ment.
        let mut slots = [1.0f32, 2.0];
        let mut st = NodeState::new(&mut slots);
        assert_eq!(st.get(0), 1.0);
        assert_eq!(st.get(99), 0.0, "hors bornes doit rendre 0.0, pas paniquer");
        st.set(99, 5.0); // ne doit rien faire, et surtout pas paniquer
        assert_eq!(st.get(1), 2.0);
    }

    #[test]
    fn node_state_conserve_entre_appels() {
        let mut slots = [0.0f32; 1];
        let mut st = NodeState::new(&mut slots);
        st.set(0, 0.25);
        assert_eq!(st.get(0), 0.25);
        st.set(0, st.get(0) + 0.25);
        assert_eq!(st.get(0), 0.5);
    }

    #[test]
    fn diagnostics_propre_par_defaut() {
        let mut d = Diagnostics::default();
        assert!(d.is_clean());
        d.non_finite += 1;
        assert!(!d.is_clean());
        d.reset();
        assert!(d.is_clean());
    }

    #[test]
    fn audio_features_par_defaut_est_silence() {
        // Un patch audio-réactif doit être inerte, pas indéfini, sans audio.
        let a = AudioFeatures::default();
        assert_eq!(a.rms, 0.0);
        assert_eq!(a.bands, [0.0; 3]);
        assert!(!a.onset);
    }
}
