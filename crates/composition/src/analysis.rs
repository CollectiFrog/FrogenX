//! Le réglage de l'analyse audio — §7 de `docs/ARCHITECTURE.md`.
//!
//! Il appartient à la composition, comme le tempo : une zone de trig n'a de sens
//! que relue avec les réglages qui l'ont fait poser, et un fichier les garde
//! ensemble. `composition` ne connaît pas la carte son : elle résout ce réglage
//! en [`AnalysisSettings`], et `app` le transmet à l'analyse.

use frogenx_core::{
    sanitize_window, AmpScale, AnalysisMethod, AnalysisSettings, FreqScale, Graph, Param,
    DEFAULT_WINDOW,
};

/// Comment la composition veut que son audio soit analysé.
///
/// **La durée est un `Param` :** rien n'interdit de la moduler, et l'analyseur
/// garde tout son historique en changeant de fenêtre (§7). Les trois autres
/// réglages sont des choix, pas des valeurs : ils se tranchent, ils ne se modulent
/// pas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioAnalysis {
    pub method: AnalysisMethod,
    pub freq_scale: FreqScale,
    pub amp_scale: AmpScale,
    /// La durée de signal analysée, en secondes.
    pub window: Param,
}

impl Default for AudioAnalysis {
    fn default() -> Self {
        Self {
            method: AnalysisMethod::default(),
            freq_scale: FreqScale::default(),
            amp_scale: AmpScale::default(),
            window: Param::fixed(DEFAULT_WINDOW),
        }
    }
}

impl AudioAnalysis {
    /// Les réglages de la frame, à lire **après** `eval_frame` du graphe de
    /// composition. La durée est saturée ici, chez le consommateur (§18.4).
    pub fn resolve(&self, graph: &Graph) -> AnalysisSettings {
        AnalysisSettings {
            method: self.method,
            freq_scale: self.freq_scale,
            amp_scale: self.amp_scale,
            window: sanitize_window(self.window.get(graph)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frogenx_core::{Constant, MAX_WINDOW, MIN_WINDOW};

    #[test]
    fn par_defaut_le_reglage_vaut_celui_de_l_analyseur() {
        let g = Graph::new();
        assert_eq!(
            AudioAnalysis::default().resolve(&g),
            AnalysisSettings::default()
        );
    }

    #[test]
    fn la_duree_est_saturee_a_la_lecture() {
        let g = Graph::new();
        let avec = |s: f32| AudioAnalysis {
            window: Param::fixed(s),
            ..Default::default()
        };
        assert_eq!(avec(0.0).resolve(&g).window, MIN_WINDOW);
        assert_eq!(avec(80.0).resolve(&g).window, MAX_WINDOW);
        assert_eq!(avec(0.25).resolve(&g).window, 0.25);
    }

    #[test]
    fn la_duree_se_module_comme_n_importe_quel_param() {
        let mut g = Graph::new();
        let lfo = g.add(Box::new(Constant(1.0)));
        g.eval_frame(0.0, 1.0 / 60.0);
        let reglage = AudioAnalysis {
            window: Param::modulated(0.1, 0.05, lfo),
            ..Default::default()
        };
        assert!((reglage.resolve(&g).window - 0.15).abs() < 1e-6);
    }
}
