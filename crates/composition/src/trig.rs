//! Le trig : une zone du spectre qui pilote la grille — §3bis.6 de
//! `docs/ARCHITECTURE.md`.
//!
//! Un `Trig` appartient à la composition, comme le tempo : il survit aux
//! changements de clip et de deck. Ses trois nœuds vivent dans le graphe de
//! composition, où tout `Param` de la régie peut les lire :
//!
//! ```text
//! gate (SpectrumZone) ─▶ pulse (Divider, 1 sur N) ─▶ flash (Envelope)
//!       porte 0/1              une frame à 1             retombe seule
//! ```

use crate::command::Command;
use crate::hierarchy::Composition;
use crate::ids::{ClipId, ColumnId, LayerId, TrigId};
use frogenx_core::{Graph, GraphError, NodeId, Param, TrigValues};
use frogenx_osc::{Divider, Envelope, SpectrumZone};

/// Constante de temps de la retombée de `flash`, en secondes.
const FLASH_RELEASE: f32 = 0.2;
/// La zone d'un nouveau trig : les grosses caisses d'un signal bien calé.
const DEFAULT_FREQ: (f32, f32) = (30.0, 120.0);
const DEFAULT_LEVEL: (f32, f32) = (0.7, 1.0);

/// Ce qu'un trig déclenche à chaque passage du compteur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrigTarget {
    /// Rien : son signal reste lisible par les `Param` de la régie.
    #[default]
    None,
    Clip {
        layer: LayerId,
        clip: ClipId,
    },
    Column(ColumnId),
}

/// Désigne un réglage d'un trig, comme `ParamSlot` pour un clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrigSlot {
    FreqMin,
    FreqMax,
    LevelMin,
    LevelMax,
    Hysteresis,
    Every,
}

/// Une zone du spectre, son compteur et sa cible.
///
/// **`zone` et `divider` sont la version de référence, le graphe en a une
/// copie :** un `Signal` est opaque une fois dans le graphe, donc on règle ici
/// et on remplace le nœud (`Graph::replace`), ce qui garde son état et donc
/// le compte en cours.
pub struct Trig {
    pub name: String,
    pub target: TrigTarget,
    /// Comment un clip le désigne : ni un identifiant, qui ne survit pas à un
    /// fichier, ni une position, qui change avec le retrait d'un autre (§3bis.6).
    channel: u32,
    zone: SpectrumZone,
    divider: Divider,
    gate: NodeId,
    pulse: NodeId,
    flash: NodeId,
}

/// Ce qu'il faut pour rattacher un trig chargé d'un fichier.
pub struct RestoredTrig {
    pub name: String,
    pub target: TrigTarget,
    pub channel: u32,
    pub zone: SpectrumZone,
    pub divider: Divider,
    pub gate: NodeId,
    pub pulse: NodeId,
    pub flash: NodeId,
}

impl Trig {
    fn wire(
        graph: &mut Graph,
        name: String,
        channel: u32,
        zone: SpectrumZone,
        every: f32,
    ) -> Result<Self, GraphError> {
        let gate = graph.add_with_deps(Box::new(zone), zone.sources())?;
        let divider = Divider::new(Param::modulated(0.0, 1.0, gate), every);
        let pulse = graph.add_with_deps(Box::new(divider), vec![gate])?;
        let flash = graph.add_with_deps(
            Box::new(Envelope::new(
                Param::modulated(0.0, 1.0, pulse),
                0.0,
                FLASH_RELEASE,
            )),
            vec![pulse],
        )?;
        Ok(Self {
            name,
            target: TrigTarget::None,
            channel,
            zone,
            divider,
            gate,
            pulse,
            flash,
        })
    }

    pub fn zone(&self) -> &SpectrumZone {
        &self.zone
    }

    /// Son canal dans `Ctx::trigs` (§3bis.6).
    pub fn channel(&self) -> u32 {
        self.channel
    }

    /// Le `N` de « 1 sur N ».
    pub fn every(&self) -> Param {
        self.divider.every
    }

    pub fn divider(&self) -> &Divider {
        &self.divider
    }

    /// `1` tant que le pic est dans la zone, compteur ou pas.
    pub fn gate(&self) -> NodeId {
        self.gate
    }

    /// `1` pendant une frame, une fois sur `every`.
    pub fn pulse(&self) -> NodeId {
        self.pulse
    }

    /// `pulse` qui retombe seule : ce qu'on branche sur une opacité.
    pub fn flash(&self) -> NodeId {
        self.flash
    }

    /// Les fronts comptés depuis le dernier passage.
    pub fn count(&self, graph: &Graph) -> u32 {
        Divider::count_in(graph.node_state(self.pulse))
    }

    fn set(&mut self, graph: &mut Graph, slot: TrigSlot, value: f32) -> bool {
        let param = match slot {
            TrigSlot::FreqMin => &mut self.zone.freq_min,
            TrigSlot::FreqMax => &mut self.zone.freq_max,
            TrigSlot::LevelMin => &mut self.zone.level_min,
            TrigSlot::LevelMax => &mut self.zone.level_max,
            TrigSlot::Hysteresis => &mut self.zone.hysteresis,
            TrigSlot::Every => &mut self.divider.every,
        };
        param.set_base(value);
        graph.replace(self.gate, Box::new(self.zone)).is_ok()
            && graph.replace(self.pulse, Box::new(self.divider)).is_ok()
    }

    fn command(&self) -> Option<Command> {
        match self.target {
            TrigTarget::None => None,
            TrigTarget::Clip { layer, clip } => Some(Command::TriggerClip { layer, clip }),
            TrigTarget::Column(column) => Some(Command::TriggerColumn { column }),
        }
    }
}

impl Composition {
    /// Ajoute un trig, sans cible. `None` si `zone` module sur un nœud périmé.
    pub fn add_trig(&mut self, zone: SpectrumZone, every: f32) -> Option<TrigId> {
        let nom = format!("Trig {}", self.trig_order.len() + 1);
        let canal = self.next_trig_channel;
        let trig = Trig::wire(&mut self.graph, nom, canal, zone, every).ok()?;
        self.next_trig_channel += 1;
        let (i, g) = self.trig_arena.insert(trig);
        let id = TrigId::new(i, g);
        self.trig_order.push(id);
        Some(id)
    }

    /// Rattache un trig dont les nœuds sont déjà dans le graphe de composition :
    /// le chargement d'un fichier (§19.1). `zone` et `divider` doivent être les
    /// copies de référence des nœuds `gate` et `pulse`.
    pub fn restore_trig(&mut self, trig: RestoredTrig) -> TrigId {
        self.next_trig_channel = self.next_trig_channel.max(trig.channel.saturating_add(1));
        let (i, g) = self.trig_arena.insert(Trig {
            name: trig.name,
            target: trig.target,
            channel: trig.channel,
            zone: trig.zone,
            divider: trig.divider,
            gate: trig.gate,
            pulse: trig.pulse,
            flash: trig.flash,
        });
        let id = TrigId::new(i, g);
        self.trig_order.push(id);
        id
    }

    /// Un trig de grosse caisse, à chaque coup.
    pub fn add_default_trig(&mut self) -> Option<TrigId> {
        self.add_trig(SpectrumZone::new(DEFAULT_FREQ, DEFAULT_LEVEL), 1.0)
    }

    /// Les trigs, dans leur ordre d'ajout.
    pub fn trigs(&self) -> &[TrigId] {
        &self.trig_order
    }

    pub fn trig(&self, id: TrigId) -> Option<&Trig> {
        self.trig_arena.get(id.index(), id.generation())
    }

    pub fn trig_mut(&mut self, id: TrigId) -> Option<&mut Trig> {
        self.trig_arena.get_mut(id.index(), id.generation())
    }

    pub fn remove_trig(&mut self, id: TrigId) -> bool {
        let Some(trig) = self.trig_arena.remove(id.index(), id.generation()) else {
            return false;
        };
        self.trig_order.retain(|t| *t != id);
        // De l'aval vers l'amont. Un nœud qu'un autre lit encore reste dans le
        // graphe (§18.2) : retirer le trig ne doit pas casser ce câble-là.
        for node in [trig.flash, trig.pulse, trig.gate] {
            let _ = self.graph.remove(node);
        }
        true
    }

    /// Règle un trig. Refusé si la valeur n'est pas finie.
    pub fn set_trig_param(&mut self, id: TrigId, slot: TrigSlot, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        let Some(trig) = self.trig_arena.get_mut(id.index(), id.generation()) else {
            return false;
        };
        trig.set(&mut self.graph, slot, value)
    }

    /// Repart au début du compte : le prochain front passe. C'est le geste de
    /// recalage sur le temps fort.
    pub fn reset_trig_count(&mut self, id: TrigId) -> bool {
        let Some(pulse) = self.trig(id).map(|t| t.pulse) else {
            return false;
        };
        self.graph.reset_node(pulse).is_ok()
    }

    /// Les commandes des trigs qui viennent de passer, à lire **après**
    /// `eval_frame` du graphe de composition. Écrit dans `out` pour ne pas
    /// allouer à chaque frame.
    /// La valeur de chaque trig, indexée par son canal, pour les graphes des
    /// clips (§3bis.6). Se lit après l'évaluation du graphe de composition.
    pub fn trig_values(&self, out: &mut Vec<TrigValues>) {
        out.clear();
        for trig in self.trig_order.iter().filter_map(|id| self.trig(*id)) {
            let canal = trig.channel as usize;
            if out.len() <= canal {
                out.resize(canal + 1, TrigValues::default());
            }
            out[canal] = TrigValues {
                gate: self.graph.value(trig.gate),
                pulse: self.graph.value(trig.pulse),
                flash: self.graph.value(trig.flash),
            };
        }
    }

    pub fn trig_commands(&self, out: &mut Vec<Command>) {
        for id in &self.trig_order {
            let Some(trig) = self.trig(*id) else { continue };
            if self.graph.value(trig.pulse) > 0.5 {
                out.extend(trig.command());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Clip;
    use crate::transport::{PlayMode, Transport};
    use frogenx_core::{AudioFeatures, Spectrum};
    use frogenx_shape::Ellipse;

    #[test]
    fn chaque_trig_a_son_canal_et_un_canal_ne_resert_jamais() {
        let mut c = Composition::new();
        let a = c.add_default_trig().unwrap();
        let b = c.add_default_trig().unwrap();
        assert_eq!(c.trig(a).unwrap().channel(), 0);
        assert_eq!(c.trig(b).unwrap().channel(), 1);
        c.remove_trig(a);
        let suivant = c.add_default_trig().unwrap();
        assert_eq!(c.trig(suivant).unwrap().channel(), 2);
    }

    #[test]
    fn les_valeurs_suivent_le_canal_meme_apres_un_retrait() {
        let mut c = Composition::new();
        let a = c.add_default_trig().unwrap();
        c.add_default_trig().unwrap();
        c.remove_trig(a);
        c.graph_mut().set_audio(plat(0.9));
        c.graph_mut().eval_frame(0.0, 1.0 / 60.0);
        let mut valeurs = vec![TrigValues::default(); 9];
        c.trig_values(&mut valeurs);
        assert_eq!(valeurs.len(), 2, "indexé par canal, pas par rang");
        assert_eq!(valeurs[0], TrigValues::default());
        assert_eq!(valeurs[1].gate, 1.0);
    }

    #[test]
    fn un_trig_restaure_repousse_le_prochain_canal() {
        let mut c = Composition::new();
        let id = c.add_default_trig().unwrap();
        let (gate, pulse, flash) = {
            let t = c.trig(id).unwrap();
            (t.gate(), t.pulse(), t.flash())
        };
        let (zone, divider) = {
            let t = c.trig(id).unwrap();
            (*t.zone(), *t.divider())
        };
        c.restore_trig(RestoredTrig {
            name: "Lu".into(),
            target: TrigTarget::None,
            channel: 5,
            zone,
            divider,
            gate,
            pulse,
            flash,
        });
        let suivant = c.add_default_trig().unwrap();
        assert_eq!(c.trig(suivant).unwrap().channel(), 6);
    }

    fn plat(niveau: f32) -> AudioFeatures {
        AudioFeatures {
            spectrum: Spectrum::flat(niveau),
            ..Default::default()
        }
    }

    /// Une frame du graphe de composition avec ce niveau de spectre.
    fn frame(c: &mut Composition, niveau: f32) -> Vec<Command> {
        c.graph_mut().set_audio(plat(niveau));
        c.graph_mut().eval_frame(0.0, 1.0 / 60.0);
        let mut commandes = Vec::new();
        c.trig_commands(&mut commandes);
        commandes
    }

    /// Un coup : monte dans la zone, puis retombe.
    fn coup(c: &mut Composition) -> Vec<Command> {
        let commandes = frame(c, 0.9);
        frame(c, 0.0);
        commandes
    }

    fn grille() -> (Composition, LayerId, ClipId, ColumnId) {
        let mut c = Composition::new();
        let deck = c.add_deck("test");
        let groupe = c.add_layer_group(deck).unwrap();
        let piste = c.add_layer(groupe).unwrap();
        let colonne = c.add_column();
        let mut g = Graph::new();
        let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
        let clip = c
            .place_clip(
                piste,
                colonne,
                Clip::shape(g, Box::new(Ellipse::circle(0.5)), transport),
            )
            .unwrap();
        (c, piste, clip, colonne)
    }

    #[test]
    fn un_trig_sans_cible_ne_commande_rien_mais_son_signal_vit() {
        let (mut c, ..) = grille();
        let id = c.add_default_trig().unwrap();
        let pulse = c.trig(id).unwrap().pulse();
        assert!(frame(&mut c, 0.9).is_empty());
        assert_eq!(c.graph().value(pulse), 1.0);
    }

    #[test]
    fn un_trig_declenche_sa_colonne_a_chaque_coup() {
        let (mut c, _, _, colonne) = grille();
        let id = c.add_default_trig().unwrap();
        c.trig_mut(id).unwrap().target = TrigTarget::Column(colonne);
        let attendu = vec![Command::TriggerColumn { column: colonne }];
        assert_eq!(coup(&mut c), attendu);
        assert_eq!(coup(&mut c), attendu);
    }

    #[test]
    fn un_trig_peut_viser_un_clip() {
        let (mut c, layer, clip, _) = grille();
        let id = c.add_default_trig().unwrap();
        c.trig_mut(id).unwrap().target = TrigTarget::Clip { layer, clip };
        assert_eq!(coup(&mut c), vec![Command::TriggerClip { layer, clip }]);
    }

    #[test]
    fn le_compteur_ne_laisse_passer_qu_un_coup_sur_quatre() {
        let (mut c, _, _, colonne) = grille();
        let id = c.add_default_trig().unwrap();
        c.trig_mut(id).unwrap().target = TrigTarget::Column(colonne);
        assert!(c.set_trig_param(id, TrigSlot::Every, 4.0));

        let passages: Vec<bool> = (0..9).map(|_| !coup(&mut c).is_empty()).collect();
        assert_eq!(
            passages,
            [true, false, false, false, true, false, false, false, true]
        );
    }

    #[test]
    fn la_porte_suit_tous_les_coups_meme_ceux_que_le_compteur_retient() {
        let (mut c, ..) = grille();
        let id = c.add_default_trig().unwrap();
        c.set_trig_param(id, TrigSlot::Every, 8.0);
        coup(&mut c);
        frame(&mut c, 0.9);
        let t = c.trig(id).unwrap();
        assert_eq!(c.graph().value(t.gate()), 1.0);
        assert_eq!(c.graph().value(t.pulse()), 0.0, "le compteur le retient");
    }

    #[test]
    fn regler_la_zone_prend_effet_sans_perdre_le_compte() {
        let (mut c, ..) = grille();
        let id = c.add_default_trig().unwrap();
        c.set_trig_param(id, TrigSlot::Every, 4.0);
        coup(&mut c);
        assert_eq!(c.trig(id).unwrap().count(c.graph()), 1);

        assert!(c.set_trig_param(id, TrigSlot::LevelMin, 0.95));
        frame(&mut c, 0.9);
        assert_eq!(c.trig(id).unwrap().count(c.graph()), 1, "sous la zone");
        frame(&mut c, 0.0);
        frame(&mut c, 0.97);
        assert_eq!(
            c.trig(id).unwrap().count(c.graph()),
            2,
            "le compte continue"
        );
    }

    #[test]
    fn un_reglage_porte_sur_le_bon_champ() {
        let (mut c, ..) = grille();
        let id = c.add_default_trig().unwrap();
        for (slot, valeur) in [
            (TrigSlot::FreqMin, 40.0),
            (TrigSlot::FreqMax, 90.0),
            (TrigSlot::LevelMin, 0.2),
            (TrigSlot::LevelMax, 0.6),
            (TrigSlot::Hysteresis, 0.1),
        ] {
            assert!(c.set_trig_param(id, slot, valeur));
        }
        let z = *c.trig(id).unwrap().zone();
        let lus = [
            z.freq_min,
            z.freq_max,
            z.level_min,
            z.level_max,
            z.hysteresis,
        ];
        assert_eq!(lus.map(|p| p.base()), [40.0, 90.0, 0.2, 0.6, 0.1]);
    }

    #[test]
    fn recaler_le_compteur_fait_passer_le_coup_suivant() {
        let (mut c, _, _, colonne) = grille();
        let id = c.add_default_trig().unwrap();
        c.trig_mut(id).unwrap().target = TrigTarget::Column(colonne);
        c.set_trig_param(id, TrigSlot::Every, 8.0);
        coup(&mut c);
        coup(&mut c);
        assert!(coup(&mut c).is_empty());

        assert!(c.reset_trig_count(id));
        assert_eq!(c.trig(id).unwrap().count(c.graph()), 0);
        assert!(!coup(&mut c).is_empty());
    }

    #[test]
    fn un_trig_survit_au_changement_de_clip() {
        let (mut c, piste, clip, colonne) = grille();
        let id = c.add_default_trig().unwrap();
        c.trig_mut(id).unwrap().target = TrigTarget::Column(colonne);
        c.set_trig_param(id, TrigSlot::Every, 2.0);
        coup(&mut c);
        c.trigger(piste, clip);
        assert_eq!(c.trig(id).unwrap().count(c.graph()), 1);
    }

    #[test]
    fn deux_trigs_comptent_chacun_de_leur_cote() {
        let (mut c, _, _, colonne) = grille();
        let (a, b) = (c.add_default_trig().unwrap(), c.add_default_trig().unwrap());
        assert_eq!(c.trigs(), [a, b]);
        assert_eq!(c.trig(b).unwrap().name, "Trig 2");
        c.trig_mut(a).unwrap().target = TrigTarget::Column(colonne);
        c.trig_mut(b).unwrap().target = TrigTarget::Column(colonne);
        c.set_trig_param(b, TrigSlot::Every, 2.0);
        assert_eq!(coup(&mut c).len(), 2);
        assert_eq!(coup(&mut c).len(), 1, "b attend son tour");
    }

    #[test]
    fn retirer_un_trig_le_debranche_du_graphe() {
        let (mut c, ..) = grille();
        let avant = c.graph().len();
        let id = c.add_default_trig().unwrap();
        let pulse = c.trig(id).unwrap().pulse();
        assert_eq!(c.graph().len(), avant + 3);

        assert!(c.remove_trig(id));
        assert_eq!(c.graph().len(), avant);
        assert!(!c.graph().is_live(pulse));
        assert!(c.trigs().is_empty());
        assert!(c.trig(id).is_none());
        assert!(!c.remove_trig(id), "déjà retiré");
    }

    #[test]
    fn un_identifiant_perime_est_refuse_sans_paniquer() {
        let (mut c, ..) = grille();
        let id = c.add_default_trig().unwrap();
        c.remove_trig(id);
        assert!(!c.set_trig_param(id, TrigSlot::Every, 4.0));
        assert!(!c.reset_trig_count(id));
    }

    #[test]
    fn une_valeur_non_finie_est_refusee() {
        let (mut c, ..) = grille();
        let id = c.add_default_trig().unwrap();
        assert!(!c.set_trig_param(id, TrigSlot::LevelMin, f32::NAN));
        assert_eq!(c.trig(id).unwrap().zone().level_min.base(), DEFAULT_LEVEL.0);
    }

    #[test]
    fn le_flash_module_l_opacite_d_une_piste() {
        // Le signal du trig est un Signal : il se branche comme un LFO.
        let (mut c, piste, ..) = grille();
        let id = c.add_default_trig().unwrap();
        let flash = c.trig(id).unwrap().flash();
        c.layer_mut(piste).unwrap().blend.opacity = Param::modulated(0.0, 1.0, flash);
        let opacite = |c: &Composition| c.layer(piste).unwrap().blend.opacity.get(c.graph());

        frame(&mut c, 0.9);
        assert_eq!(opacite(&c), 1.0);
        for _ in 0..60 {
            frame(&mut c, 0.0);
        }
        assert!(opacite(&c) < 0.01, "got {}", opacite(&c));
        assert!(c.graph().diagnostics().is_clean());
    }
}
