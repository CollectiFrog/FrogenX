//! Tests de pixels — §15.1 (« Couches »), §24.3 de `docs/ARCHITECTURE.md`.
//!
//! Rendu réel sur le GPU, relu et comparé en quelques points précis à des
//! valeurs calculées sans GPU (`srgb_to_linear`, `blend_premul`). Sans
//! adaptateur, chaque test s'arrête **avec succès** et le dit : un test qui
//! échoue faute de GPU finit désactivé, donc inutile.

use frogenx_core::{AudioFeatures, Constant, Ctx, Graph, Param};
use frogenx_layer::{
    srgb_to_linear, BlendMode, BlendParams, ColorParam, Gpu, RenderLayer, ShapeLayer,
};
use frogenx_render::{blend_premul, read_rgba16f, Compositor, LayerInput};
use frogenx_shape::{Arc, Chain, Ellipse, Rect, Transform};
use glam::UVec2;

/// Tolérance des comparaisons : les textures intermédiaires sont en
/// demi-flottants (≈ 3 chiffres significatifs).
const TOL: f32 = 3e-3;

fn gpu() -> Option<Gpu> {
    let gpu = Gpu::headless();
    if gpu.is_none() {
        eprintln!("GPU absent, test sauté (§24.3)");
    }
    gpu
}

/// Compose les couches (de bas en haut) et relit l'image finale.
fn composer(gpu: &Gpu, size: UVec2, couches: &mut [ShapeLayer], g: &Graph) -> Vec<[f32; 4]> {
    let audio = AudioFeatures::default();
    let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
    let mut comp = Compositor::new(gpu, size);
    let mut inputs: Vec<LayerInput> = couches
        .iter_mut()
        .map(|c| LayerInput { layer: c, graph: g })
        .collect();
    comp.render(gpu, 0.0, &ctx, &mut inputs);
    read_rgba16f(gpu, comp.output_texture()).expect("relecture GPU")
}

fn px(pixels: &[[f32; 4]], size: UVec2, x: u32, y: u32) -> [f32; 4] {
    pixels[(y * size.x + x) as usize]
}

fn proche(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < TOL)
}

const VIDE: [f32; 4] = [0.0; 4];

#[test]
fn un_carre_plein_au_centre_a_la_couleur_lineaire_premultipliee() {
    let Some(gpu) = gpu() else { return };
    let size = UVec2::new(64, 64);
    let g = Graph::new();
    // Un carré de côté 1 dans l'espace normalisé [-1, 1] : la moitié centrale.
    let mut couche = [ShapeLayer::new(&gpu, size, Box::new(Rect::new(1.0, 1.0)))
        .with_fill(ColorParam::srgb(1.0, 0.5, 0.0, 1.0))];
    let image = composer(&gpu, size, &mut couche, &g);

    // Le vert sRGB 0.5 doit arriver en linéaire (≈ 0.214), pas à 0.5 (§21).
    let attendu = [1.0, srgb_to_linear(0.5), 0.0, 1.0];
    let centre = px(&image, size, 32, 32);
    assert!(
        proche(centre, attendu),
        "centre {centre:?}, attendu {attendu:?}"
    );
    assert!(proche(px(&image, size, 20, 44), attendu), "intérieur");
    assert_eq!(px(&image, size, 1, 1), VIDE, "le coin reste transparent");
    assert_eq!(px(&image, size, 8, 32), VIDE, "hors du carré");
}

#[test]
fn le_haut_de_lespace_normalise_est_le_haut_de_limage() {
    // Convention : y croît vers le haut dans l'espace des formes (§17), et
    // la ligne 0 d'une image relue est en haut.
    let Some(gpu) = gpu() else { return };
    let size = UVec2::new(64, 64);
    let g = Graph::new();
    let forme = Chain::new(Box::new(Rect::new(0.4, 0.4))).then(Box::new(Transform {
        translate_y: Param::fixed(0.5),
        ..Default::default()
    }));
    let mut couche = [ShapeLayer::new(&gpu, size, Box::new(forme))
        .with_fill(ColorParam::srgb(1.0, 1.0, 1.0, 1.0))];
    let image = composer(&gpu, size, &mut couche, &g);
    assert_ne!(px(&image, size, 32, 16), VIDE, "la forme est en haut");
    assert_eq!(px(&image, size, 32, 48), VIDE, "et pas en bas");
}

#[test]
fn un_cercle_reste_rond_sur_une_cible_en_paysage() {
    // §17 : [-1, 1] sur le petit côté. Sur 128×64, le cercle de rayon 1 fait
    // 32 px de rayon dans les deux directions — sans correction, il en
    // ferait 64 en largeur.
    let Some(gpu) = gpu() else { return };
    let size = UVec2::new(128, 64);
    let g = Graph::new();
    let mut couche = [ShapeLayer::new(&gpu, size, Box::new(Ellipse::circle(1.0)))
        .with_fill(ColorParam::srgb(1.0, 1.0, 1.0, 1.0))];
    let image = composer(&gpu, size, &mut couche, &g);
    assert_ne!(
        px(&image, size, 64, 6),
        VIDE,
        "près du haut, dans le cercle"
    );
    assert_ne!(px(&image, size, 90, 32), VIDE, "à 26 px du centre : dedans");
    assert_eq!(
        px(&image, size, 100, 32),
        VIDE,
        "à 36 px du centre : dehors"
    );
}

#[test]
fn chaque_mode_de_melange_du_gpu_suit_la_reference_cpu() {
    // Le shader du compositeur et `blend_premul` doivent dire la même chose,
    // mode par mode — sinon l'un des deux ment.
    let Some(gpu) = gpu() else { return };
    let size = UVec2::new(16, 16);
    let g = Graph::new();
    let a = ColorParam::srgb(0.2, 0.6, 0.9, 1.0);
    let b = ColorParam::srgb(0.9, 0.3, 0.1, 0.8);
    let opacite = 0.7;

    for mode in BlendMode::ALL {
        let mut couches = [
            ShapeLayer::new(&gpu, size, Box::new(Rect::new(4.0, 4.0))).with_fill(a),
            ShapeLayer::new(&gpu, size, Box::new(Rect::new(4.0, 4.0)))
                .with_fill(b)
                .with_blend(BlendParams {
                    opacity: Param::fixed(opacite),
                    mode,
                }),
        ];
        let image = composer(&gpu, size, &mut couches, &g);
        let fond = blend_premul(BlendMode::Normal, VIDE, a.resolve(&g), 1.0);
        let attendu = blend_premul(mode, fond, b.resolve(&g), opacite);
        let obtenu = px(&image, size, 8, 8);
        assert!(
            proche(obtenu, attendu),
            "{mode:?} : GPU {obtenu:?}, CPU {attendu:?}"
        );
    }
}

#[test]
fn lopacite_dune_couche_est_un_param_modulable() {
    // C'est par ce Param que le CrossFader agit (§3bis.4).
    let Some(gpu) = gpu() else { return };
    let size = UVec2::new(16, 16);
    let mut g = Graph::new();
    let src = g.add(Box::new(Constant(0.5)));
    g.eval_frame(0.0, 1.0 / 60.0);
    let mut couche = [ShapeLayer::new(&gpu, size, Box::new(Rect::new(4.0, 4.0)))
        .with_fill(ColorParam::srgb(1.0, 1.0, 1.0, 1.0))
        .with_blend(BlendParams {
            opacity: Param::modulated(0.0, 1.0, src),
            mode: BlendMode::Normal,
        })];
    let image = composer(&gpu, size, &mut couche, &g);
    assert!(proche(px(&image, size, 8, 8), [0.5, 0.5, 0.5, 0.5]));
}

#[test]
fn un_contour_ouvert_nest_pas_rempli_mais_peut_etre_trace() {
    let Some(gpu) = gpu() else { return };
    let size = UVec2::new(64, 64);
    let g = Graph::new();
    let demi_cercle = || Box::new(Arc::new(0.0, std::f32::consts::PI, 0.8));

    let mut rempli =
        [ShapeLayer::new(&gpu, size, demi_cercle())
            .with_fill(ColorParam::srgb(1.0, 1.0, 1.0, 1.0))];
    let image = composer(&gpu, size, &mut rempli, &g);
    assert!(
        image.iter().all(|p| *p == VIDE),
        "un arc n'a pas d'intérieur"
    );

    let mut trace = [ShapeLayer::new(&gpu, size, demi_cercle())
        .with_stroke(Param::fixed(0.1), ColorParam::srgb(1.0, 1.0, 1.0, 1.0))];
    let image = composer(&gpu, size, &mut trace, &g);
    // Sommet de l'arc : (0, 0.8) → pixel (32, 6.4).
    assert_ne!(px(&image, size, 32, 6), VIDE, "le trait passe au sommet");
    assert_eq!(px(&image, size, 32, 32), VIDE, "le centre reste vide");
}

#[test]
fn une_forme_degeneree_rend_une_image_transparente_sans_paniquer() {
    let Some(gpu) = gpu() else { return };
    let size = UVec2::new(16, 16);
    let g = Graph::new();
    let mut couche = [
        ShapeLayer::new(&gpu, size, Box::new(Ellipse::circle(f32::NAN)))
            .with_fill(ColorParam::srgb(1.0, 1.0, 1.0, 1.0))
            .with_stroke(Param::fixed(f32::NAN), ColorParam::srgb(1.0, 1.0, 1.0, 1.0)),
    ];
    let image = composer(&gpu, size, &mut couche, &g);
    assert!(image.iter().all(|p| p.iter().all(|c| c.is_finite())));
    assert_eq!(couche[0].diagnostics().tessellation_failures, 0);
}

#[test]
fn sans_couche_la_sortie_est_transparente() {
    let Some(gpu) = gpu() else { return };
    let size = UVec2::new(8, 8);
    let image = composer(&gpu, size, &mut [], &Graph::new());
    assert!(image.iter().all(|p| *p == VIDE));
}

#[test]
fn une_couche_videe_est_effacee_a_la_frame_suivante() {
    // Une couche qui ne dessine plus rien doit sortir transparente, pas
    // garder l'image de la frame précédente.
    let Some(gpu) = gpu() else { return };
    let size = UVec2::new(16, 16);
    let g = Graph::new();
    let audio = AudioFeatures::default();
    let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
    let mut couche = ShapeLayer::new(&gpu, size, Box::new(Rect::new(4.0, 4.0)))
        .with_fill(ColorParam::srgb(1.0, 1.0, 1.0, 1.0));
    let mut comp = Compositor::new(&gpu, size);

    comp.render(
        &gpu,
        0.0,
        &ctx,
        &mut [LayerInput {
            layer: &mut couche,
            graph: &g,
        }],
    );
    let avant = read_rgba16f(&gpu, comp.output_texture()).unwrap();
    assert_ne!(px(&avant, size, 8, 8), VIDE);

    couche.fill = None;
    comp.render(
        &gpu,
        1.0 / 60.0,
        &ctx,
        &mut [LayerInput {
            layer: &mut couche,
            graph: &g,
        }],
    );
    let apres = read_rgba16f(&gpu, comp.output_texture()).unwrap();
    assert!(apres.iter().all(|p| *p == VIDE));
}

#[test]
fn le_redimensionnement_change_la_taille_de_la_sortie() {
    let Some(gpu) = gpu() else { return };
    let g = Graph::new();
    let audio = AudioFeatures::default();
    let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
    let mut couche = ShapeLayer::new(&gpu, UVec2::new(16, 16), Box::new(Ellipse::circle(0.5)))
        .with_fill(ColorParam::srgb(1.0, 1.0, 1.0, 1.0));
    let mut comp = Compositor::new(&gpu, UVec2::new(16, 16));

    let nouvelle = UVec2::new(40, 24);
    couche.resize(nouvelle, &gpu);
    comp.resize(&gpu, nouvelle);
    comp.render(
        &gpu,
        0.0,
        &ctx,
        &mut [LayerInput {
            layer: &mut couche,
            graph: &g,
        }],
    );
    let image = read_rgba16f(&gpu, comp.output_texture()).unwrap();
    assert_eq!(image.len(), 40 * 24);
    assert_ne!(
        px(&image, nouvelle, 20, 12),
        VIDE,
        "le cercle est au centre"
    );
}
