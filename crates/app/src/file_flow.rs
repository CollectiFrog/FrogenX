//! Le parcours du fichier — §23.8 de `docs/ARCHITECTURE.md`.
//!
//! Enregistrer, renoncer et quitter forment un parcours où l'on peut perdre son
//! travail. [`FileFlow`] le décide sans `rfd` ni `winit` : il reçoit les demandes
//! et les réponses, et rend l'effet à produire. Le binaire se contente de le
//! produire.

use frogenx_patch::EXTENSION;
use frogenx_ui::{Confirmation, FileRequest};
use std::path::{Path, PathBuf};

/// Ce qu'on ne fait pas sans proposer d'enregistrer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leaving {
    New,
    Open,
    Quit,
}

/// Ce que le binaire doit produire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEffect {
    PickOpen,
    PickSave,
    Save(PathBuf),
    Load(PathBuf),
    NewGrid,
    Quit,
}

/// Le fichier courant, et où en est le parcours.
#[derive(Debug, Default)]
pub struct FileFlow {
    path: Option<PathBuf>,
    /// La question posée : l'interface montre sa fenêtre tant qu'elle est là.
    confirm: Option<Leaving>,
    /// Ce qui reprend quand l'enregistrement aura réussi.
    after_save: Option<Leaving>,
    /// Une boîte de dialogue ou le disque travaille : une seule chose à la fois.
    busy: bool,
}

impl FileFlow {
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Le fichier ouvert au lancement.
    pub fn set_path(&mut self, path: PathBuf) {
        self.path = Some(path);
    }

    pub fn confirming(&self) -> bool {
        self.confirm.is_some()
    }

    pub fn request(&mut self, request: FileRequest, dirty: bool) -> Option<FileEffect> {
        if self.busy {
            return None;
        }
        match request {
            FileRequest::New => self.leave(Leaving::New, dirty),
            FileRequest::Open => self.leave(Leaving::Open, dirty),
            FileRequest::Save => self.save(),
            FileRequest::SaveAs => self.work(FileEffect::PickSave),
            FileRequest::Confirm(reponse) => {
                let quitte = self.confirm.take()?;
                match reponse {
                    Confirmation::Save => {
                        self.after_save = Some(quitte);
                        self.save()
                    }
                    Confirmation::Discard => self.proceed(quitte),
                    Confirmation::Cancel => None,
                }
            }
        }
    }

    /// L'utilisateur ferme la fenêtre de l'interface. Attendre que le disque ait
    /// fini vaut mieux qu'un fichier écrit à moitié : la demande est ignorée.
    pub fn close_requested(&mut self, dirty: bool) -> Option<FileEffect> {
        if self.busy {
            return None;
        }
        self.leave(Leaving::Quit, dirty)
    }

    /// La réponse de la boîte d'ouverture.
    pub fn picked_open(&mut self, path: Option<PathBuf>) -> Option<FileEffect> {
        self.busy = false;
        self.work(FileEffect::Load(path?))
    }

    /// La réponse de la boîte d'enregistrement. Y renoncer abandonne aussi ce
    /// qui attendait : rien ne se perd en silence.
    pub fn picked_save(&mut self, path: Option<PathBuf>) -> Option<FileEffect> {
        self.busy = false;
        let Some(path) = path else {
            self.after_save = None;
            return None;
        };
        self.work(FileEffect::Save(with_extension(path)))
    }

    /// L'écriture est finie. Un échec abandonne ce qui attendait.
    pub fn saved(&mut self, path: PathBuf, success: bool) -> Option<FileEffect> {
        self.busy = false;
        if success {
            self.path = Some(path);
            let suite = self.after_save.take()?;
            self.proceed(suite)
        } else {
            self.after_save = None;
            None
        }
    }

    /// La lecture est finie. Un refus laisse le fichier courant.
    pub fn loaded(&mut self, path: PathBuf, success: bool) {
        self.busy = false;
        if success {
            self.path = Some(path);
        }
    }

    fn leave(&mut self, leaving: Leaving, dirty: bool) -> Option<FileEffect> {
        if dirty {
            self.confirm = Some(leaving);
            None
        } else {
            self.proceed(leaving)
        }
    }

    fn proceed(&mut self, leaving: Leaving) -> Option<FileEffect> {
        match leaving {
            Leaving::New => {
                self.path = None;
                Some(FileEffect::NewGrid)
            }
            Leaving::Open => self.work(FileEffect::PickOpen),
            Leaving::Quit => Some(FileEffect::Quit),
        }
    }

    fn save(&mut self) -> Option<FileEffect> {
        match self.path.clone() {
            Some(path) => self.work(FileEffect::Save(path)),
            None => self.work(FileEffect::PickSave),
        }
    }

    fn work(&mut self, effect: FileEffect) -> Option<FileEffect> {
        self.busy = true;
        Some(effect)
    }
}

/// Une boîte d'enregistrement peut rendre un nom sans extension.
fn with_extension(path: PathBuf) -> PathBuf {
    if path.extension().is_some() {
        path
    } else {
        path.with_extension(EXTENSION)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(nom: &str) -> PathBuf {
        PathBuf::from(nom)
    }

    /// Un parcours avec un fichier déjà ouvert.
    fn ouvert() -> FileFlow {
        let mut flow = FileFlow::default();
        flow.set_path(p("a.frogenx"));
        flow
    }

    #[test]
    fn enregistrer_sans_fichier_demande_un_nom() {
        let mut flow = FileFlow::default();
        assert_eq!(
            flow.request(FileRequest::Save, true),
            Some(FileEffect::PickSave)
        );
        assert_eq!(
            flow.picked_save(Some(p("b.frogenx"))),
            Some(FileEffect::Save(p("b.frogenx")))
        );
        assert_eq!(flow.saved(p("b.frogenx"), true), None);
        assert_eq!(flow.path(), Some(Path::new("b.frogenx")));
    }

    #[test]
    fn enregistrer_avec_un_fichier_ecrit_directement() {
        let mut flow = ouvert();
        assert_eq!(
            flow.request(FileRequest::Save, true),
            Some(FileEffect::Save(p("a.frogenx")))
        );
    }

    #[test]
    fn enregistrer_sous_demande_toujours_un_nom() {
        let mut flow = ouvert();
        assert_eq!(
            flow.request(FileRequest::SaveAs, false),
            Some(FileEffect::PickSave)
        );
    }

    #[test]
    fn un_nom_sans_extension_recoit_la_sienne() {
        let mut flow = FileFlow::default();
        flow.request(FileRequest::SaveAs, false);
        assert_eq!(
            flow.picked_save(Some(p("concert"))),
            Some(FileEffect::Save(p("concert.frogenx")))
        );
    }

    #[test]
    fn sans_modification_on_ouvre_et_on_quitte_sans_question() {
        let mut flow = ouvert();
        assert_eq!(
            flow.request(FileRequest::Open, false),
            Some(FileEffect::PickOpen)
        );
        assert_eq!(flow.picked_open(None), None);
        assert_eq!(flow.close_requested(false), Some(FileEffect::Quit));
        assert!(!flow.confirming());
    }

    #[test]
    fn avec_des_modifications_quitter_pose_d_abord_la_question() {
        let mut flow = ouvert();
        assert_eq!(flow.close_requested(true), None);
        assert!(flow.confirming());
    }

    #[test]
    fn annuler_ne_fait_rien_et_ferme_la_question() {
        let mut flow = ouvert();
        flow.close_requested(true);
        let annuler = FileRequest::Confirm(Confirmation::Cancel);
        assert_eq!(flow.request(annuler, true), None);
        assert!(!flow.confirming());
    }

    #[test]
    fn ne_pas_enregistrer_quitte_tout_de_suite() {
        let mut flow = ouvert();
        flow.close_requested(true);
        let abandon = FileRequest::Confirm(Confirmation::Discard);
        assert_eq!(flow.request(abandon, true), Some(FileEffect::Quit));
    }

    #[test]
    fn enregistrer_puis_quitter_attend_que_le_disque_ait_fini() {
        let mut flow = ouvert();
        flow.close_requested(true);
        let oui = FileRequest::Confirm(Confirmation::Save);
        assert_eq!(
            flow.request(oui, true),
            Some(FileEffect::Save(p("a.frogenx")))
        );
        assert_eq!(flow.saved(p("a.frogenx"), true), Some(FileEffect::Quit));
    }

    #[test]
    fn une_ecriture_ratee_ne_quitte_pas() {
        let mut flow = ouvert();
        flow.close_requested(true);
        flow.request(FileRequest::Confirm(Confirmation::Save), true);
        assert_eq!(flow.saved(p("a.frogenx"), false), None);
        assert_eq!(flow.path(), Some(Path::new("a.frogenx")));
    }

    #[test]
    fn renoncer_au_nom_du_fichier_ne_quitte_pas() {
        let mut flow = FileFlow::default();
        flow.close_requested(true);
        assert_eq!(
            flow.request(FileRequest::Confirm(Confirmation::Save), true),
            Some(FileEffect::PickSave)
        );
        assert_eq!(flow.picked_save(None), None);
        // Le parcours reste utilisable, et ne quittera pas au prochain succès.
        flow.request(FileRequest::Save, true);
        flow.picked_save(Some(p("b.frogenx")));
        assert_eq!(flow.saved(p("b.frogenx"), true), None);
    }

    #[test]
    fn nouveau_oublie_le_fichier_courant() {
        let mut flow = ouvert();
        assert_eq!(
            flow.request(FileRequest::New, false),
            Some(FileEffect::NewGrid)
        );
        assert_eq!(flow.path(), None);
        // Enregistrer ne doit pas écraser l'ancien fichier.
        assert_eq!(
            flow.request(FileRequest::Save, true),
            Some(FileEffect::PickSave)
        );
    }

    #[test]
    fn nouveau_avec_des_modifications_garde_le_fichier_si_on_annule() {
        let mut flow = ouvert();
        assert_eq!(flow.request(FileRequest::New, true), None);
        flow.request(FileRequest::Confirm(Confirmation::Cancel), true);
        assert_eq!(flow.path(), Some(Path::new("a.frogenx")));
    }

    #[test]
    fn ouvrir_apres_avoir_enregistre_reprend_la_boite_d_ouverture() {
        let mut flow = ouvert();
        flow.request(FileRequest::Open, true);
        flow.request(FileRequest::Confirm(Confirmation::Save), true);
        assert_eq!(flow.saved(p("a.frogenx"), true), Some(FileEffect::PickOpen));
        assert_eq!(
            flow.picked_open(Some(p("b.frogenx"))),
            Some(FileEffect::Load(p("b.frogenx")))
        );
        flow.loaded(p("b.frogenx"), true);
        assert_eq!(flow.path(), Some(Path::new("b.frogenx")));
    }

    #[test]
    fn un_fichier_refuse_laisse_le_fichier_courant() {
        let mut flow = ouvert();
        flow.request(FileRequest::Open, false);
        flow.picked_open(Some(p("abime.frogenx")));
        flow.loaded(p("abime.frogenx"), false);
        assert_eq!(flow.path(), Some(Path::new("a.frogenx")));
    }

    #[test]
    fn une_seule_chose_a_la_fois() {
        let mut flow = ouvert();
        flow.request(FileRequest::Save, true);
        assert_eq!(flow.request(FileRequest::Save, true), None);
        assert_eq!(flow.request(FileRequest::Open, false), None);
    }

    #[test]
    fn on_ne_quitte_pas_pendant_que_le_disque_ecrit() {
        let mut flow = ouvert();
        flow.request(FileRequest::Save, false);
        assert_eq!(flow.close_requested(false), None);
        flow.saved(p("a.frogenx"), true);
        assert_eq!(flow.close_requested(false), Some(FileEffect::Quit));
    }

    #[test]
    fn une_reponse_sans_question_ne_fait_rien() {
        let mut flow = ouvert();
        let abandon = FileRequest::Confirm(Confirmation::Discard);
        assert_eq!(flow.request(abandon, true), None);
    }
}
