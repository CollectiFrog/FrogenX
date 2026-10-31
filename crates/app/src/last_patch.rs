//! Le dernier fichier ouvert ou enregistré — §23.8 de `docs/ARCHITECTURE.md`.
//!
//! Un chemin dans un fichier texte, dans le dossier de configuration du système.
//! Rien ici ne doit empêcher un lancement : toute erreur vaut « aucun fichier ».

use std::fs;
use std::path::{Path, PathBuf};

/// Le fichier qui retient le chemin.
const FILE_NAME: &str = "dernier_patch.txt";

/// Le dossier de configuration. `FROGENX_CONFIG_DIR` le remplace, pour essayer
/// l'application sans toucher à celui de l'utilisateur.
pub fn config_dir(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(dir) = env("FROGENX_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    if cfg!(windows) {
        return env("APPDATA").map(|d| PathBuf::from(d).join("FrogenX"));
    }
    if cfg!(target_os = "macos") {
        let home = PathBuf::from(env("HOME")?);
        return Some(home.join("Library/Application Support/FrogenX"));
    }
    env("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|d| d.join("frogenx"))
}

/// Le chemin retenu, s'il y en a un.
pub fn read(dir: &Path) -> Option<PathBuf> {
    let texte = fs::read_to_string(dir.join(FILE_NAME)).ok()?;
    let chemin = texte.trim();
    (!chemin.is_empty()).then(|| PathBuf::from(chemin))
}

/// Retient `path`. Rend `false` si ça n'a pas pu se faire : le travail de
/// l'utilisateur n'en dépend pas.
pub fn write(dir: &Path, path: &Path) -> bool {
    // Pas `canonicalize`, qui préfixe `\\?\` sous Windows : `--patch` peut être
    // relatif, et le lancement suivant partir d'ailleurs.
    let absolu = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    fs::create_dir_all(dir).is_ok()
        && fs::write(dir.join(FILE_NAME), absolu.to_string_lossy().as_bytes()).is_ok()
}

/// Le fichier à ouvrir au lancement : la ligne de commande passe d'abord.
pub fn startup_path(cli: Option<PathBuf>, last: Option<PathBuf>) -> Option<PathBuf> {
    cli.or(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dossier(nom: &str) -> PathBuf {
        std::env::temp_dir().join(format!("frogenx-last-{nom}-{}", std::process::id()))
    }

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |nom| {
            vars.iter()
                .find(|(n, _)| *n == nom)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn la_variable_d_essai_passe_devant_le_systeme() {
        let vars = [
            ("FROGENX_CONFIG_DIR", "/essai"),
            ("APPDATA", "/a"),
            ("HOME", "/h"),
        ];
        assert_eq!(config_dir(env(&vars)), Some(PathBuf::from("/essai")));
    }

    #[test]
    fn sans_environnement_il_n_y_a_pas_de_dossier() {
        assert_eq!(config_dir(env(&[])), None);
    }

    #[test]
    fn un_chemin_retenu_se_relit() {
        let d = dossier("relit");
        let patch = d.join("a.frogenx");
        assert!(write(&d, &patch));
        assert_eq!(read(&d), Some(patch));
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn rien_de_retenu_vaut_aucun_fichier() {
        assert_eq!(read(&dossier("rien")), None);
    }

    #[test]
    fn un_fichier_vide_vaut_aucun_fichier() {
        let d = dossier("vide");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(FILE_NAME), "  \n").unwrap();
        assert_eq!(read(&d), None);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn la_ligne_de_commande_passe_devant_le_dernier_fichier() {
        let (a, b) = (PathBuf::from("a"), PathBuf::from("b"));
        assert_eq!(startup_path(Some(a.clone()), Some(b.clone())), Some(a));
        assert_eq!(startup_path(None, Some(b.clone())), Some(b));
        assert_eq!(startup_path(None, None), None);
    }
}
