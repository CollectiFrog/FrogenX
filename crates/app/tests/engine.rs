//! Une frame complète, sortie factice — §24.1 (« Intégration ») de
//! `docs/ARCHITECTURE.md`.
//!
//! Sans adaptateur, chaque test s'arrête **avec succès** et le dit (§24.3).

use frogenx_app::Engine;
use frogenx_composition::{
    Clip, ClipId, Command, Composition, GraphScope, LayerId, NodeKind, ParamSlot, ParamTarget,
    PlayMode, Quantize, TransitionKind, Transport, TrigSlot, TrigTarget, WipeDirection,
};
use frogenx_core::{
    AnalysisMethod, AnalysisSettings, AudioFeatures, Clock, Constant, Ctx, FeatureKind, FreqScale,
    Graph, NodeState, Param, Signal, Spectrum,
};
use frogenx_layer::{BlendMode, BlendParams, ColorParam, Gpu, ShapeStyle};
use frogenx_output::{
    CpuFrame, CpuSink, GpuFrame, GpuSink, OutputSink, PixelLayout, PngSequenceSink, SinkStatus,
    StreamFormat,
};
use frogenx_shape::{EffectKind, FeedbackParams, Rect, VideoParams};
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
fn un_flash_de_trig_cable_dans_l_editeur_allume_une_piste() {
    let Some(gpu) = gpu() else { return };
    let (mut composition, piste, rouge, _) = une_piste();
    composition.trigger(piste, rouge);
    let trig = composition.add_default_trig().unwrap();
    let flash = composition.trig(trig).unwrap().flash();
    let (mut moteur, recu) = moteur(gpu, composition);

    let target = ParamTarget::LayerOpacity(piste);
    moteur.push(Command::SetTargetBase { target, value: 0.0 });
    moteur.push(Command::Connect {
        target,
        source: flash,
    });
    moteur.set_audio(son(0.0));
    moteur.frame(0.0);
    moteur.frame(0.0);
    moteur.set_audio(son(0.9));
    moteur.frame(0.0);
    moteur.frame(0.0);
    moteur.finish();
    assert!(moteur.is_dirty(), "un câble est une modification");

    let frames = recu.0.lock().unwrap();
    let centre = |i: usize| pixel(&frames[i], SIZE.x / 2, SIZE.y / 2);
    assert_eq!(centre(1)[3], 0, "silence : la piste est éteinte");
    assert!(centre(3)[3] > 100, "le coup l'allume : {:?}", centre(3));
}

#[test]
fn un_noeud_ajoute_et_cable_module_l_image() {
    let Some(gpu) = gpu() else { return };
    let (composition, piste, rouge, _) = une_piste();
    let (mut moteur, recu) = moteur(gpu, composition);
    moteur.push(Command::TriggerClip {
        layer: piste,
        clip: rouge,
    });
    let scope = GraphScope::Clip(rouge);
    moteur.push(Command::AddNode {
        scope,
        kind: NodeKind::Constant,
    });
    moteur.frame(0.0);

    let graphe = moteur.composition().graph_of(scope).unwrap();
    let constante = graphe
        .nodes()
        .iter()
        .find(|n| n.spec.kind == "Constant")
        .unwrap()
        .id;
    let target = ParamTarget::ClipParam {
        clip: rouge,
        slot: ParamSlot::Generator(0),
    };
    moteur.push(Command::Connect {
        target,
        source: constante,
    });
    moteur.push(Command::SetTargetBase { target, value: 0.0 });
    moteur.push(Command::SetTargetDepth { target, value: 0.0 });
    moteur.frame(0.0);
    moteur.push(Command::SetTargetDepth { target, value: 1.0 });
    moteur.frame(0.0);
    moteur.finish();

    let frames = recu.0.lock().unwrap();
    let centre = |i: usize| pixel(&frames[i], SIZE.x / 2, SIZE.y / 2);
    assert_eq!(centre(1)[3], 0, "amplitude nulle : largeur nulle");
    assert_eq!(centre(2)[0], 255, "la constante rend sa largeur au carré");
}

#[test]
fn un_trig_module_un_effet_du_clip() {
    let Some(gpu) = gpu() else { return };
    let (mut composition, piste, rouge, _) = une_piste();
    composition.trigger(piste, rouge);
    let trig = composition.add_default_trig().unwrap();
    let channel = composition.trig(trig).unwrap().channel();
    let (mut moteur, recu) = moteur(gpu, composition);

    let slot = ParamSlot::Effect {
        effect: 0,
        param: 0,
    };
    moteur.push(Command::AddEffect {
        clip: rouge,
        kind: EffectKind::Transform,
    });
    moteur.push(Command::SetParam {
        clip: rouge,
        slot,
        value: 0.1,
    });
    moteur.push(Command::SetTrigModulation {
        clip: rouge,
        slot,
        channel,
    });
    moteur.push(Command::SetDepth {
        clip: rouge,
        slot,
        value: 0.9,
    });
    moteur.set_audio(son(0.0));
    moteur.frame(0.0);
    moteur.frame(0.0);
    moteur.set_audio(son(0.9));
    moteur.frame(0.0);
    moteur.frame(0.0);
    moteur.finish();
    assert!(moteur.is_dirty(), "un branchement est une modification");

    let frames = recu.0.lock().unwrap();
    let bord = |i: usize| pixel(&frames[i], SIZE.x * 3 / 8, SIZE.y / 2);
    assert_eq!(bord(1)[3], 0, "silence : la forme reste petite");
    assert!(bord(3)[3] > 100, "le coup l'agrandit : {:?}", bord(3));
}

#[test]
fn un_trig_retire_laisse_l_effet_a_sa_base_sans_panne() {
    let Some(gpu) = gpu() else { return };
    let (mut composition, piste, rouge, _) = une_piste();
    composition.trigger(piste, rouge);
    let trig = composition.add_default_trig().unwrap();
    let channel = composition.trig(trig).unwrap().channel();
    let (mut moteur, recu) = moteur(gpu, composition);
    let slot = ParamSlot::Effect {
        effect: 0,
        param: 0,
    };
    moteur.push(Command::AddEffect {
        clip: rouge,
        kind: EffectKind::Transform,
    });
    moteur.push(Command::SetTrigModulation {
        clip: rouge,
        slot,
        channel,
    });
    moteur.set_audio(son(0.9));
    moteur.frame(0.0);
    moteur.push(Command::RemoveTrig(trig));
    moteur.frame(0.0);
    moteur.frame(0.0);
    moteur.finish();
    assert_eq!(recu.0.lock().unwrap().len(), 3);
}

#[test]
fn annuler_retrouve_le_trig_branche_sur_un_effet() {
    let Some(gpu) = gpu() else { return };
    let (mut composition, piste, rouge, _) = une_piste();
    composition.trigger(piste, rouge);
    composition.add_default_trig().unwrap();
    assert!(composition.add_effect(rouge, EffectKind::Transform));
    let (mut moteur, _) = moteur(gpu, composition);
    let slot = ParamSlot::Effect {
        effect: 0,
        param: 0,
    };
    moteur.push(Command::SetTrigModulation {
        clip: rouge,
        slot,
        channel: 0,
    });
    frames(&mut moteur, 20);
    assert_eq!(moteur.undo_label(), Some("Source de trig"));
    let fait = moteur.undo().expect("un pas à annuler");
    let clip = fait.map.clip(rouge).unwrap();
    let lit_un_trig = |m: &Engine, clip| {
        let c = m.composition().clip(clip).unwrap();
        c.params()
            .iter()
            .flat_map(|g| g.params.clone())
            .any(|p| p.trig.is_some())
    };
    assert!(!lit_un_trig(&moteur, clip));
    let fait = moteur.redo().expect("un pas à rétablir");
    assert!(lit_un_trig(&moteur, fait.map.clip(clip).unwrap()));
    moteur.finish();
}

#[test]
fn un_effet_ajoute_change_l_image_a_la_frame_suivante() {
    let Some(gpu) = gpu() else { return };
    let (composition, piste, rouge, _) = une_piste();
    let (mut moteur, recu) = moteur(gpu, composition);
    moteur.push(Command::TriggerClip {
        layer: piste,
        clip: rouge,
    });
    moteur.frame(0.0);
    moteur.push(Command::AddEffect {
        clip: rouge,
        kind: EffectKind::Transform,
    });
    moteur.push(Command::SetParam {
        clip: rouge,
        slot: ParamSlot::Effect {
            effect: 0,
            param: 0,
        },
        value: 0.1,
    });
    moteur.frame(0.0);
    moteur.finish();
    let frames = recu.0.lock().unwrap();
    let bord = |i: usize| pixel(&frames[i], SIZE.x * 3 / 8, SIZE.y / 2);
    assert_eq!(bord(0)[0], 255, "dans le carré");
    assert_eq!(bord(1)[3], 0, "réduit au dixième, il n'y arrive plus");
}

/// Une grille de départ, qui joue, en temps fixe.
fn scene(gpu: Gpu) -> Engine {
    let grille = frogenx_library::starter();
    let mut moteur = Engine::new(gpu, Clock::fixed(60.0), SIZE, grille);
    let colonne = moteur.composition().columns()[0];
    moteur.push(Command::TriggerColumn { column: colonne });
    moteur.frame(0.0);
    moteur
}

fn frames(moteur: &mut Engine, n: usize) {
    for _ in 0..n {
        moteur.frame(0.0);
    }
}

#[test]
fn un_clip_quantifie_part_dans_la_frame_de_son_temps() {
    let Some(gpu) = gpu() else { return };
    let (composition, piste, rouge, _) = une_piste();
    let (mut m, _) = moteur(gpu, composition);
    m.push(Command::SetQuantize(Quantize::Beat));
    frames(&mut m, 5);
    m.push(Command::LaunchClip {
        layer: piste,
        clip: rouge,
    });
    m.frame(0.0);
    let actif = |m: &Engine| m.composition().layer(piste).unwrap().active_clip();
    let mut attente = 0;
    while actif(&m).is_none() {
        assert!(m.composition().is_waiting(piste, rouge));
        m.frame(0.0);
        attente += 1;
        assert!(attente < 60, "le temps n'arrive pas");
    }
    let dans_le_temps = m.composition().beat().played().rem_euclid(1.0);
    assert!(dans_le_temps < 0.05, "{dans_le_temps}");
    m.finish();
}

#[test]
fn un_recalage_rejoint_le_temps_sans_toucher_au_tempo_ni_au_fichier() {
    let Some(gpu) = gpu() else { return };
    let (composition, _, _, _) = une_piste();
    let (mut m, _) = moteur(gpu, composition);
    frames(&mut m, 15);
    m.push(Command::ResyncBeat);
    frames(&mut m, 1);
    assert!(m.composition().beat().error().abs() > 0.2);
    frames(&mut m, 180);
    assert!(m.composition().beat().error().abs() < 0.01);
    assert_eq!(m.bpm(), 120.0);
    assert!(!m.is_dirty());
    m.finish();
}

#[test]
fn ranger_un_noeud_marque_le_fichier_sans_pas_d_annulation() {
    let Some(gpu) = gpu() else { return };
    let (composition, _, rouge, _) = une_piste();
    let (mut m, _) = moteur(gpu, composition);
    let scope = GraphScope::Clip(rouge);
    let node = m.composition().graph_of(scope).unwrap().nodes()[0].id;
    m.push(Command::SetNodePlace {
        scope,
        node,
        place: [12.0, 34.0],
    });
    frames(&mut m, 20);
    assert!(m.is_dirty());
    assert_eq!(m.undo_label(), None);
    m.finish();
}

#[test]
fn annuler_l_ordre_des_effets_le_rend() {
    let Some(gpu) = gpu() else { return };
    let (composition, _, rouge, _) = une_piste();
    let (mut m, _) = moteur(gpu, composition);
    for kind in [EffectKind::Smooth, EffectKind::Deform] {
        m.push(Command::AddEffect { clip: rouge, kind });
    }
    frames(&mut m, 20);
    let noms = |m: &Engine, clip| -> Vec<String> {
        let c = m.composition().clip(clip).unwrap();
        c.effects.iter().map(|e| e.kind().to_string()).collect()
    };
    let avant = noms(&m, rouge);
    m.push(Command::MoveEffect {
        clip: rouge,
        index: 0,
        to: 1,
    });
    frames(&mut m, 20);
    assert_eq!(m.undo_label(), Some("Ordre des effets"));
    assert_ne!(noms(&m, rouge), avant);
    let fait = m.undo().expect("un pas à annuler");
    assert_eq!(noms(&m, fait.map.clip(rouge).unwrap()), avant);
    m.finish();
}

/// Une vidéo d'une seule couleur, fabriquée par ffmpeg. `None` sans lui : le test
/// s'arrête avec succès, comme sans GPU (§24.3).
fn video_unie(nom: &str, couleur: &str) -> Option<std::path::PathBuf> {
    let f = std::env::temp_dir().join(format!("frogenx-{nom}-{}.mp4", std::process::id()));
    let fait = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", &format!("color={couleur}:s=64x64:r=30:d=2")])
        .args(["-c:v", "libx264", "-qp", "10", "-pix_fmt", "yuv420p"])
        .arg(&f)
        .status()
        .is_ok_and(|s| s.success());
    if !fait {
        eprintln!("ffmpeg absent, test sauté (§9.3)");
    }
    fait.then_some(f)
}

fn clip_video(fichier: &std::path::Path) -> Clip {
    Clip::video(VideoParams::new(fichier.display().to_string(), 2.0))
}

/// Une piste, deux clips : la vidéo dans la première case, puis une forme.
fn piste_video(fichier: &std::path::Path) -> (Composition, LayerId, ClipId, ClipId) {
    let (mut composition, piste, rouge, bleu) = une_piste();
    composition.remove_clip(piste, composition.columns()[0]);
    let colonne = composition.columns()[0];
    let video = composition
        .place_clip(piste, colonne, clip_video(fichier))
        .unwrap();
    let _ = rouge;
    (composition, piste, video, bleu)
}

#[test]
fn une_video_se_dessine_a_l_image() {
    let Some(gpu) = gpu() else { return };
    let Some(fichier) = video_unie("rouge", "red") else {
        return;
    };
    let (composition, piste, video, _) = piste_video(&fichier);
    let (mut m, recu) = moteur(gpu, composition);
    m.push(Command::TriggerClip {
        layer: piste,
        clip: video,
    });
    frames(&mut m, 3);
    m.finish();
    let p = recu.dernier_pixel(16, 16);
    assert!(p[0] > 240 && p[1] < 30 && p[2] < 30, "{p:?}");
    assert_eq!(p[3], 255);
    let _ = std::fs::remove_file(fichier);
}

#[test]
fn une_video_introuvable_dit_pourquoi_sans_rien_casser() {
    let Some(gpu) = gpu() else { return };
    let (mut composition, piste, rouge, _) = une_piste();
    composition.remove_clip(piste, composition.columns()[0]);
    let colonne = composition.columns()[0];
    let absente = std::path::Path::new("n-existe-pas.mp4");
    let video = composition
        .place_clip(piste, colonne, clip_video(absente))
        .unwrap();
    let _ = rouge;
    let (mut m, recu) = moteur(gpu, composition);
    m.push(Command::TriggerClip {
        layer: piste,
        clip: video,
    });
    frames(&mut m, 3);
    m.finish();
    assert_eq!(recu.dernier_pixel(16, 16)[3], 0, "la couche reste vide");
    assert!(m.video_error(video).unwrap().contains("n-existe-pas"));
}

#[test]
fn annuler_rend_une_video_sans_la_redecoder() {
    let Some(gpu) = gpu() else { return };
    let Some(fichier) = video_unie("annule", "blue") else {
        return;
    };
    let (composition, piste, video, _) = piste_video(&fichier);
    let (mut m, _) = moteur(gpu, composition);
    m.push(Command::TriggerClip {
        layer: piste,
        clip: video,
    });
    frames(&mut m, 3);
    m.push(Command::AddColumn);
    frames(&mut m, 20);
    let fait = m.undo().expect("un pas à annuler");
    let suit = fait.map.clip(video).unwrap();
    assert_eq!(m.video_error(suit), None);
    frames(&mut m, 2);
    m.finish();
    let _ = std::fs::remove_file(fichier);
}

#[test]
fn annuler_garde_le_temps() {
    let Some(gpu) = gpu() else { return };
    let (composition, _, _, _) = une_piste();
    let (mut m, _) = moteur(gpu, composition);
    m.push(Command::AddColumn);
    frames(&mut m, 20);
    let avant = m.composition().beat().played();
    m.undo().expect("un pas à annuler");
    assert_eq!(m.composition().beat().played(), avant);
    m.finish();
}

/// Un clip rouge joue, un bleu lui succède par `kind`, en 2 temps à 120 BPM : une
/// seconde, soit 60 frames. Rend chaque frame.
fn transition(gpu: Gpu, kind: TransitionKind, apres: usize) -> Vec<CpuFrame> {
    let (composition, piste, rouge, bleu) = une_piste();
    let (mut m, recu) = moteur(gpu, composition);
    m.push(Command::TriggerClip {
        layer: piste,
        clip: rouge,
    });
    m.push(Command::SetClipTransition {
        clip: bleu,
        kind,
        beats: 2.0,
    });
    m.frame(0.0);
    m.push(Command::TriggerClip {
        layer: piste,
        clip: bleu,
    });
    frames(&mut m, apres);
    m.finish();
    let frames = recu.0.lock().unwrap();
    frames.clone()
}

/// Les deux canaux au centre, en linéaire : l'octet est encodé en sRGB.
fn lineaire(px: [u8; 4]) -> (f32, f32) {
    let l = |o: u8| ((o as f32 / 255.0 + 0.055) / 1.055).powf(2.4);
    (l(px[0]), l(px[2]))
}

#[test]
fn un_fondu_passe_d_un_clip_a_l_autre_sans_creux() {
    let Some(gpu) = gpu() else { return };
    let f = transition(gpu, TransitionKind::Fade, 70);
    let centre = |i: usize| pixel(&f[i], SIZE.x / 2, SIZE.y / 2);
    assert_eq!(centre(0)[0], 255, "rouge avant");
    assert_eq!(centre(0)[3], 255);

    let (rouge, bleu) = lineaire(centre(30));
    assert!(
        (rouge - 0.5).abs() < 0.08 && (bleu - 0.5).abs() < 0.08,
        "{rouge} {bleu}"
    );
    assert_eq!(centre(30)[3], 255, "aucun creux à mi-fondu");

    assert_eq!(centre(65)[2], 255, "bleu après");
    assert_eq!(centre(65)[0], 0);
}

#[test]
fn un_volet_decouvre_le_clip_du_cote_voulu() {
    let Some(gpu) = gpu() else { return };
    let dir = TransitionKind::Wipe(WipeDirection::Right);
    let f = transition(gpu, dir, 35);
    // Vers la droite : le bleu entre par la gauche du carré.
    let (gauche, droite) = (
        pixel(&f[30], SIZE.x * 3 / 8, SIZE.y / 2),
        pixel(&f[30], SIZE.x * 5 / 8, SIZE.y / 2),
    );
    assert_eq!((gauche[2], gauche[0]), (255, 0), "{gauche:?}");
    assert_eq!((droite[0], droite[2]), (255, 0), "{droite:?}");
}

#[test]
fn la_fin_d_une_transition_vaut_une_coupure() {
    let Some(gpu) = gpu() else { return };
    let Some(gpu2) = self::gpu() else { return };
    let (a, b) = (
        transition(gpu, TransitionKind::Fade, 70),
        transition(gpu2, TransitionKind::Cut, 70),
    );
    assert_eq!(a.last().unwrap().pixels, b.last().unwrap().pixels);
}

#[test]
fn une_transition_rejoue_a_l_identique() {
    let Some(gpu) = gpu() else { return };
    let Some(gpu2) = self::gpu() else { return };
    let dir = TransitionKind::Wipe(WipeDirection::Up);
    let (a, b) = (transition(gpu, dir, 25), transition(gpu2, dir, 25));
    assert_eq!(a.last().unwrap().pixels, b.last().unwrap().pixels);
}

#[test]
fn le_clip_sortant_continue_de_jouer_pendant_la_transition() {
    let Some(gpu) = gpu() else { return };
    let (composition, piste, rouge, bleu) = une_piste();
    let (mut m, _) = moteur(gpu, composition);
    m.push(Command::TriggerClip {
        layer: piste,
        clip: rouge,
    });
    m.push(Command::SetClipTransition {
        clip: bleu,
        kind: TransitionKind::Fade,
        beats: 2.0,
    });
    m.frame(0.0);
    m.push(Command::TriggerClip {
        layer: piste,
        clip: bleu,
    });
    m.frame(0.0);
    let position = |m: &Engine| {
        let g = m.composition().clip(rouge).unwrap().graph();
        g.nodes()
            .iter()
            .map(|n| g.node_state(n.id).to_vec())
            .collect::<Vec<_>>()
    };
    let avant = position(&m);
    frames(&mut m, 5);
    assert_ne!(position(&m), avant, "il avance encore");
    assert!(m.diagnostics().iter().all(|d| d.is_clean()));
}

#[test]
fn un_echo_peut_entrer_en_fondu() {
    let Some(gpu) = gpu() else { return };
    let (mut c, piste, rouge, _) = une_piste();
    let mut g = Graph::new();
    let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
    let echo = c
        .add_clip(piste, Clip::feedback(g, Default::default(), transport))
        .unwrap();
    let (mut m, recu) = moteur(gpu, c);
    m.push(Command::TriggerClip {
        layer: piste,
        clip: rouge,
    });
    m.push(Command::SetClipTransition {
        clip: echo,
        kind: TransitionKind::Fade,
        beats: 2.0,
    });
    m.frame(0.0);
    m.push(Command::TriggerClip {
        layer: piste,
        clip: echo,
    });
    frames(&mut m, 30);
    m.finish();
    assert_eq!(recu.dernier_pixel(SIZE.x / 2, SIZE.y / 2)[0], 255);
}

#[test]
fn annuler_pendant_une_transition_la_termine_net() {
    let Some(gpu) = gpu() else { return };
    let (composition, piste, rouge, bleu) = une_piste();
    let (mut m, _) = moteur(gpu, composition);
    m.push(Command::TriggerClip {
        layer: piste,
        clip: rouge,
    });
    m.push(Command::SetClipTransition {
        clip: bleu,
        kind: TransitionKind::Fade,
        beats: 8.0,
    });
    frames(&mut m, 20);
    m.push(Command::TriggerClip {
        layer: piste,
        clip: bleu,
    });
    frames(&mut m, 2);
    assert!(m.composition().outgoing_clip(piste).is_some());
    assert_eq!(m.undo().unwrap().label, "Transition");
    let piste = m.composition().all_layers()[0];
    assert_eq!(m.composition().outgoing_clip(piste), None);
    frames(&mut m, 2);
}

/// Un clip dont la lecture est `mode`, 2 temps à 120 BPM : une seconde.
fn lecture(gpu: Gpu, mode: PlayMode) -> (Engine, LayerId, ClipId, Recu) {
    let (composition, piste, rouge, _) = une_piste();
    let (mut m, recu) = moteur(gpu, composition);
    m.push(Command::SetClipPlayback {
        clip: rouge,
        mode,
        beats: 2.0,
    });
    m.push(Command::TriggerClip {
        layer: piste,
        clip: rouge,
    });
    m.frame(0.0);
    (m, piste, rouge, recu)
}

#[test]
fn une_lecture_unique_efface_le_clip_de_l_image_a_sa_fin() {
    let Some(gpu) = gpu() else { return };
    let (mut m, piste, _, recu) = lecture(gpu, PlayMode::Once);
    frames(&mut m, 50);
    assert!(m
        .composition()
        .layer(piste)
        .unwrap()
        .active_clip()
        .is_some());
    frames(&mut m, 15);
    assert!(m
        .composition()
        .layer(piste)
        .unwrap()
        .active_clip()
        .is_none());
    m.finish();
    let frames = recu.0.lock().unwrap();
    let centre = |i: usize| pixel(&frames[i], SIZE.x / 2, SIZE.y / 2);
    assert_eq!(centre(50)[0], 255, "il joue");
    assert_eq!(centre(frames.len() - 1)[3], 0, "il est parti");
}

#[test]
fn un_aller_retour_fait_remonter_le_temps_du_clip() {
    let Some(gpu) = gpu() else { return };
    let (mut m, piste, rouge, _) = lecture(gpu, PlayMode::PingPong);
    let position = |m: &Engine| {
        let clip = m.composition().clip(rouge).unwrap();
        let Transport::BpmSync { position, .. } = clip.transport else {
            panic!("calé sur le tempo");
        };
        clip.graph().value(position)
    };
    frames(&mut m, 70);
    assert!(m.composition().layer(piste).unwrap().is_reversed());
    let (a, _) = (position(&m), frames(&mut m, 10));
    assert!(position(&m) < a, "le temps redescend");
    frames(&mut m, 60);
    assert!(!m.composition().layer(piste).unwrap().is_reversed());
    assert!(position(&m) >= 0.0);
    assert!(m.diagnostics().iter().all(|d| d.is_clean()));
}

#[test]
fn un_clip_du_catalogue_en_aller_retour_repasse_par_la_meme_forme() {
    let Some(gpu) = gpu() else { return };
    let grille = {
        let mut c = Composition::new();
        let deck = c.add_deck("test");
        let groupe = c.add_layer_group(deck).unwrap();
        let piste = c.add_layer(groupe).unwrap();
        let preset = frogenx_library::find("Polygone").and_then(frogenx_library::preset);
        let clip = c.add_clip(piste, preset.unwrap().build()).unwrap();
        (c, piste, clip)
    };
    let (composition, piste, clip) = grille;
    let (mut m, recu) = moteur(gpu, composition);
    m.push(Command::SetClipPlayback {
        clip,
        mode: PlayMode::PingPong,
        beats: 2.0,
    });
    m.push(Command::TriggerClip { layer: piste, clip });
    // Une seconde à l'aller, une au retour : le départ, à une frame près.
    frames(&mut m, 124);
    m.finish();
    let frames = recu.0.lock().unwrap();
    let couverts = |f: &CpuFrame| {
        f.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 0)
            .count()
    };
    let (a, b) = (couverts(&frames[4]), couverts(frames.last().unwrap()));
    let ecart = a.abs_diff(b) as f32 / a as f32;
    assert!(ecart < 0.15, "{a} {b}");
}

#[test]
fn annuler_ne_retourne_pas_un_clip_en_aller_retour() {
    let Some(gpu) = gpu() else { return };
    let (mut m, piste, _, _) = lecture(gpu, PlayMode::PingPong);
    frames(&mut m, 70);
    assert!(m.composition().layer(piste).unwrap().is_reversed());
    m.undo();
    let piste = m.composition().all_layers()[0];
    assert!(m.composition().layer(piste).unwrap().is_reversed());
}

#[test]
fn annuler_rend_le_reglage_d_avant_et_retablir_le_refait() {
    let Some(gpu) = gpu() else { return };
    let mut m = scene(gpu);
    assert_eq!(m.undo_label(), None);
    m.push(Command::SetBpm(90.0));
    frames(&mut m, 2);
    assert_eq!(m.bpm(), 90.0);

    assert_eq!(m.undo().unwrap().label, "Tempo");
    frames(&mut m, 1);
    assert_eq!(m.bpm(), 120.0);
    assert_eq!(m.redo_label(), Some("Tempo"));

    assert_eq!(m.redo().unwrap().label, "Tempo");
    frames(&mut m, 1);
    assert_eq!(m.bpm(), 90.0);
}

#[test]
fn un_potentiometre_tourne_s_annule_en_une_fois() {
    let Some(gpu) = gpu() else { return };
    let mut m = scene(gpu);
    m.set_gesture(true);
    for i in 0..30 {
        m.push(Command::SetBpm(100.0 + i as f32));
        m.frame(0.0);
    }
    m.set_gesture(false);
    frames(&mut m, 20);
    assert!(m.undo().is_some());
    frames(&mut m, 1);
    assert_eq!(m.bpm(), 120.0);
    assert_eq!(m.undo_label(), None, "un seul pas");
}

#[test]
fn deux_gestes_separes_font_deux_pas() {
    let Some(gpu) = gpu() else { return };
    let mut m = scene(gpu);
    m.push(Command::SetBpm(90.0));
    frames(&mut m, 20);
    m.push(Command::AddColumn);
    frames(&mut m, 20);
    assert_eq!(m.undo_label(), Some("Ajout d'une colonne"));
    m.undo();
    assert_eq!(m.undo_label(), Some("Tempo"));
}

#[test]
fn annuler_ferme_d_abord_le_pas_en_cours() {
    let Some(gpu) = gpu() else { return };
    let mut m = scene(gpu);
    m.push(Command::SetBpm(90.0));
    m.frame(0.0);
    assert!(m.undo().is_some(), "sans attendre le calme");
    frames(&mut m, 1);
    assert_eq!(m.bpm(), 120.0);
}

#[test]
fn jouer_ne_cree_pas_de_pas() {
    let Some(gpu) = gpu() else { return };
    let mut m = scene(gpu);
    let colonne = m.composition().columns()[1];
    m.push(Command::TriggerColumn { column: colonne });
    frames(&mut m, 30);
    assert_eq!(m.undo_label(), None);
}

#[test]
fn annuler_ne_change_pas_ce_qui_joue() {
    let Some(gpu) = gpu() else { return };
    let mut m = scene(gpu);
    m.push(Command::SetBpm(90.0));
    frames(&mut m, 20);
    let colonne = m.composition().columns()[1];
    m.push(Command::TriggerColumn { column: colonne });
    frames(&mut m, 5);

    let noms = |m: &Engine| {
        let c = m.composition();
        c.all_layers()
            .iter()
            .filter_map(|l| c.layer(*l)?.active_clip())
            .filter_map(|id| c.clip(id).map(|clip| clip.name.clone()))
            .collect::<Vec<_>>()
    };
    let avant = noms(&m);
    m.undo();
    frames(&mut m, 1);
    assert_eq!(noms(&m), avant);
}

#[test]
fn l_image_ne_saute_pas_quand_on_annule_ailleurs() {
    let Some(gpu) = gpu() else { return };
    let Some(gpu2) = self::gpu() else { return };
    let rendre = |gpu: Gpu, annule: bool| {
        let mut m = Engine::new(gpu, Clock::fixed(60.0), SIZE, frogenx_library::starter());
        let recu = Recu::default();
        m.add_output(OutputSink::Cpu(Box::new(recu.clone())))
            .unwrap();
        let colonne = m.composition().columns()[0];
        m.push(Command::TriggerColumn { column: colonne });
        frames(&mut m, 30);
        // Un tempo inchangé d'une grille à l'autre : seul l'aller-retour diffère.
        m.push(Command::AddColumn);
        frames(&mut m, 20);
        if annule {
            m.undo();
            m.redo();
        }
        frames(&mut m, 10);
        m.finish();
        let frames = recu.0.lock().unwrap();
        frames.last().unwrap().pixels.to_vec()
    };
    assert_eq!(rendre(gpu, false), rendre(gpu2, true));
}

#[test]
fn revenir_a_l_etat_enregistre_retire_le_marqueur() {
    let Some(gpu) = gpu() else { return };
    let mut m = scene(gpu);
    assert!(!m.is_dirty());
    m.push(Command::SetBpm(90.0));
    frames(&mut m, 20);
    assert!(m.is_dirty());
    m.undo();
    assert!(!m.is_dirty());
    m.redo();
    assert!(m.is_dirty());
}

#[test]
fn enregistrer_deplace_l_etat_propre() {
    let Some(gpu) = gpu() else { return };
    let mut m = scene(gpu);
    m.push(Command::SetBpm(90.0));
    frames(&mut m, 2);
    m.set_dirty(false);
    m.undo();
    assert!(m.is_dirty(), "le fichier contient le tempo annulé");
}

#[test]
fn poser_un_clip_s_annule() {
    let Some(gpu) = gpu() else { return };
    let mut m = scene(gpu);
    let (piste, colonne) = (
        m.composition().all_layers()[0],
        m.composition().columns()[1],
    );
    let avant = m.composition().layer(piste).unwrap().cell(1);
    m.place(piste, colonne, carre(Graph::new(), 1.0, 0.0));
    frames(&mut m, 20);
    assert_eq!(m.undo_label(), Some("Pose d'un clip"));
    m.undo();
    let c = m.composition();
    let piste = c.all_layers()[0];
    assert_eq!(c.layer(piste).unwrap().cell(1).is_some(), avant.is_some());
}

#[test]
fn un_cablage_s_annule_et_la_selection_suit() {
    let Some(gpu) = gpu() else { return };
    let mut m = scene(gpu);
    let c = m.composition();
    let piste = c.all_layers()[1];
    let clip = c.layer(piste).unwrap().cell(0).unwrap();
    let avant = c.clip(clip).unwrap().effects.len();
    m.push(Command::AddEffect {
        clip,
        kind: EffectKind::Smooth,
    });
    frames(&mut m, 20);
    assert_eq!(m.composition().clip(clip).unwrap().effects.len(), avant + 1);

    let fait = m.undo().unwrap();
    assert_eq!(fait.label, "Ajout d'un effet");
    let neuf = fait.map.clip(clip).expect("le clip existe toujours");
    assert_eq!(m.composition().clip(neuf).unwrap().effects.len(), avant);
}

#[test]
fn ouvrir_un_fichier_vide_l_historique() {
    let Some(gpu) = gpu() else { return };
    let mut m = scene(gpu);
    m.push(Command::SetBpm(90.0));
    frames(&mut m, 20);
    m.replace_composition(frogenx_library::starter());
    assert_eq!(m.undo_label(), None);
    assert!(m.undo().is_none());
}

#[test]
fn une_grille_qui_ne_s_enregistre_pas_ne_s_annule_pas_et_ne_panique_pas() {
    let Some(gpu) = gpu() else { return };
    let mut c = Composition::new();
    let deck = c.add_deck("test");
    let groupe = c.add_layer_group(deck).unwrap();
    c.add_layer(groupe).unwrap();
    c.graph_mut()
        .add(Box::new(Compteur(Arc::new(AtomicU32::new(0)))));
    let (mut m, _) = moteur(gpu, c);
    m.push(Command::SetBpm(90.0));
    frames(&mut m, 20);
    assert!(m.undo().is_none());
    assert_eq!(m.bpm(), 90.0);
}

#[test]
fn le_temps_reel_sert_une_sortie_cpu_sans_attendre() {
    let Some(gpu) = gpu() else { return };
    let (c, piste, rouge, _) = une_piste();
    let recu = Recu::default();
    let mut moteur = Engine::new(gpu, Clock::realtime(), SIZE, c);
    moteur
        .add_output(OutputSink::Cpu(Box::new(recu.clone())))
        .unwrap();
    moteur.push(Command::TriggerClip {
        layer: piste,
        clip: rouge,
    });
    for _ in 0..4 {
        moteur.frame(0.016);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    moteur.finish();
    assert!((moteur.clock().t() - 0.064).abs() < 1e-9);
    assert_eq!(recu.dernier_pixel(16, 16), [255, 0, 0, 255]);
}

/// Le moteur rend ce flux, après `frames` frames en temps réel.
fn flux_rouge(gpu: Gpu, format: StreamFormat, frames: u32) -> Vec<CpuFrame> {
    let (c, piste, rouge, _) = une_piste();
    let recu = Recu::default();
    let mut moteur = Engine::new(gpu, Clock::fixed(60.0), SIZE * 4, c);
    moteur.add_stream(Box::new(recu.clone()), format);
    moteur.push(Command::TriggerClip {
        layer: piste,
        clip: rouge,
    });
    for _ in 0..frames {
        moteur.frame(0.0);
    }
    moteur.finish();
    let frames = recu.0.lock().unwrap().clone();
    frames
}

#[test]
fn un_flux_recoit_l_image_du_moteur_a_son_format() {
    let Some(gpu) = gpu() else { return };
    let frames = flux_rouge(
        gpu,
        StreamFormat {
            width: 64,
            height: 64,
            fps: 60,
        },
        3,
    );
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].layout, PixelLayout::BgraOpaque);
    assert_eq!((frames[0].width, frames[0].height), (64, 64));
    assert_eq!(
        pixel(&frames[2], 32, 32),
        [0, 0, 255, 255],
        "rouge, en BGRA"
    );
    assert_eq!(
        pixel(&frames[2], 0, 0),
        [0, 0, 0, 255],
        "opaque sur du noir"
    );
}

#[test]
fn un_flux_prend_une_frame_sur_deux_d_un_moteur_a_60() {
    let Some(gpu) = gpu() else { return };
    let frames = flux_rouge(
        gpu,
        StreamFormat {
            width: 64,
            height: 64,
            fps: 30,
        },
        6,
    );
    let indices: Vec<u64> = frames.iter().map(|f| f.index).collect();
    assert_eq!(indices, vec![0, 2, 4]);
}

#[test]
fn changer_de_resolution_rend_comme_un_moteur_neuf() {
    let (Some(premier), Some(second)) = (gpu(), gpu()) else {
        return;
    };
    let rendu = |gpu: Gpu, depart: UVec2, resize: bool| {
        let (c, piste, rouge, _) = une_piste();
        let recu = Recu::default();
        let mut moteur = Engine::new(gpu, Clock::fixed(60.0), depart, c);
        moteur
            .add_output(OutputSink::Cpu(Box::new(recu.clone())))
            .unwrap();
        moteur.push(Command::TriggerClip {
            layer: piste,
            clip: rouge,
        });
        moteur.frame(0.0);
        if resize {
            moteur.set_render_size(SIZE * 2).unwrap();
        }
        moteur.frame(0.0);
        moteur.finish();
        let image = recu.0.lock().unwrap().last().unwrap().clone();
        image
    };
    let change = rendu(premier, SIZE, true);
    let neuf = rendu(second, SIZE * 2, false);
    assert_eq!((change.width, change.height), (64, 64));
    assert_eq!(change.pixels, neuf.pixels);
}

#[test]
fn une_resolution_impossible_est_refusee_et_ne_change_rien() {
    let Some(gpu) = gpu() else { return };
    let (c, ..) = une_piste();
    let mut moteur = Engine::new(gpu, Clock::fixed(60.0), SIZE, c);
    for taille in [UVec2::ZERO, UVec2::new(8, 64), UVec2::new(1 << 20, 64)] {
        assert!(moteur.set_render_size(taille).is_err(), "{taille}");
    }
    assert_eq!(moteur.render_size(), SIZE);
    moteur.frame(0.0);
}

#[test]
fn un_echo_et_une_transition_survivent_a_un_changement_de_resolution() {
    let Some(gpu) = gpu() else { return };
    let (mut c, piste, _, echo) = avec_echo(FeedbackParams::default());
    let bleu = c.add_clip(piste, carre(Graph::new(), 0.0, 1.0)).unwrap();
    assert!(c
        .clip_mut(bleu)
        .unwrap()
        .set_transition(TransitionKind::Fade, 4.0));
    let (mut moteur, recu) = moteur(gpu, c);
    moteur.push(Command::TriggerClip {
        layer: piste,
        clip: echo,
    });
    moteur.frame(0.0);
    moteur.push(Command::TriggerClip {
        layer: piste,
        clip: bleu,
    });
    moteur.frame(0.0);
    moteur.set_render_size(SIZE * 2).unwrap();
    for _ in 0..3 {
        moteur.frame(0.0);
    }
    moteur.finish();
    assert_eq!(recu.0.lock().unwrap().last().unwrap().width, 64);
}

#[test]
fn un_flux_suit_un_rendu_qui_passe_sous_sa_taille() {
    let Some(gpu) = gpu() else { return };
    let (c, ..) = une_piste();
    let recu = Recu::default();
    let mut moteur = Engine::new(gpu, Clock::fixed(60.0), SIZE * 2, c);
    moteur.add_stream(
        Box::new(recu.clone()),
        StreamFormat {
            width: 64,
            height: 64,
            fps: 60,
        },
    );
    moteur.frame(0.0);
    moteur.set_render_size(SIZE).unwrap();
    moteur.frame(0.0);
    moteur.finish();
    let tailles: Vec<u32> = recu.0.lock().unwrap().iter().map(|f| f.width).collect();
    assert_eq!(tailles, vec![64, 32]);
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
