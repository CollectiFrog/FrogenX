//! # frogenx-patch
//!
//! Le format de fichier — §19 de `docs/ARCHITECTURE.md`.
//!
//! [`to_json`] décrit une `Composition`, [`from_json`] en construit une neuve à
//! côté de celle qui joue : un fichier se charge en entier ou pas du tout.
//! [`save`] écrit par un fichier temporaire, pour qu'un plantage laisse l'ancienne
//! version intacte.
//!
//! Un fichier décrit une configuration, pas un instant : les clips actifs y
//! figurent, pas les phases.

mod decode;
mod encode;
mod file;

use frogenx_composition::Composition;
use frogenx_core::{GraphError, SpecError};
use serde_json::Value;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// La version écrite par ce programme. Tout changement du format l'incrémente,
/// avec sa migration et sa fixture (§19.2).
pub const CURRENT_VERSION: u64 = 3;
/// Plus de colonnes que ça n'est pas une grille : un fichier abîmé.
pub const MAX_COLUMNS: usize = 4096;
/// Refusé avant d'allouer quoi que ce soit.
pub const MAX_FILE_SIZE: u64 = 64 * 1024 * 1024;
/// L'extension des fichiers de patch.
pub const EXTENSION: &str = "frogenx";

/// Un fichier refusé : régime d'édition (§18.1).
#[derive(Debug)]
pub enum PatchError {
    Io(io::Error),
    Json(serde_json::Error),
    NoVersion,
    UnknownVersion(u64),
    TooLarge(u64),
    Spec(SpecError),
    /// Une référence qui ne désigne rien : position, colonne, case.
    Invalid(String),
    /// Une valeur que JSON n'exprime pas : le fichier ne se rouvrirait plus.
    NotFinite(String),
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Json(e) => write!(f, "fichier illisible : {e}"),
            Self::NoVersion => write!(f, "ce n'est pas un patch FrogenX : aucune version"),
            Self::UnknownVersion(v) => write!(
                f,
                "version {v} du format inconnue : ce programme lit jusqu'à la {CURRENT_VERSION}"
            ),
            Self::TooLarge(n) => write!(f, "fichier trop volumineux : {n} octets"),
            Self::Spec(e) => write!(f, "{e}"),
            Self::Invalid(e) => write!(f, "fichier incohérent : {e}"),
            Self::NotFinite(e) => write!(f, "valeur non finie : {e}"),
        }
    }
}

impl std::error::Error for PatchError {}

impl From<io::Error> for PatchError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for PatchError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl From<SpecError> for PatchError {
    fn from(e: SpecError) -> Self {
        Self::Spec(e)
    }
}

impl From<GraphError> for PatchError {
    fn from(e: GraphError) -> Self {
        Self::Invalid(format!("graphe refusé : {e:?}"))
    }
}

/// Décrit `composition` en JSON. Rend une erreur plutôt qu'un fichier qui ne se
/// rouvrirait pas : le texte produit est relu avant d'être rendu.
pub fn to_json(composition: &Composition) -> Result<String, PatchError> {
    let json = serde_json::to_string_pretty(&encode::patch(composition)?)?;
    from_json(&json)?;
    Ok(json)
}

/// Construit une `Composition` neuve. Toute erreur la laisse de côté.
pub fn from_json(json: &str) -> Result<Composition, PatchError> {
    let brut: Value = serde_json::from_str(json)?;
    decode::patch(serde_json::from_value(migrate(brut)?)?)
}

/// Amène un fichier à la version courante, une version à la fois (§19.2).
fn migrate(mut data: Value) -> Result<Value, PatchError> {
    let mut version = data["version"].as_u64().ok_or(PatchError::NoVersion)?;
    if version == 0 || version > CURRENT_VERSION {
        return Err(PatchError::UnknownVersion(version));
    }
    while version < CURRENT_VERSION {
        data = match version {
            1 => migrate_1_to_2(data),
            2 => migrate_2_to_3(data),
            _ => return Err(PatchError::UnknownVersion(version)),
        };
        version += 1;
    }
    Ok(data)
}

/// La version 2 ajoute le suivi du tempo (§7.1) : un fichier v1 ne suivait rien.
fn migrate_1_to_2(mut data: Value) -> Value {
    data["tempo_follow"] = Value::Bool(false);
    data["version"] = Value::from(2);
    data
}

/// La version 3 ajoute les clips de feedback (§6.1) : un fichier v2 n'en a pas, et
/// `generator` et `style` y sont tous présents.
fn migrate_2_to_3(mut data: Value) -> Value {
    data["version"] = Value::from(3);
    data
}

/// Le fichier temporaire de `path`, dans le même dossier : un renommage entre
/// deux disques ne serait pas atomique.
fn temporary(path: &Path) -> PathBuf {
    let mut nom = path.file_name().unwrap_or_default().to_os_string();
    nom.push(".tmp");
    path.with_file_name(nom)
}

/// Écrit `json` dans `path`. Sur le disque d'abord, renommé ensuite : un
/// plantage laisse l'ancienne version intacte (§19.1).
pub fn write(path: &Path, json: &str) -> Result<(), PatchError> {
    let temporaire = temporary(path);
    let ecriture = (|| {
        let mut fichier = fs::File::create(&temporaire)?;
        fichier.write_all(json.as_bytes())?;
        fichier.sync_all()
    })();
    if let Err(e) = ecriture.and_then(|()| fs::rename(&temporaire, path)) {
        let _ = fs::remove_file(&temporaire);
        return Err(e.into());
    }
    Ok(())
}

/// Lit un fichier, sans le décoder. Refuse d'abord ce qui est trop volumineux.
pub fn read(path: &Path) -> Result<String, PatchError> {
    let taille = fs::metadata(path)?.len();
    if taille > MAX_FILE_SIZE {
        return Err(PatchError::TooLarge(taille));
    }
    Ok(fs::read_to_string(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dossier(nom: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("frogenx-patch-{nom}-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn sans_version_ce_n_est_pas_un_patch() {
        assert!(matches!(from_json("{}"), Err(PatchError::NoVersion)));
        assert!(matches!(from_json("[1, 2]"), Err(PatchError::NoVersion)));
    }

    #[test]
    fn une_version_plus_recente_est_refusee_sans_deviner() {
        let json = format!("{{\"version\": {}}}", CURRENT_VERSION + 1);
        let erreur = from_json(&json).err().expect("refusé");
        assert!(matches!(erreur, PatchError::UnknownVersion(4)));
        assert!(erreur.to_string().contains("version 4"));
    }

    #[test]
    fn la_migration_v1_ne_suit_pas_le_tempo() {
        let migre = migrate(serde_json::json!({"version": 1})).unwrap();
        assert_eq!(migre["tempo_follow"], Value::Bool(false));
        assert_eq!(migre["version"], Value::from(CURRENT_VERSION));
    }

    #[test]
    fn la_version_zero_n_existe_pas() {
        assert!(matches!(
            from_json("{\"version\": 0}"),
            Err(PatchError::UnknownVersion(0))
        ));
    }

    #[test]
    fn du_texte_qui_n_est_pas_du_json_est_refuse() {
        assert!(matches!(from_json("pas du json"), Err(PatchError::Json(_))));
    }

    #[test]
    fn ecrire_remplace_le_fichier_sans_laisser_de_temporaire() {
        let d = dossier("ecrire");
        let chemin = d.join("a.frogenx");
        write(&chemin, "premier").unwrap();
        write(&chemin, "second").unwrap();
        assert_eq!(read(&chemin).unwrap(), "second");
        assert!(!temporary(&chemin).exists());
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn un_echec_d_ecriture_laisse_l_ancienne_version() {
        let d = dossier("echec");
        let chemin = d.join("a.frogenx");
        write(&chemin, "ancien").unwrap();
        // Le temporaire est un dossier : le créer échoue.
        fs::create_dir(temporary(&chemin)).unwrap();
        assert!(write(&chemin, "nouveau").is_err());
        assert_eq!(read(&chemin).unwrap(), "ancien");
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn un_fichier_absent_se_dit_en_clair() {
        let d = dossier("absent");
        assert!(matches!(read(&d.join("rien")), Err(PatchError::Io(_))));
        fs::remove_dir_all(d).unwrap();
    }
}
