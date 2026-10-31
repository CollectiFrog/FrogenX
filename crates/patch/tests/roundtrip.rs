//! L'aller-retour complet : une grille, son texte, une grille neuve — §19.1.
//!
//! Comparer les structures ne prouverait rien : deux clips sont égaux s'ils
//! produisent la même forme, frame après frame.

use frogenx_composition::{
    AudioAnalysis, Clip, ClipSource, ClipTransition, Composition, FadeCurve, LayerId, PlayMode,
    PositionSignal, Transport, TrigTarget, WipeDirection,
};
use frogenx_core::{
    AmpScale, AnalysisMethod, AudioFeatures, Constant, Ctx, FreqScale, Graph, Param,
};
use frogenx_library::{catalog, starter};
use frogenx_patch::{from_json, to_json, PatchError, MAX_COLUMNS};
use frogenx_shape::{BlendMode, ColorParam, Ellipse, Path, ShapeStyle};

const DT: f64 = 1.0 / 60.0;

/// La forme d'un clip, après ses effets.
fn forme(clip: &Clip, t: f64) -> Path {
    let ClipSource::Shape {
        graph, generator, ..
    } = &clip.source;
    let audio = AudioFeatures::default();
    let ctx = Ctx::new(DT, 0, &audio);
    let mut path = Path::new();
    generator.generate(t, &ctx, graph, &mut path);
    for op in &clip.effects {
        op.apply(t, &ctx, graph, &mut path);
    }
    path
}

/// Fait avancer deux clips ensemble : la même forme à chaque frame.
fn memes_formes(a: &mut Clip, b: &mut Clip, nom: &str) {
    for frame in 0..240 {
        let t = frame as f64 * DT;
        a.graph_mut().eval_frame(t, DT);
        b.graph_mut().eval_frame(t, DT);
        let (fa, fb) = (forme(a, t), forme(b, t));
        assert_eq!(fa.points, fb.points, "{nom}, frame {frame}");
        assert_eq!(fa.contours, fb.contours, "{nom}, frame {frame}");
    }
}

/// Une grille d'une piste et d'un clip.
fn une_case(clip: Clip) -> (Composition, LayerId) {
    let mut c = Composition::new();
    let deck = c.add_deck("test");
    let groupe = c.add_layer_group(deck).unwrap();
    let piste = c.add_layer(groupe).unwrap();
    c.add_clip(piste, clip).unwrap();
    (c, piste)
}

fn relu(c: &Composition) -> Composition {
    from_json(&to_json(c).expect("s'écrit")).expect("se relit")
}

/// Un cercle, avec son transport.
fn cercle() -> Clip {
    let mut g = Graph::new();
    let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
    Clip::shape(g, Box::new(Ellipse::circle(0.5)), transport).with_name("Cercle")
}

#[test]
fn chaque_clip_du_catalogue_garde_sa_forme() {
    for preset in catalog() {
        let (original, piste) = une_case(preset.build());
        let mut copie = relu(&original);
        let piste_copie = copie.layers_de_test()[0];
        let id = original.layer(piste).unwrap().cell(0).unwrap();
        let id_copie = copie.layer(piste_copie).unwrap().cell(0).unwrap();

        let mut a = preset.build();
        let b = copie.clip_mut(id_copie).unwrap();
        assert_eq!(b.name, original.clip(id).unwrap().name);
        memes_formes(&mut a, b, preset.name);
        assert!(b.graph().diagnostics().is_clean(), "{}", preset.name);
    }
}

/// Les pistes du deck actif, sans passer par le `Vec` de sortie.
trait LayersDeTest {
    fn layers_de_test(&self) -> Vec<LayerId>;
}

impl LayersDeTest for Composition {
    fn layers_de_test(&self) -> Vec<LayerId> {
        let mut pistes = Vec::new();
        self.layers_in_order(&mut pistes);
        pistes
    }
}

#[test]
fn le_texte_ne_change_pas_d_une_sauvegarde_a_l_autre() {
    // Un fichier se diffe dans git : le réenregistrer ne doit rien y changer.
    let original = to_json(&starter()).unwrap();
    let second = to_json(&from_json(&original).unwrap()).unwrap();
    assert_eq!(original, second);
}

#[test]
fn la_grille_de_depart_garde_sa_structure_et_ses_clips() {
    let original = starter();
    let copie = relu(&original);
    assert_eq!(copie.columns().len(), original.columns().len());
    assert_eq!(copie.bpm(), original.bpm());

    let (a, b) = (original.layers_de_test(), copie.layers_de_test());
    assert_eq!(a.len(), b.len());
    for (pa, pb) in a.iter().zip(&b) {
        let (la, lb) = (original.layer(*pa).unwrap(), copie.layer(*pb).unwrap());
        assert_eq!(la.name, lb.name);
        for i in 0..original.columns().len() {
            let nom = |c: &Composition, id| c.clip(id).map(|clip: &Clip| clip.name.clone());
            assert_eq!(
                la.cell(i).and_then(|id| nom(&original, id)),
                lb.cell(i).and_then(|id| nom(&copie, id)),
                "{}, case {i}",
                la.name
            );
        }
    }
}

#[test]
fn le_clip_actif_revient_et_une_piste_sans_clip_actif_reste_vide() {
    let mut c = Composition::new();
    let deck = c.add_deck("test");
    let groupe = c.add_layer_group(deck).unwrap();
    let (a, b) = (c.add_layer(groupe).unwrap(), c.add_layer(groupe).unwrap());
    c.add_clip(a, cercle()).unwrap();
    let second = c.add_clip(a, cercle().with_name("Second")).unwrap();
    c.add_clip(b, cercle()).unwrap();
    c.trigger(a, second);

    let copie = relu(&c);
    let pistes = copie.layers_de_test();
    let actif = copie.layer(pistes[0]).unwrap().active_clip().unwrap();
    assert_eq!(copie.clip(actif).unwrap().name, "Second");
    assert_eq!(copie.layer(pistes[1]).unwrap().active_clip(), None);
}

#[test]
fn une_case_vide_reste_a_sa_place() {
    let (mut c, piste) = une_case(cercle());
    let colonnes = [c.add_column(), c.add_column()];
    c.place_clip(piste, colonnes[1], cercle()).unwrap();

    let copie = relu(&c);
    let l = copie.layer(copie.layers_de_test()[0]).unwrap();
    assert!(l.cell(0).is_some() && l.cell(1).is_none() && l.cell(2).is_some());
    assert_eq!(copie.columns().len(), 3);
}

#[test]
fn le_melange_de_la_piste_et_sa_modulation_de_regie_reviennent() {
    let (mut c, piste) = une_case(cercle());
    let source = c.graph_mut().add(Box::new(Constant(0.5)));
    let l = c.layer_mut(piste).unwrap();
    l.name = "Ma piste".into();
    l.blend.mode = BlendMode::Screen;
    l.blend.opacity = Param::modulated(0.25, 0.5, source);

    let mut copie = relu(&c);
    copie.graph_mut().eval_frame(0.0, DT);
    let l = copie.layer(copie.layers_de_test()[0]).unwrap();
    assert_eq!(l.name, "Ma piste");
    assert_eq!(l.blend.mode, BlendMode::Screen);
    assert_eq!(l.blend.opacity.get(copie.graph()), 0.5);
}

#[test]
fn la_vitesse_d_un_clip_lit_le_graphe_de_composition() {
    let (mut c, piste) = une_case(cercle());
    let source = c.graph_mut().add(Box::new(Constant(1.0)));
    let id = c.layer(piste).unwrap().cell(0).unwrap();
    c.clip_mut(id).unwrap().speed = Param::modulated(0.5, 1.5, source);

    let mut copie = relu(&c);
    copie.graph_mut().eval_frame(0.0, DT);
    let id = copie
        .layer(copie.layers_de_test()[0])
        .unwrap()
        .cell(0)
        .unwrap();
    assert_eq!(copie.clip(id).unwrap().speed.get(copie.graph()), 2.0);
}

#[test]
fn le_tempo_revient() {
    let mut c = starter();
    c.tempo.tempo = Param::fixed(93.5);
    c.graph_mut().eval_frame(0.0, DT);
    let mut copie = relu(&c);
    copie.graph_mut().eval_frame(0.0, DT);
    assert_eq!(copie.bpm(), 93.5);
}

#[test]
fn l_analyse_audio_revient() {
    for method in AnalysisMethod::ALL {
        for freq_scale in FreqScale::ALL {
            for amp_scale in AmpScale::ALL {
                let mut c = starter();
                c.analysis = AudioAnalysis {
                    method,
                    freq_scale,
                    amp_scale,
                    window: Param::fixed(0.125),
                };
                assert_eq!(relu(&c).analysis, c.analysis);
            }
        }
    }
}

#[test]
fn une_analyse_inconnue_est_refusee() {
    let mut v = texte();
    v["analysis"]["method"] = "magie".into();
    assert!(refuse(v).to_string().contains("magie"));
}

#[test]
fn un_trig_garde_ses_reglages_et_sa_cible() {
    let (mut c, piste) = une_case(cercle());
    let clip = c.layer(piste).unwrap().cell(0).unwrap();
    let un = c.add_default_trig().unwrap();
    let deux = c.add_default_trig().unwrap();
    c.set_trig_param(un, frogenx_composition::TrigSlot::Every, 8.0);
    c.set_trig_param(un, frogenx_composition::TrigSlot::FreqMin, 55.0);
    c.trig_mut(un).unwrap().name = "Grosse caisse".into();
    c.trig_mut(un).unwrap().target = TrigTarget::Clip { layer: piste, clip };
    let colonne = c.columns()[0];
    c.trig_mut(deux).unwrap().target = TrigTarget::Column(colonne);

    let copie = relu(&c);
    let ids = copie.trigs();
    assert_eq!(ids.len(), 2);
    let (a, b) = (copie.trig(ids[0]).unwrap(), copie.trig(ids[1]).unwrap());
    assert_eq!(a.name, "Grosse caisse");
    assert_eq!(a.every().base(), 8.0);
    assert_eq!(a.zone().freq_min.base(), 55.0);
    assert_eq!(
        a.target,
        TrigTarget::Clip {
            layer: copie.layers_de_test()[0],
            clip: copie
                .layer(copie.layers_de_test()[0])
                .unwrap()
                .cell(0)
                .unwrap()
        }
    );
    assert_eq!(b.target, TrigTarget::Column(copie.columns()[0]));
}

#[test]
fn un_trig_recharge_se_regle_et_se_retire_comme_un_autre() {
    let (mut c, _) = une_case(cercle());
    c.add_default_trig().unwrap();

    let mut copie = relu(&c);
    let id = copie.trigs()[0];
    assert!(copie.set_trig_param(id, frogenx_composition::TrigSlot::Every, 4.0));
    assert_eq!(copie.trig(id).unwrap().every().base(), 4.0);
    assert!(copie.remove_trig(id));
    assert!(
        copie.graph_mut().nodes().is_empty(),
        "ses nœuds sont partis"
    );
}

#[test]
fn une_cible_disparue_ne_declenche_plus_rien() {
    let (mut c, piste) = une_case(cercle());
    let colonne = c.columns()[0];
    let clip = c.layer(piste).unwrap().cell(0).unwrap();
    let id = c.add_default_trig().unwrap();
    c.trig_mut(id).unwrap().target = TrigTarget::Clip { layer: piste, clip };
    c.remove_clip(piste, colonne);

    let copie = relu(&c);
    assert_eq!(
        copie.trig(copie.trigs()[0]).unwrap().target,
        TrigTarget::None
    );
}

#[test]
fn un_clip_garde_son_style_et_sa_transition() {
    let mut clip = cercle()
        .with_style(ShapeStyle::stroked(
            Param::fixed(0.03),
            ColorParam::srgb(0.25, 0.5, 0.75, 1.0),
        ))
        .with_transition(ClipTransition::Wipe {
            phase: PositionSignal::fixed_speed(0.5),
            curve: FadeCurve::SmoothStep,
            direction: WipeDirection::Up,
        });
    clip.speed = Param::fixed(1.5);
    let (c, _) = une_case(clip);

    let copie = relu(&c);
    let id = copie
        .layer(copie.layers_de_test()[0])
        .unwrap()
        .cell(0)
        .unwrap();
    let clip = copie.clip(id).unwrap();
    let ClipSource::Shape { style, .. } = &clip.source;
    let trait_ = style.stroke.expect("un trait");
    assert_eq!(trait_.width, Param::fixed(0.03));
    assert_eq!(trait_.color.g, Param::fixed(0.5));
    assert!(style.fill.is_none());
    assert_eq!(clip.speed, Param::fixed(1.5));
    let ClipTransition::Wipe {
        phase,
        curve,
        direction,
    } = &clip.transition
    else {
        panic!("la transition a changé");
    };
    assert_eq!(phase.speed, Param::fixed(0.5));
    assert_eq!(*curve, FadeCurve::SmoothStep);
    assert_eq!(*direction, WipeDirection::Up);
}

#[test]
fn une_source_retiree_s_ecrit_comme_sa_base() {
    let (mut c, piste) = une_case(cercle());
    let source = c.graph_mut().add(Box::new(Constant(1.0)));
    c.layer_mut(piste).unwrap().blend.opacity = Param::modulated(0.25, 0.5, source);
    c.graph_mut().remove(source).unwrap();

    let copie = relu(&c);
    let l = copie.layer(copie.layers_de_test()[0]).unwrap();
    assert_eq!(l.blend.opacity, Param::fixed(0.25));
}

#[test]
fn une_grille_a_plusieurs_decks_garde_le_deck_actif() {
    let mut c = Composition::new();
    let (a, b) = (c.add_deck("A"), c.add_deck("B"));
    let groupe = c.add_layer_group(b).unwrap();
    let piste = c.add_layer(groupe).unwrap();
    c.add_clip(piste, cercle()).unwrap();
    assert!(c.deck(a).is_some());
    c.set_active_deck(b);

    let copie = relu(&c);
    let actif = copie.active_deck().unwrap();
    assert_eq!(copie.deck(actif).unwrap().name, "B");
    assert_eq!(copie.layers_de_test().len(), 1);
}

#[test]
fn une_valeur_non_finie_est_refusee_a_l_enregistrement() {
    let (mut c, piste) = une_case(cercle());
    c.layer_mut(piste).unwrap().blend.opacity = Param::fixed(f32::NAN);
    assert!(matches!(to_json(&c), Err(PatchError::NotFinite(_))));
}

/// Le texte d'une petite grille, à abîmer.
fn texte() -> serde_json::Value {
    let (mut c, _) = une_case(cercle());
    c.add_default_trig().unwrap();
    serde_json::from_str(&to_json(&c).unwrap()).unwrap()
}

fn refuse(v: serde_json::Value) -> PatchError {
    from_json(&v.to_string()).err().expect("refusé")
}

#[test]
fn le_texte_de_reference_se_relit_sans_abime() {
    assert!(from_json(&texte().to_string()).is_ok());
}

#[test]
fn un_type_inconnu_est_refuse_en_le_nommant() {
    let mut v = texte();
    v["decks"][0]["groups"][0]["layers"][0]["cells"][0]["graph"][0]["kind"] = "Fantome".into();
    assert!(refuse(v).to_string().contains("Fantome"));
}

#[test]
fn une_forme_inconnue_est_refusee() {
    let mut v = texte();
    v["decks"][0]["groups"][0]["layers"][0]["cells"][0]["generator"]["kind"] = "Spirale".into();
    assert!(matches!(refuse(v), PatchError::Spec(_)));
}

#[test]
fn une_reference_en_avant_est_refusee() {
    let mut v = texte();
    v["decks"][0]["groups"][0]["layers"][0]["cells"][0]["graph"][0]["deps"] =
        serde_json::json!([1]);
    assert!(matches!(refuse(v), PatchError::Invalid(_)));
}

#[test]
fn une_source_hors_du_graphe_est_refusee() {
    let mut v = texte();
    v["tempo"] = serde_json::json!({"modulated": {"base": 1.0, "depth": 1.0, "source": 999}});
    assert!(matches!(refuse(v), PatchError::Invalid(_)));
}

#[test]
fn une_case_active_vide_est_refusee() {
    let mut v = texte();
    v["decks"][0]["groups"][0]["layers"][0]["active"] = 7.into();
    assert!(matches!(refuse(v), PatchError::Invalid(_)));
}

#[test]
fn une_piste_plus_longue_que_la_grille_est_refusee() {
    let mut v = texte();
    v["columns"] = 0.into();
    assert!(matches!(refuse(v), PatchError::Invalid(_)));
}

#[test]
fn un_nombre_de_colonnes_absurde_est_refuse_avant_d_allouer() {
    let mut v = texte();
    v["columns"] = (MAX_COLUMNS + 1).into();
    assert!(matches!(refuse(v), PatchError::Invalid(_)));
}

#[test]
fn un_trig_sur_le_mauvais_noeud_est_refuse() {
    let mut v = texte();
    // La porte désigne le compteur, qui n'est pas une zone.
    v["trigs"][0]["gate"] = v["trigs"][0]["pulse"].clone();
    assert!(matches!(refuse(v), PatchError::Spec(_)));
}

#[test]
fn un_champ_manquant_est_refuse() {
    let mut v = texte();
    v["decks"][0]["groups"][0]["layers"][0]
        .as_object_mut()
        .unwrap()
        .remove("blend");
    assert!(matches!(refuse(v), PatchError::Json(_)));
}
