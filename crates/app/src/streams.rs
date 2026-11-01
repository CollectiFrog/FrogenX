//! Les flux de la machine — §8.5 de `docs/ARCHITECTURE.md`.
//!
//! La liste vit ici, ouverte ou non. `Engine` ne connaît que ceux qui tournent,
//! et l'interface ne reçoit que des mots.

use crate::settings::StreamConfig;
use crate::Engine;
use frogenx_output::{CpuSink, CpuSinkId, SinkStatus, StreamFormat};
use frogenx_ui::StreamView;

/// Ouvre la sortie d'un flux : son nom, sa taille réelle.
pub type Opener<'a> = dyn FnMut(&str, (u32, u32)) -> Result<Box<dyn CpuSink>, String> + 'a;

struct Row {
    config: StreamConfig,
    /// `None` : fermé.
    id: Option<CpuSinkId>,
    /// Le dernier refus d'ouverture, jusqu'au prochain essai.
    error: Option<String>,
}

/// Les flux connus, dans l'ordre du menu.
pub struct Streams {
    rows: Vec<Row>,
}

/// Ce qu'on lit d'un flux dans le menu.
fn describe(ouvert: bool, status: Option<SinkStatus>, error: Option<&str>) -> String {
    if let Some(e) = error {
        return e.to_string();
    }
    match (ouvert, status) {
        (false, _) | (_, None) => "fermé".into(),
        (true, Some(SinkStatus::Active)) => "actif".into(),
        (true, Some(SinkStatus::Degraded { dropped })) => format!("{dropped} images perdues"),
        (true, Some(SinkStatus::Failed(e))) => e,
    }
}

impl Streams {
    /// Les flux enregistrés reviennent fermés : rien ne part au lancement (§8.5).
    pub fn new(configs: Vec<StreamConfig>) -> Self {
        let rows = configs.into_iter().map(|config| Row {
            config,
            id: None,
            error: None,
        });
        Self {
            rows: rows.collect(),
        }
    }

    pub fn configs(&self) -> Vec<StreamConfig> {
        self.rows.iter().map(|r| r.config.clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn views(&self, engine: &Engine) -> Vec<StreamView> {
        self.rows
            .iter()
            .map(|r| {
                let f = r.config.format;
                let status = r.id.and_then(|id| engine.stream_status(id));
                StreamView {
                    name: r.config.name.clone(),
                    size: (f.width, f.height),
                    fps: f.fps,
                    open: r.id.is_some(),
                    status: describe(r.id.is_some(), status, r.error.as_deref()),
                }
            })
            .collect()
    }

    /// Ajoute un flux fermé, sous un nom libre. Rend son rang.
    pub fn add(&mut self, format: StreamFormat) -> usize {
        let name = (1..)
            .map(|n| format!("FrogenX {n}"))
            .find(|nom| self.rows.iter().all(|r| r.config.name != *nom));
        self.add_named(name.unwrap_or_default(), format)
    }

    pub fn add_named(&mut self, name: String, format: StreamFormat) -> usize {
        self.rows.push(Row {
            config: StreamConfig { name, format },
            id: None,
            error: None,
        });
        self.rows.len() - 1
    }

    /// Ouvre un flux fermé, ferme un flux ouvert.
    pub fn toggle(&mut self, i: usize, engine: &mut Engine, open: &mut Opener<'_>) {
        let Some(row) = self.rows.get_mut(i) else {
            return;
        };
        if let Some(id) = row.id.take() {
            engine.remove_stream(id);
            return;
        }
        let rendu = engine.render_size();
        let taille = row.config.format.within((rendu.x, rendu.y));
        match open(&row.config.name, taille) {
            Ok(sink) => {
                row.id = Some(engine.add_stream(sink, row.config.format));
                row.error = None;
            }
            Err(e) => row.error = Some(e),
        }
    }

    /// Change le format d'un flux, ouvert ou non.
    pub fn set_format(&mut self, i: usize, format: StreamFormat, engine: &mut Engine) {
        let Some(row) = self.rows.get_mut(i) else {
            return;
        };
        row.config.format = format;
        if let Some(id) = row.id {
            engine.set_stream_format(id, format);
        }
    }

    pub fn format(&self, i: usize) -> Option<StreamFormat> {
        Some(self.rows.get(i)?.config.format)
    }

    /// Ferme un flux et l'oublie.
    pub fn remove(&mut self, i: usize, engine: &mut Engine) {
        if i >= self.rows.len() {
            return;
        }
        if let Some(id) = self.rows.remove(i).id {
            engine.remove_stream(id);
        }
    }
}

/// Publie un sender Spout sur l'adaptateur du moteur (§8.5).
#[cfg(windows)]
pub fn open_spout(
    info: &wgpu::AdapterInfo,
    name: &str,
    size: (u32, u32),
) -> Result<Box<dyn CpuSink>, String> {
    frogenx_output::spout::SpoutSink::new(name, size, info.vendor, info.device)
        .map(|sink| Box::new(sink) as Box<dyn CpuSink>)
        .map_err(|e| e.to_string())
}

/// Spout n'existe que sous Windows : le menu ne propose rien ailleurs (§8.5).
#[cfg(not(windows))]
pub fn open_spout(
    _: &wgpu::AdapterInfo,
    _: &str,
    _: (u32, u32),
) -> Result<Box<dyn CpuSink>, String> {
    Err("Spout n'existe que sous Windows".into())
}

/// Vrai là où un flux peut s'ouvrir.
pub const STREAMS_AVAILABLE: bool = cfg!(windows);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_flux_ferme_se_dit_ferme() {
        assert_eq!(describe(false, None, None), "fermé");
    }

    #[test]
    fn un_flux_ouvert_dit_son_etat() {
        assert_eq!(describe(true, Some(SinkStatus::Active), None), "actif");
        assert_eq!(
            describe(true, Some(SinkStatus::Degraded { dropped: 3 }), None),
            "3 images perdues"
        );
        assert_eq!(
            describe(true, Some(SinkStatus::Failed("carte perdue".into())), None),
            "carte perdue"
        );
    }

    #[test]
    fn un_refus_d_ouverture_passe_avant_tout() {
        assert_eq!(describe(false, None, Some("nom pris")), "nom pris");
    }

    fn format() -> StreamFormat {
        StreamFormat {
            width: 1280,
            height: 720,
            fps: 60,
        }
    }

    #[test]
    fn un_nouveau_flux_prend_le_premier_nom_libre() {
        let mut flux = Streams::new(Vec::new());
        flux.add(format());
        flux.add(format());
        assert_eq!(flux.configs()[1].name, "FrogenX 2");
    }

    #[test]
    fn un_nom_libere_resert() {
        let mut flux = Streams::new(vec![StreamConfig {
            name: "FrogenX 2".into(),
            format: format(),
        }]);
        flux.add(format());
        assert_eq!(flux.configs()[1].name, "FrogenX 1");
    }
}
