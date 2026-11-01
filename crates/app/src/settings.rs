//! Les réglages de la machine : le rendu et les flux — §8.5 de
//! `docs/ARCHITECTURE.md`.
//!
//! Hors du patch : un réseau change d'une salle à l'autre. Du texte, une ligne
//! par réglage, parce que seule `patch` connaît le JSON. Comme pour le dernier
//! fichier ouvert, rien ici ne doit empêcher un lancement : une ligne illisible
//! est ignorée.

use frogenx_output::StreamFormat;
use glam::UVec2;
use std::fs;
use std::path::Path;

/// Le fichier, dans le dossier de configuration (§23.8).
const FILE_NAME: &str = "sorties.txt";
/// La résolution de rendu par défaut.
pub const DEFAULT_RENDER: UVec2 = UVec2::new(1920, 1080);
/// La cadence du moteur par défaut, en images par seconde.
pub const DEFAULT_RATE: u32 = 60;
/// Au-delà, une cadence ne peut être qu'une ligne abîmée.
const MAX_RATE: u32 = 240;

/// Un flux tel qu'il est retenu : il revient fermé (§8.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamConfig {
    /// Le nom du sender Spout : KyberFrog le suit par ce nom.
    pub name: String,
    pub format: StreamFormat,
}

/// Ce qui appartient à la machine plutôt qu'au patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineSettings {
    pub render: UVec2,
    pub rate: u32,
    pub streams: Vec<StreamConfig>,
}

impl Default for MachineSettings {
    fn default() -> Self {
        Self {
            render: DEFAULT_RENDER,
            rate: DEFAULT_RATE,
            streams: Vec::new(),
        }
    }
}

impl MachineSettings {
    pub fn parse(texte: &str) -> Self {
        let mut reglages = Self::default();
        for ligne in texte.lines() {
            let mut mots = ligne.split_whitespace();
            let nombre = |mot: Option<&str>| mot.and_then(|m| m.parse::<u32>().ok());
            match mots.next() {
                Some("rendu") => {
                    if let (Some(x), Some(y)) = (nombre(mots.next()), nombre(mots.next())) {
                        reglages.render = UVec2::new(x, y);
                    }
                }
                Some("cadence") => {
                    if let Some(rate) = nombre(mots.next()).filter(|r| (1..=MAX_RATE).contains(r)) {
                        reglages.rate = rate;
                    }
                }
                Some("flux") => {
                    let format = (|| {
                        Some(StreamFormat {
                            width: nombre(mots.next()).filter(|w| *w > 0)?,
                            height: nombre(mots.next()).filter(|h| *h > 0)?,
                            fps: nombre(mots.next()).filter(|f| (1..=MAX_RATE).contains(f))?,
                        })
                    })();
                    let name = mots.collect::<Vec<_>>().join(" ");
                    if let (Some(format), false) = (format, name.is_empty()) {
                        reglages.streams.push(StreamConfig { name, format });
                    }
                }
                _ => {}
            }
        }
        reglages
    }

    pub fn format(&self) -> String {
        let mut texte = format!(
            "rendu {} {}\ncadence {}\n",
            self.render.x, self.render.y, self.rate
        );
        for flux in &self.streams {
            let f = flux.format;
            texte += &format!("flux {} {} {} {}\n", f.width, f.height, f.fps, flux.name);
        }
        texte
    }

    pub fn load(dir: &Path) -> Self {
        fs::read_to_string(dir.join(FILE_NAME))
            .map(|texte| Self::parse(&texte))
            .unwrap_or_default()
    }

    /// Rend `false` si ça n'a pas pu se faire : la performance n'en dépend pas.
    pub fn save(&self, dir: &Path) -> bool {
        fs::create_dir_all(dir).is_ok() && fs::write(dir.join(FILE_NAME), self.format()).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exemple() -> MachineSettings {
        MachineSettings {
            render: UVec2::new(3840, 2160),
            rate: 50,
            streams: vec![
                StreamConfig {
                    name: "FrogenX 1".into(),
                    format: StreamFormat {
                        width: 1280,
                        height: 720,
                        fps: 30,
                    },
                },
                StreamConfig {
                    name: "Scène   gauche".into(),
                    format: StreamFormat {
                        width: 640,
                        height: 360,
                        fps: 25,
                    },
                },
            ],
        }
    }

    #[test]
    fn les_reglages_se_relisent_a_l_identique() {
        let mut attendu = exemple();
        attendu.streams[1].name = "Scène gauche".into();
        assert_eq!(MachineSettings::parse(&exemple().format()), attendu);
    }

    #[test]
    fn un_fichier_vide_rend_les_valeurs_par_defaut() {
        assert_eq!(MachineSettings::parse(""), MachineSettings::default());
    }

    #[test]
    fn une_ligne_illisible_n_empeche_rien() {
        let texte = "rendu x 1080\ncadence 0\nflux 1280 720 0 Mauvais\nflux 1280 720 30\n\
                     n'importe quoi\ncadence 30\nflux 640 360 30 Bon\n";
        let lu = MachineSettings::parse(texte);
        assert_eq!(lu.render, DEFAULT_RENDER);
        assert_eq!(lu.rate, 30);
        assert_eq!(lu.streams.len(), 1);
        assert_eq!(lu.streams[0].name, "Bon");
    }

    #[test]
    fn les_reglages_passent_par_le_disque() {
        let dir = std::env::temp_dir().join(format!("frogenx-reglages-{}", std::process::id()));
        let mut attendu = exemple();
        attendu.streams[1].name = "Scène gauche".into();
        assert!(attendu.save(&dir));
        assert_eq!(MachineSettings::load(&dir), attendu);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn un_dossier_absent_rend_les_valeurs_par_defaut() {
        let dir = Path::new("Z:/frogenx/nulle/part");
        assert_eq!(MachineSettings::load(dir), MachineSettings::default());
    }
}
