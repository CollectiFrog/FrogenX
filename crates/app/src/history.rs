//! Annuler et rétablir — §23.11 de `docs/ARCHITECTURE.md`.
//!
//! Des images de la grille, dans le format de fichier. Ce module décide seulement
//! quand en prendre une et laquelle rendre : il ne connaît que du texte, et se
//! teste sans grille. `Engine` encode et restaure.

use std::collections::VecDeque;

/// Le plus de pas gardés : 100 images de 90 Ko, 9 Mo.
pub const MAX_STEPS: usize = 100;
/// Frames sans modification avant de fermer un pas : 200 ms à 60 fps. Groupe une
/// saisie au clavier, qu'aucun bouton tenu ne délimite.
pub const QUIET_FRAMES: u32 = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    json: String,
    /// La modification qui a mené à cette image.
    label: &'static str,
    /// Comparé à celui de la dernière sauvegarde.
    revision: u64,
}

/// Ce qu'il faut restaurer, et le nom du pas traversé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restore {
    pub json: String,
    pub label: &'static str,
}

/// L'historique. Sans image de départ, il ne fait rien : une grille qui ne
/// s'enregistre pas ne s'annule pas non plus.
#[derive(Debug, Default)]
pub struct History {
    current: Option<Step>,
    undo: VecDeque<Step>,
    redo: Vec<Step>,
    /// Le pas ouvert : le nom de sa première modification.
    pending: Option<&'static str>,
    quiet: u32,
    next_revision: u64,
    saved: Option<u64>,
}

impl History {
    /// Repart de cette image, considérée comme enregistrée.
    pub fn reset(&mut self, json: Option<String>) {
        *self = Self::default();
        self.current = json.map(|json| Step {
            json,
            label: "",
            revision: 0,
        });
        self.saved = Some(0);
        self.next_revision = 1;
    }

    /// Une modification vient d'être appliquée.
    pub fn note(&mut self, label: &'static str) {
        self.pending.get_or_insert(label);
        self.quiet = 0;
    }

    /// À chaque frame. Rend `true` s'il faut fermer le pas ouvert.
    pub fn tick(&mut self, gesture_held: bool) -> bool {
        if self.pending.is_none() {
            return false;
        }
        self.quiet = self.quiet.saturating_add(1);
        !gesture_held && self.quiet >= QUIET_FRAMES
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Ferme le pas ouvert sur cette image. `None` : la grille ne s'enregistre pas,
    /// le pas est perdu plutôt que faux.
    pub fn commit(&mut self, json: Option<String>) {
        let Some(label) = self.pending.take() else {
            return;
        };
        self.quiet = 0;
        let Some(json) = json else {
            return;
        };
        if self.current.as_ref().is_some_and(|c| c.json == json) {
            return; // un réglage revenu à sa valeur : rien à annuler
        }
        let revision = self.next_revision;
        self.next_revision += 1;
        let suivante = Step {
            json,
            label,
            revision,
        };
        if let Some(ancienne) = self.current.replace(suivante) {
            self.undo.push_back(ancienne);
        }
        self.redo.clear();
        if self.undo.len() > MAX_STEPS {
            self.undo.pop_front();
        }
    }

    /// Le nom du pas que `undo` annulerait.
    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.back().and(self.current.as_ref()).map(|c| c.label)
    }

    /// Le nom du pas que `redo` rétablirait.
    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(|s| s.label)
    }

    /// Recule d'un pas. Le pas ouvert doit être fermé.
    pub fn undo(&mut self) -> Option<Restore> {
        let precedente = self.undo.pop_back()?;
        let actuelle = self.current.replace(precedente)?;
        let label = actuelle.label;
        self.redo.push(actuelle);
        let json = self.current.as_ref()?.json.clone();
        Some(Restore { json, label })
    }

    /// Avance d'un pas.
    pub fn redo(&mut self) -> Option<Restore> {
        let suivante = self.redo.pop()?;
        let label = suivante.label;
        let json = suivante.json.clone();
        if let Some(actuelle) = self.current.replace(suivante) {
            self.undo.push_back(actuelle);
        }
        Some(Restore { json, label })
    }

    /// L'image courante vient d'être enregistrée.
    pub fn mark_saved(&mut self) {
        self.saved = self.current.as_ref().map(|c| c.revision);
    }

    /// L'enregistrement a échoué : aucune image ne vaut plus le fichier.
    pub fn mark_unsaved(&mut self) {
        self.saved = None;
    }

    /// Est-on revenu à l'image enregistrée ?
    pub fn at_saved(&self) -> bool {
        self.pending.is_none()
            && self.saved.is_some()
            && self.current.as_ref().map(|c| c.revision) == self.saved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn neuf() -> History {
        let mut h = History::default();
        h.reset(Some("0".into()));
        h
    }

    /// Une modification et son image.
    fn pas(h: &mut History, label: &'static str, json: &str) {
        h.note(label);
        h.commit(Some(json.into()));
    }

    #[test]
    fn annuler_rend_l_image_d_avant_et_nomme_le_pas() {
        let mut h = neuf();
        pas(&mut h, "Réglage", "1");
        assert_eq!(h.undo_label(), Some("Réglage"));
        assert_eq!(
            h.undo(),
            Some(Restore {
                json: "0".into(),
                label: "Réglage"
            })
        );
        assert_eq!(h.undo(), None, "rien avant le départ");
    }

    #[test]
    fn retablir_refait_ce_qui_a_ete_annule() {
        let mut h = neuf();
        pas(&mut h, "A", "1");
        pas(&mut h, "B", "2");
        h.undo();
        h.undo();
        assert_eq!(h.redo_label(), Some("A"));
        assert_eq!(h.redo().unwrap().json, "1");
        assert_eq!(h.redo().unwrap().json, "2");
        assert_eq!(h.redo(), None);
        assert_eq!(h.undo_label(), Some("B"));
    }

    #[test]
    fn une_modification_apres_annuler_efface_ce_qu_on_pouvait_retablir() {
        let mut h = neuf();
        pas(&mut h, "A", "1");
        h.undo();
        pas(&mut h, "B", "2");
        assert_eq!(h.redo(), None);
    }

    #[test]
    fn un_geste_tenu_ne_fait_qu_un_pas() {
        let mut h = neuf();
        for _ in 0..200 {
            h.note("Réglage");
            assert!(!h.tick(true), "bouton tenu : le pas reste ouvert");
        }
        for _ in 2..QUIET_FRAMES {
            assert!(!h.tick(false), "pas encore assez de calme");
        }
        assert!(h.tick(false));
        h.commit(Some("1".into()));
        assert_eq!(h.undo().unwrap().json, "0");
        assert_eq!(h.undo(), None);
    }

    #[test]
    fn deux_modifications_rapprochees_au_clavier_ne_font_qu_un_pas() {
        let mut h = neuf();
        h.note("Réglage");
        h.tick(false);
        h.note("Réglage");
        for _ in 1..QUIET_FRAMES {
            assert!(!h.tick(false));
        }
        assert!(h.tick(false));
    }

    #[test]
    fn sans_modification_il_n_y_a_rien_a_fermer() {
        let mut h = neuf();
        for _ in 0..100 {
            assert!(!h.tick(false));
        }
    }

    #[test]
    fn le_premier_nom_d_un_pas_est_le_sien() {
        let mut h = neuf();
        h.note("Câblage");
        h.note("Réglage");
        h.commit(Some("1".into()));
        assert_eq!(h.undo_label(), Some("Câblage"));
    }

    #[test]
    fn un_reglage_revenu_a_sa_valeur_ne_cree_pas_de_pas() {
        let mut h = neuf();
        pas(&mut h, "Réglage", "0");
        assert_eq!(h.undo_label(), None);
    }

    #[test]
    fn l_historique_est_borne_et_oublie_le_plus_ancien() {
        let mut h = neuf();
        for i in 1..=MAX_STEPS + 20 {
            pas(&mut h, "A", &i.to_string());
        }
        let mut n = 0;
        let mut derniere = String::new();
        while let Some(r) = h.undo() {
            n += 1;
            derniere = r.json;
        }
        assert_eq!(n, MAX_STEPS);
        assert_eq!(derniere, "20");
    }

    #[test]
    fn revenir_a_l_etat_enregistre_le_reconnait() {
        let mut h = neuf();
        assert!(h.at_saved());
        pas(&mut h, "A", "1");
        assert!(!h.at_saved());
        h.undo();
        assert!(h.at_saved());
        h.redo();
        assert!(!h.at_saved());
    }

    #[test]
    fn enregistrer_deplace_l_etat_de_reference() {
        let mut h = neuf();
        pas(&mut h, "A", "1");
        h.mark_saved();
        assert!(h.at_saved());
        h.undo();
        assert!(!h.at_saved());
    }

    #[test]
    fn un_pas_ouvert_n_est_pas_l_etat_enregistre() {
        let mut h = neuf();
        h.note("A");
        assert!(!h.at_saved());
    }

    #[test]
    fn un_echec_d_enregistrement_ne_laisse_aucune_reference() {
        let mut h = neuf();
        h.mark_unsaved();
        assert!(!h.at_saved());
    }

    #[test]
    fn une_grille_qui_ne_s_enregistre_pas_n_a_pas_d_historique() {
        let mut h = History::default();
        h.reset(None);
        h.note("A");
        h.commit(None);
        assert_eq!(h.undo_label(), None);
        assert_eq!(h.undo(), None);
        assert!(!h.has_pending());
    }

    #[test]
    fn repartir_efface_tout() {
        let mut h = neuf();
        pas(&mut h, "A", "1");
        h.reset(Some("2".into()));
        assert_eq!(h.undo(), None);
        assert!(h.at_saved());
    }
}
