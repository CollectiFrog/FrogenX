//! Une frame complète, sortie factice — §24.1 (« Intégration ») de
//! `docs/ARCHITECTURE.md`.
//!
//! Sans adaptateur, chaque test s'arrête **avec succès** et le dit (§24.3).

use frogenx_app::Engine;
use frogenx_composition::{
    Clip, ClipId, Command, Composition, LayerId, ParamSlot, PlayMode, Transport, TrigSlot,
    TrigTarget,
};
use frogenx_core::{
    AnalysisMethod, AnalysisSettings, AudioFeatures, Clock, Constant, Ctx, FeatureKind, FreqScale,
    Graph, NodeState, Param, Signal, Spectrum,
};
use frogenx_layer::{BlendMode, BlendParams, ColorParam, Gpu, ShapeStyle};
use frogenx_output::{
    CpuFrame, CpuSink, GpuFrame, GpuSink, OutputError, OutputSink, PngSequenceSink, SinkStatus,
};
use frogenx_shape::{FeedbackParams, Rect};
use glam::UVec2;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const SIZE: UVec2 = UVec2::new(32, 32);
const ROUGE: [u8; 4] = [255, 0, 0, 255];
const BLEU: [u8; 4] = [0, 0, 255, 255];
const VIDE: [u8; 4] = [0, 0, 0, 0];

fn gpu() -> Option<Gpu> {
    let gpu = Gpu::headless();
    if gpu.is_none() {
        eprintln!("GPU absent, test sauté (§24.3)");
    }
    gpu
}

#[derive(Clone, Default)]
struct Recu(Arc<Mutex<Vec<CpuFrame>>>);

impl Recu {
    fn dernier_pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let frames = self.0.lock().unwrap();
        pixel(frames.last().expect("une frame reçue"), x, y)
    }
}

impl CpuSink for Recu {
    fn submit(&mut self, frame: &CpuFrame) {
        self.0.lock().unwrap().push(frame.clone());
    }

    fn status(&self) -> SinkStatus {
        SinkStatus::Active
    }
}

fn pixel(frame: &CpuFrame, x: u32, y: u32) -> [u8; 4] {
    let i = 4 * (y * frame.width + x) as usize;
    [
        frame.pixels[i],
        frame.pixels[i + 1],
        frame.pixels[i + 2],
        frame.pixels[i + 3],
    ]
}

/// Un carré plein sur la moitié centrale, dans son propre graphe.
fn carre(g: Graph, rouge: f32, bleu: f32) -> Clip {
    let mut g = g;
    let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
    Clip::shape(g, Box::new(Rect::new(1.0, 1.0)), transport)
        .with_style(ShapeStyle::filled(ColorParam::srgb(rouge, 0.0, bleu, 1.0)))
}

/// Une piste avec un clip rouge et un clip bleu, aucun actif.
fn une_piste() -> (Composition, LayerId, ClipId, ClipId) {
    let mut c = Composition::new();
    let deck = c.add_deck("test");
    let groupe = c.add_layer_group(deck).unwrap();
    let piste = c.add_layer(groupe).unwrap();
    let rouge = c.add_clip(piste, carre(Graph::new(), 1.0, 0.0)).unwrap();
    let bleu = c.add_clip(piste, carre(Graph::new(), 0.0, 1.0)).unwrap();
    (c, piste, rouge, bleu)
}

/// Un moteur en `Fixed`, avec une sortie qui garde tout.
fn moteur(gpu: Gpu, composition: Composition) -> (Engine, Recu) {
    let recu = Recu::default();
    let mut moteur = Engine::new(gpu, Clock::fixed(60.0), SIZE, composition);
    moteur
        .add_output(OutputSink::Cpu(Box::new(recu.clone())))
        .unwrap();
    (moteur, recu)
}

/// Compte ses évaluations : un clip inactif ne doit jamais l'incrémenter.
struct Compteur(Arc<AtomicU32>);

impl Signal for Compteur {
    fn spec(&self) -> frogenx_core::Spec {
        frogenx_core::Spec::new("test")
    }

    fn eval(&self, _t: f64, _ctx: &Ctx, _state: &mut NodeState) -> f32 {
        self.0.fetch_add(1, Ordering::Relaxed);
        0.0
    }
}

#[test]
fn une_frame_traverse_tout_le_moteur_jusqu_a_une_sortie_cpu() {
    let Some(gpu) = gpu() else { return };
    let (c, layer, clip, _) = une_piste();
    let (mut moteur, recu) = moteur(gpu, c);
    moteur.push(Command::TriggerClip { layer, clip });

    for _ in 0..3 {
        // En `Fixed`, le temps mural est ignoré (§4).
        moteur.frame(99.0);
    }
    moteur.finish();

    let recu = recu.0.lock().unwrap();
    let indices: Vec<u64> = recu.iter().map(|f| f.index).collect();
    assert_eq!(indices, vec![0, 1, 2], "indices contigus en Fixed");
    for (i, frame) in recu.iter().enumerate() {
        let attendu = (i + 1) as f64 / 60.0;
        assert!((frame.t - attendu).abs() < 1e-9, "t : got {}", frame.t);
        assert_eq!((frame.width, frame.height), (SIZE.x, SIZE.y));
        assert_eq!(pixel(frame, 16, 16), ROUGE, "centre");
        assert_eq!(pixel(frame, 1, 1), VIDE, "coin transparent");
    }
    assert!(
        moteur.diagnostics().iter().all(|d| d.is_clean()),
        "got {:?}",
        moteur.diagnostics()
    );
    assert_eq!(moteur.output_statuses(), vec![SinkStatus::Active]);
}

#[test]
fn sans_clip_actif_la_sortie_est_transparente() {
    let Some(gpu) = gpu() else { return };
    let (c, ..) = une_piste();
    let (mut moteur, recu) = moteur(gpu, c);
    moteur.frame(0.0);
    moteur.finish();
    assert_eq!(recu.dernier_pixel(16, 16), VIDE);
}

#[test]
fn changer_de_clip_change_l_image_sans_residu() {
    let Some(gpu) = gpu() else { return };
    let (c, layer, rouge, bleu) = une_piste();
    let (mut moteur, recu) = moteur(gpu, c);

    moteur.push(Command::TriggerClip { layer, clip: rouge });
    moteur.frame(0.0);
    moteur.push(Command::TriggerClip { layer, clip: bleu });
    moteur.frame(0.0);
    moteur.finish();

    let frames = recu.0.lock().unwrap();
    assert_eq!(pixel(&frames[0], 16, 16), ROUGE);
    assert_eq!(pixel(&frames[1], 16, 16), BLEU);
}

#[test]
fn vider_une_piste_la_retire_de_l_image() {
    let Some(gpu) = gpu() else { return };
    let (c, layer, clip, _) = une_piste();
    let (mut moteur, recu) = moteur(gpu, c);

    moteur.push(Command::TriggerClip { layer, clip });
    moteur.frame(0.0);
    moteur.push(Command::ClearLayer { layer });
    moteur.frame(0.0);
    moteur.finish();

    let frames = recu.0.lock().unwrap();
    assert_eq!(pixel(&frames[0], 16, 16), ROUGE);
    assert_eq!(pixel(&frames[1], 16, 16), VIDE);
}

#[test]
fn seul_le_clip_actif_est_evalue() {
    let Some(gpu) = gpu() else { return };
    let mut c = Composition::new();
    let deck = c.add_deck("test");
    let groupe = c.add_layer_group(deck).unwrap();
    let layer = c.add_layer(groupe).unwrap();

    let (actif, inactif) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0)));
    let avec_compteur = |n: &Arc<AtomicU32>| {
        let mut g = Graph::new();
        g.add(Box::new(Compteur(Arc::clone(n))));
        g
    };
    let clip = c
        .add_clip(layer, carre(avec_compteur(&actif), 1.0, 0.0))
        .unwrap();
    c.add_clip(layer, carre(avec_compteur(&inactif), 0.0, 1.0))
        .unwrap();

    let (mut moteur, _recu) = moteur(gpu, c);
    moteur.push(Command::TriggerClip { layer, clip });
    for _ in 0..5 {
        moteur.frame(0.0);
    }
    moteur.finish();

    assert_eq!(actif.load(Ordering::Relaxed), 5);
    assert_eq!(inactif.load(Ordering::Relaxed), 0);
}

#[test]
fn l_opacite_d_une_piste_vient_du_graphe_de_composition_et_survit_au_clip() {
    let Some(gpu) = gpu() else { return };
    let (mut c, layer, rouge, bleu) = une_piste();
    let moitie = c.graph_mut().add(Box::new(Constant(0.5)));
    c.layer_mut(layer).unwrap().blend = BlendParams {
        opacity: Param::modulated(0.0, 1.0, moitie),
        mode: BlendMode::Normal,
    };
    let (mut moteur, recu) = moteur(gpu, c);

    moteur.push(Command::TriggerClip { layer, clip: rouge });
    moteur.frame(0.0);
    moteur.push(Command::TriggerClip { layer, clip: bleu });
    moteur.frame(0.0);
    moteur.finish();

    // Alpha droit (§8.0) : la couleur reste pleine, l'alpha porte la moitié.
    let frames = recu.0.lock().unwrap();
    let (a, b) = (pixel(&frames[0], 16, 16), pixel(&frames[1], 16, 16));
    assert_eq!(a[0], 255);
    assert_eq!(b[2], 255);
    assert!((a[3] as i32 - 128).abs() <= 1, "got {a:?}");
    assert!((b[3] as i32 - 128).abs() <= 1, "got {b:?}");
}

#[test]
fn le_tempo_atteint_le_clip_actif() {
    let Some(gpu) = gpu() else { return };
    let (c, layer, clip, _) = une_piste();
    let (mut moteur, _recu) = moteur(gpu, c);
    moteur.push(Command::TriggerClip { layer, clip });
    moteur.push(Command::SetBpm(240.0));

    for _ in 0..30 {
        moteur.frame(0.0);
    }
    moteur.finish();

    assert_eq!(moteur.bpm(), 240.0);
    let clip = moteur.composition().clip(clip).unwrap();
    let Transport::BpmSync { position, .. } = clip.transport else {
        panic!("attendu BpmSync");
    };
    // 30 frames à 60 fps font 0.5 s, soit 2 battements à 240 BPM.
    let battements = clip.graph().value(position);
    assert!((battements - 2.0).abs() < 1e-3, "got {battements}");
}

#[test]
fn un_reglage_change_l_image_a_la_frame_suivante() {
    let Some(gpu) = gpu() else { return };
    let (c, layer, clip, _) = une_piste();
    // Le rouge du remplissage, tel que l'interface le désigne.
    let slot = c.clip(clip).unwrap().params()[1].params[0].slot;
    let (mut moteur, recu) = moteur(gpu, c);

    moteur.push(Command::TriggerClip { layer, clip });
    moteur.frame(0.0);
    moteur.push(Command::SetParam {
        clip,
        slot,
        value: 0.0,
    });
    moteur.frame(0.0);
    moteur.finish();

    let frames = recu.0.lock().unwrap();
    assert_eq!(pixel(&frames[0], 16, 16), ROUGE);
    assert_eq!(pixel(&frames[1], 16, 16), [0, 0, 0, 255], "plus de rouge");
    assert_eq!(moteur.refused_commands(), 0);
}

#[test]
fn une_commande_refusee_se_compte_sans_paniquer() {
    let Some(gpu) = gpu() else { return };
    let (c, layer, ..) = une_piste();
    let (mut moteur, _recu) = moteur(gpu, c);
    moteur.push(Command::SetBpm(f32::NAN));
    moteur.push(Command::ClearLayer { layer });
    moteur.frame(0.0);
    moteur.finish();
    assert_eq!(moteur.refused_commands(), 1);
}

#[test]
fn une_colonne_de_la_grille_de_depart_change_l_image_et_reste_reproductible() {
    let Some(a) = gpu() else { return };
    let Some(b) = gpu() else { return };

    let rendre = |gpu: Gpu| {
        let grille = frogenx_library::starter();
        let colonnes = grille.columns().to_vec();
        let (mut moteur, recu) = moteur(gpu, grille);
        moteur.push(Command::TriggerColumn {
            column: colonnes[0],
        });
        for _ in 0..6 {
            moteur.frame(0.0);
        }
        moteur.push(Command::TriggerColumn {
            column: colonnes[1],
        });
        for _ in 0..6 {
            moteur.frame(0.0);
        }
        moteur.finish();
        assert!(
            moteur.diagnostics().iter().all(|d| d.is_clean()),
            "got {:?}",
            moteur.diagnostics()
        );
        let frames = recu.0.lock().unwrap().clone();
        frames
    };
    let (premier, second) = (rendre(a), rendre(b));

    assert_eq!(premier.len(), 12);
    assert_eq!(second.len(), 12);
    for (x, y) in premier.iter().zip(&second) {
        assert_eq!(x.pixels, y.pixels, "frame {} différente", x.index);
    }
    assert_ne!(
        premier[0].pixels, premier[3].pixels,
        "l'animation est figée"
    );
    assert_ne!(
        premier[5].pixels, premier[6].pixels,
        "la colonne n'a rien changé"
    );
}

/// La position du clip, en battements, après `frames` frames à 120 BPM.
fn battements(clip: &Clip) -> f32 {
    let Transport::BpmSync { position, .. } = clip.transport else {
        panic!("attendu BpmSync");
    };
    clip.graph().value(position)
}

#[test]
fn la_vitesse_d_un_clip_accelere_son_graphe_seulement() {
    let Some(gpu) = gpu() else { return };
    let (c, layer, clip, _) = une_piste();
    let (mut moteur, _recu) = moteur(gpu, c);
    moteur.push(Command::TriggerClip { layer, clip });
    moteur.push(Command::SetClipSpeed { clip, value: 2.0 });

    for _ in 0..30 {
        moteur.frame(0.0);
    }
    moteur.finish();

    // 0.5 s à 120 BPM font 1 battement ; le double à vitesse 2.
    let position = battements(moteur.composition().clip(clip).unwrap());
    assert!((position - 2.0).abs() < 1e-3, "got {position}");
    assert!(
        (moteur.clock().t() - 0.5).abs() < 1e-9,
        "l'horloge reste la même"
    );
}

#[test]
fn une_vitesse_nulle_fige_le_clip_sans_l_eteindre() {
    let Some(gpu) = gpu() else { return };
    let (c, layer, clip, _) = une_piste();
    let (mut moteur, recu) = moteur(gpu, c);
    moteur.push(Command::TriggerClip { layer, clip });
    moteur.push(Command::SetClipSpeed { clip, value: 0.0 });

    for _ in 0..5 {
        moteur.frame(0.0);
    }
    moteur.finish();

    assert_eq!(battements(moteur.composition().clip(clip).unwrap()), 0.0);
    assert_eq!(recu.dernier_pixel(16, 16), ROUGE);
}

#[test]
fn une_vitesse_modulee_vient_du_graphe_de_composition() {
    let Some(gpu) = gpu() else { return };
    let (mut c, layer, clip, _) = une_piste();
    let triple = c.graph_mut().add(Box::new(Constant(3.0)));
    c.clip_mut(clip).unwrap().speed = Param::modulated(0.0, 1.0, triple);
    let (mut moteur, _recu) = moteur(gpu, c);
    moteur.push(Command::TriggerClip { layer, clip });

    for _ in 0..30 {
        moteur.frame(0.0);
    }
    moteur.finish();

    let position = battements(moteur.composition().clip(clip).unwrap());
    assert!((position - 3.0).abs() < 1e-3, "got {position}");
}

#[test]
fn poser_un_clip_sur_la_case_qui_joue_change_l_image_a_la_frame_suivante() {
    let Some(gpu) = gpu() else { return };
    let (c, layer, rouge, _) = une_piste();
    let colonne = c.columns()[0];
    let (mut moteur, recu) = moteur(gpu, c);

    moteur.push(Command::TriggerClip { layer, clip: rouge });
    moteur.frame(0.0);
    moteur.place(layer, colonne, carre(Graph::new(), 0.0, 1.0));
    moteur.frame(0.0);
    moteur.finish();

    let frames = recu.0.lock().unwrap();
    assert_eq!(pixel(&frames[0], 16, 16), ROUGE);
    assert_eq!(pixel(&frames[1], 16, 16), BLEU);
    assert_eq!(moteur.refused_commands(), 0);
}

#[test]
fn poser_sur_une_colonne_retiree_se_compte_comme_un_refus() {
    let Some(gpu) = gpu() else { return };
    let (c, layer, ..) = une_piste();
    let colonne = c.columns()[0];
    let (mut moteur, _recu) = moteur(gpu, c);

    moteur.push(Command::RemoveColumn { column: colonne });
    moteur.place(layer, colonne, carre(Graph::new(), 0.0, 1.0));
    moteur.frame(0.0);
    moteur.finish();
    assert_eq!(moteur.refused_commands(), 1);
}

#[test]
fn retirer_la_piste_qui_joue_la_sort_de_l_image_et_du_moteur() {
    let Some(gpu) = gpu() else { return };
    let (c, layer, clip, _) = une_piste();
    let (mut moteur, recu) = moteur(gpu, c);

    moteur.push(Command::TriggerClip { layer, clip });
    moteur.frame(0.0);
    moteur.push(Command::RemoveLayer { layer });
    moteur.frame(0.0);
    moteur.push(Command::AddLayer);
    moteur.frame(0.0);
    moteur.finish();

    let frames = recu.0.lock().unwrap();
    assert_eq!(pixel(&frames[0], 16, 16), ROUGE);
    assert_eq!(pixel(&frames[1], 16, 16), VIDE);
    assert_eq!(pixel(&frames[2], 16, 16), VIDE);
    assert_eq!(moteur.refused_commands(), 0);
}

#[test]
fn chaque_clip_du_catalogue_se_rend_sur_une_piste() {
    let Some(gpu) = gpu() else { return };
    let mut c = Composition::new();
    let deck = c.add_deck("test");
    let groupe = c.add_layer_group(deck).unwrap();
    let layer = c.add_layer(groupe).unwrap();
    let (mut moteur, recu) = moteur(gpu, {
        for preset in frogenx_library::catalog() {
            c.add_clip(layer, preset.build()).unwrap();
        }
        c
    });
    let colonnes = moteur.composition().columns().to_vec();
    assert_eq!(colonnes.len(), frogenx_library::catalog().len());

    for column in &colonnes {
        moteur.push(Command::TriggerColumn { column: *column });
        moteur.frame(0.0);
    }
    moteur.finish();

    assert!(
        moteur.diagnostics().iter().all(|d| d.is_clean()),
        "got {:?}",
        moteur.diagnostics()
    );
    let frames = recu.0.lock().unwrap();
    assert_eq!(frames.len(), colonnes.len());
    for (frame, preset) in frames.iter().zip(frogenx_library::catalog()) {
        assert!(
            frame.pixels.as_chunks::<4>().0.iter().any(|px| px[3] > 0),
            "{} n'a rien dessiné",
            preset.name
        );
    }
}

#[test]
fn le_moteur_ecrit_une_sequence_png() {
    let Some(gpu) = gpu() else { return };
    let dossier = std::env::temp_dir().join(format!("frogenx-engine-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dossier);

    let (c, layer, clip, _) = une_piste();
    let mut moteur = Engine::new(gpu, Clock::fixed(60.0), SIZE, c);
    moteur
        .add_output(OutputSink::Cpu(Box::new(
            PngSequenceSink::open(&dossier).unwrap(),
        )))
        .unwrap();
    moteur.push(Command::TriggerClip { layer, clip });
    for _ in 0..4 {
        moteur.frame(0.0);
    }
    // Sans ce `finish`, les dernières frames seraient encore en file.
    moteur.finish();

    let fichiers = std::fs::read_dir(&dossier).unwrap().count();
    let derniere = PngSequenceSink::path_for(&dossier, 3).exists();
    std::fs::remove_dir_all(&dossier).unwrap();
    assert_eq!(fichiers, 4);
    assert!(derniere);
}

/// Compte les frames reçues : une sortie de scène, sans fenêtre.
struct FramesVues(Arc<AtomicU32>);

impl GpuSink for FramesVues {
    fn encode(&mut self, _: &Gpu, _: &mut wgpu::CommandEncoder, _: &GpuFrame<'_>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    fn after_submit(&mut self, _: &Gpu) {}

    fn status(&self) -> SinkStatus {
        SinkStatus::Active
    }
}

#[test]
fn fermer_une_sortie_laisse_tourner_le_moteur_et_les_autres_sorties() {
    let Some(gpu) = gpu() else { return };
    let (composition, _, _, _) = une_piste();
    let (mut moteur, recu) = moteur(gpu, composition);
    let vues = Arc::new(AtomicU32::new(0));
    let sortie = moteur.add_gpu_output(Box::new(FramesVues(Arc::clone(&vues))));

    moteur.frame(0.0);
    assert!(moteur.remove_gpu_output(sortie));
    moteur.frame(0.0);
    moteur.finish();

    assert_eq!(vues.load(Ordering::Relaxed), 1);
    assert_eq!(recu.0.lock().unwrap().len(), 2, "le moteur a continué");
    assert_eq!(moteur.output_statuses(), vec![SinkStatus::Active]);
    assert!(!moteur.remove_gpu_output(sortie));
}

#[test]
fn jouer_ne_modifie_pas_mais_regler_si() {
    let Some(gpu) = gpu() else { return };
    let (composition, piste, rouge, _) = une_piste();
    let (mut moteur, _) = moteur(gpu, composition);
    assert!(!moteur.is_dirty());

    moteur.push(Command::TriggerClip {
        layer: piste,
        clip: rouge,
    });
    moteur.frame(0.0);
    assert!(!moteur.is_dirty(), "déclencher est jouer");

    moteur.push(Command::SetBpm(90.0));
    moteur.frame(0.0);
    assert!(moteur.is_dirty());
}

#[test]
fn une_commande_refusee_ne_compte_pas_comme_une_modification() {
    let Some(gpu) = gpu() else { return };
    let (composition, _, _, _) = une_piste();
    let (mut moteur, _) = moteur(gpu, composition);
    moteur.push(Command::SetBpm(f32::NAN));
    moteur.frame(0.0);
    assert!(!moteur.is_dirty());
    assert_eq!(moteur.refused_commands(), 1);
}

#[test]
fn poser_un_clip_est_une_modification() {
    let Some(gpu) = gpu() else { return };
    let (composition, piste, _, _) = une_piste();
    let colonne = composition.columns()[0];
    let (mut moteur, _) = moteur(gpu, composition);
    moteur.place(piste, colonne, carre(Graph::new(), 0.0, 1.0));
    moteur.frame(0.0);
    assert!(moteur.is_dirty());
}

#[test]
fn un_fichier_rouvert_donne_la_meme_image() {
    let Some(gpu) = gpu() else { return };
    let Some(gpu2) = self::gpu() else { return };
    let rendre = |gpu: Gpu, grille: Composition| {
        let (mut moteur, recu) = moteur(gpu, grille);
        for _ in 0..30 {
            moteur.frame(0.0);
        }
        moteur.finish();
        let frames = recu.0.lock().unwrap();
        frames.last().expect("une frame").pixels.to_vec()
    };

    // La grille de départ a un clip actif par piste une fois sa colonne jouée.
    let mut grille = frogenx_library::starter();
    let (colonne, mut pistes) = (grille.columns()[0], Vec::new());
    grille.layers_in_order(&mut pistes);
    grille.trigger_column(colonne, &pistes);
    let texte = frogenx_patch::to_json(&grille).unwrap();
    let relue = frogenx_patch::from_json(&texte).unwrap();

    let a = rendre(gpu, grille);
    let b = rendre(gpu2, relue);
    assert!(a.iter().any(|o| *o != 0), "une image, pas du vide");
    assert_eq!(a, b);
}

#[test]
fn remplacer_la_grille_oublie_ce_qui_visait_l_ancienne() {
    let Some(gpu) = gpu() else { return };
    let (composition, piste, rouge, _) = une_piste();
    let colonne = composition.columns()[0];
    let (mut moteur, recu) = moteur(gpu, composition);
    moteur.set_dirty(true);
    moteur.push(Command::TriggerClip {
        layer: piste,
        clip: rouge,
    });
    moteur.place(piste, colonne, carre(Graph::new(), 0.0, 1.0));

    let (nouvelle, _, _, _) = une_piste();
    moteur.replace_composition(nouvelle);
    moteur.frame(0.0);
    moteur.finish();

    assert_eq!(
        moteur.refused_commands(),
        0,
        "rien n'a visé la nouvelle grille"
    );
    assert!(!moteur.is_dirty());
    assert_eq!(
        recu.dernier_pixel(SIZE.x / 2, SIZE.y / 2)[3],
        0,
        "rien ne joue"
    );
}

#[test]
fn la_grille_d_usine_joue_sa_premiere_colonne() {
    let Some(gpu) = gpu() else { return };
    let (composition, _, _, _) = une_piste();
    let (mut moteur, recu) = moteur(gpu, composition);
    moteur.play_starter(frogenx_library::starter());
    moteur.frame(0.0);
    moteur.finish();
    assert!(!moteur.is_dirty());
    assert_eq!(recu.dernier_pixel(0, 0)[3], 255, "le fond joue");
}

fn tempo_detecte(bpm: f32, confidence: f32) -> AudioFeatures {
    AudioFeatures {
        bpm,
        bpm_confidence: confidence,
        ..Default::default()
    }
}

#[test]
fn suivre_le_tempo_ecrit_le_bpm_detecte_sans_marquer_de_modification() {
    let Some(gpu) = gpu() else { return };
    let (composition, _, _, _) = une_piste();
    let (mut moteur, _) = moteur(gpu, composition);

    moteur.set_audio(tempo_detecte(133.0, 0.9));
    moteur.frame(0.0);
    assert_eq!(moteur.bpm(), 120.0, "sans suivi, rien ne bouge");

    moteur.push(Command::SetTempoFollow(true));
    moteur.frame(0.0);
    assert_eq!(moteur.bpm(), 133.0);

    moteur.set_dirty(false);
    moteur.set_audio(tempo_detecte(134.0, 0.9));
    moteur.frame(0.0);
    assert_eq!(moteur.bpm(), 134.0);
    assert!(!moteur.is_dirty(), "le tempo qui suit n'est pas un réglage");
}

#[test]
fn un_tempo_peu_fiable_ou_absent_laisse_le_dernier_en_place() {
    let Some(gpu) = gpu() else { return };
    let (composition, _, _, _) = une_piste();
    let (mut moteur, _) = moteur(gpu, composition);
    moteur.push(Command::SetTempoFollow(true));
    moteur.set_audio(tempo_detecte(133.0, 0.9));
    moteur.frame(0.0);

    moteur.set_audio(tempo_detecte(90.0, 0.1));
    moteur.frame(0.0);
    assert_eq!(moteur.bpm(), 133.0, "peu fiable");

    moteur.set_audio(tempo_detecte(0.0, 0.0));
    moteur.frame(0.0);
    assert_eq!(moteur.bpm(), 133.0, "le silence garde le tempo");
}

#[test]
fn le_son_change_l_image_par_une_source_audio() {
    let Some(gpu) = gpu() else { return };
    let (composition, piste, rouge, _) = une_piste();
    let (mut moteur, recu) = moteur(gpu, composition);
    moteur.push(Command::TriggerClip {
        layer: piste,
        clip: rouge,
    });
    moteur.frame(0.0);

    // La largeur du carré suit les graves : à zéro, il a disparu du centre.
    moteur.push(Command::SetAudioModulation {
        clip: rouge,
        slot: ParamSlot::Generator(0),
        feature: Some(FeatureKind::Bass),
    });
    moteur.push(Command::SetParam {
        clip: rouge,
        slot: ParamSlot::Generator(0),
        value: 0.0,
    });
    for _ in 0..3 {
        moteur.frame(0.0);
    }

    moteur.set_audio(AudioFeatures {
        bands: [1.0, 0.0, 0.0],
        ..Default::default()
    });
    for _ in 0..30 {
        moteur.frame(0.0);
    }
    moteur.finish();

    let frames = recu.0.lock().unwrap();
    let centre = |i: usize| pixel(&frames[i], SIZE.x / 2, SIZE.y / 2);
    assert_eq!(centre(0)[0], 255, "le carré, avant la source");
    assert_eq!(centre(3)[3], 0, "silence : rien au centre");
    assert_eq!(
        centre(frames.len() - 1)[0],
        255,
        "les graves le font revenir"
    );
}

/// Un écho sur la piste du bas, une forme rouge au-dessus, à `dt` = 1/60 s.
fn avec_echo(params: FeedbackParams) -> (Composition, LayerId, ClipId, ClipId) {
    let mut c = Composition::new();
    let deck = c.add_deck("test");
    let groupe = c.add_layer_group(deck).unwrap();
    let bas = c.add_layer(groupe).unwrap();
    let haut = c.add_layer(groupe).unwrap();
    let mut g = Graph::new();
    let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
    let echo = c
        .add_clip(bas, Clip::feedback(g, params, transport))
        .unwrap();
    c.trigger(bas, echo);
    let forme = c.add_clip(haut, carre(Graph::new(), 1.0, 0.0)).unwrap();
    (c, haut, forme, echo)
}

/// Joue la forme une frame, la retire, puis laisse l'écho seul `frames` frames.
fn frames_de_l_echo(gpu: Gpu, params: FeedbackParams, frames: usize) -> Vec<CpuFrame> {
    let (composition, haut, forme, _) = avec_echo(params);
    let (mut moteur, recu) = moteur(gpu, composition);
    moteur.push(Command::TriggerClip {
        layer: haut,
        clip: forme,
    });
    moteur.frame(0.0);
    moteur.push(Command::ClearLayer { layer: haut });
    for _ in 0..frames {
        moteur.frame(0.0);
    }
    moteur.finish();
    let frames = recu.0.lock().unwrap();
    frames.clone()
}

#[test]
fn l_echo_garde_l_image_et_la_perd_selon_sa_memoire() {
    let Some(gpu) = gpu() else { return };
    let params = FeedbackParams {
        memory: Param::fixed(0.25),
        ..Default::default()
    };
    let frames = frames_de_l_echo(gpu, params, 30);
    let centre = |i: usize| pixel(&frames[i], SIZE.x / 2, SIZE.y / 2);

    assert_eq!(centre(0)[3], 255, "la forme");
    // 15 frames font 0.25 s : une demi-vie. 30 en font deux.
    let (a, b) = (centre(15)[3] as f32, centre(30)[3] as f32);
    assert!((a - 127.5).abs() < 6.0, "après une demi-vie : {a}");
    assert!((b - 63.75).abs() < 6.0, "après deux demi-vies : {b}");
    assert_eq!(
        centre(15)[0],
        255,
        "toujours rouge : l'alpha porte la perte"
    );
}

#[test]
fn l_echo_lit_la_frame_d_avant_jamais_la_courante() {
    let Some(gpu) = gpu() else { return };
    // Le carré ne touche pas le coin : au zoom, l'écho n'y arrive qu'après lui.
    let frames = frames_de_l_echo(gpu, FeedbackParams::default(), 1);
    assert_eq!(pixel(&frames[0], 0, 0)[3], 0);
}

#[test]
fn la_premiere_frame_d_un_echo_ne_lit_rien() {
    let Some(gpu) = gpu() else { return };
    let (composition, ..) = avec_echo(FeedbackParams::default());
    let (mut moteur, recu) = moteur(gpu, composition);
    moteur.frame(0.0);
    moteur.finish();
    assert_eq!(recu.dernier_pixel(SIZE.x / 2, SIZE.y / 2)[3], 0);
}

#[test]
fn le_zoom_fait_grandir_l_image() {
    let Some(gpu) = gpu() else { return };
    // Juste hors du carré, qui couvre la moitié centrale de l'image.
    let (x, y) = (SIZE.x * 7 / 8, SIZE.y / 2);
    let fige = FeedbackParams {
        memory: Param::fixed(30.0),
        ..Default::default()
    };
    let immobile = frames_de_l_echo(gpu, fige, 30);
    assert_eq!(pixel(immobile.last().unwrap(), x, y)[3], 0, "sans zoom");

    let Some(gpu) = self::gpu() else { return };
    let zoom = FeedbackParams {
        zoom: Param::fixed(4.0),
        ..fige
    };
    let agrandi = frames_de_l_echo(gpu, zoom, 60);
    assert!(pixel(agrandi.last().unwrap(), x, y)[3] > 200, "avec zoom");
}

#[test]
fn la_derive_deplace_l_image() {
    let Some(gpu) = gpu() else { return };
    let params = FeedbackParams {
        drift_x: Param::fixed(0.5),
        memory: Param::fixed(30.0),
        ..Default::default()
    };
    let frames = frames_de_l_echo(gpu, params, 60);
    let derniere = frames.last().unwrap();
    // Le milieu du carré déplacé : ses bords s'adoucissent à chaque relecture.
    assert!(
        pixel(derniere, SIZE.x * 3 / 4, SIZE.y / 2)[3] > 200,
        "à droite"
    );
    assert_eq!(
        pixel(derniere, SIZE.x / 8, SIZE.y / 2)[3],
        0,
        "pas à gauche"
    );
}

#[test]
fn un_echo_a_l_arret_fige_l_image() {
    let Some(gpu) = gpu() else { return };
    let (composition, haut, forme, echo) = avec_echo(FeedbackParams {
        memory: Param::fixed(0.05),
        ..Default::default()
    });
    let (mut moteur, recu) = moteur(gpu, composition);
    moteur.push(Command::TriggerClip {
        layer: haut,
        clip: forme,
    });
    moteur.frame(0.0);
    moteur.push(Command::ClearLayer { layer: haut });
    moteur.push(Command::SetClipSpeed {
        clip: echo,
        value: 0.0,
    });
    for _ in 0..30 {
        moteur.frame(0.0);
    }
    moteur.finish();
    assert_eq!(recu.dernier_pixel(SIZE.x / 2, SIZE.y / 2)[3], 255);
}

#[test]
fn un_echo_rejoue_a_l_identique() {
    let Some(gpu) = gpu() else { return };
    let Some(gpu2) = self::gpu() else { return };
    let params = FeedbackParams {
        zoom: Param::fixed(1.5),
        rotation: Param::fixed(0.7),
        memory: Param::fixed(1.0),
        ..Default::default()
    };
    let (a, b) = (
        frames_de_l_echo(gpu, params, 20),
        frames_de_l_echo(gpu2, params, 20),
    );
    assert_eq!(a.last().unwrap().pixels, b.last().unwrap().pixels);
}

#[test]
fn le_son_module_un_echo() {
    let Some(gpu) = gpu() else { return };
    // Mémoire à zéro : seuls les graves la font tenir.
    let (composition, haut, forme, echo) = avec_echo(FeedbackParams {
        memory: Param::fixed(0.0),
        ..Default::default()
    });
    let (mut moteur, recu) = moteur(gpu, composition);
    moteur.push(Command::SetAudioModulation {
        clip: echo,
        slot: ParamSlot::Generator(4),
        feature: Some(FeatureKind::Bass),
    });
    moteur.push(Command::SetDepth {
        clip: echo,
        slot: ParamSlot::Generator(4),
        value: 30.0,
    });
    moteur.set_audio(AudioFeatures {
        bands: [1.0, 0.0, 0.0],
        ..Default::default()
    });
    moteur.frame(0.0);
    for _ in 0..5 {
        moteur.push(Command::TriggerClip {
            layer: haut,
            clip: forme,
        });
        moteur.frame(0.0);
    }
    moteur.push(Command::ClearLayer { layer: haut });
    for _ in 0..3 {
        moteur.frame(0.0);
    }
    moteur.finish();
    assert!(recu.dernier_pixel(SIZE.x / 2, SIZE.y / 2)[3] > 200);
}

#[test]
fn le_temps_reel_refuse_une_sortie_cpu() {
    let Some(gpu) = gpu() else { return };
    let (c, ..) = une_piste();
    let mut moteur = Engine::new(gpu, Clock::realtime(), SIZE, c);
    assert_eq!(
        moteur.add_output(OutputSink::Cpu(Box::new(Recu::default()))),
        Err(OutputError::CpuSinkRequiresFixedClock)
    );
    moteur.frame(0.016);
    assert!((moteur.clock().t() - 0.016).abs() < 1e-9);
}

/// Un spectre plat : tout le spectre à ce niveau.
fn son(niveau: f32) -> AudioFeatures {
    AudioFeatures {
        spectrum: Spectrum::flat(niveau),
        ..Default::default()
    }
}

#[test]
fn un_coup_dans_la_zone_declenche_le_clip_une_fois_sur_deux() {
    let Some(gpu) = gpu() else { return };
    let (mut c, layer, rouge, bleu) = une_piste();
    let trig = c.add_default_trig().unwrap();
    c.trig_mut(trig).unwrap().target = TrigTarget::Clip { layer, clip: bleu };
    assert!(c.set_trig_param(trig, TrigSlot::Every, 2.0));

    let (mut moteur, recu) = moteur(gpu, c);
    moteur.push(Command::TriggerClip { layer, clip: rouge });
    moteur.frame(0.0);
    let actif = |m: &Engine| m.composition().layer(layer).unwrap().active_clip();
    assert_eq!(actif(&moteur), Some(rouge));

    // Premier coup : il passe, et agit à la frame suivante (§3bis.6).
    moteur.set_audio(son(0.9));
    moteur.frame(0.0);
    assert_eq!(actif(&moteur), Some(rouge), "une frame de retard");
    moteur.set_audio(son(0.0));
    moteur.frame(0.0);
    assert_eq!(actif(&moteur), Some(bleu));
    moteur.finish();
    assert_eq!(recu.dernier_pixel(16, 16), BLEU);

    // Deuxième coup : le compteur le retient.
    moteur.push(Command::TriggerClip { layer, clip: rouge });
    moteur.frame(0.0);
    moteur.set_audio(son(0.9));
    moteur.frame(0.0);
    moteur.set_audio(son(0.0));
    moteur.frame(0.0);
    assert_eq!(actif(&moteur), Some(rouge));

    // Troisième coup : il passe de nouveau.
    moteur.set_audio(son(0.9));
    moteur.frame(0.0);
    moteur.set_audio(son(0.0));
    moteur.frame(0.0);
    assert_eq!(actif(&moteur), Some(bleu));
    assert_eq!(moteur.refused_commands(), 0);
}

#[test]
fn le_clip_actif_lit_le_spectre_dans_son_propre_graphe() {
    // Une zone dans le graphe d'un clip : le trig y est un Signal ordinaire.
    let Some(gpu) = gpu() else { return };
    let mut g = Graph::new();
    let zone = g.add(Box::new(frogenx_osc::SpectrumZone::new(
        (20.0, 20_000.0),
        (0.5, 1.0),
    )));
    let (mut c, layer, ..) = une_piste();
    let clip = c.add_clip(layer, carre(g, 1.0, 0.0)).unwrap();

    let (mut moteur, _) = moteur(gpu, c);
    moteur.push(Command::TriggerClip { layer, clip });
    moteur.set_audio(son(0.9));
    moteur.frame(0.0);
    let graphe = moteur.composition().clip(clip).unwrap().graph();
    assert_eq!(graphe.value(zone), 1.0);
}

#[test]
fn une_commande_change_ce_que_le_moteur_demande_a_l_analyse() {
    // C'est ce que le binaire transmet à l'entrée, à chaque frame (§7).
    let Some(gpu) = gpu() else { return };
    let (c, ..) = une_piste();
    let (mut moteur, _) = moteur(gpu, c);
    assert_eq!(moteur.analysis(), AnalysisSettings::default());

    moteur.push(Command::SetAnalysisMethod(AnalysisMethod::ConstantQ));
    moteur.push(Command::SetFreqScale(FreqScale::Linear));
    moteur.push(Command::SetAnalysisWindow(0.25));
    moteur.frame(0.0);
    let reglage = moteur.analysis();
    assert_eq!(reglage.method, AnalysisMethod::ConstantQ);
    assert_eq!(reglage.freq_scale, FreqScale::Linear);
    assert_eq!(reglage.window, 0.25);
}

#[test]
fn sans_audio_un_trig_reste_inerte() {
    let Some(gpu) = gpu() else { return };
    let (mut c, layer, _, bleu) = une_piste();
    let trig = c.add_default_trig().unwrap();
    c.trig_mut(trig).unwrap().target = TrigTarget::Clip { layer, clip: bleu };
    let (mut moteur, _) = moteur(gpu, c);
    for _ in 0..5 {
        moteur.frame(0.0);
    }
    let piste = moteur.composition().layer(layer).unwrap();
    assert_eq!(piste.active_clip(), None);
}
