//! L'étage de sortie sur un vrai GPU — §15.1 (« Sorties »), §24.3 de
//! `docs/ARCHITECTURE.md`.
//!
//! Sans adaptateur, chaque test s'arrête **avec succès** et le dit. La politique
//! de perte, elle, se teste sans GPU dans `fanout`.

use frogenx_layer::{Gpu, LAYER_FORMAT};
use frogenx_output::{
    letterbox, ConversionMode, ConversionPass, CpuFrame, CpuSink, DropPolicy, GpuFrame, GpuSink,
    MonitorSink, OutputSink, Outputs, PixelLayout, SinkStatus, StreamFormat,
};
use frogenx_render::{f16_to_f32, linear_premul_to_srgb8, read_rgba8};
use std::sync::{Arc, Mutex};

fn gpu() -> Option<Gpu> {
    let gpu = Gpu::headless();
    if gpu.is_none() {
        eprintln!("GPU absent, test sauté (§24.3)");
    }
    gpu
}

// Demi-flottants exacts, pour des sources sans erreur d'arrondi.
const F0: u16 = 0x0000;
const F025: u16 = 0x3400;
const F05: u16 = 0x3800;
const F1: u16 = 0x3c00;
const F2: u16 = 0x4000;

/// Une texture source telle que la rendrait le compositeur.
fn source(gpu: &Gpu, width: u32, pixels: &[[u16; 4]]) -> wgpu::Texture {
    let height = pixels.len() as u32 / width;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test.source"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: LAYER_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let octets: Vec<u8> = pixels
        .iter()
        .flatten()
        .flat_map(|c| c.to_le_bytes())
        .collect();
    gpu.queue.write_texture(
        texture.as_image_copy(),
        &octets,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 8),
            rows_per_image: Some(height),
        },
        texture.size(),
    );
    texture
}

fn convertir(
    gpu: &Gpu,
    source: &wgpu::Texture,
    format: wgpu::TextureFormat,
    size: wgpu::Extent3d,
    mode: ConversionMode,
) -> Option<Vec<u8>> {
    let cible = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test.cible"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let passe = ConversionPass::new(gpu, format, mode);
    let viewport = (mode == ConversionMode::OverBlack)
        .then(|| letterbox((source.width(), source.height()), (size.width, size.height)));
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    let vue = cible.create_view(&Default::default());
    passe.encode(gpu, &mut encoder, source, &vue, viewport);
    gpu.queue.submit(std::iter::once(encoder.finish()));
    read_rgba8(gpu, &cible)
}

#[test]
fn la_passe_suit_sa_reference_cpu_quel_que_soit_le_format_de_la_cible() {
    let Some(gpu) = gpu() else { return };
    let pixels: [[u16; 4]; 6] = [
        [F05, F025, F0, F1],   // opaque
        [F025, F025, F0, F05], // semi-transparent : dé-prémultiplié en linéaire
        [F2, F0, F1, F1],      // hors domaine : un mode Add peut dépasser 1
        [F0, F0, F0, F0],      // transparent : jamais de division par zéro
        [F1, F1, F1, F1],
        [F025, F025, F025, F1],
    ];
    let texture = source(&gpu, 3, &pixels);
    let attendu: Vec<u8> = pixels
        .iter()
        .flat_map(|p| linear_premul_to_srgb8(p.map(f16_to_f32)))
        .collect();

    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ] {
        let image = convertir(
            &gpu,
            &texture,
            format,
            texture.size(),
            ConversionMode::Straight,
        )
        .expect("relecture GPU");
        assert_eq!(image.len(), attendu.len());
        for (i, (a, b)) in image.iter().zip(&attendu).enumerate() {
            // Un écart de 1 est l'arrondi du matériel, pas une double conversion.
            assert!(
                a.abs_diff(*b) <= 1,
                "{format:?}, octet {i} : got {image:?}, want {attendu:?}"
            );
        }
    }
}

/// Le pixel `(x, y)` d'une image RGBA8 de largeur `w`.
fn pixel(image: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = 4 * (y * w + x) as usize;
    [image[i], image[i + 1], image[i + 2], image[i + 3]]
}

#[test]
fn une_cible_opaque_garde_les_proportions_sur_du_noir() {
    let Some(gpu) = gpu() else { return };
    // Source 2×1, cible carrée : l'image occupe la moitié centrale en hauteur.
    let rouge = [F1, F0, F0, F1];
    let texture = source(&gpu, 2, &[rouge, rouge]);
    let taille = wgpu::Extent3d {
        width: 16,
        height: 16,
        depth_or_array_layers: 1,
    };
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ] {
        let image = convertir(&gpu, &texture, format, taille, ConversionMode::OverBlack)
            .expect("relecture GPU");
        assert_eq!(
            pixel(&image, 16, 8, 8),
            [255, 0, 0, 255],
            "centre, {format:?}"
        );
        assert_eq!(
            pixel(&image, 16, 8, 1),
            [0, 0, 0, 255],
            "bande haute, {format:?}"
        );
        assert_eq!(
            pixel(&image, 16, 8, 14),
            [0, 0, 0, 255],
            "bande basse, {format:?}"
        );
    }
}

#[test]
fn sur_une_cible_opaque_le_transparent_se_lit_sur_du_noir() {
    let Some(gpu) = gpu() else { return };
    // Rouge à 50 % : prémultiplié, il reste du rouge linéaire 0.5 sur du noir.
    let texture = source(&gpu, 1, &[[F05, F0, F0, F05]]);
    let image = convertir(
        &gpu,
        &texture,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        texture.size(),
        ConversionMode::OverBlack,
    )
    .expect("relecture GPU");
    let attendu = linear_premul_to_srgb8([0.5, 0.0, 0.0, 1.0]);
    assert!(
        pixel(&image, 1, 0, 0)
            .iter()
            .zip(&attendu)
            .all(|(a, b)| a.abs_diff(*b) <= 1),
        "got {image:?}, want {attendu:?}"
    );
}

/// Garde les frames reçues, y compris leur `Arc`, pour comparer les sorties.
#[derive(Clone, Default)]
struct Recu(Arc<Mutex<Vec<CpuFrame>>>);

impl CpuSink for Recu {
    fn submit(&mut self, frame: &CpuFrame) {
        self.0.lock().unwrap().push(frame.clone());
    }

    fn status(&self) -> SinkStatus {
        SinkStatus::Active
    }
}

/// Note l'ordre des appels. `fail` la met en échec dès le départ.
struct JournalGpu {
    journal: Arc<Mutex<Vec<String>>>,
    fail: bool,
}

impl GpuSink for JournalGpu {
    fn encode(&mut self, _: &Gpu, _: &mut wgpu::CommandEncoder, frame: &GpuFrame<'_>) {
        self.journal
            .lock()
            .unwrap()
            .push(format!("encode {}", frame.index));
    }

    fn after_submit(&mut self, _: &Gpu) {
        self.journal.lock().unwrap().push("after_submit".into());
    }

    fn status(&self) -> SinkStatus {
        if self.fail {
            SinkStatus::Failed("surface refusée".into())
        } else {
            SinkStatus::Active
        }
    }
}

#[test]
fn le_monitor_recoit_la_frame_encodee_en_srgb_et_en_proportions() {
    let Some(gpu) = gpu() else { return };
    let monitor = MonitorSink::new(&gpu, 8, 8);
    let texture = monitor.texture();
    assert_eq!(
        texture.format(),
        wgpu::TextureFormat::Rgba8Unorm,
        "imposé par egui"
    );

    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossless);
    sorties.add(OutputSink::Gpu(Box::new(monitor))).unwrap();
    // Source 2×1 : l'image occupe la moitié centrale du monitor.
    let gris = [F025, F025, F025, F1];
    sorties.submit(&gpu, &source(&gpu, 2, &[gris, gris]), 0.0);

    let image = read_rgba8(&gpu, &texture).expect("relecture GPU");
    let attendu = linear_premul_to_srgb8([0.25, 0.25, 0.25, 1.0]);
    let centre = pixel(&image, 8, 4, 4);
    assert!(
        centre
            .iter()
            .zip(&attendu)
            .all(|(a, b)| a.abs_diff(*b) <= 1),
        "centre {centre:?}, attendu {attendu:?}"
    );
    assert_eq!(pixel(&image, 8, 4, 0), [0, 0, 0, 255], "bande haute");
    assert_eq!(sorties.statuses(), vec![SinkStatus::Active]);
}

#[test]
fn une_seule_relecture_est_partagee_par_toutes_les_sorties_cpu() {
    let Some(gpu) = gpu() else { return };
    let (a, b) = (Recu::default(), Recu::default());
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossless);
    sorties.add(OutputSink::Cpu(Box::new(a.clone()))).unwrap();
    sorties.add(OutputSink::Cpu(Box::new(b.clone()))).unwrap();

    let texture = source(&gpu, 2, &[[F1, F0, F0, F1], [F0, F0, F1, F1]]);
    for i in 0..3 {
        sorties.submit(&gpu, &texture, i as f64 / 60.0);
    }
    sorties.finish();

    let (a, b) = (a.0.lock().unwrap(), b.0.lock().unwrap());
    assert_eq!(a.len(), 3);
    assert_eq!(b.len(), 3);
    for (i, (fa, fb)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(fa.index, i as u64);
        assert_eq!(fa.t, i as f64 / 60.0);
        assert!(
            Arc::ptr_eq(&fa.pixels, &fb.pixels),
            "frame {i} : deux relectures"
        );
        assert_eq!(&*fa.pixels, &[255, 0, 0, 255, 0, 0, 255, 255]);
    }
}

#[test]
fn la_cible_de_relecture_suit_la_resolution() {
    let Some(gpu) = gpu() else { return };
    let recu = Recu::default();
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossless);
    sorties
        .add(OutputSink::Cpu(Box::new(recu.clone())))
        .unwrap();

    sorties.submit(&gpu, &source(&gpu, 1, &[[F1, F1, F1, F1]]), 0.0);
    sorties.submit(&gpu, &source(&gpu, 2, &[[F1, F1, F1, F1]; 4]), 0.0);
    sorties.finish();

    let recu = recu.0.lock().unwrap();
    assert_eq!((recu[0].width, recu[0].height), (1, 1));
    assert_eq!((recu[1].width, recu[1].height), (2, 2));
    assert_eq!(recu[1].pixels.len(), 16);
}

#[test]
fn une_sortie_gpu_presente_apres_la_soumission_une_fois_par_frame() {
    let Some(gpu) = gpu() else { return };
    let journal = Arc::new(Mutex::new(Vec::new()));
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossless);
    sorties
        .add(OutputSink::Gpu(Box::new(JournalGpu {
            journal: Arc::clone(&journal),
            fail: false,
        })))
        .unwrap();

    let texture = source(&gpu, 1, &[[F1, F1, F1, F1]]);
    sorties.submit(&gpu, &texture, 0.0);
    sorties.submit(&gpu, &texture, 0.0);

    assert_eq!(
        *journal.lock().unwrap(),
        vec!["encode 0", "after_submit", "encode 1", "after_submit"]
    );
    assert_eq!(sorties.statuses(), vec![SinkStatus::Active]);
}

#[test]
fn une_sortie_gpu_en_echec_n_est_plus_appelee_mais_reste_listee() {
    let Some(gpu) = gpu() else { return };
    let journal = Arc::new(Mutex::new(Vec::new()));
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossless);
    sorties
        .add(OutputSink::Gpu(Box::new(JournalGpu {
            journal: Arc::clone(&journal),
            fail: true,
        })))
        .unwrap();

    sorties.submit(&gpu, &source(&gpu, 1, &[[F1, F1, F1, F1]]), 0.0);

    assert!(journal.lock().unwrap().is_empty());
    assert_eq!(
        sorties.statuses(),
        vec![SinkStatus::Failed("surface refusée".into())]
    );
}

#[test]
fn une_sortie_gpu_retiree_n_est_plus_appelee_et_les_autres_continuent() {
    let Some(gpu) = gpu() else { return };
    let (retiree, gardee) = (
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Mutex::new(Vec::new())),
    );
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossless);
    let id = sorties.add_gpu(Box::new(JournalGpu {
        journal: Arc::clone(&retiree),
        fail: false,
    }));
    sorties.add_gpu(Box::new(JournalGpu {
        journal: Arc::clone(&gardee),
        fail: false,
    }));

    let texture = source(&gpu, 1, &[[F1, F1, F1, F1]]);
    sorties.submit(&gpu, &texture, 0.0);
    assert!(sorties.remove_gpu(id));
    sorties.submit(&gpu, &texture, 0.0);

    assert_eq!(*retiree.lock().unwrap(), vec!["encode 0", "after_submit"]);
    assert_eq!(
        *gardee.lock().unwrap(),
        vec!["encode 0", "after_submit", "encode 1", "after_submit"],
        "la numérotation ne saute pas"
    );
    assert_eq!(sorties.statuses(), vec![SinkStatus::Active]);
}

#[test]
fn retirer_deux_fois_une_sortie_gpu_ne_retire_rien_d_autre() {
    let Some(gpu) = gpu() else { return };
    let journal = Arc::new(Mutex::new(Vec::new()));
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossless);
    let id = sorties.add_gpu(Box::new(JournalGpu {
        journal: Arc::clone(&journal),
        fail: false,
    }));
    assert!(sorties.remove_gpu(id));
    assert!(!sorties.remove_gpu(id));

    // Un identifiant ne resert jamais : la nouvelle sortie n'est pas visée.
    sorties.add_gpu(Box::new(JournalGpu {
        journal,
        fail: false,
    }));
    assert!(!sorties.remove_gpu(id));
    assert_eq!(sorties.statuses(), vec![SinkStatus::Active]);
}

#[test]
fn une_sortie_cpu_est_servie_en_temps_reel() {
    let Some(gpu) = gpu() else { return };
    let image = plein(&gpu, 4, 4, [F1, F0, F0, F1]);
    let recu = Recu::default();
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossy);
    sorties
        .add(OutputSink::Cpu(Box::new(recu.clone())))
        .expect("le temps réel accepte les sorties CPU (§8.5)");
    jouer(&gpu, &mut sorties, &image, 0..6);
    let frames = recu.0.lock().unwrap();
    assert_eq!(frames.len(), 6, "rien n'est perdu quand la sortie suit");
    assert!(frames.windows(2).all(|p| p[0].index < p[1].index));
    assert_eq!(frames[0].layout, PixelLayout::RgbaStraight);
    assert_eq!(&frames[0].pixels[..4], &[255, 0, 0, 255]);
}

/// Une image unie, telle que la rendrait le compositeur.
fn plein(gpu: &Gpu, width: u32, height: u32, couleur: [u16; 4]) -> wgpu::Texture {
    source(gpu, width, &vec![couleur; (width * height) as usize])
}

/// Joue ces frames à 60 fps, et attend que les sorties aient tout reçu.
fn jouer(gpu: &Gpu, sorties: &mut Outputs, image: &wgpu::Texture, frames: std::ops::Range<u32>) {
    for i in frames {
        sorties.submit(gpu, image, i as f64 / 60.0);
        // Laisse le thread de relecture rendre son buffer : l'anneau n'est pas
        // le sujet de ces tests.
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    sorties.finish();
}

fn format(width: u32, height: u32, fps: u32) -> StreamFormat {
    StreamFormat { width, height, fps }
}

#[test]
fn un_flux_recoit_une_image_opaque_rangee_en_bgra() {
    let Some(gpu) = gpu() else { return };
    // Du rouge à moitié transparent : sur du noir, il reste le RGB prémultiplié.
    let image = plein(&gpu, 32, 32, [F05, F0, F0, F05]);
    let recu = Recu::default();
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossy);
    sorties.add_stream(Box::new(recu.clone()), format(32, 32, 60));
    jouer(&gpu, &mut sorties, &image, 0..1);

    let frames = recu.0.lock().unwrap();
    assert_eq!(frames[0].layout, PixelLayout::BgraOpaque);
    assert_eq!(&frames[0].pixels[..4], &[0, 0, 188, 255], "0,5 linéaire");
}

#[test]
fn un_flux_est_reduit_et_garde_les_proportions_du_rendu() {
    let Some(gpu) = gpu() else { return };
    let image = plein(&gpu, 64, 64, [F1, F1, F1, F1]);
    let recu = Recu::default();
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossy);
    sorties.add_stream(Box::new(recu.clone()), format(32, 16, 60));
    jouer(&gpu, &mut sorties, &image, 0..1);

    let frames = recu.0.lock().unwrap();
    assert_eq!((frames[0].width, frames[0].height), (32, 16));
    assert_eq!(frames[0].pixels.len(), 32 * 16 * 4);
    assert_eq!(pixel(&frames[0].pixels, 32, 1, 8), [0, 0, 0, 255], "bande");
    assert_eq!(pixel(&frames[0].pixels, 32, 16, 8), [255, 255, 255, 255]);
}

#[test]
fn un_rembourrage_de_ligne_ne_decale_pas_l_image() {
    let Some(gpu) = gpu() else { return };
    // 22 pixels : 88 octets par ligne, loin de l'alignement de 256.
    let image = plein(&gpu, 22, 22, [F1, F0, F0, F1]);
    let recu = Recu::default();
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossy);
    sorties.add_stream(Box::new(recu.clone()), format(22, 22, 60));
    jouer(&gpu, &mut sorties, &image, 0..1);
    let frames = recu.0.lock().unwrap();
    assert_eq!(frames[0].pixels.len(), 22 * 22 * 4);
    assert!(frames[0]
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .all(|p| *p == [0, 0, 255, 255]));
}

#[test]
fn deux_flux_de_meme_format_partagent_leurs_octets() {
    let Some(gpu) = gpu() else { return };
    let image = plein(&gpu, 32, 32, [F1, F1, F1, F1]);
    let (a, b) = (Recu::default(), Recu::default());
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossy);
    sorties.add_stream(Box::new(a.clone()), format(32, 32, 60));
    sorties.add_stream(Box::new(b.clone()), format(32, 32, 60));
    jouer(&gpu, &mut sorties, &image, 0..1);
    let (a, b) = (a.0.lock().unwrap(), b.0.lock().unwrap());
    assert!(Arc::ptr_eq(&a[0].pixels, &b[0].pixels));
}

#[test]
fn deux_formats_recoivent_chacun_leur_image() {
    let Some(gpu) = gpu() else { return };
    let image = plein(&gpu, 64, 64, [F1, F1, F1, F1]);
    let (a, b) = (Recu::default(), Recu::default());
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossy);
    sorties.add_stream(Box::new(a.clone()), format(64, 64, 60));
    sorties.add_stream(Box::new(b.clone()), format(32, 32, 60));
    jouer(&gpu, &mut sorties, &image, 0..2);
    assert_eq!(a.0.lock().unwrap()[1].width, 64);
    assert_eq!(b.0.lock().unwrap()[1].width, 32);
}

#[test]
fn un_flux_change_de_taille_sans_s_arreter() {
    let Some(gpu) = gpu() else { return };
    let image = plein(&gpu, 64, 64, [F1, F1, F1, F1]);
    let recu = Recu::default();
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossy);
    let id = sorties.add_stream(Box::new(recu.clone()), format(64, 64, 60));
    for i in 0..4 {
        if i == 2 {
            assert!(sorties.set_stream_format(id, format(32, 32, 60)));
        }
        sorties.submit(&gpu, &image, i as f64 / 60.0);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    sorties.finish();
    let tailles: Vec<u32> = recu.0.lock().unwrap().iter().map(|f| f.width).collect();
    assert_eq!(tailles, vec![64, 64, 32, 32]);
}

#[test]
fn la_cadence_d_un_flux_espace_ses_frames_sans_ralentir_le_reste() {
    let Some(gpu) = gpu() else { return };
    let image = plein(&gpu, 32, 32, [F1, F1, F1, F1]);
    let (lent, plein_regime) = (Recu::default(), Recu::default());
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossy);
    sorties.add_stream(Box::new(lent.clone()), format(32, 32, 30));
    sorties.add_stream(Box::new(plein_regime.clone()), format(16, 16, 60));
    jouer(&gpu, &mut sorties, &image, 0..8);
    let indices = |r: &Recu| -> Vec<u64> { r.0.lock().unwrap().iter().map(|f| f.index).collect() };
    assert_eq!(indices(&lent), vec![0, 2, 4, 6]);
    assert_eq!(indices(&plein_regime).len(), 8);
}

#[test]
fn un_flux_retire_n_est_plus_servi_et_les_autres_continuent() {
    let Some(gpu) = gpu() else { return };
    let image = plein(&gpu, 32, 32, [F1, F1, F1, F1]);
    let (parti, reste) = (Recu::default(), Recu::default());
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossy);
    let id = sorties.add_stream(Box::new(parti.clone()), format(32, 32, 60));
    sorties.add_stream(Box::new(reste.clone()), format(32, 32, 60));
    sorties.submit(&gpu, &image, 0.0);
    assert!(sorties.remove_stream(id));
    assert!(!sorties.remove_stream(id));
    assert_eq!(sorties.stream_status(id), None);
    assert_eq!(parti.0.lock().unwrap().len(), 1, "sa dernière frame arrive");
    jouer(&gpu, &mut sorties, &image, 1..3);
    assert_eq!(parti.0.lock().unwrap().len(), 1);
    assert_eq!(reste.0.lock().unwrap().len(), 3);
}

/// Une sortie qui ne rend jamais la main tant qu'on ne la libère pas.
struct Bloquee(Arc<Mutex<()>>);

impl CpuSink for Bloquee {
    fn submit(&mut self, _: &CpuFrame) {
        drop(self.0.lock().unwrap());
    }

    fn status(&self) -> SinkStatus {
        SinkStatus::Active
    }
}

#[test]
fn un_flux_qui_decroche_perd_ses_frames_sans_ralentir_la_boucle() {
    let Some(gpu) = gpu() else { return };
    let image = plein(&gpu, 32, 32, [F1, F1, F1, F1]);
    let verrou = Arc::new(Mutex::new(()));
    let tenu = verrou.lock().unwrap();
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossy);
    let id = sorties.add_stream(Box::new(Bloquee(Arc::clone(&verrou))), format(32, 32, 60));
    for i in 0..20 {
        sorties.submit(&gpu, &image, i as f64 / 60.0);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(matches!(
        sorties.stream_status(id),
        Some(SinkStatus::Degraded { dropped }) if dropped > 0
    ));
    drop(tenu);
    sorties.finish();
}

#[test]
fn un_rendu_d_une_autre_taille_borne_les_flux_suivants() {
    let Some(gpu) = gpu() else { return };
    let recu = Recu::default();
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossy);
    sorties.add_stream(Box::new(recu.clone()), format(64, 64, 60));
    sorties.submit(&gpu, &plein(&gpu, 64, 64, [F1, F1, F1, F1]), 0.0);
    std::thread::sleep(std::time::Duration::from_millis(5));
    sorties.submit(&gpu, &plein(&gpu, 32, 32, [F1, F1, F1, F1]), 1.0 / 60.0);
    sorties.finish();
    let tailles: Vec<u32> = recu.0.lock().unwrap().iter().map(|f| f.width).collect();
    assert_eq!(tailles, vec![64, 32]);
}

#[test]
fn en_rendu_offline_un_flux_ne_perd_aucune_frame() {
    let Some(gpu) = gpu() else { return };
    let image = plein(&gpu, 32, 32, [F1, F1, F1, F1]);
    let recu = Recu::default();
    let mut sorties = Outputs::new(&gpu, DropPolicy::Lossless);
    sorties.add_stream(Box::new(recu.clone()), format(16, 16, 60));
    for i in 0..6 {
        sorties.submit(&gpu, &image, i as f64 / 60.0);
    }
    sorties.finish();
    let frames = recu.0.lock().unwrap();
    assert_eq!(frames.len(), 6);
    assert!(frames.iter().enumerate().all(|(i, f)| f.index == i as u64));
    assert_eq!(frames[0].width, 16);
}
