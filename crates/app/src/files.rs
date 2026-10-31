//! Boîtes de dialogue et disque — §23.8 de `docs/ARCHITECTURE.md`.
//!
//! [`FileFlow`] décide, ce module produit. Une boîte bloquante figerait l'image
//! sur la sortie : elles attendent sur un thread à part, comme le disque, et leurs
//! réponses reviennent par un canal lu à chaque frame.

use frogenx_app::file_flow::{FileEffect, FileFlow};
use frogenx_app::{last_patch, Engine};
use frogenx_composition::Composition;
use frogenx_library::starter;
use frogenx_patch::EXTENSION;
use frogenx_ui::{FileRequest, FileView};
use rfd::AsyncFileDialog;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use winit::window::Window;

const UNTITLED: &str = "Sans titre";

/// Ce que le thread d'attente rapporte.
enum Done {
    PickedOpen(Option<PathBuf>),
    PickedSave(Option<PathBuf>),
    Written(PathBuf, Result<(), String>),
    Read(PathBuf, Result<String, String>),
}

/// Ce que la frame doit faire de plus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reaction {
    Nothing,
    /// L'interface oublie sa sélection : ses identifiants visaient l'ancienne
    /// grille (§23.8).
    GridReplaced,
    Quit,
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Ce qu'affiche la barre : le fichier, et `*` s'il reste des modifications.
fn title(path: Option<&Path>, dirty: bool) -> String {
    let nom = path.map_or_else(|| UNTITLED.to_string(), file_name);
    if dirty {
        format!("{nom} *")
    } else {
        nom
    }
}

/// Le fichier courant et les travaux en attente.
pub struct Files {
    flow: FileFlow,
    sender: Sender<Done>,
    receiver: Receiver<Done>,
    /// `None` sans dossier de configuration : le dernier fichier n'est pas retenu.
    config: Option<PathBuf>,
    status: String,
}

/// La grille du lancement. Priorité à `cli`, puis au dernier fichier ; un fichier
/// qui se refuse ne doit jamais empêcher le lancement (§23.8).
fn load_at_startup(path: &Path) -> Result<Composition, String> {
    let texte = frogenx_patch::read(path).map_err(|e| e.to_string())?;
    frogenx_patch::from_json(&texte).map_err(|e| e.to_string())
}

impl Files {
    /// Rend aussi la grille de départ, et dit si c'est la grille d'usine : c'est
    /// la seule qui attend qu'on déclenche sa première colonne.
    pub fn start(cli: Option<PathBuf>) -> (Self, Composition, bool) {
        let (sender, receiver) = channel();
        let config = last_patch::config_dir(|nom| std::env::var(nom).ok());
        let dernier = config.as_deref().and_then(last_patch::read);
        let mut files = Self {
            flow: FileFlow::default(),
            sender,
            receiver,
            config,
            status: String::new(),
        };
        let Some(path) = last_patch::startup_path(cli, dernier) else {
            return (files, starter(), true);
        };
        match load_at_startup(&path) {
            Ok(grille) => {
                files.status = format!("Ouvert : {}", file_name(&path));
                files.flow.set_path(path);
                (files, grille, false)
            }
            Err(e) => {
                files.status = format!("{} refusé : {e}. Grille de départ.", file_name(&path));
                (files, starter(), true)
            }
        }
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn title(&self, dirty: bool) -> String {
        title(self.flow.path(), dirty)
    }

    /// Ce qu'en lit l'interface.
    pub fn view<'a>(&'a self, title: &'a str) -> FileView<'a> {
        FileView {
            title,
            status: &self.status,
            confirming: self.flow.confirming(),
        }
    }

    pub fn request(
        &mut self,
        request: FileRequest,
        window: &Window,
        engine: &mut Engine,
    ) -> Reaction {
        let effet = self.flow.request(request, engine.is_dirty());
        self.apply(effet, window, engine)
    }

    /// L'utilisateur ferme la fenêtre de l'interface.
    pub fn close_requested(&mut self, window: &Window, engine: &mut Engine) -> Reaction {
        let effet = self.flow.close_requested(engine.is_dirty());
        self.apply(effet, window, engine)
    }

    /// Reprend ce qui s'est terminé depuis la dernière frame.
    pub fn poll(&mut self, window: &Window, engine: &mut Engine) -> Reaction {
        let mut reaction = Reaction::Nothing;
        while let Ok(fini) = self.receiver.try_recv() {
            let effet = match fini {
                Done::PickedOpen(path) => self.flow.picked_open(path),
                Done::PickedSave(path) => self.flow.picked_save(path),
                Done::Written(path, resultat) => self.written(path, resultat, engine),
                Done::Read(path, resultat) => {
                    if self.finish_load(path, resultat, engine) {
                        reaction = Reaction::GridReplaced;
                    }
                    None
                }
            };
            match self.apply(effet, window, engine) {
                Reaction::Nothing => {}
                autre => reaction = autre,
            }
        }
        reaction
    }

    fn written(
        &mut self,
        path: PathBuf,
        resultat: Result<(), String>,
        engine: &mut Engine,
    ) -> Option<FileEffect> {
        match &resultat {
            Ok(()) => {
                self.status = format!("Enregistré : {}", file_name(&path));
                self.remember(&path);
            }
            Err(e) => {
                self.status = format!("Enregistrement impossible : {e}");
                engine.set_dirty(true);
            }
        }
        self.flow.saved(path, resultat.is_ok())
    }

    /// Décode à côté de la grille qui joue. Rend `true` si elle a été remplacée.
    fn finish_load(
        &mut self,
        path: PathBuf,
        texte: Result<String, String>,
        engine: &mut Engine,
    ) -> bool {
        let grille = texte.and_then(|t| frogenx_patch::from_json(&t).map_err(|e| e.to_string()));
        match grille {
            Ok(grille) => {
                engine.replace_composition(grille);
                self.status = format!("Ouvert : {}", file_name(&path));
                self.remember(&path);
                self.flow.loaded(path, true);
                true
            }
            Err(e) => {
                self.status = format!("Ouverture refusée : {e}");
                self.flow.loaded(path, false);
                false
            }
        }
    }

    fn remember(&self, path: &Path) {
        if let Some(dir) = &self.config {
            last_patch::write(dir, path);
        }
    }

    fn apply(
        &mut self,
        effect: Option<FileEffect>,
        window: &Window,
        engine: &mut Engine,
    ) -> Reaction {
        match effect {
            None => Reaction::Nothing,
            Some(FileEffect::Quit) => Reaction::Quit,
            Some(FileEffect::NewGrid) => {
                engine.play_starter(starter());
                self.status.clear();
                Reaction::GridReplaced
            }
            Some(FileEffect::PickOpen) => {
                let boite = dialog(window).pick_file();
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let path = pollster::block_on(boite).map(|f| f.path().to_path_buf());
                    let _ = sender.send(Done::PickedOpen(path));
                });
                Reaction::Nothing
            }
            Some(FileEffect::PickSave) => {
                let nom = self
                    .flow
                    .path()
                    .map_or_else(|| format!("{UNTITLED}.{EXTENSION}"), file_name);
                let boite = dialog(window).set_file_name(nom).save_file();
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let path = pollster::block_on(boite).map(|f| f.path().to_path_buf());
                    let _ = sender.send(Done::PickedSave(path));
                });
                Reaction::Nothing
            }
            Some(FileEffect::Save(path)) => self.save(path, engine),
            Some(FileEffect::Load(path)) => {
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let texte = frogenx_patch::read(&path).map_err(|e| e.to_string());
                    let _ = sender.send(Done::Read(path, texte));
                });
                Reaction::Nothing
            }
        }
    }

    /// Encode ici, entre deux frames : c'est l'état de cette frame qui s'écrit.
    /// Seul le disque part sur un thread. Une modification faite pendant
    /// l'écriture marque de nouveau la grille : `set_dirty` vient avant.
    fn save(&mut self, path: PathBuf, engine: &mut Engine) -> Reaction {
        match frogenx_patch::to_json(engine.composition()) {
            Ok(texte) => {
                engine.set_dirty(false);
                let sender = self.sender.clone();
                thread::spawn(move || {
                    let resultat = frogenx_patch::write(&path, &texte).map_err(|e| e.to_string());
                    let _ = sender.send(Done::Written(path, resultat));
                });
            }
            Err(e) => {
                self.status = format!("Enregistrement impossible : {e}");
                self.flow.saved(path, false);
            }
        }
        Reaction::Nothing
    }
}

fn dialog(window: &Window) -> AsyncFileDialog {
    AsyncFileDialog::new()
        .add_filter("Patch FrogenX", &[EXTENSION])
        .set_parent(window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_barre_dit_le_fichier_et_ses_modifications() {
        let chemin = Path::new("dossier").join("concert.frogenx");
        assert_eq!(title(Some(&chemin), false), "concert.frogenx");
        assert_eq!(title(Some(&chemin), true), "concert.frogenx *");
        assert_eq!(title(None, false), "Sans titre");
        assert_eq!(title(None, true), "Sans titre *");
    }
}
