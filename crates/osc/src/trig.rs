//! Le trig — §3bis.6 de `docs/ARCHITECTURE.md`.
//!
//! Deux `Signal` qui se branchent en série, et que tout le reste ignore :
//!
//! ```text
//! Ctx.audio.spectrum ─▶ SpectrumZone ─▶ Divider ─▶ n'importe quel Param
//!                        porte 0 / 1     1 sur N
//! ```
//!
//! [`Divider`] compte les fronts de n'importe quelle porte : il ne sait pas
//! que la sienne vient de l'audio.

use frogenx_core::{
    Ctx, NodeState, Param, Signal, Spec, SpecError, SPECTRUM_MAX_HZ, SPECTRUM_MIN_HZ,
};

/// Plus grand diviseur accepté. Saturé chez le consommateur (§18.4).
pub const MAX_DIVISION: f32 = 1024.0;

/// Le pic du spectre vu depuis la zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZoneState {
    /// Sous la zone : prête à se déclencher.
    Below = 0,
    /// Entrée par le bas : la porte est ouverte.
    Open = 1,
    /// Passée au-dessus de la zone sans s'ouvrir : le son n'est pas repassé
    /// dessous, et redescendre dans la zone ne compte pas.
    Over = 2,
}

/// Une zone du plan fréquence × amplitude : une porte qui s'ouvre quand le pic
/// du spectre y **entre par le bas**.
///
/// **Par le bas, et une seule fois.** Une attaque traverse la zone en
/// montant, puis la retraverse en descendant : une porte qui suivrait
/// seulement « le pic est dans la zone » s'ouvrirait deux fois par coup. Elle
/// reste donc ouverte jusqu'à ce que le pic repasse sous `level_min -
/// hysteresis`, même s'il dépasse le haut de la zone entre-temps. Un son qui
/// saute d'un coup au-dessus de `level_max` ne la déclenche pas : c'est ce qui
/// donne un sens au haut de la zone. À `level_max = 1`, c'est un seuil simple.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectrumZone {
    /// Bornes de fréquence, en Hz. Dans n'importe quel ordre.
    pub freq_min: Param,
    pub freq_max: Param,
    /// Bornes d'amplitude, dans `[0, 1]`. Dans n'importe quel ordre.
    pub level_min: Param,
    pub level_max: Param,
    /// De combien le pic doit repasser sous la zone pour la réarmer : sans
    /// cette marge, un son qui tremble sur le bord la déclenche en rafale.
    pub hysteresis: Param,
}

impl SpectrumZone {
    pub fn new(freq: (f32, f32), level: (f32, f32)) -> Self {
        Self {
            freq_min: Param::fixed(freq.0),
            freq_max: Param::fixed(freq.1),
            level_min: Param::fixed(level.0),
            level_max: Param::fixed(level.1),
            hysteresis: Param::fixed(0.05),
        }
    }

    /// Les sources de modulation de la zone : les dépendances de son nœud.
    pub fn sources(&self) -> Vec<frogenx_core::NodeId> {
        [
            self.freq_min,
            self.freq_max,
            self.level_min,
            self.level_max,
            self.hysteresis,
        ]
        .iter()
        .filter_map(Param::source)
        .collect()
    }
}

impl SpectrumZone {
    pub fn from_spec(spec: &Spec) -> Result<Self, SpecError> {
        Ok(Self {
            freq_min: spec.param("freq_min")?,
            freq_max: spec.param("freq_max")?,
            level_min: spec.param("level_min")?,
            level_max: spec.param("level_max")?,
            hysteresis: spec.param("hysteresis")?,
        })
    }
}

impl Signal for SpectrumZone {
    fn spec(&self) -> Spec {
        Spec::new("SpectrumZone")
            .with("freq_min", self.freq_min)
            .with("freq_max", self.freq_max)
            .with("level_min", self.level_min)
            .with("level_max", self.level_max)
            .with("hysteresis", self.hysteresis)
    }

    // slots[0] = ZoneState, en f32 : 0 sous la zone, 1 ouverte, 2 passée dessus
    fn eval(&self, _t: f64, ctx: &Ctx, state: &mut NodeState) -> f32 {
        let f = |p: Param| p.eval(ctx).clamp(SPECTRUM_MIN_HZ, SPECTRUM_MAX_HZ);
        let a = |p: Param| p.eval(ctx).clamp(0.0, 1.0);
        let (l0, l1) = (a(self.level_min), a(self.level_max));
        let (bas, haut) = (l0.min(l1), l0.max(l1));
        let marge = a(self.hysteresis);

        let pic = ctx.audio.spectrum.peak(f(self.freq_min), f(self.freq_max));

        let courant = match state.get(0) as u32 {
            1 => ZoneState::Open,
            2 => ZoneState::Over,
            _ => ZoneState::Below,
        };
        let suivant = match courant {
            ZoneState::Below if pic < bas => ZoneState::Below,
            ZoneState::Below if pic <= haut => ZoneState::Open,
            ZoneState::Below => ZoneState::Over,
            _ if pic < bas - marge => ZoneState::Below,
            encore => encore,
        };
        state.set(0, suivant as u32 as f32);

        if suivant == ZoneState::Open {
            1.0
        } else {
            0.0
        }
    }

    fn state_size(&self) -> usize {
        1
    }
}

/// Laisse passer un front sur `every` : le premier, puis le `every + 1`-ième…
/// Sort `1.0` pendant une frame, `0.0` sinon.
///
/// **Le premier, pas le dernier :** partir du premier coup répond tout de
/// suite, et cale le compte sur le temps fort où on l'a lancé. `every` à 1
/// laisse tout passer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Divider {
    /// La porte dont on compte les fronts montants.
    pub input: Param,
    /// Arrondi à l'entier, et saturé dans `[1, MAX_DIVISION]`.
    pub every: Param,
}

impl Divider {
    pub fn new(input: Param, every: f32) -> Self {
        Self {
            input,
            every: Param::fixed(every),
        }
    }

    /// Le compte de ce diviseur d'après son état : `Graph::node_state`.
    pub fn count_in(state: &[f32]) -> u32 {
        state.get(1).copied().unwrap_or(0.0) as u32
    }
}

impl Divider {
    pub fn from_spec(spec: &Spec) -> Result<Self, SpecError> {
        Ok(Self {
            input: spec.param("input")?,
            every: spec.param("every")?,
        })
    }
}

/// Ce qu'on lit d'un trig.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrigOutput {
    Gate,
    Pulse,
    /// Retombe seule : ce qui se lit le mieux sur un paramètre.
    #[default]
    Flash,
}

impl TrigOutput {
    pub const ALL: [TrigOutput; 3] = [TrigOutput::Flash, TrigOutput::Pulse, TrigOutput::Gate];

    pub fn word(self) -> &'static str {
        match self {
            TrigOutput::Gate => "gate",
            TrigOutput::Pulse => "pulse",
            TrigOutput::Flash => "flash",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.word() == word)
    }

    pub fn label(self) -> &'static str {
        match self {
            TrigOutput::Gate => "porte",
            TrigOutput::Pulse => "impulsion",
            TrigOutput::Flash => "flash",
        }
    }
}

/// Un trig lu depuis un autre graphe que le sien (§3bis.6). Sans état : comme une
/// feature audio, la valeur arrive par le contexte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrigInput {
    /// Le canal du trig : stable, et jamais réutilisé.
    pub channel: u32,
    pub output: TrigOutput,
}

impl TrigInput {
    pub fn new(channel: u32) -> Self {
        Self {
            channel,
            output: TrigOutput::default(),
        }
    }

    pub fn from_spec(spec: &Spec) -> Result<Self, SpecError> {
        Ok(Self {
            channel: spec.int("channel")?,
            output: TrigOutput::from_word(spec.word("output")?)
                .ok_or_else(|| spec.field_error("output"))?,
        })
    }
}

impl Signal for TrigInput {
    fn spec(&self) -> Spec {
        Spec::new("TrigInput")
            .with("channel", self.channel)
            .with("output", self.output.word())
    }

    fn eval(&self, _t: f64, ctx: &Ctx, _state: &mut NodeState) -> f32 {
        // Un trig retiré laisse son canal vide : zéro, jamais une panne (§18).
        let valeurs = ctx.trigs.get(self.channel as usize).copied();
        let valeurs = valeurs.unwrap_or_default();
        match self.output {
            TrigOutput::Gate => valeurs.gate,
            TrigOutput::Pulse => valeurs.pulse,
            TrigOutput::Flash => valeurs.flash,
        }
    }
}

impl Signal for Divider {
    fn spec(&self) -> Spec {
        Spec::new("Divider")
            .with("input", self.input)
            .with("every", self.every)
    }

    // slots[0] = 1.0 si l'entrée était haute à la frame précédente
    // slots[1] = fronts comptés depuis le dernier passage, dans [0, every)
    fn eval(&self, _t: f64, ctx: &Ctx, state: &mut NodeState) -> f32 {
        let haut = self.input.eval(ctx) > 0.5;
        let etait_haut = state.get(0) > 0.5;
        state.set(0, if haut { 1.0 } else { 0.0 });
        if !haut || etait_haut {
            return 0.0;
        }

        // `clamp` laisse passer un NaN, et `% NaN` bloquerait le compte.
        let voulu = self.every.eval(ctx).round();
        let every = if voulu.is_nan() {
            1.0
        } else {
            voulu.clamp(1.0, MAX_DIVISION)
        };
        let compte = state.get(1);
        state.set(1, (compte + 1.0) % every);
        if compte == 0.0 {
            1.0
        } else {
            0.0
        }
    }

    fn state_size(&self) -> usize {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Envelope;
    use frogenx_core::{AudioFeatures, Constant, Graph, Spectrum, TrigValues};

    fn lu(signal: TrigInput, trigs: &[TrigValues]) -> f32 {
        let mut g = Graph::new();
        let noeud = g.add(Box::new(signal));
        g.set_trigs(trigs);
        g.eval_frame(0.0, 1.0 / 60.0);
        g.value(noeud)
    }

    #[test]
    fn une_entree_de_trig_lit_son_canal_et_sa_sortie() {
        let trigs = [
            TrigValues::default(),
            TrigValues {
                gate: 1.0,
                pulse: 0.5,
                flash: 0.25,
            },
        ];
        let mut signal = TrigInput::new(1);
        assert_eq!(lu(signal, &trigs), 0.25, "le flash par défaut");
        signal.output = TrigOutput::Pulse;
        assert_eq!(lu(signal, &trigs), 0.5);
        signal.output = TrigOutput::Gate;
        assert_eq!(lu(signal, &trigs), 1.0);
    }

    #[test]
    fn un_canal_sans_trig_vaut_zero() {
        assert_eq!(lu(TrigInput::new(7), &[]), 0.0);
    }

    #[test]
    fn une_sortie_inconnue_est_refusee_a_la_lecture() {
        let spec = Spec::new("TrigInput")
            .with("channel", 0u32)
            .with("output", "fin");
        assert!(TrigInput::from_spec(&spec).is_err());
    }

    /// Un spectre plat à `niveau`.
    fn plat(niveau: f32) -> AudioFeatures {
        AudioFeatures {
            spectrum: Spectrum::flat(niveau),
            ..Default::default()
        }
    }

    /// Fait passer le pic par ces niveaux et rend la porte à chaque frame.
    fn porte(zone: SpectrumZone, niveaux: &[f32]) -> Vec<f32> {
        let mut g = Graph::new();
        let id = g.add(Box::new(zone));
        niveaux
            .iter()
            .map(|n| {
                g.set_audio(plat(*n));
                g.eval_frame(0.0, 1.0 / 60.0);
                g.value(id)
            })
            .collect()
    }

    fn zone() -> SpectrumZone {
        SpectrumZone::new((50.0, 200.0), (0.5, 0.8))
    }

    #[test]
    fn la_porte_s_ouvre_quand_le_pic_entre_dans_la_zone() {
        assert_eq!(
            porte(zone(), &[0.0, 0.3, 0.6, 0.7, 0.3, 0.0]),
            [0.0, 0.0, 1.0, 1.0, 0.0, 0.0]
        );
    }

    #[test]
    fn une_attaque_qui_traverse_la_zone_ne_compte_qu_une_fois() {
        // Monte à travers la zone, la dépasse, redescend à travers elle : la
        // porte ne se referme pas au milieu, donc un seul front.
        assert_eq!(
            porte(zone(), &[0.0, 0.6, 0.95, 0.6, 0.0]),
            [0.0, 1.0, 1.0, 1.0, 0.0]
        );
    }

    #[test]
    fn un_son_qui_saute_au_dessus_de_la_zone_ne_la_declenche_pas() {
        // Le haut de la zone a un effet : ni à la montée, ni en redescendant.
        assert_eq!(porte(zone(), &[0.0, 0.95, 0.7, 0.6, 0.0]), [0.0; 5]);
    }

    #[test]
    fn apres_un_saut_au_dessus_la_zone_se_rearme_par_le_bas() {
        assert_eq!(
            porte(zone(), &[0.95, 0.2, 0.6]),
            [0.0, 0.0, 1.0],
            "repassée dessous, elle se déclenche à nouveau"
        );
    }

    #[test]
    fn l_hysteresis_empeche_un_son_qui_tremble_de_rafaler() {
        // Oscille juste sous le bas de la zone, sans jamais repasser la marge.
        assert_eq!(
            porte(zone(), &[0.0, 0.51, 0.49, 0.51, 0.47, 0.2]),
            [0.0, 1.0, 1.0, 1.0, 1.0, 0.0]
        );
    }

    #[test]
    fn la_zone_ne_regarde_que_ses_frequences() {
        let mut g = Graph::new();
        let id = g.add(Box::new(zone()));
        let mut audio = AudioFeatures::default();
        // Un pic loin de la zone, à 10 kHz.
        let bande = audio.spectrum.band(10_000.0);
        audio.spectrum.levels[bande] = 0.7;
        g.set_audio(audio);
        g.eval_frame(0.0, 1.0 / 60.0);
        assert_eq!(g.value(id), 0.0);
    }

    #[test]
    fn des_bornes_inversees_decrivent_la_meme_zone() {
        let inversee = SpectrumZone::new((200.0, 50.0), (0.8, 0.5));
        let niveaux = [0.0, 0.6, 0.0];
        assert_eq!(porte(inversee, &niveaux), porte(zone(), &niveaux));
    }

    #[test]
    fn le_silence_reste_inerte() {
        // Sans audio, un patch audio-réactif ne fait rien (§7).
        assert_eq!(porte(zone(), &[0.0; 4]), [0.0; 4]);
    }

    #[test]
    fn une_zone_peut_etre_modulee() {
        // Le bas de la zone suit un LFO : c'est un Param comme un autre.
        let mut g = Graph::new();
        let haut = g.add(Box::new(Constant(1.0)));
        let z = SpectrumZone {
            level_min: Param::modulated(0.2, 0.5, haut),
            ..zone()
        };
        assert_eq!(z.sources(), vec![haut]);
        let id = g.add_with_deps(Box::new(z), z.sources()).unwrap();
        g.set_audio(plat(0.6));
        g.eval_frame(0.0, 1.0 / 60.0);
        assert_eq!(g.value(id), 0.0, "le bas modulé est à 0.7");
        g.set_audio(plat(0.75));
        g.eval_frame(0.0, 1.0 / 60.0);
        assert_eq!(g.value(id), 1.0);
    }

    /// Une porte qui suit `Constant` ne bouge pas : on rejoue donc l'état.
    fn passages(every: f32, fronts: usize) -> Vec<usize> {
        let divider = Divider::new(Param::fixed(1.0), every);
        let mut slots = [0.0f32; 2];
        let audio = AudioFeatures::default();
        let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
        let mut sorties = Vec::new();
        for front in 1..=fronts {
            let mut st = NodeState::new(&mut slots);
            if divider.eval(0.0, &ctx, &mut st) > 0.5 {
                sorties.push(front);
            }
            st.set(0, 0.0); // l'entrée retombe entre deux fronts
        }
        sorties
    }

    #[test]
    fn le_diviseur_laisse_passer_un_front_sur_n() {
        assert_eq!(passages(1.0, 4), [1, 2, 3, 4]);
        assert_eq!(passages(4.0, 12), [1, 5, 9]);
        assert_eq!(passages(8.0, 17), [1, 9, 17]);
        assert_eq!(passages(16.0, 33), [1, 17, 33]);
        assert_eq!(passages(32.0, 65), [1, 33, 65]);
    }

    #[test]
    fn un_diviseur_sature_ne_bloque_jamais() {
        assert_eq!(passages(0.0, 3), [1, 2, 3], "0 vaut 1");
        assert_eq!(passages(-4.0, 3), [1, 2, 3]);
        assert_eq!(passages(f32::NAN, 3), [1, 2, 3]);
        assert_eq!(passages(4.4, 5), [1, 5], "arrondi à 4");
        assert_eq!(passages(1e9, 2), [1], "saturé, sans perdre le compte");
    }

    #[test]
    fn une_porte_maintenue_ne_compte_qu_une_fois() {
        let mut g = Graph::new();
        let haute = g.add(Box::new(Constant(1.0)));
        let d = g
            .add_with_deps(
                Box::new(Divider::new(Param::modulated(0.0, 1.0, haute), 1.0)),
                vec![haute],
            )
            .unwrap();
        let mut sorties = 0;
        for _ in 0..10 {
            g.eval_frame(0.0, 1.0 / 60.0);
            sorties += (g.value(d) > 0.5) as u32;
        }
        assert_eq!(sorties, 1);
    }

    #[test]
    fn reduire_le_diviseur_en_cours_de_compte_ne_le_bloque_pas() {
        let audio = AudioFeatures::default();
        let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
        let mut slots = [0.0, 7.0]; // compte laissé par un diviseur par 8
        let d = Divider::new(Param::fixed(1.0), 2.0);
        let mut passe = false;
        for _ in 0..3 {
            let mut st = NodeState::new(&mut slots);
            passe |= d.eval(0.0, &ctx, &mut st) > 0.5;
            st.set(0, 0.0);
        }
        assert!(passe, "le compte revient dans [0, every)");
    }

    #[test]
    fn le_compte_se_lit_dans_l_etat_du_noeud() {
        assert_eq!(Divider::count_in(&[0.0, 3.0]), 3);
        assert_eq!(Divider::count_in(&[]), 0, "nœud périmé");
    }

    #[test]
    fn la_chaine_complete_module_un_param_une_fois_sur_deux() {
        // Le câblage du §3bis.6 : zone → diviseur → enveloppe → Param.
        let mut g = Graph::new();
        let z = g.add(Box::new(zone()));
        let d = g
            .add_with_deps(
                Box::new(Divider::new(Param::modulated(0.0, 1.0, z), 2.0)),
                vec![z],
            )
            .unwrap();
        let flash = g
            .add_with_deps(
                Box::new(Envelope::new(Param::modulated(0.0, 1.0, d), 0.0, 0.1)),
                vec![d],
            )
            .unwrap();
        let rayon = Param::modulated(0.5, 0.5, flash);

        let mut pics = Vec::new();
        for coup in 0..4 {
            g.set_audio(plat(0.6));
            g.eval_frame(coup as f64, 1.0 / 60.0);
            pics.push(rayon.get(&g) > 0.9);
            g.set_audio(plat(0.0));
            for _ in 0..60 {
                g.eval_frame(coup as f64, 1.0 / 60.0);
            }
        }
        assert_eq!(pics, [true, false, true, false]);
        assert!(g.diagnostics().is_clean());
    }
}
